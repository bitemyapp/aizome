//! Unprivileged inspection of the live Aizome session. The helper's
//! `read_system` never connects to a user's compositor or follows their PATH.
use super::{Action, Completion};
use aizome_ipc::{Client, ReloadReport, Request, Response};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

type Environment = BTreeMap<String, String>;
const ENVIRONMENT: &str = "etc/xdg/aizome/environment";

struct Installed {
    compositor: PathBuf,
    shell: PathBuf,
    abi: String,
}

struct Running {
    compositor: PathBuf,
    abi: String,
    environment: Environment,
    cwd: PathBuf,
    logic_healthy: bool,
    renderer_after_restart: bool,
    reload: ReloadReport,
}

pub(super) fn read(booted: &Path, active: &Path) -> Completion {
    inspect(booted, active).unwrap_or_else(|reason| Completion::new(Action::Unknown, reason))
}

fn inspect(booted: &Path, active: &Path) -> Result<Completion, String> {
    // Always compare with the activated system. `aizome` on the user's PATH
    // can be a development build or an older profile entry.
    let compositor = std::fs::canonicalize(active.join("sw/bin/aizome"))
        .map_err(|_| "The active system's Aizome compositor could not be verified.")?;
    let shell = std::fs::canonicalize(active.join("sw/bin/aizome-shell"))
        .map_err(|_| "The active system's Aizome shell could not be verified.")?;
    let package = compositor.parent().and_then(Path::parent).ok_or("Aizome's package could not be located.")?;
    if !package.join("lib/aizome/libaizome_comp_logic.so").is_file() {
        return Err("The active system's Aizome logic library could not be found.".into());
    }
    let build = super::read_json::<serde_json::Value>(&package.join("share/aizome/build-env.json"))
        .map_err(|_| "The installed Aizome core's build information could not be read.")?;
    let abi = abi(&build["core"])?;
    let installed = Installed { compositor, shell, abi };
    let socket = aizome_ipc::socket_path().ok_or("The running Aizome session could not be located.")?;
    let running = snapshot(&socket, Path::new("/proc"), Duration::from_secs(2))?;
    let previous = environment_file(&booted.join(ENVIRONMENT), false)?;
    let current = environment_file(&active.join(ENVIRONMENT), true)?;
    let config = running
        .environment
        .get("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| running.environment.get("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("Aizome's user environment configuration could not be located.")?;
    let user = environment_file(&config.join("aizome/environment"), false)?;
    let changed = environment_changes(&previous, &current, &user, &running.environment);
    // A concurrent switch must not mix two generations into an all-clear.
    if std::fs::canonicalize("/run/current-system").ok().as_deref() != Some(active) {
        return Err("The active system changed during this check. Refresh Yukimi to check again.".into());
    }
    Ok(assess(&installed, &running, &changed))
}

fn abi(build: &serde_json::Value) -> Result<String, String> {
    build["abi"]
        .as_str()
        .filter(|abi| abi.len() == 64 && abi.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
        .ok_or_else(|| "The Aizome core's build fingerprint could not be verified.".into())
}

fn snapshot(socket: &Path, proc: &Path, timeout: Duration) -> Result<Running, String> {
    let failed = || {
        "The running Aizome desktop could not be checked. Refresh Yukimi to try again; desktop update completion is unverified.".to_owned()
    };
    let mut client = Client::connect_to(socket).map_err(|_| failed())?;
    client.stream().set_read_timeout(Some(timeout)).map_err(|_| failed())?;
    client.stream().set_write_timeout(Some(timeout)).map_err(|_| failed())?;
    let Response::Version { pid, protocol, exe: Some(exe), build: Some(build), .. } =
        client.request(&Request::Version).map_err(|_| failed())?
    else {
        return Err(failed());
    };
    if pid == 0 || protocol != aizome_ipc::PROTOCOL_VERSION {
        return Err(failed());
    }
    let process = proc.join(pid.to_string());
    let compositor = std::fs::read_link(process.join("exe")).map_err(|_| failed())?;
    if compositor != Path::new(&exe) {
        return Err(failed());
    }
    let abi = abi(&build)?;
    // This is the compositor's environment, not Yukimi's (which may have
    // been launched from a terminal with its own PATH and shell overrides).
    let raw = std::fs::read(process.join("environ")).map_err(|_| failed())?;
    let environment = String::from_utf8(raw)
        .map_err(|_| failed())?
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let cwd = std::fs::read_link(process.join("cwd")).map_err(|_| failed())?;
    let Response::Stats { stats } = client.request(&Request::Stats { reset: false }).map_err(|_| failed())? else {
        return Err(failed());
    };
    if stats.logic.core_abi != abi {
        return Err(failed());
    }
    let Response::Reload { report } = client
        .request(&Request::Reload { logic: false, shell: false, from: None, shell_from: None, check: true })
        .map_err(|_| failed())?
    else {
        return Err(failed());
    };
    Ok(Running {
        compositor,
        abi,
        environment,
        cwd,
        logic_healthy: stats.logic.healthy,
        renderer_after_restart: stats.renderer_after_restart,
        reload: report,
    })
}

fn assess(installed: &Installed, running: &Running, environment_changed: &[String]) -> Completion {
    let mut result = Completion::default();
    if installed.abi != running.abi {
        result.action = Action::Logout;
        result.reasons.push("The installed Aizome compositor core differs from the one running. Sign out and back in to use it; restarting only the shell does not replace the compositor.".into());
    }
    if !environment_changed.is_empty() {
        result.action = Action::Logout;
        result.reasons.push(format!("Aizome's startup environment changed ({}). A new login is needed to apply those settings; a shell reload keeps the session's old environment.", environment_changed.join(", ")));
    }
    if running.renderer_after_restart {
        result.action = Action::Logout;
        result.reasons.push("Aizome's configured renderer takes effect at the next login.".into());
    }
    let Some(path) = running.environment.get("PATH") else {
        result.action = Action::Unknown;
        result
            .reasons
            .insert(0, "The session's PATH is unavailable, so its Aizome launch paths could not be verified.".into());
        return result;
    };
    for (name, expected, selected) in [
        ("aizome", &installed.compositor, program("aizome", path, &running.cwd)),
        (
            "aizome-shell",
            &installed.shell,
            program(
                running
                    .environment
                    .get("AIZOME_SHELL_BIN")
                    .filter(|value| !value.is_empty())
                    .map(String::as_str)
                    .unwrap_or("aizome-shell"),
                path,
                &running.cwd,
            ),
        ),
    ] {
        let configuration = if name == "aizome" { "profile and PATH" } else { "profile, PATH and AIZOME_SHELL_BIN" };
        match selected {
            Some(selected) if selected == *expected => (),
            Some(selected) => {
                result.action = Action::Unknown;
                result.reasons.insert(0, format!("The session selects {name} from {}, instead of the active system's {}. Check your {configuration} override before relying on a reload or new login to use the update.", selected.display(), expected.display()));
            }
            None => {
                result.action = Action::Unknown;
                result.reasons.insert(0, format!("The session's {name} launch path could not be verified. Check your {configuration} before finishing the desktop update."));
            }
        }
    }
    if result.action != Action::None {
        return result;
    }
    // A development compositor reloads the library beside itself, not the
    // system package. Its check cannot certify a system update as applied.
    if running.compositor != installed.compositor && !running.compositor.starts_with("/nix/store") {
        return Completion::new(
            Action::Unknown,
            "Aizome is running from a development build. Its reload does not necessarily select the active system's desktop. Check the session's launch configuration before signing out and back in.",
        );
    }
    if !running.logic_healthy || running.reload.core.action != "same" {
        return Completion::new(
            Action::Unknown,
            "The running Aizome core or logic could not be verified as ready. Check Aizome's recovery status, then refresh Yukimi.",
        );
    }
    for (name, part) in [("compositor logic", &running.reload.logic), ("shell", &running.reload.shell)] {
        match part.action.as_str() {
            "unchanged" => (),
            "would-reload" => {
                if result.action == Action::None {
                    result.action = Action::Reload;
                }
                result.reasons.push(format!("The running Aizome {name} has not picked up the installed version."));
            }
            _ => {
                result.action = Action::Unknown;
                result.reasons.push(
                    format!("The Aizome {name} update could not be verified ({}). {}", part.action, part.detail)
                        .trim_end()
                        .to_owned(),
                );
            }
        }
    }
    // Settings are live. `reload --check` always says `would-reload` for
    // them; it is not evidence of unapplied changes.
    result
}

fn program(name: &str, path: &str, cwd: &Path) -> Option<PathBuf> {
    let candidates = if name.contains('/') {
        vec![cwd.join(name)]
    } else {
        std::env::split_paths(path).map(|dir| cwd.join(dir).join(name)).collect()
    };
    candidates
        .into_iter()
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .and_then(|candidate| std::fs::canonicalize(candidate).ok())
}

fn environment_file(path: &Path, required: bool) -> Result<Environment, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if !required && e.kind() == std::io::ErrorKind::NotFound => return Ok(Environment::new()),
        Err(_) => return Err("Aizome's startup environment configuration could not be read.".into()),
    };
    // Match aizome-session: literal KEY=VALUE lines, without shell expansion.
    Ok(text
        .split_terminator('\n')
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect())
}

