//! Transform panel: reference point, X/Y/W/H with the constrain link, rotate and shear, flips,
//! live-shape properties and the Scale Corners / Scale Strokes & Effects / Align to Pixel Grid
//! options. While the Artboard tool is in use it shows and edits the active artboard's X/Y/W/H
//! instead (a new W or H resizes every selected artboard), and with only ruler guides selected
//! their X (vertical guides) and Y (horizontal ones).

use std::ops::RangeInclusive;

use craft::behavior::IconId;
use craft::behavior::Mixed;
use craft::behavior::geometry::BoxState;
use craft::draw::ui::transform::{
    FieldArrangement, SizeUnit, TransformField, TransformFields, TransformFieldsOutput, TransformGroups, TransformRow, TransformSource,
};
use craft::keymap::CommandRef;
use craft::ui::buttons::{IconButton, Link};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, LabelPress, Props, Show, ShowContainer, SideLabel};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook};
use craft::ui::popover::Popover;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::text::{NumberField, NumberOutput, NumberPick, StepArrows};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeKind, OrientedBox};
use vectorcraft_geom::{Point, Rect, Vec2};

use super::first_selected;
use crate::VectorcraftApp;
use crate::caches::PixelAligned;
use crate::canvas::rulers::{general_lengths, general_rulers};
use crate::edits::{EditPath, apply_edits, apply_event};
use crate::fields::stops_stepper;
use crate::panel_docking::visible;
use crate::panels::{Live, part_hidden, set_part_hidden};

mod shape;
pub(crate) use shape::live_shape_rows;

pub const ANGLE_PRESETS: [f64; 9] = [-180.0, -135.0, -90.0, -45.0, 0.0, 45.0, 90.0, 135.0, 180.0];
/// The shear angle's presets: steep enough to be useful, short of the degenerate ±90°.
pub const SHEAR_PRESETS: [f64; 9] = [-60.0, -45.0, -30.0, -15.0, 0.0, 15.0, 30.0, 45.0, 60.0];

/// The panel's part key whose Hide Options flag hides the option checkboxes.
const HIDE_OPTIONS_KEY: &str = "xf-hide-options";

const HIDE_OPTIONS: &str = "ui.transform.hideOptions";
const SHOW_ON_SHAPE_CREATION: &str = "ui.transform.showOnShapeCreation";
const FLIP_HORIZONTAL: &str = "ui.transform.flipHorizontal";
const FLIP_VERTICAL: &str = "ui.transform.flipVertical";
const SCALE_STROKES: &str = "ui.transform.scaleStrokes";
const TYPE_GLYPH_BOUNDS: &str = "ui.transform.typeGlyphBounds";
const ALIGN_NEW_OBJECTS: &str = "ui.transform.alignNewObjects";
const REGISTRATION_POINT: &str = "ui.transform.registrationPoint";

/// What the Transform panel's reference point and popover remember while the app runs.
#[derive(Clone, Debug)]
pub struct TransformUi {
    /// The shared reference point (0 to 8): the Transform panel, the Control bar, the Artboards bar and Artboard Options
    /// read and set it.
    pub reference_point: usize,
    /// Whether the Transform popover under the Control bar's link is open.
    pub popover: bool,
    /// The popover closed at the press of the click still running: a click on an X:, Y:, W: or H: caption that
    /// closed it at its press leaves it closed.
    closed_by_press: bool,
    /// The point a field's running gesture turns or shears about ([`gesture_origin`]).
    gesture_origin: Value,
    /// The artboards' rectangles when a field's running gesture began ([`live_artboard`]).
    gesture_boards: Vec<Rect>,
}

impl Default for TransformUi {
    fn default() -> Self {
        TransformUi { reference_point: 4, popover: false, closed_by_press: false, gesture_origin: Value::Null, gesture_boards: Vec::new() }
    }
}

/// The nine points of a box in the order of the reference point index (row by row from the top left).
const ANCHORS: [Anchor9; 9] = [
    Anchor9::TopLeft,
    Anchor9::Top,
    Anchor9::TopRight,
    Anchor9::Left,
    Anchor9::Centre,
    Anchor9::Right,
    Anchor9::BottomLeft,
    Anchor9::Bottom,
    Anchor9::BottomRight,
];

/// The reference point index of `anchor`.
fn anchor_index(anchor: Anchor9) -> usize {
    ANCHORS.iter().position(|a| *a == anchor).unwrap_or(4)
}

/// The point `index` (0 to 8) names; the centre for an index outside the grid.
fn anchor_at(index: usize) -> Anchor9 {
    ANCHORS.get(index).copied().unwrap_or(Anchor9::Centre)
}

/// Proportional size: the other dimension when one changes with the link on.
pub fn constrained(w: f64, h: f64, new_w: Option<f64>, new_h: Option<f64>) -> (f64, f64) {
    match (new_w, new_h) {
        (Some(nw), _) if w.abs() > 1e-9 => (nw, h * nw / w),
        (_, Some(nh)) if h.abs() > 1e-9 => (w * nh / h, nh),
        (a, b) => (a.unwrap_or(w), b.unwrap_or(h)),
    }
}

