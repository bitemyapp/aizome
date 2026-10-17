//! The global menu bar (C2-Desktop's header): the Toolchest button, the
//! focused app's name and menus (or a notification's line), the Desks
//! strip, and at the right the Deskbar's header: the systray's meters and
//! widgets, `N apps ⌄` when the Deskbar is folded, and the clock.

use aizome_ipc::DeskInfo;
use aizome_kit::Icon;
use aizome_kit::draw;
use aizome_kit::element::Element;
use aizome_kit::layout::{Align, Point, Rect};
use aizome_kit::widget::SemanticRole;
use aizome_kit::{Layoutable, row, stack};
use aizome_theme::typeface;

use crate::look::{self, Hue};
use crate::actions::Action;
use crate::systray::meters::{Reading, TRACK};
use crate::widgets::drawn::{Press, around, drawn};

/// What the bar's elements say.
#[derive(Clone, Debug, PartialEq)]
pub enum BarMsg {
    /// A title, the Toolchest or a systray widget was pressed: its rectangle.
    Press(Slot, Rect),
    /// The pointer came over a title while a menu is open.
    Hover(Slot),
    /// The notification line was clicked.
    Banner(u32),
    /// A Desk was right-clicked, at.
    DeskMenu(u32, Point),
    Run(Action),
}

/// The bar's height.
pub const HEIGHT: f64 = 24.0;
/// The Deskbar's header at the bar's right: as wide as the Deskbar.
pub const HEADER: f64 = aizome_theme::metrics::DESKBAR_WIDTH;

/// Something in the bar that opens a menu or a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Toolchest,
    /// The focused app's menu `i` (0 is the app menu, under its name).
    Menu(usize),
    Tray(TrayWidget),
    /// `N apps ⌄`.
    Apps,
}

/// The systray's widgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayWidget {
    Network,
    Sound,
    Bluetooth,
    Display,
    Battery,
    Clock,
}

impl TrayWidget {
    pub fn name(self) -> &'static str {
        match self {
            TrayWidget::Network => "network",
            TrayWidget::Sound => "sound",
            TrayWidget::Bluetooth => "bluetooth",
            TrayWidget::Display => "display",
            TrayWidget::Battery => "battery",
            TrayWidget::Clock => "clock",
        }
    }

    pub fn named(n: &str) -> Option<TrayWidget> {
        [TrayWidget::Network, TrayWidget::Sound, TrayWidget::Bluetooth, TrayWidget::Display, TrayWidget::Battery, TrayWidget::Clock]
            .into_iter()
            .find(|w| w.name() == n)
    }
}

/// What the systray shows: each widget's state, `None` where its hardware
/// is absent (or the setting hides it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tray {
    pub network: Option<NetGlyph>,
    /// Output volume 0–1 and mute.
    pub sound: Option<(f32, bool)>,
    /// Powered, and a device connected.
    pub bluetooth: Option<(bool, bool)>,
    /// The built-in panel's brightness 0–1.
    pub display: Option<f32>,
    /// Charge 0–100 and whether it's charging.
    pub battery: Option<(f32, bool)>,
    pub cpu: bool,
    pub memory: bool,
}

/// The network widget's glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetGlyph {
    /// Wi-Fi with 0–4 bars.
    Wifi(u8),
    Wired,
    Vpn,
    Disconnected,
    Off,
}

impl NetGlyph {
    pub fn icon(self) -> Icon {
        match self {
            NetGlyph::Wifi(0) => Icon::WifiZero,
            NetGlyph::Wifi(1) => Icon::WifiLow,
            NetGlyph::Wifi(2 | 3) => Icon::WifiHigh,
            NetGlyph::Wifi(_) => Icon::Wifi,
            NetGlyph::Wired => Icon::Network,
            NetGlyph::Vpn => Icon::Shield,
            NetGlyph::Disconnected => Icon::WifiOff,
            NetGlyph::Off => Icon::WifiOff,
        }
    }
}

pub fn sound_icon(volume: f32, muted: bool) -> Icon {
    if muted || volume <= 0.0 {
        Icon::VolumeX
    } else if volume < 0.34 {
        Icon::Volume
    } else if volume < 0.67 {
        Icon::Volume1
    } else {
        Icon::Volume2
    }
}

