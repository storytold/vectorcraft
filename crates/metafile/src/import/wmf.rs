//! WMF records (MS-WMF) played on a [`Player`]. A placeable header gives the picture's box in
//! logical units and the units an inch; without one, the first window origin and extent are the
//! box, at 1440 units an inch.

use vectorcraft_geom::{Affine, Point, Rect, shapes};

use super::{ArcKind, BrushObj, FontObj, Obj, PenObj, Player};
use crate::bytes::Reader;
use crate::dib::{self, Alpha};
use crate::wmf::PLACEABLE_KEY;
use crate::{Imported, Kind};

/// Units an inch when the file doesn't say (twips).
const DEFAULT_INCH: f64 = 1440.0;
/// Records that change nothing VectorCraft keeps (background, raster and mapping details, palettes,
/// escapes).
const IGNORED: &[u16] = &[
    0x0102, 0x0103, 0x0104, 0x0105, 0x0107, 0x0108, 0x0201, 0x020A, 0x020B, 0x020C, 0x020D, 0x020E, 0x020F, 0x0211, 0x0410, 0x0412, 0x0231, 0x0234,
    0x0035, 0x0037, 0x0139, 0x0436, 0x0626,
];

fn point(r: &mut Reader) -> Option<Point> {
    Some(Point::new(f64::from(r.i16()?), f64::from(r.i16()?)))
}

/// A point stored y first.
fn point_yx(r: &mut Reader) -> Option<Point> {
    let y = f64::from(r.i16()?);
    Some(Point::new(f64::from(r.i16()?), y))
}

/// A rectangle stored bottom, right, top, left.
fn rect_brtl(r: &mut Reader) -> Option<Rect> {
    let (b, rt, t, l) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
    Some(Rect::new(f64::from(l), f64::from(t), f64::from(rt), f64::from(b)))
}

fn color(r: &mut Reader) -> Option<[u8; 3]> {
    let [red, g, b, _] = r.u32()?.to_le_bytes();
    Some([red, g, b])
}

fn points(r: &mut Reader, n: usize) -> Option<Vec<Point>> {
    if r.rest().len() < n * 4 {
        return None;
    }
    (0..n).map(|_| point(r)).collect()
}

/// ANSI text (read as Latin-1).
fn ansi(b: &[u8]) -> String {
    b.iter().map(|c| char::from(*c)).collect()
}

/// The records' start, and the box and units an inch of a placeable header.
fn header(bytes: &[u8]) -> Option<(usize, Option<(Rect, f64)>)> {
    let mut r = Reader::new(bytes);
    let placeable = if r.u32()? == PLACEABLE_KEY {
        r.skip(2)?;
        let (l, t, rt, b) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
        let inch = r.u16()?;
        let inch = if inch == 0 { DEFAULT_INCH } else { f64::from(inch) };
        Some((Rect::new(f64::from(l), f64::from(t), f64::from(rt), f64::from(b)).abs(), inch))
    } else {
        None
    };
    let start = if placeable.is_some() { 22 } else { 0 };
    let mut h = Reader::new(bytes.get(start..)?);
    h.skip(2)?;
    let words = usize::from(h.u16()?);
    if words < 9 {
        return None;
    }
    Some((start + words * 2, placeable))
}

/// The first window origin and extent, as the picture's box when there is no placeable header.
fn window(bytes: &[u8], mut at: usize) -> Option<Rect> {
    let (mut org, mut ext) = (None, None);
    while let Some(head) = bytes.get(at..at + 6) {
        let words = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let function = u16::from_le_bytes([head[4], head[5]]);
        if words < 3 || function == 0 {
            break;
        }
        let mut r = Reader::new(bytes.get(at + 6..).unwrap_or_default());
        match function {
            0x020B if org.is_none() => org = point_yx(&mut r),
            0x020C if ext.is_none() => ext = point_yx(&mut r),
            _ => {}
        }
        if org.is_some() && ext.is_some() {
            break;
        }
        at = at.saturating_add(words.saturating_mul(2));
    }
    let o = org.unwrap_or(Point::ZERO);
    let e = ext?;
    Some(Rect::new(o.x, o.y, o.x + e.x, o.y + e.y).abs())
}

/// Logical → document for a placeable file whose window (`org`, `ext`) is mapped onto its box `b`;
/// a negative extent flips that axis, as in GDI.
fn window_map(b: Rect, k: f64, org: Point, ext: Point) -> Affine {
    let ratio = |box_len: f64, e: f64| if e.abs() > 1e-12 { box_len / e } else { 1.0 };
    Affine::scale_non_uniform(k * ratio(b.width(), ext.x), k * ratio(b.height(), ext.y)) * Affine::translate(-org.to_vec2())
}

