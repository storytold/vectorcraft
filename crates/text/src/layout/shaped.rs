//! Every-line composition where a line's width depends on the lines above it: area type in a
//! shape other than a rectangle, or with text wrap, whose characters differ in line metrics (type
//! size, font, leading, vertical scale, an inline graphic taller than the text). A taller line
//! moves the lines below it down, to where the frame may be narrower or a wrap object splits the
//! line. Each line the composer tries is placed from where the lines above it left the pen, at its
//! own tallest metrics, as the line loop in [`super::flow`] places a composed line: the span it
//! gets is the room across its whole height.
//!
//! A paragraph the frame cannot hold is still composed whole: a line the frame has no room for,
//! and every line after it, is measured as wide as the span of the last line set in the frame,
//! less its own indents. The line loop sets only the lines that fit, and the rest overflows or
//! goes on in the next threaded frame, which composes it again at its own measures.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use super::breaking::{KinsokuRules, Metrics, Pen, word_width};
use super::flow::line_indents;
use super::{ParaStyle, SGlyph};
use crate::composer::{Exhausted, Measure};

/// The most lines the composer tries to place for one paragraph before it stops and the paragraph
/// is broken line by line.
const MAX_PROBES: usize = 1_000_000;

/// The most baseline steps placing those lines takes in all (see [`super::breaking::MAX_TOTAL_STEPS`]).
const MAX_STEPS: usize = 2_000_000;

/// The most lines the composer tries to place for all the paragraphs of one frame's layout.
const MAX_LAYOUT_PROBES: usize = 2 * MAX_PROBES;

/// The most baseline steps placing them takes over all the paragraphs of one frame's layout.
const MAX_LAYOUT_STEPS: usize = 2 * MAX_STEPS;

/// What is left of the composer's search over one frame's layout: the lines it may still try to
/// place and the baseline steps placing them may still take. Every paragraph's search takes what
/// it spends from it, and stops when it runs out, or at its own limits of [`MAX_PROBES`] and
/// [`MAX_STEPS`]. It is separate from the layout's own steps ([`Pen::steps_left`]), which the
/// search leaves to the line loop.
pub(super) struct Budget {
    probes: Cell<usize>,
    steps: Cell<usize>,
}

impl Budget {
    /// A budget of `probes` lines to try and `steps` baseline steps.
    pub(super) fn new(probes: usize, steps: usize) -> Self {
        Self { probes: Cell::new(probes), steps: Cell::new(steps) }
    }

    /// The lines left to try and the baseline steps left.
    #[cfg(test)]
    pub(super) fn left(&self) -> (usize, usize) {
        (self.probes.get(), self.steps.get())
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(MAX_LAYOUT_PROBES, MAX_LAYOUT_STEPS)
    }
}

/// Where a line starts: glyph `pos`, with the pen where the lines above it left it.
struct Start<'r> {
    pos: usize,
    /// The paragraph's first line (it takes the first-line indent).
    first: bool,
    pen: Pen<'r>,
    /// The width of the span the line above was set in, `None` for the paragraph's first line.
    span: Option<f64>,
    /// The line above was past the frame's end: this line is too, at the width of `span`.
    past: bool,
    /// The line placed at each of the metrics it was tried with, or `None` when it can't be set.
    placed: Vec<(Metrics, Option<Placed<'r>>)>,
    /// The metrics of glyphs `pos..to`, measured so far.
    to: usize,
    m: Option<Metrics>,
}

/// A line set from a [`Start`]: its width, the pen after it, the width of the span it was set
/// in, and whether it is past the frame's end (measured, but not in the frame).
#[derive(Clone)]
struct Placed<'r> {
    width: f64,
    pen: Pen<'r>,
    span: f64,
    past: bool,
}

/// What a line's placement depends on: its first glyph, whether it is the paragraph's first line,
/// the pen's cell, baselines, pending space, narrowest word without room and further spans, the
/// span of the line above and whether the line is past the frame's end.
type Key = (usize, bool, usize, Option<u64>, Option<u64>, u64, u64, Vec<[u64; 3]>, Option<u64>, bool);

fn key(pos: usize, first: bool, pen: &Pen<'_>, span: Option<f64>, past: bool) -> Key {
    (
        pos,
        first,
        pen.ri,
        pen.prev.map(f64::to_bits),
        pen.next_top.map(f64::to_bits),
        pen.pending.to_bits(),
        pen.no_room_for.to_bits(),
        pen.queued.iter().map(|&(b, x0, x1)| [b.to_bits(), x0.to_bits(), x1.to_bits()]).collect(),
        span.map(f64::to_bits),
        past,
    )
}

