# Ghostty terminal-engine research for issue #99

_Checked 2026-08-21. This is a source-and-build review only: no Zig compiler, Ghostty source, or new dependency was installed, and no Ghostty harness was run. It identifies integration risk; it is not a replacement for ADR 0011's live acceptance comparison._

## Recommendation

**Keep `alacritty_terminal` as the leading candidate.** Do not start a Ghostty implementation just for feature parity. The existing Alacritty harness is a 536-line, Cargo-only prototype and has already run `btop` successfully. Ghostty is the right fallback if the Alacritty acceptance pass identifies a specific fidelity failure it plausibly fixes, particularly Unicode graphemes or keyboard/mouse/query modes.

Zig is a **build and maintenance** cost, not a runtime requirement for the finished executable: a static release need not ask an end user to install Zig. However, every development and CI/release environment that builds Tuivir needs the compatible Zig version and the pinned Ghostty source/dependency inputs.

## What Ghostty supplies—and what it does not

The relevant component is `libghostty-vt`, not Ghostty's GUI. It is a C ABI library for VT parsing, terminal state (screen, scrollback, cursor, styles, modes), resize/reflow, and input encoding. It exposes render state for an embedder to draw; it does **not** provide a Ratatui widget, a cell renderer, or a PTY/process manager. ([upstream C API overview](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h), [upstream render API](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/render.h), [Ghostling limitations](https://github.com/ghostty-org/ghostling/blob/main/README.md#limitations))

Tuivir would still need to own:

- a PTY (for example `portable-pty`) to spawn, resize, drain, and write the provider-declared Resource Shell Session;
- a terminal-runtime owner for `Terminal`, input encoders, and render snapshots;
- a Ghostty-cell-to-Ratatui-buffer adapter, including wide-cell/grapheme and cursor projection; and
- an effect bridge which queues terminal replies to the PTY and ignores clipboard writes as ADR 0011 requires.

