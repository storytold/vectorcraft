use super::exporter::{UNDECODED, UNWRITABLE_IMAGE, q, writable, xf};
use super::{Arc, BlendMode, Color, Exporter, Image, Node, NodeKind, Paint, Rect, Size, Surface, cmyk, krilla};
use krilla::color::device_n::{Colorant, DeviceNSpace, Process};
use krilla::image::{BitsPerComponent, CustomImage, ImageColorspace};

/// The byte count of a repeated color channel, within [`vectorcraft_doc::MAX_RASTER_ALLOC`].
fn tinted_color_len(samples: usize, pixels: usize) -> Option<usize> {
    let bytes = samples.checked_mul(pixels)?;
    (u64::try_from(bytes).ok()? <= vectorcraft_doc::MAX_RASTER_ALLOC).then_some(bytes)
}

/// A colorized image's pixels: one color in every pixel, shown as much as its soft mask says.
#[derive(Clone, Hash)]
struct Tinted {
    color: Arc<Vec<u8>>,
    alpha: Arc<Vec<u8>>,
    width: u32,
    height: u32,
    space: ImageColorspace,
    device_n: Option<DeviceNSpace>,
}

impl CustomImage for Tinted {
    fn color_channel(&self) -> &[u8] {
        &self.color
    }

    fn alpha_channel(&self) -> Option<&[u8]> {
        Some(&self.alpha)
    }

    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }

    fn color_space(&self) -> ImageColorspace {
        self.space
    }

    fn device_n_space(&self) -> Option<DeviceNSpace> {
        self.device_n.clone()
    }
}

