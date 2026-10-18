//! wlr-screencopy-unstable-v1 (version 3), for xdg-desktop-portal-wlr's
//! screenshots and screencasts (and so browsers' screen sharing),
//! wf-recorder, grim and friends.
//!
//! Every frame is offered as wl_shm (XRGB8888, top row first) and, under
//! GLES, as a linux-dmabuf buffer too (XRGB8888, in any modifier the GPU
//! renders to). A copy renders the output's scene afresh, cropped to the
//! frame's region, into the client's buffer:
//!
//! - a dmabuf is bound as the render target, so the picture never leaves
//!   the GPU; `ready` follows when the GPU's fence signals, without the
//!   compositor waiting for it;
//! - wl_shm is rendered into an offscreen buffer kept for the stream and
//!   copied out row by row (under GLES read back as BGRA, so the GPU does
//!   the swizzle; under pixman straight from the image's memory).
//!
//! A stream is one client's captures of one output region, with or
//! without the pointer. It keeps a damage tracker of its own, so
//! `copy_with_damage` (what screencasts use) answers at once for a
//! stream's first frame, then only once something in its region has
//! changed, with damage events naming what did. An idle screen copies
//! nothing.

use std::any::Any;
use std::os::fd::OwnedFd;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use smithay::backend::allocator::{Buffer as _, Fourcc};
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::utils::{Relocate, RelocateRenderElement};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, BufferType, ExportMem, ImportAll, ImportMem, Offscreen, Renderer, buffer_type};
use smithay::output::Output;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::utils::{Buffer as BufferCoords, Physical, Rectangle, Size, Transform};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::shm::with_buffer_contents_mut;
use tracing::{debug, warn};

use crate::render::{OutputElement, clear_color, scene_elements};
use crate::scene::{Purpose, Scene};
use crate::state::{Aizome, State};
use crate::stats::FrameStats;

use zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1 as Frame;
use zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 as Manager;

/// Streams one manager keeps at most (a screenshot tool capturing many
/// regions in turn would otherwise collect them).
const STREAMS_PER_MANAGER: usize = 4;

/// The damage events one frame sends at most; past that, their bounds.
const MAX_DAMAGE_RECTS: usize = 16;

pub struct ScreencopyManagerState {
    /// Copies waiting to be made: for damage, or for the loop to come round.
    pending: Vec<Pending>,
    streams: Vec<Stream>,
    /// The dmabuf format frames are offered in (`None`: wl_shm only), once
    /// the backend has been asked.
    dmabuf: Option<Option<Fourcc>>,
    /// CPU time per copy (collect, render, read back or submit).
    pub stats: FrameStats,
}

pub struct GlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

struct Pending {
    frame: Frame,
    buffer: WlBuffer,
    with_damage: bool,
    /// The output's damaged-frame count when last found unchanged (`None`:
    /// not looked at yet).
    since: Option<u64>,
}

/// What a frame captures.
pub struct FrameData {
    inner: Mutex<FrameInner>,
}

#[derive(Clone)]
struct FrameInner {
    output: Option<Output>,
    /// In the output's pixels.
    region: Rectangle<i32, Physical>,
    cursor: bool,
    manager: Option<ObjectId>,
    used: bool,
}

/// One client's captures of one region of an output.
pub struct Stream {
    manager: ObjectId,
    output: Output,
    region: Rectangle<i32, Physical>,
    cursor: bool,
    tracker: OutputDamageTracker,
    /// The offscreen buffer wl_shm copies render into, the renderer's own
    /// type (a GLES renderbuffer, a pixman image), kept between frames.
    scratch: Option<Box<dyn Any>>,
}

/// A copy for the backend to make with its renderer.
pub struct Job {
    pub output: Output,
    pub buffer: WlBuffer,
    pub with_damage: bool,
    pub stream: Stream,
    /// The output's scene, as the logic (or the recovery path) draws it.
    pub scene: Scene,
}

