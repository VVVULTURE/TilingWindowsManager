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
| `twm-core`: geometry & directional focus, dwindle (insert/remove/split rules/ratios/resize/swap/toggle/force_split), master (orientations, mfact, new_status/new_on_top, centre), monocle, gaps, colour & gradient parsing, key chords & variables, actions, configuration (defaults, section salvage, ranges, unknown keys, duplicates, themes, line numbers, theme-file errors), rules & classification (hard limits vs heuristics, full-match regex, case rules, order), Bézier solver, ring rasterisation coverage, IPC wire format, the window-manager state machine (workspaces, monitors, floating, fullscreen, reserved space, adoption, focus fallback, user moves, focus_on_activate, new-window activation, layout change on reload) | 84 | **Pass** 84/84 |
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

Last full run: **9/9 pass**, release build, 2026-10-28 (before commit
`469f580`, which changed focus handling — must be re-run).

| Test | Checks | Last result (2026-10-28, release) |
|---|---|---|
| `t_hook` | Win+Right through the LL hook moves focus; key swallowed; Start menu does not open | Pass; focus changed 32 ms after the key |
| `t_hook_layout` | Layout bind through the hook | Pass (Dwindle → Master) |
| `t_hotkey` | `hotkey` backend works with a free chord and reports Win+1/Win+Left/Win+Right as not registered | Pass |
| `t_mouse` | Drag tiled window onto another → swap; drag a split edge 60 px → ratio changes | Pass (edge 235 → 295 px) |
| `t_notepad` | Real Win32/WinUI app tiles, hides with its workspace, comes back intact | Pass (after the `focus_on_activate` fix, see DEVELOPMENT_LOG) |
| `t_electron` | VS Code (throw-away profile) tiles, hides on workspace switch, comes back | Pass |
| `t_uwp` | Calculator: real process `CalculatorApp.exe` resolved through `ApplicationFrameHost`, floats by rule | Pass |
| `t_restore_on_exit` | Quit puts pre-existing windows back where they were | Pass (556 ms) |
| `t_elevated` | Elevated Task Manager is not managed; reason mentions UIPI | Pass |

## 4b. Real desktop, default configuration — `python tests/e2e/real_desktop.py`

Runs the release build exactly as a first-time user would: empty config
directory (so the default config is written), `manage_by_default = true`,
hook backend, animations on, managing **every window the tester has open**,
on both monitors. Snapshot of all top-level windows, taskbars, work areas
and Explorer's PID before; compared after. Needs 60 s without user input.
The tester's desktop at the time: Windows Terminal and File Explorer on
DISPLAY2 (1920×1080), primary DISPLAY1 2560×1440 @ 180 Hz, both 96 DPI.

**Run A** (2026-10-28, code between `89f87b0` and `469f580`: the
`focus_new_windows`, layout-reload and theme-file fixes in, the desktop
parking and deferred activation not yet): **10/10 pass.**
**Run B** (2026-10-28, `469f580`): **9/11** — the two failures are the
environment: a Windows notification toast (ShellExperienceHost) had been
given the foreground by the *previous* build's behaviour, and Windows then
refused every activation request from both the manager and Explorer
(`could not activate … (foreground is 0x103ce "New notification")`). A
re-run after the toast is dismissed is pending.

