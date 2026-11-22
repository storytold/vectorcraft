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
//! that differs from layer to layer. A layer or group in a group whose Knockout setting is 1 (read
//! as a shallow knockout) is merged with the layers below it in its group too, as the import option
//! describes: where it has pixels, they are cleared to transparency and it is drawn there at its fill
//! opacity, and its opacity fades that back toward them. In a pass-through group this is done only
//! when it and the visible layers below it are in the Normal mode. When a merge's canvases would need more
//! memory than allowed, the layer comes in alone: in the Normal mode, unclipped, or without its
//! knockout. Hidden layers are left out unless `hidden` asks for them. A hidden layer that would need
//! a merge (one in a mode VectorCraft lacks, one with a knockout in a group that changes the layers
//! below it, one that layers are clipped to, one clipped to another, an Invert, Threshold or
//! Posterize adjustment layer, or one below a merged layer) is always left out, with the layers
//! clipped to it.
//!
//! An Invert, Threshold or Posterize adjustment layer, as the import option describes for
//! adjustment layers, is merged with the visible layers below it in its group, which it changes;
//! clipped to a layer, it changes only the layers merged into that one. The rules are VectorCraft's
//! own: each color channel is taken as an 8-bit value of the sRGB color the layers are drawn in,
//! Invert turns `v` into `255 − v`, Threshold makes a pixel white when the luminance of its color
//! (Rec. 709 weights on linear sRGB, encoded back to 0..255) is above the level and black
//! otherwise, and Posterize maps each channel to the nearest of its evenly spaced levels, the higher
//! one on a tie. The adjusted color is composited over the color below in the layer's blend mode,
//! at its opacity times its fill opacity, through its layer mask; transparency stays as it was. One
//! with nothing visible below it in its group that draws (a layer that stores no pixels, other than
//! a solid color fill drawn over the canvas, a layer whose pixels are all transparent, or one at 0
//! opacity or 0 fill with nothing clipped to it doesn't, nor does a group of only such layers)
//! changes nothing; in a pass-through group it changes
//! what is below the group when the group is merged, and otherwise isn't applied. One that layers
//! are clipped to, and one clipped to another adjustment layer, isn't applied. One whose layer mask
//! needs more memory than allowed beside the canvases it would change isn't applied, and a clip, or
//! a merge for another layer, that it is part of goes on without it.
//!
//! Other adjustment layers, Invert, Threshold and Posterize data that can't be read (a level out of
//! range, too short, or a second adjustment block on the layer), fill layers that store no pixels and
//! whose fill isn't read or whose shape is a vector mask, layer effects that are switched on, vector
//! masks, and the knockouts of top-level layers, of clipped layers, of layers that others are clipped
//! to and of layers in a pass-through group with a layer in another mode, and other Knockout values,
//! and adjustments whose merge, their layer mask included, needs more memory than allowed aren't applied; the warnings count them
//! and the hidden layers left out.
//!
//! The file is untrusted: every count, length and size is checked before anything is allocated,
//! groups nest at most [`MAX_DEPTH`] deep (layers deeper than a document may nest merge into the
//! group around them, with a warning), each layer's pixels, like the canvases one step of a
//! merge holds together, must fit the memory allowed, and all the pixels decoded and the canvases
//! the merges make must fit [`TOTAL_DECODED`] times that. A merge being tried may take that much
//! past what was read before it. What the merges that aren't made, or that a hidden layer would
//! need, decoded and made is counted apart, against the same limit.

use std::cell::Cell;
use std::sync::Arc;

use image::RgbaImage;
use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_color::blend::{blend_rgb, composite};
use vectorcraft_color::cms::{Lab, lab::lab_to_srgb};
use vectorcraft_doc::{Document, ImageObject, LAYER_COLORS, LayerColor, Node, NodeKind, OpacityMask};
use vectorcraft_geom::Affine;

use super::load::{RasterImage, as_png};
use super::psdread::{Header, Reader, byte, header, layer_channel, layer_info, long_key, row_bytes, sample, to_rgb};
use crate::cmd::Session;
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

#[cfg(test)]
thread_local! {
    /// The layer lists imports on this thread have walked, and the most a test lets them walk
    /// before the import stops with an error.
    pub(super) static WALKS: Cell<(usize, usize)> = const { Cell::new((0, usize::MAX)) };
}

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
    /// An adjustment layer: the adjustment applied (`None` for another kind, or data that can't be
    /// read, which isn't applied).
    Adjustment(Option<Adjust>),
    /// A solid color fill layer: its stored pixels, or its color over the whole canvas when it
    /// stores none (nothing when the color can't be read, or when its shape is a vector mask).
    Solid(Option<Paint>),
    /// A gradient or pattern fill layer: its stored pixels, if any.
    Fill,
}

/// An adjustment the import applies.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Adjust {
    /// `nvrt`: each color channel's value `v` (0..255) becomes `255 − v`.
    Invert,
    /// `thrs`: pixels whose luminance is above the level (1..255) become white, the others black.
    Threshold(u8),
    /// `post`: each color channel takes the nearest of this many (2..255) evenly spaced levels.
    Posterize(u8),
}

impl Adjust {
    /// The adjustment of key `key` whose data is `data`; `None` when the data can't be read or its
    /// value is out of range. Invert has no data; Threshold's level and Posterize's count of levels
    /// are a 16-bit number followed by 2 bytes of padding, as psd-tools (MIT) reads them
    /// (<https://github.com/psd-tools/psd-tools>, `psd/adjustments.py`); data shorter than the 4
    /// bytes can't be read.
    fn read(key: &[u8; 4], data: &[u8]) -> Option<Self> {
        let value = || data.get(..4).map(|v| u16::from_be_bytes([v[0], v[1]]));
        match key {
            b"nvrt" => Some(Adjust::Invert),
            b"thrs" => u8::try_from(value()?).ok().filter(|&l| l >= 1).map(Adjust::Threshold),
            b"post" => u8::try_from(value()?).ok().filter(|&n| n >= 2).map(Adjust::Posterize),
            _ => None,
        }
    }

