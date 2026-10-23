#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
export GIT_AUTHOR_NAME=Test GIT_AUTHOR_EMAIL=test@example.com
export GIT_COMMITTER_NAME=Test GIT_COMMITTER_EMAIL=test@example.com

origin="$temporary_directory/origin.git"
git init --quiet --bare --initial-branch=master "$origin"
git clone --quiet "$origin" "$temporary_directory/checkout" 2>/dev/null
cd "$temporary_directory/checkout"
cp "$repository_root/Cargo.toml" "$repository_root/Cargo.lock" .
bash "$repository_root/scripts/set-release-version" 0.2.0
git add Cargo.toml Cargo.lock
git commit --quiet --message 'Release v0.2.0'
git tag v0.2.0
git push --quiet origin master v0.2.0

mkdir "$temporary_directory/bin"
cat > "$temporary_directory/bin/gh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$@" > "$temporary_directory/gh-arguments"
EOF
chmod +x "$temporary_directory/bin/gh"
export PATH="$temporary_directory/bin:$PATH"

git commit --quiet --allow-empty --message 'Local work that is not pushed'
echo 'uncommitted' >> Cargo.toml
checkout_state="$(git rev-parse HEAD; git branch --show-current; git diff)"

for version in 0.2.0 0.1.9 0.3 v0.3.0; do
  if bash "$repository_root/scripts/prepare-release" "$version" >/dev/null 2>&1; then
    echo "release unexpectedly prepared for $version" >&2
    exit 1
  fi
done
test -z "$(git ls-remote --heads origin "refs/heads/release/*")"
test ! -e "$temporary_directory/gh-arguments"

bash "$repository_root/scripts/prepare-release" 0.3.0 >/dev/null

test "$(git rev-parse HEAD; git branch --show-current; git diff)" = "$checkout_state"
test "$(git -C "$origin" rev-parse master)" = "$(git rev-parse v0.2.0)"
test "$(git -C "$origin" rev-parse release/v0.3.0^)" = "$(git -C "$origin" rev-parse master)"
test "$(git -C "$origin" log -1 --format=%s release/v0.3.0)" = 'Release v0.3.0'
test "$(git -C "$origin" diff --name-only master release/v0.3.0)" = $'Cargo.lock\nCargo.toml'
git -C "$origin" show release/v0.3.0:Cargo.toml | grep -qx 'version = "0.3.0"'
diff - "$temporary_directory/gh-arguments" <<'EOF'
pr
create
--base
master
--head
release/v0.3.0
--title
Release v0.3.0
--body
Merging this pull request publishes Tuivir v0.3.0.
EOF
