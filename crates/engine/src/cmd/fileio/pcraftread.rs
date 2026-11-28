//! `.pcraft` documents (a layered raster document in a ZIP bundle) for File › Open and File ›
//! Place, read with craft-doc and stored as PNG. The image is, in this order:
//!
//! 1. the bundle's full-size composite, when its manifest names one;
//! 2. the canvas combined from its layers, when the document is RGB at 8 or 16 bits and every
//!    visible layer is a raster layer that blends Normal ([`combine`]);
//! 3. otherwise its composite preview, with a note that names the preview's size, the canvas's
//!    and why the layers were not combined, unless the preview is the canvas's size.
//!
//! A full-size composite or preview that the manifest names but the bundle does not hold, or that
//! does not decode, is an error, as is a full-size composite of another size than the canvas. The
//! image is sized by the document's resolution, so a preview places at the canvas's size (at
//! 1 ppi when that would take less).

use std::fmt;

use craft_doc::{Limits, RenditionKind, Samples};
use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};

use super::super::Result;
use super::load::{MAX_RASTER_ALLOC, RasterImage, as_png, err, raster_image};

/// Whether `bytes` are a `.pcraft` bundle: a ZIP archive holding the manifest craft-doc reads.
pub(crate) fn is_pcraft(bytes: &[u8]) -> bool {
    craft_doc::sniff(bytes).is_some_and(|kind| kind.extension() == "pcraft")
}

/// The image of `.pcraft` bundle `bytes` (see the module documentation), with
/// [`RasterImage::note`] saying so when only a preview smaller than the canvas was read.
pub(super) fn image(bytes: &[u8]) -> Result<RasterImage> {
    let limits = limits();
    let canvas: Canvas = craft_doc::pcraft_document(bytes, &limits).map_err(err)?;
    let (w, h) = (canvas.size.width, canvas.size.height);
    if w == 0 || h == 0 {
        return Err(err(format!("the .pcraft document's canvas of {w} × {h} pixels is empty")));
    }
    let composite = craft_doc::rendition(bytes, RenditionKind::Composite, &limits).map_err(err)?;
    if let Some(full) = composite.as_ref().filter(|c| c.is_full_composite) {
        let img = stored_png(&full.bytes, canvas.ppi(), "full-size image")?;
        if (img.width, img.height) != (w, h) {
            return Err(err(format!("the .pcraft document's full-size image is {} × {} pixels, not {w} × {h} as its canvas", img.width, img.height)));
        }
        return Ok(img);
    }
    let reason = match combine(bytes, &canvas, &limits)? {
        Ok(img) => return Ok(img),
        Err(reason) => reason,
    };
    let Some(preview) = composite else {
        return Err(err(format!("the .pcraft document has no full-size image or preview, and its layers can't be combined: {reason}")));
    };
    let (pw, ph) = vectorcraft_doc::header_size(&preview.bytes).ok_or_else(|| err("the .pcraft document's preview isn't a PNG image"))?;
    // The preview covers the canvas: its resolution is the document's (72 ppi without a usable
    // one) scaled by its size, and at least 1 ppi, the least a stored image's resolution reads as.
    let (x, y) = canvas.ppi().unwrap_or((72.0, 72.0));
    let ppi = ((x * f64::from(pw) / f64::from(w)).max(1.0), (y * f64::from(ph) / f64::from(h)).max(1.0));
    let mut img = stored_png(&preview.bytes, Some(ppi), "preview")?;
    let reduced = (img.width, img.height) != (w, h);
    let (pw, ph) = (img.width, img.height);
    img.note = reduced.then(|| format!("only the {pw} × {ph} preview of this {w} × {h} pixel .pcraft document was read: {reason}"));
    Ok(img)
}

/// The limits of one read: the decoded allowance for an entry, and twice that for everything one
/// call reads and decodes.
fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_entry = MAX_RASTER_ALLOC;
    limits.max_total = MAX_RASTER_ALLOC.saturating_mul(2);
    limits
}

