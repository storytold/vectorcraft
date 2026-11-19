//! SVG Options (Export As SVG, Save As / Save a Copy to .svg): profile, styling, fonts, images,
//! object ids, decimals, encoding, minify, responsive, embedded fonts, artboards (export) or
//! editing data (save), and a read-only view of the code. OK remembers the choices for next time and runs the export or save command with
//! them as `svg: {…}`. Show in Browser (`file.export.svg.browser`) writes the SVG the choices give
//! to a temporary folder and opens it in the default browser, leaving the dialog open.

use std::borrow::Cow;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, io, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("SVG Options").into(),
    body,
    confirm,
    ok: Some("OK"),
    min_width: 380.0,
    max_width: Some(560.0),
    inputs: |_| &["path", "showCode"],
    ..DialogSpec::FORM
};

/// `Dialog::kind` of this dialog.
pub const KIND: &str = "svgOptions";

/// The UI command of the dialog's Show in Browser button.
pub const BROWSER: &str = "file.export.svg.browser";

/// The folder in the system's temporary folder that Show in Browser's previews go to.
const PREVIEW_ROOT: &str = "VectorCraft SVG Preview";

/// What OK does: Export As SVG, Save As or Save a Copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Export,
    Save,
    SaveCopy,
}

impl Mode {
    fn id(self) -> &'static str {
        match self {
            Mode::Export => "export",
            Mode::Save => "save",
            Mode::SaveCopy => "copy",
        }
    }

    fn of(d: &Dialog) -> Self {
        match d.str("mode").as_str() {
            "save" => Mode::Save,
            "copy" => Mode::SaveCopy,
            _ => Mode::Export,
        }
    }
}

/// (option value, label) for each choice list.
const STYLING: [(&str, &str); 4] = [
    ("css", "Internal CSS"),
    ("presentation", "Presentation Attributes"),
    ("style", "Style Attributes"),
    ("entities", "Style Attributes (Entity References)"),
];
const FONTS: [(bool, &str); 2] = [(false, "SVG"), (true, "Convert To Outlines")];
const IMAGES: [(&str, &str); 2] = [("embed", "Embed"), ("link", "Link")];
const OBJECT_IDS: [(&str, &str); 3] = [("layerNames", "Layer Names"), ("minimal", "Minimal"), ("unique", "Unique")];
const PROFILES: [(&str, &str); 2] = [("svg11", "SVG 1.1"), ("tiny12", "SVG Tiny 1.2")];
const ENCODINGS: [(&str, &str); 3] = [("utf8", "Unicode (UTF-8)"), ("utf16", "Unicode (UTF-16)"), ("latin1", "ISO 8859-1")];

/// Dialog fields that are not SVG options.
const UI_KEYS: [&str; 5] = ["mode", "path", "showCode", "allArtboards", "range"];

/// The options a first SVG Options dialog starts from: the engine's defaults with Internal CSS
/// and Responsive on (as the reference app's Export As starts). Hidden layers are left to the
/// command (Save keeps them, Export leaves them out).
pub(super) fn first_use() -> Map<String, Value> {
    let mut m = serde_json::to_value(vectorcraft_svg::ExportOptions::default()).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
    m.remove("hiddenLayers");
    m.insert("styling".into(), json!("css"));
    m.insert("responsive".into(), json!(true));
    m.insert("useArtboards".into(), json!(true));
    m
}

