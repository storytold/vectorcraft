//! File Info read from a PDF: the information dictionary, the XMP packet, the packet's values
//! where both give a field, and the dictionary's alone where the packet can't be read.

use vectorcraft_doc::Document;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};

use crate::*;

/// A one-page PDF whose trailer names an information dictionary with the entries `info`, and whose
/// catalog has a metadata stream holding `xmp`.
fn pdf(info: Option<&str>, xmp: Option<&str>) -> Vec<u8> {
    let x = first_extra(1);
    let mut extra = vec![];
    let mut catalog = String::new();
    if let Some(p) = xmp {
        catalog = format!("/Metadata {x} 0 R ");
        extra.push(format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{p}\nendstream", p.len()));
    }
    let info_at = x + extra.len();
    if let Some(i) = info {
        extra.push(format!("<< {i} >>"));
    }
    let objects: Vec<&str> = extra.iter().map(String::as_str).collect();
    let bytes = pdf_with_catalog(&[PdfPage::new(100.0, 100.0, "0 0 1 rg 10 10 50 50 re f")], &objects, &catalog, None);
    if info.is_none() {
        return bytes;
    }
    // The trailer follows the cross-reference table, so a longer trailer moves no object.
    let text = String::from_utf8(bytes).unwrap();
    text.replacen("/Root 1 0 R", &format!("/Root 1 0 R /Info {info_at} 0 R"), 1).into_bytes()
}

/// An XMP packet whose `rdf:RDF` holds `body`.
fn packet(body: &str) -> String {
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">{body}</rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>"
    )
}

/// An `rdf:Description` declaring the namespaces File Info is read from, with `attrs` and `body`.
fn description(attrs: &str, body: &str) -> String {
    format!(
        "<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:xmpRights=\"http://ns.adobe.com/xap/1.0/rights/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" {attrs}>{body}</rdf:Description>"
    )
}

const INFO: &str = "/Author (Grace Hopper) /Subject <FEFF00430061007400610020006C006F0067> /Keywords (navy; compilers, , cobol) \
                    /CreationDate (D:20261005140300+02'00') /ModDate (D:20261006090000Z) /Title (Ignored title)";

/// Unix seconds of 2026-10-05 12:03:00 UTC (14:03 at +02:00).
const CREATED: i64 = 1_791_201_780;
/// Unix seconds of 2026-10-06 09:00:00 UTC.
const MODIFIED: i64 = 1_791_277_200;

#[test]
fn the_information_dictionary_gives_file_info() {
    let f = file_info(&pdf(Some(INFO), None), None).unwrap();
    assert_eq!(
        f,
        FileInfo {
            author: Some("Grace Hopper".into()),
            description: Some("Cata log".into()),
            keywords: Some(vec!["navy".into(), "compilers".into(), "cobol".into()]),
            created: Some(CREATED),
            modified: Some(MODIFIED),
            ..Default::default()
        }
    );
    assert_eq!(file_info(&pdf(None, None), None), Some(FileInfo::default()));
    // Strings without a byte order mark are PDFDocEncoding, which differs from Latin-1 in places.
    let info = r"/Author (Miles O\220Brien) /Subject (Caf\351 \204 \215Menu\216\222 \030 \240) /Keywords (\223le)";
    let f = file_info(&pdf(Some(info), None), None).unwrap();
    assert_eq!(f.author.as_deref(), Some("Miles O\u{2019}Brien"));
    assert_eq!(f.description.as_deref(), Some("Caf\u{E9} \u{2014} \u{201C}Menu\u{201D}\u{2122} \u{2D8} \u{20AC}"));
    assert_eq!(f.keywords, Some(vec!["\u{FB01}le".to_string()]));
}

#[test]
fn the_xmp_packet_gives_file_info() {
    let body = description(
        "xmpRights:Marked=\"False\" xmp:ModifyDate=\"2026-10-06T09:00:00.25Z\"",
        "<dc:creator><rdf:Seq><rdf:li>Ada Lovelace</rdf:li><rdf:li>Charles Babbage</rdf:li></rdf:Seq></dc:creator>\
         <dc:description><rdf:Alt><rdf:li xml:lang=\"de\">Ein Plakat</rdf:li><rdf:li xml:lang=\"x-default\">A poster\nfor the fair</rdf:li></rdf:Alt></dc:description>\
         <dc:subject><rdf:Bag><rdf:li>fair</rdf:li><rdf:li> print </rdf:li><rdf:li></rdf:li></rdf:Bag></dc:subject>\
         <dc:rights><rdf:Alt><rdf:li xml:lang=\"en\">(c) 2026 Ada</rdf:li></rdf:Alt></dc:rights>\
         <xmpRights:WebStatement>https://example.com/rights</xmpRights:WebStatement>\
         <xmp:CreateDate>2026-10-05T14:03+02:00</xmp:CreateDate>",
    );
    let f = file_info(&pdf(None, Some(&packet(&body))), None).unwrap();
    assert_eq!(
        f,
        FileInfo {
            author: Some("Ada Lovelace, Charles Babbage".into()),
            description: Some("A poster\nfor the fair".into()),
            keywords: Some(vec!["fair".into(), "print".into()]),
            copyright_notice: Some("(c) 2026 Ada".into()),
            copyright_url: Some("https://example.com/rights".into()),
            copyrighted: Some(false),
            created: Some(CREATED),
            modified: Some(MODIFIED),
        }
    );
    // Marked True is a copyrighted work; a URL can be written as an RDF resource; a date alone is
    // midnight UTC.
    let body = description(
        "xmpRights:Marked=\"True\"",
        "<xmpRights:WebStatement rdf:resource=\"https://example.com/terms\"/><xmp:CreateDate>2026-10-05</xmp:CreateDate>",
    );
    let f = file_info(&pdf(None, Some(&packet(&body))), None).unwrap();
    assert_eq!((f.copyrighted, f.created), (Some(true), Some(1_791_158_400)));
    assert_eq!(f.copyright_url.as_deref(), Some("https://example.com/terms"));
    // Qualified values give their `rdf:value` alone, without the qualifiers' text.
    let body = description(
        "xmlns:q=\"urn:example:qualifiers\"",
        "<dc:creator><rdf:Seq><rdf:li rdf:parseType=\"Resource\"><rdf:value>Ada Lovelace</rdf:value><q:role>Author</q:role></rdf:li>\
         <rdf:li><rdf:Description rdf:value=\"Grace Hopper\" q:role=\"Editor\"/></rdf:li></rdf:Seq></dc:creator>\
         <xmp:CreateDate><rdf:Description><rdf:value>2026-10-05</rdf:value><q:source>scan</q:source></rdf:Description></xmp:CreateDate>",
    );
    let f = file_info(&pdf(None, Some(&packet(&body))), None).unwrap();
    assert_eq!((f.author.as_deref(), f.created), (Some("Ada Lovelace, Grace Hopper"), Some(1_791_158_400)));
}

