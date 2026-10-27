//! Shaping: text runs -> positioned glyphs in points (before line breaking).

use std::ops::Range;
use std::sync::Arc;

use unicode_bidi::Level;
use unicode_script::{Script, UnicodeScript};

use harfrust::{Direction, Feature, ShapeOptions, Tag, UnicodeBuffer};
use skrifa::MetadataProvider;
use skrifa::instance::Size;
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{CharStyle, InlineArt};

use crate::features::{OtFeatures, ligatures_suppressed_by};
use crate::fontdb::{FontDb, FontFace};

/// A shaped glyph with all character-style effects resolved, in points (y down).
#[derive(Clone, Debug)]
pub(crate) struct SGlyph {
    pub face: Arc<FontFace>,
    pub gid: u32,
    /// Byte offset of the cluster in the plain text, and its length in bytes.
    pub byte: usize,
    pub len: usize,
    pub run: usize,
    /// Advance in points (tracking, manual kerning and horizontal scale applied).
    pub adv: f64,
    /// Offset from the pen position, y down.
    pub dx: f64,
    pub dy: f64,
    /// Outline scale (font units -> points).
    pub sx: f64,
    pub sy: f64,
    /// Baseline shift in points (positive = up).
    pub bshift: f64,
    /// Character rotation in degrees (counter-clockwise).
    pub rotation: f64,
    pub ascent: f64,
    pub descent: f64,
    pub leading: f64,
    /// Cap height and x height in points.
    pub cap: f64,
    pub xh: f64,
    /// First source character of the cluster.
    pub ch: char,
    /// Vertical type: part of a tate-chu-yoko block (set across the column, upright).
    pub tcy: Option<Tcy>,
    /// Japanese composition: the space taken off before the glyph (an opening bracket after
    /// another, see [`crate::layout`]); the glyph is drawn that much earlier on the line.
    pub lead: f64,
    /// Proportional Metrics changed the glyph's advance (`palt`): it has its proportional width,
    /// and Japanese composition takes no more space off it (#966).
    pub proportional: bool,
    /// Vertical type with Proportional Metrics: what `vpal` does to the glyph when it stands
    /// upright ([`crate::layout`] applies it).
    pub vpal: Option<VAdjust>,
    /// Resolved Unicode bidi embedding level (logical source order).
    pub level: Level,
    /// An inline graphic ([`vectorcraft_doc::TextRun::inline`]) standing in for a glyph.
    pub inline: Option<InlineBox>,
    /// A word of a script written without spaces (Thai, Lao, Khmer, Myanmar) ends after the
    /// glyph's cluster, so a line may break there ([`mark_word_ends`]).
    pub word_end: bool,
    /// No Break (Character panel menu): keep this glyph on the same line as the neighbouring
    /// glyphs that have it.
    pub no_break: bool,
    /// The glyph is of a script whose letters join (Arabic, Syriac, N'Ko, Mongolian, Adlam…),
    /// decided for the whole segment it was shaped in ([`joins`]): on a curved path it is bent
    /// along the path so its joins meet.
    pub cursive: bool,
}

/// Does `script` join its letters (a cursive script, whose neighbouring glyphs touch)?
pub(crate) fn joins(script: Script) -> bool {
    matches!(
        script,
        Script::Arabic
            | Script::Syriac
            | Script::Nko
            | Script::Mongolian
            | Script::Adlam
            | Script::Mandaic
            | Script::Manichaean
            | Script::Psalter_Pahlavi
            | Script::Hanifi_Rohingya
            | Script::Sogdian
            | Script::Phags_Pa
    )
}

/// How `vpal` and the vertical kerning with it move an upright glyph in vertical type and change
/// its length down the column, in points: `advance` down the column, `dx` and `dy` in glyph space
/// (y down; `dy` is down the column once the glyph stands upright). `proportional`: `vpal` itself
/// changed the advance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct VAdjust {
    pub advance: f64,
    pub dx: f64,
    pub dy: f64,
    pub proportional: bool,
}

/// Placement of an inline graphic's art, glyph space being points with the pen at the origin on
/// the baseline (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InlineBox {
    /// The art's bounds in symbol space; `None` when the symbol is missing (the box only takes
    /// up room).
    pub art: Option<kurbo::Rect>,
    /// Art -> glyph space.
    pub xf: kurbo::Affine,
}

/// A glyph's place in a tate-chu-yoko block: the block takes one em of the column, its glyphs side
/// by side across it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tcy {
    /// Laid-out distance from the block's start to this glyph's pen position.
    pub pen: f64,
    /// Distance from the block's start to the glyph in the block's own (horizontal) setting.
    pub ink: f64,
    /// Width of the block's own setting.
    pub width: f64,
    /// Horizontal scale that fits the block into one em (1 when it fits as it is).
    pub squeeze: f64,
}

