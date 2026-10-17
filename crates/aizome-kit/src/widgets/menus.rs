//! Menus as they draw: a menu's popup, the in-window menu bar (where
//! there's no Aizome shell to publish to), and tooltips.
//!
//! The look is the boards': `raised` rows 24pt tall with 10pt padding,
//! labels in Archivo 13, shortcuts right-aligned in IBM Plex Mono 11.5 in
//! `text3` (the modifier glyphs from Noto Sans Symbols 2), the hovered row
//! filled with `selection`, separators as an engraved `line`, a 1pt
//! `bevel_lo` edge with the top highlight, and the hard 3pt shadow.

use std::time::{Duration, Instant};

use sidestep_ui::{Key, NamedKey, PointerButton};

use crate::draw;
use crate::element::{Common, RawElement};
use crate::engine::KitMsg;
use crate::hash::PaintHasher;
use crate::icons::Icon;
use crate::layout::{Align, Constraints, Length, Point, Rect, Size};
use crate::menu::{ItemSpec, MenuSpec};
use crate::platform::WinId;
use crate::program::AppInfo;
use crate::text::TextSpec;
use crate::theme::{FontSpec, Theme};
use crate::widget::{Event, EventCx, LayoutCx, PaintCx, SemanticAction, SemanticRole, Semantics, Status, Widget};

const PAD: f64 = 10.0;
const GUTTER: f64 = 18.0;
const ARROW: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Action,
    Submenu,
    Separator,
    Header,
}

#[derive(Clone, Copy, Debug)]
struct Row {
    y: f64,
    h: f64,
    item: usize,
    kind: RowKind,
    enabled: bool,
}

/// What a menu remembers while open.
#[derive(Default)]
pub(crate) struct MenuState {
    hover: Option<usize>,
    open_sub: Option<usize>,
    typed: String,
    typed_at: Option<Instant>,
}

/// A menu's popup.
pub(crate) struct MenuView {
    spec: MenuSpec,
    popup: WinId,
    min_width: f64,
    rows: Vec<Row>,
    gutter: f64,
    width: f64,
}

impl MenuView {
    pub fn element(spec: MenuSpec, popup: WinId, _bar: bool) -> RawElement {
        MenuView::element_min(spec, popup, _bar, 0.0)
    }

    pub fn element_min(spec: MenuSpec, popup: WinId, _bar: bool, min_width: f64) -> RawElement {
        let view = MenuView { spec, popup, min_width, rows: Vec::new(), gutter: 0.0, width: 0.0 };
        RawElement::new(view, Common::default(), Vec::new())
    }

    fn label_spec(theme: &Theme) -> TextSpec {
        TextSpec::new(FontSpec::ui(theme.metrics.body), theme.text())
    }

    fn shortcut_spec(theme: &Theme) -> TextSpec {
        TextSpec::new(FontSpec::mono(theme.metrics.shortcut), theme.text3())
    }

    fn row_at(&self, p: Point) -> Option<usize> {
        self.rows.iter().position(|r| p.y >= r.y && p.y < r.y + r.h && p.x >= 0.0 && p.x < self.width)
    }

    fn row_rect(&self, i: usize) -> Rect {
        let r = &self.rows[i];
        Rect::new(1.0, r.y, self.width - 1.0, r.y + r.h)
    }

    fn selectable(&self, i: usize) -> bool {
        let r = &self.rows[i];
        r.enabled && matches!(r.kind, RowKind::Action | RowKind::Submenu)
    }

    fn set_hover(&self, cx: &mut EventCx, row: Option<usize>) {
        let old = cx.state::<MenuState>().hover;
        if old == row {
            return;
        }
        cx.state::<MenuState>().hover = row;
        for r in [old, row].into_iter().flatten() {
            let rect = self.row_rect(r);
            cx.invalidate_rect(rect);
        }
        // A submenu opens under the pointer; moving to another item
        // closes it.
        match row.map(|r| (r, self.rows[r])) {
            Some((i, r)) if r.kind == RowKind::Submenu && r.enabled => {
                if cx.state::<MenuState>().open_sub != Some(r.item) {
                    cx.state::<MenuState>().open_sub = Some(r.item);
                    let anchor = self.row_rect(i) + cx.window_rect().origin().to_vec2();
                    cx.emit_any(Box::new(KitMsg::OpenSubmenu { popup: self.popup, path: vec![r.item], anchor }));
                }
            }
            Some(_) => {
                if cx.state::<MenuState>().open_sub.take().is_some() {
                    cx.emit_any(Box::new(KitMsg::CloseSubmenus { popup: self.popup }));
                }
            }
            None => {}
        }
    }

