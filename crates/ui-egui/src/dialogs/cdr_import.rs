//! CorelDRAW Import Options: opening (File › Open) or placing (File › Place) a `.cdr` document shows
//! the version, the container and the content the reader found, and asks for the import options:
//! the original scale, layers and groups, editable type or outlines, missing-font warnings and a
//! report of what isn't supported.
//!
//! Fields: `mode` (`open` | `place`), `name`, `path?`, `product`, `version`, `container`,
//! `pages`, `objects`, `pageW`/`pageH` (mm), `readable`, `error?`, and the `cdr` options of
//! `document.open`: `preserveScale`, `preserveLayers`, `textToOutlines`, `warnMissingFonts`,
//! `reportUnsupported`. `__place` keeps the other `file.place` params. The file's bytes stay in
//! [`crate::state::UiState::dialog_file`]. OK remembers the choices for the next file; errors keep
//! the dialog open.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use super::document_setup::check;
use super::import_pdf::{DialogFile, other_place_params, place_source, with_file};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub const KIND: &str = "cdrImport";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str("mode") == "place" { tl!("Import CorelDRAW") } else { tl!("CorelDRAW Import Options") }.into(),
    body,
    confirm,
    min_width: 460.0,
    max_width: Some(520.0),
    ..DialogSpec::FORM
};

/// The choices the next file starts from.
const REMEMBERED: [&str; 5] = ["preserveScale", "preserveLayers", "textToOutlines", "warnMissingFonts", "reportUnsupported"];

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when they
/// are a CorelDRAW document; false when they aren't (the loader then says why).
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    if fileio::detect(name, bytes).is_none_or(|f| f.id != "cdr") {
        return false;
    }
    let Ok(info) = vectorcraft_cdr::detect_version(bytes) else { return false };
    let v = vectorcraft_cdr::validate(bytes);
    app.ui.dialog_file = Some(DialogFile::new(bytes.to_vec()));
    let (w, h) = v.page_size.unwrap_or((0.0, 0.0));
    let mut fields = json!({
        "mode": if place.is_some() { "place" } else { "open" },
        "name": name,
        "product": info.product,
        "version": info.version,
        "container": info.container.label(),
        "note": info.note,
        "pages": v.pages,
        "objects": v.objects,
        "pageW": w,
        "pageH": h,
        "readable": v.ok(),
        "error": v.error,
        "preserveScale": true,
        "preserveLayers": true,
        "textToOutlines": false,
        "warnMissingFonts": true,
        "reportUnsupported": true,
    });
    if let Some(last) = app.ui.cdr_import.as_object() {
        for k in REMEMBERED {
            if let Some(v) = last.get(k).filter(|v| v.is_boolean()) {
                fields[k] = v.clone();
            }
        }
    }
    if let Some(p) = path {
        fields["path"] = json!(p);
    }
    if let Some(p) = place {
        fields["__place"] = other_place_params(p, &["cdr"]);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    true
}

/// `file.place` with params `p` asks first (true) when they name a `.cdr` document and no `cdr`
/// options.
pub fn offer_place(app: &mut VectorcraftApp, p: &Value) -> bool {
    let named = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::extension);
    if p.get("cdr").is_some() || named.as_deref() != Some("cdr") {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| ext == "cdr") else { return false };
    offer(app, &name, &bytes, path, Some(p))
}

/// The `cdr` options of `document.open` and `file.place` the fields set.
fn options(app: &VectorcraftApp, d: &Dialog) -> Value {
    let mut o = json!({
        "preserveScale": d.bool("preserveScale"),
        "preserveLayers": d.bool("preserveLayers"),
        "textToOutlines": d.bool("textToOutlines"),
        "warnMissingFonts": d.bool("warnMissingFonts"),
        "reportUnsupported": d.bool("reportUnsupported"),
    });
    // Without the original scale, a placed document's first page fits the first artboard.
    if !d.bool("preserveScale")
        && d.str("mode") == "place"
        && let Some(st) = app.session.active()
        && let Some(a) = st.doc.artboards.first()
    {
        let (pw, ph) = (d.f64("pageW", 0.0) * PT_PER_MM, d.f64("pageH", 0.0) * PT_PER_MM);
        if pw > 0.0 && ph > 0.0 {
            o["scale"] = json!((a.rect.width() / pw).min(a.rect.height() / ph).clamp(0.001, 1000.0));
        }
    }
    o
}

