//! Window-management actions: the model changes, then the windows behind
//! it hear about it (configures, focus, redraws, IPC events).

use aizome_ipc::{DeskInfo, Event, Rect as IpcRect, WindowInfo};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::{LayerSurface, PopupUngrabStrategy, WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};
use tracing::info;

use crate::state::{Aizome, State};
use crate::wm::{DeskId, Placement, Rect, Refocus, Side, WinId};

impl State {
    /// Focuses a window (or nothing): the model, keyboard focus, activated
    /// states, decorations and IPC.
    pub fn focus_window(&mut self, id: Option<WinId>) {
        let prev = self.az.wm.focused();
        self.az.wm.focus(id);
        let now = self.az.wm.focused();
        if prev != now {
            for (w, on) in [(prev, false), (now, true)] {
                if let Some(w) = w {
                    if let Some(m) = self.az.windows.get(&w) {
                        m.window.set_activated(on);
                    }
                    self.az.configure(w);
                    self.az.window_changed(w);
                    self.az.redraw_window(w);
                }
            }
            self.az.ipc.emit(&Event::WindowFocused { id: now });
            // Focus moving to another window closes the open menus.
            let surface = now.and_then(|w| self.az.windows.get(&w)).map(|m| m.surface().clone());
            if self.az.popup_grab.as_ref().is_some_and(|g| g.live() && surface.as_ref() != Some(&g.root)) {
                self.dismiss_popup_grab();
            }
            // And takes the keys from a layer surface clicked earlier (the
            // bar, the Deskbar): a window that opens or is switched to gets
            // what's typed next.
            if now.is_some() {
                self.az.layer_focus = None;
            }
        }
        if let Some(w) = now {
            // Raising changes what overlaps what.
            self.az.redraw_window(w);
            self.az.window_outputs_dirty = true;
        }
        self.update_keyboard_focus();
    }

    pub fn apply_refocus(&mut self, r: Refocus) {
        if let Refocus::To(id) = r {
            self.focus_window(id);
        } else {
            self.update_keyboard_focus();
        }
    }

    /// Shows a window wherever it is (its Desk, unminimised, its app
    /// unhidden) and focuses it.
    pub fn activate_window(&mut self, id: WinId) {
        let before = self.az.wm.current_desk();
        let desk_changed = self.az.wm.reveal(id);
        if desk_changed {
            let after = self.az.wm.current_desk();
            self.desk_switched(before, after);
        }
        self.az.window_changed(id);
        // Asked for (the switcher, a Deskbar row, IPC): the keys go to it,
        // even from a layer surface clicked to ask, and even when it had
        // the focus already.
        self.az.layer_focus = None;
        self.focus_window(Some(id));
        self.az.window_outputs_dirty = true;
        self.az.queue_redraw_all();
    }

    /// Gives keyboard focus to what should have it: the lock surface, an
    /// exclusive layer surface, a layer surface clicked on, or the focused
    /// window.
    pub fn update_keyboard_focus(&mut self) {
        let target = self.az.keyboard_target().map(crate::focus::Focus);
        let Some(keyboard) = self.az.seat.get_keyboard() else { return };
        // A popup grab gives way to whatever else must have the keyboard
        // (the lock, an exclusive layer surface, the switcher): its popups
        // close.
        if self.az.popup_grab.as_ref().is_some_and(|g| g.live() && target.as_ref().map(|f| &f.0) != Some(&g.root)) {
            self.dismiss_popup_grab();
        }
        if keyboard.current_focus() != target {
            tracing::debug!(
                target: "aizome::focus",
                from = ?keyboard.current_focus().map(|f| f.0.id()),
                to = ?target.as_ref().map(|f| f.0.id()),
                grabbed = keyboard.is_grabbed(),
                "keyboard focus"
            );
            keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
        }
    }

