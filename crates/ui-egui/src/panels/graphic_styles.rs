//! Graphic Styles panel: the document's styles as rendered thumbnails (Thumbnail View) or rows
//! (Small and Large List View), shown on a square or on type; a style with no fill and no stroke
//! shows the shape outlined in black and filled with white, under a red slash. Clicking a style applies it to the
//! selection (Alt-click adds it on top); Shift-click adds to the panel's selection every style from
//! the last one clicked without Shift through this one; Cmd/Ctrl-click adds or removes one;
//! double-click opens Graphic Style Options. Dragging a style moves it in the list, applies it
//! to the art it is dropped on (`canvas::panel_drop`), or, dropped on New Graphic Style, adds a
//! copy of it and of the styles highlighted with it at the end of the list; art dragged off the
//! canvas or the Appearance panel's thumbnail dropped on the panel becomes a new style. Holding the right button on a style
//! shows it large, on the selected object when there is one. The Control bar's Style picker lists
//! the same tiles ([`picker`]). Graphic style libraries open in library panels
//! ([`GraphicStyleLibraries`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::style_libs::STYLE_LIBRARIES;
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, GraphicStyle, Node, NodeId, NodeKind, PatternDef, StrokeLayer, StyleLibrary};
use vectorcraft_engine::cmd::stylelib;
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;
use vectorcraft_geom::{Affine, Point, shapes};

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::{alt_held, pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::menus::Item;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

/// The panel's views (`ui.panelView {panel: "graphicStyles", view}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    #[default]
    Thumbnail,
    SmallList,
    LargeList,
}

impl View {
    const ALL: [(View, &'static str); 3] =
        [(View::Thumbnail, "Thumbnail View"), (View::SmallList, "Small List View"), (View::LargeList, "Large List View")];
    /// Row height and thumbnail side of the list views; None for the thumbnail grid.
    fn row(self) -> Option<(f32, f32)> {
        match self {
            View::Thumbnail => None,
            View::SmallList => Some((18.0, 14.0)),
            View::LargeList => Some((26.0, 20.0)),
        }
    }
}

/// What the thumbnails show the styles on (Use Square / Use Text for Previews).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Preview {
    #[default]
    Square,
    Text,
}

/// Side of a thumbnail tile (Thumbnail View and the Control bar's picker).
const TILE: f32 = 40.0;
/// Side of the large preview shown while the right button is held on a style.
const LARGE: f32 = 132.0;
const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

thread_local! {
    static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
    /// Thumbnails by (look hash, pixels, preview shape, Override Character Color on type).
    static THUMBS: crate::graphics::TexCache<HashMap<(u64, u32, Preview, bool), TextureHandle>> = crate::graphics::TexCache::default();
}

// ---------- thumbnails ----------

/// Does `g` stroke with a brush?
fn uses_brush(g: &GraphicStyle) -> bool {
    g.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.brush.is_some()))
}

/// Does `g` have no fill and no stroke? A fill or stroke row whose paint is None does not count; a
/// hidden row or one with zero opacity does. The thumbnails of such a style show the preview shape
/// with a black outline, a white fill and a red slash ([`backing`], [`slash`]).
fn paints_nothing(g: &GraphicStyle) -> bool {
    g.appearance.items.iter().all(|i| i.paint().is_none())
}

/// `g` with a white fill below its rows and a black stroke `width` wide above them, and every row
/// above the contents, so the white fill also covers type's own character colors: the shape a
/// style with no fill and no stroke is shown on, with the style's effects and transparency.
fn backing(g: &GraphicStyle, width: f64) -> GraphicStyle {
    let mut g = g.clone();
    g.appearance.contents_index = None;
    g.appearance.items.insert(0, AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::WHITE))));
    g.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), width)));
    g
}

/// The red of the slash on the thumbnail of a style with no fill and no stroke.
const SLASH: [f32; 3] = [230.0, 0.0, 0.0];

/// Draw a red slash (`/`) across the middle of `img`, a third of its side long, over what is there.
fn slash(img: &mut egui::ColorImage) {
    let [w, h] = img.size;
    let side = w.min(h) as f32;
    let (cx, cy, half) = (w as f32 / 2.0, h as f32 / 2.0, side / 6.0);
    let (a, b) = (pos2(cx - half, cy + half), pos2(cx + half, cy - half));
    let radius = (side * 0.035).max(0.9);
    let ab = b - a;
    for y in 0..h {
        for x in 0..w {
            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let t = ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
            let cover = (radius + 0.5 - (p - (a + ab * t)).length()).clamp(0.0, 1.0);
            if cover > 0.0
                && let Some(px) = img.pixels.get_mut(y * w + x)
            {
                let mix = |old: u8, new: f32| (old as f32 + (new - old as f32) * cover).round() as u8;
                *px = Color32::from_rgb(mix(px.r(), SLASH[0]), mix(px.g(), SLASH[1]), mix(px.b(), SLASH[2]));
            }
        }
    }
}

/// What a style's thumbnail paints with besides the style: the patterns and brushes of a document,
/// or the patterns of a library.
#[derive(Clone, Copy)]
struct Paints<'a> {
    patterns: &'a [PatternDef],
    brushes: Option<&'a Value>,
}

impl<'a> Paints<'a> {
    fn of(d: &'a Document) -> Self {
        Self { patterns: &d.patterns, brushes: d.unknown.get("brushes") }
    }
}

/// The look of `g` hashed: its appearance and transparency, and the patterns and brushes it paints
/// with. Thumbnails are cached by it, so a style's thumbnail is re-rendered only when its look
/// changes.
fn look_hash(paints: Paints, g: &GraphicStyle) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(&(&g.appearance, g.opacity, g.blend, g.isolate, g.knockout)).unwrap_or_default().hash(&mut h);
    for it in &g.appearance.items {
        if let Paint::Pattern { pattern, .. } = it.paint()
            && let Some(p) = paints.patterns.iter().find(|p| p.name == *pattern)
        {
            serde_json::to_vec(p).unwrap_or_default().hash(&mut h);
        }
    }
    if uses_brush(g) {
        paints.brushes.map(Value::to_string).hash(&mut h);
    }
    h.finish()
}

/// The styles of a document as the panel shows them: their [`look_hash`]es and the preview shape.
struct Shown<'a> {
    d: &'a Document,
    looks: &'a [u64],
    preview: Preview,
}

impl Shown<'_> {
    /// The look hash of style `i`.
    fn look(&self, i: usize) -> u64 {
        self.looks.get(i).copied().unwrap_or_else(|| look_hash(Paints::of(self.d), &self.d.graphic_styles[i]))
    }
    /// Draw the thumbnail of style `i` into `r`.
    fn paint(&self, app: &VectorcraftApp, ui: &Ui, r: Rect, i: usize) {
        paint_thumb(app, ui, r, Paints::of(self.d), &self.d.graphic_styles[i], self.look(i), self.preview);
    }
}

/// [`look_hash`] of every style of the active document, computed once per document revision.
fn look_hashes(ctx: &egui::Context, app: &VectorcraftApp) -> Arc<Vec<u64>> {
    let Some(st) = app.session.active() else { return Arc::default() };
    let key = (st.uid, st.revision);
    match pstate::<Option<((u64, u64), Arc<Vec<u64>>)>>(ctx, "gs-looks") {
        Some((k, looks)) if k == key => looks,
        _ => {
            let looks: Arc<Vec<u64>> = Arc::new(st.doc.graphic_styles.iter().map(|g| look_hash(Paints::of(&st.doc), g)).collect());
            set_pstate(ctx, "gs-looks", Some((key, looks.clone())));
            looks
        }
    }
}

