use super::region::Region;
use super::{
    Affine, BezPath, Breakpoint, Burasagari, EPS, FirstBaseline, FontDb, HyphenationOptions, Kinsoku, LeadingModel, SGlyph, hyphen_glyph,
    hyphen_points_with, no_line_end, no_line_start, soft_line_start,
};

/// Vertical metrics of a line: (ascent, descent, leading, cap height, x height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    pub(super) asc: f64,
    pub(super) desc: f64,
    pub(super) lead: f64,
    pub(super) cap: f64,
    pub(super) xh: f64,
    /// Height of the top of the ideographic em box above the baseline.
    pub(super) top: f64,
}

impl Metrics {
    pub(super) fn of(g: &SGlyph) -> Self {
        let em = g.face.units_per_em() * g.sy;
        Self { asc: g.ascent, desc: g.descent, lead: g.leading, cap: g.cap, xh: g.xh, top: (g.face.ideographic_centre() + 0.5) * em }
    }
    pub(super) fn max(g: &[SGlyph]) -> Option<Self> {
        let mut it = g.iter();
        let first = Self::of(it.next()?);
        Some(it.fold(first, |m, g| Self {
            asc: m.asc.max(g.ascent),
            desc: m.desc.max(g.descent),
            lead: m.lead.max(g.leading),
            cap: m.cap.max(g.cap),
            xh: m.xh.max(g.xh),
            top: m.top.max(Self::of(g).top),
        }))
    }
    /// Distance from the frame top to the first baseline.
    pub(super) fn first_baseline(&self, fb: FirstBaseline, min: f64) -> f64 {
        let v = match fb {
            FirstBaseline::Ascent => self.asc,
            FirstBaseline::CapHeight => self.cap,
            FirstBaseline::XHeight => self.xh,
            FirstBaseline::Leading => self.lead,
            FirstBaseline::Fixed => 0.0,
        };
        v.max(min)
    }
}

/// Where the next line goes: the region being filled and the previous baseline in it.
#[derive(Clone)]
pub(super) struct Pen<'r> {
    pub(super) regions: Option<&'r [Region]>,
    pub(super) ri: usize,
    pub(super) prev: Option<f64>,
    pub(super) pending: f64,
    pub(super) fb: FirstBaseline,
    pub(super) fb_min: f64,
    /// Further spans at the current baseline (text wrapping on both sides of an object).
    pub(super) queued: Vec<(f64, f64, f64)>,
    /// Leading measured from em box top to em box top: where the next line's em box top goes.
    pub(super) model: LeadingModel,
    pub(super) next_top: Option<f64>,
    /// The narrowest word no line from here down is wide enough for (see [`Pen::place`]).
    pub(super) no_room_for: f64,
}

/// The largest number of steps layout takes to move a line down a frame cell to find room for it:
/// in a cell taller than this many leads, a step is the cell's height divided by this.
const MAX_STEPS: usize = 100_000;

