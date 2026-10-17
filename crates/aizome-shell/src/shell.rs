//! The shell's program: its state, its messages, and which surface shows
//! what. Each surface's view and handling lives in its own module; this
//! one keeps them together and talks to the threads (the compositor's IPC,
//! the shell socket, system services, the meters).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aizome_config::{DeskbarSettings, LockSettings, MenubarSettings, NotificationSettings, PaletteSettings, Settings};
use aizome_ipc::{AppSel, Event, OutputInfo, Request};
use aizome_kit::element::Element;
use aizome_kit::layout::{Point, Rect};
use aizome_kit::prelude::*;
use aizome_kit::{ActivationRequest, Anchors, KeyboardInteractivity, Layer, LayerOptions, MenuStyle, MsgSender, Output};
use aizome_shell_proto as proto;
use aizome_sys::apps::AppIndex;

use crate::actions::{Action, SessionVerb};
use crate::apps::registry::{Registry, RegistryChange};
use crate::apps::server::{Server, ServerEvent};
use crate::clock::Local;
use crate::ipc::{Ipc, IpcEvent};
use crate::launch::Launch;
use crate::menubar::{BarModel, BarMsg, Slot, Tray, TrayWidget};
use crate::menus::build;
use crate::menus::model::Menu as ShellMenu;
use crate::systray::meters::Reading;
use crate::wm::Wm;

/// What the shell is told.
#[derive(Clone, Debug)]
pub enum Msg {
    Ipc(IpcEvent),
    Server(ServerEvent),
    /// The installed apps were read.
    Apps(AppsLoaded),
    /// The apps' directories changed.
    AppsChanged,
    /// A system service changed.
    Sys,
    Meters(Reading),
    /// The clock's label turned.
    Tick,
    /// The settings file changed.
    SettingsChanged,
    Run(Action),
    Bar(WindowKey, BarMsg),
    Deskbar(crate::deskbar::DeskbarMsg),
    Palette(crate::palette::PaletteEvent),
    Panel(crate::systray::panels::PanelMsg),
    /// What's playing, read on a thread.
    Media(Option<crate::systray::panels::Media>),
    /// The OSD of generation `n` has shown long enough.
    OsdHide(u64),
    Desktop(u32, crate::desktop::DesktopMsg),
    /// What's in `~/Desktop`, and whether the Trash is full.
    DesktopItems(Vec<(PathBuf, bool)>, bool),
    Lock(crate::lock::LockMsg),
    Auth(crate::auth::AuthIn),
    AuthUi(crate::auth::AuthMsg),
    Tear(crate::tearoff::TearMsg),
    Confirm(crate::confirm::ConfirmMsg),
    Card(crate::notifications::cards::CardMsg),
    /// A menu's strip was dragged: keep it as a palette.
    TearOff { app: String, path: Vec<String>, output: String, at: Point },
    /// The selected text for Services came (from the app or the primary
    /// selection), for a menu waiting on it.
    Selection(u64, Option<String>),
    Note(crate::notifications::server::NoteEvent),
    /// The bar's notification line has shown long enough.
    BannerDone(u32),
    /// Nothing: a message to wake the loop.
    Nothing,
}

/// The app index, read on a thread.
#[derive(Clone)]
pub struct AppsLoaded(pub Arc<AppIndex>);

impl std::fmt::Debug for AppsLoaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppsLoaded")
    }
}

/// What a window of the shell is.
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    MenuBar { output: u32 },
    Desktop { output: u32 },
    Deskbar,
    Palette,
    Panel,
    Osd,
    Lock { output: u32 },
    Auth,
    TearOff(usize),
    Confirm,
    Cards,
}

/// An output and the shell's surfaces on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub output: Output,
    pub bar: Option<WindowKey>,
    pub desktop: Option<WindowKey>,
}

impl Screen {
    /// The connector's name (`eDP-1`), as the compositor's IPC names it.
    pub fn name(&self) -> String {
        self.output.connector.clone().unwrap_or_else(|| self.output.name.clone())
    }
}

/// How the shell starts.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Talk to the compositor, serve the shell socket, follow system
    /// services: off in tests, which send what they want heard.
    pub live: bool,
    /// The compositor's IPC socket (else `$AIZOME_SOCKET`).
    pub ipc_socket: Option<PathBuf>,
    /// The shell socket (else `$AIZOME_SHELL_SOCKET`).
    pub shell_socket: Option<PathBuf>,
}

/// The shell.
pub struct Shell {
    pub opts: Options,
    pub settings: Settings,
    pub bar_cfg: MenubarSettings,
    pub deskbar_cfg: DeskbarSettings,
    pub notes_cfg: NotificationSettings,
    pub lock_cfg: LockSettings,
    pub palette_cfg: PaletteSettings,
    pub wm: Wm,
    pub ipc: Ipc,
    pub server: Option<Server>,
    pub registry: Registry,
    pub apps: Option<Arc<AppIndex>>,
    pub sys: crate::sys::Sys,
    pub tray: Tray,
    pub screens: Vec<Screen>,
    pub surfaces: HashMap<WindowKey, Surface>,
    pub deskbar: Option<WindowKey>,
    pub meters: Reading,
    pub now: Local,
    pub renaming: Option<(u32, String)>,
    pub shelf: crate::shelf::Shelf,
    /// Apps whose windows the person folded (true) or unfolded (false).
    pub folded: HashMap<String, bool>,
    pub notes: crate::notifications::store::Store,
    /// Apps whose every unread notification shows.
    pub notes_open: Vec<String>,
    pub note_server: Option<crate::notifications::server::NoteServer>,
    /// The notification in the bar's line now, and those waiting for it.
    pub banner: Option<u32>,
    pub banner_queue: Vec<u32>,
    pub palette: Option<crate::palette::PaletteState>,
    pub frecency: aizome_search::frecency::Frecency,
    pub file_index: Option<Arc<aizome_search::files::FileIndex>>,
    pub settings_list: Vec<crate::palette::model::SettingEntry>,
    /// The theme showing is Day.
    pub theme_day: bool,
    /// The text selected in the focused app, for Services.
    pub selection: Option<String>,
    pub panel: Option<crate::systray::glue::OpenPanel>,
    pub panel_ui: crate::systray::glue::PanelUi,
    /// Panels to show instead of the system's (tests and pictures).
    pub panels_fixture: Option<crate::systray::glue::Fixture>,
    pub media: Option<crate::systray::panels::Media>,
    pub osd: Option<crate::osd::OsdState>,
    pub desktop: crate::desktop::DesktopState,
    pub(crate) wallpapers: HashMap<PathBuf, aizome_kit::Image>,
    pub trash_full: bool,
    /// The user: login and full name.
    pub user: (String, String),
    pub(crate) drag_grab: aizome_kit::Vec2,
    /// Volumes to show when the system has none (tests and pictures).
    pub volumes_fixture: Vec<aizome_sys::disks::Volume>,
    pub lock: crate::lock::LockState,
    pub authenticator: Arc<dyn crate::pam::Authenticator>,
    pub user_picture: Option<aizome_kit::Image>,
    pub auth: Option<crate::auth::AuthState>,
    pub tearoffs: Vec<crate::tearoff::TearOff>,
    pub(crate) tear_shown: HashMap<usize, crate::tearoff::Shown>,
    /// Recently used files, read when the Toolchest opens.
    pub recent: Vec<PathBuf>,
    /// A menu at the pointer waiting for the selected text: its serial and place.
    pub(crate) services_wait: Option<(u64, f64, f64)>,
    pub(crate) serial: u64,
    pub confirm: Option<crate::confirm::Confirm>,
    /// The session verb last carried out (tests: nothing happens).
    pub session_done: Option<SessionVerb>,
    /// The critical notifications' cards, and how many.
    pub cards: Option<(WindowKey, usize)>,
    /// What the shell's own notifications' actions do.
    pub note_actions: HashMap<(u32, String), Action>,
    /// Actions to carry out once the update in hand is done.
    pub(crate) pending_actions: Vec<Action>,
    pub(crate) deskbar_size: Option<aizome_kit::Size>,
    /// Apps' own icons (PNG), by app id, for apps the shell has no glyph for.
    pub(crate) icons: HashMap<String, Option<aizome_kit::Image>>,
    pub(crate) icon_lookup: Option<aizome_sys::apps::IconLookup>,
    /// The primary bar's Desks, by where they are across it.
    pub(crate) desk_strip: Vec<(u32, Rect)>,
    pub(crate) text: aizome_kit::text::TextCache,
    launches: HashMap<ActivationRequest, (Launch, Option<PathBuf>)>,
    stream: Option<MsgSender<Msg>>,
    _settings_watch: Option<aizome_config::Watcher>,
    _apps_watch: Option<aizome_sys::apps::Watcher>,
}

