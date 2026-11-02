//! Objects on Path groups: a `u` … `U` group whose children's dictionaries carry a `Persistent ID`
//! (the path's with `KeyPath` 1, each object's with its `ConstraintArtOrderId` and the angle of its
//! frame, `BBAccumRotation`: radians, counterclockwise in art space), and a dictionary of the
//! `/Document` dictionary's `Constraint Data` array naming them: `Constraint Name`, `Constraint
//! Verison` 1, `Constraint IDs` (the path's id, then the objects' in their order along the path)
//! and `Constraint Dict` with the options (`PivotPosition` 0 to 8, `Rotation` in radians,
//! `IsEquiSpaced`, `StartRangeFraction`, `ObjectsOrder`, `ConstraintArtData`). The group opens
//! with its [`ObjectsOnPath`] record and its art where it is drawn. A record with options the
//! record can't hold (a `Spacing`, uneven spacing, a start other than 0, keys this reader doesn't
//! know), or one that doesn't match one group exactly, leaves the group a plain group.

use std::collections::{HashMap, HashSet};

use vectorcraft_doc::objects_on_path::{Member, ObjectsOnPath, attachable_path};

use super::{Affine, Document, NodeId, NodeKind, Obj, Reader, V};

/// The constraint name of an Objects on Path record.
const NAME: &str = "Adobe Align On Path"; // brand-ok: the record's name in the files
/// Most records read from one file.
const MAX_RECORDS: usize = 1000;
/// Most objects one record names.
const MAX_IDS: usize = 10_000;
/// Most objects whose ids are kept.
const MAX_TAGGED: usize = 100_000;
/// The keys of `Constraint Dict` this reader reads; any other one leaves the group plain.
const KNOWN: [&str; 6] = ["StartRangeFraction", "ConstraintArtData", "ObjectsOrder", "PivotPosition", "IsEquiSpaced", "Rotation"];

/// An Objects on Path record of the `/Document` dictionary.
#[derive(Debug)]
pub(super) struct Record {
    /// The path's id, then the objects' in their order along the path.
    ids: Vec<String>,
    pivot: u8,
    /// Degrees, counterclockwise.
    rotate: f64,
}

/// What an object's dictionary says of its part in an Objects on Path group.
#[derive(Clone, Copy, Debug, Default)]
struct Tag {
    key_path: bool,
    order: Option<f64>,
    /// `BBAccumRotation`: radians, counterclockwise in art space.
    angle: Option<f64>,
}

/// The records and object ids read, for [`apply`].
#[derive(Debug, Default)]
pub(super) struct OnPath {
    records: Vec<Record>,
    /// The objects with each `Persistent ID`.
    ids: HashMap<String, Vec<NodeId>>,
    tags: HashMap<NodeId, Tag>,
}

/// The first number of entry `key`.
fn num(o: &Obj, key: &str) -> Option<f64> {
    o.nums(key).first().copied()
}

/// An Objects on Path record from a dictionary of the `Constraint Data` array: `None` for another
/// constraint, another version, or options the record can't hold.
fn record(o: &Obj) -> Option<Record> {
    if o.text("Constraint Name").as_deref() != Some(NAME) || num(o, "Constraint Verison") != Some(1.0) {
        return None;
    }
    let list = o.obj("Constraint IDs")?;
    if list.entries.len() > MAX_IDS || !list.content.is_empty() {
        return None;
    }
    let ids = list.entries.iter().map(|(_, v)| v.iter().find(|v| matches!(v, V::Str(_))).and_then(V::text)).collect::<Option<Vec<String>>>()?;
    let members = ids.len().checked_sub(1).filter(|n| *n > 0)?;
    let options = o.obj("Constraint Dict")?;
    if options.entries.iter().any(|(k, _)| !k.as_deref().is_some_and(|k| KNOWN.contains(&k))) || !options.content.is_empty() {
        return None;
    }
    if num(options, "IsEquiSpaced") != Some(1.0)
        || !matches!(options.get("StartRangeFraction").map(|_| num(options, "StartRangeFraction")), None | Some(Some(0.0)))
    {
        return None;
    }
    let pivot = num(options, "PivotPosition").filter(|p| p.fract() == 0.0 && (0.0..=8.0).contains(p))? as u8;
    let rotate = num(options, "Rotation").map(f64::to_degrees).filter(|r| r.is_finite())?;
    let order = options.obj("ObjectsOrder")?;
    let order: Vec<f64> = order.entries.iter().flat_map(|(_, v)| v.iter().filter_map(V::num)).collect();
    if order.len() != members || order.iter().enumerate().any(|(i, n)| *n != i as f64) {
        return None;
    }
    let data = options.obj("ConstraintArtData")?;
    if data.entries.len() != members || data.items().count() != members || data.items().any(|d| !d.entries.is_empty() || !d.content.is_empty()) {
        return None;
    }
    Some(Record { ids, pivot, rotate })
}

