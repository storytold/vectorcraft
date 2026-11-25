//! Gradient panel: the gradient thumbnail (its menu lists the document's gradient swatches and
//! Save to Swatches), type buttons, the Fill/Stroke proxy, angle and aspect ratio fields with
//! preset menus, Reverse, the gradient slider and the selected stop's fields.
//!
//! The slider is the framework's gradient editor: click below the ramp to add a stop, drag a stop
//! to move it (Alt drags a copy; Alt-dropping it on another stop swaps their colours), drag it off
//! to remove it, double-click it for the stop popover, and drag a diamond to move a midpoint (a
//! selected diamond's midpoint shows in Location). Dropping a colour swatch on the ramp adds a
//! stop of that colour, or recolours the stop it lands on; a global or spot colour (or a tint of
//! one) stays linked, so editing the swatch recolours the stop. The thumbnail menu marks the
//! gradient swatch the gradient was applied from. Hide Options leaves the thumbnail, the proxy
//! and the slider. Cmd/Ctrl-click on the thumbnail (the Gradient Fill box) or on the Tools
//! panel's Gradient button resets the active proxy's gradient to the linear White, Black gradient.
//!
//! With the Stroke proxy in front, the Stroke buttons lay a linear or radial gradient within the
//! stroke (placed on the page like a fill's), along it or across it (angle and aspect ratio then
//! don't apply).
//!
//! A freeform gradient shows its section instead of the angle, aspect and slider: the Draw
//! toggle (Points or Lines: how the Gradient tool adds points) and the selected point's colour
//! (click: its popover, as a stop's), opacity and spread, with Delete Point.
//!
//! The Presets menu lists ready-made gradients by theme (the built-in gradient libraries), then
//! the gradients saved with the panel menu's Save Gradient Preset…; choosing one applies it to the
//! gradient behind the active proxy, keeping its placement (`gradient.presets.*`). Hide Presets in
//! the panel menu leaves the row out.

