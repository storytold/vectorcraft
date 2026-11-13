use super::bidi::{is_rtl, para_bidi_with, visual_order};
use super::breaking::{
    Metrics, PUSH_IN_SQUEEZE, Pen, break_line, candidates, hangs, leader_outline, optical_hangs, optical_hangs_rtl, optical_punct, optical_share,
    roman_share, set_width, squeeze_em, tab_advance, word_width,
};
use super::region::Region;
use super::vertical::{
    EM_CENTER, MojikumiRules, align_shift, compress_punctuation, dot_quarter, glyph_em, halve_punctuation, punct_half, space_japanese_and_latin,
};
use super::{
    Affine, BidiInfo, Burasagari, CharStyle, Composer, Ctx, EPS, Flex, Justify, Kinsoku, KinsokuShoriType, LeadingModel, LineFit, LineInfo,
    ListMarker, ParaStyle, Params, Point, Punct, Range, SGlyph, SingleWordJustify, TextObject, apply_desired, cap_x_heights, compose, fit_line,
    hyphen_glyph, is_cjk, is_word_space, kashida, mark_word_ends, no_line_end, no_line_start, style_metrics,
};
use crate::justify::is_format_control;
use crate::shape::{keep_inseparable_pairs, keep_korean_words};

/// Does paragraph `para` push characters in (Kinsoku Shori Type Push In First or Push Out First,
/// with kinsoku on)?
fn pushes_in(para: &ParaStyle) -> bool {
    para.kinsoku != Kinsoku::None && para.kinsoku_type != KinsokuShoriType::PushOutOnly
}

/// Paragraphs longer than this (glyphs) are always broken line by line.
const MAX_COMPOSE_GLYPHS: usize = 200_000;

