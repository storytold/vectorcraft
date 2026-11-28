//! Edit › Spelling › Check Spelling…: walk through unknown words in the selection or the whole
//! document. The framework's `CheckSpellingDialog` shows the word in its sentence with suggestions
//! and files the buttons as `SpellingAction`s; this file is the walk behind it (`SpellingWalk`)
//! and the pump that runs each action against the document.
//!
//! The walk's state:
//! - `doc` and `reverts`: the drawing contents it started on; switching or reverting ends it
//! - `ids`: object IDs checked (None: entire document; Some(empty): nothing)
//! - `layer`: the one object checked while the dialog's Check All Layers is off
//! - `skipped`: `(node_id, start, word)` skipped by Ignore, which Change All leaves alone
//! - `queue`: the words the last check listed after the one shown, last first; `queue_more`
//!   whether that check had more, and `queue_from` (`(node_id, byte)`) where the next check goes
//!   on from
//! - `revision`: `[document revision, custom dictionary generation]` as of the word shown; when
//!   either has moved, the next step checks the scope again from its start
//! - `shown`: the word shown (None: the spell check has finished or found nothing)
//!
//! Ignore and Ignore All take the next word from `queue`, and a new check (`text.spelling.check`'s
//! `from`) goes on from `queue_from` once it is empty, through the rest of that type object and
//! then the objects after it, so every word is reached however many come before it. Change All
//! looks through the whole scope, one check after another, before it changes anything.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use craft::text::ui::spelling::{CheckSpellingDialog, SpellingAction, SpellingItem};
use serde_json::{Value, json};

use crate::VectorcraftApp;

/// The command its menu item runs.
pub const COMMAND: &str = "ui.checkSpelling";

/// An occurrence Ignore skipped: (id, start, word). The word is kept with its bytes, so after
/// another command moves the text, a different word that now starts there is not skipped.
type Skip = (u64, u64, String);

/// One unknown word: (id, start, end, word, lang).
type Word = (u64, usize, usize, String, String);

/// Where Check Spelling stands: what it checks, what it skipped and the word shown.
#[derive(Default)]
pub(crate) struct SpellingWalk {
    /// Uid of the document the walk started on.
    doc: u64,
    /// File Revert generation of the document the walk started on.
    reverts: u64,
    /// The selected objects; None is the whole document, which Some(empty) is not.
    ids: Option<Vec<u64>>,
    /// The one object checked while Check All Layers is off.
    layer: Option<u64>,
    /// The words the last check listed after the one shown, last first.
    queue: Vec<Word>,
    /// Where the next check goes on from when `queue` runs out: (id, byte).
    queue_from: Option<(u64, usize)>,
    /// Whether the check that filled `queue` had more words.
    queue_more: bool,
    /// The occurrences Ignore skipped.
    skipped: BTreeSet<Skip>,
    /// The document revision and dictionary generation as of the word shown.
    revision: Option<[u64; 2]>,
    /// The word shown; None when the check is complete.
    shown: Option<Word>,
}

/// Starts the walk on the active document's type and finds the first word.
fn start(app: &mut VectorcraftApp) -> Result<SpellingWalk, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let (doc, reverts, ids) = {
        let st = app.session.doc().map_err(|e| e.to_string())?;
        // The selection (groups too: the check goes through the type inside them), or, with
        // nothing selected, every visible, unlocked type object (the check's default).
        let selected: Vec<u64> = st.selection.objects.iter().map(|n| n.0).collect();
        (st.uid, st.reverts, (!selected.is_empty()).then_some(selected))
    };
    let mut w = SpellingWalk { doc, reverts, ids, ..Default::default() };
    advance(app, &mut w, None, 1)?;
    Ok(w)
}

/// Open Check Spelling on the active document's type.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut w = start(app)?;
    let dialogs = app.dialogs().ok_or("no window")?;
    let all_layers = dialogs.with(|host| host.get::<CheckSpellingDialog>().is_none_or(CheckSpellingDialog::all_layers_checked));
    if !all_layers {
        w.layer = w.shown.as_ref().map(|word| word.0);
        advance(app, &mut w, None, 1)?;
    }
    let items = items_of(app, &w)?;
    dialogs.with(|host| match host.get_mut::<CheckSpellingDialog>() {
        Some(d) => d.restart(items),
        None => {
            host.open(CheckSpellingDialog::new(items).all_layers(true));
        }
    });
    app.spelling = Some(w);
    Ok(Value::Null)
}

/// Runs the action the dialog filed, then hands it the word the walk shows now. Called every frame
/// by the dialog host; the host's lock is not held while the document is edited.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut w) = app.spelling.take() else { return };
    // PLAN: assumed `app.dialogs() -> Option<WindowDialogs>` is an owned handle, not a borrow of `app`
    let Some(dialogs) = app.dialogs() else {
        app.spelling = Some(w);
        return;
    };
    // Done, Escape and a close from outside all end the walk.
    if !dialogs.with(|host| host.is_open::<CheckSpellingDialog>()) {
        dialogs.with(|host| {
            let _ = host.take::<CheckSpellingDialog>();
        });
        return;
    }
    if check_opening_drawing(app, &w).is_err() {
        dialogs.with(|host| host.cancel::<CheckSpellingDialog>(ctx));
        return;
    }
    let filed = dialogs.with(|host| host.get_mut::<CheckSpellingDialog>().map(|d| (d.take_action(), d.all_layers_checked())));
    let Some((Some(action), all_layers)) = filed else {
        app.spelling = Some(w);
        return;
    };
    let res = match action {
        SpellingAction::Ignore(_) => ignore(app, &mut w),
        SpellingAction::Relist if all_layers != w.layer.is_none() => {
            w.layer = if all_layers { None } else { w.shown.as_ref().map(|s| s.0) };
            advance(app, &mut w, None, 1)
        }
        SpellingAction::Relist => ignore_all(app, &mut w),
        SpellingAction::Change { to, .. } => change(app, &mut w, &to),
        SpellingAction::ChangeAll { to, .. } => change_all(app, &mut w, &to),
        SpellingAction::Add(_) => add_to_dictionary(app, &mut w),
        _ => Ok(()),
    };
    if let Err(e) = res {
        app.status(&e);
    }
    let items = items_of(app, &w).unwrap_or_else(|e| {
        app.status(&e);
        Vec::new()
    });
    dialogs.with(|host| {
        if let Some(d) = host.get_mut::<CheckSpellingDialog>() {
            d.restart(items);
        }
    });
    app.spelling = Some(w);
}

