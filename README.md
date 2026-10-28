# TilingWindowsManager

A keyboard-driven tiling window manager for Windows 11, inspired by
[Hyprland](https://github.com/hyprwm/Hyprland). It arranges your windows into
tiles automatically, gives you numbered workspaces, animated transitions,
gradient borders and gaps, and is controlled from the keyboard. It runs
without administrator rights and leaves Explorer, the taskbar and the Start
menu in place.

> **Status: 0.1.0, early.** The core works and is covered by automated tests
> (see [docs/TEST_PLAN.md](docs/TEST_PLAN.md) for exactly what has and has not
> been verified). Expect rough edges with unusual applications.

Hyprland is a Wayland compositor for Linux and cannot run on Windows (see
[docs/RESEARCH.md](docs/RESEARCH.md)). This project re-implements the parts of
its behaviour that make sense on Windows, using only documented, unprivileged
Win32 APIs. It contains no Hyprland code.

## Features

| | |
|---|---|
| **Automatic tiling** | `dwindle` (binary splits, Hyprland's default), `master` (master area + stack, five orientations) and `monocle`. Per-workspace layout, switchable at runtime. |
| **Workspaces** | Workspaces 1–99, each shown on one monitor at a time and created on demand, emulated by hiding windows (not Windows virtual desktops — see [below](#workspaces-vs-windows-virtual-desktops)). |
| **Keyboard control** | Focus, move, resize, swap, float, fullscreen, workspaces, monitors, layouts, launch programs — all rebindable. `Win` bindings work by default. |
| **Animations** | Windows glide between tiles, workspaces slide, borders fade, with CSS/Hyprland-style Bézier curves. Can be turned off. |
| **Borders and gaps** | Gradient focus ring with rounded corners (or Windows 11's native border), inner/outer gaps, scaled per monitor DPI. Six built-in themes. |
| **Rules** | Per-application rules by process, class and title (regular expressions): float, ignore, workspace, monitor, size, centre, border. `twmctl explain` shows why a window is handled the way it is. |
| **Dialogs and tool windows** | Handled deliberately: dialogs, fixed-size, always-on-top windows float; tool windows, popups, fullscreen games, elevated windows and shell surfaces are left alone. |
| **Safe by default** | Hidden windows are recorded and shown again after a crash; quitting puts windows back where they were; elevated windows are never touched. |
| **Settings window** | Edit everything with a UI that keeps the comments in your config file, validates as you type, and shows live state. |
| **Command line** | `twmctl` — `hyprctl`-style control and inspection. |

## Install

### Installer (recommended)

Download `TilingWindowsManager-<version>-setup.exe` from the
[releases](https://github.com/VVVULTURE/TilingWindowsManager/releases) and run it.
It installs for the current user only, into
`%LOCALAPPDATA%\Programs\TilingWindowsManager` — **no administrator rights**.
It adds Start menu entries and can optionally start the manager when you sign in.

Silent install: `TilingWindowsManager-<version>-setup.exe /VERYSILENT /TASKS=autostart`
(omit `/TASKS=...` to not start with Windows).

### Portable

Unzip `TilingWindowsManager-<version>-portable-x64.zip` anywhere and run
`twm.exe`. Because the zip contains a file named `portable`, configuration
and logs are kept in `config\` and `data\` next to the program. Delete that
file to use the normal per-user locations instead.

### Build from source

Requires Rust 1.85+ with the MSVC toolchain and the Windows SDK.

```powershell
cargo build --release
.\target\release\twm.exe
# Packages (portable zip + installer; installer needs Inno Setup 6):
powershell -ExecutionPolicy Bypass -File tools\package.ps1
```

### Upgrade

Run the newer installer. It asks a running manager to quit (all windows are
shown and put back), replaces the files and starts it again. Your
configuration is not touched. For the portable build, quit the manager
(tray → Quit, or `twmctl quit`) and replace the files.

### Uninstall

*Settings → Apps → Installed apps → TilingWindowsManager → Uninstall*, or the
uninstaller in the install folder. It quits the manager first (restoring all
windows), removes the startup entry, and asks whether to delete your
configuration (default: keep it).

### Restore the default configuration

`twmctl reset-config` writes the default configuration and keeps the old file
as `config.toml.<timestamp>.bak`. Alternatively delete
`%APPDATA%\TilingWindowsManager\config.toml`; a fresh default is created on the
next start.

## Using it

Start `twm.exe`. An icon appears in the notification area (tray). Existing
windows are tiled on their monitors. Default key bindings (`Win` = the Windows
key; every binding can be changed):

| Keys | Action |
|---|---|
| `Win + Return` | Open a terminal (`wt.exe`) |
| `Win + C` | Close window |
| `Win + ←/→/↑/↓` | Move focus |
| `Win + Shift + ←/→/↑/↓` | Move window |
| `Win + Alt + ←/→/↑/↓` | Resize |
| `Win + 1…0` | Switch to workspace 1…10 |
| `Win + Shift + 1…0` | Move window to workspace 1…10 |
| `Win + [` / `Win + ]` | Previous / next workspace |
| `Win + Grave` (the key left of 1) | Back to the last workspace |
| `Win + ,` / `Win + .` | Focus previous / next monitor (`+ Shift` moves the window) |
| `Win + V` | Toggle floating |
| `Win + F` | Fullscreen (fill the workspace) |
| `Win + J` | Toggle split direction (dwindle) |
| `Win + \` | Next layout (`+ Shift`: next master orientation) |
| `Win + Shift + Return` | Swap with master |
| `Win + Shift + T` | Pause / resume tiling |
| `Win + Shift + R` | Reload configuration |
| `Win + F1` | Settings · `Win + Shift + F1` edit the config file |
| `Win + Shift + Escape` | Quit (shows and restores all windows) |

These replace Windows shortcuts that use the same keys (e.g. `Win + ←` snap,
`Win + 1` taskbar app) while the manager runs. `Win + L` (lock) and
`Ctrl + Alt + Del` can never be bound. If you prefer not to override Windows
shortcuts, set `$mod = "alt"` or `keyboard_backend = "hotkey"` — see
[docs/CONFIGURATION.md](docs/CONFIGURATION.md).

Dragging a tiled window with the mouse onto another swaps them; resizing a
tile with the mouse adjusts the split. (Implemented; the automated mouse test
for this has not been run yet — see the test plan.)

The tray menu has pause, reload, edit config, settings, show log,
*Start with Windows* and quit.

### Configuration

`%APPDATA%\TilingWindowsManager\config.toml` (created on first start, fully
commented). Changes apply as soon as you save. Mistakes never stop the
manager: an invalid value falls back to its default and the problem is shown
with file, line and setting in a notification, in the settings window and in
`twmctl validate`. Full reference: [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

### twmctl

```text
twmctl dispatch workspace 3     run any action
twmctl windows                  every window: process, class, title, decision
twmctl explain 0x1a2b3c         why a window is (not) tiled; matching rules
twmctl state | binds | stats    live state, bindings, resource use
twmctl validate [file]          check a config file without running anything
twmctl restore                  show windows a crashed instance left hidden
twmctl reset-config | paths | reload | quit
```

## Workspaces vs. Windows virtual desktops

Workspaces here are **emulated**: windows on inactive workspaces are hidden
(`SW_HIDE`, or minimized if you set `hide_method = "minimize"`), so they also
disappear from the taskbar. They are **not** Windows' Task View virtual
desktops. Windows offers no documented way for one program to move another
program's windows between virtual desktops or to switch desktops
(`IVirtualDesktopManager::MoveWindowToDesktop` returned `E_ACCESSDENIED` for
other processes' windows in our tests), and the undocumented interfaces change
between Windows builds. Task View desktops keep working independently.

If the manager is killed, hidden windows are listed in
`%LOCALAPPDATA%\TilingWindowsManager\hidden-windows.json` and shown again on
the next start (or with `twmctl restore`).

## Known limitations

- Windows of elevated (administrator) programs cannot be moved by an
  unelevated program (UIPI); they are left untiled. While an elevated window
  has focus, key bindings may not work (expected from UIPI, not yet verified).
- Some applications enforce their own size or position; after repeated
  conflicts the manager stops fighting and logs it.
- Mixed-DPI monitor setups, sleep/resume and display hot-plug are implemented
  but have not yet been verified on real hardware (see the test plan).
- No per-window opacity, blur, shadows or window open/close animations
  (not implemented).

## Documentation

- [Configuration reference](docs/CONFIGURATION.md)
- [Research](docs/RESEARCH.md) — Hyprland, Wayland, Windows APIs and existing Windows tiling managers
- [Architecture](docs/ARCHITECTURE.md) · [Decisions](docs/DECISIONS.md)
- [Test plan and results](docs/TEST_PLAN.md) · [Development log](docs/DEVELOPMENT_LOG.md)

## Credits and licence

MIT licence — see [LICENSE](LICENSE). Third-party crate licences:
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) (generated by `tools/third_party.py`;
included in release packages).

Layout behaviour, option names and the default look follow
[Hyprland](https://github.com/hyprwm/Hyprland) (BSD-3-Clause) and its
[wiki](https://wiki.hypr.land); no code was copied. Theme palettes:
[Catppuccin](https://catppuccin.com) (MIT), [Nord](https://www.nordtheme.com) (MIT),
[Gruvbox](https://github.com/morhetz/gruvbox) (MIT/X11), [Rosé Pine](https://github.com/rose-pine/rose-pine-theme) (MIT).
Not affiliated with any of these projects.
