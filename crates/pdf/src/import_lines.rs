//! What a file's lines of type were, beyond what each line says on its own (#508). An editor
//! writes what PDF text can't say as more art next to the text, and the type it wrapped in a
//! frame as lines:
//! - a stroke added to a type object (above or below its characters) comes as the outline of each
//!   glyph stroked, one path per glyph, right after the type (or right before it): it is the
//!   type's stroke again, so the type stays one object;
//! - area type comes as a line of point type per line, each wrapped line ending in a space. Lines
//!   in a row sharing a left edge, font, size and leading (and paragraph spacing) are one area
//!   type object again, aligned left or justified, where laying its text out in the frame breaks
//!   it where the file does: else, as where its font isn't installed, they stay lines.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Point, Rect, Shape};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{AppearanceItem, CharStyle, Justify, Node, NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::shapes;

use crate::import_text::{LineFacts, median};

/// Rebuild the type among `nodes` (and in their groups) from what its lines tell (`lines`).
pub(crate) fn rebuild(nodes: &mut Vec<Arc<Node>>, lines: &HashMap<NodeId, LineFacts>) {
    if lines.is_empty() {
        return;
    }
    for n in nodes.iter_mut() {
        if n.children().is_some_and(|c| !c.is_empty())
            && let Some(children) = Arc::make_mut(n).children_mut()
        {
            rebuild(children, lines);
        }
    }
    join_strokes(nodes, lines);
    join_paragraphs(nodes, lines);
}

/// The stroke paths `nodes` draw when they are the outlines of the glyphs whose ink boxes are
/// `ink`, stroked one by one alike (no fill, mask or blending, fully opaque).
fn outline_stroke(nodes: &[Arc<Node>], ink: &[Rect]) -> Option<AppearanceItem> {
    let first = nodes.first()?;
    let [stroke @ AppearanceItem::Stroke(_)] = first.appearance.items.as_slice() else { return None };
    let on_glyph = |n: &Node, r: &Rect| {
        let tol = 0.01 * (r.width() + r.height()) + 0.01;
        let NodeKind::Path { path, clipping: false, guide: false, live: None, .. } = &n.kind else { return false };
        let b = path.to_bezpath().bounding_box();
        n.appearance == first.appearance
            && n.extras().mask.is_none()
            && n.opacity >= 0.999
            && n.blend == BlendMode::Normal
            && [(b.x0, r.x0), (b.y0, r.y0), (b.x1, r.x1), (b.y1, r.y1)].iter().all(|(a, b)| (a - b).abs() <= tol)
    };
    (nodes.len() == ink.len() && nodes.iter().zip(ink).all(|(n, r)| on_glyph(n, r))).then(|| stroke.clone())
}

/// Each type object among `nodes` whose glyphs' outlines are stroked by the paths right after it
/// (or right before it) takes that stroke above (or below) its characters, and the paths go.
fn join_strokes(nodes: &mut Vec<Arc<Node>>, lines: &HashMap<NodeId, LineFacts>) {
    let mut i = 0;
    while i < nodes.len() {
        let ink = nodes
            .get(i)
            .filter(|n| matches!(n.kind, NodeKind::Text(_)) && n.appearance.items.is_empty())
            .and_then(|n| lines.get(&n.id))
            .map_or(&[][..], |f| f.ink.as_slice());
        let k = ink.len();
        if k > 0 {
            if let Some(stroke) = nodes.get(i + 1..i + 1 + k).and_then(|after| outline_stroke(after, ink)) {
                nodes.drain(i + 1..i + 1 + k);
                if let Some(t) = nodes.get_mut(i) {
                    Arc::make_mut(t).appearance.items.push(stroke);
                }
            } else if let Some(stroke) = i.checked_sub(k).and_then(|from| nodes.get(from..i)).and_then(|before| outline_stroke(before, ink)) {
                nodes.drain(i - k..i);
                i -= k;
                if let Some(t) = nodes.get_mut(i) {
                    let ap = &mut Arc::make_mut(t).appearance;
                    ap.items.insert(0, stroke);
                    ap.set_contents_at(1);
                }
            }
        }
        i += 1;
    }
}

/// `n` as a line that can be part of area type: horizontal point type, with what its line told.
fn line<'a>(n: &'a Node, lines: &'a HashMap<NodeId, LineFacts>) -> Option<(&'a TextObject, &'a LineFacts)> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let point = matches!(t.kind, TextKind::Point) && !t.vertical && n.extras().mask.is_none() && !t.runs.is_empty();
    point.then_some(())?;
    Some((t, lines.get(&n.id)?))
}

/// Lines of type in a row that may be one area type object.
struct Block {
    /// The step from a wrapped line's baseline to the next.
    leading: f64,
    /// The extra step after a line ending a paragraph.
    space: f64,
    /// Each line's baseline below the first (text space).
    ys: Vec<f64>,
}

