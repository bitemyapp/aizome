//! The displays themselves: DRM/KMS through libseat, libinput, hotplug,
//! VT switching and every connected output, rendered with GLES (on each
//! GPU, through smithay's multi-GPU renderer) or pixman into dumb buffers.
//!
//! Frames are drawn only when something is queued: a damaged frame is
//! queued for scan-out and the next one waits for its vblank; an
//! undamaged one is dropped and nothing is submitted.
//!
//! Device and connector handling follow anvil's udev backend (smithay,
//! MIT licence: Copyright (c) 2017 Victor Berger and Victor Brekenfeld).

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::dumb::DumbAllocator;
use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Format as DrmFormat, Modifier};
use smithay::backend::drm::compositor::FrameFlags;
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, NodeType};
use smithay::backend::egl::context::ContextPriority;
use smithay::backend::egl::{EGLDevice, EGLDisplay};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::element::RenderElementStates;
use smithay::backend::renderer::gles::{GlesRenderbuffer, GlesRenderer};
use smithay::backend::renderer::multigpu::gbm::GbmGlesBackend;
use smithay::backend::renderer::multigpu::{GpuManager, MultiRenderer};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{ImportDma, ImportMemWl};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu};
use smithay::desktop::utils::OutputPresentationFeedback;
use smithay::input::keyboard::LedState;
use smithay::output::{Mode as WlMode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::{EventLoop, RegistrationToken};
use smithay::reexports::drm::control::{Device as _, ModeTypeFlags, connector, crtc};
use smithay::reexports::input::{self as libinput, DeviceCapability, Libinput};
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::DeviceFd;
use smithay::wayland::dmabuf::DmabufFeedbackBuilder;
use smithay::wayland::presentation::Refresh;
use tracing::{error, info, warn};

use super::{Capture, Frame, capture_with, post_frame, take_feedback};
use crate::cli::RendererKind;
use crate::outputs::{OutputChange, OutputSerial};
use crate::render::{OutputElement, Purpose, clear_color, output_elements};
use crate::state::{Aizome, State};

const GLES_FORMATS: &[Fourcc] = &[Fourcc::Abgr2101010, Fourcc::Argb2101010, Fourcc::Abgr8888, Fourcc::Argb8888];
const PIXMAN_FORMATS: &[Fourcc] = &[Fourcc::Xrgb8888, Fourcc::Argb8888];

type Gpus = GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>;
type UdevRenderer<'a> = MultiRenderer<'a, 'a, GbmGlesBackend<GlesRenderer, DrmDeviceFd>, GbmGlesBackend<GlesRenderer, DrmDeviceFd>>;
type Feedback = Option<OutputPresentationFeedback>;
type GlesManager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Feedback, DrmDeviceFd>;
type GlesOutput = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Feedback, DrmDeviceFd>;
type Dumb = std::sync::Arc<std::sync::Mutex<DumbAllocator>>;
type PixmanManager = DrmOutputManager<Dumb, DrmDeviceFd, Feedback, DrmDeviceFd>;
type PixmanOutput = DrmOutput<Dumb, DrmDeviceFd, Feedback, DrmDeviceFd>;

enum Renderers {
    Gles(Box<Gpus>),
    Pixman(PixmanRenderer),
}

enum Manager {
    Gles(GlesManager),
    Pixman(PixmanManager),
}

enum SurfaceOutput {
    Gles(Box<GlesOutput>),
    Pixman(Box<PixmanOutput>),
}

/// Which device and CRTC drive an output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UdevOutputId {
    node: DrmNode,
    crtc: crtc::Handle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Redraw {
    Idle,
    /// A frame is queued; the next waits for its vblank.
    WaitingForVBlank,
}

struct Surface {
    output: Output,
    connector: connector::Handle,
    drm: SurfaceOutput,
    redraw: Redraw,
    modes: Vec<smithay::reexports::drm::control::Mode>,
}

struct Device {
    token: RegistrationToken,
    render_node: Option<DrmNode>,
    manager: Manager,
    /// Connected connectors and the CRTCs they drive.
    connectors: HashMap<connector::Handle, crtc::Handle>,
    surfaces: HashMap<crtc::Handle, Surface>,
}

pub struct Udev {
    session: LibSeatSession,
    seat: String,
    primary_gpu: DrmNode,
    renderers: Renderers,
    devices: HashMap<DrmNode, Device>,
    libinput: Libinput,
    input_devices: Vec<libinput::Device>,
    powered: bool,
}

impl Udev {
    pub fn new(event_loop: &EventLoop<'static, State>, renderer: Option<RendererKind>) -> Result<Udev, String> {
        let (session, notifier) = LibSeatSession::new().map_err(|e| format!("no session: {e}"))?;
        let seat = session.seat();
        let primary_gpu = match std::env::var("AIZOME_DRM_DEVICE") {
            Ok(p) => DrmNode::from_path(p).map_err(|e| e.to_string())?,
            Err(_) => primary_gpu(&seat)
                .ok()
                .flatten()
                .and_then(|p| DrmNode::from_path(p).ok()?.node_with_type(NodeType::Render)?.ok())
                .or_else(|| all_gpus(&seat).ok()?.into_iter().find_map(|p| DrmNode::from_path(p).ok()))
                .ok_or("no GPU")?,
        };
        info!("primary GPU {primary_gpu}");
        let renderers = match renderer {
            Some(RendererKind::Pixman) => Renderers::Pixman(PixmanRenderer::new().map_err(|e| e.to_string())?),
            _ => match GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High)) {
                Ok(g) => Renderers::Gles(Box::new(g)),
                Err(e) => {
                    warn!("GLES unavailable ({e:?}); rendering with pixman");
                    Renderers::Pixman(PixmanRenderer::new().map_err(|e| e.to_string())?)
                }
            },
        };

        let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
        libinput.udev_assign_seat(&seat).map_err(|_| "libinput couldn't take the seat".to_string())?;
        let backend = LibinputInputBackend::new(libinput.clone());
        event_loop
            .handle()
            .insert_source(backend, |event, _, state: &mut State| {
                if let InputEvent::DeviceAdded { device } = &event {
                    let mut d = device.clone();
                    if let crate::backend::Backend::Udev(u) = &mut state.backend {
                        apply_device_settings(&mut d, &state.az.settings.input);
                        if d.has_capability(DeviceCapability::Keyboard) {
                            if let Some(kb) = state.az.seat.get_keyboard() {
                                d.led_update(kb.led_state().into());
                            }
                        }
                        u.input_devices.push(d);
                    }
                } else if let InputEvent::DeviceRemoved { device } = &event {
                    if let crate::backend::Backend::Udev(u) = &mut state.backend {
                        u.input_devices.retain(|d| d != device);
                    }
                }
                state.process_input(event);
            })
            .map_err(|e| e.to_string())?;

        let li = libinput.clone();
        event_loop
            .handle()
            .insert_source(notifier, move |event, _, state: &mut State| {
                let mut li = li.clone();
                match event {
                    SessionEvent::PauseSession => {
                        info!("session paused");
                        li.suspend();
                        if let crate::backend::Backend::Udev(u) = &mut state.backend {
                            for d in u.devices.values_mut() {
                                match &mut d.manager {
                                    Manager::Gles(m) => m.pause(),
                                    Manager::Pixman(m) => m.pause(),
                                }
                            }
                        }
                    }
                    SessionEvent::ActivateSession => {
                        info!("session resumed");
                        if let Err(e) = li.resume() {
                            error!("libinput resume: {e:?}");
                        }
                        if let crate::backend::Backend::Udev(u) = &mut state.backend {
                            for d in u.devices.values_mut() {
                                let res = match &mut d.manager {
                                    Manager::Gles(m) => m.activate(false),
                                    Manager::Pixman(m) => m.activate(false),
                                };
                                if let Err(e) = res {
                                    error!("reactivating a DRM device: {e}");
                                }
                                for s in d.surfaces.values_mut() {
                                    s.redraw = Redraw::Idle;
                                    match &mut s.drm {
                                        SurfaceOutput::Gles(o) => o.reset_buffers(),
                                        SurfaceOutput::Pixman(o) => o.reset_buffers(),
                                    }
                                }
                            }
                        }
                        state.az.queue_redraw_all();
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        Ok(Udev { session, seat, primary_gpu, renderers, devices: HashMap::new(), libinput, input_devices: Vec::new(), powered: true })
    }

    /// Opens every GPU, sets up the outputs connected now, the dmabuf
    /// global, and watches for hotplug.
    pub fn init(state: &mut State) {
        let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
        let udev = match UdevBackend::new(&u.seat) {
            Ok(b) => b,
            Err(e) => {
                error!("udev: {e}");
                return;
            }
        };
        let primary_primary = u.primary_gpu.node_with_type(NodeType::Primary).and_then(|n| n.ok());
        let mut list: Vec<(u64, std::path::PathBuf)> = udev.device_list().map(|(d, p)| (d, p.to_path_buf())).collect();
        // The primary GPU first.
        list.sort_by_key(|(d, _)| !(primary_primary.is_some_and(|n| n.dev_id() == *d) || *d == u.primary_gpu.dev_id()));
        for (dev, path) in list {
            match DrmNode::from_dev_id(dev) {
                Ok(node) => {
                    if let Err(e) = device_added(state, node, &path) {
                        error!("skipping {}: {e}", path.display());
                    }
                }
                Err(e) => error!("{e}"),
            }
        }
        let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
        let primary = u.primary_gpu;
        match &mut u.renderers {
            Renderers::Gles(gpus) => {
                if let Ok(mut r) = gpus.single_renderer(&primary) {
                    state.az.shm.update_formats(r.shm_formats());
                    let formats = r.dmabuf_formats();
                    match DmabufFeedbackBuilder::new(primary.dev_id(), formats).build() {
                        Ok(f) => {
                            let g = state.az.dmabuf.create_global_with_default_feedback::<State>(&state.az.dh, &f);
                            state.az.dmabuf_global = Some(g);
                        }
                        Err(e) => warn!("dmabuf feedback: {e}"),
                    }
                }
            }
            Renderers::Pixman(r) => {
                state.az.shm.update_formats(r.shm_formats());
            }
        }
        let _ = state.az.lh.insert_source(udev, |event, _, state: &mut State| match event {
            UdevEvent::Added { device_id, path } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    if let Err(e) = device_added(state, node, &path) {
                        error!("skipping {}: {e}", path.display());
                    }
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_changed(state, node);
                }
            }
            UdevEvent::Removed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    device_removed(state, node);
                }
            }
        });
    }

    pub fn seat_name(&self) -> String {
        self.seat.clone()
    }

    pub fn renderer_name(&self) -> String {
        match self.renderers {
            Renderers::Gles(_) => "gles".into(),
            Renderers::Pixman(_) => "pixman".into(),
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Renderers::Gles(g) = &mut self.renderers {
            if let Err(e) = g.early_import(self.primary_gpu, surface) {
                tracing::debug!("early import: {e:?}");
            }
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match &mut self.renderers {
            Renderers::Gles(g) => {
                let ok = g.single_renderer(&self.primary_gpu).and_then(|mut r| r.import_dmabuf(dmabuf, None)).is_ok();
                if ok {
                    dmabuf.set_node(self.primary_gpu);
                }
                ok
            }
            Renderers::Pixman(r) => r.import_dmabuf(dmabuf, None).is_ok(),
        }
    }

    pub fn update_leds(&mut self, leds: LedState) {
        for d in &mut self.input_devices {
            if d.has_capability(DeviceCapability::Keyboard) {
                d.led_update(leds.into());
            }
        }
    }

    pub fn apply_input_settings(&mut self, input: &aizome_config::Input) {
        for d in &mut self.input_devices {
            apply_device_settings(d, input);
        }
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            warn!("changing to VT {vt}: {e}");
        }
    }

    /// Draws an output if it isn't waiting for a vblank.
    pub fn redraw(&mut self, az: &mut Aizome, output: &Output) {
        if !self.session.is_active() || !self.powered {
            if let Some(o) = az.out.get_mut(output) {
                o.redraw_queued = false;
            }
            return;
        }
        let primary = self.primary_gpu;
        let Some(id) = output.user_data().get::<UdevOutputId>().copied() else { return };
        let Some(device) = self.devices.get_mut(&id.node) else { return };
        let render_node = device.render_node.unwrap_or(primary);
        let Some(surface) = device.surfaces.get_mut(&id.crtc) else { return };
        if surface.redraw == Redraw::WaitingForVBlank {
            return;
        }
        if let Some(o) = az.out.get_mut(output) {
            o.redraw_queued = false;
        }
        let started = Instant::now();
        let result = match (&mut self.renderers, &mut surface.drm) {
            (Renderers::Gles(gpus), SurfaceOutput::Gles(drm)) => {
                let renderer = if render_node == primary {
                    gpus.single_renderer(&render_node)
                } else {
                    let format = drm.format();
                    gpus.renderer(&primary, &render_node, format)
                };
                match renderer {
                    Ok(mut r) => {
                        let elements = output_elements(az, &mut r, output, Purpose::Screen);
                        let clear = clear_color(az);
                        match drm.render_frame(&mut r, &elements, clear, FrameFlags::DEFAULT) {
                            Ok(res) => {
                                let damaged = !res.is_empty;
                                let states = res.states;
                                queue(az, output, damaged, &states, |fb| drm.queue_frame(fb).map_err(|e| format!("{e:?}")))
                                    .map(|q| (q, states))
                            }
                            Err(e) => Err(format!("{e:?}")),
                        }
                    }
                    Err(e) => Err(format!("renderer: {e:?}")),
                }
            }
            (Renderers::Pixman(r), SurfaceOutput::Pixman(drm)) => {
                let elements = output_elements(az, r, output, Purpose::Screen);
                let clear = clear_color(az);
                match drm.render_frame(r, &elements, clear, FrameFlags::DEFAULT) {
                    Ok(res) => {
                        let damaged = !res.is_empty;
                        let states = res.states;
                        queue(az, output, damaged, &states, |fb| drm.queue_frame(fb).map_err(|e| format!("{e:?}"))).map(|q| (q, states))
                    }
                    Err(e) => Err(format!("{e:?}")),
                }
            }
            _ => Err("renderer and output disagree".into()),
        };
        match result {
            Ok((queued, states)) => {
                if queued {
                    surface.redraw = Redraw::WaitingForVBlank;
                }
                let frame = Frame { damaged: queued, states, cpu: started.elapsed(), gpu: None };
                post_frame(az, output, &frame, false);
            }
            Err(e) => warn!("rendering {}: {e}", output.name()),
        }
    }

    pub fn capture(&mut self, az: &mut Aizome, output: &Output, cursor: bool) -> Result<Capture, String> {
        let primary = self.primary_gpu;
        match &mut self.renderers {
            Renderers::Gles(gpus) => {
                let mut r = gpus.single_renderer(&primary).map_err(|e| format!("{e:?}"))?;
                capture_with::<UdevRenderer<'_>, GlesRenderbuffer>(&mut r, az, output, cursor)
            }
            Renderers::Pixman(r) => capture_with::<PixmanRenderer, smithay::reexports::pixman::Image<'static, 'static>>(r, az, output, cursor),
        }
    }

    pub fn set_power(&mut self, az: &mut Aizome, on: bool) {
        self.powered = on;
        for d in self.devices.values_mut() {
            for s in d.surfaces.values_mut() {
                if !on {
                    let res = match &mut s.drm {
                        SurfaceOutput::Gles(o) => o.with_compositor(|c| c.clear()).map_err(|e| format!("{e:?}")),
                        SurfaceOutput::Pixman(o) => o.with_compositor(|c| c.clear()).map_err(|e| format!("{e:?}")),
                    };
                    if let Err(e) = res {
                        warn!("powering off {}: {e}", s.output.name());
                    }
                }
                s.redraw = Redraw::Idle;
            }
        }
        if on {
            az.queue_redraw_all();
        }
    }

    /// Mode, scale, position and transform; turning outputs off isn't
    /// supported yet.
    pub fn configure_output(&mut self, az: &mut Aizome, change: &OutputChange) -> Result<(), String> {
        let output = az.output_by_name(&change.name).ok_or_else(|| format!("no output {}", change.name))?;
        if change.enabled == Some(false) {
            return Err("turning outputs off isn't supported yet".into());
        }
        if let Some((w, h, refresh)) = change.mode {
            let primary = self.primary_gpu;
            let id = *output.user_data().get::<UdevOutputId>().ok_or("not a DRM output")?;
            let device = self.devices.get_mut(&id.node).ok_or("no device")?;
            let render_node = device.render_node.unwrap_or(primary);
            let s = device.surfaces.get_mut(&id.crtc).ok_or("no surface")?;
            let mode = s
                .modes
                .iter()
                .filter(|m| m.size() == (w as u16, h as u16))
                .min_by_key(|m| refresh.map(|r| (WlMode::from(**m).refresh - r).abs()).unwrap_or(0))
                .copied()
                .ok_or_else(|| format!("{} has no mode {w}x{h}", change.name))?;
            let res = match (&mut self.renderers, &mut s.drm) {
                (Renderers::Gles(gpus), SurfaceOutput::Gles(o)) => {
                    let mut r = gpus.single_renderer(&render_node).map_err(|e| format!("{e:?}"))?;
                    o.use_mode::<_, OutputElement<UdevRenderer<'_>>>(mode, &mut r, &DrmOutputRenderElements::default()).map_err(|e| format!("{e:?}"))
                }
                (Renderers::Pixman(r), SurfaceOutput::Pixman(o)) => {
                    o.use_mode::<_, OutputElement<PixmanRenderer>>(mode, r, &DrmOutputRenderElements::default()).map_err(|e| format!("{e:?}"))
                }
                _ => Err("renderer and output disagree".into()),
            };
            res?;
            let wl = WlMode::from(mode);
            output.change_current_state(Some(wl), None, None, None);
            if let Some(o) = az.out.get_mut(&output) {
                o.refresh = Duration::from_nanos(1_000_000_000_000 / wl.refresh.max(1) as u64);
            }
        }
        crate::outputs::apply_logical(az, change)
    }
}

