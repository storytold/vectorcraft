use super::exporter::{UNDECODED, q, writable, xf};
use super::{Arc, BlendMode, Color, Exporter, Image, NodeKind, Paint, Rect, Size, Surface, cmyk, krilla};
use krilla::color::device_n::{Colorant, DeviceNSpace, Process};
use krilla::image::{BitsPerComponent, CustomImage, ImageColorspace};

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
    /// ([`crate::images::coverage_sized`]). `None` when the paint isn't a solid color or the pixels
    /// can't be decoded; cached like [`Self::load_image`]. With `interpolate` (and the export's own
    /// setting), viewers may smooth it.
    pub(super) fn colorized_image(&mut self, im: &vectorcraft_doc::ImageObject, paint: &Paint, interpolate: bool) -> Option<Image> {
        let Paint::Solid { color, swatch, tint } = paint else { return None };
        let size = placed_size(im);
        let interpolate = interpolate && self.interpolate;
        let mut cache = format!("{}\u{0}{paint:?}\u{0}{:.3}x{:.3}", im.key, size.0, size.1);
        if !interpolate && self.interpolate {
            cache.push_str("\u{0}pixelated");
        }
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let doc = self.doc;
        let (width, height, alpha) = doc
            .image_blob(im)
            .filter(|_| !self.left_out.contains(&im.key))
            .and_then(|b| b.coverage())
            .and_then(|(w, h, cover)| crate::images::coverage_sized(w, h, cover, size, self.compression))?;
        let n = alpha.len();
        let registration = swatch.as_deref() == Some(vectorcraft_color::swatch::REGISTRATION);
        let inks: Vec<(String, [f32; 4])> = if registration {
            let spots: Vec<String> = doc.swatches_iter().filter(|w| w.spot).map(|w| w.name.clone()).collect();
            spots.iter().filter_map(|name| self.spot_ink(name)).collect()
        } else {
            swatch.as_deref().and_then(|name| self.spot_ink(name)).into_iter().collect()
        };
        let spot_colorants =
            inks.into_iter().map(|(name, full)| Colorant::Spot { name, alternate: cmyk::Color::new(q(full[0]), q(full[1]), q(full[2]), q(full[3])) });
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
        let sized = [c.color.downsample, c.gray.downsample, c.mono.downsample].iter().any(|d| *d != crate::Downsample::None);
        let interpolate = interpolate && self.interpolate;
        let mut cache = if sized { format!("{key}\u{0}{:.3}x{:.3}", size.0, size.1) } else { key.to_string() };
        if !interpolate && self.interpolate {
            cache.push_str("\u{0}pixelated");
        }
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let doc = self.doc;
        let out = self.out.clone();
        let img = doc.image_blob(im).filter(|_| !self.left_out.contains(key)).and_then(|blob| {
            let cmyk_image = blob.is_cmyk();
            if !cmyk_image && self.convert && doc.color_mode == vectorcraft_doc::ColorMode::Cmyk {
                self.rgb_in_cmyk.images.insert(key.to_string());
            }
            let r = if cmyk_image {
                crate::images::recode_cmyk(blob, size, c, interpolate, out.cmyk_pixels(), None)
            } else {
                crate::images::recode(&blob.bytes, size, c, interpolate, out.pixels())
            };
            if let Some(w) = r.warning {
                self.warn(w);
            }
            r.image.or_else(|| embed(blob, interpolate, self.sixteen_bit))
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
        let plain =
            art.opacity >= 1.0 && art.blend == BlendMode::Normal && art.extras().mask.is_none() && !art.appearance.effects.iter().any(|e| e.visible);
        if m.invert || !plain || (mi.xf, mi.width, mi.height) != (im.xf, im.width, im.height) {
            return None;
        }
        let size = placed_size(im);
        let interpolate = interpolate && self.interpolate;
        let mut cache = format!("{}\u{0}{}\u{0}{}\u{0}{:.3}x{:.3}", im.key, mi.key, m.clip, size.0, size.1);
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
            return;
        }
        let Some(img) = img.or_else(|| self.load_image(im, placed_size(im), !pixelated)) else { return };
        let Some(size) = Size::from_wh(im.width.max(1) as f32, im.height.max(1) as f32) else { return };
        s.push_transform(&xf(im.xf));
        s.draw_image(img, size);
        s.pop();
    }
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

/// Image `blob` as it is: PNG, JPEG, GIF and WebP through the PDF writer (JPEG data unchanged),
/// other formats decoded. Without `sixteen_bit` (a file before PDF 1.5), a PNG of 16 bits a
/// sample is written at 8 bits ([`png_8_bit`]).
pub(super) fn embed(blob: &vectorcraft_doc::ImageBlob, interpolate: bool, sixteen_bit: bool) -> Option<Image> {
    let bytes = blob.bytes.as_ref().clone();
    let direct = match blob.mime.as_str() {
        "image/png" if !sixteen_bit && png_16_bit(&bytes) => png_8_bit(&bytes).and_then(|png| Image::from_png(png.into(), interpolate).ok()),
        "image/png" => Image::from_png(bytes.clone().into(), interpolate).ok(),
        "image/jpeg" | "image/jpg" => Image::from_jpeg(bytes.clone().into(), interpolate).ok(),
        "image/gif" => Image::from_gif(bytes.clone().into(), interpolate).ok(),
        "image/webp" => Image::from_webp(bytes.clone().into(), interpolate).ok(),
        _ => None,
    };
    direct.or_else(|| {
        let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (w, h) = rgba.dimensions();
        Some(Image::from_rgba8(rgba.into_raw(), w, h))
    })
}