impl Pen<'_> {
    /// Baseline and horizontal span for a line with estimated metrics `est` and indents that starts
    /// with a word `word` wide; `None` = the frame is full (overflow).
    /// In a frame that isn't a rectangle, lines too short for the word are left empty and it moves
    /// down to the first one wide enough. Only when no line is does it go on the first line with
    /// room for any text, and get broken there.
    /// The `bool` is true for a further span at the previous line's baseline.
    pub(super) fn place(&mut self, est: Metrics, ind_l: f64, ind_r: f64, word: f64) -> Option<(f64, f64, f64, bool)> {
        if word > 0.0 && word < self.no_room_for {
            let mut pen = self.clone();
            if let Some(placed) = pen.place_at_least(est, ind_l, ind_r, word) {
                *self = pen;
                return Some(placed);
            }
            // Lines further down are no wider for this word: don't search for it, or a wider one, again.
            self.no_room_for = word;
        }
        self.place_at_least(est, ind_l, ind_r, 0.0)
    }

    /// [`Pen::place`], skipping lines of a non-rectangular frame narrower than `min`.
    fn place_at_least(&mut self, est: Metrics, ind_l: f64, ind_r: f64, min: f64) -> Option<(f64, f64, f64, bool)> {
        let Some(regions) = self.regions else {
            let b = self.prev.map_or(0.0, |b| self.next_baseline(b, est));
            return Some((b, f64::NEG_INFINITY, f64::INFINITY, false));
        };
        if !self.queued.is_empty() {
            let (b, x0, x1) = self.queued.remove(0);
            return Some((b, x0, x1, true));
        }
        loop {
            let r = regions.get(self.ri)?;
            let mut baseline = match self.prev {
                None if self.model == LeadingModel::EmBoxTop => r.top() + r.shift + est.top.max(self.fb_min),
                None => r.top() + r.shift + est.first_baseline(self.fb, self.fb_min),
                Some(b) => self.next_baseline(b, est) + r.gap,
            };
            // The baseline moves down the cell a lead at a time, or a `MAX_STEPS`th of the cell's
            // height when that is more, in at most `MAX_STEPS` steps.
            let step = est.lead.max(1.0).max(r.cell.height() / MAX_STEPS as f64);
            for _ in 0..=MAX_STEPS {
                if baseline + est.desc > r.bottom() + 0.01 {
                    break;
                }
                // A rectangle's lines are all as wide: none below has more room for the word
                // (until wrap shapes can narrow the first baseline: they stay bounded).
                let constant = (r.rect || r.polys.is_empty()) && r.wraps.is_empty();
                let min = if constant { 0.0 } else { min };
                let fits = |&(a, b): &(f64, f64)| {
                    let room = b - a - ind_l - ind_r;
                    room > est.asc.max(1.0) && room + EPS >= min
                };
                if r.wraps.is_empty() {
                    match r.span(baseline - est.asc, baseline + est.desc) {
                        Some(s) if fits(&s) => return Some((baseline, s.0, s.1, false)),
                        // Too narrow here is too narrow all the way down the cell: next cell.
                        Some(_) if constant => break,
                        _ => baseline += step,
                    }
                } else {
                    // Wrap objects can split a line: fill every span, left to right.
                    let mut spans: Vec<(f64, f64)> = r.spans(baseline - est.asc, baseline + est.desc).into_iter().filter(fits).collect();
                    if spans.is_empty() {
                        baseline += step;
                        continue;
                    }
                    let (a, b) = spans.remove(0);
                    self.queued = spans.into_iter().map(|(a, b)| (baseline, a, b)).collect();
                    return Some((baseline, a, b, false));
                }
            }
            // Next row/column.
            self.ri += 1;
            self.prev = None;
            self.pending = 0.0;
            self.queued.clear();
        }
    }
    pub(super) fn bottom(&self) -> f64 {
        self.regions.and_then(|r| r.get(self.ri)).map_or(f64::INFINITY, |r| r.bottom())
    }
    /// The baseline of the line after the one at `prev`, for a line of metrics `est`.
    fn next_baseline(&self, prev: f64, est: Metrics) -> f64 {
        match (self.model, self.next_top) {
            (LeadingModel::EmBoxTop, Some(top)) => top + est.top + self.pending,
            _ => prev + est.lead + self.pending,
        }
    }
    /// A line was set at `baseline` with metrics `m`: where the next one goes from.
    pub(super) fn settled(&mut self, baseline: f64, m: Metrics) {
        self.prev = Some(baseline);
        self.next_top = Some(baseline - m.top + m.lead);
    }
}

