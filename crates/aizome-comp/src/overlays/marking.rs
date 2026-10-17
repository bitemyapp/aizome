//! The marking menu on a tab's right button (C4-Marking in Aizome's look).

use crate::state::State;
use crate::wm::WinId;

pub struct Marking {
    pub win: WinId,
    pub center: (f64, f64),
}

/// Opens the menu around a point; `shown` draws it at once (from a
/// client's window-menu request) instead of after a pause.
pub fn open(state: &mut State, win: WinId, at: (f64, f64), shown: bool) {
    let _ = shown;
    state.az.overlays.marking = Some(Marking { win, center: at });
}

/// The right button went down on a tab.
pub fn press(
    state: &mut State,
    win: WinId,
    pos: smithay::utils::Point<f64, smithay::utils::Logical>,
    serial: smithay::utils::Serial,
    time: u32,
) {
    let _ = (serial, time);
    open(state, win, (pos.x, pos.y), false);
}
