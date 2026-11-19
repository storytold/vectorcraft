//! What the symbolism commands share: the Method and the Symbol Sprayer options of the Symbolism
//! Tools Options, what an instance shows of the values the tools change ([`Look`]), the mean of
//! those values under the brush ([`mean`]), and `symbol.setDensity`.

use std::collections::HashMap;

use super::symbols::{instance_center, is_symbol_set, stored_natural_size};
use super::{
    Affine, Document, EngineError, HALF, Node, NodeId, NodeKind, Paint, Point, Result, Session, Value, Vec2, bad, ids_json, json, named_ids,
};

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
/// when the Symbol Sprayer creates a set and when `symbol.setDensity` spaces it, and copied to a
/// copy of a set by [`Document::reid`].
const SET_DENSITY_KEY: &str = Document::SYMBOL_SET_DENSITY_KEY;

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
    /// Its size on the page: the side of a square with its area, the same measure for every symbol.
    pub size: f64,
    /// Its rotation in radians.
    pub angle: f64,
    pub opacity: f64,
    /// The fill the Symbol Stainer or the Symbol Styler gave it, with its amount (0..1).
    pub fill: Option<(Paint, f64)>,
}

/// The [`natural_side`] of each symbol measured so far in one command, so that one measure serves
/// every instance of the symbol.
pub(super) type Sides = HashMap<String, f64>;

/// The [`Look`] of instance `n` (None for another object, or an instance without a size).
pub(super) fn look(d: &Document, n: &Node, sides: &mut Sides) -> Option<Look> {
    let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return None };
    let [a, b, c, e, ..] = xf.as_coeffs();
    let natural = d.symbol_natural_xf(symbol).as_coeffs();
    let (sx, sy) = (natural[0].abs().max(1e-9), natural[3].abs().max(1e-9));
    let scale = ((a * e - b * c) / (sx * sy)).abs().sqrt();
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    let side = match sides.get(symbol) {
        Some(side) => *side,
        None => *sides.entry(symbol.clone()).or_insert_with(|| natural_side(d, symbol)),
    };
    let size = scale * side;
    if !(size.is_finite() && size > 0.0) {
        return None;
    }
    Some(Look {
        center: instance_center(n)?,
        size,
        angle: b.atan2(a),
        opacity: f64::from(n.opacity),
        fill: n.appearance.fill().map(|f| (f.paint.clone(), f64::from(f.opacity))),
    })
}

/// The [`Look::size`] of an instance of symbol `name` at the symbol's own size. A symbol without a
/// recorded size draws its art at the art's own size, so its art is measured.
pub(super) fn natural_side(d: &Document, name: &str) -> f64 {
    let recorded = stored_natural_size(d, name).is_some_and(|(w, h)| w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0);
    // Each side's root first, so that a huge size does not overflow.
    let art = (!recorded).then(|| d.symbol_art_bounds(name)).flatten().map(|r| r.width().sqrt() * r.height().sqrt());
    if let Some(side) = art.filter(|s| s.is_finite() && *s > 0.0) {
        return side;
    }
    let natural = d.symbol_natural_xf(name).as_coeffs();
    2.0 * HALF * natural[0].abs().max(1e-9).sqrt() * natural[3].abs().max(1e-9).sqrt()
}

