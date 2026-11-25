//! Stroke panel: the weight field, the framework stroke editor (cap, corner, align, dashes), arrowheads with
//! rendered previews, arrow scale and align, width profiles.
// PLAN: import paths of craft_ui / craft_draw items are assumed from the crates' own module layout (craft_draw is
// not yet a dependency of ui-egui; its owner adds it to Cargo.toml).

use std::sync::LazyLock;

use craft::behavior::{IconId, Mixed};
use craft::draw::ui::stroke::{self as draw, StrokeEditor, StrokeField, StrokeSections};
use craft::keymap::CommandRef;
use craft::ui::buttons::IconButton;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, FieldLabelRole, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing, TooltipSpec};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::primitives::{ContentPainterCx, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::style::DataColor;
use craft::ui::text::{LinkMode, LinkedFields};
use craft::ui::text::{NumberField, NumberOutput, NumberPick, StepArrows};
use egui::{Rect, Ui, pos2};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ArrowAlign, Arrowhead, Dash, Document, Node, StrokeLayer, Unit, WidthProfile};
use vectorcraft_engine::inspect::StrokeMixed;

use super::{Live, character, current_stroke, live_run, stroke_mixed};
use crate::VectorcraftApp;
// PLAN: assumed `crate::edits::{apply_edits, EditPath}` (wg.A16).
use crate::edits::{EditPath, apply_edits};

/// The key of [`StrokePanel::brushed`]: (document uid, revision, active Appearance row).
type BrushedKey = (u64, u64, Option<usize>);

/// What the Stroke panel remembers between frames; `UiState::stroke_panel` holds it.
#[derive(Clone, Debug, Default)]
pub(crate) struct StrokePanel {
    /// Hide Options: only the weight shows.
    pub hide_options: bool,
    /// The pattern and corner alignment of the dashes last turned off, for turning them back on.
    pub dash_last: Option<(Vec<f64>, bool)>,
    /// The two arrowhead scales move together.
    pub scale_linked: bool,
    /// The cache of [`brush_keeps_size`], keyed by (document uid, revision, active Appearance row).
    pub brushed: Option<(BrushedKey, Option<bool>)>,
    /// The Stroke popover of the weight's link label is open.
    pub link_open: bool,
}

/// The stroke a weight edit goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WeightTarget {
    /// The stroke the panels show, or the next object's with nothing selected.
    Shown,
    /// The Appearance panel's stroke row at this item index.
    Item(usize),
}

/// Stroke weight dropdown presets in points, from the ladder of `unit` (Units > Stroke): each
/// unit has its own ladder of round values in that unit, so the dropdown reads `0.25 mm`, not the
/// `0.088 mm` a converted pt ladder gives. Feet, yards and meters, which no stroke is measured
/// in, keep the pt ladder.
pub fn weight_presets(unit: Unit) -> [f64; 22] {
    let native = match unit {
        Unit::Millimeters => MM_PRESETS,
        Unit::Centimeters => CM_PRESETS,
        Unit::Inches => IN_PRESETS,
        Unit::Pixels => PX_PRESETS,
        Unit::Picas => return PC_PRESETS,
        Unit::Points | Unit::FeetInches | Unit::Feet | Unit::Meters | Unit::Yards | Unit::Q | Unit::Ha => return PT_PRESETS,
    };
    native.map(|v| unit.to_pt(v))
}

const PT_PRESETS: [f64; 22] =
    [0.25, 0.5, 0.75, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0];
const MM_PRESETS: [f64; 22] = [0.1, 0.25, 0.35, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 15.0, 20.0, 25.0, 30.0];
const CM_PRESETS: [f64; 22] = [0.01, 0.02, 0.03, 0.05, 0.06, 0.07, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0];
const IN_PRESETS: [f64; 22] =
    [0.0078, 0.0156, 0.0313, 0.0625, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0];
const PX_PRESETS: [f64; 22] =
    [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 30.0, 40.0];
/// The pica ladder (0p1 … 5p) in points, so every entry is an exact pt weight. The dropdown shows
/// them as decimal picas (`0.083 p`) until [`Unit::number`] writes `0p1` notation.
const PC_PRESETS: [f64; 22] =
    [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 15.0, 18.0, 21.0, 24.0, 27.0, 30.0, 33.0, 36.0, 48.0, 60.0];

/// A profile's points as its silhouette: none (the plain bar) for the uniform stroke.
fn silhouette(points: &[(f64, f64, f64)]) -> Option<&[(f64, f64, f64)]> {
    (points != WidthProfile::PRESETS[0].points).then_some(points)
}

/// The Arrowheads Align buttons: (alignment, `stroke.set` value, icon, tooltip).
const ARROW_ALIGN: [(ArrowAlign, &str, &str, &str); 2] = [
    (ArrowAlign::Extend, "extend", "dc-arrow-extend", "Tip extends past the end point"),
    (ArrowAlign::Tip, "tip", "dc-arrow-tip", "Tip on the end point"),
];

