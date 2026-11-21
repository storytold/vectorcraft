//! Effect → Effect Gallery: browse raster filters by category, build a stack, and
//! apply it as separate appearance effects in one undo step.
//!
//! Each filter has a thumbnail, and the dialog has its own preview, as the
//! [Filter Gallery](https://helpx.adobe.com/photoshop/desktop/effects-filters/get-started-with-filters/filter-gallery.html)
//! describes. Both are an original drawing made for VectorCraft (a small landscape), passed through
//! the filter. The thumbnails use each filter's defaults. The preview uses the visible layers of the
//! stack. The canvas preview and the one-undo OK are the same batch: [`pump`] previews it through
//! the dialog's [`LivePreview`] and OK commits it there.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::primitives::textures::{ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::requests::PathRequest;
use craft::ui::runtime::words::CommonWords;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::contract::Revision;
use craft::ui::patterns::form_dialogs::FieldValue;
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_effects::{EffectInfo, PIXEL_EFFECTS, PixelSpace};
use vectorcraft_geom::{Affine, Point};
use vectorcraft_imaging::ui::filter_gallery::{
    EffectChange, EffectLayer, EffectParameters, EffectStack, FilterCatalog, FilterCategory, FilterGalleryWords as _, FilterIndex,
    FilterGallerySession, FilterTree, FilterTreeLayout, GalleryFilter, GalleryThumb, LayerId, SessionField,
};
use craft::ui::runtime::egui::widgets::Checkbox;
use craft::ui::runtime::egui::widgets::Picture;

use super::{EffectCx, effect, effect_params, file_asks, json_of};
use super::file_asks::Owner;
use crate::VectorcraftApp;

// PLAN: the crate paths of the framework items (`craft::ui::…`, `vectorcraft_imaging::ui::filter_gallery`), `EffectCx::new(id)`,
// `effect_params(&EffectCx, &Map) -> Vec<Param>` (ef.C3, ef.C4), `json_of(&[Param], &DialogFields) -> Value` (wg.S12),
// `file_asks::ask(app, &FilePick, Owner::Free, FnOnce(&mut VectorcraftApp, String) + Send)` (fl.A3),
// `LivePreview::{run, commit}` (wg.A20), `app.dialogs: DialogHost` and `app.effect_dialogs.gallery: Option<Open>` (ef.C1, hub-w2) are
// assumed: none is in this tree yet.

/// Thumbnail of one filter, in pixels at most. The drawing is original; the filter is this app's.
const THUMB: usize = 36;
/// In-dialog preview, in pixels, drawn at twice this size.
const PREVIEW_W: usize = 120;
const PREVIEW_H: usize = 84;

/// The command the preview and OK run: the whole stack as one undo step.
const COMMAND: &str = "command.batch";

/// The dialog's key: its id and the name automation reaches it by.
const KEY: &str = "effectGallery";

/// Raster effect submenus in the gallery (the six raster filter folders it holds).
const CATEGORIES: [(&str, &[&str]); 6] = [
    ("Artistic", &["Effect", "Artistic"]),
    ("Brush Strokes", &["Effect", "Brush Strokes"]),
    ("Distort", &["Effect", "Distort"]),
    ("Sketch", &["Effect", "Sketch"]),
    ("Stylize", &["Effect", "Stylize"]),
    ("Texture", &["Effect", "Texture"]),
];

/// One layer as the engine's batch applies it.
#[derive(Clone)]
struct StackEntry {
    effect: String,
    visible: bool,
    params: Map<String, Value>,
}

/// What the app keeps of the open gallery: the dialog's id and the canvas preview it runs.
pub(crate) struct Open {
    id: DialogId,
    preview: LivePreview,
}

/// Opens the gallery on the current selection.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let _ = p;
    if app.session.active().is_none_or(|s| s.selection.is_empty()) {
        return Err(tl!("Select artwork to apply effects").into());
    }
    let dialog = EffectGalleryDialog::new();
    let id = dialog.id();
    app.dialogs.open(dialog);
    app.effect_dialogs.gallery = Some(Open { id, preview: LivePreview::default() });
    Ok(Value::Null)
}

