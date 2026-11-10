//! Parametric Shape Tool (Path Precision Tools): shapes that stay editable as numbers. A drag from
//! the centre draws one (`addon.pathtools.shape.create`): the drag's length is its size and its
//! direction its rotation (Shift: 45° steps); ↑/↓ change the kind's count or first value while
//! dragging, ←/→ its second value or corner radius. A click selects a parametric shape and shows
//! its handles (centre, size, rotation, corner radius and one of the kind's own), whose drags run
//! `addon.pathtools.shape.set`; a double-click on the canvas places one with the last settings.
//! Each gesture is one undo step.
//!
//! A parametric shape is an ordinary path that keeps its numbers in one entry of its object data
//! ([`KEY`]) with a fingerprint of the outline they drew. Moved, turned or scaled by any tool it
//! stays editable ([`ParametricShape::recover`]); once its anchors are edited it is a plain path.

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Vec2};
use vectorcraft_pathops::addons::pathtools::shapes::fingerprint;
pub use vectorcraft_pathops::addons::pathtools::shapes::{MAX_COUNT, MAX_SIZE, MIN_COUNT, MIN_SIZE, ParametricShape, ShapeKind};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "parametricShape";
/// The object data entry that holds a parametric shape's numbers (JSON). The `vc-` prefix marks it
/// as VectorCraft's own, which SVG export leaves out of the `data-*` attributes it writes.
pub const KEY: &str = "vc-pathtools-shape";
/// The longest entry read back (a damaged file's longer one is no shape).
const MAX_STORED: usize = 4096;
/// The undo steps' names.
pub const CREATE: &str = "Parametric Shape";
pub const EDIT: &str = "Edit Parametric Shape";
const BLUE: [u8; 3] = [0x4f, 0x9d, 0xff];
/// How far (px) the rotation handle stands above the shape.
const STEM_PX: f64 = 24.0;

/// `s` as commands take it and files keep it.
pub fn to_json(s: &ParametricShape) -> Value {
    json!({
        "kind": s.kind.id(), "center": [s.center.x, s.center.y], "width": s.width, "height": s.height,
        "rotation": s.rotation, "count": s.count, "a": s.a, "b": s.b, "radius": s.radius,
    })
}

/// `s` with the numbers `p` gives (as [`to_json`] writes them; any left out stay as they are). A
/// new `kind` comes first, with its own defaults for what `p` leaves out. Every number is checked:
/// the error says which is wrong.
pub fn apply(mut s: ParametricShape, p: &Value) -> Result<ParametricShape, String> {
    let given = |key: &str| p.get(key).filter(|v| !v.is_null());
    let num = |key: &str| -> Result<Option<f64>, String> {
        given(key).map(|v| v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| format!("`{key}` must be a number"))).transpose()
    };
    if let Some(k) = given("kind") {
        let kind = k.as_str().and_then(ShapeKind::parse);
        s = s.with_kind(kind.ok_or("`kind` must be one of: rectangle, ellipse, polygon, star, ring, arrow, cross, heart, gear, bubble")?);
    }
    if let Some(c) = given("center") {
        let xy = c.as_array().filter(|a| a.len() == 2).and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
        let (x, y) = xy.ok_or("`center` must be [x, y]")?;
        s.center = Point::new(x, y);
    }
    for (key, slot) in [("width", &mut s.width), ("height", &mut s.height), ("a", &mut s.a), ("b", &mut s.b), ("radius", &mut s.radius)] {
        if let Some(v) = num(key)? {
            *slot = v;
        }
    }
    if let Some(r) = num("rotation")? {
        s.rotation = vectorcraft_geom::normalize_deg(r);
    }
    if let Some(n) = num("count")? {
        if n.fract() != 0.0 || !(f64::from(MIN_COUNT)..=f64::from(MAX_COUNT)).contains(&n) {
            return Err("`count` must be a whole number from 3 to 1000".into());
        }
        s.count = n as u32;
    }
    s.check().map_err(|(key, lo, hi)| format!("`{key}` must be from {lo} to {hi}"))?;
    Ok(s)
}

/// What a parametric shape keeps in its object data ([`KEY`]): its numbers and the fingerprint of
/// `path`, the outline they drew.
pub fn stored(s: &ParametricShape, path: &PathData) -> String {
    let mut v = to_json(s);
    v["fp"] = json!(format!("{:016x}", fingerprint(path)));
    v.to_string()
}

