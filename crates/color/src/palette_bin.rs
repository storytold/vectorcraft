//! Binary swatch library files from other apps, read so that real colour libraries (including
//! spot colour books such as the ones design apps install) can be loaded:
//!
//! - `.ase` Swatch Exchange: process, global and spot colours in RGB, CMYK, Lab or Gray, with
//!   colour groups;
//! - `.acb` colour books: named spot colours (prefix + name + suffix) in RGB, CMYK or Lab;
//! - `.aco` colour palettes: RGB, HSB, CMYK, Lab and Gray colours, named when the file has the
//!   version 2 section.
//!
//! The layouts follow the publicly documented formats. Every read is bounds-checked and every count
//! is capped: these files are untrusted input.

use crate::{Color, Paint, Swatch, SwatchGroup, SwatchLibrary};

/// Most colours read from one file.
const MAX_COLORS: usize = 20_000;
/// Longest name read (UTF-16 units).
const MAX_NAME: usize = 1024;

/// A big-endian reader over a byte slice.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }
    fn u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_be_bytes([*s.first()?, *s.get(1)?]))
    }
    fn u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_be_bytes(s.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        let v = f32::from_bits(self.u32()?);
        v.is_finite().then_some(v)
    }
    /// `n` UTF-16BE units, a trailing NUL dropped.
    fn utf16(&mut self, n: usize) -> Option<String> {
        if n > MAX_NAME {
            return None;
        }
        let units: Vec<u16> = self.take(n * 2)?.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        let s = String::from_utf16_lossy(&units);
        Some(s.trim_end_matches('\0').to_string())
    }
    fn done(&self) -> bool {
        self.at >= self.b.len()
    }
}

/// Is `bytes` one of the binary libraries [`read`] understands?
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ASEF") || bytes.starts_with(b"8BCB") || sniff_aco(bytes)
}

fn sniff_aco(bytes: &[u8]) -> bool {
    let mut r = Reader::new(bytes);
    let (Some(version), Some(count)) = (r.u16(), r.u16()) else { return false };
    // A version 1 or 2 header whose entries fit the file (10 bytes each at least).
    (version == 1 || version == 2) && count > 0 && usize::from(count) * 10 + 4 <= bytes.len()
}

/// Read a binary library (`.ase`, `.acb` or `.aco`, from its content). `name` names it when the
/// file doesn't.
pub fn read(bytes: &[u8], name: &str) -> Result<SwatchLibrary, String> {
    let lib = if bytes.starts_with(b"ASEF") {
        read_ase(bytes, name)
    } else if bytes.starts_with(b"8BCB") {
        read_acb(bytes, name)
    } else if sniff_aco(bytes) {
        read_aco(bytes, name)
    } else {
        return Err("not a swatch library (.ase, .acb or .aco)".into());
    };
    let lib = lib.ok_or("the swatch library file is damaged or cut short")?;
    if lib.swatches.is_empty() && lib.groups.iter().all(|g| g.swatches.is_empty()) {
        return Err("the swatch library has no colours".into());
    }
    Ok(lib)
}

/// `base`, numbered (`base 2`, …) when `names` already has it; recorded in `names`.
fn unique(base: String, names: &mut Vec<String>) -> String {
    let n = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} {i}") }).find(|n| !names.contains(n)).unwrap_or(base);
    names.push(n.clone());
    n
}

