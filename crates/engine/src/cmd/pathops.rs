//! Pathfinder panel (`object.pathfinder.*`) and Object → Path commands backed by
//! `vectorcraft-pathops`: Offset Path, Outline Stroke, Simplify, Add Anchor Points, Split Into Grid,
//! Clean Up and Divide Objects Below.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::appearance::{AppearanceItem, StrokeLayer};
use vectorcraft_doc::{AnchorRef, Appearance, Document, Node, NodeId, NodeKind, SegmentRef, TextObject};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops as po;
use vectorcraft_pathops::{BoolOp, PathfinderOp};

use super::edit::selected_roots;
use super::{
    CommandSpec, EngineError, Result, Session, always, bad, bool_or, checked_ids_or_selection, cmd, has_doc, has_multi, has_selection, number_or,
    str_param, switch_or,
};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.pathfinder.unite",
            "Unite",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Unite)
        ),
        cmd!(
            "object.pathfinder.minusFront",
            "Minus Front",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::MinusFront)
        ),
        cmd!(
            "object.pathfinder.intersect",
            "Intersect",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Intersect)
        ),
        cmd!(
            "object.pathfinder.exclude",
            "Exclude",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Exclude)
        ),
        cmd!(
            "object.pathfinder.divide",
            "Divide",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Divide)
        ),
        cmd!(
            "object.pathfinder.trim",
            "Trim",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Trim)
        ),
        cmd!(
            "object.pathfinder.merge",
            "Merge",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Merge)
        ),
        cmd!(
            "object.pathfinder.crop",
            "Crop",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_multi,
            |s, p| run_pf(s, p, PathfinderOp::Crop)
        ),
        cmd!(
            "object.pathfinder.outline",
            "Outline",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::Outline)
        ),
        cmd!(
            "object.pathfinder.minusBack",
            "Minus Back",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), precision?: pt 0.001..10, removeRedundantPoints?: bool, removeUnpainted?: bool (default: the Pathfinder Options, see object.pathfinder.options; the values used join the journal entry)} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, p| run_pf(s, p, PathfinderOp::MinusBack)
        ),
        cmd!(
            "object.pathfinder.trap",
            "Trap…",
            ["Window", "Pathfinder"],
            None,
            "{ids?: [id…] (selected first; default: the selection), thickness?: pt 0.01..5000 (0.25; how far the lighter colour reaches into the darker one across vertical edges), heightWidth?: % 1..1000 (100; the reach across horizontal edges as a percentage of the vertical one), tintReduction?: % 0..100 (40; the trap paints the lighter colour at this percentage of its tint), trapsWithProcessColor?: bool (false; a swatch (spot) colour's trap is painted with its CMYK process equivalent), reverseTraps?: bool (false; the darker colour spreads into the lighter one, except a rich black)} adds overprinting traps where the selected objects' visible areas of different solid colours meet, grouped directly above the topmost selected object and selected; the objects stay as they are → {ids}",
            has_selection,
            trap
        ),
        cmd!(
            "object.pathfinder.options",
            "Pathfinder Options…",
            ["Window", "Pathfinder"],
            None,
            "{precision?: pt 0.001..10 (0.01; how closely the Pathfinder operations follow the paths: results are refit to within it and pieces thinner than it are dropped), removeRedundantPoints?: bool (false; the results lose the anchors that don't change their shape), removeUnpainted?: bool (false; Divide and Outline leave out the pieces of objects with no fill)} Pathfinder Options, kept in the preferences and used by every later Shape Mode and Pathfinder operation (not Trap); {} reads them → the options",
            always,
            pathfinder_options
        ),
        cmd!(
            "object.path.outlineStroke",
            "Outline Stroke",
            ["Object", "Path"],
            None,
            "{} replace stroked paths with filled outlines → {ids}",
            has_selection,
            outline_stroke
        ),
        cmd!(
            "object.path.offsetPath",
            "Offset Path…",
            ["Object", "Path"],
            None,
            "{offset: pt (negative insets), joins?: \"miter\"|\"round\"|\"bevel\", miterLimit?: 4} → {ids} (new objects above the originals)",
            has_selection,
            offset_path
        ),
        cmd!(
            "object.path.simplify",
            "Simplify…",
            ["Object", "Path"],
            None,
            "{tolerance?: pt (1), level?: 0..100 (the Simplify slider, in place of tolerance: 0 fewest anchor points, 50 is 1 pt, every 10 levels halves or doubles it), auto?: bool (Auto-Simplify: the lowest level whose tolerance is at most 1/400 of the selection's longer side, or 100 when that side is under 12.5 pt), cornerAngle?: deg (30), straightLines?: bool, wholePaths?: bool (false: a path with segments direct-selected, or between direct-selected anchors, simplifies only each run of them between its end anchors, which stay as they are, and keeps the runs selected; true simplifies those paths whole)} → {ids, before, after} anchor counts, and the level used with level or auto",
            has_selection,
            simplify
        ),
        cmd!(
            "object.path.smooth",
            "Smooth…",
            ["Object", "Path"],
            None,
            "{level?: 0..100 (50; the Smooth slider: 0 minimum smoothing, the path staying within 0.1% of its longer side, 100 maximum: refitted with curves within up to 2.5% of it, corners kept only where the path turns back, never with more anchor points), auto?: bool (Auto-Smooth: level 50), ids?: [id…] (default: the selection)} smooth each path under the objects, compound path members included, as one undo step → {ids, before, after} anchor counts, and the level used",
            has_selection,
            smooth
        ),
        cmd!("object.path.addAnchorPoints", "Add Anchor Points", ["Object", "Path"], None, "{} → {ids}", has_selection, add_anchor_points),
        cmd!(
            "object.path.divideObjectsBelow",
            "Divide Objects Below",
            ["Object", "Path"],
            None,
            "{} cut the objects below with the selected path (which is removed) → {ids}",
            has_selection,
            divide_objects_below
        ),
        cmd!(
            "object.path.splitIntoGrid",
            "Split Into Grid…",
            ["Object", "Path"],
            None,
            "{rows?: 2, columns?: 2, gutter?: pt (12; rowGutter?, columnGutter?: one of the two), totalHeight?, totalWidth?: pt (the grid's size; the selected paths' when left out), height?, width?: pt (each row's and column's size, which then sets the total), addGuides?: false (ruler guides along the row and column edges)} one grid of rectangles over the selected paths replaces them, painted as the topmost one → {ids, guides: count}",
            has_selection,
            split_into_grid
        ),
        cmd!(
            "object.path.cleanUp",
            "Clean Up…",
            ["Object", "Path"],
            None,
            "{strayPoints?: true, unpaintedObjects?: true, emptyTextPaths?: true} → {removed}",
            has_doc,
            clean_up
        ),
    ]
}