    /// Closes the popups of the grab in force (popup_done to each) and
    /// lets go of the keyboard; the pointer grab ends once the client has
    /// destroyed them (`check_popup_grab`).
    pub fn dismiss_popup_grab(&mut self) {
        let Some(g) = self.az.popup_grab.as_mut() else { return };
        if g.dismissed {
            return;
        }
        g.dismissed = true;
        let _ = g.grab.ungrab(PopupUngrabStrategy::All);
        tracing::debug!(target: "aizome::focus", "popup grab dismissed");
        let Some(keyboard) = self.az.seat.get_keyboard() else { return };
        if keyboard.is_grabbed() && self.az.popup_grab.as_ref().is_some_and(|g| g.owns_serial(|s| keyboard.has_grab(s)))
        {
            keyboard.unset_grab(self);
        }
    }

    /// Once a popup grab has ended (its popups destroyed), its keyboard and
    /// pointer grabs go and the keyboard returns to what should have it
    /// (not necessarily the grab's root: a menu of the menu bar opened by a
    /// shortcut gives the keys back to the window that had them).
    pub fn check_popup_grab(&mut self) {
        if !self.az.popup_grab.as_ref().is_some_and(|g| g.grab.has_ended()) {
            return;
        }
        let Some(g) = self.az.popup_grab.take() else { return };
        tracing::debug!(target: "aizome::focus", "popup grab ended");
        // A layer surface clicked to open its menu (the bar, for a menu or
        // a systray panel) doesn't keep the keys once the menu has gone:
        // they go back to the focused window.
        if self.az.layer_focus.as_ref() == Some(&g.root) {
            self.az.layer_focus = None;
        }
        if let Some(keyboard) = self.az.seat.get_keyboard()
            && keyboard.is_grabbed()
            && g.owns_serial(|s| keyboard.has_grab(s))
        {
            keyboard.unset_grab(self);
        }
        if let Some(pointer) = self.az.seat.get_pointer()
            && pointer.is_grabbed()
            && g.owns_serial(|s| pointer.has_grab(s))
        {
            let time = self.az.start.elapsed().as_millis() as u32;
            pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        }
        self.update_keyboard_focus();
    }

    pub fn close_window(&mut self, id: WinId) {
        if let Some(m) = self.az.windows.get(&id) {
            m.toplevel().send_close();
        }
    }

    pub fn minimise(&mut self, id: WinId) {
        let r = self.az.wm.minimise(id);
        self.after_visibility_change(id);
        self.apply_refocus(r);
    }

    pub fn unminimise(&mut self, id: WinId) {
        self.az.wm.unminimise(id);
        self.after_visibility_change(id);
        self.activate_window(id);
    }

    pub fn hide_app(&mut self, app_id: &str) {
        let r = self.az.wm.hide_app(app_id);
        self.after_app_change(app_id);
        self.apply_refocus(r);
    }

    pub fn hide_others(&mut self, app_id: &str) {
        let r = self.az.wm.hide_others(app_id);
        let ids: Vec<WinId> = self.az.wm.windows().map(|w| w.id).collect();
        for id in ids {
            self.az.window_changed(id);
        }
        self.az.window_outputs_dirty = true;
        self.az.queue_redraw_all();
        self.apply_refocus(r);
    }

    pub fn show_app(&mut self, app_id: &str) {
        self.az.wm.show_app(app_id);
        self.after_app_change(app_id);
        // Focus the app's most recent window.
        let next = self
            .az
            .wm
            .mru()
            .iter()
            .copied()
            .find(|w| self.az.wm.get(*w).is_some_and(|w| w.app_id == app_id) && self.az.wm.is_shown(*w));
        if let Some(w) = next {
            self.focus_window(Some(w));
        }
    }

    fn after_app_change(&mut self, app_id: &str) {
        let ids: Vec<WinId> = self.az.wm.windows().filter(|w| w.app_id == app_id).map(|w| w.id).collect();
        for id in ids {
            self.az.window_changed(id);
        }
        self.az.window_outputs_dirty = true;
        self.az.queue_redraw_all();
    }

    fn after_visibility_change(&mut self, id: WinId) {
        self.az.window_changed(id);
        self.az.window_outputs_dirty = true;
        self.az.redraw_window(id);
    }

