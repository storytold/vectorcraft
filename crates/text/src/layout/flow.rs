use super::bidi::{is_rtl, para_bidi_with, visual_order};
use super::breaking::{
    Metrics, Pen, break_line, candidates, hangs, leader_outline, optical_hangs, optical_hangs_rtl, optical_punct, optical_share, roman_share,
    set_width, tab_advance, word_width,
};
use super::region::Region;
use super::vertical::{EM_CENTER, align_shift, compress_punctuation, glyph_em, punct_half, space_japanese_and_latin};
use super::*;

/// Paragraphs longer than this (glyphs) are always broken line by line.
const MAX_COMPOSE_GLYPHS: usize = 200_000;

/// Every-line composition of paragraph glyphs `sg` if applicable (area text, justified or ragged,
/// with uniform line metrics or in rectangular cells without text wrap); `None` falls back to the
/// greedy single-line composer. With `continued`, the paragraph continues from the frame before,
/// and its first composed line is not its first line.
fn compose_para(
    cx: &Ctx<'_>,
    sg: &[SGlyph],
    flex: &[Flex],
    para: &ParaStyle,
    pen: &Pen<'_>,
    rtl: bool,
    continued: bool,
) -> Option<Vec<(usize, bool)>> {
    let ragged = matches!(para.justify, Justify::Auto | Justify::Left | Justify::Center | Justify::Right);
    let composer = cx.opts.composer.unwrap_or(para.composer);
    // A paragraph whose commas or full stops may hang is composed line by line (as the Japanese
    // single-line composer does).
    let may_hang = para.burasagari != Burasagari::None && sg.iter().any(hangs);
    // A tab's width depends on where it falls in its line, which the composer's fixed glyph widths
    // can't follow: such a paragraph is broken line by line ([`break_line`] measures its tabs).
    // The tab after a list marker always starts the first line and has a fixed width.
    let has_tab = sg.iter().any(|g| g.ch == '\t' && g.len > 0);
    if composer != Composer::EveryLine || pen.regions.is_none() || sg.len() < 2 || sg.len() > MAX_COMPOSE_GLYPHS || may_hang || has_tab {
        return None;
    }
    let m = Metrics::of(&sg[0]);
    // Line widths are independent of the breaks when every line has the same metrics, or when
    // a line's width does not depend on how far down the frame it is set: rectangular cells
    // without text wrap. Then a paragraph whose characters differ in line metrics (type size,
    // font, leading, vertical scale, an inline graphic taller than the text) is composed too. In
    // other shapes, or with text wrap, a taller line can move the lines below it to where the
    // frame is narrower, so such a paragraph is broken line by line.
    let plain = pen.regions.is_some_and(|r| r.iter().all(|r| (r.rect || r.polys.is_empty()) && r.wraps.is_empty()));
    if !plain && sg.iter().any(|g| Metrics::of(g) != m) {
        return None;
    }
    let total: f64 = sg.iter().map(|g| g.adv).sum();
    let mut sim = pen.clone();
    let mut widths = vec![];
    let mut acc = 0.0;
    // A line's width and placement depend on the sum of its indents alone, which the mirrored
    // indents of a right-to-left list ([`line_indents`]) keep.
    while widths.len() < sg.len() {
        let first = widths.is_empty();
        let ind_l = para.left_indent + if first && !continued { para.first_line_indent } else { 0.0 };
        // Only the first line's first word is known here. Later lines are placed as the composed
        // lines need: each fits its width, so its first word does too.
        let word = if first { word_width(sg, 0) } else { 0.0 };
        let Some((b, x0, x1, _)) = sim.place(m, ind_l, para.right_indent, word) else { break };
        let w = x1 - x0 - ind_l - para.right_indent;
        widths.push(w);
        acc += w.max(1.0);
        sim.settled(b, m);
        sim.pending = 0.0;
        if acc > total * 1.6 + 4.0 * w.max(1.0) {
            break;
        }
    }
    let last = *widths.last()?;
    let width = |k: usize| widths.get(k).copied().unwrap_or(last);
    // Lines from `uniform_from` on are interchangeable (same width).
    let uniform_from = widths.iter().rposition(|&w| (w - last).abs() > EPS).map_or(0, |k| k + 1);
    let cands = candidates(cx.text, sg, para.hyphenate.then_some(&para.hyphenation), para.kinsoku);
    let justify_last = para.justify == Justify::JustifyAll;
    let hyphen_limit = usize::try_from(para.hyphenation.hyphen_limit).unwrap_or(usize::MAX);
    let params = |tolerance| Params { justify_last, ragged, tolerance, uniform_from, hyphen_limit };
    // First look for breaks that keep every line within its range (ragged: at most one rag zone,
    // a sixth of the width, left on each line; justified: the Justification options' Minimum and
    // Maximum), then accept any lines that fit, the loosest scored worst. Justified text used to
    // stop at four times its stretch and fall back to the single-line composer; the documented
    // default range (word spacing up to 133%) is narrow enough that most paragraphs would.
    let (strict, loose) = (1.0, f64::INFINITY);
    // The same rule as the line loop below: an opening bracket starting a wrapped line gives up
    // the space before it.
    let start_credit = |i: usize| {
        let flush = !rtl && para.mojikumi == Mojikumi::LineEndHalf;
        sg.get(i).filter(|g| flush && g.lead <= 0.0).and_then(|g| punct_half(g, Punct::Opening)).unwrap_or(0.0)
    };
    compose(sg, &width, &start_credit, &cands, flex, &params(strict)).or_else(|| compose(sg, &width, &start_credit, &cands, flex, &params(loose)))
}