fn environment_changes(
    previous: &Environment,
    current: &Environment,
    user: &Environment,
    running: &Environment,
) -> Vec<String> {
    let mut expected = current.clone();
    expected.extend(user.clone());
    let keys: BTreeSet<_> = previous.keys().chain(expected.keys()).collect();
    keys.into_iter()
        .filter(|key| match expected.get(*key) {
            Some(value) => running.get(*key) != Some(value),
            // A variable removed since boot can remain in the old compositor
            // and user manager. Do not compare unrelated application variables.
            None => running.get(*key).is_some_and(|value| previous.get(*key) == Some(value)),
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aizome_ipc::ReloadPart;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::{fs::symlink, net::UnixListener};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "yukimi-aizome-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("bin")).unwrap();
            for name in ["aizome", "aizome-shell"] {
                let path = root.join("bin").join(name);
                std::fs::write(&path, "this fixture must never be executed").unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            Self { root }
        }
        fn installed(&self) -> Installed {
            Installed {
                compositor: self.root.join("bin/aizome"),
                shell: self.root.join("bin/aizome-shell"),
                abi: "a".repeat(64),
            }
        }
        fn running(&self) -> Running {
            Running {
                compositor: self.installed().compositor,
                abi: self.installed().abi,
                environment: [("PATH".into(), self.root.join("bin").display().to_string())].into(),
                cwd: self.root.clone(),
                logic_healthy: true,
                renderer_after_restart: false,
                reload: ReloadReport {
                    core: ReloadPart::new("same", ""),
                    logic: ReloadPart::new("unchanged", ""),
                    shell: ReloadPart::new("unchanged", ""),
                    settings: ReloadPart::new("would-reload", ""),
                },
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn changed_core_needs_a_login_even_after_the_shell_was_restarted() {
        let f = Fixture::new();
        let mut running = f.running();
        running.abi = "b".repeat(64);
        running.reload.core = ReloadPart::new("same", "an older PATH target can report the same core");
        running.reload.logic = ReloadPart::new("refused", "different core");
        let c = assess(&f.installed(), &running, &[]);
        assert_eq!(c.action, Action::Logout);
        assert!(c.description().contains("restarting only the shell does not replace the compositor"));
        assert!(c.description().contains("No computer restart is needed"));
    }

    #[test]
    fn compatible_logic_or_shell_changes_need_a_reload_not_logout() {
        let f = Fixture::new();
        for logic in [false, true] {
            let mut running = f.running();
            if logic {
                running.reload.logic.action = "would-reload".into();
            } else {
                running.reload.shell.action = "would-reload".into();
            }
            let c = assess(&f.installed(), &running, &[]);
            assert_eq!(c.action, Action::Reload);
            assert!(c.description().contains("Reload Aizome"));
            assert!(!c.description().contains("Sign out"));
        }
    }

    #[test]
    fn current_components_are_applied_even_though_settings_would_reload() {
        let f = Fixture::new();
        assert_eq!(assess(&f.installed(), &f.running(), &[]), Completion::default());
        let mut running = f.running();
        // Another package generation can contain the same core build.
        running.compositor = "/nix/store/previous-aizome/bin/aizome".into();
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::None);
    }

    #[test]
    fn profile_override_prevents_an_all_clear_or_misleading_sign_out_button() {
        let f = Fixture::new();
        let old = Fixture::new();
        let mut running = f.running();
        running.abi = "b".repeat(64);
        running
            .environment
            .insert("PATH".into(), format!("{}:{}", old.root.join("bin").display(), f.root.join("bin").display()));
        let c = assess(&f.installed(), &running, &[]);
        assert_eq!(c.action, Action::Unknown);
        assert!(c.description().contains("instead of the active system"));
        assert!(c.description().contains("Sign out and back in"));
        assert!(c.description().contains("Check your profile"));
        assert!(c.reasons[0].contains("instead of the active system"));
    }

    #[test]
    fn shell_override_is_detected_even_if_the_core_and_logic_are_current() {
        let f = Fixture::new();
        let old = Fixture::new();
        let mut running = f.running();
        running.environment.insert("AIZOME_SHELL_BIN".into(), old.root.join("bin/aizome-shell").display().to_string());
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Unknown);
        running.environment.insert("AIZOME_SHELL_BIN".into(), f.installed().shell.display().to_string());
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::None);
    }

    #[test]
    fn launch_paths_follow_symlinks_and_the_compositors_working_directory() {
        let f = Fixture::new();
        std::fs::create_dir(f.root.join("profile")).unwrap();
        symlink(f.root.join("bin/aizome"), f.root.join("profile/aizome")).unwrap();
        assert_eq!(program("aizome", "profile:bin", &f.root), Some(f.installed().compositor));
        assert_eq!(program("./bin/aizome-shell", "", &f.root), Some(f.installed().shell));
        assert_eq!(program("absent", "bin", &f.root), None);
        let mut running = f.running();
        running.environment.remove("PATH");
        running.cwd = f.root.join("bin");
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Unknown);
    }

    #[test]
    fn development_compositor_cannot_certify_the_system_update() {
        let f = Fixture::new();
        let mut running = f.running();
        running.compositor = f.root.join("target/debug/aizome");
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Unknown);
    }

    #[test]
    fn a_pending_renderer_change_also_needs_a_new_login() {
        let f = Fixture::new();
        let mut running = f.running();
        running.renderer_after_restart = true;
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Logout);
    }

    #[test]
    fn missing_refused_or_future_reload_results_are_not_success() {
        let f = Fixture::new();
        for action in ["", "skipped", "refused", "failed", "future-action"] {
            for logic in [false, true] {
                let mut running = f.running();
                if logic {
                    running.reload.logic.action = action.into();
                } else {
                    running.reload.shell.action = action.into();
                }
                assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Unknown, "{action}");
            }
        }
        let mut running = f.running();
        running.logic_healthy = false;
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Unknown);
        running.logic_healthy = true;
        running.reload.core.action = "skipped".into();
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::Unknown);
    }

    #[test]
    fn environment_changes_include_removed_variables_and_respect_user_settings() {
        let old: Environment =
            [("SDL_VIDEODRIVER".into(), "wayland,x11".into()), ("QT_QPA_PLATFORM".into(), "wayland".into())].into();
        let current: Environment = [("QT_QPA_PLATFORM".into(), "wayland;xcb".into())].into();
        assert_eq!(
            environment_changes(&old, &current, &Environment::new(), &old),
            vec!["QT_QPA_PLATFORM", "SDL_VIDEODRIVER"]
        );
        assert!(environment_changes(&old, &current, &old, &old).is_empty());
        assert!(environment_changes(&old, &current, &Environment::new(), &current).is_empty());
        let f = Fixture::new();
        let c = assess(&f.installed(), &f.running(), &["SDL_VIDEODRIVER".into()]);
        assert_eq!(c.action, Action::Logout);
        assert!(c.description().contains("old environment"));
    }

    #[test]
    fn environment_files_use_the_session_launchers_literal_syntax() {
        let f = Fixture::new();
        let path = f.root.join("environment");
        std::fs::write(&path, "#ignored=yes\nKEY=first\nKEY=$literal=last\nEMPTY=\n").unwrap();
        assert_eq!(
            environment_file(&path, true).unwrap(),
            [("KEY".into(), "$literal=last".into()), ("EMPTY".into(), "".into())].into()
        );
        assert!(environment_file(&f.root.join("missing"), true).is_err());
        assert!(environment_file(&f.root.join("missing"), false).unwrap().is_empty());
    }

    #[test]
    fn build_fingerprints_must_be_present_and_complete() {
        for value in [serde_json::json!({}), serde_json::json!({"abi":""}), serde_json::json!({"abi":"unknown"})] {
            assert!(abi(&value).is_err());
        }
        assert_eq!(abi(&serde_json::json!({"abi":"a".repeat(64)})).unwrap(), "a".repeat(64));
    }

    #[test]
    fn ipc_inspection_only_checks_and_reads_the_compositors_environment() {
        let f = Fixture::new();
        let process = f.root.join("proc/42");
        std::fs::create_dir_all(&process).unwrap();
        symlink(f.installed().compositor, process.join("exe")).unwrap();
        symlink(&f.root, process.join("cwd")).unwrap();
        std::fs::write(process.join("environ"), format!("PATH={}\0AIZOME_SHELL_BIN=\0", f.root.join("bin").display()))
            .unwrap();
        let socket = f.root.join("ipc");
        let listener = UnixListener::bind(&socket).unwrap();
        let version = Response::Version {
            version: "same-version-label".into(),
            protocol: 1,
            pid: 42,
            exe: Some(f.installed().compositor.display().to_string()),
            build: Some(serde_json::json!({"abi":f.installed().abi})),
        };
        let reload = Response::Reload { report: f.running().reload };
        let stats = Response::Stats {
            stats: aizome_ipc::Stats {
                logic: aizome_ipc::LogicStats { core_abi: f.installed().abi, healthy: true, ..Default::default() },
                ..Default::default()
            },
        };
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for (expected, response) in [
                (Request::Version, version),
                (Request::Stats { reset: false }, stats),
                (Request::Reload { logic: false, shell: false, from: None, shell_from: None, check: true }, reload),
            ] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), expected);
                writeln!(stream, "{}", serde_json::to_string(&response).unwrap()).unwrap();
            }
        });
        let running = snapshot(&socket, &f.root.join("proc"), Duration::from_secs(1)).unwrap();
        assert_eq!(assess(&f.installed(), &running, &[]).action, Action::None);
        server.join().unwrap();
    }

    #[test]
    fn absent_or_unresponsive_ipc_is_unverified_and_does_not_wait_forever() {
        let f = Fixture::new();
        let socket = f.root.join("ipc");
        assert!(snapshot(&socket, &f.root, Duration::from_millis(20)).is_err());
        let listener = UnixListener::bind(&socket).unwrap();
        let (done, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let start = std::time::Instant::now();
        assert!(snapshot(&socket, &f.root, Duration::from_millis(20)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        done.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn old_or_invalid_ipc_responses_cannot_be_an_all_clear() {
        for response in [
            "not json",
            r#"{"response":"error","message":"unsupported"}"#,
            r#"{"response":"version","version":"0.1.0","protocol":1}"#,
        ] {
            let f = Fixture::new();
            let socket = f.root.join("ipc");
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap()).read_line(&mut request).unwrap();
                assert_eq!(serde_json::from_str::<Request>(&request).unwrap(), Request::Version);
                writeln!(stream, "{response}").unwrap();
            });
            assert!(snapshot(&socket, &f.root, Duration::from_secs(1)).is_err());
            server.join().unwrap();
        }
    }
}