    fn activate_row(&self, cx: &mut EventCx, i: usize) {
        let row = self.rows[i];
        if !row.enabled {
            return;
        }
        match (&self.spec.items[row.item], row.kind) {
            (ItemSpec::Action(_), RowKind::Action) => {
                cx.emit_any(Box::new(KitMsg::Activate { popup: self.popup, item: row.item }));
            }
            (ItemSpec::Submenu(_), RowKind::Submenu) => {
                cx.state::<MenuState>().open_sub = Some(row.item);
                let anchor = self.row_rect(i) + cx.window_rect().origin().to_vec2();
                cx.emit_any(Box::new(KitMsg::OpenSubmenu { popup: self.popup, path: vec![row.item], anchor }));
            }
            _ => {}
        }
    }

    fn step(&self, cx: &mut EventCx, down: bool) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let start = cx.state::<MenuState>().hover;
        let mut i = start.unwrap_or(if down { n - 1 } else { 0 });
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if self.selectable(i) {
                self.set_hover_keyboard(cx, Some(i));
                return;
            }
        }
    }

    /// Hover from the keyboard: no submenu opens by itself.
    fn set_hover_keyboard(&self, cx: &mut EventCx, row: Option<usize>) {
        let old = cx.state::<MenuState>().hover;
        cx.state::<MenuState>().hover = row;
        for r in [old, row].into_iter().flatten() {
            let rect = self.row_rect(r);
            cx.invalidate_rect(rect);
        }
    }

    fn label(&self, i: usize) -> &str {
        match &self.spec.items[self.rows[i].item] {
            ItemSpec::Action(a) => &a.label,
            ItemSpec::Submenu(m) => &m.title,
            ItemSpec::Header(h) => h,
            ItemSpec::Separator => "",
        }
    }
}

