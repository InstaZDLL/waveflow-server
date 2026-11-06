#!/bin/sh
# Builds the FFmpeg WaveFlow ships: static `ffmpeg` and `ffprobe`, LGPL, with
# `--disable-everything` and only the components the server asks for.
#
#   sh packaging/ffmpeg/build.sh fetch            download and check the tarballs
#   sh packaging/ffmpeg/build.sh build   <out>    build into <out>
#   sh packaging/ffmpeg/build.sh sources <out>    pack the corresponding source
#
# `build` leaves in <out> the two executables and `LICENSES/`: the texts of
# FFmpeg, LAME, Opus and libwebp, musl's on a musl target, the MinGW-w64 runtime's
# and winpthreads' on Windows, and `SOURCE.txt` (the versions, the tarballs and
# the configure line), where the standalone archives carry them (§2 and §3 of
# docs/plans/standalone-download-plan.md). `sources` leaves `ffmpeg-<version>-sources.tar.gz`: the four tarballs exactly as
# downloaded, their checksums, this script and the musl notice it copies — the
# corresponding source a release owes for the binaries.
#
# The work happens under $WORK_DIR (default: a folder in the system's temporary
# directory); the tarballs are kept in $FFMPEG_DOWNLOADS (default:
# $WORK_DIR/downloads) and checked against the SHA-256 below on every run, so a
# tarball that changed upstream stops the build rather than entering a release.
# One already there is not downloaded again, which is how CI fetches them once
# and hands them to every target. $BUILD_COMMIT, when set, names the commit of
# this script in LICENSES/SOURCE.txt.
#
# Runs on Linux (built inside Alpine, so the binaries are static against
# musl), on macOS, and on Windows in an MSYS2 UCRT64 shell. Needs a C compiler,
# make, nasm on x86_64, pkg-config, tar, and curl 7.71 or later (for
# `--retry-all-errors`; the three build environments all ship curl 8). `.github/workflows/ffmpeg.yml`
# runs it for the five targets of the standalone download and tests the result
# against the server; docs/plans/standalone-download-plan.md §3 says why the
# build is ours and why it is LGPL.

set -eu

FFMPEG_VERSION=8.1.3
FFMPEG_SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
LAME_VERSION=3.100
LAME_SHA256=ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e
OPUS_VERSION=1.6.1
OPUS_SHA256=6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1
WEBP_VERSION=1.6.0
WEBP_SHA256=e4ab7009bf0629fd11982d4c2aa83964cf244cffba7347ecd39019a9e38c4564
# The musl release that musl-COPYRIGHT, beside this script, was copied from:
# Alpine's packages carry no licence text, and a static Linux binary owes
# musl's notice.
MUSL_VERSION=1.2.6

FFMPEG_TARBALL=ffmpeg-$FFMPEG_VERSION.tar.xz
LAME_TARBALL=lame-$LAME_VERSION.tar.gz
OPUS_TARBALL=opus-$OPUS_VERSION.tar.gz
WEBP_TARBALL=libwebp-$WEBP_VERSION.tar.gz

FFMPEG_URL=https://ffmpeg.org/releases/$FFMPEG_TARBALL
LAME_URL=https://downloads.sourceforge.net/project/lame/lame/$LAME_VERSION/$LAME_TARBALL
OPUS_URL=https://downloads.xiph.org/releases/opus/$OPUS_TARBALL
WEBP_URL=https://storage.googleapis.com/downloads.webmproject.org/releases/webp/$WEBP_TARBALL
# Asked when upstream refuses: ffmpeg.org once turned down all six attempts of a
# runner. Alpine keeps the upstream tarballs of its release, under the same
# names, and the SHA-256 above decides whether the bytes are acceptable, wherever
# they came from. A newer pin that is not on it yet only loses the fallback.
MIRROR=https://distfiles.alpinelinux.org/distfiles/v3.24