/// Runs the gallery's part of one frame: shows the stack on the canvas, files the held OK's commit and starts
/// a Browse press's file dialog. A gallery that closed takes its canvas preview away.
pub fn pump(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some(mut open) = app.effect_dialogs.gallery.take() else { return };
    let Some((on, visible, held, asked)) = app.dialogs.get_mut_by_id::<EffectGalleryDialog>(open.id).map(|dialog| {
        let visible = dialog.entries(true);
        (dialog.preview_on && !visible.is_empty(), visible, dialog.held.take(), dialog.asked.take())
    }) else {
        // Cancel leaves the preview to remove; after OK the batch is committed and nothing is left.
        open.preview.run(app, false, COMMAND, &Value::Null);
        return;
    };
    open.preview.run(app, on, COMMAND, &stack_batch_params(&visible));
    if let Some(layers) = held {
        let all = entries_of(&layers, &app_index(app, open.id), false);
        let answer = open.preview.commit(app, COMMAND, stack_batch_params(&all)).map(|_| ()).map_err(Label::from);
        if let Some(dialog) = app.dialogs.get_mut_by_id::<EffectGalleryDialog>(open.id) {
            dialog.answer_ok(answer);
        }
    }
    if let Some(layer) = asked {
        ask_texture(app, open.id, layer);
    }
    app.effect_dialogs.gallery = Some(open);
}

/// The filter index of the open gallery `id`; empty once it closed.
fn app_index(app: &mut VectorcraftApp, id: DialogId) -> FilterIndex {
    app.dialogs.get_mut_by_id::<EffectGalleryDialog>(id).map(|dialog| dialog.index.clone()).unwrap_or_default()
}

/// Asks for the image Load Texture reads into the layer `layer`. The answer finds the layer by its own id, wherever
/// it moved, and is dropped for a layer deleted since or a gallery that closed (a layer id is never used twice).
fn ask_texture(app: &mut VectorcraftApp, id: DialogId, layer: LayerId) {
    use vectorcraft_engine::cmd::fileio;
    let filters = crate::FilePick::filters(fileio::FORMATS.iter().filter(|f| f.read && f.raster).map(|f| (f.label, fileio::read_extensions(f))));
    let pick = crate::FilePick { filters, ..Default::default() };
    file_asks::ask(app, &pick, Owner::Free, move |app, path| {
        // The button is off where files can't be read.
        let loaded = app.services.read.as_ref().map(|read| read(&path).and_then(|bytes| effect::texture_fields(&bytes)));
        match loaded {
            Some(Ok(fields)) => {
                if let Some(dialog) = app.dialogs.get_mut_by_id::<EffectGalleryDialog>(id) {
                    dialog.answer_texture(layer, fields);
                }
            }
            Some(Err(e)) => app.status(e),
            None => {}
        }
    });
}

fn effects_in_category(cat: usize) -> Vec<EffectInfo> {
    let path = CATEGORIES.get(cat).map(|(_, p)| *p).unwrap_or(CATEGORIES[0].1);
    let mut out: Vec<EffectInfo> =
        vectorcraft_effects::effect_catalog().into_iter().filter(|e| e.menu == path && PIXEL_EFFECTS.contains(&e.id)).collect();
    out.sort_by(|a, b| a.label.cmp(b.label));
    out
}

