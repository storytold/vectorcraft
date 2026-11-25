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
//! per line of glyphs, with the first glyph's fill and stroke. A line whose glyphs are evenly spaced
//! apart takes that spacing as its tracking ([`Line::tracked`]), and lines stacked at one leading
//! and lined up at their starts, ends or centers become one area type object ([`blocks`]). Shapes
//! that match no font stay as they are and are reported.
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
        "{fonts?: [family…], tolerance?: 0..0.05 (default 0: exact matches only)} turn the selected outlined text (glyph paths and compound paths, inside groups too) back into live type: each shape is matched against the glyphs of the fonts the app has loaded, the document's fonts and the families in fonts, the same up to uniform scale, rotation and translation; with a tolerance above 0, a shape that is no glyph exactly may nearly be one (an outline simplified, a curve split in two, contours in another order, start or direction) when its leftover distance from the glyph, relative to the shape's outline spread (the root mean square distance of its sampled outline points from their center), is at most the tolerance (VectorCraft's own measure; shapes of at most 8 contours); a compound path holding several glyphs (a word outlined as one path, at most 256 contour groups, 512 contours and 65,536 path elements, and 16,384 contours per command) is split into them (a part matching exactly also with its contours the other way round, or with only some of them so, as an o's counter alone in an even-odd compound), and becomes type only when every one of them matches; matched glyphs become point text in that font and size at the same baseline, one object per line unless lines stack into one (below), with the first glyph's fill, stroke, opacity and blend mode, in one undo step; a line of three or more exactly matched glyphs takes the spacing most of its gaps share as its tracking (pair kerning left out) when tracked type draws all of it in place, or more of it than untracked type (when untracked type also draws it, untracked type is kept if the tracking is under one unit or the tracked type would drop a gap one space wide (a b c keeps its spaces), and the tracked type otherwise), and negative tracking only when the line is opaque, Normal, with a solid fill or none and no stroke (its glyphs may overlap); lines that follow each other in paint order with the same group, paint, font, size, turn and tracking (an untracked line of one or two glyphs takes the others' tracking), each one the same leading (0.25 to 3 times the size) below the one before, and lined up at their starts, ends or centers become one area type object with that leading (Auto when Auto gives it) and left, right or center alignment, or one multi-line point type object when area type (after Text Wrap) would not draw the outlines' glyphs in their places or when checking the Text Wrap objects on the changed layers would copy more than 2,000,000 anchors, subpaths and shapes; where even that type would move a glyph, the lines before that glyph's line stay one object and the rest are joined again from that line; lines with an opacity below 100%, a blend mode other than Normal, a gradient or pattern fill or a stroke stay one object each; a line splits where its type would not draw the outlines' glyphs in their places (uneven spacing, manual kerning, font substitutions; with a tolerance, within it), and before a near-matched glyph the line's size and turn would draw further from its shape than the tolerance; a glyph must also paint the area the shape paints under its fill rule (a nonzero compound whose inner contour runs the same way as its outer one paints its center, so a ring glyph does not replace it; a counter narrower than three times the outline tolerance across in some direction, an outline too large to check, or parts of a split compound that overlap where its fill rule leaves the overlap unpainted, or that overlap at all when the compound has an opacity below 100%, a blend mode other than Normal, a gradient or pattern fill or a stroke, also keep the shape an outline); shapes that match no font, a combining mark that would start a line (a shape whose first 16 matched glyphs include another character's starts it as that character), glyphs type cannot draw as they are, and shapes with their own opacity mask stay as they are → {ids (the new type), texts (lines joined by \\n), fonts: [{family, style, size, tracking (1/1000 em), leading (pt, null for Auto or one line), area (bool), score (how closely the line's least close glyph fits, 1 for exact, 0 at a leftover distance of 0.05)}], unmatched: [id…] (shapes that stayed as they are when another shape became type), truncated (shapes past the limits of 2048 shapes or 16 nested groups were left out)}; an error, with no unmatched list, when no shape becomes type, or when tolerance is not a number from 0 to 0.05",
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
#[derive(Clone)]
struct Line {
    pieces: Vec<NodeId>,
    /// Tracking in 1/1000 em, added after each character, spaces included.
    tracking: f64,
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
            // After the line's end, or, for type tracked tighter than that, still ahead of the
            // last glyph's start (by more than rounding); the layout check decides whether it fits.
            && ((m.origin - self.end).dot(dir) > -size * 0.25
                || self.matches.last().is_some_and(|last| (m.origin - last.origin).dot(dir) > size * 1e-3))
    }

    fn size(&self) -> f64 {
        self.scale * self.face.units_per_em()
    }

    /// The shapes of the one-contour glyphs that stand alone on this line: neither the glyph before
    /// nor the one after continues it by starting after the end of the glyph before it. A glyph
    /// that joined the line only through the tight-tracking test counts as alone. A line of three
    /// glyphs or more is type (its tracking can be read), so none of its glyphs is alone.
    fn lone_one_contour(&self) -> Vec<NodeId> {
        if self.matches.len() >= 3 {
            return vec![];
        }
        let dir = self.dir();
        let size = self.size();
        // Does glyph `k` start after the end of the glyph before it, less a quarter em?
        let after_previous = |k: usize| match (k.checked_sub(1).and_then(|j| self.matches.get(j)), self.matches.get(k)) {
            (Some(a), Some(b)) => (b.origin - (a.origin + dir * (a.face.advance(a.gid) * a.scale))).dot(dir) > -size * 0.25,
            _ => false,
        };
        let n = self.matches.len();
        (0..n)
            .filter(|&k| !after_previous(k) && !after_previous(k + 1))
            .filter_map(|k| self.matches.get(k).filter(|m| m.contours == 1).and_then(|_| self.pieces.get(k).copied()))
            .collect()
    }

    /// The line's tracking in points.
    fn tracking_pt(&self) -> f64 {
        self.tracking * self.size() / 1000.0
    }

    fn push(&mut self, id: NodeId, m: &GlyphMatch) {
        self.push_joined(id, m, None);
    }

    /// For each of `ms`'s glyphs, the distance along the line to the next when untracked type sets
    /// them with no spaces (their advances and pair kerning), and when it sets one space between
    /// them (the glyph's own advance plus a space where type draws no such layout), taking the
    /// characters laid out from `budget`; `None` when the budget is short or type does not draw
    /// every glyph without spaces.
    fn pair_steps(&self, ms: &[GlyphMatch], system_layout: bool, budget: &mut usize) -> Option<Vec<(f64, f64)>> {
        *budget = budget.checked_sub(ms.len().saturating_mul(3))?;
        let dir = self.dir();
        let mut bare = self.clone();
        bare.tracking = 0.0;
        let mut origins = |text: String| -> Vec<Point> {
            bare.text = text;
            let t = bare.probe(system_layout);
            let lay = vectorcraft_text::layout(FontDb::global(), &t);
            lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty()).map(|g| t.xf * (g.xf * Point::ORIGIN)).collect()
        };
        let joined = origins(ms.iter().map(|m| m.ch).collect());
        if joined.len() != ms.len() {
            return None;
        }
        let spaced = origins(ms.iter().map(|m| m.ch.to_string()).collect::<Vec<_>>().join(" "));
        let space = space_advance(&self.face) * self.scale;
        let spaced_ok = spaced.len() == ms.len();
        Some(
            ms.iter()
                .enumerate()
                .zip(joined.windows(2))
                .map(|((i, m), p)| {
                    let own = m.face.advance(m.gid) * m.scale + space;
                    let apart = match (spaced_ok, spaced.get(i), spaced.get(i + 1)) {
                        (true, Some(a), Some(b)) => (*b - *a).dot(dir),
                        _ => own,
                    };
                    ((p[1] - p[0]).dot(dir), apart)
                })
                .collect(),
        )
    }

    /// [`Line::push`], with `steps` the distances [`Line::pair_steps`] gives for the pair of the
    /// line's last glyph and `m`. With `steps`, a gap reads as spaces only when they fit it better
    /// than the joined pair does, as when the tracking is measured, however narrow the gap is.
    fn push_joined(&mut self, id: NodeId, m: &GlyphMatch, steps: Option<(f64, f64)>) {
        // Without `steps`, a gap, past the tracking, wider than half a space before the glyph is
        // spaces the outlines left no shape for. Each space takes the tracking too.
        let dir = self.dir();
        let tr = self.tracking_pt();
        let gap = (m.origin - self.end).dot(dir) - tr;
        // Tracking tighter than a space's width gives each space a negative advance.
        let face_space = space_advance(&self.face) * self.scale;
        let space = face_space + tr;
        let spaces = if space.abs() > 1e-9 { gap / space } else { 0.0 };
        let n = match steps {
            Some((joined, apart)) => {
                let d = (m.origin - self.matches.last().map_or(self.origin, |p| p.origin)).dot(dir);
                // How far `n` spaces set the glyph from where the outlines put it: the pair with one
                // space between them is `apart`, and each further space adds a space and the tracking.
                let spaced_err = |n: usize| (d - apart + face_space - tr - n as f64 * space).abs();
                // Kerning against a space widens or narrows the gap, so the count the gap gives, the
                // counts next to it and the count the measured distance gives are compared; the
                // rounded count wins a tie.
                let measured = (d - apart + face_space - tr) / space;
                let best = [spaces.round(), spaces.floor(), spaces.ceil(), measured.round()]
                    .into_iter()
                    .filter(|c| c.is_finite())
                    .map(|c| (c.max(1.0) as usize).min(64))
                    .min_by(|a, b| spaced_err(*a).total_cmp(&spaced_err(*b)))
                    .unwrap_or(1);
                // The joined pair wins a tie.
                if (d - joined - tr).abs() <= spaced_err(best) + self.size() * 1e-5 { 0 } else { best }
            }
            None if spaces.is_finite() && spaces > 0.5 => (spaces.round() as usize).clamp(1, 64),
            None => 0,
        };
        self.text.extend(std::iter::repeat_n(' ', n));
        self.text.push(m.ch);
        self.pieces.push(id);
        self.matches.push(m.clone());
        self.end = m.origin + dir * (m.face.advance(m.gid) * m.scale);
    }

    /// The line of this one's glyphs `from..to`, with its group, paint, transparency, tracking and
    /// the spaces between those glyphs.
    fn part(&self, from: usize, to: usize) -> Option<Line> {
        let mut text = String::new();
        let mut glyph = 0usize;
        for c in self.text.chars() {
            // The text after glyph `to - 1` is not read.
            if glyph >= to {
                break;
            }
            if c == ' ' {
                // Spaces before glyph `glyph`, after glyph `glyph - 1`.
                if glyph > from {
                    text.push(c);
                }
                continue;
            }
            if glyph >= from {
                text.push(c);
            }
            glyph += 1;
        }
        if glyph != to || to > self.matches.len() {
            return self.respaced(from, to, self.tracking);
        }
        let mut l = self.respaced(from, to.min(from.saturating_add(1)), self.tracking)?;
        l.pieces = self.pieces.get(from..to)?.to_vec();
        l.matches = self.matches.get(from..to)?.to_vec();
        let last = l.matches.last()?;
        l.end = last.origin + l.dir() * (last.face.advance(last.gid) * last.scale);
        l.text = text;
        Some(l)
    }

    /// The line of this one's glyphs `from..to` at `tracking`, with its group, paint and
    /// transparency, its spaces read again from the gaps at that tracking.
    fn respaced(&self, from: usize, to: usize, tracking: f64) -> Option<Line> {
        let (ids, ms) = (self.pieces.get(from..to)?, self.matches.get(from..to)?);
        let (first, m) = (ids.first()?, ms.first()?);
        let dir = Vec2::new(m.angle.cos(), m.angle.sin());
        let mut l = Line {
            pieces: vec![*first],
            tracking,
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

    /// The point type that sets this line, not yet laid out (for a layout check).
    fn probe(&self, system_layout: bool) -> TextObject {
        let style = CharStyle {
            font_family: self.face.family.clone(),
            font_style: self.face.style.clone(),
            // The version that matched, when two of the family and style are loaded.
            font_version: (!self.face.version.is_empty()).then(|| self.face.version.clone()),
            // Each outlined glyph was drawn on its own; a ligature would draw another shape.
            features: vec!["-liga".into()],
            size: self.scale * self.face.units_per_em(),
            tracking: self.tracking,
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
        t
    }

    /// The point type that sets this line, laid out and measured.
    fn text_object(&self, system_layout: bool) -> TextObject {
        let mut t = self.probe(system_layout);
        super::typecmd::refresh_bounds(&mut t);
        t
    }

    /// The first of this line's glyphs its type does not draw as the outlines did: another glyph
    /// (a substitution the font makes) or the same glyph elsewhere (tracking, manual spacing,
    /// more space than whole spaces make); `None` when the type draws every glyph in its place.
    fn first_mismatch(&self, t: &TextObject) -> Option<usize> {
        first_mismatch(t, &self.matches, self.size(), self.slack)
    }

    /// Along the line, where its first glyph starts and its last glyph's advance ends, from
    /// `from` (the tracking after the last glyph left out).
    fn span_from(&self, from: Point) -> (f64, f64) {
        let dir = self.dir();
        let last = self.matches.last().map_or(self.end, |m| m.origin + dir * (m.face.advance(m.gid) * m.scale));
        ((self.origin - from).dot(dir), (last - from).dot(dir))
    }

    /// This line with the tracking its glyphs are spaced by, when it has at least three glyphs.
    /// The glyphs are laid out untracked and without spaces, so each gap's extra is how much
    /// further apart the outlines are than the font's advances and pair kerning set them; a gap
    /// read as word spaces is measured from the glyph's own advance instead, as no pair kerning
    /// crosses a space. The extra most gaps share (the lower median, so that the word spaces of a
    /// few gaps do not count) places the word spaces; the tracking is then the lower median, over
    /// the gaps, of each gap's extra less its spaces, per character. The upper median of the
    /// extras is tried next, for a line whose word spaces are its tighter gaps, and last the lower
    /// median less one space, halved, for a line most of whose gaps hold a word space. A tracking within
    /// 0.05 of a whole number is tried rounded to it first, then as measured. A tracked line is
    /// kept when its type draws the whole line in place, or more of it from its start than
    /// untracked type does. When untracked type with the spaces also draws the whole line, the line stays
    /// untracked if the tracking is under one unit or the tracked reading drops a gap untracked
    /// type reads as one space. The line also stays as it is when its tracking is 0 or outside
    /// -1000..10000, when one of its glyphs is a near match, and when its tracking is negative and the line does not paint the same as one
    /// object ([`paints_as_one`]: its glyphs may overlap). Each of the nine layouts at most takes
    /// its characters, spaces included, from `budget` before it runs; without enough left, the
    /// line stays as it is, or becomes the best tracked reading laid out before the budget ran out.
    fn tracked(self, system_layout: bool, budget: &mut usize) -> Line {
        let reach = self.text.chars().count();
        self.tracked_reaching(system_layout, budget, reach)
    }

    /// [`Line::tracked`] for a line whose tracking is applied to `reach` characters, its glyphs and
    /// the spaces between them: a measured tracking within 0.05 of a whole number is read as that
    /// number only when the difference moves the character `reach` places on by no more than the
    /// tolerance [`first_mismatch`] allows.
    fn tracked_reaching(self, system_layout: bool, budget: &mut usize, reach: usize) -> Line {
        let size = self.size();
        let n = self.pieces.len();
        // A near-matched glyph sits within the tolerance of where the font draws it, so the gaps
        // of a line with one say nothing about spacing.
        if n < 3 || size <= 0.0 || self.matches.iter().any(|m| !m.exact) {
            return self;
        }
        // Each layout takes its characters, spaces included, from `budget` before it runs.
        let mut take = |text: &str| match budget.checked_sub(text.chars().count()) {
            Some(left) => {
                *budget = left;
                true
            }
            None => false,
        };
        if !take(&self.text) {
            return self;
        }
        let plain_miss = self.first_mismatch(&self.probe(system_layout));
        let dir = self.dir();
        // Where untracked type draws the glyphs of `text` (each layout dropped once read).
        let mut bare = self.clone();
        bare.tracking = 0.0;
        let mut origins = |text: String| -> Vec<Point> {
            bare.text = text;
            let t = bare.probe(system_layout);
            let lay = vectorcraft_text::layout(FontDb::global(), &t);
            lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty()).map(|g| t.xf * (g.xf * Point::ORIGIN)).collect()
        };
        // With no spaces.
        let joined: String = self.matches.iter().map(|m| m.ch).collect();
        if !take(&joined) {
            return self;
        }
        let drawn = origins(joined);
        if drawn.len() != n {
            return self;
        }
        // And with one space between each two glyphs (kerning against the space included).
        let spaced = self.matches.iter().map(|m| m.ch.to_string()).collect::<Vec<_>>().join(" ");
        if !take(&spaced) {
            return self;
        }
        let drawn_spaced = origins(spaced);
        // A font whose space has ink draws more glyphs: then the glyph's own advance stands in.
        let spaced_ok = drawn_spaced.len() == n;
        let space = space_advance(&self.face) * self.scale;
        // Each gap's extra over the joined pair, and over the pair with one space between them
        // plus a space (so the extra counts the spaces as the joined pair's extra counts none).
        let gaps: Vec<(f64, f64)> = self
            .matches
            .windows(2)
            .enumerate()
            .zip(drawn.windows(2))
            .filter_map(|((i, m), p)| match (m, p) {
                ([a, b], [pa, pb]) => {
                    let d = (b.origin - a.origin).dot(dir);
                    let apart = match (spaced_ok, drawn_spaced.get(i), drawn_spaced.get(i + 1)) {
                        (true, Some(qa), Some(qb)) => d - (*qb - *qa).dot(dir) + space,
                        _ => d - a.face.advance(a.gid) * a.scale,
                    };
                    Some((d - (*pb - *pa).dot(dir), apart))
                }
                _ => None,
            })
            .collect();
        let extra: Vec<f64> = gaps.iter().map(|g| g.0).collect();
        // The spaces in each gap at a tracking of `tr` points: each space takes the tracking too,
        // and tracking tighter than a space's width gives it a negative advance. A gap reads as
        // spaces when they fit it better than the joined pair does.
        let spaces = |tr: f64| -> Vec<usize> {
            gaps.iter()
                .map(|(joined, apart)| {
                    let step = space + tr;
                    // Spaces with no advance: one space, which still breaks the pair's kerning.
                    let s = if step.abs() > 1e-9 { (apart - tr) / step } else { 1.0 };
                    if !s.is_finite() {
                        return 0;
                    }
                    // At least one space: the space's own kerning can make it fit a narrower gap.
                    let k = (s.round().max(1.0) as usize).clamp(1, 64);
                    // Spaces only where they fit better by more than rounding (1/100,000 em).
                    if (apart - tr - k as f64 * step).abs() + size * 1e-5 < (joined - tr).abs() { k } else { 0 }
                })
                .collect()
        };
        // The tracking the gaps give when the word spaces are placed by `seed`.
        let measure = |seed: f64| -> Option<f64> {
            let per: Vec<f64> = gaps
                .iter()
                .zip(spaces(seed))
                .map(|((joined, apart), s)| if s == 0 { *joined } else { (apart - s as f64 * space) / (s + 1) as f64 })
                .collect();
            median(per, false).map(|g| g / size * 1000.0)
        };
        // The lower median of the extras places the word spaces first; the upper one second, for a
        // line whose word spaces are its tighter gaps (tracking tighter than a space).
        let lower = median(extra.clone(), false);
        // A line whose word gaps are most of its gaps has a space and two trackings in each extra
        // (tracking is added after the glyph and after the space): the seed then places the spaces
        // from the extra less one space, shared by the two. It goes last, so a reading that fits
        // without it is kept (an untracked "a b c" has no such tracking).
        // Where untracked type already draws the whole line, a space and a tracking of half its
        // advance less draw it too: that reading is not tried.
        let spaced_seed = lower.filter(|_| plain_miss.is_some()).map(|e| (e - space) / 2.0);
        let mut tries = vec![];
        for seed in [lower, median(extra, true), spaced_seed].into_iter().flatten() {
            let Some(measured) = measure(seed) else { continue };
            let rounded = round_tracking(measured);
            let drift = (rounded - measured).abs() / 1000.0 * size * reach.saturating_sub(1) as f64;
            for t in [rounded, measured] {
                if t != measured && drift > (size * 0.01).max(0.01) {
                    continue;
                }
                if !tries.contains(&t) {
                    tries.push(t);
                }
            }
        }
        // The spaces in each gap of the line as it is (the reading `plain_miss` checked).
        let mut plain_gaps = vec![];
        let mut run = 0usize;
        for (i, c) in self.text.chars().enumerate() {
            if c == ' ' {
                run += 1;
            } else {
                if i > 0 {
                    plain_gaps.push(run);
                }
                run = 0;
            }
        }
        let mut best: Option<(usize, Line)> = None;
        for t in tries {
            if t == 0.0 || !TRACKING.contains(&t) {
                continue;
            }
            if t < 0.0 && !paints_as_one(&self) {
                continue;
            }
            let mut tracked = self.clone();
            tracked.tracking = t;
            tracked.text = String::new();
            let gaps = spaces(t * size / 1000.0);
            for (k, (m, s)) in self.matches.iter().zip(std::iter::once(0).chain(gaps.iter().copied())).enumerate() {
                if k > 0 {
                    tracked.text.extend(std::iter::repeat_n(' ', s));
                }
                tracked.text.push(m.ch);
            }
            // A reading the budget cannot lay out is skipped; the next may be shorter.
            if !take(&tracked.text) {
                continue;
            }
            // When both draw the whole line (gaps as wide as whole spaces), the tracked reading is
            // kept unless it drops a gap that untracked type reads as one space: such a gap is a
            // word space ("a b c"), and the line keeps its spaces.
            let drops_a_word_space = plain_gaps.iter().zip(&gaps).any(|(plain, tracked)| *plain == 1 && *tracked == 0);
            match (tracked.first_mismatch(&tracked.probe(system_layout)), plain_miss) {
                // Under a unit of tracking, or a word space the tracked reading drops, where
                // untracked type draws the line: no tracking.
                (None, None)
                    if (t.abs() < 1.0 && t.abs() / 1000.0 * size * reach.saturating_sub(1) as f64 <= (size * 0.01).max(0.01))
                        || drops_a_word_space =>
                {
                    return self;
                }
                (None, _) => return tracked,
                (Some(k), Some(p)) if k > p && best.as_ref().is_none_or(|(b, _)| k > *b) => best = Some((k, tracked)),
                _ => {}
            }
        }
        best.map_or(self, |(_, l)| l)
    }
}

/// Does line `l` paint the same as one object as glyph by glyph where its glyphs overlap: opaque,
/// Normal blend, a solid fill or none, and no stroke? One object composites its transparency and
/// blend once, spans a gradient over all of it, and strokes after filling every glyph.
fn paints_as_one(l: &Line) -> bool {
    paints_once(l.opacity, l.blend, &l.appearance)
}

/// Does art with `opacity`, `blend` and `appearance` paint its overlaps the same as one object
/// and as several ([`paints_as_one`])?
fn paints_once(opacity: f32, blend: vectorcraft_color::BlendMode, appearance: &Appearance) -> bool {
    opacity == 1.0
        && blend == vectorcraft_color::BlendMode::Normal
        && matches!(appearance.fill_paint(), vectorcraft_color::Paint::None | vectorcraft_color::Paint::Solid { .. })
        && matches!(appearance.stroke_paint(), vectorcraft_color::Paint::None)
}

/// The tracking Retype sets, in 1/1000 em.
const TRACKING: std::ops::RangeInclusive<f64> = -1000.0..=10000.0;

/// The lower median of the finite values of `v`, or the upper one when `upper`; None for fewer
/// than two.
fn median(mut v: Vec<f64>, upper: bool) -> Option<f64> {
    v.retain(|x| x.is_finite());
    if v.len() < 2 {
        return None;
    }
    v.sort_by(f64::total_cmp);
    v.get(if upper { v.len() / 2 } else { (v.len() - 1) / 2 }).copied()
}

/// Tracking `t` rounded to a whole number when it is within 0.05 of one.
fn round_tracking(t: f64) -> f64 {
    if (t - t.round()).abs() < 0.05 { t.round() } else { t }
}

/// The first of `matches` that type `t` (at `size`) does not draw as the outlines did: another
/// glyph (a substitution the font makes) or the same glyph elsewhere (tracking, manual spacing,
/// more space than whole spaces make); `None` when the type draws every glyph in its place.
fn first_mismatch(t: &TextObject, matches: &[GlyphMatch], size: f64, slack: f64) -> Option<usize> {
    let lay = vectorcraft_text::layout(FontDb::global(), t);
    let mut drawn = lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty());
    // With a tolerance, a near-matched glyph, or any glyph of a line whose first glyph is one,
    // may sit that much further (as a fraction of the size) from where the type draws it.
    let first_exact = matches.first().is_none_or(|f| f.exact);
    for (k, m) in matches.iter().enumerate() {
        let Some(g) = drawn.next() else { return Some(k) };
        let tol = (size * (0.01 + if m.exact && first_exact { 0.0 } else { slack })).max(0.01);
        let at = t.xf * (g.xf * Point::ORIGIN);
        if g.font_id != m.face.id() || g.gid != m.gid || at.distance(m.origin) > tol {
            return Some(k);
        }
        // The type sets the glyph at its line's size and turn, which may differ from the glyph's
        // own: the glyph drawn so must still lie within the tolerance of the shape (its center
        // within the same distance as its origin).
        if !(m.exact && first_exact) {
            let (shape, moved) = set_apart(t, g, m);
            // An exact match's own leftover is within the exact matcher's reach, not the tolerance.
            let own = if m.exact { 0.0 } else { m.pose_residual };
            if own.hypot(shape) > slack + 1e-9 || moved > tol {
                return Some(k);
            }
        }
    }
    // Ink the outlines did not have: the last glyph is where it went wrong.
    drawn.next().map(|_| matches.len().saturating_sub(1))
}

