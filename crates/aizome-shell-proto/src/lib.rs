//! The protocol on Aizome's shell socket (`$AIZOME_SHELL_SOCKET`), by
//! which apps publish their menus to the global menu bar: newline-delimited
//! JSON, one message per line, each an object with a `"type"`.
//!
//! An app connects, says [`AppMessage::Hello`], then sends its menus
//! ([`AppMessage::Menus`]) and sends them again whenever they change. The
//! shell answers with [`ShellMessage::Activate`] when someone picks an
//! item (from the bar, a menu at the pointer, a torn-off palette or the
//! command palette), [`ShellMessage::AboutToShow`] before it shows a menu
//! (so the app can bring its items' states up to date), and
//! [`ShellMessage::ServicesText`] to ask for the selected text a service
//! would act on. See `docs/shell-protocol.md`.
//!
//! Items have numeric ids that stay the same while the item does (the kit
//! derives them from the item's role, else its place and label), so the
//! shell can keep a torn-off palette's items across updates.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What an app sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AppMessage {
    /// First, once: who the app is. The shell keys its menus by `pid`,
    /// which is how it knows them for the focused window.
    Hello { app_id: String, pid: u32, name: String },
    /// The app's menus, whole: the first time and whenever they change.
    Menus { menus: Vec<Menu> },
    /// The answer to [`ShellMessage::ServicesText`]: the focused widget's
    /// selected text, if there is any.
    ServicesText {
        serial: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

/// What the shell sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ShellMessage {
    /// Item `id` was chosen.
    Activate { id: u32 },
    /// Menu `menu` (a top-level menu's or a submenu's id) is about to show:
    /// send menus again if any state changed.
    AboutToShow { menu: u32 },
    /// The selected text, please, for a service: answered with
    /// [`AppMessage::ServicesText`] and the same serial.
    ServicesText { serial: u64 },
}

/// A menu of the bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Menu {
    pub id: u32,
    pub title: String,
    pub items: Vec<Item>,
}

/// What an item is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    #[default]
    Action,
    Separator,
    Submenu,
    /// A section's name: text that does nothing.
    Header,
}

/// An item of a menu.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    #[serde(default)]
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<Shortcut>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub checked: bool,
    /// What the item is to the system (`"copy"`, `"quit"`, `"minimise"`):
    /// the shell may act on some itself, and the palette shows them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// A Lucide icon's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A submenu's items.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
}

fn yes() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A key and its modifiers, as data: ❖ is `command` (Super).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Shortcut {
    /// The key: a character (`"N"`, `","`) or a name (`"Enter"`, `"Tab"`,
    /// `"Space"`, `"Escape"`, `"Backspace"`, `"Delete"`, `"Up"`, `"Down"`,
    /// `"Left"`, `"Right"`, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"`,
    /// `"F1"`…).
    pub key: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub command: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub shift: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub alt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ctrl: bool,
}

impl Shortcut {
    /// As menus print it: ⌃ ⌥ ⇧ ❖ then the key.
    pub fn label(&self) -> String {
        let mut s = String::new();
        for (on, glyph) in [(self.ctrl, "⌃"), (self.alt, "⌥"), (self.shift, "⇧"), (self.command, "❖")] {
            if on {
                s.push_str(glyph);
            }
        }
        s.push_str(&self.key);
        s
    }
}

impl Menu {
    /// The item with `id`, here or in a submenu.
    pub fn find(&self, id: u32) -> Option<&Item> {
        fn walk(items: &[Item], id: u32) -> Option<&Item> {
            items.iter().find_map(|i| if i.id == id { Some(i) } else { walk(&i.items, id) })
        }
        walk(&self.items, id)
    }
}

/// One message as a line of JSON, newline included.
pub fn encode<T: Serialize>(msg: &T) -> String {
    let mut s = serde_json::to_string(msg).expect("protocol messages serialize");
    s.push('\n');
    s
}

/// A line's message.
pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> serde_json::Result<T> {
    serde_json::from_str(line.trim_end())
}

/// Where the shell listens: `$AIZOME_SHELL_SOCKET`, if set.
pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("AIZOME_SHELL_SOCKET").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

