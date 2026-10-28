# Decisions

Each entry: the decision, the alternatives, and why. Evidence is in
[RESEARCH.md](RESEARCH.md).

## D1. A Windows program, not a Hyprland/Wayland port

Hyprland is a Wayland compositor built on Linux-only libraries (libdrm,
libinput, gbm, wayland-server); Wayland needs epoll and has no Windows
clients; Windows' compositor (DWM) cannot be replaced. **Decision:** implement
Hyprland's *behaviour* (layouts, options, binds, look) as a Win32 program that
arranges other programs' windows.

## D2. Rust

Alternatives: C#/.NET (Whim, workspacer, FancyWM), C++.
**Rust** gives a single small native executable without a runtime install,
direct Win32 access through Microsoft's `windows` crate, and lets the
platform-independent core (`twm-core`) be unit-tested on its own. Idle memory
of the debug build measured ~16 MiB working set (TEST_PLAN.md).

## D3. Emulated workspaces with `SW_HIDE`

| Option | Result |
|---|---|
| Windows virtual desktops (`IVirtualDesktopManager`) | `MoveWindowToDesktop` on another process's window: E_ACCESSDENIED (verified). No documented switching API. Undocumented interfaces change between builds. **Rejected.** |
| `DWMWA_CLOAK` | E_ACCESSDENIED on other processes' windows (verified). **Rejected.** |
| Undocumented `IApplicationView` cloak (GlazeWM, komorebi) | Undocumented, build-dependent. **Rejected.** |
| Move off-screen | Windows stay on the taskbar and in Alt+Tab, can be restored onto invisible coordinates, and break when monitors change. **Rejected.** |
| `SW_MINIMIZE` | Documented; komorebi reports issues with frequent switching; minimize animations. **Offered as `hide_method = "minimize"`.** |
| `ShowWindowAsync(SW_HIDE)` | Documented, works on other processes' windows, removes the taskbar button (matches "the window is on another workspace"). komorebi reports Electron issues; one VS Code hide/show cycle worked here. **Default**, with a crash-recovery record so a crash can never leave windows lost. |

## D4. Keyboard: low-level hook by default, `RegisterHotKey` optional

Hyprland's default binds use the Super (Windows) key. `RegisterHotKey`
refuses chords the shell already owns — verified for Win+1, Win+Left, Win+Q,
Win+Return and others — so a hook is needed to offer Hyprland-like `Win`
bindings. **Decision:** `WH_KEYBOARD_LL` on a dedicated thread (the callback
only matches chords and posts a message, never does work), swallowing only
bound chords and masking the Win/Alt release so the Start menu or menu bar
does not open. `keyboard_backend = "hotkey"` exists for people who do not
want a hook, and reports which chords could not be registered.

## D5. TOML configuration, not Hyprland's format

Hyprland moved from `hyprlang` to Lua. Executing Lua would turn the config
into a program (harder to validate, explain and edit from a UI). **TOML**:
declarative, comments, a mature Rust parser that reports positions, and
`toml_edit` lets the settings window change values *without losing the user's
comments*. Option names follow Hyprland where the concept is the same
(`gaps_in`, `gaps_out`, `border_size`, `mfact`, `split_width_multiplier`,
dispatcher names as action aliases) so Hyprland users recognise them.

## D6. Never fail on configuration

A window manager that refuses to start because of a typo leaves the user with
nothing. **Decision:** loading never fails (see ARCHITECTURE.md); every
problem is reported precisely, and syntax errors keep the last good
configuration.

## D7. Borders as four layered strip windows

Alternatives: `DWMWA_BORDER_COLOR` only (1 px, solid colour, no gradient or
width); one full-size transparent overlay per window (memory proportional to
window area); a single full-screen overlay (must redraw everything on every
move, and covers fullscreen apps). **Decision:** four click-through strips per
window drawn from an anti-aliased signed-distance ring, kept just below the
window in z-order; DWM border recolouring remains available as
`border_style = "dwm"`.

## D8. Asynchronous window moves; give up on fights

`SWP_ASYNCWINDOWPOS` / `ShowWindowAsync` / `WPF_ASYNCWINDOWPLACEMENT` so a
hung application cannot freeze the manager. Applications that keep resizing
themselves are put back up to 6 times in 3 s, then left alone (logged), to
avoid an endless tug-of-war.

## D9. Elevated windows are left alone

An unelevated process cannot move elevated windows (UIPI). The alternative —
running the manager elevated — would need administrator rights and a UAC
prompt on every start, which the project rules exclude. **Decision:** detect
by integrity level, ignore with an explicit reason in `twmctl explain`.

## D10. Opt-in conveniences only; nothing system-wide

Per-user install (`%LOCALAPPDATA%\Programs`, `PrivilegesRequired=lowest`),
autostart through `HKCU\…\Run` only when the user enables it, no services, no
scheduled tasks, no machine-wide settings. Windows' own shortcuts are only
overridden while the manager runs and only for bound chords.

## D11. Settings UI with egui

Alternatives: WinUI 3 (needs the Windows App SDK runtime), WPF/.NET (runtime),
a web UI (browser + server). **eframe/egui**: one native executable, per-monitor
DPI through winit, accessibility through AccessKit (which also makes the UI
testable through UI Automation), and the same language as the rest.

## D12. Restore on exit

Users must be able to stop the manager and get their desktop back.
`restore_on_exit` (default on) records each pre-existing window's
`WINDOWPLACEMENT` when it is first managed and restores it on quit, along
with showing all hidden windows and resetting DWM attributes the manager
changed.

## D13. Keyboard focus follows Windows, with three guards

Windows owns the foreground window; `SetForegroundWindow` only succeeds under
the conditions in its documentation. The manager's notion of focus follows
`EVENT_SYSTEM_FOREGROUND`, with three guards found in real-desktop testing:

1. **A hidden window activating itself does not switch workspace** (Notepad
   does this about 1 s after start). Like Hyprland's `misc:focus_on_activate`
   (default false), `general.focus_on_activate = false` gives focus back to
   the visible workspace. Restoring a window from the taskbar with
   `hide_method = "minimize"` still switches.
2. **Hiding the foreground window parks focus on the desktop first**
   (`GetShellWindow`, like Win+D). Otherwise Windows activates whatever is
   next in z-order — a window on another monitor (the focused monitor would
   follow, and new windows would open there) or a topmost notification toast,
   from which no unprivileged program could take the foreground back in
   testing.
3. **Activation waits for asynchronous shows.** Windows on a newly shown
   workspace appear via `ShowWindowAsync` (D8). Activating one before its own
   thread has shown it fails, so the manager activates it on its
   `EVENT_OBJECT_SHOW` and ignores interim foreground changes (up to 1 s).

`focus_new_windows` asks Windows to activate the new window; before this,
only internal focus moved, and borders and key bindings disagreed about which
window was focused when an app opened a window without activating it.