    /// Fill (the zoom box) on, off or toggled.
    pub fn set_fill(&mut self, id: WinId, on: Option<bool>) {
        let (usable, insets) = self.az.usable_for(id);
        let old = self.az.wm.get(id).map(|w| w.rect);
        if let Some(rect) = self.az.wm.set_fill(id, on, usable, insets) {
            self.after_placement(id, old, rect);
        }
    }

    pub fn set_split(&mut self, id: WinId, side: Side) {
        let (usable, insets) = self.az.usable_for(id);
        let old = self.az.wm.get(id).map(|w| w.rect);
        if let Some(rect) = self.az.wm.set_split(id, side, usable, insets) {
            self.after_placement(id, old, rect);
        }
    }

    pub fn set_fullscreen(&mut self, id: WinId, on: Option<bool>) {
        let Some(w) = self.az.wm.get(id) else { return };
        let out = self.az.output_for_rect(w.rect).map(|o| self.az.output_rect(&o)).unwrap_or(w.rect);
        let old = Some(w.rect);
        if let Some(rect) = self.az.wm.set_fullscreen(id, on, out) {
            self.after_placement(id, old, rect);
            if self.az.wm.get(id).is_some_and(|w| w.fullscreen()) {
                self.focus_window(Some(id));
            }
        }
    }

    fn after_placement(&mut self, id: WinId, old: Option<Rect>, rect: Rect) {
        let members = self.az.wm.group_members(id);
        for m in &members {
            self.az.request_size(*m, rect.w, rect.h);
            self.az.configure(*m);
            self.az.window_changed(*m);
        }
        if let Some(o) = old {
            self.az.queue_redraw_rect(o);
        }
        self.az.queue_redraw_rect(rect);
        self.az.window_outputs_dirty = true;
    }

    /// Moves a window's content to a point (its group with it).
    pub fn move_window(&mut self, id: WinId, x: i32, y: i32) {
        let Some(w) = self.az.wm.get(id) else { return };
        let old = w.rect;
        let rect = Rect::new(x, y, old.w, old.h);
        if rect == old {
            return;
        }
        for m in self.az.wm.set_rect(id, rect) {
            if let Some(w) = self.az.wm.get_mut(m)
                && w.placement != Placement::Free
            {
                w.placement = Placement::Free;
                w.restore = None;
            }
            self.az.window_changed(m);
        }
        self.az.queue_redraw_rect(old);
        self.az.queue_redraw_rect(rect);
        self.az.window_outputs_dirty = true;
    }

    /// Sets a window's content rectangle (asking the client for the size).
    pub fn resize_window(&mut self, id: WinId, rect: Rect) {
        let Some(w) = self.az.wm.get(id) else { return };
        let old = w.rect;
        // Within the sizes the client allows, as a resize by the edges is
        // (Tenki under 640pt wide stacks its words a letter to a line).
        let (min, max) = crate::grabs::size_limits(self, id);
        let rect = Rect::new(rect.x, rect.y, rect.w.clamp(min.0, max.0), rect.h.clamp(min.1, max.1));
        for m in self.az.wm.set_rect(id, rect) {
            if let Some(w) = self.az.wm.get_mut(m) {
                w.placement = Placement::Free;
                w.restore = None;
            }
            self.az.request_size(m, rect.w, rect.h);
            self.az.configure(m);
            self.az.window_changed(m);
        }
        self.az.queue_redraw_rect(old);
        self.az.queue_redraw_rect(rect);
        self.az.window_outputs_dirty = true;
    }

    pub fn set_float(&mut self, id: WinId, on: Option<bool>) {
        for m in self.az.wm.group_members(id) {
            if let Some(w) = self.az.wm.get_mut(m) {
                w.float = on.unwrap_or(!w.float);
            }
            self.az.window_changed(m);
        }
        self.az.redraw_window(id);
    }

    pub fn set_sticky(&mut self, id: WinId, on: Option<bool>) {
        let now = self.az.wm.get(id).is_some_and(|w| w.sticky);
        self.az.wm.set_sticky(id, on.unwrap_or(!now));
        for m in self.az.wm.group_members(id) {
            self.az.window_changed(m);
        }
        self.az.window_outputs_dirty = true;
        self.az.queue_redraw_all();
    }

