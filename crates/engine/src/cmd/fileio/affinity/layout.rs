use super::nodes::MAX_DEPTH;
use super::*;

/// The master page instances among `nodes` (also inside layers and groups): (master, first page,
/// first master page, page count).
pub(super) fn master_instances(nodes: &[af::Node]) -> Vec<(usize, usize, usize, usize)> {
    fn visit(n: &af::Node, depth: usize, out: &mut Vec<(usize, usize, usize, usize)>) {
        if depth > MAX_DEPTH || !n.visible {
            return;
        }
        match n.kind {
            Kind::MasterInstance { master, page, master_page, count } => out.push((master, page, master_page, count)),
            Kind::Layer | Kind::Group => n.children.iter().for_each(|c| visit(c, depth + 1, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    nodes.iter().for_each(|n| visit(n, 0, &mut out));
    out
}

/// Which master page instance each of `pages` pages shows: (master, first page, first master
/// page) of the first instance in `instances` that covers it. Each page is settled once (the
/// next page still open is found by a path-halving jump), so this is about linear in pages and
/// instances however much their ranges overlap.
pub(super) fn page_masters(instances: &[(usize, usize, usize, usize)], pages: usize) -> Vec<Option<(usize, usize, usize)>> {
    fn open_from(next: &mut [usize], mut i: usize) -> usize {
        while let Some(&n) = next.get(i) {
            if n == i {
                break;
            }
            let skip = next.get(n).copied().unwrap_or(n);
            if let Some(slot) = next.get_mut(i) {
                *slot = skip;
            }
            i = n;
        }
        i
    }
    let mut owner = vec![None; pages];
    // `next[p]` leads towards the first page from `p` on with no master yet (`pages`: none).
    let mut next: Vec<usize> = (0..=pages).collect();
    for &(master, first, master_page, count) in instances {
        let end = first.saturating_add(count).min(pages);
        let mut p = open_from(&mut next, first.min(pages));
        while p < end {
            if let Some(o) = owner.get_mut(p) {
                *o = Some((master, first, master_page));
            }
            if let Some(n) = next.get_mut(p) {
                *n = p + 1;
            }
            p = open_from(&mut next, p + 1);
        }
    }
    owner
}

/// Master names: A, B … Z, then A1, B1 ….
pub(super) fn master_prefix(index: usize) -> String {
    let letter = char::from(b'A' + (index % 26) as u8);
    match index / 26 {
        0 => letter.to_string(),
        n => format!("{letter}{n}"),
    }
}

/// A table cell's text `w` × `h` at the origin, placed as Affinity places it. Affinity aligns the
/// block from the first line's capitals' tops to the last baseline in the cell, and draws it even
/// where it doesn't fit (centred cells measured on a document's thumbnail: two lines in a cell
/// shorter than them reach out of it at both ends). VectorCraft aligns the block down to the last
/// line's descenders and keeps lines inside the frame: the frame moves by what this returns (down
/// the cell) and grows where the lines need more room.
pub(super) fn place_cell(obj: &mut TextObject, w: f64, h: f64) -> f64 {
    let share = match obj.area.vertical_align {
        VerticalAlign::Center => 0.5,
        VerticalAlign::Bottom => 1.0,
        _ => 0.0,
    };
    let mut tall = obj.clone();
    tall.area.vertical_align = VerticalAlign::Top;
    tall.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h * 4.0 + 10_000.0)) };
    let lines = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tall).lines;
    let Some(last) = lines.iter().rfind(|l| l.region == 0) else { return 0.0 };
    let (baseline, descent) = (last.baseline, last.descent);
    if !(baseline.is_finite() && descent.is_finite() && h.is_finite()) {
        return 0.0;
    }
    // (A little more than the lines need, for the rounding of the layout.)
    let frame = h.max(baseline + descent + 0.05);
    obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, frame)) };
    // Where Affinity has the block's top, less where VectorCraft puts it in the frame.
    (h - baseline) * share - (frame - baseline - descent) * share
}

