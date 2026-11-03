use super::bidi::{is_rtl, para_bidi_with, visual_order};
use super::breaking::{
    Metrics, Pen, break_line, candidates, hangs, leader_outline, optical_hangs, optical_hangs_rtl, optical_punct, optical_share, roman_share,
    set_width, tab_advance, word_width,
};
use super::region::Region;
use super::vertical::{EM_CENTER, align_shift, compress_punctuation, glyph_em, punct_half, space_japanese_and_latin};
use super::{
    Affine, BidiInfo, Burasagari, CharStyle, Composer, Ctx, EPS, Flex, Justify, LeadingModel, LineFit, LineInfo, ListMarker, Mojikumi, ParaStyle,
    Params, Point, Punct, Range, SGlyph, SingleWordJustify, TextObject, apply_desired, cap_x_heights, compose, fit_line, hyphen_glyph, is_cjk,
    is_word_space, kashida, mark_word_ends, style_metrics,
};

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
    let has_soft_break = sg.iter().any(|g| g.ch == '\u{2028}');
    if composer != Composer::EveryLine
        || pen.regions.is_none()
        || sg.len() < 2
        || sg.len() > MAX_COMPOSE_GLYPHS
        || may_hang
        || has_tab
        || has_soft_break
    {
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
    for (pi, pr) in paras.iter().enumerate() {
        let marker_text = markers.get(pi).and_then(Option::as_deref);
        if !flow_paragraph(cx, &mut pen, t, pi, pr.clone(), marker_text, regions) {
            break;
        }
    }
}

fn flow_paragraph(
    cx: &mut Ctx<'_>,
    pen: &mut Pen<'_>,
    t: &TextObject,
    pi: usize,
    pr: Range<usize>,
    marker_text: Option<&str>,
    regions: Option<&[Region]>,
) -> bool {
    let paragraph = prepare_paragraph(cx, pen, t, pi, pr, marker_text, regions);
    run_paragraph_lines(cx, pen, t, paragraph)
}

struct ParagraphData<'a, 'text> {
    para: &'a ParaStyle,
    pr: Range<usize>,
    marker_text: Option<&'a str>,
    bidi: Option<BidiInfo<'text>>,
    rtl: bool,
    continued: bool,
    sg: Vec<SGlyph>,
    compressed: Vec<bool>,
    wakan: Vec<f64>,
    flex: Vec<Flex>,
    marker: usize,
    pm: Metrics,
    composed: Option<Vec<(usize, bool)>>,
    zone: f64,
    hyphen_limit: usize,
    mirror: bool,
    regions: Option<&'a [Region]>,
}

fn prepare_paragraph<'a, 'text>(
    cx: &mut Ctx<'text>,
    pen: &mut Pen<'_>,
    t: &'a TextObject,
    pi: usize,
    pr: Range<usize>,
    marker_text: Option<&'a str>,
    regions: Option<&'a [Region]>,
) -> ParagraphData<'a, 'text> {
    let para = t.para_at(pi);
    // The leading model is a paragraph attribute: this paragraph's lines (and the space to
    // its first line) follow its own.
    pen.model = para.leading_model;
    let text = cx.text;
    let bidi = para_bidi_with(text.get(pr.clone()).unwrap_or_default(), para.direction, &cx.char_directions(&pr));
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
    let marker = if continued { 0 } else { cx.prepend_marker(&pr, para, marker_text, rtl, &mut sg) };
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
    let auto_leading = |style: &CharStyle| style.leading.unwrap_or_else(|| para.justification.auto_leading_for(style.size));
    if para.justification.auto_leading != vectorcraft_doc::JustificationOptions::default().auto_leading {
        for glyph in sg.iter_mut().filter(|glyph| glyph.inline.is_none()) {
            glyph.leading = auto_leading(cx.style_at(glyph.byte));
        }
    }
    let pm = {
        let (asc, desc, _) = style_metrics(cx.db, cx.style_at(pr.start));
        let lead = auto_leading(cx.style_at(pr.start));
        let style = cx.style_at(pr.start);
        let (cap, xh) = cap_x_heights(cx.db, style);
        let centre = cx.db.style_face(style).map_or(EM_CENTER, |face| face.ideographic_centre());
        Metrics { asc, desc, lead, cap, xh, top: (centre + 0.5) * style.size * style.v_scale / 100.0 }
    };
    if pi > 0 {
        pen.pending += para.space_before;
    }
    let composed = compose_para(cx, &sg, &flex, para, pen, rtl, continued);
    // Hyphenation options: the Hyphenation Zone applies with the Single-line Composer, and the
    // Hyphen Limit counts the lines in a row that end with a hyphen.
    let zone = if cx.opts.composer.unwrap_or(para.composer) == Composer::SingleLine { para.hyphenation.zone } else { 0.0 };
    let hyphen_limit = usize::try_from(para.hyphenation.hyphen_limit).unwrap_or(usize::MAX);
    let mirror = mirrors_indents(para, rtl, cx.vertical);
    ParagraphData {
        para,
        pr,
        marker_text,
        bidi,
        rtl,
        continued,
        sg,
        compressed,
        wakan,
        flex,
        marker,
        pm,
        composed,
        zone,
        hyphen_limit,
        mirror,
        regions,
    }
}