/// The parametric shape object `n` is, if it is one: a path (not a guide) with a [`KEY`] entry
/// whose fingerprint is its outline's, or which it is moved, turned or scaled. `None` for any
/// other object, a damaged entry, or a path whose anchors were edited since.
pub fn read(n: &Node) -> Option<ParametricShape> {
    let NodeKind::Path { path, guide: false, .. } = &n.kind else { return None };
    let (_, text) = n.extras().attrs.as_deref()?.data.iter().find(|(k, _)| k == KEY)?;
    if text.len() > MAX_STORED {
        return None;
    }
    let v: Value = serde_json::from_str(text).ok()?;
    let s = apply(ParametricShape::new(ShapeKind::Polygon, Point::ZERO, 100.0, 100.0), &v).ok()?;
    let fp = v.get("fp").and_then(Value::as_str).and_then(|f| u64::from_str_radix(f, 16).ok())?;
    if fp == fingerprint(path) { Some(s) } else { s.recover(path) }
}

/// The values the arrow keys change while drawing: ↑/↓ the first, ←/→ the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Count,
    A,
    B,
    Radius,
}

/// The kind's (↑/↓, ←/→) values.
fn arrow_fields(kind: ShapeKind) -> (Option<Field>, Option<Field>) {
    match kind {
        ShapeKind::Rectangle => (None, Some(Field::Radius)),
        ShapeKind::Ellipse => (None, None),
        ShapeKind::Polygon => (Some(Field::Count), Some(Field::Radius)),
        ShapeKind::Star | ShapeKind::Gear => (Some(Field::Count), Some(Field::A)),
        ShapeKind::Ring => (Some(Field::A), None),
        ShapeKind::Arrow | ShapeKind::Cross | ShapeKind::Heart | ShapeKind::Bubble => (Some(Field::A), Some(Field::B)),
    }
}

/// `s` with `field` one step up (`up`) or down: a count by one, a value by 5 % of its range, the
/// corner radius by `step` pt; each kept within its range.
fn stepped(mut s: ParametricShape, field: Field, up: bool, step: f64) -> ParametricShape {
    let sign = if up { 1.0 } else { -1.0 };
    let value = |v: f64, slot: usize| match s.kind.values().get(slot).copied().flatten() {
        Some(r) => (v + sign * 0.05 * (r.max - r.min)).clamp(r.min, r.max),
        None => v,
    };
    match field {
        Field::Count => s.count = if up { (s.count + 1).min(MAX_COUNT) } else { s.count.saturating_sub(1).max(MIN_COUNT) },
        Field::A => s.a = value(s.a, 0),
        Field::B => s.b = value(s.b, 1),
        Field::Radius => s.radius = (s.radius + sign * step).clamp(0.0, MAX_SIZE),
    }
    s
}

/// The handles of a selected shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grip {
    /// The centre: moves the shape.
    Move,
    /// The box's lower right corner: sizes it about its centre (Shift keeps its proportions).
    Size,
    /// Above the top: turns it (Shift: 45° steps).
    Rotate,
    /// On the box's top edge, the radius in from its left end: the corner radius.
    Corner,
    /// One of the kind's own values, where it shows.
    Value,
}

/// Where the kind's own value handle is, in the shape's unit box (−1..1 each way, y down).
fn value_spot(s: &ParametricShape) -> Option<Point> {
    let n = f64::from(s.count.max(1));
    Some(match s.kind {
        ShapeKind::Star => {
            let (sn, cs) = (std::f64::consts::PI / n).sin_cos();
            Point::new(s.a * sn, -s.a * cs)
        }
        ShapeKind::Ring => Point::new(0.0, -s.a),
        ShapeKind::Gear => Point::new(0.0, -(1.0 - s.a)),
        ShapeKind::Arrow => Point::new(1.0, -1.0 + 2.0 * s.a),
        ShapeKind::Cross => Point::new(-1.0, s.a * (1.0 - s.b)),
        ShapeKind::Heart => Point::new(0.0, -1.0 + s.a),
        ShapeKind::Bubble => Point::new(s.b, 1.0),
        ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Polygon => return None,
    })
}

