# Research

What was checked before and while building TilingWindowsManager, with the
source of every fact. "Verified locally" means observed on the development
machine: Windows 11 Pro build 26200, unelevated user session, two monitors
(2560×1440 primary and 1920×1080, both 96 DPI). Sources were read on
2026-10-27/28; where no commit hash was recorded, the date and the
repository's `pushed_at` timestamp at that time are given instead.

## 1. Hyprland

**Source**: <https://github.com/hyprwm/Hyprland>, branch `main` (repository
`pushed_at` 2026-10-28T00:54:25Z). Licence **BSD-3-Clause** (GitHub licence
API and the `LICENSE` file). Documentation: <https://wiki.hypr.land>, source
<https://github.com/hyprwm/hyprland-wiki> (BSD-3-Clause, `pushed_at`
2026-10-27T13:59:19Z).

**What it is.** "An independent, highly customizable, dynamic tiling Wayland
compositor" (repository description). It *is* the display server: it owns the
GPU output, input devices and every window surface. Its `CMakeLists.txt`
(re-checked at commit `7a5e469c780d`, 2026-10-28) requires, via
`pkg_check_modules`, `aquamarine`, `hyprlang`, `hyprcursor`, `hyprutils`,
`hyprgraphics` and a `deps` block including `wayland-server`,
`wayland-protocols`, `libdrm`, `libinput`, `libeis-1.0`, `gbm`, `xkbcommon`,
`pixman-1` and `re2` — the Linux graphics/input stack, none of which exists
on Windows.

**Conclusion: Hyprland cannot run on, or be ported to, Windows.** On Windows
the compositor is DWM, which no third-party program can replace. A Windows
tiling manager is necessarily an ordinary program that moves *other
programs'* windows through Win32. What can be carried over is behaviour:
layouts, configuration concepts, key-driven workflow and look.

**Behaviour studied** (sources downloaded with `gh api` from `main`):

| Topic | Source | Finding | Used here |
|---|---|---|---|
| Dwindle split direction | `src/layout/algorithm/tiled/dwindle/DwindleAlgorithm.cpp` l.146–150 | `SIDEBYSIDE = box.w > box.h * split_width_multiplier` | Same rule |
| Dwindle ratio | same, l.47/52, l.144 | First child gets `size / 2 * splitRatio`; ratio clamped to 0.1–1.9 | Same |
| `force_split` | wiki `configuring/layouts/dwindle-layout.md` | `0` split follows mouse, `1` new window left/top, `2` right/bottom | `1` same; `0` behaves like `2` (no cursor-based placement yet) |
| `preserve_split`, `default_split_ratio` | same wiki page | defaults `false`, `1.0` (range 0.1–1.9) | Same |
| Master layout | `src/layout/algorithm/tiled/master/MasterAlgorithm.cpp`; wiki `master-layout.md` | `mfact`, `new_status` master/slave/inherit, `new_on_top`, `orientation` left/right/top/bottom/center | Same options |
| Layouts available | wiki `config-options.md` | `dwindle`, `master`, `scrolling`, `monocle` | dwindle, master, monocle (no scrolling) |
| Gaps / border | wiki `config-options.md` l.70–74, 90–91 | `border_size` 1 (0–20), `gaps_in` 5, `gaps_out` 20, `active_border` default `0xffffffff` | Same names; defaults gaps_out 12, border 2 |
| Default look | wiki `window-rules.md` l.211 example | `colors = { "rgba(33ccffee)", "rgba(00ff99ee)" }, angle = 45` | Default `hypr` theme uses this gradient |
| Animations | wiki `configuring/core/animations.md` | Animation tree (windows, layers, fade, workspaces…); `speed` in deciseconds; Bézier curves defined by two control points; spring curves | windows / workspaces / border animations; durations in ms; Bézier only |
| Window rules | wiki `configuring/core/rules/window-rules.md` | `match = { class, title, initial_class, … }` with RegEx; effects such as float, workspace, size, center | Same model with Windows-meaningful fields (process, class, title, kind) |
| Config format | wiki (current) | Configuration is now Lua (`hl.animation(...)`, `hl.curve(...)`, `hl.window_rule`) | TOML instead — see DECISIONS.md |

