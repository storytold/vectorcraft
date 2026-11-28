//! SVG Options (Export As SVG, Save As / Save a Copy to .svg): profile, styling, fonts, images,
//! object ids, decimals, encoding, units, minify, responsive, embedded fonts, artboards (export) or
//! editing data (save), and a read-only view of the code. OK remembers the choices for next time and runs the export or save command with
//! them as `svg: {…}`. Show in Browser (`file.export.svg.browser`) writes the SVG the choices give
//! to a temporary folder and opens it in the default browser, leaving the dialog open.

use std::borrow::Cow;
use std::sync::Arc;

use craft::ui::contract::{Bind, Scroll, Show as _, ShowContainer as _, TooltipSpec};
use craft::ui::dialog::DialogId;
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::SelectableText;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio;

use super::export_host::{self, After, Export, Open};
use super::form::{fields_of, json_of};
use super::png_options::{check, choice};
use crate::{VectorcraftApp, io};

/// `Dialog::kind` of this dialog.
pub const KIND: &str = "svgOptions";

/// The UI command of the dialog's Show in Browser button.
pub const BROWSER: &str = "file.export.svg.browser";

/// The id of the dialog's Show in Browser footer button, as `take_action` reports it.
pub(super) const BROWSER_ACTION: &str = "browser";

/// The folder in the system's temporary folder that Show in Browser's previews go to.
const PREVIEW_ROOT: &str = "VectorCraft SVG Preview";

/// The longest code text the code view shows.
const MAX_CODE: usize = 200_000;

/// What OK does: Export As SVG, Save As or Save a Copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Export,
    Save,
    SaveCopy,
}

/// (option value, label) for each choice list.
const STYLING: [(&str, &str); 4] = [
    ("css", "Internal CSS"),
    ("presentation", "Presentation Attributes"),
    ("style", "Style Attributes"),
    ("entities", "Style Attributes (Entity References)"),
];
const FONTS: [(&str, &str); 2] = [("false", "SVG"), ("true", "Convert To Outlines")];
const IMAGES: [(&str, &str); 2] = [("embed", "Embed"), ("link", "Link")];
const OBJECT_IDS: [(&str, &str); 3] = [("layerNames", "Layer Names"), ("minimal", "Minimal"), ("unique", "Unique")];
const PROFILES: [(&str, &str); 2] = [("svg11", "SVG 1.1"), ("tiny12", "SVG Tiny 1.2")];
const ENCODINGS: [(&str, &str); 3] = [("utf8", "Unicode (UTF-8)"), ("utf16", "Unicode (UTF-16)"), ("latin1", "ISO 8859-1")];
const UNITS: [(&str, &str); 6] =
    [("document", "Document"), ("px", "Pixels"), ("pt", "Points"), ("mm", "Millimeters"), ("cm", "Centimeters"), ("in", "Inches")];

/// Dialog fields that are not SVG options.
const UI_KEYS: [&str; 5] = ["mode", "path", "showCode", "allArtboards", "range"];

/// What the code view's text was encoded from: the text is encoded again only when one of these
/// changes. The read roots in force and the document's folder decide whether the text may be shown.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SvgCodeKey {
    uid: u64,
    revision: u64,
    options: String,
    roots: Option<String>,
    path: Option<String>,
    selected: Option<Vec<vectorcraft_doc::NodeId>>,
}

/// The options a first SVG export starts from: the engine's defaults with Pixels,
/// Internal CSS and Responsive on. Saves choose Document unless a remembered unit overrides it.
/// Hidden layers are left to the command (Save keeps them, Export leaves them out).
pub(super) fn first_use() -> Map<String, Value> {
    let opts = vectorcraft_svg::ExportOptions { units: Some(vectorcraft_svg::SizeUnit::Px), ..Default::default() };
    let mut m = serde_json::to_value(opts).ok().and_then(|v| v.as_object().cloned()).unwrap_or_default();
    m.remove("hiddenLayers");
    m.insert("styling".into(), json!("css"));
    m.insert("responsive".into(), json!(true));
    m.insert("useArtboards".into(), json!(true));
    m
}

/// Open SVG Options for `mode`, filled with the options used last (a save starts from the ones
/// the document was last saved with). `path`: where a save goes.
pub fn open(app: &mut VectorcraftApp, mode: Mode, path: Option<&str>) {
    open_with(app, mode, path, false);
}