/// Style `g` applied (as `graphicStyle.apply` applies it) to the preview shape, rendered `px`
/// pixels square on white, with `paints`. A style with no fill and no stroke ([`paints_nothing`])
/// is shown on the shape outlined in black and filled with white, under a red slash.
fn render(app: &VectorcraftApp, paints: Paints, g: &GraphicStyle, preview: Preview, px: u32) -> Option<egui::ColorImage> {
    const SIDE: f64 = 100.0;
    let empty = paints_nothing(g);
    let backed;
    let g = if empty {
        // An outline 1.5 pixels wide on images up to 150 pixels across, and one hundredth of the
        // image width on larger ones.
        backed = backing(g, (1.5 * SIDE / px.max(1) as f64).max(1.0));
        &backed
    } else {
        g
    };
    let mut doc = Document::new(SIDE, SIDE);
    doc.patterns = paints.patterns.to_vec();
    if uses_brush(g)
        && let Some(b) = paints.brushes
    {
        doc.unknown.insert("brushes".into(), b.clone());
    }
    let id = doc.alloc_id();
    let shape = match preview {
        Preview::Square => Node::path(id, shapes::rectangle(vectorcraft_geom::Rect::new(22.0, 22.0, 78.0, 78.0)), Appearance::default()),
        Preview::Text => {
            let style = CharStyle { size: 54.0, ..Default::default() };
            let mut n = Node::new(id, NodeKind::Text(Box::new(TextObject::point(Point::ZERO, "Aa", style))));
            if let Some(b) = n.geometric_bounds() {
                n.transform(Affine::translate(Point::new(SIDE / 2.0, SIDE / 2.0) - b.center()), false);
            }
            n
        }
    };
    let layer = doc.layers[0].id;
    doc.insert(Some(layer), 0, app.session.styled(&shape, g)).ok()?;
    let img = RENDERER.with(|r| r.borrow_mut().render_region(&doc, vectorcraft_geom::Rect::new(0.0, 0.0, SIDE, SIDE), px as f64 / SIDE, true));
    let mut img = crate::canvas::color_image(&img);
    if empty {
        slash(&mut img);
    }
    Some(img)
}

/// The thumbnail of style `g` (`look`: its [`look_hash`]) at `size` points.
fn thumb(
    app: &VectorcraftApp,
    ctx: &egui::Context,
    paints: Paints,
    g: &GraphicStyle,
    look: u64,
    preview: Preview,
    size: f32,
) -> Option<TextureHandle> {
    let px = (size * ctx.pixels_per_point()).round().max(8.0) as u32;
    // Override Character Color changes only how type takes a style.
    let key = (look, px, preview, preview == Preview::Text && app.session.prefs.override_char_color);
    if let Some(t) = THUMBS.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let tex = ctx.load_texture(format!("gs-{look:x}-{px}-{preview:?}"), render(app, paints, g, preview, px)?, egui::TextureOptions::LINEAR);
    THUMBS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    Some(tex)
}

/// Draw the thumbnail of style `g` (`look`: its [`look_hash`]) into `r`.
fn paint_thumb(app: &VectorcraftApp, ui: &Ui, r: Rect, paints: Paints, g: &GraphicStyle, look: u64, preview: Preview) {
    if !ui.is_rect_visible(r) {
        return;
    }
    match thumb(app, ui.ctx(), paints, g, look, preview, r.width()) {
        Some(t) => ui.painter().image(t.id(), r, UV, Color32::WHITE),
        None => ui.painter().rect_filled(r, 0.0, Color32::WHITE),
    };
}

/// Outline thumbnail `r`: in the accent colour when selected (`on`), brighter when hovered.
pub(super) fn outline(ui: &Ui, r: Rect, on: bool, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    let color = if on {
        t.accent
    } else if hovered {
        t.text
    } else {
        t.border
    };
    ui.painter().rect_stroke(r, 0.0, Stroke::new(if on { 2.0 } else { 1.0 }, color), StrokeKind::Inside);
}

/// Style `g` on a copy of the first selected object, `px` pixels square on white; a style with no
/// fill and no stroke shows the object outlined in black and filled with white, under a red slash,
/// as [`render`] shows it. One texture, re-rendered only when the document, the style's look or the
/// size changes.
fn object_preview(app: &VectorcraftApp, ctx: &egui::Context, g: &GraphicStyle, look: u64, px: u32) -> Option<TextureHandle> {
    let st = app.session.active()?;
    let n = st.doc.node(*st.selection.objects.first()?)?;
    let slot = egui::Id::unique("gs-large-object");
    let key = egui::Id::unique((st.uid, st.revision, look, px, app.session.prefs.override_char_color));
    if let Some((k, tex)) = ctx.data(|d| d.get_temp::<(egui::Id, TextureHandle)>(slot))
        && k == key
    {
        return Some(tex);
    }
    let tex = ctx.load_texture("gs-large-object", render_object(app, &st.doc, n, g, px)?, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(slot, (key, tex.clone())));
    Some(tex)
}

/// Style `g` on a copy of `n` (an object of `doc`), rendered `px` pixels square on white: the image
/// of [`object_preview`].
fn render_object(app: &VectorcraftApp, doc: &Document, n: &Node, g: &GraphicStyle, px: u32) -> Option<egui::ColorImage> {
    let empty = paints_nothing(g);
    let styled = if empty {
        // An outline about 1.5 pixels wide on the object scaled to the preview.
        let side = n.geometric_bounds().map_or(1.0, |b| b.width().max(b.height()));
        app.session.styled(n, &backing(g, (1.5 * side / px.max(1) as f64).max(1e-3)))
    } else {
        app.session.styled(n, g)
    };
    let img = RENDERER.with(|r| r.borrow_mut().render_node_thumbnail(doc, &styled, px, Some([255; 4])))?;
    let mut img = crate::canvas::color_image(&img);
    if empty {
        slash(&mut img);
    }
    Some(img)
}

/// The large preview of style `i` next to the pointer while the right button is held on it: on
/// the selected object when there is one, else on the preview shape.
fn large_preview(app: &VectorcraftApp, ctx: &egui::Context, s: &Shown, i: usize) {
    let Some(at) = ctx.pointer_hover_pos() else { return };
    let (g, look) = (&s.d.graphic_styles[i], s.look(i));
    let px = (LARGE * ctx.pixels_per_point()).round() as u32;
    let tex = object_preview(app, ctx, g, look, px).or_else(|| thumb(app, ctx, Paints::of(s.d), g, look, s.preview, LARGE));
    let t = Tokens::get(ctx);
    let area = egui::Area::new(egui::Id::unique("gs-large")).order(egui::Order::Tooltip).fixed_pos(at + vec2(14.0, 14.0));
    area.interactable(false).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(LARGE, LARGE), Sense::hover());
            match &tex {
                Some(tex) => ui.painter().image(tex.id(), r, UV, Color32::WHITE),
                None => ui.painter().rect_filled(r, 0.0, Color32::WHITE),
            };
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
            ui.label(egui::RichText::new(&g.name).size(11.5).color(t.text));
        });
    });
}

// ---------- tiles and rows ----------

/// What a click on a style does.
enum Click {
    /// A click with the modifiers held.
    Pick(String, egui::Modifiers),
    Options(String),
}

/// What the tiles or rows saw this frame (acted on after drawing, see [`act`]).
#[derive(Default)]
struct Events {
    click: Option<Click>,
    /// A style dragged to index `to` of the list.
    moved: Option<(String, usize)>,
    /// Art or an appearance dropped on the panel: a new style from this object.
    new_from: Option<NodeId>,
    /// The style the right button is held on.
    large: Option<usize>,
    /// A style is dragged over a tile or row.
    over: bool,
}

/// The click on a style's row or tile.
fn clicked(ui: &Ui, resp: &Response, name: &str) -> Option<Click> {
    if resp.double_clicked() {
        Some(Click::Options(name.to_string()))
    } else if resp.clicked() {
        Some(Click::Pick(name.to_string(), ui.input(|i| i.modifiers)))
    } else {
        None
    }
}

