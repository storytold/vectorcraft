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
//! current colors dimmed; a second click shows the art as before), Reset, the Method and Limit to Library (its folder button imports a
//! library file and limits to it; a library's colours apply as its swatches: a spot colour stays a
//! spot colour, and joins the document's swatches). Edit tab: a hue and
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
//! The Color Groups list on the right (under the tabs in a window too narrow for both; the Show
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
//! step back and forward through the dialog's own color edits ([`History`]), not the document's
//! undo history; Cancel discards them and OK still records one undo step. Each opening starts
//! with nothing to undo, and selecting another color group or Save Changes leaves nothing to undo
//! or redo.
//!
//! Fields (`ui.dialog.set`): `rows` ([{from: [keys], to: key, exclude?, tint?: percent of a spot
//! color, added?, swatch?: {name, library} of the global or spot swatch the Color Picker's
//! swatches gave the new color, which links the row's art to it}]; a row with no current colors,
//! `from: []`, recolors no art, and with `added: true`
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
//! `weights`).

use egui::{Rect, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::harmony::{Harmony, move_on_wheel};
use vectorcraft_color::recolor::{ColorKey, GlobalAdjust, Method};
use vectorcraft_color::{Color, keep_model};
use vectorcraft_engine::cmd::color_value;
use vectorcraft_engine::cmd::swatchlib;

use super::{DialogSpec, form};
use crate::panels::c32;
use crate::panels::color::{Mode, components, from_components, track_color};
use crate::panels::swatches::{DOCUMENT_SWATCHES, colour_libraries, limit_key};
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Recolor Artwork.
pub const KIND: &str = "recolor";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Recolor Artwork").into(),
    body,
    confirm,
    preview: true,
    min_width: 540.0,
    max_width: Some(MAIN_W + STORAGE_GAP + STORAGE_W),
    inputs: |_| &[GROUP_ACTION],
    ..DialogSpec::FORM
};

/// Widths of the dialog's main part (the tabs), of the Color Groups list and of the gap between
/// them; the height of a group's row in the list.
const MAIN_W: f32 = 560.0;
const STORAGE_W: f32 = 170.0;
const STORAGE_GAP: f32 = 12.0;
const GROUP_ROW_H: f32 = 34.0;
/// The fields of the Color Groups list: whether it is shown, a request from its buttons, and the
/// group whose colors the rows were last given.
const GROUP_STORAGE: &str = "groupStorage";
const GROUP_ACTION: &str = "groupAction";
const GROUP_LOADED: &str = "__groupLoaded";
/// The `recolor.reduce` parameters the art's rows were last reduced with.
const REDUCED: &str = "__reduced";

const CMD: &str = "recolor.apply";
/// Width of the current colours of a row.
const CURRENT_W: f32 = 250.0;
/// Diameter of the Edit tab's colour wheel.
const WHEEL: f32 = 220.0;
/// Height of the Edit tab's color bars area (the wheel with its Brightness slider), and of a bar.
const BARS_H: f32 = WHEEL + 31.0;
const BAR_H: f32 = 22.0;
/// Without the Prominent Colors bar, the Edit tab's gaps above the wheel and above the sliders.
const WHEEL_GAP: f32 = 8.0;
const SLIDERS_GAP: f32 = 6.0;
/// Height of the rows table, and of either tab (so switching tabs doesn't resize the dialog).
const TABLE_H: f32 = 182.0;
const TAB_H: f32 = 400.0;
const PRESETS: [&str; 5] = ["Custom", "1 Color Job", "2 Color Job", "3 Color Job", "Color Library"];
/// The id of row `i`'s new colour chip, with `i`.
const NEW_CHIP: &str = "recolor-new";
/// The color modes of the sliders under the rows; a row whose new color is a spot color of the
/// library also has Tint.
const SLIDER_MODES: [Mode; 4] = [Mode::Hsb, Mode::Rgb, Mode::Cmyk, Mode::Lab];
/// Widths of a color slider's label, of the slider and of its number field.
const SLIDER_LABEL_W: f32 = 12.0;
const SLIDER_W: f32 = 170.0;
const SLIDER_FIELD_W: f32 = 48.0;
/// The `sliderMode` of the Edit tab's Global Adjust sliders, and the menu's name for it.
const GLOBAL: &str = "global";
const GLOBAL_ADJUST: &str = "Global Adjust";
/// The `globalAdjust` field's keys (and `recolor.apply`'s), in the order of the sliders.
const ADJUSTS: [&str; 4] = ["saturation", "brightness", "temperature", "luminosity"];
/// Widths of a Global Adjust slider's label and of the slider.
const GLOBAL_LABEL_W: f32 = 76.0;
const GLOBAL_SLIDER_W: f32 = 110.0;
/// The slider values last shown: `{row, key, mode, v}` (HSB keeps its hue at no saturation).
const COMPS: &str = "__comps";

/// Open Recolor Artwork (`ui.recolorDialog {colors?, library?, group?}`) on the selected art and,
/// with `group`, on that colour group: art is reduced to as many rows as the group has colours,
/// which become their new colours; without art the rows are the group's colours. `colors` is a
/// count (an n-colour job) or the new colours to assign (without art or a group they are the
/// rows, which OK saves as a new colour group); `library` limits to a library or "document" (""
/// picks the first library).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let art = !st.selection.is_empty();
    let group = p.get("group").and_then(Value::as_str);
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
    // Colours alone (no art, no group) become a new colour group.
    let new_group = !art && group.is_none();
    if new_group && assign.is_empty() {
        return Err("select artwork (or a colour group) to recolor".into());
    }
    let count = match p.get("colors").and_then(Value::as_u64) {
        Some(n) => json!(n.max(1)),
        None if !assign.is_empty() => json!(assign.len()),
        None => Value::Null,
    };
    let limit = match p.get("library").and_then(Value::as_str) {
        Some("") => first_library(app).unwrap_or_default(),
        Some(l) => limit_key(app, l)?,
        None => String::new(),
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
        "preview": true, THEME_PICKER: false, THEME_PICKS: [], WEIGHTS: null, FIND: false, "__art": art, "__assign": assign, "__newGroup": new_group,
        GROUP_STORAGE: true, GROUP_LOADED: group,
    });
    let mut d = Dialog::new(KIND, fields);
    // Without art the rows are there from the start: limit them now (art's rows snap as they reduce).
    let mut rs = rows(&d);
    snap(app, &d, &mut rs);
    d.fields.insert("rows".into(), Value::Array(rs));
    // Undo Changes has nothing to undo on a new opening.
    app.ui.recolor_history = History { opened: Some(snapshot(&d)), ..History::default() };
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

fn key(c: &Color) -> Value {
    json!(ColorKey::of(c).to_string())
}

/// The name a new colour group (opened on colours alone) starts with.
const NEW_GROUP: &str = "Color Group";

fn first_library(app: &VectorcraftApp) -> Option<String> {
    colour_libraries(app).into_iter().next().map(|l| l.id)
}

/// `recolor.reduce` parameters (Method only picks each row's colour, so changing it keeps the rows).
fn reduce_params(d: &Dialog) -> Value {
    json!({
        "colors": d.fields.get("colors").cloned().unwrap_or(Value::Null),
        "preserve": {"white": d.bool("preserveWhite"), "black": d.bool("preserveBlack"), "grays": d.bool("preserveGrays")},
        "limitTo": d.str("limitTo"),
    })
}

/// Reduce the art's colours into rows again when the count, preserve flags or library changed
/// (assigning a group's colours or the opener's colours as the new colours).
fn sync(app: &mut VectorcraftApp, d: &mut Dialog) {
    const DONE: &str = REDUCED;
    let sig = reduce_params(d);
    if !d.bool("__art") || d.fields.get(DONE) == Some(&sig) {
        return;
    }
    // The art's own colors are reduced, not those of the preview, which runs again afterwards.
    if d.fields.remove(super::form::PREVIEWED).is_some() {
        let _ = app.session.cancel_interaction();
    }
    let mut p = sig.clone();
    p["method"] = json!(d.str("method"));
    match app.session.execute("recolor.reduce", &p) {
        Ok(r) => {
            let mut rows = r["map"].as_array().cloned().unwrap_or_default();
            let assign = d.fields.get("__assign").and_then(Value::as_array).cloned().unwrap_or_default();
            for (row, to) in rows.iter_mut().zip(assign) {
                row["to"] = to;
            }
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
    d.fields.remove(WEIGHTS);
    d.fields.remove(WEIGHED);
}

/// The fields of Prominent Colors: the weights the rows are to take (`recolor.weigh`'s `weights`,
/// one per row; a new list moves the rows' current colors), the weights they last took, each row's
/// share of the art's uses (`recolor.weigh`'s `shares`) and the rows' current colors, exclusions
/// and whether each has a new color, which those were read for; a drag of a part's edge (each part's share of the bar while it lasts); and a request
/// from the New Color Group menu (`all` or `prominent`).
const WEIGHTS: &str = "weights";
const WEIGHED: &str = "__weighed";
const SHARES: &str = "__shares";
const SHARES_OF: &str = "__sharesOf";
const BAR_DRAG: &str = "__barDrag";
const SAVE_GROUP: &str = "__saveGroup";
/// Height of the Prominent Colors bar, and the smallest share of it a drag leaves a part. With the
/// bar the Edit tab leaves out [`WHEEL_GAP`] and [`SLIDERS_GAP`], which keeps the tab's height.
const PROMINENT_H: f32 = 10.0;
const MIN_PART: f64 = 0.01;
/// The ids of the bar's part `n` and of the edge after part `k`, with `n` or `k`.
const PROMINENT_PART: &str = "recolor-prominent-part";
const PROMINENT_EDGE: &str = "recolor-prominent-edge";

/// Cancel the preview, so commands read the art's own colors; it runs again afterwards.
fn cancel_preview(app: &mut VectorcraftApp, d: &mut Dialog) {
    if d.fields.remove(super::form::PREVIEWED).is_some() {
        let _ = app.session.cancel_interaction();
    }
}

/// With art: when `weights` changed, move the rows' current colors toward them
/// (`recolor.weigh`); when the rows' current colors changed, read each row's share of the art's
/// uses again.
fn sync_weights(app: &mut VectorcraftApp, d: &mut Dialog) {
    if !d.bool("__art") {
        return;
    }
    if let Some(w) = d.fields.get(WEIGHTS).filter(|w| !w.is_null()).cloned()
        && d.fields.get(WEIGHED) != Some(&w)
    {
        d.fields.insert(WEIGHED.into(), w.clone());
        cancel_preview(app, d);
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
    cancel_preview(app, d);
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
fn prominent_parts(d: &Dialog, rows: &[Value]) -> Vec<(usize, f64)> {
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

/// The Prominent Colors bar: a part per row of [`prominent_parts`] with the row's new color, as wide
/// as its share of the art's uses. A drag of the edge between two parts moves width from one to
/// the other (each keeps at least [`MIN_PART`] of the bar); when it ends, the parts' widths become
/// the rows' `weights`.
fn prominent_bar(ui: &mut Ui, d: &mut Dialog, rows: &[Value]) {
    let t = Tokens::get(ui.ctx());
    let parts = prominent_parts(d, rows);
    let (bar, resp) = ui.allocate_exact_size(vec2(ui.available_width(), PROMINENT_H), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), tl!("Prominent Colors")));
    if parts.is_empty() {
        ui.painter().rect_stroke(bar, 1.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        return;
    }
    let total: f64 = parts.iter().map(|p| p.1).sum();
    let mut widths: Vec<f64> = match d.fields.get(BAR_DRAG).and_then(Value::as_array) {
        Some(w) if w.len() == parts.len() => w.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect(),
        _ => parts.iter().map(|p| p.1 / total).collect(),
    };
    let w = f64::from(bar.width());
    let edge = |k: usize, widths: &[f64]| bar.left() + (widths[..=k].iter().sum::<f64>() * w) as f32;
    let mut left = bar.left();
    for (n, (i, _)) in parts.iter().enumerate() {
        let right = if n + 1 == parts.len() { bar.right() } else { edge(n, &widths) };
        let r = Rect::from_min_max(pos2(left, bar.top()), pos2(right, bar.bottom()));
        let c = to_color(&rows[*i]).or_else(|| from_colors(&rows[*i]).first().copied()).unwrap_or_default();
        chip(ui, r, c, false);
        let share = (widths[n] * 100.0).round();
        ui.interact(r, egui::Id::unique((PROMINENT_PART, n)), Sense::hover())
            .on_hover_text(format!("{} {share}%", crate::i18n::colon(&format!("{} {}", tl!("Color"), i + 1))));
        left = right;
    }
    let mut ended = false;
    for k in 0..parts.len() - 1 {
        let x = edge(k, &widths);
        let handle = Rect::from_center_size(pos2(x, bar.center().y), vec2(8.0, PROMINENT_H));
        let h = ui.interact(handle, egui::Id::unique((PROMINENT_EDGE, k)), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if h.dragged() && w > 0.0 {
            // Each part keeps MIN_PART, or half of the two parts' width when they are narrower.
            let pair = widths[k] + widths[k + 1];
            let least = MIN_PART.min(pair / 2.0);
            let moved = (widths[k] + f64::from(h.drag_delta().x) / w).clamp(least, pair - least);
            widths[k] = moved;
            widths[k + 1] = pair - moved;
            d.fields.insert(BAR_DRAG.into(), json!(widths));
        }
        if h.hovered() || h.dragged() {
            ui.painter().vline(x, bar.y_range(), egui::Stroke::new(2.0, t.accent));
        }
        ended |= h.drag_stopped();
    }
    if ended && d.fields.remove(BAR_DRAG).is_some() {
        let mut weights = vec![0.0; rows.len()];
        for ((i, _), x) in parts.iter().zip(&widths) {
            weights[*i] = (x * 10_000.0).round() / 100.0;
        }
        d.fields.insert(WEIGHTS.into(), json!(weights));
    }
}

/// The colors a new color group takes from rows `rs`: each row's new color with the Global Adjust,
/// snapped to Limit to Library, as `recolor.apply` gives them to a group; a row without a new color
/// gives its first current color, which the art keeps.
fn group_colors<'a>(app: &VectorcraftApp, d: &Dialog, rs: impl Iterator<Item = &'a Value>) -> Vec<Value> {
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
fn save_group(app: &mut VectorcraftApp, d: &mut Dialog, which: &str) -> Result<(), String> {
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
    cancel_preview(app, d);
    app.run("swatch.newGroup", json!({"colors": colors, "keepModel": true})).map(|_| ())
}

/// The colors of the document's color groups, by name, in the Swatches panel's order.
fn doc_groups(app: &VectorcraftApp) -> Vec<(String, Vec<Color>)> {
    let Some(st) = app.session.active() else { return vec![] };
    st.doc.swatch_groups.iter().map(|g| (g.name.clone(), g.swatches.iter().filter_map(|w| w.paint.color()).collect())).collect()
}

/// Make the `group` field's color group (none when it is null) the group being edited, when its
/// colors aren't the rows' yet: with art the art is reduced again to as many rows as the group has
/// colors, which become their new colors (as [`open`] does); without art the rows are its colors.
/// A name no color group has is refused, and the field goes back to the group being edited.
fn select_group(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
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
        && d.fields.contains_key(super::form::PREVIEWED)
    {
        cancel_preview(app, d);
        if !doc_groups(app).iter().any(|(g, _)| g == n) {
            d.fields.insert("group".into(), loaded);
            return Ok(());
        }
    }
    let art = d.bool("__art");
    let name = match &want {
        Value::Null => {
            // No group: without art OK saves the rows as a new color group.
            d.fields.insert(GROUP_LOADED.into(), Value::Null);
            if !art {
                d.fields.insert("__newGroup".into(), json!(true));
                d.fields.insert("groupName".into(), json!(NEW_GROUP));
            }
            restart_history(app, d);
            return Ok(());
        }
        Value::String(n) => n.clone(),
        v => {
            d.fields.insert("group".into(), loaded);
            return Err(format!("`group` must be a color group name, not {v}"));
        }
    };
    // The group's own colors, not those the preview (which rewrites the group being edited) shows.
    if doc_groups(app).iter().any(|(n, _)| *n == name) {
        cancel_preview(app, d);
    }
    let Some((_, colors)) = doc_groups(app).into_iter().find(|(n, _)| *n == name) else {
        d.fields.insert("group".into(), loaded);
        return Err(format!("no colour group `{name}`"));
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
    restart_history(app, d);
    Ok(())
}

/// Run a button of the Color Groups list: `save` rewrites the selected group with the rows' new
/// colors as OK gives them to it (with art, followed by the group's colors no row took) and the
/// name in `groupName`; `new` saves the new colors of the rows that aren't excluded as a new group;
/// `delete` removes the selected group and its swatches. Each is an undo step of its own on the
/// document, not on the preview, which runs again afterwards.
fn group_action(app: &mut VectorcraftApp, d: &mut Dialog, action: &Value) -> Result<(), String> {
    check_group_action(app, d, action)?;
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
                restart_history(app, d);
                return Ok(());
            }
            cancel_preview(app, d);
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
            restart_history(app, d);
            Ok(())
        }
        Some("save") => {
            let rs = rows(d);
            let p = apply_params(d);
            // With art, the group's colors no row took follow, adjusted and snapped as OK gives them,
            // then the colors added on the Edit tab; a row made with New Row that holds no current
            // colors gives the group no color.
            cancel_preview(app, d);
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
            restart_history(app, d);
            Ok(())
        }
        Some("delete") => {
            cancel_preview(app, d);
            app.run("swatch.delete", json!({"names": [group]}))?;
            d.fields.insert("group".into(), Value::Null);
            select_group(app, d)
        }
        _ => Ok(()),
    }
}

/// The color Save Changes gives the group's swatch for `row`: `null` when the row's color was
/// removed, which removes the swatch as OK does, otherwise its new color as OK gives it.
fn saved_color(app: &VectorcraftApp, d: &Dialog, row: &Value) -> Option<Value> {
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
fn reread_rows(app: &mut VectorcraftApp, d: &mut Dialog, old: &[Value], before: &[Vec<Value>], gone: &[Value]) {
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

/// Give each row's new color the Global Adjust (and Limit to Library) as OK would, and set the
/// Global Adjust back to 0%; nothing changes when it is 0%.
fn bake_adjust(app: &VectorcraftApp, d: &mut Dialog) {
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
    d.fields.insert("globalAdjust".into(), Value::Object(ADJUSTS.iter().map(|k| (k.to_string(), json!(0.0))).collect()));
}

/// After Save Changes with art: until the rows or the Global Adjust change, OK and the preview
/// leave the selected group's colors as they were saved.
fn mark_saved(app: &VectorcraftApp, d: &mut Dialog) {
    let group = d.fields.get("group").and_then(Value::as_str).map(str::to_owned);
    let saved: Vec<Value> =
        doc_groups(app).into_iter().find(|(n, _)| group.as_deref() == Some(n.as_str())).map(|g| g.1.iter().map(key).collect()).unwrap_or_default();
    d.fields.insert(SAVED_COLORS.into(), Value::Array(saved));
    d.fields.insert(SAVED_ROWS.into(), Value::Array(rows(d)));
    d.fields.insert(SAVED_ADJUST.into(), json!(adjust_values(d)));
}

/// Whether the rows and the Global Adjust are as [`mark_saved`] recorded them.
fn unchanged_since_save(d: &Dialog) -> bool {
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
fn swatch_row<'a>(d: &Dialog, rs: &'a [Value], own: &[Value], i: usize) -> Option<(Value, &'a Value)> {
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
/// swatch): the rows hold only the group's colors, so the save would replace or remove it.
fn check_group_action(app: &VectorcraftApp, d: &Dialog, action: &Value) -> Result<(), String> {
    let group = d.fields.get("group").and_then(Value::as_str);
    match (action.as_str(), group) {
        (Some("new"), _) => Ok(()),
        (Some("save"), Some(g)) if holds_other_paints(app, g) => {
            Err(format!("color group `{g}` holds swatches that are not colors, which Save Changes can't keep"))
        }
        (Some("save" | "delete"), Some(_)) => Ok(()),
        (Some("save" | "delete"), None) => Err("missing `group`".into()),
        _ => Err(format!("`{GROUP_ACTION}` must be \"save\", \"new\" or \"delete\", not {action}")),
    }
}

/// Whether color group `group` has a swatch that is not a color, in the document as it is without
/// the preview (which can rewrite or rename the group).
fn holds_other_paints(app: &VectorcraftApp, group: &str) -> bool {
    let Some(st) = app.session.active() else { return false };
    let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
    doc.swatch_groups.iter().any(|g| g.name == group && g.swatches.iter().any(|w| w.paint.color().is_none()))
}

/// Run the button of the Color Groups list that `groupAction` holds, if any.
fn run_group_action(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    match d.fields.remove(GROUP_ACTION) {
        Some(a) => group_action(app, d, &a),
        None => Ok(()),
    }
}

/// `ui.dialog.set` of `group`, `groupStorage` or `groupAction`: select the group, or check the
/// button, which the next frame (or OK) runs.
pub(super) fn field_set(app: &mut VectorcraftApp, field: &str) -> Result<(), String> {
    let Some(mut d) = app.ui.dialog.take() else { return Ok(()) };
    let r = match field {
        "group" => select_group(app, &mut d),
        GROUP_ACTION => match d.fields.get(GROUP_ACTION).cloned() {
            Some(a) => check_group_action(app, &d, &a),
            None => Ok(()),
        },
        GROUP_STORAGE if !d.fields.get(GROUP_STORAGE).is_some_and(Value::is_boolean) => Err(format!("`{GROUP_STORAGE}` must be true or false")),
        _ => Ok(()),
    };
    app.ui.dialog = Some(d);
    r
}

/// The Color Groups list: a row per color group of the document with its name and colors (a click
/// selects it), and the Save Changes, New Color Group and Delete buttons under it.
fn group_storage(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog, max_h: f32) {
    let t = Tokens::get(ui.ctx());
    let groups = doc_groups(app);
    let mut current = d.fields.get("group").and_then(Value::as_str).map(str::to_owned);
    // While the art previews, the group being edited shows under its new name.
    if current.is_some() && d.fields.contains_key(super::form::PREVIEWED) && !groups.iter().any(|(n, _)| Some(n) == current.as_ref()) {
        current = Some(d.str("groupName"));
    }
    let mut picked = None;
    egui::ScrollArea::vertical().id_salt("recolor-groups").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        for (name, colors) in &groups {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_ROW_H), Sense::click());
            let on = current.as_deref() == Some(name.as_str());
            if on {
                ui.painter().rect_filled(r, 2.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 2.0, t.hover);
            }
            let inner = r.shrink2(vec2(4.0, 3.0));
            ui.painter().text(inner.left_top(), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(12.0), t.text);
            let fit = ((inner.width() + 2.0) / 12.0).floor().max(0.0) as usize;
            for (n, c) in colors.iter().take(fit).enumerate() {
                let x = inner.left() + n as f32 * 12.0;
                chip(ui, Rect::from_min_size(pos2(x, inner.bottom() - 10.0), vec2(10.0, 10.0)), *c, false);
            }
            let resp = resp.on_hover_text(name);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, ui.is_enabled(), on, name));
            if resp.clicked() {
                picked = Some(name.clone());
            }
        }
    });
    if let Some(name) = picked {
        d.fields.insert("group".into(), json!(name));
        if let Err(e) = select_group(app, d) {
            app.status(e);
        }
    }
    ui.add_space(4.0);
    let selected = current.is_some();
    let any = rows(d).iter().any(|r| !excluded(r));
    let mut action = None;
    ui.horizontal(|ui| {
        if widgets::icon_button_enabled(ui, "save", tl!("Save Changes"), false, selected, 24.0).clicked() {
            action = Some("save");
        }
        if widgets::icon_button_enabled(ui, "dc-folder", tl!("New Color Group"), false, any, 24.0).clicked() {
            action = Some("new");
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete"), false, selected, 24.0).clicked() {
            action = Some("delete");
        }
    });
    if let Some(a) = action {
        d.fields.insert(GROUP_ACTION.into(), json!(a));
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
fn sync_theme(app: &mut VectorcraftApp, d: &mut Dialog) {
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
    if d.fields.remove(super::form::PREVIEWED).is_some() {
        let _ = app.session.cancel_interaction();
    }
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

/// While the Color Theme Picker is on: a layer over the canvas, under the dialog, takes the clicks
/// and drags there as picks (with Shift they add to the picks so far, else they replace them) and
/// draws a drag's marquee, whose sides follow the document's axes.
fn theme_overlay(app: &VectorcraftApp, ctx: &egui::Context, d: &mut Dialog) {
    let (Some(rect), Some(view)) = (app.canvas_rect, app.view()) else { return };
    let xf = crate::canvas::Xf::at(rect, view, app.canvas_ui_zoom);
    let accent = Tokens::get(ctx).accent;
    let id = egui::Id::unique("recolor-theme-picker");
    let shown = egui::Area::new(id).order(egui::Order::Middle).fixed_pos(rect.min).movable(false).show(ctx, |ui| {
        let resp = ui.allocate_rect(rect, Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Crosshair);
        let shift = ui.input(|i| i.modifiers.shift);
        // Where a press on the canvas began, kept from the first frame that sees it for the frame
        // that releases it (the pointer forgets it then).
        let from = egui::Id::unique("recolor-theme-drag");
        if let Some(o) = ui.input(|i| i.pointer.press_origin()).filter(|o| rect.contains(*o)) {
            ui.data_mut(|m| m.insert_temp(from, o));
        }
        if resp.clicked() {
            ui.data_mut(|m| m.remove::<egui::Pos2>(from));
            let p = xf.to_doc(resp.interact_pointer_pos().or(ui.input(|i| i.pointer.latest_pos()))?);
            return Some((json!({"at": [p.x, p.y]}), shift));
        }
        if !(resp.dragged() || resp.drag_stopped()) {
            return None;
        }
        let a = ui.data(|m| m.get_temp::<egui::Pos2>(from))?;
        let b = ui.input(|i| i.pointer.latest_pos());
        let (a, b) = (xf.to_doc(a), xf.to_doc(b?));
        if resp.drag_stopped() {
            ui.data_mut(|m| m.remove::<egui::Pos2>(from));
            return Some((json!({"rect": [a.x, a.y, b.x, b.y]}), shift));
        }
        let corners = [(a.x, a.y), (b.x, a.y), (b.x, b.y), (a.x, b.y), (a.x, a.y)];
        let pts: Vec<egui::Pos2> = corners.iter().map(|&(x, y)| xf.to_screen(vectorcraft_geom::Point::new(x, y))).collect();
        ui.painter().with_clip_rect(rect).extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(1.0, accent), 4.0, 3.0));
        None
    });
    // Above the backdrop of the dialog (which a click elsewhere raises), still under its window.
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
    let Some((pick, add)) = shown.inner else { return };
    let mut picks = if add { d.fields.get(THEME_PICKS).and_then(Value::as_array).cloned().unwrap_or_default() } else { vec![] };
    picks.push(pick);
    let extra = picks.len().saturating_sub(MAX_PICKS);
    picks.drain(..extra);
    d.fields.insert(THEME_PICKS.into(), Value::Array(picks));
}

/// With Limit to Library, the rows' new colours snap to the library's nearest colours.
fn snap(app: &VectorcraftApp, d: &Dialog, rows: &mut [Value]) {
    let Some(palette) = Some(d.str("limitTo")).filter(|l| !l.is_empty()).and_then(|l| swatchlib::limit_palette(&app.session, &l)) else { return };
    for r in rows {
        if let Some(c) = to_color(r) {
            r["to"] = key(&palette.nearest(c));
        }
    }
}

fn rows(d: &Dialog) -> Vec<Value> {
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
        r.remove(SWATCH);
    }
}

