//! Select › Start Global Edit / Stop Global Edit, and the Global Edit Options.
//!
//! Behaviour from the public help pages "Edit similar objects"
//! (<https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-similar-objects.html>)
//! and "Update assets quickly with Global Edit"
//! (<https://helpx.adobe.com/ph_fil/illustrator/how-to/global-editing-sensei.html>), in our words:
//!
//! - With one object selected, Start Global Edit finds the objects that repeat its shape: the
//!   same outline, wherever it lies and at whatever size. Two options narrow the match: Appearance
//!   (the same fill and stroke too) and Size (the same width and height too). The search covers
//!   every artboard and the canvas around them by default; it can be limited to a range of
//!   artboards (`1, 3-5`) and can leave out the objects that lie on no artboard. Images, type,
//!   masks, linked files and selections of more than one object are not supported.
//! - While Global Edit is on, an edit made to one of the found objects is made to all of them,
//!   each in proportion to its own size: halving the height of a 100 pt object halves a 20 pt
//!   match to 10 pt. Stop Global Edit (or Escape) ends it.
//!
//! Here the found objects ([`GlobalEdit::members`]) stay highlighted while the selection keeps the
//! object that was selected, so that the Transform panel and the bounding box measure that object
//! and not the whole set. Each member keeps the transform (a move and a uniform scale) that maps
//! the first object's outline onto its own. After every edit ([`propagate`], run inside
//! [`Session::edit`], so the whole change is one undo step), the first selected member the edit
//! changed is the model: what changed about it is given to every other member, and nothing else,
//! so members found by shape alone keep their own fills and strokes where the edit didn't touch
//! them:
//!
//! - its outline (and anchors), placed through that transform, so in proportion to each member's
//!   size. Only geometry is placed so: no stroke weight is scaled to a member's size;
//! - its opacity, blending, knockout and the object's effects, each when it changed;
//! - of its fills and strokes, each setting that changed (paint, weight, opacity, blending,
//!   effects, caps, dashes, arrowheads …), as it is: it goes to the member's fill or stroke of the
//!   same rank (the second fill from the bottom to the second fill), and a member without one is
//!   left without it. A weight set to 4 pt is 4 pt on a smaller member too. The pen pressure a
//!   stroke was drawn with stays each member's own;
//! - a fill or stroke added or removed: a member whose fills and strokes lie in the model's
//!   former order gains the same item at the same place (or loses the one at that place). A
//!   member with another order, and any other change to the list (items moved, or several added
//!   or removed at once), takes the model's whole list of fills and strokes.
//!
//! Members that are hidden or locked at the time are left as they are. An edit that would take any
//! member past the canvas (beyond [`vectorcraft_doc::MAX_COORD`]) is refused whole, as one that
//! takes the selection there is. Whenever the document is put back (Undo, Redo, a cancelled drag
//! or scrub, an edit that failed), the members it lacks (or no longer supports) leave the Global
//! Edit, and it ends when fewer than two are left ([`follow_history`]), so that an object made
//! later under a freed id is never edited. Rotated copies, groups (the help page "Edit groups with
//! similar objects") and excluding a member with a Shift-click are not covered.

use serde::Serialize;
use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, Scaling, Shared, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect, Shape};

use super::wand::{fill_of, stroke_of};
use super::*;

const C: &str = "select.globalEdit";
const OPTIONS: &str = "select.globalEditOptions";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "select.globalEdit",
            "Start Global Edit",
            ["Select"],
            None,
            "{appearance?: bool, size?: bool, artboards?: \"all\" | range such as \"1, 3-5\", includeCanvas?: bool} (each defaults to the Global Edit Options) while Global Edit is off: start it on the one selected object, with the objects of the same shape (and, with `appearance`, the same fill and stroke; with `size`, the same width and height) on the artboards given (and, with `includeCanvas`, off every artboard) → {active: true, count, ids}; while on: stop it (Escape does the same) → {active: false}",
            can_toggle,
            toggle
        ),
        cmd!(
            "select.globalEditOptions",
            "Global Edit Options",
            [],
            None,
            "{appearance?: bool, size?: bool, artboards?: \"all\" | range such as \"1, 3-5\", includeCanvas?: bool} set the options Start Global Edit matches with (defaults: shape only, all artboards, objects on the canvas included) → {appearance, size, artboards, includeCanvas, count: the objects Start Global Edit would take now (0 unless one supported object is selected)}",
            has_doc,
            set_options
        ),
    ]
}

