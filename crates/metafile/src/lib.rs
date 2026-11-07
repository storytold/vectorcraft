//! Windows metafiles: hand-written EMF and WMF writers and readers.
//!
//! [`export`] writes the visible art over one region of a document (an artboard) as a metafile:
//!
//! - **EMF**: paths keep their Bézier curves, fills are solid brushes, strokes geometric pens with
//!   their width, caps, joins, miter limit and dashes; strokes a pen can't draw (aligned inside or
//!   outside, width profiles, arrowheads) are their filled outlines and brush strokes their brush
//!   art. Clipping masks clip, type is written as glyph outlines, images keep their pixels (with
//!   their transparency), gradients become images clipped to what they paint and pattern fills
//!   their tiles clipped to the shape. The header's frame is the region in 0.01 mm.
//! - **WMF**: the same art with curves flattened into polygons and 16-bit coordinates, behind a
//!   placeable header that gives the size; WMF has no clipping paths, transparency, dashes on wide
//!   lines or gradients, so clipped art is written whole, images over white, dashes as separate
//!   lines and gradients and patterns as one colour (each loss comes back as a warning).
//!
//! [`import`] reads either format back into a document of one artboard (the picture's frame) and
//! one layer: paths with fills and strokes, clipping, images and text. Records it doesn't read are
//! skipped, with one warning.
//!
//! - `bytes`: little-endian writing and bounds-checked reading.
//! - `dib`: device-independent bitmaps, both ways.
//! - `scene`: the document walk both writers share.
//! - `emf`, `wmf`: the writers.
//! - `import`: the reader.

mod bytes;
mod dib;
mod emf;
mod import;
mod scene;
mod wmf;

use vectorcraft_doc::Document;
use vectorcraft_geom::Rect;

/// A Windows metafile format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Enhanced Metafile (32-bit coordinates, Bézier curves, paths and clipping).
    Emf,
    /// Windows Metafile with a placeable header (16-bit coordinates, polygons).
    Wmf,
}

impl Kind {
    /// The format id (`emf`, `wmf`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Emf => "emf",
            Self::Wmf => "wmf",
        }
    }
}

/// The metafile format `bytes` start like, if any.
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if emf::is_emf(bytes) {
        Some(Kind::Emf)
    } else if wmf::is_wmf(bytes) {
        Some(Kind::Wmf)
    } else {
        None
    }
}

/// A written metafile.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub bytes: Vec<u8>,
    /// What was approximated or left out.
    pub warnings: Vec<String>,
}

/// Write the visible art of `doc` over `region` (document space: its top-left corner is the
/// picture's origin, its size the picture's) as `kind`. Live geometry effects are applied first;
/// template layers, hidden objects and guides are left out. Raster effects should be turned into
/// images before (the engine does); the ones left are reported and left out.
pub fn export(doc: &Document, region: Rect, kind: Kind) -> Result<Output, String> {
    let r = region.abs();
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) || r.width() <= 0.0 || r.height() <= 0.0 {
        return Err("the export region is empty".into());
    }
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    let (ops, mut warnings) = scene::Scene::new(doc, r, kind == Kind::Emf).run();
    let bytes = match kind {
        Kind::Emf => emf::write(&ops, r, &doc.title),
        Kind::Wmf => wmf::write(&ops, r, &mut warnings),
    };
    Ok(Output { bytes, warnings })
}

/// A metafile read into a document.
#[derive(Clone, Debug)]
pub struct Imported {
    pub document: Document,
    pub kind: Kind,
    /// What was approximated or skipped.
    pub warnings: Vec<String>,
}

/// Read an EMF or WMF file (with or without a placeable header) into a document whose one
/// artboard is the picture's frame.
pub fn import(bytes: &[u8]) -> Result<Imported, String> {
    let mut i = import::import(bytes)?;
    i.document.limit_depth_noting(&mut i.warnings);
    Ok(i)
}

#[cfg(test)]
mod tests;
