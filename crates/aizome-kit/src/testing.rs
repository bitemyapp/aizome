//! Driving a program without a display: [`Harness`] runs it on a
//! stand-in platform that opens windows at the size asked for, keeps
//! timers on a clock the test moves, and keeps the clipboard; input is
//! what the test sends, and pictures come from Sidestep's offscreen
//! renderer, through the same rasterizer and text engine windows use.
//!
//! ```no_run
//! # use aizome_kit::prelude::*;
//! # #[derive(Default)] struct App; #[derive(Clone)] enum Msg {}
//! # impl Program for App { type Msg = Msg; fn info(&self) -> AppInfo { AppInfo::new("a", "A", "a") }
//! # fn launched(&mut self, cx: &mut Cx<Msg>) { cx.open_window(WindowSpec::new("A")); }
//! # fn update(&mut self, _: Msg, _: &mut Cx<Msg>) {} fn view(&self, _: WindowKey) -> Element<Msg> { text("Save").into() } }
//! use aizome_kit::testing::Harness;
//!
//! let mut h = Harness::new(App::default());
//! let w = h.window();
//! h.click_text(w, "Save");
//! h.type_text(w, "hello");
//! assert!(h.texts(w).iter().any(|t| t == "Saved"));
//! h.save_png(w, "target/app.png", 2.0);
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use sidestep_ui::{Appearance as Look, Cursor, Image, Key, KeyEvent, Modifiers, NamedKey, PointerButton, ScrollPhase};

use crate::app::Runtime;
use crate::engine::WinKind;
use crate::layout::{Point, Rect, Size, Vec2};
use crate::platform::{LayerChange, OpenKind, OpenSpec, Placement, Platform, Waker, WinId};
use crate::program::{Output, Program, WindowKey};
use crate::theme::Appearance;
use crate::widget::Semantics;

/// A window the stand-in platform opened.
#[derive(Clone, Debug)]
pub struct TestWindow {
    pub spec: OpenSpec,
    pub title: String,
}

/// The stand-in platform: what the engine asked, written down.
pub struct TestPlatform {
    pub windows: BTreeMap<WinId, TestWindow>,
    /// Damage since the test last took it, by window.
    pub damage: HashMap<WinId, Vec<Rect>>,
    timers: BTreeMap<u64, (Duration, Option<Duration>)>,
    next_token: u64,
    /// The test's clock: how long since the harness started.
    pub clock: Duration,
    pub clipboard: Option<String>,
    pub quit: bool,
    pub cursors: HashMap<WinId, Cursor>,
    pub text_input: HashMap<WinId, (bool, Option<Rect>)>,
    pub requested_sizes: HashMap<WinId, Size>,
    pub minimized: Vec<WinId>,
    /// Layer surfaces' changes, in order.
    pub layer_changes: Vec<(WinId, LayerChange)>,
    pub input_regions: HashMap<WinId, Option<Vec<Rect>>>,
    pub opaque_regions: HashMap<WinId, Option<Vec<Rect>>>,
    /// The outputs the test says there are.
    pub outputs: Vec<Output>,
    pub locked: bool,
    pub lock_asked: bool,
    pub activation_requests: Vec<(u64, Option<String>)>,
}

impl TestPlatform {
    fn new() -> TestPlatform {
        TestPlatform {
            windows: BTreeMap::new(),
            damage: HashMap::new(),
            timers: BTreeMap::new(),
            next_token: 0,
            clock: Duration::ZERO,
            clipboard: None,
            quit: false,
            cursors: HashMap::new(),
            text_input: HashMap::new(),
            requested_sizes: HashMap::new(),
            minimized: Vec::new(),
            layer_changes: Vec::new(),
            input_regions: HashMap::new(),
            opaque_regions: HashMap::new(),
            outputs: Vec::new(),
            locked: false,
            lock_asked: false,
            activation_requests: Vec::new(),
        }
    }