/// Queues a damaged frame for scan-out with its presentation feedback.
fn queue(
    az: &mut Aizome,
    output: &Output,
    damaged: bool,
    states: &RenderElementStates,
    q: impl FnOnce(Feedback) -> Result<(), String>,
) -> Result<bool, String> {
    if !damaged {
        return Ok(false);
    }
    let feedback = take_feedback(az, output, states);
    q(Some(feedback))?;
    Ok(true)
}

fn device_added(state: &mut State, node: DrmNode, path: &Path) -> Result<(), String> {
    let crate::backend::Backend::Udev(u) = &mut state.backend else { return Err("not udev".into()) };
    let fd = u
        .session
        .open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| format!("open: {e}"))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, notifier) = DrmDevice::new(fd.clone(), true).map_err(|e| format!("DRM: {e}"))?;
    let token = state
        .az
        .lh
        .insert_source(notifier, move |event, metadata, state: &mut State| match event {
            DrmEvent::VBlank(crtc) => vblank(state, node, crtc, metadata),
            DrmEvent::Error(e) => error!("DRM: {e:?}"),
        })
        .map_err(|e| e.to_string())?;
    let crate::backend::Backend::Udev(u) = &mut state.backend else { unreachable!() };
    let (manager, render_node) = match &mut u.renderers {
        Renderers::Gles(gpus) => {
            let gbm = GbmDevice::new(fd.clone()).map_err(|e| format!("GBM: {e}"))?;
            // Safety: the GBM device outlives the display.
            let render_node = unsafe { EGLDisplay::new(gbm.clone()) }
                .ok()
                .and_then(|d| EGLDevice::device_for_display(&d).ok())
                .filter(|d| !d.is_software())
                .map(|d| d.try_get_render_node().ok().flatten().unwrap_or(node))
                .filter(|rn| AsMut::<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>::as_mut(&mut **gpus).add_node(*rn, gbm.clone()).is_ok());
            let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
            let exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);
            let target = render_node.unwrap_or(u.primary_gpu);
            let mut r = gpus.single_renderer(&target).map_err(|e| format!("renderer: {e:?}"))?;
            let formats: FormatSet = r
                .as_mut()
                .egl_context()
                .dmabuf_render_formats()
                .iter()
                .filter(|f| render_node.is_some() || f.modifier == Modifier::Linear)
                .copied()
                .collect();
            (Manager::Gles(DrmOutputManager::new(drm, allocator, exporter, Some(gbm), GLES_FORMATS.iter().copied(), formats)), render_node)
        }
        Renderers::Pixman(_) => {
            let allocator: Dumb = std::sync::Arc::new(std::sync::Mutex::new(DumbAllocator::new(fd.clone())));
            let formats: Vec<DrmFormat> = PIXMAN_FORMATS.iter().map(|code| DrmFormat { code: *code, modifier: Modifier::Linear }).collect();
            (Manager::Pixman(DrmOutputManager::new(drm, allocator, fd.clone(), None::<GbmDevice<DrmDeviceFd>>, PIXMAN_FORMATS.iter().copied(), formats)), None)
        }
    };
    u.devices.insert(node, Device { token, render_node, manager, connectors: HashMap::new(), surfaces: HashMap::new() });
    device_changed(state, node);
    Ok(())
}

