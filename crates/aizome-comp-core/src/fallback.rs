//! The recovery path: what the core does on its own, so the session goes
//! on whatever the logic does.
//!
//! - **Without a logic** (none loaded, or one set aside after panicking
//!   again and again), windows show where the logic last put them (new
//!   ones centred), without decorations; a click focuses and raises one;
//!   keys go to the focused window; IPC lists windows, focuses and closes
//!   them. A reload brings a logic back, which takes the windows in.
//! - **The recovery overlay** (⌃⌥Delete, or `aizomectl action recovery`)
//!   is drawn by the core, above everything, logic or not: it lists the
//!   apps with windows, force-quits one, ends stuck grabs and held keys,
//!   restarts the shell and reloads the compositor's logic.

use std::collections::HashMap;

use aizome_ipc::{Request, Response, WindowInfo};
use aizome_theme::{Rgba, typeface};
use sidestep_ui::kurbo::{Point as KPoint, Size as KSize};
use sidestep_ui::{Appearance, Color, Font, TextStyle};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::input::keyboard::{Keycode, Keysym, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER, Transform};
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};

use crate::focus::Focus;
use crate::input::{Owner, SurfaceAt};
use crate::logic::{AxisEvent, MapInfo, ToplevelRequest};
use crate::scene::{Scene, SceneItem};
use crate::state::State;
use crate::{Rect, WinId};

// ---- windows without a logic -------------------------------------------------

/// A window maps with no logic running: centred on the active output,
/// focused.
pub fn window_mapped(st: &mut State, id: WinId, info: &MapInfo) {
    let usable = st.az.active_output().map(|o| st.az.usable_area(&o)).unwrap_or(Rect::new(0, 0, 1280, 800));
    let (w, h) = info.size;
    let rect = Rect::new(usable.x + (usable.w - w).max(0) / 2, usable.y + (usable.h - h).max(0) / 2, w, h);
    if let Some(m) = st.az.windows.get_mut(&id) {
        m.view.rect = rect;
        m.view.shown = true;
    }
    st.az.view.stacking.push(id);
    focus(st, Some(id));
}

pub fn window_committed(st: &mut State, id: WinId) {
    let Some(m) = st.az.windows.get_mut(&id) else { return };
    let geo = m.window.geometry();
    let old = m.view.rect;
    m.view.rect.w = geo.size.w.max(1);
    m.view.rect.h = geo.size.h.max(1);
    st.az.queue_redraw_rect(old);
    st.redraw_window_now(id);
}

pub fn window_unmapped(st: &mut State, id: WinId) {
    st.az.view.stacking.retain(|w| *w != id);
    if st.az.view.focused == Some(id) {
        let next = st.az.view.stacking.last().copied();
        focus(st, next);
    }
}

pub fn toplevel_request(st: &mut State, id: WinId, req: ToplevelRequest) {
    match req {
        ToplevelRequest::Activate { .. } => focus(st, Some(id)),
        ToplevelRequest::Maximize(_) | ToplevelRequest::Fullscreen(_) => {
            // Acknowledged, not granted.
            if let Some(m) = st.az.windows.get(&id) {
                m.toplevel().send_configure();
            }
        }
        _ => {}
    }
}

/// Focuses a window (or nothing) and raises it.
fn focus(st: &mut State, id: Option<WinId>) {
    let prev = st.az.view.focused;
    if let Some(id) = id {
        st.az.view.stacking.retain(|w| *w != id);
        st.az.view.stacking.push(id);
    }
    st.az.view.focused = id;
    for (w, on) in [(prev, false), (id, true)] {
        if let Some(m) = w.and_then(|w| st.az.windows.get(&w)) {
            m.window.set_activated(on);
            m.toplevel().send_pending_configure();
        }
    }
    if id.is_some() {
        st.az.layer_focus = None;
    }
    st.az.window_outputs_dirty = true;
    st.update_keyboard_focus();
    st.az.queue_redraw_all();
}

