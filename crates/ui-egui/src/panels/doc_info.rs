//! Document Info panel: the document's setup and what it contains (objects by kind, graphic
//! styles, brushes, objects painted with spot colors, with patterns and with gradients, symbols, fonts and their files,
//! linked and embedded images, repeat objects), for the whole document or the selection only. The flyout picks one category (or all)
//! and saves the text report (`docInfo.save`, the report File → Package writes too).

use std::sync::Arc;

use craft::keymap::CommandRef;
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::style::TypeRole;
use craft::ui::text::SelectableText;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::CATEGORIES;

use crate::VectorcraftApp;

/// What the panel's menu sets: the scope and the category shown. Neither is saved with the
/// preferences.
#[derive(Clone, Debug, Default)]
pub struct DocInfoView {
    /// Only the selection's objects are counted.
    pub selection_only: bool,
    /// The id of the one category shown; empty for all of them.
    pub category: String,
}

/// A `label: value` row. `label` is shown as given: translate an interface label first, a name
/// stays as it is. Whether the value was clicked to copy it: a click with no key held, other than
/// the second or third click of a double or triple click.
pub(crate) fn row(ui: &mut Ui, label: &str, value: &str) -> bool {
    let mut clicked = false;
    Row::new().fill_width().align(RowAlign::SpaceBetween).wrap(true).show(ui, |ui, _| {
        Text::new(label.to_string()).shade(Shade::Muted).show(ui);
        // The value can be selected and copied, and a click copies all of it (#8432). A drag selects
        // and a Shift-click extends the selection, so neither copies.
        clicked = copy_value(ui, label, value, false);
    });
    clicked
}

/// The framework readout policy introduced for Document Info in #9353: the first plain
/// primary release copies immediately, while subsequent double-clicks select text.
pub(crate) fn copy_value(ui: &mut Ui, label: &str, value: &str, mono: bool) -> bool {
    let text = SelectableText::new(label.to_string(), value);
    let shown = if mono { text.mono().show(ui) } else { text.show(ui) };
    copy_shown(ui, &shown, value)
}

/// The same guarded full-value copy for an already shown readout. An elided path remains
/// static text: selection indices must describe the text actually shown by the framework.
pub(crate) fn copy_shown(ui: &mut Ui, shown: &egui::Response, value: &str) -> bool {
    // The click's own position and keys: a move or a key change after the release in the same
    // frame leaves the pointer and the held keys elsewhere. A frame with more than one release
    // copies nothing, since the click flags below do not say which release they describe.
    let area = shown.interact_rect.intersect(ui.clip_rect());
    let primary = egui::PointerButton::Primary;
    let press_id = shown.id.with("readout_copy_press");
    let saved_origin = ui.ctx().data(|d| d.get_temp::<egui::Pos2>(press_id));
    let (origin, release) = ui.input(|i| {
        let mut origin = saved_origin;
        let mut release = None;
        let mut release_count = 0_usize;
        // egui clears pointer.press_origin on release. Remember only a press that starts in
        // this clipped readout, and consume events in order even for a same-frame click.
        for event in &i.events {
            if let egui::Event::PointerButton { pos, button, pressed, modifiers } = event
                && *button == primary
            {
                if *pressed {
                    origin = area.contains(*pos).then_some(*pos);
                } else {
                    release_count += 1;
                    release = Some((origin.take(), *pos, *modifiers));
                }
            }
        }
        let single = i.pointer.button_clicked(primary) && !i.pointer.button_double_clicked(primary) && !i.pointer.button_triple_clicked(primary);
        (origin, release.filter(|_| release_count == 1 && single))
    });
    if origin != saved_origin {
        ui.ctx().data_mut(|d| match origin {
            Some(pos) => d.insert_temp(press_id, pos),
            None => d.remove::<egui::Pos2>(press_id),
        });
    }
    let eligible = ui.is_enabled() && !value.is_empty() && area.is_positive();
    ui.ctx().accesskit_node_builder(shown.id, |node| {
        node.set_description(tl!("Copy"));
        node.add_action(egui::accesskit::Action::Click);
    });
    let activated = ui.input(|i| i.num_accesskit_action_requests(shown.id, egui::accesskit::Action::Click) > 0);
    eligible
        && (activated
            || release.is_some_and(|(origin, pos, keys)| {
                origin.is_some() && keys.is_none() && area.contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
            }))
}

/// Whether no artwork is selected. Artboards the Artboard tool has selected are not artwork:
/// `document.info` lists them under Document whatever this says.
fn no_artwork_selected(app: &VectorcraftApp) -> bool {
    match app.session.active() {
        Some(st) => st.selection.objects.is_empty(),
        None => true,
    }
}

/// `document.info` with its categories, fetched again only when the document, the scope or the
/// artboards the Artboard tool has selected change. A frame that fetches nothing returns the stored
/// result without copying it and stores nothing; a failed fetch leaves the stored result as it was.
pub(crate) fn info(app: &mut VectorcraftApp) -> Option<Arc<Value>> {
    let selection_only = app.ui.doc_info.selection_only;
    let artboard_tool = if app.session.tool_id() == "artboard" { app.session.tool_options_of("artboard") } else { Value::Null };
    let st = app.session.active()?;
    let key = (st.uid, st.revision, selection_only, artboard_tool);
    if let Some((cached, info)) = &app.canvas.doc_info
        && *cached == key
    {
        return Some(Arc::clone(info));
    }
    let info = Arc::new(app.session.execute("document.info", &json!({ "selectionOnly": selection_only })).ok()?);
    app.canvas.doc_info = Some((key, Arc::clone(&info)));
    Some(info)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(i) = info(app) else {
        Text::new(tl!("No document").to_string()).shade(Shade::Muted).show(ui);
        return;
    };
    let DocInfoView { selection_only: sel_only, category } = app.ui.doc_info.clone();
    let mut copy = None;
    Stack::new().show(ui, |ui, _| {
        if sel_only && no_artwork_selected(app) {
            Text::new(tl!("(No artwork selected)").to_string()).shade(Shade::Muted).show(ui);
        }
        let sections = i["sections"].as_array().map_or(&[][..], Vec::as_slice);
        let shown = sections.iter().filter(|s| category.is_empty() || s["id"] == category.as_str());
        for (n, s) in shown.enumerate() {
            if n > 0 {
                Divider::horizontal().show(ui);
            }
            let title = s["title"].as_str().unwrap_or_default();
            let rows = s["rows"].as_array().map_or(&[][..], Vec::as_slice);
            // Lists show how many they hold; the selection scope shows on the first.
            let title = match s["id"].as_str() {
                Some("document" | "objects") => tl!(title).to_string(),
                _ => format!("{} ({})", tl!(title), rows.len()),
            };
            let title =
                if sel_only && n == 0 && category.is_empty() { crate::i18n::fmt(tl!("{title} (selection)"), &[("title", &title)]) } else { title };
            Text::new(title).role(TypeRole::Heading).show(ui);
            if rows.is_empty() {
                Text::new(tl!("None").to_string()).shade(Shade::Muted).show(ui);
            }
            let id = s["id"].as_str().unwrap_or_default();
            let named = named_rows(id, rows);
            for (k, r) in rows.iter().enumerate() {
                let (label, value) = (r[0].as_str().unwrap_or_default(), r[1].as_str().unwrap_or_default());
                if value.is_empty() {
                    if named.contains(&k) && !label.is_empty() {
                        if copy_value(ui, label, label, false) {
                            copy = Some(label.to_string());
                        }
                    } else {
                        Text::new(super::label_or_name(label, !named.contains(&k)).to_string()).shade(Shade::Muted).show(ui);
                    }
                } else {
                    let shown = shown_value(&i, id, k, (label, value), named.contains(&k)).unwrap_or_else(|| value.to_string());
                    if row(ui, super::label_or_name(label, !named.contains(&k)), &shown) {
                        copy = Some(shown);
                    }
                }
            }
        }
    });
    if let Some(text) = copy {
        app.clipboard_out = Some(text);
        app.status("Copied the value");
    }
}

