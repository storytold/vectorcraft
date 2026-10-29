use super::driver::layout;
use super::{Affine, CharStyle, Ctx, EPS, Point, PositionedGlyph, RubyAlign, RubyPosition, Shape, TextKind, TextObject};

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
    let line_count = cx.out.lines.len();
    if line_count == 0 || cx.out.glyphs.is_empty() {
        return;
    }
    let base_baselines = cx.out.lines.iter().map(|line| line.baseline).collect::<Vec<_>>();
    let old_ascent = cx.out.lines.iter().map(|line| line.ascent).collect::<Vec<_>>();
    let old_descent = cx.out.lines.iter().map(|line| line.descent).collect::<Vec<_>>();
    let pieces = ruby_pieces(cx, t);
    // Phase A: shape each reading, place it relative to the (unshifted) baseline, and note how
    // far above/below the baseline its ink reaches.
    let placed = place_ruby_pieces(cx, &pieces, &base_baselines, &old_descent);
    if !placed.is_empty() {
        grow_ruby_lines(cx, t, placed, &old_ascent, &old_descent);
    }
}

// One piece of base text on one line, with the part of the reading that belongs to it.
struct RubyPiece {
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

struct PlacedRuby {
    line: usize,
    glyphs: Vec<PositionedGlyph>,
    ascent: f64,
    descent: f64,
}

fn ruby_pieces(cx: &Ctx<'_>, t: &TextObject) -> Vec<RubyPiece> {
    use std::collections::BTreeMap;

    let text_len = cx.text.len();
    // The glyphs by their byte, to find a span's without a scan of them all for each span.
    let byte_of = |i: usize| cx.out.glyphs.get(i).map_or(0, |g| g.byte);
    let mut by_byte: Vec<usize> = (0..cx.out.glyphs.len()).collect();
    by_byte.sort_by_key(|&i| byte_of(i));
    let longest = cx.out.glyphs.iter().map(|g| g.len.max(1)).max().unwrap_or(1);
    let mut pieces = Vec::new();
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
            let Some(glyph) = cx.out.glyphs.get(i) else { continue };
            let glyph_end = glyph.byte.saturating_add(glyph.len.max(1));
            if glyph.byte < end && glyph_end > start && cx.out.marker_of(i).is_none() {
                by_line.entry(glyph.line).or_default().push(i);
            }
        }
        let total: usize = by_line.values().map(Vec::len).sum();
        if total == 0 {
            continue;
        }
        collect_ruby_pieces_for_span(cx, span, start, total, &by_line, &mut pieces);
    }
    pieces
}

fn collect_ruby_pieces_for_span(
    cx: &Ctx<'_>,
    span: &vectorcraft_doc::text::RubySpan,
    start: usize,
    total: usize,
    by_line: &std::collections::BTreeMap<usize, Vec<usize>>,
    pieces: &mut Vec<RubyPiece>,
) {
    let reading: Vec<char> = span.text.trim().chars().collect();
    let ratio = span.safe_size_percent() / 100.0;
    // Split the reading by each line's share of the base glyphs: cumulative fractions keep
    // every character once and in order.
    let mut taken = 0;
    let mut done = 0;
    for (&line, glyph_indices) in by_line {
        done += glyph_indices.len();
        let upto = if reading.is_empty() { 0 } else { (reading.len() * done / total).min(reading.len()) };
        let part: String = reading.get(taken.min(reading.len())..upto.min(reading.len())).unwrap_or_default().iter().collect();
        taken = upto;
        if part.is_empty() {
            continue;
        }
        if let Some(piece) = ruby_piece_for_line(cx, line, start, &part, ratio, glyph_indices, span) {
            pieces.push(piece);
        }
    }
}

fn ruby_piece_for_line(
    cx: &Ctx<'_>,
    line: usize,
    start: usize,
    reading: &str,
    ratio: f64,
    glyph_indices: &[usize],
    span: &vectorcraft_doc::text::RubySpan,
) -> Option<RubyPiece> {
    let mut x0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut base_size: f64 = 0.0;
    let mut run = 0;
    let mut byte = start;
    for (index, &glyph_index) in glyph_indices.iter().enumerate() {
        let Some(glyph) = cx.out.glyphs.get(glyph_index) else { continue };
        x0 = x0.min(glyph.origin.x);
        x1 = x1.max(glyph.origin.x + glyph.advance);
        base_size = base_size.max(cx.style_at(glyph.byte).size);
        if index == 0 {
            run = glyph.run;
            byte = glyph.byte;
        }
    }
    if !(x0.is_finite() && x1.is_finite()) || base_size <= 0.0 {
        return None;
    }
    let mut style = cx.style_at(byte).clone();
    style.size = base_size * ratio;
    style.leading = None;
    style.baseline_shift = 0.0;
    style.tracking = 0.0;
    style.kerning = None;
    Some(RubyPiece {
        line,
        x0,
        x1,
        run,
        byte,
        reading: reading.to_string(),
        style,
        position: span.position,
        align: span.align,
        base_size,
        overhang: span.overhang,
    })
}

