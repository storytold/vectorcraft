//! File › Open of a PSD or PSB file with its layers as objects. The import option that converts
//! layers to objects
//! (<https://helpx.adobe.com/illustrator/desktop/add-and-import-files/import-from-other-apps/photoshop-import-options.html>)
//! keeps as much of the layer structure as it can, merges the layers of a clipping mask into one
//! layer, and discards hidden layers that need merging; the rest of what this module does is
//! VectorCraft's own. Read from the layer records of the public PSD file format specification
//! (<https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/>).
//!
//! Each layer becomes a layer of the document, with the layer's name, visibility, opacity and blend
//! mode, holding its pixels as an image at their place on the canvas. A layer group becomes a layer
//! whose sublayers are the group's layers (a pass-through group lets them blend with what is below
//! it; any other group is isolated). A layer mask becomes the layer's opacity mask: a gray image
//! whose outside is hidden or shown as the mask's default color says, kept and switched off when
//! the mask is disabled. A layer's opacity is multiplied by its fill opacity, except in a layer
//! that others are clipped to, whose fill opacity fades only its own pixels. A solid color fill
//! layer that stores no pixels and has no vector mask comes in as an image of its color over the
//! canvas (an RGB, CMYK, Grayscale, Lab or HSB color); type, shape and other fill layers come in as
//! the pixels the file stores for them.
//!
//! Layers clipped to the layer below them are merged into it as one image (named after them all,
//! joined with " + "), shown where the layer below has pixels. Layers clipped to an adjustment
//! layer, or to a fill layer drawn over the whole canvas, show over the whole canvas. A layer in a
//! blend mode VectorCraft doesn't have (Dissolve, Linear Burn, Linear Dodge, Vivid Light, Linear
//! Light, Pin Light, Hard Mix, Subtract, Divide, Darker Color, Lighter Color) is merged with the
//! layers below it in its group. A Dissolve layer's partly transparent pixels are merged as opaque
//! or transparent, opaque at the share of its pixels their transparency gives, in a fixed pattern
//! that differs from layer to layer. When a merge's canvases would need more memory than allowed,
//! the layer comes in alone: in the Normal mode, or unclipped.
//! Hidden layers are left out unless `hidden` asks for them. A hidden layer that would need a merge
//! (one in a mode VectorCraft lacks, one that layers are clipped to, one clipped to another, or
//! one below a merged layer) is always left out, with the layers clipped to it. Adjustment layers,
//! fill layers that store no pixels and whose fill isn't read or whose shape is a vector mask,
//! layer effects that are switched on, knockouts and vector masks aren't applied; the warnings
//! count them and the hidden layers left out.
//!
//! The file is untrusted: every count, length and size is checked before anything is allocated,
//! groups nest at most [`MAX_DEPTH`] deep (layers deeper than a document may nest merge into the
//! group around them, with a warning), each layer's pixels, like the canvases one step of a
//! merge holds together, must fit the memory allowed, and all the pixels decoded and the canvases
//! the merges make must fit [`TOTAL_DECODED`] times that.

use std::cell::Cell;
use std::sync::Arc;

use image::RgbaImage;
use vectorcraft_color::BlendMode;
use vectorcraft_color::blend::{blend_rgb, composite};
use vectorcraft_color::cms::{Lab, lab::lab_to_srgb};
use vectorcraft_doc::{Document, ImageObject, LAYER_COLORS, LayerColor, Node, NodeKind, OpacityMask};
use vectorcraft_geom::Affine;

use super::load::{RasterImage, as_png};
use super::psdread::{Header, Reader, byte, header, layer_channel, layer_info, long_key, row_bytes, sample, to_rgb};
use crate::cmd::place::text::mac_roman;

/// Most layer groups inside one another.
pub(super) const MAX_DEPTH: usize = 64;
/// The pixels all layers and masks decode to, merges' second readings included, and the canvases
/// the merges make may take this many times the memory allowed for one image.
const TOTAL_DECODED: u64 = 8;
/// Most channels one layer has (colors, transparency and masks), as many as a document may.
const MAX_CHANNELS: usize = 56;
/// The fewest bytes a layer record takes.
const MIN_RECORD: usize = 34;
const DAMAGED: &str = "the PSD file's layer records are damaged";

/// The keys of the tagged blocks that make a layer an adjustment layer.
const ADJUSTMENTS: [&[u8; 4]; 18] = [
    b"levl", b"curv", b"brit", b"blnc", b"hue ", b"hue2", b"selc", b"mixr", b"grdm", b"phfl", b"expA", b"vibA", b"thrs", b"nvrt", b"post", b"clrL",
    b"blwh", b"CgEd",
];

/// A layer's blend mode.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Blend {
    Mode(BlendMode),
    /// A group's Pass Through.
    Pass,
    /// A mode VectorCraft doesn't have: the layer is merged with the layers below it.
    Other(Other),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Other {
    Dissolve,
    LinearBurn,
    LinearDodge,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Subtract,
    Divide,
    DarkerColor,
    LighterColor,
}

impl Other {
    fn label(self) -> &'static str {
        match self {
            Other::Dissolve => "Dissolve",
            Other::LinearBurn => "Linear Burn",
            Other::LinearDodge => "Linear Dodge",
            Other::VividLight => "Vivid Light",
            Other::LinearLight => "Linear Light",
            Other::PinLight => "Pin Light",
            Other::HardMix => "Hard Mix",
            Other::Subtract => "Subtract",
            Other::Divide => "Divide",
            Other::DarkerColor => "Darker Color",
            Other::LighterColor => "Lighter Color",
        }
    }
}

/// The blend mode a record's key names (an unknown key: Normal).
fn blend(key: &[u8; 4]) -> Blend {
    use BlendMode as M;
    let mode = match key {
        b"pass" => return Blend::Pass,
        b"diss" => return Blend::Other(Other::Dissolve),
        b"lbrn" => return Blend::Other(Other::LinearBurn),
        b"lddg" => return Blend::Other(Other::LinearDodge),
        b"vLit" => return Blend::Other(Other::VividLight),
        b"lLit" => return Blend::Other(Other::LinearLight),
        b"pLit" => return Blend::Other(Other::PinLight),
        b"hMix" => return Blend::Other(Other::HardMix),
        b"fsub" => return Blend::Other(Other::Subtract),
        b"fdiv" => return Blend::Other(Other::Divide),
        b"dkCl" => return Blend::Other(Other::DarkerColor),
        b"lgCl" => return Blend::Other(Other::LighterColor),
        b"dark" => M::Darken,
        b"mul " => M::Multiply,
        b"idiv" => M::ColorBurn,
        b"lite" => M::Lighten,
        b"scrn" => M::Screen,
        b"div " => M::ColorDodge,
        b"over" => M::Overlay,
        b"sLit" => M::SoftLight,
        b"hLit" => M::HardLight,
        b"diff" => M::Difference,
        b"smud" => M::Exclusion,
        b"hue " => M::Hue,
        b"sat " => M::Saturation,
        b"colr" => M::Color,
        b"lum " => M::Luminosity,
        _ => M::Normal,
    };
    Blend::Mode(mode)
}

/// A rectangle of the canvas, in pixels: top, left, bottom, right.
#[derive(Clone, Copy, Debug, Default)]
struct Bounds {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
}

impl Bounds {
    fn read(r: &mut Reader<'_>) -> Result<Self, String> {
        let mut v = || r.u32().map(|v| v as i32);
        Ok(Self { top: v()?, left: v()?, bottom: v()?, right: v()? })
    }
    /// Width and height (0 when empty or inverted).
    fn size(self) -> (usize, usize) {
        let side = |a: i32, b: i32| usize::try_from(i64::from(b) - i64::from(a)).unwrap_or(0);
        (side(self.left, self.right), side(self.top, self.bottom))
    }
}

/// What a layer holds besides plain pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Content {
    Pixels,
    /// A type layer: its pixels come in.
    Type,
    /// An adjustment layer: not applied.
    Adjustment,
    /// A solid color fill layer: its stored pixels, or its color over the whole canvas when it
    /// stores none (nothing when the color can't be read, or when its shape is a vector mask).
    Solid(Option<Paint>),
    /// A gradient or pattern fill layer: its stored pixels, if any.
    Fill,
}

