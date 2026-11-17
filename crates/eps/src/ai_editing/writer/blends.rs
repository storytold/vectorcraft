//! Blends: a live blend as the `.ai` reader reads one ([`crate::import`]'s `.ai` reader, `blend.rs`
//! and `read/containers.rs`' `end_blend`).
//!
//! The blend's own art on `%_` lines: `0 Ae`, a group of its key objects, each with an
//! `/ArtDictionary` naming it a key (`ShapeBlendPathType` `K`) and its id (`ShapeBlendPathID`, a
//! letter or digit), and its spine as an unpainted path named the spine (`S`), written only when the
//! blend has a spine of its own; then its steps drawn, a group on plain lines; then `0 Ae`,
//! the `XP` line of the blend plugin ([`BLEND_PLUGIN`], `1 0 n XP`) and the blend's record, `n` bytes
//! as hex on `%` lines; then the blend's name in an `/ArtDictionary` and its transparency, as a
//! group's after its end. The record
//! is `DCBA` and little-endian 32-bit words: 1, the spacing (0 Smooth Color, 1 Specified Steps, 2
//! Specified Distance); 2, the distance as a float (Specified Steps: the spine's length shared out
//! between the steps; Smooth Color: 4); 4, the steps between the first two keys; 5, the orientation
//! (0 Align to Page, a float of 8 and the spine's starting angle in radians for Align to Path); 6,
//! the next free id (0 after the 62nd); 9, 1; 10, the number of keys; then six words a key: its id,
//! 0, the anchor of the spine it sits on as a float, two words of all ones, and the steps between
//! it and the key before it. The reader reads words 1, 2 (for Specified Distance), 5 (0, or 8 or
//! more), 10 and the keys' ids; the others are written as the record's form has them.
//!
//! Each blend is checked with the reader's own reading ([`crate::import::blend_reads`]) before it is
//! written: one the reader would read with other settings is written as its steps.

use std::collections::HashSet;
use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::blend::{blend_spine, blend_steps, blend_steps_settled, pin_spine};
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData};

use super::{MAX_NEST, Writer, Xy, hide};
use crate::import::{BLEND_PLUGIN, MAX_BLEND_KEYS, MAX_BLEND_OBJECTS, blend_reads};

pub(in crate::ai_editing) const BLENDS: &str = "blends with easing, a color shift or start points of their own, blends with effects, an appearance or an opacity mask of their own, blends of objects other than paths, compound paths and groups, of clipping groups, of more than 62 key objects, of hidden key objects, of key objects that hold a blend, of key objects with geometry or Pathfinder effects or a group's own fills and strokes, or of key objects the editing copy would change (empty paths or subpaths, paths and compound path members of several subpaths, several fills or strokes, strokes a plain line can't draw, dashes of more than 64 lengths, hidden fills or strokes, fills or strokes other than a color, in a Lab color or with transparency or effects of their own, effects or opacity masks), blends where one key object has a fill or stroke of no paint or a stroke of no width and another has one that draws, blends inside objects with geometry or Pathfinder effects or a group's own fills and strokes, blends whose spine has more than one part, blends whose step count is close to where rounding changes it, blends that would open with other settings or not as they are drawn, blends in opacity masks, pattern tiles, symbols and art cut to a clipping object or nested eight levels deep in art written in place of other objects (symbol instances, live objects, brushed strokes), blends whose art would pass the most objects the reader reads, and every blend on a large canvas are written as their steps in the editing data for other apps";

/// How far (pt) the lengths a blend counts its steps from may move when written (coordinates are
/// written to 4 decimals and read back through art space) without changing a count.
const SETTLE_LENGTH: f64 = 0.01;
/// How far (pt) each segment of a spine may add to [`SETTLE_LENGTH`]: rounding moves each of its
/// points by up to 0.00005 √2 pt, which moves the segment's length by up to six times that.
const SETTLE_SEGMENT: f64 = 5e-4;
/// How far a key's color (each channel, 0 to 1) may move when written without changing a Smooth
/// Color count.
const SETTLE_COLOR: f64 = 0.001;

