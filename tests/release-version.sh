#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
cd "$repository_root"

cargo_version="$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "tuivir") | .version')"
test "$(bash scripts/release-version "v$cargo_version")" = "$cargo_version"

mismatched_tag='v9.9.9'
[[ "$cargo_version" != '9.9.9' ]] || mismatched_tag='v8.8.8'

for tag in 0.1.0 v0.1 v01.0.0 v0.1.0-beta "$mismatched_tag"; do
  if bash scripts/release-version "$tag" >/dev/null 2>&1; then
    echo "release version unexpectedly accepted: $tag" >&2
    exit 1
  fi
done
