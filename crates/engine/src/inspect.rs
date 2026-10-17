//! Agent-friendly document summaries and view-models for panels.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{ArrowAlign, Document, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, SymbolBoxes, WidthProfile};

use crate::Session;

pub(crate) fn rect_json(r: Option<vectorcraft_geom::Rect>) -> Value {
    match r {
        Some(r) => json!({ "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height() }),
        None => Value::Null,
    }
}

/// How much of the subtree a node summary carries.
#[derive(Clone, Copy, Default)]
pub struct SummaryOpts {
    /// Child levels to include (`None`: the whole subtree). `0` is the node alone.
    pub depth: Option<u64>,
    /// Children shown per node (`None`: all, top of the stack first). A level that shows
    /// fewer children than it has reports `childCount`, so truncation is never silent.
    pub child_limit: Option<u64>,
    /// The node is a member of a compound path, which paints it: it has no paint of its own
    /// to report.
    pub compound_member: bool,
}

/// Compact tree summary of a node (for `document.inspect`): the whole subtree, symbol instances
/// measured by their art from `symbols`.
pub fn node_summary(n: &Node, symbols: &SymbolBoxes) -> Value {
    node_summary_opts(n, SummaryOpts::default(), symbols)
}

/// The same summary with [`SummaryOpts`] applied.
pub fn node_summary_opts(n: &Node, opts: SummaryOpts, symbols: &SymbolBoxes) -> Value {
    let mut v = json!({
        "id": n.id.0,
        "name": n.display_name(),
        "kind": n.kind_label(),
        "visible": n.visible,
        "locked": n.locked,
        "bounds": rect_json(n.geometric_bounds_with(symbols)),
    });
    if n.opacity < 1.0 {
        v["opacity"] = json!(n.opacity);
    }
    if n.blend != BlendMode::Normal {
        v["blend"] = json!(n.blend.label());
    }
    // The angle of its rotated bounding box (counter-clockwise degrees).
    if n.bbox_angle != 0.0 {
        v["rotation"] = json!(n.bbox_angle);
    }
    if let NodeKind::Text(t) = &n.kind {
        // Type shows its characters' paint: the first run's, as paint.proxies and the Stroke
        // options read it. Fills and strokes of the type object itself, above or below its
        // characters, come separately so the two stay apart.
        let default = vectorcraft_doc::CharStyle::default();
        let style = t.runs.first().map_or(&default, |r| &r.style);
        v["fill"] = json!(style.fill.label());
        v["stroke"] = json!(style.stroke.label());
        v["strokeWidth"] = json!(if style.stroke.is_none() { 0.0 } else { style.stroke_width });
        if n.appearance.fill().is_some() || n.appearance.stroke().is_some() {
            v["objectFill"] = json!(n.appearance.fill_paint().label());
            v["objectStroke"] = json!(n.appearance.stroke_paint().label());
            v["objectStrokeWidth"] = json!(n.appearance.stroke_width());
        }
    } else if opts.compound_member {
        v["compoundMember"] = json!(true);
    } else if !n.is_container() || matches!(n.kind, NodeKind::Compound { .. }) {
        // A compound path is a container, but its fill and stroke live on it, not on its members.
        v["fill"] = json!(n.appearance.fill_paint().label());
        v["stroke"] = json!(n.appearance.stroke_paint().label());
        v["strokeWidth"] = json!(n.appearance.stroke_width());
    }
    if let Some(st) = node_stroke(n).filter(|_| !opts.compound_member) {
        v["strokeOptions"] = stroke_options(&st);
    }
    match &n.kind {
        NodeKind::Layer { color, template, printable, preview, dim_images, clip, .. } => {
            let [r, g, b] = color.rgb();
            v["color"] = json!(format!("#{r:02x}{g:02x}{b:02x}"));
            v["template"] = json!(template);
            v["printable"] = json!(printable);
            v["preview"] = json!(preview);
            v["dimImages"] = json!(dim_images);
            v["clip"] = json!(clip);
        }
        NodeKind::Path { path, .. } => {
            v["anchors"] = json!(path.anchor_count());
            v["closed"] = json!(path.is_closed());
        }
        NodeKind::Text(t) => {
            v["text"] = json!(t.plain_text());
            // Area type and type on a path: does the text fit (the red overflow "+")? Area type
            // also reports its fit and Shrink Text's factor (1 unshrunk).
            if !matches!(t.kind, vectorcraft_doc::TextKind::Point) {
                let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
                v["overflow"] = json!(lay.overflow);
                if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }) {
                    v["fit"] = json!(t.area.fit.id());
                    v["fitScale"] = json!(lay.fit_scale);
                }
            }
        }
        _ => {}
    }
    if let Some(ch) = n.children() {
        let total = ch.len();
        let take_n = opts.child_limit.map_or(total, |l| l.min(total as u64) as usize);
        if opts.depth.is_none_or(|d| d > 0) {
            let next = SummaryOpts {
                depth: opts.depth.map(|d| d.saturating_sub(1)),
                child_limit: opts.child_limit,
                compound_member: matches!(n.kind, NodeKind::Compound { .. }),
            };
            v["children"] = Value::Array(ch.iter().rev().take(take_n).map(|c| node_summary_opts(c, next, symbols)).collect());
        }
        if take_n < total || v.get("children").is_none() {
            v["childCount"] = json!(total);
        }
    }
    v
}