use craft::behavior::{EditEvent, IconId, Mixed};
use craft::color_ui::gradient::stops::AltDrop;
use craft::color_ui::ui::DropperButton;
use craft::color_ui::ui::gradient::{ColorStop, GradientEditor, GradientStops, StopDrop, StopRef, StopRow, StopRows, StopRules};
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Checkbox, Radio, RadioGroup, Segmented, Swatch};
use craft::ui::contract::{Bind, EditResponse, Entry, MemoProps, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::{Divider, Icon, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::notices::Notice;
use craft::ui::patterns::presets::{PresetPicker, PresetRow, PresetSource};
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::access::PopupKind;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::{NumberField, NumberPick};
use egui::{Pos2, Ui};
use serde_json::{Value, json};
use vectorcraft_color::gradient::{MAX_REPEATS, MIN_STOPS, insert_stop, move_stop, remove_stop, set_midpoint};
use vectorcraft_color::{Color, Freeform, FreeformMode, Gradient, GradientKind, GradientMethod, GradientPaint, GradientSpread, GradientStop, Paint};
use vectorcraft_doc::{NodeId, StrokeGradientMode};
use vectorcraft_tools::params::stops_json;

// PLAN: assumed hub-w2's `panels::{proxy(app, ui), part_hidden(app, key), set_part_hidden(app, key, on), live_run, Live}` (co.A2, op.A5).
use super::{Live, active_paint, live_run, part_hidden, set_part_hidden};
use crate::VectorcraftApp;
// PLAN: assumed color-w18's `dialogs::gradient_stop::{open_at(app, index, at: Anchor), Anchor::Screen(Pos2)}` (co.A5).
use crate::dialogs::gradient_stop::{self, Anchor};
use crate::edits::{EditPath, apply_edits};
// PLAN: assumed widgets-w13's `paint_preview::{paint_face(&GradientPaint) -> FacePaint, paint_rect(&ContentPainterCx, Rect, &Paint)}` (wg.S6).
use crate::paint_preview::{paint_face, rect_face};
// PLAN: assumed widgets-w13's `panel_drag::PanelDrag` with `Paint { params, .. }` and `color(&self) -> Option<Color>` (wg.S7).
use crate::panel_drag::PanelDrag;

/// The Angle field's preset menu, degrees.
const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The Aspect Ratio field's preset menu, percent.
const ASPECT_PRESETS: [f64; 9] = [10.0, 25.0, 50.0, 75.0, 100.0, 150.0, 200.0, 300.0, 400.0];
// PLAN: the sheet drops `HIDE_OPTIONS` and `HIDE_PRESETS`, but each key is read by `show`, `menu_states` and `run`; kept as the op.A5 hide-store keys.
/// The hide-store key of Show/Hide Options (`panels::part_hidden`), shared with the panel's display sizes.
const HIDE_OPTIONS: &str = "gradient-hide-options";
/// The hide-store key of Show/Hide Presets.
const HIDE_PRESETS: &str = "gradient-hide-presets";

/// The panel menu's command ids (`ui.gradient.<row>`), which [`menu_rows`], [`menu_states`] and
/// [`run`] share.
const MENU_OPTIONS: &str = "ui.gradient.toggleOptions";
const MENU_ADD_TO_SWATCHES: &str = "ui.gradient.addToSwatches";
const MENU_REVERSE: &str = "ui.gradient.reverse";
const MENU_RESET: &str = "ui.gradient.resetWhiteBlack";
const MENU_PRESETS: &str = "ui.gradient.togglePresets";
const MENU_SAVE_PRESET: &str = "ui.gradient.savePreset";
const MENU_DELETE_PRESET: &str = "ui.gradient.deletePreset";

// PLAN: assumed hub-w1's `UiState::gradient_panel: panels::gradient::GradientUi` (`#[serde(skip)]`, co.A11).
/// The panel's view state that lives only while the app runs (`app.ui.gradient_panel`).
#[derive(Clone, Debug, Default)]
pub struct GradientUi {
    /// The selected midpoint diamond and the object it was picked on: it stands while no stop is
    /// selected and that object is still the first selected.
    pub(crate) mid_selected: Option<(usize, Option<NodeId>)>,
    /// The stops of the gradient the slider's open gesture began from, from its `Begin` until its
    /// `Commit` or `Cancel`. Every preview restarts from the interaction's snapshot and
    /// `paint.editGradient` sorts the stops, so the gradient shown mid-gesture is the last preview
    /// re-sorted: the editor's tags name stops of this one.
    pub(crate) begun: Option<Vec<GradientStop>>,
}

/// Select stop `i` (the session's selected stop, shared with the Gradient tool's annotator, the
/// Color panel and agents), or none.
fn select_stop(app: &mut VectorcraftApp, i: Option<usize>) {
    app.run("gradient.selectStop", json!({ "index": i })).ok();
}

/// A colour dropped on the ramp: recolours stop `on` (fully opaque), or adds an opaque stop of
/// that colour at `offset`, linked to `link` (a global swatch and tint) when given. Returns the
/// stops and the index of the stop that took the colour.
fn drop_color(g: &Gradient, on: Option<usize>, offset: f32, c: Color, link: Option<(String, f32)>) -> (Vec<GradientStop>, usize) {
    let (mut v, i) = match on.filter(|i| *i < g.stops.len()) {
        Some(i) => (g.stops.clone(), i),
        None => insert_stop(g, offset),
    };
    v[i].set_color(c, link);
    v[i].opacity = 1.0;
    (v, i)
}

/// The swatch link a dropped paint's params give a stop: `{swatch, tint?}` of a global or spot
/// colour or a tint swatch (a proxy's tint carries its percentage); none for other colours.
pub(crate) fn dropped_link(app: &VectorcraftApp, params: &Value) -> Option<(String, f32)> {
    let name = params.get("swatch")?.as_str()?;
    let d = &app.session.active()?.doc;
    let (base, own) = d.swatch_link(name)?;
    let tint = params.get("tint").and_then(Value::as_f64).map_or(own, |t| (t / 100.0).clamp(0.0, 1.0) as f32);
    Some((base, tint))
}

/// Recolor the selected stop of the gradient behind the active proxy with `c`, as dropping the
/// paint of `params` on the stop does (fully opaque, linked by [`dropped_link`]), or the selected
/// point of a freeform gradient; the stop or point stays selected and keeps the color's model.
/// Returns false, and changes nothing, when the active paint is not a gradient or no stop or point
/// is selected.
pub(crate) fn recolor_selected(app: &mut VectorcraftApp, c: Color, params: &Value) -> bool {
    let Paint::Gradient(g) = active_paint(app) else { return false };
    if edits_points(app, &g) {
        let Some(i) = app.session.selected_freeform_point().filter(|i| *i < shown_points(&g).points.len()) else { return false };
        edit_point(app, "paint.freeform.setPoint", json!({ "index": i, "color": super::color_json(&c), "keepModel": true }));
        return true;
    }
    let Some(i) = app.session.selected_stop().filter(|i| *i < g.gradient.stops.len()) else { return false };
    let (v, i) = drop_color(&g.gradient, Some(i), 0.0, c, dropped_link(app, params));
    set_stops_keeping_models(app, &v, Some(i), Live::Released);
    true
}

// ---------- shared edits ----------

/// The icon segments of `entries`: value, icon and tooltip (also the segment's name).
fn icon_entries<T>(entries: impl IntoIterator<Item = (T, &'static str, &'static str)>) -> Vec<Entry<T>> {
    entries.into_iter().map(|(v, icon, tip)| Entry::new(v, tip).icon(IconId::new(icon))).collect()
}

/// `value` as a segmented row's binding: `None` lights no segment.
fn lit<T>(value: Option<T>) -> Bind<'static, T>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    Bind::given(value.map_or(Mixed::Mixed, Mixed::Same))
}

/// The Type row: Linear, Radial, Conic, Diamond and Freeform for the paint behind the active
/// proxy (one of them lit when it is a gradient). The Gradient panel's and the Properties panel's.
pub(crate) fn type_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let kind = current(app).map(|g| g.gradient.kind);
    let entries = icon_entries([
        (GradientKind::Linear, "dc-grad-linear", tl!("Linear Gradient")),
        (GradientKind::Radial, "dc-grad-radial", tl!("Radial Gradient")),
        (GradientKind::Conic, "dc-grad-conic", tl!("Conic Gradient")),
        (GradientKind::Diamond, "dc-grad-diamond", tl!("Diamond Gradient")),
        (GradientKind::Freeform, "dc-grad-freeform", tl!("Freeform Gradient")),
    ]);
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Type:")).shade(Shade::Muted).show(ui);
        let pick = Segmented::new(tl!("Type"), lit(kind), &entries).show(ui);
        if let Some(k) = pick.events.committed() {
            edit(app, json!({"kind": k.label().to_lowercase()}), Live::Released);
        }
    });
}

/// Set how the Gradient tool adds the points of the freeform gradient behind the active proxy.
fn set_draw_mode(app: &mut VectorcraftApp, mode: FreeformMode) {
    edit(app, json!({ "mode": mode.label().to_lowercase() }), Live::Released);
}

/// The Properties panel's Draw row for a freeform gradient: Points and Lines as radio buttons.
pub(crate) fn draw_radios(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(g) = current(app).filter(|g| g.gradient.kind == GradientKind::Freeform) else { return };
    let mode = shown_points(&g).mode;
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Draw:")).shade(Shade::Muted).show(ui);
        let group = RadioGroup::new(tl!("Draw"), Bind::given(mode)).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Radio::new(FreeformMode::Points).label(tl!("Points")).show(ui);
                Radio::new(FreeformMode::Lines).label(tl!("Lines")).show(ui);
            });
        });
        if let Some(m) = group.output.events.committed() {
            set_draw_mode(app, *m);
        }
    });
}

