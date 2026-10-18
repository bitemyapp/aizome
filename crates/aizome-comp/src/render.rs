//! What an output shows, as smithay render elements, front to back:
//! the pointer, overlays (marking menu, switcher, overview), overlay-layer
//! surfaces, a focused full-screen window, top-layer surfaces, windows with
//! their tabs and frames, then bottom and background layers over the
//! desktop colour.
//!
//! Decorations are solid-colour elements with stable identities and
//! cached label textures, and window surfaces come straight from their
//! buffers, so the damage tracker redraws only what changed: moving a
//! window composites and asks nothing of its client.

use aizome_theme::typeface;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::element::utils::{CropRenderElement, RelocateRenderElement, RescaleRenderElement};
use smithay::backend::renderer::{ImportAll, ImportMem, Renderer};
use smithay::desktop::layer_map_for_output;
use smithay::desktop::utils::bbox_from_surface_tree;
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::decor::{self, Layout, TabLook};
use crate::state::Aizome;
use crate::text::{Face, Style};
use crate::wm::WinId;

smithay::backend::renderer::element::render_elements! {
    pub OutputElement<R> where R: ImportAll + ImportMem;
    Surface = WaylandSurfaceRenderElement<R>,
    Solid = SolidColorRenderElement,
    Memory = MemoryRenderBufferRenderElement<R>,
    Thumb = CropRenderElement<RelocateRenderElement<RescaleRenderElement<WaylandSurfaceRenderElement<R>>>>,
}

/// Who the frame is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The display itself.
    Screen,
    /// A screenshot or screen capture, with or without the pointer.
    Capture { cursor: bool },
}

/// The colour behind everything.
pub fn clear_color(az: &Aizome) -> [f32; 4] {
    if az.lock.is_locked() { [0.0, 0.0, 0.0, 1.0] } else { az.palette.desktop.to_premultiplied_f32() }
}

