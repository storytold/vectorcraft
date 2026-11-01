//! Blends from the editing data (`(Adobe Path Blends) 1 0 n XP`), read as live blends.
//!
//! What a blend is and does is Illustrator's public documentation: key objects, the spine the
//! steps follow, Blend Options' Spacing (Smooth Color, Specified Steps, Specified Distance) and
//! Orientation (Align to Page, Align to Path), Replace Spine and Reverse Spine
//! (<https://helpx.adobe.com/illustrator/using/blending-objects.html>).
//!
//! How the editing data writes one isn't documented. As written in files users sent (used
//! locally, never committed): the blend's own art on `%_` lines, a group holding the key objects
//! and the spine, each object's `/ArtDictionary` naming its part (`ShapeBlendPathType`: `K` a key,
//! `S` the spine) and a key's id (`ShapeBlendPathID`); then the steps drawn, a group on plain
//! lines; then `(Adobe Path Blends) 1 0 n XP` and `n` bytes as hex on `%` lines. Those bytes
//! start `DCBA`, version 1; word 10 (little endian) is the number of keys, and the words after it
//! a record per key that starts with the key's id. What the other words mean isn't known, so the
//! spacing and orientation are taken from the steps drawn instead: as many steps as they have
//! between two keys, and whichever orientation lays the blend out as they are drawn. A blend whose
//! parts don't agree, or that doesn't come out as drawn, keeps its drawn steps.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::blend::blend_expand;
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
use vectorcraft_doc::{AppearanceItem, Node, NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point};

/// The plugin that writes blends.
pub(super) const PLUGIN: &str = "Adobe Path Blends"; // brand-ok: the plugin name the files use
/// The most bytes of a blend's record read.
pub(super) const MAX_RECORD: usize = 4096;
/// The most keys read from one blend.
const MAX_KEYS: usize = 64;
/// The most steps between two keys (Blend Options' limit).
const MAX_STEPS: usize = 1000;
/// The most objects (keys and steps) one blend may lay out to be read live; more keep their
/// drawn steps.
pub(super) const MAX_BLEND_OBJECTS: usize = 5_000;
/// How far (pt) a step laid out here may be from where the file draws it.
const PLACE_TOLERANCE: f64 = 0.5;
/// How far a step's colour (each channel, 0 to 1) may be from the drawn one.
const COLOR_TOLERANCE: f32 = 0.02;
/// The note for a blend that keeps its drawn steps.
pub(super) const DRAWN: &str = "a blend VectorCraft couldn't read as a live blend comes in as its drawn steps";

/// A blend part an object's dictionary names: its type (`K`, `S`) and its id.
pub(super) type Roles = HashMap<NodeId, (String, Option<String>)>;

/// The bytes of hex digits `hex`, or `None` when they aren't pairs of hex digits.
pub(super) fn hex_bytes(hex: &[u8]) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    hex.as_chunks::<2>().0.iter().map(|&[a, b]| Some(digit(a)? << 4 | digit(b)?)).collect()
}

/// The keys' ids in a blend's record, in its order; `None` for a record that isn't one.
pub(super) fn record_ids(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.get(..4)? != b"DCBA" || bytes.len() > MAX_RECORD {
        return None;
    }
    let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    if words.get(1) != Some(&1) {
        return None;
    }
    let count = usize::try_from(*words.get(10)?).ok().filter(|c| (2..=MAX_KEYS).contains(c))?;
    let rest = words.len().checked_sub(11)?;
    if rest == 0 || !rest.is_multiple_of(count) {
        return None;
    }
    let size = rest / count;
    (0..count)
        .map(|i| {
            let w = *words.get(11 + i * size)?;
            let c = char::from_u32(w).filter(char::is_ascii_alphanumeric)?;
            Some(c.to_string())
        })
        .collect()
}

