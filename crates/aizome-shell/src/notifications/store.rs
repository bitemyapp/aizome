//! Notifications as the shell keeps them: posted, replaced, shown in the
//! bar's line for a moment, then filed under their app in the Deskbar
//! until read or dismissed. Critical ones stay as a card under the bar
//! until acted on.

use std::path::PathBuf;

/// How urgent a notification is (the spec's `urgency` hint).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    pub fn from_u8(v: u8) -> Urgency {
        match v {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// An image a notification carries.
#[derive(Clone, Debug, PartialEq)]
pub enum NoteImage {
    File(PathBuf),
    /// Straight RGBA pixels (from `image-data`).
    Pixels { width: u32, height: u32, rgba: Vec<u8> },
}

/// What an app asked to show (`Notify`'s arguments).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Incoming {
    pub app_name: String,
    /// Replace the notification with this id (0: a new one).
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    /// Pairs of action key and label.
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    /// The `desktop-entry` hint: the app's desktop entry id.
    pub desktop_entry: Option<String>,
    pub image: Option<NoteImage>,
    pub category: Option<String>,
    /// Milliseconds; -1 the server's choice, 0 never.
    pub expire_timeout: i32,
    pub transient: bool,
    pub resident: bool,
    /// The sender's process, when known.
    pub pid: Option<u32>,
}

/// A notification the shell holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    /// The Deskbar app it's filed under (an app id), or its app name.
    pub filed_under: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    pub image: Option<NoteImage>,
    pub category: Option<String>,
    pub transient: bool,
    pub resident: bool,
    /// Seconds since the epoch it came (or was last replaced).
    pub at: u64,
    pub read: bool,
}

impl Notification {
    /// The default action's key, if it has one.
    pub fn default_action(&self) -> Option<&str> {
        self.actions.iter().find(|(k, _)| k == "default").map(|(k, _)| k.as_str())
    }

    /// Actions shown as buttons (all but `default`).
    pub fn buttons(&self) -> impl Iterator<Item = &(String, String)> {
        self.actions.iter().filter(|(k, _)| k != "default")
    }

    /// The body's first line, for the bar.
    pub fn first_line(&self) -> &str {
        self.body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
    }
}

/// Why a notification closed (the spec's `NotificationClosed` reasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
    Undefined = 4,
}

/// What posting did, for the shell to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Posted {
    pub id: u32,
    pub replaced: bool,
    /// Show it in the bar's line now.
    pub banner: bool,
    /// Keep it as a card under the bar until acted on.
    pub card: bool,
}

/// Every notification the shell holds, oldest first.
#[derive(Clone, Debug, Default)]
pub struct Store {
    pub list: Vec<Notification>,
    next_id: u32,
}

impl Store {
    /// Take a notification in. `filed_under` is where the Deskbar files it;
    /// `quiet` is Do Not Disturb (or the app is muted): filed without the
    /// bar's line. Critical ones show anyway.
    pub fn post(&mut self, n: Incoming, filed_under: String, quiet: bool, now: u64) -> Posted {
        let replaced = n.replaces_id != 0 && self.list.iter().any(|x| x.id == n.replaces_id);
        let id = if replaced {
            n.replaces_id
        } else {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            while self.list.iter().any(|x| x.id == self.next_id) {
                self.next_id = self.next_id.wrapping_add(1).max(1);
            }
            self.next_id
        };
        let note = Notification {
            id,
            app_name: n.app_name,
            app_icon: n.app_icon,
            filed_under,
            summary: n.summary,
            body: strip_markup(&n.body),
            actions: n.actions,
            urgency: n.urgency,
            image: n.image,
            category: n.category,
            transient: n.transient,
            resident: n.resident,
            at: now,
            read: false,
        };
        let critical = note.urgency == Urgency::Critical;
        if replaced {
            if let Some(slot) = self.list.iter_mut().find(|x| x.id == id) {
                *slot = note;
            }
        } else {
            self.list.push(note);
        }
        // Keep the list from growing without end: the oldest read ones go.
        while self.list.len() > 200 {
            match self.list.iter().position(|x| x.read).or(Some(0)) {
                Some(i) => {
                    self.list.remove(i);
                }
                None => break,
            }
        }
        Posted { id, replaced, banner: critical || !quiet, card: critical }
    }

    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.list.iter().find(|n| n.id == id)
    }

    /// Take a notification away: whether it was there.
    pub fn close(&mut self, id: u32) -> bool {
        let before = self.list.len();
        self.list.retain(|n| n.id != id);
        self.list.len() != before
    }

    /// A transient notification goes once its line has shown.
    pub fn banner_done(&mut self, id: u32) -> bool {
        if self.get(id).is_some_and(|n| n.transient && n.urgency != Urgency::Critical) {
            return self.close(id);
        }
        false
    }

    /// Unread notifications filed under `app`, newest first.
    pub fn unread(&self, app: &str) -> Vec<&Notification> {
        self.list.iter().rev().filter(|n| !n.read && n.filed_under == app).collect()
    }

    /// Every app with unread notifications, by the newest's age.
    pub fn apps_with_unread(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in self.list.iter().rev() {
            if !n.read && !out.contains(&n.filed_under) {
                out.push(n.filed_under.clone());
            }
        }
        out
    }

    pub fn unread_count(&self) -> usize {
        self.list.iter().filter(|n| !n.read).count()
    }

    /// Mark `app`'s notifications read (its list was opened).
    pub fn mark_read(&mut self, app: &str) {
        for n in &mut self.list {
            if n.filed_under == app {
                n.read = true;
            }
        }
    }

    /// The critical ones not yet acted on, oldest first: the cards under
    /// the bar.
    pub fn cards(&self) -> Vec<&Notification> {
        self.list.iter().filter(|n| n.urgency == Urgency::Critical && !n.read).collect()
    }
}