struct PlacementContext<'a> {
    para: &'a ParaStyle,
    pr: &'a Range<usize>,
    sg: &'a mut [SGlyph],
    continued: bool,
    rtl: bool,
    mirror: bool,
    pm: Metrics,
    composed: &'a Option<Vec<(usize, bool)>>,
    hyphen_run: usize,
    hyphen_limit: usize,
    zone: f64,
    index: usize,
    para_line: usize,
}

struct LinePosition {
    baseline: f64,
    x0: f64,
    x1: f64,
    end: usize,
    hyph: bool,
    m: Metrics,
    ind_l: f64,
    ind_r: f64,
}

fn place_flow_line(cx: &mut Ctx<'_>, pen: &mut Pen<'_>, context: &mut PlacementContext<'_>) -> Option<LinePosition> {
    let PlacementContext { para, pr, sg, continued, rtl, mirror, pm, composed, hyphen_run, hyphen_limit, zone, index: i, para_line } = context;
    let n = sg.len();
    let est = if *i < n { Metrics::of(&sg[*i]) } else { *pm };
    let first_line = *para_line == 0 && !*continued;
    let (ind_l, ind_r) = line_indents(para, first_line, *mirror);
    // Measured before Mojikumi narrows an opening bracket, as the composer measures it.
    let word = word_width(sg, *i);
    // Mojikumi: an opening bracket starting a line is set flush with the line's start (the
    // space before it goes), and the line has that much more room. At a paragraph's start
    // that is the first-line indent: JIS X 4051's principle (JLREQ 3.1.5, Figure 71 ①).
    if !*rtl
        && para.mojikumi == Mojikumi::LineEndHalf
        && let Some(glyph) = sg.get_mut(*i)
        && glyph.lead <= 0.0
        && let Some(half) = punct_half(glyph, Punct::Opening)
    {
        glyph.adv -= half;
        glyph.lead += half;
    }
    // Place, break, then settle the baseline on the line's real metrics (moving on to the
    // next row/column if it no longer fits).
    loop {
        let first_in_region = pen.prev.is_none();
        let Some((mut baseline, x0, x1, same_baseline)) = pen.place(est, ind_l, ind_r, word) else {
            cx.out.overflow = cx.text.len() > if *i < n { sg[*i].byte } else { pr.start };
            return None;
        };
        let width = x1 - x0 - ind_l - ind_r;
        let hyphenate = (para.hyphenate && (*hyphen_limit == 0 || *hyphen_run < *hyphen_limit)).then_some((&para.hyphenation, *zone));
        let (end, hyph) = match composed.as_ref().and_then(|lines| lines.get(*para_line)) {
            Some(&(end, hyph)) if end > *i => (end, hyph),
            _ if *i < n => break_line(cx.text, sg, *i, width, &para.tabs, ind_l, hyphenate, para.burasagari, para.kinsoku),
            _ => (n, false),
        };
        let m = Metrics::max(&sg[*i..end]).unwrap_or(*pm);
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
        return Some(LinePosition { baseline, x0, x1, end, hyph, m, ind_l, ind_r });
    }
}

struct LineGeometry {
    position: LinePosition,
    index: usize,
    trimmed: usize,
    body: usize,
    end_trim: f64,
    hyphen: Option<SGlyph>,
    ax0: f64,
    ax1: f64,
    width: f64,
    render_width: f64,
    order: Vec<usize>,
    drawn: Vec<usize>,
    opt_l: f64,
    opt_r: f64,
    held: Vec<usize>,
    hung: Vec<usize>,
    last_of_para: bool,
}

