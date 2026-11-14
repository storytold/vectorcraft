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
//! model), Merge and Separate, Randomize order or
//! saturation and brightness, Reset, the Method and Limit to Library (its folder button imports a
//! library file and limits to it; a library's colours apply as its swatches: a spot colour stays a
//! spot colour, and joins the document's swatches). Edit tab: a hue and
//! saturation wheel with a marker per new colour (drag one; with linked handles they all turn and
//! scale together), a brightness slider, the harmony rules, and the color sliders with their mode
//! menu, whose Global Adjust shows Saturation, Brightness, Temperature and Luminosity (-100% to
//! 100%, 0% changes nothing), which change every new color at once. The buttons beside the wheel
//! show the new colors on the smooth wheel, on a segmented wheel (solid patches of 24 hues by 5
//! saturations; its markers move as freely) or as color bars (one per new color, in row order: click
//! one to select it for the sliders, drag one to move its row, which changes no color); the dialog
//! opens on the smooth wheel. With art, the Prominent Colors bar under the wheel shows the new
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
//! The Color Theme Picker button beside the tabs turns on picking from the canvas: a click on an
//! object takes the palette of the object a Selection tool click takes, a drag takes the palette
//! of the art inside its marquee, and Shift adds to the picks so far (`recolor.pickTheme`). The
//! rows that aren't excluded take the picked colors as their new colors, the most used color first
//! and repeating when the rows outnumber them, and the art previews them; OK keeps them as one
//! undo step. A new count of Colors keeps the picks: the new rows take them again.
//!
//! Fields (`ui.dialog.set`): `rows` ([{from: [keys], to: key, exclude?, tint?: percent of a spot
//! color}]), `colors` (null: Auto, or a count), `method`, `preserveWhite`, `preserveBlack`,
//! `preserveGrays`, `limitTo` (library id, "document" for the document's swatches, "" for none),
//! `group`, `groupName`, `tab` (`assign` or `edit`), `rule` (harmony id), `linked`, `preview`,
//! `sliderMode` (`hsb`, `rgb`, `cmyk`, `lab` or `tint`; unset: the row color's own model, HSB for
//! a gray and Tint for a spot color; `global`: Global Adjust, on the Edit tab), `globalAdjust`
//! (`{saturation, brightness, temperature, luminosity}` in percent, sent as `recolor.apply`'s
//! `globalAdjust` in every slider mode), `display` (the Edit tab's `smooth`, `segmented` or `bars`;
//! unset: `smooth`), `themePicker` (the Color Theme Picker is on), `themePicks` (its picks, as
//! `recolor.pickTheme`'s `picks`; a new list recolors the rows), `weights` (one per row, as
//! `recolor.weigh`'s; a new list moves the rows' current colors toward them). Changing `colors`, a
//! preserve flag or `limitTo` reduces the rows again (and drops `weights`).

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
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

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
    let start: Vec<Value> = if art { vec![] } else { assign.iter().map(|k| json!({"from": [k], "to": k})).collect() };
    // The Preserve checkboxes as they were last confirmed (#2960).
    let kept = |k: &str, default: bool| app.ui.recolor_preserve.get(k).and_then(Value::as_bool).unwrap_or(default);
    let fields = json!({
        "rows": start, "colors": count, "method": Method::ScaleTints.id(),
        "preserveWhite": kept("preserveWhite", true), "preserveBlack": kept("preserveBlack", true),
        "preserveGrays": kept("preserveGrays", false), "limitTo": limit,
        "group": group, "groupName": if new_group { Some(NEW_GROUP) } else { group }, "tab": "assign", "rule": Harmony::Complementary.id(), "linked": true,
        "preview": true, THEME_PICKER: false, THEME_PICKS: [], WEIGHTS: null, "__art": art, "__assign": assign, "__newGroup": new_group,
    });
    let mut d = Dialog::new(KIND, fields);
    // Without art the rows are there from the start: limit them now (art's rows snap as they reduce).
    let mut rs = rows(&d);
    snap(app, &d, &mut rs);
    d.fields.insert("rows".into(), Value::Array(rs));
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
    const DONE: &str = "__reduced";
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
            snap(app, d, &mut rows);
            d.fields.insert("rows".into(), Value::Array(rows));
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
                    d.fields.insert("rows".into(), Value::Array(m.clone()));
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
        ui.interact(r, egui::Id::unique((PROMINENT_PART, n)), Sense::hover()).on_hover_text(format!("{} {}: {share}%", tl!("Color"), i + 1));
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
        Some(_) if adjust.is_identity() => Some(r["to"].clone()),
        Some(c) => {
            let c = adjust.apply(c);
            Some(key(&palette.as_ref().map_or(c, |p| p.nearest(c))))
        }
    })
    .collect()
}