impl Shell {
    pub fn new(opts: Options) -> Shell {
        let settings = if opts.live { Settings::load_or_default() } else { Settings::default() };
        let mut s = Shell {
            opts,
            bar_cfg: MenubarSettings::default(),
            deskbar_cfg: DeskbarSettings::default(),
            notes_cfg: NotificationSettings::default(),
            lock_cfg: LockSettings::default(),
            palette_cfg: PaletteSettings::default(),
            settings,
            wm: Wm::default(),
            ipc: Ipc::none(),
            server: None,
            registry: Registry::default(),
            apps: None,
            sys: crate::sys::Sys::default(),
            tray: Tray { cpu: true, memory: true, ..Tray::default() },
            screens: Vec::new(),
            surfaces: HashMap::new(),
            deskbar: None,
            meters: Reading::default(),
            now: Local::now(),
            renaming: None,
            shelf: crate::shelf::Shelf::default(),
            folded: HashMap::new(),
            notes: crate::notifications::store::Store::default(),
            notes_open: Vec::new(),
            note_server: None,
            banner: None,
            banner_queue: Vec::new(),
            palette: None,
            frecency: aizome_search::frecency::Frecency::default(),
            file_index: None,
            settings_list: Vec::new(),
            theme_day: false,
            selection: None,
            panel: None,
            panel_ui: Default::default(),
            panels_fixture: None,
            media: None,
            osd: None,
            desktop: Default::default(),
            wallpapers: HashMap::new(),
            trash_full: false,
            user: crate::pam::current_user(),
            drag_grab: aizome_kit::Vec2::ZERO,
            volumes_fixture: Vec::new(),
            lock: Default::default(),
            authenticator: Arc::new(crate::pam::Pam::new("aizome")),
            user_picture: None,
            auth: None,
            tearoffs: Vec::new(),
            tear_shown: HashMap::new(),
            recent: Vec::new(),
            services_wait: None,
            serial: 0,
            confirm: None,
            session_done: None,
            cards: None,
            note_actions: HashMap::new(),
            pending_actions: Vec::new(),
            deskbar_size: None,
            icons: HashMap::new(),
            icon_lookup: None,
            desk_strip: Vec::new(),
            text: aizome_kit::text::TextCache::default(),
            launches: HashMap::new(),
            stream: None,
            _settings_watch: None,
            _apps_watch: None,
        };
        s.read_settings();
        s.theme_day = s.settings.appearance.theme.resolve(aizome_kit::theme::local_hour()) == aizome_theme::ThemeName::Day;
        s
    }

    fn read_settings(&mut self) {
        self.bar_cfg = self.settings.menubar_settings();
        self.deskbar_cfg = self.settings.deskbar_settings();
        self.notes_cfg = self.settings.notification_settings();
        self.lock_cfg = self.settings.lock_settings();
        self.palette_cfg = self.settings.palette_settings();
    }

    /// Send `msg` to the program from a thread.
    pub fn sender(&self) -> Option<MsgSender<Msg>> {
        self.stream.clone()
    }

    // Outputs and surfaces.

    /// The output the Deskbar and banners show on: the one at the layout's
    /// origin, else the first.
    pub fn primary(&self) -> Option<&Screen> {
        self.screens.iter().find(|s| s.output.frame.x0 == 0.0 && s.output.frame.y0 == 0.0).or(self.screens.first())
    }

    /// The output with the focused window, else the primary.
    pub fn focused_screen(&self) -> Option<&Screen> {
        let name = self.wm.focused_window().and_then(|w| w.output.clone());
        name.and_then(|n| self.screens.iter().find(|s| s.name() == n)).or(self.primary())
    }

    pub fn screen_named(&self, name: &str) -> Option<&Screen> {
        self.screens.iter().find(|s| s.name() == name)
    }

    /// The screen a window of the shell is on.
    pub fn screen_of(&self, w: WindowKey) -> Option<&Screen> {
        match self.surfaces.get(&w)? {
            Surface::MenuBar { output } | Surface::Desktop { output } => self.screens.iter().find(|s| s.output.id == *output),
            _ => self.primary(),
        }
    }

    /// The compositor's description of an output (its place in the layout).
    pub fn output_info(&self, screen: &Screen) -> Option<&OutputInfo> {
        self.wm.outputs.iter().find(|o| o.name == screen.name())
    }

