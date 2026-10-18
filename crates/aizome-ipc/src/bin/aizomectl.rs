//! `aizomectl`: the compositor from the command line.
//!
//! Every IPC request has a subcommand; output is readable text, or the raw
//! JSON response with `--json`. `aizomectl help` lists them.

use std::process::ExitCode;

use aizome_ipc::{
    AppSel, Client, Event, OutputInfo, Percentiles, Rect, Request, Response, Side, Synthetic, Transform, WindowInfo,
};

const USAGE: &str = "\
usage: aizomectl [--json] [--socket PATH] <command> [args]

state
  version                       compositor and protocol version
  outputs                       displays, modes, scale and usable area
  windows                       every window, bottom of the stack first
  focused-window                the focused window
  desks                         the Desks, the current one marked
  stats [--reset]               frame times per output (p50/p95/p99/max)
  events                        subscribe and print events until killed

windows (ID defaults to the focused window)
  focus ID                      focus and raise (shows it if hidden)
  close [ID]                    ask a window to close
  minimise [ID] | unminimise ID
  hide-app [APP_ID|--pid PID]   hide an app (the focused one by default)
  hide-others
  show-app APP_ID|--pid PID
  fill [ID] [on|off]            fill the usable area (toggles)
  split left|right [ID]         take half of the usable area
  fullscreen [ID] [on|off]
  float [ID] [on|off]           keep above other windows
  sticky [ID] [on|off]          show on every Desk
  move ID X Y                   move the content's top-left corner
  resize ID X Y W H             set the content rectangle
  stack A B                     stack window B into A's tab group
  unstack [ID]

desks (DESK is a 1-based number or a name)
  set-desk DESK                 switch to a Desk
  move-to-desk DESK [ID] [--follow]
  add-desk NAME [--folder DIR] [--index N]
  rename-desk DESK NAME
  set-desk-folder DESK [DIR]
  remove-desk DESK
  move-desk DESK INDEX
  overview [on|off]             the Desks overview (toggles)
  switcher [--reverse]

session
  spawn [--cwd DIR] [--no-desk-folder] -- ARGV...
  screenshot [PATH] [--output NAME] [--window ID] [--area X,Y,W,H] [--clipboard]
  reload-config
  output NAME [--mode WxH[@Hz]] [--scale S] [--position X,Y]
              [--transform normal|90|180|270|flipped|flipped-90|...] [--on|--off]
  mirror NAME SOURCE|--off      show SOURCE's picture on NAME, scaled to fit
  primary NAME                  the main display, for this session
  night-light [on|off] [--temperature K]
                                Night Light: change it, then show it
  power-off-monitors | power-on-monitors
  lock                          ask the shell to lock
  action NAME                   run a key-binding action by name
  quit                          end the session

input (through the compositor's own paths, for tests and scripts)
  input motion X Y | click [BUTTON] | button BUTTON [down|up]
  input scroll DX DY | key NAME [down|up] | chord super+shift+3 | type TEXT
                                a key or button without down or up is pressed
                                and released; type taps keys for letters,
                                digits, space and plain punctuation
";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut json = false;
    let mut socket = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" | "-j" => {
                json = true;
                args.remove(i);
            }
            "--socket" if i + 1 < args.len() => {
                socket = Some(args.remove(i + 1));
                args.remove(i);
            }
            "--" => break,
            _ => i += 1,
        }
    }
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let command = args.remove(0);
    let request = match parse(&command, args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("aizomectl: {e}");
            eprintln!("(aizomectl help lists the commands)");
            return ExitCode::from(2);
        }
    };
    let client = match socket {
        Some(path) => Client::connect_to(std::path::Path::new(&path)),
        None => Client::connect(),
    };
    let mut client = match client {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aizomectl: can't reach the compositor: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Request::Subscribe = request {
        let events = match client.events() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("aizomectl: {e}");
                return ExitCode::FAILURE;
            }
        };
        for event in events {
            match event {
                Ok(e) if json => println!("{}", serde_json::to_string(&e).unwrap()),
                Ok(e) => println!("{}", describe_event(&e)),
                Err(e) => {
                    eprintln!("aizomectl: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        return ExitCode::SUCCESS;
    }
    let response = match client.request(&request) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("aizomectl: {e}");
            return ExitCode::FAILURE;
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&response).unwrap());
        return if matches!(response, Response::Error { .. }) { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    }
    print_response(&response)
}

