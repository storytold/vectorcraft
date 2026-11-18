//! Proptest strategies: geometry, engine command sequences, junk parameters.

use proptest::prelude::*;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::{Location, ParamsPlace, key_location, location_keys};
use vectorcraft_engine::{CommandSpec, Session, command_specs};
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath};

use crate::fixtures::{art_nodes, top_level_ids};

// ---------------------------------------------------------------- geometry

pub fn arb_point(lo: f64, hi: f64) -> impl Strategy<Value = Point> {
    (lo..hi, lo..hi).prop_map(|(x, y)| Point::new(x, y))
}

/// A non-degenerate rectangle inside `[0, 200]²`.
pub fn arb_rect() -> impl Strategy<Value = Rect> {
    (0.0..150.0f64, 0.0..150.0f64, 2.0..80.0f64, 2.0..80.0f64).prop_map(|(x, y, w, h)| Rect::new(x, y, x + w, y + h))
}

/// A simple (non self-intersecting) star-shaped polygon: points at increasing angles around a
/// centre with random radii. Every angular step stays below half a turn: a wider one puts the
/// centre outside and can make the polygon cross itself.
pub fn arb_star_polygon() -> impl Strategy<Value = Vec<Point>> {
    let steps = prop::collection::vec((0.2f64..1.0, 5.0f64..40.0), 3..12).prop_filter("an angular step of half a turn or more", |v| {
        let total: f64 = v.iter().map(|(a, _)| a).sum();
        v.iter().all(|(a, _)| 2.0 * a < total)
    });
    (arb_point(40.0, 160.0), steps).prop_map(|(c, v)| {
        let total: f64 = v.iter().map(|(a, _)| a).sum();
        let mut ang = 0.0;
        v.iter()
            .map(|&(a, r)| {
                ang += a / total * std::f64::consts::TAU;
                Point::new(c.x + r * ang.cos(), c.y + r * ang.sin())
            })
            .collect()
    })
}

/// A convex polygon (regular polygon with jittered radius kept convex by using a single radius per
/// polygon and jittered angles only).
pub fn arb_convex_polygon() -> impl Strategy<Value = Vec<Point>> {
    (arb_point(40.0, 160.0), 5.0f64..40.0, prop::collection::vec(0.3f64..1.0, 3..10)).prop_map(|(c, r, v)| {
        let total: f64 = v.iter().sum();
        let mut ang = 0.0;
        v.iter()
            .map(|a| {
                ang += a / total * std::f64::consts::TAU;
                Point::new(c.x + r * ang.cos(), c.y + r * ang.sin())
            })
            .collect()
    })
}

pub fn polygon_path(pts: &[Point]) -> PathData {
    PathData::single(SubPath::polyline(pts, true))
}

/// An axis-aligned ellipse as a path.
pub fn arb_ellipse() -> impl Strategy<Value = PathData> {
    arb_rect().prop_map(vectorcraft_geom::shapes::ellipse)
}

/// Rectangles, ellipses and star-shaped polygons.
pub fn arb_closed_shape() -> impl Strategy<Value = PathData> {
    prop_oneof![arb_rect().prop_map(vectorcraft_geom::shapes::rectangle), arb_ellipse(), arb_star_polygon().prop_map(|p| polygon_path(&p)),]
}

/// A random cubic Bézier path (open or closed, possibly several subpaths, arbitrary handles).
pub fn arb_path_data() -> impl Strategy<Value = PathData> {
    let anchor = (arb_point(0.0, 200.0), arb_point(-30.0, 30.0), arb_point(-30.0, 30.0), any::<bool>())
        .prop_map(|(p, i, o, corner)| if corner { Anchor::corner(p) } else { Anchor::with_handles(p, p + i.to_vec2(), p + o.to_vec2()) });
    let sub = (prop::collection::vec(anchor, 2..8), any::<bool>()).prop_map(|(a, closed)| SubPath::new(a, closed));
    prop::collection::vec(sub, 1..3).prop_map(PathData::new)
}

