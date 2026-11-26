//! Edit → Edit Colors → Recolor Artwork: the selection's colours in rows of current → new colours,
//! previewed live (engine: `recolor.reduce`, `recolor.randomize`, `recolor.apply`).
//!
//! Assign tab: Colors (Auto, or a count: similar colours merge into rows), Preserve White, Black and
//! Grays (they open as OK on art last left them), the rows (click to select, Shift or Cmd/Ctrl-click
//! to add; a row's arrow excludes or includes it; the hex field edits its new colour, and a
//! double-click on it opens the Color Picker,
//! whose OK sets it in the model it was picked in), the color mode menu and its sliders (HSB, RGB,
//! CMYK or Lab, and Tint when the new color is a spot color of the library), which edit the
//! first selected row's new color (once they have, OK gives the art each new color in its own
//! model), New Row (a row with no current colors above the first selected row, or at the top,
//! with that row's new color; it recolors nothing until Merge moves colors into it, and until then
//! the Edit tab, the Color Theme Picker and Save All Colors leave it out), Merge and
//! Separate, Randomize order or
//! saturation and brightness, Find (with it on, the art of the selected rows shows as the preview
//! shows it, their current colors with Preview off, and the rest of the selected art shows its
//! current colors dimmed; a second click shows the art as before), Reset, the Method and Limit to Library (Other Library… imports a
//! library file and limits to it; a library's colours apply as its swatches: a spot colour stays a
//! spot colour, and joins the document's swatches). A row's right-click menu has Pick from Swatches
//! and Limit to Library. The Edit tab is [`edit_tab`]'s: a hue and
//! saturation wheel with a marker per new colour (drag one; with linked handles they all turn and
//! scale together), a brightness slider, the harmony rules, and the color sliders with their mode
//! menu, whose Global Adjust shows Saturation, Brightness, Temperature and Luminosity (-100% to
//! 100%, 0% changes nothing), which change every new color at once. The buttons beside the wheel
//! show the new colors on the smooth wheel, on a segmented wheel (solid patches of 24 hues by 5
//! saturations; its markers move as freely) or as color bars (one per new color, in row order: click
//! one to select it for the sliders, drag one to move its row, which changes no color); the dialog
//! opens on the smooth wheel. Under them, the Add Color tool adds a click on either wheel as a new
//! color (the hue and saturation clicked, at the wheel's brightness; on the segmented wheel the
//! patch's color) in a new row with no current colors, which becomes the selected one, and the
//! Remove Color tool removes the color whose marker is clicked; a bar's right-click menu has Remove
//! Color too. The base color can't be removed. A removed color's row goes, unless it has current
//! colors and art or a group is open: it then keeps them, with no new color, so that art keeps its
//! colors and the group's swatch at that place goes. Under the tools, a button switches both wheels
//! to hue and brightness (black at the center, full brightness at the edge, at the selected color's
//! saturation; a marker drag sets hue and brightness, Add Color adds the hue and brightness clicked
//! at the selected color's saturation, and the slider under the wheel is a Saturation slider) and
//! back; switching changes no color, and the dialog opens on hue and saturation. The base color is the one the harmony rules
//! build from, drawn with the larger marker; it stays the base when another color is selected for
//! the sliders. A right-click on a marker or a color bar offers Set As Base Color, Select Shade (a
//! grid of the color's hue at 7 saturations by 7 brightnesses; a click moves the marker there as a
//! drag would), Color Picker and Remove Color; a right-click elsewhere on the wheel offers Add New
//! Color, which adds the hue and saturation there as the Add Color tool does, and a color added on
//! the line from the center to a marker moves with that marker when the colors are unlinked. Set
//! Current Color As Base Color, before the color chips, makes the selected color the base. With art, the Prominent Colors bar under the wheel shows the new
//! colors, each part as wide as its row's share of the art's uses, the largest first; dragging the
//! edge between two parts moves width between them, and when the drag ends the rows' current
//! colors move toward those widths (`recolor.weigh`), and the art previews them. The New Color
//! Group menu beside the link button saves the new colors (Save All Colors, in row order) or the
//! bar's colors (Save Prominent Colors, in its order) as a color group right away, a step of its
//! own. The preset menu sets up a 1, 2 or 3
//! colour job or a library. Opened on a colour group (Edit or Apply Color Group), OK also rewrites
//! the group with the new colours (renamed by `groupName`); without art only the group changes.
//! Opened on colours alone (the Color Guide's Edit or Apply Colors without art), OK saves the new
//! colours as a colour group named `groupName`.
//!
//! The Color Groups list beside the tabs (under them in a window too narrow for both; the Show
//! Color Groups button beside the tabs shows or hides it, and it opens shown) lists the document's color groups with their colors. A click on one makes it
//! the group being edited, as opening on it would: with art the art is reduced to as many rows as
//! it has colors, which become their new colors, and without art the rows are its colors. Save
//! Changes rewrites the selected group with the rows' new colors (`swatch.editGroup`, renamed by
//! `groupName`), New Color Group saves them as a new group (`swatch.newGroup`), and Delete removes
//! the selected group with its swatches (`swatch.delete`); each is a step of its own, which the
//! Swatches panel shows. When the art has colors of the group, Save Changes keeps the group's
//! swatches in order: a swatch whose color a row lists takes that row's new color (its own color
//! when the row is excluded), and every other swatch keeps its color. After a save with art the
//! rows' new colors take the Global Adjust, which goes back to 0%. When the save recolors art
//! linked to the group's global swatches, the rows are read from the art again (a color the save
//! gave the art is excluded, and a color the art had before keeps its row's new color). Until the
//! rows or the Global Adjust change, OK changes the saved group only by its name. After a Delete
//! without art, OK saves the rows as a new color group.
//!
//! The Color Theme Picker button beside the tabs turns on picking from the canvas: a click on an
//! object takes the palette of the object a Selection tool click takes, a drag takes the palette
//! of the art inside its marquee, and Shift adds to the picks so far (`recolor.pickTheme`). The
//! rows that aren't excluded and hold current colors (or were added on the Edit tab) take the
//! picked colors as their new colors,
//! the most used color first and repeating when the rows outnumber them, and the art previews
//! them; OK keeps them as one undo step. A new count of Colors keeps the picks: the new rows take them again.
//!
//! Undo Changes and Redo Changes beside the Color Theme Picker (`ui.recolorUndo`, `ui.recolorRedo`)
//! step back and forward through the dialog's own color edits, not the document's undo history;
//! Cancel discards them and OK still records one undo step. Each opening starts with nothing to
//! undo, and selecting another color group or Save Changes leaves nothing to undo or redo.
//!
//! The dialog sits on the window's dialog host ([`RecolorDialog`]); [`pump`] feeds it what it reads of
//! the app, runs what it asked for and previews its recolour, and OK is held until the recolour is
//! committed, which can fail and keep the dialog open.
//!
//! Fields (`ui.dialog.set`): `rows` ([{from: [keys], to: key, exclude?, tint?: percent of a spot
//! color, added?}]; a row with no current colors, `from: []`, recolors no art, and with `added: true`
//! (a color the Add Color tool added) adds its new color to the color group; `to: null`, no new color;
//! `base: true`, the base color, else the first color the Edit tab shows; `tie: n` on a marker and
//! `follows: n` on a color added on its line, which moves with it; the dialog sends `recolor.apply`
//! the rows without these three), `colors` (null: Auto, or a count), `method`, `preserveWhite`, `preserveBlack`,
//! `preserveGrays`, `limitTo` (library id, "document" for the document's swatches, "" for none),
//! `group`, `groupName`, `tab` (`assign` or `edit`), `rule` (harmony id), `linked`, `preview`,
//! `sliderMode` (`hsb`, `rgb`, `cmyk`, `lab` or `tint`; unset: the row color's own model, HSB for
//! a gray and Tint for a spot color; `global`: Global Adjust, on the Edit tab), `globalAdjust`
//! (`{saturation, brightness, temperature, luminosity}` in percent, sent as `recolor.apply`'s
//! `globalAdjust` in every slider mode), `display` (the Edit tab's `smooth`, `segmented` or `bars`;
//! unset: `smooth`), `wheelTool` (`add`: the Add Color tool, `remove`: the Remove Color tool;
//! unset: neither, a click on the wheel selects a marker), `wheelAxes` (`hueBrightness`: the wheels
//! show hue and brightness; `hueSaturation`, unset or any other value: hue and saturation), `themePicker` (the Color Theme Picker is on), `themePicks` (its picks, as
//! `recolor.pickTheme`'s `picks`; a new list recolors the rows), `weights` (one per row, as
//! `recolor.weigh`'s; a new list moves the rows' current colors toward them), `findColors` (Find is
//! on; it shows the art so on the Assign tab, with art and rows selected), `groupStorage` (the
//! Color Groups list is shown), `groupAction` (`save`, `new` or `delete`: the list's buttons,
//! checked as the field is set and run by the next frame, or by OK first). Setting `group` to a color group's name selects it in the list, and null
//! selects none. Changing `colors`, a preserve flag or `limitTo` reduces the rows again (and drops
//! `weights`). The controls `__sel` (the selected rows), `__seed` (Randomize's seed) and
//! `__saveGroup` (the New Color Group menu's `all` or `prominent`) can be set too.

pub(crate) mod edit_tab;
#[cfg(test)]
pub(crate) mod fixture;
mod prominent;

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use craft::behavior::edit_history::{EditHistory, Step};
use craft::behavior::{ColumnKey, ColumnWidth, EditEvent, IconId, Mixed, Selection, SelectionChange, keyed};
use craft::color_ui::ui::{ChannelRow, ChannelUnit, ColorPanelMode, DiscRadius, patch_at};
use craft::host::PickRequest;
use craft::theme::variant::Tone;
use craft::theme::{ModelValues, Rgba, Srgba};
use craft::ui::buttons::{IconButton, ToggleButton};
use craft::ui::choice::{Checkbox, Swatch};
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Trigger,
};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormDialog, ResetFooter, ResetPick};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::runtime::requests::ColorRequest;
use craft::ui::sections::{FooterAction, FooterBar};
use craft::ui::table::Table;
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::{HexField, TextField};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_color::harmony::{self, Harmony, move_on_wheel};
use vectorcraft_color::recolor::{ColorKey, GlobalAdjust, Method};
use vectorcraft_color::{Color, keep_model};
use vectorcraft_engine::cmd::color_value;
use vectorcraft_engine::cmd::fileio::builtin_open_filters;
use vectorcraft_engine::cmd::swatchlib;

use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;
use crate::panels::color::{color_of_values, working_rgb};
use crate::panels::library_panel;
use crate::panels::swatches::{DOCUMENT_SWATCHES, colour_libraries, limit_key};
use crate::panels::{c32, srgba, whole_steps};

/// The dialog kind of Recolor Artwork.
pub const KIND: &str = "recolor";

/// The dialog's title, and the name of the undo step its recolour makes.
const TITLE: &str = "Recolor Artwork";

/// The fields of the Color Groups list: whether it is shown, a request from its buttons, and the
/// group whose colors the rows were last given.
const GROUP_STORAGE: &str = "groupStorage";
/// Rows the Color Groups list shows before it scrolls: as tall as the tabs beside it.
const GROUP_ROWS: u16 = 10;
const GROUP_ACTION: &str = "groupAction";
const GROUP_LOADED: &str = "__groupLoaded";
/// The `recolor.reduce` parameters the art's rows were last reduced with.
const REDUCED: &str = "__reduced";
/// Set by a change that leaves Undo Changes and Redo Changes no step ([`restart_history`]); the dialog's history
/// takes it.
const RESTART: &str = "__restartHistory";

const CMD: &str = "recolor.apply";
const PRESETS: [&str; 5] = ["Custom", "1 Color Job", "2 Color Job", "3 Color Job", "Color Library"];
/// The color modes of the sliders under the rows; a row whose new color is a spot color of the
/// library also has Tint.
const SLIDER_MODES: [ColorPanelMode; 4] =
    [ColorPanelMode::HsbSliders, ColorPanelMode::RgbSliders, ColorPanelMode::CmykSliders, ColorPanelMode::LabSliders];
/// Colours a slider's track shows from its start to its end.
const TRACK_STOPS: usize = 7;
/// The `sliderMode` of the Edit tab's Global Adjust sliders, and the menu's name for it.
const GLOBAL: &str = "global";
const GLOBAL_ADJUST: &str = "Global Adjust";
/// The `globalAdjust` field's keys (and `recolor.apply`'s), in the order of the sliders.
const ADJUSTS: [&str; 4] = ["saturation", "brightness", "temperature", "luminosity"];
/// The slider values last shown: `{row, key, mode, v}` (HSB keeps its hue at no saturation).
const COMPS: &str = "__comps";
/// The `__` controls `ui.dialog.set` can set: the selected rows, Randomize's seed and the New Color Group menu's
/// request.
const SETTABLE: [&str; 3] = ["__sel", "__seed", SAVE_GROUP];
/// The fields `ui.dialog.set` can set while they are unset: Preview, the Color Groups buttons, the Edit tab's slider
/// mode, display, wheel tool and wheel axes, and the Global Adjust, which opens unset (0%).
const UNSET_FIELDS: [&str; 7] = ["preview", GROUP_ACTION, "sliderMode", "display", WHEEL_TOOL, WHEEL_AXES, "globalAdjust"];

/// The columns of the Assign tab's rows: the current colors, the arrow that excludes the row, the
/// new color and its hex.
const CURRENT: ColumnKey = ColumnKey::new("current");
const ARROW: ColumnKey = ColumnKey::new("arrow");
const NEW: ColumnKey = ColumnKey::new("new");
const HEX: ColumnKey = ColumnKey::new("hex");
/// The arrow of a row that is recolored, and of one that is excluded (a dashed line without a head).
const ARROW_ON: &str = "dc-recolor-arrow";
const ARROW_OFF: &str = "dc-recolor-arrow-off";

/// The dialog's values: the JSON map automation lists and sets, and the rows and marks the model
/// functions read and write. `__` names are the dialog's own marks.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Fields {
    /// The values by name.
    pub fields: serde_json::Map<String, Value>,
}

impl Fields {
    /// The fields of the object `fields` (none when it is not an object).
    pub(crate) fn new(fields: Value) -> Self {
        Self { fields: fields.as_object().cloned().unwrap_or_default() }
    }

    /// Field `k` as a number: a number as it is, a string as a length in points; `d` otherwise.
    pub(crate) fn f64(&self, k: &str, d: f64) -> f64 {
        match self.fields.get(k) {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(d),
            Some(Value::String(s)) => vectorcraft_doc::Unit::Points.parse(s).unwrap_or(d),
            _ => d,
        }
    }

    /// Field `k` as text: a string as it is, another value as JSON, empty when missing.
    pub(crate) fn str(&self, k: &str) -> String {
        match self.fields.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        }
    }

    /// Field `k` as a bool; `false` when it is missing or not a bool.
    pub(crate) fn bool(&self, k: &str) -> bool {
        self.fields.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
}

/// The open Recolor Artwork dialog's record on the app (`app.color_dialogs.recolor`): its id and the
/// live preview of its recolour, which outlive a pass of the dialog.
pub(crate) struct Open {
    /// The dialog on the window's host.
    pub id: DialogId,
    /// The preview the pump runs and OK commits.
    pub preview: LivePreview,
}

/// Open Recolor Artwork (`ui.recolorDialog {colors?, library?, group?, preset?}`) on the selected
/// art and, with `group`, on that colour group: art is reduced to as many rows as the group has
/// colours, which become their new colours; without art the rows are the group's colours. `colors`
/// is a count (an n-colour job) or the new colours to assign (without art or a group they are the
/// rows, which OK saves as a new colour group); `library` limits to a library or "document" (""
/// picks the first library). With `preset: true` the Recolor with Preset dialog opens first.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    match p.get("preset") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => return super::recolor_preset::open(app, p),
        Some(_) => return Err("`preset` must be true or false".into()),
    }
    let st = app.session.active().ok_or("no document open")?;
    let art = !st.selection.is_empty();
    let group = match p.get("group") {
        None | Some(Value::Null) => None,
        Some(Value::String(g)) => Some(g.as_str()),
        Some(_) => return Err("`group` must be a colour group name".into()),
    };
    // The group is looked for in the document without the preview of a Recolor Artwork this one
    // replaces (which can rename the group), as that dialog's rollback below leaves it.
    let unpreviewed = match &st.interaction {
        Some(it) if app.color_dialogs.recolor.is_some() => &it.doc,
        _ => &st.doc,
    };
    if let Some(g) = group
        && !unpreviewed.swatch_groups.iter().any(|x| x.name == g)
    {
        return Err(format!("no colour group `{g}`"));
    }
    // Colours alone (no art, no group) become a new colour group.
    let new_group = !art && group.is_none();
    if new_group && !matches!(p.get("colors"), Some(Value::Array(cs)) if cs.iter().any(|c| color_value(c).is_some())) {
        return Err("select artwork (or a colour group) to recolor".into());
    }
    let limit = library_param(app, p)?;
    // A Recolor Artwork still open is replaced: its preview goes before the group's colours are read.
    cancel_open(app);
    let st = app.session.active().ok_or("no document open")?;
    let group_colors: Option<Vec<Color>> = match group {
        Some(g) => {
            let g = st.doc.swatch_groups.iter().find(|x| x.name == g).ok_or_else(|| format!("no colour group `{g}`"))?;
            Some(g.swatches.iter().filter_map(|w| w.paint.color()).collect())
        }
        None => None,
    };
    let assign: Vec<Value> = match (&group_colors, p.get("colors")) {
        (Some(cs), _) => cs.iter().map(key).collect(),
        (None, Some(Value::Array(cs))) => cs.iter().filter_map(color_value).map(|c| key(&c)).collect(),
        _ => vec![],
    };
    let count = match p.get("colors").and_then(Value::as_u64) {
        Some(n) => json!(n.max(1)),
        None if !assign.is_empty() => json!(assign.len()),
        None => Value::Null,
    };
    // Without art the rows are the group's colours, as they are.
    let mut start: Vec<Value> = if art { vec![] } else { assign.iter().map(|k| json!({"from": [k], "to": k})).collect() };
    number_slots(&mut start);
    // The Preserve checkboxes as they were last confirmed (#2960).
    let kept = |k: &str, default: bool| app.ui.recolor_preserve.get(k).and_then(Value::as_bool).unwrap_or(default);
    let fields = json!({
        "rows": start, "colors": count, "method": Method::ScaleTints.id(),
        "preserveWhite": kept("preserveWhite", true), "preserveBlack": kept("preserveBlack", true),
        "preserveGrays": kept("preserveGrays", false), "limitTo": limit,
        "group": group, "groupName": if new_group { Some(NEW_GROUP) } else { group }, "tab": "assign", "rule": Harmony::Complementary.id(), "linked": true,
        "preview": true, THEME_PICKER: false, THEME_PICKS: [], WEIGHTS: null, FIND: false, "__art": art,
        "__assign": assign, "__newGroup": new_group, GROUP_STORAGE: true, GROUP_LOADED: group,
    });
    let mut d = Fields::new(fields);
    // Without art the rows are there from the start: limit them now (art's rows snap as they reduce).
    let mut rs = rows(&d);
    snap(app, &d, &mut rs);
    d.fields.insert("rows".into(), Value::Array(rs));
    let dialog = RecolorDialog::new(app, d);
    let options = HeldOptions { cancel: Some(Box::new(cancelled)), ..HeldOptions::default() };
    let id = open_held_with(app, dialog, confirm, options).ok_or("no window to show Recolor Artwork in")?;
    app.color_dialogs.recolor = Some(Open { id, preview: LivePreview::default() });
    Ok(Value::Null)
}

/// Recolor Artwork closed with Cancel: its preview goes and its record with it, unless a Recolor
/// Artwork opened over it holds the record now.
fn cancelled(app: &mut VectorcraftApp) {
    if app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.is_open::<RecolorDialog>())) {
        return;
    }
    if let Some(open) = app.color_dialogs.recolor.take() {
        open.preview.cancel(app);
    }
}

fn key(c: &Color) -> Value {
    json!(ColorKey::of(c).to_string())
}

/// The name a new colour group (opened on colours alone) starts with.
const NEW_GROUP: &str = "Color Group";

/// Ends an open Recolor Artwork as Cancel ends it: its preview is rolled back and it closes.
pub(super) fn cancel_open(app: &mut VectorcraftApp) {
    let Some(old) = app.color_dialogs.recolor.take() else { return };
    let id = old.id;
    old.preview.cancel(app);
    if let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) {
        dialogs.with(|host| {
            host.cancel_by_id(&ctx, id);
        });
    }
}

fn first_library(app: &VectorcraftApp) -> Option<String> {
    colour_libraries(app).into_iter().next().map(|l| l.id)
}

/// Limit to Library's choices: the key `limitTo` takes and the name the menu shows. None, the
/// document's swatches, then the colour libraries: the built-in ones translated, those the user
/// saved or loaded by their names.
pub(super) fn library_choices(app: &VectorcraftApp) -> Vec<(String, String)> {
    let libs = colour_libraries(app);
    let fixed = [("", tl!("None")), (swatchlib::DOCUMENT_SWATCHES, tl!(DOCUMENT_SWATCHES))].map(|(k, n)| (k.to_owned(), n.to_owned()));
    let named = libs.iter().map(|l| (l.id.clone(), library_panel::library_name(&l.id, &l.name).to_owned()));
    fixed.into_iter().chain(named).collect()
}

/// The `limitTo` key of `ui.recolorDialog`'s `library`: none without it or for null, the first
/// library for ""; a `library` that is not a string is an error.
pub(super) fn library_param(app: &VectorcraftApp, p: &Value) -> Result<String, String> {
    match p.get("library") {
        Some(Value::String(l)) if l.is_empty() => Ok(first_library(app).unwrap_or_default()),
        Some(Value::String(l)) => limit_key(app, l),
        None | Some(Value::Null) => Ok(String::new()),
        Some(_) => Err("`library` must be a library id or name".into()),
    }
}

/// `recolor.reduce` parameters (Method only picks each row's colour, so changing it keeps the rows).
fn reduce_params(d: &Fields) -> Value {
    json!({
        "colors": d.fields.get("colors").cloned().unwrap_or(Value::Null),
        "preserve": {"white": d.bool("preserveWhite"), "black": d.bool("preserveBlack"), "grays": d.bool("preserveGrays")},
        "limitTo": d.str("limitTo"),
    })
}

/// Reduce the art's colours into rows again when the count, preserve flags or library changed
/// (assigning a group's colours or the opener's colours as the new colours).
fn sync(app: &mut VectorcraftApp, d: &mut Fields) {
    const DONE: &str = REDUCED;
    let sig = reduce_params(d);
    if !d.bool("__art") || d.fields.get(DONE) == Some(&sig) {
        return;
    }
    // The art's own colors are reduced, not those of the preview, which runs again afterwards.
    cancel_preview(app);
    let mut p = sig.clone();
    p["method"] = json!(d.str("method"));
    match app.session.execute("recolor.reduce", &p) {
        Ok(r) => {
            let mut rows = r["map"].as_array().cloned().unwrap_or_default();
            let assign = d.fields.get("__assign").and_then(Value::as_array).cloned().unwrap_or_default();
            assign_colors(&mut rows, assign);
            number_slots(&mut rows);
            snap(app, d, &mut rows);
            d.fields.insert("rows".into(), Value::Array(rows));
            // Find dims the preserved colors with the rows that aren't selected.
            d.fields.insert(PRESERVED.into(), r["preserved"].clone());
            d.fields.insert("__sel".into(), json!([]));
            // The new rows hold no slider edit.
            d.fields.remove(COMPS);
        }
        Err(e) => app.status(e.to_string()),
    }
    d.fields.insert(DONE.into(), sig);
    // New rows take the colors picked with the Color Theme Picker again.
    d.fields.remove(THEME_DONE);
    // Weights given for the old rows don't apply to the new ones.
    d.fields.insert(WEIGHTS.into(), Value::Null);
    d.fields.remove(WEIGHED);
}

/// Gives `rows` the new colors `assign`: a row that lists one of them takes it, so art in the group's own colors (art
/// linked to its swatches) keeps them; the others go to the other rows in order.
fn assign_colors(rows: &mut [Value], assign: Vec<Value>) {
    let mut given = vec![false; rows.len()];
    let mut rest = Vec::new();
    for to in assign {
        let own = row_listing(rows, &to).and_then(|r| rows.iter().position(|x| std::ptr::eq(x, r))).filter(|&i| !given[i]);
        match own {
            Some(i) => {
                rows[i]["to"] = to;
                given[i] = true;
            }
            None => rest.push(to),
        }
    }
    for (row, to) in rows.iter_mut().zip(given).filter_map(|(row, given)| (!given).then_some(row)).zip(rest) {
        row["to"] = to;
    }
}

/// The fields of Prominent Colors: the weights the rows are to take (`recolor.weigh`'s `weights`,
/// one per row; a new list moves the rows' current colors), the weights they last took, each row's
/// share of the art's uses (`recolor.weigh`'s `shares`) and the rows' current colors, exclusions
/// and whether each has a new color, which those were read for; and a request from the New Color
/// Group menu (`all` or `prominent`).
const WEIGHTS: &str = "weights";
const WEIGHED: &str = "__weighed";
const SHARES: &str = "__shares";
const SHARES_OF: &str = "__sharesOf";
const SAVE_GROUP: &str = "__saveGroup";

/// Cancel the preview, so commands read the art's own colors; the pump runs it again afterwards.
fn cancel_preview(app: &mut VectorcraftApp) {
    if let Some(preview) = app.color_dialogs.recolor.as_mut().map(|open| std::mem::take(&mut open.preview)) {
        preview.cancel(app);
    }
}

/// With art: when `weights` changed, move the rows' current colors toward them
/// (`recolor.weigh`); when the rows' current colors changed, read each row's share of the art's
/// uses again.
fn sync_weights(app: &mut VectorcraftApp, d: &mut Fields) {
    if !d.bool("__art") {
        return;
    }
    if let Some(w) = d.fields.get(WEIGHTS).filter(|w| !w.is_null()).cloned()
        && d.fields.get(WEIGHED) != Some(&w)
    {
        d.fields.insert(WEIGHED.into(), w.clone());
        cancel_preview(app);
        match app.session.execute("recolor.weigh", &json!({"map": rows(d), "weights": w})) {
            Ok(r) => {
                if let Some(m) = r["map"].as_array() {
                    let mut m = m.clone();
                    keep_marks(&rows(d), &mut m);
                    d.fields.insert("rows".into(), Value::Array(m));
                }
            }
            Err(e) => app.status(e.to_string()),
        }
    }
    let rs = rows(d);
    // A row with a new color takes a color before one without, so that changes the shares too.
    let of = Value::Array(rs.iter().map(|r| json!([r["from"], excluded(r), !r["to"].is_null()])).collect());
    if d.fields.get(SHARES_OF) == Some(&of) {
        return;
    }
    cancel_preview(app);
    let shares = match app.session.execute("recolor.weigh", &json!({"map": rs})) {
        Ok(r) => r["shares"].clone(),
        Err(e) => {
            app.status(e.to_string());
            json!([])
        }
    };
    d.fields.insert(SHARES.into(), shares);
    d.fields.insert(SHARES_OF.into(), of);
}

/// The parts of the Prominent Colors bar: each row that isn't excluded and has a share of the
/// art's uses, with that share in percent, the largest share first (on a tie, the first row).
fn prominent_parts(d: &Fields, rows: &[Value]) -> Vec<(usize, f64)> {
    let shares = d.fields.get(SHARES).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut parts: Vec<(usize, f64)> = shares
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some((i, s.as_f64().filter(|s| *s > 0.0)?)))
        .filter(|(i, _)| rows.get(*i).is_some_and(|r| !excluded(r)))
        .collect();
    parts.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    parts
}

/// The colors a new color group takes from rows `rs`: each row's new color with the Global Adjust,
/// snapped to Limit to Library, as `recolor.apply` gives them to a group; a row without a new color
/// gives its first current color, which the art keeps.
fn group_colors<'a>(app: &VectorcraftApp, d: &Fields, rs: impl Iterator<Item = &'a Value>) -> Vec<Value> {
    let adjust = global_adjust(adjust_values(d));
    let limit = d.str("limitTo");
    let palette = Some(limit).filter(|l| !l.is_empty()).and_then(|l| swatchlib::limit_palette(&app.session, &l));
    rs.filter_map(|r| match to_color(r) {
        None => from_colors(r).first().map(key),
        Some(_) if adjust.is_identity() && palette.is_none() => Some(r["to"].clone()),
        Some(c) => {
            let c = adjust.apply(c);
            Some(key(&palette.as_ref().map_or(c, |p| p.nearest(c))))
        }
    })
    .collect()
}

/// The New Color Group menu's request: save the new colors of the rows that aren't excluded and
/// hold current colors or were added on the Edit tab (Save All Colors, in row order) or of the
/// Prominent Colors bar (Save Prominent Colors, in its order) as a new color group, a step of its
/// own (`swatch.newGroup`). Either way a row whose new color was removed gives none.
fn save_group(app: &mut VectorcraftApp, d: &mut Fields, which: &str) -> Result<(), String> {
    let rs = rows(d);
    let order: Vec<usize> = match which {
        "prominent" => prominent_parts(d, &rs).into_iter().map(|p| p.0).filter(|&i| to_color(&rs[i]).is_some()).collect(),
        _ => (0..rs.len()).filter(|&i| in_group(&rs[i]) && to_color(&rs[i]).is_some()).collect(),
    };
    let colors = group_colors(app, d, order.iter().filter_map(|&i| rs.get(i)));
    if colors.is_empty() {
        return Ok(());
    }
    // The group is added to the document itself, not to the preview, which runs again afterwards.
    cancel_preview(app);
    app.run("swatch.newGroup", json!({"colors": colors, "keepModel": true})).map(|_| ())
}

/// The colors of the document's color groups, by name, in the Swatches panel's order.
fn doc_groups(app: &VectorcraftApp) -> Vec<(String, Vec<Color>)> {
    let Some(st) = app.session.active() else { return vec![] };
    st.doc.swatch_groups.iter().map(|g| (g.name.clone(), g.swatches.iter().filter_map(|w| w.paint.color()).collect())).collect()
}

