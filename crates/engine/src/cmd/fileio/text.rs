//! Export As Text (`.txt`): the document's stories, one after another in stacking order (back to
//! front), as UTF-8 or UTF-16 with LF or CRLF line endings. Threaded frames are one story,
//! written where its first frame is; hidden objects and template layers are left out. With
//! `paragraphs`, lines of point type that belong together are one paragraph (see [`paragraphs`]):
//! each paragraph ends with the line ending, and its lines are separated by `lineBreaks`. With
//! `ids`, each paragraph starts with its id in brackets (`[#58] `), which Import Text Paragraphs
//! ([`import`]) reads back.

mod import;
mod paragraphs;

pub(super) use import::import_paragraphs;

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::super::{Result, bad};
use super::FormatOption;

const C: &str = "document.export";

const ENCODING: FormatOption = FormatOption {
    name: "encoding",
    ty: "string",
    default: "\"utf8\"",
    description: "utf8 (no byte order mark) or utf16 (little-endian, with a byte order mark)",
};
const LINE_ENDINGS: FormatOption =
    FormatOption { name: "lineEndings", ty: "string", default: "\"lf\"", description: "lf (macOS, Linux) or crlf (Windows)" };
const SELECTION_ONLY: FormatOption =
    FormatOption { name: "selectionOnly", ty: "boolean", default: "false", description: "only the selected type objects (document.export)" };

const PARAGRAPHS: FormatOption = FormatOption {
    name: "paragraphs",
    ty: "boolean",
    default: "false",
    description: "lines of point type that belong together (one font family and size, a shared left edge, evenly spaced baselines; side-by-side columns continue left to right) are one paragraph, as a PDF's lines come in; each paragraph ends with lineEndings, and so do a story's paragraphs",
};
const LINE_BREAKS: FormatOption = FormatOption {
    name: "lineBreaks",
    ty: "string",
    default: "\"lf\"",
    description: "with paragraphs: what separates the lines of a paragraph (and a story's forced line breaks): lf, crlf, or space (each paragraph on one line)",
};

const IDS: FormatOption = FormatOption {
    name: "ids",
    ty: "boolean",
    default: "false",
    description: "each paragraph starts with its id in brackets and a space (implies paragraphs): `[#58] ` for lines of point type (the id of the first line), `[#120.2] ` for a story's second paragraph (the id of the type object, a thread's first frame); text.importParagraphs puts edited or translated paragraphs back by them",
};

/// The options `document.formats` lists for text.
pub(super) const OPTIONS: &[FormatOption] = &[ENCODING, LINE_ENDINGS, PARAGRAPHS, LINE_BREAKS, IDS, SELECTION_ONLY];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TextOptions {
    encoding: Option<String>,
    line_endings: Option<String>,
    paragraphs: bool,
    line_breaks: Option<String>,
    ids: bool,
}

/// The text file of `doc` with the text options of `p`.
pub(super) fn encode(doc: &Document, p: &Value) -> Result<Vec<u8>> {
    let o = if p.is_object() { TextOptions::deserialize(p).map_err(|e| bad(C, format!("Text options: {e}")))? } else { TextOptions::default() };
    let eol = match o.line_endings.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("lf") => "\n",
        Some("crlf") => "\r\n",
        Some(other) => return Err(bad(C, format!("lineEndings `{other}`: lf or crlf"))),
    };
    let mut text = String::new();
    if o.paragraphs || o.ids {
        let sep = match o.line_breaks.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("lf") => "\n",
            Some("crlf") => "\r\n",
            Some("space") => " ",
            Some(other) => return Err(bad(C, format!("lineBreaks `{other}`: lf, crlf or space"))),
        };
        for para in document_paragraphs(doc) {
            let lines: Vec<&str> = if sep == " " {
                para.lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect()
            } else {
                // A wrapped line's last space isn't text.
                para.lines.iter().map(|l| l.trim_end()).collect()
            };
            if o.ids {
                text.push_str(&format!("[#{}] ", para.id()));
            }
            text.push_str(&lines.join(sep));
            text.push_str(eol);
        }
    } else {
        for piece in pieces(doc, false) {
            let Piece::Story(_, story) = piece else { continue };
            // Any line break in the story becomes the chosen one; every story ends with one.
            let story = story.replace("\r\n", "\n").replace('\r', "\n");
            for line in story.split('\n') {
                text.push_str(line);
                text.push_str(eol);
            }
        }
    }
    match o.encoding.as_deref().map(|e| e.to_ascii_lowercase().replace(['-', '_'], "")).as_deref() {
        None | Some("utf8") => Ok(text.into_bytes()),
        Some("utf16" | "utf16le" | "unicode") => Ok([0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()),
        Some(_) => Err(bad(C, format!("encoding `{}`: utf8 or utf16", o.encoding.unwrap_or_default()))),
    }
}

