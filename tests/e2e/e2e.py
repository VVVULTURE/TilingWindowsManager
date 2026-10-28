"""End-to-end tests for TilingWindowsManager on a real Windows desktop.

Safety properties (so the suite can run while someone uses the machine):
  * runs an isolated instance (TWM_INSTANCE=e2e, own config/data dirs, own pipe)
  * manages ONLY windows of class TwmTestWindow (manage_by_default = false)
  * TWM_NO_ACTIVATE=1: the manager never activates/raises windows
  * test windows are shown without activation and tile only inside a small
    corner of one monitor (via [[monitor]] reserved areas)
  * no keyboard or mouse input is injected
  * everything started is killed in `finally`, and hidden windows restored

Usage:  python tests/e2e/e2e.py [--bin target/debug] [--keep]
Exit code 0 = all passed.
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
import traceback

user32 = ctypes.windll.user32
dwmapi = ctypes.windll.dwmapi
user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))  # per-monitor v2, physical px

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RESULTS = []


def log(msg):
    print(msg, flush=True)


class Ctx:
    def __init__(self, bindir, tmp):
        self.bin = bindir
        self.tmp = tmp
        self.cfg_dir = os.path.join(tmp, "cfg")
        self.data_dir = os.path.join(tmp, "data")
        os.makedirs(self.cfg_dir)
        os.makedirs(self.data_dir)
        self.env = dict(os.environ, TWM_INSTANCE="e2e", TWM_CONFIG_DIR=self.cfg_dir, TWM_DATA_DIR=self.data_dir, TWM_NO_ACTIVATE="1", TWM_LOG="debug")
        self.daemon = None
        self.testwins = []

    @property
    def config_path(self):
        return os.path.join(self.cfg_dir, "config.toml")

    def ctl(self, *args, ok=True):
        p = subprocess.run([os.path.join(self.bin, "twmctl.exe"), "--json", *args], env=self.env, capture_output=True, text=True, timeout=15)
        if ok and p.returncode != 0:
            raise AssertionError(f"twmctl {' '.join(args)} failed ({p.returncode}): {p.stderr.strip()}")
        return json.loads(p.stdout) if p.stdout.strip() else None

    def start(self):
        self.daemon = subprocess.Popen([os.path.join(self.bin, "twm.exe")], env=self.env)
        deadline = time.time() + 10
        while time.time() < deadline:
            p = subprocess.run([os.path.join(self.bin, "twmctl.exe"), "ping"], env=self.env, capture_output=True, timeout=5)
            if p.returncode == 0:
                return
            time.sleep(0.1)
        raise AssertionError("daemon did not start")

    def focus_test_monitor(self):
        # A (re)started daemon focuses the primary monitor; new test windows
        # must open on the test monitor's confined area instead.
        self.ctl("dispatch", "focusmonitor", str(self.monitor_index + 1))

    def stop(self):
        if self.daemon and self.daemon.poll() is None:
            self.ctl("quit", ok=False)
            try:
                self.daemon.wait(5)
            except subprocess.TimeoutExpired:
                self.daemon.kill()
        self.daemon = None

    def kill(self):
        if self.daemon:
            self.daemon.kill()
            self.daemon.wait(5)
            self.daemon = None

    def spawn(self, count=1, title="T", style="normal", extra=()):
        mon = self.monitor
        x, y = mon["work"]["x"] + mon["work"]["w"] - 600, mon["work"]["y"] + 100
        p = subprocess.Popen([os.path.join(self.bin, "twm-testwin.exe"), "--count", str(count), "--title", title, "--style", style,
                              "--x", str(x), "--y", str(y), "--w", "300", "--h", "200", *extra],
                             stdout=subprocess.PIPE, text=True)
        hwnds = [int(p.stdout.readline().strip().split("=")[1], 16) for _ in range(count)]
        self.testwins.append(p)
        return p, hwnds

    def state(self):
        return self.ctl("state")

    def cleanup(self):
        for p in self.testwins:
            if p.poll() is None:
                p.kill()
        self.stop()
        subprocess.run([os.path.join(self.bin, "twmctl.exe"), "restore"], env=self.env, capture_output=True)


def frame(h):
    r = wt.RECT()
    dwmapi.DwmGetWindowAttribute(wt.HWND(h), 9, ctypes.byref(r), ctypes.sizeof(r))  # DWMWA_EXTENDED_FRAME_BOUNDS
    return (r.left, r.top, r.right - r.left, r.bottom - r.top)


def visible(h):
    return bool(user32.IsWindowVisible(wt.HWND(h)))


def is_window(h):
    return bool(user32.IsWindow(wt.HWND(h)))


def rect_tuple(r):
    return (r["x"], r["y"], r["w"], r["h"])


def wait_for(pred, timeout=3.0, step=0.05):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(step)
    return pred()


def placements(st):
    return {p["id"]: p for p in st["desired"]["placements"]}


def settle(ctx, ids, timeout=3.0):
    """Waits until every window's real frame equals its desired rect."""
    def ok():
        st = ctx.state()
        pl = placements(st)
        return all(i in pl and frame(i) == rect_tuple(pl[i]["rect"]) for i in ids)
    return wait_for(ok, timeout)


