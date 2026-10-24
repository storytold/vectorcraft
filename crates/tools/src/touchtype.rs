//! The Touch Type tool (Shift+T): a click on a character of a type object selects that one
//! character and shows a box around it. Dragging the box, or its lower-left handle, moves the
//! character along its line and up or down. The upper-left handle scales it vertically, the
//! lower-right handle scales it horizontally, the upper-right handle scales it proportionally, and
//! the handle in the middle of the top edge rotates it. Every drag previews one `text.touchType`
//! (kerning, baseline shift, horizontal and vertical scale, character rotation) and commits it as
//! one undo step; the type stays editable. Escape lets go of the character. Behavior:
//! <https://web.archive.org/web/20261007185617/https://helpx.adobe.com/illustrator/using/tool-techniques/touch-type-tool.html>
//! and <https://web.archive.org/web/20260926095922/https://helpx.adobe.com/illustrator/desktop/design-with-text/add-manage-text/add-text-illustrator-text-tools.html>.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{CharStyle, NodeId, NodeKind, TextObject};
use vectorcraft_geom::bez::unit;
use vectorcraft_geom::{Affine, Point, Vec2};
use vectorcraft_text::{FontDb, PositionedGlyph, TextLayout, edit};

use crate::distort::perspective::in_quad;
use crate::xform::BLUE;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "touchType";

/// Create the Touch Type tool by id (None = not ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    (id == ID).then(|| Box::new(TouchTypeTool::default()) as Box<dyn Tool>)
}

/// What a drag changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// The box or its lower-left handle: kerning and baseline shift.
    Move,
    /// The upper-left handle: vertical scale.
    VScale,
    /// The lower-right handle: horizontal scale.
    HScale,
    /// The upper-right handle: both scales alike.
    Scale,
    /// The handle in the middle of the top edge: character rotation.
    Rotate,
}

/// A character's box, in text space.
#[derive(Clone, Debug)]
struct CharBox {
    /// The character's (its cluster's) first byte in the plain text.
    start: usize,
    /// Its box: the boxes of its glyphs ([`glyph_quad`]) together.
    quad: [Point; 4],
    /// The glyph's origin on its shifted baseline, which the scale handles measure from.
    pen: Point,
    /// Its pen position on the line's baseline, which character rotation turns it about.
    pivot: Point,
    /// A unit vector along its line in reading order.
    along: Vec2,
    /// The glyph's own x axis and its up axis (with its rotation, which baseline shift moves it
    /// along), unit vectors.
    ex: Vec2,
    ey: Vec2,
    /// Whether the tool moves it along its line by kerning: a character comes before it on its
    /// line and it is not right-to-left.
    kerned: bool,
    /// Its style as the Character panel shows it (horizontal scale with the object transform).
    style: CharStyle,
}

impl CharBox {
    /// The handle positions, text space.
    fn handle(&self, part: Part) -> Point {
        let [tl, tr, br, bl] = self.quad;
        match part {
            Part::Move => bl,
            Part::VScale => tl,
            Part::HScale => br,
            Part::Scale => tr,
            Part::Rotate => tl.midpoint(tr),
        }
    }
}

const PARTS: [Part; 5] = [Part::Rotate, Part::VScale, Part::Scale, Part::HScale, Part::Move];

/// A drag in progress.
#[derive(Clone, Debug)]
struct Drag {
    part: Part,
    /// Where the button went down (document space).
    press: Point,
    /// The character as it was then.
    from: CharBox,
    /// A preview has been sent (the interaction began).
    began: bool,
}

/// The type object's layout, keyed by the font database's generation and the object.
type CachedLayout = ((u64, TextObject), Arc<TextLayout>);

#[derive(Default)]
pub struct TouchTypeTool {
    /// The selected character: its type object and its first byte.
    target: Option<(NodeId, usize)>,
    drag: Option<Drag>,
    cache: Mutex<Option<CachedLayout>>,
}

fn text_of<'a>(cx: &'a ToolContext, id: NodeId) -> Option<&'a TextObject> {
    match &cx.doc.node(id)?.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

/// The linear part of `m` applied to `v`.
fn linear(m: Affine, v: Vec2) -> Vec2 {
    m * (Point::ZERO + v) - m * Point::ZERO
}