/// Connectors plugged or unplugged on a device.
fn device_changed(state: &mut State, node: DrmNode) {
    let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = u.devices.get_mut(&node) else { return };
    let drm = match &device.manager {
        Manager::Gles(m) => m.device(),
        Manager::Pixman(m) => m.device(),
    };
    let Ok(res) = drm.resource_handles() else { return };
    let mut connected = Vec::new();
    let mut gone = Vec::new();
    for &handle in res.connectors() {
        let Ok(info) = drm.get_connector(handle, true) else { continue };
        let is_on = info.state() == connector::State::Connected;
        let known = device.connectors.contains_key(&handle);
        if is_on && !known {
            // A free CRTC its encoders can drive.
            let used: Vec<crtc::Handle> = device.connectors.values().copied().collect();
            let crtc = info
                .encoders()
                .iter()
                .filter_map(|e| drm.get_encoder(*e).ok())
                .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                .find(|c| !used.contains(c));
            if let Some(crtc) = crtc {
                device.connectors.insert(handle, crtc);
                connected.push((info, crtc));
            } else {
                warn!("no free CRTC for {:?}", info.interface());
            }
        } else if !is_on && known {
            let crtc = device.connectors.remove(&handle).unwrap();
            gone.push(crtc);
        }
    }
    for crtc in gone {
        connector_disconnected(state, node, crtc);
    }
    for (info, crtc) in connected {
        connector_connected(state, node, info, crtc);
    }
}

