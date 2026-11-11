//! HP-GL import: a plotter file (HP-GL/2, `.plt`, `.hpgl`, `.hgl`, `.hpg`) as a document. Older
//! HP-GL files are read by the HP-GL/2 rules for the instructions the two share.
//!
//! The pen's moves become paths: each run drawn with the pen down is one stroked path in the
//! selected pen's color and width (0.35 mm unless PW sets another), with the line type as a dash
//! pattern and the line attributes as caps and joins. Circles, arcs, wedges, rectangles and Bézier
//! curves are drawn as curves (the chord angle isn't used). Filled rectangles, wedges and polygons
//! (polygon mode, FP) become filled paths, edged ones stroked paths. Plotter units (1016 to the
//! inch, y up) become points, through user units (SC, IP, IR) and the rotation (RO). The artboard
//! is the bounds of what the art paints, its strokes included.
//!
//! VectorCraft's own rules where the device decides: before the first SP the pen is pen 1 (a
//! device draws nothing until SP), and the hard-clip limits are 11 × 8.5 in from the origin, so
//! P2 lies there when the file sets no scaling points. SP0 draws nothing. Labels, symbol mode's symbols and raster
//! images (HP RTL) aren't drawn, the input window doesn't clip, hatched fills are filled solid,
//! and the art of every page goes on one artboard; each comes back as a warning. Printer escape
//! sequences and PJL lines around the instructions are skipped.
//!
//! - `lex`: the bytes as instructions, numbers, labels and PE data.
//! - `plot`: the instructions played on the pen, as shapes.

mod lex;
mod plot;

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, FillLayer, LayerColor, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, PathData, Rect};

use crate::Imported;
use lex::Lexer;
use plot::Plot;

/// Points per plotter unit.
const PT_PER_PLU: f64 = 72.0 / 1016.0;
/// The weight of a zero-width pen (one device dot), in points.
const THINNEST: f64 = 0.25;

/// Instructions the reader knows and draws nothing for (their parameters are read and dropped).
const IGNORED: &[[u8; 2]] = &[
    *b"AC", *b"AD", *b"AF", *b"AH", *b"AP", *b"AS", *b"CA", *b"CF", *b"CM", *b"CP", *b"CS", *b"CT", *b"CV", *b"DC", *b"DI", *b"DL", *b"DP", *b"DR",
    *b"DS", *b"DV", *b"EC", *b"ES", *b"FI", *b"FN", *b"FS", *b"GM", *b"IM", *b"IV", *b"KY", *b"LM", *b"LO", *b"MC", *b"MG", *b"MT", *b"NR", *b"OA",
    *b"OC", *b"OD", *b"OE", *b"OF", *b"OG", *b"OH", *b"OI", *b"OL", *b"OP", *b"OS", *b"OT", *b"OW", *b"PB", *b"PP", *b"PS", *b"PT", *b"QL", *b"RF",
    *b"RP", *b"SA", *b"SB", *b"SD", *b"SG", *b"SI", *b"SL", *b"SR", *b"SS", *b"ST", *b"SV", *b"TD", *b"TL", *b"TR", *b"UC", *b"VA", *b"VN", *b"VS",
    *b"XT", *b"YT",
];

/// Instructions the reader plays.
const PLAYED: &[[u8; 2]] = &[
    *b"AA", *b"AR", *b"AT", *b"BL", *b"BP", *b"BR", *b"BZ", *b"CI", *b"CO", *b"CR", *b"DF", *b"DT", *b"EA", *b"EP", *b"ER", *b"EW", *b"FP", *b"FR",
    *b"FT", *b"IN", *b"IP", *b"IR", *b"IW", *b"LA", *b"LB", *b"LT", *b"NP", *b"PA", *b"PC", *b"PD", *b"PE", *b"PG", *b"PM", *b"PR", *b"PU", *b"PW",
    *b"RA", *b"RO", *b"RR", *b"RT", *b"SC", *b"SM", *b"SP", *b"UL", *b"WD", *b"WG", *b"WU",
];

/// Instructions a plot file starts with.
const FIRST: &[[u8; 2]] = &[
    *b"IN", *b"DF", *b"BP", *b"CO", *b"PS", *b"SP", *b"PU", *b"PD", *b"PA", *b"PR", *b"IP", *b"IR", *b"SC", *b"NP", *b"PC", *b"PW", *b"WU", *b"RO",
    *b"QL", *b"MC", *b"PP", *b"TR", *b"CR", *b"LT", *b"LA", *b"FT", *b"VS", *b"FS", *b"AC", *b"PG", *b"PE", *b"DT",
];

