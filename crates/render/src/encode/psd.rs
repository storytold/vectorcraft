//! PSD export: a layered bitmap at 8 bits per channel, written from the format's public
//! specification. The file holds the header, the resolution and colour profile resources, the
//! layer records and their channels (PackBits row by row; groups as section dividers), then the
//! merged image, which readers that skip layers show. RGB, CMYK (stored as 255 − ink, as the
//! format keeps inks) or grayscale.

use std::sync::Arc;

use vectorcraft_color::{BlendMode, cms};
use vectorcraft_doc::{Document, Knockout, LayerColor, Node, NodeKind};
use vectorcraft_geom::Rect;

use super::jpeg::{self, ColorModel};
use super::tiff::packbits;
use super::{RasterExportOptions, RasterFormat, on_white};
use crate::{RenderOptions, Rendered, Renderer, fx};

/// Most pixels a side of a PSD can have (larger images need the format's large-document variant).
pub const MAX_SIDE: u32 = 30_000;

/// The name of the record that closes a group (the format's convention).
const GROUP_END: &str = "</Layer group>";

/// The PSD options of a raster export.
#[derive(Clone, Debug, PartialEq)]
pub struct PsdOptions {
    pub color_model: ColorModel,
    /// Write Layers: each top-level layer becomes a pixel layer. Off: one flat image, on white where
    /// nothing is drawn.
    pub layers: bool,
    /// With `layers`: layers and sublayers become groups, and every object in them a pixel layer of
    /// its own, named as the Layers panel names it (text by its text). A layer whose look comes from
    /// its objects together (a clipping mask, an opacity mask, its own appearance, knockout) stays
    /// one pixel layer.
    pub max_editability: bool,
    /// With `layers`: hidden layers (and hidden objects) are written as hidden layers instead of
    /// being left out.
    pub hidden_layers: bool,
    /// Embed the colour profile of the colour model.
    pub embed_icc: bool,
}

impl Default for PsdOptions {
    fn default() -> Self {
        Self { color_model: ColorModel::Rgb, layers: true, max_editability: false, hidden_layers: false, embed_icc: true }
    }
}

/// The format's colour mode number.
fn mode(model: ColorModel) -> u16 {
    match model {
        ColorModel::Gray => 1,
        ColorModel::Rgb => 3,
        ColorModel::Cmyk => 4,
    }
}

/// The format's key for a blend mode.
fn blend_key(b: BlendMode) -> [u8; 4] {
    *match b {
        BlendMode::Normal => b"norm",
        BlendMode::Darken => b"dark",
        BlendMode::Multiply => b"mul ",
        BlendMode::ColorBurn => b"idiv",
        BlendMode::Lighten => b"lite",
        BlendMode::Screen => b"scrn",
        BlendMode::ColorDodge => b"div ",
        BlendMode::Overlay => b"over",
        BlendMode::SoftLight => b"sLit",
        BlendMode::HardLight => b"hLit",
        BlendMode::Difference => b"diff",
        BlendMode::Exclusion => b"smud",
        BlendMode::Hue => b"hue ",
        BlendMode::Saturation => b"sat ",
        BlendMode::Color => b"colr",
        BlendMode::Luminosity => b"lum ",
    }
}

