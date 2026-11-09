//! Edit → Spelling (`text.spelling.check`, `text.spelling.suggest`): the unknown words of the
//! document's type, at the bytes `text.editRange` changes, each in its run's language.

use serde_json::{Value, json};

use crate::cmd::spelling::MAX_DICTIONARY_FILE_BYTES;
use crate::*;

/// A session with a new 400 × 400 document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

/// Runs a command that must succeed and gives its result.
fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

/// New point type at (10, `y`) holding `t`.
fn text(s: &mut Session, y: f64, t: &str) -> NodeId {
    NodeId(run(s, "text.create", json!({"x": 10, "y": y, "text": t}))["id"].as_u64().unwrap())
}

/// The unknown words a check lists: (id, start, end, word).
fn unknown(s: &mut Session, p: Value) -> Vec<(u64, u64, u64, String)> {
    let r = run(s, "text.spelling.check", p);
    assert_eq!(r["more"], false);
    r["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["id"].as_u64().unwrap(), w["start"].as_u64().unwrap(), w["end"].as_u64().unwrap(), w["word"].as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn check_lists_unknown_words_at_the_bytes_edit_range_changes() {
    let mut s = session();
    // "—" is three bytes, so the unknown word starts at byte 12, not character 10. Punctuation,
    // not a word, so the result doesn't depend on which dictionary is built in.
    let t = text(&mut s, 50.0, "drawing — qzxvw tools");
    assert_eq!(unknown(&mut s, json!({})), [(t.0, 12, 17, "qzxvw".to_string())]);
    let w = &run(&mut s, "text.spelling.check", json!({}))["words"][0];
    assert_eq!(w["lang"], "en", "text with no language is checked in English");
    // Change: the bytes the check gave, as one undo step.
    run(&mut s, "text.editRange", json!({"id": t.0, "start": 12, "end": 17, "insert": "vector"}));
    assert_eq!(unknown(&mut s, json!({})), []);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(unknown(&mut s, json!({})).len(), 1, "undo brings the word back");
}

#[test]
fn check_goes_through_visible_unlocked_type_in_paint_order_or_the_ids_given() {
    let mut s = session();
    let a = text(&mut s, 50.0, "first qzxvw");
    let b = text(&mut s, 100.0, "second wvxzq");
    let hidden = text(&mut s, 150.0, "hidden qqzzx");
    let locked = text(&mut s, 200.0, "locked zzqqx");
    run(&mut s, "object.hide", json!({"ids": [hidden.0]}));
    run(&mut s, "object.lock", json!({"ids": [locked.0]}));
    let ids: Vec<u64> = unknown(&mut s, json!({})).iter().map(|w| w.0).collect();
    assert_eq!(ids, [a.0, b.0], "hidden and locked type are left out");
    assert_eq!(unknown(&mut s, json!({"ids": [b.0]})), [(b.0, 7, 12, "wvxzq".to_string())]);
    assert_eq!(unknown(&mut s, json!({"ids": null})).len(), 2, "null ids is the default");
    let both = [(a.0, 6, 11, "qzxvw".to_string()), (b.0, 7, 12, "wvxzq".to_string())];
    assert_eq!(unknown(&mut s, json!({"ids": [b.0, a.0]})), both, "given ids are listed in paint order too");
    for ids in [json!(b.0), json!("all"), json!({"id": b.0}), json!([-1]), json!(["x"]), json!([999_999])] {
        assert!(s.execute("text.spelling.check", &json!({"ids": ids})).is_err(), "malformed ids {ids} is refused");
    }
}

#[test]
fn a_language_without_a_dictionary_marks_nothing() {
    let mut s = session();
    let t = text(&mut s, 50.0, "qzxvw");
    run(&mut s, "text.setFormat", json!({"ids": [t.0], "language": "German"}));
    assert_eq!(unknown(&mut s, json!({})), [], "German has no spelling dictionary in craft-proof");
}

#[test]
fn suggest_offers_dictionary_words() {
    let mut s = session();
    let r = run(&mut s, "text.spelling.suggest", json!({"word": "drawng"}));
    let got: Vec<&str> = r["suggestions"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert!(got.len() <= 5, "five at most by default: {got:?}");
    assert!(got.contains(&"drawing"), "{got:?}");
    assert_eq!(run(&mut s, "text.spelling.suggest", json!({"word": "drawng", "max": 1}))["suggestions"].as_array().unwrap().len(), 1);
    assert!(s.execute("text.spelling.suggest", &json!({})).is_err(), "a word is required");
    assert!(s.execute("text.spelling.suggest", &json!({"word": " "})).is_err(), "a blank word is refused");
    for max in [json!(0), json!(21), json!(-1), json!("3")] {
        assert!(s.execute("text.spelling.suggest", &json!({"word": "drawng", "max": max})).is_err(), "max {max}");
    }
}

#[test]
fn each_paragraph_is_checked_on_its_own_at_the_bytes_of_the_whole_text() {
    let mut s = session();
    // Read as one block, the Vietnamese first paragraph would make the whole text Vietnamese,
    // which has no dictionary, and the English typo below it would go unmarked.
    let vi = "Cảm ơn những người bạn";
    let t = text(&mut s, 50.0, &format!("{vi}\ndrawing qzxvw tools"));
    let at = vi.len() + 1 + "drawing ".len();
    assert_eq!(unknown(&mut s, json!({})), [(t.0, at as u64, at as u64 + 5, "qzxvw".to_string())]);
}

// The custom dictionary is craft-proof's, shared by the whole process, so each test below uses
// made-up words of its own: tests running at the same time never see each other's.

/// The words a check of the whole active document lists.
fn marked(s: &mut Session) -> Vec<String> {
    unknown(s, json!({})).into_iter().map(|w| w.3).collect()
}

#[test]
fn added_words_are_right_in_every_check_until_deleted() {
    let mut s = session();
    text(&mut s, 50.0, "drawing Zqvwkx tools");
    assert_eq!(marked(&mut s), ["Zqvwkx"]);
    // Kept in lower case, so the capitalized word is right too.
    let r = run(&mut s, "text.spelling.addWord", json!({"word": " Zqvwkx "}));
    assert_eq!((r["word"].as_str(), r["added"].as_bool()), (Some("zqvwkx"), Some(true)));
    assert_eq!(marked(&mut s), Vec::<String>::new());
    assert_eq!(run(&mut s, "text.spelling.addWord", json!({"word": "zqvwkx"}))["added"], false, "already there");
    let words = run(&mut s, "text.spelling.dictionary", json!({}))["words"].clone();
    assert!(words.as_array().unwrap().contains(&json!("zqvwkx")), "{words}");
    // Another document's check takes it as right too.
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    text(&mut s, 50.0, "zqvwkx");
    assert_eq!(marked(&mut s), Vec::<String>::new());
    run(&mut s, "text.spelling.removeWord", json!({"word": "ZQVWKX"}));
    assert_eq!(marked(&mut s), ["zqvwkx"]);
    let e = s.execute("text.spelling.removeWord", &json!({"word": "zqvwkx"})).unwrap_err().to_string();
    assert!(e.contains("isn't in the custom dictionary"), "{e}");
}

#[test]
fn a_changed_word_replaces_the_old_one() {
    let mut s = session();
    text(&mut s, 50.0, "xkvzqa xkvzqb");
    run(&mut s, "text.spelling.addWord", json!({"word": "xkvzqa"}));
    assert_eq!(marked(&mut s), ["xkvzqb"]);
    assert_eq!(run(&mut s, "text.spelling.changeWord", json!({"word": "xkvzqa", "to": "Xkvzqb"}))["word"], "xkvzqb");
    assert_eq!(marked(&mut s), ["xkvzqa"]);
    let e = s.execute("text.spelling.changeWord", &json!({"word": "xkvzqa", "to": "x"})).unwrap_err().to_string();
    assert!(e.contains("isn't in the custom dictionary"), "{e}");
    run(&mut s, "text.spelling.removeWord", json!({"word": "xkvzqb"}));
}

#[test]
fn the_custom_dictionary_file_is_read_and_kept_up_to_date() {
    let dir = vectorcraft_testkit::temp_dir("spelling-dictionary");
    let file = dir.join("Custom Dictionary.txt");
    std::fs::write(&file, "kqzvwx\nnot one word\n\n").unwrap();
    let mut s = session();
    s.attach_custom_dictionary(file.clone());
    text(&mut s, 50.0, "kqzvwx vkqzwx");
    assert_eq!(marked(&mut s), ["vkqzwx"], "the file's word is right; its line of three words is skipped");
    run(&mut s, "text.spelling.addWord", json!({"word": "vkqzwx"}));
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.lines().any(|l| l == "kqzvwx") && saved.lines().any(|l| l == "vkqzwx"), "{saved}");
    run(&mut s, "text.spelling.changeWord", json!({"word": "kqzvwx", "to": "kqzvwy"}));
    run(&mut s, "text.spelling.removeWord", json!({"word": "vkqzwx"}));
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.lines().any(|l| l == "kqzvwy"), "{saved}");
    assert!(!saved.lines().any(|l| l == "kqzvwx" || l == "vkqzwx"), "changed and deleted words are gone from the file: {saved}");
    run(&mut s, "text.spelling.removeWord", json!({"word": "kqzvwy"}));
}