struct LineJustification {
    per_space: f64,
    per_gap: f64,
    per_cjk: f64,
    fit: LineFit,
    start_x: f64,
    after: Vec<Option<usize>>,
    kashidas: Vec<(usize, f64)>,
}

fn run_paragraph_lines(cx: &mut Ctx<'_>, pen: &mut Pen<'_>, t: &TextObject, mut paragraph: ParagraphData<'_, '_>) -> bool {
    let n = paragraph.sg.len();
    let mut hyphen_run = 0;
    let mut line_in_paragraph = 0;
    let mut index = 0;
    let mut emitted_empty_continuation = false;
    loop {
        let pm = if index >= n
            && let Some(last) = paragraph.sg.last()
        {
            Metrics::of(last)
        } else {
            paragraph.pm
        };
        let mut placement = PlacementContext {
            para: paragraph.para,
            pr: &paragraph.pr,
            sg: &mut paragraph.sg,
            continued: paragraph.continued,
            rtl: paragraph.rtl,
            mirror: paragraph.mirror,
            pm,
            composed: &paragraph.composed,
            hyphen_run,
            hyphen_limit: paragraph.hyphen_limit,
            zone: paragraph.zone,
            index,
            para_line: line_in_paragraph,
        };
        let Some(position) = place_flow_line(cx, pen, &mut placement) else { return false };
        hyphen_run = if position.hyph && position.end > index { hyphen_run + 1 } else { 0 };
        pen.pending = 0.0;
        let geometry = prepare_line_geometry(cx, t, &paragraph, position, index);
        let justification = justify_flow_line(cx.vertical, cx.text, &paragraph, &geometry);
        emit_flow_line(cx, pen, &paragraph, &geometry, &justification);
        line_in_paragraph += 1;
        index = geometry.position.end;
        if index >= n {
            if emitted_empty_continuation {
                break;
            }
            emitted_empty_continuation = paragraph.sg.last().is_some_and(|g| g.ch == '\u{2028}');
            if !emitted_empty_continuation {
                break;
            }
        }
    }
    pen.pending += paragraph.para.space_after;
    true
}

