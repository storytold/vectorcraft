//! Artwork Cleanup (`addon.cleanup.*`), a built-in add-on: Simplify Anchors (Keep Shape), the
//! Document Health Check (check, show and fix the issues it finds), Deep Ungroup and Rejoin Path
//! Pieces, and its text tools ([`text`]). Its panel and its submenu, Object › Plug-ins › Artwork
//! Cleanup, are the UI's.

mod checks;
#[cfg(test)]
mod tests;
mod text;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{PathData, SubPath};
use vectorcraft_pathops::addons as po;

use self::checks::{Kind, MAX_OBJECTS, Outcome, Scope, set_path};
use super::super::edit::selected_roots;
use super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_doc, has_selection, number_or, switch_or, text_param};
use crate::{EngineError, Result, Session};

/// Simplify Anchors' tolerance by default.
const TOLERANCE: f64 = 20.0;
/// Rejoin Path Pieces' gap by default, and the widest (pt).
const GAP: f64 = 0.1;
const MAX_GAP: f64 = 6.0;
/// Most anchors Simplify Anchors takes at once.
const MAX_ANCHORS: usize = 2_000_000;
/// Most rounds of fixes `addon.cleanup.fix` runs.
const FIX_ROUNDS: usize = 3;
/// Most ids a finding lists (its count counts them all).
const MAX_LISTED: usize = 1000;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![
        cmd!(
            "addon.cleanup.simplify",
            "Simplify Anchors (Keep Shape)…",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{tolerance?: 1..100 (20; higher removes more anchors and lets the outline move further from where it was, up to tolerance × 0.02 pt), protectCorners?: bool (false; anchors where the path turns by more than 30° stay), ids?: [id…] (default: the selection)} remove the anchors each path under the objects can do without, refitting the handles beside each so the outline keeps its shape, as one undo step; open ends stay → {ids, before, after} anchor counts",
            has_selection,
            simplify
        ),
        cmd!(
            query "addon.cleanup.check",
            "Health Check",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{scope?: \"selection\"|\"document\" (default: the selection, or the document while nothing is selected), kinds?: [kind…] (default: all: emptyText, strayPoints (paths of one anchor), unpainted, zeroOpacity, duplicateObjects (a path stacked on an identical one), offArtboard, duplicateAnchors (neighbours within 0.01 pt), tinyHandles (under 0.01 pt), collinearAnchors (corners on a straight run, turning less than sine 0.005), nearPoints (anchors within 0.01 pt of other paths' anchors), nearlyClosed (ends within 0.01 pt), filledOpenPaths (filled, unstroked open paths), needlessClipGroups (an unpainted clip that clips nothing, hides everything, or is a rectangle round all its art), needlessCompounds (one path or none), needlessGroups (one member or none, nothing of their own), emptyArtboards (the document only))} find issues without changing anything; locked and hidden objects are left out → {scope, total, findings: [{kind, label, count, ids (the objects, at most 1000), artboards (indexes)}]} for the kinds found",
            has_doc,
            check
        ),
        cmd!(
            query "addon.cleanup.show",
            "Show Issue",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{kind: kind (see addon.cleanup.check), scope?: \"selection\"|\"document\"} select the objects with that issue → {kind, count, ids, artboards}",
            has_doc,
            show
        ),
        cmd!(
            "addon.cleanup.fix",
            "Fix All Issues",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{scope?: \"selection\"|\"document\", kinds?: [kind…] (default: all; see addon.cleanup.check)} fix the issues addon.cleanup.check finds, in up to three rounds while a round still finds some (a fix can leave issues for another kind), as one undo step: what is empty, unpainted, invisible, duplicated or off the artboards goes, anchors merge, snap or go, tiny handles retract, paths close and needless groups, compound paths and clipping groups give way to their contents → {scope, total, fixed: {kind: count}}",
            has_doc,
            fix
        ),
        cmd!(
            "addon.cleanup.deepUngroup",
            "Deep Ungroup…",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{includeClipGroups?: bool (false: clipping groups, and the art inside them, stay as they are; true: they are ungrouped too and their clipping paths become plain paths), ids?: [id…] (default: the selection)} ungroup every level of the groups among the objects, as one undo step; each object keeps the opacity of the groups it was in → {ids (the objects left, selected), ungrouped (groups)}",
            has_selection,
            deep_ungroup
        ),
        cmd!(
            "addon.cleanup.rejoin",
            "Rejoin Path Pieces…",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{gap?: pt 0..6 (0.1), ids?: [id…] (default: the selection)} weld the open paths under the objects whose ends lie within the gap of each other back into paths: an end joins one other end, the one the path turns least to go on into, pieces may run either way, and a chain whose ends meet closes; each path takes the place and paint of its topmost piece, as one undo step → {ids, joined (paths made), removed (pieces merged into them)}",
            has_selection,
            rejoin
        ),
    ];
    v.extend(text::specs());
    v
}

/// The paths (not guides) under `roots`, compound path members included.
fn paths_under(d: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for n in roots.iter().filter_map(|id| d.node(*id)) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { guide: false, .. }) {
                out.push(c.id);
            }
        });
    }
    out
}

