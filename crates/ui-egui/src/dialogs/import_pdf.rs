//! The Import PDF dialog: opening or placing a multi-page or password-protected PDF (or `.ai`)
//! asks which pages (a preview with page navigation), the box to crop to and the password.
//!
//! Fields: `mode` (`open` | `place`), `name`, `path?`, `pageCount`, `page` (the page shown, and the
//! one placed; 1-based), `allPages`, `range` and `linkPages` (open: Import PDF Pages as Links, for
//! a file read from a path and opened without a password; `null` until it is checked or cleared,
//! when it follows the pages picked: on for All, off for a range; picking the other of All and
//! Range sets it back to `null`; `__linkable` is false for a file `linkPages` does not take, such as
//! one over its size limit), `cropTo`, `textAs` (`text` | `outlines`; it starts from the `pdfTextAs`
//! preference for a PDF (`__pdf`; Editable for a `.ai` or `.ait` file), or from the `textAs` a place
//! gives (`__textAsGiven`), and OK keeps a PDF's choice there; a
//! linked place shows the file's own text, so placing linked leaves it as it is, and Outlines clears
//! Import PDF Pages as Links) and `password`. `__locked` is set
//! until the password is accepted: OK then unlocks the file, and with `__passwordOnly` (Place with
//! Show Import Options off) places its first page at once. `__place` keeps the other `file.place`
//! params (Link, Template, `at`…). The file's bytes stay out of the fields, in
//! [`crate::state::UiState::dialog_file`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_pdf::{Choice, CropTo, PdfError, TextAs};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "importPdf";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |d| if d.str("mode") == "place" { tl!("Place PDF") } else { tl!("Import PDF") }.into(), body, confirm, ..DialogSpec::FORM };

/// The longest side of the page preview (pt).
const PREVIEW: f32 = 200.0;
const LABEL_WIDTH: f32 = 70.0;

/// The file an import dialog reads (Import PDF, DXF Import Options).
#[derive(Debug)]
pub struct DialogFile {
    pub bytes: Vec<u8>,
    /// Unique per file the dialog opened (keys its preview cache).
    pub(super) token: u64,
}

impl DialogFile {
    pub fn new(bytes: Vec<u8>) -> Arc<Self> {
        static TOKEN: AtomicU64 = AtomicU64::new(0);
        Arc::new(Self { bytes, token: TOKEN.fetch_add(1, Ordering::Relaxed) })
    }
}

/// The file `file.place` params `p` name when it is one `wanted` takes by its extension (a path,
/// read through the app's services) or given as `dataBase64` → (name, bytes, path).
pub(super) fn place_source(app: &VectorcraftApp, p: &Value, wanted: impl Fn(&str) -> bool) -> Option<(String, Vec<u8>, Option<String>)> {
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    match (s("path"), s("dataBase64")) {
        (Some(path), _) if wanted(&fileio::extension(path)) => {
            let bytes = app.services.read.as_ref().and_then(|r| r(path).ok())?;
            Some((path.to_string(), bytes, Some(path.to_string())))
        }
        (None, Some(b64)) => Some((s("name").unwrap_or("Untitled").to_string(), vectorcraft_format::base64_decode(b64)?, None)),
        _ => None,
    }
}

