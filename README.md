# Tuivir

Tuivir is a terminal environment for inspecting and operating resources managed
by local virtualization and container Providers.

## Source builds

Tuivir requires Rust `1.97.1`. With the pinned toolchain installed, build and
test the locked dependency set with:

```sh
cargo build --locked
cargo test --locked
```

Run the safe installation check without loading configuration, discovering a
Provider, or initializing the terminal:

```sh
cargo run --locked -- --version
```

Provider CLIs remain optional. Tuivir discovers Docker, Incus, and other
supported Providers at runtime; they are not build or package dependencies.

## Release archives

Download the archive for your system from
[GitHub Releases](https://github.com/saif97/Tuivir/releases):

- Apple Silicon macOS: `aarch64-apple-darwin`
- Intel macOS: `x86_64-apple-darwin`
- x86-64 Omarchy or Arch Linux: `x86_64-unknown-linux-musl`

Compare the archive's SHA-256 value with the release's `SHA256SUMS`, then
extract and install it. For example, replace `<version>` and `<target>` below:

```sh
archive="tuivir-v<version>-<target>.tar.gz"
tar -xzf "$archive"
mkdir -p "$HOME/.local/bin"
install -m 755 "${archive%.tar.gz}/tuivir" "$HOME/.local/bin/tuivir"
tuivir --version
```

`~/.local/bin` must be on `PATH`. Provider CLIs remain separate optional
installations.

## Homebrew

On Apple Silicon or Intel macOS, install Tuivir from its Homebrew tap:

```sh
brew install saif97/tap/tuivir
```

Upgrade it with:

```sh
brew update
brew upgrade tuivir
```

Omarchy installation through the AUR package `tuivir-bin` is planned but is
not live yet.

## Releasing

A maintainer publishes the Cargo version by pushing its matching stable tag,
such as `v0.1.0`. If publication fails and leaves a draft GitHub Release,
delete that draft and rerun the same GitHub Actions run. Correct an already
published release with a new version rather than replacing its tag or assets.

Tuivir is available under either the MIT License or the Apache License, Version
2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
