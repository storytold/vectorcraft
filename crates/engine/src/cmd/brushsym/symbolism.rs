//! What the symbolism commands share: the Method and the Symbol Sprayer options of the Symbolism
//! Tools Options, what an instance shows of the values the tools change ([`Look`]), the mean of
//! those values under the brush ([`mean`]), and `symbol.setDensity`.

use std::collections::HashMap;

use super::symbols::{instance_center, is_symbol_set};
use super::{Affine, Document, EngineError, Node, NodeId, NodeKind, Paint, Point, Result, Session, Value, Vec2, bad, ids_json, json, named_ids};

/// Method: how the Scruncher, Sizer, Spinner, Stainer, Screener and Styler adjust instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    /// Gradually, in relation to the position of the pointer.
    UserDefined,
    /// Toward the mean of the instances under the brush.
    Average,
    /// By an amount that differs from instance to instance.
    Random,
}

/// The `method` of `symbol.adjust` (User Defined when left out or null).
pub(super) fn method_param(p: &Value, cmd: &str) -> Result<Method> {
    match p.get("method") {
        None | Some(Value::Null) => Ok(Method::UserDefined),
        Some(Value::String(s)) if s == "userDefined" => Ok(Method::UserDefined),
        Some(Value::String(s)) if s == "average" => Ok(Method::Average),
        Some(Value::String(s)) if s == "random" => Ok(Method::Random),
        Some(v) => Err(bad(cmd, format!("`method` must be userDefined, average or random, not {v}"))),
    }
}

/// The Symbol Sprayer options: whether each value of a new instance is the average of the
/// instances under the brush (true) or the User Defined value (false).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SprayModes {
    pub scrunch: bool,
    pub size: bool,
    pub spin: bool,
    pub screen: bool,
    pub stain: bool,
    pub style: bool,
}

impl SprayModes {
    /// Whether any value is an average.
    pub fn any_average(self) -> bool {
        self.scrunch || self.size || self.spin || self.screen || self.stain || self.style
    }
}

/// The `attributes` of `symbol.spray`: `{scrunch?, size?, spin?, screen?, stain?, style?}`, each
/// `average` (also when left out or null) or `userDefined`. Another key or value is an error.
pub(super) fn spray_modes(p: &Value, cmd: &str) -> Result<SprayModes> {
    let mut m = SprayModes { scrunch: true, size: true, spin: true, screen: true, stain: true, style: true };
    let map = match p.get("attributes") {
        None | Some(Value::Null) => return Ok(m),
        Some(Value::Object(map)) => map,
        Some(v) => return Err(bad(cmd, format!("`attributes` must be an object, not {v}"))),
    };
    for (key, v) in map {
        let slot = match key.as_str() {
            "scrunch" => &mut m.scrunch,
            "size" => &mut m.size,
            "spin" => &mut m.spin,
            "screen" => &mut m.screen,
            "stain" => &mut m.stain,
            "style" => &mut m.style,
            _ => return Err(bad(cmd, format!("unknown attribute `{key}`"))),
        };
        *slot = match v {
            Value::Null => true,
            Value::String(s) if s == "average" => true,
            Value::String(s) if s == "userDefined" => false,
            v => return Err(bad(cmd, format!("attribute `{key}` must be average or userDefined, not {v}"))),
        };
    }
    Ok(m)
}

/// How far apart the Symbol Sprayer keeps instances at Symbol Set Density `density` (1..10), as a
/// share of the symbol's size: a denser set is packed closer.
pub(super) fn spacing(density: f64) -> f64 {
    (1.15 - density * 0.1).max(0.1)
}

/// The document key that holds each symbol set's Symbol Set Density: `{set id: density}`, written
/// when the Symbol Sprayer creates a set and when `symbol.setDensity` spaces it.
const SET_DENSITY_KEY: &str = "symbolSetDensity";

/// The Symbol Set Density symbol set `set` was created at or last given (None when not recorded).
fn set_density_of(d: &Document, set: NodeId) -> Option<f64> {
    d.unknown.get(SET_DENSITY_KEY)?.get(set.0.to_string())?.as_f64().filter(|v| (1.0..=10.0).contains(v))
}