impl Widget for MenuView {
    fn name(&self) -> &'static str {
        "menu"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let theme = cx.env.theme;
        let m = &theme.metrics;
        let label = MenuView::label_spec(theme);
        let shortcut = MenuView::shortcut_spec(theme);
        let caption = TextSpec::new(FontSpec::mono(m.caption), theme.text3());
        self.gutter = if self.spec.items.iter().any(|i| matches!(i, ItemSpec::Action(a) if a.checked || a.icon.is_some())) {
            GUTTER
        } else {
            0.0
        };
        let mut rows = Vec::new();
        let mut y = 1.0 + 4.0;
        let mut widest: f64 = 0.0;
        for (item, spec) in self.spec.items.iter().enumerate() {
            let (kind, h, enabled, w) = match spec {
                ItemSpec::Action(a) => {
                    let lw = cx.measure(&a.label, &label).width;
                    let sw = a.shortcut.map_or(0.0, |s| cx.measure(&s.label(), &shortcut).width + 24.0);
                    (RowKind::Action, m.menu_row, a.enabled, lw + sw)
                }
                ItemSpec::Submenu(s) => {
                    let lw = cx.measure(&s.title, &label).width;
                    (RowKind::Submenu, m.menu_row, !s.items.is_empty(), lw + ARROW + 12.0)
                }
                ItemSpec::Separator => (RowKind::Separator, 9.0, false, 0.0),
                ItemSpec::Header(t) => {
                    let w = cx.measure(&t.to_uppercase(), &caption).width;
                    (RowKind::Header, 22.0, false, w)
                }
            };
            widest = widest.max(w);
            rows.push(Row { y, h, item, kind, enabled });
            y += h;
        }
        y += 4.0 + 1.0;
        self.rows = rows;
        let width = (widest + 2.0 * PAD + self.gutter + 2.0).ceil().max(180.0).max(self.min_width).min(bc.max.width - 3.0);
        self.width = width;
        let shadow = m.shadow;
        Size::new(width + shadow, y + shadow)
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let panel = Rect::new(0.0, 0.0, self.width, cx.size().height - t.metrics.shadow);
        draw::floating(cx.canvas, t, panel);
        let hover = cx.state::<MenuState>().and_then(|s| s.hover.or(s.open_sub.and_then(|it| self.rows.iter().position(|r| r.item == it))));
        let label_spec = MenuView::label_spec(t);
        let sc_spec = MenuView::shortcut_spec(t);
        for (i, row) in self.rows.iter().enumerate() {
            let r = self.row_rect(i);
            if !cx.canvas.needs(r) {
                continue;
            }
            match row.kind {
                RowKind::Separator => draw::separator(cx.canvas, t, 1.0, self.width - 1.0, row.y + 4.0),
                RowKind::Header => {
                    let spec = TextSpec { letter_spacing: 1.2, ..TextSpec::new(FontSpec::mono(t.metrics.caption), t.text3()) };
                    let text = self.label(i).to_uppercase();
                    cx.draw_text_in(&text, &spec, Rect::new(r.x0 + PAD, r.y0 + 6.0, r.x1 - PAD, r.y1), Align::Start);
                }
                RowKind::Action | RowKind::Submenu => {
                    let hot = hover == Some(i) && row.enabled;
                    if hot {
                        cx.canvas.fill_rect(r, t.selection());
                    }
                    let fg = if !row.enabled {
                        t.faint()
                    } else if hot {
                        t.on_selection()
                    } else {
                        t.text()
                    };
                    let x = r.x0 + PAD + self.gutter;
                    let mut right = r.x1 - PAD;
                    if row.kind == RowKind::Submenu {
                        let c = if hot { t.on_selection() } else { t.text3() };
                        Icon::ChevronRight.draw(cx.canvas, Point::new(right - 12.0, r.y0 + (r.height() - 12.0) / 2.0), 12.0, c);
                        right -= ARROW;
                    }
                    if let ItemSpec::Action(a) = &self.spec.items[row.item] {
                        if a.checked {
                            Icon::Check.draw(cx.canvas, Point::new(r.x0 + PAD - 3.0, r.y0 + (r.height() - 14.0) / 2.0), 14.0, fg);
                        } else if let Some(icon) = a.icon {
                            let c = if hot { fg } else { t.text3() };
                            icon.draw(cx.canvas, Point::new(r.x0 + PAD - 3.0, r.y0 + (r.height() - 14.0) / 2.0), 14.0, c);
                        }
                        if let Some(s) = a.shortcut {
                            let c = if hot { t.on_selection() } else if row.enabled { t.text3() } else { t.faint() };
                            let spec = TextSpec { color: c, ..sc_spec.clone() };
                            let w = cx.draw_text_in(&s.label(), &spec, Rect::new(x, r.y0, right, r.y1), Align::End);
                            right -= w + 16.0;
                        }
                    }
                    let spec = TextSpec { color: fg, ..label_spec.clone() };
                    cx.draw_text_in(self.label(i), &spec, Rect::new(x, r.y0, right.max(x), r.y1), Align::Start);
                }
            }
        }
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.str(&self.spec.title).f64(self.width);
        for i in &self.spec.items {
            match i {
                ItemSpec::Action(a) => {
                    h.str(&a.label).add(&(a.enabled, a.checked, a.shortcut, a.icon));
                }
                ItemSpec::Submenu(m) => {
                    h.str(&m.title);
                }
                ItemSpec::Header(t) => {
                    h.str(t);
                }
                ItemSpec::Separator => {
                    h.add(&0u8);
                }
            }
        }
    }

    fn hit(&self) -> bool {
        true
    }

    fn hover_paints(&self) -> bool {
        false
    }

    fn overflow(&self) -> f64 {
        3.0
    }

    fn event(&mut self, cx: &mut EventCx, event: &Event) -> Status {
        match event {
            Event::PointerMove { pos, .. } => {
                let row = self.row_at(*pos).filter(|&i| self.selectable(i));
                self.set_hover(cx, row);
                Status::Handled
            }
            Event::PointerLeave => {
                if cx.state::<MenuState>().open_sub.is_none() {
                    self.set_hover_keyboard(cx, None);
                }
                Status::Handled
            }
            Event::PointerDown { .. } => Status::Handled,
            Event::PointerUp { pos, button: PointerButton::Left | PointerButton::Right, .. } => {
                if let Some(i) = self.row_at(*pos).filter(|&i| self.selectable(i)) {
                    self.activate_row(cx, i);
                }
                Status::Handled
            }
            Event::Key(k) if k.pressed => {
                match &k.key {
                    Key::Named(NamedKey::ArrowDown) => self.step(cx, true),
                    Key::Named(NamedKey::ArrowUp) => self.step(cx, false),
                    Key::Named(NamedKey::Home) => {
                        let first = (0..self.rows.len()).find(|&i| self.selectable(i));
                        self.set_hover_keyboard(cx, first);
                    }
                    Key::Named(NamedKey::End) => {
                        let last = (0..self.rows.len()).rev().find(|&i| self.selectable(i));
                        self.set_hover_keyboard(cx, last);
                    }
                    Key::Named(NamedKey::Enter) | Key::Named(NamedKey::ArrowRight) => {
                        if let Some(i) = cx.state::<MenuState>().hover {
                            let right = matches!(k.key, Key::Named(NamedKey::ArrowRight));
                            if !right || self.rows[i].kind == RowKind::Submenu {
                                self.activate_row(cx, i);
                            }
                        }
                    }
                    Key::Named(NamedKey::ArrowLeft) => cx.close_popup(),
                    Key::Named(NamedKey::Escape) => cx.emit_any(Box::new(KitMsg::CloseMenus { window: self.popup })),
                    Key::Character(s) if s == " " => {
                        if let Some(i) = cx.state::<MenuState>().hover {
                            self.activate_row(cx, i);
                        }
                    }
                    Key::Character(s) => {
                        // Type-select: the first item starting with what's
                        // been typed lately.
                        let now = cx.now();
                        let st = cx.state::<MenuState>();
                        if st.typed_at.is_none_or(|t| now.duration_since(t) > Duration::from_millis(900)) {
                            st.typed.clear();
                        }
                        st.typed.push_str(&s.to_lowercase());
                        st.typed_at = Some(now);
                        let typed = st.typed.clone();
                        let found = (0..self.rows.len())
                            .find(|&i| self.selectable(i) && self.label(i).to_lowercase().starts_with(&typed));
                        if found.is_some() {
                            self.set_hover_keyboard(cx, found);
                        }
                    }
                    _ => return Status::Ignored,
                }
                Status::Handled
            }
            _ => Status::Ignored,
        }
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::Menu, label: Some(self.spec.title.clone()), ..Default::default() })
    }
}