impl SGlyph {
    pub fn is_space(&self) -> bool {
        self.inline.is_none() && matches!(self.ch, ' ' | '\t' | '\u{3000}' | '\u{2002}'..='\u{200B}')
    }
    /// A line may break after this glyph.
    pub fn break_after(&self) -> bool {
        self.is_space() || self.word_end || matches!(self.ch, '-' | '\u{2010}' | '\u{2013}' | '\u{2014}' | '/' | SOFT_HYPHEN) || is_cjk(self.ch)
    }
    /// A soft (discretionary) hyphen: invisible unless a line breaks after it.
    pub fn is_soft_hyphen(&self) -> bool {
        self.ch == SOFT_HYPHEN
    }
    /// Part of a word that may be hyphenated (letters and apostrophes).
    pub fn is_letter(&self) -> bool {
        self.ch.is_alphabetic() || matches!(self.ch, '\'' | '’')
    }
    /// Vertical type: a tate-chu-yoko glyph after its block's first, sharing that glyph's cell.
    pub fn continues_tcy(&self) -> bool {
        self.tcy.is_some_and(|t| t.pen > 0.0)
    }
}

pub(crate) const SOFT_HYPHEN: char = '\u{00AD}';

/// A visible hyphen in the face and size of `g`, placed at the end of `g`'s cluster (zero source
/// length) for a line broken inside a word.
pub(crate) fn hyphen_glyph(g: &SGlyph) -> SGlyph {
    let gid = ['-', '\u{2010}', SOFT_HYPHEN].into_iter().map(|c| g.face.glyph_for(c)).find(|&id| id != 0).unwrap_or(0);
    let mut h = g.clone();
    h.gid = gid;
    h.adv = g.face.advance(gid) * g.sx;
    h.dx = 0.0;
    h.dy = 0.0;
    h.byte = g.byte + g.len;
    h.len = 0;
    h.ch = '-';
    h.inline = None;
    h.proportional = false;
    h.vpal = None;
    h
}

/// Kinsoku (Japanese line breaking, the strict set): a character that can't start a line —
/// closing brackets, the Japanese comma and full stop, middle dots, colons, ! and ?, the long
/// vowel mark, iteration marks and small kana.
pub(crate) fn no_line_start(c: char) -> bool {
    matches!(c,
        ')' | ']' | '}' | ',' | '.' | ':' | ';' | '!' | '?' | '»' | '’' | '”' | '‐' | '–' | '‼' | '⁇' | '⁈' | '⁉'
        | '、' | '。' | '〉' | '》' | '」' | '』' | '】' | '〕' | '〗' | '〙' | '〛' | '〟' | '〜' | '゠' | '・' | '｠'
        | 'ー' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '々' | '〻'
        | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ' | 'ㇰ'..='ㇿ'
        | '！' | '）' | '，' | '．' | '：' | '；' | '？' | '］' | '｝' | '～' | '｡' | '｣' | '､' | '･' | 'ｰ' | 'ｧ'..='ｯ')
}

/// Soft kinsoku: a character of [`no_line_start`] that may start a line all the same: 々, the
/// prolonged sound mark ー (JLREQ cl-10) and small kana (cl-11), as JLREQ's level 3 rules allow
/// (Appendix C.3, https://www.w3.org/TR/jlreq/#addendum_a3).
pub(crate) fn soft_line_start(c: char) -> bool {
    matches!(
        c,
        '々' | 'ー'
            | '\u{3041}'
            | '\u{3043}'
            | '\u{3045}'
            | '\u{3047}'
            | '\u{3049}'
            | '\u{3063}'
            | '\u{3083}'
            | '\u{3085}'
            | '\u{3087}'
            | '\u{308E}'
            | '\u{3095}'
            | '\u{3096}'
            | '\u{30A1}'
            | '\u{30A3}'
            | '\u{30A5}'
            | '\u{30A7}'
            | '\u{30A9}'
            | '\u{30C3}'
            | '\u{30E3}'
            | '\u{30E5}'
            | '\u{30E7}'
            | '\u{30EE}'
            | '\u{30F5}'
            | '\u{30F6}'
            | '\u{31F0}'..='\u{31FF}'
    )
}

/// Kinsoku: a character that can't end a line (opening brackets).
pub(crate) fn no_line_end(c: char) -> bool {
    matches!(
        c,
        '(' | '['
            | '{'
            | '«'
            | '‘'
            | '“'
            | '〈'
            | '《'
            | '「'
            | '『'
            | '【'
            | '〔'
            | '〖'
            | '〘'
            | '〚'
            | '〝'
            | '（'
            | '［'
            | '｛'
            | '｟'
            | '｢'
    )
}