#[test]
fn ignore_all_skips_a_word_in_the_active_document_only() {
    let mut s = session();
    text(&mut s, 50.0, "Wqzxkv drawing wqzxkv vqzxkw");
    run(&mut s, "text.spelling.ignoreAll", json!({"word": "WQZXKV"}));
    assert_eq!(marked(&mut s), ["vqzxkw"], "every occurrence, whatever its case");
    let words = run(&mut s, "text.spelling.dictionary", json!({}))["words"].clone();
    assert!(!words.as_array().unwrap().contains(&json!("wqzxkv")), "not added to the custom dictionary");
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    text(&mut s, 50.0, "wqzxkv");
    assert_eq!(marked(&mut s), ["wqzxkv"], "another document still marks it");
}

#[test]
fn custom_dictionary_commands_refuse_what_isnt_one_word() {
    let mut s = session();
    let long = "q".repeat(65);
    for (id, p) in [
        ("text.spelling.addWord", json!({})),
        ("text.spelling.addWord", json!({"word": "  "})),
        ("text.spelling.addWord", json!({"word": "two words"})),
        ("text.spelling.addWord", json!({"word": long})),
        ("text.spelling.addWord", json!({"word": 5})),
        ("text.spelling.removeWord", json!({})),
        ("text.spelling.changeWord", json!({"word": "qzx"})),
        ("text.spelling.changeWord", json!({"word": "qzx", "to": "a b"})),
        ("text.spelling.ignoreAll", json!({"word": "a\tb"})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p} is refused");
    }
    let mut empty = Session::new();
    assert!(empty.execute("text.spelling.ignoreAll", &json!({"word": "qzx"})).is_err(), "Ignore All needs a document");
    assert!(empty.execute("text.spelling.dictionary", &json!({})).is_ok(), "the dictionary is the app's");
}

