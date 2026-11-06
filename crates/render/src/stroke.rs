//! Raster-only stroke fitting: preserve thin curved bands through fill flattening.

use crate::effects;
use vectorcraft_doc::StrokeLayer;
use vectorcraft_geom::BezPath;

/// Vello flattens the two sides of a filled stroke independently at 0.25 device pixels.
/// For strokes narrower than four times the raster tolerance, fit and preflatten the
/// band more tightly. Wider strokes and the document/export geometry are unchanged.
pub(crate) fn outline(line: &BezPath, st: &StrokeLayer, width: f64, tol: f64) -> BezPath {
    if width >= tol * 4.0 {
        return effects::stroke::line_outline(line, st, width, tol);
    }
    // Closed endpoint rounding must not consume a substantial part of a subpixel band.
    let fine = (tol / 8.0).min(width / 32.0).max(1e-4);
    let outline = effects::stroke::line_outline(line, st, width, fine);
    let mut flat = BezPath::new();
    // Keep upstream's per-curve work cap for untrusted or extreme coordinates.
    vectorcraft_geom::flatten(outline.iter(), fine, |el| flat.push(el));
    flat
}
