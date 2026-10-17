//! What the window and every pane share: the settings file, the system
//! services, the window, and the status strip.

use std::path::PathBuf;
use std::time::Duration;

use aizome_config::Settings;
use aizome_kit::{Cx, WindowKey};

use crate::app::AppMsg;
use crate::system::System;

/// How long a status line stays.
const STATUS_FOR: Duration = Duration::from_secs(5);
/// Changes within this of each other ask the compositor to reload once.
const RELOAD_AFTER: Duration = Duration::from_millis(120);

/// A line in the status strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
    pub generation: u64,
}

pub struct Shared {
    /// `settings.toml` as last read or written.
    pub settings: Settings,
    /// Where it lives: `aizome_config::Settings::path()`, or a test's.
    pub settings_path: PathBuf,
    /// Fixtures instead of the live system; nothing changes the machine.
    pub sample: bool,
    pub window: WindowKey,
    pub sys: System,
    pub status: Option<Status>,
    status_generation: u64,
    reload_pending: bool,
    /// Programs launched (in the sample: only recorded), for tests.
    pub launched: Vec<Vec<String>>,
    /// Times the compositor was asked to reload, for tests.
    pub reloads: u32,
}

impl Shared {
    pub fn new(settings_path: PathBuf, sample: bool) -> Shared {
        let settings = Settings::load_from(&settings_path).unwrap_or_else(|e| {
            log::warn!("{}: {e}; showing the defaults", settings_path.display());
            Settings::default()
        });
        Shared {
            settings,
            settings_path,
            sample,
            window: WindowKey::NONE,
            sys: System::empty(sample),
            status: None,
            status_generation: 0,
            reload_pending: false,
            launched: Vec::new(),
            reloads: 0,
        }
    }

    /// Read the file again (someone else wrote it).
    pub fn reread(&mut self) {
        if let Ok(s) = Settings::load_from(&self.settings_path) {
            self.settings = s;
        }
    }

    /// Read the file, apply `f`, write it, and ask the compositor to apply
    /// it. A file that doesn't parse is kept beside the new one.
    pub fn change(&mut self, f: impl FnOnce(&mut Settings), cx: &mut Cx<AppMsg>) {
        let mut fresh = match Settings::load_from(&self.settings_path) {
            Ok(s) => s,
            Err(aizome_config::Error::Parse(e)) => {
                let keep = self.settings_path.with_extension("toml.broken");
                let _ = std::fs::copy(&self.settings_path, &keep);
                self.say(format!("settings.toml didn't parse ({e}); kept it as {}", keep.display()), true, cx);
                self.settings.clone()
            }
            Err(_) => self.settings.clone(),
        };
        f(&mut fresh);
        if let Err(e) = fresh.save_to(&self.settings_path) {
            self.say(format!("Couldn't save {}: {e}", self.settings_path.display()), true, cx);
            return;
        }
        self.settings = fresh;
        if !self.reload_pending {
            self.reload_pending = true;
            cx.after(RELOAD_AFTER, AppMsg::Reload);
        }
    }

    /// Ask the compositor to apply the file (`reload-config`): the shell
    /// and every app follow its `config-changed`.
    pub fn reload(&mut self, cx: &mut Cx<AppMsg>) {
        self.reload_pending = false;
        self.reloads += 1;
        if self.sample || aizome_ipc::socket_path().is_none() {
            return;
        }
        cx.spawn(|| {
            let r = aizome_ipc::Client::connect().and_then(|mut c| c.request(&aizome_ipc::Request::ReloadConfig));
            match r {
                Ok(aizome_ipc::Response::Error { message }) => AppMsg::Said(format!("The compositor refused the settings: {message}"), true),
                Ok(_) => AppMsg::Nothing,
                // No compositor (another desktop): the file is enough.
                Err(e) => {
                    log::info!("reload-config: {e}");
                    AppMsg::Nothing
                }
            }
        });
    }

    /// Show `text` in the status strip for a few seconds.
    pub fn say(&mut self, text: String, error: bool, cx: &mut Cx<AppMsg>) {
        self.status_generation += 1;
        let g = self.status_generation;
        if error {
            log::warn!("{text}");
        }
        self.status = Some(Status { text, error, generation: g });
        cx.after(if error { STATUS_FOR * 2 } else { STATUS_FOR }, AppMsg::ClearStatus(g));
    }

    pub fn clear_status(&mut self, generation: u64) {
        if self.status.as_ref().is_some_and(|s| s.generation == generation) {
            self.status = None;
        }
    }

    /// Start a program, detached; in the sample only say so.
    pub fn launch(&mut self, argv: Vec<String>, cx: &mut Cx<AppMsg>) {
        let Some(program) = argv.first().cloned() else { return };
        self.launched.push(argv.clone());
        if self.sample {
            self.say(format!("Would open {}", argv.join(" ")), false, cx);
            return;
        }
        let spawned = std::process::Command::new(&program)
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                // Reap it when it exits.
                let _ = std::thread::Builder::new().name("shitsurae-reap".into()).spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => self.say(format!("Couldn't open {program}: {e}"), true, cx),
        }
    }
}