    /// Open or close each output's bar and desktop to match the outputs.
    pub fn sync_screens(&mut self, outputs: Vec<Output>, cx: &mut Cx<Msg>) {
        // Outputs gone: their surfaces go.
        let gone: Vec<Screen> = self.screens.iter().filter(|s| !outputs.iter().any(|o| o.id == s.output.id)).cloned().collect();
        for s in gone {
            for k in [s.bar, s.desktop].into_iter().flatten() {
                cx.close_window(k);
                self.surfaces.remove(&k);
            }
        }
        self.screens.retain(|s| outputs.iter().any(|o| o.id == s.output.id));
        for o in outputs {
            match self.screens.iter_mut().find(|s| s.output.id == o.id) {
                Some(s) => s.output = o,
                None => self.screens.push(Screen { output: o, bar: None, desktop: None }),
            }
        }
        // The Deskbar first: surfaces of one layer stack in the order they
        // open, and the bar's menus and panels show over what's under it.
        self.sync_deskbar(cx);
        for i in 0..self.screens.len() {
            let id = self.screens[i].output.id;
            if self.screens[i].bar.is_none() {
                let opts = LayerOptions::new(Layer::Top, "aizome-menubar")
                    .anchors(Anchors::TOP | Anchors::LEFT | Anchors::RIGHT)
                    .exclusive_zone(crate::menubar::HEIGHT as i32)
                    .keyboard(KeyboardInteractivity::OnDemand)
                    .output(id);
                let k = cx.open_window(layer(opts, 0.0, crate::menubar::HEIGHT, "Menu bar"));
                self.surfaces.insert(k, Surface::MenuBar { output: id });
                self.screens[i].bar = Some(k);
            }
            if self.screens[i].desktop.is_none() {
                let opts = LayerOptions::new(Layer::Background, "aizome-desktop")
                    .anchors(Anchors::ALL)
                    .exclusive_zone(-1)
                    .keyboard(KeyboardInteractivity::OnDemand)
                    .output(id);
                let k = cx.open_window(layer(opts, 0.0, 0.0, "Desktop"));
                self.surfaces.insert(k, Surface::Desktop { output: id });
                self.screens[i].desktop = Some(k);
            }
        }
    }

    // The bar.

    /// The focused app's name and menus (published, or the fallback
    /// ones), or Aizome's when no window has focus.
    pub fn focused_menus(&self) -> (String, Vec<ShellMenu>) {
        let Some(w) = self.wm.focused_window() else {
            return ("Aizome".into(), build::desktop(&self.wm.desks));
        };
        let conn = self.registry.for_window(w.pid, &w.app_id);
        let name = crate::apps::catalog::display_name(self.apps.as_deref(), conn.map(|c| c.name.as_str()), &w.app_id);
        match conn {
            Some(c) if !c.menus.is_empty() => (name, build::app_menus(&c.menus, c.pid)),
            _ => (name.clone(), build::fallback(&name, Some(w), &self.wm.desks)),
        }
    }

    pub fn bar_model(&self, screen: &Screen) -> BarModel {
        let (app_name, menus) = self.focused_menus();
        let primary = self.primary().is_some_and(|p| p.output.id == screen.output.id);
        let folded = self.deskbar_cfg.collapsed.then(|| self.wm.deskbar(&self.deskbar_cfg.pinned).iter().filter(|a| !a.windows.is_empty()).count());
        BarModel {
            app_name,
            menus: menus.iter().map(|m| m.title.clone()).collect(),
            open: self.panel.as_ref().filter(|p| Some(p.bar) == screen.bar).map(|p| Slot::Tray(p.widget)),
            desks: self.wm.desks.clone(),
            renaming: self.renaming.clone(),
            tray: self.tray.clone(),
            meters: self.meters.clone(),
            clock: self.now.bar_label(self.bar_cfg.clock, self.bar_cfg.clock_seconds, self.bar_cfg.clock_date),
            folded_apps: folded,
            unread: self.notes.unread_count(),
            banner: if primary { self.banner_view() } else { None },
            primary,
        }
    }

    fn bar(&mut self, win: WindowKey, msg: BarMsg, cx: &mut Cx<Msg>) {
        match msg {
            BarMsg::Run(a) => self.run(a, cx),
            BarMsg::Press(slot, rect) => self.open_slot(win, slot, rect, cx),
            BarMsg::Hover(_) => {}
            BarMsg::RenameText(t) => {
                if let Some((_, s)) = &mut self.renaming {
                    *s = t;
                }
            }
            BarMsg::RenameDone => {
                if let Some((index, name)) = self.renaming.take()
                    && !name.trim().is_empty()
                {
                    self.ipc.send(Request::RenameDesk { desk: aizome_ipc::DeskSel::Index(index), name: name.trim().to_owned() });
                }
            }
            BarMsg::RenameCancel => self.renaming = None,
            BarMsg::Banner(id) => {
                self.open_note(id, cx);
                self.banner_done(id, cx);
            }
            BarMsg::DeskMenu(index, at) => {
                let Some(desk) = self.wm.desk(index).cloned() else { return };
                let menu = build::desk_context(&desk, self.wm.desks.len() > 1);
                self.show_menu(win, Rect::from_origin_size(at, (1.0, 1.0)), &menu, MenuStyle::at_point(), cx);
            }
        }
    }

    /// Open what a slot of the bar opens.
    fn open_slot(&mut self, win: WindowKey, slot: Slot, rect: Rect, cx: &mut Cx<Msg>) {
        match slot {
            Slot::Toolchest => {
                if self.opts.live {
                    self.recent = aizome_search::recent::load(aizome_search::recent::default_path()).into_iter().take(12).map(|r| r.path).collect();
                }
                let menu = self.toolchest_menu();
                let anchor = Rect::new(rect.x0, rect.y0, rect.x1, crate::menubar::HEIGHT - 1.0);
                let output = self.screen_of(win).map(|s| s.name()).unwrap_or_default();
                let tear = Msg::TearOff { app: crate::tearoff::TOOLCHEST.into(), path: vec![], output, at: Point::new(rect.x0 + 12.0, crate::menubar::HEIGHT + 12.0) };
                let style = MenuStyle::below().titled("Toolchest").min_width(rect.width()).on_tear_off(tear);
                self.show_menu(win, anchor, &menu, style, cx);
            }
            Slot::Menu(i) => {
                let (_, menus) = self.focused_menus();
                let Some(menu) = menus.get(i) else { return };
                if let (Some(pid), Some(id)) = (menu.pid, menu.proto_id) {
                    self.about_to_show(pid, id);
                }
                let anchor = Rect::new(rect.x0, rect.y0, rect.x1, crate::menubar::HEIGHT - 1.0);
                self.show_menu(win, anchor, menu, MenuStyle::below(), cx);
            }
            Slot::Tray(w) => {
                if w == TrayWidget::Sound && self.opts.live {
                    cx.spawn(|| Msg::Media(crate::mpris::fetch()));
                }
                self.toggle_panel(win, w, rect, cx);
            }
            Slot::Apps => self.run(Action::SetDeskbarCollapsed(false), cx),
        }
    }

