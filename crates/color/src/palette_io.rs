//! Swatch library files: the native `.vcswatches` JSON (colour models, global, spot, gradients
//! and colour groups exactly), `.gpl` palettes (8-bit RGB; colour groups as `# Group:` comment
//! headers), swatch exchange `.ase` files (binary: solid colors in their own model, global, spot
//! and process colors, color groups), color books `.acb` (read only: the books a user owns, such
//! as a Pantone book) and CSS custom properties (written only).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{Color, GradientKind, Paint, Swatch, SwatchGroup, SwatchLibrary};

/// A swatch library file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteFormat {
    /// `.vcswatches`: JSON, lossless.
    Native,
    /// `.gpl`: a plain-text RGB palette many paint and design tools read.
    Gpl,
    /// `.ase`: a swatch exchange file (binary): solid colors in their own model, global, spot or
    /// process, and color groups.
    Ase,
    /// `.css`: custom properties on `:root` (colours and gradients; written only).
    Css,
}

impl PaletteFormat {
    pub const ALL: [PaletteFormat; 4] = [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Ase, PaletteFormat::Css];

    /// The format's id, which is also its extension.
    pub fn id(self) -> &'static str {
        match self {
            PaletteFormat::Native => "vcswatches",
            PaletteFormat::Gpl => "gpl",
            PaletteFormat::Ase => "ase",
            PaletteFormat::Css => "css",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaletteFormat::Native => "VectorCraft Swatches (.vcswatches)",
            PaletteFormat::Gpl => "GPL Palette (.gpl)",
            PaletteFormat::Ase => "Swatch Exchange (.ase)",
            PaletteFormat::Css => "CSS Custom Properties (.css)",
        }
    }
    /// The format with id or extension `s` (any case, leading dot allowed).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('.').to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.id() == s)
    }
    /// Can libraries be read back from this format?
    pub fn readable(self) -> bool {
        self != PaletteFormat::Css
    }
    /// Are files of this format binary rather than text?
    pub fn binary(self) -> bool {
        self == PaletteFormat::Ase
    }
    /// Does [`write`] keep a swatch with `paint` in a file of this format? None and patterns are
    /// never written; `.gpl` and `.ase` hold solid colors only, and CSS holds no freeform gradients.
    pub fn holds(self, paint: &Paint) -> bool {
        match paint {
            Paint::Solid { .. } => true,
            Paint::Gradient(g) => match self {
                PaletteFormat::Native => true,
                PaletteFormat::Css => g.gradient.kind != GradientKind::Freeform,
                PaletteFormat::Gpl | PaletteFormat::Ase => false,
            },
            Paint::None | Paint::Pattern { .. } => false,
        }
    }
}

/// The native file: a header around the library.
#[derive(Serialize, Deserialize)]
struct NativeFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    library: SwatchLibrary,
}

const NATIVE_FORMAT: &str = "vcswatches";
const GPL_HEADER: &str = "GIMP Palette";
const GPL_GROUP: &str = "# Group:";
const ASE_SIGNATURE: &[u8] = b"ASEF";
const ASE_GROUP_START: u16 = 0xC001;
const ASE_GROUP_END: u16 = 0xC002;
const ASE_COLOR: u16 = 0x0001;
/// The most colors and groups read from one `.ase` file.
const ASE_MAX_ENTRIES: usize = 100_000;
/// The most UTF-16 code units of a name that are kept.
const ASE_MAX_NAME: usize = 1024;
const ASE_CUT_SHORT: &str = "the swatch exchange file is cut short";
const ACB_SIGNATURE: &[u8] = b"8BCB";
const ACB_CUT_SHORT: &str = "the color book is cut short";

/// Write `lib` in `format` → the file's bytes. Pattern swatches (their tiles live in a document)
/// and None are left out; `.gpl` keeps solid colours only, as 8-bit RGB, and `.ase` keeps solid
/// colors in their own model. Writing `.ase` fails for a library with more colors and groups than
/// [`read_bytes`] reads from one file.
pub fn write(lib: &SwatchLibrary, format: PaletteFormat) -> Result<Vec<u8>, String> {
    let keep = |w: &&Swatch| matches!(w.paint, Paint::Solid { .. } | Paint::Gradient(_));
    let text = match format {
        PaletteFormat::Native => {
            let strip = |list: &[Swatch]| list.iter().filter(keep).cloned().collect::<Vec<_>>();
            let library = SwatchLibrary {
                name: lib.name.clone(),
                swatches: strip(&lib.swatches),
                groups: lib.groups.iter().map(|g| SwatchGroup { name: g.name.clone(), swatches: strip(&g.swatches) }).collect(),
            };
            let file = NativeFile { format: NATIVE_FORMAT.into(), version: 1, library };
            serde_json::to_string_pretty(&file).unwrap_or_default()
        }
        PaletteFormat::Gpl => write_gpl(lib),
        PaletteFormat::Ase => return write_ase(lib),
        PaletteFormat::Css => write_css(lib, keep),
    };
    Ok(text.into_bytes())
}

/// One line of text: no line breaks or tabs.
fn one_line(s: &str) -> String {
    s.split(['\n', '\r', '\t']).collect::<Vec<_>>().join(" ").trim().to_string()
}

fn write_gpl(lib: &SwatchLibrary) -> String {
    let mut out = format!("{GPL_HEADER}\nName: {}\nColumns: 0\n#\n", one_line(&lib.name));
    let colors = |out: &mut String, list: &[Swatch]| {
        for w in list {
            if let Some(c) = w.paint.color() {
                let [r, g, b, _] = c.to_rgba8(1.0);
                out.push_str(&format!("{r:3} {g:3} {b:3}\t{}\n", one_line(&w.name)));
            }
        }
    };
    colors(&mut out, &lib.swatches);
    for g in &lib.groups {
        out.push_str(&format!("{GPL_GROUP} {}\n", one_line(&g.name)));
        colors(&mut out, &g.swatches);
    }
    out
}

/// `name` as a CSS custom property (`--` and an escaped identifier): lower case, spaces as
/// hyphens, other characters an identifier can't hold escaped with a backslash.
pub fn css_property(name: &str) -> String {
    let mut out = String::from("--");
    for word in name.split_whitespace() {
        if out.len() > 2 {
            out.push('-');
        }
        for ch in word.chars().flat_map(char::to_lowercase) {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || !ch.is_ascii() {
                out.push(ch);
            } else if ch.is_ascii_graphic() {
                out.push('\\');
                out.push(ch);
            } else {
                out.push_str(&format!("\\{:x} ", ch as u32));
            }
        }
    }
    if out.len() == 2 {
        out.push_str("swatch");
    }
    out
}