/// Record `density` as the Symbol Set Density of `sets`, and drop the records of ids that are no
/// longer symbol sets.
pub(super) fn record_set_density(d: &mut Document, sets: &[NodeId], density: f64) {
    let mut map = match d.unknown.get(SET_DENSITY_KEY) {
        Some(Value::Object(m)) => m.clone(),
        _ => serde_json::Map::new(),
    };
    for set in sets {
        map.insert(set.0.to_string(), json!(density));
    }
    map.retain(|k, _| k.parse::<u64>().ok().and_then(|id| d.node(NodeId(id))).is_some_and(is_symbol_set));
    if map.is_empty() {
        d.unknown.remove(SET_DENSITY_KEY);
    } else {
        d.unknown.insert(SET_DENSITY_KEY.into(), Value::Object(map));
    }
}

/// What a symbol instance shows of the values the symbolism tools change.
#[derive(Clone, Debug)]
pub(super) struct Look {
    pub center: Point,
    /// Its size as a share of its symbol's own size (1: the symbol's size).
    pub scale: f64,
    /// Its rotation in radians.
    pub angle: f64,
    pub opacity: f64,
    /// The fill the Symbol Stainer or the Symbol Styler gave it, with its amount (0..1).
    pub fill: Option<(Paint, f64)>,
}

/// The [`Look`] of instance `n` (None for another object, or an instance without a size).
pub(super) fn look(d: &Document, n: &Node) -> Option<Look> {
    let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return None };
    let [a, b, c, e, ..] = xf.as_coeffs();
    let natural = d.symbol_natural_xf(symbol).as_coeffs();
    let (sx, sy) = (natural[0].abs().max(1e-9), natural[3].abs().max(1e-9));
    let scale = ((a * e - b * c) / (sx * sy)).abs().sqrt();
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    Some(Look {
        center: instance_center(n)?,
        scale,
        angle: b.atan2(a),
        opacity: f64::from(n.opacity),
        fill: n.appearance.fill().map(|f| (f.paint.clone(), f64::from(f.opacity))),
    })
}

/// The mean of the [`Look`]s of the instances under the brush.
#[derive(Clone, Debug)]
pub(super) struct Mean {
    /// The geometric mean of the sizes.
    pub scale: f64,
    /// The mean direction of the rotations.
    pub angle: f64,
    pub opacity: f64,
    /// The mean stain or style amount; an instance without one counts as 0.
    pub amount: f64,
    /// The stain or style of the instance nearest the brush's center that has one.
    pub paint: Option<Paint>,
    /// The middle of the instances.
    pub middle: Point,
    /// Their mean distance from [`Mean::middle`].
    pub reach: f64,
    /// The mean distance from each of them to the nearest of the others (None for one instance, or
    /// when not asked for).
    pub gap: Option<f64>,
}

/// For each of `looks`, the distance to the nearest of the others (infinite for one alone). Each
/// is first looked for in the cells around it of a grid `cell` points wide: one found within `cell`
/// is the nearest, since every one that near is in those cells. Otherwise all are measured.
fn nearest_gaps(looks: &[&Look], cell: f64) -> Vec<f64> {
    let cell = cell.max(1e-6);
    let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, l) in looks.iter().enumerate() {
        cells.entry(key(l.center, cell)).or_default().push(i);
    }
    let distance = |i: usize, j: usize| if i == j { None } else { Some(looks.get(j)?.center.distance(looks.get(i)?.center)) };
    (0..looks.len())
        .map(|i| {
            let mut best = f64::INFINITY;
            for j in around(&cells, looks.get(i).map_or(Point::ZERO, |l| l.center), cell) {
                best = distance(i, j).map_or(best, |d| best.min(d));
                // Instances on top of each other: none can be nearer.
                if best == 0.0 {
                    break;
                }
            }
            if best <= cell { best } else { (0..looks.len()).filter_map(|j| distance(i, j)).fold(f64::INFINITY, f64::min) }
        })
        .collect()
}

