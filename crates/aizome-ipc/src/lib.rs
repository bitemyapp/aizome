//! The compositor's IPC: types, socket path and a blocking client.
//!
//! The compositor listens on `$XDG_RUNTIME_DIR/aizome.$WAYLAND_DISPLAY.sock`
//! and exports the path as `AIZOME_SOCKET`. A client writes one [`Request`]
//! per line as JSON and reads one [`Response`] per line back. After
//! [`Request::Subscribe`] (answered with [`Response::Ok`]) the connection
//! carries only [`Event`]s, one per line, until it is closed.
//!
//! Enums are tagged by a `"request"`, `"response"` or `"event"` field in
//! kebab-case, e.g. `{"request":"focus","id":7}`. `docs/ipc.md` lists
//! every message.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when a message changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;

/// A window's identity for as long as it is open. Never reused within a
/// compositor's lifetime.
pub type WindowId = u64;

/// A rectangle in the global layout, in points (logical pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
}

/// Picks a Desk: by its position (1-based, as ⌃❖1…9 count) or by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeskSel {
    Index(u32),
    Name(String),
}

impl std::fmt::Display for DeskSel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeskSel::Index(i) => write!(f, "{i}"),
            DeskSel::Name(n) => write!(f, "{n}"),
        }
    }
}

impl std::str::FromStr for DeskSel {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.parse::<u32>() {
            Ok(i) => DeskSel::Index(i),
            Err(_) => DeskSel::Name(s.to_string()),
        })
    }
}

/// Picks an app: by app_id or by process ID. With neither, the focused
/// window's app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transform {
    #[default]
    Normal,
    #[serde(rename = "90")]
    _90,
    #[serde(rename = "180")]
    _180,
    #[serde(rename = "270")]
    _270,
    Flipped,
    #[serde(rename = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    Flipped270,
}

