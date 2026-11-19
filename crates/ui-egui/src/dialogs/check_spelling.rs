//! Edit › Spelling › Check Spelling…: walk through unknown words in the selection or the whole
//! document, displaying the unrecognized word in its sentence context, with suggestions, and
//! Ignore, Ignore All, Change, Change All, Add to Dictionary, and Done / Cancel.
//!
//! Fields:
//! - `not_in_dictionary`: the current unrecognized word
//! - `context`: sentence context containing the unrecognized word
//! - `context_before`, `context_word`, `context_after`: parts of the sentence context
//! - `suggestions`: alternative words suggested by the dictionary
//! - `selected_suggestion`: the suggestion currently selected
//! - `change_to`: the replacement word (from suggestions or typed)
//! - `node_id`, `start`, `end`, `lang`: location of the current misspelling
//! - `ids`: object IDs checked (None: entire document)
//! - `skipped`: list of `[node_id, start, word]` skipped by Ignore, which Change All leaves alone
//! - `queue`: the words the last check listed after the one shown, last first; `queue_more`
//!   whether that check had more, and `queue_from` (`[node_id, byte]`) where the next check goes
//!   on from
//! - `revision`: `[document revision, custom dictionary generation]` as of the word shown; when
//!   either has moved, the next step checks the scope again from its start
//! - `done`: whether spell checking has finished (or no unrecognized words found)
//!
//! Ignore and Ignore All take the next word from `queue`, and a new check (`text.spelling.check`'s
//! `from`) goes on from `queue_from` once it is empty, through the rest of that type object and
//! then the objects after it, so every word is reached however many come before it. Change All
//! looks through the whole scope, one check after another, before it changes anything.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde_json::{Value, json};

use super::{DialogAction, DialogResult, DialogSpec};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Check Spelling.
pub const KIND: &str = "checkSpelling";

/// The command its menu item runs.
pub const COMMAND: &str = "ui.checkSpelling";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Check Spelling").into(),
    body_with_actions: Some(body),
    confirm,
    ok: None,
    min_width: 480.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Ignore,
    IgnoreAll,
    Change,
    ChangeAll,
    AddToDictionary,
    Done,
    Cancel,
}

/// Open Check Spelling on the active document's type.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let (doc_uid, ids) = {
        let st = app.session.doc().map_err(|e| e.to_string())?;
        // The selection (groups too: the check goes through the type inside them), or, with
        // nothing selected, every visible, unlocked type object (the check's default).
        let selected: Vec<u64> = st.selection.objects.iter().map(|n| n.0).collect();
        (st.uid, (!selected.is_empty()).then_some(selected))
    };
    let mut d = Dialog::new(
        KIND,
        json!({
            "doc_uid": doc_uid,
            "ids": ids,
            "skipped": [],
            "queue": [],
            "queue_more": false,
            "queue_from": null,
            "not_in_dictionary": "",
            "context": "",
            "context_before": "",
            "context_word": "",
            "context_after": "",
            "change_to": "",
            "suggestions": [],
            "selected_suggestion": null,
            "node_id": null,
            "start": null,
            "end": null,
            "lang": null,
            "done": false,
        }),
    );
    advance(app, &mut d, None, 1)?;
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// An occurrence Ignore skipped: (id, start, word). The word is kept with its bytes, so after
/// another command moves the text, a different word that now starts there is not skipped.
type Skip = (u64, u64, String);