impl Exporter<'_> {
    /// Image `im` colorized with `paint` ([`vectorcraft_doc::NodeExtras::colorize`]): every pixel
    /// in the paint's color, with the image's coverage ([`vectorcraft_doc::ImageBlob::coverage`])
    /// as its soft mask, so white pixels are transparent. A spot color is written as its tint of
    /// that ink (a DeviceN space of the one ink, which lists its Separation space), Registration in
    /// every process ink and every spot ink of the document, other colors in gray, RGB or CMYK as
    /// the Output settings write them. The coverage is resampled as the Compression settings ask
    /// ([`crate::images::coverage_sized`]). `None` when the paint isn't a solid color, the image is
    /// missing or left out, its pixels can't be decoded, or its color channel exceeds
    /// [`vectorcraft_doc::MAX_RASTER_ALLOC`]; cached by key and paint (and size when images are
    /// resampled), image rejections too.
    /// With `interpolate` (and the export's own setting), viewers may smooth it.
    pub(super) fn colorized_image(&mut self, im: &vectorcraft_doc::ImageObject, paint: &Paint, interpolate: bool) -> Option<Image> {
        let Paint::Solid { color, swatch, tint } = paint else { return None };
        let size = self.image_size(im);
        let interpolate = interpolate && self.interpolate;
        let mut cache = format!("{}\u{0}{paint:?}", im.key);
        if self.downsampled() {
            cache.push_str(&format!("\u{0}{:.3}x{:.3}", size.0, size.1));
        }
        if !interpolate && self.interpolate {
            cache.push_str("\u{0}pixelated");
        }
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let doc = self.doc;
        let Some((width, height, alpha)) = doc
            .image_blob(im)
            .filter(|_| !self.left_out.contains(&im.key))
            .and_then(|b| b.coverage())
            .and_then(|(w, h, cover)| crate::images::coverage_sized(w, h, cover, size, self.compression))
        else {
            self.images.insert(cache, None);
            return None;
        };
        let n = alpha.len();
        let registration = swatch.as_deref() == Some(vectorcraft_color::swatch::REGISTRATION);
        let inks: Vec<(String, [f32; 4])> = if registration {
            let spots: Vec<String> = doc.swatches_iter().filter(|w| w.spot).map(|w| w.name.clone()).collect();
            spots.iter().filter_map(|name| self.spot_ink(name)).collect()
        } else {
            swatch.as_deref().and_then(|name| self.spot_ink(name)).into_iter().collect()
        };
        let spot_colorants =
            inks.into_iter().map(|(name, full)| Colorant::Spot { name, alternate: cmyk::Color::new_f32(full[0], full[1], full[2], full[3]) });
        let device_n = if registration {
            DeviceNSpace::new(Process::ALL.into_iter().map(Colorant::Process).chain(spot_colorants))
        } else {
            DeviceNSpace::new(spot_colorants)
        };
        let (samples, space) = match &device_n {
            Some(d) if registration => (vec![q(*tint); d.colorants().len()], ImageColorspace::DeviceN),
            Some(_) => (vec![q(*tint)], ImageColorspace::DeviceN),
            None if registration => (vec![q(*tint); 4], ImageColorspace::Cmyk),
            None => match self.page_color(color) {
                Color::Gray { k } => (vec![q(1.0 - k)], ImageColorspace::Luma),
                Color::Cmyk { c, m, y, k } => ([c, m, y, k].map(q).to_vec(), ImageColorspace::Cmyk),
                other => (other.to_rgb().map(q).to_vec(), ImageColorspace::Rgb),
            },
        };
        if tinted_color_len(samples.len(), n).is_none() {
            self.images.insert(cache, None);
            return None;
        }
        let color = Arc::new(samples.repeat(n));
        let img = Image::from_custom(Tinted { color, alpha: Arc::new(alpha), width, height, space, device_n }, interpolate).ok();
        self.images.insert(cache, img.clone());
        img
    }
    /// Image `im` placed `size` points wide and high, resampled and compressed as the Compression
    /// settings say ([`crate::images::recode`], [`crate::images::recode_cmyk`] for CMYK images);
    /// cached by key (and size when images are resampled). With `interpolate` (and the export's
    /// own setting), viewers may smooth it. `None`, with [`UNDECODED`] among the warnings, for an
    /// image that is missing, can't be decoded or is in [`Self::left_out`].
    fn load_image(&mut self, im: &vectorcraft_doc::ImageObject, size: (f64, f64), interpolate: bool) -> Option<Image> {
        let key = im.key.as_str();
        let c = self.compression;
        let interpolate = interpolate && self.interpolate;
        let mut cache = if self.downsampled() { format!("{key}\u{0}{:.3}x{:.3}", size.0, size.1) } else { key.to_string() };
        if !interpolate && self.interpolate {
            cache.push_str("\u{0}pixelated");
        }
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let doc = self.doc;
        let out = self.out.clone();
        let profiles = !out.standard.cmyk_only();
        let img = doc.image_blob(im).filter(|_| !self.left_out.contains(key)).and_then(|blob| {
            let cmyk_image = blob.is_cmyk();
            if !cmyk_image && self.convert && doc.color_mode == vectorcraft_doc::ColorMode::Cmyk {
                self.rgb_in_cmyk.images.insert(key.to_string());
            }
            let r = if cmyk_image {
                crate::images::recode_cmyk(blob, size, c, interpolate, out.cmyk_pixels(), None)
            } else {
                crate::images::recode(&blob.bytes, size, c, interpolate, out.pixels(), profiles)
            };
            if let Some(w) = r.warning {
                self.warn(w);
            }
            r.image.or_else(|| embed(blob, interpolate, self.sixteen_bit, profiles))
        });
        if img.is_none() {
            self.warn(UNDECODED);
        }
        self.images.insert(cache, img.clone());
        img
    }

    /// CMYK image `im` with the alpha of its opacity mask `m`, when the mask is a greyscale image
    /// on the image's own pixel grid (an image's soft mask as it opens): the alpha is written as the
    /// image's soft mask, as an RGBA image's is, rather than as a luminosity soft mask over it,
    /// through which CoreGraphics draws CMYK images on black boxes (#2159). `None` for other
    /// images and masks; cached like [`Self::load_image`], `interpolate` as there.
    pub(super) fn masked_image(&mut self, im: &vectorcraft_doc::ImageObject, m: &vectorcraft_doc::OpacityMask, interpolate: bool) -> Option<Image> {
        let art = &m.art;
        let NodeKind::Image(mi) = &art.kind else { return None };
        // A color-masked image in the mask's art masks with the pixels it shows.
        let shown = self.doc.shown_image(art, mi);
        let mi = &*shown;
        let plain =
            art.opacity >= 1.0 && art.blend == BlendMode::Normal && art.extras().mask.is_none() && !art.appearance.effects.iter().any(|e| e.visible);
        if m.invert || !plain || (mi.xf, mi.width, mi.height) != (im.xf, im.width, im.height) {
            return None;
        }
        let size = self.image_size(im);
        let interpolate = interpolate && self.interpolate;
        let mut cache = format!("{}\u{0}{}\u{0}{}", im.key, mi.key, m.clip);
        if self.downsampled() {
            cache.push_str(&format!("\u{0}{:.3}x{:.3}", size.0, size.1));
        }
        if !interpolate && self.interpolate {
            cache.push_str("\u{0}pixelated");
        }
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let doc = self.doc;
        let img = doc.image_blob(im).filter(|b| b.is_cmyk()).and_then(|blob| {
            let alpha = crate::images::mask_alpha(doc.image_blob(mi)?, im.width, im.height, m.clip)?;
            let r = crate::images::recode_cmyk(blob, size, self.compression, interpolate, self.out.cmyk_pixels(), Some(alpha));
            if let (Some(w), Some(_)) = (r.warning, &r.image) {
                self.warn(w);
            }
            r.image
        });
        self.images.insert(cache, img.clone());
        img
    }

    /// Draw image `im`: `img` when given (its pixels already written), else as it is loaded; with
    /// `pixelated` ([`vectorcraft_doc::NodeExtras::pixelated`]) viewers are asked not to smooth it.
    pub(super) fn image(&mut self, s: &mut Surface, im: &vectorcraft_doc::ImageObject, img: Option<Image>, pixelated: bool) {
        // An image placed or sized beyond [`MAX_COORD`] is left out.
        if !writable(s, im.xf, Rect::new(0.0, 0.0, im.width.max(1).into(), im.height.max(1).into())) {
            if self.image_left_out.is_some() {
                self.image_left_out = Some(true);
                self.warn(UNWRITABLE_IMAGE);
            }
            return;
        }
        let Some(img) = img.or_else(|| self.load_image(im, self.image_size(im), !pixelated)) else {
            self.image_left_out = self.image_left_out.map(|_| true);
            return;
        };
        let Some(size) = Size::from_wh(im.width.max(1) as f32, im.height.max(1) as f32) else {
            if self.image_left_out.is_some() {
                self.image_left_out = Some(true);
                self.warn(UNWRITABLE_IMAGE);
            }
            return;
        };
        s.push_transform(&xf(im.xf));
        s.draw_image(img, size);
        s.pop();
    }

    /// Whether the Compression settings resample any kind of image.
    fn downsampled(&self) -> bool {
        let c = self.compression;
        [c.color.downsample, c.gray.downsample, c.mono.downsample].iter().any(|d| *d != crate::Downsample::None)
    }

    /// The size image `im` is resampled for: [`Self::warp_image_size`]'s while the pieces of a
    /// mesh-warped image of the same pixels are written, else the size it is placed at.
    fn image_size(&self, im: &vectorcraft_doc::ImageObject) -> (f64, f64) {
        match &self.warp_size {
            Some(w) if (w.key.as_str(), w.width, w.height) == (im.key.as_str(), im.width, im.height) => w.size,
            _ => placed_size(im),
        }
    }

    /// When `children` are the pieces of a mesh-warped image ([`Node::warp_piece_image`]) and the
    /// Compression settings resample images: the image they show, and the widest and highest any
    /// piece places it, which every piece's image is resampled for, so that the pieces paint one
    /// image. `None` for other children, pieces of different images, or a size that isn't finite
    /// and positive.
    pub(super) fn warp_image_size(&self, children: &[Arc<Node>]) -> Option<SharedSize> {
        if !self.downsampled() {
            return None;
        }
        let mut shared: Option<SharedSize> = None;
        for c in children {
            let n = c.warp_piece_image()?;
            let NodeKind::Image(im) = &n.kind else { return None };
            let im = self.doc.shown_image(n, im);
            let (w, h) = placed_size(&im);
            match &mut shared {
                Some(s) if (s.key.as_str(), s.width, s.height) == (im.key.as_str(), im.width, im.height) => {
                    s.size = (s.size.0.max(w), s.size.1.max(h));
                }
                Some(_) => return None,
                None => shared = Some(SharedSize { key: im.key.clone(), width: im.width, height: im.height, size: (w, h) }),
            }
        }
        shared.filter(|s| s.size.0.is_finite() && s.size.1.is_finite() && s.size.0 > 0.0 && s.size.1 > 0.0)
    }
}

