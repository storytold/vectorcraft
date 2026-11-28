//! Export As Text (`.txt`): the document's stories, one after another in stacking order (back to
//! front), as UTF-8 or UTF-16 with LF or CRLF line endings. Threaded frames are one story,
//! written where its first frame is; hidden objects and template layers are left out.

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

/// The options `document.formats` lists for text.
pub(super) const OPTIONS: &[FormatOption] = &[ENCODING, LINE_ENDINGS, SELECTION_ONLY];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TextOptions {
    encoding: Option<String>,
    line_endings: Option<String>,
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
    for story in stories(doc) {
        // Any line break in the story becomes the chosen one; every story ends with one.
        let story = story.replace("\r\n", "\n").replace('\r', "\n");
        for line in story.split('\n') {
            text.push_str(line);
            text.push_str(eol);
        }
    }
    match o.encoding.as_deref().map(|e| e.to_ascii_lowercase().replace(['-', '_'], "")).as_deref() {
        None | Some("utf8") => Ok(text.into_bytes()),
        Some("utf16" | "utf16le" | "unicode") => Ok([0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect()),
        Some(_) => Err(bad(C, format!("encoding `{}`: utf8 or utf16", o.encoding.unwrap_or_default()))),
    }
}

/// The text of every visible story in stacking order (back to front).
fn stories(doc: &Document) -> Vec<String> {
    let mut out = vec![];
    let mut done = HashSet::new();
    for l in &doc.layers {
        collect(doc, l, &mut done, &mut out);
    }
    out
}

fn collect(doc: &Document, n: &Node, done: &mut HashSet<NodeId>, out: &mut Vec<String>) {
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
                out.push(frames.collect());
            }
            None => out.push(t.plain_text()),
        }
        return;
    }
    for c in n.children().into_iter().flatten() {
        collect(doc, c, done, out);
    }
}
