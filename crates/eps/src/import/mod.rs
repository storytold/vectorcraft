//! EPS and PostScript import: a small clean-room PostScript interpreter that turns what a file
//! draws into document objects.
//!
//! - `lex`: the scanner; `obj`: objects, operators and errors; `interp`: the stacks, running
//!   procedures, and the operators that don't draw.
//! - `graphics`: the graphics state, coordinates, paths, colours (grey, RGB, CMYK, spot inks as
//!   spot swatches at a tint, indexed, patterns), painting, clipping, axial and radial shadings as
//!   gradients and tiling patterns as pattern swatches.
//! - `shading`: function-based and mesh shadings as gradient meshes, sampled functions.
//! - `data`: data the program reads from itself through decoding filters, and images.
//! - `text`: fonts by name and type as point type; Type 3 fonts' glyphs drawn by their procedures.
//! - `preview`: the previews a file carries, for when its PostScript can't be read.
//! - `native`: the layers of an Illustrator file, from the editing copy of its art that it carries.
//!
//! Clipped art comes in as clipping groups. In a file in the legacy Illustrator format (versions
//! 3–8, with the prolog that defines its operators), the group operators `u` … `U` (nested) come in
//! as groups. The format's published specification documents its header comments (`%AI…`,
//! `%%Creator`) and these operators: Adobe Illustrator File Format Specification, version 7.0
//! (1998), listed by PRONOM at https://www.nationalarchives.gov.uk/PRONOM/fmt/423.
//!
//! The page is the file's `%%BoundingBox` (`%%HiResBoundingBox` when it has one), the first page
//! of a PostScript file without one. A file whose program can't be read (an operator the
//! interpreter doesn't know, an error, a limit reached) or that draws nothing comes in as its
//! preview (a TIFF image, a Windows metafile or an EPSI bitmap), with a warning that names the
//! error, the operator and the procedures it ran in; without a preview, what was drawn before the
//! error is kept (with that warning), else the file is refused. Before that, a file in the legacy
//! Illustrator format (one that names its prolog's procsets instead of defining them, as other apps
//! write it) comes in from its layers, read as editing data by `native`, when they can be read.

mod ai;
mod ate;
mod data;
mod graphics;
mod interp;
mod lex;
mod native;
mod obj;
mod preview;
mod shading;
mod text;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fontnames;
#[cfg(test)]
mod tests_generators;
#[cfg(test)]
mod tests_illustrator;
#[cfg(test)]
mod tests_images;
#[cfg(test)]
mod tests_native;

use std::sync::Arc;

use vectorcraft_color::Paint;
use vectorcraft_color::swatch::Swatch;
use vectorcraft_doc::{ColorMode, Document, LayerColor, Node};
use vectorcraft_geom::{Affine, Rect};

use graphics::{GState, Out};
use interp::{Fault, Interp};
use obj::PsError;

pub use native::{ai_alone, is_loss, layered_ai};
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
    /// In the legacy Illustrator format: its header has `%AI…` comments, names the app as the
    /// creator or names its procset (`Adobe_IllustratorA_AI3`, `Adobe_Illustrator_AI5`; see the
    /// module docs for the specification).
    illustrator: bool,
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
            let creator = value("%%Creator:").is_some_and(|v| v.contains("Illustrator")); // brand-ok: the creator its files name
            // Files of the legacy format that other apps write (Rhino's) name its procsets instead.
            let procset = line.contains("procset Adobe_Illustrator"); // brand-ok: the procset name its files give
            if header && (line.starts_with("%AI") || line.starts_with("%%AI") || creator || procset) {
                d.illustrator = true;
            }
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

