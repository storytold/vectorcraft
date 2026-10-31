use super::flow::flow;
use super::on_path::on_path;
use super::region::Region;
use super::ruby::ruby_layout;
use super::vertical::{EM_CENTER, QUARTER_TURN, align_vertically, finish_bounds};
use super::{
    Affine, BezPath, CharStyle, Ctx, FontDb, LayoutOptions, OtFeatures, Point, Rect, Shape, TextKind, TextLayout, TextObject, VerticalAlign,
};

/// Lay out a text object into text-space glyph outlines (default [`LayoutOptions`]).
pub fn layout(db: &FontDb, t: &TextObject) -> TextLayout {
    let a = &t.area;
    let opts = LayoutOptions {
        rows: a.rows,
        columns: a.columns,
        gutter: a.gutter,
        inset: a.inset,
        first_baseline: a.first_baseline,
        first_baseline_min: a.first_baseline_min,
        vertical_align: a.vertical_align,
        fit: a.fit,
        ..LayoutOptions::default()
    };
    layout_with(db, t, &opts)
}

/// Area type in a frame too short for its first line draws nothing (type that doesn't fit its frame
/// isn't drawn). For importers whose source shows that line: the frame is made taller, from its
/// top, by what the first line needs. Whether it was.
pub fn fit_first_line(db: &FontDb, t: &mut TextObject) -> bool {
    let TextKind::Area { frame } = &t.kind else { return false };
    let Some(b) = frame.bounds().filter(|b| b.height() > 1e-6) else { return false };
    if !layout(db, t).lines.is_empty() {
        return false;
    }
    // The height of its first line: the line's as point type.
    let point = TextObject { kind: TextKind::Point, ..t.clone() };
    let Some(need) = layout(db, &point).lines.first().map(|l| l.ascent + l.descent) else { return false };
    // (A little more than it, for the rounding of the layout.)
    let by = (need + 0.05) / b.height();
    if !(by.is_finite() && by > 1.0) {
        return false;
    }
    let taller = frame.transformed(Affine::translate((0.0, b.y0)) * Affine::scale_non_uniform(1.0, by) * Affine::translate((0.0, -b.y0)));
    let short = std::mem::replace(&mut t.kind, TextKind::Area { frame: taller });
    // Something else keeps the line out (a frame narrower than a letter): as the source has it.
    if layout(db, t).lines.is_empty() {
        t.kind = short;
        return false;
    }
    true
}

/// The underline and strikethrough bars of `t` laid out as `layout` (#847): per run and line, a
/// bar from the stretch's first glyph to its last one's advance, placed and sized by the font's
/// own underline and strikeout metrics through the glyph's transform (so the size, baseline shift
/// and scaling follow) → (run, bar in text space). Type on a path and vertical type have none yet.
pub fn decorations(layout: &TextLayout, db: &FontDb, t: &TextObject) -> Vec<(usize, BezPath)> {
    let mut out = vec![];
    if layout.on_path || layout.vertical {
        return out;
    }
    let glyphs = &layout.glyphs;
    let mut i = 0;
    while let Some(g) = glyphs.get(i) {
        // The stretch: the glyphs of this run on this line, in a row. A list marker has no
        // underline or strikethrough.
        let marker = layout.marker_of(i).is_some();
        let end = glyphs
            .iter()
            .enumerate()
            .skip(i)
            .position(|(k, h)| h.run != g.run || h.line != g.line || layout.marker_of(k).is_some() != marker)
            .map_or(glyphs.len(), |n| i + n);
        let stretch = glyphs.get(i..end).unwrap_or_default();
        i = end.max(i + 1);
        if marker {
            continue;
        }
        let Some(style) = t.runs.get(g.run).map(|r| &r.style) else { continue };
        if !(style.underline || style.strikethrough) {
            continue;
        }
        let Some(face) = db.face_by_id(g.font_id) else { continue };
        let (x0, x1) = stretch.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), h| {
            let (s, e) = (h.origin.x, h.origin.x + h.advance);
            (a.min(s).min(e), b.max(s).max(e))
        });
        for (on, (offset, thickness)) in [(style.underline, face.underline), (style.strikethrough, face.strikeout)] {
            if !on {
                continue;
            }
            // Font units run y down in the glyph's transform: the bar's top is at -offset.
            let (top, bottom) = ((g.xf * Point::new(0.0, -offset)).y, (g.xf * Point::new(0.0, -offset + thickness)).y);
            let bar = Rect::new(x0, top.min(bottom), x1, top.max(bottom));
            if bar.width() > 0.0 && bar.height() > 0.0 && bar.width().is_finite() && bar.height().is_finite() {
                out.push((g.run, bar.to_path(0.1)));
            }
        }
    }
    out
}

