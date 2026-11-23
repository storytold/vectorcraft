//! Import Text Paragraphs (`text.importParagraphs`): put edited or translated paragraphs back by
//! the ids Export As Text writes with `ids` (`[#58] …`, `[#120.2] …`). Each id starts a
//! paragraph, which runs to the next id; its lines are the lines of the file.
//! - A paragraph of lines of point type becomes area type set in its first line's font, size and
//!   leading, justified or aligned left as the lines were, so the new text wraps: one frame per
//!   column it ran through, threaded, the last growing with its text. The first frame keeps the
//!   first line's id, so the file's id still finds it.
//! - A story's paragraph is replaced in place; its lines are joined by forced line breaks.
//!
//! A paragraph whose text is the document's already, and an id the document doesn't have, are
//! left alone. All of it is one undo step.

use std::collections::HashMap;

use kurbo::{Affine, Rect};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Justify, NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::shapes;
use vectorcraft_text::FontDb;

use super::super::super::typecmd::{refresh_bounds, with_overflow};
use super::super::super::{Result, Session, bad, str_param};
use super::super::access::read_file_head;
use super::paragraphs::Line;
use super::{Para, Source, document_paragraphs};

const C: &str = "text.importParagraphs";

/// The largest text file read (bytes).
const MAX_BYTES: usize = 16 << 20;
/// The most paragraphs read from one file.
const MAX_PARAGRAPHS: usize = 200_000;

/// `text.importParagraphs`.
pub(in super::super) fn import_paragraphs(s: &mut Session, p: &Value) -> Result<Value> {
    let bytes = if let Some(path) = str_param(p, "path") {
        let b = read_file_head(path, MAX_BYTES + 1)?;
        if b.len() > MAX_BYTES {
            return Err(bad(C, format!("{path}: larger than {} MB", MAX_BYTES >> 20)));
        }
        b
    } else if let Some(data) = str_param(p, "data") {
        data.as_bytes().to_vec()
    } else if let Some(b64) = str_param(p, "dataBase64") {
        vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "dataBase64 isn't base64"))?
    } else {
        return Err(bad(C, "give path, data or dataBase64"));
    };
    let entries = entries(&decode(&bytes))?;
    let doc = &s.doc()?.doc;
    let paras = document_paragraphs(doc);
    let by_id: HashMap<String, &Para> = paras.iter().map(|p| (p.id(), p)).collect();
    let (mut edits, mut missing, mut unchanged) = (vec![], vec![], 0);
    for (id, lines) in &entries {
        // A plain id that is now a story (lines imported before): its one paragraph.
        let Some(para) = by_id.get(id).or_else(|| by_id.get(&format!("{id}.1")).filter(|_| !id.contains('.'))) else {
            missing.push(id.clone());
            continue;
        };
        let text = joined(&para.source, lines);
        if text == joined(&para.source, &para.lines) {
            unchanged += 1;
        } else {
            edits.push((*para, text));
        }
    }
    if edits.is_empty() {
        return Ok(json!({ "replaced": 0, "unchanged": unchanged, "missing": missing, "ids": [] }));
    }
    // What each edit needs, read before the document is borrowed for writing.
    let plans: Vec<Plan> = edits.into_iter().filter_map(|(para, text)| plan(doc, para, text)).collect();
    let replaced = plans.len();
    let ids = s.edit("Import Text Paragraphs", |d, sel| {
        let mut ids = vec![];
        for plan in plans {
            ids.extend(apply(d, plan)?);
        }
        sel.objects = ids.clone();
        sel.anchors.clear();
        Ok(ids)
    })?;
    let result = json!({ "replaced": replaced, "unchanged": unchanged, "missing": missing, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() });
    Ok(with_overflow(s, &ids, result))
}

/// The text of a file: UTF-16 with a byte order mark, else UTF-8 (an invalid byte becomes U+FFFD).
fn decode(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], le: bool| {
        let unit = |b: &[u8; 2]| if le { u16::from_le_bytes(*b) } else { u16::from_be_bytes(*b) };
        String::from_utf16_lossy(&rest.as_chunks::<2>().0.iter().map(unit).collect::<Vec<u16>>())
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The id a line starts with (`[#58] `, `[#120.2] `) and the rest of the line.
fn marker(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("[#")?;
    let (id, after) = rest.split_once(']')?;
    let mut parts = id.split('.');
    let digits = |s: Option<&str>| s.is_some_and(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()));
    let ok = digits(parts.next()) && parts.next().is_none_or(|k| digits(Some(k))) && parts.next().is_none();
    ok.then(|| (id, after.strip_prefix(' ').unwrap_or(after)))
}

