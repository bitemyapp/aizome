//! A sidestep-ui program for the compositor's tests: what Aizome's shell
//! and apps ask of a compositor, in miniature.
//!
//! `sidestep_client toplevel` opens a window that draws green, then
//! (paced by frame callbacks on its content subsurface) magenta;
//! `panel` puts a 30pt bar along the top with an exclusive zone;
//! `lock` locks the session with a lock surface per output, then unlocks
//! after a second.

use std::time::Duration;

use sidestep_ui::kurbo::Rect;
use sidestep_ui::layer_shell::{Anchors, Layer, LayerOptions};
use sidestep_ui::{App, Canvas, Color, Cx, Handler, TimerId, WindowId, WindowOptions};

pub const GREEN: Color = Color::rgb8(0x26, 0xa2, 0x69);
pub const MAGENTA: Color = Color::rgb8(0xc0, 0x26, 0x8a);
pub const BAR: Color = Color::rgb8(0x1d, 0x2b, 0x53);
pub const LOCK: Color = Color::rgb8(0x7e, 0x25, 0x53);

struct Client {
    mode: String,
    flipped: bool,
    windows: Vec<WindowId>,
    flip: Option<TimerId>,
    unlock: Option<TimerId>,
    quit: Option<TimerId>,
    tick: Option<TimerId>,
    frame: u32,
}

impl Handler for Client {
    fn launched(&mut self, cx: &mut Cx) {
        match self.mode.as_str() {
            "toplevel" => {
                let w = cx.open_window(WindowOptions::new("Sidestep test").size(320.0, 200.0));
                self.windows.push(w);
                self.flip = Some(cx.set_timer(Duration::from_millis(700)));
            }
            "animate" => {
                // A window that draws a new frame every 16ms, for measuring.
                let w = cx.open_window(WindowOptions::new("Animating").size(800.0, 500.0));
                self.windows.push(w);
                self.tick = Some(cx.set_repeating_timer(Duration::from_millis(16)));
            }
            "panel" => {
                let opts = LayerOptions::new(Layer::Top, "aizome-test-panel")
                    .anchors(Anchors::TOP | Anchors::LEFT | Anchors::RIGHT)
                    .exclusive_zone(30);
                let w = cx.open_window(WindowOptions::layer(opts).size(0.0, 30.0));
                self.windows.push(w);
            }
            "lock" => cx.lock_session(),
            m => panic!("unknown mode {m}"),
        }
    }

    fn session_locked(&mut self, cx: &mut Cx) {
        for o in cx.outputs() {
            let w = cx.open_window(WindowOptions::lock(o.id));
            self.windows.push(w);
        }
        self.unlock = Some(cx.set_timer(Duration::from_millis(1500)));
    }

    fn timer(&mut self, cx: &mut Cx, timer: TimerId) {
        if Some(timer) == self.tick {
            self.frame += 1;
            for w in self.windows.clone() {
                if let Some(mut win) = cx.window(w) {
                    win.redraw();
                }
            }
        }
        if Some(timer) == self.flip {
            self.flipped = true;
            for w in self.windows.clone() {
                if let Some(mut win) = cx.window(w) {
                    win.redraw();
                }
            }
        }
        if Some(timer) == self.unlock {
            cx.unlock_session();
            for w in std::mem::take(&mut self.windows) {
                cx.close_window(w);
            }
            // Sidestep sends the unlock with its next drawing; a window to
            // draw sends it now.
            self.mode = "toplevel".into();
            let w = cx.open_window(WindowOptions::new("After the lock").size(200.0, 100.0));
            self.windows.push(w);
            self.quit = Some(cx.set_timer(Duration::from_millis(1500)));
        }
        if Some(timer) == self.quit {
            cx.quit();
        }
    }

    fn draw(&mut self, _cx: &mut Cx, _window: WindowId, canvas: &mut Canvas) {
        let s = canvas.size();
        let color = match self.mode.as_str() {
            "panel" => BAR,
            "lock" => LOCK,
            "animate" => Color::rgb8((self.frame * 7 % 256) as u8, 0x40, (255 - self.frame * 3 % 256) as u8),
            _ if self.flipped => MAGENTA,
            _ => GREEN,
        };
        canvas.fill_rect(Rect::new(0.0, 0.0, s.width, s.height), color);
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "toplevel".into());
    let app = App::new().app_id("org.aizome.test-client").quit_when_last_window_closes(mode != "lock");
    app.run(Client { mode, flipped: false, windows: Vec::new(), flip: None, unlock: None, quit: None, tick: None, frame: 0 }).expect("run");
}
