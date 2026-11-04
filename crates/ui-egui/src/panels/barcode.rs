//! Barcode panel: the symbology, the data and the sizing of a barcode, and the button that draws
//! one.
//!
//! Every field here ends in `barcode.create`, `barcode.set` or `barcode.createBatch`, so the panel
//! and an agent driving the same document over MCP take exactly the same path. A barcode object
//! selected on the canvas is what the panel edits — the fields show its settings and every option
//! redraws it at once — and Create draws a new one from the same fields at any time.
//!
//! The panel is laid out the way a packaging shop reads it: what the code is at the top, then what
//! goes in it, then how big it comes out, then how it is drawn. Everything is in labelled columns
//! so the numbers line up and the labels can be scanned rather than read.

use egui::Ui;
use serde_json::json;
use vectorcraft_barcode::{BarcodeSpec, Ecc, Symbology};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
use crate::state::BarcodeBatch;
use crate::widgets;

/// The panel id, as the dock, the Window menu and `window.panel` name it.
pub const ID: &str = "barcode";

/// The symbology the panel starts on: the retail article number, which is what most packaging
/// needs.
const DEFAULT_SYMBOLOGY: Symbology = Symbology::Ean13;

/// A fresh draft: OCR-B for the human readable line when the machine has it, since that is the
/// face GS1 §5.2.4 recommends for it, and the document's own font otherwise.
fn draft_font(spec: &mut BarcodeSpec) {
    if spec.font.is_none() {
        spec.font = standard_font();
    }
}

/// The face the digits are set in when the panel does not name one: the machine's OCR-B, which is
/// the one the barcode standards name for the human readable line, or `None` for the document's own
/// font when the machine has no OCR-B.
pub(crate) fn standard_font() -> Option<String> {
    vectorcraft_text::FontDb::global().ocr_b()
}

/// What the symbology takes, in the interface's language — the crate's own `hint` is the English
/// source these catalogs are keyed on, and a packaging shop reading the panel should not have to
/// read it in English.
pub(super) fn hint(s: Symbology) -> &'static str {
    match s {
        Symbology::Ean13 => tl!("12 or 13 digits"),
        Symbology::Ean8 => tl!("7 or 8 digits"),
        Symbology::UpcA => tl!("11 or 12 digits"),
        Symbology::UpcE => tl!("6 or 8 digits"),
        Symbology::Ean128 => tl!("digits, with or without the (01) application identifier"),
        Symbology::Code128 | Symbology::Code128B => tl!("printable ASCII"),
        Symbology::Code128A => tl!("printable ASCII, upper case and control characters"),
        Symbology::Code128C | Symbology::Itf14 | Symbology::Itf25 => tl!("an even number of digits"),
        Symbology::Code39 => tl!("digits, A–Z and - . $ / + % and space"),
        Symbology::Code93 => tl!("ASCII"),
        Symbology::QrCode => tl!("any text"),
    }
}

/// The barcode object the selection names, or the one the selection is inside.
fn selected(app: &VectorcraftApp) -> Option<(NodeId, BarcodeSpec)> {
    let st = app.session.active()?;
    for id in st.selection.objects.iter().copied() {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if let Some(spec) = st.doc.node(c).and_then(|n| n.extras().barcode.as_deref()) {
                return Some((c, spec.clone()));
            }
            cur = st.doc.parent_of(c);
        }
    }
    None
}

/// Whether this frame's edits are drawn on the selected object at once rather than waiting for
/// Update: everything is. A control that looks live but leaves the art alone reads as broken, and
/// the text fields commit only on Enter or when the focus leaves them — never per keystroke — so
/// nothing redraws halfway through a value being typed.
pub(super) fn applies_at_once(spec: &BarcodeSpec, before: &BarcodeSpec) -> bool {
    spec != before
}

