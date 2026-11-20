//! The previews an EPS file carries for apps that don't read PostScript, opened when its
//! PostScript can't be: a TIFF image or a Windows metafile behind the binary header, or an EPSI
//! bitmap in comments, stretched over the page.

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};

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

/// Most pixels a TIFF preview may have: its RGBA copy is four bytes each besides the decoded image.
const MAX_PREVIEW_PIXELS: u64 = 1 << 26;

fn tiff_rgba(tiff: &[u8]) -> Option<image::RgbaImage> {
    let mut opts = craft_image_codecs::DecodeOptions::default();
    opts.limits.max_pixels = MAX_PREVIEW_PIXELS;
    match craft_image_codecs::decode_as_with(craft_image_codecs::Format::Tiff, tiff, &opts) {
        Ok(img) => image::RgbaImage::from_raw(img.width(), img.height(), img.to_rgba8()),
        Err(craft_image_codecs::CodecError::LimitExceeded(_)) => None,
        Err(_) => crate::tiff::palette_rgba(tiff),
    }
}

/// A document of `frame`'s size showing `img` stretched over it.
fn image_doc(img: image::RgbaImage, frame: Rect) -> Option<Document> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let raw = craft_image_codecs::Image::from_raw(w, h, craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, img.into_raw())
        .ok()?;
    let png = craft_image_codecs::encode(&raw, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).ok()?;
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
/// The shared decoder intentionally tolerates blank preview lines and CR-only line endings;
/// unlike the former local parser, neither changes the decoded samples.
fn epsi(ps: &[u8]) -> Option<image::RgbaImage> {
    let bitmap = craft_eps::epsi(ps)?;
    let rgba = bitmap.gray.into_iter().flat_map(|g| [g, g, g, 255]).collect();
    image::RgbaImage::from_raw(bitmap.width, bitmap.height, rgba)
}

#[cfg(test)]
mod tests {
    use super::epsi;

    #[test]
    fn shared_epsi_depths_keep_row_padding_and_opaque_grayscale() {
        for (depth, row, gray) in [(1, "40", [255, 0, 255]), (2, "18", [255, 170, 85]), (4, "05a0", [255, 170, 85]), (8, "0055aa", [255, 170, 85])] {
            let source = format!("%%BeginPreview: 3 2 {depth} 2\n% {row}\n% {row}\n%%EndPreview\n");
            let image = epsi(source.as_bytes()).expect("ordinary EPSI preview");
            assert_eq!(image.dimensions(), (3, 2));
            let expected: Vec<u8> = gray.into_iter().cycle().take(6).flat_map(|g| [g, g, g, 255]).collect();
            assert_eq!(image.as_raw(), &expected, "depth {depth}");
        }
    }

    #[test]
    fn shared_epsi_intentionally_accepts_blank_and_cr_only_lines() {
        let ordinary = "%%BeginPreview: 3 2 2 2\n% 18\n% e4\n%%EndPreview\n";
        let expected = epsi(ordinary.as_bytes()).expect("ordinary preview");
        for source in [ordinary.replace('\n', "\r"), ordinary.replace("% e4", "\n% e4")] {
            let image = epsi(source.as_bytes()).expect("documented shared decoder tolerance");
            assert_eq!(image, expected);
        }
        let gray = [255, 170, 85, 0, 85, 170];
        let rgba: Vec<u8> = gray.into_iter().flat_map(|g| [g, g, g, 255]).collect();
        assert_eq!(expected.as_raw(), &rgba);
    }
}
