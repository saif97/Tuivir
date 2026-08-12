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

## Planned distribution

The following supported distribution surfaces are planned but are not live yet:

- Homebrew through the `saif97/homebrew-tap` tap for macOS.
- Omarchy through the `tuivir-bin` package in the Arch User Repository (AUR).
- Versioned release archives from GitHub Releases for manual installation.

Until those surfaces are published, build Tuivir from source with the pinned
toolchain above.

Tuivir is available under either the MIT License or the Apache License, Version
2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