/// An opacity 0–1 as a byte.
fn opacity_byte(o: f32) -> u8 {
    (o.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Pixels in `model`'s planes, then alpha when `alpha`. `px` is straight RGBA; `inks` the CMYK ink
/// amounts of the same pixels, as drawn on white paper (CMYK only). The merged image (`matte`) is
/// stored on white, as readers that skip alpha show it; layers keep their colours straight under
/// their alpha.
fn planes(model: ColorModel, px: &[u8], inks: Option<&[u8]>, matte: bool, alpha: bool) -> Vec<Vec<u8>> {
    let rgba = px.as_chunks::<4>().0;
    let rgb = |p: &[u8; 4]| if matte { on_white(p) } else { [p[0], p[1], p[2]] };
    let mut out: Vec<Vec<u8>> = match model {
        ColorModel::Rgb => (0..3).map(|c| rgba.iter().map(|p| rgb(p)[c]).collect()).collect(),
        ColorModel::Gray => {
            let gray = jpeg::to_gray();
            vec![rgba.iter().map(|p| gray(rgb(p))).collect()]
        }
        ColorModel::Cmyk => {
            let inks = inks.map_or(&[][..], |i| i.as_chunks::<4>().0);
            (0..4)
                .map(|c| {
                    inks.iter()
                        .zip(rgba)
                        .map(|(k, p)| {
                            // Inks are drawn premultiplied by coverage: a layer's are straightened.
                            let a = u32::from(p[3]);
                            let ink = if matte || a == 0 { k[c] } else { ((u32::from(k[c]) * 255 + a / 2) / a).min(255) as u8 };
                            255 - ink
                        })
                        .collect()
                })
                .collect()
        }
    };
    if alpha {
        out.push(rgba.iter().map(|p| p[3]).collect());
    }
    out
}

/// Rows of `plane` (`width` bytes each) PackBits-compressed one by one: their byte counts go to
/// `counts`, the rows to `data`.
fn rle(plane: &[u8], width: usize, counts: &mut Vec<u8>, data: &mut Vec<u8>) {
    for row in plane.chunks(width.max(1)) {
        let at = data.len();
        packbits(row, data);
        // A row of at most MAX_SIDE bytes packs into fewer than 65536.
        counts.extend(u16::try_from(data.len() - at).unwrap_or(u16::MAX).to_be_bytes());
    }
}

/// One layer record (records go bottom first).
struct Record {
    name: String,
    /// Top, left, bottom, right in canvas pixels.
    rect: [i32; 4],
    /// Channel id (−1: transparency) and its data, compression code first.
    channels: Vec<(i16, Vec<u8>)>,
    blend: [u8; 4],
    opacity: u8,
    hidden: bool,
    /// Section divider: [`OPEN_GROUP`] or [`END_GROUP`].
    divider: Option<u32>,
}

/// Section divider kinds.
const OPEN_GROUP: u32 = 1;
const END_GROUP: u32 = 3;

impl Record {
    /// A record without pixels: an empty layer, or a group's divider.
    fn empty(name: String, colors: usize) -> Self {
        Self {
            name,
            rect: [0; 4],
            // Raw (code 0) and no data.
            channels: std::iter::once(-1).chain(0..colors as i16).map(|id| (id, vec![0, 0])).collect(),
            blend: *b"norm",
            opacity: 255,
            hidden: false,
            divider: None,
        }
    }

    /// A pixel layer of `planes` (colours, then alpha), `width` pixels wide, at `left`, `top`.
    fn pixels(name: String, planes: &[Vec<u8>], width: u32, height: u32, left: u32, top: u32) -> Self {
        let Some((alpha, colors)) = planes.split_last() else { return Self::empty(name, 0) };
        let channels = std::iter::once((-1, alpha))
            .chain(colors.iter().enumerate().map(|(i, p)| (i as i16, p)))
            .map(|(id, plane)| {
                let (mut counts, mut data) = (vec![0, 1], vec![]);
                rle(plane, width as usize, &mut counts, &mut data);
                counts.extend(data);
                (id, counts)
            })
            .collect();
        let [l, t, w, h] = [left, top, width, height].map(|v| i32::try_from(v).unwrap_or(i32::MAX));
        Self { channels, rect: [t, l, t.saturating_add(h), l.saturating_add(w)], ..Self::empty(name, 0) }
    }

    /// A layer of one colour (`values`: a value per plane, colours then alpha) over the whole
    /// `width` × `height` canvas, without drawing its pixels first: one packed row, repeated.
    fn solid(name: String, values: &[u8], width: u32, height: u32) -> Self {
        let Some((alpha, colors)) = values.split_last() else { return Self::empty(name, 0) };
        let channels = std::iter::once((-1, *alpha))
            .chain(colors.iter().enumerate().map(|(i, v)| (i as i16, *v)))
            .map(|(id, v)| {
                let (mut count, mut row) = (vec![], vec![]);
                rle(&vec![v; width as usize], width as usize, &mut count, &mut row);
                let mut data = vec![0, 1];
                data.extend(count.repeat(height as usize));
                data.extend(row.repeat(height as usize));
                (id, data)
            })
            .collect();
        let [w, h] = [width, height].map(|v| i32::try_from(v).unwrap_or(i32::MAX));
        Self { channels, rect: [0, 0, h, w], ..Self::empty(name, 0) }
    }

    fn write(&self, out: &mut Vec<u8>) -> Result<(), String> {
        for v in self.rect {
            out.extend(v.to_be_bytes());
        }
        out.extend((self.channels.len() as u16).to_be_bytes());
        for (id, data) in &self.channels {
            out.extend(id.to_be_bytes());
            out.extend(len32(data.len())?.to_be_bytes());
        }
        out.extend(b"8BIM");
        out.extend(self.blend);
        out.push(self.opacity);
        // Clipping: base.
        out.push(0);
        // Flags: bit 1 hidden, bit 3 "bit 4 is set", bit 4 no pixels that matter (dividers).
        out.push(0x08 | if self.hidden { 0x02 } else { 0 } | if self.divider.is_some() { 0x10 } else { 0 });
        out.push(0);
        // Extra data: no layer mask, no blending ranges, the name, then tagged blocks.
        let mut extra = vec![0; 8];
        // The name as a Pascal string of its ASCII, padded to a multiple of 4 bytes; readers take
        // the Unicode name that follows.
        let ascii: Vec<u8> = self.name.chars().map(|c| if c.is_ascii() && !c.is_ascii_control() { c as u8 } else { b'_' }).take(255).collect();
        extra.push(ascii.len() as u8);
        extra.extend(ascii);
        extra.resize(extra.len().next_multiple_of(4), 0);
        // Cut before the character that would pass 255 units, never inside a surrogate pair.
        let mut units: Vec<u16> = vec![];
        for c in self.name.chars() {
            let mut buf = [0; 2];
            let encoded = c.encode_utf16(&mut buf);
            if units.len() + encoded.len() > 255 {
                break;
            }
            units.extend_from_slice(encoded);
        }
        let mut luni = (units.len() as u32).to_be_bytes().to_vec();
        luni.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        block(&mut extra, b"luni", &luni)?;
        if let Some(kind) = self.divider {
            block(&mut extra, b"lsct", &[&kind.to_be_bytes()[..], b"8BIM", &self.blend].concat())?;
        }
        out.extend(len32(extra.len())?.to_be_bytes());
        out.extend(extra);
        Ok(())
    }
}

fn too_large() -> String {
    "the image is too large for PSD (at most 4 GB): lower the resolution".into()
}

fn len32(n: usize) -> Result<u32, String> {
    u32::try_from(n).map_err(|_| too_large())
}

/// A tagged block of additional layer information, padded to an even length.
fn block(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) -> Result<(), String> {
    let len = data.len().next_multiple_of(2);
    out.extend(b"8BIM");
    out.extend(key);
    out.extend(len32(len)?.to_be_bytes());
    out.extend(data);
    out.resize(out.len() + len - data.len(), 0);
    Ok(())
}

/// An image resource block (no name), padded to an even length.
fn resource(out: &mut Vec<u8>, id: u16, data: &[u8]) -> Result<(), String> {
    out.extend(b"8BIM");
    out.extend(id.to_be_bytes());
    // An empty Pascal name, padded to an even length.
    out.extend([0, 0]);
    out.extend(len32(data.len())?.to_be_bytes());
    out.extend(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

/// Everything a PSD file holds.
struct File {
    width: u32,
    height: u32,
    model: ColorModel,
    /// The merged image's planes: colours, then alpha when `alpha`.
    merged: Vec<Vec<u8>>,
    alpha: bool,
    /// Bottom first.
    records: Vec<Record>,
    ppi: f64,
    icc: Option<Arc<[u8]>>,
}

/// Refuse a size the format can't store.
fn check_size(w: u32, h: u32) -> Result<(), String> {
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("{w} × {h} pixels is too large for PSD (at most {MAX_SIDE} pixels a side): lower the resolution"));
    }
    Ok(())
}

impl File {
    fn write(&self) -> Result<Vec<u8>, String> {
        let (w, h) = (self.width, self.height);
        check_size(w, h)?;
        let n = w as usize * h as usize;
        if n == 0 || self.merged.is_empty() || self.merged.iter().any(|p| p.len() != n) {
            return Err("PSD encoding failed: the pixel buffer doesn't match the image size".into());
        }
        let mut out = Vec::with_capacity(n * self.merged.len() / 2 + 1024);
        // Header: signature, version 1, reserved, channels, size, depth, colour mode.
        out.extend(b"8BPS");
        out.extend(1u16.to_be_bytes());
        out.extend([0; 6]);
        out.extend((self.merged.len() as u16).to_be_bytes());
        out.extend(h.to_be_bytes());
        out.extend(w.to_be_bytes());
        out.extend(8u16.to_be_bytes());
        out.extend(mode(self.model).to_be_bytes());
        // No colour mode data.
        out.extend(0u32.to_be_bytes());
        // Image resources: the resolution (16.16 fixed point, pixels per inch shown in inches), the profile.
        let ppi = if self.ppi.is_finite() && self.ppi > 0.0 { self.ppi } else { 72.0 };
        let fixed = (ppi.min(30_000.0) * 65536.0).round() as u32;
        let res = [fixed.to_be_bytes(), [0, 1, 0, 1], fixed.to_be_bytes(), [0, 1, 0, 1]].concat();
        let mut resources = vec![];
        resource(&mut resources, 1005, &res)?;
        if let Some(icc) = &self.icc {
            resource(&mut resources, 1039, icc)?;
        }
        out.extend(len32(resources.len())?.to_be_bytes());
        out.extend(resources);
        self.write_layers(&mut out)?;
        // The merged image: PackBits, every row's byte count first.
        out.extend(1u16.to_be_bytes());
        let (mut counts, mut data) = (Vec::with_capacity(self.merged.len() * h as usize * 2), vec![]);
        for p in &self.merged {
            rle(p, w as usize, &mut counts, &mut data);
        }
        out.extend(counts);
        out.extend(data);
        Ok(out)
    }

    /// The layer and mask information section.
    fn write_layers(&self, out: &mut Vec<u8>) -> Result<(), String> {
        if self.records.is_empty() {
            out.extend(0u32.to_be_bytes());
            return Ok(());
        }
        let count = i16::try_from(self.records.len()).map_err(|_| "too many layers for PSD (at most 32767)".to_string())?;
        // A negative count: the merged image's alpha is its transparency.
        let mut info = (if self.alpha { -count } else { count }).to_be_bytes().to_vec();
        for r in &self.records {
            r.write(&mut info)?;
        }
        for (_, data) in self.records.iter().flat_map(|r| &r.channels) {
            info.extend(data);
        }
        info.resize(info.len().next_multiple_of(2), 0);
        let len = len32(info.len())?;
        // The section: the layer info's length and the info, then no global layer mask.
        out.extend(len.checked_add(8).ok_or_else(too_large)?.to_be_bytes());
        out.extend(len.to_be_bytes());
        out.extend(info);
        out.extend(0u32.to_be_bytes());
        Ok(())
    }
}

fn icc(o: &PsdOptions) -> Result<Option<Arc<[u8]>>, String> {
    o.embed_icc.then(|| cms::icc_bytes(&o.color_model.profile()).map_err(|e| format!("PSD colour profile: {e}"))).transpose()
}

/// `img` as a flat PSD: the merged image alone, on white where it is transparent (CMYK separated
/// from the screen colours; [`export`] draws inks).
pub fn encode(img: &Rendered, ppi: f64, o: &PsdOptions) -> Result<Vec<u8>, String> {
    let inks = (o.color_model == ColorModel::Cmyk).then(|| jpeg::separated(img));
    let merged = planes(o.color_model, &img.to_straight(), inks.as_deref(), true, false);
    File { width: img.width, height: img.height, model: o.color_model, merged, alpha: false, records: vec![], ppi, icc: icc(o)? }.write()
}

/// `region` of `doc` as a PSD with `opts` (see [`PsdOptions`]). The merged image is the export's
/// render; with Write Layers each part is also drawn alone as its own layer, cropped to what it
/// paints, its opacity and blend mode in the layer's record.
pub(crate) fn export(r: &mut Renderer, doc: &Document, region: Rect, opts: &RasterExportOptions) -> Result<Vec<u8>, String> {
    let o = &opts.psd;
    let scale = opts.scale();
    let (width, height) = crate::region_pixels(region, scale);
    check_size(width, height)?;
    let ro = opts.render_options(RasterFormat::Psd);
    let (mut merged, mut alpha) = {
        let px = r.render_region_with(doc, region, scale, &ro).to_straight();
        let inks = (o.color_model == ColorModel::Cmyk).then(|| r.render_region_inks(doc, region, scale, &ro));
        let alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] < 255);
        (planes(o.color_model, &px, inks.as_deref(), true, alpha), alpha)
    };
    let mut records = vec![];
    if o.layers {
        let mut base = doc.clone();
        base.layers.clear();
        let mut parts = Parts { r, base, region, scale, width, height, ro: RenderOptions { background: None, ..ro }, o, records: vec![] };
        if let Some(bg) = opts.background {
            parts.background(bg);
        }
        for l in &doc.layers {
            parts.add(l);
        }
        records = parts.records;
    }
    // Without layers an alpha plane would be read as an extra channel: the colours, on white, stay.
    if records.is_empty() && alpha {
        merged.pop();
        alpha = false;
    }
    File { width, height, model: o.color_model, merged, alpha, records, ppi: opts.ppi, icc: icc(o)? }.write()
}