fn simplify(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.simplify";
    let tolerance = number_or(p, "tolerance", TOLERANCE, C)?;
    if !(1.0..=100.0).contains(&tolerance) {
        return Err(bad(C, "`tolerance` must be from 1 to 100"));
    }
    let protect = switch_or(p, "protectCorners", false, C)?;
    let roots = checked_ids_or_selection(s, p, C)?;
    let d = &s.doc()?.doc;
    let ids = paths_under(d, &roots);
    let paths: Vec<(NodeId, &PathData)> = ids.iter().filter_map(|id| Some((*id, d.node(*id)?.path_data()?))).collect();
    let before: usize = paths.iter().map(|(_, p)| p.anchor_count()).sum();
    if before > MAX_ANCHORS {
        return Err(EngineError::Other("Simplify Anchors: the paths have too many anchors to simplify at once".into()));
    }
    let changed: Vec<(NodeId, PathData)> = paths
        .iter()
        .filter_map(|(id, path)| {
            let np = po::simplify_keep_shape(path, tolerance * po::PT_PER_TOLERANCE, protect);
            (np != **path).then_some((*id, np))
        })
        .collect();
    let removed: usize = changed.iter().filter_map(|(id, np)| Some(d.node(*id)?.path_data()?.anchor_count().saturating_sub(np.anchor_count()))).sum();
    let after = before.saturating_sub(removed);
    // When no anchor can go, the document and its undo history stay as they are.
    if !changed.is_empty() {
        s.edit("Simplify Anchors (Keep Shape)", |d, sel| {
            for (id, np) in changed {
                // The anchors left are new ones: the path is selected whole.
                sel.anchors.remove(&id);
                sel.segments.remove(&id);
                set_path(d, id, np);
            }
            Ok(())
        })?;
    }
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "before": before, "after": after }))
}

/// The scope a check, show or fix request names (by default the selection, or the document while
/// nothing is selected) and its name.
fn scope_of(s: &Session, p: &Value, cmd: &str) -> Result<(Scope, &'static str)> {
    let roots = selected_roots(s)?;
    match text_param(p, "scope", cmd)? {
        Some("document") => Ok((Scope::Document, "document")),
        None if roots.is_empty() => Ok((Scope::Document, "document")),
        Some("selection") if roots.is_empty() => Err(bad(cmd, "nothing selected")),
        Some("selection") | None => Ok((Scope::Objects(roots), "selection")),
        Some(other) => Err(bad(cmd, format!("`scope` must be \"selection\" or \"document\", not {other:?}"))),
    }
}

/// The kinds a request names (all by default), in the order Fix runs them.
fn kinds_of(p: &Value, cmd: &str) -> Result<Vec<Kind>> {
    let given: Vec<Kind> = match p.get("kinds") {
        None | Some(Value::Null) => return Ok(Kind::ALL.to_vec()),
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().and_then(Kind::parse).ok_or_else(|| bad(cmd, format!("unknown issue kind {v}")))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(cmd, "`kinds` must be a list of issue kinds")),
    };
    Ok(Kind::ALL.into_iter().filter(|k| given.contains(k)).collect())
}

