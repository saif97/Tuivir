#!/usr/bin/env bash

set -euo pipefail

repository_root="$(git rev-parse --show-toplevel)"
fixtures="$repository_root/tests/fixtures/homebrew"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
export GIT_AUTHOR_NAME=Test GIT_AUTHOR_EMAIL=test@example.com
export GIT_COMMITTER_NAME=Test GIT_COMMITTER_EMAIL=test@example.com

tap="$temporary_directory/homebrew-tap.git"
git init --quiet --bare --initial-branch=main "$tap"
git clone --quiet "$tap" "$temporary_directory/seed" 2>/dev/null
mkdir "$temporary_directory/seed/Formula"
cp "$fixtures/tuivir-v0.2.0.rb" "$temporary_directory/seed/Formula/tuivir.rb"
git -C "$temporary_directory/seed" add Formula
git -C "$temporary_directory/seed" commit --quiet --message 'Add Tuivir 0.2.0'
git -C "$temporary_directory/seed" push --quiet origin main

# The fake gh clones the local tap and serves the v0.2.0 checksums renamed for
# the requested release.
mkdir "$temporary_directory/bin"
cat > "$temporary_directory/bin/gh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
case "\$1 \$2" in
  'repo clone') git clone --quiet "$tap" "\$4" ;;
  'release download') sed "s/tuivir-v0\.2\.0-/tuivir-\$3-/" "$fixtures/SHA256SUMS-v0.2.0" > "\${@: -1}" ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$temporary_directory/bin/gh"
export PATH="$temporary_directory/bin:$PATH"

for version in 0.1.9 0.3 v0.3.0; do
  if bash "$repository_root/scripts/update-homebrew-tap" "$version" >/dev/null 2>&1; then
    echo "tap unexpectedly updated to $version" >&2
    exit 1
  fi
done
test "$(git -C "$tap" rev-list --count main)" -eq 1

bash "$repository_root/scripts/update-homebrew-tap" 0.3.0 >/dev/null

sed "s/tuivir-v0\.2\.0-/tuivir-v0.3.0-/" "$fixtures/SHA256SUMS-v0.2.0" > "$temporary_directory/SHA256SUMS"
bash "$repository_root/scripts/homebrew-formula" 0.3.0 "$temporary_directory/SHA256SUMS" > "$temporary_directory/expected.rb"
git -C "$tap" show main:Formula/tuivir.rb | diff "$temporary_directory/expected.rb" -
test "$(git -C "$tap" log -1 --format=%s main)" = 'Update Tuivir to 0.3.0'

bash "$repository_root/scripts/update-homebrew-tap" 0.3.0 >/dev/null
test "$(git -C "$tap" rev-list --count main)" -eq 2
if bash "$repository_root/scripts/update-homebrew-tap" 0.2.0 >/dev/null 2>&1; then
  echo 'tap unexpectedly downgraded to 0.2.0' >&2
  exit 1
fi
