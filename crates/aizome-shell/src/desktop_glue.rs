//! The desktop's part of the shell: its icons from the system and the
//! desktop folder, its wallpaper, and what clicks, drags and drops on it
//! do.

use std::path::PathBuf;

use aizome_kit::prelude::*;
use aizome_kit::{Image, MenuStyle};

use crate::actions::{Action, DiskVerb};
use crate::desktop::{self, DeskIcon, DesktopMsg, IconKind};
use crate::menus::model::{Entry, Menu as ShellMenu};
use crate::shell::{Msg, Shell, at_point};
use crate::widgets::drawn::DragPhase;

impl Shell {
    pub fn desktop_icons(&self) -> Vec<DeskIcon> {
        let volumes = self.sys.disks.as_ref().map(|d| d.snapshot().volumes()).unwrap_or_default();
        let volumes = if volumes.is_empty() { self.volumes_fixture.clone() } else { volumes };
        desktop::icons(&self.user.0, &volumes, self.trash_full, &self.desktop.items)
    }

    /// The wallpaper for the theme showing, decoded once.
    pub fn wallpaper(&self) -> Option<Image> {
        let path = desktop::wallpaper_path(&self.settings, self.theme_day)?;
        self.wallpapers.get(&path).cloned()
    }

    /// Open the wallpapers' files (the render thread decodes them, once per
    /// size it draws them at).
    pub fn load_wallpapers(&mut self) {
        for day in [false, true] {
            if let Some(p) = desktop::wallpaper_path(&self.settings, day)
                && !self.wallpapers.contains_key(&p)
            {
                match Image::open(&p) {
                    Ok(img) => {
                        self.wallpapers.insert(p, img);
                    }
                    Err(e) => log::warn!("the wallpaper {}: {e}", p.display()),
                }
            }
        }
    }

    pub fn desktop_view(&self, output: u32) -> Element<Msg> {
        let Some(screen) = self.screens.iter().find(|s| s.output.id == output) else { return aizome_kit::space(0.0) };
        let size = screen.output.frame.size();
        let icons = self.desktop_icons();
        desktop::view(&icons, &self.desktop, self.wallpaper(), size).map(move |m| Msg::Desktop(output, m))
    }

    fn icon(&self, key: &str) -> Option<DeskIcon> {
        self.desktop_icons().into_iter().find(|i| i.key == key)
    }

    pub fn desktop_msg(&mut self, output: u32, m: DesktopMsg, cx: &mut Cx<Msg>) {
        let height = self.screens.iter().find(|s| s.output.id == output).map_or(900.0, |s| s.output.frame.height());
        match m {
            DesktopMsg::Select(key, add) => {
                if add {
                    if let Some(i) = self.desktop.selected.iter().position(|k| *k == key) {
                        self.desktop.selected.remove(i);
                    } else {
                        self.desktop.selected.push(key);
                    }
                } else {
                    self.desktop.selected = vec![key];
                }
            }
            DesktopMsg::Clear => self.desktop.selected.clear(),
            DesktopMsg::Open(key) => self.open_icon(&key, cx),
            DesktopMsg::IconMenu(key, at) => {
                let Some(icon) = self.icon(&key) else { return };
                let Some(w) = self.screens.iter().find(|s| s.output.id == output).and_then(|s| s.bar) else { return };
                let open = Action::Palette { scope: None, query: String::new() };
                let _ = open;
                let entries = match &icon.kind {
                    IconKind::Home(p) => vec![Entry::action("Open", Action::Open { path: p.clone() })],
                    IconKind::Volume { block, mount, ejectable, drive, .. } => {
                        let mut v = vec![
                            Entry::action("Open", Action::Disk { device: block.clone(), verb: DiskVerb::Mount }),
                            Entry::separator(),
                            Entry::action("Unmount", Action::Disk { device: block.clone(), verb: DiskVerb::Unmount }).enabled(mount.is_some()),
                        ];
                        if *ejectable && let Some(d) = drive {
                            v.push(Entry::action("Eject", Action::Disk { device: d.clone(), verb: DiskVerb::Eject }));
                        }
                        v.push(Entry::separator());
                        v.push(Entry::action("Open in Tansu", Action::Disk { device: block.clone(), verb: DiskVerb::OpenInTansu }));
                        v
                    }
                    IconKind::Trash { full } => vec![
                        Entry::action("Open", Action::Open { path: crate::launch::trash_dir().join("files") }),
                        Entry::separator(),
                        Entry::action("Empty Trash", Action::EmptyTrash).enabled(*full),
                    ],
                    IconKind::Item { path, .. } => vec![
                        Entry::action("Open", Action::Open { path: path.clone() }),
                        Entry::action("Show in Hako", Action::Reveal { path: path.clone() }),
                        Entry::action("Copy Path", Action::Copy { text: path.to_string_lossy().into_owned() }),
                        Entry::separator(),
                        Entry::action("Move to Trash", Action::MoveToTrash { path: path.clone() }),
                    ],
                };
                let menu = ShellMenu::new(icon.label.clone(), entries);
                self.show_menu(w, at_point(at), &menu, MenuStyle::at_point(), cx);
            }
            DesktopMsg::IconDrag(key, phase) => {
                let icons = self.desktop_icons();
                let points = desktop::layout(&icons, &self.desktop.positions, height);
                let origin = icons.iter().zip(points).find(|(i, _)| i.key == key).map(|(_, p)| p).unwrap_or_default();
                match phase {
                    DragPhase::Start(p) => {
                        self.drag_grab = p - origin;
                        self.desktop.dragging = Some((key, origin));
                    }
                    DragPhase::Move(p) => self.desktop.dragging = Some((key, p - self.drag_grab)),
                    DragPhase::End(p) => {
                        self.desktop.dragging = None;
                        let target = desktop::icon_at(&icons, &self.desktop.positions, height, p).filter(|t| *t != key);
                        let dragged = icons.iter().find(|i| i.key == key).cloned();
                        match (target.and_then(|t| icons.iter().find(|i| i.key == t).cloned()), dragged) {
                            (Some(t), Some(DeskIcon { kind: IconKind::Item { path, .. }, .. })) if matches!(t.kind, IconKind::Trash { .. }) => {
                                self.run(Action::MoveToTrash { path }, cx);
                                self.refresh_desktop_items(cx);
                            }
                            _ => {
                                let to = p - self.drag_grab;
                                let snapped = Point::new((to.x / 4.0).round() * 4.0, (to.y / 4.0).round() * 4.0);
                                self.desktop.positions.insert(key, snapped);
                                if self.opts.live {
                                    self.desktop.save_positions(&desktop::DesktopState::path());
                                }
                            }
                        }
                    }
                }
            }
            DesktopMsg::Band(phase) => match phase {
                DragPhase::Start(p) => self.desktop.band = Some((p, p)),
                DragPhase::Move(p) => {
                    if let Some((a, _)) = self.desktop.band {
                        self.desktop.band = Some((a, p));
                        let icons = self.desktop_icons();
                        self.desktop.selected = desktop::in_band(&icons, &self.desktop.positions, height, a, p);
                    }
                }
                DragPhase::End(_) => self.desktop.band = None,
            },
            DesktopMsg::Menu(at) => {
                let origin = self.screens.iter().find(|s| s.output.id == output).map(|s| s.output.frame.origin()).unwrap_or_default();
                self.pointer_menu(origin.x + at.x, origin.y + at.y, cx);
            }
            DesktopMsg::DropOn(key, urls) => {
                let Some(icon) = self.icon(&key) else { return };
                let paths: Vec<PathBuf> = urls
                    .iter()
                    .filter_map(|u| u.strip_prefix("file://").map(|p| PathBuf::from(crate::shelf::percent_decode(p))))
                    .collect();
                if let IconKind::Trash { .. } = icon.kind {
                    for p in paths {
                        self.run(Action::MoveToTrash { path: p }, cx);
                    }
                    self.refresh_desktop_items(cx);
                }
            }
        }
    }