/// Clicks, drags and the right button on the tile or row `resp` of style `i` of `d` (`list`: rows
/// stack downwards). A drag from it carries the style ([`PanelDrag::GraphicStyle`]); a style
/// dragged over it lands before or after it by the pointer's half.
fn cell_input(ui: &Ui, resp: &Response, d: &Document, i: usize, list: bool, ev: &mut Events) {
    let name = &d.graphic_styles[i].name;
    widgets::drag_source(ui, resp, || PanelDrag::GraphicStyle(name.clone()));
    if let Some(drag) = resp.dnd_hover_payload::<PanelDrag>()
        && let PanelDrag::GraphicStyle(moving) = &*drag
    {
        ev.over = true;
        if moving == name {
            // Dropped on itself: it stays where it is.
            resp.dnd_release_payload::<PanelDrag>();
        } else {
            let r = resp.rect;
            let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
            let after = if list { at.y > r.center().y } else { at.x > r.center().x };
            let accent = Stroke::new(2.0, Tokens::get(ui.ctx()).accent);
            if list {
                let y = if after { r.bottom() } else { r.top() };
                ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], accent);
            } else {
                let x = if after { r.right() + 1.5 } else { r.left() - 1.5 };
                ui.painter().line_segment([pos2(x, r.top()), pos2(x, r.bottom())], accent);
            }
            if resp.dnd_release_payload::<PanelDrag>().is_some()
                && let Some(from) = d.graphic_style_index(moving)
            {
                let to = i + after as usize;
                ev.moved = Some((moving.clone(), if from < to { to - 1 } else { to }));
            }
        }
    }
    if resp.contains_pointer() && ui.input(|i| i.pointer.secondary_down()) {
        ev.large = Some(i);
    }
    if let Some(c) = clicked(ui, resp, name) {
        ev.click = Some(c);
    }
}

/// Drags released on the list away from the tiles: a style goes to the end, art (the first
/// object) or the Appearance panel's thumbnail becomes a new style. The list is outlined while one
/// is held over it.
fn zone_input(ui: &Ui, zone: &Response, d: &Document, ev: &mut Events) {
    let Some(drag) = zone.dnd_hover_payload::<PanelDrag>() else { return };
    let (style, source) = match &*drag {
        PanelDrag::GraphicStyle(n) => (Some(n), None),
        PanelDrag::Art(ids) => (None, ids.first().copied()),
        PanelDrag::Appearance(id) => (None, Some(*id)),
        PanelDrag::Paint { .. }
        | PanelDrag::Symbol(_)
        | PanelDrag::LibrarySymbol { .. }
        | PanelDrag::Brush { .. }
        | PanelDrag::LibraryBrush { .. }
        | PanelDrag::LibraryGraphic { .. }
        | PanelDrag::LibraryItem { .. }
        | PanelDrag::LibraryGroup { .. } => (None, None),
    };
    if style.is_none() && source.is_none() {
        return;
    }
    if !ev.over {
        ui.painter().rect_stroke(zone.rect, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
    }
    if zone.dnd_release_payload::<PanelDrag>().is_some() {
        match style {
            Some(n) => ev.moved = Some((n.clone(), d.graphic_styles.len().saturating_sub(1))),
            None => ev.new_from = source,
        }
    }
}

/// The thumbnail grid (Thumbnail View and the Control bar's picker), `sel` outlined.
fn tiles(app: &VectorcraftApp, ui: &mut Ui, s: &Shown, sel: &[String], ev: &mut Events) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for (i, g) in s.d.graphic_styles.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(TILE, TILE), Sense::click_and_drag());
            s.paint(app, ui, r, i);
            outline(ui, r, sel.contains(&g.name), resp.hovered());
            cell_input(ui, &resp.on_hover_text(&g.name), s.d, i, false, ev);
        }
    });
}

