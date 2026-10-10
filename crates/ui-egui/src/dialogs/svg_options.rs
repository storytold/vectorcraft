//! SVG Options (Export As SVG, Save As / Save a Copy to .svg): profile, styling, fonts, images,
//! object ids, decimals, encoding, minify, responsive, embedded fonts, artboards (export) or
//! editing data (save), and a read-only view of the code. OK remembers the choices for next time and runs the export or save command with
//! them as `svg: {…}`.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("SVG Options").into(),
    body,
    confirm,
    ok: Some("OK"),
    min_width: 380.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

/// `Dialog::kind` of this dialog.
pub const KIND: &str = "svgOptions";

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
        .map(|st| Value::Object(st.save_options.clone()));
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
    let mut o: Map<String, Value> = d.fields.iter().filter(|(k, _)| !UI_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
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
    if Mode::of(d) == Mode::Export && d.bool("useArtboards") {
        let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
        let pick: vectorcraft_engine::cmd::fileio::ArtboardPick = serde_json::from_value(opts.clone()).map_err(|e| e.to_string())?;
        pick.resolve(n)?;
    }
    // Remembered for next time, without the document's artboard range.
    let mut last = opts.clone();
    if let Some(o) = last.as_object_mut() {
        o.remove("range");
    }
    app.ui.svg_options = last;
    app.ui.dialog = None;
    let mut params = json!({ "svg": opts });
    if let Some(p) = d.fields.get("path").filter(|p| p.is_string()) {
        params["path"] = p.clone();
    }
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
    if Mode::of(d) == Mode::Export {
        artboards(app, ui, d);
    } else {
        check(ui, d, "preserveEditing", tl!("Preserve Editing Capabilities"), true);
    }
    ui.add_space(10.0);
    let show = d.bool("showCode");
    if widgets::secondary_button(ui, if show { tl!("Hide Code") } else { tl!("Show Code") }).clicked() {
        d.fields.insert("showCode".into(), json!(!show));
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
            let edit = egui::TextEdit::singleline(&mut range).hint_text(format!("1-{count}")).desired_width(90.0);
            if ui.add_enabled(!all, edit).changed() {
                d.fields.insert("range".into(), json!(range));
            }
        });
    });
}

/// The SVG the current options write for the first chosen artboard (read-only, selectable),
/// encoded again only when the options or the document change.
fn code_view(app: &VectorcraftApp, ui: &mut egui::Ui, d: &Dialog) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    let mut opts = options(d);
    if let Some(o) = opts.as_object_mut()
        && let Some(r) = o.remove("range")
    {
        // The first artboard of the range (or the first one).
        let first = r.as_str().and_then(|r| r.split([',', '-', '\u{2013}']).next()).and_then(|n| n.trim().parse::<usize>().ok());
        o.insert("artboard".into(), json!(first.map_or(0, |n| n.saturating_sub(1))));
    }
    // One cached text, replaced when the document or the options change.
    let key = egui::Id::new("svg-code");
    let sig = egui::Id::new((st.uid, st.revision, opts.to_string())).value();
    let code: Arc<String> = match ui.ctx().data(|m| m.get_temp::<(u64, Arc<String>)>(key)).filter(|(s, _)| *s == sig) {
        Some((_, c)) => c,
        None => {
            const MAX: usize = 200_000;
            let mut text = match vectorcraft_engine::cmd::fileio::encode_all(&st.doc, "svg", &json!({ "svg": opts })) {
                // Shown as text whatever the file's encoding.
                Ok(enc) => enc
                    .files
                    .into_iter()
                    .next()
                    .map(|(_, b)| vectorcraft_svg::text_of(&b).map_or_else(|_| String::from_utf8_lossy(&b).into_owned(), |t| t.into_owned()))
                    .unwrap_or_default(),
                Err(e) => e.to_string(),
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
        assert!(text(&written, 1).contains("style=\"fill:") && text(&written, 1).matches("<path").count() == 2);
        // Save a Copy (options given) leaves the document's path alone.
        app.run("file.saveCopy", json!({"path": "/tmp/copy.svg", "svg": {"minify": true}})).unwrap();
        assert!(!text(&written, 2).trim_end().contains('\n') && !text(&written, 2).contains("<?xml"), "minified");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.svg"));
        // A native Save As needs no options.
        app.run("file.saveAs", json!({"path": "/tmp/doc.vectorcraft"})).unwrap();
        assert!(vectorcraft_format::sniff(&written.borrow()[3].1));
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
