//! Type › Retype (#4934), phase 1: outlined text whose glyph shapes are glyphs of a font the app
//! has becomes live point text again, in that font and size, at the same baseline.
//!
//! Behaviour follows the public description of Illustrator's Retype
//! (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/identify-fonts-using-retype.html>,
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/apply-retype-suggested-fonts-to-live-text.html>):
//! select outlined text and Retype finds the fonts that match it, from the fonts on the device among
//! others, for use as live text. Illustrator ranks candidate fonts in a panel; this phase is local
//! and exact instead, and sets the text itself, as #4934 asks. Each selected glyph shape (a path or compound
//! path, inside groups too) is compared with the glyphs of the candidate fonts
//! ([`vectorcraft_text::retype`]), the same up to uniform scale, rotation and translation. Matched
//! glyphs map back to characters through the font's cmap and are rebuilt as point text, one object
//! per line of glyphs, with the first glyph's fill and stroke. Shapes that match no font stay as
//! they are and are reported.
//!
//! With a `tolerance`, a shape that matches no glyph exactly may still nearly match one: another
//! version of the font, or the outline simplified or built another way
//! ([`RetypeIndex::near_matches`]); each line reports how closely its glyphs fit. A compound path
//! holding several glyphs (a word outlined as one path) is split into its glyphs
//! ([`vectorcraft_text::retype::clusters`]), and becomes type only when every one of them does.
//! The tolerance, the score and the splitting rules are VectorCraft's own. Images are phase 2.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use kurbo::{BezPath, Point, Vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::FillRule;
use vectorcraft_text::FontDb;
use vectorcraft_text::retype::{GlyphMatch, RetypeIndex};

use super::edit::selected_roots;
use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_selected_path};

/// At most this many shapes are compared.
const MAX_PIECES: usize = 2048;
/// Groups deeper than this are not looked into.
const MAX_DEPTH: usize = 16;
/// Extra families named by `fonts`.
const MAX_FONT_PARAMS: usize = 32;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "type.retype",
        "Retype",
        ["Type"],
        None,
        "{fonts?: [family…], tolerance?: 0..0.05 (default 0: exact matches only)} turn the selected outlined text (glyph paths and compound paths, inside groups too) back into live point text: each shape is matched against the glyphs of the fonts the app has loaded, the document's fonts and the families in fonts, the same up to uniform scale, rotation and translation; with a tolerance above 0, a shape that is no glyph exactly may nearly be one (an outline simplified, a curve split in two, contours in another order, start or direction) when its leftover distance from the glyph, relative to the shape's outline spread (the root mean square distance of its sampled outline points from their center), is at most the tolerance (VectorCraft's own measure; shapes of at most 8 contours); a compound path holding several glyphs (a word outlined as one path, at most 256 contour groups and 512 contours, and 16,384 contours per command) is split into them, and becomes type only when every one of them matches; matched glyphs become point text in that font and size at the same baseline, one object per line, with the first glyph's fill, stroke, opacity and blend mode, in one undo step; a line splits where its type would not draw the outlines' glyphs in their places (tracking, manual spacing, font substitutions; with a tolerance, within it); a glyph must also paint the area the shape paints under its fill rule (a nonzero compound whose inner contour runs the same way as its outer one paints its center, so a ring glyph does not replace it); shapes that match no font, glyphs type cannot draw as they are, and shapes with their own opacity mask stay as they are → {ids (the new type), texts, fonts: [{family, style, size, score (how closely the line's least close glyph fits, 1 for exact, 0 at a leftover distance of 0.05)}], unmatched: [id…] (shapes that stayed as they are when another shape became type), truncated (shapes past the limits of 2048 shapes or 16 nested groups were left out)}; an error, with no unmatched list, when no shape matches, or when tolerance is not a number from 0 to 0.05",
        has_selected_path,
        retype
    )]
}

#[path = "retype_fonts.rs"]
pub(super) mod fonts;

/// A shape to match: a path or compound path, in document space.
struct Piece {
    id: NodeId,
    /// The group or layer holding it: glyphs of different groups, which may composite
    /// differently, never share a line.
    parent: Option<NodeId>,
    /// Other art (not retyped) lies between it and the shape before it, so the type after that
    /// art must stay painted after it: the line ends here.
    after_barrier: bool,
    path: BezPath,
    /// The shape is filled by the nonzero rule, not the even-odd rule.
    nonzero: bool,
    /// The shape has a visible fill: a shape that only has a stroke paints no area to compare.
    filled: bool,
    appearance: Appearance,
    /// The shape's own transparency, which the new type takes.
    opacity: f32,
    blend: vectorcraft_color::BlendMode,
    /// The shape has what type cannot take (its own opacity mask, more than one fill or stroke,
    /// effects, a brush, width profile or arrowheads): it stays an outline.
    keeps_outline: bool,
}

