#!/usr/bin/env python3
"""Update the two macOS URLs and hashes from the published SHA256SUMS."""
import argparse
import re
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("tag")
parser.add_argument("checksums", type=Path)
parser.add_argument("formula", type=Path)
args = parser.parse_args()
if not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", args.tag):
    raise SystemExit("Expected a stable vX.Y.Z tag")
version = args.tag[1:]
original = args.formula.read_text()
old = re.search(r'^  version "([0-9]+\.[0-9]+\.[0-9]+)"$', original, re.MULTILINE)[1]
if tuple(map(int, version.split("."))) < tuple(map(int, old.split("."))):
    raise SystemExit("Refusing to downgrade the Homebrew formula")
sums = dict(line.split()[::-1] for line in args.checksums.read_text().splitlines())
updated = original
for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
    digest = sums[f"tuivir-{args.tag}-{target}.tar.gz"]
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise SystemExit(f"Invalid checksum for {target}")
    url = f"https://github.com/saif97/Tuivir/releases/download/v{old}/tuivir-v{old}-{target}.tar.gz"
    updated, count = re.subn('url "' + re.escape(url) + r'"\s+sha256 "[0-9a-f]{64}"',
                            f'url "{url}"\n      sha256 "{digest}"', updated)
    if count != 1:
        raise SystemExit(f"Expected one upstream URL/hash for {target}")
updated = updated.replace(f'  version "{old}"', f'  version "{version}"')
updated = updated.replace(f'v{old}/tuivir-v{old}-', f'v{version}/tuivir-v{version}-')
updated = updated.replace(f'assert_equal "{old}\\n"', f'assert_equal "{version}\\n"')
if version == old and updated != original:
    raise SystemExit("Refusing to change hashes for an existing formula version")
args.formula.write_text(updated)