/// A solid color fill layer's color.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Paint {
    /// sRGB, 0..1.
    Rgb([f32; 3]),
    /// Ink amounts, 0..1.
    Cmyk([f32; 4]),
}

/// A layer mask's place and its color outside it.
#[derive(Clone, Copy, Debug)]
struct MaskInfo {
    bounds: Bounds,
    default: u8,
    disabled: bool,
    /// The mask's values, and its default color, are read inverted.
    inverted: bool,
}

/// One layer record: a layer, or a group's opening or closing divider.
struct Record<'a> {
    name: String,
    bounds: Bounds,
    /// Channel id (−1 transparency, −2 the layer mask, 0.. the colors) and its data, compression
    /// method first.
    channels: Vec<(i16, &'a [u8])>,
    blend: Blend,
    /// The layer's opacity, 0..1.
    opacity: f32,
    /// Its fill opacity, 0..1.
    fill: f32,
    clipped: bool,
    hidden: bool,
    mask: Option<MaskInfo>,
    /// The section divider type: 0 a layer, 1 or 2 a group (open or closed in the panel), 3 the
    /// end of a group (the record below its layers).
    divider: u32,
    content: Content,
    effects: bool,
    vector_mask: bool,
    knockout: bool,
}

/// The layer records of `info` (the layer count, the records, then their channels' data).
fn records(info: &[u8], psb: bool) -> Result<Vec<Record<'_>>, String> {
    let mut r = Reader::new(info);
    let count = usize::from((r.u16()? as i16).unsigned_abs());
    if count.saturating_mul(MIN_RECORD) > info.len() {
        return Err(DAMAGED.into());
    }
    let mut heads = Vec::with_capacity(count);
    for _ in 0..count {
        heads.push(record(&mut r, psb)?);
    }
    let mut out = Vec::with_capacity(count);
    for (mut rec, lengths) in heads {
        rec.channels = lengths.into_iter().map(|(id, n)| r.take(n).map(|d| (id, d))).collect::<Result<_, _>>()?;
        out.push(rec);
    }
    Ok(out)
}

/// One record, with the lengths of its channels' data (which follows every record).
fn record<'a>(r: &mut Reader<'a>, psb: bool) -> Result<(Record<'a>, Vec<(i16, usize)>), String> {
    let bounds = Bounds::read(r)?;
    let n = usize::from(r.u16()?);
    if n > MAX_CHANNELS {
        return Err(DAMAGED.into());
    }
    let mut lengths = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.u16()? as i16;
        lengths.push((id, r.length(psb)?));
    }
    if &r.array::<4>()? != b"8BIM" {
        return Err(DAMAGED.into());
    }
    let blend = blend(&r.array()?);
    let opacity = f32::from(r.u8()?) / 255.0;
    let clipped = r.u8()? != 0;
    // Bit 1: hidden.
    let hidden = r.u8()? & 2 != 0;
    r.u8()?;
    let extra = r.section(false)?;
    let mut rec = Record {
        name: String::new(),
        bounds,
        channels: vec![],
        blend,
        opacity,
        fill: 1.0,
        clipped,
        hidden,
        mask: None,
        divider: 0,
        content: Content::Pixels,
        effects: false,
        vector_mask: false,
        knockout: false,
    };
    rec.read_extra(extra, psb)?;
    Ok((rec, lengths))
}

impl Record<'_> {
    /// The layer mask, blending ranges, name and tagged blocks after the record's fixed part. A
    /// damaged mask, blending ranges or name is an error; a damaged tagged block keeps what was
    /// read before it.
    fn read_extra(&mut self, extra: &[u8], psb: bool) -> Result<(), String> {
        let damaged = |_| DAMAGED.to_string();
        let mut e = Reader::new(extra);
        let mask = e.section(false).map_err(damaged)?;
        self.mask = match mask_info(mask) {
            Some(info) => Some(info),
            None if mask.is_empty() => None,
            None => return Err(DAMAGED.into()),
        };
        // The blending ranges.
        e.section(false).map_err(damaged)?;
        // A Pascal string padded to a multiple of 4 bytes, length byte included, in the system's
        // encoding (its Unicode name follows in `luni`), read as Mac Roman as psd-tools (MIT) reads
        // it: <https://github.com/psd-tools/psd-tools>, `psd/layer_and_mask.py`.
        let n = usize::from(e.u8().map_err(damaged)?);
        let name = e.take(n).map_err(damaged)?;
        self.name = name.iter().map(|&b| mac_roman(b)).collect();
        e.take((4 - (n + 1) % 4) % 4).map_err(damaged)?;
        // Effects in a descriptor (`lfx2`, `lmfx`) say whether they are switched on; the older
        // `lrFX` block is counted whenever it is there.
        let (mut effects, mut old_effects) = (None, false);
        while e.rest().len() >= 12 {
            let (Ok(signature), Ok(key)) = (e.array::<4>(), e.array::<4>()) else { break };
            if !matches!(&signature, b"8BIM" | b"8B64") {
                break;
            }
            let Ok(data) = e.section(psb && long_key(&key)) else { break };
            match &key {
                b"luni" => {
                    if let Some(name) = unicode(data) {
                        self.name = name;
                    }
                }
                b"lsct" | b"lsdk" => {
                    if let Some(t) = data.get(..4) {
                        self.divider = u32::from_be_bytes([t[0], t[1], t[2], t[3]]);
                    }
                    if data.get(4..8) == Some(b"8BIM")
                        && let Some(Ok(key)) = data.get(8..12).map(<[u8; 4]>::try_from)
                    {
                        self.blend = blend(&key);
                    }
                }
                b"iOpa" => self.fill = data.first().map_or(1.0, |&v| f32::from(v) / 255.0),
                b"TySh" | b"tySh" => self.content = Content::Type,
                b"SoCo" => self.content = Content::Solid(solid_color(data)),
                b"GdFl" | b"PtFl" => self.content = Content::Fill,
                b"lfx2" | b"lmfx" => effects = Some(effects == Some(true) || effects_on(data)),
                b"lrFX" => old_effects = true,
                b"vmsk" | b"vsms" => self.vector_mask = true,
                // Shallow or deep knockout.
                b"knko" => self.knockout = data.first().is_some_and(|&k| k != 0),
                k if ADJUSTMENTS.contains(&k) => self.content = Content::Adjustment,
                _ => {}
            }
            // Some writers pad blocks further: skip to the next signature.
            for _ in 0..3 {
                if e.rest().is_empty() || e.rest().starts_with(b"8BIM") || e.rest().starts_with(b"8B64") {
                    break;
                }
                let _ = e.take(1);
            }
        }
        self.effects = effects.unwrap_or(old_effects);
        Ok(())
    }

    /// Does the record store no pixels (bounds of no area, or no color channel)?
    fn stores_nothing(&self) -> bool {
        let (w, h) = self.bounds.size();
        w == 0 || h == 0 || !self.channels.iter().any(|(id, _)| *id >= 0)
    }

    /// Is the layer a fill drawn from its color over the whole canvas (a solid color fill that
    /// stores no pixels and has no vector mask)?
    fn color_over_canvas(&self) -> bool {
        matches!(self.content, Content::Solid(Some(_))) && self.stores_nothing() && !self.vector_mask
    }

    /// Does the layer cover the whole canvas without drawing (an adjustment layer, or a fill that
    /// stores no pixels, has no vector mask and isn't drawn from its color)? Layers clipped to it
    /// show over the whole canvas.
    fn covers_without_drawing(&self) -> bool {
        match self.content {
            Content::Adjustment => true,
            Content::Solid(None) | Content::Fill => self.stores_nothing() && !self.vector_mask,
            _ => false,
        }
    }

    /// Is the layer a fill that stores no pixels and whose shape is a vector mask, so that it
    /// isn't drawn?
    fn undrawn_shape(&self) -> bool {
        matches!(self.content, Content::Solid(_) | Content::Fill) && self.stores_nothing() && self.vector_mask
    }
}