/// What a story, or a line of point type, gives the text file.
enum Piece {
    /// A type object's story (a thread's: its first frame's).
    Story(NodeId, String),
    Line(paragraphs::Line),
}

/// Where a paragraph of the text file comes from.
enum Source {
    /// Lines of point type, in reading order.
    Lines { lines: Vec<paragraphs::Line>, justified: bool, leading: Option<f64> },
    /// Paragraph `k` (from 0) of the story of type object `id` (a thread's first frame).
    Story { id: NodeId, k: usize },
}

/// A paragraph of the text file: its lines and where they come from.
struct Para {
    lines: Vec<String>,
    source: Source,
}

impl Para {
    /// Its id: the first line's for lines of point type, `<story>.<paragraph from 1>` in a story.
    fn id(&self) -> String {
        match &self.source {
            Source::Lines { lines, .. } => lines.first().map(|l| l.id.0.to_string()).unwrap_or_default(),
            Source::Story { id, k } => format!("{}.{}", id.0, k.saturating_add(1)),
        }
    }
}

/// The paragraphs of the text file: a story's paragraphs where the story is, and the paragraphs
/// that lines of point type make where their first line is (stacking order, back to front).
fn document_paragraphs(doc: &Document) -> Vec<Para> {
    let mut lines = vec![];
    // Each piece in stacking order: a story, or its line's index in `lines`.
    let mut slots: Vec<std::result::Result<(NodeId, String), usize>> = vec![];
    for piece in pieces(doc, true) {
        match piece {
            Piece::Story(id, s) => slots.push(Ok((id, s))),
            Piece::Line(mut l) => {
                // Lines on different artboards (a PDF's pages) never make one paragraph.
                let at = vectorcraft_geom::Point::new(l.left, l.baseline);
                l.board = doc.artboards.iter().position(|a| a.rect.contains(at));
                slots.push(Err(lines.len()));
                lines.push(l);
            }
        }
    }
    let groups = paragraphs::paragraphs(&lines);
    // The paragraph each line starts, if it is the first of its paragraph's lines.
    let mut starts: Vec<Option<usize>> = vec![None; lines.len()];
    for (k, g) in groups.iter().enumerate() {
        if let Some(slot) = g.lines.iter().copied().min().and_then(|first| starts.get_mut(first)) {
            *slot = Some(k);
        }
    }
    let mut out = vec![];
    for slot in slots {
        match slot {
            Ok((id, story)) => {
                let story = story.replace("\r\n", "\n").replace('\r', "\n");
                out.extend(
                    story
                        .split('\n')
                        .enumerate()
                        .map(|(k, para)| Para { lines: para.split('\u{2028}').map(str::to_string).collect(), source: Source::Story { id, k } }),
                );
            }
            Err(i) => {
                if let Some(g) = starts.get(i).copied().flatten().and_then(|k| groups.get(k)) {
                    let of: Vec<paragraphs::Line> = g.lines.iter().filter_map(|&j| lines.get(j)).cloned().collect();
                    out.push(Para {
                        lines: of.iter().map(|l| l.text.clone()).collect(),
                        source: Source::Lines { lines: of, justified: g.justified, leading: g.leading },
                    });
                }
            }
        }
    }
    out
}

/// Every visible story in stacking order (back to front); with `lines`, a line of point type
/// as its [`paragraphs::Line`].
fn pieces(doc: &Document, lines: bool) -> Vec<Piece> {
    let mut out = vec![];
    let mut done = HashSet::new();
    for l in &doc.layers {
        collect(doc, l, lines, &mut done, &mut out);
    }
    out
}

fn collect(doc: &Document, n: &Node, lines: bool, done: &mut HashSet<NodeId>, out: &mut Vec<Piece>) {
    if !n.visible || matches!(n.kind, NodeKind::Layer { template: true, .. }) {
        return;
    }
    if let NodeKind::Text(t) = &n.kind {
        if done.contains(&n.id) {
            return;
        }
        // A threaded frame: the whole thread's story, once.
        match doc.text_threads.iter().find(|th| th.contains(&n.id)) {
            Some(thread) => {
                done.extend(thread.iter().copied());
                let frames = thread.iter().filter_map(|id| doc.node(*id)).filter_map(|f| match &f.kind {
                    NodeKind::Text(t) => Some(t.plain_text()),
                    _ => None,
                });
                out.push(Piece::Story(thread.first().copied().unwrap_or(n.id), frames.collect()));
            }
            None => match lines.then(|| paragraphs::Line::of(n.id, t)).flatten() {
                Some(line) => out.push(Piece::Line(line)),
                None => out.push(Piece::Story(n.id, t.plain_text())),
            },
        }
        return;
    }
    for c in n.children().into_iter().flatten() {
        collect(doc, c, lines, done, out);
    }
}
