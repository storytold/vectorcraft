//! Fold line roles (`addon.packaging.setRole`, `addon.packaging.roles`,
//! `addon.packaging.setRoleSwatch`). A path, compound path or live shape has a role when its top
//! stroke is painted with the swatch the document's role table gives that role, at any tint. The
//! table lives in [`Document::unknown`] under [`ROLES_KEY`], `{cut, crease, partialCut,
//! perforation}` each naming a swatch; a role it leaves out, or names with something other than a
//! string, takes its default swatch, named after the role.

use serde_json::{Map, Value, json};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::super::super::edit::{outermost, refuse_symbol_part, roots_of};
use super::super::super::overprint::edit_counted;
use super::super::super::{CommandSpec, bad, checked_ids_or_selection, cmd, has_doc, has_selection, switch_or, text_param};
use crate::{Result, Session};

/// Document unknown key: the role table, `{role id: swatch name}`.
pub(crate) const ROLES_KEY: &str = "packagingRoles";

const SET_ROLE: &str = "addon.packaging.setRole";
const ROLES: &str = "addon.packaging.roles";
const SET_ROLE_SWATCH: &str = "addon.packaging.setRoleSwatch";

/// What a fold line does when the blank is cut and folded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// Separates the material.
    Cut,
    /// A fold.
    Crease,
    /// A fold cut part way through the board.
    PartialCut,
    /// A fold along a line of cuts.
    Perforation,
}

impl Role {
    pub(crate) const ALL: [Role; 4] = [Role::Cut, Role::Crease, Role::PartialCut, Role::Perforation];

