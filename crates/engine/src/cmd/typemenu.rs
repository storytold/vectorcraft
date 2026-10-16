//! Type menu long tail (Change Case, Smart Punctuation, Convert To Area/Point Type, Type on a Path
//! Options, Fill with Placeholder Text, Insert Special/Whitespace/Break characters) and the text
//! commands of the Edit menu (Find and Replace, Find Next, Paste without Formatting).

use serde_json::{Value, json};
use vectorcraft_doc::{CharStyle, Document, Justify, NodeId, NodeKind, Quotes, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::typecmd::refresh_bounds;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "type.changeCase",
            "Change Case",
            ["Type", "Change Case"],
            None,
            "{case: \"upper\"|\"lower\"|\"title\"|\"sentence\", ids?|id?} → {ids}",
            has_selection,
            change_case
        ),
        cmd!(
            "type.smartPunctuation",
            "Smart Punctuation…",
            ["Type"],
            None,
            "{quotes?: true (straight quotes become the Document Setup quotes), dashes?: true (-- → en dash, --- → em dash), ellipsis?: true (... → …), scope?: \"selection\"|\"document\"} → {changed}",
            has_doc,
            smart_punctuation
        ),
        cmd!(
            "type.convertToAreaType",
            "Convert To Area Type",
            ["Type"],
            None,
            "{ids?|id?} point type → area type framed by its current layout bounds → {ids}",
            has_selection,
            to_area
        ),
        cmd!(
            "type.convertToPointType",
            "Convert To Point Type",
            ["Type"],
            None,
            "{ids?|id?} area type → point type; soft line wraps become line breaks → {ids}",
            has_selection,
            to_point
        ),
        cmd!(
            "type.pathOptions",
            "Type on a Path Options…",
            ["Type", "Type on a Path"],
            None,
            "{ids?|id?, start?: 0..1, end?: 0..1|null, flip?: bool, effect?: rainbow|skew|3dRibbon|stairStep|gravity, alignToPath?: ascender|descender|center|baseline, spacing?: pt} set the selected type on a path's options: start and end place its start and end brackets as fractions of the path's length (end null: the end of the path, or once round a closed path), and flip then turns the type to the other side of its path (the path runs the other way, the brackets swap ends); none given: query → the first object's {start, end, flip: false, effect, alignToPath, spacing}",
            has_selection,
            path_options
        ),
        cmd!(
            "type.fillPlaceholder",
            "Fill with Placeholder Text",
            ["Type"],
            None,
            "{ids?|id?} replace selected text with placeholder text (area type is filled to its frame) → {ids}",
            has_selection,
            fill_placeholder
        ),
        cmd!(
            "type.insert",
            "Insert Character",
            ["Type"],
            None,
            "{char: \"bullet\"|\"copyright\"|\"ellipsis\"|\"paragraph\"|\"registered\"|\"section\"|\"trademark\"|\"emDash\"|\"enDash\"|\"discretionaryHyphen\"|\"nonBreakingHyphen\"|\"doubleLeftQuote\"|\"doubleRightQuote\"|\"singleLeftQuote\"|\"singleRightQuote\"|\"emSpace\"|\"enSpace\"|\"hairSpace\"|\"sixthSpace\"|\"thinSpace\"|\"nonBreakingSpace\"|\"figureSpace\"|\"punctuationSpace\"|\"thirdSpace\"|\"quarterSpace\"|\"tab\"|\"forcedLineBreak\"|\"paragraphReturn\" | text: string, ids?|id?: object ids (default: the selection)} append to the selected text objects",
            has_selection,
            insert_char
        ),
        cmd!(
            "edit.findReplace",
            "Find and Replace…",
            ["Edit"],
            None,
            "{find, replace?: \"\", matchCase?: false, wholeWord?: false, ids?} replace in every text object (or ids) → {count}. Matches never span style runs.",
            has_doc,
            find_replace
        ),
        cmd!(
            "edit.findNext",
            "Find Next",
            ["Edit"],
            None,
            "{find, matchCase?, wholeWord?} select the next text object (after the selected one, wrapping) containing `find` → {id|null}",
            has_doc,
            find_next
        ),
        cmd!(
            "edit.pasteWithoutFormatting",
            "Paste without Formatting",
            ["Edit"],
            Some("Cmd+Alt+V"),
            "{center?, dx?, dy?, swatchConflict?} paste as edit.paste; pasted text takes the default character style (one run) and leaves its character and paragraph styles behind → {ids, added, merged, renamed}",
            has_clipboard,
            paste_plain
        ),
    ]
}