/// Most layout passes Shrink Text to Fit runs past the first (bisection plus the final pass).
const SHRINK_PASSES: usize = 11;

/// Lay out a text object with explicit options (area type rows/columns, inset, first baseline,
/// fit, composer, OpenType features).
pub fn layout_with(db: &FontDb, t: &TextObject, opts: &LayoutOptions) -> TextLayout {
    let first = layout_once(db, t, opts);
    match (opts.fit.min_scale(), &t.kind) {
        (Some(min), TextKind::Area { .. }) if first.overflow => shrink_to_fit(db, t, opts, min).unwrap_or(first),
        _ => first,
    }
}

/// Shrink Text to Fit for text that overflows at full size: the largest scale in `min..1` at which
/// it fits, searched by bisection over the first run's document size in steps of 0.1 pt (so its scaled size
/// is a whole number of tenths of a point, the same every time). Down at `min` it lays out there,
/// still overflowing. None (keep the full-size layout) when the text has no usable size.
fn shrink_to_fit(db: &FontDb, t: &TextObject, opts: &LayoutOptions, min: f64) -> Option<TextLayout> {
    // Exact size edits after object scaling may store a local size below 0.1. Fit in the
    // same document points as the Character controls, independent of that representation.
    let points = t.style_scale()?.points;
    let size = t.runs.first().map(|r| r.style.size * points).filter(|s| s.is_finite() && *s >= 0.1)?;
    if !(min.is_finite() && min < 1.0) {
        return None;
    }
    let tenths = size * 10.0;
    if !tenths.is_finite() {
        return None;
    }
    // Candidate sizes k/10 pt for k in lo..hi: `hi` (full size, rounded down) is known to overflow
    // unless it is below the full size; `lo` is the smallest allowed.
    let mut lo = (tenths * min).ceil().max(1.0);
    let mut hi = tenths.floor();
    if hi < lo {
        hi = lo;
    }
    let at = |k: f64| -> TextLayout {
        let f = (k / tenths).clamp(min, 1.0);
        let mut l = layout_once(db, &scaled(t, f), opts);
        l.fit_scale = f;
        l
    };
    let mut passes = 0;
    let mut best: Option<TextLayout> = None;
    // `hi` itself may fit when the size isn't a whole number of tenths.
    if hi < tenths && hi > lo {
        let l = at(hi);
        passes += 1;
        if !l.overflow {
            return Some(l);
        }
    }
    // Invariant: everything at `hi` or above overflows; `lo` fits or is the floor.
    while hi - lo > 1.0 && passes + 1 < SHRINK_PASSES {
        let mid = (lo + (hi - lo) * 0.5).floor();
        let l = at(mid);
        passes += 1;
        if l.overflow {
            hi = mid;
        } else {
            lo = mid;
            best = Some(l);
        }
    }
    // `best` is the layout at `lo` once `lo` has moved; at the floor it still needs laying out.
    Some(best.unwrap_or_else(|| at(lo)))
}

/// `t` with every run's size, explicit leading and baseline shift scaled by `f`, inline art's own
/// shift included (auto leading and the art's size follow the size; paragraph spacing stays).
/// Shrink Text to Fit lays `t` out as `scaled(t, f)`, where `f` is the layout's
/// [`TextLayout::fit_scale`], so the result holds the styles the text is drawn in.
pub fn scaled(t: &TextObject, f: f64) -> TextObject {
    let mut s = t.clone();
    for r in &mut s.runs {
        r.style.size *= f;
        r.style.leading = r.style.leading.map(|l| l * f);
        r.style.baseline_shift *= f;
        if let Some(a) = &mut r.inline {
            a.baseline_shift *= f;
        }
    }
    s
}