/// Which title of the in-window bar is open or under the pointer.
#[derive(Default)]
pub(crate) struct BarState {
    pub open: Option<usize>,
    pub hover: Option<usize>,
}

/// The menu bar along a window's top where there's no Aizome shell: the
/// app's name in bold, then its menus.
pub(crate) struct MenuBarStrip {
    titles: Vec<String>,
    win: WinId,
    spans: Vec<(f64, f64)>,
}

impl MenuBarStrip {
    pub fn element(info: &AppInfo, menus: &[MenuSpec], win: WinId) -> RawElement {
        let mut titles: Vec<String> = menus.iter().map(|m| m.title.clone()).collect();
        if let Some(first) = titles.first_mut()
            && first.is_empty()
        {
            first.clone_from(&info.name);
        }
        let mut c = Common::sized(Length::Fill, Length::Shrink);
        c.key = Some(2);
        RawElement::new(MenuBarStrip { titles, win, spans: Vec::new() }, c, Vec::new())
    }

    fn spec(theme: &Theme, i: usize) -> TextSpec {
        let mut f = FontSpec::ui(theme.metrics.body);
        if i == 0 {
            f.weight = aizome_theme::typeface::WEIGHT_BOLD;
        }
        TextSpec::new(f, theme.text())
    }

    fn at(&self, x: f64) -> Option<usize> {
        self.spans.iter().position(|(a, b)| x >= *a && x < *b)
    }

    fn open(&self, cx: &mut EventCx, i: usize) {
        let (a, b) = self.spans[i];
        let h = cx.size().height;
        let anchor = Rect::new(a, 0.0, b, h - 1.0) + cx.window_rect().origin().to_vec2();
        cx.state::<BarState>().open = Some(i);
        cx.invalidate();
        cx.emit_any(Box::new(KitMsg::OpenBarMenu { window: self.win, index: i, anchor }));
    }
}