/// Does `bytes` start like an HP-GL file? After any printer escapes and PJL lines (however long),
/// it must hold nothing but instructions the reader knows, the first one of those a plot file
/// starts with, up to the eighth instruction or 4 KB past the first. Their parameters must be
/// numbers, but for a comment's quoted string and the text, characters and encoded data of LB, BL,
/// WD, DT, SM and PE. An instruction the reader doesn't know counts when its numbers end with a
/// semicolon.
pub fn is_hpgl(bytes: &[u8]) -> bool {
    let mut lex = Lexer::new(bytes);
    let mut terminator = lex::ETX;
    let mut first = 0;
    for n in 0..8 {
        if n > 0 && lex.pos() > first + 4096 {
            return true;
        }
        let Some(op) = lex.op(true) else { return n > 0 && lex.done() };
        if n == 0 {
            first = lex.pos();
        }
        if n == 0 && !FIRST.contains(&op) {
            return false;
        }
        let known = PLAYED.contains(&op) || IGNORED.contains(&op);
        match &op {
            b"CO" => {
                if !lex.comment() {
                    return false;
                }
            }
            b"LB" | b"BL" | b"WD" => lex.label(terminator),
            b"DT" => {
                terminator = lex.char().unwrap_or(lex::ETX);
                lex.params();
            }
            b"SM" => {
                // The symbol character is read so it isn't taken for an instruction.
                let _ = lex.char();
            }
            b"PE" => lex.skip_pe(),
            _ if known => lex.params(),
            // An instruction the reader doesn't know (of an older or a device's own set) counts
            // when numbers and a semicolon follow it.
            _ => {
                if !lex.closed_params() {
                    return false;
                }
            }
        }
    }
    true
}

/// Read an HP-GL file.
pub fn import(bytes: &[u8]) -> Result<Imported, String> {
    if !is_hpgl(bytes) {
        return Err("not an HP-GL file".into());
    }
    let mut lex = Lexer::new(bytes);
    let mut plot = Plot::new();
    while !plot.full
        && let Some(op) = lex.op(false)
    {
        plot.op(op, &mut lex);
    }
    Ok(build(plot.finish(lex.raster)))
}

fn rgb(c: [u8; 3]) -> Paint {
    Paint::solid(Color::rgb8(c[0], c[1], c[2]))
}

/// The plot's shapes as paths on one layer, the artboard round what they paint.
fn build(plot: Plot) -> Imported {
    // Plotter units, y up, to points, y down.
    let flip = Affine::scale_non_uniform(PT_PER_PLU, -PT_PER_PLU);
    let mut nodes: Vec<Node> = plot.shapes.iter().map(|s| node(s, flip)).collect();
    // The strokes' caps and miter joins included.
    let finite = |r: &Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite());
    let frame = nodes.iter().filter_map(Node::visual_bounds).filter(finite).reduce(|a, b| a.union(b)).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
    let mut doc = Document::new(frame.width().max(1.0), frame.height().max(1.0));
    doc.layers.clear();
    let shift = Affine::translate(-frame.origin().to_vec2());
    for n in &mut nodes {
        n.id = doc.alloc_id();
        if let NodeKind::Path { path, .. } = &mut n.kind {
            path.transform(shift);
        }
    }
    let mut layer = Node::layer(doc.alloc_id(), "Layer 1", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = nodes.into_iter().map(Arc::new).collect();
    }
    doc.layers = vec![Arc::new(layer)];
    Imported { document: doc, warnings: plot.warnings }
}

/// A shape as a path, its plotter units through `to_points`.
fn node(s: &plot::Shape, to_points: Affine) -> Node {
    let mut path = s.path.clone();
    path.apply_affine(to_points);
    let mut items = vec![];
    if let Some((c, _)) = s.fill {
        items.push(AppearanceItem::Fill(FillLayer::new(rgb(c))));
    }
    if let Some(st) = &s.stroke {
        let mut layer = StrokeLayer::new(rgb(st.color), (st.width * PT_PER_PLU).max(THINNEST));
        (layer.cap, layer.join, layer.miter_limit) = (st.cap, st.join, st.miter);
        layer.dash = st.dash.as_ref().map(|d| Dash {
            pattern: d.pattern.iter().map(|v| v * PT_PER_PLU).collect(),
            offset: d.offset * PT_PER_PLU,
            align_corners: d.adaptive,
        });
        items.push(AppearanceItem::Stroke(layer));
    }
    let mut n = Node::path(NodeId(0), PathData::from_bezpath(&path), Appearance { items, ..Appearance::default() });
    if let (NodeKind::Path { rule, .. }, Some((_, r))) = (&mut n.kind, s.fill) {
        *rule = r;
    }
    n
}

#[cfg(test)]
mod tests;