/// A draft for `symbology`: the example data that type is usually shown with, and the standard
/// font for its digits.
fn draft(symbology: Symbology) -> BarcodeSpec {
    let mut spec = BarcodeSpec { data: symbology.sample_data().to_string(), symbology, ..BarcodeSpec::default() };
    draft_font(&mut spec);
    spec
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let target = selected(app);
    let mut spec = match &target {
        Some((_, spec)) => spec.clone(),
        None => app.ui.barcode.clone(),
    };
    // What the object holds before this frame's controls are drawn: the rows edit `spec`, and the
    // difference is what the selected object is redrawn from below.
    let before = spec.clone();
    let width = ui.available_width();

    // Changing the type replaces the value, so a value the encoder refused for the old type has
    // nothing left to correct.
    let type_before = spec.symbology;
    symbology_rows(ui, &mut spec, width);
    if spec.symbology != type_before {
        app.ui.barcode_rejected = None;
    }
    ui.add_space(6.0);
    widgets::divider(ui);
    content_rows(app, ui, &mut spec, width, target.as_ref().map(|(id, _)| *id));
    ui.add_space(2.0);
    if spec.symbology.is_matrix() {
        widgets::divider(ui);
        matrix_rows(ui, &mut spec, width);
    }
    ui.add_space(2.0);
    widgets::divider(ui);
    size_rows(ui, &mut spec);
    ui.add_space(2.0);
    widgets::divider(ui);
    appearance_rows(ui, &mut spec, width);
    ui.add_space(6.0);
    action_row(app, ui, &spec, &target);
    batch_section(app, ui, &spec, width);

    // The selected object follows the panel as it is edited. A control that looks live but leaves
    // the art alone reads as broken, and Update then has nothing left to do. Everything but the
    // value itself is drawn at once — one edit for the undo stack, the same as the draft is — while
    // a value being typed waits for Update or the field's Enter, so a half-typed number never
    // redraws the object.
    if let Some((id, _)) = &target
        && applies_at_once(&spec, &before)
    {
        let mut p = serde_json::to_value(&spec).unwrap_or(json!({}));
        p["id"] = json!(id.0);
        // A value the symbology cannot carry leaves the art as it was, says why, and stays in the
        // field so it can be corrected rather than typed again from the stored value.
        if let Err(e) = app.run("barcode.set", p) {
            app.ui.barcode_note = e;
            app.ui.barcode_rejected = Some((id.0, spec.data.clone()));
        } else {
            app.ui.barcode_rejected = None;
        }
    }

    if target.is_none() {
        // The draft the panel keeps is what was typed, once every row has had its say.
        app.ui.barcode = spec;
    }
}

/// The type, with the standard it belongs to beside the name and under it.
fn symbology_rows(ui: &mut Ui, spec: &mut BarcodeSpec, width: f32) {
    // The rows name the standard each symbology belongs to, the way the barcode tools a packaging
    // shop already uses do: "EAN-13" alone does not say whether it is the Chinese, Japanese or
    // American number.
    let labels: Vec<String> = Symbology::all().iter().map(|s| format!("{} · {}", s.label(), s.origin())).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let shown = format!("{} · {}", spec.symbology.label(), spec.symbology.origin());
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Type:"));
        let rest = (width - 46.0).max(80.0);
        let before = spec.symbology;
        if let Some(i) = widgets::dropdown_names(ui, "barcode-type", &shown, &labels, rest)
            && let Some(s) = Symbology::all().get(i)
        {
            spec.symbology = *s;
        }
        // Changing the type brings that type's own example data along, so the panel starts on a
        // value the new symbology can carry instead of on the one the last type was working with.
        if spec.symbology != before {
            spec.data = spec.symbology.sample_data().to_string();
        }
    });
    widgets::dim_label(ui, spec.symbology.standard());
}

