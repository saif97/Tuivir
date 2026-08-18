#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
cd "$repository_root"

test "$(bash scripts/release-version v0.1.0)" = '0.1.0'

for tag in 0.1.0 v0.1 v01.0.0 v0.1.0-beta v9.9.9; do
  if bash scripts/release-version "$tag" >/dev/null 2>&1; then
    echo "release version unexpectedly accepted: $tag" >&2
    exit 1
  fi
done
