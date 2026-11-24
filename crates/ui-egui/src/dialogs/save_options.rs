//! A save format's options before the file is written; on the web, where there is no save panel,
//! also the Save As dialog that names the file and picks the format (SVG and PDF then go on to
//! their own dialogs). The rows come from the format's options in the engine's format table, so a
//! format gains rows as its encoder gains options.
//!
//! Fields: `__pick` (file name and format editable), `path`, `format`, and one field per option
//! (agents set them with `ui.dialog.set`); the save command it finishes is the host's
//! `Export::Save { action }`.
//! Native and `.ai` files show their save options (VectorCraft Options after Save As): each
//! artboard to a separate file, with `all` (true: every artboard) or `range`, Include Linked Files,
//! Embed ICC Profiles, Create PDF-Compatible File and Use Compression. EPS shows Use Artboards
//! (`useArtboards`, off by default: the bounding box fits the visible art) with the same `all` or
//! `range` under it, then the options Export's EPS Options shows: Preview, Overprints, Flattener
//! preset and PostScript are menus that write the option's value (`previewFormat: "tiffColor"`,
//! `overprints: "preserve"`, `flattenerPreset: "medium"` or a saved preset's name, `level: 3`).
//! A native file's Version is a menu of the versions VectorCraft writes
//! (`version`, named by the releases that read each). A
//! `.ai` file's Version comes first: PDF-compatible, or legacy version 7 (`aiVersion: "7"`, a file
//! per artboard), which takes only Transparent Images (`transparentImages`), Hidden Objects
//! (`hiddenObjects`: a hidden layer, omitted or made visible), Text (`outlineText`: editable or
//! outlines) and, with several artboards, a picker of the artboards to save (thumbnails to check,
//! field `boards`, saved as `range`).
//!
//! A menu holds text, so the numeric options a menu sets (`version`, `level`, `artboard`, and a
//! legacy file's `outlineText`) are held as the text of their value and read back by [`typed`].

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format, FormatOption, SAVE_FORMATS, SaveMode, TEMPLATE_FORMATS};
use vectorcraft_eps::{Level, Overprint, Preview};

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, Show as _, ShowCollection as _, TooltipSpec};
use craft::ui::list::{ThumbAspect, ToggleState};
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::tile_grid::{Tile, TileGrid, TilePartEvent};

use super::export_host::{self, After, Export, Open};
use super::{export_for_screens, form};
use crate::VectorcraftApp;

/// Dialog kind.
pub const KIND: &str = "saveOptions";

/// The window title: the web's Save As names the mode, a legacy `.ai` file and every other format
/// name themselves.
fn heading(pick: bool, action: &str, legacy: bool, format: &str) -> String {
    if pick {
        return match SaveMode::of(action) {
            Some(SaveMode::Copy) => tl!("Save a Copy"),
            Some(SaveMode::Template) => tl!("Save as Template"),
            _ => tl!("Save As"),
        }
        .into();
    }
    if legacy {
        return tl!("Legacy .ai Options").into();
    }
    crate::i18n::fmt(tl!("{format} Options"), &[("format", fileio::format(format).map_or("", |f| f.label))])
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

/// The checkbox that `f`'s All or Range shows under: Save each artboard to a separate file, or
/// for EPS Use Artboards (off: the visible art's bounds).
fn board_switch(f: &Format) -> Option<&'static str> {
    if separates(f) {
        Some("separateArtboards")
    } else if f.id == "eps" {
        Some("useArtboards")
    } else {
        None
    }
}