/// Open SVG Options for `mode`, filled with the options used last (a save starts from the ones
/// the document was last saved with). `path`: where a save goes.
pub fn open(app: &mut VectorcraftApp, mode: Mode, path: Option<&str>) {
    let mut fields = first_use();
    // Fonts start from Document Setup → Type → Export.
    if app.session.active().is_some_and(|st| st.doc.setup.export_text == vectorcraft_doc::ExportText::Appearance) {
        fields.insert("outlineText".into(), json!(true));
    }
    // Only options saved with an SVG format are SVG options.
    let saved = app
        .session
        .active()
        .filter(|st| mode != Mode::Export && matches!(st.format, "svg" | "svgz"))
        .map(|st| Value::Object(vectorcraft_engine::cmd::fileio::remembered_save_options(st)));
    for last in [&app.ui.svg_options].into_iter().chain(saved.as_ref()) {
        if let Some(o) = last.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    fields.insert("mode".into(), json!(mode.id()));
    fields.insert("allArtboards".into(), json!(true));
    fields.insert("range".into(), json!(""));
    if let Some(p) = path {
        fields.insert("path".into(), json!(p));
    }
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

/// The SVG options the dialog's fields describe (artboards only when exporting, editing data
/// only when saving).
fn options(d: &Dialog) -> Value {
    let mode = Mode::of(d);
    let mut o: Map<String, Value> =
        d.fields.iter().filter(|(k, _)| !UI_KEYS.contains(&k.as_str()) && !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    if mode == Mode::Export {
        o.remove("preserveEditing");
        if d.bool("useArtboards") {
            let range = d.str("range");
            let range = range.trim();
            o.insert("range".into(), json!(if d.bool("allArtboards") || range.is_empty() { "all" } else { range }));
        }
    } else {
        o.remove("useArtboards");
    }
    Value::Object(o)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let opts = options(d);
    let selection = d.bool(super::export_as::SELECTION_FIELD);
    if Mode::of(d) == Mode::Export && d.bool("useArtboards") && !selection {
        let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
        let pick: vectorcraft_engine::cmd::fileio::ArtboardPick = serde_json::from_value(opts.clone()).map_err(|e| e.to_string())?;
        pick.resolve(n)?;
    }
    // Remembered for next time, without the document's artboard range. Export Selection shows no
    // Use Artboards, so the one remembered before stays.
    let mut last = opts.clone();
    if let Some(o) = last.as_object_mut() {
        o.remove("range");
        if selection {
            match app.ui.svg_options.get("useArtboards") {
                Some(v) => o.insert("useArtboards".into(), v.clone()),
                None => o.remove("useArtboards"),
            };
        }
    }
    app.ui.svg_options = last;
    app.ui.dialog = None;
    let mut params = json!({ "svg": opts });
    if let Some(p) = d.fields.get("path").filter(|p| p.is_string()) {
        params["path"] = p.clone();
    }
    let params = super::export_as::exported(d, params);
    let id = match Mode::of(d) {
        Mode::Export => "file.export.svg",
        Mode::Save => "file.saveAs",
        Mode::SaveCopy => "file.saveCopy",
    };
    app.run(id, params)
}

/// One labelled choice row: the dropdown sets `d.fields[key]` to the chosen value. `forced`: the
/// value the export uses whatever is chosen (shown, the dropdown disabled).
fn choice<V: Into<Value> + Copy>(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, choices: &[(V, &str)], forced: Option<V>) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(label).color(t.text_dim));
    let current = forced.map(Into::into).or_else(|| d.fields.get(key).cloned()).unwrap_or(Value::Null);
    let shown = choices.iter().find(|(v, _)| (*v).into() == current).or(choices.first()).map_or("", |(_, l)| *l);
    let labels: Vec<&str> = choices.iter().map(|(_, l)| *l).collect();
    let picked = ui.add_enabled_ui(forced.is_none(), |ui| widgets::dropdown(ui, key, shown, &labels, 250.0)).inner;
    if let Some((v, _)) = picked.and_then(|i| choices.get(i)) {
        d.fields.insert(key.into(), (*v).into());
    }
    ui.end_row();
}

/// A checkbox bound to `d.fields[key]`.
fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, enabled: bool) {
    if widgets::check(ui, label, d.bool(key), enabled) {
        d.fields.insert(key.into(), json!(!d.bool(key)));
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    option_fields(ui, d, false);
    // Export Selection writes the selection's crop, without artboards to choose.
    if Mode::of(d) == Mode::Export {
        if !d.bool(super::export_as::SELECTION_FIELD) {
            artboards(app, ui, d);
        }
    } else {
        check(ui, d, "preserveEditing", tl!("Preserve Editing Capabilities"), true);
    }
    ui.add_space(10.0);
    let show = d.bool("showCode");
    let mut browser = false;
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, if show { tl!("Hide Code") } else { tl!("Show Code") }).clicked() {
            d.fields.insert("showCode".into(), json!(!show));
        }
        let b = ui.add_enabled(!io::is_web(app), |ui: &mut egui::Ui| widgets::secondary_button(ui, tl!("Show in Browser")));
        browser = b.on_disabled_hover_text(tl!("Needs the desktop app")).clicked();
    });
    if browser && let Err(e) = app.run(BROWSER, preview_params(d)) {
        app.status(e);
    }
    if show {
        ui.add_space(6.0);
        code_view(app, ui, d);
    }
    false
}

/// The SVG options: styling, fonts, images, object ids, decimals, minify, responsive, `<tspan>`s
/// and metadata. `screens` (Export for Screens' Format Settings) leaves out Images: a linked
/// image is another file.
pub(super) fn option_fields(ui: &mut egui::Ui, d: &mut Dialog, screens: bool) {
    let t = Tokens::get(ui.ctx());
    // SVG Tiny has presentation attributes alone and no web fonts.
    let tiny = d.str("profile") == "tiny12";
    egui::Grid::new("svg-options").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        choice(ui, d, "profile", tl!("SVG Profile:"), &PROFILES, None);
        choice(ui, d, "styling", tl!("Styling:"), &STYLING, tiny.then_some("presentation"));
        choice(ui, d, "outlineText", tl!("Font:"), &FONTS, None);
        if !screens {
            choice(ui, d, "images", tl!("Images:"), &IMAGES, None);
        }
        choice(ui, d, "objectIds", tl!("Object IDs:"), &OBJECT_IDS, None);
        widgets::field_label(ui, egui::RichText::new(tl!("Decimal:")).color(t.text_dim));
        let decimals = d.f64("decimals", 3.0);
        if let Some(v) = widgets::spin_plain(ui, "svg-decimals", decimals, "", 0, 70.0, 1.0, 1.0, &[]) {
            let (lo, hi) = (*vectorcraft_svg::DECIMALS.start() as f64, *vectorcraft_svg::DECIMALS.end() as f64);
            d.fields.insert("decimals".into(), json!(v.round().clamp(lo, hi) as u8));
        }
        ui.end_row();
        choice(ui, d, "encoding", tl!("Encoding:"), &ENCODINGS, None);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        check(ui, d, "minify", tl!("Minify"), true);
        ui.add_space(12.0);
        check(ui, d, "responsive", tl!("Responsive"), true);
    });
    check(ui, d, "embedFonts", tl!("Embed Fonts (Glyphs Used)"), !d.bool("outlineText") && !tiny);
    check(ui, d, "fewerTspans", tl!("Fewer <tspan> Elements"), !d.bool("outlineText"));
    check(ui, d, "metadata", tl!("Include Metadata"), true);
    if !screens {
        check(ui, d, "variableData", tl!("Include Variable Data"), true);
    }
}

