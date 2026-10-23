# Releasing Tuivir

A release has three steps. Only step 2 is automatic. Steps 1 and 3 use your
own `git` and `gh` login, so CI needs no token other than the built-in one.

## 1. Open the version pull request

Use a stable `X.Y.Z` version that is newer than the latest `vX.Y.Z` tag.

```sh
bash scripts/prepare-release 0.2.1
```

`prepare-release` fails if the version is not newer than the highest release
tag. It then creates `release/v0.2.1` from `origin/master` in a temporary
worktree, sets the `tuivir` version in `Cargo.toml` and `Cargo.lock`, pushes the
branch, and opens the pull request. Your current checkout does not change.

## 2. Merge the pull request

When CI passes, merge the pull request. The **Publish release** workflow then
starts because `Cargo.toml` changed on `master`. It compares the Cargo version
with the highest stable `vX.Y.Z` tag:

- If the version is newer, it builds the artifacts and publishes the
  `vX.Y.Z` release on the merge commit.
- If the version is the same, it does not publish a release.
- If the version is older, the run fails.

## 3. Update the Homebrew tap

After the release is published, run:

```sh
bash scripts/update-homebrew-tap 0.2.1
```

`update-homebrew-tap` clones `saif97/homebrew-tap` to a temporary directory. It
fails if the version is older than the formula version, and it does nothing if
the version is the same. Otherwise it writes `Formula/tuivir.rb` from the
release `SHA256SUMS`, commits, and pushes to the tap's `main` branch. It fails
if the release has no `SHA256SUMS` or no checksum for a macOS archive.

To check the result, run `brew upgrade saif97/tap/tuivir` and
`tuivir --version`.