/// The list marker of each of the paragraphs `paras` of `t` (None for none): bullets, and numbers
/// counted through consecutive list paragraphs from [`TextObject::list_count`], as
/// [`vectorcraft_doc::text::ParaList::next_marker`] counts them.
pub(super) fn list_markers(t: &TextObject, paras: &[Range<usize>]) -> Vec<Option<String>> {
    let mut count = t.list_count;
    paras.iter().enumerate().map(|(pi, pr)| vectorcraft_doc::text::ParaList::next_marker(t.para_at(pi).list, pr.is_empty(), &mut count)).collect()
}

/// Whether paragraph `para` (right to left when `rtl`) has its indents mirrored: a horizontal
/// right-to-left list paragraph does.
fn mirrors_indents(para: &ParaStyle, rtl: bool, vertical: bool) -> bool {
    rtl && !vertical && para.list.is_some()
}

/// The indents at the left and at the right of a line of paragraph `para` (its first line when
/// `first`). With `mirror` ([`mirrors_indents`]) the left and first-line indents are measured from
/// the right and the right indent from the left: the bullet of a right-to-left list hangs at the
/// right, and the lines it wraps to line up with the text after the bullet.
fn line_indents(para: &ParaStyle, first: bool, mirror: bool) -> (f64, f64) {
    let start = para.left_indent + if first { para.first_line_indent } else { 0.0 };
    if mirror { (para.right_indent, start) } else { (start, para.right_indent) }
}