/// What goes into the code, the check that says whether it will, and how big it comes out.
fn content_rows(app: &mut VectorcraftApp, ui: &mut Ui, spec: &mut BarcodeSpec, width: f32, target: Option<NodeId>) {
    widgets::section_header(ui, "Content");
    // A value the encoder refused is what the field shows, so it can be corrected in place rather
    // than reverting to the stored value while the pointer is still in the box.
    let rejected = app.ui.barcode_rejected.as_ref().filter(|(id, _)| target.is_some_and(|t| t.0 == *id)).map(|(_, value)| value.clone());
    let shown = rejected.unwrap_or_else(|| spec.data.clone());
    let mut asked = false;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Data:"));
        let field = (width - 46.0 - 58.0).max(80.0);
        if let Some(v) = widgets::text_field(ui, "barcode-data", Some(shown.as_str()), field, 1) {
            spec.data = v;
            // A new value is a new question: the last answer no longer applies to it.
            app.ui.barcode_note.clear();
        }
        asked = ui.button(tl!("Check")).clicked();
    });
    widgets::dim_label(ui, hint(spec.symbology));
    if asked {
        let params = serde_json::to_value(&*spec).unwrap_or(json!({}));
        app.ui.barcode_note = match app.run("barcode.check", params) {
            Ok(out) if out["ok"] == json!(true) => format!("{} {}", tl!("Valid:"), out["data"].as_str().unwrap_or("")),
            Ok(out) => out["message"].as_str().unwrap_or("").to_string(),
            Err(e) => e,
        };
    }
    if !app.ui.barcode_note.is_empty() {
        let t = crate::theme::Tokens::get(ui.ctx());
        let ok = app.ui.barcode_note.starts_with(tl!("Valid:"));
        ui.colored_label(if ok { t.success } else { t.error }, app.ui.barcode_note.clone());
    }
    match vectorcraft_barcode::encode(spec) {
        Ok(code) => {
            let (w, h) = code.size_mm();
            widgets::dim_label(ui, &format!("{w:.2} × {h:.2} mm"));
        }
        Err(e) => {
            ui.colored_label(crate::theme::Tokens::get(ui.ctx()).error, format!("{e}"));
        }
    }
}

/// The two columns of numbers: a label and its field, twice a row.
fn size_rows(ui: &mut Ui, spec: &mut BarcodeSpec) {
    widgets::section_header(ui, "Size");
    let field = 76.0;
    egui::Grid::new("barcode-size").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Magnification:"));
        if let Some(v) = widgets::mixed_field(ui, "barcode-magnification", Some(spec.magnification * 100.0), "%", 0, field) {
            spec.magnification = (v / 100.0).clamp(0.25, 10.0);
        }
        widgets::dim_label(ui, tl!("Module:"));
        let shown = spec.module_mm.unwrap_or_else(|| spec.symbology.nominal_module_mm());
        if let Some(v) = widgets::mixed_field(ui, "barcode-module", Some(shown), " mm", 3, field) {
            spec.module_mm = (v.is_finite() && v > 0.0).then_some(v);
        }
        ui.end_row();

        if !spec.symbology.is_matrix() {
            widgets::dim_label(ui, tl!("Height:"));
            let shown = spec.height_mm.unwrap_or_else(|| spec.symbology.nominal_height_mm());
            if let Some(v) = widgets::mixed_field(ui, "barcode-height", Some(shown), " mm", 2, field) {
                spec.height_mm = (v.is_finite() && v > 0.0).then_some(v);
            }
        } else {
            ui.label("");
        }
        widgets::dim_label(ui, tl!("Quiet Zone:"));
        if let Some(v) = widgets::mixed_field(ui, "barcode-quiet", Some(spec.quiet_extra), "", 0, field) {
            spec.quiet_extra = v.clamp(0.0, 100.0);
        }
        ui.end_row();

        if matches!(spec.symbology, Symbology::Code39 | Symbology::Itf14 | Symbology::Itf25) {
            widgets::dim_label(ui, tl!("Width Ratio:"));
            let nominal = spec.ratio.unwrap_or(if spec.symbology == Symbology::Code39 { 3.0 } else { 2.5 });
            if let Some(v) = widgets::mixed_field(ui, "barcode-ratio", Some(nominal), "", 2, field) {
                spec.ratio = Some(v.clamp(2.0, 3.0));
            }
            ui.label("");
            ui.label("");
            ui.end_row();
        }
    });
}