/// The most dash lengths the reader reads; a stroke with more opens solid.
const MAX_DASH_LENGTHS: usize = 64;

/// The ids the keys take, in their order (letters and digits, as the reader reads them).
const IDS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Hex digits on one `%` line of a record.
const HEX_LINE: usize = 60;

/// A blend to write live.
pub(super) struct LiveBlend {
    keys: Vec<Arc<Node>>,
    ids: Vec<String>,
    /// Its spine part; `None`: the blend has no spine of its own.
    spine: Option<PathData>,
    record: Vec<u8>,
    steps: Vec<Arc<Node>>,
}

impl Writer<'_> {
    /// Blend `n` as the reader reads it live, or `None` when it would read other settings or none
    /// (the blend is then written as its steps).
    pub(super) fn live_blend(&mut self, n: &Node) -> Option<LiveBlend> {
        let NodeKind::Blend { children: keys, spec } = &n.kind else { return None };
        // Hidden lines (pattern tiles, opacity masks), symbols' definitions and art cut to a
        // clipping object aren't read as live blends. (No blend is drawn in a look: styled objects
        // hold only paths, compound paths and groups of them.)
        if !self.blends_live || self.hidden > 0 || self.in_symbol || self.clip.is_some() || self.nest >= MAX_NEST {
            return None;
        }
        // Baking changes its keys, or the art it sits in ([`unbaked_blends`]).
        if !self.unbaked_blends.contains(&n.id) {
            return None;
        }
        if !spec.ease.is_default() || spec.color_shift != 0.0 || !spec.starts.is_empty() {
            return None;
        }
        let own = &n.appearance;
        if !own.items.is_empty() || !own.effects.is_empty() || n.extras().mask.is_some() {
            return None;
        }
        // The reader shows every key object it reads.
        if keys.len() < 2 || keys.len() > IDS.len().min(MAX_BLEND_KEYS) || !keys.iter().all(|k| k.visible && plain_key(k)) {
            return None;
        }
        // A fill or stroke the editing copy leaves out is read back as none at all, and the steps
        // blend from it to another key's otherwise.
        if left_out_against_drawn(keys) {
            return None;
        }
        // The spine written: a replaced spine (no key anchors) with an anchor where each key sits, a
        // spine edited on the canvas with its keys' anchors moved onto them.
        let spec: &BlendSpec = spec;
        // The blend lays out along the spine's first subpath, and only that part is written: a
        // spine of more parts would open without the others.
        if spec.spine.as_ref().is_some_and(|s| s.subpaths.len() > 1) {
            return None;
        }
        let written = match &spec.spine {
            None => BlendSpec { key_anchors: vec![], ..spec.clone() },
            Some(_) if spec.key_anchors.is_empty() => {
                let (path, anchors) = pin_spine(keys, spec)?;
                let anchors = anchors.into_iter().map(u32::try_from).collect::<Result<Vec<u32>, _>>().ok()?;
                BlendSpec { spine: Some(path), key_anchors: anchors, ..spec.clone() }
            }
            Some(_) => BlendSpec { spine: Some(blend_spine(keys, spec)?.0), ..spec.clone() },
        };
        // The record holds Specified Distance as a 32-bit float, which the reader reads back as its
        // shortest decimal (10 mm comes back as 28.346457 pt): the blend is compared with that
        // distance, and kept live only when it lays out as many steps with it.
        let written = match written.spacing {
            BlendSpacing::Distance(d) => {
                let read: f64 = (d as f32).to_string().parse().ok().filter(|r: &f64| r.is_finite() && *r > 0.0)?;
                let stored = BlendSpec { spacing: BlendSpacing::Distance(read), ..written.clone() };
                if blend_steps(keys, &stored) != blend_steps(keys, &written) {
                    return None;
                }
                stored
            }
            _ => written,
        };
        // As many objects as the reader lays out for one blend, twice within what its budget has left,
        // counted before the steps are drawn.
        let objects = blend_steps(keys, &written).into_iter().fold(keys.len(), usize::saturating_add);
        if objects > MAX_BLEND_OBJECTS || objects.saturating_mul(2) > self.blend_budget {
            return None;
        }
        // The reader counts the steps from the keys as written, rounded: a count that rounding can
        // change may come back other than drawn. Specified Distance counts from the spine's length,
        // and every segment of the spine adds to how far that may move.
        let segments = written.spine.as_ref().and_then(|s| s.subpaths.first()).map_or(0, |s| s.segment_count());
        let settle = match written.spacing {
            BlendSpacing::Distance(_) => SETTLE_LENGTH + segments as f64 * SETTLE_SEGMENT,
            _ => SETTLE_LENGTH,
        };
        if !blend_steps_settled(keys, &written, settle, SETTLE_COLOR) {
            return None;
        }
        let ids: Vec<String> = IDS.iter().take(keys.len()).map(|c| char::from(*c).to_string()).collect();
        let record = self.record(keys, &written)?;
        let steps: Vec<Arc<Node>> = vectorcraft_effects::expand_live_deep(Some(self.doc), n).children().cloned().unwrap_or_default();
        let drawn: Vec<Node> = steps.iter().map(|s| (**s).clone()).collect();
        // The spine part as written: the first subpath of the spine laid out.
        let part = written.spine.as_ref().and_then(|s| s.subpaths.first()).map(|sp| PathData::single(sp.clone()));
        let mut budget = self.blend_budget;
        let read = blend_reads(keys, &ids, part.as_ref(), &record, &drawn, &mut budget)?;
        if read != written {
            return None;
        }
        self.blend_budget = budget;
        Some(LiveBlend { keys: keys.clone(), ids, spine: part, record, steps })
    }

    /// The record of a blend of `keys` with the settings `spec` (its spine as written).
    fn record(&self, keys: &[Arc<Node>], spec: &BlendSpec) -> Option<Vec<u8>> {
        let (spine, anchors) = blend_spine(keys, spec)?;
        let steps = blend_steps(keys, spec);
        let first = u32::try_from(*steps.first()?).ok()?;
        let (spacing, value) = match spec.spacing {
            BlendSpacing::SmoothColor => (0, 4.0),
            BlendSpacing::Steps(n) => (1, spine.length() / (f64::from(n) + 1.0)),
            BlendSpacing::Distance(d) => (2, d),
        };
        let orientation = match spec.orientation {
            BlendOrientation::AlignToPage => 0.0,
            BlendOrientation::AlignToPath => 8.0 + self.starting_angle(&spine)?,
        };
        let count = u32::try_from(keys.len()).ok()?;
        let next = IDS.get(keys.len()).copied().map_or(0, u32::from);
        let mut words: Vec<u32> = vec![u32::from_le_bytes(*b"DCBA"), spacing, (value as f32).to_bits(), 0, first, (orientation as f32).to_bits()];
        words.extend([next, 0, 0, 1, count]);
        for (i, id) in IDS.iter().take(keys.len()).enumerate() {
            let anchor = anchors.as_ref().and_then(|a| a.get(i)).copied().unwrap_or(i);
            let before = i.checked_sub(1).and_then(|j| steps.get(j)).copied().unwrap_or(0);
            words.extend([u32::from(*id), 0, (anchor as f32).to_bits(), u32::MAX, u32::MAX, u32::try_from(before).ok()?]);
        }
        Some(words.iter().flat_map(|w| w.to_le_bytes()).collect())
    }

    /// The angle in art space (radians, from 0 to 2π) at which `spine` starts.
    fn starting_angle(&self, spine: &PathData) -> Option<f64> {
        let sp = spine.subpaths.first()?;
        let first = sp.anchors.first()?;
        let towards = if (first.h_out - first.p).hypot() > 1e-9 { first.h_out } else { sp.anchors.get(1)?.p };
        let (a, b) = (self.to_art * first.p, self.to_art * towards);
        let angle = (b.y - a.y).atan2(b.x - a.x);
        Some(if angle < 0.0 { angle + std::f64::consts::TAU } else { angle })
    }

    /// Blend `n`, of transparency `xy`, written live ([`Self::live_blend`]): its own art on `%_`
    /// lines, its steps and its record, then the state block of its transparency, before which the
    /// object's dictionary (its name, text wrap options and variables) goes. False, writing
    /// nothing, when it is written as its steps: the reader would read it otherwise, or art in it
    /// would be left out at the object limit ([`Self::take_objects`]).
    pub(super) fn blend(&mut self, n: &Node, xy: Xy) -> bool {
        let budget = self.blend_budget;
        let Some(b) = self.live_blend(n) else { return false };
        let start = self.body.len();
        let (state, gs, charges, bound, warnings) = (self.state, self.gs, self.charges(), self.bound.len(), self.warnings.len());
        let outer = std::mem::take(&mut self.body);
        self.hidden += 1;
        self.line("0 Ae");
        let opened = self.open_group("u");
        if opened {
            for (k, id) in b.keys.iter().zip(&b.ids) {
                self.blend_role = Some((k.id, format!("%_(K) /String (ShapeBlendPathType) ,\n%_({id}) /String (ShapeBlendPathID) ,\n")));
                self.node(k);
                self.blend_role = None;
            }
            if let Some(spine) = &b.spine {
                self.plain_object(spine, FillRule::NonZero, None, None, Xy::OPAQUE);
                self.body.push_str("%_/ArtDictionary :\n%_(S) /String (ShapeBlendPathType) ,\n%_;\n%_\n");
            }
            self.line("U");
            self.line("9 () XW");
        }
        self.hidden -= 1;
        let part = std::mem::replace(&mut self.body, outer);
        hide(&part, &mut self.body);
        // Readers of the printed form skip the hidden lines: the steps set their own state.
        (self.state, self.gs) = (None, None);
        self.state(n.locked, !n.visible);
        let steps = self.open_group("u");
        if steps {
            // The steps are drawn art: copies of the keys carry no text wrap options of their own.
            self.nest += 1;
            self.drawn += 1;
            for s in &b.steps {
                self.node(s);
            }
            self.drawn -= 1;
            self.nest -= 1;
            self.state(n.locked, !n.visible);
            self.line("U");
            self.line("9 () XW");
        }
        if !opened || !steps || self.left_out_since(charges) {
            let lines = self.body.split_off(start);
            self.give_back(charges, &lines);
            self.bound.truncate(bound);
            // The warnings of art written as its steps again are those steps'.
            if !self.left_out_since(charges) {
                self.warnings.truncate(warnings);
            }
            (self.state, self.gs, self.block_end, self.blend_budget) = (state, gs, usize::MAX, budget);
            return false;
        }
        self.line("0 Ae");
        let hex: String = b.record.iter().map(|x| format!("{x:02X}")).collect();
        let _ = writeln!(self.body, "({BLEND_PLUGIN}) 1 0 {} XP", b.record.len());
        for chunk in hex.as_bytes().chunks(HEX_LINE) {
            self.body.push('%');
            self.body.push_str(&String::from_utf8_lossy(chunk));
            self.body.push('\n');
        }
        // The blend's dictionary goes after the record and before this block ([`Self::node`]).
        self.state_block(xy);
        true
    }
}