fn default_params(id: &str) -> Map<String, Value> {
    match vectorcraft_effects::effect_info(id).map(|e| e.defaults) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// The raster filters of the gallery: the six folders and each filter's parameters at its defaults.
///
/// Built once when the gallery opens: the framework asks the catalog for every filter whenever it builds an index.
struct Catalog {
    categories: Vec<FilterCategory>,
    filters: Vec<GalleryFilter>,
}

impl Catalog {
    fn new() -> Self {
        let mut categories = Vec::new();
        let mut filters = Vec::new();
        for (at, (name, _)) in CATEGORIES.iter().enumerate() {
            let ids: Vec<&str> = effects_in_category(at).into_iter().map(|e| e.id).collect();
            if ids.is_empty() {
                continue;
            }
            for id in &ids {
                // The rows that depend on a layer's values (Loaded Texture) start from no loaded texture.
                let cx = EffectCx::new(id);
                filters.push(GalleryFilter::new(*id, Label::from(effect_label(id)), *name, effect_params(&cx, &default_params(id))));
            }
            categories.push(FilterCategory::new(*name, Label::from(tl!(name).to_string()), ids.into_iter().map(str::to_owned).collect()));
        }
        Catalog { categories, filters }
    }
}

impl FilterCatalog for Catalog {
    fn categories(&self) -> Vec<FilterCategory> {
        self.categories.clone()
    }

    fn filter(&self, key: &str) -> Option<GalleryFilter> {
        self.filters.iter().find(|filter| filter.key == key).cloned()
    }
}

/// The filter thumbnails: each filter at its defaults over the gallery's drawing.
struct Thumbs;

impl ThumbSource<GalleryThumb> for Thumbs {
    fn revision(&self, _: &GalleryThumb, _: &ThumbRequest) -> Revision {
        // The filter at its defaults over the fixed drawing never changes.
        Revision::default()
    }

    fn render(&self, key: &GalleryThumb, request: &ThumbRequest) -> ThumbRender {
        let side = (request.side as usize).clamp(1, THUMB);
        ThumbRender::Ready(color_image(&filtered(&key.filter, &default_params(&key.filter), side, side), side, side))
    }
}

/// The layers as the engine's batch takes them, bottom first; with `visible_only` the hidden layers are left out.
/// A layer whose filter the catalog lacks is left out too.
fn entries_of(layers: &[EffectLayer], index: &FilterIndex, visible_only: bool) -> Vec<StackEntry> {
    layers
        .iter()
        .filter(|layer| layer.visible || !visible_only)
        .filter_map(|layer| {
            let filter = index.filter(&layer.filter)?;
            let Value::Object(params) = vectorcraft_effects::upgrade_params(&layer.filter, &json_of(&filter.params, &layer.fields)) else {
                return None;
            };
            Some(StackEntry { effect: layer.filter.clone(), visible: layer.visible, params })
        })
        .collect()
}

/// The Effect Gallery on the dialog host: the framework's filter tree, settings and effect-layer stack over one
/// [`FilterGallerySession`], with a preview of the visible layers and OK held for [`pump`] to commit.
pub struct EffectGalleryDialog {
    id: DialogId,
    open: bool,
    catalog: Catalog,
    index: FilterIndex,
    session: FilterGallerySession,
    selected: usize,
    preview_on: bool,
    /// The layers OK filed; the app commits them and answers.
    held: HeldAnswer<Vec<EffectLayer>>,
    /// A Browse press waiting for the app's file dialog, with the layer it belongs to.
    asked: Option<LayerId>,
}

impl EffectGalleryDialog {
    /// A gallery on one layer of the first Artistic filter, previewing.
    pub fn new() -> Self {
        let catalog = Catalog::new();
        let first = catalog.categories.first().and_then(|category| category.filters.first()).cloned().unwrap_or_default();
        let index = FilterIndex::of(&catalog);
        let session = FilterGallerySession::new(&first, &catalog);
        EffectGalleryDialog { id: dialog_id(KEY), open: true, catalog, index, session, selected: 0, preview_on: true, held: HeldAnswer::new(), asked: None }
    }

    /// The layers as the batch takes them.
    fn entries(&self, visible_only: bool) -> Vec<StackEntry> {
        entries_of(&self.session.effects, &self.index, visible_only)
    }

    /// The app's answer to the held OK: `Ok` closes the dialog in its next pass, `Err` shows the reason and enables OK.
    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// Writes the picked texture into the layer `layer`, wherever it is now. A deleted layer takes nothing.
    fn answer_texture(&mut self, layer: LayerId, fields: Map<String, Value>) {
        let Some(at) = self.session.position(layer) else { return };
        let Some(mut values) = self.session.effects.get(at).map(|effect| effect.fields.clone()) else { return };
        for (key, value) in fields {
            if let Value::String(text) = value {
                values.set(key, FieldValue::Text(text));
            }
        }
        self.session.apply(EffectChange::with_fields(at, values), &self.catalog, &mut self.selected);
    }

    /// Applies `change` to the stack.
    fn apply(&mut self, change: EffectChange) {
        self.session.apply(change, &self.catalog, &mut self.selected);
    }
}

impl Default for EffectGalleryDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl StackDialog for EffectGalleryDialog {
    type Result = Vec<EffectLayer>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<CommonWords>();
        let EffectGalleryDialog { open, index, session, selected, preview_on, held, asked, .. } = self;
        // The app accepted the held OK: this pass closes the dialog with it. A measuring run closes nothing.
        let finish = held.is_accepted() && !craft::ui::runtime::measure::active(ui.ctx());
        let refusal = held.refusal().map(|(reason, refused)| (reason.clone(), refused.clone()));
        let current = (*selected).min(session.effects.len().saturating_sub(1));
        let visible = entries_of(&session.effects, index, true);
        let key = {
            let mut hasher = DefaultHasher::new();
            serde_json::to_string(&stack_batch_params(&visible)).unwrap_or_default().hash(&mut hasher);
            hasher.finish()
        };
        let thumbs = Thumbnails::<GalleryThumb>::in_context(ui.ctx(), "effectGallery.thumbs", ThumbnailOptions::new());
        // The body may run twice in a pass: what it reports is the last run's.
        let mut changes: Vec<EffectChange> = Vec::new();
        let mut picked_layer = None;
        let mut pressed = None;
        let effects = &session.effects;
        let mut on = *preview_on;
        let buttons = [
            FooterButton::new(words.ok.clone(), DialogRole::Accept).stays_open(true).enabled(!held.is_waiting()),
            FooterButton::new(words.cancel.clone(), DialogRole::Cancel),
        ];
        let dialog = Dialog::new(tl!("Effect Gallery"))
            .opened(Bind::bound(open))
            .buttons(buttons)
            .close_on_enter(true)
            .footer(|ui, _| {
                Checkbox::new(Bind::bound(&mut on)).label(tl!("Preview")).show(ui);
            });
        let shown = dialog.show(ui, |ui, cx| {
            changes.clear();
            picked_layer = None;
            pressed = None;
            if finish {
                cx.request_close_as(CloseReason::Activated);
            }
            let Some(layer) = effects.get(current) else { return };
            let image = || color_image(&preview_rgba(&visible, PREVIEW_W, PREVIEW_H), PREVIEW_W, PREVIEW_H);
            Stack::new().show(ui, |ui, _| {
                Picture::new(tl!("Preview"), key, &image).aspect(ThumbAspect::new(10, 7)).show(ui);
                Row::new().show(ui, |ui, _| {
                    let tree = FilterTree::new(index, &layer.filter, &thumbs, &Thumbs).layout(FilterTreeLayout::Lists).show(ui);
                    if let Some(filter) = tree.picked {
                        changes.push(EffectChange::set_filter(current, filter));
                    }
                    Stack::new().show(ui, |ui, _| {
                        let params = EffectParameters::new(current, layer, index).show(ui);
                        changes.extend(params.change);
                        if params.path.is_some() {
                            pressed = Some(layer.id);
                        }
                        let stack = EffectStack::new(effects, current, index).show(ui);
                        picked_layer = stack.selected;
                        changes.extend(stack.change);
                    });
                });
                if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| refused == effects) {
                    Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
                }
            });
        });
        *preview_on = on;
        // A pass applies the changes its own input made; a second pass of the frame reports none.
        if let Some(at) = picked_layer {
            *selected = at;
        }
        for change in changes {
            session.apply(change, &self.catalog, selected);
        }
        if asked.is_none() {
            *asked = pressed;
        }
        if held.refusal().is_some_and(|(_, refused)| refused != &session.effects) {
            held.clear_refusal();
        }
        // OK files the layers for the app, which commits them before the dialog closes.
        if shown.output.picked == Some(0) && !held.is_waiting() {
            held.file(session.effects.clone());
        }
        if finish && shown.output.closed == Some(CloseReason::Activated) {
            return Some(held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        shown.output.closed.map(|_| DialogResult::Cancel)
    }

    /// OK as the button: files the layers and answers none; once the app accepted them, answers them.
    fn confirm(&mut self, _: &Context) -> Option<Self::Result> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if !self.held.is_waiting() {
            self.held.file(self.session.effects.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// `effects`, `select` (the session's) and `preview`.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.session.json_fields(self.selected);
        fields.as_object_mut()?.insert("preview".to_owned(), Value::Bool(self.preview_on));
        Some(fields)
    }

    /// Sets `preview` here and every other field through the session.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        if field == "preview" {
            self.preview_on = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            return Ok(());
        }
        match self.session.field_change(&self.catalog, self.selected, field, &value)? {
            Some(SessionField::Change(change)) => self.apply(change),
            Some(SessionField::Select(at)) => self.selected = at.min(self.session.effects.len().saturating_sub(1)),
            None => {}
        }
        Ok(())
    }
}

