# Development log

Chronological record of what was built, what went wrong and how it was
fixed. Test results are in [TEST_PLAN.md](TEST_PLAN.md).

## Where things stand (2026-10-28)

Done and tested: core, daemon, `twmctl`, settings window, packaging
(portable zip + per-user installer), documentation. See TEST_PLAN.md for
exact results.

Still open:

1. **Re-run `tests/e2e/interactive.py` while nobody is using the PC** — the
   mouse drag swap/resize test (`t_mouse`) has never completed (its only run
   was blocked by an elevated window in the foreground). Close elevated windows first.
2. First start without a config (creates the default) — implemented, not run
   (it would tile the tester's windows).
3. Hardware/manual scenarios in TEST_PLAN.md §6 (mixed DPI, hot-plug,
   sleep/resume, elevated-focus key behaviour).
4. Upgrade over a running manager through the installer.

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
