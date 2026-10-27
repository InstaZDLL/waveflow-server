#!/bin/sh
# Holds the Rust tree to the shape the restructuring gave it.
#
# - A source file under `src/` stays under 800 lines. Past that it holds more
#   than one responsibility; split it along its seams into a module directory.
# - A unit-test file under `src/` (always named `tests.rs`, beside the module
#   it tests) stays under 1,000 lines, as does any file under `tests/`.
# - Unit tests never sit inline in the code: `#[cfg(test)] mod tests;` names a
#   `tests.rs` beside the module, and no file under `src/` opens `mod tests {`.
#
# `sh scripts/check-rust-layout.sh`, from anywhere: it moves to the
# repository root first, because run from a subfolder its `find` met no file
# and it passed having measured nothing. POSIX sh and coreutils only, so it
# runs the same on a runner and on a laptop.

set -eu

cd "$(dirname "$0")/.."

SOURCE_LIMIT=800
TEST_LIMIT=1000

# Files allowed past their limit, one path per line, each with its reason.
#
# tests/subsonic_contract/compatibility.rs: one test, the frozen v2.0-beta
# compatibility walk, which drives a single seeded instance through every
# method a validated client calls. Splitting it means splitting that test,
# which changes what it proves; it stays whole.
ALLOWLIST="
tests/subsonic_contract/compatibility.rs
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

for file in $(find src tests -name '*.rs' | sort); do
    case "$file" in
        tests/*) check_size "$file" "$TEST_LIMIT" ;;
        */tests.rs) check_size "$file" "$TEST_LIMIT" ;;
        *) check_size "$file" "$SOURCE_LIMIT" ;;
    esac
done

inline=$(grep -rnE 'mod tests[[:space:]]*\{' src || true)
if [ -n "$inline" ]; then
    echo "$inline" | while IFS= read -r hit; do
        echo "::error file=${hit%%:*}::inline test module; move it to a tests.rs beside the module: $hit"
    done
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo "Rust layout: every file within its limit, no inline test module."