/// Does an effects descriptor (`lfx2`, `lmfx`) have its master switch on and an effect switched on?
/// Read as psd-tools (MIT) reads them, a missing switch being off: `masterFXSwitch` and each
/// effect's `enab` (<https://github.com/psd-tools/psd-tools>, `api/effects.py`).
fn effects_on(data: &[u8]) -> bool {
    // A key, then its type `bool`, then one byte.
    let values = |key: &'static [u8]| data.windows(key.len() + 1).filter(move |w| w.starts_with(key)).filter_map(|w| w.last().map(|&v| v != 0));
    values(b"masterFXSwitchbool").any(|on| on) && values(b"enabbool").any(|on| on)
}

/// The layer mask data's place, default color, disabled flag (bit 1) and invert flag (bit 2).
fn mask_info(data: &[u8]) -> Option<MaskInfo> {
    let mut r = Reader::new(data);
    let bounds = Bounds::read(&mut r).ok()?;
    let default = r.u8().ok()?;
    let flags = r.u8().ok()?;
    Some(MaskInfo { bounds, default, disabled: flags & 2 != 0, inverted: flags & 4 != 0 })
}

/// A `luni` block's name: a count of UTF-16 units, then the units.
fn unicode(data: &[u8]) -> Option<String> {
    let mut r = Reader::new(data);
    let n = usize::try_from(r.u32().ok()?).ok()?;
    let units: Vec<u16> = r.take(n.checked_mul(2)?).ok()?.as_chunks::<2>().0.iter().map(|u| u16::from_be_bytes(*u)).collect();
    Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

/// The color of a solid color fill layer's descriptor, from the values after its color keys: red,
/// green and blue (0..255, or 0..1 in the `redFloat` keys), cyan, magenta, yellow and black (0..100
/// percent ink), gray (0..100 percent black), Lab (L 0..100, a and b) or hue (degrees), saturation
/// and brightness (0..100). `None` for a color in another model.
fn solid_color(data: &[u8]) -> Option<Paint> {
    // A key, then `doub` and a double, or `UntF`, a unit and a double.
    let value = |key: &[u8]| -> Option<f64> {
        let at = data.windows(key.len()).position(|w| w == key)? + key.len();
        let at = match data.get(at..at + 4)? {
            b"doub" => at + 4,
            b"UntF" => at + 8,
            _ => return None,
        };
        let v = f64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?);
        v.is_finite().then_some(v)
    };
    let unit = |v: f64| v.clamp(0.0, 1.0) as f32;
    let rgb = |keys: [&[u8]; 3], scale: f64| -> Option<[f32; 3]> {
        Some([unit(value(keys[0])? / scale), unit(value(keys[1])? / scale), unit(value(keys[2])? / scale)])
    };
    if let Some(c) = rgb([b"Rd  ", b"Grn ", b"Bl  "], 255.0).or_else(|| rgb([b"redFloat", b"greenFloat", b"blueFloat"], 1.0)) {
        return Some(Paint::Rgb(c));
    }
    if let (Some(c), Some(m), Some(y), Some(k)) = (value(b"Cyn "), value(b"Mgnt"), value(b"Ylw "), value(b"Blck")) {
        return Some(Paint::Cmyk([c, m, y, k].map(|v| unit(v / 100.0))));
    }
    if let Some(g) = value(b"Gry ") {
        return Some(Paint::Rgb([unit(1.0 - g / 100.0); 3]));
    }
    if let (Some(l), Some(a), Some(b)) = (value(b"Lmnc"), value(b"A   "), value(b"B   ")) {
        let [r, g, b] = lab_to_srgb(Lab { l: l as f32, a: a as f32, b: b as f32 });
        return Some(Paint::Rgb([r, g, b].map(|v| v.clamp(0.0, 1.0))));
    }
    let (h, s, b) = (value(b"H   ")?, value(b"Strt")?, value(b"Brgh")?);
    Some(Paint::Rgb(hsb(h, unit(s / 100.0), unit(b / 100.0))))
}

/// Hue `h` (degrees), saturation `s` and brightness `b` (0..1) as RGB.
fn hsb(h: f64, s: f32, b: f32) -> [f32; 3] {
    let h = (h.rem_euclid(360.0) / 60.0) as f32;
    let f = |n: f32| {
        let k = (n + h) % 6.0;
        b - b * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)]
}

/// The layer tree: records in the order the file stores them, bottom first.
enum Item {
    Layer(usize),
    /// A group's record and its layers.
    Group(usize, Vec<Item>),
}

fn tree(records: &[Record<'_>]) -> Result<Vec<Item>, String> {
    let mut stack: Vec<Vec<Item>> = vec![vec![]];
    for (i, rec) in records.iter().enumerate() {
        match rec.divider {
            3 => {
                if stack.len() > MAX_DEPTH {
                    return Err("the PSD file's layer groups are nested too deeply".into());
                }
                stack.push(vec![]);
            }
            1 | 2 => {
                // A group without its end divider holds nothing.
                let children = if stack.len() > 1 { stack.pop().unwrap_or_default() } else { vec![] };
                if let Some(top) = stack.last_mut() {
                    top.push(Item::Group(i, children));
                }
            }
            _ => {
                if let Some(top) = stack.last_mut() {
                    top.push(Item::Layer(i));
                }
            }
        }
    }
    // Groups the file never closes: their layers join the group around them.
    while stack.len() > 1 {
        let inner = stack.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.extend(inner);
        }
    }
    Ok(stack.pop().unwrap_or_default())
}

/// Straight RGBA, 0..1, at `left`, `top` on the canvas.
#[derive(Clone, Default)]
struct Canvas {
    left: i64,
    top: i64,
    width: usize,
    height: usize,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn from_image(left: i64, top: i64, img: &RgbaImage) -> Self {
        let px = img.pixels().map(|p| p.0.map(|v| f32::from(v) / 255.0)).collect();
        Self { left, top, width: img.width() as usize, height: img.height() as usize, px }
    }
    fn get(&self, x: i64, y: i64) -> Option<[f32; 4]> {
        let (x, y) = (usize::try_from(x - self.left).ok()?, usize::try_from(y - self.top).ok()?);
        if x >= self.width || y >= self.height {
            return None;
        }
        self.px.get(y * self.width + x).copied()
    }
    /// The canvas as an image, cropped to its painted pixels; `None` when nothing is painted.
    fn to_image(&self) -> Option<(i64, i64, RgbaImage)> {
        let painted = |x: usize, y: usize| self.px.get(y * self.width + x).is_some_and(|p| p[3] > 0.0);
        let rows: Vec<usize> = (0..self.height).filter(|&y| (0..self.width).any(|x| painted(x, y))).collect();
        let (&y0, &y1) = (rows.first()?, rows.last()?);
        let cols: Vec<usize> = (0..self.width).filter(|&x| (y0..=y1).any(|y| painted(x, y))).collect();
        let (&x0, &x1) = (cols.first()?, cols.last()?);
        let img = RgbaImage::from_fn((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32, |x, y| {
            let p = self.px.get((y0 + y as usize) * self.width + x0 + x as usize).copied().unwrap_or_default();
            image::Rgba(p.map(byte))
        });
        Some((self.left + x0 as i64, self.top + y0 as i64, img))
    }
}

/// The blend function of a mode VectorCraft doesn't have, on straight RGB.
fn blend_other(mode: Other, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    let each = |f: fn(f32, f32) -> f32| [0, 1, 2].map(|i| f(b[i].clamp(0.0, 1.0), s[i].clamp(0.0, 1.0)).clamp(0.0, 1.0));
    // Darker and Lighter Color compare the total of the channel values.
    let total = |c: [f32; 3]| c[0] + c[1] + c[2];
    let burn = |b: f32, s: f32| blend_rgb(BlendMode::ColorBurn, [b; 3], [s; 3])[0];
    let dodge = |b: f32, s: f32| blend_rgb(BlendMode::ColorDodge, [b; 3], [s; 3])[0];
    match mode {
        // Dissolve's pixels are opaque or transparent (`coverage`); the opaque ones replace.
        Other::Dissolve => s,
        Other::LinearBurn => each(|b, s| b + s - 1.0),
        Other::LinearDodge => each(|b, s| b + s),
        Other::VividLight => [0, 1, 2].map(|i| {
            let (b, s) = (b[i].clamp(0.0, 1.0), s[i].clamp(0.0, 1.0));
            if s <= 0.5 { burn(b, 2.0 * s) } else { dodge(b, 2.0 * s - 1.0) }
        }),
        Other::LinearLight => each(|b, s| b + 2.0 * s - 1.0),
        Other::PinLight => each(|b, s| if s <= 0.5 { b.min(2.0 * s) } else { b.max(2.0 * s - 1.0) }),
        Other::HardMix => each(|b, s| if b + s >= 1.0 { 1.0 } else { 0.0 }),
        Other::Subtract => each(|b, s| b - s),
        Other::Divide => each(|b, s| if s <= 0.0 { if b <= 0.0 { 0.0 } else { 1.0 } } else { b / s }),
        Other::DarkerColor => {
            if total(s) < total(b) {
                s
            } else {
                b
            }
        }
        Other::LighterColor => {
            if total(s) > total(b) {
                s
            } else {
                b
            }
        }
    }
}

/// Source-over compositing of straight `s` onto straight `b` in `mode`, as
/// [`vectorcraft_color::blend::composite`] does for the modes it has.
fn over(mode: Blend, b: [f32; 4], s: [f32; 4]) -> [f32; 4] {
    let other = match mode {
        Blend::Mode(m) => return composite(m, b, s),
        Blend::Pass => return composite(BlendMode::Normal, b, s),
        Blend::Other(o) => o,
    };
    let (ab, a) = (b[3].clamp(0.0, 1.0), s[3].clamp(0.0, 1.0));
    let ao = a + ab * (1.0 - a);
    if ao <= 0.0 {
        return [0.0; 4];
    }
    let mixed = blend_other(other, [b[0], b[1], b[2]], [s[0], s[1], s[2]]);
    let c = |i: usize| ((1.0 - ab) * s[i] + ab * mixed[i]) * a + (1.0 - a) * ab * b[i];
    [c(0) / ao, c(1) / ao, c(2) / ao, ao]
}

/// What one entry of a layer list becomes: a layer of the document.
struct Unit<'i> {
    name: String,
    visible: bool,
    opacity: f32,
    blend: Blend,
    /// The record whose layer mask the layer takes.
    mask_of: Option<usize>,
    /// The record the layer comes from (each record has its own Dissolve pattern).
    record: usize,
    body: Body<'i>,
    /// While layers are clipped to it: how its merged canvas is shaped.
    clip: Option<Clip>,
}