/// A smooth open curve through points on a gently varying function (for simplify tests): densely
/// sampled polyline-like cubic chain with C1 handles.
pub fn arb_smooth_curve() -> impl Strategy<Value = PathData> {
    (prop::collection::vec(-20.0f64..20.0, 4..9), 10.0f64..30.0).prop_map(|(ys, step)| {
        // Catmull-Rom through (i*step, y_i), converted to cubic handles.
        let pts: Vec<Point> = ys.iter().enumerate().map(|(i, y)| Point::new(i as f64 * step, 100.0 + y)).collect();
        let n = pts.len();
        let tangent = |i: usize| {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            (b - a) / 6.0
        };
        let anchors = (0..n).map(|i| Anchor::with_handles(pts[i], pts[i] - tangent(i), pts[i] + tangent(i))).collect();
        PathData::single(SubPath::new(anchors, false))
    })
}

// ---------------------------------------------------------------- engine ops

/// One abstract editing step. Object references are indices resolved against the live document
/// when the op is applied (so generated sequences stay meaningful as ids change).
#[derive(Clone, Debug)]
pub enum Op {
    Rect(f64, f64, f64, f64),
    Ellipse(f64, f64, f64, f64),
    Polygon(f64, f64, f64, u32),
    Star(f64, f64, f64, f64),
    Line(f64, f64, f64, f64),
    Path(Vec<(f64, f64)>, bool),
    Text(f64, f64),
    SelectSet(Vec<usize>),
    SelectAll,
    SelectNone,
    SelectInverse,
    Move(f64, f64, bool),
    Rotate(f64, bool),
    Scale(f64, f64),
    Reflect(bool),
    Shear(f64),
    TransformAgain,
    Nudge(i8, i8),
    Group,
    Ungroup,
    Arrange(u8),
    Pathfinder(u8),
    Fill(u8),
    Gradient(f64),
    Stroke(u8, f64, bool),
    Transparency(f64, u8),
    Align(u8),
    Distribute,
    LayerNew,
    LayerDelete,
    LayerCurrent(usize),
    Collect,
    Copy,
    Cut,
    Paste(u8),
    Duplicate,
    Clear,
    Undo,
    Redo,
    Clip,
    ClipRelease,
    Compound,
    CompoundRelease,
    /// Make Compound Shape in one of the four shape modes, a member's mode, Release or Expand.
    CompoundShape(u8),
    Lock,
    UnlockAll,
    Hide,
    ShowAll,
    Offset(f64),
    Simplify,
    OutlineStroke,
    ExpandShape,
    Effect(u8),
    ExpandAppearance,
    Batch(Vec<Op>),
}

pub const PATHFINDER: [&str; 10] = [
    "object.pathfinder.unite",
    "object.pathfinder.minusFront",
    "object.pathfinder.intersect",
    "object.pathfinder.exclude",
    "object.pathfinder.divide",
    "object.pathfinder.trim",
    "object.pathfinder.merge",
    "object.pathfinder.crop",
    "object.pathfinder.outline",
    "object.pathfinder.minusBack",
];
const ARRANGE: [&str; 5] = [
    "object.arrange.bringToFront",
    "object.arrange.bringForward",
    "object.arrange.sendBackward",
    "object.arrange.sendToBack",
    "object.arrange.sendToCurrentLayer",
];
const COLORS: [&str; 6] = ["#ff0000", "#00ff00", "#0000ff", "#222222", "#ffffff", "#ffcc00"];
const BLENDS: [&str; 4] = ["Normal", "Multiply", "Screen", "Difference"];
const PASTE: [&str; 4] = ["edit.paste", "edit.pasteInFront", "edit.pasteInBack", "edit.pasteInPlace"];

fn coord() -> impl Strategy<Value = f64> {
    -20.0..380.0f64
}
fn size() -> impl Strategy<Value = f64> {
    1.0..150.0f64
}

