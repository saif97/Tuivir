#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

cp "$repository_root/Cargo.toml" "$repository_root/Cargo.lock" "$temporary_directory"
cd "$temporary_directory"

bash "$repository_root/scripts/set-release-version" 9.8.7

test "$(bash "$repository_root/scripts/release-version")" = 9.8.7
test "$(awk 'package && /^version = / { print; exit } $0 == "name = \"tuivir\"" { package = 1 }' Cargo.lock)" = 'version = "9.8.7"'
test "$(diff "$repository_root/Cargo.toml" Cargo.toml | grep -c '^[<>]')" -eq 2
test "$(diff "$repository_root/Cargo.lock" Cargo.lock | grep -c '^[<>]')" -eq 2

for version in 9.8 v9.8.7 '9.8.7"' '9.8.7\n[evil]' --9.8.7; do
  cp "$repository_root/Cargo.toml" "$repository_root/Cargo.lock" .
  if bash "$repository_root/scripts/set-release-version" "$version" >/dev/null 2>&1; then
    echo "release version unexpectedly set: $version" >&2
    exit 1
  fi
  cmp -s "$repository_root/Cargo.toml" Cargo.toml && cmp -s "$repository_root/Cargo.lock" Cargo.lock || {
    echo "rejected release version changed Cargo files: $version" >&2
    exit 1
  }
done

cp "$repository_root/Cargo.toml" .
sed 's/^name = "tuivir"$/name = "other"/' "$repository_root/Cargo.lock" > Cargo.lock
if bash "$repository_root/scripts/set-release-version" 9.8.7 >/dev/null 2>&1; then
  echo 'release version unexpectedly set without a tuivir package in Cargo.lock' >&2
  exit 1
fi
