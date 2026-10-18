//! Where frames go: offscreen (headless), a window in another session
//! (winit), or the displays themselves (udev: DRM/KMS with libseat).

pub mod headless;
pub mod udev;
pub mod winit;

use std::time::{Duration, Instant};

use smithay::backend::allocator::Fourcc;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::RenderElementStates;
use smithay::backend::renderer::{Bind, ExportMem, ImportAll, ImportMem, Offscreen, Renderer};
use smithay::desktop::layer_map_for_output;
use smithay::desktop::utils::{
    OutputPresentationFeedback, surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
    update_surface_primary_scanout_output,
};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer as BufferCoords, Rectangle, Size, Transform};

use crate::render::{OutputElement, Purpose, clear_color, output_elements};
use crate::state::Aizome;

pub enum Backend {
    Headless(headless::Headless),
    Winit(winit::WinitBackend),
    Udev(Box<udev::Udev>),
}

/// Pixels read back from a frame: RGBA, premultiplied, rows `width × 4`
/// bytes apart.
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Backend {
    pub fn seat_name(&self) -> String {
        match self {
            Backend::Udev(u) => u.seat_name(),
            _ => "seat0".into(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Backend::Headless(_) => "headless",
            Backend::Winit(_) => "winit",
            Backend::Udev(_) => "udev",
        }
    }

    pub fn renderer_name(&self) -> String {
        match self {
            Backend::Headless(h) => h.renderer_name().into(),
            Backend::Winit(_) => "gles".into(),
            Backend::Udev(u) => u.renderer_name(),
        }
    }

    /// Draws an output if its frame is due (otherwise arranges to come
    /// back when it is).
    pub fn redraw(&mut self, az: &mut Aizome, output: &Output) {
        match self {
            Backend::Headless(h) => h.redraw(az, output),
            Backend::Winit(w) => w.redraw(az, output),
            Backend::Udev(u) => u.redraw(az, output),
        }
    }

    /// Imports a surface's new buffer ahead of the next frame.
    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Backend::Udev(u) = self {
            u.early_import(surface);
        }
    }

    /// Under GLES a wl_shm buffer's pixels are copied into a texture when
    /// it's imported, so the buffer can go back to the client then, not at
    /// its next commit: import the tree now and release the buffers it just
    /// attached (`fresh`). Pixman draws from the client's memory, and
    /// several GPUs may each read it, so there they are kept.
    ///
    /// The client hears `release` once more when the next commit replaces
    /// the buffer (smithay's own); by then its pixels are in the texture.
    pub fn release_shm_after_import(&mut self, surface: &WlSurface, fresh: &[WlBuffer]) {
        let imported = match self {
            Backend::Headless(h) => h.import_shm_copies(surface),
            Backend::Winit(w) => w.import_shm_copies(surface),
            Backend::Udev(u) => u.import_shm_copies(surface),
        };
        if imported {
            for b in fresh {
                b.release();
            }
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Backend::Headless(h) => h.import_dmabuf(dmabuf),
            Backend::Winit(w) => w.import_dmabuf(dmabuf),
            Backend::Udev(u) => u.import_dmabuf(dmabuf),
        }
    }

    /// Renders an output afresh into memory (screenshots, screencopy).
    pub fn capture(&mut self, az: &mut Aizome, output: &Output, cursor: bool) -> Result<Capture, String> {
        match self {
            Backend::Headless(h) => h.capture(az, output, cursor),
            Backend::Winit(w) => w.capture(az, output, cursor),
            Backend::Udev(u) => u.capture(az, output, cursor),
        }
    }

    /// Turns displays off or on (DPMS); a no-op off udev.
    pub fn set_power(&mut self, az: &mut Aizome, on: bool) {
        if let Backend::Udev(u) = self {
            u.set_power(az, on);
        }
        for o in az.out.values_mut() {
            o.powered = on;
        }
        if on {
            az.queue_redraw_all();
        }
    }

    /// Applies a mode, scale, position, transform or enablement change to
    /// an output.
    pub fn configure_output(&mut self, az: &mut Aizome, change: &crate::outputs::OutputChange) -> Result<(), String> {
        match self {
            Backend::Udev(u) => u.configure_output(az, change),
            Backend::Headless(h) => h.configure_output(az, change),
            Backend::Winit(_) => crate::outputs::apply_logical(az, change),
        }
    }

    pub fn headless_mut(&mut self) -> Option<&mut headless::Headless> {
        match self {
            Backend::Headless(h) => Some(h),
            _ => None,
        }
    }

    /// Switches virtual terminals (udev only).
    pub fn change_vt(&mut self, vt: i32) {
        if let Backend::Udev(u) = self {
            u.change_vt(vt);
        }
    }
}

