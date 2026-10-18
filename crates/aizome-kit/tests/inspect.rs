//! What a window writes for tests that drive a running app from outside
//! (`AIZOME_KIT_INSPECT`): its elements, where they are, and their state.

use aizome_kit::column;
use aizome_kit::prelude::*;
use aizome_kit::testing::Harness;

#[derive(Default)]
struct Form {
    on: bool,
}

#[derive(Clone, Debug)]
enum Msg {
    Toggle(bool),
}

impl Program for Form {
    type Msg = Msg;
    fn info(&self) -> AppInfo {
        AppInfo::new("org.aizome.Form", "Form", "Form")
    }
    fn launched(&mut self, cx: &mut Cx<Msg>) {
        cx.open_window(WindowSpec::new("Form").size(320.0, 200.0));
    }
    fn update(&mut self, msg: Msg, _cx: &mut Cx<Msg>) {
        match msg {
            Msg::Toggle(on) => self.on = on,
        }
    }
    fn view(&self, _w: WindowKey) -> Element<Msg> {
        column![
            text("Wi-Fi"),
            toggle(self.on).on_toggle(Msg::Toggle).id("wifi-switch").label("Wi-Fi"),
            button("Join Other Network…"),
        ]
        .spacing(8.0)
        .padding(16.0)
        .into()
    }
}

fn find<'a>(elements: &'a [serde_json::Value], key: &str, value: &str) -> &'a serde_json::Value {
    elements.iter().find(|e| e[key] == value).unwrap_or_else(|| panic!("no element with {key} {value}: {elements:#?}"))
}

#[test]
fn elements_carry_their_names_texts_places_and_states() {
    let mut h = Harness::new(Form::default());
    let w = h.window();
    let elements = h.inspected(w);

    let label = find(&elements, "text", "Wi-Fi");
    assert_eq!(label["visible"], true);
    let switch = find(&elements, "name", "wifi-switch");
    assert_eq!(switch["role"], "switch");
    assert_eq!(switch["label"], "Wi-Fi");
    assert_eq!(switch["checked"], false);
    // The rectangle is in the window's points, and a click at its middle
    // lands on it.
    let r: Vec<f64> = switch["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert!(r[0] >= 16.0 && r[1] > 16.0 && r[2] > 0.0 && r[3] > 0.0, "{r:?}");
    h.click(w, Point::new(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0));
    assert!(h.program().on);
    assert_eq!(find(&h.inspected(w), "name", "wifi-switch")["checked"], true);

    find(&elements, "text", "Join Other Network…");
}
