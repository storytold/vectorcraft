//! Scatter, Art and Pattern Brush Options: the [`super::brush_options`] of the brushes made of
//! art. Each has the brush's own values, Colorization (Method, and Key Color for Hue Shift, picked
//! from the art's colors), a Tips button beside Method that opens Colorization Tips (what each
//! method does, beside a sample of the brush painted with it) and a live preview of a stroke.
//!
//! Fields (besides `__brush`, `__type` and `name`): `__def` (the brush definition, for the preview),
//! `__colors` (the art's colours, most used first), `colorization` (`none` | `tints` |
//! `tintsAndShades` | `hueShift`), `keyColor` (a color), `keyColorPick` (whether the Key Color
//! eyedropper samples), `tips` (whether Colorization Tips is open).
//! Scatter: per value (`size`, `spacing`, `scatter` %, `rotation` °) `…Min`, `…Max` and `…Mode`
//! (`fixed` | `random` | `pressure` | `tilt` | `bearing` | `rotation`), `rotationRelativeTo` (`page` |
//! `path`), `hueJitter` (°), `saturationJitter` (%), `brightnessJitter` (%).
//! Art: `width` (%), `widthMode` (`fixed` | `pressure` | `tilt` | `bearing` | `rotation`),
//! `widthMax` (the width at the heaviest pen pressure, upright or at 180°, %), `scaleMode` (`proportional` | `stretch` | `betweenGuides`), `guideStart` and
//! `guideEnd` (% of the art's length), `direction` (`leftToRight` …), `flipAlong`, `flipAcross`,
//! `overlap` (bend on each run between sharp corners).
//! Pattern: `scale`, `spacing` (%), `fit` (`stretch` | `addSpace` | `approximate`), `flipAlong`,
//! `flipAcross`, and the tiles `outerCornerTile`, `sideTile`, `innerCornerTile`, `startTile` and
//! `endTile` (`original`: the art the brush draws there | `none` | `pattern:<name>` | for a
//! corner `autoCentered` | `autoBetween` | `autoSliced` | `autoOverlap`: a tile made from the side
//! tile). A tile key is empty until the user picks that tile (from its menu or by setting the
//! key): an empty tile shows, and saves as, what the brush has there, and any pick is kept.
//! `__tiles` (slot → preview key, for the tiles the brush has).

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::behavior::IconId;
use craft::theme::Srgba;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{CellMarks, Segmented, Swatch};
use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::{FieldRefusal, StackDialog};
use craft::ui::display::{Picture, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::UnitKind;
use craft::ui::runtime::requests::EyedropperRequest;
use craft::ui::text::NumberField;
use craft::ui::value_drag::{Slider, SliderScale};
use egui::{Color32, ColorImage, Response, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_brush::{ArtBrush, ArtScale, Brush, BrushKind, Colorization, CornerSource, Direction, PatternBrush, PatternFit, Scatter, Variation};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, PatternDef};

use super::brush_options::{clamped, mode_key};
use crate::VectorcraftApp;

/// Scatter values: (field, suffix), in [`Scatter::RANGES`] order.
const SCATTER: [(&str, UnitKind); 4] =
    [("size", UnitKind::Percent), ("spacing", UnitKind::Percent), ("scatter", UnitKind::Percent), ("rotation", UnitKind::Degrees)];

/// Art brush directions, in the order of their arrows (→ ← ↓ ↑).
const DIRECTIONS: [Direction; 4] = [Direction::LeftToRight, Direction::RightToLeft, Direction::TopToBottom, Direction::BottomToTop];

/// Pattern brush tiles: (field of the definition, slot), in the dialog's order.
const TILES: [(&str, &str); 5] =
    [("outer_corner", "outerCorner"), ("side", "side"), ("inner_corner", "innerCorner"), ("start", "start"), ("end", "end")];

/// The key of the tile whose field of the definition is `field`.
fn tile_key(field: &str) -> String {
    match field {
        "outer_corner" => "outerCornerTile".into(),
        "inner_corner" => "innerCornerTile".into(),
        _ => format!("{field}Tile"),
    }
}

/// The text of field `key`, empty when it has none.
fn text(f: &DialogFields, key: &str) -> String {
    f.text(key).unwrap_or_default().to_string()
}

/// The JSON value of a hidden field (`__def`, `__patterns`, `__colors`).
fn json_field<'f>(f: &'f DialogFields, key: &str) -> Option<&'f Value> {
    match f.get(key)? {
        FieldValue::Json(value) => Some(value),
        _ => None,
    }
}

/// The names of the document's patterns a tile can take.
pub(super) fn pattern_names(f: &DialogFields) -> Vec<String> {
    json_field(f, "__patterns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.get("name").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn pattern_choice(f: &DialogFields, choice: &str) -> Option<String> {
    let name = choice.strip_prefix("pattern:").unwrap_or(choice);
    let reserved = choice == "none" || choice == "original" || CornerSource::parse(choice).is_some();
    (!reserved && pattern_names(f).iter().any(|candidate| candidate == name)).then(|| name.to_string())
}

/// Whether tile `field`'s menu offers Original: the tile the dialog opened with, kept available after
/// a swatch is chosen so the tile can go back to it.
pub(super) fn original_available(f: &DialogFields, field: &str) -> bool {
    !field.ends_with("corner") || corner_art_field(json_field(f, "__def"), field).is_some()
}

/// Pattern brush corner tiles: (dialog field, art field of the definition, source field, art field
/// of the other kind of corner).
const CORNER_TILES: [(&str, &str, &str, &str); 2] =
    [("outerCornerTile", "outer_corner", "outer_source", "inner_corner"), ("innerCornerTile", "inner_corner", "inner_source", "outer_corner")];

/// The art field that corner tile `field` draws with in definition `def`: its own art, or, with
/// neither art nor a source, the other kind of corner's art (as [`PatternBrush`] draws it).
fn corner_art_field<'a>(def: Option<&Value>, field: &'a str) -> Option<&'a str> {
    let set = |f: &str| def.and_then(|d| d.get(f)).is_some_and(|v| !v.is_null());
    let (_, art, source, other) = CORNER_TILES.into_iter().find(|(_, art, _, _)| *art == field)?;
    if set(art) {
        Some(art)
    } else if !set(source) && set(other) {
        Some(other)
    } else {
        None
    }
}