/// The size the pixels of image `key` (`width` × `height` pixels) are resampled for while the
/// pieces of a mesh-warped image are written ([`Exporter::warp_image_size`]).
pub(super) struct SharedSize {
    key: String,
    width: u32,
    height: u32,
    size: (f64, f64),
}

/// The size image `im` is placed at: its pixel grid's sides through its transform.
fn placed_size(im: &vectorcraft_doc::ImageObject) -> (f64, f64) {
    let [a, b, c, d, ..] = im.xf.as_coeffs();
    (im.width as f64 * a.hypot(b), im.height as f64 * c.hypot(d))
}

/// Does PNG `bytes` have 16 bits a sample: the bit depth of its IHDR chunk, which follows the
/// signature?
pub(super) fn png_16_bit(bytes: &[u8]) -> bool {
    bytes.get(12..16) == Some(b"IHDR".as_slice()) && bytes.get(24) == Some(&16)
}

/// PNG `bytes` at 8 bits a sample: decoded, and encoded again as gray or RGB, with alpha when the
/// source has it, and with the source's ICC profile. `None` when it can't be decoded.
pub(super) fn png_8_bit(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::{ColorType, DynamicImage, ImageDecoder, ImageEncoder};
    let mut decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let icc = decoder.icc_profile().ok().flatten();
    let img = DynamicImage::from_decoder(decoder).ok()?;
    let img = match img.color() {
        ColorType::L8 | ColorType::L16 => DynamicImage::ImageLuma8(img.into_luma8()),
        ColorType::La8 | ColorType::La16 => DynamicImage::ImageLumaA8(img.into_luma_alpha8()),
        ColorType::Rgb8 | ColorType::Rgb16 => DynamicImage::ImageRgb8(img.into_rgb8()),
        _ => DynamicImage::ImageRgba8(img.into_rgba8()),
    };
    let mut png = vec![];
    let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc).ok()?;
    }
    img.write_with_encoder(encoder).ok()?;
    Some(png)
}