/// Text objects among `ids` and their descendants.
pub(crate) fn text_ids(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for id in ids {
        if let Some(n) = d.node(*id) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::Text(_)) && !out.contains(&c.id) {
                    out.push(c.id);
                }
            });
        }
    }
    out
}

fn texts(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let ids = targets(s, p)?;
    let t = text_ids(&s.doc()?.doc, &ids);
    if t.is_empty() {
        return Err(bad(cmd, "no text objects selected"));
    }
    Ok(t)
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

/// Rewrite every run of the given text objects with `f(run text, state)`. When no run changes, the
/// document and its undo history stay as they are.
fn edit_runs<S: Default>(s: &mut Session, label: &str, ids: &[NodeId], f: impl Fn(&str, &mut S) -> String) -> Result<usize> {
    super::overprint::edit_counted(s, label, |d| {
        let mut changed = 0;
        for id in ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let mut st = S::default();
            let mut any = false;
            // Inline graphics are art, not characters to rewrite.
            for r in t.runs.iter_mut().filter(|r| r.inline.is_none()) {
                let n = f(&r.text, &mut st);
                if n != r.text {
                    r.text = n;
                    any = true;
                }
            }
            if any {
                changed += 1;
                refresh_bounds(t);
            }
        }
        Ok(changed)
    })
}

// ---------- Change Case ----------

#[derive(Default)]
struct CaseState {
    /// Previous character (across runs).
    prev: Option<char>,
    /// Sentence start pending (sentence case).
    sentence: bool,
    started: bool,
}

fn change_case(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.changeCase";
    let case = str_param(p, "case").ok_or_else(|| bad(C, "missing case"))?.to_ascii_lowercase();
    if !["upper", "uppercase", "lower", "lowercase", "title", "sentence"].contains(&case.as_str()) {
        return Err(bad(C, "case must be upper|lower|title|sentence"));
    }
    let ids = texts(s, p, C)?;
    edit_runs::<CaseState>(s, "Change Case", &ids, |text, st| {
        if !st.started {
            st.started = true;
            st.sentence = true;
        }
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match case.as_str() {
                "upper" | "uppercase" => out.extend(c.to_uppercase()),
                "lower" | "lowercase" => out.extend(c.to_lowercase()),
                "title" => {
                    if st.prev.is_none_or(|p| p.is_whitespace() || matches!(p, '-' | '(' | '"' | '“' | '/')) {
                        out.extend(c.to_uppercase())
                    } else {
                        out.extend(c.to_lowercase())
                    }
                }
                _ => {
                    if c.is_alphabetic() && st.sentence {
                        out.extend(c.to_uppercase());
                        st.sentence = false;
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    if matches!(c, '.' | '!' | '?' | '\n') {
                        st.sentence = true;
                    }
                }
            }
            st.prev = Some(c);
        }
        out
    })?;
    Ok(ids_json(&ids))
}

// ---------- Smart Punctuation ----------

#[derive(Default)]
struct PunctState {
    prev: Option<char>,
}