/// Do the panel, the stop popover and swatch drops edit the points of `g`, the gradient behind
/// the active proxy? Only for a freeform gradient on art: the one the active tool paints with
/// lies on no object, so its stops stand for its colors, as in the Color panel.
pub(crate) fn edits_points(app: &VectorcraftApp, g: &GradientPaint) -> bool {
    g.gradient.kind == GradientKind::Freeform && !app.session.tool_paints(!app.session.fill_active)
}

/// The gradient behind the active proxy.
fn current(app: &VectorcraftApp) -> Option<GradientPaint> {
    match active_paint(app) {
        Paint::Gradient(g) => Some(*g),
        _ => None,
    }
}

/// Edit the gradient behind the active proxy (`paint.editGradient` params).
fn edit(app: &mut VectorcraftApp, params: Value, phase: Live) {
    let mut p = params;
    p["stroke"] = json!(!app.session.fill_active);
    live_run(app, "Gradient", "paint.editGradient", p, phase);
}

/// Write `stops` to the gradient behind the active proxy (live while dragging) and, once
/// released, select stop `select` (the panel, the stop popover and the Color panel share this).
pub(crate) fn set_stops(app: &mut VectorcraftApp, stops: &[GradientStop], select: Option<usize>, phase: Live) {
    write_stops(app, json!({ "stops": stops_json(stops) }), select, phase);
}

/// [`set_stops`] with `keepModel`, for colors copied from existing paint (a swatch, a dropped
/// color, a sample of the gradient) and for the Color panel's sliders, which work in the model
/// picked there: the stops keep the color models given, in a CMYK document too.
pub(crate) fn set_stops_keeping_models(app: &mut VectorcraftApp, stops: &[GradientStop], select: Option<usize>, phase: Live) {
    write_stops(app, json!({ "stops": stops_json(stops), "keepModel": true }), select, phase);
}

/// Edit the stops with `params` (live while dragging) and, once released, select stop `select`.
fn write_stops(app: &mut VectorcraftApp, params: Value, select: Option<usize>, phase: Live) {
    edit(app, params, phase);
    if phase == Live::Released
        && let Some(i) = select
        && app.session.selected_stop() != Some(i)
    {
        select_stop(app, Some(i));
    }
}

/// Paint the active proxy with the linear White, Black gradient, as one undo step (a solid or no
/// paint becomes that gradient). It runs `paint.setFill` or `paint.setStroke`, so characters
/// selected with the Type tool take it alone and an unpainted stroke gets its 1 pt weight. With
/// nothing selected it sets the default. Cmd/Ctrl-click on the Gradient Fill box or on the Tools
/// panel's Gradient button runs it.
pub(crate) fn reset_to_linear_white_black(app: &mut VectorcraftApp) {
    let cmd = if app.session.fill_active { "paint.setFill" } else { "paint.setStroke" };
    app.run(cmd, json!({ "gradient": { "kind": "linear", "stops": stops_json(&Gradient::default().stops) } })).ok();
}

/// Save the gradient as a swatch; its stops keep their color models.
fn save_to_swatches(app: &mut VectorcraftApp, g: &GradientPaint) {
    app.run("swatch.new", super::swatch_paint_params(&Paint::Gradient(Box::new(g.clone())))).ok();
}

/// The document's gradient swatches (groups included): name and gradient.
fn gradient_swatches(app: &VectorcraftApp) -> Vec<(String, GradientPaint)> {
    let Some(st) = app.session.active() else { return vec![] };
    let d = &st.doc;
    d.swatches
        .iter()
        .chain(d.swatch_groups.iter().flat_map(|g| g.swatches.iter()))
        .filter_map(|s| match &s.paint {
            Paint::Gradient(g) => Some((s.name.clone(), (**g).clone())),
            _ => None,
        })
        .collect()
}

/// The selected midpoint diamond: kept for the object it was picked on, and given up when a stop
/// is selected.
fn selected_mid(app: &VectorcraftApp, stops: usize) -> Option<usize> {
    let (i, owner) = app.ui.gradient_panel.mid_selected?;
    (app.session.selected_stop().is_none() && i + 1 < stops && owner == first_id(app)).then_some(i)
}

/// A stored fraction as the percent a field shows, without the `f32` noise (`0.3` is `30`, not `30.000001`).
fn percent(v: f32) -> f64 {
    (f64::from(v) * 1e6).round() / 1e4
}

fn first_id(app: &VectorcraftApp) -> Option<NodeId> {
    app.session.active().and_then(|d| d.selection.objects.first().copied())
}

// ---------- UI ----------

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    Stack::new().fill_width().show(ui, |ui, _| body(app, ui));
}