/// The lines from `nodes`' first on that share its left edge, angle, font and size, its look
/// (opacity, blending, appearance) and steps down by one leading after a wrapped line and by
/// another (the leading and paragraph spacing) after a line ending a paragraph: how many, and
/// the block they make unless none of them was wrapped (0 and `None`: the first isn't a line).
fn block(nodes: &[Arc<Node>], lines: &HashMap<NodeId, LineFacts>) -> (usize, Option<Block>) {
    let Some((head, (t0, f0))) = nodes.first().and_then(|n| Some((n, line(n, lines)?))) else { return (0, None) };
    let st = t0.first_style();
    let size = st.size;
    let inv = t0.xf.inverse();
    let lin = |t: &TextObject| {
        let [a, b, c, d, ..] = t.xf.as_coeffs();
        [a, b, c, d]
    };
    let (mut ys, mut wraps) = (vec![0.0], f0.wraps);
    let (mut leading, mut para): (Option<f64>, Option<f64>) = (None, None);
    for n in nodes.iter().skip(1) {
        let Some((t, f)) = line(n, lines) else { break };
        let s = t.first_style();
        let alike = n.opacity == head.opacity
            && n.blend == head.blend
            && n.appearance == head.appearance
            && s.font_family == st.font_family
            && s.font_style == st.font_style
            && (s.size - size).abs() <= size * 0.01
            && t.para == t0.para
            && lin(t).iter().zip(lin(t0)).all(|(a, b)| (a - b).abs() < 1e-6);
        let at = inv * (t.xf * Point::ZERO);
        let step = at.y - ys.last().copied().unwrap_or_default();
        if !alike || at.x.abs() > size * 0.02 || !(step > size * 0.5 && step < size * 4.0) {
            break;
        }
        let slot = if wraps { &mut leading } else { &mut para };
        match *slot {
            Some(s) if (s - step).abs() > size * 0.01 => break,
            Some(_) => {}
            None => *slot = Some(step),
        }
        ys.push(at.y);
        wraps = f.wraps;
    }
    let count = ys.len();
    let Some(leading) = leading else { return (count, None) };
    let space = para.map_or(0.0, |p| p - leading);
    (count, (space > -size * 0.01).then_some(Block { leading, space: space.max(0.0), ys }))
}

/// How far a line's characters' tracking may stray (thousandths of an em) and still be one
/// value: placing each glyph where the file put it (#722) tracks characters a little apart where
/// the file's widths are rounded, or it moved a glyph a hair (`TJ`).
const EVEN_TRACKING: f64 = 10.0;

/// A line's spacing: the tracking after its letters, and how much more its spaces take (word
/// spacing, `Tw`; `None` when it has none). `None` when its letters are tracked apart (set by
/// hand), or its spaces more than `slack` apart.
fn line_spacing(t: &TextObject, slack: f64) -> Option<(f64, Option<f64>)> {
    // The tracking of the runs with characters that are spaces (`true`) or that aren't.
    let tracking = |space: bool| -> Vec<f64> {
        t.runs.iter().filter(|r| r.text.chars().any(|c| c.is_whitespace() == space)).map(|r| r.style.tracking).collect()
    };
    let letters = tracking(false);
    let one = median(letters.clone())?;
    letters.iter().all(|x| (x - one).abs() <= EVEN_TRACKING).then_some(())?;
    let spaces = tracking(true);
    let word = match median(spaces.clone()) {
        Some(s) => {
            spaces.iter().all(|x| (x - s).abs() <= slack).then_some(())?;
            Some(s - one)
        }
        None => None,
    };
    Some((one, word))
}

/// The one word spacing (thousandths of an em past the letters' tracking) of lines spaced
/// `words` (`None`: no spaces): 0 for lines without.
fn one_word_spacing(words: &[Option<f64>]) -> Option<f64> {
    let w: Vec<f64> = words.iter().flatten().copied().collect();
    let Some(one) = median(w.clone()) else { return Some(0.0) };
    w.iter().all(|x| (x - one).abs() <= EVEN_TRACKING).then_some(if one.abs() <= EVEN_TRACKING { 0.0 } else { one })
}

