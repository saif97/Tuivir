# Tart support

Install [Tart](https://github.com/openai/tart) separately on a supported macOS
host. Tuivir uses the `tart` executable on `PATH` and respects Tart's own
environment, including `TART_HOME`. No Tuivir Provider configuration is needed.

The VMs panel contains local VMs. The Images panel contains Tart's cached OCI
images, including the tags and digests Tart lists. Disk capacity and allocated
size use Tart's decimal GB values. An unavailable disk capacity is omitted.

VMs have Overview and Config detail tabs. Config loads `tart get --format json`
for the selected VM; cached images have snapshot-backed Info. Tuivir does not
run `tart get` against cached images, because that command opens only local VMs.

## Commands

| Resource | Command | Behavior |
| --- | --- | --- |
| Stopped VM | Start (`S`) | Launches `tart run --no-graphics` in an independent session. |
| Running VM | Stop (`s`) | Uses `tart stop`, with Tart's default graceful timeout. |
| Suspended VM | Resume (`p`) | Launches `tart run --no-graphics`, which restores Tart's saved state. |
| VM | Delete (`d`) | Asks for confirmation, then stops a VM unless its last reported state is stopped, and deletes it. A failed stop aborts deletion. |
| Cached image | Delete (`d`) | Asks for confirmation and deletes only the selected cache entry through `tart delete`. |
| Running VM | Shell (`E`) | Opens `/bin/sh` through `tart exec -i -t`. |

Start and Resume use Tart's default run settings. Additional networking,
directory sharing, or device options should be supplied by running Tart
directly. Existing VMs started outside Tuivir are also listed and operable.
Tart has no native restart command; this workspace does not offer Restart.

Quitting Tuivir leaves VMs running. Start and Resume observe the process for
one second and report immediate launch errors; completion means the process
launched, rather than that the guest has finished booting. Later VM state is
reported by normal refreshes. Process output is saved to private
`launch-*.log` files under `$XDG_STATE_HOME/tuivir/provider-processes`, or
`~/.local/state/tuivir/provider-processes` when the XDG path is unset or relative.
Logs are removed when Tuivir observes a successful process exit. Logs from later
failures, or launches that outlive Tuivir, remain available for diagnosis and
may be removed when no longer needed.

Shell requires macOS 14 or newer and the
[Tart Guest Agent](https://github.com/cirruslabs/tart-guest-agent) running inside
the VM with command execution enabled. Tart reports missing-agent or unsupported
host errors inside the shell session. A shell is offered only for a running
local VM; opening it never starts a stopped VM.

## Manual verification on macOS

Use disposable VMs and cached images for destructive checks.

1. Check `tart --version` and `tart list --format json`, then launch Tuivir.
   Confirm Tart appears with its version and that VMs and cached Images match
   the CLI listing. With no Resources, both panels should remain usable.
2. Select a VM and open Config. Compare its CPU, Memory, Disk, and Display
   fields with `tart get <name> --format json`. Select an image and open Info.
3. Start a stopped VM with `S`. Check it becomes running. Quit Tuivir and
   verify the VM stays running with `tart list`; reopen Tuivir and stop it
   with `s`.
4. For a macOS VM launched externally with `tart run --suspendable`, suspend it
   with `tart suspend <name>`. Confirm Tuivir shows suspended and `p` restores
   it. Stopped VMs should offer Start instead of Resume.
5. For a VM with Guest Agent, use `E`, run a command, and leave shell input with
   `Ctrl-T q`. Check the session continues when navigating away and back.
6. Cancel a VM deletion and verify the VM remains. Confirm deletion of a
   disposable running VM, and check it is stopped and removed. Delete a
   disposable cached image and check only that cache entry is removed.

Automated tests use CLI fixtures checked against upstream Tart's List, Get,
Run, Stop, Delete, and Exec implementations, plus real local subprocess tests
for detached launches. These checks do not replace real Tart verification on
macOS.