/// Area type `w` × `h` at the origin whose lines don't all fit. Affinity shows each line whose
/// baseline is inside the frame, its descenders hanging out of it; VectorCraft's area type leaves
/// such a line out. The frame is made taller, from its top, by what those lines need. Whether it was.
pub(super) fn fit_lines(obj: &mut TextObject, w: f64, h: f64) -> bool {
    let db = vectorcraft_text::FontDb::global();
    if !(w > 0.0 && h > 0.0 && h.is_finite()) {
        return false;
    }
    let first = vectorcraft_text::layout(db, obj);
    if !first.overflow {
        return false;
    }
    // Where the lines go in a frame with room for all of them.
    let mut tall = obj.clone();
    tall.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, h * 4.0 + 10_000.0)) };
    let need = vectorcraft_text::layout(db, &tall)
        .lines
        .iter()
        .filter(|l| l.region == 0 && l.baseline <= h + 1e-6)
        .map(|l| l.baseline + l.descent)
        .filter(|b| b.is_finite())
        .fold(0.0, f64::max);
    if need <= h {
        return false;
    }
    let before = first.lines.len();
    // (A little more than they need, for the rounding of the layout.)
    let short = std::mem::replace(&mut obj.kind, TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, w, need + 0.05)) });
    // Something else keeps the lines out: as the source has it.
    if vectorcraft_text::layout(db, obj).lines.len() <= before {
        obj.kind = short;
        return false;
    }
    true
}

/// RGBA pixels as a PNG file.
pub(super) fn png(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let buf = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = Vec::new();
    buf.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// Where a line of `w` starts in a span from 0 to `room` (as a share of `room - w`).
fn share(j: Justify) -> f64 {
    match j {
        Justify::Center | Justify::JustifyCenter => 0.5,
        Justify::Right | Justify::JustifyRight => 1.0,
        _ => 0.0,
    }
}

/// Artistic text whose paragraphs are aligned differently. Affinity aligns each line within the
/// width of the longest, and places that block at the anchor by the text's own alignment (its
/// first paragraph's); VectorCraft's point type aligns each line on the anchor. Indents move the
/// lines to where Affinity has them and keep their alignment: a centred line centres between its
/// indents and a right-aligned one ends at its right indent, whatever its width.
pub(super) fn align_lines(obj: &mut TextObject) {
    let block = share(obj.para.justify);
    if obj.paras.iter().all(|p| share(p.justify) == block) {
        return;
    }
    let lines = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), obj).lines;
    let w = lines.iter().map(|l| l.x1 - l.x0).filter(|w| w.is_finite()).fold(0.0, f64::max);
    if !(w > 0.0 && w < 1e6) {
        return;
    }
    // The block's left edge, from the anchor.
    let left = -w * block;
    for p in &mut obj.paras {
        let at = share(p.justify);
        let shift = left + w * at;
        // A left-aligned line starts at its left indent, a right-aligned one ends at minus its right
        // indent, and a centred one centres on half their difference.
        if at < 0.25 {
            p.left_indent += shift;
        } else if at > 0.75 {
            p.right_indent -= shift;
        } else {
            p.left_indent += shift;
            p.right_indent -= shift;
        }
    }
    if let Some(first) = obj.paras.first() {
        obj.para = first.clone();
    }
}

pub(super) fn justify(a: AfAlign) -> Justify {
    match a {
        AfAlign::Left => Justify::Left,
        AfAlign::Center => Justify::Center,
        AfAlign::Right => Justify::Right,
        AfAlign::Justify => Justify::JustifyLeft,
    }
}

/// A font's line gap in em, as VectorCraft will set it (the installed face, or the fallback for
/// a missing one); 0 when no font is loaded.
pub(super) fn line_gap(family: &str, style: &str) -> f64 {
    let defaults = CharStyle::default();
    let family = if family.is_empty() { defaults.font_family.as_str() } else { family };
    let style = if style.is_empty() { defaults.font_style.as_str() } else { style };
    let Some(face) = vectorcraft_text::FontDb::global().face(family, style) else { return 0.0 };
    let upem = face.units_per_em();
    let gap = if upem > 0.0 { face.line_gap() / upem } else { 0.0 };
    if gap.is_finite() { gap.clamp(0.0, 2.0) } else { 0.0 }
}

pub(super) fn font_style(weight: i64, italic: bool) -> String {
    let w = match weight {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=325 => "Light",
        326..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}