/// The Global Edit Options: what a similar object must share with the selected one besides its
/// shape, and where it may lie. Kept for the session.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Match the fill and stroke too.
    pub appearance: bool,
    /// Match the width and height too.
    pub size: bool,
    /// The artboards to search (1-based range as typed); `None`: all of them.
    pub artboards: Option<String>,
    /// Include the objects that lie on no artboard.
    pub include_canvas: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { appearance: false, size: false, artboards: None, include_canvas: true }
    }
}

impl Options {
    /// These options with the ones `p` gives, checked against `d`'s artboards.
    fn with(&self, p: &Value, d: &Document, cmd: &str) -> Result<Self> {
        let mut o = self.clone();
        o.appearance = bool_or(p, "appearance", o.appearance);
        o.size = bool_or(p, "size", o.size);
        o.include_canvas = bool_or(p, "includeCanvas", o.include_canvas);
        match p.get("artboards") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.trim().is_empty() || s.trim().eq_ignore_ascii_case("all") => o.artboards = None,
            Some(Value::String(s)) => {
                vectorcraft_doc::range::parse_range(s, d.artboards.len()).map_err(|e| bad(cmd, e))?;
                o.artboards = Some(s.trim().to_string());
            }
            Some(v) => return Err(bad(cmd, format!("`artboards`: all or a range such as 1, 3-5, not {v}"))),
        }
        Ok(o)
    }

    fn json(&self, count: usize) -> Value {
        json!({
            "appearance": self.appearance,
            "size": self.size,
            "artboards": self.artboards.as_deref().unwrap_or("all"),
            "includeCanvas": self.include_canvas,
            "count": count,
        })
    }
}

/// Outlines with their layer colours ([`highlights`]).
pub type Outlines = Vec<(BezPath, [u8; 3])>;

/// A Global Edit in progress (per document; not saved, not an undo step).
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalEdit {
    /// The objects edited together, the one selected at the start first, each with the transform
    /// (a move and a uniform scale) that maps that first object's outline onto its own.
    pub members: Vec<(NodeId, Affine)>,
    /// [`highlights`] as of a document revision: the canvas asks every frame.
    outlines: std::cell::RefCell<Option<(u64, Outlines)>>,
}

/// An outline reduced to its form: the anchors and handles placed in a unit box, with what tells
/// such outlines apart (kind, fill rule, subpaths and their anchor counts, open or closed).
struct Form {
    tag: Vec<u64>,
    symbol: Option<String>,
    points: Vec<Point>,
    /// The unit box → the object.
    frame: Affine,
}

impl Form {
    fn of(n: &Node) -> Option<Self> {
        let mut tag = vec![];
        let mut points = vec![];
        let mut symbol = None;
        let mut push = |path: &PathData, tag: &mut Vec<u64>| {
            tag.push(path.subpaths.len() as u64);
            for sp in &path.subpaths {
                tag.extend([u64::from(sp.closed), sp.anchors.len() as u64]);
                points.extend(sp.anchors.iter().flat_map(|a| [a.p, a.h_in, a.h_out]));
            }
        };
        let rule = |r: vectorcraft_geom::FillRule| u64::from(r == vectorcraft_geom::FillRule::EvenOdd);
        match &n.kind {
            NodeKind::Path { path, rule: r, clipping: false, guide: false, .. } => {
                tag.extend([0, rule(*r)]);
                push(path, &mut tag);
            }
            NodeKind::Compound { children, rule: r } => {
                tag.extend([1, rule(*r), children.len() as u64]);
                for c in children {
                    push(c.path_data()?, &mut tag);
                }
            }
            NodeKind::SymbolInstance { symbol: s, xf } => {
                tag.push(2);
                symbol = Some(s.clone());
                points.extend([*xf * Point::ZERO, *xf * Point::new(1.0, 0.0), *xf * Point::new(0.0, 1.0)]);
            }
            _ => return None,
        }
        let first = *points.first()?;
        let b = points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
        let side = b.width().max(b.height());
        if !(side.is_finite() && side > 1e-9) {
            return None;
        }
        let frame = Affine::translate(b.origin().to_vec2()) * Affine::scale(side);
        let back = frame.inverse();
        points.iter_mut().for_each(|p| *p = back * *p);
        Some(Self { tag, symbol, points, frame })
    }

