#!/bin/sh
# Starts one standalone archive and plays a file through it, on the platform the
# archive is for:
#
#   sh packaging/standalone/smoke.sh <archive>
#
# What it holds is what no test of the source tree can see: that the archive has
# the files of §2 of docs/plans/standalone-download-plan.md, that the server in
# it needs nothing from the machine, finds the laptop.env and the two tools
# beside it with an empty PATH, and turns a FLAC file into an MP3 with them. It
# leaves the browser shut, and every system folder it would write to points at a
# temporary one, so it can run on a machine that has a server of its own. Set
# SMOKE_PORT when 4533 is taken there.
#
# Not the release-candidate check of #399, which is a person double-clicking on
# a real desktop: Gatekeeper and SmartScreen never look at what a script runs.

set -eu

fail() {
    echo "smoke: $*" >&2
    exit 1
}

[ $# -eq 1 ] || fail "usage: $0 <archive>"
archive=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
port=${SMOKE_PORT:-4533}
base=http://127.0.0.1:$port

case "$(basename "$archive")" in
    *-windows-*.zip) platform=windows exe=.exe ;;
    *-macos-*.zip) platform=macos exe= ;;
    *-linux-*.tar.gz) platform=linux exe= ;;
    *) fail "not a standalone archive: $archive" ;;
esac
name=$(basename "$archive" .zip)
name=$(basename "$name" .tar.gz)

# With a space in it, as a Windows profile or a Downloads folder may have.
work=$(mktemp -d)/"smoke test"
mkdir -p "$work/extracted" "$work/home" "$work/music" "$work/cwd"
server_pid=
# Under Git Bash, `kill` stops the shell's stand-in and leaves the Windows
# process serving, with its database open. /proc names the real one.
stop_server() {
    if [ "$platform" = windows ]; then
        taskkill //F //T //PID "$(cat "/proc/$server_pid/winpid")" >/dev/null 2>&1 || true
    fi
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
}
cleanup() {
    status=$?
    [ -z "$server_pid" ] || stop_server
    if [ "$status" -ne 0 ] && [ -f "$work/server.log" ]; then
        echo "smoke: the server's log:" >&2
        cat "$work/server.log" >&2
    fi
    rm -rf "$(dirname "$work")" || true
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

native() {
    if [ "$platform" = windows ]; then cygpath -w "$1"; else echo "$1"; fi
}

case "$platform" in
    windows) "$(cygpath -u "$SYSTEMROOT")/System32/tar.exe" -xf "$(native "$archive")" -C "$(native "$work/extracted")" ;;
    macos) unzip -q "$archive" -d "$work/extracted" ;;
    linux) tar -xzf "$archive" -C "$work/extracted" ;;
esac

# --- What the archive holds --------------------------------------------------

[ "$(ls "$work/extracted")" = "$name" ] || fail "the archive is not one folder named $name: $(ls "$work/extracted")"
folder=$work/extracted/$name
server=$folder/waveflow-server$exe
ffmpeg=$folder/ffmpeg$exe
ffprobe=$folder/ffprobe$exe

listing() {
    printf '%s\n' "$@" | LC_ALL=C sort | tr '\n' ' '
}
found=$(cd "$folder" && listing *)
wanted=$(listing "waveflow-server$exe" "ffmpeg$exe" "ffprobe$exe" laptop.env LICENSES README.txt)
[ "$found" = "$wanted" ] || fail "the folder holds $found, not $wanted"

case "$platform" in
    windows) notices="MinGW-w64-runtime.txt winpthreads.txt" ;;
    linux) notices="musl.txt" ;;
    macos) notices= ;;
esac
found=$(cd "$folder/LICENSES" && listing *)
# shellcheck disable=SC2086 # The notices are a list.
wanted=$(listing COPYING FFmpeg.txt LAME.txt Opus.txt SOURCE.txt libwebp.txt $notices)
[ "$found" = "$wanted" ] || fail "LICENSES holds $found, not $wanted"
# The core's line, whole: the revision read here from Cargo.lock, not from the
# archive, at the address pack.sh was told to use (GitHub's by default, GitLab's
# in .gitlab-ci.yml). A wrong revision or a lost prefix fails here.
root=$(cd "$(dirname "$0")/../.." && pwd)
core_revision=$(sed -n 's|^source = "git+https://github.com/InstaZDLL/WaveFlow?[a-z]*=[^#]*#\([0-9a-f]\{40\}\)"$|\1|p' "$root/Cargo.lock" | head -n 1)
[ -n "$core_revision" ] || fail "Cargo.lock names no waveflow-core revision"
core_line="  waveflow-core   ${WAVEFLOW_CORE_TREE:-https://github.com/InstaZDLL/WaveFlow/tree}/$core_revision"
grep -qxF "$core_line" "$folder/LICENSES/SOURCE.txt" || fail "SOURCE.txt does not say: $core_line"
for said in "crates.io" "webapp/bun.lock" "FFmpeg configure line"; do
    grep -q "$said" "$folder/LICENSES/SOURCE.txt" || fail "SOURCE.txt does not name $said"
done

if [ "$platform" != windows ]; then
    for tool in "$server" "$ffmpeg" "$ffprobe"; do
        [ -x "$tool" ] || fail "$tool is not executable"
    done
fi

# --- What it needs from the machine -------------------------------------------