/// The paragraphs of `text`: each id and its lines (without the empty lines that end it).
fn entries(text: &str) -> Result<Vec<(String, Vec<String>)>> {
    let mut out: Vec<(String, Vec<String>)> = vec![];
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match marker(line) {
            Some((id, rest)) => {
                if out.len() >= MAX_PARAGRAPHS {
                    return Err(bad(C, format!("more than {MAX_PARAGRAPHS} paragraphs")));
                }
                out.push((id.to_string(), vec![rest.to_string()]));
            }
            // Text before the first id belongs to no paragraph.
            None => {
                if let Some((_, lines)) = out.last_mut() {
                    lines.push(line.to_string());
                }
            }
        }
    }
    for (_, lines) in &mut out {
        while lines.len() > 1 && lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
    }
    if out.is_empty() {
        return Err(bad(C, "no paragraph ids ([#58] …): export the text with ids first"));
    }
    Ok(out)
}

/// `lines` as the text that replaces a paragraph from `source`: one run of words for lines of
/// point type (the frame wraps them), lines joined by forced line breaks in a story.
fn joined(source: &Source, lines: &[String]) -> String {
    match source {
        Source::Lines { .. } => lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" "),
        Source::Story { .. } => lines.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join("\u{2028}"),
    }
}

/// An edit to make.
enum Plan {
    /// Lines of point type → frames (`frames`: each column's first line and its frame), the other
    /// lines removed.
    Frames { frames: Vec<(NodeId, vectorcraft_doc::Node)>, remove: Vec<NodeId> },
    /// Paragraph `k` of the story of `id` (and its thread) becomes `text`.
    Story { id: NodeId, k: usize, text: String },
}

/// The edit that puts `text` in place of `para`; `None` when its lines are gone.
fn plan(doc: &Document, para: &Para, text: String) -> Option<Plan> {
    match &para.source {
        Source::Story { id, k } => Some(Plan::Story { id: *id, k: *k, text }),
        Source::Lines { lines, justified, leading } => frames(doc, lines, *justified, *leading, &text),
    }
}

/// The area type frames that set `text` where `lines` were: one per column (a column starts where
/// the next line is above the last), as wide as its lines (justified: as the longest line).
fn frames(doc: &Document, lines: &[Line], justified: bool, leading: Option<f64>, text: &str) -> Option<Plan> {
    let first = lines.first()?;
    let node = doc.node(first.id)?;
    let NodeKind::Text(t0) = &node.kind else { return None };
    let scale = t0.xf.as_coeffs()[0];
    let lead = leading.unwrap_or(first.size * 1.2);
    let mut columns: Vec<Vec<&Line>> = vec![];
    for l in lines {
        match columns.last_mut() {
            Some(col) if col.last().is_some_and(|prev| l.baseline >= prev.baseline - 0.5 * l.size) => col.push(l),
            _ => columns.push(vec![l]),
        }
    }
    let left_of = |col: &[&Line]| col.iter().map(|l| l.left).fold(f64::INFINITY, f64::min);
    let right_of = |col: &[&Line]| col.iter().map(|l| l.left + l.width).fold(f64::NEG_INFINITY, f64::max);
    let widest = columns.iter().map(|c| right_of(c) - left_of(c)).fold(0.0, f64::max);
    // The style most of the first line's letters are set in (not a bold lead-in), at the lines' leading.
    let letters = |r: &TextRun| r.text.chars().filter(|c| c.is_alphanumeric()).count();
    let mut style = t0.runs.iter().max_by_key(|r| letters(r)).map(|r| r.style.clone()).unwrap_or_default();
    style.leading = Some(lead / scale);
    let mut base = TextObject::clone(t0);
    base.paras.clear();
    base.ruby.clear();
    base.area = Default::default();
    base.para.justify = if justified { Justify::JustifyLeft } else { Justify::Left };
    base.para.first_line_indent = 0.0;
    // Where the first baseline sits below a frame's top.
    base.runs = vec![TextRun { text: "Hg".into(), style: style.clone(), inline: None }];
    base.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, widest / scale + first.size, 4.0 * lead / scale)) };
    let drop = vectorcraft_text::layout(FontDb::global(), &base).lines.first().map_or(first.size * 0.8 / scale, |l| l.baseline);
    let count = columns.len();
    let mut out = vec![];
    for (i, col) in columns.iter().enumerate() {
        let (Some(head), left) = (col.first(), left_of(col)) else { continue };
        let width = if justified { widest } else { right_of(col) - left } + 0.05 * first.size;
        let height = col.len() as f64 * lead;
        if ![left, width, height, head.baseline].iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut t = base.clone();
        t.runs = vec![TextRun { text: if i == 0 { text.to_string() } else { String::new() }, style: style.clone(), inline: None }];
        if i == 0 {
            t.para.first_line_indent = (first.left - left) / scale;
        }
        if i + 1 == count {
            t.area.fit = vectorcraft_doc::AreaFit::AutoHeight;
        }
        t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, -drop, width / scale, height / scale - drop)) };
        t.xf = Affine::new([scale, 0.0, 0.0, scale, left, head.baseline]);
        refresh_bounds(&mut t);
        let mut frame = doc.node(head.id).cloned()?;
        frame.kind = NodeKind::Text(Box::new(t));
        out.push((head.id, frame));
    }
    let heads: Vec<NodeId> = out.iter().map(|(id, _)| *id).collect();
    Some(Plan::Frames { remove: lines.iter().map(|l| l.id).filter(|id| !heads.contains(id)).collect(), frames: out })
}