/// Advance of a tab at pen position `x`: up to the next stop after it (explicit stops first, then
/// every ½ inch), and the leader of that stop (default stops have none). Right, centre and decimal
/// stops align the text that follows (`rest`, up to the next tab) on the stop.
pub(super) fn tab_advance<'t>(tabs: &'t [vectorcraft_doc::TabStop], origin: f64, x: f64, rest: &[SGlyph]) -> (f64, &'t str) {
    use vectorcraft_doc::TabAlign;
    let rel = x - origin;
    let seg: Vec<&SGlyph> = rest.iter().take_while(|g| g.ch != '\t' && g.ch != '\u{2028}').collect();
    let width: f64 = seg.iter().map(|g| g.adv).sum();
    let target = |pos: f64, align: TabAlign, on: char| match align {
        TabAlign::Left => pos,
        TabAlign::Right => pos - width,
        TabAlign::Center => pos - width / 2.0,
        TabAlign::Decimal => pos - seg.iter().take_while(|g| g.ch != on).map(|g| g.adv).sum::<f64>(),
    };
    let explicit = tabs.iter().find_map(|t| {
        let at = target(t.position, t.align, t.align_on);
        (at > rel + 1e-6).then_some((at, t.leader.as_str()))
    });
    let (at, leader) = explicit.unwrap_or_else(|| {
        let last = tabs.iter().map(|t| t.position).fold(0.0, f64::max).max(0.0);
        let base = rel.max(last);
        (((base / vectorcraft_doc::text::DEFAULT_TAB_INTERVAL).floor() + 1.0) * vectorcraft_doc::text::DEFAULT_TAB_INTERVAL, "")
    });
    ((at - rel).max(0.0), leader)
}

/// The width of glyphs `from..to` of `sg` set on a line whose content starts `x_start` from the
/// tab origin: tabs of the text take what [`tab_advance`] gives them, not the width of their
/// glyph. The tab after a list marker (length 0) keeps its own width.
pub(super) fn set_width(sg: &[SGlyph], from: usize, to: usize, tabs: &[vectorcraft_doc::TabStop], x_start: f64) -> f64 {
    let mut x = x_start;
    for (j, g) in sg.iter().enumerate().take(to).skip(from) {
        x += if g.ch == '\t' && g.len > 0 { tab_advance(tabs, 0.0, x, sg.get(j + 1..to).unwrap_or_default()).0 } else { g.adv };
    }
    x - x_start
}

/// Most copies of a leader drawn in one tab: a tiny type size and a far stop must not make
/// millions of outlines.
const MAX_LEADER_COPIES: f64 = 4096.0;

/// The outline of a tab leader: `leader`'s characters, in the font and size of tab `g`, repeated
/// across the tab's gap `from..to` on baseline `y`. Copies sit on a grid of the leader's width
/// from `origin` (where tab stops are measured from), so the leaders of lines above one another
/// line up; only whole copies are drawn. Characters the font lacks draw nothing.
pub(super) fn leader_outline(db: &FontDb, g: &SGlyph, leader: &str, origin: f64, from: f64, to: f64, y: f64) -> BezPath {
    let mut out = BezPath::new();
    let glyphs: Vec<(u32, f64)> = leader
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            let gid = g.face.glyph_for(c);
            (gid, g.face.advance(gid) * g.sx)
        })
        .collect();
    let width: f64 = glyphs.iter().map(|(_, a)| a).sum();
    if !(width > EPS && width.is_finite() && from.is_finite() && to.is_finite()) {
        return out;
    }
    let first = ((from - origin) / width - EPS).ceil();
    let copies = (((to - origin) / width + EPS).floor() - first).clamp(0.0, MAX_LEADER_COPIES) as usize;
    let local = Affine::rotate(-g.rotation.to_radians()) * Affine::translate((0.0, -g.bshift)) * Affine::scale_non_uniform(g.sx, g.sy);
    for k in 0..copies {
        let mut pen = origin + (first + k as f64) * width;
        for &(gid, adv) in &glyphs {
            if gid != 0 {
                let m = Affine::translate((pen, y)) * local;
                out.extend(db.outline(&g.face, gid).elements().iter().map(|el| m * *el));
            }
            pen += adv;
        }
    }
    out
}

/// Width of the word starting at glyph `i`: its advances up to the first break opportunity (spaces
/// left out, the hyphen of a soft hyphen in), the least room a line needs not to break inside it.
/// A list marker at `i` (glyphs of length 0) counts whole, with the word after it.
pub(super) fn word_width(g: &[SGlyph], i: usize) -> f64 {
    /// Glyphs measured at most: a word wrapped over many lines is measured again from each line's
    /// start, and one this long is broken inside whatever cell it meets.
    const MAX_GLYPHS: usize = 1024;
    let rest = g.get(i..).unwrap_or_default();
    let marker = rest.iter().take_while(|gl| gl.len == 0).count();
    let mut w: f64 = rest.iter().take(marker).map(|gl| gl.adv).sum();
    for gl in rest.iter().skip(marker).take(MAX_GLYPHS) {
        if gl.is_space() {
            break;
        }
        if gl.is_soft_hyphen() {
            return w + hyphen_glyph(gl).adv;
        }
        w += gl.adv;
        if gl.break_after() {
            break;
        }
    }
    w
}

