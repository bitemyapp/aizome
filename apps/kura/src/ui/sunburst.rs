//! The sunburst: the current folder in the middle, what it holds in rings
//! around it by size, drawn with the kit's `canvas` element.
//!
//! Everything it shows is the program's state ([`Burst`]): hovering a
//! segment lights it and puts its name and size in the middle; a click on
//! a folder goes into it, a click in the middle goes up a level, a right
//! click opens the item's menu, and dragging a segment carries it to the
//! Collector. Going down or up a level moves the rings over Aizome's
//! motion time (none with reduced motion): the child opens out to the whole
//! turn while its siblings fold away and the rings step inwards, or the
//! reverse.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aizome_kit::sidestep_ui::kurbo::{Circle, CircleSegment};
use aizome_kit::sidestep_ui::{Canvas, Color, PointerButton, StrokeStyle, TextLayout};
use aizome_kit::theme::FontSpec;
use aizome_kit::widget::Event;
use aizome_kit::{Icon, Point, Rect, Size, Theme};

use super::colors;
use super::drag::{DragEvent, SLOP, ZoneCell};
use super::rings::{self, Geometry, Place, RINGS, Seg, Zoom};
use super::Msg;
use crate::format;
use crate::scan::Scan;
use crate::tree::NodeId;

/// Frames while the rings move.
pub const FRAME: Duration = Duration::from_millis(8);

/// The centre moving one level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// The folder a level up.
    pub outer: NodeId,
    /// Its child, the centre before or after.
    pub inner: NodeId,
    /// Down into `inner` (else up out of it).
    pub zoom_in: bool,
    pub started: Instant,
    pub duration: Duration,
}

impl Transition {
    /// How far along, 0–1 (eased), or `None` once done.
    pub fn progress(&self, now: Instant) -> Option<f64> {
        let e = now.saturating_duration_since(self.started);
        (e < self.duration && !self.duration.is_zero()).then(|| rings::ease(e.as_secs_f64() / self.duration.as_secs_f64()))
    }
}

/// What the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Center,
    Seg(usize),
}

/// Text laid out once and drawn every frame.
#[derive(Default)]
pub struct Labels {
    made: HashMap<(String, u64), TextLayout>,
}

impl Labels {
    fn get(&mut self, theme: &Theme, text: &str, font: FontSpec, color: Color, width: Option<f64>) -> TextLayout {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (font.size.to_bits(), font.weight.to_bits(), font.tabular, color.r.to_bits(), color.g.to_bits(), color.b.to_bits(), color.a.to_bits())
            .hash(&mut h);
        width.map(f64::to_bits).hash(&mut h);
        let key = (text.to_owned(), h.finish());
        if self.made.len() > 2048 {
            self.made.clear();
        }
        self.made
            .entry(key)
            .or_insert_with(|| {
                let style = theme.style(font, color);
                match width {
                    Some(w) => aizome_kit::sidestep_ui::TextLayout::builder(text, &style)
                        .width(w)
                        .wrap(aizome_kit::sidestep_ui::Wrap::Truncate)
                        .max_lines(1)
                        .build(),
                    None => TextLayout::new(text, &style),
                }
            })
            .clone()
    }
}

/// The canvas's state: what it draws, and what its input means.
pub struct Burst {
    pub scan: Arc<Scan>,
    /// Bumped when the tree changes after the scan (a trashing).
    pub generation: u64,
    pub center: NodeId,
    pub segs: Rc<Vec<Seg>>,
    /// The outer folder's segments while the centre moves.
    pub outer: Option<Rc<Vec<Seg>>>,
    pub transition: Option<Transition>,
    /// The transition's progress now (eased), while it runs.
    pub progress: Option<f64>,
    pub hover: Option<Target>,
    pub selected: Option<NodeId>,
    pub press: Option<(Target, Point)>,
    pub dragging: bool,
    pub drag_over: bool,
    /// Whether the middle goes up a level.
    pub can_up: bool,
    /// Where the canvas is in the window, as last drawn.
    pub origin: Rc<Cell<Option<Point>>>,
    pub zone: ZoneCell,
    pub labels: Rc<RefCell<Labels>>,
}

impl Hash for Burst {
    fn hash<H: Hasher>(&self, h: &mut H) {
        (Arc::as_ptr(&self.scan) as usize).hash(h);
        self.generation.hash(h);
        self.center.hash(h);
        self.hover.hash(h);
        self.selected.hash(h);
        self.can_up.hash(h);
        self.progress.map(|p| (p * 1000.0) as u32).hash(h);
        self.transition.map(|t| (t.outer, t.inner, t.zoom_in)).hash(h);
    }
}