/// Every-line composition of paragraph glyphs `sg` if applicable (area text, justified or ragged,
/// with uniform line metrics or in rectangular cells without text wrap); `None` falls back to the
/// greedy single-line composer. With `continued`, the paragraph continues from the frame before,
/// and its first composed line is not its first line. `line_end` is how much narrower a line
/// ending with each glyph is set ([`ParagraphData::line_end`]).
#[allow(clippy::too_many_arguments)]
fn compose_para(
    cx: &Ctx<'_>,
    sg: &[SGlyph],
    flex: &[Flex],
    line_end: &[f64],
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
    // So is a paragraph that may push characters kinsoku keeps off a line's ends in onto a line.
    let may_push_in = pushes_in(para) && sg.iter().any(|g| no_line_start(g.ch) || no_line_end(g.ch));
    // A tab's width depends on where it falls in its line, which the composer's fixed glyph widths
    // can't follow: such a paragraph is broken line by line ([`break_line`] measures its tabs).
    // The tab of a list marker is on the first line, with the marker, and has a fixed width.
    let has_tab = sg.iter().any(|g| g.ch == '\t' && g.len > 0);
    let has_soft_break = sg.iter().any(|g| g.ch == '\u{2028}');
    if composer != Composer::EveryLine
        || pen.regions.is_none()
        || sg.len() < 2
        || sg.len() > MAX_COMPOSE_GLYPHS
        || may_hang
        || may_push_in
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
    // The same rule as the line loop below: an opening bracket starting a wrapped line gives up
    // the space before it.
    let start_credit = |i: usize| {
        let flush = !rtl && MojikumiRules::of(para.mojikumi).flush_start;
        sg.get(i).filter(|g| flush && g.lead <= 0.0).and_then(|g| punct_half(g, Punct::Opening)).unwrap_or(0.0)
    };
    let mut sim = pen.clone();
    let mut widths = vec![];
    let mut acc = 0.0;
    // A line's width and placement depend on the sum of its indents alone, which the mirrored
    // indents of a right-to-left list ([`line_indents`]) keep.
    while widths.len() < sg.len() {
        let first = widths.is_empty();
        let ind_l = para.laid_out_left_indent() + if first && !continued { para.first_line_indent } else { 0.0 };
        // Only the first line's first word is known here. Later lines are placed as the composed
        // lines need: each fits its width, so its first word does too. It is measured as the line
        // loop measures it, with Mojikumi's narrower start and end.
        let word = if first { word_width(sg, 0, line_end, para.kinsoku) - start_credit(0) } else { 0.0 };
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
    // A line's last character is set narrower, as the line loop sets it.
    let end_credit = |end: usize| end.checked_sub(1).and_then(|k| line_end.get(k)).copied().unwrap_or(0.0);
    let pass = |p: &Params| compose(sg, &width, &start_credit, &end_credit, &cands, flex, p);
    pass(&params(strict)).or_else(|| pass(&params(loose)))
}

/// The list marker of each of the paragraphs `paras` of `t` (None for none): bullets, and numbers
/// counted per level through consecutive list paragraphs from [`TextObject::list_counts`], as
/// [`vectorcraft_doc::text::ParaList::next_marker`] counts them.
pub(super) fn list_markers(t: &TextObject, paras: &[Range<usize>]) -> Vec<Option<String>> {
    let mut counts = t.list_counts();
    paras.iter().enumerate().map(|(pi, pr)| vectorcraft_doc::text::ParaList::next_marker(t.para_at(pi).list, pr.is_empty(), &mut counts)).collect()
}

/// Whether paragraph `para` (right to left when `rtl`) has its indents mirrored: a horizontal
/// right-to-left list paragraph does.
fn mirrors_indents(para: &ParaStyle, rtl: bool, vertical: bool) -> bool {
    rtl && !vertical && para.list.is_some()
}

/// The indents at the left and at the right of a line of paragraph `para` (its first line when
/// `first`). With `mirror` ([`mirrors_indents`]) the left and first-line indents are measured from
/// the right and the right indent from the left: the bullet of a right-to-left list hangs at the
/// right, and the lines it wraps to line up with the text after the bullet. A list level past the
/// first adds to the left indent ([`ParaStyle::laid_out_left_indent`]), mirrored with it.
fn line_indents(para: &ParaStyle, first: bool, mirror: bool) -> (f64, f64) {
    let start = para.laid_out_left_indent() + if first { para.first_line_indent } else { 0.0 };
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
        steps_left: std::rc::Rc::new(std::cell::Cell::new(super::breaking::MAX_TOTAL_STEPS)),
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
    /// How much narrower a line ending with each glyph is set ([`line_end_trim`]), for measuring
    /// where lines break; empty when the paragraph's lines are measured at full width (Mojikumi
    /// None or YakumonoZenkaku, or right-to-left text with a set that adds no space between
    /// Japanese and Latin text).
    line_end: Vec<f64>,
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
    // Korean breaks between words, unless Korean Line Break Anywhere lets it break between syllables.
    if !para.korean_line_break_anywhere {
        keep_korean_words(&mut sg);
    }
    // Bunri-Kinshi keeps inseparable pairs on one line, while kinsoku is on.
    if para.bunri_kinshi && para.kinsoku != Kinsoku::None {
        keep_inseparable_pairs(text, &mut sg);
    }
    // Japanese composition (the paragraph's Mojikumi set): every punctuation mark set half width,
    // or consecutive punctuation sharing one half-em space…
    let rules = MojikumiRules::of(para.mojikumi);
    let mut compressed = if rules.half {
        halve_punctuation(&mut sg)
    } else if rules.share {
        compress_punctuation(&mut sg)
    } else {
        vec![]
    };
    // …and a quarter em between Japanese and Latin text.
    let mut wakan = if rules.wakan { space_japanese_and_latin(&mut sg) } else { vec![] };
    // Justification…: the glyphs at Desired word and letter spacing and glyph scaling, and how
    // far a justified line may move each one from it (Dynamic Text's Block on area type sets the
    // Desired word spacing to its word gap).
    let mut flex = apply_desired(&mut sg, &t.layout_justification(para), !cx.vertical);
    // The Japanese–Latin space is part of the advance Desired glyph scaling scaled.
    for (w, f) in wakan.iter_mut().zip(&flex) {
        *w *= f.scale;
    }
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
        let face = cx.face_at(pr.start);
        let (asc, desc, _) = style_metrics(face.as_deref(), cx.style_at(pr.start));
        let lead = auto_leading(cx.style_at(pr.start));
        let style = cx.style_at(pr.start);
        let (cap, xh) = cap_x_heights(face.as_deref(), style);
        let centre = face.map_or(EM_CENTER, |face| face.ideographic_centre());
        Metrics { asc, desc, lead, cap, xh, top: (centre + 0.5) * style.size * style.v_scale / 100.0 }
    };
    if pi > 0 {
        pen.pending += para.space_before;
    }
    // A line fits when it fits as it is set: with its last character narrowed by Mojikumi.
    let end_rules = rules.at_line_end(rtl);
    let line_end =
        if end_rules.trims_line_end() { (0..sg.len()).map(|k| line_end_trim(&sg, &compressed, &wakan, end_rules, k)).collect() } else { vec![] };
    let composed = compose_para(cx, &sg, &flex, &line_end, para, pen, rtl, continued);
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
        line_end,
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
    /// [`ParagraphData::line_end`].
    line_end: &'a [f64],
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
    /// Optical Margin Alignment or Roman Hanging Punctuation is on.
    hanging: bool,
}

