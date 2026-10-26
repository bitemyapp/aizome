//! A text field and an input method, as Telegram and ibus are: an app
//! whose text input is enabled while its window has the keyboard (a
//! focused field, as Qt's and GTK's are) and which opens a grabbing context
//! menu on a right-button press, with that press's serial; and an input
//! method that grabs the keyboard while a text input is active and lets go
//! when none is, as ibus does.

use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::text_input::zv3::client::{zwp_text_input_manager_v3, zwp_text_input_v3};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2, zwp_input_method_manager_v2, zwp_input_method_v2,
};

use super::Compositor;

const BTN_RIGHT: u32 = 0x111;

fn wait<T: Clone + std::fmt::Debug>(seen: &Mutex<T>, what: &str, f: impl Fn(&T) -> bool) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = seen.lock().unwrap().clone();
        if f(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {s:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Reads and dispatches until `quit` says so or the compositor drops the
/// connection, running `each` between rounds.
fn pump<S: 'static>(conn: &Connection, queue: &mut EventQueue<S>, state: &mut S, mut each: impl FnMut(&mut S) -> bool) {
    while each(state) {
        let _ = conn.flush();
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut pfd = libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&fd), events: libc::POLLIN, revents: 0 };
            // Safety: one valid pollfd.
            if unsafe { libc::poll(&mut pfd, 1, 20) } > 0 {
                if guard.read().is_err() {
                    break;
                }
            } else {
                drop(guard);
            }
        }
        if queue.dispatch_pending(state).is_err() {
            break;
        }
    }
}

// ---- the app -----------------------------------------------------------------

pub const APP_ID: &str = "org.example.chat";

#[derive(Clone, Debug, Default)]
pub struct FieldSeen {
    /// The surface with the keyboard: `window` or `menu`.
    pub focus: Option<&'static str>,
    /// Keys pressed (evdev codes), with the surface that had the keyboard.
    pub keys: Vec<(&'static str, u32)>,
    /// The menu is drawn.
    pub menu_shown: bool,
    /// The compositor closed the menu (popup_done).
    pub menu_dismissed: bool,
}

impl FieldSeen {
    pub fn keys_to(&self, name: &str) -> Vec<u32> {
        self.keys.iter().filter(|(n, _)| *n == name).map(|(_, k)| *k).collect()
    }
}

pub enum FieldCmd {
    /// The app closes its menu (a command chosen, or Escape).
    CloseMenu,
    Quit,
}

pub struct TextApp {
    tx: Sender<FieldCmd>,
    pub seen: Arc<Mutex<FieldSeen>>,
    join: Option<JoinHandle<()>>,
}

impl TextApp {
    /// `field_in_menu`: the menu has a text field of its own (a search
    /// field, say), focused while it shows.
    pub fn spawn(c: &Compositor, field_in_menu: bool) -> TextApp {
        let stream = UnixStream::connect(c.dir.join(&c.socket)).expect("connect to the compositor");
        let (tx, rx) = channel();
        let seen = Arc::new(Mutex::new(FieldSeen::default()));
        let seen2 = seen.clone();
        let join = std::thread::spawn(move || run_app(stream, field_in_menu, rx, seen2));
        TextApp { tx, seen, join: Some(join) }
    }

    pub fn send(&self, c: FieldCmd) {
        let _ = self.tx.send(c);
    }

    pub fn seen(&self) -> FieldSeen {
        self.seen.lock().unwrap().clone()
    }