fn stack_batch_commands(stack: &[StackEntry]) -> Vec<Value> {
    let mut commands = Vec::with_capacity(stack.len() * 2);
    for (i, entry) in stack.iter().enumerate() {
        let params = Value::Object(entry.params.clone());
        commands.push(json!({ "command": "effect.apply", "params": { "effect": entry.effect, "params": params } }));
        if !entry.visible {
            commands.push(json!({ "command": "effect.setParams", "params": { "index": i, "visible": false } }));
        }
    }
    commands
}

fn stack_batch_params(stack: &[StackEntry]) -> Value {
    json!({ "label": "Effect Gallery", "commands": stack_batch_commands(stack) })
}

fn effect_label(id: &str) -> String {
    vectorcraft_effects::effect_info(id).map(|e| tl!(e.label).trim_end_matches('…').to_string()).unwrap_or_else(|| id.to_string())
}

/// Where the preview rasters sit: one document point per pixel, origin at the top-left.
fn preview_space(w: usize, h: usize) -> PixelSpace {
    let (wf, hf) = (w as f64, h as f64);
    PixelSpace {
        to_doc: Affine::IDENTITY,
        px: 1.0,
        center: Point::new(wf / 2.0, hf / 2.0),
        size: vectorcraft_geom::Vec2::new(wf, hf),
        channels: vectorcraft_effects::pixel::Channels::Rgb,
        line: 1.0,
        extent: 0.5 * wf.hypot(hf),
    }
}