/// The shapes in `n` (held by `parent`), in paint order. `barrier` is set when art that is not a
/// shape lies before the next one; it starts set for a root that does not follow the last root.
/// `truncated` is set when shapes past [`MAX_PIECES`] or [`MAX_DEPTH`] were left out.
fn collect(n: &Node, parent: Option<NodeId>, depth: usize, barrier: &mut bool, truncated: &mut bool, out: &mut Vec<Piece>) {
    if out.len() >= MAX_PIECES || depth > MAX_DEPTH {
        *barrier = true;
        *truncated = true;
        return;
    }
    // Hidden and locked art is left as it is: a hidden glyph must not come back as visible type.
    if !n.visible || n.locked {
        *barrier = true;
        return;
    }
    let piece = |path: BezPath, after_barrier: bool, rule: FillRule| Piece {
        id: n.id,
        parent,
        after_barrier,
        path,
        nonzero: rule == FillRule::NonZero,
        filled: n.appearance.fill().is_some_and(|f| f.visible && !f.paint.is_none()),
        appearance: (*n.appearance).clone(),
        opacity: n.opacity,
        blend: n.blend,
        keeps_outline: n.extras().mask.is_some() || !type_can_paint(&n.appearance),
    };
    match &n.kind {
        NodeKind::Group { children, clip: false } => {
            for c in children {
                collect(c, Some(n.id), depth + 1, barrier, truncated, out);
            }
        }
        // Only closed shapes can be glyphs: an open path stroked along a glyph's outline is not one.
        NodeKind::Path { path, rule, clipping: false, guide: false, .. } if path.is_closed() => {
            out.push(piece(path.to_bezpath(), std::mem::take(barrier), *rule))
        }
        NodeKind::Compound { children, rule } => {
            let mut path = BezPath::new();
            for c in children {
                match &c.kind {
                    // A hidden or locked path keeps its whole compound as it is.
                    NodeKind::Path { path: p, .. } if p.is_closed() && c.visible && !c.locked => path.extend(p.to_bezpath()),
                    _ => {
                        *barrier = true;
                        return;
                    }
                }
            }
            out.push(piece(path, std::mem::take(barrier), *rule));
        }
        // Live type, images, clip groups and the rest stay, and the type after them stays after them.
        _ => *barrier = true,
    }
}

/// Can type's character style paint `ap` as it is: a basic appearance (at most one fill and one
/// stroke, no effects, no item opacity or blend), with a plain stroke?
fn type_can_paint(ap: &Appearance) -> bool {
    ap.is_basic() && ap.stroke().is_none_or(|s| s.start_arrow.is_none() && s.end_arrow.is_none() && s.profile.is_none() && s.brush.is_none())
}

/// The faces to match against: the families in `fonts`, the document's fonts, then every loaded face.
fn candidate_faces(db: &FontDb, d: &Document, fonts: &[String]) -> Vec<Arc<vectorcraft_text::FontFace>> {
    let mut families: Vec<String> = fonts.iter().take(MAX_FONT_PARAMS).cloned().collect();
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            for r in &t.runs {
                if !families.contains(&r.style.font_family) && families.len() < vectorcraft_text::retype::MAX_FACES {
                    families.push(r.style.font_family.clone());
                }
            }
        }
    });
    let cap = vectorcraft_text::retype::MAX_FACES;
    let mut faces = vec![];
    'families: for f in &families {
        for style in db.styles(f) {
            // No more faces are loaded than the index takes.
            if faces.len() >= cap {
                break 'families;
            }
            if let Some(face) = db.face(f, &style)
                && !faces.iter().any(|k: &Arc<vectorcraft_text::FontFace>| k.id() == face.id())
            {
                faces.push(face);
            }
        }
    }
    for face in db.loaded_faces() {
        if faces.len() >= cap {
            break;
        }
        if !faces.iter().any(|k| k.id() == face.id()) {
            faces.push(face);
        }
    }
    faces
}

/// One line of retyped glyphs.
struct Line {
    pieces: Vec<NodeId>,
    /// What each piece matched, as `pieces`.
    matches: Vec<GlyphMatch>,
    /// Every glyph the first piece fits, while the line holds only it: the next glyph may tell
    /// which one it is (see [`join`]).
    first_cands: Vec<GlyphMatch>,
    text: String,
    face: Arc<vectorcraft_text::FontFace>,
    scale: f64,
    angle: f64,
    origin: Point,
    end: Point,
    appearance: Appearance,
    parent: Option<NodeId>,
    opacity: f32,
    blend: vectorcraft_color::BlendMode,
    /// The tolerance near matches were taken with (0: exact only): a near-matched glyph may differ
    /// that much in size, turn and place from the rest of its line ([`Self::slack_for`]).
    slack: f64,
}

impl Line {
    fn dir(&self) -> Vec2 {
        Vec2::new(self.angle.cos(), self.angle.sin())
    }

    /// The slack glyph `m` of this line is allowed: [`Self::slack`] when `m` or the line's first
    /// glyph, whose size and turn the line takes, is a near match, else 0.
    fn slack_for(&self, m: &GlyphMatch) -> f64 {
        if m.exact && self.matches.first().is_none_or(|f| f.exact) { 0.0 } else { self.slack }
    }

