#!/bin/sh
# Copies a release's files onto the Cloudflare R2 bucket served at
# download.waveflow.app, under server/releases/<tag>/, then reads every one
# back from that public address and checks it against the SHA256SUMS beside it.
#
#   sh scripts/release/publish-r2.sh <dir> <tag> [<notes.md>]
#   sh scripts/release/publish-r2.sh --check
#
# <dir> holds what the release attaches: the five archives, the FFmpeg sources
# and SHA256SUMS. <notes.md>, when given, is the release page as published,
# kept beside the files as RELEASE-NOTES.md: what GitHub alone holds (the list
# of changes it generates for a pre-release, an advisory added by hand) then
# outlives it, without waiting for a token GitLab's Free tier does not offer.
# `--check` prints `configured` when the four R2_* variables are set, `absent`
# when none is, and fails naming the missing ones when only some are: a
# half-configured mirror is a mistake, not a choice.
#
# Last, the channel manifests the update check reads (RFC-028 decision 1):
# server/beta.json for every release, server/stable.json for a stable one,
# written by channel-manifest.py, which leaves a newer one in place. They name
# the release page, RELEASE_PAGE_URL, which the caller sets.
#
# Run by release.yml after the GitHub release, and by .gitlab-ci.yml, where R2
# is the only download host. docs/guides/releasing.md says what to create.
#
#   R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY  an R2 API token, Object Read & Write
#   R2_ACCOUNT_ID                           the Cloudflare account id, or its S3 endpoint
#   R2_BUCKET                               the bucket, shared with the desktop app
#   R2_PUBLIC_URL                           default https://download.waveflow.app
#   RELEASE_PAGE_URL                        the release's page, for the manifests
#
# The prefix keeps the server's files apart from the desktop app's, which owns
# releases/<tag>/ and the channel manifests at the root of the same bucket: both
# repositories tag v2.x, and a shared prefix would let one overwrite the other.
#
# Needs the AWS CLI, curl, sha256sum and python3.

set -eu

# Prints its arguments on stderr and stops the script with status 1.
die() {
    echo "$*" >&2
    exit 1
}

# Prints `configured` when the four R2_* variables are all set and `absent`
# when none is; dies naming the missing ones when only some are.
check() {
    set_vars=
    missing=
    for name in R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_ACCOUNT_ID R2_BUCKET; do
        eval "value=\${$name:-}"
        if [ -n "$value" ]; then
            set_vars="$set_vars $name"
        else
            missing="$missing $name"
        fi
    done
    if [ -z "$set_vars" ]; then
        echo absent
    elif [ -z "$missing" ]; then
        echo configured
    else
        die "R2 is half configured: missing$missing"
    fi
}

if [ "${1:-}" = --check ]; then
    check
    exit 0
fi

[ $# -eq 2 ] || [ $# -eq 3 ] || die "usage: $0 <dir> <tag> [<notes.md>] | --check"
dir=$1
tag=$2
notes=${3:-}
[ -z "$notes" ] || [ -s "$notes" ] || die "$notes is missing or empty"
[ "$(check)" = configured ] || die "R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_ACCOUNT_ID and R2_BUCKET are required"
[ -s "$dir/SHA256SUMS" ] || die "$dir/SHA256SUMS is missing"
[ -n "${RELEASE_PAGE_URL:-}" ] || die "RELEASE_PAGE_URL is required: the channel manifests link the release page"
case "$tag" in
    v[0-9]*) ;;
    *) die "not a release tag: $tag" ;;
esac

public=${R2_PUBLIC_URL:-https://download.waveflow.app}
public=${public%/}
prefix=server/releases/$tag

# What is uploaded is what SHA256SUMS covers, plus itself: a stray file in the
# folder would otherwise be published under the release's name unchecked.
(cd "$dir" && sha256sum -c SHA256SUMS --quiet) || die "$dir does not match its SHA256SUMS"
files="SHA256SUMS $(awk '{ sub(/^\*/, "", $2); print $2 }' "$dir/SHA256SUMS")"

export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION=auto
# R2 implements the checksums S3 requires, not every one the CLI adds by
# default since 2.23; asking only for the required ones keeps uploads working
# whichever version the runner has.
export AWS_REQUEST_CHECKSUM_CALCULATION=when_required
export AWS_RESPONSE_CHECKSUM_VALIDATION=when_required
# The account id, or the S3 endpoint the dashboard shows right beside it,
# https://<id>.r2.cloudflarestorage.com: the secret has been set to the second
# once already, and the upload then failed on DNS with nothing saying why.
account=${R2_ACCOUNT_ID#https://}
account=${account%%.*}
case "$account" in
    *[!0-9a-f]* | "") die "R2_ACCOUNT_ID is neither an account id nor https://<id>.r2.cloudflarestorage.com" ;;
esac
[ "${#account}" -eq 32 ] || die "R2_ACCOUNT_ID is neither an account id nor https://<id>.r2.cloudflarestorage.com"
endpoint="https://$account.r2.cloudflarestorage.com"

for name in $files; do
    case "$name" in
        *.zip) type=application/zip ;;
        *.tar.gz) type=application/gzip ;;
        *) type="text/plain; charset=utf-8" ;;
    esac
    aws s3 cp "$dir/$name" "s3://$R2_BUCKET/$prefix/$name" \
        --endpoint-url "$endpoint" \
        --content-type "$type" \
        --cache-control "public, max-age=300" \
        --only-show-errors
    echo "uploaded $prefix/$name"
