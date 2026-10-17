//! Shitsurae's own widgets, in Aizome's look, for what the kit doesn't
//! have: pressable rows (the sidebar's panes, list rows), glyph tiles,
//! form cards, the highlight a search match gets, avatars and the mark.

use std::sync::Arc;

use aizome_kit::draw;
use aizome_kit::element::{Common, Element, Layoutable, MsgFn};
use aizome_kit::hash::PaintHasher;
use aizome_kit::layout::{Constraints, Point, Rect, Size};
use aizome_kit::theme::{Rgba, Theme, color};
use aizome_kit::widget::{
    Event, EventCx, FocusRing, Focusable, LayoutCx, PaintCx, SemanticAction, SemanticRole, Semantics, Status, Widget,
};
use aizome_kit::{Color, Icon, Image};
use aizome_kit::sidestep_ui::{Cursor, PointerButton};
use aizome_theme::{Brand, Palette};

use crate::pane::Tile;

/// The fill of a form's card: `chrome` at night, `raised` by day.
pub fn card_fill(t: &Theme) -> Color {
    if t.is_dark() { t.chrome() } else { t.raised() }
}

// Pressable rows.

/// How a [`press`] looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PressStyle {
    /// A sidebar or list row: hover shades it, selection fills it with
    /// `selection` (its text and glyphs take `on_selection`).
    #[default]
    Row,
    /// A row inside a form card: hover shades it a little, no selection.
    Plain,
    /// A tile of its own: raised, edged, hover lifts it; selection edges it
    /// in the focus colour (wallpapers, previews).
    Card,
}

/// Something pressable made of other elements.
pub struct Press<Msg> {
    common: Common,
    content: Element<Msg>,
    on_press: Option<MsgFn>,
    on_double: Option<MsgFn>,
    selected: bool,
    style: PressStyle,
    padding: [f64; 4],
    label: Option<Arc<str>>,
}

pub fn press<Msg>(content: impl Into<Element<Msg>>) -> Press<Msg> {
    Press {
        common: Common::default(),
        content: content.into(),
        on_press: None,
        on_double: None,
        selected: false,
        style: PressStyle::Row,
        padding: [0.0; 4],
        label: None,
    }
}

impl<Msg: Clone + Send + 'static> Press<Msg> {
    pub fn on_press(mut self, msg: Msg) -> Self {
        self.on_press = Some(MsgFn::new(msg));
        self
    }

    pub fn on_press_maybe(mut self, msg: Option<Msg>) -> Self {
        self.on_press = msg.map(MsgFn::new);
        self
    }

    pub fn on_double_click(mut self, msg: Msg) -> Self {
        self.on_double = Some(MsgFn::new(msg));
        self
    }
}