/// How glyph `g`, as type `t` draws it, differs from glyph `m` in the size, turn and place `m`
/// was matched in. The first value bounds, whatever points the outline is sampled at, the root
/// mean square distance between the two outlines' points about their center, relative to the
/// root mean square distance of `m`'s points from that center (the measure of near matches'
/// leftover distance): the largest stretch of the change from `m`'s size and turn to the drawn
/// one, which for a change of size and turn alone is that distance exactly. The second is how far
/// apart the two draw the center of the glyph's bounding box. Both are infinite when they cannot
/// be measured.
fn set_apart(t: &TextObject, g: &vectorcraft_text::PositionedGlyph, m: &GlyphMatch) -> (f64, f64) {
    let (sin, cos) = m.angle.sin_cos();
    let matched = kurbo::Affine::new([m.scale * cos, m.scale * sin, -m.scale * sin, m.scale * cos, m.origin.x, m.origin.y]);
    let (gd, md) = (g.xf.determinant(), matched.determinant());
    if !gd.is_finite() || gd.abs() < 1e-12 || !md.is_finite() || md.abs() < 1e-12 {
        return (f64::INFINITY, f64::INFINITY);
    }
    let drawn = t.xf * g.xf;
    // The change from the matched glyph to the drawn one, less the identity, without the move.
    let [a, b, c, d, _, _] = (drawn * matched.inverse()).as_coeffs();
    let (a, d) = (a - 1.0, d - 1.0);
    // The largest singular value of [[a, c], [b, d]].
    let (p, q) = ((a * a + b * b + c * c + d * d) / 2.0, a * d - b * c);
    let shape = (p + (p * p - q * q).max(0.0).sqrt()).sqrt();
    // The center of the glyph's box, in font units.
    let center = g.xf.inverse() * kurbo::Shape::bounding_box(&g.outline).center();
    let moved = (drawn * center).distance(matched * center);
    let finite = |v: f64| if v.is_finite() { v } else { f64::INFINITY };
    (finite(shape), finite(moved))
}