    fn open_icon(&mut self, key: &str, cx: &mut Cx<Msg>) {
        let Some(icon) = self.icon(key) else { return };
        match icon.kind {
            IconKind::Home(p) => self.run(Action::Open { path: p }, cx),
            IconKind::Trash { .. } => self.run(Action::Open { path: crate::launch::trash_dir().join("files") }, cx),
            IconKind::Item { path, .. } => self.run(Action::Open { path }, cx),
            IconKind::Volume { block, mount: Some(m), .. } => {
                let _ = block;
                self.run(Action::Open { path: PathBuf::from(m) }, cx);
            }
            IconKind::Volume { block, mount: None, .. } => self.run(Action::Disk { device: block, verb: DiskVerb::Mount }, cx),
        }
    }

    /// Mount, unmount, eject, or open a disk in Tansu.
    pub fn disk(&mut self, device: String, verb: DiskVerb, cx: &mut Cx<Msg>) {
        match verb {
            DiskVerb::OpenInTansu => self.launch(crate::launch::Launch::Argv(vec!["aizome-tansu".into(), device]), cx),
            DiskVerb::Mount => {
                let mounted = self
                    .sys
                    .disks
                    .as_ref()
                    .and_then(|d| d.snapshot().volumes().into_iter().find(|v| v.block == device))
                    .and_then(|v| v.mount_point);
                match mounted {
                    Some(m) => self.run(Action::Open { path: PathBuf::from(m) }, cx),
                    None => {
                        if let Some(d) = &self.sys.disks {
                            let task = d.mount(&device);
                            // Opened once it's mounted.
                            cx.spawn(move || match task.wait() {
                                Ok(point) => Msg::Run(Action::Open { path: PathBuf::from(point) }),
                                Err(e) => {
                                    log::warn!("mounting {device}: {e}");
                                    Msg::Nothing
                                }
                            });
                        }
                    }
                }
            }
            DiskVerb::Unmount => {
                if let Some(d) = &self.sys.disks {
                    let _ = d.unmount(&device, false);
                }
            }
            DiskVerb::Eject => {
                if let Some(d) = &self.sys.disks {
                    let _ = d.eject(&device);
                }
            }
        }
    }

    /// Read `~/Desktop` again (and whether the Trash is full), on a thread.
    pub fn refresh_desktop_items(&mut self, cx: &mut Cx<Msg>) {
        cx.spawn(|| {
            let items = desktop::read_items(&crate::launch::desktop_dir());
            Msg::DesktopItems(items, crate::launch::trash_full())
        });
    }

    /// Watch `~/Desktop` and the Trash (every two seconds, by their
    /// modification times).
    pub fn watch_desktop(&mut self) {
        let Some(tx) = self.sender() else { return };
        std::thread::Builder::new()
            .name("desktop-watch".into())
            .spawn(move || {
                let stamp = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
                let mut last = (None, None);
                loop {
                    let dir = crate::launch::desktop_dir();
                    let trash = crate::launch::trash_dir().join("files");
                    let now = (stamp(&dir), stamp(&trash));
                    if now != last {
                        last = now;
                        if !tx.send(Msg::DesktopItems(desktop::read_items(&dir), crate::launch::trash_full())) {
                            return;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            })
            .ok();
    }
}
