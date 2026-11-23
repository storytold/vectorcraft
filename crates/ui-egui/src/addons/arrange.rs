//! Arrange Tools in the interface: its submenu (Object › Plug-ins › Arrange Tools); the dialogs of
//! Arrange in Grid, Replace with Top Object, Set Size of Each, Randomize Opacity and Sort Objects,
//! which preview on the canvas and run their `addon.arrange.*` command on OK; Group Overlapping
//! Objects; and Zoom to Selection ([`ZOOM`]), which fits the active view to the selection.

use std::borrow::Cow;

use craft::menu::Node;
use craft::ui::canvas::ViewCommands;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use serde_json::{Value, json};

use super::dialogs::Spec;
use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;
use crate::dialogs::form;
use crate::dialogs::tool_options::reference_param;
use crate::menus::{Sep, c, cp};

/// Zoom to Selection: a view command, since the view is the interface's.
pub const ZOOM: &str = "addon.arrange.zoomSelection";
const GRID: &str = "addon.arrange.grid";
const REPLACE: &str = "addon.arrange.replace";
const SIZE: &str = "addon.arrange.size";
const OPACITY: &str = "addon.arrange.randomizeOpacity";
const SORT: &str = "addon.arrange.sort";
/// Each dialog's command and heading.
const DIALOGS: [(&str, &str); 5] = [
    (GRID, "Arrange in Grid"),
    (REPLACE, "Replace with Top Object"),
    (SIZE, "Set Size of Each"),
    (OPACITY, "Randomize Opacity"),
    (SORT, "Sort Objects"),
];
/// The widths and heights Set Size of Each takes (pt), and the gaps Arrange in Grid takes.
const SIZES: std::ops::RangeInclusive<f64> = 0.001..=1e6;
const GAPS: std::ops::RangeInclusive<f64> = -1e6..=1e6;
/// The seeds the Randomize Opacity dialog offers.
const SEEDS: u64 = 1_000_000;
/// The units a command's params are in; a row's shown unit does not change them.
const POINTS: LengthUnits = LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0);
/// The attributes Sort Objects sorts by and applies.
const ATTRIBUTES: [(&str, &str); 7] = [
    ("width", "Width"),
    ("height", "Height"),
    ("area", "Area"),
    ("x", "X Position"),
    ("y", "Y Position"),
    ("opacity", "Opacity"),
    ("stacking", "Stacking Order"),
];

/// Object › Plug-ins › Arrange Tools.
pub fn menu() -> Vec<Node> {
    vec![
        c("Arrange in Grid…", GRID),
        cp("Swap Objects", "addon.arrange.swap", json!({})),
        cp("Swap Objects and Sizes", "addon.arrange.swap", json!({ "size": true })),
        c("Replace with Top Object…", REPLACE),
        c("Set Size of Each…", SIZE),
        cp("Group Overlapping Objects", "addon.arrange.groupOverlapping", json!({})),
        c("Randomize Opacity…", OPACITY),
        c("Sort Objects…", SORT),
        Sep,
        cp("Zoom to Selection", ZOOM, json!({})),
    ]
}

static GRID_SPEC: Spec = Spec { command: GRID, params: grid_params, found: None };
static REPLACE_SPEC: Spec = Spec { command: REPLACE, params: replace_params, found: None };
static SIZE_SPEC: Spec = Spec { command: SIZE, params: size_params, found: None };
static OPACITY_SPEC: Spec = Spec { command: OPACITY, params: opacity_params, found: None };
static SORT_SPEC: Spec = Spec { command: SORT, params: sort_params, found: None };

/// The dialog over `rows`, titled by the heading of `command`.
fn dialog<'a>(command: &str, rows: Vec<Param>) -> ParamDialog<'a> {
    let heading = DIALOGS.iter().find(|(c, _)| *c == command).map_or("", |&(_, h)| h);
    ParamDialog::new(command, tl!(heading), Cow::Owned(rows)).previews(true)
}

fn choice(key: &str, label: &'static str, options: &[(&str, &'static str)], default: &str) -> Param {
    let options = options.iter().map(|&(id, text)| (id.to_string(), tl!(text).into())).collect();
    Param::new(key, tl!(label), ParamKind::choice(options, Some(default.into())))
}