/// Refuse a `group` that names no color group (`has` says which names one does) or isn't a name
/// or null.
fn check_group(want: &Value, has: impl Fn(&str) -> bool) -> Result<(), String> {
    match want {
        Value::Null => Ok(()),
        Value::String(n) if has(n) => Ok(()),
        Value::String(n) => Err(format!("no colour group `{n}`")),
        v => Err(format!("`group` must be a color group name, not {v}")),
    }
}

/// Make the `group` field's color group (none when it is null) the group being edited, when its
/// colors aren't the rows' yet: with art the art is reduced again to as many rows as the group has
/// colors, which become their new colors (as [`open`] does); without art the rows are its colors.
/// A name no color group has is refused ([`check_group`]), and the field goes back to the group
/// being edited.
fn select_group(app: &mut VectorcraftApp, d: &mut Fields) -> Result<(), String> {
    let want = d.fields.get("group").cloned().unwrap_or(Value::Null);
    let loaded = d.fields.get(GROUP_LOADED).cloned().unwrap_or(Value::Null);
    if want == loaded {
        return Ok(());
    }
    // The preview gives the group being edited the name in `groupName`, which the list shows: unless
    // the document (without the preview) has another group of that name, it is the same group.
    if let Value::String(n) = &want
        && loaded.is_string()
        && *n == d.str("groupName")
        && app.session.in_interaction()
    {
        cancel_preview(app);
        if !doc_groups(app).iter().any(|(g, _)| g == n) {
            d.fields.insert("group".into(), loaded);
            return Ok(());
        }
    }
    // The group's own colors, not those the preview (which rewrites the group being edited) shows.
    if let Value::String(n) = &want
        && doc_groups(app).iter().any(|(g, _)| g == n)
    {
        cancel_preview(app);
    }
    let groups = doc_groups(app);
    if let Err(e) = check_group(&want, |n| groups.iter().any(|(g, _)| g == n)) {
        d.fields.insert("group".into(), loaded);
        return Err(e);
    }
    let art = d.bool("__art");
    let Some((name, colors)) = want.as_str().and_then(|n| groups.into_iter().find(|(g, _)| g == n)) else {
        // No group: without art OK saves the rows as a new color group.
        d.fields.insert(GROUP_LOADED.into(), Value::Null);
        if !art {
            d.fields.insert("__newGroup".into(), json!(true));
            d.fields.insert("groupName".into(), json!(NEW_GROUP));
        }
        restart_history(d);
        return Ok(());
    };
    let assign: Vec<Value> = colors.iter().map(key).collect();
    d.fields.insert(GROUP_LOADED.into(), json!(name));
    d.fields.insert("groupName".into(), json!(name));
    d.fields.insert("__newGroup".into(), json!(false));
    // The group's colors take the place of colors picked with the Color Theme Picker, and the same
    // picks made again apply to its rows.
    d.fields.insert(THEME_PICKS.into(), json!([]));
    d.fields.remove(THEME_DONE);
    d.fields.remove(SAVED_ROWS);
    d.fields.remove(SAVED_COLORS);
    d.fields.remove(SWATCH_ROWS);
    if art {
        // The art is reduced again (`sync`) into a row per color of the group.
        d.fields.insert("colors".into(), json!(assign.len().max(1)));
        d.fields.remove(REDUCED);
    } else {
        let mut rs: Vec<Value> = assign.iter().map(|k| json!({"from": [k], "to": k})).collect();
        number_slots(&mut rs);
        snap(app, d, &mut rs);
        d.fields.insert("rows".into(), Value::Array(rs));
        d.fields.insert("__sel".into(), json!([]));
        d.fields.remove(COMPS);
    }
    d.fields.insert("__assign".into(), Value::Array(assign));
    restart_history(d);
    Ok(())
}

/// Run a button of the Color Groups list: `save` rewrites the selected group with the rows' new
/// colors as OK gives them to it (with art, followed by the group's colors no row took) and the
/// name in `groupName`; `new` saves the new colors of the rows that aren't excluded as a new group;
/// `delete` removes the selected group and its swatches. Each is an undo step of its own on the
/// document, not on the preview, which runs again afterwards.
fn group_action(app: &mut VectorcraftApp, d: &mut Fields, action: &Value) -> Result<(), String> {
    check_group_action(d, action, |g| holds_other_paints(app, g))?;
    let group = d.fields.get("group").and_then(Value::as_str).map(str::to_owned);
    // The rows as OK reads them (a group just selected reduces the art again).
    sync(app, d);
    sync_theme(app, d);
    sync_weights(app, d);
    match action.as_str() {
        Some("new") => save_group(app, d, "all"),
        Some("save") if unchanged_since_save(d) => {
            // Nothing changed since the last save but perhaps the name.
            let name = d.str("groupName");
            if name.trim().is_empty() || group.as_deref() == Some(name.as_str()) {
                restart_history(d);
                return Ok(());
            }
            cancel_preview(app);
            let colors: Vec<Value> = doc_groups(app)
                .into_iter()
                .find(|(n, _)| group.as_deref() == Some(n.as_str()))
                .map(|g| g.1.iter().map(key).collect())
                .unwrap_or_default();
            let r = app.run("swatch.editGroup", json!({"group": group, "colors": colors, "rename": name}))?;
            let name = r["name"].clone();
            d.fields.insert("group".into(), name.clone());
            d.fields.insert(GROUP_LOADED.into(), name.clone());
            d.fields.insert("groupName".into(), name);
            restart_history(d);
            Ok(())
        }
        Some("save") => {
            let rs = rows(d);
            let p = apply_params(d);
            // With art, the group's colors no row took follow, adjusted and snapped as OK gives them,
            // then the colors added on the Edit tab; a row made with New Row that holds no current
            // colors gives the group no color.
            cancel_preview(app);
            let own: Vec<Value> = doc_groups(app)
                .into_iter()
                .find(|(n, _)| group.as_deref() == Some(n.as_str()))
                .map(|g| g.1.iter().map(key).collect())
                .unwrap_or_default();
            // An excluded row leaves its swatch the color it has now, as OK's `"keep"` does: with
            // art, the swatch's color in `__assign`; without art, the swatch's color in the group,
            // which an earlier Save Changes may have changed.
            let filled = by_place(&rs);
            let rest: Vec<Value> =
                p["groupColors"].as_array().map(|a| a.iter().skip(filled.len()).map(|k| json!({"from": [k], "to": k})).collect()).unwrap_or_default();
            let assign: Vec<Value> =
                if d.bool("__art") { d.fields.get("__assign").and_then(Value::as_array).cloned().unwrap_or_default() } else { own.clone() };
            let mut colors: Vec<Value> = filled
                .iter()
                .enumerate()
                .filter_map(|(i, r)| if excluded(r) { Some(kept_color(&assign, i)) } else { saved_color(app, d, r) })
                .chain(group_colors(app, d, rest.iter()))
                .collect();
            // With art in a color of the group, the group keeps its swatch order: a swatch whose row
            // ([`swatch_row`]) is found takes that row's new color (its own color when the row is
            // excluded), so the art linked to it takes the color its row shows, and every other
            // swatch keeps its color. The colors added on the Edit tab follow.
            let found: Vec<Option<(Value, &Value)>> = (0..own.len()).map(|i| swatch_row(d, &rs, &own, i)).collect();
            let mut links = None;
            if d.bool("__art") && found.iter().any(Option::is_some) {
                let added = rs.iter().filter(|r| no_current(r) && is_added(r) && !excluded(r) && !r["to"].is_null());
                let mut saved_links = vec![];
                colors = vec![];
                for (k, f) in own.iter().zip(&found) {
                    let c = match f {
                        Some((_, r)) if !excluded(r) => saved_color(app, d, r).unwrap_or_else(|| k.clone()),
                        _ => k.clone(),
                    };
                    // A removed swatch leaves the saved group.
                    if !c.is_null() {
                        saved_links.push(f.as_ref().map_or(Value::Null, |f| f.0.clone()));
                    }
                    colors.push(c);
                }
                let added = group_colors(app, d, added);
                saved_links.extend(added.iter().map(|_| Value::Null));
                colors.extend(added);
                links = Some(saved_links);
            }
            let before = art_colors(app);
            let r = app.run("swatch.editGroup", json!({"group": group, "colors": colors, "keepModel": true, "rename": p["rename"]}))?;
            let name = r["name"].clone();
            d.fields.insert("group".into(), name.clone());
            d.fields.insert(GROUP_LOADED.into(), name.clone());
            d.fields.insert("groupName".into(), name);
            // The rows follow the saved group, so a later edit or OK does not remove or add a color
            // again: a removed color's row goes (its art keeps its colors). Without art, a color added
            // on the Edit tab becomes a row of its own color, after the rows of the group's other
            // colors; with art, its row goes and the color is one of the group's colors no row takes.
            let art = d.bool("__art");
            let joins = |r: &Value| no_current(r) && is_added(r) && !excluded(r) && !r["to"].is_null();
            let gone: Vec<Value> =
                rows(d).iter().filter(|r| color_removed(r)).flat_map(|r| r["from"].as_array().cloned().unwrap_or_default()).collect();
            let (mut joined, mut rs): (Vec<Value>, Vec<Value>) = rows(d).into_iter().filter(|r| !color_removed(r)).partition(|r| joins(r));
            if art {
                joined.clear();
            }
            for r in &mut joined {
                r["from"] = json!([r["to"].clone()]);
                if let Some(o) = r.as_object_mut() {
                    o.remove("added");
                }
            }
            // The save added them after the group's other colors, whatever the rows' order.
            rs.extend(joined);
            if rs != rows(d) {
                d.fields.insert("__sel".into(), json!([]));
            }
            d.fields.insert("rows".into(), Value::Array(rs));
            if art {
                // The group's colors no row takes are those of the saved group.
                let saved: Vec<Value> = doc_groups(app)
                    .into_iter()
                    .find(|(n, _)| Some(n.as_str()) == r["name"].as_str())
                    .map(|g| g.1.iter().map(key).collect())
                    .unwrap_or_default();
                d.fields.insert("__assign".into(), Value::Array(saved));
                match links {
                    Some(l) => d.fields.insert(SWATCH_ROWS.into(), Value::Array(l)),
                    None => d.fields.remove(SWATCH_ROWS),
                };
                // The saved group holds the Global Adjust: the rows' new colors take it, and it goes
                // back to 0%, so neither the preview nor OK applies it again.
                bake_adjust(app, d);
                let rs = rows(d);
                if art_colors(app) == before {
                    mark_saved(app, d);
                } else {
                    reread_rows(app, d, &rs, &before, &gone);
                }
            }
            restart_history(d);
            Ok(())
        }
        Some("delete") => {
            cancel_preview(app);
            app.run("swatch.delete", json!({"names": [group]}))?;
            d.fields.insert("group".into(), Value::Null);
            select_group(app, d)
        }
        _ => Ok(()),
    }
}

/// The color Save Changes gives the group's swatch for `row`: `null` when the row's color was
/// removed, which removes the swatch as OK does, otherwise its new color as OK gives it.
fn saved_color(app: &VectorcraftApp, d: &Fields, row: &Value) -> Option<Value> {
    if color_removed(row) { Some(Value::Null) } else { group_colors(app, d, std::iter::once(row)).into_iter().next() }
}

/// After Save Changes recolored the art linked to the group's global swatches, read the rows from
/// the art again. A row that lists a color the save gave the art (a paint of a selected object
/// that differs from `before`, [`art_colors`] before the save) is excluded with its first color,
/// so the art keeps it, also when other art has that color: `recolor.apply` recolors every use
/// of a color alike, so the row cannot keep that art's earlier new color without undoing the save. Any other row whose first current color one of the `old` rows listed takes
/// that row's new color and exclusion (art the save left as it was keeps its recolor); the rest
/// are excluded too. A row that lists only `gone` colors (those of swatches the save removed) is
/// left out: its art keeps its colors, and the row gives the group no color.
fn reread_rows(app: &mut VectorcraftApp, d: &mut Fields, old: &[Value], before: &[Vec<Value>], gone: &[Value]) {
    let given: Vec<Value> =
        art_colors(app).iter().zip(before).flat_map(|(now, was)| now.iter().zip(was).filter(|(n, w)| n != w).map(|(n, _)| n.clone())).collect();
    // A swatch whose color the save gave the art is found by that color again: the row its old
    // color found lists other art now.
    let group = d.fields.get("group").and_then(Value::as_str).map(str::to_owned);
    let saved: Vec<Value> =
        doc_groups(app).into_iter().find(|(n, _)| group.as_deref() == Some(n.as_str())).map(|g| g.1.iter().map(key).collect()).unwrap_or_default();
    if let Some(Value::Array(links)) = d.fields.get_mut(SWATCH_ROWS) {
        for (l, c) in links.iter_mut().zip(&saved) {
            if given.contains(c) {
                *l = Value::Null;
            }
        }
    }
    d.fields.insert(THEME_PICKS.into(), json!([]));
    d.fields.remove(REDUCED);
    sync(app, d);
    let mut rs = rows(d);
    for r in &mut rs {
        let Some(first) = r["from"].as_array().and_then(|f| f.first()).cloned() else { continue };
        let from_save = r["from"].as_array().is_some_and(|f| f.iter().any(|c| given.contains(c)));
        match row_listing(old, &first).filter(|_| !from_save) {
            Some(o) => {
                r["to"] = o["to"].clone();
                r["exclude"] = json!(excluded(o));
            }
            None => {
                r["to"] = first;
                r["exclude"] = json!(true);
            }
        }
    }
    rs.retain(|r| r["from"].as_array().is_none_or(|f| f.is_empty() || !f.iter().all(|c| gone.contains(c))));
    d.fields.insert("rows".into(), Value::Array(rs));
    mark_saved(app, d);
}

/// The Global Adjust at 0%: every amount, none changing a color.
fn zero_adjust() -> Value {
    Value::Object(ADJUSTS.iter().map(|k| (k.to_string(), json!(0.0))).collect())
}

/// Give each row's new color the Global Adjust (and Limit to Library) as OK would, and set the
/// Global Adjust back to 0%; nothing changes when it is 0%.
fn bake_adjust(app: &VectorcraftApp, d: &mut Fields) {
    if global_adjust(adjust_values(d)).is_identity() {
        return;
    }
    let mut rs = rows(d);
    for r in &mut rs {
        if to_color(r).is_some()
            && let Some(c) = group_colors(app, d, std::iter::once(&*r)).into_iter().next()
        {
            r["to"] = c;
        }
    }
    d.fields.insert("rows".into(), Value::Array(rs));
    d.fields.insert("globalAdjust".into(), zero_adjust());
}

/// After Save Changes with art: until the rows or the Global Adjust change, OK and the preview
/// leave the selected group's colors as they were saved.
fn mark_saved(app: &VectorcraftApp, d: &mut Fields) {
    let group = d.fields.get("group").and_then(Value::as_str).map(str::to_owned);
    let saved: Vec<Value> =
        doc_groups(app).into_iter().find(|(n, _)| group.as_deref() == Some(n.as_str())).map(|g| g.1.iter().map(key).collect()).unwrap_or_default();
    d.fields.insert(SAVED_COLORS.into(), Value::Array(saved));
    d.fields.insert(SAVED_ROWS.into(), Value::Array(rows(d)));
    d.fields.insert(SAVED_ADJUST.into(), json!(adjust_values(d)));
}

/// Whether the rows and the Global Adjust are as [`mark_saved`] recorded them.
fn unchanged_since_save(d: &Fields) -> bool {
    d.fields.get(SAVED_ROWS).and_then(Value::as_array).is_some_and(|s| *s == rows(d))
        && d.fields.get(SAVED_ADJUST).is_some_and(|a| *a == json!(adjust_values(d)))
}

/// The Global Adjust amounts [`mark_saved`] recorded.
const SAVED_ADJUST: &str = "__savedAdjust";

/// The colors of each selected object and what is inside it, as row keys, in paint order.
fn art_colors(app: &VectorcraftApp) -> Vec<Vec<Value>> {
    let Some(st) = app.session.active() else { return vec![] };
    st.selection
        .objects
        .iter()
        .map(|id| {
            let mut cs = vec![];
            if let Some(n) = st.doc.node(*id) {
                vectorcraft_doc::swatches::node_colors(n, &mut |c, _| cs.push(key(c)));
            }
            cs
        })
        .collect()
}

/// The rows [`mark_saved`] recorded after Save Changes; while the rows are these, OK and the preview
/// leave the group's colors as saved ([`SAVED_COLORS`]) and change only its name.
const SAVED_ROWS: &str = "__savedRows";

/// The selected group's colors after Save Changes. OK with a new name keeps the color of each of
/// those swatches (`"keep"`).
const SAVED_COLORS: &str = "__savedColors";

/// With art, for each swatch of the selected group after Save Changes, the color that found the row
/// that gave it its new color (`null` for a swatch no row gave one), so a later Save Changes or OK
/// finds that row again when the row still lists its old current color.
const SWATCH_ROWS: &str = "__swatchRows";

/// The row that gives the swatch at place `i` of `own` (the group's colors) its new color, with the
/// color that finds it: the row listing the color [`SWATCH_ROWS`] holds for that swatch, else the
/// first row listing the swatch's own color.
fn swatch_row<'a>(d: &Fields, rs: &'a [Value], own: &[Value], i: usize) -> Option<(Value, &'a Value)> {
    let links = d.fields.get(SWATCH_ROWS).and_then(Value::as_array).filter(|l| l.len() == own.len());
    let link = links.and_then(|l| l.get(i)).filter(|l| !l.is_null());
    link.and_then(|l| row_listing(rs, l).map(|r| (l.clone(), r))).or_else(|| own.get(i).and_then(|k| row_listing(rs, k).map(|r| (k.clone(), r))))
}

/// The color an excluded row leaves its swatch, the row being the `i`th of the rows with current
/// colors: the swatch's own color in `assign` (the group's colors), else `null`, which adds no
/// swatch past the group's, as OK's `"keep"` does.
fn kept_color(assign: &[Value], i: usize) -> Value {
    assign.get(i).cloned().unwrap_or(Value::Null)
}

/// Refuse a `groupAction` other than `save`, `new` or `delete`, `save` or `delete` with no group
/// selected, and `save` on a group with a swatch that is not a color (a gradient, pattern or None
/// swatch; `other_paints` says which groups have one): the rows hold only the group's colors, so the
/// save would replace or remove it.
fn check_group_action(d: &Fields, action: &Value, other_paints: impl Fn(&str) -> bool) -> Result<(), String> {
    let group = d.fields.get("group").and_then(Value::as_str);
    match (action.as_str(), group) {
        (Some("new"), _) => Ok(()),
        (Some("save"), Some(g)) if other_paints(g) => {
            Err(format!("color group `{g}` holds swatches that are not colors, which Save Changes can't keep"))
        }
        (Some("save" | "delete"), Some(_)) => Ok(()),
        (Some("save" | "delete"), None) => Err("missing `group`".into()),
        _ => Err(format!("`{GROUP_ACTION}` must be \"save\", \"new\" or \"delete\", not {action}")),
    }
}

/// The color groups with a swatch that is not a color, in the document as it is without the
/// preview (which can rewrite or rename the group being edited).
fn other_paint_groups(app: &VectorcraftApp) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
    doc.swatch_groups.iter().filter(|g| g.swatches.iter().any(|w| w.paint.color().is_none())).map(|g| g.name.clone()).collect()
}

/// Whether color group `group` has a swatch that is not a color ([`other_paint_groups`]).
fn holds_other_paints(app: &VectorcraftApp, group: &str) -> bool {
    other_paint_groups(app).iter().any(|g| g == group)
}

/// Run the button of the Color Groups list that `groupAction` holds, if any.
fn run_group_action(app: &mut VectorcraftApp, d: &mut Fields) -> Result<(), String> {
    match d.fields.remove(GROUP_ACTION) {
        Some(a) => group_action(app, d, &a),
        None => Ok(()),
    }
}

/// The fields of the Color Theme Picker: whether a click or drag on the canvas picks colors, and
/// the picks so far (`recolor.pickTheme`'s `picks`); the picks the rows last took them from.
const THEME_PICKER: &str = "themePicker";
const THEME_PICKS: &str = "themePicks";
const THEME_DONE: &str = "__themePicked";

/// When the picks of the Color Theme Picker changed, give the rows that aren't excluded and hold
/// current colors or were added on the Edit tab the palette of the art picked (`recolor.pickTheme`, as many colors as those
/// rows), in row order; with fewer colors than rows, the colors repeat. Picks that touch no art
/// leave the rows as they are.
fn sync_theme(app: &mut VectorcraftApp, d: &mut Fields) {
    let picks = d.fields.get(THEME_PICKS).cloned().unwrap_or(Value::Null);
    if !picks.as_array().is_some_and(|a| !a.is_empty()) || d.fields.get(THEME_DONE) == Some(&picks) {
        return;
    }
    d.fields.insert(THEME_DONE.into(), picks.clone());
    let mut rs = rows(d);
    let active = active_rows(&rs);
    if active.is_empty() {
        return;
    }
    // The art as it is, not as the preview shows it; the preview runs again afterwards.
    cancel_preview(app);
    let colors = match app.session.execute("recolor.pickTheme", &json!({"picks": picks, "colors": active.len().min(64)})) {
        Ok(r) => r["colors"].as_array().map(|a| a.iter().filter_map(color_value).collect::<Vec<_>>()).unwrap_or_default(),
        Err(e) => return app.status(e.to_string()),
    };
    if colors.is_empty() {
        return;
    }
    for (n, &i) in active.iter().enumerate() {
        set_to(&mut rs[i], &colors[n % colors.len()]);
    }
    snap(app, d, &mut rs);
    d.fields.insert("rows".into(), Value::Array(rs));
    // The rows hold no slider edit.
    d.fields.remove(COMPS);
}

/// The picks recolor.pickTheme takes at most; the oldest go first.
const MAX_PICKS: usize = 256;

/// With Limit to Library, the rows' new colours snap to the library's nearest colours.
fn snap(app: &VectorcraftApp, d: &Fields, rows: &mut [Value]) {
    let Some(palette) = Some(d.str("limitTo")).filter(|l| !l.is_empty()).and_then(|l| swatchlib::limit_palette(&app.session, &l)) else { return };
    for r in rows {
        if let Some(c) = to_color(r) {
            r["to"] = key(&palette.nearest(c));
        }
    }
}

fn rows(d: &Fields) -> Vec<Value> {
    d.fields.get("rows").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn from_colors(row: &Value) -> Vec<Color> {
    row["from"].as_array().map(|a| a.iter().filter_map(color_value).collect()).unwrap_or_default()
}

fn to_color(row: &Value) -> Option<Color> {
    color_value(&row["to"])
}

/// Give `row` the new color `c`, without a tint of the spot color it had or the model a slider
/// edit gave it.
fn set_to(row: &mut Value, c: &Color) {
    row["to"] = key(c);
    if let Some(r) = row.as_object_mut() {
        r.remove("tint");
        r.remove("model");
    }
}

/// Give each row of `rows` whose current colors and new color are those of the same row of
/// `before` that row's tint and model again, when it has none: a change that snaps back to the
/// same new color keeps the tint of it and the model a slider edit gave it.
fn keep_tints(before: &[Value], rows: &mut [Value]) {
    for (r, b) in rows.iter_mut().zip(before) {
        if r["from"] != b["from"] || r["to"] != b["to"] {
            continue;
        }
        for k in ["tint", "model"] {
            if r.get(k).is_none()
                && let Some(v) = b.get(k)
            {
                r[k] = v.clone();
            }
        }
    }
}

fn excluded(row: &Value) -> bool {
    row["exclude"].as_bool().unwrap_or(false)
}

/// Whether `row` has no current colors (a row made with New Row, or a color added on the Edit tab).
fn no_current(row: &Value) -> bool {
    row["from"].as_array().is_none_or(Vec::is_empty)
}

/// Whether `row` holds current colors and its new color was removed (`to: null`) and isn't excluded:
/// its swatch in the color group is removed.
fn color_removed(row: &Value) -> bool {
    !excluded(row) && !no_current(row) && row["to"].is_null()
}

/// Whether `row` is a color added on the Edit tab (`added`), which joins the color group.
fn is_added(row: &Value) -> bool {
    row["added"].as_bool().unwrap_or(false)
}

/// Whether `row` is a row made with New Row that holds no current colors, until colors are merged
/// into it. Its new color recolors nothing and is no color of a color group, the wheel or the
/// harmony rules. A color added on the Edit tab (`added`) is not empty: it joins the color group.
fn empty(row: &Value) -> bool {
    row["from"].as_array().is_some_and(Vec::is_empty) && !is_added(row)
}

/// Whether `row`'s new color is one of the colors a color group takes: the row isn't excluded and
/// holds current colors or is a color added on the Edit tab.
fn in_group(row: &Value) -> bool {
    !excluded(row) && !empty(row)
}

/// The seed for the next Randomize: one past the last, held at `u64::MAX`.
fn next_seed(d: &Fields) -> u64 {
    (d.f64("__seed", 0.0) as u64).saturating_add(1)
}

fn selection(d: &Fields) -> Vec<usize> {
    d.fields.get("__sel").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|i| i as usize).collect()).unwrap_or_default()
}

/// `recolor.apply` parameters: the rows, method, library and the group with its new name and
/// colors: the new colors of the rows with current colors, each in its swatch's place (`null`
/// for a row whose new color was removed, which removes its swatch, and `"keep"` for an excluded
/// row, which leaves its swatch its color), then (with art) the group's colors no row took, then the
/// colors added on the Edit tab. With art in a color of the group, each swatch whose row
/// ([`swatch_row`]) is found takes that row's new color instead, and every other swatch keeps its
/// color, as Save Changes gives them.
fn apply_params(d: &Fields) -> Value {
    let rs = rows(d);
    // The rows and Global Adjust unchanged since Save Changes: the group keeps its saved colors.
    let saved = unchanged_since_save(d);
    let assign = d.fields.get("__assign").and_then(Value::as_array).filter(|_| d.bool("__art")).map_or(&[][..], Vec::as_slice);
    let found: Vec<Option<(Value, &Value)>> = (0..assign.len()).map(|i| swatch_row(d, &rs, assign, i)).collect();
    let keep = json!("keep");
    let group = d.fields.get("group").and_then(Value::as_str).and_then(|g| {
        let name = d.str("groupName");
        let rename = (!name.trim().is_empty() && name != g).then_some(name);
        let colors: Vec<&Value> = if saved {
            // Only a new name changes the group: each swatch keeps its exact color.
            rename.as_ref()?;
            d.fields.get(SAVED_COLORS).and_then(Value::as_array).map(|a| a.iter().map(|_| &keep).collect())?
        } else {
            // A row made with New Row that holds no current colors gives the group no color; a color
            // added on the Edit tab (`added`) joins it after the others.
            let added = rs.iter().filter(|r| no_current(r) && is_added(r) && !excluded(r)).map(|r| &r["to"]).filter(|c| !c.is_null());
            // A swatch that keeps its color takes no Global Adjust.
            if found.iter().any(Option::is_some) {
                found
                    .iter()
                    .map(|f| match f {
                        Some((_, r)) if !excluded(r) => &r["to"],
                        _ => &keep,
                    })
                    .chain(added)
                    .collect()
            } else {
                let filled = by_place(&rs);
                let rest = assign.get(filled.len()..).unwrap_or_default();
                let took = filled.iter().map(|r| if excluded(r) { &keep } else { &r["to"] });
                took.chain(rest).chain(added).collect()
            }
        };
        Some(json!({"group": g, "groupColors": colors, "rename": rename}))
    });
    // A row edited with the sliders carries `model: "new"`: its art takes the mode they showed. The
    // Edit tab's own marks (the base color, colors tied to a marker) stay in the dialog.
    let map: Vec<Value> = rs.iter().map(without_marks).collect();
    let mut p = json!({"map": map, "method": d.str("method"), "limitTo": d.str("limitTo")});
    // The Global Adjust amounts, kept while the sliders show another mode; at 0% they are left out.
    let values = adjust_values(d);
    if values.iter().any(|v| *v != 0.0) {
        p["globalAdjust"] = Value::Object(ADJUSTS.iter().zip(values).map(|(k, v)| (k.to_string(), json!(v))).collect());
    }
    if let (Some(Value::Object(g)), Some(p)) = (group, p.as_object_mut()) {
        p.extend(g.into_iter().filter(|(_, v)| !v.is_null()));
    }
    p
}

/// The Find button's field, and the tint of its current color that the art of the rows that
/// aren't selected shows while it is on (VectorCraft's own amount: [`Color::tinted`]).
const FIND: &str = "findColors";
const FIND_DIM: f32 = 0.25;
/// The tints tried when a dimmed color is one a selected row lists: [`FIND_DIM`] and each lighter
/// one a percent apart, down to 1%.
const FIND_TRIES: usize = 25;
/// The colors of the art that the rows leave out (Preserve White, Black and Grays), as
/// `recolor.reduce` returned them.
const PRESERVED: &str = "__preserved";

/// A row's current color as `recolor.apply` matches it: an exact color (its key, parsed whatever
/// its case or spacing), or `Err` with the hex of a "#rrggbb" entry, which matches every color
/// shown as that hex.
type FindEntry = Result<ColorKey, String>;

fn find_entry(k: &Value) -> Option<FindEntry> {
    match k.as_str() {
        Some(s) => ColorKey::parse(s).map(Ok).or_else(|| color_value(k).map(|c| Err(c.to_hex()))),
        None => color_value(k).map(|c| Ok(ColorKey::of(&c))),
    }
}

/// The first of `rows` that lists the color entry `k`, as `recolor.apply` reads entries: rows that
/// list it as an exact color (by key, whatever its case or spacing) come before rows whose
/// "#rrggbb" entry matches it by its hex. Rows that give the color a new color (not excluded, `to`
/// not `null`) come first, since `recolor.apply` maps the color with them; an excluded row or one
/// that removes the color is found only when no such row lists it.
fn row_listing<'a>(rows: &'a [Value], k: &Value) -> Option<&'a Value> {
    let want = find_entry(k)?;
    let hex = |e: &FindEntry| e.as_ref().map_or_else(Clone::clone, |k| k.color().to_hex());
    let lists = |r: &Value, by_hex: bool| {
        r["from"].as_array().is_some_and(|f| {
            f.iter().filter_map(find_entry).any(|e| match (&e, &want) {
                (Ok(a), Ok(b)) => !by_hex && a == b,
                _ => by_hex && hex(&e) == hex(&want),
            })
        })
    };
    let find = |live: bool, by_hex: bool| rows.iter().filter(|r| !live || (!excluded(r) && !r["to"].is_null())).find(|r| lists(r, by_hex));
    find(true, false).or_else(|| find(true, true)).or_else(|| find(false, false)).or_else(|| find(false, true))
}

