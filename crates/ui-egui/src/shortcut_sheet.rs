//! Keyboard Shortcuts › Export Text…'s printable shortcut sheet and CSV file.
//!
//! The rows come from [`crate::shortcut_editor::export_sections`]: a Tools section and one section
//! per top-level menu. The sheet lists the rows that have a shortcut, each section under a colored
//! band with the shortcut on the left and the name on the right, flowed down columns of landscape
//! pages, and is written as a PDF by laying it out as a document. The CSV file lists every row
//! ([RFC 4180](https://www.rfc-editor.org/rfc/rfc4180.txt) quoting, CRLF line ends).

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};

/// One section of the list: Tools or a top-level menu, with its rows in the dialog's order.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

/// A row: the command's name within its section and its shortcuts as the dialog shows them (empty
/// when it has none).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub shortcut: String,
}

/// The sheet's paper, used in landscape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    Letter,
    A4,
}

impl PageSize {
    pub const ALL: [Self; 2] = [Self::Letter, Self::A4];

    /// The `pageSize` param's value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Letter => "letter",
            Self::A4 => "a4",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    /// Width and height of a landscape page in points.
    pub fn landscape(self) -> (f64, f64) {
        match self {
            Self::Letter => (792.0, 612.0),
            Self::A4 => (841.89, 595.28),
        }
    }
}

/// Whether a spreadsheet would read `s` as a formula: it starts with `=`, `+`, `-` or `@` and has
/// more after it, or starts with a tab or a carriage return.
fn reads_as_formula(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some('=' | '+' | '-' | '@') => chars.next().is_some(),
        Some('\t' | '\r') => true,
        _ => false,
    }
}

/// A CSV field: a field a spreadsheet would read as a formula starts with an apostrophe, and a
/// field with a comma, a double quote or a line break is quoted, with each double quote doubled.
fn csv_field(s: &str) -> String {
    let s = if reads_as_formula(s) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// The CSV file: a byte order mark, so spreadsheets read it as UTF-8, a header row of `headers`,
/// then one `section,name,shortcut` record per row, each ending in CRLF.
pub fn csv(sections: &[Section], headers: [&str; 3]) -> String {
    let mut out = String::from("\u{feff}");
    let mut record = |fields: [&str; 3]| {
        out += &fields.map(csv_field).join(",");
        out += "\r\n";
    };
    record(headers);
    for s in sections {
        for r in &s.rows {
            record([&s.title, &r.name, &r.shortcut]);
        }
    }
    out
}

/// Section band colors, in turn.
const BANDS: [(u8, u8, u8); 9] = [
    (0x2f, 0x6d, 0xb5),
    (0xc0, 0x39, 0x2b),
    (0x27, 0x86, 0x5f),
    (0x8e, 0x44, 0xad),
    (0xd3, 0x54, 0x00),
    (0x16, 0x87, 0x9a),
    (0x9a, 0x7d, 0x0a),
    (0x5d, 0x6d, 0x7e),
    (0xc2, 0x18, 0x5b),
];

const MARGIN: f64 = 30.0;
const TITLE_SIZE: f64 = 15.0;
/// From the top margin to the top of the first column.
const TITLE_SPACE: f64 = 28.0;
const COLUMNS: usize = 4;
const GUTTER: f64 = 12.0;
const BAND_HEIGHT: f64 = 13.0;
const BAND_SIZE: f64 = 8.0;
const ROW_HEIGHT: f64 = 9.5;
const ROW_SIZE: f64 = 7.0;
/// Space above a section that does not start a column.
const SECTION_SPACE: f64 = 6.0;
/// The least and the most of a column's width the shortcuts take.
const SHORTCUT_SHARE: f64 = 0.3;
const MAX_SHORTCUT_SHARE: f64 = 0.5;
/// Space between pages in the document.
const PAGE_GAP: f64 = 36.0;
const FONT: &str = "Source Sans 3";

/// The sheet's text style.
fn style(size: f64, bold: bool, fill: Color) -> CharStyle {
    CharStyle {
        font_family: FONT.into(),
        font_style: if bold { "Bold" } else { "Regular" }.into(),
        size,
        fill: Paint::solid(fill),
        ..CharStyle::default()
    }
}

/// The width `text` takes in `style`.
fn width(text: &str, style: &CharStyle) -> f64 {
    let t = TextObject::point(Point::ZERO, text, style.clone());
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds.width()
}

/// The most characters of a name or title the sheet measures; no line on a page holds more.
const MAX_FIT_CHARS: usize = 400;

/// `text`, shortened with an ellipsis to fit in `room`: the longest start of it that fits with the
/// ellipsis, found with a binary search, so a long imported name takes a few layouts.
fn fitted(text: &str, style: &CharStyle, room: f64) -> String {
    let chars: Vec<char> = text.chars().take(MAX_FIT_CHARS + 1).collect();
    if chars.len() <= MAX_FIT_CHARS && width(text, style) <= room {
        return text.to_string();
    }
    let cut = |n: usize| chars.iter().take(n).collect::<String>().trim_end().to_string() + "…";
    let (mut lo, mut hi) = (0, chars.len().min(MAX_FIT_CHARS));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(&cut(mid), style) <= room { lo = mid } else { hi = mid - 1 }
    }
    if lo == 0 && width("…", style) > room { String::new() } else { cut(lo) }
}