pub fn battery_icon(percent: f32, charging: bool) -> Icon {
    if charging {
        Icon::BatteryCharging
    } else if percent < 10.0 {
        Icon::BatteryWarning
    } else if percent < 30.0 {
        Icon::BatteryLow
    } else if percent < 70.0 {
        Icon::BatteryMedium
    } else {
        Icon::BatteryFull
    }
}

/// A notification's line in place of the app's menus.
#[derive(Clone, Debug, PartialEq)]
pub struct Banner {
    pub id: u32,
    pub glyph: Icon,
    pub hue: Hue,
    pub summary: String,
    pub body: String,
}

/// Everything the bar shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BarModel {
    pub app_name: String,
    pub menus: Vec<String>,
    /// What's open (highlighted).
    pub open: Option<Slot>,
    pub desks: Vec<DeskInfo>,
    /// A Desk being renamed in place, and the text so far.
    pub renaming: Option<(u32, String)>,
    pub tray: Tray,
    pub meters: Reading,
    pub clock: String,
    /// `Some(n)` when the Deskbar is folded: `n apps ⌄`.
    pub folded_apps: Option<usize>,
    /// Unread notifications, counted on the `N apps` control when folded.
    pub unread: usize,
    pub banner: Option<Banner>,
    /// This bar's output is the one the Deskbar hangs from.
    pub primary: bool,
}

fn ui_spec(t: &aizome_kit::Theme, size: f64, weight: f32, c: aizome_kit::Color) -> aizome_kit::text::TextSpec {
    look::ui(size * t.metrics.factor, weight, c)
}

/// A title of the bar: text with padding, highlighted while its menu is
/// open.
fn title(label: String, weight: f32, pad_l: f64, pad_r: f64, open: bool, slot: Slot, menu_open: bool) -> Element<BarMsg> {
    let measure = label.clone();
    let paint_label = label.clone();
    drawn(0.0, HEIGHT, (&label, weight.to_bits(), open, pad_l.to_bits()), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        let face = Rect::new(b.x0, 0.0, b.x1, HEIGHT - 2.0);
        let color = if open {
            cx.canvas.fill_rect(face, t.selection());
            t.on_selection()
        } else {
            if cx.is_hovered() {
                cx.canvas.fill_rect(face, t.hover(t.bar()));
            }
            t.text()
        };
        let spec = ui_spec(t, typeface::BODY, weight, color);
        cx.draw_text_in(&paint_label, &spec, Rect::new(pad_l, 0.0, b.x1 - pad_r + 4.0, HEIGHT - 1.0), Align::Start);
    })
    .fit(move |cx| {
        let t = cx.theme();
        let spec = ui_spec(t, typeface::BODY, weight, t.text());
        cx.measure(&measure, &spec).width + pad_l + pad_r
    })
    .on_press(move |p: Press| BarMsg::Press(slot, p.rect))
    .on_hover_if(menu_open, BarMsg::Hover(slot))
    .text(label, SemanticRole::MenuItem)
    .into()
}

/// The Toolchest button: the mark and "Toolchest" condensed, a `line`
/// rule at its right.
fn toolchest(open: bool) -> Element<BarMsg> {
    drawn(0.0, HEIGHT, open, move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        let face = Rect::new(0.0, 0.0, b.x1 - 1.0, HEIGHT - 2.0);
        let color = if open {
            cx.canvas.fill_rect(face, t.selection());
            t.on_selection()
        } else {
            if cx.is_hovered() {
                cx.canvas.fill_rect(face, t.hover(t.bar()));
            }
            t.text()
        };
        look::mark(cx.canvas, t, Point::new(10.0, 4.0), 16.0);
        let spec = look::condensed(12.0 * t.metrics.factor, typeface::WEIGHT_SEMIBOLD, color);
        cx.draw_text_in("Toolchest", &spec, Rect::new(32.0, 0.0, b.x1 - 4.0, HEIGHT - 1.0), Align::Start);
        cx.canvas.fill_rect(Rect::new(b.x1 - 1.0, 0.0, b.x1, HEIGHT - 2.0), t.line());
    })
    .fit(|cx| {
        let t = cx.theme();
        let spec = look::condensed(12.0 * t.metrics.factor, typeface::WEIGHT_SEMIBOLD, t.text());
        32.0 + cx.measure("Toolchest", &spec).width + 10.0 + 1.0
    })
    .on_press(|p: Press| BarMsg::Press(Slot::Toolchest, p.rect))
    .text("Toolchest", SemanticRole::MenuItem)
    .into()
}