/// What went wrong, for the user (and for us, reading a report): the PostScript error, the
/// operator that raised it and the procedure it ran in (`fault`).
fn reason(e: &PsError, fault: Option<&Fault>) -> String {
    let what = match e {
        PsError::Ps("undefined", at) => format!("it uses `{at}`, which VectorCraft's PostScript reader doesn't know"),
        PsError::Ps(name, at) => match fault.and_then(|f| f.op) {
            Some(op) if at.is_empty() || at == op => format!("a PostScript error ({name} in `{op}`)"),
            Some(op) => format!("a PostScript error ({name} in `{op}`: {at})"),
            None if at.is_empty() => format!("a PostScript error ({name})"),
            None => format!("a PostScript error ({name} in `{at}`)"),
        },
        PsError::Limit(what) => return what.to_string(),
        PsError::Exit | PsError::Stop => return "the program stopped with an error".into(),
        PsError::Quit => return "the program ended early".into(),
    };
    match fault.map(|f| f.procs.as_slice()).unwrap_or_default() {
        [] => what,
        procs => {
            let names: Vec<String> = procs.iter().map(|p| format!("`{p}`")).collect();
            format!("{what}, in {}", names.join(" in "))
        }
    }
}

/// Read an EPS or PostScript file (its first page), an Illustrator EPS through the editing copy of
/// its art when it carries one (see `native`).
pub fn import(bytes: &[u8]) -> Result<Imported, String> {
    import_with(bytes, true)
}

/// [`import`], an Illustrator EPS through the editing copy of its art only when `editing_data`
/// (else what its page prints, as any EPS).
pub fn import_with(bytes: &[u8], editing_data: bool) -> Result<Imported, String> {
    let (ps, _) = crate::sections(bytes).ok_or("the file's preview header points outside the file")?;
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
    it.illustrator = dsc.illustrator;
    let result = it.run();
    it.release();
    let fault = it.fault.take();
    let mut out = it.out;
    if let Some(n) = dsc.pages.filter(|n| *n > 1) {
        out.warn(&format!("only the first of the file's {n} pages was read"));
    }
    let drew = !out.drawn.is_empty();
    // A file of Illustrator's legacy format that names its procsets instead of defining them has
    // operators the interpreter can't run: its layers are read from the text, as editing data.
    if editing_data
        && dsc.illustrator
        && (result.is_err() || !drew)
        && let Ok((document, warnings)) = native::ai_alone(ps)
    {
        return Ok(Imported { document, warnings, preview: false });
    }
    let why = match (&result, drew) {
        (Ok(()), true) if editing_data => return Ok(native::layered(ps, finish(out))),
        (Ok(()), true) => return Ok(finish(out)),
        (Ok(()), false) => "it draws nothing VectorCraft's PostScript reader can show".to_string(),
        (Err(e), _) => reason(e, fault.as_ref()),
    };
    if let Some(p) = preview::preview(bytes, ps, frame) {
        let warnings = vec![format!("this file's PostScript couldn't be read ({why}): {}", p.what)];
        return Ok(Imported { document: p.document, warnings, preview: true });
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
        Err(e) => Err(format!("this PostScript file can't be read ({}): save it as PDF or SVG in the app that made it", reason(&e, fault.as_ref()))),
    }
}

/// The document of what `out` drew: one layer, clipped art in clipping groups (grouped art in
/// groups), the spot inks as spot swatches, in CMYK when most process colours were.
fn finish(mut out: Out) -> Imported {
    let drawn = std::mem::take(&mut out.drawn);
    let mut children = vectorcraft_doc::clipnest::nest(&mut out.doc, drawn);
    if !out.shadings.is_empty() {
        graphics::collapse(&mut children, &out.shadings);
    }
    let mut layer = Node::layer(out.doc.alloc_id(), "Layer 1", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = children;
    }
    finish_with(out, vec![Arc::new(layer)])
}

/// The document of what `out` drew, as `layers`: the spot inks as spot swatches, in CMYK when
/// most process colours were.
fn finish_with(out: Out, layers: Vec<Arc<Node>>) -> Imported {
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
    // Tiling patterns are pattern swatches.
    for name in doc.patterns.iter().map(|p| p.name.clone()).collect::<Vec<_>>() {
        if doc.swatch(&name).is_none() {
            doc.swatches.push(Swatch { paint: Paint::Pattern { pattern: name.clone(), xf: Affine::IDENTITY }, name, global: false, spot: false });
        }
    }
    doc.layers = layers;
    Imported { document: doc, warnings: out.warnings, preview: false }
}