    /// Straight RGB, 0..1, adjusted. Each channel is taken as an 8-bit value. Threshold measures
    /// the sRGB relative luminance (Rec. 709 weights on the linear values), encoded back to 0..255:
    /// above the level is white, at or below it black. Posterize maps a value to the nearest of
    /// its levels `k × 255 / (n − 1)`, the higher one on a tie.
    fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        use vectorcraft_color::cms::lab::{linear_to_srgb, srgb_to_linear};
        let v = rgb.map(byte);
        match self {
            Adjust::Invert => v.map(|v| f32::from(255 - v) / 255.0),
            Adjust::Threshold(level) => {
                let lin = v.map(|v| srgb_to_linear(f32::from(v) / 255.0));
                let y = byte(linear_to_srgb(0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]));
                [if y > level { 1.0 } else { 0.0 }; 3]
            }
            Adjust::Posterize(n) => {
                let steps = u32::from(n) - 1;
                v.map(|v| {
                    let k = (2 * u32::from(v) * steps + 255) / 510;
                    ((2 * k * 255 + steps) / (2 * steps)) as f32 / 255.0
                })
            }
        }
    }
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
    /// The Knockout setting's byte (`knko`): 0 none, 1 a shallow knockout. Any other value isn't
    /// applied.
    knockout: u8,
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
        knockout: 0,
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
        // Whether an adjustment block has been read.
        let mut adjusted = false;
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
                b"knko" => self.knockout = data.first().copied().unwrap_or(0),
                // A second adjustment block, with other blocks between them or not, makes the
                // layer's adjustment unreadable.
                k if ADJUSTMENTS.contains(&k) => {
                    let again = std::mem::replace(&mut adjusted, true);
                    self.content = Content::Adjustment(Adjust::read(k, data).filter(|_| !again));
                }
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

    /// The adjustment the layer applies, if it is an adjustment layer of a kind the import applies.
    fn adjust(&self) -> Option<Adjust> {
        match self.content {
            Content::Adjustment(a) => a,
            _ => None,
        }
    }

    /// Does the layer cover the whole canvas without drawing (an adjustment layer, or a fill that
    /// stores no pixels, has no vector mask and isn't drawn from its color)? Layers clipped to it
    /// show over the whole canvas.
    fn covers_without_drawing(&self) -> bool {
        match self.content {
            Content::Adjustment(_) => true,
            Content::Solid(None) | Content::Fill => self.stores_nothing() && !self.vector_mask,
            _ => false,
        }
    }

    /// Does the layer draw nothing (an adjustment layer, or one that stores no pixels and isn't a
    /// fill drawn from its color over the canvas; see [`Builder::image`])?
    fn draws_nothing(&self) -> bool {
        self.covers_without_drawing() || (self.stores_nothing() && !self.color_over_canvas())
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

/// The layers of the group of record `g`, looked for in `items` and the groups inside them.
fn find_group(items: &[Item], g: usize) -> Option<&[Item]> {
    items.iter().find_map(|item| match item {
        Item::Group(i, children) if *i == g => Some(children.as_slice()),
        Item::Group(_, children) => find_group(children, g),
        Item::Layer(_) => None,
    })
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
    /// Layers are clipped to it, merged into it or not.
    base: bool,
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
    /// A group's record and its layers.
    Group(usize, &'i [Item]),
    /// Layers merged into one image.
    Merged(Canvas),
}

/// Where a layer list is.
#[derive(Clone, Copy, PartialEq)]
enum Level {
    /// The file's top level.
    Top,
    /// An isolated group, or a group drawn on its own.
    Group,
    /// A pass-through group whose layers blend with what is below it.
    Pass,
}

/// What a knockout's merge with the layers below it gives.
enum Knocked {
    /// The merged canvas.
    Merged(Canvas),
    /// Nothing below it draws, it draws nothing, it is at 0 fill and cuts no painted pixel below
    /// it, or what is below it and the merge are both transparent: it comes in as a layer of its own.
    Nothing,
    /// The canvases would need more memory than allowed.
    TooLarge,
}

/// A unit's body drawn alone.
#[derive(Clone)]
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
    /// Adjustment layers of a kind the import applies that it couldn't apply (warned of with
    /// `adjustments`).
    unadjusted: usize,
    fills: usize,
    effects: usize,
    vector_masks: usize,
    /// Layers whose knockout wasn't applied.
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
    /// The bytes spent so far (see [`Builder::spent`]).
    spent: u64,
    skipped: usize,
    /// The knockouts counted as not applied.
    unknocked: usize,
    merged: usize,
    unmerged: usize,
    unclipped: usize,
    clipped_out: usize,
    /// The groups found to read no pixels.
    blank: usize,
    /// The adjustment layers counted as not applied (see [`Builder::unadjust`]).
    unadjusted: usize,
}

/// Adds the warning for `n` things to `out`: `one` when `n` is 1, `many` with the count in place
/// of `{n}` when it is more, none when it is 0. Each form is a whole sentence, the key of its own
/// catalog row.
fn note(out: &mut Vec<String>, n: usize, (one, many): (&str, &str)) {
    match n {
        0 => {}
        1 => out.push(one.to_string()),
        n => out.push(many.replace("{n}", &n.to_string())),
    }
}

/// The layer warnings in English, the form for one layer and the form for more, in the order a
/// file's warnings list them.
pub const LAYER_WARNINGS: [(&str, &str); 10] = [
    ("1 type layer came in as pixels: the text isn't editable", "{n} type layers came in as pixels: the text isn't editable"),
    ("1 adjustment layer was not applied", "{n} adjustment layers were not applied"),
    (
        "1 fill layer was not drawn (a gradient or pattern fill, a color that can't be read, or a shape that stores no pixels)",
        "{n} fill layers were not drawn (a gradient or pattern fill, a color that can't be read, or a shape that stores no pixels)",
    ),
    ("the layer effects of 1 layer weren't drawn", "the layer effects of {n} layers weren't drawn"),
    ("the vector masks of 1 layer weren't applied", "the vector masks of {n} layers weren't applied"),
    ("the knockout of 1 layer wasn't applied", "the knockout of {n} layers wasn't applied"),
    (
        "1 layer was left out (hidden layers and the layers clipped to them)",
        "{n} layers were left out (hidden layers and the layers clipped to them)",
    ),
    ("1 layer clipped to a fill layer that wasn't drawn was left out", "{n} layers clipped to fill layers that weren't drawn were left out"),
    (
        "1 layer in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed",
        "{n} layers in blend modes VectorCraft doesn't have came in as Normal: the merge needs more memory than allowed",
    ),
    (
        "1 layer clipped to the layer below came in unclipped: the merge needs more memory than allowed",
        "{n} layers clipped to the layer below came in unclipped: the merge needs more memory than allowed",
    ),
];

/// The warning for layers merged with the layers below them, `{modes}` the blend modes' names.
pub const MERGED_WARNING: &str = "layers in blend modes VectorCraft doesn't have ({modes}) were merged with the layers below them";

impl Notes {
    fn warnings(self) -> Vec<String> {
        let [types, adjustments, fills, effects, masks, knockouts, hidden, clipped_out, unmerged, unclipped] = LAYER_WARNINGS;
        let mut out = vec![];
        note(&mut out, self.types, types);
        note(&mut out, self.adjustments + self.unadjusted, adjustments);
        note(&mut out, self.fills, fills);
        note(&mut out, self.effects, effects);
        note(&mut out, self.vector_masks, masks);
        note(&mut out, self.knockouts, knockouts);
        note(&mut out, self.hidden, hidden);
        note(&mut out, self.clipped_out, clipped_out);
        if !self.merged.is_empty() {
            out.push(MERGED_WARNING.replace("{modes}", &self.merged.join(", ")));
        }
        note(&mut out, self.unmerged, unmerged);
        note(&mut out, self.unclipped, unclipped);
        out
    }

    fn counts(&self, spent: u64, skipped: usize, unknocked: usize, blank: usize, unadjusted: usize) -> Counts {
        Counts {
            spent,
            skipped,
            unknocked,
            merged: self.merged.len(),
            unmerged: self.unmerged,
            unclipped: self.unclipped,
            clipped_out: self.clipped_out,
            blank,
            unadjusted,
        }
    }
}

struct Builder<'a, F> {
    h: &'a Header<'a>,
    records: &'a [Record<'a>],
    /// The records as layers and groups (see [`tree`]).
    tree: &'a [Item],
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
    /// Those records, in the order they were counted.
    noted_log: Vec<usize>,
    /// The records counted as hidden layers left out.
    skipped: Vec<bool>,
    /// Those records, in the order they were counted.
    skip_log: Vec<usize>,
    /// The records counted as knockouts not applied.
    unknocked: Vec<bool>,
    /// Those records, in the order they were counted.
    unknock_log: Vec<usize>,
    /// The records counted as layers clipped to a fill that isn't drawn.
    clipped_out: Vec<bool>,
    /// Those records, in the order they were counted.
    clipped_out_log: Vec<usize>,
    /// The records counted as clipped layers left unclipped.
    unclipped: Vec<bool>,
    /// Those records, in the order they were counted.
    unclip_log: Vec<usize>,
    /// The records counted as adjustment layers the import applies that weren't applied.
    unadjusted: Vec<bool>,
    /// Those records, in the order they were counted.
    unadjust_log: Vec<usize>,
    /// The records that came in as layers of the document (hidden ones too, with `hidden`): not left
    /// out, though a drawing for a merge that wasn't made counted them.
    kept: Vec<bool>,
    /// Bytes of layer and mask pixels decoded, and of canvases the merges made, so far.
    spent: Cell<u64>,
    /// Those bytes that merges not made, and merges a hidden layer would need, took: given back to
    /// `spent` and counted here instead, against the same limit.
    wasted: Cell<u64>,
    /// `spent` when the innermost merge being tried began: until it is made or given up, it may
    /// spend the whole allowance past that (see [`Self::spend`]).
    trial: Cell<u64>,
    /// The layers whose pixels were read so far (each time they were).
    painted: Cell<usize>,
    /// What each group gave when drawn alone without reading any layer's pixels (nothing, or a
    /// canvas without pixels, whatever the memory left): drawing it again gives that without
    /// walking its layers.
    blank: Vec<Option<Drawn>>,
    /// The pass-through groups that laid nothing onto nothing (see [`Self::laid_nothing`]).
    laid_blank: Vec<bool>,
    /// The groups of both, in the order they were found.
    blank_log: Vec<usize>,
    /// How many times a merge was counted as not made because it needed more memory than allowed.
    squeezed: usize,
    /// The adjustment layer whose own merge with the layers below it is being made: a layer mask of
    /// its too large to read stops that merge, where one of another adjustment layer drawn in it
    /// leaves that one out.
    adjusting: Option<usize>,
    /// Whether each record's stored transparency is 0 everywhere, once [`Self::clear`] has read it.
    clear: Vec<Cell<Option<bool>>>,
    /// Whether each record's layer mask hides all of it, once [`Self::masked_out`] has read it.
    masked_out: Vec<Cell<Option<bool>>>,
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
    /// allowance, [`TOTAL_DECODED`] times the memory allowed for one image. While a merge is tried
    /// (see [`Self::try_from_here`]), only what it spent counts against the allowance.
    fn spend(&self, bytes: u64) -> Result<(), String> {
        let total = self.spent.get().saturating_add(bytes);
        self.spent.set(total);
        if total.saturating_sub(self.trial.get()) > self.max_alloc.saturating_mul(TOTAL_DECODED) {
            return Err("the PSD file's layers take more memory to read than allowed".into());
        }
        Ok(())
    }

    /// Start trying a merge that may not be made: what it spends counts against the whole
    /// allowance, however much was spent before it. Gives what [`Self::tried`] takes back.
    fn try_from_here(&self) -> u64 {
        self.trial.replace(self.spent.get())
    }

    /// End the merge tried since [`Self::try_from_here`] gave `outer`. A merge that is `made` keeps
    /// what it spent, which must then fit the allowance with what was spent before it.
    fn tried(&self, outer: u64, made: bool) -> Result<(), String> {
        self.trial.set(outer);
        if made { self.spend(0) } else { Ok(()) }
    }

    /// Move the bytes spent since `spent` was the count to [`Self::wasted`], which has an allowance
    /// of its own as large: what a merge that isn't made decoded doesn't take the room for reading
    /// the layers apart.
    fn refund(&self, spent: u64) -> Result<(), String> {
        let since = self.spent.get().saturating_sub(spent);
        self.spent.set(self.spent.get().min(spent));
        let total = self.wasted.get().saturating_add(since);
        self.wasted.set(total);
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
            Content::Adjustment(_) => return Ok(None),
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
                self.painted.set(self.painted.get() + 1);
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
        self.painted.set(self.painted.get() + 1);
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

    /// Does record `i`'s layer mask, switched on, fit the memory allowed for reading it with
    /// [`Self::mask`] beside `live` bytes of canvases held while it is read, and the copies of what
    /// is below the pass-through groups being merged? `true` when it has none, and when its size is
    /// damaged (reading it says so).
    fn mask_fits(&self, i: usize, live: u64) -> Result<bool, String> {
        let rec = self.record(i)?;
        let Some(m) = rec.mask.filter(|m| !m.disabled) else { return Ok(true) };
        if !rec.channels.iter().any(|(c, _)| *c == -2) {
            return Ok(true);
        }
        let (w, rows) = m.bounds.size();
        let max_side = if self.h.psb { 300_000 } else { 30_000 };
        if w > max_side || rows > max_side {
            return Ok(true);
        }
        let bytes = (row_bytes(w, self.h.depth) as u64 + w as u64).saturating_mul(rows as u64);
        Ok(bytes.saturating_add(live) <= self.room())
    }

    /// Does record `i` store pixels that are all fully transparent, so that it paints nothing?
    /// Its transparency is read once. `false` when it stores none, or has no transparency, or its
    /// transparency can't be read within the memory allowed or is damaged (drawing it says so); one
    /// that didn't fit the memory allowed is read again the next time.
    fn clear(&self, i: usize) -> Result<bool, String> {
        if let Some(clear) = self.clear.get(i).and_then(Cell::get) {
            return Ok(clear);
        }
        let rec = self.record(i)?;
        let (w, rows) = rec.bounds.size();
        let max_side = if self.h.psb { 300_000 } else { 30_000 };
        let alpha = rec.channels.iter().find(|(c, _)| *c == -1).map(|(_, data)| data);
        let clear = match alpha {
            Some(data) if !rec.draws_nothing() && !rec.color_over_canvas() && w <= max_side && rows <= max_side => {
                let (depth, row) = (self.h.depth, row_bytes(w, self.h.depth));
                let bytes = (row as u64).saturating_mul(rows as u64);
                if bytes > self.room() {
                    return Ok(false);
                }
                // Not counted against the import's allowance: the transparency is read again, and
                // counted, where the layer is drawn.
                match layer_channel(data, rows, row, self.h.psb, depth) {
                    Ok(plane) => {
                        (0..rows).all(|y| plane.get(y * row..(y + 1) * row).is_some_and(|r| (0..w).all(|x| byte(sample(r, x, depth, false)) == 0)))
                    }
                    Err(_) => false,
                }
            }
            _ => false,
        };
        if let Some(c) = self.clear.get(i) {
            c.set(Some(clear));
        }
        Ok(clear)
    }

    /// Does record `i` have a layer mask, switched on, that hides all of it? The mask is 0 over every
    /// pixel that isn't fully transparent of the layers whose stored pixels show where it draws: a
    /// layer's own, and a group's those of its layers, in the groups inside it too (every pixel of a
    /// layer whose transparency can't be read). For an adjustment layer, a layer that stores no
    /// pixels and a group with one of those in it, which can change pixels anywhere below them, it
    /// is 0 everywhere, the value outside it included. The mask is read once. `false` when it has
    /// none, or it can't be read within the memory allowed or is damaged (drawing it says so); a
    /// `false` that a read skipped for lack of memory decided is worked out again the next time.
    /// `group` says the record opens a group.
    fn masked_out(&self, i: usize, group: bool) -> Result<bool, String> {
        if let Some(out) = self.masked_out.get(i).and_then(Cell::get) {
            return Ok(out);
        }
        let rec = self.record(i)?;
        // The layers whose stored pixels show where it draws, when they all do.
        let layers = if group {
            match find_group(self.tree, i) {
                Some(items) => {
                    let mut layers = vec![];
                    self.painters(items, &mut layers)?.then_some(layers)
                }
                None => None,
            }
        } else {
            (!rec.stores_nothing() && !matches!(rec.content, Content::Adjustment(_))).then(|| vec![i])
        };
        let outside = rec.mask.map_or(0, |m| if m.inverted { 255 - m.default } else { m.default });
        // Whether a read skipped for lack of memory decided it.
        let mut squeezed = false;
        let out = if (layers.is_none() && outside != 0) || !self.has_mask(Some(i))? {
            false
        } else if !self.mask_fits(i, 0)? {
            squeezed = true;
            false
        } else {
            // Not counted against the import's allowance: the mask is read again, and counted, where
            // the layer is drawn or its opacity mask is made.
            let spent = self.spent.get();
            let read = self.mask(i, false);
            self.spent.set(spent);
            match read {
                // 0 everywhere hides all of anything.
                Ok(Some((_, _, m, 0))) if m.pixels().all(|p| p.0[0] == 0) => true,
                Ok(Some(mask)) => match &layers {
                    Some(layers) => {
                        let mut hides = true;
                        for &l in layers {
                            if !self.hides(i, l, &mask, &mut squeezed)? {
                                hides = false;
                                break;
                            }
                        }
                        hides
                    }
                    None => false,
                },
                Ok(None) => false,
                Err(_) => {
                    squeezed = true;
                    false
                }
            }
        };
        if (out || !squeezed)
            && let Some(c) = self.masked_out.get(i)
        {
            c.set(Some(out));
        }
        Ok(out)
    }

    /// Push onto `out` the layers in `items` (in the groups inside them too) that store pixels.
    /// `false` when one of them can change or draw pixels where it stores none: an adjustment layer,
    /// or a fill that stores no pixels.
    fn painters(&self, items: &[Item], out: &mut Vec<usize>) -> Result<bool, String> {
        for item in items {
            match item {
                Item::Layer(l) => {
                    let r = self.record(*l)?;
                    if r.covers_without_drawing() || r.color_over_canvas() || matches!(r.content, Content::Adjustment(_)) {
                        return Ok(false);
                    }
                    if !r.stores_nothing() {
                        out.push(*l);
                    }
                }
                Item::Group(_, children) => {
                    if !self.painters(children, out)? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Does `mask`, record `i`'s layer mask read with [`Self::mask`], hide every pixel of layer `l`
    /// that isn't fully transparent? Every pixel counts when `l`'s transparency can't be read beside
    /// the mask; `squeezed` is set when that is for lack of memory.
    fn hides(&self, i: usize, l: usize, mask: &(i64, i64, image::GrayImage, u8), squeezed: &mut bool) -> Result<bool, String> {
        let (left, top, m, outside) = mask;
        let (left, top, outside) = (*left, *top, *outside);
        let (right, bottom) = (left + i64::from(m.width()), top + i64::from(m.height()));
        let rec = self.record(l)?;
        let b = rec.bounds;
        let (bl, bt, br, bb) = (i64::from(b.left), i64::from(b.top), i64::from(b.right), i64::from(b.bottom));
        let max_side = if self.h.psb { 300_000 } else { 30_000 };
        let (w, rows) = b.size();
        // The sides are checked first: the row's size could overflow otherwise.
        let fits = w <= max_side && rows <= max_side;
        let row = if fits { row_bytes(w, self.h.depth) } else { 0 };
        // Its transparency, when it stores one that can be read beside the mask. Not counted against
        // the import's allowance: it is read again, and counted, where the layer is drawn.
        let alpha = match rec.channels.iter().find(|(c, _)| *c == -1) {
            Some((_, data)) if fits && !rec.draws_nothing() && !rec.color_over_canvas() => {
                if self.mask_fits(i, (row as u64).saturating_mul(rows as u64))? {
                    layer_channel(data, rows, row, self.h.psb, self.h.depth).ok()
                } else {
                    *squeezed = true;
                    None
                }
            }
            _ => None,
        };
        let within = left <= bl && top <= bt && br <= right && bb <= bottom;
        if outside != 0 && !within && alpha.is_none() {
            return Ok(false);
        }
        // Its pixels the mask's value outside it can show, or only those under the mask.
        let (x0, y0, x1, y1) = if outside != 0 { (bl, bt, br, bb) } else { (bl.max(left), bt.max(top), br.min(right), bb.min(bottom)) };
        for y in y0..y1 {
            let line = match &alpha {
                Some(plane) => match plane.get((y - bt) as usize * row..((y - bt) as usize + 1) * row) {
                    Some(r) => Some(r),
                    None => return Ok(false),
                },
                None => None,
            };
            for x in x0..x1 {
                let under = (left..right).contains(&x) && (top..bottom).contains(&y);
                let v = if under { m.get_pixel((x - left) as u32, (y - top) as u32).0[0] } else { outside };
                if v != 0 && line.is_none_or(|r| byte(sample(r, (x - bl) as usize, self.h.depth, false)) != 0) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
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
                Item::Group(i, children) => (*i, Body::Group(*i, children)),
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
                        // A clipped layer's knockout isn't applied.
                        if self.record(i)?.knockout != 0 {
                            self.unknock(i);
                        }
                        if let Some(u) = out.get_mut(b) {
                            u.base = true;
                        }
                        let spent = self.spent.get();
                        let outer = self.try_from_here();
                        let made = !unclipped && self.clip(&mut out, b, &body, i, &name, blend, opacity)?;
                        self.tried(outer, made)?;
                        if made {
                            continue;
                        }
                        self.refund(spent)?;
                        // The merge doesn't fit: this layer and the next ones clipped to the same
                        // base come in as layers of their own; an adjustment layer, which would
                        // change more than its base, isn't applied.
                        unclipped = true;
                        if self.adjustment(&body)?.is_some() {
                            self.squeezed += 1;
                            self.unadjust(i);
                            continue;
                        }
                        self.unclip(i);
                        out.push(Unit { name, visible: true, opacity, blend, mask_of: Some(i), record: i, body, clip: None, base: false });
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
            out.push(Unit { name, visible: !hidden, opacity, blend, mask_of: Some(i), record: i, body, clip: None, base: false });
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
        let (whole, undrawn, fill, base_opacity, base_adjusts, base_adjustment) = {
            let r = self.record(u.record)?;
            let layer = matches!(u.body, Body::Layer(_));
            // The base as a layer, or as the canvas of the layers already clipped to it.
            let adjustment = (layer || u.clip.is_some()) && matches!(r.content, Content::Adjustment(_));
            (layer && r.covers_without_drawing(), layer && r.undrawn_shape(), r.fill, r.opacity, layer && r.adjust().is_some(), adjustment)
        };
        // A fill whose shape is a vector mask isn't drawn, and nothing clipped to it is either.
        if undrawn && u.clip.is_none() {
            self.clip_out(i);
            return Ok(true);
        }
        // A clipped adjustment layer changes the merged canvas in place, through its layer mask.
        let adjust = self.adjustment(body)?;
        // Its layer mask too large to read needs more memory than allowed: it isn't applied, and the
        // layers clipped above it still clip to the base.
        if adjust.is_some() && !base_adjustment && !self.mask_fits(i, 0)? {
            self.squeezed += 1;
            self.unadjust(i);
            return Ok(true);
        }
        let layer = match adjust {
            Some(_) => Canvas::default(),
            None => match self.draw_body(body, Some(i))? {
                Drawn::Nothing => return Ok(true),
                Drawn::TooLarge => return Ok(false),
                Drawn::Canvas(c) => c,
            },
        };
        let room = self.room();
        let layer_bytes = 16 * layer.px.len() as u64;
        let Some(u) = out.get_mut(b) else { return Ok(true) };
        if u.clip.is_none() {
            // An adjustment layer that others are clipped to isn't applied.
            if base_adjusts {
                let record = u.record;
                self.unadjust(record);
            }
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
        if let Some(a) = adjust {
            // Clipped to an adjustment layer, it would change the layers below that one: not applied.
            if base_adjustment {
                self.unadjust(i);
                return Ok(true);
            }
            // The mask is read beside the merged canvas and the base's shape.
            let shape = match &u.clip {
                Some(Clip::Shape(s)) => 4 * s.len() as u64,
                _ => 0,
            };
            if !self.mask_fits(i, (16 * c.px.len() as u64).saturating_add(shape))? {
                self.squeezed += 1;
                self.unadjust(i);
                return Ok(true);
            }
            let mask = self.mask(i, false)?;
            adjust_onto(c, a, blend, opacity, mask.as_ref(), i);
            u.name.push_str(" + ");
            u.name.push_str(name);
            return Ok(true);
        }
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
            Some(n) => {
                *n = true;
                self.noted_log.push(i);
            }
            None => {}
        }
        let (content, empty, effects, vector_mask) = {
            let rec = self.record(i)?;
            (rec.content, rec.stores_nothing(), rec.effects, rec.vector_mask)
        };
        match content {
            Content::Type => self.notes.types += 1,
            // One the import applies is counted only where it isn't (see `unadjust`).
            Content::Adjustment(None) => self.notes.adjustments += 1,
            // A fill drawn from neither its pixels nor its color.
            Content::Fill | Content::Solid(None) if empty => self.notes.fills += 1,
            Content::Solid(Some(_)) if empty && vector_mask => self.notes.fills += 1,
            _ => {}
        }
        self.notes.effects += usize::from(effects);
        self.notes.vector_masks += usize::from(vector_mask);
        Ok(())
    }

    /// What [`Self::note`] has counted so far: the length of its log, and the counts.
    fn noted_so_far(&self) -> (usize, [usize; 5]) {
        let n = &self.notes;
        (self.noted_log.len(), [n.types, n.adjustments, n.fills, n.effects, n.vector_masks])
    }

    /// Forget what [`Self::note`] counted since `so_far`, for layers that are left out.
    fn unnote(&mut self, (logged, counts): (usize, [usize; 5])) {
        for i in self.noted_log.split_off(logged) {
            if let Some(n) = self.noted.get_mut(i) {
                *n = false;
            }
        }
        let n = &mut self.notes;
        [n.types, n.adjustments, n.fills, n.effects, n.vector_masks] = counts;
    }

    /// Count record `i` as a layer whose knockout isn't applied (once).
    fn unknock(&mut self, i: usize) {
        if let Some(s) = self.unknocked.get_mut(i)
            && !*s
        {
            *s = true;
            self.notes.knockouts += 1;
            self.unknock_log.push(i);
        }
    }

    /// Does unit `u`, at `level` above the units `below` it, take a shallow knockout this import
    /// applies? Only a layer or group in a group, neither clipped nor a base of clipped layers, does;
    /// in a pass-through group, only when it and the visible units below it are in the Normal mode, so
    /// that none of them blends with what is below the group. Any other knockout is counted as not
    /// applied. One at 0 opacity, or with no visible unit below it, changes nothing below it: it needs
    /// no merge, and a hidden one isn't left out for it.
    fn knocks(&mut self, u: &Unit<'_>, below: &[Unit<'_>], level: Level) -> Result<bool, String> {
        let rec = self.record(u.record)?;
        if rec.knockout == 0 {
            return Ok(false);
        }
        let normal = |u: &Unit<'_>| u.blend == Blend::Mode(BlendMode::Normal);
        let blends_out = level == Level::Pass && !(normal(u) && below.iter().filter(|b| b.visible).all(normal));
        let applies = rec.knockout == 1 && level != Level::Top && !blends_out && !rec.clipped && !u.base && !matches!(u.body, Body::Merged(_));
        // Its fill opacity isn't checked: at 0 fill it still clears what is below it.
        let changes = rec.opacity > 0.0 && below.iter().any(|b| b.visible);
        if !applies {
            self.unknock(u.record);
        }
        Ok(applies && changes)
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

    /// Count record `i` as a layer clipped to a fill that isn't drawn (once).
    fn clip_out(&mut self, i: usize) {
        if let Some(s) = self.clipped_out.get_mut(i)
            && !*s
        {
            *s = true;
            self.notes.clipped_out += 1;
            self.clipped_out_log.push(i);
        }
    }

    /// Count record `i` as a clipped layer left unclipped (once).
    fn unclip(&mut self, i: usize) {
        self.squeezed += 1;
        if let Some(s) = self.unclipped.get_mut(i)
            && !*s
        {
            *s = true;
            self.notes.unclipped += 1;
            self.unclip_log.push(i);
        }
    }

    /// Count record `i` as an adjustment layer the import applies that wasn't applied (once).
    fn unadjust(&mut self, i: usize) {
        if let Some(s) = self.unadjusted.get_mut(i)
            && !*s
        {
            *s = true;
            self.notes.unadjusted += 1;
            self.unadjust_log.push(i);
        }
    }

    /// The adjustment `body` applies: a layer record that is an adjustment layer the import applies.
    fn adjustment(&self, body: &Body<'_>) -> Result<Option<Adjust>, String> {
        match body {
            Body::Layer(i) => Ok(self.record(*i)?.adjust()),
            _ => Ok(None),
        }
    }

    /// The records of the adjustment layers kept in `out` at `level` (in a pass-through group, to
    /// change what is below the group), those kept in the visible pass-through groups of `out`
    /// included: from the first one with nothing that draws below it in `out`, as [`Self::leading`]
    /// keeps them. A group whose layer mask hides all of it keeps none.
    fn pending(&self, out: &[Unit<'_>], level: Level) -> Result<Vec<usize>, String> {
        let (mut records, mut draws) = (vec![], false);
        if level != Level::Pass {
            return Ok(records);
        }
        // A hidden unit, kept with the hidden layers, or one at 0 opacity, changes nothing.
        for u in out.iter().filter(|u| u.visible && u.opacity > 0.0) {
            if self.adjustment(&u.body)?.is_some() {
                records.push(u.record);
                continue;
            }
            if let (Body::Group(_, items), Blend::Pass) = (&u.body, u.blend)
                && (!draws || !records.is_empty())
                && !u.mask_of.map(|g| self.masked_out(g, true)).transpose()?.unwrap_or(false)
            {
                let (inner, inner_draws) = self.leading(items)?;
                records.extend(inner);
                draws |= inner_draws;
                continue;
            }
            draws = draws || self.unit_draws(u)?;
        }
        Ok(records)
    }

    /// Does unit `u` draw? An adjustment layer, a layer that draws nothing or whose pixels are all
    /// transparent, a group of such layers, and a unit whose layer mask hides all of it, don't.
    fn unit_draws(&self, u: &Unit<'_>) -> Result<bool, String> {
        let draws = match &u.body {
            Body::Layer(i) => !self.record(*i)?.draws_nothing() && !self.clear(*i)?,
            Body::Group(_, items) => self.draws(items)?,
            // Clipped layers merged with a base that has no pixels have none either.
            Body::Merged(c) => c.px.iter().any(|p| p[3] > 0.0),
        };
        // A base merged with the layers clipped to it draws within its own pixels, or, when it covers
        // the canvas without drawing, anywhere: its mask hides all of it as it hides the base (see
        // `base_draws`).
        let hidden = |m: usize| match &u.body {
            Body::Layer(_) => self.masked_out(m, false),
            Body::Group(..) => self.masked_out(m, true),
            Body::Merged(_) => self.masked_out(m, matches!(self.record(m)?.divider, 1 | 2)),
        };
        Ok(draws && !u.mask_of.map(hidden).transpose()?.unwrap_or(false))
    }

    /// The adjustment layers the import applies that a pass-through group with the layers `items`
    /// keeps to change what is below it: from the first one with nothing that draws below it in the
    /// group (hidden and clipped layers, other adjustment layers, layers and groups at 0 opacity, and
    /// layers and groups that draw nothing aren't counted), it and every one above it, in the
    /// pass-through groups inside it too; one at 0 opacity isn't kept. Also whether anything in
    /// `items` draws.
    fn leading(&self, items: &[Item]) -> Result<(Vec<usize>, bool), String> {
        let (mut records, mut draws) = (vec![], false);
        for (k, item) in items.iter().enumerate() {
            let (i, r, children) = match item {
                Item::Layer(i) => (*i, self.record(*i)?, None),
                Item::Group(g, children) => (*g, self.record(*g)?, Some(children)),
            };
            // A clipped one draws as part of the one it is clipped to.
            if r.hidden || r.clipped {
                continue;
            }
            let (clips, drawn) = self.base_draws(items, k)?;
            // One in a mode VectorCraft lacks (an adjustment layer aside), or with a shallow knockout
            // that cuts what is below it, is merged with the layers below it in the group, which
            // takes the ones kept so far with them (see `merged`).
            if (matches!(r.blend, Blend::Other(_)) && (children.is_some() || r.adjust().is_none())) || self.cuts(items, k, clips, drawn && draws)? {
                records.clear();
            }
            // One that layers are clipped to isn't kept, and a pass-through group they are clipped to
            // is drawn alone (see `clip`). One at 0 opacity changes nothing (see `adjust_below`).
            let keeps = !clips && r.opacity * r.fill > 0.0 && (!draws || !records.is_empty());
            match children {
                None if r.adjust().is_some() && !clips => {
                    if keeps {
                        records.push(i);
                    }
                }
                // A pass-through group whose layer mask hides all of it draws and changes nothing.
                Some(children) if r.blend == Blend::Pass && keeps && !self.masked_out(i, true)? => {
                    let (inner, inner_draws) = self.leading(children)?;
                    records.extend(inner);
                    draws |= inner_draws;
                }
                _ => draws |= drawn,
            }
        }
        Ok((records, draws))
    }

    /// Does item `k` of `items`, a pass-through group's layers, take a shallow knockout that is merged
    /// with the layers below it (see [`Self::knocks`])? Only one that `clips` nothing, is in the
    /// Normal mode over visible layers in the Normal mode back to the last one merged for its mode,
    /// and where it and something below it draw (`draws`).
    fn cuts(&self, items: &[Item], k: usize, clips: bool, draws: bool) -> Result<bool, String> {
        let record = |item: &Item| match item {
            Item::Layer(i) | Item::Group(i, _) => self.record(*i),
        };
        let Some(r) = items.get(k).map(record).transpose()? else { return Ok(false) };
        let normal = Blend::Mode(BlendMode::Normal);
        if r.knockout != 1 || clips || !draws || r.blend != normal {
            return Ok(false);
        }
        for below in items.get(..k).unwrap_or_default().iter().rev() {
            let b = record(below)?;
            if b.hidden || b.clipped {
                continue;
            }
            match b.blend {
                Blend::Other(_) if matches!(below, Item::Group(..)) || b.adjust().is_none() => break,
                blend if blend != normal => return Ok(false),
                _ => {}
            }
        }
        Ok(true)
    }

    /// Does anything in `items` draw: a visible layer or group that isn't clipped and draws (see
    /// [`Self::base_draws`])?
    fn draws(&self, items: &[Item]) -> Result<bool, String> {
        for (k, item) in items.iter().enumerate() {
            let r = match item {
                Item::Layer(i) => self.record(*i)?,
                Item::Group(g, _) => self.record(*g)?,
            };
            if !r.hidden && !r.clipped && self.base_draws(items, k)?.1 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// For item `k` of `items`, visible and not clipped: whether visible layers are clipped to it,
    /// and whether it draws, above 0 opacity: by its own pixels (a group's, those of its layers)
    /// above 0 fill, or by those of the layers clipped to it that draw, which show in its pixels at
    /// any fill, or over the canvas when it is a layer that covers it without drawing and that its
    /// layer mask doesn't hide all of.
    fn base_draws(&self, items: &[Item], k: usize) -> Result<(bool, bool), String> {
        let Some(item) = items.get(k) else { return Ok((false, false)) };
        let (r, own) = self.pixels(item)?;
        let covers = match item {
            Item::Layer(i) => r.covers_without_drawing() && !self.masked_out(*i, false)?,
            Item::Group(..) => false,
        };
        let (mut clips, mut clips_draw) = (false, false);
        for above in items.get(k + 1..).unwrap_or_default() {
            let c = match above {
                Item::Layer(i) => self.record(*i)?,
                Item::Group(g, _) => self.record(*g)?,
            };
            if !c.clipped {
                break;
            }
            if c.hidden {
                continue;
            }
            clips = true;
            if !clips_draw && c.opacity * c.fill > 0.0 {
                clips_draw = self.pixels(above)?.1;
            }
        }
        let drawn = (r.fill > 0.0 && own) || (clips_draw && (covers || own));
        Ok((clips, r.opacity > 0.0 && drawn))
    }

    /// `item`'s record, and whether it has pixels of its own: a layer that draws, or a group with
    /// something in it that draws (see [`Self::draws`]), that its layer mask doesn't hide all of.
    fn pixels(&self, item: &Item) -> Result<(&Record<'_>, bool), String> {
        Ok(match item {
            Item::Layer(i) => {
                let r = self.record(*i)?;
                (r, !r.draws_nothing() && !self.clear(*i)? && !self.masked_out(*i, false)?)
            }
            Item::Group(g, children) => (self.record(*g)?, self.draws(children)? && !self.masked_out(*g, true)?),
        })
    }

    /// `items`' units, at `level`, with the layers in blend modes VectorCraft lacks, and in a group
    /// the layers with a shallow knockout, merged with those below.
    fn merged<'i>(&mut self, items: &'i [Item], keep_hidden: bool, level: Level) -> Result<Vec<Unit<'i>>, String> {
        #[cfg(test)]
        {
            let (walked, most) = WALKS.get();
            if walked >= most {
                return Err("the import walked more layer lists than the test allows".into());
            }
            WALKS.set((walked + 1, most));
        }
        let mut out: Vec<Unit<'i>> = vec![];
        for mut u in self.units(items, keep_hidden)? {
            // A hidden layer in a mode VectorCraft lacks would need the merge for its mode: it is left
            // out without being drawn.
            if !u.visible && matches!(u.blend, Blend::Other(_)) {
                self.skip(u.record);
                continue;
            }
            if self.knocks(&u, &out, level)? {
                let saved = self.saved();
                let noted = self.noted_so_far();
                let squeezed = self.squeezed;
                let outer = self.try_from_here();
                let knocked = self.knock(&out, &u)?;
                self.tried(outer, u.visible && matches!(knocked, Knocked::Merged(_)))?;
                match knocked {
                    // A hidden layer that would need the merge is left out, and nothing its drawing
                    // counted is counted.
                    Knocked::Merged(_) | Knocked::TooLarge if !u.visible => {
                        self.restore(saved)?;
                        self.unnote(noted);
                        self.skip(u.record);
                        continue;
                    }
                    Knocked::Merged(canvas) => {
                        // Adjustment layers kept below it in the group are merged with nothing to change.
                        for r in self.pending(&out, level)? {
                            self.unadjust(r);
                        }
                        self.take_merge(&mut out, &u, canvas);
                        if let Blend::Other(other) = u.blend
                            && !self.notes.merged.contains(&other.label())
                        {
                            self.notes.merged.push(other.label());
                        }
                        continue;
                    }
                    // Nothing below it, or nothing of its own, to knock out: it comes in as a layer,
                    // with the hidden layers its drawing left out (see `kept`). What its drawing
                    // read is read again when it comes in. A hidden one may still be left out by a
                    // later merge: what its drawing counted is counted when it comes in instead.
                    Knocked::Nothing if !u.visible => {
                        self.restore(saved)?;
                        self.unnote(noted);
                    }
                    // A merge below it that its held canvas left without the memory it needed may
                    // be made when they come in without it: what drawing them counted is given back
                    // and counted again then.
                    Knocked::Nothing if self.squeezed != squeezed => self.restore(saved)?,
                    Knocked::Nothing => self.refund(saved.spent)?,
                    Knocked::TooLarge => {
                        self.restore(saved)?;
                        self.squeezed += 1;
                        self.unknock(u.record);
                    }
                }
            }
            if self.adjustment(&u.body)?.is_some() {
                self.adjust_below(&mut out, u, level)?;
                continue;
            }
            let Blend::Other(other) = u.blend else {
                out.push(u);
                continue;
            };
            // A merge that can't be made counts nothing its groups would have counted.
            let saved = self.saved();
            let pending = self.pending(&out, level)?;
            let outer = self.try_from_here();
            let made = self.merge_into(&mut out, &u)?;
            self.tried(outer, made)?;
            if made {
                // Adjustment layers kept below it in the group are merged with nothing to change.
                for r in pending {
                    self.unadjust(r);
                }
                if !self.notes.merged.contains(&other.label()) {
                    self.notes.merged.push(other.label());
                }
            } else {
                self.restore(saved)?;
                self.squeezed += 1;
                self.notes.unmerged += 1;
                u.blend = Blend::Mode(BlendMode::Normal);
                out.push(u);
            }
        }
        Ok(out)
    }

    /// `out` replaced by one layer, the merge of its visible units with `top` (see
    /// [`Self::take_merge`]). `false` when the canvas would need more memory than allowed.
    fn merge_into<'i>(&mut self, out: &mut Vec<Unit<'i>>, top: &Unit<'_>) -> Result<bool, String> {
        match out.as_mut_slice() {
            // Onto the previous merge alone: composited onto its canvas, in place.
            [prev] if prev.is_merge() => {
                let mut acc = match &mut prev.body {
                    Body::Merged(c) => Some(std::mem::take(c)),
                    _ => None,
                };
                let made = self.composite(&mut acc, top);
                if let Body::Merged(c) = &mut prev.body {
                    *c = acc.unwrap_or_default();
                }
                if made? {
                    prev.name.push_str(" + ");
                    prev.name.push_str(&top.name);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            _ => match self.merge(out, top)? {
                Some(canvas) => {
                    self.take_merge(out, top, canvas);
                    Ok(true)
                }
                None => Ok(false),
            },
        }
    }

    /// Adjustment layer `u` (one the import applies), at `level`, merged with the visible units
    /// `out` below it in its list: they become one layer, adjusted. A hidden one would need the
    /// merge and is left out. With nothing visible below it that draws (other adjustment layers and
    /// layers and groups that draw nothing or are at 0 opacity don't count), or at 0 opacity, it
    /// changes nothing
    /// and comes in as nothing, except in a pass-through group, where it is kept to change what is
    /// below the group when the group is drawn onto it; there, once one is kept (in the group or in
    /// a pass-through group in it), the ones above it are kept too, in order. One that layers are clipped to, and one whose merge needs more memory
    /// than allowed, isn't applied, and is counted.
    fn adjust_below<'i>(&mut self, out: &mut Vec<Unit<'i>>, u: Unit<'i>, level: Level) -> Result<(), String> {
        if !u.visible {
            self.skip(u.record);
            return Ok(());
        }
        // One that layers are clipped to, kept as a layer because they drew nothing or their merge
        // didn't fit, isn't applied.
        if u.base {
            self.unadjust(u.record);
            return Ok(());
        }
        if u.opacity <= 0.0 {
            return Ok(());
        }
        // A unit that draws nothing has nothing to change.
        let mut below = false;
        for b in out.iter().filter(|b| b.visible && b.opacity > 0.0) {
            below = below || self.unit_draws(b)?;
        }
        // Kept below it in the group, or in a pass-through group below it there.
        let pending = !self.pending(out, level)?.is_empty();
        if level == Level::Pass && (pending || !below) {
            out.push(u);
            return Ok(());
        }
        if !below {
            return Ok(());
        }
        let saved = self.saved();
        let outer = self.try_from_here();
        let adjusting = self.adjusting.replace(u.record);
        let made = self.merge_into(out, &u);
        self.adjusting = adjusting;
        let made = made?;
        self.tried(outer, made)?;
        if !made {
            self.restore(saved)?;
            self.squeezed += 1;
            self.unadjust(u.record);
        } else if let Blend::Other(other) = u.blend
            && !self.notes.merged.contains(&other.label())
        {
            self.notes.merged.push(other.label());
        }
        Ok(())
    }

    /// The counts as they are, for [`Self::restore`].
    fn saved(&self) -> Counts {
        self.notes.counts(self.spent.get(), self.skip_log.len(), self.unknock_log.len(), self.blank_log.len(), self.unadjust_log.len())
    }

    /// The counts as they were before a merge that couldn't be made, the bytes it spent counted as
    /// wasted (see [`Self::refund`]).
    fn restore(&mut self, c: Counts) -> Result<(), String> {
        self.refund(c.spent)?;
        for i in self.skip_log.split_off(c.skipped) {
            if let Some(s) = self.skipped.get_mut(i) {
                *s = false;
                self.notes.hidden = self.notes.hidden.saturating_sub(1);
            }
        }
        for i in self.unknock_log.split_off(c.unknocked) {
            if let Some(s) = self.unknocked.get_mut(i) {
                *s = false;
                self.notes.knockouts = self.notes.knockouts.saturating_sub(1);
            }
        }
        // The groups found since: what their drawing counted is given back, so they are drawn again.
        for g in self.blank_log.split_off(c.blank) {
            if let Some(b) = self.blank.get_mut(g) {
                *b = None;
            }
            if let Some(b) = self.laid_blank.get_mut(g) {
                *b = false;
            }
        }
        for i in self.unclip_log.split_off(c.unclipped) {
            if let Some(s) = self.unclipped.get_mut(i) {
                *s = false;
                self.notes.unclipped = self.notes.unclipped.saturating_sub(1);
            }
        }
        for i in self.clipped_out_log.split_off(c.clipped_out) {
            if let Some(s) = self.clipped_out.get_mut(i) {
                *s = false;
                self.notes.clipped_out = self.notes.clipped_out.saturating_sub(1);
            }
        }
        for i in self.unadjust_log.split_off(c.unadjusted) {
            if let Some(s) = self.unadjusted.get_mut(i) {
                *s = false;
                self.notes.unadjusted = self.notes.unadjusted.saturating_sub(1);
            }
        }
        self.notes.merged.truncate(c.merged);
        self.notes.unmerged = c.unmerged;
        Ok(())
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

    /// `out` replaced by one layer: `canvas`, the merge of its visible units with `top`, named after
    /// them, bottom first. Its hidden units are left out.
    fn take_merge<'i>(&mut self, out: &mut Vec<Unit<'i>>, top: &Unit<'_>, canvas: Canvas) {
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
        name.push_str(&top.name);
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
            record: top.record,
            body: Body::Merged(canvas),
            clip: None,
            base: false,
        });
    }

    /// The visible units `below` with `top`, a layer with a shallow knockout, knocking them out: where
    /// `top` has pixels, what is below it in its group is cleared to transparency (the group's own
    /// backdrop) and `top` is drawn there at its fill opacity; its opacity then fades all of that
    /// back toward the units below as they were. Its layer mask shapes the cut.
    fn knock(&mut self, below: &[Unit<'_>], top: &Unit<'_>) -> Result<Knocked, String> {
        let (fill, opacity) = {
            let rec = self.record(top.record)?;
            (rec.fill, rec.opacity)
        };
        let k = match self.draw_body(&top.body, top.mask_of)? {
            Drawn::Nothing => return Ok(Knocked::Nothing),
            Drawn::TooLarge => return Ok(Knocked::TooLarge),
            Drawn::Canvas(c) if c.px.iter().all(|p| p[3] <= 0.0) => return Ok(Knocked::Nothing),
            Drawn::Canvas(c) => c,
        };
        // At 0 fill, beside every layer below it, it cuts nothing: no canvas for them is needed.
        if fill <= 0.0 && self.beside(below, &k)? {
            return Ok(Knocked::Nothing);
        }
        // The layer's canvas is held while the units below are drawn.
        let bytes = 16 * k.px.len() as u64;
        self.held = self.held.saturating_add(bytes);
        let mut acc: Option<Canvas> = None;
        let mut drawn = Ok(true);
        for u in below.iter().filter(|u| u.visible) {
            drawn = self.composite(&mut acc, u);
            if !matches!(drawn, Ok(true)) {
                break;
            }
        }
        self.held = self.held.saturating_sub(bytes);
        if !drawn? {
            return Ok(Knocked::TooLarge);
        }
        // Nothing below draws.
        let Some(below) = acc.as_ref() else { return Ok(Knocked::Nothing) };
        // At 0 fill it draws nothing of its own: where it cuts no painted pixel below, no canvas for
        // the merge is needed.
        if fill <= 0.0 && !cuts_paint(below, &k, top.blend, opacity, top.record) {
            return Ok(Knocked::Nothing);
        }
        let clear = |c: &Canvas| c.px.iter().all(|p| p[3] <= 0.0);
        let below_clear = clear(below);
        if !knock_onto(&mut acc, &k, top.blend, fill, opacity, top.record, self.room(), &|n| self.spend(n))? {
            return Ok(Knocked::TooLarge);
        }
        // Nothing below to cut, and nothing of its own drawn at its fill opacity.
        if below_clear && acc.as_ref().is_none_or(clear) {
            return Ok(Knocked::Nothing);
        }
        Ok(acc.map_or(Knocked::Nothing, Knocked::Merged))
    }

    /// Do the pixels `knock` paints lie outside the bounds of every visible unit `below`? Only the
    /// bounds of layers and of merged canvases are known without drawing them: with a group below,
    /// `false`.
    fn beside(&self, below: &[Unit<'_>], knock: &Canvas) -> Result<bool, String> {
        let width = knock.width.max(1);
        let mut painted: Option<[i64; 4]> = None;
        for (k, _) in knock.px.iter().enumerate().filter(|(_, p)| p[3] > 0.0) {
            let (x, y) = (knock.left + (k % width) as i64, knock.top + (k / width) as i64);
            painted = Some(painted.map_or([x, y, x + 1, y + 1], |[l, t, r, b]| [l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)]));
        }
        let Some([left, top, right, bottom]) = painted else { return Ok(true) };
        for u in below.iter().filter(|u| u.visible) {
            let [l, t, r, b] = match &u.body {
                Body::Merged(c) => [c.left, c.top, c.left + c.width as i64, c.top + c.height as i64],
                Body::Layer(i) => {
                    let rec = self.record(*i)?;
                    match rec.content {
                        Content::Adjustment(_) => continue,
                        Content::Solid(Some(_)) if rec.color_over_canvas() => [0, 0, i64::from(self.h.width), i64::from(self.h.height)],
                        _ => {
                            let (w, h) = rec.bounds.size();
                            let (l, t) = (i64::from(rec.bounds.left), i64::from(rec.bounds.top));
                            [l, t, l + w as i64, t + h as i64]
                        }
                    }
                }
                Body::Group(..) => return Ok(false),
            };
            if l < right && left < r && t < bottom && top < b {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The bytes a merge step may still take: the memory allowed, less the copies of what is below
    /// the pass-through groups being merged.
    fn room(&self) -> u64 {
        self.max_alloc.saturating_sub(self.held)
    }

    /// Note pass-through group `g` as laying nothing onto nothing when `acc`, nothing before, still
    /// is and no layer's pixels were read since `painted`: laying its layers onto nothing again
    /// gives nothing, with the same notes.
    fn laid_nothing(&mut self, g: usize, acc: &Option<Canvas>, painted: usize) {
        if acc.is_none()
            && self.painted.get() == painted
            && let Some(b) = self.laid_blank.get_mut(g)
            && !*b
        {
            *b = true;
            self.blank_log.push(g);
        }
    }

    /// Unit `u` composited onto `acc`. A pass-through group lays its layers onto `acc` one by one,
    /// so that they blend with what is below it; at less than full opacity or with a layer mask, the
    /// result is then faded back toward `acc` as it was by the opacity and the mask. Anything else
    /// is drawn alone first. `false` when the canvas would need more memory than allowed.
    fn composite(&mut self, acc: &mut Option<Canvas>, u: &Unit<'_>) -> Result<bool, String> {
        if let (Body::Group(g, items), Blend::Pass) = (&u.body, u.blend) {
            let mask = match u.mask_of {
                Some(i) if self.has_mask(Some(i))? => self.mask(i, false)?,
                _ => None,
            };
            // Onto nothing, a group that laid nothing onto nothing before lays nothing again.
            let (fresh, painted) = (acc.is_none(), self.painted.get());
            if fresh && self.laid_blank.get(*g) == Some(&true) {
                return Ok(true);
            }
            if u.opacity >= 1.0 && mask.is_none() {
                let laid = self.lay(acc, items, Level::Pass)?;
                if laid && fresh {
                    self.laid_nothing(*g, acc, painted);
                }
                return Ok(laid);
            }
            let bytes = acc.as_ref().map_or(0, |c| 16 * c.px.len() as u64);
            if bytes > self.room() {
                return Ok(false);
            }
            self.spend(bytes)?;
            let below = acc.clone();
            self.held = self.held.saturating_add(bytes);
            let laid = self.lay(acc, items, Level::Pass);
            self.held = self.held.saturating_sub(bytes);
            if !laid? {
                return Ok(false);
            }
            if fresh {
                self.laid_nothing(*g, acc, painted);
            }
            if let Some(after) = acc.as_mut() {
                fade(after, below.as_ref(), u.opacity, mask.as_ref());
            }
            return Ok(true);
        }
        // An adjustment layer changes `acc` where it is, through its layer mask.
        if let Some(a) = self.adjustment(&u.body)? {
            if let Some(c) = acc.as_mut() {
                let mask = match u.mask_of {
                    // A layer mask too large to read beside the canvas needs more memory than
                    // allowed: the adjustment isn't applied, and the rest of the merge goes on.
                    Some(i) if !self.mask_fits(i, 16 * c.px.len() as u64)? => {
                        // The adjustment's own merge isn't made then.
                        if self.adjusting == Some(u.record) {
                            return Ok(false);
                        }
                        self.squeezed += 1;
                        self.unadjust(u.record);
                        return Ok(true);
                    }
                    Some(i) => self.mask(i, false)?,
                    None => None,
                };
                adjust_onto(c, a, u.blend, u.opacity, mask.as_ref(), u.record);
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

    /// `items`' layers, the layers of a group at `level`, composited onto `acc` one by one; `false`
    /// when a canvas would need more memory than allowed.
    fn lay(&mut self, acc: &mut Option<Canvas>, items: &[Item], level: Level) -> Result<bool, String> {
        for c in self.merged(items, false, level)? {
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
            Body::Group(g, items) => {
                let drawn = match self.blank.get(*g).cloned().flatten() {
                    Some(d) => d,
                    None => {
                        let painted = self.painted.get();
                        let mut acc: Option<Canvas> = None;
                        if !self.lay(&mut acc, items, Level::Group)? {
                            return Ok(Drawn::TooLarge);
                        }
                        let d = acc.map_or(Drawn::Nothing, Drawn::Canvas);
                        if self.painted.get() == painted
                            && let Some(b) = self.blank.get_mut(*g)
                        {
                            *b = Some(d.clone());
                            self.blank_log.push(*g);
                        }
                        d
                    }
                };
                match drawn {
                    Drawn::Canvas(c) => c,
                    other => return Ok(other),
                }
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

    /// Layers of the document for `items`, the layers at `level`, bottom first.
    fn list(&mut self, items: &[Item], level: Level) -> Result<Vec<Arc<Node>>, String> {
        let mut out = vec![];
        for u in self.merged(items, self.hidden, level)? {
            if let Some(n) = self.node(u)? {
                out.push(Arc::new(n));
            }
        }
        Ok(out)
    }

    /// The layer of the document unit `u` becomes; `None` for a layer that draws nothing (an
    /// adjustment layer, or one without pixels).
    fn node(&mut self, u: Unit<'_>) -> Result<Option<Node>, String> {
        if let Some(k) = self.kept.get_mut(u.record) {
            *k = true;
        }
        // An adjustment layer kept in a pass-through group with nothing below it in the group: the
        // layers below the group aren't changed by it.
        if self.adjustment(&u.body)?.is_some() {
            self.unadjust(u.record);
            return Ok(None);
        }
        let children = match &u.body {
            Body::Layer(i) => match self.image(*i)? {
                Some((left, top, img)) => vec![Arc::new(self.image_node(&u.name, left, top, img)?)],
                None => return Ok(None),
            },
            Body::Merged(c) => match c.to_image() {
                Some((left, top, img)) => vec![Arc::new(self.image_node(&u.name, left, top, img)?)],
                None => return Ok(None),
            },
            Body::Group(_, items) => self.list(items, if u.blend == Blend::Pass { Level::Pass } else { Level::Group })?,
        };
        let mut layer = self.layer(&u.name);
        layer.visible = u.visible;
        layer.opacity = u.opacity.clamp(0.0, 1.0);
        (layer.blend, layer.isolate) = match u.blend {
            Blend::Mode(m) => (m, matches!(u.body, Body::Group(..))),
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

/// `c` changed by adjustment `a` (of record `record`): at each pixel, the adjusted color is
/// composited over the color as it was in `blend`, at `opacity` times the value of `mask` (as
/// [`fade`] takes it) there. The transparency of each pixel stays as it was.
fn adjust_onto(c: &mut Canvas, a: Adjust, blend: Blend, opacity: f32, mask: Option<&(i64, i64, image::GrayImage, u8)>, record: usize) {
    let width = c.width.max(1);
    let opacity = opacity.clamp(0.0, 1.0);
    for (k, p) in c.px.iter_mut().enumerate() {
        if p[3] <= 0.0 {
            continue;
        }
        let (x, y) = (c.left + (k % width) as i64, c.top + (k / width) as i64);
        let m = mask.map_or(255, |(left, top, m, default)| {
            let inside = (*left..*left + i64::from(m.width())).contains(&x) && (*top..*top + i64::from(m.height())).contains(&y);
            if inside { m.get_pixel((x - left) as u32, (y - top) as u32).0[0] } else { *default }
        });
        let t = coverage(blend, opacity * f32::from(m) / 255.0, x, y, record);
        if t <= 0.0 {
            continue;
        }
        let rgb = [p[0], p[1], p[2]];
        let [r, g, b] = a.apply(rgb);
        let mixed = over(blend, [rgb[0], rgb[1], rgb[2], 1.0], [r, g, b, t]);
        *p = [mixed[0], mixed[1], mixed[2], p[3]];
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
    if !cover(acc, src, max_alloc, spend)? {
        return Ok(false);
    }
    let Some(out) = acc.as_mut() else { return Ok(false) };
    // Only the source's rows and columns change.
    let (sw, x0, y0) = (src.width.max(1), (src.left - out.left) as usize, (src.top - out.top) as usize);
    for (k, s) in src.px.iter().enumerate() {
        let Some(p) = out.px.get_mut((y0 + k / sw) * out.width + x0 + k % sw) else { continue };
        let a = coverage(blend, s[3] * opacity, src.left + (k % sw) as i64, src.top + (k / sw) as i64, record);
        *p = over(blend, *p, [s[0], s[1], s[2], a]);
    }
    Ok(true)
}

/// `knock` (of record `record`, in `blend`, with its `fill` and `opacity`), a layer with a shallow
/// knockout, knocking out `acc`, on a canvas covering both, as [`blend_onto`] makes it. Where
/// `knock` has pixels, its transparency is the share of `acc` cleared and replaced by its color at
/// `fill`; `opacity` then fades that back toward `acc` (in Dissolve, each pixel is cleared fully
/// or not at all, at the share its transparency times `opacity` gives). `false`, with `acc` as it
/// was, when the canvases would take more than `max_alloc` bytes.
#[allow(clippy::too_many_arguments)]
fn knock_onto(
    acc: &mut Option<Canvas>,
    knock: &Canvas,
    blend: Blend,
    fill: f32,
    opacity: f32,
    record: usize,
    max_alloc: u64,
    spend: &dyn Fn(u64) -> Result<(), String>,
) -> Result<bool, String> {
    if !cover(acc, knock, max_alloc, spend)? {
        return Ok(false);
    }
    let Some(out) = acc.as_mut() else { return Ok(false) };
    let (fill, opacity) = (fill.clamp(0.0, 1.0), opacity.clamp(0.0, 1.0));
    let (kw, x0, y0) = (knock.width.max(1), (knock.left - out.left) as usize, (knock.top - out.top) as usize);
    for (k, s) in knock.px.iter().enumerate() {
        let Some(p) = out.px.get_mut((y0 + k / kw) * out.width + x0 + k % kw) else { continue };
        let t = coverage(blend, s[3].clamp(0.0, 1.0) * opacity, knock.left + (k % kw) as i64, knock.top + (k / kw) as i64, record);
        let below = p[3].clamp(0.0, 1.0);
        let alpha = (1.0 - t) * below + t * fill;
        if alpha <= 0.0 {
            *p = [0.0; 4];
            continue;
        }
        let mix = |i: usize| ((1.0 - t) * below * p[i] + t * fill * s[i]) / alpha;
        *p = [mix(0), mix(1), mix(2), alpha];
    }
    Ok(true)
}

/// Does `knock`, as [`knock_onto`] takes it, clear any of a pixel of `below` that has paint?
fn cuts_paint(below: &Canvas, knock: &Canvas, blend: Blend, opacity: f32, record: usize) -> bool {
    let (kw, opacity) = (knock.width.max(1), opacity.clamp(0.0, 1.0));
    knock.px.iter().enumerate().any(|(k, s)| {
        let (x, y) = (knock.left + (k % kw) as i64, knock.top + (k / kw) as i64);
        below.get(x, y).is_some_and(|b| b[3] > 0.0) && coverage(blend, s[3].clamp(0.0, 1.0) * opacity, x, y, record) > 0.0
    })
}

/// `acc` grown, when it doesn't already, to a canvas covering itself and `src` (a new one counted
/// with `spend`). `false`, with `acc` as it was, when the canvases held, `acc` and `src` and a
/// larger canvas when one is needed, would take more than `max_alloc` bytes.
fn cover(acc: &mut Option<Canvas>, src: &Canvas, max_alloc: u64, spend: &dyn Fn(u64) -> Result<(), String>) -> Result<bool, String> {
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
        tree: &items,
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
        noted_log: vec![],
        skipped: vec![false; records.len()],
        skip_log: vec![],
        unknocked: vec![false; records.len()],
        unknock_log: vec![],
        clipped_out: vec![false; records.len()],
        clipped_out_log: vec![],
        unclipped: vec![false; records.len()],
        unclip_log: vec![],
        unadjusted: vec![false; records.len()],
        unadjust_log: vec![],
        kept: vec![false; records.len()],
        spent: Cell::new(0),
        wasted: Cell::new(0),
        trial: Cell::new(0),
        painted: Cell::new(0),
        blank: vec![None; records.len()],
        laid_blank: vec![false; records.len()],
        blank_log: vec![],
        squeezed: 0,
        adjusting: None,
        clear: vec![Cell::new(None); records.len()],
        masked_out: vec![Cell::new(None); records.len()],
    };
    let layers = b.list(&items, Level::Top)?;
    // A drawing for a knockout or a clip leaves hidden layers out; those that came in anyway aren't.
    b.notes.hidden = b.skipped.iter().zip(&b.kept).filter(|&(s, k)| *s && !*k).count();
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

/// [`import`] within the memory allowed for one image, CMYK converted with the active color
/// settings: what `document.open` and `file.place` read a PSD or PSB file's layers with.
pub(crate) fn import_active(bytes: &[u8], hidden: bool) -> Result<Option<(Document, Vec<String>)>, String> {
    let cms = vectorcraft_color::cms::active();
    import(bytes, hidden, super::MAX_RASTER_ALLOC, |c| cms.cmyk_to_srgb(c, false))
}

/// What a PSD or PSB file holds (`document.psdInfo`, the PSD Import Options dialog).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsdInfo {
    /// The large document format.
    pub psb: bool,
    /// The canvas, in pixels.
    pub width: u32,
    pub height: u32,
    /// The layers, group markers left out.
    pub layers: usize,
    /// The layers among them that are hidden.
    pub hidden: usize,
    /// The layer groups.
    pub groups: usize,
    /// The slices that come in with `importSlices` (those generated automatically, and those
    /// outside the canvas, left out).
    pub slices: usize,
}

/// What PSD or PSB file `bytes` holds, read from its header, its slices and its layer records
/// without decoding any pixels. A file without layer records, or whose layer records can't be read, has no layers: it
/// opens and places as its merged image.
pub fn info(bytes: &[u8]) -> Result<PsdInfo, String> {
    let mut r = Reader::new(bytes);
    let h = header(&mut r)?;
    let records = layer_info(h.layers, h.psb).filter(|info| info.len() > 2).and_then(|info| records(info, h.psb).ok()).unwrap_or_default();
    let layers = records.iter().filter(|r| r.divider == 0);
    Ok(PsdInfo {
        psb: h.psb,
        width: h.width,
        height: h.height,
        layers: layers.clone().count(),
        hidden: layers.filter(|r| r.hidden).count(),
        groups: records.iter().filter(|r| matches!(r.divider, 1 | 2)).count(),
        slices: super::psdslices::count(bytes),
    })
}

const INFO: &str = "document.psdInfo";

/// `document.psdInfo`.
pub(super) fn psd_info(_: &mut Session, p: &Value) -> crate::cmd::Result<Value> {
    let bytes = super::source(p, INFO)?.bytes;
    let i = info(&bytes).map_err(super::load::err)?;
    Ok(json!({
        "format": if i.psb { "psb" } else { "psd" },
        "width": i.width,
        "height": i.height,
        "layers": i.layers,
        "hiddenLayers": i.hidden,
        "groups": i.groups,
        "slices": i.slices,
    }))
}