/// Full-width punctuation for Japanese composition (JLREQ cl-01, cl-02, cl-06, cl-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Punct {
    /// An opening bracket: its half-em space is before the mark.
    Opening,
    /// A closing bracket, comma or full stop: its half-em space is after the mark.
    Closing,
}

pub(crate) fn punct(c: char) -> Option<Punct> {
    match c {
        '（' | '「' | '『' | '【' | '〔' | '〈' | '《' | '［' | '｛' | '〘' | '〖' | '｟' | '〝' => Some(Punct::Opening),
        '）' | '」' | '』' | '】' | '〕' | '〉' | '》' | '］' | '｝' | '〙' | '〗' | '｠' | '〟' | '、' | '。' | '，' | '．' => {
            Some(Punct::Closing)
        }
        _ => None,
    }
}

/// Is `c` of a script written without spaces between words, whose line breaks need a model of its
/// words: Thai, Lao, Khmer or Myanmar (UAX #14's class SA)?
pub(crate) fn is_unspaced_script(c: char) -> bool {
    matches!(c.script(), Script::Thai | Script::Lao | Script::Khmer | Script::Myanmar)
}

/// The byte offsets of `text` where a word of Thai, Lao, Khmer or Myanmar starts or ends: where a
/// line may break, and where the caret stops word by word, in text without spaces between words.
/// They come from ICU4X's line segmenter (its dictionaries on the desktop, its far smaller LSTM
/// models on the web); empty when `text` has none of those scripts. Offsets elsewhere (around
/// other text) are UAX #14's breaks.
pub(crate) fn unspaced_word_breaks(text: &str) -> Vec<usize> {
    #[cfg(not(target_arch = "wasm32"))]
    static SEGMENTER: std::sync::LazyLock<icu_segmenter::LineSegmenterBorrowed<'static>> =
        std::sync::LazyLock::new(|| icu_segmenter::LineSegmenter::new_dictionary(Default::default()));
    #[cfg(target_arch = "wasm32")]
    static SEGMENTER: std::sync::LazyLock<icu_segmenter::LineSegmenterBorrowed<'static>> =
        std::sync::LazyLock::new(|| icu_segmenter::LineSegmenter::new_auto(Default::default()));
    if !text.chars().any(is_unspaced_script) {
        return vec![];
    }
    SEGMENTER.segment_str(text).collect()
}

/// Mark the glyphs of `sg` (shaped from `text`, which starts at byte `start` of the text the
/// glyphs index) after which a word of Thai, Lao, Khmer or Myanmar ends ([`unspaced_word_breaks`]);
/// other text keeps its own breaks.
pub(crate) fn mark_word_ends(text: &str, start: usize, sg: &mut [SGlyph]) {
    let breaks: Vec<usize> = unspaced_word_breaks(text).into_iter().map(|b| b + start).collect();
    if breaks.is_empty() {
        return;
    }
    for g in sg.iter_mut().filter(|g| g.inline.is_none() && is_unspaced_script(g.ch)) {
        g.word_end = breaks.binary_search(&(g.byte + g.len)).is_ok();
    }
}

pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FFFF)
}

/// Vertical metrics (points) of a style's resolved face: (ascent, descent, leading).
pub(crate) fn style_metrics(db: &FontDb, st: &CharStyle) -> (f64, f64, f64) {
    let vs = st.v_scale / 100.0;
    let Some(face) = db.style_face(st) else {
        return (st.size * 0.8 * vs, st.size * 0.2 * vs, st.effective_leading());
    };
    let k = st.size / face.upem;
    (face.ascent * k * vs, face.descent * k * vs, st.effective_leading())
}

/// Cap height and x height (points) of a style's resolved face.
pub(crate) fn cap_x_heights(db: &FontDb, st: &CharStyle) -> (f64, f64) {
    let vs = st.v_scale / 100.0;
    let Some(face) = db.style_face(st) else {
        return (st.size * 0.7 * vs, st.size * 0.5 * vs);
    };
    let k = st.size / face.upem * vs;
    (face.cap_height * k, face.x_height * k)
}