/// [`open`] for Export Selection when `selection`: the dialog has no artboards to choose, and the
/// code and the browser preview show the selection's crop.
pub(crate) fn open_with(app: &mut VectorcraftApp, mode: Mode, path: Option<&str>, selection: bool) {
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
    if mode != Mode::Export && saved.as_ref().unwrap_or(&app.ui.svg_options).get("units").is_none() {
        // A document's legacy SVG options must not inherit Pixels from the last export.
        fields.insert("units".into(), json!("document"));
    }
    fields.insert("allArtboards".into(), json!(true));
    fields.insert("range".into(), json!(""));
    if let Some(p) = path {
        fields.insert("path".into(), json!(p));
    }
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let count = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let web = io::is_web(app);
    let initial = fields_of(&dialog_params(mode, selection, count, false), &fields);
    let dialog = HostedParamDialog::new(&ctx, KIND, initial, move |fields| {
        let tiny = fields.text("profile") == Some("tiny12");
        // SVG Tiny has presentation attributes alone; the other styles are shown disabled.
        if tiny && fields.text("styling") != Some("presentation") {
            fields.set("styling", "presentation");
        }
        if fields.bool("__hasArtboards") != Some(count > 0) {
            fields.set("__hasArtboards", count > 0);
        }
        let browser = FooterButton::new(tl!("Show in Browser"), DialogRole::Other).enabled(!web).disabled_reason(tl!("Needs the desktop app"));
        ParamDialog::new(KIND, tl!("SVG Options"), dialog_params(mode, selection, count, tiny))
            .values(Bind::bound(fields))
            .extra_buttons(vec![(BROWSER_ACTION.into(), browser)])
    });
    export_host::open(app, Export::Svg { mode }, selection, dialog);
}

/// A choice row over `(value, label)` pairs.
fn pairs(key: &str, label: &'static str, choices: &[(&str, &'static str)]) -> Param {
    let ids: Vec<&str> = choices.iter().map(|(v, _)| *v).collect();
    let labels: Vec<&'static str> = choices.iter().map(|(_, l)| *l).collect();
    choice(key, label, &ids, &labels)
}

/// The SVG options: profile, styling, fonts, images, object ids, decimals, encoding, minify,
/// responsive, `<tspan>`s and metadata. `screens` (Export for Screens' Format Settings) leaves out
/// Images and Variable Data: a linked image is another file. `tiny`: SVG Tiny has presentation
/// attributes alone and no web fonts.
pub(crate) fn svg_params(mode: Mode, screens: bool, tiny: bool) -> Vec<Param> {
    let styling = pairs("styling", tl!("Styling:"), &STYLING);
    let fonts = ParamKind::radio(FONTS.iter().map(|(v, l)| ((*v).to_string(), tl!(*l).into())).collect(), Some("false".into()));
    let (lo, hi) = (*vectorcraft_svg::DECIMALS.start() as f64, *vectorcraft_svg::DECIMALS.end() as f64);
    let mut rows = vec![
        pairs("profile", tl!("SVG Profile:"), &PROFILES),
        if tiny { styling.disabled_options(["css", "style", "entities"]) } else { styling },
        Param::new("outlineText", tl!("Font:"), fonts),
    ];
    if !screens {
        rows.push(pairs("images", tl!("Images:"), &IMAGES));
    }
    rows.push(pairs("objectIds", tl!("Object IDs:"), &OBJECT_IDS));
    rows.push(Param::number("decimals", tl!("Decimal:"), 3.0, 1.0, 0).bounds(Some(lo), Some(hi)));
    rows.push(pairs("encoding", tl!("Encoding:"), &ENCODINGS));
    // Responsive omits width and height, so the unit has nothing to act on.
    let units = if mode == Mode::Export { &UNITS[1..] } else { &UNITS[..] };
    rows.push(pairs("units", tl!("Units:"), units).enabled_when("responsive", false));
    rows.push(check("minify", tl!("Minify"), false));
    rows.push(check("responsive", tl!("Responsive"), true));
    // Embedded fonts need SVG 1.1 and text that stays text.
    let embed = check("embedFonts", tl!("Embed Fonts (Glyphs Used)"), false);
    rows.push(if tiny { embed.enabled_when("profile", "svg11") } else { embed.enabled_when("outlineText", "false") });
    rows.push(check("fewerTspans", tl!("Fewer <tspan> Elements"), false).enabled_when("outlineText", "false"));
    rows.push(check("metadata", tl!("Include Metadata"), true));
    if !screens {
        rows.push(check("variableData", tl!("Include Variable Data"), false));
    }
    rows
}

/// Use Artboards: all of them or a range; off = the bounds of all art.
fn artboard_rows(count: usize) -> Vec<Param> {
    let range = ParamKind::radio(vec![("true".into(), tl!("All").into()), ("false".into(), tl!("Range:").into())], Some("true".into()));
    let reason = TooltipSpec::new(tl!("Write each artboard, or the bounds of all art when off")).disabled_text(tl!("The document has no artboards"));
    vec![
        // `__hasArtboards` is the dialog's own mirror of the document's artboard count.
        check("useArtboards", tl!("Use Artboards"), true).enabled_when("__hasArtboards", true).tooltip(reason),
        Param::new("allArtboards", tl!("Artboards:"), range).enabled_when("useArtboards", true),
        Param::new("range", tl!("Range:"), ParamKind::text("")).enabled_when("allArtboards", "false").hint(format!("1-{count}")),
    ]
}