/// Greedy break: returns (end glyph index, hyphenated) for a line starting at `i` of `width`.
/// With burasagari, a comma or full stop that doesn't fit ends the line, hanging outside it.
/// `hyphenate` holds the paragraph's Hyphenation options and the Hyphenation Zone in force when
/// the line may hyphenate a word; a line whose text before the overflowing word already ends
/// within the zone breaks before that word. A tab is as wide as it is set: up to its next stop
/// (`tabs`), with the line's content starting `x_start` from the tab origin.
#[allow(clippy::too_many_arguments)]
pub(super) fn break_line(
    text: &str,
    g: &[SGlyph],
    i: usize,
    width: f64,
    tabs: &[vectorcraft_doc::TabStop],
    x_start: f64,
    hyphenate: Option<(&HyphenationOptions, f64)>,
    burasagari: Burasagari,
    kinsoku: Kinsoku,
) -> (usize, bool) {
    if !width.is_finite() {
        if let Some(pos) = g[i..].iter().position(|gl| gl.ch == '\u{2028}') {
            return (i + pos + 1, false);
        }
        return (g.len(), false);
    }
    let mut x = 0.0;
    let mut last_break: Option<(usize, bool)> = None;
    // A list marker (length 0) is a cluster of its own; the line holds it and at least the first
    // cluster after it.
    let first = i + g.get(i..).unwrap_or_default().iter().take_while(|h| h.len == 0).count();
    let mut j = i;
    while j < g.len() {
        let gl = &g[j];
        let mut cluster_end = j + 1;
        while g.get(cluster_end).is_some_and(|next| next.byte == gl.byte && (next.len == 0) == (gl.len == 0)) {
            cluster_end += 1;
        }
        // No Break: the marked characters stay on one line, including across a space inside the run.
        if gl.no_break && gl.ch != '\u{2028}' {
            while let Some(n) = g.get(cluster_end) {
                if !n.no_break || n.ch == '\u{2028}' {
                    break;
                }
                let b = n.byte;
                cluster_end += 1;
                while g.get(cluster_end).is_some_and(|next| next.byte == b) {
                    cluster_end += 1;
                }
            }
        }
        let cluster_width: f64 = if gl.ch == '\t' {
            tab_advance(tabs, 0.0, x_start + x, g.get(cluster_end..).unwrap_or_default()).0
        } else {
            g[j..cluster_end].iter().map(|glyph| glyph.adv).sum()
        };
        if gl.ch == '\u{2028}' {
            return (cluster_end, false);
        }
        if j > first && !gl.is_space() && x + cluster_width > width + EPS {
            // It ends the line with the spaces after it.
            if burasagari != Burasagari::None && hangs(gl) && kinsoku_allows(g, cluster_end - 1, kinsoku) {
                let mut end = cluster_end;
                while g.get(end).is_some_and(SGlyph::is_space) {
                    end += 1;
                }
                return (end, false);
            }
            break;
        }
        x += cluster_width;
        let last = &g[cluster_end - 1];
        if last.break_after() && last.len > 0 && kinsoku_allows(g, cluster_end - 1, kinsoku) && !splits_no_break(g, cluster_end) {
            if last.is_soft_hyphen() {
                if x + hyphen_glyph(last).adv <= width + EPS {
                    last_break = Some((cluster_end, true));
                }
            } else {
                last_break = Some((cluster_end, false));
            }
        }
        j = cluster_end;
    }
    if j >= g.len() {
        return (g.len(), false);
    }
    // Hyphenate the word that overflows.
    let word_start = last_break.map_or(i, |b| b.0);
    let in_zone = |zone: f64| {
        let mut te = word_start;
        while te > i && g[te - 1].is_space() {
            te -= 1;
        }
        te > i && zone > 0.0 && width - g[i..te].iter().map(|g| g.adv).sum::<f64>() <= zone + EPS
    };
    if let Some((opts, zone)) = hyphenate
        && g[j].is_letter()
        && !in_zone(zone)
    {
        let mut we = j;
        while we < g.len() && !g[we].is_space() && !g[we].break_after() {
            we += 1;
        }
        let x_ws: f64 = g[i..word_start].iter().map(|g| g.adv).sum();
        let pts: Vec<usize> = hyphen_breaks(text, g, word_start, we, opts)
            .into_iter()
            .filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku) && !splits_no_break(g, k))
            .collect();
        for &k in pts.iter().rev() {
            if k <= j && k > i {
                let w = x_ws + g[word_start..k].iter().map(|g| g.adv).sum::<f64>() + hyphen_glyph(&g[k - 1]).adv;
                if w <= width + EPS {
                    return (k, true);
                }
            }
        }
    }
    // Break candidates and greedy overflow are cluster boundaries; hyphenation also returns only
    // cluster starts, so the resulting end always keeps each source cluster together.
    let (end, hy) = match last_break {
        Some(b) if b.0 > i => b,
        _ => (j, false),
    };
    (end, hy)
}

