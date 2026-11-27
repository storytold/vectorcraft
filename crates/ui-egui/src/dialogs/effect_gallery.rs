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
use std::sync::{Arc, Weak};

use craft::imaging::ui::filter_gallery::{
    EffectChange, EffectLayer, EffectParameters, EffectStack, FilterCatalog, FilterCategory, FilterGallerySession, FilterIndex, FilterTree,
    FilterTreeLayout, GalleryFilter, GalleryThumb, LayerId, SessionField,
};
use craft::theme::variant::Tone;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft::ui::display::Picture;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use craft::ui::primitives::textures::{Revision, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::words::CommonWords;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::Selection;
use vectorcraft_effects::{EffectInfo, PIXEL_EFFECTS, PixelSpace};
use vectorcraft_geom::{Affine, Point};

use super::effect;
use super::effect_params::{EffectCx, effect_params};
use super::form::json_of;
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;
use crate::file_asks::Owner;

/// Thumbnail of one filter, in pixels at most. The drawing is original; the filter is this app's.
const THUMB: usize = 36;
/// In-dialog preview, in pixels, drawn at twice this size.
const PREVIEW_W: usize = 120;
const PREVIEW_H: usize = 84;

/// The command the preview and OK run: the whole stack as one undo step.
const COMMAND: &str = "command.batch";
/// The undo step the batch makes.
const LABEL: &str = "Effect Gallery";

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

/// What the app keeps of the open gallery and the canvas preview it runs.
pub(crate) struct Open {
    id: DialogId,
    opening: Arc<()>,
    source: (u64, u64),
    preview: LivePreview,
    preview_owner: Option<PreviewOwner>,
}

/// The interaction and batch preview last written by this gallery.
struct PreviewOwner {
    selection: Weak<Selection>,
    batch: Option<(String, Value)>,
}

impl Open {
    fn check_source(&self, app: &VectorcraftApp) -> Result<(), String> {
        let Some(doc) = app.session.active().filter(|doc| doc.uid == self.source.0) else {
            return Err(tl!("Return to the original drawing to apply these effects").into());
        };
        if doc.reverts != self.source.1 {
            return Err(tl!("The document was reverted. Reopen this dialog to continue.").into());
        }
        Ok(())
    }

    fn owns_preview(&self, app: &VectorcraftApp) -> bool {
        app.session.active().filter(|doc| (doc.uid, doc.reverts) == self.source).and_then(|doc| doc.interaction.as_ref()).is_some_and(|interaction| {
            self.preview_owner.as_ref().is_some_and(|owner| {
                owner.selection.upgrade().is_some_and(|selection| Arc::ptr_eq(&selection, &interaction.selection))
                    && owner.batch == interaction.preview
            })
        })
    }

    fn cancel_preview(self, app: &mut VectorcraftApp) {
        if self.owns_preview(app) {
            self.preview.cancel(app);
        }
    }

    fn ready_to_edit(&mut self, app: &mut VectorcraftApp) -> Result<bool, String> {
        if self.owns_preview(app) {
            return Ok(true);
        }
        self.preview = LivePreview::default();
        self.preview_owner = None;
        if app.session.in_interaction() {
            app.keep_typing()?;
        }
        Ok(!app.session.in_interaction())
    }

    fn run_preview(&mut self, app: &mut VectorcraftApp, on: bool, batch: &Value) -> Result<(), String> {
        if (on || self.owns_preview(app)) && self.ready_to_edit(app)? {
            let ran = self.preview.run(app, on, LABEL, COMMAND, batch);
            if !on {
                self.preview_owner = None;
            } else if ran.is_some() {
                self.preview_owner = app
                    .session
                    .active()
                    .and_then(|doc| doc.interaction.as_ref())
                    .map(|interaction| PreviewOwner { selection: Arc::downgrade(&interaction.selection), batch: interaction.preview.clone() });
            }
        }
        Ok(())
    }
}

/// Opens the gallery on the current selection. OK is held: [`commit`] keeps the previewed stack, and a refusal
/// keeps the dialog open with its reason; Cancel takes the canvas preview away.
pub fn open(app: &mut VectorcraftApp, _: &Value) -> Result<Value, String> {
    let Some(doc) = app.session.active().filter(|doc| !doc.selection.is_empty()) else {
        return Err(tl!("Select artwork to apply effects").into());
    };
    let source = (doc.uid, doc.reverts);
    let dialog = EffectGalleryDialog::new(app);
    let opening = Arc::clone(&dialog.opening);
    let cancelled = Arc::clone(&opening);
    let rollback = move |app: &mut VectorcraftApp| {
        if app.effect_dialogs.gallery.as_ref().is_some_and(|open| Arc::ptr_eq(&cancelled, &open.opening))
            && let Some(open) = app.effect_dialogs.gallery.take()
        {
            open.cancel_preview(app);
        }
    };
    let options = HeldOptions { cancel: Some(Box::new(rollback)), ..HeldOptions::default() };
    let id = open_held_with(app, dialog, commit, options).ok_or("no window")?;
    if let Some(old) = app.effect_dialogs.gallery.take() {
        old.cancel_preview(app);
    }
    app.effect_dialogs.gallery = Some(Open { id, opening, source, preview: LivePreview::default(), preview_owner: None });
    Ok(Value::Null)
}

/// The held OK: keeps the previewed stack as one undo step. A refused batch leaves the gallery and its preview open.
fn commit(app: &mut VectorcraftApp, answer: &GalleryAnswer) -> Result<(), String> {
    let Some(mut open) = app.effect_dialogs.gallery.take() else {
        return Err(tl!("The Effect Gallery is closed").into());
    };
    if !Arc::ptr_eq(&answer.opening, &open.opening) {
        app.effect_dialogs.gallery = Some(open);
        return Err(tl!("The Effect Gallery is closed").into());
    }
    let ready = open
        .check_source(app)
        .and_then(|()| if open.ready_to_edit(app)? { Ok(()) } else { Err(tl!("Finish the current edit before applying the effects").into()) });
    if let Err(why) = ready {
        app.effect_dialogs.gallery = Some(open);
        return Err(why);
    }
    let committed = std::mem::take(&mut open.preview).commit(app, COMMAND, answer.batch.clone()).map(drop);
    if committed.is_err() {
        open.preview_owner = None;
        app.effect_dialogs.gallery = Some(open);
    }
    committed
}

/// Runs the gallery's part of one frame: shows the visible layers on the canvas and starts a Browse press's file
/// dialog. A closed gallery's preview is the held OK's to keep or its cancel's to take away.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(mut open) = app.effect_dialogs.gallery.take() else { return };
    let id = open.id;
    let shown = dialogs.with(|host| {
        host.get_mut_by_id::<EffectGalleryDialog>(id).filter(|dialog| Arc::ptr_eq(&open.opening, &dialog.opening)).map(|dialog| {
            let visible = dialog.entries(true);
            let asked = dialog.asked.take().map(|layer| (Arc::clone(&dialog.opening), layer));
            (dialog.preview_on && !visible.is_empty(), stack_batch_params(&visible), asked)
        })
    });
    let Some((on, batch, asked)) = shown else {
        app.effect_dialogs.gallery = Some(open);
        return;
    };
    if open.check_source(app).is_ok()
        && let Err(why) = open.run_preview(app, on, &batch)
    {
        app.status(why);
    }
    app.effect_dialogs.gallery = Some(open);
    if let Some((opening, layer)) = asked {
        ask_texture(app, id, opening, layer);
    }
}