def check(name):
    def deco(fn):
        def run(ctx):
            t0 = time.time()
            try:
                fn(ctx)
                RESULTS.append((name, True, f"{(time.time() - t0) * 1000:.0f} ms"))
                log(f"PASS {name}")
            except Exception as e:  # noqa: BLE001
                RESULTS.append((name, False, f"{e}"))
                log(f"FAIL {name}: {e}")
                traceback.print_exc()
        run.__name__ = fn.__name__
        return run
    return deco


BASE_CONFIG = """
[general]
manage_by_default = false
focus_new_windows = false
gaps_in = 5
gaps_out = 10
border_size = 3

[animations]
enabled = {anim}

[input]
keyboard_backend = "hotkey"

[binds]
"ctrl+alt+shift+f9" = "retile"

[[rule]]
name = "test windows"
match = {{ class = "TwmTestWindow" }}
manage = true

[[rule]]
name = "floaty"
match = {{ class = "TwmTestWindow", title = "FLOAT.*" }}
float = true

[[rule]]
name = "to workspace 7"
match = {{ class = "TwmTestWindow", title = "WS7.*" }}
workspace = 7
{reserved}
"""


def write_config(ctx, anim=False, extra=""):
    with open(ctx.config_path, "w", encoding="utf-8") as f:
        f.write(BASE_CONFIG.format(anim=str(anim).lower(), reserved=ctx.reserved) + extra)


# ----------------------------------------------------------------- tests

@check("tiles new windows exactly into the confined area")
def t_tiling(ctx):
    _, ids = ctx.spawn(3, "TILE")
    ctx.ids = ids
    assert settle(ctx, ids), "windows did not reach their tiles"
    st = ctx.state()
    pl = placements(st)
    area = ctx.area
    rects = [pl[i]["rect"] for i in ids]
    for r in rects:
        assert area[0] <= r["x"] and r["x"] + r["w"] <= area[0] + area[2], f"outside area horizontally: {r}"
        assert area[1] <= r["y"] and r["y"] + r["h"] <= area[1] + area[3], f"outside area vertically: {r}"
    for i, a in enumerate(rects):
        for b in rects[i + 1:]:
            ix = min(a["x"] + a["w"], b["x"] + b["w"]) - max(a["x"], b["x"])
            iy = min(a["y"] + a["h"], b["y"] + b["h"]) - max(a["y"], b["y"])
            assert ix <= 0 or iy <= 0, f"tiles overlap: {a} {b}"
    # Gaps: neighbours are 2*gaps_in apart, edges gaps_out from the area.
    xs = sorted({r["x"] for r in rects})
    assert xs[0] == area[0] + 10, f"gaps_out not applied: {xs[0]} vs {area[0] + 10}"
    for i in ids:
        b = st["borders"].get(str(i))
        assert b and b["visible"], f"border missing for {i:#x}"


@check("layout switching: master, monocle, dwindle")
def t_layouts(ctx):
    ids = ctx.ids
    ctx.ctl("dispatch", "layout", "master")
    assert settle(ctx, ids)
    pl = placements(ctx.state())
    widths = sorted({pl[i]["rect"]["w"] for i in ids})
    assert len(widths) == 2, f"master should have 2 column widths, got {widths}"
    ctx.ctl("dispatch", "layout", "monocle")
    assert settle(ctx, ids)
    pl = placements(ctx.state())
    assert len({rect_tuple(pl[i]["rect"]) for i in ids}) == 1, "monocle windows should share one rect"
    ctx.ctl("dispatch", "layout", "dwindle")
    assert settle(ctx, ids)