#[test]
fn the_xmp_packet_wins_where_both_give_a_field() {
    // The packet gives the author, keywords (as `pdf:Keywords`) and the created date; the
    // dictionary still gives the description (the packet's is blank, and a description inside a
    // value is another resource's) and the modified date.
    let body = description(
        "",
        "<dc:creator><rdf:Seq><rdf:li>Ada Lovelace</rdf:li></rdf:Seq></dc:creator><pdf:Keywords>engines, notes</pdf:Keywords>\
         <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\"> </rdf:li></rdf:Alt></dc:description>\
         <h:History xmlns:h=\"urn:example:history\"><rdf:Seq><rdf:li><rdf:Description><dc:description>Nested</dc:description></rdf:Description></rdf:li></rdf:Seq></h:History>\
         <xmp:CreateDate>2026-10-07T00:00:00-05:30</xmp:CreateDate>",
    );
    let f = file_info(&pdf(Some(INFO), Some(&packet(&body))), None).unwrap();
    assert_eq!(f.author.as_deref(), Some("Ada Lovelace"));
    assert_eq!(f.keywords, Some(vec!["engines".to_string(), "notes".into()]));
    assert_eq!(f.created, Some(1_791_331_200 + 5 * 3600 + 30 * 60));
    assert_eq!(f.description.as_deref(), Some("Cata log"));
    assert_eq!(f.modified, Some(MODIFIED));
}

#[test]
fn an_unreadable_xmp_packet_leaves_the_information_dictionary() {
    let creator = "<dc:creator><rdf:Seq><rdf:li>Ada Lovelace</rdf:li></rdf:Seq></dc:creator>";
    let bad = [
        // Not well-formed.
        packet(&description("", creator)).replace("</rdf:RDF>", ""),
        // A document type declaration (roxmltree refuses a DTD by default).
        format!("<!DOCTYPE x [<!ENTITY a \"Ada\">]>{}", packet(&description("", creator))),
        // Not XML.
        "Ada Lovelace".to_string(),
        // Dates that aren't dates.
        packet(&description("xmp:CreateDate=\"2026-13-01\" xmp:ModifyDate=\"2026-10-05T24:00Z\"", "")),
        // Days their month doesn't have.
        packet(&description("xmp:CreateDate=\"2026-02-29\" xmp:ModifyDate=\"2026-04-31T10:00Z\"", "")),
    ];
    for xmp in bad {
        let f = file_info(&pdf(Some(INFO), Some(&xmp)), None).unwrap();
        assert_eq!(f.author.as_deref(), Some("Grace Hopper"), "{xmp}");
        assert_eq!((f.created, f.modified), (Some(CREATED), Some(MODIFIED)), "{xmp}");
    }
    // The dictionary's dates are checked the same way: 2026 has no February 29, 2024 has one.
    let f = file_info(&pdf(Some("/CreationDate (D:20260229) /ModDate (D:20240229)"), None), None).unwrap();
    assert_eq!((f.created, f.modified), (None, Some(1_709_164_800)));
    // A packet in UTF-16 reads.
    let utf16: Vec<u8> = packet(&description("", creator)).encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut bytes = pdf(None, Some(&"#".repeat(utf16.len())));
    let at = bytes.windows(utf16.len()).position(|w| w.iter().all(|b| *b == b'#')).unwrap();
    bytes.splice(at..at + utf16.len(), utf16);
    assert_eq!(file_info(&bytes, None).unwrap().author.as_deref(), Some("Ada Lovelace"));
    // A file that can't be read gives none.
    assert_eq!(file_info(b"%PDF-1.7\nnot a PDF", None), None);
}

#[test]
fn a_written_pdf_gives_its_file_info_back() {
    let mut d = Document::new(100.0, 100.0);
    d.metadata.author = "Ada Lovelace".into();
    d.metadata.description = "A poster\nfor the fair".into();
    d.metadata.keywords = vec!["fair".into(), "print".into()];
    let bytes = export(&d, &PdfOptions { created: Some(CREATED), ..PdfOptions::uncompressed() }).unwrap();
    let f = file_info(&bytes, None).unwrap();
    assert_eq!(f.author.as_deref(), Some("Ada Lovelace"));
    assert_eq!(f.description.as_deref(), Some("A poster\nfor the fair"));
    assert_eq!(f.keywords, Some(vec!["fair".to_string(), "print".into()]));
    assert_eq!((f.created, f.modified), (Some(CREATED), Some(CREATED)));
}
