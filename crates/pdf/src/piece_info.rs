//! An editor's private data on every page: a page-piece dictionary (ISO 32000-1 §14.5,
//! `PieceInfo`) holding the editing copy of the art that `.ai` files carry, laid out as in the
//! `.ai` files users sent, the form [`crate::illustrator_data`] reads.
//!
//! - Every page dictionary gets `/PieceInfo << /Illustrator n 0 R >>`, the same object `n` on
//!   each page, and its own `/LastModified` (which §14.5 requires with `PieceInfo`). A file with
//!   several pages whose pages do not all refer to the one data dictionary opens in other apps as
//!   a PDF, without the editing data.
//! - Object `n` is the data dictionary: `<< /LastModified (D:…) /Private m 0 R >>`.
//! - Object `m` holds `AIMetaData` (a stream of the data's header comments), `AIPrivateData1` …
//!   `AIPrivateDataN` (the data in streams of [`PRIVATE_CHUNK`] bytes, the last one shorter),
//!   `ContainerVersion`, `CreatorVersion`, `NumBlock` (N), `RoundtripStreamType` and
//!   `RoundtripVersion`. Every stream has a `/Length` and no filter: the data comes compressed.

use std::fmt::Write as _;

use crate::PdfError;
use crate::linearize::{self, Parsed};
use crate::patch::Patch;

/// Bytes in each private data stream but the last.
pub const PRIVATE_CHUNK: usize = 1 << 16;

/// The `PieceInfo` key the data dictionary is under.
const OWNER: &str = "Illustrator"; // brand-ok: the key the files use

/// The private data a page-piece dictionary carries (see the module documentation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivateData {
    /// The data, as its streams hold it joined.
    pub data: Vec<u8>,
    /// What the `AIMetaData` stream holds.
    pub metadata: Vec<u8>,
    pub container_version: u32,
    pub creator_version: u32,
    pub roundtrip_version: u32,
    pub roundtrip_stream_type: u32,
}

/// `pdf` with `p` in every page's `PieceInfo`, dated `modified` (Unix seconds, UTC).
pub(crate) fn add(pdf: Vec<u8>, p: &PrivateData, modified: i64) -> Result<Vec<u8>, PdfError> {
    let failed = |why: &str| PdfError::Write(format!("can't add the editing data for other apps: {why}"));
    let Parsed { xref, pages, .. } = linearize::parse(&pdf).map_err(failed)?;
    if pages.is_empty() {
        return Err(failed("a page can't be read"));
    }
    let dicts = pages.iter().map(|&n| xref.dict(&pdf, n).map(|(at, _)| at)).collect::<Option<Vec<usize>>>();
    let dicts = dicts.ok_or_else(|| failed("a page can't be read"))?;
    let mut patch = Patch::new(&xref);
    let metadata = patch.add_object(stream(&p.metadata));
    let blocks: Vec<u32> = p.data.chunks(PRIVATE_CHUNK).map(|c| patch.add_object(stream(c))).collect();
    let mut private = format!("<< /AIMetaData {metadata} 0 R");
    for (i, n) in blocks.iter().enumerate() {
        let _ = write!(private, " /AIPrivateData{} {n} 0 R", i + 1);
    }
    let _ = write!(
        private,
        " /ContainerVersion {} /CreatorVersion {} /NumBlock {} /RoundtripStreamType {} /RoundtripVersion {} >>",
        p.container_version,
        p.creator_version,
        blocks.len(),
        p.roundtrip_stream_type,
        p.roundtrip_version
    );
    let private = patch.add_object(private.into_bytes());
    let date = pdf_date(modified);
    let owner = patch.add_object(format!("<< /LastModified {date} /Private {private} 0 R >>").into_bytes());
    for dict in dicts {
        patch.replace(dict, dict, format!("/PieceInfo << /{OWNER} {owner} 0 R >> /LastModified {date} ").into_bytes());
    }
    patch.apply(&pdf, &xref)
}

