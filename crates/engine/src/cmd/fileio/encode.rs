//! One encoder per writable format. Each format's options are parsed, typed, from the export params
//! (unknown keys are ignored, so one params object can carry the options of several formats).

use std::borrow::Cow;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use vectorcraft_doc::{Artboard, Document};
use vectorcraft_geom::Rect;
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::jpeg::{self, JpegOptions};
use vectorcraft_render::encode::quantize::{Dither, PaletteOptions, Reduction};
use vectorcraft_render::encode::tiff::{ByteOrder, TiffOptions};
use vectorcraft_render::encode::{RasterExportOptions, RasterFormat, bmp, psd, tga};

use super::super::*;
use super::Format;
use super::imagemap::{self, MapKind};

const C: &str = "document.export";

/// The params that pick artboards (the fields of [`ArtboardPick`]).
pub const ARTBOARD_PARAMS: [&str; 3] = ["artboard", "artboards", "range"];

/// Which artboards an export covers: `range` (`"1-3, 5"`, 1-based, or `"all"`) wins over
/// `artboards` (0-based), which wins over `artboard`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ArtboardPick {
    pub artboard: Option<usize>,
    pub artboards: Option<Vec<usize>>,
    pub range: Option<String>,
}

impl ArtboardPick {
    /// The named artboards (`None`: none named), checked against the document's `count`.
    pub fn resolve(&self, count: usize) -> std::result::Result<Option<Vec<usize>>, String> {
        let v = match (&self.range, &self.artboards, self.artboard) {
            (Some(r), _, _) if r.trim().eq_ignore_ascii_case("all") => (0..count).collect(),
            (Some(r), _, _) => parse_range(r, count)?,
            (None, Some(a), _) => a.clone(),
            (None, None, Some(a)) => vec![a],
            (None, None, None) => return Ok(None),
        };
        if v.is_empty() {
            return Err("no artboards named".into());
        }
        match v.iter().find(|i| **i >= count) {
            Some(i) => Err(format!("no artboard {i} (0-based; the document has {count})")),
            None => Ok(Some(v)),
        }
    }

    /// The one artboard a single-image format writes (default: the first).
    pub fn one(&self, count: usize) -> std::result::Result<usize, String> {
        match self.resolve(count)?.as_deref() {
            None if count > 0 => Ok(0),
            None => Err("the document has no artboard".into()),
            Some([i]) => Ok(*i),
            Some(_) => Err("this format holds one artboard: export several as PDF or with document.exportForScreens".into()),
        }
    }
}

/// What an export writes: its files (one, or one per artboard for an SVG of several artboards)
/// and the image files an SVG links to.
#[derive(Default)]
pub struct Encoded {
    /// `(artboard, bytes)`; the artboard names the file when there are several.
    pub files: Vec<(Option<usize>, Vec<u8>)>,
    /// Images to write next to the file(s) (SVG `images: "link"`).
    pub linked: Vec<vectorcraft_svg::LinkedImage>,
    /// Image maps, each written beside its image under the image's name (raster `imageMap`).
    pub maps: Vec<imagemap::Map>,
    /// The encoder's warnings (features approximated or left out, options not applied yet).
    pub warnings: Vec<String>,
    /// What joins the file's stem and the artboard name when there is a file per artboard
    /// (default `-`; EPS uses `_`, as the reference app names them).
    pub joiner: Option<&'static str>,
}

impl Encoded {
    pub(super) fn one(bytes: Vec<u8>) -> Self {
        Self { files: vec![(None, bytes)], ..Self::default() }
    }