    /// The role's id in params and in the role table.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Role::Cut => "cut",
            Role::Crease => "crease",
            Role::PartialCut => "partialCut",
            Role::Perforation => "perforation",
        }
    }

    /// The role's name, which is also its default swatch's name.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Role::Cut => "Cut",
            Role::Crease => "Crease",
            Role::PartialCut => "Partial Cut",
            Role::Perforation => "Perforation",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.id() == id)
    }

    /// The color a missing role swatch is made with (VectorCraft's own values): Cut 100% magenta,
    /// Crease 100% cyan, Partial Cut 100% yellow, Perforation 50% magenta and 50% cyan.
    fn color(self) -> Color {
        match self {
            Role::Cut => Color::cmyk(0.0, 1.0, 0.0, 0.0),
            Role::Crease => Color::cmyk(1.0, 0.0, 0.0, 0.0),
            Role::PartialCut => Color::cmyk(0.0, 0.0, 1.0, 0.0),
            Role::Perforation => Color::cmyk(0.5, 0.5, 0.0, 0.0),
        }
    }

    fn index(self) -> usize {
        Role::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// The swatch each role is carried by, in [`Role::ALL`] order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleTable([String; 4]);

impl RoleTable {
    /// The document's role table, with each missing or damaged entry at its default. When a damaged
    /// table names one swatch for two roles, the later role takes its default name instead, or
    /// "Crease 2" and so on when an earlier role has that.
    pub(crate) fn of(d: &Document) -> RoleTable {
        let stored = d.unknown.get(ROLES_KEY).and_then(Value::as_object);
        let mut names: Vec<String> = Vec::with_capacity(Role::ALL.len());
        for r in Role::ALL {
            let name = match stored.and_then(|m| m.get(r.id())).and_then(Value::as_str) {
                Some(name) if !name.is_empty() && !names.iter().any(|n| n == name) => name.to_string(),
                _ => {
                    let base = r.name();
                    std::iter::once(base.to_string())
                        .chain((2..).map(|i| format!("{base} {i}")))
                        .find(|n| !names.contains(n))
                        .unwrap_or_else(|| base.to_string())
                }
            };
            names.push(name);
        }
        RoleTable(Role::ALL.map(|r| names.get(r.index()).cloned().unwrap_or_else(|| r.name().to_string())))
    }

    /// The swatch of `role`.
    pub(crate) fn swatch(&self, role: Role) -> &str {
        self.0.get(role.index()).map_or("", String::as_str)
    }

    /// The role swatch `name` carries.
    pub(crate) fn role_of_swatch(&self, name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| self.swatch(*r) == name)
    }

    /// The role of object `n`: the role of the swatch its top stroke is painted with.
    pub(crate) fn role_of(&self, n: &Node) -> Option<Role> {
        if !can_carry(n) {
            return None;
        }
        match &n.appearance.stroke()?.paint {
            Paint::Solid { swatch: Some(name), .. } => self.role_of_swatch(name),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        Value::Object(Role::ALL.into_iter().map(|r| (r.id().to_string(), json!(self.swatch(r)))).collect::<Map<_, _>>())
    }
}

/// Swatch `from` of `d` is now named `to`, and `global` says whether it is still a global color.
/// A role it carried keeps it under the new name, as the strokes linked to it do; when it stopped
/// being global, which unlinks those strokes, the role moves to a new name that `setRole` makes a
/// swatch for. Another role that named `to` without such a swatch moves to a new name too, so no
/// two roles share a swatch.
pub(crate) fn role_swatch_edited(d: &mut Document, from: &str, to: &str, global: bool) {
    let before = RoleTable::of(d);
    let mut table = before.clone();
    for r in Role::ALL {
        let name = table.swatch(r).to_string();
        let moved = if name == from && global {
            Some(to.to_string())
        } else if name == from || (name == to && from != to) {
            Some(free_role_name(d, &table, r))
        } else {
            None
        };
        if let (Some(new), Some(slot)) = (moved, table.0.get_mut(r.index())) {
            *slot = new;
        }
    }
    if table != before {
        d.unknown.insert(ROLES_KEY.to_string(), table.to_value());
    }
}

/// Swatches `merged` of `d` were merged into swatch `kept`, and `linked` says whether their strokes
/// now link to it (it is a global color). When `kept` carries no role, it takes the role of the
/// first merged swatch that carried one, so the lines of that role keep it; a merged swatch's other
/// role keeps its name, which the next `setRole` makes a swatch for.
pub(crate) fn role_swatches_merged(d: &mut Document, merged: &[String], kept: &str, linked: bool) {
    let mut table = RoleTable::of(d);
    if !linked || table.role_of_swatch(kept).is_some() {
        return;
    }
    let Some(role) = merged.iter().find_map(|m| table.role_of_swatch(m)) else { return };
    if let Some(slot) = table.0.get_mut(role.index()) {
        *slot = kept.to_string();
    }
    d.unknown.insert(ROLES_KEY.to_string(), table.to_value());
}

/// A name for `role`'s swatch that no swatch, color group or other role of `table` uses: the
/// role's name, else "Cut 2", "Cut 3"… The document holds finitely many names, so the search
/// always ends at a free one.
fn free_role_name(d: &Document, table: &RoleTable, role: Role) -> String {
    let free = |n: &str| !d.swatch_name_taken(n) && Role::ALL.into_iter().all(|r| r == role || table.swatch(r) != n);
    let base = role.name();
    if free(base) {
        return base.to_string();
    }
    (2usize..).map(|i| format!("{base} {i}")).find(|n| free(n)).unwrap_or_else(|| base.to_string())
}

/// Whether `n` can carry a role: a path (a live shape too) that is neither a clipping path nor a
/// guide, or a compound path.
pub(super) fn can_carry(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { clipping, guide, .. } => !clipping && !guide,
        NodeKind::Compound { .. } => true,
        _ => false,
    }
}

