//! The compositor end to end: a headless instance, Wayland test clients,
//! IPC and screenshot pixels, under GLES and pixman.

mod common;

use aizome_ipc::{DeskSel, Request, Synthetic};
use common::*;

// Night palette.
const FOCUS: u32 = 0xe0af68;
const TAB_IDLE: u32 = 0x2f334d;
const BEVEL_LO: u32 = 0x0b0c12;
const DESKTOP: u32 = 0x16161e;
/// `slab` (black at 50%) over the desktop.
const SHADOW_ON_DESKTOP: u32 = 0x0b0b0f;

fn opts(renderer: Option<&'static str>) -> Options<'static> {
    Options { renderer, ..Default::default() }
}

fn check_decorations(renderer: Option<&'static str>) {
    // Lean, the default tier: hard shadows.
    let c = Compositor::start(Options { renderer, settings: "", ..Default::default() });
    let _a = c.client(Spec::new("test.red", 0xcc2222).size(400, 300));
    let w = c.wait_window("test.red");
    assert!(w.focused, "a new window takes focus");
    assert!(!w.client_side_decorations);
    c.settle();
    let img = c.screenshot();
    let g = w.geometry;
    assert_eq!((g.w, g.h), (400, 300));
    // Content.
    assert_px(&img, g.x + 200, g.y + 150, rgb(0xcc2222), "content");
    // The focused tab: amber, 22pt tall, its border on top.
    assert_px(&img, g.x + 2, g.y - 14, rgb(FOCUS), "focused tab");
    assert_px(&img, g.x + 2, g.y - 25, rgb(BEVEL_LO), "tab top border");
    assert_px(&img, g.x - 1, g.y - 14, rgb(BEVEL_LO), "tab left border");
    // The rail: 2pt of focus colour above the content, the frame edge above it.
    let far = g.x + g.w - 10; // beyond the tab
    assert_px(&img, far, g.y - 1, rgb(FOCUS), "rail");
    assert_px(&img, far, g.y - 2, rgb(FOCUS), "rail");
    assert_px(&img, far, g.y - 3, rgb(BEVEL_LO), "frame top edge");
    assert_px(&img, far, g.y - 4, rgb(DESKTOP), "above the frame, beside the tab");
    // The frame's 1pt edge.
    assert_px(&img, g.x - 1, g.y + 100, rgb(BEVEL_LO), "frame left edge");
    assert_px(&img, g.x + g.w, g.y + 100, rgb(BEVEL_LO), "frame right edge");
    assert_px(&img, g.x + 100, g.y + g.h, rgb(BEVEL_LO), "frame bottom edge");
    // The hard shadow: 3pt right and down, not blurred.
    let right = g.x + g.w + 1; // first column past the frame
    assert_px(&img, right, g.y + 100, rgb(SHADOW_ON_DESKTOP), "shadow right");
    assert_px(&img, right + 2, g.y + 100, rgb(SHADOW_ON_DESKTOP), "shadow right, 3rd column");
    assert_px(&img, right + 3, g.y + 100, rgb(DESKTOP), "past the shadow");
    let bottom = g.y + g.h + 1;
    assert_px(&img, g.x + 100, bottom + 2, rgb(SHADOW_ON_DESKTOP), "shadow bottom");
    assert_px(&img, g.x + 100, bottom + 3, rgb(DESKTOP), "past the shadow");
    // Offset 3pt: the frame's left edge casts nothing straight down.
    assert_px(&img, g.x - 1, bottom, rgb(DESKTOP), "no shadow left of the offset");
    assert_px(&img, g.x + 2, bottom, rgb(SHADOW_ON_DESKTOP), "shadow starts 3pt in");
    // The frame's right shadow starts 3pt down.
    assert_px(&img, right, g.y - 3, rgb(DESKTOP), "no shadow beside the frame's top");
}

#[test]
fn decorations_gles() {
    check_decorations(None);
}

#[test]
fn decorations_pixman() {
    check_decorations(Some("pixman"));
}

#[test]
fn unfocused_tabs_are_idle() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.a", 0x2222cc).title("A"));
    let a = c.wait_window("test.a");
    let _b = c.client(Spec::new("test.b", 0x22cc22).title("B"));
    c.wait_windows("b focused", |ws| ws.iter().any(|w| w.app_id == "test.b" && w.focused));
    let ws = c.windows();
    let a2 = ws.iter().find(|w| w.app_id == "test.a").unwrap();
    let b = ws.iter().find(|w| w.app_id == "test.b").unwrap();
    assert_eq!(a.geometry, a2.geometry);
    assert_ne!((a.geometry.x, a.geometry.y), (b.geometry.x, b.geometry.y), "the second window cascades");
    c.settle();
    let img = c.screenshot();
    // B's tab is amber; A's (cascaded up-left of B, so its tab shows) idle.
    assert_px(&img, b.geometry.x + 2, b.geometry.y - 14, rgb(FOCUS), "focused tab");
    assert_px(&img, a.geometry.x + 2, a.geometry.y - 14, rgb(TAB_IDLE), "idle tab");
    // A's rail isn't amber.
    let rail = img.px(a.geometry.x + 5, a.geometry.y - 1);
    assert!(!near(rail, rgb(FOCUS)), "idle rail");
}