/// Dashes (`--` en, `---` em), ellipses and, with `quotes`, typographer's quotes.
pub(crate) fn smarten(text: &str, prev: &mut Option<char>, quotes: Option<&Quotes>, dashes: bool, ellipsis: bool) -> String {
    let mut s = text.to_string();
    if dashes {
        s = s.replace("---", "—").replace("--", "–");
    }
    if ellipsis {
        s = s.replace("...", "…");
    }
    match quotes {
        Some(q) => q.apply(&s, prev),
        None => {
            *prev = s.chars().last().or(*prev);
            s
        }
    }
}

fn smart_punctuation(s: &mut Session, p: &Value) -> Result<Value> {
    let (d, e) = (bool_or(p, "dashes", true), bool_or(p, "ellipsis", true));
    let st = s.doc()?;
    // Quotes in the document's style (Document Setup → Type).
    let q = bool_or(p, "quotes", true).then_some(st.doc.setup.quotes);
    let ids = if str_param(p, "scope") == Some("document") || st.selection.is_empty() {
        let mut v = vec![];
        st.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                v.push(n.id)
            }
        });
        v
    } else {
        text_ids(&st.doc, &st.selection.objects)
    };
    let n = edit_runs::<PunctState>(s, "Smart Punctuation", &ids, |text, st| smarten(text, &mut st.prev, q.as_ref(), d, e))?;
    Ok(json!({ "changed": n }))
}

// ---------- Area / Point conversion ----------

fn to_area(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = texts(s, p, "type.convertToAreaType")?;
    // With no point type among the objects, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::Point))) {
        return Ok(ids_json(&ids));
    }
    s.edit("Convert To Area Type", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            if !matches!(t.kind, TextKind::Point) {
                continue;
            }
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let b = if lay.bounds.area() > 0.0 { lay.bounds } else { t.estimate_bounds() };
            let size = t.first_style().size;
            // A little slack on the right so the text doesn't rewrap.
            let top = lay.lines.first().map(|l| l.baseline - l.ascent).unwrap_or(b.y0).min(b.y0);
            let slack = size * 0.5;
            // Slack goes where the alignment leaves room, so the text doesn't move or rewrap.
            let (sl, sr) = match t.para_at(0).justify {
                Justify::Center | Justify::JustifyCenter => (slack * 0.5, slack * 0.5),
                Justify::Right | Justify::JustifyRight => (slack, 0.0),
                Justify::Auto if lay.lines.first().is_some_and(|l| l.rtl) => (slack, 0.0),
                _ => (0.0, slack),
            };
            let frame = Rect::new(b.x0 - sl, top, b.x1 + sr, b.y1.max(top + 1.0));
            t.kind = TextKind::Area { frame: shapes::rectangle(frame) };
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(ids_json(&ids))
}

/// (run, byte within the run) for a byte offset into the plain text.
fn locate(runs: &[TextRun], byte: usize) -> Option<(usize, usize)> {
    let mut off = 0;
    for (i, r) in runs.iter().enumerate() {
        if byte < off + r.text.len() {
            return Some((i, byte - off));
        }
        off += r.text.len();
    }
    None
}