/// A Brushes row's value in the interface language: the brush's type (`Art Brush`), or that the
/// Brushes panel does not hold the brush. `document.info` and the text report keep them in English.
fn brush_value(value: &str) -> &str {
    match value {
        "Calligraphic Brush" => tl!("Calligraphic Brush"),
        "Scatter Brush" => tl!("Scatter Brush"),
        "Art Brush" => tl!("Art Brush"),
        "Pattern Brush" => tl!("Pattern Brush"),
        "Bristle Brush" => tl!("Bristle Brush"),
        "missing from the Brushes panel" => tl!("missing from the Brushes panel"),
        _ => value,
    }
}

/// Row `k` of Document Info section `id` (`label` and `value` as `document.info` gives them, the
/// label a name when `named`) with its value in the interface language, built from what
/// `document.info` (`info`) gives as values: the
/// units, the raster effects resolution, each brush's type, each font's type and uses, each font's file, match and
/// embedding permission, each image's size and file, each Repeat object's kind and counts, and the
/// kinds of the gradients no swatch names. Names (fonts, files, faces, swatches) stay as they are.
/// None when the row's value is shown as `document.info` gives it, or the values do not match the
/// row.
fn shown_value(info: &Value, id: &str, k: usize, (label, value): (&str, &str), named: bool) -> Option<String> {
    let entry = |list: &str| info[list].get(k).filter(|e| e["name"] == label || e["font"] == label);
    match id {
        "document" if !named => match label {
            "Units" => {
                let unit = info["document"]["unitId"].as_str()?;
                vectorcraft_doc::Unit::EAST_ASIAN.into_iter().find(|u| u.key() == unit).map(|u| tl!(u.label()).to_string())
            }
            "Raster Effects" => {
                let ppi = info["document"]["rasterEffectsPpi"].as_f64()?;
                Some(crate::i18n::fmt(tl!("{n} ppi"), &[("n", &ppi.to_string())]))
            }
            "Grid Size" | "Grid Colors" | "Simulate Colored Paper" | "Flattener Preset" => {
                transparency_value(&info["document"]["transparency"], label)
            }
            _ => None,
        },
        "brushes" => Some(brush_value(value).to_string()),
        "fonts" => font_use_value(entry("fontUses")?),
        "fontDetails" => font_detail_value(entry("fontDetails")?),
        "linkedImages" | "embeddedImages" => {
            let linked = id == "linkedImages";
            let image = info["imageObjects"].as_array()?.iter().filter(|o| o["linked"] == linked).nth(k).filter(|o| o["name"] == label)?;
            image_value(image)
        }
        "repeats" => repeat_value(entry("repeatObjects")?),
        "gradients" => gradient_value(entry("gradientObjects")?),
        _ => None,
    }
}

/// A transparency row of Document (`label`) in the interface language, from `document.info`'s
/// `document.transparency` (`t`): the grid's size, its colors' preset name (two custom colors stay
/// as `document.info` gives them), Simulate Colored Paper on or off, and a built-in flattener
/// preset's name (a saved preset's name stays as it is).
fn transparency_value(t: &Value, label: &str) -> Option<String> {
    let shown = match label {
        "Grid Size" => match t["gridSize"].as_str()? {
            "small" => tl!("Small"),
            "medium" => tl!("Medium"),
            "large" => tl!("Large"),
            _ => return None,
        },
        "Grid Colors" => match t["gridColorsName"].as_str()? {
            "Custom" => return None,
            name if vectorcraft_doc::setup::GRID_COLOR_PRESETS.iter().any(|(n, _)| *n == name) => tl!(name),
            _ => return None,
        },
        "Simulate Colored Paper" => {
            if t["simulatePaper"].as_bool()? {
                tl!("On")
            } else {
                tl!("Off")
            }
        }
        _ => match t["flattenerPreset"].as_str()? {
            "High Resolution" => tl!("High Resolution"),
            "Medium Resolution" => tl!("Medium Resolution"),
            "Low Resolution" => tl!("Low Resolution"),
            _ => return None,
        },
    };
    Some(shown.to_string())
}

/// A `fontUses` entry as Fonts shows it: the font's type (the format names TrueType and OpenType
/// stay as they are) and its number of uses in the language's plural forms.
fn font_use_value(e: &Value) -> Option<String> {
    let kind = match e["type"].as_str()? {
        "truetype" => "TrueType",
        "opentype" => "OpenType",
        "unknown" => tl!("unknown type"),
        _ => return None,
    };
    let uses = crate::i18n::tn(e["uses"].as_u64()?, "{n} use", "{n} uses");
    Some(crate::i18n::fmt(tl!("{type}; {uses}"), &[("type", kind), ("uses", &uses)]))
}