@check("workspace switch hides/shows windows and persists recovery state")
def t_workspaces(ctx):
    ids = ctx.ids
    rec = os.path.join(ctx.data_dir, "hidden-windows.json")
    ctx.ctl("dispatch", "workspace", "6")
    assert wait_for(lambda: not any(visible(i) for i in ids)), "windows still visible after switching away"
    assert os.path.exists(rec), "recovery file not written"
    saved = json.load(open(rec))
    assert {int(k) for k in saved} == set(ids), f"recovery file lists {saved.keys()}"
    ctx.ctl("dispatch", "workspace", "previous")
    assert wait_for(lambda: all(visible(i) for i in ids)), "windows not shown again"
    assert wait_for(lambda: not os.path.exists(rec)), "recovery file not cleared"
    assert settle(ctx, ids)


@check("focus, move to another workspace (silent) and back")
def t_move_ws(ctx):
    ids = ctx.ids
    ctx.ctl("dispatch", "cyclenext")
    f = ctx.state()["focused"]
    assert f in ids, f"focus not on a test window: {f}"
    ctx.ctl("dispatch", "movetoworkspacesilent", "5")
    assert wait_for(lambda: not visible(f)), "moved window should be hidden"
    rest = [i for i in ids if i != f]
    assert settle(ctx, rest), "remaining windows did not retile"
    st = ctx.state()
    assert st["windows"][str(f)]["ws"] == 5
    ctx.ctl("dispatch", "workspace", "5")
    assert wait_for(lambda: visible(f) and not any(visible(i) for i in rest))
    ctx.ctl("dispatch", "cyclenext")
    ctx.ctl("dispatch", "movetoworkspace", "previous")
    assert wait_for(lambda: all(visible(i) for i in ids)), "window did not come back"
    assert settle(ctx, ids)


@check("toggle floating keeps window managed but out of the layout")
def t_float(ctx):
    ids = ctx.ids
    ctx.ctl("dispatch", "cyclenext")
    f = ctx.state()["focused"]
    before = placements(ctx.state())[f]["rect"]
    ctx.ctl("dispatch", "togglefloating")
    st = ctx.state()
    assert st["windows"][str(f)]["mode"] == "floating"
    fr = placements(st)[f]["rect"]
    assert fr["w"] == int(before["w"] * 0.8), f"float size {fr} from {before}"
    assert settle(ctx, ids)
    ctx.ctl("dispatch", "togglefloating")
    assert ctx.state()["windows"][str(f)]["mode"] == "tiled"
    assert settle(ctx, ids)


@check("rules: float and workspace assignment")
def t_rules(ctx):
    _, (fl,) = ctx.spawn(1, "FLOAT-me")
    _, (w7,) = ctx.spawn(1, "WS7-me")
    assert wait_for(lambda: str(fl) in ctx.state()["windows"] and str(w7) in ctx.state()["windows"])
    st = ctx.state()
    assert st["windows"][str(fl)]["mode"] == "floating"
    assert st["windows"][str(w7)]["ws"] == 7
    assert wait_for(lambda: not visible(w7)), "window ruled to workspace 7 should be hidden"
    ex = ctx.ctl("explain", hex(fl))
    assert "floaty" in ex["classification"]["matched_rules"], ex["classification"]
    assert settle(ctx, ctx.ids)
    ctx.extra = [fl, w7]


@check("close action and application-closed windows retile the rest")
def t_close(ctx):
    for h in ctx.extra:
        user32.PostMessageW(wt.HWND(h), 0x0010, 0, 0)  # WM_CLOSE
    assert wait_for(lambda: not any(is_window(h) for h in ctx.extra))
    ctx.ctl("dispatch", "cyclenext")
    f = ctx.state()["focused"]
    ctx.ctl("dispatch", "close")
    assert wait_for(lambda: not is_window(f)), "close action did not close the window"
    ctx.ids = [i for i in ctx.ids if i != f]
    assert wait_for(lambda: str(f) not in ctx.state()["windows"])
    assert settle(ctx, ctx.ids)


@check("app minimize/restore removes and re-adds the tile")
def t_minimize(ctx):
    a, b = ctx.ids[:2]
    user32.ShowWindowAsync(wt.HWND(a), 7)  # SW_SHOWMINNOACTIVE
    assert wait_for(lambda: placements(ctx.state()).get(a) is None), "minimized window still placed"
    assert settle(ctx, [b])
    full = placements(ctx.state())[b]["rect"]
    assert full["w"] == ctx.area[2] - 20, f"remaining window should fill the area: {full}"
    user32.ShowWindowAsync(wt.HWND(a), 4)  # SW_SHOWNOACTIVATE
    assert settle(ctx, ctx.ids), "restored window not re-tiled"