fn place_ruby_pieces(cx: &Ctx<'_>, pieces: &[RubyPiece], baselines: &[f64], old_descent: &[f64]) -> Vec<PlacedRuby> {
    pieces.iter().filter_map(|piece| place_ruby_piece(cx, piece, baselines, old_descent)).collect()
}

fn place_ruby_piece(cx: &Ctx<'_>, piece: &RubyPiece, baselines: &[f64], old_descent: &[f64]) -> Option<PlacedRuby> {
    let baseline = baselines.get(piece.line).copied()?;
    let mini = TextObject::point(Point::ZERO, &piece.reading, piece.style.clone());
    let layout = layout(cx.db, &mini);
    let first_line = layout.lines.first()?;
    let base_width = (piece.x1 - piece.x0).max(0.0);
    let ruby_y = ruby_baseline(piece, baseline, first_line.ascent, first_line.descent, old_descent);
    let glyphs = if cx.vertical {
        place_vertical_ruby(piece, &layout.glyphs, first_line.ascent, first_line.descent, base_width, ruby_y)
    } else {
        place_horizontal_ruby(piece, &layout.glyphs, first_line, base_width, ruby_y)
    };
    let (ascent, descent) = ruby_ink_extents(&glyphs, baseline);
    (!glyphs.is_empty()).then_some(PlacedRuby { line: piece.line, glyphs, ascent, descent })
}

fn ruby_baseline(piece: &RubyPiece, baseline: f64, reading_ascent: f64, reading_descent: f64, old_descent: &[f64]) -> f64 {
    // Above: the reading's bottom sits on the base's em box top. Below: its top sits on the
    // base's em box bottom (the line's descent below the baseline).
    if piece.position == RubyPosition::Above {
        baseline - piece.base_size - reading_descent
    } else {
        baseline + old_descent.get(piece.line).copied().unwrap_or(0.0) + reading_ascent
    }
}

fn ruby_destination(piece: &RubyPiece, base_width: f64, reading_width: f64) -> f64 {
    match piece.align {
        RubyAlign::Center => piece.x0 + (base_width - reading_width) * 0.5,
        RubyAlign::Start => piece.x0,
        RubyAlign::End => piece.x1 - reading_width,
    }
}

fn place_vertical_ruby(piece: &RubyPiece, reading: &[PositionedGlyph], asc: f64, desc: f64, base_width: f64, ruby_y: f64) -> Vec<PositionedGlyph> {
    // Each reading character in its own cell along the column, upright once the lines turn.
    let cell = (asc + desc).max(piece.style.size * 0.5);
    let clusters = ruby_clusters(reading);
    let (ruby_width, squeeze) = ruby_width(clusters.len() as f64 * cell, base_width, piece.overhang);
    let x = ruby_destination(piece, base_width, ruby_width);
    let mut glyphs = Vec::new();
    for (index, cluster) in clusters.iter().enumerate() {
        let anchor = Point::new(x + (index as f64 + 0.5) * cell, ruby_y);
        let centre = ruby_cluster_center(cluster, anchor);
        let transform = squeeze_transform(
            Affine::rotate_about(-std::f64::consts::FRAC_PI_2, anchor) * Affine::translate((anchor.x - centre.x, anchor.y - centre.y)),
            x,
            squeeze,
        );
        for glyph in cluster {
            glyphs.push(ruby_glyph(glyph, transform, piece.run, piece.byte, piece.line));
        }
    }
    glyphs
}

fn ruby_clusters(glyphs: &[PositionedGlyph]) -> Vec<Vec<&PositionedGlyph>> {
    let mut clusters: Vec<Vec<&PositionedGlyph>> = Vec::new();
    for glyph in glyphs {
        match clusters.last_mut() {
            Some(cluster) if cluster.last().is_some_and(|previous| previous.byte == glyph.byte && previous.len == glyph.len) => cluster.push(glyph),
            _ => clusters.push(vec![glyph]),
        }
    }
    clusters
}

fn ruby_cluster_center(cluster: &[&PositionedGlyph], anchor: Point) -> Point {
    let bounds = cluster
        .iter()
        .filter(|glyph| !glyph.outline.elements().is_empty())
        .map(|glyph| glyph.outline.bounding_box())
        .reduce(|left, right| left.union(right));
    bounds.map_or(anchor, |bounds| Point::new((bounds.x0 + bounds.x1) * 0.5, (bounds.y0 + bounds.y1) * 0.5))
}

