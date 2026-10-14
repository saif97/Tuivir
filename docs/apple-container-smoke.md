# Apple Container verification

Live verification is **pending**. The automated CLI fixtures exercise the Apple
Container 1.5.0 data model and Tuivir's existing Provider Workspace and
application boundaries; they do not establish compatibility with a live service
or interactive terminal.

The requesting user performs this checklist on Apple Silicon with macOS 26 and
Apple Container CLI 1.5.0. Create disposable Resources with Apple's CLI before
testing lifecycle Commands or Delete. Record the actual host and CLI versions
and results below. Keep the README roadmap entry pending until these checks pass.

- [ ] Stop the service externally. Verify the installed Provider stays visible
  with recovery instructions. Run `container system start` outside Tuivir and
  refresh; verify recovery without restarting Tuivir.
- [ ] Compare Containers, Images, Networks, and Volumes with native CLI listings,
  including stopped Containers and empty Resource Panels.
- [ ] Exercise Start, Stop, and composed Restart. Check refreshed Resource State,
  per-Resource progress, and failure diagnostics.
- [ ] Cancel Delete, then confirm ordinary deletion of a stopped Container and
  force deletion of a running disposable Container.
- [ ] Inspect every Resource kind. Check the latest 200 process Logs and VM Boot
  Logs separately, finite running-container Stats, and unavailable Stats for a
  stopped Container. Check diagnostics for a Container that never started.
- [ ] Open a running Container's `/bin/sh` Shell, navigate away and back, enlarge
  it, and release input with the configured controls. Quit with a live session;
  verify confirmation and local process cleanup leave the Container running.
- [ ] Try an image without `/bin/sh`; verify the shell failure is visible.
- [ ] Verify built-in Networks are inspectable and offer no Delete. Delete a
  user-managed Network. Verify in-use Network and Volume refusals preserve
  dependent Containers.
- [ ] Delete an ordinary Image. Test Apple's retention of a configured
  infrastructure image; retention must be reported instead of success. Apple
  hides infrastructure images from native listings and inspection output; a
  successful empty inspection still means the requested reference exists.
- [ ] Switch Provider Workspaces and return. Verify navigation and remembered
  Detail View Tabs.

## Results

- macOS version: pending
- CLI version: pending
- Date and tester: pending
- Results and diagnostics: pending

## Fixture provenance

The synthetic fixtures in `tests/fixtures/apple-container/` follow the Codable
models and command payloads at Apple's
[1.5.0 tag](https://github.com/apple/container/tree/1.5.0), commit
`d265d669ecae041bf338cb3b39c4118316d138f0`:

- `SystemVersion.VersionInfo` (CLI version without a reachable server)
- `ManagedContainer`, `ContainerConfiguration`, `ContainerStatus`, `RuntimeStatus`
- `ImageResource` (full reference, descriptor, platform variants)
- `NetworkResource`, `NetworkConfiguration`, `NetworkStatus`, `ResourceLabels`
- `VolumeResource`, `VolumeConfiguration`
- `ContainerStats` (finite sample)

Dates use the CLI's ISO 8601 JSON encoding. `future-state` deliberately exercises
an unfamiliar state. The built-in Network uses a non-default name and the
`com.apple.container.resource.role=builtin` label; the user-managed Network is
named `default` to prove protection is metadata-based.

`ImageInspect` resolves requested references before filtering infrastructure
images. A missing reference fails with `image not found: <reference>`;
`ContainerizationError` exposes this through `LocalizedError` and
ArgumentParser prefixes it with `Error: `. Verification accepts only that exact
diagnostic for the requested reference and a normal failure exit; other failures
leave deletion unverified. It never infers absence from a filtered listing.
