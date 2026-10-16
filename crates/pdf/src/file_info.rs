//! File › File Info read from a PDF (and a PDF-compatible `.ai`): the document information
//! dictionary (ISO 32000-1 §14.3.3) and the XMP packet of the catalog's metadata stream (§14.3.2,
//! ISO 16684-1). Where both give a field, the XMP packet's value is taken; a packet that can't be
//! read leaves the information dictionary's values.

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{DateTime, Dict, Stream};
use roxmltree::Node;

use crate::import_scan::text_string;

/// The longest XMP packet read, in bytes once decoded; a longer one is left out.
const MAX_XMP: usize = 8 << 20;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const DC: &str = "http://purl.org/dc/elements/1.1/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/"; // brand-ok: the XMP namespace the files use
const XMP_RIGHTS: &str = "http://ns.adobe.com/xap/1.0/rights/"; // brand-ok: the XMP namespace the files use
const PDF: &str = "http://ns.adobe.com/pdf/1.3/"; // brand-ok: the XMP namespace the files use

/// The File Info fields a PDF gives, each `None` when the file gives none (or only blank text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
    /// XMP `dc:creator` (its names joined with ", "), else Info `Author`.
    pub author: Option<String>,
    /// XMP `dc:description` (its `x-default` item, else its first), else Info `Subject`.
    pub description: Option<String>,
    /// XMP `dc:subject`, else XMP `pdf:Keywords` or Info `Keywords` split at commas and semicolons.
    pub keywords: Option<Vec<String>>,
    /// XMP `dc:rights` (its `x-default` item, else its first).
    pub copyright_notice: Option<String>,
    /// XMP `xmpRights:WebStatement`.
    pub copyright_url: Option<String>,
    /// XMP `xmpRights:Marked`: true for a copyrighted work, false for a public-domain one.
    pub copyrighted: Option<bool>,
    /// XMP `xmp:CreateDate`, else Info `CreationDate`, as Unix seconds (UTC).
    pub created: Option<i64>,
    /// XMP `xmp:ModifyDate`, else Info `ModDate`, as Unix seconds (UTC).
    pub modified: Option<i64>,
}

/// The File Info of `bytes` (a PDF, or a PDF-compatible `.ai`), opened with `password` when it is
/// encrypted; `None` when it can't be opened.
pub fn file_info(bytes: &[u8], password: Option<&str>) -> Option<FileInfo> {
    Some(file_info_in(&crate::pages::open(bytes, password).ok()?))
}

/// [`file_info`] of a parsed PDF.
pub(crate) fn file_info_in(pdf: &Pdf) -> FileInfo {
    let mut f = packet(pdf).and_then(|p| xmp(&p)).unwrap_or_default();
    let info = pdf.metadata();
    let text = |b: &Option<Vec<u8>>| b.as_deref().map(info_string).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    f.author = f.author.or_else(|| text(&info.author));
    f.description = f.description.or_else(|| text(&info.subject));
    f.keywords = f.keywords.or_else(|| text(&info.keywords).map(|k| split(&k)).filter(|k| !k.is_empty()));
    f.created = f.created.or_else(|| info.creation_date.and_then(pdf_date));
    f.modified = f.modified.or_else(|| info.modification_date.and_then(pdf_date));
    f
}

/// An information dictionary string: UTF-16 or UTF-8 with its byte order mark, as [`text_string`]
/// reads it, else PDFDocEncoding (ISO 32000-1 Annex D, Table D.2).
fn info_string(b: &[u8]) -> String {
    if b.starts_with(&[0xFE, 0xFF]) || b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return text_string(b);
    }
    b.iter().map(|&c| pdf_doc_char(c)).collect()
}

/// Byte `c` of a PDFDocEncoding string. It matches Latin-1 except at 0x18 to 0x1F (accents) and
/// 0x80 to 0xA0 (punctuation, ligatures, letters and the euro sign); 0x9F has no character and
/// stays as it is.
fn pdf_doc_char(c: u8) -> char {
    const ACCENTS: [char; 8] = ['\u{2D8}', '\u{2C7}', '\u{2C6}', '\u{2D9}', '\u{2DD}', '\u{2DB}', '\u{2DA}', '\u{2DC}'];
    const HIGH: [char; 33] = [
        '\u{2022}', '\u{2020}', '\u{2021}', '\u{2026}', '\u{2014}', '\u{2013}', '\u{192}', '\u{2044}', '\u{2039}', '\u{203A}', '\u{2212}',
        '\u{2030}', '\u{201E}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{201A}', '\u{2122}', '\u{FB01}', '\u{FB02}', '\u{141}', '\u{152}',
        '\u{160}', '\u{178}', '\u{17D}', '\u{131}', '\u{142}', '\u{153}', '\u{161}', '\u{17E}', '\u{9F}', '\u{20AC}',
    ];
    let mapped = match c {
        0x18..=0x1F => ACCENTS.get(usize::from(c - 0x18)),
        0x80..=0xA0 => HIGH.get(usize::from(c - 0x80)),
        _ => None,
    };
    mapped.copied().unwrap_or(char::from(c))
}