/// Shape `text[range]`, where `runs` gives each run's byte range in `text` and style, `inlines`
/// (parallel to `runs`) the inline graphic of each run that is one, and `levels` each byte's bidi
/// embedding level from `range.start` (empty: all left to right). Glyphs come out in logical
/// order, right-to-left ones shaped right to left.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shape_range(
    db: &FontDb,
    text: &str,
    range: Range<usize>,
    runs: &[(Range<usize>, &CharStyle)],
    inlines: &[Option<&InlineArt>],
    feats: &OtFeatures,
    levels: &[Level],
    out: &mut Vec<SGlyph>,
) {
    if text.get(range.clone()).is_none() || range.is_empty() {
        return;
    }
    let level_at = |i: usize| levels.get(i - range.start).copied().unwrap_or_else(Level::ltr);
    let output_start = out.len();

    // Text runs also carry attributes the shaper does not read (fill, stroke, baseline shift and
    // others). Adjacent runs that shape alike ([`shapes_alike`]) are shaped together, so that a
    // change of those attributes is not a shaping boundary: kerning pairs, contextual forms and
    // joining go on across it. Each glyph takes those attributes from its own source run, found by
    // its source byte ([`shape_segment`]).
    /// A run of text shaped together: its byte range, style and source runs, or one inline graphic.
    struct ShapeSpan<'a> {
        bytes: Range<usize>,
        st: &'a CharStyle,
        source_runs: Range<usize>,
        art: Option<&'a InlineArt>,
    }
    let mut spans: Vec<ShapeSpan<'_>> = Vec::new();
    for (ri, (rr, st)) in runs.iter().enumerate() {
        let a = rr.start.max(range.start);
        let b = rr.end.min(range.end);
        if a >= b || text.get(a..b).is_none() {
            continue;
        }
        let art = inlines.get(ri).copied().flatten();
        // An inline graphic is a span of its own: never merged with its neighbours.
        if art.is_none()
            && let Some(previous) = spans.last_mut()
            && previous.art.is_none()
            && previous.bytes.end == a
            && shapes_alike(previous.st, st)
        {
            previous.bytes.end = b;
            previous.source_runs.end = ri + 1;
            continue;
        }
        spans.push(ShapeSpan { bytes: a..b, st, source_runs: ri..ri + 1, art });
    }

    for ShapeSpan { bytes, st, source_runs, art } in spans {
        let a = bytes.start;
        let b = bytes.end;
        if let Some(art) = art {
            if let Some(face) = db.style_face(st).or_else(|| db.face_covering('a')) {
                out.push(inline_glyph(&face, st, art, source_runs.start, a..b, text, level_at(a)));
            }
            continue;
        }
        let Some(primary) = db.style_face(st) else { continue };
        let pmap = primary.skrifa().map(|f| f.charmap());
        // Synthesized Small Caps shape lowercase letters separately (as smaller capitals).
        let small_caps = st.small_caps.is_some() && !st.all_caps;
        // Split into segments by font coverage (and case, for Small Caps).
        let mut seg = Segment { range: a..a, runs, source_runs, st, face: primary.clone(), small: false, level: level_at(a) };
        let mut cache: std::collections::HashMap<char, Arc<FontFace>> = std::collections::HashMap::new();
        let mut script = Script::Common;
        for (i, c) in text[a..b].char_indices() {
            let i = a + i;
            // Digits are drawn in the style's numeral system: the font must cover that digit.
            let c = st.digits.map(c);
            let level = level_at(i);
            // Whitespace the primary font doesn't map goes to the fallback face like any other
            // character (#1844): an ideographic space set at zero width would remove an indent.
            // What the layout handles itself (tab, line and paragraph breaks) stays.
            let covered = c.is_control() || pmap.as_ref().is_none_or(|m| m.map(c).is_some());
            let face = if covered {
                primary.clone()
            } else if let Some(f) = cache.get(&c) {
                f.clone()
            } else {
                let f = db.fallback_for_style(c, primary.id(), primary.weight, primary.italic).unwrap_or_else(|| primary.clone());
                cache.insert(c, f.clone());
                f
            };
            let small = small_caps && c.is_lowercase();
            let next_script = shaping_script(c);
            let strong_script = !matches!(next_script, Script::Common | Script::Inherited);
            let script_change = strong_script && script != Script::Common && script != next_script;
            // Combining marks, zero width joiners and non-joiners, and variation selectors stay
            // with the character before them.
            if (face.id() != seg.face.id() || small != seg.small || level != seg.level || script_change) && !is_mark(c) {
                if i > seg.range.start {
                    seg.range.end = i;
                    shape_segment(text, &range, &seg, feats, out);
                }
                seg = Segment { range: i..i, face, small, level, ..seg };
            }
            if strong_script {
                script = next_script;
            }
        }
        if b > seg.range.start {
            seg.range.end = b;
            shape_segment(text, &range, &seg, feats, out);
        }
    }
    // Right-to-left segments come out of the shaper in visual order: back to logical order for
    // line breaking (the lines are put in visual order once broken).
    if levels.iter().any(|l| l.is_rtl())
        && let Some(o) = out.get_mut(output_start..)
    {
        o.sort_by_key(|g| g.byte);
    }
}

/// Index in the original run list containing `byte`, limited to one coalesced shaping span.
/// Source runs are ordered and non-overlapping; partitioning keeps attribution bounded when a
/// document contains many adjacent style runs.
fn source_run_at(runs: &[(Range<usize>, &CharStyle)], span: &Range<usize>, byte: usize) -> Option<usize> {
    let source = runs.get(span.clone())?;
    let i = source.partition_point(|(range, _)| range.end <= byte);
    source.get(i).filter(|(range, _)| range.contains(&byte)).map(|_| span.start + i)
}

