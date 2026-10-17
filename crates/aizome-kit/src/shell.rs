//! Publishing the app's menus to the Aizome shell over
//! `$AIZOME_SHELL_SOCKET` (`aizome-shell-proto`), and acting on what the
//! shell sends back. Without the socket (another desktop), the kit draws
//! the menu bar along each window's top instead.

use std::sync::mpsc::{Receiver, channel};

use aizome_shell_proto as proto;

use crate::app::Runtime;
use crate::engine::Core;
use crate::hash::hash_of;
use crate::menu::{ActionSpec, ItemSpec, MenuSpec, Shortcut, ShortcutKey};
use crate::platform::{Platform, Waker};
use crate::program::{AppInfo, Program};

/// The connection to the shell.
pub(crate) struct ShellLink {
    sender: proto::Sender,
    rx: Receiver<proto::ShellMessage>,
}

impl ShellLink {
    /// Connect and say hello, if there's a shell to talk to.
    pub fn connect(info: &AppInfo, wake: Waker) -> Option<ShellLink> {
        ShellLink::connect_to(&proto::socket_path()?, info, wake)
    }

    /// Connect to the shell at `path` and say hello.
    pub fn connect_to(path: &std::path::Path, info: &AppInfo, wake: Waker) -> Option<ShellLink> {
        let path = path.to_path_buf();
        let mut client = match proto::Client::hello(&path, &info.app_id, &info.name) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("the shell at {}: {e}; menus go in the windows", path.display());
                return None;
            }
        };
        let sender = client.sender().ok()?;
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("aizome-shell".into())
            .spawn(move || {
                while let Ok(Some(m)) = client.recv() {
                    if tx.send(m).is_err() {
                        return;
                    }
                    wake();
                }
            })
            .ok()?;
        Some(ShellLink { sender, rx })
    }

    pub fn incoming(&mut self) -> Vec<proto::ShellMessage> {
        self.rx.try_iter().collect()
    }

    pub fn send_menus(&mut self, menus: Vec<proto::Menu>) {
        if let Err(e) = self.sender.send(&proto::AppMessage::Menus { menus }) {
            log::warn!("the shell: {e}");
        }
    }

    pub fn send(&mut self, msg: &proto::AppMessage) {
        if let Err(e) = self.sender.send(msg) {
            log::warn!("the shell: {e}");
        }
    }
}

/// An item's id: its own, else its role's, else from its menu's title,
/// its label and its place, so it stays while the item does.
fn item_id(a: &ActionSpec, path: &str, index: usize) -> u32 {
    if let Some(id) = a.id {
        return id;
    }
    let h = match a.role {
        Some(r) if r != crate::menu::MenuRole::WindowList => hash_of(&("role", r.name())),
        _ => hash_of(&(path, a.label.as_str(), index)),
    };
    // Kept off 0 and the window list's range.
    ((h as u32) & 0x3fff_ffff).max(1)
}

fn menu_id(path: &str) -> u32 {
    ((hash_of(&("menu", path)) as u32) & 0x3fff_ffff).max(1) | 0x4000_0000
}

fn shortcut(s: &Shortcut) -> proto::Shortcut {
    let key = match s.key {
        ShortcutKey::Char(c) => c.to_uppercase().collect(),
        ShortcutKey::Enter => "Enter".into(),
        ShortcutKey::Tab => "Tab".into(),
        ShortcutKey::Space => "Space".into(),
        ShortcutKey::Escape => "Escape".into(),
        ShortcutKey::Backspace => "Backspace".into(),
        ShortcutKey::Delete => "Delete".into(),
        ShortcutKey::Up => "Up".into(),
        ShortcutKey::Down => "Down".into(),
        ShortcutKey::Left => "Left".into(),
        ShortcutKey::Right => "Right".into(),
        ShortcutKey::Home => "Home".into(),
        ShortcutKey::End => "End".into(),
        ShortcutKey::PageUp => "PageUp".into(),
        ShortcutKey::PageDown => "PageDown".into(),
        ShortcutKey::F(n) => format!("F{n}"),
    };
    proto::Shortcut { key, command: s.command, shift: s.shift, alt: s.alt, ctrl: s.ctrl }
}

