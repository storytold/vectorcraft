//! File → Export → Save for Web (Legacy): an optimised web image — GIF, JPEG, PNG-8 or PNG-24 —
//! of an artboard or of the art, or one image per slice with an HTML page placing them, plus the
//! named settings the dialog and `webExport.presets.*` keep ([`WebSettings`], [`WebPreset`]).
//!
//! The pipeline: [`render`] draws the region once (straight RGBA at the output size); [`optimize`]
//! turns pixels into a file with the settings (palette reduction and the colour table's edits, web
//! snap, lossy, JPEG quality, metadata); [`decode`] reads a file back for the dialog's preview, so
//! what the preview shows and the size it reports are what Save writes. Slices are cut from the one
//! rendering, each optimised on its own.

use std::collections::HashSet;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};
use vectorcraft_doc::slices::SliceArea;
use vectorcraft_doc::{CellAlign, CellVAlign, Document, NodeId, NodeKind, SliceKind};
use vectorcraft_geom::Rect;
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::quantize::{self, Dither, Indexed, PaletteOptions, Reduction};
use vectorcraft_render::encode::web::{self, ColorSort};
use vectorcraft_render::encode::{gif, jpeg, png};

use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, f64_or, has_doc, str_param, text_param, unique_name_from};

const C: &str = "document.exportForWeb";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.exportForWeb",
            "Save for Web",
            [],
            None,
            "{path?, preset?: a webExport.presets name (its settings, then these keys), format?: gif (default)|jpg|png8|png24, reduction?: perceptual|selective (default)|adaptive|web|blackWhite|gray (gif/png8, as the next five), colors?: 128 (2–256), dither?: none|diffusion (default)|pattern|noise, ditherAmount?: 100, webSnap?: 0 (0–100 %: colours that close to a web-safe one become it), lossy?: 0 (0–100, gif), colorTable?: {locked?: [\"#rrggbb\"…] (kept when reducing), transparent?: [\"#rrggbb\"…] (become transparent), webShift?: [\"#rrggbb\"…] (become web-safe), sort?: none|hue|luminance|popularity, deleted?: [\"#rrggbb\"…] (taken out of the table: their pixels take the nearest color left, and at least one color stays), customPalette?: [\"#rrggbb\"…] (a fixed palette used instead of the reduction's, at most `colors` entries, its transparent colors taking one slot together (the transparent pixels' slot when there are any), its shifted and then its locked colors kept first; a new reduction without colorTable clears it and deleted), shifted?: {\"#source\": \"#rrggbb\"…} (each source color is written as the color given, wherever it is in the image; kept when reducing as a locked color is, before the locked ones, while one palette slot is left for the reduction; at most 256; a source the table does not hold shifts nothing; a color shifted to itself is removed; a shifted color is not web-shifted, and a transparent one stays transparent; GIF lossy moves no pixel into or out of a shifted color)} (colours as the preview's colors[].source; #rrggbbaa and #rgba are refused here and in matte), transparency?: true (gif/png8/png24), matte?: white (default)|black|none|\"#rrggbb\" (what edges blend into; jpg and opaque png24: the background), interlaced?: false (gif/png8/png24), quality?: 60 (jpg: 0–100), progressive?: false (jpg), optimized?: true (jpg), embedProfile?: false (jpg: sRGB profile), width?: px | height?: px | percent?: 100 (the image size, proportional; width wins over height, height over percent), antiAlias?: none|art (default)|type, clipToArtboard?: true (the artboard; false: the art and the slices), artboard?: 0 (0-based), convertToSrgb?: true (PNG files say they are sRGB), metadata?: none|copyright (default)|contact|all (File Info written as PNG text, a GIF or JPEG comment), slices?: all (default)|selected|none (one image per slice: images/<slice name>.<ext>), output?: images (default)|html (an HTML page placing the image or slices, <stem>.html)} → {path, files: [path…], bytes (total), width, height, warnings: [] or [a note that an image couldn't be decoded and is missing from the image]}; no path → {files: [{name, dataBase64}], bytes, width, height, warnings} (names such as page.html, images/page_01.gif). A single image without HTML is written at path itself",
            has_doc,
            export
        ),
        cmd!(
            query "document.exportForWeb.preview",
            "Save for Web Preview",
            [],
            None,
            "{…the settings of document.exportForWeb (preset?, format?…), slice?: n (1-based, from slice.list; default the whole image), kbps?: 56.6 (connection speed for the download time), image?: false (also the optimised file as dataBase64)} what Save writes for the whole image (or one slice) → {format, width, height, bytes (exactly the file's size), seconds (to download at kbps), kbps, colors?: [{color: \"#rrggbb\", source: \"#rrggbb\" (the colour the reduction made, as colorTable lists it), transparent, locked, webShifted, shifted (by colorTable.shifted; also locked), webSafe}] (gif/png8: the colour table in file order), mappedToTransparent?: [\"#rrggbb\"…], slices: images an export writes, warnings (as document.exportForWeb's), dataBase64?}",
            has_doc,
            preview
        ),
        cmd!(
            "document.exportForWeb.saveColorTable",
            "Save Color Table",
            [],
            None,
            "{path?: the .act file (.act is added to a name without an extension), …the settings of document.exportForWeb (preset?, format?: gif|png8…), slice?: n (1-based, from slice.list, among the slices `slices` names; default the whole image, whatever `slices` is)} write the GIF or PNG-8 color table the preview shows as a color table file: 772 bytes, the colors as written (red, green, blue) in file order, zero bytes up to 256 colors, then the number of colors and the transparent entry's index (65535 for none), two bytes each, most significant byte first → {path, colors (how many), transparentIndex (null for none), bytes, warnings}; no path → {dataBase64, colors, transparentIndex, bytes, warnings}. An empty path, jpg and png24 are an error; the document and the settings do not change",
            has_doc,
            save_color_table
        ),
        cmd!(
            query "webExport.loadColorTable",
            "Load Color Table",
            [],
            None,
            "{path | dataBase64: a color table file (.act: 768 bytes of 256 colors, or 772 bytes with the number of colors used, 1–256, and the transparent entry's index), …the settings of document.exportForWeb (preset?, format?: gif|png8…)} the settings with the table loaded: its colors other than the transparent entry become colorTable.customPalette, colors becomes the number of colors it uses (at least 2), the table's other edits (locked, transparent, deleted, shifted, webShift and sort) are cleared and webSnap becomes 0 →{colors: [\"#rrggbb\"…] (the colors it uses, in file order), transparentIndex (null for none; an index past the colors used names none), settings}. Give exactly one of path and dataBase64; another file size, a count of 0 or above 256, jpg and png24 are errors; nothing is remembered",
            always,
            load_color_table
        ),
        cmd!(
            query "webExport.optimizeToFileSize",
            "Optimize to File Size",
            [],
            None,
            "{desiredK: the desired file size in K (1024 bytes; above 0, at most 1048576), startWith?: current (default: the format given, its JPEG quality or GIF/PNG-8 colors lowered; GIF then takes lossy 100)|auto (GIF for art with transparency or at most 256 colors (blended over the matte without transparency), else JPEG; the other one when the first can't reach the size and the art is opaque), use?: allSlices (default: the image, or the image slices' files together)|eachSlice (every image slice's file)|currentSlice (the `slice` given, else the selected image slice, else the first), slice?: n (1-based, from slice.list), preset?, …the settings of document.exportForWeb} the settings whose files fit in the size, searched by encoding at most 32 trial settings (each one encodes every image it measures); slices share one setting, which every scope changes → {targetBytes, achieved (false: no setting tried fits, and settings are the smallest one found), bytes (as use measures them), format, settings, trials, images, warnings}; writes nothing and changes no remembered setting",
            has_doc,
            optimize_to_file_size
        ),
        cmd!(
            query "webExport.settings",
            "Save for Web Settings",
            [],
            None,
            "{reset?: false, preset?, …the settings of document.exportForWeb} remember settings for the Save for Web dialog (it opens on them): the keys given change the remembered ones, a preset replaces them, reset goes back to the defaults; {} reads them → {settings}",
            always,
            settings
        ),
        cmd!(
            query "webExport.presets.list",
            "Save for Web Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, settings}]} the built-in presets, then the saved ones; any name works as `preset` in document.exportForWeb",
            always,
            presets_list
        ),
        cmd!(
            query "webExport.presets.save",
            "Save Save for Web Preset",
            [],
            None,
            "{name, newName?: rename it, preset?: start from this preset (default: the saved preset `name`, else the defaults), settings?: {…}, …the settings of document.exportForWeb} create or change a saved preset (built-in ones can't change) → {name, settings, created}",
            always,
            presets_save
        ),
        cmd!(
            query "webExport.presets.delete",
            "Delete Save for Web Preset",
            [],
            None,
            "{name} delete a saved preset (built-in ones stay) → {deleted: name}",
            always,
            presets_delete
        ),
    ]
}

// ---------- settings ----------

/// Enums of the render crate written in settings by id.
trait Keyed: Sized + Copy {
    fn key(self) -> &'static str;
    fn parse(s: &str) -> Option<Self>;
    const KEYS: &'static str;
}

impl Keyed for Reduction {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "perceptual, selective, adaptive, web, blackWhite or gray";
}

impl Keyed for Dither {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "none, diffusion, pattern or noise";
}

impl Keyed for AntiAlias {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "none, art or type";
}

impl Keyed for ColorSort {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "none, hue, luminance or popularity";
}

fn ser_key<T: Keyed, S: Serializer>(v: &T, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(v.key())
}

fn de_key<'de, T: Keyed, D: Deserializer<'de>>(d: D) -> std::result::Result<T, D::Error> {
    let s = String::deserialize(d)?;
    T::parse(&s).ok_or_else(|| D::Error::custom(format!("`{s}`: {}", T::KEYS)))
}

/// A size in pixels: a number, rounded to whole pixels (`300.0` is 300).
fn de_pixels<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<u32>, D::Error> {
    let Some(v) = Option::<f64>::deserialize(d)? else { return Ok(None) };
    let px = v.round();
    if !(px.is_finite() && px >= 0.0 && px <= f64::from(u32::MAX)) {
        return Err(D::Error::custom(format!("{v} is not a size in pixels")));
    }
    // In range and whole: the cast is exact.
    Ok(Some(px as u32))
}

