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
//! is the distance in points, as a 32-bit float. Word 5 is 0 for Align to Page and, for Align to
//! Path, a 32-bit float of 8 or more. Word 10 is the number of keys, and the words after it are a
//! record of six words per key that starts with the key's id. For Specified Steps, the steps
//! between two keys are counted from the steps drawn. The orientation is the record's when it lays
//! the blend out as the steps are drawn, else whichever one does (the closest when both do). A blend
//! with no spine part whose record names its keys in their order has no spine of its own: it runs
//! straight between its keys' centers. A blend whose parts don't agree, whose spacing isn't one of
//! these, or that doesn't come out as drawn, keeps its drawn steps.

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::blend::{blend_expand, blend_steps};
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
use vectorcraft_doc::{AppearanceItem, Node, NodeId, NodeKind};
use vectorcraft_geom::{ParamCurve, PathData, Point, SubPath};

/// The plugin that writes blends.
pub(crate) const PLUGIN: &str = "Adobe Path Blends"; // brand-ok: the plugin name the files use
/// The most bytes of a blend's record read.
pub(super) const MAX_RECORD: usize = 4096;
/// The most keys read from one blend.
pub(crate) const MAX_KEYS: usize = 64;
/// The most steps between two keys (Blend Options' limit).
const MAX_STEPS: usize = 1000;
/// The most objects (keys and steps) one blend may lay out to be read live; more keep their
/// drawn steps.
pub(crate) const MAX_BLEND_OBJECTS: usize = 5_000;
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
    /// The orientation word 5 names, when it names one.
    pub(super) orientation: Option<BlendOrientation>,
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
    let orientation = words.get(5).and_then(|w| orientation(*w));
    Some(Record { ids, spacing, orientation })
}

/// The orientation in a record's word 5: 0 is Align to Page, a 32-bit float of 8 or more Align to
/// Path; anything else names none.
fn orientation(bits: u32) -> Option<BlendOrientation> {
    if bits == 0 {
        return Some(BlendOrientation::AlignToPage);
    }
    let a = f32::from_bits(bits);
    (a.is_finite() && a >= 8.0).then_some(BlendOrientation::AlignToPath)
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
    let in_order = named.iter().zip(ids).all(|(n, id)| *n == id.as_str());
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
    // between the keys' centers, in the record's order. With the keys in that order it is the
    // spine of a blend that has none of its own.
    let (spine, key_anchors) = match spine {
        Some(spine) => {
            let anchors = on_spine(&keys, &spine)?;
            (Some(spine), anchors)
        }
        None if in_order => (None, vec![]),
        None => {
            let spine = straight_spine(&keys, ids, roles)?;
            let anchors = on_spine(&keys, &spine)?;
            (Some(spine), anchors)
        }
    };
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
    let spec = BlendSpec { spacing, spine, key_anchors, ..BlendSpec::default() };
    // As many objects as the spacing lays out between each pair of keys (the orientation doesn't
    // change how many), counted before anything is laid out: as many as are drawn.
    if blend_steps(&keys, &spec).into_iter().fold(keys.len(), usize::saturating_add) != drawn.len() {
        return None;
    }
    *budget -= cost;
    // The drawn steps as the check compares them, once.
    let drawn: Vec<Drawn> = drawn.iter().map(|d| drawn_as(d)).collect::<Option<_>>()?;
    // The orientations that lay the steps out as drawn (the check compares the steps' centers, which
    // turning the steps along the spine leaves in place). Of those that also lay them out at their
    // drawn sizes and turned as drawn, the record's, else the closest; when none does, the closest
    // of them all.
    let fits: Vec<(f64, BlendSpec, Vec<Node>)> = [BlendOrientation::AlignToPage, BlendOrientation::AlignToPath]
        .into_iter()
        .filter_map(|orientation| {
            let spec = BlendSpec { orientation, ..spec.clone() };
            let laid = blend_expand(&keys, &spec);
            let off = how_far(&laid, &drawn, false)?;
            Some((off, spec, laid))
        })
        .collect();
    let as_drawn: Vec<&(f64, BlendSpec, Vec<Node>)> = fits.iter().filter(|(_, _, laid)| how_far(laid, &drawn, true).is_some()).collect();
    as_drawn
        .iter()
        .copied()
        .find(|(_, s, _)| Some(s.orientation) == record.orientation)
        .or_else(|| as_drawn.iter().copied().min_by(|a, b| a.0.total_cmp(&b.0)))
        .or_else(|| fits.iter().min_by(|a, b| a.0.total_cmp(&b.0)))
        .map(|(_, spec, _)| NodeKind::Blend { children: keys, spec: Box::new(spec.clone()) })
}