fn parse(command: &str, mut args: Vec<String>) -> Result<Request, String> {
    let flag = |args: &mut Vec<String>, name: &str| -> bool {
        if let Some(i) = args.iter().position(|a| a == name) {
            args.remove(i);
            true
        } else {
            false
        }
    };
    let opt = |args: &mut Vec<String>, name: &str| -> Result<Option<String>, String> {
        if let Some(i) = args.iter().position(|a| a == name) {
            if i + 1 >= args.len() {
                return Err(format!("{name} needs a value"));
            }
            let v = args.remove(i + 1);
            args.remove(i);
            Ok(Some(v))
        } else {
            Ok(None)
        }
    };
    let id = |s: &str| s.parse::<u64>().map_err(|_| format!("not a window ID: {s}"));
    let opt_id = |args: &[String], at: usize| -> Result<Option<u64>, String> { args.get(at).map(|s| id(s)).transpose() };
    let int = |s: &str| s.parse::<i32>().map_err(|_| format!("not a number: {s}"));
    let need = |args: &[String], n: usize, what: &str| -> Result<(), String> {
        if args.len() < n { Err(format!("{command} needs {what}")) } else { Ok(()) }
    };
    // `on`/`off` anywhere among the arguments sets the state; the rest is an ID.
    let state_and_id = |args: &mut Vec<String>| -> Result<(Option<bool>, Option<u64>), String> {
        let mut state = None;
        args.retain(|a| match a.as_str() {
            "on" | "true" => {
                state = Some(true);
                false
            }
            "off" | "false" => {
                state = Some(false);
                false
            }
            _ => true,
        });
        Ok((state, opt_id(args, 0)?))
    };
    let app = |args: &mut Vec<String>| -> Result<AppSel, String> {
        let pid = opt(args, "--pid")?.map(|p| p.parse::<i32>().map_err(|_| format!("not a pid: {p}"))).transpose()?;
        Ok(AppSel { app_id: args.first().cloned(), pid })
    };
    Ok(match command {
        "version" => Request::Version,
        "outputs" => Request::Outputs,
        "windows" => Request::Windows,
        "focused-window" | "focused" => Request::FocusedWindow,
        "desks" => Request::Desks,
        "stats" => Request::Stats { reset: flag(&mut args, "--reset") },
        "events" | "subscribe" => Request::Subscribe,
        "focus" => {
            need(&args, 1, "a window ID")?;
            Request::Focus { id: id(&args[0])? }
        }
        "close" => Request::Close { id: opt_id(&args, 0)? },
        "minimise" | "minimize" => Request::Minimise { id: opt_id(&args, 0)? },
        "unminimise" | "unminimize" => {
            need(&args, 1, "a window ID")?;
            Request::Unminimise { id: id(&args[0])? }
        }
        "hide-app" => Request::HideApp { app: app(&mut args)? },
        "hide-others" => Request::HideOthers,
        "show-app" => Request::ShowApp { app: app(&mut args)? },
        "fill" => {
            let (state, id) = state_and_id(&mut args)?;
            Request::Fill { id, state }
        }
        "fullscreen" => {
            let (state, id) = state_and_id(&mut args)?;
            Request::Fullscreen { id, state }
        }
        "float" => {
            let (state, id) = state_and_id(&mut args)?;
            Request::Float { id, state }
        }
        "sticky" => {
            let (state, id) = state_and_id(&mut args)?;
            Request::Sticky { id, state }
        }
        "split" => {
            need(&args, 1, "left or right")?;
            let side = match args[0].as_str() {
                "left" => Side::Left,
                "right" => Side::Right,
                s => return Err(format!("split takes left or right, not {s}")),
            };
            Request::Split { id: opt_id(&args, 1)?, side }
        }
        "move" => {
            need(&args, 3, "ID X Y")?;
            Request::Move { id: Some(id(&args[0])?), x: int(&args[1])?, y: int(&args[2])? }
        }
        "resize" => {
            need(&args, 5, "ID X Y W H")?;
            Request::Resize {
                id: Some(id(&args[0])?),
                rect: Rect::new(int(&args[1])?, int(&args[2])?, int(&args[3])?, int(&args[4])?),
            }
        }
        "stack" => {
            need(&args, 2, "two window IDs")?;
            Request::Stack { a: id(&args[0])?, b: id(&args[1])? }
        }
        "unstack" => Request::Unstack { id: opt_id(&args, 0)? },
        "set-desk" | "desk" => {
            need(&args, 1, "a Desk")?;
            Request::SetDesk { desk: args[0].parse().unwrap() }
        }
        "move-to-desk" => {
            let follow = flag(&mut args, "--follow");
            need(&args, 1, "a Desk")?;
            Request::MoveToDesk { desk: args[0].parse().unwrap(), id: opt_id(&args, 1)?, follow }
        }
        "add-desk" => {
            let folder = opt(&mut args, "--folder")?;
            let index = opt(&mut args, "--index")?.map(|s| s.parse::<u32>().map_err(|_| "bad --index".to_string())).transpose()?;
            need(&args, 1, "a name")?;
            Request::AddDesk { name: args[0].clone(), folder, index }
        }
        "rename-desk" => {
            need(&args, 2, "a Desk and a name")?;
            Request::RenameDesk { desk: args[0].parse().unwrap(), name: args[1].clone() }
        }
        "set-desk-folder" => {
            need(&args, 1, "a Desk")?;
            Request::SetDeskFolder { desk: args[0].parse().unwrap(), folder: args.get(1).cloned() }
        }
        "remove-desk" => {
            need(&args, 1, "a Desk")?;
            Request::RemoveDesk { desk: args[0].parse().unwrap() }
        }
        "move-desk" => {
            need(&args, 2, "a Desk and a position")?;
            Request::MoveDesk {
                desk: args[0].parse().unwrap(),
                index: args[1].parse().map_err(|_| format!("not a position: {}", args[1]))?,
            }
        }
        "overview" => {
            let (state, _) = state_and_id(&mut args)?;
            Request::Overview { state }
        }
        "switcher" => Request::Switcher { reverse: flag(&mut args, "--reverse"), state: None },
        "spawn" => {
            let cwd = opt(&mut args, "--cwd")?;
            let no_desk = flag(&mut args, "--no-desk-folder");
            if args.first().map(|a| a == "--").unwrap_or(false) {
                args.remove(0);
            }
            need(&args, 1, "a command")?;
            Request::Spawn { argv: args, cwd, in_desk_folder: !no_desk }
        }
        "screenshot" => {
            let output = opt(&mut args, "--output")?;
            let window = opt(&mut args, "--window")?.map(|s| id(&s)).transpose()?;
            let area = opt(&mut args, "--area")?
                .map(|s| {
                    let v: Vec<i32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                    if v.len() == 4 { Ok(Rect::new(v[0], v[1], v[2], v[3])) } else { Err(format!("bad --area {s}")) }
                })
                .transpose()?;
            let clipboard = flag(&mut args, "--clipboard");
            let path = args.first().map(|p| {
                let p = std::path::Path::new(p);
                if p.is_absolute() {
                    p.to_string_lossy().into_owned()
                } else {
                    std::env::current_dir().map(|d| d.join(p)).unwrap_or(p.to_path_buf()).to_string_lossy().into_owned()
                }
            });
            Request::Screenshot { output, path, window, area, clipboard }
        }
        "reload-config" | "reload" => Request::ReloadConfig,
        "output" => {
            let mode = opt(&mut args, "--mode")?;
            let scale = opt(&mut args, "--scale")?.map(|s| s.parse::<f64>().map_err(|_| format!("bad scale {s}"))).transpose()?;
            let position = opt(&mut args, "--position")?
                .map(|s| {
                    let v: Vec<i32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                    if v.len() == 2 { Ok([v[0], v[1]]) } else { Err(format!("bad --position {s}")) }
                })
                .transpose()?;
            let transform = opt(&mut args, "--transform")?
                .map(|t| serde_json::from_value::<Transform>(serde_json::Value::String(t.clone())).map_err(|_| format!("bad transform {t}")))
                .transpose()?;
            let enabled = if flag(&mut args, "--on") {
                Some(true)
            } else if flag(&mut args, "--off") {
                Some(false)
            } else {
                None
            };
            need(&args, 1, "an output name")?;
            Request::OutputConfig { name: args[0].clone(), mode, scale, position, transform, enabled }
        }
        "mirror" => {
            let off = flag(&mut args, "--off");
            need(&args, if off { 1 } else { 2 }, "an output and the output to mirror (or --off)")?;
            Request::MirrorOutput { name: args[0].clone(), source: if off { None } else { Some(args[1].clone()) } }
        }
        "primary" => {
            need(&args, 1, "an output name")?;
            Request::SetPrimaryOutput { name: args[0].clone() }
        }
        "night-light" => {
            let temperature = opt(&mut args, "--temperature")?.map(|t| t.parse::<u32>().map_err(|_| format!("bad temperature {t}"))).transpose()?;
            let enabled = match args.first().map(String::as_str) {
                Some("on") => Some(true),
                Some("off") => Some(false),
                None => None,
                Some(o) => return Err(format!("{o}: on or off")),
            };
            Request::NightLight { enabled, temperature }
        }
        "power-off-monitors" => Request::PowerOffMonitors,
        "power-on-monitors" => Request::PowerOnMonitors,
        "lock" => Request::Lock,
        "quit" => Request::Quit,
        "action" => {
            need(&args, 1, "an action name")?;
            Request::Action { action: args[0].clone() }
        }
        "input" => {
            need(&args, 1, "an input event")?;
            let f = |s: &str| s.parse::<f64>().map_err(|_| format!("not a number: {s}"));
            // down, up, or (with neither) a press and its release.
            let presses = |s: Option<&String>| -> Result<Vec<bool>, String> {
                match s.map(|s| s.as_str()) {
                    None => Ok(vec![true, false]),
                    Some("down" | "press" | "pressed") => Ok(vec![true]),
                    Some("up" | "release" | "released") => Ok(vec![false]),
                    Some(o) => Err(format!("{o}: say down or up, or neither for both")),
                }
            };
            let events = match args[0].as_str() {
                "motion" | "move" => {
                    need(&args, 3, "X Y")?;
                    vec![Synthetic::PointerMotion { x: f(&args[1])?, y: f(&args[2])? }]
                }
                "click" => vec![Synthetic::Click { button: args.get(1).cloned().unwrap_or_else(|| "left".into()) }],
                "button" => {
                    need(&args, 2, "a button")?;
                    presses(args.get(2))?.into_iter().map(|pressed| Synthetic::PointerButton { button: args[1].clone(), pressed }).collect()
                }
                "scroll" => {
                    need(&args, 3, "DX DY")?;
                    vec![Synthetic::PointerScroll { dx: f(&args[1])?, dy: f(&args[2])? }]
                }
                "key" => {
                    need(&args, 2, "a key")?;
                    presses(args.get(2))?.into_iter().map(|pressed| Synthetic::Key { key: args[1].clone(), pressed }).collect()
                }
                "chord" => {
                    need(&args, 2, "keys like super+tab")?;
                    vec![Synthetic::Chord { keys: args[1].clone() }]
                }
                "type" => {
                    need(&args, 2, "text")?;
                    let mut v = Vec::new();
                    for c in args[1..].join(" ").chars() {
                        let key = match c {
                            ' ' => "space".to_string(),
                            '\n' => "enter".to_string(),
                            c => c.to_string(),
                        };
                        let shifted = c.is_ascii_uppercase();
                        let key = if shifted { key.to_lowercase() } else { key };
                        if shifted {
                            v.push(Synthetic::Chord { keys: format!("shift+{key}") });
                        } else {
                            v.push(Synthetic::Key { key: key.clone(), pressed: true });
                            v.push(Synthetic::Key { key, pressed: false });
                        }
                    }
                    v
                }
                e => return Err(format!("unknown input {e}")),
            };
            Request::Synthesize { events }
        }
        other => return Err(format!("unknown command {other}")),
    })
}