    /// Does glyph `m` of shape `pc` continue this line (same group, paint and transparency, same
    /// font, size and turn, on its baseline, after its end)?
    fn continues(&self, pc: &Piece, m: &GlyphMatch) -> bool {
        let size = self.scale * self.face.units_per_em();
        let d = m.origin - self.origin;
        let dir = self.dir();
        let across = d.cross(dir).abs();
        let slack = self.slack_for(m);
        pc.parent == self.parent
            && pc.appearance == self.appearance
            && pc.opacity == self.opacity
            && pc.blend == self.blend
            && m.face.id() == self.face.id()
            // One size and turn for the whole line: the type sets every glyph at the first one's.
            && (m.scale - self.scale).abs() <= self.scale * slack.max(1e-4)
            // Wrapped: type turned 180° has angles near +π and −π.
            && ((m.angle - self.angle + PI).rem_euclid(TAU) - PI).abs() < slack.max(1e-5)
            && across < size * 0.05
            && (m.origin - self.end).dot(dir) > -size * 0.25
    }

    fn push(&mut self, id: NodeId, m: &GlyphMatch) {
        // A gap wider than half a space before the glyph is spaces the outlines left no shape for.
        let dir = self.dir();
        let gap = (m.origin - self.end).dot(dir);
        let space = space_advance(&self.face) * self.scale;
        if space > 0.0 && gap > space * 0.5 {
            let n = ((gap / space).round() as usize).clamp(1, 64);
            self.text.extend(std::iter::repeat_n(' ', n));
        }
        self.text.push(m.ch);
        self.pieces.push(id);
        self.matches.push(m.clone());
        self.end = m.origin + dir * (m.face.advance(m.gid) * m.scale);
    }

    /// The line of this one's glyphs `from..to`, with its group, paint and transparency.
    fn part(&self, from: usize, to: usize) -> Option<Line> {
        let (ids, ms) = (self.pieces.get(from..to)?, self.matches.get(from..to)?);
        let (first, m) = (ids.first()?, ms.first()?);
        let dir = Vec2::new(m.angle.cos(), m.angle.sin());
        let mut l = Line {
            pieces: vec![*first],
            matches: vec![m.clone()],
            first_cands: vec![],
            text: m.ch.to_string(),
            face: m.face.clone(),
            scale: m.scale,
            angle: m.angle,
            origin: m.origin,
            end: m.origin + dir * (m.face.advance(m.gid) * m.scale),
            appearance: self.appearance.clone(),
            parent: self.parent,
            opacity: self.opacity,
            blend: self.blend,
            slack: self.slack,
        };
        for (id, m) in ids.iter().zip(ms).skip(1) {
            l.push(*id, m);
        }
        Some(l)
    }

    /// The point type that sets this line.
    fn text_object(&self, system_layout: bool) -> TextObject {
        let style = CharStyle {
            font_family: self.face.family.clone(),
            font_style: self.face.style.clone(),
            // The version that matched, when two of the family and style are loaded.
            font_version: (!self.face.version.is_empty()).then(|| self.face.version.clone()),
            // Each outlined glyph was drawn on its own; a ligature would draw another shape.
            features: vec!["-liga".into()],
            size: self.scale * self.face.units_per_em(),
            fill: self.appearance.fill_paint(),
            stroke: self.appearance.stroke_paint(),
            stroke_width: self.appearance.stroke_width(),
            ..Default::default()
        };
        let mut style = style;
        if let Some(st) = self.appearance.stroke() {
            style.stroke_cap = st.cap;
            style.stroke_join = st.join;
            style.stroke_miter_limit = st.miter_limit;
            style.stroke_dash = st.dash.clone();
            style.stroke_align = st.align;
        }
        let mut t = TextObject::point(self.origin, &self.text, style);
        if self.angle != 0.0 {
            t.xf *= kurbo::Affine::rotate(self.angle);
        }
        t.para = super::create::new_type_para();
        t.system_layout = system_layout;
        super::typecmd::refresh_bounds(&mut t);
        t
    }

    /// The first of this line's glyphs its type does not draw as the outlines did: another glyph
    /// (a substitution the font makes) or the same glyph elsewhere (tracking, manual spacing,
    /// more space than whole spaces make); `None` when the type draws every glyph in its place.
    fn first_mismatch(&self, t: &TextObject) -> Option<usize> {
        let lay = vectorcraft_text::layout(FontDb::global(), t);
        let mut drawn = lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty());
        for (k, m) in self.matches.iter().enumerate() {
            let Some(g) = drawn.next() else { return Some(k) };
            let tol = (self.scale * self.face.units_per_em() * (0.01 + self.slack_for(m))).max(0.01);
            let at = t.xf * (g.xf * Point::ORIGIN);
            if g.font_id != m.face.id() || g.gid != m.gid || at.distance(m.origin) > tol {
                return Some(k);
            }
        }
        // Ink the outlines did not have: the last glyph is where it went wrong.
        drawn.next().map(|_| self.matches.len().saturating_sub(1))
    }
}

