//! wlr-screencopy-unstable-v1 (version 3), for xdg-desktop-portal-wlr's
//! screenshots and screencasts, grim and friends.
//!
//! Frames are offered as shm buffers (XRGB8888, top row first). `copy`
//! renders the output afresh into memory and fills the client's buffer at
//! once; `copy_with_damage` waits for the output's next frame that drew
//! something, as screencasts want.

use std::sync::Mutex;
use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};
use smithay::reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_shm};
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
use smithay::wayland::shm::with_buffer_contents_mut;
use tracing::warn;

use crate::state::State;

use zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1 as Frame;
use zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 as Manager;

pub struct ScreencopyManagerState {
    /// Copies waiting for their output's next damaged frame.
    pending: Vec<Pending>,
}

pub struct GlobalData {
    filter: Box<dyn Fn(&Client) -> bool + Send + Sync>,
}

struct Pending {
    frame: Frame,
    buffer: WlBuffer,
    /// The output's damaged-frame count when asked.
    since: u64,
}

/// What a frame captures.
pub struct FrameData {
    inner: Mutex<FrameInner>,
}

struct FrameInner {
    output: Option<Output>,
    /// In the output's pixels: x, y, w, h.
    region: (i32, i32, i32, i32),
    cursor: bool,
    used: bool,
}

impl ScreencopyManagerState {
    pub fn new<F>(dh: &DisplayHandle, filter: F) -> Self
    where
        F: Fn(&Client) -> bool + Send + Sync + 'static,
    {
        dh.create_global::<State, Manager, _>(3, GlobalData { filter: Box::new(filter) });
        ScreencopyManagerState { pending: Vec::new() }
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
        _: &Manager,
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
            FrameData { inner: Mutex::new(FrameInner { output: None, region: (0, 0, 0, 0), cursor, used: true }) },
        );
        let Some(output) = output else {
            frame.failed();
            return;
        };
        let Some(mode) = output.current_mode() else {
            frame.failed();
            return;
        };
        let size = output.current_transform().transform_size(mode.size);
        let scale = output.current_scale().fractional_scale();
        let region = match region {
            None => (0, 0, size.w, size.h),
            Some((x, y, w, h)) => {
                // Logical, relative to the output → its pixels, clamped.
                let px = |v: i32| (v as f64 * scale).round() as i32;
                let (x0, y0) = (px(x).clamp(0, size.w), px(y).clamp(0, size.h));
                let (x1, y1) = (px(x + w).clamp(0, size.w), px(y + h).clamp(0, size.h));
                (x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
            }
        };
        let (w, h) = (region.2, region.3);
        if w <= 0 || h <= 0 {
            frame.failed();
            return;
        }
        *frame.data::<FrameData>().unwrap().inner.lock().unwrap() =
            FrameInner { output: Some(output), region, cursor, used: false };
        frame.buffer(wl_shm::Format::Xrgb8888, w as u32, h as u32, (w * 4) as u32);
        if frame.version() >= 3 {
            frame.buffer_done();
        }
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
        if with_damage {
            let Some(output) = data.inner.lock().unwrap().output.clone() else { return };
            let since = state.az.out.get(&output).map(|o| o.damaged_frames).unwrap_or(0);
            state.az.screencopy.pending.push(Pending { frame: frame.clone(), buffer, since });
            state.az.queue_redraw(&output);
        } else {
            copy(state, frame, &buffer, false);
        }
    }
}

/// Fills a client's buffer with what the output shows now.
fn copy(state: &mut State, frame: &Frame, buffer: &WlBuffer, damage: bool) {
    let data = frame.data::<FrameData>().unwrap();
    let (output, region, cursor) = {
        let i = data.inner.lock().unwrap();
        (i.output.clone(), i.region, i.cursor)
    };
    let Some(output) = output else {
        frame.failed();
        return;
    };
    let capture = match state.backend.capture(&mut state.az, &output, cursor) {
        Ok(c) => c,
        Err(e) => {
            warn!("screencopy: {e}");
            frame.failed();
            return;
        }
    };
    let (rx, ry, rw, rh) = region;
    let res = with_buffer_contents_mut(buffer, |ptr, len, b| {
        if b.width != rw || b.height != rh || b.stride < rw * 4 {
            return Err("the buffer's size doesn't match");
        }
        if !matches!(b.format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888) {
            return Err("unsupported buffer format");
        }
        let need = b.offset as usize + (b.stride * (rh - 1) + rw * 4) as usize;
        if need > len {
            return Err("the buffer is too small");
        }
        // Safety: the pool is mapped `len` bytes long and the extent fits.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(b.offset as usize), len - b.offset as usize) };
        for y in 0..rh {
            let src_row = ((ry + y) as usize * capture.width as usize + rx as usize) * 4;
            let dst_row = (y * b.stride) as usize;
            for x in 0..rw as usize {
                let s = &capture.rgba[src_row + x * 4..src_row + x * 4 + 4];
                let d = &mut dst[dst_row + x * 4..dst_row + x * 4 + 4];
                // RGBA → BGRA (little-endian XRGB8888).
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
        }
        Ok(())
    });
    match res {
        Ok(Ok(())) => {
            frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
            if damage && frame.version() >= 2 {
                frame.damage(0, 0, rw as u32, rh as u32);
            }
            let t: Duration = state.az.clock.now().into();
            let secs = t.as_secs();
            frame.ready((secs >> 32) as u32, secs as u32, t.subsec_nanos());
        }
        Ok(Err(e)) => {
            warn!("screencopy: {e}");
            frame.failed();
        }
        Err(e) => {
            warn!("screencopy: {e:?}");
            frame.failed();
        }
    }
}

/// Answers copies waiting for damage once their output drew.
pub fn flush_pending(state: &mut State) {
    if state.az.screencopy.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut state.az.screencopy.pending);
    for p in pending {
        if !p.frame.is_alive() {
            continue;
        }
        let output = p.frame.data::<FrameData>().and_then(|d| d.inner.lock().unwrap().output.clone());
        let ready = output.as_ref().and_then(|o| state.az.out.get(o)).is_some_and(|o| o.damaged_frames > p.since);
        if ready {
            copy(state, &p.frame, &p.buffer, true);
        } else {
            state.az.screencopy.pending.push(p);
        }
    }
}