/// The blend `edit` (its key objects and spine, as their dictionaries name them in `roles`) and
/// `drawn` (its steps as the file draws them) make, with the keys of the record `ids`: one live
/// blend, or `None` when they don't agree or it doesn't come out as drawn.
///
/// `budget`: how many more objects blends may lay out in this import; a blend that would lay out
/// more isn't made live (each blend is laid out twice, once per orientation, and checked against
/// its drawn steps, so a file of huge blends mustn't stall the import). What it lays out is taken
/// from it.
pub(super) fn live_blend(edit: &Node, drawn: &Node, ids: &[String], roles: &Roles, budget: &mut usize) -> Option<NodeKind> {
    let parts = edit.children()?;
    let role = |n: &Arc<Node>| roles.get(&n.id);
    let keys: Vec<Arc<Node>> = parts.iter().filter(|n| role(n).is_some_and(|(t, _)| t == "K")).cloned().collect();
    let spine = parts.iter().filter(|n| role(n).is_some_and(|(t, _)| t == "S")).find_map(|n| match &n.kind {
        NodeKind::Path { path, .. } => Some(path.clone()),
        _ => None,
    });
    // Each key named once, and the record's keys the same.
    let mut named: Vec<&str> = keys.iter().filter_map(|k| role(k).and_then(|(_, id)| id.as_deref())).collect();
    if keys.len() < 2 || named.len() != keys.len() || ids.len() != keys.len() {
        return None;
    }
    named.sort_unstable();
    let mut listed: Vec<&str> = ids.iter().map(String::as_str).collect();
    listed.sort_unstable();
    if named != listed || named.windows(2).any(|w| w.first() == w.get(1)) {
        return None;
    }
    // The steps between two keys: the drawn objects that aren't keys, shared out.
    let drawn: Vec<&Arc<Node>> = drawn.children()?.iter().collect();
    let between = (drawn.len().checked_sub(keys.len())?).checked_div(keys.len() - 1)?;
    if between == 0 || between > MAX_STEPS || drawn.len() != keys.len() + between * (keys.len() - 1) || drawn.len() > MAX_BLEND_OBJECTS {
        return None;
    }
    // Both orientations are laid out.
    let cost = drawn.len().saturating_mul(2);
    if cost > *budget {
        return None;
    }
    *budget -= cost;
    // The drawn steps' centres and paints, once.
    let drawn: Vec<(Point, Fill)> = drawn.iter().map(|d| Some((centre(d)?, fill(d)))).collect::<Option<_>>()?;
    let keys: Vec<Arc<Node>> = keys
        .into_iter()
        .map(|k| {
            let mut k = (*k).clone();
            k.visible = true;
            Arc::new(k)
        })
        .collect();
    let spine = spine?;
    let key_anchors = on_spine(&keys, &spine)?;
    let spacing = BlendSpacing::Steps(u32::try_from(between).ok()?);
    // The orientation that lays the steps out as drawn, the closest when both do.
    [BlendOrientation::AlignToPage, BlendOrientation::AlignToPath]
        .into_iter()
        .filter_map(|orientation| {
            let spec = BlendSpec { spacing, orientation, spine: Some(spine.clone()), key_anchors: key_anchors.clone(), ..BlendSpec::default() };
            let laid = blend_expand(&keys, &spec);
            let off = how_far(&laid, &drawn)?;
            Some((off, spec))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, spec)| NodeKind::Blend { children: keys, spec: Box::new(spec) })
}

/// The anchor of `spine`'s first subpath each key's centre sits on (within a point), each a
/// different one.
fn on_spine(keys: &[Arc<Node>], spine: &PathData) -> Option<Vec<u32>> {
    let anchors = &spine.subpaths.first()?.anchors;
    let mut out: Vec<u32> = vec![];
    for k in keys {
        let c = centre(k)?;
        let (i, d) = anchors.iter().enumerate().map(|(i, a)| (i, (a.p - c).hypot())).min_by(|a, b| a.1.total_cmp(&b.1))?;
        let i = u32::try_from(i).ok()?;
        if d > 1.0 || out.contains(&i) {
            return None;
        }
        out.push(i);
    }
    Some(out)
}

fn centre(n: &Node) -> Option<Point> {
    n.geometric_bounds().map(|b| b.center()).filter(|p| p.x.is_finite() && p.y.is_finite())
}

/// An object's fill, as far as the check compares it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fill {
    /// No fill.
    None,
    /// A solid colour, as RGB.
    Solid([f32; 3]),
    /// A fill this can't compare (a gradient, a pattern): the blend isn't made live.
    Other,
}

/// An object's first fill.
fn fill(n: &Node) -> Fill {
    n.appearance
        .items
        .iter()
        .find_map(|i| match i {
            AppearanceItem::Fill(f) => Some(match &f.paint {
                Paint::None => Fill::None,
                Paint::Solid { color, .. } => Fill::Solid(Color::to_rgb(color)),
                _ => Fill::Other,
            }),
            _ => None,
        })
        .unwrap_or(Fill::None)
}

/// Do two fills match: both none, or solid colours within the tolerance?
fn same_fill(a: Fill, b: Fill) -> bool {
    match (a, b) {
        (Fill::None, Fill::None) => true,
        (Fill::Solid(a), Fill::Solid(b)) => a.iter().zip(b).all(|(x, y)| (x - y).abs() <= COLOR_TOLERANCE),
        _ => false,
    }
}