    /// Show one of the shell's menus over `win`.
    pub fn show_menu(&mut self, win: WindowKey, anchor: Rect, menu: &ShellMenu, style: MenuStyle<Msg>, cx: &mut Cx<Msg>) {
        let kit = build::to_kit(menu, &Msg::Run);
        cx.show_menu(win, anchor, kit, style);
    }

    pub fn toolchest_menu(&self) -> ShellMenu {
        let apps: Vec<&aizome_sys::apps::App> = self.apps.as_deref().map(|i| i.apps()).unwrap_or_default();
        let desk_folder = self.wm.current_desk_info().and_then(|d| d.folder.as_deref()).map(aizome_config::expand_home);
        let recent = self.recent.clone();
        let disks = self.sys.disks.as_ref().map(|d| d.snapshot().volumes()).unwrap_or_default().into_iter().filter_map(|v| Some((v.name, PathBuf::from(v.mount_point?)))).collect();
        build::toolchest(&build::ToolchestInput { apps: &apps, desks: &self.wm.desks, desk_folder, disks, recent })
    }

    /// Tell an app one of its menus is about to show.
    fn about_to_show(&self, pid: u32, menu: u32) {
        if let (Some(server), Some(conn)) = (&self.server, self.registry.by_pid(pid)) {
            server.send(conn.conn, &proto::ShellMessage::AboutToShow { menu });
        }
    }

    // Doing things.

    /// Carry out an action.
    pub fn run(&mut self, action: Action, cx: &mut Cx<Msg>) {
        match action {
            Action::None => {}
            Action::AppItem { pid, id } => match self.registry.by_pid(pid) {
                Some(c) => {
                    if let Some(s) = &self.server {
                        s.send(c.conn, &proto::ShellMessage::Activate { id });
                    }
                }
                None => log::debug!("item {id} of {pid}, which is gone"),
            },
            Action::Wm(req) => self.ipc.send(req),
            Action::QuitApp { app_id } => {
                for w in self.wm.windows.values().filter(|w| w.app_id == app_id) {
                    self.ipc.send(Request::Close { id: Some(w.id) });
                }
            }
            Action::FocusApp { app_id } => match self.wm.app_windows_by_recency(&app_id).first() {
                Some(w) => self.ipc.send(Request::Focus { id: w.id }),
                None => self.launch(Launch::App { app_id, targets: vec![] }, cx),
            },
            Action::Launch { app_id } | Action::NewWindow { app_id } => self.launch(Launch::App { app_id, targets: vec![] }, cx),
            Action::Spawn { argv } => self.launch(Launch::Argv(argv), cx),
            Action::Open { path } => {
                let l = crate::launch::open_path(self.apps.as_deref(), &path);
                self.launch(l, cx);
            }
            Action::Reveal { path } => {
                let l = crate::launch::reveal_path(self.apps.as_deref(), &path);
                self.launch(l, cx);
            }
            Action::OpenUrl { url } => self.launch(Launch::Argv(vec!["xdg-open".into(), url]), cx),
            Action::Settings { pane, anchor } => self.launch(crate::launch::settings(&pane, anchor.as_deref()), cx),
            Action::Copy { text } => cx.set_clipboard_text(&text),
            Action::NewDesk => {
                let name = (self.wm.desks.len() + 1).to_string();
                self.ipc.send(Request::AddDesk { name, folder: None, index: None });
            }
            Action::RenameDesk { index } => {
                let name = self.wm.desk(index).map(|d| d.name.clone()).unwrap_or_default();
                self.renaming = Some((index, name));
                cx.focus("desk-rename");
            }
            Action::Session(SessionVerb::Sleep) => self.do_session(SessionVerb::Sleep),
            Action::Session(v @ (SessionVerb::LogOut | SessionVerb::Restart | SessionVerb::ShutDown)) => self.confirm(v, cx),
            Action::Session(SessionVerb::Lock) => self.lock(cx),
            Action::Pin { app_id, on } => {
                self.deskbar_cfg.pinned.retain(|a| *a != app_id);
                if on {
                    self.deskbar_cfg.pinned.push(app_id);
                }
                self.save_section("deskbar");
            }
            Action::SetDeskbarCollapsed(on) => {
                self.deskbar_cfg.collapsed = on;
                self.save_section("deskbar");
            }
            Action::EmptyTrash => crate::launch::empty_trash(),
            Action::MoveToTrash { path } => {
                if let Err(e) = crate::launch::move_to_trash(&path) {
                    log::warn!("{}: {e}", path.display());
                }
            }
            Action::ShowDesktop => {
                for w in self.wm.windows.values().filter(|w| !w.minimised && w.desk == Some(self.wm.current_desk)) {
                    self.ipc.send(Request::Minimise { id: Some(w.id) });
                }
            }
            Action::Screenshot => self.ipc.send(Request::Screenshot { output: None, path: None, window: None, area: None, clipboard: false }),
            Action::About { app_id: None } => self.run(Action::Settings { pane: "about".into(), anchor: None }, cx),
            Action::About { app_id: Some(app_id) } => self.about_app(&app_id, cx),
            Action::Palette { scope, query } => self.open_palette(scope, query, cx),
            Action::Toolchest => self.open_toolchest(None, cx),
            Action::SetTheme { day } => self.set_theme(day),
            Action::ShowInMenu { trail } => self.show_in_menu(&trail, cx),
            Action::Disk { device, verb } => self.disk(device, verb, cx),
            Action::ToggleDoNotDisturb => {
                self.notes_cfg.do_not_disturb = !self.notes_cfg.do_not_disturb;
                self.save_section("notifications");
            }
            Action::ShelfRemove { index } => {
                self.shelf.remove(index);
                self.save_shelf();
            }
            Action::Service(verb) => self.run_service(verb, cx),
            Action::NewFolderOnDesktop => {
                let dir = crate::launch::desktop_dir();
                let mut path = dir.join("New Folder");
                let mut n = 2;
                while path.exists() {
                    path = dir.join(format!("New Folder {n}"));
                    n += 1;
                }
                if let Err(e) = std::fs::create_dir_all(&path) {
                    log::warn!("{}: {e}", path.display());
                }
            }
        }
    }

    /// Start something, asking first for an activation token so its window
    /// comes up with the keyboard.
    pub fn launch(&mut self, l: Launch, cx: &mut Cx<Msg>) {
        let cwd = self.wm.current_desk_info().and_then(|d| d.folder.as_deref()).map(aizome_config::expand_home);
        if !self.opts.live {
            log::info!("launch {l:?}");
            return;
        }
        let req = cx.request_activation_token(l.app_id());
        self.launches.insert(req, (l, cwd));
    }

