//! Edit → Spelling: the words of the document's type that its dictionaries don't know
//! (`text.spelling.check`) and the words they suggest instead (`text.spelling.suggest`), checked
//! by craft-proof in each run's language
//! ([Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)).
//! A word is changed with `text.editRange` over the bytes the check gives. Add and Edit Custom
//! Dictionary keep words in the custom dictionary (`text.spelling.dictionary`, `addWord`,
//! `removeWord`, `changeWord`), which every document's check takes as right; Ignore All
//! (`text.spelling.ignoreAll`) skips a word in the active document only.

use std::ops::Range;

use craft_proof::{IssueKind, Options, ProofLang};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};

use super::menucmds::whole_or;
use super::typemenu::{all_texts, text_ids};
use super::{CommandSpec, Result, Session, always, bad, checked_ids_param, cmd, has_doc, str_param};

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
        cmd!(
            query "text.spelling.dictionary",
            "Edit Custom Dictionary",
            [],
            None,
            "{} the words of the custom dictionary, sorted (lower case, as they are kept) → {words: [word…]}",
            always,
            dictionary
        ),
        cmd!(
            query "text.spelling.addWord",
            "Add to Custom Dictionary",
            [],
            None,
            "{word} adds word (one word, at most 64 characters; kept in lower case) to the custom dictionary, which every language's check takes as right, and saves it → {word, added: bool (false: it was there already)}",
            always,
            add_word
        ),
        cmd!(
            query "text.spelling.removeWord",
            "Delete from Custom Dictionary",
            [],
            None,
            "{word} deletes word from the custom dictionary and saves it (an error when it isn't there) → {word}",
            always,
            remove_word
        ),
        cmd!(
            query "text.spelling.changeWord",
            "Change in Custom Dictionary",
            [],
            None,
            "{word, to} replaces word in the custom dictionary with to and saves it (an error when word isn't there) → {word: to}",
            always,
            change_word
        ),
        cmd!(
            query "text.spelling.ignoreAll",
            "Ignore All",
            [],
            None,
            "{word} the active document's spelling checks skip word from now on, until it closes (not saved, not undoable) → {word}",
            has_doc,
            ignore_all
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
    let ignored = &st.spelling_ignored;
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
                if craft_proof::custom_word(word).is_some_and(|w| ignored.contains(&w)) {
                    continue;
                }
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

/// `p`'s `key` as a custom-dictionary word (trimmed, lower case), or an error naming what it must be.
fn word_param(p: &Value, key: &str, command: &str) -> Result<String> {
    let w = str_param(p, key).ok_or_else(|| bad(command, format!("missing `{key}`")))?;
    craft_proof::custom_word(w)
        .ok_or_else(|| bad(command, format!("`{key}` must be one word of at most {} characters", craft_proof::MAX_CUSTOM_WORD_CHARS)))
}

/// Largest custom dictionary file taken in, in bytes: craft-proof reads no more of one (its
/// `MAX_CUSTOM_FILE_BYTES`), so a larger file would lose its other words when written back.
pub const MAX_DICTIONARY_FILE_BYTES: u64 = 1_000_000;

/// Whether the custom dictionary file at `path` can be written over without losing words: it is
/// missing (a first save creates it), or a file that can be read whole. Otherwise why not.
fn dictionary_file_readable(path: &std::path::Path) -> std::result::Result<(), String> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
        Ok(m) if !m.is_file() => Err("it isn't a file".into()),
        Ok(m) if m.len() > MAX_DICTIONARY_FILE_BYTES => Err(format!("it is larger than {MAX_DICTIONARY_FILE_BYTES} bytes")),
        Ok(_) => std::fs::File::open(path).map(drop).map_err(|e| e.to_string()),
    }
}

/// Write the custom dictionary to its file when the session has one, leaving out `removed`. A file
/// that is there but can't be read whole is left alone rather than written over with only the
/// words read. On an error the caller takes its own change back, so memory and file agree.
fn save_dictionary(s: &Session, removed: &[String], command: &str) -> Result<()> {
    let Some(path) = &s.dictionary_file else { return Ok(()) };
    if let Err(why) = dictionary_file_readable(path) {
        return Err(bad(command, format!("the custom dictionary {} can't be read ({why}), so it is left as it is", path.display())));
    }
    craft_proof::save_user_dictionary(path, removed)
        .map_err(|e| bad(command, format!("the custom dictionary couldn't be saved to {}: {e}", path.display())))
}

/// `text.spelling.dictionary`: the custom dictionary's words.
fn dictionary(_s: &mut Session, _p: &Value) -> Result<Value> {
    Ok(json!({ "words": craft_proof::user_dictionary() }))
}

/// `text.spelling.addWord`: Add, and Edit Custom Dictionary's Add.
fn add_word(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spelling.addWord";
    let word = word_param(p, "word", C)?;
    let added = !craft_proof::user_dictionary().contains(&word);
    craft_proof::add_word(&word);
    save_dictionary(s, &[], C).inspect_err(|_| {
        // Only this command's word: words the save took in from the file stay.
        if added {
            craft_proof::remove_word(&word);
        }
    })?;
    Ok(json!({ "word": word, "added": added }))
}

/// `text.spelling.removeWord`: Edit Custom Dictionary's Delete.
fn remove_word(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spelling.removeWord";
    let word = word_param(p, "word", C)?;
    if !craft_proof::remove_word(&word) {
        return Err(bad(C, format!("`{word}` isn't in the custom dictionary")));
    }
    save_dictionary(s, std::slice::from_ref(&word), C).inspect_err(|_| craft_proof::add_word(&word))?;
    Ok(json!({ "word": word }))
}

/// `text.spelling.changeWord`: Edit Custom Dictionary's Change.
fn change_word(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spelling.changeWord";
    let word = word_param(p, "word", C)?;
    let to = word_param(p, "to", C)?;
    if !craft_proof::user_dictionary().contains(&word) {
        return Err(bad(C, format!("`{word}` isn't in the custom dictionary")));
    }
    if word == to {
        save_dictionary(s, &[], C)?;
        return Ok(json!({ "word": to }));
    }
    let to_is_new = !craft_proof::user_dictionary().contains(&to);
    craft_proof::remove_word(&word);
    craft_proof::add_word(&to);
    save_dictionary(s, std::slice::from_ref(&word), C).inspect_err(|_| {
        if to_is_new {
            craft_proof::remove_word(&to);
        }
        craft_proof::add_word(&word);
    })?;
    Ok(json!({ "word": to }))
}

/// `text.spelling.ignoreAll`: Ignore All, for the active document.
fn ignore_all(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.spelling.ignoreAll";
    let word = word_param(p, "word", C)?;
    // Not a change to the document, so its revision stays: a tool edit computed meanwhile still
    // applies (`command_sweep`'s check of commands that don't wait for one).
    s.doc_mut()?.spelling_ignored.insert(word.clone());
    Ok(json!({ "word": word }))
}

impl Session {
    /// Keep the custom dictionary in the file at `path` (the desktop app's, beside its
    /// preferences), and take in the words already there. A missing file reads as no words and is
    /// created by the first save. A file that is there but can't be read whole is logged and left
    /// as it is: changes to the dictionary are refused rather than written over it.
    pub fn attach_custom_dictionary(&mut self, path: std::path::PathBuf) {
        match dictionary_file_readable(&path) {
            Ok(()) => {
                craft_proof::load_user_dictionary(&path);
            }
            Err(why) => log::warn!("not reading the custom dictionary {}: {why}; it is left as it is", path.display()),
        }
        self.dictionary_file = Some(path);
    }
}