/// What placing a line reads from the paragraph.
struct Para<'a> {
    sg: &'a [SGlyph],
    /// How much narrower a line ending with each glyph is set (`ParagraphData::line_end`).
    line_end: &'a [f64],
    para: &'a ParaStyle,
    /// The width a line starting with each glyph gives back (Mojikumi's flush opening bracket).
    start_credit: &'a dyn Fn(usize) -> f64,
    mirror: bool,
    /// The baseline steps left to the search.
    steps: Rc<Cell<usize>>,
}

/// The measures of a paragraph's lines in a frame where they depend on the lines above.
pub(super) struct Shaped<'a, 'r> {
    para: Para<'a>,
    continued: bool,
    pen: Pen<'r>,
    starts: Vec<Start<'r>>,
    known: HashMap<Key, usize>,
    /// The lines tried so far, and the most this search may try.
    probes: usize,
    max_probes: usize,
    /// The baseline steps this search started with.
    max_steps: usize,
    /// The layout's budget, which takes what the search spent when it ends.
    budget: Rc<Budget>,
    /// The tallest metrics of glyphs `k..` for each `k`, `None` past the last glyph.
    rest: Vec<Option<Metrics>>,
}

impl<'a, 'r> Shaped<'a, 'r> {
    /// The measures of the lines of glyphs `sg` of paragraph `para`, set from `pen`. `line_end`
    /// and `start_credit` are how much narrower a line ending with each glyph is set and how much
    /// a line starting with it gives back; `continued`: the paragraph's first line is in the
    /// frame before; `mirror`: its indents are mirrored. The search places its lines with a copy
    /// of `pen` and takes its baseline steps from the layout's search budget ([`Pen::search`]),
    /// so the frame's layout keeps its own steps.
    pub(super) fn new(
        sg: &'a [SGlyph],
        line_end: &'a [f64],
        para: &'a ParaStyle,
        start_credit: &'a dyn Fn(usize) -> f64,
        continued: bool,
        mirror: bool,
        pen: &Pen<'r>,
    ) -> Self {
        let mut pen = pen.clone();
        let budget = Rc::clone(&pen.search);
        let max_steps = budget.steps.get().min(pen.steps_left.get()).min(MAX_STEPS);
        let steps = Rc::new(Cell::new(max_steps));
        pen.steps_left = Rc::clone(&steps);
        let mut rest = vec![None; sg.len() + 1];
        for (k, g) in sg.iter().enumerate().rev() {
            let m = Metrics::of(g);
            rest[k] = Some(rest[k + 1].map_or(m, |r: Metrics| r.union(m)));
        }
        Self {
            para: Para { sg, line_end, para, start_credit, mirror, steps },
            continued,
            pen,
            starts: vec![],
            known: HashMap::new(),
            probes: 0,
            max_probes: budget.probes.get().min(MAX_PROBES),
            max_steps,
            budget,
            rest,
        }
    }

    /// The start of a line at glyph `pos` with `pen`, after a line set in a span `span` wide (past
    /// the frame's end with `past`): an existing one when its placement is the same.
    fn start(&mut self, pos: usize, first: bool, pen: Pen<'r>, span: Option<f64>, past: bool) -> usize {
        let k = key(pos, first, &pen, span, past);
        if let Some(&id) = self.known.get(&k) {
            return id;
        }
        self.starts.push(Start { pos, first, pen, span, past, placed: vec![], to: pos, m: None });
        let id = self.starts.len() - 1;
        self.known.insert(k, id);
        id
    }
}

impl Measure for Shaped<'_, '_> {
    type State = usize;

    fn first(&mut self) -> usize {
        let pen = self.pen.clone();
        self.start(0, !self.continued, pen, None, false)
    }

    fn line(&mut self, &id: &usize, pos: usize, end: usize) -> Result<Option<(f64, usize)>, Exhausted> {
        if self.probes >= self.max_probes {
            return Err(Exhausted);
        }
        self.probes += 1;
        let Some(start) = self.starts.get_mut(id) else { return Ok(None) };
        let Some(placed) = line(&self.para, start, pos, end)? else { return Ok(None) };
        Ok(Some((placed.width, self.start(end, false, placed.pen, Some(placed.span), placed.past))))
    }

