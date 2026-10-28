"""Phase-6 end-to-end run against the REAL desktop with the DEFAULT config.

Unlike e2e.py (isolated, test windows only) this manages every window the
user has open, exactly as a first-time user would experience it. Run it only
while nobody is using the machine: it hides and moves real windows, changes
a monitor's resolution for a few seconds, and briefly covers the screen.

What it does (all reverted in `finally`):
  * snapshots every visible top-level window, the taskbars and work areas
  * starts twm with an EMPTY config dir and checks the default config is
    written (first run), then lets it manage the real windows
  * checks tiling, the taskbar/Explorer, live theme changes (by sampling
    screen pixels of the border), custom theme files, animation on/off,
    workspace switching, a borderless fullscreen window, 40 extra windows
    (latency, CPU, memory), stale border overlays, a temporary resolution
    change of a secondary monitor (not saved to the registry), quit,
    restart and quit again
  * compares the final position of every pre-existing window with the
    snapshot

Usage:  python tests/e2e/real_desktop.py [--bin target/release] [--no-display-change]
"""

import argparse
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e  # noqa: E402  (sets per-monitor DPI awareness)
from e2e import Ctx, frame, is_window, log, rect_tuple, visible, wait_for  # noqa: E402

user32, gdi32 = ctypes.windll.user32, ctypes.windll.gdi32
RESULTS = []
REPORT = []


def idle_ms():
    class LII(ctypes.Structure):
        _fields_ = [("cbSize", wt.UINT), ("dwTime", wt.DWORD)]
    li = LII(ctypes.sizeof(LII), 0)
    user32.GetLastInputInfo(ctypes.byref(li))
    return ctypes.windll.kernel32.GetTickCount() - li.dwTime


def check(name):
    def deco(fn):
        def run(ctx):
            t0 = time.time()
            try:
                info = fn(ctx)
                RESULTS.append((name, True, info or f"{(time.time() - t0) * 1000:.0f} ms"))
                log(f"PASS {name}" + (f" — {info}" if info else ""))
            except Exception as e:  # noqa: BLE001
                RESULTS.append((name, False, str(e)))
                log(f"FAIL {name}: {e}")
                traceback.print_exc()
        run.__name__ = fn.__name__
        return run
    return deco


# ------------------------------------------------------------ desktop probes

EnumProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def top_windows():
    out = []
    user32.EnumWindows(EnumProc(lambda h, _: out.append(h) or True), 0)
    return out


def cls(h):
    b = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(h, b, 256)
    return b.value


def title(h):
    b = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(h, b, 512)
    return b.value


def cloaked(h):
    v = wt.DWORD()
    ctypes.windll.dwmapi.DwmGetWindowAttribute(h, 14, ctypes.byref(v), 4)  # DWMWA_CLOAKED
    return v.value != 0


class WINDOWPLACEMENT(ctypes.Structure):
    _fields_ = [("length", wt.UINT), ("flags", wt.UINT), ("showCmd", wt.UINT), ("min", wt.POINT), ("max", wt.POINT), ("normal", wt.RECT)]


def placement(h):
    wp = WINDOWPLACEMENT()
    wp.length = ctypes.sizeof(wp)
    user32.GetWindowPlacement(h, ctypes.byref(wp))
    n = wp.normal
    return (wp.showCmd, (n.left, n.top, n.right, n.bottom))


def snapshot():
    snap = {}
    for h in top_windows():
        if not user32.IsWindowVisible(h) or cloaked(h) or cls(h) in ("TwmBorderStrip", "TwmTestWindow"):
            continue
        snap[h] = {"class": cls(h), "title": title(h), "frame": frame(h), "placement": placement(h)}
    return snap


def strips_visible():
    return [h for h in top_windows() if cls(h) == "TwmBorderStrip" and user32.IsWindowVisible(h)]


class MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("rcMonitor", wt.RECT), ("rcWork", wt.RECT), ("dwFlags", wt.DWORD)]


def work_areas():
    out = []
    MonProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HMONITOR, wt.HDC, ctypes.POINTER(wt.RECT), wt.LPARAM)

    def cb(hm, _dc, _r, _l):
        mi = MONITORINFO(ctypes.sizeof(MONITORINFO))
        user32.GetMonitorInfoW(hm, ctypes.byref(mi))
        w = mi.rcWork
        out.append((w.left, w.top, w.right, w.bottom))
        return True
    user32.EnumDisplayMonitors(None, None, MonProc(cb), 0)
    return sorted(out)