// ---------- helpers ----------

/// Number param that also accepts strings like `"10 pt"`.
pub(crate) fn num_param(p: &Value, key: &str) -> Option<f64> {
    match p.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')).collect();
            t.parse().ok()
        }
        _ => None,
    }
    .filter(|v: &f64| v.is_finite())
}

/// A distance param in points: a number, or a string with a unit (`"5 mm"`; bare numbers are
/// points).
pub(crate) fn len_param(p: &Value, key: &str) -> Option<f64> {
    match p.get(key)? {
        Value::String(s) => vectorcraft_doc::Unit::Points.parse(s).filter(|v| v.is_finite()).or_else(|| num_param(p, key)),
        _ => num_param(p, key),
    }
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Paint identity carried through a Pathfinder operation.
#[derive(Clone, Debug, PartialEq)]
struct Style {
    appearance: Appearance,
    opacity: f32,
    blend: BlendMode,
}

impl Style {
    fn of(n: &Node) -> Self {
        Style { appearance: (*n.appearance).clone(), opacity: n.opacity, blend: n.blend }
    }
    fn apply(&self, n: &mut Node) {
        n.appearance = self.appearance.clone().into();
        n.opacity = self.opacity;
        n.blend = self.blend;
    }
}

/// All subpaths of a path or compound path node, in document coordinates, with its fill rule.
pub(crate) fn node_path(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            let mut subs = vec![];
            for c in children {
                if let Some((p, _)) = node_path(c) {
                    subs.extend(p.subpaths);
                }
            }
            Some((PathData::new(subs), *rule))
        }
        NodeKind::Text(t) => Some((text_outline(t), FillRule::NonZero)),
        _ => None,
    }
}

/// Glyph outlines of a text object in document coordinates.
pub(crate) fn text_outline(t: &TextObject) -> PathData {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    PathData::from_bezpath(&lay.to_bezpath()).transformed(t.xf)
}

fn style_of(n: &Node) -> Style {
    match &n.kind {
        NodeKind::Text(t) => Style { appearance: t.first_style().appearance(), opacity: n.opacity, blend: n.blend },
        _ => Style::of(n),
    }
}

/// Leaf shapes (paths, compound paths, text) of a subtree in paint order; guides and clipping
/// paths are skipped.
fn leaves(n: &Node, out: &mut Vec<Node>) {
    each_leaf(n, &mut |l| out.push(l.clone()));
}

/// Calls `f` with each leaf shape of [`leaves`], in its order.
fn each_leaf(n: &Node, f: &mut impl FnMut(&Node)) {
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_) => f(n),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                each_leaf(c, f);
            }
        }
        _ => {}
    }
}

/// How many shapes a Pathfinder operation takes from `n` selected alone: its leaf shapes, however
/// deep its groups nest (an expanded stroke is a group inside the group, #1635).
pub fn shape_count(n: &Node) -> usize {
    // Counted without copying the shapes: the panel asks every frame.
    let mut count = 0;
    each_leaf(n, &mut |_| count += 1);
    count
}

/// A node for `path`: a plain path for one subpath, else a compound path.
pub(crate) fn shape_node(d: &mut Document, path: PathData, style_src: Option<&Node>) -> Node {
    let id = d.alloc_id();
    let mut n = if path.subpaths.len() <= 1 {
        Node::path(id, path, Appearance::default())
    } else {
        let children = path
            .subpaths
            .into_iter()
            .map(|sp| {
                let cid = d.alloc_id();
                Arc::new(Node::path(cid, PathData::single(sp), Appearance::default()))
            })
            .collect();
        Node::new(id, NodeKind::Compound { children, rule: FillRule::NonZero })
    };
    if let Some(src) = style_src {
        Style::of(src).apply(&mut n);
    }
    n
}

fn shape_node_styled(d: &mut Document, path: PathData, style: &Style) -> Node {
    let mut n = shape_node(d, path, None);
    style.apply(&mut n);
    n
}

/// Whether `ids` are separate objects, as [`Document::remove_many`] and [`Document::insert_beside`]
/// take them: each one is in the tree and listed once, and none lies inside another listed one.
pub(crate) fn separate(d: &Document, ids: &[NodeId]) -> bool {
    let paths = d.index_paths(ids.iter().copied());
    let mut found: Vec<_> = ids.iter().filter_map(|id| paths.get(id)).collect();
    found.sort_unstable();
    found.len() == ids.len() && found.windows(2).all(|w| !w[1].starts_with(w[0]))
}

/// Lists of nodes that go in place of objects, or directly above them; an empty list in place of
/// an object removes it. When the objects are [`separate`], [`Beside::put`] holds each list and
/// leaves the tree as it is, and [`Beside::finish`] inserts them all in one pass over the children
/// of each parent ([`Document::insert_beside`]). Otherwise [`Beside::put`] inserts each list at
/// once, where its object is at that moment.
pub(crate) struct Beside {
    replace: bool,
    waiting: Option<Vec<(NodeId, Vec<Node>)>>,
}

impl Beside {
    /// Lists that go in place of objects among `ids`.
    pub(crate) fn replacing(d: &Document, ids: &[NodeId]) -> Self {
        Beside { replace: true, waiting: separate(d, ids).then(Vec::new) }
    }

    /// Lists that go directly above objects among `ids`.
    pub(crate) fn above(d: &Document, ids: &[NodeId]) -> Self {
        Beside { replace: false, waiting: separate(d, ids).then(Vec::new) }
    }

    /// Put `nodes`, in their order, in place of object `id` or directly above it.
    pub(crate) fn put(&mut self, d: &mut Document, id: NodeId, nodes: Vec<Node>) -> Result<()> {
        if let Some(waiting) = &mut self.waiting {
            waiting.push((id, nodes));
            return Ok(());
        }
        let (parent, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
        let at = if self.replace {
            d.remove(id)?;
            idx
        } else {
            idx + 1
        };
        for (k, n) in nodes.into_iter().enumerate() {
            d.insert(parent, at + k, n)?;
        }
        Ok(())
    }

    /// Insert the lists that [`Beside::put`] held.
    pub(crate) fn finish(self, d: &mut Document) -> Result<()> {
        match self.waiting {
            Some(waiting) if !waiting.is_empty() => Ok(d.insert_beside(waiting, self.replace)?),
            _ => Ok(()),
        }
    }
}

/// Remove `ids` through a [`Beside`], which removes them in one pass when they are [`separate`].
pub(crate) fn remove_each(d: &mut Document, ids: &[NodeId]) -> Result<()> {
    let mut gone = Beside::replacing(d, ids);
    for id in ids {
        gone.put(d, *id, vec![])?;
    }
    gone.finish(d)
}

// ---------- Pathfinder ----------

/// Pathfinder panel › Pathfinder Options (`object.pathfinder.options`, the `pathfinder` preference
/// group), which every Shape Mode and Pathfinder operation of the panel uses. The defaults leave
/// the results as they were before the options existed.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PathfinderOptions {
    /// How closely results follow the paths (pt): they are refit to within it and pieces thinner
    /// than it are dropped ([`po::pathfinder_with`]).
    pub precision: f64,
    /// Results lose the anchors that don't change their shape (to within `precision`).
    pub remove_redundant_points: bool,
    /// Divide and Outline leave out the pieces of objects with no fill.
    pub remove_unpainted: bool,
}

