"""Interactive end-to-end tests: keyboard backends and real applications.

Unlike e2e.py these tests activate windows and inject keystrokes, so they
must only run while nobody is using the machine:
  * refuses to start unless the user has been idle for --min-idle seconds
  * before every step, aborts if any input arrived that we did not inject
  * runs an isolated instance managing only test windows and the specific
    applications it launches (rules), confined to a corner of one monitor
  * launched applications use throw-away profiles and are closed afterwards;
    the manager restores pre-existing windows on quit

Usage: python tests/e2e/interactive.py [--bin target/debug] [--min-idle 60]
                                      [--only t_mouse,t_notepad] [--keep]
--keep leaves the temporary folder (config, trace log twm.log) for diagnosis.
"""

import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e  # noqa: E402  (shared helpers)
from e2e import Ctx, frame, visible, is_window, wait_for, placements, settle, rect_tuple, log  # noqa: E402

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32
RESULTS = []


class LASTINPUTINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.UINT), ("dwTime", wt.DWORD)]


def last_input_tick():
    li = LASTINPUTINFO(ctypes.sizeof(LASTINPUTINFO), 0)
    user32.GetLastInputInfo(ctypes.byref(li))
    return li.dwTime


def idle_ms():
    return kernel32.GetTickCount() - last_input_tick()


class UserReturned(Exception):
    pass


GUARD = {"last": 0}


def guard():
    """Raises if input arrived that we did not inject ourselves."""
    t = last_input_tick()
    if t > GUARD["last"] + 50:
        raise UserReturned(f"user input detected ({idle_ms()} ms ago) — stopping interactive tests")


# --- SendInput -----------------------------------------------------------
class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


EXTENDED = {0x25, 0x26, 0x27, 0x28, 0x5B, 0x5C, 0x2D, 0x2E, 0x24, 0x23, 0x21, 0x22}


def send_keys(*events):
    """events: (vk, down) tuples."""
    arr = (INPUT * len(events))()
    for i, (vk, down) in enumerate(events):
        flags = (0 if down else 2) | (1 if vk in EXTENDED else 0)
        arr[i].type = 1
        arr[i].ki = KEYBDINPUT(vk, 0, flags, 0, 0)
    guard()
    user32.SendInput(len(events), arr, ctypes.sizeof(INPUT))
    time.sleep(0.03)
    GUARD["last"] = last_input_tick()


def chord(mods, key):
    send_keys(*[(m, True) for m in mods], (key, True), (key, False), *[(m, False) for m in reversed(mods)])


VK_LWIN, VK_SHIFT, VK_MENU, VK_RIGHT, VK_LEFT, VK_F10, VK_F11 = 0x5B, 0x10, 0x12, 0x27, 0x25, 0x79, 0x7A


def fg():
    return user32.GetForegroundWindow()


def cls(h):
    b = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(wt.HWND(h), b, 256)
    return b.value


def check(name):
    def deco(fn):
        def run(ctx):
            guard()
            t0 = time.time()
            try:
                info = fn(ctx)
                RESULTS.append((name, True, info or f"{(time.time() - t0) * 1000:.0f} ms"))
                log(f"PASS {name}" + (f" — {info}" if info else ""))
            except UserReturned:
                raise
            except Exception as e:  # noqa: BLE001
                RESULTS.append((name, False, str(e)))
                log(f"FAIL {name}: {e}")
        run.__name__ = fn.__name__
        return run
    return deco


CONFIG = """
[general]
manage_by_default = false
focus_new_windows = true
gaps_in = 5
gaps_out = 10

[animations]
enabled = false

[input]
keyboard_backend = "{backend}"

[binds]
"win+right" = "focus right"
"win+left" = "focus left"
"win+alt+f10" = "layout next"
"alt+shift+f8" = "layout next"
"win+1" = "workspace 1"

[[rule]]
match = {{ class = "TwmTestWindow" }}
manage = true

[[rule]]
name = "apps under test"
match = {{ process = "(notepad|code|calculatorapp|taskmgr)\\\\.exe" }}
manage = true
{reserved}
"""


def install_guarded_ctl(ctx):
    """twmctl calls may make the daemon inject its mask key (focus
    activation); accept input that happens during our own calls."""
    raw = ctx.ctl

    def ctl(*a, **kw):
        before = last_input_tick()
        r = raw(*a, **kw)
        after = last_input_tick()
        if after != before and after >= GUARD["last"]:
            GUARD["last"] = after
        return r
    ctx.ctl = ctl