/// The artboard the Artboard tool has active, while that tool is in use.
fn tool_artboard(app: &VectorcraftApp) -> Option<(usize, Rect)> {
    if app.session.tool_id() != "artboard" {
        return None;
    }
    let i = usize::try_from(app.session.tool_options().get("active")?.as_u64()?).ok()?;
    app.session.active()?.doc.artboards.get(i).map(|a| (i, a.rect))
}

/// Where an artboard goes when the panel sets its X, Y, W or H: the reference point `refi` moves to
/// the new X/Y or stays put while the size changes (both sides together when `link` is on).
pub fn artboard_rect(r: Rect, refi: usize, x: Option<f64>, y: Option<f64>, w: Option<f64>, h: Option<f64>, link: bool) -> Rect {
    let rp = vectorcraft_geom::reference_point(r, refi);
    let (w0, h0) = (r.width(), r.height());
    let (nw, nh) = if link { constrained(w0, h0, w, h) } else { (w.unwrap_or(w0), h.unwrap_or(h0)) };
    // Where the reference point sits across the box (0, ½ or 1 of each side).
    let fx = if w0 > 1e-9 { (rp.x - r.x0) / w0 } else { 0.0 };
    let fy = if h0 > 1e-9 { (rp.y - r.y0) / h0 } else { 0.0 };
    let (x0, y0) = (x.unwrap_or(rp.x) - fx * nw, y.unwrap_or(rp.y) - fy * nh);
    Rect::new(x0, y0, x0 + nw, y0 + nh)
}

/// Where the Transform panel and the Properties panel's Transform section measure the selection
/// from and turn, shear and flip it about ([`panel_reference`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelReference {
    /// The reference point (0 to 8) of the selection's box that `object.setBounds` keeps in place
    /// or moves to a new X or Y, and that the locator shows.
    pub index: usize,
    /// The point on the page: that reference point, or a symbol instance's registration point.
    pub point: Point,
    /// `point` less the box's reference point `index` (zero unless the registration point lies on
    /// none of the box's nine points, as on a sheared instance).
    shift: Vec2,
}

impl PanelReference {
    /// The `x` and `y` of `object.setBounds` that put [`Self::point`] at `x`, `y` on the page.
    pub fn bounds_xy(&self, x: f64, y: f64) -> (f64, f64) {
        (x - self.shift.x, y - self.shift.y)
    }

    /// The `origin` param of the rotations, shears and reflections: [`Self::point`].
    pub fn origin(&self) -> serde_json::Value {
        json!([self.point.x, self.point.y])
    }
}

/// The reference of box `bx` of the selection for the Transform panel and the Properties panel's
/// Transform section: the shared reference point (`app.ui.transform_panel`); with Use Registration
/// Point for Symbol on and one symbol instance selected, its symbol's registration point (Symbol
/// Options › Registration) where the instance's transform puts it, with the box's reference point
/// nearest it: after a flip the art's top-left corner is the box's top-right one, after a rotation
/// the box turns with the art.
pub fn panel_reference(app: &VectorcraftApp, bx: &OrientedBox) -> PanelReference {
    let Some(q) = symbol_registration(app) else {
        let index = app.ui.transform_panel.reference_point;
        return PanelReference { index, point: bx.reference_point(index), shift: Vec2::ZERO };
    };
    // The nearest of the box's nine points, measured in the box's own (turned) frame.
    let (local, r) = (bx.to_local(q), bx.rect);
    let third = |v: f64, start: f64, len: f64| if len > 1e-9 { ((v - start) / len * 2.0).round().clamp(0.0, 2.0) as usize } else { 1 };
    let index = third(local.y, r.y0, r.height()) * 3 + third(local.x, r.x0, r.width());
    PanelReference { index, point: q, shift: q - bx.reference_point(index) }
}

/// The page point the selection's reference is at, as the `origin` param of the rotations, shears and
/// reflections; `Null` with nothing selected.
pub(crate) fn reference_origin(app: &mut VectorcraftApp) -> Value {
    app.selection_box().map_or(Value::Null, |b| panel_reference(app, &b).origin())
}

/// [`reference_origin`] for a value of a field's gesture: where it was when the gesture began, since the gesture's
/// previews move the box it is measured on.
fn gesture_origin(app: &mut VectorcraftApp) -> Value {
    if !app.session.in_interaction() {
        app.ui.transform_panel.gesture_origin = reference_origin(app);
    }
    app.ui.transform_panel.gesture_origin.clone()
}

/// Where the registration point of the one selected symbol instance is on the page, while Use
/// Registration Point for Symbol is on: the point of the symbol's art box on the 9-point grid,
/// placed by the instance's transform.
fn symbol_registration(app: &VectorcraftApp) -> Option<Point> {
    if !app.session.prefs.use_registration_point_for_symbol {
        return None;
    }
    let st = app.session.active()?;
    let [id] = st.selection.objects.as_slice() else { return None };
    let NodeKind::SymbolInstance { symbol, xf } = &st.doc.node(*id)?.kind else { return None };
    let own = Node::new(*id, NodeKind::SymbolInstance { symbol: symbol.clone(), xf: vectorcraft_geom::Affine::IDENTITY });
    let art = own.geometric_bounds_with(&st.doc.symbol_boxes())?;
    Some(*xf * vectorcraft_geom::reference_point(art, usize::from(st.doc.symbol_registration(symbol).0)))
}