/// The list views' rows (`row`: height and thumbnail side), `sel` highlighted.
fn rows(app: &VectorcraftApp, ui: &mut Ui, s: &Shown, sel: &[String], row: (f32, f32), ev: &mut Events) {
    let t = Tokens::get(ui.ctx());
    let (h, side) = row;
    for (i, g) in s.d.graphic_styles.iter().enumerate() {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
        if sel.contains(&g.name) {
            ui.painter().rect_filled(r, 0.0, t.row_selected);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover);
        }
        let th = Rect::from_min_size(r.left_center() + vec2(4.0, -side / 2.0), vec2(side, side));
        s.paint(app, ui, th, i);
        ui.painter().rect_stroke(th, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
        ui.painter().text(pos2(th.right() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, &g.name, egui::FontId::proportional(12.0), t.text);
        cell_input(ui, &resp, s.d, i, true, ev);
    }
}

/// Run `cmd`, reporting a failure in the status bar.
fn run(app: &mut VectorcraftApp, cmd: &str, params: Value) {
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}

/// Act on what the tiles or rows saw. In the Control bar's `picker` every click applies the style
/// and closes the menu.
fn act(app: &mut VectorcraftApp, ui: &Ui, s: &Shown, ev: Events, picker: bool) {
    if let Some(i) = ev.large {
        large_preview(app, ui.ctx(), s, i);
    }
    if let Some((name, to)) = ev.moved {
        run(app, "graphicStyle.move", json!({ "name": name, "to": to }));
    }
    if let Some(id) = ev.new_from {
        run(app, "graphicStyle.new", json!({ "id": id.0 }));
    }
    match ev.click {
        Some(Click::Pick(name, _)) if picker => {
            app.run("graphicStyle.apply", json!({"name": name, "add": alt_held(ui)})).ok();
            ui.close();
        }
        Some(Click::Pick(name, m)) => {
            let order: Vec<&str> = s.d.graphic_styles.iter().map(|g| g.name.as_str()).collect();
            let sel = super::swatches::click_selection(ui, "gs-anchor", selected(ui.ctx(), s.d), &order, &name, m);
            set_pstate(ui.ctx(), "gs-sel", sel);
            // A plain click also applies the style (with nothing selected the next object drawn
            // takes it); Shift and Cmd/Ctrl clicks only select.
            if !(m.shift || m.command) {
                app.run("graphicStyle.apply", json!({"name": name, "add": alt_held(ui)})).ok();
            }
        }
        Some(Click::Options(name)) => {
            app.run("ui.graphicStyleOptions", json!({ "name": name })).ok();
        }
        None => {}
    }
}

/// The styles selected in the panel (those still in the document).
fn selected(ctx: &egui::Context, d: &Document) -> Vec<String> {
    let mut sel: Vec<String> = pstate(ctx, "gs-sel");
    sel.retain(|n| d.graphic_style(n).is_some());
    sel
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The document is shared (an `Arc`): holding it while drawing copies nothing.
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        super::empty_state(ui, "dc-graphic-styles", tl!("No document"), tl!("Open a document to see its graphic styles."));
        return;
    };
    let looks = look_hashes(ui.ctx(), app);
    let shown = Shown { d: &doc, looks: &looks, preview: pstate(ui.ctx(), "gs-preview") };
    let sel = selected(ui.ctx(), &doc);
    let view = app.ui.graphic_styles_view;
    let mut ev = Events::default();
    // In a sized panel the list fills the space above the footer (#2992), as the Artboards list
    // does; elsewhere it keeps its 220 pt.
    let fixed = widgets::panel_footer_bottom(ui).is_some();
    let list_height = widgets::panel_list_height(ui, 220.0, 2.0 + ui.spacing().item_spacing.y);
    widgets::list_box(ui, |ui| {
        let scroll = egui::ScrollArea::vertical()
            .id_salt("gs-scroll")
            .max_height(list_height)
            .min_scrolled_height(list_height.min(64.0))
            .auto_shrink([false, !fixed]);
        let out = scroll.show(ui, |ui| {
            ui.set_min_height(if fixed { list_height.min(120.0) } else { 120.0 });
            ui.set_width(ui.available_width());
            if doc.graphic_styles.is_empty() {
                super::empty_state(ui, "dc-graphic-styles", tl!("No graphic styles"), tl!("Select styled art and click New Graphic Style."));
                return;
            }
            match view.row() {
                Some(row) => rows(app, ui, &shown, &sel, row, &mut ev),
                None => tiles(app, ui, &shown, &sel, &mut ev),
            }
        });
        // The drop zone is the part of the list on screen: when the list shrinks to its rows,
        // `inner_rect` reaches past it over the footer's buttons.
        let zone = ui.interact(out.inner_rect.intersect(ui.min_rect()), ui.scope_id().with("gs-drop"), Sense::hover());
        zone_input(ui, &zone, &doc, &mut ev);
    });
    act(app, ui, &shown, ev, false);
    let sel = selected(ui.ctx(), &doc);
    let has_sel = selection_len(app) > 0;
    let linked = app.session.selection_graphic_style().is_some();
    widgets::bottom_bar(ui, |ui| {
        let open = library_panel::any_open::<GraphicStyleLibraries>(&app.ui);
        let lr = widgets::icon_button(ui, "library", GraphicStyleLibraries::MENU, open, 24.0);
        egui::Popup::menu(&lr).show(|ui| {
            ui.set_min_width(200.0);
            library_panel::library_menu::<GraphicStyleLibraries>(app, ui);
        });
        if widgets::icon_button_enabled(ui, "link-2-off", tl!("Break Link to Graphic Style"), false, linked, 24.0).clicked() {
            app.run("graphicStyle.breakLink", json!({})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        // A style dragged from the list can be dropped on it, also with no art selected: the
        // button is enabled while one is held.
        let new_enabled = has_sel || dragged_style(ui.ctx(), &doc).is_some();
        let new = widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Graphic Style (Alt-click to name it)"), false, new_enabled, 24.0);
        if new.clicked() {
            new_style(app, alt_held(ui));
        }
        drop_on_new(app, ui, &new, &sel);
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Graphic Style (Alt-click: without asking)"), false, !sel.is_empty(), 24.0)
            .clicked()
        {
            delete(app, &sel, alt_held(ui));
        }
    });
}

/// The thumbnail of style `name` in `r` (white when there is no such style): a dragged style's
/// chip and the Control bar's Style chip.
pub(crate) fn paint_style(app: &VectorcraftApp, ui: &Ui, r: Rect, name: &str) {
    match app.session.active().and_then(|st| Some((&st.doc, st.doc.graphic_style_index(name)?))) {
        Some((d, i)) => Shown { d, looks: &look_hashes(ui.ctx(), app), preview: pstate(ui.ctx(), "gs-preview") }.paint(app, ui, r, i),
        None => {
            ui.painter().rect_filled(r, 0.0, Color32::WHITE);
        }
    }
}

/// The Control bar's Style chip: the style the selection is linked to (blank when it is not).
pub(crate) fn paint_linked(app: &VectorcraftApp, ui: &Ui, r: Rect) {
    let linked = app.session.selection_graphic_style().filter(|(_, linked)| *linked).map(|(g, _)| g.name.as_str());
    paint_style(app, ui, r, linked.unwrap_or_default());
}

/// The Control bar's Style picker menu: the style tiles; clicking one applies it to the selection.
pub(crate) fn picker(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let looks = look_hashes(ui.ctx(), app);
    let shown = Shown { d: &doc, looks: &looks, preview: pstate(ui.ctx(), "gs-preview") };
    let mut ev = Events::default();
    ui.set_max_width(6.0 * (TILE + 3.0));
    tiles(app, ui, &shown, &[], &mut ev);
    act(app, ui, &shown, ev, true);
}

/// A new style from the selection, first asking for its name (`ask`) in Graphic Style Options.
fn new_style(app: &mut VectorcraftApp, ask: bool) {
    if ask {
        app.run("ui.graphicStyleOptions", json!({})).ok();
    } else {
        app.run("graphicStyle.new", json!({})).ok();
    }
}

/// The style of `d` being dragged from the panel, if one is.
fn dragged_style(ctx: &egui::Context, d: &Document) -> Option<String> {
    match egui::DragAndDrop::payload::<PanelDrag>(ctx).as_deref() {
        Some(PanelDrag::GraphicStyle(n)) if d.graphic_style(n).is_some() => Some(n.clone()),
        _ => None,
    }
}

/// The styles a style dragged from the panel carries: every highlighted style (`sel`) in panel
/// order when the dragged style `name` is one of them, else that style alone.
fn carried(d: &Document, sel: &[String], name: &str) -> Vec<String> {
    if sel.iter().any(|s| s == name) {
        d.graphic_styles.iter().filter(|g| sel.contains(&g.name)).map(|g| g.name.clone()).collect()
    } else {
        vec![name.to_string()]
    }
}

/// A style dragged onto New Graphic Style (`button`): the button is outlined while the style is
/// held over it, and on release the style, with the styles highlighted with it (`sel`), is
/// duplicated ([`duplicate`]). Other drags are left to their own targets.
fn drop_on_new(app: &mut VectorcraftApp, ui: &Ui, button: &Response, sel: &[String]) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let Some(name) = dragged_style(ui.ctx(), &doc) else { return };
    if button.dnd_hover_payload::<PanelDrag>().is_none() {
        return;
    }
    widgets::drop_target_outline(ui, button.rect);
    if button.dnd_release_payload::<PanelDrag>().is_some() {
        duplicate(app, ui.ctx(), &carried(&doc, sel, &name));
    }
}

/// Duplicates `names` (`graphicStyle.duplicate`, each copy at the end of the list) in one undo
/// step and highlights the copies. A failure is shown in the status bar and changes nothing.
fn duplicate(app: &mut VectorcraftApp, ctx: &egui::Context, names: &[String]) {
    let made = if let [name] = names {
        app.run("graphicStyle.duplicate", json!({ "name": name })).map(|r| vec![r])
    } else {
        let commands: Vec<Value> = names.iter().map(|n| json!({ "command": "graphicStyle.duplicate", "params": { "name": n } })).collect();
        app.run("command.batch", json!({ "label": "Duplicate Graphic Style", "commands": commands }))
            .map(|r| r["results"].as_array().cloned().unwrap_or_default())
    };
    match made {
        Ok(made) => set_pstate(ctx, "gs-sel", made.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect::<Vec<_>>()),
        Err(e) => app.status(e),
    }
}

/// Delete `names` after asking, or at once with `now` (Alt-click).
fn delete(app: &mut VectorcraftApp, names: &[String], now: bool) {
    let params = json!({ "names": names });
    if now {
        app.run("graphicStyle.delete", params).ok();
        return;
    }
    let message = match names {
        [n] => crate::i18n::fmt(tl!("Delete the graphic style “{name}”?"), &[("name", n)]),
        _ => crate::i18n::tn(names.len() as u64, "Delete this {n} graphic style?", "Delete these {n} graphic styles?"),
    };
    crate::dialogs::confirm::ask(app, &message, tl!("Objects using them keep their look but are no longer linked."), "graphicStyle.delete", params);
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let doc = app.session.active().map(|d| d.doc.clone());
    let sel = doc.as_ref().map(|d| selected(ui.ctx(), d)).unwrap_or_default();
    let one = (sel.len() == 1).then(|| sel[0].clone());
    let has_doc = doc.is_some();
    if menu_item(ui, tl!("New Graphic Style…"), selection_len(app) > 0, false) {
        new_style(app, true);
    }
    if menu_item(ui, tl!("Duplicate Graphic Style"), one.is_some(), false)
        && let Some(n) = &one
    {
        app.run("graphicStyle.duplicate", json!({ "name": n })).ok();
    }
    if menu_item(ui, tl!("Merge Graphic Styles"), sel.len() > 1, false)
        && let Some(d) = &doc
    {
        // Merged in panel order.
        let mut names = sel.clone();
        names.sort_by_key(|n| d.graphic_style_index(n));
        run(app, "ui.mergeGraphicStyles", json!({ "names": names }));
    }
    if menu_item(ui, tl!("Delete Graphic Style"), !sel.is_empty(), false) {
        delete(app, &sel, false);
    }
    if menu_item(ui, tl!("Break Link to Graphic Style"), app.session.selection_graphic_style().is_some(), false) {
        app.run("graphicStyle.breakLink", json!({})).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Select All Unused"), has_doc, false)
        && let Ok(r) = app.run("graphicStyle.unused", json!({}))
    {
        let names: Vec<String> = serde_json::from_value(r["names"].clone()).unwrap_or_default();
        set_pstate(ui.ctx(), "gs-sel", names);
    }
    if menu_item(ui, tl!("Sort by Name"), has_doc, false) {
        app.run("graphicStyle.sortByName", json!({})).ok();
    }
    ui.separator();
    let view = app.ui.graphic_styles_view;
    for (v, label) in View::ALL {
        if menu_item(ui, tl!(label), true, view == v)
            && let Err(e) = app.run("ui.panelView", json!({ "panel": "graphicStyles", "view": v }))
        {
            app.status(e);
        }
    }
    ui.separator();
    let preview: Preview = pstate(ui.ctx(), "gs-preview");
    for (p, label) in [(Preview::Square, tl!("Use Square for Previews")), (Preview::Text, tl!("Use Text for Previews"))] {
        if menu_item(ui, label, true, preview == p) {
            set_pstate(ui.ctx(), "gs-preview", p);
        }
    }
    let over = app.session.prefs.override_char_color;
    if menu_item(ui, tl!("Override Character Color"), true, over) {
        app.run("graphicStyle.setOptions", json!({ "overrideCharColor": !over })).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Graphic Style Options…"), one.is_some(), false)
        && let Some(n) = &one
    {
        app.run("ui.graphicStyleOptions", json!({ "name": n })).ok();
    }
    ui.menu_button(tl!("Open Graphic Style Library"), |ui| library_panel::library_menu::<GraphicStyleLibraries>(app, ui));
    if menu_item(ui, tl!("Save Graphic Style Library…"), has_doc, false) {
        run(app, "ui.saveGraphicStyleLibrary", json!({ "names": sel }));
    }
}

// ---------- libraries ----------

/// A style of a library as the library panel shows it.
pub(crate) struct LibStyle {
    lib: Arc<StyleLibrary>,
    index: usize,
    /// Its [`look_hash`].
    look: u64,
}

impl LibStyle {
    fn style(&self) -> &GraphicStyle {
        &self.lib.styles[self.index]
    }
}

/// A library's styles as the library panel shows them.
type LibStyles = Arc<Vec<LibStyle>>;

thread_local! {
    /// The libraries the library panel showed, with their styles' looks (computed once per library).
    static SHOWN_LIBS: RefCell<Vec<(Arc<StyleLibrary>, LibStyles)>> = const { RefCell::new(Vec::new()) };
}

/// `lib`'s styles with their looks.
fn shown_library(lib: Arc<StyleLibrary>) -> LibStyles {
    SHOWN_LIBS.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((_, shown)) = c.iter().find(|(l, _)| Arc::ptr_eq(l, &lib)) {
            return shown.clone();
        }
        let paints = Paints { patterns: &lib.patterns, brushes: None };
        let shown: LibStyles =
            Arc::new(lib.styles.iter().enumerate().map(|(index, g)| LibStyle { lib: lib.clone(), index, look: look_hash(paints, g) }).collect());
        if c.len() >= 16 {
            c.remove(0);
        }
        c.push((lib, shown.clone()));
        shown
    })
}

/// Style `g` in words (the library panel's list tooltips): its fills, strokes and effects, and its
/// transparency.
fn describe(g: &GraphicStyle) -> String {
    let ap = &g.appearance;
    let count = |fill: bool| match ap.items.iter().filter(|i| i.is_fill() == fill).count() {
        0 => None,
        n if fill => Some(crate::i18n::tn(n as u64, "{n} fill", "{n} fills")),
        n => Some(crate::i18n::tn(n as u64, "{n} stroke", "{n} strokes")),
    };
    let mut parts: Vec<String> = [count(true), count(false)].into_iter().flatten().collect();
    let effects = ap.effects.iter().filter_map(|e| vectorcraft_render::effects::effect_info(&e.id));
    parts.extend(effects.map(|e| e.label.trim_end_matches('…').to_string()));
    if g.opacity < 1.0 {
        parts.push(crate::i18n::fmt(tl!("{percent}% opacity"), &[("percent", &format!("{}", (g.opacity * 100.0).round()))]));
    }
    if g.blend != BlendMode::Normal {
        parts.push(tl!(g.blend.label()).to_string());
    }
    parts.join(", ")
}

/// Graphic style libraries in library panels (engine: `graphicStyle.libraries`,
/// `graphicStyle.addFromLibrary`…).
pub(crate) struct GraphicStyleLibraries;

impl LibraryKind for GraphicStyleLibraries {
    const KIND: &'static str = "graphicStyles";
    const OPEN: &'static str = "window.graphicStyleLibrary";
    const LIST: &'static str = "graphicStyle.libraries";
    const LOAD: &'static str = "graphicStyle.loadLibrary";
    const MENU: &'static str = "Graphic Style Libraries Menu";
    const ADD: &'static str = "Add to Graphic Styles";
    /// Styles need room to show their effects.
    const VIEW: super::swatches::View = super::swatches::View::LargeThumb;
    type Lib = LibStyles;
    type Item = LibStyle;

    fn list(app: &VectorcraftApp) -> Vec<LibraryRef> {
        let libs = stylelib::libraries(&app.session).into_iter();
        libs.map(|l| LibraryRef { submenu: library_panel::submenu(l.category), id: l.id, name: l.name }).collect()
    }
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)> {
        stylelib::library(&app.session, id).map(|(info, lib)| (info.name, shown_library(lib)))
    }
    fn find(app: &VectorcraftApp, key: &str) -> Option<(String, String, usize)> {
        stylelib::library(&app.session, key).map(|(info, lib)| (info.id, info.name, lib.len()))
    }
    fn path(app: &VectorcraftApp, id: &str) -> Option<String> {
        app.session.style_libraries.path(id).map(str::to_string)
    }
    fn rows<'a>(lib: &'a Self::Lib, _: &str, query: &str) -> Vec<Row<'a, LibStyle>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query);
        lib.iter().filter(|s| found(&s.style().name)).map(|s| Row { name: &s.style().name, item: Some(s) }).collect()
    }
    /// The style on the Graphic Styles panel's preview shape.
    fn draw(app: &VectorcraftApp, ui: &Ui, r: Rect, s: &LibStyle, selected: bool, hovered: bool) {
        let paints = Paints { patterns: &s.lib.patterns, brushes: None };
        paint_thumb(app, ui, r, paints, s.style(), s.look, pstate(ui.ctx(), "gs-preview"));
        outline(ui, r, selected, hovered);
    }
    fn row_icons(_: &Ui, _: Rect, _: &LibStyle) {}
    fn describe(s: &LibStyle) -> String {
        describe(s.style())
    }
    /// Adds the style and applies it to the selection (Alt: on top of its appearance), as one step.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str) {
        run(app, "graphicStyle.addFromLibrary", json!({"library": id, "name": name, "apply": true, "add": alt_held(ui)}));
    }
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>) {
        run(app, "graphicStyle.addFromLibrary", json!({"library": id, "names": names}));
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        ui.separator();
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if menu_item(ui, tl!("Other Library…"), true, false)
            && let Err(e) = app.run("window.graphicStyleLibrary.other", json!({}))
        {
            app.status(e);
        }
        let doc = app.session.active().map(|st| st.doc.clone());
        if menu_item(ui, tl!("Save Graphic Style Library…"), doc.is_some(), false) {
            let names = doc.map(|d| selected(ui.ctx(), &d)).unwrap_or_default();
            run(app, "ui.saveGraphicStyleLibrary", json!({ "names": names }));
        }
    }
}

