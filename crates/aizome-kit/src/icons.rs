//! Icons: a curated set of Lucide's (ISC licence, `icons/lucide/LICENSE`),
//! kept as their SVG elements and turned into kurbo paths the first time
//! each draws. They're stroked, as Lucide draws them, at Aizome's weight:
//! 1.5pt at 16pt, in proportion at other sizes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use sidestep_ui::kurbo::{Affine, BezPath, Circle, Ellipse, Line, Rect, RoundedRect, Shape};
use sidestep_ui::{Canvas, Color, LineCap, LineJoin, StrokeStyle};

use crate::element::{Common, Element, RawElement};
use crate::hash::PaintHasher;
use crate::layout::{Constraints, Point, Size};
use crate::theme::Role;
use crate::widget::{LayoutCx, PaintCx, SemanticRole, Semantics, Widget};

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

impl Icon {
    /// The icon Lucide calls `name`.
    pub fn from_name(name: &str) -> Option<Icon> {
        Icon::ALL.iter().copied().find(|i| i.name() == name)
    }

    /// Its outline on the 24-unit grid, as one path.
    pub fn path(self) -> Rc<BezPath> {
        thread_local! {
            static PATHS: RefCell<HashMap<Icon, Rc<BezPath>>> = RefCell::new(HashMap::new());
        }
        PATHS.with(|p| {
            p.borrow_mut()
                .entry(self)
                .or_insert_with(|| {
                    let mut path = BezPath::new();
                    for el in self.elements() {
                        if let Some(p) = parse_element(el) {
                            path.extend(p);
                        }
                    }
                    Rc::new(path)
                })
                .clone()
        })
    }

    /// Draw it `size` points square with its top left at `at`.
    pub fn draw(self, canvas: &mut Canvas, at: Point, size: f64, color: Color) {
        let path = self.path();
        let k = size / 24.0;
        let xf = Affine::translate(at.to_vec2()) * Affine::scale(k);
        let mut shape = (*path).clone();
        shape.apply_affine(xf);
        let width = stroke_width(size);
        let style = StrokeStyle::new(width).with_cap(LineCap::Round).with_join(LineJoin::Round);
        canvas.stroke(&shape, &style, color);
    }
}

/// The stroke for an icon `size` points square: 1.5pt at 16pt.
pub fn stroke_width(size: f64) -> f64 {
    (1.5 * size / 16.0).max(1.0)
}

/// One SVG element's outline.
fn parse_element(el: &str) -> Option<BezPath> {
    let tag: String = el.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let a = |name: &str| attr(el, name);
    let n = |name: &str| a(name).and_then(|v| v.trim().parse::<f64>().ok());
    Some(match tag.as_str() {
        "path" => BezPath::from_svg(&a("d")?).ok()?,
        "circle" => Circle::new((n("cx")?, n("cy")?), n("r")?).to_path(0.05),
        "ellipse" => Ellipse::new((n("cx")?, n("cy")?), (n("rx")?, n("ry")?), 0.0).to_path(0.05),
        "line" => Line::new((n("x1")?, n("y1")?), (n("x2")?, n("y2")?)).to_path(0.05),
        "rect" => {
            let r = Rect::new(n("x").unwrap_or(0.0), n("y").unwrap_or(0.0), 0.0, 0.0);
            let r = Rect::from_origin_size(r.origin(), (n("width")?, n("height")?));
            let rx = n("rx").or(n("ry")).unwrap_or(0.0);
            RoundedRect::from_rect(r, rx).to_path(0.05)
        }
        "polyline" | "polygon" => {
            let nums: Vec<f64> =
                a("points")?.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|s| s.parse().ok()).collect();
            let mut p = BezPath::new();
            for (i, xy) in nums.chunks_exact(2).enumerate() {
                if i == 0 {
                    p.move_to((xy[0], xy[1]));
                } else {
                    p.line_to((xy[0], xy[1]));
                }
            }
            if tag == "polygon" {
                p.close_path();
            }
            p
        }
        _ => return None,
    })
}

/// An attribute's value in an element's text.
fn attr(el: &str, name: &str) -> Option<String> {
    let mut rest = el;
    while let Some(i) = rest.find(name) {
        let before_ok = i == 0 || rest.as_bytes()[i - 1] == b' ';
        let after = &rest[i + name.len()..];
        if before_ok && let Some(v) = after.strip_prefix("=\"") {
            return v.find('"').map(|e| v[..e].to_owned());
        }
        rest = &rest[i + name.len()..];
    }
    None
}

// The `icon` element.

/// An icon: `icon(Icon::Wifi).size(16.0).role(Role::Text2)`.
pub struct IconEl<Msg> {
    pub(crate) common: Common,
    icon: Icon,
    size: Option<f64>,
    role: Role,
    color: Option<Color>,
    _m: std::marker::PhantomData<fn() -> Msg>,
}

crate::builder!(IconEl);

pub fn icon<Msg>(icon: Icon) -> IconEl<Msg> {
    IconEl { common: Common::default(), icon, size: None, role: Role::Text2, color: None, _m: std::marker::PhantomData }
}

impl<Msg> IconEl<Msg> {
    pub fn size(mut self, size: f64) -> Self {
        self.size = Some(size);
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl<Msg> From<IconEl<Msg>> for Element<Msg> {
    fn from(i: IconEl<Msg>) -> Element<Msg> {
        Element::from_raw(RawElement::new(
            IconWidget { icon: i.icon, size: i.size, role: i.role, color: i.color },
            i.common,
            Vec::new(),
        ))
    }
}

pub(crate) struct IconWidget {
    pub icon: Icon,
    pub size: Option<f64>,
    pub role: Role,
    pub color: Option<Color>,
}

impl Widget for IconWidget {
    fn name(&self) -> &'static str {
        "icon"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let s = self.size.unwrap_or(cx.theme().metrics.icon);
        bc.constrain(Size::new(s, s))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let s = self.size.unwrap_or(cx.theme().metrics.icon);
        let color = self.color.unwrap_or_else(|| cx.theme().role(self.role));
        let b = cx.size();
        let at = Point::new(((b.width - s) / 2.0).round(), ((b.height - s) / 2.0).round());
        self.icon.draw(cx.canvas, at, s, color);
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.add(&(self.icon, self.role)).f64(self.size.unwrap_or(0.0));
        if let Some(c) = self.color {
            h.color(c);
        }
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::Image, label: Some(self.icon.name().to_owned()), ..Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_inside_its_grid() {
        assert!(Icon::ALL.len() > 200);
        for &i in Icon::ALL {
            let p = i.path();
            assert!(!p.elements().is_empty(), "{} has an outline", i.name());
            let b = p.bounding_box();
            assert!(b.x0 >= -0.5 && b.y0 >= -0.5 && b.x1 <= 24.5 && b.y1 <= 24.5, "{} within 24: {b:?}", i.name());
        }
        assert_eq!(Icon::from_name("wifi"), Some(Icon::Wifi));
        assert_eq!(Icon::Wifi.name(), "wifi");
    }

    #[test]
    fn strokes_follow_size() {
        assert_eq!(stroke_width(16.0), 1.5);
        assert_eq!(stroke_width(32.0), 3.0);
    }

    #[test]
    fn attributes_are_found_by_whole_name() {
        let el = r#"rect x="2" y="6" width="16" height="12" rx="2""#;
        assert_eq!(attr(el, "x").as_deref(), Some("2"));
        assert_eq!(attr(el, "rx").as_deref(), Some("2"));
        assert_eq!(attr(el, "width").as_deref(), Some("16"));
    }
}
