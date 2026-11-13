use serde_json::{Value, json};
use vectorcraft_doc::{Kinsoku, NodeId, NodeKind};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.kinsoku_sets.iter().map(|k| k.name.clone()).collect()
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn text(s: &Session, id: u64) -> vectorcraft_doc::TextObject {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

/// Two paragraphs of area type, the second one selected by its range.
fn type_object(s: &mut Session) -> u64 {
    s.execute("text.create", &json!({"x": 10, "y": 10, "text": "あいう\nかきく", "area": {"width": 100, "height": 100}})).unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn tight() -> Value {
    json!({"name": "Tight", "cannotStart": "」。、う", "cannotEnd": "「", "hanging": "、。", "inseparable": ["——"]})
}

/// Kinsoku Shori Settings' OK replaces the document's custom sets as one undo step (none when
/// nothing changes); each category keeps each character once, sorted; the sets are listed with
/// their users, saved with the document and undone with it.
#[test]
fn kinsoku_sets_are_set_as_one_undo_step_and_saved() {
    let mut s = session();
    let before = steps(&s);
    let r = s.execute("type.kinsokuSets.set", &json!({"sets": [tight(), {"name": " Loose ", "cannotStart": "ーーッ"}]})).unwrap();
    assert_eq!(r["sets"][1]["name"], "Loose");
    assert_eq!(r["sets"][1]["cannotStart"], "ッー");
    assert_eq!(names(&s), ["Tight", "Loose"]);
    assert_eq!(steps(&s), before + 1);
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight(), {"name": "Loose", "cannotStart": "ッー"}]})).unwrap();
    assert_eq!(steps(&s), before + 1, "the same sets again change nothing");
    let listed = s.execute("type.kinsokuSets.list", &json!({})).unwrap();
    assert_eq!((listed["sets"][0]["name"].clone(), listed["sets"][0]["paragraphs"].clone()), (json!("Tight"), json!(0)));
    let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(*reopened.kinsoku_sets, *s.doc().unwrap().doc.kinsoku_sets);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(names(&s).is_empty());
    // A document without sets saves none.
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    assert!(!String::from_utf8_lossy(&saved).contains("kinsoku_sets"));
}

/// The sets are checked before anything changes: a built-in name, a blank or too-long name, two
/// sets of one name in any case, an unknown key, a sequence of one character or with a space, a
/// control character, a `from` the document has no set of, and more than 256 sets all fail.
#[test]
fn kinsoku_sets_refuse_what_a_set_cannot_hold() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight()]})).unwrap();
    let before = steps(&s);
    let long = "x".repeat(129);
    for bad in [
        json!([{"name": "Hard"}]),
        json!([{"name": "soft"}]),
        json!([{"name": "  "}]),
        json!([{"name": long}]),
        json!([{"name": "A"}, {"name": "a"}]),
        json!([{"name": "A", "strict": true}]),
        json!([{"name": "A", "inseparable": ["あ"]}]),
        json!([{"name": "A", "inseparable": ["あ い"]}]),
        json!([{"name": "A", "inseparable": "あい"}]),
        json!([{"name": "A", "cannotStart": "あ\u{7}"}]),
        json!([{"name": "A", "cannotStart": ["あ"]}]),
        json!([{"name": "A", "from": "Missing"}]),
        json!([{"name": "A", "from": "Tight"}, {"name": "B", "from": "tight"}]),
        json!((0..257).map(|i| json!({"name": format!("S{i}")})).collect::<Vec<_>>()),
    ] {
        assert!(s.execute("type.kinsokuSets.set", &json!({"sets": bad})).is_err(), "{bad}");
    }
    assert!(s.execute("type.kinsokuSets.set", &json!({})).is_err());
    assert_eq!((names(&s), steps(&s)), (vec!["Tight".to_string()], before), "a refused call changes nothing");
}