/// `lines` checked against their own layout: a line splits where its type stops drawing the
/// outlines, and a glyph its type cannot draw even on its own stays an outline (`unmatched`).
/// The glyphs laid out are bounded by a multiple of the number of glyphs.
fn verified(lines: Vec<Line>, system_layout: bool, unmatched: &mut Vec<NodeId>) -> Vec<Line> {
    // Glyphs laid out in all, at most: past it, the glyphs left unchecked stay outlines.
    let mut budget = lines.iter().map(|l| l.pieces.len()).sum::<usize>().saturating_mul(16).saturating_add(256);
    let mut todo: std::collections::VecDeque<Line> = lines.into();
    let mut out = vec![];
    while let Some(l) = todo.pop_front() {
        let n = l.pieces.len();
        // The line's type is laid out over a window from its start that doubles while the type
        // draws the window's glyphs as the outlines do, so finding where it stops costs about
        // twice the glyphs before that point, not the whole rest of the line each time.
        let mut w = 8usize;
        loop {
            let e = n.min(w);
            let Some(part) = l.part(0, e) else { break };
            let Some(left) = budget.checked_sub(e) else {
                // Unchecked glyphs stay outlines.
                unmatched.extend(l.pieces.iter().copied());
                break;
            };
            budget = left;
            match part.first_mismatch(&part.text_object(system_layout)) {
                // The whole line is drawn as it was: one object.
                None if e == n => {
                    out.push(part);
                    break;
                }
                None => w = w.saturating_mul(2),
                // The first glyph on its own may still be drawn as it was (a contextual form
                // changed it in the line); else it stays an outline. Checked again either way.
                Some(0) if n > 1 => {
                    todo.extend_front_parts(&l, [(0, 1), (1, n)]);
                    break;
                }
                Some(0) => {
                    unmatched.extend(l.pieces.first().copied());
                    break;
                }
                // The glyphs before the mismatch, then the rest, each checked as a line of its
                // own: the type before the mismatch may draw otherwise without what follows it (a
                // font can place a glyph by the glyph after it).
                Some(k) => {
                    todo.extend_front_parts(&l, [(0, k), (k, n)]);
                    break;
                }
            }
        }
    }
    out
}

/// Puts parts of a line at the front of the work list, in order.
trait FrontParts {
    fn extend_front_parts<const N: usize>(&mut self, l: &Line, ranges: [(usize, usize); N]);
}

impl FrontParts for std::collections::VecDeque<Line> {
    fn extend_front_parts<const N: usize>(&mut self, l: &Line, ranges: [(usize, usize); N]) {
        for (a, b) in ranges.into_iter().rev() {
            if let Some(p) = l.part(a, b) {
                self.push_front(p);
            }
        }
    }
}

fn space_advance(face: &vectorcraft_text::FontFace) -> f64 {
    face.mapped_chars().iter().find(|(c, _)| *c == ' ').map(|(_, g)| face.advance(*g)).unwrap_or(0.0)
}

/// Add shape `pc` to line `l` when one of its candidate glyphs `cands`, in one of its poses,
/// continues it (one shape can be several glyphs, and a glyph that looks the same turned fits in
/// more than one pose). A line of one shape may turn to another of that shape's candidates to take
/// the next one: an L or a turned L half round, an I upright or upside down, and the glyph after it
/// tells which. Whether it was added.
fn join(l: &mut Line, pc: &Piece, cands: &[GlyphMatch]) -> bool {
    let fits = |l: &Line| cands.iter().flat_map(|c| c.poses()).find(|mp| l.continues(pc, mp));
    if let Some(mp) = fits(l) {
        l.push(pc.id, &mp);
        return true;
    }
    let (Some(id), [current]) = (l.pieces.first().copied(), l.matches.as_slice()) else { return false };
    let current = current.clone();
    let others: Vec<GlyphMatch> = l
        .first_cands
        .iter()
        .flat_map(|c| c.poses())
        .filter(|fp| !(fp.gid == current.gid && fp.face.id() == current.face.id() && fp.angle == current.angle))
        .collect();
    for fp in others {
        let mut turned = reposed(l, id, &fp);
        if let Some(mp) = fits(&turned) {
            turned.push(pc.id, &mp);
            *l = turned;
            return true;
        }
    }
    false
}

/// Line `l`'s group, paint and transparency with glyph `m` (shape `id`) alone on it.
fn reposed(l: &Line, id: NodeId, m: &GlyphMatch) -> Line {
    let dir = Vec2::new(m.angle.cos(), m.angle.sin());
    Line {
        pieces: vec![id],
        matches: vec![m.clone()],
        first_cands: l.first_cands.clone(),
        text: m.ch.to_string(),
        face: m.face.clone(),
        scale: m.scale,
        angle: m.angle,
        origin: m.origin,
        end: m.origin + dir * (m.face.advance(m.gid) * m.scale),
        appearance: l.appearance.clone(),
        parent: l.parent,
        opacity: l.opacity,
        blend: l.blend,
        slack: l.slack,
    }
}

