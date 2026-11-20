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
//! that match no font stay as they are and are reported. Near matches (another version of the
//! font, simplified outlines, merged glyphs) and images are phase 2.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use kurbo::{BezPath, Point, Vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeId, NodeKind, TextObject};
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
        "{fonts?: [family…]} turn the selected outlined text (glyph paths and compound paths, inside groups too) back into live type: each shape is matched against the glyphs of the fonts the app has loaded, the document's fonts and the families in fonts, the same up to uniform scale, rotation and translation (exact matches only); matched glyphs become point text in that font and size at the same baseline, one object per line unless lines stack into one (below), with the first glyph's fill, stroke, opacity and blend mode, in one undo step; a line of three or more glyphs takes the spacing most of its gaps share as its tracking (pair kerning left out) when tracked type draws all of it in place, or more of it than untracked type (when untracked type also draws it, untracked type is kept if the tracking is under one unit or the tracked type would drop a gap one space wide (a b c keeps its spaces), and the tracked type otherwise), and negative tracking only when the line is opaque, Normal, with a solid fill or none and no stroke (its glyphs may overlap); lines that follow each other in paint order with the same group, paint, font, size, turn and tracking (an untracked line of one or two glyphs takes the others' tracking), each one the same leading (0.25 to 3 times the size) below the one before, and lined up at their starts, ends or centers become one area type object with that leading (Auto when Auto gives it) and left, right or center alignment, or one multi-line point type object when area type (after Text Wrap) would not draw the outlines' glyphs in their places or when checking the Text Wrap objects on the changed layers would copy more than 2,000,000 anchors, subpaths and shapes; where even that type would move a glyph, the lines before that glyph's line stay one object and the rest are joined again from that line; lines with an opacity below 100%, a blend mode other than Normal, a gradient or pattern fill or a stroke stay one object each; a line splits where its type would not draw the outlines' glyphs in their places (uneven spacing, manual kerning, font substitutions); shapes that match no font, glyphs type cannot draw as they are, and shapes with their own opacity mask stay as they are → {ids (the new type), texts (lines joined by \\n), fonts: [{family, style, size, tracking (1/1000 em), leading (pt, null for Auto or one line), area (bool)}], unmatched: [id…], truncated (shapes past the limits of 2048 shapes or 16 nested groups were left out)}; an error when no shape matches",
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
    let piece = |path: BezPath, after_barrier: bool| Piece {
        id: n.id,
        parent,
        after_barrier,
        path,
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
        NodeKind::Path { path, clipping: false, guide: false, .. } if path.is_closed() => out.push(piece(path.to_bezpath(), std::mem::take(barrier))),
        NodeKind::Compound { children, .. } => {
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
            out.push(piece(path, std::mem::take(barrier)));
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
}

impl Line {
    fn dir(&self) -> Vec2 {
        Vec2::new(self.angle.cos(), self.angle.sin())
    }

    /// Does glyph `m` of shape `pc` continue this line (same group, paint and transparency, same
    /// font, size and turn, on its baseline, after its end)?
    fn continues(&self, pc: &Piece, m: &GlyphMatch) -> bool {
        let size = self.scale * self.face.units_per_em();
        let d = m.origin - self.origin;
        let dir = self.dir();
        let across = d.cross(dir).abs();
        pc.parent == self.parent
            && pc.appearance == self.appearance
            && pc.opacity == self.opacity
            && pc.blend == self.blend
            && m.face.id() == self.face.id()
            // One size and turn for the whole line: the type sets every glyph at the first one's.
            && (m.scale - self.scale).abs() <= self.scale * 1e-4
            // Wrapped: type turned 180° has angles near +π and −π.
            && ((m.angle - self.angle + PI).rem_euclid(TAU) - PI).abs() < 1e-5
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
        // A gap, past the tracking, wider than half a space before the glyph is spaces the
        // outlines left no shape for. Each space takes the tracking too.
        let dir = self.dir();
        let gap = (m.origin - self.end).dot(dir) - self.tracking_pt();
        // Tracking tighter than a space's width gives each space a negative advance.
        let space = space_advance(&self.face) * self.scale + self.tracking_pt();
        let spaces = if space.abs() > 1e-9 { gap / space } else { 0.0 };
        if spaces.is_finite() && spaces > 0.5 {
            let n = (spaces.round() as usize).clamp(1, 64);
            self.text.extend(std::iter::repeat_n(' ', n));
        }
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
        first_mismatch(t, &self.matches, self.size())
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
    /// -1000..10000, and when its tracking is negative and the line does not paint the same as one
    /// object ([`paints_as_one`]: its glyphs may overlap). Each of the nine layouts at most takes
    /// its characters, spaces included, from `budget` before it runs; without enough left, the
    /// line stays as it is, or becomes the best tracked reading laid out before the budget ran out.
    fn tracked(self, system_layout: bool, budget: &mut usize) -> Line {
        let size = self.size();
        let n = self.pieces.len();
        if n < 3 || size <= 0.0 {
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
            for t in [round_tracking(measured), measured] {
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
                (None, None) if t.abs() < 1.0 || drops_a_word_space => return self,
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
    l.opacity == 1.0
        && l.blend == vectorcraft_color::BlendMode::Normal
        && matches!(l.appearance.fill_paint(), vectorcraft_color::Paint::None | vectorcraft_color::Paint::Solid { .. })
        && matches!(l.appearance.stroke_paint(), vectorcraft_color::Paint::None)
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
fn first_mismatch(t: &TextObject, matches: &[GlyphMatch], size: f64) -> Option<usize> {
    let lay = vectorcraft_text::layout(FontDb::global(), t);
    let mut drawn = lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty());
    let tol = (size * 0.01).max(0.01);
    for (k, m) in matches.iter().enumerate() {
        let Some(g) = drawn.next() else { return Some(k) };
        let at = t.xf * (g.xf * Point::ORIGIN);
        if g.font_id != m.face.id() || g.gid != m.gid || at.distance(m.origin) > tol {
            return Some(k);
        }
    }
    // Ink the outlines did not have: the last glyph is where it went wrong.
    drawn.next().map(|_| matches.len().saturating_sub(1))
}

/// `lines` checked against their own layout: a line splits where its type stops drawing the
/// outlines, and a glyph its type cannot draw even on its own stays an outline (`unmatched`).
/// The glyphs laid out are bounded by a multiple of the number of glyphs.
fn verified(lines: Vec<Line>, system_layout: bool, unmatched: &mut Vec<NodeId>) -> Vec<Line> {
    // Glyphs laid out in all, at most: past it, the glyphs left unchecked stay outlines.
    let mut budget = lines.iter().map(|l| l.pieces.len()).sum::<usize>().saturating_mul(16).saturating_add(256);
    // Measuring the tracking of the glyphs after a split has a budget of its own, so it cannot
    // leave glyphs unchecked; past it they keep no tracking. Each measure takes the glyphs to
    // rebuild the rest and the characters it lays out (see [`Line::tracked`]); the budget covers
    // one measure of each whole line, as `retype` gives the first, and the glyph checks' budget.
    // The glyphs measured again after splits are limited apart from it, at six times each rest's
    // glyphs from half as much again as the glyph checks' budget.
    let mut remeasured = budget.saturating_add(budget / 2);
    let mut track_budget =
        lines.iter().map(|l| l.text.chars().count().saturating_add(l.pieces.len().saturating_mul(394))).fold(budget, usize::saturating_add);
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
                // may still be drawn as it was (a contextual form changed it in the line). The
                // rest takes the tracking its own glyphs are spaced by, while the budget for
                // measuring it lasts and when type draws its first glyph as it was, followed by the
                // next two (else that glyph splits off first, and the tracking is measured from the
                // next one); past it, an untracked line's rest keeps its text as it is.
                Some(k) => {
                    let k = k.max(1);
                    // A measure takes at least six times the rest's glyphs from each budget.
                    let measurable = n - k >= 3 && remeasured >= (n - k).saturating_mul(6) && track_budget >= (n - k).saturating_mul(6);
                    let first_drawn = measurable && {
                        budget = budget.saturating_sub(3);
                        l.respaced(k, k + 3, l.tracking).is_some_and(|p| p.first_mismatch(&p.probe(system_layout)) != Some(0))
                    };
                    let rest = if first_drawn {
                        // Rebuilding the rest for the measure takes its glyphs from the budget.
                        remeasured = remeasured.saturating_sub((n - k).saturating_mul(6));
                        track_budget = track_budget.saturating_sub(n - k);
                        l.respaced(k, n, 0.0).map(|r| r.tracked(system_layout, &mut track_budget))
                    } else if l.tracking == 0.0 {
                        l.part(k, n)
                    } else {
                        l.respaced(k, n, 0.0)
                    };
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
        if let Some(k) = first_mismatch(&t, &matches, size) {
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
        if in_range(&area) && first_mismatch(&area, &matches, size).is_none() {
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
/// shape `a`, with nothing between them in paint order. `budget`: see [`Block::made`].
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
    }
}

/// A new line of shape `pc` alone, as the first of its candidate glyphs `cands` (none: `None`).
fn new_line(pc: &Piece, cands: &[GlyphMatch]) -> Option<Line> {
    let m = cands.first()?;
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
    })
}

fn retype(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.retype";
    let fonts: Vec<String> = match p.get("fonts") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(|| bad(C, "fonts must be a list of family names"))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(C, "fonts must be a list of family names")),
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
        // Only glyphs with a selected shape's key (segments per contour) can match one: the others are
        // not sampled.
        let wanted: std::collections::BTreeSet<u64> =
            pieces.iter().filter(|pc| !pc.keeps_outline).filter_map(|pc| vectorcraft_text::retype::shape_key(&pc.path)).collect();
        // Nothing to match (every shape stays an outline): no font is read.
        let index = if wanted.is_empty() { RetypeIndex::default() } else { RetypeIndex::build(db, candidate_faces(db, d, &fonts), &wanted) };
        let found: Vec<Vec<GlyphMatch>> = pieces.iter().map(|pc| if pc.keeps_outline { vec![] } else { index.matches(&pc.path) }).collect();
        // The face most shapes match, so a glyph several fonts share is set in the same font as
        // the glyphs around it.
        let mut votes: Vec<(u32, usize)> = vec![];
        for ms in &found {
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
        for (pc, ms) in pieces.iter().zip(&found) {
            if pc.after_barrier {
                open = false;
            }
            // The shape's candidates in the face most shapes fit (the first such face, as the
            // matches come upright and closest first).
            let face = ms.iter().enumerate().max_by_key(|(i, m)| (rank(m.face.id()), std::cmp::Reverse(*i))).map(|(_, m)| m.face.id());
            let cands: Vec<GlyphMatch> = ms.iter().filter(|m| Some(m.face.id()) == face).cloned().collect();
            if cands.is_empty() {
                unmatched.push(pc.id);
                open = false;
                continue;
            }
            let joined = match lines.last_mut() {
                Some(l) if open => join(l, pc, &cands),
                _ => false,
            };
            if !joined {
                lines.extend(new_line(pc, &cands));
            }
            open = true;
        }
        // A lone one-contour glyph (a period, a hyphen, a bullet) is too plain to tell from drawn
        // art (a dot, a bar): beside other shapes it stays an outline. Selected on its own, it is
        // what was asked for. Lone means alone on its baseline as the outlines lie, before the
        // layout check splits lines (tracking and manual kerning split them glyph by glyph).
        let lone_pieces: std::collections::HashSet<NodeId> = lines.iter().flat_map(Line::lone_one_contour).collect();
        // Each line's tracking is measured with nine layouts at most: the line as read (its
        // characters), its glyphs without and with one space between them (three times its
        // glyphs) and six tracked readings of at most 64 spaces in each gap (390 times its
        // glyphs), so every line has what its measure can take.
        let mut budget =
            lines.iter().map(|l| l.text.chars().count().saturating_add(l.pieces.len().saturating_mul(393))).fold(256usize, usize::saturating_add);
        let mut lines: Vec<Line> = lines.into_iter().map(|l| l.tracked(system_layout, &mut budget)).collect();
        share_tracking(&mut lines, system_layout);
        let mut lines = verified(lines, system_layout, &mut unmatched);
        if pieces.len() > 1 {
            lines.retain(|l| {
                let lone = matches!(l.pieces.as_slice(), [id] if lone_pieces.contains(id));
                if lone {
                    unmatched.extend(l.pieces.iter().copied());
                }
                !lone
            });
        }
        let at: std::collections::HashMap<NodeId, usize> = pieces.iter().enumerate().map(|(i, pc)| (pc.id, i)).collect();
        let follows = |a: NodeId, b: NodeId| match (at.get(&a), at.get(&b)) {
            (Some(i), Some(j)) => *j == i + 1 && pieces.get(*j).is_some_and(|pc| !pc.after_barrier),
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
        for l in &lines {
            let Some(first) = l.pieces.first() else { continue };
            let (par, idx, _) = d.position(*first).ok_or(EngineError::NoNode(*first))?;
            let id = d.alloc_id();
            let mut node = Node::new(id, NodeKind::Text(Box::new(l.t.clone())));
            // The glyphs' own transparency (the group's, kept, still applies over it).
            node.opacity = l.opacity;
            node.blend = l.blend;
            d.insert(par, idx, node)?;
            for pc in &l.pieces {
                if let Some((Some(parent), _, _)) = d.position(*pc) {
                    emptied.push(parent);
                }
                d.remove(*pc)?;
            }
            ids.push(id);
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
                if flowed && (t.wrap.is_empty() || first_mismatch(t, &l.matches, l.size).is_none()) {
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
            .map(|(l, id)| json!({
                "family": l.face.family,
                "style": l.face.style,
                "size": l.size,
                "tracking": l.tracking,
                "leading": l.leading,
                "area": area(id),
            }))
            .collect::<Vec<_>>(),
        "unmatched": unmatched.iter().map(|i| i.0).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
#[path = "retype_tests.rs"]
mod tests;