/// The objects a command that changes roles or angles takes for params `p`: `ids` (the outermost of
/// them), or else the selection (as `selected_roots` takes it), with a path of a compound path that
/// is locked or hidden, or in a locked or hidden group or layer, or on a locked artboard, left out
/// before it stands for its compound, and how many such paths were left out, each once.
pub(super) fn taken_roots(s: &Session, p: &Value, command: &str) -> Result<(Vec<NodeId>, usize)> {
    if p.get("ids").is_some_and(|v| !v.is_null()) {
        let roots = checked_ids_or_selection(s, p, command)?;
        return Ok(editable_members(&s.doc()?.doc, roots));
    }
    refuse_symbol_part(s)?;
    let st = s.doc()?;
    // An object inside another selected one is left out first, as it is from `ids`.
    let (kept, left) = editable_members(&st.doc, outermost(&st.doc, st.selection.in_paint_order(&st.doc)));
    Ok((roots_of(&st.doc, kept), left))
}

/// `ids` less the paths of compound paths among them that are locked or hidden (or in a locked or
/// hidden group or layer, or on a locked artboard), and how many different ones were left out.
pub(super) fn editable_members(d: &Document, ids: Vec<NodeId>) -> (Vec<NodeId>, usize) {
    let ok = d.editable();
    let member = |id: NodeId| d.parent_of(id).is_some_and(|p| matches!(d.node(p).map(|n| &n.kind), Some(NodeKind::Compound { .. })));
    let mut left = std::collections::HashSet::new();
    let kept = ids
        .into_iter()
        .filter(|id| {
            let out = member(*id) && !ok(*id);
            if out {
                left.insert(*id);
            }
            !out
        })
        .collect();
    (kept, left.len())
}

/// The objects under the outermost objects `roots` that can carry a role, through groups and clip
/// groups, each once, and
/// how many of `roots` and their members can't (text, images, symbol instances and other objects).
/// With `editable`, objects that are locked or hidden, are in a locked or hidden group or layer, or
/// are on a locked artboard are left out and counted as skipped.
pub(super) fn role_targets(d: &Document, roots: &[NodeId], editable: bool) -> (Vec<NodeId>, usize) {
    fn walk(n: &Node, out: &mut Vec<NodeId>, skipped: &mut usize) {
        match &n.kind {
            NodeKind::Group { children, .. } => {
                for c in children {
                    // A group's clipping path draws no stroke: it is skipped.
                    if matches!(c.kind, NodeKind::Path { clipping: true, .. }) {
                        *skipped += 1;
                    } else {
                        walk(c, out, skipped);
                    }
                }
            }
            _ if can_carry(n) => out.push(n.id),
            _ => *skipped += 1,
        }
    }
    let (mut out, mut skipped) = (vec![], 0);
    // A path of a compound path is painted with the compound's appearance: the compound takes the
    // role. `roots` come from `checked_ids_or_selection`, which keeps only outermost objects, so a
    // group and its member are never both roots.
    let mut seen = std::collections::HashSet::new();
    let compound_of = |id: NodeId| d.parent_of(id).filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. })));
    for n in roots.iter().map(|id| compound_of(*id).unwrap_or(*id)).filter(|id| seen.insert(*id)).filter_map(|id| d.node(id)) {
        walk(n, &mut out, &mut skipped);
    }
    if editable {
        // Named directly or not, a locked or hidden object, or one in a locked or hidden group or
        // layer or on a locked artboard, stays as it is.
        let ok = d.editable();
        let before = out.len();
        out.retain(|id| ok(*id));
        skipped += before - out.len();
    }
    (out, skipped)
}