    /// Every file to write for the destination `path` (a path or a file name): the file itself
    /// when there is one, else `{stem}-{artboard name}.{ext}` beside it (see `joiner`; a file of no artboard
    /// among several, as Save's master file, is the file itself), then the linked images beside it
    /// under their own names, then the image maps as `{image stem}.html` or `.map`.
    pub fn named<'a>(&'a self, doc: &Document, path: &str) -> Vec<(String, Cow<'a, [u8]>)> {
        // Siblings keep the path's own separators (this runs on every platform and on the web).
        let dir = &path[..path.rfind(['/', '\\']).map_or(0, |i| i + 1)];
        let sibling = |name: &str| format!("{dir}{name}");
        let mut out: Vec<(String, Cow<[u8]>)> = match self.files.as_slice() {
            [(_, bytes)] => vec![(path.to_string(), Cow::Borrowed(bytes.as_slice()))],
            files => {
                let boards: Vec<usize> = files.iter().filter_map(|(b, _)| *b).collect();
                let file = std::path::Path::new(&path[dir.len()..]);
                let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                let mut names = super::artboard_file_names(doc, &boards).into_iter();
                files
                    .iter()
                    .map(|(b, bytes)| {
                        let name = match b {
                            None => path.to_string(),
                            Some(_) => sibling(&format!("{stem}{}{}{ext}", self.joiner.unwrap_or("-"), names.next().unwrap_or_default())),
                        };
                        (name, Cow::Borrowed(bytes.as_slice()))
                    })
                    .collect()
            }
        };
        let maps: Vec<(String, Cow<[u8]>)> = self
            .maps
            .iter()
            .filter_map(|m| {
                let image = std::path::Path::new(&out.get(m.file)?.0);
                let name = image.file_name()?.to_string_lossy();
                Some((image.with_extension(m.kind.ext()).to_string_lossy().into_owned(), Cow::Owned(m.text(&name).into_bytes())))
            })
            .collect();
        out.extend(self.linked.iter().map(|l| (sibling(&l.name), Cow::Borrowed(l.bytes.as_slice()))));
        out.extend(maps);
        out
    }

    /// The one file and the warnings, when the export wrote nothing else.
    fn single(self, f: &Format) -> Result<(Vec<u8>, Vec<String>)> {
        if !self.linked.is_empty() || !self.maps.is_empty() {
            return Err(bad(C, format!("{} with linked images or an image map writes several files: use document.export", f.label)));
        }
        match <[_; 1]>::try_from(self.files) {
            Ok([(_, bytes)]) => Ok((bytes, self.warnings)),
            Err(_) => Err(bad(C, format!("{} writes one file per artboard here: use document.export, or name one artboard", f.label))),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct RasterOptions {
    #[serde(flatten)]
    boards: ArtboardPick,
    scale: Option<f64>,
    /// Pixels per inch (wins over `scale`).
    ppi: Option<f64>,
    quality: Option<u8>,
    background: Option<Value>,
    anti_alias: Option<String>,
    interlaced: Option<bool>,
    color_model: Option<String>,
    method: Option<String>,
    scans: Option<u8>,
    embed_icc: Option<bool>,
    image_map: Option<String>,
    colors: Option<u16>,
    reduction: Option<String>,
    dither: Option<String>,
    dither_amount: Option<u8>,
    transparency: Option<bool>,
    matte: Option<Value>,
    lossless: Option<bool>,
    lzw: Option<bool>,
    byte_order: Option<String>,
    /// BMP and Targa bits per pixel.
    depth: Option<u8>,
    /// BMP layout: `windows` or `os2`.
    file_format: Option<String>,
    rle: Option<bool>,
    flip_rows: Option<bool>,
    /// PSD: Write Layers.
    layers: Option<bool>,
    max_editability: Option<bool>,
    hidden_layers: Option<bool>,
}

impl RasterOptions {
    /// The render and encoder settings: `ppi`, else 72 × `scale`, within 0.01–64 pixels per point;
    /// `background`, else `page` (the document's background).
    fn settings(&self, page: Option<[u8; 3]>) -> Result<RasterExportOptions> {
        if let Some(ppi) = self.ppi.filter(|p| !(p.is_finite() && *p > 0.0)) {
            return Err(bad(C, format!("ppi must be a positive number, not {ppi}")));
        }
        let ppi = self.ppi.unwrap_or(72.0 * self.scale.unwrap_or(1.0));
        let color_model = parse(self.color_model.as_deref(), jpeg::ColorModel::from_id, "colorModel", "rgb, cmyk or gray")?;
        let embed_icc = self.embed_icc.unwrap_or(true);
        Ok(RasterExportOptions {
            ppi: (ppi / 72.0).clamp(0.01, 64.0) * 72.0,
            background: match &self.background {
                Some(v) => background(v).map_err(|e| bad(C, e))?,
                None => page,
            },
            anti_alias: self.anti_alias.as_deref().map(anti_alias).transpose().map_err(|e| bad(C, e))?.unwrap_or_default(),
            interlaced: self.interlaced.unwrap_or(false),
            quality: self.quality.unwrap_or(90).min(100),
            jpeg: JpegOptions {
                color_model,
                method: parse(self.method.as_deref(), jpeg::Method::from_id, "method", "baseline, optimized or progressive")?,
                scans: self.scans.unwrap_or(3).clamp(*jpeg::SCANS.start(), *jpeg::SCANS.end()),
                embed_icc,
            },
            palette: PaletteOptions {
                colors: self.colors.unwrap_or(256).clamp(2, 256),
                reduction: parse(
                    self.reduction.as_deref(),
                    Reduction::from_id,
                    "reduction",
                    "perceptual, selective, adaptive, web, blackWhite or gray",
                )?,
                dither: parse(self.dither.as_deref(), Dither::from_id, "dither", "none, diffusion, pattern or noise")?,
                dither_amount: self.dither_amount.unwrap_or(100).min(100),
                transparency: self.transparency.unwrap_or(true),
                matte: match &self.matte {
                    Some(v) => background(v).map_err(|e| bad(C, format!("matte: {e}")))?,
                    None => Some([255; 3]),
                },
            },
            tiff: TiffOptions {
                color_model,
                lzw: self.lzw.unwrap_or(true),
                byte_order: parse(self.byte_order.as_deref(), ByteOrder::from_id, "byteOrder", "little or big")?,
                embed_icc,
            },
            bmp: bmp::BmpOptions {
                os2: parse(self.file_format.as_deref(), bmp_layout, "fileFormat", "windows or os2")?,
                depth: self.depth.unwrap_or(24),
                rle: self.rle.unwrap_or(false),
                top_down: self.flip_rows.unwrap_or(false),
                gray: color_model == jpeg::ColorModel::Gray,
            },
            tga: tga::TgaOptions { depth: self.depth.unwrap_or(24) },
            psd: psd::PsdOptions {
                color_model,
                layers: self.layers.unwrap_or(true),
                max_editability: self.max_editability.unwrap_or(false),
                hidden_layers: self.hidden_layers.unwrap_or(false),
                embed_icc,
            },
        })
    }
}

/// The BMP layout by name: `true` for OS/2.
fn bmp_layout(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "windows" => Some(false),
        "os2" | "os/2" => Some(true),
        _ => None,
    }
}

/// Refuse options `format` can't write together: BMP and Targa depths and layouts, and colour
/// models (BMP is RGB or grey, Targa RGB).
fn check_options(format: RasterFormat, o: &RasterExportOptions) -> Result<()> {
    let model = o.jpeg.color_model;
    let r = match format {
        RasterFormat::Bmp if model == jpeg::ColorModel::Cmyk => Err("BMP files are RGB or grayscale: colorModel rgb or gray".to_string()),
        RasterFormat::Tga if model != jpeg::ColorModel::Rgb => Err("Targa files are RGB: colorModel rgb".to_string()),
        RasterFormat::Bmp => o.bmp.check(),
        RasterFormat::Tga => o.tga.check(),
        _ => Ok(()),
    };
    r.map_err(|e| bad(C, e))
}

/// An enum option by id (its default when absent).
fn parse<T: Default>(v: Option<&str>, from_id: fn(&str) -> Option<T>, name: &str, ids: &str) -> Result<T> {
    v.map(|s| from_id(s).ok_or_else(|| bad(C, format!("{name} `{s}`: {ids}")))).transpose().map(Option::unwrap_or_default)
}

/// A raster background: `transparent` (also `none` or null), `white`, `black` or a colour
/// (`"#rrggbb"`, `[r, g, b]` 0–1, `{c, m, y, k}`, `{gray}`) → opaque RGB, `None` = transparent.
pub(crate) fn background(v: &Value) -> std::result::Result<Option<[u8; 3]>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.eq_ignore_ascii_case("transparent") || s.eq_ignore_ascii_case("none") => Ok(None),
        Value::String(s) if s.eq_ignore_ascii_case("white") => Ok(Some([255; 3])),
        Value::String(s) if s.eq_ignore_ascii_case("black") => Ok(Some([0; 3])),
        other => color_value(other)
            .map(|c| {
                let [r, g, b, _] = c.to_rgba8(1.0);
                Some([r, g, b])
            })
            .ok_or_else(|| format!("background {other}: transparent, white, black or a colour such as \"#ff8800\"")),
    }
}

