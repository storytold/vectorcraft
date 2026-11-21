//! Paragraphs for Export As Text (`paragraphs: true`). A PDF, and the files other programs write
//! from one, often holds a line of point type per line that was set, so its paragraphs come as
//! loose lines. Lines that belong together share an artboard (a PDF's page), a font family and
//! size, boldness (a heading is a block of its own), a left edge and evenly spaced baselines:
//! they make a block. A block whose first baseline
//! lines up with another's just to its right continues there (columns, read left to right; the
//! gutter at most as wide as the block). Inside that run of lines a paragraph
//! ends:
//! - in justified lines (most lines as long as the longest, and each of the others ending a
//!   sentence or a lead-in), after a line clearly shorter;
//! - in ragged lines, after a shorter line that ends a sentence;
//! - before a line that starts bold among lines that don't, and between a line mostly bold (a
//!   heading) and one that isn't;
//! - before a line indented past the block's left edge (a first-line indent).
//!
//! Widths come from laying each line out, so where the file's font isn't installed its fallback
//! still tells long lines from short ones.

use vectorcraft_doc::{NodeId, TextKind, TextObject};
use vectorcraft_text::FontDb;

/// One line of horizontal point type, in document space.
#[derive(Clone)]
pub(super) struct Line {
    pub id: NodeId,
    pub text: String,
    pub left: f64,
    pub baseline: f64,
    pub size: f64,
    family: String,
    pub width: f64,
    /// It starts in a bold (or heavier) style.
    strong: bool,
    /// Most of it is bold (or heavier): a heading, as against a paragraph's lead-in.
    bold: bool,
    /// The artboard its start is on (`None`: on none).
    pub board: Option<usize>,
}

/// Whether `style` is bold or heavier.
fn is_strong(style: &str) -> bool {
    let style = style.to_ascii_lowercase();
    ["bold", "black", "heavy", "semibold", "demi"].iter().any(|w| style.contains(w))
}

impl Line {
    /// `t` as a line: one line of horizontal point type with characters, neither turned nor
    /// skewed (scaled alike both ways at most).
    pub(super) fn of(id: NodeId, t: &TextObject) -> Option<Line> {
        if t.vertical || !matches!(t.kind, TextKind::Point) {
            return None;
        }
        let text = t.plain_text();
        if text.contains(['\n', '\r', '\u{2028}']) || text.trim().is_empty() {
            return None;
        }
        let [a, b, c, d, e, f] = t.xf.as_coeffs();
        if !(a.is_finite() && a > 0.0) || b.abs() > a * 1e-6 || c.abs() > a * 1e-6 || (d - a).abs() > a * 1e-3 {
            return None;
        }
        let laid = vectorcraft_text::layout(FontDb::global(), t);
        let first = laid.lines.first()?;
        let run = t.runs.iter().find(|r| !r.text.trim().is_empty())?;
        let size = t.runs.iter().map(|r| r.style.size).fold(0.0, f64::max) * a;
        let (left, baseline, width) = (e + first.x0 * a, f + first.baseline * a, (first.x1 - first.x0) * a);
        if ![size, left, baseline, width].iter().all(|v| v.is_finite()) || size <= 0.0 {
            return None;
        }
        let strong = is_strong(&run.style.font_style);
        // Most of its letters and digits bold: separators and symbols set in another style don't count.
        let letters = |strong: bool| -> usize {
            let runs = t.runs.iter().filter(|r| !strong || is_strong(&r.style.font_style));
            runs.map(|r| r.text.chars().filter(|c| c.is_alphanumeric()).count()).sum()
        };
        let bold = letters(true) * 2 > letters(false);
        Some(Line { id, text, left, baseline, size, family: run.style.font_family.clone(), width: width.max(0.0), strong, bold, board: None })
    }

    /// Lines of one block share this: the artboard, the family, the size (to a hundredth of a
    /// point) and whether they are mostly bold. A heading set in the body's family and size is a
    /// block of its own, so the larger space below it doesn't become the body's leading and cut
    /// the body's first line off from the rest.
    fn key(&self) -> (Option<usize>, &str, i64, bool) {
        (self.board, self.family.as_str(), (self.size * 100.0).round() as i64, self.bold)
    }
}

/// Lines of one block, top to bottom.
struct Block {
    lines: Vec<usize>,
    left: f64,
    top: f64,
    last: f64,
    /// The distance between its baselines (`None` while it has one line).
    leading: Option<f64>,
}