pub fn document(s: &Session) -> Value {
    document_opts(s, SummaryOpts::default())
}

/// The same summary with [`SummaryOpts`] applied to the layer tree (artboards and
/// the rest always come whole): `depth: 0` is the skeleton — top layers with counts.
pub fn document_opts(s: &Session, opts: SummaryOpts) -> Value {
    document_scoped(s, opts, &Scope::default())
}

/// Which nodes `document.inspect` lists in place of the whole layer tree.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    /// Only these nodes (any level, already checked to exist), in the order given.
    pub ids: Option<Vec<vectorcraft_doc::NodeId>>,
    /// Only nodes of this panel label (case-insensitive), searched in the subtrees of `ids`
    /// or else of every layer.
    pub kind: Option<String>,
    /// At most this many entries after `offset` (`None`: all).
    pub limit: Option<u64>,
    /// Entries skipped first.
    pub offset: u64,
}

impl Scope {
    fn scoped(&self) -> bool {
        self.ids.is_some() || self.kind.is_some()
    }
}

/// A node with the ids of its ancestors (layer first).
type Hit<'a> = (&'a Node, Vec<u64>);

fn ancestors(d: &Document, id: vectorcraft_doc::NodeId) -> Vec<u64> {
    let mut out = Vec::new();
    let mut cur = d.parent_of(id);
    // The bound keeps a malformed tree from looping.
    while let Some(p) = cur.filter(|_| out.len() < 10_000) {
        out.push(p.0);
        cur = d.parent_of(p);
    }
    out.reverse();
    out
}

fn collect_kind<'a>(n: &'a Node, kind: &str, path: &mut Vec<u64>, out: &mut Vec<Hit<'a>>) {
    if n.kind_label().eq_ignore_ascii_case(kind) {
        out.push((n, path.clone()));
    }
    if let Some(ch) = n.children() {
        path.push(n.id.0);
        for c in ch.iter().rev() {
            collect_kind(c, kind, path, out);
        }
        path.pop();
    }
}

