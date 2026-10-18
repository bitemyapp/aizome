//! Input: pointer, keyboard, touch and gestures, from libinput, winit or
//! IPC (synthetic events for tests), all through the same paths.
//!
//! The pointer belongs to the compositor over tabs, frame edges and
//! overlays, and to clients over their surfaces; a press on a window
//! focuses and raises it. Keys go through the binding table, then the
//! command-key translation, then to the focused client.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event as _, GestureBeginEvent, GestureEndEvent,
    GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
};
use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::input::keyboard::{Keycode, Keysym, KeysymHandle, ModifiersState, keysyms};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent};
use smithay::input::touch::{DownEvent, UpEvent};
use smithay::output::Output;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Serial};
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
use smithay::wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint};
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, Layer};

use crate::bindings::{Action, Mods};
use crate::decor::{Edges, Hit};
use crate::focus::Focus;
use crate::state::State;
use crate::wm::WinId;

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// What the compositor tracks about pointer interaction.
#[derive(Default)]
pub struct Interactive {
    /// A cursor the compositor shows instead of the client's (resize
    /// arrows over edges, the arrow over tabs).
    pub cursor_override: Option<&'static str>,
    pub hover: Option<Hit>,
    /// The last press on a tab, for double-clicks.
    pub last_tab_click: Option<(Instant, WinId, (f64, f64))>,
    /// A tab being dragged: where it would stack.
    pub drag: Option<TabDrag>,
    /// Buttons whose presses the seat was given and whose releases it
    /// hasn't been yet. The seat ends its grabs (a client's implicit
    /// grab, a drag and drop) only when it holds no button, so every press
    /// it sees must be followed by its release, whatever overlay comes up
    /// in between.
    pub forwarded_buttons: HashSet<u32>,
}

#[derive(Clone, Debug)]
pub struct TabDrag {
    /// The window being dragged.
    pub win: WinId,
    /// The window whose tab it hovers, to stack with on release.
    pub target: Option<WinId>,
    /// Where the dropped tab would go (the slot), in global points.
    pub slot: Option<crate::wm::Rect>,
}

/// What lies under a point.
pub enum Under {
    /// A client surface, with its origin in global points.
    Surface(Focus, Point<f64, Logical>, Owner),
    /// A window's decorations.
    Decor(WinId, Hit),
    /// Nothing: the desktop colour.
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Window(WinId),
    Layer,
    Lock,
}

/// What a key does.
pub enum KeyDecision {
    Forward,
    /// Forward with these modifiers as the client sees them (❖ → Ctrl).
    Translate(ModifiersState),
    Swallow,
    Act(Action),
}