/// How far a left edge may stray and still be the block's (ems).
const EDGE: f64 = 0.15;
/// The most a first-line indent is (ems).
const INDENT: f64 = 4.0;
/// Baselines this far apart, at least and at most, may be lines of one block (ems).
const LEADING: (f64, f64) = (0.75, 2.5);
/// How much the distance between baselines may vary in a block (of the leading).
const EVEN: f64 = 0.1;
/// In justified lines, a line shorter than this (of the longest) ends its paragraph.
const SHORT_JUSTIFIED: f64 = 0.9;
/// In ragged lines, a line that ends a sentence and is shorter than this ends its paragraph.
const SHORT_RAGGED: f64 = 0.8;
/// Lines at least this long (of the longest) count as full.
const FULL: f64 = 0.97;

/// A paragraph that lines of point type make.
pub(super) struct Group {
    /// Indices of its lines in reading order.
    pub lines: Vec<usize>,
    /// Its lines are justified (most as long as the longest).
    pub justified: bool,
    /// The distance between its baselines (`None`: a line on its own).
    pub leading: Option<f64>,
}

/// The paragraphs `lines` make. Every line is in exactly one; the paragraphs are in the order
/// their first lines are in `lines`.
pub(super) fn paragraphs(lines: &[Line]) -> Vec<Group> {
    let mut order: Vec<usize> = (0..lines.len()).collect();
    let at = |i: usize| lines.get(i);
    order.sort_by(|&i, &j| match (at(i), at(j)) {
        (Some(a), Some(b)) => a.key().cmp(&b.key()).then(a.baseline.total_cmp(&b.baseline)).then(a.left.total_cmp(&b.left)),
        _ => i.cmp(&j),
    });
    let mut indented = vec![false; lines.len()];
    let mut out: Vec<Group> = vec![];
    let mut start = 0;
    while start < order.len() {
        let key = order.get(start).and_then(|&i| at(i)).map(Line::key);
        let end = order.iter().skip(start).position(|&i| at(i).map(Line::key) != key).map_or(order.len(), |n| start + n);
        let group = order.get(start..end).unwrap_or_default();
        let blocks = blocks(lines, group, &mut indented);
        for chain in columns(lines, &blocks) {
            let seq: Vec<usize> = chain.iter().filter_map(|&b| blocks.get(b)).flat_map(|b| b.lines.iter().copied()).collect();
            let leading = chain.iter().filter_map(|&b| blocks.get(b)?.leading).next();
            let (paras, justified) = split(lines, &seq, &indented);
            out.extend(paras.into_iter().map(|p| Group { leading: leading.filter(|_| p.len() > 1), lines: p, justified }));
        }
        start = end.max(start + 1);
    }
    out.sort_by_key(|p| p.lines.iter().copied().min().unwrap_or(usize::MAX));
    out
}