/// Whether the user picked tile `key`: the key holds the explicit choice, an empty one means the
/// brush's own tile.
fn tile_is_set(f: &DialogFields, key: &str) -> bool {
    !text(f, key).is_empty()
}

/// The choice tile `key` shows and saves: the user's pick, else what the brush opened with there
/// (a document pattern it was made from, its own art, a corner's generated source, or none).
pub(super) fn tile_choice(f: &DialogFields, key: &str) -> String {
    let picked = text(f, key);
    if !picked.is_empty() {
        return picked;
    }
    let Some((art, _)) = TILES.iter().find(|(art, _)| tile_key(art) == key) else { return "none".into() };
    let def = json_field(f, "__def");
    let swatch = def.and_then(|d| d.get(format!("{art}_swatch"))).and_then(Value::as_str);
    if let Some(name) = swatch.filter(|name| pattern_names(f).iter().any(|n| n == name)) {
        return format!("pattern:{name}");
    }
    let set = |field: &str| def.and_then(|d| d.get(field)).is_some_and(|v| !v.is_null());
    match CORNER_TILES.iter().find(|(_, corner, _, _)| corner == art) {
        Some((_, corner, _, _)) if corner_art_field(def, corner).is_some() => "original".into(),
        Some((_, _, source, _)) => {
            def.and_then(|d| d.get(source)).and_then(Value::as_str).filter(|s| CornerSource::parse(s).is_some()).unwrap_or("none").into()
        }
        None if set(art) => "original".into(),
        None => "none".into(),
    }
}

/// The most key colours offered.
const MAX_COLORS: usize = 16;

/// The color of the stroke the Colorization Tips samples are painted on.
pub(super) const TIP_STROKE: Color = Color::Rgb { r: 0.18, g: 0.44, b: 0.86 };

pub(super) fn heading(ty: &str) -> &'static str {
    match ty {
        "scatter" => tl!("Scatter Brush Options"),
        "art" => tl!("Art Brush Options"),
        _ => tl!("Pattern Brush Options"),
    }
}

fn min_key(key: &str) -> String {
    format!("{key}Min")
}

fn max_key(key: &str) -> String {
    format!("{key}Max")
}

/// A definition id of a serde enum (`leftToRight`, `addSpace` …).
fn id_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The value of the serde enum `T` that field `key` names, else `T`'s default.
fn parse_field<T: serde::de::DeserializeOwned + Default>(f: &DialogFields, key: &str) -> T {
    serde_json::from_value(json!(text(f, key))).unwrap_or_default()
}

// ---------- fields ----------

// PLAN: `reconcile_pattern_choices` is gone (tile keys open empty, `tile_choice` derives what it did); appearance-w15's `brush_options.rs` calls it at `open` and `open_new` and drops those calls.