def configure(ctx, backend):
    with open(ctx.config_path, "w", encoding="utf-8") as f:
        f.write(CONFIG.format(backend=backend, reserved=ctx.reserved))
    ctx.ctl("reload")


@check("hook backend: Win+Right moves focus, key is swallowed, Start menu stays closed")
def t_hook(ctx):
    configure(ctx, "hook")
    assert ctx.ctl("binds")["backend"] == "hook"
    logf = os.path.join(ctx.tmp, "keys.log")
    _, ids = ctx.spawn(2, "KEY", extra=("--log", logf))
    ctx.hook_ids = ids
    assert settle(ctx, ids, 3)
    pl = placements(ctx.state())
    left, right = sorted(ids, key=lambda i: pl[i]["rect"]["x"])
    ctx.ctl("dispatch", "focus", "left")
    assert wait_for(lambda: fg() == left, 2), f"manager could not activate the left window (foreground {fg():#x})"
    open(logf, "w").close()
    t0 = time.perf_counter()
    chord([VK_LWIN], VK_RIGHT)
    assert wait_for(lambda: fg() == right, 2, 0.001), f"Win+Right did not focus the right window (fg {fg():#x} {cls(fg())})"
    latency = (time.perf_counter() - t0) * 1000
    time.sleep(0.4)
    assert fg() == right, f"something took focus after Win was released: {cls(fg())} (Start menu?)"
    keys = open(logf).read()
    assert "0x27" not in keys, f"bound key leaked to the application: {keys}"
    # An unbound key passes through to the focused window.
    send_keys((VK_F11, True), (VK_F11, False))
    assert wait_for(lambda: "0x7a" in open(logf).read(), 1), "unbound key was not delivered"
    ctx.hook_ids = ids
    return f"Win+Right -> foreground change in {latency:.1f} ms (injected input to activation)"


@check("hook backend: Win+Alt+F10 cycles the layout")
def t_hook_layout(ctx):
    ws = str(ctx.state()["monitors"][ctx.monitor_index]["active_ws"])
    before = list(ctx.state()["workspaces"][ws]["layout"].keys())[0]
    chord([VK_LWIN, VK_MENU], VK_F10)
    assert wait_for(lambda: list(ctx.state()["workspaces"][ws]["layout"].keys())[0] != before, 1), "layout did not change"
    return f"{before} -> {list(ctx.state()['workspaces'][ws]['layout'].keys())[0]}"


@check("hotkey backend: Alt+Shift+F8 works and shell-owned Win+1 is reported as not registered")
def t_hotkey(ctx):
    configure(ctx, "hotkey")
    b = ctx.ctl("binds")
    assert b["backend"] == "hotkey"
    status = {x["chord"]: x["registered"] for x in b["binds"]}
    assert status["alt+shift+f8"] is True, status
    assert status["win+1"] is False and status["win+right"] is False, f"expected shell-owned chords to fail: {status}"
    ws = str(ctx.state()["monitors"][ctx.monitor_index]["active_ws"])
    before = list(ctx.state()["workspaces"][ws]["layout"].keys())[0]
    chord([VK_MENU, VK_SHIFT], 0x77)  # F8
    assert wait_for(lambda: list(ctx.state()["workspaces"][ws]["layout"].keys())[0] != before, 1), "Alt+Shift+F8 did not change layout"
    unreg = [c for c, ok in status.items() if not ok]
    return f"not registered (owned by Windows): {', '.join(unreg)}"


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


def mouse(flags):
    i = INPUT()
    i.type = 0
    ctypes.memmove(ctypes.byref(i.u), ctypes.byref(MOUSEINPUT(0, 0, 0, flags, 0, 0)), ctypes.sizeof(MOUSEINPUT))
    guard()
    user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))
    GUARD["last"] = last_input_tick()


