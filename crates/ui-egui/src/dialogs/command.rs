//! The generic parameter dialog (`ui.paramDialog`): edits a command's parameters and runs it on OK, with the
//! fixed ones (`__params`, not shown) added. One row per parameter the command takes, its kind from the value the
//! dialog opens with; the tables below say which are distances, choices and in what order.

use std::sync::Arc;

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use serde_json::{Map, Value};

use super::form;
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;

/// The open parameter dialog and what it keeps between frames.
#[derive(Default)]
pub(crate) struct CommandDialog {
    open: Option<Open>,
}

impl CommandDialog {
    /// The open dialog's heading, in English, as the menu item gave it.
    #[cfg(test)]
    pub(crate) fn label(&self) -> Option<&str> {
        self.open.as_ref().map(|o| o.label.as_str())
    }
}

/// One open parameter dialog.
struct Open {
    id: DialogId,
    command: String,
    /// The heading, in English, as the menu item gave it.
    label: String,
    /// The values the dialog opened with: they give each row its kind and its first value.
    opening: Map<String, Value>,
    /// The fixed parameters (`__params`) sent with the dialog's own.
    fixed: Map<String, Value>,
    /// Area Type Options: the Row Gutter as it opened on rows without a gutter of their own, while it is still that.
    linked: Option<f64>,
    previews: bool,
    preview: LivePreview,
    /// Set Role Swatch: the role the Swatch row was last filled for.
    filled_for: Option<String>,
    /// The document whose objects the dialog edits, when its caller supplies `__doc`.
    doc: Option<u64>,
    /// Names this dialog's own open among reopenings of the same command, for its OK and Cancel.
    token: Arc<()>,
}

/// What the rows are built against: the document's colour model and its length units.
#[derive(Clone, Copy)]
struct Setting {
    cmyk: bool,
    units: LengthUnits,
}

impl Setting {
    fn of(app: &VectorcraftApp) -> Self {
        Self {
            cmyk: app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk),
            // PLAN: assumed `canvas::rulers::general_lengths(&VectorcraftApp) -> LengthUnits` (cv.B5)
            units: crate::canvas::rulers::general_lengths(app),
        }
    }
}

/// Opens the parameter dialog of `command`, headed `label`, on `params` (a menu item's own values, `__params` for the
/// fixed ones, `__rowGutterLinked` for Area Type Options' linked Row Gutter and `__doc` for the uid of the document
/// whose objects the dialog edits).
pub(crate) fn open(app: &mut VectorcraftApp, command: &str, label: &str, mut params: Map<String, Value>) -> Result<Value, String> {
    let doc = match params.remove("__doc") {
        None => None,
        Some(value) => Some(value.as_u64().ok_or("`__doc` must be a document id")?),
    };
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    if let Some(old) = app.command_dialog.open.take()
        && on_document(app, &old)
    {
        old.preview.cancel(app);
    }
    let fixed = match params.remove("__params") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let linked = params.remove("__rowGutterLinked").and_then(|v| v.as_f64());
    params.retain(|k, _| !k.starts_with("__"));
    let previews = previews(command, &params);
    let setting = Setting::of(app);
    let fields = form::fields_of(&command_params(command, &params, setting), &params);
    let token = Arc::new(());
    let dialog = {
        let (key, heading, opening) = (command.to_string(), heading_label(label), params.clone());
        HostedParamDialog::command(&ctx, command, fields, move |fields| {
            ParamDialog::command(key.clone(), heading.clone(), command_params(&key, &opening, setting))
                .previews(previews)
                .values(craft::ui::contract::Bind::bound(fields))
        })
        // The Preview box starts off, as production's dialogs did (#5360).
        .preview_off()
    };
    let ok_token = token.clone();
    let cancel_token = token.clone();
    let options = HeldOptions { cancel: Some(Box::new(move |app: &mut VectorcraftApp| cancel(app, &cancel_token))), ..Default::default() };
    let id = open_held_with(app, dialog, move |app, answer| confirm(app, &ok_token, &answer.fields).map(|_| ()), options).ok_or("no window")?;
    app.command_dialog.open = Some(Open {
        id,
        command: command.to_string(),
        label: label.to_string(),
        opening: params,
        fixed,
        linked,
        previews,
        preview: LivePreview::default(),
        filled_for: None,
        doc,
        token,
    });
    Ok(Value::Null)
}

/// One pass of the open dialog, after the shell's: the rules that tie rows together, then the live preview. Never
/// runs a command inside the dialog host's lock.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.command_dialog.open.take() else { return };
    step(app, &mut open);
    // Nothing the step runs opens a dialog; a holder set meanwhile is the newer one.
    if app.command_dialog.open.is_none() {
        app.command_dialog.open = Some(open);
    }
}

fn step(app: &mut VectorcraftApp, open: &mut Open) {
    if !on_document(app, open) {
        return;
    }
    let Some(dialogs) = app.dialogs() else { return };
    let id = open.id;
    let seen = dialogs.with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| (d.fields().clone(), d.is_previewing())));
    let Some((fields, previewing)) = seen else { return };
    let params = command_params(&open.command, &open.opening, Setting::of(app));
    let mut json = form::json_of(&params, &fields);
    let mut sets: Vec<(&str, FieldValue)> = Vec::new();
    // Area Type Options: a Row Gutter still as it opened follows the Gutter as it is changed.
    if let Some(linked) = open.linked
        && json.get("rowGutter").and_then(Value::as_f64) == Some(linked)
        && let Some(gutter) = json.get("gutter").and_then(Value::as_f64)
        && gutter != linked
    {
        json.insert("rowGutter".into(), Value::from(gutter));
        sets.push(("rowGutter", FieldValue::Number(gutter)));
        open.linked = Some(gutter);
    }
    if open.command == crate::addons::packaging::SET_ROLE_SWATCH
        && let Some(role) = json.get("role").and_then(Value::as_str).map(str::to_string)
        && let Some(swatch) = crate::addons::packaging::follow_role(app, &role, &mut open.filled_for)
    {
        json.insert("swatch".into(), Value::String(swatch.clone()));
        sets.push(("swatch", FieldValue::Text(swatch)));
    }
    if !sets.is_empty() {
        dialogs.with(|h| {
            if let Some(d) = h.get_mut_by_id::<HostedParamDialog>(id) {
                for (key, value) in sets {
                    d.set_field(key, value);
                }
            }
        });
    }
    if open.previews {
        let value = call(&open.command, &open.fixed, open.linked, json);
        // The preview reports a refusal in the status bar itself.
        let _ = open.preview.run(app, previewing, &open.label, &open.command, &value);
    }
}