/// The fields of brush `b` (a scatter, art or pattern brush) whose definition is `def`. The tile
/// keys are not among them: a tile the user has not picked stays empty.
pub(super) fn fields(b: &Brush, def: Value, f: &mut Map<String, Value>) {
    match &b.kind {
        BrushKind::Scatter(s) => {
            for ((key, _), ((lo, hi), mode)) in SCATTER.iter().zip(s.values().into_iter().zip(s.modes())) {
                f.insert(min_key(key), json!(lo));
                f.insert(max_key(key), json!(hi));
                f.insert(mode_key(key), json!(mode.id()));
            }
            f.insert("rotationRelativeTo".into(), json!(if s.rotation_relative_to_path { "path" } else { "page" }));
            f.insert("hueJitter".into(), json!(s.hue_jitter));
            f.insert("saturationJitter".into(), json!(s.saturation_jitter));
            f.insert("brightnessJitter".into(), json!(s.brightness_jitter));
        }
        BrushKind::Art(a) => {
            let (mode, start, end) = match a.scale {
                ArtScale::Proportional => ("proportional", 0.0, 1.0),
                ArtScale::Stretch => ("stretch", 0.0, 1.0),
                ArtScale::BetweenGuides { start, end } => ("betweenGuides", start, end),
            };
            f.insert("width".into(), json!(a.width));
            f.insert("widthMode".into(), json!(a.width_mode.unwrap_or_default().id()));
            f.insert("widthMax".into(), json!(a.width_range().1));
            f.insert("scaleMode".into(), json!(mode));
            f.insert("guideStart".into(), json!(start * 100.0));
            f.insert("guideEnd".into(), json!(end * 100.0));
            f.insert("direction".into(), json!(id_of(&a.direction)));
            f.insert("flipAlong".into(), json!(a.flip_along));
            f.insert("flipAcross".into(), json!(a.flip_across));
            f.insert("overlap".into(), json!(a.overlap));
        }
        BrushKind::Pattern(p) => {
            f.insert("scale".into(), json!(p.scale));
            f.insert("spacing".into(), json!(p.spacing));
            f.insert("fit".into(), json!(id_of(&p.fit)));
            f.insert("flipAlong".into(), json!(p.flip_along));
            f.insert("flipAcross".into(), json!(p.flip_across));
            // A preview key per tile the brush has: its art's hash, so a changed tile redraws.
            let tiles: Map<String, Value> = TILES
                .iter()
                .filter_map(|(field, slot)| {
                    let art = def.get(*field).filter(|v| !v.is_null())?;
                    Some(((*slot).to_string(), json!(format!("brush-tile:{:x}", art_hash(art)))))
                })
                .collect();
            f.insert("__tiles".into(), Value::Object(tiles));
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => {}
    }
    let colors = vectorcraft_brush::art_colors(b.art());
    let colorization = b.kind.colorization();
    let key = match colorization {
        Colorization::HueShift { key } => key,
        _ => colors.first().copied().unwrap_or(Color::BLACK),
    };
    f.insert("colorization".into(), json!(colorization.id()));
    f.insert("keyColor".into(), serde_json::to_value(key).unwrap_or_default());
    f.insert("__colors".into(), serde_json::to_value(colors.into_iter().take(MAX_COLORS).collect::<Vec<_>>()).unwrap_or_default());
    f.insert("__def".into(), def);
}

/// The hash of an art value, the key of its preview.
fn art_hash(art: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    art.to_string().hash(&mut h);
    h.finish()
}

/// The Key Color: a colour set from the art's swatches (JSON), or one the eyedropper sampled.
fn key_color(f: &DialogFields) -> Color {
    match f.get("keyColor") {
        Some(FieldValue::Json(v)) => serde_json::from_value(v.clone()).unwrap_or(Color::BLACK),
        Some(FieldValue::Color([r, g, b])) => Color::rgb(*r, *g, *b),
        _ => Color::BLACK,
    }
}

fn colorization(f: &DialogFields) -> Colorization {
    Colorization::parse(&text(f, "colorization"), key_color(f)).unwrap_or_default()
}

/// How scatter value `key` varies.
fn scatter_mode(f: &DialogFields, key: &str) -> Variation {
    Variation::parse(&text(f, &mode_key(key))).unwrap_or_default()
}

/// How an art brush's width varies: Fixed, or with the pen's pressure, tilt, bearing, rotation or
/// wheel.
fn art_width_mode(f: &DialogFields) -> Variation {
    Variation::parse(&text(f, "widthMode")).filter(|m| m.follows_pen()).unwrap_or(Variation::Fixed)
}

/// `brush.options` params from the fields.
pub(super) fn params(f: &DialogFields) -> Value {
    let flag = |key: &str| f.bool(key).unwrap_or(false);
    let mut p = match text(f, "__type").as_str() {
        "scatter" => {
            let mut p = json!({ "rotation_relative_to_path": text(f, "rotationRelativeTo") == "path" });
            for ((key, _), range) in SCATTER.iter().zip(Scatter::RANGES) {
                p[*key] = json!([clamped(f, &min_key(key), range), clamped(f, &max_key(key), range)]);
            }
            p["modes"] = json!(SCATTER.map(|(key, _)| scatter_mode(f, key).id()));
            p["hue_jitter"] = json!(clamped(f, "hueJitter", Scatter::HUE_JITTER_RANGE));
            p["saturation_jitter"] = json!(clamped(f, "saturationJitter", Scatter::SATURATION_JITTER_RANGE));
            p["brightness_jitter"] = json!(clamped(f, "brightnessJitter", Scatter::BRIGHTNESS_JITTER_RANGE));
            p
        }
        "art" => {
            let scale = match text(f, "scaleMode").as_str() {
                "proportional" => ArtScale::Proportional,
                "betweenGuides" => ArtScale::BetweenGuides {
                    start: clamped(f, "guideStart", (0.0, 100.0)) / 100.0,
                    end: clamped(f, "guideEnd", (0.0, 100.0)) / 100.0,
                },
                _ => ArtScale::Stretch,
            };
            let mode = art_width_mode(f);
            let pen = mode.follows_pen();
            json!({
                "width": clamped(f, "width", ArtBrush::WIDTH_RANGE),
                "width_mode": if pen { json!(mode.id()) } else { Value::Null },
                "width_max": if pen { json!(clamped(f, "widthMax", ArtBrush::WIDTH_RANGE)) } else { Value::Null },
                "scale": serde_json::to_value(scale).unwrap_or_default(),
                "direction": id_of(&parse_field::<Direction>(f, "direction")),
                "flip_along": flag("flipAlong"),
                "flip_across": flag("flipAcross"),
                "overlap": flag("overlap"),
            })
        }
        _ => {
            let mut p = json!({
                "scale": clamped(f, "scale", PatternBrush::SCALE_RANGE),
                "spacing": clamped(f, "spacing", PatternBrush::SPACING_RANGE),
                "fit": id_of(&parse_field::<PatternFit>(f, "fit")),
                "flip_along": flag("flipAlong"),
                "flip_across": flag("flipAcross"),
            });
            // None or an automatic tile replaces the corner art. Original keeps the art the corner
            // draws, the other kind of corner's art included, so changing the other kind leaves it.
            // A None the menu was not set to, on a corner without art or a source, leaves the
            // definition as it is: that corner still draws the other kind's art when it gets some.
            let def = json_field(f, "__def");
            for (key, art, source, other) in CORNER_TILES {
                let choice = tile_choice(f, key);
                let set = tile_is_set(f, key);
                if let Some(name) = pattern_choice(f, &choice) {
                    if set {
                        p[format!("{art}_tile")] = json!({"pattern": name});
                    }
                    continue;
                }
                let unset = |field: &str| def.and_then(|d| d.get(field)).is_none_or(Value::is_null);
                if choice == CornerSource::None.id() && !set && unset(source) && corner_art_field(def, art).is_none() {
                    continue;
                }
                if let Some(s) = CornerSource::parse(&choice) {
                    p[format!("{art}_tile")] = json!(s.id());
                } else if choice == "original"
                    && let Some(from) = corner_art_field(def, art).filter(|field| *field != art)
                    && CORNER_TILES.iter().find(|(_, a, _, _)| *a == other).is_some_and(|(k, ..)| tile_choice(f, k) != "original")
                {
                    p[art] = def.and_then(|d| d.get(from)).cloned().unwrap_or_default();
                    p[format!("{art}_frame")] = def.and_then(|d| d.get(format!("{from}_frame"))).cloned().unwrap_or(Value::Null);
                }
                if choice == "original" && set {
                    p[format!("{art}_tile")] = json!("original");
                }
            }
            for art in ["side", "start", "end"] {
                let key = tile_key(art);
                if tile_is_set(f, &key) {
                    let choice = text(f, &key);
                    p[format!("{art}_tile")] = pattern_choice(f, &choice).map_or_else(|| json!(choice), |name| json!({"pattern": name}));
                }
            }
            p
        }
    };
    p["colorization"] = serde_json::to_value(colorization(f)).unwrap_or_default();
    p
}

/// The brush definition as OK would make it: `__def` with the fields' params.
pub(super) fn edited_def(f: &DialogFields) -> Value {
    let mut def = json_field(f, "__def").cloned().unwrap_or_default();
    if let (Some(o), Value::Object(p)) = (def.as_object_mut(), params(f)) {
        o.extend(p);
    }
    if text(f, "__type") == "pattern" {
        for (art, _) in TILES {
            let key = tile_key(art);
            if !tile_is_set(f, &key) {
                continue;
            }
            let choice = text(f, &key);
            if choice == "none" || (art.ends_with("corner") && CornerSource::parse(&choice).is_some()) {
                def[art] = Value::Null;
                def[format!("{art}_frame")] = Value::Null;
                def[format!("{art}_swatch")] = Value::Null;
                if art.ends_with("corner") {
                    def[if art == "outer_corner" { "outer_source" } else { "inner_source" }] = json!(choice);
                }
            } else if choice == "original" {
                def[format!("{art}_swatch")] = Value::Null;
            } else if let Some(name) = pattern_choice(f, &choice)
                && let Some(pattern) = json_field(f, "__patterns").and_then(Value::as_array).and_then(|a| a.iter().find(|p| p["name"] == name))
                && let Ok(pattern) = serde_json::from_value::<PatternDef>(pattern.clone())
            {
                def[art] = serde_json::to_value(Node::new(NodeId(0), NodeKind::Group { children: pattern.art, clip: false })).unwrap_or_default();
                def[format!("{art}_frame")] = json!(pattern.tile);
                def[format!("{art}_swatch")] = json!(name);
                if art.ends_with("corner") {
                    def[if art == "outer_corner" { "outer_source" } else { "inner_source" }] = Value::Null;
                }
            }
        }
    }
    def
}

// ---------- rows ----------

/// The rows of Brush Options for brush type `ty` (`scatter`, `art` or `pattern`): the brush's own
/// values, then Colorization. `object` is Options of Selected Object, which has no tiles and no
/// scale or direction of the art.
pub(super) fn rows(ty: &str, object: bool) -> Vec<Param> {
    let mut rows = match ty {
        "scatter" => scatter_rows(),
        "art" => art_rows(object),
        _ => pattern_rows(object),
    };
    rows.extend(colorization_rows());
    rows
}

/// A whole-number field bound to `f[key]`, kept within `range`, named `label`.
fn number_cell(ui: &mut Ui, f: &mut DialogFields, key: &str, label: &'static str, range: (f64, f64), unit: UnitKind, enabled: bool) -> Response {
    let typed = NumberField::new(label, Bind::given(Some(clamped(f, key, range))))
        .range(range.0..=range.1)
        .step(1.0)
        .unit_kind(unit)
        .fixed_decimals(0)
        .commit_decimals(0)
        .enabled(enabled)
        .id_salt(key)
        .show(ui);
    if let Some(Some(value)) = typed.edit.events.landed() {
        f.set(key, *value);
    }
    typed.edit.response.clone()
}

/// A whole-number row kept within `range` while typed. A custom row: a value set by command outside the range is
/// kept and clamped on OK ([`params`]), not refused.
fn clamped_number(key: &'static str, label: &'static str, range: (f64, f64), unit: UnitKind) -> Param {
    Param::custom(key, label, move |ui, f, _| number_cell(ui, f, key, label, range, unit, true))
}

/// A menu over `ids` (id, label) bound to the text field `key`, showing `current`.
fn id_select(ui: &mut Ui, f: &mut DialogFields, key: &str, label: &'static str, entries: &[Entry<String>], current: String) {
    if let Some(picked) = Select::new(label, Bind::given(current), entries).id_salt(key).show(ui).events.committed() {
        f.set(key, picked.as_str());
    }
}

/// The entries of the variations `offered`.
fn variation_entries(offered: &[Variation]) -> Vec<Entry<String>> {
    offered.iter().map(|m| Entry::new(m.id().to_string(), m.label())).collect()
}

/// A scatter value's row: Min, Max (live while the value varies) and how it varies.
fn scatter_row(key: &'static str, label: &'static str, unit: UnitKind, range: (f64, f64)) -> Param {
    Param::custom(min_key(key), label, move |ui, f, _| {
        let mode = scatter_mode(f, key);
        let entries = variation_entries(Variation::offered(key == "size"));
        Row::new()
            .show(ui, |ui, _| {
                number_cell(ui, f, &min_key(key), label, range, unit, true);
                number_cell(ui, f, &max_key(key), label, range, unit, mode != Variation::Fixed);
                id_select(ui, f, &mode_key(key), label, &entries, mode.id().to_string());
            })
            .output
    })
    .companions([max_key(key), mode_key(key)])
}

fn scatter_rows() -> Vec<Param> {
    let labels = [tl!("Size"), tl!("Spacing"), tl!("Scatter"), tl!("Rotation")];
    let mut rows: Vec<Param> =
        SCATTER.into_iter().zip(Scatter::RANGES).zip(labels).map(|(((key, unit), range), label)| scatter_row(key, label, unit, range)).collect();
    let relative = vec![("page".to_string(), tl!("Page").into()), ("path".to_string(), tl!("Path").into())];
    rows.push(Param::new("rotationRelativeTo", tl!("Rotation relative to"), ParamKind::radio(relative, Some("page".into()))));
    // Each copy turns its colors' hue by its own amount up to this many degrees either way.
    rows.push(clamped_number("hueJitter", tl!("Hue Jitter"), Scatter::HUE_JITTER_RANGE, UnitKind::Degrees));
    // Each copy moves its colors' saturation by its own amount up to this many points either way.
    rows.push(clamped_number("saturationJitter", tl!("Saturation Jitter"), Scatter::SATURATION_JITTER_RANGE, UnitKind::Percent));
    // Each copy moves its colors' brightness by its own amount up to this many points either way.
    rows.push(clamped_number("brightnessJitter", tl!("Brightness Jitter"), Scatter::BRIGHTNESS_JITTER_RANGE, UnitKind::Percent));
    rows
}

/// A bool row.
fn check(key: &str, label: &'static str) -> Param {
    Param::new(key, label, ParamKind::bool(false))
}

fn art_rows(object: bool) -> Vec<Param> {
    // Width, then the second width (only when the width follows the pen) and how it varies.
    let range = ArtBrush::WIDTH_RANGE;
    let modes = [Variation::Fixed, Variation::Pressure, Variation::Tilt, Variation::Bearing, Variation::Rotation, Variation::StylusWheel];
    let width = Param::custom("width", tl!("Width"), move |ui, f, _| {
        let mode = art_width_mode(f);
        let entries = variation_entries(&modes);
        Row::new()
            .show(ui, |ui, _| {
                let slid =
                    Slider::new(Bind::given(clamped(f, "width", range)), range.0..=range.1).step(1.0).label(tl!("Width")).id_salt("width").show(ui);
                if let Some(value) = slid.events.landed() {
                    // The slider quantizes new edits; Cancel restores the exact opening width.
                    f.set("width", *value);
                }
                number_cell(ui, f, "widthMax", tl!("Width"), range, UnitKind::Percent, mode.follows_pen());
                id_select(ui, f, "widthMode", tl!("Width"), &entries, mode.id().to_string());
            })
            .output
    })
    .companions(["widthMax", "widthMode"]);
    let mut rows = vec![width];
    // A path sets its own width, flips, overlap and colorization, not how the art scales or runs.
    if !object {
        let scales = vec![
            ("proportional".to_string(), tl!("Scale Proportionately").into()),
            ("stretch".to_string(), tl!("Stretch to Fit Stroke Length").into()),
            ("betweenGuides".to_string(), tl!("Stretch Between Guides").into()),
        ];
        rows.push(Param::new("scaleMode", tl!("Brush Scale Options"), ParamKind::radio(scales, Some("stretch".into()))));
        for (key, label) in [("guideStart", tl!("Start")), ("guideEnd", tl!("End"))] {
            rows.push(clamped_number(key, label, (0.0, 100.0), UnitKind::Percent).enabled_when("scaleMode", "betweenGuides"));
        }
        rows.push(direction_row());
    }
    rows.extend([check("flipAlong", tl!("Flip Along")), check("flipAcross", tl!("Flip Across")), check("overlap", tl!("Overlap"))]);
    rows
}

/// The direction of the art along the stroke: an arrow segment per direction in [`DIRECTIONS`] order,
/// named by its tooltip.
fn direction_row() -> Param {
    let arrows = [
        ("dc-arrow-right", tl!("Left to Right")),
        ("dc-arrow-left", tl!("Right to Left")),
        ("dc-arrow-down", tl!("Top to Bottom")),
        ("dc-arrow-up", tl!("Bottom to Top")),
    ];
    Param::custom("direction", tl!("Direction"), move |ui, f, _| {
        let current = DIRECTIONS.iter().position(|d| *d == parse_field::<Direction>(f, "direction")).unwrap_or(0);
        let entries: Vec<Entry<usize>> = arrows.iter().enumerate().map(|(i, (icon, tip))| Entry::new(i, *tip).icon(IconId::new(icon))).collect();
        let shown = Segmented::new(tl!("Direction"), Bind::given(current), &entries).show(ui);
        if let Some(dir) = shown.events.committed().and_then(|i| DIRECTIONS.get(*i)) {
            f.set("direction", id_of(dir));
        }
        shown.response().clone()
    })
}

fn pattern_rows(object: bool) -> Vec<Param> {
    let (lo, hi) = PatternBrush::SCALE_RANGE;
    let mut rows = vec![Param::new("scale", tl!("Scale"), ParamKind::range(lo, hi, 100.0, SliderScale::Linear)).unit(UnitKind::Percent)];
    let (lo, hi) = PatternBrush::SPACING_RANGE;
    rows.push(Param::number("spacing", tl!("Spacing"), 0.0, 1.0, 0).bounds(Some(lo), Some(hi)).unit(UnitKind::Percent));
    // The tiles belong to the brush; a path sets its own scale, spacing, flips, fit and colorization.
    if !object {
        rows.push(tiles_row());
    }
    rows.extend([check("flipAlong", tl!("Flip Along")), check("flipAcross", tl!("Flip Across"))]);
    let fits = vec![
        ("stretch".to_string(), tl!("Stretch to fit").into()),
        ("addSpace".to_string(), tl!("Add space to fit").into()),
        ("approximate".to_string(), tl!("Approximate path").into()),
    ];
    rows.push(Param::new("fit", tl!("Fit"), ParamKind::radio(fits, Some("stretch".into()))));
    rows
}

/// Built-in corner choices. Original is disabled when the corner opened with no art to draw.
fn corner_choices() -> [(&'static str, &'static str); 6] {
    [
        (CornerSource::None.id(), tl!("None")),
        ("original", tl!("Original")),
        (CornerSource::AutoCentered.id(), tl!("Auto-Centered")),
        (CornerSource::AutoBetween.id(), tl!("Auto-Between")),
        (CornerSource::AutoSliced.id(), tl!("Auto-Sliced")),
        (CornerSource::AutoOverlap.id(), tl!("Auto-Overlap")),
    ]
}

pub(super) fn tile_choices(f: &DialogFields, corner: bool) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> = if corner {
        corner_choices().into_iter().map(|(id, label)| (id.into(), label.into())).collect()
    } else {
        vec![("none".into(), tl!("None").into()), ("original".into(), tl!("Original").into())]
    };
    choices.extend(pattern_names(f).into_iter().map(|name| (format!("pattern:{name}"), name)));
    choices
}