/// The outcome of drawing one frame.
pub struct Frame {
    pub damaged: bool,
    pub states: RenderElementStates,
    pub cpu: Duration,
    pub gpu: Option<Duration>,
}

/// Draws an output's elements through a damage tracker into a bound
/// framebuffer, timing it.
pub fn render_frame<R>(
    az: &mut Aizome,
    renderer: &mut R,
    framebuffer: &mut R::Framebuffer<'_>,
    tracker: &mut OutputDamageTracker,
    age: usize,
    output: &Output,
    elements: &[OutputElement<R>],
    started: Instant,
    wait_gpu: bool,
) -> Result<Frame, String>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let clear = clear_color(az);
    let res = tracker.render_output(renderer, framebuffer, age, elements, clear).map_err(|e| format!("{e:?}"))?;
    let cpu = started.elapsed();
    let damaged = res.damage.is_some_and(|d| !d.is_empty());
    let gpu = if wait_gpu && damaged {
        let t = Instant::now();
        let _ = res.sync.wait();
        Some(t.elapsed())
    } else {
        None
    };
    let states = res.states;
    let _ = output;
    Ok(Frame { damaged, states, cpu, gpu })
}

/// After a frame: statistics, the surfaces' primary outputs, frame
/// callbacks and presentation feedback.
pub fn post_frame(az: &mut Aizome, output: &Output, frame: &Frame, presented: bool) {
    if let Some(o) = az.out.get_mut(output) {
        if frame.damaged {
            o.stats.record(frame.cpu, frame.gpu);
        } else {
            o.stats.skip();
        }
        o.frame_seq += 1;
        if frame.damaged {
            o.damaged_frames += 1;
        }
        o.last_frame = Some(Instant::now());
    }
    update_primary_outputs(az, output, &frame.states);
    send_frames(az, output);
    if presented && frame.damaged {
        let mut feedback = take_feedback(az, output, &frame.states);
        let refresh = az.out.get(output).map(|o| o.refresh).unwrap_or(Duration::from_millis(16));
        let seq = az.out.get(output).map(|o| o.frame_seq).unwrap_or(0);
        feedback.presented::<_, smithay::utils::Monotonic>(
            az.clock.now(),
            smithay::wayland::presentation::Refresh::fixed(refresh),
            seq,
            wp_presentation_feedback::Kind::empty(),
        );
    }
    // Keep animating.
    if az.anims.active() || az.overlays.animating() {
        az.anims.tick();
        az.queue_redraw(output);
    }
}

fn update_primary_outputs(az: &Aizome, output: &Output, states: &RenderElementStates) {
    let compare = smithay::backend::renderer::element::default_primary_scanout_output_compare;
    for m in az.windows.values() {
        m.window.with_surfaces(|surface, data| {
            update_surface_primary_scanout_output(surface, output, data, states, compare);
        });
    }
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        layer.with_surfaces(|surface, data| {
            update_surface_primary_scanout_output(surface, output, data, states, compare);
        });
    }
}

