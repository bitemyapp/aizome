//! The lock screen: ext-session-lock surfaces on every output — the
//! wallpaper under a `kachi` scrim, the time in IBM Plex Mono 64pt and the
//! date, and on the focused output the user's picture or initial, their
//! name, the password field (a focus rail while focused) and the mark.
//! PAM runs on a thread; a failure shakes the field and says so.

use std::sync::Arc;
use std::time::Duration;

use aizome_kit::element::Element;
use aizome_kit::layout::{Align, Point, Rect, Size};
use aizome_kit::prelude::*;
use aizome_kit::widget::SemanticRole;
use aizome_kit::{Icon, Image};
use aizome_theme::typeface;

use crate::look::{self, Hue};
use crate::pam::Authenticator;
use crate::shell::{Msg, Shell, Surface};
use crate::widgets::drawn::{around, drawn};
use crate::widgets::field::field;

/// Where locking is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Unlocked,
    /// Asked the compositor; its surfaces are open.
    Locking,
    Locked,
}

/// What the lock screen's elements say.
#[derive(Clone, Debug, PartialEq)]
pub enum LockMsg {
    Password(String),
    Submit,
    Caps(bool),
    /// PAM's answer.
    Result(Result<(), String>),
    /// A step of the shake.
    Shake(u32),
    Escape,
}

#[derive(Clone, Debug, Default)]
pub struct LockState {
    pub phase: Phase,
    /// Each output's lock surface.
    pub surfaces: Vec<(WindowKey, u32)>,
    /// The output with the password field.
    pub focused: Option<u32>,
    pub password: String,
    pub busy: bool,
    pub error: Option<String>,
    pub caps: bool,
    /// The shake's step (0: still).
    pub shake: u32,
    /// Holding logind's sleep until locked.
    pub sleep_pending: bool,
}

/// The field's sideways offset at a step of the shake.
pub fn shake_offset(step: u32) -> f64 {
    const STEPS: [f64; 9] = [0.0, -10.0, 9.0, -7.0, 6.0, -4.0, 3.0, -1.0, 0.0];
    STEPS.get(step as usize).copied().unwrap_or(0.0)
}

/// Everything one output's lock surface shows.
#[derive(Clone)]
pub struct LockView {
    pub time: String,
    pub date: String,
    /// The password and the rest show on this output.
    pub main: bool,
    pub name: String,
    pub initial: String,
    pub picture: Option<Image>,
    pub password: String,
    pub busy: bool,
    pub error: Option<String>,
    pub caps: bool,
    pub shake: f64,
    pub wallpaper: Option<Image>,
    pub size: Size,
}

const FIELD_W: f64 = 260.0;

