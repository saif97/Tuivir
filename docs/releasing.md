# Releases

## Tuivir

Open **Actions → Prepare release → Run workflow** on `master` and choose
`patch`, `minor`, or `major`. The workflow updates both Cargo versions and uses
`create-pull-request` to open or refresh one PR on `release/next`.

Review and merge after CI passes. A version change pushed to `master` creates
an annotated tag on that exact commit and starts the existing release pipeline.
Other changes do not create tags. CI and all three builds must pass before
publication. Existing tags are never moved. Manually pushed version tags still
work and must match Cargo's version and belong to `master` history.

Publication uses normal `gh release` commands: create a draft, upload the
verified archives/checksums, then publish. A rerun can finish an existing draft;
it leaves already-published assets alone. GitHub's immutable-release setting is
recommended and supported, but is not a prerequisite imposed by the scripts.

## Homebrew

The final job reads the published `SHA256SUMS`, updates the two macOS URLs,
hashes, and version assertion, and directly commits the formula to the tap.
It does not run application binaries or receive build dependencies. Identical
updates do nothing; downgrades and changed hashes for the same version fail.
A normal Git push rejects concurrent tap changes rather than overwriting them.

If delivery fails, use GitHub's **Re-run failed jobs** after fixing the problem.
There is no separate tap service or release-state database.

## One-time setup

- Allow GitHub Actions to create PRs in Tuivir's Actions settings.
- Require CI before merging to `master`; protect release tags from changes and
  deletion while allowing the release workflow to create them.
- Add the repository Actions secret `HOMEBREW_TAP_TOKEN` to Tuivir. Use an expiring
  fine-grained PAT restricted to `saif97/homebrew-tap`, **Contents: read and write**
  only. Enter it directly in GitHub's secret UI and arrange its renewal. It needs
  no PR, workflow, or administration permissions. The tap must permit direct
  pushes; do not bypass an existing PR requirement.

Local checks: `python3 tests/release-tools.py`, the existing shell tests, and
`actionlint` after moving staged workflows into `.github/workflows`. The live
GitHub handoff still needs a configured release; local tests do not publish one.