fn prepare_line_geometry(cx: &Ctx<'_>, t: &TextObject, paragraph: &ParagraphData<'_, '_>, position: LinePosition, index: usize) -> LineGeometry {
    let para = paragraph.para;
    let sg = &paragraph.sg;
    let n = sg.len();
    let (ax0, ax1) =
        if paragraph.regions.is_some() { (position.x0 + position.ind_l, position.x1 - position.ind_r) } else { (position.x0, position.x1) };
    let width = ax1 - ax0;
    let mut trimmed = position.end;
    while trimmed > index && (sg[trimmed - 1].is_space() || sg[trimmed - 1].ch == '\u{2028}') {
        trimmed -= 1;
    }
    let hyphen = (position.hyph && position.end > index).then(|| hyphen_glyph(&sg[position.end - 1]));
    // Mojikumi: a closing bracket, comma or full stop ending the line is set half width, and
    // no Japanese–Latin space is left at the end of a line.
    let end_trim = trimmed.checked_sub(1).filter(|&k| k >= index && para.mojikumi == Mojikumi::LineEndHalf).map_or(0.0, |k| {
        let half = if paragraph.compressed.get(k).copied().unwrap_or(false) { None } else { sg.get(k).and_then(|g| punct_half(g, Punct::Closing)) };
        half.unwrap_or(0.0) + paragraph.wakan.get(k).copied().unwrap_or(0.0)
    });
    let measured_width = set_width(sg, index, trimmed, &para.tabs, position.ind_l) + hyphen.as_ref().map_or(0.0, |g| g.adv) - end_trim;
    let line_start = sg.get(index).map_or(paragraph.pr.start, |g| g.byte);
    let line_end = sg.get(position.end).map_or(paragraph.pr.end, |g| g.byte);
    let order = visual_order(paragraph.bidi.as_ref().filter(|_| !cx.vertical), paragraph.pr.start, line_start..line_end, &sg[index..position.end]);
    // The glyphs before `stop` in visual order, each with the advance it is set with. A list
    // marker does not hang: it is counted with no width.
    let visible = |stop: usize| -> (Vec<usize>, Vec<(&SGlyph, f64)>) {
        let indices: Vec<usize> = order.iter().map(|&offset| index + offset).filter(|&j| j < stop).collect();
        let glyphs = indices
            .iter()
            .filter_map(|&j| {
                sg.get(j).map(|g| {
                    (
                        g,
                        if j < paragraph.marker {
                            0.0
                        } else if j + 1 == trimmed {
                            g.adv - end_trim
                        } else {
                            g.adv
                        },
                    )
                })
            })
            .collect();
        (indices, glyphs)
    };
    // Optical Margin Alignment or Roman Hanging Punctuation: the share of a glyph that hangs.
    let share = (t.optical_margin || para.roman_hanging).then_some(if t.optical_margin { optical_share } else { roman_share });
    // Burasagari: a comma or full stop ending an area type line hangs outside it (Standard:
    // when it doesn't fit, counting what hangs at the line's start as room; Forced:
    // always). The rest of the line is aligned and justified without it, and it follows
    // the line's last character.
    let room_at_start = || share.map_or(0.0, |share| optical_hangs(&visible(trimmed).1, share).0);
    let hang = (paragraph.regions.is_some() && !paragraph.rtl && hyphen.is_none() && trimmed > index + 1)
        .then(|| sg.get(trimmed - 1))
        .flatten()
        .filter(|g| hangs(g))
        .map(|g| g.adv - end_trim)
        .filter(|_| match para.burasagari {
            Burasagari::None => false,
            Burasagari::Standard => measured_width > width + room_at_start() + EPS,
            Burasagari::Forced => true,
        });
    let body = if hang.is_some() { trimmed - 1 } else { trimmed };
    let render_width = measured_width - hang.unwrap_or(0.0);
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
        if let Some(hyphen) = &hyphen {
            visible.push((hyphen, hyphen.adv));
        }
        let (left, right, left_count, right_count) =
            if paragraph.rtl && !cx.vertical { optical_hangs_rtl(&visible, share) } else { optical_hangs(&visible, share) };
        let (right, right_count) = if hang.is_some() { (0.0, 0) } else { (right, right_count) };
        let visible_count = visible.len();
        let punctuation = |k: usize| visible.get(k).is_some_and(|&(g, _)| optical_punct(g));
        let held = drawn
            .iter()
            .enumerate()
            .filter(|&(k, _)| (k < left_count && punctuation(k)) || (right_count > 0 && k + 1 + right_count >= visible_count && punctuation(k + 1)))
            .map(|(_, &j)| j)
            .collect();
        let hung = drawn
            .iter()
            .enumerate()
            .filter(|&(k, _)| k < left_count || (right_count > 0 && k + right_count >= visible_count))
            .map(|(_, &j)| j)
            .collect();
        (left, right, held, hung)
    } else {
        (0.0, 0.0, Vec::new(), Vec::new())
    };
    // Justification adds nothing to the list marker's glyphs.
    let mut held = held;
    held.extend(index..paragraph.marker.min(body));
    let last_of_para = position.end >= n || position.end.checked_sub(1).and_then(|i| sg.get(i)).is_some_and(|g| g.ch == '\u{2028}');
    LineGeometry {
        position,
        index,
        trimmed,
        body,
        end_trim,
        hyphen,
        ax0,
        ax1,
        width,
        render_width,
        order,
        drawn,
        opt_l,
        opt_r,
        held,
        hung,
        last_of_para,
    }
}

fn drawn_after(index: usize, glyph: usize, after: &[Option<usize>]) -> Option<usize> {
    glyph.checked_sub(index).and_then(|offset| after.get(offset).copied().flatten())
}

// Japanese (and Chinese) lines are justified between their characters (JLREQ 3.8): the
// gaps next to a CJK character, not inside a Latin word or a tate-chu-yoko block.
fn is_cjk_gap(glyph: usize, held: &[usize], after: &[Option<usize>], index: usize, sg: &[SGlyph]) -> bool {
    !held.contains(&glyph)
        && drawn_after(index, glyph, after)
            .and_then(|next| sg.get(glyph).zip(sg.get(next)))
            .is_some_and(|(a, b)| !a.is_space() && !b.is_space() && !b.continues_tcy() && a.ch != '\t' && (is_cjk(a.ch) || is_cjk(b.ch)))
}

