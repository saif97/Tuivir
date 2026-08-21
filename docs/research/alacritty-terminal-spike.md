# Alacritty terminal spike

**Throwaway prototype for issue #99.** This is not Tuivir product code and must
not be wired into the application. It exists to evaluate the `alacritty_terminal`
candidate under real interactive programs.

Run a local shell:

```sh
cargo run --bin alacritty_terminal_spike
```

Run `btop` directly:

```sh
cargo run --bin alacritty_terminal_spike -- --btop
```

Run a shell in an existing Docker container:

```sh
cargo run --bin alacritty_terminal_spike -- --docker CONTAINER sh
```

`Ctrl-G` exits the harness and `Ctrl-H` hides/shows the terminal grid; every
other input is directed to the PTY. The prototype has no chrome so terminal
applications receive the entire screen.
It drains the PTY continuously, including while the outer renderer is between
frames, and sends resize, focus, bracketed-paste, mouse, and terminal-query
responses through Alacritty's event loop.

## Acceptance script

Run each check both locally and via Docker. Record observed behavior in
`embedded-terminal-options.md` before selecting an engine.

1. Change directories and run a continuous producer such as `while true; do date; sleep 1; done`; use `Ctrl-H`, wait, then use it again to confirm output continued to change while hidden.
2. Run `nvim` and check normal/insert modes, arrows, function keys, `Esc`, `Ctrl-C`, colors, cursor, alternate-screen entry/exit, and several resizes.
3. Run `btop` (or `htop`) and check redraw, wheel/click/drag reporting, and resize.
4. Paste multiline text containing `e\u{301}`, emoji, flags, and ZWJ sequences.
5. Run `printf '\\033[c\\033[18t\\033[6n'` and confirm the process gets terminal replies instead of hanging.
6. Run `printf '\\033c'`; repeat the Unicode check to validate reset behavior.
7. Exit the shell and relaunch the command to confirm a clean new PTY session.