impl<Msg> Press<Msg> {
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }

    pub fn style(mut self, style: PressStyle) -> Self {
        self.style = style;
        self
    }

    /// `[vertical, horizontal]`.
    pub fn padding(mut self, [v, h]: [f64; 2]) -> Self {
        self.padding = [v, h, v, h];
        self
    }

    /// What assistive technology calls it.
    pub fn named(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl<Msg> Layoutable for Press<Msg> {
    fn common_mut(&mut self) -> &mut Common {
        &mut self.common
    }
}

impl<Msg> From<Press<Msg>> for Element<Msg> {
    fn from(p: Press<Msg>) -> Element<Msg> {
        let w = PressWidget {
            on_press: p.on_press,
            on_double: p.on_double,
            selected: p.selected,
            style: p.style,
            padding: p.padding,
            label: p.label,
        };
        let mut el = Element::new(w, vec![p.content]);
        *el.common_mut() = p.common;
        el
    }
}

struct PressWidget {
    on_press: Option<MsgFn>,
    on_double: Option<MsgFn>,
    selected: bool,
    style: PressStyle,
    padding: [f64; 4],
    label: Option<Arc<str>>,
}

impl Widget for PressWidget {
    fn name(&self) -> &'static str {
        "press"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let [top, right, bottom, left] = self.padding;
        let inner = bc.shrink(aizome_kit::Padding { top, right, bottom, left });
        let selected = self.selected && self.style == PressStyle::Row;
        let s = if cx.child_count() > 0 { cx.layout_child_selected(0, inner, selected) } else { Size::ZERO };
        if cx.child_count() > 0 {
            cx.place_child(0, Point::new(left, top));
        }
        bc.constrain(Size::new(s.width + left + right, s.height + top + bottom))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let r = cx.bounds();
        let live = self.on_press.is_some();
        match self.style {
            PressStyle::Row => {
                if self.selected {
                    cx.canvas.fill_rect(r, t.selection());
                } else if live && cx.is_pressed() {
                    cx.canvas.fill_rect(r, t.sunk());
                } else if live && cx.is_hovered() {
                    cx.canvas.fill_rect(r, t.hover(t.chrome()));
                }
            }
            PressStyle::Plain => {
                if live && cx.is_pressed() {
                    cx.canvas.fill_rect(r, t.sunk());
                } else if live && cx.is_hovered() {
                    cx.canvas.fill_rect(r, t.hover(card_fill(t)));
                }
            }
            PressStyle::Card => {
                let fill = if live && cx.is_pressed() {
                    t.sunk()
                } else if live && cx.is_hovered() {
                    t.hover(t.raised())
                } else {
                    t.raised()
                };
                draw::raised(cx.canvas, t, r, Some(fill), t.line2());
            }
        }
    }

    fn paint_over(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        if self.style == PressStyle::Card && self.selected {
            draw::border(cx.canvas, cx.bounds().inflate(1.0, 1.0), t.focus(), 2.0);
        }
        if cx.focus_visible() {
            draw::border(cx.canvas, cx.bounds(), t.focus(), 2.0);
        }
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.add(&(self.selected, self.style, self.on_press.is_some()));
    }

    fn hit(&self) -> bool {
        self.on_press.is_some()
    }

    fn solid(&self) -> bool {
        true
    }

    fn overflow(&self) -> f64 {
        if self.style == PressStyle::Card { 2.0 } else { 0.0 }
    }

    fn focusable(&self) -> Focusable {
        if self.on_press.is_some() { Focusable::Tab } else { Focusable::No }
    }

    fn focus_ring(&self) -> FocusRing {
        FocusRing::Never
    }

    fn cursor(&self) -> Option<Cursor> {
        None
    }

    fn event(&mut self, cx: &mut EventCx, event: &Event) -> Status {
        let Some(msg) = &self.on_press else { return Status::Ignored };
        match event {
            Event::PointerDown { button: PointerButton::Left, clicks, .. } => {
                cx.invalidate();
                if *clicks >= 2
                    && let Some(d) = &self.on_double
                {
                    let d = d.clone();
                    cx.emit(&d);
                }
                Status::Handled
            }
            Event::PointerUp { pos, button: PointerButton::Left, .. } => {
                cx.invalidate();
                if cx.bounds().contains(*pos) {
                    let m = msg.clone();
                    cx.emit(&m);
                }
                Status::Handled
            }
            _ => Status::Ignored,
        }
    }

    fn activate(&mut self, cx: &mut EventCx) -> bool {
        match &self.on_press {
            Some(m) => {
                let m = m.clone();
                cx.emit(&m);
                true
            }
            None => false,
        }
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics {
            role: if self.on_press.is_some() { SemanticRole::Button } else { SemanticRole::Group },
            label: self.label.as_deref().map(str::to_owned),
            selected: self.selected.then_some(true),
            actions: if self.on_press.is_some() { vec![SemanticAction::Click, SemanticAction::Focus] } else { Vec::new() },
            ..Default::default()
        })
    }
}

// Glyph tiles.

/// A Lucide glyph on a small square of a dye: the sidebar's pane icons.
pub fn tile<Msg>(icon: Icon, tile: Tile, size: f64) -> Element<Msg> {
    Element::new(TileWidget { icon, tile, size, selected: false }, Vec::new())
}

