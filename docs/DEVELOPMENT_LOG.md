# Development log

## Where things stand (end of session, 2026-10-27)

Commits on `main` (local only, not pushed yet):

- `9f130f3` core: layouts (dwindle, master, monocle), state machine, config, rules, animation curves — 80 unit tests pass.
- `0b08cce` daemon (`twm`), `twmctl`, `twm-testwin`, isolated end-to-end suite (`tests/e2e/e2e.py`, 15/15 passed).
- `290f073` restore original window placement on quit; interactive real-app/keyboard tests.
- `868aa1a` interactive tests: mouse drag test, elevated test runs last; stop tracking `__pycache__`.
- `14ab9ae` pipe client moved to `twm_core::ipc::call` (shared by `twmctl` and the coming settings app); `twm-settings.exe` windows float by default (heuristic, can be overridden by rules).

### Next steps, in order

1. **Settings window** (`crates/twm-settings`, not started): eframe 0.37 with
   `default-features = false, features = ["glow", "default_fonts", "accesskit"]`,
   plus toml_edit 0.25 so edits keep the comments in config.toml. In 0.37 the
   API is `App::ui(&mut self, ui, frame)`, `egui::Panel::left/top(..)`, and
   `CentralPanel::show(ui, ..)`. Plan: read the values the manager actually uses
   from `config::load_str` on the document being edited, and write changes
   into the `DocumentMut`, keeping each value's decor. Pages: Overview, Appearance,
   Layout, Animations, Key bindings, Windows & rules, About. Make a
   `config.toml.bak` before the first save. Also change `daemon.rs` `open_settings`
   so it passes `--config <path>`.
2. Re-run `tests/e2e/interactive.py` while the user is away (mouse drag swap/resize,
   with the elevated test last). **An elevated Task Manager left open in the foreground blocks all
   injected input (UIPI)**, so close it first.
3. Packaging: release build, portable zip, Inno Setup per-user installer
   (`PrivilegesRequired=lowest`), and a test of silent install/uninstall without elevation.
4. Docs: README, RESEARCH, ARCHITECTURE, DECISIONS, TEST_PLAN, CONFIGURATION,
   LICENSE (MIT), third-party licenses.
5. Optional CI. Create the public GitHub repo `VVVULTURE/TilingWindowsManager`
   (name checked free earlier) and push `main`. Measure the release build. Final report.

### Verified so far
- e2e 15/15 (isolated test windows): tiling, layouts, workspaces plus recovery file,
  move to workspace, float, rules, close, minimize, enforcement, config errors and live reload,
  `twmctl validate`, crash recovery, animation frames (12–13 moves per window in a
  180 ms transition), idle CPU 0–15.6 ms over 10 s, debug working set about 15.9 MiB.
- Interactive (earlier full run, 8/8): Win+Right through the hook moved focus in 33 ms with the key
  swallowed; hotkey backend reports failed registrations; Notepad, VS Code (Electron),
  and Calculator (floats) work; elevated Task Manager is ignored with a UIPI reason; restore on exit works.

### Not verified yet
Mixed-DPI monitors (both here are 96 DPI), real display change and sleep/resume, mouse
drag swap/resize, how the LL hook behaves while an elevated window has focus, and the
visual effect of the DWM border colour (the call returned OK; the screenshot was inconclusive).

## Problems hit and fixes
- `twmctl` error 233: the server disconnects after writing, so read up to the newline instead of reading to EOF.
- After a restart the focused monitor reset to primary: e2e now refocuses the test monitor.
- Alt+F10 belongs to the NVIDIA overlay, so the test uses Alt+Shift+F8. Process output is decoded as UTF-8, not cp1252.
- "Terminal steals focus back": the cause was `twm-testwin` exiting on *any* thread
  WM_TIMER (TSF installs its own timers). It now exits only on its own timer id. AttachThreadInput
  was considered and rejected because it can block on hung threads.
- Relaunching Task Manager activated the existing elevated instance. UIPI then blocked
  every injected input, so the elevated test runs last and reuses the existing instance.
- Launching user-visible apps while the user is active stole focus. Policy: disruptive
  tests only when idle (guarded by GetLastInputInfo).
- Recursion risk with `deny_unknown_fields`: removed; unknown keys are reported through `serde_ignored`.