| Check | Run A | Run B |
|---|---|---|
| First run writes `config/default.toml` byte-for-byte, 0 diagnostics | Pass, pipe answering 167 ms after launch | Pass, 126 ms |
| 2 of 8 pre-existing windows managed (Terminal, Explorer; taskbars, Progman, overlays ignored); tiles exact; taskbar rects, work areas, Explorer PID unchanged | Pass | Pass |
| Live theme change, pixel-sampled on both border rows: `minimal` e0e0e0 → measured (202,202,202)/(200,200,200), i.e. ×0.92–0.93 (the window's own shadow, README limitations); `nord` 88c0d0→81a1c1 gradient ×0.98; custom `themes/e2e-red.toml` ff2000 ×0.92–0.93; broken theme reported as `…\themes\e2e-red.toml:2: decoration.active_border: invalid hex colour … (from theme \`e2e-red\`)` | Pass | Not verified (toast held the foreground, new window could not be activated) |
| Animations: `WM_WINDOWPOSCHANGED` per window during a layout change — enabled 15, disabled 1 | Pass | Pass (15/12 vs 1) |
| Workspace switch with the real windows away and back (slide animation on): hidden in 22–37 ms, back in place in 30–43 ms (one outlier 197 ms) | Pass | Pass |
| Borderless fullscreen popup covering the primary monitor: ignored ("covers the whole monitor without a title bar"), not moved, no border pixels drawn over it | Pass | Pass |
| Switching to an empty workspace focuses the desktop; the next window opens on that monitor's workspace | (check added later) | Not verified (toast) |
| 40 extra windows (5 workspaces × 8, master layout) | Pass | Pass, but the focus-after-switch assertions were skipped because of the toast |
| Temporary resolution change of DISPLAY2 1920×1080 → 1280×720 → back (`ChangeDisplaySettingsEx` without `CDS_UPDATEREGISTRY`): manager picks up both changes; the 2 real windows re-tiled inside the new work area both times | Pass (0 windows were on it in that run — test bug, fixed) | Pass, 2 windows |
| Quit restores every pre-existing window exactly (placement + frame); no border strips left; taskbar/Explorer untouched | Pass, quit 25 ms | Pass, 28 ms |
| Restart re-manages the same windows; second quit restores them again | Pass | Pass |

Measurements with 40 extra windows (43 managed), release build:

| Metric | Run A | Run B |
|---|---|---|
| Spawn 8 windows → all tiled (includes test-window process start and IPC polling) | 290–349 ms per batch | 275–285 ms |
| Workspace switch 8 ↔ 8 windows, animations off, from launching `twmctl dispatch` until all 8 frames are in place (polled locally every 1 ms) | median 132 ms, max 144 ms | median 110 ms, max 112 ms |
| `twmctl ping` round trip incl. process start (baseline for the line above) | median 11 ms | 11 ms |
| CPU used by the manager while creating 40 windows and switching | 2062 ms | 1562 ms |
| Idle CPU over 10 s with 43 windows | 0.0 ms | 0.0 ms |
| Working set / private bytes | 14.9 / 2.9 MiB | 14.9 / 2.9 MiB |
| Visible border strips with 40 windows / after closing them | 40 / 0 | 40 / 8 (= the 2 real windows) |

Where the time goes (trace log of run A, one switch): the action was logged
35 ms before the manager reached its focus step, i.e. it spent about 35 ms
issuing the asynchronous hide/show/move calls for 16 windows; the `Hidden`
events arrived about 110 ms after the action, because each test process
handles its 8 windows' requests on one thread. Not yet profiled further.

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
| Upgrade over a **running** manager (same-version reinstall, `/VERYSILENT`): installer asks the manager to quit, replaces the files, restarts it | Pass, 2026-10-28 — manager log: "shutting down" / "restored the original placement of 2 window(s)" at 19:32:44.8, files replaced without in-use errors, new instance started 19:32:46.9; then quit, silent uninstall left no install dir, uninstall key or Run value |
| Interactive uninstall prompt "Also delete your configuration?" | Untested (silent mode answers No) |

## 6. Not yet verified (manual or hardware)

| Scenario | How to test |
|---|---|
| Mixed DPI (e.g. 150 % + 100 %) | Change one monitor's scale, check gaps/borders/tiles on both, move a window between them. |
| Monitor hot-plug | Unplug/replug a monitor; windows of the lost monitor must move to a remaining one. (A resolution change is verified in §4b.) |
| A notification toast that already holds the foreground | Observed in §4b run B: Windows refused all activation until it was dismissed. No workaround found without input from the user. |
| Sleep / resume, lock / unlock | Sleep with workspaces in use; nothing hidden may be lost. |
| Key bindings while an elevated window is focused | Focus an admin window, press a bind. Expected: not delivered (UIPI). |
| `hide_method = "minimize"` | Switch workspaces with that setting. |
| Long-running stability (hours, many apps) | Daily use with the log at `info`. |
| Screen readers with the settings window | Narrator over the settings pages. |
