# Tuivir

> “Rhymes” with *tweezer* & *beaver*.

A provider-agnostic TUI for managing and inspecting VMs, containers, and microVMs.

<p align="center">
  <img
    src="https://github.com/user-attachments/assets/0ec5cbcf-2fbe-4e95-a393-72ff6e6b54c5"
    alt="Tuivir displaying Docker containers, images, volumes, and resource details"
    width="100%"
  />
</p>

Tuivir discovers supported providers at runtime and presents their resources in
one keyboard-friendly workspace. Provider CLIs such as Docker and Incus remain
separate, optional installations.

## Installation

### Homebrew

On Apple Silicon or Intel macOS:

```sh
brew install saif97/tap/tuivir
```

### Build from source

Tuivir uses the Rust toolchain pinned in `rust-toolchain.toml`. Clone the
repository and install its locked dependency set with Cargo:

```sh
git clone https://github.com/saif97/Tuivir.git
cd Tuivir
cargo install --locked --path .
```

`cargo install` builds an optimized binary and installs it to Cargo's bin
directory, normally `~/.cargo/bin`. Ensure that directory is on `PATH`, then
check the installation:

```sh
tuivir --version
```

Contributors can run the locked test suite with `cargo test --locked`.

## Configuration

Tuivir currently supports keybinding configuration. It does not create a
configuration file automatically, so create the directory and file when you
want to override a default:

```sh
mkdir -p "$HOME/.config/tuivir"
${EDITOR:-vi} "$HOME/.config/tuivir/config.toml"
```

Tuivir reads `$XDG_CONFIG_HOME/tuivir/config.toml` when `XDG_CONFIG_HOME` is set
to an absolute path; otherwise it reads `~/.config/tuivir/config.toml`. Set
`TUIVIR_CONFIG_FILE` to an absolute file path to use a different location.
A missing discovered file means the compiled defaults below are used.

<details>
<summary>Default keybindings</summary>

```toml
[keybindings]
app_quit = ["q"]
app_help = ["?"]
app_refresh = ["ctrl+r"]

focus_resources = ["2"]
focus_resources_2 = ["3"]
focus_resources_3 = ["4"]
focus_resources_4 = ["5"]
focus_resources_5 = ["6"]
focus_resources_6 = ["7"]
focus_resources_7 = ["8"]
focus_resources_8 = ["9"]
focus_resources_9 = ["0"]
focus_details = ["enter"]
focus_next = ["tab"]
focus_previous = ["shift+tab"]

selection_next = ["j", "down"]
selection_previous = ["k", "up"]
selection_next_fast = ["J"]
selection_previous_fast = ["K"]
workspace_next = ["]"]
workspace_previous = ["["]

detail_view_next = ["l", "right"]
detail_view_previous = ["h", "left"]
detail_scroll_down = ["ctrl+d", "pagedown"]
detail_scroll_up = ["ctrl+u", "pageup"]
details_copy = ["y"]
configuration_apply = ["ctrl+a"]
configuration_edit = ["enter"]
configuration_next_field = ["down", "j"]
configuration_previous_field = ["up", "k"]
configuration_finish_field = ["enter", "esc"]
configuration_backspace = ["backspace"]
configuration_clear_field = ["ctrl+u"]
layout_boundary_left = ["<"]
layout_boundary_right = [">"]

modal_confirm = ["y", "enter"]
modal_cancel = ["esc", "n"]

resource_start = ["S"]
resource_stop = ["s"]
resource_restart = ["r"]
resource_resume = ["p"]
resource_shell = ["E"]
resource_delete = ["d"]

[resource_shell]
prefix = "ctrl+t"

[resource_shell.keybindings]
focus_tuivir = ["q"]
toggle_zoom = ["z"]
```

</details>

The file may contain only the commands you want to change. A configured list
replaces that command's complete default list; an empty list disables the
command. Keys are case-sensitive and support printable characters, named keys
such as `enter` and `pagedown`, `f1` through `f12`, and `ctrl+` or `alt+`
modifiers. Outside a focused Resource Shell Session, `ctrl+c` is reserved as
an emergency way to quit Tuivir.
Invalid or conflicting bindings are reported at startup and none of the file is
applied.

## Usage

