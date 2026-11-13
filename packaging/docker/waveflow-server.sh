#!/bin/sh
# The image's entry point, and the `waveflow-server` on its PATH (#444).
#
# The image starts as root so that it can work in a data directory it did not
# create. Home Assistant, Unraid and most NAS hand the container a folder of
# their own, owned by root or by some account of the host, and a server that
# started as its own user could not open its database there: "unable to open
# database file", before anything else. As root, this script gives the data
# directory to the server's account, then runs the server as that account and
# never as root.
#
# Every run of the binary goes through here, `docker exec … waveflow-server
# library add` and the HEALTHCHECK included, so a command an operator runs as
# root writes its files as the server's account as well: a scan or a database
# file created by root would otherwise be one the server cannot replace.
#
# Started as anyone else (`--user`, a runtime that never grants root), it only
# runs the binary: the data directory is then the caller's to own.
set -eu

binary=/usr/local/lib/waveflow-server/waveflow-server

if [ "$(id -u)" != 0 ]; then
    exec "$binary" "$@"
fi

# The account the server runs as. 4533 is the one the image creates; a NAS
# whose shares belong to another account names it here, and 0 keeps root, which
# a rootless Podman maps onto the user who started it.
uid="${WAVEFLOW_UID:-4533}"
gid="${WAVEFLOW_GID:-4533}"
for id in "$uid" "$gid"; do
    case "$id" in
        '' | *[!0-9]*)
            echo "waveflow-server: WAVEFLOW_UID and WAVEFLOW_GID must be numbers, got '$id'" >&2
            exit 64
            ;;
    esac
done

# Every mount point strictly below the directory `$1`, one per line, as the
# kernel lists them. `mountinfo` writes a space, a tab, a newline and a
# backslash as `\040`, `\011`, `\012` and `\134`, which `%b` reads back.
mounts_below() {
    while read -r _ _ _ _ point _; do
        point="$(printf '%b' "$point")"
        case "$point" in
            "$1"/*) printf '%s\n' "$point" ;;
        esac
    done < /proc/self/mountinfo
}

# Gives the data directory `$1` to the account.
#
# Only what is not already the account's, so a boot after the first walks the
# directory and changes nothing.
#
# Root walks a directory the account can write to, so the walk must not be a
# way for that account to own something else:
# - `-execdir` runs `chown` from inside each directory on `./name`, so no
#   directory on the way can be swapped for a link mid-walk; and `-h` changes a
#   link itself, never what it points at;
# - a file with a second name is skipped: a hard link to a file of root's, left
#   here, would otherwise be handed over at the next start. The server never
#   makes one;
# - a mount inside the data directory is the operator's, a music folder for
#   one, and its files belong to the host: it is left as it is, mount point
#   included. `-xdev` would not do: a folder of the host mounted from the disk
#   the data directory is on has the same device.
hand_over() {
    data="$1"
    set --
    # A here-document, not a pipe: a pipe would build the list in a subshell.
    while IFS= read -r point; do
        [ -n "$point" ] || continue
        echo "waveflow-server: leaving $point as it is: a mount inside the data directory" >&2
        # `-path` reads a pattern: its wildcards are taken literally.
        pattern="$(printf '%s\n' "$point" | sed 's/[][*?\\]/\\&/g')"
        set -- "$@" -path "$pattern" -prune -o
    done <<MOUNTS
$(mounts_below "$data")
MOUNTS
    find "$data" "$@" \( ! -user "$uid" -o ! -group "$gid" \) \
        ! \( -type f -links +1 \) \
        -execdir chown -h "$uid:$gid" {} +
}

# Whether every directory above `$1` is root's and writable by root alone.
#
# The walk starts from a path, and the kernel resolves that path again when it
# runs. A directory above the data directory that the account could write in
# would let it swap the data directory for a link to `/etc` just before, and
# root would hand `/etc` over. `/data`, under `/`, passes.
root_alone_above() {
    dir="$(dirname "$1")"
    while :; do
        if [ -n "$(find "$dir" -maxdepth 0 \( ! -user 0 -o -perm /022 \) -print)" ]; then
            return 1
        fi
        [ "$dir" = / ] && return 0
        dir="$(dirname "$dir")"
    done
}

# Only when the server itself starts: a command run beside it has no business
# walking the whole directory, and the server has already handed it over.
if [ "${1:-serve}" = serve ]; then
    data="${WAVEFLOW_DATA_DIR:-/data}"
    mkdir -p "$data"
    # Absolute and without `.`, `..` or doubled slashes, so it compares with the
    # paths `mountinfo` lists; links are kept, never followed.
    data="$(realpath --no-symlinks "$data")"
    if [ "$data" = / ]; then
        # Every file of the container would be walked, the mounts too: no
        # mount is "below /" as `mounts_below` reads it.
        echo "waveflow-server: not giving / to $uid:$gid: it is not a data directory" >&2
    elif [ "$(realpath "$data")" != "$data" ]; then
        # A link on the way may be the account's to point anywhere. The server
        # still starts, as the account, in whatever it can write.
        echo "waveflow-server: not giving $data to $uid:$gid: its path goes through a link" >&2
    elif ! root_alone_above "$data"; then
        echo "waveflow-server: not giving $data to $uid:$gid: a directory above it is not root's alone" >&2
    elif ! hand_over "$data"; then
        # A share that refuses `chown` (NFS with root squashed, SMB) may still
        # let the account write; the server says so itself if it cannot.
        echo "waveflow-server: could not give $data to $uid:$gid; starting anyway" >&2
    fi
fi

# The account's home, as `USER waveflow` used to set it.
export HOME=/home/waveflow
exec setpriv --reuid="$uid" --regid="$gid" --clear-groups -- "$binary" "$@"