/// A colour as CSS (`#rrggbb`, or `#rrggbbaa` below full opacity).
fn css_color(c: &Color, opacity: f32) -> String {
    let [r, g, b, a] = c.to_rgba8(opacity);
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// A comment's text: `*/` can't end it early.
fn css_comment(s: &str) -> String {
    one_line(s).replace("*/", "* /")
}

fn write_css(lib: &SwatchLibrary, keep: impl Fn(&&Swatch) -> bool) -> String {
    let mut out = format!("/* {} */\n:root {{\n", css_comment(&lib.name));
    let mut taken: Vec<String> = vec![];
    let mut props = |out: &mut String, list: &[Swatch]| {
        for w in list.iter().filter(&keep) {
            let value = match &w.paint {
                Paint::Solid { color, .. } => css_color(color, 1.0),
                Paint::Gradient(g) if g.gradient.kind != GradientKind::Freeform => {
                    let stops: Vec<String> =
                        g.gradient.stops.iter().map(|s| format!("{} {}%", css_color(&s.color, s.opacity), (s.offset * 100.0).round())).collect();
                    match g.gradient.kind {
                        GradientKind::Radial => format!("radial-gradient(circle, {})", stops.join(", ")),
                        _ => format!("linear-gradient({}deg, {})", (90.0 - g.angle).rem_euclid(360.0).round(), stops.join(", ")),
                    }
                }
                _ => continue,
            };
            // Names differing only in case or punctuation map to one property: number the others.
            let base = css_property(&w.name);
            let name = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base}-{i}") }).find(|n| !taken.contains(n)).unwrap_or(base);
            taken.push(name.clone());
            let note = if w.spot { " /* spot */" } else { "" };
            out.push_str(&format!("  {name}: {value};{note}\n"));
        }
    };
    props(&mut out, &lib.swatches);
    for g in &lib.groups {
        out.push_str(&format!("  /* {} */\n", css_comment(&g.name)));
        props(&mut out, &g.swatches);
    }
    out.push_str("}\n");
    out
}

/// `.ase`, as [`read_ase`] reads it: version 1.0, the ungrouped colors, then each color group as a
/// group start (its name), its colors and a group end, also for a group without colors. A color is
/// written in its own model: RGB, CMYK and gray components from 0 to 1 (gray as a level, 1 is
/// white), Lab lightness as a fraction of 100 and a and b from −128 to 127. Components are clamped
/// to those ranges, and a component that isn't a finite number is written as 0. The color type is
/// 1 for a spot color, 0 for another global color and 2 for a process color.
///
/// A swatch exchange file holds solid colors only: gradients, patterns and None are left out, and
/// a tint swatch is written as the color it shows, a process color without its link to its base.
/// The file has no library name ([`read_ase`] names a library after its file). More than
/// [`ASE_MAX_ENTRIES`] colors and groups is an error.
fn write_ase(lib: &SwatchLibrary) -> Result<Vec<u8>, String> {
    let solid = |list: &[Swatch]| list.iter().filter(|w| w.paint.color().is_some()).count();
    let entries = solid(&lib.swatches) + lib.groups.iter().map(|g| 1 + solid(&g.swatches)).sum::<usize>();
    if entries > ASE_MAX_ENTRIES {
        return Err(format!(
            "VectorCraft reads at most {ASE_MAX_ENTRIES} colors and groups from a swatch exchange file, and this library has {entries}"
        ));
    }
    let mut body = vec![];
    let mut blocks = put_ase_colors(&mut body, &lib.swatches)?;
    for g in &lib.groups {
        let mut name = vec![];
        put_ase_name(&mut name, &g.name)?;
        put_ase_block(&mut body, ASE_GROUP_START, &name)?;
        blocks += 2 + put_ase_colors(&mut body, &g.swatches)?;
        put_ase_block(&mut body, ASE_GROUP_END, &[])?;
    }
    let blocks = u32::try_from(blocks).map_err(|_| "too many swatch exchange blocks".to_string())?;
    let mut out = ASE_SIGNATURE.to_vec();
    out.extend(1u16.to_be_bytes());
    out.extend(0u16.to_be_bytes());
    out.extend(blocks.to_be_bytes());
    out.extend(body);
    Ok(out)
}

/// The color blocks of the solid colors in `list` → how many were written.
fn put_ase_colors(out: &mut Vec<u8>, list: &[Swatch]) -> Result<usize, String> {
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let ab = |v: f32| if v.is_finite() { v.clamp(-128.0, 127.0) } else { 0.0 };
    let mut n = 0;
    for (w, color) in list.iter().filter_map(|w| Some((w, w.paint.color()?))) {
        let mut body = vec![];
        put_ase_name(&mut body, &w.name)?;
        let (model, values): (&[u8; 4], Vec<f32>) = match color {
            Color::Rgb { r, g, b } => (b"RGB ", vec![unit(r), unit(g), unit(b)]),
            Color::Cmyk { c, m, y, k } => (b"CMYK", vec![unit(c), unit(m), unit(y), unit(k)]),
            Color::Lab { l, a, b } => (b"LAB ", vec![unit(l / 100.0), ab(a), ab(b)]),
            Color::Gray { k } => (b"Gray", vec![1.0 - unit(k)]),
        };
        body.extend(model);
        body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
        let kind: u16 = if w.spot {
            1
        } else if w.global {
            0
        } else {
            2
        };
        body.extend(kind.to_be_bytes());
        put_ase_block(out, ASE_COLOR, &body)?;
        n += 1;
    }
    Ok(n)
}

/// A block: its type, the length of its body and the body.
fn put_ase_block(out: &mut Vec<u8>, kind: u16, body: &[u8]) -> Result<(), String> {
    let len = u32::try_from(body.len()).map_err(|_| "a swatch exchange block is too long".to_string())?;
    out.extend(kind.to_be_bytes());
    out.extend(len.to_be_bytes());
    out.extend_from_slice(body);
    Ok(())
}