/// The blends on `doc`'s layers that baking leaves as they are: blends with nothing in them that
/// [`vectorcraft_effects::needs_bake`] evaluates (geometry and Pathfinder effects, a group's own
/// fills and strokes), inside no object that has such things of its own. Baking changes the keys
/// of the others, so they are written as their steps.
pub(in crate::ai_editing) fn unbaked_blends(doc: &Document) -> HashSet<NodeId> {
    fn walk(nodes: &[Arc<Node>], out: &mut HashSet<NodeId>) {
        for n in nodes {
            if matches!(n.kind, NodeKind::Blend { .. }) {
                if !vectorcraft_effects::needs_bake(n) {
                    out.insert(n.id);
                }
                continue;
            }
            let Some(children) = n.children() else { continue };
            let mut own = (**n).clone();
            if let Some(c) = own.children_mut() {
                c.clear();
            }
            if !vectorcraft_effects::needs_bake(&own) {
                walk(children, out);
            }
        }
    }
    let mut out = HashSet::new();
    walk(&doc.layers, &mut out);
    out
}

/// Does the reader read stroke `s`'s dashes back as they are (`read/ops.rs`' `d`): at most 64
/// lengths, each finite and not negative, and a finite offset? It reads other dashes as a solid
/// line.
fn dash_read_back(s: &StrokeLayer) -> bool {
    s.dash
        .as_ref()
        .is_none_or(|d| d.pattern.len() <= MAX_DASH_LENGTHS && d.pattern.iter().all(|v| v.is_finite() && *v >= 0.0) && d.offset.is_finite())
}

