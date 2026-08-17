#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
temporary_root="$(mktemp -d)"
trap 'rm -rf "$temporary_root"' EXIT
artifacts="$temporary_root/artifacts"
mkdir "$artifacts"

for target in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl; do
  printf '%s\n' "$target" >"$artifacts/tuivir-v0.1.0-$target.tar.gz"
done
(cd "$artifacts" && sha256sum ./*.tar.gz >SHA256SUMS)

bash "$repository_root/scripts/generate-homebrew-formula" 0.1.0 "$artifacts" "$temporary_root/tuivir.rb"
bash "$repository_root/scripts/generate-aur-package" 0.1.0 "$artifacts" "$temporary_root/aur"

grep -Fx 'class Tuivir < Formula' "$temporary_root/tuivir.rb"
grep -F 'tuivir-v0.1.0-aarch64-apple-darwin.tar.gz' "$temporary_root/tuivir.rb"
grep -F 'tuivir-v0.1.0-x86_64-apple-darwin.tar.gz' "$temporary_root/tuivir.rb"
grep -F 'bin.install Dir["*/tuivir"].first' "$temporary_root/tuivir.rb"
grep -F 'pkgname=tuivir-bin' "$temporary_root/aur/PKGBUILD"
grep -F 'tuivir-v0.1.0-x86_64-unknown-linux-musl.tar.gz' "$temporary_root/aur/PKGBUILD"
grep -F 'provides=("tuivir=${pkgver}")' "$temporary_root/aur/PKGBUILD"
grep -F "conflicts=('tuivir')" "$temporary_root/aur/PKGBUILD"
! grep -E 'docker|incus|sbx' "$temporary_root/aur/PKGBUILD"
grep -Fx 'pkgname = tuivir-bin' "$temporary_root/aur/.SRCINFO"

echo 'release package generation checks passed'