/// Everything an output shows, front first.
pub fn output_elements<R>(az: &mut Aizome, renderer: &mut R, output: &Output, purpose: Purpose) -> Vec<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let scale = output.current_scale().fractional_scale();
    let geo = az.output_geometry(output);
    let origin = (geo.loc.x as f64, geo.loc.y as f64);
    let mut out: Vec<OutputElement<R>> = Vec::new();

    let cursor = match purpose {
        Purpose::Screen => true,
        Purpose::Capture { cursor } => cursor,
    };
    if cursor {
        cursor_elements(az, renderer, output, scale, &mut out);
    }
    drag_icon_elements(az, renderer, output, scale, &mut out);

    if az.lock.is_locked() {
        if let Some(lock) = az.out.get(output).and_then(|o| o.lock_surface.clone()) {
            out.extend(
                render_elements_from_surface_tree(renderer, lock.wl_surface(), (0, 0), scale, 1.0, Kind::Unspecified)
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
        return out;
    }

    out.extend(crate::overlays::elements(az, renderer, output, scale));
    if az.overlays.covers_desktop(output) {
        // The overview hides the desktop entirely.
        return out;
    }

    popup_elements(az, renderer, output, scale, &mut out);
    layer_elements(renderer, output, Layer::Overlay, scale, &mut out);

    let shown = az.wm.shown_stacking();
    let desk_slide = az.anims.desk;
    let out_w = geo.size.w as f64;
    // A focused full-screen window covers the top layer too.
    let top_fullscreen = shown
        .last()
        .copied()
        .filter(|id| az.wm.get(*id).is_some_and(|w| w.fullscreen()) && az.wm.focused() == Some(*id));
    if let Some(id) = top_fullscreen {
        window_elements(az, renderer, id, origin, scale, (0.0, 0.0), 1.0, &mut out);
    }
    layer_elements(renderer, output, Layer::Top, scale, &mut out);

    out.extend(crate::overlays::drag_feedback(az, renderer, output, scale));

    match desk_slide {
        Some(slide) => {
            // Both Desks' windows, sliding; sticky ones stay put.
            let t = slide.progress();
            let mut list: Vec<(WinId, f64)> = Vec::new();
            for id in az.wm.shown_stacking_on(slide.to) {
                let sticky = az.wm.get(id).is_some_and(|w| w.sticky);
                list.push((id, if sticky { 0.0 } else { (1.0 - t) * out_w * slide.dir }));
            }
            let incoming: Vec<WinId> = list.iter().map(|(id, _)| *id).collect();
            let mut outgoing: Vec<(WinId, f64)> = az
                .wm
                .shown_stacking_on(slide.from)
                .into_iter()
                .filter(|id| !incoming.contains(id))
                .map(|id| (id, -t * out_w * slide.dir))
                .collect();
            outgoing.extend(list);
            for (id, dx) in outgoing.into_iter().rev() {
                window_elements(az, renderer, id, origin, scale, (dx, 0.0), 1.0, &mut out);
            }
        }
        None => {
            for id in shown.iter().rev() {
                if Some(*id) == top_fullscreen {
                    continue;
                }
                window_elements(az, renderer, *id, origin, scale, (0.0, 0.0), 1.0, &mut out);
            }
        }
    }

    layer_elements(renderer, output, Layer::Bottom, scale, &mut out);
    layer_elements(renderer, output, Layer::Background, scale, &mut out);
    out
}

/// Layer surfaces (their popups are in the popup layer).
fn layer_elements<R>(renderer: &mut R, output: &Output, layer: Layer, scale: f64, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let map = layer_map_for_output(output);
    for surface in map.layers_on(layer).rev() {
        let Some(geo) = map.layer_geometry(surface) else { continue };
        let loc = geo.loc.to_physical_precise_round(scale);
        out.extend(
            render_elements_from_surface_tree(renderer, surface.wl_surface(), loc, scale, 1.0, Kind::Unspecified)
                .into_iter()
                .map(OutputElement::Surface),
        );
    }
}

/// The popup layer: every window's and layer surface's popups, above them
/// all, newest in front.
fn popup_elements<R>(az: &Aizome, renderer: &mut R, output: &Output, scale: f64, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let geo = az.output_geometry(output);
    // Windows' popups would be in the wrong place while Desks slide.
    let sliding = az.anims.desk.is_some();
    for p in az.placed_popups().into_iter().rev() {
        if sliding && matches!(p.owner, crate::input::Owner::Window(_)) {
            continue;
        }
        if !bbox_from_surface_tree(p.popup.wl_surface(), p.loc).overlaps(geo) {
            continue;
        }
        let loc = (p.loc - geo.loc).to_physical_precise_round(scale);
        out.extend(
            render_elements_from_surface_tree(renderer, p.popup.wl_surface(), loc, scale, 1.0, Kind::Unspecified)
                .into_iter()
                .map(OutputElement::Surface),
        );
    }
}

/// A window's surfaces (popups first) and its decorations, at its place
/// plus `offset` points.
#[allow(clippy::too_many_arguments)]
pub fn window_elements<R>(
    az: &mut Aizome,
    renderer: &mut R,
    id: WinId,
    origin: (f64, f64),
    scale: f64,
    offset: (f64, f64),
    alpha: f32,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let Some(w) = az.wm.get(id) else { return };
    let rect = w.rect;
    let fullscreen = w.fullscreen();
    let Some(m) = az.windows.get(&id) else { return };
    let geo = m.window.geometry();
    let loc = Point::<f64, Logical>::from((
        rect.x as f64 - geo.loc.x as f64 + offset.0 - origin.0,
        rect.y as f64 - geo.loc.y as f64 + offset.1 - origin.1,
    ));
    let loc: Point<i32, Physical> = loc.to_physical_precise_round(scale);
    out.extend(
        render_elements_from_surface_tree(renderer, m.surface(), loc, scale, alpha, Kind::Unspecified)
            .into_iter()
            .map(OutputElement::Surface),
    );
    if fullscreen {
        return;
    }
    let Some(layout) = az.decor_layout(id) else { return };
    let layout = shift_layout(&layout, offset);
    decoration_elements(az, renderer, &layout, origin, scale, alpha, out);
}

/// A layout moved by an offset (for slides).
fn shift_layout(l: &Layout, offset: (f64, f64)) -> Layout {
    if offset == (0.0, 0.0) {
        return l.clone();
    }
    let (dx, dy) = (offset.0.round() as i32, offset.1.round() as i32);
    let mut l = l.clone();
    l.content = l.content.translate(dx, dy);
    l.frame = l.frame.translate(dx, dy);
    for t in &mut l.tabs {
        t.rect = t.rect.translate(dx, dy);
        t.close = t.close.translate(dx, dy);
        t.zoom = t.zoom.translate(dx, dy);
        t.title_x += dx as f64;
    }
    l
}

/// A layout's tabs, frame and shadow, with titles and glyphs.
pub fn decoration_elements<R>(
    az: &mut Aizome,
    renderer: &mut R,
    layout: &Layout,
    origin: (f64, f64),
    scale: f64,
    alpha: f32,
    out: &mut Vec<OutputElement<R>>,
) where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let shadows = az.settings.appearance.effects.shadows();
    // Tabs sliding into a group draw at their animated place.
    let mut layout = layout.clone();
    for t in &mut layout.tabs {
        let (dx, dy) = az.anims.tab_offset(t.win);
        if dx != 0.0 || dy != 0.0 {
            let (dx, dy) = (dx.round() as i32, dy.round() as i32);
            t.rect = t.rect.translate(dx, dy);
            t.close = t.close.translate(dx, dy);
            t.zoom = t.zoom.translate(dx, dy);
            t.title_x += dx as f64;
        }
    }
    let pieces = decor::pieces(&layout, &az.palette, shadows, scale);
    // Titles (and idle tabs' glyphs) above the tab fills.
    for t in &layout.tabs {
        let (_, ink, _) = decor::tab_colors(t.look, &az.palette);
        let title = az.wm.get(t.win).map(|w| w.title.clone()).unwrap_or_default();
        let face = if t.look == TabLook::Background { Face::TabLight } else { Face::Tab };
        let style = Style::new(face, typeface::TAB, ink);
        if t.look != TabLook::Focused {
            let app_id = az.wm.get(t.win).map(|w| w.app_id.clone()).unwrap_or_default();
            let glyph_x = t.close.x as f64 - 1.0;
            let glyph_y = t.rect.y as f64 + 1.0 + (TAB_INNER - decor::GLYPH) / 2.0;
            if let Some(e) =
                crate::icons::glyph_element(az, renderer, &app_id, ink, (glyph_x, glyph_y), origin, scale, alpha)
            {
                out.push(e);
            }
        }
        let title = az.labels.fit(&title, face, typeface::TAB, t.title_max_w);
        if let Some(label) = az.labels.get(&title, style, scale, t.title_max_w + 1.0) {
            let x = t.title_x;
            let y = t.rect.y as f64 + 1.0 + ((TAB_INNER - label.h as f64) / 2.0).round();
            let loc: Point<i32, Physical> =
                Point::<f64, Logical>::from((x - origin.0, y - origin.1)).to_physical_precise_round(scale);
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc.to_f64(),
                &label.buffer,
                Some(alpha),
                None,
                Some(Size::from((label.w, label.h))),
                Kind::Unspecified,
            ) {
                out.push(OutputElement::Memory(e));
            }
        }
    }
    let cache_owner = layout.win;
    if let Some(m) = az.windows.get_mut(&cache_owner) {
        out.extend(m.decor.elements(&pieces, origin, scale, alpha).into_iter().map(OutputElement::Solid));
    }
}

