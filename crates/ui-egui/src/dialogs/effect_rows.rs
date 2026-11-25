//! The parameter rows of the effects whose dialogs are more than the generic list: Transform, Zig Zag, Roughen,
//! Tweak, Pucker & Bloat, Scribble, Drop Shadow, Film Grain, Glass, Texturizer, Rough Pastels, the Pathfinder
//! effects and Radial Blur. Each builder runs every pass; a row that depends on one other value says so with
//! `visible_when`/`enabled_when`, one that depends on several branches here.

use std::ops::RangeInclusive;

use craft::behavior::Mixed;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::runtime::requests::PathKind;
use craft::ui::text::{NumberField, StepArrows};
use craft::ui::value_drag::{PointPicker, SliderScale};
use egui::{Rect, Stroke, pos2};
use serde_json::{Map, Value};

use super::effect_params::{EffectCx, FILM_GRAIN_FIELDS, GLASS, SHADES, choices, generic, glass_textures, texturizer_textures};
use super::form;
use super::transform_each::{self, ScaleLink};

/// A translated label; the literal stays visible to the string catalog.
macro_rules! lbl {
    ($s:literal) => {
        Label::from(tl!($s).to_string())
    };
}

/// A label of a catalog entry whose text is data (a table of choices).
fn data_label(text: &str) -> Label {
    Label::from(tl!(text).to_string())
}

/// A whole-number slider row over `0..=100` percent.
fn percent_range(key: &str, label: Label, default: f64) -> Param {
    count_range(key, label, default).unit(UnitKind::Percent)
}

/// A whole-number slider row over `0..=100` with no unit after the number.
fn count_range(key: &str, label: Label, default: f64) -> Param {
    Param::new(key, label, ParamKind::range(0.0, 100.0, default, SliderScale::Linear)).decimals(0)
}

/// The row of `key` for either unit of a Relative/Absolute pair: a share of the object's size, or a distance in the
/// document unit.
fn sized_row(key: &str, label: Label, default: f64, relative: bool, lengths: LengthUnits) -> Param {
    if relative {
        percent_range(key, label, default)
    } else {
        Param::number(key, label, default, 1.0, 0).bounds(Some(0.0), Some(100.0)).length(lengths)
    }
}

/// The Relative/Absolute choice over the flag `relative` that decides the unit of the rows above it: its two ids are
/// the flag's JSON values, which `form::json_of` writes back as a bool.
pub(super) fn relative_radio(label: impl Into<Label>, default: bool) -> Param {
    let options = vec![("true".to_owned(), lbl!("Relative")), ("false".to_owned(), lbl!("Absolute"))];
    Param::new("relative", label, ParamKind::radio(options, Some(default.to_string())))
}

/// Whether the pair's rows are shares of the object's size.
pub(super) fn is_relative(fields: &DialogFields) -> bool {
    fields.text("relative").is_none_or(|id| id == "true")
}

/// Transform: Transform Each's rows, the options, Copies, the reflections and the reference point.
pub(super) fn transform(cx: &EffectCx, _fields: &DialogFields) -> Vec<Param> {
    let lengths = cx.lengths;
    vec![
        Param::custom("transform", lbl!("Transform"), move |ui, fields, _| transform_each::rows(ui, fields, lengths, ScaleLink::None))
            .companions(["scaleH", "scaleV", "moveH", "moveV", "rotate"]),
        Param::section(lbl!("Options")),
        Param::new("objects", lbl!("Transform Objects"), ParamKind::bool(true)),
        Param::new("patterns", lbl!("Transform Patterns"), ParamKind::bool(false)),
        Param::new("strokes", lbl!("Scale Strokes & Effects"), ParamKind::bool(false)),
        Param::new("copies", lbl!("Copies:"), ParamKind::int(Some(0), Some(1000), 0)),
        Param::custom("__reflect", lbl!("Reflect"), |ui, fields, _| transform_each::reflect_checks(ui, fields))
            .companions(["reflectX", "reflectY", "random"]),
        Param::custom("__reference", lbl!("Reference Point"), |ui, fields, _| transform_each::reference_point(ui, fields)).companions(["reference"]),
    ]
}