/// The New Color Group menu's request: save the new colors of the rows that aren't excluded (Save
/// All Colors, in row order; a row without one gives its first current color) or of the Prominent Colors bar (Save Prominent Colors, in its order)
/// as a new color group, a step of its own (`swatch.newGroup`).
fn save_group(app: &mut VectorcraftApp, d: &mut Dialog, which: &str) {
    let rs = rows(d);
    let order: Vec<usize> = match which {
        "prominent" => prominent_parts(d, &rs).into_iter().map(|p| p.0).collect(),
        _ => (0..rs.len()).filter(|&i| !excluded(&rs[i])).collect(),
    };
    let colors = group_colors(app, d, order.iter().filter_map(|&i| rs.get(i)));
    if colors.is_empty() {
        return;
    }
    // The group is added to the document itself, not to the preview, which runs again afterwards.
    cancel_preview(app, d);
    if let Err(e) = app.run("swatch.newGroup", json!({"colors": colors, "keepModel": true})) {
        app.status(e);
    }
}

/// The fields of the Color Theme Picker: whether a click or drag on the canvas picks colors, and
/// the picks so far (`recolor.pickTheme`'s `picks`); the picks the rows last took them from.
const THEME_PICKER: &str = "themePicker";
const THEME_PICKS: &str = "themePicks";
const THEME_DONE: &str = "__themePicked";

/// When the picks of the Color Theme Picker changed, give the rows that aren't excluded the
/// palette of the art picked (`recolor.pickTheme`, as many colors as those rows), in row order;
/// with fewer colors than rows, the colors repeat. Picks that touch no art leave the rows as they
/// are.
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
    }
}

/// Set for the frame in which Randomly change color order shuffled the rows' new colors.
const SHUFFLED: &str = "__shuffled";

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

/// The seed for the next Randomize: one past the last, held at `u64::MAX`.
fn next_seed(d: &Dialog) -> u64 {
    (d.f64("__seed", 0.0) as u64).saturating_add(1)
}

fn selection(d: &Dialog) -> Vec<usize> {
    d.fields.get("__sel").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|i| i as usize).collect()).unwrap_or_default()
}

