//! Blends from the editing data (`(Adobe Path Blends) 1 0 n XP`), read as live blends.
//!
//! What a blend is and does is Illustrator's public documentation: key objects, the spine the
//! steps follow, Blend Options' Spacing (Smooth Color, Specified Steps, Specified Distance) and
//! Orientation (Align to Page, Align to Path), Replace Spine and Reverse Spine
//! (<https://helpx.adobe.com/illustrator/desktop/manage-colors/apply-transparency-and-blending/blend-options-overview.html>,
//! <https://web.archive.org/web/20241217212108/https://helpx.adobe.com/illustrator/using/blending-objects.html>).
//!
//! How the editing data writes one isn't documented. As written in files users sent (used
//! locally, never committed): the blend's own art on `%_` lines, a group holding the key objects
//! and the spine, each object's `/ArtDictionary` naming its part (`ShapeBlendPathType`: `K` a key,
//! `S` the spine) and a key's id (`ShapeBlendPathID`); then the steps drawn, a group on plain
//! lines; then `(Adobe Path Blends) 1 0 n XP` and `n` bytes as hex on `%` lines (`%_%` lines
//! for a blend in hidden art, as an opacity mask's is: all of it is on `%_` lines). Those bytes
//! start `DCBA`. A blend whose spine was never edited has no spine part: only its keys.
//!
//! The reader reads the words after `DCBA` (little endian) this way. Word 1 is the spacing: 0 is
//! Smooth Color, 1 is Specified Steps and 2 is Specified Distance. For Specified Distance, word 2
//! is the distance in points, as a 32-bit float. Word 10 is the number of keys, and the words after
//! it are a record of six words per key that starts with the key's id. For Specified Steps, the
//! steps between two keys are counted from the steps drawn. The orientation is taken from the steps
//! drawn: whichever one lays the blend out as they are drawn. A blend whose parts don't agree,
//! whose spacing isn't one of these, or that doesn't come out as drawn, keeps its drawn steps.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::blend::{blend_expand, blend_steps};
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
use vectorcraft_doc::{AppearanceItem, Node, NodeId, NodeKind};
use vectorcraft_geom::{PathData, Point, SubPath};

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

/// What a blend's record says: its keys' ids, in its order, and its spacing.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Record {
    pub(super) ids: Vec<String>,
    /// `None`: Specified Steps, whose steps the reader counts from the drawn ones.
    pub(super) spacing: Option<BlendSpacing>,
}

/// A blend's record, or `None` for a record that isn't one or whose spacing isn't read: an
/// unknown spacing, or a distance that isn't a positive number.
pub(super) fn record(bytes: &[u8]) -> Option<Record> {
    if bytes.get(..4)? != b"DCBA" || bytes.len() > MAX_RECORD {
        return None;
    }
    let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    let spacing = match *words.get(1)? {
        0 => Some(BlendSpacing::SmoothColor),
        1 => None,
        2 => Some(BlendSpacing::Distance(distance(*words.get(2)?)?)),
        _ => return None,
    };
    let ids = record_ids(&words)?;
    Some(Record { ids, spacing })
}

/// The distance in a record's word `bits` (a 32-bit float), as the shortest decimal that reads
/// back as that float (2.3 for the float nearest 2.3), or `None` when it isn't a positive number.
fn distance(bits: u32) -> Option<f64> {
    let d: f64 = f32::from_bits(bits).to_string().parse().ok()?;
    (d.is_finite() && d > 0.0).then_some(d)
}