/// Whether Ignore skipped this occurrence of a word.
fn is_skipped(skipped: &BTreeSet<Skip>, w: &Word) -> bool {
    skipped.contains(&(w.0, w.1 as u64, w.3.clone()))
}

/// Moves the Ignore positions in `id` that come after edits there by what each edit changed in
/// length: `edits` are the (start, end) bytes each replaced with `new_len` bytes.
fn shift_skipped(w: &mut SpellingWalk, id: u64, edits: &[(usize, usize)], new_len: usize) {
    w.skipped = std::mem::take(&mut w.skipped)
        .into_iter()
        .map(|(sid, start, word)| if sid == id { (sid, shifted(start as usize, edits, new_len) as u64, word) } else { (sid, start, word) })
        .collect();
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

/// One word of a `text.spelling.check` result.
fn word_at(w: &Value) -> Option<Word> {
    let id = w.get("id")?.as_u64()?;
    let start = usize::try_from(w.get("start")?.as_u64()?).ok()?;
    let end = usize::try_from(w.get("end")?.as_u64()?).ok()?;
    let word = w.get("word")?.as_str()?.to_string();
    let lang = w.get("lang").and_then(Value::as_str).unwrap_or("en").to_string();
    Some((id, start, end, word, lang))
}

/// The check's params for the walk's scope, going on from `from` ((id, byte)) when given. A
/// selected object that another command deleted is left out, so the rest of the selection is still
/// checked; with every one deleted, the list is empty and the check finds nothing (not the whole
/// document, which `ids` left out would check).
fn check_params(app: &VectorcraftApp, w: &SpellingWalk, from: Option<(u64, usize)>, max: usize) -> Value {
    let exists = |id: &u64| app.session.doc().is_ok_and(|st| st.doc.node(vectorcraft_doc::NodeId(*id)).is_some());
    let scope = w.layer.map(|layer| vec![layer]).or_else(|| w.ids.clone());
    let ids = match scope {
        Some(ids) => Value::Array(ids.iter().filter(|id| exists(id)).map(|&id| json!(id)).collect()),
        None => Value::Null,
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
fn advance(app: &mut VectorcraftApp, w: &mut SpellingWalk, mut from: Option<(u64, usize)>, mut max: usize) -> Result<(), String> {
    let skipped = w.skipped.clone();
    loop {
        let params = check_params(app, w, from, max);
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
        let mut rest = words.into_iter().filter(|word| !is_skipped(&skipped, word));
        if let Some(word) = rest.next() {
            w.queue = rest.collect();
            w.queue.reverse();
            w.queue_more = more;
            w.queue_from = next_from;
            return show(app, w, word);
        }
        match next_from {
            Some(f) if more => {
                from = Some(f);
                max = page();
            }
            _ => {
                w.shown = None;
                return Ok(());
            }
        }
    }
}

/// Shows the next word of `queue` that Ignore did not skip and Ignore All did not add, or goes on
/// with a new check from `queue_from` when the queue runs out. The queue holds what a check listed
/// at the document revision of the word shown, so callers use it only while [`resume_at`] gives
/// Some.
fn step(app: &mut VectorcraftApp, w: &mut SpellingWalk) -> Result<(), String> {
    while let Some(word) = w.queue.pop() {
        if is_skipped(&w.skipped, &word) || ignored_all(app, &word.3) {
            continue;
        }
        return show(app, w, word);
    }
    match w.queue_from {
        Some(from) if w.queue_more => advance(app, w, Some(from), page()),
        _ => {
            w.shown = None;
            Ok(())
        }
    }
}

/// Whether the active document's check skips `word` (Ignore All).
fn ignored_all(app: &VectorcraftApp, word: &str) -> bool {
    let Some(w) = vectorcraft_engine::cmd::spelling::custom_word(word) else { return false };
    app.session.doc().is_ok_and(|st| st.spelling_ignored.contains(&w))
}

/// Makes `word` the word shown.
fn show(app: &VectorcraftApp, w: &mut SpellingWalk, word: Word) -> Result<(), String> {
    w.shown = Some(word);
    remember_revision(app, w);
    Ok(())
}

/// The dialog's item for `word`: its sentence and suggestions. Suggestions are asked once per
/// word shown.
fn item(app: &mut VectorcraftApp, word: &Word) -> Result<SpellingItem, String> {
    let (id, start, end, text, lang) = word;
    let (before, _, after) = {
        let st = app.session.doc().map_err(|e| e.to_string())?;
        if let Some(vectorcraft_doc::NodeKind::Text(t)) = st.doc.node(vectorcraft_doc::NodeId(*id)).map(|n| &n.kind) {
            extract_sentence_context(&t.plain_text(), *start..*end)
        } else {
            (String::new(), String::new(), String::new())
        }
    };
    let sugg_res = app.run("text.spelling.suggest", json!({ "word": text, "lang": lang, "max": 10 })).map_err(|e| e.to_string())?;
    let suggestions: Vec<String> = sugg_res
        .get("suggestions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();
    Ok(SpellingItem::new(*id, *start..*end, text.clone(), suggestions).context(before, after))
}

/// The dialog's items for the walk: the word shown, or none when the check is complete.
fn items_of(app: &mut VectorcraftApp, w: &SpellingWalk) -> Result<Vec<SpellingItem>, String> {
    w.shown.as_ref().map(|word| item(app, word)).transpose().map(|i| i.into_iter().collect())
}

/// Notes the document's revision and the custom dictionary's generation as of the word shown,
/// which [`resume_at`] compares.
fn remember_revision(app: &VectorcraftApp, w: &mut SpellingWalk) {
    w.revision = revision(app);
}

/// The document's revision and the custom dictionary's generation: when either moves, the words
/// before the one shown may have changed.
fn revision(app: &VectorcraftApp) -> Option<[u64; 2]> {
    let doc = app.session.doc().map(|st| st.revision).ok()?;
    Some([doc, vectorcraft_engine::cmd::spelling::dictionary_generation()])
}

/// Where the next check goes on from: the start of the word shown. When another command changed
/// the document or the custom dictionary since the word was shown, the text before the word may
/// hold new unknown words, and `queue` may be out of date, so the check starts the scope again
/// (None); [`advance`] still skips the occurrences Ignore skipped.
fn resume_at(app: &VectorcraftApp, w: &SpellingWalk) -> Option<(u64, usize)> {
    let now = revision(app)?;
    if w.revision != Some(now) {
        return None;
    }
    let (id, start, _, true) = shown_word(app, w)? else { return None };
    Some((id, start))
}

/// The word shown, (id, start, end), and whether its object still holds it at those bytes; None
/// when there is none or its object is no longer type.
fn shown_word(app: &VectorcraftApp, w: &SpellingWalk) -> Option<(u64, usize, usize, bool)> {
    let (id, start, end, word, _) = w.shown.as_ref()?;
    let st = app.session.doc().ok()?;
    let Some(vectorcraft_doc::NodeKind::Text(t)) = st.doc.node(vectorcraft_doc::NodeId(*id)).map(|n| &n.kind) else { return None };
    let still_there = !word.is_empty() && t.plain_text().get(*start..*end).is_some_and(|text| text == word);
    Some((*id, *start, *end, still_there))
}

/// Fails when the active drawing's contents are not the ones the walk started on.
fn check_opening_drawing(app: &VectorcraftApp, w: &SpellingWalk) -> Result<(), String> {
    if !app.session.active().is_some_and(|st| st.uid == w.doc && st.reverts == w.reverts) {
        return Err("the document was switched or reverted".into());
    }
    Ok(())
}

/// The word shown, when the walk is still going and on the walk's document.
fn shown_text(app: &VectorcraftApp, w: &SpellingWalk) -> Result<Option<String>, String> {
    check_opening_drawing(app, w)?;
    Ok(w.shown.as_ref().map(|s| s.3.clone()))
}

fn ignore(app: &mut VectorcraftApp, w: &mut SpellingWalk) -> Result<(), String> {
    let Some(word) = shown_text(app, w)? else { return Ok(()) };
    let unchanged = resume_at(app, w).is_some();
    if let Some((id, start, _, true)) = shown_word(app, w) {
        w.skipped.insert((id, start as u64, word));
    }
    go_on(app, w, unchanged)
}

/// On past the word shown: the next word of the queue while nothing changed since it was shown,
/// otherwise the scope checked again from its start.
fn go_on(app: &mut VectorcraftApp, w: &mut SpellingWalk, unchanged: bool) -> Result<(), String> {
    if unchanged { step(app, w) } else { advance(app, w, None, page()) }
}

fn ignore_all(app: &mut VectorcraftApp, w: &mut SpellingWalk) -> Result<(), String> {
    let Some(word) = shown_text(app, w)? else { return Ok(()) };
    let unchanged = resume_at(app, w).is_some();
    if !word.is_empty() {
        app.run("text.spelling.ignoreAll", json!({ "word": word })).map_err(|e| e.to_string())?;
    }
    // The queue's other occurrences of the word are passed over too.
    go_on(app, w, unchanged)
}

fn change(app: &mut VectorcraftApp, w: &mut SpellingWalk, to: &str) -> Result<(), String> {
    let Some(word) = shown_text(app, w)? else { return Ok(()) };
    let from = resume_at(app, w);
    // Changed only while the word is still at the bytes shown.
    let Some((id, start, end, true)) = shown_word(app, w) else { return advance(app, w, None, page()) };
    if to.trim().is_empty() {
        return recheck(app, w, from);
    }
    if to == word {
        // Nothing to change, so no undo step: on past the word.
        return go_on(app, w, from.is_some());
    }
    app.run("text.editRange", json!({ "id": id, "start": start, "end": end, "insert": to })).map_err(|e| e.to_string())?;
    shift_skipped(w, id, &[(start, end)], to.len());
    // On from the replacement, which is shown when the dictionaries don't know it either (or
    // from the start of the scope, when the document changed under the dialog).
    recheck(app, w, from)
}

/// A new check after a change: on from `from` until the next unknown word, or the scope from its
/// start when `from` is None.
fn recheck(app: &mut VectorcraftApp, w: &mut SpellingWalk, from: Option<(u64, usize)>) -> Result<(), String> {
    match from {
        Some(from) => advance(app, w, Some(from), 1),
        None => advance(app, w, None, page()),
    }
}

fn change_all(app: &mut VectorcraftApp, w: &mut SpellingWalk, to: &str) -> Result<(), String> {
    let Some(word) = shown_text(app, w)? else { return Ok(()) };
    let here = resume_at(app, w);
    if word.is_empty() || to.trim().is_empty() {
        return recheck(app, w, here);
    }

    // Every occurrence in the scope, found before anything changes, so the bytes each check gives
    // stay true: one check after another, each going on from the end of the last word listed,
    // until none says `more`.
    let mut by_node: BTreeMap<u64, Vec<(usize, usize)>> = BTreeMap::new();
    let mut from = None;
    loop {
        let params = check_params(app, w, from, page());
        let check_res = app.run("text.spelling.check", params).map_err(|e| e.to_string())?;
        let words: Vec<Word> = check_res.get("words").and_then(Value::as_array).map(|a| a.iter().filter_map(word_at).collect()).unwrap_or_default();
        let more = check_res.get("more").and_then(Value::as_bool).unwrap_or(false);
        for found in &words {
            // An occurrence that already reads as the replacement is left alone: no edit, no undo step.
            if found.3.eq_ignore_ascii_case(&word) && found.3 != to && !is_skipped(&w.skipped, found) {
                by_node.entry(found.0).or_default().push((found.1, found.2));
            }
        }
        match words.last() {
            Some((id, _, end, _, _)) if more => from = Some((*id, *end)),
            _ => break,
        }
    }

    if by_node.is_empty() {
        // Nothing to change: on past the word shown, as Change does.
        return go_on(app, w, here.is_some());
    }
    // Every edit in one command: one undo step, each object laid out once.
    let words: Vec<Value> =
        by_node.iter().flat_map(|(&id, ranges)| ranges.iter().map(move |&(start, end)| json!({ "id": id, "start": start, "end": end }))).collect();
    app.run("text.spelling.change", json!({ "to": to, "words": words })).map_err(|e| e.to_string())?;
    for (&id, ranges) in &by_node {
        shift_skipped(w, id, ranges, to.len());
    }
    // On from the shown word's replacement, where it moved to.
    let from = here.map(|(id, start)| (id, by_node.get(&id).map_or(start, |ranges| shifted(start, ranges, to.len()))));
    recheck(app, w, from)
}

fn add_to_dictionary(app: &mut VectorcraftApp, w: &mut SpellingWalk) -> Result<(), String> {
    let Some(word) = shown_text(app, w)? else { return Ok(()) };
    // The check takes the word as right from now on, so it goes on from where the word starts.
    let from = resume_at(app, w);
    if !word.is_empty() {
        app.run("text.spelling.addWord", json!({ "word": word })).map_err(|e| e.to_string())?;
    }
    recheck(app, w, from)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::SpellingWalk;
    use crate::VectorcraftApp;

    const WORD_A: &str = "zqvspella";
    const WORD_B: &str = "zqvspellb";
    const WORD_C: &str = "zqvspellc";
    /// Never added to the custom dictionary, which the whole process shares (unlike WORD_C).
    const WORD_D: &str = "zqvspelld";

    thread_local! {
        /// The most words one Change All check lists in this test thread ([`super::page`]).
        pub(super) static PAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(vectorcraft_engine::cmd::spelling::MAX_WORDS) };
    }

    type Step = fn(&mut VectorcraftApp, &mut SpellingWalk) -> Result<(), String>;

    /// Ignore, Ignore All, Change and Change All (both to "fixed"), and Add to Dictionary.
    fn steps() -> [Step; 5] {
        [super::ignore, super::ignore_all, |a, w| super::change(a, w, "fixed"), |a, w| super::change_all(a, w, "fixed"), super::add_to_dictionary]
    }

    fn plain(app: &VectorcraftApp, id: u64) -> String {
        match &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        }
    }

    /// Starts the walk on the app, as `open` does without a window.
    fn begin(app: &mut VectorcraftApp) {
        app.spelling = Some(super::start(app).unwrap());
    }

    /// The word shown: (id, start, word).
    fn shown(app: &VectorcraftApp) -> (u64, u64, String) {
        let (id, start, _, word, _) = app.spelling.as_ref().unwrap().shown.clone().unwrap();
        (id, start as u64, word)
    }

    /// Whether the walk has no word to show.
    fn done(app: &VectorcraftApp) -> bool {
        app.spelling.as_ref().unwrap().shown.is_none()
    }

    fn act(app: &mut VectorcraftApp, f: impl FnOnce(&mut VectorcraftApp, &mut SpellingWalk) -> Result<(), String>) {
        let mut w = app.spelling.take().unwrap();
        f(app, &mut w).unwrap();
        app.spelling = Some(w);
    }

    #[test]
    fn ignore_goes_on_through_the_rest_of_the_same_type_object() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_B} one {WORD_B} two {WORD_B}"));
        let other = text(&mut app, 100.0, &format!("then {WORD_A}"));
        app.run("select.none", json!({})).unwrap();
        begin(&mut app);
        // Each step asks for one word on from the last, so every occurrence in the object is shown
        // in turn before the next object's.
        let mut seen = vec![shown(&app)];
        for _ in 0..4 {
            act(&mut app, super::ignore);
            if !done(&app) {
                seen.push(shown(&app));
            }
        }
        assert!(done(&app));
        let b = WORD_B.len() as u64;
        let want =
            vec![(id, 0, WORD_B.to_string()), (id, b + 5, WORD_B.to_string()), (id, 2 * b + 10, WORD_B.to_string()), (other, 5, WORD_A.to_string())];
        assert_eq!(seen, want);
    }

    #[test]
    fn ignore_and_change_all_reach_every_word_of_a_selected_groups_type() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let a = text(&mut app, 50.0, &format!("{WORD_B} {WORD_D} {WORD_B}"));
        let b = text(&mut app, 100.0, &format!("{WORD_D} {WORD_B}"));
        let outside = text(&mut app, 150.0, &format!("{WORD_B} outside"));
        app.run("select.set", json!({ "ids": [a, b] })).unwrap();
        let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({ "ids": [group] })).unwrap();
        PAGE.with(|p| p.set(1));

        begin(&mut app);
        let mut seen = vec![shown(&app)];
        for _ in 0..5 {
            act(&mut app, super::ignore);
            if !done(&app) {
                seen.push(shown(&app));
            }
        }
        assert!(done(&app));
        let ids: Vec<(u64, String)> = seen.into_iter().map(|(id, _, w)| (id, w)).collect();
        let want = [(a, WORD_B), (a, WORD_D), (a, WORD_B), (b, WORD_D), (b, WORD_B)].map(|(id, w)| (id, w.to_string()));
        assert_eq!(ids, want, "the group's descendants, one word at a time; not the object outside it");

        // Change All goes through the group one listed word at a time too.
        begin(&mut app);
        act(&mut app, |a, w| super::change_all(a, w, "fixed"));
        assert_eq!(plain(&app, a), format!("fixed {WORD_D} fixed"));
        assert_eq!(plain(&app, b), format!("{WORD_D} fixed"));
        assert_eq!(plain(&app, outside), format!("{WORD_B} outside"), "outside the selection");
    }

    #[test]
    fn change_all_reaches_a_target_past_a_full_check_of_other_words() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        // The second WORD_A comes after a whole check's worth of other unknown words.
        PAGE.with(|p| p.set(2));
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_D} {WORD_A} {WORD_B} {WORD_A}"));
        begin(&mut app);
        // Ignore the first WORD_A, then change all from the second.
        act(&mut app, super::ignore);
        act(&mut app, super::ignore);
        act(&mut app, super::ignore);
        assert_eq!(shown(&app).2, WORD_A);
        act(&mut app, |a, w| super::change_all(a, w, "fixed"));
        assert_eq!(plain(&app, id), format!("{WORD_A} {WORD_B} {WORD_D} fixed {WORD_B} fixed"), "the ignored one stays");
        // On from the replacement: the WORD_B after it.
        assert_eq!(shown(&app).2, WORD_B);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), format!("{WORD_A} {WORD_B} {WORD_D} {WORD_A} {WORD_B} {WORD_A}"), "one undo step");
    }

    #[test]
    fn change_all_reaches_a_target_past_the_checks_limit_in_one_object() {
        let _dictionary = crate::testing::custom_dictionary();
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        // A target, then the most other words one check lists, then a second target, all in one
        // type object.
        let others = vec![WORD_B; MAX_WORDS].join(" ");
        let id = text(&mut app, 50.0, &format!("{WORD_A} {others} {WORD_A}"));
        begin(&mut app);
        assert_eq!(shown(&app), (id, 0, WORD_A.to_string()));
        act(&mut app, |a, w| super::change_all(a, w, "fixed"));
        assert_eq!(plain(&app, id), format!("fixed {others} fixed"));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), format!("{WORD_A} {others} {WORD_A}"), "one undo step");
    }

    #[test]
    fn text_changed_under_the_dialog_is_checked_again_from_its_start() {
        let _dictionary = crate::testing::custom_dictionary();
        // A word of its own: Add to Dictionary puts it in the custom dictionary the process shares.
        const STALE: &str = "zqvstaleword";
        for f in steps() {
            let mut app = app();
            let id = text(&mut app, 50.0, &format!("hello {STALE}"));
            begin(&mut app);
            assert_eq!(shown(&app), (id, 6, STALE.to_string()));
            // Another command replaces the text with a shorter one: the shown bytes are gone.
            app.run("text.editRange", json!({ "id": id, "start": 0, "end": 6 + STALE.len(), "insert": WORD_D })).unwrap();
            act(&mut app, f);
            forget(&mut app, &[STALE]);
            assert_eq!(plain(&app, id), WORD_D, "nothing is changed at stale bytes");
            assert_eq!(shown(&app), (id, 0, WORD_D.to_string()), "the object is checked again from its start");
        }
    }

    #[test]
    fn a_restarted_check_still_skips_the_ignored_occurrences() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_A}"));
        begin(&mut app);
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 10, WORD_B.to_string()));
        // Another command changes the word shown, so the next step checks the object from its start.
        app.run("text.editRange", json!({ "id": id, "start": 10, "end": 10 + WORD_B.len(), "insert": WORD_D })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 10, WORD_D.to_string()), "the ignored first word is not shown again");
    }

    #[test]
    fn an_unknown_word_brought_back_before_the_shown_one_is_shown_again() {
        let _dictionary = crate::testing::custom_dictionary();
        // Words of their own: Add to Dictionary puts one in the custom dictionary the process shares.
        // FIRST has as many bytes as "different", so the second word stays at the bytes shown.
        const FIRST: &str = "zqvbckone";
        const SECOND: &str = "zqvbacktwo";
        for f in steps() {
            let mut app = app();
            let id = text(&mut app, 50.0, &format!("{FIRST} {SECOND}"));
            begin(&mut app);
            act(&mut app, |a, w| super::change(a, w, "different"));
            // The walk remembers the revision as of the word shown, so the next step goes on from
            // it rather than starting again.
            assert_eq!(app.spelling.as_ref().unwrap().revision.unwrap()[0], app.session.doc().unwrap().revision);
            assert_eq!(shown(&app), (id, 10, SECOND.to_string()));
            // Another command (Undo) brings the first word back before the one shown.
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, id), format!("{FIRST} {SECOND}"));
            act(&mut app, f);
            forget(&mut app, &[SECOND]);
            assert_eq!(shown(&app), (id, 0, FIRST.to_string()), "the check starts again and finds it");
        }
    }

    #[test]
    fn a_word_taken_out_of_the_custom_dictionary_is_shown_again() {
        let _dictionary = crate::testing::custom_dictionary();
        // Words of their own: the custom dictionary is shared by the whole process.
        const FIRST: &str = "zqvdictone";
        const SECOND: &str = "zqvdicttwo";
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{FIRST} {SECOND}"));
        begin(&mut app);
        act(&mut app, super::add_to_dictionary);
        assert_eq!(shown(&app), (id, 11, SECOND.to_string()));
        // Another command takes the first word out of the dictionary again.
        app.run("text.spelling.removeWord", json!({ "word": FIRST })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, 0, FIRST.to_string()), "the check starts again and finds it");
    }

    #[test]
    fn a_shown_object_that_leaves_the_scope_starts_the_scope_again() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let first = text(&mut app, 50.0, &format!("{WORD_A} here"));
        let second = text(&mut app, 100.0, &format!("{WORD_B} there"));
        app.run("select.none", json!({})).unwrap();
        begin(&mut app);
        assert_eq!(shown(&app).0, first);
        app.run("object.lock", json!({ "ids": [first] })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (second, 0, WORD_B.to_string()), "on to the type still in the scope");
    }

    #[test]
    fn a_selected_object_deleted_under_the_dialog_leaves_the_scope() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let first = text(&mut app, 50.0, &format!("{WORD_A} here"));
        let second = text(&mut app, 100.0, &format!("{WORD_B} there"));
        let outside = text(&mut app, 150.0, &format!("{WORD_C} elsewhere"));
        app.run("select.set", json!({ "ids": [first, second] })).unwrap();
        begin(&mut app);
        assert_eq!(shown(&app).0, first);
        app.run("edit.clear", json!({ "ids": [first] })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (second, 0, WORD_B.to_string()), "on to the selected type that is left");
        app.run("edit.clear", json!({ "ids": [second] })).unwrap();
        act(&mut app, super::ignore);
        assert!(done(&app), "nothing left in the selection: not the rest of the document ({outside})");
    }

    /// Marks the first `n` occurrences of WORD_B in `id` (each WORD_B and a space) as skipped by
    /// Ignore, with the last of them shown.
    fn ignored_b_words(app: &mut VectorcraftApp, id: u64, n: usize) {
        let step = WORD_B.len() + 1;
        let w = app.spelling.as_mut().unwrap();
        w.skipped = (0..n).map(|i| (id, (i * step) as u64, WORD_B.to_string())).collect();
        let last = (n - 1) * step;
        w.shown = Some((id, last, last + WORD_B.len(), WORD_B.to_string(), "en".to_string()));
    }

    #[test]
    fn ignore_reaches_the_last_of_more_unknown_words_than_one_check_lists() {
        let _dictionary = crate::testing::custom_dictionary();
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        let id = text(&mut app, 50.0, &vec![WORD_B; MAX_WORDS + 1].join(" "));
        begin(&mut app);
        ignored_b_words(&mut app, id, MAX_WORDS);
        // Another command changes the document, so the next step checks the scope from its start,
        // where every word one check lists was skipped.
        text(&mut app, 100.0, "hello");
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (id, (MAX_WORDS * (WORD_B.len() + 1)) as u64, WORD_B.to_string()), "the last occurrence");
        act(&mut app, super::ignore);
        assert!(done(&app));
    }

    #[test]
    fn ignore_and_change_all_reach_a_selected_groups_type_past_the_checks_limit() {
        let _dictionary = crate::testing::custom_dictionary();
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        let a = text(&mut app, 50.0, &vec![WORD_B; MAX_WORDS].join(" "));
        let b = text(&mut app, 100.0, &format!("{WORD_A} end"));
        let outside = text(&mut app, 150.0, &format!("{WORD_D} outside"));
        app.run("select.set", json!({ "ids": [a, b] })).unwrap();
        let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({ "ids": [group] })).unwrap();
        begin(&mut app);
        ignored_b_words(&mut app, a, MAX_WORDS);
        // An edit outside the group moves the document's revision: the group is checked again.
        app.run("text.editRange", json!({ "id": outside, "start": 0, "end": 0, "insert": "the " })).unwrap();
        act(&mut app, super::ignore);
        assert_eq!(shown(&app), (b, 0, WORD_A.to_string()), "the next object in the group");
        act(&mut app, super::ignore);
        assert!(done(&app), "not the object outside the group");

        // Change All: a target, then the most words one check lists, then a target in the next object.
        app.run("text.editRange", json!({ "id": a, "start": 0, "end": 0, "insert": format!("{WORD_A} ") })).unwrap();
        begin(&mut app);
        assert_eq!(shown(&app), (a, 0, WORD_A.to_string()));
        act(&mut app, |a, w| super::change_all(a, w, "fixed"));
        assert!(plain(&app, a).starts_with(&format!("fixed {WORD_B} ")));
        assert_eq!(plain(&app, b), "fixed end");
    }

    #[test]
    fn ignore_all_and_add_to_dictionary_go_on_past_each_check() {
        let _dictionary = crate::testing::custom_dictionary();
        PAGE.with(|p| p.set(2));
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_A} {WORD_D} {WORD_A}"));
        begin(&mut app);
        assert_eq!(shown(&app), (id, 0, WORD_A.to_string()));
        act(&mut app, super::ignore_all);
        let mut seen = vec![shown(&app)];
        act(&mut app, super::ignore);
        seen.push(shown(&app));
        act(&mut app, super::ignore);
        assert!(done(&app));
        assert_eq!(seen, vec![(id, 10, WORD_B.to_string()), (id, 30, WORD_D.to_string())], "no WORD_A after Ignore All");

        // Words of their own: the custom dictionary is shared by the whole process.
        const ADDED: &str = "zqvaddone";
        const OTHER: &str = "zqvaddtwo";
        PAGE.with(|p| p.set(1));
        let mut app = self::app();
        let id = text(&mut app, 50.0, &format!("{ADDED} {OTHER} {ADDED} {OTHER}"));
        begin(&mut app);
        act(&mut app, super::add_to_dictionary);
        let mut seen = vec![shown(&app)];
        act(&mut app, super::ignore);
        seen.push(shown(&app));
        act(&mut app, super::ignore);
        let finished = done(&app);
        forget(&mut app, &[ADDED]);
        assert!(finished);
        assert_eq!(seen, vec![(id, 10, OTHER.to_string()), (id, 30, OTHER.to_string())]);
    }

    #[test]
    fn change_to_the_same_word_changes_nothing_and_goes_on() {
        let _dictionary = crate::testing::custom_dictionary();
        type To = fn(&mut VectorcraftApp, &mut SpellingWalk, &str) -> Result<(), String>;
        for f in [super::change as To, super::change_all] {
            let mut app = app();
            let id = text(&mut app, 50.0, &format!("{WORD_D} and {WORD_D} {WORD_A}"));
            begin(&mut app);
            let depth = app.session.doc().unwrap().history.undo.len();
            act(&mut app, |a, w| f(a, w, WORD_D));
            assert_eq!(app.session.doc().unwrap().history.undo.len(), depth, "no undo step");
            assert_eq!(plain(&app, id), format!("{WORD_D} and {WORD_D} {WORD_A}"));
            let d = WORD_D.len() as u64;
            assert_eq!(shown(&app), (id, d + 5, WORD_D.to_string()), "on past the word shown");
        }
    }

    #[test]
    fn a_word_moved_by_another_command_is_not_taken_for_an_ignored_one() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("{WORD_A} {WORD_B} {WORD_A}"));
        begin(&mut app);
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
        let _dictionary = crate::testing::custom_dictionary();
        use vectorcraft_engine::cmd::spelling::MAX_WORDS;
        let mut app = app();
        let id = text(&mut app, 50.0, &vec![WORD_A; MAX_WORDS + 1].join(" "));
        begin(&mut app);
        let depth = app.session.doc().unwrap().history.undo.len();
        act(&mut app, |a, w| super::change_all(a, w, "fixed"));
        assert_eq!(plain(&app, id), vec!["fixed"; MAX_WORDS + 1].join(" "));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1, "one undo step");
        assert!(done(&app));
    }

    #[test]
    fn ignore_positions_move_with_the_edits_before_them() {
        // "ab cd ef": cd (3..5) replaced with "wxyz" moves byte 6 to 8; byte 0 stays.
        assert_eq!(super::shifted(6, &[(3, 5)], 4), 8);
        assert_eq!(super::shifted(0, &[(3, 5)], 4), 0);
        assert_eq!(super::shifted(9, &[(0, 2), (3, 5)], 1), 7);
    }

    #[test]
    fn a_check_with_all_layers_off_stays_in_the_shown_object() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let a = text(&mut app, 50.0, &format!("{WORD_A} x"));
        let b = text(&mut app, 100.0, &format!("{WORD_A} y"));
        app.run("select.none", json!({})).unwrap();
        begin(&mut app);
        assert_eq!(shown(&app).0, a);

        // Off: Ignore finds nothing in the other object.
        app.spelling.as_mut().unwrap().layer = Some(a);
        act(&mut app, super::ignore);
        assert!(done(&app), "the other object is out of scope");

        // Off: Change All reaches only the shown object.
        begin(&mut app);
        app.spelling.as_mut().unwrap().layer = Some(a);
        act(&mut app, |app, w| super::change_all(app, w, "fixed"));
        assert_eq!(plain(&app, a), "fixed x");
        assert_eq!(plain(&app, b), format!("{WORD_A} y"));

        // On again: the other object is reached.
        app.spelling.as_mut().unwrap().layer = None;
        act(&mut app, |app, w| super::advance(app, w, None, 1));
        assert_eq!(shown(&app), (b, 0, WORD_A.to_string()));
        act(&mut app, |app, w| super::change_all(app, w, "fixed"));
        assert_eq!(plain(&app, b), "fixed y");
    }

    #[test]
    fn reopening_keeps_the_unchecked_all_layers_scope_for_change_all() {
        use crate::tests::panel_harness::Panel;
        use craft::ui::dialog::StackDialog as _;

        fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.run(ui));
            }
            super::pump(app, ui.ctx());
        }

        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let first = text(&mut app, 50.0, WORD_A);
        app.run("layer.new", json!({"name": "Other text"})).unwrap();
        let second = text(&mut app, 100.0, WORD_A);
        app.run("select.none", json!({})).unwrap();
        let mut panel = Panel::new(app, draw);
        panel.app().run(super::COMMAND, json!({})).unwrap();
        panel.frame();
        let checked = shown(&panel.app()).0;
        let other = if checked == first { second } else { first };
        panel.click("Check All Layers");
        panel.app().run(super::COMMAND, json!({})).unwrap();
        panel.frame();
        {
            let app = panel.app();
            let ctx = panel.ctx();
            app.dialogs().unwrap().with(|host| {
                let dialog = host.get::<super::CheckSpellingDialog>().unwrap();
                assert!(!dialog.all_layers_checked(), "reopening preserves the chosen scope");
                let id = dialog.id();
                host.set_json_field(ctx, id, "changeTo", json!("fixed")).unwrap();
            });
        }
        panel.click("Change All");
        assert_eq!(plain(&panel.app(), checked), "fixed");
        assert_eq!(plain(&panel.app(), other), WORD_A, "the unchecked scope leaves the other layer's text alone");
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
    fn check_spelling_word_replacement_and_undo_redo() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let id = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));

        begin(&mut app);
        act(&mut app, |a, w| super::change(a, w, "replacement"));
        assert_eq!(plain(&app, id), "Prefix replacement suffix");

        // One undo step brings the original word back, and redo re-applies the replacement.
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), format!("Prefix {WORD_A} suffix"));
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(plain(&app, id), "Prefix replacement suffix");
    }

    #[test]
    fn check_spelling_change_all_in_one_undo_step() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let a = text(&mut app, 50.0, &format!("First {WORD_B} here and {WORD_B} again"));
        let b = text(&mut app, 100.0, &format!("Second {WORD_B} line"));
        app.run("select.none", json!({})).unwrap();

        begin(&mut app);
        act(&mut app, |app, w| super::change_all(app, w, "replaced"));
        assert_eq!(plain(&app, a), "First replaced here and replaced again");
        assert_eq!(plain(&app, b), "Second replaced line");

        // A single undo reverts everything Change All did.
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, a), format!("First {WORD_B} here and {WORD_B} again"));
        assert_eq!(plain(&app, b), format!("Second {WORD_B} line"));
    }

    #[test]
    fn check_spelling_dictionary_addition() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        forget(&mut app, &[WORD_C]);
        text(&mut app, 50.0, &format!("Look at {WORD_C} here"));

        begin(&mut app);
        assert_eq!(shown(&app).2, WORD_C);
        act(&mut app, super::add_to_dictionary);

        let dict = app.run("text.spelling.dictionary", json!({})).unwrap();
        let words = dict["words"].as_array().unwrap();
        assert!(words.contains(&json!(WORD_C)));
        // No other unknown word exists, so the walk is over.
        let finished = done(&app);
        forget(&mut app, &[WORD_C]);
        assert!(finished);
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
    fn check_spelling_refused_on_document_switch() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        text(&mut app, 50.0, &format!("Text {WORD_A} here"));
        begin(&mut app);

        // PLAN: the pump's close-on-switch needs the window host fixture of the hosted-dialog
        // tests; this drives the guard the pump calls (`check_opening_drawing`) through the walk's steps.
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let mut w = app.spelling.take().unwrap();
        assert!(super::check_opening_drawing(&app, &w).is_err());
        assert!(super::ignore(&mut app, &mut w).is_err(), "no step runs on another document");
    }

    #[test]
    fn queued_spelling_changes_leave_reverted_text_unchanged() {
        use crate::tests::panel_harness::Panel;
        use craft::ui::dialog::StackDialog as _;

        // The host files the real button's action; the test decides when the app consumes it.
        fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.run(ui));
            }
        }

        let _dictionary = crate::testing::custom_dictionary();
        for button in ["Change", "Change All"] {
            let mut app = app();
            let saved = format!("{WORD_A} saved");
            let id = text(&mut app, 50.0, &saved);
            app.run("select.none", json!({})).unwrap();
            app.services.read = Some(Box::new(|path: &str| std::fs::read(path).map_err(|error| error.to_string())));
            app.services.write = Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|error| error.to_string())));
            let dir = vectorcraft_testkit::TempDir::new("spelling-revert");
            app.run("file.saveAs", json!({"path": dir.join("Spelling.vectorcraft")})).unwrap();
            let state = app.session.active().unwrap();
            let (uid, reverts) = (state.uid, state.reverts);
            app.run("text.editRange", json!({"id": id, "start": saved.len(), "end": saved.len(), "insert": " draft"})).unwrap();
            let mut panel = Panel::new(app, draw);
            panel.app().run(super::COMMAND, json!({})).unwrap();
            panel.frame();
            let ctx = panel.ctx().clone();
            let set_replacement = |app: &VectorcraftApp| {
                app.dialogs().unwrap().with(|host| {
                    let dialog = host.get::<super::CheckSpellingDialog>().unwrap().id();
                    host.set_json_field(&ctx, dialog, "changeTo", json!("fixed")).unwrap();
                });
            };
            set_replacement(&panel.app());
            panel.click(button);
            assert_eq!(plain(&panel.app(), id), format!("{saved} draft"), "the action is still queued");
            panel.app().run("file.revert", json!({"confirmed": true})).unwrap();
            let undo = {
                let app = panel.app();
                let state = app.session.active().unwrap();
                assert_eq!((state.uid, state.reverts), (uid, reverts + 1));
                assert_eq!(plain(&app, id), saved, "the same word and local ID were reloaded from the native file");
                state.history.undo.len()
            };
            super::pump(&mut panel.app(), &ctx);
            assert_eq!(plain(&panel.app(), id), saved, "{button} from the old walk must not edit restored text");
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
            assert!(panel.app().spelling.is_none(), "the reverted walk is retired");
            assert!(!panel.app().dialogs().unwrap().with(|host| host.is_open::<super::CheckSpellingDialog>()));

            panel.app().run(super::COMMAND, json!({})).unwrap();
            panel.frame();
            set_replacement(&panel.app());
            panel.click(button);
            super::pump(&mut panel.app(), &ctx);
            assert_eq!(plain(&panel.app(), id), "fixed saved", "a fresh check can still change the restored text");
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
        }
    }

    #[test]
    fn check_spelling_ignores_uneditable_text() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let editable_id = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));
        let locked_id = text(&mut app, 100.0, &format!("Prefix {WORD_B} suffix"));

        // Lock the second text object
        app.run("select.set", json!({ "ids": [locked_id] })).unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.run("select.set", json!({ "ids": [] })).unwrap();

        begin(&mut app);
        assert_eq!(shown(&app).0, editable_id);
        assert_eq!(shown(&app).2, WORD_A);

        // The locked object's word is never shown.
        act(&mut app, super::ignore);
        assert!(done(&app), "locked type {locked_id} is left out");
    }

    #[test]
    fn check_spelling_change_all_containing_target_word_terminates() {
        let _dictionary = crate::testing::custom_dictionary();
        let mut app = app();
        let t1 = text(&mut app, 50.0, &format!("Prefix {WORD_A} suffix"));

        begin(&mut app);
        // Replace WORD_A with something containing WORD_A
        let replacement = format!("{WORD_A}abc");
        act(&mut app, |a, w| super::change_all(a, w, &replacement));
        // Change All replaced WORD_A; the walk moves to the new word
        assert_eq!(shown(&app).2, replacement);
        assert!(plain(&app, t1).contains(&replacement));
    }
}
