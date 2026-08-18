#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

printf 'int main(void) { return 0; }' | cc -x c -o "$temporary_directory/dynamic" -
printf 'int main(void) { return 0; }' | cc -static -x c -o "$temporary_directory/static" -

bash "$repository_root/scripts/verify-static-elf" "$temporary_directory/static"

if bash "$repository_root/scripts/verify-static-elf" "$temporary_directory/dynamic" >/dev/null 2>&1; then
  echo 'dynamic executable unexpectedly passed static verification' >&2
  exit 1
fi