    /// Sends a window (its group, its dialogs) to a Desk; with `follow`,
    /// goes along.
    pub fn move_to_desk(&mut self, id: WinId, desk: DeskId, follow: bool) {
        let r = self.az.wm.move_to_desk(id, desk);
        for m in self.az.wm.group_members(id) {
            self.az.window_changed(m);
        }
        self.az.window_outputs_dirty = true;
        self.az.queue_redraw_all();
        if follow {
            self.switch_desk(desk);
            self.focus_window(Some(id));
        } else {
            self.apply_refocus(r);
        }
        self.emit_desks();
    }

    /// Switches to a Desk, sliding over the motion time.
    pub fn switch_desk(&mut self, desk: DeskId) {
        let from = self.az.wm.current_desk();
        let Some(r) = self.az.wm.switch_desk(desk) else { return };
        self.desk_switched(from, desk);
        self.apply_refocus(r);
    }

    fn desk_switched(&mut self, from: DeskId, to: DeskId) {
        let fi = self.az.wm.desk_index(from).unwrap_or(0);
        let ti = self.az.wm.desk_index(to).unwrap_or(0);
        let dir = if ti >= fi { 1.0 } else { -1.0 };
        let ms = self.az.motion_ms();
        self.az.anims.start_desk(from, to, dir, ms);
        self.az.window_outputs_dirty = true;
        self.az.queue_redraw_all();
        let ids: Vec<WinId> = self.az.wm.windows().map(|w| w.id).collect();
        for id in ids {
            self.az.window_changed(id);
        }
        if let Some(d) = self.az.wm.desk(to) {
            let ev = Event::DeskChanged { index: ti, id: d.id, name: d.name.clone() };
            self.az.ipc.emit(&ev);
        }
        self.emit_desks();
    }

    pub fn emit_desks(&mut self) {
        let desks = desk_infos(&self.az);
        self.az.ipc.emit(&Event::DesksChanged { desks });
    }

    /// Stacks `b` into `a`'s group, sliding its tab into place.
    pub fn stack(&mut self, a: WinId, b: WinId) -> bool {
        let before: Vec<(WinId, Rect)> =
            self.az.wm.group_members(b).iter().filter_map(|m| self.az.decor_tab_rect(*m).map(|r| (*m, r))).collect();
        let old_b = self.az.wm.get(b).map(|w| w.rect);
        let Some(changed) = self.az.wm.stack(a, b) else { return false };
        let rect = self.az.wm.get(a).map(|w| w.rect).unwrap_or_default();
        for m in &changed {
            self.az.request_size(*m, rect.w, rect.h);
            self.az.configure(*m);
        }
        for m in self.az.wm.group_members(a) {
            self.az.window_changed(m);
        }
        // Tabs slide from where they were.
        let ms = self.az.motion_ms();
        for (m, r0) in before {
            if let Some(r1) = self.az.decor_tab_rect(m) {
                self.az.anims.start_tab(m, (r0.x - r1.x) as f64, (r0.y - r1.y) as f64, ms);
            }
        }
        if let Some(o) = old_b {
            self.az.queue_redraw_rect(o);
        }
        self.az.queue_redraw_rect(rect);
        self.az.window_outputs_dirty = true;
        self.focus_window(Some(b));
        true
    }

    /// Takes a window out of its group, a tab's height down and right (or
    /// at `to`).
    pub fn unstack(&mut self, id: WinId, to: Option<(i32, i32)>) -> bool {
        let Some(w) = self.az.wm.get(id) else { return false };
        let rect = w.rect;
        if !self.az.wm.unstack(id) {
            return false;
        }
        let (x, y) = to.unwrap_or((rect.x + crate::decor::TAB_H + 8, rect.y + crate::decor::TAB_H + 8));
        if let Some(w) = self.az.wm.get_mut(id) {
            w.rect = Rect::new(x, y, rect.w, rect.h);
            w.tab_offset = 0;
        }
        let members = self.az.wm.group_members(rect_owner(&self.az, id, rect));
        for m in members {
            self.az.window_changed(m);
        }
        self.az.window_changed(id);
        self.az.queue_redraw_rect(rect);
        self.az.redraw_window(id);
        self.az.window_outputs_dirty = true;
        self.focus_window(Some(id));
        true
    }

