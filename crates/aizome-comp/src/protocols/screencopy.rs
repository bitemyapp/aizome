//! wlr-screencopy-unstable-v1, for xdg-desktop-portal-wlr.

use smithay::reexports::wayland_server::{Client, DisplayHandle};

pub struct ScreencopyManagerState {}

impl ScreencopyManagerState {
    pub fn new<D, F>(_dh: &DisplayHandle, _filter: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        ScreencopyManagerState {}
    }
}