/// What a copy came to.
pub enum Outcome {
    /// Nothing in the region changed since the stream's last copy.
    Unchanged,
    /// The buffer holds the frame (once `fence` signals, if there is one).
    Copied { damage: Vec<Rectangle<i32, Physical>>, fence: Option<OwnedFd> },
}

impl ScreencopyManagerState {
    pub fn new<F>(dh: &DisplayHandle, filter: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        dh.create_global::<State, Manager, _>(3, GlobalData { filter: Box::new(filter) });
        ScreencopyManagerState { pending: Vec::new(), streams: Vec::new(), dmabuf: None, stats: FrameStats::default() }
    }

    /// Whether any copy has been made (for `stats`).
    pub fn active(&self) -> bool {
        self.stats.rendered > 0
    }
}

impl GlobalDispatch<Manager, GlobalData> for State {
    fn bind(
        _: &mut State,
        _: &DisplayHandle,
        _: &Client,
        resource: New<Manager>,
        _: &GlobalData,
        init: &mut DataInit<'_, State>,
    ) {
        init.init(resource, ());
    }

    fn can_view(client: Client, data: &GlobalData) -> bool {
        (data.filter)(&client)
    }
}

impl Dispatch<Manager, ()> for State {
    fn request(
        state: &mut State,
        _: &Client,
        manager: &Manager,
        request: zwlr_screencopy_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, State>,
    ) {
        use zwlr_screencopy_manager_v1::Request;
        let (frame, cursor, output, region) = match request {
            Request::CaptureOutput { frame, overlay_cursor, output } => (frame, overlay_cursor != 0, output, None),
            Request::CaptureOutputRegion { frame, overlay_cursor, output, x, y, width, height } => {
                (frame, overlay_cursor != 0, output, Some((x, y, width, height)))
            }
            _ => return,
        };
        let output = Output::from_resource(&output).filter(|o| state.az.outputs.contains(o));
        let frame = init.init(
            frame,
            FrameData {
                inner: Mutex::new(FrameInner {
                    output: None,
                    region: Rectangle::default(),
                    cursor,
                    manager: None,
                    used: true,
                }),
            },
        );
        let Some(output) = output else {
            frame.failed();
            return;
        };
        let Some(region) = region_of(&output, region) else {
            frame.failed();
            return;
        };
        *frame.data::<FrameData>().unwrap().inner.lock().unwrap() =
            FrameInner { output: Some(output), region, cursor, manager: Some(manager.id()), used: false };
        let (w, h) = (region.size.w as u32, region.size.h as u32);
        frame.buffer(wl_shm::Format::Xrgb8888, w, h, w * 4);
        if frame.version() >= 3 {
            if let Some(format) = dmabuf_format(state) {
                frame.linux_dmabuf(format as u32, w, h);
            }
            frame.buffer_done();
        }
    }

    fn destroyed(state: &mut State, _: ClientId, manager: &Manager, _: &()) {
        let id = manager.id();
        state.az.screencopy.streams.retain(|s| s.manager != id);
    }
}

impl Dispatch<Frame, FrameData> for State {
    fn request(
        state: &mut State,
        _: &Client,
        frame: &Frame,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, State>,
    ) {
        use zwlr_screencopy_frame_v1::Request;
        let (buffer, with_damage) = match request {
            Request::Copy { buffer } => (buffer, false),
            Request::CopyWithDamage { buffer } => (buffer, true),
            Request::Destroy => {
                state.az.screencopy.pending.retain(|p| &p.frame != frame);
                return;
            }
            _ => return,
        };
        {
            let mut inner = data.inner.lock().unwrap();
            if inner.used {
                frame.post_error(zwlr_screencopy_frame_v1::Error::AlreadyUsed, "frame already used");
                return;
            }
            inner.used = true;
        }
        // Made at the end of this loop iteration (`flush_pending`): at once
        // for `copy` and a stream's first frame, else once its region
        // changes.
        state.az.screencopy.pending.push(Pending { frame: frame.clone(), buffer, with_damage, since: None });
    }
}