/// The panel's rows, top to bottom. Hide Options leaves the thumbnail, the proxy and the slider;
/// a freeform gradient shows its section in place of the angle, aspect and slider.
fn body(app: &mut VectorcraftApp, ui: &mut Ui) {
    let gp = current(app);
    let is_grad = gp.is_some();
    let g = gp.unwrap_or_else(|| GradientPaint::new(Gradient::default()));
    let kind = g.gradient.kind;
    let hidden = part_hidden(app, HIDE_OPTIONS);
    // Header: the thumbnail and its swatch menu, the type buttons and Edit Gradient.
    Row::new().show(ui, |ui, _| {
        fill_box(app, ui, &g, is_grad);
        if hidden {
            super::proxy(app, ui);
            return;
        }
        Stack::new().show(ui, |ui, _| {
            type_row(app, ui);
            if Button::new(tl!("Edit Gradient")).show(ui).clicked() {
                app.select_tool("gradient");
            }
        });
    });
    let freeform = is_grad && kind == GradientKind::Freeform;
    let points = is_grad && edits_points(app, &g);
    if hidden {
        if !points {
            stops_editor(app, ui, &g, is_grad);
        }
        return;
    }
    if !part_hidden(app, HIDE_PRESETS) {
        presets_picker(app, ui, is_grad.then_some(&g));
    }
    // How the gradient lies on the stroke behind the Stroke proxy (a linear or radial one).
    let stroke_mode = (!app.session.fill_active && is_grad && !freeform).then(|| app.session.shown_stroke().map(|st| st.gradient_mode)).flatten();
    let modes = icon_entries([
        (StrokeGradientMode::Within, "dc-grad-stroke-within", tl!("Gradient within stroke")),
        (StrokeGradientMode::Along, "dc-grad-stroke-along", tl!("Gradient along stroke")),
        (StrokeGradientMode::Across, "dc-grad-stroke-across", tl!("Gradient across stroke")),
    ]);
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Stroke:")).shade(Shade::Muted).show(ui);
        let pick = Segmented::new(tl!("Stroke"), lit(stroke_mode), &modes).enabled(stroke_mode.is_some()).show(ui);
        if let Some(m) = pick.events.committed() {
            edit(app, json!({"strokeMode": m.name()}), Live::Released);
        }
    });
    // Along and across, the gradient follows the path: its angle and aspect ratio don't apply.
    let placed = stroke_mode.is_none_or(|m| m == StrokeGradientMode::Within);
    if points {
        freeform_rows(app, ui, &g);
        return;
    }
    // A freeform gradient the active tool paints with: the proxy and the Draw toggle, its stops and
    // their fields.
    if freeform {
        draw_row(app, ui, shown_points(&g).mode);
        stops_editor(app, ui, &g, is_grad);
        host_fields(app, ui, &g.gradient, is_grad);
        return;
    }
    method_select(app, ui, &g, is_grad);
    spread_select(app, ui, &g, is_grad);
    // The proxy beside the angle and aspect ratio fields.
    Row::new().show(ui, |ui, _| {
        super::proxy(app, ui);
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                angle_field(app, ui, &g, placed);
                let reverse = IconButton::new(IconId::new("dc-reverse"))
                    .label(tl!("Reverse Gradient"))
                    .tooltip(tl!("Reverse Gradient"))
                    .enabled(is_grad)
                    .show(ui);
                if reverse.clicked() {
                    edit(app, json!({"reverse": true}), Live::Released);
                }
            });
            aspect_field(app, ui, &g, placed);
        });
    });
    stops_editor(app, ui, &g, is_grad);
    host_fields(app, ui, &g.gradient, is_grad);
    // Dither applies to gradient fills only: off and dimmed behind the Stroke proxy.
    let fill = app.session.fill_active;
    let dither = is_grad && fill && g.gradient.dither;
    let check = Checkbox::new(Bind::given(dither))
        .label(tl!("Dither"))
        .tooltip(tl!("Fine noise hides banding in the gradient fill"))
        .enabled(is_grad && fill)
        .show(ui);
    if let Some(on) = check.events.committed() {
        edit(app, json!({"dither": on}), Live::Released);
    }
    if !is_grad {
        Notice::new(Tone::Neutral, tl!("Click the ramp or a type button to apply a gradient.")).show(ui);
    }
}

/// Apply a number field's edits to the gradient behind the active proxy through `apply`: previewed
/// while scrubbed or stepped, one undo step when the gesture ends. A blank value changes nothing.
fn apply_number(app: &mut VectorcraftApp, edit: &EditResponse<Option<f64>>, mut apply: impl FnMut(&mut VectorcraftApp, f64, Live)) {
    apply_edits(app, edit, EditPath::Preview, |app, value: &Option<f64>, phase| {
        if let Some(v) = *value {
            apply(app, v, phase);
        }
    });
}

/// A number field's preset rows: each number with its unit.
fn number_presets(values: &[f64], unit: &str) -> Vec<Entry<NumberPick>> {
    values.iter().map(|v| Entry::new(NumberPick::Number(*v), format!("{v}{unit}"))).collect()
}

/// The Angle field with its icon, dimmed while the gradient follows a stroke.
fn angle_field(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, placed: bool) {
    Icon::new(IconSpec::svg(IconId::new("rotate-ccw"))).tooltip(tl!("Angle")).enabled(placed).show(ui);
    let angle = g.geom.map_or(g.angle, |x| x.angle_deg());
    let presets = number_presets(&ANGLE_PRESETS, "°");
    let field = NumberField::new(tl!("Angle"), Bind::given(Some((angle * 10.0).round() / 10.0)))
        .unit_kind(UnitKind::Degrees)
        .range(-180.0..=180.0)
        .presets(&presets)
        .enabled(placed)
        .show(ui);
    apply_number(app, &field.edit, |app, a, phase| edit(app, json!({"angle": a}), phase));
}

/// The Aspect Ratio field with its icon. The default gradient (with nothing selected, or the Live
/// Paint Bucket's) fits each object and has no aspect: the field is dimmed for it, and for a
/// gradient that is not centred or follows a stroke.
fn aspect_field(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, placed: bool) {
    let kind = g.gradient.kind;
    let radial = kind.is_centered() && placed && app.session.gradient_edit_has_objects(!app.session.fill_active);
    Row::new().show(ui, |ui, _| {
        Icon::new(IconSpec::svg(IconId::new("scaling"))).tooltip(tl!("Aspect Ratio")).enabled(radial).show(ui);
        // A mirrored conic's negative aspect shows as its size; an edit keeps the mirror.
        let mirror = g.geom.map_or(1.0, |x| if x.is_mirrored() && kind == GradientKind::Conic { -1.0 } else { 1.0 });
        let asp = g.geom.map_or(100.0, |x| (x.aspect.abs() * 1000.0).round() / 10.0);
        let presets = number_presets(&ASPECT_PRESETS, "%");
        let field =
            NumberField::new(tl!("Aspect Ratio"), Bind::given(Some(asp))).unit("%").range(0.5..=f64::MAX).presets(&presets).enabled(radial).show(ui);
        apply_number(app, &field.edit, |app, a, phase| edit(app, json!({"aspect": a * mirror}), phase));
    });
}