/// `to / from`, at least 0.01: what a scale handle dragged from `from` to `to` multiplies the scale
/// by (1 for a glyph with no width or height).
fn factor(to: f64, from: f64) -> f64 {
    let f = to / from;
    if f.is_finite() { f.max(0.01) } else { 1.0 }
}

/// Top left, top right, bottom right and bottom left of glyph `g`'s box in text space: its advance
/// by its font's ascent and descent, scaled, shifted and turned with the glyph. None when its font
/// is gone.
fn glyph_quad(g: &PositionedGlyph) -> Option<[Point; 4]> {
    let face = FontDb::global().face_by_id(g.font_id)?;
    let (ascent, descent) = face.vertical_metrics();
    let advance = face.advance(g.gid);
    let m = g.xf;
    Some([m * Point::new(0.0, -ascent), m * Point::new(advance, -ascent), m * Point::new(advance, descent), m * Point::new(0.0, descent)])
}

/// Is glyph `gi` of `lay` an inline graphic?
fn is_inline(lay: &TextLayout, gi: usize) -> bool {
    lay.inlines.iter().any(|i| i.glyph == gi)
}

/// The box of the character whose first glyph is glyph `gi` of `lay` (the layout of `t`): the box
/// in that glyph's own frame around the boxes of all its glyphs (an All Caps ß is two); none for
/// inline graphics, glyphs whose font is gone and type whose transform is collapsed.
fn char_box(t: &TextObject, lay: &TextLayout, gi: usize) -> Option<CharBox> {
    let g: &PositionedGlyph = lay.glyphs.get(gi)?;
    if is_inline(lay, gi) {
        return None;
    }
    let m = g.xf;
    let (pen, ex, ey) = (m * Point::ZERO, unit(linear(m, Vec2::new(1.0, 0.0))), unit(linear(m, Vec2::new(0.0, -1.0))));
    // The corners of its glyphs' boxes in the first glyph's font units (y down). A list marker's
    // glyphs (length 0) stand for no character.
    let to_glyph = m.inverse();
    let corners: Vec<Point> = lay
        .glyphs
        .iter()
        .filter(|h| h.byte == g.byte && h.len > 0 && h.line == g.line)
        .filter_map(glyph_quad)
        .flatten()
        .map(|p| to_glyph * p)
        .collect();
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in corners {
        (x0, x1, y0, y1) = (x0.min(p.x), x1.max(p.x), y0.min(p.y), y1.max(p.y));
    }
    if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let quad = [m * Point::new(x0, y0), m * Point::new(x1, y0), m * Point::new(x1, y1), m * Point::new(x0, y1)];
    let d = Vec2::new(g.angle.cos(), g.angle.sin());
    let style = edit::slice_runs(&t.runs, g.byte, g.byte + g.len).first().map(|r| r.style.clone())?;
    Some(CharBox {
        start: g.byte,
        quad,
        pen,
        pivot: g.origin,
        along: if g.rtl { -d } else { d },
        ex,
        ey,
        kerned: !g.rtl && lay.glyphs.iter().any(|h| h.line == g.line && h.len > 0 && h.byte + h.len == g.byte),
        style: t.effective_char_style(&style)?,
    })
}

impl TouchTypeTool {
    /// Layout of `t`, cached by content (overlays run every frame) and by the fonts available.
    fn layout(&self, t: &TextObject) -> Arc<TextLayout> {
        let fonts = FontDb::global().generation();
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(((g, k), l)) = c.as_ref()
            && *g == fonts
            && k == t
        {
            return l.clone();
        }
        let l = Arc::new(vectorcraft_text::layout(FontDb::global(), t));
        *c = Some(((fonts, t.clone()), l.clone()));
        l
    }