/// What the reader makes of a blend written as the key objects `keys`, named by `ids` in their
/// order, the spine `spine` (`None`: no spine part), the record `bytes` and the drawn steps `drawn`:
/// the live blend's settings, or `None` where it keeps the drawn steps. Takes from `budget` as
/// [`live_blend`] does.
pub(crate) fn blend_reads(
    keys: &[Arc<Node>],
    ids: &[String],
    spine: Option<&PathData>,
    bytes: &[u8],
    drawn: &[Node],
    budget: &mut usize,
) -> Option<BlendSpec> {
    let record = record(bytes)?;
    let mut roles = Roles::new();
    let mut parts: Vec<Arc<Node>> = Vec::with_capacity(keys.len() + 1);
    for (k, id) in keys.iter().zip(ids) {
        roles.insert(k.id, ("K".to_string(), Some(id.clone())));
        parts.push(k.clone());
    }
    if let Some(s) = spine {
        let id = NodeId(u64::MAX);
        roles.insert(id, ("S".to_string(), None));
        parts.push(Arc::new(Node::path(id, s.clone(), Default::default())));
    }
    let edit = Node::new(NodeId(u64::MAX - 1), NodeKind::Group { children: parts, clip: false });
    let drawn = Node::new(NodeId(u64::MAX - 2), NodeKind::Group { children: drawn.iter().cloned().map(Arc::new).collect(), clip: false });
    match live_blend(&edit, &drawn, &record, &roles, budget)? {
        NodeKind::Blend { spec, .. } => Some(*spec),
        _ => None,
    }
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

/// The most anchors of an object's paths the check compares; an object with more is compared by its
/// size alone.
const MAX_MARK_ANCHORS: usize = 64;
/// The most drawn objects whose outlines one check compares, spread evenly from the first to the
/// last; the rest are compared by their sizes alone (the orientation turns every step alike).
const MAX_OUTLINE_CHECKS: usize = 16;
/// The most outline comparisons one check makes (a drawn object compared can have more than one
/// laid object in its place); past them, a drawn object whose outline is to be compared matches
/// none.
const MAX_OUTLINE_COMPARISONS: usize = 4 * MAX_OUTLINE_CHECKS;

/// An object as the check compares it.
#[derive(Clone, Copy, Debug)]
struct Drawn<'a> {
    centre: Point,
    fill: Fill,
    /// Its width and height.
    size: Point,
    /// The object, for its paths' outline ([`outline`]), which turning it moves.
    node: &'a Node,
}

/// `n` as the check compares it, or `None` when it has no finite bounds.
fn drawn_as(n: &Node) -> Option<Drawn<'_>> {
    let size = n.geometric_bounds().map(|b| Point::new(b.width(), b.height())).filter(|p| p.x.is_finite() && p.y.is_finite())?;
    Some(Drawn { centre: centre(n)?, fill: fill(n), size, node: n })
}

/// How the check compares an object's turn.
enum Outline {
    /// By its outline: every subpath of more than one anchor of its paths, in any order, each with
    /// its path's fill, so that a turn that trades the places of paths painted differently doesn't
    /// pass.
    Paths(Vec<(Fill, SubPath)>),
    /// By its size alone: it holds art whose drawing isn't its members (a blend, an envelope, a
    /// repeat), art past eight levels down, more than [`MAX_MARK_ANCHORS`] anchors or no subpath to
    /// compare.
    Size,
}

/// How the check compares `n`'s turn (the members of its groups and compound paths looked through,
/// at most eight levels down; a compound path's members take its fill).
fn outline(n: &Node) -> Outline {
    /// What the walk found.
    #[derive(Default)]
    struct Walk {
        subpaths: Vec<(Fill, SubPath)>,
        anchors: usize,
        /// Art compared by the size alone, or too many anchors.
        size_only: bool,
    }
    fn add(n: &Node, depth: usize, painted: Option<Fill>, w: &mut Walk) {
        if w.size_only {
            return;
        }
        match &n.kind {
            NodeKind::Path { path, .. } => {
                let f = painted.unwrap_or_else(|| fill(n));
                // A subpath of one anchor has no segment to compare with.
                for s in path.subpaths.iter().filter(|s| s.anchors.len() > 1) {
                    w.anchors = w.anchors.saturating_add(s.anchors.len());
                    if w.anchors > MAX_MARK_ANCHORS {
                        w.size_only = true;
                        return;
                    }
                    w.subpaths.push((f, s.clone()));
                }
            }
            NodeKind::Group { children, .. } if depth < 8 => {
                for c in children {
                    add(c, depth + 1, painted, w);
                }
            }
            NodeKind::Compound { children, .. } if depth < 8 => {
                let f = painted.unwrap_or_else(|| fill(n));
                for c in children {
                    add(c, depth + 1, Some(f), w);
                }
            }
            _ => w.size_only = true,
        }
    }
    let mut w = Walk::default();
    add(n, 0, None, &mut w);
    if w.size_only || w.subpaths.is_empty() { Outline::Size } else { Outline::Paths(w.subpaths) }
}