impl Burst {
    fn node_of(&self, t: Target) -> Option<NodeId> {
        match t {
            Target::Center => Some(self.center),
            Target::Seg(i) => self.segs.get(i).and_then(|s| s.node),
        }
    }

    fn target(&self, geo: &Geometry, p: Point) -> Option<Target> {
        match geo.locate(p) {
            Place::Center => Some(Target::Center),
            Place::Ring(r, x) => rings::hit(&self.segs, r, x).map(Target::Seg),
            Place::Outside => None,
        }
    }

    /// A point of the canvas in the window.
    fn in_window(&self, p: Point) -> Option<Point> {
        self.origin.get().map(|o| o + p.to_vec2())
    }

    fn draggable(&self, n: NodeId) -> bool {
        n != self.center && !crate::nix::in_store(&self.scan.tree.path(n))
    }

    /// What input over the canvas means (positions in its coordinates).
    pub fn event(&self, ev: &Event, size: Size) -> Option<Msg> {
        let geo = Geometry::new(size, RINGS);
        match ev {
            Event::PointerMove { pos, .. } => {
                if let Some((t, from)) = self.press {
                    let node = self.node_of(t)?;
                    if !self.dragging {
                        return ((*pos - from).hypot() >= SLOP && self.draggable(node)).then_some(Msg::Drag(DragEvent::Start(node)));
                    }
                    let over = self.in_window(*pos).is_some_and(|p| self.zone.get().is_some_and(|z| z.contains(p)));
                    return (over != self.drag_over).then_some(Msg::Drag(DragEvent::Over(over)));
                }
                let t = if self.progress.is_some() { None } else { self.target(&geo, *pos) };
                (t != self.hover).then_some(Msg::Hover(t))
            }
            Event::PointerLeave => (self.hover.is_some() && self.press.is_none()).then_some(Msg::Hover(None)),
            Event::PointerDown { pos, button: PointerButton::Left, .. } => {
                self.target(&geo, *pos).map(|t| Msg::Press(Some((t, *pos))))
            }
            Event::PointerDown { pos, button: PointerButton::Right, .. } => {
                let t = self.target(&geo, *pos)?;
                let node = match (t, self.node_of(t)) {
                    (_, Some(n)) => n,
                    (Target::Seg(i), None) => self.segs[i].parent,
                    (Target::Center, None) => self.center,
                };
                Some(Msg::Context(node, self.in_window(*pos).unwrap_or(*pos)))
            }
            Event::PointerUp { pos, button: PointerButton::Left, .. } => {
                (self.press.is_some() || self.dragging).then(|| Msg::Release(self.target(&geo, *pos)))
            }
            _ => None,
        }
    }

    fn fill(theme: &Theme, s: &Seg) -> Color {
        match s.node {
            Some(_) => colors::segment(theme, s.branch, s.depth, s.is_dir),
            None => colors::small(theme, s.depth),
        }
    }

    /// Draw it in `bounds`.
    pub fn draw(&self, canvas: &mut Canvas, bounds: Rect, theme: &Theme) {
        self.origin.set(Some(canvas.transform() * bounds.origin()));
        let geo = Geometry::new(bounds.size(), RINGS);
        let surface = aizome_kit::theme::color(theme.palette.surface);
        canvas.fill_rect(bounds, surface);
        let sep = StrokeStyle::new(1.0);
        let raised = aizome_kit::theme::color(theme.palette.raised);

        if let (Some(t), Some(p), Some(outer)) = (self.transition, self.progress, &self.outer) {
            // The outer folder's rings, folding to (or out of) the inner one.
            let Some(inner) = outer.iter().find(|s| s.depth == 1 && s.node == Some(t.inner)) else { return };
            let s = if t.zoom_in { p } else { 1.0 - p };
            let z = Zoom { c0: inner.x0, c1: inner.x1, s };
            let r0 = geo.boundary(-s);
            if r0 > 0.5 {
                canvas.fill(&Circle::new(geo.center, r0), raised);
            }
            for seg in outer.iter() {
                let (x0, x1) = (z.x(seg.x0), z.x(seg.x1));
                if x1 - x0 < 1e-4 {
                    continue;
                }
                let (ri, ro) = geo.radii(z.place(seg.depth));
                if ri >= geo.outer() - 0.5 {
                    continue;
                }
                let ro = ro.min(geo.outer());
                let mut fill = Burst::fill(theme, seg);
                // What ends inside the inner folder takes its colours there.
                if seg.node == Some(t.inner) {
                    fill = fill.mix(raised, s as f32);
                } else if seg.depth >= 2 && seg.branch == inner.branch {
                    let after = match seg.node {
                        Some(_) => colors::segment(theme, seg.sub, seg.depth - 1, seg.is_dir),
                        None => colors::small(theme, seg.depth - 1),
                    };
                    fill = fill.mix(after, s as f32);
                }
                let shape = CircleSegment::new(geo.center, ro, ri, Geometry::angle(x0), (x1 - x0) * TAU);
                canvas.fill(&shape, fill);
                canvas.stroke(&shape, &sep, surface);
            }
            return;
        }

        for (i, seg) in self.segs.iter().enumerate() {
            let (ri, ro) = geo.radii(f64::from(seg.depth));
            let mut fill = Burst::fill(theme, seg);
            if self.hover == Some(Target::Seg(i)) {
                fill = colors::hovered(fill);
            }
            let shape = CircleSegment::new(geo.center, ro, ri, Geometry::angle(seg.x0), seg.span() * TAU);
            canvas.fill(&shape, fill);
            canvas.stroke(&shape, &sep, surface);
        }
        // The selected item, ringed in the focus colour.
        if let Some(sel) = self.selected
            && let Some(seg) = self.segs.iter().find(|s| s.node == Some(sel))
        {
            let (ri, ro) = geo.radii(f64::from(seg.depth));
            let shape = CircleSegment::new(geo.center, ro - 1.0, ri + 1.0, Geometry::angle(seg.x0), seg.span() * TAU);
            canvas.stroke(&shape, &StrokeStyle::new(2.0), theme.focus());
        }
        let mut labels = self.labels.borrow_mut();
        self.label_segments(canvas, theme, &geo, &mut labels);
        self.middle(canvas, theme, &geo, &mut labels);
    }