fn byte(v: f32) -> u8 {
    if !v.is_finite() {
        return 0;
    }
    v.round().clamp(0.0, 255.0) as u8
}

/// An original landscape for the gallery (sky, sun, hills, a house, a river). Not a copied picture.
fn sample_drawing(w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(4)];
    if w == 0 || h == 0 || out.len() != w * h * 4 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let xf = x as f32 / w as f32;
            let yf = y as f32 / h as f32;
            let mut rgb = [90.0 + 90.0 * (1.0 - yf), 150.0 + 50.0 * (1.0 - yf), 200.0 + 40.0 * yf];
            let (dx, dy) = (xf - 0.78, yf - 0.22);
            if dx * dx + dy * dy < 0.012 {
                rgb = [252.0, 206.0, 64.0];
            }
            let hill = 0.58 + 0.08 * (xf * std::f32::consts::TAU).sin();
            if yf > hill {
                rgb = [46.0, 110.0 + 40.0 * (1.0 - (yf - hill).clamp(0.0, 1.0)), 52.0];
            }
            if (0.16..0.40).contains(&xf) && (0.46..0.70).contains(&yf) {
                rgb = [196.0, 64.0, 48.0];
            }
            let roof = 0.46 - (xf - 0.28).abs() * 1.15;
            if (0.12..0.44).contains(&xf) && yf > roof && yf < 0.48 {
                rgb = [92.0, 48.0, 36.0];
            }
            if (0.22..0.32).contains(&xf) && (0.52..0.62).contains(&yf) {
                rgb = [255.0, 236.0, 150.0];
            }
            if yf > 0.80 && (yf * 18.0 + xf * 9.0).sin() > -0.15 {
                rgb = [36.0, 78.0, 196.0];
            }
            let i = (y * w + x) * 4;
            out[i] = byte(rgb[0]);
            out[i + 1] = byte(rgb[1]);
            out[i + 2] = byte(rgb[2]);
            out[i + 3] = 255;
        }
    }
    out
}