/// `s` with its own value set from the handle dragged to `u` (unit box), within its range.
fn value_from(mut s: ParametricShape, u: Point) -> ParametricShape {
    let clamp = |v: f64, slot: usize| s.kind.values().get(slot).copied().flatten().map_or(v, |r| v.clamp(r.min, r.max));
    match s.kind {
        ShapeKind::Star | ShapeKind::Ring => s.a = clamp(u.to_vec2().hypot(), 0),
        ShapeKind::Gear => s.a = clamp(1.0 - u.to_vec2().hypot(), 0),
        ShapeKind::Arrow => s.a = clamp((u.y + 1.0) / 2.0, 0),
        ShapeKind::Cross => s.a = clamp(u.y / (1.0 - s.b).max(1e-9), 0),
        ShapeKind::Heart => s.a = clamp(u.y + 1.0, 0),
        ShapeKind::Bubble => s.b = clamp(u.x, 1),
        ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Polygon => {}
    }
    s
}

/// The handles of `s` and where they are on the page.
fn grips(cx: &ToolContext, s: &ParametricShape) -> Vec<(Grip, Point)> {
    let (f, rx, ry) = (s.frame(), s.width / 2.0, s.height / 2.0);
    let mut out = vec![(Grip::Move, s.center), (Grip::Size, f * Point::new(rx, ry)), (Grip::Rotate, f * Point::new(0.0, -ry - cx.tol(STEM_PX)))];
    if s.kind.rounds() {
        out.push((Grip::Corner, f * Point::new(-rx + s.radius.min(rx), -ry)));
    }
    if let Some(u) = value_spot(s) {
        out.push((Grip::Value, f * Point::new(u.x * rx, u.y * ry)));
    }
    out
}

/// The angle (degrees clockwise from straight up) of `p` seen from `c`; with Shift in 45° steps.
fn angle(c: Point, p: Point, shift: bool) -> f64 {
    let d = p - c;
    let deg = d.x.atan2(-d.y).to_degrees();
    vectorcraft_geom::normalize_deg(if shift { (deg / 45.0).round() * 45.0 } else { deg })
}

/// A drag under way.
#[derive(Clone, Debug, PartialEq)]
enum Drag {
    /// Drawing a shape out from its centre.
    Create { center: Point },
    /// Dragging a handle of shape `id`, which was `shape` when the drag began at `from`.
    Edit { id: NodeId, shape: ParametricShape, grip: Grip, from: Point },
}

pub struct ParametricShapeTool {
    /// What the next shape is (its kind and numbers, and the size and rotation a double-click
    /// gives it: the last drawn one's).
    next: ParametricShape,
    drag: Option<Drag>,
    /// Where the pointer last was during the drag, and the keys held.
    last: (Point, Mods),
}

impl Default for ParametricShapeTool {
    fn default() -> Self {
        Self { next: ParametricShape::new(ShapeKind::Polygon, Point::ZERO, 100.0, 100.0), drag: None, last: (Point::ZERO, Mods::default()) }
    }
}

/// The one selected object when it is an editable parametric shape.
fn selected(cx: &ToolContext) -> Option<(NodeId, ParametricShape)> {
    let [id] = cx.selection.objects.as_slice() else { return None };
    let s = read(cx.doc.node(*id)?)?;
    cx.doc.is_editable(*id).then_some((*id, s))
}

/// The editable parametric shape under `p`.
fn shape_at(cx: &ToolContext, p: Point) -> Option<NodeId> {
    let id = hit_test(cx.doc, p, cx.hit_options())?.leaf;
    (cx.doc.is_editable(id) && read(cx.doc.node(id)?).is_some()).then_some(id)
}