impl Default for PathfinderOptions {
    fn default() -> Self {
        Self { precision: po::DEFAULT_PRECISION, remove_redundant_points: false, remove_unpainted: false }
    }
}

/// The range of Pathfinder Options › Precision (pt).
pub const PATHFINDER_PRECISION_RANGE: std::ops::RangeInclusive<f64> = 0.001..=10.0;

impl PathfinderOptions {
    /// These options with those `p` names (`precision` in pt or a string with a unit), or why `p`
    /// can't set them.
    pub(crate) fn merged(&self, p: &Value) -> std::result::Result<Self, String> {
        let given = match p {
            Value::Null => return Ok(*self),
            Value::Object(given) => given,
            _ => return Err("`pathfinder` must be an object".into()),
        };
        let mut o = *self;
        for (key, v) in given {
            let flag = || v.as_bool().ok_or_else(|| format!("`{key}` must be true or false"));
            match key.as_str() {
                "precision" => {
                    o.precision =
                        len_param(p, key).filter(|v| PATHFINDER_PRECISION_RANGE.contains(v)).ok_or("precision must be from 0.001 to 10 pt")?;
                }
                "removeRedundantPoints" => o.remove_redundant_points = flag()?,
                "removeUnpainted" => o.remove_unpainted = flag()?,
                k => {
                    let path = "pathfinder";
                    return Err(format!("unknown option `{path}.{k}`"));
                }
            }
        }
        Ok(o)
    }
}

fn pathfinder_options(s: &mut Session, p: &Value) -> Result<Value> {
    s.prefs.pathfinder = s.prefs.pathfinder.merged(p).map_err(|e| bad("object.pathfinder.options", e))?;
    Ok(json!(s.prefs.pathfinder))
}

/// The last part of the id of `op`'s command (`object.pathfinder.…`).
fn pf_id(op: PathfinderOp) -> &'static str {
    match op {
        PathfinderOp::Unite => "unite",
        PathfinderOp::MinusFront => "minusFront",
        PathfinderOp::Intersect => "intersect",
        PathfinderOp::Exclude => "exclude",
        PathfinderOp::Divide => "divide",
        PathfinderOp::Trim => "trim",
        PathfinderOp::Merge => "merge",
        PathfinderOp::Crop => "crop",
        PathfinderOp::Outline => "outline",
        PathfinderOp::MinusBack => "minusBack",
        PathfinderOp::HardMix => "hardMix",
        PathfinderOp::SoftMix => "softMix",
    }
}

fn pf_label(op: PathfinderOp) -> &'static str {
    match op {
        PathfinderOp::Unite => "Unite",
        PathfinderOp::MinusFront => "Minus Front",
        PathfinderOp::Intersect => "Intersect",
        PathfinderOp::Exclude => "Exclude",
        PathfinderOp::Divide => "Divide",
        PathfinderOp::Trim => "Trim",
        PathfinderOp::Merge => "Merge",
        PathfinderOp::Crop => "Crop",
        PathfinderOp::Outline => "Outline",
        PathfinderOp::MinusBack => "Minus Back",
        PathfinderOp::HardMix => "Hard Mix",
        PathfinderOp::SoftMix => "Soft Mix",
    }
}

