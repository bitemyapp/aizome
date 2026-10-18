//! What shows under the pointer while files are dragged out of Hako: a
//! chip in the look of the Shelf's cards (an icon in its dye, the name or
//! how many items), drawn offscreen.

use aizome_kit::sidestep_ui::{Appearance, Image, kurbo};
use aizome_kit::theme::FontSpec;
use aizome_kit::{Point, Rect, Size, Theme};

use super::look;
use crate::model::entry::Entry;

/// The chip's height in points.
pub const HEIGHT: f64 = 26.0;

/// A chip for dragging `first` (and `count - 1` more), at `scale`.
pub fn chip(theme: &Theme, first: Option<&Entry>, count: usize, scale: f64) -> Image {
    let label = match (first, count) {
        (Some(e), 1) => e.display.clone(),
        (_, n) => format!("{n} items"),
    };
    let (icon, role) = match (first, count) {
        (Some(e), 1) => look::entry_icon(e),
        _ => (aizome_kit::Icon::Files, aizome_kit::Role::Accent),
    };
    let style = theme.style(FontSpec::ui(theme.metrics.small + 1.0), theme.text());
    let text_w = aizome_kit::sidestep_ui::measure(&label, &style).width.min(220.0);
    let width = (8.0 + 14.0 + 8.0 + text_w + 10.0).ceil();
    let size = Size::new(width, HEIGHT);
    let appearance = if theme.is_dark() { Appearance::DARK } else { Appearance::LIGHT };
    let fill = theme.surface();
    let edge = theme.line();
    let icon_color = theme.role(role);
    aizome_kit::sidestep_ui::offscreen::render(size, scale, appearance, |c| {
        let r = Rect::from_origin_size(Point::ZERO, size);
        c.fill_rect(r, fill.with_alpha(0.94));
        aizome_kit::draw::border(c, r, edge, 1.0);
        icon.draw(c, Point::new(8.0, ((HEIGHT - 14.0) / 2.0).round()), 14.0, icon_color);
        let layout = aizome_kit::sidestep_ui::TextLayout::builder(&label, &style)
            .width(text_w)
            .wrap(aizome_kit::sidestep_ui::Wrap::None)
            .ellipsis(true)
            .build();
        let h = layout.size().height;
        c.draw_text(&layout, kurbo::Point::new(30.0, ((HEIGHT - h) / 2.0).round()));
    })
}
