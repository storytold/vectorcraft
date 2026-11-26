//! The widths a PDF's pages give the characters of the fonts they set type in, read from the font
//! dictionaries alone (no page is drawn): a simple font's `Widths` from its `FirstChar`, or a
//! composite font's descendant's `W`, each code's character from the font's `ToUnicode` map, or for
//! a simple font without one whose `Encoding` is `WinAnsiEncoding`, from that encoding (ISO 32000-1
//! §9.6.2, §9.6.6, §9.7.4.3, §9.10.3, Annex D.2).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use craft::pdf::interpret::hayro_cmap::{BfString, CMap};
use craft::pdf::syntax::object::{Array, Dict, Name, ObjRef, ObjectIdentifier, Stream};
use craft::pdf::syntax::page::Resources;

use crate::import::replacement;
use crate::import_scan::{cid_ranges, font_name, without_subset_tag};

/// The most fonts read.
const MAX_FONTS: usize = 256;
/// The most form XObjects looked into, and how deep they may nest.
const MAX_FORMS: usize = 4096;
const MAX_FORM_DEPTH: u32 = 16;
/// The most entries of font and XObject resource dictionaries looked at, all pages and forms of the
/// file together (one budget for the whole file, not one a page).
const MAX_KEYS: usize = 1 << 16;
/// The most bytes of width arrays (`Widths`, `W`) read, all fonts together; an array that fonts
/// share by reference is read once. A subset's array is a few kilobytes.
const MAX_WIDTH_TEXT: usize = 4 << 20;
/// The most characters of a font kept.
const MAX_CHARS: usize = 4096;
/// The largest `ToUnicode` map read, decoded: a subset's map names a few hundred codes. A longer one
/// is left unread.
const MAX_TO_UNICODE: usize = 256 << 10;
/// The most bytes of `ToUnicode` maps decoded, all fonts together.
const MAX_TO_UNICODE_TOTAL: usize = 4 << 20;
/// The longest string a `ToUnicode` map read may hold, decoded, in bytes: a destination is a
/// character or a short run of them (a ligature's), and each lookup of a code in a range copies its
/// range's whole destination. A map with a longer one is left unread.
const MAX_DESTINATION: usize = 128;
/// The most codes of a composite font whose characters are looked up (its first, in `W` order):
/// [`MAX_CHARS`] bounds the characters kept, not the codes looked at to find them.
const MAX_LOOKUPS: usize = 4 * MAX_CHARS;
/// How far apart two widths a file gives one character of one font may be and still be one width,
/// in thousandths of an em: files write widths rounded.
const SAME_WIDTH: f32 = 1.0;

/// The widths a page gives the characters of one font, in thousandths of an em, by character.
pub type FontWidths = BTreeMap<char, f32>;

/// The advance widths, in thousandths of an em, that a PDF's pages give the characters of the fonts
/// they set type in, by each font's PostScript name (its `BaseFont` without a subset prefix): what
/// the copy of the font the file was set in draws them at. A `.ai`'s PDF part has them for the type
/// its text document holds, so they tell installed copies of one font apart. A font whose subsets
/// give one character two widths gives none. Cloning it, or a font's widths, shares them.
#[derive(Clone, Debug, Default)]
pub struct PageWidths {
    fonts: Arc<HashMap<String, Arc<FontWidths>>>,
}

impl PageWidths {
    /// Each character the pages set in the font of PostScript name `name` (exactly, without a
    /// subset prefix) and its width (at most 4096 characters): `None` when the pages set none in
    /// that font, or its subsets give a character two widths.
    pub fn widths(&self, name: &str) -> Option<&Arc<FontWidths>> {
        self.fonts.get(name)
    }
}

/// Which dictionary `d` is, to look at it once: where its text lies in the file as read. One
/// dictionary that pages or forms share is read from the same place each time (one in an object
/// stream lies in that stream's decoded data, which the reader keeps); telling it by its contents
/// would hash it again for each of them.
fn dict_key(d: &Dict<'_>) -> (usize, usize) {
    let text = d.data();
    (text.as_ptr() as usize, text.len())
}

/// Each CID a composite font's `W` gives a width, and that width.
type CidWidths = Rc<[(u32, f32)]>;

/// What [`page_widths`] has read so far.
#[derive(Default)]
struct Walk {
    /// Each font's characters and widths; `None` when two subsets disagree.
    fonts: HashMap<String, Option<FontWidths>>,
    /// The narrowest and widest width given each character of each font so far.
    bounds: HashMap<String, BTreeMap<char, (f32, f32)>>,
    /// The font dictionaries, form XObjects and resource dictionaries already looked at.
    seen_fonts: HashSet<(usize, usize)>,
    seen_forms: HashSet<ObjectIdentifier>,
    walked: HashSet<(usize, usize)>,
    /// The `ToUnicode` maps parsed, by their stream: `None` for one that can't be read.
    maps: HashMap<ObjectIdentifier, Option<Rc<CMap>>>,
    /// The width arrays that fonts name by reference, read: a simple font's `Widths` (its first 256
    /// numbers) and a composite font's `W` (each CID and its width).
    simple_widths: HashMap<ObjRef, Option<Rc<[f32]>>>,
    cid_widths: HashMap<ObjRef, Option<CidWidths>>,
    /// How many entries of resource dictionaries have been looked at.
    looked: usize,
    /// How many bytes of `ToUnicode` maps have been decoded.
    decoded: usize,
    /// How many width arrays have been read, and how many bytes of them.
    arrays_read: usize,
    width_text: usize,
}