/// The Pathfinder Options an operation uses: those `p` gives, else the preferences'. They join the
/// command's journal entry, so replaying it gives the same result whatever the preferences are then.
fn pf_options(s: &mut Session, p: &Value, id: &str) -> Result<PathfinderOptions> {
    let mut stored = s.prefs.pathfinder;
    if !PATHFINDER_PRECISION_RANGE.contains(&stored.precision) {
        // A preferences file edited by hand: the default, rather than refitting everything away.
        stored.precision = po::DEFAULT_PRECISION;
    }
    let keys = ["precision", "removeRedundantPoints", "removeUnpainted"];
    let given: serde_json::Map<String, Value> =
        p.as_object().into_iter().flatten().filter(|(k, _)| keys.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    let options = stored.merged(&Value::Object(given)).map_err(|e| bad(id, e))?;
    s.note_journal("precision", json!(options.precision));
    s.note_journal("removeRedundantPoints", json!(options.remove_redundant_points));
    s.note_journal("removeUnpainted", json!(options.remove_unpainted));
    Ok(options)
}

fn run_pf(s: &mut Session, p: &Value, op: PathfinderOp) -> Result<Value> {
    let roots = selected_roots(s)?;
    if roots.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let shape_mode =
        matches!(op, PathfinderOp::Unite | PathfinderOp::MinusFront | PathfinderOp::Intersect | PathfinderOp::Exclude | PathfinderOp::MinusBack);
    let label = pf_label(op);
    let options = pf_options(s, p, &format!("object.pathfinder.{}", pf_id(op)))?;
    let ids = s.edit(label, |d, sel| {
        // Build the stack (back → front); names are separate from paint/style identity.
        let mut styles: Vec<Style> = vec![];
        let mut shapes: Vec<po::Shape> = vec![];
        let mut names: Vec<Option<String>> = vec![];
        let mut push = |mut path: PathData, rule: FillRule, mut st: Style, name: Option<String>| {
            if path.is_empty() {
                return;
            }
            // Trim, Merge and Crop remove any strokes (and Merge then merges by fill alone), as the
            // public Pathfinder documentation describes.
            if matches!(op, PathfinderOp::Trim | PathfinderOp::Merge | PathfinderOp::Crop) {
                st.appearance.items.retain(|i| !matches!(i, AppearanceItem::Stroke(_)));
            }
            if op == PathfinderOp::Divide && st.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if !f.paint.is_none())) {
                // Filled open contours keep their implicit closing edge. A zero-area line
                // stays a cutter; normalised area also catches zero-signed-area self crossings.
                for sub in path.subpaths.iter_mut().filter(|sp| !sp.closed) {
                    sub.closed = po::encloses_area(sub) || po::area(&PathData::single(sub.clone()), rule) > 0.0;
                }
            }
            let key = match styles.iter().position(|x| *x == st) {
                Some(k) => k,
                None => {
                    styles.push(st);
                    styles.len() - 1
                }
            };
            shapes.push(po::Shape::new(path, rule, key as u64));
            names.push(name);
        };
        for id in &roots {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            let mut lv = vec![];
            leaves(n, &mut lv);
            if lv.is_empty() {
                continue;
            }
            if shape_mode
                && roots.len() > 1
                && lv.len() > 1
                && let Some(top) = lv.last()
            {
                // Among other objects, a group acts as one shape (the union of its leaves) with
                // its top leaf's style. Selected alone, its members are the shapes (#1450).
                let parts: Vec<(PathData, FillRule)> = lv.iter().filter_map(node_path).collect();
                let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
                let name = n.name.clone().or_else(|| lv.iter().find_map(|l| l.name.clone()));
                push(
                    po::try_unite_all_with(&refs, options.precision).map_err(|e| EngineError::Other(e.to_string()))?,
                    FillRule::NonZero,
                    style_of(top),
                    name,
                );
            } else {
                for l in &lv {
                    if let Some((p, r)) = node_path(l) {
                        push(p, r, style_of(l), n.name.clone().or_else(|| l.name.clone()));
                    }
                }
            }
        }
        if shapes.is_empty() {
            return Err(EngineError::Other(format!("{label}: select paths, compound paths, groups or text")));
        }
        if shapes.iter().any(|s| s.path.subpaths.iter().any(|sp| sp.anchors.iter().any(|a| !(a.p.x.is_finite() && a.p.y.is_finite())))) {
            return Err(EngineError::Other("paths contain invalid coordinates".into()));
        }
        let mut results = po::pathfinder_with(op, &shapes, options.precision);
        if options.remove_unpainted && matches!(op, PathfinderOp::Divide | PathfinderOp::Outline) {
            // Pathfinder Options › Divide and Outline Will Remove Unpainted Artwork: the pieces
            // of objects with no fill (or only hidden ones) go.
            let filled = |st: &Style| st.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !f.paint.is_none()));
            results.retain(|r| styles.get(r.key as usize).is_some_and(filled));
        }
        if options.remove_redundant_points {
            for r in &mut results {
                r.path = po::remove_redundant_points(&r.path, options.precision);
            }
        }
        if results.iter().all(|r| r.path.is_empty()) {
            return Err(EngineError::Other(format!("{label} produced an empty result")));
        }
        let new_node = if shape_mode {
            let r =
                results.into_iter().find(|r| !r.path.is_empty()).ok_or_else(|| EngineError::Other(format!("{label} produced an empty result")))?;
            let style = styles.get(r.key as usize).ok_or_else(|| EngineError::Other(format!("{label}: lost track of a shape's style")))?;
            let mut n = shape_node_styled(d, r.path, style);
            n.name = match op {
                // A subtraction keeps its surviving source's identity, even when unnamed.
                PathfinderOp::MinusFront => names.first().cloned().flatten(),
                PathfinderOp::MinusBack => names.last().cloned().flatten(),
                _ => names.into_iter().flatten().next(),
            };
            n
        } else {
            let mut children = vec![];
            for r in results {
                if r.path.is_empty() {
                    continue;
                }
                let st = &styles[r.key as usize];
                let mut n = shape_node_styled(d, r.path, st);
                if op == PathfinderOp::Outline {
                    // Outline: open edges stroked with the face's former fill.
                    let fill = st.appearance.fill_paint();
                    let paint = if fill.is_none() { st.appearance.stroke_paint() } else { fill };
                    n.appearance = Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, 1.0))], ..Default::default() }.into();
                }
                children.push(Arc::new(n));
            }
            let gid = d.alloc_id();
            Node::group(gid, children)
        };
        let nid = d.wrap(&roots, |_| new_node)?;
        sel.set([nid]);
        Ok(vec![nid])
    })?;
    Ok(ids_json(&ids))
}

// ---------- Trap ----------

/// Pathfinder panel › Trap ([`vectorcraft_render::effects::trap`]): the traps between the selected
/// objects' leaf shapes, as one group of overprinting paths above the topmost selected object.
fn trap(s: &mut Session, p: &Value) -> Result<Value> {
    use vectorcraft_render::effects::trap::{TRAP_LABEL, TrapOptions, TrapSource, trap_appearance, traps};
    const ID: &str = "object.pathfinder.trap";
    let mut q = p.clone();
    if let Some(t) = len_param(p, "thickness") {
        if !(po::TRAP_MIN_WIDTH..=po::TRAP_MAX_WIDTH).contains(&t) {
            return Err(bad(ID, "thickness must be from 0.01 to 5000 pt"));
        }
        q["thickness"] = json!(t);
    }
    if num_param(p, "heightWidth").is_some_and(|v| !(1.0..=1000.0).contains(&v)) {
        return Err(bad(ID, "heightWidth must be from 1 to 1000 %"));
    }
    if num_param(p, "tintReduction").is_some_and(|v| !(0.0..=100.0).contains(&v)) {
        return Err(bad(ID, "tintReduction must be from 0 to 100 %"));
    }
    // The effect-only settings stay at their defaults.
    if let Some(o) = q.as_object_mut() {
        o.remove("precision");
        o.remove("removeRedundantPoints");
    }
    let options = TrapOptions::from_params(&q);
    let roots = selected_roots(s)?;
    if roots.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let ids = s.edit(TRAP_LABEL, |d, sel| {
        let mut sources = vec![];
        for id in &roots {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            each_leaf(n, &mut |l| {
                if let Some((path, rule)) = node_path(l) {
                    sources.push(TrapSource { path, rule, fill: style_of(l).appearance.fill_paint() });
                }
            });
        }
        if sources.iter().filter(|s| s.fill.color().is_some()).count() < 2 {
            return Err(EngineError::Other("Trap: select two or more objects filled with solid colours".into()));
        }
        let made = traps(&sources, &options);
        if made.is_empty() {
            return Err(EngineError::Other("Trap: no edges of different colours meet among the selected objects".into()));
        }
        let mut children = Vec::with_capacity(made.len());
        for piece in made {
            let mut n = shape_node(d, piece.path, None);
            n.appearance = trap_appearance(piece.paint).into();
            children.push(Arc::new(n));
        }
        let gid = d.alloc_id();
        let top = *roots.last().ok_or_else(|| EngineError::Other("nothing selected".into()))?;
        d.insert_beside(vec![(top, vec![Node::group(gid, children)])], false)?;
        sel.set([gid]);
        Ok(vec![gid])
    })?;
    Ok(ids_json(&ids))
}