/// The file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebFormat {
    #[default]
    Gif,
    #[serde(alias = "jpeg")]
    Jpg,
    Png8,
    Png24,
}

impl WebFormat {
    pub const ALL: [WebFormat; 4] = [WebFormat::Gif, WebFormat::Jpg, WebFormat::Png8, WebFormat::Png24];

    pub fn id(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Jpg => "jpg",
            WebFormat::Png8 => "png8",
            WebFormat::Png24 => "png24",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WebFormat::Gif => "GIF",
            WebFormat::Jpg => "JPEG",
            WebFormat::Png8 => "PNG-8",
            WebFormat::Png24 => "PNG-24",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Jpg => "jpg",
            WebFormat::Png8 | WebFormat::Png24 => "png",
        }
    }

    /// A palette format (colour reduction, colour table).
    pub fn palette(self) -> bool {
        matches!(self, WebFormat::Gif | WebFormat::Png8)
    }
}

/// Which File Info goes into the files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebMetadata {
    None,
    #[default]
    Copyright,
    /// The copyright and the author.
    Contact,
    All,
}

impl WebMetadata {
    pub const ALL: [WebMetadata; 4] = [WebMetadata::None, WebMetadata::Copyright, WebMetadata::Contact, WebMetadata::All];

    pub fn key(self) -> &'static str {
        match self {
            WebMetadata::None => "none",
            WebMetadata::Copyright => "copyright",
            WebMetadata::Contact => "contact",
            WebMetadata::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WebMetadata::None => "None",
            WebMetadata::Copyright => "Copyright",
            WebMetadata::Contact => "Copyright and Contact Info",
            WebMetadata::All => "All",
        }
    }

    /// Whether File Info entry `keyword` (as [`vectorcraft_doc::Metadata::png_text`] names it) goes in.
    fn keeps(self, keyword: &str) -> bool {
        match self {
            WebMetadata::None => false,
            WebMetadata::Copyright => keyword.starts_with("Copyright"),
            WebMetadata::Contact => keyword.starts_with("Copyright") || keyword == "Author",
            WebMetadata::All => true,
        }
    }
}

/// Which slices get images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SliceScope {
    #[default]
    All,
    Selected,
    /// One image, slices or not.
    None,
}

impl SliceScope {
    pub const ALL: [SliceScope; 3] = [SliceScope::All, SliceScope::Selected, SliceScope::None];

    pub fn key(self) -> &'static str {
        match self {
            SliceScope::All => "all",
            SliceScope::Selected => "selected",
            SliceScope::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SliceScope::All => "All Slices",
            SliceScope::Selected => "Selected Slices",
            SliceScope::None => "No Slices",
        }
    }
}

/// What Save writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebOutput {
    #[default]
    Images,
    /// An HTML page placing the images too.
    Html,
}

impl WebOutput {
    pub const ALL: [WebOutput; 2] = [WebOutput::Images, WebOutput::Html];

    pub fn key(self) -> &'static str {
        match self {
            WebOutput::Images => "images",
            WebOutput::Html => "html",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WebOutput::Images => "Images Only",
            WebOutput::Html => "HTML and Images",
        }
    }
}

/// The colour table's edits of a palette format, by the colours the reduction makes (the
/// preview's `colors[].source`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorTable {
    /// Kept when the colours are reduced further.
    pub locked: Vec<String>,
    /// Become transparent.
    pub transparent: Vec<String>,
    /// Become their nearest web-safe colour.
    pub web_shift: Vec<String>,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub sort: ColorSort,
    /// Taken out of the table: their pixels take the nearest color left. A deleted color is no
    /// longer locked, transparent or web-shifted.
    pub deleted: Vec<String>,
    /// A fixed palette (a Custom table) used instead of the reduction's own: the colors of the
    /// table when the first color was deleted. Empty: the reduction makes the palette.
    pub custom_palette: Vec<String>,
    /// Colors shifted by hand: each source color (`#rrggbb`) and the color it is written as. A
    /// shifted color is locked, and a source the table does not hold shifts nothing.
    pub shifted: std::collections::BTreeMap<String, String>,
}

/// A color shifted by hand: (source, color written).
type Shift = ([u8; 3], [u8; 3]);

/// The most colours a colour table list holds.
pub const MAX_TABLE: usize = 256;

/// The error for a deletion that would leave the color table no color for the pixels.
pub const KEEP_ONE: &str = "colorTable.deleted: at least one color has to stay in the color table";

/// The error for a colour table list `what` (`colorTable.<what>`) with more than [`MAX_TABLE`] colours.
pub fn too_many(what: &str) -> String {
    format!("colorTable.{what}: at most {MAX_TABLE} colours")
}

impl ColorTable {
    /// Whether the table is a Custom one: a color was deleted or the palette is fixed.
    pub fn is_custom(&self) -> bool {
        !self.deleted.is_empty() || !self.custom_palette.is_empty()
    }

    fn colors(list: &[String], what: &str) -> std::result::Result<Vec<[u8; 3]>, String> {
        if list.len() > MAX_TABLE {
            return Err(too_many(what));
        }
        list.iter().map(|h| Self::rgb(h).ok_or_else(|| format!("colorTable.{what}: `{h}` is not a colour such as \"#ff8800\""))).collect()
    }

    /// The colors shifted by hand as (source, color written). An error names a value that is not a
    /// color such as `#ff8800`, two keys that name one color, or more than 256 entries.
    fn shifts(&self) -> std::result::Result<Vec<Shift>, String> {
        // One entry past the limit is enough for `colors` to refuse a longer map.
        let (keys, values): (Vec<String>, Vec<String>) = self.shifted.iter().take(MAX_TABLE + 1).map(|(k, v)| (k.clone(), v.clone())).unzip();
        let sources = Self::colors(&keys, "shifted")?;
        let to = Self::colors(&values, "shifted")?;
        for (i, (s, k)) in sources.iter().zip(&keys).enumerate() {
            if sources[..i].contains(s) {
                return Err(format!("colorTable.shifted: `{k}` names a color another key names"));
            }
        }
        Ok(sources.into_iter().zip(to).collect())
    }

    /// `h` as RGB; `None` when it is not a color such as `#ff8800`.
    fn rgb(h: &str) -> Option<[u8; 3]> {
        vectorcraft_color::Color::from_hex(h).filter(|_| !super::hex_with_alpha(h)).map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b]
        })
    }
}

/// Save for Web's settings: the params of `document.exportForWeb`, a preset's content and what the
/// dialog opens on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebSettings {
    pub format: WebFormat,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub reduction: Reduction,
    /// Most palette entries, 2–256.
    pub colors: u16,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub dither: Dither,
    /// 0–100.
    pub dither_amount: u8,
    /// Keep transparent pixels transparent (GIF, PNG-8, PNG-24).
    pub transparency: bool,
    /// What partly transparent edges blend into (JPEG and opaque PNG-24: the background): `white`,
    /// `black`, `none` or `#rrggbb`.
    pub matte: String,
    pub interlaced: bool,
    /// 0–100 %: palette colours that close to a web-safe colour become it.
    pub web_snap: u8,
    /// 0–100: lossy GIF compression.
    pub lossy: u8,
    pub color_table: ColorTable,
    /// JPEG quality 0–100.
    pub quality: u8,
    pub progressive: bool,
    /// Optimised Huffman tables (smaller JPEG files).
    pub optimized: bool,
    /// JPEG: embed the sRGB profile.
    pub embed_profile: bool,
    /// The image width in pixels (the height follows).
    #[serde(deserialize_with = "de_pixels")]
    pub width: Option<u32>,
    /// The image height in pixels (the width follows).
    #[serde(deserialize_with = "de_pixels")]
    pub height: Option<u32>,
    /// The image size in percent of the art's size in points.
    pub percent: Option<f64>,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub anti_alias: AntiAlias,
    /// The artboard (else the visible art and the slices).
    pub clip_to_artboard: bool,
    /// 0-based (default the first).
    pub artboard: Option<usize>,
    /// PNG files say their colours are sRGB (what the renderer draws).
    pub convert_to_srgb: bool,
    pub metadata: WebMetadata,
    pub slices: SliceScope,
    pub output: WebOutput,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            format: WebFormat::Gif,
            reduction: Reduction::Selective,
            colors: 128,
            dither: Dither::Diffusion,
            dither_amount: 100,
            transparency: true,
            matte: "white".into(),
            interlaced: false,
            web_snap: 0,
            lossy: 0,
            color_table: ColorTable::default(),
            quality: 60,
            progressive: false,
            optimized: true,
            embed_profile: false,
            width: None,
            height: None,
            percent: None,
            anti_alias: AntiAlias::Art,
            clip_to_artboard: true,
            artboard: None,
            convert_to_srgb: true,
            metadata: WebMetadata::Copyright,
            slices: SliceScope::All,
            output: WebOutput::Images,
        }
    }
}