/// The Method menu: how a linear or radial gradient mixes the colours between its stops, Classic
/// or Perceptual, or around the hue wheel one way or the other (Hue Clockwise and Hue
/// Counterclockwise, VectorCraft's own methods).
fn method_select(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, is_grad: bool) {
    let methods = [
        (GradientMethod::Classic, tl!("Classic")),
        (GradientMethod::Perceptual, tl!("Perceptual")),
        (GradientMethod::HueClockwise, tl!("Hue Clockwise")),
        (GradientMethod::HueCounterclockwise, tl!("Hue Counterclockwise")),
    ]
    .map(|(m, label)| Entry::new(m, label));
    let current = g.gradient.method;
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Method:")).shade(Shade::Muted).show(ui);
        let pick = Select::new(tl!("Method"), Bind::given(current), &methods).enabled(is_grad).show(ui);
        if let Some(m) = pick.events.committed().filter(|m| **m != current) {
            edit(app, json!({"method": m.id()}), Live::Released);
        }
    });
}

/// The Spread row: what a linear, radial, conic or diamond gradient paints past the ends of its
/// stops (Pad, Repeat or Reflect), and how many times Repeat and Reflect run the stops along the
/// vector (1 to [`MAX_REPEATS`]; dimmed with Pad).
fn spread_select(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, is_grad: bool) {
    let spreads = [(GradientSpread::Pad, tl!("Pad")), (GradientSpread::Repeat, tl!("Repeat")), (GradientSpread::Reflect, tl!("Reflect"))]
        .map(|(s, label)| Entry::new(s, label));
    let current = g.gradient.spread;
    let presets = number_presets(&REPEAT_PRESETS, "");
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Spread:")).shade(Shade::Muted).show(ui);
        let pick = Select::new(tl!("Spread"), Bind::given(current), &spreads).enabled(is_grad).show(ui);
        if let Some(s) = pick.events.committed().filter(|s| **s != current) {
            edit(app, json!({"spread": s.label().to_lowercase()}), Live::Released);
        }
        let repeats = NumberField::new(tl!("Repeats"), Bind::given(Some(f64::from(g.gradient.repeat_count()))))
            .range(1.0..=f64::from(MAX_REPEATS))
            .commit_decimals(0)
            .presets(&presets)
            .enabled(is_grad && !current.is_pad())
            .show(ui);
        apply_number(app, &repeats.edit, |app, r, phase| edit(app, json!({"repeats": r.round()}), phase));
    });
}

/// The Repeats field's menu.
const REPEAT_PRESETS: [f64; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 10.0];

/// A key of the Presets picker: a library (theme) heading its presets, or a preset by name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, MemoProps)]
enum PresetKey {
    /// A built-in library, or (`None`) the presets saved with Save Gradient Preset….
    Library(Option<&'static str>),
    /// A preset, by its name.
    Preset(String),
}

/// The Presets picker: the preset gradients by library (theme), then the saved ones; a choice
/// applies to the gradient behind the active proxy, keeping its placement
/// (`gradient.presets.apply`; a solid or no paint becomes the gradient). The picker shows the
/// preset the gradient is, if any.
fn presets_picker(app: &mut VectorcraftApp, ui: &mut Ui, g: Option<&GradientPaint>) {
    let shown = g.and_then(|g| app.session.gradient_preset_name(&g.gradient)).map(PresetKey::Preset);
    let enabled = crate::menus::enabled(app, "gradient.presets.apply");
    let rows = preset_rows(app);
    let source = PresetSource::listed(&rows);
    let mut picked = None;
    Row::new().enabled(enabled).show(ui, |ui, _| {
        Text::new(tl!("Presets:")).shade(Shade::Muted).show(ui);
        picked = PresetPicker::new(tl!("Presets"), Bind::given(shown.clone()), &source, Bind::kept()).show(ui).picked;
    });
    if let Some(PresetKey::Preset(name)) = picked {
        app.run("gradient.presets.apply", json!({ "name": name })).ok();
    }
}

/// The Presets picker's rows: each library's heading over its presets, then the saved presets
/// under Saved.
fn preset_rows(app: &VectorcraftApp) -> Vec<PresetRow<PresetKey>> {
    let mut rows = Vec::new();
    // The library of the rows added last (`Some(None)`: the saved presets).
    let mut library: Option<Option<&'static str>> = None;
    for e in app.session.gradient_presets() {
        if library != Some(e.library) {
            library = Some(e.library);
            rows.push(PresetRow::group(PresetKey::Library(e.library), e.library.map_or(tl!("Saved"), |l| tl!(l)), 0));
        }
        let paint = Paint::Gradient(Box::new(GradientPaint::new(e.gradient)));
        rows.push(PresetRow::item(PresetKey::Preset(e.name.clone()), e.name, 1).preview_face(ThumbAspect::SQUARE, rect_face(&paint)));
    }
    rows
}

/// The freeform points shown: the gradient's own, else (unplaced) as many as it places, coloured
/// along the stops.
pub(crate) fn shown_points(g: &GradientPaint) -> std::borrow::Cow<'_, Freeform> {
    g.freeform_on(vectorcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0))
}

/// Run a `paint.freeform.*` command on the paint behind the active proxy.
pub(crate) fn edit_point(app: &mut VectorcraftApp, cmd: &str, mut params: Value) {
    params["stroke"] = json!(!app.session.fill_active);
    live_run(app, "Gradient", cmd, params, Live::Released);
}

/// The proxy beside the Draw toggle of a freeform gradient, whose mode is `mode`.
fn draw_row(app: &mut VectorcraftApp, ui: &mut Ui, mode: FreeformMode) {
    let modes = icon_entries([
        (FreeformMode::Points, "circle", tl!("Points: clicks add free points")),
        (FreeformMode::Lines, "spline", tl!("Lines: clicks add points joined by a line")),
    ]);
    Row::new().show(ui, |ui, _| {
        super::proxy(app, ui);
        Text::new(tl!("Draw:")).shade(Shade::Muted).show(ui);
        let pick = Segmented::new(tl!("Draw"), Bind::given(mode), &modes).show(ui);
        if let Some(m) = pick.events.committed() {
            set_draw_mode(app, *m);
        }
    });
}