/// Zig Zag and Roughen.
pub(super) fn distort(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let roughen = cx.id == "distort.roughen";
    // PLAN: production suffixes Detail with "/in"; `Param::unit` takes a `UnitKind` and none stands for it
    let second = if roughen { count_range("detail", lbl!("Detail:"), 10.0) } else { count_range("ridges", lbl!("Ridges per segment:"), 4.0) };
    let points: Vec<(String, Label)> = choices(&cx.id, "points")
        .unwrap_or_default()
        .iter()
        .map(|(label, value)| ((*value).to_owned(), Label::from(crate::i18n::tr_ctx(crate::i18n::current(), "effect", label).to_string())))
        .collect();
    vec![
        Param::section(lbl!("Options")),
        sized_row("size", lbl!("Size:"), if roughen { 5.0 } else { 10.0 }, is_relative(fields), cx.lengths),
        second,
        relative_radio(Label::Empty, true),
        Param::new(
            "points",
            Label::from(crate::i18n::tr_ctx(crate::i18n::current(), "effect", "Points").to_string()),
            ParamKind::radio(points, None),
        ),
    ]
}

/// Tweak.
pub(super) fn tweak(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let relative = is_relative(fields);
    vec![
        Param::section(lbl!("Amount")),
        sized_row("h", lbl!("Horizontal:"), 10.0, relative, cx.lengths),
        sized_row("v", lbl!("Vertical:"), 10.0, relative, cx.lengths),
        relative_radio(Label::Empty, true),
        Param::section(lbl!("Modify")),
        Param::new("anchors", lbl!("Anchor Points"), ParamKind::bool(true)),
        Param::new("in", lbl!("\"In\" Control Points"), ParamKind::bool(false)),
        Param::new("out", lbl!("\"Out\" Control Points"), ParamKind::bool(false)),
    ]
}

/// Pucker & Bloat: one slider from Pucker to Bloat.
pub(super) fn pucker_bloat(_cx: &EffectCx, _fields: &DialogFields) -> Vec<Param> {
    vec![
        Param::new("amount", lbl!("Amount:"), ParamKind::range(-200.0, 200.0, 0.0, SliderScale::Linear))
            .unit(UnitKind::Percent)
            .marks_labelled([(-200.0, lbl!("Pucker")), (200.0, lbl!("Bloat"))]),
    ]
}

/// The id of the Settings preset the values match, `custom` when none does.
fn scribble_setting(fields: &DialogFields) -> &'static str {
    let values: Map<String, Value> = fields.iter().filter_map(|(k, v)| Some((k.to_owned(), serde_json::to_value(v).ok()?))).collect();
    vectorcraft_effects::scribble_setting_id(&Value::Object(values)).unwrap_or("custom")
}

/// Scribble: a Settings menu whose pick fills the other rows, then the effect's own rows.
pub(super) fn scribble(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let settings = Param::custom("__scribbleSetting", lbl!("Settings"), |ui, fields, _| {
        let presets = vectorcraft_effects::scribble_settings();
        let entries: Vec<Entry<String>> = presets
            .iter()
            .map(|p| Entry::new(p.id.to_owned(), data_label(p.label)))
            .chain([Entry::new("custom".to_owned(), lbl!("Custom"))])
            .collect();
        let picked = Select::new(lbl!("Settings"), Bind::given(scribble_setting(fields).to_owned()), &entries).show(ui);
        if let Some(id) = picked.value().cloned()
            && let Some(preset) = presets.iter().find(|p| p.id == id).and_then(|p| p.params.as_object())
        {
            for (key, value) in preset {
                if let Ok(value) = serde_json::from_value::<FieldValue>(value.clone()) {
                    fields.set(key.as_str(), value);
                }
            }
        }
        picked.response
    });
    std::iter::once(settings).chain(generic(cx, fields)).collect()
}