impl WebSettings {
    /// `self` with the settings keys of `p` (other keys are ignored: params carry `path`, `kbps`…).
    pub fn merged(&self, p: &Value) -> std::result::Result<Self, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let (Some(obj), Some(over)) = (v.as_object_mut(), p.as_object()) {
            for (k, val) in over {
                if let Some(slot) = obj.get_mut(k) {
                    *slot = val.clone();
                }
            }
        }
        let mut s: WebSettings = serde_json::from_value(v).map_err(|e| format!("Save for Web settings: {e}"))?;
        // Another reduction makes a new palette: the deletions and the fixed palette go, unless
        // the params give the color table too.
        if p.get("reduction").is_some() && p.get("colorTable").is_none() && s.reduction != self.reduction {
            s.color_table.deleted.clear();
            s.color_table.custom_palette.clear();
        }
        s.checked()
    }

    /// Reject values no image can have.
    fn checked(mut self) -> std::result::Result<Self, String> {
        self.colors = self.colors.clamp(2, 256);
        self.dither_amount = self.dither_amount.min(100);
        self.web_snap = self.web_snap.min(100);
        self.lossy = self.lossy.min(100);
        self.quality = self.quality.min(100);
        if let Some(p) = self.percent.filter(|p| !(p.is_finite() && *p > 0.0 && *p <= 6400.0)) {
            return Err(format!("percent must be above 0 and at most 6400, not {p}"));
        }
        if self.width == Some(0) || self.height == Some(0) {
            return Err("width and height are at least 1 pixel".into());
        }
        self.matte_rgb()?;
        let table = &mut self.color_table;
        for (list, what) in
            [(&table.locked, "locked"), (&table.transparent, "transparent"), (&table.web_shift, "webShift"), (&table.custom_palette, "customPalette")]
        {
            ColorTable::colors(list, what)?;
        }
        // A deleted color is no longer locked, transparent, web-shifted or shifted.
        let deleted = ColorTable::colors(&table.deleted, "deleted")?;
        if !deleted.is_empty() {
            for list in [&mut table.locked, &mut table.transparent, &mut table.web_shift] {
                list.retain(|h| ColorTable::rgb(h).is_none_or(|c| !deleted.contains(&c)));
            }
        }
        // Shifts are kept as `#rrggbb` in lowercase; a color shifted to itself is not shifted, and
        // a color shifted by hand is not web-shifted.
        let shifts: Vec<Shift> = table.shifts()?.into_iter().filter(|(s, to)| s != to && !deleted.contains(s)).collect();
        table.web_shift.retain(|h| ColorTable::rgb(h).is_none_or(|c| !shifts.iter().any(|(s, _)| *s == c)));
        table.shifted = shifts.into_iter().map(|(s, to)| (hex(s), hex(to))).collect();
        Ok(self)
    }

    /// The matte as RGB (`None`: none).
    pub fn matte_rgb(&self) -> std::result::Result<Option<[u8; 3]>, String> {
        super::fileio::background(&json!(self.matte)).map_err(|e| format!("matte: {e}"))
    }

    /// The JSON the commands return (and presets store).
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// A named set of settings: built in, or saved in the preferences ([`crate::Prefs::web_export_presets`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebPreset {
    pub name: String,
    #[serde(default)]
    pub settings: WebSettings,
}

/// The built-in presets (made here, named by what they do).
pub fn builtin_presets() -> Vec<WebPreset> {
    let d = WebSettings::default;
    let preset = |name: &str, settings: WebSettings| WebPreset { name: name.into(), settings };
    vec![
        preset("GIF, 128 colours", d()),
        preset("GIF, 32 colours, no dither", WebSettings { colors: 32, dither: Dither::None, ..d() }),
        preset("GIF, web-safe colours", WebSettings { reduction: Reduction::Web, colors: 256, dither: Dither::None, ..d() }),
        preset("JPEG, quality 80", WebSettings { format: WebFormat::Jpg, quality: 80, ..d() }),
        preset("JPEG, quality 50", WebSettings { format: WebFormat::Jpg, quality: 50, ..d() }),
        preset("JPEG, quality 25", WebSettings { format: WebFormat::Jpg, quality: 25, ..d() }),
        preset("PNG-8, 64 colours", WebSettings { format: WebFormat::Png8, colors: 64, ..d() }),
        preset("PNG-24", WebSettings { format: WebFormat::Png24, ..d() }),
    ]
}