/// Builds the sheet's document: one artboard per page.
struct Sheet {
    doc: Document,
    page: (f64, f64),
    /// Pages so far.
    pages: usize,
    /// The column being filled, 0-based, and the top of its free space.
    column: usize,
    y: f64,
    /// The shortcut column's width; the names start there.
    split: f64,
}

impl Sheet {
    fn origin(&self) -> f64 {
        (self.pages - 1) as f64 * (self.page.0 + PAGE_GAP)
    }

    fn column_width(&self) -> f64 {
        (self.page.0 - 2.0 * MARGIN - GUTTER * (COLUMNS - 1) as f64) / COLUMNS as f64
    }

    fn column_left(&self) -> f64 {
        self.origin() + MARGIN + self.column as f64 * (self.column_width() + GUTTER)
    }

    fn top(&self) -> f64 {
        MARGIN + TITLE_SPACE
    }

    fn bottom(&self) -> f64 {
        self.page.1 - MARGIN
    }

    fn add(&mut self, mut n: Node) {
        n.id = self.doc.alloc_id();
        let Some(layer) = self.doc.default_layer() else { return };
        let len = self.doc.children(Some(layer)).map_or(0, |c| c.len());
        // Insertion into the document's own layer only fails for a node id already used, and
        // every node gets a new id above.
        let _ = self.doc.insert(Some(layer), len, n);
    }