/// A `fontDetails` entry as Font Details shows it: the font's file (or that it is built in) and
/// what its license allows, the face shown in place of a style the family lacks, the face shown in
/// place of a font that is not installed, or that it is missing.
fn font_detail_value(e: &Value) -> Option<String> {
    let found = || {
        let embedding = match e["embedding"].as_str()? {
            "forbidden" => tl!("embedding not allowed"),
            "editing" => tl!("embedding for editing"),
            "previewPrint" => tl!("embedding for preview and print"),
            "allowed" => tl!("embedding allowed"),
            _ => return None,
        };
        let file = e["file"].as_str().unwrap_or(tl!("built in"));
        Some(crate::i18n::fmt(tl!("{file}; {embedding}"), &[("file", file), ("embedding", embedding)]))
    };
    let face = || Some((e["family"].as_str()?, e["style"].as_str()?));
    match e["match"].as_str()? {
        "exact" => found(),
        "style" => {
            let ((family, style), detail) = (face()?, found()?);
            Some(crate::i18n::fmt(
                tl!("substituted: shown in {family} {style} ({detail})"),
                &[("family", family), ("style", style), ("detail", &detail)],
            ))
        }
        "missingWithFallback" => {
            let (family, style) = face()?;
            Some(crate::i18n::fmt(tl!("missing: shown in {family} {style}"), &[("family", family), ("style", style)]))
        }
        "missing" => Some(tl!("missing").to_string()),
        _ => None,
    }
}

/// An `imageObjects` entry as Linked Images and Embedded Images show it: its format, its size in
/// pixels, and for a linked image its resolution and the file it shows, for an embedded one the
/// memory its pixels take. Format names and file names stay as they are.
fn image_value(o: &Value) -> Option<String> {
    use vectorcraft_engine::cmd::{byte_size, ppi_number};
    let format = match &o["format"] {
        Value::Null => tl!("Unknown format"),
        f => f.as_str()?,
    };
    let size = crate::i18n::fmt(
        tl!("{width} × {height} px"),
        &[("width", &o["width"].as_u64()?.to_string()), ("height", &o["height"].as_u64()?.to_string())],
    );
    let detail = if o["linked"] == true {
        match &o["effectivePpi"] {
            Value::Null => tl!("resolution unavailable").to_string(),
            ppi => {
                let (x, y) = (ppi_number(ppi.get(0)?.as_f64()?), ppi_number(ppi.get(1)?.as_f64()?));
                if x == y { crate::i18n::fmt(tl!("{n} ppi"), &[("n", &x)]) } else { crate::i18n::fmt(tl!("{x} × {y} ppi"), &[("x", &x), ("y", &y)]) }
            }
        }
    } else {
        match &o["memoryBytes"] {
            Value::Null => tl!("memory unavailable").to_string(),
            bytes => crate::i18n::fmt(tl!("estimated memory {memory}"), &[("memory", &byte_size(bytes.as_u64()?))]),
        }
    };
    let shown = crate::i18n::fmt(tl!("{format}; {size}; {detail}"), &[("format", format), ("size", &size), ("detail", &detail)]);
    if o["linked"] == true {
        Some(crate::i18n::fmt(tl!("{size} — {file}"), &[("size", &shown), ("file", o["file"].as_str()?)]))
    } else {
        Some(shown)
    }
}

/// A `repeatObjects` entry as Repeat Objects shows it: its kind and the number of instances it
/// draws, or for a grid its rows and columns, followed by the number it draws when that is fewer.
fn repeat_value(e: &Value) -> Option<String> {
    let kind = match e["type"].as_str()? {
        "radial" => tl!("Radial Repeat"),
        "grid" => tl!("Grid Repeat"),
        "mirror" => tl!("Mirror Repeat"),
        _ => return None,
    };
    let n = e["instances"].as_u64()?;
    let instances = crate::i18n::tn(n, "{n} instance", "{n} instances");
    if e["type"] != "grid" {
        return Some(crate::i18n::fmt(tl!("{kind}, {instances}"), &[("kind", kind), ("instances", &instances)]));
    }
    let (rows, cols) = (e["rows"].as_u64()?, e["cols"].as_u64()?);
    let (shown_rows, shown_cols) = (crate::i18n::tn(rows, "{n} row", "{n} rows"), crate::i18n::tn(cols, "{n} column", "{n} columns"));
    let grid = crate::i18n::fmt(tl!("{kind}, {rows} × {cols}"), &[("kind", kind), ("rows", &shown_rows), ("cols", &shown_cols)]);
    Some(if n < rows.saturating_mul(cols) {
        crate::i18n::fmt(tl!("{grid}, {instances}"), &[("grid", &grid), ("instances", &instances)])
    } else {
        grid
    })
}

/// A `gradientObjects` entry as Gradient Objects shows it: its gradients, a swatch's name as it
/// is and a gradient named by its kind in the interface language.
fn gradient_value(e: &Value) -> Option<String> {
    let (names, kinds) = (e["gradients"].as_array()?, e["kinds"].as_array()?);
    if names.len() != kinds.len() {
        return None;
    }
    let shown: Option<Vec<&str>> = names
        .iter()
        .zip(kinds)
        .map(|(name, kind)| match kind.as_str() {
            None => name.as_str(),
            Some("linear") => Some(tl!("Linear gradient")),
            Some("radial") => Some(tl!("Radial gradient")),
            Some("conic") => Some(tl!("Conic gradient")),
            Some("diamond") => Some(tl!("Diamond gradient")),
            Some("freeform") => Some(tl!("Freeform gradient")),
            Some(_) => name.as_str(),
        })
        .collect();
    Some(shown?.join(", "))
}

/// The rows of Document Info section `id` labelled by a name (an artboard, a style, a swatch, a
/// font, an image, a repeat object) rather than an interface label: every row of the lists, and in Document the
/// artboards, which follow the Artboards count. Names are shown as they are.
fn named_rows(id: &str, rows: &[Value]) -> std::ops::Range<usize> {
    match id {
        "objects" => 0..0,
        "document" => {
            let Some(at) = rows.iter().position(|r| r[0] == "Artboards") else { return 0..0 };
            let count = rows.get(at).and_then(|r| r[1].as_str()).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
            at + 1..(at + 1).saturating_add(count).min(rows.len())
        }
        _ => 0..rows.len(),
    }
}

/// `docInfo.save {path?, selectionOnly?, category?}`: write the text report (only `category` when given,
/// else every category) to `path`, else a picked file (the web downloads it) → `{path}`.
pub(crate) fn save_report(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let sel_only = p.get("selectionOnly").and_then(Value::as_bool).unwrap_or(false);
    let mut q = json!({ "format": "text", "selectionOnly": sel_only });
    if let Some(c) = p.get("category").and_then(Value::as_str).filter(|c| !c.is_empty()) {
        q["category"] = c.into();
    }
    let r = app.session.execute("document.info", &q).map_err(|e| e.to_string())?;
    let text = r["text"].as_str().unwrap_or_default();
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let stem = std::path::Path::new(&title).file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    crate::io::write_named(app, path, &format!("{stem} Info.txt"), text.as_bytes().to_vec(), |app, path| {
        app.status(format!("Saved {path}"));
        Ok(json!({ "path": path }))
    })
}

