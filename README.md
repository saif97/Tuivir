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

## Distribution

GitHub Releases are the canonical source of verified archives. Package managers
download those exact archives; they do not compile Tuivir from source.

- Homebrew on supported Macs: `brew tap saif97/tap && brew install tuivir`
- Omarchy/AUR: `yay -S tuivir-bin`
- Other Linux distributions: download the x86-64 musl archive from GitHub
  Releases and install `tuivir` manually.

Homebrew and AUR own upgrades: use their normal `brew upgrade` or AUR-helper
commands. There is no in-app self-updater.

Tuivir is available under either the MIT License or the Apache License, Version
2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
