//! What the compositor draws above windows: the stack-drop feedback, the
//! marking menu, the switcher, the Desks overview and the screenshot area
//! picker.

pub mod marking;

use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};
use smithay::output::Output;

use crate::render::OutputElement;
use crate::state::Aizome;
use crate::wm::WinId;

#[derive(Default)]
pub struct Overlays {
    pub marking: Option<marking::Marking>,
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
    let _ = (az, renderer, output, scale);
    Vec::new()
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