impl Walk {
    /// Note `widths`, characters of the font of PostScript name `name` and their widths. All the
    /// widths given one character, by this and the other subsets, must be the same: no two more
    /// than [`SAME_WIDTH`] apart.
    fn add(&mut self, name: String, widths: Vec<(char, f32)>) {
        let slot = self.fonts.entry(name.clone()).or_insert_with(|| Some(BTreeMap::new()));
        let Some(have) = slot else { return };
        let bounds = self.bounds.entry(name).or_default();
        for (c, w) in widths {
            if let Some((lo, hi)) = bounds.get_mut(&c) {
                (*lo, *hi) = (lo.min(w), hi.max(w));
                if *hi - *lo > SAME_WIDTH {
                    *slot = None;
                    return;
                }
            } else if have.len() < MAX_CHARS {
                have.insert(c, w);
                bounds.insert(c, (w, w));
            }
        }
    }

    /// Look at one more entry of a resource dictionary: `false` once [`MAX_KEYS`] have been.
    fn look(&mut self) -> bool {
        self.looked += 1;
        self.looked <= MAX_KEYS
    }

    /// The `ToUnicode` map `s`, parsed once however many fonts name it: `None` when it decodes to
    /// more than [`MAX_TO_UNICODE`] bytes, when the maps decoded so far reach
    /// [`MAX_TO_UNICODE_TOTAL`], when it holds a string longer than [`MAX_DESTINATION`], or when it
    /// can't be read. A map is charged what was decoded of it,
    /// at most the limit: a stream written plain comes back whole without being decoded.
    fn character_map(&mut self, s: &Stream<'_>) -> Option<Rc<CMap>> {
        let id = s.obj_id();
        if !self.maps.contains_key(&id) {
            let left = MAX_TO_UNICODE_TOTAL.saturating_sub(self.decoded).min(MAX_TO_UNICODE + 1);
            let data = s.decoded_within(left).ok();
            self.decoded += data.as_ref().map_or(left, |d| d.len().min(left));
            let map = data
                .filter(|d| d.len() < left && d.len() <= MAX_TO_UNICODE && !d.starts_with(b"bcmap") && longest_string(d) <= MAX_DESTINATION)
                .and_then(|d| CMap::parse(&d, |_| None));
            self.maps.insert(id, map.map(Rc::new));
        }
        self.maps.get(&id)?.clone()
    }

    /// Count `array`'s text against [`MAX_WIDTH_TEXT`]: `None` once the width arrays read reach it.
    fn width_array<'a>(&mut self, array: Option<Array<'a>>) -> Option<Array<'a>> {
        let array = array.filter(|_| self.width_text < MAX_WIDTH_TEXT)?;
        self.arrays_read += 1;
        self.width_text += array.data().len();
        Some(array)
    }

    /// The first 256 numbers of simple font `f`'s `Widths`, read once for all the fonts that name
    /// the same array.
    fn simple_widths(&mut self, f: &Dict<'_>) -> Option<Rc<[f32]>> {
        let read = |w: &mut Self| w.width_array(f.get::<Array<'_>>(b"Widths")).map(|a| a.iter::<f32>().take(256).collect::<Rc<[f32]>>());
        let Some(r) = f.get_ref(b"Widths") else { return read(self) };
        if let Some(have) = self.simple_widths.get(&r) {
            return have.clone();
        }
        let got = read(self);
        self.simple_widths.insert(r, got.clone());
        got
    }

    /// Each CID that composite font descendant `d`'s `W` gives a width, and the width, read once for all
    /// the fonts that name the same array.
    fn cid_widths(&mut self, d: &Dict<'_>) -> Option<CidWidths> {
        let read = |w: &mut Self| {
            let a = w.width_array(d.get::<Array<'_>>(b"W"))?;
            let mut out = Vec::new();
            cid_ranges(&a, 1, |cid, v| {
                if let [w0] = v {
                    out.push((cid, *w0 as f32));
                }
            });
            Some(out.into())
        };
        let Some(r) = d.get_ref(b"W") else { return read(self) };
        if let Some(have) = self.cid_widths.get(&r) {
            return have.clone();
        }
        let got = read(self);
        self.cid_widths.insert(r, got.clone());
        got
    }
}

/// [`PageWidths`] of the fonts of `pages` (each page's resources) and of the forms they draw.
pub(crate) fn page_widths<'a>(pages: impl IntoIterator<Item = &'a Resources<'a>>) -> PageWidths {
    let walk = walk(pages);
    let fonts = walk.fonts.into_iter().filter_map(|(name, w)| Some((name, Arc::new(w.filter(|w| !w.is_empty())?)))).collect();
    PageWidths { fonts: Arc::new(fonts) }
}