fn to_point(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = texts(s, p, "type.convertToPointType")?;
    // With no area type among the objects, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::Area { .. }))) {
        return Ok(ids_json(&ids));
    }
    s.edit("Convert To Point Type", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let TextKind::Area { frame } = &t.kind else { continue };
            let fb = frame.bounds().unwrap_or_default();
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let plain = t.plain_text();
            let baseline = lay.lines.first().map(|l| l.baseline).unwrap_or(fb.y0 + t.first_style().size * 0.8);
            // Soft wraps → explicit breaks (replace the space before the wrap when there is one).
            let mut breaks: Vec<usize> =
                lay.lines.iter().skip(1).map(|l| l.start).filter(|&b| b > 0 && plain.as_bytes().get(b - 1) != Some(&b'\n')).collect();
            breaks.sort_unstable();
            breaks.dedup();
            for b in breaks.into_iter().rev() {
                // Each wrapped line becomes a paragraph with its paragraph's attributes.
                if plain.as_bytes().get(b - 1) == Some(&b' ') {
                    if let Some((ri, bi)) = locate(&t.runs, b - 1) {
                        t.splice_paras(b - 1, b, "\n");
                        t.runs[ri].text.replace_range(bi..bi + 1, "\n");
                    }
                } else if let Some((ri, bi)) = locate(&t.runs, b) {
                    t.splice_paras(b, b, "\n");
                    // Never inside an inline graphic's run: at the end of the run before it, or a
                    // run of its own.
                    match t.runs.get(ri) {
                        Some(r) if r.inline.is_some() => match ri.checked_sub(1).and_then(|p| t.runs.get_mut(p)) {
                            Some(prev) if prev.inline.is_none() => prev.text.push('\n'),
                            _ => {
                                let style = t.runs.get(ri).map(|r| r.style.clone()).unwrap_or_default();
                                t.runs.insert(ri, TextRun::new("\n", style));
                            }
                        },
                        _ => t.runs[ri].text.insert(bi, '\n'),
                    }
                }
            }
            // The point origin sits where the alignment anchors the first line.
            let p0 = t.para_at(0);
            let (x0, x1) =
                lay.lines.first().map_or((fb.x0, fb.x1), |l| (l.avail.0 - p0.left_indent - p0.first_line_indent, l.avail.1 + p0.right_indent));
            let ox = match p0.justify {
                Justify::Center | Justify::JustifyCenter => (x0 + x1) * 0.5,
                Justify::Right | Justify::JustifyRight => x1,
                Justify::Auto if lay.lines.first().is_some_and(|l| l.rtl) => x1,
                _ => x0,
            };
            t.kind = TextKind::Point;
            t.xf *= Affine::translate((ox, baseline));
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(ids_json(&ids))
}

/// The options `type.pathOptions` sets; with none of them given it queries.
const PATH_OPTIONS: [&str; 6] = ["start", "end", "flip", "effect", "alignToPath", "spacing"];

/// Type on a path's text, if `n` is type on a path.
fn path_text(n: Option<&vectorcraft_doc::Node>) -> Option<&TextObject> {
    match n.map(|n| &n.kind) {
        Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::OnPath { .. }) => Some(t),
        _ => None,
    }
}

/// Type on a Path Options of `t`, as `type.pathOptions` takes them (`flip` is an action: false).
fn path_options_of(t: &TextObject) -> Value {
    let (start, end) = match &t.kind {
        TextKind::OnPath { start, end, .. } => (*start, *end),
        _ => (0.0, None),
    };
    json!({
        "start": start,
        "end": end,
        "flip": false,
        "effect": t.path_effect.id(),
        "alignToPath": t.path_align.id(),
        "spacing": t.path_spacing,
    })
}