/// A new line of shape `pc` alone, as the first of its candidate glyphs `cands` (none: `None`),
/// with near matches taken at tolerance `slack`.
fn new_line(pc: &Piece, cands: &[GlyphMatch], slack: f64) -> Option<Line> {
    let m = cands.first()?;
    let dir = Vec2::new(m.angle.cos(), m.angle.sin());
    Some(Line {
        pieces: vec![pc.id],
        matches: vec![m.clone()],
        first_cands: cands.to_vec(),
        text: m.ch.to_string(),
        face: m.face.clone(),
        scale: m.scale,
        angle: m.angle,
        origin: m.origin,
        end: m.origin + dir * (m.face.advance(m.gid) * m.scale),
        appearance: pc.appearance.clone(),
        parent: pc.parent,
        opacity: pc.opacity,
        blend: pc.blend,
        slack,
    })
}

/// The lines, by index, to drop so that no shape of several glyphs becomes type in part: each line
/// is given by the shapes it sets, one per glyph, and `glyphs_of` says how many glyphs a shape
/// is. A line setting a glyph of a shape not all of whose glyphs are set goes, and then so do the
/// lines setting the other glyphs of the shapes it held.
fn partial_lines(lines: &[Vec<NodeId>], glyphs_of: &std::collections::HashMap<NodeId, usize>) -> std::collections::HashSet<usize> {
    let mut dropped = std::collections::HashSet::new();
    loop {
        let mut set: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
        for (_, l) in lines.iter().enumerate().filter(|(i, _)| !dropped.contains(i)) {
            for id in l {
                *set.entry(*id).or_default() += 1;
            }
        }
        let partial: std::collections::HashSet<NodeId> =
            set.iter().filter(|(id, n)| glyphs_of.get(*id).is_some_and(|want| *n < want)).map(|(id, _)| *id).collect();
        let before = dropped.len();
        for (i, l) in lines.iter().enumerate() {
            if l.iter().any(|id| partial.contains(id)) {
                dropped.insert(i);
            }
        }
        if dropped.len() == before {
            return dropped;
        }
    }
}

/// Glyph runs longer than this (in contour groups) are not tried when a shape is split.
const MAX_RUN: usize = 3;

/// A shape of more contour groups than this is not split: it is matched whole only.
const MAX_SPLIT_GROUPS: usize = 256;

/// A shape of more contours than this is not split either, and its contours are not grouped.
const MAX_SPLIT_CONTOURS: usize = 512;

/// Contours of all the selected shapes one command groups, at most: the shapes past them are
/// matched whole only.
const MAX_SPLIT_TOTAL_CONTOURS: usize = 16_384;

/// `path`'s contour groups when it is split into glyphs: none for fewer than two or more than
/// [`MAX_SPLIT_GROUPS`], for more than [`MAX_SPLIT_CONTOURS`] contours, and for more contours than
/// `budget` holds. A path of two or more contours takes them from `budget` when it is grouped.
fn split_groups(path: &BezPath, budget: &mut usize) -> Vec<BezPath> {
    // Grouping compares every contour with every other: count them first.
    let n = path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
    if !(2..=MAX_SPLIT_CONTOURS).contains(&n) || n > *budget {
        return vec![];
    }
    *budget -= n;
    let groups = vectorcraft_text::retype::clusters(path);
    if (2..=MAX_SPLIT_GROUPS).contains(&groups.len()) { groups } else { vec![] }
}

/// The parts of shape `path` to match: the whole shape, or, when it is no glyph and holds several
/// contour `groups` ([`split_groups`]), runs of up to [`MAX_RUN`] groups in order, the longest that matches first (an i before a dotless i and a dot) among the runs after
/// which the rest of the groups can still be matched. Each part with its candidates; empty when
/// the shape matches nothing and its groups cannot all be matched. `find(path, part, near)` gives
/// a path's candidates: its `part` flag is set for a part of a split shape, whose contours Make
/// Compound Path may have turned the other way round, and its `near` flag allows near matches.
/// Exact matches come first: the whole shape, then its groups, then the whole shape nearly, then
/// its groups nearly.
fn parts<M>(path: &BezPath, groups: &[BezPath], find: &dyn Fn(&BezPath, bool, bool) -> Vec<M>) -> Vec<Vec<M>> {
    let whole = find(path, false, false);
    if !whole.is_empty() {
        return vec![whole];
    }
    let exact = split_parts(groups, &|run| find(run, true, false));
    if !exact.is_empty() {
        return exact;
    }
    let whole = find(path, false, true);
    if !whole.is_empty() {
        return vec![whole];
    }
    split_parts(groups, &|run| find(run, true, true))
}