/// `recolor.apply` parameters: the rows, method, library and the group with its new name and
/// colours: the rows' new colours, then (with art) the group's colours no row took.
fn apply_params(d: &Dialog) -> Value {
    let rs = rows(d);
    let group = d.fields.get("group").and_then(Value::as_str).map(|g| {
        let rest =
            d.fields.get("__assign").and_then(Value::as_array).filter(|_| d.bool("__art")).map_or(&[][..], |a| a.get(rs.len()..).unwrap_or_default());
        let colors: Vec<&Value> = rs.iter().map(|r| &r["to"]).chain(rest).collect();
        let name = d.str("groupName");
        let rename = (!name.trim().is_empty() && name != g).then_some(name);
        json!({"group": g, "groupColors": colors, "rename": rename})
    });
    // A row edited with the sliders carries `model: "new"`: its art takes the mode they showed.
    let mut p = json!({"map": rs, "method": d.str("method"), "limitTo": d.str("limitTo")});
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

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    sync(app, d);
    sync_theme(app, d);
    sync_weights(app, d);
    let t = Tokens::get(ui.ctx());
    let picking = d.bool(THEME_PICKER);
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The label is drawn after the menu (right-to-left), so it cannot name it by adjacency.
            if let Some(i) = widgets::dropdown_named(ui, "recolor-preset", Some(tl!("Preset:")), PRESETS[preset_of(d)], &PRESETS, 130.0) {
                set_preset(app, d, i);
            }
            ui.label(egui::RichText::new(tl!("Preset:")).color(t.text_dim));
        });
    });
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
    let (changed, pick) = if d.str("tab") == "edit" { (edit_tab(app, ui, d, &mut rs), None) } else { assign_tab(app, ui, d, &mut rs) };
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
    if let Some(which) = d.fields.remove(SAVE_GROUP) {
        save_group(app, d, which.as_str().unwrap_or_default());
    }
    if d.fields.get("group").is_some_and(Value::is_string) || d.bool("__newGroup") {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Color Group:"));
            form::text(ui, d, "groupName", 200.0);
        });
    }
    sync(app, d);
    if d.bool("__art") {
        form::preview(app, ui, d, "Recolor Artwork", CMD, apply_params(d));
    }
    if let Some(c) = pick.and_then(|i| Some((i, to_color(rows(d).get(i)?)?))) {
        *d = super::color_picker::over_recolor(d, c.1, c.0);
    }
    false
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

/// OK: keep the previewed recolour as one undo step (or apply it, e.g. to a colour group only; or
/// save colours opened alone as a new colour group).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    sync(app, &mut d);
    sync_theme(app, &mut d);
    sync_weights(app, &mut d);
    if d.bool("__newGroup") {
        let colors = group_colors(app, &d, rows(&d).iter().filter(|r| !excluded(r)));
        return form::commit_preview(app, "swatch.newGroup", json!({"name": d.str("groupName"), "colors": colors, "keepModel": true}));
    }
    let r = form::commit_preview(app, CMD, apply_params(&d))?;
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
    ui.painter().rect_filled(r, t.cr_small(), if dim { fill.gamma_multiply(0.35) } else { fill });
    ui.painter().rect_stroke(r, t.cr_small(), egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
}

/// The Assign tab. Returns true when the rows changed, and the row whose new colour was
/// double-clicked.
fn assign_tab(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog, rows: &mut Vec<Value>) -> (bool, Option<usize>) {
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
    let mut pick: Option<usize> = None;
    let shown: Vec<Color> = rows.iter().map(|r| new_shown(app, d, r)).collect();
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
                    chip(ui, nr, shown.get(i).copied().unwrap_or_default(), off);
                    if nresp.clicked() {
                        clicked = Some(i);
                    }
                    if nresp.double_clicked() {
                        pick = Some(i);
                    }
                    ui.add_space(6.0);
                    let hex = crate::panels::color::hex_digits(&to);
                    let hex_name = format!("{} {}", tl!("Color"), i + 1);
                    if let Some(h) = widgets::hex_field_named(ui, ("recolor-hex", i), Some(&hex_name), &hex)
                        && let Some(c) = crate::panels::color::parse_hex(&h)
                    {
                        set_to(row, &keep_model(to, c));
                        changed = true;
                    }
                });
                if sel.contains(&i) {
                    ui.painter().set(bg, egui::Shape::rect_filled(r.response.rect.expand2(vec2(2.0, 1.0)), t.cr(), t.row_selected));
                }
            }
        });
    });
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
                        *rows = r["map"].as_array().cloned().unwrap_or_default();
                        if !sb {
                            d.fields.insert(SHUFFLED.into(), json!(true));
                        }
                        changed = true;
                    }
                    Err(e) => app.status(e.to_string()),
                }
            }
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
    changed |= sliders(app, ui, d, rows);
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
fn new_shown(app: &VectorcraftApp, d: &Dialog, row: &Value) -> Color {
    let to = to_color(row).unwrap_or_default();
    // Only a Lab spot color can show another color; the library lookup is skipped for the rest.
    if matches!(to, Color::Lab { .. }) && d.bool("__art") && spot_of(app, d, &to).is_some() {
        spot_tinted(app, to, row_tint(row))
    } else {
        to.tinted(row_tint(row))
    }
}

