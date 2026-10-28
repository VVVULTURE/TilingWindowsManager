# Architecture

```
            ┌──────────────── twm.exe (one per user session) ─────────────────┐
 Win32 ───► │ platform layer (crates/twm/src/win)      daemon.rs               │
 events     │  events.rs   WinEvent hook ──► queue ──► handle events ──┐       │
 keys  ───► │  keyboard.rs LL hook thread ─► WM_APP_BIND ─────────────┤       │
 IPC   ───► │  ipc.rs      pipe thread ───► WM_APP+2 ─────────────────┤       │
 display ─► │  host.rs     hidden window, tray, WM_DISPLAYCHANGE ─────┤       │
            │                                                          ▼       │
            │            twm-core::state::Wm  (pure, no Win32)  dispatch()     │
            │                         │ desired()  = placements + hidden set   │
            │                         ▼                                        │
            │  apply: diff with what is on screen → SetWindowPos (animated),   │
            │         ShowWindowAsync hide/show, border overlays, focus        │
            └──────────────────────────────────────────────────────────────────┘
   twmctl.exe ──named pipe──┘        twm-settings.exe ──named pipe + config.toml
```

## Crates

| Crate | Role | Win32? |
|---|---|---|
| `twm-core` | Everything that can be decided without Windows: geometry, layouts (dwindle, master, monocle), the window-manager state machine, configuration loading and validation, rules and window classification, key chords, actions, animation curves, border ring rasterisation, IPC message types. | No — unit-tested on any OS. |
| `twm` | The daemon: Win32 event sources, applying the desired state to real windows, overlays, tray, crash recovery. | Yes |
| `twmctl` | Command-line client (IPC) plus offline `validate`, `restore`, `reset-config`, `paths`. | Pipe/paths only |
| `twm-settings` | eframe/egui settings window. Edits config.toml with `toml_edit`, talks IPC for live state. | Shell open, mutex |
| `twm-testwin` | Test helper: windows that never take focus and log their moves/keys. | Yes |
| `tools/platform-probe` | Small reversible experiments that produced the facts in RESEARCH.md. | Yes |

## The core idea: state in, desired layout out

`twm_core::state::Wm` holds monitors, workspaces (each with a layout tree),
managed windows and focus. Every input — a window appeared, a key binding, an
IPC request, a monitor change — becomes a call on `Wm` (`add_window`,
`remove_window`, `on_focus`, `on_user_move`, `dispatch(Action)`). `Wm`
returns **effects** for things only the platform can do (start a program,
close a window, reload, quit, open settings) and `desired()` computes the
complete target: one `Placement` (rectangle, border, focus) per visible
window and the set of windows that must be hidden.

Because `Wm` never touches Windows, layouts and policies are tested
directly (80 unit tests in `twm-core`), and the daemon's job is reduced to
*reconciliation*:

1. compute `desired()`;
2. hide windows that should be hidden, show ones that should not;
3. for each placement whose rectangle differs from the last applied one, start
   a tween (or move immediately when animations are off), converting the
   visible-frame rectangle to the window rectangle with the window's
   invisible border offsets (`DWMWA_EXTENDED_FRAME_BOUNDS`);
4. update borders and focus.

Animation frames are driven by a high-resolution waitable timer at the
fastest monitor refresh rate while something moves; when nothing moves the
thread blocks in `MsgWaitForMultipleObjectsEx` and uses no CPU.

## Threads

| Thread | Does | Talks to main via |
|---|---|---|
| Main | Message loop; WinEvent callbacks (out-of-context, so delivered here); all window manipulation; timer; config file watch. | — |
| Keyboard hook | `WH_KEYBOARD_LL` callback must return quickly, so it only matches chords and posts. | `PostThreadMessage(WM_APP_BIND)` |
| IPC | Accepts one pipe client at a time, forwards the request, waits ≤ 5 s for the answer. | `WM_APP+2` + channel |

All `Wm` state lives on the main thread; no locks around it.

## Windows of other programs

- **Classification** (`twm_core::rules::classify`) turns gathered facts
  (styles, owner, process, class, title, cloaked, elevated…) into
  Tile/Float/Ignore with human-readable reasons. Hard limits (elevated,
  invisible, child, cloaked, own process, shell surfaces) cannot be overridden
  by rules; heuristics can.
- **Moving**: `SetWindowPos(… SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE …)` so a hung
  application never blocks the manager. Maximized windows are restored first.
- **Enforcement**: if an application moves its tile, it is put back; after
  6 conflicts within 3 s the manager gives up on that window and logs it.
- **User moves**: a mouse drag of a floating window updates its float
  rectangle; dropping a tiled window onto another swaps them; resizing a tile
  adjusts the split (`Wm::on_user_move`).
- **Focus**: `SetForegroundWindow`; if Windows refuses (foreground lock), one
  retry after injecting an unassigned key (VK 0xE8), which made the retry
  succeed in the interactive tests. `AttachThreadInput` is not used because it
  can block on a hung application.

## Workspaces

A workspace is a layout plus a monitor. Switching computes a new `desired()`;
windows of the old workspace are hidden with `ShowWindowAsync(SW_HIDE)` (or
minimized, by configuration) and slide out/in when animated. Every hidden
window is recorded in `hidden-windows.json` (with its process id, so a reused
handle is never shown by mistake). On start, on a panic and on quit, recorded
windows are shown again; `twmctl restore` does the same offline.

## Borders

`border_style = "overlay"`: four thin layered windows (`WS_EX_LAYERED |
WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`) per bordered window,
one per edge, drawn with `UpdateLayeredWindow` from an anti-aliased
signed-distance rasterisation of the rounded ring (`twm_core::ring`). Four
strips instead of one full-size window keep memory proportional to the
perimeter, not the area. They are kept directly below the window they frame in
z-order and are click-through. Windows' own 1 px border is hidden with
`DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE` while the ring is shown and restored
on quit. `border_style = "dwm"` only recolours the native border.

## Configuration

`twm_core::config::load_str` never fails: TOML syntax errors return the
previous configuration; type errors replace one section with its defaults
(other sections still load); range/choice errors replace one value; unknown
keys produce warnings (via `serde_ignored`). Each diagnostic carries file,
line (found by locating the key in the source), dotted setting path and a
message. The daemon watches the config directory with
`FindFirstChangeNotification` and reloads on change; the settings window
writes with write-to-temp-then-rename so a half-written file is never read.

## IPC

One JSON request line → one JSON response line over
`\\.\pipe\TilingWindowsManager-<session id>[-<instance>]`, created with
`PIPE_REJECT_REMOTE_CLIENTS`. Requests: `ping`, `version`, `dispatch`,
`state`, `windows`, `explain`, `binds`, `diagnostics`, `reload`, `stats`,
`quit` (`twm_core::ipc`). `TWM_INSTANCE`, `TWM_CONFIG_DIR` and `TWM_DATA_DIR`
let the tests run isolated instances beside a real one.

## Files

| Path | Contents |
|---|---|
| `%APPDATA%\TilingWindowsManager\config.toml`, `themes\` | Configuration (portable: `config\` next to the exe) |
| `%APPDATA%\TilingWindowsManager\config.toml.bak` | Settings window's backup before its first save |
| `%LOCALAPPDATA%\TilingWindowsManager\twm.log` | Log, rotated at 2 MiB |
| `%LOCALAPPDATA%\TilingWindowsManager\hidden-windows.json` | Crash-recovery record |
| `HKCU\…\CurrentVersion\Run\TilingWindowsManager` | Only when *Start with Windows* is enabled |

Nothing is written outside the user's profile; no administrator rights are
used or requested.