/// OK: runs the command on the dialog's values (keeping a preview as the one undo step) and closes the dialog; an
/// error keeps it open with the reason under its rows. A dialog opened on one document's objects (`__doc`, its uid)
/// closes as Cancel does and runs nothing once another document is the active one.
fn confirm(app: &mut VectorcraftApp, token: &Arc<()>, fields: &DialogFields) -> Result<Value, String> {
    let open = app.command_dialog.open.take_if(|o| Arc::ptr_eq(&o.token, token)).ok_or("the dialog is gone")?;
    if !on_document(app, &open) {
        return Ok(Value::Null);
    }
    let params = command_params(&open.command, &open.opening, Setting::of(app));
    let value = call(&open.command, &open.fixed, open.linked, form::json_of(&params, fields));
    let result = if open.previews {
        open.preview.clone().commit(app, &open.command, value)
    } else {
        // A tool's click-to-size shape (Flare) goes on the active perspective plane while the grid shows.
        let at = |x: &str, y: &str| Some(vectorcraft_geom::Point::new(value.get(x)?.as_f64()?, value.get(y)?.as_f64()?));
        match at("cx", "cy").or_else(|| at("x", "y")).and_then(|pt| vectorcraft_engine::perspective_click(&app.session, &open.command, &value, pt)) {
            Some((c, p)) => app.run(&c, p),
            None => app.run(&open.command, value),
        }
    };
    if result.is_err() {
        app.command_dialog.open = Some(open);
    }
    result
}

/// Cancel: rolls back the preview of the dialog `token` names.
fn cancel(app: &mut VectorcraftApp, token: &Arc<()>) {
    if let Some(open) = app.command_dialog.open.take_if(|o| Arc::ptr_eq(&o.token, token))
        && on_document(app, &open)
    {
        open.preview.cancel(app);
    }
}

fn on_document(app: &VectorcraftApp, open: &Open) -> bool {
    open.doc.is_none_or(|uid| app.session.active().is_some_and(|st| st.uid == uid))
}

/// The rows of `command`'s dialog: one per value it opens with that the command takes, in `order(command)` first and
/// the rest by name, Area Type Options' Text Flow last.
fn command_params(command: &str, opening: &Map<String, Value>, setting: Setting) -> Vec<Param> {
    let order = order(command);
    let rank = |k: &str| order.iter().position(|o| *o == k).unwrap_or(order.len());
    let flow = |k: &str| command == "text.areaOptions" && k == "flow";
    let mut keys: Vec<&String> = opening.keys().filter(|k| !k.starts_with("__") && takes(command, k)).collect();
    keys.sort_by_key(|k| (flow(k), rank(k)));
    keys.into_iter().filter_map(|key| row(command, key, opening.get(key)?, setting)).collect()
}

/// The row of parameter `key` that opens with `value`; none for a value no row edits.
fn row(command: &str, key: &str, value: &Value, setting: Setting) -> Option<Param> {
    let label = if command == "text.areaOptions" && key == "flow" {
        tl!("Text Flow").to_string()
    } else {
        let humanized = form::humanize(key);
        tl!(humanized.trim_end_matches(':')).to_string()
    };
    if let Some(options) = rasterize_color_models(command, key, setting.cmyk).or_else(|| choices(command, key)) {
        let options = options.iter().map(|(l, v)| ((*v).to_string(), Label::from(form::choice_label(l)))).collect();
        return Some(Param::new(key, label, ParamKind::choice(options, value.as_str().map(str::to_string))));
    }
    if command == "text.areaOptions" && key == "flow" {
        let options =
            vec![("rows".to_string(), Label::from(tl!("By Rows").to_string())), ("columns".to_string(), Label::from(tl!("By Columns").to_string()))];
        return Some(Param::new(key, label, ParamKind::radio(options, value.as_str().map(str::to_string))));
    }
    Some(match value {
        Value::Bool(on) => Param::new(key, label, ParamKind::bool(*on)),
        Value::Number(n) => {
            let whole = n.is_i64() || n.is_u64();
            let range = number_range(command, key);
            // A ranged number (a fold angle) is shown to every place it keeps.
            let decimals = match range {
                Some(_) => u32::try_from(crate::addons::packaging::angle_places(n.as_f64())).unwrap_or(u32::MAX),
                None if whole => 0,
                None => 3,
            };
            let mut param = Param::number(key, label, n.as_f64().unwrap_or(0.0), 1.0, decimals);
            if let Some(range) = range {
                param = param.bounds(Some(*range.start()), Some(*range.end()));
            }
            if lengths(command).contains(&key) {
                param = param.length(setting.units);
                if let Some(range) = length_range(command, key) {
                    param = param.bounds(Some(*range.start()), Some(*range.end()));
                }
            }
            if key == "ppi" {
                param = param.unit(UnitKind::PixelsPerInch);
            }
            param
        }
        Value::String(s) => match hex_rgb(s) {
            Some(rgb) => Param::new(key, label, ParamKind::color(Some(rgb))),
            None if s.contains('\n') => Param::new(key, label, ParamKind::text(s.clone())).multiline(4),
            None => Param::new(key, label, ParamKind::text(s.clone())),
        },
        _ => return None,
    })
}