    fn rect(&mut self, r: Rect, fill: Color) {
        self.add(Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::None, 0.0)));
    }

    fn text(&mut self, at: Point, text: &str, style: CharStyle) {
        if !text.is_empty() {
            self.add(Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(at, text, style)))));
        }
    }

    /// Starts a page with `title` at its top.
    fn new_page(&mut self, title: &str) {
        self.pages += 1;
        let rect = Rect::new(self.origin(), 0.0, self.origin() + self.page.0, self.page.1);
        if self.pages == 1 {
            if let Some(a) = self.doc.artboards.first_mut() {
                a.rect = rect;
            }
        } else if let Some(mut a) = self.doc.artboards.first().cloned() {
            a.id = self.pages as u32;
            a.name = format!("Artboard {}", self.pages);
            a.rect = rect;
            self.doc.artboards.push(a);
        }
        let style = style(TITLE_SIZE, true, Color::BLACK);
        let room = self.page.0 - 2.0 * MARGIN;
        let shown = fitted(title, &style, room);
        self.text(Point::new(self.origin() + MARGIN, MARGIN + TITLE_SIZE), &shown, style);
        self.column = 0;
        self.y = self.top();
    }

    /// Room for `height` more in the column, else the next column or page.
    fn make_room(&mut self, height: f64, title: &str) -> bool {
        if self.y + height <= self.bottom() {
            return false;
        }
        if self.column + 1 < COLUMNS {
            self.column += 1;
            self.y = self.top();
        } else {
            self.new_page(title);
        }
        true
    }

    fn band(&mut self, title: &str, color: Color) {
        let (x, w) = (self.column_left(), self.column_width());
        self.rect(Rect::new(x, self.y, x + w, self.y + BAND_HEIGHT), color);
        let style = style(BAND_SIZE, true, Color::WHITE);
        let shown = fitted(title, &style, w - 8.0);
        self.text(Point::new(x + 4.0, self.y + BAND_HEIGHT - 3.6), &shown, style);
        self.y += BAND_HEIGHT;
    }

    /// Draws one line of a row: `key` on the left and, on the row's first line, `name` on the right.
    fn line(&mut self, key: &str, name: Option<&str>, shaded: bool) {
        let (x, w) = (self.column_left(), self.column_width());
        if shaded {
            self.rect(Rect::new(x, self.y, x + w, self.y + ROW_HEIGHT), Color::rgb8(0xee, 0xf0, 0xf2));
        }
        let baseline = self.y + ROW_HEIGHT - 2.6;
        self.text(Point::new(x + 3.0, baseline), key, shortcut_style(key, self.split - 6.0));
        if let Some(name) = name {
            let plain = style(ROW_SIZE, false, Color::BLACK);
            let name = fitted(name, &plain, w - self.split - 3.0);
            self.text(Point::new(x + self.split, baseline), &name, plain);
        }
        self.y += ROW_HEIGHT;
    }
}

/// The style of shortcut `key` in `room`: a shortcut is never cut short, and one too wide for
/// `room` is set smaller.
fn shortcut_style(key: &str, room: f64) -> CharStyle {
    let bold = style(ROW_SIZE, true, Color::BLACK);
    let wide = width(key, &bold);
    if wide > room && wide > 0.0 && room > 0.0 { style(ROW_SIZE * room / wide, true, Color::BLACK) } else { bold }
}

/// The separate shortcuts of a row's shortcut text, which the dialog joins with ` · `; a row has at
/// least one line.
fn shortcuts(text: &str) -> Vec<&str> {
    let keys: Vec<&str> = text.split(" · ").filter(|s| !s.is_empty()).collect();
    if keys.is_empty() { vec![""] } else { keys }
}

/// The sheet as a document on `page`-sized landscape artboards: `title` at the top of each page,
/// then each section that has a row with a shortcut under a colored band, its rows with a shortcut
/// below it (shortcut on the left, name on the right), flowed down the columns and onto further
/// pages. A section that continues in the next column repeats its band there. A row with several
/// shortcuts has a line for each, kept in one column unless the row is taller than a column, and a
/// shortcut is never shortened; a name too long for its column ends in an ellipsis.
pub fn sheet_document(title: &str, sections: &[Section], page: PageSize) -> Document {
    let page = page.landscape();
    let mut s = Sheet { doc: Document::new(page.0, page.1), page, pages: 0, column: 0, y: 0.0, split: 0.0 };
    // The shortcut column is as wide as the widest shortcut, within limits.
    let bold = style(ROW_SIZE, true, Color::BLACK);
    let widest = sections.iter().flat_map(|sec| &sec.rows).flat_map(|r| shortcuts(&r.shortcut)).map(|k| width(k, &bold)).fold(0.0, f64::max);
    let w = s.column_width();
    s.split = (widest + 6.0).clamp(w * SHORTCUT_SHARE, w * MAX_SHORTCUT_SHARE);
    s.doc.title = title.to_string();
    s.new_page(title);
    // The most a row takes in one piece: a column below its band.
    let column = s.bottom() - s.top() - BAND_HEIGHT;
    let whole = |row: &Row| shortcuts(&row.shortcut).len() as f64 * ROW_HEIGHT <= column;
    let piece = |row: &Row| if whole(row) { shortcuts(&row.shortcut).len() as f64 * ROW_HEIGHT } else { ROW_HEIGHT };
    let listed = sections.iter().filter(|sec| sec.rows.iter().any(|r| !r.shortcut.is_empty()));
    for (i, sec) in listed.enumerate() {
        let (r, g, b) = BANDS[i % BANDS.len()];
        let color = Color::rgb8(r, g, b);
        let space = if s.y > s.top() { SECTION_SPACE } else { 0.0 };
        let mut rows = sec.rows.iter().filter(|r| !r.shortcut.is_empty()).peekable();
        let first = rows.peek().map_or(ROW_HEIGHT, |r| piece(r));
        if !s.make_room(space + BAND_HEIGHT + first, title) {
            s.y += space;
        }
        s.band(&sec.title, color);
        for (n, row) in rows.enumerate() {
            if s.make_room(piece(row), title) {
                s.band(&sec.title, color);
            }
            let split = !whole(row);
            for (k, key) in shortcuts(&row.shortcut).into_iter().enumerate() {
                // A row taller than a column goes on in the next one, line by line.
                if k > 0 && split && s.make_room(ROW_HEIGHT, title) {
                    s.band(&sec.title, color);
                }
                s.line(key, (k == 0).then_some(row.name.as_str()), n % 2 == 1);
            }
        }
    }
    s.doc
}