/// Whether the custom dictionary holds `word`.
fn in_dictionary(s: &mut Session, word: &str) -> bool {
    run(s, "text.spelling.dictionary", json!({}))["words"].as_array().unwrap().contains(&json!(word))
}

#[test]
fn a_dictionary_file_that_cant_be_read_whole_is_never_written_over() {
    let dir = vectorcraft_testkit::temp_dir("spelling-unreadable");
    // A folder where the file should be can't be read as one; a file past what craft-proof reads
    // would lose its other words if written back.
    let folder = dir.join("Custom Dictionary.txt");
    std::fs::create_dir_all(&folder).unwrap();
    let big = dir.join("big.txt");
    let words = "qkzvxw\n".repeat((MAX_DICTIONARY_FILE_BYTES / 7 + 2) as usize);
    std::fs::write(&big, &words).unwrap();
    for path in [folder.clone(), big.clone()] {
        let mut s = session();
        s.attach_custom_dictionary(path.clone());
        let e = s.execute("text.spelling.addWord", &json!({"word": "zkqvxw"})).unwrap_err().to_string();
        assert!(e.contains("can't be read"), "{}: {e}", path.display());
        assert!(!in_dictionary(&mut s, "zkqvxw"), "the refused word isn't kept either");
    }
    assert!(folder.is_dir());
    assert_eq!(std::fs::read_to_string(&big).unwrap(), words, "the large file is as it was");
}

#[test]
fn a_change_that_cant_be_saved_is_taken_back() {
    let mut s = session();
    run(&mut s, "text.spelling.addWord", json!({"word": "wkqzvx"}));
    let dir = vectorcraft_testkit::temp_dir("spelling-rollback");
    let folder = dir.join("Custom Dictionary.txt");
    std::fs::create_dir_all(&folder).unwrap();
    let mut failing = session();
    failing.attach_custom_dictionary(folder);
    assert!(failing.execute("text.spelling.removeWord", &json!({"word": "wkqzvx"})).is_err());
    assert!(in_dictionary(&mut s, "wkqzvx"), "a delete that wasn't saved leaves the word");
    assert!(failing.execute("text.spelling.changeWord", &json!({"word": "wkqzvx", "to": "wkqzvy"})).is_err());
    assert!(in_dictionary(&mut s, "wkqzvx") && !in_dictionary(&mut s, "wkqzvy"), "a change that wasn't saved leaves the old word");
    run(&mut s, "text.spelling.removeWord", json!({"word": "wkqzvx"}));
}