fn print_response(r: &Response) -> ExitCode {
    match r {
        Response::Ok => {}
        Response::Error { message } => {
            eprintln!("aizomectl: {message}");
            return ExitCode::FAILURE;
        }
        Response::Version { version, protocol } => println!("aizome {version} (IPC protocol {protocol})"),
        Response::Outputs { outputs } => {
            for o in outputs {
                print_output(o);
            }
        }
        Response::Windows { windows } => {
            if windows.is_empty() {
                println!("no windows");
            }
            for w in windows {
                print_window(w);
            }
        }
        Response::Window { window } => match window {
            Some(w) => print_window(w),
            None => println!("no window has focus"),
        },
        Response::Desks { desks, .. } => {
            for d in desks {
                println!(
                    "{} {:>2}  {:<16} {:>2} window{}{}",
                    if d.current { "▸" } else { " " },
                    d.index,
                    d.name,
                    d.windows,
                    if d.windows == 1 { " " } else { "s" },
                    d.folder.as_deref().map(|f| format!("  {f}")).unwrap_or_default()
                );
            }
        }
        Response::Stats { stats } => {
            println!("{} on {}", stats.renderer, stats.backend);
            if stats.renderer_after_restart {
                println!("renderer setting: {} (after the next login)", stats.renderer_setting);
            }
            for o in &stats.outputs {
                println!(
                    "{}: {} frames rendered, {} skipped (no damage), {} samples",
                    o.name, o.frames_rendered, o.frames_skipped, o.samples
                );
                println!("  cpu {}", fmt_pct(&o.cpu_us));
                if let Some(g) = &o.gpu_us {
                    println!("  gpu {}", fmt_pct(g));
                }
            }
        }
        Response::Screenshot { path } => println!("{path}"),
        Response::Spawned { pid } => println!("{pid}"),
        Response::NightLight { state } => {
            let on = if state.enabled { "on" } else { "off" };
            let now = if state.active { "warm now" } else { "neutral now" };
            println!("night light {on}, {} K, {} schedule; {now} ({} K)", state.temperature, state.schedule, state.current);
            if state.outputs.is_empty() {
                println!("no display here can show it");
            } else {
                println!("on {}", state.outputs.join(", "));
            }
        }
    }
    ExitCode::SUCCESS
}

