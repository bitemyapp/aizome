//! Picking what to screenshot (❖⇧4): drag a rubber band over an area, or
//! press Space and click a window; Esc cancels. The shell borrows it to
//! pick what to record (IPC `pick-area`): then the pick is reported as an
//! `area-picked` event instead, and a click picks the whole display under
//! the pointer.

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};
use smithay::input::keyboard::{Keysym, keysyms};
use smithay::output::Output;

use super::painter::{Box2, Painter, mono};
use crate::decor::DecorCache;
use crate::input::KeyDecision;
use crate::render::OutputElement;
use crate::state::{Aizome, State};
use crate::wm::{Rect, WinId};

pub struct AreaPick {
    pub start: Option<(f64, f64)>,
    pub pos: (f64, f64),
    /// Space: pick a window instead.
    pub window_mode: bool,
    /// Report the pick under this tag (IPC `pick-area`) rather than take a
    /// screenshot.
    pub report: Option<String>,
    pub cache: DecorCache,
}

pub fn start(state: &mut State, report: Option<String>) {
    // A picker already up for someone else gives up first.
    if let Some(old) = state.az.overlays.area.take()
        && let Some(tag) = old.report
    {
        emit(state, tag, None, None);
    }
    let pos = state.az.pointer_pos;
    state.az.overlays.area =
        Some(AreaPick { start: None, pos: (pos.x, pos.y), window_mode: false, report, cache: DecorCache::default() });
    state.az.interactive.cursor_override = Some("crosshair");
    state.update_keyboard_focus();
    state.az.queue_redraw_all();
}

/// Closes the picker; returns the tag to report its pick under, if the
/// shell asked for it.
fn finish(state: &mut State) -> Option<String> {
    let report = state.az.overlays.area.take().and_then(|a| a.report);
    state.az.interactive.cursor_override = None;
    state.update_keyboard_focus();
    state.az.queue_redraw_all();
    report
}

/// Tells subscribers what was picked (`None`: nothing, Esc).
fn emit(state: &mut State, tag: String, area: Option<Rect>, window: Option<WinId>) {
    let output = area.and_then(|r| {
        let centre = smithay::utils::Point::from((r.x as f64 + r.w as f64 / 2.0, r.y as f64 + r.h as f64 / 2.0));
        state.az.output_at(centre).map(|o| o.name())
    });
    let area = area.map(|r| aizome_ipc::Rect::new(r.x, r.y, r.w, r.h));
    state.az.ipc.emit(&aizome_ipc::Event::AreaPicked { tag, area, output, window });
}

/// A window's frame with its tab (what the picker lights up).
fn window_bounds(az: &mut Aizome, id: WinId) -> Option<Rect> {
    az.decor_layout(id).map(|l| l.bounds()).or_else(|| az.wm.get(id).map(|w| w.rect))
}

fn band(a: (f64, f64), b: (f64, f64)) -> Rect {
    let x0 = a.0.min(b.0).floor() as i32;
    let y0 = a.1.min(b.1).floor() as i32;
    let x1 = a.0.max(b.0).ceil() as i32;
    let y1 = a.1.max(b.1).ceil() as i32;
    Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
}

/// The window under the pointer, for window mode.
fn window_at(state: &mut State, x: f64, y: f64) -> Option<WinId> {
    match state.under(smithay::utils::Point::from((x, y))) {
        crate::input::Under::Surface(_, _, crate::input::Owner::Window(id)) | crate::input::Under::Decor(id, _) => {
            Some(id)
        }
        _ => None,
    }
}