/// Do text runs in styles `a` and `b` shape alike, so that they are shaped as one piece of text?
/// They do when they differ only in paint and stroke, underline and strikethrough, overprint, the
/// character style's name, leading, baseline shift, rotation, Character Alignment, Character
/// Direction (text is shaped apart where its bidi level changes), the kerning (each glyph takes its
/// run's, see [`shape_segment`]) and the tracking, as long as the tracking keeps or drops standard
/// ligatures alike ([`crate::features::ligatures_suppressed_by`]).
fn shapes_alike(a: &CharStyle, b: &CharStyle) -> bool {
    // Every field is named, so that a new attribute has to be sorted into one group or the other.
    let CharStyle {
        font_family,
        font_style,
        font_version,
        font_axes,
        size,
        leading: _,
        tracking,
        kerning: _,
        baseline_shift: _,
        h_scale,
        v_scale,
        rotation: _,
        fill: _,
        stroke: _,
        stroke_width: _,
        underline: _,
        strikethrough: _,
        all_caps,
        features,
        style_name: _,
        overprint_fill: _,
        overprint_stroke: _,
        stroke_cap: _,
        stroke_join: _,
        stroke_miter_limit: _,
        stroke_dash: _,
        stroke_align: _,
        position,
        small_caps,
        char_align: _,
        char_direction: _,
        proportional_metrics,
        tate_chu_yoko,
        digits,
        no_break,
    } = a;
    *font_family == b.font_family
        && *font_style == b.font_style
        && *font_version == b.font_version
        && *font_axes == b.font_axes
        && *size == b.size
        && *h_scale == b.h_scale
        && *v_scale == b.v_scale
        && *all_caps == b.all_caps
        && *features == b.features
        && *position == b.position
        && *small_caps == b.small_caps
        && *proportional_metrics == b.proportional_metrics
        && *tate_chu_yoko == b.tate_chu_yoko
        && *digits == b.digits
        && *no_break == b.no_break
        && ligatures_suppressed_by(*tracking) == ligatures_suppressed_by(b.tracking)
}

/// The script `c` is shaped in: Japanese and Chinese text mixes Han, Hiragana, Katakana and
/// Bopomofo, shaped together (splitting them would cost a shaper call per change of script).
fn shaping_script(c: char) -> Script {
    match c.script() {
        Script::Hiragana | Script::Katakana | Script::Bopomofo => Script::Han,
        s => s,
    }
}

/// The synthetic glyph of an inline graphic run (`range` = its one character).
///
/// The art is scaled uniformly to `scale` x the font size tall, its left edge on the pen and its
/// vertical centre on the middle of the cap height raised by the inline's shift (see
/// [`InlineArt`]). Its advance is the scaled art width plus tracking; the glyph's ascent and
/// descent grow to the art's extent so a tall graphic opens up its line. A missing symbol takes a
/// square of the same height and draws nothing.
fn inline_glyph(face: &Arc<FontFace>, st: &CharStyle, art: &InlineArt, run: usize, range: Range<usize>, text: &str, level: Level) -> SGlyph {
    let size = if st.size.is_finite() { st.size.max(0.0) } else { 0.0 };
    let vs = if st.v_scale.is_finite() { st.v_scale / 100.0 } else { 1.0 };
    let km = size / face.upem;
    let (ascent, descent, cap, xh) = (face.ascent * km * vs, face.descent * km * vs, face.cap_height * km * vs, face.x_height * km * vs);
    let h = art.safe_scale() * size;
    let mid = cap * 0.5 + art.safe_shift();
    let art_bounds = art.bounds.filter(|b| b.height() > 1e-9 && b.width().is_finite() && b.height().is_finite());
    let (xf, width, drawn) = match art_bounds {
        Some(b) if h > 0.0 => {
            let k = h / b.height();
            let xf = kurbo::Affine::translate((0.0, -mid)) * kurbo::Affine::scale(k) * kurbo::Affine::translate((-b.x0, -(b.y0 + b.y1) * 0.5));
            (xf, b.width() * k, Some(b))
        }
        _ => (kurbo::Affine::IDENTITY, h, None),
    };
    let tracking = st.tracking / 1000.0 * size;
    let adv = width + if tracking.is_finite() { tracking } else { 0.0 };
    SGlyph {
        face: face.clone(),
        gid: 0,
        byte: range.start,
        len: range.len().max(1),
        run,
        adv: if adv.is_finite() { adv } else { 0.0 },
        dx: 0.0,
        dy: 0.0,
        // Not used to draw (the art has its own transform), but they make the glyph's em box
        // its run's em: Character Alignment and Top-to-Top leading treat an inline graphic as a
        // character of its run's size, whatever its scale (see `layout::glyph_em`).
        sx: if km.is_finite() { km * vs } else { 0.0 },
        sy: if km.is_finite() { km * vs } else { 0.0 },
        bshift: if st.baseline_shift.is_finite() { st.baseline_shift } else { 0.0 },
        rotation: 0.0,
        ascent: ascent.max(mid + h * 0.5),
        descent: descent.max(h * 0.5 - mid),
        leading: st.effective_leading(),
        cap,
        xh,
        ch: text.get(range.clone()).and_then(|s| s.chars().next()).unwrap_or(INLINE_CHAR),
        tcy: None,
        lead: 0.0,
        proportional: false,
        vpal: None,
        // U+FFFC is a bidi neutral (ON): its level, resolved from its neighbours, places it in
        // RTL text like any other glyph once the line is reordered.
        level,
        inline: Some(InlineBox { art: drawn, xf }),
        word_end: false,
        no_break: st.no_break,
        cursive: false,
    }
}