# What the server asks of FFmpeg, read from its code (§3 of the plan has the
# reasoning; the media, quality and canvas integration targets check it).
#
# Decoders: every format the scanner indexes (waveflow-core's
# AUDIO_EXTENSIONS: mp3, flac, wav, aiff/aif, ogg/oga/opus, m4a/mp4, aac,
# dsf/dff), for the transcode and the quality analysis. The MPEG audio layers
# share one decoder, so an `.mp3` that is really layer II still plays; WAV and
# AIFF carry PCM in any of the listed layouts; `dst` is the compressed form of
# DSDIFF. No video decoder: a canvas is only probed for its container and its
# streams and then remuxed with `-c copy`, and covers are never decoded here.
# `wrapped_avframe` is not a format but how the lavfi device hands a video
# source's frames to the encoder, which the fixtures' canvases need.
DECODERS="aac alac dsd_lsbf dsd_lsbf_planar dsd_msbf dsd_msbf_planar dst flac
mp1 mp1float mp2 mp2float mp3 mp3float opus vorbis
pcm_alaw pcm_f32be pcm_f32le pcm_f64be pcm_f64le pcm_mulaw pcm_s16be pcm_s16le
pcm_s24be pcm_s24le pcm_s32be pcm_s32le pcm_s8 pcm_u8
wrapped_avframe"
# Encoders: the transcode's `libmp3lame`, `libopus` and `aac`
# (src/media/transcode.rs); the analysis's `pcm_s32le`, and its spectrogram's
# `libwebp` or `mjpeg` (src/quality/). `flac`, `pcm_s16le` and `mpeg4` are
# what the integration tests generate their fixtures with: they run `ffmpeg`
# from PATH, so the release is tested with this one FFmpeg rather than two.
ENCODERS="aac flac libmp3lame libopus libwebp mjpeg mpeg4 pcm_s16le pcm_s32le"
# Not asked for, but enabled by configure because `mpeg4` selects it.
ENCODERS_SELECTED="h263"
DEMUXERS="aac aiff dsf flac iff matroska mov mp3 ogg wav"
# The transcode writes `mp3`, `ogg` and `adts`; the analysis `-f s32le`, whose
# component is `pcm_s32le`, then `image2pipe` and `webp`; a canvas is remuxed
# into `mp4` or `webm` (src/services/canvas/probe.rs). `flac`, `wav` and `mp4`
# again for fixtures.
MUXERS="adts flac image2pipe matroska mov mp3 mp4 ogg pcm_s32le wav webm webp"
PARSERS="aac flac mpegaudio opus vorbis"
# Not asked for, but enabled by configure because the `mov` muxer selects it.
PARSERS_SELECTED="ac3"
# The ones the mp4 and Matroska muxers insert on their own.
BSFS="aac_adtstoasc pgs_frame_merge vp9_superframe"
# `volume` is ReplayGain, `showspectrumpic` the spectrogram, `aresample` and
# `scale` the conversions FFmpeg inserts between a decoder and an encoder;
# `sine`, `anoisesrc`, `color` and `testsrc` are the fixtures' sources.
FILTERS="anoisesrc aformat anull aresample color format null scale
showspectrumpic sine testsrc volume"
# Not asked for, but enabled by configure: the `ffmpeg` tool selects the ones
# it inserts on its own for `-ss`/`-t`, cropping and autorotation.
FILTERS_SELECTED="atrim crop hflip rotate transpose trim vflip"
# Every input is opened under `-protocol_whitelist file` (RFC-023) and every
# output is a file or `pipe:1`. Nothing else is compiled in, so no input can
# reach the network whatever it names.
PROTOCOLS="file pipe"
# The fixtures' `-f lavfi -i sine=...`.
INDEVS="lavfi"

usage() {
    echo "usage: $0 fetch | build <out> | sources <out>" >&2
    exit 2
}

log() {
    echo "==> $*" >&2
}

# A comma-separated list, as configure takes it.
list() {
    echo $1 | tr ' ' ','
}

