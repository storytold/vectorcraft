//! Marks and Bleeds: the boxes of each exported page and its printer's marks. The marks' geometry
//! is the shared [`PrinterMarks`] (print draws the same marks); here it gets the page's bleed,
//! spot inks and page information.

use vectorcraft_color::Paint;
use vectorcraft_doc::marks::{MarkStyle, PrinterMarks, outset};
use vectorcraft_doc::setup::{MAX_BLEED, bleed_in_range};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_geom::Rect;

use crate::{BleedSettings, MarkKind, MarkSettings, PdfSettings};

impl MarkSettings {
    /// These settings as the shared mark geometry.
    pub fn printer_marks(&self) -> PrinterMarks {
        PrinterMarks {
            trim: self.trim,
            registration: self.registration,
            color_bars: self.color_bars,
            page_info: self.page_info,
            style: match self.kind {
                MarkKind::Roman => MarkStyle::Roman,
                MarkKind::Japanese => MarkStyle::Japanese,
            },
            weight: self.weight,
            offset: self.offset,
        }
    }
}

impl PdfSettings {
    /// The bleed of `doc`'s pages, `[top, bottom, left, right]` in points: the document's (Document
    /// Setup; one read from a file is kept to 0–[`MAX_BLEED`]) with Use Document Bleed, else the
    /// settings' own.
    pub fn bleed_of(&self, doc: &Document) -> [f64; 4] {
        self.bleed.of(doc)
    }

    /// A PDF/X file of `doc` has no bleed on a side where the document's bleed (Document Setup) has
    /// one.
    pub fn leaves_out_bleed(&self, doc: &Document) -> bool {
        self.standard.is_pdfx()
            && self.bleed_of(doc).into_iter().zip(bleed_in_range(doc.setup.bleed)).any(|(page, document)| page <= 0.0 && document > 0.0)
    }
}

impl BleedSettings {
    /// The bleed of `doc`'s pages, `[top, bottom, left, right]` in points: the document's with
    /// [`Self::use_document`] (kept to 0–[`MAX_BLEED`]), else these values.
    pub fn of(&self, doc: &Document) -> [f64; 4] {
        if self.use_document { doc.setup.bleed.map(|b| if b.is_finite() { b.clamp(0.0, MAX_BLEED) } else { 0.0 }) } else { self.values() }
    }
}

/// The boxes of one page, in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageBoxes {
    /// The artboard: the finished page.
    pub trim: Rect,
    /// The trim box grown by the bleed: the art it holds is kept.
    pub bleed: Rect,
    /// The sheet: the bleed box, grown to hold the printer's marks.
    pub media: Rect,
}

impl PageBoxes {
    pub fn new(trim: Rect, bleed: [f64; 4], marks: &PrinterMarks) -> Self {
        let reach = marks.reach(bleed);
        let most: [f64; 4] = std::array::from_fn(|i| bleed[i].max(reach[i]));
        Self { trim, bleed: outset(trim, bleed), media: outset(trim, most) }
    }
}

/// The page information printed under the marks: the file's title, the artboard (its name and
/// number) and the date and time of the export (UTC).
pub(crate) fn page_info(doc: &Document, title: &str, artboard: usize, created: Option<i64>) -> String {
    page_info_of(doc, title, Some(artboard), created)
}

/// [`page_info`] of a page that may show no artboard (print with artboards ignored).
pub(crate) fn page_info_of(doc: &Document, title: &str, artboard: Option<usize>, created: Option<i64>) -> String {
    let title = if title.trim().is_empty() { "Untitled" } else { title.trim() };
    let mut info = title.to_string();
    if let Some(i) = artboard {
        let name = doc.artboards.get(i).map_or("", |a| a.name.as_str());
        info.push_str(&format!("  ·  {name} ({} of {})", i + 1, doc.artboards.len()));
    }
    if let Some(t) = created {
        let [y, mo, d, h, mi, _] = vectorcraft_doc::metadata::civil(t);
        info.push_str(&format!("  ·  {y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC"));
    }
    info
}

/// The marks of a page as art (`None` without marks): its spot inks get colour bar patches.
pub(crate) fn art(doc: &Document, marks: &PrinterMarks, boxes: &PageBoxes, bleed: [f64; 4], info: &str) -> Option<Node> {
    if !marks.any() {
        return None;
    }
    let spots: Vec<Paint> = doc
        .spot_names()
        .into_iter()
        .filter_map(|name| doc.global_color(&name).map(|color| Paint::Solid { color, swatch: Some(name), tint: 1.0 }))
        .collect();
    // The marks are drawn straight from this art, so their ids don't matter.
    let mut next = 0;
    Some(marks.art(boxes.trim, bleed, &spots, info, &mut || {
        next += 1;
        NodeId(next)
    }))
}
