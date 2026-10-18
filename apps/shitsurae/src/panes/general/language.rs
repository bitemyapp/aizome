//! Language & Region: the system's language (`LANG`) and the region whose
//! formats dates, times and numbers take (`LC_TIME` and friends), through
//! systemd-localed, with a preview of the formats. A locale the system
//! configuration pins (`i18n.defaultLocale`) says so.

use std::sync::Arc;

use aizome_kit::prelude::*;
use aizome_kit::{column, row};
use aizome_sys::datetime::{Locale, LocaleState};

use super::Msg as G;
use crate::form;
use crate::pane::{PaneCx, SettingEntry, ViewCx};
use crate::system::Cell;

pub enum Svc {
    Live(Locale),
    Sample(Cell<LocaleState>),
}

impl Svc {
    fn snapshot(&self) -> Arc<LocaleState> {
        match self {
            Svc::Live(l) => l.snapshot(),
            Svc::Sample(c) => c.get(),
        }
    }
}

/// The region entries a "formats" choice sets.
const REGION_KEYS: [&str; 6] = ["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER", "LC_ADDRESS"];

/// How a locale writes things.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Formats {
    pub date: String,
    pub long_date: String,
    pub time: String,
    pub number: String,
    pub currency: String,
    pub measurement: String,
    pub first_day: String,
}

#[derive(Default)]
pub struct State {
    svc: Option<Svc>,
    /// The locales installed (`locale -a`), as `en_GB.UTF-8`.
    available: Option<Vec<String>>,
    preview: Option<(String, Formats)>,
}

#[derive(Clone, Debug)]
pub enum Msg {
    Available(Vec<String>),
    Preview(String, Formats),
    Language(usize),
    Region(usize),
    Done(&'static str, Result<(), String>),
}

pub fn entries() -> Vec<SettingEntry> {
    vec![
        SettingEntry::new("system-language", "Language").under("Language & Region").keywords(&[
            "locale",
            "translation",
            "lang",
        ]),
        SettingEntry::new("region", "Region formats").under("Language & Region").keywords(&[
            "date format",
            "number format",
            "currency",
            "units",
            "metric",
        ]),
    ]
}

/// Names of languages people know them by.
fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "ja" => "Japanese",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "nb" | "no" => "Norwegian",
        "da" => "Danish",
        "fi" => "Finnish",
        "pl" => "Polish",
        "cs" => "Czech",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "tr" => "Turkish",
        "el" => "Greek",
        "he" => "Hebrew",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "zh" => "Chinese",
        "ko" => "Korean",
        "vi" => "Vietnamese",
        "th" => "Thai",
        "id" => "Indonesian",
        "ga" => "Irish",
        "cy" => "Welsh",
        "hu" => "Hungarian",
        "ro" => "Romanian",
        "C" | "POSIX" => "Computer (C)",
        other => other,
    }
}

fn territory_name(code: &str) -> &str {
    match code {
        "US" => "United States",
        "GB" => "United Kingdom",
        "IE" => "Ireland",
        "AU" => "Australia",
        "NZ" => "New Zealand",
        "CA" => "Canada",
        "IN" => "India",
        "JP" => "Japan",
        "DE" => "Germany",
        "AT" => "Austria",
        "CH" => "Switzerland",
        "FR" => "France",
        "BE" => "Belgium",
        "ES" => "Spain",
        "MX" => "Mexico",
        "IT" => "Italy",
        "PT" => "Portugal",
        "BR" => "Brazil",
        "NL" => "Netherlands",
        "SE" => "Sweden",
        "NO" => "Norway",
        "DK" => "Denmark",
        "FI" => "Finland",
        "PL" => "Poland",
        "CZ" => "Czechia",
        "RU" => "Russia",
        "UA" => "Ukraine",
        "TR" => "Türkiye",
        "GR" => "Greece",
        "IL" => "Israel",
        "CN" => "China",
        "TW" => "Taiwan",
        "HK" => "Hong Kong",
        "KR" => "South Korea",
        "SG" => "Singapore",
        "ZA" => "South Africa",
        other => other,
    }
}

/// "English (United Kingdom)" for `en_GB.UTF-8`.
pub fn describe(locale: &str) -> String {
    let base = locale.split(['.', '@']).next().unwrap_or(locale);
    match base.split_once('_') {
        Some((l, t)) => format!("{} ({})", language_name(l), territory_name(t)),
        None => language_name(base).to_owned(),
    }
}

/// `en_GB.utf8` → `en_GB.UTF-8`.
pub fn normalize(locale: &str) -> String {
    match locale.split_once('.') {
        Some((base, enc)) if enc.eq_ignore_ascii_case("utf8") || enc.eq_ignore_ascii_case("utf-8") => {
            format!("{base}.UTF-8")
        }
        _ => locale.to_owned(),
    }
}

