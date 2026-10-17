//! Alerts, the About panel, and file panels.
//!
//! An alert is a sheet the kit builds: a title, a message and buttons,
//! the default one answering Return and the cancel one Escape. File
//! panels are Aizome's own, built from the kit's widgets: a path bar, a
//! sidebar of places, and the folder's files.

use std::marker::PhantomData;
use std::path::PathBuf;

use crate::element::{Element, MsgFn, MsgMap, RawElement};
use crate::engine::{Core, KitMsg};
use crate::layout::{Align, Length};
use crate::platform::{Platform, WinId};
use crate::program::WindowKey;
use crate::text::text;
use crate::theme::Role;
use crate::widgets::controls::button;
use crate::widgets::layout::{column, row, spacer};
use crate::Layoutable;

/// How an alert's button behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Normal,
    /// Answers Return, drawn filled.
    Default,
    /// Answers Escape.
    Cancel,
    /// Does something that can't be undone: drawn in red.
    Destructive,
}

#[derive(Clone, Debug)]
pub(crate) struct AlertButton {
    pub label: String,
    pub msg: Option<MsgFn>,
    pub kind: ButtonKind,
}

/// An alert: `Alert::new("Delete “notes.md”?", "It goes to the Trash.")
/// .destructive_button("Delete", Msg::Delete).cancel_button("Cancel")`.
pub struct Alert<Msg> {
    title: String,
    message: String,
    buttons: Vec<AlertButton>,
    _m: PhantomData<fn() -> Msg>,
}

pub(crate) struct AlertSpec {
    pub title: String,
    pub message: String,
    pub buttons: Vec<AlertButton>,
}

impl<Msg: Clone + Send + 'static> Alert<Msg> {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Alert<Msg> {
        Alert { title: title.into(), message: message.into(), buttons: Vec::new(), _m: PhantomData }
    }

    fn push(mut self, label: impl Into<String>, msg: Option<Msg>, kind: ButtonKind) -> Self {
        self.buttons.push(AlertButton { label: label.into(), msg: msg.map(MsgFn::new), kind });
        self
    }

    pub fn button(self, label: impl Into<String>, msg: Msg) -> Self {
        self.push(label, Some(msg), ButtonKind::Normal)
    }

    /// The button Return presses.
    pub fn default_button(self, label: impl Into<String>, msg: Msg) -> Self {
        self.push(label, Some(msg), ButtonKind::Default)
    }

    /// The button Escape presses; it only closes the alert.
    pub fn cancel_button(self, label: impl Into<String>) -> Self {
        self.push(label, None, ButtonKind::Cancel)
    }

    /// The cancel button, sending `msg` too.
    pub fn cancel_with(self, label: impl Into<String>, msg: Msg) -> Self {
        self.push(label, Some(msg), ButtonKind::Cancel)
    }

    pub fn destructive_button(self, label: impl Into<String>, msg: Msg) -> Self {
        self.push(label, Some(msg), ButtonKind::Destructive)
    }

    pub(crate) fn into_spec(self) -> AlertSpec {
        let mut buttons = self.buttons;
        if buttons.is_empty() {
            buttons.push(AlertButton { label: "OK".into(), msg: None, kind: ButtonKind::Default });
        }
        AlertSpec { title: self.title, message: self.message, buttons }
    }
}

pub(crate) fn show_alert(core: &mut Core, win: WinId, spec: AlertSpec) {
    let id = core.fresh_id();
    core.alerts.insert(id, crate::engine::AlertState { window: win, title: spec.title, message: spec.message, buttons: spec.buttons });
    if let Some(w) = core.wins.get_mut(&win) {
        w.alerts.push(id);
        w.dirty = true;
    }
}

/// The About panel: the app's icon, name, what it is and its version.
pub(crate) fn show_about(core: &mut Core, win: WinId) {
    let info = core.info.clone();
    let mut message = info.generic_name.clone();
    if let Some(v) = &info.version {
        message.push_str(&format!("\nVersion {v}"));
    }
    message.push_str("\nPart of Aizome 藍染");
    let spec = AlertSpec {
        title: info.name.clone(),
        message,
        buttons: vec![AlertButton { label: "OK".into(), msg: None, kind: ButtonKind::Default }],
    };
    show_alert(core, win, spec);
}