/// The keys' ids in a blend's record `words`, in its order; `None` when they aren't there.
fn record_ids(words: &[u32]) -> Option<Vec<String>> {
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
/// `drawn` (its steps as the file draws them) make, with the keys and spacing of `record`: one
/// live blend, or `None` when they don't agree or it doesn't come out as drawn.
///
/// `budget`: how many more objects blends may lay out in this import; a blend that would lay out
/// more isn't made live (each blend is laid out twice, once per orientation, and checked against
/// its drawn steps, so a file of huge blends mustn't stall the import). What it lays out is taken
/// from it.
pub(super) fn live_blend(edit: &Node, drawn: &Node, record: &Record, roles: &Roles, budget: &mut usize) -> Option<NodeKind> {
    let ids = &record.ids;
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
    let drawn: Vec<&Arc<Node>> = drawn.children()?.iter().collect();
    if drawn.len() < keys.len() || drawn.len() > MAX_BLEND_OBJECTS {
        return None;
    }
    let keys: Vec<Arc<Node>> = keys
        .into_iter()
        .map(|k| {
            let mut k = (*k).clone();
            k.visible = true;
            Arc::new(k)
        })
        .collect();
    // A blend whose spine was never edited has none written: its spine is the straight lines
    // between the keys' centres, in the record's order.
    let spine = match spine {
        Some(spine) => spine,
        None => straight_spine(&keys, ids, roles)?,
    };
    let key_anchors = on_spine(&keys, &spine)?;
    let spacing = match record.spacing {
        // Specified Steps: the steps between two keys are the drawn objects that aren't keys,
        // shared out.
        None => {
            let between = (drawn.len() - keys.len()).checked_div(keys.len() - 1)?;
            BlendSpacing::Steps(u32::try_from(between).ok().filter(|n| (1..=MAX_STEPS as u32).contains(n))?)
        }
        Some(spacing) => spacing,
    };
    // Both orientations are laid out, as many objects as are drawn each.
    let cost = drawn.len().saturating_mul(2);
    if cost > *budget {
        return None;
    }
    let spec = BlendSpec { spacing, spine: Some(spine), key_anchors, ..BlendSpec::default() };
    // As many objects as the spacing lays out between each pair of keys (the orientation doesn't
    // change how many), counted before anything is laid out: as many as are drawn.
    if blend_steps(&keys, &spec).into_iter().fold(keys.len(), usize::saturating_add) != drawn.len() {
        return None;
    }
    *budget -= cost;
    // The drawn steps' centres and paints, once.
    let drawn: Vec<(Point, Fill)> = drawn.iter().map(|d| Some((centre(d)?, fill(d)))).collect::<Option<_>>()?;
    // The orientation that lays the steps out as drawn, the closest when both do.
    [BlendOrientation::AlignToPage, BlendOrientation::AlignToPath]
        .into_iter()
        .filter_map(|orientation| {
            let spec = BlendSpec { orientation, ..spec.clone() };
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

/// The straight lines between the centres of `keys`, taken in the order of the record's `ids`: a
/// blend's spine until it is edited or replaced, as [`vectorcraft_doc::blend::blend_spine`] lays
/// out a blend with no spine of its own.
fn straight_spine(keys: &[Arc<Node>], ids: &[String], roles: &Roles) -> Option<PathData> {
    let centres = ids
        .iter()
        .map(|id| {
            let key = keys.iter().find(|k| roles.get(&k.id).and_then(|(_, named)| named.as_ref()) == Some(id))?;
            centre(key)
        })
        .collect::<Option<Vec<Point>>>()?;
    Some(PathData::single(SubPath::polyline(&centres, false)))
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

    /// Records that aren't one: too short, another magic, an unknown spacing, key counts out of
    /// range or not dividing the rest, ids that aren't letters or digits, and hex digits that
    /// don't pair up. None of them panics.
    #[test]
    fn records_are_checked() {
        let words = |w: &[u32]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let magic = u32::from_le_bytes(*b"DCBA");
        let good = words(&[magic, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 'A' as u32, 0, 0, 0, 0, 0, 'B' as u32, 0, 0, 0, 0, 0]);
        assert_eq!(record(&good).map(|r| r.ids), Some(vec!["A".to_string(), "B".to_string()]));
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
            assert_eq!(record(&bad), None, "{bad:?}");
        }
    }

    /// The spacing at word 1 (0 Smooth Color, 1 Specified Steps, 2 Specified Distance) and, for
    /// Specified Distance, the distance at word 2 as a 32-bit float, read as the decimal it was
    /// typed as. Specified Steps leaves the steps to be counted from the drawn ones, whatever word
    /// 4 holds. An unknown spacing, or a distance that isn't a positive number, isn't read.
    #[test]
    fn records_say_their_spacing() {
        let words = |spacing: u32, distance: f32, steps: u32| {
            let magic = u32::from_le_bytes(*b"DCBA");
            [magic, spacing, distance.to_bits(), 0, steps, 0, 0, 0, 0, 0, 2, 'A' as u32, 0, 0, 0, 0, 0, 'B' as u32, 0, 0, 0, 0, 0]
                .iter()
                .flat_map(|x| x.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let spacing = |b: Vec<u8>| record(&b).map(|r| r.spacing);
        assert_eq!(spacing(words(0, 4.0, 254)), Some(Some(BlendSpacing::SmoothColor)));
        assert_eq!(spacing(words(1, 50.0, 3)), Some(None));
        assert_eq!(spacing(words(1, 0.0, 0)), Some(None));
        assert_eq!(spacing(words(2, 12.5, 7)), Some(Some(BlendSpacing::Distance(12.5))));
        assert_eq!(spacing(words(2, 2.3, 0)), Some(Some(BlendSpacing::Distance(2.3))));
        assert_eq!(spacing(words(2, 0.1, 0)), Some(Some(BlendSpacing::Distance(0.1))));
        assert_eq!(spacing(words(2, 12345.67, 0)), Some(Some(BlendSpacing::Distance(12345.67))));
        for (kind, distance) in [(2, 0.0), (2, -10.0), (2, f32::NAN), (2, f32::INFINITY), (3, 10.0), (u32::MAX, 10.0)] {
            assert_eq!(spacing(words(kind, distance, 7)), None, "spacing {kind}, distance {distance}");
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
        let record = Record { ids: vec!["A".into(), "B".into()], spacing: None };
        assert!(live_blend(&edit, &drawn, &record, &Roles::new(), &mut budget).is_none());
        assert_eq!(budget, 1);
    }

    /// Two 20 pt keys (`A` at x 0 and `B` at x 100) on a straight spine through their centers,
    /// with three 20 pt steps drawn evenly between them: the blend's edit group, its drawn steps
    /// and the roles naming its parts.
    fn two_keys() -> (Node, Node, Roles) {
        let square = |id: u64, x: f64| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 40.0, x + 20.0, 60.0));
            Arc::new(Node::path(NodeId(id), path, Default::default()))
        };
        let spine = vectorcraft_geom::shapes::line(Point::new(10.0, 50.0), Point::new(110.0, 50.0));
        let spine = Arc::new(Node::path(NodeId(3), spine, Default::default()));
        let edit = Node::new(NodeId(10), NodeKind::Group { children: vec![square(1, 0.0), square(2, 100.0), spine], clip: false });
        let steps = (0..5u32).map(|i| square(20 + u64::from(i), f64::from(i) * 25.0)).collect();
        let drawn = Node::new(NodeId(11), NodeKind::Group { children: steps, clip: false });
        let roles = Roles::from([
            (NodeId(1), ("K".to_string(), Some("A".to_string()))),
            (NodeId(2), ("K".to_string(), Some("B".to_string()))),
            (NodeId(3), ("S".to_string(), None)),
        ]);
        (edit, drawn, roles)
    }

    /// The budget: a blend whose two layouts (one per orientation, five objects each) fit in what
    /// is left is made live and takes ten; one that doesn't fit, or whose spacing lays out another
    /// number of steps than are drawn, isn't made live and takes nothing.
    #[test]
    fn a_blend_takes_from_the_budget_only_when_it_is_laid_out() {
        let (edit, drawn, roles) = two_keys();
        let steps = Record { ids: vec!["A".into(), "B".into()], spacing: None };
        let mut budget = 10;
        assert!(matches!(live_blend(&edit, &drawn, &steps, &roles, &mut budget), Some(NodeKind::Blend { .. })));
        assert_eq!(budget, 0);
        let mut budget = 9;
        assert!(live_blend(&edit, &drawn, &steps, &roles, &mut budget).is_none());
        assert_eq!(budget, 9, "a blend past the budget takes nothing");
        let nine = Record { spacing: Some(BlendSpacing::Distance(10.0)), ..steps };
        let mut budget = 10;
        assert!(live_blend(&edit, &drawn, &nine, &roles, &mut budget).is_none());
        assert_eq!(budget, 10, "a spacing that lays out nine steps where three are drawn takes nothing");
    }

    /// A distance longer than the keys are apart lays out no steps: a blend drawn as its two keys
    /// alone comes in live with that distance, and one of Specified Steps drawn that way doesn't.
    #[test]
    fn a_distance_past_the_keys_lays_out_no_steps() {
        let (edit, drawn, roles) = two_keys();
        let NodeKind::Group { children, .. } = &drawn.kind else { panic!("a group") };
        let keys_alone = [children[0].clone(), children[4].clone()];
        let drawn = Node::new(NodeId(11), NodeKind::Group { children: keys_alone.to_vec(), clip: false });
        let far = Record { ids: vec!["A".into(), "B".into()], spacing: Some(BlendSpacing::Distance(150.0)) };
        let mut budget = 10;
        let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &far, &roles, &mut budget) else { panic!("a live blend") };
        assert_eq!(spec.spacing, BlendSpacing::Distance(150.0));
        assert_eq!(budget, 6);
        let steps = Record { spacing: None, ..far };
        assert!(live_blend(&edit, &drawn, &steps, &roles, &mut budget).is_none());
        assert_eq!(budget, 6);
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