absolute() {
    mkdir -p "$1"
    (cd "$1" && pwd)
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

SCRIPT=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
MUSL_NOTICE=$(dirname "$SCRIPT")/musl-COPYRIGHT
WORK_DIR=$(absolute "${WORK_DIR:-${TMPDIR:-/tmp}/waveflow-ffmpeg}")
DOWNLOADS=$(absolute "${FFMPEG_DOWNLOADS:-$WORK_DIR/downloads}")

case "$(uname -s)" in
    Linux) PLATFORM=linux ;;
    Darwin) PLATFORM=macos ;;
    MINGW* | MSYS*) PLATFORM=windows ;;
    *) echo "unsupported system: $(uname -s)" >&2; exit 1 ;;
esac

JOBS=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 2)

# Every error is retried, a reset connection included, which plain `--retry`
# leaves alone: a mirror that hiccups is not a broken tarball, and the checksum
# in fetch_one still catches one that is.
download() {
    curl --fail --silent --show-error --location --connect-timeout 30 \
        --retry 5 --retry-all-errors --retry-delay 5 \
        --output "$DOWNLOADS/$1.part" "$2"
}

# Downloads one tarball unless it is already there, then holds it to its
# checksum either way: a copy left by an earlier run is checked like a new one.
fetch_one() {
    tarball=$1
    url=$2
    expected=$3
    if [ ! -f "$DOWNLOADS/$tarball" ]; then
        log "downloading $tarball"
        if ! download "$tarball" "$url"; then
            log "$url refused it; asking the mirror"
            download "$tarball" "$MIRROR/$tarball"
        fi
        mv "$DOWNLOADS/$tarball.part" "$DOWNLOADS/$tarball"
    fi
    actual=$(sha256 "$DOWNLOADS/$tarball")
    if [ "$actual" != "$expected" ]; then
        echo "checksum mismatch for $tarball: expected $expected, got $actual" >&2
        exit 1
    fi
    echo "$expected  $tarball"
}

fetch() {
    mkdir -p "$DOWNLOADS"
    {
        fetch_one "$FFMPEG_TARBALL" "$FFMPEG_URL" "$FFMPEG_SHA256"
        fetch_one "$LAME_TARBALL" "$LAME_URL" "$LAME_SHA256"
        fetch_one "$OPUS_TARBALL" "$OPUS_URL" "$OPUS_SHA256"
        fetch_one "$WEBP_TARBALL" "$WEBP_URL" "$WEBP_SHA256"
    } > "$DOWNLOADS/SHA256SUMS"
}

# Unpacks a tarball into a fresh source folder, so a run never builds on top
# of what an earlier one left half-configured.
unpack() {
    rm -rf "$SOURCES/$2"
    tar -xf "$DOWNLOADS/$1" -C "$SOURCES"
    test -d "$SOURCES/$2"
}

build_lame() {
    log "building LAME $LAME_VERSION"
    unpack "$LAME_TARBALL" "lame-$LAME_VERSION"
    cd "$SOURCES/lame-$LAME_VERSION"
    # The library alone: no command-line front end, no mpglib decoder (FFmpeg
    # decodes MP3 itself), no assembly.
    ./configure --prefix="$PREFIX" --enable-static --disable-shared \
        --disable-frontend --disable-decoder --disable-gtktest \
        --disable-dependency-tracking
    make -j "$JOBS"
    make install
}

build_opus() {
    log "building Opus $OPUS_VERSION"
    unpack "$OPUS_TARBALL" "opus-$OPUS_VERSION"
    cd "$SOURCES/opus-$OPUS_VERSION"
    # The neural features (DRED, deep PLC, OSCE) stay off, as they are by
    # default: their weights are megabytes, and a music transcode at the
    # bitrates the server offers never uses them.
    ./configure --prefix="$PREFIX" --enable-static --disable-shared \
        --disable-doc --disable-extra-programs --disable-dependency-tracking
    make -j "$JOBS"
    make install
}

build_webp() {
    log "building libwebp $WEBP_VERSION"
    unpack "$WEBP_TARBALL" "libwebp-$WEBP_VERSION"
    cd "$SOURCES/libwebp-$WEBP_VERSION"
    # The encoder alone: no image readers for the example tools, no mux,
    # demux or decoder libraries.
    ./configure --prefix="$PREFIX" --enable-static --disable-shared \
        --disable-libwebpmux --disable-libwebpdemux --disable-libwebpdecoder \
        --disable-png --disable-jpeg --disable-tiff --disable-gif --disable-wic \
        --disable-sdl --disable-gl --disable-dependency-tracking
    make -j "$JOBS"
    make install
}