/// An alert's sheet content.
pub(crate) fn alert_element(core: &Core, id: u64) -> Option<RawElement> {
    let a = core.alerts.get(&id)?;
    let m = &core.theme.metrics;
    let mut buttons: Vec<Element<KitMsg>> = vec![spacer().into()];
    // Cancel first, the default last (at the right), as macOS orders them.
    let mut order: Vec<usize> = (0..a.buttons.len()).collect();
    order.sort_by_key(|&i| match a.buttons[i].kind {
        ButtonKind::Cancel => 0,
        ButtonKind::Normal => 1,
        ButtonKind::Destructive => 2,
        ButtonKind::Default => 3,
    });
    for i in order {
        let b = &a.buttons[i];
        let mut el = button(b.label.clone()).on_press(KitMsg::AlertChoice { alert: id, index: i }).key(i);
        el = match b.kind {
            ButtonKind::Default => el.default(),
            ButtonKind::Cancel => el.cancel(),
            ButtonKind::Destructive => el.destructive(),
            ButtonKind::Normal => el,
        };
        buttons.push(el.into());
    }
    let body: Element<KitMsg> = column(vec![
        text(a.title.clone()).size(m.title).weight(aizome_theme::typeface::WEIGHT_SEMIBOLD).wrap().into(),
        text(a.message.clone()).role(Role::Text2).wrap().into(),
        crate::widgets::layout::space(4.0),
        row(buttons).spacing(m.gap).width(Length::Fill).into(),
    ])
    .spacing(8.0)
    .padding([20.0, 20.0, 16.0, 20.0])
    .align(Align::Start)
    .width(Length::Fixed(400.0))
    .into();
    Some(body.into_raw())
}

pub(crate) fn alert_chosen(core: &mut Core, alert: u64, index: usize) {
    let Some(a) = core.alerts.remove(&alert) else { return };
    if let Some(w) = core.wins.get_mut(&a.window) {
        w.alerts.retain(|x| *x != alert);
        w.dirty = true;
    }
    if let Some(m) = a.buttons.get(index).and_then(|b| b.msg.as_ref()) {
        core.queue.push_back(m.make());
    }
}

// File panels.

/// What a file panel asks for.
#[derive(Clone, Debug, Default)]
pub struct FileDialog {
    pub title: Option<String>,
    /// The folder it opens in (else the last one, else home).
    pub directory: Option<PathBuf>,
    /// For saving: the name it suggests.
    pub file_name: Option<String>,
    /// Extensions shown (empty: all), as `("Markdown", ["md", "markdown"])`.
    pub filters: Vec<(String, Vec<String>)>,
    pub multiple: bool,
    pub directories: bool,
}

impl FileDialog {
    pub fn new() -> FileDialog {
        FileDialog::default()
    }

    pub fn title(mut self, t: impl Into<String>) -> Self {
        self.title = Some(t.into());
        self
    }

    pub fn directory(mut self, d: impl Into<PathBuf>) -> Self {
        self.directory = Some(d.into());
        self
    }

    pub fn file_name(mut self, n: impl Into<String>) -> Self {
        self.file_name = Some(n.into());
        self
    }

    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(|e| e.to_string()).collect()));
        self
    }

    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    /// Choose folders rather than files.
    pub fn directories(mut self) -> Self {
        self.directories = true;
        self
    }
}

/// A file panel's own messages.
#[derive(Clone, Debug)]
pub(crate) enum PanelMsg {
    Cancel,
}

pub(crate) struct PanelState {
    pub done: MsgMap<Option<Vec<PathBuf>>>,
}

pub(crate) fn show_panel(
    core: &mut Core,
    window: WindowKey,
    dialog: FileDialog,
    save: bool,
    done: MsgMap<Option<Vec<PathBuf>>>,
    platform: &mut dyn Platform,
) {
    let _ = (window, dialog, save, platform);
    // The native panel comes with the rest of the dialogs; until then the
    // program hears that nothing was chosen.
    core.queue.push_back(done.make(None));
}

pub(crate) fn panel_message(core: &mut Core, panel: u64, msg: PanelMsg, platform: &mut dyn Platform) {
    let _ = platform;
    match msg {
        PanelMsg::Cancel => {
            if let Some(p) = core.panels.remove(&panel) {
                core.queue.push_back(p.done.make(None));
            }
        }
    }
}