fn get_skipped(d: &Dialog) -> BTreeSet<Skip> {
    d.fields
        .get("skipped")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| match s.as_array().map(Vec::as_slice) {
                    Some([id, start, word]) => Some((id.as_u64()?, start.as_u64()?, word.as_str()?.to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn set_skipped(d: &mut Dialog, skipped: &BTreeSet<Skip>) {
    let arr: Vec<Value> = skipped.iter().map(|(id, start, word)| json!([id, start, word])).collect();
    d.fields.insert("skipped".into(), json!(arr));
}

/// Whether Ignore skipped this occurrence of a word.
fn is_skipped(skipped: &BTreeSet<Skip>, w: &Word) -> bool {
    skipped.contains(&(w.0, w.1 as u64, w.3.clone()))
}

/// Moves the Ignore positions in `id` that come after edits there by what each edit changed in
/// length: `edits` are the (start, end) bytes each replaced with `new_len` bytes.
fn shift_skipped(d: &mut Dialog, id: u64, edits: &[(usize, usize)], new_len: usize) {
    let skipped = get_skipped(d)
        .into_iter()
        .map(|(sid, start, word)| if sid == id { (sid, shifted(start as usize, edits, new_len) as u64, word) } else { (sid, start, word) })
        .collect();
    set_skipped(d, &skipped);
}

/// Where byte `at` of a text ends up after `edits` ((start, end) bytes, none of them overlapping
/// `at`) are each replaced with `new_len` bytes.
fn shifted(at: usize, edits: &[(usize, usize)], new_len: usize) -> usize {
    edits.iter().filter(|(_, end)| *end <= at).fold(at, |at, (start, end)| (at + new_len).saturating_sub(end - start))
}

/// Most unknown words one check lists when the dialog reads on through the scope (Ignore once
/// the queue is empty, a check started again, Change All): the check's own limit, or a smaller
/// one in tests.
#[cfg(not(test))]
fn page() -> usize {
    vectorcraft_engine::cmd::spelling::MAX_WORDS
}

#[cfg(test)]
fn page() -> usize {
    tests::PAGE.with(std::cell::Cell::get)
}

/// Extracts the sentence containing `range` within `text`, returning (before, word, after).
pub fn extract_sentence_context(text: &str, range: Range<usize>) -> (String, String, String) {
    let empty = (String::new(), String::new(), String::new());
    if range.start > range.end {
        return empty;
    }
    let Some(word_slice) = text.get(range.clone()) else {
        return empty;
    };
    let word = word_slice.to_string();

    let Some(before_slice) = text.get(..range.start) else {
        return empty;
    };
    let nl_start = before_slice.rfind('\n').map(|idx| idx + 1);
    let mut punct_start = None;
    for (idx, ch) in before_slice.char_indices() {
        if (ch == '.' || ch == '!' || ch == '?') && before_slice.get(idx + ch.len_utf8()..).is_some_and(|s| s.starts_with(char::is_whitespace)) {
            punct_start = Some(idx + ch.len_utf8());
        }
    }
    let start_idx = match (nl_start, punct_start) {
        (Some(nl), Some(p)) => nl.max(p),
        (Some(nl), None) => nl,
        (None, Some(p)) => p,
        (None, None) => 0,
    };

    let Some(after_slice) = text.get(range.end..) else {
        return empty;
    };
    let nl_end = after_slice.find('\n');
    let mut punct_end = None;
    for (idx, ch) in after_slice.char_indices() {
        if ch == '.' || ch == '!' || ch == '?' {
            let next_idx = idx + ch.len_utf8();
            if next_idx >= after_slice.len() || after_slice.get(next_idx..).is_some_and(|s| s.starts_with(char::is_whitespace)) {
                punct_end = Some(next_idx);
                break;
            }
        }
    }
    let end_offset = match (nl_end, punct_end) {
        (Some(nl), Some(p)) => nl.min(p),
        (Some(nl), None) => nl,
        (None, Some(p)) => p,
        (None, None) => after_slice.len(),
    };

    let mut before = text.get(start_idx..range.start).unwrap_or("").trim_start().to_string();
    let mut after = after_slice.get(..end_offset).unwrap_or("").trim_end().to_string();

    const MAX_SIDE: usize = 120;
    if before.len() > MAX_SIDE {
        let target = before.len() - MAX_SIDE;
        let split_idx = before.char_indices().map(|(idx, _)| idx).find(|&idx| idx >= target).unwrap_or(before.len());
        let suffix = before.get(split_idx..).unwrap_or("");
        before = match suffix.find(' ') {
            Some(space) => format!("…{}", suffix.get(space..).unwrap_or("")),
            None => format!("…{suffix}"),
        };
    }
    if after.len() > MAX_SIDE {
        let split_idx = after.char_indices().map(|(idx, _)| idx).take_while(|&idx| idx <= MAX_SIDE).last().unwrap_or(0);
        let prefix = after.get(..split_idx).unwrap_or("");
        after = match prefix.rfind(' ') {
            Some(space) => format!("{}…", prefix.get(..space).unwrap_or("")),
            None => format!("{prefix}…"),
        };
    }

    (before, word, after)
}

fn set_done(d: &mut Dialog) {
    d.fields.insert("done".into(), json!(true));
    d.fields.insert("not_in_dictionary".into(), json!(""));
    d.fields.insert("context".into(), json!(tl!("The spell check is complete.")));
    d.fields.insert("context_before".into(), json!(""));
    d.fields.insert("context_word".into(), json!(""));
    d.fields.insert("context_after".into(), json!(""));
    d.fields.insert("suggestions".into(), json!([]));
    d.fields.insert("selected_suggestion".into(), Value::Null);
    d.fields.insert("change_to".into(), json!(""));
    d.fields.insert("node_id".into(), Value::Null);
    d.fields.insert("start".into(), Value::Null);
    d.fields.insert("end".into(), Value::Null);
    d.fields.insert("lang".into(), Value::Null);
}

/// One unknown word: (id, start, end, word, lang).
type Word = (u64, usize, usize, String, String);

/// One word of a `text.spelling.check` result.
fn word_at(w: &Value) -> Option<Word> {
    let id = w.get("id")?.as_u64()?;
    let start = usize::try_from(w.get("start")?.as_u64()?).ok()?;
    let end = usize::try_from(w.get("end")?.as_u64()?).ok()?;
    let word = w.get("word")?.as_str()?.to_string();
    let lang = w.get("lang").and_then(Value::as_str).unwrap_or("en").to_string();
    Some((id, start, end, word, lang))
}

/// One word of `queue`, kept as `[id, start, end, word, lang]`.
fn queued_word(w: &Value) -> Option<Word> {
    match w.as_array().map(Vec::as_slice) {
        Some([id, start, end, word, lang]) => Some((
            id.as_u64()?,
            usize::try_from(start.as_u64()?).ok()?,
            usize::try_from(end.as_u64()?).ok()?,
            word.as_str()?.to_string(),
            lang.as_str()?.to_string(),
        )),
        _ => None,
    }
}

/// The check's params for the dialog's scope, going on from `from` ((id, byte)) when given. A
/// selected object that another command deleted is left out, so the rest of the selection is still
/// checked; with every one deleted, the list is empty and the check finds nothing (not the whole
/// document, which `ids` left out would check).
fn check_params(app: &VectorcraftApp, d: &Dialog, from: Option<(u64, usize)>, max: usize) -> Value {
    let ids = match d.fields.get("ids") {
        Some(Value::Array(ids)) => {
            let exists =
                |id: &Value| id.as_u64().is_some_and(|id| app.session.doc().is_ok_and(|st| st.doc.node(vectorcraft_doc::NodeId(id)).is_some()));
            Value::Array(ids.iter().filter(|id| exists(id)).cloned().collect())
        }
        _ => Value::Null,
    };
    match from {
        Some((id, byte)) => json!({ "ids": ids, "from": { "id": id, "byte": byte }, "max": max }),
        None => json!({ "ids": ids, "max": max }),
    }
}

/// Checks from `from` ((id, byte); the start of the scope when None), listing at most `max`
/// words, and shows the first one Ignore did not skip, keeping the rest in `queue`; completes the
/// spell check when there is none. Past a check whose words were all skipped it lists [`page`]
/// words at a time.
pub fn advance(app: &mut VectorcraftApp, d: &mut Dialog, mut from: Option<(u64, usize)>, mut max: usize) -> Result<(), String> {
    let skipped = get_skipped(d);
    loop {
        let params = check_params(app, d, from, max);
        let check_res = match app.run("text.spelling.check", params) {
            Ok(r) => r,
            // The object `from` names left the scope (another command hid, locked or deleted
            // it): start the scope again, where every word before is now right or skipped.
            Err(_) if from.is_some() => {
                from = None;
                max = page();
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let words: Vec<Word> = check_res.get("words").and_then(Value::as_array).map(|a| a.iter().filter_map(word_at).collect()).unwrap_or_default();
        let more = check_res.get("more").and_then(Value::as_bool).unwrap_or(false);
        let next_from = words.last().map(|w| (w.0, w.2));
        let mut rest = words.into_iter().filter(|w| !is_skipped(&skipped, w));
        if let Some(w) = rest.next() {
            let mut queue: Vec<Value> = rest.map(|(id, start, end, word, lang)| json!([id, start, end, word, lang])).collect();
            queue.reverse();
            d.fields.insert("queue".into(), Value::Array(queue));
            d.fields.insert("queue_more".into(), json!(more));
            d.fields.insert("queue_from".into(), json!(next_from.map(|(id, byte)| [id, byte as u64])));
            return show(app, d, w);
        }
        match next_from {
            Some(f) if more => {
                from = Some(f);
                max = page();
            }
            _ => {
                set_done(d);
                return Ok(());
            }
        }
    }
}

/// Shows the next word of `queue` that Ignore did not skip and Ignore All did not add, or goes on
/// with a new check from `queue_from` when the queue runs out. The queue holds what a check listed
/// at the document revision of the word shown, so callers use it only while [`resume_at`] gives
/// Some.
fn step(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let skipped = get_skipped(d);
    loop {
        let next = d.fields.get_mut("queue").and_then(Value::as_array_mut).and_then(Vec::pop);
        let Some(next) = next else { break };
        let Some(w) = queued_word(&next) else { continue };
        if is_skipped(&skipped, &w) || ignored_all(app, &w.3) {
            continue;
        }
        return show(app, d, w);
    }
    let from = d.fields.get("queue_from").and_then(Value::as_array).and_then(|a| match a.as_slice() {
        [id, byte] => Some((id.as_u64()?, usize::try_from(byte.as_u64()?).ok()?)),
        _ => None,
    });
    match from {
        Some(from) if d.bool("queue_more") => advance(app, d, Some(from), page()),
        _ => {
            set_done(d);
            Ok(())
        }
    }
}

/// Whether the active document's check skips `word` (Ignore All).
fn ignored_all(app: &VectorcraftApp, word: &str) -> bool {
    let Some(w) = vectorcraft_engine::cmd::spelling::custom_word(word) else { return false };
    app.session.doc().is_ok_and(|st| st.spelling_ignored.contains(&w))
}

/// Shows `w` with its sentence and suggestions.
fn show(app: &mut VectorcraftApp, d: &mut Dialog, w: Word) -> Result<(), String> {
    let (id, start, end, word, lang) = w;
    let (before, target, after) = {
        let st = app.session.doc().map_err(|e| e.to_string())?;
        if let Some(vectorcraft_doc::NodeKind::Text(t)) = st.doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind) {
            extract_sentence_context(&t.plain_text(), start..end)
        } else {
            (String::new(), word.clone(), String::new())
        }
    };

    let sugg_res = app.run("text.spelling.suggest", json!({ "word": word, "lang": lang, "max": 10 })).map_err(|e| e.to_string())?;
    let suggestions: Vec<String> = sugg_res
        .get("suggestions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();

    let change_to = suggestions.first().cloned().unwrap_or_else(|| word.clone());
    let full_context = format!("{before}{target}{after}");

    d.fields.insert("not_in_dictionary".into(), json!(word));
    d.fields.insert("context".into(), json!(full_context));
    d.fields.insert("context_before".into(), json!(before));
    d.fields.insert("context_word".into(), json!(target));
    d.fields.insert("context_after".into(), json!(after));
    d.fields.insert("suggestions".into(), json!(suggestions));
    d.fields.insert("selected_suggestion".into(), json!(suggestions.first().cloned()));
    d.fields.insert("change_to".into(), json!(change_to));
    d.fields.insert("node_id".into(), json!(id));
    d.fields.insert("start".into(), json!(start));
    d.fields.insert("end".into(), json!(end));
    d.fields.insert("lang".into(), json!(lang));
    d.fields.insert("done".into(), json!(false));
    remember_revision(app, d);
    Ok(())
}

/// Notes the document's revision and the custom dictionary's generation as of the word shown,
/// which [`resume_at`] compares.
fn remember_revision(app: &VectorcraftApp, d: &mut Dialog) {
    d.fields.insert("revision".into(), json!(revision(app)));
}

/// The document's revision and the custom dictionary's generation: when either moves, the words
/// before the one shown may have changed.
fn revision(app: &VectorcraftApp) -> Option<[u64; 2]> {
    let doc = app.session.doc().map(|st| st.revision).ok()?;
    Some([doc, vectorcraft_engine::cmd::spelling::dictionary_generation()])
}

/// The word shown: (id, start, end).
fn current(d: &Dialog) -> Option<(u64, usize, usize)> {
    let id = d.fields.get("node_id").and_then(Value::as_u64)?;
    let start = usize::try_from(d.fields.get("start").and_then(Value::as_u64)?).ok()?;
    let end = usize::try_from(d.fields.get("end").and_then(Value::as_u64)?).ok()?;
    Some((id, start, end))
}

/// Where the next check goes on from: the start of the word shown. When another command changed
/// the document or the custom dictionary since the word was shown, the text before the word may
/// hold new unknown words, and `queue` may be out of date, so the check starts the scope again
/// (None); [`advance`] still skips the occurrences Ignore skipped.
fn resume_at(app: &VectorcraftApp, d: &Dialog) -> Option<(u64, usize)> {
    let now = revision(app)?;
    if d.fields.get("revision") != Some(&json!(now)) {
        return None;
    }
    let (id, start, _, true) = shown_word(app, d)? else { return None };
    Some((id, start))
}

/// The word shown, (id, start, end), and whether its object still holds it at those bytes; None
/// when there is none or its object is no longer type.
fn shown_word(app: &VectorcraftApp, d: &Dialog) -> Option<(u64, usize, usize, bool)> {
    let (id, start, end) = current(d)?;
    let st = app.session.doc().ok()?;
    let Some(vectorcraft_doc::NodeKind::Text(t)) = st.doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind) else { return None };
    let word = d.str("not_in_dictionary");
    let still_there = !word.is_empty() && t.plain_text().get(start..end).is_some_and(|w| w == word);
    Some((id, start, end, still_there))
}

fn check_doc_uid(app: &VectorcraftApp, d: &Dialog) -> Result<(), String> {
    let doc_uid = d.fields.get("doc_uid").and_then(Value::as_u64);
    let active_uid = app.session.active().map(|st| st.uid);
    if doc_uid.is_some() && doc_uid != active_uid {
        return Err("the document was switched".into());
    }
    Ok(())
}

fn ignore(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let unchanged = resume_at(app, d).is_some();
    if let Some((id, start, _, true)) = shown_word(app, d) {
        let mut skipped = get_skipped(d);
        skipped.insert((id, start as u64, d.str("not_in_dictionary")));
        set_skipped(d, &skipped);
    }
    go_on(app, d, unchanged)
}

/// On past the word shown: the next word of the queue while nothing changed since it was shown,
/// otherwise the scope checked again from its start.
fn go_on(app: &mut VectorcraftApp, d: &mut Dialog, unchanged: bool) -> Result<(), String> {
    if unchanged { step(app, d) } else { advance(app, d, None, page()) }
}

fn ignore_all(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let unchanged = resume_at(app, d).is_some();
    let word = d.str("not_in_dictionary");
    if !word.is_empty() {
        app.run("text.spelling.ignoreAll", json!({ "word": word })).map_err(|e| e.to_string())?;
    }
    // The queue's other occurrences of the word are passed over too.
    go_on(app, d, unchanged)
}

fn change(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let from = resume_at(app, d);
    let change_to = d.str("change_to");
    // Changed only while the word is still at the bytes shown.
    let Some((id, start, end, true)) = shown_word(app, d) else { return advance(app, d, None, page()) };
    if change_to.trim().is_empty() {
        return recheck(app, d, from);
    }
    if change_to == d.str("not_in_dictionary") {
        // Nothing to change, so no undo step: on past the word.
        return go_on(app, d, from.is_some());
    }
    app.run("text.editRange", json!({ "id": id, "start": start, "end": end, "insert": change_to })).map_err(|e| e.to_string())?;
    shift_skipped(d, id, &[(start, end)], change_to.len());
    // On from the replacement, which is shown when the dictionaries don't know it either (or
    // from the start of the scope, when the document changed under the dialog).
    recheck(app, d, from)
}

/// A new check after a change: on from `from` until the next unknown word, or the scope from its
/// start when `from` is None.
fn recheck(app: &mut VectorcraftApp, d: &mut Dialog, from: Option<(u64, usize)>) -> Result<(), String> {
    match from {
        Some(from) => advance(app, d, Some(from), 1),
        None => advance(app, d, None, page()),
    }
}

fn change_all(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    let word = d.str("not_in_dictionary");
    let change_to = d.str("change_to");
    let here = resume_at(app, d);
    if word.is_empty() || change_to.trim().is_empty() {
        return recheck(app, d, here);
    }
    let skipped = get_skipped(d);

    // Every occurrence in the scope, found before anything changes, so the bytes each check gives
    // stay true: one check after another, each going on from the end of the last word listed,
    // until none says `more`.
    let mut by_node: BTreeMap<u64, Vec<(usize, usize)>> = BTreeMap::new();
    let mut from = None;
    loop {
        let params = check_params(app, d, from, page());
        let check_res = app.run("text.spelling.check", params).map_err(|e| e.to_string())?;
        let words: Vec<Word> = check_res.get("words").and_then(Value::as_array).map(|a| a.iter().filter_map(word_at).collect()).unwrap_or_default();
        let more = check_res.get("more").and_then(Value::as_bool).unwrap_or(false);
        for w in &words {
            // An occurrence that already reads as the replacement is left alone: no edit, no undo step.
            if w.3.eq_ignore_ascii_case(&word) && w.3 != change_to && !is_skipped(&skipped, w) {
                by_node.entry(w.0).or_default().push((w.1, w.2));
            }
        }
        match words.last() {
            Some((id, _, end, _, _)) if more => from = Some((*id, *end)),
            _ => break,
        }
    }

    if by_node.is_empty() {
        // Nothing to change: on past the word shown, as Change does.
        return go_on(app, d, here.is_some());
    }
    // Every edit in one command: one undo step, each object laid out once.
    let words: Vec<Value> =
        by_node.iter().flat_map(|(&id, ranges)| ranges.iter().map(move |&(start, end)| json!({ "id": id, "start": start, "end": end }))).collect();
    app.run("text.spelling.change", json!({ "to": change_to, "words": words })).map_err(|e| e.to_string())?;
    for (&id, ranges) in &by_node {
        shift_skipped(d, id, ranges, change_to.len());
    }
    // On from the shown word's replacement, where it moved to.
    let from = here.map(|(id, start)| (id, by_node.get(&id).map_or(start, |ranges| shifted(start, ranges, change_to.len()))));
    recheck(app, d, from)
}

fn add_to_dictionary(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    check_doc_uid(app, d)?;
    if d.bool("done") {
        return Ok(());
    }
    // The check takes the word as right from now on, so it goes on from where the word starts.
    let from = resume_at(app, d);
    let word = d.str("not_in_dictionary");
    if !word.is_empty() {
        app.run("text.spelling.addWord", json!({ "word": word })).map_err(|e| e.to_string())?;
    }
    recheck(app, d, from)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let doc_uid = d.fields.get("doc_uid").and_then(Value::as_u64);
    let active_uid = app.session.active().map(|st| st.uid);
    if doc_uid.is_some() && doc_uid != active_uid {
        return DialogAction::Cancel;
    }

    if let Some(act) = d.fields.remove("action").and_then(|v| v.as_str().map(String::from)) {
        let res = match act.as_str() {
            "ignore" => ignore(app, d),
            "ignoreAll" => ignore_all(app, d),
            "change" => change(app, d),
            "changeAll" => change_all(app, d),
            "addToDictionary" => add_to_dictionary(app, d),
            "done" | "cancel" => return DialogAction::Cancel,
            _ => Ok(()),
        };
        if let Err(e) = res {
            app.status(&e);
            d.fields.insert("error".into(), json!(e));
        } else {
            d.fields.remove("error");
        }
    }

    let mut action_to_run = None;
    let done = d.bool("done");
    let before = d.str("context_before");
    let target = d.str("context_word");
    let after = d.str("context_after");
    let mut change_to = d.str("change_to");
    let suggestions: Vec<String> = d
        .fields
        .get("suggestions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();

    let can_change = !done && !change_to.trim().is_empty();

    if crate::shortcuts::pressed_anew(ui.ctx(), egui::Key::Enter)
        || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, .. })))
    {
        if can_change {
            action_to_run = Some(Action::Change);
        } else if done {
            return DialogAction::Cancel;
        }
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        return DialogAction::Cancel;
    }

    let action_col_width = 110.0;
    let col_spacing = 16.0;
    let text_col_width = (ui.available_width() - action_col_width - col_spacing).clamp(200.0, 320.0);

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(text_col_width);
            if done {
                ui.add_space(20.0);
                widgets::dim_label(ui, tl!("The spell check is complete."));
                ui.add_space(80.0);
            } else {
                widgets::dim_label(ui, tl!("Not in Dictionary:"));
                ui.group(|ui| {
                    ui.set_width(text_col_width);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if !before.is_empty() {
                            ui.label(&before);
                        }
                        let t = Tokens::get(ui.ctx());
                        ui.add(egui::Label::new(egui::RichText::new(&target).color(t.accent).strong().underline()));
                        if !after.is_empty() {
                            ui.label(&after);
                        }
                    });
                });

                ui.add_space(8.0);
                widgets::dim_label(ui, tl!("Change to:"));
                let field_id = ui.scope_id().with(("dlg-text", "change_to"));
                widgets::take_dialog_focus(ui, field_id, &change_to);
                if ui.add(egui::TextEdit::singleline(&mut change_to).id(field_id).desired_width(text_col_width)).changed() {
                    d.fields.insert("change_to".into(), json!(change_to));
                }

                ui.add_space(8.0);
                widgets::dim_label(ui, tl!("Suggestions:"));
                widgets::list_box(ui, |ui| {
                    ui.set_width(text_col_width);
                    let row = ui.spacing().interact_size.y;
                    egui::ScrollArea::vertical().id_salt("check-spelling-suggestions").min_scrolled_height(140.0).max_height(140.0).show_rows(
                        ui,
                        row,
                        suggestions.len(),
                        |ui, range| {
                            ui.set_width(ui.available_width());
                            for s in suggestions.get(range).unwrap_or_default() {
                                let is_selected = change_to == *s;
                                if ui.selectable_label(is_selected, s.as_str()).clicked() {
                                    d.fields.insert("change_to".into(), json!(s));
                                    d.fields.insert("selected_suggestion".into(), json!(s));
                                }
                            }
                        },
                    );
                });
            }
        });

        ui.add_space(col_spacing);

        ui.vertical(|ui| {
            let w = action_col_width;
            if ui.add_enabled_ui(!done, |ui| widgets::flat_button_fit(ui, tl!("Ignore"), w).clicked()).inner {
                action_to_run = Some(Action::Ignore);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(!done, |ui| widgets::flat_button_fit(ui, tl!("Ignore All"), w).clicked()).inner {
                action_to_run = Some(Action::IgnoreAll);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(can_change, |ui| widgets::flat_button_fit(ui, tl!("Change"), w).clicked()).inner {
                action_to_run = Some(Action::Change);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(can_change, |ui| widgets::flat_button_fit(ui, tl!("Change All"), w).clicked()).inner {
                action_to_run = Some(Action::ChangeAll);
            }
            ui.add_space(6.0);
            if ui.add_enabled_ui(!done, |ui| widgets::flat_button_fit(ui, tl!("Add to Dictionary"), w).clicked()).inner {
                action_to_run = Some(Action::AddToDictionary);
            }
            ui.add_space(16.0);
            if widgets::flat_button_fit(ui, tl!("Done"), w).clicked() {
                action_to_run = Some(Action::Done);
            }
            ui.add_space(6.0);
            if widgets::flat_button_fit(ui, tl!("Cancel"), w).clicked() {
                action_to_run = Some(Action::Cancel);
            }
        });
    });

    if let Some(act) = action_to_run {
        let res = match act {
            Action::Ignore => ignore(app, d),
            Action::IgnoreAll => ignore_all(app, d),
            Action::Change => change(app, d),
            Action::ChangeAll => change_all(app, d),
            Action::AddToDictionary => add_to_dictionary(app, d),
            Action::Done | Action::Cancel => return DialogAction::Cancel,
        };
        if let Err(e) = res {
            app.status(&e);
            d.fields.insert("error".into(), json!(e));
        } else {
            d.fields.remove("error");
        }
    }

    DialogAction::None
}

