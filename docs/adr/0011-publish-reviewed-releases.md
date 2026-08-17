# Publish reviewed releases from their merge commit

Tuivir publishes one immutable GitHub Release only after a `release/vX.Y.Z` pull request merges into current `master`. The closed-pull-request workflow validates the release branch, the merged commit, Cargo metadata, and the versioned release notes before it creates or verifies the annotated tag. It then builds the three production archives from that exact commit in the same workflow run, verifies their checksums, and publishes the reviewed notes with those archives and `SHA256SUMS`.

The tag and GitHub Release are separately idempotent. A rerun accepts only an existing tag pointing at the same merge commit and an existing release with the same notes and byte-identical assets. It may complete a missing asset from an interrupted initial upload, but never replaces an existing asset, moves a tag, or changes a published release. A conflicting tag, note, or asset fails loudly; the correction path is a new patch release.

GitHub's ephemeral `GITHUB_TOKEN` creates both the tag and release. Tagging and publishing remain in one workflow because token-created tags do not reliably trigger a second workflow; no personal access token or GitHub App is introduced merely to chain automation. Homebrew and AUR are later, independent projections after this canonical release succeeds.

The release command has an integration seam that can substitute a local GitHub CLI for temporary repositories; the production workflow invokes the same command with `gh`. `CONTEXT.md` remains unchanged because reviewed publication is release engineering, not Tuivir domain language.