    fn save_section(&mut self, section: &str) {
        match section {
            "deskbar" => self.settings.set_shell_section("deskbar", &self.deskbar_cfg),
            "menubar" => self.settings.set_shell_section("menubar", &self.bar_cfg),
            "notifications" => self.settings.set_shell_section("notifications", &self.notes_cfg),
            _ => {}
        }
        if self.opts.live
            && let Err(e) = self.settings.save()
        {
            log::warn!("saving the settings: {e}");
        }
    }

    // Threads.

    fn start_threads(&mut self, cx: &mut Cx<Msg>) {
        let tx = cx.stream();
        self.stream = Some(tx.clone());
        self.frecency = aizome_search::frecency::Frecency::load(aizome_search::frecency::Frecency::default_path());
        self.shelf = crate::shelf::Shelf::load(&crate::shelf::Shelf::default_dir());
        self.desktop.load_positions(&crate::desktop::DesktopState::path());
        self.tearoffs = crate::tearoff::load(&crate::tearoff::store_path());
        self.recent = aizome_search::recent::load(aizome_search::recent::default_path()).into_iter().take(12).map(|r| r.path).collect();
        self.load_wallpapers();
        self.start_palette_threads(cx);
        {
            let tx = tx.clone();
            self.ipc = Ipc::spawn(self.opts.ipc_socket.clone(), move |e| {
                tx.send(Msg::Ipc(e));
            });
        }
        {
            let tx = tx.clone();
            let path = self.opts.shell_socket.clone().unwrap_or_else(crate::apps::server::socket_path);
            match Server::listen(&path, move |e| {
                tx.send(Msg::Server(e));
            }) {
                Ok(s) => self.server = Some(s),
                Err(e) => log::warn!("the shell socket {}: {e}", path.display()),
            }
        }
        {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("apps".into())
                .spawn(move || {
                    let index = AppIndex::load();
                    tx.send(Msg::Apps(AppsLoaded(Arc::new(index))));
                })
                .ok();
        }
        {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("meters".into())
                .spawn(move || {
                    let mut s = crate::systray::meters::Sampler::new();
                    // A first reading soon, then every two seconds.
                    std::thread::sleep(Duration::from_millis(250));
                    // Only what changes the bars wakes the shell: an idle machine
                    // doesn't redraw.
                    let mut last = None;
                    loop {
                        let r = s.read();
                        let shown = (r.bar_heights(), r.hot(), r.memory_height());
                        if last.as_ref() != Some(&shown) {
                            last = Some(shown);
                            if !tx.send(Msg::Meters(r)) {
                                return;
                            }
                        }
                        std::thread::sleep(Duration::from_secs(2));
                    }
                })
                .ok();
        }
        {
            let tx = tx.clone();
            self._settings_watch = Some(aizome_config::Watcher::spawn(move || {
                tx.send(Msg::SettingsChanged);
            }));
        }
        {
            let tx = tx.clone();
            match crate::notifications::server::NoteServer::start(move |e| {
                tx.send(Msg::Note(e));
            }) {
                Ok(s) => self.note_server = Some(s),
                Err(e) => log::warn!("the notifications server: {e}"),
            }
        }
        let tx2 = tx.clone();
        self.sys = crate::sys::Sys::start(move || {
            tx2.send(Msg::Sys);
        });
        self.watch_desktop();
        self.icon_lookup = Some(aizome_sys::apps::IconLookup::from_env());
        self.start_lock();
        self.start_polkit();
    }

    fn take_ipc(&mut self, e: IpcEvent, cx: &mut Cx<Msg>) {
        match e {
            IpcEvent::Connected(s) => self.wm.reset(s.0),
            IpcEvent::Event(e) => {
                self.wm.apply(&e);
                match e {
                    Event::ConfigChanged => self.reload_settings(),
                    Event::Binding { action, pointer, output } => self.binding(&action, pointer, output, cx),
                    Event::Idle { kind } if kind == "lock" => self.lock(cx),
                    Event::ScreenshotTaken { path } => self.screenshot_taken(PathBuf::from(path), cx),
                    _ => {}
                }
            }
            IpcEvent::Disconnected => self.wm.connected = false,
            IpcEvent::Reply { .. } => {}
        }
    }

    fn binding(&mut self, action: &str, pointer: Option<(f64, f64)>, output: Option<String>, cx: &mut Cx<Msg>) {
        match action {
            "toolchest" => self.open_toolchest(output.as_deref(), cx),
            "palette" => self.toggle_palette(None, String::new(), cx),
            "lock" => self.lock(cx),
            "app-menu" => {
                if let Some((x, y)) = pointer {
                    self.pointer_menu_soon(x, y, cx);
                }
            }
            "play-pause" => crate::mpris::control(crate::systray::panels::MediaVerb::PlayPause),
            "next" => crate::mpris::control(crate::systray::panels::MediaVerb::Next),
            "previous" => crate::mpris::control(crate::systray::panels::MediaVerb::Previous),
            a if self.media_key(a, cx) => {}
            _ => log::debug!("binding {action} at {pointer:?} on {output:?}"),
        }
    }

    /// Open the Toolchest from the bar of an output (the focused one).
    pub fn open_toolchest(&mut self, output: Option<&str>, cx: &mut Cx<Msg>) {
        let screen = output.and_then(|n| self.screen_named(n)).or(self.focused_screen());
        let Some(bar) = screen.and_then(|s| s.bar) else { return };
        // The Toolchest button's place: the bar's left.
        self.open_slot(bar, Slot::Toolchest, Rect::new(0.0, 0.0, 92.0, crate::menubar::HEIGHT), cx);
    }

    fn reload_settings(&mut self) {
        if self.opts.live {
            self.settings = Settings::load_or_default();
        }
        self.read_settings();
        self.tray = self.sys.tray(&self.bar_cfg);
        if self.opts.live {
            self.load_wallpapers();
        }
        let day = self.settings.appearance.theme.resolve(aizome_kit::theme::local_hour()) == aizome_theme::ThemeName::Day;
        self.theme_changed(day);
    }

    fn take_server(&mut self, e: ServerEvent, cx: &mut Cx<Msg>) {
        match e {
            ServerEvent::Message(conn, proto::AppMessage::Request { request }) => {
                let (ok, message) = self.shell_request(request, cx);
                if let Some(s) = &self.server {
                    s.send(conn, &proto::ShellMessage::Reply { ok, message });
                }
            }
            ServerEvent::Message(_, proto::AppMessage::ServicesText { serial, text }) => self.selection_came(serial, text, cx),
            ServerEvent::Message(conn, m) => {
                if let RegistryChange::Menus(_) | RegistryChange::App(_) = self.registry.take(conn, &m) {}
            }
            ServerEvent::Gone(conn) => {
                self.registry.gone(conn);
            }
        }
    }