impl State {
    /// Feeds an event from a backend.
    pub fn process_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        match event {
            InputEvent::Keyboard { event } => {
                let pressed = event.state() == KeyState::Pressed;
                self.keyboard_key(event.key_code().raw().saturating_sub(8), pressed, event.time_msec());
            }
            InputEvent::PointerMotion { event } => {
                self.pointer_motion_relative(event.delta(), event.delta_unaccel(), event.time(), event.time_msec());
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let Some(output) = self.az.outputs.first().cloned() else { return };
                let geo = self.az.output_geometry(&output);
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_motion_absolute(pos, event.time_msec());
            }
            InputEvent::PointerButton { event } => {
                let pressed = event.state() == ButtonState::Pressed;
                self.pointer_button(event.button_code(), pressed, event.time_msec());
            }
            InputEvent::PointerAxis { event } => self.pointer_axis::<B>(&event),
            InputEvent::GestureSwipeBegin { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_swipe_begin(
                    self,
                    &smithay::input::pointer::GestureSwipeBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        fingers: event.fingers(),
                    },
                );
            }
            InputEvent::GestureSwipeUpdate { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_swipe_update(
                    self,
                    &smithay::input::pointer::GestureSwipeUpdateEvent { time: event.time_msec(), delta: event.delta() },
                );
            }
            InputEvent::GestureSwipeEnd { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_swipe_end(
                    self,
                    &smithay::input::pointer::GestureSwipeEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: event.cancelled(),
                    },
                );
            }
            InputEvent::GesturePinchBegin { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_pinch_begin(
                    self,
                    &smithay::input::pointer::GesturePinchBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        fingers: event.fingers(),
                    },
                );
            }
            InputEvent::GesturePinchUpdate { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_pinch_update(
                    self,
                    &smithay::input::pointer::GesturePinchUpdateEvent {
                        time: event.time_msec(),
                        delta: event.delta(),
                        scale: event.scale(),
                        rotation: event.rotation(),
                    },
                );
            }
            InputEvent::GesturePinchEnd { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_pinch_end(
                    self,
                    &smithay::input::pointer::GesturePinchEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: event.cancelled(),
                    },
                );
            }
            InputEvent::GestureHoldBegin { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_hold_begin(
                    self,
                    &smithay::input::pointer::GestureHoldBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        fingers: event.fingers(),
                    },
                );
            }
            InputEvent::GestureHoldEnd { event } => {
                let Some(pointer) = self.az.seat.get_pointer() else { return };
                pointer.gesture_hold_end(
                    self,
                    &smithay::input::pointer::GestureHoldEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: event.cancelled(),
                    },
                );
            }
            InputEvent::TouchDown { event } => self.touch_down::<B>(&event),
            InputEvent::TouchUp { event } => {
                let Some(touch) = self.az.seat.get_touch() else { return };
                touch.up(
                    self,
                    &UpEvent { slot: event.slot(), serial: SERIAL_COUNTER.next_serial(), time: event.time_msec() },
                );
            }
            InputEvent::TouchMotion { event } => {
                let Some(touch) = self.az.seat.get_touch() else { return };
                let Some(output) = self.az.outputs.first().cloned() else { return };
                let geo = self.az.output_geometry(&output);
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                let under = self.surface_focus_at(pos);
                touch.motion(
                    self,
                    under,
                    &smithay::input::touch::MotionEvent { slot: event.slot(), location: pos, time: event.time_msec() },
                );
            }
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.az.seat.get_touch() {
                    touch.frame(self);
                }
            }
            InputEvent::TouchCancel { .. } => {
                if let Some(touch) = self.az.seat.get_touch() {
                    touch.cancel(self);
                }
            }
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } => {}
            _ => {}
        }
    }

    fn touch_down<B: InputBackend>(&mut self, event: &B::TouchDownEvent) {
        self.note_activity();
        let Some(touch) = self.az.seat.get_touch() else { return };
        let Some(output) = self.az.outputs.first().cloned() else { return };
        let geo = self.az.output_geometry(&output);
        let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
        if let Under::Surface(_, _, Owner::Window(id)) = self.under(pos)
            && self.az.wm.focused() != Some(id)
        {
            self.focus_window(Some(id));
        }
        let under = self.surface_focus_at(pos);
        touch.down(
            self,
            under,
            &DownEvent {
                slot: event.slot(),
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
            },
        );
    }

    fn pointer_axis<B: InputBackend>(&mut self, event: &B::PointerAxisEvent) {
        self.note_activity();
        let source = event.source();
        let natural = match source {
            AxisSource::Finger => self.az.settings.input.touchpad.natural_scroll,
            _ => self.az.settings.input.pointer.natural_scroll,
        };
        // Natural scrolling turns it round; `scroll_factor` scales it.
        let factor = match source {
            AxisSource::Finger => self.az.settings.input.touchpad.scroll_factor(),
            _ => self.az.settings.input.pointer.scroll_factor(),
        };
        let sign = if natural { -factor } else { factor };
        let h = event
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0);
        let v = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0);
        let mut frame = AxisFrame::new(event.time_msec()).source(source);
        if h != 0.0 {
            frame = frame.value(Axis::Horizontal, h * sign);
            if let Some(d) = event.amount_v120(Axis::Horizontal) {
                frame = frame.v120(Axis::Horizontal, (d * sign) as i32);
            }
        }
        if v != 0.0 {
            frame = frame.value(Axis::Vertical, v * sign);
            if let Some(d) = event.amount_v120(Axis::Vertical) {
                frame = frame.v120(Axis::Vertical, (d * sign) as i32);
            }
        }
        if source == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }
        if crate::overlays::scroll(self, h, v) {
            return;
        }
        let Some(pointer) = self.az.seat.get_pointer() else { return };
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// Scrolls by points (synthetic input).
    pub fn pointer_scroll(&mut self, dx: f64, dy: f64, time: u32) {
        self.note_activity();
        let f = self.az.settings.input.pointer.scroll_factor();
        let (dx, dy) = (dx * f, dy * f);
        let Some(pointer) = self.az.seat.get_pointer() else { return };
        let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);
        if dx != 0.0 {
            frame = frame.value(Axis::Horizontal, dx);
        }
        if dy != 0.0 {
            frame = frame.value(Axis::Vertical, dy);
        }
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    // ---- pointer -------------------------------------------------------

    pub fn pointer_motion_relative(
        &mut self,
        delta: Point<f64, Logical>,
        delta_unaccel: Point<f64, Logical>,
        utime: u64,
        time: u32,
    ) {
        let Some(pointer) = self.az.seat.get_pointer() else { return };
        let pos = pointer.current_location();
        // A locked pointer stays put; relative motion still goes out.
        let focus = pointer.current_focus();
        let mut locked = false;
        if let Some(f) = &focus {
            with_pointer_constraint(&f.0, &pointer, |c| {
                if let Some(c) = c
                    && c.is_active()
                    && let PointerConstraint::Locked(_) = &*c
                {
                    locked = true;
                }
            });
        }
        let surface_loc = pointer.current_focus().and_then(|f| self.az.surface_origin(&f.0));
        pointer.relative_motion(self, focus.zip(surface_loc), &RelativeMotionEvent { delta, delta_unaccel, utime });
        if locked {
            pointer.frame(self);
            return;
        }
        self.pointer_motion_absolute(pos + delta, time);
    }

    pub fn pointer_motion_absolute(&mut self, pos: Point<f64, Logical>, time: u32) {
        self.note_activity();
        let pos = self.clamp_to_outputs(pos);
        let old = self.az.pointer_pos;
        self.az.pointer_pos = pos;
        let Some(pointer) = self.az.seat.get_pointer() else { return };
        let under = self.under(pos);
        // The compositor's cursor over decorations; the client's over
        // surfaces.
        let (hover, override_cursor) = match &under {
            Under::Decor(_, hit @ Hit::Edge(_, e)) => (Some(*hit), Some(e.cursor_name())),
            Under::Decor(_, hit) => (Some(*hit), Some("default")),
            Under::Nothing => (None, Some("default")),
            Under::Surface(..) => (None, None),
        };
        if !pointer.is_grabbed() {
            self.az.interactive.hover = hover;
            self.az.interactive.cursor_override = override_cursor;
        }
        crate::overlays::pointer_moved(self, pos);
        let focus = match under {
            Under::Surface(f, origin, _) => Some((f, origin)),
            _ => None,
        };
        // Focus follows the mouse, when asked.
        if self.az.settings.windows.focus_follows == aizome_config::FocusFollows::Mouse
            && !pointer.is_grabbed()
            && let Some((f, _)) = &focus
            && let Some(id) = self.az.window_for_root(&f.0)
            && self.az.wm.focused() != Some(id)
        {
            let r = self.az.wm.get(id).map(|w| w.rect);
            self.focus_window_no_raise(id);
            if let Some(r) = r {
                self.az.queue_redraw_rect(r);
            }
        }
        pointer.motion(self, focus, &MotionEvent { location: pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(self);
        // Activate a pointer constraint the client asked for.
        if let Some(f) = pointer.current_focus() {
            with_pointer_constraint(&f.0, &pointer, |c| {
                if let Some(c) = c
                    && !c.is_active()
                {
                    c.activate();
                }
            });
        }
        for p in [old, pos] {
            if let Some(o) = self.az.output_at(p) {
                self.az.queue_redraw(&o);
            }
        }
    }

    fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.az.outputs.iter().any(|o| self.az.output_geometry(o).to_f64().contains(pos)) {
            return pos;
        }
        // Nearest point on any output.
        let mut best = pos;
        let mut best_d = f64::MAX;
        for o in &self.az.outputs {
            let g = self.az.output_geometry(o).to_f64();
            let x = pos.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0);
            let y = pos.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0);
            let d = (x - pos.x).powi(2) + (y - pos.y).powi(2);
            if d < best_d {
                best_d = d;
                best = Point::from((x, y));
            }
        }
        best
    }

    pub fn pointer_button(&mut self, button: u32, pressed: bool, time: u32) {
        self.note_activity();
        let Some(pointer) = self.az.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let state = if pressed { ButtonState::Pressed } else { ButtonState::Released };
        if pressed {
            self.press_outside_menus(&pointer, serial, time);
        }
        if pressed && !pointer.is_grabbed() {
            let pos = pointer.current_location();
            if crate::overlays::pointer_button(self, button, true, pos) {
                return;
            }
            if self.press_binding(button, pos, serial, time) {
                return;
            }
            match self.under(pos) {
                Under::Decor(id, hit) => {
                    self.decor_press(id, hit, button, pos, serial, time);
                    return;
                }
                Under::Surface(f, _, Owner::Window(id)) => {
                    if self.az.wm.focused() != Some(id) || !self.az.wm.is_shown(id) {
                        self.focus_window(Some(id));
                    } else {
                        self.az.wm.raise(id);
                        self.az.redraw_window(id);
                    }
                    if self.az.layer_focus.take().is_some() {
                        self.update_keyboard_focus();
                    }
                    let _ = f;
                }
                Under::Surface(f, _, Owner::Layer) => {
                    let interactive = self.az.outputs.iter().find_map(|o| {
                        layer_map_for_output(o)
                            .layer_for_surface(&f.0, WindowSurfaceType::ALL)
                            .map(|l| l.cached_state().keyboard_interactivity)
                    });
                    if interactive == Some(KeyboardInteractivity::OnDemand) {
                        self.az.layer_focus = Some(f.0.clone());
                        self.update_keyboard_focus();
                    }
                }
                _ => {}
            }
        } else if !pressed
            && !self.az.interactive.forwarded_buttons.contains(&button)
            && !pointer.is_grabbed()
            && crate::overlays::pointer_button(self, button, false, pointer.current_location())
        {
            // A release for an overlay that took its press. One whose
            // press the seat saw goes to the seat, or its grabs (and the
            // pointer) stay stuck.
            return;
        }
        if pressed {
            self.az.interactive.forwarded_buttons.insert(button);
        } else {
            self.az.interactive.forwarded_buttons.remove(&button);
        }
        pointer.button(self, &ButtonEvent { button, state, serial, time });
        pointer.frame(self);
    }

    /// A press outside the client whose menus are open (a grabbing popup)
    /// closes them and then does what a press there does (focusing a
    /// window, say), rather than only closing them.
    fn press_outside_menus(
        &mut self,
        pointer: &smithay::input::pointer::PointerHandle<State>,
        serial: Serial,
        time: u32,
    ) {
        let Some(root) = self.az.popup_grab.as_ref().filter(|g| g.live()).map(|g| g.root.clone()) else { return };
        let pos = pointer.current_location();
        let same_client = match self.under(pos) {
            Under::Surface(f, _, _) => smithay::reexports::wayland_server::Resource::id(&f.0)
                .same_client_as(&smithay::reexports::wayland_server::Resource::id(&root)),
            _ => false,
        };
        if same_client {
            return;
        }
        self.dismiss_popup_grab();
        if pointer.is_grabbed() && self.az.popup_grab.as_ref().is_some_and(|g| g.owns_serial(|s| pointer.has_grab(s))) {
            pointer.unset_grab(self, serial, time);
        }
    }

    /// Pointer bindings: ⌃❖-drag moves, ⌃❖-right-drag resizes,
    /// ❖-right-click is the shell's app menu.
    fn press_binding(&mut self, button: u32, pos: Point<f64, Logical>, serial: Serial, time: u32) -> bool {
        let Some(kb) = self.az.seat.get_keyboard() else { return false };
        let mods = Mods::from_state(&kb.modifier_state());
        if !mods.logo || self.az.lock.is_locked() {
            return false;
        }
        let target = match self.under(pos) {
            Under::Surface(_, _, Owner::Window(id)) | Under::Decor(id, _) => Some(id),
            _ => None,
        };
        if mods.ctrl && !mods.alt {
            let Some(id) = target else { return false };
            self.focus_window(Some(id));
            match button {
                BTN_LEFT => crate::grabs::start_move(self, id, pos, BTN_LEFT, serial, time, false),
                BTN_RIGHT => {
                    let edges =
                        self.az.wm.get(id).map(|w| nearest_corner(w.rect, pos)).unwrap_or(Edges::BOTTOM | Edges::RIGHT);
                    crate::grabs::start_resize(self, id, edges, pos, BTN_RIGHT, serial, time);
                }
                _ => return false,
            }
            return true;
        }
        if !mods.ctrl && !mods.alt && !mods.shift && button == BTN_RIGHT {
            let output = self.az.output_at(pos).map(|o| o.name());
            self.az.ipc.emit(&aizome_ipc::Event::Binding {
                action: "app-menu".into(),
                pointer: Some((pos.x, pos.y)),
                output,
            });
            return true;
        }
        false
    }

    fn decor_press(&mut self, id: WinId, hit: Hit, button: u32, pos: Point<f64, Logical>, serial: Serial, time: u32) {
        let shift = self.az.seat.get_keyboard().is_some_and(|k| k.modifier_state().shift);
        match (button, hit) {
            (BTN_LEFT, Hit::Close(w)) | (BTN_LEFT, Hit::Zoom(w)) => {
                self.focus_window(Some(w));
                crate::grabs::start_box_click(self, hit, pos, serial, time);
            }
            (BTN_LEFT, Hit::Tab(w)) => {
                // A double click fills (the zoom box), parks, or does nothing,
                // as `[windows] tab_double_click` says.
                let now = Instant::now();
                let double = self.az.interactive.last_tab_click.is_some_and(|(t, last, p)| {
                    last == w
                        && now.duration_since(t) < Duration::from_millis(400)
                        && (p.0 - pos.x).abs() < 4.0
                        && (p.1 - pos.y).abs() < 4.0
                });
                if double {
                    self.az.interactive.last_tab_click = None;
                    match self.az.settings.windows.tab_double_click() {
                        aizome_config::TabDoubleClick::Fill => self.set_fill(w, None),
                        aizome_config::TabDoubleClick::Minimise => self.minimise(w),
                        aizome_config::TabDoubleClick::Nothing => {}
                    }
                    return;
                }
                self.az.interactive.last_tab_click = Some((now, w, (pos.x, pos.y)));
                // A background tab shows on release (a click), or tears out
                // of its group when dragged (the move grab decides).
                if self.az.wm.is_group_active(w) {
                    self.focus_window(Some(w));
                }
                if shift {
                    crate::grabs::start_tab_slide(self, w, pos, serial, time);
                } else {
                    crate::grabs::start_move(self, w, pos, BTN_LEFT, serial, time, true);
                }
            }
            (BTN_LEFT, Hit::Edge(w, edges)) => {
                self.focus_window(Some(w));
                crate::grabs::start_resize(self, w, edges, pos, BTN_LEFT, serial, time);
            }
            (BTN_MIDDLE, Hit::Tab(w)) | (BTN_MIDDLE, Hit::Close(w)) | (BTN_MIDDLE, Hit::Zoom(w)) => {
                self.lower_window(w);
            }
            (BTN_RIGHT, Hit::Tab(w)) | (BTN_RIGHT, Hit::Close(w)) | (BTN_RIGHT, Hit::Zoom(w)) => {
                self.focus_window(Some(w));
                crate::overlays::marking::press(self, w, pos, serial, time);
            }
            _ => {
                let _ = id;
            }
        }
    }

    /// What lies under a point, top to bottom.
    pub fn under(&mut self, pos: Point<f64, Logical>) -> Under {
        let Some(output) = self.az.output_at(pos) else { return Under::Nothing };
        let out_loc = self.az.output_geometry(&output).loc.to_f64();
        if self.az.lock.is_locked() {
            if let Some(lock) = self.az.out.get(&output).and_then(|o| o.lock_surface.clone())
                && let Some((s, loc)) = smithay::desktop::utils::under_from_surface_tree(
                    lock.wl_surface(),
                    pos - out_loc,
                    (0, 0),
                    WindowSurfaceType::ALL,
                )
            {
                return Under::Surface(Focus(s), loc.to_f64() + out_loc, Owner::Lock);
            }
            return Under::Nothing;
        }
        // Popups (menus) are above everything.
        if let Some(u) = self.az.popup_under(pos) {
            return u;
        }
        if let Some(u) = layer_under(&output, &[Layer::Overlay], pos, out_loc) {
            return u;
        }
        let shown = self.az.wm.shown_stacking();
        let top_fullscreen = shown
            .last()
            .copied()
            .filter(|id| self.az.wm.get(*id).is_some_and(|w| w.fullscreen()) && self.az.wm.focused() == Some(*id));
        if let Some(id) = top_fullscreen
            && let Some(u) = self.window_under(id, pos)
        {
            return u;
        }
        if let Some(u) = layer_under(&output, &[Layer::Top], pos, out_loc) {
            return u;
        }
        for id in shown.iter().rev() {
            if Some(*id) == top_fullscreen {
                continue;
            }
            if let Some(u) = self.window_under(*id, pos) {
                return u;
            }
        }
        if let Some(u) = layer_under(&output, &[Layer::Bottom, Layer::Background], pos, out_loc) {
            return u;
        }
        Under::Nothing
    }

    fn window_under(&mut self, id: WinId, pos: Point<f64, Logical>) -> Option<Under> {
        let (rect, fullscreen) = self.az.wm.get(id).map(|w| (w.rect, w.fullscreen()))?;
        let m = self.az.windows.get(&id)?;
        let geo = m.window.geometry();
        let origin = Point::<f64, Logical>::from(((rect.x - geo.loc.x) as f64, (rect.y - geo.loc.y) as f64));
        if !fullscreen
            && let Some(layout) = self.az.decor_layout(id)
            && let Some(hit) = crate::decor::hit(&layout, pos.x, pos.y)
        {
            return Some(Under::Decor(id, hit));
        }
        let m = self.az.windows.get(&id)?;
        if let Some((s, loc)) =
            m.window.surface_under(pos - origin, WindowSurfaceType::TOPLEVEL | WindowSurfaceType::SUBSURFACE)
        {
            return Some(Under::Surface(Focus(s), loc.to_f64() + origin, Owner::Window(id)));
        }
        None
    }

    fn surface_focus_at(&mut self, pos: Point<f64, Logical>) -> Option<(Focus, Point<f64, Logical>)> {
        match self.under(pos) {
            Under::Surface(f, o, _) => Some((f, o)),
            _ => None,
        }
    }

    /// Focus without raising (focus follows mouse).
    fn focus_window_no_raise(&mut self, id: WinId) {
        let stacking: Vec<WinId> = self.az.wm.stacking().to_vec();
        self.focus_window(Some(id));
        // Put the stacking back as it was.
        let _ = stacking;
    }

    // ---- keyboard ------------------------------------------------------

    /// A key from a keyboard (or IPC): slow keys, then sticky keys, then
    /// bindings and the focused surface.
    pub fn keyboard_key(&mut self, code: u32, pressed: bool, time: u32) {
        self.note_activity();
        if let Some(ms) = self.az.settings.input.keyboard.slow_keys() {
            if pressed {
                if self.az.access.slow.contains_key(&code) {
                    return;
                }
                // It counts once held this long.
                let token = self
                    .az
                    .lh
                    .insert_source(
                        Timer::from_duration(Duration::from_millis(ms as u64)),
                        move |_, _, state: &mut State| {
                            if let Some(k) = state.az.access.slow.get_mut(&code) {
                                k.0 = None;
                                let time = state.az.start.elapsed().as_millis() as u32;
                                state.sticky_key(code, true, time);
                            }
                            TimeoutAction::Drop
                        },
                    )
                    .ok();
                self.az.access.slow.insert(code, (token,));
                return;
            }
            // Let go too soon: it never happened.
            if let Some((Some(token),)) = self.az.access.slow.remove(&code) {
                self.az.lh.remove(token);
                return;
            }
        } else if !self.az.access.slow.is_empty() {
            for (_, (t,)) in self.az.access.slow.drain() {
                if let Some(t) = t {
                    self.az.lh.remove(t);
                }
            }
        }
        self.sticky_key(code, pressed, time);
    }

    /// Sticky keys: a modifier tapped alone latches until the next key
    /// (tapped again, it lets go); that key then comes with it.
    fn sticky_key(&mut self, code: u32, pressed: bool, time: u32) {
        if !self.az.settings.input.keyboard.sticky_keys() {
            self.az.access.latched.clear();
            self.az.access.alone = None;
            return self.key_through(code, pressed, time);
        }
        if MODIFIER_KEYS.contains(&code) {
            if pressed {
                self.az.access.alone = Some(code);
            } else if self.az.access.alone.take() == Some(code) {
                let latched = &mut self.az.access.latched;
                match latched.iter().position(|m| *m == code) {
                    Some(i) => {
                        latched.remove(i);
                    }
                    None => latched.push(code),
                }
            }
            return self.key_through(code, pressed, time);
        }
        self.az.access.alone = None;
        if pressed && !self.az.access.latched.is_empty() {
            let mods = std::mem::take(&mut self.az.access.latched);
            for m in &mods {
                self.key_through(*m, true, time);
            }
            self.key_through(code, true, time);
            self.az.access.applied.push((code, mods));
            return;
        }
        if !pressed && let Some(i) = self.az.access.applied.iter().position(|(k, _)| *k == code) {
            let (_, mods) = self.az.access.applied.remove(i);
            self.key_through(code, false, time);
            for m in mods.iter().rev() {
                self.key_through(*m, false, time);
            }
            return;
        }
        self.key_through(code, pressed, time);
    }

    fn key_through(&mut self, code: u32, pressed: bool, time: u32) {
        let Some(keyboard) = self.az.seat.get_keyboard() else { return };
        let keycode = Keycode::new(code + 8);
        let state = if pressed { KeyState::Pressed } else { KeyState::Released };
        let serial = SERIAL_COUNTER.next_serial();
        let (decision, mods_changed) = keyboard.input_intercept(self, keycode, state, |state, mods, handle| {
            state.decide_key(keycode, pressed, mods, &handle)
        });
        match decision {
            KeyDecision::Forward => keyboard.input_forward(self, keycode, state, serial, time, mods_changed),
            KeyDecision::Translate(fake) => {
                let real = keyboard.modifier_state();
                keyboard.set_modifier_state(fake);
                keyboard.input_forward(self, keycode, state, serial, time, true);
                keyboard.set_modifier_state(real);
            }
            KeyDecision::Swallow => {}
            KeyDecision::Act(action) => self.run_action(action),
        }
    }

    fn decide_key(
        &mut self,
        keycode: Keycode,
        pressed: bool,
        mods: &ModifiersState,
        handle: &KeysymHandle<'_>,
    ) -> KeyDecision {
        // A swallowed press swallows its release.
        if !pressed && self.az.suppressed_keys.remove(&keycode) {
            return KeyDecision::Swallow;
        }
        let mut syms: Vec<Keysym> = Vec::new();
        if let Some(s) = handle.raw_latin_sym_or_raw_current_sym() {
            syms.push(s);
        }
        syms.push(handle.modified_sym());
        syms.extend(handle.raw_syms());
        let m = Mods::from_state(mods);

        // Overlays (switcher, overview, marking menu, area picker) take
        // keys first, releases of ❖ included.
        if let Some(d) = crate::overlays::key(self, &syms, m, pressed, handle.modified_sym().key_char()) {
            if pressed && let KeyDecision::Swallow | KeyDecision::Act(_) = d {
                self.az.suppressed_keys.insert(keycode);
            }
            return d;
        }
        if !pressed {
            if self.az.translated_keys.remove(&keycode)
                && let Some(fake) = self.translation(mods)
            {
                return KeyDecision::Translate(fake);
            }
            return KeyDecision::Forward;
        }
        // VT switching works even when locked.
        let vt = syms.iter().find_map(|s| {
            let r = s.raw();
            (keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12)
                .contains(&r)
                .then(|| (r - keysyms::KEY_XF86Switch_VT_1 + 1) as i32)
        });
        if let Some(vt) = vt {
            self.az.suppressed_keys.insert(keycode);
            return KeyDecision::Act(Action::ChangeVt(vt));
        }
        if self.az.lock.is_locked() {
            return KeyDecision::Forward;
        }
        let inhibited = self
            .az
            .seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .and_then(|f| self.az.seat.keyboard_shortcuts_inhibitor_for_surface(&f.0))
            .is_some_and(|i| i.is_active());
        if let Some(action) = self.az.bindings.find(m, &syms).cloned() {
            // A bare F10 opens the menu bar only over Aizome's apps and the
            // desktop; other apps (mc, htop) keep it.
            let f10 = m == Mods::default() && syms.iter().any(|s| s.raw() == keysyms::KEY_F10);
            let other_app = self
                .az
                .wm
                .focused()
                .filter(|w| self.az.wm.is_shown(*w))
                .and_then(|w| self.az.wm.get(w))
                .is_some_and(|w| !crate::cmdkey::is_kit_app(&w.app_id));
            let keep_for_app = f10 && other_app && matches!(&action, Action::Shell(a) if a == "menu-bar");
            // Inhibitors (VMs, remote desktops) take everything but VT keys.
            if !inhibited && !keep_for_app {
                self.az.suppressed_keys.insert(keycode);
                return KeyDecision::Act(action);
            }
        }
        if !inhibited && let Some(fake) = self.translation(mods) {
            self.az.translated_keys.insert(keycode);
            return KeyDecision::Translate(fake);
        }
        KeyDecision::Forward
    }

    /// The modifiers a non-kit client sees for a ❖ chord: Ctrl (and Shift
    /// in terminals) instead of the logo key.
    fn translation(&self, mods: &ModifiersState) -> Option<ModifiersState> {
        if !mods.logo || !self.az.settings.keys.command_key_for_all_apps {
            return None;
        }
        let id = self.az.wm.focused()?;
        let w = self.az.wm.get(id)?;
        let focused_is_window =
            self.az.seat.get_keyboard()?.current_focus().is_some_and(|f| self.az.window_for_root(&f.0) == Some(id));
        if !focused_is_window || crate::cmdkey::is_kit_app(&w.app_id) {
            return None;
        }
        let terminal = self.az.settings.keys.terminals.iter().any(|t| t == &w.app_id);
        let mut fake = *mods;
        fake.logo = false;
        fake.ctrl = true;
        if terminal {
            fake.shift = true;
        }
        Some(fake)
    }

    /// Runs a binding's action.
    pub fn run_action(&mut self, action: Action) {
        let focused = self.az.wm.focused();
        match action {
            Action::GoToDesk(n) => {
                if let Some(d) = self.az.wm.desk_at(n) {
                    self.switch_desk(d);
                }
            }
            Action::TakeToDesk(n) => {
                if let (Some(d), Some(id)) = (self.az.wm.desk_at(n), focused) {
                    self.move_to_desk(id, d, true);
                }
            }
            Action::PrevDesk | Action::NextDesk => {
                let d = self.az.wm.neighbour_desk(action == Action::NextDesk);
                self.switch_desk(d);
            }
            Action::TakeToPrevDesk | Action::TakeToNextDesk => {
                let d = self.az.wm.neighbour_desk(action == Action::TakeToNextDesk);
                if let Some(id) = focused {
                    self.move_to_desk(id, d, true);
                } else {
                    self.switch_desk(d);
                }
            }
            Action::Overview => crate::overlays::toggle_overview(self),
            Action::Switcher { reverse } => crate::overlays::open_switcher(self, reverse, true),
            Action::NextWindowOfApp => {
                if let Some(id) = focused {
                    let app = self.az.wm.get(id).map(|w| w.app_id.clone()).unwrap_or_default();
                    let mut same: Vec<WinId> = self
                        .az
                        .wm
                        .stacking()
                        .iter()
                        .copied()
                        .filter(|w| {
                            self.az.wm.get(*w).is_some_and(|w| w.app_id == app) && self.az.wm.on_current_desk(*w)
                        })
                        .collect();
                    same.sort();
                    if let Some(i) = same.iter().position(|w| *w == id) {
                        let next = same[(i + 1) % same.len()];
                        if next != id {
                            self.activate_window(next);
                        }
                    }
                }
            }
            Action::HideApp => {
                if let Some(app) = focused.and_then(|id| self.az.wm.get(id).map(|w| w.app_id.clone())) {
                    self.hide_app(&app);
                }
            }
            Action::HideOthers => {
                if let Some(app) = focused.and_then(|id| self.az.wm.get(id).map(|w| w.app_id.clone())) {
                    self.hide_others(&app);
                }
            }
            Action::Minimise => {
                if let Some(id) = focused {
                    self.minimise(id);
                }
            }
            Action::Fill => {
                if let Some(id) = focused {
                    self.set_fill(id, None);
                }
            }
            Action::SplitLeft => {
                if let Some(id) = focused {
                    self.set_split(id, crate::wm::Side::Left);
                }
            }
            Action::SplitRight => {
                if let Some(id) = focused {
                    self.set_split(id, crate::wm::Side::Right);
                }
            }
            Action::StackWithBehind => {
                if let Some(id) = focused {
                    self.stack_with_behind(id);
                }
            }
            Action::Fullscreen => {
                if let Some(id) = focused {
                    self.set_fullscreen(id, None);
                }
            }
            Action::Terminal => {
                let argv = self.az.settings.keys.terminal.clone();
                let cwd = self.desk_folder();
                if let Err(e) = self.spawn(&argv, cwd) {
                    tracing::warn!("{e}");
                }
            }
            Action::Spawn(argv) => {
                let cwd = self.desk_folder();
                if let Err(e) = self.spawn(&argv, cwd) {
                    tracing::warn!("{e}");
                }
            }
            Action::ScreenshotScreen => {
                if let Err(e) = crate::screenshot::take(self, crate::screenshot::What::Screen, None, true) {
                    tracing::warn!("screenshot: {e}");
                }
            }
            Action::ScreenshotArea => crate::overlays::start_area_pick(self),
            Action::ScreenshotWindow => {
                if let Some(id) = focused
                    && let Err(e) = crate::screenshot::take(self, crate::screenshot::What::Window(id), None, true)
                {
                    tracing::warn!("screenshot: {e}");
                }
            }
            Action::ChangeVt(vt) => self.backend.change_vt(vt),
            Action::Quit => self.quit(),
            Action::Shell(name) => {
                let output = self.az.output_at(self.az.pointer_pos).map(|o| o.name());
                self.az.ipc.emit(&aizome_ipc::Event::Binding { action: name, pointer: None, output });
            }
            Action::None => {}
        }
    }

    /// Ends the session.
    pub fn quit(&mut self) {
        crate::session::notify_stopping();
        self.az.signal.stop();
    }
}