/// Every object of the document that can carry a role, through layers and groups.
fn document_targets(d: &Document) -> Vec<NodeId> {
    fn walk(n: &Node, out: &mut Vec<NodeId>) {
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

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.packaging.setRole",
            "Make Fold Line",
            ["Object", "Plug-ins", "Packaging Tools"],
            None,
            "{role: cut|crease|partialCut|perforation|none, ids?: [id…] (default: the selection), overprint?: bool (true)} give each path, compound path and live shape among the objects (through groups and clip groups; objects that are locked or hidden, in a locked or hidden group or layer, or on a locked artboard are left out and counted as skipped, even when named in ids) a fold line role: its top stroke is painted with the role's swatch (addon.packaging.roles) at 100% and shown, a path without a stroke gets a 1 pt stroke, and the stroke overprints, or with overprint: false does not. A missing role swatch is made as a global spot swatch (Cut 100% magenta, Crease 100% cyan, Partial Cut 100% yellow, Perforation 50% magenta and 50% cyan, in the document's color mode; named \"Cut 2\" and so on, in the role table too, when a color group or a swatch that can't carry a role, such as a process color that is not global, has the name). none takes the role away: a role stroke keeps the color it shows, no longer linked to the swatch, and stops overprinting. A line keeps its own fold angle (addon.packaging.setFoldAngle) through a role change and through none. Text, images, symbol instances and other objects are skipped and counted. One undo step, none when nothing changes → {ids (the objects that can carry a role), changed, skipped}",
            has_selection,
            set_role
        ),
        cmd!(
            query "addon.packaging.roles",
            "Fold Lines",
            [],
            None,
            "{ids?: [id…] (default: the selection)} the fold line roles → {roles: [{role: cut|crease|partialCut|perforation, name, swatch (the swatch that carries it), exists (the document has the swatch), count}], scope: ids|selection|document, selected (objects given or selected), foldLines, foldAngle, ownAngles, storedAngles}; count is how many of the paths, compound paths and live shapes among the objects (through groups, locked and hidden members included, each object once) have the role, or among every object of the document while nothing is selected and ids is left out. foldLines is how many of the Crease, Partial Cut and Perforation lines among them addon.packaging.setFoldAngle can change (those not locked or hidden; none while nothing is selected and ids is left out, or while the selection holds only a layer and objects in it), foldAngle the angle in degrees they all fold at (their own or their role's), null when they fold at different angles or there are none, ownAngles how many of them have an angle of their own, and storedAngles how many hold stored angle text, an angle or not, which setFoldAngle's reset: true removes",
            has_doc,
            roles
        ),
        cmd!(
            "addon.packaging.setRoleSwatch",
            "Role Swatches…",
            ["Object", "Plug-ins", "Packaging Tools"],
            None,
            "{role: cut|crease|partialCut|perforation, swatch: a global or spot color swatch's name} make the swatch carry the role, in the document's role table, as one undo step (none when it already does). Objects stroked with the role's old swatch keep their color and no longer have the role. Refused: a missing swatch, a gradient or pattern swatch, a process color that is not global, a tint swatch, [None], [Registration], and a swatch another role has → {role, swatch, changed}",
            has_doc,
            set_role_swatch
        ),
    ]
}

/// The role `p` gives: `Some(None)` for `none` when `none` is allowed.
fn role_param(p: &Value, cmd: &str, none: bool) -> Result<Option<Role>> {
    let id = text_param(p, "role", cmd)?.ok_or_else(|| bad(cmd, "missing `role`"))?;
    match Role::from_id(id) {
        Some(r) => Ok(Some(r)),
        None if none && id == "none" => Ok(None),
        None if none => Err(bad(cmd, format!("unknown role `{id}`: cut, crease, partialCut, perforation or none"))),
        None => Err(bad(cmd, format!("unknown role `{id}`: cut, crease, partialCut or perforation"))),
    }
}

/// Why swatch `name` of `d` can't carry a role, if it can't.
fn unfit_swatch(d: &Document, name: &str) -> Option<String> {
    if name == REGISTRATION || name == "[None]" {
        return Some(format!("`{name}` can't carry a role"));
    }
    let Some(sw) = d.swatch(name) else { return Some(format!("no swatch `{name}`")) };
    match &sw.paint {
        Paint::Solid { swatch: Some(_), .. } => Some(format!("`{name}` is a tint swatch: pick its base or another global or spot color")),
        Paint::Solid { .. } if sw.global => None,
        Paint::Solid { .. } => Some(format!("`{name}` is a process color: pick a global or spot color")),
        _ => Some(format!("`{name}` isn't a color swatch")),
    }
}

