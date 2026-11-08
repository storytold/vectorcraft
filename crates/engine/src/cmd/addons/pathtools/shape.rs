//! The Parametric Shape Tool's commands (`addon.pathtools.shape.*`): draw a parametric shape, change
//! one's numbers, and turn plain rectangles, ellipses and regular polygons into them. A parametric
//! shape is an ordinary path whose numbers sit in one entry of its object data
//! (`vectorcraft_tools::addons::parametric_shape`), which SVG export leaves out.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Point};
use vectorcraft_pathops::addons::pathtools::shapes::{ParametricShape, ShapeKind, recognise};
use vectorcraft_tools::addons::parametric_shape::{CREATE, EDIT, KEY, apply, read, stored, to_json};

use super::super::super::{CommandSpec, bad, checked_id, checked_ids_or_selection, cmd, has_doc, has_selection};
use crate::{EngineError, Result, Session};

/// The undo step's name and the command's label.
const CONVERT: &str = "Convert to Parametric Shape";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.pathtools.shape.create",
            "Parametric Shape",
            [],
            None,
            "{center: [x, y], kind?: \"rectangle\"|\"ellipse\"|\"polygon\" (default)|\"star\"|\"ring\"|\"arrow\"|\"cross\"|\"heart\"|\"gear\"|\"bubble\" (a speech bubble), width?, height?: 0.01..16384 pt (100), rotation?: degrees clockwise (0: upright), count?: 3..1000 (a polygon's sides, a star's points, a gear's teeth), a?, b?: the kind's own values (star: inner radius 0.01..1 of the outer, 0.5; ring: hole 0.01..0.99 of the size, 0.5; arrow: head length and shaft width 0.05..0.95, 0.4 and 0.4; cross: bar offset −0.9..0.9 (0, − up) and arm thickness 0.05..0.95 (1/3); heart: notch depth and point roundness 0..1 (0.4 and 0); gear: tooth height 0.01..0.9 (0.2); bubble: tail height 0.05..0.9 (0.25) and tail tip −1..1 across (−0.4)), radius?: corner radius 0..16384 pt (0; no corner rounds past the middle of a side)} draw a parametric shape with the current paint, selected, as one undo step; it stays editable with addon.pathtools.shape.set until its anchors are edited (moving, turning or scaling it keeps it) → {id, shape}",
            has_doc,
            create
        ),
        cmd!(
            "addon.pathtools.shape.set",
            "Edit Parametric Shape",
            [],
            None,
            "{id, kind?, center?, width?, height?, rotation?, count?, a?, b?, radius?} change a parametric shape's numbers (as addon.pathtools.shape.create takes them; a new kind takes its own defaults for the values not given), as one undo step; with none, or the same ones, nothing changes → {id, shape (its numbers now)}",
            has_doc,
            set
        ),
        cmd!(
            "addon.pathtools.shape.convert",
            "Convert to Parametric Shape",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{ids?: [id…] (default: the selection)} make each plain closed path that is a rectangle, an ellipse or a polygon with equal sides and corners (stretched along its own axes at most) a parametric shape of the same outline, as one undo step → {ids (converted)}",
            has_selection,
            convert
        ),
    ]
}

/// Give path node `n` the outline `path` of shape `s` and keep its numbers; a live shape goes.
fn shape_node(n: &mut Node, s: &ParametricShape, path: PathData) {
    let entry = stored(s, &path);
    if let NodeKind::Path { path: p, live, .. } = &mut n.kind {
        *p = path;
        *live = None;
    }
    n.edit_attrs(|a| match a.data.iter_mut().find(|(k, _)| k == KEY) {
        Some(e) => e.1 = entry,
        None => a.data.push((KEY.into(), entry)),
    });
}

/// The outline of `s`, checked already.
fn outline(s: &ParametricShape, cmd: &str) -> Result<PathData> {
    s.path().ok_or_else(|| bad(cmd, "the shape's numbers are out of range"))
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.shape.create";
    if p.get("center").is_none_or(Value::is_null) {
        return Err(bad(C, "`center` must be [x, y]"));
    }
    let shape = apply(ParametricShape::new(ShapeKind::Polygon, Point::ZERO, 100.0, 100.0), p).map_err(|e| bad(C, e))?;
    let path = outline(&shape, C)?;
    let parent = s.doc()?.target_parent()?;
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    // shortcut: always on top of the insertion layer, as Draw Normal; honour Draw Behind/Inside if asked.
    let id = s.edit(CREATE, |d, sel| {
        let id = d.alloc_id();
        let mut n = Node::new(id, NodeKind::Path { path: PathData::default(), rule: FillRule::NonZero, live: None, clipping: false, guide: false });
        shape_node(&mut n, &shape, path);
        look.apply(d, &mut n);
        d.insert(parent, usize::MAX, n)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0, "shape": to_json(&shape) }))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.shape.set";
    let id = checked_id(s, p.get("id").unwrap_or(&Value::Null), C)?;
    let d = &s.doc()?.doc;
    if !d.is_editable(id) {
        return Err(bad(C, "the path is locked or hidden"));
    }
    let old = d.node(id).and_then(read).ok_or_else(|| bad(C, "the object is not an editable parametric shape"))?;
    let shape = apply(old, p).map_err(|e| bad(C, e))?;
    if shape != old {
        let path = outline(&shape, C)?;
        s.edit(EDIT, |d, sel| {
            shape_node(d.node_mut(id).ok_or(EngineError::NoNode(id))?, &shape, path);
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
            Ok(())
        })?;
    }
    Ok(json!({ "id": id.0, "shape": to_json(&shape) }))
}

fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.pathtools.shape.convert";
    let roots = checked_ids_or_selection(s, p, C)?;
    let d = &s.doc()?.doc;
    let mut found = vec![];
    for id in roots {
        let Some(n) = d.node(id).filter(|n| d.is_editable(id) && read(n).is_none()) else { continue };
        if let NodeKind::Path { path, guide: false, clipping: false, .. } = &n.kind
            && let Some(shape) = recognise(path)
        {
            found.push((id, shape, outline(&shape, C)?));
        }
    }
    if found.is_empty() {
        return Err(bad(C, "select plain rectangles, ellipses or regular polygons to convert"));
    }
    let ids: Vec<u64> = found.iter().map(|(id, ..)| id.0).collect();
    s.edit(CONVERT, |d, sel| {
        for (id, shape, path) in found {
            shape_node(d.node_mut(id).ok_or(EngineError::NoNode(id))?, &shape, path);
            sel.anchors.remove(&id);
            sel.segments.remove(&id);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids }))
}