/// The row that shows category `id` (all of them when empty).
fn category_row(id: &str) -> CommandRef {
    CommandRef::with_params("ui.docInfo.category", json!({ "category": id }))
}

/// The row that saves the report of what the panel shows.
fn save_row(view: &DocInfoView) -> CommandRef {
    CommandRef::with_params("docInfo.save", json!({ "selectionOnly": view.selection_only, "category": view.category }))
}

/// The panel menu's rows: the scope, every category, and Save….
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = vec![PanelRow::Command(CommandRef::new("ui.docInfo.selectionOnly")), PanelRow::Separator, PanelRow::Command(category_row(""))];
    rows.extend(CATEGORIES.iter().map(|(id, _)| PanelRow::Command(category_row(id))));
    rows.push(PanelRow::Separator);
    rows.push(PanelRow::Command(save_row(&app.ui.doc_info)));
    rows
}

/// The titles, checks and enabled states of [`menu_rows`].
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let view = &app.ui.doc_info;
    states.set(CommandRef::new("ui.docInfo.selectionOnly"), CommandState::new().title(tl!("Selection Only")).checked(view.selection_only));
    states.set(category_row(""), CommandState::new().title(tl!("All Categories")).checked(view.category.is_empty()));
    for (id, title) in CATEGORIES {
        states.set(category_row(id), CommandState::new().title(tl!(title)).checked(view.category == id));
    }
    states.set(save_row(view), CommandState::new().title(tl!("Save…")).enabled(app.session.active().is_some()));
}

