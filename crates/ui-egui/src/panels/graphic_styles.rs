//! Graphic Styles panel: the document's styles as rendered thumbnails (Thumbnail View) or rows
//! (Small and Large List View), shown on a square or on type; a style with no fill and no stroke
//! shows the shape outlined in black and filled with white, under a red slash.
//!
//! The panel is a [`PresetBrowser`] over the styles. A click applies a style to the selection (Alt
//! adds it on top); Shift and Cmd/Ctrl select several. The style last selected is the one the
//! Symbol Styler and the Symbol Sprayer's User Defined Style apply. Double-click opens Graphic Style
//! Options. A style dragged moves in the list; dropped on another with Alt held it replaces that
//! style's look, on New Graphic Style it is duplicated, on Delete Graphic Style it is deleted as
//! the button deletes. Art or an appearance dropped on the panel becomes a new style, and dropped
//! on a style with Alt held replaces that style with its look (`graphicStyle.redefine`). The
//! Control bar's Style picker lists the same styles ([`style_picker`]). Graphic style libraries open
//! in library panels ([`GraphicStyleLibraries`]).

// PLAN: `crates/ui-egui/Cargo.toml` and the workspace manifest have no `craft` facade dependency yet; the integration owner adds it.
// PLAN: assumed paths of the framework items: `craft_ui::patterns::presets::*`,
// `craft_ui::primitives::*`, `craft_ui::contract::*`, `craft::behavior::*`.
use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use craft::behavior::{EditEvent, GesturePhase, IconId, Selection};
use craft_ui::contract::{Bind, DropAt, DropPayload, DropPlace, Entry, ListDrag, ReorderMove, SelectMode};
use craft_ui::patterns::presets::{
    PickerField, PresetBrowser, PresetDrop, PresetEvent, PresetFooterEvent, PresetFooterOptions, PresetPicker, PresetReorder, PresetRow, PresetView,
    ThumbPainter,
};
use craft_ui::primitives::{ContentPainterCx, Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, Thumbnails};
use egui::{Color32, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::style_libs::STYLE_LIBRARIES;
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{
    Appearance, AppearanceItem, DEFAULT_GRAPHIC_STYLE, Document, FillLayer, GraphicStyle, Node, NodeKind, PatternDef, StrokeLayer, StyleLibrary,
};
use vectorcraft_engine::cmd::stylelib;
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;
use vectorcraft_footer::FooterAction;
use vectorcraft_geom::{Affine, Point, shapes};

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::{alt_held, selection_len};
use crate::VectorcraftApp;
use crate::menus::Item;
use crate::widgets::PanelDrag;

/// A style of the panel: its name, which is unique in a document.
pub(crate) type StyleKey = String;

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

    /// The browser's view for this one.
    fn preset_view(self) -> PresetView {
        match self {
            View::Thumbnail => PresetView::Grid,
            View::SmallList | View::LargeList => PresetView::List,
        }
    }

    /// The thumbnail zoom that makes the browser's list rows small or large; `None` keeps the grid's own.
    fn list_zoom(self) -> Option<f64> {
        match self {
            View::Thumbnail => None,
            View::SmallList => Some(0.5),
            View::LargeList => Some(2.0),
        }
    }
}

/// What the thumbnails show the styles on (Use Square / Use Text for Previews).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Preview {
    #[default]
    Square,
    Text,
}

/// Side of the large preview shown while the pointer rests on a style.
const LARGE: f32 = 132.0;

thread_local! {
    // PLAN: the sheet drops RENDERER, but `render` and `render_object` need one renderer that outlives a call.
    static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
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
/// with. A thumbnail is made again only when its look changes.
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

/// [`look_hash`] of every style of the active document, computed once per document revision.
fn look_hashes(panel: &mut GraphicStylesPanel, app: &VectorcraftApp) -> Arc<Vec<u64>> {
    let Some(st) = app.session.active() else { return Arc::default() };
    let key = (st.uid, st.revision);
    match &panel.looks {
        Some((k, looks)) if *k == key => looks.clone(),
        _ => {
            let looks: Arc<Vec<u64>> = Arc::new(st.doc.graphic_styles.iter().map(|g| look_hash(Paints::of(&st.doc), g)).collect());
            panel.looks = Some((key, looks.clone()));
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

/// The version of the picture of a style with look `look` on `preview`: Override Character Color
/// changes only how type takes a style.
fn thumb_revision(app: &VectorcraftApp, look: u64, preview: Preview) -> Revision {
    Revision::of(&(look, preview, preview == Preview::Text && app.session.prefs.override_char_color))
}

/// The styles of a document as thumbnails: the framework asks for a picture per style and revision.
pub(crate) struct StyleThumbs<'a> {
    app: &'a VectorcraftApp,
    d: &'a Document,
    looks: &'a [u64],
    preview: Preview,
}

impl ThumbSource<StyleKey> for StyleThumbs<'_> {
    fn revision(&self, key: &StyleKey, _: &ThumbRequest) -> Revision {
        let look = self.d.graphic_style_index(key).map(|i| self.looks.get(i).copied().unwrap_or_else(|| look_hash(Paints::of(self.d), &self.d.graphic_styles[i])));
        thumb_revision(self.app, look.unwrap_or_default(), self.preview)
    }

    fn render(&self, key: &StyleKey, request: &ThumbRequest) -> ThumbRender {
        let Some(g) = self.d.graphic_style(key) else { return ThumbRender::Blank };
        render(self.app, Paints::of(self.d), g, self.preview, request.side).map_or(ThumbRender::Blank, ThumbRender::Ready)
    }
}

/// The styles of a library as thumbnails.
struct LibThumbs<'a> {
    app: &'a VectorcraftApp,
    style: &'a LibStyle,
    preview: Preview,
}

impl ThumbSource<StyleKey> for LibThumbs<'_> {
    fn revision(&self, _: &StyleKey, _: &ThumbRequest) -> Revision {
        thumb_revision(self.app, self.style.look, self.preview)
    }

    fn render(&self, _: &StyleKey, request: &ThumbRequest) -> ThumbRender {
        let paints = Paints { patterns: &self.style.lib.patterns, brushes: None };
        render(self.app, paints, self.style.style(), self.preview, request.side).map_or(ThumbRender::Blank, ThumbRender::Ready)
    }
}