/// While Find is on, on the Assign tab with art and rows selected: what the art previews instead of
/// [`apply_params`], as `command.batch` parameters. A first `recolor.apply` gives each current color
/// of the rows that aren't selected, and each preserved color, its [`FIND_DIM`] tint, or the nearest lighter one when that is a
/// color a selected row lists (a color a selected row also lists stays);
/// with Preview on, a second recolors the selected rows as [`apply_params`] would (the other rows
/// excluded, no color group rewritten). None otherwise, and when there is nothing to dim with
/// Preview off. OK still applies [`apply_params`].
fn find_params(d: &Fields) -> Option<Value> {
    if !d.bool(FIND) || !d.bool("__art") || d.str("tab") == "edit" {
        return None;
    }
    let rs = rows(d);
    let sel: Vec<usize> = selection(d).into_iter().filter(|i| *i < rs.len()).collect();
    if sel.is_empty() {
        return None;
    }
    let keys = |r: &Value| r["from"].as_array().cloned().unwrap_or_default();
    let found: Vec<Value> = sel.iter().flat_map(|&i| keys(&rs[i])).collect();
    // The selected rows' entries as `recolor.apply` reads them: exact colors and "#rrggbb" ones.
    let found: Vec<FindEntry> = found.iter().filter_map(find_entry).collect();
    let found_keys: Vec<ColorKey> = found.iter().filter_map(|e| e.as_ref().ok().copied()).collect();
    let found_hex_entries: Vec<String> = found.iter().filter_map(|e| e.as_ref().err().cloned()).collect();
    let found_hex: Vec<String> = found.iter().map(|e| e.as_ref().map_or_else(Clone::clone, |k| k.color().to_hex())).collect();
    // A dimmed color must not be one the second step recolors (by its key, or by its hex as a
    // "#rrggbb" entry matches): the nearest lighter tint that is none of them is taken instead.
    let dimmed = |c: Color| {
        (0..FIND_TRIES)
            .map(|n| c.tinted(FIND_DIM - n as f32 * 0.01))
            .find(|t| !found_keys.contains(&ColorKey::of(t)) && !found_hex_entries.contains(&t.to_hex()))
            .map(|t| key(&t))
    };
    // The current colors of the rows that aren't selected, and the preserved colors.
    let preserved = d.fields.get(PRESERVED).and_then(Value::as_array).cloned().unwrap_or_default();
    let others = rs.iter().enumerate().filter(|(i, _)| !sel.contains(i)).flat_map(|(_, r)| keys(r)).chain(preserved);
    let mut dim = serde_json::Map::new();
    // A color a selected row lists stays, and so does one shown as the same hex when either entry is
    // "#rrggbb" (it matches both). Two keys of the same hex are separate colors.
    let listed = |k: &Value| match find_entry(k) {
        Some(Ok(key)) => found_keys.contains(&key) || found_hex_entries.contains(&key.color().to_hex()),
        Some(Err(hex)) => found_hex.contains(&hex),
        None => false,
    };
    for k in others.filter(|k| !listed(k)) {
        // A string entry keeps its own form ("#rrggbb" matches by hex); a color value goes by its key.
        let name = match k.as_str() {
            Some(s) => Some(s.to_string()),
            None => color_value(&k).map(|c| ColorKey::of(&c).to_string()),
        };
        if let (Some(to), Some(name)) = (color_value(&k).and_then(dimmed), name) {
            dim.insert(name, to);
        }
    }
    let mut commands = vec![];
    if !dim.is_empty() {
        commands.push(json!({"command": CMD, "params": {"map": dim}}));
    }
    if d.bool("preview") {
        let mut p = apply_params(d);
        let mut only = rs;
        for (_, r) in only.iter_mut().enumerate().filter(|(i, _)| !sel.contains(i)) {
            r["exclude"] = json!(true);
        }
        p["map"] = Value::Array(only);
        if let Some(p) = p.as_object_mut() {
            for k in ["group", "groupColors", "rename"] {
                p.remove(k);
            }
        }
        commands.push(json!({"command": CMD, "params": p}));
    }
    (!commands.is_empty()).then(|| json!({"commands": commands}))
}

/// The fields a step of Undo Changes and Redo Changes restores: what the art is recolored with.
const STEP_FIELDS: [&str; 11] =
    ["rows", "colors", "method", "preserveWhite", "preserveBlack", "preserveGrays", "limitTo", "rule", "globalAdjust", THEME_PICKS, WEIGHTS];
/// The fields that record which reduction, picks and weights the rows already took, restored with
/// [`STEP_FIELDS`] so that the restored rows are not reduced, picked or weighed again.
const STEP_MARKS: [&str; 4] = [REDUCED, THEME_DONE, WEIGHED, PRESERVED];
/// VectorCraft's own limit on the bytes of JSON the steps of Undo Changes keep; the oldest go first.
const MAX_STEP_BYTES: usize = 4 << 20;

type Snapshot = serde_json::Map<String, Value>;

/// The size of `s` as JSON, which [`MAX_STEP_BYTES`] limits.
fn snapshot_size(s: &Snapshot) -> usize {
    serde_json::to_vec(s).map_or(0, |b| b.len())
}

/// The fields of `d` a step restores ([`STEP_FIELDS`] and [`STEP_MARKS`]).
fn snapshot(d: &Fields) -> Snapshot {
    STEP_FIELDS.iter().chain(&STEP_MARKS).filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect()
}

/// Leave Undo Changes and Redo Changes no step to go to, after another color group is selected or
/// Save Changes rewrites the group: the dialog's steps start again from the fields `d` has then, as on
/// a new opening, so fields set before the next frame or step are a step back to them.
fn restart_history(d: &mut Fields) {
    d.fields.insert(RESTART.into(), json!(true));
}

/// Before the first step: when fields were set after the dialog opened (with `ui.dialog.set`, before
/// a frame read the art), the fields it `opened` with, as they read the art, are the step Undo
/// Changes goes back to. `None` when nothing was set.
fn seed(app: &mut VectorcraftApp, d: &Fields, opened: &Snapshot) -> Option<Snapshot> {
    if snapshot(d) == *opened {
        return None;
    }
    let mut first = d.clone();
    for k in STEP_FIELDS.iter().chain(&STEP_MARKS) {
        match opened.get(*k) {
            Some(v) => first.fields.insert(k.to_string(), v.clone()),
            None => first.fields.remove(*k),
        };
    }
    sync(app, &mut first);
    sync_theme(app, &mut first);
    Some(snapshot(&first))
}

/// The open dialog's own history of color edits: Undo Changes goes back one edit and Redo Changes
/// forward one, without touching the document's undo history. A change of [`STEP_FIELDS`] is one
/// step once the gesture that makes it has ended, so a drag is one step; a new edit after Undo
/// Changes ends what Redo Changes could restore.
struct Steps {
    history: EditHistory<Snapshot>,
    /// The fields after the latest step, which a change is told from.
    settled: Snapshot,
    /// The fields the dialog opened with (or had when its steps were dropped), until the pump has
    /// read the art and the history starts ([`Work::begin`]).
    opened: Option<Snapshot>,
}

impl Steps {
    /// Steps of a dialog that opened with the fields `opened` and has not read the art yet.
    fn opened(opened: Snapshot) -> Self {
        Steps { history: EditHistory::new(&opened), settled: opened.clone(), opened: Some(opened) }
    }

    /// The history from `base` on, with nothing to undo or redo.
    fn start(&mut self, base: Snapshot) {
        self.history = EditHistory::new(&base).with_budget(MAX_STEP_BYTES, snapshot_size);
        self.settled = base;
    }

    /// Takes `d`'s mark from [`restart_history`]: the steps start again from its fields.
    fn take_restart(&mut self, d: &mut Fields) {
        if d.fields.remove(RESTART).is_some() {
            self.opened = Some(snapshot(d));
        }
    }

    /// Records a change of `d`'s fields since the latest step as a step.
    fn note(&mut self, d: &Fields) {
        let now = snapshot(d);
        if now == self.settled {
            return;
        }
        let before = std::mem::replace(&mut self.settled, now.clone());
        self.history.record(&EditEvent::set(Mixed::Same(before), now, Arc::from(TITLE), None));
    }

    /// Whether `d`'s fields differ from those after the latest step.
    fn pending(&self, d: &Fields) -> bool {
        STEP_FIELDS.iter().any(|k| d.fields.get(*k) != self.settled.get(*k))
    }

    /// Whether Undo Changes (`back`) or Redo Changes has a step to go to.
    fn can(&self, d: &Fields, back: bool) -> bool {
        // Before the art is read, fields set since the dialog opened are a step back, except rows
        // and weights that the art's reduction is about to replace or drop ([`sync`]).
        if let Some(opened) = &self.opened {
            let reduces = d.bool("__art") && d.fields.get(REDUCED) != Some(&reduce_params(d));
            let kept = |k: &&&str| !(reduces && (**k == "rows" || **k == WEIGHTS));
            return back && STEP_FIELDS.iter().filter(kept).any(|k| d.fields.get(*k) != opened.get(*k));
        }
        if back { self.history.can_undo() || self.pending(d) } else { self.history.can_redo() && !self.pending(d) }
    }

    /// Undo Changes (`back`) or Redo Changes on `d`: give it the fields of the step before or after.
    /// Returns false when there is none.
    fn step(&mut self, d: &mut Fields, back: bool) -> bool {
        if self.opened.is_some() {
            return false;
        }
        self.note(d);
        let mut now = snapshot(d);
        if !self.history.step(&mut now, if back { Step::Undo } else { Step::Redo }) {
            return false;
        }
        for k in STEP_FIELDS.iter().chain(&STEP_MARKS) {
            match now.get(*k) {
                Some(v) => d.fields.insert(k.to_string(), v.clone()),
                None => d.fields.remove(*k),
            };
        }
        // The rows hold no slider edit or drag, and the selection keeps to them.
        for k in [COMPS, DRAG] {
            d.fields.remove(k);
        }
        let n = rows(d).len();
        d.fields.insert("__sel".into(), json!(selection(d).into_iter().filter(|i| *i < n).collect::<Vec<_>>()));
        self.settled = now;
        true
    }
}

/// What `show` asked for in the passes since the pump last ran, which needs the app.
#[derive(Default)]
struct Filed {
    /// Undo Changes (`true`) or Redo Changes.
    step: Option<bool>,
    /// Randomize: saturation and brightness (`true`) or color order.
    randomize: Option<bool>,
    /// Other Library…: a library file to import and limit to.
    library: bool,
    /// The row whose new color the Color Picker edits, with `true` from Pick from Swatches.
    picker: Option<(usize, bool)>,
    /// The rows before the passes edited them, which the edit snaps to the library from.
    edited: Option<Vec<Value>>,
    /// A gesture in the dialog was running after the latest pass: its change is no step yet.
    running: bool,
}

/// One row as the dialog shows it: the color its new color chip shows, the color its Tint rail runs
/// to, and the name of the library's spot color its new color is.
struct RowFeed {
    shown: Color,
    linked: Color,
    spot: Option<String>,
}

impl RowFeed {
    fn of(app: &VectorcraftApp, d: &Fields, row: &Value) -> RowFeed {
        let to = to_color(row);
        let spot = to.filter(|_| d.bool("__art")).and_then(|c| spot_of(app, d, &c));
        RowFeed { shown: new_shown(app, d, row), linked: linked(app, to.unwrap_or_default()), spot }
    }
}

/// What the dialog shows of the app, which the pump feeds it each frame: the dialog has no app of
/// its own.
struct Feed {
    /// Limit to Library's choices: the key `limitTo` takes and the name the menu shows.
    libraries: Vec<(String, String)>,
    /// The first colour library, which the Color Library preset limits to.
    first_library: Option<String>,
    /// The document's color groups with their colors, in the Swatches panel's order.
    groups: Vec<(String, Vec<Color>)>,
    /// The groups with a swatch that is not a color ([`other_paint_groups`]).
    other_paints: Vec<String>,
    /// The art previews the dialog's recolour.
    previewing: bool,
    /// Each row as the dialog shows it.
    rows: Vec<RowFeed>,
}

impl Feed {
    fn of(app: &VectorcraftApp, d: &Fields) -> Feed {
        let libraries = library_choices(app);
        Feed {
            // The colour libraries follow None and the document's swatches.
            first_library: libraries.get(2).map(|(k, _)| k.clone()),
            libraries,
            groups: doc_groups(app),
            other_paints: other_paint_groups(app),
            previewing: app.session.in_interaction(),
            rows: rows(d).iter().map(|r| RowFeed::of(app, d, r)).collect(),
        }
    }
}

/// What the pump works on with the app, out of the host: the values, their steps and what `show`
/// asked for.
struct Work {
    fields: Fields,
    steps: Steps,
    filed: Filed,
}

impl Work {
    /// Reads the art for fields set since the last frame, as the next frame would; the first time,
    /// the steps start ([`seed`]).
    fn begin(&mut self, app: &mut VectorcraftApp) {
        let baseline = self.steps.opened.take().map(|opened| seed(app, &self.fields, &opened));
        sync(app, &mut self.fields);
        sync_theme(app, &mut self.fields);
        sync_weights(app, &mut self.fields);
        if let Some(base) = baseline {
            self.steps.start(base.unwrap_or_else(|| snapshot(&self.fields)));
        }
    }

    /// One frame of the dialog's work: the group picked, the art read again, what the last passes
    /// asked for, and the change as a step once no gesture runs. Returns the Color Picker asked for.
    fn run(&mut self, app: &mut VectorcraftApp) -> Option<(usize, bool)> {
        let d = &mut self.fields;
        if let Err(e) = select_group(app, d) {
            app.status(e);
        }
        self.steps.take_restart(&mut self.fields);
        self.begin(app);
        let filed = std::mem::take(&mut self.filed);
        if let Err(e) = run_group_action(app, &mut self.fields) {
            app.status(e);
        }
        self.steps.take_restart(&mut self.fields);
        if let Some(back) = filed.step {
            self.steps.step(&mut self.fields, back);
        }
        if let Some(sb) = filed.randomize {
            randomize(app, &mut self.fields, sb);
        }
        if filed.library {
            pick_library(app);
        }
        let d = &mut self.fields;
        if let Some(before) = filed.edited {
            let mut rs = rows(d);
            snap(app, d, &mut rs);
            keep_tints(&before, &mut rs);
            d.fields.insert("rows".into(), Value::Array(rs));
        }
        sync_weights(app, d);
        if let Some(which) = d.fields.remove(SAVE_GROUP)
            && let Err(e) = save_group(app, d, which.as_str().unwrap_or_default())
        {
            app.status(e);
        }
        sync(app, d);
        // New rows take the picks now, so that a new count of Colors is one step.
        sync_theme(app, d);
        // A change is a step once the gesture that makes it has ended: a drag is one step from press
        // to release, and a change `ui.dialog.set` made is one at once.
        if self.steps.opened.is_none() && !filed.running {
            self.steps.note(&self.fields);
        }
        filed.picker
    }
}

/// Randomly change the rows' color order, or (`sb`) their saturation and brightness
/// (`recolor.randomize`), from the next seed. A shuffle moves each tint with its color.
fn randomize(app: &mut VectorcraftApp, d: &mut Fields, sb: bool) {
    let seed = next_seed(d);
    d.fields.insert("__seed".into(), json!(seed));
    let rs = rows(d);
    let p = json!({"map": rs, "order": !sb, "saturationBrightness": sb, "seed": seed});
    match app.session.execute("recolor.randomize", &p) {
        Ok(r) => {
            let mut shuffled = r["map"].as_array().cloned().unwrap_or_default();
            keep_marks(&rs, &mut shuffled);
            snap(app, d, &mut shuffled);
            d.fields.insert("rows".into(), Value::Array(shuffled));
        }
        Err(e) => app.status(e.to_string()),
    }
}

/// Other Library…: import a library (.ase, .acb, .gpl, .vcswatches or a document's swatches) and
/// limit to it.
fn pick_library(app: &mut VectorcraftApp) {
    let request = builtin_open_filters().fold(PickRequest::open("library"), |request, (name, exts)| request.filter(name, exts));
    // PLAN: assumed `file_asks::Owner::Free` (fl.A3's owner of a continuation no document holds), as appearance-w19 codes it.
    crate::file_asks::ask(app, request, crate::file_asks::Owner::Free, |app, picked| {
        let Some(path) = picked.first_path() else { return Ok(Value::Null) };
        let r = app.run("swatch.library.load", json!({ "path": path }))?;
        let library = r["library"].clone();
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| {
                if let Some(d) = host.get_mut::<RecolorDialog>() {
                    d.fields.fields.insert("limitTo".into(), library);
                }
            });
        }
        Ok(r)
    });
}

/// What the art previews of `d`, with art: Find's `command.batch` while Find shows the art so
/// (with Preview off too), else `recolor.apply` while Preview is on.
fn preview_params(d: &Fields) -> Option<(bool, &'static str, Value)> {
    if !d.bool("__art") {
        return None;
    }
    Some(match find_params(d) {
        Some(p) => (true, "command.batch", p),
        None => (d.bool("preview"), CMD, apply_params(d)),
    })
}

/// Runs the open dialog's live preview with `(on, command, params)`.
fn run_preview(app: &mut VectorcraftApp, (on, cmd, params): (bool, &'static str, Value)) {
    let Some(mut live) = app.color_dialogs.recolor.as_mut().map(|open| std::mem::take(&mut open.preview)) else { return };
    // `run` reports a preview that fails on the status line itself.
    let _ = live.run(app, on, TITLE, cmd, &params);
    if let Some(open) = app.color_dialogs.recolor.as_mut() {
        open.preview = live;
    }
}

/// Feeds the open Recolor Artwork what it shows of the app, runs what its passes asked for, asks for
/// the Color Picker over a row's new colour, gives the row the colour that picker answered and
/// previews its recolour: once a frame, after the window's dialog host ran.
// PLAN: hub-w2 adds the line `recolor::pump(app, ctx)` to `dialogs::host::pump` (co.A6 names this pump).
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(id) = app.color_dialogs.recolor.as_ref().map(|open| open.id) else { return };
    let Some(dialogs) = app.dialogs() else { return };
    // The colour the picker answered for a row reaches the row here, where the app can snap it to Limit to Library.
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<RecolorDialog>(id)
            && let Some((row, c)) = d.picked.take()
        {
            // A colour picked as RGB keeps the model of the row's new colour (CMYK in a CMYK document).
            let picked = Color::rgb(c.r, c.g, c.b);
            let was = rows(&d.fields).get(row).and_then(to_color);
            with_new_color(app, d, row, was.map_or(picked, |was| keep_model(was, picked)));
        }
    });
    let Some(mut work) = dialogs.with(|host| host.get_mut_by_id::<RecolorDialog>(id).map(RecolorDialog::take_work)) else { return };
    // Pick from Swatches opens the picker on its swatches view.
    let request = work.run(app).and_then(|(row, swatches)| {
        let r = rows(&work.fields).into_iter().nth(row)?;
        let color = to_color(&r).or_else(|| from_colors(&r).first().copied()).unwrap_or_default();
        let request = ColorRequest::new(row.to_string(), srgba(&color));
        Some(if swatches { request.on_swatches() } else { request })
    });
    // The preview runs first, so the feed lists what it shows (the group it renames among them).
    if let Some(p) = preview_params(&work.fields) {
        run_preview(app, p);
    }
    let feed = Feed::of(app, &work.fields);
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<RecolorDialog>(id) {
            d.put_work(work, feed);
            d.color_request = request;
        }
    });
}

/// Undo Changes (`ui.recolorUndo`, `back`) or Redo Changes (`ui.recolorRedo`) in the open Recolor
/// Artwork dialog → {changed, canUndo, canRedo}.
pub(crate) fn step(app: &mut VectorcraftApp, back: bool) -> Result<Value, String> {
    const NOT_OPEN: &str = "Recolor Artwork is not open";
    let id = app.color_dialogs.recolor.as_ref().map(|open| open.id).ok_or(NOT_OPEN)?;
    let dialogs = app.dialogs().ok_or(NOT_OPEN)?;
    let mut work = dialogs.with(|host| host.get_mut_by_id::<RecolorDialog>(id).map(RecolorDialog::take_work)).ok_or(NOT_OPEN)?;
    // Fields set since the last frame take effect first, as the next frame would give them.
    work.begin(app);
    let changed = work.steps.step(&mut work.fields, back);
    if changed {
        sync_weights(app, &mut work.fields);
    }
    let answer = json!({"changed": changed, "canUndo": work.steps.can(&work.fields, true), "canRedo": work.steps.can(&work.fields, false)});
    // The art previews the step at once, as the frame after a button would preview it.
    if let Some(p) = preview_params(&work.fields).filter(|_| changed) {
        run_preview(app, p);
    }
    let feed = Feed::of(app, &work.fields);
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<RecolorDialog>(id) {
            d.put_work(work, feed);
        }
    });
    Ok(answer)
}

/// Whether Undo Changes (`back`) or Redo Changes can run: Recolor Artwork is open with a step to go to.
pub(crate) fn can_step(app: &VectorcraftApp, back: bool) -> bool {
    app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.get::<RecolorDialog>().is_some_and(|d| d.steps.can(&d.fields, back))))
}

/// Row `i`'s new colour set to `c` (the Color Picker's OK), snapped to Limit to Library.
pub(super) fn with_new_color(app: &VectorcraftApp, d: &mut RecolorDialog, i: usize, c: Color) {
    let before = rows(&d.fields);
    let mut rs = before.clone();
    if let Some(r) = rs.get_mut(i) {
        set_to(r, &c);
        snap(app, &d.fields, &mut rs);
        keep_tints(&before, &mut rs);
        d.fields.fields.insert("rows".into(), Value::Array(rs));
    }
}

/// OK: keep the previewed recolour as one undo step (or apply it, e.g. to a colour group only; or
/// save colours opened alone as a new colour group). The dialog closes once this answers `Ok`; an
/// `Err` keeps it open with the reason.
fn confirm(app: &mut VectorcraftApp, fields: &Fields) -> Result<(), String> {
    let mut d = fields.clone();
    select_group(app, &mut d)?;
    sync(app, &mut d);
    sync_theme(app, &mut d);
    sync_weights(app, &mut d);
    // A Color Groups button set with OK in one request runs first, once: the open dialog takes
    // its result, so an OK that fails afterwards does not run it again.
    if d.fields.contains_key(GROUP_ACTION) {
        let r = run_group_action(app, &mut d);
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| {
                if let Some(live) = host.get_mut::<RecolorDialog>() {
                    live.take_fields(d.clone());
                }
            });
        }
        r?;
    }
    let preview = app.color_dialogs.recolor.as_mut().map(|open| std::mem::take(&mut open.preview)).unwrap_or_default();
    let committed = if d.bool("__newGroup") {
        let colors = group_colors(app, &d, rows(&d).iter().filter(|r| in_group(r) && to_color(r).is_some()));
        preview.commit(app, "swatch.newGroup", json!({"name": d.str("groupName"), "colors": colors, "keepModel": true}))
    } else {
        preview.commit(app, CMD, apply_params(&d))
    };
    committed?;
    app.color_dialogs.recolor = None;
    // The Preserve checkboxes (shown with art) open as confirmed here next time (#2960).
    if d.bool("__art") {
        app.ui.recolor_preserve =
            json!({"preserveWhite": d.bool("preserveWhite"), "preserveBlack": d.bool("preserveBlack"), "preserveGrays": d.bool("preserveGrays")});
    }
    Ok(())
}

/// The Recolor Artwork dialog on the window's dialog host: the toolbar, the Assign or Edit tab, the
/// Color Groups list, the group's name, Preview and OK and Cancel. It edits its [`Fields`]; what needs
/// the app it files for [`pump`], which feeds it the app's state. Its OK is held until [`confirm`]
/// commits the recolour.
pub(crate) struct RecolorDialog {
    id: DialogId,
    fields: Fields,
    steps: Steps,
    held: HeldAnswer<Fields>,
    feed: Feed,
    filed: Filed,
    /// The lead and anchor of the rows table's selection, whose items are `__sel`: a Shift-click
    /// range runs from the anchor.
    ends: (Option<usize>, Option<usize>),
    /// The row whose right-click menu is open, and where it opened.
    row_menu: Option<(usize, egui::Pos2)>,
    /// The Color Picker asked for over a row's new colour, keyed by the row's index, until the picker opens.
    color_request: Option<ColorRequest>,
    /// The row and colour the picker answered, until the pump gives the row that colour.
    picked: Option<(usize, Srgba)>,
    /// The Color Theme Picker's canvas tool, which the canvas runs while the picker is on.
    theme_tool: edit_tab::ThemePickTool,
}

impl RecolorDialog {
    fn new(app: &VectorcraftApp, fields: Fields) -> Self {
        RecolorDialog {
            id: dialog_id(KIND),
            feed: Feed::of(app, &fields),
            steps: Steps::opened(snapshot(&fields)),
            fields,
            held: HeldAnswer::new(),
            filed: Filed::default(),
            ends: (None, None),
            row_menu: None,
            color_request: None,
            picked: None,
            theme_tool: edit_tab::ThemePickTool::default(),
        }
    }

    /// The values, steps and requests, for the pump to work on with the app.
    fn take_work(&mut self) -> Work {
        Work {
            fields: std::mem::take(&mut self.fields),
            steps: std::mem::replace(&mut self.steps, Steps::opened(Snapshot::new())),
            filed: std::mem::take(&mut self.filed),
        }
    }

    /// The pump's work back, with what the dialog shows of the app now.
    fn put_work(&mut self, work: Work, feed: Feed) {
        self.fields = work.fields;
        self.steps = work.steps;
        self.filed = work.filed;
        self.feed = feed;
    }

    /// Fields that OK's Color Groups button left, which the dialog keeps.
    fn take_fields(&mut self, fields: Fields) {
        self.fields = fields;
        self.steps.take_restart(&mut self.fields);
    }

    /// `ui.dialog.set`: a field the dialog holds, `preview`, `groupAction` or one of the
    /// [`SETTABLE`] controls, as the dialog's own control sets it. A name no color group has, a
    /// Color Groups button that can't run ([`check_group_action`]) and a `groupStorage` that is not a
    /// bool are refused, and the dialog stays as it was.
    fn set_field(&mut self, field: &str, value: Value) -> Result<(), String> {
        let known =
            if field.starts_with("__") { SETTABLE.contains(&field) } else { self.fields.fields.contains_key(field) || UNSET_FIELDS.contains(&field) };
        if !known {
            return Err(FieldRefusal::Unknown.message(field));
        }
        match field {
            "group" => check_group(&value, |g| self.feed.groups.iter().any(|(n, _)| n == g))?,
            GROUP_ACTION => check_group_action(&self.fields, &value, |g| self.feed.other_paints.iter().any(|n| n == g))?,
            GROUP_STORAGE if !value.is_boolean() => return Err(format!("`{GROUP_STORAGE}` must be true or false")),
            _ => {}
        }
        self.fields.fields.insert(field.to_owned(), value);
        Ok(())
    }

    fn shown(&mut self, ui: &mut Ui) -> Option<DialogResult<Fields>> {
        let measuring = craft::ui::runtime::measure::active(ui.ctx());
        let finish = self.held.is_accepted();
        // The refusal of OK shows until a value changes.
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.fields) {
            self.held.clear_refusal();
        }
        let refusal = self.held.refusal().map(|(reason, _)| reason.clone());
        let waiting = self.held.is_waiting();
        let can = (self.steps.can(&self.fields, true), self.steps.can(&self.fields, false));
        let art = self.fields.bool("__art");
        let mut preview = self.fields.bool("preview");
        let footer = ResetFooter::new(ui, None, true);
        let RecolorDialog { fields, feed, filed, ends, row_menu, .. } = self;
        let mut form = FormDialog::new(tl!(TITLE), Bind::bound(fields)).accept_stays_open(true).ok_enabled(!waiting);
        if art {
            form = form.preview(Some(Bind::bound(&mut preview)));
        }
        let mut running = false;
        let out = form.show(ui, |ui, cx, d| {
            // A measuring run closes nothing: the real run of the same pass makes the close.
            if finish && !measuring {
                cx.request_close_as(CloseReason::Activated);
            }
            let ctx = ui.ctx().clone();
            let ((), gesture) = craft::ui::runtime::edit::watch_gestures(&ctx, || body(ui, d, feed, filed, ends, row_menu, can));
            if !measuring {
                running = gesture;
            }
            if let Some(reason) = &refusal {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            true
        });
        if !measuring {
            self.filed.running = running;
        }
        if art && preview != self.fields.bool("preview") {
            self.fields.fields.insert("preview".into(), json!(preview));
        }
        match out.closed {
            Some(CloseReason::Activated) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            Some(_) => Some(DialogResult::Cancel),
            None => {
                // OK files the fields for the app, which commits the recolour before the dialog closes.
                if footer.pick(out.picked) == Some(ResetPick::Accept) && !self.held.is_waiting() {
                    self.held.file(self.fields.clone());
                }
                None
            }
        }
    }
}

impl StackDialog for RecolorDialog {
    type Result = Fields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Fields>> {
        self.shown(ui)
    }

    /// OK as its button gives it: the fields are filed for the app, which answers none until it has
    /// committed the recolour; then the accepted fields.
    fn confirm(&mut self, _: &egui::Context) -> Option<Fields> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if !self.held.is_waiting() {
            self.held.file(self.fields.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.color_request.take()
    }

    /// The picker's OK for the row its request named.
    fn answer_color(&mut self, key: &str, color: Srgba) {
        if let Ok(row) = key.parse() {
            self.picked = Some((row, color));
        }
    }

    // PLAN: color-w20 asked for `canvas()` to return the main canvas while the Color Theme Picker is on; no main
    // canvas id exists in the app to return (canvas-w20 owns the tool hook, `edit_tab::with_theme_pick`).

    /// The fields without the dialog's own `__` marks.
    fn json_fields(&self) -> Option<Value> {
        Some(Value::Object(self.fields.fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect()))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        self.set_field(field, value)
    }
}

impl HoldsOk for RecolorDialog {
    type Answer = Fields;

    /// Recolor Artwork always holds its OK.
    fn held_ok(self) -> Self {
        self
    }

