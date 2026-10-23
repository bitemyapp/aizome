//! Device totals stay visible while the compact picker filters process usage.

use aizome_kit::prelude::*;
use aizome_kit::{column, row};

use crate::app::{Msg, Myaku};
use crate::model::{self, Gpu};

pub fn overview(m: &Myaku) -> Element<Msg> {
    let multiple = m.frame.gpus.len() > 1;
    let mut heading: Vec<Element<Msg>> = Vec::new();
    if multiple {
        let mut labels = vec!["All GPUs".to_owned()];
        labels.extend(m.frame.gpus.iter().enumerate().map(|(i, g)| {
            if m.frame.gpus.iter().filter(|other| other.name == g.name).count() > 1 {
                format!("GPU {i} · {}", g.name)
            } else {
                g.name.clone()
            }
        }));
        let devices: Vec<String> = m.frame.gpus.iter().map(|g| g.pci.clone()).collect();
        let selected = devices.iter().position(|pci| Some(pci) == m.gpu.as_ref()).map_or(0, |i| i + 1);
        heading.push(text("GPU").size(12.0).role(Role::Text2).into());
        heading.push(
            dropdown(labels, Some(selected))
                .compact()
                .width(280.0)
                .on_select(move |i| Msg::Gpu(i.checked_sub(1).and_then(|i| devices.get(i).cloned())))
                .id("gpu-selector")
                .label("Filter processes by GPU")
                .tooltip("Show processes using this GPU, including idle clients holding memory")
                .into(),
        );
    }
    heading.push(spacer().into());
    heading.push(text("Device totals").mono().caps().letter_spacing(1.1).size(10.5).role(Role::Text3).into());
    let cards: Vec<Element<Msg>> = m
        .frame
        .gpus
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let card = device(g, i, m.gpu.as_deref() == Some(&g.pci) || !multiple);
            // Two cards fit even at the window's minimum width. Larger inventories
            // scroll horizontally instead of squeezing their figures or growing tall.
            if m.frame.gpus.len() > 2 { card.width(328.0) } else { card.width(Length::Fill) }
        })
        .collect();
    let cards = aizome_kit::widgets::layout::row(cards).spacing(10.0);
    let cards: Element<Msg> = if m.frame.gpus.len() > 2 {
        scroll(cards).horizontal().height(102.0).width(Length::Fill).into()
    } else {
        cards.width(Length::Fill).into()
    };
    container(
        column![aizome_kit::widgets::layout::row(heading).spacing(8.0).height(24.0).width(Length::Fill), cards,]
            .spacing(6.0)
            .width(Length::Fill),
    )
    .padding([6.0, 14.0, 12.0, 14.0])
    .fill(Tone::Chrome)
    .edges(Edge::BevelLo, Sides::BOTTOM)
    .width(Length::Fill)
    .id("gpu-overview")
    .into()
}

fn device(g: &Gpu, index: usize, selected: bool) -> Element<Msg> {
    let mut detail = vec![format!("GPU {index}"), if g.integrated { "Integrated" } else { "Discrete" }.into()];
    if let Some(c) = g.temperature {
        detail.push(format!("{c:.0} °C"));
    }
    if let Some(w) = g.power {
        detail.push(model::watts(w));
    }
    let utilization = g.busy.map(|p| format!("{}%", model::percent(p))).unwrap_or_else(|| "—".into());
    let memory = g.vram_used.map(model::bytes).unwrap_or_else(|| "—".into());
    let capacity = g.vram_total.map(|n| format!("/ {}", model::bytes(n))).unwrap_or_default();
    let meter: Element<Msg> = match g.vram_used.zip(g.vram_total).filter(|(_, total)| *total > 0) {
        Some((used, total)) => {
            progress((used as f64 / total as f64).clamp(0.0, 1.0)).height(6.0).width(Length::Fill).into()
        }
        None => space(6.0),
    };
    let content = column![
        text(g.name.clone())
            .semibold()
            .size(12.0)
            .role(if selected { Role::Accent } else { Role::Text })
            .width(Length::Fill),
        text(detail.join(" · ")).size(10.5).role(Role::Text3).tabular().width(Length::Fill),
        row![
            column![
                text("Utilization").size(10.5).role(Role::Text3),
                text(utilization).size(22.0).tabular().role(Role::Accent),
            ]
            .spacing(1.0)
            .width(92.0),
            column![
                text("Video memory").size(10.5).role(Role::Text3),
                row![
                    text(memory).size(18.0).tabular().semibold(),
                    text(capacity).size(11.5).tabular().role(Role::Text2),
                ]
                .spacing(5.0),
                meter,
            ]
            .spacing(1.0)
            .width(Length::Fill),
        ]
        .spacing(16.0)
        .width(Length::Fill),
    ]
    .spacing(3.0)
    .padding([8.0, 12.0])
    .width(Length::Fill);
    container(content)
        .fill(Tone::Sunk)
        .border(Edge::Line)
        .height(98.0)
        .key(g.pci.clone())
        .id(format!("gpu-device-{index}"))
        .tooltip(format!(
            "{}\n{} · {}\nDriver totals include allocations outside the process list.",
            g.name, g.pci, g.driver
        ))
        .into()
}
