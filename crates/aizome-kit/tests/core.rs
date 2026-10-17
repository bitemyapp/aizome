//! The kit's core, headlessly: layout, damage, the focus chain, menus'
//! shortcuts and the Edit roles.

use aizome_kit::column;
use aizome_kit::prelude::*;
use aizome_kit::sidestep_ui::{Key, Modifiers, NamedKey};
use aizome_kit::testing::Harness;

/// A program whose view is a function of a counter, for poking at.
struct Probe {
    view: fn(&Probe) -> Element<Msg>,
    count: u32,
    on: bool,
    log: Vec<Msg>,
    allow_close: bool,
    menus: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Bump,
    Toggle(bool),
    Named(&'static str),
}

impl Probe {
    fn new(view: fn(&Probe) -> Element<Msg>) -> Probe {
        Probe { view, count: 0, on: false, log: Vec::new(), allow_close: true, menus: false }
    }
}

impl Program for Probe {
    type Msg = Msg;
    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Probe", "Probe", "Test")
    }
    fn launched(&mut self, cx: &mut Cx<Msg>) {
        cx.open_window(WindowSpec::new("Probe").size(400.0, 300.0));
    }
    fn update(&mut self, msg: Msg, _cx: &mut Cx<Msg>) {
        match &msg {
            Msg::Bump => self.count += 1,
            Msg::Toggle(b) => self.on = *b,
            Msg::Named(_) => {}
        }
        self.log.push(msg);
    }
    fn view(&self, _w: WindowKey) -> Element<Msg> {
        (self.view)(self)
    }
    fn menus(&self) -> MenuBar<Msg> {
        let bar = MenuBar::standard(&self.info());
        if !self.menus {
            return bar;
        }
        bar.with_file_menu(
            Menu::new("File")
                .item(MenuItem::new("New", Msg::Named("new")).shortcut(Shortcut::cmd('N')))
                .item(MenuItem::new("Export…", Msg::Named("export")).shortcut(Shortcut::cmd('E').shift())),
        )
    }
    fn close_requested(&mut self, _w: WindowKey, _cx: &mut Cx<Msg>) -> bool {
        self.log.push(Msg::Named("close?"));
        self.allow_close
    }
}

fn approx(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 0.01 && (a.y0 - b.y0).abs() < 0.01 && (a.x1 - b.x1).abs() < 0.01 && (a.y1 - b.y1).abs() < 0.01
}

#[test]
fn rows_share_leftover_room_by_portion() {
    let h = Harness::new(Probe::new(|_| {
        column![row![
            container(text("a")).width(100.0).height(20.0).id("fixed"),
            container(text("b")).width(Length::FillPortion(1)).height(20.0).id("one"),
            container(text("c")).width(Length::FillPortion(3)).height(20.0).id("three"),
        ]
        .spacing(10.0)
        .padding(20.0)
        .width(Fill)]
        .into()
    }));
    let w = h.window();
    // 400 wide less 40 padding, 20 spacing and 100 fixed: 240 to share 1:3.
    assert!(approx(h.find_id(w, "fixed").unwrap(), Rect::new(20.0, 20.0, 120.0, 40.0)), "{:?}", h.find_id(w, "fixed"));
    assert!(approx(h.find_id(w, "one").unwrap(), Rect::new(130.0, 20.0, 190.0, 40.0)), "{:?}", h.find_id(w, "one"));
    assert!(approx(h.find_id(w, "three").unwrap(), Rect::new(200.0, 20.0, 380.0, 40.0)), "{:?}", h.find_id(w, "three"));
}

#[test]
fn columns_stretch_across_and_align() {
    let h = Harness::new(Probe::new(|_| {
        column![
            container(text("wide")).width(Fill).height(30.0).id("wide"),
            container(text("narrow")).width(50.0).height(30.0).id("narrow"),
        ]
        .align(Align::Center)
        .width(Fill)
        .into()
    }));
    let w = h.window();
    assert!(approx(h.find_id(w, "wide").unwrap(), Rect::new(0.0, 0.0, 400.0, 30.0)));
    assert!(approx(h.find_id(w, "narrow").unwrap(), Rect::new(175.0, 30.0, 225.0, 60.0)));
}

#[test]
fn containers_pad_and_place_their_child() {
    let h = Harness::new(Probe::new(|_| {
        container(container(text("x")).width(40.0).height(20.0).id("child"))
            .padding([10.0, 20.0])
            .width(200.0)
            .height(100.0)
            .align_x(Align::End)
            .align_y(Align::Center)
            .into()
    }));
    let w = h.window();
    assert!(approx(h.find_id(w, "child").unwrap(), Rect::new(140.0, 40.0, 180.0, 60.0)), "{:?}", h.find_id(w, "child"));
}

fn controls(p: &Probe) -> Element<Msg> {
    column![
        button("One").on_press(Msg::Bump).id("one"),
        button("Off").id("disabled"),
        toggle(p.on).on_toggle(Msg::Toggle).id("toggle"),
        text(format!("count {}", p.count)).id("count"),
        button("Two").on_press(Msg::Named("two")).id("two"),
    ]
    .spacing(8.0)
    .padding(16.0)
    .into()
}

#[test]
fn hovering_redraws_only_what_it_touches() {
    let mut h = Harness::new(Probe::new(controls));
    let w = h.window();
    h.take_damage(w);
    let one = h.find_id(w, "one").unwrap();
    h.move_to(w, one.center());
    let damage = h.take_damage(w);
    assert!(!damage.is_empty(), "hovering a button redraws it");
    for d in &damage {
        assert!(one.inflate(4.0, 4.0).contains_rect(*d), "{d:?} is within the button {one:?}");
    }
    // Moving within it redraws nothing.
    h.move_to(w, one.center() + Vec2::new(2.0, 0.0));
    assert!(h.take_damage(w).is_empty());
}