struct TileWidget {
    icon: Icon,
    tile: Tile,
    size: f64,
    selected: bool,
}

impl Widget for TileWidget {
    fn name(&self) -> &'static str {
        "tile"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        self.selected = cx.on_selection();
        bc.constrain(Size::new(self.size, self.size))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let (fill, glyph) = self.tile.colors(&t.palette);
        let r = Rect::new(0.0, 0.0, self.size, self.size);
        cx.canvas.fill_rect(r, color(fill));
        // A 1pt edge keeps a tile apart from the selection behind it.
        let edge = if self.selected { t.on_selection().with_alpha(0.45) } else { color(fill.mix(Rgba::hex(0x000000), 0.18)) };
        draw::border(cx.canvas, r, edge, 1.0);
        draw::top(cx.canvas, r.inset(-1.0), Color::rgba8(255, 255, 255, 40), 1.0);
        let g = (self.size * 0.68).round();
        let at = Point::new(((self.size - g) / 2.0).round(), ((self.size - g) / 2.0).round());
        self.icon.draw(cx.canvas, at, g, color(glyph));
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.add(&(self.icon, self.tile, self.selected)).f64(self.size);
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::Image, label: Some(self.icon.name().to_owned()), ..Default::default() })
    }
}

// Form cards.

/// A form section's card: `card_fill`, a 1pt `line` edge and the top
/// highlight. Its children stack down it.
pub fn card<Msg>(child: impl Into<Element<Msg>>) -> Element<Msg> {
    Element::new(CardWidget, vec![child.into()])
}

struct CardWidget;

impl Widget for CardWidget {
    fn name(&self) -> &'static str {
        "card"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let pad = aizome_kit::Padding { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 };
        let s = cx.layout_child(0, bc.shrink(pad));
        cx.place_child(0, Point::new(1.0, 1.0));
        bc.constrain(Size::new(s.width + 2.0, s.height + 2.0))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let r = cx.bounds();
        cx.canvas.fill_rect(r, card_fill(t));
        draw::border(cx.canvas, r, t.line(), 1.0);
        draw::top(cx.canvas, r.inset(-1.0), t.bevel_hi(), 1.0);
    }

    fn solid(&self) -> bool {
        true
    }
}

// The highlight a search match gets.

/// A form row: highlighted (a tint of the selection and a 2pt focus rail
/// at its left) when search matches it, and the place the window reveals
/// when search opens it.
pub fn highlight<Msg>(child: impl Into<Element<Msg>>, on: bool) -> Element<Msg> {
    Element::new(HighlightWidget { on }, vec![child.into()])
}

struct HighlightWidget {
    on: bool,
}

impl Widget for HighlightWidget {
    fn name(&self) -> &'static str {
        "highlight"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let s = cx.layout_child(0, bc);
        cx.place_child(0, Point::ZERO);
        bc.constrain(s)
    }

    fn paint(&self, cx: &mut PaintCx) {
        if self.on {
            let t = cx.theme();
            let r = cx.bounds();
            cx.canvas.fill_rect(r, t.selection().with_alpha(if t.is_dark() { 0.22 } else { 0.12 }));
            draw::left(cx.canvas, r, t.focus(), 2.0);
        }
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.add(&self.on);
    }

    // Revealed by the window through the focus, without joining Tab's
    // chain or showing a ring.
    fn focusable(&self) -> Focusable {
        Focusable::Pointer
    }

    fn focus_ring(&self) -> FocusRing {
        FocusRing::Never
    }
}

// Avatars.

/// A person's picture, or their initials on a dye chosen by their name:
/// square, as everything in Aizome is.
pub fn avatar<Msg>(picture: Option<Image>, initials: &str, seed: &str, size: f64) -> Element<Msg> {
    let dye = dye_for(seed);
    Element::new(AvatarWidget { picture, initials: initials.to_owned(), dye, size }, Vec::new())
}

