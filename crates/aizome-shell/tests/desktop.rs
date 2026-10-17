//! The desktop, headless: its icons, selection, and the whole screen as
//! the boards show it (wallpaper, icons, bar and Deskbar).

mod common;

use aizome_kit::prelude::*;
use aizome_shell::desktop::DesktopMsg;
use aizome_shell::shell::Msg;
use aizome_shell::widgets::drawn::DragPhase;
use common::*;

fn board_desktop(day: bool) -> aizome_kit::testing::Harness<aizome_shell::shell::Shell> {
    let mut h = scene(day);
    with_mail_and_shelf(&mut h);
    {
        let s = h.program_mut();
        s.user = ("callen".into(), "Chris Allen".into());
        s.volumes_fixture = vec![
            aizome_sys::disks::Volume { block: "/org/freedesktop/UDisks2/block_devices/nvme0n1p3".into(), name: "nvme0".into(), ..Default::default() },
            aizome_sys::disks::Volume {
                block: "/org/freedesktop/UDisks2/block_devices/loop0".into(),
                name: "loop0".into(),
                image_file: Some("/home/callen/sekka-nightly.iso".into()),
                ..Default::default()
            },
        ];
        if day {
            s.settings.appearance.wallpaper_day = Some(wallpaper(true).to_string_lossy().into_owned());
        } else {
            s.settings.appearance.wallpaper_night = Some(wallpaper(false).to_string_lossy().into_owned());
        }
        s.theme_day = day;
        s.load_wallpapers();
        // The notifications' line has had its moment.
        s.banner = None;
        s.banner_queue.clear();
    }
    h.send(Msg::Nothing);
    h
}

#[test]
fn icons_line_up_and_select() {
    let mut h = board_desktop(false);
    let d = window_titled(&h, "Desktop");
    let texts = h.texts(d);
    for want in ["callen", "nvme0", "sekka-nightly.iso", "Trash"] {
        assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
    }
    h.send(Msg::Desktop(1, DesktopMsg::Select("trash".into(), false)));
    assert_eq!(h.program().desktop.selected, ["trash"]);
    h.send(Msg::Desktop(1, DesktopMsg::Band(DragPhase::Start(Point::new(0.0, 30.0)))));
    h.send(Msg::Desktop(1, DesktopMsg::Band(DragPhase::Move(Point::new(120.0, 200.0)))));
    assert_eq!(h.program().desktop.selected, ["home", "vol:/org/freedesktop/UDisks2/block_devices/nvme0n1p3"]);
    h.send(Msg::Desktop(1, DesktopMsg::Band(DragPhase::End(Point::new(120.0, 200.0)))));
    assert!(h.program().desktop.band.is_none());
}

#[test]
fn icons_move_where_theyre_dropped() {
    let mut h = board_desktop(false);
    h.send(Msg::Desktop(1, DesktopMsg::IconDrag("home".into(), DragPhase::Start(Point::new(40.0, 60.0)))));
    h.send(Msg::Desktop(1, DesktopMsg::IconDrag("home".into(), DragPhase::Move(Point::new(400.0, 300.0)))));
    h.send(Msg::Desktop(1, DesktopMsg::IconDrag("home".into(), DragPhase::End(Point::new(400.0, 300.0)))));
    let p = h.program().desktop.positions.get("home").copied().unwrap();
    assert_eq!(p, Point::new(376.0, 284.0));
}

#[test]
fn the_whole_screen() {
    for day in [false, true] {
        let mut h = board_desktop(day);
        let desk = window_titled(&h, "Desktop");
        let bar = window_titled(&h, "Menu bar");
        let db = window_titled(&h, "Deskbar");
        let size = aizome_shell::deskbar::size(&h.program().deskbar_model());
        h.resize(db, size);
        let at = Point::new(W - size.width, 24.0);
        let (w, hh, px) = compose(&mut h, &[(desk, Point::ZERO), (bar, Point::ZERO), (db, at)], Size::new(W, H), 2.0);
        let name = if day { "desktop-day.png" } else { "desktop-night.png" };
        aizome_kit::testing::write_png(&shots().join(name), w, hh, &px).unwrap();
        let only = if day { "desktop-icons-day.png" } else { "desktop-icons-night.png" };
        aizome_kit::testing::assert_golden(&mut h, desk, golden(only), 1.0);
    }
}