impl Widget for MenuBarStrip {
    fn name(&self) -> &'static str {
        "menu-bar"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let theme = cx.env.theme;
        let mut x = 3.0;
        self.spans.clear();
        for i in 0..self.titles.len() {
            let w = cx.measure(&self.titles[i], &MenuBarStrip::spec(theme, i)).width.ceil();
            self.spans.push((x, x + w + 18.0));
            x += w + 18.0;
        }
        bc.constrain(Size::new(x, theme.metrics.target))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let r = cx.bounds();
        cx.canvas.fill_rect(r, t.bar());
        draw::bottom(cx.canvas, r, t.bevel_lo(), 1.0);
        draw::bottom(cx.canvas, Rect::new(r.x0, r.y0, r.x1, r.y1 - 1.0), t.bevel_hi(), 1.0);
        let state = cx.state::<BarState>();
        let (open, hover) = state.map_or((None, None), |s| (s.open, s.hover));
        for (i, (a, b)) in self.spans.iter().enumerate() {
            let cell = Rect::new(*a, 0.0, *b, r.y1 - 2.0);
            let fg = if open == Some(i) {
                cx.canvas.fill_rect(cell, t.selection());
                t.on_selection()
            } else {
                if hover == Some(i) {
                    cx.canvas.fill_rect(cell, t.hover(t.bar()));
                }
                t.text()
            };
            let spec = TextSpec { color: fg, ..MenuBarStrip::spec(t, i) };
            cx.draw_text_in(&self.titles[i], &spec, cell, Align::Center);
        }
    }

    fn hash(&self, h: &mut PaintHasher) {
        for t in &self.titles {
            h.str(t);
        }
    }

    fn hit(&self) -> bool {
        true
    }

    fn solid(&self) -> bool {
        true
    }

    fn event(&mut self, cx: &mut EventCx, event: &Event) -> Status {
        match event {
            Event::PointerDown { pos, button: PointerButton::Left, .. } => {
                match self.at(pos.x) {
                    Some(i) if cx.state::<BarState>().open == Some(i) => {
                        cx.state::<BarState>().open = None;
                        cx.invalidate();
                        cx.emit_any(Box::new(KitMsg::CloseMenus { window: self.win }));
                    }
                    Some(i) => self.open(cx, i),
                    None => {}
                }
                Status::Handled
            }
            Event::PointerMove { pos, .. } => {
                let at = self.at(pos.x);
                let st = cx.state::<BarState>();
                if st.hover != at {
                    st.hover = at;
                    let open = st.open;
                    cx.invalidate();
                    if let (Some(o), Some(i)) = (open, at)
                        && o != i
                    {
                        self.open(cx, i);
                    }
                }
                Status::Handled
            }
            Event::PointerLeave => {
                cx.state::<BarState>().hover = None;
                cx.invalidate();
                Status::Handled
            }
            _ => Status::Ignored,
        }
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::MenuBar, actions: vec![SemanticAction::ShowMenu], ..Default::default() })
    }
}

/// A tooltip: a dark label, as the boards' drag hint.
pub(crate) struct TooltipView {
    text: String,
}

impl TooltipView {
    pub fn element(text: &str) -> RawElement {
        RawElement::new(TooltipView { text: text.to_owned() }, Common::default(), Vec::new())
    }

    fn spec(theme: &Theme) -> TextSpec {
        let mut f = FontSpec::ui(theme.metrics.small + 1.0);
        f.weight = aizome_theme::typeface::WEIGHT_SEMIBOLD;
        TextSpec::new(f, theme.on_menu_title())
    }
}

impl Widget for TooltipView {
    fn name(&self) -> &'static str {
        "tooltip"
    }

    fn layout(&mut self, cx: &mut LayoutCx, bc: Constraints) -> Size {
        let spec = TooltipView::spec(cx.env.theme);
        let s = cx.measure(&self.text, &spec);
        bc.constrain(Size::new((s.width + 16.0).ceil().min(420.0), (s.height + 8.0).ceil().max(22.0)))
    }

    fn paint(&self, cx: &mut PaintCx) {
        let t = cx.theme();
        let r = cx.bounds();
        cx.canvas.fill_rect(r, t.menu_title());
        draw::border(cx.canvas, r, t.bevel_lo(), 1.0);
        let spec = TooltipView::spec(t);
        cx.draw_text_in(&self.text, &spec, r.inset((-8.0, 0.0)), Align::Start);
    }

    fn hash(&self, h: &mut PaintHasher) {
        h.str(&self.text);
    }

    fn semantics(&self) -> Option<Semantics> {
        Some(Semantics { role: SemanticRole::Tooltip, label: Some(self.text.clone()), ..Default::default() })
    }
}