/// Opens or places the drawing with the options chosen, remembering them for the next time.
///
/// Fails when the file couldn't be read or is no longer loaded.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if !d.bool("readable") {
        return Err(d.str("error"));
    }
    let file = app.ui.dialog_file.clone().ok_or("the drawing is no longer loaded")?;
    let cdr = options(app, d);
    app.ui.cdr_import = Value::Object(REMEMBERED.iter().map(|k| (k.to_string(), cdr[*k].clone())).collect());
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    let r = if d.str("mode") == "place" {
        let mut p = with_file(d.fields.get("__place"), d, &file);
        p["cdr"] = cdr;
        crate::place::run(app, &p).map(|_| ())
    } else {
        // Opened on a thread of its own, with its progress in the status bar.
        let params = json!({ "cdr": cdr });
        match path.as_deref().map(|p| crate::opening::start_with(app, p, file.bytes.clone(), params.clone())) {
            Some(Ok(_)) => Ok(()),
            _ => io::open_document(app, &d.str("name"), &file.bytes, path.clone(), &params).map(|_| ()),
        }
    };
    match r {
        Ok(()) => {
            if app.ui.dialog.as_ref().is_some_and(|x| x.kind == KIND) {
                app.ui.dialog = None;
            }
            app.status(format!("{}: {}", tl!("CorelDRAW document read"), d.str("name")));
            Ok(Value::Null)
        }
        Err(e) => {
            app.ui.dialog = Some(d.clone());
            Err(e)
        }
    }
}

/// Draws the dialog: the file's product, format version and container, then the import options.
fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::subheader(ui, tl!("File"));
    widgets::dim_label(ui, &format!("{} · {} {} · {}", d.str("product"), tl!("format version"), d.f64("version", 0.0), d.str("container")));
    if d.bool("readable") {
        widgets::dim_label(
            ui,
            &format!(
                "{} {} · {} {} · {:.1} × {:.1} mm",
                d.f64("pages", 0.0),
                tl!("page(s)"),
                d.f64("objects", 0.0),
                tl!("object(s)"),
                d.f64("pageW", 0.0),
                d.f64("pageH", 0.0)
            ),
        );
    } else {
        super::form::caption(
            ui,
            &format!("{} {}", tl!("This file can't be read:"), d.fields.get("error").and_then(Value::as_str).unwrap_or_default()),
        );
    }
    if let Some(n) = d.fields.get("note").and_then(Value::as_str) {
        super::form::caption(ui, n);
    }
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Scale and Structure"));
    let place = d.str("mode") == "place";
    if widgets::check(ui, tl!("Preserve Original Document Scale"), d.bool("preserveScale") || !place, place) {
        d.fields.insert("preserveScale".into(), json!(!d.bool("preserveScale")));
    }
    if place && !d.bool("preserveScale") {
        super::form::caption(ui, tl!("The first page is scaled to fit the first artboard."));
    }
    check(ui, d, "preserveLayers", tl!("Preserve Layers and Groups"));
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Text"));
    let outlines = d.bool("textToOutlines");
    if widgets::radio(ui, tl!("Keep Text Editable When Possible"), !outlines, true) {
        d.fields.insert("textToOutlines".into(), json!(false));
    }
    if widgets::radio(ui, tl!("Convert Text to Outlines"), outlines, true) {
        d.fields.insert("textToOutlines".into(), json!(true));
    }
    check(ui, d, "warnMissingFonts", tl!("Warn About Missing Fonts"));
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Compatibility"));
    check(ui, d, "reportUnsupported", tl!("Report Unsupported Effects"));
    super::form::caption(ui, tl!("Editable vectors are never turned into images. What can't be kept is listed in the import notes."));
    false
}