/// An anti-aliasing mode by id: `none`, `art` or `type`.
pub(crate) fn anti_alias(id: &str) -> std::result::Result<AntiAlias, String> {
    AntiAlias::from_id(id).ok_or_else(|| format!("antiAlias `{id}`: none, art or type"))
}

/// The `useArtboards` param of a PDF, AI, DXF or raster export (SVG has its own): `true` writes every
/// chosen artboard (default all), `false` the bounds of the visible art instead, absent one
/// artboard (raster, DXF) or the chosen pages (PDF, AI).
fn use_artboards(p: &Value) -> Result<Option<bool>> {
    match p.get("useArtboards") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(v) => Err(bad(C, format!("useArtboards must be true or false, not {v}"))),
    }
}

/// A copy of `doc` with one artboard, `rect` (exports of the art's or the selection's bounds).
pub(super) fn single_artboard(doc: &Document, rect: Rect, name: &str) -> Document {
    with_single_artboard(doc.clone(), rect, name)
}

/// `d` with one artboard, `rect`.
pub(crate) fn with_single_artboard(mut d: Document, rect: Rect, name: &str) -> Document {
    d.artboards = vec![Artboard { id: 1, name: name.into(), rect, show_center_mark: false, show_cross_hairs: false }];
    d
}

fn boards<T>(r: std::result::Result<T, String>) -> Result<T> {
    r.map_err(|e| bad(C, e))
}