// ---------- Offset Path ----------

fn join_param(p: &Value, key: &str) -> po::Join {
    match str_param(p, key).map(str::to_ascii_lowercase).as_deref() {
        Some("round") => po::Join::Round,
        Some("bevel") => po::Join::Bevel,
        _ => po::Join::Miter,
    }
}

fn offset_path(s: &mut Session, p: &Value) -> Result<Value> {
    let delta = len_param(p, "offset").unwrap_or(10.0);
    if delta.abs() > 1e5 {
        return Err(bad("object.path.offsetPath", "offset out of range"));
    }
    let join = join_param(p, "joins");
    let miter = num_param(p, "miterLimit").unwrap_or(4.0).clamp(1.0, 500.0);
    let roots = selected_roots(s)?;
    let ids = s.edit("Offset Path", |d, sel| {
        let (mut out, mut above) = (vec![], vec![]);
        let mut paths = false;
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            let mut lv = vec![];
            leaves(&n, &mut lv);
            let mut made = vec![];
            for l in lv.iter().filter(|l| !matches!(l.kind, NodeKind::Text(_))) {
                let Some((path, rule)) = node_path(l) else { continue };
                paths = true;
                let base = if matches!(l.kind, NodeKind::Compound { .. }) && path.is_closed() { po::normalize(&path, rule) } else { path };
                let off = po::offset_path(&base, delta, join, miter);
                if off.is_empty() {
                    continue;
                }
                made.push(shape_node(d, off, Some(l)));
            }
            if made.is_empty() {
                continue;
            }
            let new = if made.len() == 1
                && !matches!(n.kind, NodeKind::Group { .. })
                && let Some(one) = made.pop()
            {
                one
            } else {
                let gid = d.alloc_id();
                Node::group(gid, made.into_iter().map(Arc::new).collect())
            };
            out.push(new.id);
            above.push((*id, vec![new]));
        }
        if out.is_empty() && paths {
            return Err(EngineError::Other("Offset Path: the offset leaves nothing of the selected paths (an inset larger than the shapes)".into()));
        }
        if out.is_empty() {
            return Err(EngineError::Other("Offset Path: select paths".into()));
        }
        d.insert_beside(above, false)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

// ---------- Outline Stroke ----------

/// Can Outline Stroke turn `st` into filled art?
fn outlinable(st: &StrokeLayer) -> bool {
    st.visible && !st.paint.is_none() && st.width > 0.0
}

/// `n` with `opacity` and `blend` applied: set on it when it has none of its own, else on a
/// group around it.
pub(crate) fn with_transparency(d: &mut Document, mut n: Node, opacity: f32, blend: BlendMode) -> Node {
    if opacity >= 1.0 && blend == BlendMode::Normal {
        return n;
    }
    if n.opacity < 1.0 || n.blend != BlendMode::Normal {
        n = Node::group(d.alloc_id(), vec![Arc::new(n)]);
    }
    n.opacity = opacity;
    n.blend = blend;
    n
}

/// The art stroke `st` of a path (`path`, `rule`) paints, as fills: its brush art (with the
/// stroke's opacity, blend mode and raster effects), or the outline of the stroke (as on the canvas)
/// filled with the stroke's paint, opacity, blend mode and effects (a gradient along or across the stroke: gradient meshes in a clip group shaped
/// like the outline). `None` when it paints nothing.
pub(crate) fn outlined_stroke(
    d: &mut Document,
    brushes: &[vectorcraft_brush::Brush],
    path: &PathData,
    rule: FillRule,
    st: &StrokeLayer,
) -> Option<Node> {
    if let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) {
        let mut pieces = vectorcraft_brush::stroke_pieces(d, b, &path.to_bezpath(), st);
        let art = match pieces.len() {
            0 => return None,
            1 => pieces.remove(0),
            _ => Node::group(NodeId(0), pieces.into_iter().map(Arc::new).collect()),
        };
        let art = d.reid(&art);
        let mut art = with_transparency(d, art, st.opacity, st.blend);
        let raster: Vec<_> = st.effects.iter().filter(|e| !vectorcraft_render::effects::is_geometry(&e.id)).cloned().collect();
        if !raster.is_empty() {
            if !art.appearance.effects.is_empty() {
                art = Node::group(d.alloc_id(), vec![Arc::new(art)]);
            }
            art.appearance.effects = raster;
        }
        return Some(art);
    }
    let outline = vectorcraft_render::effects::stroke::outline_region(path, rule, st);
    if outline.is_empty() {
        return None;
    }
    if let Some(g) = st.path_gradient() {
        // A gradient along or across the stroke: gradient meshes clipped to its outline.
        let meshes = vectorcraft_render::effects::stroke::gradient_meshes(&path.to_bezpath(), rule, st, &g.gradient);
        let mut clip = shape_node(d, outline, None);
        super::object::as_clipping_path(&mut clip).ok()?;
        let meshes: Vec<Node> = meshes.into_iter().map(|m| Node::new(d.alloc_id(), NodeKind::Mesh(m))).collect();
        let children = std::iter::once(clip).chain(meshes).map(Arc::new).collect();
        let group = Node::new(d.alloc_id(), NodeKind::Group { children, clip: true });
        let mut art = with_transparency(d, group, st.opacity, st.blend);
        art.appearance.effects = st.effects.clone();
        return Some(art);
    }
    let mut n = shape_node(d, outline, None);
    let fill = vectorcraft_doc::appearance::FillLayer {
        opacity: st.opacity,
        blend: st.blend,
        effects: st.effects.clone(),
        overprint: st.overprint,
        ..vectorcraft_doc::appearance::FillLayer::new(st.paint.stroke_as_fill())
    };
    n.appearance = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() }.into();
    Some(n)
}

/// Path `l` with every stroke outlined, in appearance order: runs of fills stay on copies of
/// the path, each stroke becomes its outline (or brush art), grouped when there are several.
/// `None` when `l` has no stroke to outline.
pub(crate) fn outline_strokes(d: &mut Document, brushes: &[vectorcraft_brush::Brush], l: &Node) -> Option<Node> {
    if !l.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if outlinable(st))) {
        return None;
    }
    let (path, rule) = node_path(l)?;
    let mut parts: Vec<Node> = vec![];
    let mut fills: Vec<AppearanceItem> = vec![];
    for item in &l.appearance.items {
        match item {
            AppearanceItem::Fill(f) if f.visible && !f.paint.is_none() => fills.push(item.clone()),
            AppearanceItem::Stroke(st) if outlinable(st) => {
                if !fills.is_empty() {
                    parts.push(styled_copy(d, l, std::mem::take(&mut fills)));
                }
                parts.extend(outlined_stroke(d, brushes, &path, rule, st));
            }
            _ => {}
        }
    }
    if !fills.is_empty() {
        parts.push(styled_copy(d, l, fills));
    }
    assemble(d, l, parts)
}