    /// Shows one member of a group (clicking its tab).
    pub fn show_group_member(&mut self, id: WinId) {
        self.az.wm.set_group_active(id);
        self.az.window_outputs_dirty = true;
        for m in self.az.wm.group_members(id) {
            self.az.window_changed(m);
        }
        self.focus_window(Some(id));
        self.az.redraw_window(id);
    }

    /// Sends a window (its group) to the back.
    pub fn lower_window(&mut self, id: WinId) {
        let r = self.az.wm.lower(id);
        self.az.redraw_window(id);
        self.az.window_outputs_dirty = true;
        self.apply_refocus(r);
    }

    /// Stacks the focused window with the one behind it (⌥❖S).
    pub fn stack_with_behind(&mut self, id: WinId) {
        if let Some(behind) = self.az.wm.window_behind(id) {
            self.stack(behind, id);
        }
    }

    /// Runs a program with the session's environment, in a folder.
    pub fn spawn(&mut self, argv: &[String], cwd: Option<std::path::PathBuf>) -> Result<u32, String> {
        crate::session::spawn(&self.az, argv, cwd)
    }

    /// The current Desk's folder, expanded.
    pub fn desk_folder(&self) -> Option<std::path::PathBuf> {
        let d = self.az.wm.desk(self.az.wm.current_desk())?;
        let f = d.folder.as_ref()?;
        let p = aizome_config::expand_home(f);
        p.is_dir().then_some(p)
    }
}

/// Some member of the group the window was in (for change events).
fn rect_owner(az: &Aizome, id: WinId, rect: Rect) -> WinId {
    az.wm.windows().find(|w| w.id != id && w.rect == rect && w.group.is_some()).map(|w| w.id).unwrap_or(id)
}

impl Aizome {
    /// What takes the keyboard from everything else: the lock surface while
    /// locked, else the topmost mapped layer surface on the overlay or top
    /// layer with exclusive keyboard interactivity.
    pub fn exclusive_keyboard_target(&self) -> Option<Option<WlSurface>> {
        // Ctrl+Alt+Del's overlay has the keys while it shows.
        if self.recovery.is_open() {
            return Some(None);
        }
        if self.lock.is_locked() {
            let out = self.active_output();
            let lock = out
                .as_ref()
                .and_then(|o| self.out.get(o))
                .and_then(|o| o.lock_surface.clone())
                .or_else(|| self.out.values().find_map(|o| o.lock_surface.clone()));
            return Some(lock.map(|l| l.wl_surface().clone()));
        }
        // The active output's first, then the others.
        let active = self.active_output();
        let outputs = active.iter().chain(self.outputs.iter().filter(|o| Some(*o) != active.as_ref()));
        for o in outputs {
            let map = layer_map_for_output(o);
            for layer in [Layer::Overlay, Layer::Top] {
                for l in map.layers_on(layer).rev() {
                    if l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive && layer_mapped(l) {
                        return Some(Some(l.wl_surface().clone()));
                    }
                }
            }
        }
        None
    }

