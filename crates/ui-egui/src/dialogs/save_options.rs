//! A save format's options before the file is written; on the web, where there is no save panel,
//! also the Save As dialog that names the file and picks the format (SVG and PDF then go on to
//! their own dialogs). The rows come from the format's options in the engine's format table, so a
//! format gains rows as its encoder gains options.
//!
//! Fields: `__action` (the save command it finishes), `__pick` (file name and format
//! editable), `path`, `format`, and one field per option (agents set them with `ui.dialog.set`).
//! Native and `.ai` files show their save options (VectorCraft Options after Save As): each
//! artboard to a separate file, with `all` (true: every artboard) or `range`, Include Linked Files,
//! Embed ICC Profiles, Create PDF-Compatible File and Use Compression. A `.ai` file's Version comes
//! first: PDF-compatible, or legacy version 7 (`aiVersion: "7"`, a file per artboard), which takes
//! only Transparent Images (`transparentImages`), Hidden Objects (`hiddenObjects`: a hidden layer,
//! omitted or made visible), Text (`outlineText`: editable or outlines) and, with several
//! artboards, a picker of the artboards to save (thumbnails to check, field `boards`, saved as
//! `range`).

use std::sync::OnceLock;

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format, FormatOption, SAVE_FORMATS, SaveMode, TEMPLATE_FORMATS};

use super::{DialogResult, DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

/// Dialog kind.
pub const KIND: &str = "saveOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading, body, confirm, min_width: 380.0, ok_enabled: Some(can_save), inputs: |_| &["all", "boards"], ..DialogSpec::FORM };

/// OK saves a legacy `.ai` file only with an artboard checked.
fn can_save(app: &VectorcraftApp, d: &Dialog) -> bool {
    !legacy_ai(d) || picked_boards(d, app.session.active().map_or(0, |st| st.doc.artboards.len())).iter().any(|b| *b)
}

fn heading(d: &Dialog) -> String {
    if d.bool("__pick") {
        return match SaveMode::of(&d.str("__action")) {
            Some(SaveMode::Copy) => tl!("Save a Copy"),
            Some(SaveMode::Template) => tl!("Save as Template"),
            _ => tl!("Save As"),
        }
        .into();
    }
    if legacy_ai(d) {
        return tl!("Legacy .ai Options").into();
    }
    crate::i18n::fmt(tl!("{format} Options"), &[("format", fileio::format(&d.str("format")).map_or("", |f| f.label))])
}

/// `f`'s options with the values a save would use (`file.formatOptions`: as last saved in that
/// format, else the defaults). Text options start empty rather than null.
pub(crate) fn option_values(app: &mut VectorcraftApp, f: &Format) -> Map<String, Value> {
    let v = app.session.execute("file.formatOptions", &json!({ "format": f.id })).unwrap_or_default();
    f.options
        .iter()
        .map(|o| {
            let value = v["options"][o.name]["value"].clone();
            (o.name.to_string(), if o.ty == "string" && value.is_null() { json!("") } else { value })
        })
        .collect()
}

/// Formats whose options have a dialog of their own (SVG Options, Save PDF), which follows this
/// one.
fn own_dialog(f: &Format) -> bool {
    matches!(f.id, "svg" | "svgz" | "pdf")
}

/// Does `f` save each artboard to a separate file on request (native and `.ai` files)? Its
/// `range` then shows under that checkbox.
fn separates(f: &Format) -> bool {
    f.options.iter().any(|o| o.name == "separateArtboards")
}

/// The options the dialog shows: arrays and objects are for agents, and a page range replaces the
/// single artboard choice; none for a format with its own dialog. A `.ait` template has no
/// Version: Save as Template writes it PDF-compatible only.
fn shown(f: &Format) -> impl Iterator<Item = &'static FormatOption> {
    let range = f.options.iter().any(|o| o.name == "range");
    let separate = separates(f);
    let options = if own_dialog(f) { &[][..] } else { f.options };
    options.iter().filter(move |o| {
        !(matches!(o.ty, "array" | "object")
            || (range && o.name == "artboard")
            || (separate && o.name == "range")
            || (matches!(f.id, "ai" | "ait") && o.name == "useArtboards")
            // EPS Save keeps a page and the complete editing document; cropped and
            // selected-only output belongs to Export, never this save dialog.
            || (f.id == "eps" && matches!(o.name, "useArtboards" | "selectedOnly"))
            || (f.id == "ait" && o.name == "aiVersion"))
    })
}