    /// The timer due first, if it's due by `until`.
    fn next_due(&self, until: Duration) -> Option<u64> {
        self.timers.iter().filter(|(_, (at, _))| *at <= until).min_by_key(|(t, (at, _))| (*at, **t)).map(|(t, _)| *t)
    }
}

impl Platform for TestPlatform {
    fn open_window(&mut self, id: WinId, spec: &OpenSpec) {
        self.windows.insert(id, TestWindow { spec: spec.clone(), title: spec.title.clone() });
    }

    fn close_window(&mut self, id: WinId) {
        self.windows.remove(&id);
        self.damage.remove(&id);
    }

    fn set_title(&mut self, id: WinId, title: &str) {
        if let Some(w) = self.windows.get_mut(&id) {
            w.title = title.to_owned();
        }
    }

    fn invalidate(&mut self, id: WinId, rect: Rect) {
        self.damage.entry(id).or_default().push(rect);
    }

    fn invalidate_all(&mut self, id: WinId) {
        let size = self.windows.get(&id).map_or(Size::new(4096.0, 4096.0), |w| w.spec.size);
        self.damage.entry(id).or_default().push(Rect::from_origin_size(Point::ZERO, size));
    }

    fn set_cursor(&mut self, id: WinId, cursor: Cursor) {
        self.cursors.insert(id, cursor);
    }

    fn set_text_input(&mut self, id: WinId, wanted: bool, caret: Option<Rect>) {
        self.text_input.insert(id, (wanted, caret));
    }

    fn request_size(&mut self, id: WinId, size: Size) {
        self.requested_sizes.insert(id, size);
    }

    fn minimize(&mut self, id: WinId) {
        self.minimized.push(id);
    }

    fn set_maximized(&mut self, _id: WinId, _on: bool) {}

    fn activate(&mut self, _id: WinId) {}

    fn begin_move(&mut self, _id: WinId) {}

    fn clipboard_text(&mut self) -> Option<String> {
        self.clipboard.clone()
    }

    fn set_clipboard_text(&mut self, text: &str) {
        self.clipboard = Some(text.to_owned());
    }

    fn set_timer(&mut self, after: Duration, repeat: bool) -> u64 {
        self.next_token += 1;
        self.timers.insert(self.next_token, (self.clock + after, repeat.then_some(after.max(Duration::from_millis(1)))));
        self.next_token
    }

    fn cancel_timer(&mut self, token: u64) {
        self.timers.remove(&token);
    }

    fn quit(&mut self) {
        self.quit = true;
    }

    fn desktop_dark(&self) -> bool {
        true
    }

    fn waker(&self) -> Waker {
        Arc::new(|| {})
    }

    fn set_layer(&mut self, id: WinId, change: LayerChange) {
        if let (LayerChange::Size(s), Some(w)) = (&change, self.windows.get_mut(&id)) {
            w.spec.size = *s;
        }
        self.layer_changes.push((id, change));
    }

    fn set_input_region(&mut self, id: WinId, region: Option<Vec<Rect>>) {
        self.input_regions.insert(id, region);
    }

    fn set_opaque_region(&mut self, id: WinId, region: Option<Vec<Rect>>) {
        self.opaque_regions.insert(id, region);
    }

    fn outputs(&self) -> Vec<Output> {
        self.outputs.clone()
    }

    fn lock_session(&mut self) {
        self.lock_asked = true;
    }

    fn unlock_session(&mut self) {
        self.lock_asked = false;
        self.locked = false;
    }

    fn is_session_locked(&self) -> bool {
        self.locked
    }

    fn request_activation_token(&mut self, app_id: Option<&str>) -> u64 {
        self.activation_requests.push((self.activation_requests.len() as u64 + 1, app_id.map(str::to_owned)));
        self.activation_requests.len() as u64
    }
}

/// A program running headlessly; see the module's documentation.
pub struct Harness<P: Program> {
    rt: Runtime<P>,
    pub platform: TestPlatform,
}