/// Which Align button is on, and whether they are usable (only a stroke with a head can be aligned).
fn arrow_align_state(st: Option<&StrokeLayer>) -> (ArrowAlign, bool) {
    st.map_or((ArrowAlign::default(), false), |s| (s.arrow_align, s.start_arrow.is_some() || s.end_arrow.is_some()))
}

/// The full pattern and alignment the Dashed Line section shows: the stroke's dashes, or with
/// dashes off the last ones turned off (`last`), at first 12 pt dashes fitted to corners.
fn dash_state(dash: Option<&Dash>, last: Option<(Vec<f64>, bool)>) -> (Vec<f64>, bool) {
    match dash {
        Some(d) => (d.pattern.clone(), d.align_corners),
        None => last.unwrap_or((vec![12.0, 12.0], true)),
    }
}

fn arrow_label(a: Option<&Arrowhead>) -> &'static str {
    a.map_or("None", Arrowhead::label)
}

/// The name an arrowhead menu shows for `a`: a built-in head's in the interface language, a
/// custom head's own.
fn arrow_name(a: Option<&Arrowhead>) -> String {
    match a {
        Some(Arrowhead::Custom(c)) => c.name.clone(),
        _ => builtin_arrow_name(arrow_label(a)).to_string(),
    }
}

/// A built-in arrowhead's name `label` in the interface language. Its `arrowhead` row comes first,
/// so a name that is also an effect's (Feather) can be translated for each one.
pub(crate) fn builtin_arrow_name(label: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "arrowhead", label)
}

/// The preview cache key of `a`: its name, and for a custom head its art.
fn arrow_key(a: Option<&Arrowhead>) -> String {
    match a {
        Some(Arrowhead::Custom(c)) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for s in &c.path.subpaths {
                (s.anchors.len(), s.closed).hash(&mut h);
                for p in s.anchors.iter().flat_map(|a| [a.p, a.h_in, a.h_out]) {
                    (p.x.to_bits(), p.y.to_bits()).hash(&mut h);
                }
            }
            format!("custom:{}:{:x}", c.name, h.finish())
        }
        _ => format!("{a:?}"),
    }
}

/// Apply Stroke panel options (`stroke.set` params): while the Type tool edits text, to the
/// selected characters' stroke.
fn set(app: &mut VectorcraftApp, p: Value) {
    if character::text_editing(app).is_some() {
        character::range_style(app, json!({ "strokeOptions": p }));
    } else {
        app.run("stroke.set", p).ok();
    }
}

/// The weight the Stroke panel, Control bar and Properties panel show: blank where the selected
/// objects differ, the default for new art without a stroke.
pub(crate) fn shown_weight(app: &VectorcraftApp, st: Option<&StrokeLayer>, mixed: &StrokeMixed) -> Option<f64> {
    (!mixed.weight).then(|| st.map_or(app.session.paint.stroke_width, |s| s.width))
}