/// How far the objects `laid` are from the drawn ones (`drawn`: their centres and fills): the
/// farthest, each drawn object matched to a laid one of its own within the tolerance (each laid
/// one matches one drawn one: the nearest still free; through a grid of cells the tolerance wide,
/// so each finds its neighbours without looking at them all), when they are as many and each
/// matches in place and fill; else `None` (also when that matching leaves one out).
fn how_far(laid: &[Node], drawn: &[(Point, Fill)]) -> Option<f64> {
    if laid.len() != drawn.len() {
        return None;
    }
    let cell = |p: Point| ((p.x / PLACE_TOLERANCE).floor() as i64, (p.y / PLACE_TOLERANCE).floor() as i64);
    let laid: Vec<(Point, Fill)> = laid.iter().map(|l| Some((centre(l)?, fill(l)))).collect::<Option<_>>()?;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::with_capacity(laid.len());
    for (i, (p, _)) in laid.iter().enumerate() {
        grid.entry(cell(*p)).or_default().push(i);
    }
    let mut used = vec![false; laid.len()];
    let mut worst: f64 = 0.0;
    for &(c, f) in drawn {
        let (cx, cy) = cell(c);
        // Coordinates far out saturate the cells: the neighbours' keys saturate too.
        let (i, near) = (-1i64..=1)
            .flat_map(|dx| (-1i64..=1).map(move |dy| (cx.saturating_add(dx), cy.saturating_add(dy))))
            .filter_map(|k| grid.get(&k))
            .flatten()
            .filter(|&&i| !used.get(i).copied().unwrap_or(true))
            .filter_map(|&i| laid.get(i).map(|l| (i, l)))
            .filter(|(_, (p, lf))| (*p - c).hypot() <= PLACE_TOLERANCE && same_fill(*lf, f))
            .map(|(i, (p, _))| (i, (*p - c).hypot()))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        if let Some(u) = used.get_mut(i) {
            *u = true;
        }
        worst = worst.max(near);
    }
    Some(worst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records that aren't one: too short, another magic, another version, key counts out of
    /// range or not dividing the rest, ids that aren't letters or digits, and hex digits that
    /// don't pair up. None of them panics.
    #[test]
    fn records_are_checked() {
        let words = |w: &[u32]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let magic = u32::from_le_bytes(*b"DCBA");
        let good = words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 'A' as u32, 0, 0, 0, 0, 0, 'B' as u32, 0, 0, 0, 0, 0]);
        assert_eq!(record_ids(&good), Some(vec!["A".to_string(), "B".to_string()]));
        for bad in [
            vec![],
            b"DCB".to_vec(),
            words(&[0x41424344, 1]),
            words(&[magic, 2, 0, 0, 0, 0, 0, 0, 0, 0, 2, 65, 0, 66, 0]),
            words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 65]),
            words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1000, 65]),
            words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 65, 0, 66]),
            words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0x20, 0, 66, 0]),
            words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, u32::MAX]),
        ] {
            assert_eq!(record_ids(&bad), None, "{bad:?}");
        }
        assert_eq!(hex_bytes(b"4A4"), None);
        assert_eq!(hex_bytes(b"4G"), None);
        assert_eq!(hex_bytes(b"4a4B"), Some(vec![0x4a, 0x4b]));
    }

    /// A blend that would lay out more than the import's budget has left isn't made live, and
    /// takes nothing from it.
    #[test]
    fn a_blend_past_the_budget_isnt_made_live() {
        let edit = Node::new(NodeId(1), NodeKind::Group { children: vec![], clip: false });
        let drawn = Node::new(NodeId(2), NodeKind::Group { children: vec![], clip: false });
        let mut budget = 1;
        assert!(live_blend(&edit, &drawn, &["A".into(), "B".into()], &Roles::new(), &mut budget).is_none());
        assert_eq!(budget, 1);
    }

    /// Two drawn steps on one laid object don't both match it: each laid object matches one drawn
    /// one, so a drawing the blend doesn't lay out isn't taken for it.
    #[test]
    fn each_laid_step_matches_one_drawn_step() {
        let square = |x: f64| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 0.0, x + 10.0, 10.0));
            Node::path(NodeId(0), path, Default::default())
        };
        let laid = [square(0.0), square(50.0)];
        let c = |x: f64| (Point::new(x + 5.0, 5.0), Fill::None);
        assert!(how_far(&laid, &[c(0.0), c(50.0)]).is_some());
        assert!(how_far(&laid, &[c(0.0), c(0.1)]).is_none(), "both drawn steps near the first laid one");
        // Coordinates far out don't overflow the grid.
        let far = [square(1e300), square(-1e300)];
        let _ = how_far(&far, &[c(1e300), c(-1e300)]);
    }
}