/// Kinsoku Set chooses a custom set by name (any case) for the paragraphs `text.setFormat` sets: each
/// holds a copy of it beside Hard, in one undo step; a name the document has no set of fails the call
/// and changes nothing; a built-in set takes the custom one's place. Paragraph styles carry it.
#[test]
fn text_set_format_chooses_a_custom_set_per_paragraph() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight()]})).unwrap();
    let id = type_object(&mut s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    assert!(s.execute("text.setFormat", &json!({"kinsoku": "Missing"})).is_err());
    assert!(text(&s, id).para_at(0).kinsoku_set.is_none(), "a refused call changes nothing");
    let before = steps(&s);
    let second = "あいう\n".len();
    s.execute("text.setFormat", &json!({"kinsoku": "tight", "start": second, "end": second})).unwrap();
    assert_eq!(steps(&s), before + 1);
    let t = text(&s, id);
    assert!(t.para_at(0).kinsoku_set.is_none());
    let p = t.para_at(1);
    assert_eq!((p.kinsoku_name(), p.kinsoku), ("Tight", Kinsoku::Hard));
    s.execute("text.setFormat", &json!({"kinsoku": "Tight", "start": second, "end": second})).unwrap();
    assert_eq!(steps(&s), before + 1, "the set the paragraph already has records nothing");
    // The file keeps the paragraph's copy.
    let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Some(NodeKind::Text(back)) = reopened.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
    assert_eq!(back.para_at(1).kinsoku_name(), "Tight");
    // A paragraph style captured from it carries the set; Soft replaces it.
    s.execute("paraStyle.new", &json!({"name": "Tight Body", "id": id, "start": second, "end": second})).unwrap();
    s.execute("text.setFormat", &json!({"kinsoku": "soft"})).unwrap();
    assert!(text(&s, id).para_at(1).kinsoku_set.is_none());
    assert_eq!(text(&s, id).para_at(1).kinsoku, Kinsoku::Soft);
    s.execute("paraStyle.apply", &json!({"name": "Tight Body", "id": id})).unwrap();
    assert_eq!(text(&s, id).para_at(0).kinsoku_name(), "Tight");
    assert_eq!(text(&s, id).para_at(1).kinsoku_name(), "Tight");
}

/// A paragraph style that gives only a built-in Kinsoku Set replaces a custom set on the paragraphs
/// it is applied to: the style keeps Kinsoku Set's custom set with its built-in choice. One whose
/// attributes give only a custom set (written by hand into a file) brings Hard with it.
#[test]
fn a_style_with_a_built_in_kinsoku_set_replaces_a_custom_set() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight()]})).unwrap();
    let id = type_object(&mut s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    for (style, kinsoku, name) in [("Soft Only", "soft", "Soft"), ("None Only", "none", "None"), ("Hard Only", "hard", "Hard")] {
        s.execute("text.setFormat", &json!({"kinsoku": "Tight"})).unwrap();
        assert_eq!(text(&s, id).para_at(0).kinsoku_name(), "Tight");
        s.execute("paraStyle.new", &json!({"name": style, "attrs": {"kinsoku": kinsoku}})).unwrap();
        s.execute("paraStyle.apply", &json!({"name": style, "id": id})).unwrap();
        let t = text(&s, id);
        assert_eq!((t.para_at(0).kinsoku_name(), t.para_at(1).kinsoku_name()), (name, name), "{style}");
    }
    s.execute("text.setFormat", &json!({"kinsoku": "soft"})).unwrap();
    let only_set = serde_json::Map::from_iter([("kinsoku_set".to_string(), tight_set(&s))]);
    let def = vectorcraft_doc::TextStyleDef { name: "Set Only".into(), attrs: only_set };
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).para_styles.push(def);
    s.execute("paraStyle.apply", &json!({"name": "Set Only", "id": id})).unwrap();
    assert_eq!(text(&s, id).para_at(0).kinsoku_name(), "Tight");
}

fn tight_set(s: &Session) -> Value {
    json!(s.doc().unwrap().doc.kinsoku_sets[0])
}

/// Redefining a paragraph style's Kinsoku Set changes the paragraphs that use the style's set and
/// keeps a paragraph's own custom set: the set and Hard beside it change together.
#[test]
fn redefining_a_style_keeps_a_paragraphs_own_custom_set() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight(), {"name": "Loose"}]})).unwrap();
    let id = type_object(&mut s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Body", "attrs": {"kinsoku_set": tight_set(&s)}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    let second = "あいう\n".len();
    s.execute("text.setFormat", &json!({"kinsoku": "Loose", "start": second, "end": second})).unwrap();
    s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"kinsoku": "soft"}})).unwrap();
    let t = text(&s, id);
    assert_eq!((t.para_at(0).kinsoku_name(), t.para_at(1).kinsoku_name()), ("Soft", "Loose"));
}

/// A paragraph style's custom set is one of the document's, by name in any case, with the
/// document's characters; a name the document has no set of fails the call.
#[test]
fn a_styles_custom_set_is_one_of_the_documents() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight()]})).unwrap();
    assert!(s.execute("paraStyle.new", &json!({"name": "Body", "attrs": {"kinsoku_set": {"name": "Missing"}}})).is_err());
    s.execute("paraStyle.new", &json!({"name": "Body", "attrs": {"kinsoku_set": {"name": "tight", "cannotStart": "x"}}})).unwrap();
    let listed = s.execute("paraStyle.list", &json!({})).unwrap();
    let body = listed["styles"].as_array().unwrap().iter().find(|st| st["name"] == "Body").unwrap().clone();
    assert_eq!(body["attrs"]["kinsoku_set"], tight_set(&s));
    assert!(s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"kinsoku_set": {"name": "Missing"}}})).is_err());
}

