//! Pathfinder panel (`object.pathfinder.*`) and Object → Path commands backed by
//! `vectorcraft-pathops`: Offset Path, Outline Stroke, Simplify, Add Anchor Points, Split Into Grid,
//! Clean Up and Divide Objects Below.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::appearance::{AppearanceItem, StrokeLayer};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops as po;
use vectorcraft_pathops::{BoolOp, PathfinderOp};

use super::edit::selected_roots;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.pathfinder.unite",
            "Unite",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Unite)
        ),
        cmd!(
            "object.pathfinder.minusFront",
            "Minus Front",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::MinusFront)
        ),
        cmd!(
            "object.pathfinder.intersect",
            "Intersect",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Intersect)
        ),
        cmd!(
            "object.pathfinder.exclude",
            "Exclude",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Exclude)
        ),
        cmd!(
            "object.pathfinder.divide",
            "Divide",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Divide)
        ),
        cmd!(
            "object.pathfinder.trim",
            "Trim",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Trim)
        ),
        cmd!(
            "object.pathfinder.merge",
            "Merge",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Merge)
        ),
        cmd!(
            "object.pathfinder.crop",
            "Crop",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_multi,
            |s, _| run_pf(s, PathfinderOp::Crop)
        ),
        cmd!(
            "object.pathfinder.outline",
            "Outline",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::Outline)
        ),
        cmd!(
            "object.pathfinder.minusBack",
            "Minus Back",
            ["Window", "Pathfinder"],
            None,
            "{} operate on the selected objects (back → front) → {ids}",
            has_selection,
            |s, _| run_pf(s, PathfinderOp::MinusBack)
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
            "{tolerance?: pt (1), cornerAngle?: deg (30), straightLines?: bool} → {ids, before, after} anchor counts",
            has_selection,
            simplify
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
            "{rows?: 2, columns?: 2, gutter?: pt (12)} → {ids}",
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
        Style { appearance: n.appearance.clone(), opacity: n.opacity, blend: n.blend }
    }
    fn apply(&self, n: &mut Node) {
        n.appearance = self.appearance.clone();
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
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_) => out.push(n.clone()),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                leaves(c, out);
            }
        }
        // A compound shape is its outline, painted as it is.
        NodeKind::CompoundShape { .. } => {
            if let Some(p) = vectorcraft_render::effects::evaluate_compound_shape(n) {
                out.push(vectorcraft_render::effects::carry_transparency(n, p));
            }
        }
        _ => {}
    }
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

// ---------- Pathfinder ----------

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
    }
}

