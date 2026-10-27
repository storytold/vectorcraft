//! EMF records (MS-EMF) played on a [`Player`]: the header gives the frame (the artboard) and the
//! reference device (device pixels → millimetres), then every record in turn until EMR_EOF.

use vectorcraft_geom::{Affine, Point, Rect};

use super::{ArcKind, BrushObj, FontObj, Obj, PenObj, Player};
use crate::bytes::{Reader, slice};
use crate::dib::{self, Alpha};
use crate::{Imported, Kind};

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// Most points one record may give (a hostile count can't make a huge allocation).
const MAX_POINTS: usize = 1 << 22;
/// Records that change nothing VectorCraft keeps (background mode and colour, raster modes,
/// palettes, colour management, comments…).
const IGNORED: &[u32] =
    &[1, 13, 15, 16, 18, 20, 21, 23, 25, 28, 48, 50, 51, 52, 65, 66, 70, 98, 100, 101, 104, 110, 111, 112, 113, 115, 119, 120, 121];

fn rect(r: &mut Reader) -> Option<Rect> {
    Some(Rect::new(f64::from(r.i32()?), f64::from(r.i32()?), f64::from(r.i32()?), f64::from(r.i32()?)))
}

fn point32(r: &mut Reader) -> Option<Point> {
    Some(Point::new(f64::from(r.i32()?), f64::from(r.i32()?)))
}

fn point16(r: &mut Reader) -> Option<Point> {
    Some(Point::new(f64::from(r.i16()?), f64::from(r.i16()?)))
}

fn color(r: &mut Reader) -> Option<[u8; 3]> {
    let [red, g, b, _] = r.u32()?.to_le_bytes();
    Some([red, g, b])
}

fn xform(r: &mut Reader) -> Option<Affine> {
    let mut v = [0.0; 6];
    for x in &mut v {
        *x = f64::from(r.f32()?);
    }
    Some(Affine::new(v))
}

/// `n` points (32-bit or 16-bit), `None` when the record is shorter.
fn points(r: &mut Reader, n: u32, wide: bool) -> Option<Vec<Point>> {
    let n = n as usize;
    if n > MAX_POINTS || r.rest().len() < n * if wide { 8 } else { 4 } {
        return None;
    }
    (0..n).map(|_| if wide { point32(r) } else { point16(r) }).collect()
}

/// The polygons of a PolyPolygon / PolyPolyline record.
fn polys(r: &mut Reader, wide: bool) -> Option<Vec<Vec<Point>>> {
    r.skip(16)?;
    let n = r.u32()? as usize;
    let total = r.u32()?;
    if n > MAX_POINTS || r.rest().len() < n * 4 {
        return None;
    }
    let counts: Vec<u32> = (0..n).map(|_| r.u32()).collect::<Option<_>>()?;
    let mut all = points(r, total, wide)?.into_iter();
    Some(counts.iter().map(|c| all.by_ref().take(*c as usize).collect()).collect())
}

/// A LOGFONTW → the font.
fn font(r: &mut Reader) -> Option<FontObj> {
    let height = f64::from(r.i32()?);
    r.skip(4)?;
    let escapement = f64::from(r.i32()?);
    r.skip(4)?;
    let weight = r.i32()?;
    let italic = r.bytes(1)?.first().is_some_and(|b| *b != 0);
    r.skip(7)?;
    let units: Vec<u16> = (0..32).map_while(|_| r.u16()).take_while(|c| *c != 0).collect();
    Some(FontObj { height, escapement, weight, italic, face: String::from_utf16_lossy(&units) })
}

/// The bitmap of an image record: its BITMAPINFO and bits at the offsets the record gives.
fn bitmap(rec: &[u8], off_bmi: u32, cb_bmi: u32, off_bits: u32, cb_bits: u32, alpha: Alpha, left: &mut u64) -> Result<dib::Rgba, String> {
    let bmi = slice(rec, off_bmi, cb_bmi).ok_or("damaged")?;
    let bits = slice(rec, off_bits, cb_bits).ok_or("damaged")?;
    dib::decode(bmi, bits, alpha, left)
}

/// The rows a DIB's source rectangle counts from the bottom in (StretchDIBits on a bottom-up DIB).
fn bottom_up(rec: &[u8], off_bmi: u32) -> bool {
    let at = off_bmi as usize;
    let size = rec.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let height = rec.get(at + 8..at + 12).and_then(|b| b.try_into().ok()).map(i32::from_le_bytes);
    size.is_some_and(|s| s >= 40) && height.is_some_and(|h| h > 0)
}

