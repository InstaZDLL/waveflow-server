#!/usr/bin/env bash
# Bootstrap a throwaway WaveFlow instance with a small but exercisable library.
#
# Client work stalls without one: an empty catalogue only lets you test
# playlists, because favourites, ratings, scrobbles and the play queue all need
# a real track id to point at. This mints synthetic audio with FFmpeg, registers
# it, scans it, and prints the credentials a client needs.
#
# Nothing here is a production path. The data directory is disposable and the
# credentials are printed in clear on purpose.
#
#   scripts/seed-dev-instance.sh [data-dir]
#
# Environment:
#   WAVEFLOW_SEED_PASSWORD            account password (dev-only constant)
#   WAVEFLOW_SEED_SUBSONIC_PASSWORD   dedicated /rest/ password (dev-only)
#   WAVEFLOW_BIN                      server binary (default: cargo run --quiet --)

set -euo pipefail

DATA_DIR="${1:-./data-dev}"
PASSWORD="${WAVEFLOW_SEED_PASSWORD:-dev-password-change-me}"
SUBSONIC_PASSWORD="${WAVEFLOW_SEED_SUBSONIC_PASSWORD:-subsonic-dev-password}"
ADMIN="${WAVEFLOW_SEED_ADMIN:-dev-admin}"
MUSIC_DIR="$DATA_DIR/music"

if ! command -v ffmpeg > /dev/null; then
  echo "error: ffmpeg is required to mint the audio fixtures" >&2
  exit 1
fi

if [ -e "$DATA_DIR" ]; then
  echo "error: $DATA_DIR already exists — remove it or pass another path" >&2
  exit 1
fi

# `cargo run --` keeps this usable from a fresh checkout; point WAVEFLOW_BIN at
# a built binary to skip the compile.
if [ -n "${WAVEFLOW_BIN:-}" ]; then
  read -r -a SERVER <<< "$WAVEFLOW_BIN"
else
  SERVER=(cargo run --quiet --)
fi

export WAVEFLOW_DATA_DIR="$DATA_DIR"

mkdir -p "$MUSIC_DIR"

# Two artists, three albums, one compilation and one track without a track
# number: enough shape to catch ordering bugs (SQLite sorts NULL first) and to
# give browse endpoints something to paginate.
mint() {
  local path="$1" title="$2" artist="$3" album="$4" album_artist="$5" track="$6" year="$7"
  mkdir -p "$(dirname "$path")"
  local args=(
    -hide_banner -loglevel error -y
    -f lavfi -i "sine=frequency=${8:-440}:duration=1"
    -metadata "title=$title"
    -metadata "artist=$artist"
    -metadata "album=$album"
    -metadata "album_artist=$album_artist"
    -metadata "date=$year"
    -metadata "genre=Test"
  )
  [ -n "$track" ] && args+=(-metadata "track=$track")
  ffmpeg "${args[@]}" -c:a libmp3lame "$path"
}

echo "Minting audio fixtures under $MUSIC_DIR …"
mint "$MUSIC_DIR/Aurora/Northern Lights/01 Polar.mp3"   "Polar"   "Aurora" "Northern Lights" "Aurora" 1 2021 440
mint "$MUSIC_DIR/Aurora/Northern Lights/02 Solstice.mp3" "Solstice" "Aurora" "Northern Lights" "Aurora" 2 2021 494
mint "$MUSIC_DIR/Aurora/Northern Lights/03 Zenith.mp3"  "Zenith"  "Aurora" "Northern Lights" "Aurora" 3 2021 523
mint "$MUSIC_DIR/Bellweather/Tideline/01 Ebb.mp3"       "Ebb"     "Bellweather" "Tideline" "Bellweather" 1 2019 587
mint "$MUSIC_DIR/Bellweather/Tideline/02 Flow.mp3"      "Flow"    "Bellweather" "Tideline" "Bellweather" 2 2019 659
# No track number on purpose — exercises the NULLS LAST ordering.
mint "$MUSIC_DIR/Various/Assorted/Untitled Drift.mp3"   "Untitled Drift" "Aurora; Bellweather" "Assorted" "Various Artists" "" 2023 349

echo
echo "Creating administrator '$ADMIN' …"
WAVEFLOW_ACCOUNT_PASSWORD="$PASSWORD" "${SERVER[@]}" account create-admin --username "$ADMIN"

echo
echo "Registering and scanning the library …"
"${SERVER[@]}" library add --owner "$ADMIN" --name "Dev library" --path "$MUSIC_DIR"

echo
echo "Creating a native API token …"
"${SERVER[@]}" token create --actor "$ADMIN" --username "$ADMIN" --name "dev-client" --scopes read,write

# Without this the instance answers 40 to every /rest/ call, which looks like a
# broken build rather than a missing credential.
echo
echo "Setting the Subsonic credential …"
WAVEFLOW_SUBSONIC_PASSWORD="$SUBSONIC_PASSWORD" "${SERVER[@]}" credential set --actor "$ADMIN" --username "$ADMIN"

cat <<EOF

──────────────────────────────────────────────────────────────
Seeded instance ready.

  data dir   $DATA_DIR
  username   $ADMIN
  password   $PASSWORD
  subsonic   $SUBSONIC_PASSWORD  (for /rest/ — the web password does not work there)

  6 tracks · 3 albums · 3 artists (one multi-artist, one untracked)

Start it with:

  WAVEFLOW_DATA_DIR=$DATA_DIR ${SERVER[*]} serve

Then authenticate over /api/v2/auth/login, or reuse the token printed above as
a Bearer credential. Delete $DATA_DIR when you are done.
──────────────────────────────────────────────────────────────
EOF
