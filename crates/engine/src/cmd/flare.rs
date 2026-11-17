//! Flares (the Flare tool): `shape.flare` draws a group of radial-gradient paths that keeps the
//! settings it was drawn with ([`FlareSpec`]), and `shape.editFlare` draws it again from them.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::flare::{DEFAULT_SEED, FlareSpec, json_close, part_node, parts};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_geom::{Affine, Point, Vec2};

use super::{EngineError, Result, Session, bad, check_numbers, f64_or, f64_req, given_objects, number_or};

const C: &str = "shape.editFlare";

/// `shape.flare`'s settings besides the center, the rings' end point, the counts and the seed:
/// (key, default).
const SETTINGS: [(&str, f64); 9] = [
    ("diameter", 100.0),
    ("opacity", 50.0),
    ("brightness", 30.0),
    ("growth", 20.0),
    ("fuzziness", 50.0),
    ("longest", 300.0),
    ("rayFuzziness", 100.0),
    ("rotation", 0.0),
    ("largest", 50.0),
];

/// The whole number of 0 or more `p` gives for `key`, or `default`.
fn count_or(p: &Value, key: &str, default: u64, cmd: &str) -> Result<u64> {
    let n = number_or(p, key, default as f64, cmd)?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
        Ok(n as u64)
    } else {
        Err(bad(cmd, format!("`{key}` must be a whole number of at least 0, not {n}")))
    }
}

/// `end`, the rings' end point, or an error naming `x2` or `y2` when that coordinate is not a
/// finite number (a `pathLength` so large that the end point overflows).
fn finite_end(end: Point, cmd: &str) -> Result<Point> {
    match [("x2", end.x), ("y2", end.y)].into_iter().find(|(_, v)| !v.is_finite()) {
        None => Ok(end),
        Some((key, _)) => Err(bad(cmd, format!("{key} must be a finite coordinate"))),
    }
}

/// `shape.flare`'s params `p` with every setting given: the defaults filled in, the counts at most
/// 50 and the rings' end point as `x2`, `y2` (from `pathLength` and `direction` when `p` gives no
/// end point), with `ringSeed` when `p` gives one.
fn settings_of(p: &Value, cmd: &str) -> Result<Value> {
    let c = Point::new(f64_req(p, "cx", cmd)?, f64_req(p, "cy", cmd)?);
    let mut numbers: Vec<&str> = SETTINGS.iter().map(|(key, _)| *key).collect();
    numbers.extend(["x2", "y2", "direction", "pathLength", "seed", "ringSeed"]);
    check_numbers(p, &numbers, cmd)?;
    let seed_of = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a whole number of at least 0, not {v}"))),
    };
    let seed = seed_of("seed")?.unwrap_or(DEFAULT_SEED);
    // Each `ringSeed` below 2^53 (what a JSON number holds exactly) places the rings anew.
    let ring_seed = match seed_of("ringSeed")? {
        Some(r) if r >= 1 << 53 => return Err(bad(cmd, format!("`ringSeed` must be below 2^53, not {r}"))),
        r => r,
    };
    let rays = count_or(p, "rays", 15, cmd)?.min(50);
    let rings = count_or(p, "rings", 10, cmd)?.min(50);
    let end = match (p.get("x2").and_then(Value::as_f64), p.get("y2").and_then(Value::as_f64)) {
        (Some(x), Some(y)) => Point::new(x, y),
        _ => {
            let a = f64_or(p, "direction", 45.0).to_radians();
            finite_end(c + Vec2::new(a.cos(), -a.sin()) * f64_or(p, "pathLength", 300.0), cmd)?
        }
    };
    let mut v = json!({ "cx": c.x, "cy": c.y, "x2": end.x, "y2": end.y, "rays": rays, "rings": rings, "seed": seed });
    if let Some(r) = ring_seed {
        v["ringSeed"] = json!(r);
    }
    for (key, default) in SETTINGS {
        v[key] = json!(f64_or(p, key, default));
    }
    Ok(v)
}