/// A piece of text shaped in one go: one face and, for Small Caps, one case, in runs that shape
/// alike ([`shapes_alike`]).
#[derive(Clone)]
struct Segment<'a> {
    range: Range<usize>,
    /// All the runs, and the indices of those the piece's shaping span takes in.
    runs: &'a [(Range<usize>, &'a CharStyle)],
    source_runs: Range<usize>,
    /// The style of the span's first run, which the others shape alike with.
    st: &'a CharStyle,
    face: Arc<FontFace>,
    /// Lowercase letters drawn as synthesized small capitals.
    small: bool,
    level: Level,
}

/// A segment shaped in one style: each glyph's id, cluster, x advance and x and y offset (font
/// units), whether `palt` gave it another advance, and what `vpal` does to it.
#[derive(Default)]
struct Shaped {
    raw: Vec<(u32, u32, i32, i32, i32)>,
    proportional: Vec<bool>,
    vpal: Vec<Option<VAdjust>>,
}

fn is_mark(c: char) -> bool {
    use unicode_general_category::{GeneralCategory, get_general_category};
    matches!(get_general_category(c), GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark | GeneralCategory::EnclosingMark)
        || matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}')
}

/// Shape `seg` and append its glyphs to `out`. Each glyph takes the tracking, kerning, baseline
/// shift, rotation and leading of its own source run. The font's kerning pairs apply after the
/// characters with automatic kerning: the manual value replaces the font's pair after its
/// character, and a character before it with automatic kerning keeps the font's pair with it
/// (except before a character that All Caps or Small Caps sets as several letters, such as ß as
/// SS). A cluster that would take in characters of runs with different styles (a ligature, a
/// base with its marks) is not formed: the segment is shaped in pieces at those run boundaries.
fn shape_segment(text: &str, context: &Range<usize>, seg: &Segment, feats: &OtFeatures, out: &mut Vec<SGlyph>) {
    let Segment { range, runs, source_runs, st, face, small, level } = seg;
    let (range, small) = (range.clone(), *small);
    // The source runs the segment's bytes lie in (a span can hold many segments).
    let span = runs.get(source_runs.clone()).unwrap_or_default();
    let first = span.partition_point(|(r, _)| r.end <= range.start);
    let last = span.partition_point(|(r, _)| r.start < range.end).max(first);
    let source_runs = &(source_runs.start + first..source_runs.start + last);
    let text_seg = &text[range.clone()];
    // The segment's script: that of its first character of a script of its own.
    let cursive = text_seg.chars().map(|c| c.script()).find(|s| !matches!(s, Script::Common | Script::Inherited)).is_some_and(joins);
    let full = st.size.max(0.0);
    // Superscript/subscript and small capitals shrink the glyphs; line metrics keep the full size.
    let (pos_scale, pos_shift) = st.position.scale_shift(full);
    let small_scale = if small { st.small_caps.unwrap_or(100.0) / 100.0 } else { 1.0 };
    let size = full * pos_scale * small_scale;
    let k = size / face.upem;
    let km = full / face.upem;
    let hs = st.h_scale / 100.0;
    let vs = st.v_scale / 100.0;
    let ascent = face.ascent * km * vs;
    let descent = face.descent * km * vs;
    let cap = face.cap_height * km * vs;
    let xh = face.x_height * km * vs;
    let upper = st.all_caps || small;
    let first_char = |byte: usize| text[byte..].chars().next().unwrap_or(' ');
    // The source run of the character at `byte`: its index and style.
    let run_at = |byte: usize| {
        let i = source_run_at(runs, source_runs, byte).unwrap_or(source_runs.start);
        (i, runs.get(i).map_or(*st, |(_, s)| *s))
    };
    // The starts of the source runs inside the segment.
    let boundaries = || runs.get(source_runs.clone()).unwrap_or_default().iter().map(|(r, _)| r.start).filter(|&b| b > range.start && b < range.end);
    // Shape the pieces of the segment between the bytes `at` (ascending, inside it) on their own.
    let shape_apart = |at: Vec<usize>, out: &mut Vec<SGlyph>| {
        let ends = at.iter().copied().chain([range.end]);
        for (start, end) in std::iter::once(range.start).chain(at.iter().copied()).zip(ends) {
            shape_segment(text, context, &Segment { range: start..end, ..seg.clone() }, feats, out);
        }
    };
    let manual = |byte: usize| run_at(byte).1.kerning.is_some();
    let mixed_kerning = boundaries().any(|b| manual(b) != manual(range.start));

    // Shape the segment as if all of it were in `st`, with the features `more` on top.
    let shape_in = |st: &CharStyle, more: &[Feature]| {
        let mut shaped = Shaped::default();
        let Shaped { raw, proportional, vpal } = &mut shaped;
        let hb_shaped = face.hb().map(|hb| {
            let shaper = face.shaper.shaper(&hb).instance(face.instance.as_ref()).build();
            // gid, cluster, x and y advance, x and y offset.
            let shape = |feats: &[Feature], direction: Direction| {
                let feats: Vec<Feature> = feats.iter().chain(more).cloned().collect();
                let mut buf = UnicodeBuffer::new();
                for (i, c) in text_seg.char_indices() {
                    let cl = (range.start + i) as u32;
                    let c = st.digits.map(c);
                    if upper {
                        for u in c.to_uppercase() {
                            buf.add(u, cl);
                        }
                    } else {
                        buf.add(c, cl);
                    }
                }
                buf.set_direction(direction);
                buf.guess_segment_properties();
                // Harfrust keeps this Unicode context for joining/positional shaping, but does not emit
                // glyphs for it. This retains context at real style/font segmentation boundaries without
                // shaping across those boundaries or claiming cross-style ligatures/mark attachment.
                if let Some(pre) = text.get(context.start..range.start) {
                    buf.set_pre_context(pre);
                }
                if let Some(post) = text.get(range.end..context.end) {
                    buf.set_post_context(post);
                }
                let gb = shaper.shape(buf, ShapeOptions::new().features(&feats));
                gb.glyph_infos()
                    .iter()
                    .zip(gb.glyph_positions())
                    .map(|(info, pos)| (info.glyph_id, info.cluster, pos.x_advance, pos.y_advance, pos.x_offset, pos.y_offset))
                    .collect::<Vec<_>>()
            };
            let along = if level.is_rtl() { Direction::RightToLeft } else { Direction::LeftToRight };
            *raw = shape(&feats.resolve(st), along).into_iter().map(|(g, c, xa, _, xo, yo)| (g, c, xa, xo, yo)).collect();
            let raw = &*raw;
            let same_glyphs = |v: &[(u32, u32, i32, i32, i32, i32)]| v.len() == raw.len() && v.iter().zip(raw).all(|(a, b)| a.0 == b.0);
            if feats.proportional(st) {
                // `palt` is positioning only: the same glyphs set on their full widths tell which
                // advances it changed (kerning and the rest change both alike).
                let full = shape(&feats.resolve_fixed_width(st), along);
                if same_glyphs(&full) {
                    *proportional = full.iter().zip(raw).map(|(a, b)| a.2 != b.2).collect();
                }
            }
            if feats.proportional_vertical(st) {
                // `vpal` adjusts heights, which only top-to-bottom shaping applies: the same glyphs set
                // top to bottom with it (and the vertical kerning that goes with it, `vkrn` and `vapk`)
                // and without either tell how it moves each and changes its advance down the column
                // (y up, advances negative). Which advances `vpal` itself changed comes from setting
                // them with it alone.
                let ttb = |proportional, kerning| shape(&feats.resolve_vertical(st, proportional, kerning), Direction::TopToBottom);
                let (with, without, alone) = (ttb(true, true), ttb(false, false), ttb(true, false));
                if same_glyphs(&with) && same_glyphs(&without) && same_glyphs(&alone) {
                    *vpal = with
                        .iter()
                        .zip(&without)
                        .zip(&alone)
                        .map(|((a, b), c)| {
                            let adj = VAdjust {
                                advance: f64::from(b.3 - a.3) * k * vs,
                                dx: f64::from(a.4 - b.4) * k * hs,
                                dy: -f64::from(a.5 - b.5) * k * vs,
                                proportional: c.3 != b.3,
                            };
                            (adj != VAdjust::default()).then_some(adj)
                        })
                        .collect();
                }
            }
        });
        if hb_shaped.is_none() {
            // Fallback: nominal glyphs and hmtx advances, no shaping.
            if let Some(f) = face.skrifa() {
                let cmap = f.charmap();
                let gm = f.glyph_metrics(Size::unscaled(), face.location());
                for (i, c) in text_seg.char_indices() {
                    let cl = (range.start + i) as u32;
                    let c = st.digits.map(c);
                    let chars: Vec<char> = if upper { c.to_uppercase().collect() } else { vec![c] };
                    for u in chars {
                        let g = cmap.map(u).unwrap_or_default();
                        let adv = gm.advance_width(g).unwrap_or(face.upem as f32 * 0.5);
                        raw.push((g.to_u32(), cl, adv.round() as i32, 0, 0));
                    }
                }
            }
        }
        shaped
    };
    let Shaped { raw, proportional, vpal } = if mixed_kerning {
        // A font's kerning pair applies after a character with automatic kerning and not after one
        // with manual kerning. The segment is shaped apart where manual kerning ends, so each piece
        // is text with automatic kerning followed by text with manual kerning. A piece is shaped
        // with the font's kerning on, and off (`kern`, and the `apkn`, `vkrn` and `vapk` that
        // `OtFeatures` turns off with it) from the second character with manual kerning on: a
        // pair is applied only when both of its glyphs have the feature on.
        let ends: Vec<usize> = boundaries().filter(|&b| manual(b - 1) && !manual(b)).collect();
        if !ends.is_empty() {
            return shape_apart(ends, out);
        }
        let first = boundaries().find(|&b| manual(b)).unwrap_or(range.end);
        // A first character with manual kerning that All Caps or Small Caps sets as several letters
        // (ß as SS) gives them one cluster, which a feature range cannot split, so the font's
        // kerning between them would stay on: it is shaped apart from the text before it.
        if upper && first_char(first).to_uppercase().nth(1).is_some() {
            return shape_apart(vec![first], out);
        }
        let off = text.get(first..range.end).and_then(|s| s.chars().next()).map_or(range.end, |c| first + c.len_utf8());
        let more: Vec<Feature> = [b"kern", b"apkn", b"vkrn", b"vapk"]
            .into_iter()
            .filter(|_| off < range.end)
            .map(|t| Feature::new(Tag::new(t), 0, off..range.end))
            .collect();
        shape_in(&CharStyle { kerning: None, ..(*st).clone() }, &more)
    } else {
        // The span's first run may lie outside the segment (a span split by font, case, script
        // or direction): the kerning mode is that of the segment's own runs.
        shape_in(run_at(range.start).1, &[])
    };
    let mut cluster_starts: Vec<usize> = raw.iter().map(|r| r.1 as usize).collect();
    cluster_starts.sort_unstable();
    cluster_starts.dedup();
    // A glyph is drawn in one style: where a cluster would take in characters of runs in different
    // styles, the runs are shaped apart, as runs that do not shape alike are.
    let inside_clusters: Vec<usize> = boundaries().filter(|&b| cluster_starts.binary_search(&b).is_err() && run_at(b - 1).1 != run_at(b).1).collect();
    if !inside_clusters.is_empty() {
        return shape_apart(inside_clusters, out);
    }
    let n = raw.len();
    for (gi, &(gid, cl, xa, xo, yo)) in raw.iter().enumerate() {
        let cl = cl as usize;
        let (run, gst) = run_at(cl);
        // Cluster end: the next larger cluster value in the segment, else the segment end.
        let end = cluster_starts.get(cluster_starts.partition_point(|&c| c <= cl)).copied().unwrap_or(range.end);
        let last_in_cluster = gi + 1 == n || raw[gi + 1].1 as usize != cl;
        let ch = first_char(cl);
        let mut adv = xa as f64 * k * hs;
        if ch == SOFT_HYPHEN {
            adv = 0.0;
        } else if last_in_cluster {
            let tracking = gst.tracking / 1000.0 * size;
            let manual_kern = gst.kerning.map(|v| v / 1000.0 * size).unwrap_or(0.0);
            adv += tracking + manual_kern;
        }
        out.push(SGlyph {
            face: face.clone(),
            gid,
            byte: cl,
            len: end.saturating_sub(cl).max(1),
            run,
            adv,
            dx: xo as f64 * k * hs,
            dy: -(yo as f64) * k * vs,
            sx: k * hs,
            sy: k * vs,
            bshift: gst.baseline_shift + pos_shift,
            rotation: gst.rotation,
            ascent,
            descent,
            leading: gst.effective_leading(),
            cap,
            xh,
            ch,
            tcy: None,
            lead: 0.0,
            proportional: proportional.get(gi).copied().unwrap_or(false),
            vpal: vpal.get(gi).copied().flatten(),
            level: *level,
            inline: None,
            word_end: false,
            no_break: gst.no_break,
            cursive,
        });
    }
}
