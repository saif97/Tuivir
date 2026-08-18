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

### Release archives

Download the archive for your system from
[GitHub Releases](https://github.com/saif97/Tuivir/releases):

- Apple Silicon macOS: `aarch64-apple-darwin`
- Intel macOS: `x86_64-apple-darwin`
- x86-64 Omarchy or Arch Linux: `x86_64-unknown-linux-musl`

Compare the archive's SHA-256 value with the release's `SHA256SUMS`, then
extract and install it. Replace `<version>` and `<target>` below:

```sh
archive="tuivir-v<version>-<target>.tar.gz"
tar -xzf "$archive"
mkdir -p "$HOME/.local/bin"
install -m 755 "${archive%.tar.gz}/tuivir" "$HOME/.local/bin/tuivir"
tuivir --version
```

`~/.local/bin` must be on `PATH`. Omarchy installation through the AUR package
`tuivir-bin` is planned but is not live yet.

### Build from source

Tuivir requires Rust `1.97.1`. With the pinned toolchain installed, build and
test the locked dependency set:

```sh
cargo build --locked
cargo test --locked
```

Check the resulting binary without loading configuration, discovering a
provider, or initializing the terminal:

```sh
cargo run --locked -- --version
```

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