    /// The surface that should have the keyboard: the lock surface, an
    /// exclusive layer surface, the root of a popup grab (the grab hands
    /// the keys on to its topmost popup), a layer surface clicked on, or
    /// the focused window.
    pub fn keyboard_target(&self) -> Option<WlSurface> {
        if let Some(t) = self.exclusive_keyboard_target() {
            return t;
        }
        if let Some(g) = &self.popup_grab
            && g.live()
            && !self.overlays.grabs_keyboard()
        {
            return Some(g.root.clone());
        }
        if let Some(l) = &self.layer_focus {
            let still = self.outputs.iter().any(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(l, WindowSurfaceType::TOPLEVEL)
                    .is_some_and(|ls| ls.cached_state().keyboard_interactivity != KeyboardInteractivity::None)
            });
            if still {
                return Some(l.clone());
            }
        }
        if self.overlays.grabs_keyboard() {
            return None;
        }
        let id = self.wm.focused()?;
        if !self.wm.is_shown(id) {
            return None;
        }
        self.windows.get(&id).map(|m| m.surface().clone())
    }

    /// Queues redraws wherever a window (with its decorations) shows.
    pub fn redraw_window(&mut self, id: WinId) {
        if let Some(r) = self.wm.get(id).map(|w| w.rect) {
            self.queue_redraw_rect(r);
        }
    }

    /// The usable area and decoration insets for placing a window.
    pub fn usable_for(&self, id: WinId) -> (Rect, crate::wm::Insets) {
        let rect = self.wm.get(id).map(|w| w.rect).unwrap_or_default();
        // Filling and splitting leave the Deskbar clear.
        let usable = self
            .output_for_rect(rect)
            .map(|o| crate::wm::beside(self.usable_area(&o), self.deskbar_rect(&o)))
            .unwrap_or(rect);
        (usable, self.insets_for(id))
    }

    /// Asks the client for a size (sent with the next configure).
    pub fn request_size(&mut self, id: WinId, w: i32, h: i32) {
        if let Some(m) = self.windows.get_mut(&id) {
            m.requested_size = Some((w, h));
        }
    }

    /// Sends a window its state: size, activated, fill/split/full screen,
    /// decoration mode and bounds.
    pub fn configure(&mut self, id: WinId) {
        let Some(w) = self.wm.get(id) else { return };
        let Some(m) = self.windows.get(&id) else { return };
        let focused = self.wm.focused() == Some(id);
        let placement = w.placement;
        let ssd = w.ssd;
        let rect = w.rect;
        let size = m.requested_size.or_else(|| (placement != Placement::Free).then_some((rect.w, rect.h)));
        let bounds = self.output_for_rect(rect).map(|o| self.usable_area(&o));
        let insets = self.insets_for(id);
        let toplevel = m.toplevel().clone();
        toplevel.with_pending_state(|s| {
            use xdg_toplevel::State as S;
            let set = |s: &mut smithay::wayland::shell::xdg::ToplevelState, st: S, on: bool| {
                if on {
                    s.states.set(st);
                } else {
                    s.states.unset(st);
                }
            };
            set(s, S::Activated, focused);
            set(s, S::Maximized, placement == Placement::Fill);
            set(s, S::Fullscreen, placement == Placement::Fullscreen);
            let left = placement == Placement::SplitLeft;
            let right = placement == Placement::SplitRight;
            set(s, S::TiledLeft, left || placement == Placement::Fill);
            set(s, S::TiledRight, right || placement == Placement::Fill);
            set(s, S::TiledTop, left || right || placement == Placement::Fill);
            set(s, S::TiledBottom, left || right || placement == Placement::Fill);
            if let Some((w, h)) = size {
                s.size = Some((w, h).into());
            }
            if let Some(b) = bounds {
                s.bounds = Some((b.w - insets.left - insets.right, b.h - insets.top - insets.bottom).into());
            }
            s.decoration_mode = Some(if ssd { DecorationMode::ServerSide } else { DecorationMode::ClientSide });
        });
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    /// Where a window's (first) tab is now, for slides.
    pub fn decor_tab_rect(&mut self, id: WinId) -> Option<Rect> {
        let l = self.decor_layout(id)?;
        l.tabs.iter().find(|t| t.win == id).map(|t| t.rect)
    }

    /// Describes a window for IPC.
    pub fn window_info(&self, id: WinId) -> Option<WindowInfo> {
        let w = self.wm.get(id)?;
        let group = self.wm.group_of(id);
        let output = self.output_for_rect(w.rect).map(|o| o.name());
        Some(WindowInfo {
            id,
            app_id: w.app_id.clone(),
            title: w.title.clone(),
            pid: w.pid,
            desk: if w.sticky { None } else { self.wm.desk_index(w.desk) },
            sticky: w.sticky,
            group: group.map(|g| g.id),
            group_members: group.map(|g| g.members.clone()).unwrap_or_default(),
            group_active: self.wm.is_group_active(id),
            focused: self.wm.focused() == Some(id),
            minimised: w.minimised,
            hidden: w.hidden,
            urgent: w.urgent,
            fullscreen: w.fullscreen(),
            fill: w.fill(),
            float: w.float,
            geometry: IpcRect::new(w.rect.x, w.rect.y, w.rect.w, w.rect.h),
            output,
            client_side_decorations: !w.ssd,
            kit_app: crate::cmdkey::is_kit_app(&w.app_id),
            parent: w.parent,
        })
    }

    /// Sends change events for windows marked dirty.
    pub fn flush_ipc_events(&mut self) {
        if self.dirty_windows.is_empty() {
            self.ipc.flush();
            return;
        }
        let dirty: Vec<WinId> = self.dirty_windows.drain().collect();
        for id in dirty {
            if let Some(info) = self.window_info(id) {
                self.ipc.emit(&Event::WindowChanged { window: info });
            }
        }
        self.ipc.flush();
    }
}

