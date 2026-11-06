#!/bin/sh
# Packs one standalone archive (§2 of docs/plans/standalone-download-plan.md).
#
#   sh packaging/standalone/pack.sh <target> <version> <source-ref> <server> <ffmpeg> <out>
#
# <target> is one of windows-x86_64, macos-arm64, macos-x86_64, linux-x86_64 and
# linux-arm64; <source-ref> the tag, or the commit of a build that has none;
# <server> the built executable; <ffmpeg> the folder packaging/ffmpeg/build.sh
# filled for that target. Leaves in <out> one archive holding one folder named
# after itself, and prints its path.
#
# Runs on the target's own runner, in Git Bash on Windows. Reads the repository
# it sits in for the licence, Cargo.lock and the files beside it.

set -eu

die() {
    echo "$*" >&2
    exit 1
}

[ $# -eq 6 ] || die "usage: $0 <target> <version> <source-ref> <server> <ffmpeg> <out>"
target=$1
version=$2
source_ref=$3
server=$4
ffmpeg=$5
out=$6

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
# Where SOURCE.txt sends a reader for the source, the ref appended. GitHub's by
# default; .gitlab-ci.yml sets both to the GitLab projects, whose tree pages
# live under `/-/tree/`, so that an archive built there does not point at a
# host that may be gone (AGPL §6(d)).
source_tree=${WAVEFLOW_SOURCE_TREE:-https://github.com/${GITHUB_REPOSITORY:-InstaZDLL/waveflow-server}/tree}
core_tree=${WAVEFLOW_CORE_TREE:-https://github.com/InstaZDLL/WaveFlow/tree}

case "$target" in
    windows-x86_64) platform=windows exe=.exe extension=zip ;;
    macos-arm64 | macos-x86_64) platform=macos exe= extension=zip ;;
    linux-x86_64 | linux-arm64) platform=linux exe= extension=tar.gz ;;
    *) die "unknown target: $target" ;;
esac

# The server's corresponding source sits on other servers only because each is
# durable and public (AGPL §6(d), §3 of the plan): the tag, crates.io, and the
# one git dependency. Anything else in the lock file has no such promise, and
# its source has to be attached to the release, so it stops the release here
# rather than leaving SOURCE.txt to say something untrue.
foreign=$(sed -n 's/^source = "\(.*\)"$/\1/p' "$root/Cargo.lock" | sort -u |
    grep -v '^registry+https://github.com/rust-lang/crates.io-index$' || true)
core_revision=$(echo "$foreign" |
    sed -n 's|^git+https://github.com/InstaZDLL/WaveFlow?rev=\([0-9a-f]\{40\}\)#\1$|\1|p')
if [ -z "$core_revision" ] ||
    [ "$foreign" != "git+https://github.com/InstaZDLL/WaveFlow?rev=$core_revision#$core_revision" ]; then
    die "Cargo.lock names a source beyond crates.io and waveflow-core's revision:
$foreign
attach its source to the release and say so in SOURCE.txt (§3 of the plan)"
fi

name=waveflow-server-$version-$target
mkdir -p "$out"
out=$(cd "$out" && pwd)
staging=$out/.staging
rm -rf "$staging" "${out:?}/$name.$extension"
folder=$staging/$name
mkdir -p "$folder/LICENSES"

# An artifact keeps no file mode, so the modes are set here rather than kept.
cp "$server" "$folder/waveflow-server$exe"
cp "$ffmpeg/ffmpeg$exe" "$ffmpeg/ffprobe$exe" "$folder/"
chmod 755 "$folder/waveflow-server$exe" "$folder/ffmpeg$exe" "$folder/ffprobe$exe"
cp "$here/laptop.env" "$folder/laptop.env"
cat "$here/README/$platform.txt" "$here/README/common.txt" > "$folder/README.txt"

cp -R "$ffmpeg/LICENSES/." "$folder/LICENSES/"
cp "$root/LICENSE" "$folder/LICENSES/COPYING"
{
    cat <<EOF
WaveFlow Server $version, for $target, under the GNU Affero General Public
License version 3 (COPYING, beside this file).

The corresponding source of this server, at the versions this archive was
built from:

  The server      $source_tree/$source_ref
                  Its Cargo.lock pins the Rust crates, and its webapp/bun.lock
                  the packages of the web client compiled into the server.
  waveflow-core   $core_tree/$core_revision
                  The one Rust dependency not published on crates.io.
  Rust crates     https://crates.io, at the versions Cargo.lock pins.
  Web packages    https://www.npmjs.com, at the versions webapp/bun.lock pins.

--------------------------------------------------------------------------------

EOF
    cat "$ffmpeg/LICENSES/SOURCE.txt"
} > "$folder/LICENSES/SOURCE.txt"

cd "$staging"
case "$target" in
    # Windows' own tar, not Git Bash's: it is the one that writes a zip.
    windows-*) "$(cygpath -u "$SYSTEMROOT")/System32/tar.exe" -a -c -f "$(cygpath -w "$out/$name.zip")" "$name" ;;
    # `zip` keeps the executable bits, which Archive Utility then restores.
    macos-*) zip -qr "$out/$name.zip" "$name" ;;
    linux-*) tar --owner=0 --group=0 -czf "$out/$name.tar.gz" "$name" ;;
esac
cd "$out"
rm -rf "$staging"
echo "$out/$name.$extension"