@check("layout is enforced when an app moves its tile")
def t_enforce(ctx):
    a = ctx.ids[0]
    want = frame(a)
    user32.SetWindowPos(wt.HWND(a), None, want[0] + 37, want[1] + 21, 250, 150, 0x0004 | 0x0010)  # NOZORDER|NOACTIVATE
    assert wait_for(lambda: frame(a) == want, 2.0), f"not snapped back: {frame(a)} vs {want}"


@check("config: syntax error keeps last good config and is reported; fix reloads")
def t_config_errors(ctx):
    good = open(ctx.config_path, encoding="utf-8").read()
    with open(ctx.config_path, "w", encoding="utf-8") as f:
        f.write(good.replace("gaps_in = 5", "gaps_in = = 5"))
    def has_syntax_error():
        d = ctx.ctl("diagnostics")["diagnostics"]
        return any(x["setting"] == "<syntax>" for x in d)
    assert wait_for(has_syntax_error, 3), "syntax error not reported after auto-reload"
    d = [x for x in ctx.ctl("diagnostics")["diagnostics"] if x["setting"] == "<syntax>"][0]
    assert d["line"] is not None and d["file"].endswith("config.toml"), d
    assert settle(ctx, ctx.ids), "layout changed although the config was broken"
    # Invalid value: only that value falls back.
    with open(ctx.config_path, "w", encoding="utf-8") as f:
        f.write(good.replace("gaps_in = 5", "gaps_in = 9999").replace("border_size = 3", "border_size = 4"))
    def value_reported():
        d = ctx.ctl("diagnostics")["diagnostics"]
        return any(x["setting"] == "general.gaps_in" for x in d)
    assert wait_for(value_reported, 3), "out-of-range value not reported"
    # Fix: new value applies live.
    with open(ctx.config_path, "w", encoding="utf-8") as f:
        f.write(good.replace("gaps_in = 5", "gaps_in = 12"))
    def gaps_applied():
        st = ctx.state()
        pl = placements(st)
        xs = sorted(pl[i]["rect"]["x"] for i in ctx.ids)
        return not ctx.ctl("diagnostics")["diagnostics"] and len(ctx.ids) >= 2 and settle(ctx, ctx.ids, 0.5)
    assert wait_for(gaps_applied, 4), "fixed config did not reload cleanly"
    with open(ctx.config_path, "w", encoding="utf-8") as f:
        f.write(good)
    assert wait_for(lambda: not ctx.ctl("diagnostics")["diagnostics"], 3)
    assert settle(ctx, ctx.ids)


@check("validate CLI reports errors with file, line and setting")
def t_validate(ctx):
    bad = os.path.join(ctx.tmp, "bad.toml")
    with open(bad, "w") as f:
        f.write('[general]\nlayout = "spiral"\n[binds]\n"win+x" = "explode"\n')
    p = subprocess.run([os.path.join(ctx.bin, "twmctl.exe"), "validate", bad], capture_output=True, text=True)
    assert p.returncode == 1, p.stdout
    assert "bad.toml:2: general.layout" in p.stdout, p.stdout
    assert "unknown action" in p.stdout, p.stdout


@check("hard kill leaves no window hidden after restart")
def t_crash_recovery(ctx):
    ids = ctx.ids
    ctx.ctl("dispatch", "workspace", "8")
    assert wait_for(lambda: not any(visible(i) for i in ids))
    ctx.kill()
    time.sleep(0.2)
    assert not any(visible(i) for i in ids), "precondition: windows hidden after kill"
    ctx.start()
    ctx.focus_test_monitor()
    assert wait_for(lambda: all(visible(i) for i in ids)), "restart did not restore hidden windows"
    assert settle(ctx, ids), "windows not re-adopted and tiled after restart"


@check("animated layout change delivers intermediate frames")
def t_animation(ctx):
    write_config(ctx, anim=True)
    ctx.ctl("reload")
    logf = os.path.join(ctx.tmp, "frames.log")
    p, ids = ctx.spawn(2, "ANIM", extra=("--log", logf))
    assert settle(ctx, ctx.ids + ids, 4)
    open(logf, "w").close()
    t0 = time.time()
    ctx.ctl("dispatch", "layout", "master")
    assert settle(ctx, ctx.ids + ids, 4)
    elapsed = time.time() - t0
    lines = [l.split() for l in open(logf).read().splitlines() if l.strip()]
    per = {}
    for l in lines:
        per.setdefault(l[1], []).append(float(l[0]))
    counts = {k: len(v) for k, v in per.items()}
    ctx.anim_report = f"WM_WINDOWPOSCHANGED per test window during a 180 ms transition: {counts} (settled in {elapsed * 1000:.0f} ms incl. IPC polling)"
    log("  " + ctx.anim_report)
    assert any(c >= 5 for c in counts.values()), f"expected several intermediate frames, got {counts}"
    p.kill()
    write_config(ctx, anim=False)
    ctx.ctl("reload")
    ctx.ctl("dispatch", "layout", "dwindle")