/// A document enum as the editor's enum of the same name: both serialise as the same camelCase word.
fn convert<A: Serialize, B: DeserializeOwned + Default>(a: A) -> B {
    serde_json::to_value(a).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// The style the stroke editor shows for `st` (the defaults without a stroke). Dashes count document units, so the
/// pattern is the stroke's own numbers.
pub(crate) fn style_of(st: Option<&StrokeLayer>) -> draw::StrokeStyle {
    let base = draw::StrokeStyle { dash_unit: draw::DashUnit::Document, ..Default::default() };
    let Some(s) = st else { return base };
    let (dashes, offset, corners) =
        s.dash.as_ref().map_or((vec![], 0.0, false), |d| (d.pattern.iter().map(|v| *v as f32).collect(), d.offset, d.align_corners));
    draw::StrokeStyle {
        width: s.width as f32,
        align: convert(s.align),
        cap: convert(s.cap),
        join: convert(s.join),
        miter_limit: s.miter_limit as f32,
        dashes,
        dash_offset: offset as f32,
        dash_align: if corners { draw::DashAlign::Corners } else { draw::DashAlign::Exact },
        ..base
    }
}

/// The editor's mixed flags for the engine's. Offset and alignment are kept per stroke by the command, so only the
/// pattern is ever shown as mixed.
fn mixed_of(m: &StrokeMixed) -> draw::StrokeMixed {
    let mut mixed = draw::StrokeMixed::default();
    (mixed.align, mixed.cap, mixed.join, mixed.miter_limit, mixed.dashes) = (m.align, m.cap, m.join, m.miter_limit, m.dash);
    mixed
}

/// A style number as the document's number, without the `f32` noise (`0.1` stays `0.1`).
fn num(v: f32) -> f64 {
    (f64::from(v) * 1e4).round() / 1e4
}

/// The `stroke.set` params of an edit of `field` that turned `old` into `new`: only what the field changes, so
/// each selected stroke keeps its other options (its dash offset and alignment included).
pub(crate) fn params_of(old: &draw::StrokeStyle, new: &draw::StrokeStyle, field: StrokeField) -> Value {
    let word = |v: &dyn erased::Word| v.word();
    let dash = if new.dashes.is_empty() { Value::Null } else { json!(new.dashes.iter().map(|v| num(*v)).collect::<Vec<_>>()) };
    let corners = new.dash_align == draw::DashAlign::Corners;
    match field {
        StrokeField::Align => json!({"align": word(&new.align)}),
        StrokeField::Cap => json!({"cap": word(&new.cap)}),
        StrokeField::Join => json!({"join": word(&new.join)}),
        StrokeField::MiterLimit => json!({"miterLimit": num(new.miter_limit)}),
        StrokeField::Dashed => json!({"dash": dash, "alignDashes": corners}),
        StrokeField::Dash(_) => json!({"dash": dash}),
        StrokeField::DashOffset => json!({"dashOffset": num(new.dash_offset)}),
        StrokeField::DashAlign => json!({"alignDashes": corners}),
        StrokeField::Preset if old.dash_align != new.dash_align => json!({"dash": dash, "alignDashes": corners}),
        StrokeField::Preset => json!({"dash": dash}),
        // The panel shows no opacity row.
        _ => json!({}),
    }
}

/// The word `stroke.set` reads for an editor enum.
mod erased {
    use serde::Serialize;
    use serde_json::Value;

    /// An enum that serialises as its camelCase word.
    pub trait Word {
        /// The word.
        fn word(&self) -> Value;
    }

    impl<T: Serialize> Word for T {
        fn word(&self) -> Value {
            serde_json::to_value(self).unwrap_or(Value::Null)
        }
    }
}

/// Sends an editor edit of `field` (`old` to `new`) to the strokes. Turning dashes off remembers the pattern; turning
/// them on brings back the remembered one, or 12 pt dashes fitted to corners.
fn apply_style(app: &mut VectorcraftApp, old: &draw::StrokeStyle, mut new: draw::StrokeStyle, field: StrokeField) {
    if matches!(field, StrokeField::Dashed) {
        if old.dashes.is_empty() {
            let (pattern, corners) = dash_state(None, app.ui.stroke_panel.dash_last.clone());
            new.dashes = pattern.iter().map(|v| *v as f32).collect();
            new.dash_align = if corners { draw::DashAlign::Corners } else { draw::DashAlign::Exact };
        } else {
            let pattern = old.dashes.iter().map(|v| f64::from(*v)).collect();
            app.ui.stroke_panel.dash_last = Some((pattern, old.dash_align == draw::DashAlign::Corners));
            new.dashes.clear();
        }
    }
    let params = params_of(old, &new, field);
    if params.as_object().is_some_and(|o| !o.is_empty()) {
        set(app, params);
    }
}

/// The unit a weight shows in. Feet, yards and meters, which no stroke is measured in, show points like the ladder.
fn shown_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Inches => UnitKind::Inches,
        Unit::Pixels => UnitKind::Pixels,
        Unit::Picas => UnitKind::Picas,
        _ => UnitKind::Points,
    }
}

/// A weight ladder: its stops in points and the rows of the presets list.
struct Ladder {
    stops: Vec<f64>,
    rows: Vec<Entry<NumberPick>>,
}

/// Every unit a stroke can show in, with its ladder.
static LADDERS: LazyLock<Vec<(Unit, Ladder)>> = LazyLock::new(|| {
    use Unit::*;
    [Millimeters, Centimeters, Inches, Pixels, Picas, Points, FeetInches, Feet, Meters, Yards, Q, Ha]
        .into_iter()
        .map(|unit| {
            let stops = weight_presets(unit).to_vec();
            let rows = stops.iter().map(|pt| Entry::new(NumberPick::Number(*pt), unit.format(*pt))).collect();
            (unit, Ladder { stops, rows })
        })
        .collect()
});

/// The stops a blank weight steps through, in points: it steps from the first.
static BLANK_WEIGHT_STOPS: [f64; 2] = [0.25, 0.5];

/// The weight field (Stroke panel, Control bar, Properties panel): Units > Stroke, the presets list, step arrows and
/// scrubbing by its label, blank when mixed. A plain step stops at each preset (0.52 pt goes down to 0.5 pt, not to
/// 0 pt); Shift or Page steps the grid by ten.
// PLAN: the sheet names `NumberField<'static, f64>`; `NumberField::length` takes `Option<f64>`.
pub(crate) fn weight_number(app: &VectorcraftApp, weight: Option<f64>) -> NumberField<'static> {
    let unit = app.session.stroke_unit();
    let lengths =
        crate::fields::bare_points(LengthUnits::new(UnitKind::Points, shown_kind(unit), 72.0), app.session.prefs.numbers_without_units_are_points);
    let ladder = LADDERS.iter().find(|(u, _)| *u == unit).or_else(|| LADDERS.iter().find(|(u, _)| *u == Unit::Points)).map(|(_, l)| l);
    let field =
        NumberField::length(tl!("Weight"), Bind::given(weight), lengths).range(0.0..=1000.0).step_arrows(StepArrows::Leading).scrub_by_label();
    // The stepper steps the number the field shows; the ladders are in points.
    let in_points = move |stops: &'static [f64], blank: bool| {
        let step = crate::fields::stops_stepper(1.0, stops, move |pt| unit.format(pt));
        move |from: f64, direction: i32, scale: f64| unit.from_pt(step(if blank { stops[0] } else { unit.to_pt(from) }, direction, scale))
    };
    match ladder {
        // A blank weight (the selected objects differ) steps in points from 0.25 pt, whatever the unit's ladder: up
        // gives 0.5 pt, down 0 pt (#5349).
        Some(l) if weight.is_none() => field.presets(&l.rows).stepper(in_points(&BLANK_WEIGHT_STOPS, true)),
        Some(l) => field.presets(&l.rows).stepper(in_points(&l.stops, false)),
        None => field,
    }
}