/// The catalog's metadata stream, decoded; `None` past [`MAX_XMP`] bytes.
fn packet(pdf: &Pdf) -> Option<Vec<u8>> {
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id())?;
    let data = catalog.get::<Stream<'_>>(b"Metadata")?.decoded_within(MAX_XMP + 1).ok()?;
    (data.len() <= MAX_XMP).then(|| data.into_owned())
}

/// The File Info fields of an XMP packet; `None` when it isn't XML in UTF-8 or UTF-16.
fn xmp(packet: &[u8]) -> Option<FileInfo> {
    let text = xml_text(packet)?;
    let xml = roxmltree::Document::parse(&text).ok()?;
    // The resource's own descriptions: those right under `rdf:RDF`, not the ones inside a value.
    let descriptions: Vec<Node<'_, '_>> = xml
        .descendants()
        .filter(|n| n.has_tag_name((RDF, "RDF")))
        .flat_map(|rdf| rdf.children())
        .filter(|n| n.has_tag_name((RDF, "Description")))
        .collect();
    let get = |ns: &str, name: &str| property(&descriptions, ns, name);
    let first = |ns: &str, name: &str| get(ns, name).into_iter().next();
    let creators = get(DC, "creator");
    let subjects = get(DC, "subject");
    Some(FileInfo {
        author: (!creators.is_empty()).then(|| creators.join(", ")),
        description: first(DC, "description"),
        keywords: if subjects.is_empty() { first(PDF, "Keywords").map(|k| split(&k)).filter(|k| !k.is_empty()) } else { Some(subjects) },
        copyright_notice: first(DC, "rights"),
        copyright_url: first(XMP_RIGHTS, "WebStatement"),
        copyrighted: first(XMP_RIGHTS, "Marked").and_then(|m| match m.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }),
        created: first(XMP, "CreateDate").and_then(|d| xmp_date(&d)),
        modified: first(XMP, "ModifyDate").and_then(|d| xmp_date(&d)),
    })
}

/// A packet's text: UTF-8 (with or without a byte order mark), or UTF-16 told by its byte order
/// mark or by a zero byte beside the `<` it starts with.
fn xml_text(b: &[u8]) -> Option<String> {
    let utf16 = |be: bool| {
        let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if be { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) }).collect();
        String::from_utf16(&units).ok().map(|s| s.trim_start_matches('\u{FEFF}').to_string())
    };
    match b {
        [0xFE, 0xFF, ..] | [0, b'<', ..] => utf16(true),
        [0xFF, 0xFE, ..] | [b'<', 0, ..] => utf16(false),
        _ => std::str::from_utf8(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)).ok().map(str::to_string),
    }
}

/// The values of property `ns` `name` on the first of `descriptions` that gives it: an attribute
/// of the `rdf:Description`, or a property element's items ([`items`]), blank ones left out.
fn property(descriptions: &[Node<'_, '_>], ns: &str, name: &str) -> Vec<String> {
    for d in descriptions {
        if let Some(v) = d.attribute((ns, name)).map(str::trim).filter(|v| !v.is_empty()) {
            return vec![v.to_string()];
        }
        let found = d.children().filter(|c| c.has_tag_name((ns, name))).map(items).find(|v| !v.is_empty());
        if let Some(v) = found {
            return v;
        }
    }
    vec![]
}

/// The items of property element `p`: the `rdf:li` of its `rdf:Seq`, `rdf:Bag` or `rdf:Alt` (the
/// `x-default` language item first), else its own value ([`value_of`]); trimmed, blank ones left
/// out.
fn items(p: Node<'_, '_>) -> Vec<String> {
    let array = p.children().find(|c| ["Seq", "Bag", "Alt"].into_iter().any(|t| c.has_tag_name((RDF, t))));
    let mut values: Vec<(bool, String)> = match array {
        Some(a) => a
            .children()
            .filter(|c| c.has_tag_name((RDF, "li")))
            .map(|li| (li.attribute((XML, "lang")).is_some_and(|l| l.eq_ignore_ascii_case("x-default")), value_of(li)))
            .collect(),
        None => vec![(false, value_of(p))],
    };
    // A stable sort: the `x-default` item first, the others in their order.
    values.sort_by_key(|(default, _)| !default);
    values.into_iter().map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
}

