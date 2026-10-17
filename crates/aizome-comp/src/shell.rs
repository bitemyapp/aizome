//! Surfaces and shells: commits, xdg toplevels and popups, decorations,
//! layer surfaces and the session lock.
//!
//! The commit handling and popup grab follow anvil's (smithay, MIT
//! licence: Copyright (c) 2017 Victor Berger and Victor Brekenfeld).

use std::time::Instant;

use aizome_ipc::Event;
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::{
    LayerSurface, PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, Window,
    WindowSurfaceType, find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output,
};
use smithay::input::pointer::Focus as PointerFocusMode;
use smithay::input::Seat;
use smithay::output::Output;
use smithay::reexports::calloop::Interest;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{Logical, Point, Rectangle, Serial};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes, add_blocker,
    add_pre_commit_hook, get_parent, is_sync_subsurface, with_states,
};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker};
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};
use smithay::{delegate_compositor, delegate_layer_shell, delegate_session_lock, delegate_xdg_decoration, delegate_xdg_shell};
use tracing::{debug, info, warn};

use crate::decor::DecorCache;
use crate::focus::Focus;
use crate::state::{ClientState, LockState, Mapped, State, Unmapped};
use crate::wm::{Rect, WinId};

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.az.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().expect("client data").compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        // Wait for a dmabuf's fence before applying the commit, so frames
        // never sample unfinished buffers.
        add_pre_commit_hook::<Self, _>(surface, |state, _dh, surface| {
            let dmabuf = with_states(surface, |data| {
                data.cached_state.get::<SurfaceAttributes>().pending().buffer.as_ref().and_then(|a| match a {
                    BufferAssignment::NewBuffer(b) => get_dmabuf(b).cloned().ok(),
                    _ => None,
                })
            });
            if let Some(dmabuf) = dmabuf {
                if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                    if let Some(client) = surface.client() {
                        let res = state.az.lh.insert_source(source, move |_, _, state| {
                            let dh = state.az.dh.clone();
                            state.client_compositor_state(&client).blocker_cleared(state, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                        }
                    }
                }
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend.early_import(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(i) = self.az.unmapped.iter().position(|u| u.window.toplevel().is_some_and(|t| t.wl_surface() == &root)) {
                if &root == surface {
                    self.commit_unmapped(i);
                }
            } else if let Some(id) = self.az.window_for_surface(&root) {
                self.commit_mapped(id, &root == surface);
            }
        }

        self.az.popups.commit(surface);
        if let Some(popup) = self.az.popups.find_popup(surface) {
            if let PopupKind::Xdg(ref p) = popup {
                if !p.is_initial_configure_sent() {
                    let _ = p.send_configure();
                }
            }
            if let Ok(root) = find_popup_root_surface(&popup) {
                self.redraw_for_surface(&root);
            }
        }

        self.commit_layer(surface);
        self.commit_misc(surface);
    }
}

impl State {
    fn commit_unmapped(&mut self, i: usize) {
        let window = self.az.unmapped[i].window.clone();
        let toplevel = window.toplevel().expect("toplevel").clone();
        if !toplevel.is_initial_configure_sent() {
            // The first configure: server-side decorations unless the client
            // said otherwise, the usable area as bounds, and the size its app
            // last had.
            let app_id = toplevel_app_id(&toplevel);
            let out = self.az.active_output();
            let usable = out.as_ref().map(|o| self.az.usable_area(o));
            let remembered = self.az.settings.windows.remember_geometry.then(|| self.az.remembered.get(&app_id)).flatten();
            toplevel.with_pending_state(|s| {
                if let Some(u) = usable {
                    let i = crate::decor::insets(true);
                    s.bounds = Some((u.w - i.left - i.right, u.h - i.top - i.bottom).into());
                }
                if let Some(r) = remembered {
                    s.size = Some((r.w, r.h).into());
                }
            });
            toplevel.send_configure();
            return;
        }
        let has_buffer = with_renderer_surface_state(toplevel.wl_surface(), |s| s.buffer().is_some()).unwrap_or(false);
        if !has_buffer {
            return;
        }
        let u = self.az.unmapped.remove(i);
        u.window.on_commit();
        self.map_window(u.window);
    }

    /// A toplevel drew its first frame: it joins the desktop.
    fn map_window(&mut self, window: Window) {
        let toplevel = window.toplevel().expect("toplevel").clone();
        let surface = toplevel.wl_surface().clone();
        let (app_id, title) = toplevel_ids(&toplevel);
        let parent = toplevel.parent().and_then(|p| self.az.window_for_surface(&p));
        let pid = self.az.dh.get_client(surface.id()).ok().and_then(|c| c.get_credentials(&self.az.dh).ok()).map(|c| c.pid);
        let ssd = toplevel.current_state().decoration_mode == Some(DecorationMode::ServerSide);
        let geo = window.geometry();
        let size = (geo.size.w.max(1), geo.size.h.max(1));
        let id = self.az.wm.insert(&app_id, &title, pid, Rect::new(0, 0, size.0, size.1), parent);
        if let Some(w) = self.az.wm.get_mut(id) {
            w.ssd = ssd;
        }
        crate::actions::place_new_window(self, id, size, parent);
        self.az.windows.insert(
            id,
            Mapped { id, window: window.clone(), decor: DecorCache::default(), resize_anchor: None, requested_size: None, opened: Instant::now() },
        );
        self.az.by_surface.insert(surface.clone(), id);
        info!(id, app_id, title, ssd, "window opened");

        // A new window of the focused app joins its group, when asked for.
        let focused = self.az.wm.focused();
        let same_app = focused.filter(|f| self.az.wm.get(*f).is_some_and(|w| w.app_id == app_id && w.id != id));
        if self.az.settings.windows.stack_new_windows_of_same_app && parent.is_none() {
            if let Some(f) = same_app {
                self.stack(f, id);
            }
        }
        self.az.window_outputs_dirty = true;
        self.focus_window(Some(id));
        self.az.configure(id);
        if let Some(info) = self.az.window_info(id) {
            self.az.ipc.emit(&Event::WindowOpened { window: info });
        }
        crate::protocols::foreign_toplevel::window_opened(self, id);
        self.emit_desks();
        self.az.redraw_window(id);
    }

    fn commit_mapped(&mut self, id: WinId, is_root: bool) {
        let Some(m) = self.az.windows.get(&id) else { return };
        m.window.on_commit();
        if !is_root {
            self.az.redraw_window(id);
            return;
        }
        let window = m.window.clone();
        let toplevel = m.toplevel().clone();
        let has_buffer = with_renderer_surface_state(toplevel.wl_surface(), |s| s.buffer().is_some()).unwrap_or(false);
        if !has_buffer {
            // Unmapped: back to waiting for a first frame.
            self.unmap_window(id);
            self.az.unmapped.push(Unmapped { window, wants_csd: false });
            return;
        }
        // Decoration mode changes.
        let ssd = toplevel.current_state().decoration_mode == Some(DecorationMode::ServerSide);
        if self.az.wm.get(id).is_some_and(|w| w.ssd != ssd) {
            let old = self.az.wm.get(id).map(|w| w.rect).unwrap();
            if let Some(w) = self.az.wm.get_mut(id) {
                w.ssd = ssd;
            }
            self.az.queue_redraw_rect(old);
            self.az.window_changed(id);
        }
        // Size changes: keep the anchored edges put while resizing from the
        // left or top; only the group's showing member sets its size.
        let geo = window.geometry();
        let Some(w) = self.az.wm.get(id) else { return };
        let old = w.rect;
        let size = (geo.size.w.max(1), geo.size.h.max(1));
        if (old.w, old.h) != size && self.az.wm.is_group_active(id) {
            let m = self.az.windows.get_mut(&id).unwrap();
            if m.requested_size == Some(size) {
                m.requested_size = None;
            }
            let mut r = Rect::new(old.x, old.y, size.0, size.1);
            if let Some(a) = m.resize_anchor {
                if a.left {
                    r.x = a.start.right() - size.0;
                }
                if a.top {
                    r.y = a.start.bottom() - size.1;
                }
            }
            let members = self.az.wm.set_rect(id, r);
            for other in members.into_iter().filter(|o| *o != id) {
                self.az.request_size(other, size.0, size.1);
                self.az.configure(other);
            }
            self.az.queue_redraw_rect(old);
            self.az.window_changed(id);
            self.az.window_outputs_dirty = true;
        } else if let Some(m) = self.az.windows.get_mut(&id) {
            if m.requested_size == Some(size) {
                m.requested_size = None;
            }
        }
        self.az.redraw_window(id);
    }

    /// A window went away (destroyed or unmapped).
    pub fn unmap_window(&mut self, id: WinId) {
        let Some(m) = self.az.windows.remove(&id) else { return };
        self.az.by_surface.retain(|_, w| *w != id);
        if let Some(w) = self.az.wm.get(id) {
            if w.placement == crate::wm::Placement::Free && w.parent.is_none() && self.az.settings.windows.remember_geometry {
                let (app, rect) = (w.app_id.clone(), w.rect);
                self.az.remembered.set(&app, rect);
            }
            self.az.queue_redraw_rect(w.rect);
        }
        for o in self.az.outputs.clone() {
            use smithay::desktop::space::SpaceElement;
            m.window.output_leave(&o);
        }
        let r = self.az.wm.remove(id);
        self.az.dirty_windows.remove(&id);
        self.az.anims.tabs.remove(&id);
        self.az.ipc.emit(&Event::WindowClosed { id });
        crate::protocols::foreign_toplevel::window_closed(self, id);
        self.emit_desks();
        crate::input::window_gone(self, id);
        crate::overlays::window_gone(&mut self.az, id);
        info!(id, "window closed");
        self.apply_refocus(r);
        self.az.window_outputs_dirty = true;
    }

    /// Queues redraws for whatever a surface belongs to.
    fn redraw_for_surface(&mut self, root: &WlSurface) {
        if let Some(id) = self.az.window_for_surface(root) {
            self.az.redraw_window(id);
            return;
        }
        let outs: Vec<Output> = self
            .az
            .outputs
            .iter()
            .filter(|o| layer_map_for_output(o).layer_for_surface(root, WindowSurfaceType::ALL).is_some())
            .cloned()
            .collect();
        for o in outs {
            self.az.queue_redraw(&o);
        }
    }

    fn commit_layer(&mut self, surface: &WlSurface) {
        let Some(output) = self
            .az
            .outputs
            .iter()
            .find(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).is_some())
            .cloned()
        else {
            return;
        };
        let initial_sent = with_states(surface, |s| {
            s.data_map.get::<LayerSurfaceData>().map(|d| d.lock().unwrap().initial_configure_sent).unwrap_or(true)
        });
        let mut map = layer_map_for_output(&output);
        let before = map.non_exclusive_zone();
        map.arrange();
        let after = map.non_exclusive_zone();
        if !initial_sent {
            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                layer.layer_surface().send_configure();
            }
        }
        drop(map);
        if before != after {
            // The usable area changed: windows filling it follow.
            self.az.window_outputs_dirty = true;
            crate::outputs::outputs_changed(&mut self.az);
            self.refit_placed_windows(&output);
        }
        self.update_keyboard_focus();
        self.az.queue_redraw(&output);
    }

    /// Fill and split windows on an output re-fit its usable area.
    fn refit_placed_windows(&mut self, output: &Output) {
        let usable = self.az.usable_area(output);
        let out_rect = self.az.output_rect(output);
        let ids: Vec<WinId> = self
            .az
            .wm
            .windows()
            .filter(|w| matches!(w.placement, crate::wm::Placement::Fill | crate::wm::Placement::SplitLeft | crate::wm::Placement::SplitRight))
            .filter(|w| out_rect.intersects(w.rect))
            .map(|w| w.id)
            .collect();
        for id in ids {
            let Some(w) = self.az.wm.get(id) else { continue };
            let insets = self.az.insets_for(id);
            let placement = w.placement;
            let area = match placement {
                crate::wm::Placement::SplitLeft => Rect::new(usable.x, usable.y, usable.w / 2, usable.h),
                crate::wm::Placement::SplitRight => Rect::new(usable.x + usable.w / 2, usable.y, usable.w - usable.w / 2, usable.h),
                _ => usable,
            };
            let r = area.inset(insets);
            let restore = w.restore;
            self.az.wm.set_placement(id, crate::wm::Placement::Free, Rect::default());
            if let Some(w) = self.az.wm.get_mut(id) {
                w.restore = restore;
            }
            self.az.wm.set_placement(id, placement, r);
            for m in self.az.wm.group_members(id) {
                self.az.request_size(m, r.w, r.h);
                self.az.configure(m);
            }
        }
    }

    fn commit_misc(&mut self, surface: &WlSurface) {
        // The pointer's own surface, a drag icon or a lock surface.
        use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
        if let CursorImageStatus::Surface(s) = &self.az.cursor_status {
            if s == surface {
                with_states(surface, |states| {
                    if let Some(attrs) = states.data_map.get::<CursorImageSurfaceData>() {
                        let delta = states.cached_state.get::<SurfaceAttributes>().current().buffer_delta.take();
                        if let Some(d) = delta {
                            attrs.lock().unwrap().hotspot -= d;
                        }
                    }
                });
                if let Some(o) = self.az.active_output() {
                    self.az.queue_redraw(&o);
                }
            }
        }
        if self.az.dnd_icon.as_ref() == Some(surface) {
            if let Some(o) = self.az.active_output() {
                self.az.queue_redraw(&o);
            }
        }
        let lock_out: Option<Output> = self
            .az
            .out
            .iter()
            .find(|(_, o)| o.lock_surface.as_ref().is_some_and(|l| l.wl_surface() == surface))
            .map(|(o, _)| o.clone());
        if let Some(o) = lock_out {
            self.az.queue_redraw(&o);
        }
    }

    /// Fits a popup on its output.
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return };
        let (origin, output) = if let Some(id) = self.az.window_for_surface(&root) {
            let (Some(w), Some(m)) = (self.az.wm.get(id), self.az.windows.get(&id)) else { return };
            let geo = m.window.geometry();
            let origin = Point::<i32, Logical>::from((w.rect.x - geo.loc.x, w.rect.y - geo.loc.y));
            (origin, self.az.output_for_rect(w.rect))
        } else {
            let found = self.az.outputs.iter().find_map(|o| {
                let map = layer_map_for_output(o);
                let layer = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)?;
                let g = map.layer_geometry(layer)?;
                Some((g.loc + o.current_location(), o.clone()))
            });
            match found {
                Some((origin, o)) => (origin, Some(o)),
                None => return,
            }
        };
        let Some(output) = output else { return };
        let mut target: Rectangle<i32, Logical> = self.az.output_geometry(&output);
        target.loc -= origin;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        popup.with_pending_state(|s| {
            s.geometry = s.positioner.get_unconstrained_geometry(target);
        });
    }
}

