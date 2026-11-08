//! Edit → Spelling (`text.spelling.check`, `text.spelling.suggest`): the unknown words of the
//! document's type, at the bytes `text.editRange` changes, each in its run's language.

use serde_json::{Value, json};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

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
