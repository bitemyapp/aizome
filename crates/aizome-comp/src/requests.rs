//! Answers to IPC requests.

use aizome_ipc::{AppSel, DeskSel, Request, Response, Stats, Synthetic};
use smithay::utils::Point;

use crate::outputs::{OutputChange, transform_from_ipc};
use crate::state::State;
use crate::wm::{DeskId, Rect, Side, WinId};

impl State {
    pub fn handle_request(&mut self, req: Request) -> Response {
        match self.request(req) {
            Ok(r) => r,
            Err(e) => Response::Error { message: e },
        }
    }

    fn window_arg(&self, id: Option<WinId>) -> Result<WinId, String> {
        match id {
            Some(id) if self.az.wm.contains(id) => Ok(id),
            Some(id) => Err(format!("no window {id}")),
            None => self.az.wm.focused().ok_or_else(|| "no window has focus".to_string()),
        }
    }

    fn desk_arg(&self, d: &DeskSel) -> Result<DeskId, String> {
        match d {
            DeskSel::Index(i) => self.az.wm.desk_at(*i).ok_or_else(|| format!("no Desk {i}")),
            DeskSel::Name(n) => self.az.wm.desk_named(n).ok_or_else(|| format!("no Desk named {n}")),
        }
    }

    fn app_arg(&self, app: &AppSel) -> Result<String, String> {
        if let Some(a) = &app.app_id {
            return Ok(a.clone());
        }
        if let Some(pid) = app.pid {
            return self
                .az
                .wm
                .windows()
                .find(|w| w.pid == Some(pid))
                .map(|w| w.app_id.clone())
                .ok_or_else(|| format!("no window of process {pid}"));
        }
        let id = self.az.wm.focused().ok_or("no window has focus")?;
        Ok(self.az.wm.get(id).map(|w| w.app_id.clone()).unwrap_or_default())
    }