/// The `file.place` params besides the file and `taken` (kept by an import dialog for its OK).
pub(super) fn other_place_params(p: &Value, taken: &[&str]) -> Value {
    let rest =
        p.as_object().into_iter().flatten().filter(|(k, _)| !matches!(k.as_str(), "path" | "name" | "dataBase64") && !taken.contains(&k.as_str()));
    Value::Object(rest.map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// `p` naming the dialog's file again: its path, or its name and bytes.
pub(super) fn with_file(p: Option<&Value>, d: &Dialog, file: &DialogFile) -> Value {
    let mut p = p.filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
    match d.fields.get("path").and_then(Value::as_str) {
        Some(path) => p["path"] = json!(path),
        None => {
            p["name"] = json!(d.str("name"));
            p["dataBase64"] = json!(vectorcraft_format::base64_encode(&file.bytes));
        }
    }
    p
}

/// A preview of `doc`'s first artboard, at most `size` points on its longer side, rendered once
/// per `key` (`None`: there is nothing to show).
pub(super) fn preview_texture(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    key: egui::Id,
    name: &str,
    size: f32,
    doc: impl FnOnce() -> Option<vectorcraft_engine::doc::Document>,
) -> Option<egui::TextureHandle> {
    if let Some(t) = ctx.data(|m| m.get_temp::<Option<egui::TextureHandle>>(key)) {
        return t;
    }
    let tex = doc().and_then(|doc| {
        let r = doc.artboards.first()?.rect;
        let px = size as f64 * ctx.pixels_per_point() as f64;
        Some(widgets::region_texture(ctx, &mut app.canvas.renderer, name, &doc, r, px))
    });
    ctx.data_mut(|m| m.insert_temp(key, tex.clone()));
    tex
}

/// `tex` fitted in a `size` square on the pasteboard, framed.
pub(super) fn show_preview(ui: &mut egui::Ui, tex: Option<&egui::TextureHandle>, size: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, t.pasteboard);
    if let Some(tex) = tex {
        let sz = tex.size_vec2();
        let ir = egui::Rect::from_center_size(r.center(), sz * (size / sz.x.max(sz.y)));
        ui.painter().image(tex.id(), ir, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        ui.painter().rect_stroke(ir, 0.0, egui::Stroke::new(1.0, t.divider), egui::StrokeKind::Outside);
    }
}

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when the
/// file is a PDF (or `.ai`/`.ait`) with several pages or a password; false when there's nothing to
/// ask (the caller goes on), including, when opening, a file that carries its VectorCraft document
/// and a `.ai`/`.ait` with its editing data.
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    let Some(format) = fileio::detect(name, bytes).map(|f| f.id).filter(|id| matches!(*id, "pdf" | "ai" | "ait")) else {
        return false;
    };
    // A file carrying its VectorCraft document (a saved .ai, or a PDF saved with Preserve Editing)
    // opens as that document, every artboard and layer: there are no pages to pick. So does a .ai
    // or .ait with its editing data, which holds them all (#6135); a PDF with editing data still
    // asks, as its pages are what it is. Placing still asks, since a place takes one page.
    // The file is read once for both checks.
    let opened = vectorcraft_pdf::open_pdf(bytes, None);
    if place.is_none() && opened.as_ref().is_ok_and(|o| o.editing().is_some() || (format != "pdf" && o.has_illustrator_data())) {
        return false;
    }
    let (pages, locked) = match opened.map(|o| o.page_count()) {
        Ok(count) if count > 1 => (count, false),
        Err(PdfError::NeedsPassword) => (0, true),
        _ => return false,
    };
    app.ui.dialog_file = Some(DialogFile::new(bytes.to_vec()));
    let mut fields = json!({
        "mode": if place.is_some() { "place" } else { "open" },
        "name": name,
        "pageCount": pages,
        "page": 1,
        "allPages": true,
        "range": "",
        "linkPages": null,
        "cropTo": CropTo::default().id(),
        // The PDF Text preference is for PDFs: a `.ai` or `.ait` file starts from Editable, as it
        // opens or is placed without the dialog.
        "textAs": if format == "pdf" { preferred_text_as(app) } else { TextAs::Text }.id(),
        "__pdf": format == "pdf",
        "password": "",
        "__locked": locked,
        "__linkable": fileio::pages_can_link(name, bytes),
    });
    if let Some(p) = path {
        fields["path"] = json!(p);
    }
    if let Some(p) = place {
        // A `textAs` the place gives starts the Text choice; one that isn't a choice stays with
        // the other params, for `file.place` to refuse.
        let given = p.get("textAs").and_then(Value::as_str).filter(|id| TextAs::IDS.iter().any(|i| i.eq_ignore_ascii_case(id)));
        if let Some(id) = given {
            fields["textAs"] = json!(text_as_of(id).id());
            fields["__textAsGiven"] = json!(true);
        }
        let taken: &[&str] = if given.is_some() { &["page", "crop", "password", "textAs"] } else { &["page", "crop", "password"] };
        fields["__place"] = other_place_params(p, taken);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    true
}

/// `file.place` with params `p` asks first (true) when they name a PDF (`.pdf`, `.ai`, `.ait`)
/// with several pages or a password and no `page`.
pub fn offer_place(app: &mut VectorcraftApp, p: &Value) -> bool {
    if p.get("page").is_some() {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| matches!(ext, "pdf" | "ai" | "ait")) else { return false };
    offer(app, &name, &bytes, path, Some(p))
}

/// [`offer_place`] when Show Import Options is off: asks (true) only for the password of an
/// encrypted PDF given no `password`, which nothing else opens, and places its first page (or
/// `page`) and crop box once it is unlocked; any other PDF is placed that way without asking.
pub fn offer_place_password(app: &mut VectorcraftApp, p: &Value) -> bool {
    // A `page` that isn't a page number is left for `file.place` to refuse.
    if p.get("password").is_some() || p.get("page").is_some_and(|v| v.as_u64().is_none_or(|n| n == 0)) {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| matches!(ext, "pdf" | "ai" | "ait")) else { return false };
    if !matches!(vectorcraft_pdf::open_pdf(&bytes, None).map(|_| ()), Err(PdfError::NeedsPassword)) {
        return false;
    }
    let offered = offer(app, &name, &bytes, path, Some(p));
    if let (true, Some(d)) = (offered, app.ui.dialog.as_mut()) {
        d.fields.insert("__passwordOnly".into(), json!(true));
        // A `page` and `crop` given are placed as given, for `file.place` to check.
        if let Some(place) = d.fields.get_mut("__place").and_then(Value::as_object_mut) {
            for k in ["page", "crop"] {
                if let Some(v) = p.get(k) {
                    place.insert(k.into(), v.clone());
                }
            }
        }
    }
    offered
}

fn page_count(d: &Dialog) -> usize {
    d.fields.get("pageCount").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// The page shown (1-based, within the file).
fn page(d: &Dialog) -> usize {
    (d.f64("page", 1.0).max(1.0) as usize).min(page_count(d).max(1))
}

fn crop(d: &Dialog) -> CropTo {
    CropTo::IDS.iter().position(|id| *id == d.str("cropTo")).and_then(|i| CropTo::ALL.get(i).copied()).unwrap_or_default()
}

fn password(d: &Dialog) -> Option<String> {
    Some(d.str("password")).filter(|p| !p.is_empty())
}

/// The labels of the Text choices, in the order of [`TextAs::ALL`].
const TEXT_AS_LABELS: [&str; 2] = ["Editable", "Outlines"];

/// The `textAs` choice `id` (any case); anything else is editable text.
fn text_as_of(id: &str) -> TextAs {
    TextAs::ALL.iter().zip(TextAs::IDS).find(|(_, i)| i.eq_ignore_ascii_case(id)).map(|(t, _)| *t).unwrap_or_default()
}

/// What a PDF's text becomes when an open or place from the app does not say: the `pdfTextAs`
/// preference.
pub(crate) fn preferred_text_as(app: &VectorcraftApp) -> TextAs {
    text_as_of(&app.session.prefs.pdf_text_as)
}

/// The `document.open` options `p` for the file `name` holding `bytes`, with the `pdfTextAs`
/// preference as its `textAs` when it is a PDF opened as Outlines and `p` has no `textAs`. A `.ai`
/// or `.ait` file opens as it says: Outlines would open it from its PDF content, without its
/// editing data. So does an open with `linkPages`, which refuses Outlines: a linked page shows the
/// file's own text.
pub(crate) fn with_preferred_text_as(app: &VectorcraftApp, name: &str, bytes: &[u8], p: &Value) -> Value {
    let given = p.get("textAs").is_some_and(|v| !v.is_null());
    let linked = p.get("linkPages").and_then(Value::as_bool) == Some(true);
    let options = p.is_object() || p.is_null();
    if given || linked || !options || preferred_text_as(app) == TextAs::Text || fileio::detect(name, bytes).map(|f| f.id) != Some("pdf") {
        return p.clone();
    }
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    p["textAs"] = json!(TextAs::Outlines.id());
    p
}

/// The `file.place` params `p` of a PDF the user places embedded (Link off, from bytes or with a
/// password) get the `pdfTextAs` preference as their `textAs` when it is Outlines and they have no
/// `textAs`. A linked page shows the file's own text, and a `.ai` or `.ait` file is placed as it
/// says.
pub(crate) fn place_with_preferred_text_as(app: &VectorcraftApp, p: &mut Value) {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    let path = p.get("path").and_then(Value::as_str);
    let name = path.or_else(|| p.get("name").and_then(Value::as_str)).unwrap_or_default();
    let linked = path.is_some() && p.get("link").and_then(Value::as_bool) != Some(false) && !given("password");
    if given("textAs") || linked || preferred_text_as(app) == TextAs::Text || fileio::extension(name) != "pdf" {
        return;
    }
    if let Some(o) = p.as_object_mut() {
        o.insert("textAs".into(), json!(TextAs::Outlines.id()));
    }
}

fn text_as(d: &Dialog) -> TextAs {
    text_as_of(&d.str("textAs"))
}

/// A place that links the page: from a path, with Link on, without a password (`file.place`
/// embeds a page read with a password) and without a `textAs` given with the place (`__textAsGiven`:
/// the Text choice then decides, and Outlines embeds the page). A linked page shows the file's own
/// text, so the Text choice does not apply to it.
fn places_linked(d: &Dialog) -> bool {
    d.str("mode") == "place"
        && d.fields.get("path").and_then(Value::as_str).is_some()
        && d.fields.get("__place").and_then(|p| p.get("link")).and_then(Value::as_bool) != Some(false)
        && password(d).is_none()
        && !d.bool("__textAsGiven")
}

/// Can the file's pages open linked: it was read from a path (a linked page is read from its
/// file again), `document.open`'s `linkPages` takes its size, and it opens without a password
/// (`linkPages` refuses one).
fn file_can_link(d: &Dialog) -> bool {
    d.fields.get("path").and_then(Value::as_str).is_some() && d.bool("__linkable") && password(d).is_none()
}

/// Can the pages open linked: [`file_can_link`], with the text kept editable (`linkPages`
/// refuses `textAs: outlines`).
fn can_link(d: &Dialog) -> bool {
    file_can_link(d) && text_as(d) == TextAs::Text
}

/// Keeps the Text choice OK confirmed in the `pdfTextAs` preference.
fn keep_text_as(app: &mut VectorcraftApp, text_as: TextAs) {
    if preferred_text_as(app) != text_as {
        // The open or place has already happened; a preference that can't be written only means
        // the next dialog starts from the old choice.
        let _ = app.run("prefs.set", json!({"key": "pdfTextAs", "value": text_as.id()}));
    }
}

/// Import PDF Pages as Links as checked or cleared, or, when it is `null`, as the pages picked
/// default it: on for All, off for a range.
fn link_pages(d: &Dialog) -> bool {
    d.fields.get("linkPages").and_then(Value::as_bool).unwrap_or_else(|| d.bool("allPages"))
}

/// Import PDF Pages as Links is checked, and can be.
pub(super) fn links_on(d: &Dialog) -> bool {
    link_pages(d) && can_link(d)
}

/// Picks All (true) or Range. A change of pick sets Import PDF Pages as Links back to that pick's
/// default; picking the one already picked keeps the box as it is.
fn pick_all_pages(d: &mut Dialog, all: bool) {
    if d.bool("allPages") != all {
        d.fields.insert("allPages".into(), json!(all));
        d.fields.insert("linkPages".into(), Value::Null);
    }
}

/// Records that `ui.dialog.set` sets `field` to `value`: a change of `allPages` sets Import PDF
/// Pages as Links back to its default, as a click on All or Range does.
pub(super) fn note_set(d: &mut Dialog, field: &str, value: &Value) {
    if field == "allPages"
        && let Some(all) = value.as_bool()
    {
        pick_all_pages(d, all);
    }
}

/// OK: unlock the file with the password, else open the pages or place the page.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let file = app.ui.dialog_file.clone().ok_or("the PDF is no longer loaded")?;
    let mut d = d.clone();
    if d.bool("__locked") {
        let info = vectorcraft_pdf::info(&file.bytes, password(&d).as_deref()).map_err(|e| e.to_string())?;
        d.fields.insert("__locked".into(), json!(false));
        d.fields.insert("pageCount".into(), json!(info.pages.len()));
        if info.pages.len() > 1 && !d.bool("__passwordOnly") {
            // Unlocked: now pick the pages.
            app.ui.dialog = Some(d);
            return Ok(Value::Null);
        }
    }
    let name = d.str("name");
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    let r = if d.str("mode") == "place" {
        let mut p = with_file(d.fields.get("__place"), &d, &file);
        // Asked only for the password: a `page` and `crop` given with the file stand.
        let [page_given, crop_given] = ["page", "crop"].map(|k| d.bool("__passwordOnly") && p.get(k).is_some());
        if !page_given {
            p["page"] = json!(page(&d));
        }
        if !crop_given {
            p["crop"] = json!(crop(&d).id());
        }
        if let Some(pw) = password(&d) {
            p["password"] = json!(pw);
        }
        // A `textAs` left with the other params is one `file.place` refuses; null leaves it out.
        if !places_linked(&d) && p.get("textAs").is_none_or(Value::is_null) {
            p["textAs"] = json!(text_as(&d).id());
        }
        crate::place::run_for_user(app, &p).map(|_| ())
    } else {
        let mut p = json!({"cropTo": crop(&d).id(), "password": password(&d), "textAs": text_as(&d).id()});
        let range = d.str("range");
        if !d.bool("allPages") && !range.trim().is_empty() {
            p["pages"] = json!(range);
        }
        if links_on(&d) {
            p["linkPages"] = json!(true);
        }
        io::open_document(app, &name, &file.bytes, path, &p).map(|_| ())
    };
    match r {
        Ok(()) => {
            // Asked only for the password, the dialog showed no Text choice to keep; the choice
            // for a `.ai` or `.ait` file is not kept for PDFs.
            if !places_linked(&d) && !d.bool("__passwordOnly") && d.bool("__pdf") {
                keep_text_as(app, text_as(&d));
            }
            // Opening may have asked something else (the profile of the document); only this
            // dialog closes.
            if app.ui.dialog.as_ref().is_some_and(|x| x.kind == KIND) {
                app.ui.dialog = None;
            }
            Ok(Value::Null)
        }
        Err(e) => {
            // Keep what was typed (the dialog stays open on errors such as a bad range).
            app.ui.dialog = Some(d);
            Err(e)
        }
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    if d.bool("__locked") {
        let name = d.str("name");
        let file = std::path::Path::new(&name).file_name().map_or(name.clone(), |f| f.to_string_lossy().into_owned());
        ui.label(egui::RichText::new(crate::i18n::fmt(tl!("“{file}” is protected by a password."), &[("file", &file)])).color(t.text));
        ui.add_space(8.0);
        password_row(ui, d);
        return false;
    }
    let count = page_count(d);
    let mut current = page(d);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            show_preview(ui, preview(app, ui.ctx(), d, current).as_ref(), PREVIEW);
            ui.add_space(6.0);
            // Page navigation: first, previous, the page number, next, last.
            ui.horizontal(|ui| {
                let nav = |ui: &mut egui::Ui, icon: &str, tip: &str, to: usize, enabled: bool| {
                    widgets::icon_button_enabled(ui, icon, tip, false, enabled, 22.0).clicked().then_some(to)
                };
                let go = [
                    nav(ui, "chevrons-left", tl!("First Page"), 1, current > 1),
                    nav(ui, "chevron-left", tl!("Previous Page"), current.saturating_sub(1).max(1), current > 1),
                ];
                if let Some(v) = widgets::range_field(ui, "pdf-page", current as f64, 1.0..=count.max(1) as f64, "", 0, 40.0) {
                    current = v as usize;
                }
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("of {count}"), &[("count", &count.to_string())])).color(t.text_dim));
                let go2 = [
                    nav(ui, "chevron-right", tl!("Next Page"), (current + 1).min(count), current < count),
                    nav(ui, "chevrons-right", tl!("Last Page"), count, current < count),
                ];
                if let Some(to) = go.into_iter().chain(go2).flatten().next() {
                    current = to;
                }
            });
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            widgets::label_row(ui, tl!("Crop To"), LABEL_WIDTH, |ui| {
                if let Some(i) = widgets::dropdown(ui, "pdf-crop", crop(d).label(), CropTo::LABELS, 130.0)
                    && let Some(id) = CropTo::IDS.get(i)
                {
                    d.fields.insert("cropTo".into(), json!(id));
                }
            });
            ui.add_space(4.0);
            let linked = places_linked(d);
            widgets::label_row(ui, tl!("Text"), LABEL_WIDTH, |ui| {
                let current = TextAs::ALL.iter().position(|t| *t == text_as(d)).and_then(|i| TEXT_AS_LABELS.get(i)).copied().unwrap_or_default();
                let picked = ui.add_enabled_ui(!linked, |ui| widgets::dropdown(ui, "pdf-text-as", current, &TEXT_AS_LABELS, 130.0)).inner;
                if let Some(t) = picked.and_then(|i| TextAs::ALL.get(i)) {
                    d.fields.insert("textAs".into(), json!(t.id()));
                }
            });
            if linked {
                widgets::dim_label(ui, tl!("Linked pages show the file's own text."));
            }
            if d.str("mode") != "place" {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(tl!("Pages")).color(t.text));
                let all = d.bool("allPages");
                if widgets::radio(ui, tl!("All"), all, true) {
                    pick_all_pages(d, true);
                }
                ui.horizontal(|ui| {
                    if widgets::radio(ui, tl!("Range:"), !all, true) {
                        pick_all_pages(d, false);
                    }
                    if super::form::text_edit_named(ui, d, "range", tl!("Range:"), 90.0).gained_focus() {
                        pick_all_pages(d, false);
                    }
                });
                widgets::dim_label(ui, tl!("e.g. 1-3, 5"));
                ui.add_space(10.0);
                if widgets::check(ui, tl!("Import PDF pages as links"), links_on(d), can_link(d)) {
                    d.fields.insert("linkPages".into(), json!(!link_pages(d)));
                }
                // Outlines alone dims the box.
                if text_as(d) == TextAs::Outlines && file_can_link(d) {
                    widgets::dim_label(ui, tl!("Linked pages show the file's own text."));
                }
            }
        });
    });
    d.fields.insert("page".into(), json!(current));
    false
}