    /// A request from the compositor or `aizomectl`.
    pub fn shell_request(&mut self, r: proto::ShellRequest, cx: &mut Cx<Msg>) -> (bool, String) {
        match r {
            proto::ShellRequest::Toolchest => {
                self.open_toolchest(None, cx);
                (true, String::new())
            }
            proto::ShellRequest::Deskbar { collapsed } => {
                let on = collapsed.unwrap_or(!self.deskbar_cfg.collapsed);
                self.run(Action::SetDeskbarCollapsed(on), cx);
                (true, String::new())
            }
            proto::ShellRequest::State => (true, self.state_json()),
            proto::ShellRequest::Palette { scope, query } => {
                let scope = scope.as_deref().and_then(crate::palette::model::scope_named);
                if query.is_empty() && scope.is_none() {
                    self.toggle_palette(None, String::new(), cx);
                } else {
                    self.open_palette(scope, query, cx);
                }
                (true, String::new())
            }
            proto::ShellRequest::AppMenu { x, y } => {
                self.pointer_menu_soon(x, y, cx);
                (true, String::new())
            }
            proto::ShellRequest::Notify { app, summary, body } => {
                let n = crate::notifications::store::Incoming { app_name: app, summary, body, ..Default::default() };
                let id = self.notes.fresh_id();
                self.take_note(crate::notifications::server::NoteEvent::Notify(id, Box::new(n)), cx);
                (true, id.to_string())
            }
            proto::ShellRequest::Osd { kind, value } => match crate::osd::kind_named(&kind) {
                Some(k) => {
                    let muted = kind == "mute";
                    self.show_osd(crate::osd::Osd { kind: k, value: value as f32, muted }, cx);
                    (true, String::new())
                }
                None => (false, format!("no OSD for {kind}")),
            },
            proto::ShellRequest::Lock => {
                self.lock(cx);
                (true, String::new())
            }
            proto::ShellRequest::Open { what } => {
                let Some(screen) = self.primary().cloned() else { return (false, "no output".into()) };
                let Some(bar) = screen.bar else { return (false, "no bar".into()) };
                if let Some(w) = TrayWidget::named(&what) {
                    let x = screen.output.frame.width() - 60.0;
                    self.toggle_panel(bar, w, Rect::new(x, 0.0, x + 16.0, crate::menubar::HEIGHT), cx);
                    (true, String::new())
                } else if what.eq_ignore_ascii_case("toolchest") {
                    self.open_toolchest(None, cx);
                    (true, String::new())
                } else {
                    let (_, menus) = self.focused_menus();
                    match menus.iter().position(|m| m.title.eq_ignore_ascii_case(&what)) {
                        Some(_) => {
                            self.show_in_menu(&what, cx);
                            (true, String::new())
                        }
                        None => (false, format!("nothing called {what}")),
                    }
                }
            }
            proto::ShellRequest::Dismiss => {
                self.close_palette(cx);
                self.close_panel(cx);
                for s in self.screens.clone() {
                    if let Some(b) = s.bar {
                        cx.close_menus(b);
                    }
                }
                (true, String::new())
            }
            other => (false, format!("not yet: {other:?}")),
        }
    }

    /// What shows, for `aizomectl` and tests.
    pub fn state_json(&self) -> String {
        let surfaces: Vec<String> = self.surfaces.values().map(|s| format!("{s:?}")).collect();
        serde_json::json!({
            "screens": self.screens.iter().map(|s| s.name()).collect::<Vec<_>>(),
            "surfaces": surfaces,
            "focused": self.wm.focused,
            "apps": self.registry.apps().map(|a| a.app_id.clone()).collect::<Vec<_>>(),
        })
        .to_string()
    }

    fn arm_clock(&mut self, cx: &mut Cx<Msg>) {
        cx.after(crate::clock::until_next_tick(self.bar_cfg.clock_seconds), Msg::Tick);
    }
}