impl Unit<'_> {
    /// Is it what a merge of layers in modes VectorCraft lacks made: one image, visible, at full
    /// opacity in the Normal mode, without a mask?
    fn is_merge(&self) -> bool {
        matches!(self.body, Body::Merged(_))
            && self.clip.is_none()
            && self.mask_of.is_none()
            && self.visible
            && self.opacity >= 1.0
            && self.blend == Blend::Mode(BlendMode::Normal)
    }
}

/// The shape of a layer merged with the layers clipped to it.
enum Clip {
    /// The layer's own transparency at each pixel of its canvas; the canvas holds its pixels at its
    /// fill opacity, the clipped layers composited over them.
    Shape(Vec<f32>),
    /// The whole canvas of the file (a layer that covers it without drawing): the canvas holds the
    /// clipped layers alone.
    Whole,
}

enum Body<'i> {
    /// A layer record's pixels.
    Layer(usize),
    /// A group's layers.
    Group(&'i [Item]),
    /// Layers merged into one image.
    Merged(Canvas),
}

/// A unit's body drawn alone.
enum Drawn {
    /// It draws nothing.
    Nothing,
    Canvas(Canvas),
    /// Its canvas would need more memory than allowed.
    TooLarge,
}

/// What the warnings say.
#[derive(Default)]
struct Notes {
    types: usize,
    adjustments: usize,
    fills: usize,
    effects: usize,
    vector_masks: usize,
    knockouts: usize,
    /// Hidden layers left out, and layers clipped to them.
    hidden: usize,
    merged: Vec<&'static str>,
    unmerged: usize,
    /// Clipped layers left unclipped: the merge needs more memory than allowed.
    unclipped: usize,
    /// Layers clipped to a fill that isn't drawn.
    clipped_out: usize,
}

/// The counts a merge that can't be made gives back.
#[derive(Clone, Copy)]
struct Counts {
    skipped: usize,
    merged: usize,
    unmerged: usize,
    unclipped: usize,
    clipped_out: usize,
}

/// `n` things: "1 layer was", "2 layers were".
fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one} was") } else { format!("{n} {many} were") }
}

impl Notes {
    fn warnings(self) -> Vec<String> {
        let mut out = vec![];
        if self.types > 0 {
            let n = if self.types == 1 { "1 type layer".to_string() } else { format!("{} type layers", self.types) };
            out.push(format!("{n} came in as pixels: the text isn't editable"));
        }
        if self.adjustments > 0 {
            out.push(format!("{} not applied", count(self.adjustments, "adjustment layer", "adjustment layers")));
        }
        if self.fills > 0 {
            out.push(format!(
                "{} not drawn (a gradient or pattern fill, a color that can't be read, or a shape that stores no pixels)",
                count(self.fills, "fill layer", "fill layers")
            ));
        }
        let layers = |n: usize| if n == 1 { "1 layer".to_string() } else { format!("{n} layers") };
        if self.effects > 0 {
            out.push(format!("the layer effects of {} weren't drawn", layers(self.effects)));
        }
        if self.vector_masks > 0 {
            out.push(format!("the vector masks of {} weren't applied", layers(self.vector_masks)));
        }
        if self.knockouts > 0 {
            out.push(format!("the knockout of {} wasn't applied", layers(self.knockouts)));
        }
        if self.hidden > 0 {
            out.push(format!("{} left out (hidden layers and the layers clipped to them)", count(self.hidden, "layer", "layers")));
        }
        if self.clipped_out == 1 {
            out.push("1 layer clipped to a fill layer that wasn't drawn was left out".into());
        } else if self.clipped_out > 1 {
            out.push(format!("{} layers clipped to fill layers that weren't drawn were left out", self.clipped_out));
        }
        if !self.merged.is_empty() {
            out.push(format!("layers in blend modes VectorCraft doesn't have ({}) were merged with the layers below them", self.merged.join(", ")));
        }
        if self.unmerged > 0 {
            out.push(format!(
                "{} in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed",
                layers(self.unmerged)
            ));
        }
        if self.unclipped > 0 {
            out.push(format!("{} clipped to the layer below came in unclipped: the merge needs more memory than allowed", layers(self.unclipped)));
        }
        out
    }

    fn counts(&self, skipped: usize) -> Counts {
        Counts { skipped, merged: self.merged.len(), unmerged: self.unmerged, unclipped: self.unclipped, clipped_out: self.clipped_out }
    }
}

struct Builder<'a, F> {
    h: &'a Header<'a>,
    records: &'a [Record<'a>],
    /// Hidden layers come in, hidden.
    hidden: bool,
    max_alloc: u64,
    /// Bytes held by the copies of what is below the pass-through groups being merged.
    held: u64,
    cmyk: F,
    doc: Document,
    /// Canvas pixels to points.
    to_doc: Affine,
    ppi: Option<(f64, f64)>,
    layers: usize,
    notes: Notes,
    /// The records [`Notes`] counted.
    noted: Vec<bool>,
    /// The records counted as hidden layers left out.
    skipped: Vec<bool>,
    /// Those records, in the order they were counted.
    skip_log: Vec<usize>,
    /// Bytes of layer and mask pixels decoded, and of canvases the merges made, so far.
    spent: Cell<u64>,
}