fn password_row(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::label_row(ui, tl!("Password"), LABEL_WIDTH, |ui| {
        let mut s = d.str("password");
        let r = ui.add(egui::TextEdit::singleline(&mut s).password(true).align(egui::Align2::LEFT_CENTER).desired_width(180.0));
        if r.changed() {
            d.fields.insert("password".into(), json!(s));
        }
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
    });
}

/// Page `page` (1-based) cropped and with its text as the dialog says (a linked place: as
/// editable text, as before the choice), rendered once per file, page, box and text choice.
fn preview(app: &mut VectorcraftApp, ctx: &egui::Context, d: &Dialog, page: usize) -> Option<egui::TextureHandle> {
    let file = app.ui.dialog_file.clone()?;
    let crop = crop(d);
    let text_as = if places_linked(d) { TextAs::Text } else { text_as(d) };
    let key = egui::Id::unique(("pdf-preview", file.token, page, crop.id(), text_as.id()));
    let opts = fileio::LoadOptions { crop, password: password(d), text_as, ..Default::default() };
    // A page that can't be read shows an empty preview (and isn't read again).
    preview_texture(app, ctx, key, &format!("pdf-preview-{page}"), PREVIEW, || {
        fileio::page_document(&file.bytes, page - 1, &opts).ok().map(|(doc, _)| doc)
    })
}