/// What IPC can still do without a logic.
pub fn request(st: &mut State, req: &Request) -> Result<Response, String> {
    match req {
        Request::Windows => {
            let windows = st.az.view.stacking.iter().filter_map(|id| window_info(st, *id)).collect();
            Ok(Response::Windows { windows })
        }
        Request::FocusedWindow => {
            Ok(Response::Window { window: st.az.view.focused.and_then(|id| window_info(st, id)) })
        }
        Request::Focus { id } => {
            if !st.az.windows.contains_key(id) {
                return Err(format!("no window {id}"));
            }
            focus(st, Some(*id));
            Ok(Response::Ok)
        }
        Request::Close { id } => {
            let id = id.or(st.az.view.focused).ok_or("no window has focus")?;
            let m = st.az.windows.get(&id).ok_or_else(|| format!("no window {id}"))?;
            m.toplevel().send_close();
            Ok(Response::Ok)
        }
        _ => Err("the compositor's logic isn't running (it failed to load or kept failing); \
                  `aizomectl reload` loads it again, `aizomectl stats` says why"
            .into()),
    }
}

fn window_info(st: &State, id: WinId) -> Option<WindowInfo> {
    let m = st.az.windows.get(&id)?;
    let (app_id, title) = crate::shell::toplevel_ids(m.toplevel());
    let pid = st.az.dh.get_client(m.surface().id()).ok().and_then(|c| c.get_credentials(&st.az.dh).ok()).map(|c| c.pid);
    let r = m.view.rect;
    Some(WindowInfo {
        id,
        app_id,
        title,
        pid,
        focused: st.az.view.focused == Some(id),
        geometry: aizome_ipc::Rect::new(r.x, r.y, r.w, r.h),
        desk: m.view.desk,
        minimised: m.view.minimised,
        hidden: m.view.hidden,
        ..Default::default()
    })
}

use smithay::reexports::wayland_server::Resource;

/// The scene without a logic: popups, layer surfaces, and windows where
/// they last were, bare.
pub fn scene(st: &mut State, output: &Output, scale: f64, out: &mut Scene) {
    let geo = st.az.output_geometry(output);
    let origin = (geo.loc.x as f64, geo.loc.y as f64);
    crate::render::popup_items(&st.az, output, scale, false, out);
    crate::render::layer_items(output, Layer::Overlay, scale, out);
    crate::render::layer_items(output, Layer::Top, scale, out);
    for id in st.az.view.stacking.iter().rev() {
        let Some(m) = st.az.windows.get(id) else { continue };
        if !m.view.shown {
            continue;
        }
        let r = m.view.rect;
        if let Some(item) = crate::render::window_item(&st.az, *id, (r.x as f64, r.y as f64), origin, scale, 1.0) {
            out.push(item);
        }
    }
    crate::render::layer_items(output, Layer::Bottom, scale, out);
    crate::render::layer_items(output, Layer::Background, scale, out);
}

/// The client surface under a point: the lock while locked, else popups,
/// overlay and top layers, windows (where the logic last put them), bottom
/// and background layers.
pub fn surface_at(st: &mut State, pos: Point<f64, Logical>) -> Option<SurfaceAt> {
    let output = st.az.output_at(pos)?;
    if st.az.lock.is_locked() {
        return crate::input::lock_under(&st.az, &output, pos);
    }
    let out_loc = st.az.output_geometry(&output).loc.to_f64();
    if let Some(u) = st.az.popup_under(pos) {
        return Some(u);
    }
    if let Some(u) = crate::input::layer_under(&output, &[Layer::Overlay, Layer::Top], pos, out_loc) {
        return Some(u);
    }
    for id in st.az.view.stacking.iter().rev() {
        let Some(m) = st.az.windows.get(id) else { continue };
        if !m.view.shown {
            continue;
        }
        let geo = m.window.geometry();
        let origin =
            Point::<f64, Logical>::from(((m.view.rect.x - geo.loc.x) as f64, (m.view.rect.y - geo.loc.y) as f64));
        if let Some((s, loc)) =
            m.window.surface_under(pos - origin, WindowSurfaceType::TOPLEVEL | WindowSurfaceType::SUBSURFACE)
        {
            return Some((Focus(s), loc.to_f64() + origin, Owner::Window(*id)));
        }
    }
    crate::input::layer_under(&output, &[Layer::Bottom, Layer::Background], pos, out_loc)
}