fn device_removed(state: &mut State, node: DrmNode) {
    let crtcs: Vec<crtc::Handle> = match &state.backend {
        crate::backend::Backend::Udev(u) => u.devices.get(&node).map(|d| d.surfaces.keys().copied().collect()).unwrap_or_default(),
        _ => return,
    };
    for crtc in crtcs {
        connector_disconnected(state, node, crtc);
    }
    let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
    if let Some(d) = u.devices.remove(&node) {
        if let (Some(rn), Renderers::Gles(g)) = (d.render_node, &mut u.renderers) {
            AsMut::<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>::as_mut(&mut **g).remove_node(&rn);
        }
        state.az.lh.remove(d.token);
    }
}

/// Make, model and serial from a connector's EDID.
fn edid_info(drm: &DrmDevice, conn: connector::Handle) -> (String, String, String) {
    let unknown = || ("Unknown".to_string(), "Unknown".to_string(), String::new());
    let Ok(props) = drm.get_properties(conn) else { return unknown() };
    let blob = props.into_iter().find_map(|(h, v)| {
        let info = drm.get_property(h).ok()?;
        (info.name().to_str() == Ok("EDID")).then_some(v)
    });
    let Some(blob) = blob.filter(|b| *b != 0) else { return unknown() };
    let Ok(edid) = drm.get_property_blob(blob) else { return unknown() };
    parse_edid(&edid).unwrap_or_else(unknown)
}