No Hyprland source code was copied; the layout code here was written from
these descriptions and is tested independently (`crates/twm-core/src/layout`).

## 2. Wayland

**Authoritative upstream**: <https://gitlab.freedesktop.org/wayland/wayland>
("Core Wayland window system code and protocol", GitLab API, default branch
`main`, last activity 2026-10-27) and
<https://gitlab.freedesktop.org/wayland/wayland-protocols> ("Additional
standard Wayland protocols"). The GitHub repositories
`gitlab-freedesktop-mirrors/wayland` and `…/wayland-protocols` describe
themselves as "Mirror of https://gitlab.freedesktop.org/…"; there is no
`wayland-project/wayland` repository on GitHub (404). Licence: MIT-style
(`COPYING`). Website: <https://wayland.freedesktop.org>.

**Relevance.** The README describes Wayland as a protocol between a compositor
and its clients plus a library implementation. `meson.build` requires
`epoll(7)` — natively on Linux, through the `epoll-shim` wrapper on FreeBSD/
OpenBSD — and has no Windows support. Windows applications do not speak
Wayland, so even a Wayland compositor for Windows would have no clients.
**Conclusion:** nothing from Wayland is used; this project talks to Win32.

## 3. Existing Windows tiling managers

Checked for techniques and licences (GitHub API, 2026-10-28). **No code from
any of them is included.** GlazeWM (GPL-3.0) and komorebi (source-available
custom licence) could not be reused in an MIT project anyway.

