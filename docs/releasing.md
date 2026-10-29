# Releasing Tuivir

## Tuivir: prepare, review, merge

After the one-time setup below, open **Actions → Prepare release → Run workflow**
on `master`. Choose `patch`, `minor`, or `major`. For example, from `0.2.0`, those
choices produce `0.2.1`, `0.3.0`, or `1.0.0`.

The workflow opens a `release/vX.Y.Z` PR changing only the package versions in
`Cargo.toml` and `Cargo.lock`. It explicitly dispatches CI on that branch because
pushes made with `GITHUB_TOKEN` do not trigger another push workflow.

Review and merge the PR after CI passes. Merging creates an annotated tag on the
exact merged commit, even if `master` has advanced. The tag workflow rejects
fork PRs, non-default-branch merges, mismatched versions, dependency changes, and
changes outside the two Cargo files. Merge, squash, and rebase merge work for
the generated single-commit PR.

The tag workflow explicitly starts **Publish release**. It tests the tagged
commit, builds all three archives, uploads them and `SHA256SUMS` to a draft,
checks the uploaded digests, and publishes. Repository release immutability
locks the assets and tag. The workflow refuses downstream delivery if the
published release is not immutable. An older release is not marked latest.

Manually pushed stable `vX.Y.Z` tags still work. They must match the package
version and point to a commit reachable from `master`.

## Homebrew: automatic delivery

After publication, **Update Homebrew** downloads the immutable release and
checks both the checksum manifest and GitHub's asset digests. It generates the
existing two-architecture macOS formula and installs/tests it on an Apple
Silicon runner. Both Mac binaries also have their version checked during their
native release builds; this is not a second full two-architecture tap test suite.

Only the tested formula is transferred to a separate Ubuntu job. That job uses
the `homebrew` environment's credential to update
`saif97/homebrew-tap/Formula/tuivir.rb` through GitHub's contents API. It never
runs the downloaded binary and has no Rust build step.

The updater refuses downgrades, different hashes for an existing version, and
manual changes to the generated formula template. Updating identical contents
is a no-op. A concurrent edit fails the API's file-SHA comparison instead of
overwriting it. There is no tap PR or auto-merge service.

## One-time repository-owner setup

Complete this before preparing the first automated release:

1. In Tuivir's Actions settings, allow GitHub Actions to create pull requests.
   The workflow requests its own Contents, Pull requests, and Actions write
   permissions; it does not require a broad default write token. Organization
   policies must allow these operations. Preparing a PR does not approve it.
2. [Enable immutable releases](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/establish-provenance-and-integrity/prevent-release-changes)
   in Tuivir. This is a repository setting, not a workflow permission. Existing
   releases such as `v0.2.0` do not become immutable retroactively.
3. Require the **checks** CI status on `master`. Protect `v*` tags from updates
   and deletion, and limit creation to the intended release actors. Make sure
   the Actions identity used by the tagging job is allowed to create tags.
   Restrict edits to release workflows to trusted maintainers.
4. Create a GitHub environment named `homebrew`. Limit its deployment refs to
   `master` and the protected `v*` tags. To keep delivery unattended, do not add
   a manual environment approval unless that pause is intentional.
5. Create an expiring **fine-grained PAT** owned by an account with access to
   `saif97/homebrew-tap`. Select **only that repository**, with
   **Contents: read and write** (and GitHub's implicit Metadata read permission).
   It needs no workflow-editing, administration, or PR permission. Store it as
   the environment secret **HOMEBREW_TAP_TOKEN** in Tuivir's `homebrew`
   environment. Enter it directly in GitHub's secret UI; do not put it in chat,
   source control, command arguments, or documentation. Assign an owner to renew
   it before expiration. Its write access covers the tap repository, not just
   the formula file. A GitHub App can replace it later if token renewal becomes
   burdensome.
6. Confirm the tap's branch rules allow this direct formula update. Do not add a
   protection bypass to make it work; if the tap requires PRs, use a PR delivery
   design instead.

No repository settings or secrets are created by these workflows. The code
change alone does not complete this setup.

## Recovery

- **Preparation failed:** rerun the same bump choice. It reuses an identical
  branch and open PR. If `master` or the release branch has changed, it refuses
  to overwrite the branch. Close the obsolete PR, delete its release branch,
  and prepare again after inspecting why it became stale. Only one release PR
  should remain open.
- **Tagging or dispatch failed:** rerun the failed **Tag merged release** job.
  An existing tag is accepted only if it still points to the same commit.
- **Build/upload failed:** rerun **Publish release**. Partial drafts can be
  repaired, including replacing unpublished draft assets. Published assets
  are never replaced by the helper.
- **Homebrew failed:** run **Update Homebrew** on `master`, supplying the
  already-published tag. This downloads the original assets and does not rebuild
  the binaries. Rerunning publication also skips builds for an existing
  immutable release. If the failure was a concurrent tap edit, rerun to read the
  new file SHA; version and template checks still apply.
- **Immutability was not enabled:** publication reports an error after GitHub
  publishes a mutable release, and Homebrew stops. Enable the setting and
  prepare a new version; do not silently replace an already-public release.

Publication and tap writes are serialized. GitHub concurrency is not a durable
FIFO queue: a newer run can replace an older pending run. Do not launch a batch
of releases; manually rerun any displaced release that is still needed.

## Local verification

The helper uses Python 3.11+ and the standard library. GitHub operations use
`gh`; the automated command tests replace only that external boundary with a
stateful fake and use real disposable Git repositories.

```sh
python3 tests/release-automation.py
bash tests/release-version.sh
bash tests/static-elf.sh
actionlint
```

These checks do not create real tags, releases, PRs, or tap commits on GitHub.
The live GitHub event handoff and macOS Homebrew installation still need the
first configured release to exercise them.