/// Lines `nodes` of block `b` as one area type object, aligned left or justified, when laying it
/// out breaks its lines where the file does.
fn area_type(nodes: &[Arc<Node>], lines: &HashMap<NodeId, LineFacts>, b: &Block) -> Option<TextObject> {
    let texts: Vec<(&TextObject, &LineFacts)> = nodes.iter().map(|n| line(n, lines)).collect::<Option<_>>()?;
    let (first, _) = texts.first()?;
    let size = first.first_style().size;
    // Each line's tracking and word spacing. A justified line spaces its words out (word spacing,
    // `Tw`), which justification does again (#722); word spacing the same on every line is the
    // type's own.
    // A wrapped line's spaces may each be tracked apart, within a fiftieth of an em of each other.
    let spacings: Vec<(f64, Option<f64>)> =
        texts.iter().map(|(t, f)| line_spacing(t, if f.wraps { 20.0 } else { EVEN_TRACKING })).collect::<Option<_>>()?;
    let trackings: Vec<f64> = spacings.iter().map(|(t, _)| *t).collect();
    let words: Vec<Option<f64>> = spacings.iter().map(|(_, w)| *w).collect();
    let wrapped: Vec<f64> = texts.iter().filter(|(_, f)| f.wraps).map(|(_, f)| f.length).collect();
    let longest = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
    let mut tries = vec![];
    // Wrapped lines all as long: justified (a paragraph's last line aligned left), tracked and
    // word spaced as the lines that end paragraphs are (the others are spaced out).
    let full = longest(&wrapped);
    let ends: Vec<usize> = texts.iter().enumerate().filter(|(_, (_, f))| !f.wraps).map(|(i, _)| i).collect();
    let end_words: Vec<Option<f64>> = ends.iter().filter_map(|i| words.get(*i).copied()).collect();
    if wrapped.len() >= 2
        && wrapped.iter().all(|l| (full - l).abs() <= size * 0.02)
        && let Some(word) = one_word_spacing(&end_words)
    {
        let tracking =
            median(ends.iter().filter_map(|i| trackings.get(*i).copied()).collect()).or_else(|| trackings.iter().copied().reduce(f64::min))?;
        tries.push((Justify::JustifyLeft, full, tracking, word));
    }
    // Every line spaced alike: aligned left.
    if let Some(word) = one_word_spacing(&words) {
        let all: Vec<f64> = texts.iter().map(|(_, f)| f.length).collect();
        tries.push((Justify::Left, longest(&all) + size * 0.02, median(trackings)?, word));
    }
    tries.into_iter().find_map(|(justify, width, tracking, word)| laid_out(&texts, b, justify, width, tracking, word))
}

/// The area type of lines `texts` (block `b`): `width` wide, aligned `justify`, its characters
/// tracked `tracking` and its spaces `word` more; `None` when it doesn't lay out as the lines did.
fn laid_out(texts: &[(&TextObject, &LineFacts)], b: &Block, justify: Justify, width: f64, tracking: f64, word: f64) -> Option<TextObject> {
    let (first, _) = texts.first()?;
    let size = first.first_style().size;
    // Leading as set, unless it is Auto's (120% of the size).
    let explicit =
        (b.leading - size * 1.2).abs() > size * 1e-3 || texts.iter().any(|(t, _)| t.runs.iter().any(|r| (r.style.size - size).abs() > 1e-9));
    let mut runs: Vec<TextRun> = vec![];
    let mut push = |text: &str, style: CharStyle| match runs.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        _ => runs.push(TextRun { text: text.to_string(), style, inline: None }),
    };
    for (k, (t, f)) in texts.iter().enumerate() {
        let mut style = CharStyle::default();
        for r in &t.runs {
            style = r.style.clone();
            style.tracking = if r.text.trim().is_empty() { tracking + word } else { tracking };
            style.leading = explicit.then_some((b.leading * 1000.0).round() / 1000.0);
            push(&r.text, style.clone());
        }
        // A wrapped line's space, or the end of a paragraph.
        if k + 1 < texts.len() {
            if f.wraps {
                style.tracking = tracking + word;
                push(" ", style);
            } else {
                push("\n", style);
            }
        }
    }
    let mut t = (*first).clone();
    t.runs = runs;
    t.para.justify = justify;
    t.para.space_before = b.space;
    let last = b.ys.last().copied()?;
    let db = vectorcraft_text::FontDb::global();
    // The frame's top puts the first baseline where the file has it.
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, width, last + b.leading.max(size) + size)) };
    let top = -vectorcraft_text::layout(db, &t).lines.first()?.baseline;
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, top, width, last + b.leading.max(size))) };
    let laid = vectorcraft_text::layout(db, &t);
    let plain = t.plain_text();
    let same = !laid.overflow
        && laid.lines.len() == texts.len()
        && laid.lines.iter().zip(texts).zip(&b.ys).all(|((l, (line, f)), y)| {
            let text = plain.get(l.start..l.end).unwrap_or_default();
            text.trim() == line.plain_text().trim()
                && (l.baseline - y).abs() <= size * 0.05
                && (justify != Justify::JustifyLeft || !f.wraps || (l.x1 - f.length).abs() <= size * 0.05)
        });
    t.cached_bounds = Some(laid.bounds);
    same.then_some(t)
}

/// Lines of point type in a row among `nodes` that were wrapped in a frame become one area type
/// object again (see [`area_type`]).
fn join_paragraphs(nodes: &mut Vec<Arc<Node>>, lines: &HashMap<NodeId, LineFacts>) {
    let mut i = 0;
    while i < nodes.len() {
        let (count, b) = nodes.get(i..).map_or((0, None), |rest| block(rest, lines));
        let area = b.filter(|_| count > 1).and_then(|b| area_type(nodes.get(i..i + count)?, lines, &b));
        if let Some(area) = area
            && let Some(head) = nodes.get_mut(i)
        {
            Arc::make_mut(head).kind = NodeKind::Text(Box::new(area));
            nodes.drain(i + 1..i + count);
            i += 1;
        } else {
            // Lines that don't make area type: none of them starts it either.
            i += count.max(1);
        }
    }
}