def taskbars():
    out = {}
    for c in ("Shell_TrayWnd", "Shell_SecondaryTrayWnd"):
        h = user32.FindWindowW(c, None)
        if h:
            r = wt.RECT()
            user32.GetWindowRect(h, ctypes.byref(r))
            out[c] = (r.left, r.top, r.right, r.bottom, bool(user32.IsWindowVisible(h)))
    return out


def explorer_pids():
    p = subprocess.run(["tasklist", "/FI", "IMAGENAME eq explorer.exe", "/FO", "CSV", "/NH"], capture_output=True, text=True)
    return sorted(int(l.split('","')[1]) for l in p.stdout.splitlines() if l.startswith('"explorer'))


def pixel(x, y):
    """Reads one composed screen pixel (CAPTUREBLT includes layered windows)."""
    sdc = user32.GetDC(None)
    mdc = gdi32.CreateCompatibleDC(sdc)
    bmp = gdi32.CreateCompatibleBitmap(sdc, 1, 1)
    old = gdi32.SelectObject(mdc, bmp)
    gdi32.BitBlt(mdc, 0, 0, 1, 1, sdc, x, y, 0x00CC0020 | 0x40000000)
    c = gdi32.GetPixel(mdc, 0, 0)
    gdi32.SelectObject(mdc, old)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mdc)
    user32.ReleaseDC(None, sdc)
    return (c & 0xff, (c >> 8) & 0xff, (c >> 16) & 0xff)


def near(c, ref, tol=12):
    return all(abs(a - b) <= tol for a, b in zip(c, ref))


def between(c, a, b, tol=8):
    return all(min(x, y) - tol <= v <= max(x, y) + tol for v, x, y in zip(c, a, b))


def shaded(c, a, b=None, tol=8):
    """Darkening factor k (0.75-1.0) for which c matches colour a (or a
    colour between a and b) times k, else None. The strips sit below their
    window in z-order, so the window's own DWM shadow darkens them a bit."""
    b = b or a
    for k100 in range(100, 74, -1):
        k = k100 / 100
        if between(c, tuple(x * k for x in a), tuple(x * k for x in b), tol):
            return k
    return None


# ------------------------------------------------------------ display mode

class DEVMODEW(ctypes.Structure):
    _fields_ = [("dmDeviceName", wt.WCHAR * 32), ("dmSpecVersion", wt.WORD), ("dmDriverVersion", wt.WORD), ("dmSize", wt.WORD),
                ("dmDriverExtra", wt.WORD), ("dmFields", wt.DWORD), ("dmPositionX", wt.LONG), ("dmPositionY", wt.LONG),
                ("dmDisplayOrientation", wt.DWORD), ("dmDisplayFixedOutput", wt.DWORD), ("dmColor", ctypes.c_short),
                ("dmDuplex", ctypes.c_short), ("dmYResolution", ctypes.c_short), ("dmTTOption", ctypes.c_short),
                ("dmCollate", ctypes.c_short), ("dmFormName", wt.WCHAR * 32), ("dmLogPixels", wt.WORD), ("dmBitsPerPel", wt.DWORD),
                ("dmPelsWidth", wt.DWORD), ("dmPelsHeight", wt.DWORD), ("dmDisplayFlags", wt.DWORD), ("dmDisplayFrequency", wt.DWORD),
                ("dmICMMethod", wt.DWORD), ("dmICMIntent", wt.DWORD), ("dmMediaType", wt.DWORD), ("dmDitherType", wt.DWORD),
                ("dmReserved1", wt.DWORD), ("dmReserved2", wt.DWORD), ("dmPanningWidth", wt.DWORD), ("dmPanningHeight", wt.DWORD)]


def current_mode(dev):
    dm = DEVMODEW()
    dm.dmSize = ctypes.sizeof(dm)
    user32.EnumDisplaySettingsW(dev, -1, ctypes.byref(dm))  # ENUM_CURRENT_SETTINGS
    return dm


def has_mode(dev, w, h):
    i = 0
    dm = DEVMODEW()
    dm.dmSize = ctypes.sizeof(dm)
    while user32.EnumDisplaySettingsW(dev, i, ctypes.byref(dm)):
        if dm.dmPelsWidth == w and dm.dmPelsHeight == h:
            return True
        i += 1
    return False