/// The most glyphs [`verified`] measures the tracking of after a split.
const MEASURE_SPAN: usize = 32;

/// One part of shape `id`, by the glyph it matched (face, glyph, origin, turn and scale): the parts
/// of a compound path share its id.
type PartKey = (NodeId, u32, u32, [u64; 4]);

fn part_key(id: NodeId, m: &GlyphMatch) -> PartKey {
    (id, m.face.id(), m.gid, [m.origin.x.to_bits(), m.origin.y.to_bits(), m.angle.to_bits(), m.scale.to_bits()])
}

/// `lines` checked against their own layout: a line splits where its type stops drawing the
/// outlines, and a glyph its type cannot draw even on its own stays an outline (`unmatched`).
/// `spacing` holds, for a part of a shape read as a combining mark, the other character that part
/// is. The glyphs laid out are bounded by a multiple of the number of glyphs.
fn verified(
    lines: Vec<Line>,
    system_layout: bool,
    spacing: &std::collections::HashMap<PartKey, GlyphMatch>,
    unmatched: &mut Vec<NodeId>,
) -> Vec<Line> {
    // Glyphs laid out in all, at most: past it, the glyphs left unchecked stay outlines.
    let mut budget = lines.iter().map(|l| l.pieces.len()).sum::<usize>().saturating_mul(16).saturating_add(256);
    // Measuring the tracking of the glyphs after a split has a budget of its own, so it cannot
    // leave glyphs unchecked; past it they keep no tracking. Each measure takes the glyphs to
    // rebuild the rest and the characters it lays out (see [`Line::tracked`]); the budget covers
    // one measure of each whole line, as `retype` gives the first, and the glyph checks' budget.
    // The glyphs measured again after splits are limited apart from it, at six times the glyphs
    // each measure reads from half as much again as the glyph checks' budget.
    let mut remeasured = budget.saturating_add(budget / 2);
    let mut track_budget =
        lines.iter().map(|l| l.text.chars().count().saturating_add(l.pieces.len().saturating_mul(394))).fold(budget, usize::saturating_add);
    let mut todo: std::collections::VecDeque<Line> = lines.into();
    let mut out = vec![];
    while let Some(l) = todo.pop_front() {
        let n = l.pieces.len();
        // A combining mark goes with the character before it: a mark that a split left first on
        // a line becomes the other character its shape is (`spacing`), alone on a line, or else
        // stays an outline, and the line goes on from the glyph after it ([`new_line`] starts no
        // line with one).
        if l.matches.first().is_some_and(|m| vectorcraft_text::retype::is_combining_mark(m.ch)) {
            let first = l.pieces.first().copied();
            let alone = first.zip(l.matches.first()).and_then(|(id, mark)| spacing.get(&part_key(id, mark)).map(|m| reposed(&l, id, m)));
            if alone.is_none() {
                unmatched.extend(first);
            }
            if n > 1 {
                // The tracking was measured with the mark's gap in it: the rest is measured again
                // without it, from the same budgets as the rest after a split. A rest that starts
                // with a mark too is not measured here: its own pass leaves that mark out and
                // measures the glyphs after it.
                let measurable = n > 3 && remeasured >= (n - 1).saturating_mul(6) && track_budget >= (n - 1).saturating_mul(6);
                let rest = if l.matches.get(1).is_some_and(|m| vectorcraft_text::retype::is_combining_mark(m.ch)) {
                    l.part(1, n)
                } else if measurable {
                    remeasured = remeasured.saturating_sub((n - 1).saturating_mul(6));
                    track_budget = track_budget.saturating_sub(n - 1);
                    l.respaced(1, n, 0.0).map(|r| r.tracked(system_layout, &mut track_budget))
                } else if l.tracking == 0.0 {
                    l.part(1, n)
                } else {
                    l.respaced(1, n, 0.0)
                };
                match rest {
                    Some(rest) => todo.push_front(rest),
                    None => unmatched.extend(l.pieces.get(1..).unwrap_or_default().iter().copied()),
                }
            }
            // Checked before the rest, so the lines keep the outlines' order.
            if let Some(alone) = alone {
                todo.push_front(alone);
            }
            continue;
        }
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
            match part.first_mismatch(&part.probe(system_layout)) {
                // The whole line is drawn as it was: one object.
                None if e == n => {
                    out.push(part);
                    break;
                }
                None => w = w.saturating_mul(2),
                // Tracking that type does not set as the outlines are spaced: the line is
                // checked again without it.
                Some(k) if l.tracking != 0.0 && k < 2 => {
                    match l.respaced(0, n, 0.0) {
                        Some(p) => todo.push_front(p),
                        None => unmatched.extend(l.pieces.iter().copied()),
                    }
                    break;
                }
                // A single glyph its type cannot draw as it was stays an outline.
                Some(0) if n == 1 => {
                    unmatched.extend(l.pieces.first().copied());
                    break;
                }
                // The glyphs before the mismatch, then the rest, each checked as a line of its
                // own: the type before the mismatch may draw otherwise without what follows it (a
                // font can place a glyph by the glyph after it), and the first glyph on its own
                // may still be drawn as it was (a contextual form changed it in the line).
                Some(k) => {
                    let k = k.max(1);
                    let rest = rest_after_split(&l, k, system_layout, &mut budget, &mut remeasured, &mut track_budget);
                    match rest {
                        Some(r) => todo.push_front(r),
                        None => unmatched.extend(l.pieces.get(k..).unwrap_or_default().iter().copied()),
                    }
                    todo.extend_front_parts(&l, [(0, k)]);
                    break;
                }
            }
        }
    }
    out
}