impl<P: Program> Harness<P> {
    /// Launch `program` in Night, with the menus published (no in-window
    /// menu bar).
    pub fn new(program: P) -> Harness<P> {
        Harness::with(program, Appearance::default(), false)
    }

    /// Launch `program` in `appearance`, with or without the in-window
    /// menu bar (what windows show where there's no Aizome shell).
    pub fn with(program: P, appearance: Appearance, menu_bar: bool) -> Harness<P> {
        crate::fonts::install();
        let mut rt = Runtime::new(program, appearance);
        rt.core.in_window_menus = menu_bar;
        let mut platform = TestPlatform::new();
        rt.launch(&mut platform, Vec::new());
        Harness { rt, platform }
    }

    pub fn program(&self) -> &P {
        &self.rt.program
    }

    pub fn program_mut(&mut self) -> &mut P {
        &mut self.rt.program
    }

    /// The program's windows, oldest first.
    pub fn windows(&self) -> Vec<WindowKey> {
        self.rt.core.app_windows().into_iter().map(|(_, k)| k).collect()
    }

    /// The first window.
    pub fn window(&self) -> WindowKey {
        self.windows().first().copied().expect("the program opened a window")
    }

    fn win(&self, key: WindowKey) -> WinId {
        self.rt.core.window_of(key).expect("an open window")
    }

    /// Popups open over `key`'s window (menus, popovers), oldest first,
    /// with where they'd show in the window's coordinates.
    pub fn popups(&self, key: WindowKey) -> Vec<(u32, Rect)> {
        let mut out = Vec::new();
        let mut stack = vec![self.win(key)];
        while let Some(w) = stack.pop() {
            let Some(win) = self.rt.core.wins.get(&w) else { continue };
            for &p in &win.popups {
                if let Some(r) = self.popup_rect(p) {
                    out.push((p, r));
                }
                stack.push(p);
            }
        }
        out.sort_by_key(|(p, _)| *p);
        out
    }

    /// Where popup `p` shows, in its top window's coordinates.
    fn popup_rect(&self, p: WinId) -> Option<Rect> {
        let spec = &self.platform.windows.get(&p)?.spec;
        let size = self.rt.core.wins.get(&p)?.size;
        let OpenKind::Popup { parent, anchor, placement, .. } = spec.kind else { return None };
        let parent_origin = match self.rt.core.wins.get(&parent).map(|w| &w.kind) {
            Some(WinKind::Popup { .. }) => self.popup_rect(parent)?.origin(),
            _ => Point::ZERO,
        };
        let a = anchor + parent_origin.to_vec2();
        let origin = match placement {
            Placement::Below => Point::new(a.x0, a.y1),
            Placement::AtPoint => Point::new(a.x0, a.y0),
            Placement::Beside => Point::new(a.x1 + 2.0, a.y0 - 5.0),
            Placement::Above => Point::new(a.x0, a.y0 - size.height),
        };
        Some(Rect::from_origin_size(origin, size))
    }

    /// Move the clock on by `d`, firing the timers that come due.
    pub fn advance(&mut self, d: Duration) {
        let until = self.platform.clock + d;
        while let Some(token) = self.platform.next_due(until) {
            let (at, repeat) = self.platform.timers[&token];
            self.platform.clock = at.max(self.platform.clock);
            match repeat {
                Some(p) => {
                    self.platform.timers.insert(token, (at + p, Some(p)));
                }
                None => {
                    self.platform.timers.remove(&token);
                }
            }
            self.rt.timer(token, &mut self.platform);
        }
        self.platform.clock = until;
    }

    /// Send the program a message, as if a widget had.
    pub fn send(&mut self, msg: P::Msg) {
        self.rt.core.queue.push_back(Box::new(msg));
        self.rt.flush(&mut self.platform);
    }