pub fn pointer_motion(st: &mut State, pos: Point<f64, Logical>, time: u32) {
    let Some(pointer) = st.az.seat.get_pointer() else { return };
    let under = surface_at(st, pos);
    if !pointer.is_grabbed() {
        st.az.cursor_override = if under.is_none() { Some(smithay::input::pointer::CursorIcon::Default) } else { None };
    }
    let focus = under.map(|(f, o, _)| (f, o));
    pointer.motion(st, focus, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
    pointer.frame(st);
}

pub fn pointer_button(st: &mut State, button: u32, pressed: bool, time: u32) {
    let Some(pointer) = st.az.seat.get_pointer() else { return };
    let serial = SERIAL_COUNTER.next_serial();
    if pressed {
        let pos = pointer.current_location();
        let under = surface_at(st, pos);
        st.press_outside_menus(under.as_ref().map(|u| &u.0.0), serial, time);
        if !pointer.is_grabbed() {
            focus_under(st, under);
        }
    }
    let state = if pressed {
        smithay::backend::input::ButtonState::Pressed
    } else {
        smithay::backend::input::ButtonState::Released
    };
    pointer.button(st, &ButtonEvent { button, state, serial, time });
    pointer.frame(st);
}

pub fn pointer_axis(st: &mut State, a: &AxisEvent) {
    let Some(pointer) = st.az.seat.get_pointer() else { return };
    let mut frame = AxisFrame::new(a.time).source(a.source);
    use smithay::backend::input::Axis;
    if a.h != 0.0 {
        frame = frame.value(Axis::Horizontal, a.h);
    }
    if a.v != 0.0 {
        frame = frame.value(Axis::Vertical, a.v);
    }
    pointer.axis(st, frame);
    pointer.frame(st);
}

/// A touch or click focuses what's under it.
pub fn focus_at(st: &mut State, pos: Point<f64, Logical>) {
    let under = surface_at(st, pos);
    focus_under(st, under);
}

fn focus_under(st: &mut State, under: Option<SurfaceAt>) {
    match under {
        Some((_, _, Owner::Window(id))) => {
            if st.az.view.focused != Some(id) || st.az.view.stacking.last() != Some(&id) {
                focus(st, Some(id));
            }
        }
        Some((f, _, Owner::Layer)) => {
            let interactive = st.az.outputs.iter().find_map(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(&f.0, WindowSurfaceType::ALL)
                    .map(|l| l.cached_state().keyboard_interactivity)
            });
            if interactive == Some(KeyboardInteractivity::OnDemand) {
                st.az.layer_focus = Some(f.0.clone());
                st.update_keyboard_focus();
            }
        }
        _ => {}
    }
}

// ---- the recovery overlay ------------------------------------------------------

/// A label by its text, size (quarter points), colour and buffer scale.
type LabelKey = (String, u32, [u8; 4], u32);

/// The recovery overlay, while it's open.
#[derive(Default)]
pub struct Recovery {
    /// The app picked (an index into `apps`).
    pub selected: usize,
    /// The last thing it did, said under the list.
    pub status: String,
    /// What it lists, from the last drawing: app id, pids, window count.
    apps: Vec<(String, Vec<i32>, usize)>,
    /// Labels it has drawn, by text and colour, and its pieces' identities.
    labels: HashMap<LabelKey, (MemoryRenderBuffer, i32, i32)>,
    pieces: Vec<(Id, CommitCounter, [u8; 4])>,
}