/// Paint `key`'s thumbnail of `source` into `rect`; white until the picture is there.
fn paint_thumb(cx: &ContentPainterCx<'_>, rect: Rect, thumbs: &Thumbnails<StyleKey>, source: &dyn ThumbSource<StyleKey>, key: &StyleKey) {
    let ctx = cx.painter.ctx().clone();
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), ctx.pixels_per_point());
    let state = thumbs.state(&ctx, source, key, &request, Priority::VISIBLE);
    if !thumbs.paint(cx, rect, &state, &ThumbPaint::default().checker(false)) {
        cx.painter.rect_filled(rect, 0.0, Color32::WHITE);
    }
}

/// Style `g` on a copy of the first selected object (else on the preview shape), `px` pixels square
/// on white; a style with no fill and no stroke shows the object outlined in black and filled with
/// white, under a red slash, as [`render`] shows it. One texture, made again only when the
/// document, the style's look or the size changes.
fn object_preview(app: &VectorcraftApp, ctx: &egui::Context, g: &GraphicStyle, look: u64, preview: Preview, px: u32) -> Option<TextureHandle> {
    let st = app.session.active()?;
    let slot = egui::Id::new("gs-large");
    let key = egui::Id::new((st.uid, st.revision, look, px, preview, app.session.prefs.override_char_color));
    if let Some((k, tex)) = ctx.data(|d| d.get_temp::<(egui::Id, TextureHandle)>(slot))
        && k == key
    {
        return Some(tex);
    }
    let image = match st.selection.objects.first().and_then(|id| st.doc.node(*id)) {
        Some(n) => render_object(app, &st.doc, n, g, px)?,
        None => render(app, Paints::of(&st.doc), g, preview, px)?,
    };
    let tex = ctx.load_texture("gs-large", image, egui::TextureOptions::LINEAR);
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

/// The tip over a style: it large, on the selected object when there is one, with its name.
fn large_tip(thumbs: &StyleThumbs, ui: &mut Ui, key: &StyleKey) {
    let Some(i) = thumbs.d.graphic_style_index(key) else { return };
    let px = (LARGE * ui.ctx().pixels_per_point()).round() as u32;
    let look = thumbs.looks.get(i).copied().unwrap_or_default();
    let tex = object_preview(thumbs.app, ui.ctx(), &thumbs.d.graphic_styles[i], look, thumbs.preview, px);
    let (r, _) = ui.allocate_exact_size(vec2(LARGE, LARGE), Sense::hover());
    match &tex {
        Some(tex) => ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE),
        None => ui.painter().rect_filled(r, 0.0, Color32::WHITE),
    };
    ui.label(key);
}

// ---------- the panel ----------

/// The panel's rows: one preset per style, in document order.
fn style_rows(d: &Document) -> Vec<PresetRow<StyleKey>> {
    d.graphic_styles.iter().map(|g| PresetRow::item(g.name.clone(), g.name.clone(), 0)).collect()
}

/// The styles selected in the panel (those still in the document), in panel order.
fn selected(app: &VectorcraftApp) -> Vec<String> {
    let Some(st) = app.session.active() else { return Vec::new() };
    st.doc.graphic_styles.iter().filter(|g| app.ui.graphic_styles_panel.selection.contains(&g.name)).map(|g| g.name.clone()).collect()
}

/// Select `names` in the panel; the last is the lead.
fn select(app: &mut VectorcraftApp, names: Vec<String>) {
    let lead = names.last().cloned();
    app.ui.graphic_styles_panel.selection = Selection::new(names, lead.clone(), lead);
}

/// The styles a command on `key` acts on: the selection when `key` is one of them, else `key` alone.
fn targets(app: &VectorcraftApp, key: &str) -> Vec<String> {
    let sel = selected(app);
    if sel.iter().any(|s| s == key) { sel } else { vec![key.to_string()] }
}

