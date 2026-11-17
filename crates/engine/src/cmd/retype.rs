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
        "{fonts?: [family…]} turn the selected outlined text (glyph paths and compound paths, inside groups too) back into live point text: each shape is matched against the glyphs of the fonts the app has loaded, the document's fonts and the families in fonts, the same up to uniform scale, rotation and translation (exact matches only); matched glyphs become point text in that font and size at the same baseline, one object per line, with the first glyph's fill and stroke, in one undo step; shapes that match no font stay as they are → {ids (the new type), texts, fonts: [{family, style, size}], unmatched: [id…]}; an error when no shape matches",
        has_selected_path,
        retype
    )]
}

/// A shape to match: a path or compound path, in document space.
struct Piece {
    id: NodeId,
    /// The group holding it (`None` for a selected shape at the top): glyphs of different
    /// groups, which may composite differently, never share a line.
    parent: Option<NodeId>,
    path: BezPath,
    appearance: Appearance,
    /// The shape's own transparency, which the new type takes.
    opacity: f32,
    blend: vectorcraft_color::BlendMode,
}

fn collect(n: &Node, parent: Option<NodeId>, depth: usize, out: &mut Vec<Piece>) {
    if out.len() >= MAX_PIECES || depth > MAX_DEPTH {
        return;
    }
    let piece = |path: BezPath| Piece { id: n.id, parent, path, appearance: (*n.appearance).clone(), opacity: n.opacity, blend: n.blend };
    match &n.kind {
        NodeKind::Group { children, clip: false } => {
            for c in children {
                collect(c, Some(n.id), depth + 1, out);
            }
        }
        NodeKind::Path { path, clipping: false, guide: false, .. } => out.push(piece(path.to_bezpath())),
        NodeKind::Compound { children, .. } => {
            let mut path = BezPath::new();
            for c in children {
                if let NodeKind::Path { path: p, .. } = &c.kind {
                    path.extend(p.to_bezpath());
                }
            }
            out.push(piece(path));
        }
        _ => {}
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
        self.end = m.origin + dir * (m.face.advance(m.gid) * m.scale);
    }
}

fn space_advance(face: &vectorcraft_text::FontFace) -> f64 {
    face.mapped_chars().iter().find(|(c, _)| *c == ' ').map(|(_, g)| face.advance(*g)).unwrap_or(0.0)
}

fn new_line(pc: &Piece, m: &GlyphMatch) -> Line {
    let dir = Vec2::new(m.angle.cos(), m.angle.sin());
    Line {
        pieces: vec![pc.id],
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
    let (lines, unmatched) = {
        let d = &s.doc()?.doc;
        let mut pieces = vec![];
        for r in &roots {
            if let Some(n) = d.node(*r) {
                collect(n, None, 0, &mut pieces);
            }
        }
        if pieces.is_empty() {
            return Err(EngineError::Other("Retype: select outlined text (paths or compound paths)".into()));
        }
        let index = RetypeIndex::build(db, candidate_faces(db, d, &fonts));
        let found: Vec<Vec<GlyphMatch>> = pieces.iter().map(|pc| index.matches(&pc.path)).collect();
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
        (lines, unmatched)
    };
    if lines.is_empty() {
        return Err(EngineError::Other(format!("Retype: no selected shape matches a glyph of an available font ({} checked)", unmatched.len())));
    }
    let system_layout = s.doc()?.doc.setup.system_layout;
    let made = s.edit("Retype", |d, sel| {
        let mut ids = vec![];
        let mut emptied = vec![];
        for l in &lines {
            let Some(first) = l.pieces.first() else { continue };
            let (par, idx, _) = d.position(*first).ok_or(EngineError::NoNode(*first))?;
            let style = CharStyle {
                font_family: l.face.family.clone(),
                font_style: l.face.style.clone(),
                // The version that matched, when two of the family and style are loaded.
                font_version: (!l.face.version.is_empty()).then(|| l.face.version.clone()),
                size: l.scale * l.face.units_per_em(),
                fill: l.appearance.fill_paint(),
                stroke: l.appearance.stroke_paint(),
                stroke_width: l.appearance.stroke_width(),
                ..Default::default()
            };
            let mut t = TextObject::point(l.origin, &l.text, style);
            if l.angle != 0.0 {
                t.xf *= kurbo::Affine::rotate(l.angle);
            }
            t.para = super::create::new_type_para();
            t.system_layout = system_layout;
            super::typecmd::refresh_bounds(&mut t);
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
    }))
}

#[cfg(test)]
#[path = "retype_tests.rs"]
mod tests;