/// Sends a weight edit of `out` to `target`: a scrub or slider drag is one undo step, a typed, stepped or picked
/// value commits at once, and while the Type tool edits text the weight goes to the selected characters.
pub(crate) fn apply_weight(app: &mut VectorcraftApp, out: &NumberOutput, target: WeightTarget) {
    apply_edits(app, &out.edit, EditPath::Preview, |app, weight: &Option<f64>, phase| {
        let Some(w) = *weight else { return };
        match target {
            WeightTarget::Shown if character::text_editing(app).is_some() => {
                if matches!(phase, Live::Released) {
                    set(app, json!({"weight": w}));
                }
            }
            WeightTarget::Shown => live_run(app, tl!("Stroke Weight"), "stroke.set", json!({"weight": w}), phase),
            WeightTarget::Item(index) => {
                let params = super::appearance::aligned(app, json!({"index": index, "weight": w}));
                live_run(app, tl!("Stroke Weight"), "appearance.setItem", params, phase);
            }
        }
    });
}

/// The weight field in a row labelled `label` with the side label `side`: the output of the number field, none when
/// the row's body did not run.
fn weight_row(app: &VectorcraftApp, ui: &mut Ui, label: &'static str, side: SideLabel, weight: Option<f64>) -> Option<NumberOutput> {
    Field::new(label).side(side).id_salt("weight").show(ui, |ui, _| weight_number(app, weight).show(ui)).inner
}

/// The weight field whose link side label (`label`: the Control bar's "Stroke:", the Properties panel's "Stroke")
/// opens the Stroke panel in a popover under it, and whose drag scrubs the weight. The field keeps its name, Weight:
/// the label beside the Stroke chip says which paint it is the weight of.
pub(crate) fn link_field(app: &mut VectorcraftApp, ui: &mut Ui, label: &'static str, weight: Option<f64>) {
    let side = SideLabel::new().scrub(true).link().role(FieldLabelRole::Describes).tooltip(TooltipSpec::new(tl!("Stroke options")));
    let mut open = app.ui.stroke_panel.link_open;
    if let Some(out) = weight_row(app, ui, label, side, weight) {
        apply_weight(app, &out, WeightTarget::Shown);
        open ^= out.link;
    }
    Popover::new(tl!("Stroke"), Bind::bound(&mut open)).show(ui, |ui, _| show(app, ui));
    app.ui.stroke_panel.link_open = open;
}

/// The Stroke panel: the weight, the stroke editor (cap, corner, align, dashes), arrowheads, scale, align and the
/// width profile.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let st = current_stroke(app);
    let mixed = stroke_mixed(app, ui.ctx());
    let weight = shown_weight(app, st.as_ref(), &mixed);
    if let Some(out) = weight_row(app, ui, tl!("Weight"), SideLabel::new().scrub(true), weight) {
        apply_weight(app, &out, WeightTarget::Shown);
    }
    if app.ui.stroke_panel.hide_options {
        return;
    }
    let style = style_of(st.as_ref());
    let output = StrokeEditor::new(tl!("Stroke"), Bind::given(style.clone()))
        .sections(StrokeSections::PANEL | StrokeSections::DASH_ALIGN)
        .closed(mixed.can_align)
        .mixed(&mixed_of(&mixed))
        .show(ui);
    if let (Some(new), Some(field)) = (output.events.landed(), output.field) {
        apply_style(app, &style, new.clone(), field);
    }
    Divider::horizontal().show(ui);
    arrow_rows(app, ui, st.as_ref());
    Divider::horizontal().show(ui);
    Field::new(tl!("Profile")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            profile_select(app, ui);
            flip_buttons(app, ui, st.as_ref());
        });
    });
}