/// An option's label: the save options as the reference app's options dialog names them, the
/// others from their names.
fn label(o: &FormatOption) -> String {
    match o.name {
        "separateArtboards" => tl!("Save each artboard to a separate file").into(),
        "includeLinked" => tl!("Include Linked Files").into(),
        "embedProfiles" => tl!("Embed ICC Profiles").into(),
        "pdfCompatible" => tl!("Create PDF-Compatible File").into(),
        "compress" => tl!("Use Compression").into(),
        _ => form::humanize(o.name),
    }
}

/// Every artboard (the All choice under Save each artboard to a separate file): `all` when set,
/// else no range given.
fn all_artboards(d: &Dialog) -> bool {
    d.fields.get("all").and_then(Value::as_bool).unwrap_or_else(|| d.str("range").trim().is_empty())
}

/// All or Range, under Save each artboard to a separate file (enabled while it is on).
fn artboard_range(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.label("");
    ui.add_enabled_ui(d.bool("separateArtboards"), |ui| {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            let mut all = all_artboards(d);
            if ui.radio_value(&mut all, true, tl!("All")).changed() | ui.radio_value(&mut all, false, tl!("Range:")).changed() {
                d.fields.insert("all".into(), json!(all));
            }
            let mut range = d.str("range");
            let field = egui::TextEdit::singleline(&mut range).align(egui::Align2::LEFT_CENTER).desired_width(90.0).hint_text("1-3, 5");
            if ui.add_enabled(!all, field).changed() {
                d.fields.insert("range".into(), json!(range));
            }
        });
    });
    ui.end_row();
}

/// The Save As format menu (labels in [`SAVE_FORMATS`] order).
fn save_labels() -> &'static [&'static str] {
    static LABELS: OnceLock<Vec<&'static str>> = OnceLock::new();
    LABELS.get_or_init(|| SAVE_FORMATS.iter().filter_map(|id| fileio::format(id)).map(|f| f.label).collect())
}

/// The Save as Template format menu (labels in [`TEMPLATE_FORMATS`] order).
fn template_labels() -> &'static [&'static str] {
    static LABELS: OnceLock<Vec<&'static str>> = OnceLock::new();
    LABELS.get_or_init(|| TEMPLATE_FORMATS.iter().filter_map(|id| fileio::format(id)).map(|f| f.label).collect())
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let Some(f) = fileio::format(&d.str("format")) else { return false };
    let mut switch = None;
    egui::Grid::new("save-options").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        if d.bool("__pick") {
            form::field_w(ui, d, "path", tl!("File Name:"), 240.0);
            let (ids, labels) = if SaveMode::of(&d.str("__action")) == Some(SaveMode::Template) {
                (TEMPLATE_FORMATS, template_labels())
            } else {
                (SAVE_FORMATS, save_labels())
            };
            ui.label(egui::RichText::new(tl!("Format:")).color(t.text_dim));
            switch = widgets::dropdown(ui, "save-format", f.label, labels, 254.0).and_then(|i| ids.get(i).copied());
            ui.end_row();
        }
        // Version first; the legacy version takes only its own options.
        let legacy = legacy_ai(d);
        let mut rows: Vec<&FormatOption> = shown(f).filter(|o| LEGACY_ONLY.contains(&o.name) == legacy || o.name == "aiVersion").collect();
        rows.sort_by_key(|o| o.name != "aiVersion");
        for o in rows {
            option_row(app, ui, d, o);
        }
    });
    if legacy_ai(d) && app.session.active().is_some_and(|st| st.doc.artboards.len() > 1) {
        artboard_picker(app, ui, d);
    }
    if let Some(id) = switch.filter(|id| *id != f.id) {
        switch_format(app, d, id);
    }
    false
}

/// The `.ai` options only the legacy version takes (and the only ones it takes, besides the
/// version).
const LEGACY_ONLY: &[&str] = &["transparentImages", "hiddenObjects", "outlineText"];

/// Is a `.ai` file to be saved in the legacy version 7 (`aiVersion: "7"`)?
fn legacy_ai(d: &Dialog) -> bool {
    d.str("format") == "ai" && d.str("aiVersion").trim() == "7"
}

