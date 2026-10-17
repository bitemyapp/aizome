//! Aizome's drawing vocabulary: square edges on whole points, 1pt bevels
//! (`bevel_hi` along the top inside, `bevel_lo` around the outside), the
//! hard 3pt shadow under floating things, and the 2pt focus ring.
//!
//! Edges are filled rectangles rather than strokes, so they land on whole
//! device pixels at every integer scale with no smoothing.

use sidestep_ui::{Canvas, Color};

use crate::layout::Rect;
use crate::theme::Theme;

/// A 1pt (or `w`) border inside `r`.
pub fn border(canvas: &mut Canvas, r: Rect, color: Color, w: f64) {
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    canvas.fill_rect(Rect::new(r.x0, r.y0, r.x1, r.y0 + w), color);
    canvas.fill_rect(Rect::new(r.x0, r.y1 - w, r.x1, r.y1), color);
    canvas.fill_rect(Rect::new(r.x0, r.y0 + w, r.x0 + w, r.y1 - w), color);
    canvas.fill_rect(Rect::new(r.x1 - w, r.y0 + w, r.x1, r.y1 - w), color);
}

/// A line along the top of `r`, `w` thick.
pub fn top(canvas: &mut Canvas, r: Rect, color: Color, w: f64) {
    canvas.fill_rect(Rect::new(r.x0, r.y0, r.x1, r.y0 + w), color);
}

pub fn bottom(canvas: &mut Canvas, r: Rect, color: Color, w: f64) {
    canvas.fill_rect(Rect::new(r.x0, r.y1 - w, r.x1, r.y1), color);
}

pub fn left(canvas: &mut Canvas, r: Rect, color: Color, w: f64) {
    canvas.fill_rect(Rect::new(r.x0, r.y0, r.x0 + w, r.y1), color);
}

pub fn right(canvas: &mut Canvas, r: Rect, color: Color, w: f64) {
    canvas.fill_rect(Rect::new(r.x1 - w, r.y0, r.x1, r.y1), color);
}

/// A raised face: `fill`, a 1pt `edge` border, and the top highlight just
/// inside it. The board's toolbar buttons and pop-ups.
pub fn raised(canvas: &mut Canvas, theme: &Theme, r: Rect, fill: Option<Color>, edge: Color) {
    if let Some(f) = fill {
        canvas.fill_rect(r, f);
    }
    border(canvas, r, edge, 1.0);
    let inner = r.inset(-1.0);
    top(canvas, inner, theme.bevel_hi(), 1.0);
}

/// A sunken well: `fill`, a 1pt `edge` border, and a shade along the top
/// inside, as fields and tracks are.
pub fn sunken(canvas: &mut Canvas, theme: &Theme, r: Rect, fill: Color, edge: Color) {
    canvas.fill_rect(r, fill);
    border(canvas, r, edge, 1.0);
    let inner = r.inset(-1.0);
    let shade = if theme.is_dark() { theme.bevel_lo().with_alpha(0.55) } else { theme.bevel_lo().with_alpha(0.18) };
    top(canvas, inner, shade, 1.0);
}

/// A floating panel's look (menus, popovers, sheets): `raised`, a 1pt
/// `bevel_lo` edge, the top highlight, and the hard shadow 3pt right and
/// down when effects allow.
pub fn floating(canvas: &mut Canvas, theme: &Theme, r: Rect) {
    if theme.shadows() {
        let s = theme.metrics.shadow;
        canvas.fill_rect(Rect::new(r.x0 + s, r.y1, r.x1 + s, r.y1 + s), theme.slab());
        canvas.fill_rect(Rect::new(r.x1, r.y0 + s, r.x1 + s, r.y1), theme.slab());
    }
    canvas.fill_rect(r, theme.raised());
    border(canvas, r, theme.bevel_lo(), 1.0);
    top(canvas, r.inset(-1.0), theme.bevel_hi(), 1.0);
}

/// The 2pt focus ring outside `r`.
pub fn focus_ring(canvas: &mut Canvas, r: Rect, color: Color) {
    border(canvas, r.inflate(2.0, 2.0), color, 2.0);
}

/// A separator: a `line` rule with the highlight under it.
pub fn separator(canvas: &mut Canvas, theme: &Theme, x0: f64, x1: f64, y: f64) {
    canvas.fill_rect(Rect::new(x0, y, x1, y + 1.0), theme.line());
    canvas.fill_rect(Rect::new(x0, y + 1.0, x1, y + 2.0), theme.bevel_hi());
}

/// A rectangle with `r`'s centre, `w` by `h`, on whole points.
pub fn centered(r: Rect, w: f64, h: f64) -> Rect {
    let x = (r.x0 + (r.width() - w) / 2.0).round();
    let y = (r.y0 + (r.height() - h) / 2.0).round();
    Rect::new(x, y, x + w, y + h)
}
