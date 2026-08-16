#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
prepare_release="$repository_root/scripts/prepare-release"
temporary_root="$(mktemp -d)"
trap 'rm -rf "$temporary_root"' EXIT

new_repository() {
  local name="$1"
  local repository="$temporary_root/$name"
  mkdir -p "$repository/src"
  cat >"$repository/Cargo.toml" <<'EOF'
[package]
name = "tuivir"
version = "0.1.0"
edition = "2024"
EOF
  : >"$repository/src/lib.rs"
  git -C "$repository" init --initial-branch=master --quiet
  git -C "$repository" config user.email release-tests@example.invalid
  git -C "$repository" config user.name 'Release tests'
  git -C "$repository" add .
  git -C "$repository" commit --quiet -m 'Create fixture'
  printf '%s\n' "$repository"
}

assert_rejected() {
  local repository="$1"
  local version="$2"
  shift 2
  if (cd "$repository" && env -u GITHUB_REF "$@" bash "$prepare_release" "$version"); then
    echo "expected $version to be rejected" >&2
    exit 1
  fi
}

first_release="$(new_repository first-release)"
(cd "$first_release" && env -u GITHUB_REF bash "$prepare_release" 0.1.0)
test "$(git -C "$first_release" branch --show-current)" = release/v0.1.0
test "$(cat "$first_release/docs/releases/v0.1.0.md")" = $'# Tuivir v0.1.0\n\n## Changes\n\n- Create fixture'
test "$(git -C "$first_release" log -1 --format=%s)" = 'Prepare v0.1.0 release'

later_release="$(new_repository later-release)"
git -C "$later_release" tag v0.1.0
printf 'A documented change.\n' >"$later_release/README.md"
git -C "$later_release" add README.md
git -C "$later_release" commit --quiet -m 'Document later release'
(cd "$later_release" && env -u GITHUB_REF bash "$prepare_release" 0.2.0)
grep -Fx 'version = "0.2.0"' "$later_release/Cargo.toml"
grep -A1 -Fx 'name = "tuivir"' "$later_release/Cargo.lock" | grep -Fx 'version = "0.2.0"'
test "$(cat "$later_release/docs/releases/v0.2.0.md")" = $'# Tuivir v0.2.0\n\n## Changes\n\n- Document later release'

invalid_version="$(new_repository invalid-version)"
assert_rejected "$invalid_version" v0.1.0 env

existing_version="$(new_repository existing-version)"
git -C "$existing_version" tag v0.1.0
assert_rejected "$existing_version" 0.1.0 env

non_increasing="$(new_repository non-increasing)"
git -C "$non_increasing" tag v0.2.0
assert_rejected "$non_increasing" 0.1.0 env

concurrent_release="$(new_repository concurrent-release)"
assert_rejected "$concurrent_release" 0.1.0 env RELEASE_PREPARATION_OPEN_RELEASE_PR=true

stale_master="$(new_repository stale-master)"
git -C "$stale_master" commit --allow-empty --quiet -m 'Advance master'
git -C "$stale_master" switch --quiet --detach HEAD~1
assert_rejected "$stale_master" 0.1.0 env GITHUB_REF=refs/heads/master

rerun="$(new_repository rerun)"
(cd "$rerun" && env -u GITHUB_REF bash "$prepare_release" 0.1.0)
git -C "$rerun" switch --quiet master
assert_rejected "$rerun" 0.1.0 env

echo 'release preparation integration checks passed'