/// The artboards a legacy `.ai` save writes (field `boards`: one bool per artboard), as last
/// saved (`range`), else all of them.
fn picked_boards(d: &Dialog, n: usize) -> Vec<bool> {
    if let Some(a) = d.fields.get("boards").and_then(Value::as_array) {
        let mut b: Vec<bool> = a.iter().map(|b| b.as_bool().unwrap_or(false)).collect();
        b.resize(n, false);
        return b;
    }
    let range = d.str("range");
    match (ArtboardPick { range: Some(range.clone()), ..Default::default() }).resolve(n) {
        Ok(Some(chosen)) if !range.trim().is_empty() => (0..n).map(|i| chosen.contains(&i)).collect(),
        _ => vec![true; n],
    }
}

/// The file each checked artboard is saved to (`None` for the others): the file itself when one
/// is checked, else `{stem}-{artboard}.ai` beside it, as the save names them.
fn board_files(app: &VectorcraftApp, d: &Dialog, boards: &[bool]) -> Vec<Option<String>> {
    let checked: Vec<usize> = boards.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i).collect();
    let path = d.str("path");
    let file = std::path::Path::new(path.trim());
    let name = file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut names = match (app.session.active(), checked.len()) {
        (_, 1) => vec![name],
        (Some(st), _) => fileio::artboard_file_names(&st.doc, &checked).into_iter().map(|b| format!("{stem}-{b}.ai")).collect(),
        (None, _) => vec![],
    }
    .into_iter();
    boards.iter().map(|on| if *on { names.next() } else { None }).collect()
}

/// `text` as it fits in `width` with `font`: whole, else its start cut short before its last
/// `tail` characters ("desktop-i…-Artboard-1.ai"), else as much of its end as fits.
fn elide_before_tail(ui: &egui::Ui, text: &str, tail: usize, font: &egui::FontId, width: f32) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE).size().x <= width;
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let split = chars.len().saturating_sub(tail);
    let (head, end) = chars.split_at(split);
    let end: String = end.iter().collect();
    for p in (1..head.len()).rev() {
        let s = format!("{}…{end}", head.iter().take(p).collect::<String>());
        if fits(&s) {
            return s;
        }
    }
    for k in (1..=chars.len()).rev() {
        let s = format!("…{}", chars.iter().skip(chars.len() - k).collect::<String>());
        if fits(&s) {
            return s;
        }
    }
    "…".into()
}

/// Legacy `.ai`: the artboards to save, as thumbnails to check; each is saved as a file of its own.
fn artboard_picker(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    const TILE: egui::Vec2 = egui::vec2(116.0, 84.0);
    let t = Tokens::get(ui.ctx());
    let names: Vec<String> = app.session.active().map(|st| st.doc.artboards.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
    let n = names.len();
    let mut boards = picked_boards(d, n);
    let before = boards.clone();
    ui.add_space(10.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(tl!("A legacy .ai file holds one artboard: each artboard you check is saved as a separate file.")).color(t.text_dim),
        )
        .wrap(),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.small_button(tl!("Select All")).clicked() {
            boards.iter_mut().for_each(|b| *b = true);
        }
        if ui.small_button(tl!("Clear")).clicked() {
            boards.iter_mut().for_each(|b| *b = false);
        }
        let checked = boards.iter().filter(|b| **b).count();
        let (count, total) = (checked.to_string(), n.to_string());
        let text = crate::i18n::fmt(tl!("{count} of {total} artboards"), &[("count", &count), ("total", &total)]);
        ui.label(egui::RichText::new(text).color(if checked == 0 { t.warning } else { t.text_dim }));
        if checked == 0 {
            ui.label(egui::RichText::new(tl!("Check an artboard to save.")).color(t.warning));
        }
    });
    ui.add_space(4.0);
    let files = board_files(app, d, &boards);
    let stem = std::path::Path::new(d.str("path").trim()).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
            for (i, name) in names.iter().enumerate() {
                let on = boards.get(i).copied().unwrap_or(false);
                ui.allocate_ui(egui::vec2(TILE.x, TILE.y + 44.0), |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(TILE.x);
                        let (r, resp) = ui.allocate_exact_size(TILE, egui::Sense::click());
                        ui.painter().rect_filled(r, 3.0, t.pasteboard);
                        let inner = r.shrink(6.0);
                        match super::export_for_screens::artboard_thumb(app, ui.ctx(), i) {
                            Some(tex) => {
                                let sz = tex.size_vec2();
                                let s = (inner.width() / sz.x).min(inner.height() / sz.y).min(1.0);
                                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                ui.painter().image(tex.id(), egui::Rect::from_center_size(inner.center(), sz * s), uv, egui::Color32::WHITE);
                            }
                            None => {
                                ui.painter().rect_filled(inner, 0.0, egui::Color32::WHITE);
                            }
                        }
                        let stroke = if on { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, t.border) };
                        ui.painter().rect_stroke(r, 3.0, stroke, egui::StrokeKind::Inside);
                        let mut flip = resp.on_hover_text(name.as_str()).clicked();
                        flip |= widgets::check_name(ui, name, on);
                        let file = files.get(i).cloned().flatten().unwrap_or_default();
                        // The end tells the files apart: the start gives way first.
                        let font = egui::TextStyle::Small.resolve(ui.style());
                        let tail = file.strip_prefix(stem.as_str()).map_or(0, |t| t.chars().count());
                        let shown = elide_before_tail(ui, &file, tail, &font, TILE.x);
                        let label = ui.add(egui::Label::new(egui::RichText::new(&shown).small().color(t.text_dim)));
                        if shown != file {
                            label.on_hover_text(file);
                        }
                        if flip && let Some(b) = boards.get_mut(i) {
                            *b ^= true;
                        }
                    });
                });
            }
        });
    });
    if boards != before || !d.fields.contains_key("boards") {
        d.fields.insert("boards".into(), json!(boards));
    }
}