/// The pattern's five tiles, each with its art (or None when the brush has no such tile). A menu
/// below each preview selects retained art, no art, or a document pattern; corners also offer
/// generated choices. The row's own key is the outer corner's; the other four are companions.
fn tiles_row() -> Param {
    let names = [tl!("Outer Corner Tile"), tl!("Side Tile"), tl!("Inner Corner Tile"), tl!("Start Tile"), tl!("End Tile")];
    let keys = TILES.map(|(field, _)| tile_key(field));
    let (own, companions) = (keys[0].clone(), keys[1..].to_vec());
    Param::custom(own, tl!("Tiles"), move |ui, f, _| {
        let preview_def = edited_def(f);
        Row::new()
            .show(ui, |ui, _| {
                for (((field, _slot), name), key) in TILES.into_iter().zip(names).zip(&keys) {
                    let corner = CORNER_TILES.iter().any(|(_, art, _, _)| *art == field);
                    // The art the tile draws with: its own, or a corner's that falls back on the other's.
                    let drawn = if corner { corner_art_field(Some(&preview_def), field) } else { Some(field) };
                    let choice = tile_choice(f, key);
                    let original_enabled = original_available(f, field);
                    let choices = tile_choices(f, corner);
                    let entries: Vec<Entry<String>> = choices
                        .iter()
                        .map(|(id, label)| Entry::new(id.clone(), label.clone()).enabled(id != "original" || original_enabled))
                        .collect();
                    // A tile that shows no art says what it is instead.
                    let shown = (choice != "original" && pattern_choice(f, &choice).is_none()).then_some(choice.as_str());
                    let art = drawn
                        .and_then(|d| preview_def.get(d))
                        .filter(|_| shown.is_none())
                        .and_then(|v| serde_json::from_value::<Node>(v.clone()).ok());
                    let label = shown.and_then(|c| choices.iter().find(|(id, _)| id == c)).map_or(tl!("None"), |(_, l)| l.as_str()).to_string();
                    let key_hash = drawn.and_then(|d| preview_def.get(d)).map_or(0, art_hash);
                    Stack::new().show(ui, |ui, _| {
                        match &art {
                            Some(art) => {
                                let image = || blank_if_none([152, 88], |w, h| tile_doc(art, w, h));
                                Picture::new(name, key_hash, &image).aspect(ThumbAspect::new(76, 44)).show(ui);
                            }
                            None => {
                                Text::new(label.clone()).show(ui);
                            }
                        }
                        Text::new(name).show(ui);
                        id_select(ui, f, key, name, &entries, choice.clone());
                    });
                }
            })
            .output
    })
    .opens_with("")
    .companions(companions)
    .full_width()
}