/// Kind `kind`'s fix run on a copy of the active document: what Fix would change.
fn dry_run(s: &Session, kind: Kind, scope: &Scope) -> Result<Outcome> {
    let st = s.doc()?;
    let (mut d, mut sel, mut scope) = ((*st.doc).clone(), st.selection.clone(), scope.clone());
    kind.fix(&mut d, &mut sel, &mut scope)
}

fn finding(kind: Kind, o: &Outcome) -> Value {
    let ids: Vec<u64> = o.ids.iter().take(MAX_LISTED).map(|i| i.0).collect();
    json!({ "kind": kind.id(), "label": kind.label(), "count": o.count, "ids": ids, "artboards": o.artboards })
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.check";
    let (scope, name) = scope_of(s, p, C)?;
    let mut findings = vec![];
    let mut total = 0;
    for kind in kinds_of(p, C)? {
        let o = dry_run(s, kind, &scope)?;
        if o.count > 0 {
            total += o.count;
            findings.push(finding(kind, &o));
        }
    }
    Ok(json!({ "scope": name, "total": total, "findings": findings }))
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.show";
    let id = text_param(p, "kind", C)?.ok_or_else(|| bad(C, "missing `kind`"))?;
    let kind = Kind::parse(id).ok_or_else(|| bad(C, format!("unknown issue kind {id:?}")))?;
    let (scope, _) = scope_of(s, p, C)?;
    let o = dry_run(s, kind, &scope)?;
    if o.count == 0 {
        return Err(EngineError::Other("None of these issues are left: check again".into()));
    }
    // Artboards are no objects to select: the selection stays as it is for them.
    let ids = o.ids.clone();
    if !ids.is_empty() {
        s.select(|_, sel| sel.set(ids))?;
    }
    Ok(finding(kind, &o))
}

fn fix(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.fix";
    let (mut scope, name) = scope_of(s, p, C)?;
    let kinds = kinds_of(p, C)?;
    // Fixed on a copy, kind after kind (what one leaves, the next cleans up), then kept as one step.
    let st = s.doc()?;
    let (mut d, mut sel) = ((*st.doc).clone(), st.selection.clone());
    let mut fixed = Map::new();
    let mut total = 0;
    // A fix can leave issues for a kind that ran before it (groups unwrapped side by side may hold
    // duplicates): the kinds run again, a few rounds at most, until a round finds nothing.
    for _ in 0..FIX_ROUNDS {
        let before = total;
        for &kind in &kinds {
            let o = kind.fix(&mut d, &mut sel, &mut scope)?;
            if o.count > 0 {
                total += o.count;
                let n = fixed.get(kind.id()).and_then(Value::as_u64).unwrap_or(0);
                fixed.insert(kind.id().into(), json!(n + o.count as u64));
            }
        }
        if total == before {
            break;
        }
    }
    // With nothing to fix, the document and its undo history stay as they are.
    if total > 0 {
        sel.prune(&d);
        s.edit("Fix Issues", |doc, selection| {
            *doc = d;
            *selection = sel;
            Ok(())
        })?;
    }
    Ok(json!({ "scope": name, "total": total, "fixed": fixed }))
}

/// Is `n` a group Deep Ungroup takes apart (a clipping group only with `clips`)?
fn ungroupable(n: &Node, clips: bool) -> bool {
    matches!(n.kind, NodeKind::Group { clip, .. } if clips || !clip)
}

/// `n` with every level of grouping Deep Ungroup takes apart removed, into `out`; `count` counts
/// the groups. Members take on the opacity of the groups they were in, and a clipping path
/// becomes a plain path.
fn flatten(n: Node, clips: bool, out: &mut Vec<Node>, count: &mut usize) {
    if !ungroupable(&n, clips) {
        out.push(n);
        return;
    }
    let NodeKind::Group { children, clip } = &n.kind else { return };
    *count += 1;
    for (k, c) in children.iter().enumerate() {
        let mut c = (**c).clone();
        if *clip
            && k == 0
            && let NodeKind::Path { clipping, .. } = &mut c.kind
        {
            *clipping = false;
        }
        c.opacity *= n.opacity;
        flatten(c, clips, out, count);
    }
}