/// The freeform section: the proxy beside the Draw toggle, then the selected point's colour
/// (click: its popover, as a stop's), opacity and spread, and Delete Point.
fn freeform_rows(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint) {
    let f = shown_points(g);
    draw_row(app, ui, f.mode);
    let sel = app.session.selected_freeform_point().filter(|i| *i < f.points.len());
    let point = sel.and_then(|i| f.points.get(i)).copied();
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Color:")).shade(Shade::Muted).show(ui);
        let color = point.map_or(Mixed::Mixed, |p| Mixed::Same(srgba(&p.color, 1.0)));
        let chip = Swatch::new(tl!("Edit the point"), color).tooltip(tl!("Edit the point")).enabled(sel.is_some()).show(ui);
        // The selected point's popover, as a stop's swatch opens the stop's (#4953).
        if chip.clicked
            && let Some(i) = sel
        {
            gradient_stop::open_at(app, i, Anchor::Screen(chip.response.rect.left_bottom()));
        }
        if let Some(p) = point {
            Text::new(p.color.to_hex().to_uppercase()).shade(Shade::Muted).show(ui);
        }
    });
    Row::new().show(ui, |ui, _| {
        for (label, key, value) in [(tl!("Opacity"), "opacity", point.map(|p| p.opacity)), (tl!("Spread"), "spread", point.map(|p| p.spread))] {
            // Blank while no point is selected.
            let shown = value.map(|v| (f64::from(v) * 1000.0).round() / 10.0);
            let field = NumberField::new(label, Bind::given(shown)).range(0.0..=100.0).unit("%").enabled(sel.is_some()).show(ui);
            if let Some(Some(v)) = field.edit.events.committed()
                && let Some(i) = sel
            {
                edit_point(app, "paint.freeform.setPoint", json!({ "index": i, key: v.clamp(0.0, 100.0) / 100.0 }));
            }
        }
        let can_del = sel.is_some() && f.points.len() > 1;
        let delete = IconButton::new(IconId::new("trash-2")).label(tl!("Delete Point")).tooltip(tl!("Delete Point")).enabled(can_del).show(ui);
        if delete.clicked()
            && let Some(i) = sel
        {
            edit_point(app, "paint.freeform.deletePoint", json!({ "index": i }));
        }
    });
    if sel.is_none() {
        Notice::new(Tone::Neutral, tl!("Click the art with the Gradient tool to add or select points.")).show(ui);
    }
}

/// `c` at `opacity` as the framework's colour.
fn srgba(c: &Color, opacity: f32) -> Srgba {
    let [r, g, b] = c.to_rgb();
    Srgba::new(r, g, b, opacity)
}

/// The Gradient Fill box (click: apply a gradient; Cmd/Ctrl-click: reset to the linear White,
/// Black gradient) and, beside it, the menu of gradient swatches with Save to Swatches.
fn fill_box(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, is_grad: bool) {
    let first = g.gradient.stops.first().map_or(Srgba::BLACK, |s| srgba(&s.color, s.opacity));
    let thumb = Swatch::new(tl!("Gradient Fill"), first).face(paint_face(g)).tooltip(tl!("Gradient Fill: click to apply")).show(ui);
    if thumb.clicked {
        if thumb.activation.is_some_and(|a| a.mods.cmd) {
            reset_to_linear_white_black(app);
        } else if !is_grad {
            edit(app, json!({}), Live::Released);
        }
    }
    let swatches = gradient_swatches(app);
    let chevron = IconButton::new(IconId::new("chevron-down"))
        .label(tl!("Gradient swatches"))
        .tooltip(tl!("Gradient swatches"))
        .popup(PopupKind::Menu)
        .show(ui);
    let mut chosen = None;
    let mut save = false;
    Menu::new(tl!("Gradient swatches")).opened_by(&chevron.response, Trigger::Press).show(ui, |ui, _| {
        if swatches.is_empty() {
            MenuItem::new(tl!("No gradient swatches")).enabled(false).show(ui);
        }
        for (name, grad) in &swatches {
            // The gradient swatch the gradient was applied from is checked.
            let row = MenuItem::new(name.clone()).sample(ThumbAspect::SQUARE, paint_face(grad)).check(g.swatch.as_ref() == Some(name));
            if row.show(ui).ran {
                chosen = Some(name.clone());
            }
        }
        Divider::horizontal().show(ui);
        save = MenuItem::new(tl!("Save to Swatches")).enabled(is_grad).show(ui).ran;
    });
    if save {
        save_to_swatches(app, g);
    }
    if let Some(name) = chosen {
        super::apply_click(app, ui, json!({ "swatch": name }));
    }
}