/// A name as `.ase` stores it: a `u16` count of UTF-16 code units, the terminating zero included,
/// then the units. NUL characters are left out, and a name longer than [`ASE_MAX_NAME`] code units
/// is cut before the character that would pass that limit.
fn put_ase_name(out: &mut Vec<u8>, name: &str) -> Result<(), String> {
    let mut units: Vec<u16> = vec![];
    for c in name.chars().filter(|&c| c != '\0') {
        let mut buf = [0; 2];
        let encoded = c.encode_utf16(&mut buf);
        if units.len() + encoded.len() > ASE_MAX_NAME {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units.push(0);
    let count = u16::try_from(units.len()).map_err(|_| "a swatch exchange name is too long".to_string())?;
    out.extend(count.to_be_bytes());
    out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    Ok(())
}

/// Read a `.vcswatches` or `.gpl` library (detected from its content). `name` names it when the
/// file doesn't.
pub fn read(text: &str, name: &str) -> Result<SwatchLibrary, String> {
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with(GPL_HEADER) {
        return Ok(read_gpl(text, name));
    }
    if text.starts_with('{') {
        let f: NativeFile = serde_json::from_str(text).map_err(|e| format!("not a swatch library: {e}"))?;
        if f.format != NATIVE_FORMAT {
            return Err(format!("not a swatch library (format `{}`)", f.format));
        }
        let mut lib = f.library;
        if lib.name.trim().is_empty() {
            lib.name = name.into();
        }
        return Ok(lib);
    }
    Err("not a swatch library (.vcswatches or .gpl)".into())
}

/// Is `text` a library [`read`] understands?
pub fn sniff(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with(GPL_HEADER) || (t.starts_with('{') && t.contains(NATIVE_FORMAT))
}

/// Read the bytes of a library file: a swatch exchange `.ase` file, else a `.vcswatches` or
/// `.gpl` library ([`read`]). `name` names it when the file doesn't.
pub fn read_bytes(bytes: &[u8], name: &str) -> Result<SwatchLibrary, String> {
    if bytes.starts_with(ASE_SIGNATURE) {
        return read_ase(bytes, name);
    }
    if bytes.starts_with(ACB_SIGNATURE) {
        return read_acb(bytes, name).map_err(|e| if e == ASE_CUT_SHORT { ACB_CUT_SHORT.into() } else { e });
    }
    read(&String::from_utf8_lossy(bytes), name)
}

/// Are `bytes` a library [`read_bytes`] understands? Text that isn't valid UTF-8 is checked as
/// [`read_bytes`] reads it, with the invalid bytes replaced (its first KiB is enough).
pub fn sniff_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(ASE_SIGNATURE)
        || bytes.starts_with(ACB_SIGNATURE)
        || std::str::from_utf8(bytes).is_ok_and(sniff)
        || sniff(&String::from_utf8_lossy(bytes.get(..1024).unwrap_or(bytes)))
}

/// `.gpl`: `Name:` names the library, `# Group: name` starts a colour group, other `#` lines and
/// headers (`Columns:`) are skipped, as are lines that aren't `r g b [name]`. Unnamed colors are
/// named by their values and unnamed groups "Color Group"; a name a color or group already has gets
/// a number ([`UniqueNames`]).
fn read_gpl(text: &str, fallback: &str) -> SwatchLibrary {
    let mut lib = SwatchLibrary { name: fallback.into(), ..Default::default() };
    let mut names = UniqueNames::default();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if let Some(g) = line.strip_prefix(GPL_GROUP) {
            lib.groups.push(SwatchGroup { name: names.unique(group_name(g)), swatches: vec![] });
            continue;
        }
        if let Some(n) = line.strip_prefix("Name:") {
            lib.name = n.trim().to_string();
            continue;
        }
        // Comments, headers (`Columns: 4`) and anything else that isn't a colour are skipped.
        let words: Vec<&str> = line.split_whitespace().collect();
        let rgb: Vec<u8> = words.iter().take(3).map_while(|v| v.parse().ok()).collect();
        let [r, g, b] = rgb[..] else { continue };
        let rest = words[3..].join(" ");
        let base = if rest.is_empty() { format!("R={r} G={g} B={b}") } else { rest };
        let w = Swatch { name: names.unique(base), paint: Paint::solid(Color::rgb8(r, g, b)), global: false, spot: false };
        match lib.groups.last_mut() {
            Some(grp) => grp.swatches.push(w),
            None => lib.swatches.push(w),
        }
    }
    lib
}

/// `.ase`, from public descriptions of the format: the layout from
/// <https://www.selapa.net/swatches/colors/fileformats.php>, the component ranges from
/// <https://github.com/nsfmc/swatch>. After the signature come, big-endian, a `u16` major (1) and
/// minor version, a `u32` block count and the blocks, each a `u16` type, a `u32` body length and
/// the body. A group start's body is a name. A color's body is a name, a four-character model
/// (`RGB `, `CMYK`, `LAB `, `Gray`), its components as `f32` (3, 4, 3 and 1 of them) and a `u16`
/// color type (0 global, 1 spot, 2 process). RGB, CMYK and gray components run from 0 to 1 (a
/// gray level of 1 is white), Lab lightness from 0 to 1 and a and b from −128 to 127. A lightness
/// above 1 is read as L* itself. A name is a `u16` count of UTF-16 code units, the terminating zero
/// included, then the units.
///
/// Color groups become groups: a group start inside an open group ends that group, and a group
/// left open ends with the file. Blocks of other types and colors in other models are skipped.
/// Unnamed colors are named by their values and unnamed groups "Color Group"; a name a color or
/// group already has gets a number ([`UniqueNames`]). A truncated file, a major version other than
/// 1 or more than [`ASE_MAX_ENTRIES`] colors and groups is an error.
fn read_ase(bytes: &[u8], fallback: &str) -> Result<SwatchLibrary, String> {
    let mut data = AseData(bytes);
    data.bytes(ASE_SIGNATURE.len())?;
    let (major, minor) = (data.u16()?, data.u16()?);
    if major != 1 {
        return Err(format!("swatch exchange version {major}.{minor} isn't supported (only 1.x)"));
    }
    let blocks = data.u32()?;
    let mut lib = SwatchLibrary { name: fallback.into(), ..Default::default() };
    let mut names = UniqueNames::default();
    let (mut in_group, mut entries) = (false, 0usize);
    for _ in 0..blocks {
        let kind = data.u16()?;
        let len = usize::try_from(data.u32()?).unwrap_or(usize::MAX);
        let mut body = AseData(data.bytes(len)?);
        if matches!(kind, ASE_GROUP_START | ASE_COLOR) {
            entries += 1;
            if entries > ASE_MAX_ENTRIES {
                return Err(format!("the swatch exchange file holds more than {ASE_MAX_ENTRIES} colors and groups"));
            }
        }
        match kind {
            ASE_GROUP_START => {
                lib.groups.push(SwatchGroup { name: names.unique(group_name(&body.name()?)), swatches: vec![] });
                in_group = true;
            }
            ASE_GROUP_END => in_group = false,
            ASE_COLOR => {
                let Some(mut w) = ase_swatch(&mut body)? else { continue };
                w.name = names.unique(std::mem::take(&mut w.name));
                match lib.groups.last_mut().filter(|_| in_group) {
                    Some(g) => g.swatches.push(w),
                    None => lib.swatches.push(w),
                }
            }
            // Other blocks are skipped.
            _ => {}
        }
    }
    Ok(lib)
}