fn path_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.pathOptions";
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        texts(s, p, C)?.into_iter().filter(|i| path_text(d.node(*i)).is_some()).collect()
    };
    let first = *ids.first().ok_or_else(|| bad(C, "select type on a path"))?;
    let options = |s: &Session| -> Result<Value> {
        let t = path_text(s.doc()?.doc.node(first)).ok_or_else(|| bad(C, "select type on a path"))?;
        Ok(path_options_of(t))
    };
    if !PATH_OPTIONS.iter().any(|k| p.get(k).is_some()) {
        return options(s);
    }
    // A bracket: a fraction of the path's length.
    let fraction = |k: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|x| x.is_finite()).map(|x| Some(x.clamp(0.0, 1.0))).ok_or_else(|| bad(C, format!("`{k}` is a number 0..1"))),
        }
    };
    let start = fraction("start")?;
    // `end: null` puts the end bracket back at the end of the path.
    let end = p.get("end").map(|_| fraction("end")).transpose()?;
    let flip = bool_or(p, "flip", false);
    let effect = match str_param(p, "effect") {
        Some(e) => Some(vectorcraft_doc::PathEffect::parse(e).ok_or_else(|| bad(C, format!("unknown effect `{e}`")))?),
        None => None,
    };
    let align = match str_param(p, "alignToPath") {
        Some(a) => Some(vectorcraft_doc::PathAlign::parse(a).ok_or_else(|| bad(C, format!("unknown alignToPath `{a}`")))?),
        None => None,
    };
    let spacing = p.get("spacing").and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x.clamp(-1000.0, 1000.0));
    // Without a flip, when every object already has every value given, the document and its undo
    // history stay as they are.
    let has_values = |t: &TextObject| match &t.kind {
        TextKind::OnPath { start: st, end: en, .. } => {
            start.is_none_or(|v| v == *st)
                && end.is_none_or(|v| v == *en)
                && effect.is_none_or(|e| t.path_effect == e)
                && align.is_none_or(|a| t.path_align == a)
                && spacing.is_none_or(|v| t.path_spacing == v)
        }
        _ => true,
    };
    let d = &s.doc()?.doc;
    if !flip && ids.iter().all(|id| path_text(d.node(*id)).is_none_or(has_values)) {
        return options(s);
    }
    s.edit("Type on a Path Options", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            let TextKind::OnPath { start: st, end: en, .. } = &mut t.kind else { continue };
            if let Some(v) = start {
                *st = v;
            }
            if let Some(v) = end {
                *en = v;
            }
            // After the brackets: they are given on the path as it runs before the flip.
            if flip {
                t.flip_on_path();
            }
            if let Some(e) = effect {
                t.path_effect = e;
            }
            if let Some(a) = align {
                t.path_align = a;
            }
            if let Some(v) = spacing {
                t.path_spacing = v;
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    options(s)
}

// ---------- placeholder / insert ----------

/// VectorCraft's own placeholder copy (not the classical Latin text).
pub(crate) const PLACEHOLDER: &str = "Sample copy flows here while the layout takes shape. Swap these words for real text once the \
design is settled. Every line is only a stand-in that shows size, rhythm and colour. Headlines, captions and body text all \
start as rough drafts like this one. Keep going until the frame is full and the page feels balanced.";

fn fill_placeholder(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = texts(s, p, "type.fillPlaceholder")?;
    // When filling leaves every object as it was, the document and its undo history stay as they
    // are.
    super::overprint::edit_counted(s, "Fill with Placeholder Text", |d| {
        let mut changed = 0;
        for id in &ids {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                let before = t.clone();
                fill_with_placeholder(t);
                changed += usize::from(*t != before);
            }
        }
        Ok(changed)
    })?;
    Ok(ids_json(&ids))
}

/// Replace `t`'s text with placeholder text in its first style: area type is filled to its frame,
/// other type gets a sentence (Type › Fill with Placeholder Text, and new type with Preferences ›
/// Type › Fill New Type Objects With Placeholder Text).
pub(crate) fn fill_with_placeholder(t: &mut TextObject) {
    let style = t.first_style();
    let text = match &t.kind {
        TextKind::Area { frame } => {
            let b = frame.bounds().unwrap_or_default();
            let chars_per_line = (b.width() / (style.size * 0.5)).max(1.0);
            let lines = (b.height() / style.effective_leading()).max(1.0);
            let want = (chars_per_line * lines) as usize;
            let mut out = String::new();
            for (i, w) in PLACEHOLDER.split(' ').cycle().enumerate() {
                if out.len() >= want || i >= 10_000 {
                    break;
                }
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(w);
            }
            out
        }
        _ => PLACEHOLDER.split(". ").next().unwrap_or(PLACEHOLDER).to_string() + ".",
    };
    t.runs = vec![TextRun { text, style, inline: None }];
    refresh_bounds(t);
}