/// The options the dialog shows: arrays and objects are for agents, as is a native file's
/// `precision`, and a page range replaces the single artboard choice; none for a format with its
/// own dialog. A `.ait` template has no Version: Save as Template writes it PDF-compatible only.
fn shown(f: &Format) -> impl Iterator<Item = &'static FormatOption> {
    let range = f.options.iter().any(|o| o.name == "range");
    let switched = board_switch(f).is_some();
    let options = if own_dialog(f) { &[][..] } else { f.options };
    options.iter().filter(move |o| {
        !(matches!(o.ty, "array" | "object")
            || o.name == "precision"
            || (range && o.name == "artboard")
            || (switched && o.name == "range")
            || (matches!(f.id, "ai" | "ait") && o.name == "useArtboards")
            // EPS Save writes all the artwork: selected-only output belongs to Export.
            || (f.id == "eps" && o.name == "selectedOnly")
            || (f.id == "ait" && o.name == "aiVersion"))
    })
}

/// An option's label: its written label, or for an option without one, its name split into words.
fn label(o: &FormatOption) -> String {
    written_label(o).unwrap_or_else(|| form::humanize(o.name))
}

/// The written label of option `o`, translated.
fn written_label(o: &FormatOption) -> Option<String> {
    Some(match o.name {
        "separateArtboards" => tl!("Save each artboard to a separate file").into(),
        "useArtboards" => tl!("Use Artboards").into(),
        "includeLinked" | "includeLinkedFiles" => tl!("Include Linked Files").into(),
        "embedProfiles" => tl!("Embed ICC Profiles").into(),
        "pdfCompatible" => tl!("Create PDF-Compatible File").into(),
        "compress" => tl!("Use Compression").into(),
        "preview" => tl!("Embed Preview").into(),
        "editingData" => tl!("Include Editing Data for Other Apps").into(),
        "version" | "aiVersion" => tl!("Version:").into(),
        "transparentImages" => tl!("Transparent Images:").into(),
        "hiddenObjects" => tl!("Hidden Objects:").into(),
        "outlineText" => tl!("Text:").into(),
        "thumbnails" => tl!("Include Document Thumbnails").into(),
        "previewFormat" => tl!("Preview:").into(),
        "transparentPreview" => tl!("Transparent").into(),
        "overprints" => tl!("Overprints:").into(),
        "flattenerPreset" => tl!("Flattener preset:").into(),
        "embedFonts" => tl!("Embed Fonts (for other applications)").into(),
        "cmykPostScript" => tl!("Include CMYK PostScript in RGB Files").into(),
        "compatibleGradients" => tl!("Compatible Gradient and Gradient Mesh Printing").into(),
        "level" => tl!("PostScript:").into(),
        _ => return None,
    })
}

/// The values of an option that takes one of a fixed set, each with the name its menu shows. The
/// first `builtins` names are VectorCraft's own (translated where they are shown); the others are
/// names the user gave (saved flattener presets), shown as they are.
struct Choices {
    values: Vec<Value>,
    names: Vec<String>,
    builtins: usize,
}

/// The choices of `f`'s option `o`, when it takes one of a fixed set of values: EPS's preview
/// format, overprints, PostScript level and flattener preset (the built-in presets by id, then the
/// saved ones by name).
fn choices(app: &VectorcraftApp, f: &Format, o: &FormatOption) -> Option<Choices> {
    if f.id != "eps" {
        return None;
    }
    let (pairs, builtins): (Vec<(Value, String)>, Option<usize>) = match o.name {
        "previewFormat" => (Preview::ALL.iter().map(|p| (json!(p.id()), p.label().to_string())).collect(), None),
        "overprints" => (Overprint::ALL.iter().map(|p| (json!(p.id()), p.label().to_string())).collect(), None),
        // The level is a number in the save options.
        "level" => (Level::ALL.iter().filter_map(|l| Some((json!(l.id().parse::<u64>().ok()?), l.label().to_string()))).collect(), None),
        "flattenerPreset" => {
            let id = |name: &str| FlattenOptions::PRESETS.into_iter().find(|id| FlattenOptions::preset_label(id) == Some(name));
            let pairs =
                app.session.flattener_presets().iter().map(|p| (id(&p.name).map_or_else(|| json!(p.name), |id| json!(id)), p.name.clone())).collect();
            (pairs, Some(FlattenOptions::builtin_presets().len()))
        }
        _ => return None,
    };
    let builtins = builtins.unwrap_or(pairs.len());
    let (values, names): (Vec<Value>, Vec<String>) = pairs.into_iter().unzip();
    Some(Choices { values, names, builtins })
}