/// A plain copy of path `l` (new ids, no transparency, name or mask) painted with `items`.
pub(crate) fn styled_copy(d: &mut Document, l: &Node, items: Vec<AppearanceItem>) -> Node {
    let mut f = d.reid(l);
    f.appearance = Appearance { items, ..Default::default() }.into();
    f.opacity = 1.0;
    f.blend = BlendMode::Normal;
    f.name = None;
    f.extras_mut().mask = None;
    if let NodeKind::Path { live, .. } = &mut f.kind {
        *live = None;
    }
    f
}

/// What stands for object `l` once its appearance became `parts` (paint order): the one part (in a
/// group when it has effects of its own) or a group of them, with `l`'s transparency, name, opacity
/// mask and effects. `None` without parts.
pub(crate) fn assemble(d: &mut Document, l: &Node, mut parts: Vec<Node>) -> Option<Node> {
    let root = match parts.len() {
        0 => return None,
        1 if parts[0].appearance.effects.is_empty() => parts.remove(0),
        _ => Node::group(d.alloc_id(), parts.into_iter().map(Arc::new).collect()),
    };
    let mut root = with_transparency(d, root, l.opacity, l.blend);
    root.name = l.name.clone();
    root.extras_mut().mask = l.extras().mask.clone();
    root.appearance.effects = l.appearance.effects.clone();
    Some(root)
}

/// Outline the strokes of every path under `root` in place: → (the id now standing for `root`,
/// how many paths changed).
pub(crate) fn outline_strokes_under(d: &mut Document, brushes: &[vectorcraft_brush::Brush], root: NodeId) -> Result<(NodeId, usize)> {
    replace_leaves(d, root, |d, l| if matches!(l.kind, NodeKind::Text(_)) { None } else { outline_strokes(d, brushes, l) })
}

/// Replace each leaf shape under `root` ([`leaves`]) by what `f` makes of it (`None` keeps it):
/// → (the id now standing for `root`, how many leaves changed).
pub(crate) fn replace_leaves(d: &mut Document, root: NodeId, mut f: impl FnMut(&mut Document, &Node) -> Option<Node>) -> Result<(NodeId, usize)> {
    let Some(n) = d.node(root).cloned() else { return Ok((root, 0)) };
    let mut lv = vec![];
    leaves(&n, &mut lv);
    let (mut replacement, mut changed) = (root, 0);
    for l in lv {
        let Some(new) = f(d, &l) else { continue };
        let nid = new.id;
        *d.node_mut(l.id).ok_or(EngineError::NoNode(l.id))? = new;
        changed += 1;
        if l.id == root {
            replacement = nid;
        }
    }
    Ok((replacement, changed))
}

fn outline_stroke(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let ids = s.edit("Outline Stroke", |d, sel| {
        let brushes = vectorcraft_brush::library(d);
        let mut new_sel = vec![];
        let mut changed = 0;
        for root in &roots {
            let (id, n) = outline_strokes_under(d, &brushes, *root)?;
            new_sel.push(id);
            changed += n;
        }
        if changed == 0 {
            return Err(EngineError::Other("Outline Stroke: no stroked paths selected".into()));
        }
        sel.set(new_sel.iter().copied());
        Ok(new_sel)
    })?;
    Ok(ids_json(&ids))
}

// ---------- Simplify / Add Anchor Points ----------

/// Ids of every plain path node under the selected roots (compound children included).
fn path_ids(d: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for r in roots {
        if let Some(n) = d.node(*r) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::Path { guide: false, .. }) {
                    out.push(c.id)
                }
            });
        }
    }
    out
}

/// The direct selection a path keeps after an edit of part of it: (its selected anchors, and its
/// selected segments when they were picked as segments). `None` selects the path whole.
type PartSelection = Option<(BTreeSet<AnchorRef>, Option<BTreeSet<SegmentRef>>)>;