/// Every row of the dialog for `mode`. Export Selection writes the selection's crop, without
/// artboards to choose.
fn dialog_params(mode: Mode, selection: bool, count: usize, tiny: bool) -> Vec<Param> {
    let mut rows = svg_params(mode, false, tiny);
    match mode {
        Mode::Export if !selection => rows.extend(artboard_rows(count)),
        Mode::Export => {}
        Mode::Save | Mode::SaveCopy => rows.push(check("preserveEditing", tl!("Preserve Editing Capabilities"), false)),
    }
    rows.push(Param::new("showCode", tl!("Show Code"), ParamKind::bool(false)).in_footer(tl!("Hide Code")));
    rows.push(Param::custom("__code", tl!("Code"), code_view).visible_when("showCode", true).opens_with(""));
    rows
}

/// A two-id radio or a check box as a flag, whichever the value holds.
fn flag(f: &DialogFields, key: &str, default: bool) -> bool {
    f.bool(key).or_else(|| f.text(key).map(|t| t == "true")).unwrap_or(default)
}

/// The SVG options the dialog's fields describe (artboards only when exporting, editing data
/// only when saving).
fn options(mode: Mode, f: &DialogFields) -> Value {
    let tiny = f.text("profile") == Some("tiny12");
    let mut o = json_of(&dialog_params(mode, false, 1, tiny), f);
    o.retain(|k, _| !UI_KEYS.contains(&k.as_str()) && !k.starts_with("__"));
    if o.get("units").and_then(Value::as_str) == Some("document") {
        o.remove("units");
    }
    if mode == Mode::Export {
        o.remove("preserveEditing");
        if f.bool("useArtboards").unwrap_or(false) {
            let range = f.text("range").unwrap_or_default().trim();
            o.insert("range".into(), json!(if flag(f, "allArtboards", true) || range.is_empty() { "all" } else { range }));
        }
    } else {
        o.remove("useArtboards");
    }
    Value::Object(o)
}

pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    let Export::Svg { mode } = open.kind else { return Err("not the SVG options".into()) };
    let selection = open.selection;
    let opts = options(mode, fields);
    if mode == Mode::Export && fields.bool("useArtboards").unwrap_or(false) && !selection {
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
    let mut params = json!({ "svg": opts });
    if let Some(p) = fields.text("path") {
        params["path"] = json!(p);
    }
    let params = super::export_as::exported(selection, params);
    let id = match mode {
        Mode::Export => "file.export.svg",
        Mode::Save => "file.saveAs",
        Mode::SaveCopy => "file.saveCopy",
    };
    // The command runs in OK, so a refusal keeps the dialog open; nothing follows its closing.
    app.run(id, params).map(|_| None)
}

/// The SVG options the dialog's OK writes with: [`options`], with hidden layers kept (not
/// displayed) when OK saves, as a save keeps them unless told otherwise.
fn written_options(mode: Mode, f: &DialogFields) -> Value {
    let mut o = options(mode, f);
    if mode != Mode::Export
        && let Some(m) = o.as_object_mut()
    {
        m.entry("hiddenLayers").or_insert(json!(true));
    }
    o
}

/// The params of [`BROWSER`] for the dialog's fields: the SVG options OK writes with
/// ([`written_options`]), and `exportSelection` when Export Selection opened it.
pub(super) fn preview_params(mode: Mode, selection: bool, f: &DialogFields) -> Value {
    super::export_as::exported(selection, json!({ "svg": written_options(mode, f) }))
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
        let dir = super::preview_folders::temp_folder_under(PREVIEW_ROOT)?;
        let target = format!("{dir}/{}.svg", super::preview_folders::preview_name(&stem));
        let named = enc.named(&doc, &target);
        for (p, bytes) in &named {
            io::write_to(&mut app.services, p, bytes).inspect_err(|_| super::preview_folders::remove_preview_folder(&dir))?;
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
/// the selection alone, cropped to it, as the code view shows it. Encoded again only when what
/// [`SvgCodeKey`] holds changes; a failure is the text (it names no file outside the read roots).
fn code_text(app: &mut VectorcraftApp, mode: Mode, selection: bool, fields: &DialogFields) -> Option<Arc<str>> {
    let st = app.session.active()?;
    let opts = written_options(mode, fields);
    let key = SvgCodeKey {
        uid: st.uid,
        revision: st.revision,
        options: opts.to_string(),
        roots: vectorcraft_engine::file_access::current().map(|r| r.describe()),
        path: st.path.clone(),
        selected: selection.then(|| st.selection.objects.clone()),
    };
    if let Some((cached, text)) = &app.canvas.svg_code
        && *cached == key
    {
        return Some(text.clone());
    }
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
    if text.len() > MAX_CODE {
        let cut = (0..=MAX_CODE).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n…");
    }
    let text: Arc<str> = text.into();
    app.canvas.svg_code = Some((key, text.clone()));
    Some(text)
}

/// Fills the code view of the open dialog `id` with the SVG the options write, while Show Code is
/// on; the export host calls it each pass. The document is encoded with no lock on the host held.
pub(super) fn refresh_code(app: &mut VectorcraftApp, id: DialogId, mode: Mode, selection: bool) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(fields) = dialogs.with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())) else { return };
    if fields.bool("showCode") != Some(true) {
        return;
    }
    let Some(text) = code_text(app, mode, selection, &fields) else { return };
    if fields.text("__code") != Some(&*text) {
        dialogs.with(|h| h.get_mut_by_id::<HostedParamDialog>(id).map(|d| d.set_field("__code", FieldValue::Text(text.to_string()))));
    }
}