/// Frame callbacks for everything visible on an output (and, now and
/// then, for hidden windows so they don't stall forever).
pub fn send_frames(az: &Aizome, output: &Output) {
    let time = az.now();
    let throttle = Some(Duration::from_millis(995));
    let geo = az.output_geometry(output);
    for m in az.windows.values() {
        let visible = az.wm.is_shown(m.id)
            && az.wm.get(m.id).is_some_and(|w| crate::state::from_rect(w.rect.grow(crate::decor::OUTSET)).overlaps(geo));
        if visible {
            m.window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        } else {
            m.window.send_frame(output, time, throttle, surface_primary_scanout_output);
        }
    }
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
    }
    drop(map);
    if let Some(lock) = az.out.get(output).and_then(|o| o.lock_surface.as_ref()) {
        smithay::desktop::utils::send_frames_surface_tree(lock.wl_surface(), output, time, throttle, |_, _| Some(output.clone()));
    }
    if let CursorImageStatus::Surface(s) = &az.cursor_status {
        smithay::desktop::utils::send_frames_surface_tree(s, output, time, throttle, |_, _| Some(output.clone()));
    }
    if let Some(icon) = &az.dnd_icon {
        smithay::desktop::utils::send_frames_surface_tree(icon, output, time, throttle, |_, _| Some(output.clone()));
    }
    for u in &az.unmapped {
        if let Some(t) = u.window.toplevel() {
            smithay::desktop::utils::send_frames_surface_tree(t.wl_surface(), output, time, throttle, |_, _| Some(output.clone()));
        }
    }
}

pub fn take_feedback(az: &Aizome, output: &Output, states: &RenderElementStates) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    for m in az.windows.values() {
        if az.wm.is_shown(m.id) {
            m.window.take_presentation_feedback(
                &mut feedback,
                surface_primary_scanout_output,
                |surface, _| surface_presentation_feedback_flags_from_states(surface, states),
            );
        }
    }
    let map = layer_map_for_output(output);
    for layer in map.layers() {
        layer.take_presentation_feedback(
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| surface_presentation_feedback_flags_from_states(surface, states),
        );
    }
    feedback
}

/// Renders an output into a fresh offscreen buffer and reads it back,
/// upright (the output's transform undone), with or without the pointer.
pub fn capture_with<R, T>(renderer: &mut R, az: &mut Aizome, output: &Output, cursor: bool) -> Result<Capture, String>
where
    R: Renderer + ImportAll + ImportMem + Offscreen<T> + Bind<T> + ExportMem,
    R::TextureId: Clone + Send + 'static,
{
    let mode = output.current_mode().ok_or("the output has no mode")?;
    let size = output.current_transform().transform_size(mode.size);
    let scale = output.current_scale().fractional_scale();
    let elements = output_elements(az, renderer, output, Purpose::Capture { cursor });
    let buffer_size: Size<i32, BufferCoords> = Size::from((size.w, size.h));
    let mut target: T = renderer.create_buffer(Fourcc::Abgr8888, buffer_size).map_err(|e| format!("offscreen buffer: {e:?}"))?;
    let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
    let clear = clear_color(az);
    let rgba = {
        let mut fb = renderer.bind(&mut target).map_err(|e| format!("bind: {e:?}"))?;
        let res = tracker.render_output(renderer, &mut fb, 0, &elements, clear).map_err(|e| format!("render: {e:?}"))?;
        let _ = res.sync.wait();
        let mapping = renderer
            .copy_framebuffer(&fb, Rectangle::from_size(buffer_size), Fourcc::Abgr8888)
            .map_err(|e| format!("read back: {e:?}"))?;
        renderer.map_texture(&mapping).map_err(|e| format!("map: {e:?}"))?.to_vec()
    };
    Ok(Capture { width: size.w as u32, height: size.h as u32, rgba })
}

/// The wl_shm buffers a commit of this surface tree is about to make
/// current (call before `on_commit_buffer_handler`, which takes them).
pub fn fresh_shm_buffers(surface: &WlSurface) -> Vec<WlBuffer> {
    use smithay::backend::renderer::{BufferType, buffer_type};
    use smithay::wayland::compositor::{BufferAssignment, SurfaceAttributes, TraversalAction, with_surface_tree_upward};
    let mut out = Vec::new();
    with_surface_tree_upward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| {
            if let Some(BufferAssignment::NewBuffer(b)) = states.cached_state.get::<SurfaceAttributes>().current().buffer.as_ref()
                && matches!(buffer_type(b), Some(BufferType::Shm))
            {
                out.push(b.clone());
            }
        },
        |_, _, _| true,
    );
    out
}