impl ParametricShapeTool {
    /// The shape a drag to `p` with `mods` makes, and the command that makes it.
    fn shaped(&self, drag: &Drag, p: Point, mods: Mods) -> (&'static str, ParametricShape, Option<NodeId>) {
        let fit = |v: f64| v.clamp(MIN_SIZE, MAX_SIZE);
        match *drag {
            Drag::Create { center } => {
                let r = center.distance(p);
                let aspect = self.next.width / self.next.height;
                let s = ParametricShape {
                    center,
                    width: fit(2.0 * r * aspect),
                    height: fit(2.0 * r),
                    rotation: angle(center, p, mods.shift),
                    ..self.next
                };
                ("addon.pathtools.shape.create", s, None)
            }
            Drag::Edit { id, shape: s, grip, from } => {
                let local = s.frame().inverse() * p;
                let (rx, ry) = (s.width / 2.0, s.height / 2.0);
                let out = match grip {
                    Grip::Move => ParametricShape { center: s.center + (p - from), ..s },
                    Grip::Size if mods.shift => {
                        let k = (local.x.abs() / rx).max(local.y.abs() / ry);
                        ParametricShape { width: fit(s.width * k), height: fit(s.height * k), ..s }
                    }
                    Grip::Size => ParametricShape { width: fit(2.0 * local.x.abs()), height: fit(2.0 * local.y.abs()), ..s },
                    Grip::Rotate => ParametricShape { rotation: angle(s.center, p, mods.shift), ..s },
                    Grip::Corner => ParametricShape { radius: (local.x + rx).clamp(0.0, rx.min(MAX_SIZE)), ..s },
                    Grip::Value => value_from(s, Point::new(local.x / rx, local.y / ry)),
                };
                ("addon.pathtools.shape.set", out, Some(id))
            }
        }
    }

    /// The preview of the drag under way at the pointer's last place.
    fn preview(&self) -> Option<Action> {
        let drag = self.drag.as_ref()?;
        let (cmd, s, id) = self.shaped(drag, self.last.0, self.last.1);
        let mut p = to_json(&s);
        if let Some(id) = id {
            p["id"] = json!(id.0);
        }
        Some(Action::Preview(cmd.into(), p))
    }
}