/// The code view row: the text `refresh_code` keeps in `__code`, read-only and selectable.
fn code_view(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut CustomContext) -> egui::Response {
    let code = fields.text("__code").unwrap_or_default();
    Stack::new()
        .scroll(Scroll::vertical())
        .room_height(260.0)
        .show(ui, |ui, _| {
            SelectableText::new(tl!("Code"), code).mono().multiline().show(ui);
        })
        .output
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use vectorcraft_engine::Session;

    use super::super::tests_export::{confirm, frame, kind, set, try_confirm};
    use super::super::tests_raster_formats::fields;
    use super::*;
    use crate::Services;

    type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

    /// An app with two artboards and a rectangle whose writer records what it is given.
    fn app() -> (VectorcraftApp, Written) {
        let written = Written::default();
        let w = written.clone();
        let services = Services {
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
        app.run("file.new", json!({"width": 120, "height": 80, "artboards": 2})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
        (app, written)
    }

    /// SVG Options open for `mode` and `path`, drawn once.
    fn start(app: &mut VectorcraftApp, mode: Mode, path: &str, selection: bool) {
        open_with(app, mode, Some(path), selection);
        frame(app);
    }

    fn text(w: &Written, i: usize) -> String {
        String::from_utf8(w.borrow()[i].1.clone()).unwrap()
    }

    /// The URLs the app asked egui to open in one frame of the whole app (the frame's tick sends them).
    fn open_urls(app: &mut VectorcraftApp) -> Vec<String> {
        let ctx = app.ui_ctx.clone().expect("a test app has a window");
        crate::testing::app_frame(app, &ctx, Default::default())
            .platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::OpenUrl(o) => Some(o.url.clone()),
                _ => None,
            })
            .collect()
    }

    /// [`app`] whose previews go to a temporary folder of the test's own ([`preview_tmp`]), not the system's.
    fn browsing() -> (VectorcraftApp, Written) {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let tmp = vectorcraft_testkit::temp_dir(&format!("svg-browser-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        super::super::preview_folders::TEST_TMP.set(Some(tmp));
        app()
    }

    /// The SVG preview folder inside this test's temporary folder ([`browsing`]), with `/` between
    /// its parts.
    fn preview_tmp() -> String {
        let tmp = super::super::preview_folders::TEST_TMP.with_borrow(Clone::clone).expect("browsing sets the temporary folder");
        tmp.join(PREVIEW_ROOT).to_string_lossy().replace('\\', "/")
    }

    /// Show in Browser for the open dialog's current choices, as its button runs it.
    fn show_in_browser(app: &mut VectorcraftApp, mode: Mode, selection: bool) -> Result<Value, String> {
        let p = preview_params(mode, selection, &fields(app));
        app.run(BROWSER, p)
    }

    /// Show in Browser writes the SVG of the dialog's current, unconfirmed choices for the first
    /// artboard of the range to a new temporary folder and opens that file once. The dialog stays
    /// open, nothing is remembered, and OK then writes the same SVG to the chosen file.
    #[test]
    fn show_in_browser_opens_what_ok_writes_without_confirming() {
        let (mut app, written) = browsing();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        set(&mut app, "decimals", json!(1));
        set(&mut app, "allArtboards", json!("false"));
        set(&mut app, "range", json!("2"));
        frame(&mut app);
        let remembered = app.ui.svg_options.clone();
        let r = show_in_browser(&mut app, Mode::Export, false).unwrap_or_else(|e| panic!("{e}"));
        let path = r["path"].as_str().unwrap().to_string();
        assert!(path.starts_with(&format!("{}/", preview_tmp())) && path.ends_with("/Untitled-1.svg"), "{path}");
        let mut opened = open_urls(&mut app);
        assert_eq!(opened, [io::file_url(&path)]);
        assert_eq!(written.borrow().len(), 1);
        assert_eq!(written.borrow()[0].0, path);
        assert_eq!(kind(&app).as_deref(), Some(KIND), "the dialog stays open");
        assert_eq!(app.ui.svg_options, remembered, "nothing is remembered");
        assert!(app.session.active().unwrap().path.is_none());
        confirm(&mut app);
        assert_eq!(written.borrow()[1].0, "/tmp/out.svg");
        assert_eq!(text(&written, 0), text(&written, 1), "the preview is the file OK writes");
        opened.extend(open_urls(&mut app));
        assert_eq!(opened.len(), 1);
    }

    /// A range the document has no artboard for is refused, as OK refuses it: nothing is written
    /// or opened, and the dialog stays open.
    #[test]
    fn show_in_browser_refuses_a_range_the_document_lacks() {
        let (mut app, written) = browsing();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        set(&mut app, "allArtboards", json!("false"));
        set(&mut app, "range", json!("5"));
        assert!(show_in_browser(&mut app, Mode::Export, false).is_err());
        assert!(written.borrow().is_empty() && open_urls(&mut app).is_empty());
        assert_eq!(kind(&app).as_deref(), Some(KIND));
    }

    /// A write that fails opens nothing and removes the folder made for that preview.
    #[test]
    fn show_in_browser_removes_its_folder_when_a_write_fails() {
        let (mut app, _) = browsing();
        app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("disk full".to_string())));
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        assert!(show_in_browser(&mut app, Mode::Export, false).unwrap_err().contains("disk full"));
        assert!(open_urls(&mut app).is_empty());
        let left: Vec<_> = std::fs::read_dir(preview_tmp()).map(|d| d.flatten().map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }

    /// Under Export Selection the preview is the selection alone, cropped to it, as OK writes it.
    #[test]
    fn show_in_browser_under_export_selection_shows_the_selection() {
        let (mut app, written) = browsing();
        app.run("shape.rectangle", json!({"x": 70, "y": 40, "width": 20, "height": 20})).unwrap();
        start(&mut app, Mode::Export, "/tmp/selection.svg", true);
        show_in_browser(&mut app, Mode::Export, true).unwrap_or_else(|e| panic!("{e}"));
        confirm(&mut app);
        assert_eq!(written.borrow()[1].0, "/tmp/selection.svg");
        assert_eq!(text(&written, 0), text(&written, 1));
        assert_eq!(text(&written, 0).matches("<rect").count(), 1, "{}", text(&written, 0));
        assert_eq!(open_urls(&mut app).len(), 1);
    }

    /// From Save As, the preview leaves the document's path and unsaved state alone, and OK still
    /// saves to the chosen file.
    #[test]
    fn show_in_browser_from_save_as_leaves_the_document_unsaved() {
        let (mut app, written) = browsing();
        start(&mut app, Mode::Save, "/tmp/doc.svg", false);
        let dirty = app.session.active().unwrap().is_dirty();
        show_in_browser(&mut app, Mode::Save, false).unwrap_or_else(|e| panic!("{e}"));
        let st = app.session.active().unwrap();
        assert_eq!((st.path.as_deref(), st.is_dirty()), (None, dirty));
        assert_eq!(open_urls(&mut app).len(), 1);
        confirm(&mut app);
        assert_eq!(written.borrow()[1].0, "/tmp/doc.svg");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.svg"));
    }

    /// From Save As, a hidden layer is in the preview, kept hidden, as the save keeps it. The two
    /// differ only in the title: the save takes it from the file's name, the preview from the
    /// document's.
    #[test]
    fn show_in_browser_from_save_as_keeps_hidden_layers_as_the_save_does() {
        let (mut app, written) = browsing();
        let notes = app.run("layer.new", json!({"name": "Notes"})).unwrap()["id"].clone();
        app.run("shape.ellipse", json!({"x": 60, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("layer.setProps", json!({"id": notes, "visible": false})).unwrap();
        start(&mut app, Mode::Save, "/tmp/doc.svg", false);
        show_in_browser(&mut app, Mode::Save, false).unwrap_or_else(|e| panic!("{e}"));
        confirm(&mut app);
        assert!(text(&written, 0).contains("id=\"Notes\""), "{}", text(&written, 0));
        assert_eq!(text(&written, 0).replace("<title>Untitled-1</title>", "<title>doc</title>"), text(&written, 1));
    }

    /// A document with no artboards is previewed by the bounds of its art, as the SVG writer
    /// writes it, rather than refused for its artboard range.
    #[test]
    fn show_in_browser_previews_a_document_without_artboards() {
        let (mut app, written) = browsing();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).artboards.clear();
        show_in_browser(&mut app, Mode::Export, false).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(vectorcraft_testkit::svg_shapes(&text(&written, 0)), 1, "{}", text(&written, 0));
        assert_eq!(open_urls(&mut app).len(), 1);
    }

    /// The web build has no file to open, and a control or MCP request may not open another app:
    /// both are refused before anything is written or opened.
    #[test]
    fn show_in_browser_is_refused_on_the_web_and_for_automation() {
        let (mut app, written) = browsing();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        app.ui.answering_control = true;
        assert!(show_in_browser(&mut app, Mode::Export, false).unwrap_err().contains("opens another app"));
        app.ui.answering_control = false;
        assert!(crate::menus::enabled(&app, BROWSER));
        app.services.download = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
        assert!(!crate::menus::enabled(&app, BROWSER), "the command palette and menus show it disabled on the web");
        assert!(show_in_browser(&mut app, Mode::Export, false).is_err());
        assert!(written.borrow().is_empty() && open_urls(&mut app).is_empty());
    }

    /// SVG previews have their own folder: making one removes only old SVG preview folders and
    /// leaves Save for Web's alone.
    #[test]
    fn svg_preview_folders_leave_save_for_web_previews_alone() {
        let tmp = vectorcraft_testkit::temp_dir("svg-preview-folders");
        let web = super::super::preview_folders::preview_folder_in(&tmp, std::time::Duration::from_secs(3600)).unwrap();
        let old = super::super::preview_folders::preview_folder_under(&tmp, PREVIEW_ROOT, std::time::Duration::from_secs(3600)).unwrap();
        let new = super::super::preview_folders::preview_folder_under(&tmp, PREVIEW_ROOT, std::time::Duration::ZERO).unwrap();
        assert!(new.contains("/VectorCraft SVG Preview/"), "{new}");
        assert!(std::path::Path::new(&web).is_dir(), "Save for Web's preview is kept");
        assert!(!std::path::Path::new(&old).exists(), "the old SVG preview is removed");
        assert!(std::path::Path::new(&new).is_dir());
    }

    #[test]
    fn export_as_svg_opens_the_options_and_remembers_them() {
        let (mut app, written) = app();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        assert_eq!(kind(&app).as_deref(), Some(KIND));
        // Show Code draws the SVG the options write.
        set(&mut app, "showCode", json!(true));
        frame(&mut app);
        assert_eq!(kind(&app).as_deref(), Some(KIND));
        set(&mut app, "decimals", json!(1));
        set(&mut app, "allArtboards", json!("false"));
        set(&mut app, "range", json!("1"));
        confirm(&mut app);
        assert_eq!(kind(&app), None);
        let svg = text(&written, 0);
        assert_eq!(written.borrow()[0].0, "/tmp/out.svg");
        assert!(svg.contains("<style>") && !svg.contains(" width=\"120\""), "{svg}");
        assert_eq!(app.ui.svg_options["decimals"], 1);
        assert!(app.ui.svg_options.get("range").is_none(), "the range belongs to the document");
        // Next time the dialog starts from these choices.
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        assert_eq!(fields(&app).number("decimals"), Some(1.0));
    }

    #[test]
    fn svg_options_write_and_remember_units_and_decimals() {
        let (mut app, written) = app();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        set(&mut app, "responsive", json!(false));
        set(&mut app, "units", json!("mm"));
        set(&mut app, "decimals", json!(1));
        confirm(&mut app);
        let svg = text(&written, 0);
        assert!(svg.contains("mm\"") && svg.contains("viewBox=\"0 0 120 80\""), "{svg}");
        assert_eq!(app.ui.svg_options["decimals"], 1);
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        assert_eq!(fields(&app).text("units"), Some("mm"));
        set(&mut app, "responsive", json!(true));
        confirm(&mut app);
        let svg = text(&written, 2);
        assert!(!svg.contains("mm\"") && svg.contains("viewBox=\"0 0 120 80\""), "{svg}");
    }

    #[test]
    fn legacy_svg_saves_keep_document_units() {
        for mode in [Mode::Save, Mode::SaveCopy] {
            for last in [json!({"responsive": false}), json!({"responsive": false, "units": "px"})] {
                let (mut app, written) = app();
                let original = std::env::temp_dir().join("vc-original.svg").to_string_lossy().replace('\\', "/");
                let path = std::env::temp_dir().join("vc-saved.svg").to_string_lossy().replace('\\', "/");
                app.run("document.setUnits", json!({"units": "millimeters"})).unwrap();
                app.run("file.saveAs", json!({"path": original, "svg": {"responsive": false}})).unwrap();
                let before = vectorcraft_svg::import(&text(&written, 0)).unwrap();
                app.ui.svg_options = last;
                start(&mut app, mode, &path, false);
                confirm(&mut app);
                let after = vectorcraft_svg::import(&text(&written, 1)).unwrap();
                assert_eq!(after.units, vectorcraft_doc::Unit::Millimeters, "{mode:?}: {}", text(&written, 1));
                let (a, b) = (before.artboards[0].rect, after.artboards[0].rect);
                assert!((a.width() - b.width()).abs() < 1e-6 && (a.height() - b.height()).abs() < 1e-6, "{mode:?}: {a:?} -> {b:?}");
                let expected = if mode == Mode::SaveCopy { &original } else { &path };
                assert_eq!(app.session.active().unwrap().path.as_deref(), Some(expected.as_str()));
            }
        }
    }

    #[test]
    fn svg_saves_keep_explicit_units_and_allow_pixel_overrides() {
        for mode in [Mode::Save, Mode::SaveCopy] {
            let (mut app, written) = app();
            let path = std::env::temp_dir().join("vc-explicit.svg").to_string_lossy().replace('\\', "/");
            app.run("document.setUnits", json!({"units": "millimeters"})).unwrap();
            app.run("file.saveAs", json!({"path": path, "svg": {"responsive": false, "units": "in"}})).unwrap();
            app.ui.svg_options = json!({"responsive": false, "units": "px"});
            start(&mut app, mode, &path, false);
            confirm(&mut app);
            assert_eq!(vectorcraft_svg::import(&text(&written, 1)).unwrap().units, vectorcraft_doc::Unit::Inches);
            start(&mut app, mode, &path, false);
            set(&mut app, "units", json!("px"));
            confirm(&mut app);
            assert_eq!(vectorcraft_svg::import(&text(&written, 2)).unwrap().units, vectorcraft_doc::Unit::Pixels);
        }
    }

    #[test]
    fn svg_export_keeps_pixels_when_legacy_options_omit_units() {
        let (mut app, written) = app();
        let path = std::env::temp_dir().join("vc-export.svg").to_string_lossy().replace('\\', "/");
        app.run("document.setUnits", json!({"units": "millimeters"})).unwrap();
        app.ui.svg_options = json!({"responsive": false});
        start(&mut app, Mode::Export, &path, false);
        confirm(&mut app);
        assert_eq!(vectorcraft_svg::import(&text(&written, 0)).unwrap().units, vectorcraft_doc::Unit::Pixels);
    }

    /// OK with a range the document lacks keeps the dialog open with the reason, writes nothing and
    /// remembers nothing.
    #[test]
    fn a_bad_range_in_svg_options_keeps_the_dialog_and_the_saved_options() {
        let (mut app, written) = app();
        start(&mut app, Mode::Export, "/tmp/out.svg", false);
        let before = app.ui.svg_options.clone();
        set(&mut app, "allArtboards", json!("false"));
        set(&mut app, "range", json!("9"));
        assert!(try_confirm(&mut app).is_err());
        assert_eq!(kind(&app).as_deref(), Some(KIND));
        assert!(written.borrow().is_empty());
        assert_eq!(app.ui.svg_options, before);
    }

    /// The text the code view shows after two frames, with `roots` in force.
    fn code(app: &mut VectorcraftApp, roots: Option<&Arc<vectorcraft_engine::file_access::AutomationRoots>>) -> String {
        for _ in 0..2 {
            vectorcraft_engine::file_access::confine(roots, || frame(app));
        }
        fields(app).text("__code").unwrap_or_default().to_string()
    }

    /// Show Code under read roots draws no SVG of a document that links a JavaScript file outside
    /// them: it shows the error, which names no file or folder. Without roots it shows the link.
    #[test]
    fn show_code_leaves_out_a_linked_javascript_file_outside_the_read_roots() {
        let tmp = std::env::temp_dir();
        let root = tmp.join(format!("vc-svg-code-root-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let script = tmp.join(format!("vc-svg-code-away-{}", std::process::id())).join("menu.js").to_string_lossy().into_owned();
        let (mut app, _) = app();
        app.run("svgInteractivity.addFile", json!({"file": script})).unwrap();
        start(&mut app, Mode::Export, "/tmp/code.svg", false);
        set(&mut app, "showCode", json!(true));
        let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap().unwrap();
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
        let (mut app, _) = app();
        app.run("file.place", json!({"path": photo, "link": true, "at": [50, 50]})).unwrap();
        start(&mut app, Mode::Export, "/tmp/code.svg", false);
        set(&mut app, "showCode", json!(true));
        let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[&root], &[]).unwrap().unwrap();
        let embedded = code(&mut app, Some(&roots));
        assert!(embedded.contains("data:image/png;base64,") && !embedded.contains(&photo), "{embedded}");
        set(&mut app, "images", json!("link"));
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
        let (mut app, written) = app();
        app.run("shape.rectangle", json!({"x": 70, "y": 40, "width": 20, "height": 20})).unwrap();
        app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
        start(&mut app, Mode::Export, "/tmp/selection.svg", true);
        set(&mut app, "showCode", json!(true));
        let code = code(&mut app, None);
        confirm(&mut app);
        let svg = text(&written, 0);
        assert_eq!(written.borrow()[0].0, "/tmp/selection.svg");
        assert_eq!(code, svg, "Show Code shows what OK writes");
        // The shadow reaches past the 21-point square, and the other rectangle is left out.
        assert!(!svg.contains("viewBox=\"0 0 120 80\"") && !svg.contains("viewBox=\"0 0 21 21\""), "{svg}");
        assert!(!svg.contains("width=\"40\""), "{svg}");
        assert_eq!(svg.matches("<rect").count(), 1, "{svg}");
        assert!(svg.contains("width=\"20\" height=\"20\"") && svg.contains("<feOffset"), "{svg}");
    }

    /// The dialog's own fields (`__` ones, the UI keys) never reach the save, which is given only
    /// the SVG options, so it is not refused for an unknown key.
    #[test]
    fn save_as_svg_from_a_save_panel_passes_only_the_options() {
        let (mut app, written) = app();
        start(&mut app, Mode::Save, "/tmp/panel.svg", false);
        let o = written_options(Mode::Save, &fields(&app));
        let keys = o.as_object().unwrap();
        assert!(UI_KEYS.iter().all(|k| !keys.contains_key(*k)) && keys.keys().all(|k| !k.starts_with("__")), "{o}");
        try_confirm(&mut app).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(written.borrow()[0].0, "/tmp/panel.svg");
    }

    #[test]
    fn all_artboards_export_one_file_each() {
        let (mut app, written) = app();
        start(&mut app, Mode::Export, "/tmp/set.svg", false);
        confirm(&mut app);
        let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(names, ["/tmp/set-Artboard-1.svg", "/tmp/set-Artboard-2.svg"]);
    }

    #[test]
    fn save_as_svg_asks_for_options_then_save_reuses_them() {
        let (mut app, written) = app();
        let r = app.run("file.saveAs", json!({"path": "/tmp/doc.svg"})).unwrap();
        assert_eq!(r["dialog"], KIND);
        frame(&mut app);
        assert_eq!(fields(&app).text("path"), Some("/tmp/doc.svg"));
        set(&mut app, "styling", json!("style"));
        confirm(&mut app);
        assert!(text(&written, 0).contains("style=\"fill:"));
        let st = app.session.active().unwrap();
        assert_eq!(st.path.as_deref(), Some("/tmp/doc.svg"));
        assert!(!st.is_dirty());
        // Save writes SVG again with the same options, no dialog.
        app.run("shape.ellipse", json!({"x": 60, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("file.save", Value::Null).unwrap();
        assert_eq!(kind(&app), None);
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
        let (mut app, written) = app();
        app.run("file.saveAs", json!({"path": "/tmp/doc.svg", "svg": {"artboard": 1}})).unwrap();
        assert_eq!(app.session.active().unwrap().save_options["artboard"], 1);
        app.run("artboard.delete", json!({"index": 1})).unwrap();
        app.run("file.saveAs", json!({"path": "/tmp/doc.svg"})).unwrap();
        frame(&mut app);
        assert!(fields(&app).get("artboard").is_none());
        confirm(&mut app);
        assert_eq!((written.borrow().len(), written.borrow()[1].0.as_str()), (2, "/tmp/doc.svg"));
    }

    /// Include Variable Data starts off, draws in the dialog, and writes the text variables and
    /// data sets into the saved SVG when it is on.
    #[test]
    fn include_variable_data_reaches_the_file() {
        let (mut app, written) = app();
        let id = app.run("text.create", json!({"x": 10, "y": 60, "text": "Alice"})).unwrap()["id"].clone();
        app.run("variable.define", json!({"name": "Name", "kind": "text"})).unwrap();
        app.run("variable.bind", json!({"variable": "Name", "ids": [id]})).unwrap();
        app.run("dataset.new", json!({"name": "Row", "values": {"Name": "Bob"}})).unwrap();
        app.run("file.saveAs", json!({"path": "/tmp/vars.svg"})).unwrap();
        frame(&mut app);
        assert_ne!(fields(&app).bool("variableData"), Some(true), "off at first");
        set(&mut app, "variableData", json!(true));
        confirm(&mut app);
        let svg = text(&written, 0);
        assert!(svg.contains("<variable varName=\"Name\"") && svg.contains("dataSetName=\"Row\"") && svg.contains("<p>Bob</p>"), "{svg}");
    }

    #[test]
    fn profile_encoding_and_embedded_fonts_reach_the_file() {
        let (mut app, written) = app();
        app.run("text.create", json!({"x": 10, "y": 60, "text": "Grüße €"})).unwrap();
        start(&mut app, Mode::Export, "/tmp/tiny.svg", false);
        set(&mut app, "profile", json!("tiny12"));
        set(&mut app, "encoding", json!("latin1"));
        set(&mut app, "showCode", json!(true));
        // Tiny shows Presentation Attributes (disabled) and greys out Embed Fonts; the code view
        // reads the Latin-1 file.
        frame(&mut app);
        confirm(&mut app);
        let bytes = written.borrow()[0].1.clone();
        assert!(bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>"));
        let svg = vectorcraft_svg::text_of(&bytes).unwrap();
        assert!(svg.contains("baseProfile=\"tiny\"") && !svg.contains("<style>") && svg.contains("&#x20AC;"), "{svg}");
        // SVG 1.1 with embedded fonts: an @font-face rule for the type.
        start(&mut app, Mode::Export, "/tmp/tiny.svg", false);
        set(&mut app, "profile", json!("svg11"));
        set(&mut app, "embedFonts", json!(true));
        confirm(&mut app);
        // Two artboards: two files each time; the type is on the first.
        assert_eq!(written.borrow().len(), 4);
        assert!(String::from_utf8_lossy(&written.borrow()[2].1).contains("@font-face{"));
    }
}