/// A tile preview's pixels: the document `build` makes at `size`, white where it makes none.
fn blank_if_none(size: [usize; 2], build: impl FnOnce(f64, f64) -> Option<Document>) -> ColorImage {
    crate::graphics::doc_image(size, build).unwrap_or_else(|| ColorImage::filled(size, Color32::WHITE))
}

// ---------- colorization ----------

fn colorization_rows() -> Vec<Param> {
    let methods = [tl!("None"), tl!("Tints"), tl!("Tints and Shades"), tl!("Hue Shift")];
    let method = Param::custom("colorization", tl!("Method"), move |ui, f, _| {
        let entries: Vec<Entry<String>> = Colorization::IDS.iter().zip(methods).map(|(id, label)| Entry::new((*id).to_string(), label)).collect();
        let current = text(f, "colorization");
        let (def, ty, key) = (edited_def(f), text(f, "__type"), key_color(f));
        let mut open = f.bool("tips").unwrap_or(false);
        let was_open = open;
        let shown = Row::new()
            .show(ui, |ui, _| {
                id_select(ui, f, "colorization", tl!("Method"), &entries, current.clone());
                // Tips opens and closes Colorization Tips; Escape and a click outside it close it too.
                let button = Button::new(tl!("Tips")).id_salt("tips").show(ui);
                if clicked(&button) {
                    open = !open;
                }
                Popover::new(tl!("Colorization Tips"), Bind::bound(&mut open))
                    .opened_by(button.response())
                    .show(ui, |ui, _| tips(ui, &def, &ty, key));
            })
            .output;
        if open != was_open {
            f.set("tips", open);
        }
        shown
    })
    .companions(["tips"])
    // A method set from outside is held as the select holds it, so Key Color's `enabled_when` reads it.
    .on_set(|f: &mut DialogFields, key: &str, value: Value| {
        let read = match key {
            "colorization" => value.as_str().filter(|id| Colorization::IDS.contains(id)).map(|id| FieldValue::Text(id.to_string())),
            _ => value.as_bool().map(FieldValue::Bool),
        };
        f.set(key, read.ok_or_else(|| FieldRefusal::Rejects(&value).message(key))?);
        Ok(())
    });
    // Hue Shift's key colour: the art colour that becomes the stroke colour.
    let key_color_row = Param::custom("keyColors", tl!("Key Color"), |ui, f, cx| {
        let colors: Vec<Color> = json_field(f, "__colors").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let key = key_color(f);
        let mut pick = f.bool("keyColorPick").unwrap_or(false);
        // The dialog claims Escape while the row samples, and says so once it ends the pick.
        if cx.sampling_ended() {
            pick = false;
        }
        let shown = Row::new()
            .show(ui, |ui, _| {
                for &c in &colors {
                    let [r, g, b] = c.to_rgb();
                    let well = Swatch::new(c.to_hex(), Srgba::new(r, g, b, 1.0)).marks(CellMarks::default().edge(c == key)).show(ui);
                    if well.clicked {
                        f.set("keyColor", FieldValue::Json(serde_json::to_value(c).unwrap_or_default()));
                    }
                }
                let dropper = IconButton::new(IconSpec::svg(IconId::new("pipette")))
                    .label(tl!("Eyedropper: click the art to sample the key color"))
                    .id_salt("pipette")
                    .show(ui);
                if dropper.clicks > 0 {
                    pick = !pick;
                }
            })
            .output;
        if pick {
            cx.arm_eyedropper(EyedropperRequest::new("keyColor"));
        }
        cx.sampling(pick);
        if pick != f.bool("keyColorPick").unwrap_or(false) {
            f.set("keyColorPick", pick);
        }
        shown
    })
    .companions(["keyColor", "keyColorPick"])
    .enabled_when("colorization", "hueShift");
    vec![Param::section(tl!("Colorization")), method, key_color_row]
}