/// A frame's region in the output's pixels (upright: the output's
/// transform undone), from a logical region relative to the output, or
/// all of it.
fn region_of(output: &Output, logical: Option<(i32, i32, i32, i32)>) -> Option<Rectangle<i32, Physical>> {
    let mode = output.current_mode()?;
    let size = output.current_transform().transform_size(mode.size);
    let scale = output.current_scale().fractional_scale();
    let region = match logical {
        None => Rectangle::from_size(size),
        Some((x, y, w, h)) => {
            let px = |v: i32| (v as f64 * scale).round() as i32;
            let (x0, y0) = (px(x).clamp(0, size.w), px(y).clamp(0, size.h));
            let (x1, y1) = (px(x.saturating_add(w)).clamp(0, size.w), px(y.saturating_add(h)).clamp(0, size.h));
            Rectangle::new((x0, y0).into(), ((x1 - x0).max(0), (y1 - y0).max(0)).into())
        }
    };
    (region.size.w > 0 && region.size.h > 0).then_some(region)
}

/// The dmabuf format to offer (asked of the backend once).
fn dmabuf_format(state: &mut State) -> Option<Fourcc> {
    if let Some(f) = state.az.screencopy.dmabuf {
        return f;
    }
    let f = state.backend.screencopy_dmabuf_format();
    debug!("screencopy offers dmabuf: {f:?}");
    state.az.screencopy.dmabuf = Some(f);
    f
}

/// The format a renderer can render dmabufs into for screencopy, if any.
pub fn renderable_dmabuf_format<R: Bind<smithay::backend::allocator::dmabuf::Dmabuf>>(renderer: &R) -> Option<Fourcc> {
    let formats = Bind::<smithay::backend::allocator::dmabuf::Dmabuf>::supported_formats(renderer)?;
    [Fourcc::Xrgb8888, Fourcc::Argb8888].into_iter().find(|f| formats.iter().any(|d| d.code == *f))
}

/// Makes the copies whose time has come (after every loop iteration).
pub fn flush_pending(state: &mut State) {
    if state.az.screencopy.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut state.az.screencopy.pending);
    for mut p in pending {
        if !p.frame.is_alive() {
            continue;
        }
        let Some(info) = p.frame.data::<FrameData>().map(|d| d.inner.lock().unwrap().clone()) else { continue };
        let (Some(output), Some(manager)) = (info.output.clone(), info.manager.clone()) else {
            p.frame.failed();
            continue;
        };
        if !state.az.outputs.contains(&output) {
            p.frame.failed();
            continue;
        }
        let drawn = state.az.out.get(&output).map(|o| o.damaged_frames).unwrap_or(0);
        if p.with_damage && p.since == Some(drawn) {
            // Nothing drew since it was last found unchanged.
            state.az.screencopy.pending.push(p);
            continue;
        }
        let stream = take_stream(state, &manager, &output, info.region, info.cursor);
        let scene = state.scene(&output, Purpose::Capture { cursor: info.cursor });
        let mut job = Job { output, buffer: p.buffer.clone(), with_damage: p.with_damage, stream, scene };
        let started = Instant::now();
        let result = state.backend.screencopy(&mut state.az, &mut job);
        let cpu = started.elapsed();
        state.az.screencopy.streams.push(job.stream);
        match result {
            Ok(Outcome::Unchanged) => {
                p.since = Some(drawn);
                state.az.screencopy.pending.push(p);
            }
            Ok(Outcome::Copied { damage, fence }) => {
                state.az.screencopy.stats.record(cpu, None);
                let damage = if p.with_damage { damage_rects(damage, info.region.size) } else { Vec::new() };
                match fence {
                    Some(fence) => ready_after(state, fence, p.frame, damage),
                    None => ready(state, &p.frame, &damage),
                }
            }
            Err(e) => {
                warn!("screencopy: {e}");
                p.frame.failed();
            }
        }
    }
}