fn unit(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

// ---------- .ase ----------

fn read_ase(bytes: &[u8], name: &str) -> Option<SwatchLibrary> {
    let mut r = Reader::new(bytes);
    r.take(4)?;
    let (_major, _minor) = (r.u16()?, r.u16()?);
    let blocks = r.u32()? as usize;
    let mut lib = SwatchLibrary { name: name.into(), ..Default::default() };
    let mut names = vec![];
    let mut in_group = false;
    let mut count = 0;
    for _ in 0..blocks.min(MAX_COLORS * 2) {
        if r.done() {
            break;
        }
        let kind = r.u16()?;
        let len = r.u32()? as usize;
        let mut b = Reader::new(r.take(len)?);
        match kind {
            // Group start: its name.
            0xC001 => {
                let n = usize::from(b.u16()?);
                let g = b.utf16(n)?;
                lib.groups.push(SwatchGroup { name: g, swatches: vec![] });
                in_group = true;
            }
            0xC002 => in_group = false,
            0x0001 => {
                let n = usize::from(b.u16()?);
                let label = b.utf16(n)?;
                let model = b.take(4)?;
                let color = match model {
                    b"CMYK" => Color::cmyk(unit(b.f32()?), unit(b.f32()?), unit(b.f32()?), unit(b.f32()?)),
                    b"RGB " => Color::rgb(unit(b.f32()?), unit(b.f32()?), unit(b.f32()?)),
                    b"LAB " => {
                        let (l, a, bb) = (b.f32()?, b.f32()?, b.f32()?);
                        // L is stored 0..1 (a fraction of 100).
                        Color::lab((if l <= 1.0 { l * 100.0 } else { l }).clamp(0.0, 100.0), a.clamp(-128.0, 127.0), bb.clamp(-128.0, 127.0))
                    }
                    b"Gray" => Color::gray(1.0 - unit(b.f32()?)),
                    _ => continue,
                };
                // 0 global, 1 spot, 2 normal (process).
                let kind = b.u16().unwrap_or(2);
                let label = if label.trim().is_empty() { color.to_hex() } else { label };
                let w = Swatch { name: unique(label, &mut names), paint: Paint::solid(color), global: kind <= 1, spot: kind == 1 };
                match lib.groups.last_mut().filter(|_| in_group) {
                    Some(g) => g.swatches.push(w),
                    None => lib.swatches.push(w),
                }
                count += 1;
                if count >= MAX_COLORS {
                    break;
                }
            }
            _ => {}
        }
    }
    Some(lib)
}

// ---------- .acb ----------

/// An `.acb` string: a u32 length (UTF-16 units) and the text. Localisable strings
/// (`$$$/key=Text`) keep their text.
fn acb_string(r: &mut Reader) -> Option<String> {
    let n = r.u32()? as usize;
    let s = r.utf16(n)?;
    Some(match s.strip_prefix("$$$") {
        Some(rest) => rest.split_once('=').map_or(String::new(), |(_, t)| t.to_string()),
        None => s,
    })
}

fn read_acb(bytes: &[u8], name: &str) -> Option<SwatchLibrary> {
    let mut r = Reader::new(bytes);
    r.take(4)?;
    let (_version, _id) = (r.u16()?, r.u16()?);
    let title = acb_string(&mut r)?;
    let prefix = acb_string(&mut r)?;
    let postfix = acb_string(&mut r)?;
    let _description = acb_string(&mut r)?;
    let count = usize::from(r.u16()?);
    let (_page, _offset) = (r.u16()?, r.u16()?);
    let space = r.u16()?;
    let mut lib = SwatchLibrary { name: if title.trim().is_empty() { name.into() } else { title.trim().to_string() }, ..Default::default() };
    let mut names = vec![];
    for _ in 0..count.min(MAX_COLORS) {
        let label = acb_string(&mut r)?;
        let _code = r.take(6)?;
        let color = match space {
            0 => Color::rgb8(r.u8()?, r.u8()?, r.u8()?),
            // Ink amounts stored inverted: 255 is no ink.
            2 => {
                let mut ink = || r.u8().map(|v| f32::from(255 - v) / 255.0);
                Color::cmyk(ink()?, ink()?, ink()?, ink()?)
            }
            7 => {
                let l = f32::from(r.u8()?) / 255.0 * 100.0;
                let (a, b) = (f32::from(r.u8()?) - 128.0, f32::from(r.u8()?) - 128.0);
                Color::lab(l, a, b)
            }
            _ => return None,
        };
        // Pages are padded with unnamed entries.
        if label.trim().is_empty() {
            continue;
        }
        let full = format!("{prefix}{label}{postfix}").trim().to_string();
        lib.swatches.push(Swatch { name: unique(full, &mut names), paint: Paint::solid(color), global: true, spot: true });
    }
    // A trailing "spflspot" / "spflproc" marks the book spot or process.
    if let Some(i) = bytes.windows(8).rposition(|w| w.starts_with(b"spfl"))
        && bytes.get(i + 4..i + 8) == Some(b"proc".as_slice())
    {
        for w in &mut lib.swatches {
            (w.spot, w.global) = (false, false);
        }
    }
    Some(lib)
}

// ---------- .aco ----------

fn aco_color(space: u16, w: u16, x: u16, y: u16, z: u16) -> Option<Color> {
    let f = |v: u16| f32::from(v) / 65535.0;
    Some(match space {
        0 => Color::rgb(f(w), f(x), f(y)),
        1 => Color::from_hsb(f(w) * 360.0, f(x), f(y)),
        // Ink stored inverted: 65535 is no ink.
        2 => Color::cmyk(1.0 - f(w), 1.0 - f(x), 1.0 - f(y), 1.0 - f(z)),
        7 => Color::lab((f32::from(w) / 100.0).clamp(0.0, 100.0), f32::from(x as i16) / 100.0, f32::from(y as i16) / 100.0),
        8 => Color::gray((f32::from(w) / 10000.0).clamp(0.0, 1.0)),
        _ => return None,
    })
}

fn read_aco(bytes: &[u8], name: &str) -> Option<SwatchLibrary> {
    let mut r = Reader::new(bytes);
    let mut version = r.u16()?;
    let mut count = usize::from(r.u16()?);
    // Version 1 (unnamed) comes first; a version 2 section after it repeats the colours with names.
    if version == 1 {
        r.take(count.checked_mul(10)?)?;
        match (r.u16(), r.u16()) {
            (Some(2), Some(c)) => (version, count) = (2, usize::from(c)),
            _ => {
                r = Reader::new(bytes);
                r.take(4)?;
            }
        }
    }
    let mut lib = SwatchLibrary { name: name.into(), ..Default::default() };
    let mut names = vec![];
    for _ in 0..count.min(MAX_COLORS) {
        let (space, w, x, y, z) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        let label = if version == 2 {
            let n = r.u32()? as usize;
            r.utf16(n)?
        } else {
            String::new()
        };
        let Some(color) = aco_color(space, w, x, y, z) else { continue };
        let label = if label.trim().is_empty() { color.to_hex() } else { label };
        lib.swatches.push(Swatch { name: unique(label, &mut names), paint: Paint::solid(color), global: false, spot: false });
    }
    Some(lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_be_bytes).collect()
    }

    fn ase_color(name: &str, model: &[u8; 4], v: &[f32], kind: u16) -> Vec<u8> {
        let mut d = vec![];
        d.extend(((name.encode_utf16().count() + 1) as u16).to_be_bytes());
        d.extend(utf16(name));
        d.extend(model);
        for x in v {
            d.extend(x.to_be_bytes());
        }
        d.extend(kind.to_be_bytes());
        let mut b = 1u16.to_be_bytes().to_vec();
        b.extend((d.len() as u32).to_be_bytes());
        b.extend(d);
        b
    }

    fn block(kind: u16, d: Vec<u8>) -> Vec<u8> {
        let mut b = kind.to_be_bytes().to_vec();
        b.extend((d.len() as u32).to_be_bytes());
        b.extend(d);
        b
    }

    #[test]
    fn ase_reads_models_kinds_and_groups() {
        let mut f = b"ASEF".to_vec();
        f.extend([0, 1, 0, 0]);
        f.extend(5u32.to_be_bytes());
        f.extend(ase_color("Ink Red", b"CMYK", &[0.0, 1.0, 1.0, 0.0], 1));
        let mut g = 6u16.to_be_bytes().to_vec();
        g.extend(utf16("Brand"));
        f.extend(block(0xC001, g));
        f.extend(ase_color("Sky", b"RGB ", &[0.0, 0.5, 1.0], 0));
        f.extend(ase_color("Clay", b"LAB ", &[0.5, 20.0, 30.0], 2));
        f.extend(block(0xC002, vec![]));
        let lib = read(&f, "lib").unwrap();
        assert_eq!(lib.swatches.len(), 1);
        let red = &lib.swatches[0];
        assert!(red.spot && red.global && red.name == "Ink Red");
        assert_eq!(red.paint.color(), Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)));
        assert_eq!(lib.groups.len(), 1);
        assert_eq!(lib.groups[0].name, "Brand");
        assert_eq!(lib.groups[0].swatches[0].paint.color(), Some(Color::rgb(0.0, 0.5, 1.0)));
        assert!(lib.groups[0].swatches[0].global && !lib.groups[0].swatches[0].spot);
        assert_eq!(lib.groups[0].swatches[1].paint.color(), Some(Color::lab(50.0, 20.0, 30.0)));
    }

    fn acb_string(s: &str) -> Vec<u8> {
        let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut b = ((units.len() / 2) as u32).to_be_bytes().to_vec();
        b.extend(units);
        b
    }

    #[test]
    fn acb_reads_a_spot_colour_book() {
        let mut f = b"8BCB".to_vec();
        f.extend([0, 1, 0, 42]);
        f.extend(acb_string("$$$/colorbook/Test/title=Test Book"));
        f.extend(acb_string("TEST "));
        f.extend(acb_string(" C"));
        f.extend(acb_string(""));
        f.extend(3u16.to_be_bytes());
        f.extend([0, 7, 0, 0]);
        f.extend(2u16.to_be_bytes()); // CMYK
        for (n, cmyk) in [("100", [0u8, 255, 255, 255]), ("", [255, 255, 255, 255]), ("200", [255, 0, 255, 255])] {
            f.extend(acb_string(n));
            f.extend(b"000000");
            f.extend(cmyk);
        }
        f.extend(b"spflspot");
        let lib = read(&f, "x").unwrap();
        assert_eq!(lib.name, "Test Book");
        let names: Vec<&str> = lib.swatches.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["TEST 100 C", "TEST 200 C"], "padding entries are skipped");
        assert!(lib.swatches.iter().all(|w| w.spot && w.global));
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::cmyk(1.0, 0.0, 0.0, 0.0)));
    }

    #[test]
    fn aco_reads_named_colours_from_version_2() {
        let mut f = vec![];
        let colors: [(u16, [u16; 4], &str); 2] = [(0, [65535, 0, 0, 0], "Red"), (8, [5000, 0, 0, 0], "Half")];
        f.extend(1u16.to_be_bytes());
        f.extend(2u16.to_be_bytes());
        for (s, v, _) in colors {
            f.extend(s.to_be_bytes());
            v.iter().for_each(|x| f.extend(x.to_be_bytes()));
        }
        f.extend(2u16.to_be_bytes());
        f.extend(2u16.to_be_bytes());
        for (s, v, n) in colors {
            f.extend(s.to_be_bytes());
            v.iter().for_each(|x| f.extend(x.to_be_bytes()));
            f.extend(((n.len() + 1) as u32).to_be_bytes());
            f.extend(utf16(n));
        }
        let lib = read(&f, "pal").unwrap();
        assert_eq!(lib.name, "pal");
        assert_eq!(lib.swatches[0].name, "Red");
        assert_eq!(lib.swatches[0].paint.color(), Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(lib.swatches[1].paint.color(), Some(Color::gray(0.5)));
    }

    #[test]
    fn junk_and_truncated_files_are_errors_not_panics() {
        for f in [&b"ASEF"[..], b"ASEF\0\x01\0\0\xff\xff\xff\xff\0\x01", b"8BCB\0\x01", b"\0\x01\0\x05", b"hello", &[]] {
            assert!(read(f, "x").is_err());
        }
        // Every prefix of a valid file reads or fails cleanly.
        let mut f = b"ASEF".to_vec();
        f.extend([0, 1, 0, 0]);
        f.extend(1u32.to_be_bytes());
        f.extend(ase_color("A", b"RGB ", &[1.0, 0.0, 0.0], 2));
        for n in 0..f.len() {
            let _ = read(&f[..n], "x");
        }
    }
}