pub fn desk_infos(az: &Aizome) -> Vec<DeskInfo> {
    az.wm
        .desks()
        .iter()
        .enumerate()
        .map(|(i, d)| DeskInfo {
            index: i as u32 + 1,
            id: d.id,
            name: d.name.clone(),
            folder: d.folder.clone(),
            windows: az.wm.desk_window_count(d.id) as u32,
            current: d.id == az.wm.current_desk(),
        })
        .collect()
}

/// Brings windows that ended up off every output back onto the first.
pub fn rescue_windows(az: &mut Aizome) {
    // To the main display.
    let Some(first) = az.primary_output() else { return };
    let usable = az.usable_area(&first);
    let ids: Vec<WinId> = az.wm.windows().map(|w| w.id).collect();
    for id in ids {
        let Some(w) = az.wm.get(id) else { continue };
        let on_any = az.outputs.iter().any(|o| az.output_rect(o).intersects(w.rect));
        if !on_any {
            let insets = az.insets_for(id);
            let r = crate::wm::clamp_into(w.rect, usable, insets);
            az.wm.set_rect(id, r);
            az.window_changed(id);
        }
    }
    az.window_outputs_dirty = true;
    az.queue_redraw_all();
}

/// When a window first maps: where it goes, and whether it takes focus.
pub fn place_new_window(state: &mut State, id: WinId, size: (i32, i32), parent: Option<WinId>) {
    let az = &mut state.az;
    let Some(output) = parent
        .and_then(|p| az.wm.get(p).map(|w| w.rect))
        .and_then(|r| az.output_for_rect(r))
        .or_else(|| az.active_output())
    else {
        return;
    };
    let usable = az.usable_area(&output);
    let insets = az.insets_for(id);
    let avoid: Vec<Rect> = az.deskbar_rect(&output).into_iter().collect();
    let app_id = az.wm.get(id).map(|w| w.app_id.clone()).unwrap_or_default();
    let remembered = (parent.is_none() && az.settings.windows.remember_geometry)
        .then(|| az.remembered.get(&app_id))
        .flatten()
        .filter(|r| az.outputs.iter().any(|o| az.usable_area(o).intersects(*r)))
        .filter(|r| !az.wm.windows().any(|w| w.id != id && az.wm.is_shown(w.id) && w.rect.x == r.x && w.rect.y == r.y));
    let rect = match remembered {
        Some(r) => crate::wm::clamp_into(Rect::new(r.x, r.y, size.0, size.1), usable, insets),
        None => az.wm.place(size, usable, &avoid, insets, parent),
    };
    az.wm.set_rect(id, rect);
    info!(id, app_id, ?rect, "placed");
}

/// A layer surface is mapped once it has a buffer (a null one unmaps it).
pub fn layer_mapped(l: &LayerSurface) -> bool {
    with_renderer_surface_state(l.wl_surface(), |s| s.buffer().is_some()).unwrap_or(false)
}