pub fn pointer_button(state: &mut State, pressed: bool, x: f64, y: f64) -> bool {
    let Some(a) = state.az.overlays.area.as_mut() else { return false };
    if a.window_mode {
        if pressed {
            let w = window_at(state, x, y);
            let bounds = w.and_then(|w| window_bounds(&mut state.az, w));
            match finish(state) {
                Some(tag) => emit(state, tag, bounds, w),
                None => {
                    if let Some(w) = w
                        && let Err(e) = crate::screenshot::take(state, crate::screenshot::What::Window(w), None, true)
                    {
                        tracing::warn!("screenshot: {e}");
                    }
                }
            }
        }
        return true;
    }
    if pressed {
        a.start = Some((x, y));
        a.pos = (x, y);
    } else if let Some(s) = a.start.take() {
        let r = band(s, (x, y));
        let dragged = r.w > 2 && r.h > 2;
        // A click, for the shell, is the whole display under the pointer.
        let display = state.az.output_at(smithay::utils::Point::from((x, y))).map(|o| state.az.output_rect(&o));
        match finish(state) {
            Some(tag) => emit(state, tag, if dragged { Some(r) } else { display }, None),
            None => {
                if dragged && let Err(e) = crate::screenshot::take(state, crate::screenshot::What::Area(r), None, true)
                {
                    tracing::warn!("screenshot: {e}");
                }
            }
        }
    }
    true
}

pub fn pointer_moved(state: &mut State, x: f64, y: f64) {
    if let Some(a) = state.az.overlays.area.as_mut() {
        a.pos = (x, y);
        state.az.queue_redraw_all();
    }
}

pub fn key(state: &mut State, syms: &[Keysym], pressed: bool) -> Option<KeyDecision> {
    let a = state.az.overlays.area.as_mut()?;
    if pressed {
        if syms.iter().any(|s| s.raw() == keysyms::KEY_Escape) {
            if let Some(tag) = finish(state) {
                emit(state, tag, None, None);
            }
        } else if syms.iter().any(|s| s.raw() == keysyms::KEY_space) {
            a.window_mode = !a.window_mode;
            a.start = None;
            state.az.interactive.cursor_override = Some(if a.window_mode { "pointer" } else { "crosshair" });
            state.az.queue_redraw_all();
        }
    }
    Some(KeyDecision::Swallow)
}

pub fn elements<R>(az: &mut Aizome, renderer: &mut R, output: &Output, scale: f64) -> Vec<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let Some(mut a) = az.overlays.area.take() else { return Vec::new() };
    let geo = az.output_geometry(output);
    let origin = (geo.loc.x as f64, geo.loc.y as f64);
    let mut pt: Painter<R> = Painter::new(origin, scale, 1.0);
    let p = az.palette;
    // A whole display gets only an outline, no wash.
    let mut wash = true;
    let (rect, label) = if a.window_mode {
        let w = az.wm.shown_stacking().into_iter().rev().find(|id| {
            az.decor_layout(*id)
                .map(|l| l.bounds())
                .or_else(|| az.wm.get(*id).map(|w| w.rect))
                .is_some_and(|r| r.contains(a.pos.0, a.pos.1))
        });
        let r = w.and_then(|id| az.decor_layout(id).map(|l| l.bounds()).or_else(|| az.wm.get(id).map(|w| w.rect)));
        (r, "click a window · Space for an area · Esc".to_string())
    } else if let Some(s) = a.start {
        let r = band(s, a.pos);
        (Some(r), format!("{} × {}", r.w, r.h))
    } else if a.report.is_some() {
        // For the shell (recording), a click takes the display under the
        // pointer: show which.
        wash = false;
        let display = az.output_at(smithay::utils::Point::from(a.pos)).map(|o| az.output_rect(&o));
        (display, "drag over an area · click for the display · Space for a window · Esc".to_string())
    } else {
        (None, "drag over an area · Space for a window · Esc".to_string())
    };
    if let Some(r) = rect {
        let b = Box2::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        if wash {
            pt.fill(b, p.focus.with_alpha(0.15));
        }
        pt.outline(b, 1.0, p.focus);
    }
    let lb = Box2::new(a.pos.0 + 14.0, a.pos.1 + 14.0, 10.0, 20.0);
    if geo.to_f64().contains(smithay::utils::Point::from(a.pos)) {
        let (tw, _) = az.labels.measure(&label, crate::text::Face::Mono, 11.0);
        let chip = Box2::new(lb.x, lb.y, tw.ceil() + 12.0, 20.0);
        pt.fill(chip, p.menu_title);
        pt.text_center(az, renderer, &label, mono(11.0, p.on_menu_title), chip);
    }
    let out = pt.finish(&mut a.cache);
    az.overlays.area = Some(a);
    out
}