/// The [`Mean`] of `looks`, the instances under a brush centered on `at` (None for none), with
/// [`Mean::gap`] when `gap_cell` gives the grid width [`nearest_gaps`] uses.
pub(super) fn mean(looks: &[&Look], at: Point, gap_cell: Option<f64>) -> Option<Mean> {
    if looks.is_empty() {
        return None;
    }
    let n = looks.len() as f64;
    let (mut log_scale, mut sin, mut cos, mut opacity, mut amount, mut middle) = (0.0, 0.0, 0.0, 0.0, 0.0, Vec2::ZERO);
    let mut paint: Option<(&Paint, f64)> = None;
    for l in looks {
        log_scale += l.scale.ln();
        sin += l.angle.sin();
        cos += l.angle.cos();
        opacity += l.opacity;
        middle += l.center.to_vec2();
        if let Some((p, o)) = &l.fill {
            amount += o;
            let far = l.center.distance(at);
            if paint.is_none_or(|(_, nearest)| far < nearest) {
                paint = Some((p, far));
            }
        }
    }
    let middle = (middle / n).to_point();
    // Rotations that cancel out have no mean direction: the first instance's stands for it.
    let angle = if sin.hypot(cos) > 1e-9 { sin.atan2(cos) } else { looks[0].angle };
    // One instance under the brush has no spacing there.
    let gap = gap_cell.filter(|_| looks.len() > 1).map(|cell| nearest_gaps(looks, cell).iter().sum::<f64>() / n);
    Some(Mean {
        scale: (log_scale / n).exp(),
        angle,
        opacity: opacity / n,
        amount: amount / n,
        paint: paint.map(|(p, _)| p.clone()),
        middle,
        reach: looks.iter().map(|l| l.center.distance(middle)).sum::<f64>() / n,
        gap,
    })
}

/// `angle` (radians) brought into −π..π.
pub(super) fn wrap_angle(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    let a = angle.rem_euclid(turn);
    if a > std::f64::consts::PI { a - turn } else { a }
}

/// The instances of a symbol set by where they are, to find those under a brush without looking at
/// every one for every point of a stroke.
pub(super) struct Grid {
    cell: f64,
    looks: Vec<Look>,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

fn key(p: Point, cell: f64) -> (i64, i64) {
    ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64)
}

/// The indices in the cell of `at`, then in the eight around it. Saturating: a point far out on the
/// canvas has a key at the end of the range.
fn around(cells: &HashMap<(i64, i64), Vec<usize>>, at: Point, cell: f64) -> impl Iterator<Item = usize> + '_ {
    let (cx, cy) = key(at, cell);
    let ring = (cx.saturating_sub(1)..=cx.saturating_add(1))
        .flat_map(move |x| (cy.saturating_sub(1)..=cy.saturating_add(1)).map(move |y| (x, y)))
        .filter(move |c| *c != (cx, cy));
    std::iter::once((cx, cy)).chain(ring).flat_map(move |c| cells.get(&c).into_iter().flatten().copied())
}

impl Grid {
    /// `looks` in cells `cell` points wide (the brush's radius).
    pub fn new(looks: Vec<Look>, cell: f64) -> Self {
        let cell = cell.max(1.0);
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, l) in looks.iter().enumerate() {
            cells.entry(key(l.center, cell)).or_default().push(i);
        }
        Self { cell, looks, cells }
    }

    /// The mean of the instances within `radius` (at most the cell width) of `at`, with
    /// [`Mean::gap`] when `with_gap` is set.
    pub fn mean(&self, at: Point, radius: f64, with_gap: bool) -> Option<Mean> {
        let near: Vec<&Look> =
            around(&self.cells, at, self.cell).filter_map(|i| self.looks.get(i)).filter(|l| l.center.distance(at) <= radius).collect();
        mean(&near, at, with_gap.then_some(radius / 4.0))
    }
}