/// The rest of `l` from glyph `k` (at least 1) after [`verified`] splits it there. The rest takes
/// the tracking its own glyphs are spaced by, while the budgets for measuring it last and when type
/// draws its first glyph as it was, followed by the next two (else that glyph splits off first, and
/// the tracking is measured from the next one); past it, an untracked line's rest keeps its text as
/// it is. The tracking is measured from the rest's first [`MEASURE_SPAN`] glyphs at most, and the
/// glyphs after them follow at that tracking, so each split costs the same however long the rest
/// is, and splits at irregular gaps early in a line leave the budget for a long evenly tracked run
/// after them. A gap after the glyphs measured reads as spaces only when they fit it better than
/// the joined pair does, as in the glyphs measured: laying those pairs out is set aside from
/// `track_budget` before the tracking is measured, and without enough for both the rest is not
/// measured.
fn rest_after_split(l: &Line, k: usize, system_layout: bool, budget: &mut usize, remeasured: &mut usize, track_budget: &mut usize) -> Option<Line> {
    let n = l.pieces.len();
    let span = (n - k).min(MEASURE_SPAN);
    let more = n - k - span;
    // What [`Line::pair_steps`] takes for the last glyph measured and the glyphs after it.
    let pairs = if more > 0 { more.saturating_add(1).saturating_mul(3) } else { 0 };
    // A measure takes at least six times the glyphs it reads from each budget.
    let measurable = n - k >= 3 && *remeasured >= span.saturating_mul(6) && *track_budget >= span.saturating_mul(6).saturating_add(pairs);
    let first_drawn = measurable && {
        *budget = budget.saturating_sub(3);
        l.respaced(k, k + 3, l.tracking).is_some_and(|p| p.first_mismatch(&p.probe(system_layout)) != Some(0))
    };
    if !first_drawn {
        return if l.tracking == 0.0 { l.part(k, n) } else { l.respaced(k, n, 0.0) };
    }
    // Rebuilding the glyphs measured takes them from the budget.
    *remeasured = remeasured.saturating_sub(span.saturating_mul(6));
    *track_budget = track_budget.saturating_sub(span).saturating_sub(pairs);
    let mut pair_budget = pairs;
    // The characters the rest's tracking applies to: its glyphs and the spaces between them (a
    // space before glyph `at` follows glyph `at - 1`).
    let reach = l
        .text
        .chars()
        .scan(0usize, |glyph, c| {
            let at = *glyph;
            *glyph += usize::from(c != ' ');
            Some(if c == ' ' { at > k } else { at >= k })
        })
        .filter(|kept| *kept)
        .count()
        .max(n - k);
    let mut r = l.respaced(k, k + span, 0.0)?.tracked_reaching(system_layout, track_budget, reach);
    // Tracking that leaves a space under half its advance: the gaps alone do not show the spaces
    // past the glyphs measured (only the kerning a space breaks may), so the whole rest is measured
    // while the budget lasts.
    let space = space_advance(&r.face) * r.scale;
    if more > 0
        && space + r.tracking_pt() < space / 2.0
        && *remeasured >= (n - k).saturating_mul(6)
        && track_budget.saturating_add(pair_budget) >= (n - k).saturating_mul(6)
        && let Some(whole) = l.respaced(k, n, 0.0)
    {
        *remeasured = remeasured.saturating_sub((n - k).saturating_mul(6));
        *track_budget = track_budget.saturating_add(pair_budget).saturating_sub(n - k);
        return Some(whole.tracked(system_layout, track_budget));
    }
    // The glyphs after those measured follow at that tracking.
    let steps = match more {
        0 => vec![],
        _ => l.matches.get(k + span - 1..).and_then(|ms| r.pair_steps(ms, system_layout, &mut pair_budget)).unwrap_or_default(),
    };
    for (j, (id, m)) in l.pieces.iter().zip(&l.matches).skip(k + span).enumerate() {
        r.push_joined(*id, m, steps.get(j).copied());
    }
    Some(r)
}