/// PNG `png` from the bundle (its `what`) as a stored image whose PNG records resolution `ppi`. The
/// image's resolution is the one read back from the PNG, as the Links commands read it later.
fn stored_png(png: &[u8], ppi: Option<(f64, f64)>, what: &str) -> Result<RasterImage> {
    if craft_image_codecs::detect(png) != Some(craft_image_codecs::Format::Png) {
        return Err(err(format!("the .pcraft document's {what} isn't a PNG image")));
    }
    match ppi {
        Some(r) => raster_image(&super::ppi::with_png_resolution(png, r)),
        None => raster_image(png),
    }
}

/// The document members that size the image and decide whether its layers can be combined.
#[derive(Deserialize)]
struct Canvas {
    size: Size,
    resolution_dpi: f32,
    mode: String,
    depth: String,
    #[serde(default)]
    icc_profile: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct Size {
    width: u32,
    height: u32,
}

impl Canvas {
    /// The document's resolution, when it is a usable one.
    fn ppi(&self) -> Option<(f64, f64)> {
        let dpi = f64::from(self.resolution_dpi);
        (dpi.is_finite() && (1.0..=100_000.0).contains(&dpi)).then_some((dpi, dpi))
    }
}

/// The document's layers, bottom first.
#[derive(Deserialize)]
struct Layers {
    layers: Vec<Layer>,
}

/// The members of a layer that change how it draws. Members that don't (its name, locks, label,
/// id, links, XMP and the blocks kept for PSD files) are skipped; any other member is kept in
/// `unknown`, so a layer with a setting this reader doesn't know is not combined.
#[derive(Default)]
struct Layer {
    visible: bool,
    blend: String,
    opacity: f32,
    fill_opacity: f32,
    clipped: bool,
    mask: Option<Mask>,
    vector_mask: Option<Toggle>,
    effects: Option<Effects>,
    kind: String,
    frame: bool,
    advanced_blending: bool,
    unknown: Option<String>,
}

#[derive(Deserialize)]
struct Mask {
    enabled: bool,
    density: f32,
    feather: f32,
}

#[derive(Deserialize)]
struct Toggle {
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct Effects {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    items: Vec<IgnoredAny>,
}

/// A layer's `content`: its kind. A group's children are skipped: a visible group is not combined.
#[derive(Deserialize)]
struct Content {
    kind: String,
}

impl<'de> Deserialize<'de> for Layer {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct LayerVisitor;
        impl<'de> Visitor<'de> for LayerVisitor {
            type Value = Layer;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a layer object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Layer, A::Error> {
                let mut l = Layer { fill_opacity: 1.0, ..Layer::default() };
                let (mut visible, mut blend, mut opacity, mut kind) = (None, None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "visible" => visible = Some(map.next_value()?),
                        "blend" => blend = Some(map.next_value()?),
                        "opacity" => opacity = Some(map.next_value()?),
                        "fill_opacity" => l.fill_opacity = map.next_value()?,
                        "clipped" => l.clipped = map.next_value()?,
                        "mask" => l.mask = map.next_value()?,
                        "vector_mask" => l.vector_mask = map.next_value()?,
                        "effects" => l.effects = map.next_value()?,
                        "content" => kind = Some(map.next_value::<Content>()?.kind),
                        "frame" => l.frame = map.next_value::<Option<IgnoredAny>>()?.is_some(),
                        "excluded_channels" => l.advanced_blending |= map.next_value::<u32>()? != 0,
                        "blend_if" | "advanced" => l.advanced_blending |= map.next_value::<Option<IgnoredAny>>()?.is_some(),
                        "id" | "name" | "locks" | "label" | "psd_blocks" | "psd_id" | "fill_cache" | "link_group" | "video" | "xmp" => {
                            map.next_value::<IgnoredAny>()?;
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                            l.unknown.get_or_insert(key);
                        }
                    }
                }
                l.visible = visible.ok_or_else(|| serde::de::Error::missing_field("visible"))?;
                l.blend = blend.ok_or_else(|| serde::de::Error::missing_field("blend"))?;
                l.opacity = opacity.ok_or_else(|| serde::de::Error::missing_field("opacity"))?;
                l.kind = kind.ok_or_else(|| serde::de::Error::missing_field("content"))?;
                Ok(l)
            }
        }
        d.deserialize_map(LayerVisitor)
    }
}