/// Drop Shadow: the generic rows with step arrows, then the Color/Darkness choice and its two fields.
pub(super) fn drop_shadow(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let shades = SHADES.iter().map(|(label, value)| ((*value).to_owned(), data_label(label))).collect();
    let rows = generic(cx, fields)
        .into_iter()
        .filter(|p| !matches!(p.key.as_str(), "shade" | "color" | "darkness"))
        .map(|p| if matches!(p.key.as_str(), "opacity" | "x" | "y" | "blur") { p.step_arrows(StepArrows::Leading) } else { p });
    rows.chain([
        Param::new("shade", Label::Empty, ParamKind::radio(shades, None)),
        Param::new("color", lbl!("Color"), ParamKind::color(None)).enabled_when("shade", "color"),
        Param::number("darkness", lbl!("Darkness"), 100.0, 1.0, 0)
            .bounds(Some(0.0), Some(100.0))
            .unit(UnitKind::Percent)
            .step_arrows(StepArrows::Leading)
            .enabled_when("shade", "darkness"),
    ])
    .collect()
}

/// Film Grain: one whole-number slider for each of its controls.
pub(super) fn film_grain(_cx: &EffectCx, _fields: &DialogFields) -> Vec<Param> {
    FILM_GRAIN_FIELDS
        .iter()
        .map(|(key, label, max)| Param::new(*key, data_label(label), ParamKind::range(0.0, *max, 0.0, SliderScale::Linear)).decimals(0))
        .collect()
}

/// Glass, Texturizer and Rough Pastels: the Texture menu with the loaded texture's size, a path to load one from,
/// then the other rows. The list does not depend on the values, so one list serves the dialog and the gallery.
pub(super) fn textured(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    use vectorcraft_effects::pixel::{TEXTURE_IMAGE, texture};
    let menu: fn(bool) -> form::Choices = if cx.id == GLASS { glass_textures } else { texturizer_textures };
    let exts: Vec<String> = vectorcraft_engine::cmd::fileio::FORMATS
        .iter()
        .filter(|f| f.read && f.raster)
        .flat_map(|f| vectorcraft_engine::cmd::fileio::read_extensions(f).iter().map(|e| e.to_string()))
        .collect();
    let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
    let picker = Param::custom("texture", lbl!("Texture:"), move |ui, fields, cx| {
        let size = fields.text(TEXTURE_IMAGE).and_then(texture).map(|t| t.size());
        let table = menu(size.is_some());
        let entries: Vec<Entry<String>> = table.iter().map(|(label, value)| Entry::new((*value).to_owned(), data_label(label))).collect();
        let current = fields.text("texture").unwrap_or_default();
        let current = table.iter().find(|(_, value)| value.eq_ignore_ascii_case(current)).map_or("", |(_, value)| *value).to_owned();
        let mut picked = None;
        let shown = Row::new().show(ui, |ui, _| {
            picked = Some(Select::new(lbl!("Texture"), Bind::given(current.clone()), &entries).show(ui));
            if let Some((w, h)) = size {
                craft::ui::display::Text::new(format!("{w} × {h} px")).show(ui);
            }
        });
        let Some(picked) = picked else { return shown.output };
        if let Some(id) = picked.value().cloned() {
            fields.set("texture", id);
        }
        // The pick is an edit of the rows, which the Appearance panel's editor applies.
        cx.gesture(&picked.events);
        picked.response
    })
    .companions([TEXTURE_IMAGE]);
    let own = ["texture", "texturePath", TEXTURE_IMAGE];
    [picker, Param::new("texturePath", lbl!("Load Texture…"), ParamKind::text("")).browse(PathKind::open_file(&exts))]
        .into_iter()
        .chain(generic(cx, fields).into_iter().filter(|p| !own.contains(&p.key.as_str())))
        .collect()
}