/// Most units (anchors, subpaths and shapes) that Text Wrap objects may copy into the wrap shapes
/// of the area type on the layers Retype changes
/// ([`vectorcraft_doc::text::wrap_copy_units`]) when Retype checks how they flow new area type.
const MAX_WRAP_WORK: usize = 2_000_000;

#[cfg(not(test))]
fn max_wrap_work() -> usize {
    MAX_WRAP_WORK
}

#[cfg(test)]
thread_local! {
    /// [`MAX_WRAP_WORK`], lowered by tests on their own thread.
    static WRAP_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_WRAP_WORK) };
}

#[cfg(test)]
fn max_wrap_work() -> usize {
    WRAP_LIMIT.with(|c| c.get())
}

/// Lines farther apart than this many times their size do not join into one object.
const MAX_LEADING: f64 = 3.0;

/// What Retype makes: one type object and the shapes it replaces.
struct Made {
    t: TextObject,
    /// For area type, the point type that also draws the glyphs in place: the type kept when
    /// Text Wrap objects above it change the area type's layout.
    point: Option<TextObject>,
    /// The glyphs the type must draw, in order.
    matches: Vec<GlyphMatch>,
    pieces: Vec<NodeId>,
    text: String,
    face: Arc<vectorcraft_text::FontFace>,
    size: f64,
    /// The tolerance near matches were taken with ([`Line::slack`]).
    slack: f64,
    tracking: f64,
    /// The leading set on the type: None for Auto or a single line.
    leading: Option<f64>,
    opacity: f32,
    blend: vectorcraft_color::BlendMode,
}

impl Made {
    fn line(l: &Line, system_layout: bool) -> Made {
        Made {
            t: l.text_object(system_layout),
            point: None,
            matches: l.matches.clone(),
            pieces: l.pieces.clone(),
            text: l.text.clone(),
            face: l.face.clone(),
            size: l.size(),
            slack: l.slack,
            tracking: l.tracking,
            leading: None,
            opacity: l.opacity,
            blend: l.blend,
        }
    }
}

/// The alignments lines can share, in the order one is chosen: their starts, their ends, their
/// centers.
const ALIGNS: [vectorcraft_doc::Justify; 3] = [vectorcraft_doc::Justify::Left, vectorcraft_doc::Justify::Right, vectorcraft_doc::Justify::Center];

/// A run of lines that may become one object: lines after the first, each one leading further
/// across, in the same group, paint, transparency, font, size, turn and tracking, lined up alike.
struct Block {
    lines: Vec<Line>,
    /// The mean distance between baselines, from the first line's to the last line's.
    leading: f64,
    /// Which of [`ALIGNS`] every line so far shares.
    aligns: [bool; 3],
    /// The tracking of the first line that has one ([`Block::spaced`]).
    tracking: Option<f64>,
}

impl Block {
    fn new(l: Line) -> Block {
        let tracking = Block::spaced(&l).then_some(l.tracking);
        Block { lines: vec![l], leading: 0.0, aligns: [true; 3], tracking }
    }

    /// Does line `l` have a tracking of its own? A line of fewer than three glyphs that kept no
    /// tracking has no gaps to measure it by ([`Line::tracked`]): it takes the block's.
    fn spaced(l: &Line) -> bool {
        l.pieces.len() >= 3 || l.tracking != 0.0
    }

    /// Add `l` when it continues the block; `l` back when it does not. `follows` tells whether the
    /// shape `l` starts with comes right after the block's last shape, with nothing between them
    /// in paint order.
    fn take(&mut self, l: Line, follows: impl Fn(NodeId, NodeId) -> bool) -> Option<Line> {
        let (Some(first), Some(prev)) = (self.lines.first(), self.lines.last()) else { return Some(l) };
        let (Some(a), Some(b)) = (prev.pieces.last(), l.pieces.first()) else { return Some(l) };
        let size = first.size();
        let tol = (size * 0.01).max(0.01);
        let same = follows(*a, *b)
            && l.parent == first.parent
            && l.appearance == first.appearance
            && l.opacity == first.opacity
            && l.blend == first.blend
            && l.face.id() == first.face.id()
            && (l.scale - first.scale).abs() <= first.scale * 1e-4
            && ((l.angle - first.angle + PI).rem_euclid(TAU) - PI).abs() < 1e-5
            // The lines' estimates may differ a little; the layout check decides.
            && (!Block::spaced(&l) || self.tracking.is_none_or(|t| (l.tracking - t).abs() < 1.0))
            // Lines whose ink overlaps, or a gradient spanning them, would paint otherwise as one.
            && paints_as_one(first);
        if !same {
            return Some(l);
        }
        // Across the lines, down from the first line's baseline.
        let across = Vec2::new(-first.angle.sin(), first.angle.cos());
        let down = (l.origin - first.origin).dot(across);
        let n = self.lines.len() as f64;
        // The mean distance between the baselines so far, which this line's must continue.
        let leading = if self.lines.len() == 1 { down } else { self.leading };
        // A quarter of the size is the least leading; the outlines' positions are rounded.
        if !(leading >= size * 0.25 * (1.0 - 1e-6) && leading <= size * MAX_LEADING) || (down - n * leading).abs() > tol {
            return Some(l);
        }
        let leading = down / n;
        let (s0, e0) = first.span_from(first.origin);
        let (s, e) = l.span_from(first.origin);
        let fits = [(s - s0).abs() <= tol, (e - e0).abs() <= tol, ((s + e) - (s0 + e0)).abs() * 0.5 <= tol];
        let aligns = [self.aligns[0] && fits[0], self.aligns[1] && fits[1], self.aligns[2] && fits[2]];
        if !aligns.contains(&true) {
            return Some(l);
        }
        self.leading = leading;
        self.aligns = aligns;
        if self.tracking.is_none() && Block::spaced(&l) {
            self.tracking = Some(l.tracking);
        }
        self.lines.push(l);
        None
    }