/// Can glyph `g` hang outside the line (burasagari)? An East Asian comma or full stop, full width
/// (、。，．) or half width (､｡); not a closing bracket, nor Latin punctuation (Latin text keeps
/// its line breaks and composer). Never an inline graphic (U+FFFC).
pub(super) fn hangs(g: &SGlyph) -> bool {
    g.inline.is_none() && matches!(g.ch, '、' | '。' | '，' | '．' | '､' | '｡') && g.tcy.is_none()
}

/// Optical Margin Alignment: the share of glyph `g`'s advance that hangs outside the margin when
/// `g` starts or ends a line. Quotation marks, hyphens, full stops and commas hang whole, and
/// asterisks, tildes, ellipses, en and em dashes, colons and semicolons hang by half (the amounts
/// published for Roman Hanging Punctuation). A, V, W, Y, v, w and y hang by a tenth of their
/// advance, VectorCraft's own amount for their slanted sides. Other characters, East Asian
/// punctuation (which burasagari hangs) and inline graphics don't hang.
pub(super) fn optical_share(g: &SGlyph) -> f64 {
    if g.inline.is_some() || g.tcy.is_some() {
        return 0.0;
    }
    match g.ch {
        '-' | '\u{2010}' | '\u{2011}' | '.' | ',' => 1.0,
        c if is_quote(c) => 1.0,
        '*' | '~' | '…' | '–' | '—' | ':' | ';' => 0.5,
        'A' | 'V' | 'W' | 'Y' | 'v' | 'w' | 'y' => 0.1,
        _ => 0.0,
    }
}

/// Roman Hanging Punctuation: the share of glyph `g`'s advance that hangs outside the margin when
/// `g` starts or ends a line: [`optical_share`]'s punctuation, and no letters.
pub(super) fn roman_share(g: &SGlyph) -> f64 {
    if g.ch.is_alphabetic() { 0.0 } else { optical_share(g) }
}

/// A punctuation mark that hangs ([`optical_share`]), not a letter.
pub(super) fn optical_punct(g: &SGlyph) -> bool {
    !g.ch.is_alphabetic() && optical_share(g) > 0.0
}

/// A single or double quotation mark: straight, curly, low-9 or reversed-9.
fn is_quote(c: char) -> bool {
    matches!(c, '\'' | '"' | '‘' | '’' | '‚' | '‛' | '“' | '”' | '„' | '‟')
}

