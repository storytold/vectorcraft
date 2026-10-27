use super::*;

/// What an Affinity file opened as.
pub(in super::super) struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
    /// Only the embedded preview: never placed, used as a template or mined for styles.
    pub preview_only: bool,
}

pub(in super::super) fn import(name: &str, bytes: &[u8]) -> Result<Imported> {
    match vectorcraft_affinity::read(bytes, vectorcraft_affinity::Limits::default()) {
        Ok(d) => {
            let mut b = Builder::new(72.0 / d.dpi);
            b.document(&d)?;
            b.doc.limit_depth_noting(&mut b.warnings);
            let mut warnings = d.warnings.iter().map(|w| format!("Affinity: not imported or approximated: {w}.")).collect::<Vec<_>>();
            warnings.extend(b.warnings.iter().map(|w| format!("Affinity: {w}.")));
            Ok(Imported { doc: b.doc, warnings, preview_only: false })
        }
        Err(native) => {
            let (doc, w) = preview(name, bytes, &native)?;
            Ok(Imported { doc, warnings: vec![w], preview_only: true })
        }
    }
}

/// The embedded preview, for files whose native document can't be read.
fn preview(name: &str, bytes: &[u8], native: &vectorcraft_affinity::Error) -> Result<(Document, String)> {
    let p = vectorcraft_affinity::preview(bytes).map_err(|e| err(format!("{native}; its embedded preview can't be read either ({e})")))?;
    // `raster_doc` normally probes PNG dimensions and keeps its encoded bytes. Verify this
    // untrusted preview's complete pixels first, so corrupt data cannot create a blank image.
    let mut reader = image::ImageReader::with_format(Cursor::new(p.png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 << 20);
    reader.limits(limits);
    let _pixels = reader.decode().map_err(err)?;
    let d = raster_doc(name, p.png)?;
    Ok((
        d,
        format!(
            "The native Affinity document could not be read ({native}). Opened only its embedded {}×{} PNG preview, which may be smaller than the document: no layers, vectors, text or pages. Save a new copy; the Affinity source cannot be saved back. Export SVG or PDF from Affinity for editable vectors.",
            p.width, p.height
        ),
    ))
}