/// The Transform panel menu's choices for what the panel transforms: the value of the
/// `transformPanelPatterns` preference each sets, and the id of its menu row.
const PATTERN_MODES: [(&str, &str); 3] =
    [("object", "ui.transform.objectOnly"), ("pattern", "ui.transform.patternOnly"), ("both", "ui.transform.both")];

/// Run transform command `cmd` from the Transform panel or the Properties panel's Transform
/// section with `params` and the `objects` and `patterns` switches of the panel menu's Transform
/// Object Only (the object; its pattern fills and strokes stay as they are), Transform Pattern
/// Only (the pattern tiles alone) or Transform Both. Until one is chosen (`default`, or any other
/// value) neither is passed: patterns follow General › Transform Pattern Tiles. → the command's result.
fn try_transform(app: &mut VectorcraftApp, cmd: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let params = with_pattern_switches(app, params);
    app.run(cmd, params)
}

/// [`try_transform`] for a value of a field's gesture in its phase `live`: each value previews the transform from where
/// the gesture began, and the release keeps the last as one undo step. A typed value previews as it is typed and applies
/// again with its Enter's modifiers (Alt: to a copy), and a relative transform (a shear, a pattern-only move) applies
/// once. An error shows in the status bar.
fn live_transform(app: &mut VectorcraftApp, cmd: &str, params: serde_json::Value, live: Live) {
    let params = with_pattern_switches(app, params);
    let label = vectorcraft_engine::find_command(cmd).map_or(cmd, |spec| spec.label);
    super::live_run(app, label, cmd, params, live);
}

/// `params` with the `objects` and `patterns` switches [`try_transform`] adds.
pub fn with_pattern_switches(app: &VectorcraftApp, mut params: serde_json::Value) -> serde_json::Value {
    let switches = match app.session.prefs.transform_panel_patterns.as_str() {
        "object" => Some((true, false)),
        "pattern" => Some((false, true)),
        "both" => Some((true, true)),
        _ => None,
    };
    if let Some((objects, patterns)) = switches {
        params["objects"] = json!(objects);
        params["patterns"] = json!(patterns);
    }
    params
}

/// The panel menu's checked choice of [`PATTERN_MODES`]: the one chosen, else (`default`) what
/// General › Transform Pattern Tiles makes the panel do, Transform Both when it is on and
/// Transform Object Only when it is off.
fn pattern_mode(app: &VectorcraftApp) -> &'static str {
    let prefs = &app.session.prefs;
    PATTERN_MODES.iter().map(|(v, _)| *v).find(|v| *v == prefs.transform_panel_patterns).unwrap_or(if prefs.transform_pattern_tiles {
        "both"
    } else {
        "object"
    })
}

/// Set artboard `index` to a new X, Y, W or H from reference point `refi` ([`artboard_rect`]) with
/// `artboard.setProps`, passing the Artboard tool's Move and Scale Artwork with Artboard options as
/// `moveArt` and `scaleArt`; an error is shown in the status bar. A new X or Y moves artboard
/// `index` alone; a new W or H resizes every selected artboard about its own reference point
/// ([`crate::panels::artboards::resized_with`]), in one undo step.
pub fn set_artboard(app: &mut VectorcraftApp, index: usize, refi: usize, xywh: [Option<f64>; 4], link: bool) {
    let rects: Vec<Rect> = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.rect).collect()).unwrap_or_default();
    let Some(p) = artboard_props(app, index, refi, xywh, link, &rects) else { return };
    if let Err(e) = app.run("artboard.setProps", p) {
        app.status(e);
    }
}

/// [`set_artboard`] for a value of a field's gesture in its phase `live`: each value previews from the artboards as the
/// gesture began, and the release keeps the last as one undo step, so Cmd+Enter keeps the proportions the artboard had
/// before the typed value previewed.
fn live_artboard(app: &mut VectorcraftApp, index: usize, refi: usize, xywh: [Option<f64>; 4], link: bool, live: Live) {
    if !app.session.in_interaction() {
        app.ui.transform_panel.gesture_boards = app.session.active().map(|d| d.doc.artboards.iter().map(|a| a.rect).collect()).unwrap_or_default();
    }
    let rects = app.ui.transform_panel.gesture_boards.clone();
    if let Some(p) = artboard_props(app, index, refi, xywh, link, &rects) {
        const CMD: &str = "artboard.setProps";
        let label = vectorcraft_engine::find_command(CMD).map_or(CMD, |spec| spec.label);
        super::live_run(app, label, CMD, p, live);
    }
}

/// The `artboard.setProps` params of [`set_artboard`] for the artboards at `rects`; `None` when none changes.
fn artboard_props(app: &VectorcraftApp, index: usize, refi: usize, xywh: [Option<f64>; 4], link: bool, rects: &[Rect]) -> Option<Value> {
    let [x, y, w, h] = xywh;
    let indices = if x.is_none() && y.is_none() { crate::panels::artboards::resized_with(app, index) } else { vec![index] };
    let common = json!({"scaleArt": crate::panels::artboards::scale_art(app), "moveArt": crate::panels::artboards::move_art(app)});
    let entries = indices
        .iter()
        .filter_map(|&i| {
            let n = artboard_rect(*rects.get(i)?, refi, x, y, w, h, link);
            Some(json!({"index": i, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height()}))
        })
        .collect();
    crate::panels::artboards::set_props_params(common, entries)
}