/// Optical Margin Alignment and Roman Hanging Punctuation: how far a line hangs outside its left
/// and right margins, from the glyph at each end of `line` (its glyphs in visual order, each with
/// the advance it is set with) and its `share` ([`optical_share`] or [`roman_share`]), and how many glyphs hang at each end → (left, right, left glyphs,
/// right glyphs). A punctuation mark followed by a quotation mark hangs together with it: `."`
/// ending a line and `'"` starting one hang both their glyphs. A line of one glyph hangs at
/// neither end.
pub(super) fn optical_hangs(line: &[(&SGlyph, f64)], share: fn(&SGlyph) -> f64) -> (f64, f64, usize, usize) {
    let n = line.len();
    if n < 2 {
        return (0.0, 0.0, 0, 0);
    }
    let hang = |k: usize| line.get(k).map_or(0.0, |&(g, adv)| share(g) * adv.max(0.0));
    let punct = |k: usize| line.get(k).is_some_and(|&(g, _)| optical_punct(g));
    let quote = |k: usize| line.get(k).is_some_and(|&(g, _)| is_quote(g.ch) && share(g) > 0.0);
    let mut left = hang(0);
    // The glyphs that hang at the left; the right end takes none of them.
    let mut taken = usize::from(left > 0.0);
    if left > 0.0 && n > 2 && punct(0) && quote(1) {
        left += hang(1);
        taken = 2;
    }
    let mut right = hang(n - 1);
    let mut right_glyphs = usize::from(right > 0.0);
    if right > 0.0 && n > 2 && n - 2 >= taken && punct(n - 2) && quote(n - 1) {
        right += hang(n - 2);
        right_glyphs = 2;
    }
    (left, right, taken, right_glyphs)
}

/// Works as [`optical_hangs`] does for a right-to-left line, whose `line` is also in visual order.
/// The line is read from right to left: `."` ending it hangs both its glyphs at the left, `'"` starting it hangs both at
/// the right, and the right end takes its glyphs first.
pub(super) fn optical_hangs_rtl(line: &[(&SGlyph, f64)], share: fn(&SGlyph) -> f64) -> (f64, f64, usize, usize) {
    let reversed: Vec<(&SGlyph, f64)> = line.iter().rev().copied().collect();
    let (right, left, right_glyphs, left_glyphs) = optical_hangs(&reversed, share);
    (left, right, left_glyphs, right_glyphs)
}

/// Would a line ending at glyph index `end` split a No Break run?
fn splits_no_break(g: &[SGlyph], end: usize) -> bool {
    end > 0 && g.get(end - 1).is_some_and(|a| a.no_break) && g.get(end).is_some_and(|b| b.no_break)
}

/// Does kinsoku allow a line break after glyph `j`? Not after an opening bracket, nor before a
/// closing one, a comma, a full stop, a small kana… (the character is pushed to the next line
/// with the one before it).
fn kinsoku_allows(g: &[SGlyph], j: usize, kinsoku: Kinsoku) -> bool {
    g.get(j).is_some_and(|gl| kinsoku_between(gl.ch, g.get(j + 1).map(|n| n.ch), kinsoku))
}

/// Kinsoku for a break between `before` and `after` (none: the end of the paragraph): Hard keeps
/// every character of the set off the line start and end, Soft lets 々, ー and small kana start a
/// line, None allows the break.
fn kinsoku_between(before: char, after: Option<char>, kinsoku: Kinsoku) -> bool {
    match kinsoku {
        Kinsoku::None => true,
        Kinsoku::Hard => !no_line_end(before) && after.is_none_or(|c| !no_line_start(c)),
        Kinsoku::Soft => !no_line_end(before) && after.is_none_or(|c| !no_line_start(c) || soft_line_start(c)),
    }
}

#[cfg(test)]
#[test]
fn kinsoku_keeps_closing_marks_off_line_starts_and_opening_ones_off_line_ends() {
    for (a, b) in [('字', '。'), ('弧', '」'), ('」', '、'), ('ャ', 'ー'), ('カ', 'ッ'), ('「', 'か'), ('（', '雅')] {
        assert!(!kinsoku_between(a, Some(b), Kinsoku::Hard), "{a}{b}");
    }
    for (a, b) in [('。', '雅'), ('」', 'は'), ('字', '「'), ('の', '演')] {
        assert!(kinsoku_between(a, Some(b), Kinsoku::Hard), "{a}{b}");
    }
    assert!(kinsoku_between('。', None, Kinsoku::Hard));
}

