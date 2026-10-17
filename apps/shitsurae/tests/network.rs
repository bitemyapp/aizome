//! The Network pane over the sample: Wi-Fi, joining, details, VPNs.

mod common;

use aizome_kit::layout::Size;
use aizome_kit::testing::Harness;
use aizome_kit::theme::{Appearance, ThemeChoice};
use shitsurae::Shitsurae;

fn open(name: &str) -> Harness<Shitsurae> {
    let (mut h, _) = common::open(name, "network");
    let w = h.window();
    h.resize(w, Size::new(940.0, 1180.0));
    settle(&mut h, |h| has(h, "Kohaku"));
    h
}

#[test]
fn shows_wifi_networks_and_vpns() {
    let mut h = open("network-main");
    let w = h.window();
    assert_eq!(h.program().pane_id(), "network");
    let texts = h.texts(w);
    for want in ["Wi-Fi", "Kohaku", "Aizome Studio", "Hanada Café", "Other Network…", "Office", "Saved Networks", "Ethernet", "Cable unplugged"] {
        assert!(texts.iter().any(|t| t == want), "{want} missing from {texts:?}");
    }
    h.save_png(w, common::shots_dir().join("network-night.png"), 2.0);
    h.set_appearance(Appearance { theme: ThemeChoice::Day, ..Appearance::default() });
    h.save_png(w, common::shots_dir().join("network-day.png"), 2.0);
}

#[test]
fn joining_asks_for_the_password_and_says_how_it_went() {
    let mut h = open("network-join");
    let w = h.window();
    // A secured network asks for its password in a sheet.
    assert!(h.click_text(w, "Tōkyō Guest"));
    settle(&mut h, |h| has(h, "Join “Tōkyō Guest”"));
    h.save_png(w, common::shots_dir().join("network-password.png"), 2.0);
    h.type_text(w, "kamenozoki");
    assert!(h.click_text(w, "Join"));
    // The Wi-Fi row says so, and so does the status strip once joining ends.
    settle(&mut h, |h| h.texts(h.window()).iter().filter(|t| *t == "Connected to Tōkyō Guest").count() == 2);
    let texts = h.texts(w);
    assert_eq!(texts.iter().filter(|t| *t == "Connected to Tōkyō Guest").count(), 2, "{texts:?}");
    // WEP isn't offered.
    assert!(h.click_text(w, "Old Router"));
    settle(&mut h, |h| h.texts(h.window()).iter().any(|t| t.contains("WEP")));
    assert!(h.texts(w).iter().any(|t| t.contains("isn't offered")), "{:?}", h.texts(w));
}

#[test]
fn a_wrong_password_is_said_plainly() {
    let mut h = open("network-wrong");
    let w = h.window();
    assert!(h.click_text(w, "Tōkyō Guest"));
    settle(&mut h, |h| has(h, "Join “Tōkyō Guest”"));
    h.type_text(w, "wrong");
    // Too short for WPA3? No: any length. Join with it.
    assert!(h.click_text(w, "Join"));
    settle(&mut h, |h| h.texts(h.window()).iter().any(|t| t.contains("Check the password")));
    assert!(h.texts(w).iter().any(|t| t.contains("Couldn't join Tōkyō Guest")), "{:?}", h.texts(w));
}

#[test]
fn details_show_addresses_and_settings() {
    let mut h = open("network-details");
    let w = h.window();
    assert!(h.click_text(w, "Details…"));
    settle(&mut h, |h| has(h, "Configure IPv4"));
    let texts = h.texts(w);
    for want in ["Join automatically", "Metered connection", "Private address", "DNS servers", "Configure IPv6", "Forget This Network…", "192.168.10.42/24\nfd00:10::42/64"] {
        assert!(texts.iter().any(|t| t == want), "{want} missing from {texts:?}");
    }
    h.save_png(w, common::shots_dir().join("network-details.png"), 2.0);
    // Back to the network list.
    assert!(h.click_text(w, "Network"));
    settle(&mut h, |h| has(h, "Other Network…"));
}

#[test]
fn hidden_networks_join_by_name() {
    let mut h = open("network-hidden");
    let w = h.window();
    assert!(h.click_text(w, "Other Network…"));
    settle(&mut h, |h| has(h, "Join a Network by Name"));
    h.save_png(w, common::shots_dir().join("network-hidden.png"), 2.0);
}

/// Let background work (sample tasks, the sample compositor) come back:
/// flush the window until `until` holds, up to two seconds.
fn settle(h: &mut Harness<Shitsurae>, until: impl Fn(&Harness<Shitsurae>) -> bool) {
    for _ in 0..400 {
        if until(h) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.send(shitsurae::AppMsg::Nothing);
    }
}

fn has(h: &Harness<Shitsurae>, s: &str) -> bool {
    let w = h.window();
    h.texts(w).iter().any(|t| t == s)
}