/// [`parts`] of a shape split into contour `groups`, each run's candidates from `find`; empty for
/// no groups and when the groups cannot all be matched.
fn split_parts<M>(groups: &[BezPath], find: &dyn Fn(&BezPath) -> Vec<M>) -> Vec<Vec<M>> {
    if groups.is_empty() {
        return vec![];
    }
    let g = groups.len();
    // From the last group back: the run that starts at each group, the longest that matches and
    // leaves a rest that can all be matched, with its candidates. Runs past such a start are
    // never tried for one whose rest cannot be matched.
    let mut chosen: Vec<Option<(usize, Vec<M>)>> = (0..g).map(|_| None).collect();
    for i in (0..g).rev() {
        for n in (1..=MAX_RUN.min(g - i)).rev() {
            let rest_ok = i + n == g || chosen.get(i + n).is_some_and(Option::is_some);
            if !rest_ok {
                continue;
            }
            let Some(run) = groups.get(i..i + n) else { continue };
            let cands = find(&joined(run));
            if !cands.is_empty() {
                if let Some(c) = chosen.get_mut(i) {
                    *c = Some((n, cands));
                }
                break;
            }
        }
    }
    let mut out = vec![];
    let mut i = 0;
    while i < g {
        let Some(Some((n, cands))) = chosen.get_mut(i).map(Option::take) else { return vec![] };
        out.push(cands);
        i += n;
    }
    out
}

/// Contour groups joined into one path.
fn joined(run: &[BezPath]) -> BezPath {
    let mut p = BezPath::new();
    for g in run {
        p.extend(g.iter());
    }
    p
}

/// Every run of contour `groups` ([`split_groups`]) [`parts`] may try.
fn runs(groups: &[BezPath]) -> Vec<BezPath> {
    let mut out = vec![];
    for i in 0..groups.len() {
        for n in 1..=MAX_RUN.min(groups.len() - i) {
            if let Some(run) = groups.get(i..i + n) {
                out.push(joined(run));
            }
        }
    }
    out
}

/// The keys the index looks glyphs up by for the `shapes` (each with its contour groups) and
/// every run of their groups: their exact [`shape_key`](vectorcraft_text::retype::shape_key)s and,
/// with a `tolerance` above 0, at most [`MAX_NEAR_KEYS`](vectorcraft_text::retype::MAX_NEAR_KEYS)
/// distinct [`near_key`](vectorcraft_text::retype::near_key)s: every whole shape's first, in the
/// order the shapes come, then the runs of their groups. Paths past that many near keys are not
/// looked up for near matches.
fn lookup_keys<'a>(
    shapes: impl IntoIterator<Item = (&'a BezPath, &'a [BezPath])>,
    tolerance: f64,
) -> (std::collections::BTreeSet<u64>, Vec<vectorcraft_text::retype::NearKey>) {
    use vectorcraft_text::retype::{MAX_NEAR_KEYS, near_key, shape_key};
    let mut wanted = std::collections::BTreeSet::new();
    let mut near_wanted = vec![];
    let mut add = |run: &BezPath| {
        wanted.extend(shape_key(run));
        if tolerance > 0.0
            && near_wanted.len() < MAX_NEAR_KEYS
            && let Some(k) = near_key(run)
            && !near_wanted.contains(&k)
        {
            near_wanted.push(k);
        }
    };
    let shapes: Vec<(&BezPath, &[BezPath])> = shapes.into_iter().collect();
    // The whole shapes first, then the runs of their groups.
    for (path, _) in &shapes {
        add(path);
    }
    for (_, groups) in &shapes {
        for run in runs(groups) {
            add(&run);
        }
    }
    (wanted, near_wanted)
}