configure_ffmpeg() {
    case "$PLATFORM" in
        windows) threads=--enable-w32threads ;;
        *) threads=--enable-pthreads ;;
    esac
    # A static executable where the system allows one: musl on Linux, the
    # MinGW runtime on Windows. macOS has no static libSystem, so its binaries
    # link the system's and nothing else.
    case "$PLATFORM" in
        macos) static_exe= ;;
        *) static_exe=--extra-ldexeflags=-static ;;
    esac
    # `--disable-autodetect` keeps every library the build machine happens to
    # have out of the binary (zlib, iconv, SDL, VideoToolbox, ...): what is
    # linked is what is listed here, on every target. No `--enable-gpl`, no
    # `--enable-version3`, no `--enable-nonfree`: FFmpeg stays LGPL-2.1-or-later.
    set -- \
        --prefix="$PREFIX" \
        --extra-version=waveflow \
        --pkg-config-flags=--static \
        --extra-cflags="-I$PREFIX/include" \
        --extra-ldflags="-L$PREFIX/lib" \
        $static_exe \
        --enable-static --disable-shared \
        --disable-autodetect $threads \
        --disable-everything \
        --disable-doc --disable-ffplay --disable-network \
        --enable-libmp3lame --enable-libopus --enable-libwebp \
        --enable-decoder="$(list "$DECODERS")" \
        --enable-encoder="$(list "$ENCODERS")" \
        --enable-demuxer="$(list "$DEMUXERS")" \
        --enable-muxer="$(list "$MUXERS")" \
        --enable-parser="$(list "$PARSERS")" \
        --enable-bsf="$(list "$BSFS")" \
        --enable-filter="$(list "$FILTERS")" \
        --enable-protocol="$(list "$PROTOCOLS")" \
        --enable-indev="$(list "$INDEVS")"
    CONFIGURE_LINE="./configure $*"
    ./configure "$@"
}

build_ffmpeg() {
    log "building FFmpeg $FFMPEG_VERSION"
    unpack "$FFMPEG_TARBALL" "ffmpeg-$FFMPEG_VERSION"
    cd "$SOURCES/ffmpeg-$FFMPEG_VERSION"
    configure_ffmpeg
    components_wrong=
    check_components DECODER "$DECODERS"
    check_components ENCODER "$ENCODERS" "$ENCODERS_SELECTED"
    check_components DEMUXER "$DEMUXERS"
    check_components MUXER "$MUXERS"
    check_components PARSER "$PARSERS" "$PARSERS_SELECTED"
    check_components BSF "$BSFS"
    check_components FILTER "$FILTERS" "$FILTERS_SELECTED"
    check_components PROTOCOL "$PROTOCOLS"
    check_components INDEV "$INDEVS"
    check_components OUTDEV ""
    [ -z "$components_wrong" ] || exit 1
    make -j "$JOBS"
    make install
}

# What configure enabled must be exactly what is listed above. It takes a name
# it does not know with a warning lost in its output and builds without it:
# `--enable-muxer=s32le` once left the analysis with no way to write its
# samples. And it enables on its own whatever a listed component or the
# `ffmpeg` tool selects, so a new `--enable-lib...` could bring in a decoder or
# an encoder of that library's nobody asked for; what is selected that way
# today is named in the *_SELECTED lists. Every kind is checked before failing,
# so one run reports everything that is off. `frame_thread_encoder` is
# configure's switch for threaded encoding, not an encoder, and is skipped.
check_components() {
    label=$(echo "$1" | tr 'A-Z' 'a-z')
    enabled=$(sed -n -e '/^CONFIG_FRAME_THREAD_ENCODER=/d' \
        -e "s/^CONFIG_\\([A-Z0-9_]*\\)_$1=yes\$/\\1/p" ffbuild/config.mak | tr 'A-Z' 'a-z')
    for name in $2; do
        if ! echo "$enabled" | grep -qx "$name"; then
            echo "configure did not enable the $name $label" >&2
            components_wrong=1
        fi
    done
    for name in $enabled; do
        case " $(echo $2 ${3:-}) " in
            *" $name "*) ;;
            *)
                echo "configure enabled the $name $label, which is not listed" >&2
                components_wrong=1
                ;;
        esac
    done
}