def set_mode(dev, w, h):
    dm = current_mode(dev)
    dm.dmPelsWidth, dm.dmPelsHeight = w, h
    dm.dmFields = 0x80000 | 0x100000  # DM_PELSWIDTH | DM_PELSHEIGHT
    return user32.ChangeDisplaySettingsExW(dev, ctypes.byref(dm), None, 0, None)  # dynamic only, no CDS_UPDATEREGISTRY


def restore_modes():
    # NULL devmode = go back to the mode stored in the registry.
    return user32.ChangeDisplaySettingsExW(None, None, None, 0, None)


# ------------------------------------------------------------ helpers

def managed(ctx):
    return {int(k): v for k, v in ctx.state()["windows"].items()}


def tiled_mismatches(ctx):
    """(id, actual, desired) for visible tiled windows not at their tile."""
    st = ctx.state()
    out = []
    for p in st["desired"]["placements"]:
        i = p["id"]
        if p.get("floating") or not visible(i):
            continue
        if frame(i) != rect_tuple(p["rect"]):
            out.append((i, frame(i), rect_tuple(p["rect"])))
    return out


def edit_config(ctx, old, new):
    text = open(ctx.config_path, encoding="utf-8").read()
    assert old in text, f"{old!r} not in config"
    open(ctx.config_path, "w", encoding="utf-8").write(text.replace(old, new, 1))