/// `role`'s swatch in `d`, made as a global spot swatch in the document's color mode when `d`
/// lacks it → its paint at 100%. When a color group or a swatch that can't carry a role (a process
/// color that is not global, a tint, a gradient) has the swatch's name, the swatch is made under a
/// free name, which the role table then gives the role.
fn role_paint(d: &mut Document, role: Role) -> Result<Paint> {
    let mut table = RoleTable::of(d);
    let current = table.swatch(role);
    let taken = match d.swatch(current) {
        Some(_) => unfit_swatch(d, current).is_some(),
        None => d.swatch_name_taken(current),
    };
    if taken {
        let free = free_role_name(d, &table, role);
        if let Some(slot) = table.0.get_mut(role.index()) {
            *slot = free;
        }
        d.unknown.insert(ROLES_KEY.to_string(), table.to_value());
    }
    let name = table.swatch(role);
    if d.swatch(name).is_none() {
        let model = d.color_mode.model();
        let color = role.color();
        let color = if color.model() == model {
            color
        } else {
            let cms = vectorcraft_color::cms::active();
            cms.convert(&color, model, cms.settings().intent)
        };
        d.swatches.push(Swatch { name: name.to_string(), paint: Paint::solid(color), global: true, spot: true });
    }
    if let Some(why) = unfit_swatch(d, name) {
        return Err(bad(SET_ROLE, format!("{why} (the {} role's swatch; addon.packaging.setRoleSwatch picks another)", role.name())));
    }
    d.tint_paint(name, 1.0).ok_or_else(|| bad(SET_ROLE, format!("`{name}` isn't a global or spot color")))
}

/// Gives object `id` of `d` `role`'s `paint` (`None`: takes its role away) → whether it changed.
fn give_role(d: &mut Document, table: &RoleTable, id: NodeId, paint: Option<&Paint>, overprint: bool) -> bool {
    let Some(n) = d.node_mut(id) else { return false };
    let had = table.role_of(n);
    let mut a = (*n.appearance).clone();
    match paint {
        Some(paint) => {
            a.set_stroke(paint.clone());
            if let Some(st) = a.stroke_mut() {
                if st.width <= 0.0 {
                    st.width = 1.0;
                }
                // A hidden stroke would carry the role without drawing or printing the line.
                st.visible = true;
                st.overprint = overprint;
            }
        }
        None => {
            if had.is_none() {
                return false;
            }
            if let Some(st) = a.stroke_mut() {
                if let Paint::Solid { color, .. } = st.paint {
                    st.paint = Paint::solid(color);
                }
                st.overprint = false;
            }
        }
    }
    if a == *n.appearance {
        return false;
    }
    n.appearance = vectorcraft_doc::Shared::new(a);
    true
}

fn set_role(s: &mut Session, p: &Value) -> Result<Value> {
    let role = role_param(p, SET_ROLE, true)?;
    let overprint = switch_or(p, "overprint", true, SET_ROLE)?;
    let (roots, left) = taken_roots(s, p, SET_ROLE)?;
    let (ids, skipped) = role_targets(&s.doc()?.doc, &roots, true);
    let skipped = skipped + left;
    if ids.is_empty() {
        return Err(bad(SET_ROLE, "select a path that is neither locked nor hidden: text, images and symbol instances can't have a fold line role"));
    }
    let label = role.map_or("Remove Fold Line Role", |_| "Make Fold Line");
    let mut changed = 0;
    edit_counted(s, label, |d| {
        let before = (d.swatches.len(), d.unknown.get(ROLES_KEY).cloned());
        let paint = role.map(|r| role_paint(d, r)).transpose()?;
        // A role swatch made again (its strokes may still name it) is a change of its own.
        let made = (d.swatches.len(), d.unknown.get(ROLES_KEY).cloned()) != before;
        let table = RoleTable::of(d);
        changed = ids.iter().filter(|id| give_role(d, &table, **id, paint.as_ref(), overprint)).count();
        Ok(changed + usize::from(made))
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "changed": changed, "skipped": skipped }))
}

