//! The displays themselves: DRM/KMS through libseat, libinput, hotplug and
//! VT switching.

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use super::Capture;
use crate::state::Aizome;

pub struct Udev {
    seat: String,
}

impl Udev {
    pub fn new(_event_loop: &smithay::reexports::calloop::EventLoop<'static, crate::state::State>, _renderer: Option<crate::cli::RendererKind>) -> Result<Udev, String> {
        Err("the udev backend isn't built yet".into())
    }
    pub fn init(_state: &mut crate::state::State) {}
    pub fn seat_name(&self) -> String {
        self.seat.clone()
    }
    pub fn renderer_name(&self) -> String {
        "gles".into()
    }
    pub fn redraw(&mut self, _az: &mut Aizome, _output: &Output) {}
    pub fn early_import(&mut self, _surface: &WlSurface) {}
    pub fn import_dmabuf(&mut self, _dmabuf: &Dmabuf) -> bool {
        false
    }
    pub fn capture(&mut self, _az: &mut Aizome, _output: &Output, _cursor: bool) -> Result<Capture, String> {
        Err("not yet".into())
    }
    pub fn set_power(&mut self, _az: &mut Aizome, _on: bool) {}
    pub fn configure_output(&mut self, _az: &mut Aizome, _change: &crate::outputs::OutputChange) -> Result<(), String> {
        Err("not yet".into())
    }
    pub fn change_vt(&mut self, _vt: i32) {}
    pub fn apply_input_settings(&mut self, _input: &aizome_config::Input) {}
}
