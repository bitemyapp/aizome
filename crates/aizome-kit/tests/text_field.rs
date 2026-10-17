//! `text_field`: typing, moving, selecting, the clipboard, undo and input
//! methods.

use aizome_kit::column;
use aizome_kit::prelude::*;
use aizome_kit::sidestep_ui::{Key, Modifiers, NamedKey};
use aizome_kit::testing::Harness;

#[derive(Default)]
struct Form {
    name: String,
    secret: String,
    query: String,
    submitted: u32,
}

#[derive(Clone, Debug)]
enum Msg {
    Name(String),
    Secret(String),
    Query(String),
    Submit,
}

impl Program for Form {
    type Msg = Msg;
    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Form", "Form", "Test")
    }
    fn launched(&mut self, cx: &mut Cx<Msg>) {
        cx.open_window(WindowSpec::new("Form").size(360.0, 200.0));
    }
    fn update(&mut self, msg: Msg, _cx: &mut Cx<Msg>) {
        match msg {
            Msg::Name(s) => self.name = s,
            Msg::Secret(s) => self.secret = s,
            Msg::Query(s) => self.query = s,
            Msg::Submit => self.submitted += 1,
        }
    }
    fn view(&self, _w: WindowKey) -> Element<Msg> {
        column![
            text_field(&self.name).placeholder("Name").on_change(Msg::Name).on_submit(Msg::Submit).id("name"),
            text_field(&self.secret).password().on_change(Msg::Secret).id("secret"),
            search_field(&self.query).on_change(Msg::Query).id("search"),
        ]
        .spacing(8.0)
        .padding(12.0)
        .into()
    }
}

fn setup() -> (Harness<Form>, WindowKey) {
    let mut h = Harness::new(Form::default());
    let w = h.window();
    h.focus_window(w, true);
    let name = h.find_id(w, "name").expect("the name field");
    h.click(w, name.center());
    (h, w)
}

const NONE: Modifiers = Modifiers::NONE;

#[test]
fn typing_moving_and_deleting() {
    let (mut h, w) = setup();
    h.type_text(w, "hello world");
    assert_eq!(h.program().name, "hello world");
    h.named(w, NamedKey::ArrowLeft, Modifiers::ALT);
    h.type_text(w, "big ");
    assert_eq!(h.program().name, "hello big world");
    h.named(w, NamedKey::Backspace, Modifiers::ALT);
    assert_eq!(h.program().name, "hello world");
    h.named(w, NamedKey::ArrowLeft, Modifiers::SUPER);
    h.named(w, NamedKey::Delete, NONE);
    assert_eq!(h.program().name, "ello world");
    h.named(w, NamedKey::End, NONE);
    h.named(w, NamedKey::Backspace, NONE);
    assert_eq!(h.program().name, "ello worl");
    h.named(w, NamedKey::Enter, NONE);
    assert_eq!(h.program().submitted, 1);
}

#[test]
fn selecting_copying_and_pasting() {
    let (mut h, w) = setup();
    h.type_text(w, "one two");
    // ⇧⌥← selects "two".
    h.named(w, NamedKey::ArrowLeft, Modifiers::ALT | Modifiers::SHIFT);
    assert_eq!(h.selection(w).as_deref(), Some("two"));
    h.command(w, 'c');
    assert_eq!(h.platform.clipboard.as_deref(), Some("two"));
    h.command(w, 'x');
    assert_eq!(h.program().name, "one ");
    h.named(w, NamedKey::ArrowLeft, Modifiers::SUPER);
    h.command(w, 'v');
    assert_eq!(h.program().name, "twoone ");
    h.command(w, 'a');
    assert_eq!(h.selection(w).as_deref(), Some("twoone "));
    h.type_text(w, "x");
    assert_eq!(h.program().name, "x");
}

#[test]
fn undo_takes_back_a_burst_of_typing_at_once() {
    let (mut h, w) = setup();
    h.type_text(w, "abc");
    h.named(w, NamedKey::Backspace, NONE);
    assert_eq!(h.program().name, "ab");
    h.command(w, 'z');
    assert_eq!(h.program().name, "abc", "the deletion undone");
    h.command(w, 'z');
    assert_eq!(h.program().name, "", "the typing undone, all of it");
    h.key(w, Key::Character("Z".into()), Modifiers::SUPER | Modifiers::SHIFT);
    assert_eq!(h.program().name, "abc");
}

#[test]
fn input_methods_compose_then_commit() {
    let (mut h, w) = setup();
    h.ime(w, None, "にほ");
    assert_eq!(h.program().name, "", "composing isn't the text yet");
    h.ime(w, Some("日本"), "");
    assert_eq!(h.program().name, "日本");
    assert!(h.platform.text_input.values().any(|(on, caret)| *on && caret.is_some()), "the input method knows where the caret is");
}

#[test]
fn passwords_show_dots_and_dont_copy() {
    let (mut h, w) = setup();
    let secret = h.find_id(w, "secret").unwrap();
    h.click(w, secret.center());
    h.type_text(w, "hunter2");
    assert_eq!(h.program().secret, "hunter2");
    h.command(w, 'a');
    h.command(w, 'c');
    assert_eq!(h.platform.clipboard, None);
    assert_eq!(h.selection(w), None);
}

#[test]
fn search_fields_clear() {
    let (mut h, w) = setup();
    let search = h.find_id(w, "search").unwrap();
    h.click(w, search.center());
    h.type_text(w, "kura");
    assert_eq!(h.program().query, "kura");
    h.named(w, NamedKey::Escape, NONE);
    assert_eq!(h.program().query, "");
    h.type_text(w, "x");
    let search = h.find_id(w, "search").unwrap();
    // The clear button at the right end.
    h.click(w, Point::new(search.x1 - 14.0, search.center().y));
    assert_eq!(h.program().query, "");
    let png = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fields.png");
    h.type_text(w, "find me");
    h.save_png(w, &png, 2.0);
}

#[test]
fn clicks_place_the_caret_and_double_clicks_select_words() {
    let (mut h, w) = setup();
    h.type_text(w, "alpha beta");
    let name = h.find_id(w, "name").unwrap();
    // Far right of the text: the end.
    h.click(w, Point::new(name.x1 - 4.0, name.center().y));
    h.type_text(w, "!");
    assert_eq!(h.program().name, "alpha beta!");
    // A double click on the first word.
    h.press(w, Point::new(name.x0 + 12.0, name.center().y), aizome_kit::sidestep_ui::PointerButton::Left, 2);
    h.release(w, Point::new(name.x0 + 12.0, name.center().y), aizome_kit::sidestep_ui::PointerButton::Left);
    assert_eq!(h.selection(w).as_deref(), Some("alpha"));
}