/// The stream for a capture, made if new (taken out while its copy runs).
fn take_stream(
    state: &mut State,
    manager: &ObjectId,
    output: &Output,
    region: Rectangle<i32, Physical>,
    cursor: bool,
) -> Stream {
    let streams = &mut state.az.screencopy.streams;
    // Streams of outputs that went go with them.
    let outputs = &state.az.outputs;
    streams.retain(|s| outputs.contains(&s.output));
    if let Some(i) = streams
        .iter()
        .position(|s| &s.manager == manager && &s.output == output && s.region == region && s.cursor == cursor)
    {
        return streams.remove(i);
    }
    // Make room: the manager's oldest stream goes.
    let mine: Vec<usize> = streams.iter().enumerate().filter(|(_, s)| &s.manager == manager).map(|(i, _)| i).collect();
    if mine.len() >= STREAMS_PER_MANAGER {
        streams.remove(mine[0]);
    }
    let scale = output.current_scale().fractional_scale();
    Stream {
        manager: manager.clone(),
        output: output.clone(),
        region,
        cursor,
        tracker: OutputDamageTracker::new(region.size, scale, Transform::Normal),
        scratch: None,
    }
}

/// Damage rectangles for the client, inside the buffer, and not too many.
fn damage_rects(damage: Vec<Rectangle<i32, Physical>>, size: Size<i32, Physical>) -> Vec<Rectangle<i32, Physical>> {
    let bounds = Rectangle::from_size(size);
    let mut rects: Vec<_> =
        damage.into_iter().filter_map(|r| r.intersection(bounds)).filter(|r| !r.is_empty()).collect();
    if rects.len() > MAX_DAMAGE_RECTS {
        let all = rects.iter().skip(1).fold(rects[0], |a, r| a.merge(*r));
        rects = vec![all];
    }
    if rects.is_empty() {
        rects.push(bounds);
    }
    rects
}

/// Tells the client its frame is in the buffer.
fn ready(state: &State, frame: &Frame, damage: &[Rectangle<i32, Physical>]) {
    if !frame.is_alive() {
        return;
    }
    frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
    if frame.version() >= 2 {
        for r in damage {
            frame.damage(r.loc.x as u32, r.loc.y as u32, r.size.w as u32, r.size.h as u32);
        }
    }
    let t: Duration = state.az.clock.now().into();
    let secs = t.as_secs();
    frame.ready((secs >> 32) as u32, secs as u32, t.subsec_nanos());
}

/// `ready` once the GPU has finished drawing into the client's dmabuf,
/// without waiting for it here.
fn ready_after(state: &mut State, fence: OwnedFd, frame: Frame, damage: Vec<Rectangle<i32, Physical>>) {
    let source = Generic::new(fence, Interest::READ, Mode::OneShot);
    let fallback = (frame.clone(), damage.clone());
    let mut once = Some((frame, damage));
    let res = state.az.lh.insert_source(source, move |_, _, state: &mut State| {
        if let Some((frame, damage)) = once.take() {
            ready(state, &frame, &damage);
        }
        Ok(PostAction::Remove)
    });
    if let Err(e) = res {
        // Can't watch the fence: wait for it here rather than never answer.
        warn!("screencopy: watching the fence: {}", e.error);
        let fence = e.inserted;
        // Safety: polling a sync file we own.
        let mut fd =
            libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(fence.get_ref()), events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut fd, 1, 1000) };
        ready(state, &fallback.0, &fallback.1);
    }
}

/// The output's scene for a stream: its region moved to the origin.
fn stream_elements<R>(renderer: &mut R, job: &Job) -> Vec<RelocateRenderElement<OutputElement<R>>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let offset = job.stream.region.loc;
    let scale = job.output.current_scale().fractional_scale();
    scene_elements(renderer, scale, &job.scene)
        .into_iter()
        .map(|e| RelocateRenderElement::from_element(e, (-offset.x, -offset.y), Relocate::Relative))
        .collect()
}

/// What changed in the stream's region since its last copy: `None` when
/// nothing did and the client only wants changes.
fn stream_damage<E>(job: &mut Job, elements: &[E]) -> Result<Option<Vec<Rectangle<i32, Physical>>>, String>
where
    E: smithay::backend::renderer::element::Element,
{
    let (damage, _) = job.stream.tracker.damage_output(1, elements).map_err(|e| format!("damage: {e:?}"))?;
    let damage = damage.cloned().unwrap_or_default();
    if job.with_damage && damage.is_empty() {
        return Ok(None);
    }
    Ok(Some(damage))
}

