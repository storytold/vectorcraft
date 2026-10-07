//! Swatch library files: the native `.vcswatches` JSON (colour models, global, spot, gradients
//! and colour groups exactly), `.gpl` palettes (8-bit RGB; colour groups as `# Group:` comment
//! headers) and CSS custom properties (written only).

use serde::{Deserialize, Serialize};

use crate::{Color, GradientKind, Paint, Swatch, SwatchGroup, SwatchLibrary};

/// A swatch library file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteFormat {
    /// `.vcswatches`: JSON, lossless.
    Native,
    /// `.gpl`: a plain-text RGB palette many paint and design tools read.
    Gpl,
    /// `.css`: custom properties on `:root` (colours and gradients; written only).
    Css,
}

impl PaletteFormat {
    pub const ALL: [PaletteFormat; 3] = [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Css];

    /// The format's id, which is also its extension.
    pub fn id(self) -> &'static str {
        match self {
            PaletteFormat::Native => "vcswatches",
            PaletteFormat::Gpl => "gpl",
            PaletteFormat::Css => "css",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaletteFormat::Native => "VectorCraft Swatches (.vcswatches)",
            PaletteFormat::Gpl => "GPL Palette (.gpl)",
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

/// Write `lib` in `format`. Pattern swatches (their tiles live in a document) and None are left
/// out; `.gpl` keeps solid colours only, as 8-bit RGB.
pub fn write(lib: &SwatchLibrary, format: PaletteFormat) -> String {
    let keep = |w: &&Swatch| matches!(w.paint, Paint::Solid { .. } | Paint::Gradient(_));
    match format {
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
        PaletteFormat::Css => write_css(lib, keep),
    }
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

/// Read any swatch library file: binary ones from other apps (`.ase`, `.acb`, `.aco`, see
/// [`crate::palette_bin`]) or the text ones [`read`] reads.
pub fn read_bytes(bytes: &[u8], name: &str) -> Result<SwatchLibrary, String> {
    if crate::palette_bin::sniff(bytes) {
        return crate::palette_bin::read(bytes, name);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not a swatch library (.vcswatches, .gpl, .ase, .acb or .aco)".to_string())?;
    read(text, name)
}

/// Is `bytes` a library [`read_bytes`] understands?
pub fn sniff_bytes(bytes: &[u8]) -> bool {
    crate::palette_bin::sniff(bytes) || std::str::from_utf8(bytes).is_ok_and(sniff)
}

/// Is `text` a library [`read`] understands?
pub fn sniff(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with(GPL_HEADER) || (t.starts_with('{') && t.contains(NATIVE_FORMAT))
}

/// `.gpl`: `Name:` names the library, `# Group: name` starts a colour group, other `#` lines and
/// headers (`Columns:`) are skipped, as are lines that aren't `r g b [name]`. Unnamed colours are
/// named by their values; repeated names get a number.
fn read_gpl(text: &str, fallback: &str) -> SwatchLibrary {
    let mut lib = SwatchLibrary { name: fallback.into(), ..Default::default() };
    let mut names: Vec<String> = vec![];
    for line in text.lines().skip(1) {
        let line = line.trim();
        if let Some(g) = line.strip_prefix(GPL_GROUP) {
            lib.groups.push(SwatchGroup { name: g.trim().to_string(), swatches: vec![] });
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
        let name = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} {i}") }).find(|n| !names.contains(n)).unwrap_or(base);
        names.push(name.clone());
        let w = Swatch { name, paint: Paint::solid(Color::rgb8(r, g, b)), global: false, spot: false };
        match lib.groups.last_mut() {
            Some(grp) => grp.swatches.push(w),
            None => lib.swatches.push(w),
        }
    }
    lib
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Gradient, GradientPaint, GradientStop};

    fn sample() -> SwatchLibrary {
        let grad = Paint::Gradient(Box::new(GradientPaint::new(Gradient {
            kind: GradientKind::Linear,
            stops: vec![
                GradientStop { midpoint: 0.4, ..GradientStop::new(0.0, Color::cmyk(0.1, 0.2, 0.3, 0.0)) },
                GradientStop { opacity: 0.5, ..GradientStop::new(1.0, Color::rgb8(255, 0, 0)) },
            ],
            ..Gradient::default()
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

    #[test]
    fn native_round_trip_keeps_cmyk_spot_and_groups() {
        let lib = sample();
        let text = write(&lib, PaletteFormat::Native);
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
        let text = write(&sample(), PaletteFormat::Gpl);
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
    }

    #[test]
    fn css_escapes_names_and_writes_gradients() {
        assert_eq!(css_property("Sky Blue"), "--sky-blue");
        assert_eq!(css_property("R=255 G=0 B=0"), "--r\\=255-g\\=0-b\\=0");
        assert_eq!(css_property("#FF00CC"), "--\\#ff00cc");
        assert_eq!(css_property("Café 50%"), "--café-50\\%");
        assert_eq!(css_property("  "), "--swatch");
        let text = write(&sample(), PaletteFormat::Css);
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
        let text = write(&lib, PaletteFormat::Css);
        assert!(text.contains("--red: #000000;") && text.contains("--red-2: #000000;"));
        assert!(read(&text, "x").is_err(), "CSS is written only");
    }
}