#[test]
fn toggling_redraws_the_switch_and_nothing_else() {
    let mut h = Harness::new(Probe::new(controls));
    let w = h.window();
    let toggle = h.find_id(w, "toggle").unwrap();
    h.move_to(w, Point::new(390.0, 290.0));
    h.take_damage(w);
    h.click(w, toggle.center());
    assert!(h.program().on);
    let damage = h.take_damage(w);
    assert!(!damage.is_empty());
    for d in &damage {
        assert!(toggle.inflate(4.0, 4.0).contains_rect(*d), "{d:?} is within the switch {toggle:?}");
    }
}

#[test]
fn a_changed_text_redraws_its_old_and_new_place() {
    let mut h = Harness::new(Probe::new(controls));
    let w = h.window();
    let count = h.find_id(w, "count").unwrap();
    h.take_damage(w);
    h.send(Msg::Bump);
    let damage = h.take_damage(w);
    assert!(damage.iter().any(|d| d.intersect(count).area() > 0.0));
    let all: f64 = damage.iter().map(|d| d.area()).sum();
    assert!(all < 400.0 * 300.0 / 4.0, "a small part of the window: {damage:?}");
}

#[test]
fn tab_walks_the_focus_chain_skipping_what_cant_be_used() {
    let mut h = Harness::new(Probe::new(controls));
    let w = h.window();
    h.focus_window(w, true);
    let order: Vec<Option<String>> = (0..4)
        .map(|_| {
            h.named(w, NamedKey::Tab, Modifiers::NONE);
            h.focused_name(w)
        })
        .collect();
    assert_eq!(order, [Some("one".into()), Some("toggle".into()), Some("two".into()), Some("one".into())]);
    h.named(w, NamedKey::Tab, Modifiers::SHIFT);
    assert_eq!(h.focused_name(w).as_deref(), Some("two"));
    // Space works the focused control; Return too.
    h.key(w, Key::Character(" ".into()), Modifiers::NONE);
    assert_eq!(h.program().log.last(), Some(&Msg::Named("two")));
    h.named(w, NamedKey::Tab, Modifiers::SHIFT);
    h.named(w, NamedKey::Enter, Modifiers::NONE);
    assert!(h.program().on, "Return flipped the focused switch");
}

#[test]
fn menu_shortcuts_send_their_messages() {
    let mut p = Probe::new(controls);
    p.menus = true;
    let mut h = Harness::new(p);
    let w = h.window();
    h.command(w, 'n');
    assert_eq!(h.program().log.last(), Some(&Msg::Named("new")));
    h.key(w, Key::Character("E".into()), Modifiers::SUPER | Modifiers::SHIFT);
    assert_eq!(h.program().log.last(), Some(&Msg::Named("export")));
    // Quit asks every window.
    h.program_mut().allow_close = false;
    h.command(w, 'q');
    assert_eq!(h.program().log.last(), Some(&Msg::Named("close?")));
    assert!(!h.platform.quit);
    h.program_mut().allow_close = true;
    h.command(w, 'q');
    assert!(h.platform.quit);
}

#[test]
fn keys_find_their_widget_after_a_rebuild() {
    let mut h = Harness::new(Probe::new(|p| {
        let mut items: Vec<Element<Msg>> = Vec::new();
        if p.count > 0 {
            items.push(text("new first").into());
        }
        items.push(toggle(p.on).on_toggle(Msg::Toggle).key("t").id("t").into());
        items.push(button("Bump").on_press(Msg::Bump).id("b").into());
        aizome_kit::widgets::layout::column(items).into()
    }));
    let w = h.window();
    h.focus_window(w, true);
    h.named(w, NamedKey::Tab, Modifiers::NONE);
    assert_eq!(h.focused_name(w).as_deref(), Some("t"));
    h.send(Msg::Bump);
    // A sibling came before it; the key keeps its focus.
    assert_eq!(h.focused_name(w).as_deref(), Some("t"));
}

#[test]
fn window_sizes_are_the_contents_under_an_in_window_menu_bar() {
    let h = Harness::with(Probe::new(|_| text("hi").into()), aizome_kit::theme::Appearance::default(), true);
    let w = h.window();
    let bar = h.theme().metrics.target;
    // Asked for 400 × 300 of content: the bar comes on top.
    assert_eq!(h.size(w), Size::new(400.0, 300.0 + bar));
    let win = *h.platform.windows.keys().next().expect("a window");
    assert_eq!(h.platform.windows[&win].spec.size, Size::new(400.0, 300.0 + bar));
    // Where the shell shows the menus, the size is the window's.
    let h2 = Harness::new(Probe::new(|_| text("hi").into()));
    assert_eq!(h2.size(h2.window()), Size::new(400.0, 300.0));
}

#[test]
fn text_limited_to_lines_wraps_up_to_them() {
    const WORDS: &str = "Operations stay in points. Each window renders at its output's scale: the integer one wherever it can.";
    let h = Harness::new(Probe::new(|_| {
        column![
            container(text(WORDS)).width(120.0).id("one-line"),
            container(text(WORDS).max_lines(2)).width(120.0).id("two-lines"),
            container(text(WORDS).wrap()).width(120.0).id("all-lines"),
        ]
        .into()
    }));
    let w = h.window();
    let one = h.find_id(w, "one-line").unwrap().height();
    let two = h.find_id(w, "two-lines").unwrap().height();
    let all = h.find_id(w, "all-lines").unwrap().height();
    assert!(two > one * 1.6 && two < one * 2.4, "two lines: {two} (one: {one})");
    assert!(all > two * 1.5, "unlimited, it takes more: {all}");
}