/// [`document_opts`] with the layer tree replaced by the nodes `scope` picks. Without `ids`
/// or `kind` they are the top layers (`layers`, paged by `limit`/`offset`); with either they
/// are `nodes` (each with its `path` of ancestor ids when picked by `kind`). A paged or scoped
/// reply also reports `total`, the entries before paging.
pub fn document_scoped(s: &Session, opts: SummaryOpts, scope: &Scope) -> Value {
    let Some(st) = s.active() else { return Value::Null };
    let d = &st.doc;
    let symbols = d.symbol_boxes();
    let mut hits: Vec<Hit> = Vec::new();
    match (&scope.ids, scope.kind.as_deref().filter(|k| !k.is_empty())) {
        (None, None) => hits.extend(d.layers.iter().rev().map(|l| (&**l, Vec::new()))),
        (Some(ids), None) => hits.extend(ids.iter().filter_map(|&i| d.node(i)).map(|n| (n, Vec::new()))),
        (ids, Some(kind)) => {
            let mut path = Vec::new();
            match ids {
                Some(ids) => {
                    for n in ids.iter().filter_map(|&i| d.node(i)) {
                        path = ancestors(d, n.id);
                        collect_kind(n, kind, &mut path, &mut hits);
                    }
                }
                None => d.layers.iter().rev().for_each(|l| collect_kind(l, kind, &mut path, &mut hits)),
            }
        }
    }
    let total = hits.len();
    let skip = usize::try_from(scope.offset).unwrap_or(usize::MAX);
    let take = scope.limit.map_or(usize::MAX, |l| usize::try_from(l).unwrap_or(usize::MAX));
    let by_kind = scope.kind.as_deref().is_some_and(|k| !k.is_empty());
    let nodes: Vec<Value> = hits
        .into_iter()
        .skip(skip)
        .take(take)
        .map(|(n, path)| {
            let member = d.parent_of(n.id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
            let mut v = node_summary_opts(n, SummaryOpts { compound_member: member, ..opts }, &symbols);
            if by_kind {
                v["path"] = json!(path);
            }
            v
        })
        .collect();
    let mut out = document_base(s);
    if let Some(o) = out.as_object_mut() {
        o.insert(if scope.scoped() { "nodes" } else { "layers" }.into(), Value::Array(nodes));
        if scope.scoped() || scope.limit.is_some() || scope.offset > 0 {
            o.insert("total".into(), json!(total));
        }
    }
    out
}

/// Everything `document.inspect` reports except the layer tree.
fn document_base(s: &Session) -> Value {
    let Some(st) = s.active() else { return Value::Null };
    let d = &st.doc;
    json!({
        "title": st.title(),
        "path": st.path,
        "dirty": st.is_dirty(),
        "revision": st.revision,
        "units": d.units.label(),
        "colorMode": format!("{:?}", d.color_mode),
        // An artboard's name and rectangle, and `locked: true` when it is locked.
        "artboards": d.artboards.iter().map(|a| {
            let mut v = json!({"name": a.name, "x": a.rect.x0, "y": a.rect.y0, "width": a.rect.width(), "height": a.rect.height()});
            if a.locked {
                v["locked"] = json!(true);
            }
            v
        }).collect::<Vec<_>>(),
        "currentLayer": st.active_layer.map(|l| l.0),
        // The rows highlighted in the Layers panel (`layer.setCurrent`, `layer.highlight`).
        "layerRows": st.highlighted_rows().iter().map(|i| i.0).collect::<Vec<_>>(),
        "isolation": st.isolation.map(|l| l.0),
        "selection": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>(),
        "selectionBounds": rect_json(d.bounds_of(&st.selection.objects, false)),
        // The angle the selection's bounding box stands at (counter-clockwise degrees).
        "selectionRotation": d.bbox_angle(&st.selection.objects),
        // The layer, group or object targeted through the Layers panel (`layer.target`).
        "target": st.selection.target.map(|t| t.0),
        // The key object Align aligns to and Distribute Spacing spaces from (`select.key`).
        "key": st.selection.key.map(|k| k.0),
        "history": st.history.undo.iter().map(|h| h.label.clone()).collect::<Vec<_>>(),
        "redo": st.history.redo.iter().rev().map(|h| h.label.clone()).collect::<Vec<_>>(),
        "objects": d.node_count(),
        "tool": s.tool_id(),
        "paint": {"fill": s.paint.fill.label(), "stroke": s.paint.stroke.label(), "strokeWidth": s.paint.stroke_width, "fillActive": s.fill_active, "appearanceItem": s.appearance_item()},
        "pasteRemembersLayers": d.paste_remembers_layers,
    })
}

/// What `document.find` matches: every given filter must hit (AND). All matching is
/// case-insensitive; `name` and `text` are substrings, `kind` is the exact panel label
/// (`Layer`, `Group`, `Path`, `Type`, `Image`, ...).
pub struct FindFilter {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub text: Option<String>,
}

impl FindFilter {
    /// True when no effective filter was given (empty strings count as absent).
    pub fn is_empty(&self) -> bool {
        [self.name.as_ref(), self.kind.as_ref(), self.text.as_ref()].iter().all(|f| f.is_none())
    }

    fn hits(&self, n: &Node) -> bool {
        if let Some(needle) = &self.name
            && !n.display_name().to_lowercase().contains(needle)
        {
            return false;
        }
        if let Some(kind) = &self.kind
            && !n.kind_label().eq_ignore_ascii_case(kind)
        {
            return false;
        }
        if let Some(needle) = &self.text {
            let NodeKind::Text(t) = &n.kind else { return false };
            if !t.plain_text().to_lowercase().contains(needle) {
                return false;
            }
        }
        true
    }
}

/// Search the whole layer tree, top of the stack first like the Layers panel.
/// Returns at most `limit` hits as `{id, name, kind, path}` (`path` is the ancestor
/// chain, layer first) plus the total hit count, so a capped reply stays explicit.
pub fn find_nodes(doc: &Document, filter: &FindFilter, limit: u64) -> (Vec<Value>, u64) {
    use std::sync::Arc;
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut ancestors: Vec<u64> = Vec::new();
    fn walk(nodes: &[Arc<Node>], ancestors: &mut Vec<u64>, filter: &FindFilter, limit: u64, out: &mut Vec<Value>, total: &mut u64) {
        for n in nodes.iter().rev() {
            if filter.hits(n) {
                *total += 1;
                if (out.len() as u64) < limit {
                    out.push(json!({"id": n.id.0, "name": n.display_name(), "kind": n.kind_label(), "path": ancestors}));
                }
            }
            if let Some(ch) = n.children() {
                ancestors.push(n.id.0);
                walk(ch, ancestors, filter, limit, out, total);
                ancestors.pop();
            }
        }
    }
    walk(&doc.layers, &mut ancestors, filter, limit, &mut out, &mut total);
    (out, total)
}

/// Cap names as `stroke.set` takes them.
pub const CAPS: [(LineCap, &str); 3] = [(LineCap::Butt, "butt"), (LineCap::Round, "round"), (LineCap::Square, "square")];
/// Join names as `stroke.set` takes them.
pub const JOINS: [(LineJoin, &str); 3] = [(LineJoin::Miter, "miter"), (LineJoin::Round, "round"), (LineJoin::Bevel, "bevel")];
/// Stroke alignment names as `stroke.set` takes them.
pub const ALIGNS: [(StrokeAlign, &str); 3] = [(StrokeAlign::Center, "center"), (StrokeAlign::Inside, "inside"), (StrokeAlign::Outside, "outside")];

fn name_of<T: PartialEq>(table: &[(T, &'static str)], v: T) -> &'static str {
    table.iter().find(|(k, _)| *k == v).map_or("", |(_, n)| n)
}

/// The stroke an object's Stroke panel options describe: type's first run's character stroke,
/// else the topmost stroke of its own (groups and layers too). None without a stroke.
pub fn node_stroke(n: &Node) -> Option<StrokeLayer> {
    match &n.kind {
        NodeKind::Text(t) => Some(t.runs.first().map_or_else(|| vectorcraft_doc::CharStyle::default().stroke_layer(), |r| r.style.stroke_layer())),
        _ => n.appearance.stroke().cloned(),
    }
}

/// A stroke's options in `stroke.set` terms: cap, join, miterLimit, align, dash (null when
/// solid), dashOffset, alignDashes, startArrow/endArrow (null for none), arrowAlign and profile,
/// plus widthPoints: the profile's `[t, left, right]` points (width factors; null when uniform)
/// and gradientMode: how a gradient lies on it (`paint.editGradient` strokeMode).
pub fn stroke_options(st: &StrokeLayer) -> Value {
    json!({
        "cap": name_of(&CAPS, st.cap),
        "join": name_of(&JOINS, st.join),
        "miterLimit": st.miter_limit,
        "align": name_of(&ALIGNS, st.align),
        "dash": st.dash.as_ref().map(|d| &d.pattern),
        "dashOffset": st.dash.as_ref().map_or(0.0, |d| d.offset),
        "alignDashes": st.dash.as_ref().is_some_and(|d| d.align_corners),
        "startArrow": st.start_arrow,
        "endArrow": st.end_arrow,
        "arrowAlign": if st.arrow_align == ArrowAlign::Tip { "tip" } else { "extend" },
        "profile": WidthProfile::id_of(st.profile.as_ref()),
        "widthPoints": st.profile.as_ref().map(|p| &p.points),
        "gradientMode": st.gradient_mode.name(),
    })
}

/// Which Stroke panel values the selected objects don't share (their fields show blank), and
/// whether Align Stroke applies to them (no open path or type among them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StrokeMixed {
    pub weight: bool,
    pub cap: bool,
    pub join: bool,
    pub miter_limit: bool,
    pub align: bool,
    pub dash: bool,
    pub can_align: bool,
}
