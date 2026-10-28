//! twmctl — control and inspect a running TilingWindowsManager.
//!
//! Exit codes: 0 success, 1 the request failed / config has errors,
//! 2 the window manager is not running.

#[path = "../../twm/src/paths.rs"]
#[allow(dead_code)]
mod paths;

use serde_json::Value;
use twm_core::ipc::{Request, Response};

const USAGE: &str = "\
twmctl — control TilingWindowsManager

USAGE: twmctl [--json] [--instance NAME] <command> [args]

Commands that talk to the running manager:
  dispatch <action...>   run an action, e.g. `twmctl dispatch workspace 3`
  state                  monitors, workspaces and managed windows
  windows                all visible windows with process/class/title and decision
  explain <hwnd>         why a window is (not) managed; which rules match
  binds                  active key bindings
  diagnostics            problems found in the configuration
  stats                  memory/CPU/timing counters
  reload                 reload the configuration file
  ping | version | quit

Commands that work without the manager:
  validate [file]        check a configuration file (default: your config)
  restore                show windows a crashed instance left hidden
  paths                  print the configuration, log and recovery paths
";

fn call(req: &Request) -> Result<Response, String> {
    twm_core::ipc::call(&paths::pipe_name(), req)
}

fn print_table(rows: &[Vec<String>]) {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols).map(|c| rows.iter().map(|r| r.get(c).map(|s| s.chars().count()).unwrap_or(0)).max().unwrap_or(0).min(60)).collect();
    for r in rows {
        let line: Vec<String> = r.iter().enumerate().map(|(i, s)| {
            let s: String = s.chars().take(60).collect();
            format!("{s:<w$}", w = widths[i])
        }).collect();
        println!("{}", line.join("  ").trim_end());
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

fn pretty(cmd: &str, data: &Value) {
    match cmd {
        "windows" => {
            let mut rows = vec![vec!["HWND".into(), "DECISION".into(), "WS".into(), "PROCESS".into(), "CLASS".into(), "TITLE".into()]];
            for w in data.as_array().into_iter().flatten() {
                let managed = w["managed"].as_bool().unwrap_or(false);
                let dec = format!("{}{}", s(&w["decision"]), if managed { "" } else { " (unmanaged)" });
                rows.push(vec![format!("{:#x}", w["hwnd"].as_u64().unwrap_or(0)), dec, s(&w["workspace"]), s(&w["process"]), s(&w["class"]), s(&w["title"])]);
            }
            print_table(&rows);
        }
        "state" => {
            println!("focused monitor {}, focused window {}, paused {}", data["focused_monitor"].as_u64().map(|m| m + 1).unwrap_or(0), data["focused"].as_u64().map(|v| format!("{v:#x}")).unwrap_or("-".into()), data["paused"]);
            for (i, m) in data["monitors"].as_array().into_iter().flatten().enumerate() {
                println!("monitor {} {} {}x{} @ {},{} dpi {} — workspace {}", i + 1, s(&m["id"]), m["rect"]["w"], m["rect"]["h"], m["rect"]["x"], m["rect"]["y"], m["dpi"], m["active_ws"]);
            }
            let mut rows = vec![vec!["HWND".into(), "WS".into(), "MODE".into(), "RECT".into(), "PROCESS".into(), "TITLE".into()]];
            for (_, w) in data["windows"].as_object().into_iter().flatten() {
                let id = w["id"].as_u64().unwrap_or(0);
                let rect = data["desired"]["placements"].as_array().and_then(|ps| ps.iter().find(|p| p["id"].as_u64() == Some(id))).map(|p| format!("{},{} {}x{}", p["rect"]["x"], p["rect"]["y"], p["rect"]["w"], p["rect"]["h"])).unwrap_or_else(|| "hidden".into());
                rows.push(vec![format!("{id:#x}"), s(&w["ws"]), s(&w["mode"]), rect, s(&w["facts"]["process"]), s(&w["facts"]["title"])]);
            }
            print_table(&rows);
        }
        "binds" => {
            println!("keyboard backend: {}", s(&data["backend"]));
            let mut rows = vec![vec!["CHORD".into(), "ACTION".into(), "STATUS".into()]];
            for b in data["binds"].as_array().into_iter().flatten() {
                rows.push(vec![s(&b["chord"]), s(&b["description"]), if b["registered"].as_bool() == Some(false) { "NOT REGISTERED".into() } else { "ok".into() }]);
            }
            print_table(&rows);
        }
        "diagnostics" | "reload" => {
            let d = data["diagnostics"].as_array().cloned().unwrap_or_default();
            if d.is_empty() {
                println!("configuration OK");
            }
            for x in d {
                let line = x["line"].as_u64().map(|l| format!(":{l}")).unwrap_or_default();
                println!("{}: {}{}: {}: {}", s(&x["severity"]), s(&x["file"]), line, s(&x["setting"]), s(&x["message"]));
            }
            for f in data["bind_failures"].as_array().into_iter().flatten() {
                println!("warning: binding not registered: {}", s(f));
            }
        }
        _ => println!("{}", serde_json::to_string_pretty(data).unwrap_or_default()),
    }
}

fn validate(path: Option<String>) -> i32 {
    let path = path.map(std::path::PathBuf::from).unwrap_or_else(paths::default_config_file);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return 1;
        }
    };
    let themes = paths::themes_dir();
    let r = twm_core::config::load_str(&text, &path.display().to_string(), None, &|n| std::fs::read_to_string(themes.join(format!("{n}.toml"))).ok());
    for d in &r.diagnostics {
        println!("{d}");
    }
    let errors = r.errors().count();
    let warnings = r.diagnostics.len() - errors;
    println!("{}: {errors} error(s), {warnings} warning(s), {} binding(s), {} rule(s)", path.display(), r.config.binds.len(), r.config.rules.len());
    i32::from(errors > 0)
}