/// Flow paragraphs `paras` (byte ranges) of `t`, each with its own paragraph attributes
/// (alignment, indents, spacing, direction, mojikumi, leading model…).
pub(super) fn flow(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, regions: Option<&[Region]>) {
    let mut pen = Pen {
        regions,
        ri: 0,
        prev: None,
        pending: 0.0,
        fb: cx.opts.first_baseline,
        fb_min: cx.opts.first_baseline_min,
        queued: vec![],
        model: t.para_at(0).leading_model,
        next_top: None,
        no_room_for: f64::INFINITY,
    };
    let markers = list_markers(t, paras);
    'paras: for (pi, pr) in paras.iter().enumerate() {
        let para = t.para_at(pi);
        let marker_text = markers.get(pi).and_then(Option::as_deref);
        // The leading model is a paragraph attribute: this paragraph's lines (and the space to
        // its first line) follow its own.
        pen.model = para.leading_model;
        let text = cx.text;
        let bidi = para_bidi_with(text.get(pr.clone()).unwrap_or_default(), para.direction, &cx.char_directions(pr));
        let rtl = is_rtl(bidi.as_ref());
        let mut sg = cx.shape_para(pr.clone(), bidi.as_ref());
        // Thai, Lao, Khmer and Myanmar break lines between words, not at spaces.
        mark_word_ends(text.get(pr.clone()).unwrap_or_default(), pr.start, &mut sg);
        // Japanese composition: consecutive punctuation shares one half-em space.
        let mut compressed = if para.mojikumi == Mojikumi::LineEndHalf { compress_punctuation(&mut sg) } else { vec![] };
        // …and a quarter em between Japanese and Latin text.
        let mut wakan = if para.mojikumi == Mojikumi::LineEndHalf { space_japanese_and_latin(&mut sg) } else { vec![] };
        // Justification…: the glyphs at Desired word and letter spacing and glyph scaling, and how
        // far a justified line may move each one from it.
        let mut flex = apply_desired(&mut sg, &para.justification, !cx.vertical);
        // Threaded text: a first paragraph that continues from the frame before has its first
        // line, first-line indent and list marker there.
        let continued = pi == 0 && t.first_para_continues;
        // Bullets and numbers: the list marker's glyphs `0..marker` start the paragraph.
        // Justification and Optical Margin Alignment leave them as they are.
        let marker = if continued { 0 } else { cx.prepend_marker(pr, para, marker_text, rtl, &mut sg) };
        if marker > 0 {
            flex.splice(0..0, vec![Flex::default(); marker]);
            if !compressed.is_empty() {
                compressed.splice(0..0, vec![false; marker]);
            }
            if !wakan.is_empty() {
                wakan.splice(0..0, vec![0.0; marker]);
            }
        }
        // …and Auto Leading: characters whose leading is Auto take this paragraph's share of their size.
        let auto_leading = |st: &CharStyle| st.leading.unwrap_or_else(|| para.justification.auto_leading_for(st.size));
        if para.justification.auto_leading != vectorcraft_doc::JustificationOptions::default().auto_leading {
            for g in sg.iter_mut().filter(|g| g.inline.is_none()) {
                g.leading = auto_leading(cx.style_at(g.byte));
            }
        }
        let pm = {
            let (asc, desc, _) = style_metrics(cx.db, cx.style_at(pr.start));
            let lead = auto_leading(cx.style_at(pr.start));
            let st = cx.style_at(pr.start);
            let (cap, xh) = cap_x_heights(cx.db, st);
            let centre = cx.db.style_face(st).map_or(EM_CENTER, |f| f.ideographic_centre());
            Metrics { asc, desc, lead, cap, xh, top: (centre + 0.5) * st.size * st.v_scale / 100.0 }
        };
        if pi > 0 {
            pen.pending += para.space_before;
        }
        let n = sg.len();
        let composed = compose_para(cx, &sg, &flex, para, &pen, rtl, continued);
        // Hyphenation options: the Hyphenation Zone applies with the Single-line Composer, and the
        // Hyphen Limit counts the lines in a row that end with a hyphen.
        let zone = if cx.opts.composer.unwrap_or(para.composer) == Composer::SingleLine { para.hyphenation.zone } else { 0.0 };
        let hyphen_limit = usize::try_from(para.hyphenation.hyphen_limit).unwrap_or(usize::MAX);
        let mut hyphen_run = 0;
        let mut li_para = 0;
        let mut i = 0;
        let mirror = mirrors_indents(para, rtl, cx.vertical);
        loop {
            let est = if i < n { Metrics::of(&sg[i]) } else { pm };
            let first_line = li_para == 0 && !continued;
            let (ind_l, ind_r) = line_indents(para, first_line, mirror);
            // Measured before Mojikumi narrows an opening bracket, as the composer measures it.
            let word = word_width(&sg, i);
            // Mojikumi: an opening bracket starting a line is set flush with the line's start (the
            // space before it goes), and the line has that much more room. At a paragraph's start
            // that is the first-line indent: JIS X 4051's principle (JLREQ 3.1.5, Figure 71 ①).
            if !rtl
                && para.mojikumi == Mojikumi::LineEndHalf
                && let Some(g) = sg.get_mut(i)
                && g.lead <= 0.0
                && let Some(h) = punct_half(g, Punct::Opening)
            {
                g.adv -= h;
                g.lead += h;
            }
            // Place, break, then settle the baseline on the line's real metrics (moving on to the
            // next row/column if it no longer fits).
            let (baseline, x0, x1, end, hyph, m) = loop {
                let first_in_region = pen.prev.is_none();
                let Some((mut baseline, x0, x1, same_baseline)) = pen.place(est, ind_l, ind_r, word) else {
                    cx.out.overflow = cx.text.len() > if i < n { sg[i].byte } else { pr.start };
                    break 'paras;
                };
                let width = x1 - x0 - ind_l - ind_r;
                let hyphenate = (para.hyphenate && (hyphen_limit == 0 || hyphen_run < hyphen_limit)).then_some((&para.hyphenation, zone));
                let (end, hyph) = match composed.as_ref().and_then(|c| c.get(li_para)) {
                    Some(&(e, h)) if e > i => (e, h),
                    _ if i < n => break_line(cx.text, &sg, i, width, &para.tabs, ind_l, hyphenate, para.burasagari, para.kinsoku),
                    _ => (n, false),
                };
                let m = Metrics::max(&sg[i..end]).unwrap_or(pm);
                baseline += if same_baseline || (pen.regions.is_none() && first_in_region) {
                    0.0
                } else if pen.model == LeadingModel::EmBoxTop {
                    // The em box top is fixed (the frame's top, or the line above's): the baseline
                    // hangs from it by this line's tallest em box.
                    if first_in_region { m.top.max(pen.fb_min) - est.top.max(pen.fb_min) } else { m.top - est.top }
                } else if first_in_region {
                    m.first_baseline(pen.fb, pen.fb_min) - est.first_baseline(pen.fb, pen.fb_min)
                } else {
                    m.lead - est.lead
                };
                if pen.regions.is_some() && baseline + m.desc > pen.bottom() + 0.01 {
                    // Try the next row/column; overflow if there is none.
                    pen.ri += 1;
                    pen.prev = None;
                    pen.pending = 0.0;
                    pen.queued.clear();
                    continue;
                }
                break (baseline, x0, x1, end, hyph, m);
            };
            hyphen_run = if hyph && end > i { hyphen_run + 1 } else { 0 };
            let (ax0, ax1) = if regions.is_some() { (x0 + ind_l, x1 - ind_r) } else { (x0, x1) };
            let width = ax1 - ax0;
            pen.pending = 0.0;
            let last_of_para = end >= n;
            let mut trimmed = end;
            while trimmed > i && sg[trimmed - 1].is_space() {
                trimmed -= 1;
            }
            let hyphen = (hyph && end > i).then(|| hyphen_glyph(&sg[end - 1]));
            // Mojikumi: a closing bracket, comma or full stop ending the line is set half width, and
            // no Japanese–Latin space is left at the end of a line.
            let end_trim = match trimmed.checked_sub(1).filter(|&k| k >= i && para.mojikumi == Mojikumi::LineEndHalf) {
                Some(k) => {
                    let half = if compressed.get(k).copied().unwrap_or(false) { None } else { sg.get(k).and_then(|g| punct_half(g, Punct::Closing)) };
                    half.unwrap_or(0.0) + wakan.get(k).copied().unwrap_or(0.0)
                }
                None => 0.0,
            };
            let w: f64 = set_width(&sg, i, trimmed, &para.tabs, ind_l) + hyphen.as_ref().map_or(0.0, |h| h.adv) - end_trim;
            let line_start = sg.get(i).map_or(pr.start, |g| g.byte);
            let line_end = sg.get(end).map_or(pr.end, |g| g.byte);
            let order = visual_order(bidi.as_ref().filter(|_| !cx.vertical), pr.start, line_start..line_end, &sg[i..end]);
            // Optical Margin Alignment or Roman Hanging Punctuation: the share of a glyph that hangs.
            let share: Option<fn(&SGlyph) -> f64> =
                (t.optical_margin || para.roman_hanging).then_some(if t.optical_margin { optical_share } else { roman_share });
            // The glyphs before `stop` in visual order, each with the advance it is set with. A list
            // marker does not hang: it is counted with no width.
            let set = |j: usize, g: &SGlyph| {
                if j < marker {
                    0.0
                } else if j + 1 == trimmed {
                    g.adv - end_trim
                } else {
                    g.adv
                }
            };
            let visible = |stop: usize| -> (Vec<usize>, Vec<(&SGlyph, f64)>) {
                let at: Vec<usize> = order.iter().map(|&offset| i + offset).filter(|&j| j < stop).collect();
                let glyphs = at.iter().filter_map(|&j| sg.get(j).map(|g| (g, set(j, g)))).collect();
                (at, glyphs)
            };
            // Burasagari: a comma or full stop ending an area type line hangs outside it (Standard:
            // when it doesn't fit, counting what hangs at the line's start as room; Forced:
            // always). The rest of the line is aligned and justified without it, and it follows
            // the line's last character.
            let room_at_start = || share.map_or(0.0, |share| optical_hangs(&visible(trimmed).1, share).0);
            let hang = (regions.is_some() && !rtl && hyphen.is_none() && trimmed > i + 1)
                .then(|| sg.get(trimmed - 1))
                .flatten()
                .filter(|g| hangs(g))
                .map(|g| g.adv - end_trim)
                .filter(|_| match para.burasagari {
                    Burasagari::None => false,
                    Burasagari::Standard => w > width + room_at_start() + EPS,
                    Burasagari::Forced => true,
                });
            let body = if hang.is_some() { trimmed - 1 } else { trimmed };
            let w = w - hang.unwrap_or(0.0);
            // The glyphs before the line's trailing spaces (and its burasagari mark), in the order
            // they are drawn.
            let (drawn, mut visible) = visible(body);
            // Optical Margin Alignment: the glyphs at the ends of a line hang outside its margins
            // by `opt_l` and `opt_r`. The line is aligned and justified in a measure that much
            // wider at each end. A left-aligned line then hangs only at its left, a right-aligned
            // line hangs only at its right, and centered and justified lines hang at both; Auto
            // aligns a right-to-left line at its right. A right-to-left line is read from right to
            // left ([`optical_hangs_rtl`]). A line that ends in a burasagari mark hangs only at its
            // left. Justification adds no space after a punctuation mark hanging at the left, nor
            // before one hanging at the right (`held`). A letter that hangs by part of its width
            // keeps its gaps. Glyph Scaling leaves the glyphs that hang (`hung`) at their width, so
            // they hang exactly by their published share. Roman Hanging Punctuation (a paragraph
            // attribute) hangs the same punctuation by the same amounts, and no letters; with
            // Optical Margin Alignment on as well, the letters hang too.
            let (opt_l, opt_r, held, hung) = if let Some(share) = share {
                if let Some(h) = &hyphen {
                    visible.push((h, h.adv));
                }
                let (l, r, left_glyphs, right_glyphs) =
                    if rtl && !cx.vertical { optical_hangs_rtl(&visible, share) } else { optical_hangs(&visible, share) };
                let (r, right_glyphs) = if hang.is_some() { (0.0, 0) } else { (r, right_glyphs) };
                let n_visible = visible.len();
                let punct = |k: usize| visible.get(k).is_some_and(|&(g, _)| optical_punct(g));
                let held: Vec<usize> = drawn
                    .iter()
                    .enumerate()
                    .filter(|&(k, _)| (k < left_glyphs && punct(k)) || (right_glyphs > 0 && k + 1 + right_glyphs >= n_visible && punct(k + 1)))
                    .map(|(_, &j)| j)
                    .collect();
                let hung: Vec<usize> = drawn
                    .iter()
                    .enumerate()
                    .filter(|&(k, _)| k < left_glyphs || (right_glyphs > 0 && k + right_glyphs >= n_visible))
                    .map(|(_, &j)| j)
                    .collect();
                (l, r, held, hung)
            } else {
                (0.0, 0.0, vec![], vec![])
            };
            // Justification adds nothing to the list marker's glyphs.
            let mut held = held;
            held.extend(i..marker.min(body));
            // What justification may change of glyph `j`: nothing of a `held` glyph, and no glyph
            // scaling of a `hung` one.
            let line_flex = |j: usize| {
                if held.contains(&j) {
                    return Flex::default();
                }
                let mut f = flex.get(j).copied().unwrap_or_default();
                if hung.contains(&j) {
                    f.glyph = (0.0, 0.0);
                }
                f
            };
            let room = width + opt_l + opt_r;
            let (align, justify) = match para.justify {
                Justify::Auto => (if rtl { 2 } else { 0 }, false),
                Justify::Left => (0, false),
                Justify::Center => (1, false),
                Justify::Right => (2, false),
                Justify::JustifyLeft => (0, !last_of_para),
                Justify::JustifyCenter => (1, !last_of_para),
                Justify::JustifyRight => (2, !last_of_para),
                Justify::JustifyAll => (0, true),
            };
            let justify = justify && regions.is_some();
            let (mut per_space, mut per_gap, mut per_cjk) = (0.0, 0.0, 0.0);
            let mut fit = LineFit::default();
            let spaces = (i..body).filter(|&j| j >= marker && sg[j].is_space()).count();
            // A gap that justification adds between two glyphs goes into the advance of the one
            // drawn first: `drawn_after(j)` is the glyph drawn right after glyph `j`, which in a
            // right-to-left line comes before it in the text.
            let mut after: Vec<Option<usize>> = vec![None; body.saturating_sub(i)];
            for (&a, &b) in drawn.iter().zip(drawn.iter().skip(1)) {
                if let Some(slot) = after.get_mut(a - i) {
                    *slot = Some(b);
                }
            }
            let drawn_after = |j: usize| j.checked_sub(i).and_then(|k| after.get(k).copied().flatten());
            // Japanese (and Chinese) lines are justified between their characters (JLREQ 3.8): the
            // gaps next to a CJK character, not inside a Latin word or a tate-chu-yoko block.
            let cjk_gap = |j: usize| {
                !held.contains(&j)
                    && drawn_after(j)
                        .and_then(|k| sg.get(j).zip(sg.get(k)))
                        .is_some_and(|(a, b)| !a.is_space() && !b.is_space() && !b.continues_tcy() && a.ch != '\t' && (is_cjk(a.ch) || is_cjk(b.ch)))
            };
            let cjk_gaps = (i..body).filter(|&j| cjk_gap(j)).count();
            // Single Word Justification: a justified line holding one word (no word space) is
            // spread between its letters (Full Justify) or aligned.
            let single_word = justify && spaces == 0 && cjk_gaps == 0;
            let (align, justify) = match (single_word, para.justification.single_word) {
                (false, _) | (true, SingleWordJustify::FullJustify) => (align, justify),
                (true, SingleWordJustify::AlignLeft) => (0, false),
                (true, SingleWordJustify::AlignCenter) => (1, false),
                (true, SingleWordJustify::AlignRight) => (2, false),
            };
            // Insert Kashidas: the Arabic words of a horizontal line take its room on kashidas,
            // and the word spaces and gaps take what is left.
            let kashidas =
                if justify && !cx.vertical && room > w + EPS { kashida::plan(cx.text, &sg, i..body, para.kashida, room - w) } else { vec![] };
            let w = w + kashidas.iter().map(|k| k.1).sum::<f64>();
            if justify && (room - w).abs() > EPS {
                if cjk_gaps > 0 && room > w {
                    // Spread over the CJK gaps and the word spaces alike.
                    let per = (room - w) / (cjk_gaps + spaces) as f64;
                    (per_space, per_cjk) = (per, per);
                } else if spaces > 0 {
                    // Word spacing, letter spacing and glyph scaling within their ranges, then the
                    // word spaces alone; composed lines may shrink word spaces (never below zero).
                    let line: Vec<Flex> = (i..body).map(line_flex).collect();
                    let floor = (i..body).filter(|&j| j >= marker && sg[j].is_space()).map(|j| sg[j].adv).fold(f64::MAX, f64::min);
                    fit = fit_line(&line, room - w, spaces, floor);
                } else if room > w {
                    // One word: its letter spacing and glyph scaling, then the gaps between its glyphs.
                    let line: Vec<Flex> = (i..body).map(line_flex).collect();
                    fit = fit_line(&line, room - w, 0, 0.0);
                    let used: f64 = line.iter().map(|f| fit.letter(f) + fit.glyph(f)).sum();
                    let rest = room - w - used;
                    let gaps = (i..body)
                        .filter(|&j| !held.contains(&j) && drawn_after(j).and_then(|k| sg.get(k)).is_some_and(|next| !next.continues_tcy()))
                        .count();
                    if gaps > 0 && rest > EPS {
                        per_gap = rest / gaps as f64;
                    }
                }
            }
            let start_x = if justify {
                ax0 - opt_l
            } else if regions.is_none() {
                match align {
                    0 => ind_l - opt_l,
                    1 => (ind_l - opt_l - ind_r + opt_r - w) * 0.5,
                    _ => -ind_r + opt_r - w,
                }
            } else {
                match align {
                    0 => ax0 - opt_l,
                    1 => ax0 - opt_l + (room - w) * 0.5,
                    _ => ax1 + opt_r - w,
                }
            };
            let li = cx.out.lines.len();
            let glyph_start = cx.out.glyphs.len();
            // Character Alignment lines smaller characters up with the line's largest em box.
            let line_em = sg.get(i..end).map_or(0.0, |line| line.iter().map(glyph_em).fold(0.0, f64::max));
            let mut x_end = start_x;
            // Tab stops are measured from the frame's left edge (point type: the origin).
            let tab_origin = if regions.is_some() { x0 } else { 0.0 };
            // A line with tabs is measured as if it started at the left (see `set_width`), then
            // aligned as a whole: its tabs stop where they were measured, however it is aligned.
            let tab_shift = start_x - if regions.is_some() { ax0 } else { ind_l };
            // RTL's logical trailing spaces precede the visible content. Keep them outside
            // the aligned content extent, just as LTR trailing spaces extend to its right.
            let leading_space_width: f64 =
                order.iter().take_while(|&&offset| i + offset >= trimmed).filter_map(|&offset| sg.get(i + offset)).map(|g| g.adv).sum();
            let mut x = start_x - leading_space_width;
            // The glyphs drawn for the list marker, and the index of its tab's glyph, when the line starts the paragraph.
            let mut marker_glyphs: Option<Range<usize>> = None;
            let mut marker_tab = 0;
            for (oi, &offset) in order.iter().enumerate() {
                let j = i + offset;
                let Some(g) = sg.get(j) else { continue };
                let mut adv = g.adv;
                if j + 1 == trimmed {
                    adv -= end_trim;
                }
                // Glyph scaling a justified line adds to the glyph (1: none).
                let mut scale = 1.0;
                let mut leader = "";
                if g.ch == '\t' {
                    // The tab after a list marker keeps the width `prepend_marker` gave it.
                    if g.len > 0 {
                        (adv, leader) = tab_advance(&para.tabs, tab_origin, x - tab_shift, &sg[j + 1..trimmed.max(j + 1)]);
                    }
                } else if j < body {
                    let f = line_flex(j);
                    if is_word_space(g) {
                        adv += fit.space(&f);
                    } else {
                        adv += fit.letter(&f) + fit.glyph(&f);
                        scale = fit.glyph_scale(&f);
                    }
                    if g.is_space() {
                        adv += per_space;
                    } else if per_cjk != 0.0 {
                        if cjk_gap(j) {
                            adv += per_cjk;
                        }
                    } else if !held.contains(&j) && drawn_after(j).and_then(|k| sg.get(k)).is_some_and(|next| !next.continues_tcy()) {
                        // Between glyphs, never inside a tate-chu-yoko block (one cell).
                        adv += per_gap;
                    }
                }
                // The kashida before a cluster in the text is drawn at the cluster's right, between
                // it and the letter before it, and the cluster's last glyph advances over it: the
                // caret, selection and hit testing take it as part of the cluster.
                let kashida = kashidas
                    .binary_search_by_key(&g.byte, |k| k.0)
                    .ok()
                    .and_then(|at| kashidas.get(at))
                    .filter(|_| order.get(oi + 1).and_then(|&o| sg.get(i + o)).is_none_or(|next| next.byte != g.byte))
                    .map(|k| k.1);
                adv += kashida.unwrap_or(0.0);
                let y = baseline - align_shift(g, cx.style_at(g.byte).char_align, line_em, cx.vertical);
                let place = if scale != 1.0 { Affine::translate((x, y)) * Affine::scale_non_uniform(scale, 1.0) } else { Affine::translate((x, y)) };
                if j < marker {
                    let at = cx.out.glyphs.len();
                    marker_glyphs = Some(marker_glyphs.map_or(at..at + 1, |r: Range<usize>| r.start.min(at)..r.end.max(at + 1)));
                    if j + 1 == marker {
                        marker_tab = at;
                    }
                }
                cx.emit(g, place, Point::new(x, y), 0.0, adv, li);
                if let Some(out) = cx.out.glyphs.last_mut() {
                    out.scale = flex.get(j).map_or(1.0, |f| f.scale) * scale;
                }
                if !leader.is_empty() {
                    // The leader belongs to the tab's glyph: the gap stays one character for the
                    // caret, selection and hit testing.
                    let dots = leader_outline(cx.db, g, leader, tab_origin, x, x + adv, y);
                    if let Some(tab) = cx.out.glyphs.last_mut() {
                        tab.outline.extend(dots);
                    }
                }
                if let Some(len) = kashida {
                    cx.emit_kashida(g, para.kashida, len, Point::new(x + adv - len, y), li);
                }
                x += adv;
                if j < trimmed {
                    x_end = x;
                }
            }
            if let (Some(glyphs), Some(text)) = (marker_glyphs, marker_text) {
                cx.out.markers.push(ListMarker { glyphs, tab: marker_tab, text: text.to_string() });
            }
            if let Some(h) = &hyphen {
                // The hyphen follows the last non-space glyph.
                let hx = x_end;
                cx.emit(h, Affine::translate((hx, baseline)), Point::new(hx, baseline), 0.0, h.adv, li);
                x_end = hx + h.adv;
            }
            cx.out.lines.push(LineInfo {
                rtl: rtl && !cx.vertical,
                baseline,
                x0: start_x,
                x1: x_end,
                ascent: m.asc,
                descent: m.desc,
                first_baseline: if pen.model == LeadingModel::EmBoxTop { m.top.max(pen.fb_min) } else { m.first_baseline(pen.fb, pen.fb_min) },
                start: if i < n { sg[i].byte } else { pr.start },
                end: if last_of_para { pr.end } else { sg[end].byte },
                glyph_start,
                glyph_end: cx.out.glyphs.len(),
                avail: if regions.is_some() { (ax0, ax1) } else { (start_x, x_end) },
                region: if regions.is_some() { pen.ri } else { 0 },
            });
            pen.settled(baseline, m);
            // Further spans of this line band share the settled baseline.
            for q in &mut pen.queued {
                q.0 = baseline;
            }
            li_para += 1;
            i = end;
            if i >= n {
                break;
            }
        }
        pen.pending += para.space_after;
    }
}