# The protocols the build answers with must be exactly `file` and `pipe`, for
# input and for output: the point of the list above is that nothing else
# exists, so a configure change that let one in fails here.
check_protocols() {
    found=$("$1" -hide_banner -protocols | tr -d '\r' | sed -n 's/^  *\([a-z0-9_]*\)$/\1/p' | sort -u | tr '\n' ' ')
    if [ "$found" != "file pipe " ]; then
        echo "unexpected protocols in $1: $found" >&2
        exit 1
    fi
    log "protocols: $found"
}

# The notice has to be the one of the musl being linked, so a build against
# another version stops here, before any compiling, until the copy is refreshed.
check_musl_notice() {
    case "$TARGET" in
        *-musl*) ;;
        *) return 0 ;;
    esac
    linked=$(ldd --version 2>&1 | sed -n 's/^Version //p')
    if [ "$linked" != "$MUSL_VERSION" ]; then
        echo "this links musl $linked, but musl-COPYRIGHT is from $MUSL_VERSION" >&2
        exit 1
    fi
}

write_source() {
    {
        echo "FFmpeg for WaveFlow Server, built by packaging/ffmpeg/build.sh"
        echo
        echo "Target:   $TARGET"
        echo "Compiler: $($CC --version 2>/dev/null | head -n 1)"
        if [ -n "${BUILD_COMMIT:-}" ]; then
            echo "Script:   ${WAVEFLOW_SOURCE_BLOB:-https://github.com/${GITHUB_REPOSITORY:-InstaZDLL/waveflow-server}/blob}/$BUILD_COMMIT/packaging/ffmpeg/build.sh"
        fi
        echo
        echo "Components (licence):"
        echo "  FFmpeg  $FFMPEG_VERSION  (LGPL-2.1-or-later)"
        echo "  LAME    $LAME_VERSION   (LGPL-2.0-or-later)"
        echo "  Opus    $OPUS_VERSION   (BSD-3-Clause)"
        echo "  libwebp $WEBP_VERSION   (BSD-3-Clause)"
        case "$TARGET" in
            *-musl*) echo "  musl    $MUSL_VERSION   (MIT), the C library, linked statically" ;;
        esac
        if [ "$PLATFORM" = windows ]; then
            echo "  The MinGW-w64 runtime and winpthreads, linked statically: their"
            echo "  notices are MinGW-w64-runtime.txt and winpthreads.txt, beside"
            echo "  this file. GCC's runtime is under its Runtime Library Exception."
        fi
        echo
        echo "Source tarballs (SHA-256), as downloaded from upstream:"
        sed 's/^/  /' "$DOWNLOADS/SHA256SUMS"
        echo
        echo "The four tarballs and this script are attached to the release that"
        echo "ships these binaries, as ffmpeg-$FFMPEG_VERSION-sources.tar.gz."
        echo
        echo "FFmpeg configure line:"
        echo "  $CONFIGURE_LINE"
    } > "$OUT/LICENSES/SOURCE.txt"
}

