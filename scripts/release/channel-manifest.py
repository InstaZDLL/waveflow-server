#!/usr/bin/env python3
"""Writes a channel manifest the update check reads (RFC-028 decision 1).

    python3 scripts/release/channel-manifest.py <tag> <page> <notes|-> <published> <current|->
    python3 scripts/release/channel-manifest.py --self-test

Prints the manifest for <tag> on stdout, or exits 3 and prints nothing when
<current> — the manifest already published, a file, `-` for none — names a
newer version: a manifest never moves backwards, so a patch to an older line or
a retried old tag cannot tell every server out there to go back. The same
version again goes through, so a retry can correct a page address.

The precedence is Semantic Versioning 2.0.0 §11, the rule the server applies to
the same strings with the `semver` crate: a pre-release sorts before its
release, identifiers compare numerically when both are numbers, a number sorts
before a word, and a shorter list of equal identifiers sorts first. Build
metadata is ignored. A version that is not MAJOR.MINOR.PATCH with an optional
pre-release is refused, here as there.
"""

import json
import re
import sys

KEEP = 3
VERSION = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$",
    re.ASCII,
)


def parse(version):
    """Splits a version into its numbers and pre-release identifiers, or
    raises ValueError for anything Semantic Versioning does not allow."""
    # fullmatch, not match: `$` alone lets a trailing newline through.
    match = VERSION.fullmatch(version)
    if not match:
        raise ValueError(f"not a semantic version: {version!r}")
    numbers = tuple(int(part) for part in match.group(1, 2, 3))
    pre = tuple(match.group(4).split(".")) if match.group(4) else ()
    return numbers, pre


def compare(a, b):
    """Returns -1, 0 or 1 as version a sorts before, with or after version b."""
    (numbers_a, pre_a), (numbers_b, pre_b) = parse(a), parse(b)
    if numbers_a != numbers_b:
        return -1 if numbers_a < numbers_b else 1
    if not pre_a or not pre_b:
        # The release sorts after its pre-releases: an empty list is the larger.
        return (not pre_a) - (not pre_b)
    for x, y in zip(pre_a, pre_b):
        if x == y:
            continue
        x_number, y_number = x.isdigit(), y.isdigit()
        if x_number and y_number:
            return -1 if int(x) < int(y) else 1
        if x_number != y_number:
            return -1 if x_number else 1
        return -1 if x < y else 1
    return (len(pre_a) > len(pre_b)) - (len(pre_a) < len(pre_b))


def self_test():
    """The precedence examples of Semantic Versioning §11, and the refusals."""
    ordered = [
        "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta",
        "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "2.0.0",
        "2.1.0-rc.1", "2.1.0", "2.1.1", "2.1.2", "2.10.0",
    ]
    for low, high in zip(ordered, ordered[1:]):
        assert compare(low, high) == -1, (low, high)
        assert compare(high, low) == 1, (high, low)
    assert compare("2.1.2", "2.1.2+build.5") == 0
    for bad in ["2.1", "v2.1.2", "02.1.2", "2.1.2-", "2.1.2-01", " 2.1.2", "2.1.2\n", "２.1.2"]:
        try:
            parse(bad)
        except ValueError:
            continue
        raise AssertionError(f"accepted {bad!r}")
    print("channel-manifest self-test: ok")


def main():
    """Prints the manifest of the tag named, or exits 3 when the published
    one is newer."""
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    if len(sys.argv) != 6:
        sys.exit(__doc__.strip().splitlines()[2].strip())
    tag, page, notes, published, current = sys.argv[1:]
    if not tag.startswith("v"):
        sys.exit(f"not a release tag: {tag}")
    version = tag[1:]
    parse(version)
    if current != "-":
        with open(current, encoding="utf-8") as handle:
            published_version = json.load(handle)["version"]
        if compare(published_version, version) > 0:
            print(f"{published_version} is already published, newer than {version}: kept", file=sys.stderr)
            sys.exit(KEEP)
    manifest = {
        "version": version,
        "tag": tag,
        "published_at": published,
        "page": page,
        "notes": None if notes == "-" else notes,
    }
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