    fn same(&self, o: &Form) -> bool {
        self.tag == o.tag
            && self.symbol == o.symbol
            && self.points.len() == o.points.len()
            && self.points.iter().zip(&o.points).all(|(a, b)| a.distance(*b) < 1e-6)
    }
}

/// Global Edit supports paths, compound paths and symbol instances; not images, type, linked
/// files, objects with an opacity mask, clipping paths or guides (the help page lists images,
/// text, masks and links as unsupported).
fn supported(n: &Node) -> bool {
    n.extras().mask.is_none() && Form::of(n).is_some()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

/// The members a Global Edit started on the one selected object would take, with `o`: that
/// object first, then the others in paint order.
fn matches(s: &Session, o: &Options) -> Result<Vec<(NodeId, Affine)>> {
    let st = s.doc()?;
    let d = &st.doc;
    let [id] = st.selection.objects.as_slice() else { return Err(bad(C, "select one object")) };
    let key = d.node(*id).ok_or_else(|| bad(C, "nothing selected"))?;
    let form = match Form::of(key) {
        Some(f) if supported(key) => f,
        _ => return Err(bad(C, "Global Edit works on paths, compound paths and symbol instances")),
    };
    let symbols = d.symbol_boxes();
    let size = |n: &Node| n.geometric_bounds_with(&symbols).map(|b| (b.width(), b.height()));
    let key_size = size(key);
    let (fill, stroke) = (fill_of(key), stroke_of(key));
    let boards: Option<Vec<Rect>> = match &o.artboards {
        None => None,
        Some(r) => Some(
            vectorcraft_doc::range::parse_range(r, d.artboards.len())
                .map_err(|e| bad(C, e))?
                .into_iter()
                .filter_map(|i| d.artboards.get(i).map(|a| a.rect))
                .collect(),
        ),
    };
    let on = |r: Rect, b: Rect| b.intersect(r).area() > 0.0 || vectorcraft_doc::artboard_holds(r, b.center());
    let in_scope = |n: &Node| {
        let Some(b) = n.geometric_bounds_with(&symbols) else { return false };
        if !d.artboards.iter().any(|a| on(a.rect, b)) {
            return o.include_canvas;
        }
        boards.as_ref().is_none_or(|rs| rs.iter().any(|r| on(*r, b)))
    };
    let mut out = vec![(*id, Affine::IDENTITY)];
    let back = form.frame.inverse();
    super::select::walk_selectable(d, st.isolation, |n| {
        if n.id == *id {
            return false;
        }
        match &n.kind {
            // Look into layers and plain groups (clip groups too: their art, not their clipping path).
            NodeKind::Layer { .. } => return true,
            NodeKind::Group { .. } => {
                let x = n.extras();
                return x.graph.is_none() && x.shaper.is_none() && x.dimension.is_none() && x.objects_on_path.is_none() && x.trace.is_none();
            }
            _ => {}
        }
        if let Some(f) = Form::of(n).filter(|f| f.same(&form) && supported(n))
            && (!o.size || matches!((key_size, size(n)), (Some((w, h)), Some((w2, h2))) if close(w, w2) && close(h, h2)))
            && (!o.appearance || {
                let (f2, s2) = (fill_of(n), stroke_of(n));
                super::select::same_paint(&fill, &f2, true) && super::select::same_paint(&stroke.0, &s2.0, true) && close(stroke.1, s2.1)
            })
            && in_scope(n)
        {
            out.push((n.id, f.frame * back));
        }
        // Compound paths, symbol instances and the other objects are taken whole.
        false
    });
    Ok(out)
}

fn can_toggle(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.global_edit.is_some() {
        return Ok(());
    }
    let [id] = st.selection.objects.as_slice() else {
        return Err(if st.selection.is_empty() { "nothing selected" } else { "select one object" }.into());
    };
    match st.doc.node(*id) {
        Some(n) if supported(n) => Ok(()),
        _ => Err("Global Edit works on paths, compound paths and symbol instances".into()),
    }
}

fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc()?.global_edit.is_some() {
        let st = s.doc_mut()?;
        st.global_edit = None;
        st.revision += 1;
        return Ok(json!({ "active": false }));
    }
    let o = s.global_edit_options.with(p, &s.doc()?.doc, C)?;
    let members = matches(s, &o)?;
    if members.len() < 2 {
        return Err(bad(C, "no similar objects found (rotated or reflected copies are not matched); change the Global Edit Options and try again"));
    }
    let ids: Vec<u64> = members.iter().map(|(id, _)| id.0).collect();
    let st = s.doc_mut()?;
    st.global_edit = Some(GlobalEdit { members, outlines: Default::default() });
    st.revision += 1;
    Ok(json!({ "active": true, "count": ids.len(), "ids": ids }))
}