/// Why visible layer `l` can't be combined by [`combine`], or `None` when it can.
fn unsupported(l: &Layer) -> Option<String> {
    let unit = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
    if let Some(key) = &l.unknown {
        return Some(format!("a visible layer has a setting this version doesn't read (`{key}`)"));
    }
    if l.kind != "raster" {
        let kind = match l.kind.as_str() {
            "group" => "a visible layer group",
            "text" => "a visible type layer",
            "shape" => "a visible shape layer",
            "smart" => "a visible smart object",
            "adjustment" => "a visible adjustment layer",
            "fill" => "a visible fill layer",
            _ => "a visible layer of a kind this version doesn't read",
        };
        return Some(format!("the document has {kind}"));
    }
    let reason = if l.blend != "Normal" {
        "a visible layer blends in a mode other than Normal"
    } else if !unit(l.opacity) || !unit(l.fill_opacity) {
        "a visible layer's opacity is out of range"
    } else if l.clipped {
        "a visible layer is clipped to the layer below it"
    } else if l.effects.as_ref().is_some_and(|e| e.enabled && !e.items.is_empty()) {
        "a visible layer has layer effects"
    } else if l.vector_mask.as_ref().is_some_and(|v| v.enabled) {
        "a visible layer has a vector mask"
    } else if l.mask.as_ref().is_some_and(|m| m.enabled && (m.feather != 0.0 || !unit(m.density))) {
        "a visible layer has a feathered layer mask"
    } else if l.advanced_blending {
        "a visible layer uses advanced blending options"
    } else if l.frame {
        "a visible layer has a frame"
    } else {
        return None;
    };
    Some(reason.into())
}