/// A row's `swatch`: the swatch its new color was picked from (`recolor.apply`'s row `swatch`).
const SWATCH: &str = "swatch";

/// Set for the frame in which Randomly change color order shuffled the rows' new colors.
const SHUFFLED: &str = "__shuffled";

/// Give each row of `rows` whose current colors and new color are those of the same row of
/// `before` that row's tint, model and swatch again, when it has none: a change that snaps back to
/// the same new color keeps the tint of it, the model a slider edit gave it and the swatch it was
/// picked from.
fn keep_tints(before: &[Value], rows: &mut [Value]) {
    for (r, b) in rows.iter_mut().zip(before) {
        if r["from"] != b["from"] || r["to"] != b["to"] {
            continue;
        }
        for k in ["tint", "model", SWATCH] {
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
fn next_seed(d: &Dialog) -> u64 {
    (d.f64("__seed", 0.0) as u64).saturating_add(1)
}

fn selection(d: &Dialog) -> Vec<usize> {
    d.fields.get("__sel").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|i| i as usize).collect()).unwrap_or_default()
}

/// `recolor.apply` parameters: the rows, method, library and the group with its new name and
/// colors: the new colors of the rows with current colors, each in its swatch's place (`null`
/// for a row whose new color was removed, which removes its swatch, and `"keep"` for an excluded
/// row, which leaves its swatch its color), then (with art) the group's colors no row took, then the
/// colors added on the Edit tab. With art in a color of the group, each swatch whose row
/// ([`swatch_row`]) is found takes that row's new color instead, and every other swatch keeps its
/// color, as Save Changes gives them.
fn apply_params(d: &Dialog) -> Value {
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
fn find_params(d: &Dialog) -> Option<Value> {
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

/// The tabs, and the Color Groups list on their right while it is shown.
fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    if let Err(e) = select_group(app, d) {
        app.status(e);
    }
    let room = ui.available_width();
    // Without the room for both side by side (a narrow window), the list goes under the tabs.
    let beside = room >= SPEC.min_width + STORAGE_GAP + STORAGE_W;
    // The Color Picker that a double-click on a new color or Pick from Swatches asks for opens over
    // the dialog after the list is drawn, so the window keeps its size while the picker is open.
    let mut picker = None;
    if !d.bool(GROUP_STORAGE) || !beside {
        ui.vertical(|ui| {
            ui.set_width(room.min(MAIN_W));
            picker = main(app, ui, d);
            if d.bool(GROUP_STORAGE) {
                ui.add_space(6.0);
                group_storage(app, ui, d, 3.0 * GROUP_ROW_H);
            }
        });
    } else {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((room - STORAGE_GAP - STORAGE_W).clamp(0.0, MAIN_W));
                picker = main(app, ui, d);
            });
            ui.add_space(STORAGE_GAP);
            ui.vertical(|ui| {
                ui.set_width(STORAGE_W);
                group_storage(app, ui, d, TAB_H - 30.0);
            });
        });
    }
    if let Some((i, swatches)) = picker
        && let Some(row) = rows(d).get(i)
    {
        let c = to_color(row).or_else(|| from_colors(row).first().copied()).unwrap_or_default();
        *d = super::color_picker::over_recolor(d, c, i, swatches);
    }
    false
}

/// The tabs and the controls under them. Returns the row whose new color a double-click or Pick
/// from Swatches asked to pick in the Color Picker, with `true` for Pick from Swatches.
fn main(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> Option<(usize, bool)> {
    seed(app, d);
    sync(app, d);
    sync_theme(app, d);
    sync_weights(app, d);
    // The fields as the dialog read the art are where Undo Changes starts, also when the pointer is
    // down on the first frame and the first step is recorded only on release.
    app.ui.recolor_history.start(d);
    if let Err(e) = run_group_action(app, d) {
        app.status(e);
    }
    let t = Tokens::get(ui.ctx());
    let picking = d.bool(THEME_PICKER);
    let mut stepped = None;
    ui.horizontal(|ui| {
        let tab = d.str("tab");
        for (id, label) in [("assign", tl!("Assign")), ("edit", tl!("Edit"))] {
            if ui.selectable_label(tab == id || (id == "assign" && tab != "edit"), egui::RichText::new(label).font(theme::semibold(12.5))).clicked() {
                d.fields.insert("tab".into(), json!(id));
            }
        }
        ui.add_space(8.0);
        if widgets::icon_button(ui, "pipette", tl!("Color Theme Picker"), picking, 24.0).clicked() {
            d.fields.insert(THEME_PICKER.into(), json!(!picking));
        }
        let h = &app.ui.recolor_history;
        for (back, icon, tip) in [(true, "undo-2", tl!("Undo Changes")), (false, "redo-2", tl!("Redo Changes"))] {
            if widgets::icon_button_enabled(ui, icon, tip, false, h.can(d, back), 24.0).clicked() {
                stepped = Some(back);
            }
        }
        let storage = d.bool(GROUP_STORAGE);
        if widgets::icon_button(ui, "library", tl!("Show Color Groups"), storage, 24.0).clicked() {
            d.fields.insert(GROUP_STORAGE.into(), json!(!storage));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The label is drawn after the menu (right-to-left), so it cannot name it by adjacency.
            if let Some(i) = widgets::dropdown_named(ui, "recolor-preset", Some(tl!("Preset:")), PRESETS[preset_of(d)], &PRESETS, 130.0) {
                set_preset(app, d, i);
            }
            ui.label(egui::RichText::new(tl!("Preset:")).color(t.text_dim));
        });
    });
    if let Some(back) = stepped {
        app.ui.recolor_history.step(d, back);
    }
    widgets::divider(ui);
    ui.add_space(4.0);
    if d.bool(THEME_PICKER) {
        widgets::dim_label(ui, tl!("Click or drag across the art to pick its colors; Shift adds more"));
        ui.add_space(4.0);
        theme_overlay(app, ui.ctx(), d);
        sync_theme(app, d);
    }
    let mut rs = rows(d);
    let top = ui.cursor().top();
    let (changed, pick) = if d.str("tab") == "edit" { edit_tab(app, ui, d, &mut rs) } else { assign_tab(app, ui, d, &mut rs) };
    if d.str("tab") != "edit" {
        ui.add_space((TAB_H - (ui.cursor().top() - top)).max(0.0));
    }
    if changed {
        snap(app, d, &mut rs);
        // A shuffle by `recolor.randomize` has already moved each tint with its color.
        if d.fields.remove(SHUFFLED).is_none() {
            keep_tints(&rows(d), &mut rs);
        }
        d.fields.insert("rows".into(), Value::Array(rs));
    }
    sync_weights(app, d);
    if let Some(which) = d.fields.remove(SAVE_GROUP)
        && let Err(e) = save_group(app, d, which.as_str().unwrap_or_default())
    {
        app.status(e);
    }
    if d.fields.get("group").is_some_and(Value::is_string) || d.bool("__newGroup") {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Color Group:"));
            form::text(ui, d, "groupName", 200.0);
        });
    }
    sync(app, d);
    // New rows take the picks now, so that a new count of Colors is one step.
    sync_theme(app, d);
    // A change is a step once the pointer is up: a drag is one step from press to release.
    if !ui.input(|i| i.pointer.any_down()) {
        app.ui.recolor_history.note(d);
    }
    if d.bool("__art") {
        match find_params(d) {
            // Find shows the art so with Preview off too.
            Some(p) => {
                ui.add_space(8.0);
                form::preview_check(ui, d);
                form::preview_now(app, d, "Recolor Artwork", "command.batch", p);
            }
            None => form::preview(app, ui, d, "Recolor Artwork", CMD, apply_params(d)),
        }
    }
    pick
}

/// `d` with row `i`'s new colour set to `c` (the Color Picker's OK), snapped to Limit to Library.
pub(super) fn with_new_color(app: &VectorcraftApp, mut d: Dialog, i: usize, c: Color) -> Dialog {
    let mut rs = rows(&d);
    if let Some(r) = rs.get_mut(i) {
        set_to(r, &c);
        snap(app, &d, &mut rs);
        keep_tints(&rows(&d), &mut rs);
        d.fields.insert("rows".into(), Value::Array(rs));
    }
    d
}

/// `d` with row `i`'s new color set to color `c` of a swatch picked from the Color Picker's
/// swatches: for a global or spot swatch, `swatch` (`{name, library}`), which the row keeps, so
/// `recolor.apply` links its art to it, unless Limit to Library snaps the color to another one
/// (colors compare as [`ColorKey`]s, as the row holds them); for another swatch, `None`, which
/// takes away a swatch picked for the row before, and without Limit to Library the tint it gave.
pub(super) fn with_new_swatch(app: &VectorcraftApp, d: Dialog, i: usize, c: Color, swatch: Option<Value>) -> Dialog {
    let mut d = with_new_color(app, d, i, c);
    let keeps_tint = !d.str("limitTo").is_empty();
    let mut rs = rows(&d);
    let Some(r) = rs.get_mut(i) else { return d };
    match swatch {
        Some(w) if to_color(r).is_some_and(|t| ColorKey::of(&t) == ColorKey::of(&c)) => r[SWATCH] = w,
        Some(_) => return d,
        None => {
            let Some(o) = r.as_object_mut() else { return d };
            if o.remove(SWATCH).is_none() {
                return d;
            }
            if !keeps_tint {
                o.remove("tint");
            }
        }
    }
    d.fields.insert("rows".into(), Value::Array(rs));
    d
}

/// The fields a step of Undo Changes and Redo Changes restores: what the art is recolored with.
const STEP_FIELDS: [&str; 11] =
    ["rows", "colors", "method", "preserveWhite", "preserveBlack", "preserveGrays", "limitTo", "rule", "globalAdjust", THEME_PICKS, WEIGHTS];
/// The fields that record which reduction, picks and weights the rows already took, restored with
/// [`STEP_FIELDS`] so that the restored rows are not reduced, picked or weighed again.
const STEP_MARKS: [&str; 4] = ["__reduced", THEME_DONE, WEIGHED, PRESERVED];
/// VectorCraft's own limits on the steps Undo Changes keeps: at most this many, and at most this
/// many bytes of them as JSON; the oldest go first.
const MAX_STEPS: usize = 64;
const MAX_STEP_BYTES: usize = 4 << 20;

type Snapshot = serde_json::Map<String, Value>;

/// The open dialog's own history of color edits: Undo Changes goes back one edit and Redo Changes
/// forward one, without touching the document's undo history. A change of [`STEP_FIELDS`] is one
/// step once the pointer is up, so a drag is one step; a new edit after Undo Changes ends what Redo
/// Changes could restore.
#[derive(Clone, Debug, Default)]
pub struct History {
    /// The fields the dialog opened with (or had when its steps were dropped), before it read the
    /// art, until its first frame or command.
    opened: Option<Snapshot>,
    /// The fields after the last step (none before the dialog's first frame).
    base: Option<Snapshot>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

/// Drop the dialog's steps, after another color group is selected or Save Changes rewrites the
/// group: Undo Changes and Redo Changes then have no step to go to. The fields of `d` are where
/// Undo Changes starts again, as on a new opening, so fields set before the next frame or step
/// (with `ui.dialog.set`) are a step back to them.
fn restart_history(app: &mut VectorcraftApp, d: &Dialog) {
    app.ui.recolor_history = History { opened: Some(snapshot(d)), ..History::default() };
}

/// The size of `s` as JSON, which [`MAX_STEP_BYTES`] limits.
fn snapshot_size(s: &Snapshot) -> usize {
    serde_json::to_vec(s).map_or(0, |b| b.len())
}

/// The fields of `d` a step restores ([`STEP_FIELDS`] and [`STEP_MARKS`]).
fn snapshot(d: &Dialog) -> Snapshot {
    STEP_FIELDS.iter().chain(&STEP_MARKS).filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect()
}

/// Before the first step is recorded: when fields were set after the dialog opened (with
/// `ui.dialog.set`, before a frame drew it), the fields it opened with, as they read the art, are
/// the step Undo Changes goes back to.
fn seed(app: &mut VectorcraftApp, d: &Dialog) {
    let Some(opened) = app.ui.recolor_history.opened.take() else { return };
    if app.ui.recolor_history.base.is_some() || snapshot(d) == opened {
        return;
    }
    let mut first = d.clone();
    first.fields.remove(super::form::PREVIEWED);
    for k in STEP_FIELDS.iter().chain(&STEP_MARKS) {
        match opened.get(*k) {
            Some(v) => first.fields.insert(k.to_string(), v.clone()),
            None => first.fields.remove(*k),
        };
    }
    sync(app, &mut first);
    sync_theme(app, &mut first);
    app.ui.recolor_history.base = Some(snapshot(&first));
}

impl History {
    /// Take the fields of `d` as those after the last step when no step has them yet.
    fn start(&mut self, d: &Dialog) {
        if self.base.is_none() {
            self.base = Some(snapshot(d));
        }
    }

    /// Whether the fields of `d` differ from those after the last step.
    fn pending(&self, d: &Dialog) -> bool {
        self.base.as_ref().is_some_and(|b| STEP_FIELDS.iter().any(|k| d.fields.get(*k) != b.get(*k)))
    }

    /// Record a change of `d`'s fields since the last step as a step.
    fn note(&mut self, d: &Dialog) {
        if self.base.is_some() && !self.pending(d) {
            return;
        }
        if let Some(before) = self.base.replace(snapshot(d)) {
            self.undo.push(before);
            self.redo.clear();
            self.trim();
        }
    }

    /// Drop the oldest steps past [`MAX_STEPS`] and [`MAX_STEP_BYTES`], keeping the newest step
    /// even when it and the current fields alone pass the byte limit.
    fn trim(&mut self) {
        let extra = self.undo.len().saturating_sub(MAX_STEPS);
        self.undo.drain(..extra);
        let mut total: usize = self.base.iter().chain(&self.undo).map(snapshot_size).sum();
        let mut gone = 0;
        while total > MAX_STEP_BYTES && gone + 1 < self.undo.len() {
            total -= snapshot_size(&self.undo[gone]);
            gone += 1;
        }
        self.undo.drain(..gone);
    }

    fn can(&self, d: &Dialog, back: bool) -> bool {
        // Before the first frame or command, fields set since the dialog opened are a step back,
        // except rows and weights that the art's reduction is about to replace or drop ([`sync`]).
        if self.base.is_none()
            && let Some(opened) = &self.opened
        {
            let reduces = d.bool("__art") && d.fields.get("__reduced") != Some(&reduce_params(d));
            let kept = |k: &&&str| !(reduces && (**k == "rows" || **k == WEIGHTS));
            return back && STEP_FIELDS.iter().filter(kept).any(|k| d.fields.get(*k) != opened.get(*k));
        }
        if back { !self.undo.is_empty() || self.pending(d) } else { !self.redo.is_empty() && !self.pending(d) }
    }

    /// Undo Changes (`back`) or Redo Changes on `d`: give it the fields of the step before or after.
    /// Returns false when there is none.
    fn step(&mut self, d: &mut Dialog, back: bool) -> bool {
        self.note(d);
        let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(s) = from.pop() else { return false };
        to.extend(self.base.take());
        for k in STEP_FIELDS.iter().chain(&STEP_MARKS) {
            match s.get(*k) {
                Some(v) => d.fields.insert(k.to_string(), v.clone()),
                None => d.fields.remove(*k),
            };
        }
        // The rows hold no slider edit or drag, and the selection keeps to them.
        for k in [COMPS, DRAG, BAR_DRAG, SHUFFLED] {
            d.fields.remove(k);
        }
        let n = rows(d).len();
        d.fields.insert("__sel".into(), json!(selection(d).into_iter().filter(|i| *i < n).collect::<Vec<_>>()));
        self.base = Some(s);
        true
    }
}

/// Undo Changes (`ui.recolorUndo`, `back`) or Redo Changes (`ui.recolorRedo`) in the open Recolor
/// Artwork dialog → {changed, canUndo, canRedo}.
pub(crate) fn step(app: &mut VectorcraftApp, back: bool) -> Result<Value, String> {
    if app.ui.dialog.as_ref().is_none_or(|d| d.kind != KIND) {
        return Err("Recolor Artwork is not open".into());
    }
    let Some(mut d) = app.ui.dialog.take() else { return Err("Recolor Artwork is not open".into()) };
    // Fields set since the last frame take effect first, as the next frame would give them.
    seed(app, &d);
    sync(app, &mut d);
    sync_theme(app, &mut d);
    sync_weights(app, &mut d);
    let mut h = std::mem::take(&mut app.ui.recolor_history);
    let changed = h.step(&mut d, back);
    let r = json!({"changed": changed, "canUndo": h.can(&d, true), "canRedo": h.can(&d, false)});
    app.ui.recolor_history = h;
    // The art previews the step at once, as the frame after a button would preview it.
    if changed && d.bool("__art") {
        sync_weights(app, &mut d);
        match find_params(&d) {
            Some(p) => {
                let _ = form::preview_now(app, &mut d, "Recolor Artwork", "command.batch", p);
            }
            None => {
                let p = apply_params(&d);
                form::preview_without_check(app, &mut d, "Recolor Artwork", CMD, p);
            }
        }
    }
    app.ui.dialog = Some(d);
    Ok(r)
}

/// Whether Undo Changes (`back`) or Redo Changes can run: Recolor Artwork is open with a step to go to.
pub(crate) fn can_step(app: &VectorcraftApp, back: bool) -> bool {
    app.ui.dialog.as_ref().filter(|d| d.kind == KIND).is_some_and(|d| app.ui.recolor_history.can(d, back))
}

/// OK: keep the previewed recolour as one undo step (or apply it, e.g. to a colour group only; or
/// save colours opened alone as a new colour group).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    select_group(app, &mut d)?;
    sync(app, &mut d);
    sync_theme(app, &mut d);
    sync_weights(app, &mut d);
    // A Color Groups button set with OK in one request runs first, once: the open dialog takes
    // its result, so an OK that fails afterwards does not run it again.
    if d.fields.contains_key(GROUP_ACTION) {
        let r = run_group_action(app, &mut d);
        if let Some(live) = app.ui.dialog.as_mut().filter(|x| x.kind == KIND) {
            *live = d.clone();
        }
        r?;
    }
    if d.bool("__newGroup") {
        let colors = group_colors(app, &d, rows(&d).iter().filter(|r| in_group(r) && to_color(r).is_some()));
        let r = form::commit_preview(app, "swatch.newGroup", json!({"name": d.str("groupName"), "colors": colors, "keepModel": true}))?;
        app.ui.recolor_history = History::default();
        return Ok(r);
    }
    let r = form::commit_preview(app, CMD, apply_params(&d))?;
    app.ui.recolor_history = History::default();
    // The Preserve checkboxes (shown with art) open as confirmed here next time (#2960).
    if d.bool("__art") {
        app.ui.recolor_preserve =
            json!({"preserveWhite": d.bool("preserveWhite"), "preserveBlack": d.bool("preserveBlack"), "preserveGrays": d.bool("preserveGrays")});
    }
    Ok(r)
}