/// The installed UTF-8 locales, from `locale -a`.
pub fn installed() -> Vec<String> {
    let out = std::process::Command::new("locale").arg("-a").output();
    let mut v: Vec<String> = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| l.to_ascii_lowercase().contains("utf"))
            .map(normalize)
            .collect(),
        Err(_) => Vec::new(),
    };
    v.sort_by_key(|l| describe(l));
    v.dedup();
    v
}

/// What `locale` says a locale writes, through `date`, `printf` and
/// `locale` with `LC_ALL` set (read-only).
pub fn formats_of(locale: &str) -> Formats {
    let run = |prog: &str, args: &[&str]| -> String {
        std::process::Command::new(prog)
            .args(args)
            .env("LC_ALL", locale)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default()
    };
    let when = "@1792273320"; // 2026-10-17 21:42 UTC
    let d = run("date", &["-u", "-d", when, "+%x|%A %-d %B %Y|%X"]);
    let mut parts = d.split('|');
    let date = parts.next().unwrap_or("").to_owned();
    let long_date = parts.next().unwrap_or("").to_owned();
    let time = parts.next().unwrap_or("").to_owned();
    let number = run("printf", &["%'.2f", "1234567.89"]);
    let symbol = run("locale", &["currency_symbol"]);
    let measurement = match run("locale", &["measurement"]).as_str() {
        "2" => "US customary",
        _ => "Metric",
    }
    .to_owned();
    let first = run("locale", &["first_weekday"]).parse::<usize>().unwrap_or(1);
    let first_day =
        ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"][(first + 6) % 7].to_owned();
    let money = run("printf", &["%'.2f", "1234.56"]);
    Formats { date, long_date, time, number, currency: format!("{symbol}{money}"), measurement, first_day }
}

/// The sample's formats, for the few locales it has.
pub fn sample_formats(locale: &str) -> Formats {
    let f = |a: &str, b: &str, c: &str, d: &str, e: &str, m: &str, w: &str| Formats {
        date: a.into(),
        long_date: b.into(),
        time: c.into(),
        number: d.into(),
        currency: e.into(),
        measurement: m.into(),
        first_day: w.into(),
    };
    match locale.split('.').next().unwrap_or("") {
        "en_US" => f(
            "10/17/2026",
            "Saturday 17 October 2026",
            "09:42:00 PM",
            "1,234,567.89",
            "$1,234.56",
            "US customary",
            "Sunday",
        ),
        "ja_JP" => {
            f("2026年10月17日", "土曜日 17 10月 2026", "21時42分00秒", "1,234,567.89", "￥1,234.56", "Metric", "Sunday")
        }
        "de_DE" => {
            f("17.10.2026", "Samstag 17 Oktober 2026", "21:42:00", "1.234.567,89", "€1.234,56", "Metric", "Monday")
        }
        _ => f("17/10/26", "Saturday 17 October 2026", "21:42:00", "1,234,567.89", "£1,234.56", "Metric", "Monday"),
    }
}

fn sample_state() -> LocaleState {
    LocaleState {
        available: true,
        locale: vec!["LANG=en_GB.UTF-8".into()],
        locale_managed: false,
        ..LocaleState::default()
    }
}

impl State {
    fn snapshot(&self) -> Arc<LocaleState> {
        self.svc.as_ref().map(Svc::snapshot).unwrap_or_default()
    }

    fn region(&self) -> Option<String> {
        let s = self.snapshot();
        s.locale.iter().find_map(|e| e.strip_prefix("LC_TIME=").map(str::to_owned))
    }

    fn shown_locale(&self) -> String {
        self.region().or_else(|| self.snapshot().lang().map(str::to_owned)).unwrap_or_else(|| "C.UTF-8".into())
    }

    pub fn shown(&mut self, cx: &mut PaneCx<G>) {
        if self.svc.is_none() {
            self.svc = Some(if cx.sample() {
                Svc::Sample(Cell::new(sample_state(), cx.wake()))
            } else {
                Svc::Live(Locale::new(cx.wake()))
            });
        }
        if self.available.is_none() {
            if cx.sample() {
                self.available =
                    Some(vec!["de_DE.UTF-8".into(), "en_GB.UTF-8".into(), "en_US.UTF-8".into(), "ja_JP.UTF-8".into()]);
            } else {
                cx.spawn(|| G::Language(Msg::Available(installed())));
            }
        }
        self.refresh_preview(cx);
    }