    fn take_ok(&mut self) -> Option<Fields> {
        self.held.take()
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

/// The dialog's body: the toolbar, the tab shown with the Color Groups list beside it (under it
/// when the room is too narrow for both), and the group's name.
fn body(
    ui: &mut Ui,
    d: &mut Fields,
    feed: &Feed,
    filed: &mut Filed,
    ends: &mut (Option<usize>, Option<usize>),
    row_menu: &mut Option<(usize, egui::Pos2)>,
    can: (bool, bool),
) {
    toolbar(ui, d, feed, filed, can);
    if d.bool(THEME_PICKER) {
        Text::new(tl!("Click or drag across the art to pick its colors; Shift adds more")).shade(Shade::Muted).show(ui);
    }
    let before = rows(d);
    let mut rs = before.clone();
    let mut changed = false;
    Row::new().wrap(true).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            if d.str("tab") == "edit" {
                let (edited, pick) = edit_tab::show(ui, d, feed, &mut rs);
                changed = edited;
                if pick.is_some() {
                    filed.picker = pick;
                }
            } else {
                changed = assign_tab(ui, d, feed, filed, ends, row_menu, &mut rs);
            }
        });
        if d.bool(GROUP_STORAGE) {
            group_list(ui, d, feed);
        }
    });
    if changed {
        d.fields.insert("rows".into(), Value::Array(rs));
        filed.edited.get_or_insert(before);
    }
    if d.fields.get("group").is_some_and(Value::is_string) || d.bool("__newGroup") {
        let mut name = d.str("groupName");
        TextField::new(tl!("Color Group:"), Bind::bound(&mut name)).show(ui);
        if name != d.str("groupName") {
            d.fields.insert("groupName".into(), json!(name));
        }
    }
}

/// An icon-only toggle named `label` over the bool field `key` of `d`.
fn toggle_field(ui: &mut Ui, d: &mut Fields, key: &str, label: &'static str, icon: &'static str, enabled: bool) {
    let mut on = d.bool(key);
    ToggleButton::new(label, Bind::bound(&mut on)).icon(IconId::new(icon)).icon_only().enabled(enabled).show(ui);
    if on != d.bool(key) {
        d.fields.insert(key.into(), json!(on));
    }
}

/// The toolbar: the Assign and Edit tabs, the Color Theme Picker, Undo Changes and Redo Changes
/// (`can` says whether each has a step), Show Color Groups and the preset menu.
fn toolbar(ui: &mut Ui, d: &mut Fields, feed: &Feed, filed: &mut Filed, (can_undo, can_redo): (bool, bool)) {
    Row::new().show(ui, |ui, _| {
        let titles = [tl!("Assign").to_owned(), tl!("Edit").to_owned()];
        let tabs = [0usize, 1];
        let source = keyed(&tabs, |k| *k).with_text(|k| Cow::Owned(titles.get(*k).cloned().unwrap_or_default()));
        let shown = usize::from(d.str("tab") == "edit");
        let strip = TabStrip::new(tl!(TITLE))
            .kind(TabKind::Text)
            .shown(shown)
            .show_items(ui, &source, &mut |key, _| Tab::new(titles.get(*key).cloned().unwrap_or_default()));
        for event in strip.response.events {
            if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                && let Some(&k) = keys.last()
            {
                d.fields.insert("tab".into(), json!(if k == 1 { "edit" } else { "assign" }));
            }
        }
        toggle_field(ui, d, THEME_PICKER, tl!("Color Theme Picker"), "pipette", true);
        if IconButton::new(IconId::new("undo-2")).label(tl!("Undo Changes")).enabled(can_undo).show(ui).clicked() {
            filed.step = Some(true);
        }
        if IconButton::new(IconId::new("redo-2")).label(tl!("Redo Changes")).enabled(can_redo).show(ui).clicked() {
            filed.step = Some(false);
        }
        toggle_field(ui, d, GROUP_STORAGE, tl!("Show Color Groups"), "library", true);
        let presets: Vec<Entry<usize>> = PRESETS.iter().enumerate().map(|(i, p)| Entry::new(i, Label::from(*p))).collect();
        if let Some(&i) = Select::new(tl!("Preset:"), Bind::given(preset_of(d)), &presets).show(ui).value() {
            set_preset(d, i, feed.first_library.as_deref());
        }
    });
}

/// The preset the fields match: an n-colour job (n rows, Scale Tints), a library, or Custom.
fn preset_of(d: &Fields) -> usize {
    match d.fields.get("colors").and_then(Value::as_u64) {
        Some(n @ 1..=3) if d.str("method") == Method::ScaleTints.id() => n as usize,
        _ if !d.str("limitTo").is_empty() => 4,
        _ => 0,
    }
}

/// Set up preset `i`: an n-colour job, or (without a library yet) `first_library`.
fn set_preset(d: &mut Fields, i: usize, first_library: Option<&str>) {
    match i {
        1..=3 => {
            d.fields.insert("colors".into(), json!(i));
            d.fields.insert("method".into(), json!(Method::ScaleTints.id()));
        }
        4 if d.str("limitTo").is_empty() => {
            d.fields.insert("limitTo".into(), json!(first_library.unwrap_or_default()));
        }
        _ => {}
    }
}

/// `c` as the hex field's value.
fn hex_rgba(c: &Color) -> Rgba {
    Rgba::from_hex(&c.to_hex()).unwrap_or(Rgba::rgb(0, 0, 0))
}

/// The colors side by side across `rect`, each an equal part of it.
fn chips(rect: egui::Rect, colors: &[egui::Color32]) -> Vec<egui::Shape> {
    let w = rect.width() / colors.len().max(1) as f32;
    colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            egui::Shape::rect_filled(egui::Rect::from_min_size(rect.min + egui::vec2(i as f32 * w, 0.0), egui::vec2(w, rect.height())), 0.0, *c)
        })
        .collect()
}

/// An edit a row of the Assign tab's table made.
enum RowEdit {
    /// The arrow: excluded (`true`) or recolored.
    Exclude(usize, bool),
    /// The hex field's new color.
    Hex(usize, Rgba),
    /// A double-click on the new color: the Color Picker.
    Pick(usize),
}

/// The Assign tab. Returns true when the rows changed; the Color Picker a double-click or Pick from
/// Swatches asks for, Randomize and Other Library… are filed for the pump.
fn assign_tab(
    ui: &mut Ui,
    d: &mut Fields,
    feed: &Feed,
    filed: &mut Filed,
    ends: &mut (Option<usize>, Option<usize>),
    row_menu: &mut Option<(usize, egui::Pos2)>,
    rows: &mut Vec<Value>,
) -> bool {
    let mut changed = false;
    let art = d.bool("__art");
    let total: usize = rows.iter().map(|r| r["from"].as_array().map_or(0, Vec::len)).sum();
    // Reduction and the preserve rules apply to art (a colour group alone has a row per colour).
    if art {
        Row::new().show(ui, |ui, _| {
            let counts: Vec<Entry<usize>> = std::iter::once(Entry::new(0, Label::from(tl!("Auto"))))
                .chain((1..=total).map(|n| Entry::new(n, Label::from(n.to_string()))))
                .collect();
            let current = d.fields.get("colors").and_then(Value::as_u64).map_or(0, |n| n as usize);
            if let Some(&n) = Select::new(tl!("Colors:"), Bind::given(current), &counts).show(ui).value() {
                d.fields.insert("colors".into(), if n == 0 { Value::Null } else { json!(n) });
            }
            Text::new(tl!("Preserve:")).shade(Shade::Muted).show(ui);
            for (k, label) in [("preserveWhite", tl!("White")), ("preserveBlack", tl!("Black")), ("preserveGrays", tl!("Grays"))] {
                let mut on = d.bool(k);
                Checkbox::new(Bind::bound(&mut on)).label(label).show(ui);
                if on != d.bool(k) {
                    d.fields.insert(k.into(), json!(on));
                }
            }
        });
    }
    let columns = vec![
        ColumnSpec::new(
            CURRENT,
            Label::from(crate::i18n::fmt(tl!("Current Colors ({total})"), &[("total", &total.to_string())])),
            ColumnWidth::fill(1.0),
        ),
        ColumnSpec::new(ARROW, Label::from(""), ColumnWidth::Content),
        ColumnSpec::new(NEW, Label::from(tl!("New")), ColumnWidth::Content),
        ColumnSpec::new(HEX, Label::from(tl!("Hex")), ColumnWidth::Content),
    ];
    // A row is named by its current colors.
    let names: Vec<String> = rows
        .iter()
        .map(|r| {
            let hex: Vec<String> = from_colors(r).iter().map(Color::to_hex).collect();
            if hex.is_empty() { tl!("New Row").to_owned() } else { hex.join(", ") }
        })
        .collect();
    // The table's selection is the rows' `__sel`; its lead and anchor stay with the dialog, so a
    // Shift-click range runs from the anchor. An end that is no longer selected goes.
    let items = selection(d);
    let end = |e: Option<usize>| e.filter(|i| items.contains(i));
    let mut sel = Selection::new(items.clone(), end(ends.0), end(ends.1));
    let keys: Vec<usize> = (0..rows.len()).collect();
    let source = keyed(&keys, |k| *k).with_text(|k| Cow::Owned(names.get(*k).cloned().unwrap_or_default()));
    let edits: RefCell<Vec<RowEdit>> = RefCell::new(Vec::new());
    let shown_rows: &[Value] = rows;
    let out = Table::<usize>::new(tl!("Colors"), columns)
        .min_rows(5)
        .max_rows(8)
        .empty(tl!("No colors to recolor (preserved colors are left out)."))
        .selection(Bind::bound(&mut sel))
        .show_items(ui, &source, &mut |key, icx| {
            let i = *key;
            let edits = &edits;
            let row = shown_rows.get(i).cloned().unwrap_or_default();
            let off = excluded(&row);
            let from: Vec<egui::Color32> = from_colors(&row).iter().map(|c| if off { c32(c).gamma_multiply(0.35) } else { c32(c) }).collect();
            let to = to_color(&row);
            let fill = to.map(|c| srgba(&feed.rows.get(i).map_or(c, |f| f.shown)));
            let arrow =
                ToggleButton::new(tl!("Recolor this row"), Bind::given(!off)).icon(IconId::new(ARROW_OFF)).on_icon(IconId::new(ARROW_ON)).icon_only();
            let swatch = Swatch::new(format!("{} {}", tl!("New"), i + 1), fill.map_or(Mixed::Mixed, Mixed::Same)).enabled(!off);
            let hex = HexField::hex(format!("{} {}", tl!("Color"), i + 1), Bind::given(to.map_or(Rgba::rgb(0, 0, 0), |c| hex_rgba(&c))))
                .enabled(to.is_some());
            // PLAN: the current colors are the row's thumbnail strip, not a `SwatchTable` cell: a cell field takes its
            // own presses, so a click on them would not select the row as it does in production.
            ListRow::new(names.get(i).cloned().unwrap_or_default())
                .cells(icx.cells())
                .thumb_aspect(ThumbSize::Small, ThumbAspect::new(6, 1), move |cx, rect| cx.data_shapes(chips(rect, &from)))
                .cell_field(ARROW, arrow, move |out| {
                    if let Some(&on) = out.value() {
                        edits.borrow_mut().push(RowEdit::Exclude(i, !on));
                    }
                })
                .cell_field(NEW, swatch, move |out| {
                    // A double-click is the first pass's input: a second pass of the frame reports none.
                    if out.response.double_clicked() && to.is_some() {
                        edits.borrow_mut().push(RowEdit::Pick(i));
                    }
                })
                .cell_field(HEX, hex, move |out| {
                    if let Some(c) = out.events.committed() {
                        edits.borrow_mut().push(RowEdit::Hex(i, *c));
                    }
                })
        });
    let mut items = sel.items.clone();
    items.sort_unstable();
    items.dedup();
    let end = |e: Option<usize>| e.filter(|i| items.contains(i));
    *ends = (end(sel.lead), end(sel.anchor));
    if items != selection(d) {
        d.fields.insert("__sel".into(), json!(items));
    }
    for edit in edits.into_inner() {
        match edit {
            RowEdit::Exclude(i, off) => {
                if let Some(r) = rows.get_mut(i) {
                    r["exclude"] = json!(off);
                    changed = true;
                }
            }
            RowEdit::Hex(i, c) => {
                if let (Some(r), Some(new)) = (rows.get_mut(i), Color::from_hex(&c.to_hex())) {
                    let to = to_color(r).unwrap_or_default();
                    set_to(r, &keep_model(to, new));
                    changed = true;
                }
            }
            RowEdit::Pick(i) => filed.picker = Some((i, false)),
        }
    }
    for event in out.events {
        if let CollectionEvent::ContextMenu(request) = event {
            *row_menu = Some((request.key, request.at));
        }
    }
    // A row's right-click menu: Pick from Swatches, and Limit to Library's choices.
    if let Some((i, at)) = *row_menu {
        let mut open = true;
        let mut limit_to = None;
        let current = d.str("limitTo");
        let menu = Menu::new(tl!("Colors")).opened_by(&out.response, Trigger::Manual).anchor(Anchor::Point(at)).open(Bind::bound(&mut open)).show(
            ui,
            |ui, _| {
                if MenuItem::new(tl!("Pick from Swatches")).show(ui).ran {
                    filed.picker = Some((i, true));
                }
                Menu::submenu_row(ui, tl!("Limit to Library"), |ui, _| {
                    for (key, name) in &feed.libraries {
                        if MenuItem::new(name.clone()).check(*key == current).show(ui).ran {
                            limit_to = Some(key.clone());
                        }
                    }
                });
            },
        );
        if menu.output.closed.is_some() || !open {
            *row_menu = None;
        }
        if let Some(k) = limit_to {
            d.fields.insert("limitTo".into(), json!(k));
            changed = true;
        }
    }
    let sel = selection(d);
    Row::new().show(ui, |ui, _| {
        // New Row is available only with art.
        if IconButton::new(IconId::new("plus")).label(tl!("New Row")).enabled(art).show(ui).clicks == 1 {
            let at = new_row(rows, &sel);
            d.fields.insert("__sel".into(), json!([at]));
            // The slider edit was kept for a row index that now names another row.
            d.fields.remove(COMPS);
            changed = true;
        }
        if IconButton::new(IconId::new("combine")).label(tl!("Merge colors into one row")).enabled(sel.len() > 1).show(ui).clicks == 1 {
            merge(rows, &sel);
            d.fields.insert("__sel".into(), json!([sel.iter().min()]));
            changed = true;
        }
        let can_split = sel.iter().any(|i| rows.get(*i).is_some_and(|r| r["from"].as_array().is_some_and(|a| a.len() > 1)));
        if IconButton::new(IconId::new("ungroup")).label(tl!("Separate colors into rows")).enabled(can_split).show(ui).clicks == 1 {
            separate(rows, &sel);
            d.fields.insert("__sel".into(), json!([]));
            changed = true;
        }
        for (icon, label, sb) in
            [("arrow-left-right", tl!("Randomly change color order"), false), ("sparkles", tl!("Randomly change saturation and brightness"), true)]
        {
            if IconButton::new(IconId::new(icon)).label(label).enabled(rows.len() > 1 || sb).show(ui).clicked() {
                filed.randomize = Some(sb);
            }
        }
        toggle_field(ui, d, FIND, tl!("Find the selected colors in the artwork"), "search", art);
        if IconButton::new(IconId::new("rotate-ccw")).label(tl!("Reset the rows to the artwork's colors")).show(ui).clicks == 1 {
            d.fields.remove(REDUCED);
            d.fields.remove(COMPS);
            // The rows keep no slider edit, also when they cannot be reduced again.
            for r in rows.iter_mut().filter_map(Value::as_object_mut) {
                r.remove("model");
            }
            d.fields.insert("rows".into(), Value::Array(rows.clone()));
        }
        let target = SliderTarget::at(feed, d, rows, first_selected(d, rows));
        mode_menu(ui, d, &target, false);
    });
    let row = first_selected(d, rows);
    changed |= sliders(ui, feed, d, rows, row);
    Row::new().show(ui, |ui, _| {
        let methods: Vec<Entry<usize>> = Method::ALL.iter().enumerate().map(|(i, m)| Entry::new(i, Label::from(m.label()))).collect();
        let method = Method::parse(&d.str("method")).unwrap_or_default();
        let at = Method::ALL.iter().position(|m| m.id() == method.id()).unwrap_or(0);
        if let Some(&i) = Select::new(tl!("Method:"), Bind::given(at), &methods).show(ui).value()
            && let Some(m) = Method::ALL.get(i)
        {
            d.fields.insert("method".into(), json!(m.id()));
        }
    });
    Row::new().show(ui, |ui, _| {
        let libraries: Vec<Entry<usize>> = feed.libraries.iter().enumerate().map(|(i, (_, name))| Entry::new(i, Label::from(name.clone()))).collect();
        let current = d.str("limitTo");
        let at = feed.libraries.iter().position(|(k, _)| *k == current).unwrap_or(0);
        if let Some(&i) = Select::new(tl!("Limit to Library:"), Bind::given(at), &libraries).show(ui).value()
            && let Some((k, _)) = feed.libraries.get(i)
        {
            d.fields.insert("limitTo".into(), json!(k));
        }
        if IconButton::new(IconId::new("folder-open")).label(tl!("Other Library…")).show(ui).clicks == 1 {
            filed.library = true;
        }
    });
    changed
}

/// The Color Groups list: a row per color group of the document with its name and colors (a click
/// selects it, as the `group` field does), and Save Changes, New Color Group and Delete under it,
/// which set `groupAction`.
fn group_list(ui: &mut Ui, d: &mut Fields, feed: &Feed) {
    let names: Vec<String> = feed.groups.iter().map(|(n, _)| n.clone()).collect();
    let mut current = d.fields.get("group").and_then(Value::as_str).map(str::to_owned);
    // While the art previews, the group being edited shows under its new name.
    if current.as_ref().is_some_and(|c| feed.previewing && !names.contains(c)) {
        current = Some(d.str("groupName"));
    }
    let mut chosen = current.as_ref().and_then(|c| names.iter().position(|n| n == c)).map_or_else(Selection::default, Selection::single);
    let keys: Vec<usize> = (0..names.len()).collect();
    let mut picked: Option<Value> = None;
    Stack::new().show(ui, |ui, _| {
        let source = keyed(&keys, |k| *k).with_text(|k| Cow::Owned(names.get(*k).cloned().unwrap_or_default()));
        let out = List::new(tl!("Color Groups")).select(SelectMode::Single).max_rows(GROUP_ROWS).selection(Bind::bound(&mut chosen)).show_items(
            ui,
            &source,
            &mut |key, _| {
                let colors: Vec<egui::Color32> = feed.groups.get(*key).map(|(_, cs)| cs.iter().map(c32).collect()).unwrap_or_default();
                ListRow::new(names.get(*key).cloned().unwrap_or_default())
                    .thumb_aspect(ThumbSize::Small, ThumbAspect::new(4, 1), move |cx, rect| cx.data_shapes(chips(rect, &colors)))
            },
        );
        for event in out.events {
            if let CollectionEvent::Selection { change, .. } = event {
                picked = match change {
                    SelectionChange::Set(keys) => keys.last().and_then(|k| names.get(*k)).map(|n| json!(n)),
                    SelectionChange::Toggle(k) => names.get(k).map(|n| json!(n)),
                    SelectionChange::Clear => Some(Value::Null),
                    _ => picked.take(),
                };
            }
        }
        let selected = current.is_some();
        let any = rows(d).iter().any(|r| !excluded(r));
        // From the end edge: Delete, New Color Group, Save Changes.
        let actions = [
            FooterAction::new(IconId::new("trash-2"), tl!("Delete")).enabled(selected),
            FooterAction::new(IconId::new("dc-folder"), tl!("New Color Group")).enabled(any),
            FooterAction::new(IconId::new("save"), tl!("Save Changes")).enabled(selected),
        ];
        let footer = FooterBar::actions(actions).show_footer(ui);
        if let Some(action) = footer.picked.and_then(|i| ["delete", "new", "save"].get(i)) {
            d.fields.insert(GROUP_ACTION.into(), json!(action));
        }
    });
    if let Some(group) = picked {
        d.fields.insert("group".into(), group);
    }
}

/// The tint (0..1) `row` gives its spot color (1 without one).
fn row_tint(row: &Value) -> f32 {
    row["tint"].as_f64().map_or(1.0, |t| (t / 100.0).clamp(0.0, 1.0) as f32)
}

/// Spot color `c` as art linked to it shows it: a Lab spot color is the active document's
/// working-CMYK equivalent while the Spot Colors options use CMYK values
/// ([`vectorcraft_doc::Document::linked_color`]).
fn linked(app: &VectorcraftApp, c: Color) -> Color {
    app.session.active().map_or(c, |st| st.doc.linked_color(c, true))
}

/// Tint `t` (0..1) of spot color `c` as art linked to it shows it ([`linked`]): its inks scale.
fn spot_tinted(app: &VectorcraftApp, c: Color, t: f32) -> Color {
    linked(app, c).tinted(t)
}

/// The color row `row`'s new color chip shows: what the art takes, at the row's tint (a spot color
/// of the library as [`spot_tinted`] shows it).
fn new_shown(app: &VectorcraftApp, d: &Fields, row: &Value) -> Color {
    let to = to_color(row).unwrap_or_default();
    // Only a Lab spot color can show another color; the library lookup is skipped for the rest.
    if matches!(to, Color::Lab { .. }) && d.bool("__art") && spot_of(app, d, &to).is_some() {
        spot_tinted(app, to, row_tint(row))
    } else {
        to.tinted(row_tint(row))
    }
}

/// The `sliderMode` id of `m` (`hsb`, `rgb`, `cmyk`, `lab`), which is also its values' model id.
fn mode_id(m: ColorPanelMode) -> &'static str {
    match m {
        ColorPanelMode::RgbSliders => "rgb",
        ColorPanelMode::CmykSliders => "cmyk",
        ColorPanelMode::LabSliders => "lab",
        _ => "hsb",
    }
}

/// The color mode menu's name for `m`.
fn mode_label(m: ColorPanelMode) -> &'static str {
    match m {
        ColorPanelMode::RgbSliders => "RGB",
        ColorPanelMode::CmykSliders => "CMYK",
        ColorPanelMode::LabSliders => "Lab",
        _ => "HSB",
    }
}

/// The channels the sliders show in `mode`: label, least and greatest value, unit.
fn channels(mode: ColorPanelMode) -> &'static [(&'static str, f64, f64, ChannelUnit)] {
    match mode {
        ColorPanelMode::RgbSliders => {
            &[("R", 0.0, 255.0, ChannelUnit::None), ("G", 0.0, 255.0, ChannelUnit::None), ("B", 0.0, 255.0, ChannelUnit::None)]
        }
        ColorPanelMode::CmykSliders => &[
            ("C", 0.0, 100.0, ChannelUnit::Percent),
            ("M", 0.0, 100.0, ChannelUnit::Percent),
            ("Y", 0.0, 100.0, ChannelUnit::Percent),
            ("K", 0.0, 100.0, ChannelUnit::Percent),
        ],
        ColorPanelMode::LabSliders => {
            &[("L", 0.0, 100.0, ChannelUnit::None), ("a", -128.0, 127.0, ChannelUnit::None), ("b", -128.0, 127.0, ChannelUnit::None)]
        }
        _ => &[("H", 0.0, 360.0, ChannelUnit::Degrees), ("S", 0.0, 100.0, ChannelUnit::Percent), ("B", 0.0, 100.0, ChannelUnit::Percent)],
    }
}

/// `c`'s values in `mode` (HSB °/%/% and RGB 0 to 255 of the working RGB, CMYK %, Lab L*a*b*).
fn values_in(mode: ColorPanelMode, c: &Color) -> Vec<f32> {
    match mode {
        ColorPanelMode::RgbSliders => working_rgb(c).iter().map(|v| (v * 255.0).clamp(0.0, 255.0)).collect(),
        ColorPanelMode::CmykSliders => c.to_cmyk().iter().map(|v| v * 100.0).collect(),
        ColorPanelMode::LabSliders => {
            let l = c.to_lab();
            vec![l.l, l.a, l.b]
        }
        _ => {
            let hsb = egui::ecolor::Hsva::from_rgb(working_rgb(c));
            vec![hsb.h * 360.0, hsb.s * 100.0, hsb.v * 100.0]
        }
    }
}

/// The color `values` give in `mode`.
fn color_in(mode: ColorPanelMode, values: &[f32]) -> Color {
    color_of_values(&ModelValues::new(mode_id(mode), values))
}

/// The track of channel `n` (from `lo` to `hi`) of `mode` with the other values `comps`.
fn track(mode: ColorPanelMode, comps: &[f32], n: usize, lo: f64, hi: f64) -> Vec<Srgba> {
    (0..TRACK_STOPS)
        .map(|k| {
            let mut v = comps.to_vec();
            if let Some(x) = v.get_mut(n) {
                *x = (lo + (hi - lo) * k as f64 / (TRACK_STOPS - 1) as f64) as f32;
            }
            srgba(&color_in(mode, &v))
        })
        .collect()
}

/// The name of the library swatch (Limit to Library) that `recolor.apply` gives new color `c`, when
/// it is a spot color: the first swatch of the library color nearest `c`.
fn spot_of(app: &VectorcraftApp, d: &Fields, c: &Color) -> Option<String> {
    let lib = d.str("limitTo");
    if lib.is_empty() {
        return None;
    }
    let near = swatchlib::limit_palette(&app.session, &lib)?.nearest(*c);
    swatchlib::library_swatches(&app.session, &lib)?.into_iter().find(|w| w.paint.color() == Some(near)).filter(|w| w.spot).map(|w| w.name)
}

/// The mode the sliders show new color `c` in, `None` for Tint: the mode picked in the menu, else
/// `c`'s own model (HSB for a gray), or Tint for a spot color (`spot`).
fn slider_mode(d: &Fields, c: &Color, spot: bool) -> Option<ColorPanelMode> {
    let picked = d.str("sliderMode");
    match SLIDER_MODES.into_iter().find(|m| mode_id(*m) == picked) {
        Some(m) => Some(m),
        None if spot => None,
        None => Some(match c {
            Color::Rgb { .. } => ColorPanelMode::RgbSliders,
            Color::Cmyk { .. } => ColorPanelMode::CmykSliders,
            Color::Lab { .. } => ColorPanelMode::LabSliders,
            Color::Gray { .. } => ColorPanelMode::HsbSliders,
        }),
    }
}

/// The values the sliders show for row `i`'s new color `c` in `mode`: those they set last while
/// the color is the one they set, else `c`'s.
fn slider_values(d: &Fields, rows: &[Value], i: usize, c: &Color, mode: ColorPanelMode) -> Vec<f32> {
    let to = rows.get(i).map(|r| r["to"].clone()).unwrap_or_default();
    let held = d.fields.get(COMPS).filter(|s| s["row"] == json!(i) && s["key"] == to && s["mode"] == json!(mode_id(mode)));
    let v: Option<Vec<f32>> = held.and_then(|s| s["v"].as_array()).map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect());
    v.filter(|v| v.len() == channels(mode).len()).unwrap_or_else(|| values_in(mode, c))
}

/// The first selected row of `rows`.
fn first_selected(d: &Fields, rows: &[Value]) -> Option<usize> {
    selection(d).into_iter().filter(|i| *i < rows.len()).min()
}

/// What the color sliders edit: a row (the first selected row on the Assign tab, the base color's
/// row on the Edit tab), its new color, the name of the library's spot color that color is, and the
/// color art linked to it shows.
struct SliderTarget {
    row: Option<usize>,
    color: Option<Color>,
    spot: Option<String>,
    linked: Color,
}

impl SliderTarget {
    /// What the color sliders edit when they edit row `row`.
    fn at(feed: &Feed, d: &Fields, rows: &[Value], row: Option<usize>) -> Self {
        let row = row.filter(|i| *i < rows.len());
        let color = row.and_then(|i| rows.get(i)).and_then(to_color);
        let fed = row.and_then(|i| feed.rows.get(i));
        // A row tint acts only on art, so without art a spot color shows the process modes.
        let spot = fed.and_then(|f| f.spot.clone()).filter(|_| d.bool("__art") && color.is_some());
        Self { row, color, spot, linked: fed.map_or(color.unwrap_or_default(), |f| f.linked) }
    }

    /// The mode the sliders show (`None`: Tint).
    fn mode(&self, d: &Fields) -> Option<ColorPanelMode> {
        slider_mode(d, &self.color.unwrap_or_default(), self.spot.is_some())
    }
}

/// The color mode menu of the sliders (HSB, RGB, CMYK, Lab, Tint for a spot color, and with
/// `global` Global Adjust).
fn mode_menu(ui: &mut Ui, d: &mut Fields, target: &SliderTarget, global: bool) {
    let mut modes: Vec<(&'static str, &'static str)> = SLIDER_MODES.iter().map(|m| (mode_id(*m), mode_label(*m))).collect();
    if target.spot.is_some() {
        modes.push(("tint", "Tint"));
    }
    if global {
        modes.push((GLOBAL, GLOBAL_ADJUST));
    }
    let current = if global && d.str("sliderMode") == GLOBAL { GLOBAL } else { target.mode(d).map_or("tint", mode_id) };
    let at = modes.iter().position(|(id, _)| *id == current).unwrap_or(0);
    let entries: Vec<Entry<usize>> = modes.iter().enumerate().map(|(i, (_, label))| Entry::new(i, Label::from(*label))).collect();
    let chosen = Select::new(tl!("Color Mode:"), Bind::given(at), &entries).enabled(target.color.is_some() || global).show(ui);
    if let Some((id, _)) = chosen.value().and_then(|i| modes.get(*i)) {
        d.fields.insert("sliderMode".into(), json!(id));
    }
}

/// The Global Adjust amounts, in percent from -100 to 100, of `ADJUSTS`' keys in the
/// `globalAdjust` field (0 for one it lacks).
fn adjust_values(d: &Fields) -> [f64; 4] {
    let given = d.fields.get("globalAdjust");
    ADJUSTS.map(|k| given.and_then(|g| g.get(k)).and_then(Value::as_f64).unwrap_or(0.0).clamp(-100.0, 100.0))
}

/// The Global Adjust of `values` (percent, in the order of `ADJUSTS`).
fn global_adjust(values: [f64; 4]) -> GlobalAdjust {
    let [saturation, brightness, temperature, luminosity] = values.map(|v| (v / 100.0) as f32);
    GlobalAdjust { saturation, brightness, temperature, luminosity }
}