/// A notification's line: the app's glyph, the summary and the body's
/// first line.
fn banner(b: &Banner) -> Element<BarMsg> {
    let b2 = b.clone();
    drawn(0.0, HEIGHT, (b.id, &b.summary, &b.body), move |cx| {
        let t = cx.theme();
        let bounds = cx.bounds();
        b2.glyph.draw(cx.canvas, Point::new(12.0, 4.0), 16.0, b2.hue.color(t));
        let s = ui_spec(t, typeface::BODY, typeface::WEIGHT_SEMIBOLD, t.text());
        let w = cx.draw_text_in(&b2.summary, &s, Rect::new(36.0, 0.0, bounds.x1 * 0.55, HEIGHT - 1.0), Align::Start);
        let body = ui_spec(t, typeface::BODY, typeface::WEIGHT_REGULAR, t.text2());
        cx.draw_text_in(&b2.body, &body, Rect::new(36.0 + w + 10.0, 0.0, bounds.x1 - 8.0, HEIGHT - 1.0), Align::Start);
    })
    .on_click(BarMsg::Banner(b.id))
    .text(format!("{} {}", b.summary, b.body), SemanticRole::StatusBar)
    .into()
}

/// The Desks strip: `1 Mail  2 Build  3 Write  +`, bordered, the current
/// Desk on `selection`.
fn desks(model: &BarModel) -> Element<BarMsg> {
    let mut items: Vec<Element<BarMsg>> = Vec::new();
    let n = model.desks.len();
    for (i, d) in model.desks.iter().enumerate() {
        let index = d.index;
        let current = d.current;
        let renaming = model.renaming.as_ref().filter(|(r, _)| *r == index).map(|(_, s)| s.clone());
        let name = renaming.clone().unwrap_or_else(|| d.name.clone());
        let label_num = index.to_string();
        let show_name = name != label_num || renaming.is_some();
        let (num, nm) = (label_num.clone(), name.clone());
        let last = i + 1 == n;
        items.push(
            drawn(0.0, 18.0, (&name, current, renaming.is_some(), last), move |cx| {
                let t = cx.theme();
                let b = cx.bounds();
                let (fg, numc) = if current {
                    cx.canvas.fill_rect(Rect::new(0.0, 0.0, b.x1, b.y1), t.selection());
                    (t.on_selection(), t.on_selection())
                } else {
                    if cx.is_hovered() {
                        cx.canvas.fill_rect(Rect::new(0.0, 0.0, b.x1, b.y1), t.hover(t.bar()));
                    }
                    (t.text2(), t.text3())
                };
                let ns = look::mono(12.0 * t.metrics.factor, if current { typeface::WEIGHT_SEMIBOLD } else { typeface::WEIGHT_REGULAR }, numc);
                let w = cx.draw_text_in(&num, &ns, Rect::new(9.0, 0.0, b.x1, b.y1), Align::Start);
                if show_name {
                    let weight = if current { typeface::WEIGHT_TAB } else { typeface::WEIGHT_REGULAR };
                    let s = ui_spec(t, 12.0, weight, fg);
                    let x = 9.0 + w + 5.0;
                    let tw = cx.draw_text_in(&nm, &s, Rect::new(x, 0.0, b.x1 - 9.0, b.y1), Align::Start);
                    if renaming.is_some() {
                        // The caret at the end of what's typed.
                        cx.canvas.fill_rect(Rect::new(x + tw + 1.0, 3.0, x + tw + 2.0, b.y1 - 3.0), t.focus());
                    }
                }
                // The rule between Desks.
                cx.canvas.fill_rect(Rect::new(b.x1 - 1.0, 0.0, b.x1, b.y1), t.line());
            })
            .fit({
                let (num, name) = (index.to_string(), name.clone());
                move |cx| {
                    let t = cx.theme().clone();
                    let ns = look::mono(12.0 * t.metrics.factor, typeface::WEIGHT_REGULAR, t.text3());
                    let mut w = 9.0 + cx.measure(&num, &ns).width + 9.0 + 1.0;
                    if show_name {
                        let s = ui_spec(&t, 12.0, typeface::WEIGHT_TAB, t.text());
                        w += 5.0 + cx.measure(&name, &s).width;
                    }
                    w
                }
            })
            .on_click(BarMsg::Run(crate::actions::wm::set_desk(index)))
            .on_double_click(BarMsg::Run(Action::RenameDesk { index }))
            .on_right_click(move |p: Press| BarMsg::DeskMenu(index, p.at))
            .text(format!("{index} {name}"), SemanticRole::Tab)
            .key(("desk", d.id))
            .into(),
        );
    }
    items.push(
        drawn(24.0, 18.0, "+", |cx| {
            let t = cx.theme();
            let b = cx.bounds();
            if cx.is_hovered() {
                cx.canvas.fill_rect(b, t.hover(t.bar()));
            }
            let s = ui_spec(t, 12.0, typeface::WEIGHT_REGULAR, t.text3());
            cx.draw_text_in("+", &s, b, Align::Center);
        })
        .on_click(BarMsg::Run(Action::NewDesk))
        .text("New Desk", SemanticRole::Button)
        .key("new-desk")
        .into(),
    );
    let strip = row(items);
    // The strip's frame: a 1pt `bevel_lo` edge and a highlight under it.
    around(strip, [1.0, 1.0, 2.0, 1.0], aizome_kit::Size::ZERO, "strip-frame", |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        draw::border(cx.canvas, Rect::new(0.0, 0.0, b.x1, b.y1 - 1.0), t.bevel_lo(), 1.0);
        cx.canvas.fill_rect(Rect::new(0.0, b.y1 - 1.0, b.x1, b.y1), t.bevel_hi());
    })
    .key("desks")
    .into()
}