def drag(x0, y0, x1, y1, steps=25):
    """Left-button drag with real (injected) input, like a user would."""
    user32.SetCursorPos(x0, y0)
    time.sleep(0.05)
    mouse(0x0002)  # LEFTDOWN
    time.sleep(0.1)
    for s in range(1, steps + 1):
        user32.SetCursorPos(int(x0 + (x1 - x0) * s / steps), int(y0 + (y1 - y0) * s / steps))
        mouse(0x0001)  # MOVE (relative 0) so the move is seen as input
        time.sleep(0.015)
    time.sleep(0.1)
    mouse(0x0004)  # LEFTUP
    GUARD["last"] = last_input_tick()
    time.sleep(0.3)


@check("mouse: dropping a tile on another swaps them; dragging an edge resizes the split")
def t_mouse(ctx):
    configure(ctx, "hook")
    old = wt.POINT()
    user32.GetCursorPos(ctypes.byref(old))
    proc, ids = ctx.spawn(2, "DRAG")
    try:
        # Earlier tests cycle the layout; swapping is only visible in dwindle/master.
        ctx.ctl("dispatch", "layout", "dwindle")
        assert settle(ctx, ids, 3)
        pl = placements(ctx.state())
        left, right = sorted(ids, key=lambda i: pl[i]["rect"]["x"])
        lf, rf = frame(left), frame(right)
        # Title bar of the left window -> centre of the right window.
        drag(lf[0] + 60, lf[1] + 12, rf[0] + rf[2] // 2, rf[1] + rf[3] // 2)
        assert settle(ctx, ids, 3), "tiles did not settle after the drop"
        pl = placements(ctx.state())
        assert pl[left]["rect"]["x"] > pl[right]["rect"]["x"], "windows were not swapped"
        # Now `right` is on the left: drag its right edge 60 px right (more could
        # push the neighbour below the system minimum window width). Grab 1 px
        # outside the visible frame: inside its own invisible resize border, before
        # the neighbour's border (which starts gaps - border px further right).
        f = frame(right)
        before_w = f[2]
        drag(f[0] + f[2] + 1, f[1] + f[3] // 2, f[0] + f[2] + 61, f[1] + f[3] // 2)
        assert settle(ctx, ids, 3), "tiles did not settle after the resize"
        after_w = placements(ctx.state())[right]["rect"]["w"]
        assert abs(after_w - (before_w + 60)) <= 8, f"split did not follow the drag: {before_w} -> {after_w}"
        return f"swap ok; edge drag {before_w} -> {after_w} px"
    finally:
        user32.SetCursorPos(old.x, old.y)
        # Free the space: later tests need tiles above apps' minimum sizes.
        proc.kill()
        wait_for(lambda: not any(is_window(i) for i in ids), 3)
        time.sleep(0.3)
        # Closing them makes the manager re-focus another window, which may
        # inject its activation key: that input is ours, not the user's.
        GUARD["last"] = last_input_tick()


def launch(args, **kw):
    return subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, **kw)


def find_new_window(ctx, process, before, timeout=15):
    def look():
        for w in ctx.ctl("windows"):
            if w["process"].lower() == process and w["hwnd"] not in before and w["managed"]:
                return w["hwnd"]
        return None
    end = time.time() + timeout
    while time.time() < end:
        guard()
        h = look()
        if h:
            return h
        time.sleep(0.2)
    return None


def close_app(h):
    """Close a tested app so the next test's tiles stay above apps' minimum
    window sizes (the test area is small). Focus moves as a result, which may
    make the manager inject its activation key: count that as our input."""
    user32.PostMessageW(wt.HWND(h), 0x0010, 0, 0)  # WM_CLOSE
    wait_for(lambda: not is_window(h), 5)
    time.sleep(0.3)
    GUARD["last"] = last_input_tick()


def snapshot_ids(ctx):
    return {w["hwnd"] for w in ctx.ctl("windows")}


@check("Notepad (WinUI) tiles, hides with its workspace and comes back intact")
def t_notepad(ctx):
    configure(ctx, "hook")
    before = snapshot_ids(ctx)
    f = os.path.join(ctx.tmp, "twm-notepad-test.txt")
    open(f, "w").write("TilingWindowsManager test file\n")
    launch(["notepad.exe", f])
    h = find_new_window(ctx, "notepad.exe", before)
    assert h, "Notepad window was not managed"
    ctx.apps.append(h)
    assert settle(ctx, [h] + ctx.hook_ids, 5), f"Notepad not at its tile: {frame(h)}"
    ws = ctx.state()["windows"][str(h)]["ws"]
    ctx.ctl("dispatch", "workspace", "6")
    assert wait_for(lambda: not visible(h)), "Notepad still visible"
    time.sleep(1.0)
    assert not visible(h), "Notepad re-showed itself while hidden"
    ctx.ctl("dispatch", "workspace", str(ws))
    assert wait_for(lambda: visible(h)), "Notepad not shown again"
    assert settle(ctx, [h], 3)
    name = cls(h)
    close_app(h)
    return f"class {name}"


@check("VS Code (Electron) tiles, hides with its workspace and comes back")
def t_electron(ctx):
    code = os.path.expandvars(r"%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe")
    if not os.path.exists(code):
        return "skipped: VS Code not installed"
    prof = os.path.join(ctx.tmp, "vscode-profile")
    before = snapshot_ids(ctx)
    ctx.procs.append(launch([code, "--user-data-dir", prof, "--extensions-dir", os.path.join(prof, "ext"), "--new-window", "--disable-extensions", "--skip-welcome", "--skip-release-notes", "--disable-workspace-trust"]))
    h = find_new_window(ctx, "code.exe", before, 30)
    assert h, "VS Code window was not managed"
    ctx.apps.append(h)
    time.sleep(2)  # let Electron finish its own startup placement
    assert settle(ctx, [h], 5), f"VS Code not at its tile: {frame(h)} vs {placements(ctx.state()).get(h)}"
    ws = ctx.state()["windows"][str(h)]["ws"]
    ctx.ctl("dispatch", "workspace", "6")
    assert wait_for(lambda: not visible(h)), "VS Code still visible"
    time.sleep(1.5)
    reshown = visible(h)
    ctx.ctl("dispatch", "workspace", str(ws))
    assert wait_for(lambda: visible(h)), "VS Code not shown again"
    ok = settle(ctx, [h], 3)
    shot = os.path.join(ctx.tmp, "vscode.png")
    assert not reshown, "Electron re-showed itself while its workspace was hidden"
    assert ok, "VS Code not back at its tile"
    return f"class {cls(h)}; not re-shown while hidden"


@check("Calculator (UWP) is identified by its real process and floats by rule")
def t_uwp(ctx):
    with open(ctx.config_path, "a", encoding="utf-8") as f:
        f.write('\n[[rule]]\nname = "calc floats"\nmatch = { process = "CalculatorApp\\\\.exe" }\nfloat = true\n')
    ctx.ctl("reload")
    before = snapshot_ids(ctx)
    launch(["calc.exe"])
    h = find_new_window(ctx, "calculatorapp.exe", before, 15)
    assert h, "Calculator window not managed (process not resolved through ApplicationFrameHost?)"
    ctx.apps.append(h)
    st = ctx.state()["windows"][str(h)]
    assert st["mode"] == "floating", st["mode"]
    return f"class {st['facts']['class']}, process {st['facts']['process']}"


@check("elevated Task Manager is detected and left alone (UIPI)")
def t_elevated(ctx):
    def find():
        return next((w["hwnd"] for w in ctx.ctl("windows") if w["process"].lower() == "taskmgr.exe"), None)
    h = find()
    if not h:
        # Minimized: an unelevated process cannot close an elevated window
        # afterwards (UIPI), so keep it out of the way. Task Manager is
        # single-instance; starting it again would activate the existing
        # (elevated) window, after which UIPI blocks all injected input —
        # so this test runs last and reuses an open instance.
        subprocess.run(["cmd", "/c", "start", "/min", "", "taskmgr.exe"])
        end = time.time() + 10
        while time.time() < end and not h:
            guard()
            h = find()
            time.sleep(0.2)
    assert h, "Task Manager window not found"
    ctx.apps.append(h)
    ex = ctx.ctl("explain", hex(h))
    assert ex["facts"]["elevated"], f"not detected as elevated: {ex['facts']}"
    assert ex["managed"] is None and ex["classification"]["decision"] == "ignore"
    return "; ".join(ex["classification"]["reasons"])


@check("quit restores pre-existing windows to their original placement")
def t_restore_on_exit(ctx):
    # Restart so the already-open test windows count as pre-existing.
    ids = ctx.hook_ids
    orig = {i: frame(i) for i in ids}
    ctx.stop()
    # Move them away from where the manager will tile them.
    for n, i in enumerate(ids):
        user32.SetWindowPos(wt.HWND(i), None, ctx.area[0] + 40 + n * 30, ctx.area[1] + 40 + n * 30, 300, 200, 0x14)
    time.sleep(0.2)
    orig = {i: frame(i) for i in ids}
    ctx.start()
    ctx.focus_test_monitor()
    assert wait_for(lambda: all(frame(i) != orig[i] for i in ids), 3), "windows were not tiled after restart"
    ctx.stop()
    assert wait_for(lambda: all(frame(i) == orig[i] for i in ids), 3), f"not restored: {[(frame(i), orig[i]) for i in ids]}"
    ctx.start()  # later tests need a running manager
    ctx.focus_test_monitor()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(e2e.ROOT, "target", "debug"))
    ap.add_argument("--min-idle", type=float, default=60)
    ap.add_argument("--monitor", type=int, default=0)
    ap.add_argument("--only", default="", help="comma-separated test function names")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()
    if idle_ms() < args.min_idle * 1000:
        log(f"user active {idle_ms() / 1000:.0f} s ago; interactive tests need {args.min_idle:.0f} s of idle time — not running")
        return 3
    GUARD["last"] = last_input_tick()
    tmp = tempfile.mkdtemp(prefix="twm-int-")
    ctx = Ctx(os.path.abspath(args.bin), tmp)
    ctx.env.pop("TWM_NO_ACTIVATE", None)
    ctx.apps, ctx.procs = [], []
    install_guarded_ctl(ctx)
    aborted = None
    try:
        ctx.reserved = ""
        configure_initial = CONFIG.format(backend="hotkey", reserved="")
        open(ctx.config_path, "w").write(configure_initial)
        ctx.start()
        mons = ctx.state()["monitors"]
        mi = (args.monitor or len(mons)) - 1
        mon = mons[mi]
        ctx.monitor, ctx.monitor_index = mon, mi
        w, h = mon["work"]["w"], mon["work"]["h"]
        aw, ah = 1000, 640
        lines = []
        for i, m in enumerate(mons):
            s = m["dpi"] / 96
            if i == mi:
                lines.append(f"[[monitor]]\nindex = {i + 1}\nreserved = {{ bottom = {int((h - ah) / s)}, left = {int((w - aw) / s)} }}\n")
            else:
                lines.append(f"[[monitor]]\nindex = {i + 1}\nreserved = {{ bottom = {int((m['work']['h'] - 300) / s)}, left = {int((m['work']['w'] - 300) / s)} }}\n")
        ctx.reserved = "\n".join(lines)
        ctx.area = (mon["work"]["x"] + w - aw, mon["work"]["y"], aw, ah)
        configure(ctx, "hotkey")
        ctx.focus_test_monitor()
        tests = [t_hook, t_hook_layout, t_hotkey, t_mouse, t_notepad, t_electron, t_uwp, t_restore_on_exit, t_elevated]
        only = [x for x in args.only.split(",") if x]
        if only:
            # t_hook spawns the windows later tests rely on.
            tests = [t for t in tests if t.__name__ in only or t.__name__ == "t_hook"]
        for t in tests:
            t(ctx)
    except UserReturned as e:
        aborted = str(e)
        log(aborted)
    finally:
        for h in ctx.apps:
            if is_window(h):
                user32.PostMessageW(wt.HWND(h), 0x0010, 0, 0)
        ctx.cleanup()
        for p in ctx.procs:
            try:
                p.wait(5)
            except subprocess.TimeoutExpired:
                p.kill()
        subprocess.run(["taskkill", "/F", "/IM", "CalculatorApp.exe"], capture_output=True)
        time.sleep(1)
        if args.keep:
            log(f"kept {tmp} (trace log: {os.path.join(ctx.data_dir, 'twm.log')})")
        else:
            shutil.rmtree(tmp, ignore_errors=True)
    passed = sum(1 for r in RESULTS if r[1])
    log(f"\n{passed}/{len(RESULTS)} interactive checks passed" + (f" (aborted: {aborted})" if aborted else ""))
    for name, ok, info in RESULTS:
        log(f"  {'PASS' if ok else 'FAIL'}  {name}  ({info})")
    return 0 if passed == len(RESULTS) and not aborted else 1


if __name__ == "__main__":
    sys.exit(main())
