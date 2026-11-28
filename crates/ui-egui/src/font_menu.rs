//! The app's configuration of the framework font picker, shared by every font field: the Character,
//! Properties and Glyphs panels, the Control bar, Composite Fonts and the dimension dialog.
//!
//! [`family_picker`] is the one construction of a [`FontPicker`]; its settings (stars, recents, preview
//! size, names language) are a view of the app's stores ([`picker_settings`]) and are written back
//! only where the picker changed them ([`store_picker_settings`]). The Document and Recently Added
//! filters are the picker's family sets ([`family_sets`]); their families are found only while the
//! filter is on and are cached on the canvas cache ([`document_family_set`], [`recently_added_set`]).
//! [`font_fields`] applies a pick, an arrow step or a wheel run to the selected text (one live
//! interaction while the row under the pointer previews, one undo step per pick); a host that only picks a font
//! reads the pick from a [`family_picker`] over its [`PickerHold`]. [`paint_family_sample`] and [`menu_sample`] set a sample of text in a family's own
//! face.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::SystemTime;

use craft::behavior::{EditEvent, Preview};
use craft::font_manager::picker::{FamilySet, FilterLayout, FontFilters, FontPicker, FontPickerSettings, FontPreviewSize, FontScript};
use craft::ui::contract::{Bind, Entry, Show as _};
use craft::ui::fonts::{FaceBytes, FaceState};
use craft::ui::pickers::Select;
use craft::ui::primitives::{ContentPainterCx, DocRun, DocText, FacePaint, FacesCx as _};
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use egui::{Color32, Rect, Ui, vec2};
use serde_json::json;
use vectorcraft_text::FontDb;

use crate::VectorcraftApp;
use crate::caches::FontsKey;
use crate::edits::{self, EditPath};
use crate::panels::Live;

/// The sample a row shows when its family lacks the characters of the text.
const FALLBACK_SAMPLE: &str = "Sample";
/// What a sample is drawn on with Show samples on white.
const SAMPLE_PAPER: Color32 = Color32::WHITE;
/// Space above and below a sample's paper.
const PAPER_INSET: f32 = 2.0;
/// Space between the left edge of a sample's paper and its type.
const PAPER_PAD: f32 = 3.0;
/// The share of a sample's height its type's size takes.
const SAMPLE_SCALE: f32 = 0.7;

/// The keys of the two family sets in [`FontFilters::sets`].
const DOCUMENT_SET: &str = "document";
const ADDED_SET: &str = "recently-added";

/// Font Preview Size's stored values (Preferences › Type) and the picker sizes they stand for.
const PREVIEW_SIZES: [(&str, FontPreviewSize); 3] =
    [("small", FontPreviewSize::Small), ("medium", FontPreviewSize::Medium), ("large", FontPreviewSize::Large)];

/// The picker's remembered state as the app's stores hold it.
pub(crate) fn picker_settings(app: &VectorcraftApp) -> FontPickerSettings {
    let prefs = &app.session.prefs;
    let preview = if prefs.font_preview {
        PREVIEW_SIZES.iter().find(|(key, _)| *key == prefs.font_preview_size).map_or(FontPreviewSize::Medium, |(_, size)| *size)
    } else {
        FontPreviewSize::None
    };
    let mut settings = FontPickerSettings::default();
    settings.favorites = app.ui.favorite_fonts.clone();
    settings.recent = app.recent_fonts().to_vec();
    settings.recent_limit = usize::try_from(prefs.recent_fonts_count).unwrap_or(usize::MAX);
    settings.preview = preview;
    settings.names_in_english = prefs.font_names_in_english;
    settings.samples_on_white = prefs.font_samples_on_white;
    settings
}

/// Writes back what the picker changed in `after` against `before`: the stars, and a Font Preview Size
/// its row of buttons chose (written like Preferences does). Recents are not written: the font command a
/// pick runs notes them.
pub(crate) fn store_picker_settings(app: &mut VectorcraftApp, before: &FontPickerSettings, after: FontPickerSettings) {
    if after.favorites != before.favorites {
        app.ui.favorite_fonts = after.favorites;
    }
    if after.preview != before.preview
        && let Some((key, _)) = PREVIEW_SIZES.iter().find(|(_, size)| *size == after.preview)
    {
        match app.run("prefs.set", json!({"key": "fontPreviewSize", "value": key})) {
            Ok(_) => {
                crate::prefs_dialog::snapshot(app);
                app.prefs_changed = true;
            }
            Err(e) => app.ui.status = e,
        }
    }
}