/// Colorization Tips: for each method, a sample of the brush as OK would make it, painted with
/// that method on a [`TIP_STROKE`] stroke (Hue Shift with the dialog's Key Color), beside the
/// method's name and what it does.
fn tips(ui: &mut Ui, def: &Value, ty: &str, key: Color) {
    Text::new(tl!("The samples show this brush on a blue stroke.")).show(ui);
    let names = [tl!("None"), tl!("Tints"), tl!("Tints and Shades"), tl!("Hue Shift")];
    let about = [
        tl!("The art keeps its own colors."),
        tl!("Black becomes the stroke color and white stays white. Other colors become tints of the stroke color by their lightness."),
        tl!("Black and white stay as they are. Dark colors become shades of the stroke color and light colors become tints of it."),
        tl!("The key color becomes the stroke color. The other colors shift their hue by the same amount and keep their saturation and brightness."),
    ];
    for ((id, name), about) in Colorization::IDS.into_iter().zip(names).zip(about) {
        let Some(method) = Colorization::parse(id, key) else { continue };
        Row::new().show(ui, |ui, _| {
            let image = || blank_if_none([240, 88], |w, h| tip_doc(def, ty, method, w, h));
            Picture::new(name, art_hash(&json!([def, method])), &image).aspect(ThumbAspect::new(120, 44)).show(ui);
            Stack::new().show(ui, |ui, _| {
                Text::new(name).show(ui);
                Text::new(about).show(ui);
            });
        });
    }
}