/// Use Artboards: all of them or a range; off = the bounds of all art.
fn artboards(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let count = app.session.active().map_or(0, |st| st.doc.artboards.len());
    ui.add_space(4.0);
    check(ui, d, "useArtboards", tl!("Use Artboards"), count > 0);
    let on = d.bool("useArtboards") && count > 0;
    ui.add_enabled_ui(on, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            let all = d.bool("allArtboards");
            if ui.radio(all, tl!("All")).clicked() {
                d.fields.insert("allArtboards".into(), json!(true));
            }
            if ui.radio(!all, tl!("Range:")).clicked() {
                d.fields.insert("allArtboards".into(), json!(false));
            }
            let mut range = d.str("range");
            let edit = egui::TextEdit::singleline(&mut range).align(egui::Align2::LEFT_CENTER).hint_text(format!("1-{count}")).desired_width(90.0);
            if ui.add_enabled(!all, edit).changed() {
                d.fields.insert("range".into(), json!(range));
            }
        });
    });
}

/// The SVG options the dialog `d`'s OK writes with: [`options`], with hidden layers kept (not
/// displayed) when OK saves, as a save keeps them unless told otherwise.
fn written_options(d: &Dialog) -> Value {
    let mut o = options(d);
    if Mode::of(d) != Mode::Export
        && let Some(m) = o.as_object_mut()
    {
        m.entry("hiddenLayers").or_insert(json!(true));
    }
    o
}

/// The params of [`BROWSER`] for the dialog `d`: the SVG options OK writes with
/// ([`written_options`]), and `exportSelection` when Export Selection opened it.
fn preview_params(d: &Dialog) -> Value {
    super::export_as::exported(d, json!({ "svg": written_options(d) }))
}

/// The SVG file that the SVG options `svg` write for one page of the document `st`, and the
/// document it is drawn from. With `selection` (Export Selection) the page is the selection alone,
/// cropped to it. Otherwise it is the first artboard the options choose, checked against the
/// document's artboards as OK checks them, or what the options write without artboards (also for
/// a document that has none). Linked scripts and images are checked against the read roots in
/// force first.
fn encode_page<'a>(
    st: &'a vectorcraft_engine::DocState,
    svg: &Value,
    selection: bool,
) -> Result<(Cow<'a, vectorcraft_doc::Document>, fileio::Encoded), String> {
    let mut opts = svg.clone();
    if let Some(o) = opts.as_object_mut() {
        if selection {
            // Export Selection writes one crop, without artboards to choose.
            for k in fileio::ARTBOARD_PARAMS.iter().chain(&["useArtboards"]) {
                o.remove(*k);
            }
        } else if st.doc.artboards.is_empty() {
            for k in fileio::ARTBOARD_PARAMS {
                o.remove(k);
            }
        } else {
            let pick: fileio::ArtboardPick = serde_json::from_value(Value::Object(o.clone())).map_err(|e| e.to_string())?;
            if let Some(first) = pick.resolve(st.doc.artboards.len())?.and_then(|v| v.first().copied()) {
                for k in fileio::ARTBOARD_PARAMS {
                    o.remove(k);
                }
                o.insert("artboard".into(), json!(first));
            }
        }
    }
    let params = json!({ "svg": opts });
    fileio::check_linked_scripts("document.export", "svg", st, &params).map_err(|e| e.to_string())?;
    let doc = if selection {
        Cow::Owned(fileio::selection_doc(st, "document.exportSelection", false).map_err(|e| e.to_string())?.0)
    } else {
        Cow::Borrowed(&*st.doc)
    };
    fileio::check_linked_images("document.export", "svg", st, &doc, &params).map_err(|e| e.to_string())?;
    let enc = fileio::encode_all(&doc, "svg", &params).map_err(|e| e.to_string())?;
    Ok((doc, enc))
}

