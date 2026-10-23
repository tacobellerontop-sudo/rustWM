# rustWM

A keyboard-driven, dynamic tiling window manager for Windows 10/11, written in Rust.
Inspired by komorebi and Hyprland: windows sit in an ordered list per workspace and a
layout (dwindle, master-stack, monocle) computes where they go. One TOML config, built-in
hotkeys, and an `rwm` CLI over a named pipe.

> Work in progress. See the milestones below for what works today.

## Layout of the repo

| Crate | Job |
|---|---|
| `rwm-core` | Pure Rust: window state, layouts, commands, rules. No Win32, tested on any OS through a fake `Platform`. |
| `rwm-win32` | The real `Platform` over the `windows` crate: enumeration, hooks, hotkeys, monitors, DPI, positioning. |
| `rwm-config` | TOML schema, defaults, validation, keybinding parser. |
| `rwm-ipc` | JSON message types and the named-pipe client/server (`\\.\pipe\rustwm`). |
| `rustwm` | The daemon binary (`rustwm.exe`). |
| `rwm-cli` | The `rwm.exe` command users and scripts call. |

## Building

```sh
cargo build --release
cargo test
```

The daemon only runs on Windows, but `cargo test` works on Linux and macOS too.
Logging is controlled with `RUST_LOG`, e.g. `RUST_LOG=rwm_core=debug`.

## Trying it (M1)

```powershell
cargo build --release
.\target\release\rustwm.exe --debug-tile   # tiles every window side by side; Ctrl+C to quit
.\target\release\rwm.exe restore           # shows any window rustWM hid, even after a crash
```

## Milestones

- [x] **M0 Skeleton**: workspace, CI on Windows and Linux, logging, `Platform` trait, fake platform.
- [x] **M1 See and move**: enumerate and filter windows, WinEvent hook, monitors and DPI, move/resize, `rwm restore`.
- [ ] **M2 Dwindle tiling**: workspaces, dwindle layout, gaps, auto-tile, geometric focus and swap, hotkeys.
- [ ] **M3 Config, workspaces, layouts**: TOML + hot reload, `terminal` command, rules, 1–9 workspaces, multi-monitor, master-stack, monocle, floating, fullscreen.
- [ ] **M4 IPC and polish**: named pipe, `rwm` CLI, query and subscribe, special workspace, resize mode, tray, autostart.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
