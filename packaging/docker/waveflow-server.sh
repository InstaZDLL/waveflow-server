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

# Only when the server itself starts: a command run beside it has no business
# walking the whole directory, and the server has already handed it over.
if [ "${1:-serve}" = serve ]; then
    data="${WAVEFLOW_DATA_DIR:-/data}"
    mkdir -p "$data"
    # Only what is not already the account's, so a boot after the first walks
    # the directory and changes nothing.
    #
    # Root walks a directory the account can write to, so the walk must not be
    # a way for that account to own something else:
    # - `-execdir` runs `chown` from inside each directory on `./name`, so no
    #   directory on the way can be swapped for a link mid-walk; and `-h`
    #   changes a link itself, never what it points at;
    # - a file with a second name is skipped: a hard link to a file of root's,
    #   left here, would otherwise be handed over at the next start. The
    #   server never makes one.
    if ! find "$data" \( ! -user "$uid" -o ! -group "$gid" \) \
        ! \( -type f -links +1 \) \
        -execdir chown -h "$uid:$gid" {} +; then
        # A share that refuses `chown` (NFS with root squashed, SMB) may still
        # let the account write; the server says so itself if it cannot.
        echo "waveflow-server: could not give $data to $uid:$gid; starting anyway" >&2
    fi
fi

# The account's home, as `USER waveflow` used to set it.
export HOME=/home/waveflow
exec setpriv --reuid="$uid" --regid="$gid" --clear-groups -- "$binary" "$@"