impl Reader<'_> {
    /// The Objects on Path records of a `Constraint Data` array in the `/Document` dictionary `o`.
    pub(super) fn constraint_data(&mut self, o: &Obj) {
        let Some(list) = super::document::find_entry(o, "Constraint Data", 0) else { return };
        for d in list.items().take(MAX_RECORDS) {
            if self.on_path.records.len() >= MAX_RECORDS {
                return;
            }
            if let Some(r) = record(d) {
                self.on_path.records.push(r);
            }
        }
    }

    /// The `Persistent ID` an object's dictionary `o` gives the last object, with its part in an
    /// Objects on Path group.
    pub(super) fn constraint_art(&mut self, o: &Obj) {
        let Some(uuid) = o.text("Persistent ID") else { return };
        let Some(id) = self.last_object().filter(|n| !matches!(n.kind, NodeKind::Layer { .. })).map(|n| n.id) else { return };
        if self.on_path.tags.len() >= MAX_TAGGED {
            return;
        }
        let angle = o.text("BBAccumRotation").and_then(|s| s.trim().parse::<f64>().ok()).filter(|a| a.is_finite());
        let tag = Tag { key_path: num(o, "KeyPath") == Some(1.0), order: num(o, "ConstraintArtOrderId"), angle };
        self.on_path.tags.insert(id, tag);
        self.on_path.ids.entry(uuid).or_default().push(id);
    }
}

/// Give each group that a record of `found` names exactly its [`ObjectsOnPath`] record.
pub(super) fn apply(doc: &mut Document, found: &OnPath) {
    if found.records.is_empty() {
        return;
    }
    // Where every object with an id is, found in one walk of the tree (setting a record doesn't
    // move anything).
    let at = doc.index_paths(found.tags.keys().copied());
    for r in &found.records {
        // A record that doesn't match leaves the group as it was read.
        let _ = apply_one(doc, found, &at, r);
    }
}

fn apply_one(doc: &mut Document, found: &OnPath, at: &HashMap<NodeId, Vec<usize>>, r: &Record) -> Option<()> {
    // The one object still in the document with the id `uuid` (a commented copy may have gone).
    let resolve = |uuid: &str| -> Option<(NodeId, &Vec<usize>)> {
        let mut live = found.ids.get(uuid)?.iter().filter_map(|id| at.get(id).map(|p| (*id, p)));
        let one = live.next()?;
        live.next().is_none().then_some(one)
    };
    let (first, rest) = r.ids.split_first()?;
    let (path, path_at) = resolve(first)?;
    if !found.tags.get(&path)?.key_path {
        return None;
    }
    let (_, group_at) = path_at.split_last()?;
    let mut seen = HashSet::from([path]);
    let mut members = Vec::with_capacity(rest.len());
    for (i, uuid) in rest.iter().enumerate() {
        let (id, place) = resolve(uuid)?;
        let tag = found.tags.get(&id)?;
        if !seen.insert(id) || place.split_last().map(|(_, up)| up) != Some(group_at) || tag.key_path || tag.order != Some(i as f64) {
            return None;
        }
        // The frame the object is drawn in: turned by its angle (the document's y points down)
        // about the center of its bounds.
        let angle = -tag.angle?;
        let center = doc.node_at(place)?.geometric_bounds()?.center();
        members.push(Member { id, frame: Affine::rotate_about(angle, center), angle });
    }
    let g = doc.node_at(group_at)?;
    let NodeKind::Group { children, clip: false } = &g.kind else { return None };
    if g.extras().objects_on_path.is_some() || children.len() != seen.len() || children.iter().any(|c| !seen.contains(&c.id)) {
        return None;
    }
    attachable_path(children.iter().find(|c| c.id == path)?)?;
    let on_path = ObjectsOnPath { path, members, pivot: r.pivot, rotate: r.rotate };
    doc.node_mut_at(group_at)?.extras_mut().objects_on_path = Some(Box::new(on_path));
    Some(())
}