    fn request(&mut self, req: Request) -> Result<Response, String> {
        Ok(match req {
            Request::Version => Response::Version { version: env!("CARGO_PKG_VERSION").into(), protocol: aizome_ipc::PROTOCOL_VERSION },
            Request::Outputs => Response::Outputs { outputs: crate::outputs::all_info(&self.az) },
            Request::Windows => {
                let windows = self.az.wm.stacking().iter().filter_map(|id| self.az.window_info(*id)).collect();
                Response::Windows { windows }
            }
            Request::Desks => {
                let desks = crate::actions::desk_infos(&self.az);
                let current = self.az.wm.desk_index(self.az.wm.current_desk()).unwrap_or(1);
                Response::Desks { desks, current }
            }
            Request::FocusedWindow => Response::Window { window: self.az.wm.focused().and_then(|id| self.az.window_info(id)) },
            Request::Focus { id } => {
                let id = self.window_arg(Some(id))?;
                self.activate_window(id);
                Response::Ok
            }
            Request::Close { id } => {
                let id = self.window_arg(id)?;
                self.close_window(id);
                Response::Ok
            }
            Request::Minimise { id } => {
                let id = self.window_arg(id)?;
                self.minimise(id);
                Response::Ok
            }
            Request::Unminimise { id } => {
                let id = self.window_arg(Some(id))?;
                self.unminimise(id);
                Response::Ok
            }
            Request::HideApp { app } => {
                let app = self.app_arg(&app)?;
                self.hide_app(&app);
                Response::Ok
            }
            Request::HideOthers => {
                let app = self.app_arg(&AppSel::default())?;
                self.hide_others(&app);
                Response::Ok
            }
            Request::ShowApp { app } => {
                let app = self.app_arg(&app)?;
                self.show_app(&app);
                Response::Ok
            }
            Request::Fill { id, state } => {
                let id = self.window_arg(id)?;
                self.set_fill(id, state);
                Response::Ok
            }
            Request::Split { id, side } => {
                let id = self.window_arg(id)?;
                let side = match side {
                    aizome_ipc::Side::Left => Side::Left,
                    aizome_ipc::Side::Right => Side::Right,
                };
                self.set_split(id, side);
                Response::Ok
            }
            Request::Fullscreen { id, state } => {
                let id = self.window_arg(id)?;
                self.set_fullscreen(id, state);
                Response::Ok
            }
            Request::Float { id, state } => {
                let id = self.window_arg(id)?;
                self.set_float(id, state);
                Response::Ok
            }
            Request::Sticky { id, state } => {
                let id = self.window_arg(id)?;
                self.set_sticky(id, state);
                Response::Ok
            }
            Request::MoveToDesk { id, desk, follow } => {
                let id = self.window_arg(id)?;
                let desk = self.desk_arg(&desk)?;
                self.move_to_desk(id, desk, follow);
                Response::Ok
            }
            Request::SetDesk { desk } => {
                let desk = self.desk_arg(&desk)?;
                self.switch_desk(desk);
                Response::Ok
            }
            Request::AddDesk { name, folder, index } => {
                self.az.wm.add_desk(name, folder, index);
                self.emit_desks();
                Response::Ok
            }
            Request::RenameDesk { desk, name } => {
                let desk = self.desk_arg(&desk)?;
                self.az.wm.rename_desk(desk, name);
                self.emit_desks();
                Response::Ok
            }
            Request::SetDeskFolder { desk, folder } => {
                let desk = self.desk_arg(&desk)?;
                self.az.wm.set_desk_folder(desk, folder);
                self.emit_desks();
                Response::Ok
            }
            Request::RemoveDesk { desk } => {
                let desk = self.desk_arg(&desk)?;
                let before = self.az.wm.current_desk();
                let r = self.az.wm.remove_desk(desk).ok_or("the last Desk stays")?;
                if self.az.wm.current_desk() != before {
                    self.az.window_outputs_dirty = true;
                    self.az.queue_redraw_all();
                }
                let ids: Vec<WinId> = self.az.wm.windows().map(|w| w.id).collect();
                for id in ids {
                    self.az.window_changed(id);
                }
                self.apply_refocus(r);
                self.emit_desks();
                Response::Ok
            }
            Request::MoveDesk { desk, index } => {
                let desk = self.desk_arg(&desk)?;
                self.az.wm.move_desk(desk, index);
                self.emit_desks();
                Response::Ok
            }
            Request::Stack { a, b } => {
                let a = self.window_arg(Some(a))?;
                let b = self.window_arg(Some(b))?;
                if !self.stack(a, b) {
                    return Err("those windows are already stacked".into());
                }
                Response::Ok
            }
            Request::Unstack { id } => {
                let id = self.window_arg(id)?;
                if !self.unstack(id, None) {
                    return Err(format!("window {id} isn't stacked"));
                }
                Response::Ok
            }
            Request::Move { id, x, y } => {
                let id = self.window_arg(id)?;
                self.move_window(id, x, y);
                Response::Ok
            }
            Request::Resize { id, rect } => {
                let id = self.window_arg(id)?;
                self.resize_window(id, Rect::new(rect.x, rect.y, rect.w, rect.h));
                Response::Ok
            }
            Request::Overview { state } => {
                let open = self.az.overlays.overview_open();
                if state.unwrap_or(!open) != open {
                    crate::overlays::toggle_overview(self);
                }
                Response::Ok
            }
            Request::Switcher { reverse, state } => {
                match state {
                    Some(false) => crate::overlays::close_switcher(self, false),
                    _ => crate::overlays::open_switcher(self, reverse, false),
                }
                Response::Ok
            }
            Request::Spawn { argv, cwd, in_desk_folder } => {
                let cwd = cwd.map(|c| aizome_config::expand_home(&c)).or_else(|| if in_desk_folder { self.desk_folder() } else { None });
                let pid = self.spawn(&argv, cwd)?;
                Response::Spawned { pid }
            }
            Request::Screenshot { output, path, window, area, clipboard } => {
                use crate::screenshot::What;
                let what = if let Some(id) = window {
                    What::Window(self.window_arg(Some(id))?)
                } else if let Some(a) = area {
                    What::Area(Rect::new(a.x, a.y, a.w, a.h))
                } else if let Some(name) = output {
                    What::Output(self.az.output_by_name(&name).ok_or_else(|| format!("no output {name}"))?)
                } else {
                    What::Screen
                };
                let path = crate::screenshot::take(self, what, path.map(std::path::PathBuf::from), clipboard)?;
                Response::Screenshot { path: path.to_string_lossy().into_owned() }
            }
            Request::ReloadConfig => {
                self.reload_config()?;
                Response::Ok
            }
            Request::OutputConfig { name, mode, scale, position, transform, enabled } => {
                let mode = match mode {
                    Some(m) => Some(aizome_config::parse_mode(&m).ok_or_else(|| format!("bad mode {m}"))?),
                    None => None,
                };
                let change = OutputChange {
                    name,
                    mode,
                    scale,
                    position: position.map(|p| (p[0], p[1])),
                    transform: transform.map(transform_from_ipc),
                    enabled,
                };
                self.backend.configure_output(&mut self.az, &change)?;
                Response::Ok
            }
            Request::Stats { reset } => {
                let outputs = self
                    .az
                    .outputs
                    .iter()
                    .filter_map(|o| self.az.out.get(o).map(|s| s.stats.report(&o.name())))
                    .collect();
                let stats = Stats { renderer: self.backend.renderer_name(), backend: self.backend.name().into(), outputs };
                if reset {
                    for o in self.az.out.values_mut() {
                        o.stats.reset();
                    }
                }
                Response::Stats { stats }
            }
            Request::Quit => {
                self.quit();
                Response::Ok
            }
            Request::PowerOffMonitors => {
                self.backend.set_power(&mut self.az, false);
                Response::Ok
            }
            Request::PowerOnMonitors => {
                self.backend.set_power(&mut self.az, true);
                Response::Ok
            }
            Request::Lock => {
                self.az.ipc.emit(&aizome_ipc::Event::Binding { action: "lock".into(), pointer: None, output: None });
                Response::Ok
            }
            Request::Subscribe => Response::Ok,
            Request::Action { action } => {
                let a = crate::bindings::Action::parse(&action).ok_or_else(|| format!("unknown action {action}"))?;
                self.run_action(a);
                Response::Ok
            }
            Request::Synthesize { events } => {
                for e in events {
                    self.synthesize(e)?;
                }
                Response::Ok
            }
        })
    }