/// The arrowhead rows: the start and end heads with Swap, the two scales with their link, and the alignment.
fn arrow_rows(app: &mut VectorcraftApp, ui: &mut Ui, st: Option<&StrokeLayer>) {
    let custom: Vec<Arrowhead> = app.session.arrowheads.heads().iter().cloned().map(Arrowhead::Custom).collect();
    let entries: Vec<Entry<Option<Arrowhead>>> = std::iter::once(None)
        .chain(Arrowhead::ALL.iter().cloned().map(Some))
        .chain(custom.into_iter().map(Some))
        .map(|a| Entry::new(a.clone(), arrow_name(a.as_ref())))
        .collect();
    let heads = [
        (st.and_then(|s| s.start_arrow.clone()), true, tl!("Start arrowhead"), "startArrow"),
        (st.and_then(|s| s.end_arrow.clone()), false, tl!("End arrowhead"), "endArrow"),
    ];
    Field::new(tl!("Arrowheads")).show(ui, |ui, _| {
        // The two heads share the width the Swap button leaves; a field too narrow for all three puts Swap on a
        // new line.
        Row::new().fill_width().wrap(true).show(ui, |ui, _| {
            for (head, start, label, key) in &heads {
                let (start, key) = (*start, *key);
                let out = Select::new(*label, Bind::given(head.clone()), &entries)
                    .picture(move |a, cx, rect| arrow_thumb(cx, rect, a.as_ref(), start))
                    .sizing(Sizing::FillWidth)
                    .show(ui);
                // A custom head goes with its art, as document.inspect reports it.
                if let Some(a) = out.events.landed() {
                    set(app, json!({ key: a }));
                }
            }
            let swap = IconButton::new(IconId::new("arrow-left-right")).label(tl!("Swap start and end arrowheads")).enabled(st.is_some()).show(ui);
            if swap.clicked() {
                app.run("stroke.setAdvanced", json!({"swapArrows": true})).ok();
            }
        });
    });
    let (arrow_align, has_head) = arrow_align_state(st);
    let scale = st.map_or((100.0, 100.0), |s| s.arrow_scale);
    let linked = app.ui.stroke_panel.scale_linked;
    let out = LinkedFields::new(tl!("Scale"), Bind::given(Mixed::each([Some(scale.0), Some(scale.1)])), Bind::given(linked))
        .mode(LinkMode::Equal)
        .unit_kind(UnitKind::Percent)
        .enabled(has_head)
        .show(ui);
    if let Some(on) = out.link.landed() {
        app.ui.stroke_panel.scale_linked = *on;
    }
    if let Some([a, b]) = out.edit.events.landed().map(|p| (*p).map(|m| m.same().copied().flatten())) {
        app.run("stroke.setAdvanced", json!({"arrowScale": [a.unwrap_or(scale.0), b.unwrap_or(scale.1)]})).ok();
    }
    let aligns: Vec<Entry<ArrowAlign>> = ARROW_ALIGN.iter().map(|(v, _, icon, tip)| Entry::new(*v, *tip).icon(IconId::new(icon))).collect();
    let out = Segmented::new(tl!("Align"), Bind::given(arrow_align), &aligns).enabled(has_head).show(ui);
    if let Some(v) = out.events.landed()
        && let Some((_, name, _, _)) = ARROW_ALIGN.iter().find(|(a, ..)| a == v)
    {
        set(app, json!({"arrowAlign": name}));
    }
}

/// The Flip Along and Flip Across buttons: on only for a stroke with a profile that a brush does not keep the size of.
fn flip_buttons(app: &mut VectorcraftApp, ui: &mut Ui, st: Option<&StrokeLayer>) {
    let can_flip = st.is_some_and(|s| s.profile.is_some()) && brush_keeps_size(app, st).is_none();
    // Lucide names these icons for their mirror line: the vertical-line icon flips the
    // profile along the path, while the horizontal-line icon flips it across the path.
    for (icon, label, way) in [("flip-vertical-2", tl!("Flip Along"), "along"), ("flip-horizontal-2", tl!("Flip Across"), "across")] {
        if IconButton::new(IconId::new(icon)).label(label).enabled(can_flip).show(ui).clicked() {
            app.run("stroke.setAdvanced", json!({"flipProfile": way})).ok();
        }
    }
}

/// The document an arrowhead preview renders: a `w`×`h` pt line with head `a` at its left
/// (`start`) or right end, its tip on the end point, drawn white (the preview is tinted). A custom
/// head is made smaller than the built-in ones when it would not fit (below the smallest head
/// weight, its outline is scaled down), and the line's end moves in by as much as the head reaches
/// past its tip.
pub(super) fn arrow_doc(mut a: Option<Arrowhead>, start: bool, w: f64, h: f64) -> Option<Document> {
    const WIDTH: f64 = 1.5;
    // The smallest weight `StrokeLayer::arrow_weight` gives a head.
    const LEAST: f64 = 0.25;
    // Heads of weight 2.5 pt: 10 pt long and wide.
    let mut scale: f64 = 500.0 / 3.0;
    let mut inset = 0.0;
    if let Some(Arrowhead::Custom(c)) = &a
        && let Some(b) = c.path.control_bounds()
    {
        let (long, half) = (b.x1.max(0.0) - b.x0.min(0.0), b.y0.abs().max(b.y1.abs()));
        let fit = |room: f64, units: f64| if units > 1e-9 { room.max(0.0) / units } else { f64::INFINITY };
        let weight = fit(w - 6.0, long).min(fit(h / 2.0 - 1.0, half)).min(2.5);
        inset = -b.x0.min(0.0) * weight;
        if weight < LEAST {
            let path = c.path.transformed(vectorcraft_geom::Affine::scale(weight / LEAST));
            a = Some(Arrowhead::Custom(std::sync::Arc::new(vectorcraft_doc::CustomArrowhead::new(c.name.clone(), path))));
        }
        scale = scale.min(weight.max(LEAST) / WIDTH * 100.0);
    }
    let mut doc = Document::new(w, h);
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::WHITE), WIDTH);
    let st = ap.stroke_mut()?;
    st.arrow_scale = (scale, scale);
    st.arrow_align = ArrowAlign::Tip;
    if start {
        st.start_arrow = a
    } else {
        st.end_arrow = a
    }
    let (x0, x1) = if start { (3.0 + inset, w - 3.0) } else { (3.0, w - 3.0 - inset) };
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((x0, h / 2.0));
    bp.line_to((x1, h / 2.0));
    let (id, l) = (doc.alloc_id(), doc.layers[0].id);
    doc.insert(Some(l), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), ap)).ok()?;
    Some(doc)
}

