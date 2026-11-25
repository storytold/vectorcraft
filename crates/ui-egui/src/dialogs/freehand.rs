//! Tool Options of the freehand tools (double-click Pencil, Paintbrush, Smooth, Blob Brush or
//! Eraser): the options each tool keeps, which last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool, values}`. The Pencil also has `liveFit`
//! (Live curve fitting, on: its preview fits the stroke while it is drawn).
//!
//! Fields: `tool`; `fidelity` (screen pixels for the Pencil, Paintbrush and Smooth, points for the
//! Blob Brush), `fill` (Pencil and
//! Paintbrush), `keepSelected` (Pencil, Paintbrush and Blob Brush), `mergeOnlySelection` (Blob
//! Brush), `closeWithin` and `editWithin` (screen pixels, 0 turns it off; Pencil and Paintbrush),
//! `angle` (degrees) and `roundness` (percent; Blob Brush and Eraser), `livePreview` (Pencil: the stroke shows
//! with its fill and stroke while it is drawn, or as a thin line until the release), `size`
//! (points; Blob Brush and Eraser), and `sizeMode` and `sizeVariation` (points; Blob Brush and
//! Eraser: with Pressure the tip is Size − Variation wide at the lightest pen pressure and Size +
//! Variation at the heaviest). The Blob Brush's `sizeMode` is `fixed` or `pressure`. The Eraser's
//! Angle, Roundness and Size each have a menu (`angleMode`, `roundnessMode`, `sizeMode`: `fixed`,
//! `random`, `pressure`, `tilt`, `bearing` or `rotation`) before the field and a Variation field
//! under it (`angleVariation` in degrees, `roundnessVariation` in percent, `sizeVariation`),
//! greyed while the menu is Fixed.