| Project | Licence | Language | Last push | Notes relevant here |
|---|---|---|---|---|
| [komorebi](https://github.com/LGUG2Z/komorebi) | "Komorebi License" 2.0.0 (GitHub: NOASSERTION) | Rust | 2026-10-27 | `window-hiding-behaviour`: `hide` (SW_HIDE) is marked "END OF LIFE … has issues with Electron apps"; `minimize` "has issues with frequent workspace switching"; `cloak` uses "the undocumented SetCloak Win32 function" (`docs/cli/window-hiding-behaviour.md`). Uses `SetWinEventHook`, `DWMWA_BORDER_COLOR` (GitHub code search). |
| [GlazeWM](https://github.com/glzr-io/glazewm) | GPL-3.0 | Rust | 2026-06-20 | `native_window.rs`: `ShowWindowAsync(SW_HIDE)`, `SWP_ASYNCWINDOWPOS`, `WPF_ASYNCWINDOWPLACEMENT`, `DWMWA_EXTENDED_FRAME_BOUNDS`, `DWMWA_BORDER_COLOR`, cloak through an undocumented `IApplicationView` (`view.set_cloak`), `ITaskbarList::AddTab`. Keyboard via `WH_KEYBOARD_LL` (`keyboard_hook.rs`). |
| [Whim](https://github.com/dalyIsaac/Whim) | MIT | C# | 2026-03-06 | — |
| [workspacer](https://github.com/workspacer/workspacer) | MIT | C# | 2025-06-15 | — |
| [FancyWM](https://github.com/FancyWM/fancywm) | NOASSERTION | C# | 2026-09-21 | — |

The komorebi note about Electron and SW_HIDE was tested here: VS Code was
hidden and shown again by workspace switching without problems
(`tests/e2e/interactive.py`, `t_electron`; one hide/show cycle, so this does not
rule out the problem komorebi reports).

## 4. Windows APIs

Microsoft Learn pages were read in full (page `git_commit_id`s are recorded
in the development transcript). Each row says what the documentation states
and what was verified locally with `tools/platform-probe` or the test suites.

| API | Documentation says | Verified locally |
|---|---|---|
| [`SetWinEventHook`](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setwineventhook) | With `WINEVENT_OUTOFCONTEXT` the callback is not mapped into other processes; events are queued, "guaranteed to be in sequential order"; no DLL needed (`hmodWinEventProc` NULL); "The client thread that calls SetWinEventHook must have a message loop"; `WINEVENT_SKIPOWNPROCESS` filters the caller's own events. | Used for create/show/hide/destroy/foreground/move-size/minimize/cloak/name events; all e2e tests depend on it. |
| [`RegisterHotKey`](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey) | `MOD_WIN`: "Keyboard shortcuts that involve the WINDOWS key are reserved for use by the operating system." F12 "is reserved for use by the debugger". | Registration attempts (`platform-probe hotkeys`): **already registered** — Win+1, Win+H, Win+Q, Win+Return, Win+Left, Win+L, Win+Shift+1, Win+Alt+1, Win+Ctrl+1, Alt+Tab, Ctrl+Alt+Del. **Free** — Win+J, Win+Shift+H, Win+Alt+H, Alt+1, Alt+H, Alt+Shift+H, Alt+Return. Alt+F10 belonged to the NVIDIA overlay on this machine. Hence the default `hook` backend. |
| `WH_KEYBOARD_LL` ([LowLevelKeyboardProc](https://learn.microsoft.com/windows/win32/winmsg/lowlevelkeyboardproc)) | (page not re-read for this project) | Win+Right handled in 33 ms, key swallowed, Start menu did not open (`interactive.py t_hook`). Behaviour while an elevated window is focused: **not verified**. |
| [`DWMWINDOWATTRIBUTE`](https://learn.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute) `DWMWA_EXTENDED_FRAME_BOUNDS` | "Retrieves the extended frame bounds rectangle in screen space." | Invisible resize borders measured: Windows Terminal 5/0/5/5 px (left/top/right/bottom), Character Map 7/0/7/7 px at 96 DPI. Tiles are placed by the visible frame. |
| `DWMWA_BORDER_COLOR` | Window border colour; `DWMWA_COLOR_NONE` suppresses it; Windows 11 build 22000+. | `DwmSetWindowAttribute` on another process's window returned S_OK; a screenshot check of the colour was **inconclusive** (the sampled window edge was covered). Used for `border_style = "dwm"` and to hide the native border under the overlay ring. |
| `DWMWA_WINDOW_CORNER_PREFERENCE` | Rounded-corner preference; Windows 11 build 22000+. | Applied with `corner_preference`. |
| `DWMWA_CLOAK` | "Cloaks the window such that it is not visible to the user." | On another process's window: **E_ACCESSDENIED** (0x80070005). Not usable for workspaces. |
| [`IVirtualDesktopManager::MoveWindowToDesktop`](https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ivirtualdesktopmanager-movewindowtodesktop) | "Moves a window to the specified virtual desktop." No documented method switches desktops. | On another process's window (same desktop and a bogus GUID): **E_ACCESSDENIED**. `GetWindowDesktopId` works. Hence emulated workspaces. |
| `ShowWindowAsync(SW_HIDE / SW_SHOWNA)` | — | Hides/shows other processes' windows, including their taskbar buttons; e2e workspace tests; crash recovery restores them (e2e "hard kill" test). |
| `SetWindowPos` with `SWP_ASYNCWINDOWPOS` | — | Moves other processes' windows without blocking on hung apps; 12–14 intermediate positions delivered per window during a 180 ms animation (e2e). |
| `SetForegroundWindow` ([docs](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setforegroundwindow)) | (page not re-read for this project) | Direct calls sometimes fail; one retry after injecting an unassigned key (VK 0xE8) succeeded in the interactive tests. `AttachThreadInput` deliberately not used (it can block on hung threads). |
| UIPI (integrity levels) | — | Elevated windows (Task Manager auto-elevates on this machine) are detected by comparing token integrity and left alone. While an elevated window was in the foreground, `SendInput` from the unelevated test was blocked entirely (mouse test could not run). |
| `IVirtualDesktopManager`, `ITaskbarList`, `IApplicationView` cloaking | Undocumented parts (IApplicationView) change between builds. | Not used. |

## 5. Implementation stack

- **Rust** 1.85+ (developed with 1.99.1, MSVC), [`windows`](https://crates.io/crates/windows) 0.62 bindings.
- **eframe/egui** 0.37 (glow backend, AccessKit) for the settings window; **toml_edit** 0.25 for comment-preserving edits.
- **Inno Setup 6** (per-user, `PrivilegesRequired=lowest`) for the installer.

Licences of all bundled crates: `THIRD_PARTY_LICENSES.md` (generated).