    /// The block as one area type object when type draws every glyph in its place as area type,
    /// else as one point type object when that does. When neither does, the lines before the line
    /// of the first glyph the type moves are made again as a block, and the lines from that one on
    /// are joined again ([`blocks`]); the first line on its own stays one object. Area type whose
    /// frame would reach past [`crate::MAX_COORD`] stays point type, and point type whose bounds
    /// would stays one object per line. Each try takes six times the
    /// block's glyphs from `budget` (point and area type are each laid out, measured and checked);
    /// without enough left, each line is one object.
    fn made(self, system_layout: bool, budget: &mut usize) -> Vec<Made> {
        let single = |lines: &[Line]| lines.iter().map(|l| Made::line(l, system_layout)).collect::<Vec<_>>();
        let (Some(first), true) = (self.lines.first(), self.lines.len() > 1) else { return single(&self.lines) };
        let Some(justify) = ALIGNS.iter().zip(self.aligns).find(|(_, ok)| *ok).map(|(j, _)| *j) else { return single(&self.lines) };
        let glyphs = self.lines.iter().map(|l| l.pieces.len()).fold(0usize, usize::saturating_add);
        let Some(left) = budget.checked_sub(glyphs.saturating_mul(6)) else { return single(&self.lines) };
        *budget = left;
        let mut all = first.clone();
        all.tracking = self.tracking.unwrap_or(first.tracking);
        all.text = self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        let mut t = all.probe(system_layout);
        t.para.justify = justify;
        let size = first.size();
        let mut set_leading = None;
        if let Some(r) = t.runs.first_mut() {
            // The leading that puts the last baseline where the last line's is: Auto when Auto
            // does, else the mean distance between baselines, to a thousandth of a point (as
            // typed leading is) when that still puts the last baseline in place.
            let gaps = self.lines.len().saturating_sub(1) as f64;
            let across = Vec2::new(-first.angle.sin(), first.angle.cos());
            let span = self.lines.last().map_or(0.0, |l| (l.origin - first.origin).dot(across));
            if (t.para.effective_leading(&r.style) * gaps - span).abs() > size * 1e-4 {
                let exact = span / gaps;
                let rounded = (exact * 1000.0).round() / 1000.0;
                let leading = if ((rounded - exact) * gaps).abs() <= size * 1e-4 { rounded } else { exact };
                r.style.leading = Some(leading);
                set_leading = Some(leading);
            }
        }
        let matches: Vec<GlyphMatch> = self.lines.iter().flat_map(|l| l.matches.iter().cloned()).collect();
        // Aligned at its end or center, point type starts its first line elsewhere: the type moves
        // so its first glyph is where the first outline is.
        let lay = vectorcraft_text::layout(FontDb::global(), &t);
        if let (Some(g), Some(m)) = (lay.glyphs.iter().find(|g| !g.outline.elements().is_empty()), matches.first()) {
            let at = t.xf * (g.xf * Point::ORIGIN);
            t.xf = kurbo::Affine::translate(m.origin - at) * t.xf;
        }
        super::typecmd::refresh_bounds(&mut t);
        if let Some(k) = first_mismatch(&t, &matches, size, first.slack) {
            // The line that holds glyph `k`.
            let mut seen = 0usize;
            let at = self.lines.iter().position(|l| {
                seen = seen.saturating_add(l.pieces.len());
                k < seen
            });
            let Block { mut lines, leading, aligns, .. } = self;
            let rest = lines.split_off(at.unwrap_or(0).clamp(1, lines.len()));
            let mut out = if lines.len() > 1 {
                let tracking = lines.iter().find(|l| Block::spaced(l)).map(|l| l.tracking);
                Block { lines, leading, aligns, tracking }.made(system_layout, budget)
            } else {
                single(&lines)
            };
            // The lines of the rest followed each other in the block.
            out.extend(blocks(rest, |_, _| true, system_layout, budget));
            return out;
        }
        // The edit is refused when an object's bounds reach past the coordinate limit. A turned
        // block's box can reach a corner that none of its lines does: the lines then stay apart.
        let in_range =
            |t: &TextObject| Node::new(NodeId(0), NodeKind::Text(Box::new(t.clone()))).geometric_bounds().is_none_or(crate::sanity::rect_in_range);
        if !in_range(&t) {
            return single(&self.lines);
        }
        let mut area = t.clone();
        super::typemenu::point_to_area(&mut area);
        let mut point = None;
        if in_range(&area) && first_mismatch(&area, &matches, size, first.slack).is_none() {
            point = Some(std::mem::replace(&mut t, area));
        }
        vec![Made {
            t,
            point,
            matches,
            pieces: self.lines.iter().flat_map(|l| l.pieces.iter().copied()).collect(),
            text: all.text,
            face: first.face.clone(),
            size,
            slack: first.slack,
            tracking: all.tracking,
            leading: set_leading,
            opacity: first.opacity,
            blend: first.blend,
        }]
    }
}

/// A line of two untracked glyphs has one gap, too few to measure a tracking by
/// ([`Line::tracked`]). It is set at the tracking of the nearest tracked line of the same font,
/// size and turn, so the layout check ([`verified`]) tries it as that line is set: the check falls
/// back to the line untracked, and then splits it, when the pair is not spaced so. A pair that
/// untracked type already draws in place keeps its text, and negative tracking goes only to a pair
/// that paints the same as one object ([`paints_as_one`]).
fn share_tracking(lines: &mut [Line], system_layout: bool) {
    for i in 0..lines.len() {
        let Some(l) = lines.get(i).filter(|l| l.pieces.len() == 2 && l.tracking == 0.0) else { continue };
        if l.first_mismatch(&l.probe(system_layout)).is_none() {
            continue;
        }
        let like = |o: &Line| {
            o.tracking != 0.0
                && o.parent == l.parent
                && o.face.id() == l.face.id()
                && (o.scale - l.scale).abs() <= l.scale * 1e-4
                && ((o.angle - l.angle + PI).rem_euclid(TAU) - PI).abs() < 1e-5
        };
        let nearest = lines.iter().enumerate().filter(|(_, o)| like(o)).min_by_key(|(j, _)| j.abs_diff(i)).map(|(_, o)| o.tracking);
        if let Some(p) = nearest.filter(|t| *t > 0.0 || paints_as_one(l)).and_then(|t| l.respaced(0, 2, t)) {
            lines[i] = p;
        }
    }
}