/// The styles `names` of those that may be deleted: not the Default Graphic Style.
fn doomed(names: &[String]) -> Vec<String> {
    names.iter().filter(|n| *n != DEFAULT_GRAPHIC_STYLE).cloned().collect()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The document is shared (an `Arc`): holding it while drawing copies nothing.
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        super::empty_state(ui, "dc-graphic-styles", tl!("No document"), tl!("Open a document to see its graphic styles."));
        return;
    };
    // The panel's state is out of the app while the browser writes it and the thumbnails read the app.
    let mut panel = std::mem::take(&mut app.ui.graphic_styles_panel);
    let looks = look_hashes(&mut panel, app);
    let view = app.ui.graphic_styles_view;
    if let Some(zoom) = view.list_zoom() {
        panel.state.zoom = zoom;
    }
    let before = panel.selection.clone();
    let (alt, linked) = (alt_held(ui), app.session.selection_graphic_style().is_some());
    let rows = style_rows(&doc);
    let thumbs = StyleThumbs { app, d: &doc, looks: &looks, preview: panel.preview };
    let sel_count = before.items.len();
    let events = {
        let (state, selection, store) = (&mut panel.state, &mut panel.selection, &panel.thumbs);
        let footer = PresetFooterOptions::new()
            .new_group(false)
            .can_delete(sel_count > 0)
            .slider(view == View::Thumbnail)
            .action(FooterAction::new(IconId::new("link_2_off"), tl!("Break Link to Graphic Style").to_string()).enabled(linked));
        let in_doc = |key: &StyleKey| doc.graphic_style(key).is_some();
        // PLAN: assumed `Entry::new(value, label)` with `.enabled(bool)`, and `PresetBrowser::row_items`.
        let items = |key: &StyleKey| {
            let (is_default, multi) = (key == DEFAULT_GRAPHIC_STYLE, targets(app, key).len() > 1);
            vec![
                Entry::new("options".to_string(), tl!("Graphic Style Options…").to_string()).enabled(!is_default && !multi),
                Entry::new("duplicate".to_string(), tl!("Duplicate Graphic Style").to_string()),
                Entry::new("redefine".to_string(), tl!("Redefine Graphic Style").to_string()).enabled(selection_len(app) > 0),
                Entry::new("breakLink".to_string(), tl!("Break Link to Graphic Style").to_string()).enabled(linked_to(app, key)),
            ]
        };
        let browser = PresetBrowser::new(tl!("Graphic Styles").to_string(), Bind::bound(state), &rows)
            .select(SelectMode::Multi)
            .selection(Bind::bound(selection))
            .view_lock(view.preset_view())
            .reorder(PresetReorder::items_only())
            .footer(footer)
            .empty(tl!("Select styled art and click New Graphic Style.").to_string())
            .can_delete(|key: &StyleKey| key != DEFAULT_GRAPHIC_STYLE && in_doc(key))
            .can_rename(|key: &StyleKey| key != DEFAULT_GRAPHIC_STYLE)
            .row_items(items)
            .thumb(ThumbPainter::new(|cx, rect, info| paint_thumb(cx, rect, store, &thumbs, info.key)))
            .tip(|ui, key| large_tip(&thumbs, ui, key))
            .takes_drop(PresetDrop::new::<PanelDrag>(DropPlace::Between, |_, _| true))
            .takes_drop(PresetDrop::new::<PanelDrag>(DropPlace::Onto, |_, _| true))
            .takes_drop(PresetDrop::new::<ListDrag<StyleKey>>(DropPlace::Onto, |drag, at| at.over().is_some_and(|name| !drag.keys.contains(name))));
        browser.show(ui).events
    };
    app.ui.graphic_styles_panel = panel;
    for event in events {
        on_event(app, event, alt);
    }
    // The Symbol Styler and the Symbol Sprayer's User Defined Style take the style last selected; Shift and
    // Cmd/Ctrl selections apply nothing, so they are followed here and not through `Select`.
    let now = &app.ui.graphic_styles_panel.selection;
    if *now != before
        && let Some(lead) = now.lead.clone().filter(|l| now.contains(l))
    {
        run_status(app, "tool.setOption", json!({"tool": "symbolStyler", "values": {"style": lead}}));
    }
}

/// Is the selection linked to style `name` with its look?
fn linked_to(app: &VectorcraftApp, name: &str) -> bool {
    app.session.selection_graphic_style().is_some_and(|(g, linked)| linked && g.name == name)
}

/// Run `cmd`, reporting a failure in the status bar.
fn run_status(app: &mut VectorcraftApp, cmd: &str, params: Value) {
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}

/// Run `commands` as one undo step and return their results.
fn batch(app: &mut VectorcraftApp, label: &str, commands: Vec<Value>) -> Result<Vec<Value>, String> {
    app.run("command.batch", json!({ "label": label, "commands": commands })).map(|r| r["results"].as_array().cloned().unwrap_or_default())
}

/// Carry out what the browser reported (`alt`: Alt was held in this pass).
fn on_event(app: &mut VectorcraftApp, event: PresetEvent<StyleKey>, alt: bool) {
    match event {
        // A plain click also applies the style (with nothing selected the next object drawn takes it).
        PresetEvent::Select { key, mods } => run_status(app, "graphicStyle.apply", json!({"name": key, "add": mods.alt})),
        PresetEvent::Activate { key, .. } => run_status(app, "ui.graphicStyleOptions", json!({ "name": key })),
        PresetEvent::Rename { key, text } => run_status(app, "graphicStyle.rename", json!({ "name": key, "to": text })),
        PresetEvent::Delete(key) => {
            let names = doomed(&targets(app, &key));
            delete(app, &names, alt);
        }
        PresetEvent::Reorder(edit) => reorder(app, edit),
        PresetEvent::Dropped { at, payload, mods, .. } => dropped(app, &at, &payload, mods.alt),
        PresetEvent::Menu { target, command } => context_command(app, &target, &command),
        PresetEvent::Footer(footer) => on_footer(app, footer, alt),
        _ => {}
    }
}