/// `.acb` (a color book, read only), from public descriptions of the format: after the signature
/// come, big-endian, a `u16` version (1) and book id, the title, prefix, suffix and description
/// as strings, a `u16` color count, page size and page selector offset, a `u16` color space (0
/// RGB, 2 CMYK, 7 Lab) and the colors: each a name, a six-character catalog code and its
/// components as bytes. RGB runs from 0 to 255, CMYK ink is stored inverted (255 is no ink), Lab
/// lightness is 0 to 255 for 0 to 100 and a and b are offset by 128. A string is a `u32` count of
/// UTF-16 code units, then the units; a `$$$/key=Text` string means `Text`, and `^R`, `^C` stand
/// for ® and ©. An optional `spflspot` or `spflproc` at the end says whether the colors are spot
/// or process colors (a book without it is a spot color book).
///
/// The book's colors are named with its prefix and suffix ("PANTONE 185 C"); colors without a
/// name (the padding of a book's pages) are left out. A version other than 1, a color space other
/// than those three or more than [`ASE_MAX_ENTRIES`] colors is an error.
fn read_acb(bytes: &[u8], fallback: &str) -> Result<SwatchLibrary, String> {
    let mut data = AseData(bytes);
    data.bytes(ACB_SIGNATURE.len())?;
    let version = data.u16()?;
    if version != 1 {
        return Err(format!("color book version {version} isn't supported (only 1)"));
    }
    let _book_id = data.u16()?;
    let [title, prefix, suffix, _description] = [data.acb_string()?, data.acb_string()?, data.acb_string()?, data.acb_string()?];
    let count = usize::from(data.u16()?);
    if count > ASE_MAX_ENTRIES {
        return Err(format!("the color book holds more than {ASE_MAX_ENTRIES} colors"));
    }
    let (_page_size, _page_offset) = (data.u16()?, data.u16()?);
    let space = data.u16()?;
    let mut colors = Vec::with_capacity(count);
    for _ in 0..count {
        let name = data.acb_string()?;
        let _code = data.bytes(6)?;
        let color = match space {
            0 => {
                let [r, g, b] = data.array::<3>()?;
                Color::rgb8(r, g, b)
            }
            2 => {
                let ink = |v: u8| f32::from(255 - v) / 255.0;
                let [c, m, y, k] = data.array::<4>()?;
                Color::cmyk(ink(c), ink(m), ink(y), ink(k))
            }
            7 => {
                let [l, a, b] = data.array::<3>()?;
                Color::lab(f32::from(l) * 100.0 / 255.0, f32::from(a) - 128.0, f32::from(b) - 128.0)
            }
            other => return Err(format!("color books in color space {other} aren't supported (only RGB, CMYK and Lab)")),
        };
        colors.push((name, color));
    }
    // Spot colors unless the book says process ones.
    let spot = data.0.windows(8).last() != Some(b"spflproc".as_slice());
    let mut lib = SwatchLibrary { name: if title.is_empty() { fallback.into() } else { title }, ..Default::default() };
    let mut names = UniqueNames::default();
    for (name, color) in colors.into_iter().filter(|(n, _)| !n.trim().is_empty()) {
        let name = names.unique(format!("{prefix}{name}{suffix}").trim().to_string());
        lib.swatches.push(Swatch { name, paint: Paint::solid(color), global: spot, spot });
    }
    Ok(lib)
}

/// The swatch of an `.ase` color block, `None` for a model other than RGB, CMYK, Lab and Gray.
/// Components are clamped to their ranges.
fn ase_swatch(body: &mut AseData) -> Result<Option<Swatch>, String> {
    let name = body.name()?;
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let color = match &body.array::<4>()? {
        b"RGB " => {
            let [r, g, b] = [body.f32()?, body.f32()?, body.f32()?];
            Color::rgb(unit(r), unit(g), unit(b))
        }
        b"CMYK" => {
            let [c, m, y, k] = [body.f32()?, body.f32()?, body.f32()?, body.f32()?];
            Color::cmyk(unit(c), unit(m), unit(y), unit(k))
        }
        // Lightness is stored as a fraction of 100. Some writers store L* itself, which shows as a
        // value above 1.
        b"LAB " => {
            let [l, a, b] = [body.f32()?, body.f32()?, body.f32()?];
            let l = if l > 1.0 { l.min(100.0) } else { unit(l) * 100.0 };
            Color::lab(l, a.clamp(-128.0, 127.0), b.clamp(-128.0, 127.0))
        }
        // A gray level (1 is white); a gray color holds its ink.
        b"Gray" => Color::gray(1.0 - unit(body.f32()?)),
        _ => return Ok(None),
    };
    // Some writers leave the color type out, which makes a process color.
    let kind = if body.0.len() >= 2 { body.u16()? } else { 2 };
    // Spot colors are global.
    let (global, spot) = match kind {
        0 => (true, false),
        1 => (true, true),
        _ => (false, false),
    };
    let name = if name.is_empty() { value_name(color) } else { name };
    Ok(Some(Swatch { name, paint: Paint::solid(color), global, spot }))
}

/// A color's values in its own model, as new swatches are named ("C=10 M=20 Y=30 K=0",
/// "R=255 G=128 B=0", "Gray K=40", "L=52 a=70 b=-30").
fn value_name(c: Color) -> String {
    let pct = |v: f32| (v * 100.0).round();
    let byte = |v: f32| (v * 255.0).round();
    match c {
        Color::Cmyk { c, m, y, k } => format!("C={} M={} Y={} K={}", pct(c), pct(m), pct(y), pct(k)),
        Color::Rgb { r, g, b } => format!("R={} G={} B={}", byte(r), byte(g), byte(b)),
        Color::Gray { k } => format!("Gray K={}", pct(k)),
        // `+ 0.0` turns a rounded -0 into 0.
        Color::Lab { l, a, b } => format!("L={} a={} b={}", l.round() + 0.0, a.round() + 0.0, b.round() + 0.0),
    }
}

/// `.ase` data, read field by field; reading past its end is an error.
struct AseData<'a>(&'a [u8]);

impl<'a> AseData<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(ASE_CUT_SHORT)?;
        self.0 = rest;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(ASE_CUT_SHORT)?;
        self.0 = rest;
        Ok(*head)
    }
    fn u16(&mut self) -> Result<u16, String> {
        self.array().map(u16::from_be_bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        self.array().map(u32::from_be_bytes)
    }
    /// A component; one that isn't a finite number reads as 0.
    fn f32(&mut self) -> Result<f32, String> {
        let v = f32::from_be_bytes(self.array()?);
        Ok(if v.is_finite() { v } else { 0.0 })
    }
    /// A color book string ([`read_acb`]): a `u32` count of UTF-16 code units, then the units,
    /// with `$$$/key=Text` read as `Text` and `^R`, `^C` as ®, ©; at most [`ASE_MAX_NAME`] units
    /// are kept.
    fn acb_string(&mut self) -> Result<String, String> {
        let n = usize::try_from(self.u32()?).unwrap_or(usize::MAX);
        let (units, _) = self.bytes(n.checked_mul(2).ok_or(ASE_CUT_SHORT)?)?.as_chunks::<2>();
        let units: Vec<u16> = units.iter().take(ASE_MAX_NAME).map(|u| u16::from_be_bytes(*u)).take_while(|&u| u != 0).collect();
        let s = String::from_utf16_lossy(&units);
        let s = match s.strip_prefix("$$$/") {
            Some(key) => key.split_once('=').map_or("", |(_, text)| text).to_string(),
            None => s,
        };
        Ok(s.replace("^R", "®").replace("^C", "©"))
    }
    /// A name up to its terminating zero, trimmed. A block that ends before it has no name.
    fn name(&mut self) -> Result<String, String> {
        if self.0.is_empty() {
            return Ok(String::new());
        }
        let n = usize::from(self.u16()?);
        let (units, _) = self.bytes(n * 2)?.as_chunks::<2>();
        let units: Vec<u16> = units.iter().take(ASE_MAX_NAME).map(|u| u16::from_be_bytes(*u)).take_while(|&u| u != 0).collect();
        Ok(String::from_utf16_lossy(&units).trim().to_string())
    }
}

/// A group's name as a file gives it, trimmed; "Color Group" when it has none.
fn group_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { "Color Group".into() } else { name.to_string() }
}

/// The swatch and group names a reader has handed out. Swatches and color groups share one set of
/// names, as in a document, and a name already taken gets the first free number ("Black 2").
#[derive(Default)]
struct UniqueNames {
    taken: HashSet<String>,
    /// The last number tried for each repeated name.
    last: HashMap<String, usize>,
}

