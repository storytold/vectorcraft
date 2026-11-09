//! Edit → Spelling: the words of the document's type that its dictionaries don't know
//! (`text.spelling.check`) and the words they suggest instead (`text.spelling.suggest`), checked
//! by craft-proof in each run's language
//! ([Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)).
//! A word is changed with `text.editRange` over the bytes the check gives.

use std::ops::Range;

use craft_proof::{IssueKind, Options, ProofLang};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};

use super::menucmds::whole_or;
use super::typemenu::{all_texts, text_ids};
use super::{CommandSpec, Result, Session, bad, checked_ids_param, cmd, has_doc, str_param};

/// Most unknown words one check lists; past it the result says `more`.
pub const MAX_WORDS: usize = 10_000;

/// Most suggestions one word gets.
pub const MAX_SUGGESTIONS: u64 = 20;

/// Longest word suggestions are looked for, in bytes: the search costs more with the length.
const MAX_WORD_LEN: usize = 128;

/// The Edit › Spelling commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "text.spelling.check",
            "Check Spelling",
            [],
            None,
            "{ids?: [id…] (type objects, or groups holding them; default: every visible, unlocked type object)} the words the dictionaries don't know, each in its run's language (English when it has none; none in a language without a dictionary), each paragraph on its own, in paint order (given ids too) → {words: [{id, start: byte, end: byte, word, lang}…], more: bool (more than 10000 words, the rest left out)}. text.editRange {id, start, end, insert} changes one",
            has_doc,
            check
        ),
        cmd!(
            query "text.spelling.suggest",
            "Spelling Suggestions",
            [],
            None,
            "{word, max?: 1..20 (default 5), lang?: language tag (default English)} the words the dictionary of lang suggests instead of word, best first → {suggestions: [word…]}",
            has_doc,
            suggest
        ),
    ]
}

/// `text.spelling.check`: the unknown words of the given type objects (or every visible, unlocked
/// one), in paint order, each paragraph checked on its own, at most [`MAX_WORDS`] of them.
fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spelling.check";
    // Given `ids` are checked like Find and Replace's, so a malformed one is an error rather than
    // a check of the whole document.
    let ids = match p.get("ids").filter(|v| !v.is_null()) {
        Some(_) => in_paint_order(&s.doc()?.doc, text_ids(&s.doc()?.doc, &checked_ids_param(s, p, "ids", C)?)),
        None => all_texts(&s.doc()?.doc),
    };
    let st = s.doc()?;
    let opts = Options::default();
    let mut words = vec![];
    let mut more = false;
    'texts: for id in ids {
        let Some(NodeKind::Text(t)) = st.doc.node(id).map(|n| &n.kind) else { continue };
        let text = t.plain_text();
        // Each run in its own language, at its bytes of the plain text.
        let mut spans: Vec<(Range<usize>, ProofLang)> = Vec::with_capacity(t.runs.len());
        let mut at = 0;
        for r in &t.runs {
            let end = at + r.text.len();
            spans.push((at..end, ProofLang::from_tag(r.style.language.as_deref())));
            at = end;
        }
        // craft-proof checks one paragraph at a time (its language detection reads the
        // paragraph), and the check stops at the paragraph where the list fills up rather than
        // proofing the rest of the text.
        let mut para_start = 0;
        for para in text.split('\n') {
            let para_end = para_start + para.len();
            let para_spans: Vec<(Range<usize>, ProofLang)> = spans
                .iter()
                .filter_map(|(r, lang)| {
                    let (a, b) = (r.start.max(para_start), r.end.min(para_end));
                    (a < b).then(|| (a - para_start..b - para_start, *lang))
                })
                .collect();
            for issue in craft_proof::check_text_with(para, &para_spans, &opts, false, false) {
                if issue.kind != IssueKind::Spelling {
                    continue;
                }
                let (start, end) = (para_start + issue.start, para_start + issue.end);
                let Some(word) = text.get(start..end) else { continue };
                if words.len() == MAX_WORDS {
                    more = true;
                    break 'texts;
                }
                words.push(json!({ "id": id.0, "start": start, "end": end, "word": word, "lang": issue.lang.tag() }));
            }
            // Past the paragraph and its `\n`.
            para_start = para_end + 1;
        }
    }
    Ok(json!({ "words": words, "more": more }))
}

/// `ids` sorted back to front, as the document paints them: by their index paths, which are in
/// paint order.
fn in_paint_order(d: &Document, mut ids: Vec<NodeId>) -> Vec<NodeId> {
    let paths = d.index_paths(ids.iter().copied());
    ids.sort_by(|a, b| paths.get(a).cmp(&paths.get(b)));
    ids
}

/// `text.spelling.suggest`: up to `max` of the words the dictionary of `lang` suggests for `word`.
fn suggest(_s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spelling.suggest";
    let word = str_param(p, "word").map(str::trim).filter(|w| !w.is_empty()).ok_or_else(|| bad(C, "missing `word`"))?;
    let max = whole_or(p, "max", 5, C)?;
    if !(1..=MAX_SUGGESTIONS).contains(&max) {
        return Err(bad(C, format!("`max` must be 1 to {MAX_SUGGESTIONS}")));
    }
    if word.len() > MAX_WORD_LEN {
        return Ok(json!({ "suggestions": [] }));
    }
    let lang = ProofLang::from_tag(str_param(p, "lang"));
    // `max` is at most MAX_SUGGESTIONS, so it fits.
    let suggestions = craft_proof::suggest_in(word, usize::try_from(max).unwrap_or(5), lang);
    Ok(json!({ "suggestions": suggestions }))
}
