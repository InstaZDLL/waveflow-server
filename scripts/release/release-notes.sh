#!/bin/sh
# Writes the body of a release page: the notes the tag carries, when it has
# some, then the download table and the pull commands.
#
#   sh scripts/release/release-notes.sh <tag> <download-base> <image> <hub-image> > notes.md
#
# <download-base> is the address the archives are downloaded from, the files'
# names appended: the GitHub release on GitHub, the R2 copy on GitLab.
# <image> the first registry's image (GHCR, or GitLab's registry); <hub-image>
# Docker Hub's, without `docker.io/`. The FFmpeg sources' name is read from
# release-assets/, where both pipelines put what they attach.
#
# Exits 2, with the page written, when the tag has no notes: the caller then
# asks its platform to append the list of merged changes, as GitHub does with
# `--generate-notes`. A pre-release has the list either way; release.yml
# decides that.

set -eu

[ $# -eq 4 ] || {
    echo "usage: $0 <tag> <download-base> <image> <hub-image>" >&2
    exit 1
}
tag=$1
base=${2%/}
image=$3
hub_image=$4
version=${tag#v}
case "$version" in
    *-*) channel=beta ;;
    *) channel=latest ;;
esac

status=0
notes_file="docs/releases/$tag.md"
if [ -s "$notes_file" ]; then
    cat "$notes_file"
    printf '\n'
else
    status=2
fi

sources=$(basename release-assets/ffmpeg-*-sources.tar.gz)
# printf with single-quoted formats rather than a heredoc: the body is mostly
# backticks, and every one would be a command substitution in an unquoted
# here-document.
printf '## Download\n\nThe server for one computer, without Docker: extract the archive and start `waveflow-server` inside it. Each archive holds the server, its own FFmpeg and a `README.txt` to read first.\n\n'
printf '| System | Archive |\n| --- | --- |\n'
while IFS='|' read -r system suffix; do
    file="waveflow-server-$version-$suffix"
    printf '| %s | [%s](%s/%s) |\n' "$system" "$file" "$base" "$file"
done <<'ARCHIVES'
Windows (x86_64)|windows-x86_64.zip
macOS (Apple silicon)|macos-arm64.zip
macOS (Intel)|macos-x86_64.zip
Linux (x86_64)|linux-x86_64.tar.gz
Linux (arm64)|linux-arm64.tar.gz
ARCHIVES
printf '\nThe Windows executables carry a self-signed certificate and the macOS ones an ad-hoc signature, which neither system trusts, so both warn on the first launch: `README.txt` says what to click past. `SHA256SUMS` holds the checksums of them all, and `%s` the source of the FFmpeg they carry.\n\n' "$sources"
printf '## Container image\n\n```bash\ndocker pull %s:%s\n```\n\nThe same image, digest for digest, is on Docker Hub:\n\n```bash\ndocker pull %s:%s\n```\n\nAlso published as `%s` in both, for `linux/amd64` and `linux/arm64` (a Raspberry Pi 4 or 5 on a 64-bit system).\n' \
    "$image" "$version" "$hub_image" "$version" "$channel"

exit "$status"