/// A request to the compositor. Optional `id` fields mean the focused
/// window when left out; optional `state` fields toggle when left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    /// → [`Response::Version`].
    Version,
    /// → [`Response::Outputs`].
    Outputs,
    /// → [`Response::Windows`], bottom of the stacking order first.
    Windows,
    /// → [`Response::Desks`].
    Desks,
    /// → [`Response::Window`].
    FocusedWindow,
    /// Focuses and raises a window, switching to its Desk and showing it
    /// if it was minimised or its app hidden.
    Focus { id: WindowId },
    /// Asks a window to close (the app may ask to save first).
    Close {
        #[serde(default)]
        id: Option<WindowId>,
    },
    Minimise {
        #[serde(default)]
        id: Option<WindowId>,
    },
    Unminimise { id: WindowId },
    /// Hides every window of an app (❖H).
    HideApp {
        #[serde(default, flatten)]
        app: AppSel,
    },
    /// Hides every app but the focused one's (❖⌥H).
    HideOthers,
    ShowApp {
        #[serde(default, flatten)]
        app: AppSel,
    },
    /// Fills the usable area (the zoom box).
    Fill {
        #[serde(default)]
        id: Option<WindowId>,
        #[serde(default)]
        state: Option<bool>,
    },
    /// Takes the left or right half of the usable area.
    Split {
        #[serde(default)]
        id: Option<WindowId>,
        side: Side,
    },
    Fullscreen {
        #[serde(default)]
        id: Option<WindowId>,
        #[serde(default)]
        state: Option<bool>,
    },
    /// Keeps a window above the others.
    Float {
        #[serde(default)]
        id: Option<WindowId>,
        #[serde(default)]
        state: Option<bool>,
    },
    /// Shows a window on every Desk.
    Sticky {
        #[serde(default)]
        id: Option<WindowId>,
        #[serde(default)]
        state: Option<bool>,
    },
    /// Sends a window (and its tab group) to a Desk; with `follow`, goes
    /// there too.
    MoveToDesk {
        #[serde(default)]
        id: Option<WindowId>,
        desk: DeskSel,
        #[serde(default)]
        follow: bool,
    },
    /// Switches to a Desk.
    SetDesk { desk: DeskSel },
    AddDesk {
        name: String,
        #[serde(default)]
        folder: Option<String>,
        /// 1-based position; the end when left out.
        #[serde(default)]
        index: Option<u32>,
    },
    RenameDesk { desk: DeskSel, name: String },
    SetDeskFolder {
        desk: DeskSel,
        #[serde(default)]
        folder: Option<String>,
    },
    /// Removes a Desk; its windows move to the Desk before it.
    RemoveDesk { desk: DeskSel },
    /// Moves a Desk to another position (1-based).
    MoveDesk { desk: DeskSel, index: u32 },
    /// Stacks window `b` into window `a`'s tab group.
    Stack { a: WindowId, b: WindowId },
    /// Takes a window out of its tab group.
    Unstack {
        #[serde(default)]
        id: Option<WindowId>,
    },
    /// Moves a window's content to `x`, `y` (the frame and tab sit
    /// outside it).
    Move {
        #[serde(default)]
        id: Option<WindowId>,
        x: i32,
        y: i32,
    },
    /// Sets a window's content rectangle.
    Resize {
        #[serde(default)]
        id: Option<WindowId>,
        rect: Rect,
    },
    /// Opens or closes the Desks overview.
    Overview {
        #[serde(default)]
        state: Option<bool>,
    },
    /// Opens the switcher (it stays until Esc or a pick, since no ❖ is
    /// held), or closes it with `state: false`.
    Switcher {
        #[serde(default)]
        reverse: bool,
        #[serde(default)]
        state: Option<bool>,
    },
    /// Runs a program, by default in the current Desk's folder.
    Spawn {
        argv: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default = "yes")]
        in_desk_folder: bool,
    },
    /// Captures an output (or every output, side by side, when left out)
    /// to a PNG. Without a path, the usual Screenshots folder.
    Screenshot {
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        path: Option<String>,
        /// Only this window (with its frame and tab).
        #[serde(default)]
        window: Option<WindowId>,
        /// Only this area of the global layout.
        #[serde(default)]
        area: Option<Rect>,
        /// Also put the image on the clipboard as image/png.
        #[serde(default)]
        clipboard: bool,
    },
    /// Re-reads settings.toml and applies it.
    ReloadConfig,
    /// Changes an output for this session (Settings saves to the file).
    OutputConfig {
        name: String,
        /// `"WxH"` or `"WxH@Hz"`.
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        scale: Option<f64>,
        #[serde(default)]
        position: Option<[i32; 2]>,
        #[serde(default)]
        transform: Option<Transform>,
        #[serde(default)]
        enabled: Option<bool>,
    },
    /// → [`Response::Stats`].
    Stats {
        /// Clears the histograms after reading them.
        #[serde(default)]
        reset: bool,
    },
    /// Ends the session.
    Quit,
    PowerOffMonitors,
    PowerOnMonitors,
    /// Asks the shell to lock (sends `binding` `lock` to subscribers).
    Lock,
    /// Turns this connection into an event stream.
    Subscribe,
    /// Performs a key-binding action by name, as if its keys were pressed
    /// (e.g. `"overview"`, `"stack-with-behind"`).
    Action { action: String },
}

fn yes() -> bool {
    true
}

/// The compositor's answer to one request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum Response {
    Ok,
    Error { message: String },
    Version { version: String, protocol: u32 },
    Outputs { outputs: Vec<OutputInfo> },
    Windows { windows: Vec<WindowInfo> },
    Window { window: Option<WindowInfo> },
    Desks { desks: Vec<DeskInfo>, current: u32 },
    Stats { stats: Stats },
    Screenshot { path: String },
    Spawned { pid: u32 },
}

impl Response {
    pub fn error(message: impl Into<String>) -> Response {
        Response::Error { message: message.into() }
    }
}