impl UniqueNames {
    fn unique(&mut self, base: String) -> String {
        if self.taken.insert(base.clone()) {
            return base;
        }
        // Each pass tries the next number, and only finitely many names are taken.
        let n = self.last.entry(base.clone()).or_insert(1);
        loop {
            *n += 1;
            let name = format!("{base} {n}");
            if self.taken.insert(name.clone()) {
                return name;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Gradient, GradientPaint, GradientStop};
    use proptest::prelude::*;

    /// A color book's string: a `u32` count of UTF-16 code units, then the units.
    fn acb_str(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        out.extend((units.len() as u32).to_be_bytes());
        units.iter().for_each(|u| out.extend(u.to_be_bytes()));
    }

    /// A color book in `space` (0 RGB, 2 CMYK, 7 Lab) with `colors` (name, components) and an
    /// optional spot/process marker.
    fn acb(space: u16, colors: &[(&str, &[u8])], marker: &[u8]) -> Vec<u8> {
        let mut b = b"8BCB".to_vec();
        b.extend(1u16.to_be_bytes());
        b.extend(3000u16.to_be_bytes());
        for s in ["$$$/colorbook/Test/title=Test Book^R", "TEST ", " C", "a book"] {
            acb_str(&mut b, s);
        }
        b.extend((colors.len() as u16).to_be_bytes());
        b.extend(7u16.to_be_bytes());
        b.extend(0u16.to_be_bytes());
        b.extend(space.to_be_bytes());
        for (name, comps) in colors {
            acb_str(&mut b, name);
            b.extend(b"TST001");
            b.extend(*comps);
        }
        b.extend(marker);
        b
    }

    /// #831: color books (`.acb`) are read: their colors in RGB, CMYK or Lab, named with the book's
    /// prefix and suffix, spot unless the book says process; page padding is left out.
    #[test]
    fn color_books_are_read() {
        let lib = read_bytes(&acb(0, &[("185", &[228, 0, 43]), ("", &[0, 0, 0]), ("Red 032", &[239, 51, 64])], b"spflspot"), "fallback").unwrap();
        assert_eq!(lib.name, "Test Book®");
        let names: Vec<&str> = lib.swatches.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["TEST 185 C", "TEST Red 032 C"]);
        assert!(lib.swatches.iter().all(|s| s.spot && s.global));
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::rgb8(228, 0, 43)));
        // CMYK ink is stored inverted; a book marked process gives process colors.
        let lib = read_bytes(&acb(2, &[("Cyan", &[0, 255, 255, 255])], b"spflproc"), "x").unwrap();
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::cmyk(1.0, 0.0, 0.0, 0.0)));
        assert!(!lib.swatches[0].spot && !lib.swatches[0].global);
        // Lab: lightness 0..255 for 0..100, a and b offset by 128; no marker: spot.
        let lib = read_bytes(&acb(7, &[("Grey", &[255, 128, 128])], b""), "x").unwrap();
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::lab(100.0, 0.0, 0.0)));
        assert!(lib.swatches[0].spot);
        assert!(sniff_bytes(&acb(0, &[], b"")));
        // Unsupported versions and color spaces, and short files, are errors.
        let mut v2 = acb(0, &[], b"");
        v2[5] = 2;
        assert!(read_bytes(&v2, "x").is_err());
        assert!(read_bytes(&acb(1, &[("x", &[1, 2, 3])], b""), "x").is_err());
        let full = acb(0, &[("185", &[228, 0, 43])], b"");
        for cut in 4..full.len() {
            assert!(read_bytes(&full[..cut], "x").is_err(), "{cut} bytes");
        }
        let mut huge = b"8BCB\0\x01\0\x01".to_vec();
        huge.extend(u32::MAX.to_be_bytes());
        assert!(read_bytes(&huge, "x").is_err(), "a string longer than the file");
    }

    fn sample() -> SwatchLibrary {
        let grad = Paint::Gradient(Box::new(GradientPaint::new(Gradient {
            kind: GradientKind::Linear,
            stops: vec![
                GradientStop { midpoint: 0.4, ..GradientStop::new(0.0, Color::cmyk(0.1, 0.2, 0.3, 0.0)) },
                GradientStop { opacity: 0.5, ..GradientStop::new(1.0, Color::rgb8(255, 0, 0)) },
            ],
        })));
        SwatchLibrary {
            name: "Brand */ Colours".into(),
            swatches: vec![
                Swatch { name: "Ink".into(), paint: Paint::solid(Color::cmyk(1.0, 0.5, 0.0, 0.2)), global: true, spot: true },
                Swatch { name: "Fade".into(), paint: grad, global: false, spot: false },
                Swatch { name: "Tiles".into(), paint: Paint::Pattern { pattern: "Dots".into(), xf: Default::default() }, global: false, spot: false },
            ],
            groups: vec![SwatchGroup {
                name: "Neutrals".into(),
                swatches: vec![
                    Swatch { name: "R=255 G=0 B=0".into(), paint: Paint::solid(Color::rgb8(255, 0, 0)), global: false, spot: false },
                    Swatch { name: "Mist".into(), paint: Paint::solid(Color::gray(0.25)), global: true, spot: false },
                ],
            }],
        }
    }

    /// `lib` written as text format `f`.
    fn written(lib: &SwatchLibrary, f: PaletteFormat) -> String {
        String::from_utf8(write(lib, f).unwrap()).unwrap()
    }

    #[test]
    fn native_round_trip_keeps_cmyk_spot_and_groups() {
        let lib = sample();
        let text = written(&lib, PaletteFormat::Native);
        assert!(sniff(&text));
        let back = read(&text, "fallback").unwrap();
        let mut expected = lib.clone();
        expected.swatches.retain(|w| !matches!(w.paint, Paint::Pattern { .. }));
        assert_eq!(back, expected, "everything but the pattern comes back exactly");
        assert!(back.swatches[0].spot && back.swatches[0].global);
        assert_eq!(back.swatches[0].paint.color(), Some(Color::cmyk(1.0, 0.5, 0.0, 0.2)));
        assert!(read("{\"format\": \"other\", \"version\": 1, \"name\": \"x\"}", "x").is_err());
    }

    #[test]
    fn gpl_writes_rgb_with_group_headers_and_reads_comments_and_headers() {
        let text = written(&sample(), PaletteFormat::Gpl);
        assert!(text.starts_with("GIMP Palette\nName: Brand */ Colours\n"));
        assert!(text.contains("# Group: Neutrals\n255   0   0\tR=255 G=0 B=0\n"), "{text}");
        assert!(!text.contains("Fade") && !text.contains("Tiles"), "only solid colours");
        let back = read(&text, "x").unwrap();
        assert_eq!(back.name, "Brand */ Colours");
        assert_eq!(back.swatches.len(), 1);
        assert_eq!(back.groups[0].swatches[1].paint.color(), Some(Color::rgb8(191, 191, 191)), "grey as 8-bit RGB");
        // Another tool's palette: comments, a Columns header, blank lines, unnamed and repeated names.
        let other = "GIMP Palette\r\nName: Sunset\r\nColumns: 4\r\n# a comment\r\n\r\n  0 128 255\tSky Blue\r\n10 20 30\r\n0 0 0 Black\r\n1 1 1 Black\r\nbad line\r\n";
        let lib = read(other, "file").unwrap();
        assert_eq!(lib.name, "Sunset");
        let names: Vec<&str> = lib.swatches.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Sky Blue", "R=10 G=20 B=30", "Black", "Black 2"]);
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::rgb8(0, 128, 255)));
        assert_eq!(read("GIMP Palette\n1 2 3 x\n", "Fallback").unwrap().name, "Fallback");
        // Colors and groups share names; an unnamed group is "Color Group".
        let lib = read("GIMP Palette\n0 0 0 Black\n# Group: Black\n1 1 1 Black\n# Group:\n", "x").unwrap();
        let groups: Vec<(&str, Vec<&str>)> =
            lib.groups.iter().map(|g| (g.name.as_str(), g.swatches.iter().map(|w| w.name.as_str()).collect())).collect();
        assert_eq!(groups, [("Black 2", vec!["Black 3"]), ("Color Group", vec![])]);
    }

    #[test]
    fn css_escapes_names_and_writes_gradients() {
        assert_eq!(css_property("Sky Blue"), "--sky-blue");
        assert_eq!(css_property("R=255 G=0 B=0"), "--r\\=255-g\\=0-b\\=0");
        assert_eq!(css_property("#FF00CC"), "--\\#ff00cc");
        assert_eq!(css_property("Café 50%"), "--café-50\\%");
        assert_eq!(css_property("  "), "--swatch");
        let text = written(&sample(), PaletteFormat::Css);
        assert!(text.starts_with("/* Brand * / Colours */\n:root {\n"), "{text}");
        assert!(text.contains("  --ink: #"), "{text}");
        assert!(text.contains("; /* spot */"));
        assert!(text.contains("  --fade: linear-gradient(90deg, #"), "{text}");
        assert!(text.contains("#ff000080 100%)"), "half-transparent stop: {text}");
        assert!(text.contains("  /* Neutrals */\n  --r\\=255-g\\=0-b\\=0: #ff0000;\n"));
        assert!(!text.contains("tiles"));
        // Names that collide once escaped are numbered.
        let lib = SwatchLibrary {
            name: "x".into(),
            swatches: ["Red", "red"].map(|n| Swatch { name: n.into(), paint: Paint::solid(Color::BLACK), global: false, spot: false }).to_vec(),
            groups: vec![],
        };
        let text = written(&lib, PaletteFormat::Css);
        assert!(text.contains("--red: #000000;") && text.contains("--red-2: #000000;"));
        assert!(read(&text, "x").is_err(), "CSS is written only");
    }

    #[test]
    fn holds_counts_the_swatches_each_format_writes() {
        // `sample()` holds the spot color Ink, the linear gradient Fade, the pattern Tiles and two solid colors in a group.
        let mut lib = sample();
        let Paint::Gradient(mut mesh) = lib.swatches[1].paint.clone() else { panic!("Fade is a gradient") };
        mesh.gradient.kind = GradientKind::Freeform;
        lib.swatches.push(Swatch { name: "Mesh".into(), paint: Paint::Gradient(mesh), global: false, spot: false });
        lib.swatches.push(Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false });
        let held = |f: PaletteFormat| lib.iter().filter(|w| f.holds(&w.paint)).count();
        assert_eq!(PaletteFormat::ALL.map(held), [5, 3, 3, 4], "vcswatches, gpl, ase, css");
        for f in [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Ase] {
            assert_eq!(read_bytes(&write(&lib, f).unwrap(), "x").unwrap().len(), held(f), "{f:?}");
        }
        let css = written(&lib, PaletteFormat::Css);
        assert_eq!(css.lines().filter(|l| l.starts_with("  --")).count(), held(PaletteFormat::Css), "one property each: {css}");
    }

    /// A swatch exchange file of `blocks` (type, body).
    fn ase(blocks: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut out = b"ASEF\0\x01\0\0".to_vec();
        out.extend((blocks.len() as u32).to_be_bytes());
        for (kind, body) in blocks {
            out.extend(kind.to_be_bytes());
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(body);
        }
        out
    }

    /// A name as `.ase` stores it.
    fn ase_name(name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut out = (units.len() as u16).to_be_bytes().to_vec();
        out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        out
    }

    /// A color block: name, model, components and color type.
    fn ase_color(name: &str, model: &[u8; 4], values: &[f32], kind: u16) -> (u16, Vec<u8>) {
        let mut body = ase_name(name);
        body.extend(model);
        body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
        body.extend(kind.to_be_bytes());
        (ASE_COLOR, body)
    }

    #[test]
    fn ase_reads_color_models_kinds_and_groups() {
        let bytes = ase(&[
            ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0),
            ase_color("Ink", b"CMYK", &[1.0, 0.5, 0.0, 0.2], 1),
            (ASE_GROUP_START, ase_name("Neutrals")),
            ase_color("Mist", b"Gray", &[0.75], 2),
            ase_color("Clay", b"LAB ", &[0.5, 20.0, -30.0], 2),
            (ASE_GROUP_END, vec![]),
            ase_color("Paper", b"RGB ", &[1.0, 1.0, 1.0], 2),
        ]);
        assert!(sniff_bytes(&bytes));
        let sw = |name: &str, color, global, spot| Swatch { name: name.into(), paint: Paint::solid(color), global, spot };
        let expected = SwatchLibrary {
            name: "Brand".into(),
            swatches: vec![
                sw("Sky", Color::rgb(0.0, 0.5, 1.0), true, false),
                sw("Ink", Color::cmyk(1.0, 0.5, 0.0, 0.2), true, true),
                sw("Paper", Color::WHITE, false, false),
            ],
            groups: vec![SwatchGroup {
                name: "Neutrals".into(),
                swatches: vec![sw("Mist", Color::gray(0.25), false, false), sw("Clay", Color::lab(50.0, 20.0, -30.0), false, false)],
            }],
        };
        assert_eq!(read_bytes(&bytes, "Brand").unwrap(), expected, "named after the file; 0 global, 1 spot, 2 process");
        // A lightness above 1 is L* itself.
        let deep = read_bytes(&ase(&[ase_color("Deep", b"LAB ", &[53.0, 20.0, -30.0], 2)]), "x").unwrap();
        assert_eq!(deep.swatches[0].paint.color(), Some(Color::lab(53.0, 20.0, -30.0)));
        // Text libraries read through the same call, also when they aren't valid UTF-8.
        assert!(sniff_bytes(b"GIMP Palette\n1 2 3 x\n") && !sniff_bytes(b"ASE") && !sniff_bytes(&[0xff, 0xfe, 0]));
        assert_eq!(read_bytes(b"GIMP Palette\n1 2 3 x\n", "F").unwrap().swatches.len(), 1);
        let latin1 = b"GIMP Palette\n1 2 3 Caf\xe9\n";
        assert!(sniff_bytes(latin1));
        assert_eq!(read_bytes(latin1, "F").unwrap().swatches.len(), 1);
    }

    #[test]
    fn ase_names_unnamed_and_repeated_entries_and_skips_other_blocks() {
        let mut untyped = ase_color("Plain", b"RGB ", &[0.2, 0.4, 0.6], 2);
        untyped.1.truncate(untyped.1.len() - 2);
        let bytes = ase(&[
            ase_color("Café 🎨", b"RGB ", &[1.0, 0.0, 0.0], 2),
            ase_color("", b"CMYK", &[0.1, 0.2, 0.3, 0.0], 2),
            ase_color(" Red ", b"RGB ", &[2.0, -1.0, f32::NAN], 0),
            ase_color("Red", b"Gray", &[0.0], 2),
            untyped,
            ase_color("Hue", b"HSB ", &[0.1, 0.2, 0.3], 2),
            (0x0042, vec![1, 2, 3]),
            (ASE_GROUP_END, vec![]),
            (ASE_GROUP_START, ase_name("")),
            (ASE_GROUP_START, ase_name("Red")),
            ase_color("Red", b"Gray", &[1.0], 2),
        ]);
        let lib = read_bytes(&bytes, "x").unwrap();
        let names: Vec<&str> = lib.swatches.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Café 🎨", "C=10 M=20 Y=30 K=0", "Red", "Red 2", "Plain"], "an HSB color and an unknown block are skipped");
        assert_eq!(lib.swatches[2].paint.color(), Some(Color::rgb(1.0, 0.0, 0.0)), "clamped, NaN read as 0");
        assert!(lib.swatches[2].global && !lib.swatches[4].global, "without a color type: process");
        // A group start ends the open group, and groups and colors share names.
        let groups: Vec<(&str, Vec<&str>)> =
            lib.groups.iter().map(|g| (g.name.as_str(), g.swatches.iter().map(|w| w.name.as_str()).collect())).collect();
        assert_eq!(groups, [("Color Group", vec![]), ("Red 3", vec!["Red 4"])]);
        assert_eq!(lib.groups[1].swatches[0].paint.color(), Some(Color::gray(0.0)), "gray level 1 is white");
    }

    #[test]
    fn damaged_ase_files_are_errors() {
        let good = ase(&[(ASE_GROUP_START, ase_name("G")), ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0)]);
        assert_eq!(read_bytes(&good, "x").unwrap().len(), 1);
        for n in ASE_SIGNATURE.len()..good.len() {
            assert!(read_bytes(&good[..n], "x").is_err(), "cut at {n}");
        }
        let mut v2 = good.clone();
        v2[5] = 2;
        assert!(read_bytes(&v2, "x").unwrap_err().contains("2.0"), "version 2.0");
        // Too many colors, or too many groups (empty group starts take 6 bytes each).
        let colors: Vec<(u16, Vec<u8>)> = (0..=ASE_MAX_ENTRIES).map(|_| ase_color("Gray", b"Gray", &[0.5], 2)).collect();
        assert!(read_bytes(&ase(&colors), "x").unwrap_err().contains("more than"));
        let groups: Vec<(u16, Vec<u8>)> = (0..=ASE_MAX_ENTRIES).map(|_| (ASE_GROUP_START, vec![])).collect();
        assert!(read_bytes(&ase(&groups), "x").unwrap_err().contains("more than"));
        assert_eq!(read_bytes(&ase(&groups[1..]), "x").unwrap().groups.len(), ASE_MAX_ENTRIES, "up to the limit");
    }

    #[test]
    fn ase_writes_each_model_kind_and_group() {
        let sw = |name: &str, color, global, spot| Swatch { name: name.into(), paint: Paint::solid(color), global, spot };
        let tint = Swatch {
            name: "Ink 40%".into(),
            paint: Paint::Solid { color: Color::cmyk(0.4, 0.2, 0.0, 0.08), swatch: Some("Ink".into()), tint: 0.4 },
            global: false,
            spot: false,
        };
        // `sample()` holds the spot color Ink, the gradient Fade, the pattern Tiles and the group Neutrals.
        let mut lib = sample();
        lib.swatches.insert(0, sw("Sky", Color::rgb(0.0, 0.5, 1.0), true, false));
        lib.swatches.push(tint);
        lib.groups[0].swatches = vec![sw("Mist", Color::gray(0.25), false, false), sw("Clay", Color::lab(50.0, 20.0, -30.0), false, false)];
        lib.groups.push(SwatchGroup { name: "Empty".into(), swatches: vec![] });
        let bytes = write(&lib, PaletteFormat::Ase).unwrap();
        let expected = ase(&[
            ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0),
            ase_color("Ink", b"CMYK", &[1.0, 0.5, 0.0, 0.2], 1),
            ase_color("Ink 40%", b"CMYK", &[0.4, 0.2, 0.0, 0.08], 2),
            (ASE_GROUP_START, ase_name("Neutrals")),
            ase_color("Mist", b"Gray", &[0.75], 2),
            ase_color("Clay", b"LAB ", &[0.5, 20.0, -30.0], 2),
            (ASE_GROUP_END, vec![]),
            (ASE_GROUP_START, ase_name("Empty")),
            (ASE_GROUP_END, vec![]),
        ]);
        assert_eq!(bytes, expected, "no gradient or pattern; the tint as the process color it shows");
        let back = read_bytes(&bytes, "Brand").unwrap();
        assert_eq!((back.len(), back.groups.len()), (5, 2), "the empty group stays");
        assert_eq!(PaletteFormat::parse(".ASE"), Some(PaletteFormat::Ase));
        assert!(PaletteFormat::Ase.readable() && PaletteFormat::Ase.binary() && !PaletteFormat::Gpl.binary());
    }

    #[test]
    fn ase_names_lose_nul_and_stop_at_the_reader_limit() {
        let names = ["Re\0d".to_string(), "é".repeat(1030), format!("a{}", "🎨".repeat(600))];
        let swatches = names.iter().map(|n| Swatch { name: n.clone(), paint: Paint::solid(Color::BLACK), global: false, spot: false }).collect();
        let lib = SwatchLibrary { name: "x".into(), swatches, groups: vec![] };
        let back = read_bytes(&write(&lib, PaletteFormat::Ase).unwrap(), "x").unwrap();
        let got: Vec<&str> = back.swatches.iter().map(|w| w.name.as_str()).collect();
        // At most 1024 UTF-16 code units, cut between characters: "a" and 511 two-unit emoji.
        assert_eq!(got, ["Red".to_string(), "é".repeat(1024), format!("a{}", "🎨".repeat(511))]);
    }

    #[test]
    fn ase_clamps_components_and_writes_non_finite_ones_as_zero() {
        let sw = |name: &str, color| Swatch { name: name.into(), paint: Paint::solid(color), global: false, spot: false };
        let swatches = vec![
            sw("Lab", Color::lab(100.5, 300.0, -300.0)),
            sw("Rgb", Color::rgb(2.0, -1.0, f32::NAN)),
            sw("Cmyk", Color::cmyk(1.5, 0.0, 0.0, f32::INFINITY)),
            sw("Gray", Color::gray(1.5)),
        ];
        let bytes = write(&SwatchLibrary { name: "x".into(), swatches, groups: vec![] }, PaletteFormat::Ase).unwrap();
        let expected = ase(&[
            ase_color("Lab", b"LAB ", &[1.0, 127.0, -128.0], 2),
            ase_color("Rgb", b"RGB ", &[1.0, 0.0, 0.0], 2),
            ase_color("Cmyk", b"CMYK", &[1.0, 0.0, 0.0, 0.0], 2),
            ase_color("Gray", b"Gray", &[0.0], 2),
        ]);
        assert_eq!(bytes, expected, "clamped to the ranges .ase holds, a component that isn't finite as 0");
        // Lightness 100.5 is written as 1, which reads back as L* 100.
        let back = read_bytes(&bytes, "x").unwrap();
        assert_eq!(back.swatches[0].paint.color(), Some(Color::lab(100.0, 127.0, -128.0)));
    }

    #[test]
    fn ase_write_refuses_more_entries_than_the_reader_takes() {
        let gray = |i: usize| Swatch { name: format!("G{i}"), paint: Paint::solid(Color::gray(0.5)), global: false, spot: false };
        // The limit counts colors and groups: a group and `ASE_MAX_ENTRIES - 1` colors reach it.
        let mut lib = SwatchLibrary {
            name: "x".into(),
            swatches: (1..ASE_MAX_ENTRIES).map(gray).collect(),
            groups: vec![SwatchGroup { name: "Group".into(), swatches: vec![] }],
        };
        let back = read_bytes(&write(&lib, PaletteFormat::Ase).unwrap(), "x").unwrap();
        assert_eq!((back.len(), back.groups.len()), (ASE_MAX_ENTRIES - 1, 1));
        lib.swatches.push(gray(0));
        let e = write(&lib, PaletteFormat::Ase).unwrap_err();
        assert!(e.contains("reads at most 100000 colors and groups from a swatch exchange file, and this library has 100001"), "{e}");
        // A gradient is left out and is not counted.
        lib.swatches.last_mut().unwrap().paint = sample().swatches[1].paint.clone();
        assert!(write(&lib, PaletteFormat::Ase).is_ok());
    }

    /// A color in one of the four models, in the ranges `.ase` holds.
    fn arb_color() -> impl Strategy<Value = Color> {
        let unit = || 0.0f32..=1.0;
        prop_oneof![
            (unit(), unit(), unit()).prop_map(|(r, g, b)| Color::rgb(r, g, b)),
            (unit(), unit(), unit(), unit()).prop_map(|(c, m, y, k)| Color::cmyk(c, m, y, k)),
            unit().prop_map(Color::gray),
            (0.0f32..=100.0, -128.0f32..=127.0, -128.0f32..=127.0).prop_map(|(l, a, b)| Color::lab(l, a, b)),
        ]
    }

    /// A swatch's paint with its global and spot flags: a solid color with any flags, a tint swatch
    /// (both flags off) or a gradient.
    fn arb_paint() -> impl Strategy<Value = (Paint, bool, bool)> {
        prop_oneof![
            4 => (arb_color(), any::<bool>(), any::<bool>()).prop_map(|(c, global, spot)| (Paint::solid(c), global, spot)),
            1 => arb_color().prop_map(|c| (Paint::Solid { color: c, swatch: Some("Base".into()), tint: 0.4 }, false, false)),
            1 => Just((sample().swatches[1].paint.clone(), false, false)),
        ]
    }

    /// Ungrouped swatches and color groups. A name is a number and up to 12 characters of any script
    /// but NUL, trimmed, so swatches and groups have unique names, as in a document.
    fn arb_library() -> impl Strategy<Value = SwatchLibrary> {
        let swatches = || prop::collection::vec((arb_paint(), "[^\\x00]{0,12}"), 0..12);
        (swatches(), prop::collection::vec(("[^\\x00]{0,12}", swatches()), 0..4)).prop_map(|(loose, groups)| {
            let mut n = 0;
            let mut name = |prefix: &str, base: &str| {
                n += 1;
                format!("{prefix}{n} {base}").trim().to_string()
            };
            let mut lib = SwatchLibrary { name: "Fuzz".into(), ..Default::default() };
            for ((paint, global, spot), base) in loose {
                lib.swatches.push(Swatch { name: name("", &base), paint, global, spot });
            }
            for (base, list) in groups {
                let mut g = SwatchGroup { name: name("G", &base), swatches: vec![] };
                for ((paint, global, spot), base) in list {
                    g.swatches.push(Swatch { name: name("", &base), paint, global, spot });
                }
                lib.groups.push(g);
            }
            lib
        })
    }

    /// Are `got` and `want` in the same model, with every component within 1e-4? Lab lightness and
    /// gray read back within `f32` rounding.
    fn close(got: Color, want: Color) -> bool {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        match (got, want) {
            (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) => near(r, r2) && near(g, g2) && near(b, b2),
            (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => near(c, c2) && near(m, m2) && near(y, y2) && near(k, k2),
            (Color::Gray { k }, Color::Gray { k: k2 }) => near(k, k2),
            (Color::Lab { l, a, b }, Color::Lab { l: l2, a: a2, b: b2 }) => near(l, l2) && near(a, a2) && near(b, b2),
            _ => false,
        }
    }

    /// The swatches of `list` as an `.ase` file reads them back: solid colors only, unlinked, spot
    /// colors global.
    fn ase_expected(list: &[Swatch]) -> Vec<Swatch> {
        list.iter()
            .filter_map(|w| Some(Swatch { name: w.name.clone(), paint: Paint::solid(w.paint.color()?), global: w.global || w.spot, spot: w.spot }))
            .collect()
    }

    /// Do swatches `got` have the names, flags and unlinked colors (within 1e-4) of `want`?
    fn same_swatches(got: &[Swatch], want: &[Swatch]) -> bool {
        got.len() == want.len()
            && got.iter().zip(want).all(|(g, w)| {
                let color = match (&g.paint, w.paint.color()) {
                    (Paint::Solid { color, swatch: None, .. }, Some(c)) => close(*color, c),
                    _ => false,
                };
                g.name == w.name && g.global == w.global && g.spot == w.spot && color
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        /// A written `.ase` file reads back as the library's solid colors in their own model, with
        /// their names, kinds and color groups.
        #[test]
        fn ase_round_trip_keeps_models_kinds_and_groups(lib in arb_library()) {
            let bytes = write(&lib, PaletteFormat::Ase).unwrap();
            prop_assert!(sniff_bytes(&bytes));
            let back = read_bytes(&bytes, &lib.name).unwrap();
            prop_assert_eq!(&back.name, &lib.name);
            prop_assert!(same_swatches(&back.swatches, &ase_expected(&lib.swatches)), "{:?}", back.swatches);
            prop_assert_eq!(back.groups.len(), lib.groups.len());
            for (g, want) in back.groups.iter().zip(&lib.groups) {
                prop_assert_eq!(&g.name, &want.name);
                prop_assert!(same_swatches(&g.swatches, &ase_expected(&want.swatches)), "{}: {:?}", g.name, g.swatches);
            }
        }

        /// A written `.vcswatches`, `.gpl` or `.ase` file reads back with as many swatches as
        /// [`PaletteFormat::holds`] counts.
        #[test]
        fn written_files_read_back_with_the_swatches_holds_counts(lib in arb_library()) {
            for f in [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Ase] {
                let back = read_bytes(&write(&lib, f).unwrap(), &lib.name).unwrap();
                prop_assert_eq!(back.len(), lib.iter().filter(|w| f.holds(&w.paint)).count(), "{:?}", f);
            }
        }
    }
}
