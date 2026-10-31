//! Artwork Cleanup's text tools: Break Text into Lines / Paragraphs / Words / Glyphs, Combine Point
//! Text, Split Text at Cursor, Remove Line / Paragraph Breaks, Set Alignment, Keep Position, Reset
//! Text Transform and Replace Missing Fonts. Each is one undo step. Pieces keep their characters'
//! styles and stay where the layout drew them: each is moved so the glyph of its first character
//! lands where it was.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Document, Justify, Node, NodeId, NodeKind, ParaStyle, RubySpan, TextKind, TextObject};
use vectorcraft_geom::{Affine, Point, Vec2};
use vectorcraft_text::{FontDb, FontMatch, TextLayout, edit};

use crate::cmd::textedit::editing_range;
use crate::cmd::typecmd::refresh_bounds;
use crate::cmd::{CommandSpec, bad, checked_ids_or_selection, cmd, has_doc, has_selection, switch_or, text_param};
use crate::{EngineError, Result, Session};

/// Most text objects one command takes.
const MAX_TEXTS: usize = 10_000;
/// Most pieces Break Text and Split Text at Cursor make at once.
const MAX_PIECES: usize = 20_000;
/// Most breaks Remove Line / Paragraph Breaks removes at once.
const MAX_BREAKS: usize = 200_000;
/// Most pieces Combine Point Text tries to join, counted over every try: past it, what it joined
/// so far is the result.
const MAX_JOIN_TRIES: usize = 200_000;
/// How far (pt) a glyph Combine Point Text sets may land from where it was.
const TOLERANCE: f64 = 0.01;
/// Combine Point Text: pieces whose baselines are within this many ems of each other share a line
/// (the difference becomes baseline shift); a gap wider than `MAX_GAP` ems, or an overlap of more
/// than `MAX_OVERLAP`, starts another object.
const SAME_LINE: f64 = 0.4;
const MAX_GAP: f64 = 10.0;
const MAX_OVERLAP: f64 = 1.0;
/// Tracking Combine Point Text may give a character (thousandths of an em).
const TRACKING: std::ops::RangeInclusive<f64> = -1000.0..=10000.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.cleanup.breakText",
            "Break Text",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{by?: \"lines\"|\"paragraphs\"|\"words\"|\"glyphs\" (lines), ids?: [id…] (default: the selection)} each horizontal point or area type object becomes point type objects, one per line, paragraph, word or glyph, each where its characters were and with their character styles, as one undo step; spaces between pieces go, a paragraph's lines stay lines (a wrapped line ends in a line break), justified lines close up, and a paragraph broken out whole keeps its bullet (not its number; other pieces have no list markers). Type on a path, vertical type, threaded type and area type whose text doesn't fit stay as they are → {ids (the pieces, selected), skipped}",
            has_selection,
            break_text
        ),
        cmd!(
            "addon.cleanup.combineText",
            "Combine Point Text",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{ids?: [id…] (default: the selection)} merge one-line horizontal point type into as few objects as it can, every glyph staying where it was: pieces turned and sheared alike whose baselines line up (within 0.4 em; the difference becomes baseline shift) join left to right, a space goes between pieces at least half a space apart, and tracking on the character before each piece keeps the spacing; each object keeps its character styles, and the leftmost piece of each run its place and paint, as one undo step → {ids (the objects made, selected), combined (pieces merged into them)}",
            has_selection,
            combine_text
        ),
        cmd!(
            "addon.cleanup.removeBreaks",
            "Remove Breaks",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{kind: \"line\"|\"paragraph\", ids?: [id…]} remove the forced line breaks (line) or paragraph returns (paragraph) in the characters selected with a Type tool, else in all the text of the selected type, as one undo step; a break between two words becomes a space → {ids, removed}",
            has_selection,
            remove_breaks
        ),
        cmd!(
            "addon.cleanup.splitText",
            "Split Text at Cursor",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{} while a Type tool edits horizontal point or area type: the text before the selected characters, they and the text after become point type objects of their own (at a cursor: before and after it), each where its characters were, as one undo step; spaces at the ends of each piece go, and editing ends with the pieces selected → {ids}",
            editing_text,
            split_text
        ),
        cmd!(
            "addon.cleanup.alignInPlace",
            "Set Alignment, Keep Position",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{align: \"left\"|\"center\"|\"right\", ids?: [id…] (default: the selection)} align every paragraph of each horizontal point type object so, moving the object so its longest line stays where it is (one line: the text doesn't move), as one undo step → {ids (the objects changed)}",
            has_selection,
            align_in_place
        ),
        cmd!(
            "addon.cleanup.resetTextTransform",
            "Reset Text Transform",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{scale?: bool (false: the horizontal scale stays; true: it goes too, the object's and every character's), ids?: [id…] (default: the selection)} take the rotation, shear and reflection off each point type object, its size and the centre of its text staying, as one undo step → {ids (the objects changed)}",
            has_selection,
            reset_transform
        ),
        cmd!(
            "addon.cleanup.replaceFonts",
            "Replace Missing Fonts…",
            ["Object", "Plug-ins", "Artwork Cleanup"],
            None,
            "{font?: family (default: the default type font, Source Sans 3; any of its names)} set every character of the document's type whose font family isn't available in `font`, in the style it has closest to the one each had, as one undo step → {runs (style runs changed), font}",
            has_doc,
            replace_fonts
        ),
    ]
}