fn set_options(s: &mut Session, p: &Value) -> Result<Value> {
    let o = s.global_edit_options.with(p, &s.doc()?.doc, OPTIONS)?;
    s.global_edit_options = o.clone();
    Ok(o.json(match_count(s)))
}

/// How many objects Start Global Edit would take now with the session's options (the Result(s)
/// line of the options): 0 unless one supported object is selected.
pub fn match_count(s: &Session) -> usize {
    match s.active() {
        Some(st) if st.global_edit.is_none() => matches(s, &s.global_edit_options).map_or(0, |m| m.len()),
        Some(st) => st.global_edit.as_ref().map_or(0, |g| g.members.len()),
        None => 0,
    }
}

/// Is `id` there, shown and unlocked, with every layer and group it is in?
fn editable(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).is_some_and(|a| a.iter().all(|i| d.node(*i).is_some_and(|n| n.visible && !n.locked && !n.is_template())))
}

/// After the document of `st` was put back to an earlier one (Undo, Redo, a cancelled interaction
/// or undo group, a failed or panicked command, a preview starting again, Discard Empty Type): the
/// members it lacks (or no longer supports) leave the Global Edit, so that an object made later
/// under a freed id is never edited; with fewer than two left, Global Edit ends.
pub(crate) fn follow_history(st: &mut crate::DocState) {
    let Some(ge) = &mut st.global_edit else { return };
    let d = &st.doc;
    ge.members.retain(|(id, _)| d.node(*id).is_some_and(supported));
    if ge.members.len() < 2 {
        st.global_edit = None;
    }
}

/// What an edit changed about a member: what [`propagate`] gives the others.
#[derive(Default)]
struct Changed {
    outline: bool,
    /// The member's appearance before and after the edit, when it changed.
    appearance: Option<[Shared<Appearance>; 2]>,
    opacity: bool,
    blend: bool,
    isolate: bool,
    knockout: bool,
    knockout_shape: bool,
}

impl Changed {
    fn between(a: &Node, b: &Node) -> Self {
        Self {
            outline: a.kind != b.kind || a.bbox_angle != b.bbox_angle,
            appearance: (a.appearance != b.appearance).then(|| [a.appearance.clone(), b.appearance.clone()]),
            opacity: a.opacity != b.opacity,
            blend: a.blend != b.blend,
            isolate: a.isolate != b.isolate,
            knockout: a.knockout != b.knockout,
            knockout_shape: a.knockout_shape != b.knockout_shape,
        }
    }

    fn any(&self) -> bool {
        self.outline || self.appearance.is_some() || self.opacity || self.blend || self.isolate || self.knockout || self.knockout_shape
    }