fn saved_index(saved: &[WebPreset], name: &str) -> Option<usize> {
    saved.iter().position(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

fn is_builtin(name: &str) -> bool {
    builtin_presets().iter().any(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

impl Session {
    /// Every Save for Web preset: the built-in ones, then the saved ones.
    pub fn web_presets(&self) -> Vec<WebPreset> {
        let mut v = builtin_presets();
        v.extend(self.prefs.web_export_presets.iter().cloned());
        v
    }

    /// The settings `p` asks for: its `preset` (else `base`) with its settings keys.
    pub fn web_settings_over(&self, base: &WebSettings, p: &Value) -> std::result::Result<WebSettings, String> {
        let start = match p.get("preset").and_then(Value::as_str) {
            Some(name) => {
                let key = name.trim();
                self.web_presets()
                    .into_iter()
                    .find(|q| q.name.eq_ignore_ascii_case(key))
                    .ok_or_else(|| format!("unknown preset `{name}` (see webExport.presets.list)"))?
                    .settings
            }
            None => base.clone(),
        };
        start.merged(p)
    }

    /// The settings of a `document.exportForWeb` call: the defaults (or its preset) with its keys.
    pub fn web_settings(&self, p: &Value) -> std::result::Result<WebSettings, String> {
        self.web_settings_over(&WebSettings::default(), p)
    }

    /// What the Save for Web dialog opens on: the remembered settings, else the defaults.
    pub fn last_web_settings(&self) -> WebSettings {
        self.prefs.web_export_settings.clone().unwrap_or_default()
    }
}

// ---------- the pipeline ----------

/// The region an export covers and its pixels per point.
pub fn region(doc: &Document, s: &WebSettings) -> std::result::Result<(Rect, f64), String> {
    let r = if s.clip_to_artboard {
        let i = s.artboard.unwrap_or(0);
        doc.artboards.get(i).map(|a| a.rect).ok_or_else(|| format!("no artboard {i} (0-based; the document has {})", doc.artboards.len()))?
    } else {
        let slices = if s.slices == SliceScope::None { vec![] } else { doc.slice_layout() };
        slices
            .iter()
            .fold(vectorcraft_render::encode::art_bounds(doc), |acc, a| vectorcraft_geom::union_opt(acc, Some(a.rect)))
            .ok_or("nothing to save: the document has no visible art")?
    };
    let scale = match (s.width, s.height, s.percent) {
        (Some(w), _, _) => f64::from(w) / r.width(),
        (None, Some(h), _) => f64::from(h) / r.height(),
        (None, None, Some(p)) => p / 100.0,
        _ => 1.0,
    };
    vectorcraft_render::raster_size(r, scale)?;
    Ok((r, scale))
}

/// A rendering of the region an export covers, before optimisation.
#[derive(Clone, Debug)]
pub struct WebRender {
    /// Straight RGBA, `width`×`height`.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub region: Rect,
    /// Pixels per point.
    pub scale: f64,
}

/// Draw what `s` exports of `doc` (template layers left out; on white when the document's
/// background is white).
pub fn render(doc: &Document, s: &WebSettings) -> std::result::Result<WebRender, String> {
    let doc = doc.without_edit_modes();
    let (region, scale) = region(&doc, s)?;
    let page = (doc.setup.background == vectorcraft_doc::Background::White).then_some([255; 4]);
    let opts = vectorcraft_render::RenderOptions {
        background: page,
        skip_templates: true,
        anti_alias: s.anti_alias,
        precise: true,
        supersample: true,
        ..Default::default()
    };
    let img = vectorcraft_render::Renderer::new().render_region_with(&doc, region, scale, &opts);
    Ok(WebRender { rgba: img.to_straight(), width: img.width, height: img.height, region, scale })
}

/// One entry of a palette format's colour table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableColor {
    /// As written.
    pub color: [u8; 3],
    /// As the reduction made it (what the colour table's lists name).
    pub source: [u8; 3],
    pub transparent: bool,
    pub locked: bool,
    pub web_shifted: bool,
    /// Shifted by hand (`colorTable.shifted`): written as `color` instead of `source`.
    pub shifted: bool,
}

/// An optimised image file.
#[derive(Clone, Debug, Default)]
pub struct Optimized {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The colour table (palette formats), in file order.
    pub colors: Vec<TableColor>,
    /// The colour table's colours that became transparent.
    pub mapped: Vec<[u8; 3]>,
    /// For [`preview`]: the same pixels in a file every decoder reads (a JPEG with optimised
    /// Huffman tables decodes wrongly in the `image` crate's decoder; a baseline one of the same
    /// quality has the same pixels). Empty when `bytes` serves.
    display: Vec<u8>,
}

impl Optimized {
    /// The file the preview decodes ([`decode`]).
    pub fn display(&self) -> &[u8] {
        if self.display.is_empty() { &self.bytes } else { &self.display }
    }
}

/// `#rrggbb`.
pub fn hex([r, g, b]: [u8; 3]) -> String {
    vectorcraft_color::Color::rgb8(r, g, b).to_hex()
}

/// `rgba` (straight, `w`×`h`) flattened on `matte`.
fn flatten(rgba: &[u8], matte: [u8; 3]) -> impl Iterator<Item = [u8; 3]> + '_ {
    rgba.as_chunks::<4>().0.iter().map(move |p| {
        let a = u32::from(p[3]);
        [0, 1, 2].map(|i| ((u32::from(p[i]) * a + u32::from(matte[i]) * (255 - a) + 127) / 255) as u8)
    })
}

/// The File Info text `s.metadata` lets into the files: `(keyword, text)`.
fn metadata(doc: &Document, s: &WebSettings) -> Vec<(&'static str, String)> {
    doc.metadata.png_text(doc.info_title()).into_iter().filter(|(k, _)| s.metadata.keeps(k)).map(|(k, v)| (k, v.into_owned())).collect()
}

/// Encode straight RGBA pixels (`w`×`h`) as `s` says; `doc` gives the metadata.
pub fn optimize(doc: &Document, rgba: &[u8], w: u32, h: u32, s: &WebSettings) -> std::result::Result<Optimized, String> {
    encode(doc, rgba, w, h, s, false)
}

/// [`optimize`] for the dialog's preview: also what [`Optimized::display`] decodes.
pub fn preview_image(doc: &Document, rgba: &[u8], w: u32, h: u32, s: &WebSettings) -> std::result::Result<Optimized, String> {
    encode(doc, rgba, w, h, s, true)
}

fn encode(doc: &Document, rgba: &[u8], w: u32, h: u32, s: &WebSettings, preview: bool) -> std::result::Result<Optimized, String> {
    if rgba.len() as u64 != u64::from(w) * u64::from(h) * 4 || w == 0 || h == 0 {
        return Err("Save for Web: the image is empty".into());
    }
    let matte = s.matte_rgb()?;
    let meta = metadata(doc, s);
    let comment = || meta.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("\n");
    let png_extras = |file: Vec<u8>| {
        let file = if s.convert_to_srgb { png::with_srgb(file) } else { file };
        let text: Vec<(&str, std::borrow::Cow<str>)> = meta.iter().map(|(k, v)| (*k, v.as_str().into())).collect();
        png::with_text(file, &text)
    };
    let png_options = png::PngOptions { ppi: Some(72.0), interlaced: s.interlaced };
    let mut out = Optimized { width: w, height: h, ..Default::default() };
    out.bytes = match s.format {
        WebFormat::Jpg => {
            let rgb: Vec<u8> = flatten(rgba, matte.unwrap_or([255; 3])).flatten().collect();
            let method = match (s.progressive, s.optimized) {
                (true, _) => jpeg::Method::Progressive,
                (false, true) => jpeg::Method::Optimized,
                (false, false) => jpeg::Method::Baseline,
            };
            let o = jpeg::JpegOptions { color_model: jpeg::ColorModel::Rgb, method, scans: 3, embed_icc: s.embed_profile };
            if preview && method == jpeg::Method::Optimized {
                let baseline = jpeg::JpegOptions { method: jpeg::Method::Baseline, embed_icc: false, ..o.clone() };
                out.display = jpeg::encode(&rgb, w, h, s.quality, None, &baseline)?;
            }
            web::jpeg_comment(jpeg::encode(&rgb, w, h, s.quality, Some(72.0), &o)?, &comment())
        }
        WebFormat::Png24 => {
            let file = match (s.transparency, matte.unwrap_or([255; 3])) {
                (true, _) => png::encode(rgba, w, h, &png_options)?,
                (false, m) => {
                    let opaque: Vec<u8> = flatten(rgba, m).flat_map(|[r, g, b]| [r, g, b, 255]).collect();
                    png::encode(&opaque, w, h, &png_options)?
                }
            };
            png_extras(file)
        }
        WebFormat::Gif | WebFormat::Png8 => {
            let PaletteImage { ix, colors, mapped } = palette_image(rgba, w, h, s, matte)?;
            out.colors = colors;
            out.mapped = mapped;
            match s.format {
                WebFormat::Gif => web::gif_comment(gif::encode(&ix, s.interlaced)?, &comment()),
                _ => png_extras(png::encode_indexed(&ix, &png_options)?),
            }
        }
    };
    Ok(out)
}

/// A palette format's image with the colour table's edits.
struct PaletteImage {
    ix: Indexed,
    /// Its colour table.
    colors: Vec<TableColor>,
    /// The colours mapped to transparent.
    mapped: Vec<[u8; 3]>,
}

fn palette_image(rgba: &[u8], w: u32, h: u32, s: &WebSettings, matte: Option<[u8; 3]>) -> std::result::Result<PaletteImage, String> {
    let table = &s.color_table;
    let locked = ColorTable::colors(&table.locked, "locked")?;
    let to_clear = ColorTable::colors(&table.transparent, "transparent")?;
    let shift = ColorTable::colors(&table.web_shift, "webShift")?;
    let by_hand = table.shifts()?;
    let shifted_to = |src: &[u8; 3]| by_hand.iter().find(|(s, _)| s == src).map(|(_, to)| *to);
    // A color shifted by hand is kept when the colors are reduced, as a locked one is, and takes
    // one of the slots kept for them before the locked colors do.
    let kept: Vec<[u8; 3]> = by_hand.iter().map(|(s, _)| *s).chain(locked.iter().copied()).collect();
    let o = PaletteOptions {
        colors: s.colors,
        reduction: s.reduction,
        dither: s.dither,
        dither_amount: s.dither_amount,
        transparency: s.transparency,
        matte,
    };
    let (mut ix, dropped) = reduced(rgba, w, h, s, &o, &kept)?;
    // Map to transparent works on the colours the reduction made.
    let before = ix.palette.clone();
    let clear = ix.transparent.map(usize::from);
    let mut mapped: Vec<[u8; 3]> = before.iter().enumerate().filter(|(i, c)| Some(*i) != clear && to_clear.contains(c)).map(|(_, c)| *c).collect();
    // The colors mapped to transparent that did not fit in a fixed palette and whose pixels became
    // transparent.
    for c in dropped {
        if !mapped.contains(&c) {
            mapped.push(c);
        }
    }
    let remap = web::to_transparent(&mut ix, |i| Some(i) != clear && before.get(i).is_some_and(|c| to_clear.contains(c)));
    let mut sources = ix.palette.clone();
    for (old, new) in remap.iter().enumerate() {
        if let (Some(n), Some(c)) = (new, before.get(old))
            && let Some(slot) = sources.get_mut(usize::from(*n))
        {
            *slot = *c;
        }
    }
    // Web shift and web snap (locked colors and colors shifted by hand stay as they are).
    let clear = ix.transparent.map(usize::from);
    for (i, c) in ix.palette.iter_mut().enumerate() {
        let src = sources.get(i).copied().unwrap_or(*c);
        if Some(i) == clear || shifted_to(&src).is_some() || locked.contains(&src) {
            continue;
        }
        if shift.contains(&src) {
            *c = quantize::web_safe(*c);
        } else if let Some(snapped) = web::snap(*c, s.web_snap) {
            *c = snapped;
        }
    }
    // Lossy compares the pixels with the colors written for them, and moves no pixel into or out of
    // a color shifted by hand.
    if s.format == WebFormat::Gif {
        let fixed: Vec<u8> = sources.iter().enumerate().filter(|(_, c)| shifted_to(c).is_some()).filter_map(|(i, _)| u8::try_from(i).ok()).collect();
        web::lossy(&mut ix, rgba, s.lossy, &fixed);
    }
    // The colors shifted by hand.
    for (i, c) in ix.palette.iter_mut().enumerate() {
        if Some(i) != clear
            && let Some(to) = sources.get(i).and_then(shifted_to)
        {
            *c = to;
        }
    }
    let order = web::sort(&mut ix, table.sort);
    let sources: Vec<[u8; 3]> = order.iter().map(|&i| sources.get(i).copied().unwrap_or_default()).collect();
    let colors = ix
        .palette
        .iter()
        .zip(&sources)
        .enumerate()
        .map(|(i, (c, src))| {
            let transparent = ix.transparent == u8::try_from(i).ok();
            let shifted = !transparent && shifted_to(src).is_some();
            TableColor {
                color: *c,
                source: *src,
                transparent,
                locked: locked.contains(src) || shifted,
                web_shifted: shift.contains(src) && !shifted,
                shifted,
            }
        })
        .collect();
    Ok(PaletteImage { ix, colors, mapped })
}

/// The pixels reduced to the palette `s` gives: the reduction's own (`locked` kept), or the
/// color table's fixed one, without the deleted colors. The pixels of a deleted color take the
/// nearest color left, with `o`'s dither. A deletion has to leave a color that is not mapped to
/// transparent. A fixed palette keeps its colors mapped to transparent, which take one slot of
/// `colors` together (the transparent pixels' slot when there are any), and the opaque colors that
/// fit in the rest: the `locked` ones first, in their order, then the others in palette order. Also returns the
/// colors mapped to transparent that did not fit and whose pixels became transparent.
fn reduced(
    rgba: &[u8],
    w: u32,
    h: u32,
    s: &WebSettings,
    o: &PaletteOptions,
    locked: &[[u8; 3]],
) -> std::result::Result<(Indexed, Vec<[u8; 3]>), String> {
    let table = &s.color_table;
    let deleted = ColorTable::colors(&table.deleted, "deleted")?;
    let custom = ColorTable::colors(&table.custom_palette, "customPalette")?;
    let to_clear = ColorTable::colors(&table.transparent, "transparent")?;
    let opaque_left = |left: &[[u8; 3]]| left.iter().any(|c| !to_clear.contains(c));
    if custom.is_empty() {
        let ix = quantize::quantize_locked(rgba, w, h, o, locked);
        let clear = ix.transparent.map(usize::from);
        let gone = |i: usize, c: &[u8; 3]| Some(i) != clear && deleted.contains(c);
        if !ix.palette.iter().enumerate().any(|(i, c)| gone(i, c)) {
            return Ok((ix, vec![]));
        }
        let left: Vec<[u8; 3]> = ix.palette.iter().enumerate().filter(|&(i, c)| Some(i) != clear && !deleted.contains(c)).map(|(_, c)| *c).collect();
        if left.is_empty() || !opaque_left(&left) {
            return Err(KEEP_ONE.into());
        }
        return Ok((quantize::quantize_to(rgba, w, h, o, &left), vec![]));
    }
    // A fixed palette holds at most `colors` entries, the transparent one included.
    let has_clear = s.transparency && rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 128);
    let removed = custom.iter().any(|c| deleted.contains(c));
    let mut left: Vec<[u8; 3]> = vec![];
    for c in custom {
        if !deleted.contains(&c) && !left.contains(&c) {
            left.push(c);
        }
    }
    if left.is_empty() || (removed && !opaque_left(&left)) {
        return Err(KEEP_ONE.into());
    }
    // The colors mapped to transparent all become the one transparent entry, so together they take
    // one slot, and the opaque colors fill the rest. With transparent pixels that slot is theirs, so
    // the colors mapped to transparent take no entry of their own and their pixels take the
    // transparent pixels' entry. Without them, when no pixel takes one of those colors, the slot
    // goes to the next opaque color instead.
    let kept = fixed_palette(&left, &to_clear, locked, s.colors, has_clear, !has_clear);
    let mut ix = quantize::quantize_to(rgba, w, h, o, &kept);
    let dropped = clear_dropped(&mut ix, rgba, o, &kept, &left, &to_clear);
    if !has_clear && !ix.palette.iter().any(|c| to_clear.contains(c)) && left.iter().any(|c| to_clear.contains(c)) {
        return Ok((quantize::quantize_to(rgba, w, h, o, &fixed_palette(&left, &to_clear, locked, s.colors, has_clear, false)), vec![]));
    }
    Ok((ix, dropped))
}

/// The entries of the fixed palette `left` that fit in `colors`, in palette order. The opaque
/// colors in `locked` are kept first, in the order `locked` lists them (the colors shifted by hand
/// before the locked ones), then the other opaque colors in palette order. With `clear`,
/// the colors in `to_clear` are kept and take one slot together; without it they are left out. The
/// list holds at most 256 entries, less one when `has_clear` (the transparent pixels' entry).
fn fixed_palette(left: &[[u8; 3]], to_clear: &[[u8; 3]], locked: &[[u8; 3]], colors: u16, has_clear: bool, clear: bool) -> Vec<[u8; 3]> {
    let any_clear = clear && left.iter().any(|c| to_clear.contains(c));
    let total = 256 - usize::from(has_clear);
    let room = (usize::from(colors.clamp(2, 256)) - usize::from(has_clear || any_clear)).min(total - usize::from(any_clear));
    let opaque = |c: &&[u8; 3]| !to_clear.contains(c);
    let mut first: Vec<[u8; 3]> = vec![];
    for c in locked.iter().filter(opaque).filter(|c| left.contains(c)) {
        if first.len() < room && !first.contains(c) {
            first.push(*c);
        }
    }
    let rest = room - first.len();
    let others: Vec<[u8; 3]> = left.iter().filter(opaque).filter(|c| !locked.contains(c)).take(rest).copied().collect();
    let mut clear_room = if clear { total - first.len() - others.len() } else { 0 };
    left.iter()
        .copied()
        .filter(|c| {
            if !to_clear.contains(c) {
                return first.contains(c) || others.contains(c);
            }
            let keep = clear_room > 0;
            clear_room = clear_room.saturating_sub(1);
            keep
        })
        .collect()
}

/// Makes transparent the pixels of `ix` whose nearest color is a color of `left` mapped to
/// transparent (`to_clear`) that does not fit in the fixed palette `kept`. Such a pixel takes the
/// transparent entry, or an entry mapped to transparent, which Map to Transparent clears later.
/// Returns those colors that a pixel took.
fn clear_dropped(ix: &mut Indexed, rgba: &[u8], o: &PaletteOptions, kept: &[[u8; 3]], left: &[[u8; 3]], to_clear: &[[u8; 3]]) -> Vec<[u8; 3]> {
    let dropped: Vec<[u8; 3]> = left.iter().filter(|c| to_clear.contains(c) && !kept.contains(c)).copied().collect();
    if dropped.is_empty() {
        return vec![];
    }
    let all: Vec<[u8; 3]> = kept.iter().chain(&dropped).copied().collect();
    let Indexed { palette, transparent, indices, .. } = ix;
    // The entry such pixels take, looked up at the first of them.
    let mut entry: Option<Option<u8>> = None;
    let mut taken = vec![false; dropped.len()];
    let mut pixel = 0;
    quantize::nearest_entries(rgba, o, &all, |n| {
        if let Some(d) = n.and_then(|n| n.checked_sub(kept.len())) {
            let e = *entry.get_or_insert_with(|| {
                transparent.or_else(|| palette.iter().position(|c| to_clear.contains(c)).and_then(|i| u8::try_from(i).ok())).or_else(|| {
                    // A kept color mapped to transparent that no pixel took; at most 255 entries are used then.
                    let c = kept.iter().find(|c| to_clear.contains(c))?;
                    let i = u8::try_from(palette.len()).ok()?;
                    palette.push(*c);
                    Some(i)
                })
            });
            if let (Some(e), Some(v)) = (e, indices.get_mut(pixel)) {
                *v = e;
                if let Some(t) = taken.get_mut(d) {
                    *t = true;
                }
            }
        }
        pixel += 1;
    });
    dropped.into_iter().zip(taken).filter(|(_, t)| *t).map(|(c, _)| c).collect()
}

/// An optimised image as straight RGBA → `(pixels, width, height)` (the dialog's preview).
pub fn decode(o: &Optimized) -> std::result::Result<(Vec<u8>, u32, u32), String> {
    let img = craft_image_codecs::decode(o.display()).map_err(|e| format!("the optimised image can't be read back: {e}"))?;
    let (w, h) = img.dimensions();
    Ok((img.to_rgba8(), w, h))
}

/// Seconds to download `bytes` at `kbps` kilobits per second.
pub fn download_seconds(bytes: usize, kbps: f64) -> f64 {
    if kbps > 0.0 { bytes as f64 * 8.0 / (kbps * 1000.0) } else { 0.0 }
}

/// Connection speeds the dialog offers (kilobits per second).
pub const SPEEDS: [f64; 8] = [28.8, 56.6, 128.0, 256.0, 512.0, 1024.0, 2048.0, 8192.0];

// ---------- slices and the HTML page ----------

/// One slice as written: its area in image pixels and what its cell holds.
struct Cell<'a> {
    area: &'a SliceArea,
    /// Pixel rectangle `[x0, y0, x1, y1]` in the image.
    px: [u32; 4],
}