/// The Document and Recently Added filters of the picker. Document is unavailable (`None`) while no
/// document is open.
pub(crate) fn family_sets<'a>(document: Option<&'a [String]>, recently_added: &'a [String]) -> [FamilySet<'a>; 2] {
    [
        FamilySet::new(DOCUMENT_SET, tl!("Document"), document.unwrap_or_default())
            .hint(tl!("Fonts used in this document"))
            .enabled(document.is_some()),
        FamilySet::new(ADDED_SET, tl!("Recently Added"), recently_added).hint(tl!("Fonts added in the last 30 days")).ordered(),
    ]
}

/// The one font family picker of the app.
pub(crate) fn family_picker<'a>(
    label: impl Into<Label>,
    family: Bind<'a, String>,
    settings: Bind<'a, FontPickerSettings>,
    filters: Bind<'a, FontFilters>,
    sets: &'a [FamilySet<'a>],
) -> FontPicker<'a> {
    FontPicker::new(label, family, FontDb::global().catalog())
        .settings(settings)
        .filters(filters)
        .sets(sets)
        .scripts(&FontScript::ALL)
        .filter_layout(FilterLayout::Chips)
}

/// What one pass of a picker reads of the app, for a host that has no `app` while it draws (a dialog).
pub(crate) struct PickerHold {
    /// The picker's remembered state.
    pub settings: FontPickerSettings,
    /// The filters every font picker shares.
    pub filters: FontFilters,
    /// The families the document's type is set in; `None` while no document is open.
    pub document: Option<Arc<[String]>>,
    /// The recently added families.
    pub recently_added: Arc<[String]>,
}

/// The picker's inputs now. A set's families are found only while its filter is on; the other is empty.
pub(crate) fn hold(app: &mut VectorcraftApp) -> PickerHold {
    let filters = app.ui.font_filters.clone();
    let on = |key: &str| filters.sets.iter().any(|k| k == key);
    let (document_on, added_on) = (on(DOCUMENT_SET), on(ADDED_SET));
    let document = app.session.active().is_some().then(|| if document_on { document_family_set(app) } else { Arc::from(Vec::new()) });
    let recently_added = if added_on { recently_added_set(app, added_clock()) } else { Arc::from(Vec::new()) };
    PickerHold { settings: picker_settings(app), filters, document, recently_added }
}

impl PickerHold {
    /// Writes back what the picker changed, given the settings `before` it ran.
    pub(crate) fn store(self, app: &mut VectorcraftApp, before: &FontPickerSettings) {
        store_picker_settings(app, before, self.settings);
        app.ui.font_filters = self.filters;
    }
}

/// The time the Recently Added filter counts back from. On wasm, which has no installed fonts to date,
/// the Unix epoch.
fn added_clock() -> SystemTime {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
    }
    #[cfg(target_arch = "wasm32")]
    {
        SystemTime::UNIX_EPOCH
    }
}

/// The families the picker for `family` shows: its field, filters and sets drawn in `ui`.
fn show_picker(
    app: &mut VectorcraftApp,
    ui: &mut Ui,
    label: impl Into<Label>,
    family: &str,
    sample: Option<&str>,
) -> craft::font_manager::picker::FontPickerOutput {
    let mut hold = hold(app);
    let before = hold.settings.clone();
    let out = {
        let PickerHold { settings, filters, document, recently_added } = &mut hold;
        let sets = family_sets(document.as_deref(), recently_added);
        let mut picker = family_picker(label, Bind::given(family.to_owned()), Bind::bound(settings), Bind::bound(filters), &sets);
        if let Some(text) = sample {
            picker = picker.preview_text(text);
        }
        picker.show(ui)
    };
    hold.store(app, &before);
    out
}