/// One option: a checkbox, an artboard menu, a version menu, a number or a text field.
fn option_row(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, o: &FormatOption) {
    let t = Tokens::get(ui.ctx());
    let label = label(o);
    let value = d.fields.get(o.name).cloned().unwrap_or(Value::Null);
    if o.name == "aiVersion" {
        ui.label(egui::RichText::new(tl!("Version:")).color(t.text_dim)).on_hover_text(o.description);
        let versions = [tl!("PDF-compatible"), tl!("Legacy (version 7)")];
        let at = usize::from(legacy_ai(d));
        if let Some(i) = widgets::dropdown(ui, ("save-option", o.name), versions.get(at).copied().unwrap_or_default(), &versions, 254.0) {
            d.fields.insert(o.name.into(), json!(if i == 1 { "7" } else { "pdf" }));
        }
        ui.end_row();
        return;
    }
    if o.name == "transparentImages" {
        ui.label(egui::RichText::new(tl!("Transparent Images:")).color(t.text_dim)).on_hover_text(o.description);
        let choices = [tl!("Flatten"), tl!("Omit")];
        let at = usize::from(d.str("transparentImages").trim().eq_ignore_ascii_case("omit"));
        if let Some(i) = widgets::dropdown(ui, ("save-option", o.name), choices.get(at).copied().unwrap_or_default(), &choices, 254.0) {
            d.fields.insert(o.name.into(), json!(if i == 1 { "omit" } else { "flatten" }));
        }
        ui.end_row();
        return;
    }
    if o.name == "hiddenObjects" {
        ui.label(egui::RichText::new(tl!("Hidden Objects:")).color(t.text_dim)).on_hover_text(o.description);
        let choices = [tl!("Hidden Layer"), tl!("Omit"), tl!("Make Visible")];
        let at = match d.str("hiddenObjects").trim() {
            "omit" => 1,
            "visible" => 2,
            _ => 0,
        };
        if let Some(i) = widgets::dropdown(ui, ("save-option", o.name), choices.get(at).copied().unwrap_or_default(), &choices, 254.0) {
            d.fields.insert(o.name.into(), json!(["layer", "omit", "visible"].get(i).copied().unwrap_or("layer")));
        }
        ui.end_row();
        return;
    }
    if o.name == "outlineText" && legacy_ai(d) {
        ui.label(egui::RichText::new(tl!("Text:")).color(t.text_dim)).on_hover_text(o.description);
        let choices = [tl!("Editable"), tl!("Outlines")];
        let at = usize::from(value.as_bool().unwrap_or(false));
        if let Some(i) = widgets::dropdown(ui, ("save-option", o.name), choices.get(at).copied().unwrap_or_default(), &choices, 254.0) {
            d.fields.insert(o.name.into(), json!(i == 1));
        }
        ui.end_row();
        return;
    }
    if o.ty == "boolean" {
        ui.label("");
        let on = value.as_bool().unwrap_or(false);
        if widgets::check_tip(ui, label.trim_end_matches(':'), on, true, o.description) {
            d.fields.insert(o.name.into(), json!(!on));
        }
        ui.end_row();
        if o.name == "separateArtboards" {
            artboard_range(ui, d);
        }
        return;
    }
    if o.ty == "string" {
        form::field_w(ui, d, o.name, &label, 240.0);
        return;
    }
    ui.label(egui::RichText::new(&label).color(t.text_dim)).on_hover_text(o.description);
    let artboards = app.session.active().filter(|_| o.name == "artboard").map(|st| &st.doc.artboards);
    if let Some(boards) = artboards {
        let i = value.as_u64().unwrap_or(0) as usize;
        let names: Vec<&str> = boards.iter().map(|a| a.name.as_str()).collect();
        if let Some(i) = widgets::dropdown_names(ui, ("save-option", o.name), names.get(i).copied().unwrap_or(""), &names, 254.0) {
            d.fields.insert(o.name.into(), json!(i));
        }
    } else {
        let integer = o.ty == "integer";
        let x = value.as_f64().unwrap_or(0.0);
        let field = ui.scope(|ui| widgets::range_field(ui, ("save-option", o.name), x, 0.0..=f64::MAX, "", if integer { 0 } else { 2 }, 80.0));
        field.response.on_hover_text(o.description);
        if let Some(x) = field.inner {
            d.fields.insert(o.name.into(), if integer { json!(x as i64) } else { json!(x) });
        }
    }
    ui.end_row();
}

