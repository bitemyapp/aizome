//! The shell's sections, typed: `[menubar]`, `[deskbar]`, `[notifications]`,
//! `[lock]` and `[palette]`.
//!
//! [`Settings`](crate::Settings) keeps these sections as plain tables (so
//! a load and save never loses a key this version doesn't know); the
//! accessors here read them into the types below, missing keys taking
//! their defaults and a key of the wrong type its default alone, and
//! [`Settings::set_shell_section`](crate::Settings::set_shell_section)
//! writes one back over its table, keeping the keys it doesn't know.

use serde::{Deserialize, Serialize};

/// How the menu bar's clock reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClockFormat {
    /// 21:42.
    #[default]
    #[serde(rename = "24h")]
    H24,
    /// 9:42 PM.
    #[serde(rename = "12h")]
    H12,
}

/// The systray's widgets, left to right.
pub const SYSTRAY_WIDGETS: [&str; 7] = ["cpu", "memory", "network", "sound", "bluetooth", "display", "battery"];

/// `[menubar]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenubarSettings {
    pub clock: ClockFormat,
    /// The clock shows seconds (it then ticks every second).
    pub clock_seconds: bool,
    /// The weekday and date before the time ("Tue 13").
    pub clock_date: bool,
    /// The systray widgets that show, from [`SYSTRAY_WIDGETS`] (a widget
    /// whose hardware is absent hides anyway).
    pub widgets: Vec<String>,
    /// The battery's percentage beside its glyph.
    pub battery_percent: bool,
}

impl Default for MenubarSettings {
    fn default() -> Self {
        MenubarSettings {
            clock: ClockFormat::H24,
            clock_seconds: false,
            clock_date: false,
            widgets: SYSTRAY_WIDGETS.iter().map(|s| s.to_string()).collect(),
            battery_percent: true,
        }
    }
}

impl MenubarSettings {
    pub fn shows(&self, widget: &str) -> bool {
        self.widgets.iter().any(|w| w == widget)
    }
}

/// `[deskbar]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeskbarSettings {
    /// Folded into the bar's `N apps ⌄` control.
    pub collapsed: bool,
    /// App ids kept in the Deskbar while not running ("Keep in Deskbar").
    pub pinned: Vec<String>,
}

/// `[notifications]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettings {
    /// File new notifications without showing them in the bar.
    pub do_not_disturb: bool,
    /// Do Not Disturb every day from this time ("22:00") …
    pub quiet_from: Option<String>,
    /// … until this one ("07:00").
    pub quiet_until: Option<String>,
    /// How long a notification shows in the bar, in seconds.
    pub banner_seconds: u32,
    /// Apps (by app id or name) whose notifications are filed silently.
    pub muted_apps: Vec<String>,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        NotificationSettings {
            do_not_disturb: false,
            quiet_from: None,
            quiet_until: None,
            banner_seconds: 5,
            muted_apps: Vec::new(),
        }
    }
}

impl NotificationSettings {
    /// Whether Do Not Disturb holds at `minutes` past local midnight: set by
    /// hand, or inside the quiet hours (which may wrap past midnight).
    pub fn quiet_at(&self, minutes: u32) -> bool {
        if self.do_not_disturb {
            return true;
        }
        let (Some(from), Some(until)) = (self.quiet_from.as_deref().and_then(hhmm), self.quiet_until.as_deref().and_then(hhmm))
        else {
            return false;
        };
        if from <= until { (from..until).contains(&minutes) } else { minutes >= from || minutes < until }
    }
}

/// Minutes past midnight of `"HH:MM"`.
pub fn hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// `[lock]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LockSettings {
    /// Lock before the machine sleeps.
    pub before_sleep: bool,
    /// The user's picture on the lock screen (else their initial).
    pub show_picture: bool,
    /// The PAM service the lock screen authenticates with.
    pub pam_service: String,
}

impl Default for LockSettings {
    fn default() -> Self {
        LockSettings { before_sleep: true, show_picture: true, pam_service: "aizome".into() }
    }
}

/// `[palette]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaletteSettings {
    /// Folders the Files scope indexes ("~" is the home folder).
    pub roots: Vec<String>,
    /// The file index is rebuilt in the background when older than this.
    pub reindex_minutes: u32,
    /// Most rows shown per section under All.
    pub per_section: u32,
}