/// The size of an arrowhead preview in points.
const ARROW_PREVIEW: (f64, f64) = (56.0, 22.0);

/// Renders [`arrow_doc`] for the thumbnails: the key is the head's [`arrow_key`] and the end, so the picture of a key
/// never changes.
struct ArrowSource<'a>(Option<&'a Arrowhead>);

impl ThumbSource<(String, bool)> for ArrowSource<'_> {
    fn revision(&self, key: &(String, bool), _: &ThumbRequest) -> Revision {
        Revision::of(key)
    }

    fn render(&self, key: &(String, bool), request: &ThumbRequest) -> ThumbRender {
        let (w, h) = ARROW_PREVIEW;
        let Some(doc) = arrow_doc(self.0.cloned(), key.1, w, h) else { return ThumbRender::Blank };
        let scale = f64::from(request.side.max(1)) / w;
        let (px, py) = ((w * scale).round().max(1.0) as u32, (h * scale).round().max(1.0) as u32);
        let img = vectorcraft_render::Renderer::new().render_region(&doc, vectorcraft_geom::Rect::new(0.0, 0.0, w, h), scale, false);
        let rgba: Vec<u8> = (0..py).flat_map(|y| (0..px).map(move |x| (x, y))).flat_map(|(x, y)| img.pixel(x, y)).collect();
        ThumbRender::Ready(egui::ColorImage::from_rgba_unmultiplied([px as usize, py as usize], &rgba))
    }
}

/// Draws an arrowhead preview (the head as the canvas draws it) in `rect`: a line with the head at the left (`start`)
/// or right end, in the ink of the surface.
pub(crate) fn arrow_thumb(cx: &ContentPainterCx, rect: Rect, arrow: Option<&Arrowhead>, start: bool) {
    let ctx = cx.painter.ctx().clone();
    let thumbs = Thumbnails::<(String, bool)>::in_context(&ctx, "stroke-arrows", ThumbnailOptions::default());
    let key = (arrow_key(arrow), start);
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.pixels_per_point());
    let state = thumbs.state(&ctx, &ArrowSource(arrow), &key, &request, craft::ui::primitives::Priority::VISIBLE);
    thumbs.paint(cx, rect, &state, &ThumbPaint::default().checker(false).tint(DataColor::new(cx.ink())));
}

/// The silhouette of a width profile (its points; None: the plain bar) in `r` as its top and bottom edges. A profile
/// made with the Width tool can be many times the stroke's weight: the silhouette is scaled to fit `r` by its widest
/// point (#909).
fn profile_outline(r: Rect, points: Option<&[(f64, f64, f64)]>) -> (Vec<egui::Pos2>, Vec<egui::Pos2>) {
    let n = 32;
    let half = r.height() / 2.0 - 1.0;
    let widths: Vec<(f64, f64)> = (0..=n).map(|i| points.map_or((0.35, 0.35), |p| WidthProfile::at_points(p, i as f64 / n as f64))).collect();
    let widest = widths.iter().map(|(l, rr)| l.max(*rr)).filter(|w| w.is_finite()).fold(1.0, f64::max);
    let side = |w: f64| if w.is_finite() { (w / widest).clamp(0.0, 1.0) as f32 * half } else { 0.0 };
    let x = |i: usize| r.left() + i as f32 / n as f32 * r.width();
    let top = widths.iter().enumerate().map(|(i, (l, _))| pos2(x(i), r.center().y - side(*l))).collect();
    let bottom = widths.iter().enumerate().map(|(i, (_, rr))| pos2(x(i), r.center().y + side(*rr))).collect();
    (top, bottom)
}

