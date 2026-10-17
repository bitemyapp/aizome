//! Foreign toplevels: ext-foreign-toplevel-list (smithay) and
//! wlr-foreign-toplevel-management.

use smithay::reexports::wayland_server::{Client, DisplayHandle};

use crate::state::State;
use crate::wm::WinId;

pub struct ForeignToplevelManagerState {}

impl ForeignToplevelManagerState {
    pub fn new<D, F>(_dh: &DisplayHandle, _filter: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        ForeignToplevelManagerState {}
    }
}

pub fn window_opened(_state: &mut State, _id: WinId) {}
pub fn window_closed(_state: &mut State, _id: WinId) {}
pub fn window_changed(_state: &mut State, _id: WinId) {}
pub fn refresh(_state: &mut State) {}
