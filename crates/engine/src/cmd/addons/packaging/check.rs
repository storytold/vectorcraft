//! Check Fold Lines (`addon.packaging.check`): the blanks the fold lines make, their panels and
//! hinges, and what is wrong with the lines, found by [`vectorcraft_three_d::fold::blanks`].

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::PathData;
use vectorcraft_three_d::fold::{Blank, FoldLine, Issue, IssueCode, LineKind, MAX_SEGMENTS, blanks};

use super::super::super::pathops::length_given;
use super::super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_doc, switch_or};
use super::roles::{Role, RoleTable, can_carry, role_targets};
use crate::{Result, Session};

const CHECK: &str = "addon.packaging.check";

/// How far apart the ends of two lines may be and still be joined, in points.
pub(super) const TOLERANCE: (f64, f64, f64) = (0.01, 0.1, 10.0);

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.packaging.check",
        "Check Fold Lines",
        ["Object", "Plug-ins", "Packaging Tools"],
        None,
        "{ids?: [id…] (default: the selection), select?: bool (false), tolerance?: pt 0.01..10 (0.1)} check the fold lines (addon.packaging.roles) among the objects (through groups and clip groups), or, while nothing is selected and ids is left out, every fold line of the document; hidden lines are left out, also in a selected group. Cut lines bound the blanks: each connected piece of them that encloses material is one blank, its outline is its largest closed cut line, and a closed cut line inside it is a hole, so a slot drawn as a rectangle across the outline leaves the same material as a notch drawn in it; open cut lines that meet end to end count as one closed cut line. Crease, Partial Cut and Perforation lines divide a blank into panels; the pieces of them between two panels are that pair's hinge. A cut line end that comes within tolerance of another cut line is joined to it. Issues, in this order: noCut, open, looseEnd (errors), gap (a warning: the gap is closed), tooComplex (an error: more than 65,536 segments or 256 panels in a blank), cutOverCrease (an error), duplicate (two cut lines, or two lines that fold of any role), danglingCrease, noSplit, outside (wholly, or in part for tolerance or more), notStraight (a hinge's pieces are not on one straight line), sliver, ambiguous, loosePanel (joined to the largest panel by no chain of hinges), nonPrinting (warnings). With select, the objects the issues name become the selection, unless no issue names one. Changes nothing in the document → {ok (no error), blanks, panels, hinges, holes, issues: [{severity: error|warning, code, message (English), ids, at?: [x, y]}], scope: ids|selection|document}",
        has_doc,
        check
    )]
}

/// The English message for an issue (the Fold Lines panel shows it in the interface language).
fn message(code: IssueCode, scope: &str) -> &'static str {
    match code {
        IssueCode::NoCut if scope == "document" => "No cut lines are in the document.",
        IssueCode::NoCut if scope == "ids" => "No cut lines are among the given objects.",
        IssueCode::NoCut => "No cut lines are in the selection.",
        IssueCode::Open => "The cut lines do not enclose an area.",
        IssueCode::LooseEnd => "A cut line ends without meeting another cut line.",
        IssueCode::Gap => "A gap between cut lines was closed.",
        IssueCode::TooComplex => "The fold lines are too complex to check.",
        IssueCode::CutOverCrease => "A cut line and a crease lie on top of each other.",
        IssueCode::Duplicate => "Two fold lines lie on top of each other.",
        IssueCode::DanglingCrease => "A crease ends inside a panel.",
        IssueCode::NoSplit => "A crease does not divide two panels.",
        IssueCode::Outside => "A crease lies outside the cut outline, wholly or in part.",
        IssueCode::NotStraight => "The pieces of a hinge are not on one straight line.",
        IssueCode::Sliver => "A gap between cut lines is too narrow to tell apart.",
        IssueCode::Ambiguous => "A cut-out area holds other fold lines or another blank.",
        IssueCode::LoosePanel => "A panel is not joined to the rest by a crease.",
        IssueCode::NonPrinting => "A fold line is on a non-printing layer.",
    }
}

/// Every object of the document that can carry a role and is shown: on a shown layer, in shown
/// groups, and shown itself.
fn shown_targets(d: &Document) -> Vec<NodeId> {
    fn walk(n: &Node, out: &mut Vec<NodeId>) {
        if !n.visible {
            return;
        }
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => children.iter().for_each(|c| walk(c, out)),
            _ if can_carry(n) => out.push(n.id),
            _ => {}
        }
    }
    let mut out = vec![];
    d.layers.iter().for_each(|l| walk(l, &mut out));
    out
}

/// The path of object `n` in document coordinates: a path's own, or a compound path's members'.
fn path_of(n: &Node) -> Option<PathData> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => Some(PathData::new(
            children
                .iter()
                .filter_map(|c| match &c.kind {
                    NodeKind::Path { path, .. } => Some(path.subpaths.iter().cloned()),
                    _ => None,
                })
                .flatten()
                .collect(),
        )),
        _ => None,
    }
}

/// Whether object `id` is on a layer, or a sublayer, that does not print.
fn on_non_printing_layer(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).unwrap_or_default().iter().filter_map(|a| d.node(*a)).any(|n| matches!(n.kind, NodeKind::Layer { printable: false, .. }))
}

