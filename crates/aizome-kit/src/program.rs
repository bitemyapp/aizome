//! The program: an app's state, how messages change it, and how it looks.

use std::path::PathBuf;

use crate::cx::Cx;
use crate::element::Element;
use crate::icons::Icon;
use crate::layout::Size;
use crate::menu::MenuBar;

/// An Aizome app, Elm's way: state that [`update`](Program::update)
/// changes with messages, and a [`view`](Program::view) of each window
/// drawn from it.
///
/// ```no_run
/// use aizome_kit::prelude::*;
/// use aizome_kit::column; // std has a `column!` too: name ours
///
/// #[derive(Default)]
/// struct Counter { count: i64 }
///
/// #[derive(Clone, Debug)]
/// enum Msg { Add }
///
/// impl Program for Counter {
///     type Msg = Msg;
///     fn info(&self) -> AppInfo {
///         AppInfo::new("org.aizome.Counter", "Counter", "Counter").icon(Icon::Calculator)
///     }
///     fn launched(&mut self, cx: &mut Cx<Msg>) {
///         cx.open_window(WindowSpec::new("Counter").size(320.0, 200.0));
///     }
///     fn update(&mut self, msg: Msg, _cx: &mut Cx<Msg>) {
///         match msg { Msg::Add => self.count += 1 }
///     }
///     fn view(&self, _window: WindowKey) -> Element<Msg> {
///         column![
///             text(format!("{}", self.count)).size(28.0).tabular(),
///             button("Add").primary().on_press(Msg::Add),
///         ].spacing(12.0).padding(16.0).into()
///     }
/// }
///
/// fn main() { aizome_kit::run(Counter::default()) }
/// ```
pub trait Program: Sized + 'static {
    type Msg: Clone + Send + 'static;

    /// Who the app is: its id, names and icon.
    fn info(&self) -> AppInfo;

    /// The app started: open the first window(s).
    fn launched(&mut self, cx: &mut Cx<Self::Msg>);

    fn update(&mut self, msg: Self::Msg, cx: &mut Cx<Self::Msg>);

    /// What `window` shows now.
    fn view(&self, window: WindowKey) -> Element<Self::Msg>;

    /// The app's menus, in the global menu bar.
    fn menus(&self) -> MenuBar<Self::Msg> {
        MenuBar::standard(&self.info())
    }

    /// The user asked to close `window`: whether it closes. (The kit
    /// closes it; ask first with a sheet and close it later if not.)
    fn close_requested(&mut self, window: WindowKey, cx: &mut Cx<Self::Msg>) -> bool {
        let _ = (window, cx);
        true
    }

    /// Files to open, from the command line (or the desktop).
    fn open(&mut self, paths: Vec<PathBuf>, cx: &mut Cx<Self::Msg>) {
        let _ = (paths, cx);
    }
}

/// Who an app is.
#[derive(Clone, Debug, PartialEq)]
pub struct AppInfo {
    /// Reverse-DNS, as its desktop file is named: `org.aizome.Soroban`.
    pub app_id: String,
    /// Its own name: `Soroban`.
    pub name: String,
    /// What it is: `Calculator`.
    pub generic_name: String,
    pub icon: Option<Icon>,
    pub version: Option<String>,
}

impl AppInfo {
    pub fn new(app_id: impl Into<String>, name: impl Into<String>, generic_name: impl Into<String>) -> AppInfo {
        AppInfo {
            app_id: app_id.into(),
            name: name.into(),
            generic_name: generic_name.into(),
            icon: None,
            version: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> AppInfo {
        self.icon = Some(icon);
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> AppInfo {
        self.version = Some(version.into());
        self
    }
}

/// A window of the app (or a sheet), as long as it's open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowKey(pub(crate) u64);

impl WindowKey {
    /// A key that names no window (before the first opens).
    pub const NONE: WindowKey = WindowKey(0);
}

/// What kind of window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowKind {
    /// A document or main window.
    #[default]
    Normal,
    /// Kept over `parent` (a dialog of its own).
    Dialog { parent: WindowKey },
    /// A small floating window (a palette, an inspector).
    Utility,
}

/// What a window is asked to be: `WindowSpec::new("Soroban").size(320.0, 480.0)`.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSpec {
    pub title: String,
    pub size: Size,
    pub min_size: Option<Size>,
    pub max_size: Option<Size>,
    pub resizable: bool,
    pub kind: WindowKind,
}

impl WindowSpec {
    pub fn new(title: impl Into<String>) -> WindowSpec {
        WindowSpec {
            title: title.into(),
            size: Size::new(640.0, 480.0),
            min_size: None,
            max_size: None,
            resizable: true,
            kind: WindowKind::Normal,
        }
    }

    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = Size::new(width, height);
        self
    }

    pub fn min_size(mut self, width: f64, height: f64) -> Self {
        self.min_size = Some(Size::new(width, height));
        self
    }

    pub fn max_size(mut self, width: f64, height: f64) -> Self {
        self.max_size = Some(Size::new(width, height));
        self
    }

    pub fn resizable(mut self, on: bool) -> Self {
        self.resizable = on;
        self
    }

    pub fn kind(mut self, kind: WindowKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn dialog(self, parent: WindowKey) -> Self {
        self.kind(WindowKind::Dialog { parent })
    }

    pub fn utility(self) -> Self {
        self.kind(WindowKind::Utility)
    }
}
