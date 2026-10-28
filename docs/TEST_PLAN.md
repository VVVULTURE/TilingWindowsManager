# Test plan and results

Status legend: **Pass** = run and passed (date, build), **Fail** = run and
failed, **Not run** = exists but has not been executed in its current form,
**Manual** = needs a person / hardware change, **Untested** = no test yet.

Environment for all results: Windows 11 Pro build 26200, unelevated user
(member of Administrators with a filtered token), two monitors 2560×1440 @
180 Hz (primary) and 1920×1080, both at 100 % (96 DPI); Rust 1.99.1 MSVC.

## 1. Unit tests — `cargo test --workspace`

No Windows APIs involved; deterministic.

| Area | Tests | Result (2026-10-28) |
|---|---|---|
| `twm-core`: geometry & directional focus, dwindle (insert/remove/split rules/ratios/resize/swap/toggle/force_split), master (orientations, mfact, new_status/new_on_top, centre), monocle, gaps, colour & gradient parsing, key chords & variables, actions, configuration (defaults, section salvage, ranges, unknown keys, duplicates, themes, line numbers), rules & classification (hard limits vs heuristics, full-match regex, case rules, order), Bézier solver, ring rasterisation coverage, IPC wire format, the window-manager state machine (workspaces, monitors, floating, fullscreen, reserved space, adoption, focus fallback, user moves) | 80 | **Pass** 80/80 |
| `twm-settings`: comment-preserving edits, nested section creation, binds rewrite keeping comments and order, rule edits/reorder/delete, exact round-trip of LF and CRLF files, editing the full default config keeps every comment line and validates cleanly | 6 | **Pass** 6/6 |

## 2. End-to-end, isolated — `python tests/e2e/e2e.py [--bin target/release]`

Runs a separate manager instance (`TWM_INSTANCE`, own config/data folders)
whose configuration manages **only** `twm-testwin` windows inside small
reserved screen corners and uses a single hotkey. Test windows never take
focus. Safe while the PC is in use.

| # | Check | Debug | Release |
|---|---|---|---|
| 1 | New windows tile exactly into the area | Pass | Pass |
| 2 | Layout switching master → monocle → dwindle | Pass | Pass |
| 3 | Workspace switch hides/shows; recovery file written/cleared | Pass | Pass |
| 4 | Focus, move to workspace (silent) and back | Pass | Pass |
| 5 | Toggle floating | Pass | Pass |
| 6 | Rules: float, workspace assignment | Pass | Pass |
| 7 | Close action and app-closed windows retile | Pass | Pass |
| 8 | App minimize/restore removes/re-adds the tile | Pass | Pass |
| 9 | Layout enforced when an app moves its tile | Pass | Pass |
| 10 | Syntax error keeps last good config, reported; fix reloads | Pass | Pass |
| 11 | `twmctl validate` reports file, line, setting | Pass | Pass |
| 12 | Hard kill leaves no hidden window after restart | Pass | Pass |
| 13 | Animated layout change delivers intermediate frames | Pass | Pass |
| 14 | Idle cost over 10 s | Pass | Pass |
| 15 | Quit restores hidden windows and exits | Pass | Pass |

Last run 2026-10-28: debug 15/15, release 15/15.

### Measurements (from tests 13 and 14; `GetProcessMemoryInfo`, `GetProcessTimes`, `WM_WINDOWPOSCHANGED` logs)

| | Debug | Release |
|---|---|---|
| Idle CPU time over 10 s, 2 managed windows | 15.6 ms (0.16 % of one core) | 0.0 ms |
| Working set / private bytes | 15.9 MiB / 2.7 MiB | 13.3 MiB / 2.5 MiB |
| Window moves delivered per window during a 180 ms animated transition | 13–14 | 17 |
| Transition settled (incl. IPC polling) | 221 ms | 219 ms |

Settings window (`twm-settings.exe`, release, default config, open and idle):
99.9 MiB working set, 115.9 MiB private (mostly the OpenGL context); 0.01 s
CPU over 6 idle seconds. It runs only while open.

Interpretation limits: CPU time is sampled with ~15.6 ms resolution
(`GetProcessTimes`), so "0.0 ms" means less than one tick. Frame counts depend
on the monitor refresh rate and on the test app's message loop.

## 3. Settings window — `python tests/settings/test_settings.py [--release]`

Drives the real `twm-settings.exe` through UI Automation (AccessKit):
invoking buttons and setting slider values sends no mouse or keyboard input,
so it runs while the PC is in use. Isolated manager instance as in §2.

