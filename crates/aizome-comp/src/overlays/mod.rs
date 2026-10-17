//! What the compositor draws above windows: the stack-drop feedback, the
//! marking menu, the switcher, the Desks overview and the screenshot area
//! picker.

pub mod marking;

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};
use smithay::output::Output;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::utils::{Logical, Physical, Point};

use crate::decor::Piece;
use crate::render::OutputElement;
use crate::text::{Face, Style};
use crate::state::Aizome;
use crate::wm::WinId;

#[derive(Default)]
pub struct Overlays {
    pub marking: Option<marking::Marking>,
    pub drag_cache: crate::decor::DecorCache,
}

impl Overlays {
    /// The overview covers everything else on its outputs.
    pub fn covers_desktop(&self, _output: &Output) -> bool {
        false
    }

    /// While a modal overlay is up, the focused tab doesn't show focus.
    pub fn steals_focus_look(&self) -> bool {
        false
    }

    /// Keys go to the compositor, not the focused client.
    pub fn grabs_keyboard(&self) -> bool {
        false
    }

    pub fn animating(&self) -> bool {
        false
    }
}

/// Overlay elements for an output, front first.
pub fn elements<R>(az: &mut Aizome, renderer: &mut R, output: &Output, scale: f64) -> Vec<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let _ = (az, renderer, output, scale);
    Vec::new()
}

/// Feedback for a tab being dragged: drawn under the top layer.
pub fn drag_feedback<R>(az: &mut Aizome, renderer: &mut R, output: &Output, scale: f64) -> Vec<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let mut out = Vec::new();
    let Some(drag) = az.interactive.drag.clone() else { return out };
    let (Some(slot), Some(target)) = (drag.slot, drag.target) else { return out };
    let geo = az.output_geometry(output);
    let origin = (geo.loc.x as f64, geo.loc.y as f64);
    if !az.output_rect(output).intersects(slot.grow(60)) {
        return out;
    }
    let p = az.palette;
    // The label above the slot: "Release to stack with ‹title›".
    let title = az.wm.get(target).map(|w| w.title.clone()).unwrap_or_default();
    let text = format!("Release to stack with {title}");
    let text = az.labels.fit(&text, Face::UiSemibold, 12.0, 420.0);
    let style = Style::new(Face::UiSemibold, 12.0, p.on_menu_title);
    let mut pieces = Vec::new();
    // Slot: focus at 18% with a 2pt dashed focus border.
    pieces.push(Piece::rect(slot, p.focus.with_alpha(0.18)));
    dashed(&mut pieces, slot, 2.0, 6.0, 4.0, p.focus);
    let mut label_el = None;
    if let Some(label) = az.labels.get(&text, style, scale, 440.0) {
        let (w, h) = (label.w as f64 + 16.0, 22.0);
        let x = slot.x as f64;
        let y = slot.y as f64 - h - 18.0;
        pieces.push(Piece::new(x, y, w, h, p.menu_title));
        let loc: Point<i32, Physical> =
            Point::<f64, Logical>::from((x + 8.0 - origin.0, y + ((h - label.h as f64) / 2.0).round() - origin.1)).to_physical_precise_round(scale);
        label_el = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64(),
            &label.buffer,
            None,
            None,
            Some((label.w, label.h).into()),
            Kind::Unspecified,
        )
        .ok()
        .map(OutputElement::Memory);
    }
    out.extend(label_el);
    out.extend(az.overlays.drag_cache.elements(&pieces, origin, scale, 1.0).into_iter().map(OutputElement::Solid));
    out
}

/// A dashed outline: dashes `dash` long with `gap` between, `t` thick.
pub fn dashed(out: &mut Vec<Piece>, r: crate::wm::Rect, t: f64, dash: f64, gap: f64, color: aizome_theme::Rgba) {
    let (x, y, w, h) = (r.x as f64, r.y as f64, r.w as f64, r.h as f64);
    let mut along = |len: f64, mut put: Box<dyn FnMut(f64, f64) + '_>| {
        let mut s = 0.0;
        while s < len {
            let l = dash.min(len - s);
            put(s, l);
            s += dash + gap;
        }
    };
    let mut horiz = Vec::new();
    along(w, Box::new(|s, l| horiz.push((s, l))));
    for (s, l) in &horiz {
        out.push(Piece::new(x + s, y, *l, t, color));
        out.push(Piece::new(x + s, y + h - t, *l, t, color));
    }
    let mut vert = Vec::new();
    along(h - 2.0 * t, Box::new(|s, l| vert.push((s, l))));
    for (s, l) in &vert {
        out.push(Piece::new(x, y + t + s, t, *l, color));
        out.push(Piece::new(x + w - t, y + t + s, t, *l, color));
    }
}

/// A window closed: overlays forget it.
pub fn window_gone(az: &mut Aizome, id: WinId) {
    let _ = (az, id);
}

/// Scrolling over an overlay; true if it took it.
pub fn scroll(state: &mut crate::state::State, h: f64, v: f64) -> bool {
    let _ = (state, h, v);
    false
}

pub fn pointer_moved(state: &mut crate::state::State, pos: smithay::utils::Point<f64, smithay::utils::Logical>) {
    let _ = (state, pos);
}

/// A button over an overlay; true if it took it.
pub fn pointer_button(
    state: &mut crate::state::State,
    button: u32,
    pressed: bool,
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
) -> bool {
    let _ = (state, button, pressed, pos);
    false
}

/// A key while an overlay is up; `None` lets it through.
pub fn key(
    state: &mut crate::state::State,
    syms: &[smithay::input::keyboard::Keysym],
    mods: crate::bindings::Mods,
    pressed: bool,
) -> Option<crate::input::KeyDecision> {
    let _ = (state, syms, mods, pressed);
    None
}

pub fn toggle_overview(state: &mut crate::state::State) {
    let _ = state;
}

pub fn open_switcher(state: &mut crate::state::State, reverse: bool, held: bool) {
    let _ = (state, reverse, held);
}

pub fn start_area_pick(state: &mut crate::state::State) {
    let _ = state;
}

pub fn close_switcher(state: &mut crate::state::State, activate: bool) {
    let _ = (state, activate);
}

impl Overlays {
    pub fn overview_open(&self) -> bool {
        false
    }
}