/// The Pathfinder effects: Soft Mix's Mixing Rate where it has one, then the Pathfinder Options.
pub(super) fn pathfinder(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let own: Vec<Param> = if super::effect::has_only_pathfinder_options(&cx.id) {
        Vec::new()
    } else {
        generic(cx, fields).into_iter().filter(|p| !vectorcraft_effects::PATHFINDER_OPTION_KEYS.contains(&p.key.as_str())).collect()
    };
    own.into_iter().chain(super::pathfinder_options::params()).collect()
}

/// A Blur Center coordinate kept to a hundredth of a percent of the box.
fn center_round(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

/// Holds Radial Blur's centre inside the object's bounds, once, where the dialog opens, so OK sends the centre the
/// box shows.
pub(super) fn clamp_blur_center(params: &mut Map<String, Value>) {
    for key in ["centerX", "centerY"] {
        if let Some(v) = params.get(key).and_then(Value::as_f64) {
            params.insert(key.into(), Value::from(v.clamp(0.0, 1.0)));
        }
    }
}

/// Paints Radial Blur's pattern about the centre: rays for Zoom, rings for Spin.
fn blur_pattern(cx: &ContentPainterCx, rect: Rect, center: [f64; 2], zoom: bool) {
    let at = pos2(rect.left() + center[0] as f32 * rect.width(), rect.top() + center[1] as f32 * rect.height());
    let stroke = Stroke::new(1.0, cx.ink().gamma_multiply(0.4));
    let reach = rect.width() * std::f32::consts::SQRT_2;
    if zoom {
        for k in 0..24 {
            let a = k as f32 * std::f32::consts::TAU / 24.0;
            let dir = egui::vec2(a.cos(), a.sin());
            cx.painter.line_segment([at + dir * 6.0, at + dir * reach], stroke);
        }
    } else {
        for k in 1..=12 {
            cx.painter.circle_stroke(at, k as f32 * reach / 12.0, stroke);
        }
    }
}

/// Radial Blur: Amount, Blur Method and Quality, then the Blur Center box with its Horizontal and Vertical fields.
pub(super) fn radial_blur(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let center = Param::custom("__blurCenter", lbl!("Blur Center"), |ui, fields, _| {
        let c = [fields.number("centerX").unwrap_or(0.5).clamp(0.0, 1.0), fields.number("centerY").unwrap_or(0.5).clamp(0.0, 1.0)];
        let zoom = fields.text("method").is_some_and(|m| m.eq_ignore_ascii_case("zoom"));
        let mut next = c;
        let unit: RangeInclusive<f64> = 0.0..=1.0;
        let row = Row::new().show(ui, |ui, _| {
            let picker = PointPicker::new(Bind::given([Mixed::Same(c[0]), Mixed::Same(c[1])]), unit.clone(), unit.clone())
                .label(lbl!("Blur Center"))
                .id_salt("point")
                .backdrop(move |cx: &ContentPainterCx, rect: Rect| blur_pattern(cx, rect, c, zoom))
                .show(ui);
            if let Some([Mixed::Same(x), Mixed::Same(y)]) = picker.value().cloned() {
                next = [x, y];
            }
            Stack::new().show(ui, |ui, _| {
                for (axis, label) in [lbl!("Horizontal"), lbl!("Vertical")].into_iter().enumerate() {
                    let shown = NumberField::new(label, Bind::given(Some(c[axis] * 100.0)))
                        .unit("%")
                        .range(0.0..=100.0)
                        .commit_decimals(1)
                        .id_salt(axis)
                        .show(ui);
                    if let Some(v) = shown.edit.value().cloned().flatten() {
                        next[axis] = v / 100.0;
                    }
                }
            });
        });
        if next != c {
            fields.set("centerX", center_round(next[0].clamp(0.0, 1.0)));
            fields.set("centerY", center_round(next[1].clamp(0.0, 1.0)));
        }
        row.output
    })
    .companions(["centerX", "centerY"]);
    let rows = generic(cx, fields).into_iter().filter(|p| !matches!(p.key.as_str(), "centerX" | "centerY"));
    rows.chain([center]).collect()
}