/// What the dialog's rows need of the app, taken once when it opens: a hosted dialog's build
/// closure runs every pass and must not run commands.
#[derive(Default)]
struct Menus {
    /// The formats the web's Save As lists, in menu order; empty when the format is fixed.
    formats: Vec<&'static str>,
    /// Each listed format's option values, as the menu rows hold them ([`for_rows`]).
    values: BTreeMap<String, Map<String, Value>>,
    /// The menus of the EPS options, by option name.
    choices: BTreeMap<&'static str, Choices>,
    /// The artboards' names.
    names: Vec<String>,
    /// The artboards' thumbnails (empty unless a legacy `.ai` save can pick artboards).
    thumbs: Vec<Option<egui::TextureHandle>>,
    /// The document, for the file each checked artboard is saved to; it cannot go stale while the
    /// modal dialog is open.
    doc: Option<Arc<Document>>,
}

/// The rows of artboard tiles the legacy picker shows before it scrolls.
const BOARD_ROWS: u16 = 3;

/// The numeric options a menu sets are held as the text of their value.
const NUMBER_MENUS: [&str; 3] = ["version", "level", "artboard"];

/// Is a `.ai` file in `format` with Version `version` to be saved in the legacy version 7?
fn is_legacy(format: &str, version: Option<&str>) -> bool {
    format == "ai" && version.is_some_and(|v| v.trim() == "7")
}

/// `values` as the rows hold them: menus of numbers keep the number's text, and the All choice
/// under the artboard switch is `all` (no range: every artboard).
fn for_rows(f: &Format, mut values: Map<String, Value>) -> Map<String, Value> {
    for name in NUMBER_MENUS {
        if let Some(Value::Number(n)) = values.get(name) {
            let id = n.to_string();
            values.insert(name.into(), json!(id));
        }
    }
    if is_legacy(f.id, values.get("aiVersion").and_then(Value::as_str)) {
        let outlines = values.get("outlineText").and_then(Value::as_bool).unwrap_or(false);
        values.insert("outlineText".into(), json!(outlines.to_string()));
    }
    if let Some(switch) = board_switch(f) {
        // The check box holds on or off: an option left unset (EPS's Use Artboards) shows off.
        if values.get(switch).is_none_or(Value::is_null) {
            values.insert(switch.into(), json!(false));
        }
        let all = values.get("range").and_then(Value::as_str).is_none_or(|r| r.trim().is_empty());
        values.insert("all".into(), json!(all));
    }
    values
}

