//! What's playing (MPRIS on the session bus), for the sound panel: the
//! first player's title, artist and state, and ⏮ ⏯ ⏭. Every call runs on a
//! thread of its own.

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use crate::systray::panels::{Media, MediaVerb};

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

fn players(conn: &Connection) -> Vec<String> {
    let proxy = zbus::blocking::fdo::DBusProxy::new(conn);
    let Ok(proxy) = proxy else { return Vec::new() };
    let mut names: Vec<String> = proxy
        .list_names()
        .map(|v| v.into_iter().map(|n| n.to_string()).filter(|n| n.starts_with("org.mpris.MediaPlayer2.")).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn get(conn: &Connection, name: &str, prop: &str) -> Option<OwnedValue> {
    let p =
        zbus::blocking::fdo::PropertiesProxy::builder(conn).destination(name).ok()?.path(PATH).ok()?.build().ok()?;
    p.get(zbus::names::InterfaceName::try_from(PLAYER).ok()?, prop).ok()
}

/// The player playing (or the first one), now.
pub fn fetch() -> Option<Media> {
    let conn = Connection::session().ok()?;
    let names = players(&conn);
    let mut best: Option<Media> = None;
    for n in names {
        let status = get(&conn, &n, "PlaybackStatus").and_then(|v| String::try_from(v).ok()).unwrap_or_default();
        let meta = get(&conn, &n, "Metadata");
        let (mut title, mut artist) = (String::new(), String::new());
        if let Some(m) = meta
            && let Ok(dict) = std::collections::HashMap::<String, OwnedValue>::try_from(m)
        {
            title =
                dict.get("xesam:title").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
            artist = dict
                .get("xesam:artist")
                .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
                .map(|a| a.join(", "))
                .unwrap_or_default();
        }
        let m = Media { title, artist, playing: status == "Playing" };
        if m.playing {
            return Some(m);
        }
        best.get_or_insert(m);
    }
    best.filter(|m| !m.title.is_empty())
}

/// ⏮ ⏯ ⏭ on the player playing (or the first one).
pub fn control(verb: MediaVerb) {
    std::thread::spawn(move || {
        let Ok(conn) = Connection::session() else { return };
        let names = players(&conn);
        let target = names
            .iter()
            .find(|n| {
                get(&conn, n, "PlaybackStatus").and_then(|v| String::try_from(v).ok()).as_deref() == Some("Playing")
            })
            .or(names.first())
            .cloned();
        let Some(name) = target else { return };
        let method = match verb {
            MediaVerb::Previous => "Previous",
            MediaVerb::PlayPause => "PlayPause",
            MediaVerb::Next => "Next",
        };
        if let Err(e) = conn.call_method(Some(name.as_str()), PATH, Some(PLAYER), method, &()) {
            log::debug!("{name} {method}: {e}");
        }
    });
}
