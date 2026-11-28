//! Set Fold Angle (`addon.packaging.setFoldAngle`): the angle each fold line folds at. A Crease,
//! Partial Cut or Perforation line keeps its own angle in its data under [`KEY`], as a number of
//! degrees from -180 to 180; without one it folds at its role's angle (90° for a Crease or a
//! Perforation, -90° for a Partial Cut). A positive angle folds away from the printed side.

use std::collections::HashSet;
use std::sync::{Arc, Weak};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_three_d::fold::kept_angle;

use super::super::super::edit::refuse_symbol_part;
use super::super::super::overprint::edit_counted;
use super::super::super::pathops::number_given;
use super::super::super::{CommandSpec, bad, cmd, switch_or};
use super::check::kind;
use super::roles::{Role, RoleTable, editable_members, role_targets, taken_roots};
use crate::{Result, Session};

/// Object data key: a fold line's own fold angle, in degrees. Native save keeps it; SVG export
/// leaves `vc-` keys out.
pub(crate) const KEY: &str = "vc-fold-angle";

const SET_FOLD_ANGLE: &str = "addon.packaging.setFoldAngle";

/// The range a fold angle may take, in degrees.
const RANGE: std::ops::RangeInclusive<f64> = -180.0..=180.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.packaging.setFoldAngle",
        "Set Fold Angle…",
        ["Object", "Plug-ins", "Packaging Tools"],
        None,
        "{angle?: degrees -180..180, reset?: bool (false), ids?: [id…] (default: the selection)} give each Crease, Partial Cut and Perforation line among the objects (addon.packaging.roles; through groups and clip groups; objects that are locked or hidden, in a locked or hidden group or layer, or on a locked artboard are left out and counted as skipped, even when named in ids) its own fold angle, rounded to the millionth of a degree, or with reset: true take it away, so the line folds at its role's angle again: 90° for a Crease or a Perforation, -90° for a Partial Cut. A positive angle folds the panel away from the printed side, a negative one toward it; addon.packaging.fold and the Fold Preview window fold each hinge at the angle of the line with the most length in it. Cut lines and objects without a role are skipped and counted. Give exactly one of angle and reset: true. One undo step, none when nothing changes → {ids (the fold lines changed or kept), changed, skipped}",
        has_fold_line,
        set_fold_angle
    )]
}

/// Why Set Fold Angle has nothing to change.
const NO_FOLD_LINE: &str = "select a crease, partial cut or perforation line that is neither locked nor hidden";

/// The last answer of [`has_fold_line`], with what it was for.
struct Answer {
    /// The document (a weak reference keeps another document from taking the same address).
    doc: Weak<Document>,
    revision: u64,
    /// The objects looked at: the selection, or the `ids` a command was given in its place.
    objects: Vec<NodeId>,
    found: bool,
}

thread_local! {
    /// Menus ask [`has_fold_line`] on every refresh, and its answer changes only with the
    /// document, its revision and the objects looked at.
    static LAST_ANSWER: std::cell::RefCell<Option<Answer>> = const { std::cell::RefCell::new(None) };
}

/// The selection holds a line that folds and that Set Fold Angle can change, or an id of no object.
fn has_fold_line(s: &Session) -> std::result::Result<(), String> {
    refuse_symbol_part(s).map_err(|e| e.to_string())?;
    let st = s.doc().map_err(|e| e.to_string())?;
    let objects = &st.selection.objects;
    let kept = LAST_ANSWER.with_borrow(|last| match last {
        Some(a) if a.revision == st.revision && a.objects == *objects && Weak::ptr_eq(&a.doc, &Arc::downgrade(&st.doc)) => Some(a.found),
        _ => None,
    });
    let found = kept.unwrap_or_else(|| {
        // An id of no object (given as `ids`) is left to the command, which names it.
        let found = objects.iter().any(|id| st.doc.node(*id).is_none()) || any_fold_line(&st.doc, objects);
        LAST_ANSWER.set(Some(Answer { doc: Arc::downgrade(&st.doc), revision: st.revision, objects: objects.clone(), found }));
        found
    });
    if found { Ok(()) } else { Err(NO_FOLD_LINE.into()) }
}