Run Tuivir in a terminal with a supported provider CLI installed:

```sh
tuivir
```

The interface shows the available commands and their key bindings in the
footer. Run `tuivir --help` for command-line options.

### Resource CPU and memory

Select a Docker container, Incus instance, or local Tart VM, focus Details,
and move to the Settings tab with the left/right keys. Editable settings appear
first. The same tab shows the remaining native configuration under read-only
settings, including nested fields; CPU/memory controls are not repeated there.
Settings replaces the separate Config/Inspect tab for these Resources and Apple
Containers. Other Resource kinds keep their native inspection views.

Page Up/Page Down, Ctrl+U/Ctrl+D, or the mouse wheel scroll long read-only content
while editable controls remain visible. Read-only settings do not receive editing
focus. Native field paths identify nested values; arrays retain their JSON values.

Use up/down to select a field and Enter to edit. Backspace removes a character;
Ctrl+U clears the input. Enter or Esc keeps the draft without applying it.
Ctrl+A opens **Review changes**, showing the selected Resource, old/new values,
and any necessary downtime. Confirm Apply changes with Enter or y, or cancel
with Esc or n. Hints follow customized Keybindings.

Memory defaults to **GiB**: enter `0.5`, `1.5`, or `8`, for example. Tuivir
converts to the Provider's native units. Tart changes must resolve to whole MiB;
guest-specific minimums are checked by Tart. Docker memory can be `0` for
unlimited; nonzero limits must be at least 6 MiB and fit the existing memory+swap
limit. Incus also accepts whole percentages such as `50%`.

Docker CPU limits accept decimal CPUs (`0` is unlimited). Tart accepts positive
whole CPU counts. Incus accepts CPU counts or CPU IDs/ranges; supported servers
also accept VM CPU topology syntax. Incus edits set instance overrides without
changing shared profiles. Running Tart VMs stop/start for CPU/memory changes;
stopped VMs stay stopped.

Docker Sandbox limits are read-only where reported. Apple Container shows
inspected CPU count and memory in GiB read-only: its CLI has no supported
in-place CPU/memory resize operation for existing Containers. Configure those
resources at creation outside Tuivir. Disk sizing is outside this feature.

Drafts survive Resource and Provider Workspace navigation for the current
session. Refresh with Ctrl+R to reload actual values while keeping edits.
Apply re-reads edited fields and refuses external conflicts until you review
again. After failures, actual values and Resource State are refreshed; saved
fields become clean and remaining attempted values stay in the draft. Required
stop/start operates on the same Resource and preserves its original running or
stopped state on success. Restart failures remain visible even when settings
were saved. Automatic rollback and draft persistence across restarts are not
provided.

### Apple Container

The Apple Container Provider Workspace operates existing Containers, Images,
Networks, and Volumes through the current user's local `container` CLI and its
inherited settings. The targeted baseline is Apple Silicon, macOS 26, and CLI
1.5.0. Other installed CLI versions attempt normal operations; older-version
compatibility is not promised.

Start the service explicitly outside Tuivir with `container system start`.
An installed CLI remains visible when the service is unavailable; refresh the
Workspace after starting the service to recover. Tuivir does not administer the
service or create Resources.

Running Containers offer Stop, composed Stop-then-Start Restart, Delete, and an
embedded `/bin/sh` Shell. Stopped Containers offer Start and Delete. Delete uses
the usual confirmation, forcing container deletion only when the last reported
Resource State is not positively stopped. Built-in Networks are inspectable
without Delete; dependency refusals are preserved. Image deletion verifies the
requested reference rather than relying on exit status or filtered listings.

Containers provide the latest 200 process Logs, separate VM Boot Logs, finite
Stats, and Settings. Images, Networks, and Volumes provide Inspect. Resource Shell
Sessions use the existing navigation, input controls, and cleanup behavior.

Automated fixture validation is separate from live Mac verification, which is
pending the [supported-macOS smoke checklist](docs/apple-container-smoke.md).

## Provider roadmap
- [x] Docker
- [x] Incus
- [x] Docker Sandbox
- [x] Tart
- [ ] Apple `container`
- [ ] Podman

- [x] Edit Resource CPU and memory configuration

Tuivir is available under either the MIT License or the Apache License, Version
2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
