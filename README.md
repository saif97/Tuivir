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

```toml
[keybindings]
"app.quit" = ["q"]
"app.help" = ["?"]
"app.refresh" = ["ctrl+r"]

"focus.providers" = ["1"]
"focus.resources" = ["2"]
"focus.resources.2" = ["3"]
"focus.resources.3" = ["4"]
"focus.resources.4" = ["5"]
"focus.resources.5" = ["6"]
"focus.resources.6" = ["7"]
"focus.resources.7" = ["8"]
"focus.resources.8" = ["9"]
"focus.resources.9" = ["0"]
"focus.details" = ["enter"]
"focus.next" = ["tab"]
"focus.previous" = ["shift+tab"]

"selection.next" = ["j", "down"]
"selection.previous" = ["k", "up"]
"selection.next.fast" = ["J"]
"selection.previous.fast" = ["K"]
"workspace.next" = ["]"]
"workspace.previous" = ["["]

"detail.view.next" = ["l", "right"]
"detail.view.previous" = ["h", "left"]
"detail.scroll.down" = ["ctrl+d", "pagedown"]
"detail.scroll.up" = ["ctrl+u", "pageup"]
"details.copy" = ["y"]
"layout.boundary.left" = ["<"]
"layout.boundary.right" = [">"]

"modal.confirm" = ["y", "enter"]
"modal.cancel" = ["esc", "n"]

"resource.start" = ["S"]
"resource.stop" = ["s"]
"resource.restart" = ["r"]
"resource.resume" = ["p"]
"resource.shell" = ["E"]
"resource.delete" = ["d"]
```

The file may contain only the commands you want to change. A configured list
replaces that command's complete default list; an empty list disables the
command. Keys are case-sensitive and support printable characters, named keys
such as `enter` and `pagedown`, `f1` through `f12`, and `ctrl+` or `alt+`
modifiers. `ctrl+c` is always reserved as an emergency way to quit Tuivir.
Invalid or conflicting bindings are reported at startup and none of the file is
applied.

## Usage

Run Tuivir in a terminal with a supported provider CLI installed:

```sh
tuivir
```

The interface shows the available commands and their key bindings in the
footer. Run `tuivir --help` for command-line options.

## Releasing

A maintainer publishes the Cargo version by pushing its matching stable tag,
such as `v0.1.0`. If publication fails and leaves a draft GitHub Release,
delete that draft and rerun the same GitHub Actions run. Correct an already
published release with a new version rather than replacing its tag or assets.

Tuivir is available under either the MIT License or the Apache License, Version
2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