/// A copy with a GPU renderer (GLES, or GLES across GPUs): dmabufs are
/// render targets; wl_shm goes through an offscreen buffer of type `T`
/// read back as BGRA.
pub fn copy_gpu<R, T>(renderer: &mut R, az: &mut Aizome, job: &mut Job) -> Result<Outcome, String>
where
    R: Renderer + ImportAll + ImportMem + Offscreen<T> + Bind<smithay::backend::allocator::dmabuf::Dmabuf> + ExportMem,
    R::TextureId: Clone + Send + 'static,
    T: 'static,
{
    let elements = stream_elements(renderer, job);
    let Some(damage) = stream_damage(job, &elements)? else { return Ok(Outcome::Unchanged) };
    let clear = clear_color(az);
    let size = job.stream.region.size;
    match buffer_type(&job.buffer) {
        Some(BufferType::Dma) => {
            let mut dmabuf = get_dmabuf(&job.buffer).map_err(|_| "not a dmabuf")?.clone();
            if dmabuf.size() != Size::<i32, BufferCoords>::from((size.w, size.h)) {
                return Err(format!("the dmabuf is {:?}, the frame {:?}", dmabuf.size(), size));
            }
            let mut fb = renderer.bind(&mut dmabuf).map_err(|e| format!("binding the dmabuf: {e:?}"))?;
            // Every pixel: the client's buffer holds nothing we know.
            let res = job
                .stream
                .tracker
                .render_output(renderer, &mut fb, 0, &elements, clear)
                .map_err(|e| format!("render: {e:?}"))?;
            let fence = match res.sync.export() {
                Some(fd) => Some(fd),
                None => {
                    let _ = res.sync.wait();
                    None
                }
            };
            Ok(Outcome::Copied { damage, fence })
        }
        Some(BufferType::Shm) => {
            let buffer_size: Size<i32, BufferCoords> = (size.w, size.h).into();
            let mut scratch =
                match job.stream.scratch.take().and_then(|s| s.downcast::<(Size<i32, BufferCoords>, T)>().ok()) {
                    Some(s) if s.0 == buffer_size => s,
                    _ => Box::new((
                        buffer_size,
                        renderer
                            .create_buffer(Fourcc::Abgr8888, buffer_size)
                            .map_err(|e| format!("offscreen: {e:?}"))?,
                    )),
                };
            let result = (|| {
                let mut fb = renderer.bind(&mut scratch.1).map_err(|e| format!("bind: {e:?}"))?;
                job.stream
                    .tracker
                    .render_output(renderer, &mut fb, 0, &elements, clear)
                    .map_err(|e| format!("render: {e:?}"))?;
                let all = Rectangle::from_size(buffer_size);
                // BGRA is the client's byte order: the GPU swizzles as it
                // reads (EXT_read_format_bgra), else the CPU does.
                let (mapping, bgra) = match renderer.copy_framebuffer(&fb, all, Fourcc::Argb8888) {
                    Ok(m) => (m, true),
                    Err(e) => {
                        debug!("screencopy: no BGRA read-back ({e:?}): swizzling on the CPU");
                        let m = renderer
                            .copy_framebuffer(&fb, all, Fourcc::Abgr8888)
                            .map_err(|e| format!("read back: {e:?}"))?;
                        (m, false)
                    }
                };
                let pixels = renderer.map_texture(&mapping).map_err(|e| format!("map: {e:?}"))?;
                write_shm(&job.buffer, pixels, size.w as usize * 4, size, bgra)
            })();
            job.stream.scratch = Some(scratch);
            result.map(|()| Outcome::Copied { damage, fence: None })
        }
        _ => Err("the buffer is neither wl_shm nor a dmabuf".into()),
    }
}

