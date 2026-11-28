//! Windows metafiles: hand-written EMF and WMF writers and readers.
//!
//! [`export`] writes the visible art over one region of a document (an artboard) as a metafile,
//! and [`export_regions`] one metafile per region:
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
//! [`import`] reads either format (an EMF with its EMF+ records too, through `craft_metafile`)
//! into a document of one artboard (the picture's frame) and one layer: paths with fills and
//! strokes, clipping, images and text. Records it doesn't read are skipped, with one warning.
//! A picture cut short by a reader limit, images and pattern brushes that can't be read, clips
//! that can't be applied and raster operations other than a plain copy each add a warning.
//! [`import_pict`] reads QuickDraw pictures (PICT, versions 1 and 2) into the same kind of
//! document.
//!
//! The app walks its document and maps its drawing operations to the shared writers.

#[cfg(test)]
mod bytes;
mod dib;
mod encode;
#[cfg(test)]
mod expansion_test;
mod import;
mod scene;

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
    craft_metafile::sniff(bytes).map(|kind| match kind {
        craft_metafile::Kind::Emf => Kind::Emf,
        craft_metafile::Kind::Wmf => Kind::Wmf,
    })
}

/// A written metafile.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub bytes: Vec<u8>,
    /// What was approximated or left out.
    pub warnings: Vec<String>,
}

/// Write the visible art of `doc` over `region` (document space: its top-left corner is the
/// picture's origin, its size the picture's) as `kind`. Live geometry effects are applied first,
/// in symbols' art too, where each instance places that art (EMF clips 9-slice cells, WMF does
/// not); template layers, hidden objects and guides are left out. Raster effects should be turned
/// into images before (the engine does); the ones left are reported and left out.
pub fn export(doc: &Document, region: Rect, kind: Kind) -> Result<Output, String> {
    Ok(export_regions(doc, &[region], kind)?.pop().unwrap_or_default())
}

/// The error [`export_regions`] returns for `regions`, without writing anything: a region whose
/// corners are not all finite, or that has no width or no height, is empty. Corners in either
/// order are allowed.
pub fn check_regions(regions: &[Rect]) -> Result<(), String> {
    let empty = |r: Rect| ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) || r.width() <= 0.0 || r.height() <= 0.0;
    if regions.iter().any(|r| empty(r.abs())) {
        return Err("the export region is empty".into());
    }
    Ok(())
}

/// Write the visible art of `doc` over each of `regions` as `kind`, one metafile per region in the
/// same order, each as [`export`] writes it over that region. The live effects are applied once
/// for all of them. When any region is empty, nothing is written and the result is an error.
pub fn export_regions(doc: &Document, regions: &[Rect], kind: Kind) -> Result<Vec<Output>, String> {
    check_regions(regions)?;
    let regions: Vec<Rect> = regions.iter().map(|r| r.abs()).collect();
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    // The scene writes layers that do not print and leaves hidden layers and objects out.
    let layers = vectorcraft_effects::WrittenLayers { hidden: false, non_printing: true, hidden_objects: false };
    let baked = vectorcraft_effects::bake_document_placing_symbols(doc, kind == Kind::Emf, layers);
    let doc = baked.as_ref().unwrap_or(doc);
    regions
        .into_iter()
        .map(|r| {
            let (ops, mut warnings) = scene::Scene::new(doc, r, kind == Kind::Emf).run();
            encode::write(ops, r, kind, doc.info_title(), &mut warnings).map(|bytes| Output { bytes, warnings })
        })
        .collect()
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

/// A QuickDraw picture read into a document.
#[derive(Clone, Debug)]
pub struct Picture {
    pub document: Document,
    /// What was approximated or skipped.
    pub warnings: Vec<String>,
}

/// Do `bytes` hold a QuickDraw picture (version 1 or 2), after a PICT file's 512 bytes of
/// application data or on their own?
pub fn is_pict(bytes: &[u8]) -> bool {
    craft_metafile::is_pict(bytes)
}

/// Read a QuickDraw picture (version 1 or 2; a PICT file, or a picture on its own) into a document
/// whose one artboard is the picture's frame.
pub fn import_pict(bytes: &[u8]) -> Result<Picture, String> {
    let mut p = import::import_pict(bytes)?;
    p.document.limit_depth_noting(&mut p.warnings);
    Ok(p)
}

#[cfg(test)]
mod tests;