/// The sheet as a PDF, one page per artboard of [`sheet_document`].
pub fn sheet_pdf(title: &str, sections: &[Section], page: PageSize) -> Result<Vec<u8>, String> {
    let doc = sheet_document(title, sections, page);
    let opts = vectorcraft_pdf::PdfOptions { title: Some(title.to_string()), ..Default::default() };
    vectorcraft_pdf::export(&doc, &opts).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, shortcut: &str) -> Row {
        Row { name: name.into(), shortcut: shortcut.into() }
    }

    /// Every type object in `doc`, in stacking order: its text, its style and its left edge.
    fn text_objects(doc: &Document) -> Vec<(String, CharStyle, f64)> {
        let mut out = vec![];
        let Some(layer) = doc.default_layer() else { return out };
        for n in doc.children(Some(layer)).into_iter().flatten() {
            if let NodeKind::Text(t) = &n.kind {
                out.push((t.plain_text(), t.first_style(), t.xf.translation().x));
            }
        }
        out
    }

    /// The text of every type object in `doc`, in stacking order.
    fn texts(doc: &Document) -> Vec<String> {
        text_objects(doc).into_iter().map(|(t, ..)| t).collect()
    }

    /// Every node of `doc` lies on one of its pages.
    fn assert_on_pages(doc: &Document) {
        let l = doc.default_layer().unwrap_or(NodeId(0));
        for n in doc.children(Some(l)).into_iter().flatten() {
            let Some(b) = n.geometric_bounds() else { continue };
            let on = |a: &vectorcraft_doc::Artboard| {
                let r = a.rect.inflate(0.5, 0.5);
                r.union(b) == r
            };
            assert!(doc.artboards.iter().any(on), "{b:?} is off the pages");
        }
    }

    /// A field with a comma, a double quote or a line break is quoted, with its double quotes
    /// doubled; other fields are written as they are, and each record ends in CRLF.
    #[test]
    fn csv_quotes_fields_with_commas_quotes_and_line_breaks() {
        let sections = [
            Section { title: "Edit".into(), rows: vec![row("Paste in Front, Place", "Ctrl+F"), row("Say \"Hi\"", "")] },
            Section { title: "Two\nLines".into(), rows: vec![row("Plain", "Ctrl+,")] },
        ];
        let text = csv(&sections, ["Group", "Command", "Shortcut"]);
        assert_eq!(
            text,
            "\u{feff}Group,Command,Shortcut\r\nEdit,\"Paste in Front, Place\",Ctrl+F\r\nEdit,\"Say \"\"Hi\"\"\",\r\n\"Two\nLines\",Plain,\"Ctrl+,\"\r\n"
        );
    }

    /// A field a spreadsheet would read as a formula starts with an apostrophe; a lone `+`, `-`,
    /// `=` or `@` key and other fields are written as they are.
    #[test]
    fn csv_keeps_fields_from_reading_as_formulas() {
        let sections = [Section {
            title: "Plug-ins".into(),
            rows: vec![row("=HYPERLINK(\"x\")", "+"), row("@Sum", "-"), row("-Minus", "- · Ctrl+-"), row("Tab\tName", "=")],
        }];
        let text = csv(&sections, ["Group", "Command", "Shortcut"]);
        assert_eq!(
            text,
            "\u{feff}Group,Command,Shortcut\r\nPlug-ins,\"'=HYPERLINK(\"\"x\"\")\",+\r\nPlug-ins,'@Sum,-\r\nPlug-ins,'-Minus,'- · Ctrl+-\r\nPlug-ins,Tab\tName,=\r\n"
        );
        assert_eq!(csv_field("\tx"), "'\tx");
        assert_eq!(csv_field("\rx"), "\"'\rx\"");
    }

    /// The sheet lists the rows that have a shortcut and leaves out the others, and a section with
    /// no shortcut at all has no band.
    #[test]
    fn sheet_leaves_out_rows_and_sections_without_a_shortcut() {
        let sections = [
            Section { title: "Tools".into(), rows: vec![row("Pen", "P"), row("Unbound Tool", "")] },
            Section { title: "Empty Menu".into(), rows: vec![row("Nothing", "")] },
            Section { title: "Edit".into(), rows: vec![row("Copy", "Ctrl+C")] },
        ];
        let doc = sheet_document("Keyboard Shortcuts · Custom", &sections, PageSize::Letter);
        let t = texts(&doc);
        assert_eq!(t, ["Keyboard Shortcuts · Custom", "Tools", "P", "Pen", "Edit", "Ctrl+C", "Copy"]);
    }

    /// Each page is a landscape artboard of the paper picked, and rows that do not fit on one page
    /// go on to more pages of the same size, each with the title.
    #[test]
    fn sheet_pages_are_landscape_pages_of_the_paper_picked() {
        let rows: Vec<Row> = (0..600).map(|i| row(&format!("Command {i}"), &format!("Ctrl+{i}"))).collect();
        let sections = [Section { title: "Edit".into(), rows }];
        for (page, (w, h)) in [(PageSize::Letter, (792.0, 612.0)), (PageSize::A4, (841.89, 595.28))] {
            let doc = sheet_document("Sheet", &sections, page);
            assert!(doc.artboards.len() >= 2, "{page:?}: {} pages", doc.artboards.len());
            for a in &doc.artboards {
                assert!((a.rect.width() - w).abs() < 1e-9 && (a.rect.height() - h).abs() < 1e-9, "{page:?}: {:?}", a.rect);
            }
            let t = texts(&doc);
            assert_eq!(t.iter().filter(|s| *s == "Sheet").count(), doc.artboards.len(), "a title on every page");
            assert_eq!(t.iter().filter(|s| s.starts_with("Command ")).count(), 600, "every row once");
            assert_on_pages(&doc);
            // Each column the section takes starts with its band.
            let mut columns: Vec<f64> = vec![];
            for (_, _, x) in text_objects(&doc).iter().filter(|(t, ..)| t.starts_with("Command ")) {
                if !columns.iter().any(|c| (c - x).abs() < 1e-6) {
                    columns.push(*x);
                }
            }
            assert!(columns.len() > COLUMNS, "{page:?}: {} columns", columns.len());
            assert_eq!(t.iter().filter(|s| *s == "Edit").count(), columns.len(), "{page:?}: a band in every column");
        }
        let pdf = sheet_pdf("Sheet", &[Section { title: "Edit".into(), rows: vec![row("Copy", "Ctrl+C")] }], PageSize::A4).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert_eq!(PageSize::parse("a4"), Some(PageSize::A4));
        assert_eq!(PageSize::parse("A4"), None);
        assert_eq!(PageSize::parse("legal"), None);
    }

    /// A row with more shortcuts than a column holds goes on in the next column, under its band
    /// again, and stays on the pages.
    #[test]
    fn a_row_taller_than_a_column_goes_on_in_the_next() {
        let keys: Vec<String> = (0..150).map(|i| format!("Ctrl+F{i}")).collect();
        let sections = [Section { title: "Edit".into(), rows: vec![row("Many", &keys.join(" · ")), row("After", "Ctrl+A")] }];
        let doc = sheet_document("Sheet", &sections, PageSize::Letter);
        assert_on_pages(&doc);
        let t = texts(&doc);
        for k in &keys {
            assert_eq!(t.iter().filter(|s| *s == k).count(), 1, "{k}");
        }
        assert_eq!(t.iter().filter(|s| *s == "Many").count(), 1);
        assert!(t.iter().filter(|s| *s == "Edit").count() >= 3, "{t:?}");
        assert!(t.contains(&"After".to_string()));
    }

    /// The sheet's font has the macOS modifier symbols the key names use, and the ellipsis.
    #[test]
    fn the_sheets_font_has_the_key_name_characters() {
        let db = vectorcraft_text::FontDb::global();
        for face in ["Regular", "Bold"] {
            let f = db.face(FONT, face).expect("the bundled face");
            for c in "⌃⌥⇧⌘…".chars() {
                assert!(f.covers(c), "{face} lacks {c} (U+{:04X})", c as u32);
            }
        }
    }

    /// A row with several shortcuts lists each on a line of its own, and a shortcut too wide for
    /// its column is kept whole and set smaller to fit.
    #[test]
    fn sheet_keeps_every_shortcut_of_a_row_whole() {
        let long = "Ctrl+Alt+Shift+".repeat(6) + "PageDown";
        let sections = [Section { title: "Edit".into(), rows: vec![row("Copy", "Ctrl+C · Ctrl+Insert"), row("Long", &long)] }];
        let objects = text_objects(&sheet_document("Sheet", &sections, PageSize::Letter));
        let t: Vec<&str> = objects.iter().map(|(t, ..)| t.as_str()).collect();
        assert_eq!(t, ["Sheet", "Edit", "Ctrl+C", "Copy", "Ctrl+Insert", long.as_str(), "Long"]);
        let (_, short, _) = &objects[2];
        assert_eq!(short.size, ROW_SIZE);
        let (_, set, _) = &objects[5];
        let (w, _) = PageSize::Letter.landscape();
        let column = (w - 2.0 * MARGIN - GUTTER * (COLUMNS - 1) as f64) / COLUMNS as f64;
        assert!(set.size < ROW_SIZE, "{}", set.size);
        assert!(width(&long, set) <= column * MAX_SHORTCUT_SHARE - 6.0 + 0.01, "{} wide", width(&long, set));
    }

    /// A name too long for its column is cut short with an ellipsis.
    #[test]
    fn a_long_name_is_cut_short_to_fit_its_column() {
        let long = "Command ".repeat(20);
        let doc = sheet_document("Sheet", &[Section { title: "Edit".into(), rows: vec![row(&long, "Ctrl+L")] }], PageSize::Letter);
        let shown = texts(&doc).into_iter().find(|t| t.starts_with("Command")).unwrap_or_default();
        assert!(shown.ends_with('…') && shown.len() < long.len(), "{shown}");
        let plain = style(ROW_SIZE, false, Color::BLACK);
        let room = 100.0;
        assert!(width(&shown, &plain) > 0.0);
        let fit = fitted(&long, &plain, room);
        assert!(width(&fit, &plain) <= room, "{fit}");
        let one_more = long.chars().take(fit.chars().count()).collect::<String>().trim_end().to_string() + "…";
        assert!(width(&one_more, &plain) > room || one_more == fit, "{fit} is the longest start that fits");
        // A name far longer than any line is measured only in part.
        let huge = "x".repeat(1_000_000);
        let title = fitted(&huge, &style(TITLE_SIZE, true, Color::BLACK), 700.0);
        assert!(title.ends_with('…') && title.chars().count() <= MAX_FIT_CHARS + 1, "{} chars", title.chars().count());
    }
}
