//! EPS and PostScript import: a small clean-room PostScript interpreter that turns what a file
//! draws into document objects.
//!
//! - `lex`: the scanner; `obj`: objects, operators and errors; `interp`: the stacks, running
//!   procedures, and the operators that don't draw.
//! - `graphics`: the graphics state, coordinates, paths, colours (grey, RGB, CMYK, spot inks as
//!   spot swatches at a tint, indexed), painting, clipping, and axial and radial shadings as
//!   gradients.
//! - `data`: data the program reads from itself through decoding filters, and images.
//! - `text`: fonts by name and type as point type.
//!
//! The page is the file's `%%BoundingBox` (`%%HiResBoundingBox` when it has one), the first page
//! of a PostScript file without one. A file whose program can't be read (an operator the
//! interpreter doesn't know, an error, a limit reached) or that draws nothing comes in as its TIFF
//! preview, with a warning; without a preview, what was drawn before the error is kept (with a
//! warning), else the file is refused.

mod data;
mod graphics;
mod interp;
mod lex;
mod obj;
mod text;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fontnames;

use std::sync::Arc;

use vectorcraft_color::Paint;
use vectorcraft_color::swatch::Swatch;
use vectorcraft_doc::{ColorMode, Document, ImageBlob, ImageObject, LayerColor, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};

use graphics::{GState, Out};
use interp::Interp;
use obj::PsError;

pub use text::family_style;

/// The page of a PostScript file without a bounding box (US Letter).
const LETTER: [f64; 4] = [0.0, 0.0, 612.0, 792.0];
/// Largest page side read (points).
const MAX_SIDE: f64 = 1e5;

/// A read file.
#[derive(Debug)]
pub struct Imported {
    pub document: Document,
    pub warnings: Vec<String>,
    /// The document shows the file's preview: its PostScript couldn't be read.
    pub preview: bool,
}

/// The DSC comments the reader uses.
#[derive(Debug, Default, PartialEq)]
struct Dsc {
    /// `llx lly urx ury` in PostScript's default space.
    bbox: Option<[f64; 4]>,
    pages: Option<u32>,
}

/// The four numbers of a bounding box comment's value, when it has them and they make a box.
fn box_of(v: &str) -> Option<[f64; 4]> {
    let n: Vec<f64> = v.split_whitespace().map_while(|w| w.parse::<f64>().ok()).collect();
    let [x0, y0, x1, y1] = n.as_slice() else { return None };
    let b = [*x0, *y0, *x1, *y1];
    (b.iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE) && x1 > x0 && y1 > y0).then_some(b)
}

impl Dsc {
    /// The comments of `ps`: from the header, or (`(atend)`) the last ones in the file.
    fn read(ps: &[u8]) -> Self {
        let mut d = Dsc::default();
        let (mut bbox, mut hires, mut atend) = (None, None, false);
        let mut header = true;
        for raw in ps.split(|b| matches!(b, b'\n' | b'\r')).filter(|l| !l.is_empty()) {
            // Past the header only comments can matter.
            if !header && !raw.starts_with(b"%%") {
                continue;
            }
            let line = String::from_utf8_lossy(raw);
            let line = line.trim_end();
            if header && (line.starts_with("%%EndComments") || !line.starts_with('%')) {
                header = false;
                if !atend {
                    break;
                }
            }
            let value = |key: &str| line.strip_prefix(key).map(str::trim);
            if let Some(v) = value("%%HiResBoundingBox:") {
                if header || atend {
                    hires = box_of(v).or(hires);
                }
            } else if let Some(v) = value("%%BoundingBox:") {
                if v.starts_with("(atend)") {
                    atend = true;
                } else if header || atend {
                    bbox = box_of(v).or(bbox);
                }
            } else if let Some(v) = value("%%Pages:")
                && header
            {
                d.pages = v.split_whitespace().next().and_then(|n| n.parse().ok());
            }
        }
        d.bbox = hires.or(bbox);
        d
    }
}

/// What went wrong, for the user.
fn reason(e: &PsError) -> String {
    match e {
        PsError::Ps("undefined", at) => format!("it uses `{at}`, which Vector W3K2's PostScript reader doesn't know"),
        PsError::Ps(name, at) if at.is_empty() => format!("a PostScript error ({name})"),
        PsError::Ps(name, at) => format!("a PostScript error ({name} in `{at}`)"),
        PsError::Limit(what) => what.to_string(),
        PsError::Exit | PsError::Stop => "the program stopped with an error".into(),
        PsError::Quit => "the program ended early".into(),
    }
}

