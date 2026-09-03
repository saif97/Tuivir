# Tuivir

A terminal interface for inspecting and operating resources managed by local
virtualization and container providers.

<p align="center">
  <img
    src="https://github.com/user-attachments/assets/c6f9713c-a175-4d1a-961a-dd24618eaecc"
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

Upgrade an existing installation with:

```sh
brew update
brew upgrade tuivir
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

focus_providers = ["1"]
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
prefix = "ctrl+]"

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

### Resource Shell Sessions

Resources that support a shell expose a Shell Detail View Tab. Select that tab
and press Enter to start its embedded Resource Shell Session in Details, or
press `E` from a Resource to start or resume it enlarged. The same session stays
alive while you navigate and moves between presentations without restarting.

When terminal input is focused, keys — including `Ctrl-C` — go to the shell.
The Shell Prefix defaults to `Ctrl-]`; use `Ctrl-] q` to return input to
Tuivir (and restore Details from an enlarged session), or `Ctrl-] z` to change
the current session's presentation without releasing keyboard focus. Configure
`resource_shell.prefix` with any supported key, and configure the ordered
`focus_tuivir` and `toggle_zoom` Keybindings under
`resource_shell.keybindings`. The first Keybinding is the inline hint;
`focus_tuivir` must contain a key and an empty `toggle_zoom` list disables it.
Repeating the Shell Prefix sends it literally to the Resource Shell Session,
and any other following key sends both keys to the session. Quitting Tuivir
asks for confirmation when Resource Shell Sessions are still live;
confirmation ends only their local Provider CLI processes, not the underlying
Resources.

## Releasing

A maintainer publishes the Cargo version by pushing its matching stable tag,
such as `v0.1.0`. If publication fails and leaves a draft GitHub Release,
delete that draft and rerun the same GitHub Actions run. Correct an already
published release with a new version rather than replacing its tag or assets.

Tuivir is available under either the MIT License or the Apache License, Version
2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
