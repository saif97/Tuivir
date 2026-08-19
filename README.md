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

Tuivir requires Rust `1.97.1`. Clone the repository and install its locked
dependency set with Cargo:

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
