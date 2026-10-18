//! Session startup and the processes the compositor keeps alive.
//!
//! Once the Wayland socket and IPC are up, the compositor exports its
//! environment to the systemd user manager and D-Bus activation, tells
//! systemd it is ready (when started as `aizome.service`), and starts
//! `aizome-session.target`. It runs `aizome-shell` and
//! `xwayland-satellite` (when installed) and restarts them, backing off,
//! if they die.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use tracing::{info, warn};

use crate::state::{Aizome, State};

/// A process the compositor keeps running.
pub struct Supervised {
    pub name: String,
    pub argv: Vec<String>,
    child: Option<Child>,
    started: Instant,
    backoff: Duration,
    restart_at: Option<Instant>,
    /// Asked to restart: start again at once once it has gone.
    restart_now: bool,
}

#[derive(Default)]
pub struct Children {
    pub supervised: Vec<Supervised>,
    /// The environment every child gets on top of ours.
    pub env: Vec<(String, String)>,
}

/// The variables the session shares with everything it starts.
pub fn session_env(az: &Aizome) -> Vec<(String, String)> {
    let mut env = vec![
        ("WAYLAND_DISPLAY".to_string(), az.socket_name.to_string_lossy().into_owned()),
        ("XDG_CURRENT_DESKTOP".into(), "Aizome".into()),
        ("XDG_SESSION_DESKTOP".into(), "aizome".into()),
        ("XDG_SESSION_TYPE".into(), "wayland".into()),
    ];
    if let Some(p) = &az.ipc.path {
        env.push(("AIZOME_SOCKET".into(), p.to_string_lossy().into_owned()));
    }
    let shell = aizome_ipc::runtime_dir().join(format!("aizome-shell.{}.sock", az.socket_name.to_string_lossy()));
    env.push(("AIZOME_SHELL_SOCKET".into(), shell.to_string_lossy().into_owned()));
    env.extend(az.children.env.iter().cloned());
    env
}

/// Runs a program detached from the compositor, with the session's
/// environment, optionally in a folder.
pub fn spawn(az: &Aizome, argv: &[String], cwd: Option<PathBuf>) -> Result<u32, String> {
    let Some((prog, args)) = argv.split_first() else { return Err("nothing to run".into()) };
    let mut cmd = Command::new(prog);
    cmd.args(args).stdin(Stdio::null()).envs(session_env(az));
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    } else {
        cmd.current_dir(aizome_config::home_dir());
    }
    // Safety: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("{prog}: {e}"))?;
    let pid = child.id();
    // Reap it when it exits.
    std::thread::Builder::new()
        .name("aizome-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok();
    info!(pid, "spawned {prog}");
    Ok(pid)
}

/// Finds a program on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A free X display number.
fn free_x_display() -> Option<u32> {
    (1..64).find(|n| {
        !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
    })
}

/// Starts the session's helpers and talks to systemd.
pub fn start(state: &mut State) {
    let opts = state.az.options.clone();

    if !opts.no_xwayland {
        if let Some(bin) = which("xwayland-satellite") {
            if let Some(n) = free_x_display() {
                let display = format!(":{n}");
                state.az.children.env.push(("DISPLAY".into(), display.clone()));
                state
                    .az
                    .children
                    .supervised
                    .push(Supervised::new("xwayland-satellite", vec![bin.to_string_lossy().into_owned(), display]));
            }
        } else {
            info!("xwayland-satellite isn't installed; X11 apps won't run");
        }
    }
    if !opts.no_shell {
        if let Some(bin) = which("aizome-shell") {
            state
                .az
                .children
                .supervised
                .push(Supervised::new("aizome-shell", vec![bin.to_string_lossy().into_owned()]));
        } else {
            info!("aizome-shell isn't installed; running without a shell");
        }
    }
    let env = session_env(&state.az);
    for s in state.az.children.supervised.iter_mut() {
        start_child(s, &env);
    }
    // Supervision runs off the same timer that reaps.
    let _ = state.az.lh.insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state: &mut State| {
        supervise(state);
        TimeoutAction::ToDuration(Duration::from_secs(1))
    });

    if !opts.no_session {
        let env = session_env(&state.az);
        let names: Vec<String> = env.iter().map(|(k, _)| k.clone()).collect();
        let under_systemd = std::env::var_os("NOTIFY_SOCKET").is_some() || std::env::var_os("INVOCATION_ID").is_some();
        if under_systemd {
            run_quietly(
                Command::new("systemctl").arg("--user").arg("import-environment").args(&names).envs(env.clone()),
            );
        }
        let mut dbus = Command::new("dbus-update-activation-environment");
        if under_systemd {
            dbus.arg("--systemd");
        }
        run_quietly(dbus.args(&names).envs(env.clone()));
        notify_ready();
        if under_systemd {
            run_quietly(Command::new("systemctl").args(["--user", "--no-block", "start", "aizome-session.target"]));
        }
    }

    if !opts.command.is_empty() {
        let argv = opts.command.clone();
        if let Err(e) = spawn(&state.az, &argv, None) {
            warn!("{e}");
        }
    }
}