pub(crate) fn special_char(name: &str) -> Option<&'static str> {
    Some(match name {
        "bullet" => "•",
        "copyright" => "©",
        "ellipsis" => "…",
        "paragraph" => "¶",
        "registered" => "®",
        "section" => "§",
        "trademark" => "™",
        "emDash" => "—",
        "enDash" => "–",
        "discretionaryHyphen" => "\u{00AD}",
        "nonBreakingHyphen" => "\u{2011}",
        "doubleLeftQuote" => "“",
        "doubleRightQuote" => "”",
        "singleLeftQuote" => "‘",
        "singleRightQuote" => "’",
        "emSpace" => "\u{2003}",
        "enSpace" => "\u{2002}",
        "hairSpace" => "\u{200A}",
        "sixthSpace" => "\u{2006}",
        "thinSpace" => "\u{2009}",
        "nonBreakingSpace" => "\u{00A0}",
        "figureSpace" => "\u{2007}",
        "punctuationSpace" => "\u{2008}",
        "thirdSpace" => "\u{2004}",
        "quarterSpace" => "\u{2005}",
        "tab" => "\t",
        // Our text model breaks lines at paragraph returns only.
        "forcedLineBreak" | "paragraphReturn" => "\n",
        _ => return None,
    })
}

fn insert_char(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "type.insert";
    let text = match (str_param(p, "char"), str_param(p, "text")) {
        (Some(c), _) => special_char(c).ok_or_else(|| bad(C, format!("unknown character `{c}`")))?.to_string(),
        (None, Some(t)) => t.to_string(),
        _ => return Err(bad(C, "give `char` or `text`")),
    };
    let ids = texts(s, p, C)?;
    // When `text` is empty and every object ends with a run of text, the document and its undo
    // history stay as they are.
    let d = &s.doc()?.doc;
    let ends_with_text = |id: &NodeId| match d.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => t.runs.last().is_some_and(|r| r.inline.is_none()),
        _ => false,
    };
    if text.is_empty() && ids.iter().all(ends_with_text) {
        return Ok(ids_json(&ids));
    }
    s.edit("Typing", |d, _| {
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            match t.runs.last_mut() {
                Some(r) if r.inline.is_none() => r.text.push_str(&text),
                // After an inline graphic: a run of its own, in its style.
                Some(r) => {
                    let style = r.style.clone();
                    t.runs.push(TextRun::new(text.clone(), style));
                }
                None => t.runs.push(TextRun { text: text.clone(), style: CharStyle::default(), inline: None }),
            }
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(ids_json(&ids))
}

// ---------- Find and Replace ----------

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte ranges of `needle` in `hay`.
pub(crate) fn find_all(hay: &str, needle: &str, match_case: bool, whole_word: bool) -> Vec<(usize, usize)> {
    let nc: Vec<char> = needle.chars().collect();
    if nc.is_empty() {
        return vec![];
    }
    let hc: Vec<(usize, char)> = hay.char_indices().collect();
    let eq = |a: char, b: char| if match_case { a == b } else { a == b || a.to_lowercase().eq(b.to_lowercase()) };
    let mut out = vec![];
    let mut i = 0;
    while i + nc.len() <= hc.len() {
        if (0..nc.len()).all(|k| eq(hc[i + k].1, nc[k])) {
            let before = i.checked_sub(1).map(|j| hc[j].1);
            let after = hc.get(i + nc.len()).map(|x| x.1);
            if !whole_word || (!before.is_some_and(is_word) && !after.is_some_and(is_word)) {
                let start = hc[i].0;
                let end = hc.get(i + nc.len()).map(|x| x.0).unwrap_or(hay.len());
                out.push((start, end));
                i += nc.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

fn all_texts(d: &Document) -> Vec<NodeId> {
    let mut v = vec![];
    for l in d.layers.iter().filter(|l| l.visible && !l.locked) {
        l.walk(&mut |n| {
            if matches!(n.kind, NodeKind::Text(_)) && n.visible && !n.locked {
                v.push(n.id);
            }
        });
    }
    v
}

fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.findReplace";
    let find = str_param(p, "find").filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "missing `find`"))?.to_string();
    let rep = str_param(p, "replace").unwrap_or("").to_string();
    let (mc, ww) = (bool_or(p, "matchCase", false), bool_or(p, "wholeWord", false));
    let ids = match ids_param(p, "ids") {
        Some(v) => text_ids(&s.doc()?.doc, &v),
        None => all_texts(&s.doc()?.doc),
    };
    let st = s.doc()?;
    let total: usize = ids
        .iter()
        .filter_map(|id| match &st.doc.node(*id)?.kind {
            NodeKind::Text(t) => Some(t.runs.iter().map(|r| find_all(&r.text, &find, mc, ww).len()).sum::<usize>()),
            _ => None,
        })
        .sum();
    if total == 0 {
        return Ok(json!({ "count": 0 }));
    }
    super::overprint::edit_counted(s, "Replace", |d| {
        let mut changed = 0;
        for id in &ids {
            let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            // Matches as (run, start, end) with offsets inside the run. Inline graphics are art, not
            // characters to rewrite.
            let mut hits = vec![];
            let mut base = 0;
            for (ri, r) in t.runs.iter().enumerate() {
                if r.inline.is_none() {
                    for (a, b) in find_all(&r.text, &find, mc, ww) {
                        if r.text.get(a..b) != Some(rep.as_str()) {
                            hits.push((ri, base + a, base + b, a, b));
                        }
                    }
                }
                base += r.text.len();
            }
            if hits.is_empty() {
                continue;
            }
            // Last match first, so the offsets of earlier ones stay valid. The paragraph styles follow
            // each edit, as `text.editRange` does.
            for (ri, ta, tb, a, b) in hits.into_iter().rev() {
                t.splice_paras(ta, tb, &rep);
                if let Some(r) = t.runs.get_mut(ri) {
                    r.text.replace_range(a..b, &rep);
                }
            }
            changed += 1;
            refresh_bounds(t);
        }
        Ok(changed)
    })?;
    Ok(json!({ "count": total }))
}

fn find_next(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "edit.findNext";
    let find = str_param(p, "find").filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "missing `find`"))?.to_string();
    let (mc, ww) = (bool_or(p, "matchCase", false), bool_or(p, "wholeWord", false));
    let st = s.doc()?;
    let all = all_texts(&st.doc);
    let cur = st.selection.objects.first().and_then(|id| all.iter().position(|x| x == id));
    let n = all.len();
    let start = cur.map(|c| c + 1).unwrap_or(0);
    let hit = (0..n).map(|k| all[(start + k) % n.max(1)]).find(|id| match st.doc.node(*id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => !find_all(&t.plain_text(), &find, mc, ww).is_empty(),
        _ => false,
    });
    match hit {
        Some(id) => {
            s.select(|_, sel| sel.set([id]))?;
            Ok(json!({ "id": id.0 }))
        }
        None => Ok(json!({ "id": null })),
    }
}

// ---------- Paste without Formatting ----------

fn strip_formatting(n: &mut vectorcraft_doc::Node) {
    if let NodeKind::Text(t) = &mut n.kind {
        let text = t.plain_text();
        t.runs = vec![TextRun { text, style: CharStyle::default(), inline: None }];
        t.set_all_paras(Default::default());
        refresh_bounds(t);
    }
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            strip_formatting(std::sync::Arc::make_mut(c));
        }
    }
}

fn paste_plain(s: &mut Session, p: &Value) -> Result<Value> {
    let saved = s.clipboard.nodes.clone();
    for n in &mut s.clipboard.nodes {
        strip_formatting(n);
    }
    let paste = find_command("edit.paste").map(|c| c.run).ok_or_else(|| EngineError::Other("paste unavailable".into()))?;
    let r = paste(s, p);
    s.clipboard.nodes = saved;
    if r.is_ok()
        && let Some(e) = s.active_mut().and_then(|d| d.history.undo.last_mut())
    {
        e.label = "Paste without Formatting".into();
    }
    r
}
