//! The previews an EPS file carries for apps that don't read PostScript, opened when its
//! PostScript can't be: a TIFF image or a Windows metafile behind the binary header, or an EPSI
//! bitmap in comments, stretched over the page.

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};

/// Largest EPSI bitmap side read (pixels).
const MAX_EPSI: usize = 8192;

/// A preview's document and how the user is told about it.
pub(super) struct Preview {
    pub document: Document,
    /// What was opened instead ("its preview was placed instead, …").
    pub what: &'static str,
}

/// The best preview of `bytes` (the whole file; `ps` its PostScript) over `frame`: the TIFF
/// image, else the metafile's vectors, else the EPSI bitmap.
pub(super) fn preview(bytes: &[u8], ps: &[u8], frame: Rect) -> Option<Preview> {
    const IMAGE: &str = "its preview was placed instead, at screen resolution";
    if let Some(document) = crate::sections(bytes).and_then(|(_, tiff)| tiff).and_then(tiff_rgba).and_then(|img| image_doc(img, frame)) {
        return Some(Preview { document, what: IMAGE });
    }
    if let Some(document) = crate::metafile_preview(bytes).and_then(|wmf| metafile_doc(wmf, frame)) {
        return Some(Preview { document, what: "its Windows metafile preview was opened instead, which may simplify the art" });
    }
    epsi(ps).and_then(|img| image_doc(img, frame)).map(|document| Preview { document, what: IMAGE })
}

fn tiff_rgba(tiff: &[u8]) -> Option<image::RgbaImage> {
    match image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff) {
        Ok(img) => Some(img.to_rgba8()),
        Err(_) => crate::tiff::palette_rgba(tiff),
    }
}

/// A document of `frame`'s size showing `img` stretched over it.
fn image_doc(img: image::RgbaImage, frame: Rect) -> Option<Document> {
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
    let mut n = Node::new(
        doc.alloc_id(),
        NodeKind::Image(Box::new(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None })),
    );
    n.name = Some("Preview".into());
    let layer = doc.layers.first().map(|l| l.id)?;
    doc.insert(Some(layer), 0, n).ok()?;
    Some(doc)
}

/// A document of `frame`'s size with the metafile's art stretched from its picture frame over it.
fn metafile_doc(wmf: &[u8], frame: Rect) -> Option<Document> {
    let mut doc = vectorcraft_metafile::import(wmf).ok()?.document;
    let from = doc.artboards.first()?.rect;
    if from.width() <= 0.0 || from.height() <= 0.0 {
        return None;
    }
    let a = Affine::translate(frame.origin().to_vec2())
        * Affine::scale_non_uniform(frame.width() / from.width(), frame.height() / from.height())
        * Affine::translate(-from.origin().to_vec2());
    for layer in &mut doc.layers {
        let layer = std::sync::Arc::make_mut(layer);
        for n in layer.children_mut().into_iter().flatten() {
            std::sync::Arc::make_mut(n).transform(a, true);
        }
    }
    if let Some(board) = doc.artboards.first_mut() {
        board.rect = frame;
    }
    doc.layers.iter().any(|l| l.children().is_some_and(|c| !c.is_empty())).then_some(doc)
}

/// The EPSI preview in `ps`'s comments (`%%BeginPreview: width height depth lines`, hex rows,
/// 0 white and the largest value black).
fn epsi(ps: &[u8]) -> Option<image::RgbaImage> {
    let text = String::from_utf8_lossy(ps);
    let mut lines = text.lines();
    let head = lines.find_map(|l| l.strip_prefix("%%BeginPreview:"))?;
    let n: Vec<usize> = head.split_whitespace().map_while(|w| w.parse().ok()).collect();
    let [w, h, depth, ..] = n[..] else { return None };
    if !(1..=MAX_EPSI).contains(&w) || !(1..=MAX_EPSI).contains(&h) || ![1, 2, 4, 8].contains(&depth) {
        return None;
    }
    let row = (w * depth).div_ceil(8);
    let mut hex = Vec::with_capacity((row * h * 2).min(ps.len()));
    for l in lines {
        if l.starts_with("%%EndPreview") || hex.len() >= row * h * 2 {
            break;
        }
        hex.extend(l.strip_prefix('%')?.bytes().filter(u8::is_ascii_hexdigit));
    }
    let bytes = super::lex::hex_decode(&hex)?;
    let max = (1u32 << depth) - 1;
    let mut img = image::RgbaImage::new(w as u32, h as u32);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let bit = y as usize * row * 8 + x as usize * depth;
        let byte = u32::from(bytes.get(bit / 8).copied().unwrap_or(0));
        let v = (byte >> (8 - depth - bit % 8)) & max;
        let grey = (255 - v * 255 / max) as u8;
        *px = image::Rgba([grey, grey, grey, 255]);
    }
    Some(img)
}