    pub fn wait(&self, what: &str, f: impl Fn(&FieldSeen) -> bool) -> FieldSeen {
        wait(&self.seen, what, f)
    }
}

impl Drop for TextApp {
    fn drop(&mut self) {
        let _ = self.tx.send(FieldCmd::Quit);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Menu {
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    popup: xdg_popup::XdgPopup,
}

struct App {
    seen: Arc<Mutex<FieldSeen>>,
    field_in_menu: bool,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm: xdg_wm_base::XdgWmBase,
    seat: wl_seat::WlSeat,
    surface: wl_surface::WlSurface,
    xdg: xdg_surface::XdgSurface,
    text_input: zwp_text_input_v3::ZwpTextInputV3,
    menu: Option<Menu>,
    /// Where the pointer is on the window.
    pointer: (f64, f64),
}

impl App {
    fn name_of(&self, surface: &wl_surface::WlSurface) -> Option<&'static str> {
        if *surface == self.surface {
            Some("window")
        } else if self.menu.as_ref().is_some_and(|m| m.surface == *surface) {
            Some("menu")
        } else {
            None
        }
    }

    fn draw(&self, surface: &wl_surface::WlSurface, (w, h): (i32, i32), color: u32, qh: &QueueHandle<App>) {
        let len = (w * h * 4) as usize;
        // Safety: memfd_create with a static name.
        let fd = unsafe { OwnedFd::from_raw_fd(libc::memfd_create(c"aizome-text-test".as_ptr(), 0)) };
        let mut file = std::fs::File::from(fd);
        let px = [color as u8, (color >> 8) as u8, (color >> 16) as u8, 0xff];
        let data: Vec<u8> = std::iter::repeat_n(px, (w * h) as usize).flatten().collect();
        file.write_all(&data).unwrap();
        let pool = self.shm.create_pool(file.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
        pool.destroy();
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, w, h);
        surface.commit();
    }

    /// The context menu at the pointer, grabbing with the press's serial.
    fn open_menu(&mut self, serial: u32, qh: &QueueHandle<App>) {
        if self.menu.is_some() {
            return;
        }
        let surface = self.compositor.create_surface(qh, ());
        let positioner = self.wm.create_positioner(qh, ());
        positioner.set_size(200, 150);
        positioner.set_anchor_rect(self.pointer.0 as i32, self.pointer.1 as i32, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        let xdg = self.wm.get_xdg_surface(&surface, qh, ());
        let popup = xdg.get_popup(Some(&self.xdg), &positioner, qh, ());
        popup.grab(&self.seat, serial);
        positioner.destroy();
        surface.commit();
        self.menu = Some(Menu { surface, xdg, popup });
    }

    fn close_menu(&mut self) {
        if let Some(m) = self.menu.take() {
            // No leave comes for a surface the client destroyed.
            let mut seen = self.seen.lock().unwrap();
            if seen.focus == Some("menu") {
                seen.focus = None;
            }
            seen.menu_shown = false;
            m.popup.destroy();
            m.xdg.destroy();
            m.surface.destroy();
        }
    }
}

fn run_app(stream: UnixStream, field_in_menu: bool, rx: Receiver<FieldCmd>, seen: Arc<Mutex<FieldSeen>>) {
    let conn = Connection::from_socket(stream).expect("wayland connection");
    let (globals, mut queue): (_, EventQueue<App>) = registry_queue_init(&conn).expect("registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
    let seat: wl_seat::WlSeat = globals.bind(&qh, 5..=9, ()).expect("wl_seat");
    let manager: zwp_text_input_manager_v3::ZwpTextInputManagerV3 =
        globals.bind(&qh, 1..=1, ()).expect("zwp_text_input_manager_v3");
    let _keyboard = seat.get_keyboard(&qh, ());
    let _pointer = seat.get_pointer(&qh, ());
    let text_input = manager.get_text_input(&seat, &qh, ());
    let surface = compositor.create_surface(&qh, ());
    let xdg = wm.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_app_id(APP_ID.into());
    toplevel.set_title("Chat".into());
    surface.commit();
    let mut app = App {
        seen,
        field_in_menu,
        compositor,
        shm,
        wm,
        seat,
        surface,
        xdg,
        text_input,
        menu: None,
        pointer: (0.0, 0.0),
    };
    pump(&conn, &mut queue, &mut app, |app| {
        for cmd in rx.try_iter() {
            match cmd {
                FieldCmd::CloseMenu => app.close_menu(),
                FieldCmd::Quit => return false,
            }
        }
        true
    });
    app.close_menu();
    toplevel.destroy();
    app.xdg.destroy();
    app.surface.destroy();
    let _ = conn.flush();
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! ignore {
    ($s:ty: $($t:ty),*) => {$(
        impl Dispatch<$t, ()> for $s {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(App:
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
    wl_seat::WlSeat,
    xdg_positioner::XdgPositioner,
    xdg_toplevel::XdgToplevel,
    zwp_text_input_manager_v3::ZwpTextInputManagerV3
);

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        _: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        app: &mut Self,
        xs: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else { return };
        xs.ack_configure(serial);
        if *xs == app.xdg {
            app.draw(&app.surface, (400, 300), 0x2266aa, qh);
        } else if let Some(m) = app.menu.as_ref().filter(|m| m.xdg == *xs) {
            app.draw(&m.surface, (200, 150), 0xeeeeee, qh);
            app.seen.lock().unwrap().menu_shown = true;
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ()> for App {
    fn event(
        app: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_popup::Event::PopupDone = event {
            app.seen.lock().unwrap().menu_dismissed = true;
            app.close_menu();
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                let name = app.name_of(&surface);
                app.seen.lock().unwrap().focus = name;
            }
            wl_keyboard::Event::Leave { .. } => app.seen.lock().unwrap().focus = None,
            wl_keyboard::Event::Key { key, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. } => {
                let mut seen = app.seen.lock().unwrap();
                let to = seen.focus.unwrap_or_default();
                seen.keys.push((to, key));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, surface_x, surface_y, .. } if surface == app.surface => {
                app.pointer = (surface_x, surface_y);
            }
            wl_pointer::Event::Motion { surface_x, surface_y, .. } if app.menu.is_none() => {
                app.pointer = (surface_x, surface_y);
            }
            // Qt and GTK open a context menu on the press, holding it.
            wl_pointer::Event::Button {
                serial,
                button: BTN_RIGHT,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => app.open_menu(serial, qh),
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for App {
    fn event(
        app: &mut Self,
        ti: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // The field is focused while its surface has the keyboard.
            zwp_text_input_v3::Event::Enter { surface } => {
                let field = match app.name_of(&surface) {
                    Some("window") => true,
                    Some("menu") => app.field_in_menu,
                    _ => false,
                };
                if field {
                    ti.enable();
                    ti.set_content_type(
                        zwp_text_input_v3::ContentHint::None,
                        zwp_text_input_v3::ContentPurpose::Normal,
                    );
                    ti.commit();
                }
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                ti.disable();
                ti.commit();
            }
            _ => {}
        }
    }
}

// ---- the input method ----------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ImeSeen {
    /// Activated for a text input (as of the last `done`).
    pub active: bool,
    /// Times it has been activated.
    pub activations: u32,
    /// It holds a keyboard grab.
    pub grabbed: bool,
    /// Keys pressed (evdev codes) that came through its grab.
    pub keys: Vec<u32>,
}

pub struct InputMethod {
    quit: Sender<()>,
    pub seen: Arc<Mutex<ImeSeen>>,
    join: Option<JoinHandle<()>>,
}

impl InputMethod {
    /// Running once this returns: text inputs focused after it reach it.
    pub fn spawn(c: &Compositor) -> InputMethod {
        let stream = UnixStream::connect(c.dir.join(&c.socket)).expect("connect to the compositor");
        let (quit, rx) = channel();
        let (ready_tx, ready) = channel();
        let seen = Arc::new(Mutex::new(ImeSeen::default()));
        let seen2 = seen.clone();
        let join = std::thread::spawn(move || run_ime(stream, rx, ready_tx, seen2));
        ready.recv_timeout(Duration::from_secs(10)).expect("the input method came up");
        InputMethod { quit, seen, join: Some(join) }
    }

    pub fn seen(&self) -> ImeSeen {
        self.seen.lock().unwrap().clone()
    }

    pub fn wait(&self, what: &str, f: impl Fn(&ImeSeen) -> bool) -> ImeSeen {
        wait(&self.seen, what, f)
    }
}

impl Drop for InputMethod {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Ime {
    seen: Arc<Mutex<ImeSeen>>,
    input_method: zwp_input_method_v2::ZwpInputMethodV2,
    grab: Option<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2>,
    active: bool,
    pending: Option<bool>,
}

fn run_ime(stream: UnixStream, quit: Receiver<()>, ready: Sender<()>, seen: Arc<Mutex<ImeSeen>>) {
    let conn = Connection::from_socket(stream).expect("wayland connection");
    let (globals, mut queue): (_, EventQueue<Ime>) = registry_queue_init(&conn).expect("registry");
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 5..=9, ()).expect("wl_seat");
    let manager: zwp_input_method_manager_v2::ZwpInputMethodManagerV2 =
        globals.bind(&qh, 1..=1, ()).expect("zwp_input_method_manager_v2");
    let input_method = manager.get_input_method(&seat, &qh, ());
    let mut ime = Ime { seen, input_method, grab: None, active: false, pending: None };
    queue.roundtrip(&mut ime).expect("roundtrip");
    let _ = ready.send(());
    pump(&conn, &mut queue, &mut ime, |_| quit.try_recv().is_err());
    if let Some(g) = ime.grab.take() {
        g.release();
    }
    ime.input_method.destroy();
    let _ = conn.flush();
}

ignore!(Ime: wl_seat::WlSeat, zwp_input_method_manager_v2::ZwpInputMethodManagerV2);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Ime {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for Ime {
    fn event(
        ime: &mut Self,
        im: &zwp_input_method_v2::ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => ime.pending = Some(true),
            zwp_input_method_v2::Event::Deactivate => ime.pending = Some(false),
            // As ibus: the keyboard is grabbed on activation and let go on
            // deactivation, once `done` applies them.
            zwp_input_method_v2::Event::Done => {
                match ime.pending.take() {
                    Some(true) if !ime.active => {
                        ime.active = true;
                        ime.grab = Some(im.grab_keyboard(qh, ()));
                    }
                    Some(false) if ime.active => {
                        ime.active = false;
                        if let Some(g) = ime.grab.take() {
                            g.release();
                        }
                    }
                    _ => {}
                }
                let mut seen = ime.seen.lock().unwrap();
                if ime.active && !seen.active {
                    seen.activations += 1;
                }
                seen.active = ime.active;
                seen.grabbed = ime.grab.is_some();
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2, ()> for Ime {
    fn event(
        ime: &mut Self,
        _: &zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
        event: zwp_input_method_keyboard_grab_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_input_method_keyboard_grab_v2::Event::Key {
            key,
            state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            ..
        } = event
        {
            ime.seen.lock().unwrap().keys.push(key);
        }
    }
}