/// The CPU meter: a 3pt bar per core (up to eight), `green` at rest and
/// `amber` when hot, on `line2` tracks 14pt tall.
fn cpu_meter(m: &Reading) -> Element<BarMsg> {
    let heights = m.bar_heights();
    let hot = m.hot();
    let n = heights.len().max(1);
    let w = n as f64 * 4.0 - 1.0;
    drawn(w, HEIGHT, (heights.clone(), hot.clone()), move |cx| {
        let t = cx.theme();
        let top = ((HEIGHT - TRACK) / 2.0).round();
        for (i, h) in heights.iter().enumerate() {
            let x = i as f64 * 4.0;
            cx.canvas.fill_rect(Rect::new(x, top, x + 3.0, top + TRACK), t.line2());
            let c = if hot[i] { Hue::Amber.color(t) } else { Hue::Green.color(t) };
            cx.canvas.fill_rect(Rect::new(x, top + TRACK - *h as f64, x + 3.0, top + TRACK), c);
        }
    })
    .text(format!("CPU {:.0}%", m.total), SemanticRole::Meter)
    .key("cpu")
    .into()
}

/// The memory meter: one 4pt bar in `blue`.
fn memory_meter(m: &Reading) -> Element<BarMsg> {
    let h = m.memory_height();
    drawn(4.0, HEIGHT, h, move |cx| {
        let t = cx.theme();
        let top = ((HEIGHT - TRACK) / 2.0).round();
        cx.canvas.fill_rect(Rect::new(0.0, top, 4.0, top + TRACK), t.line2());
        cx.canvas.fill_rect(Rect::new(0.0, top + TRACK - h as f64, 4.0, top + TRACK), Hue::Blue.color(t));
    })
    .text(format!("Memory {:.0}%", m.memory), SemanticRole::Meter)
    .key("memory")
    .into()
}

/// A systray widget: its glyph (and the battery's percentage).
fn tray_icon(widget: TrayWidget, icon: Icon, text: Option<String>, open: bool, label: String) -> Element<BarMsg> {
    let measure = text.clone();
    drawn(0.0, HEIGHT, (icon, text.clone(), open), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        let face = Rect::new(0.0, 2.0, b.x1, HEIGHT - 3.0);
        let fg = if open {
            cx.canvas.fill_rect(face.inflate(3.0, 0.0), t.selection());
            t.on_selection()
        } else {
            if cx.is_hovered() {
                cx.canvas.fill_rect(face.inflate(3.0, 0.0), t.hover(t.bar()));
            }
            t.text2()
        };
        icon.draw(cx.canvas, Point::new(0.0, ((HEIGHT - 15.0) / 2.0).round()), 15.0, fg);
        if let Some(s) = &text {
            let spec = look::digits(11.5 * t.metrics.factor, typeface::WEIGHT_REGULAR, fg);
            cx.draw_text_in(s, &spec, Rect::new(19.0, 0.0, b.x1, HEIGHT - 1.0), Align::Start);
        }
    })
    .fit(move |cx| {
        let t = cx.theme();
        match &measure {
            Some(s) => 19.0 + cx.measure(s, &look::digits(11.5 * t.metrics.factor, typeface::WEIGHT_REGULAR, t.text2())).width,
            None => 15.0,
        }
    })
    .on_press(move |p: Press| BarMsg::Press(Slot::Tray(widget), p.rect))
    .text(label, SemanticRole::Button)
    .key(("tray", widget))
    .into()
}