/// Do the keys hold a fill or stroke that the editing copy leaves out (a fill of no paint; a stroke
/// of no paint or of no width, as `visible_items` leaves them out) where another holds one of the
/// same kind that it writes? The steps blend the width, paint and settings of the one left out, and
/// the keys read back blend from no fill or stroke, so the steps would open otherwise.
fn left_out_against_drawn(keys: &[Arc<Node>]) -> bool {
    // [fill, stroke]: is one left out somewhere, is one written somewhere.
    let mut left_out = [false; 2];
    let mut drawn = [false; 2];
    for k in keys {
        k.walk(&mut |n| {
            for i in &n.appearance.items {
                let (kind, out) = match i {
                    AppearanceItem::Fill(f) => (0, f.paint.is_none()),
                    AppearanceItem::Stroke(s) => (1, s.paint.is_none() || !(s.width > 0.0 && s.width.is_finite())),
                };
                if out {
                    left_out[kind] = true;
                } else {
                    drawn[kind] = true;
                }
            }
        });
    }
    (left_out[0] && drawn[0]) || (left_out[1] && drawn[1])
}

/// Can `k` be a key object written live, as the reader reads it back: paths, compound paths and
/// groups other than clipping groups (holding no blend), without effects or opacity masks, each with
/// at most one fill and one plain stroke over it (a line the data draws as it is), each fill and
/// stroke shown, in a color other than Lab or none, opaque in the normal blending mode? (A group's own fills and
/// strokes are baked into art of their own, and such blends aren't [`unbaked_blends`].)
fn plain_key(k: &Node) -> bool {
    let mut plain = true;
    k.walk(&mut |n| {
        // A clipping group may be written as its art cut to the clipping object.
        let kind = match &n.kind {
            // The writer leaves out a path with nothing to draw and subpaths without anchors, and
            // writes a path of several subpaths as a compound path.
            NodeKind::Path { path, guide: false, .. } => matches!(path.subpaths.as_slice(), [one] if !one.anchors.is_empty()),
            // A compound path is written as one path, and read back with a member for each
            // subpath.
            NodeKind::Compound { children, .. } => {
                !children.is_empty()
                    && children.iter().all(|c| matches!(&c.kind, NodeKind::Path { path, guide: false, .. } if path.subpaths.len() == 1))
            }
            NodeKind::Group { clip: false, .. } => true,
            _ => false,
        };
        let items = &n.appearance.items;
        let fills = items.iter().filter(|i| matches!(i, AppearanceItem::Fill(_))).count();
        let strokes = items.iter().filter(|i| matches!(i, AppearanceItem::Stroke(_))).count();
        let order = !matches!(items.as_slice(), [AppearanceItem::Stroke(_), AppearanceItem::Fill(_)]);
        let quiet = items.iter().all(|i| {
            i.visible()
                && matches!(i.paint(), Paint::Solid { .. } | Paint::None)
                // A color that names no global or spot swatch is written in RGB, so a Lab color
                // would come back as another.
                && !matches!(i.paint(), Paint::Solid { color: Color::Lab { .. }, .. })
                && i.effects().is_empty()
                && i.opacity() >= 1.0
                && i.blend() == BlendMode::Normal
                && !matches!(i, AppearanceItem::Stroke(s) if !vectorcraft_effects::stroke::is_plain(s) || !dash_read_back(s))
        });
        plain &= kind && fills <= 1 && strokes <= 1 && order && quiet && n.appearance.effects.is_empty() && n.extras().mask.is_none();
    });
    plain
}