pub(super) fn play(bytes: &[u8]) -> Result<Imported, String> {
    let (start, placeable) = header(bytes).ok_or("the WMF header is damaged")?;
    let (frame, inch) = match placeable {
        Some((b, inch)) if b.width() > 0.0 && b.height() > 0.0 => (Some(b), inch),
        _ => (window(bytes, start).filter(|w| w.width() > 0.0 && w.height() > 0.0), DEFAULT_INCH),
    };
    let k = 72.0 / inch;
    let origin = frame.map_or(Point::ZERO, |f| f.origin());
    let artboard = frame.map_or(Rect::new(0.0, 0.0, 1.0, 1.0), |f| Rect::new(0.0, 0.0, f.width() * k, f.height() * k));
    let mut p = Player::new(Kind::Wmf, Affine::scale(k) * Affine::translate(-origin.to_vec2()), 25.4 / inch, artboard);
    // With a placeable header, the window records map the window onto the box.
    let placed = placeable.map(|(b, _)| b).filter(|b| b.width() > 0.0 && b.height() > 0.0);
    let mut win = (placed.map_or(Point::ZERO, |b| b.origin()), placed.map_or(Point::ZERO, |b| Point::new(b.width(), b.height())));
    let mut at = start;
    let mut ended = false;
    while let Some(head) = bytes.get(at..at + 6) {
        let words = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let function = u16::from_le_bytes([head[4], head[5]]);
        if function == 0 {
            ended = true;
            break;
        }
        let Some(rec) = words.checked_mul(2).and_then(|n| bytes.get(at..at.checked_add(n)?)).filter(|_| words >= 3) else { break };
        if !matches!(function, 0x0213 | 0x0214) {
            p.flush();
        }
        if let (Some(b), 0x020B | 0x020C) = (placed, function) {
            if let Some(v) = point_yx(&mut Reader::new(rec.get(6..).unwrap_or_default())) {
                if function == 0x020B {
                    win.0 = v;
                } else {
                    win.1 = v;
                }
                p.dev_to_doc = window_map(b, k, win.0, win.1);
            }
        } else if record(&mut p, function, rec.get(6..).unwrap_or_default()).is_none() {
            p.skip();
        }
        at += rec.len();
    }
    if !ended {
        p.warn("the file ends early: what it held was imported");
    }
    let mut out = p.finish();
    // No box given: the art's bounds.
    if frame.is_none()
        && let Some(b) = out.document.layers.first().and_then(|l| vectorcraft_doc::live::nodes_bounds(l.children().map_or(&[][..], |c| c.as_slice())))
        && let Some(a) = out.document.artboards.first_mut()
    {
        a.rect = Rect::from_origin_size(b.origin(), (b.width().max(1.0), b.height().max(1.0)));
    }
    Ok(out)
}

/// An image record's source rectangle (from the bottom for a bottom-up DIB) and destination.
fn blit(p: &mut Player, rop: u32, src: (i16, i16, i16, i16), dest: (i16, i16, i16, i16), packed: Option<&[u8]>) {
    let (xd, yd, wd, hd) = (f64::from(dest.0), f64::from(dest.1), f64::from(dest.2), f64::from(dest.3));
    let d = Rect::new(xd, yd, xd + wd, yd + hd);
    let Some(packed) = packed else { return p.pattern_blit(d, rop) };
    let parts = dib::split(packed);
    let img = parts.ok_or_else(|| "damaged".to_string()).and_then(|(bmi, bits)| dib::decode(bmi, bits, Alpha::Ignore, &mut p.pixels_left));
    let bottom_up = parts.and_then(|(bmi, _)| bmi.get(8..12)).is_some_and(|h| i32::from_le_bytes([h[0], h[1], h[2], h[3]]) > 0);
    let ih = img.as_ref().map_or(0.0, |i| f64::from(i.height));
    let (xs, ys, ws, hs) = (f64::from(src.0), f64::from(src.1), f64::from(src.2), f64::from(src.3));
    let top = if bottom_up { ih - ys - hs } else { ys };
    p.image(img, Rect::new(xs, top, xs + ws, top + hs), d, rop, 1.0);
}