/// Runs the panel menu's own rows; `None` for an id that is not one of them. An unknown category
/// is refused with the ones that exist.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.docInfo.selectionOnly" => {
            app.ui.doc_info.selection_only = !app.ui.doc_info.selection_only;
            Some(Ok(Value::Null))
        }
        "ui.docInfo.category" => {
            let category = params.get("category").and_then(Value::as_str).unwrap_or_default();
            if !category.is_empty() && !CATEGORIES.iter().any(|(id, _)| *id == category) {
                let ids: Vec<&str> = CATEGORIES.iter().map(|(id, _)| *id).collect();
                return Some(Err(format!("unknown category `{category}`; one of {}", ids.join(", "))));
            }
            app.ui.doc_info.category = category.to_string();
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::LangExt;

    /// The panel's painted text with category `category` shown.
    fn painted(app: &mut VectorcraftApp, category: &str) -> String {
        run(app, "ui.docInfo.category", &json!({ "category": category })).unwrap().unwrap();
        crate::tests_labels::painted_text(app, show)
    }

    /// Artboard, font and other names in the rows are shown as they are, the labels in the UI
    /// language: an artboard named like a label ("Units") is still a name.
    #[test]
    fn names_in_rows_are_told_from_labels() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 0, "name": "Units"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        let info = app.session.execute("document.info", &json!({})).unwrap();
        let rows = |id: &str| info["sections"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()["rows"].as_array().unwrap().clone();
        let doc = rows("document");
        let named: Vec<&str> = named_rows("document", &doc).map(|k| doc[k][0].as_str().unwrap()).collect();
        assert_eq!(named, ["Units"], "{doc:?}");
        assert_eq!(doc.iter().filter(|r| r[0] == "Units").count(), 2, "the label and the artboard");
        assert_eq!(named_rows("objects", &rows("objects")), 0..0);
        let fonts = rows("fonts");
        assert!(!fonts.is_empty());
        assert_eq!(named_rows("fonts", &fonts), 0..fonts.len());
    }

    #[test]
    fn categories_filter_the_panel_and_save_writes_the_report() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        let text = painted(&mut app, "");
        assert!(text.contains("Color Mode") && text.contains("Artboard 1"), "{text}");
        let ids: Vec<String> = menu_rows(&app).iter().filter_map(|r| if let PanelRow::Command(c) = r { Some(c.id.clone()) } else { None }).collect();
        assert_eq!(ids.iter().filter(|i| *i == "ui.docInfo.category").count(), CATEGORIES.len() + 1, "all categories and one row per section");
        assert!(ids.contains(&"docInfo.save".to_string()), "{ids:?}");
        // Gradient Objects lists the objects painted with a gradient by name.
        let r = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        let ramp = json!({"kind": "radial", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        app.session.execute("paint.setFill", &json!({"ids": [r], "gradient": ramp})).unwrap();
        app.session.execute("layer.setProps", &json!({"id": r, "name": "Glow"})).unwrap();
        let gradients = painted(&mut app, "gradients");
        assert!(gradients.contains("Gradient Objects (1)") && gradients.contains("Glow") && gradients.contains("Radial gradient"), "{gradients}");
        // Spot Color Objects lists the objects painted with a spot color by name.
        let spot = json!({"model": "cmyk", "c": 0.0, "m": 0.8, "y": 0.6, "k": 0.0});
        app.session.execute("swatch.new", &json!({"name": "Signal Red", "color": spot, "spot": true})).unwrap();
        let badge = app.session.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.session.execute("paint.setFill", &json!({"ids": [badge], "swatch": "Signal Red"})).unwrap();
        app.session.execute("layer.setProps", &json!({"id": badge, "name": "Badge"})).unwrap();
        let spots = painted(&mut app, "spotColors");
        assert!(spots.contains("Spot Color Objects (1)") && spots.contains("Badge") && spots.contains("Signal Red"), "{spots}");
        let filtered = painted(&mut app, "fonts");
        assert!(filtered.contains("Source Sans 3 Regular") && !filtered.contains("Color Mode"), "{filtered}");
        // Save…: written through the host.
        let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().extend_from_slice(b);
            Ok(())
        }));
        let r = app.run("docInfo.save", json!({"path": "/tmp/info.txt"})).unwrap();
        assert_eq!(r["path"], "/tmp/info.txt");
        let report = String::from_utf8(written.borrow().clone()).unwrap();
        assert!(report.starts_with("Document Info: ") && report.contains("\nFONT DETAILS\n") && report.contains("\nREPEAT OBJECTS\n"), "{report}");
        assert!(report.contains("\nGRADIENT OBJECTS\nGlow: Radial gradient\n"), "{report}");
        assert!(report.contains("\nSPOT COLOR OBJECTS\nBadge: Signal Red\n"), "{report}");
    }

    /// The report the panel menu's Save… writes with category `category` shown ("" for All
    /// Categories) and Selection Only set to `sel_only`.
    fn saved_from_menu(app: &mut VectorcraftApp, category: &str, sel_only: bool) -> String {
        let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().extend_from_slice(b);
            Ok(())
        }));
        app.ui.doc_info.category = category.to_string();
        app.ui.doc_info.selection_only = sel_only;
        let save = menu_rows(app)
            .into_iter()
            .find_map(|r| match r {
                PanelRow::Command(c) if c.id == "docInfo.save" => Some(c),
                _ => None,
            })
            .expect("a Save… row");
        let mut params = save.params.clone();
        params["path"] = "/tmp/info.txt".into();
        app.run(&save.id, params).unwrap();
        let report = String::from_utf8(written.borrow().clone()).unwrap();
        assert!(report.starts_with("Document Info: "), "nothing saved: {report:?}");
        report
    }

    /// Save… writes the category the panel shows, with Selection Only applied; All Categories
    /// writes every category.
    #[test]
    fn save_writes_the_category_the_panel_shows() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let text = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
        let objects = saved_from_menu(&mut app, "objects", false);
        assert!(objects.contains("\nOBJECTS\n") && objects.contains("Paths: 1") && objects.contains("Text Objects: 1"), "{objects}");
        assert!(!objects.contains("\nDOCUMENT\n") && !objects.contains("\nFONTS\n"), "only Objects: {objects}");
        let fonts = saved_from_menu(&mut app, "fonts", false);
        assert!(fonts.contains("\nFONTS\nSource Sans 3 Regular: TrueType; 1 use\n"), "{fonts}");
        assert!(!fonts.contains("\nOBJECTS\n") && !fonts.contains("\nFONT DETAILS\n"), "only Fonts: {fonts}");
        let all = saved_from_menu(&mut app, "", false);
        assert!(
            all.contains("\nDOCUMENT\n") && all.contains("\nOBJECTS\n") && all.contains("\nFONTS\n") && all.contains("\nREPEAT OBJECTS\n"),
            "{all}"
        );
        app.session.execute("select.set", &json!({"ids": [text]})).unwrap();
        let selected = saved_from_menu(&mut app, "objects", true);
        assert!(selected.contains("(the selection only)") && selected.contains("Text Objects: 1"), "{selected}");
        assert!(!selected.contains("Paths: 1") && !selected.contains("\nFONTS\n"), "the selection's objects only: {selected}");
        // The command takes the category too; an unknown one is an error and writes nothing.
        let err = app.run("docInfo.save", json!({"path": "/tmp/info.txt", "category": "colors"})).unwrap_err();
        assert!(err.contains("category `colors`"), "{err}");
    }

    /// Section `id`'s rows as the panel shows them for `info`: each label and its shown value.
    fn shown_rows(info: &Value, id: &str) -> Vec<(String, String)> {
        let section = info["sections"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap_or_else(|| panic!("no {id}"));
        let rows = section["rows"].as_array().unwrap();
        let named = named_rows(id, rows);
        rows.iter()
            .enumerate()
            .map(|(k, r)| {
                let (label, value) = (r[0].as_str().unwrap(), r[1].as_str().unwrap());
                (label.to_string(), shown_value(info, id, k, (label, value), named.contains(&k)).unwrap_or_else(|| value.to_string()))
            })
            .collect()
    }

    /// The values Document Info makes (the units, the raster effects resolution, each font's file
    /// and state, each image's size, each Repeat object's kind and counts, and gradients named by
    /// their kind) show in the interface language, from the same `document.info` after a language
    /// change; font, face, file and swatch names stay as they are, also with Selection Only, and
    /// `document.info` and the saved report stay in English.
    #[test]
    fn values_follow_interface_language_and_keep_names() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 400, "units": "Millimeters"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Heavy", "font": "Source Sans 3", "style": "Black Wide"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 120, "text": "Gone", "font": "Qx Absent Family"})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 200, "width": 20, "height": 10})).unwrap()["id"].clone()
        };
        let ramp = |kind: &str| json!({"kind": kind, "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        app.session.execute("swatch.new", &json!({"name": "Linear gradient", "gradient": ramp("radial")})).unwrap();
        let glow = rect(&mut app, 0.0);
        app.session.execute("paint.setFill", &json!({"ids": [glow], "swatch": "Linear gradient"})).unwrap();
        app.session.execute("paint.setStroke", &json!({"ids": [glow], "gradient": ramp("radial")})).unwrap();
        app.session.execute("layer.setProps", &json!({"id": glow, "name": "Glow"})).unwrap();
        let star = rect(&mut app, 50.0);
        app.session.execute("object.repeat.radial", &json!({"ids": [star], "instances": 6})).unwrap();
        let tile = rect(&mut app, 100.0);
        app.session.execute("object.repeat.grid", &json!({"ids": [tile], "rows": 2, "cols": 1})).unwrap();
        let pic = rect(&mut app, 150.0);
        app.session.execute("select.set", &json!({"ids": [pic]})).unwrap();
        app.session.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        let info = app.session.execute("document.info", &json!({})).unwrap();
        let value = |rows: &[(String, String)], label: &str| {
            rows.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no {label} in {rows:?}")).1.clone()
        };
        let english = |id: &str| shown_rows(&info, id);
        assert_eq!((value(&english("document"), "Units"), value(&english("document"), "Raster Effects")), ("Millimeters".into(), "72 ppi".into()));
        assert_eq!(value(&english("fontDetails"), "Source Sans 3 Regular"), "built in; embedding allowed");
        assert_eq!(value(&english("gradients"), "Glow"), "Linear gradient, Radial gradient");
        let repeats = english("repeats");
        assert_eq!((repeats[0].1.as_str(), repeats[1].1.as_str()), ("Radial Repeat, 6 instances", "Grid Repeat, 2 rows × 1 column"));
        // Every value the panel builds reads as `document.info`'s English row.
        for s in info["sections"].as_array().unwrap() {
            let id = s["id"].as_str().unwrap();
            let rows: Vec<(String, String)> =
                s["rows"].as_array().unwrap().iter().map(|r| (r[0].as_str().unwrap().into(), r[1].as_str().unwrap().into())).collect();
            assert_eq!(shown_rows(&info, id), rows, "{id}");
        }
        let Some(de) = crate::i18n::Lang::from_code("de") else { panic!("no German catalog") };
        crate::i18n::set_current(de);
        let german = |id: &str| shown_rows(&info, id);
        let (document, fonts, images, gradients, repeats) =
            (german("document"), german("fontDetails"), german("embeddedImages"), german("gradients"), german("repeats"));
        app.session.execute("select.set", &json!({"ids": [glow]})).unwrap();
        let selected = shown_rows(&app.session.execute("document.info", &json!({"selectionOnly": true})).unwrap(), "gradients");
        let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().extend_from_slice(b);
            Ok(())
        }));
        app.run("docInfo.save", json!({"path": "/tmp/info.txt", "category": "gradients", "selectionOnly": true})).unwrap();
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let saved = String::from_utf8(written.borrow().clone()).unwrap();
        assert_eq!((value(&document, "Units"), value(&document, "Raster Effects")), ("Millimeter".into(), "72 ppi".into()));

        assert_eq!(value(&fonts, "Source Sans 3 Regular"), "integriert; Einbetten erlaubt");
        let wide = value(&fonts, "Source Sans 3 Black Wide");
        assert!(wide.starts_with("ersetzt: angezeigt in Source Sans 3 ") && wide.ends_with("Einbetten erlaubt)"), "{wide}");
        let gone = fonts.iter().find(|(l, _)| l.starts_with("Qx Absent Family")).unwrap_or_else(|| panic!("{fonts:?}")).1.clone();
        assert!(gone == "fehlt" || gone.starts_with("fehlt: angezeigt in "), "{gone}");
        assert_eq!(images.len(), 1);
        assert!(images[0].1.contains(" × ") && images[0].1.contains(" px; geschätzter Speicher ") && images[0].1.ends_with(" B"), "{images:?}");
        // A swatch named like a kind keeps its name; the gradient named by its kind is translated.
        assert_eq!(value(&gradients, "Glow"), "Linear gradient, Radialer Verlauf");
        assert_eq!(value(&selected, "Glow"), "Linear gradient, Radialer Verlauf");
        assert_eq!((repeats[0].1.as_str(), repeats[1].1.as_str()), ("Radiale Wiederholung, 6 Instanzen", "Rasterwiederholung, 2 Zeilen × 1 Spalte"));
        assert!(saved.contains("(the selection only)") && saved.contains("\nGRADIENT OBJECTS\nGlow: Linear gradient, Radial gradient\n"), "{saved}");
        let report = app.session.execute("document.info", &json!({"format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(report.contains("Units: Millimeters\n") && report.contains("Radial Repeat, 6 instances"), "{report}");
    }

    /// Linked Images and Embedded Images show each image's format, size, resolution or memory in
    /// the interface language, the format and file names as they are; values that do not fit an
    /// image entry are shown as `document.info` gives the row.
    #[test]
    fn image_details_show_in_the_interface_language() {
        let linked = |format: Value, ppi: Value| json!({"name": "a.png", "width": 3, "height": 2, "linked": true, "file": "/art/a.png", "format": format, "effectivePpi": ppi});
        let embedded =
            |memory: Value| json!({"name": "Image", "width": 3, "height": 2, "linked": false, "file": null, "format": "PNG", "memoryBytes": memory});
        let shown = || {
            [
                linked(json!("PNG"), json!([150.0, 150.0])),
                linked(json!(null), json!([144.0, 72.0])),
                linked(json!("TIFF"), json!(null)),
                embedded(json!(1536)),
                embedded(json!(null)),
            ]
            .map(|o| image_value(&o))
        };
        assert_eq!(
            shown(),
            [
                "PNG; 3 × 2 px; 150 ppi — /art/a.png",
                "Unknown format; 3 × 2 px; 144 × 72 ppi — /art/a.png",
                "TIFF; 3 × 2 px; resolution unavailable — /art/a.png",
                "PNG; 3 × 2 px; estimated memory 1.5 KiB",
                "PNG; 3 × 2 px; memory unavailable",
            ]
            .map(|v| Some(v.to_string()))
        );
        let Some(de) = crate::i18n::Lang::from_code("de") else { panic!("no German catalog") };
        crate::i18n::set_current(de);
        let german = shown();
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(
            german,
            [
                "PNG; 3 × 2 px; 150 ppi – /art/a.png",
                "Unbekanntes Format; 3 × 2 px; 144 × 72 ppi – /art/a.png",
                "TIFF; 3 × 2 px; Auflösung nicht verfügbar – /art/a.png",
                "PNG; 3 × 2 px; geschätzter Speicher 1.5 KiB",
                "PNG; 3 × 2 px; Speicher nicht verfügbar",
            ]
            .map(|v| Some(v.to_string()))
        );
        assert_eq!(image_value(&linked(json!("PNG"), json!([150.0]))), None);
        assert_eq!(image_value(&embedded(json!("big"))), None);
    }

    /// Document shows the transparency settings with their values in the interface language (two
    /// custom grid colors as they are), and while the Artboard tool is the current tool the
    /// artboards it has selected, following its selection without an edit to the document.
    #[test]
    fn document_shows_transparency_and_the_artboard_tools_selection() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 1, "name": "Back"})).unwrap();
        let document = |app: &mut VectorcraftApp| {
            let info = info(app).unwrap();
            shown_rows(&info, "document")
        };
        let value = |rows: &[(String, String)], label: &str| rows.iter().find(|(l, _)| l == label).map(|(_, v)| v.clone());
        let english = document(&mut app);
        assert_eq!(value(&english, "Grid Colors").as_deref(), Some("Light"));
        assert_eq!(value(&english, "X"), None, "no artboard rows with another tool: {english:?}");
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([1]));
        let rows = document(&mut app);
        let at = rows.iter().position(|(l, v)| l == "Artboard" && v == "Back").unwrap_or_else(|| panic!("{rows:?}"));
        assert_eq!(rows[at + 1..at + 5].iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["60 pt", "80 pt", "0 pt", "0 pt"]);
        // Selecting another artboard changes the rows, though the document did not change.
        app.session.set_tool_option("selected", &json!([0, 1]));
        let rows = document(&mut app);
        assert_eq!(rows.iter().filter(|(l, _)| l == "Artboard").count(), 2, "{rows:?}");
        assert!(rows.contains(&("X".into(), "150 pt".into())) && rows.contains(&("Y".into(), "40 pt".into())), "from artboard 1's corner: {rows:?}");
        app.session
            .execute(
                "document.setup",
                &json!({"gridSize": "large", "simulatePaper": true, "flattenerPreset": "Low Resolution", "gridColors": "Purple"}),
            )
            .unwrap();
        let Some(de) = crate::i18n::Lang::from_code("de") else { panic!("no German catalog") };
        crate::i18n::set_current(de);
        let german = document(&mut app);
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let shown = |label: &str| value(&german, label).unwrap_or_else(|| panic!("no {label} in {german:?}"));
        assert_eq!(
            [shown("Grid Size"), shown("Grid Colors"), shown("Simulate Colored Paper"), shown("Flattener Preset")],
            ["Groß", "Lila", "Ein", "Niedrige Auflösung"]
        );
        app.session.execute("document.setup", &json!({"gridColors": ["#ff0000", "#00ff00"]})).unwrap();
        crate::i18n::set_current(de);
        let custom = value(&document(&mut app), "Grid Colors");
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert_eq!(custom.as_deref(), Some("#ff0000, #00ff00"));
    }

    /// The by-value cache the panel kept before its result was shared, with the same key.
    type CopiedCache = Option<((u64, u64, bool, Value), Value)>;

    /// `document.info` read as the panel read it before its result was shared: the stored result
    /// copied out by value on every call, kept in `cache` so it does not touch the panel's.
    fn copied_info(app: &mut VectorcraftApp, cache: &mut CopiedCache) -> Option<Value> {
        let selection_only = app.ui.doc_info.selection_only;
        let artboard_tool = if app.session.tool_id() == "artboard" { app.session.tool_options_of("artboard") } else { Value::Null };
        let st = app.session.active()?;
        let key = (st.uid, st.revision, selection_only, artboard_tool);
        if let Some((cached, info)) = cache.as_ref()
            && *cached == key
        {
            return Some(info.clone());
        }
        let info = app.session.execute("document.info", &json!({ "selectionOnly": selection_only })).ok()?;
        *cache = Some((key, info.clone()));
        Some(info)
    }

    /// A frame that fetches nothing keeps the stored `document.info` result without a copy and
    /// shows the same rows; another document (at the same revision), an edit, Selection Only and
    /// the Artboard tool's options each store a new one, and a category does not. With no
    /// document nothing is fetched and the stored result stays as it was.
    #[test]
    fn frames_that_fetch_nothing_reuse_the_cached_rows() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        let mut old: CopiedCache = None;
        let mut frame = |app: &mut VectorcraftApp| {
            let text = crate::tests_labels::painted_text(app, show);
            let Some((key, stored)) = app.canvas.doc_info.clone() else { panic!("nothing stored") };
            // The rows are the ones the by-value read gives for the same state.
            let has_doc = app.session.active().is_some();
            assert_eq!(has_doc.then(|| (*stored).clone()), copied_info(app, &mut old), "the same rows as before");
            (key, stored, text)
        };
        let (_, first, text) = frame(&mut app);
        assert!(text.contains("Color Mode") && text.contains("Artboard 1"), "{text}");
        let (_, same, again) = frame(&mut app);
        assert!(Arc::ptr_eq(&first, &same), "a frame that fetches nothing stores no new result");
        assert!(Arc::ptr_eq(&info(&mut app).unwrap(), &same), "the stored result is returned without a copy");
        assert_eq!(text, again);
        // Another document at the same revision.
        let revision = |app: &VectorcraftApp| app.session.active().map(|st| st.revision);
        let at = revision(&app);
        let before = app.session.active().map(|st| st.uid);
        app.session.execute("document.activate", &json!({"index": 0})).unwrap();
        assert_eq!(revision(&app), at, "both documents are at the same revision");
        let (key, other, _) = frame(&mut app);
        assert!(!Arc::ptr_eq(&same, &other), "another document stores a new result");
        assert_ne!(Some(key.0), before);
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let (_, edited, _) = frame(&mut app);
        assert!(!Arc::ptr_eq(&other, &edited), "an edit stores a new result");
        assert!(shown_rows(&edited, "objects").contains(&("Paths".into(), "1".into())), "{:?}", shown_rows(&edited, "objects"));
        run(&mut app, "ui.docInfo.selectionOnly", &json!({})).unwrap().unwrap();
        let (_, selection, text) = frame(&mut app);
        assert!(!Arc::ptr_eq(&edited, &selection), "Selection Only stores a new result");
        assert!(text.contains("(selection)"), "{text}");
        run(&mut app, "ui.docInfo.selectionOnly", &json!({})).unwrap().unwrap();
        run(&mut app, "ui.docInfo.category", &json!({"category": "objects"})).unwrap().unwrap();
        let (_, all, _) = frame(&mut app);
        let (_, category, _) = frame(&mut app);
        assert!(Arc::ptr_eq(&all, &category), "a category filters the stored result");
        run(&mut app, "ui.docInfo.category", &json!({"category": ""})).unwrap().unwrap();
        app.session.execute("artboard.new", &json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
        let (_, two, _) = frame(&mut app);
        assert!(!Arc::ptr_eq(&category, &two), "a new artboard stores a new result");
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([0]));
        let (_, tool, _) = frame(&mut app);
        assert!(!Arc::ptr_eq(&two, &tool), "the Artboard tool stores a new result");
        let artboard_rows = |i: &Value| shown_rows(i, "document").iter().filter(|(l, _)| l == "Artboard").count();
        assert_eq!(artboard_rows(&tool), 1);
        app.session.set_tool_option("selected", &json!([0, 1]));
        let (_, picked, _) = frame(&mut app);
        assert!(!Arc::ptr_eq(&tool, &picked), "the Artboard tool's options store a new result");
        assert_eq!(artboard_rows(&picked), 2);
        let (_, kept, _) = frame(&mut app);
        assert!(Arc::ptr_eq(&picked, &kept));
        app.select_tool("selection");
        let (key, away, _) = frame(&mut app);
        assert!(!Arc::ptr_eq(&kept, &away), "another tool stores a new result");
        assert_eq!(key.3, Value::Null);
        // With no document the panel shows its empty state and keeps the stored result.
        app.session.execute("file.close", &json!({})).unwrap();
        app.session.execute("file.close", &json!({})).unwrap();
        let (_, closed, text) = frame(&mut app);
        assert!(Arc::ptr_eq(&away, &closed), "nothing is fetched or stored without a document");
        assert!(text.contains("No document"), "{text}");
    }

    /// Time a frame's read of Document Info's result for a document with 5,000 objects painted
    /// with a gradient (a row each): by the by-value read before the result was shared
    /// ([`copied_info`]) and by [`info`]. Per run each side reads 2,000 times; the two sides
    /// alternate, and the median time per read of 7 runs is printed, as this thread's CPU time
    /// where the system reports it (Linux), else wall-clock time.
    /// Run with `cargo test --release -p vectorcraft-ui-egui --lib doc_info::tests::cached_rows_read_timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual timing probe; performance depends on machine load"]
    fn cached_rows_read_timing() {
        const OBJECTS: usize = 5_000;
        const READS: u32 = 2_000;
        fn thread_cpu_ns() -> Option<u128> {
            std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
        }
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 2000, "height": 2000})).unwrap();
        let mut ids = vec![];
        for i in 0..OBJECTS {
            let r = json!({"x": i % 100 * 20, "y": i / 100 * 20, "width": 10, "height": 10});
            ids.push(app.session.execute("shape.rectangle", &r).unwrap()["id"].clone());
        }
        let ramp = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        app.session.execute("paint.setFill", &json!({"ids": ids, "gradient": ramp})).unwrap();
        let mut old: CopiedCache = None;
        let rows =
            info(&mut app).unwrap()["sections"].as_array().into_iter().flatten().map(|s| s["rows"].as_array().map_or(0, Vec::len)).sum::<usize>();
        assert_eq!(copied_info(&mut app, &mut old), info(&mut app).map(|i| (*i).clone()));
        let time = |app: &mut VectorcraftApp, f: &mut dyn FnMut(&mut VectorcraftApp) -> usize| {
            let (cpu, wall) = (thread_cpu_ns(), std::time::Instant::now());
            for _ in 0..READS {
                assert!(f(app) > 0);
            }
            let ns = thread_cpu_ns().zip(cpu).map_or_else(|| wall.elapsed().as_nanos(), |(b, a)| b - a);
            ns as f64 / f64::from(READS) / 1000.0
        };
        let sections = |i: &Value| i["sections"].as_array().map_or(0, Vec::len);
        let (mut copied, mut shared) = (vec![], vec![]);
        for _ in 0..7 {
            copied.push(time(&mut app, &mut |app| copied_info(app, &mut old).map_or(0, |i| sections(&i))));
            shared.push(time(&mut app, &mut |app| info(app).map_or(0, |i| sections(&i))));
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let clock = if thread_cpu_ns().is_some() { "thread CPU time" } else { "wall-clock time" };
        eprintln!("{OBJECTS} objects, {rows} rows; {clock} per read, median of 7 runs");
        eprintln!("copied ({READS} reads a run): {:>10.3} us  {copied:.3?}", median(&mut copied));
        eprintln!("shared ({READS} reads a run): {:>10.3} us  {shared:.3?}", median(&mut shared));
    }

    /// A Repeat object's counts take each language's plural forms, and a grid that draws fewer
    /// instances than its rows and columns give is followed by the number it draws.
    #[test]
    fn repeat_value_uses_locale_plural_forms_and_drawn_count() {
        let radial = |n: u64| json!({"name": "R", "type": "radial", "instances": n});
        let grid = json!({"name": "G", "type": "grid", "rows": 500, "cols": 500, "instances": 125_000});
        let in_lang = |code: &str, e: &Value| {
            let Some(lang) = crate::i18n::Lang::from_code(code) else { panic!("no {code} catalog") };
            crate::i18n::set_current(lang);
            let shown = repeat_value(e);
            crate::i18n::set_current(crate::i18n::Lang::EN);
            shown.unwrap_or_else(|| panic!("{e}"))
        };
        assert_eq!(in_lang("en", &radial(1)), "Radial Repeat, 1 instance");
        assert_eq!(in_lang("en", &grid), "Grid Repeat, 500 rows × 500 columns, 125000 instances");
        assert_eq!(
            [1, 3, 5, 21].map(|n| in_lang("ru", &radial(n))),
            [
                "Радиальный повтор, 1 экземпляр",
                "Радиальный повтор, 3 экземпляра",
                "Радиальный повтор, 5 экземпляров",
                "Радиальный повтор, 21 экземпляр"
            ]
        );
        assert_eq!(in_lang("ru", &grid), "Повтор сеткой, 500 строк × 500 столбцов, 125000 экземпляров");
        assert_eq!([1, 2].map(|n| in_lang("ja", &radial(n))), ["ラジアルリピート、インスタンス 1 個", "ラジアルリピート、インスタンス 2 個"]);
        // A full grid shows its rows and columns alone; an unknown kind is shown as `document.info` gives it.
        assert_eq!(in_lang("de", &json!({"type": "grid", "rows": 1, "cols": 3, "instances": 3})), "Rasterwiederholung, 1 Zeile × 3 Spalten");
        assert_eq!(repeat_value(&json!({"type": "spiral", "instances": 2})), None);
    }

    /// Objects rows the panel paints with Selection Only `on`, the Objects category shown.
    fn objects_panel(app: &mut VectorcraftApp, on: bool) -> String {
        app.ui.doc_info.selection_only = on;
        app.ui.doc_info.category = "objects".to_string();
        crate::tests_labels::painted_text(app, show)
    }

    /// With Selection Only off, Objects counts every object in the document.
    #[test]
    fn test_doc_info_counts_entire_document() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 80, "y": 10, "width": 30, "height": 20})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Hi"})).unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let text = objects_panel(&mut app, false);
        assert!(text.contains("Paths\n2\n") && text.contains("Text Objects\n1\n"), "{text}");
        assert!(!text.contains("(No artwork selected)"), "{text}");
    }

    /// Selection Only counts the selected object and what it holds, and an empty selection says so.
    #[test]
    fn test_doc_info_selection_only_toggles_and_filters_counts() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        let path = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap()["id"].clone();
        let text_id = app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Hi"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [text_id]})).unwrap();
        let text = objects_panel(&mut app, true);
        assert!(text.contains("Text Objects\n1\n") && !text.contains("Paths\n"), "the selected type only: {text}");
        app.session.execute("select.set", &json!({"ids": [path]})).unwrap();
        let path_only = objects_panel(&mut app, true);
        assert!(path_only.contains("Paths\n1\n") && !path_only.contains("Text Objects\n"), "the selected path only: {path_only}");
        app.session.execute("select.none", &json!({})).unwrap();
        let empty = objects_panel(&mut app, true);
        assert!(empty.contains("(No artwork selected)") && !empty.contains("Paths\n") && !empty.contains("Text Objects\n"), "{empty}");
    }

    /// With Selection Only on, an artboard the Artboard tool has selected still shows under
    /// Document when no artwork is selected, and the empty-state message speaks of artwork only.
    #[test]
    fn test_doc_info_selection_only_keeps_selected_artboard() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 150, "y": 40, "width": 60, "height": 80})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 1, "name": "Back"})).unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("artboard");
        app.session.set_tool_option("selected", &json!([1]));
        app.ui.doc_info.selection_only = true;
        app.ui.doc_info.category = "document".to_string();
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Back"), "the selected artboard: {text}");
        assert!(!text.contains("(No selection)"), "no generic empty state: {text}");
        assert!(text.contains("(No artwork selected)"), "{text}");
    }

    /// The panel menu's Selection Only item toggles `selection_only` and updates its checked state.
    #[test]
    fn test_doc_info_panel_menu_toggles_state() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let check = |app: &VectorcraftApp| {
            let mut states = CommandStates::default();
            menu_states(app, &mut states);
            states.get(&CommandRef::new("ui.docInfo.selectionOnly")).and_then(CommandState::is_checked)
        };
        assert_eq!(check(&app), Some(false));
        run(&mut app, "ui.docInfo.selectionOnly", &json!({})).unwrap().unwrap();
        assert_eq!(check(&app), Some(true));
        run(&mut app, "ui.docInfo.selectionOnly", &json!({})).unwrap().unwrap();
        assert_eq!(check(&app), Some(false));
    }
}