pub(super) fn options<T: DeserializeOwned + Default>(f: &Format, p: &Value) -> Result<T> {
    if !p.is_object() {
        return Ok(T::default());
    }
    T::deserialize(p).map_err(|e| bad(C, format!("{} options: {e}", f.label)))
}

/// Most pixels a side of a WebP image can have (its sizes are stored in 14 bits).
const WEBP_SIDE: f64 = 16383.0;

/// Refuse a raster export its format can't store (instead of writing an empty file). The
/// renderer's own size limits are [`vectorcraft_render::raster_size`]'s.
fn check_format_size(f: &Format, w: f64, h: f64) -> Result<()> {
    let max = match f.id {
        "webp" => WEBP_SIDE,
        "psd" => f64::from(psd::MAX_SIDE),
        _ => return Ok(()),
    };
    if w.round() > max || h.round() > max {
        return Err(bad(
            C,
            format!("{} × {} pixels is too large for {} (at most {max} pixels a side): lower the scale", w.round(), h.round(), f.label),
        ));
    }
    Ok(())
}

/// Encode `doc` as `format` (an id or extension from [`super::FORMATS`]) with that format's
/// options from `p` (see `document.formats`) into one file. Raster formats leave template layers
/// out, and no format writes the opacity-mask editing layer. Exports that write several files (an
/// SVG per artboard, linked images) go through [`encode_all`].
pub fn encode(doc: &Document, format: &str, p: &Value) -> Result<Vec<u8>> {
    encode_with_warnings(doc, format, p).map(|(bytes, _)| bytes)
}

/// Like [`encode`], also returning the encoder's warnings (PDF and SVG: options not applied yet,
/// features approximated or left out; every format that draws type: fonts written in the fallback
/// font).
pub fn encode_with_warnings(doc: &Document, format: &str, p: &Value) -> Result<(Vec<u8>, Vec<String>)> {
    let f = super::writable(C, Some(format), None)?;
    encode_all(doc, f.id, p)?.single(f)
}

/// [`encode`], with every file the export writes.
pub fn encode_all(doc: &Document, format: &str, p: &Value) -> Result<Encoded> {
    let format = super::writable(C, Some(format), None)?.id;
    // Formats with editing data prepare their drawn output separately from their original
    // document. Other formats only need the placed files' art.
    if !matches!(format, "vectorcraft" | "template" | "ai" | "pdf" | "eps" | "svg" | "svgz") {
        let (full, warnings) = crate::cmd::place::document::full_documents(doc);
        if !warnings.is_empty() {
            let mut enc = encode_all_of(&full, format, p)?;
            enc.warnings.extend(warnings);
            return Ok(enc);
        }
        return encode_all_of(&full, format, p);
    }
    encode_all_of(doc, format, p)
}

/// [`encode_all`] of `doc` as it is.
fn encode_all_of(doc: &Document, format: &str, p: &Value) -> Result<Encoded> {
    let f = super::writable(C, Some(format), None)?;
    let mut enc = encode_files(doc, f, p)?;
    // Formats that draw type: missing fonts come out in the fallback font (PDF and SVG say so
    // themselves: SVG only when it outlines or embeds fonts).
    if (f.raster || matches!(f.id, "eps" | "emf" | "wmf"))
        && let Some(w) = crate::cmd::fonts::substitution_warning(doc)
    {
        enc.warnings.push(w);
    }
    Ok(enc)
}

