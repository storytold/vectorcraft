//! Type set in fonts that aren't available as named asks before it is edited: a click or a
//! double-click that would start editing it opens a question ([`MISSING_FONT_EDIT`]) instead, and
//! editing starts at the clicked place only after OK (the tool option `missingFontEdit`). The
//! fonts are those `text.missingFonts` reads, for the story of the type clicked only.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, TextRun};
use vectorcraft_text::{FontDb, FontMatch};

/// The dialog kind of the question, with the fields `token`, `id`, `missing` and `style` (lists of
/// "family style" names) and `more` (how many more fonts the lists leave out).
pub const MISSING_FONT_EDIT: &str = "missingFontEdit";

/// The most fonts the question names.
const LISTED: usize = 8;

/// The longest family or style name checked (bytes), as `text.missingFonts` reads them.
const MAX_NAME: usize = 256;

/// An edit of type set in fonts that aren't available, waiting for the answer to its question.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PendingEdit {
    /// The question's token, which its answer names.
    pub token: u64,
    /// The document (its uid) and its revision when the question was asked: an answer to a
    /// document since changed starts no editing.
    pub revision: (u64, u64),
    /// The type clicked, and the caret's byte offset there.
    pub id: NodeId,
    pub byte: usize,
    /// OK was chosen: the next [`crate::Tool::poll`] starts editing.
    pub accepted: bool,
}

/// The fonts (family, style) run `r` is drawn in: its own, or, set in a composite font, those of
/// the parts its characters fall in. An inline graphic and an empty run draw no glyphs.
fn run_fonts(db: &FontDb, r: &TextRun) -> Vec<(String, String)> {
    if r.inline.is_some() || r.text.is_empty() {
        return vec![];
    }
    db.composite_fonts_for(&r.style.font_family, &r.text).unwrap_or_else(|| vec![(r.style.font_family.clone(), r.style.font_style.clone())])
}

/// The fonts that the story of type object `id` ([`Document::story_of`]: every frame threaded with
/// it) is drawn in and that aren't available as named, each once, in the order first used, with how
/// they resolve ([`FontMatch::Missing`] or [`FontMatch::Style`]); at most [`LISTED`], with how many
/// more there are. CSS generic families and names longer than 256 bytes are left out, as
/// `text.missingFonts` leaves them out.
pub(super) fn unavailable_fonts(doc: &Document, id: NodeId) -> (Vec<(String, String, FontMatch)>, usize) {
    let db = FontDb::global();
    let mut seen = BTreeSet::new();
    let mut listed = Vec::new();
    let mut more = 0;
    for frame in doc.story_of(id) {
        let Some(NodeKind::Text(t)) = doc.node(frame).map(|n| &n.kind) else { continue };
        for (family, style) in t.runs.iter().flat_map(|r| run_fonts(db, r)) {
            if family.trim().is_empty() || vectorcraft_text::is_generic_family(&family) || family.len() > MAX_NAME || style.len() > MAX_NAME {
                continue;
            }
            if !seen.insert((family.clone(), style.clone())) {
                continue;
            }
            let found = db.resolve(&family, &style).map_or(FontMatch::Missing, |(_, m)| m);
            if found == FontMatch::Exact {
                continue;
            }
            if listed.len() < LISTED {
                listed.push((family, style, found));
            } else {
                more += 1;
            }
        }
    }
    (listed, more)
}

/// The question's fields for `fonts` ([`unavailable_fonts`]) and the edit `token` of type `id`.
pub(super) fn question(token: u64, id: NodeId, (fonts, more): &(Vec<(String, String, FontMatch)>, usize)) -> Value {
    let names = |m: FontMatch| fonts.iter().filter(|f| f.2 == m).map(|(family, style, _)| format!("{family} {style}")).collect::<Vec<_>>();
    json!({"token": token, "id": id.0, "missing": names(FontMatch::Missing), "style": names(FontMatch::Style), "more": more})
}