fn roles(s: &mut Session, p: &Value) -> Result<Value> {
    let given = p.get("ids").is_some_and(|v| !v.is_null());
    // The objects Set Fold Angle takes: a locked or hidden path of a compound path does not stand
    // for its compound there. They are the objects the counts are of unless such a path was left
    // out, and only then are those found apart.
    let (taken, left) = taken_roots(s, p, ROLES)?;
    let (roots, angle_roots) = if left == 0 { (taken, vec![]) } else { (checked_ids_or_selection(s, p, ROLES)?, taken) };
    let st = s.doc()?;
    let d = &st.doc;
    let (scope, ids) = match (given, roots.is_empty()) {
        (true, _) => ("ids", role_targets(d, &roots, false).0),
        (false, false) => ("selection", role_targets(d, &roots, false).0),
        (false, true) => ("document", document_targets(d)),
    };
    let whole = scope == "document";
    // The lines Set Fold Angle can change are those of the objects it takes: with nothing selected,
    // or a selection that holds no such object (only a layer and what is in it), it takes none,
    // though the counts are the document's.
    let angles_of_all = given || !roots.is_empty();
    let table = RoleTable::of(d);
    let mut counts = [0usize; 4];
    let editable = d.editable();
    let mut angles = super::angle::FoldAngles::default();
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        let Some(r) = table.role_of(n) else { continue };
        if let Some(c) = counts.get_mut(r.index()) {
            *c += 1;
        }
        if left == 0 && angles_of_all && super::angle::folds(r) && editable(n.id) {
            angles.add(n, r);
        }
    }
    if !whole && left > 0 {
        angles = super::angle::fold_angles(d, &angle_roots);
    }
    let rows: Vec<Value> = Role::ALL
        .into_iter()
        .zip(counts)
        .map(|(r, count)| {
            let swatch = table.swatch(r);
            json!({ "role": r.id(), "name": r.name(), "swatch": swatch, "exists": d.swatch(swatch).is_some(), "count": count })
        })
        .collect();
    Ok(json!({
        "roles": rows, "scope": scope, "selected": roots.len(),
        "foldLines": angles.lines, "foldAngle": angles.angle, "ownAngles": angles.own, "storedAngles": angles.stored,
    }))
}

fn set_role_swatch(s: &mut Session, p: &Value) -> Result<Value> {
    let Some(role) = role_param(p, SET_ROLE_SWATCH, false)? else { return Err(bad(SET_ROLE_SWATCH, "missing `role`")) };
    let name = text_param(p, "swatch", SET_ROLE_SWATCH)?.ok_or_else(|| bad(SET_ROLE_SWATCH, "missing `swatch`"))?;
    let d = &s.doc()?.doc;
    if let Some(why) = unfit_swatch(d, name) {
        return Err(bad(SET_ROLE_SWATCH, why));
    }
    let mut table = RoleTable::of(d);
    if let Some(other) = Role::ALL.into_iter().find(|r| *r != role && table.swatch(*r) == name) {
        return Err(bad(SET_ROLE_SWATCH, format!("`{name}` already carries the {} role", other.name())));
    }
    if let Some(slot) = table.0.get_mut(role.index()) {
        *slot = name.to_string();
    }
    let value = table.to_value();
    let changed = RoleTable::of(d) != table;
    if changed {
        s.edit("Role Swatches", |d, _| {
            d.unknown.insert(ROLES_KEY.to_string(), value);
            Ok(())
        })?;
    }
    Ok(json!({ "role": role.id(), "swatch": name, "changed": changed }))
}