/// Read an EPS or PostScript file (its first page).
pub fn import(bytes: &[u8]) -> Result<Imported, String> {
    let (ps, tiff) = crate::sections(bytes).ok_or("the file's preview header points outside the file")?;
    if !ps.starts_with(b"%!") {
        return Err("this is not a PostScript file".into());
    }
    let dsc = Dsc::read(ps);
    let [llx, lly, urx, ury] = dsc.bbox.unwrap_or(LETTER);
    let frame = Rect::new(0.0, 0.0, urx - llx, ury - lly);
    let mut doc = Document::new(frame.width(), frame.height());
    doc.layers.clear();
    let page = Affine::new([1.0, 0.0, 0.0, -1.0, -llx, ury]);
    let mut it = Interp::new(ps, GState::default(), Out::new(doc, page, frame));
    let result = it.run();
    let mut out = it.out;
    if let Some(n) = dsc.pages.filter(|n| *n > 1) {
        out.warn(&format!("only the first of the file's {n} pages was read"));
    }
    let drew = !out.drawn.is_empty();
    let why = match (&result, drew) {
        (Ok(()), true) => return Ok(finish(out)),
        (Ok(()), false) => "it draws nothing Vector W3K2's PostScript reader can show".to_string(),
        (Err(e), _) => reason(e),
    };
    if let Some(doc) = tiff.and_then(|t| preview(t, frame)) {
        let warnings = vec![format!("this file's PostScript couldn't be read ({why}): its preview was placed instead, at screen resolution")];
        return Ok(Imported { document: doc, warnings, preview: true });
    }
    if drew {
        out.warnings.insert(0, format!("the file was read up to an error ({why}): what follows it was left out"));
        return Ok(finish(out));
    }
    match result {
        Ok(()) => {
            out.warn("the file draws nothing");
            Ok(finish(out))
        }
        Err(e) => Err(format!("this PostScript file can't be read ({}): save it as PDF or SVG in the app that made it", reason(&e))),
    }
}

/// The document of what `out` drew: one layer, clipped art in clipping groups, the spot inks as
/// spot swatches, in CMYK when most process colours were.
fn finish(mut out: Out) -> Imported {
    let drawn = std::mem::take(&mut out.drawn);
    let mut children = vectorcraft_doc::clipnest::nest(&mut out.doc, drawn);
    if !out.shadings.is_empty() {
        graphics::collapse(&mut children, &out.shadings);
    }
    let mut doc = out.doc;
    if out.cmyk > out.rgb {
        doc.color_mode = ColorMode::Cmyk;
        (doc.swatches, doc.swatch_groups) = vectorcraft_color::default_swatches(ColorMode::Cmyk.model());
    }
    for (name, color) in out.spots {
        if doc.swatch(&name).is_none() {
            doc.swatches.push(Swatch { name, paint: Paint::solid(color), global: true, spot: true });
        }
    }
    let mut layer = Node::layer(doc.alloc_id(), "Layer 1", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = children;
    }
    doc.layers = vec![Arc::new(layer)];
    Imported { document: doc, warnings: out.warnings, preview: false }
}

/// A document of `frame`'s size showing TIFF preview `tiff` stretched over it.
fn preview(tiff: &[u8], frame: Rect) -> Option<Document> {
    let img = match image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff) {
        Ok(img) => img.to_rgba8(),
        Err(_) => crate::tiff::palette_rgba(tiff)?,
    };
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    let mut doc = Document::new(frame.width(), frame.height());
    let blob = ImageBlob::new("image/png", png);
    let key = blob.content_key();
    doc.images.insert(key.clone(), blob);
    let xf = Affine::scale_non_uniform(frame.width() / f64::from(w), frame.height() / f64::from(h));
    let mut n = Node::new(doc.alloc_id(), NodeKind::Image(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default() }));
    n.name = Some("Preview".into());
    let layer = doc.layers.first().map(|l| l.id)?;
    doc.insert(Some(layer), 0, n).ok()?;
    Some(doc)
}
