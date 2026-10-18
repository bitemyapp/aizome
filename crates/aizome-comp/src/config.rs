//! Settings applied live: theme and focus colour on tabs, effects tier,
//! keyboard layout and repeat, pointer and touchpad, outputs and Desks.

use aizome_config::Settings;
use aizome_ipc::Event;
use aizome_theme::Palette;
use smithay::input::keyboard::XkbConfig;
use tracing::{info, warn};

use crate::state::State;

/// The local hour, for the auto theme.
pub fn local_hour() -> u32 {
    // Safety: localtime_r writes only into `tm`.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_hour as u32
    }
}

pub fn palette_for(s: &Settings) -> Palette {
    let theme = s.appearance.theme.resolve(local_hour());
    Palette::new(theme, s.appearance.focus)
}

impl State {
    /// Re-reads the settings file and applies what changed.
    pub fn reload_config(&mut self) -> Result<(), String> {
        let path = self.az.options.config.clone().unwrap_or_else(Settings::path);
        let new = Settings::load_from(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        // Installed apps and icon themes may have changed too.
        self.az.icons.clear();
        self.apply_settings(new);
        Ok(())
    }

    pub fn apply_settings(&mut self, new: Settings) {
        let old = std::mem::replace(&mut self.az.settings, new);
        let s = self.az.settings.clone();

        // Theme, focus colour and effects: tabs and frames redraw.
        self.az.set_palette(palette_for(&s));
        if old.appearance.effects != s.appearance.effects || old.appearance.reduced_motion != s.appearance.reduced_motion {
            self.az.queue_redraw_all();
        }
        if old.appearance.cursor_theme != s.appearance.cursor_theme || old.appearance.cursor_size != s.appearance.cursor_size {
            self.az.cursor_theme = crate::cursor::CursorTheme::load(&s.appearance.cursor_theme, s.appearance.cursor_size);
            self.az.queue_redraw_all();
        }

        if old.keys != s.keys {
            self.az.bindings = crate::bindings::Bindings::new(&s.keys.bindings);
        }

        // Keyboard.
        let (ok, nk) = (&old.input.keyboard, &s.input.keyboard);
        if let Some(kb) = self.az.seat.get_keyboard() {
            if ok.layouts != nk.layouts || ok.variant != nk.variant || ok.options != nk.options || ok.model != nk.model {
                let layouts = nk.layouts.join(",");
                let cfg = XkbConfig {
                    rules: "",
                    model: &nk.model,
                    layout: &layouts,
                    variant: &nk.variant,
                    options: if nk.options.is_empty() { None } else { Some(nk.options.clone()) },
                };
                if let Err(e) = kb.set_xkb_config(self, cfg) {
                    warn!("keyboard layout {layouts}: {e:?}");
                }
            }
            if ok.repeat_delay != nk.repeat_delay || ok.repeat_rate != nk.repeat_rate {
                kb.change_repeat_info(nk.repeat_rate as i32, nk.repeat_delay as i32);
            }
        }

        // Pointer and touchpad (libinput devices on udev).
        if (old.input.pointer != s.input.pointer || old.input.touchpad != s.input.touchpad)
            && let crate::backend::Backend::Udev(u) = &mut self.backend {
                u.apply_input_settings(&s.input);
            }

        // Desks.
        let wanted: Vec<(String, Option<String>)> = s.desks_or_default().into_iter().map(|d| (d.name, d.folder)).collect();
        if old.desks_or_default() != s.desks_or_default() && self.az.wm.sync_desks(&wanted) {
            self.az.window_outputs_dirty = true;
            self.az.queue_redraw_all();
            self.emit_desks();
            let r = self.az.wm.focused().filter(|f| self.az.wm.is_shown(*f));
            if r.is_none() {
                let next = self.az.wm.next_focus(None);
                self.focus_window(next);
            }
        }

        // Outputs.
        if old.outputs != s.outputs {
            self.apply_output_settings();
        }
        if old.night_light != s.night_light {
            self.update_night_light(true);
        }

        self.az.idle.reset_timers(&s);
        info!("settings applied");
        self.az.ipc.emit(&Event::ConfigChanged);
    }

    /// Applies the settings' output entries to the outputs present: modes,
    /// scale, place and transform, on or off, then mirroring.
    pub fn apply_output_settings(&mut self) {
        let entry = |az: &crate::state::Aizome, o: &smithay::output::Output| {
            let props = o.physical_properties();
            let serial = o.user_data().get::<crate::outputs::OutputSerial>().map(|s| s.0.clone()).unwrap_or_default();
            az.settings.output(&o.name(), &props.make, &props.model, &serial).cloned()
        };
        for o in self.az.known_outputs() {
            let Some(cfg) = entry(&self.az, &o) else { continue };
            let change = crate::outputs::OutputChange {
                name: o.name(),
                mode: cfg.parsed_mode(),
                scale: cfg.scale,
                position: cfg.position.map(|p| (p[0], p[1])),
                transform: cfg.transform.map(crate::outputs::transform_from_config),
                enabled: cfg.enabled,
            };
            if let Err(e) = self.configure_output(&change) {
                warn!("output {}: {e}", o.name());
            }
        }
        for o in self.az.known_outputs() {
            if self.az.disabled_outputs.contains(&o) {
                continue;
            }
            let want = entry(&self.az, &o).and_then(|c| c.mirror().map(str::to_owned));
            let have = self.az.mirror_source(&o).map(|s| s.name());
            if want != have
                && let Err(e) = self.mirror_output(&o.name(), want.as_deref())
            {
                warn!("output {}: {e}", o.name());
            }
        }
        crate::outputs::outputs_changed(&mut self.az);
    }
}