    pub fn resize(&mut self, key: WindowKey, size: Size) {
        let win = self.win(key);
        self.rt.core.resized(win, size);
        self.platform.invalidate_all(win);
        if let Some(w) = self.platform.windows.get_mut(&win) {
            w.spec.size = size;
        }
        self.rt.flush(&mut self.platform);
    }

    // Input.

    pub fn move_to(&mut self, key: WindowKey, at: Point) {
        let win = self.win(key);
        self.rt.core.pointer_move(win, at, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    pub fn press(&mut self, key: WindowKey, at: Point, button: PointerButton, clicks: u32) {
        let win = self.win(key);
        self.rt.core.pointer_move(win, at, &mut self.platform);
        self.rt.core.pointer_button(win, at, button, true, clicks, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    pub fn release(&mut self, key: WindowKey, at: Point, button: PointerButton) {
        let win = self.win(key);
        self.rt.core.pointer_button(win, at, button, false, 1, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    /// Press and release the left button at `at`.
    pub fn click(&mut self, key: WindowKey, at: Point) {
        self.press(key, at, PointerButton::Left, 1);
        self.release(key, at, PointerButton::Left);
    }

    /// Click the middle of the first element showing `text`; whether
    /// there was one.
    pub fn click_text(&mut self, key: WindowKey, text: &str) -> bool {
        match self.find(key, text) {
            Some(r) => {
                self.click(key, r.center());
                true
            }
            None => false,
        }
    }

    /// Click in popup `popup` at `at` (its own coordinates).
    pub fn click_popup(&mut self, popup: u32, at: Point) {
        self.rt.core.pointer_move(popup, at, &mut self.platform);
        self.rt.core.pointer_button(popup, at, PointerButton::Left, true, 1, &mut self.platform);
        self.rt.core.pointer_button(popup, at, PointerButton::Left, false, 1, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    /// Drag with the left button from `from` to `to`, in `steps` moves.
    pub fn drag(&mut self, key: WindowKey, from: Point, to: Point, steps: usize) {
        self.press(key, from, PointerButton::Left, 1);
        let win = self.win(key);
        for i in 1..=steps.max(1) {
            let t = i as f64 / steps.max(1) as f64;
            let p = from.lerp(to, t);
            self.rt.core.pointer_move(win, p, &mut self.platform);
            self.rt.flush(&mut self.platform);
        }
        self.release(key, to, PointerButton::Left);
    }

    pub fn scroll(&mut self, key: WindowKey, at: Point, delta: Vec2) {
        let win = self.win(key);
        self.rt.core.pointer_move(win, at, &mut self.platform);
        self.rt.core.scroll(win, at, delta, ScrollPhase::None, ScrollPhase::None, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    /// A key press (and release) in the window with the keyboard.
    pub fn key(&mut self, key: WindowKey, k: Key, mods: Modifiers) {
        let win = self.keyboard_window(key);
        let text = match &k {
            Key::Character(s) if !mods.super_key() && !mods.control() => Some(s.clone()),
            _ => None,
        };
        let press = KeyEvent { key: k.clone(), text, pressed: true, repeat: false, code: 0, modifiers: mods, composing: None };
        self.rt.core.key(win, press, &mut self.platform);
        self.rt.flush(&mut self.platform);
        let release = KeyEvent { key: k, text: None, pressed: false, repeat: false, code: 0, modifiers: mods, composing: None };
        self.rt.core.key(win, release, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    pub fn named(&mut self, key: WindowKey, k: NamedKey, mods: Modifiers) {
        self.key(key, Key::Named(k), mods);
    }

    /// ❖ and a character: `h.command(w, 'c')`.
    pub fn command(&mut self, key: WindowKey, c: char) {
        self.key(key, Key::Character(c.to_string()), Modifiers::SUPER);
    }

    /// Type `text`, a key per character.
    pub fn type_text(&mut self, key: WindowKey, text: &str) {
        for c in text.chars() {
            let mods = if c.is_uppercase() { Modifiers::SHIFT } else { Modifiers::NONE };
            match c {
                '\n' => self.named(key, NamedKey::Enter, Modifiers::NONE),
                '\t' => self.named(key, NamedKey::Tab, Modifiers::NONE),
                _ => self.key(key, Key::Character(c.to_string()), mods),
            }
        }
    }

    /// An input method committing `commit`, then composing `preedit`.
    pub fn ime(&mut self, key: WindowKey, commit: Option<&str>, preedit: &str) {
        let win = self.keyboard_window(key);
        if let Some(c) = commit {
            self.rt.core.ime(win, sidestep_ui::Ime::Commit(c.to_owned()), &mut self.platform);
        }
        let n = preedit.len();
        let pre = sidestep_ui::Ime::Preedit { text: preedit.to_owned(), selection: (n > 0).then_some(n..n) };
        self.rt.core.ime(win, pre, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    /// The window keys go to: the newest grabbing popup over `key`'s, else
    /// the window itself.
    fn keyboard_window(&self, key: WindowKey) -> WinId {
        let win = self.win(key);
        let popups = self.popups(key);
        popups
            .iter()
            .rev()
            .map(|(p, _)| *p)
            .find(|p| self.rt.core.wins.get(p).is_some_and(|w| w.has_keyboard))
            .unwrap_or(win)
    }

    /// Give the window the keyboard (or take it away).
    pub fn focus_window(&mut self, key: WindowKey, on: bool) {
        let win = self.win(key);
        self.rt.core.window_focus(win, on, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    /// The popup windows of the kit, raw: for driving menus.
    pub fn popup_keyboard(&mut self, popup: u32, k: Key, mods: Modifiers) {
        let press = KeyEvent { key: k, text: None, pressed: true, repeat: false, code: 0, modifiers: mods, composing: None };
        self.rt.core.key(popup, press, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    // Looking.

    /// Where the first element showing exactly `text` is.
    pub fn find(&self, key: WindowKey, text: &str) -> Option<Rect> {
        let w = self.rt.core.wins.get(&self.win(key))?;
        w.tree.nodes.iter().find(|n| !n.dead && n.widget.text() == Some(text) && n.clip.intersect(n.rect).area() > 0.0).map(|n| n.rect)
    }

    /// Where the element given `.id(name)` is.
    pub fn find_id(&self, key: WindowKey, name: &str) -> Option<Rect> {
        let w = self.rt.core.wins.get(&self.win(key))?;
        let id = w.tree.names.get(name)?;
        w.tree.get(*id).map(|i| w.tree.nodes[i].rect)
    }

    /// Every piece of text showing, in tree order.
    pub fn texts(&self, key: WindowKey) -> Vec<String> {
        self.texts_in(self.win(key))
    }

    /// The text showing in popup `popup` (a menu's items aren't
    /// elements: its semantics give its title).
    pub fn texts_in(&self, win: WinId) -> Vec<String> {
        let Some(w) = self.rt.core.wins.get(&win) else { return Vec::new() };
        w.tree.nodes.iter().filter(|n| !n.dead).filter_map(|n| n.widget.text().map(str::to_owned)).collect()
    }

    /// What assistive technology would be told, element by element.
    pub fn semantics(&self, key: WindowKey) -> Vec<(Semantics, Rect)> {
        let Some(w) = self.rt.core.wins.get(&self.win(key)) else { return Vec::new() };
        crate::a11y::nodes(&w.tree)
    }

    /// The text selected in the focused widget.
    pub fn selection(&self, key: WindowKey) -> Option<String> {
        let w = self.rt.core.wins.get(&self.win(key))?;
        let i = w.tree.get(w.focused?)?;
        let n = &w.tree.nodes[i];
        n.widget.selection(w.states.get(&n.id).map(|b| &**b))
    }

    /// The name (`.id`) of the focused element, if it has one.
    pub fn focused_name(&self, key: WindowKey) -> Option<String> {
        let w = self.rt.core.wins.get(&self.win(key))?;
        let f = w.focused?;
        w.tree.names.iter().find(|(_, id)| **id == f).map(|(n, _)| n.to_string())
    }

    /// The focused element's rectangle.
    pub fn focused_rect(&self, key: WindowKey) -> Option<Rect> {
        let w = self.rt.core.wins.get(&self.win(key))?;
        w.tree.get(w.focused?).map(|i| w.tree.nodes[i].rect)
    }

    /// The window's title.
    pub fn title(&self, key: WindowKey) -> String {
        self.platform.windows.get(&self.win(key)).map(|w| w.title.clone()).unwrap_or_default()
    }

    /// Damage since the last call, for `key`'s window.
    pub fn take_damage(&mut self, key: WindowKey) -> Vec<Rect> {
        let win = self.win(key);
        self.platform.damage.remove(&win).unwrap_or_default()
    }

    /// The window's size.
    pub fn size(&self, key: WindowKey) -> Size {
        self.rt.core.wins.get(&self.win(key)).map_or(Size::ZERO, |w| w.size)
    }

    pub fn stats(&self) -> &crate::stats::Stats {
        &self.rt.core.stats
    }

    pub fn reset_stats(&mut self) {
        self.rt.core.stats.clear();
    }

    pub fn theme(&self) -> &crate::theme::Theme {
        &self.rt.core.theme
    }

    /// The compositor describing `outputs` (the program hears
    /// `outputs_changed`).
    pub fn set_outputs(&mut self, outputs: Vec<Output>) {
        self.platform.outputs = outputs;
        self.rt.outputs_changed(&mut self.platform);
    }

    /// The compositor confirming the lock the program asked for.
    pub fn confirm_lock(&mut self) {
        if self.platform.lock_asked {
            self.platform.locked = true;
            self.rt.session_locked(&mut self.platform);
        }
    }

    /// The compositor refusing (or ending) the lock.
    pub fn finish_lock(&mut self) {
        self.platform.lock_asked = false;
        self.platform.locked = false;
        self.rt.session_lock_finished(&mut self.platform);
    }

    /// An activation token arriving for request `n` (counting from 1).
    pub fn activation_token(&mut self, n: u64, token: Option<&str>) {
        self.rt.activation_token(n, token.map(str::to_owned), &mut self.platform);
    }

    /// Kit windows by their engine id (for popups' pictures).
    pub fn render_popup(&mut self, popup: u32, scale: f64) -> Image {
        self.render_win(popup, scale)
    }

    /// Change the appearance, as Settings would.
    pub fn set_appearance(&mut self, a: Appearance) {
        self.rt.core.retheme(a, &mut self.platform);
        self.rt.flush(&mut self.platform);
    }

    // Pictures.

    fn look(&self) -> Look {
        Look { dark: self.rt.core.theme.is_dark(), high_contrast: false }
    }

    /// Draw window `key` whole, at `scale`.
    pub fn render(&mut self, key: WindowKey, scale: f64) -> Image {
        let win = self.win(key);
        self.render_win(win, scale)
    }

    fn render_win(&mut self, win: WinId, scale: f64) -> Image {
        let size = self.rt.core.wins.get(&win).map_or(Size::new(1.0, 1.0), |w| w.size);
        let look = self.look();
        let core = &mut self.rt.core;
        sidestep_ui::offscreen::render(size, scale, look, |c| core.paint(win, c))
    }

    /// Draw only `rect` of the window (as a frame redrawing that damage
    /// would): what a frame costs.
    pub fn render_rect(&mut self, key: WindowKey, rect: Rect, scale: f64) -> Image {
        let win = self.win(key);
        let look = self.look();
        let core = &mut self.rt.core;
        sidestep_ui::offscreen::render(rect.size(), scale, look, |c| {
            c.translate(-rect.origin().to_vec2());
            core.paint(win, c);
        })
    }

    /// The window with its popups composited where they'd show, as RGBA.
    pub fn render_rgba(&mut self, key: WindowKey, scale: f64) -> (u32, u32, Vec<u8>) {
        let image = self.render(key, scale);
        let (w, h) = image.pixel_size();
        let mut px = image.to_rgba8().expect("pixels").to_vec();
        for (p, r) in self.popups(key) {
            let pop = self.render_win(p, scale);
            let (pw, ph) = pop.pixel_size();
            let src = pop.to_rgba8().expect("pixels");
            let (ox, oy) = ((r.x0 * scale).round() as i64, (r.y0 * scale).round() as i64);
            for y in 0..ph as i64 {
                for x in 0..pw as i64 {
                    let (dx, dy) = (ox + x, oy + y);
                    if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
                        continue;
                    }
                    let s = &src[((y * pw as i64 + x) * 4) as usize..][..4];
                    let d = &mut px[((dy * w as i64 + dx) * 4) as usize..][..4];
                    blend(d, s);
                }
            }
        }
        (w, h, px)
    }

    /// Write the window (and its popups) to a PNG at `scale`.
    pub fn save_png(&mut self, key: WindowKey, path: impl AsRef<Path>, scale: f64) {
        let (w, h, px) = self.render_rgba(key, scale);
        write_png(path.as_ref(), w, h, &px).expect("the PNG written");
    }
}

/// `s` over `d`, straight alpha.
fn blend(d: &mut [u8], s: &[u8]) {
    let sa = f32::from(s[3]) / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = f32::from(d[3]) / 255.0;
    let oa = sa + da * (1.0 - sa);
    for i in 0..3 {
        let c = (f32::from(s[i]) * sa + f32::from(d[i]) * da * (1.0 - sa)) / oa.max(1e-6);
        d[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    d[3] = (oa * 255.0).round() as u8;
}

/// Write straight RGBA pixels to a PNG.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(std::io::Error::other)?;
    w.write_image_data(rgba).map_err(std::io::Error::other)?;
    Ok(())
}

/// Read a PNG's straight RGBA pixels.
pub fn read_png(path: &Path) -> std::io::Result<(u32, u32, Vec<u8>)> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = dec.read_info().map_err(std::io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(std::io::Error::other)?;
    buf.truncate(info.buffer_size());
    let px = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return Err(std::io::Error::other("an RGB or RGBA PNG")),
    };
    Ok((info.width, info.height, px))
}

/// How two pictures differ: the share of pixels off by more than
/// `tolerance` in any channel.
pub fn difference(a: &[u8], b: &[u8], tolerance: u8) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let off = a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tolerance)).count();
    off as f64 / (a.len() / 4) as f64
}

/// Compare the window's picture to the golden image at `path` (scale
/// given); with `AIZOME_UPDATE_GOLDENS=1` (or no golden yet) write it
/// instead. Panics if more than 0.5% of the pixels differ, writing what
/// was drawn beside the golden as `*.actual.png`.
pub fn assert_golden<P: Program>(h: &mut Harness<P>, key: WindowKey, path: impl AsRef<Path>, scale: f64) {
    let path = path.as_ref();
    let (w, hgt, px) = h.render_rgba(key, scale);
    let update = std::env::var_os("AIZOME_UPDATE_GOLDENS").is_some_and(|v| v != "0");
    if update || !path.exists() {
        write_png(path, w, hgt, &px).expect("the golden written");
        return;
    }
    let (gw, gh, golden) = read_png(path).expect("the golden read");
    let diff = if (gw, gh) == (w, hgt) { difference(&px, &golden, 24) } else { 1.0 };
    if diff > 0.005 {
        let actual = path.with_extension("actual.png");
        let _ = write_png(&actual, w, hgt, &px);
        panic!("{} differs from what's drawn in {:.2}% of its pixels (see {})", path.display(), diff * 100.0, actual.display());
    }
}