fn kind(role: Role) -> LineKind {
    match role {
        Role::Cut => LineKind::Cut,
        Role::Crease => LineKind::Crease,
        Role::PartialCut => LineKind::PartialCut,
        Role::Perforation => LineKind::Perforation,
    }
}

/// The fold lines a check or a fold reads: where they came from (`ids`, `selection` or
/// `document`), the lines, and the ids of those on a non-printing layer.
pub(super) struct Lines {
    pub scope: &'static str,
    pub lines: Vec<FoldLine>,
    pub non_printing: Vec<u64>,
}

/// The tolerance `p` gives for `cmd`, or the default.
pub(super) fn tolerance(p: &Value, cmd: &str) -> Result<f64> {
    match length_given(p, "tolerance", cmd)? {
        None => Ok(TOLERANCE.1),
        Some(t) if (TOLERANCE.0..=TOLERANCE.2).contains(&t) => Ok(t),
        Some(_) => Err(bad(cmd, format!("`tolerance` must be from {} to {} pt", TOLERANCE.0, TOLERANCE.2))),
    }
}

/// The fold lines among the objects `p` names for `cmd` (`ids`, else the selection), or, while
/// nothing is selected and `ids` is left out, every shown fold line of the document. Hidden lines
/// are left out in every scope, members of a selected group included.
pub(super) fn lines(s: &Session, p: &Value, cmd: &str) -> Result<Lines> {
    let given = p.get("ids").is_some_and(|v| !v.is_null());
    let roots = checked_ids_or_selection(s, p, cmd)?;
    let d = &s.doc()?.doc;
    let shown = |ids: Vec<NodeId>| ids.into_iter().filter(|id| d.is_visible(*id)).collect::<Vec<_>>();
    Ok(match (given, roots.is_empty()) {
        (true, _) => lines_of(d, "ids", shown(role_targets(d, &roots, false).0)),
        (false, false) => lines_of(d, "selection", shown(role_targets(d, &roots, false).0)),
        (false, true) => document_lines(d),
    })
}

/// Every shown fold line of `d`.
pub(super) fn document_lines(d: &Document) -> Lines {
    lines_of(d, "document", shown_targets(d))
}

fn lines_of(d: &Document, scope: &'static str, targets: Vec<NodeId>) -> Lines {
    let table = RoleTable::of(d);
    let mut lines = vec![];
    let mut non_printing = vec![];
    let mut segments = 0usize;
    for id in targets {
        // Once the lines have more segments than the check takes, it reports them as too complex:
        // the rest are not copied.
        if segments > MAX_SEGMENTS {
            break;
        }
        let Some(n) = d.node(id) else { continue };
        let (Some(role), Some(path)) = (table.role_of(n), path_of(n)) else { continue };
        if on_non_printing_layer(d, id) {
            non_printing.push(id.0);
        }
        segments = segments.saturating_add(path.subpaths.iter().map(|sp| sp.segment_count()).sum());
        lines.push(FoldLine { path, kind: kind(role), key: id.0, angle: None });
    }
    Lines { scope, lines, non_printing }
}

/// The blanks `lines` make and their issues, the non-printing one last.
pub(super) fn found(lines: &Lines, tolerance: f64) -> (Vec<Blank>, Vec<Issue>) {
    let (found, mut issues) = blanks(&lines.lines, tolerance);
    if !lines.non_printing.is_empty() {
        issues.push(Issue { code: IssueCode::NonPrinting, keys: lines.non_printing.clone(), at: None });
    }
    (found, issues)
}

/// `issues` as the rows `addon.packaging.check` and `addon.packaging.fold` answer with.
pub(super) fn issue_rows(issues: &[Issue], scope: &str) -> Vec<Value> {
    issues
        .iter()
        .map(|i| {
            let mut row = json!({
                "severity": if i.code.is_error() { "error" } else { "warning" },
                "code": i.code.id(),
                "message": message(i.code, scope),
                "ids": i.keys,
            });
            if let (Some(at), Some(o)) = (i.at, row.as_object_mut()) {
                o.insert("at".into(), json!([at.x, at.y]));
            }
            row
        })
        .collect()
}

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let select = switch_or(p, "select", false, CHECK)?;
    let tolerance = tolerance(p, CHECK)?;
    let lines = lines(s, p, CHECK)?;
    let scope = lines.scope;
    let (found, issues) = found(&lines, tolerance);
    let d = &s.doc()?.doc;
    let ok = !issues.iter().any(|i| i.code.is_error());
    let rows = issue_rows(&issues, scope);
    if select {
        let mut ids: Vec<NodeId> = issues.iter().flat_map(|i| i.keys.iter().map(|k| NodeId(*k))).filter(|id| d.node(*id).is_some()).collect();
        ids.sort_unstable_by_key(|id| id.0);
        ids.dedup();
        if !ids.is_empty() {
            s.select(|_, sel| sel.set(ids.iter().copied()))?;
        }
    }
    Ok(json!({
        "ok": ok,
        "blanks": found.len(),
        "panels": found.iter().map(|b| b.panels.len()).sum::<usize>(),
        "hinges": found.iter().map(|b| b.hinges.len()).sum::<usize>(),
        "holes": found.iter().map(|b| b.holes.len()).sum::<usize>(),
        "issues": rows,
        "scope": scope,
    }))
}

#[cfg(test)]
mod tests;
