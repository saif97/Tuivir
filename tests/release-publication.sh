#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
publish_release="$repository_root/scripts/publish-release"
temporary_root="$(mktemp -d)"
trap 'rm -rf "$temporary_root"' EXIT

new_release_repository() {
  local name="$1"
  local repository="$temporary_root/$name/repository"
  local remote="$temporary_root/$name/origin.git"
  mkdir -p "$repository/src" "$temporary_root/$name/artifacts" "$temporary_root/$name/gh"
  git init --bare --quiet "$remote"
  git -C "$repository" init --initial-branch=master --quiet
  git -C "$repository" config user.email release-tests@example.invalid
  git -C "$repository" config user.name 'Release tests'
  git -C "$repository" remote add origin "$remote"
  cat >"$repository/Cargo.toml" <<'EOF'
[package]
name = "tuivir"
version = "0.1.0"
edition = "2024"
EOF
  : >"$repository/src/lib.rs"
  git -C "$repository" add .
  git -C "$repository" commit --quiet -m 'Create fixture'
  git -C "$repository" push --quiet --set-upstream origin master
  git -C "$repository" switch --quiet --create release/v0.1.0
  mkdir -p "$repository/docs/releases"
  cat >"$repository/docs/releases/v0.1.0.md" <<'EOF'
# Tuivir v0.1.0

## Changes

- Create fixture
EOF
  cargo generate-lockfile --manifest-path "$repository/Cargo.toml" --quiet
  git -C "$repository" add Cargo.lock docs/releases/v0.1.0.md
  git -C "$repository" commit --quiet -m 'Prepare v0.1.0 release'
  git -C "$repository" switch --quiet master
  git -C "$repository" merge --no-ff --quiet release/v0.1.0 -m 'Merge release v0.1.0'
  git -C "$repository" push --quiet origin master
  for target in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl; do
    printf '%s\n' "$target" >"$temporary_root/$name/artifacts/tuivir-v0.1.0-$target.tar.gz"
  done
  (cd "$temporary_root/$name/artifacts" && sha256sum ./*.tar.gz >SHA256SUMS)
  printf '%s\n' "$repository"
}

publish() {
  local repository="$1"
  local name="$2"
  (
    cd "$repository"
    RELEASE_GH="$repository_root/tests/fixtures/fake-gh" \
      FAKE_GH_STATE="$temporary_root/$name/gh" \
      RELEASE_ARTIFACT_DIR="$temporary_root/$name/artifacts" \
      RELEASE_BRANCH=release/v0.1.0 \
      bash "$publish_release" 0.1.0
  )
}

release="$(new_release_repository valid)"
publish "$release" valid
test "$(git -C "$release" cat-file -t refs/tags/v0.1.0)" = tag
test "$(git -C "$release" rev-list -n 1 v0.1.0)" = "$(git -C "$release" rev-parse HEAD)"
test "$(find "$temporary_root/valid/gh/releases/v0.1.0/assets" -type f | wc -l)" -eq 4
publish "$release" valid

partial="$(new_release_repository partial)"
publish "$partial" partial
rm "$temporary_root/partial/gh/releases/v0.1.0/assets/tuivir-v0.1.0-x86_64-apple-darwin.tar.gz"
publish "$partial" partial
test -f "$temporary_root/partial/gh/releases/v0.1.0/assets/tuivir-v0.1.0-x86_64-apple-darwin.tar.gz"

conflicting_tag="$(new_release_repository conflicting-tag)"
git -C "$conflicting_tag" tag -a v0.1.0 HEAD~1 -m 'Wrong release target'
if publish "$conflicting_tag" conflicting-tag; then
  echo 'expected a conflicting tag to be rejected' >&2
  exit 1
fi

echo 'release publication integration checks passed'