impl Program for Shell {
    type Msg = Msg;

    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Shell", "Aizome", "Desktop Shell")
    }

    fn launched(&mut self, cx: &mut Cx<Msg>) {
        if self.opts.live {
            self.start_threads(cx);
        }
        let outputs = cx.outputs();
        self.sync_screens(outputs, cx);
        self.arm_clock(cx);
    }

    fn update(&mut self, msg: Msg, cx: &mut Cx<Msg>) {
        match msg {
            Msg::Ipc(e) => self.take_ipc(e, cx),
            Msg::Server(e) => self.take_server(e, cx),
            Msg::Apps(AppsLoaded(index)) => {
                if self.opts.live && self._apps_watch.is_none()
                    && let Some(tx) = self.stream.clone()
                {
                    self._apps_watch = Some(index.watch(aizome_sys::Wake::new(move || {
                        tx.send(Msg::AppsChanged);
                    })));
                }
                self.apps = Some(index);
            }
            Msg::AppsChanged => {
                if let Some(tx) = self.stream.clone() {
                    std::thread::spawn(move || {
                        tx.send(Msg::Apps(AppsLoaded(Arc::new(AppIndex::load()))));
                    });
                }
            }
            Msg::Sys => {
                self.tray = self.sys.tray(&self.bar_cfg);
                self.session_events(cx);
            }
            Msg::Meters(r) => self.meters = r,
            Msg::Tick => {
                self.now = Local::now();
                self.arm_clock(cx);
            }
            Msg::SettingsChanged => self.reload_settings(),
            Msg::Run(a) => self.run(a, cx),
            Msg::Bar(w, m) => self.bar(w, m, cx),
            Msg::Deskbar(m) => self.deskbar_msg(m, cx),
            Msg::Palette(e) => self.palette_event(e, cx),
            Msg::Panel(m) => self.panel_msg(m, cx),
            Msg::Media(m) => self.media = m,
            Msg::OsdHide(g) => self.hide_osd(g, cx),
            Msg::Desktop(o, m) => self.desktop_msg(o, m, cx),
            Msg::Lock(m) => self.lock_msg(m, cx),
            Msg::Auth(e) => self.auth_in(e, cx),
            Msg::AuthUi(m) => self.auth_msg(m, cx),
            Msg::Tear(m) => self.tear_msg(m, cx),
            Msg::Confirm(m) => self.confirm_msg(m, cx),
            Msg::Card(m) => self.card_msg(m, cx),
            Msg::TearOff { app, path, output, at } => self.tear_off(app, path, output, at, cx),
            Msg::Selection(serial, text) => self.selection_came(serial, text, cx),
            Msg::DesktopItems(items, full) => {
                self.desktop.items = items;
                self.trash_full = full;
            }
            Msg::Note(e) => self.take_note(e, cx),
            Msg::BannerDone(id) => self.banner_done(id, cx),
            Msg::Nothing => {}
        }
        self.after_update(cx);
    }

    fn view(&self, window: WindowKey) -> Element<Msg> {
        match self.surfaces.get(&window) {
            Some(Surface::MenuBar { output }) => {
                let Some(screen) = self.screens.iter().find(|s| s.output.id == *output) else { return aizome_kit::space(0.0) };
                crate::menubar::view(&self.bar_model(screen)).map(move |m| Msg::Bar(window, m))
            }
            Some(Surface::Desktop { output }) => self.desktop_view(*output),
            Some(Surface::Deskbar) => crate::deskbar::view(&self.deskbar_model()).map(Msg::Deskbar),
            Some(Surface::Palette) => match (self.palette_view(), &self.palette) {
                (Some(v), Some(p)) => {
                    let inner = crate::palette::view::view(&v, p.scroll).map(|m| Msg::Palette(crate::palette::PaletteEvent::Ui(m)));
                    crate::widgets::keys::keys(inner, |k| crate::palette::key_event(k).map(Msg::Palette))
                }
                _ => aizome_kit::space(0.0),
            },
            Some(Surface::Lock { output }) => {
                let inner = self.lock_view(*output);
                crate::widgets::keys::keys(inner, |k| {
                    matches!(k.key, aizome_kit::sidestep_ui::Key::Named(aizome_kit::sidestep_ui::NamedKey::Escape)).then_some(Msg::Lock(crate::lock::LockMsg::Escape))
                })
            }
            Some(Surface::TearOff(i)) => self.tear_view(*i),
            Some(Surface::Cards) => crate::notifications::cards::view(&self.card_views()).map(Msg::Card),
            Some(Surface::Confirm) => match &self.confirm {
                Some(c) => crate::confirm::view(c.verb, c.left).map(Msg::Confirm),
                None => aizome_kit::space(0.0),
            },
            Some(Surface::Auth) => match self.auth_view() {
                Some(v) => crate::auth::view(&v).map(Msg::AuthUi),
                None => aizome_kit::space(0.0),
            },
            Some(Surface::Osd) => match &self.osd {
                Some(o) => crate::osd::view(&o.osd),
                None => aizome_kit::space(0.0),
            },
            Some(Surface::Panel) => match &self.panel {
                Some(p) => crate::systray::panels::view(&self.panel_data(p.widget)).map(Msg::Panel),
                None => aizome_kit::space(0.0),
            },
            None => aizome_kit::space(0.0),
        }
    }

    fn menus(&self) -> MenuBar<Msg> {
        // No menus of its own (it draws the bar), and no shortcuts but the
        // Edit ones its text fields take.
        let i = |label: &str, role: MenuRole, s: Shortcut| MenuItem::role_item(label, role).shortcut(s);
        MenuBar::new().with_menu(
            Menu::new("Edit")
                .item(i("Undo", MenuRole::Undo, Shortcut::cmd('Z')))
                .item(i("Redo", MenuRole::Redo, Shortcut::cmd('Z').shift()))
                .item(i("Cut", MenuRole::Cut, Shortcut::cmd('X')))
                .item(i("Copy", MenuRole::Copy, Shortcut::cmd('C')))
                .item(i("Paste", MenuRole::Paste, Shortcut::cmd('V')))
                .item(i("Select All", MenuRole::SelectAll, Shortcut::cmd('A'))),
        )
    }

    fn close_requested(&mut self, window: WindowKey, _cx: &mut Cx<Msg>) -> bool {
        if self.panel.as_ref().is_some_and(|p| p.window == window) {
            self.panel = None;
        }
        // A layer the compositor took away (its output went): forget it; it's
        // opened again when the outputs change.
        if let Some(s) = self.surfaces.remove(&window) {
            for sc in &mut self.screens {
                if sc.bar == Some(window) {
                    sc.bar = None;
                }
                if sc.desktop == Some(window) {
                    sc.desktop = None;
                }
            }
            if s == Surface::Deskbar {
                self.deskbar = None;
            }
        }
        true
    }

    fn outputs_changed(&mut self, cx: &mut Cx<Msg>) {
        let outputs = cx.outputs();
        self.sync_screens(outputs, cx);
    }

    fn activation_token(&mut self, request: ActivationRequest, token: Option<String>, _cx: &mut Cx<Msg>) {
        if let Some((l, cwd)) = self.launches.remove(&request) {
            crate::launch::start(self.apps.as_deref(), &l, token, cwd);
        }
    }

    fn quit_when_last_window_closes(&self) -> bool {
        false
    }

    fn session_locked(&mut self, cx: &mut Cx<Msg>) {
        self.lock_confirmed(cx);
    }

    fn session_lock_finished(&mut self, _cx: &mut Cx<Msg>) {
        self.lock_ended();
    }
}

/// The pointer's place as a rectangle to anchor a menu at.
pub fn at_point(p: Point) -> Rect {
    Rect::from_origin_size(p, (1.0, 1.0))
}

/// Ask the compositor what app owns a window, for `AppSel`.
pub fn app_sel(app_id: &str) -> AppSel {
    AppSel { app_id: Some(app_id.to_owned()), pid: None }
}

/// A desktop surface's window spec, titled for logs and tests.
pub fn layer(opts: LayerOptions, width: f64, height: f64, title: &str) -> WindowSpec {
    let mut s = WindowSpec::layer(opts, width, height);
    s.title = title.to_owned();
    s
}

impl Shell {
    /// What every update ends with: surfaces sized to their content, the
    /// geometry drags need.
    pub fn after_update(&mut self, cx: &mut Cx<Msg>) {
        self.find_icons();
        for a in std::mem::take(&mut self.pending_actions) {
            self.run(a, cx);
        }
        self.sync_deskbar(cx);
        self.sync_panel(cx);
        self.sync_tearoffs(cx);
        self.sync_cards(cx);
        if let Some(screen) = self.primary().cloned() {
            let model = self.bar_model(&screen);
            let theme = cx.theme().clone();
            let text = &mut self.text;
            let mut measure = |s: &str, spec: &aizome_kit::text::TextSpec| text.layout(&theme, s, spec, None).size().width;
            self.desk_strip = crate::menubar::desk_rects(&theme, &mut measure, &model, screen.output.frame.width());
        }
    }
}