/// Do two outlines ([`Outline::Paths`]) draw the same lines in the same fills: each anchor of one,
/// and the points a quarter, half and three quarters along each of its segments (which show how a
/// curved side bends), within `tolerance` of the other's subpaths of a fill that matches its own?
/// Where a path starts, how many anchors it has on a straight or curved stretch, and the order of
/// the paths don't count.
fn same_outline(a: &[(Fill, SubPath)], b: &[(Fill, SubPath)], tolerance: f64) -> bool {
    let on = |from: &[(Fill, SubPath)], to: &[(Fill, SubPath)]| {
        from.iter().all(|(f, s)| {
            // Fills the check can't compare (gradients, patterns) are compared with each other by
            // their outline alone.
            let to = PathData::new(
                to.iter().filter(|(g, _)| same_fill(*f, *g) || (*f == Fill::Other && *g == Fill::Other)).map(|(_, t)| t.clone()).collect(),
            );
            let near = |p: Point| to.nearest(p).is_some_and(|(.., d)| d <= tolerance);
            s.anchors.iter().all(|p| near(p.p))
                && (0..s.segment_count()).all(|i| {
                    let c = s.segment(i);
                    [0.25, 0.5, 0.75].into_iter().all(|t| near(c.eval(t)))
                })
        })
    };
    on(a, b) && on(b, a)
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
/// matches in place and fill, and with `turned` also in width and height and, for at most
/// [`MAX_OUTLINE_CHECKS`] drawn objects spread from the first to the last, in the outline of its
/// paths (each within twice the tolerance), which turning an object changes; else `None` (also when
/// that matching leaves one out, and when it would take more than [`MAX_OUTLINE_COMPARISONS`]
/// outline comparisons).
fn how_far(laid: &[Node], drawn: &[Drawn], turned: bool) -> Option<f64> {
    if laid.len() != drawn.len() {
        return None;
    }
    let cell = |p: Point| ((p.x / PLACE_TOLERANCE).floor() as i64, (p.y / PLACE_TOLERANCE).floor() as i64);
    let laid: Vec<Drawn> = laid.iter().map(drawn_as).collect::<Option<_>>()?;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::with_capacity(laid.len());
    for (i, l) in laid.iter().enumerate() {
        grid.entry(cell(l.centre)).or_default().push(i);
    }
    let close = |a: Point, b: Point| (a.x - b.x).abs() <= 2.0 * PLACE_TOLERANCE && (a.y - b.y).abs() <= 2.0 * PLACE_TOLERANCE;
    // The drawn objects whose outlines are compared: all of them, or as many as the limit allows,
    // spread evenly so that the first and the last are among them.
    let mut compare_outline = vec![drawn.len() <= MAX_OUTLINE_CHECKS; drawn.len()];
    if drawn.len() > MAX_OUTLINE_CHECKS {
        for j in 0..MAX_OUTLINE_CHECKS {
            if let Some(c) = compare_outline.get_mut(j * (drawn.len() - 1) / (MAX_OUTLINE_CHECKS - 1)) {
                *c = true;
            }
        }
    }
    let outlines_compared = std::cell::Cell::new(0usize);
    let same_turn = |l: &Drawn, d: &Drawn, compare: bool| {
        if !turned {
            return true;
        }
        if !close(l.size, d.size) || !compare {
            return close(l.size, d.size);
        }
        // An outline that isn't compared doesn't count as the same.
        if outlines_compared.get() >= MAX_OUTLINE_COMPARISONS {
            return false;
        }
        outlines_compared.set(outlines_compared.get() + 1);
        match (outline(l.node), outline(d.node)) {
            (Outline::Paths(a), Outline::Paths(b)) => same_outline(&a, &b, 2.0 * PLACE_TOLERANCE),
            // Live art against its drawing, too many anchors, or no paths: compared by the size
            // alone.
            _ => true,
        }
    };
    let mut used = vec![false; laid.len()];
    let mut worst: f64 = 0.0;
    for (k, d) in drawn.iter().enumerate() {
        let (c, f) = (d.centre, d.fill);
        let compare = compare_outline.get(k).copied().unwrap_or(false);
        let (cx, cy) = cell(c);
        // Coordinates far out saturate the cells: the neighbours' keys saturate too.
        let candidates = (-1i64..=1)
            .flat_map(|dx| (-1i64..=1).map(move |dy| (cx.saturating_add(dx), cy.saturating_add(dy))))
            .filter_map(|k| grid.get(&k))
            .flatten()
            .filter(|&&i| !used.get(i).copied().unwrap_or(true))
            .filter_map(|&i| laid.get(i).map(|l| (i, l)))
            .filter(|(_, l)| (l.centre - c).hypot() <= PLACE_TOLERANCE && same_fill(l.fill, f));
        let (i, near) = if turned && compare {
            // Outlines are tried nearest first, so that steps crowded in one place don't spend the
            // outline comparisons on candidates farther off.
            let mut nearest: Vec<(usize, f64)> =
                candidates.filter(|(_, l)| close(l.size, d.size)).map(|(i, l)| (i, (l.centre - c).hypot())).collect();
            nearest.sort_by(|a, b| a.1.total_cmp(&b.1));
            nearest.into_iter().find(|(i, _)| laid.get(*i).is_some_and(|l| same_turn(l, d, true)))?
        } else {
            candidates.filter(|(_, l)| same_turn(l, d, false)).map(|(i, l)| (i, (l.centre - c).hypot())).min_by(|a, b| a.1.total_cmp(&b.1))?
        };
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
        let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: None };
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
        let steps = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: None };
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
        let far = Record { ids: vec!["A".into(), "B".into()], spacing: Some(BlendSpacing::Distance(150.0)), orientation: None };
        let mut budget = 10;
        let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &far, &roles, &mut budget) else { panic!("a live blend") };
        assert_eq!(spec.spacing, BlendSpacing::Distance(150.0));
        assert_eq!(budget, 6);
        let steps = Record { spacing: None, ..far };
        assert!(live_blend(&edit, &drawn, &steps, &roles, &mut budget).is_none());
        assert_eq!(budget, 6);
    }

    /// Word 5 names the orientation: 0 Align to Page, a 32-bit float of 8 or more (8 and an angle)
    /// Align to Path; any other value names none, and a record too short to hold it names none.
    #[test]
    fn records_say_their_orientation() {
        let magic = u32::from_le_bytes(*b"DCBA");
        let words = |w5: u32| {
            [magic, 1, 50f32.to_bits(), 0, 3, w5, 'C' as u32, 0, 0, 1, 2, 'A' as u32, 0, 0, u32::MAX, u32::MAX, 0, 'B' as u32, 0, 1f32.to_bits()]
                .into_iter()
                .chain([u32::MAX, u32::MAX, 3])
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<u8>>()
        };
        let orientation = |w5: u32| record(&words(w5)).map(|r| r.orientation);
        assert_eq!(orientation(0), Some(Some(BlendOrientation::AlignToPage)));
        assert_eq!(orientation(8f32.to_bits()), Some(Some(BlendOrientation::AlignToPath)));
        assert_eq!(orientation(11.141593f32.to_bits()), Some(Some(BlendOrientation::AlignToPath)));
        for other in [1f32.to_bits(), 7.9f32.to_bits(), (-9f32).to_bits(), f32::NAN.to_bits(), f32::INFINITY.to_bits()] {
            assert_eq!(orientation(other), Some(None), "{other:#x}");
        }
    }

    /// On a level straight spine both orientations lay two squares' steps out alike: the blend takes
    /// the record's orientation, and Align to Page when the record names none.
    #[test]
    fn the_records_orientation_is_kept_where_both_lay_out_alike() {
        let (edit, drawn, roles) = two_keys();
        for (named, want) in [
            (Some(BlendOrientation::AlignToPath), BlendOrientation::AlignToPath),
            (Some(BlendOrientation::AlignToPage), BlendOrientation::AlignToPage),
            (None, BlendOrientation::AlignToPage),
        ] {
            let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: named };
            let mut budget = 10;
            let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &record, &roles, &mut budget) else { panic!("a live blend") };
            assert_eq!(spec.orientation, want, "{named:?}");
        }
    }

    /// Two drawn steps on one laid object don't both match it: each laid object matches one drawn
    /// one, so a drawing the blend doesn't lay out isn't taken for it.
    #[test]
    fn each_laid_step_matches_one_drawn_step() {
        let square = |x: f64| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 0.0, x + 10.0, 10.0));
            Node::path(NodeId(0), path, Default::default())
        };
        // `laid` against the objects `drawn` as the check compares them.
        let check = |laid: &[Node], drawn: &[Node], turned: bool| {
            let drawn: Vec<Drawn> = drawn.iter().map(|d| drawn_as(d).expect("bounds")).collect();
            how_far(laid, &drawn, turned)
        };
        let laid = [square(0.0), square(50.0)];
        assert!(check(&laid, &[square(0.0), square(50.0)], true).is_some());
        assert!(check(&laid, &[square(0.0), square(0.1)], false).is_none(), "both drawn steps near the first laid one");
        // Sizes and outlines count only when asked for: a wider rectangle with the same center, and
        // a triangle turned a half turn.
        let wide = |x: f64| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x - 5.0, 2.5, x + 15.0, 7.5));
            Node::path(NodeId(0), path, Default::default())
        };
        assert!(check(&laid, &[wide(0.0), wide(50.0)], false).is_some());
        assert!(check(&laid, &[wide(0.0), wide(50.0)], true).is_none());
        // Steps on one center match by their size, in whichever order they are drawn.
        let big = |x: f64| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x - 5.0, -5.0, x + 15.0, 15.0));
            Node::path(NodeId(0), path, Default::default())
        };
        assert!(check(&[square(0.0), big(0.0)], &[big(0.0), square(0.0)], true).is_some());
        // A group's members in close reds match in whichever order they are drawn.
        let reds = |x: f64, order: [usize; 3]| {
            let member = |k: usize| {
                let left = x + k as f64 * 10.0;
                let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(left, 0.0, left + 8.0, 8.0));
                let red = [0.25, 0.265, 0.28].get(k).copied().unwrap_or(0.0);
                Arc::new(Node::path(NodeId(0), path, vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(red, 0.0, 0.0)), Paint::None, 0.0)))
            };
            Node::group(NodeId(0), order.iter().map(|k| member(*k)).collect())
        };
        assert!(check(&[reds(0.0, [0, 1, 2]), reds(50.0, [0, 1, 2])], &[reds(0.0, [1, 0, 2]), reds(50.0, [1, 0, 2])], true).is_some());
        // A triangle turned a half turn has its anchors elsewhere.
        let polygon = |points: &[Point]| Node::path(NodeId(0), PathData::single(SubPath::polyline(points, true)), Default::default());
        let triangle = |x: f64, turn: f64| {
            let mut n = polygon(&[Point::new(x, 0.0), Point::new(x, 10.0), Point::new(x + 10.0, 5.0)]);
            n.transform(vectorcraft_geom::Affine::rotate_about(turn, Point::new(x + 5.0, 5.0)), false);
            n
        };
        let laid = [triangle(0.0, 0.0), triangle(50.0, 0.0)];
        let turned = |x: f64| triangle(x, std::f64::consts::PI);
        assert!(check(&laid, &[turned(0.0), turned(50.0)], false).is_some());
        assert!(check(&laid, &[turned(0.0), turned(50.0)], true).is_none());
        // So has one in a group, filled with a gradient, which the check compares by its outline.
        let graded = |x: f64, turn: f64| {
            let mut t = triangle(x, turn);
            let gradient = vectorcraft_color::gradient::GradientPaint::new(vectorcraft_color::gradient::Gradient::default());
            t.appearance.set_fill(Paint::Gradient(Box::new(gradient)));
            Node::group(NodeId(0), vec![Arc::new(t)])
        };
        let in_groups = [graded(0.0, 0.0), graded(50.0, 0.0)];
        assert!(check(&in_groups, &[graded(0.0, 0.0), graded(50.0, 0.0)], true).is_some());
        assert!(check(&in_groups, &[graded(0.0, std::f64::consts::PI), graded(50.0, std::f64::consts::PI)], true).is_none());
        // The same triangle drawn from another of its anchors matches.
        let shifted = |x: f64| polygon(&[Point::new(x + 10.0, 5.0), Point::new(x, 0.0), Point::new(x, 10.0)]);
        assert!(check(&laid, &[shifted(0.0), shifted(50.0)], true).is_some());
        // So does the same triangle drawn with another anchor on one of its sides.
        let split = |x: f64| polygon(&[Point::new(x, 0.0), Point::new(x, 5.0), Point::new(x, 10.0), Point::new(x + 10.0, 5.0)]);
        assert!(check(&laid, &[split(0.0), split(50.0)], true).is_some());
        // A square with its top side bent inward has the same corners as its half turn, whose bent
        // side is at the bottom: the points along its sides tell them apart.
        let bent = |x: f64, turn: f64| {
            let (tl, tr) = (Point::new(x, 0.0), Point::new(x + 10.0, 0.0));
            let anchors = vec![
                vectorcraft_geom::Anchor::with_handles(tl, tl, Point::new(x + 3.0, 4.0)),
                vectorcraft_geom::Anchor::with_handles(tr, Point::new(x + 7.0, 4.0), tr),
                vectorcraft_geom::Anchor::corner(Point::new(x + 10.0, 10.0)),
                vectorcraft_geom::Anchor::corner(Point::new(x, 10.0)),
            ];
            let mut n = Node::path(NodeId(0), PathData::single(SubPath::new(anchors, true)), Default::default());
            n.transform(vectorcraft_geom::Affine::rotate_about(turn, Point::new(x + 5.0, 5.0)), false);
            n
        };
        let upright = [bent(0.0, 0.0), bent(50.0, 0.0)];
        assert!(check(&upright, &[bent(0.0, 0.0), bent(50.0, 0.0)], true).is_some());
        let half = std::f64::consts::PI;
        assert!(check(&upright, &[bent(0.0, half), bent(50.0, half)], false).is_some());
        assert!(check(&upright, &[bent(0.0, half), bent(50.0, half)], true).is_none(), "the bent side at the bottom");
        // Past the limit of outlines compared, the last object is still among them.
        let row = |last: Node| {
            let mut row: Vec<Node> = (0..MAX_OUTLINE_CHECKS).map(|i| square(20.0 * i as f64)).collect();
            row.push(last);
            row
        };
        let end = 20.0 * MAX_OUTLINE_CHECKS as f64;
        assert!(check(&row(bent(end, 0.0)), &row(bent(end, 0.0)), true).is_some());
        assert!(check(&row(bent(end, 0.0)), &row(bent(end, half)), true).is_none(), "the last one turned");
        // A hundred steps crowded within the tolerance of each other match their drawn copies:
        // each drawn one is compared with the nearest laid one first.
        let crowd = |turn: f64| (0..100u32).map(|i| triangle(0.4 * f64::from(i) / 99.0, turn)).collect::<Vec<_>>();
        assert!(check(&crowd(0.0), &crowd(0.0), true).is_some());
        assert!(check(&crowd(0.0), &crowd(half), true).is_none(), "the crowd turned");
        // A compound path matches with its contours in another order, and not turned a half turn.
        let pair = |x: f64, flip: bool, turn: f64| {
            let a = SubPath::polyline(&[Point::new(x, 0.0), Point::new(x, 4.0), Point::new(x + 10.0, 2.0)], true);
            let b = SubPath::polyline(&[Point::new(x, 6.0), Point::new(x, 10.0), Point::new(x + 4.0, 8.0)], true);
            let subpaths = if flip { vec![b, a] } else { vec![a, b] };
            let mut n = Node::path(NodeId(0), PathData::new(subpaths), Default::default());
            n.transform(vectorcraft_geom::Affine::rotate_about(turn, Point::new(x + 5.0, 5.0)), false);
            n
        };
        let laid = [pair(0.0, false, 0.0), pair(50.0, false, 0.0)];
        assert!(check(&laid, &[pair(0.0, true, 0.0), pair(50.0, true, 0.0)], true).is_some());
        assert!(check(&laid, &[pair(0.0, true, std::f64::consts::PI), pair(50.0, true, std::f64::consts::PI)], true).is_none());
        // A lone anchor in a path draws nothing: the same path with one matches itself.
        let dotted = |x: f64| {
            let triangle = SubPath::polyline(&[Point::new(x, 0.0), Point::new(x, 10.0), Point::new(x + 10.0, 5.0)], true);
            let dot = SubPath::polyline(&[Point::new(x + 3.0, 5.0)], false);
            Node::path(NodeId(0), PathData::new(vec![triangle, dot]), Default::default())
        };
        assert!(check(&[dotted(0.0), dotted(50.0)], &[dotted(0.0), dotted(50.0)], true).is_some());
        // A live blend draws more than its keys: one is compared with its drawn copy by size alone.
        let blend = |x: f64| {
            let keys = vec![Arc::new(triangle(x, 0.0)), Arc::new(triangle(x, 0.0))];
            Node::new(NodeId(0), NodeKind::Blend { children: keys, spec: Box::default() })
        };
        let expanded = |x: f64| {
            let inside = SubPath::polyline(&[Point::new(x + 3.0, 4.0), Point::new(x + 3.0, 6.0)], false);
            let inside = Node::path(NodeId(0), PathData::single(inside), Default::default());
            Node::group(NodeId(0), vec![Arc::new(triangle(x, 0.0)), Arc::new(inside)])
        };
        assert!(check(&[blend(0.0), blend(50.0)], &[expanded(0.0), expanded(50.0)], true).is_some());
        // Coordinates far out don't overflow the grid.
        let far = [square(1e300), square(-1e300)];
        let _ = check(&far, &[square(1e300), square(-1e300)], false);
    }

    /// Triangles pointing right on a spine running right to left, with their steps drawn pointing
    /// right: Align to Path would turn each one (keys included) a half turn, which leaves their
    /// centers and sizes as they are but not their points. A record naming Align to Path doesn't
    /// make the blend Align to Path.
    #[test]
    fn the_records_orientation_must_lay_the_steps_out_turned_as_drawn() {
        let triangle = |id: u64, x: f64| {
            let points = [Point::new(x, 0.0), Point::new(x, 10.0), Point::new(x + 20.0, 5.0)];
            Arc::new(Node::path(NodeId(id), PathData::single(SubPath::polyline(&points, true)), Default::default()))
        };
        let spine = vectorcraft_geom::shapes::line(Point::new(110.0, 5.0), Point::new(10.0, 5.0));
        let spine = Arc::new(Node::path(NodeId(3), spine, Default::default()));
        let edit = Node::new(NodeId(10), NodeKind::Group { children: vec![triangle(1, 100.0), triangle(2, 0.0), spine], clip: false });
        let steps = (0..5u32).map(|i| triangle(20 + u64::from(i), 100.0 - f64::from(i) * 25.0)).collect();
        let drawn = Node::new(NodeId(11), NodeKind::Group { children: steps, clip: false });
        let roles = Roles::from([
            (NodeId(1), ("K".to_string(), Some("A".to_string()))),
            (NodeId(2), ("K".to_string(), Some("B".to_string()))),
            (NodeId(3), ("S".to_string(), None)),
        ]);
        for named in [BlendOrientation::AlignToPath, BlendOrientation::AlignToPage] {
            let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: Some(named) };
            let mut budget = 10;
            let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &record, &roles, &mut budget) else { panic!("a live blend") };
            assert_eq!(spec.orientation, BlendOrientation::AlignToPage, "{named:?}");
        }
    }

    /// Triangles pointing right on a spine 1 pt long running right to left, with 998 steps drawn
    /// unturned, so that every step sits within the tolerance of each one. Align to Path would turn
    /// each one to point left. A record naming Align to Path doesn't make the blend Align to Path,
    /// however many steps one step's place matches; a record naming Align to Page keeps it.
    #[test]
    fn the_records_orientation_is_checked_on_crowded_steps() {
        let triangle = |id: u64, x: f64| {
            let points = [Point::new(x, 0.0), Point::new(x, 10.0), Point::new(x + 20.0, 5.0)];
            Arc::new(Node::path(NodeId(id), PathData::single(SubPath::polyline(&points, true)), Default::default()))
        };
        let spine = vectorcraft_geom::shapes::line(Point::new(11.0, 5.0), Point::new(10.0, 5.0));
        let spine = Arc::new(Node::path(NodeId(3), spine, Default::default()));
        let edit = Node::new(NodeId(10), NodeKind::Group { children: vec![triangle(1, 1.0), triangle(2, 0.0), spine], clip: false });
        let steps = (0..1000u32).map(|i| triangle(20 + u64::from(i), 1.0 - f64::from(i) / 999.0)).collect();
        let drawn = Node::new(NodeId(11), NodeKind::Group { children: steps, clip: false });
        let roles = Roles::from([
            (NodeId(1), ("K".to_string(), Some("A".to_string()))),
            (NodeId(2), ("K".to_string(), Some("B".to_string()))),
            (NodeId(3), ("S".to_string(), None)),
        ]);
        for named in [BlendOrientation::AlignToPath, BlendOrientation::AlignToPage] {
            let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: Some(named) };
            let mut budget = MAX_BLEND_OBJECTS;
            let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &record, &roles, &mut budget) else { panic!("a live blend") };
            assert_eq!(spec.orientation, BlendOrientation::AlignToPage, "{named:?}");
        }
    }

    /// Groups of a red square on the left and a blue one on the right, on a spine running right to
    /// left, with their steps drawn unturned: Align to Path would give each a half turn, which keeps
    /// the group's outline but trades the squares' places. A record naming Align to Path doesn't make
    /// the blend Align to Path.
    #[test]
    fn the_records_orientation_must_keep_each_member_where_it_is_drawn() {
        let square = |id: u64, x: f64, color: Color| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(x, 0.0, x + 10.0, 10.0));
            Arc::new(Node::path(NodeId(id), path, vectorcraft_doc::Appearance::basic(Paint::solid(color), Paint::None, 0.0)))
        };
        let pair = |id: u64, x: f64| {
            let children = vec![square(id * 10, x, Color::rgb(1.0, 0.0, 0.0)), square(id * 10 + 1, x + 10.0, Color::rgb(0.0, 0.0, 1.0))];
            Arc::new(Node::new(NodeId(id), NodeKind::Group { children, clip: false }))
        };
        let spine = vectorcraft_geom::shapes::line(Point::new(110.0, 5.0), Point::new(10.0, 5.0));
        let spine = Arc::new(Node::path(NodeId(3), spine, Default::default()));
        let edit = Node::new(NodeId(4), NodeKind::Group { children: vec![pair(1, 100.0), pair(2, 0.0), spine], clip: false });
        let steps = (0..5u32).map(|i| pair(30 + u64::from(i), 100.0 - f64::from(i) * 25.0)).collect();
        let drawn = Node::new(NodeId(5), NodeKind::Group { children: steps, clip: false });
        let roles = Roles::from([
            (NodeId(1), ("K".to_string(), Some("A".to_string()))),
            (NodeId(2), ("K".to_string(), Some("B".to_string()))),
            (NodeId(3), ("S".to_string(), None)),
        ]);
        for named in [BlendOrientation::AlignToPath, BlendOrientation::AlignToPage] {
            let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: Some(named) };
            let mut budget = 100;
            let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &record, &roles, &mut budget) else { panic!("a live blend") };
            assert_eq!(spec.orientation, BlendOrientation::AlignToPage, "{named:?}");
        }
        // Drawn turned, blue on the left: the blend is Align to Path, whichever orientation the
        // record names.
        let turned = |id: u64, x: f64| {
            let children = vec![square(id * 10, x, Color::rgb(0.0, 0.0, 1.0)), square(id * 10 + 1, x + 10.0, Color::rgb(1.0, 0.0, 0.0))];
            Arc::new(Node::new(NodeId(id), NodeKind::Group { children, clip: false }))
        };
        let steps = (0..5u32).map(|i| turned(30 + u64::from(i), 100.0 - f64::from(i) * 25.0)).collect();
        let drawn = Node::new(NodeId(5), NodeKind::Group { children: steps, clip: false });
        for named in [BlendOrientation::AlignToPath, BlendOrientation::AlignToPage] {
            let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: Some(named) };
            let mut budget = 100;
            let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &record, &roles, &mut budget) else { panic!("a live blend") };
            assert_eq!(spec.orientation, BlendOrientation::AlignToPath, "{named:?}");
        }
    }

    /// Rectangles 20 pt wide and 10 pt high on a vertical spine, with their steps drawn upright: Align
    /// to Path would turn each one (keys included) a quarter turn, which leaves the centers in place
    /// but not the sizes. A record naming Align to Path doesn't make the blend Align to Path; a
    /// record naming Align to Page does.
    #[test]
    fn the_records_orientation_must_lay_the_steps_out_at_their_drawn_sizes() {
        let rect = |id: u64, y: f64| {
            let path = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, y, 20.0, y + 10.0));
            Arc::new(Node::path(NodeId(id), path, Default::default()))
        };
        let spine = vectorcraft_geom::shapes::line(Point::new(10.0, 5.0), Point::new(10.0, 105.0));
        let spine = Arc::new(Node::path(NodeId(3), spine, Default::default()));
        let edit = Node::new(NodeId(10), NodeKind::Group { children: vec![rect(1, 0.0), rect(2, 100.0), spine], clip: false });
        let steps = (0..5u32).map(|i| rect(20 + u64::from(i), f64::from(i) * 25.0)).collect();
        let drawn = Node::new(NodeId(11), NodeKind::Group { children: steps, clip: false });
        let roles = Roles::from([
            (NodeId(1), ("K".to_string(), Some("A".to_string()))),
            (NodeId(2), ("K".to_string(), Some("B".to_string()))),
            (NodeId(3), ("S".to_string(), None)),
        ]);
        for named in [BlendOrientation::AlignToPath, BlendOrientation::AlignToPage] {
            let record = Record { ids: vec!["A".into(), "B".into()], spacing: None, orientation: Some(named) };
            let mut budget = 10;
            let Some(NodeKind::Blend { spec, .. }) = live_blend(&edit, &drawn, &record, &roles, &mut budget) else { panic!("a live blend") };
            assert_eq!(spec.orientation, BlendOrientation::AlignToPage, "{named:?}");
        }
    }
}