/// A document of `w` × `h` showing brush definition `def` (of type `ty`) with colorization
/// `method`, drawn by [`vectorcraft_brush::stroke_pieces`] along a gentle S-curve stroked
/// [`TIP_STROKE`] 1 pt wide, and scaled to fit, centered.
pub(super) fn tip_doc(def: &Value, ty: &str, method: Colorization, w: f64, h: f64) -> Option<Document> {
    let mut def = match def {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    def.insert("name".into(), json!("tip"));
    def.insert("type".into(), json!(ty));
    def.insert("colorization".into(), serde_json::to_value(method).ok()?);
    let brush: Brush = serde_json::from_value(Value::Object(def)).ok()?;
    let ap = Appearance::basic(Paint::None, Paint::solid(TIP_STROKE), 1.0);
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((0.0, 20.0));
    bp.curve_to((35.0, 6.0), (65.0, 34.0), (100.0, 20.0));
    let pieces = vectorcraft_brush::stroke_pieces(&Document::new(w, h), &brush, &bp, ap.stroke()?);
    tile_doc(&Node::group(NodeId(0), pieces.into_iter().map(Arc::new).collect()), w, h)
}

/// A document of `w` × `h` showing `art` scaled to fit, centred.
fn tile_doc(art: &Node, w: f64, h: f64) -> Option<Document> {
    let b = art.visual_bounds().or_else(|| art.geometric_bounds())?;
    let k = ((w - 8.0) / b.width().max(1e-6)).min((h - 8.0) / b.height().max(1e-6));
    if !k.is_finite() || k <= 0.0 {
        return None;
    }
    let mut doc = Document::new(w, h);
    let mut n = doc.reid(art);
    let xf = vectorcraft_geom::Affine::translate((w / 2.0, h / 2.0))
        * vectorcraft_geom::Affine::scale(k)
        * vectorcraft_geom::Affine::translate(-b.center().to_vec2());
    n.transform(xf, true);
    let l = doc.layers.first()?.id;
    doc.insert(Some(l), 0, n).ok()?;
    Some(doc)
}

// ---------- the Key Color pick ----------

/// Whether the active document is the one the pick began on.
fn key_color_owner_valid(app: &VectorcraftApp) -> bool {
    let Some(pick) = app.ui.brush_key_color_pick.as_ref() else { return false };
    app.session.active().is_some_and(|doc| doc.uid == pick.doc)
}

/// Whether the Eyedropper tool still samples for the pick.
pub(crate) fn key_color_pick_active(app: &VectorcraftApp) -> bool {
    key_color_owner_valid(app) && app.session.tool_id() == "eyedropper" && app.session.tool_options()["brushKeyColor"] == true
}

/// Whether the window lost the focus, or the touch of a press was cancelled, in this frame's input.
/// The pump runs once a frame, so it reads the frame's events once.
fn input_lost(app: &VectorcraftApp) -> bool {
    app.ui_ctx.as_ref().is_some_and(|ctx| {
        ctx.input(|input| {
            !input.focused || input.events.iter().any(|event| matches!(event, egui::Event::Touch { phase: egui::TouchPhase::Cancel, .. }))
        })
    })
}

/// Runs the Key Color pick of the open Brush Options `dialog`, once a frame while it is open. The
/// row asks for the pick by arming its dropper: that arms the Eyedropper tool, sampling for this
/// dialog. The pick ends, returning to the tool that armed it, when the dialog ends its sampling
/// (Escape), the row is toggled off, the window loses focus, a touch is cancelled or another
/// document becomes active. A pick on a frame where none was asked for changes nothing.
pub(super) fn key_color_pick(app: &mut VectorcraftApp, dialog: &mut HostedParamDialog) {
    let id = StackDialog::id(dialog);
    let ended = dialog.take_sampling_ended();
    let asked = dialog.take_eyedropper().is_some();
    let wanted = dialog.fields().bool("keyColorPick").unwrap_or(false);
    let mine = app.ui.brush_key_color_pick.as_ref().is_some_and(|pick| pick.dialog == id);
    if mine {
        if ended || !wanted || !key_color_pick_active(app) || input_lost(app) {
            end_key_color_pick(app, true);
            dialog.clear_field("keyColorPick");
        }
        return;
    }
    if !asked || app.ui.brush_key_color_pick.is_some() {
        return;
    }
    let Some(doc) = app.session.active().map(|doc| doc.uid) else {
        dialog.clear_field("keyColorPick");
        return;
    };
    let back = app.session.tool_id().to_string();
    if let Err(error) = app.try_select_tool("eyedropper") {
        app.status(error);
        dialog.clear_field("keyColorPick");
        return;
    }
    app.session.set_tool_option("brushKeyColor", &json!(true));
    app.session.set_tool_option("stop", &json!(back));
    app.ui.brush_key_color_pick = Some(crate::state::BrushKeyColorPick { doc, back, dialog: id });
}

/// Ends the pick, if there is one: clears the transient mode it set on the Eyedropper tool and, when
/// `restore` holds and the tool is still the Eyedropper, returns to the tool that armed it.
pub(crate) fn end_key_color_pick(app: &mut VectorcraftApp, restore: bool) {
    let Some(pick) = app.ui.brush_key_color_pick.take() else { return };
    let options = Map::from_iter([("brushKeyColor".into(), json!(false)), ("stop".into(), Value::Null)]);
    app.session.set_tool_options(Some("eyedropper"), &options);
    if restore && app.session.tool_id() == "eyedropper" {
        app.return_to_tool(&pick.back);
    }
}

/// Applies a sampled colour to the Brush Options dialog that armed the pick (`brushKeyColor` tool
/// request). Nothing changes without a live pick of the active document, or unless the dialog
/// shows Hue Shift.
pub(super) fn apply_key_color_sample(app: &mut VectorcraftApp, color: &Value) {
    let Some(id) = app.ui.brush_key_color_pick.as_ref().map(|pick| pick.dialog) else { return };
    let valid = key_color_owner_valid(app);
    end_key_color_pick(app, false);
    let Some(c) = vectorcraft_engine::cmd::color_value(color).filter(|_| valid) else { return };
    let [r, g, b] = c.to_rgb();
    let Some(dialogs) = app.dialogs() else { return };
    dialogs.with(|host| {
        let Some(dialog) = host.get_mut_by_id::<HostedParamDialog>(id) else { return };
        if dialog.fields().text("colorization") == Some("hueShift") {
            dialog.answer_eyedropper("keyColor", Srgba::new(r, g, b, 1.0));
        }
        dialog.clear_field("keyColorPick");
    });
}