/// One window, as the shell's Deskbar and the switcher see it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub app_id: String,
    pub title: String,
    pub pid: Option<i32>,
    /// 1-based Desk position; `None` when sticky.
    pub desk: Option<u32>,
    pub sticky: bool,
    /// The tab group, when stacked with others.
    pub group: Option<u64>,
    /// The members of the group in tab order (empty when not stacked).
    pub group_members: Vec<WindowId>,
    /// Whether this is the member of its group that shows.
    pub group_active: bool,
    pub focused: bool,
    pub minimised: bool,
    pub hidden: bool,
    /// Asked for attention (activation without a valid token).
    pub urgent: bool,
    pub fullscreen: bool,
    pub fill: bool,
    pub float: bool,
    /// The content rectangle (inside the frame).
    pub geometry: Rect,
    /// The output showing most of it.
    pub output: Option<String>,
    /// Draws its own title bar; Aizome adds only the tab.
    pub client_side_decorations: bool,
    /// Uses Aizome's kit (app_id `org.aizome.…`): no command-key
    /// translation.
    pub kit_app: bool,
    /// The window this one is a dialog of.
    pub parent: Option<WindowId>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeskInfo {
    /// 1-based position.
    pub index: u32,
    /// Stable identity while the compositor runs.
    pub id: u64,
    pub name: String,
    pub folder: Option<String>,
    pub windows: u32,
    pub current: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    /// Millihertz.
    pub refresh: i32,
    pub preferred: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub enabled: bool,
    pub current_mode: Option<Mode>,
    pub modes: Vec<Mode>,
    pub scale: f64,
    pub transform: Transform,
    /// Position and size in the global layout, in points.
    pub logical: Rect,
    /// The area left for windows (outside layer-shell exclusive zones).
    pub usable: Rect,
    pub physical_size_mm: (i32, i32),
    pub powered: bool,
}

/// Frame-time statistics, per output.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub renderer: String,
    pub backend: String,
    pub outputs: Vec<OutputStats>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutputStats {
    pub name: String,
    /// Frames rendered (with damage) since start or the last reset.
    pub frames_rendered: u64,
    /// Redraws that found no damage and so drew nothing.
    pub frames_skipped: u64,
    /// CPU time per rendered frame (collect, render, submit), µs.
    pub cpu_us: Percentiles,
    /// GPU time per frame, µs, when the renderer can measure it.
    pub gpu_us: Option<Percentiles>,
    /// The samples behind the percentiles (the rolling window).
    pub samples: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub mean: f64,
}

/// Something that happened, sent to subscribed connections.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    WindowOpened { window: WindowInfo },
    WindowClosed { id: WindowId },
    /// Title, app_id, desk, state, geometry or group changed.
    WindowChanged { window: WindowInfo },
    WindowFocused { id: Option<WindowId> },
    /// The current Desk changed.
    DeskChanged { index: u32, id: u64, name: String },
    /// Desks were added, removed, renamed or reordered.
    DesksChanged { desks: Vec<DeskInfo> },
    OutputsChanged { outputs: Vec<OutputInfo> },
    ConfigChanged,
    /// A key or pointer binding that belongs to the shell (`palette`,
    /// `toolchest`, `lock`, `screenshot-tool`, the media keys, `app-menu`
    /// for ❖-right-click with the pointer position, …; docs/ipc.md lists
    /// them).
    Binding {
        action: String,
        /// Pointer position in the global layout, for pointer bindings.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pointer: Option<(f64, f64)>,
        /// The output under the pointer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
    ScreenshotTaken { path: String },
    /// No input for the configured time: `lock` or `monitors-off`.
    Idle { kind: String },
    Resumed,
    LockStateChanged { locked: bool },
}

/// `$XDG_RUNTIME_DIR/aizome.$WAYLAND_DISPLAY.sock`.
pub fn socket_path_for(wayland_display: &str) -> PathBuf {
    runtime_dir().join(format!("aizome.{wayland_display}.sock"))
}