/// The Global Adjust sliders, two to a line: Saturation, Brightness, Temperature and Luminosity,
/// each from -100% to 100% (0% changes nothing). They change every new color at once, so they
/// edit no row; each rail shows its amounts on `sample`.
fn global_sliders(ui: &mut Ui, d: &mut Fields, sample: Color) {
    let labels = [tl!("Saturation"), tl!("Brightness"), tl!("Temperature"), tl!("Luminosity")];
    let values = adjust_values(d);
    let mut set: Option<(usize, f64)> = None;
    for (line, pair) in labels.chunks(2).enumerate() {
        Row::new().show(ui, |ui, _| {
            for (k, label) in pair.iter().enumerate() {
                let n = line * 2 + k;
                // The rail: `sample` with this slider's amount alone.
                let stops = (0..TRACK_STOPS)
                    .map(|s| {
                        let mut one = [0.0; 4];
                        if let Some(x) = one.get_mut(n) {
                            *x = -100.0 + 200.0 * s as f64 / (TRACK_STOPS - 1) as f64;
                        }
                        srgba(&global_adjust(one).apply(sample))
                    })
                    .collect();
                let value = values.get(n).copied().unwrap_or(0.0);
                let Some(out) =
                    ChannelRow::new(*label, Bind::given(value), whole_steps(-100.0..=100.0)).unit(ChannelUnit::Percent).track(stops).show(ui)
                else {
                    continue;
                };
                if let Some(&v) = out.events.value().filter(|v| v.is_finite()) {
                    set = Some((n, v.round()));
                }
            }
        });
    }
    if let Some((n, v)) = set {
        let mut values = values;
        if let Some(x) = values.get_mut(n) {
            *x = v;
        }
        d.fields.insert("globalAdjust".into(), Value::Object(ADJUSTS.iter().zip(values).map(|(k, v)| (k.to_string(), json!(v))).collect()));
    }
}

/// The color sliders, two to a line: they edit row `row`'s new color (the others keep theirs) in
/// the mode of [`mode_menu`], or its tint of a spot color of the library. Without a row they are
/// disabled. Returns true when they changed a row.
fn sliders(ui: &mut Ui, feed: &Feed, d: &mut Fields, rows: &mut [Value], row: Option<usize>) -> bool {
    let target = SliderTarget::at(feed, d, rows, row);
    let mut changed = false;
    Stack::new().enabled(target.color.is_some()).show(ui, |ui, _| {
        let (Some(i), Some(c)) = (target.row, target.color) else {
            // Nothing to edit: the mode's sliders, disabled.
            let mode = target.mode(d).unwrap_or(ColorPanelMode::HsbSliders);
            for pair in channels(mode).chunks(2) {
                Row::new().show(ui, |ui, _| {
                    for &(name, lo, hi, unit) in pair {
                        // A disabled row edits nothing: its output has no edit to read.
                        let _ = ChannelRow::new(name, Bind::given(lo), whole_steps(lo..=hi)).short(name).unit(unit).show(ui);
                    }
                });
            }
            return;
        };
        let Some(mode) = target.mode(d) else {
            // Tint of the spot color, and its name.
            let tint = f64::from(row_tint(&rows[i])) * 100.0;
            let stops: Vec<Srgba> = (0..TRACK_STOPS).map(|k| srgba(&target.linked.tinted(k as f32 / (TRACK_STOPS - 1) as f32))).collect();
            Row::new().show(ui, |ui, _| {
                if let Some(out) = ChannelRow::new(tl!("Tint"), Bind::given(tint), whole_steps(0.0..=100.0))
                    .short("T")
                    .unit(ChannelUnit::Percent)
                    .track(stops.clone())
                    .show(ui)
                    && let Some(&v) = out.events.value().filter(|v| v.is_finite())
                {
                    rows[i]["tint"] = json!(v.round());
                    changed = true;
                }
                Text::new(Label::from(target.spot.clone().unwrap_or_default())).show(ui);
            });
            return;
        };
        let comps = slider_values(d, rows, i, &c, mode);
        let mut set: Option<Vec<f32>> = None;
        for (line, pair) in channels(mode).chunks(2).enumerate() {
            Row::new().show(ui, |ui, _| {
                for (k, &(name, lo, hi, unit)) in pair.iter().enumerate() {
                    let n = line * 2 + k;
                    let value = f64::from(comps.get(n).copied().unwrap_or(0.0));
                    let Some(out) = ChannelRow::new(name, Bind::given(value), whole_steps(lo..=hi))
                        .short(name)
                        .unit(unit)
                        .track(track(mode, &comps, n, lo, hi))
                        .show(ui)
                    else {
                        continue;
                    };
                    if let Some(&v) = out.events.value().filter(|v| v.is_finite()) {
                        let mut next = set.take().unwrap_or_else(|| comps.clone());
                        if let Some(x) = next.get_mut(n) {
                            *x = v.clamp(lo, hi) as f32;
                        }
                        set = Some(next);
                    }
                }
            });
        }
        if let Some(v) = set {
            // The mode stays as the edit changes the color's model (a gray edited in HSB is RGB).
            d.fields.insert("sliderMode".into(), json!(mode_id(mode)));
            set_to(&mut rows[i], &color_in(mode, &v));
            d.fields.insert(COMPS.into(), json!({"row": i, "key": rows[i]["to"], "mode": mode_id(mode), "v": v}));
            rows[i]["model"] = json!("new");
            changed = true;
        }
    });
    changed
}

/// Merge the selected rows into the first of them (its new colour stays).
fn merge(rows: &mut Vec<Value>, sel: &[usize]) {
    let mut sel: Vec<usize> = sel.iter().copied().filter(|i| *i < rows.len()).collect();
    sel.sort_unstable();
    sel.dedup();
    let Some((&first, rest)) = sel.split_first() else { return };
    let mut moved = vec![];
    for &i in rest.iter().rev() {
        let row = rows.remove(i);
        moved.splice(0..0, row["from"].as_array().cloned().unwrap_or_default());
        // The base color merged into another row makes that row the base, and the colors that
        // followed the merged row's marker follow that row's.
        if row[BASE] == json!(true) {
            rows[first][BASE] = json!(true);
        }
        if let Some(t) = row[TIE].as_u64() {
            match rows[first][TIE].as_u64() {
                Some(to) => {
                    for r in rows.iter_mut().filter(|r| r[FOLLOWS].as_u64() == Some(t)) {
                        r[FOLLOWS] = json!(to);
                    }
                }
                None => rows[first][TIE] = json!(t),
            }
        }
    }
    if let Some(dst) = rows[first]["from"].as_array_mut() {
        dst.extend(moved);
    }
}

/// Split the selected rows into one row per colour, each its own new colour.
fn separate(rows: &mut Vec<Value>, sel: &[usize]) {
    let mut out = vec![];
    for (i, row) in rows.drain(..).enumerate() {
        match row["from"].as_array() {
            Some(a) if sel.contains(&i) && a.len() > 1 => {
                let at = out.len();
                out.extend(a.iter().map(|k| json!({"from": [k], "to": k})));
                // A separated row's marks (the base color, a marker's tie, a color it follows, its place) go
                // to the row of its first color.
                for k in [BASE, TIE, FOLLOWS, SLOT] {
                    if let Some(v) = row.get(k) {
                        out[at][k] = v.clone();
                    }
                }
            }
            _ => out.push(row),
        }
    }
    *rows = out;
}

/// New Row: insert a row with no current colors above the first selected row, or at the top
/// when none is selected, and return its index. Its new color is the new color of the row it goes
/// above (black without one). It recolors nothing until current colors are merged into it.
fn new_row(rows: &mut Vec<Value>, sel: &[usize]) -> usize {
    let at = sel.iter().copied().filter(|i| *i < rows.len()).min().unwrap_or(0);
    let to = rows.get(at).and_then(to_color).unwrap_or_default();
    rows.insert(at, json!({"from": [], "to": key(&to)}));
    at
}

/// The rows the Edit tab shows, the harmony rules fill and the Color Theme Picker recolors: those
/// with a new colour that aren't excluded and hold current colors or were added on the Edit tab.
fn active_rows(rows: &[Value]) -> Vec<usize> {
    (0..rows.len()).filter(|i| in_group(&rows[*i]) && to_color(&rows[*i]).is_some()).collect()
}

/// The row the Edit tab's color sliders edit: the first selected row when the tab shows it (one of
/// `active`), else the base color's row `base`. A selected row the tab does not show (excluded, or
/// empty) is left as it is.
fn edit_slider_row(d: &Fields, rows: &[Value], active: &[usize], base: Option<usize>) -> Option<usize> {
    first_selected(d, rows).filter(|i| active.contains(i)).or(base)
}

/// Set row `i`'s new colour to hue/saturation/brightness `hsb` ([`move_on_wheel`]: with `linked`
/// the other rows keep their relation to it; unlinked, the colors added on its marker's line, which
/// follow its `tie`, and those that follow theirs, keep theirs). Excluded rows stay.
fn move_color(rows: &mut [Value], i: usize, hsb: [f32; 3], linked: bool) {
    let all = active_rows(rows);
    let active: Vec<usize> = if linked {
        all
    } else {
        // Row `i` and the rows that follow it, directly or through another follower.
        let mut moving = vec![i];
        let mut n = 0;
        while let Some(&j) = moving.get(n) {
            if let Some(t) = rows.get(j).and_then(|r| r[TIE].as_u64()) {
                let next: Vec<usize> = all.iter().copied().filter(|&k| rows[k][FOLLOWS].as_u64() == Some(t) && !moving.contains(&k)).collect();
                moving.extend(next);
            }
            n += 1;
        }
        all.into_iter().filter(|j| moving.contains(j)).collect()
    };
    let Some(k) = active.iter().position(|&j| j == i) else { return };
    let before: Vec<Color> = active.iter().filter_map(|&j| to_color(&rows[j])).collect();
    let mut colors = before.clone();
    move_on_wheel(&mut colors, k, hsb, true);
    for ((&j, c), old) in active.iter().zip(&colors).zip(&before) {
        if key(c) != key(old) {
            set_to(&mut rows[j], c);
        }
    }
}

/// The rows when a drag on the Edit tab's wheel started.
const DRAG: &str = "__drag";

/// The field of the Edit tab's wheel tools: `add` (Add Color) or `remove` (Remove Color).
const WHEEL_TOOL: &str = "wheelTool";

/// The field of what the Edit tab's wheels show: `hueBrightness` (hue and brightness), or anything
/// else (hue and saturation).
const WHEEL_AXES: &str = "wheelAxes";

/// The channel the radius of the Edit tab's wheels shows, from [`WHEEL_AXES`].
fn wheel_axes(d: &Fields) -> DiscRadius {
    if d.str(WHEEL_AXES) == "hueBrightness" { DiscRadius::Brightness } else { DiscRadius::Saturation }
}

/// The new color the Add Color tool gives a click at hue `h` (degrees) and `t`, the channel the
/// wheel's `radius` shows (saturation, or brightness) at the click: with the other channel of
/// `base` (the selected color, whose other channel the wheel shows; full without one), the centre of
/// the segmented wheel's patch there with `segmented` ([`patch_at`]), in `base`'s model.
fn added_color(base: Option<Color>, h: f32, t: f32, segmented: bool, radius: DiscRadius) -> Color {
    let [_, s, v] = base.map_or([0.0, 1.0, 1.0], |b| b.to_hsb());
    let (h, t) = if segmented { patch_at(h, t) } else { (h, t) };
    let c = match radius {
        DiscRadius::Brightness => Color::from_hsb(h, s, t),
        _ => Color::from_hsb(h, t, v),
    };
    base.map_or(c, |b| keep_model(b, c))
}

/// Add new color `c` as a row with no current colors, marked `added`, after the others. Returns
/// its index.
fn add_color(rows: &mut Vec<Value>, c: &Color) -> usize {
    rows.push(json!({"from": [], "to": key(c), "added": true}));
    rows.len() - 1
}

/// Remove row `i`'s new color: a row with no current colors (or, without `keep`, any row) goes,
/// and with `keep` (art, or a color group whose swatches the rows are) a row with current colors
/// stays with no new color, so that its art keeps its colors and its group swatch is removed in
/// its place. The base row `base` stays. Returns the base row's index afterwards, or `None` when
/// nothing changed.
fn remove_color(rows: &mut Vec<Value>, i: usize, base: Option<usize>, keep: bool) -> Option<usize> {
    if Some(i) == base || i >= rows.len() || to_color(&rows[i]).is_none() {
        return None;
    }
    if keep && !no_current(&rows[i]) {
        rows[i]["to"] = Value::Null;
        if let Some(r) = rows[i].as_object_mut() {
            r.remove("tint");
            r.remove("model");
        }
        return Some(base.unwrap_or(0));
    }
    rows.remove(i);
    Some(base.map_or(0, |b| if b > i { b - 1 } else { b }))
}

/// Whether Remove Color keeps a row with current colors (see [`remove_color`]): with art, or with a
/// color group.
fn keeps_rows(d: &Fields) -> bool {
    d.bool("__art") || d.fields.get("group").is_some_and(Value::is_string)
}

/// After Remove Color: select the base row at `base`; the slider values held were for a row index
/// that may now hold another row.
fn removed(d: &mut Fields, base: usize) {
    d.fields.insert("__sel".into(), json!([base]));
    d.fields.remove(COMPS);
}

/// A row's mark as the base color, the color the harmony rules build from (`true`).
const BASE: &str = "base";
/// A row's number on a marker that the colors added on its line follow.
const TIE: &str = "tie";
/// A row's mark as a color added on a marker's line: the marker's `tie`, which it moves with.
const FOLLOWS: &str = "follows";
/// A row's place when the rows were made (the group swatch it took), which it keeps when it moves.
const SLOT: &str = "slot";

/// Give each row its place in `rows` as its [`SLOT`].
fn number_slots(rows: &mut [Value]) {
    for (i, r) in rows.iter_mut().enumerate() {
        r[SLOT] = json!(i);
    }
}

/// The rows with current colors in the order of their group swatches: each row takes the swatch at
/// its [`SLOT`], so a row moved among the color bars keeps its swatch; rows that don't all carry a
/// place go in row order.
fn by_place(rs: &[Value]) -> Vec<&Value> {
    let mut filled: Vec<&Value> = rs.iter().filter(|r| !no_current(r)).collect();
    if filled.iter().all(|r| r[SLOT].is_u64()) {
        filled.sort_by_key(|r| r[SLOT].as_u64());
    }
    filled
}

/// Select Shade's grid: saturations across, from 0% to 100%, and brightnesses down, from 100%.
const SHADE_COLS: usize = 7;
const SHADE_ROWS: usize = 7;

/// The base color's row: the row of `active` (the rows the Edit tab shows) marked `base`, else
/// the first of them.
fn base_row(rows: &[Value], active: &[usize]) -> Option<usize> {
    active.iter().copied().find(|&i| rows[i][BASE] == json!(true)).or(active.first().copied())
}

/// Whether Set As Base Color would change anything for row `i`: it is shown on the Edit tab, and it
/// isn't the base already, or it is the base only because the marked row is hidden (excluded, or
/// with no new color), whose mark would make it the base again once shown.
fn can_set_base(rows: &[Value], i: usize) -> bool {
    let active = active_rows(rows);
    active.contains(&i) && (base_row(rows, &active) != Some(i) || rows.iter().enumerate().any(|(k, r)| k != i && r[BASE] == json!(true)))
}

/// Make row `i` the base color (Set As Base Color). Returns false when that changes nothing
/// ([`can_set_base`]).
fn set_base(rows: &mut [Value], i: usize) -> bool {
    if !can_set_base(rows, i) {
        return false;
    }
    for r in rows.iter_mut() {
        if let Some(o) = r.as_object_mut() {
            o.remove(BASE);
        }
    }
    rows[i][BASE] = json!(true);
    true
}

/// Make row `follower` move with the marker of row `leader`: the leader keeps its `tie` (a new
/// number, past every `tie` and `follows`, when it has none, so that colors left from a removed
/// marker follow no other) and the follower takes it as `follows`.
fn tie_to(rows: &mut [Value], leader: usize, follower: usize) {
    if leader == follower || leader >= rows.len() || follower >= rows.len() {
        return;
    }
    let used = rows.iter().flat_map(|r| [r[TIE].as_u64(), r[FOLLOWS].as_u64()]).flatten().max();
    let t = rows[leader][TIE].as_u64().unwrap_or_else(|| used.map_or(1, |m| m.saturating_add(1)));
    rows[leader][TIE] = json!(t);
    rows[follower][FOLLOWS] = json!(t);
}

/// Give each row of `after` the base, tie and follows marks of the row at its place in `before`,
/// when the two have as many rows (the engine's `recolor.randomize` and `recolor.weigh` keep the
/// rows in order and return them without the dialog's own marks).
fn keep_marks(before: &[Value], after: &mut [Value]) {
    if before.len() != after.len() {
        return;
    }
    for (a, b) in after.iter_mut().zip(before) {
        for k in [BASE, TIE, FOLLOWS, SLOT] {
            if a.get(k).is_none()
                && let Some(v) = b.get(k)
            {
                a[k] = v.clone();
            }
        }
    }
}

/// `row` without the dialog's base, tie, follows and place marks.
fn without_marks(row: &Value) -> Value {
    let mut r = row.clone();
    if let Some(o) = r.as_object_mut() {
        for k in [BASE, TIE, FOLLOWS, SLOT] {
            o.remove(k);
        }
    }
    r
}

/// The saturation and brightness of Select Shade's cell in column `col` and row `row`.
fn shade_at(col: usize, row: usize) -> [f32; 2] {
    [col as f32 / (SHADE_COLS - 1) as f32, 1.0 - row as f32 / SHADE_ROWS as f32]
}

/// What a marker's or a color bar's right-click menu, or the wheel's, chose.
enum MenuChoice {
    /// Set As Base Color for the row.
    Base(usize),
    /// Select Shade: the row's color at this saturation and brightness.
    Shade(usize, [f32; 2]),
    /// Color Picker for the row.
    Picker(usize),
    /// Remove Color for the row.
    Remove(usize),
    /// Add New Color at this hue and saturation, moving with the marker of the row on whose line
    /// it is.
    Add([f32; 2], Option<usize>),
}

/// Carry out a right-click menu's `choice` on the rows (`base`: the base color's row; `shown`: the
/// color whose other channel the wheel shows; `segmented`: the menu was on the segmented wheel;
/// `radius`: what the wheel's radius shows). Returns whether the rows changed and the row whose new
/// color the Color Picker is to edit.
fn choose(
    d: &mut Fields,
    rows: &mut Vec<Value>,
    choice: MenuChoice,
    base: Option<usize>,
    shown: Option<Color>,
    segmented: bool,
    radius: DiscRadius,
) -> (bool, Option<usize>) {
    match choice {
        MenuChoice::Base(i) => (set_base(rows, i), None),
        MenuChoice::Shade(i, [s, v]) => {
            let Some(h) = rows.get(i).and_then(to_color).map(|c| c.to_hsb()[0]) else { return (false, None) };
            let before = rows.clone();
            move_color(rows, i, [h, s, v], d.bool("linked"));
            d.fields.insert("__sel".into(), json!([i]));
            d.fields.remove(COMPS);
            (*rows != before, None)
        }
        MenuChoice::Picker(i) => {
            d.fields.insert("__sel".into(), json!([i]));
            (false, Some(i))
        }
        MenuChoice::Remove(i) => match remove_color(rows, i, base, keeps_rows(d)) {
            Some(b) => {
                removed(d, b);
                (true, None)
            }
            None => (false, None),
        },
        MenuChoice::Add([h, s], line) => {
            let i = add_color(rows, &added_color(shown, h, s, segmented, radius));
            if let Some(l) = line {
                tie_to(rows, l, i);
            }
            d.fields.insert("__sel".into(), json!([i]));
            d.fields.remove(COMPS);
            (true, None)
        }
    }
}

/// Move the row of the `from`th active row (of `active`, ascending row indexes) to before the
/// `to`th, or after the last for `active.len()`; the rows between shift by one. Returns the row's
/// new index, or `None` when it stays where it is.
fn move_bar(rows: &mut [Value], active: &[usize], from: usize, to: usize) -> Option<usize> {
    let &src = active.get(from)?;
    if to == from || to == from + 1 || to > active.len() {
        return None;
    }
    if to > from {
        let dst = active[to - 1];
        rows[src..=dst].rotate_left(1);
        Some(dst)
    } else {
        let dst = active[to];
        rows[dst..=src].rotate_right(1);
        Some(dst)
    }
}

