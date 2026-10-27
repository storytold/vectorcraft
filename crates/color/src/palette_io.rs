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
    let keep = |w: &&Swatch| format.holds(&w.paint);
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
    let mut taken: HashSet<String> = HashSet::new();
    // The next number to try for each base, so repeated names don't search the ones used before.
    let mut next: HashMap<String, usize> = HashMap::new();
    let mut props = |out: &mut String, list: &[Swatch]| {
        for w in list.iter().filter(&keep) {
            let value = match &w.paint {
                Paint::Solid { color, .. } => css_color(color, 1.0),
                Paint::Gradient(g) if g.gradient.kind != GradientKind::Freeform => {
                    let stops: Vec<String> =
                        g.gradient.stops.iter().map(|s| format!("{} {}%", css_color(&s.color, s.opacity), s.offset * 100.0)).collect();
                    match g.gradient.kind {
                        GradientKind::Radial => format!("radial-gradient(circle, {})", stops.join(", ")),
                        _ => format!("linear-gradient({}deg, {})", (90.0 - g.angle).rem_euclid(360.0).round(), stops.join(", ")),
                    }
                }
                _ => continue,
            };
            // Names differing only in case or punctuation map to one property: number the others.
            let base = css_property(&w.name);
            let name = if taken.insert(base.clone()) {
                base
            } else {
                let n = next.entry(base.clone()).or_insert(1);
                loop {
                    *n += 1;
                    let name = format!("{base}-{n}");
                    if taken.insert(name.clone()) {
                        break name;
                    }
                }
            };
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
            if !n.trim().is_empty() {
                lib.name = n.trim().to_string();
            }
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
    // Every name gets the prefix and the suffix, and is kept twice (the swatch's and the set of
    // names taken): a megabyte book of long affixes and many colors came to hundreds of MB.
    let mut kept = 0usize;
    for (name, color) in colors.into_iter().filter(|(n, _)| !n.trim().is_empty()) {
        kept = kept.saturating_add(2 * (prefix.len() + name.len() + suffix.len()));
        if kept > ACB_MAX_NAME_BYTES {
            return Err(format!("the color book's names take more than {} MB", ACB_MAX_NAME_BYTES >> 20));
        }
        let name = names.unique(format!("{prefix}{name}{suffix}").trim().to_string());
        lib.swatches.push(Swatch { name, paint: Paint::solid(color), global: spot, spot });
    }
    Ok(lib)
}

/// Most bytes a color book's full color names (with its prefix and suffix) may take, counted
/// twice as they are kept twice.
const ACB_MAX_NAME_BYTES: usize = 64 << 20;

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
mod tests;