/// Most pixels an image may have to be embedded: twice what the app reads (64 million), so every image
/// it holds is still written.
const MAX_EMBED_PIXELS: u64 = 1 << 27;

/// Image `blob` as it is: PNG, JPEG, GIF and WebP through the PDF writer (JPEG data unchanged),
/// other formats (a TIFF) decoded with their 16-bit samples and, when `profiles` allows it (not in
/// PDF/X-1a), the ICC profile they carry ([`crate::images::decoded_image`]). Without `profiles`, an
/// image of any format that carries a gray or RGB profile is decoded too, and written without it.
/// Without `sixteen_bit` (a file before PDF 1.5), a PNG or another image of 16 bits a sample is
/// written at 8 bits ([`png_8_bit`]).
pub(super) fn embed(blob: &vectorcraft_doc::ImageBlob, interpolate: bool, sixteen_bit: bool, profiles: bool) -> Option<Image> {
    // The header's size first: the writer's decoders allocate what it declares, and the encoded
    // bytes of an image that is far larger than any the app reads may be a few kilobytes.
    let size = image::ImageReader::new(std::io::Cursor::new(blob.bytes.as_slice())).with_guessed_format().ok()?.into_dimensions();
    if size.is_ok_and(|(w, h)| u64::from(w) * u64::from(h) > MAX_EMBED_PIXELS) {
        return None;
    }
    let bytes = blob.bytes.as_ref().clone();
    // The PDF writer keeps a gray or RGB image's ICC profile: without `profiles`, such an image is
    // decoded and written without it.
    let tagged = || {
        let space = crate::images::source_icc(&bytes).and_then(|icc| vectorcraft_doc::cmyk::icc_color_space(&icc));
        matches!(space.as_ref(), Some(b"GRAY" | b"RGB "))
    };
    let mime = if !profiles && tagged() { "" } else { blob.mime.as_str() };
    let direct = match mime {
        "image/png" if !sixteen_bit && png_16_bit(&bytes) => png_8_bit(&bytes).and_then(|png| Image::from_png(png.into(), interpolate).ok()),
        "image/png" => Image::from_png(bytes.clone().into(), interpolate).ok(),
        "image/jpeg" | "image/jpg" => Image::from_jpeg(bytes.clone().into(), interpolate).ok(),
        "image/gif" => Image::from_gif(bytes.clone().into(), interpolate).ok(),
        "image/webp" => Image::from_webp(bytes.clone().into(), interpolate).ok(),
        _ => None,
    };
    direct.or_else(|| crate::images::decoded_image(&bytes, interpolate, sixteen_bit, profiles))
}

