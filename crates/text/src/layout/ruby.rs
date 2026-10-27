use super::driver::layout;
use super::*;

/// Ruby (furigana): lays each reading out over its base text in line space, grows the line's
/// ascent/descent so the ruby fits, and moves the lines after it on in the block direction.
///
/// Positions are measured from the line's baseline. Vertical type uses the same geometry — ruby
/// "above" the baseline is right of the column once the lines are turned — with each reading
/// character set upright along the column (a quarter turn that the final vertical turn undoes).
/// A reading over a line break is split between the lines, by the base characters' share of it.
/// A reading wider than its base hangs over the neighbours (`overhang`), or is squeezed to the
/// base's width. Type on a path is left out (it has no above/below).
pub(super) fn ruby_layout(cx: &mut Ctx<'_>, t: &TextObject) {
    use std::collections::BTreeMap;

    let db = cx.db;
    let text_len = cx.text.len();
    let n_lines = cx.out.lines.len();
    if n_lines == 0 || cx.out.glyphs.is_empty() {
        return;
    }
    let base_baselines: Vec<f64> = cx.out.lines.iter().map(|l| l.baseline).collect();
    let old_ascent: Vec<f64> = cx.out.lines.iter().map(|l| l.ascent).collect();
    let old_descent: Vec<f64> = cx.out.lines.iter().map(|l| l.descent).collect();
    // One piece of base text on one line, with the part of the reading that belongs to it.
    struct Piece {
        line: usize,
        x0: f64,
        x1: f64,
        run: usize,
        byte: usize,
        reading: String,
        style: CharStyle,
        position: RubyPosition,
        align: RubyAlign,
        base_size: f64,
        overhang: bool,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    // The glyphs by their byte, to find a span's without a scan of them all for each span.
    let byte_of = |i: usize| cx.out.glyphs.get(i).map_or(0, |g| g.byte);
    let mut by_byte: Vec<usize> = (0..cx.out.glyphs.len()).collect();
    by_byte.sort_by_key(|&i| byte_of(i));
    let longest = cx.out.glyphs.iter().map(|g| g.len.max(1)).max().unwrap_or(1);
    for span in &t.ruby {
        let (start, end) = (span.start.min(text_len), span.end.min(text_len));
        if start >= end || span.text.trim().is_empty() {
            continue;
        }
        // The base glyphs of the span, by line; a glyph's cluster is its byte range (`len`, or
        // one byte for generated glyphs). A list marker is not part of the base.
        let mut by_line: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let hi = by_byte.partition_point(|&i| byte_of(i) < end);
        let lo = by_byte.partition_point(|&i| byte_of(i).saturating_add(longest) <= start).min(hi);
        let mut hits: Vec<usize> = by_byte.get(lo..hi).unwrap_or_default().to_vec();
        // In glyph order, as the scan found them.
        hits.sort_unstable();
        for i in hits {
            let Some(g) = cx.out.glyphs.get(i) else { continue };
            let b = g.byte.saturating_add(g.len.max(1));
            if g.byte < end && b > start && cx.out.marker_of(i).is_none() {
                by_line.entry(g.line).or_default().push(i);
            }
        }
        let total: usize = by_line.values().map(Vec::len).sum();
        if total == 0 {
            continue;
        }
        let reading: Vec<char> = span.text.trim().chars().collect();
        let ratio = span.safe_size_percent() / 100.0;
        // Split the reading by each line's share of the base glyphs: cumulative fractions keep
        // every character once and in order.
        let mut taken = 0;
        let mut done = 0;
        let lines: Vec<usize> = by_line.keys().copied().collect();
        for line in &lines {
            let count = by_line.get(line).map_or(0, Vec::len);
            done += count;
            let upto = if reading.is_empty() { 0 } else { (reading.len() * done / total).min(reading.len()) };
            let part: String = reading.get(taken.min(reading.len())..upto.min(reading.len())).unwrap_or_default().iter().collect();
            taken = upto;
            if part.is_empty() {
                continue;
            }
            let glyphs = by_line.get(line).map(Vec::as_slice).unwrap_or_default();
            let mut x0 = f64::INFINITY;
            let mut x1 = f64::NEG_INFINITY;
            let mut base_size: f64 = 0.0;
            let mut run = 0;
            let mut byte = start;
            for (j, &gi) in glyphs.iter().enumerate() {
                let Some(g) = cx.out.glyphs.get(gi) else { continue };
                x0 = x0.min(g.origin.x);
                x1 = x1.max(g.origin.x + g.advance);
                base_size = base_size.max(cx.style_at(g.byte).size);
                if j == 0 {
                    run = g.run;
                    byte = g.byte;
                }
            }
            if !(x0.is_finite() && x1.is_finite()) || base_size <= 0.0 {
                continue;
            }
            let mut style = cx.style_at(byte).clone();
            style.size = base_size * ratio;
            style.leading = None;
            style.baseline_shift = 0.0;
            style.tracking = 0.0;
            style.kerning = None;
            pieces.push(Piece {
                line: *line,
                x0,
                x1,
                run,
                byte,
                reading: part,
                style,
                position: span.position,
                align: span.align,
                base_size,
                overhang: span.overhang,
            });
        }
    }
    // Phase A: shape each reading, place it relative to the (unshifted) baseline, and note how
    // far above/below the baseline its ink reaches.
    let mut placed: Vec<(usize, Vec<PositionedGlyph>, f64, f64)> = Vec::new(); // (line, glyphs, ascent, descent)
    for piece in &pieces {
        let Some(baseline) = base_baselines.get(piece.line).copied() else { continue };
        let mini = TextObject::point(Point::ZERO, &piece.reading, piece.style.clone());
        let lay = layout(db, &mini);
        let Some(first_line) = lay.lines.first() else { continue };
        let mini_base = first_line.baseline;
        let mini_asc = first_line.ascent;
        let mini_desc = first_line.descent;
        let base_w = (piece.x1 - piece.x0).max(0.0);
        // Above: the reading's bottom sits on the base's em box top. Below: its top sits on the
        // base's em box bottom (the line's descent below the baseline).
        let ruby_y = if piece.position == RubyPosition::Above {
            baseline - piece.base_size - mini_desc
        } else {
            baseline + old_descent.get(piece.line).copied().unwrap_or(0.0) + mini_asc
        };
        let dest = |width: f64| match piece.align {
            RubyAlign::Center => piece.x0 + (base_w - width) * 0.5,
            RubyAlign::Start => piece.x0,
            RubyAlign::End => piece.x1 - width,
        };
        let mut glyphs: Vec<PositionedGlyph> = Vec::new();
        if cx.vertical {
            // Each reading character in its own cell along the column, upright once the lines turn.
            let cell = (mini_asc + mini_desc).max(piece.style.size * 0.5);
            let mut clusters: Vec<Vec<&PositionedGlyph>> = Vec::new();
            for g in &lay.glyphs {
                match clusters.last_mut() {
                    Some(c) if c.last().is_some_and(|p| p.byte == g.byte && p.len == g.len) => c.push(g),
                    _ => clusters.push(vec![g]),
                }
            }
            let mut ruby_w = clusters.len() as f64 * cell;
            let mut squeeze = 1.0;
            if !piece.overhang && ruby_w > base_w + EPS && ruby_w > EPS {
                squeeze = base_w / ruby_w;
                ruby_w = base_w;
            }
            let x = dest(ruby_w);
            for (ci, cluster) in clusters.iter().enumerate() {
                let anchor = Point::new(x + (ci as f64 + 0.5) * cell, ruby_y);
                let mut box_: Option<Rect> = None;
                for g in cluster {
                    if g.outline.elements().is_empty() {
                        continue;
                    }
                    let b = g.outline.bounding_box();
                    box_ = Some(box_.map_or(b, |a| a.union(b)));
                }
                let centre = box_.map_or(anchor, |b| Point::new((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5));
                let mut m =
                    Affine::rotate_about(-std::f64::consts::FRAC_PI_2, anchor) * Affine::translate((anchor.x - centre.x, anchor.y - centre.y));
                if squeeze != 1.0 {
                    m = Affine::translate((x, 0.0)) * Affine::scale_non_uniform(squeeze, 1.0) * Affine::translate((-x, 0.0)) * m;
                }
                for g in cluster {
                    glyphs.push(ruby_glyph(g, m, piece.run, piece.byte, piece.line));
                }
            }
        } else {
            // The reading set along the line: the line's extent, centred or aligned to the base.
            let mut ruby_w = (first_line.x1 - first_line.x0).max(0.0);
            let mut squeeze = 1.0;
            if !piece.overhang && ruby_w > base_w + EPS && ruby_w > EPS {
                squeeze = base_w / ruby_w;
                ruby_w = base_w;
            }
            let x = dest(ruby_w);
            let mut m = Affine::translate((x - first_line.x0, ruby_y - mini_base));
            if squeeze != 1.0 {
                m = Affine::translate((x, 0.0)) * Affine::scale_non_uniform(squeeze, 1.0) * Affine::translate((-x, 0.0)) * m;
            }
            for g in &lay.glyphs {
                glyphs.push(ruby_glyph(g, m, piece.run, piece.byte, piece.line));
            }
        }
        // How far the reading reaches above and below the base line's baseline (its ink).
        let mut ascent: f64 = 0.0;
        let mut descent: f64 = 0.0;
        for g in &glyphs {
            if g.outline.elements().is_empty() {
                continue;
            }
            let b = g.outline.bounding_box();
            ascent = ascent.max(baseline - b.y0);
            descent = descent.max(b.y1 - baseline);
        }
        if !glyphs.is_empty() {
            placed.push((piece.line, glyphs, ascent, descent));
        }
    }
    if placed.is_empty() {
        return;
    }
    // Phase B: grow each line's box to fit its ruby, then move the lines on after it by what they
    // need in the block direction (the first line too for area type, so the ruby stays inside the
    // frame; point type's first baseline is its anchor and stays).
    let mut growth = vec![0.0_f64; n_lines];
    for (line, _, ascent, descent) in &placed {
        if let (Some(a), Some(d)) = (old_ascent.get(*line), old_descent.get(*line)) {
            let grown = (a.max(*ascent) + d.max(*descent)) - (a + d);
            if grown > 0.0 {
                let g = growth.get_mut(*line);
                if let Some(slot) = g {
                    *slot = slot.max(grown);
                }
            }
        }
    }
    // A line's ruby grows above it: the line moves on in the block direction by its own growth,
    // so the line before it keeps its room (for area type the first line moves too, keeping the
    // ruby inside the frame; point type's first baseline is its anchor and stays).
    let area = matches!(t.kind, TextKind::Area { .. });
    let mut shift = vec![0.0_f64; n_lines];
    let mut cum = 0.0;
    for i in 0..n_lines {
        if area || i > 0 {
            cum += growth.get(i).copied().unwrap_or(0.0);
            if let Some(s) = shift.get_mut(i) {
                *s = cum;
            }
        }
    }
    // Grow the lines' boxes to fit their ruby, then move them on in the block direction.
    for (line, _, ascent, descent) in &placed {
        if let Some(l) = cx.out.lines.get_mut(*line) {
            l.ascent = l.ascent.max(*ascent);
            l.descent = l.descent.max(*descent);
        }
    }
    for (i, line) in cx.out.lines.iter_mut().enumerate() {
        line.baseline += shift.get(i).copied().unwrap_or(0.0);
    }
    for g in &mut cx.out.glyphs {
        let s = shift.get(g.line).copied().unwrap_or(0.0);
        if s != 0.0 {
            g.origin.y += s;
            g.outline.apply_affine(Affine::translate((0.0, s)));
            g.xf = Affine::translate((0.0, s)) * g.xf;
        }
    }
    for (line, glyphs, _, _) in placed {
        let s = shift.get(line).copied().unwrap_or(0.0);
        for mut g in glyphs {
            if s != 0.0 {
                g.origin.y += s;
                g.outline.apply_affine(Affine::translate((0.0, s)));
            }
            g.line = line;
            cx.out.ruby.push(g);
        }
    }
}

/// A ruby glyph from one of a reading's laid-out glyphs, moved by `m` into the base text's line
/// space. It is not a character of the text: `len` 0, the base text's `run` for fill and stroke.
fn ruby_glyph(g: &PositionedGlyph, m: Affine, run: usize, byte: usize, line: usize) -> PositionedGlyph {
    PositionedGlyph {
        outline: m * g.outline.clone(),
        run,
        byte,
        origin: m * g.origin,
        advance: 0.0,
        len: 0,
        angle: 0.0,
        line,
        font_id: g.font_id,
        gid: g.gid,
        xf: Affine::IDENTITY,
        rtl: false,
        scale: 1.0,
        // Ruby draws outlines only.
        image: None,
        colour: None,
        bend: None,
    }
}