fn place_horizontal_ruby(
    piece: &RubyPiece,
    reading: &[PositionedGlyph],
    line: &super::LineInfo,
    base_width: f64,
    ruby_y: f64,
) -> Vec<PositionedGlyph> {
    // The reading set along the line: the line's extent, centred or aligned to the base.
    let reading_width = (line.x1 - line.x0).max(0.0);
    let (ruby_width, squeeze) = ruby_width(reading_width, base_width, piece.overhang);
    let x = ruby_destination(piece, base_width, ruby_width);
    let transform = squeeze_transform(Affine::translate((x - line.x0, ruby_y - line.baseline)), x, squeeze);
    reading.iter().map(|glyph| ruby_glyph(glyph, transform, piece.run, piece.byte, piece.line)).collect()
}

fn ruby_width(reading_width: f64, base_width: f64, overhang: bool) -> (f64, f64) {
    if !overhang && reading_width > base_width + EPS && reading_width > EPS { (base_width, base_width / reading_width) } else { (reading_width, 1.0) }
}

fn squeeze_transform(transform: Affine, x: f64, squeeze: f64) -> Affine {
    if squeeze == 1.0 {
        transform
    } else {
        Affine::translate((x, 0.0)) * Affine::scale_non_uniform(squeeze, 1.0) * Affine::translate((-x, 0.0)) * transform
    }
}

fn ruby_ink_extents(glyphs: &[PositionedGlyph], baseline: f64) -> (f64, f64) {
    // How far the reading reaches above and below the base line's baseline (its ink).
    let mut ascent: f64 = 0.0;
    let mut descent: f64 = 0.0;
    for glyph in glyphs.iter().filter(|glyph| !glyph.outline.elements().is_empty()) {
        let bounds = glyph.outline.bounding_box();
        ascent = ascent.max(baseline - bounds.y0);
        descent = descent.max(bounds.y1 - baseline);
    }
    (ascent, descent)
}

fn grow_ruby_lines(cx: &mut Ctx<'_>, t: &TextObject, placed: Vec<PlacedRuby>, old_ascent: &[f64], old_descent: &[f64]) {
    let line_count = cx.out.lines.len();
    // Phase B: grow each line's box to fit its ruby, then move the lines on after it by what they
    // need in the block direction (the first line too for area type, so the ruby stays inside the
    // frame; point type's first baseline is its anchor and stays).
    let mut growth = vec![0.0_f64; line_count];
    for ruby in &placed {
        if let (Some(asc), Some(desc), Some(slot)) = (old_ascent.get(ruby.line), old_descent.get(ruby.line), growth.get_mut(ruby.line)) {
            let grown = (asc.max(ruby.ascent) + desc.max(ruby.descent)) - (asc + desc);
            *slot = slot.max(grown.max(0.0));
        }
    }
    // A line's ruby grows above it: the line moves on in the block direction by its own growth,
    // so the line before it keeps its room (for area type the first line moves too, keeping the
    // ruby inside the frame; point type's first baseline is its anchor and stays).
    let area = matches!(t.kind, TextKind::Area { .. });
    let mut shift = vec![0.0_f64; line_count];
    let mut cumulative = 0.0;
    for (index, amount) in growth.iter().copied().enumerate() {
        if area || index > 0 {
            cumulative += amount;
            shift[index] = cumulative;
        }
    }
    // Grow the lines' boxes to fit their ruby, then move them on in the block direction.
    for ruby in &placed {
        if let Some(line) = cx.out.lines.get_mut(ruby.line) {
            line.ascent = line.ascent.max(ruby.ascent);
            line.descent = line.descent.max(ruby.descent);
        }
    }
    for (index, line) in cx.out.lines.iter_mut().enumerate() {
        line.baseline += shift.get(index).copied().unwrap_or(0.0);
    }
    shift_base_glyphs(cx, &shift);
    append_ruby_glyphs(cx, placed, &shift);
}

fn shift_base_glyphs(cx: &mut Ctx<'_>, shift: &[f64]) {
    for glyph in &mut cx.out.glyphs {
        let amount = shift.get(glyph.line).copied().unwrap_or(0.0);
        if amount != 0.0 {
            glyph.origin.y += amount;
            glyph.outline.apply_affine(Affine::translate((0.0, amount)));
            glyph.xf = Affine::translate((0.0, amount)) * glyph.xf;
        }
    }
}

fn append_ruby_glyphs(cx: &mut Ctx<'_>, placed: Vec<PlacedRuby>, shift: &[f64]) {
    for ruby in placed {
        let amount = shift.get(ruby.line).copied().unwrap_or(0.0);
        for mut glyph in ruby.glyphs {
            if amount != 0.0 {
                glyph.origin.y += amount;
                glyph.outline.apply_affine(Affine::translate((0.0, amount)));
            }
            glyph.line = ruby.line;
            cx.out.ruby.push(glyph);
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
        unturned: Affine::IDENTITY,
    }
}
