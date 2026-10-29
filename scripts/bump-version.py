#!/usr/bin/env python3
"""Bump Tuivir's package version without changing dependency versions."""
import argparse
import re
import tomllib
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("bump", choices=("patch", "minor", "major"))
kind = parser.parse_args().bump
manifest = Path("Cargo.toml").read_text()
old = tomllib.loads(manifest)["package"]["version"]
if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", old):
    raise SystemExit("Expected a stable X.Y.Z package version")
parts = list(map(int, old.split(".")))
index = {"major": 0, "minor": 1, "patch": 2}[kind]
parts[index] += 1
parts[index + 1:] = [0] * (2 - index)
version = ".".join(map(str, parts))
changes = {}
for filename, prefix in (("Cargo.toml", r"(?ms)(^\[package\]\n.*?^version = \")"),
                         ("Cargo.lock", r"(?m)(^name = \"tuivir\"\nversion = \")")):
    text, count = re.subn(prefix + re.escape(old) + '(")',
                          lambda match: match[1] + version + match[2],
                          Path(filename).read_text())
    if count != 1:
        raise SystemExit(f"Expected one matching Tuivir version in {filename}")
    changes[filename] = text
for filename, text in changes.items():
    Path(filename).write_text(text)
print(version)