/// `#rrggbb` as colour components.
fn hex_rgb(s: &str) -> Option<[f32; 3]> {
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok().map(|v| f32::from(v) / 255.0);
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The dialog's heading in the UI language.
fn heading_label(label: &str) -> String {
    heading_in(crate::i18n::current(), label)
}

/// `label` as a heading in `lang`. A menu item opens the dialog with its label less the "…", so
/// the menu item's row (the label with "…") is looked up first, without its "…", and then the
/// plain row, as the effect dialog's heading is (`effect.rs`, `heading_label`).
fn heading_in(lang: crate::i18n::Lang, label: &str) -> String {
    use crate::i18n::{has, tr};
    let dotted = format!("{label}…");
    if has(lang, &dotted) {
        return tr(lang, &dotted).trim_end_matches('…').trim_end().to_string();
    }
    tr(lang, label).to_string()
}

/// Does this dialog show a Preview checkbox? Grid Repeat Options does: the public page for the
/// grid options lists one, and the radial and mirror pages do not. Type on a Path Options does
/// too (#5360), as public tutorials of the dialog show, and Type Tools › Release Type on a Path
/// works the same way (#5532), and so do Image Tools › Remove Background and Color Mask and
/// Cutting Tools › Weed Box and Lines, Banner Hems and Grommets and Nest Shapes, and Path
/// Precision Tools › Common Tangents and the bounding shapes (Add Bounding Rectangle… and Add
/// Bounding Circle…). (Cancel rolls a preview back whichever dialog started it.)
fn previews(command: &str, opening: &Map<String, Value>) -> bool {
    match command {
        "object.repeat.options" => opening.contains_key("gridType"),
        "type.pathOptions"
        | crate::addons::typetools::RELEASE
        | crate::addons::image::REMOVE_BACKGROUND
        | crate::addons::image::COLOR_MASK
        | crate::addons::cutting::WEED
        | crate::addons::cutting::BANNER
        | crate::addons::cutting::NEST
        | crate::addons::pathtools::COMMON_TANGENTS
        | crate::addons::pathtools::BOUNDING_RECTANGLE
        | crate::addons::pathtools::BOUNDING_CIRCLE => true,
        _ => false,
    }
}

/// The parameters the command runs with: the dialog's values and the fixed ones.
fn call(command: &str, fixed: &Map<String, Value>, linked: Option<f64>, mut params: Map<String, Value>) -> Value {
    params.extend(fixed.clone());
    // Area Type Options: a Row Gutter left as it opened, on rows without a gutter of their own,
    // isn't sent, so the rows take the Gutter.
    if let Some(linked) = linked
        && params.get("rowGutter").and_then(Value::as_f64) == Some(linked)
    {
        params.remove("rowGutter");
    }
    // Weed Box and Lines: a Spacing of 0, or none, leaves the Count in use; any other Spacing is
    // sent instead of the Count, so the command refuses one below 0.
    if command == crate::addons::cutting::WEED {
        let zero = |s: &Value| match s {
            Value::Null => true,
            Value::String(t) => vectorcraft_doc::Unit::Points.parse(t).or_else(|| t.trim().parse().ok()) == Some(0.0),
            v => v.as_f64() == Some(0.0),
        };
        let by_count = params.get("spacing").is_none_or(zero);
        params.remove(if by_count { "spacing" } else { "count" });
    }
    Value::Object(params)
}

/// Does `command` take parameter `key` (its params description names it)?
fn takes(command: &str, key: &str) -> bool {
    // An app command (Rotate View) is checked against its own params doc.
    let params = vectorcraft_engine::find_command(command)
        .map(|c| c.params)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == command).map(|c| c.3));
    params.is_some_and(|params| vectorcraft_engine::check_keys(command, params, &serde_json::json!({ key: null })).is_ok())
}

/// The parameters of the commands this dialog edits that are distances.
fn lengths(command: &str) -> &'static [&'static str] {
    match command {
        "graph.create" => &["width", "height"],
        "shape.flare" => &["diameter", "pathLength"],
        "perspective.grid.set" => &["cell", "distance"],
        crate::addons::cutting::WEED => &["margin", "spacing", "gap", "weight"],
        crate::addons::cutting::BANNER => &["hem", "spacing", "grommetInset", "diameter", "weight"],
        crate::addons::cutting::NEST => &["width", "height", "margin", "spacing"],
        "object.repeat.options" => &["radius", "hSpacing", "vSpacing"],
        "text.areaOptions" => &["width", "height", "gutter", "rowGutter", "inset", "firstBaselineMin", "lineGap"],
        "type.pathOptions" => &["spacing"],
        "object.pathfinder.trap" => &["thickness"],
        crate::addons::cutting::BRIDGES => &["width"],
        "object.textWrap.options" => &["offset"],
        crate::addons::pathtools::BOUNDING_RECTANGLE | crate::addons::pathtools::BOUNDING_CIRCLE => &["offset"],
        _ => &[],
    }
}

/// The range (points) of a distance parameter of the commands this dialog edits, where the
/// command has one: the field refuses a distance typed outside it.
pub(crate) fn length_range(command: &str, key: &str) -> Option<std::ops::RangeInclusive<f64>> {
    match (command, key) {
        ("object.textWrap.options", "offset") => Some(vectorcraft_engine::TEXT_WRAP_OFFSET_RANGE),
        // The bounding shapes take an offset of 0 or more, and the command refuses one that takes
        // the shape past the canvas edge.
        (crate::addons::pathtools::BOUNDING_RECTANGLE | crate::addons::pathtools::BOUNDING_CIRCLE, "offset") => {
            Some(0.0..=vectorcraft_engine::MAX_COORD)
        }
        _ => None,
    }
}

/// The range of a number parameter (not a distance) of the commands this dialog edits, where the
/// command has one: the field keeps what is typed within it.
fn number_range(command: &str, key: &str) -> Option<std::ops::RangeInclusive<f64>> {
    match (command, key) {
        (crate::addons::packaging::SET_FOLD_ANGLE, "angle") => Some(crate::addons::packaging::FOLD_ANGLES),
        _ => None,
    }
}