/// The slices of `doc` that get cells (`selected`: the selected slice ids for
/// [`SliceScope::Selected`]), in pixels of `r`. Empty when the document has none (or `s` says none).
fn cells<'a>(areas: &'a [SliceArea], selected: &[NodeId], s: &WebSettings, r: &WebRender) -> std::result::Result<Vec<Cell<'a>>, String> {
    if s.slices == SliceScope::None {
        return Ok(vec![]);
    }
    let edge = |v: f64, o: f64, max: u32| ((v - o) * r.scale).round().clamp(0.0, f64::from(max)) as u32;
    let out: Vec<Cell> = areas
        .iter()
        .filter(|a| s.slices == SliceScope::All || a.id.is_some_and(|id| selected.contains(&id)))
        .map(|a| Cell {
            area: a,
            px: [
                edge(a.rect.x0, r.region.x0, r.width),
                edge(a.rect.y0, r.region.y0, r.height),
                edge(a.rect.x1, r.region.x0, r.width),
                edge(a.rect.y1, r.region.y0, r.height),
            ],
        })
        .filter(|c| c.px[2] > c.px[0] && c.px[3] > c.px[1])
        .collect();
    if out.is_empty() && s.slices == SliceScope::Selected {
        return Err("no selected slice in the image: select slices, or save all of them".into());
    }
    Ok(out)
}

/// The pixels of `px` (`[x0, y0, x1, y1]`) of `r`.
fn crop(r: &WebRender, [x0, y0, x1, y1]: [u32; 4]) -> Vec<u8> {
    let stride = r.width as usize * 4;
    (y0..y1)
        .flat_map(|y| r.rgba.get(y as usize * stride + x0 as usize * 4..y as usize * stride + x1 as usize * 4).unwrap_or_default())
        .copied()
        .collect()
}

/// What an export writes, by name relative to its folder.
pub struct WebFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The images folder of HTML and sliced output.
const IMAGES: &str = "images";

/// The files of an export of `doc` named after `stem`: one image (`<stem>.<ext>`), or one per
/// slice (`images/<slice name>.<ext>`), and with HTML output the page (`<stem>.html`, its image in
/// `images/`). The page comes first. Returns the image size too.
pub fn export_files(doc: &Document, selected: &[NodeId], s: &WebSettings, stem: &str) -> std::result::Result<(Vec<WebFile>, (u32, u32)), String> {
    let r = render(doc, s)?;
    let areas = doc.slice_layout();
    let cells = cells(&areas, selected, s, &r)?;
    let ext = s.format.ext();
    let html = s.output == WebOutput::Html;
    let mut images = vec![];
    // (cell, image file name) for the page.
    let mut placed: Vec<(Option<&Cell>, Option<String>)> = vec![];
    if cells.is_empty() {
        let name = if html { format!("{IMAGES}/{stem}.{ext}") } else { format!("{stem}.{ext}") };
        images.push(WebFile { name: name.clone(), bytes: optimize(doc, &r.rgba, r.width, r.height, s)?.bytes });
        placed.push((None, Some(name)));
    } else {
        let mut taken = HashSet::new();
        // Per base, where the suffix search resumes, so many slices of one name don't each count up from 2.
        let mut next = std::collections::HashMap::new();
        for c in &cells {
            let kind = c.area.id.and_then(|id| doc.slice_options(id)).map(|o| o.kind).unwrap_or_default();
            if kind != SliceKind::Image {
                placed.push((Some(c), None));
                continue;
            }
            let base: String = doc.slice_name(c.area).chars().map(|ch| if ch.is_alphanumeric() || "-_.".contains(ch) { ch } else { '-' }).collect();
            // A slice named after a Windows device (`aux`, `NUL`…) gets a `-` after the device name.
            let file = |base: &str| super::fileio::safe_file_name(&format!("{base}.{ext}"));
            let base = unique_name_from(&base, &mut next, |n| taken.contains(&file(n).to_lowercase()));
            taken.insert(file(&base).to_lowercase());
            let name = format!("{IMAGES}/{}", file(&base));
            let [x0, y0, x1, y1] = c.px;
            images.push(WebFile { name: name.clone(), bytes: optimize(doc, &crop(&r, c.px), x1 - x0, y1 - y0, s)?.bytes });
            placed.push((Some(c), Some(name)));
        }
    }
    let mut files = vec![];
    if html {
        files.push(WebFile { name: format!("{stem}.html"), bytes: page(doc, stem, (r.width, r.height), &placed, s).into_bytes() });
    }
    files.extend(images);
    Ok((files, (r.width, r.height)))
}