fn run_quietly(cmd: &mut Command) {
    match cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => {}
        Ok(s) => warn!("{:?} exited with {s}", cmd.get_program()),
        Err(e) => warn!("{:?}: {e}", cmd.get_program()),
    }
}

/// `READY=1` on `$NOTIFY_SOCKET` (no libsystemd needed).
pub fn notify_ready() {
    sd_notify("READY=1\nSTATUS=Aizome is running");
}

pub fn notify_stopping() {
    sd_notify("STOPPING=1");
}

pub fn sd_notify(msg: &str) {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else { return };
    let path = path.to_string_lossy().into_owned();
    let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() else { return };
    let res = if let Some(abstract_name) = path.strip_prefix('@') {
        use std::os::linux::net::SocketAddrExt;
        std::os::unix::net::SocketAddr::from_abstract_name(abstract_name.as_bytes())
            .and_then(|addr| sock.send_to_addr(msg.as_bytes(), &addr))
    } else {
        sock.send_to(msg.as_bytes(), &path)
    };
    if let Err(e) = res {
        warn!("sd_notify: {e}");
    }
}

impl Supervised {
    fn new(name: &str, argv: Vec<String>) -> Supervised {
        Supervised {
            name: name.into(),
            argv,
            child: None,
            started: Instant::now(),
            backoff: Duration::from_secs(1),
            restart_at: None,
            restart_now: false,
        }
    }
}

fn start_child(s: &mut Supervised, env: &[(String, String)]) {
    let mut cmd = Command::new(&s.argv[0]);
    cmd.args(&s.argv[1..]).stdin(Stdio::null()).envs(env.iter().cloned()).current_dir(aizome_config::home_dir());
    match cmd.spawn() {
        Ok(child) => {
            info!(pid = child.id(), "started {}", s.name);
            s.child = Some(child);
            s.started = Instant::now();
            s.restart_at = None;
        }
        Err(e) => warn!("{}: {e}", s.name),
    }
}

fn supervise(state: &mut State) {
    let now = Instant::now();
    let mut restart = Vec::new();
    for (i, s) in state.az.children.supervised.iter_mut().enumerate() {
        if let Some(child) = &mut s.child {
            if let Ok(Some(status)) = child.try_wait() {
                s.child = None;
                if std::mem::take(&mut s.restart_now) {
                    info!("{} stopped ({status}); starting it again", s.name);
                    s.restart_at = Some(now);
                    restart.push(i);
                    continue;
                }
                warn!("{} exited ({status}); restarting in {:?}", s.name, s.backoff);
                // A long run resets the backoff.
                if s.started.elapsed() > Duration::from_secs(60) {
                    s.backoff = Duration::from_secs(1);
                }
                s.restart_at = Some(now + s.backoff);
                s.backoff = (s.backoff * 2).min(Duration::from_secs(30));
            }
        } else if s.restart_at.is_some_and(|t| now >= t) {
            s.restart_now = false;
            restart.push(i);
        }
    }
    let env = session_env(&state.az);
    for i in restart {
        start_child(&mut state.az.children.supervised[i], &env);
    }
}

/// Restarts a supervised helper now (the recovery overlay's Restart
/// Shell): SIGKILL, since it may be hung, and a start as soon as it has
/// gone, without the usual backoff. Apps are untouched.
pub fn restart_supervised(state: &mut State, name: &str) -> Result<String, String> {
    let s = state
        .az
        .children
        .supervised
        .iter_mut()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("{name} isn't running under Aizome"))?;
    s.backoff = Duration::from_secs(1);
    s.restart_now = true;
    match &mut s.child {
        Some(child) => {
            let pid = child.id();
            // Safety: kill with a valid signal, to a child of ours.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            info!(pid, "restarting {name}");
        }
        None => s.restart_at = Some(Instant::now()),
    }
    // Come back for it shortly rather than at the next second (the usual
    // supervision carries on if it takes longer to go).
    let until = Instant::now() + Duration::from_secs(5);
    let _ =
        state.az.lh.insert_source(Timer::from_duration(Duration::from_millis(50)), move |_, _, state: &mut State| {
            supervise(state);
            if Instant::now() < until && state.az.children.supervised.iter().any(|s| s.restart_now) {
                TimeoutAction::ToDuration(Duration::from_millis(50))
            } else {
                TimeoutAction::Drop
            }
        });
    Ok(format!("Restarting {name}"))
}

/// Stops the helpers when the compositor quits.
pub fn stop_children(az: &mut Aizome) {
    for s in &mut az.children.supervised {
        if let Some(c) = &mut s.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