fn check(key: &str, label: &'static str, default: bool) -> Param {
    Param::new(key, tl!(label), ParamKind::bool(default))
}

fn length(key: &str, label: &'static str, default: f64, bounds: std::ops::RangeInclusive<f64>, lengths: LengthUnits) -> Param {
    Param::number(key, tl!(label), default, 1.0, 2).bounds(Some(*bounds.start()), Some(*bounds.end())).length(lengths)
}

fn percent(key: &str, label: &'static str, default: f64) -> Param {
    Param::number(key, tl!(label), default, 1.0, 0).bounds(Some(0.0), Some(100.0)).unit(UnitKind::Percent)
}

fn seed() -> Param {
    Param::new("seed", tl!("Seed:"), ParamKind::int(Some(0), Some(SEEDS as i64 - 1), 0))
}

fn scale_strokes() -> Param {
    check("scaleStrokes", "Scale Strokes & Effects", false)
}

fn grid_rows(lengths: LengthUnits) -> Vec<Param> {
    let layouts = [("grid", "Grid by Row"), ("row", "Arrange by Row"), ("column", "Arrange by Column")];
    vec![
        choice("layout", "Layout:", &layouts, "grid"),
        Param::new("columns", tl!("Columns:"), ParamKind::int(Some(1), Some(10_000), 1)).enabled_when("layout", "grid"),
        length("gapX", "H Spacing:", 10.0, GAPS, lengths),
        length("gapY", "V Spacing:", 10.0, GAPS, lengths),
        check("centerOnArtboard", "Center on Artboard", false),
    ]
}

fn replace_rows() -> Vec<Param> {
    let fits = [("keep", "Actual Size"), ("bounds", "Bounds"), ("proportional", "Bounds (Fit Proportionally)")];
    vec![choice("fit", "Size:", &fits, "keep"), check("keep", "Keep Original", false), scale_strokes()]
}

fn size_rows(lengths: LengthUnits) -> Vec<Param> {
    vec![
        check("widthOn", "Width:", true),
        length("width", "", 100.0, SIZES, lengths).enabled_when("widthOn", true),
        check("heightOn", "Height:", true),
        length("height", "", 100.0, SIZES, lengths).enabled_when("heightOn", true),
        // Only one size given: the other can follow it.
        check("proportional", "Constrain Width and Height Proportions", false).enabled_when_differs("widthOn", "heightOn"),
        reference_param("reference", tl!("Reference Point")),
        scale_strokes(),
    ]
}

fn opacity_rows() -> Vec<Param> {
    vec![percent("min", "Minimum:", 30.0), percent("max", "Maximum:", 100.0), seed()]
}

fn sort_rows() -> Vec<Param> {
    let orders = [("ascending", "Small to Large"), ("descending", "Large to Small"), ("random", "Random")];
    // Stacking order only trades places; only sizes scale strokes.
    let sizes = ["width", "height", "area"];
    let placed = ATTRIBUTES.iter().map(|a| a.0).filter(|key| *key != "stacking");
    vec![
        choice("by", "Sort By:", &ATTRIBUTES, "area"),
        choice("apply", "Apply To:", &ATTRIBUTES, "stacking"),
        choice("order", "Order:", &orders, "ascending"),
        check("distribute", "Distribute Evenly", false).enabled_when_in("apply", placed),
        scale_strokes().enabled_when_in("apply", sizes),
    ]
}

fn grid_params(fields: &DialogFields) -> Value {
    Value::Object(form::json_of(&grid_rows(POINTS), fields))
}

fn replace_params(fields: &DialogFields) -> Value {
    Value::Object(form::json_of(&replace_rows(), fields))
}

/// Of Set Size of Each's width and height, those ticked.
fn size_params(fields: &DialogFields) -> Value {
    let mut params = form::json_of(&size_rows(POINTS), fields);
    for side in ["width", "height"] {
        let tick = format!("{side}On");
        params.remove(&tick);
        if fields.bool(&tick) != Some(true) {
            params.remove(side);
        }
    }
    Value::Object(params)
}