pub fn view(v: &LockView) -> Element<LockMsg> {
    let size = v.size;
    let wall = v.wallpaper.clone();
    let ground = drawn(size.width, size.height, wall.as_ref().map(|i| i.pixel_size()), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        cx.canvas.fill_rect(b, aizome_kit::theme::color(aizome_theme::Rgba::hex(0x16161e)));
        if let Some(img) = &wall {
            let s = img.size();
            if s.width > 0.0 && s.height > 0.0 {
                let k = (b.width() / s.width).max(b.height() / s.height);
                let (cw, ch) = (b.width() / k, b.height() / k);
                let src = Rect::new((s.width - cw) / 2.0, (s.height - ch) / 2.0, (s.width + cw) / 2.0, (s.height + ch) / 2.0);
                cx.canvas.draw_image_with(img, Some(src), b, aizome_kit::sidestep_ui::ImageOptions::default());
            }
        }
        // The kachi scrim, at 70%.
        cx.canvas.fill_rect(b, aizome_kit::theme::color(aizome_theme::Rgba::hexa(0x16161e, 0.7)));
        let _ = t;
    });
    let (time, date) = (v.time.clone(), v.date.clone());
    let clock = drawn(size.width, 120.0, (time.clone(), date.clone()), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        let ts = look::digits(64.0, typeface::WEIGHT_SEMIBOLD, aizome_kit::theme::color(aizome_theme::NIGHT.text));
        cx.draw_text_in(&time, &ts, Rect::new(0.0, 0.0, b.x1, 86.0), Align::Center);
        let ds = look::ui(17.0, typeface::WEIGHT_REGULAR, aizome_kit::theme::color(aizome_theme::NIGHT.text2));
        cx.draw_text_in(&date, &ds, Rect::new(0.0, 86.0, b.x1, 116.0), Align::Center);
        let _ = t;
    })
    .text(v.time.clone(), SemanticRole::Label);
    let mut column: Vec<Element<LockMsg>> = vec![clock.into()];
    if v.main {
        let (initial, picture, name) = (v.initial.clone(), v.picture.clone(), v.name.clone());
        let who = drawn(size.width, 120.0, (name.clone(), picture.is_some()), move |cx| {
            let b = cx.bounds();
            let tile = Rect::new(((b.width() - 72.0) / 2.0).round(), 8.0, ((b.width() + 72.0) / 2.0).round(), 80.0);
            match &picture {
                Some(p) => cx.canvas.draw_image(p, tile),
                None => {
                    cx.canvas.fill_rect(tile, aizome_kit::theme::color(aizome_theme::NIGHT.selection));
                    let s = look::ui(34.0, typeface::WEIGHT_SEMIBOLD, aizome_kit::theme::color(aizome_theme::NIGHT.on_selection));
                    cx.draw_text_in(&initial, &s, tile, Align::Center);
                }
            }
            aizome_kit::draw::border(cx.canvas, tile, aizome_kit::theme::color(aizome_theme::NIGHT.bevel_lo), 1.0);
            let ns = look::ui(16.0, typeface::WEIGHT_TAB, aizome_kit::theme::color(aizome_theme::NIGHT.text));
            cx.draw_text_in(&name, &ns, Rect::new(0.0, 88.0, b.x1, 112.0), Align::Center);
        })
        .text(v.name.clone(), SemanticRole::Label);
        column.push(who.into());
        let f = field(&v.password, |_| look::ui(15.0, typeface::WEIGHT_REGULAR, aizome_kit::theme::color(aizome_theme::NIGHT.text)))
            .secure()
            .placeholder(if v.busy { "Checking…" } else { "Password" })
            .height(32.0)
            .on_change(LockMsg::Password)
            .on_submit(LockMsg::Submit)
            .on_caps(LockMsg::Caps);
        let boxed = around(Element::from(f).id("lock-password"), [0.0, 10.0, 0.0, 12.0], Size::new(FIELD_W, 34.0), ("lock-field", v.busy), |cx| {
            let n = aizome_theme::NIGHT;
            let b = cx.bounds();
            let c = aizome_kit::theme::color;
            cx.canvas.fill_rect(b, c(n.sunk));
            aizome_kit::draw::border(cx.canvas, b, c(n.line2), 1.0);
            // The focus rail while it has the keyboard.
            cx.canvas.fill_rect(Rect::new(0.0, 0.0, 2.0, b.y1), c(n.focus));
        });
        let off = v.shake;
        let row = aizome_kit::container(boxed).padding([0.0, 0.0, 0.0, ((size.width - FIELD_W) / 2.0 + off).max(0.0)]);
        column.push(row.into());
        let msg = if let Some(e) = &v.error {
            Some((e.clone(), Hue::Red))
        } else if v.caps {
            Some(("Caps Lock is on".to_owned(), Hue::Amber))
        } else {
            None
        };
        let line = drawn(size.width, 30.0, msg.as_ref().map(|(m, _)| m.clone()), move |cx| {
            if let Some((m, hue)) = &msg {
                let night = aizome_kit::Theme::night();
                let s = look::ui(13.0, typeface::WEIGHT_REGULAR, hue.color(&night));
                cx.draw_text_in(m, &s, Rect::new(0.0, 8.0, cx.bounds().x1, 28.0), Align::Center);
            }
        });
        column.push(line.into());
    }
    let block_h = if v.main { 120.0 + 120.0 + 34.0 + 30.0 } else { 120.0 };
    let top = ((size.height - block_h) / 2.0 - 40.0).max(24.0);
    let main = v.main;
    let mark = drawn(size.width, 40.0, main, move |cx| {
        if main {
            let b = cx.bounds();
            let night = aizome_kit::Theme::night();
            look::mark(cx.canvas, &night, Point::new(((b.width() - 24.0) / 2.0).round(), 8.0), 24.0);
        }
    });
    let content = aizome_kit::column(column).padding([top, 0.0, 0.0, 0.0]);
    aizome_kit::stack(vec![
        ground.into(),
        content.into(),
        aizome_kit::container(mark).padding([size.height - 72.0, 0.0, 0.0, 0.0]).into(),
    ])
    .width(size.width)
    .height(size.height)
    .into()
}