/// The height inside a tab under its top border.
const TAB_INNER: f64 = (decor::TAB_H - decor::BORDER) as f64;

fn cursor_elements<R>(az: &mut Aizome, renderer: &mut R, output: &Output, scale: f64, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let geo = az.output_geometry(output);
    let pos = az.pointer_pos;
    if !geo.to_f64().contains(pos) {
        return;
    }
    let rel = pos - geo.loc.to_f64();
    let named = match (&az.interactive.cursor_override, &az.cursor_status) {
        (Some(name), _) => Some(*name),
        (None, CursorImageStatus::Hidden) => return,
        (None, CursorImageStatus::Named(icon)) => Some(icon.name()),
        (None, CursorImageStatus::Surface(_)) => None,
    };
    match named {
        Some(name) => {
            let img = az.cursor_theme.get(name, scale.ceil().max(1.0) as u32);
            let loc: Point<i32, Physical> =
                Point::<f64, Logical>::from((rel.x - img.hot.0, rel.y - img.hot.1)).to_physical_precise_round(scale);
            let size = Size::from((img.size.0.round() as i32, img.size.1.round() as i32));
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                loc.to_f64(),
                &img.buffer,
                None,
                None,
                Some(size),
                Kind::Cursor,
            ) {
                out.push(OutputElement::Memory(e));
            }
        }
        None => {
            let CursorImageStatus::Surface(surface) = &az.cursor_status else { return };
            let hotspot = with_states(surface, |states| {
                states.data_map.get::<CursorImageSurfaceData>().map(|d| d.lock().unwrap().hotspot).unwrap_or_default()
            });
            let loc: Point<i32, Physical> = (rel - hotspot.to_f64()).to_physical_precise_round(scale);
            out.extend(
                render_elements_from_surface_tree(renderer, surface, loc, scale, 1.0, Kind::Cursor)
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
    }
}

/// A window's surfaces scaled into a box (overview and switcher
/// thumbnails): the client's current buffers, scaled at composition, never
/// redrawn by the client.
pub fn thumbnail_elements<R>(
    az: &Aizome,
    renderer: &mut R,
    id: WinId,
    dst: (f64, f64, f64, f64),
    origin: (f64, f64),
    scale: f64,
    alpha: f32,
) -> Vec<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let Some(m) = az.windows.get(&id) else { return Vec::new() };
    let geo = m.window.geometry();
    if geo.size.w <= 0 || geo.size.h <= 0 {
        return Vec::new();
    }
    let k = (dst.2 / geo.size.w as f64).min(dst.3 / geo.size.h as f64);
    // Render the surface at its origin, rescale about the origin, then move
    // to the box (centred), cropped to it.
    let surface_loc: Point<i32, Physical> =
        Point::<f64, Logical>::from((-geo.loc.x as f64, -geo.loc.y as f64)).to_physical_precise_round(scale);
    let elems: Vec<WaylandSurfaceRenderElement<R>> =
        render_elements_from_surface_tree(renderer, m.surface(), surface_loc, scale, alpha, Kind::Unspecified);
    let w = geo.size.w as f64 * k;
    let h = geo.size.h as f64 * k;
    let x = dst.0 + (dst.2 - w) / 2.0;
    let y = dst.1 + (dst.3 - h) / 2.0;
    let target: Point<i32, Physical> =
        Point::<f64, Logical>::from((x - origin.0, y - origin.1)).to_physical_precise_round(scale);
    let crop: Rectangle<i32, Physical> =
        Rectangle::new(target, Size::<f64, Logical>::from((w, h)).to_physical_precise_round(scale));
    elems
        .into_iter()
        .filter_map(|e| {
            let e = RescaleRenderElement::from_element(e, Point::from((0, 0)), k);
            let e = RelocateRenderElement::from_element(
                e,
                target,
                smithay::backend::renderer::element::utils::Relocate::Relative,
            );
            CropRenderElement::from_element(e, scale, crop).map(OutputElement::Thumb)
        })
        .collect()
}

