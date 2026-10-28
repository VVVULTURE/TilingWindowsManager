# Configuration reference

The configuration is one [TOML](https://toml.io) file:

| Install type | Location |
|---|---|
| Installer / normal | `%APPDATA%\TilingWindowsManager\config.toml` |
| Portable (a `portable` file next to `twm.exe`) | `config\config.toml` next to `twm.exe` |
| Explicit | `twm.exe --config <file>` |

`twmctl paths` prints the locations in use. The file is created with every
option commented on first start (the same content as
[`config/default.toml`](../config/default.toml)). Options you leave out take
their default. The settings window (`Win + F1`, or *Settings* in the tray menu)
edits the same file and keeps your comments.

## How changes and mistakes are handled

- Saving the file applies it immediately; no restart is needed.
- A **syntax error** (e.g. a missing quote) keeps the last working
  configuration and reports the line.
- An **invalid value** (out of range, unknown name) falls back to that
  option's default; the rest of the file still applies.
- A section of the wrong type (e.g. `general = 5`) is replaced by its defaults.
- An invalid **rule** or **binding** is skipped; the others still work.
- **Unknown options** are reported as warnings (usually typos) and ignored.

Each problem is reported with file, line (when known), the setting's path
and what was wrong — in a tray notification, the settings window, the log and:

```text
> twmctl validate bad.toml        (real output; path shortened)
error: bad.toml:3: general.layout: `spiral` is not one of dwindle, master, monocle; using `dwindle`
error: bad.toml:2: general.gaps_in: 9999 is out of range 0..=200; using 5
error: bad.toml:6: decoration.active_border: unrecognised colour `blue` (use #RRGGBB, #RRGGBBAA, rgba(RRGGBBAA) or rgb(RRGGBB)); using default
error: bad.toml:9: binds."win+q": `focus`: `sideways` is not a direction (left, right, up, down) — binding skipped
warning: bad.toml: binds: no key bindings are configured; the manager can only be controlled from the tray and twmctl
bad.toml: 4 error(s), 1 warning(s), 0 binding(s), 0 rule(s)
```

`twmctl validate [file]` checks a file without changing anything (exit code 1
when there are errors). `twmctl reset-config` restores the default file and
keeps a backup.

## Sizes and DPI

Pixel values (gaps, border width, rounding, reserved space, rule sizes) are
at 100 % scaling (96 DPI) and are scaled by each monitor's DPI.

## Top level

| Option | Default | |
|---|---|---|
| `theme` | `"hypr"` | Border colour theme: `hypr`, `catppuccin-mocha`, `nord`, `gruvbox`, `rose-pine`, `minimal`, or the name of a file `themes\<name>.toml` in the config folder. Values in `[decoration]` override the theme. |

A theme file contains a `[decoration]` table, e.g.

```toml
[decoration]
active_border = "rgba(cba6f7ff) rgba(89b4faff) 45deg"
inactive_border = "rgba(45475aaa)"
```

## `[variables]`

Free-form `name = "value"` pairs that can be used as `$name` in key chords and
actions. The default defines `mod = "win"` and `terminal = "wt.exe"`, used as
`"$mod+return" = "exec $terminal"`. Changing `mod` to `"alt"` moves every
default binding off the Windows key.

## `[general]`

| Option | Default | Range / values | |
|---|---|---|---|
| `layout` | `"dwindle"` | `dwindle`, `master`, `monocle` | Layout of new workspaces. |
| `gaps_in` | `5` | 0–200 px | Gap around each window; neighbours are `2 × gaps_in` apart. |
| `gaps_out` | `12` | 0–400 px | Gap between windows and the monitor's work area edge. |
| `border_size` | `2` | 0–20 px | Width of the overlay border ring. |
| `manage_by_default` | `true` | | `false` = opt-in mode: only windows a rule sets `manage = true` on are managed. |
| `float_unresizable` | `true` | | Windows without a resize frame float. |
| `float_dialogs` | `true` | | Owned windows (dialogs) float. |
| `hide_method` | `"hide"` | `hide`, `minimize` | How windows on inactive workspaces are hidden. `hide` removes them from the taskbar too; `minimize` keeps taskbar buttons. |
| `focus_new_windows` | `true` | | Give focus to newly managed windows. |
| `mouse_follows_focus` | `false` | | Move the pointer to windows focused with the keyboard. |
| `drag_to_swap` | `true` | | Dropping a dragged tiled window on another one swaps them. |
| `restore_on_exit` | `true` | | On quit, put windows that existed before the manager started back to their original position and state. |

## `[decoration]`

| Option | Default | Values | |
|---|---|---|---|
| `border_style` | `"overlay"` | `overlay`, `dwm`, `none` | `overlay`: a ring drawn around the window (gradients, any width, rounded). `dwm`: recolour Windows 11's own 1 px window border (solid colour only). `none`: no border. |
| `active_border` | from theme | colour or gradient | Border of the focused window. |
| `inactive_border` | from theme | colour or gradient | Border of other managed windows. |
| `inactive_borders` | `true` | | Draw borders on unfocused windows. |
| `rounding` | `8` | 0–50 px | Corner radius of the overlay ring. |
| `corner_preference` | `"default"` | `default`, `round`, `small`, `square` | Asks Windows 11 to round or square the corners of managed windows (`DWMWA_WINDOW_CORNER_PREFERENCE`). |

**Colours** use Hyprland's notation: `rgba(RRGGBBAA)`, `rgb(RRGGBB)`,
`rgba(r, g, b, a)` with `a` 0.0–1.0, `rgb(r, g, b)`, `#RRGGBB`, `#RRGGBBAA`,
or legacy `0xAARRGGBB`. A **gradient** is up to 10 colours separated by spaces,
optionally followed by an angle: `"rgba(33ccffee) rgba(00ff99ee) 45deg"`.

## `[animations]`

| Option | Default | |
|---|---|---|
| `enabled` | `true` | Master switch. `false` moves windows instantly. |

Three animations, each a table with `enabled`, `duration` (milliseconds,
0–2000) and `curve` (a name from `[animations.bezier]`):

| Table | Default | Animates |
|---|---|---|
| `[animations.windows]` | 180 ms, `smooth` | Windows moving/resizing between tiles. |
| `[animations.workspaces]` | 220 ms, `smooth`, `style = "slide"` | Workspace switching. `style`: `slide`, `slidevert`, `none`. |
| `[animations.border]` | 150 ms, `linear` | Border colour fade on focus change. |

`[animations.bezier]` defines curves as `name = [x1, y1, x2, y2]` — the same
control points as CSS `cubic-bezier()` and Hyprland's `bezier`. `x1` and `x2`
must be within 0–1; `y` values may overshoot (e.g. for a bounce). `smooth`
and `linear` always exist. Note: Hyprland's `speed` is in deciseconds; here
`duration` is in milliseconds.

## `[dwindle]`

| Option | Default | Range | |
|---|---|---|---|
| `split_width_multiplier` | `1.0` | 0.1–3.0 | A container is split side by side when `width > height × multiplier`, otherwise top/bottom. |
| `default_split_ratio` | `1.0` | 0.1–1.9 | Size share of a new split; 1.0 = 50/50 (Hyprland semantics: first child gets `ratio / 2`). |
| `preserve_split` | `false` | | Keep a split's direction when its container changes shape. |
| `force_split` | `0` | 0–2 | `1` = new window goes left/above the focused one; `0` and `2` = right/below. |

## `[master]`

| Option | Default | Values | |
|---|---|---|---|
| `mfact` | `0.55` | 0.05–0.95 | Fraction of the screen used by the master area. |
| `orientation` | `"left"` | `left`, `right`, `top`, `bottom`, `center` | Where the master area is. `center` puts the stack on both sides. |
| `new_status` | `"slave"` | `master`, `slave`, `inherit` | Whether new windows join the master area or the stack (`inherit`: same as the focused window). |
| `new_on_top` | `false` | | Insert new windows at the top of their area. |

## `[input]`

| Option | Default | |
|---|---|---|
| `keyboard_backend` | `"hook"` | `hook`: a low-level keyboard hook (`WH_KEYBOARD_LL`). Bindings that use the Windows key work and take priority over Windows' own shortcuts with the same keys; the bound key press is not passed on to the focused app. `hotkey`: `RegisterHotKey`. No hook is installed, but chords Windows or another program already owns cannot be registered (on Windows 11: e.g. `win+1`, `win+left`, `win+q`, `win+return`); failures are listed in the tray, `twmctl binds` and the settings window. |

## `[binds]`

`"chord" = "action"`. A chord is modifiers and one key joined with `+`:

- **Modifiers**: `win` (also `super`, `mod4`, `meta`), `ctrl` (`control`), `alt`, `shift`.
- **Keys**: `a`–`z`, `0`–`9`, `f1`–`f24`, `left` `right` `up` `down`, `return`
  (`enter`), `space`, `tab`, `escape`, `backspace`, `delete`, `insert`, `home`,
  `end`, `pageup`, `pagedown`, `comma`, `period`, `slash`, `backslash`,
  `bracketleft`, `bracketright`, `grave`, `minus`, `equal`, `semicolon`,
  `apostrophe`, `numpad0`–`numpad9`, `multiply`, `add`, `subtract`, `decimal`,
  `divide`, and others listed in `crates/twm-core/src/keys.rs`.
- A chord needs `win`, `ctrl` or `alt`, except function keys. `f12` is
  rejected (reserved by Windows for debuggers). `win+l` cannot be bound (Windows
  handles it before any program sees it).

Two chords that resolve to the same keys are reported as duplicates.

### Actions

Hyprland's dispatcher names are accepted as aliases where they exist.

| Action | Aliases | |
|---|---|---|
| `focus <left\|right\|up\|down>` | `movefocus` | Focus the nearest window in that direction (crosses monitors). |
| `move <dir>` | `movewindow`, `swap`, `swapwindow` | Swap with the neighbour in that direction; at the edge of a monitor, move to the next monitor that way. Floating windows are moved by a step. |
| `resize <dir> [amount]` | `resizeactive` | Grow/shrink by `amount` (fraction of the screen, default 0.05). |
| `cyclenext`, `cycleprev` | | Focus the next/previous window on the workspace. |
| `workspace <n\|next\|prev\|previous>` | | Switch workspace (1–99). `next`/`prev` cycle the existing workspaces on the focused monitor; `previous` goes back to the last one. |
| `movetoworkspace <n\|…>` | | Move the focused window there and follow it. |
| `movetoworkspacesilent <n\|…>` | | Move it without switching. |
| `focusmonitor <n\|next\|prev\|dir>` | | Focus another monitor. |
| `movetomonitor <n\|next\|prev\|dir>` | `movewindowtomonitor` | Move the focused window to another monitor. |
| `layout <dwindle\|master\|monocle\|next>` | `nextlayout`, `cyclelayout` | Change this workspace's layout. |
| `togglefloating` | `togglefloat`, `float` | |
| `fullscreen` | `togglefullscreen`, `maximize` | Fill the monitor's work area (taskbar stays visible) without gaps or border; other tiles stay behind it. |
| `togglesplit` | | Flip the focused split (dwindle). |
| `splitratio <+x\|-x\|exact x>` | | Change the focused split's ratio (dwindle). |
| `swapwithmaster` | | (master) |
| `orientationnext` | | Cycle the master orientation. |
| `close` | `killactive` | Ask the focused window to close (like clicking ×). |
| `exec <command line>` | | Start a program, e.g. `exec "C:\Program Files\App\app.exe" --flag`. |
| `reload` | | Reload the configuration. |
| `togglepause` | `pause`, `toggletiling` | Stop/resume managing windows (all hidden windows are shown while paused). |
| `retile` | | Re-apply the layout. |
| `settings` | `opensettings` | Open the settings window. |
| `openconfig` | `editconfig` | Open config.toml in your editor (Notepad if `.toml` has no association). |
| `quit` | `exit` | Quit, showing all windows and restoring their positions. |

Monitors are numbered with the primary monitor as 1, then left to right,
top to bottom (`twmctl state` lists them).

## `[[rule]]` — per-application rules

```toml
[[rule]]
name = "Spotify on workspace 9"        # shown in `twmctl explain`
match = { process = "spotify\\.exe" }   # all given fields must match
workspace = 9

[[rule]]
name = "Small floating calculator"
match = { class = "ApplicationFrameWindow", title = "Calculator" }
float = true
size = [400, 600]
center = true
```

**Matching** (`match = { ... }`, all given fields must match):

| Field | |
|---|---|
| `process` | Executable file name, e.g. `code\.exe`. Case-insensitive. |
| `class` | Window class name. Case-sensitive. |
| `title` | Window title. Case-sensitive; use `(?i)` for case-insensitive. |
| `kind` | `normal`, `dialog`, `tool`, `popup`. |
| `resizable` | `true` / `false`. |

Patterns are [regular expressions](https://docs.rs/regex/latest/regex/#syntax)
that must match the **whole** value (`note` does not match `notepad.exe`; use
`note.*`). Remember to escape dots and to double backslashes inside TOML
basic strings (`"spotify\\.exe"`) or use literal strings (`'spotify\.exe'`).

**Effects** (all optional):

| Effect | |
|---|---|
| `manage` | `false` = never touch the window. `true` = manage it even if the heuristics would ignore it (e.g. a tool window). |
| `float` | `true` / `false`. |
| `workspace` | 1–99: open on that workspace. |
| `monitor` | 1…: open on that monitor. |
| `border` | `false` hides the border for this window. |
| `size` | `[width, height]` when floating (minimum 50). |
| `center` | Centre when floating. |

Rules apply in file order; when several match, later rules win per effect.
Rules are checked when a window first appears.

**What rules cannot override.** These windows are never managed: windows of
elevated programs (Windows does not let an unelevated program move them),
invisible or cloaked windows, child windows, the manager's own windows,
and shell surfaces (taskbar, desktop, Start, search, notification and input
surfaces). The defaults that rules *can* override: tool windows, windows that
cannot be activated, popups without a title bar, and fullscreen apps are
ignored; dialogs, fixed-size and always-on-top windows float.

To write a rule, look the window up with `twmctl windows` (or *Windows &
rules* in the settings window, which can create the rule for you) and check
the result with `twmctl explain <hwnd>`:

```text
> twmctl explain 0x1c078c        (real output, test window, opt-in configuration)
window 0x1c078c  twm-testwin.exe  class "TwmTestWindow"  title "twm-test"
decision: float (managed: workspace 1, floating)
  - general.manage_by_default = false (a rule can enable it)
  - rule `Floating test window`: manage = true
  - rule `Floating test window`: float = true
matched rules: Floating test window
```

## `[[workspace]]` — workspace rules

```toml
[[workspace]]
id = 1          # 1-99
monitor = 2     # always show workspace 1 on monitor 2
layout = "master"
```

`name` is accepted but not displayed anywhere yet.

## `[[monitor]]` — reserved space

Keep part of a monitor free of tiles, e.g. for a desktop widget or a bar:

```toml
[[monitor]]
index = 1                       # monitor number as in `twmctl state`
reserved = { top = 40, left = 0, right = 0, bottom = 0 }   # px, 0-4000
```