/// Enabled while a Type tool edits text.
fn editing_text(s: &Session) -> std::result::Result<(), String> {
    match s.tool_options().get("editing") {
        Some(v) if !v.is_null() => Ok(()),
        _ => Err("Click in type with a Type tool to split it".into()),
    }
}

/// The type objects under `roots` (inside groups and layers, not live art) that can be edited.
fn texts_under(d: &Document, roots: &[NodeId]) -> Result<Vec<NodeId>> {
    fn visit(n: &Node, out: &mut Vec<NodeId>) {
        match &n.kind {
            NodeKind::Text(_) => out.push(n.id),
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().for_each(|c| visit(c, out)),
            _ => {}
        }
    }
    let mut out = vec![];
    roots.iter().filter_map(|id| d.node(*id)).for_each(|n| visit(n, &mut out));
    let editable = d.editable();
    out.retain(|id| editable(*id));
    out.dedup();
    if out.len() > MAX_TEXTS {
        return Err(EngineError::Other("Too many type objects to change at once".into()));
    }
    Ok(out)
}

fn text_of(d: &Document, id: NodeId) -> Option<&TextObject> {
    match &d.node(id)?.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

fn layout(t: &TextObject) -> TextLayout {
    vectorcraft_text::layout(FontDb::global(), t)
}

/// Where the layout draws the character at `byte` (the one it starts in): its glyph's pen position.
fn glyph_at(lay: &TextLayout, byte: usize) -> Option<Point> {
    let start = lay.glyphs.iter().filter(|g| g.len > 0 && g.byte <= byte).map(|g| g.byte).max()?;
    lay.glyphs.iter().find(|g| g.len > 0 && g.byte == start).map(|g| g.origin)
}

/// The pen position of the glyph that starts at `byte`, if one does.
fn glyph_starting(lay: &TextLayout, byte: usize) -> Option<Point> {
    lay.glyphs.iter().find(|g| g.len > 0 && g.byte == byte).map(|g| g.origin)
}

/// The first glyph of line `l`, in text order.
fn line_start(lay: &TextLayout, l: usize) -> Option<Point> {
    let line = lay.lines.get(l)?;
    let g = lay.glyphs.get(line.glyph_start..line.glyph_end)?.iter().filter(|g| g.len > 0).min_by_key(|g| g.byte)?;
    Some(g.origin)
}

/// `a..b` of `text` without the white space at its ends; None when nothing else is left.
fn trimmed(text: &str, a: usize, b: usize) -> Option<(usize, usize)> {
    let s = text.get(a..b)?;
    let start = a + (s.len() - s.trim_start().len());
    let end = a + s.trim_end().len();
    (start < end).then_some((start, end))
}

/// Paragraph alignment as point type keeps it: justified lines align to their side.
fn point_justify(j: Justify) -> Justify {
    match j {
        Justify::JustifyLeft | Justify::JustifyAll => Justify::Left,
        Justify::JustifyCenter => Justify::Center,
        Justify::JustifyRight => Justify::Right,
        j => j,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum By {
    Lines,
    Paragraphs,
    Words,
    Glyphs,
}

/// A type object being cut into point type pieces: its text as drawn (Shrink Text to Fit's sizes
/// taken in) and its layout.
struct Source {
    t: TextObject,
    lay: TextLayout,
    text: String,
}

impl Source {
    /// Horizontal point or area type that isn't threaded and whose text all fits; None otherwise.
    fn of(d: &Document, id: NodeId) -> Option<Self> {
        let t = text_of(d, id)?;
        if t.vertical || matches!(t.kind, TextKind::OnPath { .. }) || d.text_threads.iter().any(|th| th.contains(&id)) {
            return None;
        }
        let lay = layout(t);
        // Text left out of a frame would be lost with it.
        if lay.overflow {
            return None;
        }
        let t = if (lay.fit_scale - 1.0).abs() > 1e-9 { vectorcraft_text::scaled(t, lay.fit_scale) } else { t.clone() };
        Some(Self { text: t.plain_text(), t, lay })
    }

    /// The byte ranges of the pieces `by` cuts the text into.
    fn ranges(&self, by: By) -> Vec<(usize, usize)> {
        let text = &self.text;
        match by {
            By::Paragraphs => {
                let mut out = vec![];
                let mut a = 0;
                for (i, _) in text.match_indices('\n') {
                    out.push((a, i));
                    a = i + 1;
                }
                out.push((a, text.len()));
                out
            }
            By::Lines => self.lay.lines.iter().map(|l| (l.start, l.end)).collect(),
            By::Words => {
                let mut out = vec![];
                for l in &self.lay.lines {
                    let Some(s) = text.get(l.start..l.end) else { continue };
                    let mut word: Option<usize> = None;
                    for (i, c) in s.char_indices() {
                        match (c.is_whitespace(), word) {
                            (true, Some(w)) => {
                                out.push((l.start + w, l.start + i));
                                word = None;
                            }
                            (false, None) => word = Some(i),
                            _ => {}
                        }
                    }
                    if let Some(w) = word {
                        out.push((l.start + w, l.end));
                    }
                }
                out
            }
            By::Glyphs => {
                // One piece per cluster (a ligature's characters stay together).
                let mut clusters = std::collections::BTreeMap::new();
                for g in self.lay.glyphs.iter().filter(|g| g.len > 0) {
                    let end = clusters.entry(g.byte).or_insert(g.byte + g.len);
                    *end = (*end).max(g.byte + g.len);
                }
                clusters.into_iter().collect()
            }
        }
    }

    /// Point type holding bytes `a..b` (white space at its ends left out) where they were drawn.
    /// Lines the layout wrapped end in a forced line break; `lists` keeps bulleted paragraphs'
    /// bullets where the piece starts its paragraph.
    fn piece(&self, (a, b): (usize, usize), lists: bool) -> Option<TextObject> {
        let (a, b) = trimmed(&self.text, a, b)?;
        let mut p = TextObject::point(Point::ZERO, "", CharStyle::default());
        p.runs = edit::slice_runs(&self.t.runs, a, b);
        p.xf = self.t.xf;
        p.optical_margin = self.t.optical_margin;
        p.system_layout = self.t.system_layout;
        p.fractional_widths = self.t.fractional_widths;
        p.ruby =
            self.t.ruby.iter().filter(|r| a <= r.start && r.end <= b).map(|r| RubySpan { start: r.start - a, end: r.end - a, ..r.clone() }).collect();
        // The paragraphs it holds, with their styles; the first keeps its bullet only from its start.
        let starts_para = self.text.get(..a).is_some_and(|s| s.rsplit('\n').next().is_some_and(|line| line.trim().is_empty()));
        let styles = self.t.paragraph_styles();
        let mut paras: Vec<ParaStyle> = styles.get(self.t.paragraphs_in(a, b))?.to_vec();
        for (i, para) in paras.iter_mut().enumerate() {
            para.justify = point_justify(para.justify);
            if !(lists && (i > 0 || starts_para) && para.list.as_ref().is_some_and(|l| l.number.is_none())) {
                para.list = None;
            }
        }
        p.set_paragraph_styles(paras);
        // Lines the layout wrapped end in a forced line break, in place of the spaces they ended in.
        let mut wraps = vec![];
        for next in self.lay.lines.iter().skip(1).filter(|l| a < l.start && l.start < b) {
            let content = self.text.get(..next.start)?.trim_end_matches([' ', '\t']);
            // A line that ends in a break of its own keeps it.
            if !content.ends_with(['\n', '\u{2028}']) {
                wraps.push((content.len().max(a) - a, next.start - a));
            }
        }
        for &(s, e) in wraps.iter().rev() {
            edit::replace_range(&mut p.runs, s, e, "\u{2028}");
            p.ruby_after_replace(s, e, '\u{2028}'.len_utf8());
        }
        self.place(&mut p, a)?;
        Some(p)
    }

    /// Move `p` (cut from byte `a`) so the glyph of its first character lands where it was drawn.
    /// A piece of several lines that starts inside a left-aligned line takes a first-line indent,
    /// so its later lines stay where they were too.
    fn place(&self, p: &mut TextObject, a: usize) -> Option<()> {
        let lay = layout(p);
        let mut shift: Vec2 = glyph_at(&self.lay, a)? - glyph_at(&lay, 0)?;
        let first = self.lay.lines.iter().rposition(|l| l.start <= a);
        if let (Some(now), Some(was)) = (line_start(&lay, 1), first.and_then(|l| line_start(&self.lay, l + 1))) {
            let off = was.x - (now.x + shift.x);
            let left = match p.para.justify {
                Justify::Left => true,
                Justify::Auto => lay.lines.first().is_some_and(|l| !l.rtl),
                _ => false,
            };
            if left && off.abs() > 1e-6 {
                p.edit_paras(Some(0..1), |para| para.first_line_indent -= off);
                shift.x += off;
            }
        }
        p.xf = self.t.xf * Affine::translate(shift);
        Some(())
    }
}

/// Put each object of `items` (id, the node, its pieces) out of the document, its pieces in its
/// place, as one undo step labelled `label`, the pieces selected → their ids.
fn replace_with_pieces(s: &mut Session, label: &str, items: Vec<(NodeId, Node, Vec<TextObject>)>) -> Result<Vec<u64>> {
    s.edit(label, |d, sel| {
        let (mut made, mut places) = (vec![], vec![]);
        for (id, mut node, pieces) in items {
            // A light template: the pieces take its paint and attributes, not its text.
            node.kind = NodeKind::Text(Box::new(TextObject::point(Point::ZERO, "", CharStyle::default())));
            let nodes: Vec<Node> = pieces
                .into_iter()
                .map(|mut t| {
                    refresh_bounds(&mut t);
                    let mut n = d.reid(&node);
                    n.kind = NodeKind::Text(Box::new(t));
                    n
                })
                .collect();
            made.extend(nodes.iter().map(|n| n.id));
            places.push((id, nodes));
        }
        d.insert_beside(places, true)?;
        sel.set(made.iter().copied());
        Ok(made.iter().map(|i| i.0).collect())
    })
}

fn ids_json(ids: &[NodeId]) -> Vec<u64> {
    ids.iter().map(|i| i.0).collect()
}

fn break_text(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.breakText";
    let by = match text_param(p, "by", C)?.unwrap_or("lines") {
        "lines" => By::Lines,
        "paragraphs" => By::Paragraphs,
        "words" => By::Words,
        "glyphs" => By::Glyphs,
        other => return Err(bad(C, format!("`by` must be lines, paragraphs, words or glyphs, not {other:?}"))),
    };
    let d = &s.doc()?.doc;
    let (mut items, mut skipped, mut count) = (vec![], vec![], 0);
    for id in texts_under(d, &checked_ids_or_selection(s, p, C)?)? {
        let (Some(src), Some(node)) = (Source::of(d, id), d.node(id)) else {
            skipped.push(id);
            continue;
        };
        let ranges = src.ranges(by);
        count += ranges.len();
        if count > MAX_PIECES {
            return Err(EngineError::Other("Too many pieces to make at once".into()));
        }
        let pieces: Vec<TextObject> = ranges.into_iter().filter_map(|r| src.piece(r, by == By::Paragraphs)).collect();
        if pieces.is_empty() {
            skipped.push(id);
        } else {
            items.push((id, node.clone(), pieces));
        }
    }
    if items.is_empty() {
        return Err(EngineError::Other("Select horizontal point or area type whose text fits".into()));
    }
    let ids = replace_with_pieces(s, "Break Text", items)?;
    Ok(json!({ "ids": ids, "skipped": ids_json(&skipped) }))
}

fn split_text(s: &mut Session, _: &Value) -> Result<Value> {
    let Some((id, a, b)) = editing_range(s)? else {
        return Err(EngineError::Other("Click in type with a Type tool to split it".into()));
    };
    let d = &s.doc()?.doc;
    let (Some(src), Some(node)) = (Source::of(d, id), d.node(id).filter(|_| d.is_editable(id))) else {
        return Err(EngineError::Other("Select horizontal point or area type whose text fits".into()));
    };
    let text = &src.text;
    let (a, b) = (edit::floor_char(text, a.min(b)), edit::floor_char(text, a.max(b)));
    let cuts: Vec<(usize, usize)> = if a == b { vec![(0, a), (a, text.len())] } else { vec![(0, a), (a, b), (b, text.len())] };
    let pieces: Vec<TextObject> = cuts.into_iter().filter_map(|r| src.piece(r, true)).collect();
    if pieces.len() < 2 {
        return Err(EngineError::Other("Nothing to split at the cursor".into()));
    }
    let node = node.clone();
    let ids = replace_with_pieces(s, "Split Text at Cursor", vec![(id, node, pieces)])?;
    // The text edited is gone: the pieces stay selected.
    s.set_tool_option("stopEditing", &Value::Bool(true));
    Ok(json!({ "ids": ids }))
}

/// One piece of point type Combine Point Text takes: its text with its object's scale taken into
/// its characters, and where it starts and ends along its line in the frame of its turn and shear.
struct Piece {
    id: NodeId,
    t: TextObject,
    /// The object's turn and shear (`xf` without its move).
    turn: [f64; 4],
    /// Where its first character's glyph starts on its baseline, in that frame.
    at: Point,
    /// Where its last glyph ends along the line, in that frame.
    end: f64,
    /// Its first character's size.
    size: f64,
    /// Each character's pen position from the first's (byte, x), as it lays out alone.
    pens: Vec<(usize, f64)>,
}

impl Piece {
    fn of(d: &Document, id: NodeId) -> Option<(Self, &Node)> {
        let node = d.node(id)?;
        let NodeKind::Text(t) = &node.kind else { return None };
        let text = t.plain_text();
        if t.vertical || !matches!(t.kind, TextKind::Point) || text.trim().is_empty() || text.contains(['\n', '\u{2028}']) {
            return None;
        }
        let mut t = (**t).clone();
        t.scale_text_attributes();
        t.style_scale()?;
        let lay = layout(&t);
        let line = lay.lines.first().filter(|l| !l.rtl && lay.lines.len() == 1)?;
        let first = glyph_at(&lay, 0)?;
        // The first glyph of each cluster (a mark drawn over it shares its byte).
        let mut seen = std::collections::HashSet::new();
        let pens = lay.glyphs.iter().filter(|g| g.len > 0 && seen.insert(g.byte)).map(|g| (g.byte, g.origin.x - first.x)).collect();
        let end = lay.glyphs.iter().map(|g| g.origin.x + g.advance).fold(first.x, f64::max) - first.x;
        let [a, b, c, dd, _, _] = t.xf.as_coeffs();
        let turn = Affine::new([a, b, c, dd, 0.0, 0.0]);
        let at = turn.inverse() * (t.xf * Point::new(first.x, line.baseline));
        let size = t.runs.first()?.style.size;
        if !(at.x.is_finite() && at.y.is_finite() && size > 0.0) {
            return None;
        }
        Some((Self { id, t, turn: [a, b, c, dd], at, end: at.x + end, size, pens }, node))
    }

    /// Whether `o` is turned and sheared alike.
    fn turned_like(&self, o: &Piece) -> bool {
        self.turn.iter().zip(&o.turn).all(|(x, y)| (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.0))
    }
}

/// Whether two objects look alike apart from their text (paint, opacity, blending, effects).
fn same_look(a: &Node, b: &Node) -> bool {
    a.appearance == b.appearance
        && a.opacity == b.opacity
        && a.blend == b.blend
        && a.isolate == b.isolate
        && a.knockout == b.knockout
        && a.extras == b.extras
}

/// How wide a space of `style` is.
fn space_width(style: &CharStyle) -> f64 {
    let probe = TextObject::point(Point::ZERO, " ", CharStyle { tracking: 0.0, ..style.clone() });
    layout(&probe).glyphs.first().map_or(style.size * 0.25, |g| g.advance)
}

/// `chain` (pieces left to right on one line) as one point type object whose glyphs land where
/// theirs were, or the index of the first piece that can't join the ones before it so.
fn merge(chain: &[&Piece]) -> std::result::Result<TextObject, usize> {
    let Some(first) = chain.first() else { return Err(0) };
    let mut runs = vec![];
    // Each piece's first byte, and the byte range of the character before it (whose tracking sets
    // the gap).
    let (mut starts, mut before) = (vec![], vec![]);
    let mut text = String::new();
    for (k, p) in chain.iter().enumerate() {
        if let Some(prev) = k.checked_sub(1).and_then(|i| chain.get(i)) {
            let starts_blank = p.t.plain_text().starts_with(char::is_whitespace);
            let last = prev.t.runs.iter().rev().find(|r| !r.text.is_empty()).map(|r| r.style.clone()).unwrap_or_default();
            if !text.ends_with(char::is_whitespace) && !starts_blank && p.at.x - prev.end >= 0.5 * space_width(&last) {
                runs.push(vectorcraft_doc::TextRun::new(" ", last));
                text.push(' ');
            }
            let c = text.chars().next_back().map_or(1, char::len_utf8);
            before.push((text.len() - c, text.len()));
        }
        starts.push(text.len());
        // Baselines a little apart: the difference is baseline shift (up is positive).
        let shift = first.at.y - p.at.y;
        for r in &p.t.runs {
            let mut r = r.clone();
            r.style.baseline_shift += shift;
            text.push_str(&r.text);
            runs.push(r);
        }
    }
    let mut m = first.t.clone();
    m.set_all_paras(ParaStyle { justify: Justify::Left, ..first.t.para.clone() });
    let [a, b, c, d] = first.turn;
    let base = Affine::new([a, b, c, d, 0.0, 0.0]) * Affine::translate(first.at.to_vec2());
    m.xf = base;
    m.runs = runs;
    m.ruby.clear();
    // Tracking after the character before each piece, until each lands where it was.
    let targets: Vec<f64> = chain.iter().map(|p| p.at.x - first.at.x).collect();
    let mut lay = layout(&m);
    for _ in 0..4 {
        // A piece whose first character joins the one before it (a ligature) can't keep its place.
        let xs: Vec<f64> = match starts.iter().enumerate().map(|(k, b)| glyph_starting(&lay, *b).map(|g| g.x).ok_or(k)).collect() {
            Ok(xs) => xs,
            Err(k) => return Err(k.max(1)),
        };
        let x0 = xs.first().copied().unwrap_or(0.0);
        let off: Vec<f64> = targets.iter().zip(&xs).map(|(t, x)| t - (x - x0)).collect();
        if off.iter().all(|o| o.abs() <= TOLERANCE * 0.5) {
            break;
        }
        // Moving piece k moves those after it as far.
        for (k, w) in off.windows(2).enumerate() {
            let (delta, Some(&(s, e))) = (w[1] - w[0], before.get(k)) else { continue };
            if delta.abs() > 1e-9 {
                edit::style_range(&mut m.runs, s, e, |st| st.tracking += delta / st.size.max(1e-6) * 1000.0);
                if !TRACKING.contains(&edit::style_at(&m.runs, s).tracking) {
                    return Err(k + 1);
                }
            }
        }
        lay = layout(&m);
    }
    // Every glyph where it was, within the tolerance; else the run ends before the piece that moved.
    let x0 = glyph_starting(&lay, 0).map_or(0.0, |g| g.x);
    for (k, p) in chain.iter().enumerate() {
        let (Some(&start), Some(&target)) = (starts.get(k), targets.get(k)) else { return Err(k.max(1)) };
        let moved = p.pens.iter().any(|(byte, x)| glyph_starting(&lay, start + byte).is_none_or(|g| (g.x - x0 - target - x).abs() > TOLERANCE));
        if moved {
            return Err(k.max(1));
        }
    }
    m.xf = base * Affine::translate((-x0, 0.0));
    Ok(m)
}

fn combine_text(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.combineText";
    let d = &s.doc()?.doc;
    let mut pieces: Vec<(Piece, &Node)> =
        texts_under(d, &checked_ids_or_selection(s, p, C)?)?.into_iter().filter_map(|id| Piece::of(d, id)).collect();
    // Lines: pieces turned alike and looking alike, by baseline, then left to right.
    pieces.sort_by(|x, y| x.0.at.y.total_cmp(&y.0.at.y).then(x.0.at.x.total_cmp(&y.0.at.x)));
    let mut lines: Vec<Vec<(&Piece, &Node)>> = vec![];
    for (piece, node) in &pieces {
        let fits = |l: &Vec<(&Piece, &Node)>| {
            l.first()
                .is_some_and(|(f, n)| f.turned_like(piece) && same_look(n, node) && (f.at.y - piece.at.y).abs() <= SAME_LINE * f.size.min(piece.size))
        };
        match lines.iter_mut().find(|l| fits(l)) {
            Some(l) => l.push((piece, node)),
            None => lines.push(vec![(piece, node)]),
        }
    }
    // Runs along each line: a wide gap or an overlap starts another.
    let mut chains: Vec<Vec<&Piece>> = vec![];
    for mut l in lines {
        l.sort_by(|x, y| x.0.at.x.total_cmp(&y.0.at.x));
        let mut chain: Vec<&Piece> = vec![];
        for (piece, _) in l {
            if let Some(prev) = chain.last() {
                let gap = piece.at.x - prev.end;
                if gap > MAX_GAP * prev.size || gap < -MAX_OVERLAP * prev.size {
                    chains.push(std::mem::take(&mut chain));
                }
            }
            chain.push(piece);
        }
        chains.push(chain);
    }
    // Each run as one object, or as several where a piece can't keep its place joined.
    let mut made: Vec<(NodeId, TextObject, Vec<NodeId>)> = vec![];
    let mut tries = 0;
    'chains: for chain in chains {
        let mut rest: &[&Piece] = &chain;
        while rest.len() > 1 {
            let mut upto = rest.len();
            while let Some(part) = rest.get(..upto) {
                tries += part.len();
                if tries > MAX_JOIN_TRIES {
                    break 'chains;
                }
                match merge(part) {
                    Ok(m) => {
                        let gone = part.iter().skip(1).map(|p| p.id).collect();
                        if let Some(first) = part.first() {
                            made.push((first.id, m, gone));
                        }
                        break;
                    }
                    // A run of one stays as it is.
                    Err(k) if k <= 1 => {
                        upto = 1;
                        break;
                    }
                    Err(k) => upto = k,
                }
            }
            rest = rest.get(upto.max(1)..).unwrap_or_default();
        }
    }
    if made.is_empty() {
        return Err(EngineError::Other("Select two or more one-line point type objects on one baseline to combine".into()));
    }
    let combined: usize = made.iter().map(|m| m.2.len()).sum();
    let ids: Vec<u64> = made.iter().map(|m| m.0.0).collect();
    s.edit("Combine Point Text", |d, sel| {
        let mut gone = vec![];
        for (id, mut t, others) in made {
            refresh_bounds(&mut t);
            if let Some(n) = d.node_mut(id) {
                n.kind = NodeKind::Text(Box::new(t));
            }
            gone.extend(others);
        }
        d.remove_many(&gone)?;
        sel.set(ids.iter().map(|i| NodeId(*i)));
        Ok(())
    })?;
    Ok(json!({ "ids": ids, "combined": combined }))
}

/// Remove the breaks `brk` in bytes `a..b` of `t`: one between two words becomes a space → how many
/// went.
fn strip_breaks(t: &mut TextObject, a: usize, b: usize, brk: char) -> usize {
    let text = t.plain_text();
    let (a, b) = (edit::floor_char(&text, a), edit::floor_char(&text, b));
    let Some(range) = text.get(a..b) else { return 0 };
    let mut prev = text.get(..a).and_then(|s| s.chars().next_back());
    let mut cuts = vec![];
    for (i, c) in range.char_indices() {
        let at = a + i;
        if c != brk {
            prev = Some(c);
            continue;
        }
        // The next character left: past the breaks that go too.
        let after = at + c.len_utf8();
        let next = text.get(after..).and_then(|rest| rest.char_indices().find(|(j, n)| *n != brk || after + j >= b).map(|(_, n)| n));
        let space = prev.is_some_and(|p| !p.is_whitespace()) && next.is_some_and(|n| !n.is_whitespace());
        if space {
            prev = Some(' ');
        }
        cuts.push((at, space));
    }
    for &(at, space) in cuts.iter().rev() {
        let insert = if space { " " } else { "" };
        let end = at + brk.len_utf8();
        t.splice_paras(at, end, insert);
        edit::replace_range(&mut t.runs, at, end, insert);
        t.ruby_after_replace(at, end, insert.len());
    }
    cuts.len()
}

fn remove_breaks(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.removeBreaks";
    let (brk, label, none) = match text_param(p, "kind", C)? {
        Some("line") => ('\u{2028}', "Remove Line Breaks", "No line breaks to remove"),
        Some("paragraph") => ('\n', "Remove Paragraph Breaks", "No paragraph breaks to remove"),
        Some(other) => return Err(bad(C, format!("`kind` must be \"line\" or \"paragraph\", not {other:?}"))),
        None => return Err(bad(C, "missing `kind`")),
    };
    // The characters a Type tool has selected, else all the text of the selected type.
    let editing = if p.get("ids").is_some() { None } else { editing_range(s)?.filter(|(_, a, b)| a != b) };
    let d = &s.doc()?.doc;
    let targets: Vec<(NodeId, usize, usize)> = match editing {
        Some((id, a, b)) => vec![(id, a.min(b), a.max(b))].into_iter().filter(|(id, ..)| d.is_editable(*id)).collect(),
        None => texts_under(d, &checked_ids_or_selection(s, p, C)?)?.into_iter().map(|id| (id, 0, usize::MAX)).collect(),
    };
    let count: usize = targets
        .iter()
        .filter_map(|(id, a, b)| {
            let text = text_of(d, *id)?.plain_text();
            let (a, b) = (edit::floor_char(&text, *a), edit::floor_char(&text, *b));
            Some(text.get(a..b)?.matches(brk).count())
        })
        .sum();
    if count == 0 {
        return Err(EngineError::Other(none.into()));
    }
    if count > MAX_BREAKS {
        return Err(EngineError::Other("Too many breaks to remove at once".into()));
    }
    let (removed, ids) = s.edit(label, |d, _| {
        let (mut removed, mut ids) = (0, vec![]);
        for (id, a, b) in targets {
            let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { continue };
            let n = strip_breaks(t, a, b, brk);
            if n > 0 {
                refresh_bounds(t);
                removed += n;
                ids.push(id.0);
            }
        }
        Ok((removed, ids))
    })?;
    Ok(json!({ "ids": ids, "removed": removed }))
}

/// Horizontal point type among the objects `p` names.
fn point_texts(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids: Vec<NodeId> = texts_under(d, &checked_ids_or_selection(s, p, cmd)?)?
        .into_iter()
        .filter(|id| text_of(d, *id).is_some_and(|t| matches!(t.kind, TextKind::Point)))
        .collect();
    if ids.is_empty() {
        return Err(EngineError::Other("Select point type".into()));
    }
    Ok(ids)
}

/// Replace the type objects of `changed` as one undo step labelled `label` → their ids.
fn set_texts(s: &mut Session, label: &str, changed: Vec<(NodeId, TextObject)>, reset_angle: bool) -> Result<Value> {
    let ids: Vec<u64> = changed.iter().map(|c| c.0.0).collect();
    s.edit(label, |d, _| {
        for (id, mut t) in changed {
            refresh_bounds(&mut t);
            if let Some(n) = d.node_mut(id) {
                n.kind = NodeKind::Text(Box::new(t));
                if reset_angle {
                    n.bbox_angle = 0.0;
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids }))
}

fn align_in_place(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.alignInPlace";
    let justify = match text_param(p, "align", C)? {
        Some("left") => Justify::Left,
        Some("center") => Justify::Center,
        Some("right") => Justify::Right,
        Some(other) => return Err(bad(C, format!("`align` must be \"left\", \"center\" or \"right\", not {other:?}"))),
        None => return Err(bad(C, "missing `align`")),
    };
    let ids = point_texts(s, p, C)?;
    let d = &s.doc()?.doc;
    let mut changed = vec![];
    for id in ids {
        let Some(t) = text_of(d, id).filter(|t| !t.vertical && t.para_styles().any(|para| para.justify != justify)) else { continue };
        let was = layout(t);
        // The longest line stays where it is.
        let Some((i, line)) = was.lines.iter().enumerate().max_by(|a, b| (a.1.x1 - a.1.x0).total_cmp(&(b.1.x1 - b.1.x0))) else { continue };
        let mut n = t.clone();
        n.edit_paras(None, |para| para.justify = justify);
        let Some(now) = layout(&n).lines.get(i).map(|l| l.x0) else { continue };
        n.xf *= Affine::translate((line.x0 - now, 0.0));
        changed.push((id, n));
    }
    if changed.is_empty() {
        return Err(EngineError::Other("The point type is aligned that way already".into()));
    }
    set_texts(s, "Set Alignment, Keep Position", changed, false)
}

fn reset_transform(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.resetTextTransform";
    let scale = switch_or(p, "scale", false, C)?;
    let ids = point_texts(s, p, C)?;
    let d = &s.doc()?.doc;
    let mut changed = vec![];
    for id in ids {
        let Some(t) = text_of(d, id) else { continue };
        let [a, b, c, dd, _, _] = t.xf.as_coeffs();
        // The object's own axes: its width along its baseline, and its height square to it.
        let (sx, det) = (a.hypot(b), a * dd - b * c);
        if !(sx > 1e-12 && sx.is_finite() && det.is_finite() && det.abs() > 1e-12) {
            continue;
        }
        let sy = det.abs() / sx;
        let unscaled = |t: &TextObject| t.runs.iter().all(|r| r.style.h_scale == 100.0);
        let upright = b.abs() <= 1e-9 * sx && c.abs() <= 1e-9 * sy && a > 0.0 && dd > 0.0;
        if upright && (!scale || ((sx - sy).abs() <= 1e-9 * sy && unscaled(t))) {
            continue;
        }
        let mut n = t.clone();
        let w = if scale {
            n.runs.iter_mut().for_each(|r| r.style.h_scale = 100.0);
            sy
        } else {
            sx
        };
        // The centre of the text stays where it is.
        let centre = t.xf * layout(t).bounds.center();
        let turn = Affine::scale_non_uniform(w, sy);
        let now = turn * layout(&n).bounds.center();
        n.xf = Affine::translate(centre - now) * turn;
        changed.push((id, n));
    }
    if changed.is_empty() {
        return Err(EngineError::Other("The point type has no transform to reset".into()));
    }
    set_texts(s, "Reset Text Transform", changed, true)
}

fn replace_fonts(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.cleanup.replaceFonts";
    let default = CharStyle::default().font_family;
    let wanted = text_param(p, "font", C)?.filter(|f| !f.trim().is_empty()).unwrap_or(&default);
    let db = FontDb::global();
    // Any name of the family is stored as its own name.
    let family = match db.resolve(wanted, "Regular") {
        Some((f, m)) if m != FontMatch::Missing => f.family.clone(),
        _ => return Err(bad(C, "the replacement font is not available")),
    };
    let missing = |st: &CharStyle| db.resolve(&st.font_family, &st.font_style).is_none_or(|(_, m)| m == FontMatch::Missing);
    let d = &s.doc()?.doc;
    let mut ids = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind
                && t.runs.iter().any(|r| missing(&r.style))
            {
                ids.push(n.id);
            }
        });
    }
    if ids.is_empty() {
        return Err(EngineError::Other("No fonts are missing".into()));
    }
    let runs = s.edit("Replace Missing Fonts", |d, _| {
        let mut runs = 0;
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            for r in t.runs.iter_mut().filter(|r| missing(&r.style)) {
                // The style the font has closest to the one the character had.
                r.style.font_style = db.face(&family, &r.style.font_style).map_or_else(|| r.style.font_style.clone(), |f| f.style.clone());
                r.style.font_family = family.clone();
                r.style.font_version = None;
                r.style.font_axes.clear();
                runs += 1;
            }
            edit::normalize(&mut t.runs);
            refresh_bounds(t);
        }
        Ok(runs)
    })?;
    Ok(json!({ "runs": runs, "font": family }))
}