pub fn toggle_recovery(st: &mut State) {
    if st.az.recovery.take().is_some() {
        st.az.queue_redraw_all();
        st.update_keyboard_focus();
        return;
    }
    // Whatever the logic was doing with the pointer, it lets go.
    st.end_logic_grabs();
    st.az.recovery = Some(Recovery::default());
    refresh_apps(st);
    st.update_keyboard_focus();
    st.az.queue_redraw_all();
}

/// The apps with windows: their ids, processes and window counts.
fn refresh_apps(st: &mut State) {
    let mut apps: Vec<(String, Vec<i32>, usize)> = Vec::new();
    let mut ids: Vec<WinId> = st.az.windows.keys().copied().collect();
    ids.sort();
    for id in ids {
        let Some(m) = st.az.windows.get(&id) else { continue };
        let (app_id, _) = crate::shell::toplevel_ids(m.toplevel());
        let pid =
            st.az.dh.get_client(m.surface().id()).ok().and_then(|c| c.get_credentials(&st.az.dh).ok()).map(|c| c.pid);
        let name = if app_id.is_empty() { "(no app id)".to_string() } else { app_id };
        match apps.iter_mut().find(|a| a.0 == name) {
            Some(a) => {
                a.2 += 1;
                if let Some(p) = pid
                    && !a.1.contains(&p)
                {
                    a.1.push(p);
                }
            }
            None => apps.push((name, pid.into_iter().collect(), 1)),
        }
    }
    if let Some(r) = st.az.recovery.as_mut() {
        r.apps = apps;
        r.selected = r.selected.min(r.apps.len().saturating_sub(1));
    }
}

/// Keys while the overlay is open: ↑↓ pick an app, K force-quits it, I
/// ends stuck grabs and held keys, S restarts the shell, R reloads the
/// logic, Esc closes.
pub(crate) fn recovery_key(st: &mut State, keycode: Keycode, pressed: bool, syms: &[Keysym]) -> crate::input::Decision {
    if !pressed {
        return crate::input::Decision::Swallow;
    }
    st.az.suppressed_keys.insert(keycode);
    let is = |k: u32| syms.iter().any(|s| s.raw() == k);
    let n = st.az.recovery.as_ref().map(|r| r.apps.len()).unwrap_or(0);
    let mut status = None;
    if is(keysyms::KEY_Escape) {
        toggle_recovery(st);
        return crate::input::Decision::Swallow;
    } else if is(keysyms::KEY_Down) {
        if let Some(r) = st.az.recovery.as_mut() {
            r.selected = (r.selected + 1).min(n.saturating_sub(1));
        }
    } else if is(keysyms::KEY_Up) {
        if let Some(r) = st.az.recovery.as_mut() {
            r.selected = r.selected.saturating_sub(1);
        }
    } else if is(keysyms::KEY_k) || is(keysyms::KEY_K) {
        let pick = st.az.recovery.as_ref().and_then(|r| r.apps.get(r.selected).cloned());
        if let Some((app, pids, _)) = pick {
            for p in &pids {
                // Safety: kill() with a pid from the client's credentials.
                unsafe {
                    libc::kill(*p, libc::SIGKILL);
                }
            }
            status =
                Some(format!("Force-quit {app} ({} process{})", pids.len(), if pids.len() == 1 { "" } else { "es" }));
        }
    } else if is(keysyms::KEY_i) || is(keysyms::KEY_I) {
        reset_input(st);
        status = Some("Ended grabs and let go of held keys".into());
    } else if is(keysyms::KEY_s) || is(keysyms::KEY_S) {
        status = Some(match st.restart_shell() {
            Ok(_) => "Restarting the shell".into(),
            Err(e) => format!("The shell: {e}"),
        });
    } else if is(keysyms::KEY_r) || is(keysyms::KEY_R) {
        st.lg.pending_reload = true;
        status = Some("Reloading Aizome".into());
    }
    if let Some(s) = status
        && let Some(r) = st.az.recovery.as_mut()
    {
        r.status = s;
    }
    refresh_apps(st);
    st.az.queue_redraw_all();
    crate::input::Decision::Swallow
}

