# Release automation and Homebrew comparators

Researched 2026-10-24 using GitHub's API (`gh api`), raw repository files, and first-party documentation. Scope: `jesseduffield/lazydocker`, `jesseduffield/lazygit`, `herdrdev/herdr`, `sharkdp/bat`, and `sharkdp/fd`, plus cargo-dist and first-party Homebrew/GitHub documentation. No workflow execution, external mutations, or implementation changes.

## Decision-relevant findings

There is no single standard release architecture across these projects. Lazydocker provides a particularly relevant precedent for a small, seamless custom-tap release: GoReleaser publishes binaries and directly commits a generated binary formula to its owner's tap. Lazygit has migrated its old tap to Homebrew core. Bat and fd use custom Cargo/cross workflows and distribute through core; they do not automate a custom Homebrew tap in the inspected workflows. [LD1–LD4, LG1–LG4, B1, F1, H1–H4]

For Tuivir, retaining its existing three Rust archives and checksums, adding a test gate, and automating the existing binary formula is a reasonable minimal direction. A GitHub App, a formula PR, tests on both Mac architectures, provenance, and immutable releases are independent decisions—not a package that these comparators establish as necessary. This is a recommendation, not a claim that simpler systems are equally secure.

The Tuivir baseline was checked against [publication](../../.github/workflows/publish-release.yml), [artifact builds](../../.github/workflows/build-release-artifacts.yml), [CI](../../.github/workflows/ci.yml), and the [tap formula](https://github.com/saif97/homebrew-tap/blob/main/Formula/tuivir.rb): `vX.Y.Z` releases, three archives plus `SHA256SUMS`, no CI test gate for publication, and manual tap updates. The comparator researcher received that baseline in its brief; the parent agent verified it. Classic branch-protection and default Actions-permission settings returned HTTP 403, so their effective configuration remains unverified.

## Evidence scope and limitations

The pinned sources below represent default-branch snapshots at research time, not necessarily the workflows used for each repository's latest release. In particular, old articles about lazygit's tag-triggered GoReleaser/tap workflow would misdescribe its current files. Secret names reveal use, not credential type, granted repositories, expiration, or exact PAT/App scopes. No inference about branch/tag protections, environment approvals, repository-level immutable-release settings, or effective default token permissions should be made from their absence in YAML.

### Lazydocker: GoReleaser plus an upstream-owned binary tap

- **Trigger and tooling:** push tags matching `v*` start `.github/workflows/cd.yml`; this is a glob, not strict semantic-version validation. One Ubuntu job runs Go 1.24.x and GoReleaser **v1.17.2**, via `goreleaser-action@v1`, with `release --clean`. `CGO_ENABLED=0` enables its Go builds for Windows, Darwin, and Linux across several architectures. Configuration creates archives and `checksums.txt`. [LD1, LD2]
- **Homebrew ownership:** upstream recommends `jesseduffield/lazydocker/lazydocker`, while also documenting core's `brew install lazydocker`. The custom repository is `jesseduffield/homebrew-lazydocker`. Its generated formula selects release-asset URLs and SHA-256 values by platform/architecture and uses `bin.install "lazydocker"`; it does not build from source or define a `bottle` block. [LD3, LD4]
- **Update mechanism:** `.goreleaser.yml` supplies the tap owner/name, with no `pull_request.enabled`, branch override, or separate tap token. GoReleaser v1.17.2's implementation directly creates/updates the formula file when PR mode is disabled; its documented default is the tap's default branch. Thus this is a direct API commit (the equivalent of direct push), not an automatic PR. The current tap commit is titled “Brew formula update for lazydocker version v0.25.2” and authored as `goreleaserbot`. [LD2, LD4, G1, G2]
- **Credentials:** release passes `secrets.TOKEN_GITHUB` as `GITHUB_TOKEN`. With no tap token override, that credential is reused for the cross-repository formula update. Neither release YAML nor config declares the secret's scopes or a `permissions:` block; its exact grants are **unknown**. The variable name does not make it GitHub's built-in ephemeral token. [LD1, LD2, G1]
- **CI/security choices and limits:** separate CI runs on master pushes and PRs, with Ubuntu/Windows tests, builds, linting, and vendor checks. Release has no test step or dependency on that CI. GoReleaser's executable version is fixed, but Actions are referenced by tags, including old major versions; no artifact attestation step appears in release. The generated tap formula has no `test do` block. These are observable limits, not evidence that the project is compromised. [LD1, LD4, LD5]

### Lazygit: manual release orchestration, Homebrew core distribution

- **Trigger and tooling:** current `release.yml` uses **`workflow_dispatch`**, not a tag push trigger. Its schedule is commented out. Inputs choose minor/patch bump, release branch, and whether to ignore blocking issues/PRs. It validates version shape, checks changes and generated docs/schema, optionally checks `blocks-release`, creates/pushes an annotated tag, then runs GoReleaser `version: v2`. The action itself is pinned to a commit, but `v2` does not freeze the GoReleaser binary version. [LG1]
- **Operational wrinkle:** workflow comments describe running in a fork while releasing `jesseduffield/lazygit`; checkout explicitly targets upstream using `LAZYGIT_RELEASE_PAT`. It verifies its workflow matches upstream master. This is configuration evidence, not verification of where maintainers actually invoke it. [LG1]
- **Builds:** GoReleaser config uses `CGO_ENABLED=0`, builds FreeBSD/Windows/Darwin/Linux variants, writes archives and `checksums.txt`, and uses GitHub-native changelogs. There is **no `brews` publisher** in current config. [LG2]
- **Homebrew:** README recommends `brew install lazygit`. The historical `jesseduffield/homebrew-lazygit` repository removed `lazygit.rb`; its migration file maps `lazygit` to `homebrew/core`. Core owns the active source formula and bottle definitions. Core update PR #318929 for 0.66.2 was opened by BrewTestBot, demonstrating Homebrew-side version automation. This is not an upstream tap push or evidence of a required manual update every release. [LG3, LG4, H2, H7]
- **Credentials and gate:** `LAZYGIT_RELEASE_PAT` handles checkout, issue/PR queries, tag push, and GoReleaser publishing. Exact scopes and repository selection are not visible; release YAML declares no `permissions:` block. Its checks are release-process checks, not a test gate. Separate CI has Ubuntu/Windows unit tests, Linux integration tests across Git versions including a race-detector run, and build/lint/generated-file checks; release does not explicitly await those jobs. [LG1, LG5]

### Bat: hand-written Cargo/cross release jobs, Homebrew core

- **Trigger and tooling:** `CICD.yml` runs on manual dispatch, PRs, master pushes, and **all tag names (`'*'`)**. Upload to GitHub Releases is conditional on `^refs/tags/v[0-9].*`; this is broader than `vX.Y.Z`. Metadata and asset names come from Cargo's package version. No explicit equality check between that version and the pushed tag appears. [B1]
- **Build/publish:** a 13-target Linux/macOS/Windows matrix uses Cargo or `cross`, with `cross` installed from a pinned Git revision. It runs `build --locked --release`, target tests, smoke/feature checks, packages archives and Debian packages, uploads workflow artifacts, then calls `softprops/action-gh-release@v2`. Intel and ARM Mac runners are included. ARM targets restrict tests to library/binary targets, so this is not uniform integration-test coverage. No `cargo publish` or Homebrew publisher appears in this workflow. [B1]
- **Gate nuance:** each target's upload follows that target's checks. However, `build` needs only metadata. The `all-jobs` summary depends on lint, MSRV, license, audit, extra tests, documentation, and build; **publication occurs inside build before the summary gate**. A successful target can publish while another target or an independent check fails. [B1]
- **Credentials/security:** release uses `secrets.GITHUB_TOKEN`; no explicit workflow/job `permissions:` block specifies its effective rights. Winget uses a separate `WINGET_TOKEN` with undisclosed scopes. `cargo audit`, locked Cargo commands, and a pinned cross revision are positive observations. Action references include mutable `@stable`, `@master` (including upload-artifact), and version tags; no artifact attestation or standalone checksum-manifest generation is visible in this workflow. This is not a general security endorsement. [B1]
- **Homebrew:** README recommends core. Its formula downloads the tagged **source** archive, builds with Rust, installs completions/manuals, and defines Homebrew bottle hashes and a functional/linkage test. It links Homebrew's libgit2/oniguruma rather than simply installing the upstream release binary. Updates belong to Homebrew's PR/bottle process, not this release workflow. [B2, H3, H5, H6]

### Fd: Cargo/cross releases with attestations, Homebrew core

- **Trigger:** same broad trigger pattern as bat: all tags start CI, while `^refs/tags/v[0-9].*` enables archive publication. Cargo metadata supplies asset versions; no explicit tag/version equality check appears. [F1]
- **Build/publish:** 14-target Cargo/cross matrix; native Intel/ARM Mac runners; locked release builds and target tests; `cross` v0.2.5 downloaded from GitHub Releases; helper scripts package archives and Debian packages. ARM-family targets restrict tests to the binary target. Release uploads use a commit-pinned `softprops/action-gh-release`; no `cargo publish` or Homebrew publisher appears in the workflow. [F1]
- **Visible permissions:** workflow default is `contents: read`. The **build job** explicitly requests `contents: write`, `id-token: write`, and `attestations: write`. `actions/attest` is commit-pinned and runs for release refs before publication. Upload-artifact and release actions are also commit-pinned; checkout uses a version tag with `persist-credentials: false`, while Rust toolchain actions retain `@stable`/`@master`. Winget again uses `WINGET_TOKEN`, whose scopes are unknown. [F1]
- **Limits:** write permissions are assigned to the entire build job, not a small separate publish job, although fork-PR token restrictions can reduce effective grants. `build` depends only on metadata; lint/format/MSRV jobs are not dependencies of publication. Each target can publish independently. The downloaded cross executable is version-selected but the download step has no explicit digest verification. Attestation improves traceability but does not fix test-gate or credential-boundary concerns. No standalone checksum-manifest generation is visible in the workflow itself. [F1]
- **Homebrew:** README points to core; its source formula invokes Cargo, installs completions/manual, tests a filename search, and contains Homebrew bottle hashes. Core's lifecycle is independent of upstream release-asset uploads and their attestations. [F2, H4–H6]

## Homebrew ownership, credentials, and bottles

All four have Homebrew-owned **core** formulae. Each pinned core formula references a source archive, has a build dependency on Go or Rust, and includes a `bottle do` block. Lazydocker additionally maintains its upstream-owned binary tap. Core formula changes flow through Homebrew: its `autobump.yml` runs every three hours or manually, uses `brew bump --no-fork --open-pr`, and supplies `HOMEBREW_CORE_REPO_WORKFLOW_TOKEN`; the workflow's ordinary token has `contents: read`, while the secret's exact permissions are not public. It also supplies a separate SSH signing key for commits. This does not establish that every formula is automatically bumped on every release; maintainers/contributors can also submit `brew bump-formula-pr`. [H1–H7, H8]

A **binary formula** can simply download the upstream archive selected for the user's OS/CPU, verify its specified SHA-256, and install the executable. Lazydocker's tap does exactly that. A **Homebrew bottle** is Homebrew's packaged result of installing a formula with `--build-bottle` and running `brew bottle`; core's BrewTestBot builds/tests packages, updates bottle metadata, and uploads bottles to GitHub Packages through Homebrew's approval process. Both offer precompiled installation, but they differ in producer, build environment, artifact layout, metadata, and update authority. Tuivir does not need its own bottle pipeline merely to distribute its existing binaries through a custom tap. [LD4, H5]

No observed upstream release workflow here opens a Homebrew formula PR. The visible custom-tap publisher commits directly; Homebrew itself opens core PRs. A PR workflow for Tuivir remains a valid design choice, but it is not required by these examples. [LD2, LG2, B1, F1, G1, H6]

## Herdr: useful release safeguards, a different Homebrew ownership model

Inspected 2026-10-24 at herdrdev/herdr commit `5608de0608918aba2f9d501e7793b984a0ca9cfa`. The user's “herder” is interpreted as Herdr, matching the repository's reference to `herdr/`.

- Stable publication starts from a `v*` tag. Its workflow separates read-only builds from a `contents: write` release job, pins actions to commit SHAs, checks both the original actor and rerun actor for repository-admin permission, and validates the release source before building. [Workflow](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/.github/workflows/release.yml)
- Its release validator requires annotated stable tags with preview provenance and checks a previously published immutable preview. This is a preview-to-stable promotion policy, with hotfix handling, rather than a minimal tag-and-publish process. [Validator](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/scripts/release.py)
- The preview workflow runs checks before building and stages release assets in a draft before publishing. It includes retry handling for an already-published immutable preview. [Preview workflow](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/.github/workflows/preview.yml)
- Herdr's README advertises `brew install herdr`. The official Homebrew formula downloads the source tag, builds using Rust and Zig, defines Homebrew-produced bottles, and uses `github_latest` livecheck. Its recent version and bottle commits are by BrewTestBot. This is not a custom tap receiving upstream binary URL substitutions. [README](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/README.md), [core formula](https://github.com/Homebrew/homebrew-core/blob/7dfc430d61211b9a426fe80d6decba9d8c8368d5/Formula/h/herdr.rb), [bot version update](https://github.com/Homebrew/homebrew-core/commit/fd43b684a476c59484653e448d8994b346b1011f)
- Herdr does have a post-release cross-repository Nix dispatch. It is a separate, nonfatal job with its own named secret, and failure guidance says to rerun the package update while retaining the published release. The secret's actual scopes cannot be inferred from its name. [Release workflow, update-nix-package](https://github.com/herdrdev/herdr/blob/5608de0608918aba2f9d501e7793b984a0ca9cfa/.github/workflows/release.yml#L262)

**Lesson for Tuivir:** retain source/version checks and separation between release publication and downstream delivery. Do not import Herdr's preview channel, custom promotion validator, documentation publication, and issue-management automation merely to update one formula.

## Tooling and platform facts that affect the decision

### cargo-dist is a credible Rust option, not a mandatory migration

The project's own Rust quickstart describes version preparation followed by pushing a version tag; dist generates the build/release workflow. Its Homebrew installer uses prebuilt archives in a custom tap. The publisher template checks out the tap using `HOMEBREW_TAP_TOKEN`, commits generated formulas, and pushes directly. It does not require a formula PR/auto-merge workflow.

- [Rust quickstart](https://github.com/axodotdev/cargo-dist/blob/86d735eb02c02941465c4df3017539f74adacb6a/book/src/quickstart/rust.md)
- [Homebrew installer documentation](https://github.com/axodotdev/cargo-dist/blob/86d735eb02c02941465c4df3017539f74adacb6a/book/src/installers/homebrew.md)
- [Actual Homebrew publisher template](https://github.com/axodotdev/cargo-dist/blob/86d735eb02c02941465c4df3017539f74adacb6a/cargo-dist/templates/ci/github/partials/publish_homebrew.yml.j2)

Two caveats matter. Its documentation still suggests a classic `repo`-scoped PAT; that is not the credential recommendation for Tuivir. It also explicitly warns that publishing an older maintenance release can replace the tap's newer version. A tool does not remove the need to choose credential scope and release-order policy.

### Homebrew core is a separate distribution service

Homebrew documents automatic update PRs for eligible packages in its official repositories, normally checked every three hours. This does not promise immediate delivery, and it does not automatically maintain arbitrary third-party taps. Official core formulas must build from source or install portable platform-independent output; Tuivir's platform-specific prebuilt-binary formula cannot simply be copied into core unchanged.

- [Autobump](https://github.com/Homebrew/brew/blob/c5b9f65d0b679d719545df334e8d433a5b6ded3f/docs/Autobump.md)
- [Acceptable formulae](https://github.com/Homebrew/brew/blob/c5b9f65d0b679d719545df334e8d433a5b6ded3f/docs/Acceptable-Formulae.md)

GoReleaser's legacy `brews` feature is deprecated in its current docs in favor of Homebrew casks. It still illustrates the generated-tap pattern, but copying an older project's configuration is not necessarily the right starting point for a new setup. [GoReleaser Homebrew formulas](https://github.com/goreleaser/goreleaser/blob/0162ee4bbe06909cc13956e147a6adb01feb127d/www/content/customization/publish/homebrew_formulas.md)

### Credentials and immutability

- `GITHUB_TOKEN` is scoped to the repository containing the workflow. The Tuivir workflow therefore needs a separate credential to write its separate tap. [GitHub token scope](https://github.com/github/docs/blob/02c25d16369fe27f9bdb28d1201f6b452f0d21dd/content/actions/concepts/security/github_token.md)
- A fine-grained PAT can limit write access to just the tap and just `Contents: write`. No PR, workflow-editing, or administration permission is necessary for a direct formula commit. This is repository scope, not path scope: a stolen token could still change other ordinary files in the tap. Expiration creates a real renewal obligation. A tap-only GitHub App is an alternative with short-lived installation tokens, but its private key is still a secret to protect. [PAT permissions and expiration](https://github.com/github/docs/blob/02c25d16369fe27f9bdb28d1201f6b452f0d21dd/content/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens.md), [App installation authentication](https://github.com/github/docs/blob/02c25d16369fe27f9bdb28d1201f6b452f0d21dd/content/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation.md)
- GitHub's immutable releases lock assets and the associated tag and automatically produce a release attestation. GitHub recommends create draft, attach all assets, then publish. A release attestation binds published contents; it is not the same as a separately generated build-provenance attestation. [Immutable releases](https://github.com/github/docs/blob/02c25d16369fe27f9bdb28d1201f6b452f0d21dd/content/code-security/concepts/supply-chain-security/immutable-releases.md)
- Do not rely on a tag pushed with `GITHUB_TOKEN` to start another push workflow. Explicit dispatch is supported. Current GitHub documentation also has a feature-dependent exception where token-created PRs can create approval-required runs; an unattended PR workflow should not assume those checks automatically execute. [Trigger behavior](https://github.com/github/docs/blob/02c25d16369fe27f9bdb28d1201f6b452f0d21dd/data/reusables/actions/actions-do-not-trigger-workflows.md)

## Recommended scope for Tuivir

This is a design recommendation, not a change to repository configuration.

1. **Keep the current Rust build/release workflows.** They already build the required three targets, check the built binary version, enforce static Linux linking, and publish checksummed archives. Adding a release framework solely to fill one downstream gap introduces migration work. Reconsider cargo-dist when more installers/targets make that migration pay for itself.
2. **Use a deliberate version bump and protected tag as the release decision.** Fix the masked validation failure; require the version to match, the tagged source to belong to protected master, and CI to pass for that exact commit. Do not infer the next version from commit messages without an explicit project policy.
3. **Publish a complete immutable release.** Use draft staging, preserve the existing action pins and locked dependencies, and isolate write credentials from builds. Separate build-provenance verification can be added later; it is not necessary to build a custom verifier just to automate the tap.
4. **Add one downstream job that generates, checks, and directly commits the formula.** Render from fixed upstream coordinates and exact published assets; verify both macOS archive hashes and preserve the version assertion. Smoke-test the staged formula before publishing it. Existing native release builds already check both Mac binaries; there is no need to begin by standing up a whole second CI/auto-merge service in the tap.
5. **Give the writer only tap access.** Start with an expiring fine-grained PAT scoped to `saif97/homebrew-tap`, `Contents: write`, confined to the final writer job/step; plan renewal. An App is the alternative if avoiding PAT renewals outweighs its setup cost. Never pass the tap credential to the Rust build or installation test. Direct commits require compatible tap branch rules; do not silently bypass an existing PR requirement.
6. **Make recovery explicit.** Retrying the tap update must use the already-published immutable assets, not rebuild or overwrite the release. Same version/same hashes is a no-op; same version/different hashes is an error; older versions cannot replace newer tap versions. Serialize writes and handle concurrent changes with bounded retries or a clear failure. GitHub Actions concurrency is not a durable FIFO release queue.
7. **Test the new behavior first.** Following this repository's TDD instruction, cover bad tags, version mismatch, missing/wrong assets, formula version/hash rendering, retry no-ops, and downgrade rejection before implementing the updater. A real staged Homebrew install verifies the packaging contract.

This is a narrower recommendation than the earlier proposal of an App plus a tap PR plus automatic merging plus independent two-architecture tap CI and provenance enforcement. Those additions remain appropriate if independently reviewed distribution changes become a requirement. The core first implementation is the existing pipeline plus a small, testable updater and its isolated credential.

No production workflows, tags, releases, secrets, branch rules, or tap contents were changed by this research.

## Comparator source index

Comparator repository file links below use immutable commit IDs. Herdr, tooling, and platform sources are linked inline in their respective sections. Documentation links A1–A2 are live first-party documentation checked on the research date. PR links are historical supporting evidence, not immutable configuration snapshots.

- **LD1:** [lazydocker release workflow](https://github.com/jesseduffield/lazydocker/blob/7e7aadc2071d58031bf2daafca1fbd4093efc23f/.github/workflows/cd.yml).
- **LD2:** [lazydocker GoReleaser config](https://github.com/jesseduffield/lazydocker/blob/7e7aadc2071d58031bf2daafca1fbd4093efc23f/.goreleaser.yml).
- **LD3:** [lazydocker installation README](https://github.com/jesseduffield/lazydocker/blob/7e7aadc2071d58031bf2daafca1fbd4093efc23f/README.md#homebrew).
- **LD4:** [generated custom-tap formula](https://github.com/jesseduffield/homebrew-lazydocker/blob/49a5eebdab1ac2322511b2cb9040703d3b004bb4/lazydocker.rb), [update commit](https://github.com/jesseduffield/homebrew-lazydocker/commit/49a5eebdab1ac2322511b2cb9040703d3b004bb4).
- **LD5:** [lazydocker CI](https://github.com/jesseduffield/lazydocker/blob/7e7aadc2071d58031bf2daafca1fbd4093efc23f/.github/workflows/ci.yml).
- **LG1:** [lazygit release workflow](https://github.com/jesseduffield/lazygit/blob/4e1eebe278790f257a875da6633234391a5a85b3/.github/workflows/release.yml).
- **LG2:** [lazygit GoReleaser config](https://github.com/jesseduffield/lazygit/blob/4e1eebe278790f257a875da6633234391a5a85b3/.goreleaser.yml).
- **LG3:** [lazygit installation README](https://github.com/jesseduffield/lazygit/blob/4e1eebe278790f257a875da6633234391a5a85b3/README.md#homebrew).
- **LG4:** [tap migration](https://github.com/jesseduffield/homebrew-lazygit/blob/56cd22cc17753d722a305826610083f77961beb8/tap_migrations.json), [formula-removal commit](https://github.com/jesseduffield/homebrew-lazygit/commit/56cd22cc17753d722a305826610083f77961beb8).
- **LG5:** [lazygit CI](https://github.com/jesseduffield/lazygit/blob/4e1eebe278790f257a875da6633234391a5a85b3/.github/workflows/ci.yml).
- **B1:** [bat CICD](https://github.com/sharkdp/bat/blob/a9aa73d215455a479f072da75dd39f19826acfea/.github/workflows/CICD.yml).
- **B2:** [bat installation README](https://github.com/sharkdp/bat/blob/a9aa73d215455a479f072da75dd39f19826acfea/README.md).
- **F1:** [fd CICD](https://github.com/sharkdp/fd/blob/c44735ec0f130c2486afe9c976937c03b57a8c62/.github/workflows/CICD.yml).
- **F2:** [fd installation README](https://github.com/sharkdp/fd/blob/c44735ec0f130c2486afe9c976937c03b57a8c62/README.md).
- **G1:** [GoReleaser v1.17.2 brew publisher implementation](https://github.com/goreleaser/goreleaser/blob/6afe717dc9e716bb7e2d893d500fdf3d8b94b872/internal/pipe/brew/brew.go#L121-L171).
- **G2:** [GoReleaser v1.17.2 Homebrew documentation](https://github.com/goreleaser/goreleaser/blob/6afe717dc9e716bb7e2d893d500fdf3d8b94b872/www/docs/customization/homebrew.md).
- **H1:** [core lazydocker formula](https://github.com/Homebrew/homebrew-core/blob/7dfc430d61211b9a426fe80d6decba9d8c8368d5/Formula/l/lazydocker.rb).
- **H2:** [core lazygit formula](https://github.com/Homebrew/homebrew-core/blob/7dfc430d61211b9a426fe80d6decba9d8c8368d5/Formula/l/lazygit.rb).
- **H3:** [core bat formula](https://github.com/Homebrew/homebrew-core/blob/7dfc430d61211b9a426fe80d6decba9d8c8368d5/Formula/b/bat.rb).
- **H4:** [core fd formula](https://github.com/Homebrew/homebrew-core/blob/7dfc430d61211b9a426fe80d6decba9d8c8368d5/Formula/f/fd.rb).
- **H5:** [Homebrew bottle documentation](https://github.com/Homebrew/brew/blob/c5b9f65d0b679d719545df334e8d433a5b6ded3f/docs/Bottles.md).
- **H6:** [core autobump workflow](https://github.com/Homebrew/homebrew-core/blob/7dfc430d61211b9a426fe80d6decba9d8c8368d5/.github/workflows/autobump.yml).
- **H7:** [BrewTestBot lazygit 0.66.2 update PR](https://github.com/Homebrew/homebrew-core/pull/318929), [version commit](https://github.com/Homebrew/homebrew-core/commit/5a555824d5ea33f2e7becbd622d10386579cef0f).
- **H8:** [Homebrew contribution/update instructions](https://github.com/Homebrew/brew/blob/c5b9f65d0b679d719545df334e8d433a5b6ded3f/docs/How-To-Open-a-Homebrew-Pull-Request.md).
- **A1:** [GitHub contents API permissions](https://docs.github.com/en/rest/repos/contents#create-or-update-file-contents).
- **A2:** [GitHub workflow-token scope and lifetime](https://docs.github.com/en/actions/concepts/security/github_token).