impl Default for PaletteSettings {
    fn default() -> Self {
        PaletteSettings { roots: vec!["~".into()], reindex_minutes: 10, per_section: 4 }
    }
}

/// Reads a section's table as `T`, a field of the wrong type taking its
/// default rather than spoiling the rest.
pub(crate) fn read<T: Default + Serialize + for<'de> Deserialize<'de>>(table: Option<&toml::Table>) -> T {
    let Some(table) = table else { return T::default() };
    if let Ok(t) = toml::Value::Table(table.clone()).try_into::<T>() {
        return t;
    }
    // Field by field over the defaults.
    let Ok(toml::Value::Table(mut base)) = toml::Value::try_from(T::default()) else { return T::default() };
    for (k, v) in table {
        let mut trial = base.clone();
        trial.insert(k.clone(), v.clone());
        if toml::Value::Table(trial.clone()).try_into::<T>().is_ok() {
            base = trial;
        } else {
            log::warn!("settings: ignoring {k} = {v}");
        }
    }
    toml::Value::Table(base).try_into::<T>().unwrap_or_default()
}

/// Writes `value` over `table`, keeping the keys it doesn't have.
pub(crate) fn write<T: Serialize>(table: &mut toml::Table, value: &T) {
    if let Ok(toml::Value::Table(t)) = toml::Value::try_from(value) {
        for (k, v) in t {
            table.insert(k, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Settings;

    use super::*;

    #[test]
    fn sections_read_with_defaults() {
        let s = Settings::from_toml("[menubar]\nclock = \"12h\"\nfuture = 1\n[deskbar]\npinned = [\"org.aizome.Hako\"]\n").unwrap();
        let m = s.menubar_settings();
        assert_eq!(m.clock, ClockFormat::H12);
        assert!(m.shows("battery"));
        assert_eq!(s.deskbar_settings().pinned, ["org.aizome.Hako"]);
        assert!(!s.deskbar_settings().collapsed);
        assert_eq!(s.notification_settings().banner_seconds, 5);
        assert_eq!(s.lock_settings().pam_service, "aizome");
        assert_eq!(s.palette_settings().reindex_minutes, 10);
    }

    #[test]
    fn a_bad_key_spoils_only_itself() {
        let s = Settings::from_toml("[menubar]\nclock = 7\nclock_seconds = true\n").unwrap();
        let m = s.menubar_settings();
        assert_eq!(m.clock, ClockFormat::H24);
        assert!(m.clock_seconds);
    }

    #[test]
    fn writing_a_section_keeps_unknown_keys() {
        let mut s = Settings::from_toml("[deskbar]\nsparkle = true\n").unwrap();
        let mut d = s.deskbar_settings();
        d.collapsed = true;
        s.set_shell_section("deskbar", &d);
        let again = Settings::from_toml(&s.to_toml().unwrap()).unwrap();
        assert!(again.deskbar_settings().collapsed);
        assert_eq!(again.deskbar.get("sparkle").and_then(|v| v.as_bool()), Some(true));
        // [palette] isn't a field of its own: it lives in the extra keys.
        let mut p = again.palette_settings();
        p.per_section = 6;
        let mut again = again;
        again.set_shell_section("palette", &p);
        let back = Settings::from_toml(&again.to_toml().unwrap()).unwrap();
        assert_eq!(back.palette_settings().per_section, 6);
    }

    #[test]
    fn quiet_hours_wrap_past_midnight() {
        let n = NotificationSettings { quiet_from: Some("22:00".into()), quiet_until: Some("07:00".into()), ..Default::default() };
        assert!(n.quiet_at(23 * 60));
        assert!(n.quiet_at(6 * 60 + 59));
        assert!(!n.quiet_at(12 * 60));
        let day = NotificationSettings { quiet_from: Some("12:00".into()), quiet_until: Some("13:00".into()), ..Default::default() };
        assert!(day.quiet_at(12 * 60 + 30) && !day.quiet_at(13 * 60));
        assert!(NotificationSettings { do_not_disturb: true, ..Default::default() }.quiet_at(0));
        assert_eq!(hhmm("7:05"), Some(425));
        assert_eq!(hhmm("25:00"), None);
    }
}