    /// Give `n` what changed, from `model` (the changed member placed on `n`).
    fn apply(&self, model: Node, n: &mut Node) {
        if self.outline {
            n.kind = model.kind;
            n.bbox_angle = model.bbox_angle;
        }
        if let Some([a, b]) = &self.appearance {
            give_appearance(a, b, &model.appearance, &mut n.appearance);
        }
        if self.opacity {
            n.opacity = model.opacity;
        }
        if self.blend {
            n.blend = model.blend;
        }
        if self.isolate {
            n.isolate = model.isolate;
        }
        if self.knockout {
            n.knockout = model.knockout;
        }
        if self.knockout_shape {
            n.knockout_shape = model.knockout_shape;
        }
    }
}

/// Set each field of `$n` that differs between `$a` and `$b` to the binding of the same name.
macro_rules! give_fields {
    ($a:ident, $b:ident, $n:ident: $($f:ident),+) => {
        $(
            if $a.$f != $b.$f {
                $n.$f = Clone::clone($f);
            }
        )+
    };
}

/// Give `n` what changed about the model's appearance from `a` (before the edit) to `b` (after),
/// with the values of `m` (`b` placed on `n`'s object); the module doc gives the rules.
fn give_appearance(a: &Appearance, b: &Appearance, m: &Appearance, n: &mut Shared<Appearance>) {
    let Appearance { items, effects, contents_index } = m;
    if a.effects != b.effects || a.contents_index != b.contents_index {
        let n = &mut **n;
        give_fields!(a, b, n: effects, contents_index);
    }
    if a.items == b.items {
        return;
    }
    let kinds = |v: &[AppearanceItem]| v.iter().map(AppearanceItem::is_fill).collect::<Vec<_>>();
    if kinds(&a.items) == kinds(&b.items) {
        // Fills and strokes kept in order: each changed one goes to the member's of its kind and
        // rank.
        for (k, ((p, q), mq)) in a.items.iter().zip(&b.items).zip(items).enumerate() {
            if p == q {
                continue;
            }
            let rank = a.items.iter().take(k).filter(|x| x.is_fill() == p.is_fill()).count();
            if let Some(t) = n.items.iter_mut().filter(|x| x.is_fill() == p.is_fill()).nth(rank) {
                give_item(p, q, mq, t);
            }
        }
        return;
    }
    // One item added (`grew`) or removed at `at`, the others kept in order; the member must hold
    // the model's former kinds in order.
    let one = |long: &[AppearanceItem], short: &[AppearanceItem]| {
        if long.len() != short.len() + 1 {
            return None;
        }
        let at = long.iter().zip(short).position(|(x, y)| x != y).unwrap_or(short.len());
        let mut rest = kinds(long);
        rest.remove(at);
        (rest == kinds(short)).then_some(at)
    };
    let step = match (one(&b.items, &a.items), one(&a.items, &b.items)) {
        (Some(at), _) => Some((at, true)),
        (None, Some(at)) => Some((at, false)),
        _ => None,
    };
    match step {
        Some((at, grew)) if kinds(&n.items) == kinds(&a.items) => {
            let n = &mut n.items;
            for (i, (p, t)) in a.items.iter().zip(n.iter_mut()).enumerate() {
                // The model's item after the edit that was `p` before it.
                let j = match (grew, i < at) {
                    (_, true) => i,
                    (true, false) => i + 1,
                    (false, false) if i == at => continue,
                    (false, false) => i - 1,
                };
                if let (Some(q), Some(mq)) = (b.items.get(j), items.get(j))
                    && p != q
                {
                    give_item(p, q, mq, t);
                }
            }
            match (grew, items.get(at)) {
                (true, Some(new)) => n.insert(at.min(n.len()), new.clone()),
                (false, _) if at < n.len() => {
                    n.remove(at);
                }
                _ => {}
            }
        }
        _ => n.items = items.clone(),
    }
}