/// `$AIZOME_SOCKET`, or the path for `$WAYLAND_DISPLAY`.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AIZOME_SOCKET").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let display = std::env::var("WAYLAND_DISPLAY").ok().filter(|v| !v.is_empty())?;
    Some(socket_path_for(&display))
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// A blocking connection to the compositor.
///
/// ```no_run
/// let mut c = aizome_ipc::Client::connect()?;
/// let windows = c.request(&aizome_ipc::Request::Windows)?;
/// for event in c.events()? {
///     println!("{:?}", event?);
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    /// Connects to `$AIZOME_SOCKET` (or the socket for `$WAYLAND_DISPLAY`).
    pub fn connect() -> io::Result<Client> {
        let path = socket_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "neither AIZOME_SOCKET nor WAYLAND_DISPLAY is set")
        })?;
        Client::connect_to(&path)
    }

    pub fn connect_to(path: &Path) -> io::Result<Client> {
        let stream = UnixStream::connect(path)?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Client { writer: stream, reader })
    }

    /// Sends a request and waits for its response.
    pub fn request(&mut self, request: &Request) -> io::Result<Response> {
        let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;
        let mut buf = String::new();
        if self.reader.read_line(&mut buf)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the compositor closed the connection"));
        }
        serde_json::from_str(buf.trim_end()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Like `request`, but turns `Response::Error` into an `Err`.
    pub fn call(&mut self, request: &Request) -> io::Result<Response> {
        match self.request(request)? {
            Response::Error { message } => Err(io::Error::other(message)),
            r => Ok(r),
        }
    }

    /// Subscribes and returns the event stream. The connection carries
    /// nothing else afterwards.
    pub fn events(mut self) -> io::Result<Events> {
        match self.request(&Request::Subscribe)? {
            Response::Ok => Ok(Events { reader: self.reader, _writer: self.writer }),
            Response::Error { message } => Err(io::Error::other(message)),
            other => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected {other:?}"))),
        }
    }

    /// The underlying socket, e.g. to set a read timeout.
    pub fn stream(&self) -> &UnixStream {
        &self.writer
    }
}

/// Events from a subscribed connection. Ends when the compositor closes it.
pub struct Events {
    reader: BufReader<UnixStream>,
    _writer: UnixStream,
}

impl Events {
    pub fn stream(&self) -> &UnixStream {
        self.reader.get_ref()
    }
}

impl Iterator for Events {
    type Item = io::Result<Event>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut buf = String::new();
        match self.reader.read_line(&mut buf) {
            Ok(0) => None,
            Ok(_) => Some(
                serde_json::from_str(buf.trim_end()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            ),
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_tagged_in_kebab_case() {
        let r = Request::Focus { id: 7 };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"request":"focus","id":7}"#);
        let r: Request = serde_json::from_str(r#"{"request":"close"}"#).unwrap();
        assert_eq!(r, Request::Close { id: None });
        let r: Request = serde_json::from_str(r#"{"request":"move-to-desk","desk":2}"#).unwrap();
        assert_eq!(r, Request::MoveToDesk { id: None, desk: DeskSel::Index(2), follow: false });
        let r: Request = serde_json::from_str(r#"{"request":"set-desk","desk":"Mail"}"#).unwrap();
        assert_eq!(r, Request::SetDesk { desk: DeskSel::Name("Mail".into()) });
        let r: Request = serde_json::from_str(r#"{"request":"hide-app","app_id":"foot"}"#).unwrap();
        assert_eq!(r, Request::HideApp { app: AppSel { app_id: Some("foot".into()), pid: None } });
        let r: Request = serde_json::from_str(r#"{"request":"spawn","argv":["foot"]}"#).unwrap();
        assert_eq!(r, Request::Spawn { argv: vec!["foot".into()], cwd: None, in_desk_folder: true });
    }

    #[test]
    fn responses_and_events_round_trip() {
        let r = Response::Desks {
            desks: vec![DeskInfo { index: 1, id: 1, name: "1".into(), ..Default::default() }],
            current: 1,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&s).unwrap(), r);
        let e = Event::Binding { action: "palette".into(), pointer: None, output: None };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"event":"binding","action":"palette"}"#);
    }

    #[test]
    fn socket_path_uses_runtime_dir() {
        let p = socket_path_for("wayland-3");
        assert!(p.to_string_lossy().ends_with("aizome.wayland-3.sock"));
    }
}