/// The preset the fields match: an n-colour job (n rows, Scale Tints), a library, or Custom.
fn preset_of(d: &Dialog) -> usize {
    match d.fields.get("colors").and_then(Value::as_u64) {
        Some(n @ 1..=3) if d.str("method") == Method::ScaleTints.id() => n as usize,
        _ if !d.str("limitTo").is_empty() => 4,
        _ => 0,
    }
}

fn set_preset(app: &VectorcraftApp, d: &mut Dialog, i: usize) {
    match i {
        1..=3 => {
            d.fields.insert("colors".into(), json!(i));
            d.fields.insert("method".into(), json!(Method::ScaleTints.id()));
        }
        4 if d.str("limitTo").is_empty() => {
            d.fields.insert("limitTo".into(), json!(first_library(app).unwrap_or_default()));
        }
        _ => {}
    }
}

/// A colour chip with a hairline frame.
fn chip(ui: &Ui, r: Rect, c: Color, dim: bool) {
    let t = Tokens::get(ui.ctx());
    let fill = c32(&c);
    ui.painter().rect_filled(r, 1.0, if dim { fill.gamma_multiply(0.35) } else { fill });
    ui.painter().rect_stroke(r, 1.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
}

/// The Assign tab. Returns true when the rows changed, and the row whose new colour was
/// double-clicked or chosen via "Pick from Swatches" (with `true` for swatches view).
fn assign_tab(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog, rows: &mut Vec<Value>) -> (bool, Option<(usize, bool)>) {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let total: usize = rows.iter().map(|r| r["from"].as_array().map_or(0, Vec::len)).sum();
    // Reduction and the preserve rules apply to art (a colour group alone has a row per colour).
    if d.bool("__art") {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Colors:")).color(t.text_dim));
            let labels: Vec<String> = std::iter::once(tl!("Auto").to_string()).chain((1..=total).map(|n| n.to_string())).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let cur = d.fields.get("colors").and_then(Value::as_u64).map_or(tl!("Auto").to_string(), |n| n.to_string());
            if let Some(i) = widgets::dropdown_named(ui, "recolor-count", Some(tl!("Colors:")), &cur, &refs, 70.0) {
                d.fields.insert("colors".into(), if i == 0 { Value::Null } else { json!(i) });
            }
            ui.add_space(18.0);
            ui.label(egui::RichText::new(tl!("Preserve:")).color(t.text_dim));
            for (k, label) in [("preserveWhite", tl!("White")), ("preserveBlack", tl!("Black")), ("preserveGrays", tl!("Grays"))] {
                let on = d.bool(k);
                if widgets::check(ui, label, on, true) {
                    d.fields.insert(k.into(), json!(!on));
                }
            }
        });
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.add_sized(
            [CURRENT_W, 16.0],
            egui::Label::new(
                egui::RichText::new(crate::i18n::fmt(tl!("Current Colors ({total})"), &[("total", &total.to_string())])).color(t.text_dim),
            ),
        );
        ui.add_space(60.0);
        ui.label(egui::RichText::new(tl!("New")).color(t.text_dim));
    });
    let mut sel = selection(d);
    let mut clicked: Option<usize> = None;
    let mut pick: Option<(usize, bool)> = None;
    let mut limit_to: Option<String> = None;
    let links = RowLinks::new(app, d);
    let shown: Vec<Color> = rows.iter().map(|r| new_shown(app, d, &links, r)).collect();
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("recolor-rows").max_height(TABLE_H).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(TABLE_H);
            if rows.is_empty() {
                ui.add_space(8.0);
                widgets::dim_label(ui, tl!("No colors to recolor (preserved colors are left out)."));
            }
            for (i, row) in rows.iter_mut().enumerate() {
                let bg = ui.painter().add(egui::Shape::Noop);
                let off = excluded(row);
                let r = ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let from = from_colors(row);
                    let (cur, cr) = ui.allocate_exact_size(vec2(CURRENT_W, 20.0), Sense::click());
                    let w = CURRENT_W / from.len().max(1) as f32;
                    for (j, c) in from.iter().enumerate() {
                        let x = cur.left() + j as f32 * w;
                        chip(ui, Rect::from_min_max(pos2(x, cur.top()), pos2(x + w - if w > 4.0 { 2.0 } else { 0.0 }, cur.bottom())), *c, false);
                    }
                    // A row made with New Row shows an empty slot until colors are merged into it.
                    if from.is_empty() {
                        ui.painter().rect_stroke(cur, 1.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    }
                    if cr.clicked() {
                        clicked = Some(i);
                    }
                    let (ar, aresp) = ui.allocate_exact_size(vec2(44.0, 20.0), Sense::click());
                    arrow(ui, ar, !off, aresp.hovered());
                    if aresp.on_hover_text(if off { tl!("Recolor this row") } else { tl!("Keep this row's colors (exclude)") }).clicked() {
                        row["exclude"] = json!(!off);
                        changed = true;
                    }
                    let (nr, _) = ui.allocate_exact_size(vec2(46.0, 20.0), Sense::hover());
                    let nresp = ui.interact(nr, egui::Id::unique((NEW_CHIP, i)), Sense::click());
                    let to = to_color(row).unwrap_or_default();
                    // A row whose new color was removed shows an empty frame and an empty hex field.
                    let has_new = to_color(row).is_some();
                    if has_new {
                        chip(ui, nr, shown.get(i).copied().unwrap_or_default(), off);
                    } else {
                        ui.painter().rect_stroke(nr, 1.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    }
                    if nresp.clicked() {
                        clicked = Some(i);
                    }
                    if nresp.double_clicked() && has_new {
                        pick = Some((i, false));
                    }
                    ui.add_space(6.0);
                    let hex = if has_new { crate::panels::color::hex_digits(&to) } else { String::new() };
                    let hex_name = format!("{} {}", tl!("Color"), i + 1);
                    if let Some(h) = widgets::hex_field_named(ui, ("recolor-hex", i), Some(&hex_name), &hex)
                        && let Some(c) = crate::panels::color::parse_hex(&h)
                    {
                        set_to(row, &keep_model(to, c));
                        changed = true;
                    }
                    let mut row_menu = |ui: &mut Ui| {
                        if widgets::menu_item(ui, tl!("Pick from Swatches"), true, false) {
                            pick = Some((i, true));
                            ui.close();
                        }
                        ui.menu_button(tl!("Limit to Library"), |ui| {
                            let cur = d.str("limitTo");
                            if widgets::menu_item(ui, tl!("None"), true, cur.is_empty()) {
                                limit_to = Some("".to_string());
                                ui.close();
                            }
                            ui.separator();
                            if let Some(k) = crate::panels::swatches::library_list(app, ui, &cur) {
                                limit_to = Some(k);
                                ui.close();
                            }
                        });
                    };
                    if (cr.secondary_clicked() || nresp.secondary_clicked()) && !sel.contains(&i) {
                        clicked = Some(i);
                    }
                    cr.context_menu(&mut row_menu);
                    nresp.context_menu(&mut row_menu);
                    let empty_w = ui.available_width();
                    if empty_w > 0.0 {
                        let (_, empty_resp) = ui.allocate_exact_size(vec2(empty_w, 20.0), Sense::click());
                        if empty_resp.clicked() {
                            clicked = Some(i);
                        }
                        if empty_resp.secondary_clicked() && !sel.contains(&i) {
                            clicked = Some(i);
                        }
                        empty_resp.context_menu(&mut row_menu);
                    }
                });
                if sel.contains(&i) {
                    ui.painter().set(bg, egui::Shape::rect_filled(r.response.rect.expand2(vec2(2.0, 1.0)), 2.0, t.row_selected));
                }
            }
        });
    });
    if let Some(k) = limit_to {
        d.fields.insert("limitTo".into(), json!(k));
        changed = true;
    }
    if let Some(i) = clicked {
        let m = ui.input(|i| i.modifiers);
        match sel.iter().position(|s| *s == i) {
            Some(p) if m.shift || m.command => {
                sel.remove(p);
            }
            None if m.shift || m.command => sel.push(i),
            _ => sel = vec![i],
        }
        d.fields.insert("__sel".into(), json!(sel));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // New Row is available only with art.
        if widgets::icon_button_enabled(ui, "plus", tl!("New Row"), false, d.bool("__art"), 24.0).clicked() {
            let at = new_row(rows, &sel);
            d.fields.insert("__sel".into(), json!([at]));
            // The slider edit was kept for a row index that now names another row.
            d.fields.remove(COMPS);
            changed = true;
        }
        let can_merge = sel.len() > 1;
        if widgets::icon_button_enabled(ui, "combine", tl!("Merge colors into one row"), false, can_merge, 24.0).clicked() {
            merge(rows, &sel);
            d.fields.insert("__sel".into(), json!([sel.iter().min()]));
            changed = true;
        }
        let can_split = sel.iter().any(|i| rows.get(*i).is_some_and(|r| r["from"].as_array().is_some_and(|a| a.len() > 1)));
        if widgets::icon_button_enabled(ui, "ungroup", tl!("Separate colors into rows"), false, can_split, 24.0).clicked() {
            separate(rows, &sel);
            d.fields.insert("__sel".into(), json!([]));
            changed = true;
        }
        ui.add_space(12.0);
        for (icon, tip, sb) in
            [("arrow-left-right", tl!("Randomly change color order"), false), ("sparkles", tl!("Randomly change saturation and brightness"), true)]
        {
            if widgets::icon_button_enabled(ui, icon, tip, false, rows.len() > 1 || sb, 24.0).clicked() {
                let seed = next_seed(d);
                d.fields.insert("__seed".into(), json!(seed));
                let p = json!({"map": rows, "order": !sb, "saturationBrightness": sb, "seed": seed});
                match app.session.execute("recolor.randomize", &p) {
                    Ok(r) => {
                        let mut shuffled = r["map"].as_array().cloned().unwrap_or_default();
                        keep_marks(rows, &mut shuffled);
                        *rows = shuffled;
                        if !sb {
                            d.fields.insert(SHUFFLED.into(), json!(true));
                        }
                        changed = true;
                    }
                    Err(e) => app.status(e.to_string()),
                }
            }
        }
        let finding = d.bool(FIND);
        if widgets::icon_button_enabled(ui, "search", tl!("Find the selected colors in the artwork"), finding, d.bool("__art"), 24.0).clicked() {
            d.fields.insert(FIND.into(), json!(!finding));
        }
        if widgets::icon_button(ui, "rotate-ccw", tl!("Reset the rows to the artwork's colors"), false, 24.0).clicked() {
            d.fields.remove("__reduced");
            d.fields.remove(COMPS);
            // The rows keep no slider edit, also when they cannot be reduced again.
            let saved = d.fields.get_mut("rows").and_then(Value::as_array_mut).into_iter().flatten();
            for r in rows.iter_mut().chain(saved).filter_map(Value::as_object_mut) {
                r.remove("model");
            }
        }
        let target = SliderTarget::of(app, d, rows);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| mode_menu(ui, d, &target, false));
    });
    ui.add_space(6.0);
    let row = first_selected(d, rows);
    changed |= sliders(app, ui, d, rows, row);
    ui.add_space(8.0);
    egui::Grid::new("recolor-options").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label(egui::RichText::new(tl!("Method:")).color(t.text_dim));
        let m = Method::parse(&d.str("method")).unwrap_or_default();
        let labels: Vec<&str> = Method::ALL.iter().map(|m| m.label()).collect();
        let chosen = ui.name_inputs(tl!("Method:"), |ui| widgets::dropdown(ui, "recolor-method", m.label(), &labels, 180.0));
        if let Some(i) = chosen {
            d.fields.insert("method".into(), json!(Method::ALL[i].id()));
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Limit to Library:")).color(t.text_dim));
        // None, the document's swatches, then the colour libraries: the built-in ones (at the top
        // level of the library menu) translated, those the user saved or loaded by their names.
        let libs = colour_libraries(app);
        let keys: Vec<&str> = ["", swatchlib::DOCUMENT_SWATCHES].into_iter().chain(libs.iter().map(|l| l.id.as_str())).collect();
        let names: Vec<&str> = ["None", DOCUMENT_SWATCHES].into_iter().chain(libs.iter().map(|l| l.name.as_str())).collect();
        let builtin = |k: usize| k.checked_sub(2).is_none_or(|i| libs.get(i).is_some_and(|l| l.submenu.is_none()));
        let shown = super::shown_names(crate::i18n::current(), &names, builtin);
        let cur = d.str("limitTo");
        let at = keys.iter().position(|k| *k == cur).unwrap_or(0);
        let current = shown.get(at).copied().unwrap_or_default();
        ui.horizontal(|ui| {
            let library = ui.name_inputs(tl!("Limit to Library:"), |ui| widgets::dropdown_names(ui, "recolor-library", current, &shown, 180.0));
            if let Some(k) = library.and_then(|i| keys.get(i)) {
                d.fields.insert("limitTo".into(), json!(k));
            }
            // Import a library (.ase, .acb, .gpl, .vcswatches or a document's swatches) and limit to it.
            if app.services.pick_open.is_some() && widgets::icon_button(ui, "folder-open", tl!("Other Library…"), false, 24.0).clicked() {
                let r = crate::picks::in_dialog(app, d, |app, d| {
                    let filters = crate::FilePick::filters(vectorcraft_engine::cmd::fileio::builtin_open_filters());
                    let pick = crate::FilePick { filters, ..Default::default() };
                    let Some(path) = crate::picks::open(app, &pick) else { return Ok(()) };
                    let r = app.run("swatch.library.load", json!({ "path": path }))?;
                    d.fields.insert("limitTo".into(), r["library"].clone());
                    Ok(())
                });
                if let Err(e) = r {
                    app.status(e);
                }
            }
        });
        ui.end_row();
    });
    (changed, pick)
}

/// The arrow between a row's current and new colours: solid when the row is recoloured, a dashed
/// line without a head when it is excluded.
fn arrow(ui: &Ui, r: Rect, on: bool, hovered: bool) {
    let t = Tokens::get(ui.ctx());
    let c = if hovered {
        t.text_strong
    } else if on {
        t.text
    } else {
        t.text_disabled
    };
    let (a, b) = (pos2(r.left() + 6.0, r.center().y), pos2(r.right() - 6.0, r.center().y));
    let stroke = egui::Stroke::new(1.2, c);
    if on {
        ui.painter().line_segment([a, b], stroke);
        ui.painter().line_segment([b, b + vec2(-5.0, -4.0)], stroke);
        ui.painter().line_segment([b, b + vec2(-5.0, 4.0)], stroke);
    } else {
        ui.painter().extend(egui::Shape::dashed_line(&[a, b], stroke, 3.0, 3.0));
    }
}

/// The tint (0..1) `row` gives its spot color (1 without one).
fn row_tint(row: &Value) -> f32 {
    row["tint"].as_f64().map_or(1.0, |t| (t / 100.0).clamp(0.0, 1.0) as f32)
}

/// Tint `t` (0..1) of spot color `c` as art linked to it shows it: a Lab spot color is the active
/// document's working-CMYK equivalent while the Spot Colors options use CMYK values
/// ([`vectorcraft_doc::Document::linked_color`]), and its inks scale.
fn spot_tinted(app: &VectorcraftApp, c: Color, t: f32) -> Color {
    app.session.active().map_or(c, |st| st.doc.linked_color(c, true)).tinted(t)
}

/// The color row `row`'s new color chip shows: what the art takes, at the row's tint (a spot color
/// of the library as [`spot_tinted`] shows it).
fn new_shown(app: &VectorcraftApp, d: &Dialog, links: &RowLinks, row: &Value) -> Color {
    let to = to_color(row).unwrap_or_default();
    // Only a Lab spot color can show another color; the library lookup is skipped for the rest.
    if matches!(to, Color::Lab { .. }) && d.bool("__art") && linked_spot(app, d, links, row, &to).is_some() {
        spot_tinted(app, to, row_tint(row))
    } else if row_tint(row) < 1.0 && tint_applies(app, d, links, row, &to) {
        to.tinted(row_tint(row))
    } else {
        // The swatch the tint was for no longer links the art, which takes the full color.
        to
    }
}