fn opacity_params(fields: &DialogFields) -> Value {
    Value::Object(form::json_of(&opacity_rows(), fields))
}

fn sort_params(fields: &DialogFields) -> Value {
    Value::Object(form::json_of(&sort_rows(), fields))
}

/// Opens the dialog of command `id` on the active document; `None` for another command and without a document.
pub(crate) fn open(app: &mut VectorcraftApp, id: &str) -> Option<Result<Value, String>> {
    let &(command, _) = DIALOGS.iter().find(|(command, _)| *command == id)?;
    let st = app.session.active()?;
    let strokes = app.session.prefs.scale_strokes;
    let lengths = general_lengths(app);
    let (rows, opening) = match command {
        // A square grid to start with: as many columns as the square root of the count.
        GRID => (
            grid_rows(lengths),
            json!({
                "layout": "grid", "columns": (st.selection.objects.len().max(1) as f64).sqrt().ceil() as u64,
                "gapX": 10, "gapY": 10, "centerOnArtboard": false,
            }),
        ),
        REPLACE => (replace_rows(), json!({ "fit": "keep", "keep": false, "scaleStrokes": strokes })),
        // A new seed each time the document has changed, so running it again varies the result.
        OPACITY => (opacity_rows(), json!({ "min": 30, "max": 100, "seed": st.revision % SEEDS })),
        // The random order's seed (not shown): new whenever the document has changed, so each run
        // shuffles anew while Preview and OK agree.
        SORT => (
            sort_rows(),
            json!({
                "by": "area", "apply": "stacking", "order": "ascending", "distribute": false, "seed": st.revision % SEEDS, "scaleStrokes": strokes,
            }),
        ),
        // The first selected object's size, as the command measures it (a side it lacks: the least).
        _ => {
            let (w, h) = st.selection.objects.first().and_then(|id| app.session.transform_box(&[*id])).map_or((100.0, 100.0), |b| {
                (b.rect.width().clamp(*SIZES.start(), *SIZES.end()), b.rect.height().clamp(*SIZES.start(), *SIZES.end()))
            });
            (
                size_rows(lengths),
                json!({
                    "width": w, "height": h, "widthOn": true, "heightOn": true, "proportional": false, "reference": 4, "scaleStrokes": strokes,
                }),
            )
        }
    };
    let fields = form::fields_of(&rows, opening.as_object()?);
    let opened = match command {
        GRID => super::dialogs::open(app, &GRID_SPEC, fields, move |_| dialog(GRID, grid_rows(lengths))),
        REPLACE => super::dialogs::open(app, &REPLACE_SPEC, fields, |_| dialog(REPLACE, replace_rows())),
        OPACITY => super::dialogs::open(app, &OPACITY_SPEC, fields, |fields| {
            let (min, max) = (fields.number("min").unwrap_or(0.0), fields.number("max").unwrap_or(100.0));
            dialog(OPACITY, opacity_rows()).ok_enabled(min <= max)
        }),
        SORT => super::dialogs::open(app, &SORT_SPEC, fields, |_| dialog(SORT, sort_rows())),
        _ => super::dialogs::open(app, &SIZE_SPEC, fields, move |fields| {
            let either = fields.bool("widthOn").unwrap_or(true) || fields.bool("heightOn").unwrap_or(true);
            dialog(SIZE, size_rows(lengths)).ok_enabled(either).ok_reason(tl!("Tick Width or Height.").into())
        }),
    };
    Some(opened)
}

/// Zoom to Selection: the active view zooms and centres so the selection's bounds fill the
/// window, with a margin. An error with nothing selected.
pub(crate) fn zoom(app: &mut VectorcraftApp) -> Result<Value, String> {
    let Some(st) = app.session.active().filter(|st| !st.selection.objects.is_empty()) else {
        return Err("nothing selected".into());
    };
    let bounds = st.doc.bounds_of(&st.selection.objects, true);
    // At once, as production fitted: a dialog's OK that selects and zooms shows the fit without another frame.
    app.view_now(ViewCommands::FitLayers, bounds);
    Ok(Value::Null)
}

#[cfg(test)]
mod tests;