// PLAN: the imports below assume `craft_ui` paths as the framework's own code uses them; `ui::brush_settings`
// of the draw crate is where `TipEllipse` and `TipAngle` live.
use craft::draw::ui::brush_settings::{TipAngle, TipEllipse};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Picture, PictureSize};
use craft::ui::layout_components::Row;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::primitives::{ContentPainterCx, PaintShape};
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use craft::ui::value_drag::{Slider, SliderScale};
use egui::{Response, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_brush::Variation;

use super::host::open_held;
use super::variation::variation_param;
use super::{form, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of the freehand tools' Tool Options.
pub const KIND: &str = "freehandOptions";

/// The tools the dialog serves.
pub const TOOLS: [&str; 5] = ["pencil", "paintbrush", "smooth", "blobBrush", "eraser"];

/// The options the dialog edits, in the order it shows them (a tool has those it reports).
const KEYS: [&str; 18] = [
    "fidelity",
    "angle",
    "angleMode",
    "angleVariation",
    "roundness",
    "roundnessMode",
    "roundnessVariation",
    "size",
    "sizeMode",
    "sizeVariation",
    "fill",
    "altSmooth",
    "keepSelected",
    "mergeOnlySelection",
    "closeWithin",
    "editWithin",
    "livePreview",
    "liveFit",
];

/// The top of the Fidelity slider, in the tool's unit (screen pixels for the Pencil, Paintbrush
/// and Smooth, points for the Blob Brush); it starts at the lowest Fidelity the tool takes,
/// and the field takes up to 100.
const FIDELITY_MAX: f64 = 20.0;

/// Fidelity `v` stepped `n` steps by the slider's and the field's keys: an arrow steps hundredths up to 0.1,
/// tenths up to 1 and whole units above, in the tool's unit (a step down from 0.1 or 1 takes the finer step below
/// it); ten steps at once (Shift with an arrow, Page Up/Down) multiply or divide by ten, so they go
/// 0.01 → 0.1 → 1 → 10.
fn fidelity_step(v: f64, n: f64) -> f64 {
    if n.abs() >= 10.0 {
        let times = 10f64.powf((n / 10.0).round());
        return ((v * times) * 100.0).round() / 100.0;
    }
    // The step at `v` going up, or going down from it.
    let unit = |v: f64, up: bool| {
        let at = |edge: f64| if up { v < edge - 1e-9 } else { v <= edge + 1e-9 };
        if at(0.1) {
            0.01
        } else if at(1.0) {
            0.1
        } else {
            1.0
        }
    };
    let up = n > 0.0;
    let mut v = v;
    for _ in 0..(n.abs().min(1000.0) as usize) {
        let u = unit(v, up);
        let q = v / u;
        let k = if up { (q + 1e-6).floor() + 1.0 } else { (q - 1e-6).ceil() - 1.0 };
        v = ((k * u) * 100.0).round() / 100.0;
    }
    v
}

/// The unit of tool `tool`'s Fidelity. The Pencil's, the Paintbrush's and the Smooth tool's
/// Fidelity is on screen, the same at any zoom (#5729); the Blob Brush's is in points.
fn fidelity_unit(tool: &str) -> UnitKind {
    if matches!(tool, "pencil" | "paintbrush" | "smooth") { UnitKind::Pixels } else { UnitKind::Points }
}

/// Fidelity `v` as the slider and the field hold it: hundredths below 1, tenths from 1.
fn fidelity_round(v: f64) -> f64 {
    let step = if v < 1.0 { 100.0 } else { 10.0 };
    (v * step).round() / step
}

/// The Fidelity field of tool `tool`, showing `v` to two decimals from the tool's lowest Fidelity to
/// 100 and stepping by [`fidelity_step`], named by the label drawn just before it. Returns the value
/// typed or stepped to. The Contextual Task Bar shows the Smooth tool's.
pub(crate) fn fidelity_field(ui: &mut Ui, id: &str, v: f64, tool: &str) -> Option<f64> {
    let min = vectorcraft_tools::draw2::min_fidelity(tool);
    NumberField::new(tl!("Fidelity"), Bind::given(Some(v)))
        .id_salt(id.to_owned())
        .range(min..=100.0)
        .unit_kind(fidelity_unit(tool))
        .fixed_decimals(2)
        .stepper(|v, dir, scale| fidelity_step(v, f64::from(dir) * scale))
        .show(ui)
        .edit
        .value()
        .and_then(|typed| typed.to_owned())
}

/// The Fidelity row: a logarithmic slider from the tool's lowest Fidelity to [`FIDELITY_MAX`] with the field at its
/// end, and Accurate and Smooth under the slider's ends.
fn fidelity_row(ui: &mut Ui, fields: &mut DialogFields, tool: &str) -> Response {
    let min = vectorcraft_tools::draw2::min_fidelity(tool);
    let v = fields.number("fidelity").unwrap_or(1.5);
    let mut set = None;
    let shown = Row::new().show(ui, |ui, _| {
        let slid = Slider::new(Bind::given(v), min..=FIDELITY_MAX)
            .scale(SliderScale::Log)
            .marks_labelled([(min, tl!("Accurate")), (FIDELITY_MAX, tl!("Smooth"))])
            .label(tl!("Fidelity"))
            .id_salt("fidelity-slider")
            .show(ui);
        if let Some(x) = slid.value() {
            set = Some(fidelity_round(*x).clamp(min, FIDELITY_MAX));
        }
        if let Some(x) = fidelity_field(ui, "fidelity-field", v, tool) {
            set = Some(x);
        }
    });
    if let Some(x) = set.filter(|x| *x != v) {
        fields.set("fidelity", FieldValue::Number(x));
    }
    shown.output
}

/// The tip row: the Angle and Roundness widget beside three dabs of the tip, the middle one darker. It edits the
/// `angle` and `roundness` the rows above it hold; `least` is the lowest roundness the tool's tip takes.
fn tip_row(ui: &mut Ui, fields: &mut DialogFields, least: f64) -> Response {
    let (angle, roundness) = (fields.number("angle").unwrap_or(0.0), fields.number("roundness").unwrap_or(100.0));
    let start = TipAngle::new(angle, roundness / 100.0);
    let mut tip = start;
    let shown = Row::new().show(ui, |ui, _| {
        let _ = TipEllipse::new(tl!("Tip"), Bind::bound(&mut tip)).least_roundness(least).show(ui);
        dabs(ui, tip);
    });
    if tip != start {
        fields.set("angle", FieldValue::Number(tip.degrees.round()));
        fields.set("roundness", FieldValue::Number((tip.roundness * 100.0).round()));
    }
    shown.output
}

/// Three dabs of `tip`, as a stroke of it lays them down: a picture seven tip halves wide and two high, at the
/// small sample size, each dab as long as the picture is high.
fn dabs(ui: &mut Ui, tip: TipAngle) {
    // Decorative: the Tip control beside it is the row's accessible control.
    Picture::painter("", ThumbAspect::new(7, 2), move |cx, rect| paint_dabs(cx, rect, tip)).extent(PictureSize::Sample(ThumbSize::Small)).show(ui);
}

/// Paints three dabs of `tip` across `rect`, the middle one in the text color and the outer two in the muted one.
fn paint_dabs(cx: &ContentPainterCx<'_>, rect: egui::Rect, tip: TipAngle) {
    let style = cx.style_cx();
    let (dim, ink) = (style.color(style.tokens.color.text.muted), style.color(style.tokens.color.text.normal));
    let long = rect.height() / 2.0;
    let a = tip.degrees.clamp(-180.0, 180.0).to_radians();
    let u = egui::vec2(a.cos() as f32, -a.sin() as f32);
    let v = egui::vec2(-u.y, u.x);
    let short = long * (tip.roundness.clamp(0.0, 1.0) as f32).max(0.001);
    cx.with_draw(|draw| {
        for (i, color) in [dim, ink, dim].into_iter().enumerate() {
            let center = egui::pos2(rect.min.x + long + i as f32 * 2.5 * long, rect.center().y);
            let points = (0..32)
                .map(|k| {
                    let s = std::f32::consts::TAU * k as f32 / 32.0;
                    center + u * (long * s.cos()) + v * (short * s.sin())
                })
                .collect();
            draw.fill(&PaintShape::Polygon(points), color);
        }
    });
}

/// The rows of tool `tool`'s dialog: those of `present` (the option keys the tool reports), in the order the
/// dialog shows them.
fn params(tool: &str, present: &[&str]) -> Vec<Param> {
    let has = |key: &str| present.contains(&key);
    let blob = tool == "blobBrush";
    let eraser = has("angleMode");
    let check = |key: &'static str, label: &'static str| Param::new(key, label, ParamKind::bool(false));
    let pixels = |key: &'static str, label: &'static str, default: f64| {
        Param::number(key, label, default, 1.0, 0).unit(UnitKind::Pixels).bounds(Some(0.0), Some(100.0))
    };
    let mut rows = vec![];
    if has("fidelity") {
        let tool = tool.to_owned();
        rows.push(Param::custom("fidelity", tl!("Fidelity:"), move |ui, fields, _| fidelity_row(ui, fields, &tool)));
        rows.push(
            Param::read_only_text(
                "fidelityNote",
                "",
                tl!("A lower value follows the stroke closely; a higher one gives fewer points and smoother curves."),
            )
            .full_width(),
        );
    }
    if eraser {
        rows.push(variation_param("angle", tl!("Angle:"), UnitKind::Degrees, 0, (-180.0, 180.0), (0.0, 180.0), &Variation::ALL));
        rows.push(variation_param("roundness", tl!("Roundness:"), UnitKind::Percent, 0, (1.0, 100.0), (0.0, 100.0), &Variation::ALL));
        rows.push(Param::custom("tip", "", |ui, fields, _| tip_row(ui, fields, 0.01)).full_width());
        rows.push(variation_param("size", tl!("Size:"), UnitKind::Points, 1, (0.1, 1000.0), (0.0, 1000.0), &Variation::ALL));
    } else {
        if has("angle") {
            rows.push(Param::number("angle", tl!("Angle:"), 0.0, 1.0, 0).unit(UnitKind::Degrees).bounds(Some(-180.0), Some(180.0)));
        }
        if has("roundness") {
            let min = if blob { 0.0 } else { 1.0 };
            rows.push(Param::number("roundness", tl!("Roundness:"), 100.0, 1.0, 0).unit(UnitKind::Percent).bounds(Some(min), Some(100.0)));
        }
        if blob {
            rows.push(Param::custom("tip", "", |ui, fields, _| tip_row(ui, fields, 0.0)).full_width());
        }
        if has("sizeMode") {
            let offered = &[Variation::Fixed, Variation::Pressure];
            rows.push(variation_param("size", tl!("Size:"), UnitKind::Points, 1, (0.1, 1000.0), (0.0, 1000.0), offered));
        } else if has("size") {
            rows.push(Param::number("size", tl!("Size:"), 10.0, 1.0, 1).unit(UnitKind::Points).bounds(Some(0.1), Some(1000.0)));
        }
    }
    for (key, label) in [
        ("fill", tl!("Fill new strokes")),
        ("altSmooth", tl!("Alt key toggles to Smooth Tool")),
        ("keepSelected", tl!("Keep selected")),
        ("mergeOnlySelection", tl!("Merge only with selection")),
    ] {
        if has(key) {
            rows.push(check(key, label));
        }
    }
    for (key, label, default) in
        [("closeWithin", tl!("Close paths when ends are within:"), 15.0), ("editWithin", tl!("Edit selected paths within:"), 12.0)]
    {
        if has(key) {
            rows.push(pixels(key, label, default));
        }
    }
    for (key, label) in [("livePreview", tl!("Live preview")), ("liveFit", tl!("Live curve fitting"))] {
        if has(key) {
            rows.push(check(key, label));
        }
    }
    rows
}

/// Open the options of freehand tool `tool` with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    if !TOOLS.contains(&tool) {
        return Err(format!("`{tool}` isn't a freehand tool"));
    }
    let o = app.session.tool_options_of(tool);
    let present: Vec<&'static str> = KEYS.iter().copied().filter(|key| o.get(*key).is_some()).collect();
    let mut values: Map<String, Value> = present.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    values.insert("tool".into(), json!(tool));
    let fields = form::fields_of(&params(tool, &present), &values);
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = {
        let (tool, present) = (tool.to_owned(), present.clone());
        HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
            ParamDialog::new(KIND, tool_options::title(&tool), params(&tool, &present)).values(Bind::bound(fields))
        })
    };
    let ok = {
        let (tool, present) = (tool.to_owned(), present);
        move |app: &mut VectorcraftApp, result: &craft::ui::patterns::form_dialogs::ParamResult| {
            let values: Map<String, Value> =
                form::json_of(&params(&tool, &present), &result.fields).into_iter().filter(|(key, _)| present.contains(&key.as_str())).collect();
            tool_options::set_options(app, &tool, values)
        }
    };
    open_held(app, dialog, ok).ok_or_else(|| "no window".to_string())?;
    Ok(json!({ "dialog": KIND }))
}

#[cfg(test)]
mod tests;