    /// Names (and sizes, room allowing) on the largest segments of the
    /// first two rings.
    fn label_segments(&self, canvas: &mut Canvas, theme: &Theme, geo: &Geometry, labels: &mut Labels) {
        let tree = &self.scan.tree;
        let mut order: Vec<usize> = (0..self.segs.len()).filter(|&i| self.segs[i].depth <= 2 && self.segs[i].node.is_some()).collect();
        order.sort_by(|&a, &b| self.segs[b].span().total_cmp(&self.segs[a].span()));
        let small = theme.metrics.small;
        for i in order.into_iter().take(24) {
            let seg = self.segs[i];
            if seg.span() < 0.03 {
                break;
            }
            let Some(node) = seg.node else { continue };
            let (ri, ro) = geo.radii(f64::from(seg.depth));
            let fill = Burst::fill(theme, &seg);
            let ink = colors::text_on(if self.hover == Some(Target::Seg(i)) { colors::hovered(fill) } else { fill });
            let mut name: String = tree.name(node).into_owned();
            if name.chars().count() > 22 {
                name = name.chars().take(21).collect::<String>() + "…";
            }
            let mut nf = FontSpec::ui(small + 0.5);
            nf.weight = 600.0;
            let name_l = labels.get(theme, &name, nf, ink, None);
            let mut sf = FontSpec::ui(small);
            sf.tabular = true;
            let size_l = labels.get(theme, &format::size(tree.node(node).alloc), sf, ink.with_alpha(0.8), None);
            let (nw, nh) = (name_l.size().width, name_l.size().height);
            let (sw, sh) = (size_l.size().width, size_l.size().height);
            let mid = Geometry::angle((seg.x0 + seg.x1) / 2.0);
            let rm = (ri + ro) / 2.0;
            let c = Point::new(geo.center.x + rm * mid.cos(), geo.center.y + rm * mid.sin());
            let both = Rect::new(c.x - nw.max(sw) / 2.0, c.y - (nh + sh) / 2.0, c.x + nw.max(sw) / 2.0, c.y + (nh + sh) / 2.0);
            if fits(geo, &seg, ri, ro, both.inflate(3.0, 1.0)) {
                canvas.draw_text(&name_l, Point::new((c.x - nw / 2.0).round(), both.y0.round()));
                canvas.draw_text(&size_l, Point::new((c.x - sw / 2.0).round(), (both.y0 + nh).round()));
                continue;
            }
            let one = Rect::new(c.x - nw / 2.0, c.y - nh / 2.0, c.x + nw / 2.0, c.y + nh / 2.0);
            if fits(geo, &seg, ri, ro, one.inflate(3.0, 1.0)) {
                canvas.draw_text(&name_l, Point::new(one.x0.round(), one.y0.round()));
                continue;
            }
            // Along the arc, for long thin segments.
            let mid_x = (seg.x0 + seg.x1) / 2.0;
            let lines: [&[&TextLayout]; 2] = [&[&name_l, &size_l], &[&name_l]];
            for set in lines {
                let w = set.iter().map(|l| l.size().width).fold(0.0, f64::max);
                let h: f64 = set.iter().map(|l| l.size().height).sum();
                let r = (ri + ro) / 2.0;
                let half = (w / 2.0 + 4.0).atan2(r - h / 2.0);
                let room = (seg.span() * TAU) / 2.0;
                let outer = ((r + h / 2.0).powi(2) + (w / 2.0 + 3.0).powi(2)).sqrt();
                if half >= room || r - h / 2.0 < ri + 2.0 || outer > ro - 1.5 {
                    continue;
                }
                let a = Geometry::angle(mid_x);
                // Upright: the lower half reads along the bottom.
                let turn = if a.sin() > 0.0 { a - FRAC_PI_2 } else { a + FRAC_PI_2 };
                canvas.save();
                canvas.translate(geo.center.to_vec2() + aizome_kit::Vec2::new(r * a.cos(), r * a.sin()));
                canvas.rotate(turn);
                let mut y = -h / 2.0;
                for l in set {
                    canvas.draw_text(l, Point::new(-l.size().width / 2.0, y));
                    y += l.size().height;
                }
                canvas.restore();
                break;
            }
        }
    }