/// Draws a width profile's silhouette ([`profile_outline`]) into `rect` in the surface's ink, clipped to it.
pub fn paint_profile(cx: &ContentPainterCx, rect: Rect, points: Option<&[(f64, f64, f64)]>) {
    let (top, bottom) = profile_outline(rect, points);
    let ink = cx.ink();
    let mut mesh = egui::Mesh::default();
    for (t, b) in top.iter().zip(&bottom) {
        mesh.colored_vertex(*t, ink);
        mesh.colored_vertex(*b, ink);
    }
    for i in 0..top.len().saturating_sub(1) as u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    cx.painter.with_clip_rect(rect).add(egui::Shape::mesh(mesh));
}

/// While a width profile changes none of the strokes the Profile list edits, because each draws
/// with a brush: whether any of them has a profile. None while a profile changes one of them.
/// With nothing selected, and while the Live Paint Bucket is active, the list sets up the next
/// object drawn, whose stroke is `shown` (the stroke the panels show). Otherwise it edits the
/// strokes `stroke.set` targets
/// ([`vectorcraft_engine::Session::brushed_strokes_profiled`], cached per document revision and
/// active Appearance row in [`StrokePanel::brushed`]), such as a selected group's own stroke while `shown` is its
/// first member's.
pub(crate) fn brush_keeps_size(app: &mut VectorcraftApp, shown: Option<&StrokeLayer>) -> Option<bool> {
    let st = app.session.active()?;
    let bucket = app.session.bucket_paints();
    if bucket || (st.selection.objects.is_empty() && st.selection.subjects().is_empty()) {
        let s = shown?;
        let name = s.brush.as_deref()?;
        return super::brushes::document_brushes(st).iter().any(|b| b.name == name).then_some(s.profile.is_some());
    }
    let key = (st.uid, st.revision, app.session.appearance_item());
    if let Some((k, v)) = app.ui.stroke_panel.brushed
        && k == key
    {
        return v;
    }
    let v = app.session.brushed_strokes_profiled();
    app.ui.stroke_panel.brushed = Some((key, v));
    v
}

/// The Profile select (Stroke panel, Properties panel, Control bar): each row shows its silhouette, the stroke's own
/// profile is picked, the built-in profiles come first and then the saved ones. While the strokes draw with a brush
/// ([`brush_keeps_size`]) the select is disabled and says why; it opens no list while every one is uniform, and only
/// the Uniform row, which removes the profiles, while one has a profile.
pub(crate) fn profile_select(app: &mut VectorcraftApp, ui: &mut Ui) {
    let shown = current_stroke(app);
    let cur = shown.as_ref().and_then(|s| s.profile.as_ref());
    let brushed = brush_keeps_size(app, shown.as_ref());
    let mut entries = vec![];
    // Each entry's id and silhouette, as `WidthProfile::points`.
    let mut shapes: Vec<(String, Option<Vec<_>>)> = vec![];
    let mut picked = String::new();
    let mut divide = true;
    for e in app.session.profile_entries().filter(|e| brushed != Some(true) || e.matches(None)) {
        if !e.built_in && std::mem::take(&mut divide) {
            entries.push(Entry::separator());
        }
        if e.matches(cur) {
            picked = e.id.to_string();
        }
        // A saved profile's name is the user's.
        entries.push(Entry::new(e.id.to_string(), super::label_or_name(e.label, e.built_in).to_string()));
        shapes.push((e.id.to_string(), silhouette(e.points).map(<[_]>::to_vec)));
    }
    let tip = TooltipSpec::new(tl!("Variable Width Profile"))
        .disabled_text(tl!("The stroke's brush keeps its own size, so a width profile does not change it. Remove the brush to use a profile."));
    let out = Select::new(tl!("Profile"), Bind::given(picked), &entries)
        .picture(|id: &String, cx, rect| {
            if let Some((_, points)) = shapes.iter().find(|(i, _)| i == id) {
                paint_profile(cx, rect, points.as_deref());
            }
        })
        // Brushed strokes with a profile still open the list, to its Uniform row.
        .enabled(brushed != Some(false))
        .tooltip(tip)
        .show(ui);
    if let Some(id) = out.events.landed() {
        set(app, json!({"profile": id}));
    }
}

/// The command a ≡ menu row runs to ask for the name of a new saved profile.
fn add_profile(app: &VectorcraftApp) -> CommandRef {
    let params =
        json!({"command": "stroke.widthProfile.add", "label": "Variable Width Profile", "params": {"name": app.session.next_profile_name()}});
    CommandRef::with_params("ui.paramDialog", params)
}

/// The rows of the Stroke panel's ≡ menu; their titles and enabled flags come from [`menu_states`].
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![
        PanelRow::Command(CommandRef::new("ui.stroke.options")),
        PanelRow::Separator,
        PanelRow::Command(add_profile(app)),
        PanelRow::Command(CommandRef::new("stroke.widthProfile.delete")),
        PanelRow::Command(CommandRef::new("stroke.widthProfile.reset")),
        PanelRow::Separator,
        // Opens the arrowheads file, whose symbols are the custom heads; saving it reloads them.
        PanelRow::Command(CommandRef::new("stroke.arrowheads.edit")),
    ]
}