fn deep_ungroup(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.deepUngroup";
    let clips = switch_or(p, "includeClipGroups", false, C)?;
    let roots = checked_ids_or_selection(s, p, C)?;
    let d = &s.doc()?.doc;
    let (mut items, mut selected, mut count) = (vec![], vec![], 0);
    for id in &roots {
        match d.node(*id).filter(|n| ungroupable(n, clips)) {
            Some(n) => {
                let mut out = vec![];
                flatten(n.clone(), clips, &mut out, &mut count);
                selected.extend(out.iter().map(|n| n.id));
                items.push((*id, out));
            }
            None => selected.push(*id),
        }
    }
    // With no group to take apart, the document and its undo history stay as they are.
    if !items.is_empty() {
        let ids = selected.clone();
        s.edit("Deep Ungroup", |d, sel| {
            d.insert_beside(items, true)?;
            sel.set(ids);
            Ok(())
        })?;
    }
    Ok(json!({ "ids": selected.iter().map(|i| i.0).collect::<Vec<_>>(), "ungrouped": count }))
}

/// The open single-subpath paths under `roots` that Rejoin Path Pieces takes, in paint order
/// (compound path members, clipping paths and guides stay out).
fn pieces(d: &Document, roots: &[NodeId]) -> Vec<(NodeId, SubPath)> {
    fn visit(n: &Node, out: &mut Vec<(NodeId, SubPath)>) {
        match &n.kind {
            NodeKind::Path { path, guide: false, clipping: false, .. } => {
                if let [sp] = path.subpaths.as_slice()
                    && !sp.closed
                {
                    out.push((n.id, sp.clone()));
                }
            }
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().for_each(|c| visit(c, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    roots.iter().filter_map(|id| d.node(*id)).for_each(|n| visit(n, &mut out));
    let editable = d.editable();
    out.retain(|(id, _)| editable(*id));
    out
}

fn rejoin(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.rejoin";
    let gap = number_or(p, "gap", GAP, C)?;
    if !(0.0..=MAX_GAP).contains(&gap) {
        return Err(bad(C, "`gap` must be from 0 to 6 pt"));
    }
    let roots = checked_ids_or_selection(s, p, C)?;
    let found = pieces(&s.doc()?.doc, &roots);
    if found.len() > MAX_OBJECTS {
        return Err(EngineError::Other("Rejoin Path Pieces: too many paths to rejoin at once".into()));
    }
    let subpaths: Vec<SubPath> = found.iter().map(|(_, sp)| sp.clone()).collect();
    // Each chain becomes its topmost piece (the last in paint order); the others go.
    let (mut kept, mut gone) = (vec![], vec![]);
    for j in po::rejoin(&subpaths, gap) {
        let ids: Vec<NodeId> = j.members.iter().filter_map(|&m| found.get(m).map(|f| f.0)).collect();
        let Some(&top) = j.members.iter().max().and_then(|&m| found.get(m).map(|f| &f.0)) else { continue };
        gone.extend(ids.into_iter().filter(|id| *id != top));
        kept.push((top, j.path));
    }
    if !kept.is_empty() {
        let ids: Vec<NodeId> = kept.iter().map(|k| k.0).collect();
        s.edit("Rejoin Path Pieces", |d, sel| {
            for (id, sp) in &kept {
                set_path(d, *id, PathData::single(sp.clone()));
            }
            d.remove_many(&gone)?;
            sel.set(ids);
            Ok(())
        })?;
    }
    let ids: Vec<u64> = kept.iter().map(|k| k.0.0).collect();
    Ok(json!({ "ids": ids, "joined": kept.len(), "removed": gone.len() }))
}