/// How an edit of the fields was made: the link or Cmd+Enter keeps the proportions, Alt+Enter copies.
#[derive(Clone, Copy)]
struct Intent {
    proportional: bool,
    copy: bool,
}

/// The groups of the panel's form: the reference point, X and Y, W and H with the link.
fn panel_groups() -> TransformGroups {
    let mut groups = TransformGroups::NONE;
    groups.reference = true;
    groups.position = true;
    groups.size = true;
    groups
}

/// The corners of `bx` on the page, clockwise from the top left, with the reference as the pivot.
fn box_state(app: &VectorcraftApp, bx: &OrientedBox) -> BoxState {
    BoxState::new([bx.reference_point(0), bx.reference_point(2), bx.reference_point(8), bx.reference_point(6)], panel_reference(app, bx).point)
}

/// The transform fields over `state` (none: every field blank and disabled) measured in the document's general unit.
fn fields<'a>(app: &VectorcraftApp, state: Option<BoxState>, start: Rect, groups: TransformGroups, bar: bool) -> TransformFields<'a> {
    let rulers = general_rulers(app);
    let size = SizeUnit::length(rulers.x, rulers.y);
    let has = state.is_some();
    let row = state.map_or_else(Bind::kept, |s| Bind::given(TransformRow::new(s, 0.0)));
    let fields = TransformFields::new(tl!("Transform"), row, start, Bind::given(app.session.prefs.constrain_proportions))
        .rulers(rulers)
        .size_unit(size)
        .reference_look(ReferencePointLook::Squares)
        .groups(groups)
        .enabled(has);
    if bar { fields.bar() } else { fields.arrangement(FieldArrangement::PositionFirst) }
}

/// The width and height of the box a row reports, along its own sides.
fn sides(row: &TransformRow) -> (f64, f64) {
    let q = row.state.quad;
    ((q[1] - q[0]).hypot(), (q[2] - q[1]).hypot())
}

/// Makes the point the row's pivot sits on the shared reference point.
fn pick_reference(app: &mut VectorcraftApp, row: &TransformRow) {
    if let Some(anchor) = row.state.anchor_of_pivot() {
        app.ui.transform_panel.reference_point = anchor_index(anchor);
    }
}

/// Applies what the fields reported along `path`, one undo step per gesture: `run` gets each edit with the field it came
/// from, how it was made and its phase. A changed link is saved first.
fn apply_fields(
    app: &mut VectorcraftApp,
    out: &TransformFieldsOutput,
    path: EditPath,
    mut run: impl FnMut(&mut VectorcraftApp, TransformSource, &TransformRow, Intent, Live),
) {
    if let Some(on) = out.linked {
        set_pref(app, "constrainProportions", on);
    }
    let linked = app.session.prefs.constrain_proportions;
    let no_anchors = app.session.active().is_some_and(|st| st.selection.anchors.is_empty());
    for (event, source) in out.events.iter().zip(&out.sources) {
        let source = *source;
        let entered = out.submitted.filter(|(field, _)| TransformSource::Field(*field) == source);
        let intent = Intent {
            proportional: linked || entered.is_some_and(|(field, mods)| mods.cmd && matches!(field, TransformField::W | TransformField::H)),
            copy: entered.is_some_and(|(_, mods)| mods.alt) && no_anchors,
        };
        apply_event(app, event, &path, &mut |app, row, live| run(app, source, row, intent, live));
    }
}

/// `object.setBounds` for an edit of X, Y, W or H of the selection's fields, in its phase `live`.
fn set_object_bounds(app: &mut VectorcraftApp, source: TransformSource, row: &TransformRow, intent: Intent, live: Live) {
    let Some(bx) = app.selection_box() else { return };
    let reference = panel_reference(app, &bx);
    let (w, h) = sides(row);
    let p = row.state.pivot;
    let (key, value, sized) = match source {
        TransformSource::Reference => return pick_reference(app, row),
        // `object.setBounds`'s x and y put the reference (the registration point) at the typed place.
        TransformSource::Field(TransformField::X) => ("x", reference.bounds_xy(p.x, 0.0).0, false),
        TransformSource::Field(TransformField::Y) => ("y", reference.bounds_xy(0.0, p.y).1, false),
        TransformSource::Field(TransformField::W) => ("width", w, true),
        TransformSource::Field(TransformField::H) => ("height", h, true),
        _ => return,
    };
    let mut params = json!({"reference": reference.index, "copy": intent.copy, key: value});
    if sized {
        params["proportional"] = json!(intent.proportional);
    }
    live_transform(app, "object.setBounds", params, live);
}