#[cfg(test)]
mod tests {
    use super::{Exporter, tinted_color_len};
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Document, ImageBlob, ImageObject};
    use vectorcraft_geom::Affine;

    #[test]
    fn an_image_whose_header_declares_far_more_pixels_than_the_app_reads_is_left_out() {
        // A real 8 × 8 PNG whose IHDR (width, height at 16..24) claims 65535 × 65535, with its CRC fixed.
        let mut png = vec![];
        image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 8)).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png[16..24].copy_from_slice(&[0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF]);
        let crc =
            !png[12..29].iter().fold(!0u32, |c, b| (0..8).fold(c ^ u32::from(*b), |c, _| if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }));
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        assert!(super::embed(&ImageBlob::png(png), false, true, true).is_none());
    }

    #[test]
    fn colorized_image_caches_rejected_images() {
        let mut doc = Document::new(10.0, 10.0);
        doc.images.insert("bad".into(), ImageBlob::png(vec![1, 2, 3]));
        doc.images.insert("left-out".into(), ImageBlob::png(vec![1, 2, 3]));
        let settings = crate::PdfSettings::default();
        let mut exporter = Exporter::new(&doc, &settings);
        exporter.left_out.insert("left-out".into());
        let paint = Paint::solid(Color::rgb(1.0, 0.0, 0.0));

        for key in ["missing", "bad", "left-out"] {
            let image =
                ImageObject { key: key.into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
            assert!(exporter.colorized_image(&image, &paint, true).is_none(), "{key}");
            let cache = format!("{key}\u{0}{paint:?}");
            assert!(matches!(exporter.images.get(&cache), Some(None)), "rejection for {key} was not cached");
            assert!(exporter.colorized_image(&image, &paint, true).is_none(), "second call for {key}");
        }
        assert_eq!(exporter.images.len(), 3);
    }

    /// Without downsampling, a colorized image and a CMYK image with a grid mask are made once for
    /// every size they are placed at.
    #[test]
    fn colorized_and_grid_masked_images_are_made_once_for_every_size_without_downsampling() {
        let gray = |pixels: Vec<u8>| {
            let mut png = vec![];
            image::GrayImage::from_raw(2, 2, pixels).unwrap().write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            ImageBlob::png(png)
        };
        let mut doc = Document::new(100.0, 100.0);
        doc.images.insert("pic".into(), gray(vec![0, 255, 128, 64]));
        doc.images.insert("alpha".into(), gray(vec![255, 128, 0, 64]));
        let inks = vectorcraft_doc::cmyk::Inks::new(2, 2, [0u8, 102, 255, 0].repeat(4)).unwrap();
        doc.images.insert("inks".into(), ImageBlob::cmyk_tiff(&inks).unwrap());
        let settings = crate::PdfSettings::default();
        let mut exporter = Exporter::new(&doc, &settings);
        let paint = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
        let image = |key: &str, scale: f64| ImageObject {
            key: key.into(),
            width: 2,
            height: 2,
            xf: Affine::scale(scale),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        for scale in [10.0, 20.0] {
            assert!(exporter.colorized_image(&image("pic", scale), &paint, true).is_some(), "colorized at {scale}");
            let art = vectorcraft_doc::Node::new(doc.layers[0].id, vectorcraft_doc::NodeKind::Image(Box::new(image("alpha", scale))));
            let mask = vectorcraft_doc::OpacityMask::new(art, true);
            assert!(exporter.masked_image(&image("inks", scale), &mask, true).is_some(), "masked at {scale}");
        }
        assert_eq!(exporter.images.len(), 2, "{:?}", exporter.images.keys().collect::<Vec<_>>());
    }

    #[test]
    fn tinted_color_rejects_oversized_and_overflowing_buffers() {
        let max = usize::try_from(vectorcraft_doc::MAX_RASTER_ALLOC).unwrap_or(usize::MAX);
        assert_eq!(tinted_color_len(4, max / 4), Some(max));
        assert_eq!(tinted_color_len(4, max / 4 + 1), None);
        assert_eq!(tinted_color_len(4, usize::MAX), None);
    }
}