/// Make `plan` in `d` → the ids of the type objects it changed or made.
fn apply(d: &mut Document, plan: Plan) -> Result<Vec<NodeId>> {
    match plan {
        Plan::Frames { frames, remove } => {
            let mut ids = vec![];
            let mut items = vec![];
            for (k, (head, mut frame)) in frames.into_iter().enumerate() {
                // The first frame keeps the first line's id; the others are new.
                frame.id = if k == 0 { head } else { d.alloc_id() };
                ids.push(frame.id);
                items.push((head, vec![frame]));
            }
            d.insert_beside(items, true).map_err(|e| bad(C, e.to_string()))?;
            d.remove_many(&remove).map_err(|e| bad(C, e.to_string()))?;
            if ids.len() >= 2 {
                d.text_threads.push(ids.clone());
            }
            Ok(ids)
        }
        Plan::Story { id, k, text } => {
            let thread = d.text_threads.iter().find(|th| th.contains(&id)).cloned().unwrap_or_else(|| vec![id]);
            let Some((&first, rest)) = thread.split_first() else { return Ok(vec![]) };
            let mut runs: Vec<TextRun> = vec![];
            for fid in &thread {
                if let Some(NodeKind::Text(t)) = d.node(*fid).map(|n| &n.kind) {
                    runs.extend(t.runs.iter().cloned());
                }
            }
            let plain: String = runs.iter().map(|r| r.text.as_str()).collect();
            let Some((start, end)) = paragraph_range(&plain, k) else { return Ok(vec![]) };
            let Some(NodeKind::Text(t)) = d.node_mut(first).map(|n| &mut n.kind) else { return Ok(vec![]) };
            if rest.is_empty() {
                t.splice_paras(start, end, &text);
            }
            vectorcraft_text::edit::replace_range(&mut runs, start, end, &text);
            // The whole story goes in the first frame; the thread flows it on after the edit.
            t.runs = runs;
            t.ruby.clear();
            refresh_bounds(t);
            for fid in rest {
                if let Some(NodeKind::Text(t)) = d.node_mut(*fid).map(|n| &mut n.kind) {
                    t.runs.clear();
                    t.ruby.clear();
                }
            }
            Ok(vec![first])
        }
    }
}

/// The byte range of paragraph `k` (from 0) of `plain`.
fn paragraph_range(plain: &str, k: usize) -> Option<(usize, usize)> {
    let mut start = 0;
    for (i, para) in plain.split('\n').enumerate() {
        if i == k {
            return Some((start, start + para.len()));
        }
        start += para.len() + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_start_paragraphs_that_run_to_the_next() {
        let text = "stray\r\n[#58] One\nline two\r\n[#120.2] Zwei\r\n\r\n[#7]\n[#x] not an id\n[#1.2.3] nor this\n";
        let got = entries(text).unwrap();
        let ids: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["58", "120.2", "7"]);
        assert_eq!(got[0].1, ["One", "line two"]);
        assert_eq!(got[1].1, ["Zwei"], "the empty lines that end it go");
        assert_eq!(got[2].1, ["", "[#x] not an id", "[#1.2.3] nor this"]);
        assert!(entries("no ids here").is_err());
        assert_eq!(decode(&[0xFF, 0xFE, b'[', 0, b'#', 0]), "[#");
        assert_eq!(decode(&[0xEF, 0xBB, 0xBF, b'a']), "a");
        assert_eq!(paragraph_range("ab\ncde\n", 1), Some((3, 6)));
        assert_eq!(paragraph_range("ab", 1), None);
    }
}