/// A copy with pixman: into an image kept for the stream, then into the
/// client's wl_shm memory (pixman can't draw there directly: it reads
/// other clients' wl_shm while compositing, and smithay guards one pool
/// at a time).
pub fn copy_pixman(renderer: &mut PixmanRenderer, az: &mut Aizome, job: &mut Job) -> Result<Outcome, String> {
    use smithay::reexports::pixman::{FormatCode, Image};
    let elements = stream_elements(renderer, job);
    let Some(damage) = stream_damage(job, &elements)? else { return Ok(Outcome::Unchanged) };
    if !matches!(buffer_type(&job.buffer), Some(BufferType::Shm)) {
        return Err("pixman copies only into wl_shm buffers".into());
    }
    let clear = clear_color(az);
    let size = job.stream.region.size;
    let mut image = match job.stream.scratch.take().and_then(|s| s.downcast::<Image<'static, 'static>>().ok()) {
        Some(i) if i.width() == size.w as usize && i.height() == size.h as usize => i,
        _ => Box::new(
            Image::new(FormatCode::A8R8G8B8, size.w as usize, size.h as usize, false)
                .map_err(|_| "an offscreen image")?,
        ),
    };
    let result = (|| {
        let mut fb = renderer.bind(&mut *image).map_err(|e| format!("bind: {e:?}"))?;
        job.stream
            .tracker
            .render_output(renderer, &mut fb, 0, &elements, clear)
            .map_err(|e| format!("render: {e:?}"))?;
        drop(fb);
        let stride = image.stride();
        // Safety: the image owns `stride × height` bytes, alive while `image` is.
        let pixels = unsafe { std::slice::from_raw_parts(image.data() as *const u8, stride * size.h as usize) };
        // A8R8G8B8 in memory is BGRA: the client's byte order.
        write_shm(&job.buffer, pixels, stride, size, true)
    })();
    job.stream.scratch = Some(image);
    result.map(|()| Outcome::Copied { damage, fence: None })
}

/// Copies rows of pixels (BGRA, or RGBA to swizzle) into a client's
/// XRGB8888 or ARGB8888 wl_shm buffer.
fn write_shm(
    buffer: &WlBuffer,
    pixels: &[u8],
    src_stride: usize,
    size: Size<i32, Physical>,
    bgra: bool,
) -> Result<(), String> {
    let (w, h) = (size.w as usize, size.h as usize);
    let res = with_buffer_contents_mut(buffer, |ptr, len, b| {
        if b.width as usize != w || b.height as usize != h || (b.stride as usize) < w * 4 {
            return Err(format!("the buffer is {}×{}, the frame {w}×{h}", b.width, b.height));
        }
        if !matches!(b.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888) {
            return Err(format!("unsupported buffer format {:?}", b.format));
        }
        let (offset, stride) = (b.offset as usize, b.stride as usize);
        if offset + stride * (h - 1) + w * 4 > len || pixels.len() < src_stride * (h - 1) + w * 4 {
            return Err("the buffer is too small".into());
        }
        // Safety: the pool is mapped `len` bytes long and the extent fits.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), len - offset) };
        for y in 0..h {
            let src = &pixels[y * src_stride..y * src_stride + w * 4];
            let row = &mut dst[y * stride..y * stride + w * 4];
            if bgra {
                row.copy_from_slice(src);
            } else {
                for (d, s) in row.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                    *d = [s[2], s[1], s[0], s[3]];
                }
            }
        }
        Ok(())
    });
    match res {
        Ok(r) => r,
        Err(e) => Err(format!("{e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_is_clipped_and_bounded() {
        let size = Size::from((100, 50));
        let r = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        assert_eq!(damage_rects(vec![r(90, 40, 20, 20)], size), vec![r(90, 40, 10, 10)]);
        // Nothing inside: the whole buffer, rather than no damage at all.
        assert_eq!(damage_rects(vec![r(200, 0, 5, 5)], size), vec![r(0, 0, 100, 50)]);
        let many: Vec<_> = (0..40).map(|i| r(i, i, 1, 1)).collect();
        assert_eq!(damage_rects(many, size), vec![r(0, 0, 40, 40)]);
    }
}