/// A stream object of `bytes`, not filtered.
fn stream(bytes: &[u8]) -> Vec<u8> {
    let mut s = format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes();
    s.extend_from_slice(bytes);
    s.extend_from_slice(b"\nendstream");
    s
}

/// `at` (Unix seconds) as a PDF date string in UTC (ISO 32000-1 §7.9.4), with the offset written
/// as hours and minutes, as in the files users sent.
fn pdf_date(at: i64) -> String {
    let [y, mo, d, h, mi, s] = vectorcraft_doc::metadata::civil(at);
    format!("(D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}+00'00')")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PdfOptions, export};
    use vectorcraft_doc::Document;

    fn data(len: usize) -> PrivateData {
        PrivateData {
            data: (0..len).map(|i| (i % 251) as u8).collect(),
            metadata: b"%!PS-Adobe-3.0\n%%EndComments\n".to_vec(),
            container_version: 12,
            creator_version: 30,
            roundtrip_version: 24,
            roundtrip_stream_type: 2,
        }
    }

    #[test]
    fn the_first_page_carries_the_data_in_streams_of_64_kib() {
        let doc = Document::new(200.0, 100.0);
        let p = data(PRIVATE_CHUNK * 2 + 10);
        let opts = PdfOptions { private_data: Some(p.clone()), ..PdfOptions::uncompressed() };
        let pdf = export(&doc, &opts).unwrap();
        assert_eq!(crate::illustrator_data(&pdf, None).as_deref(), Some(&p.data[..]));
        let text = String::from_utf8_lossy(&pdf);
        for key in ["/AIPrivateData1 ", "/AIPrivateData2 ", "/AIPrivateData3 ", "/NumBlock 3 ", "/ContainerVersion 12 ", "/CreatorVersion 30 "] {
            assert!(text.contains(key), "{key}");
        }
        assert!(!text.contains("/AIPrivateData4"));
        assert_eq!(text.matches("/LastModified (D:").count(), 2, "the page's and the data dictionary's");
        assert!(text.contains(&format!("<< /Length {PRIVATE_CHUNK} >>")) && text.contains("<< /Length 10 >>"));
    }

    #[test]
    fn every_page_refers_to_the_one_data_dictionary() {
        let mut doc = Document::new(200.0, 100.0);
        let mut second = doc.artboards[0].clone();
        (second.id, second.name, second.rect) = (2, "Back".into(), vectorcraft_geom::Rect::new(240.0, 0.0, 440.0, 100.0));
        doc.artboards.push(second);
        let opts = PdfOptions { private_data: Some(data(10)), ..PdfOptions::uncompressed() };
        let pdf = export(&doc, &opts).unwrap();
        let Parsed { xref, pages, .. } = linearize::parse(&pdf).unwrap();
        assert_eq!(pages.len(), 2);
        let owners: Vec<String> = pages
            .iter()
            .map(|&n| {
                let (at, end) = xref.dict(&pdf, n).unwrap();
                let page = String::from_utf8_lossy(&pdf[at..end]).into_owned();
                assert!(page.contains("/LastModified (D:"), "{page}");
                let i = page.find("/PieceInfo << /Illustrator ").expect("every page carries the data");
                page[i..].split(">>").next().unwrap().to_string()
            })
            .collect();
        assert_eq!(owners[0], owners[1]);
        assert_eq!(String::from_utf8_lossy(&pdf).matches("/AIPrivateData1 ").count(), 1, "the data is written once");
    }

    #[test]
    fn without_the_data_the_page_has_no_piece_info() {
        let pdf = export(&Document::new(200.0, 100.0), &PdfOptions::uncompressed()).unwrap();
        assert!(!String::from_utf8_lossy(&pdf).contains("/PieceInfo"));
        assert_eq!(crate::illustrator_data(&pdf, None), None);
    }

    #[test]
    fn dates_are_in_utc() {
        assert_eq!(pdf_date(0), "(D:19700101000000+00'00')");
        assert_eq!(pdf_date(1_793_000_000), "(D:20261026073320+00'00')");
    }
}