// What justification may change of glyph `j`: nothing of a `held` glyph, and no glyph
// scaling of a `hung` one.
fn flow_line_flex(glyph: usize, flex: &[Flex], held: &[usize], hung: &[usize]) -> Flex {
    if held.contains(&glyph) {
        return Flex::default();
    }
    let mut value = flex.get(glyph).copied().unwrap_or_default();
    if hung.contains(&glyph) {
        value.glyph = (0.0, 0.0);
    }
    value
}

fn justify_flow_line(vertical: bool, text: &str, paragraph: &ParagraphData<'_, '_>, line: &LineGeometry) -> LineJustification {
    let para = paragraph.para;
    let sg = &paragraph.sg;
    let (held, hung) = (&line.held, &line.hung);
    let room = line.width + line.opt_l + line.opt_r;
    let (mut align, mut justify) = match para.justify {
        Justify::Auto => (if paragraph.rtl { 2 } else { 0 }, false),
        Justify::Left => (0, false),
        Justify::Center => (1, false),
        Justify::Right => (2, false),
        Justify::JustifyLeft => (0, !line.last_of_para),
        Justify::JustifyCenter => (1, !line.last_of_para),
        Justify::JustifyRight => (2, !line.last_of_para),
        Justify::JustifyAll => (0, true),
    };
    justify &= paragraph.regions.is_some();
    let spaces = (line.index..line.body).filter(|&j| j >= paragraph.marker && sg[j].is_space()).count();
    // A gap that justification adds between two glyphs goes into the advance of the one
    // drawn first: `drawn_after(j)` is the glyph drawn right after glyph `j`, which in a
    // right-to-left line comes before it in the text.
    let mut after: Vec<Option<usize>> = vec![None; line.body.saturating_sub(line.index)];
    for (&a, &b) in line.drawn.iter().zip(line.drawn.iter().skip(1)) {
        if let Some(slot) = after.get_mut(a - line.index) {
            *slot = Some(b);
        }
    }
    let cjk_gaps = (line.index..line.body).filter(|&j| is_cjk_gap(j, held, &after, line.index, sg)).count();
    // Single Word Justification: a justified line holding one word (no word space) is
    // spread between its letters (Full Justify) or aligned.
    let single_word = justify && spaces == 0 && cjk_gaps == 0;
    (align, justify) = match (single_word, para.justification.single_word) {
        (false, _) | (true, SingleWordJustify::FullJustify) => (align, justify),
        (true, SingleWordJustify::AlignLeft) => (0, false),
        (true, SingleWordJustify::AlignCenter) => (1, false),
        (true, SingleWordJustify::AlignRight) => (2, false),
    };
    // Insert Kashidas: the Arabic words of a horizontal line take its room on kashidas,
    // and the word spaces and gaps take what is left.
    let kashidas = if justify && !vertical && room > line.render_width + EPS {
        kashida::plan(text, sg, line.index..line.body, para.kashida, room - line.render_width)
    } else {
        Vec::new()
    };
    let width = line.render_width + kashidas.iter().map(|k| k.1).sum::<f64>();
    let (mut per_space, mut per_gap, mut per_cjk) = (0.0, 0.0, 0.0);
    let mut fit = LineFit::default();
    if justify && (room - width).abs() > EPS {
        if cjk_gaps > 0 && room > width {
            // Spread over the CJK gaps and the word spaces alike.
            per_space = (room - width) / (cjk_gaps + spaces) as f64;
            per_cjk = per_space;
        } else if spaces > 0 {
            // Word spacing, letter spacing and glyph scaling within their ranges, then the
            // word spaces alone; composed lines may shrink word spaces (never below zero).
            let flex: Vec<Flex> = (line.index..line.body).map(|j| flow_line_flex(j, &paragraph.flex, held, hung)).collect();
            let floor = (line.index..line.body).filter(|&j| j >= paragraph.marker && sg[j].is_space()).map(|j| sg[j].adv).fold(f64::MAX, f64::min);
            fit = fit_line(&flex, room - width, spaces, floor);
        } else if room > width {
            // One word: its letter spacing and glyph scaling, then the gaps between its glyphs.
            let flex: Vec<Flex> = (line.index..line.body).map(|j| flow_line_flex(j, &paragraph.flex, held, hung)).collect();
            fit = fit_line(&flex, room - width, 0, 0.0);
            let used: f64 = flex.iter().map(|f| fit.letter(f) + fit.glyph(f)).sum();
            let rest = room - width - used;
            let gaps = (line.index..line.body)
                .filter(|&j| {
                    !held.contains(&j) && drawn_after(line.index, j, &after).and_then(|next| sg.get(next)).is_some_and(|next| !next.continues_tcy())
                })
                .count();
            if gaps > 0 && rest > EPS {
                per_gap = rest / gaps as f64;
            }
        }
    }
    let position = &line.position;
    let start_x = if justify {
        line.ax0 - line.opt_l
    } else if paragraph.regions.is_none() {
        match align {
            0 => position.ind_l - line.opt_l,
            1 => (position.ind_l - line.opt_l - position.ind_r + line.opt_r - width) * 0.5,
            _ => -position.ind_r + line.opt_r - width,
        }
    } else {
        match align {
            0 => line.ax0 - line.opt_l,
            1 => line.ax0 - line.opt_l + (room - width) * 0.5,
            _ => line.ax1 + line.opt_r - width,
        }
    };
    LineJustification { per_space, per_gap, per_cjk, fit, start_x, after, kashidas }
}