    fn synthesize(&mut self, e: Synthetic) -> Result<(), String> {
        let time = self.az.start.elapsed().as_millis() as u32;
        match e {
            Synthetic::PointerMotion { x, y } => self.pointer_motion_absolute(Point::from((x, y)), time),
            Synthetic::PointerButton { button, pressed } => self.pointer_button(button_code(&button)?, pressed, time),
            Synthetic::Click { button } => {
                let b = button_code(&button)?;
                self.pointer_button(b, true, time);
                self.pointer_button(b, false, time);
            }
            Synthetic::PointerScroll { dx, dy } => self.pointer_scroll(dx, dy, time),
            Synthetic::Key { key, pressed } => self.keyboard_key(key_code(&key)?, pressed, time),
            Synthetic::Chord { keys } => {
                let codes: Vec<u32> = keys.split('+').map(|k| key_code(k.trim())).collect::<Result<_, _>>()?;
                for c in &codes {
                    self.keyboard_key(*c, true, time);
                }
                for c in codes.iter().rev() {
                    self.keyboard_key(*c, false, time);
                }
            }
        }
        Ok(())
    }
}

fn button_code(name: &str) -> Result<u32, String> {
    Ok(match name {
        "left" | "" => crate::input::BTN_LEFT,
        "right" => crate::input::BTN_RIGHT,
        "middle" => crate::input::BTN_MIDDLE,
        n => n.parse().map_err(|_| format!("unknown button {n}"))?,
    })
}

/// evdev key codes by name.
pub fn key_code(name: &str) -> Result<u32, String> {
    if let Ok(n) = name.parse::<u32>()
        && name.len() > 1 {
            return Ok(n);
        }
    let lower = name.to_lowercase();
    let code = match lower.as_str() {
        "esc" | "escape" => 1,
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        "0" => 11,
        "minus" | "-" => 12,
        "equal" | "=" => 13,
        "backspace" => 14,
        "tab" => 15,
        "q" => 16,
        "w" => 17,
        "e" => 18,
        "r" => 19,
        "t" => 20,
        "y" => 21,
        "u" => 22,
        "i" => 23,
        "o" => 24,
        "p" => 25,
        "leftbrace" | "[" => 26,
        "rightbrace" | "]" => 27,
        "return" | "enter" => 28,
        "ctrl" | "control" | "leftctrl" => 29,
        "a" => 30,
        "s" => 31,
        "d" => 32,
        "f" => 33,
        "g" => 34,
        "h" => 35,
        "j" => 36,
        "k" => 37,
        "l" => 38,
        "semicolon" | ";" => 39,
        "apostrophe" | "'" => 40,
        "grave" | "`" => 41,
        "shift" | "leftshift" => 42,
        "backslash" | "\\" => 43,
        "z" => 44,
        "x" => 45,
        "c" => 46,
        "v" => 47,
        "b" => 48,
        "n" => 49,
        "m" => 50,
        "comma" | "," => 51,
        "dot" | "period" | "." => 52,
        "slash" | "/" => 53,
        "rightshift" => 54,
        "alt" | "option" | "leftalt" => 56,
        "space" => 57,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        "print" | "sysrq" => 99,
        "home" => 102,
        "up" => 103,
        "pageup" => 104,
        "left" => 105,
        "right" => 106,
        "end" => 107,
        "down" => 108,
        "pagedown" => 109,
        "delete" => 111,
        "mute" => 113,
        "volumedown" => 114,
        "volumeup" => 115,
        "super" | "logo" | "cmd" | "leftmeta" => 125,
        _ => return Err(format!("unknown key {name}")),
    };
    Ok(code)
}