/// Read the fonts of `pages` and of the forms they draw. Each resource dictionary is walked once,
/// though pages and forms share it; a form without resources of its own takes those of what draws
/// it, which are walked already. At most [`MAX_KEYS`] entries are looked at.
fn walk<'a>(pages: impl IntoIterator<Item = &'a Resources<'a>>) -> Walk {
    let mut w = Walk::default();
    let mut stack: Vec<(Resources<'a>, u32)> = pages.into_iter().map(|r| (r.clone(), 0)).collect();
    while let Some((res, depth)) = stack.pop() {
        if w.walked.insert(dict_key(&res.fonts)) {
            for key in res.fonts.keys() {
                if w.seen_fonts.len() >= MAX_FONTS || !w.look() {
                    break;
                }
                let Some(f) = res.fonts.get::<Dict<'_>>(key.as_ref()) else { continue };
                if w.seen_fonts.insert(dict_key(&f))
                    && let Some((name, widths)) = font_widths(&f, &mut w)
                {
                    w.add(name, widths);
                }
            }
        }
        if depth >= MAX_FORM_DEPTH || !w.walked.insert(dict_key(&res.x_objects)) {
            continue;
        }
        for key in res.x_objects.keys() {
            if w.seen_forms.len() >= MAX_FORMS || !w.look() {
                break;
            }
            let Some(s) = res.x_objects.get::<Stream<'_>>(key.as_ref()) else { continue };
            let d = s.dict();
            let form = d.get::<Name<'_>>(b"Subtype").is_some_and(|t| t.as_ref() == b"Form");
            if let (true, Some(own)) = (form, d.get::<Dict<'_>>(b"Resources"))
                && w.seen_forms.insert(s.obj_id())
            {
                stack.push((Resources::from_scope(Some(own), res.clone()), depth + 1));
            }
        }
    }
    w
}

/// The PostScript name of font `f` (its `BaseFont` without a subset prefix) and the width of each
/// character it names, at most [`MAX_CHARS`] (of a composite font, among its first [`MAX_LOOKUPS`]
/// codes with a width). A code's character comes from the font's `ToUnicode`
/// map or, for a simple font without one whose `Encoding` is `WinAnsiEncoding`, from that encoding;
/// a composite font's codes are its CIDs, so only under an `Identity-H` or `Identity-V` CMap. A
/// width of 0 is a code the subset doesn't use, and is left out. `None` for a Type 3 font (its
/// widths are in its own glyph space) and a font whose codes' characters can't be told.
fn font_widths(f: &Dict<'_>, walk: &mut Walk) -> Option<(String, Vec<(char, f32)>)> {
    let subtype = f.get::<Name<'_>>(b"Subtype")?;
    if subtype.as_ref() == b"Type3" {
        return None;
    }
    let name = without_subset_tag(&font_name(f)?).to_string();
    let composite = subtype.as_ref() == b"Type0";
    let encoding = f.get::<Name<'_>>(b"Encoding");
    // Only a font without a `ToUnicode` map falls back to its encoding: one whose map can't be
    // read gives none.
    let to_unicode = match f.get::<Stream<'_>>(b"ToUnicode") {
        Some(s) => Some(walk.character_map(&s)?),
        None if f.contains_key(b"ToUnicode") => return None,
        None => None,
    };
    let win_ansi = !composite && encoding.as_ref().is_some_and(|e| e.as_ref() == b"WinAnsiEncoding");
    if to_unicode.is_none() && !win_ansi {
        return None;
    }
    let char_of = |code: u32| match &to_unicode {
        Some(map) => match map.lookup_bf_string(code).filter(|u| !replacement(u)) {
            Some(BfString::Char(c)) => Some(c),
            _ => None,
        },
        None => win_ansi_char(code),
    };
    let width = |w: f32| (w.is_finite() && w > 0.0 && w < 1e5).then_some(w);
    let mut widths = Vec::new();
    if composite {
        if !encoding.is_some_and(|e| matches!(e.as_ref(), b"Identity-H" | b"Identity-V")) {
            return None;
        }
        let d = f.get::<Array<'_>>(b"DescendantFonts").and_then(|a| a.iter::<Dict<'_>>().next())?;
        let mut looked = 0;
        for (cid, w) in walk.cid_widths(&d)?.iter() {
            if widths.len() >= MAX_CHARS || looked >= MAX_LOOKUPS {
                break;
            }
            let Some(w) = width(*w) else { continue };
            looked += 1;
            if let Some(c) = char_of(*cid) {
                widths.push((c, w));
            }
        }
    } else {
        let first = f.get::<u32>(b"FirstChar")?;
        // A simple font's codes are bytes.
        for (code, w) in (first..256).zip(walk.simple_widths(f)?.iter()) {
            let Some(w) = width(*w) else { continue };
            if let Some(c) = char_of(code) {
                widths.push((c, w));
            }
        }
    }
    Some((name, widths))
}

