//! Picking what to screenshot (❖⇧4): drag a rubber band over an area, or
//! press Space and click a window; Esc cancels.

use smithay::input::keyboard::{Keysym, keysyms};
use smithay::input::pointer::CursorIcon;
use smithay::output::Output;

use super::painter::{Box2, Painter, mono};
use crate::decor::DecorCache;
use crate::prelude::*;
use crate::wm::{Rect, WinId};
use aizome_comp_core::logic::KeyDecision;

pub struct AreaPick {
    pub start: Option<(f64, f64)>,
    pub pos: (f64, f64),
    /// Space: pick a window instead.
    pub window_mode: bool,
    pub cache: DecorCache,
}

pub fn start(state: &mut State) {
    let pos = state.az.pointer_pos;
    state.lg.m().overlays.area =
        Some(AreaPick { start: None, pos: (pos.x, pos.y), window_mode: false, cache: DecorCache::default() });
    state.az.cursor_override = Some(CursorIcon::Crosshair);
    state.update_keyboard_focus();
    state.az.queue_redraw_all();
}

fn finish(state: &mut State) {
    state.lg.m().overlays.area = None;
    state.az.cursor_override = None;
    state.update_keyboard_focus();
    state.az.queue_redraw_all();
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
        crate::input::Under::Surface(_, _, aizome_comp_core::input::Owner::Window(id))
        | crate::input::Under::Decor(id, _) => Some(id),
        _ => None,
    }
}

pub fn pointer_button(state: &mut State, pressed: bool, x: f64, y: f64) -> bool {
    let Some(a) = state.lg.m().overlays.area.as_mut() else { return false };
    if a.window_mode {
        if pressed {
            let w = window_at(state, x, y);
            finish(state);
            if let Some(w) = w
                && let Err(e) =
                    aizome_comp_core::screenshot::take(state, aizome_comp_core::screenshot::What::Window(w), None, true)
            {
                tracing::warn!("screenshot: {e}");
            }
        }
        return true;
    }
    if pressed {
        a.start = Some((x, y));
        a.pos = (x, y);
    } else if let Some(s) = a.start.take() {
        let r = band(s, (x, y));
        finish(state);
        if r.w > 2
            && r.h > 2
            && let Err(e) =
                aizome_comp_core::screenshot::take(state, aizome_comp_core::screenshot::What::Area(r), None, true)
        {
            tracing::warn!("screenshot: {e}");
        }
    }
    true
}

pub fn pointer_moved(state: &mut State, x: f64, y: f64) {
    if let Some(a) = state.lg.m().overlays.area.as_mut() {
        a.pos = (x, y);
        state.az.queue_redraw_all();
    }
}

pub fn key(state: &mut State, syms: &[Keysym], pressed: bool) -> Option<KeyDecision> {
    let a = state.lg.m().overlays.area.as_mut()?;
    if pressed {
        if syms.iter().any(|s| s.raw() == keysyms::KEY_Escape) {
            finish(state);
        } else if syms.iter().any(|s| s.raw() == keysyms::KEY_space) {
            a.window_mode = !a.window_mode;
            a.start = None;
            state.az.cursor_override = Some(if a.window_mode { CursorIcon::Pointer } else { CursorIcon::Crosshair });
            state.az.queue_redraw_all();
        }
    }
    Some(KeyDecision::Swallow)
}

pub fn items(st: &mut State, output: &Output, scale: f64) -> aizome_comp_core::scene::Scene {
    let (az, l) = (&st.az, st.lg.m());
    let Some(mut a) = l.overlays.area.take() else { return Vec::new() };
    let geo = az.output_geometry(output);
    let origin = (geo.loc.x as f64, geo.loc.y as f64);
    let mut pt: Painter = Painter::new(origin, scale, 1.0);
    let p = az.palette;
    let (rect, label) = if a.window_mode {
        let w = l.wm.shown_stacking().into_iter().rev().find(|id| {
            l.decor_layout(*id)
                .map(|l| l.bounds())
                .or_else(|| l.wm.get(*id).map(|w| w.rect))
                .is_some_and(|r| r.contains(a.pos.0, a.pos.1))
        });
        let r = w.and_then(|id| l.decor_layout(id).map(|l| l.bounds()).or_else(|| l.wm.get(id).map(|w| w.rect)));
        (r, "click a window · Space for an area · Esc".to_string())
    } else if let Some(s) = a.start {
        let r = band(s, a.pos);
        (Some(r), format!("{} × {}", r.w, r.h))
    } else {
        (None, "drag over an area · Space for a window · Esc".to_string())
    };
    if let Some(r) = rect {
        let b = Box2::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64);
        pt.fill(b, p.focus.with_alpha(0.15));
        pt.outline(b, 1.0, p.focus);
    }
    let lb = Box2::new(a.pos.0 + 14.0, a.pos.1 + 14.0, 10.0, 20.0);
    if geo.to_f64().contains(smithay::utils::Point::from(a.pos)) {
        let (tw, _) = l.labels.measure(&label, crate::text::Face::Mono, 11.0);
        let chip = Box2::new(lb.x, lb.y, tw.ceil() + 12.0, 20.0);
        pt.fill(chip, p.menu_title);
        pt.text_center(l, &label, mono(11.0, p.on_menu_title), chip);
    }
    let out = pt.finish(&mut a.cache);
    l.overlays.area = Some(a);
    out
}