fn run_pf(s: &mut Session, op: PathfinderOp) -> Result<Value> {
    // A Shape Mode on compound-shape members sets their mode.
    if let Some(r) = super::compoundshape::shape_mode_on_members(s, op) {
        return r;
    }
    let roots = selected_roots(s)?;
    let Some(&top) = roots.last() else { return Err(EngineError::Other("nothing selected".into())) };
    let shape_mode =
        matches!(op, PathfinderOp::Unite | PathfinderOp::MinusFront | PathfinderOp::Intersect | PathfinderOp::Exclude | PathfinderOp::MinusBack);
    let label = pf_label(op);
    let ids = s.edit(label, |d, sel| {
        // Build the stack (back → front) with a style per entry.
        let mut styles: Vec<Style> = vec![];
        let mut shapes: Vec<po::Shape> = vec![];
        let mut push = |path: PathData, rule: FillRule, st: Style| {
            if path.is_empty() {
                return;
            }
            let key = match styles.iter().position(|x| *x == st) {
                Some(k) => k,
                None => {
                    styles.push(st);
                    styles.len() - 1
                }
            };
            shapes.push(po::Shape::new(path, rule, key as u64));
        };
        for id in &roots {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            let mut lv = vec![];
            leaves(n, &mut lv);
            if lv.is_empty() {
                continue;
            }
            if shape_mode
                && lv.len() > 1
                && let Some(top) = lv.last()
            {
                // A group acts as one shape (the union of its leaves) with its top leaf's style.
                let parts: Vec<(PathData, FillRule)> = lv.iter().filter_map(node_path).collect();
                let refs: Vec<(&PathData, FillRule)> = parts.iter().map(|(p, r)| (p, *r)).collect();
                push(po::unite_all(&refs), FillRule::NonZero, style_of(top));
            } else {
                for l in &lv {
                    if let Some((p, r)) = node_path(l) {
                        push(p, r, style_of(l));
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
        let results = po::pathfinder(op, &shapes);
        if results.iter().all(|r| r.path.is_empty()) {
            return Err(EngineError::Other(format!("{label} produced an empty result")));
        }
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let new_node = if shape_mode {
            let r =
                results.into_iter().find(|r| !r.path.is_empty()).ok_or_else(|| EngineError::Other(format!("{label} produced an empty result")))?;
            let style = styles.get(r.key as usize).ok_or_else(|| EngineError::Other(format!("{label}: lost track of a shape's style")))?;
            shape_node_styled(d, r.path, style)
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
                    n.appearance = Appearance { items: vec![AppearanceItem::Stroke(StrokeLayer::new(paint, 1.0))], ..Default::default() };
                }
                children.push(Arc::new(n));
            }
            let gid = d.alloc_id();
            Node::group(gid, children)
        };
        let nid = new_node.id;
        d.insert(par, idx + 1, new_node)?;
        for id in &roots {
            d.remove(*id)?;
        }
        sel.set([nid]);
        Ok(vec![nid])
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
        let mut out = vec![];
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            let mut lv = vec![];
            leaves(&n, &mut lv);
            let mut made = vec![];
            for l in lv.iter().filter(|l| !matches!(l.kind, NodeKind::Text(_))) {
                let Some((path, rule)) = node_path(l) else { continue };
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
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            out.push(d.insert(par, idx + 1, new)?);
        }
        if out.is_empty() {
            return Err(EngineError::Other("Offset Path: select paths".into()));
        }
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

/// The art stroke `st` of a path (`path`, `rule`) paints, as fills: its brush art, or the
/// outline of the stroke (as on the canvas) filled with the stroke's paint, opacity, blend mode
/// and effects (a gradient along or across the stroke: gradient meshes in a clip group shaped
/// like the outline). `None` when it paints nothing.
pub(crate) fn outlined_stroke(
    d: &mut Document,
    brushes: &[vectorcraft_brush::Brush],
    path: &PathData,
    rule: FillRule,
    st: &StrokeLayer,
) -> Option<Node> {
    if let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) {
        let mut pieces = vectorcraft_brush::stroke_pieces(b, &path.to_bezpath(), st);
        let art = match pieces.len() {
            0 => return None,
            1 => pieces.remove(0),
            _ => Node::group(NodeId(0), pieces.into_iter().map(Arc::new).collect()),
        };
        let art = d.reid(&art);
        return Some(with_transparency(d, art, st.opacity, st.blend));
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
        ..vectorcraft_doc::appearance::FillLayer::new(st.paint.clone())
    };
    n.appearance = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() };
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
    f.appearance = Appearance { items, ..Default::default() };
    f.opacity = 1.0;
    f.blend = BlendMode::Normal;
    f.name = None;
    f.mask = None;
    if let NodeKind::Path { live, .. } = &mut f.kind {
        *live = None;
    }
    f
}

/// What stands for object `l` once its appearance became `parts` (paint order): the one part or a
/// group of them, with `l`'s transparency, name, opacity mask and effects. `None` without parts.
pub(crate) fn assemble(d: &mut Document, l: &Node, mut parts: Vec<Node>) -> Option<Node> {
    let root = match parts.len() {
        0 => return None,
        1 => parts.remove(0),
        _ => Node::group(d.alloc_id(), parts.into_iter().map(Arc::new).collect()),
    };
    let mut root = with_transparency(d, root, l.opacity, l.blend);
    root.name = l.name.clone();
    root.mask = l.mask.clone();
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
        let (par, idx, _) = d.position(l.id).ok_or(EngineError::NoNode(l.id))?;
        let nid = d.insert(par, idx, new)?;
        d.remove(l.id)?;
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

fn edit_paths(s: &mut Session, label: &str, f: impl Fn(&PathData) -> PathData) -> Result<(Vec<NodeId>, usize, usize)> {
    let roots = selected_roots(s)?;
    s.edit(label, |d, _| {
        let ids = path_ids(d, &roots);
        if ids.is_empty() {
            return Err(EngineError::Other(format!("{label}: select paths")));
        }
        let (mut before, mut after) = (0, 0);
        for id in &ids {
            if let Some(NodeKind::Path { path, live, .. }) = d.node_mut(*id).map(|n| &mut n.kind) {
                before += path.anchor_count();
                let np = f(path);
                if np.is_empty() && !path.is_empty() {
                    after += path.anchor_count();
                    continue;
                }
                after += np.anchor_count();
                *path = np;
                *live = None;
            }
        }
        Ok((ids, before, after))
    })
}

fn simplify(s: &mut Session, p: &Value) -> Result<Value> {
    let opts = po::SimplifyOptions {
        tolerance: len_param(p, "tolerance").unwrap_or(1.0).clamp(1e-4, 1e4),
        corner_angle_deg: num_param(p, "cornerAngle").unwrap_or(30.0).clamp(0.0, 180.0),
        straight_lines: bool_or(p, "straightLines", false),
    };
    let (ids, before, after) = edit_paths(s, "Simplify", |path| po::simplify_with(path, &opts))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "before": before, "after": after }))
}

fn add_anchor_points(s: &mut Session, _: &Value) -> Result<Value> {
    let (ids, before, after) = edit_paths(s, "Add Anchor Points", po::add_anchor_points)?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "before": before, "after": after }))
}