/// The clock, alone in its element so its tick repaints only itself.
fn clock(label: &str, open: bool) -> Element<BarMsg> {
    let s = label.to_owned();
    let m = label.to_owned();
    drawn(0.0, HEIGHT, (s.clone(), open), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        let fg = if open {
            cx.canvas.fill_rect(Rect::new(-4.0, 2.0, b.x1 + 4.0, HEIGHT - 3.0), t.selection());
            t.on_selection()
        } else {
            t.text()
        };
        let spec = look::digits(12.5 * t.metrics.factor, typeface::WEIGHT_SEMIBOLD, fg);
        cx.draw_text_in(&s, &spec, Rect::new(0.0, 0.0, b.x1, HEIGHT - 1.0), Align::End);
    })
    .fit(move |cx| {
        let t = cx.theme();
        cx.measure(&m, &look::digits(12.5 * t.metrics.factor, typeface::WEIGHT_SEMIBOLD, t.text())).width
    })
    .on_press(|p: Press| BarMsg::Press(Slot::Tray(TrayWidget::Clock), p.rect))
    .text(label.to_owned(), SemanticRole::Label)
    .key("clock")
    .into()
}

/// `N apps ⌄`: the folded Deskbar.
fn apps_control(n: usize, unread: usize, open: bool) -> Element<BarMsg> {
    let label = if n == 1 { "1 app".to_owned() } else { format!("{n} apps") };
    let m = label.clone();
    drawn(0.0, HEIGHT, (label.clone(), unread, open), move |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        if cx.is_hovered() || open {
            cx.canvas.fill_rect(Rect::new(-4.0, 2.0, b.x1 + 2.0, HEIGHT - 3.0), t.hover(t.bar()));
        }
        let s = ui_spec(t, 12.0, typeface::WEIGHT_SEMIBOLD, t.text2());
        let w = cx.draw_text_in(&label, &s, Rect::new(0.0, 0.0, b.x1, HEIGHT - 1.0), Align::Start);
        let mut x = w + 9.0;
        Icon::ChevronDown.draw(cx.canvas, Point::new(x, 6.0), 12.0, t.text3());
        x += 12.0;
        if unread > 0 {
            let r = look::digits(11.0 * t.metrics.factor, typeface::WEIGHT_SEMIBOLD, Hue::Red.color(t));
            cx.draw_text_in(&unread.to_string(), &r, Rect::new(x + 6.0, 0.0, b.x1, HEIGHT - 1.0), Align::Start);
        }
    })
    .fit(move |cx| {
        let t = cx.theme();
        let w = cx.measure(&m, &ui_spec(t, 12.0, typeface::WEIGHT_SEMIBOLD, t.text2())).width + 9.0 + 12.0;
        if unread > 0 { w + 6.0 + 8.0 * unread.to_string().len() as f64 } else { w }
    })
    .on_click(BarMsg::Run(Action::SetDeskbarCollapsed(false)))
    .text(format!("{n} apps"), SemanticRole::Button)
    .key("apps")
    .into()
}