/// The source rectangle `(x, y, w, h)` of an image of `img`'s height, counted from the top.
fn source(x: i32, y: i32, w: i32, h: i32, img: &Result<dib::Rgba, String>, from_bottom: bool) -> Rect {
    let ih = img.as_ref().map_or(0.0, |i| f64::from(i.height));
    let (x, y, w, h) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h));
    let top = if from_bottom { ih - y - h } else { y };
    Rect::new(x, top, x + w, top + h)
}

/// The size and placement of the picture: (device → document, millimetres a pixel, the artboard).
fn frame(bytes: &[u8]) -> Option<(Affine, f64, Rect)> {
    let mut r = Reader::new(bytes);
    r.skip(4)?;
    let header_size = r.u32()?;
    let bounds = rect(&mut r)?;
    let frame = rect(&mut r)?;
    // Signature, version, bytes, records, handles, description and palette size.
    r.skip(32)?;
    let device = (f64::from(r.i32()?), f64::from(r.i32()?));
    let mm = (f64::from(r.i32()?), f64::from(r.i32()?));
    // The device in micrometres (more precise), when the header has it.
    let micro = if header_size >= 108 {
        r.skip(12)?;
        Some((f64::from(r.u32()?), f64::from(r.u32()?)))
    } else {
        None
    };
    let ok = |v: f64| v.is_finite() && v > 0.0;
    let mm_per_px = match micro {
        Some((mx, _)) if ok(mx) && ok(device.0) => mx / 1000.0 / device.0,
        _ if ok(mm.0) && ok(device.0) => mm.0 / device.0,
        _ => 25.4 / 96.0,
    };
    let mm_per_px = if mm_per_px.is_finite() && mm_per_px > 1e-9 { mm_per_px } else { 25.4 / 96.0 };
    let k = mm_per_px * PT_PER_MM;
    // The frame (0.01 mm) in device pixels; an empty frame falls back to the bounds.
    let f = frame.abs();
    let (origin, size) = if f.width() > 0.0 && f.height() > 0.0 {
        (Point::new(f.x0 / 100.0 / mm_per_px, f.y0 / 100.0 / mm_per_px), (f.width() / 100.0 * PT_PER_MM, f.height() / 100.0 * PT_PER_MM))
    } else {
        let b = bounds.abs();
        (b.origin(), ((b.width() + 1.0) * k, (b.height() + 1.0) * k))
    };
    let dev_to_doc = Affine::scale(k) * Affine::translate(-origin.to_vec2());
    let side = |v: f64| if v.is_finite() { v.clamp(1.0, 1e6) } else { 1.0 };
    Some((dev_to_doc, mm_per_px, Rect::new(0.0, 0.0, side(size.0), side(size.1))))
}

pub(super) fn play(bytes: &[u8]) -> Result<Imported, String> {
    let (dev_to_doc, mm_per_px, artboard) = frame(bytes).ok_or("the EMF header is damaged")?;
    let mut p = Player::new(Kind::Emf, dev_to_doc, mm_per_px, artboard);
    let mut at = 0usize;
    let mut ended = false;
    while let Some(head) = bytes.get(at..at + 8) {
        let kind = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let Some(rec) = bytes.get(at..at.saturating_add(size)).filter(|_| size >= 8) else { break };
        if kind == 14 {
            ended = true;
            break;
        }
        if !matches!(kind, 27 | 54 | 5 | 6 | 88 | 89) {
            p.flush();
        }
        if record(&mut p, kind, rec).is_none() {
            p.skip();
        }
        at += size;
    }
    if !ended {
        p.warn("the file ends early: what it held was imported");
    }
    Ok(p.finish())
}