/// The `sliderMode` id of `m` (`hsb`, `rgb`, `cmyk`, `lab`).
fn mode_id(m: Mode) -> String {
    serde_json::to_value(m).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The name of the library swatch (Limit to Library) that `recolor.apply` gives new color `c`, when
/// it is a spot color: the first swatch of the library color nearest `c`.
fn spot_of(app: &VectorcraftApp, d: &Dialog, c: &Color) -> Option<String> {
    let lib = d.str("limitTo");
    if lib.is_empty() {
        return None;
    }
    let near = swatchlib::limit_palette(&app.session, &lib)?.nearest(*c);
    swatchlib::library_swatches(&app.session, &lib)?.into_iter().find(|w| w.paint.color() == Some(near)).filter(|w| w.spot).map(|w| w.name)
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

/// What the color sliders edit: the first selected row, its new color, and the name of the
/// library's spot color that color is.
struct SliderTarget {
    row: Option<usize>,
    color: Option<Color>,
    spot: Option<String>,
}

impl SliderTarget {
    fn of(app: &VectorcraftApp, d: &Dialog, rows: &[Value]) -> Self {
        let row = selection(d).into_iter().filter(|i| *i < rows.len()).min();
        let color = row.and_then(|i| to_color(&rows[i]));
        // A row tint acts only on art, so without art a spot color shows the process modes.
        let spot = color.filter(|_| d.bool("__art")).and_then(|c| spot_of(app, d, &c));
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
fn slider_cell(
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

/// The color sliders under the rows, two to a line: they edit the first selected row's new color
/// (the others keep theirs) in the mode of [`mode_menu`], or its tint of a spot color of the
/// library. Without a selected row they are disabled. Returns true when they changed a row.
fn sliders(app: &VectorcraftApp, ui: &mut Ui, d: &mut Dialog, rows: &mut [Value]) -> bool {
    let target = SliderTarget::of(app, d, rows);
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
            Some(a) if sel.contains(&i) && a.len() > 1 => out.extend(a.iter().map(|k| json!({"from": [k], "to": k}))),
            _ => out.push(row),
        }
    }
    *rows = out;
}

/// The rows the Edit tab shows: those with a new colour, not excluded.
fn active_rows(rows: &[Value]) -> Vec<usize> {
    (0..rows.len()).filter(|i| !excluded(&rows[*i]) && to_color(&rows[*i]).is_some()).collect()
}

/// Set row `i`'s new colour to hue/saturation/brightness `hsb` ([`move_on_wheel`]: with `linked`
/// the other rows keep their relation to it). Excluded rows stay.
fn move_color(rows: &mut [Value], i: usize, hsb: [f32; 3], linked: bool) {
    let active = active_rows(rows);
    let Some(k) = active.iter().position(|&j| j == i) else { return };
    let before: Vec<Color> = active.iter().filter_map(|&j| to_color(&rows[j])).collect();
    let mut colors = before.clone();
    move_on_wheel(&mut colors, k, hsb, linked);
    for ((&j, c), old) in active.iter().zip(&colors).zip(&before) {
        if key(c) != key(old) {
            set_to(&mut rows[j], c);
        }
    }
}

/// The rows when a drag on the Edit tab's wheel started.
const DRAG: &str = "__drag";

/// The Edit tab. Returns true when the rows changed.
fn edit_tab(app: &VectorcraftApp, ui: &mut Ui, d: &mut Dialog, rows: &mut [Value]) -> bool {
    let top = ui.cursor().top();
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    let linked = d.bool("linked");
    let active = active_rows(rows);
    let base = selection(d).into_iter().find(|i| active.contains(i)).or(active.first().copied());
    let target = SliderTarget::of(app, d, rows);
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
            let prominent = d.bool("__art") && !prominent_parts(d, rows).is_empty();
            let saved = egui::Popup::menu(&button).show(|ui| {
                if widgets::menu_item_name(ui, tl!("Save All Colors"), rows.iter().any(|r| !excluded(r)), false) {
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
    let display = d.str("display");
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
        });
        ui.vertical(|ui| match display.as_str() {
            "segmented" => w = widgets::segmented_wheel(ui, "recolor-wheel", WHEEL, &colors, marked),
            "bars" => changed |= color_bars(ui, d, rows, &active, base),
            _ => w = widgets::harmony_wheel(ui, "recolor-wheel", WHEEL, &colors, marked),
        });
    });
    // Prominent Colors: the art's new colors, each as wide as its share of the art.
    if art {
        prominent_bar(ui, d, rows);
    }
    if let Some(k) = w.pressed {
        d.fields.insert("__sel".into(), json!([active[k]]));
    }
    // A drag (of a marker or the brightness slider) moves the rows as they were when it started:
    // a new colour read back from its model (CMYK through the colour profile) isn't exactly the
    // colour written, so building on last frame's rows drifted the linked colours (#1733).
    if let Some((k, set)) = w.moved {
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
    if d.str("sliderMode") == GLOBAL {
        let sample = base.and_then(|b| to_color(&rows[b])).unwrap_or(Color::rgb(0.8, 0.45, 0.25));
        global_sliders(ui, d, sample);
    } else {
        changed |= sliders(app, ui, d, rows);
    }
    // The new colours: click one to make it the base colour.
    // The chips' area is placed one item spacing below what is above it, so the spacing is part of the tab's height.
    let min_height = (TAB_H - (ui.cursor().top() - top) - ui.spacing().item_spacing.y).max(0.0);
    super::modal::scrolled_rows(ui, "recolor-edit-colors", |ui| {
        // Keep the usual tab height when it fits; the empty space scrolls with the chips.
        ui.set_min_height(min_height);
        ui.horizontal_wrapped(|ui| {
            for &i in &active {
                let (r, resp) = ui.allocate_exact_size(vec2(28.0, 20.0), Sense::click());
                chip(ui, r, to_color(&rows[i]).unwrap_or_default(), false);
                if Some(i) == base {
                    ui.painter().rect_stroke(r.expand(1.5), t.cr_small(), egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Outside);
                }
                if resp.on_hover_text(rows[i]["to"].as_str().unwrap_or_default()).clicked() {
                    d.fields.insert("__sel".into(), json!([i]));
                }
            }
        });
    });
    changed
}

/// The Edit tab's color bars: a solid bar per row of `active` with its new color, in row order, the
/// base row's framed, scrolling past the wheel's height. A click on a bar selects its row (the
/// sliders under it edit that row's new color); a drag moves the bar's row to where the line under
/// the pointer shows, keeping every new color. Returns true when the rows moved.
fn color_bars(ui: &mut Ui, d: &mut Dialog, rows: &mut [Value], active: &[usize], base: Option<usize>) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut rects: Vec<Rect> = Vec::with_capacity(active.len());
    let (mut dragging, mut released, mut dropped) = (None, None, None);
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
                resp.on_hover_text(rows[i]["to"].as_str().unwrap_or_default());
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
    let Some(moved) = dropped.and_then(|(n, to)| move_bar(rows, active, n, to)) else { return false };
    d.fields.insert("__sel".into(), json!([moved]));
    // The slider values held are those of a row index that may now hold another row.
    d.fields.remove(COMPS);
    true
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

    /// Randomize after a seed past `u64::MAX` keeps the largest seed.
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
        assert_eq!(new_shown(&app, &d, &rs[0]), cmyk.tinted(0.5));
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
}
