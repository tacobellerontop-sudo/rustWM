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

## Using it

```powershell
cargo build --release              # or download a release zip
.\target\release\rwm.exe start     # starts rustwm.exe in the background, with a tray icon
.\target\release\rwm.exe stop      # quits and restores every window
rwm autostart on                   # start at login
```

Or just double-click `rwm.exe` (or run it with no arguments) for a menu: start/stop,
edit and check the config, see your keybindings, autostart, and "Add rwm to PATH" so
`rwm` works without `.\` in any new terminal.

Keep both `.exe` files in the same folder. Logs go to
`%LOCALAPPDATA%\rustwm\rustwm.log`; run `rustwm.exe` directly to see them in a console.

On first run rustWM writes a commented config to
`%USERPROFILE%\.config\rustwm\rustwm.toml`. Edits are picked up when you save; a
broken file is reported in the log and the previous config keeps running.

```toml
[general]
default_layout = "dwindle"       # dwindle | master | monocle
new_window = "after-focused"     # after-focused | end | start
workspaces = ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
workspace_mode = "per-monitor"   # per-monitor | global (Win+Alt+W toggles)
terminal = "wezterm-gui.exe"     # Win+Enter; defaults to wt.exe, then cmd.exe

[gaps]
inner = 8
outer = 8
smart = true

[bindings]                       # merged over the defaults; "none" unbinds
"win+t" = "exec wezterm-gui.exe start --cwd ~"

[[rules]]
match = { exe = "Spotify.exe" }  # exe/class: whole-value regex; title: regex search
actions = ["workspace 9"]        # float | ignore | tile | workspace <name>
```

### Windows that can't be resized

Some windows (launchers like the Riot Client, dialogs) have a fixed or minimum size.
rustWM floats them on its own: windows without a sizing border float when they open,
and a tiled window that doesn't take the size it was given is floated, centred, about a
second later. Riot Client and the League client float by default. To change what
happens to an app, add a rule:

```toml
[[rules]]
match = { exe = "SomeLauncher.exe" }
actions = ["float"]              # always float it

[[rules]]
match = { exe = "MyEditor.exe" }
actions = ["tile"]               # always tile it, even if it looks fixed-size
```

Tiling a floated window yourself (Win + F) also keeps it tiled.

### Workspaces on several monitors

Every monitor has its own copy of workspaces 1–9. `workspace_mode` decides how they switch:

- **per-monitor** (default): Win + 1…9 switches only the monitor you're on. That's the
  monitor of the focused window, or the one under the mouse after you click its desktop
  or taskbar. New windows open on that monitor too.
- **global**: Win + 1…9 switches every monitor at once, so all your screens act as one big
  desktop. Windows stay on the monitor they're on; sending a window to workspace 3 keeps it
  on its monitor's half of workspace 3.

Win + Alt + W (`workspace-mode toggle`) flips between them until the next reload;
set `workspace_mode` in the config to keep it.

### Resizing with the mouse

Drag the edge of a tiled window to resize it; its neighbours follow and the split is
remembered. Dragging from the title bar moves it instead: drop it on another window to
swap them, or on another monitor to move it there. Floating windows move and resize
freely.

### Default keys

| Keys | Command |
|---|---|
| Win + H/J/K/L or arrows | focus left / down / up / right (crosses monitors) |
| Win + Shift + H/J/K/L or arrows | move (swap) left / down / up / right |
| Win + N / P | focus next / previous |
| Win + 1…9 | switch workspace (this monitor, or all of them in global mode) |
| Win + Shift + 1…9 | send window to workspace |
| Win + Ctrl + Left/Right | focus monitor |
| Win + Ctrl + Shift + Left/Right | send window to monitor |
| Win + Enter | open your terminal |
| Win + Shift + Enter | promote to the first slot |
| Win + Space | cycle layout (dwindle → master → monocle) |
| Win + F / Win + Shift + F | toggle floating / fullscreen |
| Win + = / Win + - | grow / shrink the focused window |
| Win + Shift + Q | close window |
| Win + Shift + R | reload config |
| Win + ` / Win + Shift + ` | toggle the scratchpad / send window to it |
| Win + Alt + R | resize mode: H/J/K/L or arrows, Esc or Enter to leave |
| Win + Ctrl + R | retile |
| Win + Alt + W | toggle global / per-monitor workspaces |
| Win + Ctrl + Shift + Q | quit and restore all windows |

Dragging a tiled window onto another swaps them; dropping it on another monitor moves it
there.

### Commands

Bindings and `rwm <command>` use one command language:
`focus <dir|next|prev>`, `move <dir|next|prev>`, `promote`, `close`, `terminal`,
`exec <command line>`, `workspace <name|n>`, `move-to-workspace <name|n>`,
`workspace-mode <global|per-monitor|toggle>`,
`focus-monitor <dir>`, `move-to-monitor <dir>`, `layout <dwindle|master|monocle|reset>`,
`cycle-layout`, `resize <grow|shrink> [amount]`, `toggle-float`, `toggle-fullscreen`,
`toggle-special [name]`, `mode <name>`, `retile`, `reload`, `restore`, `quit`.
`move-to-workspace special:<name>` sends a window to a special workspace, and rules can use
`workspace special:<name>` too. An empty special workspace opens your terminal when shown.

### Scripting and status bars

`rwm` talks to rustWM over the named pipe `\\.\pipe\rustwm` with newline-delimited JSON,
and anything else (a status bar like Zebar or YASB, a script) can use the same pipe:

```text
rwm query state | windows | focused | config | version
rwm subscribe [focus] [workspace] [window]     # one JSON line per change
rwm check-config [path]
rwm restore                                    # works even if rustWM crashed
```

| Request | Reply |
|---|---|
| `{"cmd": "focus left"}` | `{"v":1,"ok":true}` once the command ran, or `{"v":1,"ok":false,"error":"..."}` |
| `{"query": "state"}` | `{"v":1,"ok":true,"data":{...}}`: monitors, workspaces, windows, focus, mode |
| `{"subscribe": ["workspace","focus"]}` | `{"v":1,"ok":true}`, then `{"v":1,"event":"init","state":{...}}` and one event per change |

### Known limitations

- **Win + L** always locks the PC; Windows does not let any app override it. To use it
  for focus, set the `DisableLockWorkstation` registry value
  (`HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System`), or use Win + Right.
- Windows of elevated (admin) apps cannot be moved by a non-elevated rustWM.
- Windows on inactive workspaces are hidden, so they leave the taskbar until you switch back.

## Milestones

- [x] **M0 Skeleton**: workspace, CI on Windows and Linux, logging, `Platform` trait, fake platform.
- [x] **M1 See and move**: enumerate and filter windows, WinEvent hook, monitors and DPI, move/resize, `rwm restore`.
- [x] **M2 Dwindle tiling**: workspaces, dwindle layout, gaps, auto-tile, geometric focus and swap, hotkeys.
- [x] **M3 Config, workspaces, layouts**: TOML + hot reload, `terminal` command, rules, 1–9 workspaces, multi-monitor, master-stack, monocle, floating, fullscreen.
- [x] **M4 IPC and polish**: named pipe, `rwm` CLI, query and subscribe, special workspace, resize mode, tray, autostart.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