/// Replace each path under `roots` ([`path_ids`]) by what `f` makes of it, given its
/// direct-selected anchors and picked segments (none when it is selected whole): → (their ids, how
/// many anchors they have before and after). A path that `f` would empty or returns unchanged is
/// left as it is, with its live shape and its direct selection; a replaced path loses its live
/// shape and has the direct selection `f` gives it.
fn edit_paths(
    s: &mut Session,
    roots: &[NodeId],
    label: &str,
    f: impl Fn(&PathData, Option<&BTreeSet<AnchorRef>>, Option<&BTreeSet<SegmentRef>>) -> (PathData, PartSelection),
) -> Result<(Vec<NodeId>, usize, usize)> {
    let st = s.doc()?;
    let (d, sel) = (&st.doc, &st.selection);
    let ids = path_ids(d, roots);
    if ids.is_empty() {
        return Err(EngineError::Other(format!("{label}: select paths")));
    }
    let (mut before, mut after, mut changed) = (0, 0, vec![]);
    for id in &ids {
        let Some(NodeKind::Path { path, .. }) = d.node(*id).map(|n| &n.kind) else { continue };
        before += path.anchor_count();
        let (np, part) = f(path, sel.anchors.get(id), sel.segments.get(id));
        if (np.is_empty() && !path.is_empty()) || np == *path {
            after += path.anchor_count();
            continue;
        }
        after += np.anchor_count();
        changed.push((*id, np, part));
    }
    // When no path changes, the document and its undo history stay as they are.
    if !changed.is_empty() {
        s.edit(label, |d, sel| {
            for (id, np, part) in changed {
                if let Some(NodeKind::Path { path, live, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
                    // Refit and subdivision change what the old component indexes refer to: the
                    // path is selected whole, or with the part `f` says.
                    sel.anchors.remove(&id);
                    sel.segments.remove(&id);
                    if let Some((anchors, segments)) = part.filter(|(a, _)| a.len() < np.anchor_count()) {
                        sel.anchors.insert(id, anchors);
                        if let Some(segments) = segments {
                            sel.segments.insert(id, segments);
                        }
                    }
                    *path = np;
                    *live = None;
                }
            }
            Ok(())
        })?;
    }
    Ok((ids, before, after))
}

/// The segments of `path` a direct selection picks: the `segments` picked as such, or else every
/// segment with both its anchors among `anchors`. `None` when the path is selected whole, or when
/// no segment of it is selected.
fn selected_segments(
    path: &PathData,
    anchors: Option<&BTreeSet<AnchorRef>>,
    segments: Option<&BTreeSet<SegmentRef>>,
) -> Option<BTreeSet<SegmentRef>> {
    let anchors = anchors?;
    let picked: BTreeSet<SegmentRef> = match segments {
        Some(segs) => segs.iter().copied().filter(|&(si, i)| path.subpaths.get(si).is_some_and(|sp| i < sp.segment_count())).collect(),
        None => path
            .subpaths
            .iter()
            .enumerate()
            .flat_map(|(si, sp)| {
                (0..sp.segment_count())
                    .filter(move |&i| anchors.contains(&(si, i)) && anchors.contains(&(si, (i + 1) % sp.anchors.len())))
                    .map(move |i| (si, i))
            })
            .collect(),
    };
    (!picked.is_empty()).then_some(picked)
}

/// Auto-Simplify's level for the selection: [`po::auto_simplify_level`] for the longer side of the
/// selected objects' bounds. Object › Path › Simplify's slider opens at it, and
/// `object.path.simplify {auto: true}` simplifies at it.
pub fn auto_simplify_level(s: &Session) -> Result<f64> {
    let roots = selected_roots(s)?;
    let size = s.doc()?.doc.bounds_of(&roots, false).map_or(0.0, |b| b.width().max(b.height()));
    Ok(po::auto_simplify_level(size))
}

fn simplify(s: &mut Session, p: &Value) -> Result<Value> {
    // With auto, the slider's level is Auto-Simplify's for the selection's size; otherwise it is the level given.
    let level = if bool_or(p, "auto", false) {
        Some(auto_simplify_level(s)?)
    } else {
        num_param(p, "level").map(|l| if l.is_finite() { l.round().clamp(0.0, 100.0) } else { po::SIMPLIFY_LEVEL })
    };
    let tolerance = match level {
        Some(l) => po::simplify_level_tolerance(l),
        None => len_param(p, "tolerance").unwrap_or(1.0),
    };
    let opts = po::SimplifyOptions {
        tolerance: tolerance.clamp(1e-4, 1e4),
        corner_angle_deg: num_param(p, "cornerAngle").unwrap_or(30.0).clamp(0.0, 180.0),
        straight_lines: bool_or(p, "straightLines", false),
    };
    let whole = bool_or(p, "wholePaths", false);
    // A path with segments direct-selected simplifies only their runs, and keeps those runs (and the
    // other anchors it had selected) direct-selected.
    let roots = selected_roots(s)?;
    let (ids, before, after) = edit_paths(s, &roots, "Simplify", |path, anchors, segments| {
        let Some(picked) = selected_segments(path, anchors, segments).filter(|_| !whole) else {
            return (po::simplify_with(path, &opts), None);
        };
        let part = po::simplify_part(path, &picked, &opts);
        let mut kept: BTreeSet<AnchorRef> = anchors.into_iter().flatten().filter_map(|a| part.anchors.get(a).copied()).collect();
        for &(si, i) in &part.segments {
            let m = part.path.subpaths.get(si).map_or(1, |sp| sp.anchors.len().max(1));
            kept.extend([(si, i), (si, (i + 1) % m)]);
        }
        let segs = segments.map(|_| part.segments.clone());
        (part.path, Some((kept, segs)))
    })?;
    let mut out = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "before": before, "after": after });
    if let Some(l) = level {
        out["level"] = json!(l);
    }
    Ok(out)
}

/// Object › Path › Smooth (`object.path.smooth`): [`po::smooth_level`] on each path under the
/// objects given or selected, as one undo step; Auto-Smooth (`auto`) uses [`po::SMOOTH_AUTO_LEVEL`].
fn smooth(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.path.smooth";
    let level = number_or(p, "level", po::SMOOTH_AUTO_LEVEL, C)?.round().clamp(0.0, 100.0);
    let level = if switch_or(p, "auto", false, C)? { po::SMOOTH_AUTO_LEVEL } else { level };
    let roots = checked_ids_or_selection(s, p, C)?;
    let (ids, before, after) = edit_paths(s, &roots, "Smooth", |path, _, _| (po::smooth_level(path, level), None))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "before": before, "after": after, "level": level }))
}

fn add_anchor_points(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let (ids, before, after) = edit_paths(s, &roots, "Add Anchor Points", |path, _, _| (po::add_anchor_points(path), None))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "before": before, "after": after }))
}

// ---------- Split Into Grid ----------

/// One direction of the grid: the (gutter, total size) of `n` rows or columns. A `size` for each
/// makes the total; otherwise it is `total`, or `extent` (the selection's) when neither is given.
fn grid_axis(p: &Value, n: f64, [size, gutter, total]: [&str; 3], extent: f64) -> (f64, f64) {
    let gutter = len_param(p, gutter).or_else(|| len_param(p, "gutter")).unwrap_or(12.0).max(0.0);
    let total = match len_param(p, size) {
        Some(size) => n * size + (n - 1.0) * gutter,
        None => len_param(p, total).unwrap_or(extent),
    };
    (gutter, total)
}