/// `symbol.setDensity {from, to, ids?|id?}`: space the instances of the selected symbol sets as
/// Symbol Set Density `to` packs them, from the density each set was created at or last given
/// (`from` for a set without one): each instance moves along the line from the middle of its set's
/// instances, to [`spacing`]`(to) / `[`spacing`]` of the set's density` of its distance from it.
/// Every selected set then has density `to`, also one whose instances do not move. One undo step,
/// and none when no instance moves and every set already has density `to`.
pub(super) fn symbol_set_density(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.setDensity";
    let density = |key: &str| match p.get(key) {
        Some(v) => v.as_f64().filter(|d| (1.0..=10.0).contains(d)).ok_or_else(|| bad(C, format!("`{key}` must be a number from 1 to 10, not {v}"))),
        None => Err(bad(C, format!("missing `{key}`"))),
    };
    let (from, to) = (density("from")?, density("to")?);
    crate::cmd::edit::refuse_symbol_part(s)?;
    let st = s.doc()?;
    let d = &st.doc;
    let named = named_ids(p)?;
    let ids: Vec<NodeId> = named.clone().unwrap_or_else(|| st.selection.objects.clone());
    if let Some(missing) = named.iter().flatten().find(|id| d.node(**id).is_none()) {
        return Err(EngineError::Other(format!("no object {}", missing.0)));
    }
    let editable = d.editable();
    let mut sets: Vec<NodeId> = vec![];
    for id in ids {
        if d.node(id).is_some_and(is_symbol_set) && editable(id) && !sets.contains(&id) {
            sets.push(id);
        }
    }
    if sets.is_empty() {
        return Err(EngineError::Other("no symbol set selected".into()));
    }
    let mut moves: Vec<(NodeId, Vec2)> = vec![];
    let mut changed: Vec<NodeId> = vec![];
    for set in &sets {
        // A set packs from the density it was created at or last given, else from `from`.
        let own = set_density_of(d, *set);
        let ratio = spacing(to) / spacing(own.unwrap_or(from));
        // A set takes density `to` even when none of its instances moves (one instance, or each
        // locked or on top of the others), so a later change packs it from `to`.
        let restamp = own != Some(to);
        let centers: Vec<(NodeId, Point)> =
            d.node(*set).and_then(|n| n.children()).into_iter().flatten().filter_map(|c| Some((c.id, instance_center(c)?))).collect();
        if centers.len() < 2 {
            if restamp {
                changed.push(*set);
            }
            continue;
        }
        let middle = (centers.iter().fold(Vec2::ZERO, |sum, (_, c)| sum + c.to_vec2()) / centers.len() as f64).to_point();
        let before = moves.len();
        // A locked or hidden instance stays where it is.
        moves.extend(
            centers.iter().filter(|(id, _)| editable(*id)).map(|(id, c)| (*id, (*c - middle) * (ratio - 1.0))).filter(|(_, v)| v.hypot() > 1e-9),
        );
        if moves.len() > before || restamp {
            changed.push(*set);
        }
    }
    drop(editable);
    if !changed.is_empty() {
        s.edit("Symbol Set Density", |d, _| {
            for (id, v) in &moves {
                if let Some(n) = d.node_mut(*id) {
                    n.transform(Affine::translate(*v), false);
                }
            }
            record_set_density(d, &changed, to);
            Ok(())
        })?;
    }
    Ok(json!({ "ids": ids_json(&changed), "count": moves.len() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Look {
        Look { center: Point::new(x, y), scale: 1.0, angle: 0.0, opacity: 1.0, fill: None }
    }

    /// The gaps are each instance's distance to its nearest neighbor, measured against every other
    /// instance: close pairs whose members are far apart in the order, instances on top of each
    /// other, and instances farther apart than the grid's cells.
    #[test]
    fn nearest_gaps_match_measuring_every_pair() {
        let mut looks: Vec<Look> = (0..300).map(|i| at(f64::from(i % 20) * 9.0, f64::from(i / 20) * 7.0)).collect();
        // The second member of each pair comes 300 places later.
        looks.extend((0..300).map(|i| at(f64::from(i % 20) * 9.0 + 0.5, f64::from(i / 20) * 7.0)));
        looks.extend([at(500.0, 500.0), at(500.0, 500.0), at(900.0, 40.0)]);
        let refs: Vec<&Look> = looks.iter().collect();
        let brute: Vec<f64> = (0..refs.len())
            .map(|i| (0..refs.len()).filter(|j| *j != i).map(|j| refs[i].center.distance(refs[j].center)).fold(f64::INFINITY, f64::min))
            .collect();
        assert_eq!(nearest_gaps(&refs, 10.0), brute);
        assert_eq!(nearest_gaps(&refs[..1], 10.0), [f64::INFINITY]);
    }
}