/// The canvas of `.pcraft` bundle `bytes` combined from its layers, or why it can't be: the
/// document must be RGB at 8 or 16 bits, the canvas's pixels as 32-bit floating-point RGBA within
/// [`MAX_RASTER_ALLOC`], and each visible layer a raster layer that blends Normal at an opacity and
/// fill opacity between 0 and 1, with no clipping, layer effects, vector mask, frame or advanced
/// blending options, and a layer mask that is off or not feathered. Hidden layers are not read.
/// Each layer is laid over the layers below it as the straight-alpha Normal blend lays a source
/// over a backdrop, its alpha scaled by its opacity, its fill opacity and its layer mask (the mask
/// value scaled toward opaque by the mask's density). An RGB ICC profile the document names is
/// kept in the PNG; another profile is a reason not to combine. A visible layer without saved
/// pixels, or with samples other than 8 or 16 bits, is a reason too.
fn combine(bytes: &[u8], canvas: &Canvas, limits: &Limits) -> Result<std::result::Result<RasterImage, String>> {
    let no = |reason: &str| Ok(Err(reason.to_string()));
    if canvas.mode != "Rgb" {
        return Ok(Err(format!("the document's color mode is {}, not RGB", canvas.mode)));
    }
    let deep = match canvas.depth.as_str() {
        "U8" => false,
        "U16" => true,
        _ => return no("the document stores 32-bit samples"),
    };
    let (w, h) = (canvas.size.width, canvas.size.height);
    let pixels = u64::from(w) * u64::from(h);
    if pixels.saturating_mul(16) > MAX_RASTER_ALLOC {
        return no("the canvas is too large to combine its layers");
    }
    let Layers { layers } = craft_doc::pcraft_document(bytes, limits).map_err(err)?;
    let visible: Vec<usize> = layers.iter().enumerate().filter(|(_, l)| l.visible).map(|(i, _)| i).collect();
    if let Some(reason) = visible.iter().find_map(|&i| unsupported(&layers[i])) {
        return Ok(Err(reason));
    }
    let icc = match canvas.icc_profile {
        Some(_) => craft_doc::pcraft_icc_profile(bytes, limits).map_err(err)?,
        None => None,
    };
    if icc.as_deref().is_some_and(|icc| vectorcraft_doc::cmyk::icc_color_space(icc) != Some(*b"RGB ")) {
        return no("the document's color profile isn't an RGB profile");
    }
    let n = usize::try_from(pixels).map_err(err)?;
    let mut out = vec![[0.0f32; 4]; n];
    for i in visible {
        let l = &layers[i];
        let Some(px) = craft_doc::pcraft_layer_rgba(bytes, &[i], limits).map_err(err)? else {
            return no("a visible layer has no saved pixels");
        };
        if (px.width, px.height) != (w, h) || px.rgba.len() != n * 4 {
            return Err(err("a layer of the .pcraft document doesn't cover its canvas"));
        }
        let Some(rgba) = Levels::of(&px.rgba) else { return no("a visible layer stores 32-bit samples") };
        let mask = match px.mask.as_ref().filter(|m| m.enabled) {
            Some(m) => match Levels::of(&m.alpha).filter(|_| m.alpha.len() == n) {
                Some(levels) => Some((levels, m.density)),
                None => return no("a visible layer's mask stores 32-bit samples"),
            },
            None => None,
        };
        let opacity = l.opacity * l.fill_opacity;
        for (p, b) in out.iter_mut().enumerate() {
            let coverage = mask.as_ref().map_or(1.0, |(m, density)| 1.0 - density * (1.0 - m.at(p)));
            let s = [rgba.at(p * 4), rgba.at(p * 4 + 1), rgba.at(p * 4 + 2), rgba.at(p * 4 + 3)];
            *b = over(*b, s, opacity * coverage);
        }
    }
    let img: image::DynamicImage = if deep {
        let samples = out.iter().flat_map(|p| level16(*p)).collect();
        image::ImageBuffer::from_raw(w, h, samples)
            .map(image::DynamicImage::ImageRgba16)
            .ok_or_else(|| err("the combined layers don't fit the canvas"))?
    } else {
        let samples = out.iter().flat_map(|p| level8(*p)).collect();
        image::ImageBuffer::from_raw(w, h, samples)
            .map(image::DynamicImage::ImageRgba8)
            .ok_or_else(|| err("the combined layers don't fit the canvas"))?
    };
    drop(out);
    as_png(img, icc, canvas.ppi()).map(Ok)
}

/// 8- or 16-bit samples read as levels from 0 to 1.
enum Levels<'a> {
    U8(&'a [u8]),
    U16(&'a [u16]),
}

impl<'a> Levels<'a> {
    fn of(s: &'a Samples) -> Option<Self> {
        match s {
            Samples::U8(v) => Some(Levels::U8(v)),
            Samples::U16(v) => Some(Levels::U16(v)),
            _ => None,
        }
    }

    fn at(&self, i: usize) -> f32 {
        match self {
            Levels::U8(v) => v.get(i).map_or(0.0, |&v| f32::from(v) / 255.0),
            Levels::U16(v) => v.get(i).map_or(0.0, |&v| f32::from(v) / 65535.0),
        }
    }
}

/// Straight-alpha source `s` laid over backdrop `b` with the Normal blend, the source's alpha
/// scaled by `k`.
fn over(b: [f32; 4], s: [f32; 4], k: f32) -> [f32; 4] {
    let sa = s[3] * k;
    if sa <= 0.0 {
        return b;
    }
    let kb = b[3] * (1.0 - sa);
    let a = sa + kb;
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mix = |c: usize| (kb * b[c] + sa * s[c]) / a;
    [mix(0), mix(1), mix(2), a]
}

fn level8(p: [f32; 4]) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if p[3] <= 0.0 { [0; 4] } else { [q(p[0]), q(p[1]), q(p[2]), q(p[3])] }
}

fn level16(p: [f32; 4]) -> [u16; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    if p[3] <= 0.0 { [0; 4] } else { [q(p[0]), q(p[1]), q(p[2]), q(p[3])] }
}