fn emit_flow_line(cx: &mut Ctx<'_>, pen: &mut Pen<'_>, paragraph: &ParagraphData<'_, '_>, line: &LineGeometry, justification: &LineJustification) {
    let para = paragraph.para;
    let sg = &paragraph.sg;
    let index = line.index;
    let trimmed = line.trimmed;
    let body = line.body;
    let order = &line.order;
    let start_x = justification.start_x;
    let (ax0, ax1) = (line.ax0, line.ax1);
    let position = &line.position;
    let line_index = cx.out.lines.len();
    let glyph_start = cx.out.glyphs.len();
    // Character Alignment lines smaller characters up with the line's largest em box.
    let line_em = sg.get(index..position.end).map_or(0.0, |glyphs| glyphs.iter().map(glyph_em).fold(0.0, f64::max));
    // Tab stops are measured from the frame's left edge (point type: the origin).
    let tab_origin = if paragraph.regions.is_some() { position.x0 } else { 0.0 };
    // A line with tabs is measured as if it started at the left (see `set_width`), then
    // aligned as a whole: its tabs stop where they were measured, however it is aligned.
    let tab_shift = start_x - if paragraph.regions.is_some() { ax0 } else { position.ind_l };
    // RTL's logical trailing spaces precede the visible content. Keep them outside
    // the aligned content extent, just as LTR trailing spaces extend to its right.
    let leading_space_width: f64 =
        order.iter().take_while(|&&offset| index + offset >= trimmed).filter_map(|&offset| sg.get(index + offset)).map(|glyph| glyph.adv).sum();
    let mut x = start_x - leading_space_width;
    let mut x_end = start_x;
    // The glyphs drawn for the list marker, and the index of its tab's glyph, when the line starts the paragraph.
    let mut marker_glyphs: Option<Range<usize>> = None;
    let mut marker_tab = 0;
    for (order_index, &offset) in order.iter().enumerate() {
        let glyph_index = index + offset;
        let Some(glyph) = sg.get(glyph_index) else { continue };
        let mut advance = glyph.adv;
        if glyph_index + 1 == trimmed {
            advance -= line.end_trim;
        }
        // Glyph scaling a justified line adds to the glyph (1: none).
        let mut scale = 1.0;
        let mut leader = "";
        if glyph.ch == '\t' {
            // The tab after a list marker keeps the width `prepend_marker` gave it.
            if glyph.len > 0 {
                (advance, leader) = tab_advance(&para.tabs, tab_origin, x - tab_shift, &sg[glyph_index + 1..trimmed.max(glyph_index + 1)]);
            }
        } else if glyph_index < body {
            let flex = flow_line_flex(glyph_index, &paragraph.flex, &line.held, &line.hung);
            if is_word_space(glyph) {
                advance += justification.fit.space(&flex);
            } else {
                advance += justification.fit.letter(&flex) + justification.fit.glyph(&flex);
                scale = justification.fit.glyph_scale(&flex);
            }
            if glyph.is_space() {
                advance += justification.per_space;
            } else if justification.per_cjk != 0.0 {
                if is_cjk_gap(glyph_index, &line.held, &justification.after, index, sg) {
                    advance += justification.per_cjk;
                }
            } else if !line.held.contains(&glyph_index)
                && drawn_after(index, glyph_index, &justification.after).and_then(|next| sg.get(next)).is_some_and(|next| !next.continues_tcy())
            {
                // Between glyphs, never inside a tate-chu-yoko block (one cell).
                advance += justification.per_gap;
            }
        }
        // The kashida before a cluster in the text is drawn at the cluster's right, between
        // it and the letter before it, and the cluster's last glyph advances over it: the
        // caret, selection and hit testing take it as part of the cluster.
        let kashida = justification
            .kashidas
            .binary_search_by_key(&glyph.byte, |item| item.0)
            .ok()
            .and_then(|at| justification.kashidas.get(at))
            .filter(|_| order.get(order_index + 1).and_then(|&offset| sg.get(index + offset)).is_none_or(|next| next.byte != glyph.byte))
            .map(|item| item.1);
        advance += kashida.unwrap_or(0.0);
        let y = line.position.baseline - align_shift(glyph, cx.style_at(glyph.byte).char_align, line_em, cx.vertical);
        let transform = if scale != 1.0 { Affine::translate((x, y)) * Affine::scale_non_uniform(scale, 1.0) } else { Affine::translate((x, y)) };
        if glyph_index < paragraph.marker {
            let at = cx.out.glyphs.len();
            marker_glyphs = Some(marker_glyphs.map_or(at..at + 1, |range: Range<usize>| range.start.min(at)..range.end.max(at + 1)));
            if glyph_index + 1 == paragraph.marker {
                marker_tab = at;
            }
        }
        cx.emit(glyph, transform, Point::new(x, y), 0.0, advance, line_index);
        if let Some(out) = cx.out.glyphs.last_mut() {
            out.scale = paragraph.flex.get(glyph_index).map_or(1.0, |flex| flex.scale) * scale;
        }
        if !leader.is_empty() {
            // The leader belongs to the tab's glyph: the gap stays one character for the
            // caret, selection and hit testing.
            let dots = leader_outline(cx.db, glyph, leader, tab_origin, x, x + advance, y);
            if let Some(tab) = cx.out.glyphs.last_mut() {
                tab.outline.extend(dots);
            }
        }
        if let Some(length) = kashida {
            cx.emit_kashida(glyph, para.kashida, length, Point::new(x + advance - length, y), line_index);
        }
        x += advance;
        if glyph_index < trimmed {
            x_end = x;
        }
    }
    if let (Some(glyphs), Some(text)) = (marker_glyphs, paragraph.marker_text) {
        cx.out.markers.push(ListMarker { glyphs, tab: marker_tab, text: text.to_string() });
    }
    if let Some(hyphen) = &line.hyphen {
        // The hyphen follows the last non-space glyph.
        let x = x_end;
        cx.emit(hyphen, Affine::translate((x, position.baseline)), Point::new(x, position.baseline), 0.0, hyphen.adv, line_index);
        x_end = x + hyphen.adv;
    }
    cx.out.lines.push(LineInfo {
        rtl: paragraph.rtl && !cx.vertical,
        baseline: position.baseline,
        x0: start_x,
        x1: x_end,
        ascent: position.m.asc,
        descent: position.m.desc,
        first_baseline: if pen.model == LeadingModel::EmBoxTop {
            position.m.top.max(pen.fb_min)
        } else {
            position.m.first_baseline(pen.fb, pen.fb_min)
        },
        start: if index < sg.len() { sg[index].byte } else { paragraph.pr.end },
        end: if position.end >= sg.len() { paragraph.pr.end } else { sg[position.end].byte },
        glyph_start,
        glyph_end: cx.out.glyphs.len(),
        avail: if paragraph.regions.is_some() { (ax0, ax1) } else { (start_x, x_end) },
        region: if paragraph.regions.is_some() { pen.ri } else { 0 },
    });
    pen.settled(position.baseline, position.m);
    let forced_break = position.end > index && sg.get(position.end - 1).is_some_and(|g| g.ch == '\u{2028}');
    if forced_break {
        pen.queued.clear();
    } else {
        // Further spans of this line band share the settled baseline.
        for queued in &mut pen.queued {
            queued.0 = position.baseline;
        }
    }
}