/// The titles and enabled flags of the ≡ menu's commands.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let options = if app.ui.stroke_panel.hide_options { tl!("Show Options") } else { tl!("Hide Options") };
    states.set(CommandRef::new("ui.stroke.options"), CommandState::new().title(options));
    let enabled = |id: &str| crate::menus::enabled(app, id);
    states.set(add_profile(app), CommandState::new().title(tl!("Add to Profiles…")).enabled(enabled("stroke.widthProfile.add")));
    // Deletes the selected stroke's saved profile.
    let shown = current_stroke(app).and_then(|s| s.profile);
    let saved = shown.as_ref().and_then(|p| app.session.profile_entry(Some(p))).is_some_and(|e| !e.built_in);
    states.set(CommandRef::new("stroke.widthProfile.delete"), CommandState::new().title(tl!("Delete Profile")).enabled(saved));
    states.set(
        CommandRef::new("stroke.widthProfile.reset"),
        CommandState::new().title(tl!("Reset Profiles")).enabled(enabled("stroke.widthProfile.reset")),
    );
    states.set(
        CommandRef::new("stroke.arrowheads.edit"),
        CommandState::new().title(tl!("Edit Arrowheads…")).enabled(enabled("stroke.arrowheads.edit")),
    );
}

/// Runs the Stroke panel's own menu command: Hide or Show Options. `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if id != "ui.stroke.options" {
        return None;
    }
    app.ui.stroke_panel.hide_options ^= true;
    Some(Ok(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;

    #[test]
    fn editing_an_extended_dash_keeps_other_stroke_options_and_is_undoable() {
        use vectorcraft_engine::Session;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.line", &json!({"x1": 10, "y1": 50, "x2": 90, "y2": 50})).unwrap();
        set(&mut app, json!({"dash": [0, 6, 0, 7, 0, 8, 0, 9], "dashOffset": 3, "alignDashes": true, "cap": "round"}));
        let before = current_stroke(&app).unwrap();
        let old = style_of(Some(&before));
        let new = old.with_dash_pairs(&[(0.0, Some(6.0)), (0.0, Some(7.0)), (0.0, Some(10.0))]);
        set(&mut app, params_of(&old, &new, StrokeField::Dash(5)));
        let after = current_stroke(&app).unwrap();
        let mut expected = before.clone();
        expected.dash.as_mut().unwrap().pattern[5] = 10.0;
        assert_eq!(after, expected, "the tail, offset, alignment, cap and other options stay");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(current_stroke(&app).unwrap(), before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(current_stroke(&app).unwrap(), after);
    }

    /// #909: a profile many times the stroke's weight (made with the Width tool) draws inside its
    /// box, scaled by its widest point, and the plain bar keeps its look.
    #[test]
    fn wide_profiles_fit_their_preview() {
        let r = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(80.0, 16.0));
        let (top, bottom) = profile_outline(r, Some(&[(0.0, 1.0, 1.0), (0.5, 10.0, 10.0), (1.0, 1.0, 1.0)]));
        let all: Vec<_> = top.iter().chain(&bottom).collect();
        assert!(all.iter().all(|p| p.y >= r.top() && p.y <= r.bottom()), "{all:?}");
        let middle = all.iter().map(|p| (p.y - r.center().y).abs()).fold(0.0, f32::max);
        assert!((middle - (r.height() / 2.0 - 1.0)).abs() < 1e-3, "the widest point fills the box ({middle})");
        let (top, bottom) = profile_outline(r, None);
        assert!(top.iter().chain(&bottom).all(|p| (p.y - r.center().y).abs() <= 0.35 * (r.height() / 2.0 - 1.0) + 1e-3));
    }

    #[test]
    fn while_typing_the_panel_strokes_the_selected_characters() {
        use vectorcraft_doc::{LineJoin, NodeKind};
        use vectorcraft_engine::{Session, ViewInfo};
        use vectorcraft_tools::{PointerEvent, PointerKind};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let v = ViewInfo::default();
        app.session.select_tool("type", v).unwrap();
        for k in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(k, 50.0, 200.0), v).unwrap();
        }
        app.session.tool_text("Make this bold", v).unwrap();
        app.session.set_tool_option("select", &json!({"start": 5, "end": 9}));
        set(&mut app, json!({"weight": 4, "join": "round"}));
        let id = app.session.active().unwrap().selection.objects[0];
        let NodeKind::Text(t) = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("type") };
        let joins: Vec<(&str, LineJoin, f64)> = t.runs.iter().map(|r| (r.text.as_str(), r.style.stroke_join, r.style.stroke_width)).collect();
        assert_eq!(joins, [("Make ", LineJoin::Miter, 0.0), ("this", LineJoin::Round, 4.0), (" bold", LineJoin::Miter, 0.0)]);
        // The panel shows the selected characters' stroke.
        assert_eq!(current_stroke(&app).map(|s| (s.join, s.width)), Some((LineJoin::Round, 4.0)));
        Panel::new(app, show).frame();
    }
}