/// Play one record (`rec`: all of it, with its type and size). `None`: unknown or damaged.
fn record(p: &mut Player, kind: u32, rec: &[u8]) -> Option<()> {
    let mut r = Reader::new(rec.get(8..)?);
    match kind {
        // PolyBezier, Polygon, Polyline, PolyBezierTo, PolylineTo and their 16-bit forms.
        2..=6 | 85..=89 => {
            let wide = kind < 85;
            r.skip(16)?;
            let n = r.u32()?;
            let pts = points(&mut r, n, wide)?;
            match if wide { kind } else { kind - 83 } {
                2 => p.poly(&pts, false, true),
                3 => p.poly(&pts, true, false),
                4 => p.poly(&pts, false, false),
                5 => p.poly_to(&pts, true),
                _ => p.poly_to(&pts, false),
            }
        }
        7 | 90 => p.poly_poly(&polys(&mut r, kind == 7)?, false),
        8 | 91 => p.poly_poly(&polys(&mut r, kind == 8)?, true),
        9 => p.set_window_ext(f64::from(r.i32()?), f64::from(r.i32()?)),
        10 => p.set_window_org(f64::from(r.i32()?), f64::from(r.i32()?)),
        11 => p.set_viewport_ext(f64::from(r.i32()?), f64::from(r.i32()?)),
        12 => p.set_viewport_org(f64::from(r.i32()?), f64::from(r.i32()?)),
        17 => p.set_map_mode(r.u32()?),
        19 => p.set_fill_mode(r.u32()? == 1),
        22 => p.set_text_align(r.u32()?),
        24 => p.set_text_color(color(&mut r)?),
        27 => p.move_to(point32(&mut r)?),
        29 => p.exclude_clip(),
        30 => p.clip_rect(rect(&mut r)?),
        31 | 32 => {
            let (xn, xd, yn, yd) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
            p.scale_ext(kind == 32, (f64::from(xn), f64::from(xd)), (f64::from(yn), f64::from(yd)));
        }
        33 => p.save(),
        34 => p.restore(r.i32()?),
        35 => p.world(xform(&mut r)?, 4),
        36 => {
            let x = xform(&mut r)?;
            p.world(x, r.u32()?);
        }
        37 => p.select(r.u32()?),
        38 => {
            let ih = r.u32()?;
            let style = r.u32()?;
            let width = f64::from(r.i32()?);
            r.skip(4)?;
            p.create_at(ih, Obj::Pen(PenObj { style, width, color: color(&mut r)?, entries: vec![] }));
        }
        39 => {
            let ih = r.u32()?;
            let style = r.u32()?;
            p.create_at(ih, Obj::Brush(BrushObj::from_log(style, color(&mut r)?)));
        }
        40 => p.delete(r.u32()?),
        42 => p.figure(vectorcraft_geom::shapes::ellipse(rect(&mut r)?.abs()).to_bezpath(), true),
        43 => p.figure(vectorcraft_geom::shapes::rectangle(rect(&mut r)?.abs()).to_bezpath(), true),
        44 => {
            let b = rect(&mut r)?;
            let (cx, cy) = (f64::from(r.i32()?), f64::from(r.i32()?));
            p.figure(super::round_rect(b, cx, cy), true);
        }
        45..=47 => {
            let b = rect(&mut r)?;
            let (s, e) = (point32(&mut r)?, point32(&mut r)?);
            p.arc(b, s, e, [ArcKind::Open, ArcKind::Chord, ArcKind::Pie][(kind - 45) as usize]);
        }
        49 | 99 | 122 => p.create_at(r.u32()?, Obj::Other),
        54 => {
            let pt = point32(&mut r)?;
            p.poly_to(&[pt], false);
        }
        57 => p.set_arc_ccw(r.u32()? != 2),
        58 => p.set_miter(f64::from(r.u32()?)),
        59 => p.begin_path(),
        60 => p.end_path(),
        61 => p.close_figure(),
        62 => p.paint_path(true, false),
        63 => p.paint_path(true, true),
        64 => p.paint_path(false, true),
        67 => p.clip_path(r.u32()?),
        68 => p.abort_path(),
        70 => {
            // EMF+ records ride in comments starting "EMF+".
            r.skip(4)?;
            if r.u32()? == 0x2B46_4D45 {
                p.emf_plus = true;
            }
        }
        75 => {
            let cb = r.u32()?;
            let mode = r.u32()?;
            let data = r.bytes(cb as usize)?;
            let mut d = Reader::new(data);
            let rects = if cb >= 32 {
                d.skip(8)?;
                let n = d.u32()? as usize;
                d.skip(20)?;
                if n > MAX_POINTS || d.rest().len() < n * 16 {
                    return None;
                }
                (0..n).map(|_| rect(&mut d)).collect::<Option<Vec<_>>>()?
            } else {
                vec![]
            };
            p.clip_region(&rects, mode);
        }
        76 | 77 => {
            r.skip(16)?;
            let dest = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
            let rop = r.u32()?;
            let (xs, ys) = (r.i32()?, r.i32()?);
            r.skip(24 + 4 + 4)?;
            let (off_bmi, cb_bmi, off_bits, cb_bits) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let (ws, hs) = if kind == 77 { (r.i32()?, r.i32()?) } else { (dest.2, dest.3) };
            let d = Rect::new(f64::from(dest.0), f64::from(dest.1), f64::from(dest.0) + f64::from(dest.2), f64::from(dest.1) + f64::from(dest.3));
            if cb_bmi == 0 {
                p.pattern_blit(d, rop);
            } else {
                let img = bitmap(rec, off_bmi, cb_bmi, off_bits, cb_bits, Alpha::Ignore, &mut p.pixels_left);
                let src = source(xs, ys, ws, hs, &img, false);
                p.image(img, src, d, rop, 1.0);
            }
        }
        81 => {
            r.skip(16)?;
            let (xd, yd, xs, ys, ws, hs) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?, r.i32()?, r.i32()?);
            let (off_bmi, cb_bmi, off_bits, cb_bits) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            r.skip(4)?;
            let rop = r.u32()?;
            let (wd, hd) = (r.i32()?, r.i32()?);
            let img = bitmap(rec, off_bmi, cb_bmi, off_bits, cb_bits, Alpha::Ignore, &mut p.pixels_left);
            let src = source(xs, ys, ws, hs, &img, bottom_up(rec, off_bmi));
            let d = Rect::new(f64::from(xd), f64::from(yd), f64::from(xd) + f64::from(wd), f64::from(yd) + f64::from(hd));
            p.image(img, src, d, rop, 1.0);
        }
        82 => {
            let ih = r.u32()?;
            p.create_at(ih, Obj::Font(font(&mut r)?));
        }
        83 | 84 => {
            r.skip(16 + 4 + 8)?;
            let at = point32(&mut r)?;
            let n = r.u32()? as usize;
            let off = r.u32()?;
            let s = if kind == 84 {
                let raw = slice(rec, off, u32::try_from(n.checked_mul(2)?).ok()?)?;
                String::from_utf16_lossy(&raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>())
            } else {
                slice(rec, off, u32::try_from(n).ok()?)?.iter().map(|b| char::from(*b)).collect()
            };
            p.text(at, &s);
        }
        93 | 94 => p.create_at(r.u32()?, Obj::Brush(BrushObj::Pattern)),
        95 => {
            let ih = r.u32()?;
            r.skip(16)?;
            let style = r.u32()?;
            let width = f64::from(r.u32()?);
            let brush = r.u32()?;
            let c = color(&mut r)?;
            r.skip(4)?;
            let n = (r.u32()? as usize).min(64);
            let entries: Vec<f64> = (0..n).map_while(|_| r.u32()).map(f64::from).collect();
            // A pen painted with the null brush draws nothing.
            let style = if brush == 1 { super::PS_NULL } else { style };
            p.create_at(ih, Obj::Pen(PenObj { style, width, color: c, entries }));
        }
        114 => {
            r.skip(16)?;
            let (xd, yd, wd, hd) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
            let blend = r.bytes(4)?;
            let (constant, per_pixel) = (blend.get(2).copied().unwrap_or(255), blend.get(3).is_some_and(|f| *f & 1 != 0));
            let (xs, ys) = (r.i32()?, r.i32()?);
            r.skip(24 + 4 + 4)?;
            let (off_bmi, cb_bmi, off_bits, cb_bits) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
            let (ws, hs) = (r.i32()?, r.i32()?);
            let alpha = if per_pixel { Alpha::Premultiplied } else { Alpha::Ignore };
            let img = bitmap(rec, off_bmi, cb_bmi, off_bits, cb_bits, alpha, &mut p.pixels_left);
            let src = source(xs, ys, ws, hs, &img, false);
            let d = Rect::new(f64::from(xd), f64::from(yd), f64::from(xd) + f64::from(wd), f64::from(yd) + f64::from(hd));
            p.image(img, src, d, crate::import::SRCCOPY, f64::from(constant) / 255.0);
        }
        k if IGNORED.contains(&k) => {}
        _ => return None,
    }
    Some(())
}
