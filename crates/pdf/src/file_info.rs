//! File › File Info read from a PDF (and a PDF-compatible `.ai`): the document information
//! dictionary (ISO 32000-1 §14.3.3) and the XMP packet of the catalog's metadata stream (§14.3.2,
//! ISO 16684-1). Where both give a field, the XMP packet's value is taken; a packet that can't be
//! read leaves the information dictionary's values.

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{DateTime, Dict, Stream};
use craft_xml::{Canonical, Doctype, El, Limits, Names, Options};

use crate::import_scan::text_string;

/// The longest XMP packet read, in bytes once decoded; a longer one is left out.
const MAX_XMP: usize = 8 << 20;
/// The deepest the elements of an XMP packet may nest; a packet nested deeper is left out.
const MAX_NESTING: usize = 256;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub(crate) const DC: &str = "http://purl.org/dc/elements/1.1/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/"; // brand-ok: the XMP namespace the files use
pub(crate) const XMP_RIGHTS: &str = "http://ns.adobe.com/xap/1.0/rights/"; // brand-ok: the XMP namespace the files use
const PDF: &str = "http://ns.adobe.com/pdf/1.3/"; // brand-ok: the XMP namespace the files use

const XMP_NAMESPACES: Canonical<'static> =
    Canonical::new(&[("rdf", RDF), ("dc", DC), ("xmp", XMP), ("xmpRights", XMP_RIGHTS), ("pdf", PDF)]);

/// The File Info fields a PDF gives, each `None` when the file gives none (or only blank text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
    /// XMP `dc:title` (its `x-default` item, else its first), else Info `Title`.
    pub title: Option<String>,
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
    let text = |b: &Option<Vec<u8>>| b.as_deref().map(text_string).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    f.title = f.title.or_else(|| text(&info.title));
    f.author = f.author.or_else(|| text(&info.author));
    f.description = f.description.or_else(|| text(&info.subject));
    f.keywords = f.keywords.or_else(|| text(&info.keywords).map(|k| split(&k)).filter(|k| !k.is_empty()));
    f.created = f.created.or_else(|| info.creation_date.and_then(pdf_date));
    f.modified = f.modified.or_else(|| info.modification_date.and_then(pdf_date));
    f
}

/// The catalog's metadata stream, decoded; `None` past [`MAX_XMP`] bytes.
fn packet(pdf: &Pdf) -> Option<Vec<u8>> {
    let xref = pdf.xref();
    let catalog = xref.get::<Dict<'_>>(xref.root_id())?;
    let data = catalog.get::<Stream<'_>>(b"Metadata")?.decoded_within(MAX_XMP + 1).ok()?;
    (data.len() <= MAX_XMP).then(|| data.into_owned())
}

/// The File Info fields of an XMP packet; `None` when it isn't XML in UTF-8 or UTF-16, or when its
/// [`xmp`] of `packet`; `None` when it isn't XML in UTF-8 or UTF-16, or when its elements nest
/// more than [`MAX_NESTING`] deep.
fn xmp(packet: &[u8]) -> Option<FileInfo> {
    let text = xml_text(packet)?;
    let opts = Options {
        names: Names::Canonical(&XMP_NAMESPACES),
        doctype: Doctype::Reject,
        limits: Limits { max_depth: MAX_NESTING, ..Limits::DEFAULT },
        ..Options::default()
    }
    .strict();
    let root = opts.parse(text.as_bytes()).ok()?;
    // The resource's own descriptions: those right under `rdf:RDF`, not the ones inside a value.
    let descriptions: Vec<&El> =
        std::iter::once(&root).chain(root.descendants()).filter(|n| n.is("rdf:RDF")).flat_map(|rdf| rdf.children("rdf:Description")).collect();
    let get = |name: &str| property(&descriptions, name);
    let first = |name: &str| get(name).into_iter().next();
    let creators = get("dc:creator");
    let subjects = get("dc:subject");
    Some(FileInfo {
        title: first("dc:title"),
        author: (!creators.is_empty()).then(|| creators.join(", ")),
        description: first("dc:description"),
        keywords: if subjects.is_empty() { first("pdf:Keywords").map(|k| split(&k)).filter(|k| !k.is_empty()) } else { Some(subjects) },
        copyright_notice: first("dc:rights"),
        copyright_url: first("xmpRights:WebStatement"),
        copyrighted: first("xmpRights:Marked").and_then(|m| match m.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }),
        created: first("xmp:CreateDate").and_then(|d| xmp_date(&d)),
        modified: first("xmp:ModifyDate").and_then(|d| xmp_date(&d)),
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

/// The values of property `name` on the first of `descriptions` that gives it: an attribute
/// of the `rdf:Description`, or a property element's items ([`items`]), blank ones left out.
fn property(descriptions: &[&El], name: &str) -> Vec<String> {
    for d in descriptions {
        if let Some(v) = d.attr(name).map(str::trim).filter(|v| !v.is_empty()) {
            return vec![v.to_string()];
        }
        let found = d.children(name).map(items).find(|v| !v.is_empty());
        if let Some(v) = found {
            return v;
        }
    }
    vec![]
}

/// The items of property element `p`: the `rdf:li` of its `rdf:Seq`, `rdf:Bag` or `rdf:Alt` (the
/// `x-default` language item first), else its own value ([`value_of`]); trimmed, blank ones left
/// out.
fn items(p: &El) -> Vec<String> {
    let array = p.elements().find(|c| matches!(c.name.as_str(), "rdf:Seq" | "rdf:Bag" | "rdf:Alt"));
    let mut values: Vec<(bool, String)> = match array {
        Some(a) => a
            .children("rdf:li")
            .map(|li| (li.attr("xml:lang").is_some_and(|l| l.eq_ignore_ascii_case("x-default")), value_of(li)))
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
fn value_of(mut n: &El) -> String {
    // Each step goes to an element inside `n`, so the loop ends.
    loop {
        if let Some(uri) = n.attr("rdf:resource") {
            return uri.to_string();
        }
        let holder = n.child("rdf:Description").unwrap_or(n);
        if let Some(v) = holder.attr("rdf:value") {
            return v.to_string();
        }
        match holder.child("rdf:value") {
            Some(v) => n = v,
            None => return n.deep_text(),
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
    let ymd = i32::try_from(year).ok().zip(u32::try_from(month).ok()).zip(u32::try_from(day).ok());
    ymd.is_some_and(|((y, m), d)| chrono::NaiveDate::from_ymd_opt(y, m, d).is_some())
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
/// (the inverse of [`vectorcraft_doc::metadata::civil`]). The date has already passed [`in_month`],
/// so it is a real day; an impossible one reads as the epoch rather than panicking.
fn unix([year, month, day, hour, minute, second]: [i64; 6], offset: i64) -> i64 {
    let days = i32::try_from(year)
        .ok()
        .zip(u32::try_from(month).ok())
        .zip(u32::try_from(day).ok())
        .and_then(|((y, m), d)| chrono::NaiveDate::from_ymd_opt(y, m, d))
        .zip(chrono::NaiveDate::from_ymd_opt(1970, 1, 1))
        .map(|(date, epoch)| (date - epoch).num_days())
        .unwrap_or(0);
    days * 86_400 + hour * 3600 + minute * 60 + second - offset * 60
}