/// The HTML page placing the image or the slices (absolutely positioned, so overlapping slices
/// stack as laid out).
fn page(doc: &Document, stem: &str, (w, h): (u32, u32), placed: &[(Option<&Cell>, Option<String>)], s: &WebSettings) -> String {
    use super::fileio::imagemap::attr;
    use std::fmt::Write as _;
    let id = attr(stem);
    let mut out = String::new();
    let _ = writeln!(out, "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{id}</title>");
    let _ = writeln!(out, "<style>\nbody {{ margin: 0; }}\n.slices {{ position: relative; width: {w}px; height: {h}px; }}");
    let _ = writeln!(
        out,
        ".slices > * {{ position: absolute; display: block; overflow: hidden; }}\n.slices img {{ display: block; border: 0; }}\n</style>"
    );
    let _ = writeln!(out, "</head>\n<body>\n<div class=\"slices\" id=\"{id}\">");
    let matte = s.matte_rgb().ok().flatten().map(hex).unwrap_or_default();
    for (cell, image) in placed {
        let [x0, y0, x1, y1] = cell.map_or([0, 0, w, h], |c| c.px);
        let (cw, ch) = (x1 - x0, y1 - y0);
        let pos = format!("left: {x0}px; top: {y0}px; width: {cw}px; height: {ch}px;");
        let o = cell.and_then(|c| c.area.id).and_then(|i| doc.slice_options(i)).cloned().unwrap_or_default();
        match image {
            Some(src) => {
                let img = format!("<img src=\"{}\" width=\"{cw}\" height=\"{ch}\" alt=\"{}\">", attr(src), attr(&o.alt));
                if o.url.is_empty() {
                    let _ = writeln!(out, "<div style=\"{pos}\">{img}</div>");
                } else {
                    let target = if o.target.is_empty() { String::new() } else { format!(" target=\"{}\"", attr(&o.target)) };
                    let title = if o.message.is_empty() { String::new() } else { format!(" title=\"{}\"", attr(&o.message)) };
                    let _ = writeln!(out, "<a href=\"{}\"{target}{title} style=\"{pos}\">{img}</a>", attr(&o.url));
                }
            }
            None => {
                let bg = match o.background.as_str() {
                    "" => String::new(),
                    "matte" if !matte.is_empty() => format!(" background: {matte};"),
                    c if c.starts_with('#') => format!(" background: {};", attr(c)),
                    _ => String::new(),
                };
                let justify = match o.h_align {
                    CellAlign::Center => "center",
                    CellAlign::Right => "flex-end",
                    _ => "flex-start",
                };
                let align = match o.v_align {
                    CellVAlign::Middle => "center",
                    CellVAlign::Bottom => "flex-end",
                    CellVAlign::Baseline => "baseline",
                    _ => "flex-start",
                };
                // No Image: the cell's text as it is (HTML allowed); HTML Text: the type's text.
                let text = match o.kind {
                    SliceKind::HtmlText => cell
                        .and_then(|c| c.area.id)
                        .and_then(|i| doc.node(i))
                        .and_then(|n| match &n.kind {
                            NodeKind::Text(t) => Some(t.plain_text().split(['\n', '\r', '\u{2029}']).map(attr).collect::<Vec<_>>().join("<br>")),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    _ => o.text.clone(),
                };
                let _ = writeln!(out, "<div style=\"{pos}{bg} display: flex; justify-content: {justify}; align-items: {align};\">{text}</div>");
            }
        }
    }
    out.push_str("</div>\n</body>\n</html>\n");
    out
}

// ---------- commands ----------

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let settings = s.web_settings(p).map_err(|e| bad(C, e))?;
    let st = s.doc()?;
    let selected = super::slices::selected_slices(&st.doc, &st.selection);
    let path = str_param(p, "path").filter(|p| !p.trim().is_empty());
    let stem = match path {
        Some(p) => super::fileio::file_stem(p),
        // Without a path the files are named after the title, following Windows' file-naming rules.
        None => super::fileio::safe_file_name(&super::fileio::file_stem(&st.doc.title)),
    };
    let stem = if stem.trim().is_empty() { "Untitled".to_string() } else { stem };
    let failed = vectorcraft_render::decode_failures();
    let (files, (width, height)) = export_files(&st.doc, &selected, &settings, &stem).map_err(|e| bad(C, e))?;
    let warnings = undecoded(failed);
    let total: usize = files.iter().map(|f| f.bytes.len()).sum();
    let Some(path) = path else {
        let rows: Vec<Value> = files.iter().map(|f| json!({ "name": f.name, "dataBase64": vectorcraft_format::base64_encode(&f.bytes) })).collect();
        return Ok(json!({ "files": rows, "bytes": total, "width": width, "height": height, "warnings": warnings }));
    };
    // Siblings keep the path's own separators.
    let dir = &path[..path.rfind(['/', '\\']).map_or(0, |i| i + 1)];
    if files.iter().any(|f| f.name.contains('/')) {
        super::fileio::create_dir(&format!("{dir}{IMAGES}"))?;
    }
    let mut written = vec![];
    for f in &files {
        // A lone image goes where it was asked to.
        let target = if files.len() == 1 && !f.name.contains('/') { path.to_string() } else { format!("{dir}{}", f.name) };
        super::fileio::write_file(&target, &f.bytes)?;
        written.push(target);
    }
    Ok(json!({ "path": written.first(), "files": written, "bytes": total, "width": width, "height": height, "warnings": warnings }))
}

/// The warnings of a rendering that started when this thread's decode failure count was `failed`:
/// the note the raster exports add (`fileio::encode::encode_all_of`) when an image has failed to
/// decode since then and so is missing from the image.
fn undecoded(failed: u64) -> Vec<&'static str> {
    super::rasterfx::undecoded_warning(failed).into_iter().collect()
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    const P: &str = "document.exportForWeb.preview";
    let settings = s.web_settings(p).map_err(|e| bad(P, e))?;
    let kbps = f64_or(p, "kbps", 56.6);
    if !(kbps.is_finite() && kbps > 0.0) {
        return Err(bad(P, format!("kbps must be a positive number, not {kbps}")));
    }
    let Shown { o, images, warnings } = shown(s, &settings, p, P, true)?;
    let mut out = json!({
        "format": settings.format.id(), "width": o.width, "height": o.height, "bytes": o.bytes.len(),
        "seconds": download_seconds(o.bytes.len(), kbps), "kbps": kbps, "slices": images, "warnings": warnings,
    });
    if settings.format.palette() {
        out["colors"] = o
            .colors
            .iter()
            .map(|c| {
                json!({
                    "color": hex(c.color), "source": hex(c.source), "transparent": c.transparent, "locked": c.locked,
                    "webShifted": c.web_shifted, "shifted": c.shifted, "webSafe": quantize::web_safe(c.color) == c.color,
                })
            })
            .collect();
        out["mappedToTransparent"] = o.mapped.iter().map(|c| json!(hex(*c))).collect();
    }
    if bool_or(p, "image", false) {
        out["dataBase64"] = json!(vectorcraft_format::base64_encode(&o.bytes));
    }
    Ok(out)
}

/// The file Save writes with some settings, as the preview shows it.
struct Shown {
    o: Optimized,
    /// The images an export writes (0 when they were not counted).
    images: usize,
    /// The export's warnings.
    warnings: Vec<&'static str>,
}

/// What Save writes with `settings` for the whole image, or for the slice `p.slice` names (1-based,
/// from `slice.list`), for the command `cmd`. A `slice` that is not a whole number from 1 up is an
/// error. With `count_images` false and no `slice`, the slices are not looked up: `images` is 0,
/// and selected slices with none selected are not an error.
fn shown(s: &mut Session, settings: &WebSettings, p: &Value, cmd: &str, count_images: bool) -> Result<Shown> {
    let slice = match p.get("slice") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|n| *n >= 1).ok_or_else(|| bad(cmd, "`slice` is a slice number from `slice.list`"))?),
    };
    let st = s.doc()?;
    let doc = &st.doc;
    let failed = vectorcraft_render::decode_failures();
    let r = render(doc, settings).map_err(|e| bad(cmd, e))?;
    let warnings = undecoded(failed);
    let areas = doc.slice_layout();
    let cells = if count_images || slice.is_some() {
        let selected = super::slices::selected_slices(doc, &st.selection);
        cells(&areas, &selected, settings, &r).map_err(|e| bad(cmd, e))?
    } else {
        vec![]
    };
    let images = if count_images {
        cells.iter().filter(|c| c.area.id.and_then(|id| doc.slice_options(id)).is_none_or(|o| o.kind == SliceKind::Image)).count().max(1)
    } else {
        0
    };
    let o = match slice {
        Some(n) => {
            let c =
                cells.iter().find(|c| c.area.number as u64 == n).ok_or_else(|| bad(cmd, format!("no slice {n} in the image (see slice.list)")))?;
            let [x0, y0, x1, y1] = c.px;
            optimize(doc, &crop(&r, c.px), x1 - x0, y1 - y0, settings)
        }
        None => optimize(doc, &r.rgba, r.width, r.height, settings),
    }
    .map_err(|e| bad(cmd, e))?;
    Ok(Shown { o, images, warnings })
}

// ---------- color table files ----------

/// The extension of a color table file.
pub const COLOR_TABLE_EXT: &str = "act";
/// The size of a color table file with its color count and transparent entry: 256 RGB triples
/// (768 bytes), then the two numbers, each two bytes, most significant byte first.
pub const ACT_LEN: usize = 772;
/// The size of a color table file of 256 RGB triples alone.
const ACT_COLORS_LEN: usize = 768;
/// The transparent entry of a color table file that has none.
const ACT_NO_TRANSPARENT: u16 = 0xFFFF;

/// The error for bytes that are not a color table file.
const NOT_A_TABLE: &str = "not a color table: an .act file is 768 or 772 bytes long and uses 1 to 256 colors";

/// A color table file (`.act`) of `colors`, a palette format's color table in file order: each
/// color as written, red, green and blue, one byte each; zero bytes up to 256 colors; the number
/// of colors and the index of the transparent entry (`0xFFFF` for none), two bytes each, most
/// significant byte first. Colors past the 256th are left out.
pub fn act_bytes(colors: &[TableColor]) -> Vec<u8> {
    let colors = &colors[..colors.len().min(256)];
    let mut out = vec![0u8; ACT_LEN];
    for (slot, c) in out.as_chunks_mut::<3>().0.iter_mut().zip(colors) {
        *slot = c.color;
    }
    // At most 256 colors: both numbers fit in two bytes.
    let count = colors.len() as u16;
    let transparent = colors.iter().position(|c| c.transparent).map_or(ACT_NO_TRANSPARENT, |i| i as u16);
    out[ACT_COLORS_LEN..ACT_COLORS_LEN + 2].copy_from_slice(&count.to_be_bytes());
    out[ACT_COLORS_LEN + 2..].copy_from_slice(&transparent.to_be_bytes());
    out
}

/// The colors of a color table file and its transparent entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActTable {
    /// The colors the file uses, in its order.
    pub colors: Vec<[u8; 3]>,
    /// The index of the transparent entry, when the file names one of `colors`.
    pub transparent: Option<usize>,
}