| Check | Result |
|---|---|
| Manager starts; settings window opens without taking focus | Pass |
| Manager classifies the settings window via the dedicated heuristic | Pass |
| Controls are exposed with their row labels as accessible names | Pass |
| No write before Save; Save edits in place keeping comments and CRLF | Pass |
| Backup `config.toml.bak` written before the first save | Pass |
| Removing a binding + Save → running manager reloads (1 binding left) | Pass |
| Close with unsaved changes is held; Discard closes without writing | Pass |
| A file with a syntax error is shown, not overwritten | Pass |

2026-10-28: 15/15, three consecutive runs.

## 4. Interactive, real input and real apps — `python tests/e2e/interactive.py`

Uses `SendInput` and launches real applications, so it **takes over the
keyboard and mouse**: run only while nobody is using the PC (each step checks
`GetLastInputInfo` and aborts if the user is active). Elevated windows in the
foreground block injected input (UIPI), so close them first.

| Test | Checks | Last result |
|---|---|---|
| `t_hook` | Win+Right through the LL hook moves focus; key swallowed; Start menu does not open | Pass (33 ms), 2026-10-27 |
| `t_hook_layout` | Layout bind through the hook | Pass, 2026-10-27 |
| `t_hotkey` | `hotkey` backend works with a free chord and reports Win+1/Win+Left as not registered | Pass, 2026-10-27 |
| `t_notepad` | Real Win32 app tiles, closes, retiles | Pass, 2026-10-27 |
| `t_electron` | VS Code (throw-away profile) tiles, hides on workspace switch, comes back | Pass, 2026-10-27 |
| `t_uwp` | Calculator: real process `CalculatorApp.exe` resolved through `ApplicationFrameHost`, floats by rule | Pass, 2026-10-27 |
| `t_restore_on_exit` | Quit puts pre-existing windows back where they were | Pass, 2026-10-27 |
| `t_elevated` | Elevated Task Manager is not managed; reason mentions UIPI | Pass, 2026-10-27 |
| `t_mouse` | Drag tiled window onto another → swap; drag a split edge → ratio changes | **Not run successfully**: the only attempt ran while an elevated window was in the foreground, which blocks injected input. Must be re-run. |

## 5. Packaging

| Check | Result |
|---|---|
| Release build of all binaries, embedded icon and version info | Pass (FileVersion 0.1.0 read back) |
| Portable marker: `twmctl paths` shows `config\`/`data\` next to the exe when `portable` exists | Pass |
| `twmctl reset-config` writes default, keeps timestamped backup | Pass |
| Installer `/VERYSILENT`: runs unelevated (Inno log: "User privileges: None", "Administrative install mode: No"), files in `%LOCALAPPDATA%\Programs\TilingWindowsManager`, two Start menu shortcuts, HKCU uninstall entry, nothing in HKLM, manager not started, no autostart value | Pass, 2026-10-28 |
| Installed `twmctl paths` / `validate` on the shipped example config (0 errors, 52 bindings, 2 rules) | Pass |
| Reinstall over an existing install with `/TASKS=autostart` writes the HKCU Run value | Pass |
| Silent uninstall removes the folder, shortcuts, Run value and uninstall entry; leaves no config behind (none existed) | Pass |
| Portable zip: contains `portable` marker, paths resolve next to the exe, example config validates | Pass |
| Upgrade over a **running** manager (quit → replace → restart) | Untested (would start the manager on the tester's real windows) |
| Interactive uninstall prompt "Also delete your configuration?" | Untested (silent mode answers No) |

## 6. Not yet verified (manual or hardware)

| Scenario | How to test |
|---|---|
| Mixed DPI (e.g. 150 % + 100 %) | Change one monitor's scale, check gaps/borders/tiles on both, move a window between them. |
| Monitor hot-plug / resolution change | Unplug/replug or change resolution; windows of the lost monitor must move to a remaining one. |
| Sleep / resume, lock / unlock | Sleep with workspaces in use; nothing hidden may be lost. |
| Key bindings while an elevated window is focused | Focus an admin window, press a bind. Expected: not delivered (UIPI). |
| First start without a config file creates the default | Start the installed build on a profile without `%APPDATA%\TilingWindowsManager`. (Not run automatically: it would tile the tester's real windows.) |
| `hide_method = "minimize"` | Switch workspaces with that setting. |
| Long-running stability (hours, many apps) | Daily use with the log at `info`. |
| Screen readers with the settings window | Narrator over the settings pages. |
