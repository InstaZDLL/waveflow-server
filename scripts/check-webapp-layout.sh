#!/bin/sh
# Holds the web client to the shape the restructuring gave it.
#
# - A code file under `webapp/src/` (`.ts` or `.tsx`, not a test) stays under
#   600 lines. Past that it holds more than one responsibility; split it along
#   its seams, a module directory with an `index` if others import it.
# - A stylesheet under `webapp/src/` stays under 800 lines. The entry sheets
#   are ordered lists of `@import`; a part that grows past the limit becomes
#   two parts, in cascade order.
# - A unit test under `webapp/src/` (`*.test.ts` or `*.test.tsx`, beside the
#   file it tests) stays under 1,000 lines, as does any file under
#   `webapp/e2e/`, specs and fixtures alike.
#
# Run from the repository root: `sh scripts/check-webapp-layout.sh`. POSIX sh
# and coreutils only, so it runs the same on a runner and on a laptop.

set -eu

CODE_LIMIT=600
STYLE_LIMIT=800
TEST_LIMIT=1000

# Files allowed past their limit, one path per line, each with its reason.
ALLOWLIST="
"

failed=0

allowed() {
    case "$ALLOWLIST" in
        *"
$1
"*) return 0 ;;
    esac
    return 1
}

report() {
    # `::error` is a GitHub annotation; elsewhere it reads as a plain line.
    echo "::error file=$1::$2"
    failed=1
}

check_size() {
    file=$1
    limit=$2
    lines=$(wc -l < "$file" | tr -d ' ')
    if [ "$lines" -gt "$limit" ] && ! allowed "$file"; then
        report "$file" "$file has $lines lines; the limit is $limit."
    fi
}

# One path per line, read whole: a `for` over `$(find)` would split a name
# with a space in it into pieces that name no file, and the file itself would
# go unmeasured. Fed by a here-document rather than a pipe, so the loop runs
# in this shell and what it sets in `failed` survives it.
while IFS= read -r file; do
    [ -n "$file" ] || continue
    case "$file" in
        *.test.ts | *.test.tsx) check_size "$file" "$TEST_LIMIT" ;;
        *.css) check_size "$file" "$STYLE_LIMIT" ;;
        *) check_size "$file" "$CODE_LIMIT" ;;
    esac
done <<EOF
$(find webapp/src -type f \( -name '*.ts' -o -name '*.tsx' -o -name '*.css' \) | sort)
EOF

while IFS= read -r file; do
    [ -n "$file" ] || continue
    check_size "$file" "$TEST_LIMIT"
done <<EOF
$(find webapp/e2e -type f | sort)
EOF

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo "Webapp layout: every file within its limit."