/// The panel's fields of the selection's bounding box, rotated with rotated objects: W/H are its own sides, X/Y its
/// reference point on the page. With nothing selected they are blank and disabled. `groups` picks the cells shown.
pub(crate) fn object_fields(app: &mut VectorcraftApp, ui: &mut Ui, groups: TransformGroups) {
    let bx = app.selection_box();
    let state = bx.as_ref().map(|b| box_state(app, b));
    let start = bx.map_or(Rect::ZERO, |b| b.rect);
    let out = fields(app, state, start, groups, false).show(ui);
    apply_fields(app, &out, EditPath::Preview, set_object_bounds);
}

/// The Control bar's form of the selection's fields; `None` with nothing selected. The output's `submitted` carries
/// the modifiers of the Enter that applied a value.
pub(crate) fn object_bar_fields(app: &mut VectorcraftApp, ui: &mut Ui) -> Option<TransformFieldsOutput> {
    let bx = app.selection_box()?;
    let state = box_state(app, &bx);
    let out = fields(app, Some(state), bx.rect, panel_groups(), true).caption_press(LabelPress::Link).show(ui);
    apply_fields(app, &out, EditPath::Preview, set_object_bounds);
    // A click on a caption opens the Transform popover as the Transform link does, or closes it.
    if out.link.is_some() {
        let panel = &mut app.ui.transform_panel;
        panel.popover = !panel.closed_by_press;
    }
    Some(out)
}

/// A field of an angle: its icon as a scrubbing label, the value in degrees and the presets on a ladder of stops.
fn angle_field(
    ui: &mut Ui,
    label: &'static str,
    icon: &'static str,
    shown: f64,
    range: RangeInclusive<f64>,
    presets: &[f64],
    enabled: bool,
) -> Option<NumberOutput> {
    let entries: Vec<Entry<NumberPick>> = presets.iter().map(|v| Entry::new(NumberPick::Number(*v), format!("{v}°"))).collect();
    Field::new(label)
        .side(SideLabel::new().icon(IconId::new(icon)).scrub(true))
        .enabled(enabled)
        .show(ui, |ui, _| {
            NumberField::new(label, Bind::given(Some(shown)))
                .unit("°")
                .commit_decimals(2)
                .range(range.clone())
                .step_arrows(StepArrows::Leading)
                .presets(&entries)
                .stepper(stops_stepper(15.0, presets, |v| format!("{v:.2}")))
                .show(ui)
        })
        .inner
}

/// Rotate: the bounding box's angle, and a new value turns the selection to it about the reference point
/// (Alt+Enter turns a copy).
pub(crate) fn rotate_field(app: &mut VectorcraftApp, ui: &mut Ui) {
    let bx = app.selection_box();
    let angle = bx.as_ref().map_or(0.0, |b| b.angle);
    let Some(out) = angle_field(ui, tl!("Rotate"), "rotate-ccw", angle, -360.0..=f64::INFINITY, &ANGLE_PRESETS, bx.is_some()) else { return };
    let copy = out.edit.submit_mods.alt && app.session.active().is_some_and(|st| st.selection.anchors.is_empty());
    apply_edits(app, &out.edit, EditPath::Preview, |app, v, live| {
        // The angle shown is the preview's while the gesture runs: its release keeps it.
        if let Some(a) = *v
            && (a != angle || app.session.in_interaction())
        {
            let origin = gesture_origin(app);
            live_transform(app, "object.rotate", json!({"angle": a, "absolute": true, "origin": origin, "copy": copy}), live);
        }
    });
}

/// Shear is relative: the field rests at 0° and a new value from -359° to 359° shears by it. The field has no minimum,
/// so `object.shear` checks every value: it refuses ±90°, ±270° and a value outside -359° to 359°, and the status bar
/// shows its error.
fn shear_field(app: &mut VectorcraftApp, ui: &mut Ui) {
    let has = app.selection_box().is_some();
    let Some(out) = angle_field(ui, tl!("Shear"), "dc-shear", 0.0, f64::NEG_INFINITY..=f64::INFINITY, &SHEAR_PRESETS, has) else { return };
    let copy = out.edit.submit_mods.alt && app.session.active().is_some_and(|st| st.selection.anchors.is_empty());
    apply_edits(app, &out.edit, EditPath::Preview, |app, v, live| {
        if let Some(a) = *v
            && a != 0.0
        {
            let origin = gesture_origin(app);
            live_transform(app, "object.shear", json!({"angle": a, "axis": "horizontal", "origin": origin, "copy": copy}), live);
        }
    });
}

/// The reference point picker alone, for forms that have no transform fields (Artboard Options).
pub(crate) fn reference_picker(app: &mut VectorcraftApp, ui: &mut Ui) {
    let at = anchor_at(app.ui.transform_panel.reference_point);
    let out = ReferencePoint::new(tl!("Reference Point"), Bind::given(Some(at))).look(ReferencePointLook::Squares).show(ui);
    apply_edits(app, &out, EditPath::Scope, |app, anchor, _| {
        if let Some(anchor) = *anchor {
            app.ui.transform_panel.reference_point = anchor_index(anchor);
        }
    });
}