/// The value of element `n`: the URI of its `rdf:resource` attribute (how RDF writes a URI value);
/// for a qualified value, its `rdf:value` (an attribute or an element, on `n` or on an
/// `rdf:Description` inside it), without the qualifiers; else the text inside it.
fn value_of(mut n: Node<'_, '_>) -> String {
    // Each step goes to an element inside `n`, so the loop ends.
    loop {
        if let Some(uri) = n.attribute((RDF, "resource")) {
            return uri.to_string();
        }
        let holder = n.children().find(|c| c.has_tag_name((RDF, "Description"))).unwrap_or(n);
        if let Some(v) = holder.attribute((RDF, "value")) {
            return v.to_string();
        }
        match holder.children().find(|c| c.has_tag_name((RDF, "value"))) {
            Some(v) => n = v,
            None => return n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect(),
        }
    }
}

/// Keywords written as one text (Info `Keywords`, XMP `pdf:Keywords`): split at commas and
/// semicolons, trimmed, blank ones left out.
fn split(s: &str) -> Vec<String> {
    s.split([',', ';']).map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).collect()
}

/// A PDF date (Info `CreationDate`, `ModDate`) as Unix seconds; `None` for a day its month doesn't
/// have.
fn pdf_date(d: DateTime) -> Option<i64> {
    let sign = if d.utc_offset_hour < 0 { -1 } else { 1 };
    let offset = i64::from(d.utc_offset_hour) * 60 + sign * i64::from(d.utc_offset_minute);
    let [year, month, day] = [d.year, d.month.into(), d.day.into()].map(i64::from);
    let time = [d.hour, d.minute, d.second].map(i64::from);
    in_month(year, month, day).then(|| unix([year, month, day, time[0], time[1], time[2]], offset))
}

/// Whether `day` is a day of `month` (1 to 12) in `year` (proleptic Gregorian).
fn in_month(year: i64, month: i64, day: i64) -> bool {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=last).contains(&day)
}

/// An XMP date (ISO 16684-1: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, then optionally `Thh:mm`, `:ss` with
/// a fraction, and a zone `Z` or `±hh:mm`) as Unix seconds; `None` when it isn't one.
fn xmp_date(s: &str) -> Option<i64> {
    let num = |p: Option<&str>, len: usize, max: i64| -> Option<i64> {
        let p = p?;
        let n: i64 = p.parse().ok().filter(|_| p.len() == len && p.bytes().all(|b| b.is_ascii_digit()))?;
        (n <= max).then_some(n)
    };
    let (date, time) = s.trim().split_once('T').map_or((s.trim(), None), |(d, t)| (d, Some(t)));
    let mut ymd = date.split('-');
    let year = num(ymd.next(), 4, 9999)?;
    let month = ymd.next().map_or(Some(1), |m| num(Some(m), 2, 12).filter(|m| *m >= 1))?;
    let day = ymd.next().map_or(Some(1), |d| num(Some(d), 2, 31).filter(|d| *d >= 1))?;
    if ymd.next().is_some() || !in_month(year, month, day) {
        return None;
    }
    let Some(time) = time else { return Some(unix([year, month, day, 0, 0, 0], 0)) };
    let (clock, zone) = time.find(['Z', '+', '-']).map_or((time, ""), |at| time.split_at(at));
    let mut hms = clock.split(':');
    let hour = num(hms.next(), 2, 23)?;
    let minute = num(hms.next(), 2, 59)?;
    // A leap second (60) counts as 59; the fraction is dropped.
    let second = hms.next().map_or(Some(0), |s| num(s.split('.').next(), 2, 60).map(|s| s.min(59)))?;
    if hms.next().is_some() {
        return None;
    }
    let offset = match zone {
        "" | "Z" => 0,
        _ => {
            let (sign, hm) = zone.split_at_checked(1)?;
            let (h, m) = hm.split_once(':')?;
            (if sign == "-" { -1 } else { 1 }) * (num(Some(h), 2, 23)? * 60 + num(Some(m), 2, 59)?)
        }
    };
    Some(unix([year, month, day, hour, minute, second], offset))
}

/// `[year, month, day, hour, minute, second]` at a UTC offset of `offset` minutes → Unix seconds
/// (days-from-civil, proleptic Gregorian: the inverse of [`vectorcraft_doc::metadata::civil`]).
fn unix([year, month, day, hour, minute, second]: [i64; 6], offset: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3600 + minute * 60 + second - offset * 60
}