impl<F: Fn([f32; 4]) -> [f32; 3]> Builder<'_, F> {
    fn record(&self, i: usize) -> Result<&Record<'_>, String> {
        self.records.get(i).ok_or_else(|| DAMAGED.to_string())
    }

    /// Refuse to allocate `bytes` past the memory allowed.
    fn fits(&self, bytes: u64) -> Result<(), String> {
        if bytes > self.max_alloc { Err("a layer of the PSD file is too large to read".into()) } else { Ok(()) }
    }

    /// Count `bytes` of decoded pixels, or of a canvas a merge makes, against the whole import's
    /// allowance, [`TOTAL_DECODED`] times the memory allowed for one image.
    fn spend(&self, bytes: u64) -> Result<(), String> {
        let total = self.spent.get().saturating_add(bytes);
        self.spent.set(total);
        if total > self.max_alloc.saturating_mul(TOTAL_DECODED) {
            return Err("the PSD file's layers take more memory to read than allowed".into());
        }
        Ok(())
    }

    /// Record `i`'s pixels as straight RGBA, at their place on the canvas; `None` when it has none.
    fn image(&self, i: usize) -> Result<Option<(i64, i64, RgbaImage)>, String> {
        let rec = self.record(i)?;
        let h = self.h;
        let max_side = if h.psb { 300_000 } else { 30_000 };
        let (w, rows) = match rec.content {
            Content::Adjustment => return Ok(None),
            // A fill layer that stores no pixels (a shape layer stores its shape's; one that
            // doesn't, with only its vector mask, isn't drawn).
            Content::Solid(Some(paint)) if rec.color_over_canvas() => {
                let (w, rows) = (h.width as usize, h.height as usize);
                self.fits(4 * w as u64 * rows as u64)?;
                self.spend(4 * w as u64 * rows as u64)?;
                let rgb = match paint {
                    Paint::Rgb(rgb) => rgb,
                    Paint::Cmyk(ink) => (self.cmyk)(ink),
                };
                let px = image::Rgba([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 255]);
                return Ok(Some((0, 0, RgbaImage::from_pixel(h.width, h.height, px))));
            }
            Content::Pixels | Content::Type | Content::Solid(_) | Content::Fill => rec.bounds.size(),
        };
        if rec.stores_nothing() {
            return Ok(None);
        }
        if w > max_side || rows > max_side {
            return Err(DAMAGED.into());
        }
        let colors = h.mode.colors();
        let row = row_bytes(w, h.depth);
        self.fits(((colors as u64 + 1) * row as u64).saturating_add(4 * w as u64).saturating_mul(rows as u64))?;
        self.spend(4 * w as u64 * rows as u64)?;
        let plane = |id: i16| -> Result<Option<Vec<u8>>, String> {
            match rec.channels.iter().find(|(c, _)| *c == id) {
                Some((_, data)) => layer_channel(data, rows, row, h.psb, h.depth).map(Some),
                None => Ok(None),
            }
        };
        let planes = (0..colors as i16).map(plane).collect::<Result<Vec<_>, _>>()?;
        let alpha = plane(-1)?;
        let linear = h.depth == 32;
        let img = RgbaImage::from_fn(w as u32, rows as u32, |x, y| {
            let (x, y) = (x as usize, y as usize);
            let at = |p: &Option<Vec<u8>>, linear: bool, missing: f32| {
                p.as_ref().and_then(|p| p.get(y * row..(y + 1) * row)).map_or(missing, |r| sample(r, x, h.depth, linear))
            };
            let rgb = to_rgb(h, |c| planes.get(c).map_or(0.0, |p| at(p, linear, 0.0)), &self.cmyk);
            image::Rgba([byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(at(&alpha, false, 1.0))])
        });
        Ok(Some((i64::from(rec.bounds.left), i64::from(rec.bounds.top), img)))
    }

    /// Record `i`'s layer mask as gray values at their place, with the value outside them; `None`
    /// when it has none, or it is disabled and not `even_disabled`.
    fn mask(&self, i: usize, even_disabled: bool) -> Result<Option<(i64, i64, image::GrayImage, u8)>, String> {
        let rec = self.record(i)?;
        let Some(m) = rec.mask.filter(|m| even_disabled || !m.disabled) else { return Ok(None) };
        let Some((_, data)) = rec.channels.iter().find(|(c, _)| *c == -2) else { return Ok(None) };
        let (w, rows) = m.bounds.size();
        let max_side = if self.h.psb { 300_000 } else { 30_000 };
        if w > max_side || rows > max_side {
            return Err(DAMAGED.into());
        }
        let row = row_bytes(w, self.h.depth);
        self.fits((row as u64 + w as u64).saturating_mul(rows as u64))?;
        self.spend(w as u64 * rows as u64)?;
        let plane = layer_channel(data, rows, row, self.h.psb, self.h.depth)?;
        let read = |v: u8| if m.inverted { 255 - v } else { v };
        let img = image::GrayImage::from_fn(w as u32, rows as u32, |x, y| {
            let r = plane.get(y as usize * row..(y as usize + 1) * row).unwrap_or_default();
            image::Luma([read(byte(sample(r, x as usize, self.h.depth, false)))])
        });
        Ok(Some((i64::from(m.bounds.left), i64::from(m.bounds.top), img, read(m.default))))
    }

    /// Multiply `c`'s transparency by record `i`'s layer mask.
    fn apply_mask(&self, c: &mut Canvas, i: usize) -> Result<(), String> {
        let Some((left, top, m, default)) = self.mask(i, false)? else { return Ok(()) };
        let (w, h) = (i64::from(m.width()), i64::from(m.height()));
        for (k, p) in c.px.iter_mut().enumerate() {
            let (x, y) = (c.left + (k % c.width.max(1)) as i64, c.top + (k / c.width.max(1)) as i64);
            let inside = (left..left + w).contains(&x) && (top..top + h).contains(&y);
            let v = if inside { m.get_pixel((x - left) as u32, (y - top) as u32).0[0] } else { default };
            p[3] *= f32::from(v) / 255.0;
        }
        Ok(())
    }

    /// What `items` (a layer list, bottom first) become, before the modes VectorCraft lacks are
    /// merged: layers clipped to the one below are merged into it, and hidden ones are left out
    /// unless `keep_hidden`. Hidden layers a clip would merge are always left out.
    fn units<'i>(&mut self, items: &'i [Item], keep_hidden: bool) -> Result<Vec<Unit<'i>>, String> {
        let mut out: Vec<Unit<'i>> = vec![];
        // The layer the next clipped layers clip to: its index in `out`, or `None` when it was left
        // out (its clipped layers go with it).
        let mut base: Option<Option<usize>> = None;
        // The next clipped layers come in unclipped: a merge with their base didn't fit.
        let mut unclipped = false;
        for item in items {
            let (i, body) = match item {
                Item::Layer(i) => (*i, Body::Layer(*i)),
                Item::Group(i, children) => (*i, Body::Group(children)),
            };
            let rec = self.record(i)?;
            let (name, hidden, clipped, blend) = (rec.name.clone(), rec.hidden, rec.clipped, rec.blend);
            let opacity = rec.opacity * rec.fill;
            if clipped && let Some(b) = base {
                match b {
                    // Clipped to a layer left out, or hidden itself: left out.
                    None => self.skip(i),
                    Some(_) if hidden => self.skip(i),
                    // A hidden base (with `keep_hidden`) would need the merge: it is left out with
                    // the layers clipped to it.
                    Some(b) if out.get(b).is_some_and(|u| !u.visible) => {
                        if b < out.len() {
                            let u = out.remove(b);
                            self.skip(u.record);
                        }
                        self.skip(i);
                        base = Some(None);
                    }
                    Some(b) => {
                        // Counted even when it draws nothing (an adjustment layer).
                        self.note(i)?;
                        if !unclipped && self.clip(&mut out, b, &body, i, &name, blend, opacity)? {
                            continue;
                        }
                        // The merge doesn't fit: this layer and the next ones clipped to the same
                        // base come in as layers of their own.
                        unclipped = true;
                        self.notes.unclipped += 1;
                        out.push(Unit { name, visible: true, opacity, blend, mask_of: Some(i), record: i, body, clip: None });
                    }
                }
                continue;
            }
            unclipped = false;
            if hidden && !keep_hidden {
                self.skip(i);
                base = Some(None);
                continue;
            }
            self.note(i)?;
            out.push(Unit { name, visible: !hidden, opacity, blend, mask_of: Some(i), record: i, body, clip: None });
            base = Some(Some(out.len() - 1));
        }
        for u in &mut out {
            self.finish_clip(u);
        }
        Ok(out)
    }

    /// Merge the layer of record `i` (`body`, named `name`, drawn alone with its layer mask, at
    /// `opacity` in `blend`) into unit `b` of `out`, which it is clipped to. The base's own layer
    /// mask stays its opacity mask, which hides the clipped layers with it. `false`, with `out` as
    /// it was, when the canvases would need more memory than allowed.
    #[allow(clippy::too_many_arguments)]
    fn clip(&mut self, out: &mut [Unit<'_>], b: usize, body: &Body<'_>, i: usize, name: &str, blend: Blend, opacity: f32) -> Result<bool, String> {
        let Some(u) = out.get(b) else { return Ok(true) };
        let (whole, undrawn, fill, base_opacity) = {
            let r = self.record(u.record)?;
            let layer = matches!(u.body, Body::Layer(_));
            (layer && r.covers_without_drawing(), layer && r.undrawn_shape(), r.fill, r.opacity)
        };
        // A fill whose shape is a vector mask isn't drawn, and nothing clipped to it is either.
        if undrawn && u.clip.is_none() {
            self.notes.clipped_out += 1;
            return Ok(true);
        }
        let layer = match self.draw_body(body, Some(i))? {
            Drawn::Nothing => return Ok(true),
            Drawn::TooLarge => return Ok(false),
            Drawn::Canvas(c) => c,
        };
        let room = self.room();
        let layer_bytes = 16 * layer.px.len() as u64;
        let Some(u) = out.get_mut(b) else { return Ok(true) };
        if u.clip.is_none() {
            let (canvas, clip) = if whole {
                (Canvas::default(), Clip::Whole)
            } else {
                match self.draw_body(&u.body, None)? {
                    Drawn::TooLarge => return Ok(false),
                    // A base that draws nothing shows nothing of its clipped layers either.
                    Drawn::Nothing => (Canvas::default(), Clip::Shape(vec![])),
                    Drawn::Canvas(mut c) => {
                        // Its transparency, 4 bytes a pixel, is kept beside the canvas.
                        if (20 * c.px.len() as u64).saturating_add(layer_bytes) > room {
                            return Ok(false);
                        }
                        self.spend(4 * c.px.len() as u64)?;
                        let shape = c.px.iter().map(|p| p[3]).collect();
                        for p in &mut c.px {
                            p[3] = fill;
                        }
                        (c, Clip::Shape(shape))
                    }
                }
            };
            u.body = Body::Merged(canvas);
            u.clip = Some(clip);
            u.opacity = base_opacity;
        }
        let Body::Merged(c) = &mut u.body else { return Ok(true) };
        match &u.clip {
            Some(Clip::Shape(_)) => {
                if (20 * c.px.len() as u64).saturating_add(layer_bytes) > room {
                    return Ok(false);
                }
                clip_onto(c, &layer, blend, opacity, i);
            }
            _ => {
                let mut acc = (!c.px.is_empty()).then(|| std::mem::take(c));
                let laid = blend_onto(&mut acc, &layer, blend, opacity, i, room, &|n| self.spend(n));
                *c = acc.unwrap_or_default();
                if !laid? {
                    return Ok(false);
                }
            }
        }
        u.name.push_str(" + ");
        u.name.push_str(name);
        Ok(true)
    }

    /// Unit `u`'s merged canvas, once every layer clipped to it is in: transparent outside its
    /// shape.
    fn finish_clip(&self, u: &mut Unit<'_>) {
        let Some(clip) = u.clip.take() else { return };
        let Body::Merged(c) = &mut u.body else { return };
        match clip {
            Clip::Shape(shape) => c.px.iter_mut().zip(shape).for_each(|(p, a)| p[3] *= a),
            Clip::Whole => {
                let (w, h, width) = (i64::from(self.h.width), i64::from(self.h.height), c.width.max(1));
                for (k, p) in c.px.iter_mut().enumerate() {
                    let (x, y) = (c.left + (k % width) as i64, c.top + (k / width) as i64);
                    if !(0..w).contains(&x) || !(0..h).contains(&y) {
                        p[3] = 0.0;
                    }
                }
            }
        }
    }

    /// Count what record `i` holds that doesn't come in.
    fn note(&mut self, i: usize) -> Result<(), String> {
        // Once, though a merge may draw it again.
        match self.noted.get_mut(i) {
            Some(true) => return Ok(()),
            Some(n) => *n = true,
            None => {}
        }
        let (content, empty, effects, vector_mask, knockout) = {
            let rec = self.record(i)?;
            (rec.content, rec.stores_nothing(), rec.effects, rec.vector_mask, rec.knockout)
        };
        match content {
            Content::Type => self.notes.types += 1,
            Content::Adjustment => self.notes.adjustments += 1,
            // A fill drawn from neither its pixels nor its color.
            Content::Fill | Content::Solid(None) if empty => self.notes.fills += 1,
            Content::Solid(Some(_)) if empty && vector_mask => self.notes.fills += 1,
            _ => {}
        }
        self.notes.effects += usize::from(effects);
        self.notes.vector_masks += usize::from(vector_mask);
        self.notes.knockouts += usize::from(knockout);
        Ok(())
    }

    /// Count record `i` as a hidden layer left out (once).
    fn skip(&mut self, i: usize) {
        if let Some(s) = self.skipped.get_mut(i)
            && !*s
        {
            *s = true;
            self.notes.hidden += 1;
            self.skip_log.push(i);
        }
    }

    /// `items`' units with the layers in blend modes VectorCraft lacks merged with those below.
    fn merged<'i>(&mut self, items: &'i [Item], keep_hidden: bool) -> Result<Vec<Unit<'i>>, String> {
        let mut out: Vec<Unit<'i>> = vec![];
        for mut u in self.units(items, keep_hidden)? {
            let Blend::Other(other) = u.blend else {
                out.push(u);
                continue;
            };
            // A hidden layer that would need the merge is left out.
            if !u.visible {
                self.skip(u.record);
                continue;
            }
            // A merge that can't be made counts nothing its groups would have counted.
            let saved = self.notes.counts(self.skip_log.len());
            let made = match out.as_mut_slice() {
                // Onto the previous merge alone: composited onto its canvas, in place.
                [prev] if prev.is_merge() => {
                    let mut acc = match &mut prev.body {
                        Body::Merged(c) => Some(std::mem::take(c)),
                        _ => None,
                    };
                    let made = self.composite(&mut acc, &u);
                    if let Body::Merged(c) = &mut prev.body {
                        *c = acc.unwrap_or_default();
                    }
                    if made? {
                        prev.name.push_str(" + ");
                        prev.name.push_str(&u.name);
                        true
                    } else {
                        false
                    }
                }
                _ => match self.merge(&out, &u)? {
                    Some(canvas) => {
                        let mut name = String::new();
                        for (k, b) in out.iter_mut().filter(|b| b.visible).enumerate() {
                            if k == 0 {
                                name = std::mem::take(&mut b.name);
                            } else {
                                name.push_str(" + ");
                                name.push_str(&b.name);
                            }
                        }
                        if out.iter().any(|b| b.visible) {
                            name.push_str(" + ");
                        }
                        name.push_str(&u.name);
                        // Hidden layers below are left out with the rest.
                        for b in out.iter().filter(|b| !b.visible).map(|b| b.record).collect::<Vec<_>>() {
                            self.skip(b);
                        }
                        out.clear();
                        out.push(Unit {
                            name,
                            visible: true,
                            opacity: 1.0,
                            blend: Blend::Mode(BlendMode::Normal),
                            mask_of: None,
                            record: u.record,
                            body: Body::Merged(canvas),
                            clip: None,
                        });
                        true
                    }
                    None => false,
                },
            };
            if made {
                if !self.notes.merged.contains(&other.label()) {
                    self.notes.merged.push(other.label());
                }
            } else {
                self.restore(saved);
                self.notes.unmerged += 1;
                u.blend = Blend::Mode(BlendMode::Normal);
                out.push(u);
            }
        }
        Ok(out)
    }

    /// The counts as they were before a merge that couldn't be made.
    fn restore(&mut self, c: Counts) {
        for i in self.skip_log.split_off(c.skipped) {
            if let Some(s) = self.skipped.get_mut(i) {
                *s = false;
                self.notes.hidden = self.notes.hidden.saturating_sub(1);
            }
        }
        self.notes.merged.truncate(c.merged);
        self.notes.unmerged = c.unmerged;
        self.notes.unclipped = c.unclipped;
        self.notes.clipped_out = c.clipped_out;
    }

    /// The visible units `below` with `top` composited over them; `None` when the canvas would need
    /// more memory than allowed.
    fn merge(&mut self, below: &[Unit<'_>], top: &Unit<'_>) -> Result<Option<Canvas>, String> {
        let mut acc: Option<Canvas> = None;
        for u in below.iter().filter(|u| u.visible).chain([top]) {
            if !self.composite(&mut acc, u)? {
                return Ok(None);
            }
        }
        Ok(Some(acc.unwrap_or_default()))
    }

    /// The bytes a merge step may still take: the memory allowed, less the copies of what is below
    /// the pass-through groups being merged.
    fn room(&self) -> u64 {
        self.max_alloc.saturating_sub(self.held)
    }

    /// Unit `u` composited onto `acc`. A pass-through group lays its layers onto `acc` one by one,
    /// so that they blend with what is below it; at less than full opacity or with a layer mask, the
    /// result is then faded back toward `acc` as it was by the opacity and the mask. Anything else
    /// is drawn alone first. `false` when the canvas would need more memory than allowed.
    fn composite(&mut self, acc: &mut Option<Canvas>, u: &Unit<'_>) -> Result<bool, String> {
        if let (Body::Group(items), Blend::Pass) = (&u.body, u.blend) {
            let mask = match u.mask_of {
                Some(i) if self.has_mask(Some(i))? => self.mask(i, false)?,
                _ => None,
            };
            if u.opacity >= 1.0 && mask.is_none() {
                return self.lay(acc, items);
            }
            let bytes = acc.as_ref().map_or(0, |c| 16 * c.px.len() as u64);
            if bytes > self.room() {
                return Ok(false);
            }
            self.spend(bytes)?;
            let below = acc.clone();
            self.held = self.held.saturating_add(bytes);
            let laid = self.lay(acc, items);
            self.held = self.held.saturating_sub(bytes);
            if !laid? {
                return Ok(false);
            }
            if let Some(after) = acc.as_mut() {
                fade(after, below.as_ref(), u.opacity, mask.as_ref());
            }
            return Ok(true);
        }
        let c = match self.draw_body(&u.body, u.mask_of)? {
            Drawn::Nothing => return Ok(true),
            Drawn::Canvas(c) => c,
            Drawn::TooLarge => return Ok(false),
        };
        blend_onto(acc, &c, u.blend, u.opacity, u.record, self.room(), &|n| self.spend(n))
    }

    /// `items`' layers composited onto `acc` one by one; `false` when a canvas would need more
    /// memory than allowed.
    fn lay(&mut self, acc: &mut Option<Canvas>, items: &[Item]) -> Result<bool, String> {
        for c in self.merged(items, false)? {
            if !self.composite(acc, &c)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Does record `mask_of` have a layer mask that is switched on?
    fn has_mask(&self, mask_of: Option<usize>) -> Result<bool, String> {
        let Some(i) = mask_of else { return Ok(false) };
        let rec = self.record(i)?;
        Ok(rec.mask.is_some_and(|m| !m.disabled) && rec.channels.iter().any(|(c, _)| *c == -2))
    }

    /// A unit's body drawn alone, with the layer mask of record `mask_of`, before its own opacity
    /// and blend mode. Its canvas counts against the import's allowance.
    fn draw_body(&mut self, body: &Body<'_>, mask_of: Option<usize>) -> Result<Drawn, String> {
        let mut c = match body {
            Body::Merged(c) => {
                let bytes = 16 * c.px.len() as u64;
                if bytes > self.room() {
                    return Ok(Drawn::TooLarge);
                }
                self.spend(bytes)?;
                c.clone()
            }
            Body::Layer(i) => match self.image(*i)? {
                Some((left, top, img)) => {
                    let bytes = 16 * u64::from(img.width()) * u64::from(img.height());
                    if bytes > self.room() {
                        return Ok(Drawn::TooLarge);
                    }
                    self.spend(bytes)?;
                    Canvas::from_image(left, top, &img)
                }
                None => return Ok(Drawn::Nothing),
            },
            Body::Group(items) => {
                let mut acc: Option<Canvas> = None;
                if !self.lay(&mut acc, items)? {
                    return Ok(Drawn::TooLarge);
                }
                let Some(acc) = acc else { return Ok(Drawn::Nothing) };
                acc
            }
        };
        if let Some(i) = mask_of {
            self.apply_mask(&mut c, i)?;
        }
        Ok(Drawn::Canvas(c))
    }

    /// A new layer of the document named `name`, colored as the next new layer would be.
    fn layer(&mut self, name: &str) -> Node {
        let n = Node::layer(self.doc.alloc_id(), name, LayerColor::Preset((self.layers % LAYER_COLORS.len()) as u8));
        self.layers += 1;
        n
    }

    /// `img` at `left`, `top` on the canvas as an image of the document named `name`.
    fn image_node(&mut self, name: &str, left: i64, top: i64, img: RgbaImage) -> Result<Node, String> {
        let RasterImage { key, blob, width, height, .. } = as_png(img, None, self.ppi).map_err(|e| e.to_string())?;
        let xf = self.to_doc * Affine::translate((left as f64, top as f64));
        let mut n = Node::new(
            self.doc.alloc_id(),
            NodeKind::Image(Box::new(ImageObject { key: key.clone(), width, height, xf, link: None, placement: Default::default(), pixels: None })),
        );
        n.name = Some(name.to_string());
        self.doc.images.insert(key, blob);
        Ok(n)
    }

    /// Layers of the document for `items`, bottom first.
    fn list(&mut self, items: &[Item]) -> Result<Vec<Arc<Node>>, String> {
        let mut out = vec![];
        for u in self.merged(items, self.hidden)? {
            if let Some(n) = self.node(u)? {
                out.push(Arc::new(n));
            }
        }
        Ok(out)
    }

    /// The layer of the document unit `u` becomes; `None` for a layer that draws nothing (an
    /// adjustment layer, or one without pixels).
    fn node(&mut self, u: Unit<'_>) -> Result<Option<Node>, String> {
        let children = match &u.body {
            Body::Layer(i) => match self.image(*i)? {
                Some((left, top, img)) => vec![Arc::new(self.image_node(&u.name, left, top, img)?)],
                None => return Ok(None),
            },
            Body::Merged(c) => match c.to_image() {
                Some((left, top, img)) => vec![Arc::new(self.image_node(&u.name, left, top, img)?)],
                None => return Ok(None),
            },
            Body::Group(items) => self.list(items)?,
        };
        let mut layer = self.layer(&u.name);
        layer.visible = u.visible;
        layer.opacity = u.opacity.clamp(0.0, 1.0);
        (layer.blend, layer.isolate) = match u.blend {
            Blend::Mode(m) => (m, matches!(u.body, Body::Group(_))),
            Blend::Pass | Blend::Other(_) => (BlendMode::Normal, false),
        };
        if let Some(c) = layer.children_mut() {
            *c = children;
        }
        if let Some(i) = u.mask_of {
            layer.extras_mut().mask = self.opacity_mask(i)?.map(Box::new);
        }
        Ok(Some(layer))
    }

    /// Record `i`'s layer mask as an opacity mask: a gray image, outside which the layer is hidden
    /// or shown as the mask's default color says (a mask of no pixels hides all of it, or none).
    fn opacity_mask(&mut self, i: usize) -> Result<Option<OpacityMask>, String> {
        let disabled = self.record(i)?.mask.is_some_and(|m| m.disabled);
        let Some((left, top, gray, default)) = self.mask(i, true)? else { return Ok(None) };
        let (art, clip) = if gray.width() == 0 || gray.height() == 0 {
            if default == 255 {
                return Ok(None);
            }
            let black = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
            (self.image_node("Layer Mask", left, top, black)?, true)
        } else {
            self.fits(5 * u64::from(gray.width()) * u64::from(gray.height()))?;
            let rgba = RgbaImage::from_fn(gray.width(), gray.height(), |x, y| {
                let v = gray.get_pixel(x, y).0[0];
                image::Rgba([v, v, v, 255])
            });
            (self.image_node("Layer Mask", left, top, rgba)?, default == 0)
        };
        let mut mask = OpacityMask::new(art, clip);
        mask.disabled = disabled;
        Ok(Some(mask))
    }
}

/// The coverage of a pixel at `x`, `y` of the canvas with `alpha`, 0..1, in `blend`: in Dissolve
/// fully opaque or fully transparent, opaque at a share of the pixels equal to `alpha`, chosen by a
/// fixed hash of the pixel's place and of `record`, so that each layer has its own pattern; `alpha`
/// in every other mode.
fn coverage(blend: Blend, alpha: f32, x: i64, y: i64, record: usize) -> f32 {
    if !matches!(blend, Blend::Other(Other::Dissolve)) {
        return alpha;
    }
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (record as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    // The top 24 bits, 0..1.
    let r = (h >> 40) as f32 / (1u64 << 24) as f32;
    if r < alpha { 1.0 } else { 0.0 }
}

/// `layer` (of record `record`) composited onto `base`, the canvas of the layer it is clipped to,
/// in `blend` at `opacity`, where the two overlap.
fn clip_onto(base: &mut Canvas, layer: &Canvas, blend: Blend, opacity: f32, record: usize) {
    let (x0, x1) = (base.left.max(layer.left), (base.left + base.width as i64).min(layer.left + layer.width as i64));
    let (y0, y1) = (base.top.max(layer.top), (base.top + base.height as i64).min(layer.top + layer.height as i64));
    for y in y0..y1 {
        for x in x0..x1 {
            let Some(mut s) = layer.get(x, y) else { continue };
            let k = (y - base.top) as usize * base.width + (x - base.left) as usize;
            let Some(b) = base.px.get_mut(k) else { continue };
            s[3] = coverage(blend, s[3] * opacity, x, y, record);
            *b = over(blend, *b, s);
        }
    }
}

/// `after` faded back toward `before` (transparent where it has no pixels) by `opacity` times the
/// value of `mask` (its place, its gray values and the value outside it) at each pixel, in
/// premultiplied color.
fn fade(after: &mut Canvas, before: Option<&Canvas>, opacity: f32, mask: Option<&(i64, i64, image::GrayImage, u8)>) {
    let width = after.width.max(1);
    for (k, p) in after.px.iter_mut().enumerate() {
        let (x, y) = (after.left + (k % width) as i64, after.top + (k / width) as i64);
        let m = mask.map_or(255, |(left, top, m, default)| {
            let inside = (*left..*left + i64::from(m.width())).contains(&x) && (*top..*top + i64::from(m.height())).contains(&y);
            if inside { m.get_pixel((x - left) as u32, (y - top) as u32).0[0] } else { *default }
        });
        let t = opacity * f32::from(m) / 255.0;
        let b = before.and_then(|c| c.get(x, y)).unwrap_or([0.0; 4]);
        let alpha = (1.0 - t) * b[3] + t * p[3];
        if alpha <= 0.0 {
            *p = [0.0; 4];
            continue;
        }
        let mix = |i: usize| ((1.0 - t) * b[3] * b[i] + t * p[3] * p[i]) / alpha;
        *p = [mix(0), mix(1), mix(2), alpha];
    }
}

/// `src` (of record `record`) at `opacity` composited onto `acc` (nothing when `None`) in `blend`,
/// on a canvas covering both (`acc` itself when it covers `src`; a larger one is counted with
/// `spend`). `false`, with `acc` as it was, when the canvases held, `acc` and `src` and a larger
/// canvas when one is needed, would take more than `max_alloc` bytes.
fn blend_onto(
    acc: &mut Option<Canvas>,
    src: &Canvas,
    blend: Blend,
    opacity: f32,
    record: usize,
    max_alloc: u64,
    spend: &dyn Fn(u64) -> Result<(), String>,
) -> Result<bool, String> {
    let (mut left, mut top, mut right, mut bottom) = (src.left, src.top, src.left + src.width as i64, src.top + src.height as i64);
    if let Some(a) = acc.as_ref() {
        left = left.min(a.left);
        top = top.min(a.top);
        right = right.max(a.left + a.width as i64);
        bottom = bottom.max(a.top + a.height as i64);
    }
    let (Ok(width), Ok(height)) = (usize::try_from(right - left), usize::try_from(bottom - top)) else { return Ok(false) };
    let held = (src.px.len() as u64).saturating_add(acc.as_ref().map_or(0, |a| a.px.len() as u64));
    let covers = acc.as_ref().is_some_and(|a| (a.left, a.top, a.width, a.height) == (left, top, width, height));
    if covers {
        if held.saturating_mul(16) > max_alloc {
            return Ok(false);
        }
    } else {
        let bytes = (width as u64).saturating_mul(height as u64).saturating_mul(16);
        if bytes.saturating_add(held.saturating_mul(16)) > max_alloc {
            return Ok(false);
        }
        spend(bytes)?;
        let mut out = Canvas { left, top, width, height, px: vec![[0.0; 4]; width * height] };
        if let Some(a) = acc.take() {
            for (y, row) in a.px.chunks(a.width.max(1)).enumerate() {
                let at = (a.top - top) as usize + y;
                let start = at * width + (a.left - left) as usize;
                if let Some(dst) = out.px.get_mut(start..start + row.len()) {
                    dst.copy_from_slice(row);
                }
            }
        }
        *acc = Some(out);
    }
    let Some(out) = acc.as_mut() else { return Ok(false) };
    // Only the source's rows and columns change.
    let (sw, x0, y0) = (src.width.max(1), (src.left - left) as usize, (src.top - top) as usize);
    for (k, s) in src.px.iter().enumerate() {
        let Some(p) = out.px.get_mut((y0 + k / sw) * width + x0 + k % sw) else { continue };
        let a = coverage(blend, s[3] * opacity, src.left + (k % sw) as i64, src.top + (k / sw) as i64, record);
        *p = over(blend, *p, [s[0], s[1], s[2], a]);
    }
    Ok(true)
}

/// PSD or PSB file `bytes` with its layers as layers of the document (see the module
/// documentation), with warnings for what doesn't come in. `None` when the file has no layers (its
/// merged image is all of it). CMYK converts through `cmyk` (ink amounts 0..1 → sRGB).
pub(super) fn import(
    bytes: &[u8],
    hidden: bool,
    max_alloc: u64,
    cmyk: impl Fn([f32; 4]) -> [f32; 3],
) -> Result<Option<(Document, Vec<String>)>, String> {
    let mut r = Reader::new(bytes);
    let h = header(&mut r)?;
    // Layer info that holds only the count (which says whether the merged image has transparency)
    // has no layers either.
    let Some(info) = layer_info(h.layers, h.psb).filter(|info| info.len() > 2) else { return Ok(None) };
    let records = records(info, h.psb)?;
    if records.is_empty() {
        return Ok(None);
    }
    let items = tree(&records)?;
    let ppi = super::ppi::resolution(bytes).filter(|(x, y)| x.is_finite() && y.is_finite() && *x > 0.0 && *y > 0.0);
    let (sx, sy) = crate::cmd::place::pt_per_px(ppi);
    let doc = Document::new(f64::from(h.width) * sx, f64::from(h.height) * sy);
    let mut b = Builder {
        h: &h,
        records: &records,
        hidden,
        max_alloc,
        held: 0,
        cmyk,
        doc,
        to_doc: Affine::scale_non_uniform(sx, sy),
        ppi,
        layers: 0,
        notes: Notes::default(),
        noted: vec![false; records.len()],
        skipped: vec![false; records.len()],
        skip_log: vec![],
        spent: Cell::new(0),
    };
    let layers = b.list(&items)?;
    let Builder { mut doc, notes, .. } = b;
    if !layers.is_empty() {
        doc.layers = layers;
    }
    // Groups nest up to `MAX_DEPTH`, deeper than a document may: what is too deep merges into
    // the group around it, with a warning.
    let mut warnings = notes.warnings();
    doc.limit_depth_noting(&mut warnings);
    Ok(Some((doc, warnings)))
}