fn fmt_pct(p: &Percentiles) -> String {
    let ms = |us: f64| us / 1000.0;
    format!(
        "p50 {:.3}ms  p95 {:.3}ms  p99 {:.3}ms  max {:.3}ms  mean {:.3}ms",
        ms(p.p50),
        ms(p.p95),
        ms(p.p99),
        ms(p.max),
        ms(p.mean)
    )
}

fn print_output(o: &OutputInfo) {
    let mode = o
        .current_mode
        .as_ref()
        .map(|m| format!("{}x{} @ {:.3} Hz", m.width, m.height, m.refresh as f64 / 1000.0))
        .unwrap_or_else(|| "off".into());
    let mut tags = Vec::new();
    if !o.enabled {
        tags.push("(disabled)".to_string());
    }
    if o.primary {
        tags.push("(primary)".to_string());
    }
    if let Some(m) = &o.mirror {
        tags.push(format!("(mirrors {m})"));
    }
    println!("{} \"{} {}\" {}", o.name, o.make, o.model, tags.join(" "));
    println!("  mode      {mode}");
    println!("  scale     {}", o.scale);
    println!("  transform {:?}", o.transform);
    println!("  layout    {},{} {}x{} pt", o.logical.x, o.logical.y, o.logical.w, o.logical.h);
    println!("  usable    {},{} {}x{} pt", o.usable.x, o.usable.y, o.usable.w, o.usable.h);
    if !o.powered {
        println!("  powered off");
    }
    if o.modes.len() > 1 {
        let modes: Vec<String> =
            o.modes.iter().map(|m| format!("{}x{}@{:.2}", m.width, m.height, m.refresh as f64 / 1000.0)).collect();
        println!("  modes     {}", modes.join(" "));
    }
}