struct LinePosition {
    baseline: f64,
    x0: f64,
    x1: f64,
    end: usize,
    hyph: bool,
    /// Kinsoku Shori Type pushed characters in onto the line ([`break_line`]).
    pushed_in: bool,
    m: Metrics,
    ind_l: f64,
    ind_r: f64,
}

fn place_flow_line(cx: &mut Ctx<'_>, pen: &mut Pen<'_>, context: &mut PlacementContext<'_>) -> Option<LinePosition> {
    let PlacementContext {
        para,
        pr,
        sg,
        line_end,
        continued,
        rtl,
        mirror,
        pm,
        composed,
        hyphen_run,
        hyphen_limit,
        zone,
        index: i,
        para_line,
        hanging,
    } = context;
    let n = sg.len();
    let est = if *i < n { Metrics::of(&sg[*i]) } else { *pm };
    let first_line = *para_line == 0 && !*continued;
    let (ind_l, ind_r) = line_indents(para, first_line, *mirror);
    // Mojikumi: an opening bracket starting a line is set flush with the line's start (the
    // space before it goes), and the line has that much more room. At a paragraph's start
    // that is the first-line indent: JIS X 4051's principle (JLREQ 3.1.5, Figure 71 ①).
    if !*rtl
        && MojikumiRules::of(para.mojikumi).flush_start
        && let Some(glyph) = sg.get_mut(*i)
        && glyph.lead <= 0.0
        && let Some(half) = punct_half(glyph, Punct::Opening)
    {
        glyph.adv -= half;
        glyph.lead += half;
    }
    // The line's first word, as narrow as the line sets it at both ends.
    let word = word_width(sg, *i, line_end, para.kinsoku);
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
        let (end, hyph, pushed_in) = match composed.as_ref().and_then(|lines| lines.get(*para_line)) {
            Some(&(end, hyph)) if end > *i => (end, hyph, false),
            _ if *i < n => {
                let line = |ty| {
                    break_line(
                        cx.text,
                        sg,
                        *i,
                        width,
                        &para.tabs,
                        ind_l,
                        hyphenate,
                        para.burasagari,
                        para.kinsoku,
                        ty,
                        *hanging,
                        cx.vertical,
                        line_end,
                    )
                };
                let (end, hyph) = line(para.kinsoku_type);
                // The line pushed characters in where it ends after the line that pushes out.
                let pushed_in = pushes_in(para) && end > line(KinsokuShoriType::PushOutOnly).0;
                (end, hyph, pushed_in)
            }
            _ => (n, false, false),
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
        return Some(LinePosition { baseline, x0, x1, end, hyph, pushed_in, m, ind_l, ind_r });
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
    /// The tracking after the line's last character, when that character ends the line (see
    /// [`prepare_line_geometry`]); 0 otherwise.
    trailing: f64,
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
    /// Kinsoku Shori Type: the share of an em (of the glyph before it, at most
    /// [`PUSH_IN_SQUEEZE`]) each gap next to a CJK character closes by on a line that pushed
    /// characters in; 0 on other lines.
    squeeze: f64,
    fit: LineFit,
    start_x: f64,
    after: Vec<Option<usize>>,
    kashidas: Vec<(usize, f64)>,
    /// Right-aligned area type: the line's last tab, which takes the text after it to the right
    /// edge (the text before it sits at its stops, from the left edge).
    last_tab: Option<usize>,
    /// The tracking after the line's last character that its alignment leaves out of its width:
    /// [`LineGeometry::trailing`] for a line that isn't justified, 0 for one that is.
    trailing: f64,
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
            line_end: &paragraph.line_end,
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
            hanging: t.optical_margin || paragraph.para.roman_hanging,
        };
        let Some(position) = place_flow_line(cx, pen, &mut placement) else { return false };
        hyphen_run = if position.hyph && position.end > index { hyphen_run + 1 } else { 0 };
        pen.pending = 0.0;
        let geometry = prepare_line_geometry(cx, t, &paragraph, position, index);
        let justification = justify_flow_line(cx, &paragraph, &geometry);
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

/// Mojikumi: how much narrower a line ending with glyph `k` of `sg` is set by the paragraph's set
/// `rules`. With [`MojikumiRules::half_end`], a closing bracket, comma or full stop ending the line
/// is set half width, and with [`MojikumiRules::dot_end`] a middle dot loses the quarter em after
/// it, unless the glyph already lost that space (`compressed`); no Japanese–Latin space (`wakan`)
/// is left after the line's last character. Never more than the glyph's advance, and 0 for an
/// advance or a space that is not finite.
pub(super) fn line_end_trim(sg: &[SGlyph], compressed: &[bool], wakan: &[f64], rules: MojikumiRules, k: usize) -> f64 {
    let Some(g) = sg.get(k) else { return 0.0 };
    let lost = compressed.get(k).copied().unwrap_or(false);
    let half = (!lost && rules.half_end).then(|| punct_half(g, Punct::Closing)).flatten();
    let half = half.or_else(|| (!lost && rules.dot_end).then(|| dot_quarter(g)).flatten());
    let trim = half.unwrap_or(0.0) + wakan.get(k).copied().unwrap_or(0.0);
    if trim.is_finite() && g.adv.is_finite() { trim.min(g.adv).max(0.0) } else { 0.0 }
}

/// The geometry of the line of `paragraph` from glyph `index`, placed at `position`: where it
/// ends with its trailing spaces trimmed, its hyphen, its room and margins, the glyphs that hang or
/// stay out of justification, and whether it is the paragraph's last line.
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
    let rules = MojikumiRules::of(para.mojikumi).at_line_end(paragraph.rtl);
    let end_trim = trimmed
        .checked_sub(1)
        .filter(|&k| k >= index && rules.trims_line_end())
        .map_or(0.0, |k| line_end_trim(sg, &paragraph.compressed, &paragraph.wakan, rules, k));
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
    // A line a soft return ends is justified as a wrapped one is: only the paragraph's last line
    // keeps the alignment of Justify Left, Center or Right. A paragraph that ends in a soft return
    // ends with the empty line after it (`run_paragraph_lines`).
    let soft_return = position.end > index && sg.get(position.end - 1).is_some_and(|g| g.ch == '\u{2028}');
    let last_of_para = position.end >= n && !soft_return;
    // The tracking after the line's last character: an aligned line is measured without it. The
    // page of users' `.ai` files sets a centred line of tracked type half that tracking right of
    // where a line measured with it goes, and a right-aligned one the whole tracking right (in
    // lines that end a paragraph). The last character is the last one the line draws before a
    // paragraph's end, a forced line break, or the spaces the layout wraps the line at (they hang
    // past it). Spaces or a tab typed before a paragraph's end or a forced line break are the
    // line's last characters, and the letter before them keeps its tracking: the page counts it in
    // the 2 lines of the files that end so. Forced line breaks and wraps aren't in the files; they
    // take the rule so that turning a wrap into a forced line break or a new paragraph (Break
    // Text, Convert To Point Type) keeps the line where it was. A line a hyphen ends, one whose
    // last character hangs, and vertical and right-to-left lines keep the tracking.
    let tail = sg.get(trimmed..position.end).unwrap_or_default();
    let ends_drawn = match tail.split_last() {
        None => true,
        Some((last, before)) if last.ch == '\u{2028}' => before.is_empty(),
        // Spaces at a wrap: the paragraph goes on.
        Some(_) => position.end < n,
    };
    let trailing = trimmed
        .checked_sub(1)
        .and_then(|k| sg.get(k))
        .filter(|g| {
            ends_drawn && trimmed > index && !g.level.is_rtl() && !cx.vertical && !paragraph.rtl && hyphen.is_none() && hang.is_none() && opt_r == 0.0
        })
        .map_or(0.0, |g| if g.tracking.is_finite() { g.tracking } else { 0.0 });
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
        trailing,
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

// The next drawn glyph after `glyph` that is not a zero-width format control.
fn visible_after(index: usize, glyph: usize, after: &[Option<usize>], sg: &[SGlyph]) -> Option<usize> {
    let mut next = drawn_after(index, glyph, after);
    for _ in 0..after.len() {
        match next {
            Some(n) if sg.get(n).is_some_and(|g| is_format_control(g.ch)) => next = drawn_after(index, n, after),
            _ => break,
        }
    }
    next
}

// The glyph after which a gap between drawn glyphs opens, and the visible glyph that follows it:
// not a held glyph, and never a zero-width format control on either side.
fn gap_pair<'a>(glyph: usize, held: &[usize], after: &[Option<usize>], index: usize, sg: &'a [SGlyph]) -> Option<(&'a SGlyph, &'a SGlyph)> {
    if held.contains(&glyph) {
        return None;
    }
    let a = sg.get(glyph).filter(|a| !is_format_control(a.ch))?;
    visible_after(index, glyph, after, sg).and_then(|next| sg.get(next)).map(|b| (a, b))
}