/// Give the member's fill or stroke `n` the settings of the model's that changed from `a` to `b`,
/// with the values of `m` (`b` placed on the member). A stroke's pen pressure stays its own.
fn give_item(a: &AppearanceItem, b: &AppearanceItem, m: &AppearanceItem, n: &mut AppearanceItem) {
    use AppearanceItem::{Fill, Stroke};
    match (a, b, m, n) {
        (Fill(a), Fill(b), Fill(m), Fill(n)) => {
            let FillLayer { paint, opacity, blend, visible, effects, overprint } = m;
            give_fields!(a, b, n: paint, opacity, blend, visible, effects, overprint);
        }
        (Stroke(a), Stroke(b), Stroke(m), Stroke(n)) => {
            let StrokeLayer {
                paint,
                width,
                cap,
                join,
                miter_limit,
                align,
                dash,
                start_arrow,
                end_arrow,
                arrow_scale,
                profile,
                brush,
                opacity,
                blend,
                visible,
                effects,
                arrow_align,
                overprint,
                gradient_mode,
                pressure: _,
            } = m;
            give_fields!(a, b, n: paint, width, cap, join, miter_limit, align, dash, start_arrow, end_arrow, arrow_scale, profile, brush, opacity, blend, visible, effects, arrow_align, overprint, gradient_mode);
        }
        _ => {}
    }
}

/// Give the children of a copy placed on another object ids of that object's: its own children's
/// in order, new ids for any more and for everything deeper.
fn renumber(children: &mut [std::sync::Arc<Node>], own: &[NodeId], d: &mut Document) {
    for (i, c) in children.iter_mut().enumerate() {
        let c = std::sync::Arc::make_mut(c);
        c.id = own.get(i).copied().unwrap_or_else(|| d.alloc_id());
        if let Some(ch) = c.children_mut() {
            renumber(ch, &[], d);
        }
    }
}

/// After an edit made while Global Edit is on (`before` → `d`): the first member selected before
/// or after the edit that it changed is the model, and every other member that is shown and
/// unlocked gets what changed about it, placed by the members' transforms. → the members it
/// changed so (which [`Session::edit`] checks stay within the canvas, as it checks the selection).
pub(crate) fn propagate(before: &Document, d: &mut Document, ge: &GlobalEdit, selected: [&[NodeId]; 2]) -> Vec<NodeId> {
    let model = ge.members.iter().filter(|(id, _)| selected.iter().any(|s| s.contains(id))).find_map(|(id, xf)| {
        let (a, b) = (before.node(*id)?, d.node(*id)?);
        if std::ptr::eq(a, b) {
            return None;
        }
        let changed = Changed::between(a, b);
        changed.any().then(|| (*id, *xf, changed, b.clone()))
    });
    let Some((model_id, model_xf, changed, model)) = model else { return vec![] };
    let back = model_xf.inverse();
    let mut edited = vec![];
    for (id, xf) in &ge.members {
        if *id == model_id || !editable(d, *id) {
            continue;
        }
        let mut placed = model.clone();
        placed.transform(*xf * back, Scaling::default());
        if changed.outline {
            let own: Vec<NodeId> = d.node(*id).and_then(Node::children).map(|c| c.iter().map(|c| c.id).collect()).unwrap_or_default();
            if let Some(ch) = placed.children_mut() {
                renumber(ch, &own, d);
            }
        }
        if let Some(n) = d.node_mut(*id) {
            changed.apply(placed, n);
            edited.push(*id);
        }
    }
    edited
}

/// The outlines of the members of the active document's Global Edit that are not selected, in
/// their layer's colour: the canvas highlights them while Global Edit is on.
pub fn highlights(s: &Session) -> Outlines {
    let Some(st) = s.active() else { return vec![] };
    let Some(ge) = &st.global_edit else { return vec![] };
    if let Some((rev, out)) = &*ge.outlines.borrow()
        && *rev == st.revision
    {
        return out.clone();
    }
    let symbols = st.doc.symbol_boxes();
    let out: Vec<_> = ge
        .members
        .iter()
        .filter(|(id, _)| !st.selection.contains(*id))
        .filter_map(|(id, _)| {
            let n = st.doc.node(*id)?;
            let path = match n.stroke_path() {
                Some(p) => p,
                None => n.geometric_bounds_with(&symbols)?.to_path(0.1),
            };
            Some((path, st.doc.layer_color(*id)))
        })
        .collect();
    *ge.outlines.borrow_mut() = Some((st.revision, out.clone()));
    out
}