/// A style or styles dragged to a new place in the list: one undo step.
fn reorder(app: &mut VectorcraftApp, edit: EditEvent<ReorderMove<StyleKey>>) {
    if !matches!(edit.phase(), Some(GesturePhase::Commit) | None) {
        return;
    }
    let Some(mv) = edit.value() else { return };
    let commands = mv.keys.iter().enumerate().map(|(i, k)| json!({ "command": "graphicStyle.move", "params": { "name": k, "to": mv.to + i } }));
    if let Err(e) = batch(app, "Move Graphic Style", commands.collect()) {
        app.status(e);
    }
}

/// A host payload dropped on the panel. Art (its first object) or an appearance becomes a new style,
/// or with Alt held over a style replaces that style with its look, unless the object is linked to it with
/// its look. A style dropped on another with Alt held replaces that style with its look.
fn dropped(app: &mut VectorcraftApp, at: &DropAt<StyleKey>, payload: &DropPayload, alt: bool) {
    // PLAN: assumed `DropPayload::get::<T>() -> Option<&T>` and `DropAt::over() -> Option<&K>`.
    let object = payload.get::<PanelDrag>().and_then(|p| match p {
        PanelDrag::Art(ids) => ids.first().copied(),
        PanelDrag::Appearance(id) => Some(*id),
        _ => None,
    });
    match (at.over(), alt, object) {
        (Some(name), true, Some(id)) => {
            if !app.session.graphic_style_of(id).is_some_and(|(g, linked)| linked && g.name == *name) {
                run_status(app, "graphicStyle.redefine", json!({ "name": name, "id": id.0 }));
            }
        }
        (_, _, Some(id)) => run_status(app, "graphicStyle.new", json!({ "id": id.0 })),
        (Some(name), true, None) => {
            if let Some(drag) = payload.get::<ListDrag<StyleKey>>().filter(|d| d.grabbed != *name) {
                run_status(app, "graphicStyle.redefine", json!({ "name": name, "from": drag.grabbed }));
            }
        }
        _ => {}
    }
}

/// A row of a style's context menu that is not the browser's own.
fn context_command(app: &mut VectorcraftApp, target: &str, command: &str) {
    match command {
        "options" => run_status(app, "ui.graphicStyleOptions", json!({ "name": target })),
        "duplicate" => duplicate(app, &targets(app, target)),
        "redefine" => run_status(app, "graphicStyle.redefine", json!({ "name": target })),
        "breakLink" => run_status(app, "graphicStyle.breakLink", json!({})),
        _ => {}
    }
}

/// A button of the footer.
fn on_footer(app: &mut VectorcraftApp, event: PresetFooterEvent<StyleKey>, alt: bool) {
    match event {
        PresetFooterEvent::New => new_style(app, alt),
        PresetFooterEvent::Delete => {
            let names = doomed(&selected(app));
            if !names.is_empty() {
                delete(app, &names, alt);
            }
        }
        PresetFooterEvent::Custom { index: 0 } => run_status(app, "graphicStyle.breakLink", json!({})),
        // PLAN: assumed `PresetFooterEvent::{DropNew, DropDelete} { keys }`: styles dropped on the footer's New and Delete (ap.AP7).
        PresetFooterEvent::DropNew { keys } => duplicate(app, &carried_by(app, &keys)),
        PresetFooterEvent::DropDelete { keys } => {
            let names = doomed(&carried_by(app, &keys));
            delete(app, &names, alt);
        }
        _ => {}
    }
}

/// The styles a drag of `keys` carries: [`carried`] over the panel's selection.
fn carried_by(app: &VectorcraftApp, keys: &[StyleKey]) -> Vec<String> {
    match (app.session.active(), keys.first()) {
        (Some(st), Some(first)) => carried(&st.doc, &selected(app), first),
        _ => Vec::new(),
    }
}

/// The Control bar's Style picker: the styles in a popover, the field showing the style the selection is
/// linked to (blank when it is not); picking one applies it to the selection, with Alt on top of its look.
pub(crate) fn style_picker(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let mut panel = std::mem::take(&mut app.ui.graphic_styles_panel);
    let looks = look_hashes(&mut panel, app);
    let mut linked: Option<StyleKey> = app.session.selection_graphic_style().filter(|(_, linked)| *linked).map(|(g, _)| g.name.clone());
    let rows = style_rows(&doc);
    let thumbs = StyleThumbs { app, d: &doc, looks: &looks, preview: panel.preview };
    let out = {
        let (state, store) = (&mut panel.picker_state, &panel.thumbs);
        // PLAN: assumed `PresetPicker::thumb(ThumbPainter)` as the browser's.
        PresetPicker::new(tl!("Graphic Styles").to_string(), Bind::bound(&mut linked), &rows, Bind::bound(state))
            .field(PickerField::Thumbnail)
            .thumb(ThumbPainter::new(|cx, rect, info| paint_thumb(cx, rect, store, &thumbs, info.key)))
            .show(ui)
    };
    app.ui.graphic_styles_panel = panel;
    if let Some(name) = out.picked {
        run_status(app, "graphicStyle.apply", json!({ "name": name, "add": out.mods.alt }));
    }
}