/// Load Texture follows `layer` within its gallery opening and drops the reply when either is gone.
fn ask_texture(app: &mut VectorcraftApp, id: DialogId, opening: Arc<()>, layer: LayerId) {
    effect::pick_texture(app, Owner::Free, move |app, _, fields| {
        let live = app.effect_dialogs.gallery.as_ref().is_some_and(|open| {
            Arc::ptr_eq(&opening, &open.opening) && app.session.documents().iter().any(|doc| (doc.uid, doc.reverts) == open.source)
        });
        if !live {
            return;
        }
        let Some(dialogs) = app.dialogs() else { return };
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut_by_id::<EffectGalleryDialog>(id).filter(|dialog| Arc::ptr_eq(&opening, &dialog.opening)) {
                dialog.answer_texture(layer, fields);
            }
        });
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
    fn new(app: &VectorcraftApp) -> Self {
        let mut categories = Vec::new();
        let mut filters = Vec::new();
        for (at, (name, _)) in CATEGORIES.iter().enumerate() {
            let ids: Vec<&str> = effects_in_category(at).into_iter().map(|e| e.id).collect();
            if ids.is_empty() {
                continue;
            }
            for id in &ids {
                // The rows do not depend on the values: a texture row reads the loaded texture each pass.
                let cx = EffectCx::new(app, id, false);
                filters.push(GalleryFilter::new(*id, Label::from(effect_label(id)), *name, effect_params(&cx, &DialogFields::new())));
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
            let Value::Object(params) = vectorcraft_effects::upgrade_params(&layer.filter, &Value::Object(json_of(&filter.params, &layer.fields)))
            else {
                return None;
            };
            Some(StackEntry { effect: layer.filter.clone(), visible: layer.visible, params })
        })
        .collect()
}

/// The Effect Gallery on the dialog host: the framework's filter tree, settings and effect-layer stack over one
/// [`FilterGallerySession`], with a preview of the visible layers and OK held for [`commit`].
pub(crate) struct EffectGalleryDialog {
    id: DialogId,
    /// Dialog IDs and layer IDs can both be reused when the gallery opens again.
    opening: Arc<()>,
    open: bool,
    catalog: Catalog,
    index: FilterIndex,
    session: FilterGallerySession,
    selected: usize,
    preview_on: bool,
    /// The batch OK filed; the app commits it and answers.
    held: HeldAnswer<Value>,
    /// A Browse press waiting for the app's file dialog, with the layer it belongs to.
    asked: Option<LayerId>,
}

impl EffectGalleryDialog {
    /// A gallery on one layer of the first Artistic filter, previewing.
    fn new(app: &VectorcraftApp) -> Self {
        let catalog = Catalog::new(app);
        let first = catalog.categories.first().and_then(|category| category.filters.first()).cloned().unwrap_or_default();
        let index = FilterIndex::of(&catalog);
        let session = FilterGallerySession::new(&first, &catalog);
        EffectGalleryDialog {
            id: dialog_id(KEY),
            opening: Arc::new(()),
            open: true,
            catalog,
            index,
            session,
            selected: 0,
            preview_on: true,
            held: HeldAnswer::new(),
            asked: None,
        }
    }

    /// The layers as the batch takes them.
    fn entries(&self, visible_only: bool) -> Vec<StackEntry> {
        entries_of(&self.session.effects, &self.index, visible_only)
    }

    /// Writes the picked texture into the layer `layer`, wherever it is now. A deleted layer takes nothing.
    fn answer_texture(&mut self, layer: LayerId, fields: [(&'static str, FieldValue); 2]) {
        let Some(at) = self.session.position(layer) else { return };
        let Some(mut values) = self.session.effects.get(at).map(|effect| effect.fields.clone()) else { return };
        for (key, value) in fields {
            values.set(key, value);
        }
        self.apply(EffectChange::with_fields(at, values));
    }

    /// Applies `change` to the stack.
    fn apply(&mut self, change: EffectChange) {
        self.session.apply(change, &self.catalog, &mut self.selected);
    }
}

/// The stack filed for OK and the opening that supplied it.
#[derive(Clone)]
pub(crate) struct GalleryAnswer {
    opening: Arc<()>,
    batch: Value,
}

impl HoldsOk for EffectGalleryDialog {
    type Answer = GalleryAnswer;

    fn held_ok(self) -> Self {
        self
    }

    fn take_ok(&mut self) -> Option<GalleryAnswer> {
        Some(GalleryAnswer { opening: Arc::clone(&self.opening), batch: self.held.take()? })
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

impl StackDialog for EffectGalleryDialog {
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        let words = Env::of(ui).words::<CommonWords>();
        let EffectGalleryDialog { open, catalog, index, session, selected, preview_on, held, asked, .. } = self;
        // The app accepted the held OK: this pass closes the dialog with it. A measuring run closes nothing.
        let finish = held.is_accepted() && !craft::ui::runtime::measure::active(ui.ctx());
        // A refusal is cleared once the stack changes, so one still held is for the stack shown.
        let refusal = held.refusal().map(|(reason, _)| reason.clone());
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
        let dialog = Dialog::new(tl!("Effect Gallery")).opened(Bind::bound(open)).buttons(buttons).close_on_enter(true).footer(|ui, _| {
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
                if let Some(reason) = &refusal {
                    Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
                }
            });
        });
        *preview_on = on;
        // A pass applies the changes its own input made; a second pass of the frame reports none.
        if let Some(at) = picked_layer {
            *selected = at;
        }
        let changed = !changes.is_empty();
        for change in changes {
            session.apply(change, &*catalog, selected);
        }
        if asked.is_none() {
            *asked = pressed;
        }
        if changed {
            held.clear_refusal();
        }
        // OK files the batch for the app, which commits it before the dialog closes.
        if shown.output.picked == Some(0) && !held.is_waiting() {
            held.file(stack_batch_params(&entries_of(&session.effects, index, false)));
        }
        if finish && shown.output.closed == Some(CloseReason::Activated) {
            return Some(held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        shown.output.closed.map(|_| DialogResult::Cancel)
    }

    /// OK as the button: files the batch and answers none; once the app accepted it, answers it.
    fn confirm(&mut self, _: &Context) -> Option<Value> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if !self.held.is_waiting() {
            self.held.file(stack_batch_params(&self.entries(false)));
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
            Some(SessionField::Change(change)) => {
                self.apply(change);
                self.held.clear_refusal();
            }
            Some(SessionField::Select(at)) => self.selected = at.min(self.session.effects.len().saturating_sub(1)),
            _ => {}
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
    json!({ "label": LABEL, "commands": stack_batch_commands(stack) })
}

fn effect_label(id: &str) -> String {
    crate::i18n::effect_name(crate::i18n::current(), id).unwrap_or_else(|| id.to_string())
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
    use craft::ui::dialog::{DialogId, StackDialog as _, dialog_id};
    use serde_json::{Value, json};
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    use super::EffectGalleryDialog;
    use crate::VectorcraftApp;

    /// An app with a window context and a 40 pt square, and the gallery open on it. Returns the square's id.
    fn drawing() -> (VectorcraftApp, egui::Context, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        super::open(&mut app, &json!({})).unwrap();
        (app, ctx, id)
    }

    fn id() -> DialogId {
        dialog_id(super::KEY)
    }

    /// `ui.dialog.set` on the gallery.
    fn set(app: &VectorcraftApp, ctx: &egui::Context, field: &str, value: Value) {
        app.dialogs().unwrap().with(|host| host.set_json_field(ctx, id(), field, value)).unwrap();
    }

    /// The pump of one frame: the canvas preview, the held OK and the file asks.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
        crate::dialogs::host::pump(app, ctx);
    }

    fn effects(app: &VectorcraftApp, node: u64) -> Vec<vectorcraft_doc::Effect> {
        app.session.active().unwrap().doc.node(NodeId(node)).unwrap().appearance.effects.clone()
    }

    #[test]
    fn stack_preview_is_one_interaction() {
        let (mut app, ctx, node) = drawing();
        set(&app, &ctx, "effects", json!([{ "filter": "artistic.cutout", "visible": true, "params": { "levels": 3 } }]));
        frame(&mut app, &ctx);
        assert!(app.session.in_interaction(), "the stack previews on the canvas");
        assert!(app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id())));
        frame(&mut app, &ctx);
        assert!(!app.session.in_interaction());
        assert!(effects(&app, node).is_empty());
    }

    #[test]
    fn stack_ok_is_one_undo_step() {
        let (mut app, ctx, node) = drawing();
        set(
            &app,
            &ctx,
            "effects",
            json!([
                { "filter": "artistic.cutout", "visible": true, "params": { "levels": 3 } },
                { "filter": "stylize.glowingEdges", "visible": false, "params": { "edgeWidth": 2 } }
            ]),
        );
        frame(&mut app, &ctx);
        let steps = app.session.active().unwrap().history.undo.len();
        // OK is held: the first answer files the batch, the pump commits it, the second closes the dialog.
        let dialogs = app.dialogs().unwrap();
        assert!(!dialogs.with(|host| host.confirm_by_id(&ctx, id())));
        frame(&mut app, &ctx);
        assert!(dialogs.with(|host| host.confirm_by_id(&ctx, id())));
        let applied = effects(&app, node);
        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0].id, "artistic.cutout");
        assert_eq!(applied[0].params.get("levels").and_then(Value::as_i64), Some(3));
        assert_eq!(applied[1].id, "stylize.glowingEdges");
        assert!(!applied[1].visible);
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1);
    }

    /// A delayed Load Texture… reply follows its layer when it moves and is dropped when the layer or its gallery closes.
    #[test]
    fn settings_load_texture_answers_into_the_gallery() {
        use vectorcraft_effects::pixel::{TEXTURE_IMAGE, texture};
        let dir = std::env::temp_dir().join(format!("vectorcraft-gallery-texture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("texture.png");
        image::RgbaImage::from_fn(6, 4, |x, y| image::Rgba([(x * 40) as u8, (y * 60) as u8, 90, 255])).save(&path).unwrap();
        let picked = path.to_string_lossy().into_owned();
        let (mut app, ctx, _) = drawing();
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        let picker = crate::tests::picks::scripted(&mut app);
        let loaded_rows = |app: &VectorcraftApp| -> Vec<bool> {
            let fields = app.dialogs().unwrap().with(|host| host.get_mut_by_id::<EffectGalleryDialog>(id()).unwrap().json_fields().unwrap());
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
        let rough_pastels = || {
            let layer = json!({ "filter": "artistic.roughPastels", "visible": true, "params": {} });
            json!([layer, layer])
        };
        // Selects `row`, presses Browse of its Texture row, and lets `meanwhile` change the gallery before the answer.
        let ask = |app: &mut VectorcraftApp, row: usize, meanwhile: &dyn Fn(&mut VectorcraftApp)| {
            set(app, &ctx, "effects", rough_pastels());
            set(app, &ctx, "select", json!(row));
            // The Texture row's Browse press, as the dialog's pass records it.
            app.dialogs().unwrap().with(|host| {
                let dialog = host.get_mut_by_id::<EffectGalleryDialog>(id()).unwrap();
                dialog.asked = Some(dialog.session.effects[dialog.selected].id);
            });
            frame(app, &ctx);
            meanwhile(app);
            picker.answer(&[picked.as_str()]);
            crate::file_asks::poll(app);
            loaded_rows(app)
        };
        assert_eq!(ask(&mut app, 1, &|_| {}), [false, true], "the selected layer asked");
        assert_eq!(ask(&mut app, 0, &|app| set(app, &ctx, "move", json!({"from": 0, "to": 1}))), [false, true], "the layer that asked moved");
        assert_eq!(ask(&mut app, 1, &|app| set(app, &ctx, "delete", json!(true))), [false], "the layer that asked was deleted");
        let reopen = |app: &mut VectorcraftApp| {
            assert!(app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id())));
            frame(app, &ctx);
            super::open(app, &json!({})).unwrap();
        };
        // Both openings start with identical stacks, which gives their corresponding layers the same session-local IDs.
        reopen(&mut app);
        let answers = ask(&mut app, 0, &|app| {
            reopen(app);
            set(app, &ctx, "effects", rough_pastels());
        });
        assert_eq!(answers, [false, false], "a reopened gallery keeps its own textures");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn ownership_stack(app: &VectorcraftApp, ctx: &egui::Context) {
        set(app, ctx, "effects", json!([{ "filter": "artistic.cutout", "visible": true, "params": { "levels": 3 } }]));
    }

    fn second_drawing(app: &mut VectorcraftApp) -> u64 {
        app.run("file.new", json!({"width": 220, "height": 220})).unwrap();
        app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
    }

    fn hold_ok(app: &VectorcraftApp, ctx: &egui::Context) {
        assert!(!app.dialogs().unwrap().with(|host| host.confirm_by_id(ctx, id())));
    }

    #[test]
    fn gallery_ownership_preview_resumes_in_the_opening_drawing() {
        let (mut app, ctx, original) = drawing();
        ownership_stack(&app, &ctx);
        frame(&mut app, &ctx);
        assert_eq!(effects(&app, original)[0].id, "artistic.cutout");
        let other = second_drawing(&mut app);
        frame(&mut app, &ctx);
        assert!(effects(&app, other).is_empty(), "gallery preview reached the other drawing");
        assert!(!app.session.in_interaction());
        app.run("document.activate", json!({"index": 0})).unwrap();
        frame(&mut app, &ctx);
        assert_eq!(effects(&app, original)[0].id, "artistic.cutout");
    }

    #[test]
    fn gallery_ownership_ok_refuses_the_other_drawing_and_can_retry() {
        let (mut app, ctx, original) = drawing();
        ownership_stack(&app, &ctx);
        hold_ok(&app, &ctx);
        let other = second_drawing(&mut app);
        let history = app.session.active().unwrap().history.undo.len();
        frame(&mut app, &ctx);
        assert!(effects(&app, other).is_empty(), "gallery OK reached the other drawing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), history);
        app.run("document.activate", json!({"index": 0})).unwrap();
        let history = app.session.active().unwrap().history.undo.len();
        hold_ok(&app, &ctx);
        frame(&mut app, &ctx);
        assert_eq!(effects(&app, original)[0].id, "artistic.cutout");
        assert_eq!(app.session.active().unwrap().history.undo.len(), history + 1);
    }

    #[test]
    fn gallery_ownership_cancel_preserves_the_other_drawings_edit() {
        let (mut app, ctx, _) = drawing();
        ownership_stack(&app, &ctx);
        frame(&mut app, &ctx);
        second_drawing(&mut app);
        app.session.begin_interaction("Other drawing's edit").unwrap();
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id())));
        frame(&mut app, &ctx);
        assert!(app.session.in_interaction(), "gallery Cancel ended the other drawing's edit");
        assert!(app.session.active().unwrap().doc == before);
    }

    #[test]
    fn gallery_ownership_cancel_preserves_a_replaced_preview_in_the_same_drawing() {
        let (mut app, ctx, _) = drawing();
        ownership_stack(&app, &ctx);
        frame(&mut app, &ctx);
        app.session.preview("paint.setFill", &json!({"color": "#00aa00"})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id())));
        frame(&mut app, &ctx);
        assert!(app.session.in_interaction(), "gallery Cancel ended another preview on the same interaction");
        assert!(app.session.active().unwrap().doc == before);
    }

    #[test]
    fn gallery_ownership_replacement_keeps_its_own_preview_and_answer() {
        let (mut app, ctx, original) = drawing();
        ownership_stack(&app, &ctx);
        frame(&mut app, &ctx);
        super::open(&mut app, &json!({})).unwrap();
        set(&app, &ctx, "effects", json!([{ "filter": "artistic.cutout", "visible": true, "params": { "levels": 5 } }]));
        frame(&mut app, &ctx);
        assert!(app.effect_dialogs.gallery.is_some(), "a replaced gallery's Cancel removed the new opening");
        assert_eq!(effects(&app, original)[0].params["levels"], 5);
        hold_ok(&app, &ctx);
        frame(&mut app, &ctx);
        assert_eq!(effects(&app, original)[0].params["levels"], 5);
    }

    #[test]
    fn gallery_ownership_ok_refuses_a_reloaded_drawing() {
        let (mut app, ctx, original) = drawing();
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        let folder = vectorcraft_testkit::TempDir::new("effect-gallery-opening-revert");
        let path = folder.join("drawing.vectorcraft");
        app.run("file.saveAs", json!({"path": path})).unwrap();
        app.run("shape.rectangle", json!({"x": 80, "y": 80, "width": 30, "height": 30})).unwrap();
        app.run("select.set", json!({"ids": [original]})).unwrap();
        let source = app.session.active().unwrap();
        let (uid, generation) = (source.uid, source.reverts);
        ownership_stack(&app, &ctx);
        hold_ok(&app, &ctx);
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let reloaded = app.session.active().unwrap();
        assert_eq!(reloaded.uid, uid);
        assert_ne!(reloaded.reverts, generation);
        app.run("select.set", json!({"ids": [original]})).unwrap();
        let history = app.session.active().unwrap().history.undo.len();
        frame(&mut app, &ctx);
        assert!(effects(&app, original).is_empty(), "gallery OK reached the reloaded drawing");
        assert_eq!(app.session.active().unwrap().history.undo.len(), history);
        super::open(&mut app, &json!({})).unwrap();
        ownership_stack(&app, &ctx);
        hold_ok(&app, &ctx);
        frame(&mut app, &ctx);
        assert_eq!(effects(&app, original)[0].id, "artistic.cutout");
    }
}