/// A QR Code's error correction, version and mask.
fn matrix_rows(ui: &mut Ui, spec: &mut BarcodeSpec, _width: f32) {
    widgets::section_header(ui, "QR Code");
    let field = 86.0;
    egui::Grid::new("barcode-matrix").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        let levels: Vec<&str> = Ecc::all().iter().map(|e| e.label()).collect();
        widgets::dim_label(ui, tl!("Error Correction:"));
        if let Some(i) = widgets::dropdown_names(ui, "barcode-ecc", spec.ecc.label(), &levels, field)
            && let Some(e) = Ecc::all().get(i)
        {
            spec.ecc = *e;
        }
        ui.end_row();

        widgets::dim_label(ui, tl!("Version:"));
        let shown = spec.version.map_or_else(|| tl!("Automatic").to_string(), |v| v.to_string());
        let options: Vec<String> = std::iter::once(tl!("Automatic").to_string()).chain((1..=40).map(|v| v.to_string())).collect();
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "barcode-version", &shown, &refs, field) {
            spec.version = (i > 0).then_some(i as u8);
        }
        ui.end_row();

        widgets::dim_label(ui, tl!("Mask:"));
        let shown = spec.mask.map_or_else(|| tl!("Automatic").to_string(), |v| v.to_string());
        let options: Vec<String> = std::iter::once(tl!("Automatic").to_string()).chain((0..8).map(|v| v.to_string())).collect();
        let refs: Vec<&str> = options.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "barcode-mask", &shown, &refs, field) {
            spec.mask = (i > 0).then_some((i - 1) as u8);
        }
        ui.end_row();
    });
}

/// How the code is drawn: the human readable line and its face, and the white box behind it.
fn appearance_rows(ui: &mut Ui, spec: &mut BarcodeSpec, width: f32) {
    widgets::section_header(ui, "Appearance");
    if !spec.symbology.is_matrix() {
        ui.horizontal(|ui| {
            if widgets::check(ui, tl!("Human Readable Line"), spec.text, true) {
                spec.text = !spec.text;
            }
            if widgets::check(ui, tl!("Outline the digits"), spec.outline_text, spec.text) {
                spec.outline_text = !spec.outline_text;
            }
        });
    }
    if widgets::check(ui, tl!("White background box"), spec.background, true) {
        spec.background = !spec.background;
    }
    if spec.symbology.is_matrix() {
        return;
    }
    let families = vectorcraft_text::FontDb::global().families();
    // The first entry names the face the engine will actually set the digits in when the spec does
    // not, rather than a "Document Default" that a machine with OCR-B would silently ignore.
    let standard = standard_font();
    let first = standard.clone().unwrap_or_else(|| tl!("Document Default").to_string());
    let shown = spec.font.clone().unwrap_or_else(|| first.clone());
    let options: Vec<String> = std::iter::once(first.clone()).chain(families.iter().cloned()).collect();
    let refs: Vec<&str> = options.iter().map(String::as_str).collect();
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Font:"));
        if let Some(i) = widgets::dropdown_names(ui, "barcode-font", &shown, &refs, (width - 46.0).max(80.0)) {
            spec.font = (i > 0).then(|| options.get(i).cloned().unwrap_or_default());
        }
    });
    if families.is_empty() {
        widgets::dim_label(ui, tl!("No fonts are installed."));
    } else if standard.is_none() {
        widgets::dim_label(ui, tl!("Install OCR-B for the standard digits."));
    }
}

/// The one row that changes the document.
fn action_row(app: &mut VectorcraftApp, ui: &mut Ui, spec: &BarcodeSpec, target: &Option<(NodeId, BarcodeSpec)>) {
    ui.horizontal(|ui| {
        // Create is here whatever is selected: picking another type and making another code is what
        // the panel is mostly for, and it used to be reachable only with nothing selected — so
        // choosing a type after drawing one changed that barcode instead of making a new one.
        if ui.button(tl!("Create")).clicked() {
            let params = serde_json::to_value(spec).unwrap_or(json!({}));
            app.run("barcode.create", params).ok();
        }
        if let Some((id, _)) = target
            && ui.button(tl!("Update")).clicked()
        {
            let mut p = serde_json::to_value(spec).unwrap_or(json!({}));
            p["id"] = json!(id.0);
            app.run("barcode.set", p).ok();
        }
        if target.is_some() && ui.button(tl!("New")).clicked() {
            // Back to making one: until something else is selected the panel keeps editing what is
            // on the canvas, which is what made a second code impossible to start.
            app.run("select.none", json!({})).ok();
            app.ui.barcode = draft(spec.symbology);
        }
        if ui.button(tl!("Reset")).clicked() {
            app.ui.barcode = draft(DEFAULT_SYMBOLOGY);
        }
    });
}