/// Give the rows `active` the colours of `rule` from the base colour `b` of row `base` (the base row
/// first, then the others in order, the rule's colours repeating), each in its row's model. A row
/// whose new color changes loses its tint.
fn apply_rule(rows: &mut [Value], rule: Harmony, base: usize, b: Color, active: &[usize]) {
    let colors = harmony::apply(rule, b);
    let order = std::iter::once(base).chain(active.iter().copied().filter(|i| *i != base));
    for (n, i) in order.enumerate() {
        let old = to_color(&rows[i]).unwrap_or(b);
        let c = keep_model(old, colors[n % colors.len()]);
        if key(&c) != rows[i]["to"] {
            set_to(&mut rows[i], &c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::Recolor;
    use super::*;
    use vectorcraft_engine::Session;

    fn rows() -> Vec<Value> {
        vec![
            json!({"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"}),
            json!({"from": ["rgb 0 0 255", "rgb 0 0 128"], "to": "rgb 0 0 255"}),
            json!({"from": ["rgb 0 255 0"], "to": "rgb 0 255 0"}),
        ]
    }

    /// An app with a new `width` by 100 document.
    fn app(width: u32) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": width, "height": 100})).unwrap();
        app
    }

    /// Recolor Artwork opened on `app` with `params`.
    fn opened(app: VectorcraftApp, params: Value) -> Recolor {
        let rc = Recolor::new(app);
        rc.app().run("ui.recolorDialog", params).unwrap();
        rc
    }

    #[test]
    fn recolor_dialog_keeps_the_default_model_without_an_extra_dropdown() {
        let rc = opened(app(100), json!({"colors": ["#ff8000"]}));
        let d = rc.fields();
        assert!(!d.fields.contains_key("model"));
        assert!(apply_params(&d).get("model").is_none());
    }

    /// `recolor.apply` gets the Global Adjust amounts only when one of them is not 0%, whatever
    /// mode the sliders show.
    #[test]
    fn apply_params_send_global_adjust_only_when_it_changes_colors() {
        let mut rc = opened(app(100), json!({"colors": ["#ff8000"]}));
        let mut d = rc.fields();
        assert!(apply_params(&d).get("globalAdjust").is_none(), "0% as the dialog opens");
        d.fields.insert("globalAdjust".into(), json!({"saturation": 0, "brightness": 0}));
        assert!(apply_params(&d).get("globalAdjust").is_none(), "all 0%");
        d.fields.insert("globalAdjust".into(), json!({"temperature": 25, "luminosity": 500}));
        d.fields.insert("sliderMode".into(), json!("hsb"));
        assert_eq!(
            apply_params(&d)["globalAdjust"],
            json!({"saturation": 0.0, "brightness": 0.0, "temperature": 25.0, "luminosity": 100.0}),
            "every amount, clamped to 100%"
        );
        // Colors opened alone become a new color group with the adjustment.
        rc.set("globalAdjust", json!({"saturation": -100})).unwrap();
        rc.confirm().unwrap();
        let app = rc.app();
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("a new color group");
        let c = g.swatches[0].paint.color().unwrap();
        assert!(c.to_hsb()[1] < 1e-3, "{c:?}");
    }

    /// Remove Color: a row with no current colors goes, with art or a group a row's current colors
    /// stay with no new color, without either the row goes, and the base row stays; the base row's
    /// index follows.
    #[test]
    fn remove_color_drops_added_rows_and_empties_rows_of_art() {
        let mut r = rows();
        assert_eq!(add_color(&mut r, &Color::rgb(0.0, 1.0, 1.0)), 3);
        assert_eq!(r[3], json!({"from": [], "to": "rgb 0 255 255", "added": true}));
        assert_eq!(remove_color(&mut r, 0, Some(0), true), None, "the base row stays");
        r[1]["tint"] = json!(50);
        assert_eq!(remove_color(&mut r, 1, Some(2), true), Some(2));
        assert_eq!(r[1], json!({"from": ["rgb 0 0 255", "rgb 0 0 128"], "to": null}), "the art's colors stay, without a tint");
        assert_eq!(remove_color(&mut r, 1, Some(2), true), None, "no new color left to remove");
        assert_eq!(remove_color(&mut r, 3, Some(2), true), Some(2));
        assert_eq!(r.len(), 3, "the added row goes");
        assert_eq!(remove_color(&mut r, 9, Some(2), true), None);
        let mut r = rows();
        assert_eq!(remove_color(&mut r, 0, Some(2), false), Some(1), "without art the row goes, and the base row moves up");
        assert_eq!(r.len(), 2);
        assert_eq!(r[1]["from"], json!(["rgb 0 255 0"]));
    }

    /// An added color is the hue and saturation clicked at the base color's brightness (full without
    /// one), the patch's color on the segmented wheel, in the base color's model.
    #[test]
    fn an_added_color_takes_the_base_colors_brightness_and_model() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        let sat = DiscRadius::Saturation;
        let base = Color::from_hsb(10.0, 0.8, 0.6);
        assert!(near(added_color(Some(base), 120.0, 0.5, false, sat).to_hsb(), [120.0, 0.5, 0.6]));
        assert!(near(added_color(None, 200.0, 0.25, false, sat).to_hsb(), [200.0, 0.25, 1.0]));
        assert!(near(added_color(None, 7.0, 0.95, true, sat).to_hsb(), [0.0, 0.9, 1.0]), "the patch's color");
        let cmyk = color_value(&json!("cmyk 0 100 100 0")).unwrap();
        assert!(matches!(added_color(Some(cmyk), 120.0, 0.5, false, sat), Color::Cmyk { .. }));
    }

    /// On the hue and brightness wheel an added color is the hue and brightness clicked at the base
    /// color's saturation (full without one), the patch's color on the segmented wheel (its ring is
    /// a brightness step), in the base color's model. A black base adds a gray of the brightness
    /// clicked, and the wheel's center is black.
    #[test]
    fn an_added_color_on_the_brightness_wheel_takes_the_base_colors_saturation() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        let bright = DiscRadius::Brightness;
        let base = Color::from_hsb(10.0, 0.8, 0.6);
        assert!(near(added_color(Some(base), 120.0, 0.5, false, bright).to_hsb(), [120.0, 0.8, 0.5]));
        assert!(near(added_color(None, 200.0, 0.25, false, bright).to_hsb(), [200.0, 1.0, 0.25]));
        assert!(near(added_color(Some(base), 7.0, 0.95, true, bright).to_hsb(), [0.0, 0.8, 0.9]), "the patch's color");
        assert!(near(added_color(Some(base), 7.0, 0.0, true, bright).to_hsb(), [0.0, 0.8, 0.1]), "the innermost ring");
        let black = Color::from_hsb(0.0, 0.0, 0.0);
        let [_, s, v] = added_color(Some(black), 90.0, 0.4, false, bright).to_hsb();
        assert!(s.abs() < 1e-3 && (v - 0.4).abs() < 1e-3, "a gray: {s} {v}");
        let [_, _, v] = added_color(Some(base), 33.0, 0.0, false, bright).to_hsb();
        assert!(v.abs() < 1e-3, "black at the center");
        let cmyk = color_value(&json!("cmyk 0 100 100 0")).unwrap();
        assert!(matches!(added_color(Some(cmyk), 120.0, 0.5, false, bright), Color::Cmyk { .. }));
    }

    /// With a color group and art, a row without a new color removes its swatch in its place, the
    /// group's colors no row of art took stay in theirs, and a color added on the Edit tab adds its
    /// new color after them; a row made with New Row, which holds no current colors, adds none.
    #[test]
    fn apply_params_give_the_group_added_colors_and_keep_its_other_colors() {
        let mut d = Fields::new(json!({"group": "G", "groupName": "G", "__art": true, "__assign": ["rgb 1 1 1", "rgb 2 2 2", "rgb 3 3 3"]}));
        let mut rs = vec![json!({"from": ["rgb 255 0 0"], "to": "rgb 1 1 1"}), json!({"from": ["rgb 0 0 255"], "to": null})];
        new_row(&mut rs, &[]);
        add_color(&mut rs, &Color::rgb(9.0 / 255.0, 9.0 / 255.0, 9.0 / 255.0));
        d.fields.insert("rows".into(), json!(rs));
        assert_eq!(apply_params(&d)["groupColors"], json!(["rgb 1 1 1", null, "rgb 3 3 3", "rgb 9 9 9"]));
        // An added color whose row is excluded joins no group.
        rs[3]["exclude"] = json!(true);
        d.fields.insert("rows".into(), json!(rs));
        assert_eq!(apply_params(&d)["groupColors"], json!(["rgb 1 1 1", null, "rgb 3 3 3"]));
    }

    /// An excluded row with current colors keeps the group's swatch at its place as it is, even
    /// after its new color was edited or removed, and with Global Adjust on.
    #[test]
    fn an_excluded_row_keeps_its_group_swatch() {
        let mut d = Fields::new(json!({"group": "G", "groupName": "G", "__art": true, "__assign": ["rgb 1 1 1", "rgb 2 2 2", "rgb 3 3 3"]}));
        let rs = json!([
            {"from": ["rgb 255 0 0"], "to": "rgb 7 7 7", "exclude": true},
            {"from": ["rgb 0 0 255"], "to": null, "exclude": true},
            {"from": ["rgb 0 255 0"], "to": "rgb 8 8 8"},
        ]);
        d.fields.insert("rows".into(), rs);
        assert_eq!(apply_params(&d)["groupColors"], json!(["keep", "keep", "rgb 8 8 8"]));
        // Without art the rows are the group's colors.
        let mut app = app(100);
        app.run("swatch.newGroup", json!({"name": "G", "colors": ["#2878f0", "#ff0000"]})).unwrap();
        let mut rc = opened(app, json!({"group": "G"}));
        let mut rs = rc.rows();
        rs[0]["to"] = json!("#00ff00");
        rs[0]["exclude"] = json!(true);
        rs[1]["to"] = json!("#0000ff");
        rc.set("rows", json!(rs)).unwrap();
        rc.set("globalAdjust", json!({"brightness": -50})).unwrap();
        rc.confirm().unwrap();
        let app = rc.app();
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "G").unwrap();
        let hex: Vec<String> = g.swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect();
        assert_eq!(hex[0], "#2878f0", "{hex:?}");
        assert_ne!(hex[1], "#ff0000", "{hex:?}");
    }

    /// A row moved among the color bars keeps the group swatch it took when the rows were made, so
    /// an excluded row's swatch stays as it is and the other swatches keep their places.
    #[test]
    fn a_moved_row_keeps_its_group_swatch() {
        let mut app = app(100);
        app.run("swatch.newGroup", json!({"name": "G", "colors": ["#ff0000", "#0000ff", "#00ff00"]})).unwrap();
        let mut rc = opened(app, json!({"group": "G"}));
        let mut rs = rc.rows();
        rs[1]["exclude"] = json!(true);
        rs[2]["to"] = json!("#ffff00");
        let active = active_rows(&rs);
        assert_eq!(active, [0, 2]);
        // Drag the last bar (green, now yellow) above the first (red).
        assert_eq!(move_bar(&mut rs, &active, 1, 0), Some(0));
        rc.set("rows", json!(rs)).unwrap();
        let colors = &apply_params(&rc.fields())["groupColors"];
        assert_eq!(colors[1], json!("keep"), "{colors}");
        rc.confirm().unwrap();
        let app = rc.app();
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "G").unwrap();
        let hex: Vec<String> = g.swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect();
        assert_eq!(hex, ["#ff0000", "#0000ff", "#ffff00"]);
        // Rows reduced from art carry their place too, and `recolor.apply` gets none.
        let mut rows = vec![json!({"from": ["rgb 255 0 0"], "to": "rgb 1 1 1"}), json!({"from": ["rgb 0 0 255"], "to": "rgb 2 2 2"})];
        number_slots(&mut rows);
        assert_eq!(rows.iter().map(|r| r[SLOT].clone()).collect::<Vec<_>>(), [json!(0), json!(1)]);
        assert!(without_marks(&rows[0]).get(SLOT).is_none());
    }

    #[test]
    fn merge_and_separate_rows() {
        let mut r = rows();
        merge(&mut r, &[2, 0]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["from"], json!(["rgb 255 0 0", "rgb 0 255 0"]));
        assert_eq!(r[0]["to"], "rgb 255 0 0", "the first row's new colour stays");
        separate(&mut r, &[1]);
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], json!({"from": ["rgb 0 0 255"], "to": "rgb 0 0 255"}));
        assert_eq!(r[2]["to"], "rgb 0 0 128");
    }

    /// A selection that lists a row twice merges it once, as `ui.dialog.set` can store one.
    #[test]
    fn merge_takes_a_repeated_row_once() {
        let mut r = rows();
        merge(&mut r, &[0, 2, 2]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["from"], json!(["rgb 255 0 0", "rgb 0 255 0"]));
        merge(&mut r, &[1, 1]);
        assert_eq!(r.len(), 2);
    }

    /// New Row goes above the first selected row with that row's new color, or at the top; it has
    /// no current colors, and a merge with it as the first selected row fills it.
    #[test]
    fn a_new_row_goes_above_the_selection_and_takes_merged_colors() {
        let mut r = rows();
        assert_eq!(new_row(&mut r, &[2, 1]), 1);
        assert_eq!(r.len(), 4);
        assert_eq!(r[1], json!({"from": [], "to": "rgb 0 0 255"}));
        assert_eq!(r[2]["from"], json!(["rgb 0 0 255", "rgb 0 0 128"]), "the selected row moved down");
        merge(&mut r, &[1, 3]);
        assert_eq!(r[1], json!({"from": ["rgb 0 255 0"], "to": "rgb 0 0 255"}));
        assert_eq!(new_row(&mut r, &[]), 0);
        assert_eq!(r[0], json!({"from": [], "to": "rgb 255 0 0"}));
        let mut none = vec![];
        assert_eq!(new_row(&mut none, &[5]), 0);
        assert_eq!(none, vec![json!({"from": [], "to": key(&Color::BLACK)})]);
    }

    /// Opened on a color group, OK rewrites it with the new colors of the rows that hold current
    /// colors, and the group's color no such row lists keeps its color: an empty row gives it no color.
    #[test]
    fn an_empty_row_gives_the_group_no_color() {
        let mut r = rows();
        new_row(&mut r, &[]);
        let fields = json!({"rows": r, "group": "Trio", "__art": true, "__assign": ["rgb 255 0 0", "rgb 0 0 255", "rgb 0 255 0", "rgb 9 9 9"]});
        let p = apply_params(&Fields::new(fields));
        assert_eq!(p["groupColors"], json!(["rgb 255 0 0", "rgb 0 0 255", "rgb 0 255 0", "keep"]));
    }

    /// Opened on colors alone, OK saves the new colors of the rows that hold current colors as the
    /// new color group, in row order: an empty row gives it no color.
    #[test]
    fn an_empty_row_gives_a_new_group_no_color() {
        let mut rc = opened(app(100), json!({"colors": ["#ff8000", "#0080ff"]}));
        let mut rs = rc.rows();
        let at = new_row(&mut rs, &[1]);
        rs[at]["to"] = json!("rgb 255 0 255");
        rc.set("rows", Value::Array(rs)).unwrap();
        let groups = rc.app().session.doc().unwrap().doc.swatch_groups.len();
        rc.confirm().unwrap();
        let app = rc.app();
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.swatch_groups.len(), groups + 1, "a new color group");
        let colors: Vec<String> = doc.swatch_groups.last().unwrap().swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect();
        assert_eq!(colors, ["#ff8000", "#0080ff"]);
    }

    /// A row that lists a color exactly comes before an earlier row whose "#rrggbb" entry matches
    /// it, as `recolor.apply` reads them; a hex entry matches a key of the same hex.
    #[test]
    fn a_row_listing_a_color_exactly_comes_before_a_hex_match() {
        let rs =
            [json!({"from": ["#0000ff"], "to": "rgb 0 255 0"}), json!({"from": ["RGB  0 0 255"], "to": "rgb 255 0 0"}), json!({"from": ["#ff0000"]})];
        assert_eq!(row_listing(&rs, &json!("rgb 0 0 255")), Some(&rs[1]));
        assert_eq!(row_listing(&rs, &json!("rgb 255 0 0")), Some(&rs[2]));
        assert_eq!(row_listing(&rs, &json!("#ff0000")), Some(&rs[2]));
        assert_eq!(row_listing(&rs, &json!("rgb 0 255 0")), None);
        assert_eq!(row_listing(&rs, &json!("cmyk 0 0 0 0")), None);
    }

    /// A row that gives a color a new color comes before an earlier row that removes it or is
    /// excluded, exact or by hex, as `recolor.apply` maps the color with it; those rows are found
    /// only when no such row lists the color.
    #[test]
    fn a_row_giving_a_color_a_new_color_comes_before_one_removing_it() {
        let rs = [
            json!({"from": ["rgb 0 0 255"], "to": null}),
            json!({"from": ["rgb 0 0 255"], "to": "rgb 0 0 0", "exclude": true}),
            json!({"from": ["#0000ff"], "to": "rgb 0 255 0"}),
            json!({"from": ["rgb 0 0 255"], "to": "rgb 255 0 0"}),
            json!({"from": ["rgb 255 255 0"], "to": null}),
        ];
        assert_eq!(row_listing(&rs, &json!("rgb 0 0 255")), Some(&rs[3]));
        assert_eq!(row_listing(&rs[..3], &json!("rgb 0 0 255")), Some(&rs[2]), "by hex");
        assert_eq!(row_listing(&rs[..2], &json!("rgb 0 0 255")), Some(&rs[0]));
        assert_eq!(row_listing(&rs, &json!("rgb 255 255 0")), Some(&rs[4]));
    }

    #[test]
    fn linked_handles_move_together() {
        let mut r = rows();
        move_color(&mut r, 0, [120.0, 1.0, 1.0], false);
        assert_eq!(r[0]["to"], "rgb 0 255 0");
        assert_eq!(r[1]["to"], "rgb 0 0 255", "unlinked: only that colour");
        let mut r = rows();
        move_color(&mut r, 0, [120.0, 1.0, 1.0], true);
        assert_eq!(r[1]["to"], "rgb 255 0 0", "linked: every colour turns by the same hue");
        assert_eq!(r[2]["to"], "rgb 0 0 255");
        // Excluded rows stay.
        let mut r = rows();
        r[2]["exclude"] = json!(true);
        move_color(&mut r, 0, [120.0, 1.0, 1.0], true);
        assert_eq!(r[2]["to"], "rgb 0 255 0");
    }

    #[test]
    fn harmony_rules_fill_the_rows_from_the_base() {
        let mut r = rows();
        apply_rule(&mut r, Harmony::Triad, 0, Color::rgb(1.0, 0.0, 0.0), &[0, 1, 2]);
        assert_eq!([&r[0]["to"], &r[1]["to"], &r[2]["to"]], ["rgb 255 0 0", "rgb 0 255 0", "rgb 0 0 255"]);
    }

    /// The base color is the row marked `base`, else the first row shown; Set As Base Color moves the
    /// mark, and does nothing for the base row already or a row the Edit tab doesn't show.
    #[test]
    fn set_base_moves_the_base_mark() {
        let mut r = rows();
        assert_eq!(base_row(&r, &active_rows(&r)), Some(0));
        assert!(!set_base(&mut r, 0), "the first row is the base already");
        assert!(set_base(&mut r, 2));
        assert_eq!(base_row(&r, &active_rows(&r)), Some(2));
        assert!(set_base(&mut r, 1));
        assert_eq!(r.iter().filter(|x| x[BASE] == json!(true)).count(), 1, "one base mark: {r:?}");
        assert!(!set_base(&mut r, 1));
        r[2]["exclude"] = json!(true);
        assert!(!set_base(&mut r, 2), "an excluded row isn't shown");
        assert!(!set_base(&mut r, 9));
        // An excluded base row leaves the first row shown as the base.
        r[1]["exclude"] = json!(true);
        assert_eq!(base_row(&r, &active_rows(&r)), Some(0));
        // Setting that row as the base keeps it the base once the marked row is shown again.
        assert!(set_base(&mut r, 0), "the hidden row's mark moves to it");
        r[1]["exclude"] = json!(false);
        assert_eq!(base_row(&r, &active_rows(&r)), Some(0), "{r:?}");
    }

    /// Merge and Separate keep the base mark on a row of the base color's colors; Randomize's rows
    /// (as the engine returns them) take the marks at their places again; `recolor.apply` gets no marks.
    #[test]
    fn the_base_mark_follows_merge_separate_and_randomize() {
        let mut r = rows();
        set_base(&mut r, 2);
        merge(&mut r, &[0, 2]);
        assert_eq!(r[0][BASE], json!(true), "merged into the first row: {r:?}");
        let mut r = rows();
        set_base(&mut r, 1);
        separate(&mut r, &[1]);
        assert_eq!(r.len(), 4);
        assert_eq!((r[1][BASE].clone(), r[2].get(BASE)), (json!(true), None), "{r:?}");
        // A marker's tie goes with it too.
        let mut t = rows();
        t[1][TIE] = json!(1);
        separate(&mut t, &[1]);
        assert_eq!((t[1][TIE].clone(), t[2].get(TIE)), (json!(1), None));
        let mut t = rows();
        t[0][TIE] = json!(1);
        t[2][TIE] = json!(3);
        t.push(json!({"from": [], "to": "rgb 1 2 3", "follows": 3}));
        merge(&mut t, &[0, 2]);
        assert_eq!((t[0][TIE].clone(), t[2][FOLLOWS].clone()), (json!(1), json!(1)), "its follower follows the row merged into: {t:?}");
        let before = r.clone();
        let mut shuffled: Vec<Value> = r.iter().map(without_marks).collect();
        assert!(shuffled.iter().all(|x| x.get(BASE).is_none()));
        keep_marks(&before, &mut shuffled);
        assert_eq!(shuffled, before);
        let mut fewer = vec![json!({"from": [], "to": "rgb 1 1 1"})];
        keep_marks(&before, &mut fewer);
        assert!(fewer[0].get(BASE).is_none(), "rows of another count take no marks");
        let mut d = Fields::new(json!({}));
        d.fields.insert("rows".into(), json!(before));
        assert!(apply_params(&d)["map"].as_array().unwrap().iter().all(|x| x.get(BASE).is_none()));
    }

    /// A color added on a marker's line moves with that marker when the colors are unlinked; it
    /// moves alone itself, and linked colors all move together as before.
    #[test]
    fn a_color_added_on_a_markers_line_moves_with_it() {
        let hue = |r: &[Value], i: usize| to_color(&r[i]).unwrap().to_hsb()[0];
        let mut r = rows();
        let added = add_color(&mut r, &Color::from_hsb(240.0, 0.5, 1.0));
        tie_to(&mut r, 1, added);
        assert_eq!((r[1][TIE].clone(), r[added][FOLLOWS].clone()), (json!(1), json!(1)));
        move_color(&mut r, 1, [270.0, 1.0, 1.0], false);
        assert!((hue(&r, added) - 270.0).abs() < 0.5, "the added color turns with its marker: {}", hue(&r, added));
        assert!((hue(&r, 0) - 0.0).abs() < 0.5 && (hue(&r, 2) - 120.0).abs() < 0.5, "the others stay");
        move_color(&mut r, added, [300.0, 0.5, 1.0], false);
        assert!((hue(&r, 1) - 270.0).abs() < 0.5, "its marker stays");
        // A second color tied to the same marker shares its number.
        let next = add_color(&mut r, &Color::from_hsb(270.0, 0.2, 1.0));
        tie_to(&mut r, 1, next);
        assert_eq!(r[next][FOLLOWS], json!(1));
        let other = add_color(&mut r, &Color::from_hsb(0.0, 0.2, 1.0));
        tie_to(&mut r, 0, other);
        assert_eq!(r[0][TIE], json!(2), "another marker takes a new number");
        tie_to(&mut r, 0, 0);
        assert!(r[0].get(FOLLOWS).is_none(), "a row isn't tied to itself");
        // Colors left from a removed marker follow no marker tied later.
        let mut r = rows();
        let added = add_color(&mut r, &Color::from_hsb(240.0, 0.5, 1.0));
        tie_to(&mut r, 1, added);
        r.remove(1);
        let next = add_color(&mut r, &Color::from_hsb(0.0, 0.5, 1.0));
        tie_to(&mut r, 0, next);
        assert_eq!(r[0][TIE], json!(2), "{r:?}");
        // A color added on an added color's line moves with the first marker too.
        let mut r = rows();
        let added = add_color(&mut r, &Color::from_hsb(240.0, 0.5, 1.0));
        tie_to(&mut r, 1, added);
        let chained = add_color(&mut r, &Color::from_hsb(240.0, 0.25, 1.0));
        tie_to(&mut r, added, chained);
        move_color(&mut r, 1, [250.0, 1.0, 1.0], false);
        assert!((hue(&r, chained) - 250.0).abs() < 0.5, "{r:?}");
    }

    /// A row made with New Row is not on the Edit tab while it is empty: a harmony rule gives it
    /// no color and a linked move leaves its new color as it is.
    #[test]
    fn an_empty_row_takes_no_harmony_or_wheel_color() {
        let mut r = rows();
        new_row(&mut r, &[]);
        assert_eq!(r[0]["to"], "rgb 255 0 0");
        let active = active_rows(&r);
        assert_eq!(active, [1, 2, 3]);
        apply_rule(&mut r, Harmony::Triad, 1, Color::rgb(1.0, 0.0, 0.0), &active);
        assert_eq!([&r[0]["to"], &r[1]["to"], &r[2]["to"], &r[3]["to"]], ["rgb 255 0 0", "rgb 255 0 0", "rgb 0 255 0", "rgb 0 0 255"]);
        move_color(&mut r, 1, [120.0, 1.0, 1.0], true);
        assert_eq!(r[1]["to"], "rgb 0 255 0");
        assert_eq!(r[0]["to"], "rgb 255 0 0", "the empty row keeps its new color");
        // With the empty row selected, the sliders edit the base color's row instead.
        let sel = |s: Value| Fields::new(json!({"__sel": s}));
        assert_eq!(edit_slider_row(&sel(json!([0])), &r, &active, Some(1)), Some(1));
        assert_eq!(edit_slider_row(&sel(json!([3, 2])), &r, &active, Some(3)), Some(2), "a shown row: the first selected");
        assert_eq!(edit_slider_row(&sel(json!([0])), &r, &[], None), None, "no row to edit");
    }

    /// A color added on the Edit tab holds no current colors but is not an empty row: the Edit tab
    /// shows it and a color group takes it.
    #[test]
    fn an_added_color_is_not_an_empty_row() {
        let added = json!({"from": [], "to": "rgb 0 255 255", "added": true});
        assert!(!empty(&added) && in_group(&added));
        let mut r = rows();
        r.push(added);
        assert_eq!(active_rows(&r), [0, 1, 2, 3]);
    }

    /// A rule or a wheel move that gives a row a new color drops the row's tint; a row whose
    /// color stays keeps it.
    #[test]
    fn a_new_color_from_the_edit_tab_drops_the_rows_tint() {
        let tinted = || {
            let mut r = rows();
            for row in &mut r {
                row["tint"] = json!(40.0);
            }
            r
        };
        let mut r = tinted();
        apply_rule(&mut r, Harmony::Triad, 0, Color::rgb(1.0, 0.0, 0.0), &[0, 1, 2]);
        assert_eq!(r[0]["tint"], json!(40.0), "the base row's color stays");
        assert!(r[1].get("tint").is_none() && r[2].get("tint").is_none(), "{r:?}");
        let mut r = tinted();
        move_color(&mut r, 0, [120.0, 1.0, 1.0], false);
        assert!(r[0].get("tint").is_none(), "the moved row: {:?}", r[0]);
        assert_eq!(r[1]["tint"], json!(40.0), "unlinked: the other rows keep theirs");
    }

    /// The Color Picker's OK keeps a row's tint when the row's new color stays, and drops it otherwise.
    #[test]
    fn a_picked_color_keeps_the_tint_only_of_the_same_color() {
        let mut rc = opened(app(100), json!({"colors": ["#ff0000"]}));
        rc.set("rows", json!([{"from": ["rgb 255 0 0"], "to": "rgb 255 0 0", "tint": 40.0}])).unwrap();
        rc.with_dialog(|app, d| with_new_color(app, d, 0, Color::rgb(1.0, 0.0, 0.0))).unwrap();
        assert_eq!(rc.rows()[0]["tint"], json!(40.0));
        rc.with_dialog(|app, d| with_new_color(app, d, 0, Color::rgb(0.0, 1.0, 0.0))).unwrap();
        assert!(rc.rows()[0].get("tint").is_none(), "{:?}", rc.rows());
    }

    /// With the Spot Colors options on CMYK values, a row whose new color is a Lab spot color of the
    /// library shows the art's color at its tint: the working-CMYK equivalent with its inks scaled.
    #[test]
    fn a_tinted_lab_spot_row_shows_the_cmyk_color_the_art_takes() {
        let mut app = app(100);
        let lab = json!({"l": 50, "a": 60, "b": -20});
        app.run("swatch.new", json!({"name": "Lab Ink", "color": lab, "spot": true})).unwrap();
        let data = app.run("swatch.library.save", json!({"names": ["Lab Ink"], "name": "Inks"})).unwrap()["data"].as_str().unwrap().to_string();
        let lib = app.run("swatch.library.load", json!({"data": data, "name": "inks.vcswatches"})).unwrap()["library"].as_str().unwrap().to_string();
        app.run("swatch.delete", json!({"names": ["Lab Ink"]})).unwrap();
        app.run("swatch.spotOptions", json!({"useLab": false})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": lab})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let rc = opened(app, json!({"library": lib}));
        let mut d = rc.fields();
        sync(&mut rc.app(), &mut d);
        let mut rs = super::rows(&d);
        rs[0]["tint"] = json!(50.0);
        let c = to_color(&rs[0]).unwrap();
        assert!(matches!(c, Color::Lab { .. }), "{c:?}");
        let app = rc.app();
        let cmyk = app.session.doc().unwrap().doc.linked_color(c, true);
        assert!(matches!(cmyk, Color::Cmyk { .. }), "{cmyk:?}");
        assert_eq!(new_shown(&app, &d, &rs[0]), cmyk.tinted(0.5));
    }

    /// The new colors of the rows.
    fn new_colors(rc: &Recolor) -> Vec<Value> {
        rc.rows().iter().map(|r| r["to"].clone()).collect()
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// Double-clicking a row's new color opens the Color Picker on that new color, over the dialog.
    /// Cancel leaves the rows and the preview as they were; OK sets that row's new color, and only
    /// that row's, in the document's model (a color picked as RGB is CMYK in a CMYK document), and
    /// the art previews it; nothing is committed until Recolor Artwork's own OK.
    // The row's `ColorRequest` opens the framework's `ColorPickerDialog` as a child of Recolor Artwork. Its
    // `ui.dialog.set` takes only `#rrggbb`, so the trial picks in RGB; a CMYK mix and a Lab color kept in their model
    // are color-w8's picker trials (`in_document_model`).
    #[test]
    fn double_clicking_a_new_color_picks_it_in_its_own_model() {
        use craft::color_ui::ui::ColorPickerDialog;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"})).unwrap();
        let mut ids = vec![];
        for (n, c) in [json!({"c": 0, "m": 100, "y": 100, "k": 0}), json!({"c": 100, "m": 0, "y": 0, "k": 0})].into_iter().enumerate() {
            let id = app.run("shape.rectangle", json!({"x": 10 + 30 * n, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"ids": [id], "color": c})).unwrap();
            app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
            ids.push(id.as_u64().unwrap());
        }
        app.run("select.all", json!({})).unwrap();
        // New colors other than the current ones, so the picker can only start from the new one.
        let assign = json!([{"c": 0, "m": 0, "y": 100, "k": 0}, {"c": 0, "m": 100, "y": 0, "k": 0}]);
        let mut rc = opened(app, json!({"colors": assign}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        let before = undo_len(&rc.app());
        // The previewed fill of the rectangle whose color row `row` recolors.
        let fill = |rc: &Recolor, row: &Value| {
            let from = from_colors(row);
            let app = rc.app();
            let doc = &app.session.doc().unwrap().doc;
            let original = |id: u64| if id == ids[0] { Color::cmyk(0.0, 1.0, 1.0, 0.0) } else { Color::cmyk(1.0, 0.0, 0.0, 0.0) };
            let id = *ids.iter().find(|id| from.contains(&original(**id))).expect("a rectangle for the row");
            doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill().and_then(|f| f.paint.color())
        };
        rc.settled();
        let all = rc.rows();
        assert_eq!(all.len(), 2);
        let i = 1;
        assert_ne!(all[i]["from"][0], all[i]["to"], "the row's new color isn't its current color");
        let previewed = to_color(&all[i]);
        assert_eq!(fill(&rc, &all[i]), previewed, "the art previews the row's new color");
        let new = format!("{} {}", tl!("New"), i + 1);
        let picker = |rc: &Recolor| rc.host(|host, _| host.get::<ColorPickerDialog>().map(StackDialog::id)).flatten();

        rc.double_click(&new).frame();
        let id = picker(&rc).expect("a double-click opens the Color Picker");
        let color = rc.host(|host, _| host.get::<ColorPickerDialog>().and_then(StackDialog::json_fields)).flatten().unwrap_or_default();
        assert_eq!(color["color"].as_str(), previewed.map(|c| c.to_hex()).as_deref(), "on the row's new color");
        assert_eq!(fill(&rc, &all[0]), to_color(&all[0]), "the picker previews nothing itself");
        rc.host(|host, ctx| host.cancel_by_id(ctx, id));
        rc.frame().frame();
        assert!(rc.is_open() && picker(&rc).is_none(), "Cancel comes back to Recolor Artwork");
        assert_eq!(rc.rows(), all, "with the rows as they were");
        assert_eq!(fill(&rc, &all[i]), previewed, "and the art previews them again");

        rc.settled().double_click(&new).frame();
        let id = picker(&rc).expect("the Color Picker opens again");
        rc.host(|host, ctx| host.set_json_field(ctx, id, "color", json!("#ff0000"))).unwrap().unwrap();
        rc.host(|host, ctx| host.confirm_by_id(ctx, id));
        rc.frame().frame();
        let rows = rc.rows();
        let to = rows[i]["to"].clone();
        assert!(to.as_str().is_some_and(|k| k.starts_with("cmyk ")), "a color picked as RGB is CMYK in a CMYK document: {to}");
        assert_eq!(rows[0], all[0], "the other row stays");
        assert!(rc.app().session.in_interaction(), "the dialog still previews");
        assert_eq!(undo_len(&rc.app()), before, "nothing is committed before Recolor Artwork's OK");
        assert_eq!(fill(&rc, &rows[i]), to_color(&rows[i]), "the art previews the picked color");
    }

    /// A 15 pt square at (`x`, 10) filled with `fill`, without a stroke.
    fn square(app: &mut VectorcraftApp, x: f64, fill: &str) -> u64 {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 15, "height": 15})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [id], "color": fill})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        id
    }

    /// The fill of object `id`, as hex.
    fn fill_hex(rc: &Recolor, id: u64) -> String {
        let app = rc.app();
        let doc = &app.session.doc().unwrap().doc;
        doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill().and_then(|f| f.paint.color()).unwrap().to_hex()
    }

    /// Red and blue squares selected, and green and yellow squares beside them; Recolor Artwork
    /// open on the selection with the Exact method, one frame drawn.
    fn theme_setup() -> (Recolor, [u64; 2]) {
        let mut app = app(300);
        let art = [square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff")];
        square(&mut app, 100.0, "#00ff00");
        square(&mut app, 130.0, "#ffff00");
        app.run("select.set", json!({"ids": art})).unwrap();
        let mut rc = opened(app, json!({}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        (rc, art)
    }

    /// The Color Theme Picker's picks (the `themePicks` field) give the rows that aren't excluded
    /// the palette of the art picked, in row order, repeating it when the rows outnumber its
    /// colors, and the art previews them. Picks that touch no art leave the rows as they are, and
    /// OK with rows that give the art its own colors back records no undo step.
    #[test]
    fn theme_picks_give_the_rows_the_palette_of_the_art_picked() {
        let (mut rc, [red, blue]) = theme_setup();
        let mut start = new_colors(&rc);
        start.sort_by_key(Value::to_string);
        assert_eq!(start, [json!("rgb 0 0 255"), json!("rgb 255 0 0")]);
        // The red art's row and the blue art's row.
        let row_of = |rc: &Recolor, key: &str| rc.rows().iter().position(|r| r["from"][0] == key).unwrap();
        let (r, b) = (row_of(&rc, "rgb 255 0 0"), row_of(&rc, "rgb 0 0 255"));
        let undo = undo_len(&rc.app());
        let fills = |rc: &Recolor| (fill_hex(rc, red), fill_hex(rc, blue));
        rc.set(THEME_PICKS, json!([{"at": [107, 17]}])).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 255 0"), json!("rgb 0 255 0")], "one color for both rows");
        assert_eq!(fills(&rc), ("#00ff00".into(), "#00ff00".into()), "the art previews it");
        rc.set(THEME_PICKS, json!([{"at": [107, 17]}, {"rect": [125, 5, 150, 30]}])).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 255 0"), json!("rgb 255 255 0")], "a pick added: two colors");
        // An excluded row (the red art's) keeps its new color.
        let mut rs = rc.rows();
        rs[r]["exclude"] = json!(true);
        rc.set("rows", Value::Array(rs)).unwrap();
        let kept = new_colors(&rc)[r].clone();
        rc.set(THEME_PICKS, json!([{"at": [47, 17]}])).unwrap();
        rc.frame();
        let after = new_colors(&rc);
        assert_eq!((&after[r], &after[b]), (&kept, &json!("rgb 0 0 255")));
        rc.set(THEME_PICKS, json!([{"at": [250, 80]}])).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), after, "empty canvas: the rows stay");
        assert_eq!(undo_len(&rc.app()), undo, "nothing is committed before OK");
        rc.confirm().unwrap();
        assert_eq!(fills(&rc), ("#ff0000".into(), "#0000ff".into()), "the excluded row kept red");
        assert_eq!(undo_len(&rc.app()), undo, "the art is unchanged: no undo step");
    }

    /// The Color Theme Picker gives its colors only to the rows that hold current colors: a row
    /// made with New Row keeps its new color and takes no place in the palette.
    #[test]
    fn theme_picks_leave_an_empty_row_out() {
        let (mut rc, _) = theme_setup();
        let mut rs = rc.rows();
        let at = new_row(&mut rs, &[]);
        rs[at]["to"] = json!("rgb 9 9 9");
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.set(THEME_PICKS, json!([{"at": [107, 17]}, {"rect": [125, 5, 150, 30]}])).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 9 9 9"), json!("rgb 0 255 0"), json!("rgb 255 255 0")]);
    }

    /// A row made with New Row recolors nothing while it is empty, and OK keeps it; the blue art's
    /// color merged into it takes its new color, in the preview and at OK.
    #[test]
    fn colors_merged_into_a_new_row_take_its_new_color() {
        let (mut rc, [red, blue]) = theme_setup();
        let undo = undo_len(&rc.app());
        let fills = |rc: &Recolor| (fill_hex(rc, red), fill_hex(rc, blue));
        let mut rs = rc.rows();
        let at = new_row(&mut rs, &[]);
        rs[at]["to"] = json!("rgb 0 255 0");
        rc.set("rows", Value::Array(rs.clone())).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#ff0000".into(), "#0000ff".into()), "an empty row recolors nothing");
        let b = rs.iter().position(|r| r["from"] == json!([BLUE])).unwrap();
        merge(&mut rs, &[at, b]);
        // A second empty row stays empty and is sent at OK too.
        new_row(&mut rs, &[]);
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.frame();
        assert_eq!(fill_hex(&rc, blue), "#00ff00", "the art previews the merge");
        rc.confirm().unwrap();
        assert_eq!(fills(&rc), ("#ff0000".into(), "#00ff00".into()));
        assert_eq!(undo_len(&rc.app()), undo + 1, "one undo step");
    }

    const RED: &str = "rgb 255 0 0";
    const PURPLE: &str = "rgb 128 0 204";
    const BLUE: &str = "rgb 0 0 255";

    /// Two red squares, a purple one and a blue one, selected; Recolor Artwork open on them on the
    /// Edit tab with the Exact method and the rows set: the blue to yellow, then the red and purple
    /// to green (the purple's square id last).
    fn prominent_setup() -> (Recolor, u64) {
        let mut app = app(300);
        let ids = [
            square(&mut app, 10.0, "#ff0000"),
            square(&mut app, 40.0, "#ff0000"),
            square(&mut app, 70.0, "#8000cc"),
            square(&mut app, 100.0, "#0000ff"),
        ];
        app.run("select.set", json!({"ids": ids})).unwrap();
        let mut rc = opened(app, json!({}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.set("tab", json!("edit")).unwrap();
        rc.frame();
        rc.set("rows", json!([{"from": [BLUE], "to": "rgb 255 255 0"}, {"from": [RED, PURPLE], "to": "rgb 0 255 0"}])).unwrap();
        rc.frame();
        (rc, ids[2])
    }

    fn froms(rc: &Recolor) -> Vec<Value> {
        rc.rows().iter().map(|r| r["from"].clone()).collect()
    }

    /// The Prominent Colors bar holds each row's share of the art's uses, the largest first; new
    /// `weights` move the rows' current colors toward them, the art previews the result, and only
    /// OK commits it, as one undo step.
    #[test]
    fn weights_move_the_rows_colors_and_the_art_previews_them() {
        let (mut rc, purple) = prominent_setup();
        assert_eq!(rc.field(SHARES), Some(json!([25.0, 75.0])));
        assert_eq!(prominent_parts(&rc.fields(), &rc.rows()), [(1, 75.0), (0, 25.0)], "the largest share first");
        assert_eq!(fill_hex(&rc, purple), "#00ff00", "the art previews the rows");
        let undo = undo_len(&rc.app());
        rc.set(WEIGHTS, json!([50, 50])).unwrap();
        rc.frame();
        assert_eq!(froms(&rc), [json!([BLUE, PURPLE]), json!([RED])]);
        assert_eq!(rc.field(SHARES), Some(json!([50.0, 50.0])));
        assert_eq!(fill_hex(&rc, purple), "#ffff00", "the moved color previews its new row's color");
        assert_eq!(undo_len(&rc.app()), undo, "nothing is committed before OK");
        // A new count of Colors reduces the rows again and drops the weights.
        rc.set("colors", json!(2)).unwrap();
        rc.frame();
        assert_eq!(rc.field(WEIGHTS), Some(Value::Null));
        rc.set("rows", json!([{"from": [BLUE, PURPLE], "to": "rgb 255 255 0"}, {"from": [RED], "to": "rgb 0 255 0"}])).unwrap();
        rc.confirm().unwrap();
        assert_eq!(fill_hex(&rc, purple), "#ffff00");
        assert_eq!(undo_len(&rc.app()), undo + 1, "one undo step");
    }

    /// A row that takes a new color takes the colors it shares with other rows, so the bar reads
    /// the shares again.
    #[test]
    fn a_new_color_for_a_row_reads_the_shares_again() {
        let (mut rc, _) = prominent_setup();
        rc.set("rows", json!([{"from": [RED, PURPLE], "to": null}, {"from": [RED, BLUE], "to": "rgb 255 255 0"}])).unwrap();
        rc.frame();
        assert_eq!(rc.field(SHARES), Some(json!([25.0, 75.0])));
        rc.set("rows", json!([{"from": [RED, PURPLE], "to": "rgb 0 255 0"}, {"from": [RED, BLUE], "to": "rgb 255 255 0"}])).unwrap();
        rc.frame();
        assert_eq!(rc.field(SHARES), Some(json!([75.0, 25.0])));
    }

    /// Dragging the edge between two parts of the bar to the left gives the right-hand part more
    /// weight; the rows wait for the drag to end, and when it ends the rows' current colors move.
    #[test]
    fn dragging_a_prominent_edge_moves_colors_between_rows() {
        let (mut rc, purple) = prominent_setup();
        rc.settled();
        let bar = rc.by_label("Prominent Colors").expect("the Prominent Colors bar").rect;
        // The edges are pointer-only parts with no access node: the first is on the right side of the leftmost part.
        let first = *rc.graphics_in(bar).first().expect("the bar's parts");
        let edge = egui::pos2(first.right(), first.center().y);
        // From 75% to 50%: a quarter of the bar to the left.
        let to = edge - egui::vec2(bar.width() * 0.25, 0.0);
        rc.press_at(edge);
        for k in 1..=4 {
            rc.move_to(edge + (to - edge) * (k as f32 / 4.0));
        }
        assert_eq!(froms(&rc)[0], json!([BLUE]), "the rows wait for the drag to end");
        rc.release(to);
        let w: Vec<f64> = rc.field(WEIGHTS).and_then(|w| w.as_array().cloned()).unwrap_or_default().iter().filter_map(Value::as_f64).collect();
        assert!(w.len() == 2 && (w[0] - 50.0).abs() < 2.0 && (w[1] - 50.0).abs() < 2.0, "{w:?}");
        rc.frame();
        assert_eq!(froms(&rc), [json!([BLUE, PURPLE]), json!([RED])]);
        assert_eq!(fill_hex(&rc, purple), "#ffff00");
    }

    /// The colors of the last color group, as hex.
    fn last_group(app: &VectorcraftApp) -> Vec<String> {
        let doc = &app.session.doc().unwrap().doc;
        doc.swatch_groups.last().unwrap().swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect()
    }

    /// The New Color Group menu's Save All Colors saves the new colors in row order and Save
    /// Prominent Colors the bar's colors, the largest share first, each as its own step; the
    /// preview of the recolor runs on.
    #[test]
    fn save_all_and_prominent_colors_make_color_groups() {
        let (mut rc, purple) = prominent_setup();
        let groups = |rc: &Recolor| rc.app().session.doc().unwrap().doc.swatch_groups.len();
        let (n, undo) = (groups(&rc), undo_len(&rc.app()));
        for (which, colors) in [("all", ["#ffff00", "#00ff00"]), ("prominent", ["#00ff00", "#ffff00"])] {
            rc.set(SAVE_GROUP, json!(which)).unwrap();
            rc.frame().frame();
            assert_eq!(last_group(&rc.app()), colors, "{which}");
            assert_eq!(fill_hex(&rc, purple), "#00ff00", "{which}: the art still previews the rows");
        }
        assert_eq!(groups(&rc), n + 2);
        assert_eq!(undo_len(&rc.app()), undo + 2, "a step for each group");
    }

    /// A color removed on the Edit tab leaves its row with no new color, and neither Save All
    /// Colors, Save Prominent Colors nor OK on colors opened alone puts the row's current color in
    /// the new color group.
    #[test]
    fn a_removed_color_joins_no_saved_color_group() {
        let (mut rc, _) = prominent_setup();
        rc.set("rows", json!([{"from": [BLUE], "to": null}, {"from": [RED, PURPLE], "to": "rgb 0 255 0"}])).unwrap();
        rc.set(SAVE_GROUP, json!("all")).unwrap();
        rc.frame().frame();
        assert_eq!(last_group(&rc.app()), ["#00ff00"], "Save All Colors");
        let n = rc.app().session.doc().unwrap().doc.swatch_groups.len();
        rc.set(SAVE_GROUP, json!("prominent")).unwrap();
        rc.frame().frame();
        assert_eq!(rc.app().session.doc().unwrap().doc.swatch_groups.len(), n + 1, "Save Prominent Colors makes a group");
        assert_eq!(last_group(&rc.app()), ["#00ff00"], "Save Prominent Colors");

        let mut rc = opened(app(100), json!({"colors": ["#ff0000", "#0000ff"]}));
        rc.set("rows", json!([{"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"}, {"from": ["rgb 0 0 255"], "to": null}])).unwrap();
        rc.confirm().unwrap();
        assert_eq!(last_group(&rc.app()), ["#ff0000"], "OK on colors opened alone");
    }

    /// Save All Colors leaves out a row made with New Row while it is empty; with only empty rows
    /// it saves no group and records no step.
    #[test]
    fn save_all_colors_leaves_an_empty_row_out() {
        let (mut rc, _) = prominent_setup();
        let groups = |rc: &Recolor| rc.app().session.doc().unwrap().doc.swatch_groups.len();
        let (n, undo) = (groups(&rc), undo_len(&rc.app()));
        let save = |rc: &mut Recolor, rows: Value| {
            rc.set("rows", rows).unwrap();
            rc.set(SAVE_GROUP, json!("all")).unwrap();
            rc.frame().frame();
        };
        let blank = json!({"from": [], "to": "rgb 255 0 255"});
        save(&mut rc, json!([blank, {"from": [BLUE], "to": "rgb 255 255 0"}, {"from": [RED, PURPLE], "to": "rgb 0 255 0"}]));
        assert_eq!(last_group(&rc.app()), ["#ffff00", "#00ff00"]);
        assert_eq!(groups(&rc), n + 1);
        save(&mut rc, json!([blank, {"from": [BLUE, RED, PURPLE], "to": "rgb 0 255 0", "exclude": true}]));
        assert_eq!(groups(&rc), n + 1, "only an empty and an excluded row: no group");
        assert_eq!(undo_len(&rc.app()), undo + 1, "and no step");
    }

    /// The Color Theme Picker button turns picking from the canvas on: off, the canvas tool takes no
    /// picks; on, it does, and the picks it files give the rows the palette of the art picked, which
    /// the art previews.
    // PLAN: the click and Shift-drag on the canvas are color-w20's `ThemePickTool` trials (Open question 2): this
    // trial drives the button through the dialog and the gate `edit_tab::with_theme_pick` (cv.S3a) the canvas runs.
    #[test]
    fn the_theme_picker_takes_clicks_and_shift_drags_on_the_canvas() {
        let (mut rc, [red, _]) = theme_setup();
        rc.settled();
        assert_eq!(rc.field(THEME_PICKS), Some(json!([])), "off: no picks");
        assert!(!edit_tab::with_theme_pick(&mut rc.app(), |_, _| {}), "off: the canvas keeps its tool");
        rc.click_label(tl!("Color Theme Picker"));
        assert_eq!(rc.field(THEME_PICKER), Some(json!(true)), "the button turns the picker on");
        assert!(edit_tab::with_theme_pick(&mut rc.app(), |_, _| {}), "on: the picker takes the canvas");
        rc.set(THEME_PICKS, json!([{"at": [107, 17]}])).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 255 0"), json!("rgb 0 255 0")]);
        assert_eq!(fill_hex(&rc, red), "#00ff00", "the art previews the picked color");
        rc.set(THEME_PICKS, json!([{"at": [107, 17]}, {"rect": [125, 5, 150, 30]}])).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 255 0"), json!("rgb 255 255 0")], "a Shift pick adds to the picks");
    }

    /// Find previews nothing of its own while it is off, on the Edit tab, without art or without a
    /// selected row. On, a first `recolor.apply` dims each current color of the other rows (a color
    /// a selected row also lists stays), and with Preview on a second recolors the selected rows
    /// alone, without rewriting the color group.
    #[test]
    fn find_dims_the_colors_of_the_rows_not_selected() {
        let mut d = Fields::new(
            json!({"rows": rows(), "__art": true, "preview": true, "method": "exact", "limitTo": "", "group": "G", "groupName": "H", "__sel": [0]}),
        );
        assert_eq!(find_params(&d), None, "off");
        d.fields.insert(FIND.into(), json!(true));
        let dim = |k: &str| key(&color_value(&json!(k)).unwrap().tinted(FIND_DIM));
        let p = find_params(&d).expect("on");
        let cmds = p["commands"].as_array().unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0]["command"], CMD);
        assert_eq!(
            cmds[0]["params"]["map"],
            json!({"rgb 0 0 255": dim("rgb 0 0 255"), "rgb 0 0 128": dim("rgb 0 0 128"), "rgb 0 255 0": dim("rgb 0 255 0")})
        );
        assert_eq!(dim("rgb 0 0 255"), json!("rgb 191.25 191.25 255"), "a quarter of the color, the rest white");
        let second = &cmds[1]["params"];
        assert_eq!(second["map"][0], rows()[0], "the selected row recolors as it would");
        assert_eq!((second["map"][1]["exclude"].clone(), second["map"][2]["exclude"].clone()), (json!(true), json!(true)));
        assert!(second.get("group").is_none() && second.get("rename").is_none(), "{second}");
        // A color that a selected row lists as a hex stays.
        let mut e = d.clone();
        let mut rs = rows();
        rs[0]["from"] = json!(["#0000ff"]);
        e.fields.insert("rows".into(), json!(rs));
        let map = find_params(&e).unwrap()["commands"][0]["params"]["map"].clone();
        assert!(map.get("rgb 0 0 255").is_none() && map.get("rgb 0 0 128").is_some(), "{map}");
        // A color a selected row lists by its key keeps only that color: another key of the same hex
        // dims, and a hex entry of it stays (it would dim the selected color too).
        let mut e = d.clone();
        let mut rs = rows();
        rs[0]["from"] = json!(["rgb 191 191 255"]);
        rs[1]["from"] = json!(["rgb 191.25 191.25 255", "#bfbfff"]);
        e.fields.insert("rows".into(), json!(rs));
        let map = find_params(&e).unwrap()["commands"][0]["params"]["map"].clone();
        assert!(map.get("rgb 191.25 191.25 255").is_some() && map.get("#bfbfff").is_none(), "{map}");
        // A key is read whatever its case or spacing, as `recolor.apply` reads it.
        let mut e = d.clone();
        let mut rs = rows();
        rs[0]["from"] = json!(["RGB  191 191 255"]);
        rs[1]["from"] = json!(["rgb 191 191 255", "rgb 0 0 128"]);
        e.fields.insert("rows".into(), json!(rs));
        let map = find_params(&e).unwrap()["commands"][0]["params"]["map"].clone();
        assert!(map.get("rgb 191 191 255").is_none() && map.get("rgb 0 0 128").is_some(), "{map}");
        // The preserved colors dim too.
        let mut e = d.clone();
        e.fields.insert(PRESERVED.into(), json!(["rgb 0 0 0"]));
        assert_eq!(find_params(&e).unwrap()["commands"][0]["params"]["map"]["rgb 0 0 0"], dim("rgb 0 0 0"));
        // A color given as a value instead of a string dims under its key, as `recolor.apply` reads it.
        let mut e = d.clone();
        let mut rs = rows();
        rs[1]["from"] = json!([[0.0, 0.0, 1.0], {"c": 0, "m": 100, "y": 100, "k": 0}]);
        e.fields.insert("rows".into(), json!(rs));
        let map = find_params(&e).unwrap()["commands"][0]["params"]["map"].clone();
        assert_eq!(map["rgb 0 0 255"], dim("rgb 0 0 255"), "{map}");
        assert_eq!(map["cmyk 0 100 100 0"], dim("cmyk 0 100 100 0"), "{map}");
        // A color a selected row lists is not dimmed; with Preview off only the dimming runs.
        let mut rs = rows();
        rs[2]["from"] = json!(["rgb 0 255 0", "rgb 255 0 0"]);
        d.fields.insert("rows".into(), json!(rs));
        d.fields.insert("preview".into(), json!(false));
        let p = find_params(&d).unwrap();
        assert_eq!(p["commands"].as_array().map(Vec::len), Some(1));
        assert!(p["commands"][0]["params"]["map"].get("rgb 255 0 0").is_none(), "{p}");
        // A dimmed color that a selected row lists would be recolored with it: a lighter tint is taken.
        let mut rs = rows();
        rs[0]["from"] = json!(["rgb 255 0 0", "rgb 191.25 191.25 255"]);
        d.fields.insert("rows".into(), json!(rs));
        let p = find_params(&d).unwrap();
        let to = color_value(&p["commands"][0]["params"]["map"]["rgb 0 0 255"]).unwrap();
        assert_eq!(to.to_hex(), "#c2c2ff", "{p}");
        // So is one that a selected "#rrggbb" entry matches by its hex.
        rs[0]["from"] = json!(["rgb 255 0 0", "#bfbfff"]);
        d.fields.insert("rows".into(), json!(rs));
        let p = find_params(&d).unwrap();
        let to = color_value(&p["commands"][0]["params"]["map"]["rgb 0 0 255"]).unwrap();
        assert_eq!(to.to_hex(), "#c2c2ff", "{p}");
        // When the twenty tints from 25% through 6% are all listed, the 5% tint is taken.
        let blue = color_value(&json!("rgb 0 0 255")).unwrap();
        let tint = |n: usize| key(&blue.tinted(FIND_DIM - n as f32 * 0.01));
        let mut from = vec![json!("rgb 255 0 0")];
        from.extend((0..20).map(tint));
        rs[0]["from"] = json!(from);
        d.fields.insert("rows".into(), json!(rs));
        let p = find_params(&d).unwrap();
        assert_eq!(p["commands"][0]["params"]["map"]["rgb 0 0 255"], tint(20), "{p}");
        // Another key of the same hex matches only itself, so the tint is kept.
        rs[0]["from"] = json!(["rgb 255 0 0", "rgb 191 191 255"]);
        d.fields.insert("rows".into(), json!(rs));
        let p = find_params(&d).unwrap();
        assert_eq!(p["commands"][0]["params"]["map"]["rgb 0 0 255"], dim("rgb 0 0 255"), "{p}");
        // Every row selected with Preview off: nothing to show.
        d.fields.insert("__sel".into(), json!([0, 1, 2]));
        assert_eq!(find_params(&d), None);
        for (k, v) in [("__sel", json!([])), ("__sel", json!([7])), ("tab", json!("edit")), ("__art", json!(false))] {
            let mut e = d.clone();
            e.fields.insert("preview".into(), json!(true));
            e.fields.insert("__sel".into(), json!([0]));
            e.fields.insert(k.into(), v.clone());
            assert_eq!(find_params(&e), None, "{k}: {v}");
        }
    }

    /// In the dialog, Find with the red art's row selected shows the red art's new color and the
    /// blue art dimmed, with Preview off the red art's own color; off again, the art shows as the
    /// preview has it. Cancel leaves the art as it was with no undo step, and OK with Find on
    /// applies the rows to all the art, as one undo step.
    #[test]
    fn find_shows_the_selected_rows_art_and_dims_the_rest() {
        let (mut rc, [red, blue]) = theme_setup();
        let undo = undo_len(&rc.app());
        let fills = |rc: &Recolor| (fill_hex(rc, red), fill_hex(rc, blue));
        let rs = json!([{"from": ["rgb 255 0 0"], "to": "rgb 0 255 0"}, {"from": ["rgb 0 0 255"], "to": "rgb 255 255 0"}]);
        rc.set("rows", rs.clone()).unwrap();
        rc.set("__sel", json!([0])).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#00ff00".into(), "#ffff00".into()), "off: the preview");
        rc.set(FIND, json!(true)).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#00ff00".into(), "#bfbfff".into()), "on: the other art dimmed");
        rc.set("preview", json!(false)).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#ff0000".into(), "#bfbfff".into()), "Preview off: the current colors");
        rc.set("__sel", json!([1])).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#ffbfbf".into(), "#0000ff".into()), "the other row selected");
        rc.set(FIND, json!(false)).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#ff0000".into(), "#0000ff".into()), "off with Preview off: the art as it is");
        assert_eq!(undo_len(&rc.app()), undo, "Find records no undo step");
        rc.set(FIND, json!(true)).unwrap();
        rc.frame();
        rc.cancel();
        assert!(!rc.is_open());
        assert_eq!(fills(&rc), ("#ff0000".into(), "#0000ff".into()), "Cancel: the art as it was");
        assert_eq!(undo_len(&rc.app()), undo);
        // A black square joins the selection: Preserve Black leaves it out of the rows.
        let black = square(&mut rc.app(), 70.0, "#000000");
        let undo = undo_len(&rc.app());
        rc.app().run("select.set", json!({"ids": [red, blue, black]})).unwrap();
        rc.app().run("ui.recolorDialog", json!({})).unwrap();
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        rc.set("rows", rs).unwrap();
        rc.set("__sel", json!([0])).unwrap();
        rc.set(FIND, json!(true)).unwrap();
        rc.frame();
        assert_eq!(fills(&rc), ("#00ff00".into(), "#bfbfff".into()));
        assert_eq!(fill_hex(&rc, black), "#bfbfbf", "the preserved black dims too");
        rc.confirm().unwrap();
        assert_eq!(fills(&rc), ("#00ff00".into(), "#ffff00".into()), "OK applies the rows to all the art");
        assert_eq!(fill_hex(&rc, black), "#000000", "and keeps the preserved black");
        assert_eq!(undo_len(&rc.app()), undo + 1, "one undo step");
    }

    /// In a CMYK document colors opened alone are saved as a new color group in their own model.
    #[test]
    fn a_new_group_from_colors_keeps_their_model_in_a_cmyk_document() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"})).unwrap();
        let mut rc = opened(app, json!({"colors": ["#ff8000"]}));
        rc.confirm().unwrap();
        let app = rc.app();
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("a new color group");
        assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), [Color::rgb8(255, 128, 0)]);
    }

    fn group_hex(app: &VectorcraftApp, name: &str) -> Option<Vec<String>> {
        let doc = &app.session.doc().unwrap().doc;
        doc.swatch_groups.iter().find(|g| g.name == name).map(|g| g.swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect())
    }

    /// An app whose document has the color groups `groups` (name and colors), without art.
    fn groups_app(groups: &[(&str, &[&str])]) -> VectorcraftApp {
        let mut app = app(100);
        for (name, colors) in groups {
            app.run("swatch.newGroup", json!({"name": name, "colors": colors, "keepModel": true})).unwrap();
        }
        app
    }

    /// Without art: a group picked in the Color Groups list (the `group` field) gives the rows its
    /// colors, and a name no group has is refused. Save Changes rewrites the selected group with the
    /// rows' new colors and the name field, as one step, and records none when nothing changes;
    /// Delete removes it, after which OK saves the rows as a new group; New Color Group adds one.
    #[test]
    fn the_color_groups_list_selects_saves_and_deletes_groups() {
        let mut rc = opened(groups_app(&[("A", &["#ff0000", "#00ff00"]), ("B", &["#0000ff", "#ffff00", "#ffffff"])]), json!({"group": "A"}));
        assert_eq!(new_colors(&rc), [json!("rgb 255 0 0"), json!("rgb 0 255 0")]);
        assert_eq!(rc.field(GROUP_STORAGE), Some(json!(true)), "the list opens shown");
        rc.settled();

        rc.set("group", json!("B")).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 0 255"), json!("rgb 255 255 0"), json!("rgb 255 255 255")]);
        assert_eq!(rc.fields().str("groupName"), "B");
        assert_eq!(rc.set("group", json!("Nope")), Err("no colour group `Nope`".into()));
        assert_eq!(rc.field("group"), Some(json!("B")), "a refused name leaves B selected");
        assert!(rc.set(GROUP_STORAGE, json!("yes")).is_err());
        assert!(rc.set(GROUP_ACTION, json!("rename")).is_err());

        let undo = undo_len(&rc.app());
        let mut rs = rc.rows();
        rs[0]["to"] = json!("rgb 0 0 0");
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.set("groupName", json!("Night")).unwrap();
        rc.settled();
        assert!(rc.enabled(tl!("Save Changes")), "a selected group can be saved");
        rc.click_label(tl!("Save Changes"));
        assert_eq!(group_hex(&rc.app(), "Night"), Some(vec!["#000000".into(), "#ffff00".into(), "#ffffff".into()]));
        assert_eq!(group_hex(&rc.app(), "B"), None, "renamed");
        assert_eq!(rc.field("group"), Some(json!("Night")));
        assert_eq!(undo_len(&rc.app()), undo + 1, "one step");
        rc.act("save").unwrap();
        assert_eq!(undo_len(&rc.app()), undo + 1, "saving the same colors again records no step");

        rc.settled().click_label(tl!("Delete"));
        assert_eq!(group_hex(&rc.app(), "Night"), None);
        assert_eq!(undo_len(&rc.app()), undo + 2);
        let d = rc.fields();
        assert!(d.fields["group"].is_null() && d.bool("__newGroup"), "{:?}", d.fields);
        assert_eq!(rc.act("delete"), Err("missing `group`".into()));
        assert_eq!(rc.act("save"), Err("missing `group`".into()));
        assert_eq!(undo_len(&rc.app()), undo + 2, "a refused button records no step");

        let n = rc.app().session.doc().unwrap().doc.swatch_groups.len();
        rc.settled().click_label(tl!("New Color Group"));
        assert_eq!(rc.app().session.doc().unwrap().doc.swatch_groups.len(), n + 1);
        assert_eq!(last_group_colors(&rc.app()), ["#000000", "#ffff00", "#ffffff"]);
        assert_eq!(group_hex(&rc.app(), "A"), Some(vec!["#ff0000".into(), "#00ff00".into()]), "other groups are left as they are");
    }

    /// The colors of the last color group that are colors, as hex.
    fn last_group_colors(app: &VectorcraftApp) -> Vec<String> {
        let doc = &app.session.doc().unwrap().doc;
        doc.swatch_groups.last().unwrap().swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect()
    }

    /// Without art: rows of a group picked in the list keep their swatches when a color bar moves,
    /// at OK and at Save Changes, and an excluded row's swatch stays as it is.
    #[test]
    fn a_moved_row_of_a_listed_group_keeps_its_swatch_at_save_changes() {
        let mut rc = opened(groups_app(&[("A", &["#000000"]), ("G", &["#ff0000", "#0000ff", "#00ff00"])]), json!({"group": "A"}));
        rc.set("group", json!("G")).unwrap();
        rc.frame();
        let mut rs = rc.rows();
        rs[1]["exclude"] = json!(true);
        rs[2]["to"] = json!("#ffff00");
        let active = active_rows(&rs);
        // Drag the last bar (green, now yellow) above the first (red).
        assert_eq!(move_bar(&mut rs, &active, 1, 0), Some(0));
        rc.set("rows", Value::Array(rs)).unwrap();
        assert_eq!(apply_params(&rc.fields())["groupColors"][1], json!("keep"));
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "G"), Some(vec!["#ff0000".into(), "#0000ff".into(), "#ffff00".into()]));
    }

    /// Sets row 0's new color to `to`, as `ui.dialog.set` of the rows does.
    fn edit_first(rc: &mut Recolor, to: &str) {
        let mut rs = rc.rows();
        rs[0]["to"] = json!(to);
        rc.set("rows", Value::Array(rs)).unwrap();
    }

    /// Without art: selecting another color group in the list and Save Changes leave Undo Changes
    /// and Redo Changes no step to go to, so OK gives the selected group the rows read for it.
    #[test]
    fn another_group_or_save_changes_leaves_nothing_to_undo() {
        let mut rc = opened(groups_app(&[("A", &["#ff0000", "#00ff00"]), ("B", &["#0000ff", "#ffff00", "#ffffff"])]), json!({"group": "A"}));
        rc.frame();
        edit_first(&mut rc, "rgb 0 0 0");
        rc.frame();
        assert!(can(&rc, true), "the edit is a step");

        rc.set("group", json!("B")).unwrap();
        rc.frame();
        assert!(!can(&rc, true) && !can(&rc, false), "another group leaves no step");
        assert_eq!(step(&mut rc.app(), true).unwrap()["changed"], json!(false));
        assert_eq!(new_colors(&rc).len(), 3, "the rows of B");

        edit_first(&mut rc, "rgb 0 128 0");
        rc.frame();
        assert!(can(&rc, true), "the edit is a step");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "B"), Some(vec!["#008000".into(), "#ffff00".into(), "#ffffff".into()]));
        assert!(!can(&rc, true) && !can(&rc, false), "Save Changes leaves no step");
        assert_eq!(step(&mut rc.app(), true).unwrap()["changed"], json!(false));

        rc.confirm().unwrap();
        assert_eq!(group_hex(&rc.app(), "B"), Some(vec!["#008000".into(), "#ffff00".into(), "#ffffff".into()]), "OK keeps B's saved colors");
        assert_eq!(group_hex(&rc.app(), "A"), Some(vec!["#ff0000".into(), "#00ff00".into()]), "and leaves A as it was");
    }

    /// Without art: rows set after another color group is selected, with no frame between, are a
    /// step back to the rows read for that group, which Undo Changes and Redo Changes step through.
    #[test]
    fn an_edit_right_after_another_group_is_a_step() {
        let mut rc = opened(groups_app(&[("A", &["#ff0000", "#00ff00"]), ("B", &["#0000ff", "#ffff00", "#ffffff"])]), json!({"group": "A"}));
        rc.frame();
        rc.set("group", json!("B")).unwrap();
        rc.frame();
        assert!(!can(&rc, true) && !can(&rc, false), "another group leaves no step");
        let b = new_colors(&rc);
        edit_first(&mut rc, "rgb 0 0 0");
        assert!(can(&rc, true), "the edit is a step");
        assert_eq!(step(&mut rc.app(), true).unwrap()["changed"], json!(true));
        assert_eq!(new_colors(&rc), b, "Undo Changes gives back the rows read for B");
        assert_eq!(step(&mut rc.app(), false).unwrap()["changed"], json!(true));
        assert_eq!(new_colors(&rc)[0], json!("rgb 0 0 0"), "Redo Changes gives back the edit");
    }

    /// With art not linked to the group: Save Changes with the rows as the last Save Changes left
    /// them, after an edit and Undo Changes, leaves Redo Changes no step to go to either.
    #[test]
    fn an_unchanged_save_changes_leaves_nothing_to_redo() {
        let mut app = app(300);
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        let mut rc = opened(app, json!({"group": "Plain"}));
        rc.frame();
        edit_first(&mut rc, "rgb 0 128 0");
        rc.frame();
        rc.act("save").unwrap();
        assert!(rc.fields().fields.contains_key(SAVED_ROWS), "the art kept its colors");
        edit_first(&mut rc, "rgb 0 0 0");
        rc.frame();
        assert_eq!(step(&mut rc.app(), true).unwrap()["changed"], json!(true));
        assert!(unchanged_since_save(&rc.fields()), "the rows are as saved");
        assert!(can(&rc, false), "the edit can be redone");
        rc.act("save").unwrap();
        assert!(!can(&rc, true) && !can(&rc, false), "Save Changes leaves no step");
    }

    /// Without art: after a group is picked in the list, the same Color Theme Picker picks that gave
    /// the group before it its colors give the new group's rows the palette of the art picked too.
    #[test]
    fn theme_picks_apply_again_after_another_group_is_picked() {
        let mut app = app(300);
        square(&mut app, 100.0, "#00ff00");
        app.run("select.set", json!({"ids": []})).unwrap();
        app.run("swatch.newGroup", json!({"name": "A", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "B", "colors": ["#0000ff", "#ffff00", "#ffffff"], "keepModel": true})).unwrap();
        let mut rc = opened(app, json!({"group": "A"}));
        rc.frame();
        let picks = json!([{"at": [107, 17]}]);
        rc.set(THEME_PICKS, picks.clone()).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 255 0"), json!("rgb 0 255 0")]);
        rc.set("group", json!("B")).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 0 255"), json!("rgb 255 255 0"), json!("rgb 255 255 255")]);
        rc.set(THEME_PICKS, picks).unwrap();
        rc.frame();
        assert_eq!(new_colors(&rc), [json!("rgb 0 255 0"), json!("rgb 0 255 0"), json!("rgb 0 255 0")], "the picks apply to B's rows");
    }

    /// A gradient swatch, for a color group.
    fn fade_swatch() -> vectorcraft_color::Swatch {
        let paint = vectorcraft_color::Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
        vectorcraft_color::Swatch { name: "Fade".into(), paint, global: false, spot: false }
    }

    /// The refusal of Save Changes on a group with a swatch that is not a color.
    fn not_colors(group: &str) -> Result<(), String> {
        Err(format!("color group `{group}` holds swatches that are not colors, which Save Changes can't keep"))
    }

    /// Save Changes on a group with a swatch that is not a color is refused, renamed or not, and
    /// leaves the group as it is: the rows hold only its colors.
    #[test]
    fn saving_a_group_with_a_swatch_that_is_not_a_color_is_refused() {
        let mut app = groups_app(&[("Mixed", &["#ff0000", "#00ff00"])]);
        let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
        let g = doc.swatch_groups.iter_mut().find(|g| g.name == "Mixed").unwrap();
        g.swatches.insert(1, fade_swatch());
        let mut rc = opened(app, json!({"group": "Mixed"}));
        assert_eq!(new_colors(&rc), [json!("rgb 255 0 0"), json!("rgb 0 255 0")]);
        let undo = undo_len(&rc.app());
        edit_first(&mut rc, "rgb 0 0 0");
        assert_eq!(rc.act("save"), not_colors("Mixed"));
        rc.set("groupName", json!("Renamed")).unwrap();
        assert_eq!(rc.act("save"), not_colors("Mixed"));
        assert_eq!(rc.field(GROUP_ACTION), None, "a refused button is not held for OK");
        rc.cancel();
        let app = rc.app();
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Mixed").unwrap();
        assert_eq!(g.swatches[1], fade_swatch(), "the swatch is kept");
        assert_eq!(group_hex(&app, "Mixed"), Some(vec!["#ff0000".into(), "#00ff00".into()]));
        assert_eq!(undo_len(&app), undo, "no step");
    }

    /// With art, the preview rewrites the group being edited and keeps its other swatch; Save
    /// Changes is still refused, as the group is without the preview, and Cancel leaves it as it was.
    #[test]
    fn saving_a_group_with_a_swatch_that_is_not_a_color_is_refused_while_previewing() {
        let mut app = app(300);
        app.run("swatch.newGroup", json!({"name": "Mixed", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
        let g = doc.swatch_groups.iter_mut().find(|g| g.name == "Mixed").unwrap();
        g.swatches.push(fade_swatch());
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        let mut rc = opened(app, json!({"group": "Mixed"}));
        rc.frame();
        let undo = undo_len(&rc.app());
        let mut rs = rc.rows();
        let row_of = |rs: &[Value], hex: &str| rs.iter().position(|r| from_colors(r).iter().any(|c| c.to_hex() == hex)).unwrap();
        let red_row = row_of(&rs, "#ff0000");
        rs[red_row]["to"] = json!("rgb 0 0 0");
        let blue_to = to_color(&rs[row_of(&rs, "#0000ff")]).unwrap().to_hex();
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.frame();
        assert!(rc.app().session.in_interaction(), "the art previews");
        let previewed = rc.app().session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Mixed").map(|g| g.swatches.clone());
        assert_eq!(previewed.as_ref().map(Vec::len), Some(3), "the preview's rewrite of the group keeps its other swatch");
        assert_eq!(previewed.and_then(|s| s.last().cloned()), Some(fade_swatch()), "with its paint");
        assert_eq!(group_hex(&rc.app(), "Mixed"), Some(vec!["#000000".into(), blue_to]), "each color swatch takes its row's new color");
        assert_eq!(rc.act("save"), not_colors("Mixed"));
        rc.cancel();
        let app = rc.app();
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Mixed").unwrap();
        assert_eq!(g.swatches.last(), Some(&fade_swatch()), "the swatch is kept");
        assert_eq!(group_hex(&app, "Mixed"), Some(vec!["#ff0000".into(), "#0000ff".into()]));
        assert_eq!(undo_len(&app), undo, "no step");
    }

    /// Red and blue squares and the squares of `more`, selected, whose colors are the global swatches
    /// of the group "Linked"; Recolor Artwork open on it with the Exact method, one frame drawn.
    fn linked_setup(more: &[(f64, &str)]) -> (Recolor, Vec<u64>) {
        let mut app = app(300);
        let mut ids = vec![square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff")];
        for (x, fill) in more {
            ids.push(square(&mut app, *x, fill));
        }
        app.run("select.set", json!({"ids": ids})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let mut rc = opened(app, json!({"group": "Linked"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        (rc, ids)
    }

    /// Sets the new color of the rows whose first current color is `hex` to `to`.
    fn set_row(rc: &mut Recolor, hex: &str, to: Value) {
        let mut rs = rc.rows();
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some(hex) {
                r["to"] = to.clone();
            }
        }
        rc.set("rows", Value::Array(rs)).unwrap();
    }

    /// Save Changes on a group whose global swatches the selected art is linked to: the art takes
    /// the group's new colors, and the rows are read from the art again, so they list its colors as
    /// they are now. A color the art had before keeps its row's new color; a color the save gave
    /// it stays as it is.
    #[test]
    fn saving_a_group_the_art_links_to_reads_the_rows_again() {
        let (mut rc, ids) = linked_setup(&[]);
        let red = ids[0];
        set_row(&mut rc, "#ff0000", json!("rgb 0 128 0"));
        rc.act("save").unwrap();
        rc.frame();
        // The rows took the group's colors in their order, so one of the swatches is now green.
        let saved = group_hex(&rc.app(), "Linked").unwrap();
        assert!(saved.len() == 2 && saved.contains(&"#008000".to_string()), "{saved:?}");
        let mut want = saved.clone();
        want.sort();
        assert_eq!(fill_hex(&rc, red), "#008000", "the red square keeps the new color its row gave it");
        for r in rc.rows() {
            if !from_colors(&r).iter().any(|c| ["#ff0000", "#0000ff"].contains(&c.to_hex().as_str())) {
                assert!(excluded(&r) && r["to"] == r["from"][0], "a color the save gave the art stays: {r}");
            }
        }
        let mut froms: Vec<String> = rc.rows().iter().flat_map(from_colors).map(|c| c.to_hex()).collect();
        froms.sort();
        assert_eq!(froms, want, "the rows list the art's colors as they are now");
        assert!(apply_params(&rc.fields()).get("group").is_none(), "OK leaves the saved group as it is");
        // OK with only a new name keeps the preview as one step and renames the group, keeping its
        // saved colors.
        let undo = undo_len(&rc.app());
        rc.set("groupName", json!("Renamed")).unwrap();
        rc.confirm().unwrap();
        assert_eq!(group_hex(&rc.app(), "Renamed"), Some(saved));
        assert_eq!(group_hex(&rc.app(), "Linked"), None);
        assert_eq!(fill_hex(&rc, red), "#008000");
        assert_eq!(undo_len(&rc.app()), undo + 1);
    }

    /// OK with only a new name after Save Changes leaves every swatch of the group its exact color:
    /// a CMYK swatch no row lists keeps components finer than a hundredth of a percent.
    #[test]
    fn renaming_a_saved_group_keeps_its_exact_colors() {
        let mut app = app(300);
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let fine = Color::cmyk(0.123_456, 0.654_321, 0.0, 0.010_101);
        let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
        let g = doc.swatch_groups.iter_mut().find(|g| g.name == "Linked").unwrap();
        g.swatches.push(vectorcraft_color::Swatch { name: "Fine".into(), paint: vectorcraft_color::Paint::solid(fine), global: false, spot: false });
        let mut rc = opened(app, json!({"group": "Linked"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        set_row(&mut rc, "#ff0000", json!("rgb 0 128 0"));
        rc.act("save").unwrap();
        rc.frame();
        let swatches = |rc: &Recolor, name: &str| {
            let app = rc.app();
            let doc = &app.session.doc().unwrap().doc;
            doc.swatch_groups.iter().find(|g| g.name == name).map(|g| g.swatches.iter().map(|w| w.paint.color()).collect::<Vec<_>>())
        };
        let saved = swatches(&rc, "Linked").unwrap();
        assert_eq!(saved.last(), Some(&Some(fine)), "Save Changes keeps the swatch no row lists");
        rc.set("groupName", json!("Renamed")).unwrap();
        rc.confirm().unwrap();
        assert_eq!(swatches(&rc, "Renamed"), Some(saved), "the rename keeps every color exactly");
    }

    /// Save Changes where one row's new color is another row's current color: the art linked to the
    /// group keeps the colors the save gave it, and is not recolored again by the other row; a
    /// Global Adjust set afterwards gives OK the group again.
    #[test]
    fn saving_a_group_keeps_the_colors_it_gave_the_linked_art() {
        let (mut rc, ids) = linked_setup(&[]);
        let mut rs = rc.rows();
        for r in &mut rs {
            let from = from_colors(r).first().map(|c| c.to_hex());
            r["to"] = match from.as_deref() {
                Some("#ff0000") => json!("rgb 0 0 255"),
                _ => json!("rgb 0 128 0"),
            };
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        rc.frame();
        let mut group = group_hex(&rc.app(), "Linked").unwrap();
        group.sort();
        let mut fills = vec![fill_hex(&rc, ids[0]), fill_hex(&rc, ids[1])];
        fills.sort();
        assert_eq!(fills, group, "the art shows the saved group's colors, not a second recolor");
        assert!(apply_params(&rc.fields()).get("group").is_none());
        let mut d = rc.fields();
        d.fields.insert("globalAdjust".into(), json!({"brightness": -50.0}));
        assert!(apply_params(&d).get("group").is_some(), "a new Global Adjust rewrites the group at OK");
    }

    /// Save Changes with art in only some colors of the group: the swatches keep their order, and a
    /// swatch no row lists keeps its color.
    #[test]
    fn saving_a_group_keeps_the_swatches_the_art_does_not_use() {
        let mut app = app(300);
        let red = square(&mut app, 10.0, "#ff0000");
        let blue = square(&mut app, 40.0, "#0000ff");
        let yellow = square(&mut app, 70.0, "#ffff00");
        app.run("select.set", json!({"ids": [red, blue, yellow]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let order = group_hex(&app, "Linked").unwrap();
        app.run("select.set", json!({"ids": [blue, yellow]})).unwrap();
        let mut rc = opened(app, json!({"group": "Linked"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        let mut rs = rc.rows();
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#0000ff") => json!("rgb 0 128 0"),
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        let want: Vec<String> = order.iter().map(|h| if h == "#0000ff" { "#008000".into() } else { h.clone() }).collect();
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(want), "the swatches keep their order and the red one its color");
        assert_eq!(fill_hex(&rc, red), "#ff0000", "art linked to a swatch no row lists is left as it is");
    }

    /// Save Changes with art in colors of the group keeps the Edit tab's changes: a removed color's
    /// swatch is removed, and a color added on the Edit tab joins the group after the others.
    #[test]
    fn saving_a_group_keeps_added_and_removed_colors() {
        let (mut rc, ids) = linked_setup(&[(70.0, "#ffff00")]);
        let order = group_hex(&rc.app(), "Linked").unwrap();
        let mut rs = rc.rows();
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#0000ff") => Value::Null,
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        rs.push(json!({"from": [], "to": "rgb 0 128 0", "added": true}));
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        let mut want: Vec<String> = order.into_iter().filter(|h| h != "#0000ff").collect();
        want.push("#008000".into());
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(want.clone()), "the removed swatch goes and the added color follows the others");
        assert_eq!(fill_hex(&rc, ids[1]), "#0000ff", "art linked to a removed swatch keeps its color");
        let rs = rc.rows();
        assert!(rs.iter().all(|r| !r["to"].is_null() && !is_added(r)), "the rows follow the saved group: {rs:?}");
        let assign = rc.field("__assign").unwrap_or_default();
        let assign: Vec<String> = assign.as_array().cloned().unwrap_or_default().iter().filter_map(color_value).map(|c| c.to_hex()).collect();
        assert_eq!(Some(assign), group_hex(&rc.app(), "Linked"), "a later edit gives the group no removed swatch back");
        rc.confirm().unwrap();
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(want), "OK removes and adds nothing more");
    }

    /// The colors `d`'s OK gives the selected group, as hex, `"keep"` read as the color the swatch
    /// at its place has.
    fn sent_by(app: &VectorcraftApp, d: &Fields) -> Vec<String> {
        let own = group_hex(app, &d.str("group")).unwrap_or_default();
        let p = apply_params(d);
        let hex = |(i, k): (usize, &Value)| if *k == json!("keep") { own.get(i).cloned() } else { color_value(k).map(|c| c.to_hex()) };
        p["groupColors"].as_array().unwrap().iter().enumerate().filter_map(hex).collect()
    }

    /// The colors OK gives the selected group, as [`sent_by`] reads them.
    fn sent_hex(rc: &Recolor) -> Vec<String> {
        let d = rc.fields();
        sent_by(&rc.app(), &d)
    }

    /// Whether Undo Changes (`back`) or Redo Changes can run.
    fn can(rc: &Recolor, back: bool) -> bool {
        can_step(&rc.app(), back)
    }

    /// Save Changes that recolors linked art reads the rows from the art again; a color added on the
    /// Edit tab stays one of the group's colors and a removed one does not come back, so OK after a
    /// further edit keeps them as saved.
    #[test]
    fn saving_a_group_that_reads_the_rows_again_keeps_an_added_color() {
        let (mut rc, ids) = linked_setup(&[(70.0, "#00ff00")]);
        let mut rs = rc.rows();
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#ff0000") => json!("rgb 0 128 0"),
                Some("#00ff00") => Value::Null,
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        rs.push(json!({"from": [], "to": "rgb 255 255 0", "added": true}));
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        assert_eq!(fill_hex(&rc, ids[0]), "#008000", "the save recolored the linked art");
        assert_eq!(fill_hex(&rc, ids[2]), "#00ff00", "art linked to the removed swatch keeps its color");
        let group = group_hex(&rc.app(), "Linked").unwrap();
        assert!(group.contains(&"#ffff00".to_string()) && !group.contains(&"#00ff00".to_string()), "{group:?}");
        let mut d = rc.fields();
        let mut rs = super::rows(&d);
        assert!(rs.iter().all(|r| !is_added(r)), "{rs:?}");
        if let Some(r) = rs.last_mut() {
            r["to"] = json!("rgb 10 10 10");
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        let colors = sent_by(&rc.app(), &d);
        assert_eq!(colors.iter().filter(|c| *c == "#ffff00").count(), 1, "OK after a further edit keeps the added color once: {colors:?}");
        assert!(!colors.contains(&"#00ff00".to_string()), "nor brings the removed color back: {colors:?}");
    }

    /// Save Changes where the rows list their current colors as "#rrggbb": a row matches the swatch
    /// of the same color, so the swatches keep their order and the one a row lists takes its new
    /// color; read again afterwards, a row whose old entry was a hex keeps its new color.
    #[test]
    fn saving_a_group_matches_rows_that_list_hex_colors() {
        let mut app = app(300);
        let red = square(&mut app, 10.0, "#ff0000");
        let blue = square(&mut app, 40.0, "#0000ff");
        let yellow = square(&mut app, 70.0, "#ffff00");
        app.run("select.set", json!({"ids": [red, blue, yellow]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let order = group_hex(&app, "Linked").unwrap();
        let orange = square(&mut app, 100.0, "#ff8000");
        app.run("select.set", json!({"ids": [blue, yellow, orange]})).unwrap();
        let mut rc = opened(app, json!({"group": "Linked"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        let mut rs = rc.rows();
        for r in &mut rs {
            let hex: Vec<String> = from_colors(r).iter().map(|c| c.to_hex()).collect();
            r["to"] = match hex.first().map(String::as_str) {
                Some("#0000ff") => json!("rgb 0 128 0"),
                Some("#ff8000") => json!("rgb 128 0 128"),
                _ => r["from"][0].clone(),
            };
            r["from"] = json!(hex);
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        let want: Vec<String> = order.iter().map(|h| if h == "#0000ff" { "#008000".into() } else { h.clone() }).collect();
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(want), "the blue swatch takes the green, the red one keeps its color");
        assert_eq!((fill_hex(&rc, red), fill_hex(&rc, blue)), ("#ff0000".into(), "#008000".into()));
        let rs = rc.rows();
        let o = rs.iter().find(|r| from_colors(r).iter().any(|c| c.to_hex() == "#ff8000")).unwrap();
        assert!(!excluded(o) && o["to"] == json!("rgb 128 0 128"), "the orange row keeps its new color: {o}");
    }

    /// An app with the color group "Plain" of `colors`, not linked to art, and squares of `art`
    /// selected; Recolor Artwork open on the group with the Exact method, one frame drawn.
    fn plain_setup(colors: &[&str], art: &[&str]) -> (Recolor, Vec<u64>) {
        let mut app = app(300);
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": colors, "keepModel": true})).unwrap();
        let ids: Vec<u64> = art.iter().enumerate().map(|(n, fill)| square(&mut app, 10.0 + 30.0 * n as f64, fill)).collect();
        app.run("select.set", json!({"ids": ids})).unwrap();
        let mut rc = opened(app, json!({"group": "Plain"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        (rc, ids)
    }

    /// Save Changes with art in the group's colors that is not linked to its swatches: the art is
    /// left to the preview, and OK leaves the saved group as it is until the rows change.
    #[test]
    fn saving_a_group_with_unlinked_art_keeps_the_saved_group_at_ok() {
        let (mut rc, ids) = plain_setup(&["#ff0000", "#0000ff"], &["#ff0000", "#0000ff"]);
        let mut rs = rc.rows();
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#0000ff") => json!("rgb 0 128 0"),
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#ff0000".into(), "#008000".into()]));
        rc.frame();
        assert_eq!(fill_hex(&rc, ids[1]), "#008000", "the art previews its row's new color");
        assert!(apply_params(&rc.fields()).get("group").is_none(), "OK leaves the saved group as it is");
    }

    /// With unlinked art in some of the group's colors, a second Save Changes after a further edit of
    /// a row gives the swatch the first save gave that row's color the row's new color, and OK does
    /// too; the swatch no row lists keeps its color both times.
    #[test]
    fn saving_a_group_twice_with_unlinked_art_gives_a_row_its_swatch_again() {
        let (mut rc, _) = plain_setup(&["#ff0000", "#0000ff", "#ffff00"], &["#0000ff", "#ffff00"]);
        set_row(&mut rc, "#0000ff", json!("rgb 0 128 0"));
        set_row(&mut rc, "#ffff00", json!("rgb 255 255 0"));
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#ff0000".into(), "#008000".into(), "#ffff00".into()]));
        rc.frame();
        set_row(&mut rc, "#0000ff", json!("rgb 128 0 128"));
        assert_eq!(sent_hex(&rc), ["#ff0000", "#800080", "#ffff00"], "OK gives the blue row's swatch its new color");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#ff0000".into(), "#800080".into(), "#ffff00".into()]), "the second save does too");
    }

    /// Without art, an excluded row leaves its swatch its own color at Save Changes and at OK, even
    /// when its new color was changed before it was excluded.
    #[test]
    fn saving_a_group_without_art_keeps_an_excluded_rows_color() {
        let mut rc = opened(groups_app(&[("Plain", &["#ff0000", "#0000ff"])]), json!({"group": "Plain"}));
        rc.frame();
        let mut rs = rc.rows();
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some("#ff0000") {
                r["to"] = json!("rgb 0 128 0");
                r["exclude"] = json!(true);
            }
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        assert_eq!(sent_hex(&rc), ["#ff0000", "#0000ff"], "OK keeps the excluded row's swatch");
        let p = apply_params(&rc.fields());
        assert_eq!(p["groupColors"][0], "keep", "so the Global Adjust leaves it as it is too: {p}");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#ff0000".into(), "#0000ff".into()]), "so does Save Changes");
    }

    /// With art in more colors than the group has swatches, an excluded row past the swatches adds
    /// no color at Save Changes, as at OK.
    #[test]
    fn saving_a_group_with_art_adds_no_color_for_an_excluded_row_past_its_swatches() {
        let mut app = app(300);
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000"], "keepModel": true})).unwrap();
        let (blue, yellow) = (square(&mut app, 10.0, "#0000ff"), square(&mut app, 40.0, "#ffff00"));
        app.run("select.set", json!({"ids": [blue, yellow]})).unwrap();
        let mut rc = opened(app, json!({"group": "Plain"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.set("colors", json!(2)).unwrap();
        rc.frame();
        let mut rs = rc.rows();
        assert_eq!(rs.len(), 2, "a row for each color of the art: {rs:?}");
        for r in &mut rs {
            r["exclude"] = json!(true);
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        // `recolor.apply` adds nothing for a "keep" past the group's swatches.
        assert_eq!(apply_params(&rc.fields())["groupColors"], json!(["keep", "keep"]), "OK adds no color");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#ff0000".into()]), "Save Changes adds none either");
    }

    /// Without art, a row excluded after Save Changes gave its swatch a new color leaves the swatch
    /// that saved color at a second Save Changes, as OK does.
    #[test]
    fn saving_a_group_without_art_keeps_a_saved_color_when_its_row_is_excluded() {
        let mut rc = opened(groups_app(&[("Plain", &["#ff0000", "#0000ff"])]), json!({"group": "Plain"}));
        rc.frame();
        set_row(&mut rc, "#ff0000", json!("rgb 0 128 0"));
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#008000".into(), "#0000ff".into()]));
        let mut rs = rc.rows();
        for r in &mut rs {
            if r["to"] == json!("rgb 0 128 0") {
                r["exclude"] = json!(true);
            }
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        assert_eq!(sent_hex(&rc), ["#008000", "#0000ff"], "OK keeps the saved color");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#008000".into(), "#0000ff".into()]), "so does a second Save Changes");
    }

    /// Save Changes that recolors linked art so that one swatch takes another swatch's old color:
    /// read again, the row of the second swatch's new color gives that swatch a later new color.
    #[test]
    fn saving_a_group_twice_with_linked_art_finds_the_row_of_a_swatchs_new_color() {
        let (mut rc, _) = linked_setup(&[]);
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(vec!["#ff0000".into(), "#0000ff".into()]));
        set_row(&mut rc, "#ff0000", json!("rgb 0 0 255"));
        set_row(&mut rc, "#0000ff", json!("rgb 0 128 0"));
        rc.act("save").unwrap();
        rc.frame();
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(vec!["#0000ff".into(), "#008000".into()]));
        let mut rs = rc.rows();
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some("#008000") {
                r["to"] = json!("rgb 128 0 128");
                r["exclude"] = json!(false);
            }
        }
        rc.set("rows", Value::Array(rs)).unwrap();
        assert_eq!(sent_hex(&rc), ["#0000ff", "#800080"], "OK gives the green swatch the green row's new color");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Linked"), Some(vec!["#0000ff".into(), "#800080".into()]), "so does Save Changes");
    }

    /// Save Changes with a Global Adjust: the saved group holds it, the rows' new colors take it and
    /// it goes back to 0%, so OK with a new name renames the group without adjusting it again.
    #[test]
    fn saving_a_group_with_a_global_adjust_applies_it_once() {
        let mut app = app(300);
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        let mut rc = opened(app, json!({"group": "Plain"}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.set("globalAdjust", json!({"brightness": -50.0})).unwrap();
        rc.frame();
        rc.act("save").unwrap();
        let saved = group_hex(&rc.app(), "Plain").unwrap();
        assert!(!saved.contains(&"#ff0000".to_string()), "the save darkened the group: {saved:?}");
        rc.frame();
        assert_eq!(adjust_values(&rc.fields()), [0.0; 4]);
        rc.set("groupName", json!("Dark")).unwrap();
        let p = apply_params(&rc.fields());
        assert!(p.get("globalAdjust").is_none(), "{p}");
        assert_eq!(p["groupColors"], json!(vec!["keep"; saved.len()]), "OK keeps each saved color");
        rc.confirm().unwrap();
        assert_eq!(group_hex(&rc.app(), "Dark"), Some(saved), "OK renames the group with its saved colors");
    }

    /// Without art, Save Changes with a Global Adjust gives the group the rows' new colors with the
    /// adjustment, and OK gives it the same colors again: the adjustment applies to the rows' new
    /// colors, not to the saved group.
    #[test]
    fn saving_a_group_without_art_with_a_global_adjust_applies_it_once() {
        let mut rc = opened(groups_app(&[("Plain", &["#ff0000", "#0000ff"])]), json!({"group": "Plain"}));
        rc.set("globalAdjust", json!({"brightness": -50.0})).unwrap();
        rc.frame();
        rc.act("save").unwrap();
        let saved = group_hex(&rc.app(), "Plain").unwrap();
        assert!(!saved.contains(&"#ff0000".to_string()), "the save darkened the group: {saved:?}");
        rc.confirm().unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain").unwrap(), saved, "OK does not darken the group again");
    }

    /// With Limit to Library, Save Changes gives the group the library's nearest color to a row's
    /// new color, as OK does, also when the rows were set without being snapped.
    #[test]
    fn saving_a_group_snaps_its_colors_to_the_library() {
        let mut app = app(100);
        let doc = &app.session.doc().unwrap().doc;
        let names: Vec<_> = doc
            .swatches
            .iter()
            .filter(|s| !s.is_reserved())
            .map(|s| s.name.clone())
            .chain(doc.swatch_groups.iter().map(|g| g.name.clone()))
            .collect();
        app.run("swatch.delete", json!({"names": names})).unwrap();
        app.run("swatch.new", json!({"name": "Only Yellow", "color": "#ffff00"})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ffff00"], "keepModel": true})).unwrap();
        let mut rc = opened(app, json!({"group": "Plain"}));
        rc.set("limitTo", json!("document")).unwrap();
        rc.frame();
        edit_first(&mut rc, "rgb 250 250 10");
        rc.act("save").unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(vec!["#ffff00".into()]), "the saved color is the library's");
    }

    /// Without art, Save Changes removes a removed color's swatch and adds a color added on the Edit
    /// tab, and OK afterwards gives the group the same colors: it removes and adds nothing more.
    #[test]
    fn saving_a_group_without_art_with_added_and_removed_colors_keeps_them_at_ok() {
        let mut rc = opened(groups_app(&[("Plain", &["#ff0000", "#00ff00", "#0000ff"])]), json!({"group": "Plain"}));
        rc.frame();
        let mut rs = rc.rows();
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some("#00ff00") {
                r["to"] = Value::Null;
            }
        }
        // The added color's row was moved ahead of the others.
        rs.insert(0, json!({"from": [], "to": "rgb 255 255 0", "added": true}));
        rc.set("rows", Value::Array(rs)).unwrap();
        rc.act("save").unwrap();
        let want: Vec<String> = ["#ff0000", "#0000ff", "#ffff00"].map(String::from).to_vec();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(want.clone()), "the save removes green and adds yellow");
        rc.confirm().unwrap();
        assert_eq!(group_hex(&rc.app(), "Plain"), Some(want), "OK gives the group the same colors");
    }

    /// With art: a group picked in the list reduces the art to a row per color of the group, which
    /// become the rows' new colors, and the art previews them; a button set with the group runs on
    /// the group's rows. The name the preview gives the group being edited picks that group, unless
    /// the document has another group of that name. The list hides with `groupStorage`.
    #[test]
    fn a_group_picked_in_the_list_recolors_the_art() {
        let mut app = app(300);
        app.run("swatch.newGroup", json!({"name": "Warm", "colors": ["#ffff00", "#00ff00"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Cool", "colors": ["#0000ff"], "keepModel": true})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        let mut rc = opened(app, json!({}));
        rc.set("method", json!(Method::Exact.id())).unwrap();
        rc.frame();
        assert_eq!(fill_hex(&rc, red), "#ff0000");
        // A button set with the group, before a frame, saves the rows the group gives the art.
        let n = rc.app().session.doc().unwrap().doc.swatch_groups.len();
        rc.set("group", json!("Warm")).unwrap();
        rc.act("new").unwrap();
        let mut saved = last_group_colors(&rc.app());
        saved.sort();
        assert_eq!((rc.app().session.doc().unwrap().doc.swatch_groups.len(), saved), (n + 1, vec!["#00ff00".to_string(), "#ffff00".to_string()]));
        rc.frame();
        assert_eq!(rc.rows().len(), 2);
        assert_eq!(rc.fields().str("groupName"), "Warm");
        let mut shown = [fill_hex(&rc, red), fill_hex(&rc, blue)];
        shown.sort();
        assert_eq!(shown, ["#00ff00", "#ffff00"], "the art previews the group's colors");
        // The name the preview gives the group being edited picks that group in the list.
        rc.set("groupName", json!("Sunny")).unwrap();
        rc.frame();
        rc.set("group", json!("Sunny")).unwrap();
        rc.frame();
        let d = rc.fields();
        assert_eq!((&d.fields["group"], &d.fields[GROUP_LOADED], d.str("groupName")), (&json!("Warm"), &json!("Warm"), "Sunny".into()));
        // A group the document has under that name is picked instead.
        rc.set("groupName", json!("Cool")).unwrap();
        rc.frame();
        rc.set("group", json!("Cool")).unwrap();
        rc.frame().frame();
        let d = rc.fields();
        assert_eq!((&d.fields[GROUP_LOADED], super::rows(&d).len()), (&json!("Cool"), 1));
        rc.settled();
        assert!(rc.by_label(tl!("Color Groups")).is_some(), "the list shows");
        rc.set(GROUP_STORAGE, json!(false)).unwrap();
        rc.settled();
        assert!(rc.by_label(tl!("Color Groups")).is_none(), "and hides with `groupStorage`");
    }
}