/// The manufacturer's PNP id, the monitor name and serial descriptors.
pub fn parse_edid(e: &[u8]) -> Option<(String, String, String)> {
    if e.len() < 128 || e[0..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    let id = u16::from_be_bytes([e[8], e[9]]);
    let letter = |v: u16| (b'A' + ((v & 0x1f) as u8).saturating_sub(1)) as char;
    let make: String = [letter(id >> 10), letter(id >> 5), letter(id)].iter().collect();
    let product = u16::from_le_bytes([e[10], e[11]]);
    let serial_num = u32::from_le_bytes([e[12], e[13], e[14], e[15]]);
    let mut model = format!("0x{product:04X}");
    let mut serial = if serial_num != 0 { serial_num.to_string() } else { String::new() };
    for d in 0..4 {
        let b = &e[54 + d * 18..72 + d * 18];
        if b[0] == 0 && b[1] == 0 {
            let text: String = b[5..18].iter().take_while(|c| **c != b'\n').map(|c| *c as char).collect::<String>().trim().to_string();
            match b[3] {
                0xfc if !text.is_empty() => model = text,
                0xff if !text.is_empty() => serial = text,
                _ => {}
            }
        }
    }
    Some((make, model, serial))
}

/// 2 for HiDPI panels (over ~180 dots per inch), else 1.
pub fn auto_scale(width_px: i32, width_mm: i32) -> f64 {
    if width_mm <= 0 {
        return 1.0;
    }
    let dpi = width_px as f64 / (width_mm as f64 / 25.4);
    if dpi > 180.0 { 2.0 } else { 1.0 }
}

fn connector_connected(state: &mut State, node: DrmNode, info: connector::Info, crtc: crtc::Handle) {
    let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
    let primary = u.primary_gpu;
    let Some(device) = u.devices.get_mut(&node) else { return };
    let drm = match &device.manager {
        Manager::Gles(m) => m.device(),
        Manager::Pixman(m) => m.device(),
    };
    let (make, model, serial) = edid_info(drm, info.handle());
    let modes: Vec<_> = info.modes().to_vec();
    if modes.is_empty() {
        return;
    }
    let cfg = state.az.settings.output(&name, &make, &model, &serial).cloned();
    if cfg.as_ref().and_then(|c| c.enabled) == Some(false) {
        info!("{name} is disabled in the settings");
        return;
    }
    let preferred = modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied().unwrap_or(modes[0]);
    let drm_mode = cfg
        .as_ref()
        .and_then(|c| c.parsed_mode())
        .and_then(|(w, h, r)| {
            modes
                .iter()
                .filter(|m| m.size() == (w as u16, h as u16))
                .min_by_key(|m| r.map(|r| (WlMode::from(**m).refresh - r).abs()).unwrap_or(0))
                .copied()
        })
        .unwrap_or(preferred);
    let (mm_w, mm_h) = info.size().unwrap_or((0, 0));
    let output = Output::new(
        name.clone(),
        PhysicalProperties { size: (mm_w as i32, mm_h as i32).into(), subpixel: Subpixel::Unknown, make: make.clone(), model: model.clone() },
    );
    for m in &modes {
        output.add_mode(WlMode::from(*m));
    }
    output.set_preferred(WlMode::from(preferred));
    output.user_data().insert_if_missing(|| UdevOutputId { node, crtc });
    output.user_data().insert_if_missing(|| OutputSerial(serial.clone()));
    let wl_mode = WlMode::from(drm_mode);
    output.change_current_state(Some(wl_mode), None, None, None);
    let render_node = device.render_node.unwrap_or(primary);
    let drm_output = match (&mut u.renderers, &mut device.manager) {
        (Renderers::Gles(gpus), Manager::Gles(m)) => {
            let Ok(mut r) = gpus.single_renderer(&render_node) else { return };
            m.initialize_output::<_, OutputElement<UdevRenderer<'_>>>(crtc, drm_mode, &[info.handle()], &output, None, &mut r, &DrmOutputRenderElements::default())
                .map(|o| SurfaceOutput::Gles(Box::new(o)))
                .map_err(|e| format!("{e:?}"))
        }
        (Renderers::Pixman(r), Manager::Pixman(m)) => m
            .initialize_output::<_, OutputElement<PixmanRenderer>>(crtc, drm_mode, &[info.handle()], &output, None, r, &DrmOutputRenderElements::default())
            .map(|o| SurfaceOutput::Pixman(Box::new(o)))
            .map_err(|e| format!("{e:?}")),
        _ => Err("renderer and device disagree".into()),
    };
    let drm_output = match drm_output {
        Ok(o) => o,
        Err(e) => {
            warn!("setting up {name}: {e}");
            return;
        }
    };
    device.surfaces.insert(crtc, Surface { output: output.clone(), connector: info.handle(), drm: drm_output, redraw: Redraw::Idle, modes });
    let scale = cfg.as_ref().and_then(|c| c.scale).unwrap_or_else(|| auto_scale(wl_mode.size.w, mm_w as i32));
    let x = state.az.outputs.iter().map(|o| state.az.output_rect(o).right()).max().unwrap_or(0);
    let pos = cfg.as_ref().and_then(|c| c.position).map(|p| (p[0], p[1])).unwrap_or((x, 0));
    info!(name, make, model, ?pos, scale, "output connected");
    crate::outputs::add_output(&mut state.az, &output, wl_mode, scale, pos, &serial);
    if let Some(t) = cfg.and_then(|c| c.transform) {
        output.change_current_state(None, Some(crate::outputs::transform_from_config(t)), None, None);
    }
    crate::actions::rescue_windows(&mut state.az);
}

fn connector_disconnected(state: &mut State, node: DrmNode, crtc: crtc::Handle) {
    let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = u.devices.get_mut(&node) else { return };
    let Some(surface) = device.surfaces.remove(&crtc) else { return };
    device.connectors.retain(|_, c| *c != crtc);
    info!("output {} disconnected", surface.output.name());
    crate::outputs::remove_output(&mut state.az, &surface.output);
    let _ = surface.connector;
}

/// A frame reached the screen: presentation feedback, then the next frame
/// may draw.
fn vblank(state: &mut State, node: DrmNode, crtc: crtc::Handle, metadata: &mut Option<DrmEventMetadata>) {
    let crate::backend::Backend::Udev(u) = &mut state.backend else { return };
    let Some(device) = u.devices.get_mut(&node) else { return };
    let Some(surface) = device.surfaces.get_mut(&crtc) else { return };
    surface.redraw = Redraw::Idle;
    let submitted = match &mut surface.drm {
        SurfaceOutput::Gles(o) => o.frame_submitted().map_err(|e| format!("{e:?}")),
        SurfaceOutput::Pixman(o) => o.frame_submitted().map_err(|e| format!("{e:?}")),
    };
    let output = surface.output.clone();
    let refresh = output.current_mode().map(|m| Duration::from_nanos(1_000_000_000_000 / m.refresh.max(1) as u64)).unwrap_or(Duration::from_millis(16));
    let (time, flags) = match metadata.as_ref().map(|m| m.time) {
        Some(DrmEventTime::Monotonic(t)) if !t.is_zero() => (
            t.into(),
            wp_presentation_feedback::Kind::Vsync | wp_presentation_feedback::Kind::HwClock | wp_presentation_feedback::Kind::HwCompletion,
        ),
        _ => (state.az.clock.now(), wp_presentation_feedback::Kind::Vsync),
    };
    let seq = metadata.as_ref().map(|m| m.sequence as u64).unwrap_or(0);
    match submitted {
        Ok(Some(Some(mut feedback))) => feedback.presented(time, Refresh::fixed(refresh), seq, flags),
        Ok(_) => {}
        Err(e) => warn!("frame submission on {}: {e}", output.name()),
    }
    // Animations and anything queued meanwhile draw now.
    if state.az.anims.active() {
        state.az.queue_redraw(&output);
    }
}

/// Pointer acceleration, scrolling, tapping and the rest, from the
/// settings.
fn apply_device_settings(d: &mut libinput::Device, input: &aizome_config::Input) {
    use aizome_config::{AccelProfile, ClickMethod};
    let touchpad = d.config_tap_finger_count() > 0;
    let (profile, speed, natural) = if touchpad {
        (input.touchpad.accel_profile, input.touchpad.speed, input.touchpad.natural_scroll)
    } else {
        (input.pointer.accel_profile, input.pointer.speed, input.pointer.natural_scroll)
    };
    if d.config_accel_is_available() {
        let _ = d.config_accel_set_profile(match profile {
            AccelProfile::Flat => libinput::AccelProfile::Flat,
            AccelProfile::Adaptive => libinput::AccelProfile::Adaptive,
        });
        let _ = d.config_accel_set_speed(speed.clamp(-1.0, 1.0));
    }
    if d.config_scroll_has_natural_scroll() {
        let _ = d.config_scroll_set_natural_scroll_enabled(natural);
    }
    if d.config_left_handed_is_available() {
        let _ = d.config_left_handed_set(input.pointer.left_handed);
    }
    if touchpad {
        let _ = d.config_tap_set_enabled(input.touchpad.tap);
        if d.config_dwt_is_available() {
            let _ = d.config_dwt_set_enabled(input.touchpad.disable_while_typing);
        }
        let _ = d.config_click_set_method(match input.touchpad.click_method {
            ClickMethod::Clickfinger => libinput::ClickMethod::Clickfinger,
            ClickMethod::ButtonAreas => libinput::ClickMethod::ButtonAreas,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edid_names() {
        let mut e = vec![0u8; 128];
        e[0..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        // "BOE": B=2, O=15, E=5.
        let id: u16 = (2 << 10) | (15 << 5) | 5;
        e[8..10].copy_from_slice(&id.to_be_bytes());
        e[54..59].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
        e[59..67].copy_from_slice(b"NE135A1\n");
        let (make, model, _) = parse_edid(&e).unwrap();
        assert_eq!(make, "BOE");
        assert_eq!(model, "NE135A1");
        assert!(parse_edid(&[0; 10]).is_none());
    }

    #[test]
    fn hidpi_panels_get_scale_2() {
        assert_eq!(auto_scale(2880, 290), 2.0);
        assert_eq!(auto_scale(1920, 527), 1.0);
        assert_eq!(auto_scale(1920, 0), 1.0);
    }
}
