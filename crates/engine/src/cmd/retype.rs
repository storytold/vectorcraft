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
    /// The shape has an opacity mask of its own, which type cannot take: it stays an outline.
    masked: bool,
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
    let piece = |path: BezPath, after_barrier: bool| Piece {
        id: n.id,
        parent,
        after_barrier,
        path,
        appearance: (*n.appearance).clone(),
        opacity: n.opacity,
        blend: n.blend,
        masked: n.extras().mask.is_some(),
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
                    NodeKind::Path { path: p, .. } if p.is_closed() => path.extend(p.to_bezpath()),
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
    let mut faces = vec![];
    for f in &families {
        for style in db.styles(f) {
            if let Some(face) = db.face(f, &style) {
                faces.push(face);
            }
        }
    }
    faces.extend(db.loaded_faces());
    faces
}

/// One line of retyped glyphs.
struct Line {
    pieces: Vec<NodeId>,
    /// What each piece matched, as `pieces`.
    matches: Vec<GlyphMatch>,
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
            && (m.scale - self.scale).abs() <= self.scale * 0.005
            && (m.angle - self.angle).abs() < 1e-3
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
/// The layouts are bounded by the number of glyphs.
fn verified(lines: Vec<Line>, system_layout: bool, unmatched: &mut Vec<NodeId>) -> Vec<Line> {
    let mut budget = lines.iter().map(|l| l.pieces.len() * 2 + 2).sum::<usize>();
    let mut todo: std::collections::VecDeque<Line> = lines.into();
    let mut out = vec![];
    while let Some(l) = todo.pop_front() {
        if budget == 0 {
            // Unchecked glyphs stay outlines.
            unmatched.extend(l.pieces);
            continue;
        }
        budget -= 1;
        let n = l.pieces.len();
        match l.first_mismatch(&l.text_object(system_layout)) {
            None => out.push(l),
            Some(0) => {
                unmatched.extend(l.pieces.first().copied());
                if let Some(rest) = l.part(1, n) {
                    todo.push_front(rest);
                }
            }
            Some(k) => {
                for part in [l.part(k, n), l.part(0, k)].into_iter().flatten() {
                    todo.push_front(part);
                }
            }
        }
    }
    out
}

fn space_advance(face: &vectorcraft_text::FontFace) -> f64 {
    face.mapped_chars().iter().find(|(c, _)| *c == ' ').map(|(_, g)| face.advance(*g)).unwrap_or(0.0)
}

fn new_line(pc: &Piece, m: &GlyphMatch) -> Line {
    let dir = Vec2::new(m.angle.cos(), m.angle.sin());
    Line {
        pieces: vec![pc.id],
        matches: vec![m.clone()],
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
    }
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
        let index = RetypeIndex::build(db, candidate_faces(db, d, &fonts));
        let found: Vec<Vec<GlyphMatch>> = pieces.iter().map(|pc| if pc.masked { vec![] } else { index.matches(&pc.path) }).collect();
        // The face most shapes match, so a glyph several fonts share is set in the same font as
        // the glyphs around it.
        let mut votes: Vec<(u32, usize)> = vec![];
        for m in found.iter().flatten() {
            match votes.iter_mut().find(|(f, _)| *f == m.face.id()) {
                Some(v) => v.1 += 1,
                None => votes.push((m.face.id(), 1)),
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
            // Earlier matches win ties: they are the closer fits.
            let best = ms.iter().enumerate().max_by_key(|(i, m)| (rank(m.face.id()), std::cmp::Reverse(*i))).map(|(_, m)| m);
            let Some(m) = best else {
                unmatched.push(pc.id);
                open = false;
                continue;
            };
            match lines.last_mut() {
                Some(l) if open && l.continues(pc, m) => l.push(pc.id, m),
                _ => lines.push(new_line(pc, m)),
            }
            open = true;
        }
        let lines = verified(lines, system_layout, &mut unmatched);
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
                && n.extras().mask.is_none();
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