// ---------- Split Into Grid ----------

fn split_into_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let rows = num_param(p, "rows").unwrap_or(2.0).round();
    let cols = num_param(p, "columns").unwrap_or(2.0).round();
    if !(1.0..=500.0).contains(&rows) || !(1.0..=500.0).contains(&cols) {
        return Err(bad("object.path.splitIntoGrid", "rows and columns must be 1..500"));
    }
    let gutter = len_param(p, "gutter").unwrap_or(12.0).max(0.0);
    let roots = selected_roots(s)?;
    let ids = s.edit("Split Into Grid", |d, sel| {
        let mut out = vec![];
        for id in &roots {
            let Some(n) = d.node(*id).cloned() else { continue };
            if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) {
                continue;
            }
            let Some(b) = n.geometric_bounds() else { continue };
            let cells = po::split_into_grid(b, rows as usize, cols as usize, gutter);
            if cells.is_empty() {
                continue;
            }
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            d.remove(*id)?;
            for (k, c) in cells.into_iter().enumerate() {
                let node = shape_node(d, c, Some(&n));
                out.push(d.insert(par, idx + k, node)?);
            }
        }
        if out.is_empty() {
            return Err(EngineError::Other("Split Into Grid: select paths large enough for the grid".into()));
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
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
        v.retain(|id| d.is_editable(*id));
        v
    };
    if found.is_empty() {
        return Ok(json!({ "removed": 0 }));
    }
    s.edit("Clean Up", |d, _| {
        for id in &found {
            if d.node(*id).is_some() {
                d.remove(*id)?;
            }
        }
        Ok(())
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
        targets.retain(|id| {
            *id != cutter_id
                && d.is_editable(*id)
                && d.is_visible(*id)
                && d.index_path(*id).is_some_and(|ip| ip < cutter_pos)
                && d.node(*id).and_then(Node::geometric_bounds).is_some_and(|b| b.intersect(cb).area() > 0.0)
        });
        let mut out = vec![];
        for t in targets {
            let Some(n) = d.node(t).cloned() else { continue };
            let Some((tp, tr)) = node_path(&n) else { continue };
            let inside = po::boolean(&tp, tr, &cpath, crule, BoolOp::Intersect);
            let outside = po::boolean(&tp, tr, &cpath, crule, BoolOp::Difference);
            if inside.is_empty() || outside.is_empty() {
                continue;
            }
            let (par, idx, _) = d.position(t).ok_or(EngineError::NoNode(t))?;
            d.remove(t)?;
            for (k, piece) in [outside, inside].into_iter().enumerate() {
                let node = shape_node(d, piece, Some(&n));
                out.push(d.insert(par, idx + k, node)?);
            }
        }
        d.remove(cutter_id)?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}
