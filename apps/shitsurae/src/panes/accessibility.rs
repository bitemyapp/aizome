//! Accessibility: seeing (High Contrast, larger text, the pointer's size,
//! no motion), hearing (a flash for the alert sound) and typing (sticky
//! and slow keys). Mostly `[appearance]`, so the same switches as
//! Appearance, gathered where people look for them.

use aizome_config::{ThemeChoice, UiSize};
use aizome_kit::prelude::*;
use aizome_kit::row;
use aizome_theme::{FocusColor, ThemeName};

use crate::extra;
use crate::form;
use crate::pane::{Group, Pane, PaneCx, SettingEntry, Tile, ViewCx};
use crate::preview::{self, Look};

#[derive(Default)]
pub struct Accessibility;

#[derive(Clone, Debug)]
pub enum Msg {
    HighContrast(bool),
    Larger(bool),
    ReducedMotion(bool),
    Cursor(f64),
    Flash(bool),
    Sticky(bool),
    Slow(bool),
    SlowMs(f64),
}

pub const CURSOR_SIZES: [u32; 5] = [24, 32, 40, 48, 64];

impl Pane for Accessibility {
    type Msg = Msg;

    fn id(&self) -> &'static str {
        "accessibility"
    }

    fn title(&self) -> &'static str {
        "Accessibility"
    }

    fn icon(&self) -> Icon {
        Icon::Accessibility
    }

    fn tile(&self) -> Tile {
        Tile::Hanada
    }

    fn group(&self) -> Group {
        Group::System
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["a11y", "vision", "hearing", "motor", "low vision"]
    }

    fn entries(&self) -> Vec<SettingEntry> {
        vec![
            SettingEntry::new("high-contrast", "High Contrast").keywords(&["contrast", "low vision", "readability"]),
            SettingEntry::new("larger-text", "Larger text").keywords(&["font size", "zoom", "big"]),
            SettingEntry::new("cursor-size", "Pointer size").keywords(&["cursor", "mouse pointer", "big cursor"]),
            SettingEntry::new("reduced-motion", "Reduce motion").keywords(&["animation", "vestibular"]),
            SettingEntry::new("flash", "Flash the screen for alerts").keywords(&["visual bell", "deaf", "hearing"]),
            SettingEntry::new("sticky-keys", "Sticky keys").keywords(&["modifier", "one hand", "accessx"]),
            SettingEntry::new("slow-keys", "Slow keys").keywords(&["tremor", "accidental", "accessx"]),
        ]
    }

    fn update(&mut self, msg: Msg, cx: &mut PaneCx<Msg>) {
        match msg {
            Msg::HighContrast(on) => cx.change(|s| {
                let a = &mut s.appearance;
                if on {
                    // Remember what it was, to go back to.
                    let was = match a.theme {
                        ThemeChoice::Day => "day",
                        ThemeChoice::Auto => "auto",
                        _ => "night",
                    };
                    a.extra.insert("theme_before_contrast".into(), toml::Value::String(was.into()));
                    a.theme = ThemeChoice::HighContrast;
                } else {
                    a.theme = match a.extra.remove("theme_before_contrast").as_ref().and_then(toml::Value::as_str) {
                        Some("day") => ThemeChoice::Day,
                        Some("auto") => ThemeChoice::Auto,
                        _ => ThemeChoice::Night,
                    };
                }
            }),
            Msg::Larger(on) => cx.change(|s| s.appearance.size = if on { UiSize::Large } else { UiSize::Medium }),
            Msg::ReducedMotion(on) => cx.change(|s| s.appearance.reduced_motion = on),
            Msg::Cursor(v) => cx.change(|s| {
                let i = (v.round() as usize).min(CURSOR_SIZES.len() - 1);
                s.appearance.cursor_size = CURSOR_SIZES[i];
            }),
            Msg::Flash(on) => cx.change(|s| extra::set(&mut s.sound, "visual_alert", on)),
            Msg::Sticky(on) => cx.change(|s| extra::set(&mut s.input.keyboard.extra, "sticky_keys", on)),
            Msg::Slow(on) => cx.change(|s| extra::set(&mut s.input.keyboard.extra, "slow_keys", on)),
            Msg::SlowMs(v) => cx.change(|s| {
                extra::set(&mut s.input.keyboard.extra, "slow_keys_ms", ((v / 50.0).round() * 50.0) as i64)
            }),
        }
    }

    fn view(&self, vcx: &ViewCx) -> Element<Msg> {
        let s = vcx.settings();
        let a = &s.appearance;
        let hc = a.theme == ThemeChoice::HighContrast;
        let cursor_i = CURSOR_SIZES.iter().position(|c| *c >= a.cursor_size).unwrap_or(0);
        let k = &s.input.keyboard.extra;
        let slow = extra::get_bool(k, "slow_keys", false);
        let slow_ms = extra::get_int(k, "slow_keys_ms", 300) as f64;
        let now = match aizome_kit::theme::Appearance::from(a).theme_name() {
            ThemeName::HighContrast => ThemeName::Night,
            t => t,
        };
        let contrast = aizome_kit::row![
            preview::desktop(Look::Theme(now), a.focus, a.effects, 120.0, 76.0),
            icon(Icon::ArrowRight).size(16.0).role(Role::Text3),
            preview::desktop(Look::Theme(ThemeName::HighContrast), FocusColor::Amber, a.effects, 120.0, 76.0),
        ]
        .spacing(12.0)
        .align(Align::Center);
        let mut typing = vec![
            form::row("Sticky keys", toggle(extra::get_bool(k, "sticky_keys", false)).on_toggle(Msg::Sticky))
                .entry("sticky-keys")
                .help("Press ⌃, ⌥, ⇧ or ❖ and let go: it holds for the next key, so shortcuts need one finger."),
            form::row("Slow keys", toggle(slow).on_toggle(Msg::Slow))
                .entry("slow-keys")
                .help("A key counts only when held a moment, so brushes and bounces don't type."),
        ];
        if slow {
            typing.push(
                form::row(
                    "Hold for",
                    crate::panes::mouse::speed_slider(slow_ms, 100.0..=1000.0, "Short", "Long", Msg::SlowMs),
                )
                .detail(format!("{} ms", slow_ms as i64)),
            );
        }
        form::page(vec![
            vcx.section(
                "Seeing",
                vec![
                    form::whole(container(contrast).padding([14.0, form::INSET]).width(Fill).align_x(Align::Center)),
                    form::row("High Contrast", toggle(hc).on_toggle(Msg::HighContrast)).entry("high-contrast").help(
                        "Stronger edges and text everywhere, every colour at least 7:1 against what's behind it.",
                    ),
                    form::row("Larger text", toggle(a.size == UiSize::Large).on_toggle(Msg::Larger))
                        .entry("larger-text")
                        .help("Text and controls a size up. Appearance has the sizes between."),
                    form::row(
                        "Pointer size",
                        row![
                            slider(cursor_i as f64, 0.0..=(CURSOR_SIZES.len() - 1) as f64)
                                .step(1.0)
                                .ticks(CURSOR_SIZES.len())
                                .on_change(Msg::Cursor)
                                .width(180.0),
                            icon(Icon::MousePointer)
                                .size((a.cursor_size as f64 / 2.0).clamp(12.0, 32.0))
                                .role(Role::Text),
                        ]
                        .spacing(10.0)
                        .align(Align::Center),
                    )
                    .entry("cursor-size")
                    .detail(format!("{} pt", a.cursor_size)),
                    form::row("Reduce motion", toggle(a.reduced_motion).on_toggle(Msg::ReducedMotion))
                        .entry("reduced-motion")
                        .help("Nothing slides or fades: menus, the Deskbar and windows appear where they go."),
                ],
            ),
            vcx.section(
                "Hearing",
                vec![
                    form::row(
                        "Flash the screen for alerts",
                        toggle(extra::get_bool(&s.sound, "visual_alert", false)).on_toggle(Msg::Flash),
                    )
                    .entry("flash")
                    .help("The menu bar flashes when the alert sound plays, so it's seen as well as heard."),
                ],
            ),
            vcx.section("Typing", typing),
        ])
    }
}