    /// A longer line is placed as this one is, and gets the same room, unless a glyph after it
    /// changes the line's tallest metrics. Past the frame's end every line is as wide.
    fn may_widen(&mut self, &id: &usize, pos: usize, end: usize) -> bool {
        let Some(start) = self.starts.get(id).filter(|start| !start.past) else { return false };
        let m = if start.pos == pos && start.to == end { start.m } else { self.para.sg.get(pos..end).and_then(Metrics::max) };
        match (m, self.rest.get(end).copied().flatten()) {
            (Some(m), Some(rest)) => m.union(rest) != m,
            _ => false,
        }
    }
}

/// The search's lines tried and baseline steps spent come off the layout's budget, so the
/// paragraphs of one frame share it.
impl Drop for Shaped<'_, '_> {
    fn drop(&mut self) {
        let spent = self.max_steps.saturating_sub(self.para.steps.get());
        self.budget.steps.set(self.budget.steps.get().saturating_sub(spent));
        self.budget.probes.set(self.budget.probes.get().saturating_sub(self.probes));
    }
}

/// The line of glyphs `pos..end` from `start`; `None` when it can't be set.
fn line<'r>(p: &Para<'_>, start: &mut Start<'r>, pos: usize, end: usize) -> Result<Option<Placed<'r>>, Exhausted> {
    if start.pos != pos || end <= pos {
        return Ok(None);
    }
    if start.past {
        return Ok(past_end(p, start));
    }
    // The line's tallest metrics, measured on from the glyphs already measured.
    if end < start.to {
        start.m = p.sg.get(pos..end).and_then(Metrics::max);
        start.to = end;
    } else if end > start.to {
        let more = p.sg.get(start.to..end).and_then(Metrics::max);
        start.m = match (start.m, more) {
            (Some(m), Some(more)) => Some(m.union(more)),
            (m, more) => m.or(more),
        };
        start.to = end;
    }
    let Some(m) = start.m else { return Ok(None) };
    if let Some((_, placed)) = start.placed.iter().find(|(at, _)| *at == m) {
        return Ok(placed.clone());
    }
    let placed = place(p, start, m)?;
    start.placed.push((m, placed.clone()));
    Ok(placed)
}

/// A line from `start` past the frame's end: as wide as the span of the last line set in the frame,
/// less its indents, and the pen left where the frame ran out. `None` when the paragraph has no line
/// in the frame to measure it by.
fn past_end<'r>(p: &Para<'_>, start: &Start<'r>) -> Option<Placed<'r>> {
    let span = start.span.filter(|span| span.is_finite())?;
    let (ind_l, ind_r) = line_indents(p.para, start.first, p.mirror);
    Some(Placed { width: span - ind_l - ind_r, pen: start.pen.clone(), span, past: true })
}

/// The line from `start` placed with metrics `m`, as the line loop places and settles it; past the
/// frame's end ([`past_end`]) when no frame cell has room for it.
fn place<'r>(p: &Para<'_>, start: &Start<'r>, m: Metrics) -> Result<Option<Placed<'r>>, Exhausted> {
    let (ind_l, ind_r) = line_indents(p.para, start.first, p.mirror);
    let word = word_width(p.sg, start.pos, p.line_end, KinsokuRules::of(p.para)) - (p.start_credit)(start.pos);
    let mut pen = start.pen.clone();
    loop {
        match pen.place(m, ind_l, ind_r, word) {
            Some((baseline, x0, x1, _)) if baseline.is_finite() && (x1 - x0).is_finite() => {
                if baseline + m.desc > pen.bottom() + 0.01 {
                    // A further span of the line above, too low for this line: the next row or column.
                    pen.ri += 1;
                    pen.prev = None;
                    pen.pending = 0.0;
                    pen.queued.clear();
                    continue;
                }
                pen.pending = 0.0;
                pen.settled(baseline, m);
                // Further spans of this line band share the settled baseline.
                for queued in &mut pen.queued {
                    queued.0 = baseline;
                }
                return Ok(Some(Placed { width: x1 - x0 - ind_l - ind_r, pen, span: x1 - x0, past: false }));
            }
            Some(_) => return Ok(None),
            // Out of steps: the frame may still have room further down.
            None if p.steps.get() == 0 => return Err(Exhausted),
            None => return Ok(past_end(p, start)),
        }
    }
}