/// The body's text without its markup (`<b>`, `<i>`, `<u>`, `<a>`, `<img>`)
/// and with its entities read.
pub fn strip_markup(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        match rest[i..].find('>') {
            Some(j) => {
                let tag = &rest[i + 1..i + j];
                if tag.eq_ignore_ascii_case("br") || tag.eq_ignore_ascii_case("br/") {
                    out.push('\n');
                }
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(app: &str, summary: &str) -> Incoming {
        Incoming { app_name: app.into(), summary: summary.into(), body: "line one\nline two".into(), ..Default::default() }
    }

    #[test]
    fn posting_files_under_the_app_and_shows_the_line() {
        let mut s = Store::default();
        let p = s.post(note("Mail", "Re: tiles"), "thunderbird".into(), false, 10);
        assert!(p.banner && !p.card && !p.replaced);
        assert_eq!(s.unread("thunderbird").len(), 1);
        assert_eq!(s.get(p.id).unwrap().first_line(), "line one");
        let q = s.post(note("Mail", "Second"), "thunderbird".into(), true, 11);
        assert!(!q.banner, "Do Not Disturb files it without the line");
        assert_eq!(s.unread("thunderbird")[0].summary, "Second");
        assert_eq!(s.apps_with_unread(), ["thunderbird"]);
        s.mark_read("thunderbird");
        assert_eq!(s.unread_count(), 0);
    }

    #[test]
    fn replacing_keeps_the_id() {
        let mut s = Store::default();
        let a = s.post(note("Build", "Compiling"), "foot".into(), false, 0);
        let b = s.post(Incoming { replaces_id: a.id, ..note("Build", "Done") }, "foot".into(), false, 1);
        assert!(b.replaced);
        assert_eq!(a.id, b.id);
        assert_eq!(s.list.len(), 1);
        assert_eq!(s.list[0].summary, "Done");
        // Replacing one that's gone makes a new one.
        s.close(a.id);
        let c = s.post(Incoming { replaces_id: a.id, ..note("Build", "Again") }, "foot".into(), false, 2);
        assert!(!c.replaced);
    }

    #[test]
    fn critical_ones_stay_as_cards_even_in_dnd() {
        let mut s = Store::default();
        let p = s.post(Incoming { urgency: Urgency::Critical, ..note("Power", "Battery low") }, "power".into(), true, 0);
        assert!(p.banner && p.card);
        assert_eq!(s.cards().len(), 1);
        assert!(!s.banner_done(p.id));
    }

    #[test]
    fn transient_ones_go_after_their_line() {
        let mut s = Store::default();
        let p = s.post(Incoming { transient: true, ..note("Vol", "50%") }, "x".into(), false, 0);
        assert!(s.banner_done(p.id));
        assert!(s.get(p.id).is_none());
    }

    #[test]
    fn markup_is_stripped() {
        assert_eq!(strip_markup("<b>Bold</b> &amp; <i>it</i><br>next"), "Bold & it\nnext");
        assert_eq!(strip_markup("a < b"), "a < b");
        assert_eq!(strip_markup("<a href=\"x\">link</a>"), "link");
    }

    #[test]
    fn actions_split_default_from_buttons() {
        let mut s = Store::default();
        let p = s.post(
            Incoming { actions: vec![("default".into(), "Open".into()), ("reply".into(), "Reply".into())], ..note("Mail", "x") },
            "m".into(),
            false,
            0,
        );
        let n = s.get(p.id).unwrap();
        assert_eq!(n.default_action(), Some("default"));
        assert_eq!(n.buttons().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["reply"]);
    }
}