fn restore() -> i32 {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let path = paths::recovery_file();
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!("nothing to restore ({} does not exist)", path.display());
        return 0;
    };
    let map: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap_or_default();
    let mut n = 0;
    for (k, v) in &map {
        let Ok(id) = k.parse::<u64>() else { continue };
        let h = HWND(id as usize as *mut _);
        let mut pid = 0u32;
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                GetWindowThreadProcessId(h, Some(&mut pid));
                if Some(pid as u64) == v["pid"].as_u64() {
                    let _ = ShowWindowAsync(h, SW_SHOWNA);
                    n += 1;
                    println!("restored {id:#x} {}", s(&v["title"]));
                }
            }
        }
    }
    let _ = std::fs::remove_file(&path);
    println!("{n} window(s) restored");
    0
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut json = false;
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--json" => {
                json = true;
                args.remove(0);
            }
            "--instance" if args.len() > 1 => {
                std::env::set_var("TWM_INSTANCE", &args[1]);
                args.drain(..2);
            }
            _ => break,
        }
    }
    let Some(cmd) = args.first().cloned() else {
        print!("{USAGE}");
        std::process::exit(1);
    };
    let rest = &args[1..];
    let req = match cmd.as_str() {
        "validate" => std::process::exit(validate(rest.first().cloned())),
        "restore" => std::process::exit(restore()),
        "paths" => {
            println!("config:   {}\nthemes:   {}\nlog:      {}\nrecovery: {}\npipe:     {}", paths::default_config_file().display(), paths::themes_dir().display(), paths::log_file().display(), paths::recovery_file().display(), paths::pipe_name());
            return;
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            return;
        }
        "dispatch" if !rest.is_empty() => Request::Dispatch { action: rest.join(" ") },
        "explain" if rest.len() == 1 => {
            let h = rest[0].trim_start_matches("0x");
            let id = u64::from_str_radix(h, 16).or_else(|_| rest[0].parse::<u64>());
            match id {
                Ok(id) => Request::Explain { hwnd: id },
                Err(_) => {
                    eprintln!("explain: `{}` is not a window handle", rest[0]);
                    std::process::exit(1);
                }
            }
        }
        "state" => Request::State,
        "windows" => Request::Windows,
        "binds" => Request::Binds,
        "diagnostics" => Request::Diagnostics,
        "stats" => Request::Stats,
        "reload" => Request::Reload,
        "ping" => Request::Ping,
        "version" => Request::Version,
        "quit" => Request::Quit,
        _ => {
            eprint!("unknown or incomplete command `{}`\n\n{USAGE}", args.join(" "));
            std::process::exit(1);
        }
    };
    match call(&req) {
        Ok(r) if r.ok => {
            if json {
                println!("{}", serde_json::to_string(&r.data).unwrap_or_default());
            } else {
                pretty(&cmd, &r.data);
            }
        }
        Ok(r) => {
            eprintln!("error: {}", r.error.unwrap_or_default());
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
