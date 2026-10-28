"""End-to-end test of twm-settings against an isolated manager instance.

Drives the real settings window through UI Automation (tests/settings/uia.ps1):
invoking buttons and setting slider values does not move the mouse or send
keystrokes, and the window is opened without activation, so this is safe to
run while the PC is in use.

    python tests/settings/test_settings.py [--release]
"""
import ctypes
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
PROFILE = "release" if "--release" in sys.argv else "debug"
BIN = ROOT / "target" / PROFILE
UIA = ROOT / "tests" / "settings" / "uia.ps1"
TITLE = "TilingWindowsManager Settings"
INSTANCE = "settingstest"

CONFIG = """# settings test configuration
[general]
manage_by_default = false   # never touch the user's windows
focus_new_windows = false
gaps_in = 5                 # inner gap comment

[input]
keyboard_backend = "hotkey"

[binds]
# two harmless chords
"ctrl+alt+shift+f9" = "retile"
"ctrl+alt+shift+f10" = "reload"
""".replace("\n", "\r\n")

results = []


def check(name, ok, detail=""):
    results.append((name, ok))
    print(f"{'PASS' if ok else 'FAIL'}  {name}{('  -- ' + detail) if detail and not ok else ''}")


def uia(*args):
    r = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(UIA), *args],
                       capture_output=True, text=True, encoding="utf-8", timeout=60)
    out = (r.stdout + r.stderr).strip()
    if r.returncode != 0:
        raise RuntimeError(f"uia {args}: {out}")
    return out


def uia_wait(text, timeout=8.0):
    """List the UI until `text` appears (the accessibility tree fills in
    after the first frames); returns the last listing."""
    end = time.time() + timeout
    while True:
        names = uia("-Action", "list")
        if text in names or time.time() > end:
            return names
        time.sleep(0.25)


def ctl(env, *args):
    r = subprocess.run([str(BIN / "twmctl.exe"), "--json", "--instance", INSTANCE, *args],
                       capture_output=True, text=True, encoding="utf-8", env=env, timeout=20)
    return json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else None


def find_window():
    h = ctypes.windll.user32.FindWindowW(None, TITLE)
    # winit creates the window hidden and shows it after the first frame.
    return h if h and ctypes.windll.user32.IsWindowVisible(h) else 0


def wait(pred, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        v = pred()
        if v:
            return v
        time.sleep(0.1)
    return pred()


def main():
    if ctypes.windll.user32.FindWindowW(None, TITLE):
        print("A settings window is already open; close it first (the test finds the window by title).")
        return 2
    tmp =pathlib.Path(tempfile.mkdtemp(prefix="twm-settings-test-"))
    cfg_dir, data_dir = tmp / "cfg", tmp / "data"
    cfg_dir.mkdir()
    data_dir.mkdir()
    cfg = cfg_dir / "config.toml"
    cfg.write_bytes(CONFIG.encode())
    env = dict(os.environ, TWM_INSTANCE=INSTANCE, TWM_CONFIG_DIR=str(cfg_dir), TWM_DATA_DIR=str(data_dir), TWM_NO_ACTIVATE="1")
    daemon = subprocess.Popen([str(BIN / "twm.exe")], env=env)
    settings = None
    try:
        check("manager starts", wait(lambda: ctl(env, "ping")) == "pong")
        settings = subprocess.Popen([str(BIN / "twm-settings.exe"), "--config", str(cfg)], env=dict(env, TWM_SETTINGS_PAGE="appearance"))
        hwnd = wait(find_window)
        check("settings window opens", bool(hwnd))
        check("window did not take focus", ctypes.windll.user32.GetForegroundWindow() != hwnd)

        # The manager classifies the settings window with the dedicated heuristic.
        exp = ctl(env, "explain", hex(hwnd))
        reasons = exp["classification"]["reasons"] if exp else []
        check("manager recognises the settings window", any("settings window" in r for r in reasons), str(reasons))

        # Controls carry their row label as accessible name.
        names = uia_wait("ControlType.Slider | Between windows")
        check("controls are labelled", "ControlType.Slider | Between windows" in names, names[:400])

        # Edit a value and save: comment, CRLF and the other lines survive.
        uia("-Action", "setvalue", "-Name", "Between windows", "-Value", "20")
        time.sleep(0.5)
        check("unchanged file before Save", cfg.read_bytes() == CONFIG.encode())
        uia("-Action", "invoke", "-Name", "Save")
        text = wait(lambda: (b"gaps_in = 20" in cfg.read_bytes()) and cfg.read_bytes())
        ok = bool(text) and re.search(rb"gaps_in = 20 +# inner gap comment\r\n", text) is not None and b"# two harmless chords\r\n" in text and b"\n" not in text.replace(b"\r\n", b"")
        check("save edits in place (comments, CRLF kept)", ok, repr(text))
        check("backup of the original written", (cfg_dir / "config.toml.bak").read_bytes() == CONFIG.encode())

        # Remove a binding on the Key bindings page; the manager reloads by itself.
        uia("-Action", "invoke", "-Name", "Key bindings")
        uia_wait("ControlType.Button | Remove")
        uia("-Action", "invoke", "-Name", "Remove", "-Index", "0")
        time.sleep(0.3)
        uia("-Action", "invoke", "-Name", "Save")
        binds = wait(lambda: (b := ctl(env, "binds")) and len(b["binds"]) == 1 and b)
        check("manager reloaded the saved file", bool(binds), json.dumps(ctl(env, "binds")))
        check("file has one binding left", cfg.read_text().count("ctrl+alt+shift+f") == 1)

        # Unsaved changes stop the window from closing until the user decides.
        uia("-Action", "invoke", "-Name", "Appearance")
        uia_wait("ControlType.Slider | Screen edge")
        uia("-Action", "setvalue", "-Name", "Screen edge", "-Value", "30")
        time.sleep(0.3)
        ctypes.windll.user32.PostMessageW(hwnd, 0x0010, 0, 0)  # WM_CLOSE
        time.sleep(1.0)
        check("close is held while there are unsaved changes", settings.poll() is None and bool(find_window()))
        uia_wait("ControlType.Button | Discard")
        uia("-Action", "invoke", "-Name", "Discard")
        check("Discard closes the window", wait(lambda: settings.poll() is not None, 5))
        check("discarded change not written", b"gaps_out" not in cfg.read_bytes())

        # A file with a syntax error is shown, not overwritten.
        cfg.write_bytes(b"[general\r\ngaps_in = 1\r\n")
        settings = subprocess.Popen([str(BIN / "twm-settings.exe"), "--config", str(cfg)], env=env)
        wait(find_window)
        names = uia_wait("The configuration file cannot be read")
        check("syntax error page shown", "The configuration file cannot be read" in names, names[:400])
        check("broken file left untouched", cfg.read_bytes() == b"[general\r\ngaps_in = 1\r\n")
    finally:
        if settings and settings.poll() is None:
            settings.kill()
        ctl(env, "quit")
        try:
            daemon.wait(10)
        except subprocess.TimeoutExpired:
            daemon.kill()
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