/// Read a color table file: 768 bytes are 256 colors; 772 bytes add the number of colors used
/// (1 to 256) and the transparent entry's index. A transparent index past the colors used names
/// no entry. Any other size, or a count of 0 or above 256, is an error.
pub fn read_act(bytes: &[u8]) -> std::result::Result<ActTable, String> {
    let (count, transparent) = match bytes.len() {
        ACT_COLORS_LEN => (256, None),
        ACT_LEN => {
            let at = |i: usize| usize::from(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
            (at(ACT_COLORS_LEN), Some(at(ACT_COLORS_LEN + 2)))
        }
        _ => return Err(NOT_A_TABLE.into()),
    };
    if !(1..=256).contains(&count) {
        return Err(NOT_A_TABLE.into());
    }
    let colors: Vec<[u8; 3]> = bytes[..count * 3].as_chunks::<3>().0.to_vec();
    Ok(ActTable { colors, transparent: transparent.filter(|i| *i < count) })
}

/// `settings` with the color table `t` loaded: its colors other than the transparent entry become
/// the fixed palette (`colorTable.customPalette`), and `colors` becomes the number of colors the
/// file uses (at least 2). The table's other edits (locked, transparent, deleted, shifted and
/// web-shifted colors, and its sort) are cleared and Web Snap is set to 0, so every loaded color is
/// written as it is, in the file's order. A table whose only color is its transparent entry loads
/// that color.
pub fn with_table(settings: &WebSettings, t: &ActTable) -> WebSettings {
    let mut s = settings.clone();
    let mut palette: Vec<String> = t.colors.iter().enumerate().filter(|(i, _)| Some(*i) != t.transparent).map(|(_, c)| hex(*c)).collect();
    if palette.is_empty() {
        palette = t.colors.iter().map(|c| hex(*c)).collect();
    }
    s.colors = (t.colors.len() as u16).clamp(2, 256);
    s.web_snap = 0;
    s.color_table = ColorTable { custom_palette: palette, ..ColorTable::default() };
    s
}

/// `cmd`'s error for a format without a color table.
fn no_table(cmd: &str, f: WebFormat) -> super::EngineError {
    let format = f.id();
    bad(cmd, format!("`{format}` has no color table: use gif or png8"))
}

fn save_color_table(s: &mut Session, p: &Value) -> Result<Value> {
    const P: &str = "document.exportForWeb.saveColorTable";
    let settings = s.web_settings(p).map_err(|e| bad(P, e))?;
    if !settings.format.palette() {
        return Err(no_table(P, settings.format));
    }
    let path = text_param(p, "path", P)?;
    if path.is_some_and(|p| p.trim().is_empty()) {
        return Err(bad(P, "`path` is empty: give a file path, or leave it out for `dataBase64`"));
    }
    let path = path.map(|p| if std::path::Path::new(p).extension().is_none() { format!("{p}.{COLOR_TABLE_EXT}") } else { p.to_string() });
    let Shown { o, warnings, .. } = shown(s, &settings, p, P, false)?;
    let bytes = act_bytes(&o.colors);
    let mut out = json!({
        "colors": o.colors.len().min(256),
        "transparentIndex": o.colors.iter().position(|c| c.transparent),
        "bytes": bytes.len(),
        "warnings": warnings,
    });
    match path {
        Some(path) => {
            super::fileio::write_file(&path, &bytes)?;
            out["path"] = json!(path);
        }
        None => out["dataBase64"] = json!(vectorcraft_format::base64_encode(&bytes)),
    }
    Ok(out)
}

fn load_color_table(s: &mut Session, p: &Value) -> Result<Value> {
    const L: &str = "webExport.loadColorTable";
    let settings = s.web_settings(p).map_err(|e| bad(L, e))?;
    if !settings.format.palette() {
        return Err(no_table(L, settings.format));
    }
    // One byte more than a color table shows a file that is too long.
    let bytes = match (text_param(p, "path", L)?, text_param(p, "dataBase64", L)?) {
        (Some(path), None) => super::fileio::read_file_head(path, ACT_LEN + 1)?,
        (None, Some(b64)) => {
            // Text past the base64 of the longest table makes the file too long.
            if b64.bytes().filter(|b| !b.is_ascii_whitespace()).count() > ACT_LEN.div_ceil(3) * 4 {
                return Err(bad(L, NOT_A_TABLE));
            }
            super::scripts::decode_head(b64, ACT_LEN + 1).ok_or_else(|| bad(L, "bad dataBase64"))?
        }
        _ => return Err(bad(L, "give exactly one of `path` and `dataBase64`")),
    };
    let t = read_act(&bytes).map_err(|e| bad(L, e))?;
    let loaded = with_table(&settings, &t);
    Ok(json!({
        "colors": t.colors.iter().map(|c| hex(*c)).collect::<Vec<_>>(),
        "transparentIndex": t.transparent,
        "settings": loaded.to_json(),
    }))
}

// ---------- Optimize to File Size ----------

/// The most trial settings `webExport.optimizeToFileSize` encodes. Each trial encodes every image
/// it measures once.
pub const OPTIMIZE_TRIALS: usize = 32;
/// The largest desired file size, in K (1 GiB).
pub const MAX_DESIRED_K: f64 = 1_048_576.0;

/// Where Optimize to File Size starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartWith {
    /// The current format, with its JPEG quality or palette colors lowered.
    Current,
    /// GIF or JPEG, by the image's content.
    Auto,
}

/// The pixels `px` (`[x0, y0, x1, y1]`) of `r` as an image: borrowed when `px` is the whole render,
/// else a copy.
fn file_pixels(r: &WebRender, px: [u32; 4]) -> std::borrow::Cow<'_, [u8]> {
    if px == [0, 0, r.width, r.height] { std::borrow::Cow::Borrowed(r.rgba.as_slice()) } else { std::borrow::Cow::Owned(crop(r, px)) }
}

/// The settings Optimize to File Size found.
#[derive(Clone, Debug)]
pub struct SizeFit {
    pub settings: WebSettings,
    /// The size the settings give the measured images.
    pub bytes: usize,
    /// Whether `bytes` is within the target.
    pub achieved: bool,
    /// The trial settings encoded.
    pub trials: usize,
}

/// The files a search measures and the trials it has left.
struct Trials<'a> {
    doc: &'a Document,
    r: &'a WebRender,
    /// The pixels (`[x0, y0, x1, y1]`) of each file in `r`.
    files: &'a [[u32; 4]],
    /// Measure the largest file (else the files together).
    each: bool,
    left: usize,
    used: usize,
}

impl Trials<'_> {
    /// The size `s` gives the files, encoded one at a time; `None` when no trial is left.
    fn size(&mut self, s: &WebSettings) -> std::result::Result<Option<usize>, String> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        self.used += 1;
        let mut out = 0usize;
        for &px in self.files {
            let n = optimize(self.doc, &file_pixels(self.r, px), px[2] - px[0], px[3] - px[1], s)?.bytes.len();
            out = if self.each { out.max(n) } else { out.saturating_add(n) };
        }
        Ok(Some(out))
    }
}

/// `(settings, bytes)` of a trial.
pub type Trial = (WebSettings, usize);

/// Keep `(s, n)` in `smallest` when it is the smallest trial so far.
fn keep_smallest(smallest: &mut Option<Trial>, s: WebSettings, n: usize) {
    if smallest.as_ref().is_none_or(|(_, b)| n < *b) {
        *smallest = Some((s, n));
    }
}

/// The values [`highest_within`] measures spread over its range before it narrows a step.
pub const OPTIMIZE_SAMPLES: u16 = 6;

/// Measures the size a setting gives; `None` when no trial is left.
pub type MeasureSize<'a> = dyn FnMut(&WebSettings) -> std::result::Result<Option<usize>, String> + 'a;

/// The highest value in `lo..hi` that `set` gives `start` whose size is within `target`, when the
/// value `hi` is over it. A lower value can give a larger file, so the search measures up to
/// [`OPTIMIZE_SAMPLES`] values spread over the range, from `hi - 1` down, stops at the first that
/// fits, and halves the step between it and the measured value above it, keeping the highest value
/// measured within `target`. `None` when no value measured fits or no trial is left; the trials over
/// the target go to `smallest`.
pub fn highest_within(
    size: &mut MeasureSize,
    start: &WebSettings,
    (lo, hi): (u16, u16),
    set: fn(&mut WebSettings, u16),
    target: usize,
    smallest: &mut Option<Trial>,
) -> std::result::Result<Option<Trial>, String> {
    let with = |v: u16| {
        let mut s = start.clone();
        set(&mut s, v);
        s
    };
    if lo >= hi {
        return Ok(None);
    }
    let span = u32::from(hi - lo);
    let samples = u32::from(OPTIMIZE_SAMPLES).min(span);
    let (mut above, mut found) = (hi, None);
    for i in (0..samples).rev() {
        let step = if samples == 1 { 0 } else { (span - 1) * i / (samples - 1) };
        let v = lo.saturating_add(u16::try_from(step).unwrap_or(0));
        let s = with(v);
        let Some(n) = size(&s)? else { return Ok(None) };
        if n <= target {
            found = Some((v, (s, n)));
            break;
        }
        keep_smallest(smallest, s, n);
        above = v;
    }
    let Some((mut lo, mut best)) = found else { return Ok(None) };
    let mut hi = above;
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let s = with(mid);
        let Some(n) = size(&s)? else { break };
        if n <= target {
            lo = mid;
            best = (s, n);
        } else {
            keep_smallest(smallest, s, n);
            hi = mid;
        }
    }
    Ok(Some(best))
}

/// Sets the JPEG quality, for [`highest_within`].
pub fn set_quality(s: &mut WebSettings, v: u16) {
    s.quality = u8::try_from(v).unwrap_or(100);
}

/// Sets the palette colors, for [`highest_within`].
pub fn set_colors(s: &mut WebSettings, v: u16) {
    s.colors = v;
}