// Japanese (and Chinese) lines are justified between their characters (JLREQ 3.8): the
// gaps next to a CJK character, not inside a Latin word, a tate-chu-yoko block or a shaped
// cluster.
fn is_cjk_gap(glyph: usize, held: &[usize], after: &[Option<usize>], index: usize, sg: &[SGlyph]) -> bool {
    gap_pair(glyph, held, after, index, sg).is_some_and(|(a, b)| {
        a.byte != b.byte && !a.is_space() && !b.is_space() && !b.continues_tcy() && a.ch != '\t' && (is_cjk(a.ch) || is_cjk(b.ch))
    }) && !in_pair(glyph, index, after, sg)
}

// Bunri-Kinshi: are glyph `glyph` and the glyph drawn after it in the two halves of an inseparable
// pair? Justification adds no space between them. The halves are neighboring clusters: the
// earlier one's last glyph keeps the next, and the later one starts right after it. Right to left,
// the clusters are drawn in the other order, each with its glyphs in logical order, so the two
// drawn neighbors need not be neighbors in `sg`.
fn in_pair(glyph: usize, index: usize, after: &[Option<usize>], sg: &[SGlyph]) -> bool {
    let Some(next) = drawn_after(index, glyph, after) else {
        return false;
    };
    let (Some(a), Some(b)) = (sg.get(glyph), sg.get(next)) else {
        return false;
    };
    if a.byte == b.byte {
        return false;
    }
    let (first, later) = if a.byte < b.byte { (glyph, b.byte) } else { (next, a.byte) };
    let byte = sg.get(first).map_or(0, |g| g.byte);
    let last = first + sg.iter().skip(first + 1).take_while(|g| g.byte == byte).count();
    sg.get(last).is_some_and(|g| g.keeps_next) && sg.get(last + 1).is_some_and(|g| g.byte == later)
}