/// The `sliderMode` id of `m` (`hsb`, `rgb`, `cmyk`, `lab`).
fn mode_id(m: Mode) -> String {
    serde_json::to_value(m).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// What decides how `recolor.apply` links the art of the rows to swatches, read once for all rows:
/// Limit to Library's palette and color swatches, the Global Adjust, and the names of the color
/// group's global swatches that OK's rewrite gives another color ([`rewritten_to_another`]).
struct RowLinks {
    library: Option<(vectorcraft_color::recolor::Palette, Vec<vectorcraft_color::swatch::Swatch>)>,
    adjust: GlobalAdjust,
    rewritten: Vec<String>,
}

impl RowLinks {
    fn new(app: &VectorcraftApp, d: &Dialog) -> Self {
        let lib = d.str("limitTo");
        let library = if lib.is_empty() { None } else { limit_palette_before_preview(app, &lib).zip(limit_swatches(app, &lib)) };
        let rewritten = rewritten_to_another(app, d, library.as_ref().map(|(palette, _)| palette));
        Self { library, adjust: global_adjust(adjust_values(d)), rewritten }
    }

    /// The library swatch (Limit to Library) that `recolor.apply` gives new color `c`: the first
    /// swatch of the library color nearest `c` after the Global Adjust.
    fn library_swatch(&self, c: &Color) -> Option<&vectorcraft_color::swatch::Swatch> {
        let (palette, swatches) = self.library.as_ref()?;
        let near = palette.nearest(self.adjust.apply(*c));
        swatches.iter().find(|w| w.paint.color() == Some(near))
    }
}

/// Limit to Library's palette for `lib`, as [`swatchlib::limit_palette`] gives it, with the
/// document's swatches read as they were before the preview, which `recolor.apply` runs on.
fn limit_palette_before_preview(app: &VectorcraftApp, lib: &str) -> Option<vectorcraft_color::recolor::Palette> {
    if lib != swatchlib::DOCUMENT_SWATCHES {
        return swatchlib::limit_palette(&app.session, lib);
    }
    let st = app.session.active()?;
    let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
    Some(vectorcraft_color::recolor::Palette::new(doc.swatches_iter().filter_map(|w| w.paint.color())))
}

/// The color swatches of Limit to Library's `lib`, as [`swatchlib::library_swatches`] gives them,
/// with the document's swatches read as they were before the preview.
fn limit_swatches(app: &VectorcraftApp, lib: &str) -> Option<Vec<vectorcraft_color::swatch::Swatch>> {
    if lib != swatchlib::DOCUMENT_SWATCHES {
        return swatchlib::library_swatches(&app.session, lib);
    }
    let st = app.session.active()?;
    let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
    Some(doc.swatches_iter().filter(|w| matches!(w.paint, vectorcraft_color::Paint::Solid { swatch: None, .. })).cloned().collect())
}

/// The name of the spot swatch the art of `row`, with new color `c`, links to: Limit to Library's
/// swatch of that color when the library has one (a process swatch there links nothing, and the
/// row's own `swatch` is not used, as in `recolor.apply`), else the row's own spot swatch.
fn linked_spot(app: &VectorcraftApp, d: &Dialog, links: &RowLinks, row: &Value, c: &Color) -> Option<String> {
    match links.library_swatch(c) {
        Some(w) => (w.spot && !link_rewritten(app, links, w)).then(|| w.name.clone()),
        None => own_spot(app, d, links, row, c),
    }
}

/// The name of the spot swatch `row`'s new color `c` was picked from (its `swatch`), while that
/// swatch has color `c`, no Global Adjust applies without Limit to Library and OK's rewrite of the
/// color group gives it no other color ([`link_rewritten`]), as `recolor.apply` reads it.
fn own_spot(app: &VectorcraftApp, d: &Dialog, links: &RowLinks, row: &Value, c: &Color) -> Option<String> {
    if d.str("limitTo").is_empty() && !global_adjust(adjust_values(d)).is_identity() {
        return None;
    }
    row_swatch(app, row, c).filter(|w| w.spot && !link_rewritten(app, links, w)).map(|w| w.name)
}

/// Whether the art `recolor.apply` links to swatch `w` (a library's or the document's) links nothing
/// because OK's rewrite of the color group gives it another color: `w` joins the document under its
/// own name only when the document's swatch of that name has its paint, Global and spot, and is
/// added under a free name otherwise, which no swatch of the group has.
fn link_rewritten(app: &VectorcraftApp, links: &RowLinks, w: &vectorcraft_color::swatch::Swatch) -> bool {
    if !links.rewritten.contains(&w.name) {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
    doc.swatch(&w.name).is_some_and(|x| x.paint == w.paint && x.global == w.global && x.spot == w.spot)
}

/// The names of the color group's global swatches that OK's rewrite (`groupColors` of
/// [`apply_params`]) gives a color other than their own, as `recolor.apply` reads it: each color
/// takes the Global Adjust and, with Limit to Library, the library's nearest color (`palette`);
/// `"keep"` and a removed color give none. Art whose row picked such a swatch links nothing.
fn rewritten_to_another(app: &VectorcraftApp, d: &Dialog, palette: Option<&vectorcraft_color::recolor::Palette>) -> Vec<String> {
    let p = apply_params(d);
    let (Some(group), Some(colors)) = (p["group"].as_str(), p["groupColors"].as_array()) else { return Vec::new() };
    let Some(st) = app.session.active() else { return Vec::new() };
    let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
    let adjust = global_adjust(adjust_values(d));
    let Some(g) = doc.swatch_groups.iter().find(|g| g.name == group) else { return Vec::new() };
    g.swatches
        .iter()
        .filter_map(|w| Some((w, w.paint.color()?)))
        .zip(colors)
        .filter(|((w, own), v)| {
            let new = color_value(v).filter(|_| v.as_str() != Some("keep")).map(|c| adjust.apply(c));
            let new = new.map(|c| palette.map_or(c, |pl| pl.nearest(c)));
            w.global && !w.is_reserved() && new.is_some_and(|c| doc.linked_color(c, w.spot) != doc.linked_color(*own, w.spot))
        })
        .map(|((w, _), _)| w.name.clone())
        .collect()
}

/// The name and library key of `row`'s `swatch` when it is `{name, library?}` with a name, as
/// `recolor.apply` reads it.
fn swatch_ref(row: &Value) -> Option<(&str, &str)> {
    let w = row.get(SWATCH)?;
    let name = w.get("name")?.as_str().filter(|n| !n.is_empty())?;
    let library = match w.get("library") {
        None | Some(Value::Null) => swatchlib::DOCUMENT_SWATCHES,
        Some(l) => l.as_str().filter(|l| !l.is_empty())?,
    };
    Some((name, library))
}

/// The swatch `row`'s `swatch` names, with color `c` (as a color key), when it is a global or spot
/// swatch; `None` when it is a process swatch or not there (deleted since the pick). Of several
/// swatches with that name, the first with that color counts, as `recolor.apply` finds it. The
/// document's swatches are read as they were before the preview, which `recolor.apply` runs on.
fn row_swatch(app: &VectorcraftApp, row: &Value, c: &Color) -> Option<vectorcraft_color::swatch::Swatch> {
    let (name, lib) = swatch_ref(row)?;
    let fits = |s: &&vectorcraft_color::swatch::Swatch| {
        s.name == name
            && matches!(s.paint, vectorcraft_color::Paint::Solid { swatch: None, .. })
            && s.paint.color().is_some_and(|w| ColorKey::of(&w) == ColorKey::of(c))
    };
    let found = if lib == swatchlib::DOCUMENT_SWATCHES {
        let st = app.session.active()?;
        let doc = st.interaction.as_ref().map_or(&st.doc, |it| &it.doc);
        doc.swatches_iter().find(fits).cloned()
    } else {
        let (_, l) = swatchlib::library(&app.session, lib)?;
        l.swatches.iter().chain(l.groups.iter().flat_map(|g| g.swatches.iter())).find(fits).cloned()
    };
    found.filter(|s| s.global || s.spot)
}

/// Whether the new color chip shows `row`'s tint: with Limit to Library unless the library's swatch
/// of `c` is one OK's rewrite of the color group gives another color ([`link_rewritten`]), when the
/// row has no `swatch`, or when its `swatch` links the art ([`row_swatch`], with no Global Adjust,
/// and not [`link_rewritten`]). A row whose `swatch` links nothing recolors its art with the full
/// color.
fn tint_applies(app: &VectorcraftApp, d: &Dialog, links: &RowLinks, row: &Value, c: &Color) -> bool {
    if !d.str("limitTo").is_empty() {
        return links.library_swatch(c).is_none_or(|w| !link_rewritten(app, links, w));
    }
    swatch_ref(row)
        .is_none_or(|_| global_adjust(adjust_values(d)).is_identity() && row_swatch(app, row, c).is_some_and(|w| !link_rewritten(app, links, &w)))
}

/// The mode the sliders show new color `c` in, `None` for Tint: the mode picked in the menu, else
/// `c`'s own model (HSB for a gray), or Tint for a spot color (`spot`).
fn slider_mode(d: &Dialog, c: &Color, spot: bool) -> Option<Mode> {
    let picked = d.str("sliderMode");
    match SLIDER_MODES.into_iter().find(|m| mode_id(*m) == picked) {
        Some(m) => Some(m),
        None if spot => None,
        None => Some(match Mode::of(c) {
            Mode::Grayscale => Mode::Hsb,
            m => m,
        }),
    }
}

/// The values the sliders show for row `i`'s new color `c` in `mode`: those they set last while
/// the color is the one they set, else `c`'s.
fn slider_values(d: &Dialog, rows: &[Value], i: usize, c: &Color, mode: Mode) -> Vec<f32> {
    let held = d.fields.get(COMPS).filter(|s| s["row"] == json!(i) && s["key"] == rows[i]["to"] && s["mode"] == json!(mode_id(mode)));
    let v: Option<Vec<f32>> = held.and_then(|s| s["v"].as_array()).map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect());
    v.filter(|v| v.len() == mode.labels().len()).unwrap_or_else(|| components(mode, c))
}

/// The first selected row of `rows`.
fn first_selected(d: &Dialog, rows: &[Value]) -> Option<usize> {
    selection(d).into_iter().filter(|i| *i < rows.len()).min()
}

/// What the color sliders edit: a row (the first selected row on the Assign tab, the base color's
/// row on the Edit tab), its new color, and the name of the library's spot color that color is.
struct SliderTarget {
    row: Option<usize>,
    color: Option<Color>,
    spot: Option<String>,
}

impl SliderTarget {
    fn of(app: &VectorcraftApp, d: &Dialog, rows: &[Value]) -> Self {
        Self::at(app, d, rows, first_selected(d, rows))
    }

    /// What the color sliders edit when they edit row `row`.
    fn at(app: &VectorcraftApp, d: &Dialog, rows: &[Value], row: Option<usize>) -> Self {
        let row = row.filter(|i| *i < rows.len());
        let color = row.and_then(|i| to_color(&rows[i]));
        // A row tint acts only on art, so without art a spot color shows the process modes.
        let spot = color.filter(|_| d.bool("__art")).and_then(|c| linked_spot(app, d, &RowLinks::new(app, d), &rows[row?], &c));
        Self { row, color, spot }
    }

    /// The mode the sliders show (`None`: Tint).
    fn mode(&self, d: &Dialog) -> Option<Mode> {
        slider_mode(d, &self.color.unwrap_or_default(), self.spot.is_some())
    }
}

/// The color mode menu of the sliders (HSB, RGB, CMYK, Lab, Tint for a spot color, and with
/// `global` Global Adjust), drawn right to left: the menu, then its label.
fn mode_menu(ui: &mut Ui, d: &mut Dialog, target: &SliderTarget, global: bool) {
    let t = Tokens::get(ui.ctx());
    let mut labels: Vec<&str> = SLIDER_MODES.iter().map(|m| m.label()).collect();
    if target.spot.is_some() {
        labels.push("Tint");
    }
    if global {
        labels.push(GLOBAL_ADJUST);
    }
    let adjusting = global && d.str("sliderMode") == GLOBAL;
    let current = if adjusting { GLOBAL_ADJUST } else { target.mode(d).map_or("Tint", Mode::label) };
    // The label is drawn after the menu (right-to-left), so it cannot name it by adjacency.
    let chosen = ui
        .add_enabled_ui(target.color.is_some() || global, |ui| {
            widgets::dropdown_named(ui, "recolor-slider-mode", Some(tl!("Color Mode:")), current, &labels, if global { 110.0 } else { 90.0 })
        })
        .inner;
    if let Some(i) = chosen {
        let id = match SLIDER_MODES.get(i) {
            Some(m) => mode_id(*m),
            None if labels.get(i) == Some(&GLOBAL_ADJUST) => GLOBAL.to_string(),
            None => "tint".to_string(),
        };
        d.fields.insert("sliderMode".into(), json!(id));
    }
    ui.label(egui::RichText::new(tl!("Color Mode:")).color(t.text_dim));
}

/// The Global Adjust amounts, in percent from -100 to 100, of `ADJUSTS`' keys in the
/// `globalAdjust` field (0 for one it lacks).
fn adjust_values(d: &Dialog) -> [f64; 4] {
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
fn global_sliders(ui: &mut Ui, d: &mut Dialog, sample: Color) {
    let labels = [tl!("Saturation"), tl!("Brightness"), tl!("Temperature"), tl!("Luminosity")];
    let values = adjust_values(d);
    let mut set: Option<(usize, f64)> = None;
    egui::Grid::new("recolor-global-adjust").num_columns(6).spacing([6.0, 4.0]).show(ui, |ui| {
        for (n, lbl) in labels.iter().enumerate() {
            let shown = widgets::SliderValue::field(values[n], -100.0..=100.0, "%", 0);
            // The rail: `sample` with this slider's amount alone.
            let track = |x: f32| {
                let mut one = [0.0; 4];
                one[n] = f64::from(x) * 200.0 - 100.0;
                c32(&global_adjust(one).apply(sample))
            };
            if let Some(v) = slider_cell(ui, ("global", n), lbl, shown, "%", &track, GLOBAL_LABEL_W, GLOBAL_SLIDER_W) {
                set = Some((n, v));
            }
            if n % 2 == 1 {
                ui.end_row();
            }
        }
    });
    if let Some((n, v)) = set {
        let mut values = values;
        values[n] = v;
        d.fields.insert("globalAdjust".into(), Value::Object(ADJUSTS.iter().zip(values).map(|(k, v)| (k.to_string(), json!(v))).collect()));
    }
}

/// One color slider in the grid: its label `label_w` wide, its rail with `track` `slider_w` wide
/// and its number field, showing `shown` with `suffix`. Returns the whole number in `shown`'s
/// range that a drag, a key or the field sets.
#[allow(clippy::too_many_arguments)]
pub(crate) fn slider_cell(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy,
    label: &str,
    shown: widgets::SliderValue,
    suffix: &str,
    track: &dyn Fn(f32) -> egui::Color32,
    label_w: f32,
    slider_w: f32,
) -> Option<f64> {
    let t = Tokens::get(ui.ctx());
    let (lo, hi) = (*shown.range.start(), *shown.range.end());
    let value = shown.value;
    ui.add_sized(vec2(label_w, 22.0), egui::Label::new(egui::RichText::new(label).size(12.5).color(t.text)));
    let unit = ((value - lo) / (hi - lo)) as f32;
    let (dragged, _, keyed) = widgets::color_slider_stepped(ui, ("recolor-slider", id), label, Some(shown), unit, slider_w, track);
    let typed = widgets::plain_field_named(ui, ("recolor-slider-field", id), Some(label), value, suffix, 0, SLIDER_FIELD_W);
    dragged.map(|x| (lo + f64::from(x) * (hi - lo)).round()).or(keyed).or(typed).map(|v| v.clamp(lo, hi)).filter(|v| (v - value).abs() > 1e-6)
}

/// The color sliders, two to a line: they edit row `row`'s new color (the others keep theirs) in
/// the mode of [`mode_menu`], or its tint of a spot color of the library. Without a row they are
/// disabled. Returns true when they changed a row.
fn sliders(app: &VectorcraftApp, ui: &mut Ui, d: &mut Dialog, rows: &mut [Value], row: Option<usize>) -> bool {
    let target = SliderTarget::at(app, d, rows, row);
    let mode = target.mode(d);
    let mut changed = false;
    let top = ui.cursor().top();
    let lines = mode.map_or(1, |m| m.labels().len().div_ceil(2));
    ui.add_enabled_ui(target.color.is_some(), |ui| {
        egui::Grid::new("recolor-sliders").num_columns(6).spacing([6.0, 4.0]).show(ui, |ui| {
            let (Some(i), Some(c)) = (target.row, target.color) else {
                // Nothing to edit: the mode's sliders, disabled.
                let mode = mode.unwrap_or(Mode::Hsb);
                let input = Tokens::get(ui.ctx()).input;
                let gray = move |_: f32| input;
                for (n, lbl) in mode.labels().iter().enumerate() {
                    let shown = widgets::SliderValue::field(0.0, f64::from(mode.min(n))..=f64::from(mode.max(n)), mode.suffix(n), 0);
                    slider_cell(ui, n, lbl, shown, mode.suffix(n), &gray, SLIDER_LABEL_W, SLIDER_W);
                    if n % 2 == 1 {
                        ui.end_row();
                    }
                }
                return;
            };
            let Some(mode) = mode else {
                // Tint of the spot color, and its name.
                let tint = f64::from(row_tint(&rows[i])) * 100.0;
                let shown = widgets::SliderValue::field(tint, 0.0..=100.0, "%", 0);
                if let Some(v) = slider_cell(ui, usize::MAX, "T", shown, "%", &|x| c32(&spot_tinted(app, c, x)), SLIDER_LABEL_W, SLIDER_W) {
                    rows[i]["tint"] = json!(v);
                    changed = true;
                }
                ui.label(egui::RichText::new(target.spot.as_deref().unwrap_or_default()).color(Tokens::get(ui.ctx()).text));
                return;
            };
            let comps = slider_values(d, rows, i, &c, mode);
            let mut set: Option<Vec<f32>> = None;
            for (n, lbl) in mode.labels().iter().enumerate() {
                let v = comps.get(n).copied().unwrap_or(0.0);
                let shown = widgets::SliderValue::field(f64::from(v), f64::from(mode.min(n))..=f64::from(mode.max(n)), mode.suffix(n), 0);
                if let Some(nv) = slider_cell(ui, n, lbl, shown, mode.suffix(n), &|x| c32(&track_color(mode, &comps, n, x)), SLIDER_LABEL_W, SLIDER_W)
                {
                    let mut c2 = comps.clone();
                    c2[n] = nv as f32;
                    set = Some(c2);
                }
                if n % 2 == 1 {
                    ui.end_row();
                }
            }
            if let Some(c2) = set {
                // The mode stays as the edit changes the color's model (a gray edited in HSB is RGB).
                d.fields.insert("sliderMode".into(), json!(mode_id(mode)));
                set_to(&mut rows[i], &from_components(mode, &c2));
                d.fields.insert(COMPS.into(), json!({"row": i, "key": rows[i]["to"], "mode": mode_id(mode), "v": c2}));
                rows[i]["model"] = json!("new");
                changed = true;
            }
        });
    });
    // Room for two lines of sliders whatever the mode, so the dialog keeps its height.
    let per = (ui.cursor().top() - top) / lines as f32;
    ui.add_space(per * (2 - lines.min(2)) as f32);
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
fn edit_slider_row(d: &Dialog, rows: &[Value], active: &[usize], base: Option<usize>) -> Option<usize> {
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

/// What the Edit tab's wheels show, from [`WHEEL_AXES`].
fn wheel_axes(d: &Dialog) -> widgets::WheelAxes {
    if d.str(WHEEL_AXES) == "hueBrightness" { widgets::WheelAxes::HueBrightness } else { widgets::WheelAxes::HueSaturation }
}

/// The new color the Add Color tool gives a click at hue `h` (degrees) and `t`, the saturation (or
/// with [`widgets::WheelAxes::HueBrightness`] the brightness) clicked: with the other component of
/// `base` (the selected color, whose other component the wheel shows; full without one), the segmented
/// wheel's patch color with `segmented`, in `base`'s model.
fn added_color(base: Option<Color>, h: f32, t: f32, segmented: bool, axes: widgets::WheelAxes) -> Color {
    let o = base.map_or(1.0, |b| b.to_hsb()[axes.other()]);
    let c = if segmented {
        widgets::segment_color_on(axes, h, t, o)
    } else {
        let [h, s, v] = axes.hsb(h, t, o);
        Color::from_hsb(h, s, v)
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
            r.remove(SWATCH);
        }
        return Some(base.unwrap_or(0));
    }
    rows.remove(i);
    Some(base.map_or(0, |b| if b > i { b - 1 } else { b }))
}

/// Whether Remove Color keeps a row with current colors (see [`remove_color`]): with art, or with a
/// color group.
fn keeps_rows(d: &Dialog) -> bool {
    d.bool("__art") || d.fields.get("group").is_some_and(Value::is_string)
}

/// After Remove Color: select the base row at `base`; the slider values held were for a row index
/// that may now hold another row.
fn removed(d: &mut Dialog, base: usize) {
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

/// The target of the wheel's right-click menu: `{marker, line, hs}` (row indexes and the hue and
/// saturation clicked).
const MENU_AT: &str = "__wheelMenu";
/// Select Shade's grid: saturations across, from 0% to 100%, and brightnesses down, from 100%.
const SHADE_COLS: usize = 7;
const SHADE_ROWS: usize = 7;
const SHADE_CELL: f32 = 16.0;

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

/// Select Shade's grid of `c`'s hue, the cell nearest `c` framed. Returns the saturation and
/// brightness of the cell clicked.
fn shade_grid(ui: &mut Ui, c: &Color) -> Option<[f32; 2]> {
    let t = Tokens::get(ui.ctx());
    let [h, s0, v0] = c.to_hsb();
    let near = ((s0 * (SHADE_COLS - 1) as f32).round() as usize, (((1.0 - v0) * SHADE_ROWS as f32).round() as usize).min(SHADE_ROWS - 1));
    let mut out = None;
    egui::Grid::new("recolor-shades").spacing([2.0, 2.0]).show(ui, |ui| {
        for row in 0..SHADE_ROWS {
            for col in 0..SHADE_COLS {
                let [s, v] = shade_at(col, row);
                let (r, resp) = ui.allocate_exact_size(vec2(SHADE_CELL, SHADE_CELL), Sense::click());
                ui.painter().rect_filled(r, 1.0, c32(&Color::from_hsb(h, s, v)));
                if (col, row) == near {
                    ui.painter().rect_stroke(r, 1.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
                }
                let name = format!("{} {}%, {} {}%", tl!("Saturation"), (s * 100.0).round(), tl!("Brightness"), (v * 100.0).round());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                if resp.clicked() {
                    out = Some([s, v]);
                    ui.close();
                }
            }
            ui.end_row();
        }
    });
    out
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

/// A marker's or a color bar's right-click menu for row `i`: Set As Base Color and Remove Color
/// (neither for the base color `base`), Select Shade and Color Picker.
fn color_menu(ui: &mut Ui, rows: &[Value], i: usize, base: Option<usize>) -> Option<MenuChoice> {
    let mut choice = None;
    if widgets::menu_item_name(ui, tl!("Set As Base Color"), can_set_base(rows, i), false) {
        choice = Some(MenuChoice::Base(i));
    }
    if let Some(c) = rows.get(i).and_then(to_color) {
        // Indented like the items beside it (their check column).
        ui.menu_button(format!("   {}", tl!("Select Shade")), |ui| {
            if let Some(sv) = shade_grid(ui, &c) {
                choice = Some(MenuChoice::Shade(i, sv));
            }
        });
    }
    if widgets::menu_item_name(ui, tl!("Color Picker"), true, false) {
        choice = Some(MenuChoice::Picker(i));
    }
    if widgets::menu_item_name(ui, tl!("Remove Color"), Some(i) != base, false) {
        choice = Some(MenuChoice::Remove(i));
    }
    choice
}

/// Carry out a right-click menu's `choice` on the rows (`base`: the base color's row; `shown`: the
/// color whose brightness the wheel shows; `segmented`: the menu was on the segmented wheel).
/// Returns whether the rows changed and the row whose new color the Color Picker is to edit.
fn choose(
    d: &mut Dialog,
    rows: &mut Vec<Value>,
    choice: MenuChoice,
    base: Option<usize>,
    shown: Option<Color>,
    segmented: bool,
    axes: widgets::WheelAxes,
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
            let i = add_color(rows, &added_color(shown, h, s, segmented, axes));
            if let Some(l) = line {
                tie_to(rows, l, i);
            }
            d.fields.insert("__sel".into(), json!([i]));
            d.fields.remove(COMPS);
            (true, None)
        }
    }
}

/// The Edit tab. Returns whether the rows changed and the row whose new color a right-click menu's
/// Color Picker is to edit.
fn edit_tab(app: &VectorcraftApp, ui: &mut Ui, d: &mut Dialog, rows: &mut Vec<Value>) -> (bool, Option<(usize, bool)>) {
    let top = ui.cursor().top();
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let mut pick = None;
    let linked = d.bool("linked");
    let active = active_rows(rows);
    // The harmony rules build from the base color; the sliders and the Brightness slider edit the
    // selected color (the base color when the tab does not show the first selected row).
    let base = base_row(rows, &active);
    let sel = edit_slider_row(d, rows, &active, base);
    let target = SliderTarget::at(app, d, rows, sel);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Harmony Rules:")).color(t.text_dim));
        let rule = Harmony::parse(&d.str("rule")).unwrap_or(Harmony::Complementary);
        let labels: Vec<&str> = Harmony::ALL.iter().map(|h| h.label()).collect();
        // Choosing a rule gives the new colours its colours, from the base colour. The heading is
        // not a field label widget, so the menu carries its own accessible name.
        if let Some(i) = widgets::dropdown_named(ui, "recolor-rule", Some(tl!("Harmony Rules:")), rule.label(), &labels, 170.0) {
            d.fields.insert("rule".into(), json!(Harmony::ALL[i].id()));
            if let Some((b, c)) = base.and_then(|b| Some((b, to_color(&rows[b])?))) {
                apply_rule(rows, Harmony::ALL[i], b, c, &active);
                changed = true;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = if linked { tl!("Unlink harmony colors") } else { tl!("Link harmony colors") };
            if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tip, linked, 24.0).clicked() {
                d.fields.insert("linked".into(), json!(!linked));
            }
            // New Color Group: save the wheel's colors or the Prominent Colors as a color group.
            let button = widgets::icon_button(ui, "dc-folder", tl!("New Color Group"), false, 24.0);
            let prominent = d.bool("__art") && prominent_parts(d, rows).iter().any(|p| rows.get(p.0).and_then(to_color).is_some());
            let saved = egui::Popup::menu(&button).show(|ui| {
                if widgets::menu_item_name(ui, tl!("Save All Colors"), rows.iter().any(|r| in_group(r) && to_color(r).is_some()), false) {
                    return Some("all");
                }
                widgets::menu_item_name(ui, tl!("Save Prominent Colors"), prominent, false).then_some("prominent")
            });
            if let Some(which) = saved.and_then(|r| r.inner) {
                d.fields.insert(SAVE_GROUP.into(), json!(which));
            }
            ui.add_space(8.0);
            // The mode of the color sliders under the wheel: the selected new color in a color
            // mode, or Global Adjust for all of them.
            mode_menu(ui, d, &target, true);
        });
    });
    // With art the Prominent Colors bar has the room of the two gaps, so the tab is no taller with
    // it: on a short screen all of the tab but the chips has to fit.
    let art = d.bool("__art");
    if !art {
        ui.add_space(WHEEL_GAP);
    }
    let colors: Vec<Color> = active.iter().filter_map(|&i| to_color(&rows[i])).collect();
    let marked = base.and_then(|b| active.iter().position(|&i| i == b));
    let selected = sel.and_then(|s| active.iter().position(|&i| i == s));
    let shown = sel.and_then(|s| to_color(&rows[s]));
    let mut choice = None;
    let display = d.str("display");
    let tool = d.str(WHEEL_TOOL);
    let axes = wheel_axes(d);
    // The tools act on either wheel, not on the color bars.
    let on_wheel = display != "bars";
    let mut w = widgets::WheelResponse::default();
    ui.horizontal(|ui| {
        // The display buttons, beside the wheel so the tab keeps its height.
        ui.vertical(|ui| {
            for (id, icon, tip) in [
                ("smooth", "circle", tl!("Show smooth color wheel")),
                ("segmented", "dc-polar-grid", tl!("Show segmented color wheel")),
                ("bars", "dc-list-view", tl!("Show color bars")),
            ] {
                let on = display == id || (id == "smooth" && !matches!(display.as_str(), "segmented" | "bars"));
                if widgets::icon_button(ui, icon, tip, on, 24.0).clicked() {
                    d.fields.insert("display".into(), json!(id));
                }
            }
            // The wheel tools: a click on the button turns its tool on, or off again.
            ui.add_space(8.0);
            for (id, icon, tip) in [("add", "plus", tl!("Add Color Tool")), ("remove", "minus", tl!("Remove Color Tool"))] {
                let on = tool == id;
                if widgets::icon_button_enabled(ui, icon, tip, on && on_wheel, on_wheel, 24.0).clicked() {
                    d.fields.insert(WHEEL_TOOL.into(), if on { Value::Null } else { json!(id) });
                }
            }
            // What the wheels show: a click switches between hue and saturation, and hue and
            // brightness. It changes no color.
            ui.add_space(8.0);
            let bright = axes == widgets::WheelAxes::HueBrightness;
            let tip = if bright { tl!("Show saturation and hue on wheel") } else { tl!("Show brightness and hue on wheel") };
            if widgets::icon_button_enabled(ui, "sun", tip, bright && on_wheel, on_wheel, 24.0).clicked() {
                d.fields.insert(WHEEL_AXES.into(), json!(if bright { "hueSaturation" } else { "hueBrightness" }));
            }
        });
        ui.vertical(|ui| match display.as_str() {
            "bars" => {
                let (moved, chosen) = color_bars(ui, d, rows, &active, base);
                changed |= moved;
                choice = chosen;
            }
            _ => w = widgets::recolor_wheel(ui, "recolor-wheel", WHEEL, &colors, marked, selected, display == "segmented", axes),
        });
    });
    // A right-click on the wheel: a marker's menu, or Add New Color elsewhere. Its target is kept
    // while the menu stays open.
    if let Some((hit, line, hs)) = w.secondary {
        d.fields.insert(MENU_AT.into(), json!({"marker": hit.map(|k| active[k]), "line": line.map(|k| active[k]), "hs": hs}));
    } else if w.response.as_ref().is_some_and(egui::Response::secondary_clicked) {
        // A right-click off the wheel (in a corner of its square) targets nothing: its menu's Add
        // New Color is off, and no earlier marker is changed.
        d.fields.remove(MENU_AT);
    }
    if let Some(resp) = &w.response {
        let at = d.fields.get(MENU_AT).cloned().unwrap_or_default();
        let row = |k: &str| at[k].as_u64().map(|i| i as usize).filter(|i| active.contains(i));
        let hs = at["hs"].as_array().and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32]));
        resp.context_menu(|ui| match row("marker") {
            Some(i) => choice = color_menu(ui, rows, i, base),
            None => {
                if widgets::menu_item_name(ui, tl!("Add New Color"), hs.is_some(), false)
                    && let Some(hs) = hs
                {
                    choice = Some(MenuChoice::Add(hs, row("line")));
                }
            }
        });
    }
    // A menu's choice changes the rows the wheel's indexes were for, so the wheel's own clicks and
    // drags of this frame are left alone.
    let chosen = choice.is_some();
    if let Some(c) = choice {
        let (did, picked) = choose(d, rows, c, base, shown, display == "segmented", axes);
        changed |= did;
        pick = picked.map(|i| (i, false));
    }
    if chosen {
        w.pressed = None;
        w.moved = None;
        w.clicked = None;
    }
    // A click with a wheel tool adds a color there, or removes the color of the marker clicked.
    let clicked = w.clicked.filter(|_| on_wheel && matches!(tool.as_str(), "add" | "remove"));
    if let Some((hit, [h, out])) = clicked {
        if tool == "add" {
            let c = added_color(shown, h, out, display == "segmented", axes);
            let i = add_color(rows, &c);
            d.fields.insert("__sel".into(), json!([i]));
            d.fields.remove(COMPS);
            changed = true;
        } else if let Some(b) = hit.and_then(|k| remove_color(rows, active[k], base, keeps_rows(d))) {
            removed(d, b);
            changed = true;
        }
    }
    // Prominent Colors: the art's new colors, each as wide as its share of the art.
    if art {
        prominent_bar(ui, d, rows);
    }
    // A tool's click selects no marker: Add Color selects the new row, and Remove Color keeps the
    // base color.
    if let Some(k) = w.pressed.filter(|_| clicked.is_none()) {
        d.fields.insert("__sel".into(), json!([active[k]]));
    }
    // A drag (of a marker or the brightness slider) moves the rows as they were when it started:
    // a new colour read back from its model (CMYK through the colour profile) isn't exactly the
    // colour written, so building on last frame's rows drifted the linked colours (#1733).
    if let Some((k, set)) = w.moved.filter(|_| clicked.is_none()) {
        let start = match d.fields.get(DRAG).and_then(Value::as_array) {
            Some(s) if s.len() == rows.len() => s.clone(),
            _ => rows.to_vec(),
        };
        // What the control didn't set comes from the start too, not from last frame's colour.
        if let Some(old) = start.get(active[k]).and_then(to_color).map(|c| c.to_hsb()) {
            let hsb = [0, 1, 2].map(|n| set[n].unwrap_or(old[n]));
            rows.clone_from_slice(&start);
            move_color(rows, active[k], hsb, linked);
            changed = true;
        }
        d.fields.insert(DRAG.into(), Value::Array(start));
    }
    if !ui.input(|i| i.pointer.any_down()) {
        d.fields.remove(DRAG);
    }
    if !art {
        ui.add_space(SLIDERS_GAP);
    }
    // A color added or removed above changed the rows, so the sliders and chips read them afresh.
    let active = active_rows(rows);
    let base = base_row(rows, &active);
    let sel = edit_slider_row(d, rows, &active, base);
    if d.str("sliderMode") == GLOBAL {
        let sample = sel.and_then(|b| to_color(&rows[b])).unwrap_or(Color::rgb(0.8, 0.45, 0.25));
        global_sliders(ui, d, sample);
    } else {
        changed |= sliders(app, ui, d, rows, sel);
    }
    // The new colours: click one to select it; Set Current Color As Base Color, before them, makes
    // the selected one the base color, which carries a double ring as its marker does.
    let min_height = (TAB_H - (ui.cursor().top() - top)).max(0.0);
    let mut make_base = None;
    super::modal::scrolled_rows(ui, "recolor-edit-colors", |ui| {
        // Keep the usual tab height when it fits; the empty space scrolls with the chips.
        ui.set_min_height(min_height);
        ui.horizontal_wrapped(|ui| {
            let can = sel.is_some_and(|s| can_set_base(rows, s));
            if widgets::icon_button_enabled(ui, "circle-dot", tl!("Set Current Color As Base Color"), false, can, 20.0).clicked() {
                make_base = sel;
            }
            for &i in &active {
                let (r, resp) = ui.allocate_exact_size(vec2(28.0, 20.0), Sense::click());
                chip(ui, r, to_color(&rows[i]).unwrap_or_default(), false);
                if Some(i) == sel {
                    ui.painter().rect_stroke(r.expand(1.5), 2.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
                }
                if Some(i) == base {
                    ui.painter().circle_stroke(r.center(), 4.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                    ui.painter().circle_stroke(r.center(), 5.5, egui::Stroke::new(1.0, egui::Color32::BLACK));
                }
                if resp.on_hover_text(rows[i]["to"].as_str().unwrap_or_default()).clicked() {
                    d.fields.insert("__sel".into(), json!([i]));
                }
            }
        });
    });
    if let Some(i) = make_base {
        changed |= set_base(rows, i);
    }
    (changed, pick)
}

/// The Edit tab's color bars: a solid bar per row of `active` with its new color, in row order, the
/// base row's framed, scrolling past the wheel's height. A click on a bar selects its row (the
/// sliders under it edit that row's new color); a drag moves the bar's row to where the line under
/// the pointer shows, keeping every new color. A bar's right-click menu is a marker's
/// ([`color_menu`]). Returns true when the rows moved, and the menu's choice.
fn color_bars(ui: &mut Ui, d: &mut Dialog, rows: &mut [Value], active: &[usize], base: Option<usize>) -> (bool, Option<MenuChoice>) {
    let t = Tokens::get(ui.ctx());
    let mut rects: Vec<Rect> = Vec::with_capacity(active.len());
    let (mut dragging, mut released, mut dropped, mut choice) = (None, None, None, None);
    // The area is given its full height first: in the row beside the display buttons a scroll area
    // alone is only as tall as the room the row has.
    let area = vec2(ui.available_width(), BARS_H);
    ui.allocate_ui_with_layout(area, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_size(area);
        egui::ScrollArea::vertical().id_salt("recolor-bars").max_height(BARS_H).show(ui, |ui| {
            ui.set_min_height(BARS_H);
            ui.set_width(ui.available_width());
            if active.is_empty() {
                ui.add_space(8.0);
                widgets::dim_label(ui, tl!("No colors to recolor (preserved colors are left out)."));
            }
            for (n, &i) in active.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 4.0, BAR_H), Sense::click_and_drag());
                chip(ui, r, to_color(&rows[i]).unwrap_or_default(), false);
                if Some(i) == base {
                    ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                }
                let name = format!("{} {}", tl!("Color"), i + 1);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &name));
                if resp.clicked() || resp.drag_started() {
                    d.fields.insert("__sel".into(), json!([i]));
                }
                if resp.dragged() {
                    dragging = Some(n);
                }
                if resp.drag_stopped() {
                    released = Some(n);
                }
                rects.push(r);
                let resp = resp.on_hover_text(rows[i]["to"].as_str().unwrap_or_default());
                resp.context_menu(|ui| {
                    if let Some(c) = color_menu(ui, rows, i, base) {
                        choice = Some(c);
                    }
                });
            }
            // The place a dragged bar goes: before the first bar whose middle is below the pointer.
            let at = ui.input(|i| i.pointer.latest_pos()).map(|p| rects.iter().filter(|r| r.center().y < p.y).count());
            if let (Some(n), Some(to)) = (dragging, at)
                && to != n
                && to != n + 1
            {
                let y = rects.get(to).map_or_else(|| rects.last().map_or(0.0, |r| r.bottom() + 1.5), |r| r.top() - 1.5);
                let x = rects.first().map_or(ui.max_rect().x_range(), |r| r.x_range());
                ui.painter().hline(x, y, egui::Stroke::new(2.0, t.accent));
            }
            dropped = released.zip(at);
        });
    });
    if choice.is_some() {
        return (false, choice);
    }
    let Some(moved) = dropped.and_then(|(n, to)| move_bar(rows, active, n, to)) else { return (false, None) };
    d.fields.insert("__sel".into(), json!([moved]));
    // The slider values held are those of a row index that may now hold another row.
    d.fields.remove(COMPS);
    (true, None)
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
    let colors = rule.apply(b);
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
    use super::*;

    fn rows() -> Vec<Value> {
        vec![
            json!({"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"}),
            json!({"from": ["rgb 0 0 255", "rgb 0 0 128"], "to": "rgb 0 0 255"}),
            json!({"from": ["rgb 0 255 0"], "to": "rgb 0 255 0"}),
        ]
    }

    #[test]
    fn recolor_dialog_keeps_the_default_model_without_an_extra_dropdown() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": ["#ff8000"]})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(!d.fields.contains_key("model"));
        assert!(apply_params(d).get("model").is_none());
    }

    /// `recolor.apply` gets the Global Adjust amounts only when one of them is not 0%, whatever
    /// mode the sliders show.
    #[test]
    fn apply_params_send_global_adjust_only_when_it_changes_colors() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": ["#ff8000"]})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(apply_params(d).get("globalAdjust").is_none(), "no field");
        d.fields.insert("globalAdjust".into(), json!({"saturation": 0, "brightness": 0}));
        assert!(apply_params(d).get("globalAdjust").is_none(), "all 0%");
        d.fields.insert("globalAdjust".into(), json!({"temperature": 25, "luminosity": 500}));
        d.fields.insert("sliderMode".into(), json!("hsb"));
        assert_eq!(
            apply_params(d)["globalAdjust"],
            json!({"saturation": 0.0, "brightness": 0.0, "temperature": 25.0, "luminosity": 100.0}),
            "every amount, clamped to 100%"
        );
        // Colors opened alone become a new color group with the adjustment.
        d.fields.insert("globalAdjust".into(), json!({"saturation": -100}));
        let d = d.clone();
        confirm(&mut app, &d).unwrap();
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
        let sat = widgets::WheelAxes::HueSaturation;
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
        let bright = widgets::WheelAxes::HueBrightness;
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
        let mut d = Dialog::new(KIND, json!({"group": "G", "groupName": "G", "__art": true, "__assign": ["rgb 1 1 1", "rgb 2 2 2", "rgb 3 3 3"]}));
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
        let mut d = Dialog::new(KIND, json!({"group": "G", "groupName": "G", "__art": true, "__assign": ["rgb 1 1 1", "rgb 2 2 2", "rgb 3 3 3"]}));
        let rs = json!([
            {"from": ["rgb 255 0 0"], "to": "rgb 7 7 7", "exclude": true},
            {"from": ["rgb 0 0 255"], "to": null, "exclude": true},
            {"from": ["rgb 0 255 0"], "to": "rgb 8 8 8"},
        ]);
        d.fields.insert("rows".into(), rs);
        assert_eq!(apply_params(&d)["groupColors"], json!(["keep", "keep", "rgb 8 8 8"]));
        // Without art the rows are the group's colors.
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "G", "colors": ["#2878f0", "#ff0000"]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "G"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let mut rs = super::rows(&d);
        rs[0]["to"] = json!("#00ff00");
        rs[0]["exclude"] = json!(true);
        rs[1]["to"] = json!("#0000ff");
        d.fields.insert("rows".into(), json!(rs));
        d.fields.insert("globalAdjust".into(), json!({"brightness": -50}));
        confirm(&mut app, &d).unwrap();
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "G").unwrap();
        let hex: Vec<String> = g.swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect();
        assert_eq!(hex[0], "#2878f0", "{hex:?}");
        assert_ne!(hex[1], "#ff0000", "{hex:?}");
    }

    /// A row moved among the color bars keeps the group swatch it took when the rows were made, so
    /// an excluded row's swatch stays as it is and the other swatches keep their places.
    #[test]
    fn a_moved_row_keeps_its_group_swatch() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "G", "colors": ["#ff0000", "#0000ff", "#00ff00"]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "G"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let mut rs = super::rows(&d);
        rs[1]["exclude"] = json!(true);
        rs[2]["to"] = json!("#ffff00");
        let active = active_rows(&rs);
        assert_eq!(active, [0, 2]);
        // Drag the last bar (green, now yellow) above the first (red).
        assert_eq!(move_bar(&mut rs, &active, 1, 0), Some(0));
        d.fields.insert("rows".into(), json!(rs));
        let colors = &apply_params(&d)["groupColors"];
        assert_eq!(colors[1], json!("keep"), "{colors}");
        confirm(&mut app, &d).unwrap();
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
        let p = apply_params(&Dialog::new("recolor", fields));
        assert_eq!(p["groupColors"], json!(["rgb 255 0 0", "rgb 0 0 255", "rgb 0 255 0", "keep"]));
    }

    /// Opened on colors alone, OK saves the new colors of the rows that hold current colors as the
    /// new color group, in row order: an empty row gives it no color.
    #[test]
    fn an_empty_row_gives_a_new_group_no_color() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": ["#ff8000", "#0080ff"]})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        let at = new_row(&mut rs, &[1]);
        rs[at]["to"] = json!("rgb 255 0 255");
        d.fields.insert("rows".into(), Value::Array(rs));
        let d = d.clone();
        let groups = app.session.doc().unwrap().doc.swatch_groups.len();
        confirm(&mut app, &d).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.swatch_groups.len(), groups + 1, "a new color group");
        let colors: Vec<String> = doc.swatch_groups.last().unwrap().swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect();
        assert_eq!(colors, ["#ff8000", "#0080ff"]);
    }

    /// Randomize after a seed past `u64::MAX` keeps the largest seed.
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
    fn next_seed_holds_at_the_largest() {
        assert_eq!(next_seed(&Dialog::new("recolor", json!({}))), 1);
        assert_eq!(next_seed(&Dialog::new("recolor", json!({"__seed": 4}))), 5);
        assert_eq!(next_seed(&Dialog::new("recolor", json!({"__seed": 1e30}))), u64::MAX);
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
        let mut d = Dialog::new(KIND, json!({}));
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

    /// Select Shade's grid runs from 0% to 100% saturation across and from 100% brightness down.
    #[test]
    fn the_shade_grid_spans_saturation_and_brightness() {
        assert_eq!(shade_at(0, 0), [0.0, 1.0]);
        assert_eq!(shade_at(SHADE_COLS - 1, 0)[0], 1.0);
        let last = shade_at(0, SHADE_ROWS - 1)[1];
        assert!(last > 0.0 && last < 0.2, "{last}");
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
        let sel = |s: Value| Dialog::new("recolor", json!({"__sel": s}));
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
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": ["#ff0000"]})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("rows".into(), json!([{"from": ["rgb 255 0 0"], "to": "rgb 255 0 0", "tint": 40.0}]));
        let same = with_new_color(&app, d.clone(), 0, Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(super::rows(&same)[0]["tint"], json!(40.0));
        let other = with_new_color(&app, d, 0, Color::rgb(0.0, 1.0, 0.0));
        assert!(super::rows(&other)[0].get("tint").is_none(), "{:?}", super::rows(&other));
    }

    /// A spot swatch picked for a row with no Limit to Library gives the row the swatch and the
    /// sliders its Tint; another new color drops the swatch, and a Global Adjust, under which
    /// `recolor.apply` does not use the swatch, shows the color modes instead of Tint.
    #[test]
    fn a_picked_spot_swatch_gives_its_row_the_tint_slider() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": "cmyk 0 100 0 0", "spot": true})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": "#808080"})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": 1})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("rows".into(), json!([{"from": ["rgb 128 128 128"], "to": "rgb 128 128 128"}]));
        let ink = Color::cmyk(0.0, 1.0, 0.0, 0.0);
        let picked = with_new_swatch(&app, d.clone(), 0, ink, Some(json!({"name": "Ink", "library": "document"})));
        let rs = super::rows(&picked);
        assert_eq!(rs[0][SWATCH], json!({"name": "Ink", "library": "document"}));
        assert_eq!(SliderTarget::at(&app, &picked, &rs, Some(0)).spot.as_deref(), Some("Ink"), "the Tint slider");
        let other = with_new_color(&app, picked.clone(), 0, Color::rgb(0.0, 1.0, 0.0));
        assert!(super::rows(&other)[0].get(SWATCH).is_none(), "{:?}", super::rows(&other));
        let mut adjusted = picked;
        adjusted.fields.insert("globalAdjust".into(), json!({"saturation": 20.0, "brightness": 0.0, "temperature": 0.0, "luminosity": 0.0}));
        assert_eq!(SliderTarget::at(&app, &adjusted, &rs, Some(0)).spot, None, "no Tint under a Global Adjust");
    }

    /// A row's chip shows its tint while its picked swatch links the art, also while a preview
    /// changes that swatch; once the swatch is deleted the chip shows the full color the art takes
    /// and OK recolors with it. A process swatch picked in its place takes the row's swatch and tint
    /// away.
    #[test]
    fn a_picked_swatch_that_is_gone_links_nothing() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": "cmyk 0 100 0 0", "spot": true})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": "#808080"})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": 1})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("rows".into(), json!([{"from": ["rgb 128 128 128"], "to": "rgb 128 128 128"}]));
        let ink = Color::cmyk(0.0, 1.0, 0.0, 0.0);
        let mut picked = with_new_swatch(&app, d, 0, ink, Some(json!({"name": "Ink", "library": "document"})));
        let mut rs = super::rows(&picked);
        rs[0]["tint"] = json!(40.0);
        picked.fields.insert("rows".into(), Value::Array(rs));
        let rs = super::rows(&picked);
        let tinted = new_shown(&app, &picked, &RowLinks::new(&app, &picked), &rs[0]);
        assert!(tinted != ink && tinted.to_rgb()[1] > ink.to_rgb()[1], "the chip at 40%: {tinted:?}");
        // A process swatch of the same color picked for the row.
        let process = with_new_swatch(&app, picked.clone(), 0, ink, None);
        let r = &super::rows(&process)[0];
        assert!(r.get(SWATCH).is_none() && r.get("tint").is_none(), "{r}");
        // A preview that gives the swatch another color: the row still links to it.
        let _ = app.session.cancel_interaction();
        app.session.begin_interaction("Swatch Options").unwrap();
        app.session.preview("swatch.edit", &json!({"name": "Ink", "color": "cmyk 100 0 0 0"})).unwrap();
        assert_eq!(SliderTarget::at(&app, &picked, &rs, Some(0)).spot.as_deref(), Some("Ink"), "the swatch as it was before the preview");
        assert_eq!(new_shown(&app, &picked, &RowLinks::new(&app, &picked), &rs[0]), tinted);
        let _ = app.session.cancel_interaction();
        app.run("swatch.delete", json!({"names": ["Ink"]})).unwrap();
        assert_eq!(SliderTarget::at(&app, &picked, &rs, Some(0)).spot, None, "no Tint slider");
        assert_eq!(new_shown(&app, &picked, &RowLinks::new(&app, &picked), &rs[0]), ink, "the full color the art takes");
        let r = app.session.execute(CMD, &apply_params(&picked)).unwrap();
        assert_eq!(r["swatchesNotFound"], json!(["Ink"]), "{r}");
        let doc = &app.session.doc().unwrap().doc;
        match doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint() {
            vectorcraft_color::Paint::Solid { color, swatch, .. } => {
                assert_eq!(swatch, None, "no link");
                let [r, g, b] = color.to_rgb();
                assert!(r > 0.9 && g < 0.1 && b > 0.4, "the full ink, not a 40% tint or the gray: {color:?}");
            }
            p => panic!("{p:?}"),
        }
    }

    /// A row picked from a spot swatch of the color group shows Tint and its tint on the chip while
    /// OK leaves the swatch its color, and no Tint and the full color once OK gives the swatch
    /// another color, as `recolor.apply` then gives the art.
    #[test]
    fn a_picked_swatch_the_group_rewrites_shows_the_full_color() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": "cmyk 0 100 0 0", "spot": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Inks", "swatches": ["Ink"]})).unwrap();
        for c in ["cmyk 0 100 0 0", "#808080"] {
            let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"ids": [id], "color": c})).unwrap();
            app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Inks"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let ink = Color::cmyk(0.0, 1.0, 0.0, 0.0);
        let rows = |first: &str| {
            json!([
                {"from": ["cmyk 0 100 0 0"], "to": first},
                {"from": ["rgb 128 128 128"], "to": "cmyk 0 100 0 0", "swatch": {"name": "Ink", "library": "document"}, "tint": 40.0},
            ])
        };
        d.fields.insert("rows".into(), rows("cmyk 0 100 0 0"));
        let rs = super::rows(&d);
        assert_eq!(SliderTarget::at(&app, &d, &rs, Some(1)).spot.as_deref(), Some("Ink"), "Ink keeps its color");
        assert_ne!(new_shown(&app, &d, &RowLinks::new(&app, &d), &rs[1]), ink, "the chip at 40%");
        d.fields.insert("rows".into(), rows("rgb 0 255 0"));
        let rs = super::rows(&d);
        assert_eq!(SliderTarget::at(&app, &d, &rs, Some(1)).spot, None, "no Tint slider once Ink takes green");
        assert_eq!(new_shown(&app, &d, &RowLinks::new(&app, &d), &rs[1]), ink, "the full color the art takes");
    }

    /// With the Spot Colors options on CMYK values, a row whose new color is a Lab spot color of the
    /// library shows the art's color at its tint: the working-CMYK equivalent with its inks scaled.
    #[test]
    fn a_tinted_lab_spot_row_shows_the_cmyk_color_the_art_takes() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
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
        app.run("ui.recolorDialog", json!({"library": lib})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        sync(&mut app, &mut d);
        let mut rs = super::rows(&d);
        rs[0]["tint"] = json!(50.0);
        let c = to_color(&rs[0]).unwrap();
        assert!(matches!(c, Color::Lab { .. }), "{c:?}");
        let cmyk = app.session.doc().unwrap().doc.linked_color(c, true);
        assert!(matches!(cmyk, Color::Cmyk { .. }), "{cmyk:?}");
        assert_eq!(new_shown(&app, &d, &RowLinks::new(&app, &d), &rs[0]), cmyk.tinted(0.5));
    }

    /// Runs one frame of the open dialog at `time` and returns row `i`'s new color chip, when the
    /// Recolor Artwork dialog drew it.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>, i: usize) -> Option<Rect> {
        let input = egui::RawInput { time: Some(time), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        ctx.read_response(egui::Id::unique((NEW_CHIP, i))).map(|r| r.rect)
    }

    /// Runs frames from `time` until the dialog stops moving (it centres itself as it measures its
    /// size) and returns row `i`'s new color chip.
    fn settled(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, i: usize) -> Option<Rect> {
        let mut last = None;
        for n in 0..10 {
            let r = frame(app, ctx, time + f64::from(n) * 0.001, vec![], i);
            if r.is_some() && r == last {
                return r;
            }
            last = r;
        }
        last
    }

    /// Double-clicks row `i`'s new color chip, as drawn by the last frame at `rect`.
    fn double_click(app: &mut VectorcraftApp, ctx: &egui::Context, rect: Rect, time: f64, i: usize) {
        let pos = rect.center();
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, time, vec![egui::Event::PointerMoved(pos), button(true)], i);
        frame(app, ctx, time + 0.05, vec![button(false)], i);
        frame(app, ctx, time + 0.1, vec![button(true)], i);
        frame(app, ctx, time + 0.15, vec![button(false)], i);
    }

    /// Double-clicking a row's new color opens the Color Picker on that new color, over the dialog.
    /// OK sets that row's new color, and only that row's, in the model it was picked in (a CMYK mix
    /// stays that mix, not a hex value converted back), and the art previews it; a color picked as
    /// RGB is CMYK in a CMYK document. Cancel leaves the rows and the preview as they were, and
    /// nothing is committed until Recolor Artwork's own OK.
    #[test]
    fn double_clicking_a_new_color_picks_it_in_its_own_model() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
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
        app.run("ui.recolorDialog", json!({"colors": assign})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        // The previewed fill of the rectangle whose color row `row` recolors.
        let fill = |app: &VectorcraftApp, row: &Value| {
            let from = from_colors(row);
            let doc = &app.session.doc().unwrap().doc;
            let original = |id: u64| if id == ids[0] { Color::cmyk(0.0, 1.0, 1.0, 0.0) } else { Color::cmyk(1.0, 0.0, 0.0, 0.0) };
            let id = *ids.iter().find(|id| from.contains(&original(**id))).expect("a rectangle for the row");
            doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill().and_then(|f| f.paint.color())
        };
        let before = undo(&app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let all = super::rows(app.ui.dialog.as_ref().unwrap());
        assert_eq!(all.len(), 2);
        let i = 1;
        assert_ne!(all[i]["from"][0], all[i]["to"], "the row's new color isn't its current color");
        let rect = settled(&mut app, &ctx, 0.01, i).expect("the row's new color is drawn");
        let previewed = to_color(&all[i]);
        assert_eq!(fill(&app, &all[i]), previewed, "the art previews the row's new color");

        double_click(&mut app, &ctx, rect, 0.1, i);
        frame(&mut app, &ctx, 0.3, vec![], i);
        assert_eq!(fill(&app, &all[0]), to_color(&all[0]), "the picker previews nothing itself");
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, "colorPicker", "a double-click opens the Color Picker");
        assert_eq!(d.fields.get("color").and_then(color_value), previewed, "on the row's new color");
        crate::dialogs::cancel(&mut app);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "Cancel comes back to Recolor Artwork");
        assert_eq!(super::rows(app.ui.dialog.as_ref().unwrap()), all, "with the rows as they were");
        frame(&mut app, &ctx, 0.5, vec![], i);
        assert_eq!(fill(&app, &all[i]), previewed, "and the art previews them again");

        let rect = settled(&mut app, &ctx, 1.0, i).unwrap();
        double_click(&mut app, &ctx, rect, 1.1, i);
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.kind, "colorPicker");
        // 100 50 0 0 is a mix the round trip through sRGB changes.
        d.fields.insert("color".into(), json!({"c": 100, "m": 50, "y": 0, "k": 0}));
        crate::dialogs::confirm(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, KIND, "OK comes back to Recolor Artwork");
        let rows = super::rows(d);
        assert_eq!(rows[i]["to"], "cmyk 100 50 0 0", "the row keeps the CMYK mix picked");
        assert_eq!(rows[0], all[0], "the other row stays");
        assert!(app.session.in_interaction(), "the dialog still previews");
        assert_eq!(undo(&app), before, "nothing is committed before Recolor Artwork's OK");
        frame(&mut app, &ctx, 1.5, vec![], i);
        assert_eq!(fill(&app, &rows[i]), Some(Color::cmyk(1.0, 0.5, 0.0, 0.0)), "the art previews the picked color");

        let rect = settled(&mut app, &ctx, 2.0, i).unwrap();
        double_click(&mut app, &ctx, rect, 2.1, i);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("color".into(), json!([1.0, 0.0, 0.0]));
        crate::dialogs::confirm(&mut app).unwrap();
        let to = super::rows(app.ui.dialog.as_ref().unwrap())[i]["to"].clone();
        assert!(to.as_str().is_some_and(|k| k.starts_with("cmyk ")), "a color picked as RGB is CMYK in a CMYK document: {to}");

        let rect = settled(&mut app, &ctx, 3.0, i).unwrap();
        double_click(&mut app, &ctx, rect, 3.1, i);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("color".into(), json!({"l": 55.0, "a": 20.0, "b": -30.0}));
        crate::dialogs::confirm(&mut app).unwrap();
        let to = super::rows(app.ui.dialog.as_ref().unwrap())[i]["to"].clone();
        assert_eq!(to, "lab 55 20 -30", "a Lab color picked in the picker is kept as Lab: {to}");
    }

    /// A 15 pt square at (`x`, 10) filled with `fill`, without a stroke.
    fn square(app: &mut VectorcraftApp, x: f64, fill: &str) -> u64 {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 15, "height": 15})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [id], "color": fill})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        id
    }

    fn fill_hex(app: &VectorcraftApp, id: u64) -> String {
        let doc = &app.session.doc().unwrap().doc;
        doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill().and_then(|f| f.paint.color()).unwrap().to_hex()
    }

    /// Red and blue squares selected, and green and yellow squares beside them; Recolor Artwork
    /// open on the selection with the Exact method, one frame drawn.
    fn theme_setup(app: &mut VectorcraftApp, ctx: &egui::Context) -> [u64; 2] {
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let art = [square(app, 10.0, "#ff0000"), square(app, 40.0, "#0000ff")];
        square(app, 100.0, "#00ff00");
        square(app, 130.0, "#ffff00");
        app.run("select.set", json!({"ids": art})).unwrap();
        app.run("ui.recolorDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(ctx);
        frame(app, ctx, 0.0, vec![], 0);
        art
    }

    fn new_colors(app: &VectorcraftApp) -> Vec<Value> {
        super::rows(app.ui.dialog.as_ref().unwrap()).iter().map(|r| r["to"].clone()).collect()
    }

    /// The Color Theme Picker's picks (the `themePicks` field) give the rows that aren't excluded
    /// the palette of the art picked, in row order, repeating it when the rows outnumber its
    /// colors, and the art previews them. Picks that touch no art leave the rows as they are, and
    /// OK with rows that give the art its own colors back records no undo step.
    #[test]
    fn theme_picks_give_the_rows_the_palette_of_the_art_picked() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let [red, blue] = theme_setup(&mut app, &ctx);
        let mut start = new_colors(&app);
        start.sort_by_key(Value::to_string);
        assert_eq!(start, [json!("rgb 0 0 255"), json!("rgb 255 0 0")]);
        // The red art's row and the blue art's row.
        let row_of = |app: &VectorcraftApp, key: &str| super::rows(app.ui.dialog.as_ref().unwrap()).iter().position(|r| r["from"][0] == key).unwrap();
        let (r, b) = (row_of(&app, "rgb 255 0 0"), row_of(&app, "rgb 0 0 255"));
        let undo = app.session.doc().unwrap().history.undo.len();
        let set = |app: &mut VectorcraftApp, picks: Value| {
            app.ui.dialog.as_mut().unwrap().fields.insert(THEME_PICKS.into(), picks);
        };
        set(&mut app, json!([{"at": [107, 17]}]));
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert_eq!(new_colors(&app), [json!("rgb 0 255 0"), json!("rgb 0 255 0")], "one color for both rows");
        assert_eq!((fill_hex(&app, red), fill_hex(&app, blue)), ("#00ff00".into(), "#00ff00".into()), "the art previews it");
        set(&mut app, json!([{"at": [107, 17]}, {"rect": [125, 5, 150, 30]}]));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(new_colors(&app), [json!("rgb 0 255 0"), json!("rgb 255 255 0")], "a pick added: two colors");
        // An excluded row (the red art's) keeps its new color.
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        rs[r]["exclude"] = json!(true);
        d.fields.insert("rows".into(), Value::Array(rs));
        let kept = new_colors(&app)[r].clone();
        set(&mut app, json!([{"at": [47, 17]}]));
        frame(&mut app, &ctx, 0.3, vec![], 0);
        let after = new_colors(&app);
        assert_eq!((&after[r], &after[b]), (&kept, &json!("rgb 0 0 255")));
        set(&mut app, json!([{"at": [250, 80]}]));
        frame(&mut app, &ctx, 0.4, vec![], 0);
        assert_eq!(new_colors(&app), after, "empty canvas: the rows stay");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "nothing is committed before OK");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((fill_hex(&app, red), fill_hex(&app, blue)), ("#ff0000".into(), "#0000ff".into()), "the excluded row kept red");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "the art is unchanged: no undo step");
    }

    /// The Color Theme Picker gives its colors only to the rows that hold current colors: a row
    /// made with New Row keeps its new color and takes no place in the palette.
    #[test]
    fn theme_picks_leave_an_empty_row_out() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        theme_setup(&mut app, &ctx);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        let at = new_row(&mut rs, &[]);
        rs[at]["to"] = json!("rgb 9 9 9");
        d.fields.insert("rows".into(), Value::Array(rs));
        d.fields.insert(THEME_PICKS.into(), json!([{"at": [107, 17]}, {"rect": [125, 5, 150, 30]}]));
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert_eq!(new_colors(&app), [json!("rgb 9 9 9"), json!("rgb 0 255 0"), json!("rgb 255 255 0")]);
    }

    /// A row made with New Row recolors nothing while it is empty, and OK keeps it; the blue art's
    /// color merged into it takes its new color, in the preview and at OK.
    #[test]
    fn colors_merged_into_a_new_row_take_its_new_color() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let [red, blue] = theme_setup(&mut app, &ctx);
        let undo = app.session.doc().unwrap().history.undo.len();
        let set_rows = |app: &mut VectorcraftApp, rs: Vec<Value>| {
            app.ui.dialog.as_mut().unwrap().fields.insert("rows".into(), Value::Array(rs));
        };
        let mut rs = super::rows(app.ui.dialog.as_ref().unwrap());
        let at = new_row(&mut rs, &[]);
        rs[at]["to"] = json!("rgb 0 255 0");
        set_rows(&mut app, rs.clone());
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert_eq!((fill_hex(&app, red), fill_hex(&app, blue)), ("#ff0000".into(), "#0000ff".into()), "an empty row recolors nothing");
        let b = rs.iter().position(|r| r["from"] == json!([BLUE])).unwrap();
        merge(&mut rs, &[at, b]);
        // A second empty row stays empty and is sent at OK too.
        new_row(&mut rs, &[]);
        set_rows(&mut app, rs);
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(fill_hex(&app, blue), "#00ff00", "the art previews the merge");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((fill_hex(&app, red), fill_hex(&app, blue)), ("#ff0000".into(), "#00ff00".into()));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    const RED: &str = "rgb 255 0 0";
    const PURPLE: &str = "rgb 128 0 204";
    const BLUE: &str = "rgb 0 0 255";

    /// Two red squares, a purple one and a blue one, selected; Recolor Artwork open on them on the
    /// Edit tab with the Exact method and the rows set: the blue to yellow, then the red and purple
    /// to green (the purple's square id last). One frame drawn.
    fn prominent_setup(app: &mut VectorcraftApp, ctx: &egui::Context) -> u64 {
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let ids = [square(app, 10.0, "#ff0000"), square(app, 40.0, "#ff0000"), square(app, 70.0, "#8000cc"), square(app, 100.0, "#0000ff")];
        app.run("select.set", json!({"ids": ids})).unwrap();
        app.run("ui.recolorDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("method".into(), json!(Method::Exact.id()));
        d.fields.insert("tab".into(), json!("edit"));
        crate::theme::install_fonts(ctx);
        frame(app, ctx, 0.0, vec![], 0);
        let rows = json!([{"from": [BLUE], "to": "rgb 255 255 0"}, {"from": [RED, PURPLE], "to": "rgb 0 255 0"}]);
        app.ui.dialog.as_mut().unwrap().fields.insert("rows".into(), rows);
        frame(app, ctx, 0.01, vec![], 0);
        ids[2]
    }

    fn froms(app: &VectorcraftApp) -> Vec<Value> {
        super::rows(app.ui.dialog.as_ref().unwrap()).iter().map(|r| r["from"].clone()).collect()
    }

    /// The Prominent Colors bar holds each row's share of the art's uses, the largest first; new
    /// `weights` move the rows' current colors toward them, the art previews the result, and only
    /// OK commits it, as one undo step.
    #[test]
    fn weights_move_the_rows_colors_and_the_art_previews_them() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let purple = prominent_setup(&mut app, &ctx);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.fields[SHARES], json!([25.0, 75.0]));
        assert_eq!(prominent_parts(d, &super::rows(d)), [(1, 75.0), (0, 25.0)], "the largest share first");
        assert_eq!(fill_hex(&app, purple), "#00ff00", "the art previews the rows");
        let undo = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert(WEIGHTS.into(), json!([50, 50]));
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert_eq!(froms(&app), [json!([BLUE, PURPLE]), json!([RED])]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields[SHARES], json!([50.0, 50.0]));
        assert_eq!(fill_hex(&app, purple), "#ffff00", "the moved color previews its new row's color");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "nothing is committed before OK");
        // A new count of Colors reduces the rows again and drops the weights.
        app.ui.dialog.as_mut().unwrap().fields.insert("colors".into(), json!(2));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get(WEIGHTS), None);
        app.ui
            .dialog
            .as_mut()
            .unwrap()
            .fields
            .insert("rows".into(), json!([{"from": [BLUE, PURPLE], "to": "rgb 255 255 0"}, {"from": [RED], "to": "rgb 0 255 0"}]));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(fill_hex(&app, purple), "#ffff00");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// A row that takes a new color takes the colors it shares with other rows, so the bar reads
    /// the shares again.
    #[test]
    fn a_new_color_for_a_row_reads_the_shares_again() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        prominent_setup(&mut app, &ctx);
        let rows = json!([{"from": [RED, PURPLE], "to": null}, {"from": [RED, BLUE], "to": "rgb 255 255 0"}]);
        app.ui.dialog.as_mut().unwrap().fields.insert("rows".into(), rows);
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields[SHARES], json!([25.0, 75.0]));
        let rows = json!([{"from": [RED, PURPLE], "to": "rgb 0 255 0"}, {"from": [RED, BLUE], "to": "rgb 255 255 0"}]);
        app.ui.dialog.as_mut().unwrap().fields.insert("rows".into(), rows);
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields[SHARES], json!([75.0, 25.0]));
    }

    /// Dragging the edge between two parts of the bar to the left gives the right-hand part more
    /// weight; when the drag ends, the rows' current colors move.
    #[test]
    fn dragging_a_prominent_edge_moves_colors_between_rows() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let purple = prominent_setup(&mut app, &ctx);
        let edge = |ctx: &egui::Context| ctx.read_response(egui::Id::unique((PROMINENT_EDGE, 0usize))).map(|r| r.rect);
        let mut last = None;
        for n in 0..10 {
            frame(&mut app, &ctx, 0.02 + f64::from(n) * 0.001, vec![], 0);
            let r = edge(&ctx);
            if r.is_some() && r == last {
                break;
            }
            last = r;
        }
        let at = edge(&ctx).expect("the edge between the two parts").center();
        let bar = ctx.read_response(egui::Id::unique((PROMINENT_PART, 0usize))).unwrap().rect;
        // From 75% to 50%: a quarter of the bar to the left.
        let to = at - vec2((bar.width() / 0.75) * 0.25, 0.0);
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at), button(at, true)], 0);
        for k in 1..=5 {
            frame(&mut app, &ctx, 0.1 + f64::from(k) * 0.02, vec![egui::Event::PointerMoved(at + (to - at) * (k as f32 / 5.0))], 0);
        }
        assert!(app.ui.dialog.as_ref().unwrap().fields.contains_key(BAR_DRAG), "the drag is under way");
        assert_eq!(froms(&app)[0], json!([BLUE]), "the rows wait for the drag to end");
        frame(&mut app, &ctx, 0.3, vec![button(to, false)], 0);
        frame(&mut app, &ctx, 0.32, vec![], 0);
        let d = app.ui.dialog.as_ref().unwrap();
        let w = d.fields[WEIGHTS].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>();
        assert!((w[0] - 50.0).abs() < 2.0 && (w[1] - 50.0).abs() < 2.0, "{w:?}");
        assert_eq!(froms(&app), [json!([BLUE, PURPLE]), json!([RED])]);
        assert_eq!(fill_hex(&app, purple), "#ffff00");
    }

    /// The New Color Group menu's Save All Colors saves the new colors in row order and Save
    /// Prominent Colors the bar's colors, the largest share first, each as its own step; the
    /// preview of the recolor runs on.
    #[test]
    fn save_all_and_prominent_colors_make_color_groups() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let purple = prominent_setup(&mut app, &ctx);
        let groups = |app: &VectorcraftApp| app.session.doc().unwrap().doc.swatch_groups.len();
        let last = |app: &VectorcraftApp| -> Vec<String> {
            let doc = &app.session.doc().unwrap().doc;
            doc.swatch_groups.last().unwrap().swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect()
        };
        let (n, undo) = (groups(&app), app.session.doc().unwrap().history.undo.len());
        for (which, colors, t) in [("all", ["#ffff00", "#00ff00"], 0.1), ("prominent", ["#00ff00", "#ffff00"], 0.2)] {
            app.ui.dialog.as_mut().unwrap().fields.insert(SAVE_GROUP.into(), json!(which));
            frame(&mut app, &ctx, t, vec![], 0);
            frame(&mut app, &ctx, t + 0.01, vec![], 0);
            assert_eq!(last(&app), colors, "{which}");
            assert_eq!(fill_hex(&app, purple), "#00ff00", "{which}: the art still previews the rows");
        }
        assert_eq!(groups(&app), n + 2);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 2, "a step for each group");
    }

    /// A color removed on the Edit tab leaves its row with no new color, and neither Save All
    /// Colors, Save Prominent Colors nor OK on colors opened alone puts the row's current color in
    /// the new color group.
    #[test]
    fn a_removed_color_joins_no_saved_color_group() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        prominent_setup(&mut app, &ctx);
        let last = |app: &VectorcraftApp| -> Vec<String> {
            let doc = &app.session.doc().unwrap().doc;
            doc.swatch_groups.last().unwrap().swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect()
        };
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("rows".into(), json!([{"from": [BLUE], "to": null}, {"from": [RED, PURPLE], "to": "rgb 0 255 0"}]));
        d.fields.insert(SAVE_GROUP.into(), json!("all"));
        frame(&mut app, &ctx, 0.1, vec![], 0);
        frame(&mut app, &ctx, 0.11, vec![], 0);
        assert_eq!(last(&app), ["#00ff00"], "Save All Colors");
        let n = app.session.doc().unwrap().doc.swatch_groups.len();
        app.ui.dialog.as_mut().unwrap().fields.insert(SAVE_GROUP.into(), json!("prominent"));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        frame(&mut app, &ctx, 0.21, vec![], 0);
        assert_eq!(app.session.doc().unwrap().doc.swatch_groups.len(), n + 1, "Save Prominent Colors makes a group");
        assert_eq!(last(&app), ["#00ff00"], "Save Prominent Colors");

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": ["#ff0000", "#0000ff"]})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("rows".into(), json!([{"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"}, {"from": ["rgb 0 0 255"], "to": null}]));
        let d = d.clone();
        confirm(&mut app, &d).unwrap();
        assert_eq!(last(&app), ["#ff0000"], "OK on colors opened alone");
    }

    /// Save All Colors leaves out a row made with New Row while it is empty; with only empty rows
    /// it saves no group and records no step.
    #[test]
    fn save_all_colors_leaves_an_empty_row_out() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        prominent_setup(&mut app, &ctx);
        let groups = |app: &VectorcraftApp| app.session.doc().unwrap().doc.swatch_groups.len();
        let (n, undo) = (groups(&app), app.session.doc().unwrap().history.undo.len());
        let save = |app: &mut VectorcraftApp, rows: Value, t: f64| {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("rows".into(), rows);
            d.fields.insert(SAVE_GROUP.into(), json!("all"));
            frame(app, &ctx, t, vec![], 0);
            frame(app, &ctx, t + 0.01, vec![], 0);
        };
        let blank = json!({"from": [], "to": "rgb 255 0 255"});
        save(&mut app, json!([blank, {"from": [BLUE], "to": "rgb 255 255 0"}, {"from": [RED, PURPLE], "to": "rgb 0 255 0"}]), 0.1);
        let doc = &app.session.doc().unwrap().doc;
        let colors: Vec<String> = doc.swatch_groups.last().unwrap().swatches.iter().map(|w| w.paint.color().unwrap().to_hex()).collect();
        assert_eq!(colors, ["#ffff00", "#00ff00"]);
        assert_eq!(groups(&app), n + 1);
        save(&mut app, json!([blank, {"from": [BLUE, RED, PURPLE], "to": "rgb 0 255 0", "exclude": true}]), 0.2);
        assert_eq!(groups(&app), n + 1, "only an empty and an excluded row: no group");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "and no step");
    }

    /// One frame with `modifiers` held.
    fn frame_with(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { time: Some(time), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    /// With the Color Theme Picker on, a click on the canvas picks the object there and a Shift-drag
    /// adds the art inside its marquee; with it off the dialog takes no picks from the canvas.
    #[test]
    fn the_theme_picker_takes_clicks_and_shift_drags_on_the_canvas() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let [red, _] = theme_setup(&mut app, &ctx);
        // Clear of the dialog, which centers itself in the screen.
        let canvas = Rect::from_min_size(pos2(-1000.0, -1000.0), vec2(3000.0, 3000.0));
        app.canvas_rect = Some(canvas);
        let xf = crate::canvas::Xf::new(canvas, app.view().unwrap());
        let at = |x: f64, y: f64| xf.to_screen(vectorcraft_geom::Point::new(x, y));
        let button = |pos, pressed, modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        let none = egui::Modifiers::NONE;
        let click = |app: &mut VectorcraftApp, pos: egui::Pos2, t: f64| {
            frame_with(app, &ctx, t, vec![egui::Event::PointerMoved(pos), button(pos, true, none)], none);
            frame_with(app, &ctx, t + 0.05, vec![button(pos, false, none)], none);
            frame_with(app, &ctx, t + 0.1, vec![], none);
        };
        let green = at(107.0, 17.0);
        assert!(canvas.contains(green), "{green:?}");
        click(&mut app, green, 1.0);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get(THEME_PICKS), Some(&json!([])), "off: no picks");
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get(THEME_PICKER), Some(&json!(false)), "the fields `ui.dialog.set` can set");
        app.ui.dialog.as_mut().unwrap().fields.insert(THEME_PICKER.into(), json!(true));
        for n in 0..5 {
            frame_with(&mut app, &ctx, 2.0 + f64::from(n) * 0.01, vec![], none);
        }
        click(&mut app, green, 3.0);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields[THEME_PICKS].as_array().map(Vec::len), Some(1));
        assert_eq!(new_colors(&app), [json!("rgb 0 255 0"), json!("rgb 0 255 0")]);
        assert_eq!(fill_hex(&app, red), "#00ff00", "the art previews the picked color");
        // A Shift-drag around the yellow square.
        let shift = egui::Modifiers::SHIFT;
        let (a, b) = (at(125.0, 5.0), at(150.0, 30.0));
        frame_with(&mut app, &ctx, 4.0, vec![egui::Event::PointerMoved(a), button(a, true, shift)], shift);
        frame_with(&mut app, &ctx, 4.05, vec![egui::Event::PointerMoved(a + (b - a) * 0.5)], shift);
        frame_with(&mut app, &ctx, 4.1, vec![egui::Event::PointerMoved(b)], shift);
        frame_with(&mut app, &ctx, 4.15, vec![button(b, false, shift)], shift);
        frame_with(&mut app, &ctx, 4.2, vec![], none);
        let picks = app.ui.dialog.as_ref().unwrap().fields[THEME_PICKS].clone();
        assert_eq!(picks.as_array().map(Vec::len), Some(2), "{picks}");
        assert!(picks[1].get("rect").is_some(), "{picks}");
        assert_eq!(new_colors(&app), [json!("rgb 0 255 0"), json!("rgb 255 255 0")]);
    }

    /// Find previews nothing of its own while it is off, on the Edit tab, without art or without a
    /// selected row. On, a first `recolor.apply` dims each current color of the other rows (a color
    /// a selected row also lists stays), and with Preview on a second recolors the selected rows
    /// alone, without rewriting the color group.
    #[test]
    fn find_dims_the_colors_of_the_rows_not_selected() {
        let mut d = Dialog::new(
            KIND,
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
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        let [red, blue] = theme_setup(&mut app, &ctx);
        let undo = app.session.doc().unwrap().history.undo.len();
        let set = |app: &mut VectorcraftApp, k: &str, v: Value| {
            app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
        };
        let rs = json!([{"from": ["rgb 255 0 0"], "to": "rgb 0 255 0"}, {"from": ["rgb 0 0 255"], "to": "rgb 255 255 0"}]);
        set(&mut app, "rows", rs.clone());
        set(&mut app, "__sel", json!([0]));
        frame(&mut app, &ctx, 0.1, vec![], 0);
        let fills = |app: &VectorcraftApp| (fill_hex(app, red), fill_hex(app, blue));
        assert_eq!(fills(&app), ("#00ff00".into(), "#ffff00".into()), "off: the preview");
        set(&mut app, FIND, json!(true));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(fills(&app), ("#00ff00".into(), "#bfbfff".into()), "on: the other art dimmed");
        set(&mut app, "preview", json!(false));
        frame(&mut app, &ctx, 0.3, vec![], 0);
        assert_eq!(fills(&app), ("#ff0000".into(), "#bfbfff".into()), "Preview off: the current colors");
        set(&mut app, "__sel", json!([1]));
        frame(&mut app, &ctx, 0.4, vec![], 0);
        assert_eq!(fills(&app), ("#ffbfbf".into(), "#0000ff".into()), "the other row selected");
        set(&mut app, FIND, json!(false));
        frame(&mut app, &ctx, 0.5, vec![], 0);
        assert_eq!(fills(&app), ("#ff0000".into(), "#0000ff".into()), "off with Preview off: the art as it is");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "Find records no undo step");
        set(&mut app, FIND, json!(true));
        frame(&mut app, &ctx, 0.6, vec![], 0);
        crate::dialogs::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert_eq!(fills(&app), ("#ff0000".into(), "#0000ff".into()), "Cancel: the art as it was");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        // A black square joins the selection: Preserve Black leaves it out of the rows.
        let black = square(&mut app, 70.0, "#000000");
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("select.set", json!({"ids": [red, blue, black]})).unwrap();
        app.run("ui.recolorDialog", json!({})).unwrap();
        set(&mut app, "method", json!(Method::Exact.id()));
        frame(&mut app, &ctx, 1.0, vec![], 0);
        set(&mut app, "rows", rs);
        set(&mut app, "__sel", json!([0]));
        set(&mut app, FIND, json!(true));
        frame(&mut app, &ctx, 1.1, vec![], 0);
        assert_eq!(fills(&app), ("#00ff00".into(), "#bfbfff".into()));
        assert_eq!(fill_hex(&app, black), "#bfbfbf", "the preserved black dims too");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(fills(&app), ("#00ff00".into(), "#ffff00".into()), "OK applies the rows to all the art");
        assert_eq!(fill_hex(&app, black), "#000000", "and keeps the preserved black");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// In a CMYK document colors opened alone are saved as a new color group in their own model.
    #[test]
    fn a_new_group_from_colors_keeps_their_model_in_a_cmyk_document() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"})).unwrap();
        app.run("ui.recolorDialog", json!({"colors": ["#ff8000"]})).unwrap();
        let d = app.ui.dialog.clone().expect("Recolor Artwork opens");
        confirm(&mut app, &d).unwrap();
        let g = app.session.doc().unwrap().doc.swatch_groups.last().expect("a new color group");
        assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), [Color::rgb8(255, 128, 0)]);
    }

    /// Undo Changes keeps at most [`MAX_STEPS`] steps and [`MAX_STEP_BYTES`] of them, the oldest
    /// going first; fields that change nothing make no step, and with fields larger than the limit
    /// only the newest step is kept, so Undo Changes is never on with no step to go to.
    #[test]
    fn undo_changes_keeps_a_bounded_history() {
        let mut d = Dialog::new(KIND, json!({"rows": [], "colors": null}));
        let mut h = History::default();
        h.note(&d);
        h.note(&d);
        assert!(h.undo.is_empty(), "no change, no step");
        for n in 0..MAX_STEPS + 6 {
            d.fields.insert("colors".into(), json!(n));
            h.note(&d);
        }
        assert_eq!(h.undo.len(), MAX_STEPS, "the oldest steps go first");
        assert_eq!(h.undo[0]["colors"], json!(5));
        // Rows of 100 KiB each: the steps kept stay within the byte limit.
        let big = |n: usize| json!([{"from": [format!("{n}{}", "x".repeat(100 << 10))], "to": null}]);
        for n in 0..60 {
            d.fields.insert("rows".into(), big(n));
            h.note(&d);
        }
        let size = |s: &Snapshot| serde_json::to_vec(s).unwrap().len();
        let total: usize = h.base.iter().chain(&h.undo).map(size).sum();
        assert!(total <= MAX_STEP_BYTES, "{total}");
        assert!(h.undo.len() < 60 && !h.undo.is_empty(), "{}", h.undo.len());
        assert!(h.step(&mut d, true), "the newest step is kept");
        assert_eq!(d.fields["rows"], big(58));
        // Current fields past the limit alone: only the step to them is kept, so Undo Changes
        // still goes back one step.
        d.fields.insert("rows".into(), json!([{"from": ["x".repeat(MAX_STEP_BYTES)], "to": null}]));
        h.note(&d);
        assert!(h.undo.len() == 1 && h.redo.is_empty(), "{}", h.undo.len());
        d.fields.insert("colors".into(), json!(1));
        assert!(h.can(&d, true), "the change to colors is a step too");
        assert!(h.step(&mut d, true), "and it is kept");
        assert_eq!(d.fields["colors"], json!(MAX_STEPS + 5), "Undo Changes gives back the colors before it");
        assert_eq!(h.undo.len(), 0, "the step before it went to keep the newest");
        assert!(!h.can(&d, true));
    }

    /// Sets `field` of the open dialog to `value` as `ui.dialog.set` does: a refused value leaves
    /// the dialog as it was.
    fn set(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let before = app.ui.dialog.clone();
        let old = app.ui.dialog.as_ref().and_then(|d| d.fields.get(field).cloned());
        if let Some(d) = app.ui.dialog.as_mut() {
            crate::dialogs::set_field(d, field.into(), value);
        }
        let r = crate::dialogs::field_set(app, field, old.as_ref());
        if r.is_err() {
            app.ui.dialog = before;
        }
        r
    }

    fn group_hex(app: &VectorcraftApp, name: &str) -> Option<Vec<String>> {
        let doc = &app.session.doc().unwrap().doc;
        doc.swatch_groups.iter().find(|g| g.name == name).map(|g| g.swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect())
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// Sets `groupAction` to `a` as `ui.dialog.set` does and draws the frame that runs it.
    fn act(app: &mut VectorcraftApp, ctx: &egui::Context, t: f64, a: &str) -> Result<(), String> {
        set(app, GROUP_ACTION, json!(a))?;
        frame(app, ctx, t, vec![], 0);
        Ok(())
    }

    /// Without art: a group picked in the Color Groups list (the `group` field) gives the rows its
    /// colors, and a name no group has is refused. Save Changes rewrites the selected group with the
    /// rows' new colors and the name field, as one step, and records none when nothing changes;
    /// Delete removes it, after which OK saves the rows as a new group; New Color Group adds one.
    #[test]
    fn the_color_groups_list_selects_saves_and_deletes_groups() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "A", "colors": ["#ff0000", "#00ff00"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "B", "colors": ["#0000ff", "#ffff00", "#ffffff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "A"})).unwrap();
        assert_eq!(new_colors(&app), [json!("rgb 255 0 0"), json!("rgb 0 255 0")]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields[GROUP_STORAGE], true, "the list opens shown");

        set(&mut app, "group", json!("B")).unwrap();
        assert_eq!(new_colors(&app), [json!("rgb 0 0 255"), json!("rgb 255 255 0"), json!("rgb 255 255 255")]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("groupName"), "B");
        assert_eq!(set(&mut app, "group", json!("Nope")), Err("no colour group `Nope`".into()));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["group"], "B", "a refused name leaves B selected");
        assert!(set(&mut app, GROUP_STORAGE, json!("yes")).is_err());
        assert!(set(&mut app, GROUP_ACTION, json!("rename")).is_err());

        let undo = undo_len(&app);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        rs[0]["to"] = json!("rgb 0 0 0");
        d.fields.insert("rows".into(), Value::Array(rs));
        d.fields.insert("groupName".into(), json!("Night"));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Night"), Some(vec!["#000000".into(), "#ffff00".into(), "#ffffff".into()]));
        assert_eq!(group_hex(&app, "B"), None, "renamed");
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["group"], "Night");
        assert_eq!(undo_len(&app), undo + 1, "one step");
        act(&mut app, &ctx, 0.2, "save").unwrap();
        assert_eq!(undo_len(&app), undo + 1, "saving the same colors again records no step");

        act(&mut app, &ctx, 0.3, "delete").unwrap();
        assert_eq!(group_hex(&app, "Night"), None);
        assert_eq!(undo_len(&app), undo + 2);
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.fields["group"].is_null() && d.bool("__newGroup"), "{:?}", d.fields);
        assert_eq!(act(&mut app, &ctx, 0.4, "delete"), Err("missing `group`".into()));
        assert_eq!(act(&mut app, &ctx, 0.5, "save"), Err("missing `group`".into()));
        assert_eq!(undo_len(&app), undo + 2, "a refused button records no step");

        let n = app.session.doc().unwrap().doc.swatch_groups.len();
        act(&mut app, &ctx, 0.6, "new").unwrap();
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.swatch_groups.len(), n + 1);
        let last: Vec<String> = doc.swatch_groups.last().unwrap().swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect();
        assert_eq!(last, ["#000000", "#ffff00", "#ffffff"]);
        assert_eq!(group_hex(&app, "A"), Some(vec!["#ff0000".into(), "#00ff00".into()]), "other groups are left as they are");
    }

    /// Without art: rows of a group picked in the list keep their swatches when a color bar moves,
    /// at OK and at Save Changes, and an excluded row's swatch stays as it is.
    #[test]
    fn a_moved_row_of_a_listed_group_keeps_its_swatch_at_save_changes() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "A", "colors": ["#000000"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "G", "colors": ["#ff0000", "#0000ff", "#00ff00"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "A"})).unwrap();
        set(&mut app, "group", json!("G")).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        rs[1]["exclude"] = json!(true);
        rs[2]["to"] = json!("#ffff00");
        let active = active_rows(&rs);
        // Drag the last bar (green, now yellow) above the first (red).
        assert_eq!(move_bar(&mut rs, &active, 1, 0), Some(0));
        d.fields.insert("rows".into(), Value::Array(rs));
        assert_eq!(apply_params(d)["groupColors"][1], json!("keep"));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "G"), Some(vec!["#ff0000".into(), "#0000ff".into(), "#ffff00".into()]));
    }

    /// Without art: selecting another color group in the list and Save Changes leave Undo Changes
    /// and Redo Changes no step to go to, so OK gives the selected group the rows read for it.
    #[test]
    fn another_group_or_save_changes_leaves_nothing_to_undo() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "A", "colors": ["#ff0000", "#00ff00"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "B", "colors": ["#0000ff", "#ffff00", "#ffffff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "A"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let edit = |app: &mut VectorcraftApp, to: &str| {
            let d = app.ui.dialog.as_mut().unwrap();
            let mut rs = super::rows(d);
            rs[0]["to"] = json!(to);
            d.fields.insert("rows".into(), Value::Array(rs));
        };
        edit(&mut app, "rgb 0 0 0");
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert!(super::can_step(&app, true), "the edit is a step");

        set(&mut app, "group", json!("B")).unwrap();
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert!(!super::can_step(&app, true) && !super::can_step(&app, false), "another group leaves no step");
        assert_eq!(super::step(&mut app, true).unwrap()["changed"], json!(false));
        assert_eq!(new_colors(&app).len(), 3, "the rows of B");

        edit(&mut app, "rgb 0 128 0");
        frame(&mut app, &ctx, 0.3, vec![], 0);
        assert!(super::can_step(&app, true), "the edit is a step");
        act(&mut app, &ctx, 0.4, "save").unwrap();
        assert_eq!(group_hex(&app, "B"), Some(vec!["#008000".into(), "#ffff00".into(), "#ffffff".into()]));
        assert!(!super::can_step(&app, true) && !super::can_step(&app, false), "Save Changes leaves no step");
        assert_eq!(super::step(&mut app, true).unwrap()["changed"], json!(false));

        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(group_hex(&app, "B"), Some(vec!["#008000".into(), "#ffff00".into(), "#ffffff".into()]), "OK keeps B's saved colors");
        assert_eq!(group_hex(&app, "A"), Some(vec!["#ff0000".into(), "#00ff00".into()]), "and leaves A as it was");
    }

    /// Without art: rows set after another color group is selected, with no frame between, are a
    /// step back to the rows read for that group.
    #[test]
    fn an_edit_right_after_another_group_is_a_step() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "A", "colors": ["#ff0000", "#00ff00"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "B", "colors": ["#0000ff", "#ffff00", "#ffffff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "A"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        set(&mut app, "group", json!("B")).unwrap();
        assert!(!super::can_step(&app, true) && !super::can_step(&app, false), "another group leaves no step");
        let b = new_colors(&app);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        rs[0]["to"] = json!("rgb 0 0 0");
        d.fields.insert("rows".into(), Value::Array(rs));
        assert!(super::can_step(&app, true), "the edit is a step");
        assert_eq!(super::step(&mut app, true).unwrap()["changed"], json!(true));
        assert_eq!(new_colors(&app), b, "Undo Changes gives back the rows read for B");
        assert_eq!(super::step(&mut app, false).unwrap()["changed"], json!(true));
        assert_eq!(new_colors(&app)[0], json!("rgb 0 0 0"), "Redo Changes gives back the edit");
    }

    /// With art not linked to the group: Save Changes with the rows as the last Save Changes left
    /// them, after an edit and Undo Changes, leaves Redo Changes no step to go to either.
    #[test]
    fn an_unchanged_save_changes_leaves_nothing_to_redo() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let edit = |app: &mut VectorcraftApp, to: &str| {
            let d = app.ui.dialog.as_mut().unwrap();
            let mut rs = super::rows(d);
            rs[0]["to"] = json!(to);
            d.fields.insert("rows".into(), Value::Array(rs));
        };
        edit(&mut app, "rgb 0 128 0");
        frame(&mut app, &ctx, 0.1, vec![], 0);
        act(&mut app, &ctx, 0.2, "save").unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().fields.contains_key(SAVED_ROWS), "the art kept its colors");
        edit(&mut app, "rgb 0 0 0");
        frame(&mut app, &ctx, 0.3, vec![], 0);
        assert_eq!(super::step(&mut app, true).unwrap()["changed"], json!(true));
        assert!(unchanged_since_save(app.ui.dialog.as_ref().unwrap()), "the rows are as saved");
        assert!(super::can_step(&app, false), "the edit can be redone");
        act(&mut app, &ctx, 0.4, "save").unwrap();
        assert!(!super::can_step(&app, true) && !super::can_step(&app, false), "Save Changes leaves no step");
    }

    /// Without art: after a group is picked in the list, the same Color Theme Picker picks that gave
    /// the group before it its colors give the new group's rows the palette of the art picked too.
    #[test]
    fn theme_picks_apply_again_after_another_group_is_picked() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        square(&mut app, 100.0, "#00ff00");
        app.run("select.set", json!({"ids": []})).unwrap();
        app.run("swatch.newGroup", json!({"name": "A", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "B", "colors": ["#0000ff", "#ffff00", "#ffffff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "A"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let picks = json!([{"at": [107, 17]}]);
        app.ui.dialog.as_mut().unwrap().fields.insert(THEME_PICKS.into(), picks.clone());
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert_eq!(new_colors(&app), [json!("rgb 0 255 0"), json!("rgb 0 255 0")]);
        set(&mut app, "group", json!("B")).unwrap();
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(new_colors(&app), [json!("rgb 0 0 255"), json!("rgb 255 255 0"), json!("rgb 255 255 255")]);
        app.ui.dialog.as_mut().unwrap().fields.insert(THEME_PICKS.into(), picks);
        frame(&mut app, &ctx, 0.3, vec![], 0);
        assert_eq!(new_colors(&app), [json!("rgb 0 255 0"), json!("rgb 0 255 0"), json!("rgb 0 255 0")], "the picks apply to B's rows");
    }

    /// Save Changes on a group with a swatch that is not a color is refused, renamed or not, and
    /// leaves the group as it is: the rows hold only its colors.
    #[test]
    fn saving_a_group_with_a_swatch_that_is_not_a_color_is_refused() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Mixed", "colors": ["#ff0000", "#00ff00"], "keepModel": true})).unwrap();
        let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
        let g = doc.swatch_groups.iter_mut().find(|g| g.name == "Mixed").unwrap();
        g.swatches.insert(1, fade_swatch());
        app.run("ui.recolorDialog", json!({"group": "Mixed"})).unwrap();
        assert_eq!(new_colors(&app), [json!("rgb 255 0 0"), json!("rgb 0 255 0")]);
        let undo = undo_len(&app);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        rs[0]["to"] = json!("rgb 0 0 0");
        d.fields.insert("rows".into(), Value::Array(rs));
        let refused: Result<(), String> = Err("color group `Mixed` holds swatches that are not colors, which Save Changes can't keep".into());
        assert_eq!(act(&mut app, &ctx, 0.1, "save"), refused);
        app.ui.dialog.as_mut().unwrap().fields.insert("groupName".into(), json!("Renamed"));
        assert_eq!(act(&mut app, &ctx, 0.2, "save"), refused);
        // OK first runs a save set together with it, and the dialog stays open.
        app.ui.dialog.as_mut().unwrap().fields.insert(GROUP_ACTION.into(), json!("save"));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Mixed").unwrap();
        assert_eq!(g.swatches[1], fade_swatch(), "the swatch is kept");
        assert_eq!(group_hex(&app, "Mixed"), Some(vec!["#ff0000".into(), "#00ff00".into()]));
        assert_eq!(undo_len(&app), undo, "no step");
    }

    /// A gradient swatch, for a color group.
    fn fade_swatch() -> vectorcraft_color::Swatch {
        let paint = vectorcraft_color::Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
        vectorcraft_color::Swatch { name: "Fade".into(), paint, global: false, spot: false }
    }

    /// With art, the preview rewrites the group being edited and keeps its other swatch; Save
    /// Changes is still refused, as the group is without the preview, and Cancel leaves it as it was.
    #[test]
    fn saving_a_group_with_a_swatch_that_is_not_a_color_is_refused_while_previewing() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Mixed", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
        let g = doc.swatch_groups.iter_mut().find(|g| g.name == "Mixed").unwrap();
        g.swatches.push(fade_swatch());
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Mixed"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let undo = undo_len(&app);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        let row_of = |rs: &[Value], hex: &str| rs.iter().position(|r| super::from_colors(r).iter().any(|c| c.to_hex() == hex)).unwrap();
        let red_row = row_of(&rs, "#ff0000");
        rs[red_row]["to"] = json!("rgb 0 0 0");
        let blue_to = super::to_color(&rs[row_of(&rs, "#0000ff")]).unwrap().to_hex();
        d.fields.insert("rows".into(), Value::Array(rs));
        frame(&mut app, &ctx, 0.1, vec![], 0);
        assert!(app.session.in_interaction(), "the art previews");
        let previewed = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Mixed").map(|g| g.swatches.clone());
        assert_eq!(previewed.as_ref().map(Vec::len), Some(3), "the preview's rewrite of the group keeps its other swatch");
        assert_eq!(previewed.and_then(|s| s.last().cloned()), Some(fade_swatch()), "with its paint");
        assert_eq!(group_hex(&app, "Mixed"), Some(vec!["#000000".into(), blue_to]), "each color swatch takes its row's new color");
        let refused: Result<(), String> = Err("color group `Mixed` holds swatches that are not colors, which Save Changes can't keep".into());
        assert_eq!(act(&mut app, &ctx, 0.2, "save"), refused);
        crate::dialogs::cancel(&mut app);
        let g = app.session.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Mixed").unwrap();
        assert_eq!(g.swatches.last(), Some(&fade_swatch()), "the swatch is kept");
        assert_eq!(group_hex(&app, "Mixed"), Some(vec!["#ff0000".into(), "#0000ff".into()]));
        assert_eq!(undo_len(&app), undo, "no step");
    }

    /// Save Changes on a group whose global swatches the selected art is linked to: the art takes
    /// the group's new colors, and the rows are read from the art again, so they list its colors as
    /// they are now. A color the art had before keeps its row's new color; a color the save gave
    /// it stays as it is.
    #[test]
    fn saving_a_group_the_art_links_to_reads_the_rows_again() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if r["from"].as_array().is_some_and(|f| f.contains(&json!("rgb 255 0 0"))) {
                r["to"] = json!("rgb 0 128 0");
            }
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        frame(&mut app, &ctx, 0.2, vec![], 0);
        // The rows took the group's colors in their order, so one of the swatches is now green.
        let saved = group_hex(&app, "Linked").unwrap();
        assert!(saved.len() == 2 && saved.contains(&"#008000".to_string()), "{saved:?}");
        let mut want = saved.clone();
        want.sort();
        assert_eq!(fill_hex(&app, red), "#008000", "the red square keeps the new color its row gave it");
        for r in super::rows(app.ui.dialog.as_ref().unwrap()) {
            if !from_colors(&r).iter().any(|c| ["#ff0000", "#0000ff"].contains(&c.to_hex().as_str())) {
                assert!(excluded(&r) && r["to"] == r["from"][0], "a color the save gave the art stays: {r}");
            }
        }
        let mut froms: Vec<String> = super::rows(app.ui.dialog.as_ref().unwrap()).iter().flat_map(from_colors).map(|c| c.to_hex()).collect();
        froms.sort();
        assert_eq!(froms, want, "the rows list the art's colors as they are now");
        assert!(apply_params(app.ui.dialog.as_ref().unwrap()).get("group").is_none(), "OK leaves the saved group as it is");
        // OK with only a new name keeps the preview as one step and renames the group, keeping its
        // saved colors.
        let undo = undo_len(&app);
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("groupName".into(), json!("Renamed"));
        confirm(&mut app, &d).unwrap();
        assert_eq!(group_hex(&app, "Renamed"), Some(saved));
        assert_eq!(group_hex(&app, "Linked"), None);
        assert_eq!(fill_hex(&app, red), "#008000");
        assert_eq!(undo_len(&app), undo + 1);
    }

    /// OK with only a new name after Save Changes leaves every swatch of the group its exact color:
    /// a CMYK swatch no row lists keeps components finer than a hundredth of a percent.
    #[test]
    fn renaming_a_saved_group_keeps_its_exact_colors() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let fine = Color::cmyk(0.123_456, 0.654_321, 0.0, 0.010_101);
        let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
        let g = doc.swatch_groups.iter_mut().find(|g| g.name == "Linked").unwrap();
        g.swatches.push(vectorcraft_color::Swatch { name: "Fine".into(), paint: vectorcraft_color::Paint::solid(fine), global: false, spot: false });
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if r["from"].as_array().is_some_and(|f| f.contains(&json!("rgb 255 0 0"))) {
                r["to"] = json!("rgb 0 128 0");
            }
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        frame(&mut app, &ctx, 0.2, vec![], 0);
        let swatches = |app: &VectorcraftApp, name: &str| {
            let doc = &app.session.doc().unwrap().doc;
            doc.swatch_groups.iter().find(|g| g.name == name).map(|g| g.swatches.iter().map(|w| w.paint.color()).collect::<Vec<_>>())
        };
        let saved = swatches(&app, "Linked").unwrap();
        assert_eq!(saved.last(), Some(&Some(fine)), "Save Changes keeps the swatch no row lists");
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("groupName".into(), json!("Renamed"));
        confirm(&mut app, &d).unwrap();
        assert_eq!(swatches(&app, "Renamed"), Some(saved), "the rename keeps every color exactly");
    }

    /// Save Changes where one row's new color is another row's current color: the art linked to the
    /// group keeps the colors the save gave it, and is not recolored again by the other row; a
    /// Global Adjust set afterwards gives OK the group again.
    #[test]
    fn saving_a_group_keeps_the_colors_it_gave_the_linked_art() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            let from = from_colors(r).first().map(|c| c.to_hex());
            r["to"] = match from.as_deref() {
                Some("#ff0000") => json!("rgb 0 0 255"),
                _ => json!("rgb 0 128 0"),
            };
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        frame(&mut app, &ctx, 0.2, vec![], 0);
        let mut group = group_hex(&app, "Linked").unwrap();
        group.sort();
        let mut fills = vec![fill_hex(&app, red), fill_hex(&app, blue)];
        fills.sort();
        assert_eq!(fills, group, "the art shows the saved group's colors, not a second recolor");
        assert!(apply_params(app.ui.dialog.as_ref().unwrap()).get("group").is_none());
        app.ui.dialog.as_mut().unwrap().fields.insert("globalAdjust".into(), json!({"brightness": -50.0}));
        assert!(apply_params(app.ui.dialog.as_ref().unwrap()).get("group").is_some(), "a new Global Adjust rewrites the group at OK");
    }

    /// Save Changes with art in only some colors of the group: the swatches keep their order, and a
    /// swatch no row lists keeps its color.
    #[test]
    fn saving_a_group_keeps_the_swatches_the_art_does_not_use() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let red = square(&mut app, 10.0, "#ff0000");
        let blue = square(&mut app, 40.0, "#0000ff");
        let yellow = square(&mut app, 70.0, "#ffff00");
        app.run("select.set", json!({"ids": [red, blue, yellow]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let order = group_hex(&app, "Linked").unwrap();
        app.run("select.set", json!({"ids": [blue, yellow]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#0000ff") => json!("rgb 0 128 0"),
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        let want: Vec<String> = order.iter().map(|h| if h == "#0000ff" { "#008000".into() } else { h.clone() }).collect();
        assert_eq!(group_hex(&app, "Linked"), Some(want), "the swatches keep their order and the red one its color");
        assert_eq!(fill_hex(&app, red), "#ff0000", "art linked to a swatch no row lists is left as it is");
    }

    /// Save Changes with art in colors of the group keeps the Edit tab's changes: a removed color's
    /// swatch is removed, and a color added on the Edit tab joins the group after the others.
    #[test]
    fn saving_a_group_keeps_added_and_removed_colors() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let red = square(&mut app, 10.0, "#ff0000");
        let blue = square(&mut app, 40.0, "#0000ff");
        let yellow = square(&mut app, 70.0, "#ffff00");
        app.run("select.set", json!({"ids": [red, blue, yellow]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let order = group_hex(&app, "Linked").unwrap();
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#0000ff") => Value::Null,
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        rs.push(json!({"from": [], "to": "rgb 0 128 0", "added": true}));
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        let mut want: Vec<String> = order.into_iter().filter(|h| h != "#0000ff").collect();
        want.push("#008000".into());
        assert_eq!(group_hex(&app, "Linked"), Some(want.clone()), "the removed swatch goes and the added color follows the others");
        assert_eq!(fill_hex(&app, blue), "#0000ff", "art linked to a removed swatch keeps its color");
        let rs = super::rows(app.ui.dialog.as_ref().unwrap());
        assert!(rs.iter().all(|r| !r["to"].is_null() && !is_added(r)), "the rows follow the saved group: {rs:?}");
        let assign = app.ui.dialog.as_ref().unwrap().fields["__assign"].clone();
        let assign: Vec<String> = assign.as_array().unwrap().iter().filter_map(color_value).map(|c| c.to_hex()).collect();
        assert_eq!(Some(assign), group_hex(&app, "Linked"), "a later edit gives the group no removed swatch back");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(group_hex(&app, "Linked"), Some(want), "OK removes and adds nothing more");
    }

    /// Save Changes that recolors linked art reads the rows from the art again; a color added on the
    /// Edit tab stays one of the group's colors and a removed one does not come back, so OK after a
    /// further edit keeps them as saved.
    #[test]
    fn saving_a_group_that_reads_the_rows_again_keeps_an_added_color() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let red = square(&mut app, 10.0, "#ff0000");
        let blue = square(&mut app, 40.0, "#0000ff");
        let lime = square(&mut app, 70.0, "#00ff00");
        app.run("select.set", json!({"ids": [red, blue, lime]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#ff0000") => json!("rgb 0 128 0"),
                Some("#00ff00") => Value::Null,
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        rs.push(json!({"from": [], "to": "rgb 255 255 0", "added": true}));
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(fill_hex(&app, red), "#008000", "the save recolored the linked art");
        assert_eq!(fill_hex(&app, lime), "#00ff00", "art linked to the removed swatch keeps its color");
        let group = group_hex(&app, "Linked").unwrap();
        assert!(group.contains(&"#ffff00".to_string()) && !group.contains(&"#00ff00".to_string()), "{group:?}");
        let mut d = app.ui.dialog.clone().unwrap();
        let mut rs = super::rows(&d);
        assert!(rs.iter().all(|r| !is_added(r)), "{rs:?}");
        if let Some(r) = rs.last_mut() {
            r["to"] = json!("rgb 10 10 10");
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        let colors = sent_by(&app, &d);
        assert_eq!(colors.iter().filter(|c| *c == "#ffff00").count(), 1, "OK after a further edit keeps the added color once: {colors:?}");
        assert!(!colors.contains(&"#00ff00".to_string()), "nor brings the removed color back: {colors:?}");
    }

    /// Save Changes where the rows list their current colors as "#rrggbb": a row matches the swatch
    /// of the same color, so the swatches keep their order and the one a row lists takes its new
    /// color; read again afterwards, a row whose old entry was a hex keeps its new color.
    #[test]
    fn saving_a_group_matches_rows_that_list_hex_colors() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let red = square(&mut app, 10.0, "#ff0000");
        let blue = square(&mut app, 40.0, "#0000ff");
        let yellow = square(&mut app, 70.0, "#ffff00");
        app.run("select.set", json!({"ids": [red, blue, yellow]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        let order = group_hex(&app, "Linked").unwrap();
        let orange = square(&mut app, 100.0, "#ff8000");
        app.run("select.set", json!({"ids": [blue, yellow, orange]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            let hex: Vec<String> = from_colors(r).iter().map(|c| c.to_hex()).collect();
            r["to"] = match hex.first().map(String::as_str) {
                Some("#0000ff") => json!("rgb 0 128 0"),
                Some("#ff8000") => json!("rgb 128 0 128"),
                _ => r["from"][0].clone(),
            };
            r["from"] = json!(hex);
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        let want: Vec<String> = order.iter().map(|h| if h == "#0000ff" { "#008000".into() } else { h.clone() }).collect();
        assert_eq!(group_hex(&app, "Linked"), Some(want), "the blue swatch takes the green, the red one keeps its color");
        assert_eq!((fill_hex(&app, red), fill_hex(&app, blue)), ("#ff0000".into(), "#008000".into()));
        let rs = super::rows(app.ui.dialog.as_ref().unwrap());
        let o = rs.iter().find(|r| from_colors(r).iter().any(|c| c.to_hex() == "#ff8000")).unwrap();
        assert!(!excluded(o) && o["to"] == json!("rgb 128 0 128"), "the orange row keeps its new color: {o}");
    }

    /// Save Changes with art in the group's colors that is not linked to its swatches: the art is
    /// left to the preview, and OK leaves the saved group as it is until the rows change.
    #[test]
    fn saving_a_group_with_unlinked_art_keeps_the_saved_group_at_ok() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            let to = match from_colors(r).first().map(|c| c.to_hex()).as_deref() {
                Some("#0000ff") => json!("rgb 0 128 0"),
                _ => r["from"][0].clone(),
            };
            r["to"] = to;
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#ff0000".into(), "#008000".into()]));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(fill_hex(&app, blue), "#008000", "the art previews its row's new color");
        assert!(apply_params(app.ui.dialog.as_ref().unwrap()).get("group").is_none(), "OK leaves the saved group as it is");
    }

    /// The colors `d`'s OK gives the selected group, as hex, `"keep"` read as the color the swatch
    /// at its place has.
    fn sent_by(app: &VectorcraftApp, d: &Dialog) -> Vec<String> {
        let own = group_hex(app, &d.str("group")).unwrap_or_default();
        let p = apply_params(d);
        let hex = |(i, k): (usize, &Value)| if *k == json!("keep") { own.get(i).cloned() } else { color_value(k).map(|c| c.to_hex()) };
        p["groupColors"].as_array().unwrap().iter().enumerate().filter_map(hex).collect()
    }

    /// The colors OK gives the selected group, as [`sent_by`] reads them.
    fn sent_hex(app: &VectorcraftApp) -> Vec<String> {
        sent_by(app, app.ui.dialog.as_ref().unwrap())
    }

    /// Sets the new color of the rows whose first current color is `hex` to `to`.
    fn set_row(app: &mut VectorcraftApp, hex: &str, to: Value) {
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some(hex) {
                r["to"] = to.clone();
            }
        }
        d.fields.insert("rows".into(), Value::Array(rs));
    }

    /// With unlinked art in some of the group's colors, a second Save Changes after a further edit of
    /// a row gives the swatch the first save gave that row's color the row's new color, and OK does
    /// too; the swatch no row lists keeps its color both times.
    #[test]
    fn saving_a_group_twice_with_unlinked_art_gives_a_row_its_swatch_again() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff", "#ffff00"], "keepModel": true})).unwrap();
        let (blue, yellow) = (square(&mut app, 10.0, "#0000ff"), square(&mut app, 40.0, "#ffff00"));
        app.run("select.set", json!({"ids": [blue, yellow]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        set_row(&mut app, "#0000ff", json!("rgb 0 128 0"));
        set_row(&mut app, "#ffff00", json!("rgb 255 255 0"));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#ff0000".into(), "#008000".into(), "#ffff00".into()]));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        set_row(&mut app, "#0000ff", json!("rgb 128 0 128"));
        let sent = sent_hex(&app);
        assert_eq!(sent, ["#ff0000", "#800080", "#ffff00"], "OK gives the blue row's swatch its new color");
        act(&mut app, &ctx, 0.3, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#ff0000".into(), "#800080".into(), "#ffff00".into()]), "the second save does too");
    }

    /// Without art, an excluded row leaves its swatch its own color at Save Changes and at OK, even
    /// when its new color was changed before it was excluded.
    #[test]
    fn saving_a_group_without_art_keeps_an_excluded_rows_color() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some("#ff0000") {
                r["to"] = json!("rgb 0 128 0");
                r["exclude"] = json!(true);
            }
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        let sent = sent_hex(&app);
        assert_eq!(sent, ["#ff0000", "#0000ff"], "OK keeps the excluded row's swatch");
        let p = apply_params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(p["groupColors"][0], "keep", "so the Global Adjust leaves it as it is too: {p}");
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#ff0000".into(), "#0000ff".into()]), "so does Save Changes");
    }

    /// With art in more colors than the group has swatches, an excluded row past the swatches adds
    /// no color at Save Changes, as at OK.
    #[test]
    fn saving_a_group_with_art_adds_no_color_for_an_excluded_row_past_its_swatches() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000"], "keepModel": true})).unwrap();
        let (blue, yellow) = (square(&mut app, 10.0, "#0000ff"), square(&mut app, 40.0, "#ffff00"));
        app.run("select.set", json!({"ids": [blue, yellow]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("method".into(), json!(Method::Exact.id()));
        d.fields.insert("colors".into(), json!(2));
        d.fields.remove(REDUCED);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        assert_eq!(rs.len(), 2, "a row for each color of the art: {rs:?}");
        for r in &mut rs {
            r["exclude"] = json!(true);
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        // `recolor.apply` adds nothing for a "keep" past the group's swatches.
        assert_eq!(apply_params(app.ui.dialog.as_ref().unwrap())["groupColors"], json!(["keep", "keep"]), "OK adds no color");
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#ff0000".into()]), "Save Changes adds none either");
    }

    /// Without art, a row excluded after Save Changes gave its swatch a new color leaves the swatch
    /// that saved color at a second Save Changes, as OK does.
    #[test]
    fn saving_a_group_without_art_keeps_a_saved_color_when_its_row_is_excluded() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        set_row(&mut app, "#ff0000", json!("rgb 0 128 0"));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#008000".into(), "#0000ff".into()]));
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if r["to"] == json!("rgb 0 128 0") {
                r["exclude"] = json!(true);
            }
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        assert_eq!(sent_hex(&app), ["#008000", "#0000ff"], "OK keeps the saved color");
        act(&mut app, &ctx, 0.2, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#008000".into(), "#0000ff".into()]), "so does a second Save Changes");
    }

    /// Save Changes that recolors linked art so that one swatch takes another swatch's old color:
    /// read again, the row of the second swatch's new color gives that swatch a later new color.
    #[test]
    fn saving_a_group_twice_with_linked_art_finds_the_row_of_a_swatchs_new_color() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Linked", "fromArtwork": true, "toGlobal": true})).unwrap();
        assert_eq!(group_hex(&app, "Linked"), Some(vec!["#ff0000".into(), "#0000ff".into()]));
        app.run("ui.recolorDialog", json!({"group": "Linked"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        set_row(&mut app, "#ff0000", json!("rgb 0 0 255"));
        set_row(&mut app, "#0000ff", json!("rgb 0 128 0"));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        frame(&mut app, &ctx, 0.2, vec![], 0);
        assert_eq!(group_hex(&app, "Linked"), Some(vec!["#0000ff".into(), "#008000".into()]));
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some("#008000") {
                r["to"] = json!("rgb 128 0 128");
                r["exclude"] = json!(false);
            }
        }
        d.fields.insert("rows".into(), Value::Array(rs));
        assert_eq!(sent_hex(&app), ["#0000ff", "#800080"], "OK gives the green swatch the green row's new color");
        act(&mut app, &ctx, 0.3, "save").unwrap();
        assert_eq!(group_hex(&app, "Linked"), Some(vec!["#0000ff".into(), "#800080".into()]), "so does Save Changes");
    }

    /// Save Changes with a Global Adjust: the saved group holds it, the rows' new colors take it and
    /// it goes back to 0%, so OK with a new name renames the group without adjusting it again.
    #[test]
    fn saving_a_group_with_a_global_adjust_applies_it_once() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("method".into(), json!(Method::Exact.id()));
        d.fields.insert("globalAdjust".into(), json!({"brightness": -50.0}));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        act(&mut app, &ctx, 0.1, "save").unwrap();
        let saved = group_hex(&app, "Plain").unwrap();
        assert!(!saved.contains(&"#ff0000".to_string()), "the save darkened the group: {saved:?}");
        frame(&mut app, &ctx, 0.2, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(adjust_values(d), [0.0; 4]);
        d.fields.insert("groupName".into(), json!("Dark"));
        let p = apply_params(app.ui.dialog.as_ref().unwrap());
        assert!(p.get("globalAdjust").is_none(), "{p}");
        assert_eq!(p["groupColors"], json!(vec!["keep"; saved.len()]), "OK keeps each saved color");
        let d = app.ui.dialog.clone().unwrap();
        confirm(&mut app, &d).unwrap();
        assert_eq!(group_hex(&app, "Dark"), Some(saved), "OK renames the group with its saved colors");
    }

    /// Without art, Save Changes with a Global Adjust gives the group the rows' new colors with the
    /// adjustment, and OK gives it the same colors again: the adjustment applies to the rows' new
    /// colors, not to the saved group.
    #[test]
    fn saving_a_group_without_art_with_a_global_adjust_applies_it_once() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#0000ff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("globalAdjust".into(), json!({"brightness": -50.0}));
        frame(&mut app, &ctx, 0.0, vec![], 0);
        act(&mut app, &ctx, 0.1, "save").unwrap();
        let saved = group_hex(&app, "Plain").unwrap();
        assert!(!saved.contains(&"#ff0000".to_string()), "the save darkened the group: {saved:?}");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(group_hex(&app, "Plain").unwrap(), saved, "OK does not darken the group again");
    }

    /// With Limit to Library, Save Changes gives the group the library's nearest color to a row's
    /// new color, as OK does, also when the rows were set without being snapped.
    #[test]
    fn saving_a_group_snaps_its_colors_to_the_library() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
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
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("limitTo".into(), json!("document"));
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        rs[0]["to"] = json!("rgb 250 250 10");
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(vec!["#ffff00".into()]), "the saved color is the library's");
    }

    /// Without art, Save Changes removes a removed color's swatch and adds a color added on the Edit
    /// tab, and OK afterwards gives the group the same colors: it removes and adds nothing more.
    #[test]
    fn saving_a_group_without_art_with_added_and_removed_colors_keeps_them_at_ok() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Plain", "colors": ["#ff0000", "#00ff00", "#0000ff"], "keepModel": true})).unwrap();
        app.run("ui.recolorDialog", json!({"group": "Plain"})).unwrap();
        frame(&mut app, &ctx, 0.0, vec![], 0);
        let d = app.ui.dialog.as_mut().unwrap();
        let mut rs = super::rows(d);
        for r in &mut rs {
            if from_colors(r).first().map(|c| c.to_hex()).as_deref() == Some("#00ff00") {
                r["to"] = Value::Null;
            }
        }
        // The added color's row was moved ahead of the others.
        rs.insert(0, json!({"from": [], "to": "rgb 255 255 0", "added": true}));
        d.fields.insert("rows".into(), Value::Array(rs));
        act(&mut app, &ctx, 0.1, "save").unwrap();
        let want: Vec<String> = ["#ff0000", "#0000ff", "#ffff00"].map(String::from).to_vec();
        assert_eq!(group_hex(&app, "Plain"), Some(want.clone()), "the save removes green and adds yellow");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(group_hex(&app, "Plain"), Some(want), "OK gives the group the same colors");
    }

    /// With art: a group picked in the list reduces the art to a row per color of the group, which
    /// become the rows' new colors, and the art previews them; the list draws beside the tabs and
    /// hides with `groupStorage`.
    #[test]
    fn a_group_picked_in_the_list_recolors_the_art() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Warm", "colors": ["#ffff00", "#00ff00"], "keepModel": true})).unwrap();
        app.run("swatch.newGroup", json!({"name": "Cool", "colors": ["#0000ff"], "keepModel": true})).unwrap();
        let (red, blue) = (square(&mut app, 10.0, "#ff0000"), square(&mut app, 40.0, "#0000ff"));
        app.run("select.set", json!({"ids": [red, blue]})).unwrap();
        app.run("ui.recolorDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("method".into(), json!(Method::Exact.id()));
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, 0.0, vec![], 0);
        assert_eq!(fill_hex(&app, red), "#ff0000");
        // A button set with the group, before a frame, saves the rows the group gives the art.
        let n = app.session.doc().unwrap().doc.swatch_groups.len();
        set(&mut app, "group", json!("Warm")).unwrap();
        act(&mut app, &ctx, 0.1, "new").unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let mut saved: Vec<String> = doc.swatch_groups.last().unwrap().swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect();
        saved.sort();
        assert_eq!((doc.swatch_groups.len(), saved), (n + 1, vec!["#00ff00".to_string(), "#ffff00".to_string()]));
        frame(&mut app, &ctx, 0.2, vec![], 0);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(super::rows(d).len(), 2);
        assert_eq!(d.str("groupName"), "Warm");
        let mut shown = [fill_hex(&app, red), fill_hex(&app, blue)];
        shown.sort();
        assert_eq!(shown, ["#00ff00", "#ffff00"], "the art previews the group's colors");
        // The name the preview gives the group being edited picks that group in the list.
        app.ui.dialog.as_mut().unwrap().fields.insert("groupName".into(), json!("Sunny"));
        frame(&mut app, &ctx, 0.21, vec![], 0);
        set(&mut app, "group", json!("Sunny")).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((&d.fields["group"], &d.fields[GROUP_LOADED], d.str("groupName")), (&json!("Warm"), &json!("Warm"), "Sunny".into()));
        // A group the document has under that name is picked instead.
        app.ui.dialog.as_mut().unwrap().fields.insert("groupName".into(), json!("Cool"));
        frame(&mut app, &ctx, 0.22, vec![], 0);
        set(&mut app, "group", json!("Cool")).unwrap();
        frame(&mut app, &ctx, 0.23, vec![], 0);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((&d.fields[GROUP_LOADED], super::rows(d).len()), (&json!("Cool"), 1));
        set(&mut app, GROUP_STORAGE, json!(false)).unwrap();
        frame(&mut app, &ctx, 0.3, vec![], 0);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields[GROUP_STORAGE], false);
    }
}