/// The objects of a flare drawn with the settings `v`, transformed by `xf`. A new flare's parts
/// (`old` None) get new ids and take the document's Align New Objects to Pixel Grid. A flare drawn
/// again from its parts `old` keeps, for each part, the id and pixel-aligned property of the old
/// part it replaces (the same kind of part, counted in order: the second ring replaces the second
/// ring); a part that replaces none gets a new id and the first old part's property.
fn part_nodes(d: &mut Document, v: &Value, xf: Affine, old: Option<&[Arc<Node>]>) -> Vec<Arc<Node>> {
    let mut seen: Vec<&str> = vec![];
    parts(v)
        .into_iter()
        .map(|part| {
            let name = part.0;
            let nth = seen.iter().filter(|n| **n == name).count();
            seen.push(name);
            let replaced = old.and_then(|old| old.iter().filter(|c| c.name.as_deref() == Some(name)).nth(nth));
            let id = replaced.map_or_else(|| d.alloc_id(), |c| c.id);
            let mut n = part_node(id, part, Affine::IDENTITY, false);
            match old {
                None => super::object::align_new_object(d, &mut n),
                Some(old) => n.extras_mut().pixel_aligned = replaced.or(old.first()).is_some_and(|c| c.extras().pixel_aligned),
            }
            if xf != Affine::IDENTITY {
                n.transform(xf, false);
            }
            Arc::new(n)
        })
        .collect()
}