pub fn toplevel_ids(t: &ToplevelSurface) -> (String, String) {
    with_states(t.wl_surface(), |s| {
        let d = s.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
        (d.app_id.clone().unwrap_or_default(), d.title.clone().unwrap_or_default())
    })
}

fn toplevel_app_id(t: &ToplevelSurface) -> String {
    toplevel_ids(t).0
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.az.xdg_shell
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        self.az.unmapped.push(Unmapped { window, wants_csd: true });
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(e) = self.az.popups.track_popup(PopupKind::from(surface)) {
            warn!("tracking a popup: {e}");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|s| {
            s.geometry = positioner.get_geometry();
            s.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        let Some(id) = self.az.window_for_surface(surface.wl_surface()) else { return };
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        crate::grabs::start_client_move(self, &seat, id, serial);
    }

    fn resize_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial, edges: xdg_toplevel::ResizeEdge) {
        let Some(id) = self.az.window_for_surface(surface.wl_surface()) else { return };
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        crate::grabs::start_client_resize(self, &seat, id, serial, crate::grabs::edges_from_xdg(edges));
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        let Some(seat) = Seat::<State>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        let Ok(mut grab) = self.az.popups.grab_popup(Focus(root), kind, &seat, serial) else { return };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed() && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, PointerFocusMode::Keep);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        match self.az.window_for_surface(surface.wl_surface()) {
            Some(id) => self.set_fill(id, Some(true)),
            None => {
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            self.set_fill(id, Some(false));
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        match self.az.window_for_surface(surface.wl_surface()) {
            Some(id) => self.set_fullscreen(id, Some(true)),
            None => {
                if surface.is_initial_configure_sent() {
                    surface.send_configure();
                }
            }
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            self.set_fullscreen(id, Some(false));
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            self.minimise(id);
        }
    }

    fn show_window_menu(&mut self, surface: ToplevelSurface, _seat: WlSeat, _serial: Serial, location: Point<i32, Logical>) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            let (Some(w), Some(m)) = (self.az.wm.get(id), self.az.windows.get(&id)) else { return };
            let geo = m.window.geometry();
            let p = (w.rect.x - geo.loc.x + location.x, w.rect.y - geo.loc.y + location.y);
            crate::overlays::marking::open(self, id, (p.0 as f64, p.1 as f64), true);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            self.unmap_window(id);
        }
        self.az.unmapped.retain(|u| u.window.toplevel().is_some_and(|t| t != &surface));
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        if let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(surface)) {
            self.redraw_for_surface(&root);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            let (app_id, _) = toplevel_ids(&surface);
            if let Some(w) = self.az.wm.get_mut(id) {
                w.app_id = app_id;
            }
            self.az.window_changed(id);
            self.az.redraw_window(id);
            crate::protocols::foreign_toplevel::window_changed(self, id);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            let (_, title) = toplevel_ids(&surface);
            if let Some(w) = self.az.wm.get_mut(id) {
                w.title = title;
            }
            self.az.window_changed(id);
            self.az.redraw_window(id);
            crate::protocols::foreign_toplevel::window_changed(self, id);
        }
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if let Some(id) = self.az.window_for_surface(surface.wl_surface()) {
            let parent = surface.parent().and_then(|p| self.az.window_for_surface(&p));
            if let Some(w) = self.az.wm.get_mut(id) {
                w.parent = parent;
            }
            self.az.window_changed(id);
        }
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        // Aizome prefers to draw the frame.
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if let Some(u) = self.az.unmapped.iter_mut().find(|u| u.window.toplevel() == Some(&toplevel)) {
            u.wants_csd = false;
        }
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        // Clients that insist draw their own; they get the tab only.
        toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.az.layer_shell
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let output = output.as_ref().and_then(Output::from_resource).or_else(|| self.az.active_output());
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        debug!(namespace, output = output.name(), "layer surface");
        let mut map = layer_map_for_output(&output);
        if let Err(e) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            warn!("mapping a layer surface: {e}");
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&popup);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let found = self.az.outputs.iter().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            layer.map(|l| (o.clone(), l))
        });
        if let Some((output, layer)) = found {
            let mut map = layer_map_for_output(&output);
            map.unmap_layer(&layer);
            drop(map);
            if self.az.layer_focus.as_ref() == Some(surface.wl_surface()) {
                self.az.layer_focus = None;
            }
            crate::outputs::outputs_changed(&mut self.az);
            self.update_keyboard_focus();
            self.az.queue_redraw(&output);
        }
    }
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.az.session_lock
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        info!("locking the session");
        self.az.lock = LockState::Locking(confirmation, Instant::now());
        self.az.queue_redraw_all();
        self.update_keyboard_focus();
        self.az.ipc.emit(&Event::LockStateChanged { locked: true });
    }

    fn unlock(&mut self) {
        info!("unlocking the session");
        self.az.lock = LockState::Unlocked;
        for o in self.az.out.values_mut() {
            o.lock_surface = None;
        }
        self.az.queue_redraw_all();
        self.update_keyboard_focus();
        self.az.ipc.emit(&Event::LockStateChanged { locked: false });
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else { return };
        let geo = self.az.output_geometry(&output);
        surface.with_pending_state(|s| {
            s.size = Some((geo.size.w as u32, geo.size.h as u32).into());
        });
        surface.send_configure();
        if let Some(o) = self.az.out.get_mut(&output) {
            o.lock_surface = Some(surface);
        }
        self.update_keyboard_focus();
        self.az.queue_redraw(&output);
    }
}

impl State {
    /// Confirms a pending lock once every output has drawn a frame since.
    pub fn check_lock(&mut self) {
        let LockState::Locking(_, since) = &self.az.lock else { return };
        let since = *since;
        let drawn = self.az.out.values().all(|o| !o.powered || o.last_frame.is_some_and(|t| t > since));
        if drawn {
            if let LockState::Locking(locker, _) = std::mem::take(&mut self.az.lock) {
                locker.lock();
            }
            self.az.lock = LockState::Locked;
        }
    }
}

delegate_compositor!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_layer_shell!(State);
delegate_session_lock!(State);
