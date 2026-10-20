#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
fixtures="$repository_root/tests/fixtures/homebrew"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

bash "$repository_root/scripts/homebrew-formula" 0.2.0 "$fixtures/SHA256SUMS-v0.2.0" > "$temporary_directory/tuivir.rb"
diff -u "$fixtures/tuivir-v0.2.0.rb" "$temporary_directory/tuivir.rb"

grep -v aarch64-apple-darwin "$fixtures/SHA256SUMS-v0.2.0" > "$temporary_directory/SHA256SUMS"
if bash "$repository_root/scripts/homebrew-formula" 0.2.0 "$temporary_directory/SHA256SUMS" >/dev/null 2>&1; then
  echo 'formula unexpectedly rendered without an Apple Silicon checksum' >&2
  exit 1
fi

for version in 0.2 v0.2.0 '0.2.0";system("id");"' '0.2.0#{`id`}'; do
  sed "s/tuivir-v0\.2\.0-/tuivir-v$version-/" "$fixtures/SHA256SUMS-v0.2.0" > "$temporary_directory/SHA256SUMS"
  if bash "$repository_root/scripts/homebrew-formula" "$version" "$temporary_directory/SHA256SUMS" >/dev/null 2>&1; then
    echo "formula unexpectedly rendered for version $version" >&2
    exit 1
  fi
done