fn items(spec: &MenuSpec, path: &str) -> Vec<proto::Item> {
    let mut out = Vec::new();
    for (i, item) in spec.items.iter().enumerate() {
        out.push(match item {
            ItemSpec::Action(a) => proto::Item {
                id: item_id(a, path, i),
                kind: proto::ItemKind::Action,
                label: a.label.clone(),
                shortcut: a.shortcut.as_ref().map(shortcut),
                enabled: a.enabled,
                checked: a.checked,
                role: a.role.map(|r| r.name().to_owned()),
                icon: a.icon.map(|i| i.name().to_owned()),
                items: Vec::new(),
            },
            ItemSpec::Separator => proto::Item { id: 0, kind: proto::ItemKind::Separator, enabled: false, ..Default::default() },
            ItemSpec::Header(t) => proto::Item { id: 0, kind: proto::ItemKind::Header, label: t.clone(), enabled: false, ..Default::default() },
            ItemSpec::Submenu(m) => {
                let sub = format!("{path}/{}", m.title);
                proto::Item {
                    id: menu_id(&sub),
                    kind: proto::ItemKind::Submenu,
                    label: m.title.clone(),
                    enabled: !m.items.is_empty(),
                    items: items(m, &sub),
                    ..Default::default()
                }
            }
        });
    }
    out
}

/// The menus as the shell gets them, their states settled.
pub(crate) fn published(core: &Core) -> Vec<proto::Menu> {
    core.menubar
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut m = m.clone();
            core.settle_menu(&mut m);
            let path = if i == 0 { "app".to_owned() } else { m.title.clone() };
            proto::Menu { id: menu_id(&path), title: m.title.clone(), items: items(&m, &path) }
        })
        .collect()
}

/// The action with the shell's id `id`.
pub(crate) fn find(core: &Core, id: u32) -> Option<ActionSpec> {
    fn walk(spec: &MenuSpec, path: &str, id: u32) -> Option<ActionSpec> {
        for (i, item) in spec.items.iter().enumerate() {
            match item {
                ItemSpec::Action(a) if item_id(a, path, i) == id => return Some(a.clone()),
                ItemSpec::Submenu(m) => {
                    if let Some(a) = walk(m, &format!("{path}/{}", m.title), id) {
                        return Some(a);
                    }
                }
                _ => {}
            }
        }
        None
    }
    core.menubar.iter().enumerate().find_map(|(i, m)| {
        let mut m = m.clone();
        core.settle_menu(&mut m);
        let path = if i == 0 { "app".to_owned() } else { m.title.clone() };
        walk(&m, &path, id)
    })
}

/// What the shell said.
pub(crate) fn handle<P: Program>(rt: &mut Runtime<P>, msg: proto::ShellMessage, platform: &mut dyn Platform) {
    match msg {
        proto::ShellMessage::Activate { id } => {
            let Some(a) = find(&rt.core, id) else {
                log::debug!("the shell chose item {id}, which isn't there any more");
                return;
            };
            let win = rt.core.key_window.or_else(|| rt.core.app_windows().first().map(|(w, _)| *w));
            if let Some(win) = win {
                rt.core.perform(&a, win, platform);
            }
        }
        proto::ShellMessage::AboutToShow { .. } => {
            rt.refresh_menus(platform);
            rt.publish_menus();
        }
        proto::ShellMessage::ServicesText { serial } => {
            let text = rt.core.focused_selection();
            if let Some(s) = &mut rt.shell {
                s.send(&proto::AppMessage::ServicesText { serial, text });
            }
        }
        // Answers to requests, which apps don't make.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::MenuBar;
    use crate::theme::Appearance;

    #[test]
    fn published_menus_keep_their_ids() {
        let info = AppInfo::new("org.aizome.Test", "Test", "Tester");
        let mut core = Core::new(info.clone(), Appearance::default());
        core.menubar = MenuBar::<()>::standard(&info).into_specs();
        let a = published(&core);
        let b = published(&core);
        assert_eq!(a, b);
        assert_eq!(a[0].title, "Test");
        let edit = &a[1];
        let copy = edit.items.iter().find(|i| i.label == "Copy").expect("Copy");
        assert_eq!(copy.role.as_deref(), Some("copy"));
        assert_eq!(copy.shortcut.as_ref().map(|s| s.label()), Some("❖C".to_owned()));
        // Without a focused text widget, Copy is disabled.
        assert!(!copy.enabled);
        let found = find(&core, copy.id).expect("found by its id");
        assert_eq!(found.label, "Copy");
        let find_menu = edit.items.iter().find(|i| i.kind == proto::ItemKind::Submenu).expect("Find ▸");
        assert!(find_menu.items.iter().any(|i| i.label == "Find…"));
        // Every action's id is distinct.
        let mut ids: Vec<u32> = Vec::new();
        fn collect(items: &[proto::Item], ids: &mut Vec<u32>) {
            for i in items {
                if i.kind == proto::ItemKind::Action {
                    ids.push(i.id);
                }
                collect(&i.items, ids);
            }
        }
        for m in &a {
            collect(&m.items, &mut ids);
        }
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