fn split_into_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let rows = num_param(p, "rows").unwrap_or(2.0).round();
    let cols = num_param(p, "columns").unwrap_or(2.0).round();
    if !(1.0..=500.0).contains(&rows) || !(1.0..=500.0).contains(&cols) {
        return Err(bad("object.path.splitIntoGrid", "rows and columns must be 1..500"));
    }
    let too_small = || EngineError::Other("Split Into Grid: select paths large enough for the grid".into());
    // The selected paths, back to front: one grid over them all replaces them, where the topmost
    // one was and painted as it is.
    let d = &s.doc()?.doc;
    let roots = selected_roots(s)?;
    let paths: Vec<&Node> =
        roots.iter().filter_map(|id| d.node(*id)).filter(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })).collect();
    let Some((top, below)) = paths.split_last().map(|(top, below)| (Node::clone(top), below.iter().map(|n| n.id).collect::<Vec<_>>())) else {
        return Err(too_small());
    };
    let Some(b) = paths.iter().filter_map(|n| n.geometric_bounds()).reduce(|a, b| a.union(b)) else { return Err(too_small()) };
    let (row_gutter, height) = grid_axis(p, rows, ["height", "rowGutter", "totalHeight"], b.height());
    let (col_gutter, width) = grid_axis(p, cols, ["width", "columnGutter", "totalWidth"], b.width());
    let (rows, cols) = (rows as usize, cols as usize);
    let cells = po::split_into_grid(vectorcraft_geom::Rect::new(b.x0, b.y0, b.x0 + width, b.y0 + height), rows, cols, (row_gutter, col_gutter));
    if cells.is_empty() {
        return Err(too_small());
    }
    let mut edges: Vec<(bool, f64)> = Vec::new();
    if bool_or(p, "addGuides", false) {
        // Row-major cells: the first row has the column edges, each row's first cell the row edges.
        let bounds = cells.iter().filter_map(PathData::bounds);
        edges.extend(bounds.clone().take(cols).flat_map(|c| [(true, c.x0), (true, c.x1)]));
        edges.extend(bounds.step_by(cols).flat_map(|c| [(false, c.y0), (false, c.y1)]));
        // Without a gutter, neighbours share an edge.
        edges.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-9);
    }
    let ids = s.edit("Split Into Grid", |d, sel| {
        d.remove_many(&below)?;
        let (par, idx, _) = d.position(top.id).ok_or(EngineError::NoNode(top.id))?;
        let layer = d.layer_containing(top.id);
        d.remove(top.id)?;
        let mut out = Vec::with_capacity(cells.len());
        for (k, c) in cells.into_iter().enumerate() {
            let node = shape_node(d, c, Some(&top));
            out.push(d.insert(par, idx + k, node)?);
        }
        d.guides.extend(edges.iter().map(|&(vertical, pos)| vectorcraft_doc::Guide { layer, ..vectorcraft_doc::Guide::new(vertical, pos) }));
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "guides": edges.len() }))
}

// ---------- Clean Up ----------

fn is_painted(a: &Appearance) -> bool {
    a.items.iter().any(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    })
}

fn clean_up(s: &mut Session, p: &Value) -> Result<Value> {
    let stray = bool_or(p, "strayPoints", true);
    let unpainted = bool_or(p, "unpaintedObjects", true);
    let empty_text = bool_or(p, "emptyTextPaths", true);
    let found = {
        let d = &s.doc()?.doc;
        let mut v = vec![];
        fn visit(n: &Node, parent_compound: bool, parent_clip: bool, first: bool, v: &mut Vec<NodeId>, opts: (bool, bool, bool)) {
            let (stray, unpainted, empty_text) = opts;
            match &n.kind {
                NodeKind::Path { path, clipping, guide, .. } => {
                    if *guide || *clipping || (parent_clip && first) {
                        return;
                    }
                    if (stray && path.anchor_count() <= 1) || (unpainted && !parent_compound && !is_painted(&n.appearance)) {
                        v.push(n.id);
                    }
                }
                NodeKind::Compound { children, .. } => {
                    if unpainted && !(parent_clip && first) && !is_painted(&n.appearance) {
                        v.push(n.id);
                        return;
                    }
                    for c in children {
                        visit(c, true, false, false, v, opts);
                    }
                }
                NodeKind::Text(t) if empty_text && t.plain_text().trim().is_empty() => v.push(n.id),
                NodeKind::Layer { children, .. } => {
                    for c in children {
                        visit(c, false, false, false, v, opts);
                    }
                }
                NodeKind::Group { children, clip } => {
                    for (i, c) in children.iter().enumerate() {
                        visit(c, false, *clip, i == 0, v, opts);
                    }
                }
                _ => {}
            }
        }
        for l in &d.layers {
            visit(l, false, false, false, &mut v, (stray, unpainted, empty_text));
        }
        let editable = d.editable();
        v.retain(|id| editable(*id));
        v
    };
    if found.is_empty() {
        return Ok(json!({ "removed": 0 }));
    }
    s.edit("Clean Up", |d, _| {
        let mut gone = Beside::replacing(d, &found);
        for id in &found {
            if d.node(*id).is_some() {
                gone.put(d, *id, vec![])?;
            }
        }
        gone.finish(d)
    })?;
    Ok(json!({ "removed": found.len() }))
}

// ---------- Divide Objects Below ----------

fn divide_objects_below(s: &mut Session, _: &Value) -> Result<Value> {
    let roots = selected_roots(s)?;
    let [cutter_id] = roots[..] else { return Err(EngineError::Other("Divide Objects Below: select exactly one path".into())) };
    let ids = s.edit("Divide Objects Below", |d, sel| {
        let cutter = d.node(cutter_id).cloned().ok_or(EngineError::NoNode(cutter_id))?;
        let Some((cpath, crule)) = node_path(&cutter).filter(|_| !matches!(cutter.kind, NodeKind::Text(_))) else {
            return Err(EngineError::Other("Divide Objects Below: select a path".into()));
        };
        let Some(cb) = cpath.bounds() else { return Err(EngineError::Other("empty cutter".into())) };
        let cutter_pos = d.index_path(cutter_id).unwrap_or_default();
        let mut targets = vec![];
        for l in &d.layers {
            l.walk(&mut |n| {
                if let NodeKind::Path { path, guide: false, clipping: false, .. } = &n.kind
                    && path.is_closed()
                {
                    targets.push(n.id);
                }
                if matches!(n.kind, NodeKind::Compound { .. }) {
                    targets.push(n.id);
                }
            });
        }
        // Compound children are handled via their compound.
        targets.retain(|id| !matches!(d.parent_of(*id).and_then(|p| d.node(p)).map(|p| &p.kind), Some(NodeKind::Compound { .. })));
        let editable = d.editable();
        targets.retain(|id| {
            *id != cutter_id
                && editable(*id)
                && d.is_visible(*id)
                && d.index_path(*id).is_some_and(|ip| ip < cutter_pos)
                && d.node(*id).and_then(Node::geometric_bounds).is_some_and(|b| b.intersect(cb).area() > 0.0)
        });
        drop(editable);
        let mut out = vec![];
        let mut cut = Beside::replacing(d, &[targets.as_slice(), &[cutter_id]].concat());
        for t in targets {
            let Some(n) = d.node(t).cloned() else { continue };
            let Some((tp, tr)) = node_path(&n) else { continue };
            let inside = po::boolean(&tp, tr, &cpath, crule, BoolOp::Intersect);
            let outside = po::boolean(&tp, tr, &cpath, crule, BoolOp::Difference);
            if inside.is_empty() || outside.is_empty() {
                continue;
            }
            let pieces: Vec<Node> = [outside, inside].into_iter().map(|piece| shape_node(d, piece, Some(&n))).collect();
            out.extend(pieces.iter().map(|p| p.id));
            cut.put(d, t, pieces)?;
        }
        cut.put(d, cutter_id, vec![])?;
        cut.finish(d)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}