/// An app's end of the connection, blocking.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Client> {
        let stream = UnixStream::connect(path)?;
        let writer = stream.try_clone()?;
        Ok(Client { reader: BufReader::new(stream), writer })
    }

    /// Connect and say hello.
    pub fn hello(path: impl AsRef<Path>, app_id: &str, name: &str) -> io::Result<Client> {
        let mut c = Client::connect(path)?;
        c.send(&AppMessage::Hello { app_id: app_id.into(), pid: std::process::id(), name: name.into() })?;
        Ok(c)
    }

    pub fn send(&mut self, msg: &AppMessage) -> io::Result<()> {
        self.writer.write_all(encode(msg).as_bytes())?;
        self.writer.flush()
    }

    /// The next message, waiting for it; `None` once the shell hung up.
    /// Lines that aren't messages this side knows are skipped.
    pub fn recv(&mut self) -> io::Result<Option<ShellMessage>> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(m) = decode(&line) {
                return Ok(Some(m));
            }
        }
    }

    /// A second handle for sending from another thread than the one
    /// receiving.
    pub fn sender(&self) -> io::Result<Sender> {
        Ok(Sender(self.writer.try_clone()?))
    }
}

/// Sends to the shell (from any thread).
pub struct Sender(UnixStream);

impl Sender {
    pub fn send(&mut self, msg: &AppMessage) -> io::Result<()> {
        self.0.write_all(encode(msg).as_bytes())?;
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_tagged_kebab_case_lines() {
        let hello = AppMessage::Hello { app_id: "org.aizome.Soroban".into(), pid: 7, name: "Soroban".into() };
        let line = encode(&hello);
        assert_eq!(line, "{\"type\":\"hello\",\"app_id\":\"org.aizome.Soroban\",\"pid\":7,\"name\":\"Soroban\"}\n");
        assert_eq!(decode::<AppMessage>(&line).unwrap(), hello);
        assert_eq!(decode::<ShellMessage>("{\"type\":\"about-to-show\",\"menu\":3}").unwrap(), ShellMessage::AboutToShow { menu: 3 });
        assert_eq!(decode::<ShellMessage>("{\"type\":\"activate\",\"id\":12}\n").unwrap(), ShellMessage::Activate { id: 12 });
    }

    #[test]
    fn items_leave_out_defaults() {
        let item = Item {
            id: 4,
            label: "Copy".into(),
            shortcut: Some(Shortcut { key: "C".into(), command: true, ..Default::default() }),
            enabled: true,
            role: Some("copy".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(json, r#"{"id":4,"kind":"action","label":"Copy","shortcut":{"key":"C","command":true},"enabled":true,"role":"copy"}"#);
        let back: Item = serde_json::from_str(r#"{"id":4,"label":"Copy"}"#).unwrap();
        assert!(back.enabled && !back.checked && back.kind == ItemKind::Action);
        assert_eq!(item.shortcut.unwrap().label(), "❖C");
    }

    #[test]
    fn a_client_talks_over_a_socket() {
        let dir = std::env::temp_dir().join(format!("aizome-shell-proto-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("shell.sock");
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let shell = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
            let hello: AppMessage = decode(&lines.next().unwrap().unwrap()).unwrap();
            let menus: AppMessage = decode(&lines.next().unwrap().unwrap()).unwrap();
            let mut w = stream;
            w.write_all(encode(&ShellMessage::Activate { id: 9 }).as_bytes()).unwrap();
            (hello, menus)
        });
        let mut c = Client::hello(&path, "org.aizome.Test", "Test").unwrap();
        let menu = Menu { id: 1, title: "File".into(), items: vec![Item { id: 9, label: "New".into(), enabled: true, ..Default::default() }] };
        c.send(&AppMessage::Menus { menus: vec![menu.clone()] }).unwrap();
        assert_eq!(c.recv().unwrap(), Some(ShellMessage::Activate { id: 9 }));
        let (hello, menus) = shell.join().unwrap();
        assert!(matches!(hello, AppMessage::Hello { pid, .. } if pid == std::process::id()));
        assert_eq!(menus, AppMessage::Menus { menus: vec![menu] });
        assert_eq!(c.recv().unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