// Single Word Justification's Full Justify spreads a line's leftover room over the gaps between
// its drawn glyphs: not after a held glyph, nor inside a tate-chu-yoko block (one cell), a shaped
// cluster, whose glyphs (a letter and its combining marks, a conjunct and its matra) stay
// together, or an inseparable pair. On a line that ends in a drawn hyphen, the gap before the
// hyphen counts too: it follows the line's last visible glyph.
fn is_letter_gap(glyph: usize, held: &[usize], after: &[Option<usize>], index: usize, sg: &[SGlyph], hyphen: bool) -> bool {
    if held.contains(&glyph) || sg.get(glyph).is_none_or(|a| is_format_control(a.ch)) || in_pair(glyph, index, after, sg) {
        return false;
    }
    match gap_pair(glyph, held, after, index, sg) {
        Some((a, b)) => a.byte != b.byte && !b.continues_tcy(),
        None => hyphen,
    }
}

// A space that takes a share of a justified line's room: not a zero-width space, which is a line
// break opportunity but not a word separator.
fn is_justified_space(g: &SGlyph) -> bool {
    g.is_space() && !is_format_control(g.ch)
}

// May a pushed-in line close a CJK gap after `g`? Not after a list marker or an inline graphic,
// which the push-in's room does not count either.
fn may_squeeze(g: &SGlyph) -> bool {
    g.len > 0 && g.inline.is_none()
}