/// A click while the overlay is open closes it when outside the panel.
pub fn recovery_button(st: &mut State, _button: u32, pressed: bool) -> bool {
    if pressed {
        toggle_recovery(st);
    }
    true
}

/// Ends whatever holds the pointer or keyboard: grabs, popup grabs, keys
/// swallowed or translated, the logic's overlays.
pub fn reset_input(st: &mut State) {
    st.end_logic_grabs();
    st.dismiss_popup_grab();
    let time = st.az.start.elapsed().as_millis() as u32;
    if let Some(p) = st.az.seat.get_pointer()
        && p.is_grabbed()
    {
        p.unset_grab(st, SERIAL_COUNTER.next_serial(), time);
    }
    if let Some(k) = st.az.seat.get_keyboard()
        && k.is_grabbed()
    {
        k.unset_grab(st);
    }
    st.az.suppressed_keys.clear();
    st.az.translated_keys.clear();
    st.lg_call((), |vt, s| (vt.quiesce)(s));
    st.update_keyboard_focus();
}

/// The overlay, drawn by the core on the active output.
pub fn recovery_items(st: &mut State, output: &Output, scale: f64, out: &mut Scene) {
    if st.az.recovery.is_none() || st.az.active_output().as_ref() != Some(output) {
        return;
    }
    let geo = st.az.output_geometry(output);
    let p = st.az.palette;
    let logic = if st.lg.healthy() {
        format!("Window manager: running (logic {})", st.lg.build().unwrap_or("?"))
    } else if st.lg.build().is_some() {
        "Window manager: failed".to_string()
    } else {
        "Window manager: not loaded — windows show without decorations".to_string()
    };
    let error = st.lg.last_error.clone();
    let Some(r) = st.az.recovery.as_mut() else { return };
    let mut lines: Vec<(String, f64, Rgba, bool)> =
        vec![("Aizome recovery".into(), 18.0, p.text, false), (logic, 12.0, p.text2, false)];
    if let Some(e) = error {
        lines.push((format!("Last failure: {}", truncate(&e, 110)), 11.0, p.red, false));
    }
    lines.push((String::new(), 8.0, p.text, false));
    if r.apps.is_empty() {
        lines.push(("No windows".into(), 13.0, p.text3, false));
    }
    for (i, (app, pids, n)) in r.apps.iter().enumerate() {
        let pids: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
        let text = format!("{app}   {n} window{}   pid {}", if *n == 1 { "" } else { "s" }, pids.join(", "));
        lines.push((text, 13.0, if i == r.selected { p.on_selection } else { p.text }, i == r.selected));
    }
    lines.push((String::new(), 8.0, p.text, false));
    lines.push((r.status.clone(), 12.0, p.text2, false));
    lines.push((
        "↑↓ pick   K force-quit   I reset input   S restart shell   R reload Aizome   Esc close".into(),
        11.0,
        p.text3,
        false,
    ));

    let buffer_scale = scale.ceil().max(1.0) as u32;
    let mut rendered = Vec::new();
    let mut width: f64 = 360.0;
    for (text, size, color, selected) in &lines {
        if text.is_empty() {
            rendered.push((None, *size as i32 + 2, *selected));
            continue;
        }
        let key = (text.clone(), (*size * 4.0) as u32, [color.r, color.g, color.b, color.a], buffer_scale);
        let entry = r.labels.entry(key).or_insert_with(|| label(text, *size, *color, buffer_scale));
        width = width.max(entry.1 as f64);
        rendered.push((Some(entry.clone()), entry.2 + 6, *selected));
    }
    let pad = 20.0;
    let w = (width + 2.0 * pad).min(geo.size.w as f64 - 40.0);
    let h: f64 = rendered.iter().map(|(_, h, _)| *h as f64).sum::<f64>() + 2.0 * pad;
    let x0 = geo.loc.x as f64 + ((geo.size.w as f64 - w) / 2.0).round();
    let y0 = geo.loc.y as f64 + ((geo.size.h as f64 - h) / 3.0).round();
    let origin = (geo.loc.x as f64, geo.loc.y as f64);

    // Text first (front), then the panel's pieces.
    let mut y = y0 + pad;
    let mut pieces: Vec<(Rect, Rgba)> = Vec::new();
    for (label, h, selected) in &rendered {
        if *selected {
            pieces.push((Rect::new((x0 + 8.0) as i32, y as i32 - 2, (w - 16.0) as i32, *h), p.selection));
        }
        if let Some((buffer, lw, lh)) = label {
            let loc: Point<i32, Physical> =
                Point::<f64, Logical>::from((x0 + pad - origin.0, y - origin.1)).to_physical_precise_round(scale);
            out.push(SceneItem::Memory {
                buffer: buffer.clone(),
                loc: loc.to_f64(),
                alpha: None,
                src: None,
                size: Some((*lw, *lh).into()),
                kind: Kind::Unspecified,
            });
        }
        y += *h as f64;
    }
    let panel = Rect::new(x0 as i32, y0 as i32, w as i32, h as i32);
    let mut all = vec![
        (Rect::new(panel.x + 3, panel.y + 3, panel.w, panel.h), p.slab),
        (panel, p.bevel_lo),
        (Rect::new(panel.x + 1, panel.y + 1, panel.w - 2, panel.h - 2), p.raised),
        (Rect::new(panel.x + 1, panel.y + 1, panel.w - 2, 2), p.red),
    ];
    all.extend(pieces);
    while r.pieces.len() < all.len() {
        r.pieces.push((Id::new(), CommitCounter::default(), [0; 4]));
    }
    let mut solids = Vec::new();
    for (i, (rect, color)) in all.iter().enumerate() {
        let slot = &mut r.pieces[i];
        let c8 = [color.r, color.g, color.b, color.a];
        if slot.2 != c8 {
            slot.2 = c8;
            slot.1.increment();
        }
        let geo: Rectangle<i32, Physical> = Rectangle::new(
            Point::<f64, Logical>::from((rect.x as f64 - origin.0, rect.y as f64 - origin.1))
                .to_physical_precise_round(scale),
            smithay::utils::Size::<f64, Logical>::from((rect.w as f64, rect.h as f64)).to_physical_precise_round(scale),
        );
        solids.push(SceneItem::Solid(SolidColorRenderElement::new(
            slot.0.clone(),
            geo,
            slot.1,
            color.to_premultiplied_f32(),
            Kind::Unspecified,
        )));
    }
    out.extend(solids.into_iter().rev());
}

