# Releasing Tuivir

A release has three steps. Only step 2 is automatic.

## 1. Open the version pull request

Use a stable `X.Y.Z` version that is newer than the latest `vX.Y.Z` tag.

```sh
git switch master && git pull --ff-only
git switch --create release/v0.2.1
bash scripts/set-release-version 0.2.1
git commit --all --message "Release v0.2.1"
git push --set-upstream origin release/v0.2.1
gh pr create --base master --title "Release v0.2.1" --body "Merging this pull request publishes Tuivir v0.2.1."
```

`set-release-version` changes the `tuivir` version in `Cargo.toml` and
`Cargo.lock`. It rejects a version that is not a stable `X.Y.Z`.

## 2. Merge the pull request

When CI passes, merge the pull request. The **Publish release** workflow then
starts because `Cargo.toml` changed on `master`. It compares the Cargo version
with the highest stable `vX.Y.Z` tag:

- If the version is newer, it builds the artifacts and publishes the
  `vX.Y.Z` release on the merge commit.
- If the version is the same, it does not publish a release.
- If the version is older, the run fails.

## 3. Update the Homebrew tap

After the release is published, run these commands in a checkout of
`saif97/homebrew-tap`. Set `TUIVIR` to your Tuivir checkout.

```sh
git switch main && git pull --ff-only
gh release download v0.2.1 --repo saif97/Tuivir --pattern SHA256SUMS --output /tmp/SHA256SUMS --clobber
bash "$TUIVIR/scripts/homebrew-formula" 0.2.1 /tmp/SHA256SUMS > Formula/tuivir.rb
git commit --all --message "Update Tuivir to 0.2.1"
git push
```

`homebrew-formula` fails if `SHA256SUMS` has no checksum for one of the macOS
archives. To check the result, run `brew upgrade saif97/tap/tuivir` and
`tuivir --version`.