/// `lines`, in paint order, as the objects Retype makes: runs of lines that stack as the lines of
/// one paragraph do become one object ([`Block`]). `follows(a, b)`: shape `b` comes right after
/// shape `a`, or is `a`, with nothing between them in paint order. `budget`: see [`Block::made`].
fn blocks(lines: Vec<Line>, follows: impl Fn(NodeId, NodeId) -> bool, system_layout: bool, budget: &mut usize) -> Vec<Made> {
    let mut out = vec![];
    let mut block: Option<Block> = None;
    for l in lines {
        let l = match block.as_mut() {
            Some(b) => match b.take(l, &follows) {
                None => continue,
                Some(l) => l,
            },
            None => l,
        };
        if let Some(b) = block.take() {
            out.extend(b.made(system_layout, budget));
        }
        block = Some(Block::new(l));
    }
    if let Some(b) = block {
        out.extend(b.made(system_layout, budget));
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
        // No line starts with a combining mark (see [`new_line`]).
        .filter(|fp| !vectorcraft_text::retype::is_combining_mark(fp.ch))
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
        tracking: 0.0,
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

/// A new line of shape `pc` alone, as the first of its candidate glyphs `cands` that is not a
/// combining mark (none: `None`), with near matches taken at tolerance `slack`. No line starts
/// with a mark: a mark combines with the character before it.
fn new_line(pc: &Piece, cands: &[GlyphMatch], slack: f64) -> Option<Line> {
    let m = cands.iter().find(|m| !vectorcraft_text::retype::is_combining_mark(m.ch))?;
    let dir = Vec2::new(m.angle.cos(), m.angle.sin());
    Some(Line {
        pieces: vec![pc.id],
        tracking: 0.0,
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

/// Complete splits of one shape that one [`split_parts`] search (exact or near) checks the paint
/// of, at most. When none of them is accepted, that search leaves the shape whole.
const MAX_SPLIT_TRIES: usize = 32;

/// A shape of more contour groups than this is not split: it is matched whole only.
const MAX_SPLIT_GROUPS: usize = 256;

/// A shape of more contours than this is not split either, and its contours are not grouped.
const MAX_SPLIT_CONTOURS: usize = 512;

/// Contours of all the selected shapes one command groups, at most: the shapes past them are
/// matched whole only.
const MAX_SPLIT_TOTAL_CONTOURS: usize = 16_384;

/// A shape of more path elements than this is not split, and its contours are not grouped: 16
/// times the most a glyph may have ([`vectorcraft_text::retype::MAX_ELEMENTS`]).
const MAX_SPLIT_ELEMENTS: usize = 16 * vectorcraft_text::retype::MAX_ELEMENTS;

/// `path`'s contour groups when it is split into glyphs: none for fewer than two or more than
/// [`MAX_SPLIT_GROUPS`], for more than [`MAX_SPLIT_CONTOURS`] contours or [`MAX_SPLIT_ELEMENTS`]
/// path elements, and for more contours than `budget` holds. A path of two or more contours takes
/// them from `budget` when it is grouped.
fn split_groups(path: &BezPath, budget: &mut usize) -> Vec<BezPath> {
    // Grouping copies the contours and compares every contour with every other: count first.
    let n = path.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
    if !(2..=MAX_SPLIT_CONTOURS).contains(&n) || n > *budget || path.elements().len() > MAX_SPLIT_ELEMENTS {
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
/// its groups nearly. A split of several parts is kept only when `together(runs, near)` accepts the
/// paths of its runs (the parts, set one beside another, paint what the shape paints); `near` is
/// set when the parts were found by near matching.
fn parts<M>(
    path: &BezPath,
    groups: &[BezPath],
    find: &dyn Fn(&BezPath, bool, bool) -> Vec<M>,
    together: &dyn Fn(&[BezPath], bool) -> bool,
) -> Vec<Vec<M>> {
    let whole = find(path, false, false);
    if !whole.is_empty() {
        return vec![whole];
    }
    let exact = split_parts(groups, &|run| find(run, true, false), &|runs| together(runs, false));
    if !exact.is_empty() {
        return exact;
    }
    let whole = find(path, false, true);
    if !whole.is_empty() {
        return vec![whole];
    }
    split_parts(groups, &|run| find(run, true, true), &|runs| together(runs, true))
}

/// [`parts`] of a shape split into contour `groups`, each run's candidates from `find`; empty for
/// no groups, when the groups cannot all be matched and when `together` rejects every split of
/// several parts it is asked about. The complete splits are tried with the longest runs first,
/// then with the shortest runs, then the others in order, a longer run at an earlier group first.
/// `together` is asked about at most [`MAX_SPLIT_TRIES`] splits. `find` is asked about a run only
/// when the search reaches it, and about each run at most once, so at most [`MAX_RUN`] times per
/// group.
fn split_parts<M>(groups: &[BezPath], find: &dyn Fn(&BezPath) -> Vec<M>, together: &dyn Fn(&[BezPath]) -> bool) -> Vec<Vec<M>> {
    let g = groups.len();
    let mut s = Splits { groups, find, found: (0..g).map(|_| (0..MAX_RUN).map(|_| None).collect()).collect(), whole: vec![None; g] };
    // A split is the runs it takes, each as (start, number of groups).
    let mut first = vec![];
    if g == 0 || !s.extend(&mut first, false) {
        return vec![];
    }
    let runs_of =
        |split: &[(usize, usize)]| -> Vec<BezPath> { split.iter().map(|&(at, n)| groups.get(at..at + n).map(joined).unwrap_or_default()).collect() };
    let accepted = 'search: {
        if first.len() == 1 {
            break 'search Some(first);
        }
        // The shortest runs are looked up only once the longest are rejected.
        let mut shortest = vec![];
        let mut shortest_next = true;
        let mut rest = first.clone();
        let mut candidate = Some(first.clone());
        let mut tried = 0;
        while let Some(split) = candidate.take() {
            if tried == MAX_SPLIT_TRIES {
                break;
            }
            tried += 1;
            if together(&runs_of(&split)) {
                break 'search Some(split);
            }
            if std::mem::take(&mut shortest_next) && s.extend(&mut shortest, true) && shortest != first {
                candidate = Some(shortest.clone());
                continue;
            }
            candidate = loop {
                if !s.advance(&mut rest) {
                    break None;
                }
                if rest != shortest {
                    break Some(rest.clone());
                }
            };
        }
        None
    };
    let Some(split) = accepted else { return vec![] };
    let mut out = vec![];
    for (at, n) in split {
        let Some(Some(cands)) = s.found.get_mut(at).and_then(|f| f.get_mut(n.wrapping_sub(1))).map(Option::take) else { return vec![] };
        out.push(cands);
    }
    out
}

/// The state of one [`split_parts`] search: each run's candidates and whether the groups from each
/// start can all be matched, each found when the search first needs it.
struct Splits<'a, M> {
    groups: &'a [BezPath],
    find: &'a dyn Fn(&BezPath) -> Vec<M>,
    /// By start and length less one: the run's candidates, once looked up.
    found: Vec<Vec<Option<Vec<M>>>>,
    /// By start: whether the groups from there on can all be matched, once known.
    whole: Vec<Option<bool>>,
}

impl<M> Splits<'_, M> {
    /// Whether the run of `n` groups at `at` matches, looking it up the first time.
    fn matches(&mut self, at: usize, n: usize) -> bool {
        let Some(slot) = self.found.get_mut(at).and_then(|f| f.get_mut(n.wrapping_sub(1))) else { return false };
        if slot.is_none() {
            let cands = self.groups.get(at..at + n).map(|run| (self.find)(&joined(run))).unwrap_or_default();
            *slot = Some(cands);
        }
        slot.as_ref().is_some_and(|c| !c.is_empty())
    }

    /// Whether the groups from `at` on can all be matched. The search goes from `at` forward, the
    /// longest run first, and no start is searched twice, so it ends after at most [`MAX_RUN`]
    /// lookups and checks per group.
    fn completes(&mut self, at: usize) -> bool {
        let g = self.groups.len();
        if at >= g {
            return at == g;
        }
        if let Some(known) = self.whole.get(at).copied().flatten() {
            return known;
        }
        // From `at`: the starts still to settle, each with the run length to try next.
        let mut stack: Vec<(usize, usize)> = vec![(at, MAX_RUN.min(g - at))];
        while let Some(&(i, n)) = stack.last() {
            if self.whole.get(i).copied().flatten().is_some() {
                stack.pop();
                continue;
            }
            if n == 0 {
                if let Some(w) = self.whole.get_mut(i) {
                    *w = Some(false);
                }
                stack.pop();
                continue;
            }
            if let Some(top) = stack.last_mut() {
                top.1 = n - 1;
            }
            if !self.matches(i, n) {
                continue;
            }
            let next = i + n;
            match if next == g { Some(true) } else { self.whole.get(next).copied().flatten() } {
                Some(true) => {
                    if let Some(w) = self.whole.get_mut(i) {
                        *w = Some(true);
                    }
                    stack.pop();
                }
                Some(false) => {}
                None => {
                    // Settle `next` first, then try this run again.
                    if let Some(top) = stack.last_mut() {
                        top.1 = n;
                    }
                    stack.push((next, MAX_RUN.min(g - next)));
                }
            }
        }
        self.whole.get(at).copied().flatten() == Some(true)
    }

    /// Whether `n` groups at `at` are a run a complete split can take.
    fn fits(&mut self, at: usize, n: usize) -> bool {
        self.matches(at, n) && self.completes(at + n)
    }

    /// Takes `split` on to the last group, with the longest run that fits at each start (or, with
    /// `shortest`, the shortest); false when no run fits.
    fn extend(&mut self, split: &mut Vec<(usize, usize)>, shortest: bool) -> bool {
        let g = self.groups.len();
        let mut i = split.last().map_or(0, |&(at, n)| at + n);
        while i < g {
            let most = MAX_RUN.min(g - i);
            let lens: Vec<usize> = if shortest { (1..=most).collect() } else { (1..=most).rev().collect() };
            let Some(n) = lens.into_iter().find(|&n| self.fits(i, n)) else { return false };
            split.push((i, n));
            i += n;
        }
        true
    }

    /// Moves `split` on to the next complete split, in order: the runs before the last start where
    /// a shorter run fits kept, that shorter run, and the longest runs after it. False past the last.
    fn advance(&mut self, split: &mut Vec<(usize, usize)>) -> bool {
        while let Some((at, n)) = split.pop() {
            if let Some(m) = (1..n).rev().find(|&m| self.fits(at, m)) {
                split.push((at, m));
                return self.extend(split, false);
            }
        }
        false
    }
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
        let found: Vec<Vec<Vec<GlyphMatch>>> = pieces
            .iter()
            .zip(&groups)
            .map(|(pc, g)| {
                if pc.keeps_outline {
                    return vec![];
                }
                let fill = pc.filled.then_some(pc.nonzero);
                // Parts that all match exactly are checked at tolerance 0, in the near phase as in
                // the exact one; only a part that matches nearly brings the tolerance in.
                // The glyphs of a split may become separate objects: when the shape does not paint
                // its overlaps as several objects would (translucent, another blend, a gradient or
                // pattern fill, a stroke), its parts must not overlap, which the parts read by the
                // even-odd rule against them all joined show.
                let once = paints_once(pc.opacity, pc.blend, &pc.appearance);
                let together = |runs: &[BezPath], near: bool| {
                    if fill.is_none() && once {
                        return true;
                    }
                    let nearly = near && runs.iter().any(|r| find(r, true, false, fill).is_empty());
                    let tol = if nearly { tolerance } else { 0.0 };
                    fill.is_none_or(|nz| vectorcraft_text::retype::parts_paint_alike(&pc.path, runs, nz, tol))
                        && (once || vectorcraft_text::retype::parts_paint_alike(&joined(runs), runs, false, tol))
                };
                parts(&pc.path, g, &|p, part, near| find(p, part, near, fill), &together)
            })
            .collect();
        // The face most shapes match, so a glyph several fonts share is set in the same font as
        // the glyphs around it. Among faces that as many shapes match, the one whose glyphs fit
        // them closest in all (the least leftover distance summed over those shapes), so every
        // part that fits it takes the same face, exact matches as near ones. Simplified outlines
        // of one font can nearly fit another font's look-alike glyphs, some of them more closely.
        // A part's matches are all exact or all near, so the distances a face sums compare alike.
        let mut votes: Vec<(u32, usize, f64)> = vec![];
        for ms in found.iter().flatten() {
            // Each shape votes once for each face it fits, with the leftover distance of its
            // closest glyph in that face.
            let mut faces: Vec<(u32, f64)> = vec![];
            for m in ms {
                let residual = if m.residual.is_finite() { m.residual } else { f64::INFINITY };
                match faces.iter_mut().find(|(f, _)| *f == m.face.id()) {
                    Some(f) => f.1 = f.1.min(residual),
                    None => faces.push((m.face.id(), residual)),
                }
            }
            for (f, residual) in faces {
                match votes.iter_mut().find(|(v, ..)| *v == f) {
                    Some(v) => {
                        v.1 += 1;
                        v.2 += residual;
                    }
                    None => votes.push((f, 1, residual)),
                }
            }
        }
        let rank = |face: u32| votes.iter().find(|(f, ..)| *f == face).map_or((0, f64::INFINITY), |v| (v.1, v.2));
        // Most votes, then the least summed distance, then the first (the matches come upright and
        // closest first).
        let ranked = |(i, a): &(usize, &GlyphMatch), (j, b): &(usize, &GlyphMatch)| {
            let ((va, da), (vb, db)) = (rank(a.face.id()), rank(b.face.id()));
            va.cmp(&vb).then(db.total_cmp(&da)).then(j.cmp(i))
        };
        let mut lines: Vec<Line> = vec![];
        let mut unmatched = vec![];
        // An unmatched shape between glyphs ends the line: the type after it is painted after it.
        let mut open = false;
        let mut spacing: std::collections::HashMap<PartKey, GlyphMatch> = std::collections::HashMap::new();
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
                // The part's candidates in the face most shapes fit (see `ranked`).
                let face = ms.iter().enumerate().max_by(ranked).map(|(_, m)| m.face.id());
                let cands: Vec<GlyphMatch> = ms.iter().filter(|m| Some(m.face.id()) == face).cloned().collect();
                let joined = match lines.last_mut() {
                    Some(l) if open => join(l, pc, &cands),
                    _ => false,
                };
                // A part that joined a line, perhaps as a mark, that is also another character, in
                // that face or else in the face most shapes fit of those it is a character in: the
                // character, should a split leave the part first on a line ([`verified`]). Kept by
                // the glyph the part joined as, since the parts of a compound path share its id.
                let is_mark = |m: &GlyphMatch| vectorcraft_text::retype::is_combining_mark(m.ch);
                if joined
                    && cands.iter().any(is_mark)
                    && let Some(m) = cands
                        .iter()
                        .find(|m| !is_mark(m))
                        .or_else(|| ms.iter().enumerate().filter(|(_, m)| !is_mark(m)).max_by(ranked).map(|(_, m)| m))
                    && let Some(at) = lines.last().and_then(|l| l.matches.last())
                {
                    spacing.insert(part_key(pc.id, at), m.clone());
                }
                if !joined {
                    // A part that is only marks in that face may be another character in a face
                    // fewer shapes fit: it starts its line in the one of those most shapes fit.
                    let started = new_line(pc, &cands, tolerance).or_else(|| {
                        let face = ms.iter().enumerate().filter(|(_, m)| !is_mark(m)).max_by(ranked).map(|(_, m)| m.face.id());
                        let cands: Vec<GlyphMatch> = ms.iter().filter(|m| Some(m.face.id()) == face).cloned().collect();
                        new_line(pc, &cands, tolerance)
                    });
                    match started {
                        Some(l) => lines.push(l),
                        // Only combining marks: no line starts with one, so the shape stays an
                        // outline (a shape of several glyphs then is not whole type, see
                        // `partial_lines`) and the glyph after it starts the next line.
                        None => {
                            unmatched.push(pc.id);
                            open = false;
                            continue;
                        }
                    }
                }
                open = true;
            }
        }
        // A lone one-contour glyph (a period, a hyphen, a bullet) is too plain to tell from drawn
        // art (a dot, a bar): beside other shapes, the other glyphs of a split compound path
        // included, it stays an outline. Selected on its own, it is what was asked for. Lone means
        // alone on its baseline as the outlines lie, before the layout check splits lines (tracking
        // and manual kerning split them glyph by glyph).
        let lone_pieces: std::collections::HashSet<NodeId> = lines.iter().flat_map(Line::lone_one_contour).collect();
        // Each line's tracking is measured with nine layouts at most: the line as read (its
        // characters), its glyphs without and with one space between them (three times its
        // glyphs) and six tracked readings of at most 64 spaces in each gap (390 times its
        // glyphs), so every line has what its measure can take.
        let mut budget =
            lines.iter().map(|l| l.text.chars().count().saturating_add(l.pieces.len().saturating_mul(393))).fold(256usize, usize::saturating_add);
        let mut lines: Vec<Line> = lines.into_iter().map(|l| l.tracked(system_layout, &mut budget)).collect();
        share_tracking(&mut lines, system_layout);
        let mut lines = verified(lines, system_layout, &spacing, &mut unmatched);
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
        let at: std::collections::HashMap<NodeId, usize> = pieces.iter().enumerate().map(|(i, pc)| (pc.id, i)).collect();
        // Lines split from one compound path share its shape, with nothing between them.
        let follows = |a: NodeId, b: NodeId| match (at.get(&a), at.get(&b)) {
            (Some(i), Some(j)) => *j == *i || (*j == i + 1 && pieces.get(*j).is_some_and(|pc| !pc.after_barrier)),
            _ => false,
        };
        // Joining lines takes at most four tries' work per glyph (see [`Block::made`]), with room
        // for small blocks.
        let mut budget = lines.iter().map(|l| l.pieces.len()).fold(0usize, usize::saturating_add).saturating_mul(24).saturating_add(256);
        (blocks(lines, follows, system_layout, &mut budget), unmatched, truncated)
    };
    if lines.is_empty() {
        return Err(EngineError::Other(format!("Retype: no selected shape matches a glyph of an available font ({} checked)", unmatched.len())));
    }
    let made = s.edit("Retype", |d, sel| {
        // The layers as they were, for the Text Wrap check below.
        let before = lines.iter().any(|l| l.point.is_some()).then(|| d.clone());
        let mut ids = vec![];
        let mut emptied = vec![];
        // Every line's type goes in before the shapes go: a shape of several glyphs is the first
        // shape of more than one line. Each goes just before its first shape, so lines from one
        // shape keep their order.
        for l in &lines {
            let Some(first) = l.pieces.first() else { continue };
            let (par, idx, _) = d.position(*first).ok_or(EngineError::NoNode(*first))?;
            let id = d.alloc_id();
            let mut node = Node::new(id, NodeKind::Text(Box::new(l.t.clone())));
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
        // Text Wrap objects above new area type flow it once the edit is done: area type they
        // would move a glyph of becomes the point type that draws it in place.
        if let Some(before) = &before {
            // Worked out on a copy, so the edit's own Text Wrap refresh still sees every other
            // area type whose wrap changes, and only on the layers this edit changed, as that
            // refresh is. The copy holds only the changed layers while the work is counted; past
            // the limit, the new area type is not flowed and all of it becomes its point type.
            let mut probe = d.clone();
            let all = std::mem::take(&mut probe.layers);
            probe.layers = all
                .iter()
                .enumerate()
                .filter(|(i, layer)| {
                    // As resolve_wrap tells an unchanged layer: the same layer at its index first.
                    !(before.layers.get(*i).is_some_and(|b| Arc::ptr_eq(b, layer))
                        || before.layers.iter().any(|b| b.id == layer.id && Arc::ptr_eq(b, layer)))
                })
                .map(|(_, layer)| layer.clone())
                .collect();
            let units = vectorcraft_doc::text::wrap_copy_units(&probe);
            let flowed = units <= max_wrap_work();
            probe.layers = all;
            if flowed && units > 0 {
                vectorcraft_doc::text::resolve_wrap(Some(before), &mut probe);
            }
            // With nothing to copy, no area type on those layers has a wrap shape.
            for (l, id) in lines.iter().zip(&ids).filter(|_| units > 0) {
                let Some(point) = &l.point else { continue };
                let Some(NodeKind::Text(t)) = probe.node(*id).map(|n| &n.kind) else { continue };
                if flowed && (t.wrap.is_empty() || first_mismatch(t, &l.matches, l.size, l.slack).is_none()) {
                    continue;
                }
                if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                    **t = point.clone();
                }
            }
        }
        let mut keep: Vec<NodeId> = roots.iter().copied().filter(|r| d.node(*r).is_some()).collect();
        keep.extend(&ids);
        sel.set(keep);
        Ok(ids)
    })?;
    let d = &s.doc()?.doc;
    let area =
        |id: &NodeId| matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }));
    Ok(json!({
        "ids": made.iter().map(|i| i.0).collect::<Vec<_>>(),
        "texts": lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
        "fonts": lines
            .iter()
            .zip(&made)
            .map(|(l, id)| {
                let score = l.matches.iter().map(GlyphMatch::score).fold(1.0, f64::min);
                json!({
                    "family": l.face.family,
                    "style": l.face.style,
                    "size": l.size,
                    "tracking": l.tracking,
                    "leading": l.leading,
                    "area": area(id),
                    "score": (score * 1000.0).round() / 1000.0,
                })
            })
            .collect::<Vec<_>>(),
        "unmatched": unmatched.iter().map(|i| i.0).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
#[path = "retype_tests.rs"]
mod tests;