/// The fields of artboard `index` at `r` about the shared reference point, in the panel's form or the bar's.
fn artboard_form(app: &mut VectorcraftApp, ui: &mut Ui, index: usize, r: Rect, bar: bool) -> TransformFieldsOutput {
    let refi = app.ui.transform_panel.reference_point;
    let mut state = BoxState::from_rect(r);
    state.pivot = vectorcraft_geom::reference_point(r, refi);
    let out = fields(app, Some(state), r, panel_groups(), bar).show(ui);
    apply_fields(app, &out, EditPath::Preview, |app, source, row, intent, live| {
        let (w, h) = sides(row);
        let p = row.state.pivot;
        let xywh = match source {
            TransformSource::Reference => return pick_reference(app, row),
            TransformSource::Field(TransformField::X) => [Some(p.x), None, None, None],
            TransformSource::Field(TransformField::Y) => [None, Some(p.y), None, None],
            TransformSource::Field(TransformField::W) => [None, None, Some(w), None],
            TransformSource::Field(TransformField::H) => [None, None, None, Some(h)],
            _ => return,
        };
        live_artboard(app, index, refi, xywh, intent.proportional, live);
    });
    out
}

/// The Artboard tool's view of the panel, and the Properties panel's Transform section in Edit Artboards: the reference
/// point, artboard `index`'s position and size at `r` ([`set_artboard`]), and the Constrain Width and Height
/// Proportions link.
pub(crate) fn artboard_fields(app: &mut VectorcraftApp, ui: &mut Ui, index: usize, r: Rect) {
    artboard_form(app, ui, index, r, false);
}

/// The Control bar's form of [`artboard_fields`].
pub(crate) fn artboard_bar_fields(app: &mut VectorcraftApp, ui: &mut Ui, index: usize, r: Rect) -> TransformFieldsOutput {
    artboard_form(app, ui, index, r, true)
}

/// With ruler guides selected and no art: the X of the vertical ones and the Y of the horizontal
/// ones (the least of each; none where no such guide is selected).
pub(crate) fn selected_guides(app: &VectorcraftApp) -> Option<(Option<f64>, Option<f64>)> {
    let st = app.session.active()?;
    if !st.selection.objects.is_empty() || st.selection.guides.is_empty() {
        return None;
    }
    let least = |vertical: bool| {
        st.selection.guides.iter().filter_map(|i| st.doc.ruler_guide(*i)).filter(|g| g.vertical == vertical).map(|g| g.pos).min_by(f64::total_cmp)
    };
    Some((least(true), least(false)))
}

/// The selected ruler guides' view of the panel (#1597), also the Properties panel's Transform
/// section for them: X moves the vertical ones and Y the horizontal ones, together.
pub(crate) fn guide_fields(app: &mut VectorcraftApp, ui: &mut Ui, (x, y): (Option<f64>, Option<f64>)) {
    let lengths = general_lengths(app);
    let origin = app.ruler_origin();
    for (label, pos, zero, by, vertical) in [("X", x, origin.x, "dx", true), ("Y", y, origin.y, "dy", false)] {
        let edit = Field::new(tl!(label))
            .enabled(pos.is_some())
            .show(ui, |ui, _| NumberField::length(tl!(label), Bind::given(pos.map(|p| p - zero)), lengths).show(ui).edit)
            .inner;
        let Some(edit) = edit else { continue };
        apply_edits(app, &edit, EditPath::Scope, |app, v, _| {
            // The guide's place now: every step of a gesture moves it from where the document has it.
            let now = selected_guides(app).and_then(|(x, y)| if vertical { x } else { y });
            if let (Some(v), Some(now)) = (*v, now)
                && let Err(e) = app.run("guide.move", json!({ by: v + zero - now }))
            {
                app.status(e);
            }
        });
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        Text::new(tl!("No document")).shade(Shade::Muted).show(ui);
        return;
    }
    if let Some((i, r)) = tool_artboard(app) {
        artboard_fields(app, ui, i, r);
        return;
    }
    if let Some(guides) = selected_guides(app) {
        guide_fields(app, ui, guides);
        return;
    }
    object_fields(app, ui, panel_groups());
    Row::new().show(ui, |ui, _| {
        rotate_field(app, ui);
        shear_field(app, ui);
    });
    // Lucide's flip icons are named for their mirror line: flip-vertical-2 draws a vertical
    // line (the left-right flip), flip-horizontal-2 a horizontal one (top-bottom).
    Row::new().show(ui, |ui, _| {
        IconButton::command(IconId::new("flip-vertical-2"), FLIP_HORIZONTAL).show(ui);
        IconButton::command(IconId::new("flip-horizontal-2"), FLIP_VERTICAL).show(ui);
    });
    // Live shape properties (a path with live corners is no shape).
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
        && !matches!(**live, LiveShape::Path { .. })
        && !live_shape_rows(app, ui, &n, live, true)
    {
        Text::new(tl!("Shape Properties:")).show(ui);
        Text::new(tl!(n.kind_label())).shade(Shade::Muted).show(ui);
    }
    if part_hidden(app, HIDE_OPTIONS_KEY) {
        return;
    }
    let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
    option_check(app, ui, tl!("Scale Corners"), "scaleCorners", sc);
    option_check(app, ui, tl!("Scale Strokes & Effects"), "scaleStrokes", ss);
    pixel_grid_check(app, ui);
}