/// The order the fields of the commands this dialog edits come in, where the order of their
/// names isn't it (the rest follow, by name).
fn order(command: &str) -> &'static [&'static str] {
    match command {
        crate::addons::cutting::WEED => &["margin", "box", "direction", "count", "spacing", "gap", "weight", "swatch"],
        crate::addons::cutting::BANNER => {
            &["hem", "top", "right", "bottom", "left", "grommets", "spacing", "grommetInset", "diameter", "weight", "swatch"]
        }
        crate::addons::cutting::NEST => &["roll", "width", "height", "margin", "spacing", "rotate", "usePreviewBounds"],
        // The public Trap options' order.
        "object.pathfinder.trap" => &["thickness", "heightWidth", "tintReduction", "trapsWithProcessColor", "reverseTraps"],
        "addon.color.printFixes" => &["inkLimit", "excludePureBlack", "removeWhiteOverprint", "overprintBlack", "textOnly", "fill", "stroke"],
        crate::addons::typetools::RELEASE => &["mode", "keepOriginal"],
        crate::addons::image::REMOVE_BACKGROUND | crate::addons::image::COLOR_MASK => {
            &["background", "colors", "contiguous", "match", "tolerance", "softness"]
        }
        crate::addons::cutting::BRIDGES => &["width", "count", "direction", "keepOriginal"],
        crate::addons::packaging::SET_ROLE_SWATCH => &["role", "swatch"],
        crate::addons::pathtools::COMMON_TANGENTS => &["tangents", "centerline"],
        crate::addons::pathtools::BOUNDING_RECTANGLE | crate::addons::pathtools::BOUNDING_CIRCLE => &["offset", "each"],
        _ => &[],
    }
}

/// The parameters of the commands this dialog edits that pick one of some values.
fn choices(command: &str, key: &str) -> Option<form::Choices> {
    match (command, key) {
        ("type.pathOptions", "effect") => Some(crate::menus::PATH_EFFECTS),
        ("type.pathOptions", "alignToPath") => Some(PATH_ALIGN),
        (crate::addons::packaging::SET_ROLE_SWATCH, "role") => Some(crate::addons::packaging::ROLES),
        (crate::addons::typetools::RELEASE, "mode") => Some(crate::addons::typetools::RELEASE_MODES),
        (crate::addons::cutting::BRIDGES, "direction") => Some(crate::addons::cutting::BRIDGE_DIRECTIONS),
        ("text.areaOptions", "fit") => Some(AREA_FIT),
        ("text.areaOptions", "firstBaseline") => Some(FIRST_BASELINE),
        ("text.areaOptions", "verticalAlign") => Some(VERTICAL_ALIGN),
        (crate::addons::cutting::WEED, "direction") => Some(crate::addons::cutting::WEED_DIRECTIONS),
        (crate::addons::pathtools::COMMON_TANGENTS, "tangents") => Some(crate::addons::pathtools::TANGENT_KINDS),
        (crate::addons::image::REMOVE_BACKGROUND | crate::addons::image::COLOR_MASK, "background") => Some(crate::addons::image::BACKGROUNDS),
        (crate::addons::image::REMOVE_BACKGROUND | crate::addons::image::COLOR_MASK, "match") => Some(crate::addons::image::MATCHES),
        ("object.rasterize", "antiAlias") => Some(RASTERIZE_ANTI_ALIAS),
        ("object.rasterize", "background") => Some(RASTERIZE_BACKGROUNDS),
        ("object.mesh.create", "appearance") => Some(MESH_APPEARANCES),
        ("object.repeat.options", "gridType") => Some(REPEAT_GRID_TYPES),
        _ => None,
    }
}

/// Object › Rasterize › Anti-aliasing: the modes `vectorcraft_render::AntiAlias` names, with its
/// labels, as the Rasterize effect's dialog lists them (`effect.rs`, `own_choices`).
const RASTERIZE_ANTI_ALIAS: form::Choices = &[("None", "none"), ("Art Optimized", "art"), ("Type Optimized", "type")];

/// Object › Rasterize › Background, as Document Raster Effects Settings and the Rasterize effect
/// list it. A color the command was given another way (black, a hex color) shows as it is.
const RASTERIZE_BACKGROUNDS: form::Choices = &[("White", "white"), ("Transparent", "transparent")];

/// Object › Create Gradient Mesh › Appearance: the values `object.mesh.create` takes.
const MESH_APPEARANCES: form::Choices = &[("Flat", "flat"), ("To Center", "center"), ("To Edge", "edge")];

/// Repeat Options › Grid Type of a grid repeat: the tile types, labeled as the Pattern Options
/// panel labels them (`panels/pattern_options.rs`, `TILE_LABELS`).
const REPEAT_GRID_TYPES: form::Choices = &[
    ("Grid", "grid"),
    ("Brick by Row", "brickByRow"),
    ("Brick by Column", "brickByColumn"),
    ("Hex by Column", "hexByColumn"),
    ("Hex by Row", "hexByRow"),
];

/// Object › Rasterize › Color Model: the values `object.rasterize` takes, the document's own
/// colour model first (#5660).
fn rasterize_color_models(command: &str, key: &str, cmyk: bool) -> Option<form::Choices> {
    match (command, key) {
        ("object.rasterize", "colorModel") => Some(if cmyk { RASTERIZE_CMYK } else { RASTERIZE_RGB }),
        _ => None,
    }
}

/// Object › Rasterize › Color Model in an RGB and in a CMYK document.
const RASTERIZE_RGB: form::Choices = &[("RGB", "rgb"), ("Grayscale", "grayscale"), ("Bitmap", "bitmap")];
const RASTERIZE_CMYK: form::Choices = &[("CMYK", "cmyk"), ("Grayscale", "grayscale"), ("Bitmap", "bitmap")];

/// Type on a Path Options › Align to Path.
const PATH_ALIGN: form::Choices = &[("Ascender", "ascender"), ("Descender", "descender"), ("Center", "center"), ("Baseline", "baseline")];

/// Area Type Options › Fit (Block is Dynamic Text's, as `type.dynamicText` sets it).
const AREA_FIT: form::Choices = &[("None", "none"), ("Auto Size", "autoHeight"), ("Shrink Text to Fit", "shrinkText"), ("Block", "block")];

/// Area Type Options › First Baseline.
const FIRST_BASELINE: form::Choices = &[
    ("Ascent", "ascent"),
    ("Cap Height", "capHeight"),
    ("x Height", "xHeight"),
    ("Leading", "leading"),
    ("Em Box Height", "emBoxHeight"),
    ("Fixed", "fixed"),
];