/// `file.export.svg.browser {svg?, exportSelection?}` (SVG Options' Show in Browser): the SVG file
/// that the SVG options `svg` write for one page ([`encode_page`]), written with its linked images
/// to a new folder inside `VectorCraft SVG Preview` in the system's temporary folder and opened in
/// the default browser (desktop) → `{path, warnings}`. Nothing is written to the chosen
/// destination, and the document, its path and the remembered choices are left as they were.
/// When the export has warnings, the status bar shows them as [`io::report_saved`] shows a save's
/// notes. Nothing opens when a check, the encoding or a write fails, and a failed write removes
/// the new folder.
pub(crate) fn browser_preview(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if io::is_web(app) {
        return Err("no file system here".into());
    }
    app.refuse_automation(BROWSER)?;
    let (selection, p) = io::take_selection(p)?;
    let svg = p.get("svg").cloned().unwrap_or_else(|| json!({}));
    let (path, warnings) = {
        let st = app.session.active().ok_or("no document")?;
        let stem = fileio::file_stem(&st.doc.title);
        let (doc, enc) = encode_page(st, &svg, selection)?;
        let dir = super::save_for_web::temp_folder_under(PREVIEW_ROOT)?;
        let target = format!("{dir}/{}.svg", super::save_for_web::preview_name(&stem));
        let named = enc.named(&doc, &target);
        for (p, bytes) in &named {
            io::write_to(&mut app.services, p, bytes).inspect_err(|_| super::save_for_web::remove_preview_folder(&dir))?;
        }
        (named.first().map_or(target, |(p, _)| p.clone()), enc.notes())
    };
    app.open_url(&io::file_url(&path));
    let r = json!({ "path": path, "warnings": warnings });
    if !warnings.is_empty() {
        io::report_saved(app, &path, &r);
    }
    Ok(r)
}