/// A check box of the boolean preference `key`, shown as `on`.
fn option_check(app: &mut VectorcraftApp, ui: &mut Ui, label: &'static str, key: &str, on: bool) {
    let out = Checkbox::new(Bind::given(on)).label(label).show(ui);
    apply_edits(app, &out, EditPath::Scope, |app, v, _| set_pref(app, key, *v));
}

/// The selected objects' Align to Pixel Grid: on or off when they agree, `None` when they differ,
/// with whether every one can be aligned (a path that is not a guide and not inside a compound path,
/// or a compound path). `None` with nothing selected.
fn pixel_aligned_state(app: &VectorcraftApp) -> Option<PixelAligned> {
    let st = app.session.active()?;
    let nodes: Vec<&Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let first = nodes.first()?.extras().pixel_aligned;
    let value = nodes.iter().all(|n| n.extras().pixel_aligned == first).then_some(first);
    let in_compound = |n: &Node| st.doc.parent_of(n.id).and_then(|p| st.doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
    let alignable = nodes.iter().all(|n| matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) && !in_compound(n));
    Some((value, alignable))
}

/// [`pixel_aligned_state`], measured once per (document uid, revision)
/// ([`crate::caches::CanvasCache::pixel_aligned`]): the panel reads it every frame, and Select All
/// can select hundreds of thousands of paths.
fn pixel_aligned_shown(app: &mut VectorcraftApp) -> Option<PixelAligned> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision);
    if let Some((k, state)) = app.canvas.pixel_aligned
        && k == key
    {
        return state;
    }
    let state = pixel_aligned_state(app);
    app.canvas.pixel_aligned = Some((key, state));
    state
}

/// The Align to Pixel Grid checkbox (`object.setPixelAligned`): a dash when the selected objects
/// differ, and unavailable unless every selected object can be aligned. A click turns it on for
/// them all, or off when it is on for them all.
fn pixel_grid_check(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (value, alignable) = pixel_aligned_shown(app).unwrap_or((Some(false), false));
    let shown = value.map_or(Mixed::Mixed, Mixed::Same);
    let out = Checkbox::new(Bind::given(shown)).label(tl!("Align to Pixel Grid")).enabled(alignable).show(ui);
    if out.events.iter().next().is_none() {
        return;
    }
    let ids = pixel_aligned_ids(app);
    if let Err(e) = app.run("object.setPixelAligned", json!({"ids": ids, "on": value != Some(true)})) {
        app.status(e);
    }
}

/// The ids of the selected objects that are in the document, for `object.setPixelAligned`: read
/// when the Align to Pixel Grid checkbox is clicked.
fn pixel_aligned_ids(app: &VectorcraftApp) -> Vec<u64> {
    app.session.active().map(|st| st.selection.objects.iter().filter(|id| st.doc.node(**id).is_some()).map(|id| id.0).collect()).unwrap_or_default()
}

/// The Control bar's underlined "Transform" link: a click toggles the Transform panel in a
/// popover under it, as the Stroke link does with the Stroke panel.
pub(crate) fn link(app: &mut VectorcraftApp, ui: &mut Ui) {
    let out = Link::new(tl!("Transform")).tooltip(tl!("Transform options")).show(ui);
    popover(app, ui, &out.response);
}

/// The Transform panel in a popover under `opener`, which a click on it toggles: the Control bar's Transform [`link`],
/// and its X:, Y:, W: and H: labels.
pub(crate) fn popover(app: &mut VectorcraftApp, ui: &mut Ui, opener: &egui::Response) {
    // The popover's body edits the app, so the open flag is held apart from it for the pass.
    let mut open = app.ui.transform_panel.popover;
    if opener.clicked() {
        open = !open;
    }
    let shown = Popover::new(tl!("Transform"), Bind::bound(&mut open)).opened_by(opener).show(ui, |ui, _| show(app, ui));
    let clicking = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
    let panel = &mut app.ui.transform_panel;
    panel.closed_by_press = shown.output.closed == Some(CloseReason::Outside) || (panel.closed_by_press && clicking);
    panel.popover = open;
}

/// Set a boolean preference (through `prefs.set`, as agents do) and return the command's result.
fn pref(app: &mut VectorcraftApp, key: &str, value: Value) -> Result<Value, String> {
    app.run("prefs.set", json!({ "key": key, "value": value }))
}

/// Toggle a boolean preference (through `prefs.set`, as agents do).
pub fn set_pref(app: &mut VectorcraftApp, key: &str, on: bool) {
    if let Err(e) = pref(app, key, json!(on)) {
        app.status(e);
    }
}

/// Show on Shape Creation (`showOnShapeCreation`): after a shape was drawn
/// ([`vectorcraft_engine::Session::take_shape_drawn`]), the Transform panel shows as Window ›
/// Transform shows it, unless it can be seen already. In Presentation Mode, or with the option off,
/// the panels stay as they are. Called once per frame, after the canvas and the dialogs; a frame
/// that shows the panel asks `ctx` for another, which draws it.
pub fn show_on_shape_creation(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let drawn = app.session.take_shape_drawn();
    if !drawn || !app.session.prefs.show_on_shape_creation || app.ui.screen_mode >= 3 || visible(app, "transform") {
        return;
    }
    match app.run("window.panel", json!({ "panel": "transform" })) {
        Ok(_) => ctx.request_repaint(),
        Err(e) => app.status(e),
    }
}

