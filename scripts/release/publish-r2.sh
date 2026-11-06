#!/bin/sh
# Copies a release's files onto the Cloudflare R2 bucket served at
# download.waveflow.app, under server/releases/<tag>/, then reads every one
# back from that public address and checks it against the SHA256SUMS beside it.
#
#   sh scripts/release/publish-r2.sh <dir> <tag>
#   sh scripts/release/publish-r2.sh --check
#
# <dir> holds what the release attaches: the five archives, the FFmpeg sources
# and SHA256SUMS. `--check` prints `configured` when the four R2_* variables are
# set, `absent` when none is, and fails naming the missing ones when only some
# are: a half-configured mirror is a mistake, not a choice.
#
# Run by release.yml after the GitHub release, and by .gitlab-ci.yml, where R2
# is the only download host. docs/guides/releasing.md says what to create.
#
#   R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY  an R2 API token, Object Read & Write
#   R2_ACCOUNT_ID                           the Cloudflare account id, or its S3 endpoint
#   R2_BUCKET                               the bucket, shared with the desktop app
#   R2_PUBLIC_URL                           default https://download.waveflow.app
#
# The prefix keeps the server's files apart from the desktop app's, which owns
# releases/<tag>/ and the channel manifests at the root of the same bucket: both
# repositories tag v2.x, and a shared prefix would let one overwrite the other.
#
# Needs the AWS CLI, curl and sha256sum.

set -eu

die() {
    echo "$*" >&2
    exit 1
}

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

[ $# -eq 2 ] || die "usage: $0 <dir> <tag> | --check"
dir=$1
tag=$2
[ "$(check)" = configured ] || die "R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_ACCOUNT_ID and R2_BUCKET are required"
[ -s "$dir/SHA256SUMS" ] || die "$dir/SHA256SUMS is missing"
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
echo "$public/$prefix/ serves all $(echo "$files" | wc -w | tr -d ' ') files, checked against SHA256SUMS"