fn arb_leaf_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        // creation (weighted up so documents have content)
        4 => (coord(), coord(), size(), size()).prop_map(|(a, b, c, d)| Op::Rect(a, b, c, d)),
        3 => (coord(), coord(), size(), size()).prop_map(|(a, b, c, d)| Op::Ellipse(a, b, c, d)),
        1 => (coord(), coord(), 5.0..80.0f64, 3u32..9).prop_map(|(a, b, c, d)| Op::Polygon(a, b, c, d)),
        1 => (coord(), coord(), 10.0..80.0f64, 3.0..40.0f64).prop_map(|(a, b, c, d)| Op::Star(a, b, c, d)),
        1 => (coord(), coord(), coord(), coord()).prop_map(|(a, b, c, d)| Op::Line(a, b, c, d)),
        1 => (prop::collection::vec((coord(), coord()), 2..6), any::<bool>()).prop_map(|(v, c)| Op::Path(v, c)),
        1 => (coord(), coord()).prop_map(|(a, b)| Op::Text(a, b)),
        // selection
        5 => prop::collection::vec(0usize..64, 1..4).prop_map(Op::SelectSet),
        1 => Just(Op::SelectAll),
        1 => Just(Op::SelectNone),
        1 => Just(Op::SelectInverse),
        // transforms
        2 => (-50.0..50.0f64, -50.0..50.0f64, any::<bool>()).prop_map(|(a, b, c)| Op::Move(a, b, c)),
        1 => (-180.0..180.0f64, any::<bool>()).prop_map(|(a, c)| Op::Rotate(a, c)),
        1 => (20.0..300.0f64, 20.0..300.0f64).prop_map(|(a, b)| Op::Scale(a, b)),
        1 => any::<bool>().prop_map(Op::Reflect),
        1 => (-45.0..45.0f64).prop_map(Op::Shear),
        1 => Just(Op::TransformAgain),
        1 => (-1i8..=1, -1i8..=1).prop_map(|(a, b)| Op::Nudge(a, b)),
        // structure
        2 => Just(Op::Group),
        2 => Just(Op::Ungroup),
        2 => (0u8..5).prop_map(Op::Arrange),
        3 => (0u8..10).prop_map(Op::Pathfinder),
        // paint
        2 => (0u8..6).prop_map(Op::Fill),
        1 => (0.0..360.0f64).prop_map(Op::Gradient),
        1 => (0u8..6, 0.0..10.0f64, any::<bool>()).prop_map(|(a, b, c)| Op::Stroke(a, b, c)),
        1 => (0.0..100.0f64, 0u8..4).prop_map(|(a, b)| Op::Transparency(a, b)),
        1 => (0u8..6).prop_map(Op::Align),
        1 => Just(Op::Distribute),
        // layers
        1 => Just(Op::LayerNew),
        1 => Just(Op::LayerDelete),
        1 => (0usize..4).prop_map(Op::LayerCurrent),
        1 => Just(Op::Collect),
        // clipboard
        1 => Just(Op::Copy),
        1 => Just(Op::Cut),
        2 => (0u8..4).prop_map(Op::Paste),
        1 => Just(Op::Duplicate),
        1 => Just(Op::Clear),
        // history
        3 => Just(Op::Undo),
        2 => Just(Op::Redo),
        // object menu
        1 => Just(Op::Clip),
        1 => Just(Op::ClipRelease),
        1 => Just(Op::Compound),
        1 => Just(Op::CompoundRelease),
        1 => (0u8..7).prop_map(Op::CompoundShape),
        1 => Just(Op::Lock),
        1 => Just(Op::UnlockAll),
        1 => Just(Op::Hide),
        1 => Just(Op::ShowAll),
        1 => (-10.0..10.0f64).prop_map(Op::Offset),
        1 => Just(Op::Simplify),
        1 => Just(Op::OutlineStroke),
        1 => Just(Op::ExpandShape),
        1 => (0u8..4).prop_map(Op::Effect),
        1 => Just(Op::ExpandAppearance),
    ]
}