    /// The middle: the folder (or what the pointer is on), its size and
    /// what's in it, and the way up.
    fn middle(&self, canvas: &mut Canvas, theme: &Theme, geo: &Geometry, labels: &mut Labels) {
        let tree = &self.scan.tree;
        let raised = aizome_kit::theme::color(theme.palette.raised);
        let hole = Circle::new(geo.center, geo.hole - 1.0);
        let lit = self.hover == Some(Target::Center) && self.can_up;
        canvas.fill(&hole, if lit { theme.hover(raised) } else { raised });
        canvas.stroke(&hole, &StrokeStyle::new(1.0), theme.bevel_lo());

        let total = tree.node(self.center).alloc;
        let (title, size, detail) = match self.hover {
            Some(Target::Seg(i)) => {
                let seg = self.segs[i];
                match seg.node {
                    Some(n) => {
                        let node = tree.node(n);
                        let what = if node.is_dir() {
                            format::plural(u64::from(node.items), "item", "items")
                        } else {
                            crate::kinds::classify(tree.name_bytes(n), false).title().to_owned()
                        };
                        (tree.name(n).into_owned(), node.alloc, format!("{what} · {}", format::percent(node.alloc, total)))
                    }
                    None => {
                        let bytes = ((seg.x1 - seg.x0) * total as f64) as u64;
                        ("Smaller items".to_owned(), bytes, format!("in {}", tree.name(seg.parent)))
                    }
                }
            }
            _ => {
                let n = tree.node(self.center);
                let title = if self.center == tree.root() { format::path(tree.root_path()) } else { tree.name(self.center).into_owned() };
                (title, n.alloc, format::plural(u64::from(n.items), "item", "items"))
            }
        };
        let width = (geo.hole * 1.6).floor();
        let mut tf = FontSpec::ui(theme.metrics.body);
        tf.weight = 600.0;
        let title_l = labels.get(theme, &title, tf, theme.text(), Some(width));
        let mut sf = FontSpec::ui(theme.metrics.title + 3.0);
        sf.weight = 650.0;
        sf.tabular = true;
        let size_l = labels.get(theme, &format::size(size), sf, theme.text(), None);
        let detail_l = labels.get(theme, &detail, FontSpec::ui(theme.metrics.small), theme.text3(), Some(width));
        let h = title_l.size().height + size_l.size().height + detail_l.size().height + 2.0;
        let mut y = geo.center.y - h / 2.0;
        if self.can_up && self.hover.is_none_or(|t| t == Target::Center) {
            let s = 14.0;
            let c = if lit { theme.text() } else { theme.text3() };
            Icon::ArrowUp.draw(canvas, Point::new(geo.center.x - s / 2.0, (y - s - 4.0).round()), s, c);
        }
        for l in [&title_l, &size_l] {
            canvas.draw_text(l, Point::new((geo.center.x - l.size().width / 2.0).round(), y.round()));
            y += l.size().height + 1.0;
        }
        canvas.draw_text(&detail_l, Point::new((geo.center.x - detail_l.size().width / 2.0).round(), y.round()));
    }
}

/// Whether `r` lies inside the segment's ring and angles.
fn fits(geo: &Geometry, seg: &Seg, ri: f64, ro: f64, r: Rect) -> bool {
    let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)];
    corners.iter().all(|&p| {
        let v = p - geo.center;
        let d = v.hypot();
        if d < ri + 1.0 || d > ro - 1.0 {
            return false;
        }
        let mut x = ((v.y.atan2(v.x) + FRAC_PI_2) / TAU).rem_euclid(1.0);
        if seg.x1 > 0.99 && x < 0.01 {
            x += 1.0;
        }
        x >= seg.x0 && x <= seg.x1
    })
}
