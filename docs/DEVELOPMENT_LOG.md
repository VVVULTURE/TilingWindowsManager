# Development log

Chronological record of what was built, what went wrong and how it was
fixed. Test results are in [TEST_PLAN.md](TEST_PLAN.md).

## Where things stand (2026-10-28, evening)

Done and tested: core, daemon, `twmctl`, settings window, packaging
(portable zip + per-user installer), documentation, the interactive suite
(9/9) and a real-desktop run with the default configuration (10/10 on the
build before the last focus changes). See TEST_PLAN.md for exact results.

Still open:

1. Re-run `tests/e2e/real_desktop.py` and `tests/e2e/interactive.py` on
   commit `469f580` or later, once the notification toast that holds the
   foreground on the test PC has been dismissed (see below).
2. Hardware/manual scenarios in TEST_PLAN.md §6 (mixed DPI, hot-plug,
   sleep/resume, elevated-focus key behaviour).
3. Upgrade over a running manager through the installer.

## 2026-10-27 — research, core, daemon

Commits `9f130f3` (core), `0b08cce` (daemon, twmctl, test window, e2e suite),
`290f073` (restore on exit, interactive tests), `868aa1a`.

- Research and platform probe (RESEARCH.md): RegisterHotKey conflicts,
  invisible borders, DWM border colour, cloak and virtual-desktop access
  denied for foreign windows → emulated workspaces with SW_HIDE, LL keyboard hook.
- `twm-core` written test-first: 80 unit tests.
- Daemon: WinEvent hook, keyboard hook thread, IPC, overlays, tray, recovery.
- e2e suite 15/15 with isolated test windows; interactive suite with real apps.

### Problems and fixes

- **Dwindle `remove`** first draft had a confused helper; rewritten around
  "replace the parent split with the sibling".
- **Master resize direction** was inverted for some orientations; replaced by
  an explicit table of which direction grows the master area.
- `Chord` needed `Ord` for use as a map key — derived.
- `deny_unknown_fields` plus retry-on-error could recurse on nested unknown
  keys → removed; unknown keys are reported through `serde_ignored`.
- `twmctl` error 233: the server disconnects right after writing, which
  surfaces as `ERROR_PIPE_NOT_CONNECTED` instead of EOF → read up to the newline.
- After a restart the focused monitor reset to the primary; e2e now refocuses
  the test monitor explicitly.
- Alt+F10 belongs to the NVIDIA overlay on the test machine → test uses Alt+Shift+F8.
  Child-process output decoded as UTF-8, not cp1252.
- "Terminal steals focus back": `twm-testwin` exited on *any* thread
  `WM_TIMER` — the text input framework installs its own timers when a window
  gains focus. It now exits only on its own timer id. An experimental
  `AttachThreadInput` focus path was removed (it can block on hung threads).
- Relaunching Task Manager activated the existing *elevated* instance; UIPI
  then blocked every injected input. The elevated test now runs last and
  reuses an existing instance.
- Launching apps while the user was working stole focus → policy: disruptive
  tests only run when the PC is idle (`GetLastInputInfo` guard).

## 2026-10-28 — settings window, packaging, docs

Commits `14ab9ae` (shared IPC client, settings window floats),
`a5063c8` (settings window), then packaging and docs.

- `twm-settings` (eframe/egui 0.37 + toml_edit 0.25): seven pages, live state
  over IPC, comment-preserving edits, validation with the manager's loader,
  `.bak` before the first save, atomic write, close guard for unsaved changes.
- UI Automation test (`tests/settings`) — no synthetic input needed.
- `twmctl reset-config`, readable `twmctl explain`, first-run default config,
  portable mode (`portable` marker file), embedded icon/version resources,
  `tools/package.ps1`, Inno Setup script, `tools/third_party.py`.
- Release measurements and installer tests (TEST_PLAN.md §2, §5).

### Problems and fixes

- **Every file looked modified on open**: `toml_edit` prints `\n`, but the
  checkout (and Notepad-saved files) use `\r\n`. The editor now records the
  file's line ending and writes it back the same way; covered by a round-trip test.
- **Text fields squeezed to a few characters** inside egui grids: `desired_width`
  is capped by the previous frame's column width. Fixed with `min_size`.