/// The same dye every time for a name.
pub fn dye_for(seed: &str) -> usize {
    let mut h: u32 = 2166136261;
    for b in seed.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16777619);
    }
    (h % 5) as usize
}

const AVATAR_DYES: [u32; 5] = [0x3d59a1, 0x7aa2f7, 0x24283b, 0x9a6a12, 0x4f7a28];

struct AvatarWidget {
    picture: Option<Image>,
    initials: String,
    dye: usize,
    size: f64,
}

impl Widget for AvatarWidget {
    fn name(&self) -> &'static str {
        "avatar"
    }

    fn layout(&mut self, _cx: &mut LayoutCx, bc: Constraints) -> Size {
        bc.constrain(Size::new(self.size, self.size))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let r = Rect::new(0.0, 0.0, self.size, self.size);
        match &self.picture {
            Some(img) => {
                // Cover the square, cropping the longer side.
                let s = img.size();
                let k = (self.size / s.width).max(self.size / s.height);
                let (w, h) = (s.width * k, s.height * k);
                let dst = Rect::new((self.size - w) / 2.0, (self.size - h) / 2.0, (self.size + w) / 2.0, (self.size + h) / 2.0);
                cx.canvas.save();
                cx.canvas.clip_rect(r);
                cx.canvas.draw_image(img, dst);
                cx.canvas.restore();
            }
            None => {
                let fill = Rgba::hex(AVATAR_DYES[self.dye % AVATAR_DYES.len()]);
                cx.canvas.fill_rect(r, color(fill));
                let spec = aizome_kit::text::TextSpec::new(
                    aizome_kit::FontSpec { weight: 650.0, ..aizome_kit::FontSpec::ui((self.size * 0.42).round()) },
                    Color::rgba8(0xee, 0xf1, 0xff, 0xff),
                );
                cx.draw_text_in(&self.initials, &spec, r, aizome_kit::Align::Center);
            }
        }
        draw::border(cx.canvas, r, t.bevel_lo().with_alpha(0.6), 1.0);
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.str(&self.initials).add(&(self.dye, self.picture.as_ref().map(|i| i.pixel_size()))).f64(self.size);
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::Image, label: Some(self.initials.clone()), ..Default::default() })
    }
}

// The mark.

/// Aizome's mark, `module` points to a module (the mark is 4 × 4
/// modules): the amber tab over three dye stripes.
pub fn mark<Msg>(module: f64) -> Element<Msg> {
    Element::new(MarkWidget { module }, Vec::new())
}

struct MarkWidget {
    module: f64,
}

impl Widget for MarkWidget {
    fn name(&self) -> &'static str {
        "mark"
    }

    fn layout(&mut self, _cx: &mut LayoutCx, bc: Constraints) -> Size {
        bc.constrain(Size::new(self.module * 4.0, self.module * 4.0))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let brand = Brand::for_theme(cx.theme().palette.name);
        draw_mark(cx.canvas, &brand, Point::ZERO, self.module);
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.f64(self.module);
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::Image, label: Some("Aizome".into()), ..Default::default() })
    }
}

/// Draw the mark with its top left at `at`, `module` points a module.
pub fn draw_mark(canvas: &mut aizome_kit::sidestep_ui::Canvas, brand: &Brand, at: Point, module: f64) {
    for ([x, y, w, h], c) in brand.mark_rects() {
        let r = Rect::new(
            at.x + f64::from(x) * module,
            at.y + f64::from(y) * module,
            at.x + f64::from(x + w) * module,
            at.y + f64::from(y + h) * module,
        );
        canvas.fill_rect(r, color(c));
    }
}

/// The palette `p`'s colour as Sidestep's.
pub fn c(p: Rgba) -> Color {
    color(p)
}

/// Night, Day or High Contrast with the focus colour `focus`.
pub fn palette(name: aizome_theme::ThemeName, focus: aizome_theme::FocusColor) -> Palette {
    Palette::new(name, focus)
}