impl Shell {
    /// Lock: a lock surface on every output, the password on the focused
    /// one's.
    pub fn lock(&mut self, cx: &mut Cx<Msg>) {
        if self.lock.phase != Phase::Unlocked {
            return;
        }
        // Menus, panels and the palette go.
        self.close_palette(cx);
        self.close_panel(cx);
        for s in self.screens.clone() {
            if let Some(b) = s.bar {
                cx.close_menus(b);
            }
        }
        cx.lock_session();
        self.lock.phase = Phase::Locking;
        self.lock.password.clear();
        self.lock.error = None;
        self.lock.focused = self.focused_screen().map(|s| s.output.id);
        for s in self.screens.clone() {
            let k = cx.open_window(WindowSpec::lock(s.output.id));
            self.surfaces.insert(k, Surface::Lock { output: s.output.id });
            self.lock.surfaces.push((k, s.output.id));
        }
        cx.focus("lock-password");
    }

    pub fn lock_confirmed(&mut self, cx: &mut Cx<Msg>) {
        self.lock.phase = Phase::Locked;
        if let Some(l) = &self.sys.logind {
            let _ = l.set_locked_hint(true);
            if self.lock.sleep_pending {
                self.lock.sleep_pending = false;
                l.sleep_ready();
            }
        }
        cx.focus("lock-password");
    }

    /// The lock ended (unlocked, or the compositor refused it).
    pub fn lock_ended(&mut self) {
        for (k, _) in std::mem::take(&mut self.lock.surfaces) {
            self.surfaces.remove(&k);
        }
        self.lock.phase = Phase::Unlocked;
        self.lock.password.clear();
        self.lock.busy = false;
        self.lock.error = None;
        if let Some(l) = &self.sys.logind {
            let _ = l.set_locked_hint(false);
            if self.lock.sleep_pending {
                self.lock.sleep_pending = false;
                l.sleep_ready();
            }
        }
    }

    pub fn unlock(&mut self, cx: &mut Cx<Msg>) {
        if self.lock.phase == Phase::Unlocked {
            return;
        }
        cx.unlock_session();
        self.lock_ended();
    }

    pub fn lock_msg(&mut self, m: LockMsg, cx: &mut Cx<Msg>) {
        match m {
            LockMsg::Password(p) => {
                if !self.lock.busy {
                    self.lock.password = p;
                    self.lock.error = None;
                }
            }
            LockMsg::Caps(on) => self.lock.caps = on,
            LockMsg::Escape => {
                self.lock.password.clear();
                self.lock.error = None;
            }
            LockMsg::Submit => {
                if self.lock.busy || self.lock.password.is_empty() {
                    return;
                }
                self.lock.busy = true;
                let user = self.user.0.clone();
                let password = std::mem::take(&mut self.lock.password);
                let auth: Arc<dyn Authenticator> = self.authenticator.clone();
                cx.spawn(move || Msg::Lock(LockMsg::Result(auth.authenticate(&user, &password))));
            }
            LockMsg::Result(r) => {
                self.lock.busy = false;
                match r {
                    Ok(()) => self.unlock(cx),
                    Err(e) => {
                        log::info!("unlocking: {e}");
                        self.lock.error = Some("That password didn't work. Try again.".into());
                        if self.motion() {
                            self.lock.shake = 1;
                            cx.after(Duration::from_millis(30), Msg::Lock(LockMsg::Shake(2)));
                        }
                        cx.focus("lock-password");
                    }
                }
            }
            LockMsg::Shake(step) => {
                self.lock.shake = if step >= 9 { 0 } else { step };
                if step < 9 {
                    cx.after(Duration::from_millis(30), Msg::Lock(LockMsg::Shake(step + 1)));
                }
            }
        }
    }

    /// Whether things may move: not in Austere, not with reduced motion.
    pub fn motion(&self) -> bool {
        let a = &self.settings.appearance;
        !a.reduced_motion && a.effects != aizome_theme::Effects::Austere
    }

    pub fn lock_view(&self, output: u32) -> Element<Msg> {
        let Some(screen) = self.screens.iter().find(|s| s.output.id == output) else { return aizome_kit::space(0.0) };
        let main = self.lock.focused.unwrap_or(output) == output;
        let name = self.user.1.clone();
        let picture = self.user_picture.clone();
        let v = LockView {
            time: self.now.time_label(self.bar_cfg.clock, false),
            date: self.now.date_label(),
            main,
            initial: name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default(),
            name,
            picture,
            password: self.lock.password.clone(),
            busy: self.lock.busy,
            error: self.lock.error.clone(),
            caps: self.lock.caps,
            shake: shake_offset(self.lock.shake),
            wallpaper: self.wallpaper(),
            size: screen.output.frame.size(),
        };
        view(&v).map(Msg::Lock)
    }
}

/// The glyph a lock screen's error would use.
pub fn error_glyph() -> Icon {
    Icon::CircleAlert
}