/// The folded batch section: many codes at once, one a line, laid out as a grid.
fn batch_section(app: &mut VectorcraftApp, ui: &mut Ui, spec: &BarcodeSpec, width: f32) {
    ui.add_space(4.0);
    let open = app.ui.barcode_batch_open;
    if widgets::section_toggle(ui, "Batch", open) {
        app.ui.barcode_batch_open = !open;
    }
    if open {
        batch_rows(app, ui, spec, width);
    }
}

fn batch_rows(app: &mut VectorcraftApp, ui: &mut Ui, spec: &BarcodeSpec, width: f32) {
    let mut batch = app.ui.barcode_batch.clone();
    if let Some(v) = widgets::text_field(ui, "barcode-batch-data", Some(batch.data.as_str()), width, 5) {
        batch.data = v;
    }
    // What goes in the box, since an empty multi-line field says nothing about it.
    widgets::dim_label(ui, tl!("One code a line, each a whole value of the type above."));
    let field = ((width - 132.0) / 2.0).clamp(40.0, 90.0);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Max per Row:"));
        if let Some(v) = widgets::mixed_field(ui, "barcode-batch-columns", Some(f64::from(batch.columns)), "", 0, field) {
            batch.columns = v.clamp(1.0, 200.0) as u32;
        }
        widgets::dim_label(ui, tl!("Gap:"));
        if let Some(v) = widgets::mixed_field(ui, "barcode-batch-gap", Some(batch.gap), " mm", 1, field) {
            batch.gap = v.clamp(0.0, 1000.0);
        }
    });
    let count = batch_entries(&batch.data, spec.symbology).len();
    widgets::dim_label(ui, &format!("{} {count}", tl!("Codes:")));
    let mut run = false;
    ui.horizontal(|ui| {
        run = ui.add_enabled(count > 0, egui::Button::new(tl!("Create Batch"))).clicked();
        if ui.button(tl!("Clear")).clicked() {
            batch = BarcodeBatch::default();
        }
    });
    if run {
        let mut p = serde_json::to_value(spec).unwrap_or(json!({}));
        p["data"] = json!(batch_entries(&batch.data, spec.symbology).join("\n"));
        p["columns"] = json!(batch.columns);
        p["gapX"] = json!(batch.gap);
        p["gapY"] = json!(batch.gap);
        app.ui.barcode_batch_note = match app.run("barcode.createBatch", p) {
            Ok(out) => {
                let drawn = out["count"].as_u64().unwrap_or(0);
                let skipped = out["skipped"].as_array().map_or(0, Vec::len);
                if skipped == 0 {
                    format!("{} {drawn}", tl!("Drawn:"))
                } else {
                    let first = out["skipped"].as_array().and_then(|a| a.first()).and_then(|v| v["message"].as_str()).unwrap_or("");
                    format!("{} {drawn}, {skipped} — {first}", tl!("Drawn:"))
                }
            }
            Err(_) => tl!("No line in the box is a code this type can carry.").to_string(),
        };
    }
    if !app.ui.barcode_batch_note.is_empty() {
        let t = crate::theme::Tokens::get(ui.ctx());
        let ok = app.ui.barcode_batch_note.starts_with(tl!("Drawn:"));
        ui.colored_label(if ok { t.success } else { t.error }, app.ui.barcode_batch_note.clone());
    }
    app.ui.barcode_batch = batch;
}

/// The codes pasted into the batch box, one per line — and one per comma too, when the whole paste
/// is on one line, since that is how a spreadsheet column and a CSV row arrive. Only the numeric
/// sets, which carry digits alone, can take a comma as a separator: Code 39's Full ASCII, Code 128,
/// Code 93 and a QR Code all carry one as data, so a value with one in it is one code, not two.
pub(super) fn batch_entries(text: &str, symbology: Symbology) -> Vec<String> {
    let digits_only =
        matches!(symbology, Symbology::Ean13 | Symbology::Ean8 | Symbology::UpcA | Symbology::UpcE | Symbology::Itf14 | Symbology::Itf25);
    let sep = if text.contains('\n') || !digits_only { '\n' } else { ',' };
    text.split(sep).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Items of the panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let mut any = false;
    if ui.button(tl!("Reset")).clicked() {
        app.ui.barcode = draft(DEFAULT_SYMBOLOGY);
        any = true;
    }
    any
}