/// The decoded length, in bytes, of the longest string in CMap text `data`: a hex string's digits
/// halved, or a literal string's bytes (escapes counted as written, so at least its length). A `<<`
/// opens a dictionary, not a string, and a `%` outside a string starts a comment. An ASCII85
/// string (`<~`), which no `ToUnicode` map writes and whose `z` stands for four bytes, counts as
/// longer than any.
fn longest_string(data: &[u8]) -> usize {
    let mut longest = 0;
    let mut i = 0;
    while let Some(&b) = data.get(i) {
        i += 1;
        let len = match b {
            b'%' => {
                while data.get(i).is_some_and(|b| !matches!(b, b'\n' | b'\r')) {
                    i += 1;
                }
                continue;
            }
            b'<' if data.get(i) == Some(&b'~') => return usize::MAX,
            b'<' if data.get(i) == Some(&b'<') => {
                i += 1;
                continue;
            }
            b'<' => {
                let mut digits = 0;
                while let Some(&b) = data.get(i) {
                    i += 1;
                    if b == b'>' {
                        break;
                    }
                    digits += usize::from(b.is_ascii_hexdigit());
                }
                digits.div_ceil(2)
            }
            b'(' => {
                let (mut depth, mut len) = (1usize, 0);
                while let Some(&b) = data.get(i) {
                    i += 1;
                    match b {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    len += 1;
                }
                len
            }
            _ => continue,
        };
        longest = longest.max(len);
    }
    longest
}

/// The character of code `code` in `WinAnsiEncoding` (ISO 32000-1 Annex D.2), which is Windows-1252
/// but for the codes it leaves unused: `None` for those, for the controls, and for 160 and 173, which
/// it gives the space and the hyphen a second time.
fn win_ansi_char(code: u32) -> Option<char> {
    /// Codes 128 to 159; `\0` where the encoding has no character.
    const HIGH: [char; 32] = [
        '\u{20AC}', '\0', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}',
        '\u{152}', '\0', '\u{17D}', '\0', '\0', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{2DC}',
        '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\0', '\u{17E}', '\u{178}',
    ];
    match code {
        0x20..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF => char::from_u32(code),
        0x80..=0x9F => HIGH.get(usize::try_from(code - 0x80).ok()?).copied().filter(|c| *c != '\0'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};

    /// A simple font named `name`, in `encoding`, whose widths from code `first` on are `widths`.
    fn simple(subtype: &str, name: &str, encoding: &str, first: u32, widths: &str) -> String {
        format!("<< /Type /Font /Subtype /{subtype} /BaseFont /{name} /Encoding /{encoding} /FirstChar {first} /Widths [{widths}] >>")
    }

    /// The pages' fonts are read from their resources and from the forms they draw, a subset's
    /// prefix aside: a simple font's characters from `WinAnsiEncoding` (a code 0 wide is left out)
    /// or its `ToUnicode` map, a composite font's from its `ToUnicode` map, by CID. The subsets of one
    /// font come together; a font whose subsets give a character two widths, a font in another
    /// encoding without a `ToUnicode` map, and a Type 3 font give none.
    #[test]
    fn the_pages_give_each_font_the_widths_of_its_characters() {
        let extra = first_extra(2);
        let fonts = [
            simple("TrueType", "ABCDEF+Sample-Regular", "WinAnsiEncoding", 65, "600 0 520.5"),
            simple("Type1", "GHIJKL+Sample-Regular", "WinAnsiEncoding", 128, "700"),
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /MNOPQR+Wide-Bold-Identity-H /Encoding /Identity-H /ToUnicode {extra} 0 R \
                 /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /MNOPQR+Wide-Bold /W [3 [250 610]] >>] >>"
            ),
            simple("Type1", "Roman-Mac", "MacRomanEncoding", 65, "500"),
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /Mapped /Encoding /MacRomanEncoding /FirstChar 3 /Widths [333] /ToUnicode {extra} 0 R >>"
            ),
            "<< /Type /Font /Subtype /Type3 /BaseFont /Drawn /Encoding /WinAnsiEncoding /FirstChar 65 /Widths [1] /FontMatrix [1 0 0 1 0 0] >>"
                .to_string(),
            simple("TrueType", "STUVWX+Clash", "WinAnsiEncoding", 65, "640"),
        ];
        let resources = |fonts: &[String], more: &str| {
            let list: Vec<String> = fonts.iter().enumerate().map(|(i, f)| format!("/F{i} {f}")).collect();
            format!("/Font << {} >> {more}", list.join(" "))
        };
        let first = PdfPage { resources: resources(&fonts, &format!("/XObject << /Fm0 {} 0 R >>", extra + 1)), ..PdfPage::new(100.0, 100.0, "") };
        let second = PdfPage {
            resources: resources(&[simple("TrueType", "YZABCD+Clash", "WinAnsiEncoding", 65, "600")], ""),
            ..PdfPage::new(100.0, 100.0, "")
        };
        let cmap =
            "begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n2 beginbfchar\n<0003> <0020>\n<0004> <03A9>\nendbfchar\nendcmap\n";
        let form_font = simple("TrueType", "EFGHIJ+Formed", "WinAnsiEncoding", 97, "480");
        let file = pdf_with(
            &[first, second],
            &[
                &format!("<< /Length {} >>\nstream\n{cmap}endstream", cmap.len()),
                &format!(
                    "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Font << /F0 {form_font} >> >> /Length 0 >>\nstream\n\nendstream"
                ),
            ],
            None,
        );
        let widths = crate::open_pdf(&file, None).unwrap().page_widths();
        let got = |name: &str| widths.widths(name).map(|w| w.iter().map(|(c, w)| (*c, *w)).collect::<Vec<_>>());
        assert_eq!(got("Sample-Regular"), Some(vec![('A', 600.0), ('C', 520.5), ('€', 700.0)]));
        assert_eq!(got("Wide-Bold"), Some(vec![(' ', 250.0), ('Ω', 610.0)]));
        assert_eq!(got("Mapped"), Some(vec![(' ', 333.0)]));
        assert_eq!(got("Formed"), Some(vec![('a', 480.0)]));
        for none in ["ABCDEF+Sample-Regular", "Roman-Mac", "Drawn", "Clash", "Missing"] {
            assert_eq!(got(none), None, "{none}");
        }
    }

    /// [`super::walk`] of the pages of `file`.
    fn walked(file: &[u8]) -> super::Walk {
        let o = crate::open_pdf(file, None).unwrap();
        let pages = o.pdf.pages();
        super::walk(pages.iter().map(|p| p.resources()))
    }

    /// Each resource dictionary is walked once: forms without resources of their own (which take
    /// those of what draws them), forms that share one, and pages that share one look at each
    /// entry once, so a file of thousands of them reads in time in proportion to it.
    #[test]
    fn shared_resources_are_walked_once() {
        let n = 4000;
        let extra = first_extra(1);
        let font = simple("TrueType", "Shared", "WinAnsiEncoding", 65, "500");
        // Forms without resources, then forms that share one resource dictionary (the last object).
        let shared = extra + 2 * n;
        let forms: Vec<String> = (0..2 * n)
            .map(|i| {
                let own = if i < n { String::new() } else { format!("/Resources {shared} 0 R ") };
                format!("<< /Type /XObject /Subtype /Form /BBox [0 0 1 {i}] {own}/Length 0 >>\nstream\n\nendstream")
            })
            .chain(std::iter::once(format!("<< /Font << /F0 {} >> >>", simple("TrueType", "Formed", "WinAnsiEncoding", 66, "600"))))
            .collect();
        let names: Vec<String> = (0..2 * n).map(|i| format!("/X{i} {} 0 R", extra + i)).collect();
        let page = PdfPage { resources: format!("/Font << /F0 {font} >> /XObject << {} >>", names.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let refs: Vec<&str> = forms.iter().map(String::as_str).collect();
        let w = walked(&pdf_with(&[page], &refs, None));
        assert!(w.fonts.contains_key("Shared") && w.fonts.contains_key("Formed"));
        // The page's font and forms, and the shared dictionary's font.
        assert!(w.looked <= 1 + 2 * n + 1, "{} entries looked at", w.looked);
        // A thousand pages that share one font dictionary of 4000 entries.
        let pages_n = 1000;
        let extra = first_extra(pages_n);
        let entries: Vec<String> = (0..n).map(|i| format!("/F{i} {} 0 R", extra + 1 + i % 50)).collect();
        let objects: Vec<String> = std::iter::once(format!("<< {} >>", entries.join(" ")))
            .chain((0..50).map(|i| simple("TrueType", &format!("S{i}"), "WinAnsiEncoding", 65, "500")))
            .collect();
        let pages: Vec<PdfPage> =
            (0..pages_n).map(|_| PdfPage { resources: format!("/Font {extra} 0 R"), ..PdfPage::new(100.0, 100.0, "") }).collect();
        let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
        let w = walked(&pdf_with(&pages, &refs, None));
        assert_eq!(w.fonts.len(), 50);
        assert!(w.looked <= n, "{} entries looked at", w.looked);
    }

    /// Past [`super::MAX_FONTS`] fonts no more entries are looked at.
    #[test]
    fn fonts_past_the_most_read_are_not_looked_at() {
        let fonts: Vec<String> =
            (0..super::MAX_FONTS + 100).map(|i| format!("/F{i} {}", simple("TrueType", &format!("N{i}"), "WinAnsiEncoding", 65, "500"))).collect();
        let page = PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let w = walked(&pdf_with(&[page], &[], None));
        assert_eq!(w.fonts.len(), super::MAX_FONTS);
        assert!(w.looked <= super::MAX_FONTS + 1, "{} entries looked at", w.looked);
    }

    /// Past [`super::MAX_KEYS`] entries of resource dictionaries, none are looked at: a font
    /// dictionary of more entries than that, all naming one font, is looked at that far.
    #[test]
    fn entries_past_the_most_looked_at_are_left() {
        let extra = first_extra(1);
        let entries: Vec<String> = (0..super::MAX_KEYS + 1000).map(|i| format!("/F{i} {extra} 0 R")).collect();
        let page = PdfPage { resources: format!("/Font << {} >>", entries.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let w = walked(&pdf_with(&[page], &[&simple("TrueType", "Many", "WinAnsiEncoding", 65, "500")], None));
        assert_eq!(w.fonts.len(), 1);
        assert!(w.looked <= super::MAX_KEYS + 1, "{} entries looked at", w.looked);
    }

    /// A width array that fonts share by reference is read once for all of them: 128 composite
    /// fonts naming one `W` and 128 simple fonts naming one `Widths`, each of 100,000 numbers.
    #[test]
    fn a_width_array_fonts_share_is_read_once() {
        let numbers = 100_000;
        let big: String = std::iter::repeat_n("500 ", numbers).collect();
        let extra = first_extra(1);
        let cmap = "begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfrange\n<0000> <00FF> <0041>\nendbfrange\nendcmap\n";
        let fonts: Vec<String> = (0..256)
            .map(|i| {
                if i % 2 == 0 {
                    format!(
                        "/F{i} << /Type /Font /Subtype /Type0 /BaseFont /W{i} /Encoding /Identity-H /ToUnicode {} 0 R \
                         /DescendantFonts [<< /Subtype /CIDFontType2 /W {extra} 0 R >>] >>",
                        extra + 1
                    )
                } else {
                    format!(
                        "/F{i} << /Type /Font /Subtype /TrueType /BaseFont /S{i} /Encoding /WinAnsiEncoding /FirstChar 65 /Widths {} 0 R >>",
                        extra + 2
                    )
                }
            })
            .collect();
        let page = PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let file = pdf_with(
            &[page],
            &[&format!("[0 [{big}]]"), &format!("<< /Length {} >>\nstream\n{cmap}endstream", cmap.len()), &format!("[{big}]")],
            None,
        );
        let w = walked(&file);
        assert_eq!(w.arrays_read, 2);
        assert!(w.width_text <= 2 * (big.len() + 10), "{} bytes of width arrays read", w.width_text);
        assert_eq!(w.fonts.len(), 256);
        assert!(w.fonts.values().all(|f| f.as_ref().is_some_and(|f| f.get(&'A') == Some(&500.0))));
    }

    /// Past [`super::MAX_WIDTH_TEXT`] bytes of width arrays no more are read: of six fonts each with
    /// an array of a million bytes, the sixth gives no widths.
    #[test]
    fn width_arrays_past_the_most_read_are_left() {
        let big: String = std::iter::repeat_n("500 ", 250_000).collect();
        let extra = first_extra(1);
        let fonts: Vec<String> = (0..6)
            .map(|i| {
                format!(
                    "/F{i} << /Type /Font /Subtype /TrueType /BaseFont /M{i} /Encoding /WinAnsiEncoding /FirstChar 65 /Widths {} 0 R >>",
                    extra + i
                )
            })
            .collect();
        let page = PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let arrays: Vec<String> = (0..6).map(|_| format!("[{big}]")).collect();
        let refs: Vec<&str> = arrays.iter().map(String::as_str).collect();
        let w = walked(&pdf_with(&[page], &refs, None));
        assert_eq!(w.arrays_read, 5);
        assert_eq!(w.fonts.values().filter(|f| f.as_ref().is_some_and(|f| !f.is_empty())).count(), 5);
    }

    /// Widths that drift past [`super::SAME_WIDTH`] over several subsets are two widths, though
    /// each lies within it of the one read before: subsets giving `A` 600, 600.8 and 601.6 give the
    /// font none, as do subsets giving `B` 501.5, 500.6 and 500. Two within it still give one. Each
    /// subset is on a page of its own, so they are read in turn (a font dictionary's entries come in
    /// no set order).
    #[test]
    fn widths_that_drift_apart_over_several_subsets_give_none() {
        let tags = ["AAAAAA", "BBBBBB", "CCCCCC"];
        let pages: Vec<PdfPage> = (0..3)
            .map(|i| {
                let mut fonts = vec![
                    format!("/D {}", simple("TrueType", &format!("{}+Drift", tags[i]), "WinAnsiEncoding", 65, ["600", "600.8", "601.6"][i])),
                    format!("/B {}", simple("TrueType", &format!("{}+Back", tags[i]), "WinAnsiEncoding", 65, ["0 501.5", "0 500.6", "0 500"][i])),
                ];
                if i < 2 {
                    fonts.push(format!("/S {}", simple("TrueType", &format!("{}+Steady", tags[i]), "WinAnsiEncoding", 65, ["600", "600.8"][i])));
                }
                PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") }
            })
            .collect();
        let widths = crate::open_pdf(&pdf_with(&pages, &[], None), None).unwrap().page_widths();
        assert!(widths.widths("Drift").is_none());
        assert!(widths.widths("Back").is_none());
        assert!(widths.widths("Steady").is_some_and(|w| w.contains_key(&'A')));
    }

    /// Only a simple font without a `ToUnicode` map takes its characters from `WinAnsiEncoding`:
    /// one whose map is left unread (longer than [`super::MAX_TO_UNICODE`]) or isn't a stream gives
    /// no widths.
    #[test]
    fn a_to_unicode_map_that_cant_be_read_gives_no_widths() {
        let mut big = String::from("begincmap\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n");
        while big.len() <= super::MAX_TO_UNICODE {
            big.push_str("1 beginbfchar\n<41> <0042>\nendbfchar\n");
        }
        big.push_str("endcmap\n");
        let extra = first_extra(1);
        let font = |name: &str, to_unicode: &str| {
            format!("<< /Type /Font /Subtype /TrueType /BaseFont /{name} /Encoding /WinAnsiEncoding /FirstChar 65 /Widths [600]{to_unicode} >>")
        };
        let page = PdfPage {
            resources: format!(
                "/Font << /F0 {} /F1 {} /F2 {} >>",
                font("Unread", &format!(" /ToUnicode {extra} 0 R")),
                font("Named", " /ToUnicode /Identity-H"),
                font("Plain", "")
            ),
            ..PdfPage::new(100.0, 100.0, "")
        };
        let file = pdf_with(&[page], &[&format!("<< /Length {} >>\nstream\n{big}endstream", big.len())], None);
        let widths = crate::open_pdf(&file, None).unwrap().page_widths();
        assert!(widths.widths("Unread").is_none());
        assert!(widths.widths("Named").is_none());
        assert_eq!(widths.widths("Plain").map(|w| w.get(&'A').copied()), Some(Some(600.0)));
    }

    /// A `ToUnicode` map with a destination longer than [`super::MAX_DESTINATION`] is left unread,
    /// though the whole map is under [`super::MAX_TO_UNICODE`]: each lookup of a code in its range
    /// would copy the whole destination. A hex destination of 60,000 characters, a literal one of
    /// 600, and ASCII85 ones (`z` stands for four bytes, and one can hide the rest of its line
    /// behind a `%` the pre-scan would take for a comment) are all left unread. A map with a long
    /// name in a dictionary (`<<`) and a comment with an unclosed `(`, more than the limit before
    /// the map's end, is still read.
    #[test]
    fn a_to_unicode_map_with_a_long_destination_is_left_unread() {
        let head = "begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfchar\n<0001> <0041>\nendbfchar\n";
        let range = |dst: &str| format!("{head}1 beginbfrange\n<0100> <01FF> {dst}\nendbfrange\nendcmap\n");
        let hex = range(&format!("<{}>", "0042".repeat(60_000)));
        let literal = range(&format!("({})", "B".repeat(600)));
        let zeros = range(&format!("<~{}~>", "z".repeat(1_000)));
        let hidden = format!("{head}<~>%~> 1 beginbfrange <0100> <01FF> <{}> endbfrange\nendcmap\n", "0042".repeat(300));
        let padding = "1 beginbfchar\n<0002> <0042>\nendbfchar\n".repeat(10);
        let short = format!(
            "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 /{} 1 >> def\n% (an unclosed comment\n{head}{padding}endcmap\n",
            "A".repeat(600)
        );
        assert!(hex.len() < super::MAX_TO_UNICODE);
        let extra = first_extra(1);
        let names = ["Hex", "Literal", "Zeros", "Hidden", "Short"];
        let fonts: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                format!(
                    "/{name} << /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /ToUnicode {} 0 R \
                     /DescendantFonts [<< /Subtype /CIDFontType2 /W [1 [500] 256 511 500] >>] >>",
                    extra + i
                )
            })
            .collect();
        let page = PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let maps: Vec<String> =
            [hex, literal, zeros, hidden, short].iter().map(|m| format!("<< /Length {} >>\nstream\n{m}endstream", m.len())).collect();
        let refs: Vec<&str> = maps.iter().map(String::as_str).collect();
        let file = pdf_with(&[page], &refs, None);
        let widths = crate::open_pdf(&file, None).unwrap().page_widths();
        for unread in ["Hex", "Literal", "Zeros", "Hidden"] {
            assert!(widths.widths(unread).is_none(), "{unread}");
        }
        let short = widths.widths("Short").map(|w| w.iter().map(|(c, w)| (*c, *w)).collect::<Vec<_>>());
        assert_eq!(short, Some(vec![('A', 500.0)]));
    }

    /// Past [`super::MAX_LOOKUPS`] codes of a composite font, no more are looked up, though few of
    /// them gave a character: of two fonts whose `W` gives all 65,536 CIDs a width, the one whose
    /// map gives CID 1 a character gives its width, and the one whose map only gives CID 20,000 one
    /// gives none.
    #[test]
    fn codes_past_the_most_looked_up_are_left() {
        let cmap = |code: &str| {
            format!("begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfchar\n<{code}> <0041>\nendbfchar\nendcmap\n")
        };
        const { assert!(20_000 > super::MAX_LOOKUPS) };
        let extra = first_extra(1);
        let font = |name: &str, map: usize| {
            format!(
                "/{name} << /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /ToUnicode {} 0 R \
                 /DescendantFonts [<< /Subtype /CIDFontType2 /W {} 0 R >>] >>",
                extra + map,
                extra + 2
            )
        };
        let page = PdfPage { resources: format!("/Font << {} {} >>", font("Near", 0), font("Far", 1)), ..PdfPage::new(100.0, 100.0, "") };
        let maps: Vec<String> =
            ["0001", "4E20"].iter().map(|c| cmap(c)).map(|m| format!("<< /Length {} >>\nstream\n{m}endstream", m.len())).collect();
        let file = pdf_with(&[page], &[&maps[0], &maps[1], "[0 65535 500]"], None);
        let widths = crate::open_pdf(&file, None).unwrap().page_widths();
        assert_eq!(widths.widths("Near").map(|w| w.get(&'A').copied()), Some(Some(500.0)));
        assert!(widths.widths("Far").is_none());
    }

    /// [`crate::import_scan::cid_ranges`] gives each CID of a `W` its values, a list's in turn (an
    /// incomplete last group left out) and a range's to each, and reads no more of a long list than
    /// the CIDs it takes.
    #[test]
    fn cid_ranges_give_each_cid_its_values() {
        let extra = first_extra(1);
        let long: String = std::iter::repeat_n("7 ", 70_000).collect();
        let arrays = ["[1 [10 20 30] 100 102 7]".to_string(), "[1 [1 2 3 4 5 6 7]]".to_string(), format!("[0 [{long}]]")];
        let page = PdfPage {
            resources: format!("/Properties << /A {extra} 0 R /B {} 0 R /C {} 0 R >>", extra + 1, extra + 2),
            ..PdfPage::new(10.0, 10.0, "")
        };
        let refs: Vec<&str> = arrays.iter().map(String::as_str).collect();
        let file = pdf_with(&[page], &refs, None);
        let o = crate::open_pdf(&file, None).unwrap();
        let pages = o.pdf.pages();
        let props = &pages.iter().next().unwrap().resources().properties;
        let ranges = |key: &str, per: usize| {
            let mut out = vec![];
            crate::import_scan::cid_ranges(&props.get::<craft::pdf::syntax::object::Array<'_>>(key).unwrap(), per, |cid, v| {
                out.push((cid, v.to_vec()))
            });
            out
        };
        let one = |v: &[(u32, f64)]| v.iter().map(|(c, w)| (*c, vec![*w])).collect::<Vec<_>>();
        assert_eq!(ranges("A", 1), one(&[(1, 10.0), (2, 20.0), (3, 30.0), (100, 7.0), (101, 7.0), (102, 7.0)]));
        assert_eq!(ranges("B", 3), [(1, vec![1.0, 2.0, 3.0]), (2, vec![4.0, 5.0, 6.0])]);
        assert_eq!(ranges("C", 1).len(), 65_536);
    }

    /// A `ToUnicode` map written plain and longer than the limits is charged no more than a
    /// decoded one could be: the small maps of the other fonts are still read.
    #[test]
    fn a_long_plain_map_leaves_the_others_read() {
        let mut big = String::from("begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        while big.len() <= super::MAX_TO_UNICODE_TOTAL + 1000 {
            big.push_str("1 beginbfchar\n<0001> <0041>\nendbfchar\n");
        }
        big.push_str("endcmap\n");
        let small = "begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfchar\n<0001> <0042>\nendbfchar\nendcmap\n";
        let extra = first_extra(1);
        let cid = |name: &str, map: usize| {
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /ToUnicode {} 0 R /DescendantFonts [<< /Subtype /CIDFontType2 /W [1 [500]] >>] >>",
                extra + map
            )
        };
        let n = 40;
        let fonts: Vec<String> =
            std::iter::once(format!("/A {}", cid("Big", 0))).chain((0..n).map(|i| format!("/F{i} {}", cid(&format!("Small{i}"), 1 + i)))).collect();
        let page = PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let objects: Vec<String> = std::iter::once(format!("<< /Length {} >>\nstream\n{big}endstream", big.len()))
            .chain((0..n).map(|_| format!("<< /Length {} >>\nstream\n{small}endstream", small.len())))
            .collect();
        let refs: Vec<&str> = objects.iter().map(String::as_str).collect();
        let w = walked(&pdf_with(&[page], &refs, None));
        assert!(w.fonts.get("Big").is_some_and(Option::is_none) || !w.fonts.contains_key("Big"));
        assert_eq!((0..n).filter(|i| w.fonts.get(&format!("Small{i}")).is_some_and(|f| f.as_ref().is_some_and(|m| !m.is_empty()))).count(), n);
        assert!(w.decoded <= super::MAX_TO_UNICODE + 1 + n * small.len(), "{} bytes charged", w.decoded);
    }

    /// A `ToUnicode` map is parsed once however many fonts name it, and one longer than
    /// [`super::MAX_TO_UNICODE`] decoded is left unread, whether its stream is written plain or
    /// compressed: its fonts give no widths.
    #[test]
    fn a_to_unicode_map_is_read_once_and_only_up_to_its_limit() {
        use std::io::Write as _;
        let small =
            "begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n1 beginbfchar\n<0001> <0041>\nendbfchar\nendcmap\n".to_string();
        let mut big = String::from("begincmap\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        while big.len() <= super::MAX_TO_UNICODE {
            big.push_str("1 beginbfchar\n<0001> <0041>\nendbfchar\n");
        }
        big.push_str("endcmap\n");
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(big.as_bytes()).unwrap();
        let hex: String = z.finish().unwrap().iter().map(|b| format!("{b:02X}")).collect();
        let extra = first_extra(1);
        let cid = |name: &str, map: usize| {
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /ToUnicode {} 0 R \
                 /DescendantFonts [<< /Subtype /CIDFontType2 /W [1 [500]] >>] >>",
                extra + map
            )
        };
        let fonts: Vec<String> = (0..64).map(|i| format!("/F{i} {}", cid(&format!("U{i}"), i % 3))).collect();
        let page = PdfPage { resources: format!("/Font << {} >>", fonts.join(" ")), ..PdfPage::new(100.0, 100.0, "") };
        let file = pdf_with(
            &[page],
            &[
                &format!("<< /Length {} >>\nstream\n{small}endstream", small.len()),
                &format!("<< /Length {} >>\nstream\n{big}endstream", big.len()),
                &format!("<< /Length {} /Filter [/ASCIIHexDecode /FlateDecode] >>\nstream\n{hex}>\nendstream", hex.len() + 1),
            ],
            None,
        );
        let w = walked(&file);
        assert_eq!(w.maps.len(), 3, "each map parsed once");
        // Each long one charged up to just past the limit.
        assert!(w.decoded <= small.len() + 2 * (super::MAX_TO_UNICODE + 1), "{} bytes charged", w.decoded);
        assert_eq!(w.maps.values().filter(|m| m.is_some()).count(), 1, "the long ones left unread");
        let widths = crate::open_pdf(&file, None).unwrap().page_widths();
        assert_eq!(widths.widths("U0").map(|w| w.iter().map(|(c, w)| (*c, *w)).collect::<Vec<_>>()), Some(vec![('A', 500.0)]));
        assert!(widths.widths("U1").is_none() && widths.widths("U2").is_none());
        // Every font of the name shares one map of its widths.
        assert!(std::sync::Arc::ptr_eq(widths.widths("U0").unwrap(), widths.clone().widths("U0").unwrap()));
    }
}
