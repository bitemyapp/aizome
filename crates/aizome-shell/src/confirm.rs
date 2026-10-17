//! Restart, Shut Down and Log Out ask first: a small dialog that counts
//! down 60 seconds, then does it (Cancel, Escape, or doing it now).

use std::time::Duration;

use aizome_kit::element::Element;
use aizome_kit::layout::{Align, Point, Rect, Size};
use aizome_kit::prelude::*;
use aizome_kit::widget::SemanticRole;
use aizome_kit::{KeyboardInteractivity, Layer, LayerOptions, button};
use aizome_theme::typeface;

use crate::actions::SessionVerb;
use crate::look;
use crate::shell::{Msg, Shell, Surface, layer};
use crate::widgets::drawn::{around, drawn};

pub const WIDTH: f64 = 380.0;
pub const HEIGHT: f64 = 150.0;
pub const SECONDS: u32 = 60;

#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmMsg {
    Now,
    Cancel,
    /// A second passed (of countdown `generation`).
    Tick(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Confirm {
    pub window: WindowKey,
    pub verb: SessionVerb,
    pub left: u32,
    pub generation: u64,
}

pub fn words(verb: SessionVerb, left: u32) -> (String, String, String) {
    let (title, what, button) = match verb {
        SessionVerb::Restart => ("Restart now?", "The computer restarts", "Restart"),
        SessionVerb::ShutDown => ("Shut down now?", "The computer shuts down", "Shut Down"),
        _ => ("Log out now?", "You'll be logged out", "Log Out"),
    };
    let s = if left == 1 { "" } else { "s" };
    (title.into(), format!("{what} in {left} second{s}. Open apps may ask to save first."), button.into())
}

pub fn view(verb: SessionVerb, left: u32) -> Element<ConfirmMsg> {
    let (title, body, act) = words(verb, left);
    let label = format!("{title} {body}");
    let text = drawn(WIDTH - 48.0, 70.0, (title.clone(), body.clone()), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        look::mark(cx.canvas, t, Point::new(0.0, 2.0), 28.0);
        let ts = look::ui(15.0 * t.metrics.factor, typeface::WEIGHT_TAB, t.text());
        cx.draw_text_in(&title, &ts, Rect::new(44.0, 0.0, b.x1, 22.0), Align::Start);
        let mut bs = look::ui(13.0 * t.metrics.factor, typeface::WEIGHT_REGULAR, t.text2());
        bs.wrap = aizome_kit::sidestep_ui::Wrap::Words;
        let l = cx.layout_text(&body, &bs, Some(b.width() - 44.0));
        cx.canvas.draw_text(&l, Point::new(44.0, 26.0));
    })
    .text(label, SemanticRole::Label);
    let buttons = aizome_kit::row(vec![
        aizome_kit::spacer().into(),
        button("Cancel").cancel().on_press(ConfirmMsg::Cancel).into(),
        button(act).default().on_press(ConfirmMsg::Now).into(),
    ])
    .spacing(8.0);
    let content = aizome_kit::column(vec![text.into(), aizome_kit::spacer().into(), buttons.into()]).padding(24.0).width(WIDTH).height(HEIGHT);
    let framed = around(content, 0.0, Size::new(WIDTH, HEIGHT), "confirm", |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        look::panel(cx.canvas, t, b);
        cx.canvas.fill_rect(Rect::new(1.0, 1.0, b.x1 - 1.0, 3.0), t.focus());
    });
    let keyed = crate::widgets::keys::focus_keys(framed, |k| match &k.key {
        aizome_kit::sidestep_ui::Key::Named(aizome_kit::sidestep_ui::NamedKey::Escape) => Some(ConfirmMsg::Cancel),
        aizome_kit::sidestep_ui::Key::Named(aizome_kit::sidestep_ui::NamedKey::Enter) => Some(ConfirmMsg::Now),
        _ => None,
    });
    aizome_kit::container(Element::from(keyed).id("confirm")).padding([0.0, 3.0, 3.0, 0.0]).into()
}

impl Shell {
    /// Ask before restarting, shutting down or logging out.
    pub fn confirm(&mut self, verb: SessionVerb, cx: &mut Cx<Msg>) {
        if let Some(c) = &mut self.confirm {
            c.verb = verb;
            c.left = SECONDS;
            return;
        }
        let Some(output) = self.focused_screen().map(|s| s.output.id) else { return };
        let opts = LayerOptions::new(Layer::Overlay, "aizome-confirm").keyboard(KeyboardInteractivity::Exclusive).output(output);
        let window = cx.open_window(layer(opts, WIDTH + 3.0, HEIGHT + 3.0, "Confirm"));
        self.surfaces.insert(window, Surface::Confirm);
        self.serial += 1;
        let generation = self.serial;
        self.confirm = Some(Confirm { window, verb, left: SECONDS, generation });
        cx.after(Duration::from_secs(1), Msg::Confirm(ConfirmMsg::Tick(generation)));
        cx.focus("confirm");
    }

    pub fn confirm_msg(&mut self, m: ConfirmMsg, cx: &mut Cx<Msg>) {
        let Some(c) = self.confirm.clone() else { return };
        match m {
            ConfirmMsg::Cancel => self.close_confirm(cx),
            ConfirmMsg::Now => {
                self.close_confirm(cx);
                self.do_session(c.verb);
            }
            ConfirmMsg::Tick(g) if g == c.generation => {
                if c.left <= 1 {
                    self.close_confirm(cx);
                    self.do_session(c.verb);
                } else {
                    if let Some(c) = &mut self.confirm {
                        c.left -= 1;
                    }
                    cx.after(Duration::from_secs(1), Msg::Confirm(ConfirmMsg::Tick(g)));
                }
            }
            ConfirmMsg::Tick(_) => {}
        }
    }

    fn close_confirm(&mut self, cx: &mut Cx<Msg>) {
        if let Some(c) = self.confirm.take() {
            cx.close_window(c.window);
            self.surfaces.remove(&c.window);
        }
    }

    /// Restart, shut down or log out, now.
    pub fn do_session(&mut self, verb: SessionVerb) {
        if !self.opts.live {
            log::info!("{verb:?}");
            self.session_done = Some(verb);
            return;
        }
        match verb {
            SessionVerb::LogOut => self.ipc.send(aizome_ipc::Request::Quit),
            SessionVerb::Restart => {
                if let Some(l) = &self.sys.logind {
                    let _ = l.reboot();
                }
            }
            SessionVerb::ShutDown => {
                if let Some(l) = &self.sys.logind {
                    let _ = l.power_off();
                }
            }
            SessionVerb::Sleep => {
                if let Some(l) = &self.sys.logind {
                    let _ = l.suspend();
                }
            }
            SessionVerb::Lock => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_count_down() {
        let (t, b, a) = words(SessionVerb::ShutDown, 60);
        assert_eq!(t, "Shut down now?");
        assert!(b.contains("in 60 seconds"));
        assert_eq!(a, "Shut Down");
        assert!(words(SessionVerb::Restart, 1).1.contains("in 1 second."));
    }
}