/// [`encode_all`] without the font substitution warning.
fn encode_files(doc: &Document, f: &Format, p: &Value) -> Result<Encoded> {
    let doc = &*doc.without_edit_modes();
    let n = doc.artboards.len();
    // SVG reads its own `useArtboards` (an SVG option).
    let use_artboards = if f.raster || matches!(f.id, "pdf" | "ai" | "dxf" | "emf" | "wmf") { use_artboards(p)? } else { None };
    if use_artboards == Some(false) {
        let bounds = vectorcraft_render::encode::art_bounds(doc).ok_or_else(|| bad(C, "nothing to export: the document has no visible art"))?;
        let mut q = super::export::without_artboards(p);
        if let Some(o) = q.as_object_mut() {
            o.remove("useArtboards");
        }
        return encode_files(&single_artboard(doc, bounds, "Art"), f, &q);
    }
    let bytes = match f.id {
        "vectorcraft" => super::native::encode(C, f, doc, p)?,
        // A native file flagged so opening it starts a new untitled document.
        "template" => {
            let mut d = doc.clone();
            d.template = true;
            super::native::encode(C, f, &d, p)?
        }
        "txt" => super::text::encode(doc, p)?,
        "svg" | "svgz" => return super::svg::encode(doc, p, f.id == "svgz").map_err(|e| bad(C, e)),
        "dxf" => return super::dxf::encode(doc, p, use_artboards),
        // EPS reads its own `useArtboards` (the art's bounds unless asked).
        "eps" => return super::eps::encode(doc, p),
        "emf" => return super::metafile::encode(doc, p, use_artboards, vectorcraft_metafile::Kind::Emf),
        "wmf" => return super::metafile::encode(doc, p, use_artboards, vectorcraft_metafile::Kind::Wmf),
        "ai" => return super::save::encode_ai(C, f, doc, p),
        "pdf" => {
            let (bytes, warnings) = super::pdf::encode(C, doc, p)?;
            return Ok(Encoded { warnings, ..Encoded::one(bytes) });
        }
        "png" | "jpg" | "webp" | "gif" | "png8" | "tiff" | "bmp" | "tga" | "psd" => {
            let o: RasterOptions = options(f, p)?;
            // New Document → Background Contents: White makes the export opaque, unless `background`
            // says otherwise (JPEG has no alpha: white either way).
            let page = (doc.setup.background == vectorcraft_doc::Background::White).then_some([255; 3]);
            let settings = o.settings(page)?;
            let scale = settings.scale();
            let format = match f.id {
                "png" => RasterFormat::Png,
                "jpg" => RasterFormat::Jpeg,
                "gif" => RasterFormat::Gif,
                "png8" => RasterFormat::Png8,
                "tiff" => RasterFormat::Tiff,
                "bmp" => RasterFormat::Bmp,
                "tga" => RasterFormat::Tga,
                "psd" => RasterFormat::Psd,
                _ => RasterFormat::WebP,
            };
            check_options(format, &settings)?;
            // Use Artboards: one file per chosen artboard (default all); else the one chosen.
            let chosen = match use_artboards {
                Some(true) => boards(o.boards.resolve(n))?.unwrap_or_else(|| (0..n).collect()),
                _ => vec![boards(o.boards.one(n))?],
            };
            // JPEG Options → Image Map.
            let map = match (format, o.image_map.as_deref()) {
                (RasterFormat::Jpeg, Some(m)) => MapKind::from_id(m).ok_or_else(|| bad(C, format!("imageMap `{m}`: none, client or server")))?,
                _ => MapKind::None,
            };
            let mut enc = Encoded::default();
            let mut renderer = vectorcraft_render::Renderer::new();
            for b in chosen {
                let region = doc.artboards.get(b).ok_or_else(|| bad(C, format!("no artboard {}", b + 1)))?.rect;
                check_format_size(f, region.width() * scale, region.height() * scale)?;
                vectorcraft_render::raster_size(region, scale).map_err(|e| bad(C, e))?;
                enc.maps.extend(imagemap::build(doc, region, scale, map, enc.files.len()));
                let bytes = renderer.export_region(doc, region, format, &settings).map_err(EngineError::Other)?;
                // File Info as PNG text chunks.
                let bytes = if matches!(format, RasterFormat::Png | RasterFormat::Png8) {
                    vectorcraft_render::encode::png::with_text(bytes, &doc.metadata.png_text(&doc.title))
                } else {
                    bytes
                };
                enc.files.push((Some(b), bytes));
            }
            if enc.files.is_empty() {
                return Err(bad(C, "the document has no artboard"));
            }
            if format == RasterFormat::WebP && o.lossless == Some(false) {
                enc.warnings.push("lossy WebP isn't available yet: the file is lossless (exact, but larger)".into());
            }
            return Ok(enc);
        }
        _ => return Err(bad(C, format!("no encoder for {} yet", f.label))),
    };
    Ok(Encoded::one(bytes))
}