/// Any op, including (non-nested) batches.
pub fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        30 => arb_leaf_op(),
        1 => prop::collection::vec(arb_leaf_op().prop_filter("no history ops in batches", |o| !matches!(o, Op::Undo | Op::Redo)), 1..5).prop_map(Op::Batch),
    ]
}

/// A sequence of `len` ops.
pub fn arb_ops(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(arb_op(), len)
}

fn pick(ids: &[vectorcraft_doc::NodeId], i: usize) -> Option<u64> {
    if ids.is_empty() { None } else { Some(ids[i % ids.len()].0) }
}

impl Op {
    /// The command id and params this op maps to in the current session state.
    pub fn command(&self, s: &Session) -> (String, Value) {
        let doc = s.active().map(|d| d.doc.clone());
        // Children of compound paths are excluded: grouping one of them nests a group inside the
        // compound (known bug, see crates/engine/tests/it/known_bugs.rs).
        let art: Vec<_> = doc
            .as_ref()
            .map(|d| {
                art_nodes(d)
                    .iter()
                    .filter(|n| {
                        !d.parent_of(n.id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, vectorcraft_doc::NodeKind::Compound { .. }))
                    })
                    .map(|n| n.id)
                    .collect()
            })
            .unwrap_or_default();
        let layers: Vec<_> = doc.as_ref().map(|d| d.layers.iter().map(|l| l.id.0).collect()).unwrap_or_default();
        let c = |id: &str, p: Value| (id.to_string(), p);
        match self {
            Op::Rect(x, y, w, h) => c("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})),
            Op::Ellipse(x, y, w, h) => c("shape.ellipse", json!({"x": x, "y": y, "width": w, "height": h})),
            Op::Polygon(x, y, r, n) => c("shape.polygon", json!({"cx": x, "cy": y, "radius": r, "sides": n})),
            Op::Star(x, y, r1, r2) => c("shape.star", json!({"cx": x, "cy": y, "radius1": r1, "radius2": r2})),
            Op::Line(a, b, cc, d) => c("shape.line", json!({"x1": a, "y1": b, "x2": cc, "y2": d})),
            Op::Path(pts, closed) => {
                c("path.create", json!({"anchors": pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect::<Vec<_>>(), "closed": closed}))
            }
            Op::Text(x, y) => c("text.create", json!({"x": x, "y": y, "text": "Ab", "size": 18})),
            Op::SelectSet(ix) => c("select.set", json!({"ids": ix.iter().filter_map(|i| pick(&art, *i)).collect::<Vec<_>>()})),
            Op::SelectAll => c("select.all", json!({})),
            Op::SelectNone => c("select.none", json!({})),
            Op::SelectInverse => c("select.inverse", json!({})),
            Op::Move(dx, dy, copy) => c("object.move", json!({"dx": dx, "dy": dy, "copy": copy})),
            Op::Rotate(a, copy) => c("object.rotate", json!({"angle": a, "copy": copy})),
            Op::Scale(sx, sy) => c("object.scale", json!({"sx": sx, "sy": sy})),
            Op::Reflect(v) => c("object.reflect", json!({"axis": if *v { "vertical" } else { "horizontal" }})),
            Op::Shear(a) => c("object.shear", json!({"angle": a})),
            Op::TransformAgain => c("object.transformAgain", json!({})),
            Op::Nudge(x, y) => c("object.nudge", json!({"dx": x, "dy": y})),
            Op::Group => c("object.group", json!({})),
            Op::Ungroup => c("object.ungroup", json!({})),
            Op::Arrange(i) => {
                let i = *i as usize % ARRANGE.len();
                // Known bug (crates/engine/tests/it/known_bugs.rs): Bring to Front/Forward panics when
                // the selection spans several parents. Avoid generating that case.
                let multi_parent = doc.as_ref().zip(s.active()).is_some_and(|(d, st)| {
                    let mut parents: Vec<_> = st.selection.objects.iter().map(|id| d.parent_of(*id)).collect();
                    parents.dedup();
                    parents.len() > 1
                });
                if i < 2 && multi_parent { c("document.inspect", json!({})) } else { c(ARRANGE[i], json!({})) }
            }
            Op::Pathfinder(i) => c(PATHFINDER[*i as usize % PATHFINDER.len()], json!({})),
            Op::Fill(i) => c("paint.setFill", json!({"color": COLORS[*i as usize % COLORS.len()]})),
            Op::Gradient(a) => c(
                "paint.setFill",
                json!({"gradient": {"kind": if *a > 180.0 { "radial" } else { "linear" }, "angle": a, "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
            ),
            Op::Stroke(i, w, dash) => {
                let mut p = json!({"weight": w});
                if *dash {
                    p["dash"] = json!([4, 2]);
                }
                let _ = s;
                // Also set the colour through setStroke in a separate step would need two commands;
                // use the colour index to pick an arrowhead instead to cover more of stroke.set.
                if i % 2 == 0 {
                    p["endArrow"] = json!("Triangle");
                }
                c("stroke.set", p)
            }
            Op::Transparency(o, b) => c("transparency.set", json!({"opacity": o, "blend": BLENDS[*b as usize % BLENDS.len()]})),
            Op::Align(i) => {
                let p = match i % 6 {
                    0 => json!({"horizontal": "left"}),
                    1 => json!({"horizontal": "center"}),
                    2 => json!({"horizontal": "right"}),
                    3 => json!({"vertical": "top"}),
                    4 => json!({"vertical": "center", "to": "artboard"}),
                    _ => json!({"vertical": "bottom", "to": "key"}),
                };
                c("object.align", p)
            }
            Op::Distribute => c("object.distribute", json!({"horizontal": "center"})),
            Op::LayerNew => c("layer.new", json!({})),
            Op::LayerDelete => c("layer.delete", json!({})),
            Op::LayerCurrent(i) => c("layer.setCurrent", json!({"id": if layers.is_empty() { 0 } else { layers[i % layers.len()] }})),
            Op::Collect => c("layer.collectInNew", json!({})),
            Op::Copy => c("edit.copy", json!({})),
            Op::Cut => c("edit.cut", json!({})),
            Op::Paste(i) => c(PASTE[*i as usize % PASTE.len()], json!({})),
            Op::Duplicate => c("edit.duplicate", json!({"dx": 5, "dy": 5})),
            Op::Clear => c("edit.clear", json!({})),
            Op::Undo => c("edit.undo", json!({})),
            Op::Redo => c("edit.redo", json!({})),
            Op::Clip => c("object.clippingMask.make", json!({})),
            Op::ClipRelease => c("object.clippingMask.release", json!({})),
            Op::Compound => c("object.compoundPath.make", json!({})),
            Op::CompoundRelease => c("object.compoundPath.release", json!({})),
            Op::CompoundShape(i) => {
                const MODES: [&str; 4] = ["add", "subtract", "intersect", "exclude"];
                match i % 7 {
                    i @ 0..4 => c("object.compoundShape.make", json!({"mode": MODES[i as usize]})),
                    4 => c("object.compoundShape.setMode", json!({"mode": "exclude"})),
                    5 => c("object.compoundShape.release", json!({})),
                    _ => c("object.compoundShape.expand", json!({})),
                }
            }
            Op::Lock => c("object.lock", json!({})),
            Op::UnlockAll => c("object.unlockAll", json!({})),
            Op::Hide => c("object.hide", json!({})),
            Op::ShowAll => c("object.showAll", json!({})),
            Op::Offset(d) => c("object.path.offsetPath", json!({"offset": d})),
            Op::Simplify => c("object.path.simplify", json!({"tolerance": 2})),
            Op::OutlineStroke => c("object.path.outlineStroke", json!({})),
            Op::ExpandShape => c("object.expandShape", json!({})),
            Op::Effect(i) => {
                let (id, p) = match i % 4 {
                    0 => ("distort.roughen", json!({"size": 3, "detail": 4, "seed": 1})),
                    1 => ("stylize.dropShadow", json!({})),
                    2 => ("distort.zigZag", json!({"size": 3, "ridges": 2})),
                    _ => ("stylize.roundCorners", json!({"radius": 5})),
                };
                c("effect.apply", json!({"effect": id, "params": p}))
            }
            Op::ExpandAppearance => c("effect.expandAppearance", json!({})),
            Op::Batch(ops) => {
                // Batches are resolved against the state *before* the batch; index-based
                // references may therefore point at objects later steps removed (then that step
                // errors and the whole batch rolls back — also worth exercising).
                let cmds: Vec<Value> = ops
                    .iter()
                    // See the Arrange note above: selection inside a batch isn't known up front.
                    .map(|o| if let Op::Arrange(i) = o { Op::Arrange(2 + i % 3) } else { o.clone() })
                    .map(|o| {
                        let (id, p) = o.command(s);
                        json!({"command": id, "params": p})
                    })
                    .collect();
                c("command.batch", json!({"label": "Gen batch", "commands": cmds}))
            }
        }
    }

    /// Apply this op to the session.
    pub fn apply(&self, s: &mut Session) -> vectorcraft_engine::Result<Value> {
        let (id, p) = self.command(s);
        s.execute(&id, &p)
    }
}

/// Keep the unused-import lint quiet when only some helpers are used.
#[doc(hidden)]
pub fn _top_level(s: &Session) -> Vec<vectorcraft_doc::NodeId> {
    s.active().map(|d| top_level_ids(&d.doc)).unwrap_or_default()
}

// ---------------------------------------------------------------- junk params

/// Hostile JSON values: wrong types, negative/huge/non-finite-as-null numbers, deep/empty containers.
pub fn junk_values() -> Vec<Value> {
    vec![
        Value::Null,
        json!(true),
        json!(-1),
        json!(0),
        json!(-1e9),
        json!(1e12),
        json!(1e308),
        json!(-1e308),
        json!(f64::MIN_POSITIVE),
        json!(u64::MAX),
        json!(i64::MIN),
        json!(0.5),
        json!(""),
        json!("junk"),
        json!("#zzzzzz"),
        json!([]),
        json!([1]),
        json!([f64::MAX, -f64::MAX]),
        json!([[1, 2], [3]]),
        json!({}),
        json!({"x": "y"}),
        json!([[[[[]]]]]),
    ]
}

/// Every parameter key mentioned in any command's params doc (plus a few common ones).
pub fn all_param_keys() -> Vec<String> {
    let mut keys: std::collections::BTreeSet<String> =
        ["ids", "id", "index", "x", "y", "width", "height", "points", "anchors", "params", "effect", "commands"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    for spec in command_specs() {
        keys.extend(param_keys(spec.params));
    }
    keys.into_iter().collect()
}

/// Keys of a params doc string like `{x, y, width?, radius?: pt} → {id}` (only the part before `→`),
/// with every key the engine reads in it ([`vectorcraft_engine::cmd::params::doc_keys`]: `path | name`,
/// `folder? (…)`).
pub fn param_keys(doc: &str) -> Vec<String> {
    let described = vectorcraft_engine::cmd::params::doc_keys(doc).map(|p| p.keys).unwrap_or_default();
    let doc = doc.split('→').next().unwrap_or("");
    let chars: Vec<char> = doc.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_lowercase() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            // Previous non-space char must open a field; next char must close/type it.
            let before = chars[..start].iter().rev().find(|c| **c != ' ').copied();
            let after = chars.get(i).copied();
            if matches!(before, Some('{') | Some(',') | Some('|'))
                && matches!(after, Some('?') | Some(':') | Some(',') | Some('}') | Some('=') | Some('|'))
            {
                out.push(chars[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    out.extend(described.into_iter().map(str::to_string));
    out.sort();
    out.dedup();
    out
}

/// The keys of command `spec`: those of its params doc ([`param_keys`]), of its schema and its file
/// and folder keys ([`location_keys`]).
pub fn command_param_keys(spec: &CommandSpec) -> Vec<String> {
    let mut keys = param_keys(spec.params);
    keys.extend(spec.schema.into_iter().flat_map(|s| s.params.iter().map(|p| p.name.to_string())));
    keys.extend(location_keys(spec).into_iter().map(|(k, _)| k.to_string()));
    keys.sort();
    keys.dedup();
    keys
}

/// The location to send under the path key `k` ([`key_location`]): `safe_path` for a file, a folder
/// next to it (in the same temp folder) for a folder key.
pub fn safe_location(k: &str, safe_path: &str) -> String {
    location(key_location(k).unwrap_or(Location::File), safe_path)
}

/// The temp location for a key that names `l`: `safe_path` for a file, a folder next to it (in
/// the same temp folder) for a folder.
fn location(l: Location, safe_path: &str) -> String {
    if l == Location::Folder { format!("{safe_path}-folder") } else { safe_path.to_string() }
}

/// `v`, found at `place` in a command's params, with every string in it that can name a place on
/// disk sent to the temp location: `v` itself and the members of arrays in it, at any depth, when
/// `kind` is `Some`, and in an object at any depth, the strings under a key that names a file or a
/// folder there ([`ParamsPlace::member`]: a call or a recorded step inside `v` keeps its command's
/// meaning). Other values keep their junk shape.
fn redirect(place: ParamsPlace, kind: Option<Location>, v: &Value, safe_path: &str) -> Value {
    match v {
        Value::String(_) => kind.map_or_else(|| v.clone(), |l| json!(location(l, safe_path))),
        Value::Array(a) => {
            let step = ParamsPlace::step(a);
            Value::Array(a.iter().enumerate().map(|(i, x)| redirect(place.item(i, step), kind, x, safe_path)).collect())
        }
        Value::Object(o) => {
            let carried = ParamsPlace::carried(o);
            let member = |k: &String, x: &Value| {
                let (below, l) = place.member(k, carried);
                (k.clone(), redirect(below, l, x, safe_path))
            };
            Value::Object(o.iter().map(|(k, x)| member(k, x)).collect())
        }
        _ => v.clone(),
    }
}

/// `v`, sent under key `k` at `place`, as [`redirect`] makes it.
fn redirect_under(place: ParamsPlace, k: &str, v: &Value, safe_path: &str) -> Value {
    let (below, kind) = place.member(k, None);
    redirect(below, kind, v, safe_path)
}

/// `v`, with its strings sent to the temp location when `k` is a path key in a command that gives it
/// no other meaning ([`key_location`]), and the strings under path keys in objects inside it too.
pub fn safe_value(k: &str, v: &Value, safe_path: &str) -> Value {
    redirect_under(ParamsPlace::top(None), k, v, safe_path)
}

/// `v` as [`safe_value`] makes it for command `spec` ([`vectorcraft_engine::cmd::location_in`]): a key that names part of
/// the document in it keeps its strings, at the top level and below.
pub fn safe_value_for_command(spec: &CommandSpec, k: &str, v: &Value, safe_path: &str) -> Value {
    redirect_under(ParamsPlace::top(Some(spec)), k, v, safe_path)
}

/// A deterministic list of junk param objects for `doc` (the command's params doc): each
/// documented key set to each junk value, all keys junk at once, wrong top-level types. A string
/// under a key that names a file or folder in a command that gives it no other meaning
/// ([`key_location`]) is a temp location.
pub fn junk_params(doc: &str, safe_path: &str) -> Vec<Value> {
    junk_params_with(&param_keys(doc), ParamsPlace::top(None), safe_path)
}

/// [`junk_params`] for command `spec`, with the keys of its schema and its file and folder keys too
/// ([`command_param_keys`]), and a string under each key that names a file or folder in it
/// ([`vectorcraft_engine::cmd::location_in`]) a temp location, as are the strings in an array of them and under the path
/// keys of objects in one (`paths: [{ids, path}]`), which it also sends.
pub fn junk_params_for_command(spec: &CommandSpec, safe_path: &str) -> Vec<Value> {
    junk_params_with(&command_param_keys(spec), ParamsPlace::top(Some(spec)), safe_path)
}

/// The junk cases for `keys` at `place`, the top of a command's params: what is sent for a junk value
/// under a key is [`redirect_under`] it, and a key that names a file or a folder there also gets a
/// list of locations and a list of objects with a `path`.
fn junk_params_with(keys: &[String], place: ParamsPlace, safe_path: &str) -> Vec<Value> {
    let junk = junk_values();
    let mut out = vec![Value::Null, json!([]), json!("str"), json!(42), json!({"unknownKey": 1})];
    for k in keys {
        let (below, kind) = place.member(k, None);
        for (i, v) in junk.iter().enumerate() {
            let v = redirect(below, kind, v, safe_path);
            // Sparse: one key at a time with every junk value, plus a few combos.
            let mut o = serde_json::Map::new();
            o.insert(k.clone(), v.clone());
            out.push(Value::Object(o));
            if i % 5 == 0 {
                let mut all = serde_json::Map::new();
                for k2 in keys {
                    let v2 = place.member(k2, None).1.map_or_else(|| v.clone(), |l| json!(location(l, safe_path)));
                    all.insert(k2.clone(), v2);
                }
                out.push(Value::Object(all));
            }
        }
        if let Some(l) = kind {
            for v in [json!(["junk", "", 1]), json!([{"ids": [1], "path": "junk"}, {"path": null}])] {
                out.push(json!({ k.as_str(): redirect(below, Some(l), &v, safe_path) }));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strings the sweeps send keep the meaning the command gives their key, at every depth: an
    /// export's folder grouping and a web link stay as sent, also inside a `command.batch` step and
    /// a recorded step, and a real file or folder key there goes to the temp location. A key that names
    /// part of the document in a recorded step's command stays, and the same key in a command that
    /// doesn't take it as a param names a file.
    #[test]
    fn nested_locations_keep_command_meaning() {
        let spec = |id: &str| command_specs().iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no command {id}"));
        let safe = "/tmp/sweep/out.bin";
        let settings = json!({"folders": "rows", "formats": [{"format": "png", "folder": "junk"}]});
        assert_eq!(
            safe_value_for_command(spec("document.exportPresets.save"), "settings", &settings, safe),
            json!({"folders": "rows", "formats": [{"format": "png", "folder": "/tmp/sweep/out.bin-folder"}]})
        );
        let steps = json!([
            {"command": "attributes.set", "params": {"url": "https://example.com/a"}},
            {"command": "document.save", "params": {"path": "junk"}},
            {"command": "object.path.simplify", "params": {"wholePaths": "junk"}},
        ]);
        assert_eq!(
            safe_value_for_command(spec("command.batch"), "commands", &steps, safe),
            json!([
                {"command": "attributes.set", "params": {"url": "https://example.com/a"}},
                {"command": "document.save", "params": {"path": safe}},
                {"command": "object.path.simplify", "params": {"wholePaths": "junk"}},
            ])
        );
        let recorded = json!([
            ["text.createInPath", {"path": "junk"}],
            ["object.setLiveShape", {"path": "junk"}],
            ["document.compare", {"diffPath": "junk"}],
        ]);
        assert_eq!(
            safe_value("steps", &recorded, safe),
            json!([["text.createInPath", {"path": "junk"}], ["object.setLiveShape", {"path": safe}], ["document.compare", {"diffPath": safe}]])
        );
        assert_eq!(safe_value("url", &json!("https://example.com/a"), safe), json!("https://example.com/a"));
        assert_eq!(safe_value("destinationFolder", &json!("junk"), safe), json!("/tmp/sweep/out.bin-folder"));
    }
}
