//! Translate VectorCraft's drawing operations to the shared metafile writers.
use craft_metafile::{
    Cap, Join,
    write::{self, Dash, Op, Pen},
};
use vectorcraft_doc::{LineCap, LineJoin};
use vectorcraft_geom::{FillRule, Rect};

use crate::{Kind, scene};

pub(crate) fn write(ops: Vec<scene::Op>, region: Rect, kind: Kind, title: &str, warnings: &mut Vec<String>) -> Result<Vec<u8>, String> {
    let ops: Vec<_> = ops.into_iter().map(|op| convert(op, kind)).collect();
    let invalid = || "the export region is empty".to_string();
    match kind {
        Kind::Emf => write::emf(&ops, region, "VectorCraft", title).ok_or_else(invalid),
        Kind::Wmf => {
            let out = write::wmf(&ops, region).ok_or_else(invalid)?;
            for (lost, warning) in [
                (out.translucent_images, "WMF has no transparency: images are written over white"),
                (out.turned_images, "WMF can't rotate or skew images: they are written upright, filling their bounds"),
                (out.clips_ignored, "clipping masks are not applied: the art they clip is written whole"),
                (out.oversized_images, "WMF images with a side over 32,767 pixels were left out"),
            ] {
                if lost && !warnings.iter().any(|w| w == warning) {
                    warnings.push(warning.to_string());
                }
            }
            Ok(out.bytes)
        }
    }
}

fn convert(op: scene::Op, kind: Kind) -> Op {
    match op {
        scene::Op::Fill { path, rule, rgb, .. } => Op::Fill { path, even_odd: rule == FillRule::EvenOdd, rgb },
        scene::Op::Stroke { mut path, pen } => {
            let dash = pen.dash.and_then(|dash| {
                let native = Dash { pattern: dash.pattern.clone(), offset: dash.offset };
                if kind == Kind::Emf && native.emf_pen_draws() {
                    Some(native)
                } else {
                    // Keep VectorCraft's own dash placement for strokes that cannot use an
                    // EMF native pen, including adjusted dashes around corners in WMF.
                    if let Some(dashed) = vectorcraft_effects::stroke::dash(&path, &dash) {
                        path = dashed.path;
                    }
                    None
                }
            });
            let cap = match pen.cap {
                LineCap::Butt => Cap::Flat,
                LineCap::Round => Cap::Round,
                LineCap::Square => Cap::Square,
            };
            let join = match pen.join {
                LineJoin::Miter => Join::Miter,
                LineJoin::Round => Join::Round,
                LineJoin::Bevel => Join::Bevel,
            };
            Op::Stroke { path, pen: Pen { rgb: pen.rgb, width: pen.width, cap, join, miter: pen.miter, dash } }
        }
        scene::Op::Image { image, xf, opacity } => Op::Image { image, xf, opacity },
        scene::Op::Clip { path, rule } => Op::Clip { path, even_odd: rule == FillRule::EvenOdd },
        scene::Op::Unclip => Op::Unclip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonnative_metafile_pens_keep_vectorcraft_corner_alignment() {
        let mut path = kurbo::BezPath::new();
        path.move_to((7.0, 8.0));
        path.line_to((42.0, 8.0));
        path.line_to((42.0, 45.0));
        path.line_to((7.0, 45.0));
        path.close_path();
        let dash = vectorcraft_doc::Dash { pattern: vec![5.0, 3.0], offset: 1.25, align_corners: true };
        let plain = scene::Pen { rgb: [30, 60, 90], paint: None, width: 1.0, cap: LineCap::Round, join: LineJoin::Round, miter: 4.0, dash: None };
        let with_dash = scene::Pen { dash: Some(dash.clone()), ..plain.clone() };
        let segments = vectorcraft_effects::stroke::dash(&path, &dash).unwrap().path;
        let region = Rect::new(0.0, 0.0, 50.0, 50.0);
        for kind in [Kind::Emf, Kind::Wmf] {
            let actual = write(vec![scene::Op::Stroke { path: path.clone(), pen: with_dash.clone() }], region, kind, "", &mut Vec::new()).unwrap();
            let already_dashed =
                write(vec![scene::Op::Stroke { path: segments.clone(), pen: plain.clone() }], region, kind, "", &mut Vec::new()).unwrap();
            let unsplit = write(vec![scene::Op::Stroke { path: path.clone(), pen: plain.clone() }], region, kind, "", &mut Vec::new()).unwrap();
            assert_eq!(actual, already_dashed, "the wire writer must receive the app-aligned segments: {kind:?}");
            assert_ne!(actual, unsplit, "the fixture must distinguish a lost dash pattern: {kind:?}");
        }
    }
}