/// A fixed menu of `(id, label)` entries.
fn menu(entries: &[(&str, &'static str)]) -> ParamKind {
    ParamKind::choice(entries.iter().map(|(id, name)| (id.to_string(), Label::from(*name))).collect(), None)
}

/// The rows of `f`'s dialog, in the order the options are listed: a `.ai` file's Version first,
/// EPS's PostScript level last; the web's file name and format first of all.
fn params(f: &Format, menus: &Arc<Menus>, legacy: bool) -> Vec<Param> {
    let mut rows = vec![];
    if !menus.formats.is_empty() {
        rows.push(Param::new("path", tl!("File Name:"), ParamKind::text("")));
        let formats = menus.formats.iter().filter_map(|id| fileio::format(id)).map(|g| (g.id.to_string(), Label::from(g.label))).collect();
        rows.push(Param::new("format", tl!("Format:"), ParamKind::choice(formats, None)));
    }
    let mut options: Vec<&FormatOption> = shown(f).filter(|o| fileio::file_reads(f, legacy, o.name)).collect();
    options.sort_by_key(|o| (o.name != "aiVersion", f.id == "eps" && o.name == "level"));
    for o in options {
        rows.extend(option_rows(f, o, menus, legacy));
    }
    if legacy && menus.names.len() > 1 {
        let menus = Arc::clone(menus);
        // PLAN: assumed Param::custom's draw closure answers the response of the whole picker.
        rows.push(Param::custom("boards", "", move |ui, fields, _| boards_picker(ui, fields, &menus)).full_width());
    }
    rows
}

/// The row or rows of option `o` of format `f`: a menu for a fixed set, a check box for a switch
/// (with All or Range under an artboard switch), else a field.
fn option_rows(f: &Format, o: &FormatOption, menus: &Arc<Menus>, legacy: bool) -> Vec<Param> {
    let label = label(o);
    let row = |kind| Param::new(o.name, label.clone(), kind).tooltip(o.description);
    let fixed = |entries: &[(&str, &'static str)]| vec![row(menu(entries))];
    match o.name {
        "aiVersion" => return fixed(&[("pdf", tl!("PDF-compatible")), ("7", tl!("Legacy (version 7)"))]),
        "transparentImages" => return fixed(&[("flatten", tl!("Flatten")), ("omit", tl!("Omit"))]),
        "hiddenObjects" => return fixed(&[("layer", tl!("Hidden Layer")), ("omit", tl!("Omit")), ("visible", tl!("Make Visible"))]),
        "outlineText" if legacy => return fixed(&[("false", tl!("Editable")), ("true", tl!("Outlines"))]),
        "version" if o.ty == "integer" => {
            let versions = NATIVE_VERSIONS
                .iter()
                .map(|(n, release)| {
                    let name = crate::i18n::fmt(tl!("VectorCraft {release} and later (version {n})"), &[("release", release), ("n", &n.to_string())]);
                    (n.to_string(), Label::from(name))
                })
                .collect();
            return vec![row(ParamKind::choice(versions, None))];
        }
        _ => {}
    }
    if let Some(c) = menus.choices.get(o.name).filter(|_| f.id == "eps") {
        let entries = c.values.iter().zip(&c.names).enumerate().map(|(i, (value, name))| {
            let id = value.as_str().map_or_else(|| value.to_string(), str::to_string);
            (id, Label::from(if i < c.builtins { crate::i18n::t(name).to_string() } else { name.clone() }))
        });
        return vec![row(ParamKind::choice(entries.collect(), None))];
    }
    // Only the colour preview can keep transparency.
    if f.id == "eps" && o.name == "transparentPreview" {
        let kind = ParamKind::radio(vec![("true".into(), tl!("Transparent").into()), ("false".into(), tl!("Opaque").into())], None);
        return vec![Param::new(o.name, "", kind).enabled_when("previewFormat", "tiffColor")];
    }
    // Type is written as glyph outlines, which need no fonts: the row is on, and never enabled.
    if f.id == "eps" && o.name == "embedFonts" {
        let reason = TooltipSpec::new(o.description).disabled_text(tl!("Type is written as glyph outlines, which need no fonts."));
        return vec![Param::new(o.name, label.clone(), ParamKind::bool(true)).enabled_when_in(o.name, Vec::<bool>::new()).tooltip(reason)];
    }
    match o.ty {
        "boolean" => {
            let check = Param::new(o.name, label.trim_end_matches(':').to_string(), ParamKind::bool(false)).tooltip(o.description);
            // Versions older than compression read only plain files.
            let check = if o.name == "compress" { check.enabled_when_in("version", compressed_versions()) } else { check };
            let mut rows = vec![check];
            if matches!(o.name, "separateArtboards" | "useArtboards") {
                let all = ParamKind::radio(vec![("true".into(), tl!("All").into()), ("false".into(), tl!("Range:").into())], None);
                rows.push(Param::new("all", "", all).enabled_when(o.name, true));
                rows.push(Param::new("range", "", ParamKind::text("")).hint("1-3, 5").enabled_when("all", "false"));
            }
            rows
        }
        "string" => vec![row(ParamKind::text(""))],
        _ if o.name == "artboard" && !menus.names.is_empty() => {
            let names = menus.names.iter().enumerate().map(|(i, name)| (i.to_string(), Label::from(name.clone()))).collect();
            vec![row(ParamKind::choice(names, None))]
        }
        "integer" => vec![row(ParamKind::int(Some(0), None, 0))],
        _ => vec![row(ParamKind::number(Some(0.0), None, 0.0, 0.01, 2))],
    }
}

/// The ids of the native versions that can be compressed.
fn compressed_versions() -> Vec<String> {
    NATIVE_VERSIONS.iter().filter(|(n, _)| *n >= vectorcraft_format::COMPRESSED_SINCE).map(|(n, _)| n.to_string()).collect()
}

/// The value of a two-way field of `fields` (a check box, or the All or Range radio's `true` and
/// `false`).
fn flag(fields: &DialogFields, key: &str) -> Option<bool> {
    fields.bool(key).or_else(|| fields.text(key).and_then(|t| t.parse().ok()))
}

/// Every artboard (the All choice under Save each artboard to a separate file or Use Artboards):
/// `all` when set, else no range given.
fn all_artboards(fields: &DialogFields) -> bool {
    flag(fields, "all").unwrap_or_else(|| fields.text("range").is_none_or(|r| r.trim().is_empty()))
}

/// Is a `.ai` file to be saved in the legacy version 7 (`aiVersion: "7"`)?
fn legacy_ai(fields: &DialogFields) -> bool {
    is_legacy(fields.text("format").unwrap_or_default(), fields.text("aiVersion"))
}

/// The checks of the legacy `.ai` picker as the dialog holds them (`boards`: one bool per artboard).
fn boards_in(fields: &DialogFields) -> Option<Vec<bool>> {
    match fields.get("boards")? {
        FieldValue::Json(Value::Array(a)) => Some(a.iter().map(|b| b.as_bool().unwrap_or(false)).collect()),
        _ => fields.list("boards").map(|l| l.iter().map(|b| matches!(b, FieldValue::Bool(true))).collect()),
    }
}

/// The artboards a legacy `.ai` save writes (field `boards`: one bool per artboard), as last
/// saved (`range`), else all of them.
fn picked_boards(fields: &DialogFields, n: usize) -> Vec<bool> {
    if let Some(mut b) = boards_in(fields) {
        b.resize(n, false);
        return b;
    }
    let range = fields.text("range").unwrap_or_default().to_string();
    match (ArtboardPick { range: Some(range.clone()), ..Default::default() }).resolve(n) {
        Ok(Some(chosen)) if !range.trim().is_empty() => (0..n).map(|i| chosen.contains(&i)).collect(),
        _ => vec![true; n],
    }
}

/// The file each checked artboard is saved to (`None` for the others): the file itself when one
/// is checked, else `{stem}-{artboard}.ai` beside it, as the save names them.
fn board_files(doc: Option<&Document>, path: &str, boards: &[bool]) -> Vec<Option<String>> {
    let checked: Vec<usize> = boards.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i).collect();
    let file = std::path::Path::new(path.trim());
    let name = file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut names = match (doc, checked.len()) {
        (_, 1) => vec![name],
        (Some(doc), _) => fileio::artboard_file_names(doc, &checked).into_iter().map(|b| format!("{stem}-{b}.ai")).collect(),
        (None, _) => vec![],
    }
    .into_iter();
    boards.iter().map(|on| if *on { names.next() } else { None }).collect()
}

/// Legacy `.ai`: the artboards to save, as thumbnails to check; each is saved as a file of its own.
fn boards_picker(ui: &mut egui::Ui, fields: &mut DialogFields, menus: &Menus) -> egui::Response {
    let n = menus.names.len();
    let mut boards = picked_boards(fields, n);
    let before = boards.clone();
    let files = board_files(menus.doc.as_deref(), fields.text("path").unwrap_or_default(), &boards);
    let response = ui
        .vertical(|ui| {
            ui.label(tl!("A legacy .ai file holds one artboard: each artboard you check is saved as a separate file."));
            ui.horizontal(|ui| {
                if Button::new(tl!("Select All")).show(ui).clicks > 0 {
                    boards.iter_mut().for_each(|b| *b = true);
                }
                if Button::new(tl!("Clear")).show(ui).clicks > 0 {
                    boards.iter_mut().for_each(|b| *b = false);
                }
                let checked = boards.iter().filter(|b| **b).count();
                let (count, total) = (checked.to_string(), n.to_string());
                ui.label(crate::i18n::fmt(tl!("{count} of {total} artboards"), &[("count", &count), ("total", &total)]));
            });
            if !boards.iter().any(|b| *b) {
                Notice::new(Tone::Warn, tl!("Check an artboard to save.")).show(ui);
            }
            // PLAN: assumed TileGrid::show_items(ui, &source, &mut |key, _| tile) as the framework's stories call it;
            // the checks arrive as `CollectionEvent::Item(key, TilePartEvent::Check { .. })` in `response.events`.
            let grid = TileGrid::new("Artboards").checks(true).max_rows(BOARD_ROWS).show_items(ui, &Boards(n), &mut |i, _| {
                let on = boards.get(*i).copied().unwrap_or(false);
                let mut tile = Tile::new(menus.names.get(*i).cloned().unwrap_or_default()).check(if on { ToggleState::On } else { ToggleState::Off });
                if let Some(file) = files.get(*i).cloned().flatten() {
                    tile = tile.detail(file);
                }
                match menus.thumbs.get(*i).cloned().flatten() {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        // The thumbnail's pixel size is at most a few hundred: it fits `u16`.
                        let aspect = ThumbAspect::new(size.x.max(1.0) as u16, size.y.max(1.0) as u16);
                        tile.thumb(aspect, move |cx, rect| {
                            cx.picture(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), 0.0);
                        })
                    }
                    None => tile,
                }
            });
            for event in &grid.response.events {
                if let CollectionEvent::Item(i, TilePartEvent::Check { .. }) = event
                    && let Some(b) = boards.get_mut(*i)
                {
                    *b ^= true;
                }
            }
        })
        .response;
    if boards != before || boards_in(fields).is_none() {
        fields.set("boards", FieldValue::Json(json!(boards)));
    }
    response
}

/// The artboards of the picker, keyed by position.
struct Boards(usize);

// PLAN: assumed DisplayOrder and ItemSource as the tile grid's stories implement them (craft_behavior).
impl craft::behavior::DisplayOrder<usize> for Boards {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl craft::behavior::ItemSource<usize> for Boards {
    fn size_hint(&self, _: usize) -> craft::behavior::SizeHint {
        craft::behavior::SizeHint::Aspect(1.0)
    }
}

/// The native versions VectorCraft writes, newest first, each with the first release that reads
/// it (the format version each release tag writes: 2 in 0.1, 3 from 0.2, 4 from 0.8).
const NATIVE_VERSIONS: [(u32, &str); 4] = [(4, "0.8"), (3, "0.2"), (2, "0.1"), (1, "0.1")];

/// The native version the dialog will write (the current one unless chosen).
fn native_version_of(fields: &DialogFields) -> u32 {
    let chosen = fields.int("version").or_else(|| fields.text("version").and_then(|t| t.trim().parse().ok()));
    chosen.and_then(|v| u32::try_from(v).ok()).unwrap_or(vectorcraft_format::VERSION)
}

/// Is a native file to be saved in a version that can't be compressed?
fn uncompressed(fields: &DialogFields) -> bool {
    native_version_of(fields) < vectorcraft_format::COMPRESSED_SINCE
}

/// Web Save As: when the format picked differs from the one the rows show, the old format's
/// options go, the new format's values (as last saved) come, and the file name takes the new
/// extension. A legacy file's Text menu and a native version's text follow the rows.
fn sync(fields: &mut DialogFields, menus: &Menus) {
    let shown = fields.text("__shownFormat").map(str::to_string);
    let picked = fields.text("format").map(str::to_string);
    if let (Some(shown), Some(picked)) = (shown, picked)
        && shown != picked
        && let (Some(old), Some(new), Some(values)) = (fileio::format(&shown), fileio::format(&picked), menus.values.get(&picked))
    {
        for o in old.options {
            fields.remove(o.name);
        }
        let legacy = is_legacy(new.id, values.get("aiVersion").and_then(Value::as_str));
        let fresh = form::fields_of(&params(new, &Arc::new(Menus::default()), legacy), values);
        for (key, value) in fresh.iter() {
            fields.set(key, value.clone());
        }
        let path = with_extension(fields.text("path").unwrap_or_default(), new);
        fields.set("path", path);
        fields.set("__shownFormat", new.id);
    }
    // The Text menu of a legacy file holds `"true"` or `"false"`; the check box of any other, a bool.
    let outlines = flag(fields, "outlineText");
    if legacy_ai(fields) {
        fields.set("outlineText", outlines.unwrap_or(false).to_string());
    } else if fields.text("outlineText").is_some() {
        fields.set("outlineText", outlines.unwrap_or(false));
    }
}

/// `path` ending in `f`'s extension (kept when it already names `f`).
fn with_extension(path: &str, f: &Format) -> String {
    if fileio::format_for_name(path).is_some_and(|g| g.id == f.id) {
        return path.to_string();
    }
    std::path::Path::new(path).with_extension(f.extensions[0]).to_string_lossy().to_string()
}

/// `value` of option `o` as the save reads it: the menus of numbers and of a legacy file's Text
/// hold text.
fn typed(o: &FormatOption, value: Value) -> Value {
    let Some(text) = value.as_str() else { return value };
    if o.name == "level" || o.ty == "integer" {
        return text.trim().parse::<u64>().map_or(value, |n| json!(n));
    }
    if o.ty == "boolean" {
        return text.trim().parse::<bool>().map_or(value, |b| json!(b));
    }
    value
}

/// Opens the save options dialog of format `f` before writing `path`: `action` is the save command
/// it finishes, and `pick` (the web's Save As) also names the file and chooses the format.
pub(crate) fn open(app: &mut VectorcraftApp, action: &str, f: &'static Format, path: &str, pick: bool) -> Value {
    let Some(ctx) = app.ui_ctx.clone() else { return json!({ "pending": Value::Null }) };
    let formats: Vec<&'static str> = match (pick, SaveMode::of(action)) {
        (false, _) => vec![],
        (true, Some(SaveMode::Template)) => TEMPLATE_FORMATS.to_vec(),
        (true, _) => SAVE_FORMATS.to_vec(),
    };
    let listed: Vec<&'static Format> = std::iter::once(f).chain(formats.iter().filter_map(|id| fileio::format(id))).collect();
    let mut menus = Menus { formats, ..Menus::default() };
    for g in &listed {
        if !menus.values.contains_key(g.id) {
            let values = for_rows(g, option_values(app, g));
            menus.values.insert(g.id.to_string(), values);
        }
        for o in g.options {
            if let Some(c) = choices(app, g, o) {
                menus.choices.insert(o.name, c);
            }
        }
    }
    if let Some(st) = app.session.active() {
        menus.names = st.doc.artboards.iter().map(|a| a.name.clone()).collect();
        menus.doc = Some(st.doc.clone());
    }
    if menus.names.len() > 1 && listed.iter().any(|g| g.id == "ai") {
        menus.thumbs = (0..menus.names.len()).map(|i| export_for_screens::artboard_thumb(app, &ctx, i)).collect();
    }
    let mut values = menus.values.get(f.id).cloned().unwrap_or_default();
    values.insert("__pick".into(), json!(pick));
    values.insert("__shownFormat".into(), json!(f.id));
    values.insert("path".into(), json!(path));
    values.insert("format".into(), json!(f.id));
    let menus = Arc::new(menus);
    let legacy = is_legacy(f.id, values.get("aiVersion").and_then(Value::as_str));
    let mut fields = form::fields_of(&params(f, &menus, legacy), &values);
    if menus.names.len() > 1 && legacy {
        let boards = picked_boards(&fields, menus.names.len());
        fields.set("boards", FieldValue::Json(json!(boards)));
    }
    let (fallback, action_name) = (f, action.to_string());
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        sync(fields, &menus);
        let legacy = legacy_ai(fields);
        let f = fields.text("format").and_then(fileio::format).unwrap_or(fallback);
        let title = heading(fields.bool("__pick").unwrap_or(false), &action_name, legacy, f.id);
        let ok = !legacy || picked_boards(fields, menus.names.len()).iter().any(|b| *b);
        ParamDialog::new(KIND, title, params(f, &menus, legacy)).ok_enabled(ok).values(Bind::bound(fields))
    });
    match export_host::open(app, Export::Save { action: action.into() }, false, dialog) {
        Some(_) => json!({ "pending": KIND }),
        None => json!({ "pending": Value::Null }),
    }
}

