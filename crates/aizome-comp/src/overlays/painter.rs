//! Drawing overlays in Aizome's look: flat rectangles, 1pt bevels, hard
//! 3pt shadows, labels set by Sidestep, app icons and live thumbnails, in
//! painter's order. Pieces keep their identities between frames by their
//! place in the drawing order, so an unchanged overlay causes no damage.

use aizome_theme::{Palette, Rgba};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};
use smithay::utils::{Logical, Physical, Point};

use crate::decor::{DecorCache, Piece};
use crate::render::OutputElement;
use crate::state::Aizome;
use crate::text::{Face, Style};
use crate::wm::{Rect, WinId};

enum Item<R: Renderer + ImportAll + ImportMem> {
    Piece(Piece),
    /// Front to back, as smithay gives them.
    Elems(Vec<OutputElement<R>>),
}

pub struct Painter<R: Renderer + ImportAll + ImportMem> {
    items: Vec<Item<R>>,
    pub origin: (f64, f64),
    pub scale: f64,
    /// Everything fades by this (overlays coming in).
    pub alpha: f32,
}

/// A rectangle in points with fractions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Box2 {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Box2 {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Box2 {
        Box2 { x, y, w, h }
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn inset(&self, d: f64) -> Box2 {
        Box2::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn rect(&self) -> Rect {
        Rect::new(self.x.round() as i32, self.y.round() as i32, self.w.round() as i32, self.h.round() as i32)
    }
}

impl<R> Painter<R>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    pub fn new(origin: (f64, f64), scale: f64, alpha: f32) -> Painter<R> {
        Painter { items: Vec::new(), origin, scale, alpha }
    }

    pub fn fill(&mut self, b: Box2, color: Rgba) {
        if b.w > 0.0 && b.h > 0.0 && color.a > 0 {
            self.items.push(Item::Piece(Piece::new(b.x, b.y, b.w, b.h, color)));
        }
    }

    /// A 1pt outline.
    pub fn outline(&mut self, b: Box2, t: f64, color: Rgba) {
        self.fill(Box2::new(b.x, b.y, b.w, t), color);
        self.fill(Box2::new(b.x, b.bottom() - t, b.w, t), color);
        self.fill(Box2::new(b.x, b.y + t, t, b.h - 2.0 * t), color);
        self.fill(Box2::new(b.right() - t, b.y + t, t, b.h - 2.0 * t), color);
    }

    /// A dashed outline.
    pub fn dashed(&mut self, b: Box2, t: f64, color: Rgba) {
        let mut v = Vec::new();
        super::dashed(&mut v, b.rect(), t, 6.0, 4.0, color);
        for p in v {
            self.items.push(Item::Piece(p));
        }
    }

    /// A panel in Aizome's look: the hard 3pt shadow, a 1pt `bevel_lo`
    /// edge, the fill and a 1pt `bevel_hi` top highlight.
    pub fn panel(&mut self, b: Box2, fill: Rgba, p: &Palette, shadow: bool) {
        if shadow {
            self.fill(Box2::new(b.right(), b.y + 3.0, 3.0, b.h), p.slab);
            self.fill(Box2::new(b.x + 3.0, b.bottom(), b.w - 3.0, 3.0), p.slab);
        }
        self.fill(b, p.bevel_lo);
        self.fill(b.inset(1.0), fill);
        self.fill(Box2::new(b.x + 1.0, b.y + 1.0, b.w - 2.0, 1.0), p.bevel_hi);
    }

    /// Text with its top left at (x, y), shortened to `max_w`; returns its
    /// width.
    pub fn text(&mut self, az: &mut Aizome, renderer: &mut R, text: &str, style: Style, x: f64, y: f64, max_w: f64) -> f64 {
        let fitted = az.labels.fit(text, style.face, style.size(), max_w);
        let Some(label) = az.labels.get(&fitted, style, self.scale, max_w + 1.0) else { return 0.0 };
        let loc: Point<i32, Physical> = Point::<f64, Logical>::from((x - self.origin.0, y - self.origin.1)).to_physical_precise_round(self.scale);
        if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64(),
            &label.buffer,
            Some(self.alpha),
            None,
            Some((label.w, label.h).into()),
            Kind::Unspecified,
        ) {
            self.items.push(Item::Elems(vec![OutputElement::Memory(e)]));
        }
        label.w as f64
    }

    /// Text centred vertically in a box of height `h` from `y`.
    #[allow(clippy::too_many_arguments)]
    pub fn text_mid(&mut self, az: &mut Aizome, renderer: &mut R, text: &str, style: Style, x: f64, y: f64, h: f64, max_w: f64) -> f64 {
        let (_, th) = az.labels.measure(text, style.face, style.size());
        self.text(az, renderer, text, style, x, y + ((h - th.ceil()) / 2.0).round(), max_w)
    }

    /// Text centred in a box.
    #[allow(clippy::too_many_arguments)]
    pub fn text_center(&mut self, az: &mut Aizome, renderer: &mut R, text: &str, style: Style, b: Box2) -> f64 {
        let (tw, th) = az.labels.measure(text, style.face, style.size());
        let w = tw.min(b.w);
        self.text(az, renderer, text, style, (b.x + (b.w - w) / 2.0).round(), (b.y + (b.h - th.ceil()) / 2.0).round(), b.w)
    }

    pub fn icon(&mut self, az: &mut Aizome, renderer: &mut R, app_id: &str, tint: crate::icons::Tint, x: f64, y: f64, size: i32, alpha: f32) {
        let scale = self.scale;
        let origin = self.origin;
        if let Some(tex) = az.icons.texture(app_id, size, scale.ceil().max(1.0) as u32, tint) {
            if let Some(e) = crate::icons::icon_element(renderer, tex, (x, y), origin, scale, alpha * self.alpha) {
                self.items.push(Item::Elems(vec![e]));
            }
        }
    }

    /// A window's live content scaled into a box.
    pub fn thumbnail(&mut self, az: &Aizome, renderer: &mut R, id: WinId, b: Box2, alpha: f32) {
        let e = crate::render::thumbnail_elements(az, renderer, id, (b.x, b.y, b.w, b.h), self.origin, self.scale, alpha * self.alpha);
        if !e.is_empty() {
            self.items.push(Item::Elems(e));
        }
    }

    /// The elements, front first, with stable identities from `cache`.
    pub fn finish(self, cache: &mut DecorCache) -> Vec<OutputElement<R>> {
        let alpha = self.alpha;
        let mut out = Vec::new();
        let mut slot = 0;
        for item in self.items {
            match item {
                Item::Piece(p) => {
                    if let Some(e) = cache.element(slot, &p, self.origin, self.scale, alpha) {
                        out.push(vec![OutputElement::Solid(e)]);
                    }
                    slot += 1;
                }
                Item::Elems(e) => out.push(e),
            }
        }
        out.into_iter().rev().flatten().collect()
    }
}

/// Text styles the overlays use.
pub fn ui(size: f64, color: Rgba) -> Style {
    Style::new(Face::Ui, size, color)
}
pub fn semibold(size: f64, color: Rgba) -> Style {
    Style::new(Face::UiSemibold, size, color)
}
pub fn bold(size: f64, color: Rgba) -> Style {
    Style::new(Face::UiBold, size, color)
}
pub fn mono(size: f64, color: Rgba) -> Style {
    Style::new(Face::Mono, size, color)
}
pub fn mono_bold(size: f64, color: Rgba) -> Style {
    Style::new(Face::MonoBold, size, color)
}