case "$platform" in
    # A static binary answers one of these two, depending on ldd's version.
    linux)
        for tool in "$server" "$ffmpeg" "$ffprobe"; do
            links=$(ldd "$tool" 2>&1 || true)
            case "$links" in
                *"statically linked"* | *"not a dynamic executable"*) ;;
                *) fail "$tool is not static: $links" ;;
            esac
        done
        ;;
    macos)
        for tool in "$server" "$ffmpeg" "$ffprobe"; do
            extra=$(otool -L "$tool" | tail -n +2 | grep -v -E '^[[:space:]]+(/usr/lib/|/System/Library/)' || true)
            [ -z "$extra" ] || fail "$tool loads something a Mac does not have: $extra"
        done
        ;;
    # The names of the DLLs a program loads are plain text in it. Windows carries
    # none of these on its own, and a laptop that has never installed the
    # redistributable would refuse the executable before it printed a line.
    windows)
        for tool in "$server" "$ffmpeg" "$ffprobe"; do
            if LC_ALL=C grep -aqiE 'vcruntime[0-9]+[a-z_0-9]*\.dll|msvcp[0-9]+\.dll|libwinpthread-1\.dll|libgcc_s[a-z_0-9-]*\.dll' "$tool"; then
                fail "$tool loads a runtime Windows does not have"
            fi
        done
        ;;
esac

# --- Laptop mode --------------------------------------------------------------

# For a subshell, so the empty PATH is the server's alone. Only WAVEFLOW_BIND,
# for SMOKE_PORT, and the browser's switch come from outside; laptop mode itself
# comes from the file in the archive.
native_home=$(native "$work/home")
server_environment() {
    PATH=/nonexistent
    HOME=$work/home
    LOCALAPPDATA=$native_home
    WAVEFLOW_BIND=127.0.0.1:$port
    WAVEFLOW_LAPTOP_OPEN_BROWSER=false
    RUST_LOG=info
    export PATH HOME LOCALAPPDATA WAVEFLOW_BIND WAVEFLOW_LAPTOP_OPEN_BROWSER RUST_LOG
    unset XDG_DATA_HOME WAVEFLOW_LAPTOP
    cd "$work/cwd"
}
run_server() (
    server_environment
    exec "$server" "$@"
)

# Not `run_server &`: a function that is a subshell, in the background, is two
# forks, and `$!` would name the outer one while the server went on serving.
(
    server_environment
    exec "$server"
) > "$work/server.log" 2>&1 &
server_pid=$!

attempts=0
until curl --silent --fail --output /dev/null "$base/ready"; do
    kill -0 "$server_pid" 2>/dev/null || fail "the server stopped before it was ready"
    attempts=$((attempts + 1))
    [ "$attempts" -le 120 ] || fail "the server was not ready after two minutes"
    sleep 1
done

grep -q "settings read from laptop.env" "$work/server.log" || fail "laptop.env was not read"
grep -q "Close this window to stop the server" "$work/server.log" || fail "no laptop banner"
case "$platform" in
    windows) data=$work/home/"WaveFlow Server" ;;
    macos) data=$work/home/"Library/Application Support/WaveFlow Server" ;;
    linux) data=$work/home/.local/share/waveflow-server ;;
esac
[ -s "$data/setup-token" ] || fail "no setup token in $data"
[ ! -e "$work/cwd/data" ] || fail "the server wrote its data where it was started"

# build.rs embeds a placeholder rather than fail when the client was not built.
page=$(curl --silent --fail "$base/")
case "$page" in
    *"The web client is not built"*) fail "the archive carries the placeholder page" ;;
    *'<div id="root"'*) ;;
    *) fail "the root page is not the web client: $page" ;;
esac

# --- A file, played ------------------------------------------------------------

"$ffmpeg" -hide_banner -loglevel error -f lavfi -i sine=frequency=440:duration=3 \
    -metadata title=SmokeTone -metadata artist=Smoke -metadata album=Smoke \
    -c:a flac "$(native "$work/music/tone.flac")"

# Their standard output stays out of the log: `credential set` prints an API key.
web_password=smoke-web-$$-password
app_password=smoke-app-$$-password
(WAVEFLOW_ACCOUNT_PASSWORD=$web_password && export WAVEFLOW_ACCOUNT_PASSWORD &&
    run_server account create-admin --username smoke >/dev/null)
run_server library add --owner smoke --name Music --path "$(native "$work/music")" >/dev/null
(WAVEFLOW_SUBSONIC_PASSWORD=$app_password && export WAVEFLOW_SUBSONIC_PASSWORD &&
    run_server credential set --actor smoke --username smoke >/dev/null)

# The song's own id, not its artist's or its album's: only a song says `isDir`
# right after it.
auth="u=smoke&p=$app_password&v=1.16.1&c=smoke"
answer=$(curl --silent --fail --show-error "$base/rest/search3.view?$auth&f=json&query=SmokeTone")
id=$(echo "$answer" | sed -n 's/.*"id":"\([^"]*\)","isDir":false.*/\1/p')
[ -n "$id" ] || fail "the scan did not find the file: $answer"

curl --silent --fail --show-error --output "$work/tone.mp3" \
    "$base/rest/stream.view?$auth&id=$id&format=mp3&maxBitRate=128"
format=$("$ffprobe" -hide_banner -loglevel error -show_entries format=format_name \
    -of default=noprint_wrappers=1:nokey=1 "$(native "$work/tone.mp3")" | tr -d '\r')
[ "$format" = mp3 ] || fail "the stream is $format, not mp3"

# This script may run on a machine that goes on being used.
stop_server
if curl --silent --output /dev/null --max-time 5 "$base/ready"; then
    fail "the server still answers after being stopped"
fi
server_pid=

echo "smoke: $name started in laptop mode and played a file"