#[test]
fn ipc_focus_raises_and_activates() {
    let c = Compositor::start(opts(None));
    let a = c.client(Spec::new("test.a", 0x2222cc));
    let _wa = c.wait_window("test.a");
    let b = c.client(Spec::new("test.b", 0x22cc22));
    let wb = c.wait_window("test.b");
    b.wait_seen("b activated", |s| s.activated);
    let wa = c.window("test.a").unwrap();
    c.ok(Request::Focus { id: wa.id });
    a.wait_seen("a activated", |s| s.activated);
    b.wait_seen("b deactivated", |s| !s.activated);
    let ws = c.windows();
    assert_eq!(ws.last().unwrap().id, wa.id, "raised to the top");
    assert!(ws.iter().find(|w| w.id == wb.id).is_some_and(|w| !w.focused));
}

#[test]
fn stacks_share_a_frame_and_switch() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.a", 0x2222cc).title("Alpha"));
    let wa = c.wait_window("test.a");
    let _b = c.client(Spec::new("test.b", 0x22cc22).title("Beta").size(300, 200));
    let wb = c.wait_window("test.b");
    c.ok(Request::Stack { a: wa.id, b: wb.id });
    let ws = c.wait_windows("b resized into a's frame", |ws| {
        let a = ws.iter().find(|w| w.id == wa.id).unwrap();
        let b = ws.iter().find(|w| w.id == wb.id).unwrap();
        a.geometry == b.geometry && a.group.is_some() && a.group == b.group
    });
    let b = ws.iter().find(|w| w.id == wb.id).unwrap();
    assert!(b.group_active && b.focused);
    assert_eq!(b.group_members, vec![wa.id, wb.id]);
    c.settle();
    let g = b.geometry;
    let img = c.screenshot();
    assert_px(&img, g.x + 20, g.y + 20, rgb(0x22cc22), "B shows");
    // Two tabs side by side: Alpha's (background) then Beta's (focused).
    assert_px(&img, g.x + 2, g.y - 14, rgb(0x2a2e45), "Alpha's background tab");
    let row: Vec<[u8; 3]> = (g.x..g.x + g.w).map(|x| img.px(x, g.y - 14)).collect();
    let amber = row.iter().position(|p| near(*p, rgb(FOCUS))).expect("Beta's focused tab");
    assert!(amber > 64, "Beta's tab comes after Alpha's");
    // Showing Alpha (clicking its tab).
    c.ok(Request::Synthesize {
        events: vec![
            Synthetic::PointerMotion { x: g.x as f64 + 30.0, y: g.y as f64 - 12.0 },
            Synthetic::Click { button: "left".into() },
        ],
    });
    c.wait_windows("alpha shows", |ws| ws.iter().any(|w| w.id == wa.id && w.group_active && w.focused));
    c.settle();
    let img = c.screenshot();
    assert_px(&img, g.x + 20, g.y + 20, rgb(0x2222cc), "A shows");
}

#[test]
fn desks_hide_and_show_windows() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.a", 0xcc2222));
    let wa = c.wait_window("test.a");
    c.settle();
    let g = wa.geometry;
    let img = c.screenshot();
    assert_px(&img, g.x + 10, g.y + 10, rgb(0xcc2222), "on desk 1");
    c.ok(Request::SetDesk { desk: DeskSel::Index(2) });
    c.settle();
    let img = c.screenshot();
    assert_px(&img, g.x + 10, g.y + 10, rgb(DESKTOP), "gone on desk 2");
    assert_px(&img, g.x + 2, g.y - 14, rgb(DESKTOP), "its tab too");
    c.ok(Request::MoveToDesk { id: Some(wa.id), desk: DeskSel::Index(2), follow: false });
    c.settle();
    let img = c.screenshot();
    assert_px(&img, g.x + 10, g.y + 10, rgb(0xcc2222), "moved to desk 2");
    let w = c.window("test.a").unwrap();
    assert_eq!(w.desk, Some(2));
}