impl Shell {
    /// About an app: its name, generic name and description, in the bar's
    /// line.
    fn about_app(&mut self, app_id: &str, cx: &mut Cx<Msg>) {
        let (name, _, _) = self.app_look(app_id);
        let entry = self.apps.as_deref().and_then(|i| i.match_window(app_id)).cloned();
        let body = entry
            .map(|e| [e.generic_name.unwrap_or_default(), e.comment.unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — "))
            .unwrap_or_default();
        let n = crate::notifications::store::Incoming {
            app_name: name.clone(),
            summary: name,
            body: if body.is_empty() { app_id.to_owned() } else { body },
            transient: true,
            desktop_entry: Some(app_id.to_owned()),
            ..Default::default()
        };
        let id = self.notes.fresh_id();
        self.take_note(crate::notifications::server::NoteEvent::Notify(id, Box::new(n)), cx);
    }

    /// Open the focused app's menu a command is in (its trail's first
    /// part), under the bar.
    pub fn show_in_menu(&mut self, trail: &str, cx: &mut Cx<Msg>) {
        let first = trail.split(" › ").next().unwrap_or("");
        let (_, menus) = self.focused_menus();
        let Some(i) = menus.iter().position(|m| m.title == first) else { return };
        let Some(bar) = self.focused_screen().and_then(|s| s.bar) else { return };
        let x = 92.0 + 40.0 * i as f64;
        self.open_slot(bar, Slot::Menu(i), Rect::new(x, 0.0, x + 40.0, crate::menubar::HEIGHT), cx);
    }

    /// Night or Day: written to the settings, which the compositor and every
    /// app follow.
    pub fn set_theme(&mut self, day: bool) {
        self.settings.appearance.theme = if day { aizome_config::ThemeChoice::Day } else { aizome_config::ThemeChoice::Night };
        if self.opts.live {
            if let Err(e) = self.settings.save() {
                log::warn!("saving the theme: {e}");
            }
            self.ipc.send(Request::ReloadConfig);
        }
        self.theme_changed(day);
    }

    /// The theme turned: foot instances and GTK follow (foot reloads its
    /// colours on SIGUSR1 for dark and SIGUSR2 for light).
    pub fn theme_changed(&mut self, day: bool) {
        if self.theme_day == day {
            return;
        }
        self.theme_day = day;
        if !self.opts.live {
            return;
        }
        std::thread::spawn(move || {
            let signal = if day { libc::SIGUSR2 } else { libc::SIGUSR1 };
            if let Ok(rd) = std::fs::read_dir("/proc") {
                let me = unsafe { libc::getuid() };
                for e in rd.flatten() {
                    let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
                    let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
                    let owner = std::os::unix::fs::MetadataExt::uid(&match e.metadata() {
                        Ok(m) => m,
                        Err(_) => continue,
                    });
                    if comm.trim() == "foot" && owner == me {
                        // SAFETY: a signal to the user's own foot.
                        unsafe { libc::kill(pid, signal) };
                    }
                }
            }
            let scheme = if day { "'prefer-light'" } else { "'prefer-dark'" };
            let _ = std::process::Command::new("dconf")
                .args(["write", "/org/gnome/desktop/interface/color-scheme", scheme])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }
}

impl Shell {
    /// The focused app's whole menu at a point of the global layout (NeXT's
    /// menu at the pointer), as a popup of that output's bar.
    pub fn pointer_menu(&mut self, x: f64, y: f64, cx: &mut Cx<Msg>) {
        let screen = self
            .screens
            .iter()
            .find(|s| s.output.frame.contains(Point::new(x, y)))
            .or(self.focused_screen())
            .cloned();
        let Some(screen) = screen else { return };
        let Some(bar) = screen.bar else { return };
        let (name, menus) = self.focused_menus();
        let window = self.wm.focused_window();
        let hide = Action::Wm(Request::HideApp { app: AppSel { app_id: None, pid: window.and_then(|w| w.pid) } });
        let quit = match window {
            Some(w) => {
                let conn = self.registry.for_window(w.pid, &w.app_id);
                let quit_item = menus.first().and_then(|m| m.flatten().into_iter().find(|f| f.entry.role.as_deref() == Some("quit")));
                match (conn, quit_item) {
                    (Some(_), Some(f)) => f.entry.action,
                    _ => Action::QuitApp { app_id: w.app_id.clone() },
                }
            }
            None => Action::Session(SessionVerb::LogOut),
        };
        let text = self.selection.clone();
        let menu = build::pointer_menu(&name, &menus, build::services(text.as_deref()), hide, quit);
        let at = Point::new(x - screen.output.frame.x0, y - screen.output.frame.y0);
        let app = match window {
            Some(w) => w.app_id.clone(),
            None => crate::tearoff::DESKTOP.into(),
        };
        let tear = Msg::TearOff { app, path: vec![], output: screen.name(), at };
        let style = MenuStyle::at_point().titled(name.clone()).on_tear_off(tear);
        self.show_menu(bar, at_point(at), &menu, style, cx);
    }
}

/// Whether this shell runs an Aizome session (not a test beside another
/// desktop): only then does it hold logind's sleep, serve polkit, and lock
/// for logind.
pub fn real_session() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|v| v.split(':').any(|d| d.eq_ignore_ascii_case("aizome")))
}

impl Shell {
    /// logind's session events: lock and unlock asked for, the machine
    /// about to sleep (lock first, then let it).
    fn session_events(&mut self, cx: &mut Cx<Msg>) {
        let events = match &self.sys.logind {
            Some(l) if real_session() => l.take_events(),
            _ => return,
        };
        use aizome_sys::power::SessionEvent;
        for e in events {
            match e {
                SessionEvent::Lock => self.lock(cx),
                SessionEvent::Unlock => self.unlock(cx),
                SessionEvent::PrepareForSleep => {
                    if self.lock_cfg.before_sleep && self.lock.phase == crate::lock::Phase::Unlocked {
                        self.lock.sleep_pending = true;
                        self.lock(cx);
                    } else if let Some(l) = &self.sys.logind {
                        l.sleep_ready();
                    }
                }
                SessionEvent::Resumed => {
                    // Take the delay again for the next sleep.
                    if let Some(l) = &self.sys.logind
                        && let Err(e) = l.take_sleep_delay()
                    {
                        log::warn!("logind's sleep delay: {e}");
                    }
                }
                SessionEvent::PrepareForShutdown => {}
            }
        }
    }

    /// What the lock screen needs from the system: PAM's service, the
    /// user's picture, and logind's sleep delay.
    fn start_lock(&mut self) {
        self.authenticator = Arc::new(crate::pam::Pam::new(self.lock_cfg.pam_service.clone()));
        if self.lock_cfg.show_picture {
            let home = aizome_config::home_dir();
            let candidates = [home.join(".face"), PathBuf::from(format!("/var/lib/AccountsService/icons/{}", self.user.0))];
            self.user_picture = candidates.iter().find(|p| p.is_file()).and_then(|p| aizome_kit::Image::open(p).ok());
        }
        if real_session()
            && self.lock_cfg.before_sleep
            && let Some(l) = &self.sys.logind
            && let Err(e) = l.take_sleep_delay()
        {
            log::warn!("logind's sleep delay: {e}");
        }
    }
}