/// A new style from the selection, first asking for its name (`ask`) in Graphic Style Options.
fn new_style(app: &mut VectorcraftApp, ask: bool) {
    if ask {
        app.run("ui.graphicStyleOptions", json!({})).ok();
    } else {
        app.run("graphicStyle.new", json!({})).ok();
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

/// Duplicates `names` (`graphicStyle.duplicate`, each copy at the end of the list) in one undo
/// step and highlights the copies. A failure is shown in the status bar and changes nothing.
fn duplicate(app: &mut VectorcraftApp, names: &[String]) {
    let made = if let [name] = names {
        app.run("graphicStyle.duplicate", json!({ "name": name })).map(|r| vec![r])
    } else {
        let commands = names.iter().map(|n| json!({ "command": "graphicStyle.duplicate", "params": { "name": n } })).collect();
        batch(app, "Duplicate Graphic Style", commands)
    };
    match made {
        Ok(made) => select(app, made.iter().filter_map(|r| r["name"].as_str().map(str::to_string)).collect()),
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

// ---------- the panel menu ----------

/// The id of the panel-menu row `row` that is not a catalog command.
fn row_id(row: &str) -> String {
    format!("ui.graphicStyles.{row}")
}

/// The panel menu's rows, in the order of the panel menu.
// PLAN: assumed A15 shapes: `menu_rows(&VectorcraftApp) -> Vec<Item>` with ids from `row_id` leaked to `&'static str`
// by `ROWS`, `menu_states(&VectorcraftApp) -> Vec<(&'static str, bool, bool)>` as (id, enabled, checked),
// `run(&mut VectorcraftApp, &str, &Value) -> Result<Value, String>`.
const ROWS: [&str; 9] =
    ["ui.graphicStyles.new", "ui.graphicStyles.duplicate", "ui.graphicStyles.merge", "ui.graphicStyles.delete", "ui.graphicStyles.selectUnused", "ui.graphicStyles.preview", "ui.graphicStyles.options", "ui.graphicStyles.square", "ui.graphicStyles.text"];

/// The panel menu.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<Item> {
    let sel = selected(app);
    let mut items = vec![
        Item::Cmd("New Graphic Style…", ROWS[0], Value::Null),
        Item::Cmd("Duplicate Graphic Style", ROWS[1], Value::Null),
        Item::Cmd("Merge Graphic Styles", ROWS[2], Value::Null),
        Item::Cmd("Delete Graphic Style", ROWS[3], Value::Null),
        Item::Cmd("Break Link to Graphic Style", "graphicStyle.breakLink", Value::Null),
        Item::Sep,
        Item::Cmd("Select All Unused", ROWS[4], Value::Null),
        Item::Cmd("Sort by Name", "graphicStyle.sortByName", Value::Null),
        Item::Sep,
    ];
    items.extend(View::ALL.iter().map(|(v, label)| Item::Cmd(label, "ui.panelView", json!({ "panel": "graphicStyles", "view": v }))));
    items.extend([
        Item::Sep,
        Item::Cmd("Use Square for Previews", ROWS[7], Value::Null),
        Item::Cmd("Use Text for Previews", ROWS[8], Value::Null),
        Item::Cmd("Override Character Color", "graphicStyle.setOptions", json!({ "overrideCharColor": !app.session.prefs.override_char_color })),
        Item::Sep,
        Item::Cmd("Graphic Style Options…", ROWS[6], Value::Null),
        Item::Sub("Open Graphic Style Library", window_menu()),
        Item::Cmd("Save Graphic Style Library…", "ui.saveGraphicStyleLibrary", json!({ "names": sel })),
    ]);
    items
}

/// Whether each state-carrying row is enabled and checked: Merge needs two or more selected styles, Delete a
/// selected style other than the Default, Options and Duplicate exactly one.
pub(crate) fn menu_states(app: &VectorcraftApp) -> Vec<(&'static str, bool, bool)> {
    let sel = selected(app);
    let one = sel.len() == 1;
    let preview = app.ui.graphic_styles_panel.preview;
    let mut states = vec![
        (ROWS[0], selection_len(app) > 0, false),
        (ROWS[1], one, false),
        (ROWS[2], sel.len() > 1, false),
        (ROWS[3], !doomed(&sel).is_empty(), false),
        (ROWS[4], app.session.active().is_some(), false),
        (ROWS[6], one && sel[0] != DEFAULT_GRAPHIC_STYLE, false),
        (ROWS[7], true, preview == Preview::Square),
        (ROWS[8], true, preview == Preview::Text),
        ("graphicStyle.breakLink", app.session.selection_graphic_style().is_some(), false),
        ("graphicStyle.setOptions", true, app.session.prefs.override_char_color),
    ];
    states.extend(View::ALL.iter().map(|(v, _)| ("ui.panelView", true, app.ui.graphic_styles_view == *v)));
    states
}

/// Run the panel-menu row `id`.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    let sel = selected(app);
    match id {
        _ if id == ROWS[0] => new_style(app, true),
        _ if id == ROWS[1] => duplicate(app, &sel),
        // Merged in panel order, which `selected` keeps.
        _ if id == ROWS[2] => run_status(app, "ui.mergeGraphicStyles", json!({ "names": sel })),
        _ if id == ROWS[3] => delete(app, &doomed(&sel), false),
        _ if id == ROWS[4] => {
            let names: Vec<String> = serde_json::from_value(app.run("graphicStyle.unused", json!({}))?["names"].clone()).unwrap_or_default();
            select(app, names);
        }
        _ if id == ROWS[5] => {}
        _ if id == ROWS[6] => {
            if let [name] = sel.as_slice() {
                run_status(app, "ui.graphicStyleOptions", json!({ "name": name }));
            }
        }
        _ if id == ROWS[7] => app.ui.graphic_styles_panel.preview = Preview::Square,
        _ if id == ROWS[8] => app.ui.graphic_styles_panel.preview = Preview::Text,
        _ => return app.run(id, params.clone()),
    }
    Ok(Value::Null)
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
    /// Find matches a style's name as shown and as stored.
    fn rows<'a>(lib: &'a Self::Lib, id: &str, query: &str) -> Vec<Row<'a, LibStyle>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query) || Self::label(id, n).to_lowercase().contains(query);
        lib.iter().filter(|s| found(&s.style().name)).map(|s| Row { name: &s.style().name, item: Some(s) }).collect()
    }
    /// A built-in library's style in the UI language; a User Defined or loaded library's style as
    /// it is named.
    fn label<'a>(id: &str, name: &'a str) -> Cow<'a, str> {
        super::label_or_name(name, STYLE_LIBRARIES.iter().any(|b| b.id == id)).into()
    }
    // PLAN: assumed ap.AS9 `thumb(app, cx, rect, item)` in place of `draw` and `row_icons`.
    /// The style on the Graphic Styles panel's preview shape.
    fn thumb(app: &VectorcraftApp, cx: &ContentPainterCx<'_>, r: Rect, s: &LibStyle) {
        let source = LibThumbs { app, style: s, preview: app.ui.graphic_styles_panel.preview };
        paint_thumb(cx, r, &app.ui.graphic_styles_panel.thumbs, &source, &s.style().name);
    }
    fn describe(s: &LibStyle) -> String {
        describe(s.style())
    }
    /// Adds the style and applies it to the selection (Alt: on top of its appearance), as one step.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str) {
        run_status(app, "graphicStyle.addFromLibrary", json!({"library": id, "name": name, "apply": true, "add": alt_held(ui)}));
    }
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>) {
        run_status(app, "graphicStyle.addFromLibrary", json!({"library": id, "names": names}));
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        use crate::menus::menu_item;
        ui.separator();
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if menu_item(ui, tl!("Other Library…"), true, false)
            && let Err(e) = app.run("window.graphicStyleLibrary.other", json!({}))
        {
            app.status(e);
        }
        if menu_item(ui, tl!("Save Graphic Style Library…"), app.session.active().is_some(), false) {
            run_status(app, "ui.saveGraphicStyleLibrary", json!({ "names": selected(app) }));
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
    use craft_keymap::Mods;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect()
    }

    fn doc(app: &VectorcraftApp) -> Document {
        (*app.session.active().unwrap().doc).clone()
    }

    /// A `Dropped` event of `payload` over `over` with Alt as `alt`.
    // PLAN: assumed `DropAt::onto(key)`, `DropAt::end()`, `DropPayload::new(value)` and `DropOperation::Move`.
    fn dropped<P: Send + Sync + 'static>(over: Option<&str>, payload: P, alt: bool) -> PresetEvent<StyleKey> {
        let at = over.map_or_else(DropAt::end, |n| DropAt::onto(n.to_string()));
        let mods = Mods { alt, ..Mods::NONE };
        PresetEvent::Dropped { at, op: craft::behavior::DropOperation::Move, payload: DropPayload::new(payload), mods }
    }

    #[test]
    fn a_delete_event_asks_then_deletes_every_selected_style() {
        let mut app = app();
        app.session.execute("graphicStyle.new", &json!({"name": "Mine"})).unwrap();
        let before = names(&app);
        let unused: Vec<String> = serde_json::from_value(app.session.execute("graphicStyle.unused", &json!({})).unwrap()["names"].clone()).unwrap();
        assert_eq!(unused.len(), 4);
        select(&mut app, unused.clone());
        on_event(&mut app, PresetEvent::Delete(unused[0].clone()), false);
        assert_eq!(names(&app), before, "nothing goes before OK");
        // PLAN: assumed `confirm::open_question(app) -> Option<String>` and `host::confirm_by_id`.
        assert_eq!(crate::dialogs::confirm::open_question(&app).as_deref(), Some("Delete these 4 graphic styles?"));
        crate::dialogs::host::confirm_by_id(&mut app, crate::dialogs::confirm::KIND);
        assert_eq!(names(&app), ["Mine"]);
        assert!(selected(&app).is_empty());
        // Alt deletes at once.
        on_event(&mut app, PresetEvent::Delete("Mine".into()), true);
        assert!(names(&app).is_empty() && crate::dialogs::confirm::open_question(&app).is_none());
    }

    #[test]
    fn thumbnails_are_versioned_by_look_and_change_when_a_style_changes() {
        let mut app = app();
        let mut panel = GraphicStylesPanel::default();
        let request = ThumbRequest::for_cell(40.0, 1.0);
        let rev = |app: &VectorcraftApp, panel: &mut GraphicStylesPanel, i: usize, preview: Preview| {
            let d = doc(app);
            let looks = look_hashes(panel, app);
            StyleThumbs { app, d: &d, looks: &looks, preview }.revision(&d.graphic_styles[i].name, &request)
        };
        app.session.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
        app.session.execute("graphicStyle.new", &json!({"name": "Red"})).unwrap();
        let i = doc(&app).graphic_style_index("Red").unwrap();
        let (red, first) = (rev(&app, &mut panel, i, Preview::Square), rev(&app, &mut panel, 0, Preview::Square));
        // An unrelated edit keeps the revision; each preview shape has its own.
        app.session.execute("shape.ellipse", &json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
        assert_eq!(rev(&app, &mut panel, i, Preview::Square), red);
        assert_ne!(rev(&app, &mut panel, i, Preview::Text), red);
        // Redefining the style changes its look only.
        let rect = doc(&app).layers[0].children().unwrap()[0].id;
        app.session.execute("paint.setFill", &json!({"color": "#0000ff", "ids": [rect.0]})).unwrap();
        app.session.execute("graphicStyle.redefine", &json!({"name": "Red", "id": rect.0})).unwrap();
        assert_ne!(rev(&app, &mut panel, i, Preview::Square), red);
        assert_eq!(rev(&app, &mut panel, 0, Preview::Square), first);
        let d = doc(&app);
        let img = render(&app, Paints::of(&d), &d.graphic_styles[i], Preview::Square, 40).unwrap();
        assert_eq!(img.pixels[20 * img.size[0] + 20], Color32::from_rgb(0, 0, 255));
        assert_eq!(img.pixels[2 * img.size[0] + 2], Color32::WHITE);
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

    /// Merge opens the options dialog; a reorder runs `graphicStyle.move`.
    #[test]
    fn merge_asks_for_a_name_and_a_reorder_moves_styles() {
        let mut app = app();
        let n0 = names(&app);
        select(&mut app, vec![n0[1].clone(), n0[2].clone()]);
        run(&mut app, "ui.graphicStyles.merge", &Value::Null).unwrap();
        let kind = crate::dialogs::graphic_style_options::KIND;
        crate::dialogs::host::set_json_field(&mut app, kind, "name", json!("Both"));
        crate::dialogs::host::confirm_by_id(&mut app, kind);
        crate::dialogs::host::pump(&mut app);
        let d = doc(&app);
        let (a, b, both) = (d.graphic_style(&n0[1]).unwrap(), d.graphic_style(&n0[2]).unwrap(), d.graphic_style("Both").unwrap());
        assert_eq!(both.appearance.items.len(), a.appearance.items.len() + b.appearance.items.len());
        assert!(app.run("ui.mergeGraphicStyles", json!({"names": [n0[1]]})).is_err());
        // PLAN: assumed `EditEvent::set(Mixed::Same(before), after, name, None)` as the browser builds it.
        let mv = ReorderMove { key: "Both".to_string(), keys: vec!["Both".to_string()], to: 0, copy: false, parent: None };
        let edit = EditEvent::set(craft::behavior::Mixed::Same(mv.clone()), mv, std::sync::Arc::from("Move"), None);
        on_event(&mut app, PresetEvent::Reorder(edit), false);
        assert_eq!(names(&app)[0], "Both");
    }

    /// Styles dropped on the footer's New duplicate at the end, a highlighted style carrying the others.
    #[test]
    fn a_style_dropped_on_new_graphic_style_is_duplicated_at_the_end() {
        let mut app = app();
        let n = names(&app);
        select(&mut app, vec![n[3].clone(), n[0].clone()]);
        on_event(&mut app, PresetEvent::Footer(PresetFooterEvent::DropNew { keys: vec![n[3].clone()] }), false);
        assert_eq!(names(&app)[n.len()..], [format!("{} copy", n[0]), format!("{} copy", n[3])]);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(names(&app), n, "one undo step");
        // A style that is not highlighted is copied alone.
        on_event(&mut app, PresetEvent::Footer(PresetFooterEvent::DropNew { keys: vec![n[2].clone()] }), false);
        assert_eq!(names(&app)[n.len()..], [format!("{} copy", n[2])]);
    }

    /// Delete takes the drop and asks; with Alt it deletes at once.
    #[test]
    fn a_style_dropped_on_delete_graphic_style_is_deleted_as_the_button_deletes() {
        let mut app = app();
        app.run("graphicStyle.new", json!({"name": "Mine"})).unwrap();
        let n = names(&app);
        on_event(&mut app, PresetEvent::Footer(PresetFooterEvent::DropDelete { keys: vec!["Mine".into()] }), false);
        assert_eq!(names(&app), n);
        assert_eq!(crate::dialogs::confirm::open_question(&app).as_deref(), Some("Delete the graphic style “Mine”?"));
        crate::dialogs::host::confirm_by_id(&mut app, crate::dialogs::confirm::KIND);
        assert!(!names(&app).iter().any(|s| s == "Mine"));
        app.run("edit.undo", json!({})).unwrap();
        on_event(&mut app, PresetEvent::Footer(PresetFooterEvent::DropDelete { keys: vec!["Mine".into()] }), true);
        assert!(!names(&app).iter().any(|s| s == "Mine") && crate::dialogs::confirm::open_question(&app).is_none());
    }

    /// Art or an appearance dropped on a style with Alt replaces that style; not when the object already links it.
    #[test]
    fn art_or_an_appearance_dropped_on_a_style_with_alt_replaces_that_style() {
        let mut app = app();
        let rect = doc(&app).layers[0].children().unwrap()[0].id;
        app.run("paint.setFill", json!({"color": "#ff0000", "ids": [rect.0]})).unwrap();
        app.run("graphicStyle.new", json!({"name": "Red", "id": rect.0})).unwrap();
        app.run("shape.ellipse", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
        let oval = app.session.active().unwrap().selection.objects[0];
        app.run("paint.setFill", json!({"color": "#00ff00", "ids": [oval.0]})).unwrap();
        let (red, green) = (doc(&app).graphic_style("Red").unwrap().appearance.fill_paint(), doc(&app).node(oval).unwrap().appearance.fill_paint());
        let n = names(&app);
        for payload in [PanelDrag::Art(vec![oval]), PanelDrag::Appearance(oval)] {
            on_event(&mut app, dropped(Some("Red"), payload, true), false);
            let d = doc(&app);
            assert_eq!(d.graphic_style("Red").unwrap().appearance.fill_paint(), green);
            assert_eq!(d.node(oval).unwrap().extras().graphic_style, d.graphic_style("Red").map(|g| g.id));
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(doc(&app).graphic_style("Red").unwrap().appearance.fill_paint(), red);
        }
        // Without Alt it makes a new style.
        on_event(&mut app, dropped(Some("Red"), PanelDrag::Art(vec![oval]), false), false);
        assert_eq!(names(&app).len(), n.len() + 1);
        app.run("edit.undo", json!({})).unwrap();
        // The rectangle is linked to Red with its look: nothing changes.
        let steps = app.session.doc().unwrap().history.undo.len();
        on_event(&mut app, dropped(Some("Red"), PanelDrag::Art(vec![rect]), true), false);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps);
    }

    /// A style dropped on another with Alt takes that style's look; on itself it does nothing.
    #[test]
    fn a_style_dropped_on_another_with_alt_replaces_that_style() {
        let mut app = app();
        let rect = doc(&app).layers[0].children().unwrap()[0].id;
        app.run("paint.setFill", json!({"color": "#ff0000", "ids": [rect.0]})).unwrap();
        app.run("graphicStyle.new", json!({"name": "Red", "id": rect.0})).unwrap();
        app.run("paint.setFill", json!({"color": "#00ff00", "ids": [rect.0]})).unwrap();
        app.run("graphicStyle.new", json!({"name": "Green", "id": rect.0})).unwrap();
        let green = doc(&app).graphic_style("Green").unwrap().clone();
        let drag = |grabbed: &str| ListDrag::new(egui::Id::NULL, vec![grabbed.to_string()], grabbed.to_string());
        on_event(&mut app, dropped(Some("Red"), drag("Green"), true), false);
        let d = doc(&app);
        assert_eq!(d.graphic_style("Red").unwrap().appearance, green.appearance);
        assert_eq!(d.graphic_style("Green").unwrap(), &green, "Green stays as it was");
        app.run("edit.undo", json!({})).unwrap();
        let steps = app.session.doc().unwrap().history.undo.len();
        on_event(&mut app, dropped(Some("Red"), drag("Red"), true), false);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps);
    }

    /// A plain pick applies the style and the Styler follows the selection; the range is the browser's.
    #[test]
    fn a_plain_pick_applies_the_style_and_shift_or_cmd_apply_nothing() {
        let mut app = app();
        let n = names(&app);
        let linked = |app: &VectorcraftApp| app.session.selection_graphic_style().map(|(g, _)| g.name.clone());
        on_event(&mut app, PresetEvent::select(n[0].clone(), Mods::NONE), false);
        assert_eq!(linked(&app).as_ref(), Some(&n[0]));
        // The browser reports no `Select` for Shift or Cmd/Ctrl, so a multiple selection applies nothing.
        select(&mut app, n[..3].to_vec());
        assert_eq!(linked(&app).as_ref(), Some(&n[0]));
        assert_eq!(selected(&app), n[..3]);
    }

    /// The context menu's rows run their commands; Duplicate acts on the whole selection.
    #[test]
    fn the_row_menu_extras_run_their_commands() {
        let mut app = app();
        app.run("graphicStyle.new", json!({"name": "Style A"})).unwrap();
        app.run("graphicStyle.new", json!({"name": "Style B"})).unwrap();
        select(&mut app, vec!["Style A".into(), "Style B".into()]);
        on_event(&mut app, PresetEvent::Menu { target: "Style A".into(), command: "duplicate".into() }, false);
        assert!(names(&app).contains(&"Style A copy".to_string()) && names(&app).contains(&"Style B copy".to_string()));
        on_event(&mut app, PresetEvent::Menu { target: "Style A".into(), command: "options".into() }, false);
        assert_eq!(crate::dialogs::host::open_dialogs(&app), [crate::dialogs::graphic_style_options::KIND]);
    }

    /// Default's protections: no Options, no Delete; Merge needs two or more styles.
    #[test]
    fn right_click_context_menu_default_style_disables_options_and_delete() {
        let mut app = app();
        select(&mut app, vec![DEFAULT_GRAPHIC_STYLE.to_string()]);
        let state = |app: &VectorcraftApp, id: &str| menu_states(app).into_iter().find(|s| s.0 == id).map(|s| s.1);
        assert_eq!(state(&app, "ui.graphicStyles.options"), Some(false), "Options disabled on Default Graphic Style");
        assert_eq!(state(&app, "ui.graphicStyles.delete"), Some(false), "Delete disabled on Default Graphic Style");
        assert_eq!(state(&app, "ui.graphicStyles.duplicate"), Some(true));
        assert_eq!(state(&app, "ui.graphicStyles.merge"), Some(false));
        let n = names(&app);
        select(&mut app, n[..2].to_vec());
        assert_eq!(state(&app, "ui.graphicStyles.merge"), Some(true));
    }
}