/// The panel menu's rows: Hide Options, Show on Shape Creation, the flips, what the panel scales and measures, what it
/// transforms, and Use Registration Point for Symbol.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let row = |id: &str| PanelRow::Command(CommandRef::new(id));
    let mut rows = vec![
        row(HIDE_OPTIONS),
        row(SHOW_ON_SHAPE_CREATION),
        PanelRow::Separator,
        row(FLIP_HORIZONTAL),
        row(FLIP_VERTICAL),
        PanelRow::Separator,
        row(SCALE_STROKES),
        row(TYPE_GLYPH_BOUNDS),
        row(ALIGN_NEW_OBJECTS),
        PanelRow::Separator,
    ];
    rows.extend(PATTERN_MODES.iter().map(|(_, id)| row(id)));
    rows.push(PanelRow::Separator);
    rows.push(row(REGISTRATION_POINT));
    rows
}

/// The menu rows' checks, titles and enabled states for this frame.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let prefs = &app.session.prefs;
    let has = super::selection_len(app) > 0;
    let mut set = |id: &str, state: CommandState| states.set(CommandRef::new(id), state);
    let title = if part_hidden(app, HIDE_OPTIONS_KEY) { tl!("Show Options") } else { tl!("Hide Options") };
    set(HIDE_OPTIONS, CommandState::new().title(title));
    set(SHOW_ON_SHAPE_CREATION, CommandState::new().title(tl!("Show on Shape Creation")).checked(prefs.show_on_shape_creation));
    set(FLIP_HORIZONTAL, CommandState::new().title(tl!("Flip Horizontal")).enabled(has));
    set(FLIP_VERTICAL, CommandState::new().title(tl!("Flip Vertical")).enabled(has));
    set(SCALE_STROKES, CommandState::new().title(tl!("Scale Strokes & Effects")).checked(prefs.scale_strokes));
    set(TYPE_GLYPH_BOUNDS, CommandState::new().title(tl!("Measure Type by Glyph Bounds")).checked(prefs.type_glyph_bounds));
    let align_new = app.session.active().map(|st| st.doc.setup.align_new_objects_to_pixel_grid);
    set(
        ALIGN_NEW_OBJECTS,
        CommandState::new().title(tl!("Align New Objects to Pixel Grid")).enabled(align_new.is_some()).checked(align_new == Some(true)),
    );
    let current = pattern_mode(app);
    for ((value, id), title) in PATTERN_MODES.into_iter().zip([tl!("Transform Object Only"), tl!("Transform Pattern Only"), tl!("Transform Both")]) {
        set(id, CommandState::new().title(title).checked(value == current));
    }
    set(REGISTRATION_POINT, CommandState::new().title(tl!("Use Registration Point for Symbol")).checked(prefs.use_registration_point_for_symbol));
}

/// Runs the panel menu row `id`; `None` for an id that is not one of its rows. An error is also shown in the status bar.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let result = menu_command(app, id)?;
    if let Err(e) = &result {
        app.status(e.clone());
    }
    Some(result)
}

/// The command a menu row runs.
fn menu_command(app: &mut VectorcraftApp, id: &str) -> Option<Result<Value, String>> {
    if let Some((value, _)) = PATTERN_MODES.iter().find(|(_, row)| *row == id) {
        return Some(pref(app, "transformPanelPatterns", json!(value)));
    }
    let prefs = &app.session.prefs;
    let toggle = match id {
        SHOW_ON_SHAPE_CREATION => Some(("showOnShapeCreation", prefs.show_on_shape_creation)),
        SCALE_STROKES => Some(("scaleStrokes", prefs.scale_strokes)),
        TYPE_GLYPH_BOUNDS => Some(("typeGlyphBounds", prefs.type_glyph_bounds)),
        REGISTRATION_POINT => Some(("useRegistrationPointForSymbol", prefs.use_registration_point_for_symbol)),
        _ => None,
    };
    if let Some((key, on)) = toggle {
        return Some(pref(app, key, json!(!on)));
    }
    Some(match id {
        HIDE_OPTIONS => {
            let hidden = part_hidden(app, HIDE_OPTIONS_KEY);
            set_part_hidden(app, HIDE_OPTIONS_KEY, !hidden);
            Ok(Value::Null)
        }
        // The flips mirror across the panel's reference point, as its flip buttons do.
        FLIP_HORIZONTAL | FLIP_VERTICAL => {
            let axis = if id == FLIP_HORIZONTAL { "vertical" } else { "horizontal" };
            let origin = reference_origin(app);
            try_transform(app, "object.reflect", json!({"axis": axis, "origin": origin}))
        }
        ALIGN_NEW_OBJECTS => match app.session.active().map(|st| st.doc.setup.align_new_objects_to_pixel_grid) {
            Some(on) => app.run("document.setup", json!({"alignNewObjectsToPixelGrid": !on})),
            None => Err("No document".to_string()),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