    fn refresh_preview(&mut self, cx: &mut PaneCx<G>) {
        let loc = self.shown_locale();
        if self.preview.as_ref().is_some_and(|(l, _)| *l == loc) {
            return;
        }
        if cx.sample() {
            self.preview = Some((loc.clone(), sample_formats(&loc)));
        } else {
            cx.spawn(move || G::Language(Msg::Preview(loc.clone(), formats_of(&loc))));
        }
    }

    fn set(&mut self, entries: Vec<String>, cx: &mut PaneCx<G>) {
        let task = match &self.svc {
            Some(Svc::Live(l)) => l.set_locale(&entries),
            Some(Svc::Sample(c)) => c.update(move |s| s.locale = entries),
            None => return,
        };
        cx.wait(task, |r| {
            G::Language(Msg::Done("Language and formats apply when you next log in", r.map_err(|e| e.to_string())))
        });
    }

    pub fn update(&mut self, msg: Msg, cx: &mut PaneCx<G>) {
        match msg {
            Msg::Available(v) => self.available = Some(v),
            Msg::Preview(l, f) => self.preview = Some((l, f)),
            Msg::Language(i) => {
                let Some(l) = self.available.as_ref().and_then(|v| v.get(i)).cloned() else { return };
                let mut entries: Vec<String> =
                    self.snapshot().locale.iter().filter(|e| !e.starts_with("LANG=")).cloned().collect();
                entries.insert(0, format!("LANG={l}"));
                self.set(entries, cx);
            }
            Msg::Region(i) => {
                let lang = self.snapshot().lang().unwrap_or("C.UTF-8").to_owned();
                let mut entries = vec![format!("LANG={lang}")];
                // 0 is "the language's own".
                if i > 0
                    && let Some(r) = self.available.as_ref().and_then(|v| v.get(i - 1)).cloned()
                    && r != lang
                {
                    entries.extend(REGION_KEYS.iter().map(|k| format!("{k}={r}")));
                }
                self.set(entries, cx);
                self.preview = None;
            }
            Msg::Done(what, Ok(())) => {
                cx.say(what);
                self.refresh_preview(cx);
            }
            Msg::Done(_, Err(e)) => cx.failed("Language & Region", e),
        }
    }

    pub fn view(&self, vcx: &ViewCx) -> Element<Msg> {
        let s = self.snapshot();
        let lang = s.lang().unwrap_or("C.UTF-8").to_owned();
        let region = self.region();
        let available = self.available.clone().unwrap_or_default();
        let mut rows = Vec::new();
        if s.locale_managed {
            rows.push(
                form::managed("Language", describe(&lang), "Change i18n.defaultLocale in the system configuration.")
                    .entry("system-language"),
            );
            rows.push(
                form::managed(
                    "Region formats",
                    region.as_deref().map(describe).unwrap_or_else(|| "The language's own".into()),
                    "Change i18n.extraLocaleSettings.",
                )
                .entry("region"),
            );
        } else {
            let names: Vec<String> = available.iter().map(|l| describe(l)).collect();
            let lang_i = available.iter().position(|l| *l == lang);
            rows.push(
                form::row(
                    "Language",
                    dropdown(names.clone(), lang_i)
                        .placeholder(describe(&lang))
                        .on_select(Msg::Language)
                        .min_width(220.0),
                )
                .entry("system-language")
                .help("Menus, messages and apps speak it, where they've been translated. Applies at the next login."),
            );
            let mut region_names = vec![format!("As {}", describe(&lang))];
            region_names.extend(names);
            let region_i = match &region {
                None => Some(0),
                Some(r) => available.iter().position(|l| l == r).map(|i| i + 1),
            };
            rows.push(
                form::row("Region formats", dropdown(region_names, region_i).on_select(Msg::Region).min_width(220.0))
                    .entry("region")
                    .help("How dates, times, numbers, money and measurements are written."),
            );
        }
        let preview: Element<Msg> = match &self.preview {
            None => progress_indeterminate().width(160.0).into(),
            Some((_, f)) => {
                let line = |k: &str, v: &str| -> Element<Msg> {
                    row![
                        text(k.to_owned()).size(12.0).role(Role::Text3).width(120.0),
                        text(v.to_owned()).mono().size(12.0).selectable()
                    ]
                    .spacing(8.0)
                    .into()
                };
                column![
                    line("Date", &f.date),
                    line("Long date", &f.long_date),
                    line("Time", &f.time),
                    line("Number", &f.number),
                    line("Money", &f.currency),
                    line("Measurement", &f.measurement),
                    line("Week starts", &f.first_day),
                ]
                .spacing(6.0)
                .into()
            }
        };
        form::page(vec![
            vcx.section("Language and Region", rows),
            vcx.section("Formats", vec![form::whole(container(preview).padding([14.0, form::INSET]).width(Fill))]),
        ])
    }
}