def border_probe(ctx, hwnd):
    """Middle of the top border edge of `hwnd`: (outer row, inner row)."""
    x, y, w, _ = frame(hwnd)
    return (x + w // 2, y - 2), (x + w // 2, y - 1)


def border_colour(ctx, hwnd):
    o, i = border_probe(ctx, hwnd)
    return pixel(*o), pixel(*i)


def spawn_at(ctx, count, ttl, x, y, w=400, h=300, style="normal", extra=()):
    p = subprocess.Popen([os.path.join(ctx.bin, "twm-testwin.exe"), "--count", str(count), "--title", ttl, "--style", style,
                          "--x", str(x), "--y", str(y), "--w", str(w), "--h", str(h), *extra], stdout=subprocess.PIPE, text=True)
    ids = [int(p.stdout.readline().strip().split("=")[1], 16) for _ in range(count)]
    ctx.testwins.append(p)
    return p, ids


# ------------------------------------------------------------ checks

@check("first run writes the default config and starts")
def t_first_run(ctx):
    assert not os.path.exists(ctx.config_path)
    t0 = time.time()
    ctx.start()
    ctx.start_ms = (time.time() - t0) * 1000
    assert os.path.exists(ctx.config_path), "no config written"
    want = open(os.path.join(e2e.ROOT, "config", "default.toml"), encoding="utf-8").read()
    got = open(ctx.config_path, encoding="utf-8").read()
    assert got.replace("\r\n", "\n") == want.replace("\r\n", "\n"), "written config differs from config/default.toml"
    d = ctx.ctl("diagnostics")["diagnostics"]
    assert not d, f"default config has diagnostics: {d}"
    return f"ready (pipe answering) {ctx.start_ms:.0f} ms after launch"


@check("real windows are managed and tiled; taskbar, work areas and Explorer untouched")
def t_tiles_real(ctx):
    time.sleep(1.5)
    m = managed(ctx)
    wins = ctx.ctl("windows")
    pre = [w for w in wins if w["hwnd"] in ctx.snap]
    ctx.real_managed = [w["hwnd"] for w in pre if w["managed"]]
    lines = [f"{w['process']}:{w['class']}={w['decision']}" for w in pre]
    log("  pre-existing windows: " + "; ".join(lines))
    assert ctx.real_managed, "no pre-existing window was managed"
    bad = tiled_mismatches(ctx)
    ctx.overflow = bad
    assert taskbars() == ctx.taskbars, f"taskbar changed: {taskbars()} vs {ctx.taskbars}"
    assert work_areas() == ctx.work, "monitor work areas changed"
    assert explorer_pids() == ctx.explorer, "explorer.exe restarted"
    assert not bad, f"windows not at their tile: {[(hex(i), a, d) for i, a, d in bad]}"
    return f"{len(ctx.real_managed)} of {len(pre)} pre-existing windows managed ({len(m)} total)"


@check("theme changes recolour the active border live (pixel-sampled)")
def t_theme(ctx):
    edit_config(ctx, 'theme = "hypr"', 'theme = "minimal"')
    ctx.ctl("dispatch", "focusmonitor", str(ctx.primary_index + 1))
    p, ids = spawn_at(ctx, 1, "THEME", ctx.primary["work"]["x"] + 200, ctx.primary["work"]["y"] + 200, extra=("--color", "202020"))
    h = ids[0]
    ctx.ctl("reload")
    assert wait_for(lambda: ctx.state()["foreground"] == h, 3), f"new window not focused (fg {ctx.state()['foreground']})"
    assert wait_for(lambda: not tiled_mismatches(ctx), 3)
    time.sleep(0.4)  # border fade
    c1 = border_colour(ctx, h)
    k1 = [shaded(c, (0xe0, 0xe0, 0xe0)) for c in c1]
    assert all(k1), f"minimal active border expected e0e0e0 (outer, inner rows), got {c1}"
    edit_config(ctx, 'theme = "minimal"', 'theme = "nord"')
    time.sleep(0.8)  # file watcher + redraw
    c2 = border_colour(ctx, h)
    k2 = [shaded(c, (0x88, 0xc0, 0xd0), (0x81, 0xa1, 0xc1)) for c in c2]
    assert all(k2), f"nord gradient expected between 88c0d0 and 81a1c1, got {c2}"
    # user theme file
    tdir = os.path.join(ctx.cfg_dir, "themes")
    os.makedirs(tdir, exist_ok=True)
    open(os.path.join(tdir, "e2e-red.toml"), "w").write('[decoration]\nactive_border = "rgba(ff2000ff)"\ninactive_border = "rgba(00000000)"\n')
    edit_config(ctx, 'theme = "nord"', 'theme = "e2e-red"')
    time.sleep(0.8)
    c3 = border_colour(ctx, h)
    k3 = [shaded(c, (0xff, 0x20, 0x00)) for c in c3]
    assert all(k3), f"custom theme expected ff2000, got {c3}"
    # broken user theme: named in the diagnostics, last good colours kept
    open(os.path.join(tdir, "e2e-red.toml"), "w").write('[decoration]\nactive_border = "rgba(nothex)"\n')
    ctx.ctl("reload")
    d = ctx.ctl("diagnostics")["diagnostics"]
    hit = [x for x in d if x["file"].endswith("e2e-red.toml") and x["setting"] == "decoration.active_border" and x["line"] == 2]
    msg = f"{hit[0]['file']}:{hit[0]['line']}: {hit[0]['setting']}: {hit[0]['message']}" if hit else str(d)
    assert hit, f"theme error not reported against the theme file, line 2: {d}"
    edit_config(ctx, 'theme = "e2e-red"', 'theme = "hypr"')
    time.sleep(0.8)
    c4 = border_colour(ctx, h)
    p.kill()
    return (f"(outer, inner) border rows: minimal {c1} k={k1}, nord {c2} k={k2}, custom {c3} k={k3}, back to hypr {c4}; "
            f"broken theme reported as: {msg}")


def frames_for_layout_change(ctx, logf, ids):
    open(logf, "w").close()
    ctx.ctl("dispatch", "layout", "next")
    time.sleep(0.8)
    per = {}
    for l in open(logf).read().splitlines():
        if l.strip():
            per.setdefault(l.split()[1], 0)
            per[l.split()[1]] += 1
    return per


@check("animations: several frames when enabled, one jump when disabled")
def t_animations(ctx):
    logf = os.path.join(ctx.tmp, "frames.log")
    w = ctx.primary["work"]
    ctx.ctl("dispatch", "focusmonitor", str(ctx.primary_index + 1))
    p, ids = spawn_at(ctx, 2, "ANIM", w["x"] + 300, w["y"] + 300, extra=("--log", logf))
    assert wait_for(lambda: not tiled_mismatches(ctx), 3)
    on = frames_for_layout_change(ctx, logf, ids)
    edit_config(ctx, "[animations]\nenabled = true", "[animations]\nenabled = false")
    time.sleep(0.8)
    off = frames_for_layout_change(ctx, logf, ids)
    ctx.ctl("dispatch", "layout", "dwindle")
    p.kill()
    assert max(on.values(), default=0) >= 5, f"enabled: expected several frames, got {on}"
    assert max(off.values(), default=0) <= 3, f"disabled: expected 1-2 moves, got {off}"
    edit_config(ctx, "[animations]\nenabled = false", "[animations]\nenabled = true")
    return f"WM_WINDOWPOSCHANGED per window: enabled {on}, disabled {off}"


@check("workspace switch hides real windows and brings them back in place")
def t_workspace(ctx):
    st = ctx.state()
    first = st["windows"][str(ctx.real_managed[0])]["ws"]
    mi = st["workspaces"][str(first)]["monitor"]
    ctx.ctl("dispatch", "focusmonitor", str(mi + 1))
    act = st["monitors"][mi]["active_ws"]
    mine = [i for i in ctx.real_managed if visible(i) and st["windows"][str(i)]["ws"] == act]
    assert mine, "no real window on that monitor's active workspace"
    before = {i: frame(i) for i in ctx.real_managed}
    times = []
    for _ in range(3):
        t0 = time.perf_counter()
        ctx.ctl("dispatch", "workspace", "20")
        assert wait_for(lambda: not any(visible(i) for i in mine), 3, 0.005), "windows still visible on workspace 20"
        t1 = time.perf_counter()
        ctx.ctl("dispatch", "workspace", str(act))
        assert wait_for(lambda: all(visible(i) and frame(i) == before[i] for i in mine), 3, 0.005), \
            f"not restored: {[(hex(i), frame(i), before[i]) for i in mine if frame(i) != before[i]]}"
        t2 = time.perf_counter()
        times.append(((t1 - t0) * 1000, (t2 - t1) * 1000))
    return f"{len(mine)} real windows on {st['monitors'][mi]['id']} workspace {act}; away/back with default slide animation: " + ", ".join(f"{a:.0f}/{b:.0f} ms" for a, b in times)


@check("a borderless fullscreen window is ignored and no border is drawn over it")
def t_fullscreen(ctx):
    m = ctx.primary["rect"]
    p, ids = spawn_at(ctx, 1, "FULLSCREEN", m["x"], m["y"], m["w"], m["h"], style="popup", extra=("--color", "ff00ff"))
    h = ids[0]
    time.sleep(0.8)
    ex = ctx.ctl("explain", f"{h:#x}")
    assert ex["managed"] is None and ex["classification"]["decision"] == "ignore", ex["classification"]
    assert frame(h) == (m["x"], m["y"], m["w"], m["h"]), f"fullscreen window moved: {frame(h)}"
    # Probe border positions of real windows: the fullscreen window must cover them.
    leaks = []
    for i in ctx.real_managed:
        if not visible(i):
            continue
        x, y, w, hh = frame(i)
        if m["x"] <= x < m["x"] + m["w"] and m["y"] <= y - 1 < m["y"] + m["h"]:
            c = pixel(x + w // 2, y - 1)
            if not near(c, (0xff, 0x00, 0xff), 4):
                leaks.append((hex(i), c))
    p.kill()
    assert wait_for(lambda: not is_window(h), 3)
    assert not leaks, f"something drawn over the fullscreen window: {leaks}"
    return "; ".join(ex["classification"]["reasons"])


@check("switching to an empty workspace focuses the desktop; the next window opens on that monitor")
def t_empty_ws(ctx):
    w = ctx.primary["work"]
    ctx.ctl("dispatch", "focusmonitor", str(ctx.primary_index + 1))
    home = ctx.state()["monitors"][ctx.primary_index]["active_ws"]
    p, ids = spawn_at(ctx, 1, "EMPTYWS", w["x"] + 100, w["y"] + 100)
    try:
        assert wait_for(lambda: user32.GetForegroundWindow() == ids[0], 2), "new window not activated"
        ctx.ctl("dispatch", "workspace", "30")
        desk = user32.GetShellWindow()
        assert wait_for(lambda: user32.GetForegroundWindow() == desk, 2), \
            f"foreground is {user32.GetForegroundWindow():#x} {title(user32.GetForegroundWindow())!r}, not the desktop"
        time.sleep(0.3)
        st = ctx.state()
        assert st["focused_monitor"] == ctx.primary_index, f"focused monitor moved to {st['focused_monitor']}"
        p2, ids2 = spawn_at(ctx, 1, "ONEMPTY", w["x"] + 100, w["y"] + 100)
        assert wait_for(lambda: str(ids2[0]) in ctx.state()["windows"], 2)
        st = ctx.state()
        assert st["windows"][str(ids2[0])]["ws"] == 30, f"new window went to workspace {st['windows'][str(ids2[0])]['ws']}"
        p2.kill()
    finally:
        p.kill()
        # Closing our last window lets Windows activate one on another
        # monitor, and the manager follows the keyboard focus there.
        ctx.ctl("dispatch", "focusmonitor", str(ctx.primary_index + 1))
        ctx.ctl("dispatch", "workspace", str(home))
    return f"desktop {desk:#x} focused on empty workspace 30; new window opened there"


@check("40 extra windows: tiling latency, CPU and memory, no stale overlays")
def t_many(ctx):
    w = ctx.primary["work"]
    s0 = ctx.ctl("stats")
    edit_config(ctx, 'layout = "dwindle"', 'layout = "master"')
    time.sleep(0.8)
    home = ctx.state()["monitors"][ctx.primary_index]["active_ws"]
    try:
        return many_body(ctx, w, s0, home)
    finally:
        for p in ctx.testwins:
            if p.poll() is None:
                p.kill()
        ctx.ctl("dispatch", "focusmonitor", str(ctx.primary_index + 1))
        ctx.ctl("dispatch", "workspace", str(home))
        cfg = open(ctx.config_path, encoding="utf-8").read()
        cfg = cfg.replace("[animations]\nenabled = false", "[animations]\nenabled = true").replace('layout = "master"', 'layout = "dwindle"')
        open(ctx.config_path, "w", encoding="utf-8").write(cfg)
        time.sleep(1.0)


def many_body(ctx, w, s0, home):
    batches, settle_ms = [], []
    for k, ws in enumerate(range(11, 16)):
        ctx.ctl("dispatch", "focusmonitor", str(ctx.primary_index + 1))
        ctx.ctl("dispatch", "workspace", str(ws))
        t0 = time.perf_counter()
        _, ids = spawn_at(ctx, 8, f"MANY{ws}", w["x"] + 100, w["y"] + 100)
        ok = wait_for(lambda: all(str(i) in ctx.state()["windows"] for i in ids) and not tiled_mismatches(ctx), 6, 0.01)
        settle_ms.append((time.perf_counter() - t0) * 1000)
        assert ok, f"workspace {ws}: 8 windows did not settle: {tiled_mismatches(ctx)[:3]}"
        batches.append(ids)
    n = len(managed(ctx))
    # workspace switch latency between two full workspaces, animations off
    edit_config(ctx, "[animations]\nenabled = true", "[animations]\nenabled = false")
    time.sleep(0.8)
    target = {}
    for ws, ids in ((11, batches[0]), (12, batches[1])):
        ctx.ctl("dispatch", "workspace", str(ws))
        assert wait_for(lambda: all(visible(i) for i in ids) and not tiled_mismatches(ctx), 3)
        target.update({i: frame(i) for i in ids})
    ctl_ms = []
    for _ in range(10):
        t0 = time.perf_counter()
        ctx.ctl("ping")
        ctl_ms.append((time.perf_counter() - t0) * 1000)
    sw, interference = [], []
    for _ in range(5):
        for ws, ids in ((11, batches[0]), (12, batches[1])):
            t0 = time.perf_counter()
            ctx.ctl("dispatch", "workspace", str(ws))
            assert wait_for(lambda: all(visible(i) and frame(i) == target[i] for i in ids), 3, 0.001), "switch did not settle"
            sw.append((time.perf_counter() - t0) * 1000)
            # Focus must land on the shown workspace, not on another monitor.
            if not wait_for(lambda: user32.GetForegroundWindow() in ids, 1):
                fg = user32.GetForegroundWindow()
                if cls(fg) == "Windows.UI.Core.CoreWindow" and title(fg) == "New notification":
                    interference.append(f"switch to {ws}: a notification toast held the foreground")
                    continue
                raise AssertionError(f"foreground after switch to {ws}: {fg:#x} {title(fg)!r}")
            st = ctx.state()
            assert st["focused_monitor"] == ctx.primary_index and st["focused"] in ids, \
                f"focus moved: monitor {st['focused_monitor']}, window {st['focused']}"
    a = ctx.ctl("stats")
    time.sleep(10)
    b = ctx.ctl("stats")
    idle = (b["cpu_kernel_ms"] + b["cpu_user_ms"]) - (a["cpu_kernel_ms"] + a["cpu_user_ms"])
    busy = (a["cpu_kernel_ms"] + a["cpu_user_ms"]) - (s0["cpu_kernel_ms"] + s0["cpu_user_ms"])
    strips_with = len(strips_visible())
    for p in ctx.testwins:
        if p.poll() is None:
            p.kill()
    gone = wait_for(lambda: not any(is_window(i) for ids in batches for i in ids), 5)
    time.sleep(1.0)
    ctx.ctl("dispatch", "workspace", str(home))
    time.sleep(0.5)
    st = ctx.state()
    stale = [k for k in st["borders"] if int(k) not in {int(x) for x in st["windows"]}]
    vis_strips = strips_visible()
    expected = 4 * sum(1 for v in st["borders"].values() if v["visible"])
    assert gone, "test windows did not close"
    assert not stale, f"border objects left for closed windows: {stale}"
    assert len(vis_strips) <= expected, f"{len(vis_strips)} visible strips but only {expected} expected"
    ctx.many_report = (f"{n} managed windows; settle per batch of 8 (spawn→all tiled, incl. IPC polling): "
                       f"{', '.join(f'{x:.0f}' for x in settle_ms)} ms; workspace switch 8↔8 windows, animations off, from launching "
                       f"`twmctl dispatch` until all 8 frames are in place: median {statistics.median(sw):.0f} ms, max {max(sw):.0f} ms "
                       f"(a bare `twmctl ping` round trip incl. process start: median {statistics.median(ctl_ms):.0f} ms); CPU while creating/switching {busy:.0f} ms; "
                       f"idle CPU {idle:.1f} ms over 10 s; working set {b['working_set_bytes'] / 1048576:.1f} MiB, private "
                       f"{b['private_bytes'] / 1048576:.1f} MiB; visible strips with 40 windows {strips_with}, after closing {len(vis_strips)}"
                       + (f"; NOT VERIFIED (outside interference): {interference}" if interference else ""))
    return ctx.many_report


@check("temporary resolution change of a secondary monitor is followed and reverted")
def t_display_change(ctx):
    if ctx.args.no_display_change:
        return "skipped (--no-display-change)"
    mons = ctx.state()["monitors"]
    sec = [i for i, m in enumerate(mons) if i != ctx.primary_index]
    if not sec:
        return "skipped: only one monitor"
    i = sec[0]
    dev = mons[i]["id"]
    cur = current_mode(dev)
    w0, h0 = cur.dmPelsWidth, cur.dmPelsHeight
    target = next(((w, h) for w, h in ((1280, 720), (1600, 900), (1024, 768)) if (w, h) != (w0, h0) and has_mode(dev, w, h)), None)
    if not target:
        return f"skipped: no alternative mode on {dev}"
    st = ctx.state()
    on_sec = [k for k, v in managed(ctx).items() if st["workspaces"][str(v["ws"])]["monitor"] == i and visible(k)]
    log(f"  secondary monitor index {i} ({dev}); managed: " + ", ".join(f"{k:#x} ws {v['ws']} mon {st['workspaces'][str(v['ws'])]['monitor']} vis {visible(k)}" for k, v in managed(ctx).items()))
    ctx.mode_changed = True
    rc = set_mode(dev, *target)
    assert rc == 0, f"ChangeDisplaySettingsEx returned {rc}"
    try:
        ok = wait_for(lambda: ctx.state()["monitors"][i]["rect"]["w"] == target[0], 5)
        assert ok, f"manager did not pick up the new mode: {ctx.state()['monitors'][i]['rect']}"
        time.sleep(1.5)
        bad = tiled_mismatches(ctx)
        wr = ctx.state()["monitors"][i]["work"]
        outside = [hex(k) for k in on_sec if not (wr["x"] <= frame(k)[0] and frame(k)[0] + frame(k)[2] <= wr["x"] + wr["w"])]
    finally:
        rc2 = restore_modes()
        ctx.mode_changed = False
    assert rc2 == 0, f"restoring the display mode returned {rc2}"
    assert wait_for(lambda: ctx.state()["monitors"][i]["rect"]["w"] == w0, 5), "manager did not pick up the restored mode"
    time.sleep(1.5)
    bad2 = tiled_mismatches(ctx)
    assert not outside, f"windows outside the shrunken monitor: {outside}"
    assert not bad and not bad2, f"not at tiles: during {bad[:3]}, after {bad2[:3]}"
    return f"{dev} {w0}x{h0} → {target[0]}x{target[1]} → {w0}x{h0}; {len(on_sec)} managed windows on it re-tiled both times"


@check("quit restores every pre-existing window; no overlays remain")
def t_quit(ctx):
    t0 = time.time()
    ctx.stop()
    ms = (time.time() - t0) * 1000
    time.sleep(0.5)
    diff = compare(ctx)
    assert not strips_visible(), "border strips still visible after quit"
    assert taskbars() == ctx.taskbars and work_areas() == ctx.work and explorer_pids() == ctx.explorer
    assert not diff, f"windows not restored: {diff}"
    return f"quit took {ms:.0f} ms"


@check("restart re-tiles the same windows; second quit restores them again")
def t_restart(ctx):
    ctx.start()
    time.sleep(1.5)
    m = managed(ctx)
    again = [i for i in ctx.real_managed if i in m]
    assert again, "nothing managed after restart"
    assert not tiled_mismatches(ctx), "windows not at tiles after restart"
    ctx.stop()
    time.sleep(0.5)
    diff = compare(ctx)
    assert not diff, f"windows not restored after second quit: {diff}"
    return f"{len(again)} windows re-managed"


def ensure_normal_foreground():
    """Windows refuses to let other apps take the foreground from some
    windows (an elevated app, a notification toast). Explorer can, so ask it
    to open a folder. Returns the folder window it opened, or None."""
    fg = user32.GetForegroundWindow()
    if fg and cls(fg) not in ("Windows.UI.Core.CoreWindow",) and user32.IsWindowVisible(fg):
        return None
    before = {h for h in top_windows() if cls(h) == "CabinetWClass"}
    log(f"foreground is {fg:#x} {cls(fg)!r} {title(fg)!r}; asking Explorer to open a folder to take it back")
    subprocess.run(["powershell", "-NoProfile", "-Command", f"(New-Object -ComObject Shell.Application).Open('{tempfile.gettempdir()}')"], capture_output=True)
    new = []
    wait_for(lambda: bool(new.extend(h for h in top_windows() if cls(h) == "CabinetWClass" and h not in before and h not in new) or new) and user32.GetForegroundWindow() in new, 5)
    fg = user32.GetForegroundWindow()
    if fg not in new:
        raise SystemExit(f"could not get a normal foreground window (foreground {fg:#x} {title(fg)!r}); not running")
    time.sleep(0.5)
    return fg


def compare(ctx):
    diff = []
    for h, s in ctx.snap.items():
        if not is_window(h):
            continue
        if placement(h) != s["placement"] or (visible(h) and frame(h) != s["frame"]):
            diff.append((hex(h), s["title"][:30], s["frame"], frame(h)))
    return diff


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(e2e.ROOT, "target", "release"))
    ap.add_argument("--min-idle", type=float, default=60)
    ap.add_argument("--no-display-change", action="store_true")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()
    if idle_ms() < args.min_idle * 1000:
        log(f"user active {idle_ms() / 1000:.0f} s ago; need {args.min_idle:.0f} s idle — not running")
        return 3
    tmp = tempfile.mkdtemp(prefix="twm-real-")
    ctx = Ctx(os.path.abspath(args.bin), tmp)
    ctx.args = args
    ctx.mode_changed = False
    # Default instance behaviour, but config/data in a temp dir so the user's
    # own %APPDATA% config is never touched.
    for k in ("TWM_INSTANCE", "TWM_NO_ACTIVATE"):
        ctx.env.pop(k, None)
    ctx.env["TWM_INSTANCE"] = "real-e2e"
    opened = ensure_normal_foreground()
    ctx.snap, ctx.taskbars, ctx.work, ctx.explorer = snapshot(), taskbars(), work_areas(), explorer_pids()
    log(f"snapshot: {len(ctx.snap)} visible top-level windows, taskbars {ctx.taskbars}, work areas {ctx.work}")
    try:
        t_first_run(ctx)
        if ctx.daemon is None:
            return 1
        mons = ctx.state()["monitors"]
        ctx.primary_index = next(i for i, m in enumerate(mons) if m["rect"]["x"] == 0 and m["rect"]["y"] == 0)
        ctx.primary = mons[ctx.primary_index]
        for t in (t_tiles_real, t_theme, t_animations, t_workspace, t_fullscreen, t_empty_ws, t_many, t_display_change, t_quit, t_restart):
            t(ctx)
    finally:
        if ctx.mode_changed:
            restore_modes()
        for p in ctx.testwins:
            if p.poll() is None:
                p.kill()
        ctx.stop()
        subprocess.run([os.path.join(ctx.bin, "twmctl.exe"), "restore"], env=ctx.env, capture_output=True)
        if opened and is_window(opened):
            user32.PostMessageW(opened, 0x0010, 0, 0)  # WM_CLOSE the folder window we opened
        if args.keep:
            log(f"kept {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)
    passed = sum(1 for r in RESULTS if r[1])
    log(f"\n{passed}/{len(RESULTS)} real-desktop checks passed")
    for name, ok, info in RESULTS:
        log(f"  {'PASS' if ok else 'FAIL'}  {name}  ({info})")
    left = compare(ctx)
    if left:
        log(f"  windows whose placement differs from the snapshot: {left}")
    return 0 if passed == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