fn confirm(app: &mut VectorcraftApp, _: &Dialog) -> DialogResult {
    app.ui.dialog = None;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    use serde_json::json;

    use crate::VectorcraftApp;

    const WORD_A: &str = "zqvspella";
    const WORD_B: &str = "zqvspellb";
    const WORD_C: &str = "zqvspellc";
    /// Never added to the custom dictionary, which the whole process shares (unlike WORD_C).
    const WORD_D: &str = "zqvspelld";

    thread_local! {
        /// The most words one Change All check lists in this test thread ([`super::change_all_page`]).
        pub(super) static PAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(vectorcraft_engine::cmd::spelling::MAX_WORDS) };
    }

    fn plain(app: &VectorcraftApp, id: u64) -> String {
        match &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        }
    }

    /// The word the dialog shows: (id, start, word).
    fn shown(app: &VectorcraftApp) -> (u64, u64, String) {
        let d = app.ui.dialog.as_ref().unwrap();
        (d.fields["node_id"].as_u64().unwrap(), d.fields["start"].as_u64().unwrap(), d.str("not_in_dictionary"))
    }

    fn act(app: &mut VectorcraftApp, f: fn(&mut VectorcraftApp, &mut crate::state::Dialog) -> Result<(), String>) {
        let mut d = app.ui.dialog.take().unwrap();
        f(app, &mut d).unwrap();
        app.ui.dialog = Some(d);
    }

    #[test]
    fn ignore_goes_on_through_the_rest_of_the_same_type_object() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_B} one {WORD_B} two {WORD_B}"));
        let other = text(&mut app, 100.0, &format!("then {WORD_A}"));
        app.run("select.none", json!({})).unwrap();
        super::open(&mut app).unwrap();
        // Each step asks for one word on from the last, so every occurrence in the object is shown
        // in turn before the next object's.
        let mut seen = vec![shown(&app)];
        for _ in 0..4 {
            act(&mut app, super::ignore);
            if !app.ui.dialog.as_ref().unwrap().bool("done") {
                seen.push(shown(&app));
            }
        }
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
        let b = WORD_B.len() as u64;
        let want =
            vec![(id, 0, WORD_B.to_string()), (id, b + 5, WORD_B.to_string()), (id, 2 * b + 10, WORD_B.to_string()), (other, 5, WORD_A.to_string())];
        assert_eq!(seen, want);
    }

    #[test]
    fn ignore_and_change_all_reach_every_word_of_a_selected_groups_type() {
        let mut app = app();
        let a = text(&mut app, 50.0, &format!("{WORD_B} {WORD_D} {WORD_B}"));
        let b = text(&mut app, 100.0, &format!("{WORD_D} {WORD_B}"));
        let outside = text(&mut app, 150.0, &format!("{WORD_B} outside"));
        app.run("select.set", json!({ "ids": [a, b] })).unwrap();
        let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({ "ids": [group] })).unwrap();
        PAGE.with(|p| p.set(1));

        super::open(&mut app).unwrap();
        let mut seen = vec![shown(&app)];
        for _ in 0..5 {
            act(&mut app, super::ignore);
            if !app.ui.dialog.as_ref().unwrap().bool("done") {
                seen.push(shown(&app));
            }
        }
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
        let ids: Vec<(u64, String)> = seen.into_iter().map(|(id, _, w)| (id, w)).collect();
        let want = [(a, WORD_B), (a, WORD_D), (a, WORD_B), (b, WORD_D), (b, WORD_B)].map(|(id, w)| (id, w.to_string()));
        assert_eq!(ids, want, "the group's descendants, one word at a time; not the object outside it");

        // Change All goes through the group one listed word at a time too.
        super::open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
        act(&mut app, super::change_all);
        assert_eq!(plain(&app, a), format!("fixed {WORD_D} fixed"));
        assert_eq!(plain(&app, b), format!("{WORD_D} fixed"));
        assert_eq!(plain(&app, outside), format!("{WORD_B} outside"), "outside the selection");
    }

    #[test]
    fn change_all_reaches_a_target_past_a_full_check_of_other_words() {
        let mut app = app();
        // The second WORD_A comes after a whole check's worth of other unknown words.
        PAGE.with(|p| p.set(2));
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_D} {WORD_A} {WORD_B} {WORD_A}"));
        super::open(&mut app).unwrap();
        // Ignore the first WORD_A, then change all from the second.
        act(&mut app, super::ignore);
        act(&mut app, super::ignore);
        act(&mut app, super::ignore);
        assert_eq!(shown(&app).2, WORD_A);
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
        act(&mut app, super::change_all);
        assert_eq!(plain(&app, id), format!("{WORD_A} {WORD_B} {WORD_D} fixed {WORD_B} fixed"), "the ignored one stays");
        // On from the replacement: the WORD_B after it.
        assert_eq!(shown(&app).2, WORD_B);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), format!("{WORD_A} {WORD_B} {WORD_D} {WORD_A} {WORD_B} {WORD_A}"), "one undo step");
    }

    #[test]
    fn change_all_reaches_a_target_past_the_checks_limit_in_one_object() {
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        // A target, then the most other words one check lists, then a second target, all in one
        // type object.
        let others = vec![WORD_B; MAX_WORDS].join(" ");
        let id = text(&mut app, 50.0, &format!("{WORD_A} {others} {WORD_A}"));
        super::open(&mut app).unwrap();
        assert_eq!(shown(&app), (id, 0, WORD_A.to_string()));
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
        act(&mut app, super::change_all);
        assert_eq!(plain(&app, id), format!("fixed {others} fixed"));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), format!("{WORD_A} {others} {WORD_A}"), "one undo step");
    }

    #[test]
    fn text_changed_under_the_dialog_is_checked_again_from_its_start() {
        // A word of its own: Add to Dictionary puts it in the custom dictionary the process shares.
        const STALE: &str = "zqvstaleword";
        for f in [super::ignore, super::ignore_all, super::change, super::change_all, super::add_to_dictionary] {
            let mut app = app();
            let id = text(&mut app, 50.0, &format!("hello {STALE}"));
            super::open(&mut app).unwrap();
            assert_eq!(shown(&app), (id, 6, STALE.to_string()));
            // Another command replaces the text with a shorter one: the shown bytes are gone.
            app.run("text.editRange", json!({ "id": id, "start": 0, "end": 6 + STALE.len(), "insert": WORD_D })).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
            act(&mut app, f);
            forget(&mut app, &[STALE]);
            assert_eq!(plain(&app, id), WORD_D, "nothing is changed at stale bytes");
            assert_eq!(shown(&app), (id, 0, WORD_D.to_string()), "the object is checked again from its start");
        }
    }

    #[test]
    fn a_restarted_check_still_skips_the_ignored_occurrences() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_A}"));
        super::open(&mut app).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 10, WORD_B.to_string()));
        // Another command changes the word shown, so the next step checks the object from its start.
        app.run("text.editRange", json!({ "id": id, "start": 10, "end": 10 + WORD_B.len(), "insert": WORD_D })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 10, WORD_D.to_string()), "the ignored first word is not shown again");
    }

    #[test]
    fn an_unknown_word_brought_back_before_the_shown_one_is_shown_again() {
        // Words of their own: Add to Dictionary puts one in the custom dictionary the process shares.
        // FIRST has as many bytes as "different", so the second word stays at the bytes shown.
        const FIRST: &str = "zqvbckone";
        const SECOND: &str = "zqvbacktwo";
        for f in [super::ignore, super::ignore_all, super::change, super::change_all, super::add_to_dictionary] {
            let mut app = app();
            let id = text(&mut app, 50.0, &format!("{FIRST} {SECOND}"));
            super::open(&mut app).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("different"));
            act(&mut app, super::change);
            // Drawing the dialog leaves the document's revision alone, so the next step goes on
            // from the word shown rather than starting again.
            let ctx = context();
            frame(&mut app, &ctx, vec![]);
            frame(&mut app, &ctx, vec![]);
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!(d.fields["revision"][0], json!(app.session.doc().unwrap().revision));
            assert_eq!(shown(&app), (id, 10, SECOND.to_string()));
            // Another command (Undo) brings the first word back before the one shown.
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, id), format!("{FIRST} {SECOND}"));
            app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
            act(&mut app, f);
            forget(&mut app, &[SECOND]);
            assert_eq!(shown(&app), (id, 0, FIRST.to_string()), "the check starts again and finds it");
        }
    }

    #[test]
    fn a_word_taken_out_of_the_custom_dictionary_is_shown_again() {
        // Words of their own: the custom dictionary is shared by the whole process.
        const FIRST: &str = "zqvdictone";
        const SECOND: &str = "zqvdicttwo";
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{FIRST} {SECOND}"));
        super::open(&mut app).unwrap();
        act(&mut app, super::add_to_dictionary);
        assert_eq!(shown(&app), (id, 11, SECOND.to_string()));
        // Another command takes the first word out of the dictionary again.
        app.run("text.spelling.removeWord", json!({ "word": FIRST })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 0, FIRST.to_string()), "the check starts again and finds it");
    }

    #[test]
    fn a_shown_object_that_leaves_the_scope_starts_the_scope_again() {
        let mut app = app();
        let first = text(&mut app, 50.0, &format!("{WORD_A} here"));
        let second = text(&mut app, 100.0, &format!("{WORD_B} there"));
        app.run("select.none", json!({})).unwrap();
        super::open(&mut app).unwrap();
        assert_eq!(shown(&app).0, first);
        app.run("object.lock", json!({ "ids": [first] })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (second, 0, WORD_B.to_string()), "on to the type still in the scope");
    }

    #[test]
    fn a_selected_object_deleted_under_the_dialog_leaves_the_scope() {
        let mut app = app();
        let first = text(&mut app, 50.0, &format!("{WORD_A} here"));
        let second = text(&mut app, 100.0, &format!("{WORD_B} there"));
        let outside = text(&mut app, 150.0, &format!("{WORD_C} elsewhere"));
        app.run("select.set", json!({ "ids": [first, second] })).unwrap();
        super::open(&mut app).unwrap();
        assert_eq!(shown(&app).0, first);
        app.run("edit.clear", json!({ "ids": [first] })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (second, 0, WORD_B.to_string()), "on to the selected type that is left");
        app.run("edit.clear", json!({ "ids": [second] })).unwrap();
        act(&mut app, super::ignore);
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("done"), "nothing left in the selection: not the rest of the document ({outside})");
    }

    /// Marks the first `n` occurrences of WORD_B in `id` (each WORD_B and a space) as skipped by
    /// Ignore, with the last of them shown.
    fn ignored_b_words(app: &mut VectorcraftApp, id: u64, n: usize) {
        let step = WORD_B.len() + 1;
        let d = app.ui.dialog.as_mut().unwrap();
        let skipped: Vec<_> = (0..n).map(|i| json!([id, i * step, WORD_B])).collect();
        d.fields.insert("skipped".into(), json!(skipped));
        let last = (n - 1) * step;
        for (k, v) in [("node_id", json!(id)), ("start", json!(last)), ("end", json!(last + WORD_B.len())), ("not_in_dictionary", json!(WORD_B))] {
            d.fields.insert(k.into(), v);
        }
    }

    #[test]
    fn ignore_reaches_the_last_of_more_unknown_words_than_one_check_lists() {
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        let id = text(&mut app, 50.0, &vec![WORD_B; MAX_WORDS + 1].join(" "));
        super::open(&mut app).unwrap();
        ignored_b_words(&mut app, id, MAX_WORDS);
        // Another command changes the document, so the next step checks the scope from its start,
        // where every word one check lists was skipped.
        text(&mut app, 100.0, "hello");
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, (MAX_WORDS * (WORD_B.len() + 1)) as u64, WORD_B.to_string()), "the last occurrence");
        act(&mut app, super::ignore);
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
    }

    #[test]
    fn ignore_and_change_all_reach_a_selected_groups_type_past_the_checks_limit() {
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        let a = text(&mut app, 50.0, &vec![WORD_B; MAX_WORDS].join(" "));
        let b = text(&mut app, 100.0, &format!("{WORD_A} end"));
        let outside = text(&mut app, 150.0, &format!("{WORD_D} outside"));
        app.run("select.set", json!({ "ids": [a, b] })).unwrap();
        let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({ "ids": [group] })).unwrap();
        super::open(&mut app).unwrap();
        ignored_b_words(&mut app, a, MAX_WORDS);
        // An edit outside the group moves the document's revision: the group is checked again.
        app.run("text.editRange", json!({ "id": outside, "start": 0, "end": 0, "insert": "the " })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (b, 0, WORD_A.to_string()), "the next object in the group");
        act(&mut app, super::ignore);
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"), "not the object outside the group");

        // Change All: a target, then the most words one check lists, then a target in the next object.
        app.run("text.editRange", json!({ "id": a, "start": 0, "end": 0, "insert": format!("{WORD_A} ") })).unwrap();
        super::open(&mut app).unwrap();
        assert_eq!(shown(&app), (a, 0, WORD_A.to_string()));
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
        act(&mut app, super::change_all);
        assert!(plain(&app, a).starts_with(&format!("fixed {WORD_B} ")));
        assert_eq!(plain(&app, b), "fixed end");
    }

    #[test]
    fn ignore_all_and_add_to_dictionary_go_on_past_each_check() {
        PAGE.with(|p| p.set(2));
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_A} {WORD_D} {WORD_A}"));
        super::open(&mut app).unwrap();
        assert_eq!(shown(&app), (id, 0, WORD_A.to_string()));
        act(&mut app, super::ignore_all);
        let mut seen = vec![shown(&app)];
        act(&mut app, super::ignore);
        seen.push(shown(&app));
        act(&mut app, super::ignore);
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
        assert_eq!(seen, vec![(id, 10, WORD_B.to_string()), (id, 30, WORD_D.to_string())], "no WORD_A after Ignore All");

        // Words of their own: the custom dictionary is shared by the whole process.
        const ADDED: &str = "zqvaddone";
        const OTHER: &str = "zqvaddtwo";
        PAGE.with(|p| p.set(1));
        let mut app = super::tests::app();
        let id = text(&mut app, 50.0, &format!("{ADDED} {OTHER} {ADDED} {OTHER}"));
        super::open(&mut app).unwrap();
        act(&mut app, super::add_to_dictionary);
        let mut seen = vec![shown(&app)];
        act(&mut app, super::ignore);
        seen.push(shown(&app));
        act(&mut app, super::ignore);
        let done = app.ui.dialog.as_ref().unwrap().bool("done");
        forget(&mut app, &[ADDED]);
        assert!(done);
        assert_eq!(seen, vec![(id, 10, OTHER.to_string()), (id, 30, OTHER.to_string())]);
    }

    #[test]
    fn change_to_the_same_word_changes_nothing_and_goes_on() {
        for f in [super::change, super::change_all] {
            let mut app = app();
            let id = text(&mut app, 50.0, &format!("{WORD_D} and {WORD_D} {WORD_A}"));
            super::open(&mut app).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!(WORD_D));
            let depth = app.session.doc().unwrap().history.undo.len();
            act(&mut app, f);
            assert_eq!(app.session.doc().unwrap().history.undo.len(), depth, "no undo step");
            assert_eq!(plain(&app, id), format!("{WORD_D} and {WORD_D} {WORD_A}"));
            let d = WORD_D.len() as u64;
            assert_eq!(shown(&app), (id, d + 5, WORD_D.to_string()), "on past the word shown");
        }
    }

    #[test]
    fn a_word_moved_by_another_command_is_not_taken_for_an_ignored_one() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_A}"));
        super::open(&mut app).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 10, WORD_B.to_string()));
        // Another command puts a different unknown word where the ignored one was.
        let len = plain(&app, id).len();
        app.run("text.editRange", json!({ "id": id, "start": 0, "end": len, "insert": format!("{WORD_D} {WORD_A} {WORD_B}") })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 0, WORD_D.to_string()), "the word now at the ignored bytes is shown");
    }

    #[test]
    fn change_all_changes_a_whole_checks_worth_of_occurrences_in_one_edit() {
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        let id = text(&mut app, 50.0, &vec![WORD_A; MAX_WORDS + 1].join(" "));
        super::open(&mut app).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));
        let depth = app.session.doc().unwrap().history.undo.len();
        act(&mut app, super::change_all);
        assert_eq!(plain(&app, id), vec!["fixed"; MAX_WORDS + 1].join(" "));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1, "one undo step");
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
    }

    #[test]
    fn ignore_positions_move_with_the_edits_before_them() {
        // "ab cd ef": cd (3..5) replaced with "wxyz" moves byte 6 to 8; byte 0 stays.
        assert_eq!(super::shifted(6, &[(3, 5)], 4), 8);
        assert_eq!(super::shifted(0, &[(3, 5)], 4), 0);
        assert_eq!(super::shifted(9, &[(0, 2), (3, 5)], 1), 7);
    }

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app
    }

    fn text(app: &mut VectorcraftApp, y: f64, text: &str) -> u64 {
        app.run("text.create", json!({"x": 10, "y": y, "text": text})).unwrap()["id"].as_u64().unwrap()
    }

    fn forget(app: &mut VectorcraftApp, words: &[&str]) {
        for w in words {
            let _ = app.run("text.spelling.removeWord", json!({ "word": w }));
        }
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        let time = ctx.input(|i| i.time) + 0.25;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) -> Vec<(String, Rect)> {
        frame(app, ctx, vec![]);
        let texts = frame(app, ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, vec![Event::PointerMoved(at)]);
        frame(app, ctx, vec![press(true)]);
        frame(app, ctx, vec![press(false)]);
        frame(app, ctx, vec![])
    }

    fn key_press(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key) {
        let down = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, vec![down]);
        let up = Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, vec![up]);
    }

    #[test]
    fn test_extract_sentence_context() {
        let text = "First sentence. Here is teh sample typo! Third sentence.";
        let (before, word, after) = super::extract_sentence_context(text, 24..27);
        assert_eq!(word, "teh");
        assert_eq!(before, "Here is ");
        assert_eq!(after, " sample typo!");

        let single = "Hello world";
        let (b, w, a) = super::extract_sentence_context(single, 6..11);
        assert_eq!(w, "world");
        assert_eq!(b, "Hello ");
        assert_eq!(a, "");

        #[allow(clippy::reversed_empty_ranges)]
        let (b_inv, w_inv, a_inv) = super::extract_sentence_context(single, 10..5);
        assert!(w_inv.is_empty() && b_inv.is_empty() && a_inv.is_empty());

        // Multibyte UTF-8 regression test (e.g. 61 copies of 'é', which is 122 bytes)
        let after_multibyte = format!("word {}", "é".repeat(70));
        let (b_mb, w_mb, a_mb) = super::extract_sentence_context(&after_multibyte, 0..4);
        assert_eq!(w_mb, "word");
        assert_eq!(b_mb, "");
        assert!(a_mb.ends_with('…'));

        let before_multibyte = format!("{} typo end.", "é".repeat(70));
        let typo_start = "é".repeat(70).len() + 1;
        let typo_end = typo_start + 4;
        let (b_mb2, w_mb2, a_mb2) = super::extract_sentence_context(&before_multibyte, typo_start..typo_end);
        assert_eq!(w_mb2, "typo");
        assert!(b_mb2.starts_with('…'));
        assert_eq!(a_mb2, " end.");

        // Stale / out-of-bounds or non-char boundary offsets never panic
        let (b_oob, w_oob, a_oob) = super::extract_sentence_context(single, 5..50);
        assert!(w_oob.is_empty() && b_oob.is_empty() && a_oob.is_empty());

        let utf8_sample = "Héllo world";
        let (b_mid, w_mid, a_mid) = super::extract_sentence_context(utf8_sample, 2..4);
        assert!(w_mid.is_empty() && b_mid.is_empty() && a_mid.is_empty());
    }

    #[test]
    fn menu_opens_check_spelling_dialog() {
        let mut app = app();
        text(&mut app, 50.0, &format!("Hello {WORD_A} world"));

        let e = crate::menus::menu_entries(&app).into_iter().find(|e| e.label == "Check Spelling…").expect("in Edit menu");
        assert_eq!(e.command.as_deref(), Some(super::COMMAND));
        assert!(e.enabled);

        crate::menus::invoke(&mut app, super::COMMAND, json!({}));
        let d = app.ui.dialog.as_ref().expect("Check Spelling dialog opens");
        assert_eq!(d.kind, super::KIND);
        assert_eq!(d.str("not_in_dictionary"), WORD_A);
        assert!(d.str("context").contains(WORD_A));
        assert!(!d.bool("done"));
    }

    #[test]
    fn check_spelling_word_replacement_and_undo_redo() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));

        super::open(&mut app).unwrap();
        let ctx = context();

        // Change replacement word
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("replacement"));
        click(&mut app, &ctx, "Change");

        // Verify word replacement in the document
        let plain = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain, "Prefix replacement suffix");

        // Verify one-step undo brings the original word back
        app.run("edit.undo", json!({})).unwrap();
        let plain_undone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_undone, format!("Prefix {WORD_A} suffix"));

        // Verify redo re-applies the replacement
        app.run("edit.redo", json!({})).unwrap();
        let plain_redone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_redone, "Prefix replacement suffix");
    }

    #[test]
    fn check_spelling_change_all_in_one_undo_step() {
        let mut app = app();
        let a = text(&mut app, 50.0, &format!("First {WORD_B} here and {WORD_B} again"));
        let b = text(&mut app, 100.0, &format!("Second {WORD_B} line"));
        app.run("select.none", json!({})).unwrap();

        super::open(&mut app).unwrap();
        let ctx = context();

        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("replaced"));
        click(&mut app, &ctx, "Change All");

        let plain_a = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(a)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        let plain_b = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(b)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_a, "First replaced here and replaced again");
        assert_eq!(plain_b, "Second replaced line");

        // Single undo reverts all changes made by Change All
        app.run("edit.undo", json!({})).unwrap();
        let plain_a_undone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(a)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        let plain_b_undone = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(b)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain_a_undone, format!("First {WORD_B} here and {WORD_B} again"));
        assert_eq!(plain_b_undone, format!("Second {WORD_B} line"));
    }

    #[test]
    fn check_spelling_dictionary_addition() {
        let mut app = app();
        forget(&mut app, &[WORD_C]);
        text(&mut app, 50.0, &format!("Look at {WORD_C} here"));

        super::open(&mut app).unwrap();
        let ctx = context();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_C);

        click(&mut app, &ctx, "Add to Dictionary");

        // Verify WORD_C was added to user dictionary
        let dict = app.run("text.spelling.dictionary", json!({})).unwrap();
        let words = dict["words"].as_array().unwrap();
        assert!(words.contains(&json!(WORD_C)));

        // Dialog should now have completed because no other unknown words exist
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));

        forget(&mut app, &[WORD_C]);
    }

    #[test]
    fn check_spelling_ignore_and_ignore_all() {
        let mut app = app();
        text(&mut app, 50.0, &format!("{WORD_A} then {WORD_B}"));

        super::open(&mut app).unwrap();
        let ctx = context();

        // First word is WORD_A
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_A);

        // Clicking Ignore moves to WORD_B without modifying the document or ignore list
        click(&mut app, &ctx, "Ignore");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_B);
        assert!(!app.session.doc().unwrap().spelling_ignored.contains(WORD_A));

        // Clicking Ignore All adds WORD_B to the document's spelling_ignored list
        click(&mut app, &ctx, "Ignore All");
        assert!(app.session.doc().unwrap().spelling_ignored.contains(WORD_B));
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));
    }

    #[test]
    fn check_spelling_keyboard_navigation() {
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("Item {WORD_A} text"));

        super::open(&mut app).unwrap();
        let ctx = context();

        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!("fixed"));

        // Enter triggers Change
        key_press(&mut app, &ctx, egui::Key::Enter);

        let plain = {
            let st = app.session.doc().unwrap();
            match &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
                vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
                _ => panic!("not text"),
            }
        };
        assert_eq!(plain, "Item fixed text");
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"));

        // When done, Enter closes the dialog
        key_press(&mut app, &ctx, egui::Key::Enter);
        assert!(app.ui.dialog.is_none(), "Enter on done closes dialog");

        // Enter when change_to is empty and review is unfinished leaves dialog open
        text(&mut app, 100.0, &format!("More {WORD_A}"));
        super::open(&mut app).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("done"));
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!(""));
        key_press(&mut app, &ctx, egui::Key::Enter);
        assert!(app.ui.dialog.is_some(), "Enter with empty change_to leaves dialog open");

        // Test Escape closes dialog
        key_press(&mut app, &ctx, egui::Key::Escape);
        assert!(app.ui.dialog.is_none(), "Escape closes dialog");
    }

    #[test]
    fn check_spelling_selected_group_finds_nested_text() {
        let mut app = app();
        let t = text(&mut app, 50.0, &format!("Grouped {WORD_A} item"));
        app.run("select.set", json!({ "ids": [t] })).unwrap();
        let g = app.run("object.group", json!({})).unwrap();
        let group_id = g["id"].as_u64().unwrap();
        app.run("select.set", json!({ "ids": [group_id] })).unwrap();

        super::open(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().expect("dialog opens");
        assert_eq!(d.str("not_in_dictionary"), WORD_A);
    }

    #[test]
    fn check_spelling_when_no_misspellings() {
        let mut app = app();
        text(&mut app, 50.0, "All correctly spelled English words here");

        super::open(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().expect("dialog opens");
        assert!(d.bool("done"));
        assert!(d.str("context").contains("The spell check is complete."));

        let ctx = context();
        click(&mut app, &ctx, "Done");
        assert!(app.ui.dialog.is_none());
    }

    #[test]
    fn check_spelling_refused_on_document_switch() {
        let mut app = app();
        text(&mut app, 50.0, &format!("Text {WORD_A} here"));

        super::open(&mut app).unwrap();
        assert!(app.ui.dialog.is_some());

        // Create a new document, switching active document UID
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();

        let ctx = context();
        frame(&mut app, &ctx, vec![]);
        assert!(app.ui.dialog.is_none(), "switching documents closes spelling dialog");
    }

    #[test]
    fn test_extract_sentence_context_multiline() {
        let text = "First sentence.\nSecond line with teh typo.\nThird line ends.";
        let range = text.find("teh").map(|idx| idx..idx + 3).unwrap();
        let (before, word, after) = super::extract_sentence_context(text, range);
        assert_eq!(word, "teh");
        assert_eq!(before, "Second line with ");
        assert_eq!(after, " typo.");
        assert!(!before.contains("First"));
        assert!(!after.contains("Third"));

        let text2 = "Far before. Near before.\nLine with teh typo. Near after.\nFar after.";
        let range2 = text2.find("teh").map(|idx| idx..idx + 3).unwrap();
        let (b2, w2, a2) = super::extract_sentence_context(text2, range2);
        assert_eq!(w2, "teh");
        assert_eq!(b2, "Line with ");
        assert_eq!(a2, " typo.");
        assert!(!b2.contains("Near before"));
        assert!(!a2.contains("Near after"));
    }

    #[test]
    fn check_spelling_ignores_uneditable_text() {
        let mut app = app();
        let editable_id = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));
        let locked_id = text(&mut app, 100.0, &format!("Prefix {WORD_B} suffix"));

        // Lock the second text object
        app.run("select.set", json!({ "ids": [locked_id] })).unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.run("select.set", json!({ "ids": [] })).unwrap();

        super::open(&mut app).unwrap();
        let ctx = context();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), WORD_A);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["node_id"], json!(editable_id));

        // The locked object's word is never shown.
        click(&mut app, &ctx, "Ignore");
        assert!(app.ui.dialog.as_ref().unwrap().bool("done"), "locked type {locked_id} is left out");
    }

    #[test]
    fn check_spelling_change_all_containing_target_word_terminates() {
        let mut app = app();
        let t1 = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));

        super::open(&mut app).unwrap();
        assert!(app.ui.dialog.is_some());

        // Replace WORD_A with something containing WORD_A
        let ctx = context();
        let replacement = format!("{WORD_A}abc");
        app.ui.dialog.as_mut().unwrap().fields.insert("change_to".into(), json!(replacement));

        click(&mut app, &ctx, "Change All");
        // Change All successfully replaced WORD_A; advance moves to the new word
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("not_in_dictionary"), format!("{WORD_A}abc"));

        let st = app.session.doc().unwrap();
        let t1_text = match &st.doc.node(vectorcraft_doc::NodeId(t1)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        };
        assert!(t1_text.contains(&format!("{WORD_A}abc")));
    }
}