/// Whether the objects `roots` of `d` hold a line that folds and can be changed, looking through
/// groups and clip groups as [`role_targets`] does and stopping at the first. As [`taken_roots`]
/// takes them, an object inside another one given is left out (a crease in a selected layer counts
/// no more than the layer does), a layer is left out, and a path of a compound path stands for the
/// compound path, which holds its role, unless it is locked or hidden.
fn any_fold_line(d: &Document, roots: &[NodeId]) -> bool {
    fn find(n: &Node, hit: &dyn Fn(&Node) -> bool) -> bool {
        match &n.kind {
            // A group's clipping path draws no stroke.
            NodeKind::Group { children, .. } => children.iter().any(|c| !matches!(c.kind, NodeKind::Path { clipping: true, .. }) && find(c, hit)),
            _ => hit(n),
        }
    }
    let table = RoleTable::of(d);
    let editable = d.editable();
    let hit = |n: &Node| table.role_of(n).is_some_and(folds) && editable(n.id);
    let given: HashSet<NodeId> = roots.iter().copied().collect();
    // Each object's place, from the document's lookup of ids.
    let outer = |id: NodeId| -> Option<NodeId> {
        let a = d.ancestry(id)?;
        let (_, up) = a.split_last()?;
        if up.iter().any(|p| given.contains(p)) {
            return None;
        }
        match up.last().copied().filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. }))) {
            // A locked or hidden path of a compound path is left out, as `editable_members` leaves it.
            Some(_) if !editable(id) => None,
            Some(compound) => Some(compound),
            None => Some(id),
        }
    };
    // An object given twice is looked through once.
    let mut seen = HashSet::new();
    roots
        .iter()
        .filter_map(|id| outer(*id))
        .filter(|id| seen.insert(*id))
        .filter_map(|id| d.node(id))
        .filter(|n| !n.is_layer())
        .any(|n| find(n, &hit))
}

/// The text `n`'s data holds under [`KEY`], if any.
fn stored(n: &Node) -> Option<&str> {
    n.extras().attrs.as_deref()?.data.iter().find(|(k, _)| k == KEY).map(|(_, v)| v.as_str())
}

/// `n`'s own fold angle: the number its data holds under [`KEY`], when it is a finite number from
/// -180 to 180, to the millionth of a degree it folds at ([`kept_angle`]). Anything else reads as
/// no angle of its own.
pub(super) fn own_angle(n: &Node) -> Option<f64> {
    stored(n)?.trim().parse::<f64>().ok().filter(|a| a.is_finite() && RANGE.contains(a)).map(kept_angle)
}

/// Whether a line with `role` folds: every role but Cut.
pub(super) fn folds(role: Role) -> bool {
    kind(role).fold_angle().is_some()
}

/// Writes `angle` (`None`: takes it away) into object `id`'s data → whether its data changed.
fn store(d: &mut Document, id: NodeId, angle: Option<f64>) -> bool {
    // `+ 0.0` turns -0 into 0, so a line set to 0° reads "0".
    let text = angle.map(|a| format!("{}", a + 0.0));
    // Read first: taking the node to change it copies it, its geometry included. An angle stored
    // as other text for the same number ("45.0") is kept; a reset removes any stored text.
    let same = |n: &Node| match angle {
        Some(a) => own_angle(n) == Some(a),
        None => stored(n).is_none(),
    };
    if d.node(id).is_none_or(same) {
        return false;
    }
    let Some(n) = d.node_mut(id) else { return false };
    n.edit_attrs(|a| match text {
        Some(text) => match a.data.iter_mut().find(|(k, _)| k == KEY) {
            Some(e) => e.1 = text,
            None => a.data.push((KEY.into(), text)),
        },
        None => a.data.retain(|(k, _)| k != KEY),
    });
    true
}