/// The blocks of `group` (lines of one family and size, top to bottom), marking the lines
/// indented past their block's edge in `indented`.
fn blocks(lines: &[Line], group: &[usize], indented: &mut [bool]) -> Vec<Block> {
    let mut done: Vec<Block> = vec![];
    let mut open: Vec<Block> = vec![];
    for &i in group {
        let Some(l) = lines.get(i) else { continue };
        // Blocks too far above can't go on.
        let (gone, kept): (Vec<Block>, Vec<Block>) = open.into_iter().partition(|b| l.baseline - b.last > LEADING.1 * l.size);
        done.extend(gone);
        open = kept;
        // The block this line goes on: its own left edge before an indent, the evenest spacing first.
        let fit = |b: &Block| {
            let gap = l.baseline - b.last;
            let spaced = gap >= LEADING.0 * l.size && gap <= LEADING.1 * l.size && b.leading.is_none_or(|ld| (gap - ld).abs() <= EVEN * ld);
            let dx = l.left - b.left;
            let edge = if dx.abs() <= EDGE * l.size {
                Some(0)
            } else if dx > 0.0 && dx <= INDENT * l.size {
                Some(1)
            } else if b.lines.len() == 1 && dx < 0.0 && -dx <= INDENT * l.size {
                // The block's one line was indented: this line has the edge.
                Some(2)
            } else {
                None
            };
            edge.filter(|_| spaced).map(|e| (e, (gap - b.leading.unwrap_or(gap)).abs()))
        };
        let best =
            open.iter().enumerate().filter_map(|(k, b)| fit(b).map(|f| (k, f))).min_by(|(_, a), (_, b)| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        match best.and_then(|(k, (edge, _))| Some((open.get_mut(k)?, edge))) {
            Some((b, edge)) => {
                b.leading = b.leading.or(Some(l.baseline - b.last));
                b.last = l.baseline;
                b.lines.push(i);
                match edge {
                    1 => {
                        if let Some(flag) = indented.get_mut(i) {
                            *flag = true;
                        }
                    }
                    2 => b.left = l.left,
                    _ => {}
                }
            }
            None => open.push(Block { lines: vec![i], left: l.left, top: l.baseline, last: l.baseline, leading: None }),
        }
    }
    done.extend(open);
    done
}

/// The longest of `ids`' lines.
fn longest(lines: &[Line], ids: &[usize]) -> f64 {
    ids.iter().filter_map(|&i| lines.get(i)).map(|l| l.width).fold(0.0, f64::max)
}

/// `blocks` (of one artboard, family and size) as chains of columns: each block followed by the
/// block whose first baseline lines up with its own, nearest to its right, spaced alike, and
/// separated from it by a gutter no wider than it (a table's column further off isn't its next).
fn columns(lines: &[Line], blocks: &[Block]) -> Vec<Vec<usize>> {
    let mut next: Vec<Option<usize>> = vec![None; blocks.len()];
    let mut follows = vec![false; blocks.len()];
    let mut by_left: Vec<usize> = (0..blocks.len()).collect();
    by_left.sort_by(|&a, &b| match (blocks.get(a), blocks.get(b)) {
        (Some(x), Some(y)) => x.left.total_cmp(&y.left),
        _ => a.cmp(&b),
    });
    for &a in &by_left {
        let Some(ba) = blocks.get(a) else { continue };
        let (Some(lead), Some(size)) = (ba.leading, ba.lines.first().and_then(|&i| lines.get(i)).map(|l| l.size)) else { continue };
        let width = longest(lines, &ba.lines);
        let candidate = by_left
            .iter()
            .copied()
            .filter(|&b| !follows.get(b).copied().unwrap_or(true))
            .filter_map(|b| blocks.get(b).map(|bb| (b, bb)))
            .filter(|(_, bb)| {
                (bb.top - ba.top).abs() <= 0.5 * size
                    && bb.left > ba.left + 0.5 * width
                    && bb.left <= ba.left + 2.0 * width
                    && bb.leading.is_some_and(|ld| (ld - lead).abs() <= EVEN * lead)
            })
            .min_by(|(_, x), (_, y)| x.left.total_cmp(&y.left));
        if let Some((b, _)) = candidate {
            if let Some(n) = next.get_mut(a) {
                *n = Some(b);
            }
            if let Some(f) = follows.get_mut(b) {
                *f = true;
            }
        }
    }
    let mut chains = vec![];
    for (b, &after) in follows.iter().enumerate() {
        if after {
            continue;
        }
        // Each step moves right, so a chain ends.
        let mut chain = vec![b];
        let mut at = b;
        while let Some(&Some(n)) = next.get(at) {
            if chain.len() > blocks.len() {
                break;
            }
            chain.push(n);
            at = n;
        }
        chains.push(chain);
    }
    chains
}

/// Whether `text` ends a sentence: a full stop, question or exclamation mark or an ellipsis,
/// before any closing quotes or brackets.
fn ends_sentence(text: &str) -> bool {
    text.trim_end().trim_end_matches(['"', '\'', '»', '«', '”', '“', '’', ')', ']']).ends_with(['.', '!', '?', '…'])
}

/// The run of lines `seq` (a block's, or its columns') cut into paragraphs, and whether its
/// lines are justified.
fn split(lines: &[Line], seq: &[usize], indented: &[bool]) -> (Vec<Vec<usize>>, bool) {
    let all: Vec<&Line> = seq.iter().filter_map(|&i| lines.get(i)).collect();
    let longest = longest(lines, seq);
    let inner = all.len().saturating_sub(1);
    let full = all.iter().take(inner).filter(|l| l.width >= FULL * longest).count();
    // Fewer lines can't tell justified type from ragged. In justified type a line that isn't full
    // ends its paragraph, so it ends a sentence or a lead-in (a colon): ragged lines that mostly
    // run nearly as long as the longest have one that doesn't.
    let ends_para = |l: &&Line| ends_sentence(&l.text) || l.text.trim_end().ends_with(':');
    let justified = all.len() >= 4 && full * 2 >= inner && all.iter().take(inner).all(|l| l.width >= FULL * longest || ends_para(l));
    let all_strong = all.iter().all(|l| l.strong);
    let mut out = vec![];
    let mut para = vec![];
    for (k, (&i, l)) in seq.iter().zip(&all).enumerate() {
        para.push(i);
        let Some((&ni, n)) = seq.get(k + 1).zip(all.get(k + 1)) else { break };
        let ends = indented.get(ni).copied().unwrap_or(false)
            || (justified && l.width < SHORT_JUSTIFIED * longest)
            || (!justified && l.width < SHORT_RAGGED * longest && ends_sentence(&l.text))
            || (!all_strong && n.strong && !l.strong)
            || l.bold != n.bold;
        if ends {
            out.push(std::mem::take(&mut para));
        }
    }
    if !para.is_empty() {
        out.push(para);
    }
    (out, justified)
}