    /// The selected character's type object and box, while that object is selected.
    fn current<'c>(&self, cx: &'c ToolContext) -> Option<(NodeId, &'c TextObject, CharBox)> {
        let (id, start) = self.target?;
        let selected = std::iter::successors(Some(id), |c| cx.doc.parent_of(*c)).any(|c| cx.selection.contains(c));
        let t = text_of(cx, id).filter(|_| selected)?;
        let lay = self.layout(t);
        let gi = lay.glyphs.iter().position(|g| g.byte <= start && start < g.byte + g.len)?;
        Some((id, t, char_box(t, &lay, gi)?))
    }

    /// The character of a type object under `p` (document space): the one whose box holds it, else
    /// the nearest one in the object clicked.
    fn char_at(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, usize)> {
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { type_path_only: false, ..cx.hit_options() })?;
        let t = text_of(cx, h.leaf)?;
        let lay = self.layout(t);
        let local = t.xf.inverse() * p;
        let boxes: Vec<(usize, [Point; 4])> = (lay.glyphs.iter().enumerate())
            .filter(|(gi, g)| g.len > 0 && !is_inline(&lay, *gi))
            .filter_map(|(_, g)| Some((g.byte, glyph_quad(g)?)))
            .collect();
        let center = |q: &[Point; 4]| q[0].midpoint(q[2]);
        let hit = boxes.iter().rev().find(|(_, q)| in_quad(q, local));
        let best = hit.or_else(|| boxes.iter().min_by(|(_, a), (_, b)| (center(a) - local).hypot2().total_cmp(&(center(b) - local).hypot2())))?;
        Some((h.leaf, best.0))
    }

    /// The handle of `b` (of an object drawn through `xf`) under `p`, document space.
    fn part_at(cx: &ToolContext, xf: Affine, b: &CharBox, p: Point) -> Option<Part> {
        let tol = cx.tol(5.0);
        if let Some(part) = PARTS.into_iter().find(|part| (xf * b.handle(*part) - p).hypot() <= tol) {
            return Some(part);
        }
        in_quad(&b.quad, xf.inverse() * p).then_some(Part::Move)
    }

    /// Document-space move for a character box (`b` of `t`): kerning along the line and baseline
    /// shift along the character's up axis.
    fn move_params(id: NodeId, t: &TextObject, b: &CharBox, moved: Vec2) -> Value {
        let doc_len = |v: Vec2| linear(t.xf, v).hypot();
        let det = b.along.cross(b.ey);
        let (along, rise) = if !b.kerned {
            (0.0, moved.dot(b.ey))
        } else if det.abs() > 0.1 {
            (moved.cross(b.ey) / det, b.along.cross(moved) / det)
        } else {
            (moved.dot(b.along), 0.0)
        };
        let mut out = json!({"id": id.0, "start": b.start});
        let dx = along * doc_len(b.along);
        let dy = rise * doc_len(b.ey);
        if dx != 0.0 {
            out["dx"] = json!(dx);
        }
        if dy != 0.0 {
            out["dy"] = json!(dy);
        }
        out
    }

    /// Arrow-key nudge in document space.
    fn nudge_params(id: NodeId, t: &TextObject, b: &CharBox, doc_dx: f64, doc_dy: f64) -> Value {
        Self::move_params(id, t, b, linear(t.xf.inverse(), Vec2::new(doc_dx, doc_dy)))
    }

    fn arrow_nudge(key: ToolKey, vertical: bool, step: f64) -> Option<Vec2> {
        let key = if vertical {
            match key {
                ToolKey::Up => ToolKey::Left,
                ToolKey::Down => ToolKey::Right,
                ToolKey::Right => ToolKey::Up,
                ToolKey::Left => ToolKey::Down,
                other => other,
            }
        } else {
            key
        };
        match key {
            ToolKey::Left => Some(Vec2::new(-step, 0.0)),
            ToolKey::Right => Some(Vec2::new(step, 0.0)),
            ToolKey::Up => Some(Vec2::new(0.0, -step)),
            ToolKey::Down => Some(Vec2::new(0.0, step)),
            _ => None,
        }
    }

    /// The `text.touchType` params for the drag `d` with the pointer at `p` (document space).
    fn params(id: NodeId, t: &TextObject, d: &Drag, p: Point) -> Value {
        let b = &d.from;
        let inv = t.xf.inverse();
        let moved = linear(inv, p - d.press);
        let st = &b.style;
        let mut out = json!({"id": id.0, "start": b.start});
        match d.part {
            Part::Move => out = Self::move_params(id, t, b, moved),
            Part::HScale => {
                let w = (b.quad[1] - b.quad[0]).hypot();
                out["hScale"] = json!(st.h_scale * factor(w + moved.dot(b.ex), w));
            }
            Part::VScale => {
                let h = (b.quad[0] - b.pen).dot(b.ey);
                out["vScale"] = json!(st.v_scale * factor(h + moved.dot(b.ey), h));
            }
            Part::Scale => {
                let v = b.quad[1] - b.pen;
                let s = factor((v + moved).dot(v), v.hypot2());
                out["hScale"] = json!(st.h_scale * s);
                out["vScale"] = json!(st.v_scale * s);
            }
            Part::Rotate => {
                let (from, to) = (inv * d.press - b.pivot, inv * p - b.pivot);
                // Text space has y down, so a growing angle turns clockwise; rotation counts
                // counter-clockwise.
                let turned = (to.y.atan2(to.x) - from.y.atan2(from.x)).to_degrees();
                out["rotation"] = json!(st.rotation - turned);
            }
        }
        out
    }
}