/// Play one record (`r`: its parameters). `None`: unknown or damaged.
fn record(p: &mut Player, function: u16, body: &[u8]) -> Option<()> {
    let mut r = Reader::new(body);
    match function {
        0x001E => p.save(),
        0x0127 => p.restore(i32::from(r.i16()?)),
        0x0106 => p.set_fill_mode(r.u16()? == 1),
        0x0209 => p.set_text_color(color(&mut r)?),
        0x012E => p.set_text_align(u32::from(r.u16()?)),
        0x0214 => p.move_to(point_yx(&mut r)?),
        0x0213 => {
            let pt = point_yx(&mut r)?;
            p.poly_to(&[pt], false);
        }
        0x0324 | 0x0325 => {
            let n = usize::try_from(r.i16()?).ok()?;
            let pts = points(&mut r, n)?;
            p.poly(&pts, function == 0x0324, false);
        }
        0x0538 => {
            let n = usize::from(r.u16()?);
            if r.rest().len() < n * 2 {
                return None;
            }
            let counts: Vec<usize> = (0..n).map(|_| r.u16().map(usize::from)).collect::<Option<_>>()?;
            let mut polys = Vec::with_capacity(n);
            for c in counts {
                polys.push(points(&mut r, c)?);
            }
            p.poly_poly(&polys, true);
        }
        0x041B => p.figure(shapes::rectangle(rect_brtl(&mut r)?.abs()).to_bezpath(), true),
        0x0418 => p.figure(shapes::ellipse(rect_brtl(&mut r)?.abs()).to_bezpath(), true),
        0x061C => {
            let (h, w) = (f64::from(r.i16()?), f64::from(r.i16()?));
            p.figure(super::round_rect(rect_brtl(&mut r)?, w, h), true);
        }
        0x0817 | 0x081A | 0x0830 => {
            let (end, start) = (point_yx(&mut r)?, point_yx(&mut r)?);
            let b = rect_brtl(&mut r)?;
            let kind = match function {
                0x0817 => ArcKind::Open,
                0x081A => ArcKind::Pie,
                _ => ArcKind::Chord,
            };
            p.arc(b, start, end, kind);
        }
        0x0416 => p.clip_rect(rect_brtl(&mut r)?),
        0x0415 => p.exclude_clip(),
        0x012D => p.select(u32::from(r.u16()?)),
        0x01F0 => p.delete(u32::from(r.u16()?)),
        0x02FA => {
            let style = u32::from(r.u16()?);
            let width = f64::from(r.i16()?);
            r.skip(2)?;
            p.create(Obj::Pen(PenObj { style, width, color: color(&mut r)?, entries: vec![] }));
        }
        0x02FC => {
            let style = u32::from(r.u16()?);
            p.create(Obj::Brush(BrushObj::from_log(style, color(&mut r)?)));
        }
        0x02FB => {
            let height = f64::from(r.i16()?);
            r.skip(2)?;
            let escapement = f64::from(r.i16()?);
            r.skip(2)?;
            let weight = i32::from(r.i16()?);
            let italic = r.bytes(1)?.first().is_some_and(|b| *b != 0);
            r.skip(7)?;
            let face: Vec<u8> = r.rest().iter().take(32).take_while(|b| **b != 0).copied().collect();
            p.create(Obj::Font(FontObj { height, escapement, weight, italic, face: ansi(&face) }));
        }
        0x00F7 | 0x06FF => p.create(Obj::Other),
        0x01F9 | 0x0142 => p.create(Obj::Brush(BrushObj::Pattern)),
        0x0521 => {
            let n = usize::try_from(r.i16()?).ok()?;
            let s = ansi(r.bytes(n)?);
            r.skip(n % 2)?;
            let at = point_yx(&mut r)?;
            p.text(at, &s);
        }
        0x0A32 => {
            let at = point_yx(&mut r)?;
            let n = usize::try_from(r.i16()?).ok()?;
            let opts = r.u16()?;
            // ETO_OPAQUE or ETO_CLIPPED: a rectangle comes first.
            if opts & 0x6 != 0 {
                r.skip(8)?;
            }
            let s = ansi(r.bytes(n)?);
            p.text(at, &s);
        }
        0x0F43 => {
            let rop = r.u32()?;
            r.skip(2)?;
            let (hs, ws, ys, xs) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            let (hd, wd, yd, xd) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            blit(p, rop, (xs, ys, ws, hs), (xd, yd, wd, hd), Some(r.rest()));
        }
        0x0B41 => {
            let rop = r.u32()?;
            let (hs, ws, ys, xs) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            // Without a bitmap, a reserved word comes before the destination.
            let with_bitmap = body.len() > 22;
            if !with_bitmap {
                r.skip(2)?;
            }
            let (hd, wd, yd, xd) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            blit(p, rop, (xs, ys, ws, hs), (xd, yd, wd, hd), with_bitmap.then(|| r.rest()));
        }
        0x0940 => {
            let rop = r.u32()?;
            let (ys, xs) = (r.i16()?, r.i16()?);
            let with_bitmap = body.len() > 20;
            if !with_bitmap {
                r.skip(2)?;
            }
            let (h, w, yd, xd) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            blit(p, rop, (xs, ys, w, h), (xd, yd, w, h), with_bitmap.then(|| r.rest()));
        }
        0x061D => {
            let rop = r.u32()?;
            let (h, w, y, x) = (f64::from(r.i16()?), f64::from(r.i16()?), f64::from(r.i16()?), f64::from(r.i16()?));
            p.pattern_blit(Rect::new(x, y, x + w, y + h), rop);
        }
        f if IGNORED.contains(&f) => {}
        _ => return None,
    }
    Some(())
}
