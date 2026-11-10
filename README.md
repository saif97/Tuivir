# Tuivir

A terminal UI for managing VMs, containers, and microVMs.

![Tuivir displaying Docker containers, images, volumes, and resource details](https://github.com/user-attachments/assets/0ec5cbcf-2fbe-4e95-a393-72ff6e6b54c5)

Works with Docker, Incus, Docker Sandbox, and Tart. Apple Container support is
experimental.

## Install

**Homebrew (macOS)**

```sh
brew install saif97/tap/tuivir
```

**From source (Linux or macOS)** — requires [Rust](https://rustup.rs/).

```sh
git clone https://github.com/saif97/Tuivir.git
cd Tuivir
cargo install --locked --path .
```

Ensure `~/.cargo/bin` is on your `PATH`.

## Usage

Install your provider's CLI and start its service, then run:

```sh
tuivir
```

Tuivir discovers installed providers automatically. Browse resources, inspect
logs and stats, open shells, and manage resource lifecycles. Supported resources
also offer CPU and memory editing in **Settings**.

Press `?` for help or `q` to quit. The footer shows available commands.

## Configuration

Create `~/.config/tuivir/config.toml` to customize keybindings:

```toml
[keybindings]
app_quit = ["q"]
app_refresh = ["ctrl+r"]
```

Only include commands you want to change. Each list replaces the defaults;
`[]` disables a command. Keys are case-sensitive.

Tuivir also respects `$XDG_CONFIG_HOME/tuivir/config.toml`. Set
`TUIVIR_CONFIG_FILE` to an absolute path to use another file.

## Development

Run tests with `cargo test --locked`. See [Releasing](docs/releasing.md) for the
release process.

## License

[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
