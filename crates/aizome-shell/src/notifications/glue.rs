//! The notifications' part of the shell: what a click on a card or the
//! bar's line does.

use crate::shell::{Msg, Shell};
use aizome_kit::Cx;

impl Shell {
    /// A notification's card or line was clicked: its default action, and
    /// it's read.
    pub fn open_note(&mut self, id: u32, _cx: &mut Cx<Msg>) {
        let Some(n) = self.notes.get(id).cloned() else { return };
        if n.default_action().is_some() {
            self.note_action(id, "default");
        } else {
            self.notes.close(id);
            self.note_closed(id, crate::notifications::store::CloseReason::Dismissed);
        }
    }

    pub fn dismiss_note(&mut self, id: u32) {
        if self.notes.close(id) {
            self.note_closed(id, crate::notifications::store::CloseReason::Dismissed);
        }
    }

    /// Invoke one of a notification's actions; it closes unless resident.
    pub fn note_action(&mut self, id: u32, key: &str) {
        let Some(n) = self.notes.get(id).cloned() else { return };
        self.note_invoked(id, key);
        if !n.resident {
            self.notes.close(id);
            self.note_closed(id, crate::notifications::store::CloseReason::Dismissed);
        }
    }

    /// Tell the app an action was invoked (the D-Bus signal).
    pub fn note_invoked(&mut self, id: u32, key: &str) {
        if let Some(s) = &self.note_server {
            s.action_invoked(id, key);
        }
    }

    /// Tell the app a notification closed.
    pub fn note_closed(&mut self, id: u32, reason: crate::notifications::store::CloseReason) {
        if let Some(s) = &self.note_server {
            s.closed(id, reason);
        }
    }
}

impl Shell {
    /// What the notifications server heard.
    pub fn take_note(&mut self, e: crate::notifications::server::NoteEvent, cx: &mut Cx<Msg>) {
        use crate::notifications::server::NoteEvent;
        match e {
            NoteEvent::Notify(id, n) => {
                let filed = self.file_under(&n);
                let now = crate::clock::Local::now();
                let muted = self.notes_cfg.muted_apps.iter().any(|m| *m == n.app_name || *m == filed);
                let quiet = self.notes_cfg.quiet_at(now.minutes()) || muted || self.locked();
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let posted = self.notes.post_as(id, *n, filed, quiet, at);
                if posted.banner {
                    self.show_banner(id, cx);
                }
            }
            NoteEvent::Close(id) => {
                if self.notes.close(id) {
                    self.note_closed(id, crate::notifications::store::CloseReason::Closed);
                }
                self.banner_queue.retain(|b| *b != id);
                if self.banner == Some(id) {
                    self.banner_done(id, cx);
                }
            }
        }
    }

    /// Whether the screen is locked (notifications are only filed then).
    pub fn locked(&self) -> bool {
        self.wm.locked || self.lock.phase != crate::lock::Phase::Unlocked
    }

    /// The Deskbar app a notification is filed under: its desktop entry's
    /// app if that's running (or known), else a running app of its name,
    /// else its name.
    pub fn file_under(&self, n: &crate::notifications::store::Incoming) -> String {
        let running: Vec<String> = self.wm.deskbar(&self.deskbar_cfg.pinned).into_iter().map(|a| a.app_id).collect();
        if let Some(entry) = &n.desktop_entry {
            let entry = entry.trim_end_matches(".desktop");
            if let Some(a) = running.iter().find(|a| a.eq_ignore_ascii_case(entry) || a.rsplit('.').next().is_some_and(|l| l.eq_ignore_ascii_case(entry.rsplit('.').next().unwrap_or(entry)))) {
                return a.clone();
            }
            return entry.to_owned();
        }
        if let Some(pid) = n.pid
            && let Some(w) = self.wm.windows.values().find(|w| w.pid == Some(pid as i32))
        {
            return w.app_id.clone();
        }
        let name = n.app_name.to_lowercase();
        if let Some(a) = running.iter().find(|a| self.app_look(a).0.to_lowercase() == name || a.to_lowercase() == name) {
            return a.clone();
        }
        if n.app_name.is_empty() { "notifications".into() } else { n.app_name.clone() }
    }

    /// Show a notification in the bar's line (after the one there now).
    pub fn show_banner(&mut self, id: u32, cx: &mut Cx<Msg>) {
        if self.banner == Some(id) {
            return;
        }
        if self.banner.is_some() {
            if !self.banner_queue.contains(&id) {
                self.banner_queue.push(id);
            }
            return;
        }
        self.banner = Some(id);
        let secs = self.notes_cfg.banner_seconds.max(1) as u64;
        cx.after(std::time::Duration::from_secs(secs), Msg::BannerDone(id));
    }

    /// The line has shown long enough: the next one, if any.
    pub fn banner_done(&mut self, id: u32, cx: &mut Cx<Msg>) {
        if self.banner != Some(id) {
            return;
        }
        self.banner = None;
        if self.notes.banner_done(id) {
            self.note_closed(id, crate::notifications::store::CloseReason::Expired);
        }
        while !self.banner_queue.is_empty() {
            let next = self.banner_queue.remove(0);
            if self.notes.get(next).is_some() {
                self.show_banner(next, cx);
                break;
            }
        }
    }

    /// The bar's line for the notification showing.
    pub fn banner_view(&self) -> Option<crate::menubar::Banner> {
        let n = self.notes.get(self.banner?)?;
        let (_, glyph, hue) = self.app_look(&n.filed_under);
        let glyph = if crate::apps::catalog::glyph_for(&n.filed_under).is_some() { glyph } else { aizome_kit::Icon::Bell };
        Some(crate::menubar::Banner { id: n.id, glyph, hue, summary: n.summary.clone(), body: n.first_line().to_owned() })
    }
}