/// Area Type Options › Align (vertical alignment of the lines in each row/column).
const VERTICAL_ALIGN: form::Choices = &[("Top", "top"), ("Center", "center"), ("Bottom", "bottom"), ("Justify", "justify")];

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::{NodeKind, TextFlow, VerticalAlign};
    use vectorcraft_engine::Session;

    use super::*;
    use crate::i18n::LangExt as _;

    /// An app with a window context, so a dialog can open.
    fn windowed() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    fn id_of(app: &VectorcraftApp) -> DialogId {
        app.command_dialog.open.as_ref().expect("a command dialog is open").id
    }

    /// One frame's pump, twice: the pump of this dialog and then the host's drain of its answers.
    fn pump(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        for _ in 0..2 {
            crate::dialogs::host::pump(app, &ctx);
        }
    }

    /// Sets `field` as automation does and runs a frame.
    fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), id_of(app));
        app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, field, value)).unwrap();
        pump(app);
    }

    fn answer_ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), id_of(app));
        app.dialogs().unwrap().with(|h| h.confirm_by_id(&ctx, id));
        pump(app);
    }

    fn answer_cancel(app: &mut VectorcraftApp) {
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), id_of(app));
        app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id));
        pump(app);
    }

    /// The text area of node `$id`.
    macro_rules! area {
        ($app:expr, $id:expr) => {
            match &$app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId($id)).unwrap().kind {
                NodeKind::Text(t) => &t.area,
                _ => panic!("text"),
            }
        };
    }

    /// A window app with the object `command` edits selected → the object's id.
    fn prepared(command: &str) -> (VectorcraftApp, u64) {
        let mut app = windowed();
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let rect = json!({"x": 10, "y": 10, "width": 20, "height": 20});
        let id = match command {
            "text.areaOptions" => app.run("text.create", json!({"x": 20, "y": 20, "text": "Hello", "area": {"width": 200, "height": 200}})),
            "object.rasterize" => {
                let id = app.run("shape.ellipse", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
                app.run("paint.setFill", json!({"color": "#ff0000", "ids": [id]})).unwrap();
                Ok(json!({ "id": id }))
            }
            "object.repeat.options" => {
                let id = app.run("shape.rectangle", rect).unwrap()["id"].as_u64().unwrap();
                app.run("object.repeat.grid", json!({"ids": [id], "rows": 2, "cols": 2})).unwrap();
                Ok(json!({ "id": id }))
            }
            _ => app.run("shape.rectangle", rect),
        }
        .unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    /// The top-left pixel of the selected raster.
    fn corner(app: &VectorcraftApp) -> [u8; 4] {
        pixel(app, false)
    }

    /// The top-left or the middle pixel of the selected raster.
    fn pixel(app: &VectorcraftApp, middle: bool) -> [u8; 4] {
        let st = app.session.active().unwrap();
        let key = st.selection.objects.iter().find_map(|id| match st.doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Image(i)) => Some(i.key.clone()),
            _ => None,
        });
        let decoded = craft_image_codecs::decode(&st.doc.images[&key.expect("the object became an image")].bytes).unwrap();
        let img = image::RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8()).unwrap();
        let (x, y) = if middle { (img.width() / 2, img.height() / 2) } else { (0, 0) };
        img.get_pixel(x, y).0
    }

    fn cols(app: &VectorcraftApp) -> u64 {
        crate::panels::pattern_options::repeat_fields(app).unwrap()["cols"].as_u64().unwrap()
    }

    #[test]
    fn hosted_commands_keep_actual_keyboard_edits_until_ok() {
        use egui::accesskit::Role;
        use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        let type_and_accept = |app: &mut VectorcraftApp, command: &str, name: &str, role: Role, value: &str, preview: bool| {
            let ctx = app.ui_ctx.clone().unwrap();
            let time = std::cell::Cell::new(0.0);
            let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
                ctx.enable_accesskit();
                time.set(time.get() + 0.1);
                crate::testing::app_frame(app, &ctx, RawInput { events, time: Some(time.get()), screen_rect: Some(viewport), ..Default::default() })
            };
            let locate = |app: &mut VectorcraftApp, label: &str, role: Role| {
                frame(app, vec![]);
                let nodes = frame(app, vec![]).platform_output.accesskit_update.unwrap().nodes;
                let node = nodes
                    .iter()
                    .map(|(_, node)| node)
                    .find(|node| {
                        node.role() == role
                            && (node.label() == Some(label)
                                || node.labelled_by().iter().any(|id| nodes.iter().any(|(key, name)| key == id && name.label() == Some(label))))
                    })
                    .unwrap_or_else(|| panic!("{command}: missing {role:?} {label}"));
                let b = node.bounds().unwrap();
                let at = pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
                assert!(viewport.contains(at), "{command}: {label} is reachable");
                at
            };
            let click = |app: &mut VectorcraftApp, at: Pos2| {
                frame(app, vec![Event::PointerMoved(at)]);
                for pressed in [true, false] {
                    frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
                }
                frame(app, vec![]);
            };
            frame(app, vec![]);
            assert_eq!(ctx.content_rect(), viewport);
            crate::menus::invoke(app, command, json!({}));
            let id = id_of(app);
            if preview {
                let toggle = locate(app, "Preview", Role::CheckBox);
                click(app, toggle);
                assert!(app.session.in_interaction(), "{command}: pointer turns Preview on");
            }
            let at = locate(app, name, role);
            click(app, at);
            frame(app, vec![Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }]);
            frame(app, vec![Event::Text(value.to_owned())]);
            frame(app, vec![Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
            frame(app, vec![]);
            let live_columns = preview.then(|| cols(app));
            let ok = locate(app, "OK", Role::Button);
            click(app, ok);
            frame(app, vec![]);
            assert!(app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).is_none()), "{command}: OK closes the dialog");
            live_columns
        };

        let (mut repeat, _) = prepared("object.repeat.options");
        let preview = type_and_accept(&mut repeat, "object.repeat.options", "Columns", Role::SpinButton, "5", true);
        let (mut selection, _) = prepared("select.save");
        type_and_accept(&mut selection, "select.save", "Name", Role::TextInput, "Typed selection", false);
        let saved = &selection.session.active().unwrap().doc.saved_selections;
        assert_eq!((preview, cols(&repeat), saved.last().map(|s| s.name.as_str())), (Some(5), 5, Some("Typed selection")));
    }

    #[test]
    fn area_type_options_choices_match_the_params_doc() {
        let doc = vectorcraft_engine::find_command("text.areaOptions").unwrap().params;
        for key in ["firstBaseline", "verticalAlign"] {
            let values: Vec<&str> = choices("text.areaOptions", key).unwrap().iter().map(|(_, v)| *v).collect();
            assert!(doc.contains(&format!("{key}?: {}", values.join("|"))), "{key}: {values:?}");
        }
        assert!(choices("text.areaOptions", "firstBaselineMin").is_none(), "a distance, not a choice");
        assert!(choices("text.areaOptions", "inset").is_none());
    }

    /// Each row of a command's dialog, set as automation sets it and confirmed, reaches the engine under its
    /// own name.
    #[test]
    fn a_parameter_row_sets_its_commands_param() {
        struct Row {
            command: &'static str,
            field: &'static str,
            value: Value,
            effect: fn(&VectorcraftApp, u64) -> bool,
        }
        let rows = [
            Row {
                command: "text.areaOptions",
                field: "verticalAlign",
                value: json!("center"),
                effect: |app, id| area!(app, id).vertical_align == VerticalAlign::Center,
            },
            Row {
                command: "text.areaOptions",
                field: "firstBaseline",
                value: json!("emBoxHeight"),
                effect: |app, id| area!(app, id).first_baseline == vectorcraft_doc::FirstBaseline::EmBoxHeight,
            },
            Row { command: "text.areaOptions", field: "flow", value: json!("rows"), effect: |app, id| area!(app, id).flow == TextFlow::Rows },
            Row { command: "object.rasterize", field: "background", value: json!("white"), effect: |app, _| corner(app) == [255, 255, 255, 255] },
            Row {
                command: "object.mesh.create",
                field: "appearance",
                value: json!("center"),
                effect: |app, _| {
                    let st = app.session.active().unwrap();
                    st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Mesh(_))))
                },
            },
            Row {
                command: "object.repeat.options",
                field: "gridType",
                value: json!("brickByRow"),
                effect: |app, _| crate::panels::pattern_options::repeat_fields(app).unwrap()["gridType"] == json!("brickByRow"),
            },
        ];
        for row in rows {
            let (mut app, id) = prepared(row.command);
            // A click on the menu row: Create Gradient Mesh… carries the values its dialog opens on.
            let item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some(row.command)).expect("a menu row");
            let (cmd, p) = crate::menus::click_target(&item.label, row.command, &item.params);
            crate::menus::invoke(&mut app, &cmd, p);
            set(&mut app, row.field, row.value);
            answer_ok(&mut app);
            assert!((row.effect)(&app, id), "{} {}", row.command, row.field);
            assert!(app.command_dialog.open.is_none(), "{}: OK closed the dialog", row.command);
        }
    }

    /// Area Type Options: a Row Gutter of its own is sent as set, and one left as it opened follows the Gutter.
    #[test]
    fn a_row_gutter_left_as_opened_follows_the_gutter() {
        let (mut app, id) = prepared("text.areaOptions");
        let gutters = |app: &VectorcraftApp| (area!(app, id).gutter, area!(app, id).row_gutter);
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        set(&mut app, "rowGutter", json!(30.0));
        answer_ok(&mut app);
        assert_eq!(gutters(&app), (18.0, Some(30.0)));
        // Rows with a gutter of their own keep it when only the Gutter changes.
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        set(&mut app, "gutter", json!(12.0));
        answer_ok(&mut app);
        assert_eq!(gutters(&app), (12.0, Some(30.0)));
        // Without one, a changed Gutter is the rows' too.
        app.run("text.areaOptions", json!({"gutter": 18})).unwrap();
        crate::menus::invoke(&mut app, "text.areaOptions", json!({}));
        set(&mut app, "gutter", json!(10.0));
        answer_ok(&mut app);
        assert_eq!(gutters(&app), (10.0, None));
    }

    /// Object › Rasterize › Color Model lists the models the command takes, the document's own first (#5660): every
    /// value runs, and picking Grayscale rasterizes in grays.
    #[test]
    fn rasterize_color_model_is_a_dropdown_of_the_models_the_command_takes() {
        for (mode, own) in [("rgb", "RGB"), ("cmyk", "CMYK")] {
            let models = rasterize_color_models("object.rasterize", "colorModel", mode == "cmyk").unwrap();
            assert_eq!(models.first(), Some(&(own, mode)), "the document's own model first");
            for (_, value) in models {
                let mut s = Session::new();
                s.execute("file.new", &json!({"width": 100, "height": 100, "colorMode": mode})).unwrap();
                s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
                s.execute("object.rasterize", &json!({"ppi": 72, "colorModel": value})).unwrap_or_else(|e| panic!("{mode} {value}: {e}"));
            }
        }
        assert!(rasterize_color_models("object.rasterize", "background", false).is_none());
        assert!(rasterize_color_models("document.rasterEffectsSettings", "colorModel", false).is_none());
        let (mut app, _) = prepared("object.rasterize");
        crate::menus::invoke(&mut app, "object.rasterize", json!({}));
        set(&mut app, "colorModel", json!("grayscale"));
        answer_ok(&mut app);
        // The ellipse's middle: its corner is the transparent background.
        let p = pixel(&app, true);
        assert!(p[0] == p[1] && p[1] == p[2] && p[0] < 128, "red in grayscale: {p:?}");
    }

    /// #2651: with Preview on, the repeat on the canvas follows the fields while the dialog is open; Cancel puts it
    /// back without a step of history; OK keeps what was previewed as one step.
    #[test]
    fn grid_repeat_options_preview_on_the_canvas() {
        let (mut app, _) = prepared("object.repeat.options");
        let steps = app.session.doc().unwrap().history.undo.len();
        crate::menus::invoke(&mut app, "object.repeat.options", json!({}));
        set(&mut app, "preview", json!(true));
        set(&mut app, "cols", json!(3));
        assert_eq!(cols(&app), 3, "the canvas shows 3 columns while the dialog is open");
        assert!(app.session.in_interaction());
        set(&mut app, "cols", json!(4));
        assert_eq!(cols(&app), 4, "and follows the next edit");
        answer_cancel(&mut app);
        assert_eq!(cols(&app), 2, "Cancel restores the original repeat");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps, "and adds no undo step");
        assert!(!app.session.in_interaction());
        crate::menus::invoke(&mut app, "object.repeat.options", json!({}));
        set(&mut app, "preview", json!(true));
        set(&mut app, "cols", json!(5));
        answer_ok(&mut app);
        assert_eq!(cols(&app), 5);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(cols(&app), 2, "one Undo takes the whole change back");
    }

    /// Radial and Mirror Repeat Options have no Preview (their public pages list none): their fields apply on OK
    /// only.
    #[test]
    fn radial_repeat_options_have_no_preview() {
        let mut app = windowed();
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("object.repeat.radial", json!({"ids": [id], "instances": 4})).unwrap();
        crate::menus::invoke(&mut app, "object.repeat.options", json!({}));
        let (ctx, dialog) = (app.ui_ctx.clone().unwrap(), id_of(&app));
        let refused = app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, dialog, "preview", json!(true)));
        assert!(refused.is_err() || !app.command_dialog.open.as_ref().unwrap().previews);
        set(&mut app, "instances", json!(7));
        assert_eq!(crate::panels::pattern_options::repeat_fields(&app).unwrap()["instances"], 4);
        assert!(!app.session.in_interaction());
    }

    /// Text Wrap Options › Offset is a distance, and its range is the one `object.textWrap.options`
    /// accepts: the command takes the range's ends and refuses an Offset outside them.
    #[test]
    fn text_wrap_offset_is_a_distance_within_the_commands_range() {
        assert!(lengths("object.textWrap.options").contains(&"offset"));
        assert!(!lengths("object.textWrap.options").contains(&"invert"));
        let range = length_range("object.textWrap.options", "offset").expect("a range");
        assert_eq!(range, vectorcraft_engine::TEXT_WRAP_OFFSET_RANGE);
        assert_eq!(length_range("object.textWrap.options", "invert"), None);
        assert_eq!(length_range("text.areaOptions", "inset"), None);
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        for end in [*range.end(), *range.start()] {
            assert_eq!(s.execute("object.textWrap.options", &json!({ "offset": end })).unwrap()["offset"], json!(end));
        }
        for sent in [5000.0, -5000.0] {
            assert!(s.execute("object.textWrap.options", &json!({ "offset": sent })).is_err(), "{sent}");
        }
    }

    /// The choices this dialog lists for Rasterize, Create Gradient Mesh and Repeat Options are the
    /// values their commands take (#8020, #7991): each one runs, and every language with complete
    #[test]
    fn document_scoped_dialog_confirms_only_in_its_opening_document() {
        for (scoped, switch) in [(true, false), (true, true), (false, true)] {
            let mut app = windowed();
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            let mut params = json!({"x": 10, "y": 10, "width": 20, "height": 20}).as_object().unwrap().clone();
            if scoped {
                params.insert("__doc".into(), json!(app.session.active().unwrap().uid));
            }
            open(&mut app, "shape.rectangle", "Rectangle", params).unwrap();
            pump(&mut app);
            if switch {
                app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            }
            answer_ok(&mut app);
            assert!(app.command_dialog.open.is_none());
            assert_eq!(app.session.active().unwrap().selection.is_empty(), scoped && switch);
            if switch {
                assert!(app.session.set_active(0));
                assert!(app.session.active().unwrap().selection.is_empty());
            }
        }
    }

    #[test]
    fn document_scoped_dialog_rejects_an_invalid_document_id() {
        for doc in [json!(null), json!("1"), json!(-1), json!(1.5)] {
            let mut app = windowed();
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            let params = json!({"width": 20, "height": 20, "__doc": doc}).as_object().unwrap().clone();
            assert!(open(&mut app, "shape.rectangle", "Rectangle", params).is_err(), "{doc}");
            assert!(app.command_dialog.open.is_none());
        }
    }

    /// menus has a row for each label.
    #[test]
    fn choice_fields_list_the_values_their_commands_take() {
        use vectorcraft_doc::pattern::TileType;
        use vectorcraft_render::AntiAlias;
        let listed = |command: &str, key: &str| choices(command, key).unwrap_or_else(|| panic!("{command} {key}: a choice"));
        let aa: Vec<(&str, &str)> = AntiAlias::ALL.iter().map(|a| (a.label(), a.id())).collect();
        assert_eq!(listed("object.rasterize", "antiAlias"), aa.as_slice());
        for (_, value) in listed("object.rasterize", "background") {
            assert!(vectorcraft_doc::Background::parse(value).is_some(), "{value}");
        }
        for (label, value) in listed("object.repeat.options", "gridType") {
            let tile = TileType::parse(value, None).unwrap_or_else(|| panic!("{value}"));
            assert_eq!((tile.id(), tile.label()), (*value, *label));
        }
        let run = |command: &str, key: &str, value: &str| {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
            let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
            if command == "object.repeat.options" {
                s.execute("object.repeat.grid", &json!({"ids": [id], "rows": 2, "cols": 2})).unwrap();
            }
            s.execute(command, &json!({ key: value })).unwrap_or_else(|e| panic!("{command} {key} {value}: {e}"));
        };
        let fields = [
            ("object.rasterize", "antiAlias"),
            ("object.rasterize", "background"),
            ("object.mesh.create", "appearance"),
            ("object.repeat.options", "gridType"),
        ];
        for (command, key) in fields {
            for (label, value) in listed(command, key) {
                run(command, key, value);
                for lang in crate::i18n::Lang::all().filter(|l| l.complete_menus()) {
                    assert!(crate::i18n::has(lang, label), "{}: {label}", lang.code());
                }
            }
        }
    }

    use crate::addons::packaging::{SET_FOLD_ANGLE, SET_ROLE};

    /// A window app with two rectangles, `a` a crease and `b` a cut line, nothing selected → (app, a, b).
    fn fold_lines() -> (VectorcraftApp, u64, u64) {
        let mut app = windowed();
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
        };
        let (a, b) = (rect(&mut app, 20.0), rect(&mut app, 100.0));
        app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
        app.run(SET_ROLE, json!({"role": "cut", "ids": [b]})).unwrap();
        app.run("select.none", json!({})).unwrap();
        (app, a, b)
    }

    /// The angle the open Set Fold Angle… dialog shows.
    fn shown_angle(app: &VectorcraftApp) -> Option<f64> {
        let id = id_of(app);
        app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().number("angle")))
    }

    /// Set Fold Angle… from its menu item → the angle its dialog opens on, the dialog then canceled; `None` when no
    /// dialog opens.
    fn opened_angle(app: &mut VectorcraftApp) -> Option<f64> {
        crate::menus::invoke(app, SET_FOLD_ANGLE, json!({}));
        app.command_dialog.open.as_ref()?;
        pump(app);
        let angle = shown_angle(app);
        answer_cancel(app);
        angle
    }

    /// The angle the selection's fold lines fold at, after selecting `id`.
    fn fold_angle(app: &mut VectorcraftApp, id: u64) -> Option<f64> {
        app.run("select.set", json!({"ids": [id]})).unwrap();
        vectorcraft_engine::cmd::selection_fold_angles(&app.session).angle
    }

    #[test]
    fn set_fold_angle_opens_its_dialog_on_the_selected_lines_angle() {
        let (mut app, a, b) = fold_lines();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        assert_eq!(opened_angle(&mut app), Some(90.0), "a crease folds at 90° until it has an angle of its own");
        app.run(SET_FOLD_ANGLE, json!({"angle": 45})).unwrap();
        assert_eq!(opened_angle(&mut app), Some(45.0));
        // The cut line does not fold, so the crease's angle is still the one shown.
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        assert_eq!(opened_angle(&mut app), Some(45.0));
        // Lines that differ open on 90°.
        let c = app.run("shape.rectangle", json!({"x": 180, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run(SET_ROLE, json!({"role": "partialCut", "ids": [c]})).unwrap();
        app.run("select.set", json!({"ids": [a, c]})).unwrap();
        assert_eq!(opened_angle(&mut app), Some(90.0));
        // With only a cut line selected there is nothing to set: no dialog opens, and ui.menuDialog gives the reason.
        app.run("select.set", json!({"ids": [b]})).unwrap();
        assert_eq!(opened_angle(&mut app), None);
        assert!(app.command_dialog.open.is_none());
        let e = app.run("ui.menuDialog", json!({"command": SET_FOLD_ANGLE})).unwrap_err();
        assert!(e.contains("select a crease, partial cut or perforation line"), "{e}");
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.run("ui.menuDialog", json!({"command": SET_FOLD_ANGLE})).unwrap();
        pump(&mut app);
        assert_eq!(shown_angle(&app), Some(45.0));
        answer_cancel(&mut app);
        // The options dialog opened with an angle keeps it.
        crate::menus::invoke(&mut app, "ui.paramDialog", json!({"command": SET_FOLD_ANGLE, "label": "Set Fold Angle", "params": {"angle": 30}}));
        pump(&mut app);
        assert_eq!(shown_angle(&app), Some(30.0));
    }

    /// Set Fold Angle…'s OK changes the lines the dialog opened on, even when the selection changes while it is open
    /// (a remote command can change it): the other crease keeps its angle.
    #[test]
    fn set_fold_angle_s_ok_changes_the_lines_the_dialog_opened_on() {
        let (mut app, a, b) = fold_lines();
        app.run(SET_ROLE, json!({"role": "crease", "ids": [b]})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        crate::menus::invoke(&mut app, SET_FOLD_ANGLE, json!({}));
        pump(&mut app);
        app.run("select.set", json!({"ids": [b]})).unwrap();
        set(&mut app, "angle", json!(30.0));
        answer_ok(&mut app);
        assert!(app.command_dialog.open.is_none(), "OK closes the dialog");
        assert_eq!(fold_angle(&mut app, a), Some(30.0), "the line the dialog opened on");
        assert_eq!(fold_angle(&mut app, b), Some(90.0), "the line selected later keeps its role's angle");
    }

    /// Set Fold Angle…'s OK changes nothing once another document is the active one, even a line there with the same
    /// id as the line the dialog opened on.
    #[test]
    fn set_fold_angle_s_ok_changes_nothing_in_another_document() {
        let (mut app, a, _) = fold_lines();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        crate::menus::invoke(&mut app, SET_FOLD_ANGLE, json!({}));
        pump(&mut app);
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let other = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(other, a, "the same id in the other document");
        app.run(SET_ROLE, json!({"role": "crease", "ids": [other]})).unwrap();
        set(&mut app, "angle", json!(30.0));
        answer_ok(&mut app);
        assert!(app.command_dialog.open.is_none(), "OK closes the dialog");
        assert_eq!(fold_angle(&mut app, other), Some(90.0), "the other document's line keeps its role's angle");
        assert!(app.session.set_active(0));
        assert_eq!(fold_angle(&mut app, a), Some(90.0), "the first document's line is left as it was");
    }

    /// Set Fold Angle…'s Angle row takes -180° to 180°, the range the command takes, and shows every place the angle
    /// it opens on has.
    #[test]
    fn set_fold_angle_s_angle_row_keeps_the_commands_range() {
        let (mut app, a, _) = fold_lines();
        app.run(SET_FOLD_ANGLE, json!({"angle": 179.999999, "ids": [a]})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        let param = row(SET_FOLD_ANGLE, "angle", &json!(179.999999), Setting::of(&app)).expect("a row");
        match param.kind {
            ParamKind::Number { min, max, decimals, .. } => assert_eq!((min, max, decimals), (Some(-180.0), Some(180.0), 6)),
            _ => panic!("a number row"),
        }
        crate::menus::invoke(&mut app, SET_FOLD_ANGLE, json!({}));
        pump(&mut app);
        assert_eq!(shown_angle(&app), Some(179.999999));
        let (ctx, id) = (app.ui_ctx.clone().unwrap(), id_of(&app));
        assert!(app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, "angle", json!(181))).is_err(), "past the range");
        set(&mut app, "angle", json!(-180));
        answer_ok(&mut app);
        assert_eq!(fold_angle(&mut app, a), Some(-180.0));
    }
}