/// Checks the chosen options and, when they hold, answers the step that writes the file once the
/// dialog has closed (blank options fall back to the format's defaults).
pub(crate) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    let Export::Save { action } = &open.kind else { return Err("not a save dialog".into()) };
    let f = fileio::format(fields.text("format").unwrap_or_default()).ok_or("unknown format")?;
    let path = fields.text("path").unwrap_or_default().to_string();
    if path.trim().is_empty() {
        return Err("name the file".into());
    }
    let pick = fields.bool("__pick").unwrap_or(false);
    let path = if pick { with_extension(path.trim(), f) } else { path };
    let legacy = legacy_ai(fields);
    let d = form::json_of(&params(f, &Arc::new(Menus::default()), legacy), fields);
    // Only the options the file reads: a `.ai` file's version takes its own (when the document's
    // own file is a `.ai`, the save remembers the others as they were).
    let mut options: Map<String, Value> = f
        .options
        .iter()
        .filter(|o| fileio::file_reads(f, legacy, o.name))
        .filter_map(|o| d.get(o.name).filter(|v| !v.is_null() && v.as_str() != Some("")).map(|v| (o.name.to_string(), typed(o, v.clone()))))
        .collect();
    if uncompressed(fields) {
        // Use Compression shows off there, whatever it held before.
        options.remove("compress");
    }
    let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
    if legacy {
        // The artboards checked in the picker (all of them: no range).
        options.remove("range");
        let boards = picked_boards(fields, n);
        if !boards.iter().any(|b| *b) {
            return Err(tl!("Check an artboard to save.").into());
        }
        if !boards.iter().all(|b| *b) {
            options.insert("range".into(), json!(export_for_screens::range_text(&boards)));
        }
    } else if let Some(switch) = board_switch(f) {
        let on = flag(fields, switch).unwrap_or(false);
        if switch == "useArtboards" {
            // All or Range alone names the artboards: an artboard remembered from an earlier
            // save would otherwise turn Use Artboards on, or narrow All to itself.
            options.remove("artboard");
            options.remove("artboards");
            options.insert(switch.into(), json!(on));
        }
        if !on || all_artboards(fields) {
            options.remove("range");
        } else {
            // A bad range keeps the dialog open.
            ArtboardPick { range: Some(fields.text("range").unwrap_or_default().to_string()), ..Default::default() }.resolve(n)?;
        }
    }
    let action = action.clone();
    Ok(Some(Box::new(move |app: &mut VectorcraftApp| match SaveMode::of(&action) {
        Some(mode) if own_dialog(f) => crate::io::ask_format_options(app, mode, f, &path),
        Some(mode) => crate::io::save(app, mode, &json!({ "path": path, "format": f.id, "options": options }), false),
        None => Err(format!("`{action}` is no save command")),
    })))
}

#[cfg(test)]
mod tests;