#[test]
fn clicking_focuses_and_dragging_the_tab_moves() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.a", 0x2222cc));
    let wa = c.wait_window("test.a");
    let _b = c.client(Spec::new("test.b", 0x22cc22));
    c.wait_windows("b focused", |ws| ws.iter().any(|w| w.app_id == "test.b" && w.focused));
    // Click A's content where B doesn't cover it.
    let p = (wa.geometry.x as f64 + 5.0, wa.geometry.y as f64 + 5.0);
    c.ok(Request::Synthesize {
        events: vec![Synthetic::PointerMotion { x: p.0, y: p.1 }, Synthetic::Click { button: "left".into() }],
    });
    c.wait_windows("a focused by a click", |ws| ws.iter().any(|w| w.id == wa.id && w.focused));
    // Drag A by its tab.
    let tab = (wa.geometry.x as f64 + 60.0, wa.geometry.y as f64 - 12.0);
    c.ok(Request::Synthesize {
        events: vec![
            Synthetic::PointerMotion { x: tab.0, y: tab.1 },
            Synthetic::PointerButton { button: "left".into(), pressed: true },
            Synthetic::PointerMotion { x: tab.0 + 20.0, y: tab.1 + 10.0 },
            Synthetic::PointerMotion { x: tab.0 + 100.0, y: tab.1 + 50.0 },
            Synthetic::PointerButton { button: "left".into(), pressed: false },
        ],
    });
    let w = c.window("test.a").unwrap();
    assert_eq!((w.geometry.x, w.geometry.y), (wa.geometry.x + 100, wa.geometry.y + 50));
}

#[test]
fn the_close_box_closes() {
    let c = Compositor::start(opts(None));
    let a = c.client(Spec::new("test.a", 0x2222cc));
    let wa = c.wait_window("test.a");
    let close = (wa.geometry.x as f64 + 10.0, wa.geometry.y as f64 - 14.0);
    c.ok(Request::Synthesize { events: vec![Synthetic::PointerMotion { x: close.0, y: close.1 }, Synthetic::Click { button: "left".into() }] });
    a.wait_seen("close", |s| s.close_requested);
    c.wait_windows("closed", |ws| ws.is_empty());
}

#[test]
fn client_side_decorations_get_the_tab_only() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.csd", 0x22cccc).csd());
    let w = c.wait_window("test.csd");
    assert!(w.client_side_decorations);
    c.settle();
    let g = w.geometry;
    let img = c.screenshot();
    assert_px(&img, g.x + 2, g.y - 10, rgb(FOCUS), "the tab sits right on the content");
    assert_px(&img, g.x + 2, g.y + 2, rgb(0x22cccc), "no rail over the content");
    assert_px(&img, g.x - 1, g.y + 100, rgb(DESKTOP), "no frame");
}

#[test]
fn fill_takes_the_usable_area() {
    let c = Compositor::start(opts(None));
    let a = c.client(Spec::new("test.a", 0x2222cc));
    let wa = c.wait_window("test.a");
    c.ok(Request::Fill { id: Some(wa.id), state: Some(true) });
    a.wait_seen("maximized", |s| s.maximized && s.size == (1280 - 2, 800 - 26));
    let w = c.wait_windows("filled", |ws| ws[0].fill && ws[0].geometry.w == 1278);
    assert_eq!(w[0].geometry, aizome_ipc::Rect::new(1, 25, 1278, 774));
    c.ok(Request::Fill { id: Some(wa.id), state: Some(false) });
    a.wait_seen("restored", |s| !s.maximized && s.size == (400, 300));
}

#[test]
fn idle_means_no_frames() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.a", 0x2222cc));
    c.wait_window("test.a");
    c.settle();
    let before = frames(&c);
    std::thread::sleep(std::time::Duration::from_millis(400));
    let after = frames(&c);
    assert_eq!(before, after, "nothing changed, nothing drawn");
    // Moving a window draws, without the client redrawing.
    let w = c.window("test.a").unwrap();
    c.ok(Request::Move { id: Some(w.id), x: w.geometry.x + 30, y: w.geometry.y });
    c.settle();
    assert!(frames(&c) > after);
}

fn frames(c: &Compositor) -> u64 {
    match c.request(Request::Stats { reset: false }) {
        aizome_ipc::Response::Stats { stats } => stats.outputs[0].frames_rendered,
        r => panic!("{r:?}"),
    }
}

#[test]
fn austere_has_no_shadow() {
    let c = Compositor::start(opts(None));
    let _a = c.client(Spec::new("test.a", 0xcc2222));
    let w = c.wait_window("test.a");
    c.settle();
    let g = w.geometry;
    let img = c.screenshot();
    assert_px(&img, g.x + g.w + 1, g.y + 100, rgb(DESKTOP), "no shadow right");
    assert_px(&img, g.x + 100, g.y + g.h + 2, rgb(DESKTOP), "no shadow below");
    assert_px(&img, g.x + 2, g.y - 14, rgb(FOCUS), "the tab is still there");
}

#[test]
fn long_titles_keep_the_tab_short() {
    let c = Compositor::start(opts(None));
    let long = "callen@ganymede: ~/work/a/very/long/path/that/goes/on/and/on/for/ever/and/ever/sidestep";
    let _a = c.client(Spec::new("test.a", 0xcc2222).size(900, 300).title(long));
    let w = c.wait_window("test.a");
    c.settle();
    let g = w.geometry;
    let img = c.screenshot();
    // A lone tab is at most 360pt wide: past that, the frame's top edge.
    assert_px(&img, g.x - 1 + 359, g.y - 14, rgb(BEVEL_LO), "the tab's right border");
    assert_px(&img, g.x - 1 + 365, g.y - 14, rgb(DESKTOP), "beside the tab");
}