// How much a pushed-in line's `squeeze` adds to the CJK gap after `g`.
fn squeeze_after(g: &SGlyph, squeeze: f64, vertical: bool) -> f64 {
    if may_squeeze(g) { squeeze * squeeze_em(g, vertical) } else { 0.0 }
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

/// How `line` of `paragraph` is aligned: where it starts, what justification adds to its spaces,
/// gaps and glyphs, its kashidas, and, in right-aligned area type, its last tab, which takes the
/// text after it to the right edge.
fn justify_flow_line(cx: &Ctx<'_>, paragraph: &ParagraphData<'_, '_>, line: &LineGeometry) -> LineJustification {
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
    let spaces = (line.index..line.body).filter(|&j| j >= paragraph.marker && is_justified_space(&sg[j])).count();
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
    let kashidas = if justify && !cx.vertical && room > line.render_width + EPS {
        kashida::plan(cx.text, sg, line.index..line.body, para.kashida, room - line.render_width, |byte| cx.style_at(byte).kashidas)
    } else {
        Vec::new()
    };
    let width = line.render_width + kashidas.iter().map(|k| k.1).sum::<f64>();
    let (mut per_space, mut per_gap, mut per_cjk) = (0.0, 0.0, 0.0);
    let mut fit = LineFit::default();
    let mut squeeze = 0.0;
    // Kinsoku Shori Type: a line of area type that pushed characters in is wider than its
    // measure, and closes its gaps next to CJK characters to fit, however it is aligned: each by
    // the same share of the em of the glyph before it, at most [`PUSH_IN_SQUEEZE`].
    let pushed_in = line.position.pushed_in && paragraph.regions.is_some() && cjk_gaps > 0 && width > room + EPS;
    if pushed_in {
        let ems: f64 = (line.index..line.body)
            .filter(|&j| is_cjk_gap(j, held, &after, line.index, sg) && may_squeeze(&sg[j]))
            .map(|j| squeeze_em(&sg[j], cx.vertical))
            .sum();
        if ems > EPS {
            squeeze = ((room - width) / ems).max(-PUSH_IN_SQUEEZE);
        }
    } else if justify && (room - width).abs() > EPS {
        if cjk_gaps > 0 && room > width {
            // Spread over the CJK gaps and the word spaces alike.
            per_space = (room - width) / (cjk_gaps + spaces) as f64;
            per_cjk = per_space;
        } else if spaces > 0 {
            // Word spacing, letter spacing and glyph scaling within their ranges, then the
            // word spaces alone; composed lines may shrink word spaces (never below zero).
            let flex: Vec<Flex> = (line.index..line.body).map(|j| flow_line_flex(j, &paragraph.flex, held, hung)).collect();
            let floor =
                (line.index..line.body).filter(|&j| j >= paragraph.marker && is_justified_space(&sg[j])).map(|j| sg[j].adv).fold(f64::MAX, f64::min);
            fit = fit_line(&flex, room - width, spaces, floor);
        } else if room > width {
            // One word: its letter spacing and glyph scaling, then the gaps between its glyphs.
            let flex: Vec<Flex> = (line.index..line.body).map(|j| flow_line_flex(j, &paragraph.flex, held, hung)).collect();
            fit = fit_line(&flex, room - width, 0, 0.0);
            let used: f64 = flex.iter().map(|f| fit.letter(f) + fit.glyph(f)).sum();
            let rest = room - width - used;
            let hyphen = line.hyphen.is_some();
            let gaps = (line.index..line.body).filter(|&j| is_letter_gap(j, held, &after, line.index, sg, hyphen)).count();
            if gaps > 0 && rest > EPS {
                per_gap = rest / gaps as f64;
            }
        }
    }
    let position = &line.position;
    // Right-aligned area type with a tab: the text before the last tab sits at its stops, from the
    // left edge, and the text after it against the right edge, as Illustrator's own layout in a
    // user's `.ai` sets a price list. Centred and justified lines are aligned as a whole.
    let last_tab = (para.justify == Justify::Right && !justify && !paragraph.rtl && paragraph.regions.is_some())
        // A tab ending the line counts: its text sits at the left edge, the tab up to the right one.
        .then(|| (line.index..line.position.end).rev().find(|&j| sg.get(j).is_some_and(|g| g.ch == '\t' && g.len > 0)))
        .flatten();
    // An aligned line is as wide as its characters without the tracking after the last one.
    let trailing = if justify || pushed_in { 0.0 } else { line.trailing };
    let width = width - trailing;
    let start_x = if justify || pushed_in || last_tab.is_some() {
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
    LineJustification { per_space, per_gap, per_cjk, squeeze, fit, start_x, after, kashidas, last_tab, trailing }
}

/// The advance and leader of tab `glyph_index` of `line`, at pen position `x`: up to its stop
/// ([`tab_advance`], measured from `tab_origin` less `tab_shift`), or, for the last tab of
/// right-aligned area type ([`LineJustification::last_tab`]), up to where the rest of the line
/// ends on the right edge. The tab after a list marker keeps the width `prepend_marker` gave it
/// (`advance`).
fn tab_glyph<'p>(
    paragraph: &'p ParagraphData<'_, '_>,
    line: &LineGeometry,
    justification: &LineJustification,
    glyph_index: usize,
    x: f64,
    (tab_origin, tab_shift): (f64, f64),
    advance: f64,
) -> (f64, &'p str) {
    let sg = &paragraph.sg;
    // The tab of a list marker keeps the width `prepend_marker` gave it.
    if !sg.get(glyph_index).is_some_and(|g| g.len > 0) {
        return (advance, "");
    }
    let (advance, leader) = tab_advance(&paragraph.para.tabs, tab_origin, x - tab_shift, sg.get(glyph_index + 1..line.trimmed).unwrap_or_default());
    if justification.last_tab != Some(glyph_index) {
        return (advance, leader);
    }
    // The width of what follows up to the line's end (nothing, when the tab ends the line), with
    // the hyphen a hyphenated line ends with, and without the tracking after its last character,
    // as a right-aligned line is measured.
    let after = sg.get(glyph_index + 1..line.trimmed).unwrap_or_default();
    let hyphen = line.hyphen.as_ref().map_or(0.0, |g| g.adv);
    let rest = if after.is_empty() { 0.0 } else { after.iter().map(|g| g.adv).sum::<f64>() - line.end_trim + hyphen - justification.trailing };
    ((line.ax1 + line.opt_r - rest - x).max(0.0), leader)
}

/// Lay out `line` of `paragraph` as `justification` aligns it: each glyph placed along the
/// baseline (tabs as [`tab_glyph`] sets them), its hyphen, and the line's record, then the pen
/// moved on past it.
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
            (advance, leader) = tab_glyph(paragraph, line, justification, glyph_index, x, (tab_origin, tab_shift), advance);
        } else if glyph_index < body {
            let flex = flow_line_flex(glyph_index, &paragraph.flex, &line.held, &line.hung);
            if is_word_space(glyph) {
                advance += justification.fit.space(&flex);
            } else {
                advance += justification.fit.letter(&flex) + justification.fit.glyph(&flex);
                scale = justification.fit.glyph_scale(&flex);
            }
            if is_justified_space(glyph) {
                advance += justification.per_space;
            } else if justification.per_cjk != 0.0 || justification.squeeze != 0.0 {
                if is_cjk_gap(glyph_index, &line.held, &justification.after, index, sg) {
                    advance += justification.per_cjk + squeeze_after(glyph, justification.squeeze, cx.vertical);
                }
            } else if is_letter_gap(glyph_index, &line.held, &justification.after, index, sg, line.hyphen.is_some()) {
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
        // A glyph scaled to fill the line keeps its Insert Aki before it at its width.
        let aki = if scale != 1.0 && glyph.aki_shift.is_finite() { glyph.aki_shift * (1.0 - scale) } else { 0.0 };
        let transform =
            if scale != 1.0 { Affine::translate((x + aki, y)) * Affine::scale_non_uniform(scale, 1.0) } else { Affine::translate((x, y)) };
        if glyph_index < paragraph.marker {
            let at = cx.out.glyphs.len();
            marker_glyphs = Some(marker_glyphs.map_or(at..at + 1, |range: Range<usize>| range.start.min(at)..range.end.max(at + 1)));
            if glyph.ch == '\t' {
                marker_tab = at;
            }
        }
        cx.emit(glyph, transform, Point::new(x, y), 0.0, advance, line_index);
        // Its own width starts where the glyph is drawn, with the Aki kept at its width.
        let own_shift = cx.own_width_shift(glyph, aki);
        if let Some(out) = cx.out.glyphs.last_mut() {
            out.scale = paragraph.flex.get(glyph_index).map_or(1.0, |flex| flex.scale) * scale;
            out.width_start += own_shift;
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
        trailing: justification.trailing,
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