impl crate::state::Aizome {
    /// The window a surface (or one of its subsurfaces or popups) belongs to.
    pub fn window_for_root(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<WinId> {
        let mut root = surface.clone();
        while let Some(p) = smithay::wayland::compositor::get_parent(&root) {
            root = p;
        }
        if let Some(id) = self.window_for_surface(&root) {
            return Some(id);
        }
        // A popup's root.
        let popup = self.popups.find_popup(&root)?;
        let r = smithay::desktop::find_popup_root_surface(&popup).ok()?;
        self.window_for_surface(&r)
    }
}

/// The layer surface (or a subsurface of one) under a point; their popups
/// are in the popup layer.
fn layer_under(
    output: &Output,
    layers: &[Layer],
    pos: Point<f64, Logical>,
    out_loc: Point<f64, Logical>,
) -> Option<Under> {
    let map = layer_map_for_output(output);
    for layer in layers {
        for l in map.layers_on(*layer).rev() {
            let Some(geo) = map.layer_geometry(l) else { continue };
            let local = pos - out_loc - geo.loc.to_f64();
            if let Some((s, loc)) = l.surface_under(local, WindowSurfaceType::TOPLEVEL | WindowSurfaceType::SUBSURFACE)
            {
                return Some(Under::Surface(Focus(s), loc.to_f64() + geo.loc.to_f64() + out_loc, Owner::Layer));
            }
        }
    }
    None
}

/// The corner of a rect nearest a point.
fn nearest_corner(r: crate::wm::Rect, p: Point<f64, Logical>) -> Edges {
    let (cx, cy) = r.center();
    let mut e = Edges::empty();
    e |= if p.x < cx as f64 { Edges::LEFT } else { Edges::RIGHT };
    e |= if p.y < cy as f64 { Edges::TOP } else { Edges::BOTTOM };
    e
}

/// A window closed: interactive state forgets it.
pub fn window_gone(state: &mut State, id: WinId) {
    let i = &mut state.az.interactive;
    if matches!(i.hover, Some(Hit::Tab(w) | Hit::Close(w) | Hit::Zoom(w) | Hit::Edge(w, _)) if w == id) {
        i.hover = None;
    }
    if i.drag.as_ref().is_some_and(|d| d.win == id || d.target == Some(id)) {
        i.drag = None;
    }
    let _ = HashSet::<u32>::new();
}

/// Shift, Ctrl, Alt and Super, left and right (evdev codes).
const MODIFIER_KEYS: [u32; 8] = [42, 54, 29, 97, 56, 100, 125, 126];

/// Accessibility state of the keyboard: slow keys waiting to count, and
/// sticky modifiers.
#[derive(Default)]
pub struct Access {
    /// Keys held under slow keys: the timer while it hasn't counted yet.
    pub slow: std::collections::HashMap<u32, (Option<smithay::reexports::calloop::RegistrationToken>,)>,
    /// A modifier pressed with nothing else since.
    pub alone: Option<u32>,
    /// Modifiers latched for the next key.
    pub latched: Vec<u32>,
    /// Keys down with latched modifiers pressed for them.
    pub applied: Vec<(u32, Vec<u32>)>,
}
