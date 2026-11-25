//! The metafile readers. EMF (with its EMF+ records) and WMF files, and QuickDraw pictures (PICT),
//! are read by `craft_metafile`, and `picture` turns what they draw into document objects.
//!
//! Fills and strokes become paths with a fill and a stroke; lines drawn one after another with the
//! same pen join into one path. Clips become clipping groups around what they clip. Images keep
//! their pixels (as PNG), text becomes point type in the font the file names.

mod picture;

use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{BezPath, PathEl, Point, Rect, Shape};

/// Most objects an import makes (a hostile file can't exhaust memory with them).
const MAX_NODES: usize = 1 << 20;
/// Objects and clips reaching further than this from the origin (points, well inside the canvas)
/// are left out: they come from damaged or hostile coordinates.
const LIMIT: f64 = 1e6;
/// Width of a cosmetic pen (one pixel of a 96 ppi screen), points.
const HAIRLINE: f64 = 0.75;

const BAD_IMAGE: &str = "an image that couldn't be read was left out";
const FAR_AWAY: &str = "objects far outside the picture were left out";

/// Is `r` finite and within [`LIMIT`]?
fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= LIMIT)
}

/// Is `path` one upright rectangle around the whole of `frame`? Writers clip to one first; it clips
/// nothing in the picture. A path of more than one figure is not one rectangle, even when its
/// figures run along the sides of a box. Nor is one figure that runs back over its own edges, so
/// that it encloses less than the whole box.
fn around(path: &BezPath, frame: Rect) -> bool {
    if path.elements().len() > 6 {
        return false;
    }
    let b = path.bounding_box();
    let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
    let near = |p: Point, q: Point| (p - q).hypot() <= 1e-6 * (1.0 + b.width().max(b.height()));
    // Each edge, the closing one included, runs along a side: from a corner to itself or to a
    // neighbor (corners are in order around the box), never across it.
    let along = |i: usize, j: usize| (i + 4 - j) % 4 != 2;
    let mut seen = [false; 4];
    let mut points = Vec::with_capacity(6);
    let (mut first, mut last) = (None, None);
    for e in path.elements() {
        let (p, start) = match e {
            PathEl::MoveTo(p) => (*p, true),
            PathEl::LineTo(p) => (*p, false),
            PathEl::ClosePath => break,
            _ => return false,
        };
        let Some(i) = corners.iter().position(|c| near(*c, p)) else { return false };
        if start {
            if first.is_some() {
                return false;
            }
            first = Some(i);
        } else if last.is_some_and(|l| !along(l, i)) {
            return false;
        }
        last = Some(i);
        seen[i] = true;
        points.push(corners[i]);
    }
    if let (Some(f), Some(l)) = (first, last)
        && !along(f, l)
    {
        return false;
    }
    // Every edge runs along a side, so the figure winds the same number of times around every
    // point inside the box: once, when its area is the box's.
    let twice_area: f64 = points.iter().zip(points.iter().cycle().skip(1)).map(|(p, q)| (*p - corners[0]).cross(*q - corners[0])).sum();
    if (twice_area.abs() - 2.0 * b.area()).abs() > 1e-6 * (1.0 + 2.0 * b.area()) {
        return false;
    }
    let b = b.inflate(0.5, 0.5);
    seen.iter().all(|s| *s) && b.contains(frame.origin()) && b.contains(Point::new(frame.x1, frame.y1))
}

fn rgb(c: [u8; 3]) -> Paint {
    Paint::solid(Color::rgb8(c[0], c[1], c[2]))
}

pub(crate) use picture::import;

/// Read a QuickDraw picture (PICT, versions 1 and 2) through `craft_metafile`.
pub(crate) fn import_pict(bytes: &[u8]) -> Result<crate::Picture, String> {
    let pic = craft_metafile::parse_pict(bytes).map_err(|e| match e {
        craft_metafile::Error::NotPict => "not a PICT file".to_string(),
        _ => "the PICT file is damaged".to_string(),
    })?;
    Ok(picture::convert(pic))
}