- **Missing glyph** for the status bullet (not in egui's bundled fonts) → painted circle.
- **Controls had no accessible names** (UI Automation listed unnamed sliders).
  Each control is now `labelled_by` its row label — good for screen readers and
  makes the UI testable by name.
- **Adding a key to an inline table** produced `{ class = "B" , title = "T" }`;
  inline tables are re-formatted after edits (TOML forbids comments inside them,
  so nothing is lost).
- UIA test pitfalls: egui's selectable buttons expose Toggle/SelectionItem, not
  Invoke; a control and its label share a name (filter by control type); the
  accessibility tree fills in after the first frames (wait for elements);
  `FindWindow` found a *stale* settings window from an earlier probe — the
  test now refuses to run if one is open.
- Shell pitfalls while scripting edits: backslash escapes in heredocs passed
  to Python were altered by the shell layer; such edits were redone with
  exact-match file edits. PowerShell turns `$null` into `""` for string
  parameters (`FindWindowW`) → `[NullString]::Value`.
- `twmctl explain` reasons said "manage_by_default = false and no rule enables
  it" even when a rule did → reworded to "(a rule can enable it)".
- Documentation claims were re-checked against code and sources before
  publishing (e.g. Hyprland's `force_split = 0` follows the mouse — not
  implemented here and documented as such; `fullscreen` fills the work area,
  not the whole monitor).

## 2026-10-28 (evening) — real-desktop validation

Commits `89f87b0` (focus_on_activate, DPI-scaled drag gaps, interactive
suite hardening) and `469f580` (fixes below), plus
`tests/e2e/real_desktop.py`: the release build with an empty config
directory, the default configuration and the tester's real windows on both
monitors (TEST_PLAN.md §4b).

### Problems and fixes

- **Notepad switched the workspace by itself** (interactive suite): Notepad
  activates its window about 1 s after start; the manager followed the
  activation and switched to the hidden workspace. Added
  `general.focus_on_activate` (default false, as Hyprland's
  `misc:focus_on_activate`); see DECISIONS D13.
- **Edge-drag resize snapped back** (interactive suite): the test grabbed the
  neighbour's invisible resize border, and a 120 px drag pushed the
  neighbour below Windows' minimum window width (122 px). The test now grabs
  1 px outside the visible frame and drags 60 px. While there:
  `on_user_move` did not scale `gaps_in` by the monitor DPI — fixed.
- **Real apps "not at their tile"** (interactive suite): windows left open by
  earlier tests had shrunk the tiles below the apps' minimum sizes. Tests now
  close what they open. The limitation itself is documented (README).
- **New windows opened on the other monitor** (real desktop): hiding the
  foreground window on a workspace switch made Windows activate the next
  window in z-order — Windows Terminal on the second monitor — and the
  manager's focused monitor followed it. Also, the window that should get
  focus was activated before its asynchronous show had happened, so Windows
  refused (`could not activate … (foreground is 0x0)`). Fix: park focus on
  the desktop before hiding the foreground window, activate the target on
  its Show event, ignore interim foreground changes (D13).
- **A notification toast took the foreground** through the same mechanism
  (it was the next window in z-order). After that Windows refused every
  activation — from the manager (even with the injected-input technique)
  and from Explorer's `Shell.Application.Open`. The fix above prevents the
  manager from handing focus to a toast; a toast that already holds the
  foreground still blocks activation until the user clicks (README).
  The keyboard hook is not the cause: it runs on its own thread.
- **`focus_new_windows` only moved internal focus**: a window shown without
  activation was "focused" for key bindings while Windows kept the old
  foreground and border. It now requests activation.
- **Changing the layout in the config did nothing** for existing workspaces
  until restart (`set_config` only updated master options). A changed
  configured layout (`general.layout` or `[[workspace]] layout`) now converts
  existing workspaces; a layout picked at runtime survives unrelated reloads.
- **Theme-file errors pointed at config.toml**. They now name the theme file
  and line and say "(from theme `x`)"; a theme syntax error names its line.
- **Border colour is darkened by the window's own shadow** (pixel sampling:
  ×0.92–0.98). Strips sit below their window by design (D7); moving them above
  needs care with topmost windows and was not attempted. Documented.
- Test-side mistakes found on the way: assuming workspace 2 belonged to the
  primary monitor (it is created for the second one), assuming new windows open
  where they are created (they open on the focused monitor, like Hyprland),
  counting windows by the wrong field, measuring switch latency with an IPC
  call per poll (each `twmctl` start costs ~11 ms), and truncating a message
  before the part being checked. A Windows notification appearing during a
  run is reported as interference, not counted as a pass.