This is fully feasible, but is more host code than the proven Alacritty route. Terminal effects (including device-attribute and size-query replies) are off by default. Enabled callbacks run synchronously during a VT write, must not re-enter a write on the same terminal, and must not block. The bridge must therefore enqueue a reply rather than synchronously write to a contended PTY. The library also starts no background timer or thread for scrollback compression; its embedder schedules it. ([effects and caller-owned compression](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/terminal.h#L40-L79))

## Build, packaging, and FFI cost

Upstream can build the VT library alone, but its CMake integration finds Zig as a required program and invokes `zig build -Demit-lib-vt`. Current Ghostty `main` declares Zig **0.16.0 or newer**. CMake does not remove the Zig dependency; it is a wrapper around the Zig build. ([upstream CMake contract](https://github.com/ghostty-org/ghostty/blob/main/dist/cmake/README.md), [upstream build declaration](https://github.com/ghostty-org/ghostty/blob/main/build.zig.zon), [CMake implementation](https://github.com/ghostty-org/ghostty/blob/main/CMakeLists.txt#L1128-L1189))

There is no upstream Rust crate. Ghostty's reference project says it cannot promise official non-C/Zig bindings. The practical Rust option is the community-maintained `libghostty-vt` wrapper: raw bindings are generated from `ghostty/vt.h`, with safe `Terminal`, `RenderState`, key, and mouse APIs. ([Ghostling on bindings](https://github.com/ghostty-org/ghostling/blob/main/README.md#L130-L151), [binding project](https://github.com/Uzaaft/libghostty-rs))

That wrapper still requires Zig 0.16.x and, by default, fetches a **pinned Ghostty commit at build time**. It documents `GHOSTTY_SOURCE_DIR` and `GHOSTTY_ZIG_SYSTEM_DIR` overrides for reproducible, network-free package builds. It statically links by default; dynamic linking adds loader-path packaging on Linux and macOS. ([binding build contract](https://github.com/Uzaaft/libghostty-rs#building))

For a responsible Tuivir integration, vendor/pin an upstream source distribution (or rigorously pin the wrapper source input), pin Zig in CI, and prohibit a Cargo build from silently downloading native build inputs. This is continuing release work, not one developer setup step. The C header calls the API incomplete and unstable, with breaking changes expected; upstream also says the library has not been tagged. Each pin bump needs the complete acceptance pass. ([API stability warning](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h#L9-L24), [upstream status](https://github.com/ghostty-org/ghostty#cross-platform-libghostty-for-embeddable-terminals))

Herdr demonstrates the operational cost in a nearby Rust project. It vendors a specific Ghostty distribution snapshot, maps Rust targets to Zig targets, invokes `zig build -Demit-lib-vt` from `build.rs`, and statically links the archive. Its pinned snapshot has a local patch so DEC mode 2027 grapheme clustering survives RIS. A source pin makes that sort of terminal-behaviour delta Tuivir's responsibility. ([Herdr build script](../../../opensource/herdr/build.rs), [vendor metadata](../../../opensource/herdr/vendor/libghostty-vt.vendor.json), [patch inventory](../../../opensource/herdr/vendor/libghostty-vt.patches.md))

Herdr's CI currently pins Zig 0.15.2 for *its* vendored source, whereas the current community wrapper requires 0.16.x. Do not copy Herdr's version by name: the Ghostty source revision, wrapper bindings, Zig version, and archive form one compatibility unit. ([Herdr CI](../../../opensource/herdr/.github/workflows/ci.yml), [wrapper build requirements](https://github.com/Uzaaft/libghostty-rs#building))

## Runtime ownership wrinkle

The safe wrapper intentionally makes its types `!Send` and `!Sync`: the authors do not assume that the C API can be moved or shared across threads. They recommend one dedicated OS thread/task and channels to communicate with the application. That fits ADR 0005's single-owner model, but fixes the design: a runtime thread must own each terminal and serially process PTY bytes, resize and input requests, effects, and render snapshots. Tuivir must not move `Terminal` through Tokio tasks or place it in the shared locks used by the Alacritty spike. ([binding thread-safety contract](https://docs.rs/libghostty-vt/latest/libghostty_vt/#thread-safety))

This is manageable rather than blocking. It means Ghostty's fidelity comes with a stricter concurrency boundary and a separate PTY implementation. Ghostty does not create those threads for the embedder.

## Comparison with the Alacritty prototype

| Concern | `alacritty_terminal` (current prototype) | `libghostty-vt` |
| --- | --- | --- |
| Build | Cargo dependency already in Tuivir; no foreign toolchain. | Zig native build plus C ABI/static archive; pin and package source inputs. |
| PTY/runtime | Supplies platform PTY types and an event loop, used by the spike. | VT engine only; Tuivir owns PTY, reader/writer, child lifecycle, and runtime thread. |
| Rendering | Custom cell adapter already exists in the spike. | Same custom adapter work; Ghostty supplies render state, not a Ratatui renderer. |
| Replies/input | Existing spike routes PTY replies, resize, focus, paste, keyboard, and mouse. | APIs exist, but callbacks and encoders must be connected; reentrancy rules apply. |
| Thread model | Spike uses Alacritty's event loop and a mutex-protected terminal. | Binding is `!Send`/`!Sync`; terminal owns a dedicated runtime thread and channels. |
| Maintenance | Rust dependency released alongside Alacritty. | Explicitly unstable C API plus wrapper/source/Zig compatibility matrix. |

Alacritty remains substantial product work: the session lifecycle, hidden-output draining, custom rendering, and input routing in ADR 0011 are common to both options. It is lower risk because it is already a compiled dependency here and has a working interactive `btop` path. Its published terminal crate includes the PTY support and event loop that the prototype uses. ([Alacritty terminal crate](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/Cargo.toml), [event-loop implementation](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/event_loop.rs), [local prototype](alacritty-terminal-spike.md))

## Decision gate

Do **not** install Zig or vendor Ghostty now. Finish the documented Alacritty acceptance script with Neovim, Docker, Unicode-after-RIS, repeated resize, and hidden output. Open a Ghostty spike only for a concrete Alacritty failure.

If that happens, time-box the Ghostty harness and require all of these before selecting it:

1. pinned upstream source, pinned Zig version, and an offline/reproducible CI build;
2. one terminal-runtime-thread design with channel-based requests/snapshots and nonblocking effect-to-PTY writeback;
3. the same local and Docker acceptance script, including `nvim`, `btop`, RIS, and clean restart; and
4. an explicit owner and regression suite for every downstream patch required across an upstream pin bump.

Without that demonstrated Alacritty gap, Ghostty is extra terminal surface that Ratatui cannot fully expose (notably inline graphics), purchased with a material toolchain and maintenance burden. It is a good escape hatch, not the default production stack for issue #99.