/// The font family picker and the font style select of the selected text (Character and Properties
/// panels, Control bar, Contextual Task Bar), applying what they pick.
pub(crate) fn font_fields(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((s, _)) = crate::panels::character::text_style(app) else { return };
    let sample = sample_text(app);
    let out = show_picker(app, ui, tl!("Font Family"), &s.font_family, sample.as_deref());
    // The preview ends before the pick that ends it is applied.
    apply_preview(app, out.edit.response.id, out.preview);
    let style = out.picked_style;
    let mut applied = false;
    for event in out.edit.events {
        applied = true;
        apply_family(app, event, style.clone());
    }
    // A style row of the family the field shows sets no new family value.
    if !applied && let (Some(family), Some(style)) = (out.picked, style) {
        apply_font(app, &family, Some(&style), Live::Released);
    }
    let styles = FontDb::global().styles(&s.font_family);
    let entries: Vec<Entry<String>> = styles.iter().map(|name| Entry::new(name.clone(), name.clone())).collect();
    let mut shown = s.font_style.clone();
    Select::new(tl!("Style"), Bind::bound(&mut shown), &entries).show(ui);
    if shown != s.font_style {
        apply_font(app, &s.font_family, Some(&shown), Live::Released);
    }
}

/// Sets `family` (and `style`, else the closest the family has) on the text the font fields act on, in
/// the phase of the gesture.
fn apply_font(app: &mut VectorcraftApp, family: &str, style: Option<&str>, phase: Live) {
    if phase == Live::Released {
        crate::panels::character::end_typing(app);
    }
    let (cmd, params) = font_command(app, family, style);
    crate::panels::live_run(app, "Font", cmd, params, phase);
}

/// Applies a step of a family edit: a pick or an arrow step is one undo step, a wheel run previews per
/// notch and commits one step.
pub(crate) fn apply_family(app: &mut VectorcraftApp, event: EditEvent<String>, style: Option<String>) {
    edits::apply_event(app, &event, &EditPath::Preview, &mut |app, family, phase| apply_font(app, family, style.as_deref(), phase));
}

/// Shows or ends the live preview of the row under the pointer or the keyboard on the selected text, in
/// one interaction that leaves nothing in the history. `owner` is the picker that shows it.
pub(crate) fn apply_preview(app: &mut VectorcraftApp, owner: egui::Id, preview: Preview<String>) {
    match preview {
        Preview::Shows(family) => {
            if app.font_preview_owner.is_none() {
                crate::panels::character::end_typing(app);
                if let Err(e) = app.begin_interaction("Font") {
                    app.status(e);
                    return;
                }
            }
            app.font_preview_owner = Some(owner);
            let (cmd, params) = font_command(app, &family, None);
            if let Err(e) = app.session.preview(cmd, &params) {
                app.ui.status = e.to_string();
            }
            app.sync_views();
        }
        Preview::Ended => cancel_preview(app),
        _ => {}
    }
}

/// Ends the preview of a picker that stopped showing (`FrameOutput::ended_previews`).
pub(crate) fn end_previews(app: &mut VectorcraftApp, ended: &[egui::Id]) {
    if app.font_preview_owner.is_some_and(|owner| ended.contains(&owner)) {
        cancel_preview(app);
    }
}

/// Takes the preview back.
fn cancel_preview(app: &mut VectorcraftApp) {
    if app.font_preview_owner.take().is_some() {
        // No interaction left to cancel is the state this wants.
        app.session.cancel_interaction().ok();
        app.sync_views();
    }
}