fn print_window(w: &WindowInfo) {
    let mut flags = Vec::new();
    if w.focused {
        flags.push("focused".to_string());
    }
    if w.minimised {
        flags.push("minimised".into());
    }
    if w.hidden {
        flags.push("hidden".into());
    }
    if w.urgent {
        flags.push("urgent".into());
    }
    if w.fullscreen {
        flags.push("fullscreen".into());
    }
    if w.fill {
        flags.push("fill".into());
    }
    if w.float {
        flags.push("float".into());
    }
    if let Some(g) = w.group {
        flags.push(format!("stack {g}{}", if w.group_active { " (showing)" } else { "" }));
    }
    let desk = match (w.sticky, w.desk) {
        (true, _) => "all desks".to_string(),
        (_, Some(d)) => format!("desk {d}"),
        _ => "-".into(),
    };
    println!(
        "{:>4}  {:<24} {:<32} {:<9} {},{} {}x{}  {}",
        w.id,
        truncate(&w.app_id, 24),
        truncate(&w.title, 32),
        desk,
        w.geometry.x,
        w.geometry.y,
        w.geometry.w,
        w.geometry.h,
        flags.join(", ")
    );
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('…');
        t
    }
}

fn describe_event(e: &Event) -> String {
    match e {
        Event::WindowOpened { window } => format!("opened {} {} \"{}\"", window.id, window.app_id, window.title),
        Event::WindowClosed { id } => format!("closed {id}"),
        Event::WindowChanged { window } => format!("changed {} \"{}\"", window.id, window.title),
        Event::WindowFocused { id } => match id {
            Some(id) => format!("focused {id}"),
            None => "focused nothing".into(),
        },
        Event::DeskChanged { index, name, .. } => format!("desk {index} {name}"),
        Event::DesksChanged { desks } => {
            format!("desks {}", desks.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(" "))
        }
        Event::OutputsChanged { outputs } => {
            format!("outputs {}", outputs.iter().map(|o| o.name.as_str()).collect::<Vec<_>>().join(" "))
        }
        Event::ConfigChanged => "config changed".into(),
        Event::Binding { action, pointer, output } => {
            let mut s = format!("binding {action}");
            if let Some((x, y)) = pointer {
                s += &format!(" at {x:.0},{y:.0}");
            }
            if let Some(o) = output {
                s += &format!(" on {o}");
            }
            s
        }
        Event::ScreenshotTaken { path } => format!("screenshot {path}"),
        Event::Idle { kind } => format!("idle {kind}"),
        Event::Resumed => "resumed".into(),
        Event::LockStateChanged { locked } => format!("locked {locked}"),
        Event::NightLightChanged { state } => format!("night-light {} {} K now", if state.enabled { "on" } else { "off" }, state.current),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aizome_ipc::DeskSel;

    fn p(cmd: &str, args: &[&str]) -> Request {
        parse(cmd, args.iter().map(|s| s.to_string()).collect()).unwrap()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p("fill", &[]), Request::Fill { id: None, state: None });
        assert_eq!(p("fill", &["4", "on"]), Request::Fill { id: Some(4), state: Some(true) });
        assert_eq!(p("split", &["left"]), Request::Split { id: None, side: Side::Left });
        assert_eq!(
            p("move-to-desk", &["Mail", "3", "--follow"]),
            Request::MoveToDesk { id: Some(3), desk: DeskSel::Name("Mail".into()), follow: true }
        );
        assert_eq!(
            p("spawn", &["--", "foot", "-e", "htop"]),
            Request::Spawn { argv: vec!["foot".into(), "-e".into(), "htop".into()], cwd: None, in_desk_folder: true }
        );
        assert_eq!(
            p("output", &["eDP-1", "--scale", "2", "--transform", "90", "--off"]),
            Request::OutputConfig {
                name: "eDP-1".into(),
                mode: None,
                scale: Some(2.0),
                position: None,
                transform: Some(Transform::_90),
                enabled: Some(false)
            }
        );
    }

    #[test]
    fn keys_tap_unless_told_down_or_up() {
        let key = |k: &str, pressed| Synthetic::Key { key: k.into(), pressed };
        assert_eq!(p("input", &["key", "escape"]), Request::Synthesize { events: vec![key("escape", true), key("escape", false)] });
        assert_eq!(p("input", &["key", "super", "down"]), Request::Synthesize { events: vec![key("super", true)] });
        assert_eq!(p("input", &["key", "super", "up"]), Request::Synthesize { events: vec![key("super", false)] });
        assert_eq!(
            p("input", &["type", "Hi", "x"]),
            Request::Synthesize {
                events: vec![
                    Synthetic::Chord { keys: "shift+h".into() },
                    key("i", true),
                    key("i", false),
                    key("space", true),
                    key("space", false),
                    key("x", true),
                    key("x", false),
                ]
            }
        );
        assert!(parse("input", vec!["key".into(), "a".into(), "sideways".into()]).is_err());
    }
}