fn truncate(s: &str, n: usize) -> String {
    let mut c: Vec<char> = s.chars().take(n + 1).collect();
    if c.len() > n {
        c.truncate(n);
        c.push('…');
    }
    c.into_iter().collect()
}

/// A line of text in Archivo (or the system's sans), rasterized by
/// Sidestep: its buffer and size in points.
fn label(text: &str, size: f64, color: Rgba, scale: u32) -> (MemoryRenderBuffer, i32, i32) {
    let font = Font::named(typeface::UI, size).unwrap_or_else(|| Font::system(size)).with_weight(560.0);
    let style = TextStyle::new(font, Color::rgba8(color.r, color.g, color.b, color.a));
    let m = sidestep_ui::measure(text, &style);
    let (w, h) = (m.width.ceil().max(1.0), m.height.ceil().max(1.0));
    let appearance = if color.luminance() > 0.5 { Appearance::DARK } else { Appearance::LIGHT };
    let image = sidestep_ui::offscreen::render(KSize::new(w, h), scale as f64, appearance, |c| {
        c.draw_label(text, &style, KPoint::ZERO);
    });
    let (pw, ph) = image.pixel_size();
    let px = image.to_premultiplied_rgba8().unwrap_or_else(|| vec![0; pw as usize * ph as usize * 4]);
    let buffer = MemoryRenderBuffer::from_slice(
        &px,
        Fourcc::Abgr8888,
        (pw as i32, ph as i32),
        scale as i32,
        Transform::Normal,
        None,
    );
    (buffer, w as i32, h as i32)
}