write_licences() {
    {
        cat "$SOURCES/ffmpeg-$FFMPEG_VERSION/LICENSE.md"
        echo
        cat "$SOURCES/ffmpeg-$FFMPEG_VERSION/COPYING.LGPLv2.1"
    } > "$OUT/LICENSES/FFmpeg.txt"
    {
        cat "$SOURCES/lame-$LAME_VERSION/LICENSE"
        echo
        cat "$SOURCES/lame-$LAME_VERSION/COPYING"
    } > "$OUT/LICENSES/LAME.txt"
    cp "$SOURCES/opus-$OPUS_VERSION/COPYING" "$OUT/LICENSES/Opus.txt"
    {
        cat "$SOURCES/libwebp-$WEBP_VERSION/COPYING"
        echo
        cat "$SOURCES/libwebp-$WEBP_VERSION/PATENTS"
    } > "$OUT/LICENSES/libwebp.txt"
    case "$TARGET" in
        *-musl*) cp "$MUSL_NOTICE" "$OUT/LICENSES/musl.txt" ;;
    esac
    # `-static` copies MinGW-w64's runtime into both executables, and that
    # runtime's own note says what this owes: this file, in the package. The
    # winpthreads library comes along, though FFmpeg's threads are w32threads,
    # for `nanosleep` and `clock_gettime` and for GCC's runtime: its sources'
    # paths can be read in the binary. Its notice is a file of its own. Both are
    # copied from the packages this build just linked, so they are that
    # runtime's, and a package that moved them stops the build here.
    if [ "$PLATFORM" = windows ]; then
        cp "$MINGW_PREFIX/share/licenses/crt/COPYING.MinGW-w64-runtime.txt" "$OUT/LICENSES/MinGW-w64-runtime.txt"
        cp "$MINGW_PREFIX/share/licenses/winpthreads/COPYING" "$OUT/LICENSES/winpthreads.txt"
    fi
}

build() {
    OUT=$(absolute "$1")
    fetch
    SOURCES=$WORK_DIR/src
    PREFIX=$WORK_DIR/prefix
    rm -rf "$PREFIX"
    mkdir -p "$SOURCES" "$PREFIX"
    PKG_CONFIG_PATH=$PREFIX/lib/pkgconfig
    export PKG_CONFIG_PATH
    # MinGW may name its compiler `gcc` only; autoconf and FFmpeg find it
    # either way, this only labels SOURCE.txt.
    CC=cc
    command -v cc >/dev/null 2>&1 || CC=gcc
    TARGET=$($CC -dumpmachine)
    check_musl_notice

    build_lame
    build_opus
    build_webp
    build_ffmpeg

    case "$PLATFORM" in
        windows) exe=.exe ;;
        *) exe= ;;
    esac
    cp "$PREFIX/bin/ffmpeg$exe" "$PREFIX/bin/ffprobe$exe" "$OUT/"
    check_protocols "$OUT/ffmpeg$exe"
    # What the binaries still load from the system, for the log: libSystem on
    # macOS, Windows' own DLLs on Windows. The Linux ones are static.
    case "$PLATFORM" in
        macos) otool -L "$OUT/ffmpeg" ;;
        windows) objdump -p "$OUT/ffmpeg.exe" | grep 'DLL Name' ;;
        *) ;;
    esac
    mkdir -p "$OUT/LICENSES"
    write_licences
    write_source
    log "built into $OUT"
    ls -l "$OUT"
}

sources() {
    OUT=$(absolute "$1")
    fetch
    bundle=ffmpeg-$FFMPEG_VERSION-sources
    staging=$WORK_DIR/$bundle
    rm -rf "$staging"
    mkdir -p "$staging"
    cp "$DOWNLOADS/$FFMPEG_TARBALL" "$DOWNLOADS/$LAME_TARBALL" \
        "$DOWNLOADS/$OPUS_TARBALL" "$DOWNLOADS/$WEBP_TARBALL" \
        "$DOWNLOADS/SHA256SUMS" "$staging/"
    cp "$SCRIPT" "$staging/build.sh"
    cp "$MUSL_NOTICE" "$staging/"
    tar -czf "$OUT/$bundle.tar.gz" -C "$WORK_DIR" "$bundle"
    log "packed $OUT/$bundle.tar.gz"
}

[ $# -ge 1 ] || usage
command=$1
shift
case "$command" in
    fetch) [ $# -eq 0 ] || usage; fetch; cat "$DOWNLOADS/SHA256SUMS" ;;
    build) [ $# -eq 1 ] || usage; build "$1" ;;
    sources) [ $# -eq 1 ] || usage; sources "$1" ;;
    *) usage ;;
esac
