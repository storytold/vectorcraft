//! Inline graphics in text: document symbols placed in a run of text, flowing like a glyph
//! (beyond Illustrator; InDesign's inline anchored objects). See [`vectorcraft_doc::InlineArt`].

use serde_json::{Value, json};
use vectorcraft_doc::text::INLINE_MAX_SCALE;
use vectorcraft_doc::{Document, InlineArt, NodeId, NodeKind, TextRun};
use vectorcraft_text::edit;

use super::typecmd::refresh_bounds;
use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_doc, id_param, str_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "text.insertInline",
        "Insert Inline Symbol",
        ["Type"],
        None,
        "{id?: text id (default: the text the Type tool edits), at?: byte offset (default: the Type tool's selection, which it replaces; clamped to the text), symbol?: symbol name (default: the Symbols panel's current symbol), scale?: art height ÷ font size = 1 (0 < scale ≤ 100), shift?: pt raise = 0} place a document symbol inline in the text as one character (U+FFFC) that flows like a glyph: its art is scaled to `scale` × the font size tall, its vertical centre on the middle of the cap height raised by `shift`. Typing, Backspace and Delete over it remove it whole → {id, caret}",
        has_doc,
        insert_inline
    )]
}

/// Inline graphics follow their symbols: after an edit of a document that has (or had) symbols,
/// resolve the inline art of every text again (only texts whose art changed are touched).
pub(crate) fn refresh(before: &Document, d: &mut Document) {
    if before.symbols.is_empty() && d.symbols.is_empty() {
        return;
    }
    for id in d.resolve_inline_art() {
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            refresh_bounds(t);
        }
    }
}

fn insert_inline(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.insertInline";
    let tool = s.tool_options();
    let editing = tool.get("editing").and_then(Value::as_u64).map(NodeId);
    let id = match p.get("id") {
        None | Some(Value::Null) => editing.ok_or_else(|| bad(C, "give `id` (no text is being edited)"))?,
        Some(_) => id_param(p, "id").ok_or_else(|| bad(C, "`id` must be a node id"))?,
    };
    let doc = &s.doc()?.doc;
    let NodeKind::Text(t) = &doc.node(id).ok_or(EngineError::NoNode(id))?.kind else {
        return Err(bad(C, format!("node {} is not text", id.0)));
    };
    let len = edit::runs_len(&t.runs);
    let from_tool = editing == Some(id);
    let (a, b) = match p.get("at") {
        None | Some(Value::Null) if from_tool => {
            let g = |k: &str| tool.get(k).and_then(Value::as_u64).map_or(len, |v| (v as usize).min(len));
            let (x, y) = (g("start"), g("end"));
            (x.min(y), x.max(y))
        }
        None | Some(Value::Null) => (len, len),
        Some(v) => {
            let at = v.as_u64().ok_or_else(|| bad(C, "`at` must be a byte offset"))?.min(len as u64) as usize;
            (at, at)
        }
    };
    let symbol = match str_param(p, "symbol") {
        Some(n) if doc.symbols.iter().any(|s| s.name == n) => n.to_string(),
        Some(n) => return Err(bad(C, format!("no symbol named `{n}`"))),
        None if p.get("symbol").is_some_and(|v| !v.is_null()) => return Err(bad(C, "`symbol` must be a symbol name")),
        None => current_symbol(doc).ok_or_else(|| EngineError::Other("the document has no symbols".into()))?,
    };
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 1.0,
        Some(v) => v
            .as_f64()
            .filter(|k| k.is_finite() && *k > 0.0 && *k <= INLINE_MAX_SCALE)
            .ok_or_else(|| bad(C, "`scale` must be a number in (0, 100]"))?,
    };
    let shift = match p.get("shift") {
        None | Some(Value::Null) => 0.0,
        Some(v) => v.as_f64().filter(|k| k.is_finite() && k.abs() <= 1e5).ok_or_else(|| bad(C, "`shift` must be a number of points"))?,
    };
    // A typing session ends first: the insertion is a step of its own.
    let typing = tool.get("typing").and_then(Value::as_bool) == Some(true);
    if typing && from_tool {
        s.set_tool_option("commitTyping", &Value::Bool(true));
        s.commit_interaction()?;
    }
    let caret = s.edit("Insert Inline Symbol", |d, _| {
        let mut art = InlineArt { scale, baseline_shift: shift, ..InlineArt::new(symbol.clone()) };
        art.bounds = d.symbol_art_bounds(&symbol);
        let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(id)) };
        let style = edit::insertion_style(&t.runs, a, b);
        let caret = edit::replace_range_styled(&mut t.runs, a, b, &[TextRun::inline(art, style)]);
        // The inline character is one plain-text byte range: ruby over it follows the text.
        t.ruby_after_replace(a, b, vectorcraft_doc::text::INLINE_CHAR.len_utf8());
        refresh_bounds(t);
        Ok(caret)
    })?;
    if from_tool {
        s.set_tool_option("select", &json!({"start": caret, "end": caret}));
    }
    Ok(json!({"id": id.0, "caret": caret}))
}

/// The Symbols panel's current symbol, else the first one.
fn current_symbol(d: &Document) -> Option<String> {
    d.unknown
        .get("currentSymbol")
        .and_then(Value::as_str)
        .filter(|n| d.symbols.iter().any(|s| s.name == *n))
        .map(str::to_string)
        .or_else(|| d.symbols.first().map(|s| s.name.clone()))
}