/// `shape.flare`: a flare group that keeps its settings.
pub(super) fn flare(s: &mut Session, p: &Value) -> Result<Value> {
    let v = settings_of(p, "shape.flare")?;
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Flare", |d, sel| {
        let children = part_nodes(d, &v, Affine::IDENTITY, None);
        let gid = d.alloc_id();
        let mut g = Node::group(gid, children);
        g.name = Some("Flare".into());
        g.extras_mut().flare = Some(Box::new(FlareSpec::new(v)));
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The settings of flare `id`: a group drawn by the Flare tool whose parts are still the ones its
/// settings draw, in place ([`FlareSpec::draws`]). A flare whose parts were edited, distorted,
/// removed or added to, or whose art was flattened to a line, is no longer one.
pub fn flare_settings(d: &Document, id: NodeId) -> Option<FlareSpec> {
    let n = d.node(id)?;
    let spec = n.extras().flare.as_deref()?;
    // Settings a file holds are read as `shape.flare` reads them.
    let spec = FlareSpec { params: settings_of(&spec.params, "shape.flare").ok()?, transform: spec.transform };
    spec.draws(n).then_some(spec)
}

/// The flares `shape.editFlare` changes: those `p` names, else the selected flares. Each must be a
/// flare that can be edited.
fn flares_to_edit(s: &Session, p: &Value) -> Result<Vec<(NodeId, FlareSpec)>> {
    let given = given_objects(s, p, C)?;
    let d = &s.doc()?.doc;
    let flare = |id: NodeId| flare_settings(d, id).filter(|_| d.is_editable(id)).map(|spec| (id, spec));
    let flares: Vec<(NodeId, FlareSpec)> = match &given {
        Some(ids) => ids.iter().map(|id| flare(*id)).collect::<Option<Vec<_>>>().unwrap_or_default(),
        None => s.doc()?.selection.objects.iter().filter_map(|id| flare(*id)).collect(),
    };
    if flares.is_empty() {
        return Err(EngineError::Other("select a flare".into()));
    }
    Ok(flares)
}

/// The settings `old` changed as `p` asks (`shape.editFlare`'s params). The center and the end
/// point `p` gives are in the document, where the flare is now drawn, which `xf` maps from where
/// it was drawn.
fn edited(old: &Value, p: &Value, xf: Affine) -> Result<Value> {
    let mut v = old.clone();
    let inv = xf.inverse();
    let point = |kx: &str, ky: &str| -> Result<Option<Point>> {
        if p.get(kx).is_none_or(Value::is_null) && p.get(ky).is_none_or(Value::is_null) {
            return Ok(None);
        }
        // A point that leaves the finite range where the flare was drawn is refused, not stored.
        let q = inv * Point::new(f64_req(p, kx, C)?, f64_req(p, ky, C)?);
        match [(kx, q.x), (ky, q.y)].into_iter().find(|(_, v)| !v.is_finite()) {
            None => Ok(Some(q)),
            Some((key, _)) => Err(bad(C, format!("{key} must be a finite coordinate"))),
        }
    };
    let c = point("cx", "cy")?;
    let end = point("x2", "y2")?;
    if let Some(c) = c {
        v["cx"] = json!(c.x);
        v["cy"] = json!(c.y);
    }
    let centre = Point::new(f64_or(&v, "cx", 0.0), f64_or(&v, "cy", 0.0));
    let old_end = Point::new(f64_or(&v, "x2", centre.x), f64_or(&v, "y2", centre.y));
    let end = match end {
        Some(e) => e,
        None if p.get("pathLength").is_some_and(|l| !l.is_null()) || p.get("direction").is_some_and(|a| !a.is_null()) => {
            // The path length and the direction are where the flare is drawn now; the direction is
            // measured counter-clockwise from 3 o'clock, as `shape.flare` takes it.
            let (c, e) = (xf * centre, xf * old_end);
            let d = e - c;
            let a = number_or(p, "direction", (-d.y).atan2(d.x).to_degrees(), C)?.to_radians();
            finite_end(inv * (c + Vec2::new(a.cos(), -a.sin()) * number_or(p, "pathLength", d.hypot(), C)?), C)?
        }
        // Moving the center leaves the rings' end point where it is.
        None => old_end,
    };
    v["x2"] = json!(end.x);
    v["y2"] = json!(end.y);
    for (key, _) in SETTINGS {
        if p.get(key).is_some_and(|x| !x.is_null()) {
            v[key] = json!(number_or(p, key, 0.0, C)?);
        }
    }
    for key in ["rays", "rings"] {
        if p.get(key).is_some_and(|x| !x.is_null()) {
            v[key] = json!(count_or(p, key, 0, C)?.min(50));
        }
    }
    settings_of(&v, C)
}

/// `shape.editFlare`: draw the flares again with the settings `p` changes, each in its place and
/// transformed as it has been since it was drawn, as one undo step.
pub(super) fn edit_flare(s: &mut Session, p: &Value) -> Result<Value> {
    let mut numbers: Vec<&str> = SETTINGS.iter().map(|(key, _)| *key).collect();
    numbers.extend(["cx", "cy", "x2", "y2", "pathLength", "direction", "rays", "rings"]);
    check_numbers(p, &numbers, C)?;
    let flares = flares_to_edit(s, p)?;
    let mut changed: Vec<(NodeId, FlareSpec)> = vec![];
    for (id, spec) in &flares {
        let v = edited(&spec.params, p, spec.transform)?;
        if !json_close(&v, &spec.params, 1e-9) {
            changed.push((*id, FlareSpec { params: v, transform: spec.transform }));
        }
    }
    let ids: Vec<u64> = flares.iter().map(|(id, _)| id.0).collect();
    // Nothing changes: no undo step.
    if changed.is_empty() {
        return Ok(json!({ "ids": ids }));
    }
    let edited_ids: Vec<NodeId> = changed.iter().map(|(id, _)| *id).collect();
    s.edit("Edit Flare", |d, _| {
        for (id, spec) in changed {
            let old: Vec<Arc<Node>> = d.node(id).and_then(Node::children).cloned().unwrap_or_default();
            let children = part_nodes(d, &spec.params, spec.transform, Some(&old));
            let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
            if let Some(ch) = n.children_mut() {
                *ch = children;
            }
            n.extras_mut().flare = Some(Box::new(spec));
        }
        if !crate::sanity::objects_in_range(d, &edited_ids) {
            return Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()));
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids }))
}