/// The families the type in `doc` is set in, lowercase and by the name the menu lists them under
/// ([`vectorcraft_text::FontDb::canonical`]): the runs of every text object in the layers, symbols,
/// patterns, graph designs and stored scatter brushes, hidden, locked and masked ones included, with
/// a composite font named as such. A pattern being edited is read from its editing layer only. Each
/// run's family is looked up with its style, so a family and style split from a PostScript name
/// (`Honoka`, `Mincho`) find the installed font of that name. Styles no type uses are left out.
fn document_families(db: &vectorcraft_text::FontDb, doc: &vectorcraft_doc::Document) -> HashSet<String> {
    // Each family and style once, so a document with many runs in a few fonts looks each one up
    // once.
    let mut named: HashMap<String, HashSet<String>> = HashMap::new();
    let mut add = |n: &vectorcraft_doc::Node| {
        if let vectorcraft_doc::NodeKind::Text(t) = &n.kind {
            for r in &t.runs {
                let (family, style) = (&r.style.font_family, &r.style.font_style);
                match named.get_mut(family) {
                    Some(styles) => {
                        if !styles.contains(style) {
                            styles.insert(style.clone());
                        }
                    }
                    None => {
                        named.insert(family.clone(), HashSet::from([style.clone()]));
                    }
                }
            }
        }
    };
    let editing = doc.pattern_edit.as_ref().map(|e| e.pattern.as_str());
    let patterns = doc.patterns.iter().filter(|p| Some(p.name.as_str()) != editing).flat_map(|p| p.art.iter().map(|a| &**a));
    let designs = doc.graph_designs.iter().map(|g| &*g.art);
    for n in doc.layers.iter().map(|l| &**l).chain(doc.symbols.iter().map(|s| &*s.art)).chain(patterns).chain(designs) {
        vectorcraft_doc::swatches::walk_with_masks(n, &mut add);
    }
    // A scatter brush draws copies of its art's type along a stroke; art and pattern brushes drop
    // type when they bend their art. Malformed library entries are skipped.
    let brushes = doc.unknown.get(vectorcraft_brush::DOC_KEY).map(vectorcraft_brush::parse_library).unwrap_or_default();
    for b in &brushes {
        if let vectorcraft_brush::BrushKind::Scatter(s) = &b.kind {
            vectorcraft_doc::swatches::walk_with_masks(&s.art, &mut add);
        }
    }
    named.iter().flat_map(|(f, styles)| styles.iter().map(move |s| db.canonical(f, s).0.to_lowercase())).collect()
}

/// The families the Document filter lists, sorted: those of the active document (the one a running
/// interaction shows), found again only when the document, its version or the font list changed. The
/// names are lowercase ([`document_families`]), so the filter matches them ignoring case.
pub(crate) fn document_family_set(app: &mut VectorcraftApp) -> Arc<[String]> {
    let db = FontDb::global();
    let Some(state) = app.session.active() else { return Arc::from(Vec::new()) };
    let doc = state.interaction.as_ref().map_or(&state.doc, |interaction| &interaction.doc);
    let key = FontsKey { uid: state.uid, doc: Arc::downgrade(doc), generation: db.generation() };
    if let Some((held, families)) = &app.canvas.document_families
        && *held == key
    {
        return families.clone();
    }
    let mut names: Vec<String> = document_families(db, doc).into_iter().collect();
    names.sort_unstable();
    let families: Arc<[String]> = Arc::from(names);
    app.canvas.document_families = Some((key, families.clone()));
    families
}

/// The families the Recently Added filter lists, newest first: those `FontDb` dates within
/// [`vectorcraft_text::RECENTLY_ADDED_WITHIN`] before `now`, found again only when the font list
/// changed, `now` reached the instant the list may change at, or the clock went back before the time
/// the list was found for.
pub(crate) fn recently_added_set(app: &mut VectorcraftApp, now: SystemTime) -> Arc<[String]> {
    let db = FontDb::global();
    let generation = db.generation();
    if let Some((held, asked, families, recheck_at)) = &app.canvas.recently_added
        && *held == generation
        && *asked <= now
        && recheck_at.is_none_or(|at| now < at)
    {
        return families.clone();
    }
    let list = db.recently_added(now);
    let families: Arc<[String]> = Arc::from(list.families);
    app.canvas.recently_added = Some((generation, now, families.clone(), list.recheck_at));
    families
}