/// The host fields row under the slider: the selected stop's colour (click: its popover; the
/// eyedropper samples one from the art) and caption, its opacity and location (a selected
/// diamond's midpoint instead), and Delete Stop, which never goes below two stops.
fn host_fields(app: &mut VectorcraftApp, ui: &mut Ui, g: &Gradient, is_grad: bool) {
    let stops = &g.stops;
    let sel = app.session.selected_stop().filter(|i| *i < stops.len() && is_grad);
    let mid = selected_mid(app, stops.len()).filter(|_| is_grad);
    let stop = sel.and_then(|i| stops.get(i));
    if let (Some(i), Some(s)) = (sel, stop) {
        Row::new().show(ui, |ui, _| {
            Text::new(tl!("Color:")).shade(Shade::Muted).show(ui);
            let chip = Swatch::new(tl!("Edit the stop"), srgba(&s.color, 1.0)).tooltip(tl!("Edit the stop")).show(ui);
            if chip.clicked {
                open_popover(app, i, chip.response.rect.left_bottom());
            }
            // A linked stop names its swatch and tint.
            let caption = match &s.swatch {
                Some(n) => format!("{n} {}%", vectorcraft_color::tint_percent(s.tint)),
                None => s.color.to_hex().to_uppercase(),
            };
            Text::new(caption).shade(Shade::Muted).show(ui);
            let armed = app.session.tool_id() == "eyedropper";
            let dropper = DropperButton::new(tl!("Eyedropper: click the art to sample the stop's colour"), Bind::given(armed)).show(ui);
            if dropper.request.is_some() {
                stop_eyedropper(app);
            }
        });
    }
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Opacity:")).shade(Shade::Muted).show(ui);
        let shown = stop.map_or(100.0, |s| f64::from(s.opacity) * 100.0);
        let opacity = NumberField::new(tl!("Opacity"), Bind::given(Some(shown))).range(0.0..=100.0).unit("%").enabled(sel.is_some()).show(ui);
        apply_number(app, &opacity.edit, |app, v, phase| {
            let Some(i) = sel else { return };
            let mut v2 = stops.clone();
            if let Some(s) = v2.get_mut(i) {
                s.opacity = (v / 100.0).clamp(0.0, 1.0) as f32;
            }
            set_stops(app, &v2, Some(i), phase);
        });
        Text::new(tl!("Location:")).shade(Shade::Muted).show(ui);
        let loc = match (mid, stop) {
            (Some(m), _) => stops.get(m).map_or(0.0, |s| percent(s.midpoint)),
            (None, Some(s)) => percent(s.offset),
            _ => 0.0,
        };
        let location =
            NumberField::new(tl!("Location"), Bind::given(Some(loc))).range(0.0..=100.0).unit("%").enabled(sel.is_some() || mid.is_some()).show(ui);
        apply_number(app, &location.edit, |app, v, phase| {
            let v = (v / 100.0) as f32;
            if let Some(m) = mid {
                set_stops(app, &set_midpoint(stops, m, v), None, phase);
            } else if let Some(i) = sel {
                let (v2, ni) = move_stop(stops, i, v);
                set_stops(app, &v2, Some(ni), phase);
            }
        });
        let can_del = sel.is_some() && stops.len() > MIN_STOPS;
        let delete = IconButton::new(IconId::new("trash-2")).label(tl!("Delete Stop")).tooltip(tl!("Delete Stop")).enabled(can_del).show(ui);
        if delete.clicked()
            && let Some(i) = sel
            && let Some(v) = remove_stop(stops, i)
        {
            let next = i.min(v.len().saturating_sub(1));
            set_stops(app, &v, Some(next), Live::Released);
        }
    });
}

/// The editor's stops of `g`: each stop's tag is its index in `g` plus one, which [`app_stops`]
/// reads back; the opacity is the colour's alpha.
fn editor_stops(g: &GradientPaint) -> GradientStops {
    let stops = g.gradient.stops.iter().zip(1_u64..);
    GradientStops::new(stops.map(|(s, tag)| ColorStop::new(s.offset, srgba(&s.color, s.opacity)).with_midpoint(s.midpoint).with_tag(tag)).collect())
}

/// The app's stops for the editor's `s`, read by the tags [`editor_stops`] gave `base` (a stop of
/// gradient `g`'s stops when the gesture began, else of its stops now). A tagged stop keeps its
/// source's colour, model, swatch link and tint, and takes the editor's location, opacity and
/// midpoint; a tag that appears twice (an Alt copy) keeps its source's for both, and a colour swap
/// moves the tags with the colours. An added stop (tag 0) takes the colour `insert_stop` samples
/// from `base` at its location, unlinked.
fn app_stops(g: &Gradient, base: &[GradientStop], s: &GradientStops) -> Vec<GradientStop> {
    let mut from = g.clone();
    from.stops = base.to_vec();
    s.color
        .iter()
        .map(|c| {
            let source = usize::try_from(c.tag).ok().and_then(|tag| tag.checked_sub(1)).and_then(|i| base.get(i)).cloned();
            let mut stop = source.unwrap_or_else(|| {
                let (added, i) = insert_stop(&from, c.location);
                added.get(i).cloned().unwrap_or_else(|| GradientStop::new(c.location, Color::rgb(c.color.r, c.color.g, c.color.b)))
            });
            stop.offset = c.location;
            stop.opacity = c.color.a;
            stop.midpoint = c.midpoint;
            stop
        })
        .collect()
}

/// The gradient slider: the framework's editor over the stops (one row, colour stops with their
/// opacity, midpoint diamonds, Alt-drop swaps colours), with the panel's fields under it instead
/// of the editor's. Its edits keep the stops' models; a colour dropped on it recolours the stop it
/// lands on or adds one; a stop's colour request opens the stop popover.
fn stops_editor(app: &mut VectorcraftApp, ui: &mut Ui, g: &GradientPaint, is_grad: bool) {
    let stops = &g.gradient.stops;
    let base = app.ui.gradient_panel.begun.clone().unwrap_or_else(|| stops.clone());
    // The ramp and new stops sample the app's own gradient (its method, its colour models). The
    // editor's sampler is a `'static` closure, so it owns its copy of the gradient.
    let gradient = g.gradient.clone();
    let sampler = move |s: &GradientStops, t: f32| {
        let mut shown = gradient.clone();
        shown.stops = app_stops(&gradient, &base, s);
        let (c, o) = shown.sample(t);
        srgba(&c, o)
    };
    let selected = match selected_mid(app, stops.len()) {
        Some(m) => Some(StopRef::Midpoint(StopRow::Color, m)),
        None => app.session.selected_stop().filter(|i| *i < stops.len()).map(StopRef::Color),
    };
    let mut shared = selected;
    let out = GradientEditor::new(tl!("Gradient"), Bind::given(editor_stops(g)))
        .rules(StopRules::new(StopRows::Combined).alt_drop(AltDrop::Swap))
        .fields(false)
        .sampler(&sampler)
        .shared_selection(Bind::bound(&mut shared))
        .drop_target::<PanelDrag>()
        .show(ui);
    let events = &out.edit.events;
    if !is_grad {
        // The slider shows the default gradient: an edit of it applies a gradient first.
        if events.committed().is_some() {
            edit(app, json!({}), Live::Released);
        }
        return;
    }
    if events.began() {
        app.ui.gradient_panel.begun = Some(stops.clone());
    }
    apply_edits(app, &out.edit, EditPath::Preview, |app, s: &GradientStops, phase| {
        let base = app.ui.gradient_panel.begun.clone().unwrap_or_else(|| stops.clone());
        set_stops_keeping_models(app, &app_stops(&g.gradient, &base, s), None, phase);
    });
    if matches!(events.last(), Some(EditEvent::Commit { .. } | EditEvent::Cancel { .. } | EditEvent::Set { .. })) {
        app.ui.gradient_panel.begun = None;
    }
    // A selection made mid-gesture lands with the gesture's commit, as the stops do.
    if out.selected != selected && (events.is_empty() || events.committed().is_some()) {
        match out.selected {
            Some(StopRef::Midpoint(_, m)) => {
                app.ui.gradient_panel.mid_selected = Some((m, first_id(app)));
                if app.session.selected_stop().is_some() {
                    select_stop(app, None);
                }
            }
            Some(StopRef::Color(i)) => {
                app.ui.gradient_panel.mid_selected = None;
                if app.session.selected_stop() != Some(i) {
                    select_stop(app, Some(i));
                }
            }
            _ => {}
        }
    }
    if let Some(request) = &out.color_requested {
        let at = request.rect.map_or(out.edit.response.rect.left_bottom(), |r| r.left_bottom());
        open_popover(app, request.index, at);
    }
    // A colour swatch dropped on the slider: a global or spot colour (or a tint of one) stays linked.
    if let Some((place, drag)) = &out.dropped
        && let Some(c) = drag.color()
    {
        let link = match drag {
            PanelDrag::Paint { params, .. } => dropped_link(app, params),
            _ => None,
        };
        let (on, offset) = match *place {
            StopDrop::OnStop(i) => (Some(i), 0.0),
            StopDrop::At(at) => (None, at),
            _ => return,
        };
        let (v, i) = drop_color(&g.gradient, on, offset, c, link);
        set_stops_keeping_models(app, &v, Some(i), Live::Released);
    }
}