/// `window.graphicStyleLibrary {library}`: open a graphic style library in its own panel
/// (`library` null closes every graphic style library panel).
pub(crate) fn open_library(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    library_panel::open_command::<GraphicStyleLibraries>(app, p)
}

/// `graphicStyle.loadLibrary` params, then open the library in its panel.
pub(crate) fn load_library(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let r = app.run("graphicStyle.loadLibrary", params)?;
    open_library(app, &json!({ "library": r["library"] }))
}

/// Other Library…: load the library (or document) at `path`, else one picked in an open dialog.
pub(crate) fn other_library(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    match library_panel::pick_library_file(app, path, library_panel::library_pick_filters(GraphicStyleLibraries::KIND))? {
        Some(path) => load_library(app, json!({ "path": path })),
        None => Ok(Value::Null),
    }
}

/// The id prefix of the Window → Graphic Style Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userGraphicStyleLibrary";

/// The User Defined library slot `id` (`window.userGraphicStyleLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, stylelib::libraries(&app.session))
}

/// Window → Graphic Style Libraries.
pub(crate) fn window_menu() -> Vec<Item> {
    const SLOTS: [&str; 10] = [
        "window.userGraphicStyleLibrary1",
        "window.userGraphicStyleLibrary2",
        "window.userGraphicStyleLibrary3",
        "window.userGraphicStyleLibrary4",
        "window.userGraphicStyleLibrary5",
        "window.userGraphicStyleLibrary6",
        "window.userGraphicStyleLibrary7",
        "window.userGraphicStyleLibrary8",
        "window.userGraphicStyleLibrary9",
        "window.userGraphicStyleLibrary10",
    ];
    let open =
        |b: &'static vectorcraft_doc::style_libs::BuiltinStyleLibrary| Item::Cmd(b.name, GraphicStyleLibraries::OPEN, json!({ "library": b.id }));
    let mut items: Vec<Item> = STYLE_LIBRARIES.iter().map(open).collect();
    items.extend([
        Item::Sep,
        Item::Sub("User Defined", SLOTS.iter().map(|id| Item::Cmd("User Library", id, Value::Null)).collect()),
        Item::Sep,
        Item::Cmd("Other Library…", "window.graphicStyleLibrary.other", Value::Null),
        Item::Cmd("Save Graphic Style Library…", "ui.saveGraphicStyleLibrary", Value::Null),
    ]);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// The texts `draw` paints in one headless frame with `events`.
    fn frame_with(ctx: &egui::Context, events: Vec<egui::Event>, draw: impl FnMut(&mut Ui)) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(260.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, draw);
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    fn frame(ctx: &egui::Context, draw: impl FnMut(&mut Ui)) -> Vec<String> {
        frame_with(ctx, vec![], draw)
    }

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect()
    }

    /// In a panel made taller the list fills the space above its footer instead of staying 220 pt
    /// tall with blank space below it (#2992); outside a sized panel it keeps that height.
    #[test]
    fn the_list_fills_a_tall_panel_above_its_footer() {
        let mut app = app();
        for i in 0..30 {
            app.session.execute("graphicStyle.new", &json!({"name": format!("Style {i:02}")})).unwrap();
        }
        let ctx = egui::Context::default();
        app.ui.graphic_styles_view = View::LargeList;
        // The clip rect of the first style's name: the list's visible area.
        let mut list = |bottom: Option<f32>| {
            let mut clip = None;
            for _ in 0..3 {
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(260.0, 700.0))), ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| {
                    let info = egui::UiStackInfo::default().with_tag_value(widgets::PANEL_FOOTER, bottom);
                    ui.scope_builder(egui::UiBuilder::new().ui_stack_info(info), |ui| show(&mut app, ui));
                });
                out.textures_delta.clear();
                clip = out.shapes.iter().find(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == "Style 00")).map(|c| c.clip_rect);
            }
            clip.expect("the first style's row is drawn")
        };
        let tall = list(Some(650.0));
        assert!(tall.height() > 400.0 && tall.bottom() <= 650.0 - 31.0, "the list fills the panel above its footer: {tall:?}");
        let loose = list(None);
        assert!(loose.height() <= 220.5, "unsized, the list keeps its height: {loose:?}");
    }

    #[test]
    fn list_view_names_the_styles_and_deletes_the_selected_ones_after_asking() {
        let mut app = app();
        app.session.execute("graphicStyle.new", &json!({"name": "Mine"})).unwrap();
        let ctx = egui::Context::default();
        app.ui.graphic_styles_view = View::LargeList;
        let texts = frame(&ctx, |ui| {
            show(&mut app, ui);
            menu(&mut app, ui);
        });
        // The Appearance panel names the style the object is linked to.
        let shown = frame(&ctx, |ui| super::super::appearance::show(&mut app, ui));
        assert!(shown.iter().any(|t| t == "Rectangle: Mine"), "{shown:?}");
        for t in ["Mine", "Sunshine", "Small List View", "Use Text for Previews", "Override Character Color"] {
            assert!(texts.iter().any(|x| x.ends_with(t)), "{t}: {texts:?}");
        }
        // Select All Unused picks every style but the one the rectangle is linked to.
        let unused: Vec<String> = serde_json::from_value(app.session.execute("graphicStyle.unused", &json!({})).unwrap()["names"].clone()).unwrap();
        assert_eq!(unused.len(), 4);
        set_pstate(&ctx, "gs-sel", unused.clone());
        app.ui.graphic_styles_view = View::SmallList;
        frame(&ctx, |ui| show(&mut app, ui));
        // Delete asks first; OK deletes, and the panel selection follows.
        delete(&mut app, &unused, false);
        assert_eq!(names(&app).len(), 5);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("message"), "Delete these 4 graphic styles?");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(names(&app), ["Mine"]);
        assert!(selected(&ctx, &app.session.active().unwrap().doc.clone()).is_empty());
        // Alt-click deletes at once.
        delete(&mut app, &["Mine".to_string()], true);
        assert!(names(&app).is_empty() && app.ui.dialog.is_none());
    }

    #[test]
    fn thumbnails_are_cached_by_look_and_rerendered_when_a_style_changes() {
        let mut app = app();
        let ctx = egui::Context::default();
        let tex = |app: &VectorcraftApp, i: usize, preview: Preview| {
            let d = app.session.active().unwrap().doc.clone();
            let looks = look_hashes(&ctx, app);
            thumb(app, &ctx, Paints::of(&d), &d.graphic_styles[i], looks[i], preview, TILE).unwrap().id()
        };
        app.session.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
        app.session.execute("graphicStyle.new", &json!({"name": "Red"})).unwrap();
        let i = app.session.active().unwrap().doc.graphic_style_index("Red").unwrap();
        let (red, first) = (tex(&app, i, Preview::Square), tex(&app, 0, Preview::Square));
        // Unchanged styles keep their texture, also across document edits.
        app.session.execute("shape.ellipse", &json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
        assert_eq!(tex(&app, i, Preview::Square), red);
        assert_ne!(tex(&app, i, Preview::Text), red, "each preview shape has its own");
        // Redefining the style changes its look: a new thumbnail; the others are kept.
        let rect = app.session.active().unwrap().doc.layers[0].children().unwrap()[0].id;
        app.session.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [rect.0]})).unwrap();
        app.session.execute("graphicStyle.redefine", &json!({"name": "Red", "id": rect.0})).unwrap();
        assert_ne!(tex(&app, i, Preview::Square), red);
        assert_eq!(tex(&app, 0, Preview::Square), first);
        // The rendered thumbnail shows the style on the square, on white.
        let d = app.session.active().unwrap().doc.clone();
        let img = render(&app, Paints::of(&d), &d.graphic_styles[i], Preview::Square, 40).unwrap();
        let px = |x: usize, y: usize| img.pixels[y * img.size[0] + x];
        assert_eq!(px(20, 20), Color32::from_rgb(0, 0, 255));
        assert_eq!(px(2, 2), Color32::WHITE);
    }

    /// Is `c` the slash's red?
    fn red(c: Color32) -> bool {
        c.r() > 180 && c.g() < 70 && c.b() < 70
    }

    /// Is `c` near black?
    fn dark(c: Color32) -> bool {
        c.r() < 90 && c.g() < 90 && c.b() < 90
    }

    /// A style with no fill and no stroke shows its thumbnail on the preview square or on type,
    /// outlined in black and filled with white, under a red slash; so does a style whose rows paint
    /// None, and one with only effects, whose effects still show. The large preview on the selected
    /// object shows the same.
    #[test]
    fn a_style_with_no_fill_and_no_stroke_shows_an_outlined_shape_and_a_red_slash() {
        let app = app();
        let d = app.session.active().unwrap().doc.clone();
        let image = |app: &VectorcraftApp, g: &GraphicStyle, preview: Preview| render(app, Paints::of(&d), g, preview, 40).unwrap();
        let px = |img: &egui::ColorImage, x: usize, y: usize| img.pixels[y * img.size[0] + x];
        let empty = GraphicStyle::new("Empty", Appearance::default());
        let nones = GraphicStyle::new("Nones", Appearance::basic(Paint::None, Paint::None, 1.0));
        for g in [&empty, &nones] {
            let img = image(&app, g, Preview::Square);
            // The square spans 8.8 to 31.2 pixels: a black edge, white inside, red in the middle.
            assert!(red(px(&img, 20, 20)), "{}: {:?}", g.name, px(&img, 20, 20));
            assert!((7..=10).any(|x| dark(px(&img, x, 20))), "{}: the left edge is black", g.name);
            assert_eq!(px(&img, 13, 13), Color32::WHITE, "{}", g.name);
            assert_eq!(px(&img, 2, 2), Color32::WHITE, "{}", g.name);
            assert!(!red(px(&img, 13, 13)) && !red(px(&img, 27, 27)));
        }
        // On type: the slash over the outlined characters.
        let img = image(&app, &empty, Preview::Text);
        assert!(red(px(&img, 20, 20)));
        assert!(img.pixels.iter().any(|c| dark(*c)), "the characters are outlined in black");
        // Effects still show: a drop shadow 10 points down and right of the square.
        let shadow = vectorcraft_doc::Effect { id: "stylize.dropShadow".into(), params: json!({"x": 10, "y": 10, "blur": 0}), visible: true };
        let effects = GraphicStyle::new("Shadow", Appearance { effects: vec![shadow], ..Appearance::default() });
        let img = image(&app, &effects, Preview::Square);
        assert!(red(px(&img, 20, 20)));
        assert_ne!(px(&img, 33, 33), Color32::WHITE, "the shadow shows below the square");
        // The large preview on the selected rectangle.
        let st = app.session.active().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        let img = render_object(&app, &st.doc, n, &empty, 60).unwrap();
        assert!(red(px(&img, 30, 30)));
        assert!(dark(px(&img, 1, 30)), "{:?}", px(&img, 1, 30));
        assert_eq!(px(&img, 10, 10), Color32::WHITE);
        // A style that paints keeps its own look there, without the slash.
        let blue = GraphicStyle::new("Blue", Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 1.0));
        let img = render_object(&app, &st.doc, n, &blue, 60).unwrap();
        assert_eq!(px(&img, 30, 30), Color32::from_rgb(0, 0, 255));
    }

    /// The white fill of a style with no fill and no stroke paints above type's characters even
    /// when the style puts its Characters row above some of its rows and Override Character Color
    /// is off: the type preview looks the same as for the style without that row order.
    #[test]
    fn the_white_fill_of_a_style_with_no_paint_covers_the_characters_whatever_its_contents_row() {
        let mut app = app();
        app.session.prefs.override_char_color = false;
        let d = app.session.active().unwrap().doc.clone();
        let image = |g: &GraphicStyle| render(&app, Paints::of(&d), g, Preview::Text, 40).unwrap();
        let plain = GraphicStyle::new("Nones", Appearance::basic(Paint::None, Paint::None, 1.0));
        let mut above = plain.clone();
        above.appearance.set_contents_at(1);
        assert_eq!(above.appearance.contents_index, Some(1));
        assert!(paints_nothing(&above));
        assert!(image(&above).pixels == image(&plain).pixels, "the characters' own fill shows through the white fill");
    }

    /// A style with only a fill, or only a stroke, shows its own paint; a hidden fill or one with
    /// zero opacity still counts as a fill, so its thumbnail shows no slash.
    #[test]
    fn a_fill_or_a_stroke_alone_or_a_hidden_fill_shows_no_slash() {
        let app = app();
        let d = app.session.active().unwrap().doc.clone();
        let image = |g: &GraphicStyle| render(&app, Paints::of(&d), g, Preview::Square, 40).unwrap();
        let blue = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
        let fill = GraphicStyle::new("Fill", Appearance::basic(blue.clone(), Paint::None, 1.0));
        let stroke = GraphicStyle::new("Stroke", Appearance::basic(Paint::None, Paint::solid(Color::rgb(0.0, 0.6, 0.0)), 4.0));
        let mut hidden = GraphicStyle::new("Hidden", Appearance::basic(blue.clone(), Paint::None, 1.0));
        if let Some(f) = hidden.appearance.fill_mut() {
            f.visible = false;
        }
        let mut clear = GraphicStyle::new("Clear", Appearance::basic(blue, Paint::None, 1.0));
        if let Some(f) = clear.appearance.fill_mut() {
            f.opacity = 0.0;
        }
        for g in [&fill, &stroke, &hidden, &clear] {
            assert!(!paints_nothing(g), "{}", g.name);
            assert!(!image(g).pixels.iter().any(|c| red(*c)), "{} shows no slash", g.name);
        }
        assert_eq!(image(&fill).pixels[20 * 40 + 20], Color32::from_rgb(0, 0, 255));
        assert_eq!(image(&hidden).pixels[20 * 40 + 20], Color32::WHITE);
        assert!(image(&stroke).pixels.iter().any(|c| c.g() > 100 && c.r() < 60 && c.b() < 60), "the stroke shows");
    }

    #[test]
    fn merge_asks_for_a_name_and_drops_move_styles_or_make_new_ones() {
        let mut app = app();
        let ctx = egui::Context::default();
        let n0 = names(&app);
        // Merge Graphic Styles names the new style in Graphic Style Options.
        app.run("ui.mergeGraphicStyles", json!({"names": [n0[1], n0[2]]})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Both"));
        crate::dialogs::confirm(&mut app).unwrap();
        let d = app.session.active().unwrap().doc.clone();
        let (a, b, both) = (d.graphic_style(&n0[1]).unwrap(), d.graphic_style(&n0[2]).unwrap(), d.graphic_style("Both").unwrap());
        assert_eq!(both.appearance.items.len(), a.appearance.items.len() + b.appearance.items.len());
        assert!(app.run("ui.mergeGraphicStyles", json!({"names": [n0[1]]})).is_err());

        // A style dragged onto the top half of the first row lands first (Small List View: rows
        // of 18 points from the top of the list).
        app.ui.graphic_styles_view = View::SmallList;
        frame(&ctx, |ui| show(&mut app, ui));
        let release = |at: egui::Pos2| {
            let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
            vec![egui::Event::PointerMoved(at), up]
        };
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::GraphicStyle("Both".into()));
        frame_with(&ctx, release(pos2(60.0, 6.0)), |ui| show(&mut app, ui));
        assert_eq!(names(&app)[0], "Both");
        // Art dragged off the canvas onto the panel becomes a new style.
        let rect = app.session.active().unwrap().doc.layers[0].children().unwrap()[0].id;
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![rect]));
        frame_with(&ctx, release(pos2(60.0, 116.0)), |ui| show(&mut app, ui));
        let d = app.session.active().unwrap().doc.clone();
        assert_eq!(d.graphic_styles.len(), n0.len() + 2);
        assert_eq!(d.node(rect).unwrap().extras().graphic_style, d.graphic_styles.last().map(|g| g.id));
    }

    /// A style dropped on New Graphic Style is duplicated at the end of the list, also with no art
    /// selected; a highlighted style carries the other highlighted styles with it, in one undo step.
    /// A style dropped on the Delete button is not moved in the list.
    #[test]
    fn a_style_dropped_on_new_graphic_style_is_duplicated_at_the_end() {
        let mut app = app();
        app.run("select.none", json!({})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let screen = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(260.0, 600.0)));
        // One frame of the panel with `events`: the center of the footer button labeled `label`
        // and whether it is enabled.
        let button = |app: &mut VectorcraftApp, events: Vec<egui::Event>, label: &str| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: screen, events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            let nodes = &out.platform_output.accesskit_update.as_ref().expect("accessibility tree").nodes;
            let (_, node) = nodes.iter().find(|(_, n)| n.label() == Some(label)).unwrap_or_else(|| panic!("{label}"));
            let b = node.bounds().expect("visible button");
            (pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32), !node.is_disabled())
        };
        let new_button = |app: &mut VectorcraftApp, events: Vec<egui::Event>| button(app, events, "New Graphic Style (Alt-click to name it)");
        let release = |at: egui::Pos2| {
            let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
            vec![egui::Event::PointerMoved(at), up]
        };
        let links = |app: &VectorcraftApp| {
            app.session.active().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.extras().graphic_style).collect::<Vec<_>>()
        };
        let doc = |app: &VectorcraftApp| app.session.active().unwrap().doc.clone();
        for view in [View::SmallList, View::Thumbnail] {
            app.ui.graphic_styles_view = view;
            let n = names(&app);
            let linked = links(&app);
            // With no art selected the button is off until a style is held over it.
            let (at, on) = new_button(&mut app, vec![]);
            assert!(!on, "{view:?}: off with no art selected");
            egui::DragAndDrop::set_payload(&ctx, PanelDrag::GraphicStyle(n[1].clone()));
            let (_, on) = new_button(&mut app, vec![egui::Event::PointerMoved(at)]);
            assert!(on, "{view:?}: on while a style is dragged");
            assert_eq!(names(&app), n, "{view:?}: holding it there changes nothing");
            // Released on it: a copy with the same look and a new id goes to the end, highlighted.
            egui::DragAndDrop::set_payload(&ctx, PanelDrag::GraphicStyle(n[1].clone()));
            new_button(&mut app, release(at));
            let copy = format!("{} copy", n[1]);
            assert_eq!(names(&app)[..n.len()], n[..], "{view:?}");
            assert_eq!(names(&app)[n.len()..], *std::slice::from_ref(&copy), "{view:?}");
            let d = doc(&app);
            let (src, made) = (d.graphic_style(&n[1]).unwrap(), d.graphic_style(&copy).unwrap());
            assert_eq!(made.appearance, src.appearance);
            assert_ne!(made.id, src.id);
            assert_eq!(links(&app), linked, "{view:?}: no art is linked or unlinked");
            assert_eq!(selected(&ctx, &d), std::slice::from_ref(&copy));
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(names(&app), n, "{view:?}: one undo step");
        }

        // Two highlighted styles, one of them dragged: both are copied in panel order, in one step.
        app.ui.graphic_styles_view = View::SmallList;
        let n = names(&app);
        let (at, _) = new_button(&mut app, vec![]);
        set_pstate(&ctx, "gs-sel", vec![n[3].clone(), n[0].clone()]);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::GraphicStyle(n[3].clone()));
        new_button(&mut app, release(at));
        let copies = [format!("{} copy", n[0]), format!("{} copy", n[3])];
        assert_eq!(names(&app)[n.len()..], copies);
        assert_eq!(selected(&ctx, &doc(&app)), copies);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(names(&app), n, "the group's copies go in one undo step");
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(names(&app)[n.len()..], copies);
        app.run("edit.undo", json!({})).unwrap();

        // A style that is not highlighted is copied alone.
        set_pstate(&ctx, "gs-sel", vec![n[3].clone(), n[0].clone()]);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::GraphicStyle(n[2].clone()));
        new_button(&mut app, release(at));
        assert_eq!(names(&app)[n.len()..], [format!("{} copy", n[2])]);
        app.run("edit.undo", json!({})).unwrap();

        // Other drags dropped on the button make nothing.
        let rect = doc(&app).layers[0].children().unwrap()[0].id;
        for payload in [PanelDrag::Art(vec![rect]), PanelDrag::Appearance(rect), PanelDrag::GraphicStyle("No such style".into())] {
            egui::DragAndDrop::set_payload(&ctx, payload);
            new_button(&mut app, release(at));
            assert_eq!(names(&app), n);
        }
        // A style dropped on the Delete button below the list is not moved in the list.
        let (trash, _) = button(&mut app, vec![], "Delete Graphic Style (Alt-click: without asking)");
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::GraphicStyle(n[1].clone()));
        button(&mut app, release(trash), "Delete Graphic Style (Alt-click: without asking)");
        assert_eq!(names(&app), n);
    }

    #[test]
    fn shift_click_selects_a_range_and_cmd_click_adds_or_removes_one_style() {
        let mut app = app();
        app.session.execute("graphicStyle.new", &json!({"name": "Mine"})).unwrap();
        let n = names(&app);
        assert_eq!(n.len(), 5);
        let ctx = egui::Context::default();
        app.ui.graphic_styles_view = View::SmallList;
        frame(&ctx, |ui| show(&mut app, ui));
        // A click on row `i` (Small List View: rows of 18 points, 3 apart) with `modifiers` held,
        // a second after the last one so that two clicks on a row are not a double-click.
        let mut time = 0.0;
        let mut click = |app: &mut VectorcraftApp, i: usize, modifiers: egui::Modifiers| {
            let at = pos2(60.0, 10.0 + 21.0 * i as f32);
            time += 1.0;
            for pressed in [true, false] {
                let button = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(260.0, 600.0))),
                    events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(at), button],
                    time: Some(time + if pressed { 0.0 } else { 0.05 }),
                    ..Default::default()
                };
                ctx.run_ui(raw, |ui| show(app, ui)).textures_delta.clear();
            }
            selected(&ctx, &app.session.active().unwrap().doc.clone())
        };
        let linked = |app: &VectorcraftApp| app.session.selection_graphic_style().map(|(g, _)| g.name.clone());
        // A plain click applies the style and selects it alone.
        assert_eq!(click(&mut app, 0, egui::Modifiers::NONE), [n[0].clone()]);
        assert_eq!(linked(&app).as_ref(), Some(&n[0]));
        // Shift-click selects from that style to the clicked one, and applies nothing.
        assert_eq!(click(&mut app, 3, egui::Modifiers::SHIFT), n[..4]);
        assert_eq!(linked(&app).as_ref(), Some(&n[0]));
        // Cmd/Ctrl-click removes a selected style and adds it back.
        assert_eq!(click(&mut app, 2, egui::Modifiers::COMMAND), [n[0].clone(), n[1].clone(), n[3].clone()]);
        assert_eq!(click(&mut app, 2, egui::Modifiers::COMMAND), [n[0].clone(), n[1].clone(), n[3].clone(), n[2].clone()]);
        // Shift-click extends from the style Cmd/Ctrl-clicked last.
        click(&mut app, 0, egui::Modifiers::NONE);
        click(&mut app, 2, egui::Modifiers::COMMAND);
        assert_eq!(click(&mut app, 4, egui::Modifiers::SHIFT), [n[0].clone(), n[2].clone(), n[3].clone(), n[4].clone()]);
        assert_eq!(linked(&app).as_ref(), Some(&n[0]));
        // A plain click applies the next style.
        assert_eq!(click(&mut app, 4, egui::Modifiers::NONE), [n[4].clone()]);
        assert_eq!(linked(&app).as_ref(), Some(&n[4]));
    }

    #[test]
    fn the_right_button_shows_a_large_preview_and_the_picker_applies_styles() {
        let mut app = app();
        let ctx = egui::Context::default();
        let first = names(&app)[0].clone();
        let button = |button: egui::PointerButton, pressed: bool| {
            let at = pos2(20.0, 20.0);
            vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() }]
        };
        frame(&ctx, |ui| show(&mut app, ui));
        // Held on the first tile: the large preview (on the selected rectangle) names the style
        // (a new area shows from its second frame).
        frame_with(&ctx, button(egui::PointerButton::Secondary, true), |ui| show(&mut app, ui));
        let texts = frame(&ctx, |ui| show(&mut app, ui));
        assert!(texts.contains(&first), "{texts:?}");
        let texts = frame_with(&ctx, button(egui::PointerButton::Secondary, false), |ui| show(&mut app, ui));
        assert!(!texts.contains(&first));
        // The Control bar's picker: a click on a tile applies that style to the selection.
        let name = names(&app)[2].clone();
        let at = pos2(2.0 * (TILE + 3.0) + 20.0, 20.0);
        let click = |pressed: bool| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
            ]
        };
        let ctx = egui::Context::default();
        frame(&ctx, |ui| picker(&mut app, ui));
        frame_with(&ctx, click(true), |ui| picker(&mut app, ui));
        frame_with(&ctx, click(false), |ui| picker(&mut app, ui));
        assert_eq!(app.session.selection_graphic_style().map(|(g, linked)| (g.name.clone(), linked)), Some((name, true)));
    }
}