/// `text` (or [`FALLBACK_SAMPLE`] when the family lacks some of its characters) set in `family`'s
/// `style`, filling `rect`'s height: black on white paper with `on_white`, else in the interface's text
/// colour. Nothing is painted while the face loads or when it cannot be loaded.
pub(crate) fn paint_family_sample(cx: &ContentPainterCx, rect: Rect, family: &str, style: &str, text: Option<&str>, on_white: bool) {
    let db = FontDb::global();
    let catalog = db.catalog();
    let loaded = db.face(family, style);
    let state = match catalog.resolve(family, style) {
        Some((face, _)) => cx.document_face(&format!("font-sample/{}/{}", face.family, face.style), || {
            let catalog = Arc::clone(catalog);
            move || {
                let data = catalog.load(&face).ok()?;
                Some(FaceBytes::new(data.bytes, data.index).coords(face.variations.iter().copied()))
            }
        }),
        // The catalog finds the installed fonts only: a bundled family is drawn from the database's own face.
        None => {
            let Some(face) = loaded.as_ref() else { return };
            cx.document_face(&format!("font-sample/{}/{}", face.family, face.style), || {
                let face = Arc::clone(face);
                move || Some(FaceBytes::new(Arc::from(face.file_data()), face.face_index()).coords(face.variations().iter().copied()))
            })
        }
    };
    let FaceState::Ready(font) = state else { return };
    let covers = |s: &str| !s.trim().is_empty() && loaded.as_ref().is_some_and(|f| s.chars().filter(|c| !c.is_whitespace()).all(|c| f.covers(c)));
    let text = text.filter(|t| covers(t)).unwrap_or(FALLBACK_SAMPLE);
    let area = if on_white {
        let paper = rect.shrink2(vec2(0.0, PAPER_INSET));
        if paper.is_positive() {
            cx.painter.rect_filled(paper, 2.0, SAMPLE_PAPER);
        }
        paper
    } else {
        rect
    };
    let pad = if on_white { PAPER_PAD } else { 0.0 };
    let color = DataColor::new(if on_white { Color32::BLACK } else { cx.ink() });
    let runs = [DocRun::new(text, font, area.height() * SAMPLE_SCALE, color)];
    let layout = cx.layout(&DocText { wrap: Some((area.width() - pad).max(0.0)), ..DocText::new(&runs).elide() });
    let at = egui::pos2(area.left() + pad, area.center().y - layout.galley.size().y / 2.0);
    cx.painter.galley(at, layout.galley.clone(), color.0);
}

/// A menu row's sample of `family` in its regular style; two samples of one family and paper are equal.
pub(crate) fn menu_sample(family: &str, on_white: bool) -> FacePaint {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (family, on_white).hash(&mut hasher);
    let family = family.to_owned();
    FacePaint::new(hasher.finish(), move |cx, rect| paint_family_sample(cx, rect, &family, "Regular", None, on_white))
}

/// The command that sets font `family` (and `style`, else the closest the family has) on what the
/// menus act on.
fn font_command(app: &VectorcraftApp, family: &str, style: Option<&str>) -> (&'static str, serde_json::Value) {
    let mut p = json!({"font": family});
    if let Some(s) = style {
        p["style"] = json!(s);
    }
    // After Select All in threaded text: every frame of the story.
    if let Some(first) = crate::panels::character::text_story(app).and_then(|story| story.first().copied()) {
        p["id"] = json!(first.0);
        p["story"] = json!(true);
        return ("text.setRangeStyle", p);
    }
    match crate::panels::character::text_editing(app) {
        Some((id, a, b)) => {
            p["id"] = json!(id.0);
            if b > a {
                p["start"] = json!(a);
                p["end"] = json!(b);
            }
            ("text.setRangeStyle", p)
        }
        None => {
            // With the Selection tool: every frame of each selected story.
            crate::panels::character::target_selected_stories(app, &mut p);
            ("text.setStyle", p)
        }
    }
}

/// The text the menus sample: the Type tool's selected characters, else the first line of the
/// first selected text object, at most a dozen characters.
pub(crate) fn sample_text(app: &VectorcraftApp) -> Option<String> {
    let doc = &app.session.active()?.doc;
    let text = match crate::panels::character::text_editing(app) {
        Some((id, a, b)) if b > a => match &doc.node(id)?.kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text().get(a..b).map(str::to_string),
            _ => None,
        },
        _ => app.session.active()?.selection.objects.iter().find_map(|id| match &doc.node(*id)?.kind {
            vectorcraft_doc::NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        }),
    }?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(12).collect())
}

#[cfg(test)]
mod tests;