/// Editing a set gives every paragraph and paragraph style using it the new characters, renaming it
/// (`from`) the new name too, and deleting it sets them to Hard: each in one undo step, which undo
/// takes back with the set. Paragraphs using no set, or another set, stay as they are.
#[test]
fn editing_renaming_and_deleting_a_set_updates_its_users() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight(), {"name": "Other", "cannotStart": "ー"}]})).unwrap();
    let id = type_object(&mut s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let second = "あいう\n".len();
    s.execute("text.setFormat", &json!({"kinsoku": "Tight", "start": 0, "end": 0})).unwrap();
    s.execute("text.setFormat", &json!({"kinsoku": "Other", "start": second, "end": second})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Uses Tight", "id": id, "start": 0, "end": 0})).unwrap();
    let style_set = |s: &Session| s.doc().unwrap().doc.para_styles.iter().find(|st| st.name == "Uses Tight").unwrap().attrs["kinsoku_set"].clone();
    assert_eq!(style_set(&s)["name"], "Tight");
    let listed = s.execute("type.kinsokuSets.list", &json!({})).unwrap();
    assert_eq!((listed["sets"][0]["paragraphs"].clone(), listed["sets"][0]["styles"].clone()), (json!(1), json!(1)));

    // Edit: 「 may end a line now, and う may start one.
    let before = steps(&s);
    let edited = json!({"name": "Tight", "cannotStart": "」。、", "hanging": "、。", "inseparable": ["——"]});
    s.execute("type.kinsokuSets.set", &json!({"sets": [edited, {"name": "Other", "cannotStart": "ー"}]})).unwrap();
    assert_eq!(steps(&s), before + 1);
    let p = text(&s, id).para_at(0).clone();
    assert_eq!(p.kinsoku_set.as_ref().unwrap().cannot_start, ['、', '。', '」']);
    assert!(p.kinsoku_set.as_ref().unwrap().cannot_end.is_empty());
    assert_eq!(style_set(&s)["cannotStart"], "、。」");
    assert_eq!(text(&s, id).para_at(1).kinsoku_name(), "Other", "another set's paragraph stays");

    // Rename: the users take the new name.
    let renamed = json!({"name": "Strict", "from": "Tight", "cannotStart": "」。、", "hanging": "、。", "inseparable": ["——"]});
    s.execute("type.kinsokuSets.set", &json!({"sets": [renamed, {"name": "Other", "cannotStart": "ー"}]})).unwrap();
    assert_eq!(names(&s), ["Strict", "Other"]);
    assert_eq!(text(&s, id).para_at(0).kinsoku_name(), "Strict");
    assert_eq!(style_set(&s)["name"], "Strict");

    // Delete: its users take Hard, in one step that undo takes back.
    let before = steps(&s);
    s.execute("type.kinsokuSets.set", &json!({"sets": [{"name": "Other", "cannotStart": "ー"}]})).unwrap();
    assert_eq!(steps(&s), before + 1);
    let p = text(&s, id).para_at(0).clone();
    assert_eq!((p.kinsoku_set.is_none(), p.kinsoku), (true, Kinsoku::Hard));
    assert_eq!(style_set(&s), Value::Null);
    assert_eq!(text(&s, id).para_at(1).kinsoku_name(), "Other");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s, id).para_at(0).kinsoku_name(), "Strict");
    assert_eq!(style_set(&s)["name"], "Strict");
    assert_eq!(names(&s), ["Strict", "Other"]);
}