/// Run one raster filter on a premultiplied RGBA buffer. Leaves it unchanged when `id`
/// is not one of those filters or the buffer is the wrong size.
fn apply_pixel(id: &str, params: &Map<String, Value>, data: &mut [u8], w: usize, h: usize) {
    let Some(n) = w.checked_mul(h).and_then(|n| n.checked_mul(4)) else { return };
    if n != data.len() || w == 0 || h == 0 {
        return;
    }
    let effect = vectorcraft_doc::Effect { id: id.to_string(), params: Value::Object(params.clone()), visible: true };
    let Some(vectorcraft_effects::RasterFx::Pixel(fx)) = vectorcraft_effects::raster_effects(std::slice::from_ref(&effect)).into_iter().next() else {
        return;
    };
    fx.apply(data, w, h, &preview_space(w, h));
}

fn filtered(id: &str, params: &Map<String, Value>, w: usize, h: usize) -> Vec<u8> {
    let mut data = sample_drawing(w, h);
    apply_pixel(id, params, &mut data, w, h);
    data
}

/// The drawing with `layers` applied in order.
fn preview_rgba(layers: &[StackEntry], w: usize, h: usize) -> Vec<u8> {
    let mut data = sample_drawing(w, h);
    for layer in layers {
        apply_pixel(&layer.effect, &layer.params, &mut data, w, h);
    }
    data
}