/// The systray region: the Deskbar's header, `HEADER` wide (more if its
/// widgets need it), its left edge in line with the Deskbar's.
fn header(model: &BarModel) -> Element<BarMsg> {
    let open = |w: TrayWidget| model.open == Some(Slot::Tray(w));
    let mut items: Vec<Element<BarMsg>> = Vec::new();
    let tray = &model.tray;
    if tray.cpu {
        items.push(cpu_meter(&model.meters));
    }
    if tray.memory {
        items.push(memory_meter(&model.meters));
    }
    if let Some(n) = model.folded_apps {
        items.push(apps_control(n, model.unread, model.open == Some(Slot::Apps)));
    } else {
        if let Some(g) = tray.network {
            items.push(tray_icon(TrayWidget::Network, g.icon(), None, open(TrayWidget::Network), "Network".into()));
        }
        if let Some((v, m)) = tray.sound {
            items.push(tray_icon(TrayWidget::Sound, sound_icon(v, m), None, open(TrayWidget::Sound), format!("Volume {:.0}%", v * 100.0)));
        }
        if let Some((on, conn)) = tray.bluetooth {
            let icon = if !on {
                Icon::BluetoothOff
            } else if conn {
                Icon::BluetoothConnected
            } else {
                Icon::Bluetooth
            };
            items.push(tray_icon(TrayWidget::Bluetooth, icon, None, open(TrayWidget::Bluetooth), "Bluetooth".into()));
        }
        if let Some(b) = tray.display {
            items.push(tray_icon(TrayWidget::Display, Icon::SunDim, None, open(TrayWidget::Display), format!("Brightness {:.0}%", b * 100.0)));
        }
        if let Some((p, c)) = tray.battery {
            items.push(tray_icon(
                TrayWidget::Battery,
                battery_icon(p, c),
                Some(format!("{p:.0}%")),
                open(TrayWidget::Battery),
                format!("Battery {p:.0}%"),
            ));
        }
    }
    items.push(aizome_kit::spacer().into());
    items.push(clock(&model.clock, open(TrayWidget::Clock)));
    let content = row(items).spacing(9.0).align(Align::Center).height(HEIGHT);
    around(content, [0.0, 10.0, 0.0, 11.0], aizome_kit::Size::new(HEADER, HEIGHT), "header-edge", |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        cx.canvas.fill_rect(Rect::new(0.0, 0.0, 1.0, b.y1 - 1.0), t.bevel_lo());
        cx.canvas.fill_rect(Rect::new(1.0, 0.0, 2.0, b.y1 - 1.0), t.bevel_hi());
    })
    .key("header")
    .into()
}

/// The menu bar of one output.
pub fn view(model: &BarModel) -> Element<BarMsg> {
    let menu_open = matches!(model.open, Some(Slot::Menu(_) | Slot::Toolchest));
    let mut left: Vec<Element<BarMsg>> = vec![toolchest(model.open == Some(Slot::Toolchest))];
    match &model.banner {
        Some(b) => left.push(banner(b)),
        None => {
            if !model.app_name.is_empty() {
                left.push(
                    title(model.app_name.clone(), typeface::WEIGHT_BOLD, 12.0, 9.0, model.open == Some(Slot::Menu(0)), Slot::Menu(0), menu_open)
                        .key("app-name"),
                );
            }
            for (i, m) in model.menus.iter().enumerate().skip(1) {
                left.push(
                    title(m.clone(), typeface::WEIGHT_REGULAR, 9.0, 9.0, model.open == Some(Slot::Menu(i)), Slot::Menu(i), menu_open)
                        .key(("menu", i, m.as_str())),
                );
            }
        }
    }
    let mut items: Vec<Element<BarMsg>> = left;
    items.push(aizome_kit::spacer().into());
    if !model.desks.is_empty() {
        items.push(desks(model));
        items.push(aizome_kit::space(12.0));
    }
    if model.primary {
        items.push(header(model));
    } else {
        // Other outputs' bars keep the clock at their right.
        items.push(aizome_kit::container(clock(&model.clock, false)).padding([0.0, 10.0]).into());
    }
    let bar = row(items).align(Align::Center).height(HEIGHT);
    let ground = drawn(0.0, HEIGHT, "bar-ground", |cx| {
        let t = cx.theme();
        let b = cx.bounds();
        cx.canvas.fill_rect(b, t.bar());
        cx.canvas.fill_rect(Rect::new(0.0, HEIGHT - 2.0, b.x1, HEIGHT - 1.0), t.bevel_hi());
        cx.canvas.fill_rect(Rect::new(0.0, HEIGHT - 1.0, b.x1, HEIGHT), t.bevel_lo());
    });
    stack(vec![ground.into(), bar.into()]).height(HEIGHT).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_follow_levels() {
        assert_eq!(sound_icon(0.5, true), Icon::VolumeX);
        assert_eq!(sound_icon(0.9, false), Icon::Volume2);
        assert_eq!(battery_icon(84.0, false), Icon::BatteryFull);
        assert_eq!(battery_icon(5.0, false), Icon::BatteryWarning);
        assert_eq!(battery_icon(5.0, true), Icon::BatteryCharging);
        assert_eq!(NetGlyph::Wifi(4).icon(), Icon::Wifi);
        assert_eq!(TrayWidget::named("battery"), Some(TrayWidget::Battery));
    }
}
