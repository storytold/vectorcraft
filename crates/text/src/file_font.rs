//! Fonts files carried for type whose fonts aren't available here ([`vectorcraft_doc::FileFonts`]):
//! a TrueType font built from the glyphs a file drew, each with the outline and advance the file
//! gave it, for the character it shows. Built from outlines, it is the same whatever the file's
//! font program was (Type 1, CFF or TrueType, simple or CID-keyed): CFF's cubic curves become
//! quadratic ones within a quarter of a thousandth of an em.

use kurbo::{BezPath, PathEl, Shape};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::glyf::{Bbox, GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::head::{Flags, Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::tables::post::Post;
use write_fonts::types::{FWord, Fixed, GlyphId, Tag, UfWord, Version16Dot16};

/// Units per em of the fonts built: two per thousandth of an em, the unit of PDF glyph space, so
/// the widths a file gives in whole and half thousandths stay exact.
pub const FILE_FONT_UPEM: u16 = 2000;
/// Font units per thousandth of an em.
const SCALE: f64 = FILE_FONT_UPEM as f64 / 1000.0;
/// How far a quadratic curve may stray from the cubic it stands for (font units).
const ACCURACY: f64 = 0.5;
/// The most glyphs a font is built with: TrueType counts them in 16 bits, `.notdef` included.
pub const MAX_FILE_GLYPHS: usize = 65_534;
/// The furthest a point may lie from the glyph's origin (font units): further, the glyph is left
/// out (its character takes the substitute).
const MAX_COORD: f64 = 30_000.0;

/// A glyph a file drew: the character it shows, its outline (thousandths of an em, y up, as PDF
/// glyph space measures) and its advance (thousandths of an em).
#[derive(Clone, Debug)]
pub struct FileGlyph {
    pub ch: char,
    pub outline: BezPath,
    pub advance: f64,
}

/// What a font built from a file's glyphs is called and how it is set.
#[derive(Clone, Debug, Default)]
pub struct FileFontInfo {
    pub family: String,
    pub style: String,
    /// The PostScript name of the file's font (without a subset's tag).
    pub postscript: String,
    /// Ascent and descent (thousandths of an em, the descent below the baseline negative).
    pub ascent: f64,
    pub descent: f64,
    /// The file's font's licence flags (OS/2 `fsType`) when it says; else 4, print and preview,
    /// which lets an export embed it as the file did.
    pub fs_type: Option<u16>,
}

/// A TrueType font of `glyphs` (the first glyph of each character; at most [`MAX_FILE_GLYPHS`])
/// named and set as `info` says. A glyph whose outline can't be written is left out: its character
/// takes the substitute. `None` when no glyph can be written.
pub fn build_file_font(info: &FileFontInfo, glyphs: &[FileGlyph]) -> Option<Vec<u8>> {
    let mut seen = std::collections::HashSet::new();
    let mut made: Vec<(char, Glyph, u16)> = vec![];
    for g in glyphs {
        if made.len() >= MAX_FILE_GLYPHS {
            break;
        }
        if seen.contains(&g.ch) {
            continue;
        }
        let Some(glyph) = glyph(&g.outline) else { continue };
        let advance = (g.advance * SCALE).round();
        let advance = if advance.is_finite() { advance.clamp(0.0, f64::from(u16::MAX)) as u16 } else { 0 };
        seen.insert(g.ch);
        made.push((g.ch, glyph, advance));
    }
    if made.is_empty() {
        return None;
    }
    let notdef = (Glyph::Empty, FILE_FONT_UPEM / 2);
    let all = std::iter::once((&notdef.0, notdef.1)).chain(made.iter().map(|(_, g, a)| (g, *a)));
    let mut glyf = GlyfLocaBuilder::new();
    let mut metrics = vec![];
    let mut bounds: Option<Bbox> = None;
    let (mut max_points, mut max_contours) = (0usize, 0usize);
    let (mut min_lsb, mut min_rsb, mut max_extent, mut max_advance) = (i16::MAX, i16::MAX, i16::MIN, 0u16);
    for (g, advance) in all {
        glyf.add_glyph(g).ok()?;
        let b = match g {
            Glyph::Simple(s) => {
                max_points = max_points.max(s.contours.iter().map(|c| c.len()).sum());
                max_contours = max_contours.max(s.contours.len());
                Some(s.bbox)
            }
            _ => None,
        };
        let lsb = b.map_or(0, |b| b.x_min);
        if let Some(b) = b {
            bounds = Some(bounds.map_or(b, |u| u.union(b)));
            min_lsb = min_lsb.min(b.x_min);
            min_rsb = min_rsb.min((i32::from(advance) - i32::from(b.x_max)).clamp(i16::MIN.into(), i16::MAX.into()) as i16);
            max_extent = max_extent.max(b.x_max);
        }
        max_advance = max_advance.max(advance);
        metrics.push(LongMetric::new(advance, lsb));
    }
    let (glyf, loca, loca_format) = glyf.build();
    let count = u16::try_from(metrics.len()).ok()?;
    let bounds = bounds.unwrap_or_default();
    let units = |v: f64| {
        let v = (v * SCALE).round();
        if v.is_finite() { v.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16 } else { 0 }
    };
    // A font without them stands on an em from 0.88 up to 0.12 down, as PDF's vertical metrics do.
    let ascent = if info.ascent > 0.0 { units(info.ascent) } else { units(880.0) };
    let descent = if info.descent < 0.0 { units(info.descent) } else { units(-120.0) };
    let bold = info.style.to_ascii_lowercase().contains("bold");
    let italic = ["italic", "oblique"].iter().any(|s| info.style.to_ascii_lowercase().contains(s));
    let head = Head {
        font_revision: Fixed::from_f64(1.0),
        flags: Flags::from_bits_truncate(0b1011),
        units_per_em: FILE_FONT_UPEM,
        x_min: bounds.x_min,
        y_min: bounds.y_min,
        x_max: bounds.x_max,
        y_max: bounds.y_max,
        mac_style: MacStyle::from_bits_truncate(u16::from(bold) | (u16::from(italic) << 1)),
        lowest_rec_ppem: 6,
        index_to_loc_format: loca_format as i16,
        ..Head::default()
    };
    let hhea = Hhea {
        ascender: FWord::new(ascent),
        descender: FWord::new(descent),
        line_gap: FWord::new(0),
        advance_width_max: UfWord::new(max_advance),
        min_left_side_bearing: FWord::new(if min_lsb == i16::MAX { 0 } else { min_lsb }),
        min_right_side_bearing: FWord::new(if min_rsb == i16::MAX { 0 } else { min_rsb }),
        x_max_extent: FWord::new(if max_extent == i16::MIN { 0 } else { max_extent }),
        caret_slope_rise: 1,
        caret_slope_run: 0,
        caret_offset: 0,
        number_of_h_metrics: count,
    };
    let maxp = Maxp {
        num_glyphs: count,
        max_points: Some(u16::try_from(max_points).unwrap_or(u16::MAX)),
        max_contours: Some(u16::try_from(max_contours).unwrap_or(u16::MAX)),
        max_composite_points: Some(0),
        max_composite_contours: Some(0),
        max_zones: Some(1),
        max_twilight_points: Some(0),
        max_storage: Some(0),
        max_function_defs: Some(0),
        max_instruction_defs: Some(0),
        max_stack_elements: Some(0),
        max_size_of_instructions: Some(0),
        max_component_elements: Some(0),
        max_component_depth: Some(0),
    };
    let chars: Vec<u32> = made.iter().map(|(c, ..)| u32::from(*c)).collect();
    let first = chars.iter().copied().min().unwrap_or(0).min(0xffff) as u16;
    let last = chars.iter().copied().max().unwrap_or(0).min(0xffff) as u16;
    let advances: Vec<u32> = metrics.iter().skip(1).map(|m| u32::from(m.advance)).filter(|a| *a > 0).collect();
    let avg = advances.iter().sum::<u32>().checked_div(advances.len() as u32).unwrap_or(0);
    let selection = if bold || italic {
        u16::from(italic) | (u16::from(bold) << 5)
    } else {
        // Regular, and the typographic ascent and descent are the line's.
        0x40 | 0x80
    };
    let os2 = Os2 {
        x_avg_char_width: i16::try_from(avg).unwrap_or(i16::MAX),
        us_weight_class: if bold { 700 } else { 400 },
        us_width_class: 5,
        fs_type: info.fs_type.unwrap_or(4),
        y_strikeout_size: units(50.0),
        y_strikeout_position: units(250.0),
        ach_vend_id: Tag::new(b"NONE"),
        fs_selection: SelectionFlags::from_bits_truncate(selection),
        us_first_char_index: first,
        us_last_char_index: last,
        s_typo_ascender: ascent,
        s_typo_descender: descent,
        s_typo_line_gap: 0,
        us_win_ascent: u16::try_from(ascent.max(bounds.y_max)).unwrap_or(0),
        us_win_descent: u16::try_from(-i32::from(descent.min(bounds.y_min))).unwrap_or(0),
        ul_code_page_range_1: Some(0),
        ul_code_page_range_2: Some(0),
        sx_height: Some(0),
        s_cap_height: Some(0),
        us_default_char: Some(0),
        us_break_char: Some(32),
        us_max_context: Some(1),
        ..Os2::default()
    };
    let post = Post {
        version: Version16Dot16::VERSION_3_0,
        underline_position: FWord::new(units(-75.0)),
        underline_thickness: FWord::new(units(50.0)),
        ..Post::default()
    };
    let cmap = Cmap::from_mappings(made.iter().enumerate().map(|(i, (c, ..))| (*c, GlyphId::new(i as u32 + 1)))).ok()?;
    let hmtx = Hmtx::new(metrics, vec![]);
    let full = if info.style.is_empty() { info.family.clone() } else { format!("{} {}", info.family, info.style) };
    let name = name_table(&[(1, &info.family), (2, &info.style), (4, &full), (5, "Version 1.0"), (6, &postscript_name(&info.postscript))])?;
    let mut b = FontBuilder::new();
    b.add_table(&head).ok()?;
    b.add_table(&hhea).ok()?;
    b.add_table(&maxp).ok()?;
    b.add_table(&os2).ok()?;
    b.add_table(&hmtx).ok()?;
    b.add_table(&cmap).ok()?;
    b.add_table(&post).ok()?;
    b.add_table(&glyf).ok()?;
    b.add_table(&loca).ok()?;
    b.add_raw(Tag::new(b"name"), name);
    Some(b.build())
}

/// `outline` (thousandths of an em, y up) as a TrueType glyph in font units: its cubic curves as
/// quadratic ones, its contours that draw nothing (a lone point) left out. `Glyph::Empty` for a
/// glyph without ink (a space); `None` when it can't be written (points that aren't numbers or lie
/// too far out, a malformed path).
fn glyph(outline: &BezPath) -> Option<Glyph> {
    let at = |p: kurbo::Point| kurbo::Point::new(p.x * SCALE, p.y * SCALE);
    let mut out = BezPath::new();
    // The contour being read: its start, and its drawing commands (font units).
    let mut contour: Option<(kurbo::Point, Vec<PathEl>)> = None;
    let mut last = kurbo::Point::ZERO;
    let flush = |c: Option<(kurbo::Point, Vec<PathEl>)>, out: &mut BezPath| {
        if let Some((start, els)) = c.filter(|(_, els)| !els.is_empty()) {
            out.move_to(start);
            for e in els {
                out.push(e);
            }
            out.close_path();
        }
    };
    // A point that isn't a number can't be written (nor bounded: `min`/`max` skip NaN).
    let finite = |el: &PathEl| match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => p.is_finite(),
        PathEl::QuadTo(a, p) => a.is_finite() && p.is_finite(),
        PathEl::CurveTo(a, b, p) => a.is_finite() && b.is_finite() && p.is_finite(),
        PathEl::ClosePath => true,
    };
    if !outline.elements().iter().all(finite) {
        return None;
    }
    for el in outline.elements() {
        let p = match *el {
            PathEl::MoveTo(p) => {
                flush(contour.take(), &mut out);
                last = at(p);
                contour = Some((last, vec![]));
                continue;
            }
            PathEl::ClosePath => {
                if let Some((start, _)) = &contour {
                    last = *start;
                }
                flush(contour.take(), &mut out);
                continue;
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => at(p),
        };
        // A drawing command without a move goes on from where the pen is.
        let (_, els) = contour.get_or_insert((last, vec![]));
        match *el {
            PathEl::QuadTo(c, _) => els.push(PathEl::QuadTo(at(c), p)),
            PathEl::CurveTo(c1, c2, _) => {
                let cubic = kurbo::CubicBez::new(last, at(c1), at(c2), p);
                els.extend(cubic.to_quads(ACCURACY).map(|(.., q)| PathEl::QuadTo(q.p1, q.p2)));
            }
            _ => els.push(PathEl::LineTo(p)),
        }
        last = p;
    }
    flush(contour.take(), &mut out);
    if out.elements().is_empty() {
        return Some(Glyph::Empty);
    }
    let b = out.bounding_box();
    if !(b.is_finite() && b.x0 > -MAX_COORD && b.y0 > -MAX_COORD && b.x1 < MAX_COORD && b.y1 < MAX_COORD) {
        return None;
    }
    SimpleGlyph::from_bezpath(&out).ok().map(Glyph::Simple)
}

/// `name` as a PostScript name: printable ASCII other than the characters PostScript reserves, at
/// most 63 of them ("Font" when none is left).
fn postscript_name(name: &str) -> String {
    let s: String = name.chars().filter(|c| c.is_ascii_graphic() && !"[](){}<>/%".contains(*c)).take(63).collect();
    if s.is_empty() { "Font".to_string() } else { s }
}

/// A `name` table of Windows English (US) records (name id, string).
pub(crate) fn name_table(records: &[(u16, &str)]) -> Option<Vec<u8>> {
    let mut records = records.to_vec();
    records.sort_by_key(|r| r.0);
    let n = u16::try_from(records.len()).ok()?;
    let (mut head, mut strings) = (Vec::new(), Vec::new());
    for v in [0, n, 6 + 12 * n] {
        head.extend_from_slice(&v.to_be_bytes());
    }
    for (id, s) in records {
        let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3, 1, 0x0409, id, u16::try_from(bytes.len()).ok()?, u16::try_from(strings.len()).ok()?] {
            head.extend_from_slice(&v.to_be_bytes());
        }
        strings.extend(bytes);
    }
    head.extend(strings);
    Some(head)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kurbo::{Affine, Rect};
    use skrifa::MetadataProvider;

    use super::*;
    use crate::FontDb;

    /// Source Serif 4's glyphs of `text` as a PDF draws them (thousandths of an em, y up).
    fn glyphs(text: &str) -> Vec<FileGlyph> {
        let db = FontDb::global();
        let face = db.face("Source Serif 4", "Regular").unwrap();
        let k = 1000.0 / face.units_per_em();
        text.chars()
            .map(|ch| {
                let gid = face.glyph_for(ch);
                // The database's outlines run y down.
                let outline = Affine::scale_non_uniform(k, -k) * db.outline(&face, gid).as_ref().clone();
                FileGlyph { ch, outline, advance: face.advance(gid) * k }
            })
            .collect()
    }

    fn info() -> FileFontInfo {
        FileFontInfo { family: "Kept Serif".into(), style: "Regular".into(), postscript: "KeptSerif-Regular".into(), ..Default::default() }
    }

    fn near(a: Rect, b: Rect, tol: f64) -> bool {
        (a.x0 - b.x0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.y1 - b.y1).abs() < tol
    }

    #[test]
    fn a_font_built_from_a_files_glyphs_draws_them_with_their_widths() {
        let from = glyphs("Hamburg é&Q ");
        let bytes = build_file_font(&info(), &from).unwrap();
        let font = skrifa::FontRef::new(&bytes).unwrap();
        assert_eq!(font.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default()).units_per_em, FILE_FONT_UPEM);
        let db = FontDb::with_font_dirs(vec![]);
        assert!(db.keep_file_font("kept", Arc::new(bytes)));
        let face = db.file_font("kept").unwrap();
        assert_eq!((face.family.as_str(), face.style.as_str()), ("Kept Serif", "Regular"));
        assert_eq!(face.postscript_name(), "KeptSerif-Regular");
        // Kept apart: no family name finds it.
        assert!(!db.has_family("Kept Serif"));
        let k = 1000.0 / face.units_per_em();
        for g in &from {
            let gid = face.glyph_for(g.ch);
            assert_ne!(gid, 0, "{}", g.ch);
            assert!((face.advance(gid) * k - g.advance).abs() < 0.5, "{}: the file's width", g.ch);
            let drawn = Affine::scale_non_uniform(k, -k) * db.outline(&face, gid).as_ref().clone();
            if g.outline.elements().is_empty() {
                assert!(drawn.elements().is_empty());
            } else {
                assert!(
                    near(drawn.bounding_box(), g.outline.bounding_box(), 0.6),
                    "{}: {:?} vs {:?}",
                    g.ch,
                    drawn.bounding_box(),
                    g.outline.bounding_box()
                );
            }
        }
    }

    #[test]
    fn glyphs_that_cannot_be_written_are_left_out() {
        let mut from = glyphs("ab");
        // A second glyph for `a`: the first is kept.
        from.push(FileGlyph { ch: 'a', outline: glyphs("x").remove(0).outline, advance: 999.0 });
        let mut nan = BezPath::new();
        nan.move_to((0.0, 0.0));
        nan.line_to((f64::NAN, 10.0));
        nan.line_to((10.0, 10.0));
        from.push(FileGlyph { ch: 'c', outline: nan, advance: 500.0 });
        let mut far = BezPath::new();
        far.move_to((0.0, 0.0));
        far.line_to((1e9, 0.0));
        far.line_to((0.0, 10.0));
        from.push(FileGlyph { ch: 'd', outline: far, advance: 500.0 });
        // A lone point draws nothing; an advance that isn't a number is none.
        let mut dot = BezPath::new();
        dot.move_to((5.0, 5.0));
        from.push(FileGlyph { ch: 'e', outline: dot, advance: f64::NAN });
        let db = FontDb::with_font_dirs(vec![]);
        assert!(db.keep_file_font("kept", Arc::new(build_file_font(&info(), &from).unwrap())));
        let face = db.file_font("kept").unwrap();
        assert!(face.covers('a') && face.covers('b') && face.covers('e'));
        assert!(!face.covers('c') && !face.covers('d'));
        assert!((face.advance(face.glyph_for('a')) - from[0].advance * 2.0).abs() < 1.0, "the first glyph of a character");
        assert_eq!(face.advance(face.glyph_for('e')), 0.0);
        assert!(db.outline(&face, face.glyph_for('e')).elements().is_empty());
        assert!(build_file_font(&info(), &[]).is_none());
        // Bytes that aren't a font are kept by no one.
        assert!(!db.keep_file_font("junk", Arc::new(b"not a font".to_vec())));
        assert!(db.file_font("junk").is_none());
    }

    #[test]
    fn extreme_negative_descent_does_not_overflow() {
        let mut i = info();
        i.descent = -100_000.0;
        let bytes = build_file_font(&i, &glyphs("a"));
        assert!(bytes.is_some());
    }
}