/// The mean of the [`Look`]s of the instances under the brush.
#[derive(Clone, Debug)]
pub(super) struct Mean {
    /// The geometric mean of the sizes ([`Look::size`]).
    pub size: f64,
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

/// The x (`axis` 0) or y (`axis` 1) of `p`.
fn coord(p: Point, axis: usize) -> f64 {
    if axis == 0 { p.x } else { p.y }
}

/// Arrange `order` (indices into `pts`) as a k-d tree: the middle entry of each slice splits the
/// rest by x at even depths and by y at odd ones, the smaller values before it.
fn kd_build(order: &mut [usize], pts: &[Point], axis: usize) {
    if order.len() <= 1 {
        return;
    }
    let mid = order.len() / 2;
    let at = |i: &usize| pts.get(*i).map_or(0.0, |p| coord(*p, axis));
    order.select_nth_unstable_by(mid, |a, b| at(a).total_cmp(&at(b)));
    let (left, right) = order.split_at_mut(mid);
    kd_build(left, pts, 1 - axis);
    kd_build(right.get_mut(1..).unwrap_or_default(), pts, 1 - axis);
}

/// The distance from `pts[i]` to the nearest of the others in k-d tree `order`, if nearer than
/// `best`. Instances on top of each other end the search: none can be nearer.
fn kd_nearest(order: &[usize], pts: &[Point], axis: usize, i: usize, best: &mut f64) {
    let (Some(&j), Some(&p)) = (order.get(order.len() / 2), pts.get(i)) else { return };
    let Some(&q) = pts.get(j) else { return };
    if j != i {
        *best = best.min(p.distance(q));
    }
    if *best == 0.0 {
        return;
    }
    let (left, right) = order.split_at(order.len() / 2);
    let right = right.get(1..).unwrap_or_default();
    let diff = coord(p, axis) - coord(q, axis);
    let (near, far) = if diff < 0.0 { (left, right) } else { (right, left) };
    kd_nearest(near, pts, 1 - axis, i, best);
    // The other side holds nothing nearer than the split line.
    if diff.abs() < *best {
        kd_nearest(far, pts, 1 - axis, i, best);
    }
}

/// For each of `looks`, the distance to the nearest of the others (infinite for one alone), found
/// through a k-d tree of their centers.
fn nearest_gaps(looks: &[&Look]) -> Vec<f64> {
    let pts: Vec<Point> = looks.iter().map(|l| l.center).collect();
    let mut order: Vec<usize> = (0..pts.len()).collect();
    kd_build(&mut order, &pts, 0);
    (0..pts.len())
        .map(|i| {
            let mut best = f64::INFINITY;
            kd_nearest(&order, &pts, 0, i, &mut best);
            best
        })
        .collect()
}

/// The [`Mean`] of `looks`, the instances under a brush centered on `at` (None for none), with
/// [`Mean::gap`] when `with_gap` is set.
pub(super) fn mean(looks: &[&Look], at: Point, with_gap: bool) -> Option<Mean> {
    if looks.is_empty() {
        return None;
    }
    let n = looks.len() as f64;
    let (mut log_size, mut sin, mut cos, mut opacity, mut amount, mut middle) = (0.0, 0.0, 0.0, 0.0, 0.0, Vec2::ZERO);
    let mut paint: Option<(&Paint, f64)> = None;
    for l in looks {
        log_size += l.size.ln();
        sin += l.angle.sin();
        cos += l.angle.cos();
        opacity += l.opacity;
        middle += l.center.to_vec2();
        // A fill at no opacity shows nothing: it is no stain or style, and its paint is not taken.
        if let Some((p, o)) = &l.fill
            && *o > 0.0
        {
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
    let gap = (with_gap && looks.len() > 1).then(|| nearest_gaps(looks).iter().sum::<f64>() / n);
    Some(Mean {
        size: (log_size / n).exp(),
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
        mean(&near, at, with_gap)
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
        Look { center: Point::new(x, y), size: 20.0, angle: 0.0, opacity: 1.0, fill: None }
    }

    /// The distance from each instance in `from` to the nearest other one in `refs`, measured
    /// against every other instance.
    fn brute(refs: &[&Look], from: impl Iterator<Item = usize>) -> Vec<f64> {
        from.map(|i| (0..refs.len()).filter(|j| *j != i).map(|j| refs[i].center.distance(refs[j].center)).fold(f64::INFINITY, f64::min)).collect()
    }

    /// The gaps are each instance's distance to its nearest neighbor, measured against every other
    /// instance: close pairs whose members are far apart in the order, instances on top of each
    /// other, instances far from all others, many instances close together, and many on one line.
    #[test]
    fn nearest_gaps_match_measuring_every_pair() {
        let mut looks: Vec<Look> = (0..300).map(|i| at(f64::from(i % 20) * 9.0, f64::from(i / 20) * 7.0)).collect();
        // The second member of each pair comes 300 places later.
        looks.extend((0..300).map(|i| at(f64::from(i % 20) * 9.0 + 0.5, f64::from(i / 20) * 7.0)));
        looks.extend([at(500.0, 500.0), at(500.0, 500.0), at(900.0, 40.0)]);
        // 400 instances within one point of each other, and 200 on one upright line.
        looks.extend((0..400).map(|i| at(700.0 + f64::from(i % 20) * 0.05, 700.0 + f64::from(i / 20) * 0.05)));
        looks.extend((0..200).map(|i| at(-50.0, f64::from(i) * 1.5)));
        let refs: Vec<&Look> = looks.iter().collect();
        assert_eq!(nearest_gaps(&refs), brute(&refs, 0..refs.len()));
        assert_eq!(nearest_gaps(&refs[..1]), [f64::INFINITY]);
        assert_eq!(nearest_gaps(&refs[..0]), Vec::<f64>::new());
    }
}