impl Tool for TouchTypeTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                let mut out = vec![];
                let on_box = self.current(cx).and_then(|(_, t, b)| Some((Self::part_at(cx, t.xf, &b, ev.pos)?, b)));
                let (part, from) = match on_box {
                    Some(hit) => hit,
                    None => {
                        // A click on another character selects it (and its type object).
                        self.target = self.char_at(cx, ev.pos);
                        let Some((id, _)) = self.target else { return out };
                        if !cx.selection.contains(id) {
                            out.push(Action::Exec("select.set".into(), json!({"ids": [id.0]})));
                        }
                        let Some(t) = text_of(cx, id) else { return out };
                        let lay = self.layout(t);
                        let start = self.target.map_or(0, |(_, s)| s);
                        let Some(b) = lay.glyphs.iter().position(|g| g.byte == start && g.len > 0).and_then(|gi| char_box(t, &lay, gi)) else {
                            return out;
                        };
                        (Part::Move, b)
                    }
                };
                self.drag = Some(Drag { part, press: ev.pos, from, began: false });
                out
            }
            PointerKind::Drag => {
                let (Some((id, _)), Some(d)) = (self.target, self.drag.as_mut()) else { return vec![] };
                let Some(t) = text_of(cx, id) else { return vec![] };
                if !d.began && (ev.pos - d.press).hypot() < cx.tol(1.0) {
                    return vec![];
                }
                let mut out = vec![];
                if !d.began {
                    d.began = true;
                    out.push(Action::Begin("Touch Type".into()));
                }
                out.push(Action::Preview("text.touchType".into(), Self::params(id, t, d, ev.pos)));
                out
            }
            PointerKind::Up => match self.drag.take() {
                Some(d) if d.began => vec![Action::Commit],
                _ => vec![],
            },
            PointerKind::DoubleClick | PointerKind::Move => vec![],
        }
    }

    /// Escape cancels a drag or clears the selection; arrow keys nudge the selected character.
    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            return match self.drag.take() {
                Some(d) if d.began => vec![Action::Cancel],
                Some(_) => vec![],
                None => {
                    self.target = None;
                    vec![]
                }
            };
        }
        if self.drag.is_some() {
            return vec![];
        }
        let Some((id, t, b)) = self.current(cx) else { return vec![] };
        let lay = self.layout(t);
        let step = cx.keyboard_increment * if mods.shift { 10.0 } else { 1.0 };
        let Some(d) = Self::arrow_nudge(key, lay.vertical, step) else { return vec![] };
        let params = Self::nudge_params(id, t, &b, d.x, d.y);
        if params.get("dx").is_none() && params.get("dy").is_none() {
            return vec![];
        }
        vec![Action::Exec("text.touchType".into(), params)]
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        if key == ToolKey::Escape {
            return self.drag.is_some() || self.current(cx).is_some();
        }
        if self.drag.is_some() {
            return false;
        }
        self.current(cx).is_some() && matches!(key, ToolKey::Left | ToolKey::Right | ToolKey::Up | ToolKey::Down)
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some((_, t, b)) = self.current(cx) else { return vec![] };
        let q = b.quad.map(|p| t.xf * p);
        let mut out: Vec<Overlay> = (0..4).map(|i| Overlay::Line { a: q[i], b: q[(i + 1) % 4], color: BLUE, dashed: false }).collect();
        for part in PARTS {
            let p = t.xf * b.handle(part);
            out.push(match part {
                Part::Rotate => Overlay::CurvePoint { p, color: BLUE, filled: false, size: 7.0 },
                Part::Move => Overlay::Anchor { p, color: BLUE, filled: true, size: 7.0 },
                _ => Overlay::Anchor { p, color: BLUE, filled: false, size: 7.0 },
            });
        }
        out
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        let Some((_, t, b)) = self.current(cx) else { return Cursor::Arrow };
        match Self::part_at(cx, t.xf, &b, p) {
            Some(Part::Rotate) => Cursor::rotate_about(t.xf * b.pivot, p),
            Some(Part::VScale) => Cursor::ResizeV,
            Some(Part::HScale) => Cursor::ResizeH,
            Some(Part::Scale) => Cursor::ResizeNeSw,
            Some(Part::Move) => Cursor::Move,
            None => Cursor::Arrow,
        }
    }

    /// `{id, start}`: the selected character's type object and first byte, or `{}`.
    fn options(&self) -> Value {
        match self.target {
            Some((id, start)) => json!({"id": id.0, "start": start}),
            None => json!({}),
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.target = None;
        match self.drag.take() {
            Some(d) if d.began => vec![Action::Cancel],
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::{Document, Node, Selection};

    use super::*;
    use crate::testutil::{cx, paint};

    /// 40 pt point type "Type" at (100, 200), selected.
    fn doc_with_type() -> (Document, NodeId, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let t = TextObject::point(Point::new(100.0, 200.0), "Type", CharStyle { size: 40.0, ..Default::default() });
        d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        let mut s = Selection::default();
        s.add(id);
        (d, id, s)
    }

    /// The selected character's box in document space.
    fn quad(t: &TouchTypeTool, cx: &ToolContext) -> [Point; 4] {
        let (_, text, b) = t.current(cx).expect("a character is selected");
        b.quad.map(|p| text.xf * p)
    }

    /// A press at `from`, a drag to `to` and the release: the actions of each.
    fn drag(t: &mut TouchTypeTool, cx: &ToolContext, from: Point, to: Point) -> Vec<Vec<Action>> {
        [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)]
            .into_iter()
            .map(|(kind, p)| t.pointer(cx, &PointerEvent::new(kind, p.x, p.y)))
            .collect()
    }

    fn preview(acts: &[Action]) -> &Value {
        acts.iter()
            .find_map(|a| match a {
                Action::Preview(c, p) if c == "text.touchType" => Some(p),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text.touchType preview: {acts:?}"))
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_click_selects_one_character_and_shows_its_box() {
        let (d, id, s) = doc_with_type();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TouchTypeTool::default();
        assert!(t.overlays(&cx).is_empty(), "no character yet");
        // Somewhere in the second character, the "y".
        let t_object = text_of(&cx, id).unwrap().clone();
        let lay = vectorcraft_text::layout(FontDb::global(), &t_object);
        let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
        let at = t_object.xf * (y.origin + Vec2::new(y.advance / 2.0, -10.0));
        let acts = drag(&mut t, &cx, at, at);
        assert!(acts.iter().all(Vec::is_empty), "a click changes nothing: {acts:?}");
        assert_eq!(t.options(), json!({"id": id.0, "start": 1}));
        // Its box: four sides and five handles, the rotate handle in the middle of the top edge.
        let ov = t.overlays(&cx);
        assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::Line { .. })).count(), 4);
        assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count(), 4);
        assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::CurvePoint { .. })).count(), 1);
        let q = quad(&t, &cx);
        assert!(q[0].x <= at.x && at.x <= q[1].x && q[0].y < at.y && at.y < q[3].y, "{q:?} holds {at:?}");
        assert_eq!(t.cursor(&cx, q[0].midpoint(q[1]), Mods::default()), Cursor::rotate_about(t_object.xf * y.origin, q[0].midpoint(q[1])));
        assert_eq!(t.cursor(&cx, q[2], Mods::default()), Cursor::ResizeH);
        assert_eq!(t.cursor(&cx, q[0], Mods::default()), Cursor::ResizeV);
        assert_eq!(t.cursor(&cx, q[1], Mods::default()), Cursor::ResizeNeSw);
        assert_eq!(t.cursor(&cx, q[0].midpoint(q[2]), Mods::default()), Cursor::Move);
        // Escape lets go of it.
        assert!(t.claims_key(&cx, ToolKey::Escape));
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(t.overlays(&cx).is_empty() && t.options() == json!({}));
        // A click on type that isn't selected selects it too.
        let none = Selection::default();
        let cx = crate::testutil::cx(&d, &none, &p);
        let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    }

    /// In a list item, the first character's box holds that character and not the bullet before
    /// it, and a drag of the box moves that character.
    #[test]
    fn the_first_character_of_a_list_item_has_its_own_box_and_drags() {
        let (mut d, id, s) = doc_with_type();
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            t.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
            t.para.left_indent = 18.0;
            t.para.first_line_indent = -18.0;
        }
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TouchTypeTool::default();
        let t_object = text_of(&cx, id).unwrap().clone();
        let lay = vectorcraft_text::layout(FontDb::global(), &t_object);
        let first = lay.glyphs.iter().find(|g| g.byte == 0 && g.len > 0).unwrap();
        let at = t_object.xf * (first.origin + Vec2::new(first.advance / 2.0, -10.0));
        drag(&mut t, &cx, at, at);
        assert_eq!(t.options(), json!({"id": id.0, "start": 0}));
        let (q, pen) = (quad(&t, &cx), t_object.xf * first.origin);
        assert!(q[0].x > pen.x - 5.0, "the box starts at the T, 18 pt past the bullet: {q:?}, the T at {pen:?}");
        let acts = drag(&mut t, &cx, at, at + Vec2::new(0.0, -4.0));
        assert_eq!(preview(&acts[1])["start"].as_u64(), Some(0));
        // A drag on the character once the type is deselected selects the type and moves it.
        let none = Selection::default();
        let cx = crate::testutil::cx(&d, &none, &p);
        let acts = drag(&mut t, &cx, at, at + Vec2::new(0.0, -4.0));
        assert_eq!(acts[0], vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
        assert_eq!(preview(&acts[1])["start"].as_u64(), Some(0));
    }

    #[test]
    fn dragging_the_box_moves_the_character_along_its_line_and_up() {
        let (d, id, s) = doc_with_type();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TouchTypeTool::default();
        let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
        let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
        let at = Point::new(100.0, 200.0) + (y.origin.to_vec2() + Vec2::new(y.advance / 2.0, -10.0));
        // 10 pt right and 4 pt up: one preview, one undo step.
        let acts = drag(&mut t, &cx, at, at + Vec2::new(10.0, -4.0));
        assert_eq!(acts[1][0], Action::Begin("Touch Type".into()));
        let p1 = preview(&acts[1]);
        assert_eq!((p1["id"].as_u64(), p1["start"].as_u64()), (Some(id.0), Some(1)));
        assert!(close(p1["dx"].as_f64().unwrap(), 10.0) && close(p1["dy"].as_f64().unwrap(), 4.0), "{p1}");
        assert_eq!(acts[2], vec![Action::Commit]);
        // The lower-left handle moves it too.
        let bl = quad(&t, &cx)[3];
        let acts = drag(&mut t, &cx, bl, bl + Vec2::new(-3.0, 2.0));
        let p2 = preview(&acts[1]);
        assert!(close(p2["dx"].as_f64().unwrap(), -3.0) && close(p2["dy"].as_f64().unwrap(), -2.0), "{p2}");
        // The first character has no character before it on its line: it moves only up and down.
        let first = Point::new(100.0, 200.0) + Vec2::new(lay.glyphs[0].advance / 2.0, -10.0);
        let acts = drag(&mut t, &cx, first, first + Vec2::new(10.0, -4.0));
        let p3 = preview(&acts[1]);
        assert_eq!(p3["start"], json!(0));
        assert!(close(p3.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.0), 0.0) && close(p3["dy"].as_f64().unwrap(), 4.0), "{p3}");
    }

    #[test]
    fn arrow_keys_nudge_the_selected_character() {
        let (d, id, s) = doc_with_type();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TouchTypeTool::default();
        let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
        let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
        let at = Point::new(100.0, 200.0) + (y.origin.to_vec2() + Vec2::new(y.advance / 2.0, -10.0));
        drag(&mut t, &cx, at, at);
        assert!(t.claims_key(&cx, ToolKey::Right));
        let acts = t.key(&cx, ToolKey::Right, Mods::default());
        let p1 = match &acts[0] {
            Action::Exec(c, v) if c == "text.touchType" => v,
            other => panic!("{other:?}"),
        };
        assert!(close(p1["dx"].as_f64().unwrap(), 1.0) && p1.get("dy").is_none(), "{p1}");
        // The first character can't kern: a horizontal nudge does nothing.
        let first = Point::new(100.0, 200.0) + Vec2::new(lay.glyphs[0].advance / 2.0, -10.0);
        drag(&mut t, &cx, first, first);
        assert!(t.key(&cx, ToolKey::Left, Mods::default()).is_empty());
        let acts = t.key(&cx, ToolKey::Up, Mods::default());
        let p2 = match &acts[0] {
            Action::Exec(_, v) => v,
            other => panic!("{other:?}"),
        };
        assert!(close(p2["dy"].as_f64().unwrap(), 1.0), "{p2}");
        // Shift multiplies the increment (on a character that can kern).
        drag(&mut t, &cx, at, at);
        let acts = t.key(&cx, ToolKey::Right, Mods { shift: true, ..Mods::default() });
        let p3 = match &acts[0] {
            Action::Exec(_, v) => v,
            other => panic!("{other:?}"),
        };
        assert!(close(p3["dx"].as_f64().unwrap(), 10.0), "{p3}");
    }

    #[test]
    fn the_handles_scale_and_rotate_the_character() {
        let (d, id, s) = doc_with_type();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = TouchTypeTool::default();
        let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
        let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
        let at = Point::new(100.0, 200.0) + (y.origin.to_vec2() + Vec2::new(y.advance / 2.0, -10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, at.x, at.y));
        let [tl, tr, br, _] = quad(&t, &cx);
        let (w, h) = (tr.x - tl.x, br.y - tr.y);
        let above = 200.0 - tl.y;
        // The lower-right handle: horizontal scale; half the width again is 150 %.
        let acts = drag(&mut t, &cx, br, br + Vec2::new(w / 2.0, 7.0));
        let ps = preview(&acts[1]);
        assert!(close(ps["hScale"].as_f64().unwrap(), 150.0) && ps.get("vScale").is_none(), "{ps}");
        // The upper-left handle: vertical scale, measured from the baseline.
        let acts = drag(&mut t, &cx, tl, tl + Vec2::new(5.0, -above));
        let ps = preview(&acts[1]);
        assert!(close(ps["vScale"].as_f64().unwrap(), 200.0) && ps.get("hScale").is_none(), "{ps}");
        // The upper-right handle: both alike.
        let pen = Point::new(tl.x, 200.0);
        let acts = drag(&mut t, &cx, tr, pen + (tr - pen) * 0.5);
        let ps = preview(&acts[1]);
        assert!(close(ps["hScale"].as_f64().unwrap(), 50.0) && close(ps["vScale"].as_f64().unwrap(), 50.0), "{ps}");
        // The top handle: a quarter turn counter-clockwise about its pen position on the baseline.
        let top = tl.midpoint(tr);
        let pivot = Point::new(100.0, 200.0) + y.origin.to_vec2();
        let r = top - pivot;
        let acts = drag(&mut t, &cx, top, pivot + Vec2::new(r.y, -r.x));
        let ps = preview(&acts[1]);
        assert!(close(ps["rotation"].as_f64().unwrap(), 90.0), "{ps}");
        assert!(h > 0.0);
        // Escape mid-drag cancels the drag and keeps the character.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, top.x, top.y));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, top.x + 20.0, top.y));
        assert!(t.claims_key(&cx, ToolKey::Escape));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, top.x + 20.0, top.y)), vec![]);
        assert_eq!(t.options(), json!({"id": id.0, "start": 1}));
        // Switching tools mid-drag drops the drag.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, top.x, top.y));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, top.x + 20.0, top.y));
        assert_eq!(t.deactivate(&cx), vec![Action::Cancel]);
    }

    #[test]
    fn the_box_of_a_character_drawn_with_two_glyphs_holds_both() {
        // An All Caps ß is two glyphs (SS) with one byte range.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let text = TextObject::point(Point::new(100.0, 200.0), "aße", CharStyle { size: 40.0, all_caps: true, ..Default::default() });
        d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(text.clone())))).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let lay = vectorcraft_text::layout(FontDb::global(), &text);
        let esses: Vec<_> = lay.glyphs.iter().filter(|g| g.byte == 1).collect();
        assert_eq!(esses.len(), 2);
        // A click on the second S selects the ß, and its box reaches from the first S to the end of the second.
        let second = esses[1];
        let at = text.xf * (second.origin + Vec2::new(second.advance / 2.0, -10.0));
        let mut t = TouchTypeTool::default();
        drag(&mut t, &cx, at, at);
        assert_eq!(t.options(), json!({"id": id.0, "start": 1}));
        let q = quad(&t, &cx);
        let ends = |g: &PositionedGlyph| (text.xf * glyph_quad(g).unwrap()[1]).x;
        assert!((q[1].x - ends(second)).abs() < 1e-9 && q[0].x < ends(esses[0]), "{q:?}");
        // Dragging the lower-right handle by half the box's width is 150 %.
        let w = q[1].x - q[0].x;
        let acts = drag(&mut t, &cx, q[2], q[2] + Vec2::new(w / 2.0, 0.0));
        assert!(close(preview(&acts[1])["hScale"].as_f64().unwrap(), 150.0));
        // Rotated, each S turns about its own origin: the box still holds both, square to the first.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let text = TextObject::point(Point::new(100.0, 200.0), "aße", CharStyle { size: 40.0, all_caps: true, rotation: 30.0, ..Default::default() });
        d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(text.clone())))).unwrap();
        let mut s = Selection::default();
        s.add(id);
        let cx = crate::testutil::cx(&d, &s, &p);
        let t = TouchTypeTool { target: Some((id, 1)), ..Default::default() };
        let q = quad(&t, &cx);
        let (across, down) = (q[1] - q[0], q[3] - q[0]);
        assert!(across.dot(down).abs() < 1e-9, "a rectangle: {q:?}");
        let lay = vectorcraft_text::layout(FontDb::global(), &text);
        for g in lay.glyphs.iter().filter(|g| g.byte == 1) {
            for c in glyph_quad(g).unwrap() {
                let v = text.xf * c - q[0];
                let (a, b) = (v.dot(across) / across.hypot2(), v.dot(down) / down.hypot2());
                assert!((-1e-9..=1.0 + 1e-9).contains(&a) && (-1e-9..=1.0 + 1e-9).contains(&b), "{c:?} outside {q:?}");
            }
        }
    }

    /// A glyph skewed by its transform (the Skew effect of type on a path) keeps its own box.
    #[test]
    fn a_skewed_glyph_keeps_its_own_box() {
        let text = TextObject::point(Point::new(100.0, 200.0), "Type", CharStyle { size: 40.0, ..Default::default() });
        let mut lay = vectorcraft_text::layout(FontDb::global(), &text);
        lay.glyphs[1].xf *= Affine::skew(0.5, 0.0);
        let b = char_box(&text, &lay, 1).unwrap();
        let own = glyph_quad(&lay.glyphs[1]).unwrap();
        assert!(b.quad.iter().zip(own).all(|(a, b)| (*a - b).hypot() < 1e-9), "{:?} {own:?}", b.quad);
    }

    /// `create` makes the Touch Type tool: a click with it selects a character.
    #[test]
    fn the_tool_is_made_by_its_id() {
        let (d, id, s) = doc_with_type();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
        let at = Point::new(100.0, 200.0) + Vec2::new(lay.glyphs[0].advance / 2.0, -10.0);
        let mut t = crate::create(ID);
        assert_eq!(t.id(), ID);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, at.x, at.y));
        assert_eq!(t.options(), json!({"id": id.0, "start": 0}));
    }
}
