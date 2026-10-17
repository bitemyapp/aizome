//! Pictures of Tansu's main views in Night and Day, from the sample disks:
//! golden images (`AIZOME_UPDATE_GOLDENS=1` rewrites them).

use std::rc::Rc;

use aizome_kit::testing::{Harness, assert_golden};
use aizome_kit::theme::{Appearance, ThemeChoice};
use tansu::app::{Msg, Tansu};
use tansu::mock::Mock;

fn harness(theme: ThemeChoice) -> (Harness<Tansu>, Rc<Mock>) {
    let mock = Rc::new(Mock::default());
    let h = Harness::with(Tansu::new(mock.clone()), Appearance { theme, ..Appearance::default() }, false);
    (h, mock)
}

fn name(theme: ThemeChoice) -> &'static str {
    if theme == ThemeChoice::Day { "day" } else { "night" }
}

#[test]
fn the_system_disk() {
    for theme in [ThemeChoice::Night, ThemeChoice::Day] {
        let (mut h, _) = harness(theme);
        let w = h.window();
        assert_golden(&mut h, w, format!("tests/snapshots/system-disk-{}.png", name(theme)), 1.0);
    }
}

#[test]
fn a_usb_stick_with_pending_operations() {
    for theme in [ThemeChoice::Night, ThemeChoice::Day] {
        let (mut h, _) = harness(theme);
        let w = h.window();
        let stick = h.program().drives.iter().find(|d| d.name.contains("Cruzer")).unwrap().key.clone();
        h.send(Msg::SelectDrive(stick));
        assert_golden(&mut h, w, format!("tests/snapshots/usb-stick-{}.png", name(theme)), 1.0);
    }
}