/// Soft kinsoku lets 々, ー and small kana (JLREQ cl-09's 々, cl-10, cl-11) start a line and keeps
/// the rest of the set (half-width forms aren't in those classes); None allows any break.
#[cfg(test)]
#[test]
fn soft_kinsoku_lets_small_kana_and_the_prolonged_sound_mark_start_a_line() {
    for (a, b) in [('ャ', 'ー'), ('カ', 'ッ'), ('時', '々'), ('ト', 'ㇰ'), ('か', 'ゃ')] {
        assert!(!kinsoku_between(a, Some(b), Kinsoku::Hard), "{a}{b}");
        assert!(kinsoku_between(a, Some(b), Kinsoku::Soft), "{a}{b}");
    }
    for (a, b) in [('字', '。'), ('弧', '」'), ('「', 'か'), ('ｶ', 'ｯ'), ('ｶ', 'ｰ'), ('字', 'ゝ')] {
        assert!(!kinsoku_between(a, Some(b), Kinsoku::Soft), "{a}{b}");
        assert!(kinsoku_between(a, Some(b), Kinsoku::None), "{a}{b}");
    }
}

/// Glyph indices inside `g[ws..we]` (a word) where a hyphenated break may go under `opts`.
fn hyphen_breaks(text: &str, g: &[SGlyph], ws: usize, we: usize, opts: &HyphenationOptions) -> Vec<usize> {
    // Strip leading/trailing punctuation (quotes, commas).
    let (mut a, mut b) = (ws, we);
    while a < b && !g[a].is_letter() {
        a += 1;
    }
    while b > a && !g[b - 1].is_letter() {
        b -= 1;
    }
    if b <= a || g[a..b].iter().any(|g| !g.is_letter()) {
        return vec![];
    }
    let (s, e) = (g[a].byte, g[b - 1].byte + g[b - 1].len);
    let Some(word) = text.get(s..e) else { return vec![] };
    let offs: Vec<usize> = word.char_indices().map(|(o, _)| s + o).collect();
    // The first glyph of each cluster start (not inside a ligature), by its byte: one pass, not a
    // search for each point.
    let mut starts = std::collections::HashMap::new();
    for k in a + 1..b {
        if g[k].byte != g[k - 1].byte {
            starts.entry(g[k].byte).or_insert(k);
        }
    }
    hyphen_points_with(word, opts).into_iter().filter_map(|ci| starts.get(offs.get(ci)?).copied()).collect()
}

/// Break candidates for the every-line composer: hyphenation points under `hyphenate` (the
/// paragraph's Hyphenation options) when it is given. No break comes before a space: a line that
/// may end after a dash, a Thai word or a space ends after the spaces that follow it, as in
/// [`break_line`].
pub(super) fn candidates(text: &str, g: &[SGlyph], hyphenate: Option<&HyphenationOptions>, kinsoku: Kinsoku) -> Vec<Breakpoint> {
    let mut v = vec![];
    let mut ws = 0;
    for (j, gl) in g.iter().enumerate() {
        let end_of_word = gl.is_space() || gl.break_after();
        if end_of_word {
            if let Some(opts) = hyphenate
                && j > ws
            {
                for k in
                    hyphen_breaks(text, g, ws, j, opts).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku) && !splits_no_break(g, k))
                {
                    v.push(Breakpoint { end: k, hyphen: hyphen_glyph(&g[k - 1]).adv });
                }
            }
            let hy = if gl.is_soft_hyphen() { hyphen_glyph(gl).adv } else { 0.0 };
            if j + 1 < g.len() && g[j + 1].byte != gl.byte && !g[j + 1].is_space() && kinsoku_allows(g, j, kinsoku) && !splits_no_break(g, j + 1) {
                v.push(Breakpoint { end: j + 1, hyphen: hy });
            }
            ws = j + 1;
        }
    }
    if let Some(opts) = hyphenate
        && g.len() > ws
    {
        for k in
            hyphen_breaks(text, g, ws, g.len(), opts).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku) && !splits_no_break(g, k))
        {
            v.push(Breakpoint { end: k, hyphen: hyphen_glyph(&g[k - 1]).adv });
        }
    }
    v.sort_by_key(|b| b.end);
    v.dedup_by_key(|b| b.end);
    v
}