/// Draws the parts of a document as layer records, bottom first.
struct Parts<'a> {
    r: &'a mut Renderer,
    /// The document without its layers: each part is drawn on it alone.
    base: Document,
    region: Rect,
    scale: f64,
    width: u32,
    height: u32,
    /// Transparent: a part's pixels alone.
    ro: RenderOptions,
    o: &'a PsdOptions,
    records: Vec<Record>,
}

impl Parts<'_> {
    fn colors(&self) -> usize {
        self.o.color_model.channels()
    }

    /// The export's background colour as the bottom layer.
    fn background(&mut self, rgb: [u8; 3]) {
        let px = [rgb[0], rgb[1], rgb[2], 255];
        let inks = (self.o.color_model == ColorModel::Cmyk).then(|| jpeg::separated(&Rendered { width: 1, height: 1, pixels: px.to_vec() }));
        let values: Vec<u8> = planes(self.o.color_model, &px, inks.as_deref(), false, true).iter().map(|p| p.first().copied().unwrap_or(0)).collect();
        self.records.push(Record::solid("Background".into(), &values, self.width, self.height));
    }

    /// Node `n` (a top-level layer, or with Maximum Editability anything in one) and its parts.
    fn add(&mut self, n: &Node) {
        // Templates and guides aren't artwork.
        if n.is_template() || n.is_guide() || !(n.visible || self.o.hidden_layers) {
            return;
        }
        let (hidden, opacity) = (!n.visible, opacity_byte(n.opacity));
        if self.o.max_editability
            && splits(n)
            && let Some(children) = n.children()
        {
            self.records.push(Record { divider: Some(END_GROUP), ..Record::empty(GROUP_END.into(), self.colors()) });
            for c in children {
                self.add(c);
            }
            // A group in normal blending lets its layers blend with what is below it.
            let blend = if n.blend == BlendMode::Normal && !n.isolate { *b"pass" } else { blend_key(n.blend) };
            self.records.push(Record { divider: Some(OPEN_GROUP), blend, opacity, hidden, ..Record::empty(n.display_name(), self.colors()) });
            return;
        }
        let record = self.draw(n);
        self.records.push(Record { blend: blend_key(n.blend), opacity, hidden, ..record });
    }

    /// `n` drawn alone, visible, at full opacity in normal blending (its record carries those).
    fn draw(&mut self, n: &Node) -> Record {
        let name = n.display_name();
        let mut part = n.clone();
        (part.visible, part.opacity, part.blend) = (true, 1.0, BlendMode::Normal);
        let layer = if part.is_layer() {
            part
        } else {
            let mut l = Node::layer(self.base.alloc_id(), "", LayerColor::Preset(0));
            if let Some(children) = l.children_mut() {
                children.push(Arc::new(part));
            }
            l
        };
        // What the part paints, in canvas pixels (all of it when effects may reach further than the
        // art's bounds say).
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        let bounds = if reaches_beyond(&layer) { Some(Rect::new(0.0, 0.0, w, h)) } else { None };
        self.base.layers = vec![Arc::new(layer)];
        let (region, s) = (self.region, self.scale);
        let bounds = bounds.or_else(|| {
            let b = super::art_bounds(&self.base)?;
            // A pixel's margin for anti-aliasing.
            Some(Rect::new(
                ((b.x0 - region.x0) * s).floor() - 1.0,
                ((b.y0 - region.y0) * s).floor() - 1.0,
                ((b.x1 - region.x0) * s).ceil() + 1.0,
                ((b.y1 - region.y0) * s).ceil() + 1.0,
            ))
        });
        let colors = self.colors();
        let Some(b) = bounds.filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())) else { return Record::empty(name, colors) };
        let (x0, y0, x1, y1) = (b.x0.clamp(0.0, w), b.y0.clamp(0.0, h), b.x1.clamp(0.0, w), b.y1.clamp(0.0, h));
        if x1 <= x0 || y1 <= y0 {
            return Record::empty(name, colors);
        }
        let sub = Rect::new(region.x0 + x0 / s, region.y0 + y0 / s, region.x0 + x1 / s, region.y0 + y1 / s);
        let img = self.r.render_region_with(&self.base, sub, s, &self.ro);
        let inks = (self.o.color_model == ColorModel::Cmyk).then(|| self.r.render_region_inks(&self.base, sub, s, &self.ro));
        let (left, top) = (x0 as u32, y0 as u32);
        // Within the canvas, cropped to the pixels it paints.
        let (iw, ih) = (img.width.min(self.width - left) as usize, img.height.min(self.height - top) as usize);
        let px = img.to_straight();
        let painted = |x: usize, y: usize| px.get((y * img.width as usize + x) * 4 + 3).is_some_and(|a| *a > 0);
        let rows: Vec<usize> = (0..ih).filter(|y| (0..iw).any(|x| painted(x, *y))).collect();
        let (Some(&cy0), Some(&cy1)) = (rows.first(), rows.last()) else { return Record::empty(name, colors) };
        let cols: Vec<usize> = (0..iw).filter(|x| (cy0..=cy1).any(|y| painted(*x, y))).collect();
        let (Some(&cx0), Some(&cx1)) = (cols.first(), cols.last()) else { return Record::empty(name, colors) };
        let crop = |buf: &[u8]| -> Vec<u8> {
            (cy0..=cy1)
                .flat_map(|y| buf.get((y * img.width as usize + cx0) * 4..(y * img.width as usize + cx1 + 1) * 4).unwrap_or_default())
                .copied()
                .collect()
        };
        let planes = planes(self.o.color_model, &crop(&px), inks.as_deref().map(crop).as_deref(), false, true);
        Record::pixels(name, &planes, (cx1 - cx0 + 1) as u32, (cy1 - cy0 + 1) as u32, left + cx0 as u32, top + cy0 as u32)
    }
}

/// Can layer `n` be written as a group of its parts without changing its look?
fn splits(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Layer { clip: false, .. }) && n.extras().mask.is_none() && n.knockout != Knockout::On && !fx::has_object_fx(n)
}

/// Does anything in `n` carry effects of its own that may paint outside the art's bounds?
fn reaches_beyond(n: &Node) -> bool {
    fx::has_object_fx(n) || n.children().is_some_and(|c| c.iter().any(|c| reaches_beyond(c)))
}