/// Type pasted into another document brings its custom sets: a missing set is added, the
/// document's set of that name with the same characters is used, and one with other characters
/// keeps its own and the pasted set comes in under a free name, so editing the document's set
/// leaves the pasted paragraphs alone. A paragraph style that comes along takes the same set. A
/// document with no room for another set gives the pasted paragraphs Hard.
#[test]
fn pasted_type_brings_its_kinsoku_sets() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight()]})).unwrap();
    let id = type_object(&mut s);
    s.execute("paraStyle.new", &json!({"name": "Tight Body", "attrs": {"kinsoku_set": tight_set(&s)}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Tight Body", "id": id})).unwrap();
    let source = s.doc().unwrap().doc.kinsoku_sets[0].clone();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = |s: &mut Session| -> (vectorcraft_doc::TextObject, Value) {
        let v = s.execute("edit.paste", &json!({})).unwrap();
        (text(s, v["ids"][0].as_u64().unwrap()), v)
    };
    let style_set = |s: &Session| s.doc().unwrap().doc.para_styles.iter().find(|st| st.name == "Tight Body").unwrap().attrs["kinsoku_set"].clone();

    // Pasted back after the set was edited: the document's set as it is now, laid out anew.
    s.execute("type.kinsokuSets.set", &json!({"sets": [{"name": "Tight", "cannotStart": "いうかきく"}]})).unwrap();
    let (t, _) = pasted(&mut s);
    let now = s.doc().unwrap().doc.kinsoku_sets[0].clone();
    assert_eq!(t.para_at(0).kinsoku_set.as_deref(), Some(&now));
    let mut fresh = t.clone();
    crate::cmd::typecmd::refresh_bounds(&mut fresh);
    assert!(fresh == t, "the pasted type is laid out with the document's set");

    // A document without the set.
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let (t, _) = pasted(&mut s);
    assert_eq!(names(&s), ["Tight"]);
    assert_eq!(t.para_at(1).kinsoku_set.as_deref(), Some(&source));
    assert_eq!(style_set(&s), json!(source));
    let (_, again) = pasted(&mut s);
    assert_eq!((again["added"].clone(), names(&s)), (json!(0), vec!["Tight".to_string()]), "a second paste uses the set the first added");

    // A document whose set of that name has other characters.
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s.execute("type.kinsokuSets.set", &json!({"sets": [{"name": "TIGHT", "cannotStart": "ー"}]})).unwrap();
    let (t, v) = pasted(&mut s);
    assert_eq!(names(&s), ["TIGHT", "Tight 2"]);
    let p = t.para_at(0);
    assert_eq!((p.kinsoku_name(), p.kinsoku_set.as_deref().map(|k| &k.cannot_start)), ("Tight 2", Some(&source.cannot_start)));
    assert_eq!(style_set(&s)["name"], "Tight 2");
    assert!(v["renamed"].as_array().unwrap().contains(&json!({"kind": "kinsokuSet", "from": "Tight", "to": "Tight 2"})), "{v}");
    let kept = json!(s.doc().unwrap().doc.kinsoku_sets[1]);
    s.execute("type.kinsokuSets.set", &json!({"sets": [{"name": "TIGHT", "cannotStart": "ッ"}, kept]})).unwrap();
    let users = s.execute("type.kinsokuSets.list", &json!({})).unwrap();
    assert_eq!(users["sets"][0]["paragraphs"], 0, "editing the document's own set leaves the pasted paragraphs alone: {users}");

    // A document with no room for another set.
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let full: Vec<Value> = (0..256).map(|i| json!({"name": format!("Set {i}")})).collect();
    s.execute("type.kinsokuSets.set", &json!({"sets": full})).unwrap();
    let (t, _) = pasted(&mut s);
    assert_eq!(names(&s).len(), 256);
    let p = t.para_at(0);
    assert_eq!((p.kinsoku_set.is_none(), p.kinsoku), (true, Kinsoku::Hard));
    assert_eq!(style_set(&s), Value::Null);
}

/// New Set's starting point: Hard and Soft give their characters, None gives an empty set, and a
/// custom set (any case) gives its own; an unknown base fails.
#[test]
fn a_new_set_starts_from_a_built_in_or_custom_set() {
    let mut s = session();
    s.execute("type.kinsokuSets.set", &json!({"sets": [tight()]})).unwrap();
    let hard = s.execute("type.kinsokuSets.base", &json!({"base": "hard", "name": "New"})).unwrap();
    let soft = s.execute("type.kinsokuSets.base", &json!({"base": "soft"})).unwrap();
    let start = |v: &Value| v["set"]["cannotStart"].as_str().unwrap().to_string();
    assert!(start(&hard).contains('ー') && !start(&soft).contains('ー') && start(&soft).contains('」'));
    assert_eq!(hard["set"]["name"], "New");
    assert_eq!(hard["set"]["hanging"], "、。，．｡､");
    assert_eq!(hard["set"]["inseparable"].as_array().unwrap().len(), 5);
    let none = s.execute("type.kinsokuSets.base", &json!({"base": "none"})).unwrap();
    assert_eq!(none["set"]["cannotStart"], "");
    let custom = s.execute("type.kinsokuSets.base", &json!({"base": "TIGHT"})).unwrap();
    assert_eq!(custom["set"]["cannotEnd"], "「");
    assert!(s.execute("type.kinsokuSets.base", &json!({"base": "Missing"})).is_err());
}