fn set_fold_angle(s: &mut Session, p: &Value) -> Result<Value> {
    let angle = number_given(p, "angle", SET_FOLD_ANGLE)?;
    let reset = switch_or(p, "reset", false, SET_FOLD_ANGLE)?;
    let angle = match (angle, reset) {
        (Some(_), true) => return Err(bad(SET_FOLD_ANGLE, "give `angle` or `reset: true`, not both")),
        (None, false) => return Err(bad(SET_FOLD_ANGLE, "missing `angle`")),
        (Some(a), false) if !RANGE.contains(&a) => return Err(bad(SET_FOLD_ANGLE, "`angle` must be from -180 to 180 degrees")),
        // Kept to the millionth of a degree, the angle the line folds at.
        (a, _) => a.map(kept_angle),
    };
    let (roots, named) = taken_roots(s, p, SET_FOLD_ANGLE)?;
    let d = &s.doc()?.doc;
    let (targets, skipped) = role_targets(d, &roots, false);
    let mut skipped = skipped + named;
    let mut ids = folding(d, &RoleTable::of(d), &targets);
    {
        let editable = d.editable();
        ids.retain(|id| editable(*id));
    }
    skipped += targets.len() - ids.len();
    if ids.is_empty() {
        return Err(bad(SET_FOLD_ANGLE, NO_FOLD_LINE));
    }
    let label = if angle.is_some() { "Set Fold Angle" } else { "Reset Fold Angle" };
    let changed = edit_counted(s, label, |d| Ok(ids.iter().filter(|id| store(d, **id, angle)).count()))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "changed": changed, "skipped": skipped }))
}

/// The lines among `ids` of `d` that fold (Crease, Partial Cut, Perforation), in order.
fn folding(d: &Document, table: &RoleTable, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter().copied().filter(|id| d.node(*id).and_then(|n| table.role_of(n)).is_some_and(folds)).collect()
}

/// What Set Fold Angle can change among the objects `roots` of `d` (through groups and clip
/// groups, leaving out what is locked or hidden, as `addon.packaging.setFoldAngle` does).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FoldAngles {
    /// How many lines that fold it can change.
    pub lines: usize,
    /// The angle they all fold at (their own or their role's); `None` when they differ or there
    /// are none.
    pub angle: Option<f64>,
    /// How many of them have an angle of their own.
    pub own: usize,
    /// How many of them hold text under [`KEY`], an angle or not: what `reset: true` takes away.
    pub stored: usize,
}

/// The [`FoldAngles`] of the selected objects of the active document, as Set Fold Angle reads them
/// with no `ids` (nothing, while it would refuse the selection).
pub fn selection_fold_angles(s: &Session) -> FoldAngles {
    match (taken_roots(s, &Value::Null, SET_FOLD_ANGLE), s.doc()) {
        (Ok((roots, _)), Ok(st)) => fold_angles(&st.doc, &roots),
        _ => FoldAngles::default(),
    }
}

/// The lines Set Fold Angle changes among the selected objects of the active document, as it takes
/// them with no `ids` (none, while it would refuse the selection). Given back as `ids`, they are the
/// lines it changes.
pub fn selection_fold_lines(s: &Session) -> Vec<NodeId> {
    match (taken_roots(s, &Value::Null, SET_FOLD_ANGLE), s.doc()) {
        (Ok((roots, _)), Ok(st)) => changeable(&st.doc, &roots).into_iter().map(|(n, _)| n.id).collect(),
        _ => Vec::new(),
    }
}

/// The [`FoldAngles`] of the outermost objects `roots` of `d`.
pub(super) fn fold_angles(d: &Document, roots: &[NodeId]) -> FoldAngles {
    let mut f = FoldAngles::default();
    for (n, role) in changeable(d, roots) {
        f.add(n, role);
    }
    f
}

/// The lines that fold among the outermost objects `roots` of `d` that Set Fold Angle can change,
/// with their roles, in order.
fn changeable<'a>(d: &'a Document, roots: &[NodeId]) -> Vec<(&'a Node, Role)> {
    let table = RoleTable::of(d);
    let editable = d.editable();
    let targets = role_targets(d, &editable_members(d, roots.to_vec()).0, false).0;
    // The role first: whether an object can be edited can take its bounds to find out.
    targets
        .iter()
        .filter_map(|id| d.node(*id))
        .filter_map(|n| table.role_of(n).filter(|r| folds(*r)).filter(|_| editable(n.id)).map(|r| (n, r)))
        .collect()
}

impl FoldAngles {
    /// Counts line `n`, which has `role` and can be changed; a line that does not fold is left out.
    pub(super) fn add(&mut self, n: &Node, role: Role) {
        let Some(default) = kind(role).fold_angle() else { return };
        let own = own_angle(n);
        let a = own.unwrap_or(default);
        self.angle = if self.lines == 0 { Some(a) } else { self.angle.filter(|b| *b == a) };
        self.lines += 1;
        self.own += usize::from(own.is_some());
        self.stored += usize::from(stored(n).is_some());
    }
}

#[cfg(test)]
mod tests;
