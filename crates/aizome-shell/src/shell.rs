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
use crate::menubar::{BarModel, BarMsg, Slot, Tray};
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
            launches: HashMap::new(),
            stream: None,
            _settings_watch: None,
            _apps_watch: None,
        };
        s.read_settings();
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
            open: None,
            desks: self.wm.desks.clone(),
            renaming: self.renaming.clone(),
            tray: self.tray.clone(),
            meters: self.meters.clone(),
            clock: self.now.bar_label(self.bar_cfg.clock, self.bar_cfg.clock_seconds, self.bar_cfg.clock_date),
            folded_apps: folded,
            unread: 0,
            banner: None,
            primary,
        }
    }

    fn bar(&mut self, win: WindowKey, msg: BarMsg, cx: &mut Cx<Msg>) {
        match msg {
            BarMsg::Run(a) => self.run(a, cx),
            BarMsg::Press(slot, rect) => self.open_slot(win, slot, rect, cx),
            BarMsg::Hover(_) => {}
            BarMsg::Banner(_) => {}
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
                let menu = self.toolchest_menu();
                let anchor = Rect::new(rect.x0, rect.y0, rect.x1, crate::menubar::HEIGHT - 1.0);
                self.show_menu(win, anchor, &menu, MenuStyle::below().titled("Toolchest").min_width(rect.width()), cx);
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
            Slot::Tray(_) | Slot::Apps => {}
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
        let recent = aizome_search::recent::load(aizome_search::recent::default_path()).into_iter().map(|r| r.path).collect();
        build::toolchest(&build::ToolchestInput { apps: &apps, desks: &self.wm.desks, desk_folder, disks: Vec::new(), recent })
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
            }
            Action::Session(SessionVerb::LogOut) => self.ipc.send(Request::Quit),
            Action::Session(SessionVerb::Sleep) => {
                if let Some(l) = &self.sys.logind {
                    let _ = l.suspend();
                }
            }
            Action::Session(SessionVerb::Restart) => {
                if let Some(l) = &self.sys.logind {
                    let _ = l.reboot();
                }
            }
            Action::Session(SessionVerb::ShutDown) => {
                if let Some(l) = &self.sys.logind {
                    let _ = l.power_off();
                }
            }
            Action::Session(SessionVerb::Lock) => {}
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
            Action::About { .. } | Action::Palette { .. } | Action::Toolchest | Action::SetTheme { .. } => {}
            Action::ToggleDoNotDisturb | Action::Service(_) | Action::Disk { .. } | Action::ShelfRemove { .. } => {}
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
                    loop {
                        std::thread::sleep(Duration::from_secs(2));
                        if !tx.send(Msg::Meters(s.read())) {
                            return;
                        }
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
        let tx2 = tx.clone();
        self.sys = crate::sys::Sys::start(move || {
            tx2.send(Msg::Sys);
        });
    }

    fn take_ipc(&mut self, e: IpcEvent, cx: &mut Cx<Msg>) {
        match e {
            IpcEvent::Connected(s) => self.wm.reset(s.0),
            IpcEvent::Event(e) => {
                self.wm.apply(&e);
                match e {
                    Event::ConfigChanged => self.reload_settings(),
                    Event::Binding { action, pointer, output } => self.binding(&action, pointer, output, cx),
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
    }

    fn take_server(&mut self, e: ServerEvent, cx: &mut Cx<Msg>) {
        match e {
            ServerEvent::Message(conn, proto::AppMessage::Request { request }) => {
                let (ok, message) = self.shell_request(request, cx);
                if let Some(s) = &self.server {
                    s.send(conn, &proto::ShellMessage::Reply { ok, message });
                }
            }
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
            Msg::Sys => self.tray = self.sys.tray(&self.bar_cfg),
            Msg::Meters(r) => self.meters = r,
            Msg::Tick => {
                self.now = Local::now();
                self.arm_clock(cx);
            }
            Msg::SettingsChanged => self.reload_settings(),
            Msg::Run(a) => self.run(a, cx),
            Msg::Bar(w, m) => self.bar(w, m, cx),
            Msg::Nothing => {}
        }
    }

    fn view(&self, window: WindowKey) -> Element<Msg> {
        match self.surfaces.get(&window) {
            Some(Surface::MenuBar { output }) => {
                let Some(screen) = self.screens.iter().find(|s| s.output.id == *output) else { return aizome_kit::space(0.0) };
                crate::menubar::view(&self.bar_model(screen)).map(move |m| Msg::Bar(window, m))
            }
            Some(Surface::Desktop { .. }) => crate::widgets::drawn::drawn(0.0, 0.0, "desktop", |cx| {
                let t = cx.theme();
                let b = cx.bounds();
                cx.canvas.fill_rect(b, aizome_kit::theme::color(t.palette.desktop));
            })
            .into(),
            Some(Surface::Deskbar) | None => aizome_kit::space(0.0),
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