done
if [ -n "$notes" ]; then
    aws s3 cp "$notes" "s3://$R2_BUCKET/$prefix/RELEASE-NOTES.md" \
        --endpoint-url "$endpoint" \
        --content-type "text/markdown; charset=utf-8" \
        --cache-control "public, max-age=300" \
        --only-show-errors
    echo "uploaded $prefix/RELEASE-NOTES.md"
fi

# Read back through the domain people download from, not the S3 endpoint: the
# promise is that the address works, and a bucket with no domain, or a domain
# on another bucket, uploads fine and serves nothing. The query string steps
# past Cloudflare's cache, which may still hold a retried release's old file.
check_dir=$(mktemp -d)
trap 'rm -rf "$check_dir"' EXIT
stamp=$(date +%s)
for name in $files; do
    curl --fail --silent --show-error --location --retry 3 \
        --output "$check_dir/$name" "$public/$prefix/$name?v=$stamp"
done
# Against the local SHA256SUMS, not the one served: a domain answering another
# release's set, consistent with itself, would pass a check against its own.
cmp -s "$dir/SHA256SUMS" "$check_dir/SHA256SUMS" ||
    die "$public/$prefix/SHA256SUMS is not the one uploaded"
cp "$dir/SHA256SUMS" "$check_dir/SHA256SUMS.uploaded"
(cd "$check_dir" && sha256sum -c SHA256SUMS.uploaded --quiet) ||
    die "the files served at $public/$prefix/ do not match the uploaded SHA256SUMS"
if [ -n "$notes" ]; then
    curl --fail --silent --show-error --location --retry 3 \
        --output "$check_dir/RELEASE-NOTES.md" "$public/$prefix/RELEASE-NOTES.md?v=$stamp"
    cmp -s "$notes" "$check_dir/RELEASE-NOTES.md" ||
        die "$public/$prefix/RELEASE-NOTES.md is not the page uploaded"
    echo "$public/$prefix/RELEASE-NOTES.md serves the page uploaded"
fi
echo "$public/$prefix/ serves all $(echo "$files" | wc -w | tr -d ' ') files, checked against SHA256SUMS"

# The channel manifests, after the files they point to are known to be served.
# A pre-release moves beta only; a stable release moves both, so a server on a
# pre-release hears of the stable version that follows it.
here=$(cd "$(dirname "$0")" && pwd)
case "${tag#v}" in
    *-*) channels=beta ;;
    *) channels="stable beta" ;;
esac
notes_url=-
[ -z "$notes" ] || notes_url="$public/$prefix/RELEASE-NOTES.md"
published=$(date -u +%Y-%m-%dT%H:%M:%SZ)
for channel in $channels; do
    current=-
    code=$(curl --silent --show-error --location --retry 3 --output "$check_dir/current-$channel.json" \
        --write-out '%{http_code}' "$public/server/$channel.json?v=$stamp")
    case "$code" in
        200) current="$check_dir/current-$channel.json" ;;
        404) ;;
        *) die "$public/server/$channel.json answered $code: the published manifest could not be read" ;;
    esac
    status=0
    python3 "$here/channel-manifest.py" "$tag" "$RELEASE_PAGE_URL" "$notes_url" "$published" "$current" \
        > "$check_dir/$channel.json" || status=$?
    case "$status" in
        0) ;;
        3) echo "server/$channel.json names a newer version: left as it is"; continue ;;
        *) die "the $channel manifest could not be written" ;;
    esac
    aws s3 cp "$check_dir/$channel.json" "s3://$R2_BUCKET/server/$channel.json" \
        --endpoint-url "$endpoint" \
        --content-type "application/json" \
        --cache-control "no-cache" \
        --only-show-errors
    curl --fail --silent --show-error --location --retry 3 \
        --output "$check_dir/served-$channel.json" "$public/server/$channel.json?v=$stamp-read"
    cmp -s "$check_dir/$channel.json" "$check_dir/served-$channel.json" ||
        die "$public/server/$channel.json is not the manifest uploaded"
    echo "$public/server/$channel.json now names ${tag#v}"
done