/// Straight RGBA for egui, from the filters' premultiplied bytes.
fn color_image(data: &[u8], w: usize, h: usize) -> egui::ColorImage {
    let mut pixels = Vec::with_capacity(w.saturating_mul(h));
    let (rows, _) = data.as_chunks::<4>();
    for p in rows {
        let a = u32::from(p[3]);
        let (r, g, b) = if a == 0 {
            (0, 0, 0)
        } else if a == 255 {
            (p[0], p[1], p[2])
        } else {
            let s = |c: u8| ((u32::from(c) * 255 + a / 2) / a).min(255) as u8;
            (s(p[0]), s(p[1]), s(p[2]))
        };
        pixels.push(egui::Color32::from_rgba_unmultiplied(r, g, b, p[3]));
    }
    if pixels.len() != w.saturating_mul(h) {
        return egui::ColorImage::filled([w.max(1), h.max(1)], egui::Color32::TRANSPARENT);
    }
    egui::ColorImage::new([w, h], pixels)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::Session;

    use craft::ui::dialog::StackDialog as _;

    use super::EffectGalleryDialog;
    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::host::pump(app, ui.ctx()));
        out.textures_delta.clear();
        ctx
    }

    fn id() -> craft::ui::dialog::DialogId {
        craft::ui::dialog::dialog_id(super::KEY)
    }

    fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
        app.run("ui.dialog.set", json!({"dialog": super::KEY, "field": field, "value": value})).unwrap();
    }

    fn two_layers(app: &mut VectorcraftApp) {
        set(
            app,
            "effects",
            json!([
                { "filter": "artistic.cutout", "visible": true, "params": { "levels": 3 } },
                { "filter": "stylize.glowingEdges", "visible": false, "params": { "edgeWidth": 2 } }
            ]),
        );
    }

    fn drawing(app: &mut VectorcraftApp) -> u64 {
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.effectGalleryDialog", json!({})).unwrap();
        id
    }

    #[test]
    fn stack_preview_is_one_interaction() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let id_node = drawing(&mut app);
        set(&mut app, "effects", json!([{ "filter": "artistic.cutout", "visible": true, "params": { "levels": 3 } }]));
        let ctx = frame(&mut app);
        assert!(app.session.in_interaction(), "the stack previews on the canvas");
        assert!(app.dialogs.cancel_by_id(&ctx, id()));
        frame(&mut app);
        assert!(!app.session.in_interaction());
        assert!(app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id_node)).unwrap().appearance.effects.is_empty());
    }

    #[test]
    fn stack_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let id_node = drawing(&mut app);
        two_layers(&mut app);
        let ctx = frame(&mut app);
        let steps = app.session.active().unwrap().history.undo.len();
        // OK is held: the first answer files the layers, the pump commits them, the second closes the dialog.
        assert!(!app.dialogs.confirm_by_id(&ctx, id()));
        frame(&mut app);
        assert!(app.dialogs.confirm_by_id(&ctx, id()));
        let st = app.session.active().unwrap();
        let n = st.doc.node(vectorcraft_doc::NodeId(id_node)).unwrap();
        assert_eq!(n.appearance.effects.len(), 2);
        assert_eq!(n.appearance.effects[0].id, "artistic.cutout");
        assert_eq!(n.appearance.effects[0].params.get("levels").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(n.appearance.effects[1].id, "stylize.glowingEdges");
        assert!(!n.appearance.effects[1].visible);
        assert_eq!(st.history.undo.len(), steps + 1);
    }

    /// Load Texture… in the Settings, with the file dialog shown off the UI thread: the image goes into the layer
    /// that asked wherever it moved, and an answer for a deleted layer is dropped.
    #[test]
    fn settings_load_texture_answers_into_the_gallery() {
        use vectorcraft_effects::pixel::{TEXTURE_IMAGE, texture};
        let dir = std::env::temp_dir().join(format!("vectorcraft-gallery-texture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("texture.png");
        image::RgbaImage::from_fn(6, 4, |x, y| image::Rgba([(x * 40) as u8, (y * 60) as u8, 90, 255])).save(&path).unwrap();
        let picked = path.to_string_lossy().into_owned();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        // PLAN: assumed `crate::tests::picks::scripted(&mut App) -> Picker` (fl.A20) and `file_asks::poll`.
        let picker = crate::tests::picks::scripted(&mut app);
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        drawing(&mut app);
        let layers = || {
            json!([
                { "filter": "artistic.roughPastels", "visible": true, "params": {} },
                { "filter": "artistic.roughPastels", "visible": true, "params": {} }
            ])
        };
        let loaded_rows = |app: &VectorcraftApp| -> Vec<bool> {
            let fields = app.dialogs.get::<EffectGalleryDialog>().unwrap().json_fields().unwrap();
            fields["effects"]
                .as_array()
                .unwrap()
                .iter()
                .map(|layer| {
                    let params = &layer["params"];
                    params[TEXTURE_IMAGE].as_str().and_then(texture).map(|t| t.size()) == Some((6, 4)) && params["texture"] == json!("loaded")
                })
                .collect()
        };
        // Selects `row`, presses Browse of its Texture row, and lets `meanwhile` change the stack before the answer.
        let ask = |app: &mut VectorcraftApp, row: usize, meanwhile: &dyn Fn(&mut VectorcraftApp)| {
            set(app, "effects", layers());
            set(app, "select", json!(row));
            frame(app);
            app.run("ui.click", json!({"name": "Browse"})).unwrap();
            let ctx = frame(app);
            meanwhile(app);
            picker.answer(&[picked.as_str()]);
            crate::file_asks::poll(app, &ctx);
            loaded_rows(app)
        };
        assert_eq!(ask(&mut app, 1, &|_| {}), [false, true], "the selected layer asked");
        assert_eq!(ask(&mut app, 0, &|app| set(app, "move", json!({"from": 0, "to": 1}))), [false, true], "the layer that asked moved");
        assert_eq!(
            ask(&mut app, 1, &|app| {
                set(app, "delete", json!(true));
            }),
            [false],
            "the layer that asked was deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