/// The SVG the current options write for the first chosen artboard, or under Export Selection for
/// the selection alone, cropped to it (read-only, selectable), encoded again only when the
/// options, the document or the selection change.
fn code_view(app: &VectorcraftApp, ui: &mut egui::Ui, d: &Dialog) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    let selection = d.bool(super::export_as::SELECTION_FIELD);
    let opts = written_options(d);
    // One cached text, replaced when the document, the options or the selection change.
    let key = egui::Id::unique("svg-code");
    // The read roots in force and the document's folder decide whether the text may be shown.
    let roots = vectorcraft_engine::file_access::current().map(|r| r.describe());
    let selected = selection.then(|| st.selection.objects.clone());
    let sig = egui::Id::unique((st.uid, st.revision, opts.to_string(), roots, st.path.as_deref(), selected)).value();
    let code: Arc<String> = match ui.ctx().data(|m| m.get_temp::<(u64, Arc<String>)>(key)).filter(|(s, _)| *s == sig) {
        Some((_, c)) => c,
        None => {
            const MAX: usize = 200_000;
            let mut text = match encode_page(st, &opts, selection) {
                // Shown as text whatever the file's encoding.
                Ok((_, enc)) => enc
                    .files
                    .into_iter()
                    .next()
                    .map(|(_, b)| vectorcraft_svg::text_of(&b).map_or_else(|_| String::from_utf8_lossy(&b).into_owned(), |t| t.into_owned()))
                    .unwrap_or_default(),
                Err(e) => e,
            };
            if text.len() > MAX {
                let cut = (0..=MAX).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
                text.truncate(cut);
                text.push_str("\n…");
            }
            let c = Arc::new(text);
            ui.ctx().data_mut(|m| m.insert_temp(key, (sig, c.clone())));
            c
        }
    };
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::both().max_height(260.0).max_width(520.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut code.as_str())
                    .font(theme::mono(11.5))
                    .text_color(t.text)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(12),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;

    type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

    /// An app with two artboards and a rectangle whose save dialog answers `picked` and whose
    /// writer records what it is given.
    fn app(picked: &str) -> (VectorcraftApp, Written) {
        let written = Written::default();
        let w = written.clone();
        let picked = picked.to_string();
        let services = Services {
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            pick_save: Some(Box::new(move |_: &crate::FilePick| Some(picked.clone()))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 120, "height": 80, "artboards": 2})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
        (app, written)
    }

    /// One headless frame of the dialog layer.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn text(w: &Written, i: usize) -> String {
        String::from_utf8(w.borrow()[i].1.clone()).unwrap()
    }

    type Opened = Rc<RefCell<Vec<String>>>;

    /// [`app`] whose browser records each URL it is asked to open. Its previews go to a temporary
    /// folder of the test's own ([`preview_tmp`]), not the system's.
    fn browsing(picked: &str) -> (VectorcraftApp, Written, Opened) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tmp = vectorcraft_testkit::temp_dir(&format!("svg-browser-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        super::super::save_for_web::TEST_TMP.set(Some(tmp));
        let (mut app, written) = app(picked);
        let opened = Opened::default();
        let o = opened.clone();
        app.services.open_url = Some(Box::new(move |u: &str| o.borrow_mut().push(u.to_string())));
        (app, written, opened)
    }

    /// The SVG preview folder inside this test's temporary folder ([`browsing`]), with `/` between
    /// its parts.
    fn preview_tmp() -> String {
        let tmp = super::super::save_for_web::TEST_TMP.with_borrow(Clone::clone).expect("browsing sets the temporary folder");
        tmp.join(PREVIEW_ROOT).to_string_lossy().replace('\\', "/")
    }

    /// Show in Browser for the open dialog's current choices, as its button runs it.
    fn show_in_browser(app: &mut VectorcraftApp) -> Result<Value, String> {
        let p = preview_params(app.ui.dialog.as_ref().expect("SVG Options open"));
        app.run(BROWSER, p)
    }

    /// Show in Browser writes the SVG of the dialog's current, unconfirmed choices for the first
    /// artboard of the range to a new temporary folder and opens that file once. The dialog stays
    /// open, nothing is remembered, and OK then writes the same SVG to the chosen file.
    #[test]
    fn show_in_browser_opens_what_ok_writes_without_confirming() {
        let (mut app, written, opened) = browsing("/tmp/out.svg");
        app.run("file.export.svg", Value::Null).unwrap();
        frame(&mut app);
        let f = &mut app.ui.dialog.as_mut().unwrap().fields;
        f.insert("decimals".into(), json!(1));
        f.insert("allArtboards".into(), json!(false));
        f.insert("range".into(), json!("2"));
        let remembered = app.ui.svg_options.clone();
        let r = show_in_browser(&mut app).unwrap_or_else(|e| panic!("{e}"));
        let path = r["path"].as_str().unwrap().to_string();
        assert!(path.starts_with(&format!("{}/", preview_tmp())) && path.ends_with("/Untitled-1.svg"), "{path}");
        assert_eq!(*opened.borrow(), [io::file_url(&path)]);
        assert_eq!(written.borrow().len(), 1);
        assert_eq!(written.borrow()[0].0, path);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "the dialog stays open");
        assert_eq!(app.ui.svg_options, remembered, "nothing is remembered");
        assert!(app.session.active().unwrap().path.is_none());
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(written.borrow()[1].0, "/tmp/out.svg");
        assert_eq!(text(&written, 0), text(&written, 1), "the preview is the file OK writes");
        assert_eq!(opened.borrow().len(), 1);
    }

    /// A range the document has no artboard for is refused, as OK refuses it: nothing is written
    /// or opened, and the dialog stays open.
    #[test]
    fn show_in_browser_refuses_a_range_the_document_lacks() {
        let (mut app, written, opened) = browsing("/tmp/out.svg");
        app.run("file.export.svg", Value::Null).unwrap();
        let f = &mut app.ui.dialog.as_mut().unwrap().fields;
        f.insert("allArtboards".into(), json!(false));
        f.insert("range".into(), json!("5"));
        assert!(show_in_browser(&mut app).is_err());
        assert!(written.borrow().is_empty() && opened.borrow().is_empty());
        assert!(app.ui.dialog.is_some());
    }

    /// A write that fails opens nothing and removes the folder made for that preview.
    #[test]
    fn show_in_browser_removes_its_folder_when_a_write_fails() {
        let (mut app, _, opened) = browsing("/tmp/out.svg");
        app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("disk full".to_string())));
        app.run("file.export.svg", Value::Null).unwrap();
        assert!(show_in_browser(&mut app).unwrap_err().contains("disk full"));
        assert!(opened.borrow().is_empty());
        let left: Vec<_> = std::fs::read_dir(preview_tmp()).map(|d| d.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }

    /// Under Export Selection the preview is the selection alone, cropped to it, as OK writes it.
    #[test]
    fn show_in_browser_under_export_selection_shows_the_selection() {
        let (mut app, written, opened) = browsing("/tmp/selection.svg");
        app.run("shape.rectangle", json!({"x": 70, "y": 40, "width": 20, "height": 20})).unwrap();
        app.run("document.exportSelection", Value::Null).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool(super::super::export_as::SELECTION_FIELD));
        show_in_browser(&mut app).unwrap_or_else(|e| panic!("{e}"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(written.borrow()[1].0, "/tmp/selection.svg");
        assert_eq!(text(&written, 0), text(&written, 1));
        assert_eq!(text(&written, 0).matches("<rect").count(), 1, "{}", text(&written, 0));
        assert_eq!(opened.borrow().len(), 1);
    }

    /// From Save As, the preview leaves the document's path and unsaved state alone, and OK still
    /// saves to the chosen file.
    #[test]
    fn show_in_browser_from_save_as_leaves_the_document_unsaved() {
        let (mut app, written, opened) = browsing("/tmp/doc.svg");
        app.run("file.saveAs", Value::Null).unwrap();
        let dirty = app.session.active().unwrap().is_dirty();
        show_in_browser(&mut app).unwrap_or_else(|e| panic!("{e}"));
        let st = app.session.active().unwrap();
        assert_eq!((st.path.as_deref(), st.is_dirty()), (None, dirty));
        assert_eq!(opened.borrow().len(), 1);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(written.borrow()[1].0, "/tmp/doc.svg");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.svg"));
    }

    /// From Save As, a hidden layer is in the preview, kept hidden, as the save keeps it. The two
    /// differ only in the title: the save takes it from the file's name, the preview from the
    /// document's.
    #[test]
    fn show_in_browser_from_save_as_keeps_hidden_layers_as_the_save_does() {
        let (mut app, written, _) = browsing("/tmp/doc.svg");
        let notes = app.run("layer.new", json!({"name": "Notes"})).unwrap()["id"].clone();
        app.run("shape.ellipse", json!({"x": 60, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("layer.setProps", json!({"id": notes, "visible": false})).unwrap();
        app.run("file.saveAs", Value::Null).unwrap();
        show_in_browser(&mut app).unwrap_or_else(|e| panic!("{e}"));
        super::super::confirm(&mut app).unwrap();
        assert!(text(&written, 0).contains("id=\"Notes\""), "{}", text(&written, 0));
        assert_eq!(text(&written, 0).replace("<title>Untitled-1</title>", "<title>doc</title>"), text(&written, 1));
    }

    /// A document with no artboards is previewed by the bounds of its art, as the SVG writer
    /// writes it, rather than refused for its artboard range.
    #[test]
    fn show_in_browser_previews_a_document_without_artboards() {
        let (mut app, written, opened) = browsing("/tmp/out.svg");
        app.run("file.export.svg", Value::Null).unwrap();
        std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).artboards.clear();
        show_in_browser(&mut app).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(vectorcraft_testkit::svg_shapes(&text(&written, 0)), 1, "{}", text(&written, 0));
        assert_eq!(opened.borrow().len(), 1);
    }

    /// The web build has no file to open, and a control or MCP request may not open another app:
    /// both are refused before anything is written or opened.
    #[test]
    fn show_in_browser_is_refused_on_the_web_and_for_automation() {
        let (mut app, written, opened) = browsing("/tmp/out.svg");
        app.run("file.export.svg", Value::Null).unwrap();
        app.ui.answering_control = true;
        assert!(show_in_browser(&mut app).unwrap_err().contains("opens another app"));
        app.ui.answering_control = false;
        assert!(crate::menus::enabled(&app, BROWSER));
        app.services.download = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
        assert!(!crate::menus::enabled(&app, BROWSER), "the command palette and menus show it disabled on the web");
        assert!(show_in_browser(&mut app).is_err());
        assert!(written.borrow().is_empty() && opened.borrow().is_empty());
    }

    /// SVG previews have their own folder: making one removes only old SVG preview folders and
    /// leaves Save for Web's alone.
    #[test]
    fn svg_preview_folders_leave_save_for_web_previews_alone() {
        let tmp = vectorcraft_testkit::temp_dir("svg-preview-folders");
        let web = super::super::save_for_web::preview_folder_in(&tmp, std::time::Duration::from_secs(3600)).unwrap();
        let old = super::super::save_for_web::preview_folder_under(&tmp, PREVIEW_ROOT, std::time::Duration::from_secs(3600)).unwrap();
        let new = super::super::save_for_web::preview_folder_under(&tmp, PREVIEW_ROOT, std::time::Duration::ZERO).unwrap();
        assert!(new.contains("/VectorCraft SVG Preview/"), "{new}");
        assert!(std::path::Path::new(&web).is_dir(), "Save for Web's preview is kept");
        assert!(!std::path::Path::new(&old).exists(), "the old SVG preview is removed");
        assert!(std::path::Path::new(&new).is_dir());
    }

    #[test]
    fn export_as_svg_opens_the_options_and_remembers_them() {
        let (mut app, written) = app("/tmp/out.svg");
        app.run("file.export.svg", Value::Null).unwrap();
        let d = app.ui.dialog.clone().expect("SVG Options opened");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.str("styling"), d.bool("responsive"), d.str("mode")), ("css".into(), true, "export".into()), "first-use defaults");
        frame(&mut app);
        // Show Code draws the SVG the options write.
        app.ui.dialog.as_mut().unwrap().fields.insert("showCode".into(), json!(true));
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
        let f = &mut app.ui.dialog.as_mut().unwrap().fields;
        f.insert("decimals".into(), json!(1));
        f.insert("allArtboards".into(), json!(false));
        f.insert("range".into(), json!("1"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let svg = text(&written, 0);
        assert_eq!(written.borrow()[0].0, "/tmp/out.svg");
        assert!(svg.contains("<style>") && !svg.contains(" width=\"120\""), "{svg}");
        assert_eq!(app.ui.svg_options["decimals"], 1);
        assert!(app.ui.svg_options.get("range").is_none(), "the range belongs to the document");
        // Next time the dialog starts from these choices.
        app.run("file.export.svg", Value::Null).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("decimals", 0.0), 1.0);
    }

    /// Show Code under read roots draws no SVG of a document that links a JavaScript file outside
    /// them: it shows the error, which names no file or folder. Without roots it shows the link.
    #[test]
    fn show_code_leaves_out_a_linked_javascript_file_outside_the_read_roots() {
        let tmp = std::env::temp_dir();
        let root = tmp.join(format!("vc-svg-code-root-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = tmp.join(format!("vc-svg-code-away-{}", std::process::id())).join("menu.js").to_string_lossy().into_owned();
        let (mut app, _) = app("/tmp/code.svg");
        app.run("svgInteractivity.addFile", json!({"file": script})).unwrap();
        app.run("file.export.svg", Value::Null).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("showCode".into(), json!(true));
        let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap().unwrap();
        // The text the code view shows after two frames in one context, with `roots` in force.
        let code = |app: &mut VectorcraftApp, roots: Option<&Arc<vectorcraft_engine::file_access::AutomationRoots>>| {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            for _ in 0..2 {
                let mut out =
                    vectorcraft_engine::file_access::confine(roots, || ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx())));
                out.textures_delta.clear();
            }
            ctx.data(|m| m.get_temp::<(u64, Arc<String>)>(egui::Id::unique("svg-code"))).map(|(_, c)| c.to_string()).unwrap_or_default()
        };
        let confined = code(&mut app, Some(&roots));
        assert!(confined.contains("a linked JavaScript file is outside the read roots"), "{confined}");
        assert!(!confined.contains("menu.js") && !confined.contains("vc-svg-code"), "{confined}");
        let shown = code(&mut app, None);
        assert!(shown.contains(&format!("<script xlink:href=\"{script}\"/>")), "{shown}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Show Code under read roots draws no SVG that would name a linked image's file outside them
    /// (Images: Link): it shows the error, which names no file or folder. Embedded pixels are
    /// shown, and without roots the link is shown.
    #[test]
    fn show_code_leaves_out_a_linked_image_file_outside_the_read_roots() {
        let tmp = std::env::temp_dir();
        let root = tmp.join(format!("vc-svg-image-root-{}", std::process::id()));
        let away = tmp.join(format!("vc-svg-image-away-{}", std::process::id()));
        for dir in [&root, &away] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let photo = away.join("photo.png").to_string_lossy().into_owned();
        let mut png = vec![];
        image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&photo, png).unwrap();
        let (mut app, _) = app("/tmp/code.svg");
        app.run("file.place", json!({"path": photo, "link": true, "at": [50, 50]})).unwrap();
        app.run("file.export.svg", Value::Null).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("showCode".into(), json!(true));
        let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap().unwrap();
        // The text the code view shows after two frames in one context, with `roots` in force.
        let code = |app: &mut VectorcraftApp, roots: Option<&Arc<vectorcraft_engine::file_access::AutomationRoots>>| {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx);
            for _ in 0..2 {
                let mut out =
                    vectorcraft_engine::file_access::confine(roots, || ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx())));
                out.textures_delta.clear();
            }
            ctx.data(|m| m.get_temp::<(u64, Arc<String>)>(egui::Id::unique("svg-code"))).map(|(_, c)| c.to_string()).unwrap_or_default()
        };
        let embedded = code(&mut app, Some(&roots));
        assert!(embedded.contains("data:image/png;base64,") && !embedded.contains(&photo), "{embedded}");
        app.ui.dialog.as_mut().unwrap().fields.insert("images".into(), json!("link"));
        let confined = code(&mut app, Some(&roots));
        assert!(confined.contains("a linked image file is outside the read roots"), "{confined}");
        assert!(!confined.contains("photo.png") && !confined.contains("vc-svg-image"), "{confined}");
        let shown = code(&mut app, None);
        assert!(shown.contains(&format!("xlink:href=\"{photo}\"")), "{shown}");
        for dir in [root, away] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Show Code under Export Selection shows the SVG that OK writes: the selection alone, cropped
    /// to it with what its effects draw, not the whole document.
    #[test]
    fn show_code_under_export_selection_shows_the_selection() {
        let (mut app, written) = app("/tmp/selection.svg");
        app.run("shape.rectangle", json!({"x": 70, "y": 40, "width": 20, "height": 20})).unwrap();
        app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
        app.run("document.exportSelection", Value::Null).unwrap();
        let d = app.ui.dialog.as_mut().expect("SVG Options opened");
        assert_eq!(d.kind, KIND);
        assert!(d.bool(super::super::export_as::SELECTION_FIELD));
        d.fields.insert("showCode".into(), json!(true));
        // The text the code view shows after two frames in one context.
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        for _ in 0..2 {
            let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
        }
        let code = ctx.data(|m| m.get_temp::<(u64, Arc<String>)>(egui::Id::unique("svg-code"))).map(|(_, c)| c.to_string()).unwrap_or_default();
        super::super::confirm(&mut app).unwrap();
        let svg = text(&written, 0);
        assert_eq!(written.borrow()[0].0, "/tmp/selection.svg");
        assert_eq!(code, svg, "Show Code shows what OK writes");
        // The shadow reaches past the 21-point square, and the other rectangle is left out.
        assert!(!svg.contains("viewBox=\"0 0 120 80\"") && !svg.contains("viewBox=\"0 0 21 21\""), "{svg}");
        assert!(!svg.contains("width=\"40\""), "{svg}");
        assert_eq!(svg.matches("<rect").count(), 1, "{svg}");
        assert!(svg.contains("width=\"20\" height=\"20\"") && svg.contains("<feOffset"), "{svg}");
    }

    /// A dialog a save panel opened carries the dialog layer's own `__` fields; the save is given
    /// only the SVG options, so it is not refused for an unknown key.
    #[test]
    fn save_as_svg_from_a_save_panel_passes_only_the_options() {
        let (mut app, written) = app("/tmp/panel.svg");
        app.run("file.saveAs", Value::Null).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert(crate::dialogs::AFTER_PANEL.into(), json!(true));
        assert!(options(app.ui.dialog.as_ref().unwrap()).get(crate::dialogs::AFTER_PANEL).is_none());
        let d = app.ui.dialog.take().unwrap();
        confirm(&mut app, &d).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(written.borrow()[0].0, "/tmp/panel.svg");
    }

    #[test]
    fn all_artboards_export_one_file_each() {
        let (mut app, written) = app("/tmp/set.svg");
        app.run("file.export.svg", Value::Null).unwrap();
        super::super::confirm(&mut app).unwrap();
        let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(names, ["/tmp/set-Artboard-1.svg", "/tmp/set-Artboard-2.svg"]);
    }

    #[test]
    fn save_as_svg_asks_for_options_then_save_reuses_them() {
        let (mut app, written) = app("/tmp/doc.svg");
        let r = app.run("file.saveAs", Value::Null).unwrap();
        assert_eq!(r["dialog"], KIND);
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.str("mode"), d.str("path")), ("save".into(), "/tmp/doc.svg".into()));
        frame(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("styling".into(), json!("style"));
        super::super::confirm(&mut app).unwrap();
        assert!(text(&written, 0).contains("style=\"fill:"));
        let st = app.session.active().unwrap();
        assert_eq!(st.path.as_deref(), Some("/tmp/doc.svg"));
        assert!(!st.is_dirty());
        // Save writes SVG again with the same options, no dialog.
        app.run("shape.ellipse", json!({"x": 60, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("file.save", Value::Null).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(written.borrow()[1].0, "/tmp/doc.svg");
        assert!(text(&written, 1).contains("style=\"fill:") && vectorcraft_testkit::svg_shapes(&text(&written, 1)) == 2);
        // Save a Copy (options given) leaves the document's path alone.
        app.run("file.saveCopy", json!({"path": "/tmp/copy.svg", "svg": {"minify": true}})).unwrap();
        assert!(!text(&written, 2).trim_end().contains('\n') && !text(&written, 2).contains("<?xml"), "minified");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.svg"));
        // A native Save As needs no options.
        app.run("file.saveAs", json!({"path": "/tmp/doc.vectorcraft"})).unwrap();
        assert!(vectorcraft_format::sniff(&written.borrow()[3].1));
    }

    #[test]
    fn save_as_svg_leaves_out_a_remembered_artboard_the_document_no_longer_has() {
        let (mut app, written) = app("/tmp/doc.svg");
        app.run("file.saveAs", json!({"path": "/tmp/doc.svg", "svg": {"artboard": 1}})).unwrap();
        assert_eq!(app.session.active().unwrap().save_options["artboard"], 1);
        app.run("artboard.delete", json!({"index": 1})).unwrap();
        app.run("file.saveAs", Value::Null).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().fields.get("artboard").is_none());
        super::super::confirm(&mut app).unwrap();
        assert_eq!((written.borrow().len(), written.borrow()[1].0.as_str()), (2, "/tmp/doc.svg"));
    }

    /// Include Variable Data starts off, draws in the dialog, and writes the text variables and
    /// data sets into the saved SVG when it is on.
    #[test]
    fn include_variable_data_reaches_the_file() {
        let (mut app, written) = app("/tmp/vars.svg");
        let id = app.run("text.create", json!({"x": 10, "y": 60, "text": "Alice"})).unwrap()["id"].clone();
        app.run("variable.define", json!({"name": "Name", "kind": "text"})).unwrap();
        app.run("variable.bind", json!({"variable": "Name", "ids": [id]})).unwrap();
        app.run("dataset.new", json!({"name": "Row", "values": {"Name": "Bob"}})).unwrap();
        app.run("file.saveAs", Value::Null).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("variableData"), "off at first");
        frame(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("variableData".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        let svg = text(&written, 0);
        assert!(svg.contains("<variable varName=\"Name\"") && svg.contains("dataSetName=\"Row\"") && svg.contains("<p>Bob</p>"), "{svg}");
    }

    #[test]
    fn profile_encoding_and_embedded_fonts_reach_the_file() {
        let (mut app, written) = app("/tmp/tiny.svg");
        app.run("text.create", json!({"x": 10, "y": 60, "text": "Grüße €"})).unwrap();
        app.run("file.export.svg", Value::Null).unwrap();
        let d = app.ui.dialog.clone().unwrap();
        assert_eq!((d.str("profile"), d.str("encoding"), d.bool("embedFonts")), ("svg11".into(), "utf8".into(), false), "the defaults");
        let f = &mut app.ui.dialog.as_mut().unwrap().fields;
        f.insert("profile".into(), json!("tiny12"));
        f.insert("encoding".into(), json!("latin1"));
        f.insert("showCode".into(), json!(true));
        // Tiny shows Presentation Attributes (disabled) and greys out Embed Fonts; the code view
        // reads the Latin-1 file.
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        let bytes = written.borrow()[0].1.clone();
        assert!(bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>"));
        let svg = vectorcraft_svg::text_of(&bytes).unwrap();
        assert!(svg.contains("baseProfile=\"tiny\"") && !svg.contains("<style>") && svg.contains("&#x20AC;"), "{svg}");
        // SVG 1.1 with embedded fonts: an @font-face rule for the type.
        app.run("file.export.svg", Value::Null).unwrap();
        let f = &mut app.ui.dialog.as_mut().unwrap().fields;
        f.insert("profile".into(), json!("svg11"));
        f.insert("embedFonts".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        // Two artboards: two files each time; the type is on the first.
        assert_eq!(written.borrow().len(), 4);
        assert!(String::from_utf8_lossy(&written.borrow()[2].1).contains("@font-face{"));
    }
}