/// One layout pass, without fitting.
fn layout_once(db: &FontDb, t: &TextObject, opts: &LayoutOptions) -> TextLayout {
    let text = t.plain_text();
    let mut runs = Vec::with_capacity(t.runs.len());
    let mut inlines = Vec::with_capacity(t.runs.len());
    let mut off = 0;
    for r in &t.runs {
        runs.push((off..off + r.text.len(), &r.style));
        // Only a well-formed inline run (one object replacement character) is a graphic.
        inlines.push(r.inline.as_ref().filter(|_| crate::edit::is_inline_text(&r.text)));
        off += r.text.len();
    }
    let mut paras = Vec::new();
    let mut s = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            paras.push(s..i);
            s = i + 1;
        }
    }
    paras.push(s..text.len());
    let vertical = t.vertical;
    let is_on_path = matches!(&t.kind, TextKind::OnPath { .. });
    let vopts;
    let opts = if vertical {
        vopts = LayoutOptions { features: OtFeatures { vertical: true, ..opts.features }, ..opts.clone() };
        &vopts
    } else {
        opts
    };
    let mut cx = Ctx {
        on_path: is_on_path,
        db,
        text: &text,
        runs,
        inlines,
        default: CharStyle::default(),
        opts,
        vertical,
        system_layout: t.system_layout,
        fractional_widths: t.fractional_widths,
        out: TextLayout::default(),
    };
    // Vertical type: line space turned a quarter turn clockwise (lines become columns, each next
    // one to the left). Point type's anchor is on the first column's centre line. Vertical path
    // type stays on its path (each glyph turned across it in `emit`).
    let line_xf = match &t.kind {
        _ if !vertical => Affine::IDENTITY,
        TextKind::OnPath { .. } => Affine::IDENTITY,
        TextKind::Point => {
            let first = cx.style_at(0);
            let centre = db.style_face(first).map_or(EM_CENTER, |f| f.ideographic_centre());
            Affine::translate((-centre * first.size, 0.0)) * QUARTER_TURN
        }
        _ => QUARTER_TURN,
    };
    match &t.kind {
        TextKind::Point => flow(&mut cx, &paras, t, None),
        TextKind::Area { frame } => {
            let to_lines = line_xf.inverse();
            let wrap: Vec<vectorcraft_doc::WrapShape> = if vertical {
                t.wrap.iter().map(|w| vectorcraft_doc::WrapShape { path: w.path.transformed(to_lines), ..w.clone() }).collect()
            } else {
                t.wrap.clone()
            };
            let mut regions = Region::cells(&(to_lines * frame.to_bezpath()), opts, &wrap);
            cx.out.frames = regions.iter().map(|r| line_xf.transform_rect_bbox(r.cell)).collect();
            flow(&mut cx, &paras, t, Some(&regions));
            if opts.vertical_align != VerticalAlign::Top {
                align_vertically(&mut cx, &paras, t, &mut regions);
            }
        }
        TextKind::OnPath { path, .. } => on_path(&mut cx, &paras, t, path),
    }
    // Ruby (furigana): placed over its base text in line space, before vertical type turns the
    // lines into columns, so the same pass puts it above (right in vertical type) and widens the
    // lines it sits on. Type on a path has no above/below; its ruby is left out for now.
    if !is_on_path && !t.ruby.is_empty() {
        ruby_layout(&mut cx, t);
    }
    if vertical && !is_on_path {
        // Glyph origins, outlines and transforms to text space (lines stay in line space).
        for g in &mut cx.out.glyphs {
            g.outline.apply_affine(line_xf);
            g.xf = line_xf * g.xf;
            g.origin = line_xf * g.origin;
            g.angle += std::f64::consts::FRAC_PI_2;
        }
        // Ruby glyphs are placed in line space too (upright, along the column, in the vertical
        // pass); the same turn takes them to text space.
        for g in &mut cx.out.ruby {
            g.outline.apply_affine(line_xf);
            g.xf = line_xf * g.xf;
            g.origin = line_xf * g.origin;
            g.angle += std::f64::consts::FRAC_PI_2;
        }
        for i in &mut cx.out.inlines {
            i.xf = line_xf * i.xf;
            i.bounds = line_xf.transform_rect_bbox(i.bounds);
        }
        cx.out.vertical = true;
        cx.out.line_xf = line_xf;
    }
    finish_bounds(&mut cx.out);
    cx.out
}
