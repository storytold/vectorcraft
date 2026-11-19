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
//! they are and are reported. Near matches (another version of the font, simplified outlines,
//! merged glyphs) and images are phase 2.

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
        "{fonts?: [family…]} turn the selected outlined text (glyph paths and compound paths, inside groups too) back into live point text: each shape is matched against the glyphs of the fonts the app has loaded, the document's fonts and the families in fonts, the same up to uniform scale, rotation and translation (exact matches only); matched glyphs become point text in that font and size at the same baseline, one object per line, with the first glyph's fill, stroke, opacity and blend mode, in one undo step; a line splits where its type would not draw the outlines' glyphs in their places (tracking, manual spacing, font substitutions); shapes that match no font, glyphs type cannot draw as they are, and shapes with their own opacity mask stay as they are → {ids (the new type), texts, fonts: [{family, style, size}], unmatched: [id…], truncated (shapes past the limits of 2048 shapes or 16 nested groups were left out)}; an error when no shape matches",
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
        let tol = (self.scale * self.face.units_per_em() * 0.01).max(0.01);
        for (k, m) in self.matches.iter().enumerate() {
            let Some(g) = drawn.next() else { return Some(k) };
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
    }
}

/// A new line of shape `pc` alone, as the first of its candidate glyphs `cands` (none: `None`).
fn new_line(pc: &Piece, cands: &[GlyphMatch]) -> Option<Line> {
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
        let lone_pieces: std::collections::HashSet<NodeId> =
            lines.iter().filter(|l| matches!(l.matches.as_slice(), [m] if m.contours == 1)).flat_map(|l| l.pieces.iter().copied()).collect();
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
        (lines, unmatched, truncated)
    };
    if lines.is_empty() {
        return Err(EngineError::Other(format!("Retype: no selected shape matches a glyph of an available font ({} checked)", unmatched.len())));
    }
    let made = s.edit("Retype", |d, sel| {
        let mut ids = vec![];
        let mut emptied = vec![];
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
        let mut keep: Vec<NodeId> = roots.iter().copied().filter(|r| d.node(*r).is_some()).collect();
        keep.extend(&ids);
        sel.set(keep);
        Ok(ids)
    })?;
    Ok(json!({
        "ids": made.iter().map(|i| i.0).collect::<Vec<_>>(),
        "texts": lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>(),
        "fonts": lines.iter().map(|l| json!({"family": l.face.family, "style": l.face.style, "size": l.scale * l.face.units_per_em()})).collect::<Vec<_>>(),
        "unmatched": unmatched.iter().map(|i| i.0).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

#[cfg(test)]
#[path = "retype_tests.rs"]
mod tests;