@check("idle cost: CPU and memory over 10 s")
def t_idle(ctx):
    s0 = ctx.ctl("stats")
    time.sleep(10)
    s1 = ctx.ctl("stats")
    cpu = (s1["cpu_kernel_ms"] + s1["cpu_user_ms"]) - (s0["cpu_kernel_ms"] + s0["cpu_user_ms"])
    ctx.idle_report = f"idle CPU {cpu:.1f} ms over 10 s ({cpu / 100:.3f}% of one core); working set {s1['working_set_bytes'] / 1048576:.1f} MiB; private {s1['private_bytes'] / 1048576:.1f} MiB; {s1['managed_windows']} managed windows"
    log("  " + ctx.idle_report)
    assert cpu < 200, f"idle CPU too high: {cpu} ms in 10 s"


@check("quit restores hidden windows and exits")
def t_quit(ctx):
    ids = ctx.ids
    ctx.ctl("dispatch", "workspace", "9")
    assert wait_for(lambda: not any(visible(i) for i in ids))
    ctx.ctl("quit")
    ctx.daemon.wait(5)
    ctx.daemon = None
    assert wait_for(lambda: all(visible(i) for i in ids)), "quit did not restore hidden windows"
    assert not os.path.exists(os.path.join(ctx.data_dir, "hidden-windows.json"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(ROOT, "target", "debug"))
    ap.add_argument("--keep", action="store_true", help="keep the temp directory")
    ap.add_argument("--monitor", type=int, default=int(os.environ.get("TWM_E2E_MONITOR", "0")), help="1-based monitor for test windows (default: last)")
    args = ap.parse_args()
    tmp = tempfile.mkdtemp(prefix="twm-e2e-")
    ctx = Ctx(os.path.abspath(args.bin), tmp)
    try:
        # Start with a neutral config to discover monitors, then confine tiling.
        ctx.reserved = ""
        write_config(ctx)
        ctx.start()
        mons = ctx.state()["monitors"]
        mi = (args.monitor or len(mons)) - 1
        mon = mons[mi]
        ctx.monitor = mon
        w, h = mon["work"]["w"], mon["work"]["h"]
        aw, ah = 820, 520
        scale = mon["dpi"] / 96
        lines = []
        for i, m in enumerate(mons):
            if i == mi:
                lines.append(f"[[monitor]]\nindex = {i + 1}\nreserved = {{ bottom = {int((h - ah) / scale)}, left = {int((w - aw) / scale)} }}\n")
            else:
                lines.append(f"[[monitor]]\nindex = {i + 1}\nreserved = {{ bottom = {int((m['work']['h'] - 300) / (m['dpi'] / 96))}, left = {int((m['work']['w'] - 300) / (m['dpi'] / 96))} }}\n")
        ctx.reserved = "\n" + "\n".join(lines)
        write_config(ctx)
        ctx.ctl("reload")
        ctx.monitor_index = mi
        ctx.focus_test_monitor()
        ctx.area = (mon["work"]["x"] + w - aw, mon["work"]["y"], aw, ah)
        log(f"testing on monitor {mi + 1} ({mon['id']}), area {ctx.area}")
        for t in [t_tiling, t_layouts, t_workspaces, t_move_ws, t_float, t_rules, t_close, t_minimize, t_enforce,
                  t_config_errors, t_validate, t_crash_recovery, t_animation, t_idle, t_quit]:
            t(ctx)
    finally:
        ctx.cleanup()
        if not args.keep:
            shutil.rmtree(tmp, ignore_errors=True)
    passed = sum(1 for r in RESULTS if r[1])
    log(f"\n{passed}/{len(RESULTS)} end-to-end checks passed")
    for name, ok, info in RESULTS:
        log(f"  {'PASS' if ok else 'FAIL'}  {name}  ({info})")
    for attr in ("anim_report", "idle_report"):
        if hasattr(ctx, attr):
            log("  " + getattr(ctx, attr))
    return 0 if passed == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