impl Aizome {
    /// A window's decoration layout (its group's, when stacked), or none
    /// for full-screen windows.
    pub fn decor_layout(&mut self, id: WinId) -> Option<Layout> {
        let w = self.wm.get(id)?;
        if w.fullscreen() {
            return None;
        }
        let (content, ssd, offset) = (w.rect, w.ssd, w.tab_offset);
        let members = self.wm.group_members(id);
        let active = members.iter().copied().find(|m| self.wm.is_group_active(*m)).unwrap_or(id);
        let focused = self.wm.focused().is_some_and(|f| members.contains(&f)) && !self.overlays.steals_focus_look();
        let titles: Vec<(WinId, String)> =
            members.iter().map(|m| (*m, self.wm.get(*m).map(|w| w.title.clone()).unwrap_or_default())).collect();
        let widths: Vec<(WinId, f64)> =
            titles.iter().map(|(m, t)| (*m, self.labels.measure(t, Face::Tab, typeface::TAB).0)).collect();
        Some(decor::layout(content, ssd, focused, &widths, active, offset))
    }
}

/// A drag-and-drop icon: at the pointer, moved by its offsets, above
/// everything but the cursor (and in screenshots, as a window would be).
fn drag_icon_elements<R>(az: &Aizome, renderer: &mut R, output: &Output, scale: f64, out: &mut Vec<OutputElement<R>>)
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    let Some(icon) = az.dnd_icon.clone() else { return };
    let geo = az.output_geometry(output);
    let at = az.pointer_pos + az.dnd_offset.to_f64();
    let bbox = smithay::desktop::utils::bbox_from_surface_tree(&icon, at.to_i32_round());
    if !bbox.overlaps(geo) {
        return;
    }
    let loc: Point<i32, Physical> = (at - geo.loc.to_f64()).to_physical_precise_round(scale);
    out.extend(
        render_elements_from_surface_tree(renderer, &icon, loc, scale, 1.0, Kind::Unspecified)
            .into_iter()
            .map(OutputElement::Surface),
    );
}