/// The best settings of `start`'s format within `target`: `start` when it fits, else the highest
/// JPEG quality or the most palette colors [`highest_within`] finds that fit, and for GIF, when none
/// is found, lossy 100 with the most colors that fit. When nothing tried fits, the smallest trial, not achieved.
/// `None` when no trial is left.
fn search_format(t: &mut Trials, start: &WebSettings, target: usize) -> std::result::Result<Option<SizeFit>, String> {
    let fit = |(settings, bytes): Trial, achieved: bool| SizeFit { settings, bytes, achieved, trials: 0 };
    let Some(n) = t.size(start)? else { return Ok(None) };
    if n <= target {
        return Ok(Some(fit((start.clone(), n), true)));
    }
    let mut smallest = Some((start.clone(), n));
    let found = match start.format {
        WebFormat::Jpg => highest_within(&mut |s: &WebSettings| t.size(s), start, (1, u16::from(start.quality)), set_quality, target, &mut smallest)?,
        WebFormat::Gif | WebFormat::Png8 => {
            match highest_within(&mut |s: &WebSettings| t.size(s), start, (2, start.colors), set_colors, target, &mut smallest)? {
                None if start.format == WebFormat::Gif && start.lossy < 100 => {
                    let lossy = WebSettings { lossy: 100, ..start.clone() };
                    match t.size(&lossy)? {
                        Some(n) if n <= target => Some((lossy, n)),
                        Some(n) => {
                            keep_smallest(&mut smallest, lossy.clone(), n);
                            highest_within(&mut |s: &WebSettings| t.size(s), &lossy, (2, lossy.colors), set_colors, target, &mut smallest)?
                        }
                        None => None,
                    }
                }
                found => found,
            }
        }
        WebFormat::Png24 => None,
    };
    Ok(match (found, smallest) {
        (Some(f), _) => Some(fit(f, true)),
        (None, Some(s)) => Some(fit(s, false)),
        (None, None) => None,
    })
}

/// The format Auto Select GIF/JPEG starts with, and whether the files (`[x0, y0, x1, y1]` of `r`)
/// are opaque: GIF for files with transparent pixels (when `transparency` keeps them) or at most
/// 256 colors, else JPEG. The colors counted are the ones the files are written with: without
/// `transparency`, the pixels blended over `matte`.
fn auto_format(r: &WebRender, files: &[[u32; 4]], transparency: bool, matte: [u8; 3]) -> (WebFormat, bool) {
    let stride = r.width as usize * 4;
    let row =
        move |y: u32, x0: u32, x1: u32| r.rgba.get(y as usize * stride + x0 as usize * 4..y as usize * stride + x1 as usize * 4).unwrap_or_default();
    let pixels = || files.iter().flat_map(move |&[x0, y0, x1, y1]| (y0..y1).flat_map(move |y| row(y, x0, x1).as_chunks::<4>().0.iter()));
    let opaque = !transparency || pixels().all(|p| p[3] == 255);
    if !opaque {
        return (WebFormat::Gif, false);
    }
    let mut seen = HashSet::new();
    // An opaque pixel blends to itself.
    let few = pixels().all(|p| {
        seen.insert(flatten(p, matte).next().unwrap_or_default());
        seen.len() <= 256
    });
    (if few { WebFormat::Gif } else { WebFormat::Jpg }, true)
}

/// Optimize to File Size: the settings, from `s`, whose files (the pixels `[x0, y0, x1, y1]` of
/// `r`) fit in `target` bytes, measured together, or with `each` the largest one. At most
/// [`OPTIMIZE_TRIALS`] trial settings are encoded, and a trial encodes the files one at a time.
pub fn optimize_to_size(
    doc: &Document,
    r: &WebRender,
    files: &[[u32; 4]],
    s: &WebSettings,
    start: StartWith,
    each: bool,
    target: usize,
) -> std::result::Result<SizeFit, String> {
    let mut t = Trials { doc, r, files, each, left: OPTIMIZE_TRIALS, used: 0 };
    let with = |format: WebFormat| WebSettings { format, ..s.clone() };
    let found = match start {
        _ if files.is_empty() => None,
        StartWith::Current => search_format(&mut t, s, target)?,
        StartWith::Auto => {
            let (first, opaque) = auto_format(r, files, s.transparency, s.matte_rgb()?.unwrap_or([255; 3]));
            let other = if first == WebFormat::Gif { WebFormat::Jpg } else { WebFormat::Gif };
            match search_format(&mut t, &with(first), target)? {
                Some(a) if !a.achieved && opaque => match search_format(&mut t, &with(other), target)? {
                    Some(b) if b.achieved || b.bytes < a.bytes => Some(b),
                    _ => Some(a),
                },
                a => a,
            }
        }
    };
    let mut best = found.ok_or("Optimize to File Size: there is no image to measure")?;
    best.trials = t.used;
    Ok(best)
}

fn optimize_to_file_size(s: &mut Session, p: &Value) -> Result<Value> {
    const O: &str = "webExport.optimizeToFileSize";
    let k = p
        .get("desiredK")
        .and_then(Value::as_f64)
        .filter(|k| k.is_finite() && *k > 0.0 && *k <= MAX_DESIRED_K)
        .ok_or_else(|| bad(O, "give `desiredK`, the desired file size in K (1024 bytes): a number above 0 and at most 1048576"))?;
    let target = ((k * 1024.0).floor() as usize).max(1);
    let start = match p.get("startWith").map(Value::as_str) {
        None | Some(Some("current")) => StartWith::Current,
        Some(Some("auto")) => StartWith::Auto,
        Some(_) => return Err(bad(O, "`startWith` is `current` or `auto`")),
    };
    let scope = match p.get("use").map(Value::as_str) {
        None if p.get("slice").is_some() => "currentSlice",
        None => "allSlices",
        Some(Some(u @ ("allSlices" | "eachSlice" | "currentSlice"))) => u,
        Some(_) => return Err(bad(O, "`use` is `allSlices`, `eachSlice` or `currentSlice`")),
    };
    let slice = match p.get("slice").map(|v| v.as_u64().filter(|n| *n >= 1 && scope == "currentSlice")) {
        None => None,
        Some(Some(n)) => Some(n),
        Some(None) => return Err(bad(O, "`slice` is a slice number from `slice.list`, with `use: currentSlice`")),
    };
    let settings = s.web_settings(p).map_err(|e| bad(O, e))?;
    let st = s.doc()?;
    let doc = &st.doc;
    let failed = vectorcraft_render::decode_failures();
    let r = render(doc, &settings).map_err(|e| bad(O, e))?;
    let warnings = undecoded(failed);
    let areas = doc.slice_layout();
    let selected = super::slices::selected_slices(doc, &st.selection);
    let cells = cells(&areas, &selected, &settings, &r).map_err(|e| bad(O, e))?;
    let image_cells: Vec<&Cell> =
        cells.iter().filter(|c| c.area.id.and_then(|id| doc.slice_options(id)).is_none_or(|o| o.kind == SliceKind::Image)).collect();
    let picked: Vec<&Cell> = match (scope, slice) {
        ("currentSlice", Some(n)) => {
            let c = image_cells.iter().find(|c| c.area.number as u64 == n);
            vec![*c.ok_or_else(|| bad(O, format!("no slice {n} in the image (see slice.list)")))?]
        }
        ("currentSlice", None) => {
            image_cells.iter().find(|c| c.area.id.is_some_and(|id| selected.contains(&id))).or(image_cells.first()).copied().into_iter().collect()
        }
        _ => image_cells,
    };
    let files: Vec<[u32; 4]> = if cells.is_empty() { vec![[0, 0, r.width, r.height]] } else { picked.iter().map(|c| c.px).collect() };
    let fit = optimize_to_size(doc, &r, &files, &settings, start, scope == "eachSlice", target).map_err(|e| bad(O, e))?;
    Ok(json!({
        "targetBytes": target, "achieved": fit.achieved, "bytes": fit.bytes, "format": fit.settings.format.id(),
        "settings": fit.settings.to_json(), "trials": fit.trials, "images": files.len(), "warnings": warnings,
    }))
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    const S: &str = "webExport.settings";
    let base = if bool_or(p, "reset", false) { WebSettings::default() } else { s.last_web_settings() };
    let next = s.web_settings_over(&base, p).map_err(|e| bad(S, e))?;
    if p.as_object().is_some_and(|o| !o.is_empty()) {
        s.prefs.web_export_settings = Some(next.clone());
    }
    Ok(json!({ "settings": next.to_json() }))
}

fn presets_list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows = builtin_presets()
        .into_iter()
        .map(|p| (p, true))
        .chain(s.prefs.web_export_presets.iter().cloned().map(|p| (p, false)))
        .map(|(p, builtin)| json!({ "name": p.name, "builtIn": builtin, "settings": p.settings.to_json() }));
    Ok(json!({ "presets": rows.collect::<Vec<_>>() }))
}

fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    const S: &str = "webExport.presets.save";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(S, "give the preset a `name`"))?.to_string();
    if is_builtin(&name) {
        return Err(bad(S, format!("`{name}` is a built-in preset and can't change: save it under another name")));
    }
    let at = saved_index(&s.prefs.web_export_presets, &name);
    let base = at.and_then(|i| s.prefs.web_export_presets.get(i)).map(|q| q.settings.clone()).unwrap_or_default();
    // Settings at the top level, or in `settings`.
    let mut keys = p.as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(inner)) = keys.remove("settings") {
        keys.extend(inner);
    }
    let settings = s.web_settings_over(&base, &Value::Object(keys)).map_err(|e| bad(S, e))?;
    let name = match str_param(p, "newName").map(str::trim) {
        Some("") => return Err(bad(S, "`newName` is empty")),
        Some(n) if is_builtin(n) || saved_index(&s.prefs.web_export_presets, n).is_some_and(|i| Some(i) != at) => {
            return Err(bad(S, format!("a preset named `{n}` exists")));
        }
        Some(n) => n.to_string(),
        None => at.and_then(|i| s.prefs.web_export_presets.get(i)).map_or(name, |q| q.name.clone()),
    };
    let preset = WebPreset { name: name.clone(), settings: settings.clone() };
    match at.and_then(|i| s.prefs.web_export_presets.get_mut(i)) {
        Some(slot) => *slot = preset,
        None => s.prefs.web_export_presets.push(preset),
    }
    Ok(json!({ "name": name, "settings": settings.to_json(), "created": at.is_none() }))
}

fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const S: &str = "webExport.presets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(S, "missing `name`"))?;
    if is_builtin(name) {
        return Err(bad(S, format!("`{name}` is a built-in preset and stays")));
    }
    let i = saved_index(&s.prefs.web_export_presets, name).ok_or_else(|| bad(S, format!("no saved preset named `{name}`")))?;
    Ok(json!({ "deleted": s.prefs.web_export_presets.remove(i).name }))
}