/// Web Save As: another format brings its own options and file extension.
fn switch_format(app: &mut VectorcraftApp, d: &mut Dialog, id: &str) {
    let (Some(old), Some(new)) = (fileio::format(&d.str("format")), fileio::format(id)) else { return };
    for o in old.options {
        d.fields.remove(o.name);
    }
    d.fields.extend(option_values(app, new));
    d.fields.insert("format".into(), json!(new.id));
    d.fields.insert("path".into(), json!(with_extension(&d.str("path"), new)));
}

/// `path` ending in `f`'s extension (kept when it already names `f`).
fn with_extension(path: &str, f: &Format) -> String {
    if fileio::format_for_name(path).is_some_and(|g| g.id == f.id) {
        return path.to_string();
    }
    std::path::Path::new(path).with_extension(f.extensions[0]).to_string_lossy().to_string()
}

/// Write the file with the chosen options (blank ones fall back to the format's defaults).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let f = fileio::format(&d.str("format")).ok_or("unknown format")?;
    let path = d.str("path");
    if path.trim().is_empty() {
        return Err("name the file".into());
    }
    let path = if d.bool("__pick") { with_extension(path.trim(), f) } else { path };
    let mut options: Map<String, Value> = f
        .options
        .iter()
        .filter_map(|o| d.fields.get(o.name).filter(|v| !v.is_null() && v.as_str() != Some("")).map(|v| (o.name.to_string(), v.clone())))
        .collect();
    if legacy_ai(d) {
        // The artboards checked in the picker (all of them: no range).
        options.remove("range");
        let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
        let boards = picked_boards(d, n);
        if !boards.iter().any(|b| *b) {
            return Err(tl!("Check an artboard to save.").into());
        }
        if !boards.iter().all(|b| *b) {
            options.insert("range".into(), json!(super::export_for_screens::range_text(&boards)));
        }
    } else if separates(f) {
        if !d.bool("separateArtboards") || all_artboards(d) {
            options.remove("range");
        } else {
            // A bad range keeps the dialog open.
            let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
            ArtboardPick { range: Some(d.str("range")), ..Default::default() }.resolve(n)?;
        }
    }
    app.ui.dialog = None;
    match SaveMode::of(&d.str("__action")) {
        Some(mode) if own_dialog(f) => io::ask_format_options(app, mode, f, &path),
        Some(mode) => io::save(app, mode, &json!({ "path": path, "format": f.id, "options": options }), false),
        None => Err(format!("`{}` is no save command", d.str("__action"))),
    }
}

#[cfg(test)]
mod tests;