impl Tool for ParametricShapeTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => vec![],
            PointerKind::Down => {
                let tol = cx.point_tol();
                let grip = selected(cx).and_then(|(id, s)| {
                    let near = grips(cx, &s).into_iter().filter(|(_, at)| at.distance(ev.pos) <= tol);
                    near.min_by(|a, b| a.1.distance(ev.pos).total_cmp(&b.1.distance(ev.pos))).map(|(g, _)| (id, s, g))
                });
                if let Some((id, shape, grip)) = grip {
                    self.drag = Some(Drag::Edit { id, shape, grip, from: ev.pos });
                    self.last = (ev.pos, ev.mods);
                    return vec![Action::Begin(EDIT.into())];
                }
                if let Some(id) = shape_at(cx, ev.pos) {
                    return if cx.selection.objects == [id] { vec![] } else { vec![Action::Exec("select.set".into(), json!({ "ids": [id.0] }))] };
                }
                let (center, _) = crate::guides::snap_draw(cx, ev.pos, &[]);
                self.drag = Some(Drag::Create { center });
                self.last = (center, ev.mods);
                vec![Action::Begin(CREATE.into())]
            }
            PointerKind::Drag | PointerKind::Up => {
                if self.drag.is_none() {
                    return vec![];
                }
                self.last = (ev.pos, ev.mods);
                let Some(preview) = self.preview() else { return vec![] };
                if ev.kind == PointerKind::Drag {
                    return vec![preview];
                }
                let Some(drag) = self.drag.take() else { return vec![] };
                match drag {
                    // A click draws nothing (a double-click places a shape).
                    Drag::Create { center } if center.distance(ev.pos) <= cx.tol(2.0) => vec![Action::Cancel],
                    Drag::Create { .. } => {
                        if let Action::Preview(_, p) = &preview
                            && let Ok(s) = apply(self.next, p)
                        {
                            self.next = ParametricShape { center: Point::ZERO, ..s };
                        }
                        vec![preview, Action::Commit]
                    }
                    Drag::Edit { from, .. } if from == ev.pos => vec![Action::Cancel],
                    Drag::Edit { .. } => vec![preview, Action::Commit],
                }
            }
            PointerKind::DoubleClick => {
                if self.drag.is_some() || shape_at(cx, ev.pos).is_some() {
                    return vec![];
                }
                let s = ParametricShape { center: ev.pos, ..self.next };
                vec![Action::Exec("addon.pathtools.shape.create".into(), to_json(&s))]
            }
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.drag.take().is_some() {
            return vec![Action::Cancel];
        }
        if !matches!(self.drag, Some(Drag::Create { .. })) {
            return vec![];
        }
        let (primary, secondary) = arrow_fields(self.next.kind);
        let (field, up) = match key {
            ToolKey::Up => (primary, true),
            ToolKey::Down => (primary, false),
            ToolKey::Right => (secondary, true),
            ToolKey::Left => (secondary, false),
            _ => return vec![],
        };
        let Some(field) = field else { return vec![] };
        self.next = stepped(self.next, field, up, cx.keyboard_increment.max(0.01));
        self.preview().into_iter().collect()
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        match self.drag {
            Some(Drag::Create { .. }) => matches!(key, ToolKey::Up | ToolKey::Down | ToolKey::Left | ToolKey::Right | ToolKey::Escape),
            Some(Drag::Edit { .. }) => key == ToolKey::Escape,
            None => false,
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if let Some(Drag::Create { center }) = &self.drag {
            let (_, s, _) = self.shaped(&Drag::Create { center: *center }, self.last.0, self.last.1);
            let text = format!("{}\nAngle: {:.1}°", cx.size_label(s.width, s.height), s.rotation);
            return vec![Overlay::Measure { p: self.last.0 + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text }];
        }
        let Some((_, s)) = selected(cx) else { return vec![] };
        // The shape's box, then its handles.
        let (rx, ry) = (s.width / 2.0, s.height / 2.0);
        let mut bp = BezPath::new();
        let f: Affine = s.frame();
        let corners = [(-rx, -ry), (rx, -ry), (rx, ry), (-rx, ry)].map(|(x, y)| f * Point::new(x, y));
        for (i, p) in corners.iter().enumerate() {
            if i == 0 { bp.move_to(*p) } else { bp.line_to(*p) }
        }
        bp.close_path();
        let mut out = vec![Overlay::Path { path: bp, color: BLUE, width: 1.0, dashed: true }];
        for (grip, p) in grips(cx, &s) {
            if grip == Grip::Rotate {
                out.push(Overlay::Line { a: f * Point::new(0.0, -ry), b: p, color: BLUE, dashed: false });
            }
            out.push(match grip {
                Grip::Move | Grip::Size => Overlay::Anchor { p, color: BLUE, filled: grip == Grip::Move, size: 7.0 },
                Grip::Rotate => Overlay::Handle { p, color: BLUE },
                Grip::Corner | Grip::Value => Overlay::CurvePoint { p, color: BLUE, filled: grip == Grip::Value, size: 7.0 },
            });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::ParametricShape
    }

    /// The next shape's numbers (its size and rotation are a double-click's).
    fn options(&self) -> Value {
        let mut v = to_json(&self.next);
        if let Some(o) = v.as_object_mut() {
            o.remove("center");
        }
        v
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        if key != "center"
            && let Ok(s) = apply(self.next, &json!({ key: v }))
        {
            self.next = s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Selection};

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    /// A document with a parametric star about (250, 250), 100 × 100: the document and its id.
    fn doc_with_star() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let s = ParametricShape::new(ShapeKind::Star, Point::new(250.0, 250.0), 100.0, 100.0);
        let path = s.path().unwrap();
        let mut n = Node::path(id, path.clone(), Appearance::default_art());
        n.edit_attrs(|a| a.data.push((KEY.into(), stored(&s, &path))));
        d.insert(Some(l), 0, n).unwrap();
        (d, id)
    }

    fn preview_of(acts: &[Action]) -> (String, Value) {
        match acts.first() {
            Some(Action::Preview(c, p)) => (c.clone(), p.clone()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_drag_from_the_centre_draws_sized_and_turned_and_arrows_change_its_numbers() {
        let (d, _) = doc_with_rect();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = ParametricShapeTool::default();
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0)), vec![Action::Begin(CREATE.into())]);
        // 40 pt to the right: 80 across, turned a quarter clockwise.
        let acts = t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 300.0));
        let (cmd, p) = preview_of(&acts);
        assert_eq!(cmd, "addon.pathtools.shape.create");
        assert_eq!(
            (p["kind"].clone(), p["center"].clone(), p["width"].clone(), p["rotation"].clone()),
            (json!("polygon"), json!([300.0, 300.0]), json!(80.0), json!(90.0))
        );
        assert!(t.claims_key(&cx, ToolKey::Up) && t.busy());
        // ↑ adds a side, ← takes a point off the corner radius (none left: stays 0), → adds one.
        let (_, p) = preview_of(&t.key(&cx, ToolKey::Up, Mods::default()));
        assert_eq!(p["count"], json!(7));
        let (_, p) = preview_of(&t.key(&cx, ToolKey::Right, Mods::default()));
        assert_eq!(p["radius"], json!(1.0));
        // Shift turns in 45° steps.
        let shifted = ev(PointerKind::Drag, 340.0, 290.0).with_mods(Mods { shift: true, ..Mods::default() });
        let (_, p) = preview_of(&t.pointer(&cx, &shifted));
        assert_eq!(p["rotation"], json!(90.0));
        let up = t.pointer(&cx, &ev(PointerKind::Up, 300.0, 250.0));
        assert!(
            matches!(up.as_slice(), [Action::Preview(_, p), Action::Commit] if p["rotation"] == json!(0.0) && p["height"] == json!(100.0)),
            "{up:?}"
        );
        assert!(!t.busy() && !t.claims_key(&cx, ToolKey::Up), "arrows nudge again");
        // The next shape remembers the numbers; a double-click places it.
        assert_eq!((t.options()["count"].clone(), t.options()["radius"].clone()), (json!(7), json!(1.0)));
        let acts = t.pointer(&cx, &ev(PointerKind::DoubleClick, 50.0, 400.0));
        let [Action::Exec(cmd, p)] = acts.as_slice() else { panic!("{acts:?}") };
        assert_eq!(
            (cmd.as_str(), p["center"].clone(), p["count"].clone(), p["height"].clone()),
            ("addon.pathtools.shape.create", json!([50.0, 400.0]), json!(7), json!(100.0))
        );
        // A click without a drag draws nothing; Escape drops a drag.
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0)), vec![Action::Cancel]);
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::ParametricShape);
    }

    #[test]
    fn each_kinds_arrow_keys_step_within_range() {
        let s = ParametricShape::new(ShapeKind::Star, Point::ZERO, 10.0, 10.0);
        assert_eq!(stepped(s, Field::Count, false, 1.0).count, 4);
        assert_eq!(stepped(ParametricShape { count: 3, ..s }, Field::Count, false, 1.0).count, 3);
        assert_eq!(stepped(ParametricShape { count: MAX_COUNT, ..s }, Field::Count, true, 1.0).count, MAX_COUNT);
        assert!((stepped(s, Field::A, true, 1.0).a - 0.5495).abs() < 1e-9);
        assert_eq!(stepped(ParametricShape { a: 1.0, ..s }, Field::A, true, 1.0).a, 1.0);
        for kind in ShapeKind::ALL {
            let (up, side) = arrow_fields(kind);
            let s = ParametricShape::new(kind, Point::ZERO, 10.0, 10.0);
            for f in [up, side].into_iter().flatten() {
                assert_eq!(stepped(s, f, true, 1.0).check(), Ok(()), "{kind:?} {f:?}");
                assert_eq!(stepped(s, f, false, 1.0).check(), Ok(()), "{kind:?} {f:?}");
            }
        }
    }

    #[test]
    fn a_click_selects_a_shape_and_its_handles_drag_into_set() {
        let (d, id) = doc_with_star();
        let p = paint();
        let none = Selection::default();
        let mut t = ParametricShapeTool::default();
        let acts = t.pointer(&cx(&d, &none, &p), &ev(PointerKind::Down, 250.0, 250.0));
        assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [id.0] }))]);
        assert!(t.overlays(&cx(&d, &none, &p)).is_empty());
        let mut sel = Selection::default();
        sel.set([id]);
        let cx = cx(&d, &sel, &p);
        let ov = t.overlays(&cx);
        assert!(matches!(ov.first(), Some(Overlay::Path { dashed: true, .. })) && ov.len() >= 6, "{ov:?}");
        // The size handle, at the box's lower right corner: 150 × 120 now.
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0)), vec![Action::Begin(EDIT.into())]);
        let (cmd, p) = preview_of(&t.pointer(&cx, &ev(PointerKind::Drag, 325.0, 310.0)));
        assert_eq!(
            (cmd.as_str(), p["id"].clone(), p["width"].clone(), p["height"].clone()),
            ("addon.pathtools.shape.set", json!(id.0), json!(150.0), json!(120.0))
        );
        assert!(matches!(t.pointer(&cx, &ev(PointerKind::Up, 325.0, 310.0)).as_slice(), [Action::Preview(..), Action::Commit]));
        // The centre moves it.
        t.pointer(&cx, &ev(PointerKind::Down, 251.0, 249.0));
        let (_, p) = preview_of(&t.pointer(&cx, &ev(PointerKind::Drag, 261.0, 229.0)));
        assert_eq!(p["center"], json!([260.0, 230.0]));
        t.key(&cx, ToolKey::Escape, Mods::default());
        // The rotation handle above the top.
        t.pointer(&cx, &ev(PointerKind::Down, 250.0, 200.0 - STEM_PX));
        let (_, p) = preview_of(&t.pointer(&cx, &ev(PointerKind::Drag, 350.0, 250.0)));
        assert_eq!(p["rotation"], json!(90.0));
        t.key(&cx, ToolKey::Escape, Mods::default());
        // The star's inner radius, on its first inner corner.
        let inner = Point::new(250.0, 250.0) + Vec2::new((36f64).to_radians().sin(), -(36f64).to_radians().cos()) * 25.0;
        t.pointer(&cx, &ev(PointerKind::Down, inner.x, inner.y));
        let (_, p) = preview_of(&t.pointer(&cx, &ev(PointerKind::Drag, 250.0, 210.0)));
        assert!((p["a"].as_f64().unwrap() - 0.8).abs() < 1e-9, "{p}");
        t.key(&cx, ToolKey::Escape, Mods::default());
        // The corner radius on the top edge.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0));
        let (_, p) = preview_of(&t.pointer(&cx, &ev(PointerKind::Drag, 212.0, 200.0)));
        assert_eq!(p["radius"], json!(12.0));
        // Pressing and letting go of a handle without moving changes nothing.
        t.key(&cx, ToolKey::Escape, Mods::default());
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 300.0)), vec![Action::Cancel]);
        // A double-click on a shape places none.
        assert!(t.pointer(&cx, &ev(PointerKind::DoubleClick, 250.0, 250.0)).is_empty());
    }

    #[test]
    fn stored_numbers_are_read_back_only_while_the_path_is_the_shape() {
        let (mut d, id) = doc_with_star();
        let s = read(d.node(id).unwrap()).unwrap();
        assert_eq!((s.kind, s.center, s.count), (ShapeKind::Star, Point::new(250.0, 250.0), 5));
        // Moved: still the shape, about its new centre.
        d.node_mut(id).unwrap().path_data_mut().unwrap().transform(Affine::translate((10.0, 0.0)));
        assert_eq!(read(d.node(id).unwrap()).map(|s| s.center), Some(Point::new(260.0, 250.0)));
        // An anchor moved: a plain path.
        d.node_mut(id).unwrap().path_data_mut().unwrap().anchor_mut(0, 2).unwrap().p.x += 3.0;
        assert_eq!(read(d.node(id).unwrap()), None);
        // Damaged entries are no shape.
        for bad in ["", "{", "[]", r#"{"kind":"blob"}"#, r#"{"kind":"star","count":5000,"fp":"0"}"#, r#"{"kind":"star"}"#] {
            let mut n = d.node(id).unwrap().clone();
            n.edit_attrs(|a| a.data = vec![(KEY.into(), bad.into())]);
            assert_eq!(read(&n), None, "{bad}");
        }
        let plain = Node::path(NodeId(99), PathData::default(), Appearance::default_art());
        assert_eq!(read(&plain), None);
    }

    #[test]
    fn options_set_the_next_shape_and_refuse_bad_values() {
        let mut t = ParametricShapeTool::default();
        t.set_option("kind", &json!("gear"));
        t.set_option("count", &json!(20));
        t.set_option("count", &json!(5000));
        t.set_option("a", &json!(0.3));
        t.set_option("width", &json!(-4));
        t.set_option("kind", &json!("cloud"));
        let o = t.options();
        assert_eq!((o["kind"].clone(), o["count"].clone(), o["a"].clone(), o["width"].clone()), (json!("gear"), json!(20), json!(0.3), json!(100.0)));
        assert!(o.get("center").is_none());
        assert!(apply(t.next, &json!({"center": [1, "x"]})).is_err());
        assert_eq!(apply(t.next, &json!({"rotation": 450})).map(|s| s.rotation), Ok(90.0));
    }
}
