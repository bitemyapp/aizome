//! Sounds the shell makes: the one a notification asks for, and the system
//! bell's. Named sounds come from the freedesktop sound theme where it's
//! installed; files play with `pw-play` (PipeWire), else `paplay`, else
//! `aplay`, each in a process of its own, reaped on a thread.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Where the sound theme's files may be: `$XDG_DATA_HOME`,
/// `$XDG_DATA_DIRS`, NixOS's system profile, `/usr/share`.
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(h) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(h));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share"));
    }
    if let Some(d) = std::env::var_os("XDG_DATA_DIRS") {
        dirs.extend(std::env::split_paths(&d));
    }
    dirs.push(PathBuf::from("/run/current-system/sw/share"));
    dirs.push(PathBuf::from("/usr/share"));
    dirs
}

/// The theme's file for a sound name (`message-new-instant`, `bell`).
pub fn theme_file(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    for dir in data_dirs() {
        for ext in ["oga", "ogg", "wav"] {
            let p = dir.join("sounds/freedesktop/stereo").join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Play a sound file, without waiting.
pub fn play(file: PathBuf) {
    let _ = std::thread::Builder::new().name("aizome-sound".into()).spawn(move || {
        for player in ["pw-play", "paplay", "aplay"] {
            match Command::new(player)
                .arg(&file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(_) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    log::warn!("{player} {}: {e}", file.display());
                    return;
                }
            }
        }
        log::debug!("no pw-play, paplay or aplay for {}", file.display());
    });
}