/// The stop's eyedropper: the Eyedropper tool samples the next colour clicked on the art into the
/// selected stop, then hands back to the current tool (or, while a pick is already armed, to the
/// tool that armed it).
pub(crate) fn stop_eyedropper(app: &mut VectorcraftApp) {
    let tool = app.session.tool_id();
    let back = (tool == "eyedropper").then(|| app.session.tool_returns_to()).flatten().unwrap_or_else(|| tool.to_string());
    app.select_tool("eyedropper");
    app.session.set_tool_option("stop", &json!(back));
}

/// Open the stop popover for stop `i` at screen position `at`, the stop selected first.
fn open_popover(app: &mut VectorcraftApp, i: usize, at: Pos2) {
    select_stop(app, Some(i));
    gradient_stop::open_at(app, i, Anchor::Screen(at));
}

/// The panel menu's rows, in the reference panel's order; each row is a `ui.gradient.*` command
/// whose label and enabled state [`menu_states`] gives and which [`run`] runs.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let row = |id: &str| PanelRow::Command(CommandRef::new(id));
    vec![
        row(MENU_OPTIONS),
        PanelRow::Separator,
        row(MENU_ADD_TO_SWATCHES),
        row(MENU_REVERSE),
        row(MENU_RESET),
        PanelRow::Separator,
        row(MENU_PRESETS),
        row(MENU_SAVE_PRESET),
        row(MENU_DELETE_PRESET),
    ]
}

/// The panel menu's labels and enabled states: Show/Hide Options and Show/Hide Presets name what
/// a click does; Add to Swatches and Reverse need a gradient behind the active proxy, and Delete
/// Gradient Preset a saved preset it is.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let has = current(app).is_some();
    let toggle = |key: &str, show: &str, hide: &str| CommandState::new().title(if part_hidden(app, key) { show } else { hide });
    states.set(MENU_OPTIONS, toggle(HIDE_OPTIONS, tl!("Show Options"), tl!("Hide Options")));
    states.set(MENU_ADD_TO_SWATCHES, CommandState::new().title(tl!("Add to Swatches")).enabled(has));
    states.set(MENU_REVERSE, CommandState::new().title(tl!("Reverse Gradient")).enabled(has));
    states.set(MENU_RESET, CommandState::new().title(tl!("Reset to White, Black")));
    states.set(MENU_PRESETS, toggle(HIDE_PRESETS, tl!("Show Presets"), tl!("Hide Presets")));
    let save = crate::menus::enabled(app, "gradient.presets.save");
    states.set(MENU_SAVE_PRESET, CommandState::new().title(tl!("Save Gradient Preset…")).enabled(save));
    let delete = app.session.active_saved_gradient_preset().is_some();
    states.set(MENU_DELETE_PRESET, CommandState::new().title(tl!("Delete Gradient Preset")).enabled(delete));
}

/// Run the panel menu's row `id`; `None` for an id that is not this panel's.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    match id {
        MENU_OPTIONS | MENU_PRESETS => {
            let key = if id == MENU_OPTIONS { HIDE_OPTIONS } else { HIDE_PRESETS };
            let hide = !part_hidden(app, key);
            set_part_hidden(app, key, hide);
        }
        MENU_ADD_TO_SWATCHES => {
            if let Some(g) = current(app) {
                save_to_swatches(app, &g);
            }
        }
        MENU_REVERSE => edit(app, json!({"reverse": true}), Live::Released),
        MENU_RESET => set_stops(app, &Gradient::default().stops, None, Live::Released),
        MENU_SAVE_PRESET => {
            let name = app.session.next_gradient_preset_name();
            let p = json!({"command": "gradient.presets.save", "label": "Gradient Preset", "params": {"name": name}});
            return Some(app.run("ui.paramDialog", p));
        }
        // Deletes the saved preset the gradient behind the active proxy is.
        MENU_DELETE_PRESET => return Some(app.run("gradient.presets.delete", json!({}))),
        _ => return None,
    }
    Some(Ok(Value::Null))
}

#[cfg(test)]
mod tests;
