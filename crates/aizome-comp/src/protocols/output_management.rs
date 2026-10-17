//! wlr-output-management, so wlr-randr and kanshi work.

use smithay::reexports::wayland_server::{Client, DisplayHandle};

use crate::state::Aizome;

pub struct OutputManagementState {}

impl OutputManagementState {
    pub fn new<D, F>(_dh: &DisplayHandle, _filter: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        OutputManagementState {}
    }
}

pub fn outputs_changed(_az: &mut Aizome) {}