fn retype(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.retype";
    let fonts: Vec<String> = match p.get("fonts") {
        None => vec![],
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(|| bad(C, "fonts must be a list of family names"))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(C, "fonts must be a list of family names")),
    };
    let tolerance = match p.get("tolerance") {
        None => 0.0,
        Some(v) => match v.as_f64() {
            Some(t) if (0.0..=vectorcraft_text::retype::MAX_TOLERANCE).contains(&t) => t,
            _ => return Err(bad(C, "tolerance must be a number from 0 to 0.05")),
        },
    };
    let roots = selected_roots(s)?;
    let db = FontDb::global();
    let system_layout = s.doc()?.doc.setup.system_layout;
    let (lines, unmatched, truncated) = {
        let d = &s.doc()?.doc;
        let mut pieces = vec![];
        let mut truncated = false;
        let mut last: Option<(Option<NodeId>, usize)> = None;
        // Carried across roots: a selected root that yields no shape (live type, an open path) still
        // ends the line.
        let mut barrier = true;
        for r in &roots {
            let Some(n) = d.node(*r) else { continue };
            let parent = d.parent_of(*r);
            let idx = d.position(*r).map(|(_, i, _)| i);
            // A root right after the last one, in the same parent, continues its lines; unselected
            // art between two roots is a barrier.
            if !matches!((last, idx), (Some((p, i)), Some(j)) if p == parent && i + 1 == j) {
                barrier = true;
            }
            last = idx.map(|i| (parent, i));
            collect(n, parent, 0, &mut barrier, &mut truncated, &mut pieces);
        }
        if pieces.is_empty() {
            return Err(EngineError::Other("Retype: select outlined text (paths or compound paths)".into()));
        }
        // Only glyphs with a selected shape's key (segments per contour) can match one exactly, and
        // only glyphs with as many contours and a close area-to-length ratio nearly: the others are
        // not sampled. A shape of several glyphs is looked up by the runs of its groups too.
        let mut budget = MAX_SPLIT_TOTAL_CONTOURS;
        let groups: Vec<Vec<BezPath>> = pieces.iter().map(|pc| if pc.keeps_outline { vec![] } else { split_groups(&pc.path, &mut budget) }).collect();
        let (wanted, near_wanted) =
            lookup_keys(pieces.iter().zip(&groups).filter(|(pc, _)| !pc.keeps_outline).map(|(pc, g)| (&pc.path, g.as_slice())), tolerance);
        // Nothing to match (every shape stays an outline): no font is read.
        let index = if wanted.is_empty() && near_wanted.is_empty() {
            RetypeIndex::default()
        } else {
            RetypeIndex::build_near(db, candidate_faces(db, d, &fonts), &wanted, &near_wanted)
        };
        // Exact matches first; near ones, when allowed, only for a path with none.
        // With a visible fill, a glyph is kept only when it paints the area the shape paints under
        // the shape's fill rule (`fill` is `Some(nonzero)`).
        let find = |path: &BezPath, part: bool, near: bool, fill: Option<bool>| {
            let painted = |ms: Vec<GlyphMatch>, tol: f64| -> Vec<GlyphMatch> {
                match fill {
                    Some(nonzero) => ms.iter().filter_map(|m| index.painted_agrees(m, path, nonzero, tol)).collect(),
                    None => ms,
                }
            };
            let exact = painted(if part { index.matches_either_way(path) } else { index.all_matches(path) }, 0.0);
            if exact.is_empty() && near { painted(index.near_matches(path, tolerance), tolerance) } else { exact }
        };
        // Each shape's parts (one, unless it holds several glyphs), each with its candidates.
        let found: Vec<Vec<Vec<GlyphMatch>>> =
            pieces
                .iter()
                .zip(&groups)
                .map(|(pc, g)| {
                    if pc.keeps_outline { vec![] } else { parts(&pc.path, g, &|p, part, near| find(p, part, near, pc.filled.then_some(pc.nonzero))) }
                })
                .collect();
        // The face most shapes match, so a glyph several fonts share is set in the same font as
        // the glyphs around it.
        let mut votes: Vec<(u32, usize)> = vec![];
        for ms in found.iter().flatten() {
            // Each shape votes once for each face it fits.
            let mut faces: Vec<u32> = ms.iter().map(|m| m.face.id()).collect();
            faces.sort_unstable();
            faces.dedup();
            for f in faces {
                match votes.iter_mut().find(|(v, _)| *v == f) {
                    Some(v) => v.1 += 1,
                    None => votes.push((f, 1)),
                }
            }
        }
        let rank = |face: u32| votes.iter().find(|(f, _)| *f == face).map_or(0, |v| v.1);
        let mut lines: Vec<Line> = vec![];
        let mut unmatched = vec![];
        // An unmatched shape between glyphs ends the line: the type after it is painted after it.
        let mut open = false;
        // How many glyphs each shape is: a shape of several becomes type only when all of them do.
        let mut glyphs_of: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
        for (pc, parts) in pieces.iter().zip(&found) {
            if pc.after_barrier {
                open = false;
            }
            if parts.is_empty() {
                unmatched.push(pc.id);
                open = false;
                continue;
            }
            glyphs_of.insert(pc.id, parts.len());
            for ms in parts {
                // The part's candidates in the face most shapes fit (the first such face, as the
                // matches come upright and closest first).
                let face = ms.iter().enumerate().max_by_key(|(i, m)| (rank(m.face.id()), std::cmp::Reverse(*i))).map(|(_, m)| m.face.id());
                let cands: Vec<GlyphMatch> = ms.iter().filter(|m| Some(m.face.id()) == face).cloned().collect();
                let joined = match lines.last_mut() {
                    Some(l) if open => join(l, pc, &cands),
                    _ => false,
                };
                if !joined {
                    lines.extend(new_line(pc, &cands, tolerance));
                }
                open = true;
            }
        }
        // A lone one-contour glyph (a period, a hyphen, a bullet) is too plain to tell from drawn
        // art (a dot, a bar): beside other shapes, the other glyphs of a split compound path
        // included, it stays an outline. Selected on its own, it is what was asked for. Lone means
        // alone on its baseline as the outlines lie, before the layout check splits lines (tracking
        // and manual kerning split them glyph by glyph).
        let lone_pieces: std::collections::HashSet<NodeId> =
            lines.iter().filter(|l| matches!(l.matches.as_slice(), [m] if m.contours == 1)).flat_map(|l| l.pieces.iter().copied()).collect();
        let mut lines = verified(lines, system_layout, &mut unmatched);
        if pieces.len() > 1 || glyphs_of.values().any(|n| *n > 1) {
            lines.retain(|l| {
                let lone = matches!(l.pieces.as_slice(), [id] if lone_pieces.contains(id));
                if lone {
                    unmatched.extend(l.pieces.iter().copied());
                }
                !lone
            });
        }
        let dropped = partial_lines(&lines.iter().map(|l| l.pieces.clone()).collect::<Vec<_>>(), &glyphs_of);
        let mut k = 0;
        lines.retain(|l| {
            let keep = !dropped.contains(&k);
            k += 1;
            if !keep {
                unmatched.extend(l.pieces.iter().copied());
            }
            keep
        });
        // A shape of several glyphs is reported once, and not when it became type.
        let kept: std::collections::HashSet<NodeId> = lines.iter().flat_map(|l| l.pieces.iter().copied()).collect();
        let mut seen = std::collections::HashSet::new();
        unmatched.retain(|id| !kept.contains(id) && seen.insert(*id));
        (lines, unmatched, truncated)
    };
    if lines.is_empty() {
        return Err(EngineError::Other(format!("Retype: no selected shape matches a glyph of an available font ({} checked)", unmatched.len())));
    }
    let made = s.edit("Retype", |d, sel| {
        let mut ids = vec![];
        let mut emptied = vec![];
        // Every line's type goes in before the shapes go: a shape of several glyphs is the first
        // shape of more than one line. Each goes just before its first shape, so lines from one
        // shape keep their order.
        for l in &lines {
            let Some(first) = l.pieces.first() else { continue };
            let (par, idx, _) = d.position(*first).ok_or(EngineError::NoNode(*first))?;
            let t = l.text_object(system_layout);
            let id = d.alloc_id();
            let mut node = Node::new(id, NodeKind::Text(Box::new(t)));
            // The glyphs' own transparency (the group's, kept, still applies over it).
            node.opacity = l.opacity;
            node.blend = l.blend;
            d.insert(par, idx, node)?;
            ids.push(id);
        }
        let mut removed = std::collections::HashSet::new();
        for pc in lines.iter().flat_map(|l| l.pieces.iter()) {
            if !removed.insert(*pc) {
                continue;
            }
            if let Some((Some(parent), _, _)) = d.position(*pc) {
                emptied.push(parent);
            }
            d.remove(*pc)?;
        }
        // A group left holding only the new type (Create Outlines' group) gives way to it, unless
        // the group has transparency, effects, a mask or a name of its own, which then keep
        // applying to the type inside it. A group left empty goes.
        emptied.sort_unstable();
        emptied.dedup();
        for g in emptied {
            let Some(n) = d.node(g) else { continue };
            let NodeKind::Group { children, clip: false } = &n.kind else { continue };
            let plain = n.opacity == 1.0
                && n.blend == vectorcraft_color::BlendMode::Normal
                && *n.appearance == Appearance::default()
                && n.name.is_none()
                // Its mask, Attributes note and the rest of what few objects have stay with it.
                && {
                    let mut ex = n.extras().clone();
                    ex.type_outlines = false;
                    ex == vectorcraft_doc::NodeExtras::default()
                }
                // Isolation and knockout change how the group composites its type.
                && !n.isolate
                && n.knockout == vectorcraft_doc::Knockout::default()
                && !n.knockout_shape;
            let only_type = children.iter().all(|c| ids.contains(&c.id));
            if children.is_empty() {
                d.remove(g)?;
            } else if plain && only_type {
                let kids: Vec<Arc<Node>> = children.clone();
                let (par, idx, _) = d.position(g).ok_or(EngineError::NoNode(g))?;
                d.remove(g)?;
                for (i, k) in kids.into_iter().enumerate() {
                    d.insert(par, idx + i, (*k).clone())?;
                }
            }
        }
        let mut keep: Vec<NodeId> = roots.iter().copied().filter(|r| d.node(*r).is_some()).collect();
        keep.extend(&ids);
        sel.set(keep);
        Ok(ids)
    })?;
    Ok(json!({
        "ids": made.iter().map(|i| i.0).collect::<Vec<_>>(),
        "texts": lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
        "fonts": lines
            .iter()
            .map(|l| {
                let score = l.matches.iter().map(GlyphMatch::score).fold(1.0, f64::min);
                json!({"family": l.face.family, "style": l.face.style, "size": l.scale * l.face.units_per_em(), "score": (score * 1000.0).round() / 1000.0})
            })
            .collect::<Vec<_>>(),
        "unmatched": unmatched.iter().map(|i| i.0).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
#[path = "retype_tests.rs"]
mod tests;
