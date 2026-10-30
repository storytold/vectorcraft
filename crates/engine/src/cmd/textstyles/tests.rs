use super::*;

fn session_with_text(text: &str) -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = s.execute("text.create", &json!({"x": 10, "y": 50, "text": text})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    (s, id)
}

fn runs(s: &Session, id: u64) -> Vec<vectorcraft_doc::TextRun> {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!(),
    }
}

#[test]
fn char_style_new_apply_range_redefine_keeps_overrides() {
    let (mut s, id) = session_with_text("Hello world");
    s.execute("charStyle.new", &json!({"name": "Emphasis", "attrs": {"size": 24.0, "underline": true}})).unwrap();
    // Style "world" only (a Type tool range).
    s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11})).unwrap();
    let r = runs(&s, id);
    assert_eq!(r.len(), 2);
    assert_eq!((r[1].text.as_str(), r[1].style.size, r[1].style.underline), ("world", 24.0, true));
    assert_eq!(r[1].style.style_name.as_deref(), Some("Emphasis"));
    assert_eq!(r[0].style.style_name, None);
    // A local override on the styled run survives a redefinition; other attributes update.
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 6, "end": 11, "size": 30.0})).unwrap();
    s.execute("charStyle.setAttrs", &json!({"name": "Emphasis", "attrs": {"size": 18.0, "underline": false, "tracking": 50.0}})).unwrap();
    let r = runs(&s, id);
    assert_eq!((r[1].style.size, r[1].style.underline, r[1].style.tracking), (30.0, false, 50.0));
    // Clear Overrides: back to exactly the style's attributes.
    s.execute("charStyle.apply", &json!({"name": "Emphasis", "id": id, "start": 6, "end": 11, "clearOverrides": true})).unwrap();
    assert_eq!(runs(&s, id)[1].style.size, 18.0);
    let list = s.execute("charStyle.list", &json!({})).unwrap();
    assert_eq!(list["styles"][0]["name"], NORMAL_CHAR);
    assert_eq!(list["styles"][1]["uses"], 1);
}

/// Applying and redefining a style record a step named from a template with the style's name
/// in it (`HistoryEntry::named`), and undo and redo keep it. The Normal styles' names are
/// marked as names VectorCraft gives. A new style's step has a name of its own.
#[test]
fn style_steps_are_named_from_a_template() {
    let (mut s, id) = session_with_text("Hello");
    let last = |s: &Session| {
        let e = s.doc().unwrap().history.undo.last().unwrap();
        (e.label.clone(), e.named().map(|n| (n.template, n.name.clone(), n.built_in)))
    };
    let named = |template, name: &str, built_in| Some((template, name.to_string(), built_in));
    s.execute("charStyle.new", &json!({"name": "Body"})).unwrap();
    assert_eq!(last(&s), ("New Character Style".into(), None));
    s.execute("charStyle.apply", &json!({"name": "Body", "id": id})).unwrap();
    assert_eq!(last(&s), ("Apply Body".into(), named("Apply {name}", "Body", false)));
    s.execute("edit.undo", &json!({})).unwrap();
    let redo = s.doc().unwrap().history.redo.last().unwrap().named().cloned();
    assert_eq!(redo.map(|n| n.label()).as_deref(), Some("Apply Body"));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(last(&s), ("Apply Body".into(), named("Apply {name}", "Body", false)));
    s.execute("charStyle.apply", &json!({"name": NORMAL_CHAR, "id": id})).unwrap();
    assert_eq!(last(&s), (format!("Apply {NORMAL_CHAR}"), named("Apply {name}", NORMAL_CHAR, true)));
    s.execute("paraStyle.new", &json!({"name": "Lead"})).unwrap();
    assert_eq!(last(&s), ("New Paragraph Style".into(), None));
    s.execute("paraStyle.setAttrs", &json!({"name": "Lead", "attrs": {"direction": "rightToLeft"}})).unwrap();
    assert_eq!(last(&s), ("Redefine Lead".into(), named("Redefine {name}", "Lead", false)));
    // A step renamed since it was recorded is no longer named from the template.
    s.doc_mut().unwrap().history.undo.last_mut().unwrap().label = "Typing".into();
    assert_eq!(last(&s), ("Typing".into(), None));
}

#[test]
fn new_from_selection_rename_delete_and_undo() {
    let (mut s, id) = session_with_text("Title");
    s.execute("text.setStyle", &json!({"size": 40})).unwrap();
    let name = s.execute("charStyle.new", &json!({})).unwrap()["name"].as_str().unwrap().to_string();
    assert_eq!(name, "Character Style 1");
    assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs["size"], 40.0);
    s.execute("charStyle.apply", &json!({"name": name})).unwrap();
    s.execute("charStyle.rename", &json!({"name": name, "to": "Heading"})).unwrap();
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
    assert!(s.execute("charStyle.rename", &json!({"name": NORMAL_CHAR, "to": "x"})).is_err());
    s.execute("charStyle.duplicate", &json!({"name": "Heading"})).unwrap();
    s.execute("charStyle.delete", &json!({"name": "Heading"})).unwrap();
    assert_eq!(runs(&s, id)[0].style.style_name, None);
    assert_eq!(runs(&s, id)[0].style.size, 40.0, "deleting keeps the look");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Heading"));
    assert!(s.execute("charStyle.new", &json!({"name": "Bad", "attrs": {"nope": 1}})).is_err());
    // Attributes are stored canonically (a hand-written colour matches the text's afterwards).
    s.execute(
        "charStyle.new",
        &json!({"name": "Pink", "attrs": {"fill": {"type": "solid", "color": {"model": "rgb", "r": 0.9, "g": 0.2, "b": 0.4}}}}),
    )
    .unwrap();
    s.execute("charStyle.apply", &json!({"name": "Pink"})).unwrap();
    let fill = serde_json::to_value(&runs(&s, id)[0].style.fill).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == "Pink").unwrap().attrs["fill"], fill);
}

/// New Character Style and New Paragraph Style trim the name they are given, as a rename
/// trims `to`; the trimmed name is the one compared with the styles there are.
#[test]
fn new_styles_trim_their_names_as_a_rename_does() {
    let (mut s, _) = session_with_text("Body text");
    for p in ["charStyle", "paraStyle"] {
        assert_eq!(s.execute(&format!("{p}.new"), &json!({"name": "  Body \t"})).unwrap()["name"], "Body", "{p}");
        let list = s.execute(&format!("{p}.list"), &json!({})).unwrap();
        let names: Vec<&str> = list["styles"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
        assert_eq!(names[1..], ["Body"], "{p}");
        let e = s.execute(&format!("{p}.new"), &json!({"name": " Body"})).unwrap_err().to_string();
        assert!(e.contains("a style named `Body` already exists"), "{p}: {e}");
        let e = s.execute(&format!("{p}.new"), &json!({"name": " \t "})).unwrap_err().to_string();
        assert!(e.contains("give the style a name"), "{p}: {e}");
        assert_eq!(s.execute(&format!("{p}.duplicate"), &json!({"name": "Body"})).unwrap()["name"], "Body copy", "{p}");
    }
    // A style whose name has spaces around it, as a saved file can hold, is copied under the
    // trimmed name, numbered past the copies there are.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).char_styles.push(TextStyleDef { name: " Old ".into(), attrs: Map::new() });
    s.execute("charStyle.new", &json!({"name": "Old copy"})).unwrap();
    assert_eq!(s.execute("charStyle.duplicate", &json!({"name": " Old "})).unwrap()["name"], "Old copy 2");
}

fn unused(s: &mut Session, p: &str) -> Vec<String> {
    serde_json::from_value(s.execute(&format!("{p}.unused"), &json!({})).unwrap()["names"].clone()).unwrap()
}

fn listed(s: &mut Session, p: &str) -> Vec<String> {
    let list = s.execute(&format!("{p}.list"), &json!({})).unwrap();
    list["styles"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).map(str::to_string).collect()
}

/// Select All Unused: `charStyle.unused` and `paraStyle.unused` list, in panel order and without an
/// undo step, the styles no text uses, never the Normal style. Type in a symbol, an opacity mask, a
/// graph design, a pattern tile or a brush's art counts, as type in the layers does.
#[test]
fn unused_styles_are_those_no_text_uses_wherever_the_text_is() {
    let (mut s, id) = session_with_text("Used");
    let names = ["Spare", "Used", "Symbol", "Mask", "Design", "Pattern", "Brush", "Also spare"];
    for p in ["charStyle", "paraStyle"] {
        for n in names {
            s.execute(&format!("{p}.new"), &json!({"name": n, "attrs": {}})).unwrap();
        }
        s.execute(&format!("{p}.apply"), &json!({"name": "Used", "id": id})).unwrap();
    }
    // Type in style `name`, selected.
    let styled = |s: &mut Session, name: &str| {
        let t = s.execute("text.create", &json!({"x": 10, "y": 150, "text": name})).unwrap()["id"].as_u64().unwrap();
        for p in ["charStyle", "paraStyle"] {
            s.execute(&format!("{p}.apply"), &json!({"name": name, "id": t})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [t]})).unwrap();
        NodeId(t)
    };
    let clear = |s: &mut Session, id: NodeId| {
        s.execute("edit.clear", &json!({"ids": [id.0]})).unwrap();
    };
    styled(&mut s, "Symbol");
    let sym = s.execute("symbol.new", &json!({"name": "Mark"})).unwrap();
    clear(&mut s, NodeId(sym["id"].as_u64().unwrap()));
    let masked = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
    let mask = styled(&mut s, "Mask");
    s.execute("select.set", &json!({"ids": [masked, mask.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let design = styled(&mut s, "Design");
    s.execute("graph.design", &json!({"new": "Bar"})).unwrap();
    clear(&mut s, design);
    let tile = styled(&mut s, "Pattern");
    s.execute("object.pattern.make", &json!({"edit": false})).unwrap();
    if s.doc().unwrap().doc.node(tile).is_some() {
        clear(&mut s, tile);
    }
    let art = styled(&mut s, "Brush");
    s.execute("brush.new", &json!({"type": "art", "name": "Lettering"})).unwrap();
    clear(&mut s, art);
    let undo = s.doc().unwrap().history.undo.len();
    for p in ["charStyle", "paraStyle"] {
        assert_eq!(unused(&mut s, p), ["Spare", "Also spare"], "{p}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    let list = s.execute("charStyle.list", &json!({})).unwrap();
    let uses = |name: &str| list["styles"].as_array().unwrap().iter().find(|x| x["name"] == name).unwrap()["uses"].as_u64();
    for name in ["Symbol", "Mask", "Design", "Pattern", "Brush"] {
        assert_eq!(uses(name), Some(1), "{name}");
    }
}

/// `charStyle.delete` and `paraStyle.delete` take `names` and delete them in one undo step; text
/// that used them keeps its look. A name that is not a style, the Normal style, or `names` that is
/// not an array of strings deletes none.
#[test]
fn several_styles_are_deleted_in_one_step() {
    let (mut s, id) = session_with_text("Title");
    s.execute("text.setStyle", &json!({"size": 40})).unwrap();
    for p in ["charStyle", "paraStyle"] {
        for n in ["A", "B", "C"] {
            s.execute(&format!("{p}.new"), &json!({"name": n})).unwrap();
        }
        s.execute(&format!("{p}.apply"), &json!({"name": "B", "id": id})).unwrap();
        let before = listed(&mut s, p);
        let undo = s.doc().unwrap().history.undo.len();
        let normal = before[0].clone();
        for bad in [json!(["A", "Nope"]), json!(["A", normal]), json!([]), json!(["A", 7]), json!("A")] {
            assert!(s.execute(&format!("{p}.delete"), &json!({"names": bad})).is_err(), "{p} {bad}");
        }
        assert_eq!((listed(&mut s, p), s.doc().unwrap().history.undo.len()), (before.clone(), undo), "{p}");
        s.execute(&format!("{p}.delete"), &json!({"names": ["A", "B"]})).unwrap();
        assert_eq!(listed(&mut s, p), [normal.as_str(), "C"], "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{p}");
        assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Delete Styles", "{p}");
        assert_eq!(listed(&mut s, p), before, "{p}");
    }
    s.execute("charStyle.delete", &json!({"names": ["B"]})).unwrap();
    assert_eq!((runs(&s, id)[0].style.style_name.as_deref(), runs(&s, id)[0].style.size), (None, 40.0));
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Delete Style");
}

/// #1001: attributes their JSON leaves out at the defaults (Auto leading and kerning, no
/// mojikumi) are still attributes a style can set, and a style captured with one applies to
/// text at the default, is duplicated and comes back out of a library.
#[test]
fn styles_take_attributes_left_out_at_their_defaults() {
    let (mut s, id) = session_with_text("A");
    s.execute("charStyle.new", &json!({"name": "Kerned", "attrs": {"kerning": 40, "leading": 30}})).unwrap();
    s.execute("charStyle.setAttrs", &json!({"name": "Kerned", "attrs": {"kerning": 20, "small_caps": 70}})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Punctuation", "attrs": {"mojikumi": "lineEndHalf"}})).unwrap();
    s.execute("paraStyle.setAttrs", &json!({"name": "Punctuation", "attrs": {"direction": "rightToLeft"}})).unwrap();
    for bad in [json!({"nope": 1}), json!({"justify_auto": true})] {
        assert!(s.execute("paraStyle.new", &json!({"name": "Bad", "attrs": bad})).is_err(), "{bad}");
    }
    // Captured from text with explicit leading, applied to fresh Auto-leading text.
    s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Spaced"})).unwrap();
    let fresh = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "B"})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(runs(&s, fresh)[0].style.leading, None);
    for clear in [false, true] {
        s.execute("charStyle.apply", &json!({"name": "Spaced", "id": fresh, "clearOverrides": clear})).unwrap();
        assert_eq!(runs(&s, fresh)[0].style.leading, Some(30.0));
    }
    let copy = s.execute("charStyle.duplicate", &json!({"name": "Spaced"})).unwrap()["name"].as_str().unwrap().to_string();
    assert_eq!(s.doc().unwrap().doc.char_styles.iter().find(|d| d.name == copy).unwrap().attrs["leading"], 30.0);
    s.execute("charStyle.apply", &json!({"name": "Kerned", "id": fresh})).unwrap();
    let st = runs(&s, fresh)[0].style.clone();
    assert_eq!((st.kerning, st.small_caps), (Some(20.0), Some(70.0)));
    s.execute("paraStyle.apply", &json!({"name": "Punctuation", "id": id})).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!() };
    assert_eq!((t.para.mojikumi, t.para.direction), (vectorcraft_doc::Mojikumi::LineEndHalf, Some(vectorcraft_doc::ParaDirection::RightToLeft)));
    // A library keeps the captured attributes and makes the style again where it's used.
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let lib = s.execute("library.add", &json!({"kind": "charStyle", "name": "Lib Spaced"})).unwrap();
    let other = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "C"})).unwrap()["id"].as_u64().unwrap();
    s.execute("library.use", &json!({"library": lib["library"], "kind": "charStyle", "item": lib["name"]})).unwrap();
    assert_eq!(runs(&s, other)[0].style.leading, Some(30.0));
}

#[test]
fn auto_leading_redefine_keeps_overrides_and_history() {
    for redefine in [true, false] {
        let (mut s, source) = session_with_text("Source");
        let [linked, local] =
            ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
        let ids = [source, linked, local];
        s.execute("charStyle.new", &json!({"name": "Body", "attrs": {"leading": 30}})).unwrap();
        s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
        s.execute("text.setStyle", &json!({"ids": [local], "leading": 42})).unwrap();
        s.execute("text.setStyle", &json!({"ids": [source], "leading": "auto"})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let undo = s.doc().unwrap().history.undo.len();
        if redefine {
            s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
        } else {
            s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": null}})).unwrap();
        }
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [None, None, Some(42.0)]);
        assert!(ids.iter().all(|&id| runs(&s, id)[0].style.style_name.as_deref() == Some("Body")));
        let after = s.doc().unwrap().doc.clone();
        assert_eq!(after.char_styles[0].attrs.get("leading"), Some(&Value::Null), "Auto is explicit, not an unset attribute");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, after);
        let back = vectorcraft_format::load(&vectorcraft_format::save(&after, false)).unwrap();
        assert_eq!(back.char_styles, after.char_styles);
        for id in ids {
            let NodeKind::Text(t) = &back.node(NodeId(id)).unwrap().kind else { panic!("not text") };
            assert_eq!(t.runs, runs(&s, id));
        }
        // Auto users follow the next numeric definition; real numeric and Auto overrides stay.
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": 26}})).unwrap();
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(26.0), Some(26.0), Some(42.0)]);
        s.execute("text.setStyle", &json!({"ids": [linked], "leading": "auto"})).unwrap();
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"leading": 28}})).unwrap();
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(28.0), None, Some(42.0)]);
        // Omitting leading replaces the definition without resetting the text's leading.
        s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"tracking": 20}})).unwrap();
        assert!(!s.doc().unwrap().doc.char_styles[0].attrs.contains_key("leading"));
        assert_eq!(ids.map(|id| runs(&s, id)[0].style.leading), [Some(28.0), None, Some(42.0)]);
    }
}

/// The attribute names a style of type `T` can set, without the style name.
fn settable<T: DeserializeOwned>() -> std::collections::BTreeSet<String> {
    attr_names::<T>().into_iter().filter(|&n| n != "style_name").map(String::from).collect()
}

/// New and Redefine capture every paragraph attribute. A paragraph style redefined from a
/// paragraph at the defaults gives its paragraphs the defaults back where they had the old
/// definition's values, and `setAttrs` with a default value does too; local overrides, and
/// attributes `setAttrs` leaves out, stay.
#[test]
fn para_style_redefined_to_the_defaults_updates_its_paragraphs() {
    use vectorcraft_doc::{Composer, HyphenationOptions};
    let (mut s, source) = session_with_text("Source");
    let [linked, local] =
        ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
    let ids = [source, linked, local];
    let para = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para_at(0).clone(),
        _ => panic!("not text"),
    };
    let limits = |s: &Session| ids.map(|id| para(s, id).hyphenation.hyphen_limit);
    let composers = |s: &Session| ids.map(|id| para(s, id).composer);
    let defined = |s: &Session, i: usize| s.doc().unwrap().doc.para_styles[i].attrs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    s.execute("text.setFormat", &json!({"ids": ids, "hyphenLimit": 2, "composer": "singleLine"})).unwrap();
    s.execute("paraStyle.new", &json!({"name": "Body", "id": source})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [local], "hyphenLimit": 4})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [source], "hyphenLimit": 0, "composer": "everyLine"})).unwrap();
    s.execute("paraStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
    assert_eq!((limits(&s), composers(&s)), ([0, 0, 4], [Composer::EveryLine; 3]));
    assert_eq!(para(&s, linked).hyphenation, HyphenationOptions::default());
    assert!(ids.iter().all(|&id| para(&s, id).style_name.as_deref() == Some("Body")));
    assert_eq!(defined(&s, 0), settable::<ParaStyle>(), "Redefine captures every attribute");
    s.execute("paraStyle.new", &json!({"name": "Copy", "id": local})).unwrap();
    assert_eq!(defined(&s, 1), settable::<ParaStyle>(), "New captures every attribute");
    // Style options: a default value reaches the users too; an attribute left out stays as it is.
    s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"hyphenation": {"hyphen_limit": 3}, "composer": "singleLine"}})).unwrap();
    assert_eq!((limits(&s), composers(&s)), ([3, 3, 4], [Composer::SingleLine; 3]));
    s.execute("paraStyle.setAttrs", &json!({"name": "Body", "attrs": {"composer": "everyLine"}})).unwrap();
    assert_eq!((limits(&s), composers(&s)), ([3, 3, 4], [Composer::EveryLine; 3]));
    assert_eq!(Value::Object(s.doc().unwrap().doc.para_styles[0].attrs.clone()), json!({"composer": "everyLine"}));
}

/// The same for character styles: Auto kerning and the Roman baseline alignment, which the
/// run's JSON leaves out, are captured and reach the style's users.
#[test]
fn char_style_redefined_to_the_defaults_updates_its_text() {
    use vectorcraft_doc::CharAlign;
    let (mut s, source) = session_with_text("Source");
    let [linked, local] =
        ["Linked", "Local"].map(|text| s.execute("text.create", &json!({"x": 10, "y": 90, "text": text})).unwrap()["id"].as_u64().unwrap());
    let ids = [source, linked, local];
    let kerning = |s: &Session| ids.map(|id| runs(s, id)[0].style.kerning);
    let aligns = |s: &Session| ids.map(|id| runs(s, id)[0].style.char_align);
    let defined = |s: &Session, i: usize| s.doc().unwrap().doc.char_styles[i].attrs.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    s.execute("text.setFormat", &json!({"ids": ids, "kerning": 40, "charAlign": "emBoxCenter"})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "id": source})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [local], "kerning": 60})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [source], "kerning": "auto", "charAlign": "romanBaseline"})).unwrap();
    s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
    assert_eq!((kerning(&s), aligns(&s)), ([None, None, Some(60.0)], [CharAlign::RomanBaseline; 3]));
    assert_eq!(defined(&s, 0), settable::<CharStyle>(), "Redefine captures every attribute");
    s.execute("charStyle.new", &json!({"name": "Copy", "id": local})).unwrap();
    assert_eq!(defined(&s, 1), settable::<CharStyle>(), "New captures every attribute");
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"kerning": 10, "charAlign": "emBoxTop"}})).unwrap();
    assert_eq!((kerning(&s), aligns(&s)), ([Some(10.0), Some(10.0), Some(60.0)], [CharAlign::EmBoxTop; 3]));
    s.execute("charStyle.setAttrs", &json!({"name": "Body", "attrs": {"charAlign": "romanBaseline"}})).unwrap();
    assert_eq!((kerning(&s), aligns(&s)), ([Some(10.0), Some(10.0), Some(60.0)], [CharAlign::RomanBaseline; 3]));
    assert_eq!(Value::Object(s.doc().unwrap().doc.char_styles[0].attrs.clone()), json!({"charAlign": "romanBaseline"}));
}

/// A style captured from type without Variable Font settings sets none explicitly (`{}`):
/// applying it clears type's settings, and redefining it from type whose settings were reset
/// resets the type that uses it.
#[test]
fn variable_font_settings_are_captured_explicitly() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let (mut s, source) = session_with_text("Source");
    let linked = s.execute("text.create", &json!({"x": 10, "y": 90, "text": "Linked"})).unwrap()["id"].as_u64().unwrap();
    let ids = [source, linked];
    s.execute("text.setStyle", &json!({"ids": ids, "font": TWO_AXIS_FAMILY})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "id": source})).unwrap();
    assert_eq!(s.doc().unwrap().doc.char_styles[0].attrs.get("fontAxes"), Some(&json!({})));
    s.execute("text.setFormat", &json!({"ids": [linked], "fontAxes": {"wght": 650}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Body", "ids": ids})).unwrap();
    let wght = |s: &Session| ids.map(|id| runs(s, id)[0].style.font_axes.get("wght").copied());
    assert_eq!(wght(&s), [None, None], "applying the style clears the settings");
    for (set, want) in [(json!(650), Some(650.0)), (json!(null), None)] {
        s.execute("text.setFormat", &json!({"ids": [source], "fontAxes": {"wght": set}})).unwrap();
        s.execute("charStyle.redefine", &json!({"name": "Body", "id": source})).unwrap();
        assert_eq!(wght(&s), [want, want], "redefined from {set}");
    }
}

/// A style that changes the font, applied or redefined, leaves none of the old font's
/// version and Variable Font settings, unless the style sets them.
#[test]
fn another_font_from_a_style_clears_the_old_fonts_version_and_settings() {
    let _fonts = crate::adding_fonts();
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let (mut s, id) = session_with_text("Source");
    let set_variable = |s: &mut Session| {
        s.execute("text.setStyle", &json!({"ids": [id], "font": TWO_AXIS_FAMILY})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [id], "fontAxes": {"wght": 650}})).unwrap();
        s.edit("version", |d, _| {
            if let Some(t) = text_mut(d, NodeId(id)) {
                t.runs.iter_mut().for_each(|r| r.style.font_version = Some("Version 2.000".into()));
            }
            Ok(())
        })
        .unwrap();
    };
    let font = |s: &Session| {
        let st = runs(s, id)[0].style.clone();
        (st.font_family, st.font_version, st.font_axes.get("wght").copied())
    };
    set_variable(&mut s);
    s.execute("charStyle.new", &json!({"name": "Plain", "attrs": {"font_family": "Inter"}})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Same", "attrs": {"size": 20}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Same", "id": id})).unwrap();
    assert_eq!(font(&s), (TWO_AXIS_FAMILY.to_string(), Some("Version 2.000".to_string()), Some(650.0)), "the same font keeps them");
    s.execute("charStyle.apply", &json!({"name": "Plain", "id": id})).unwrap();
    assert_eq!(font(&s), ("Inter".to_string(), None, None));
    // A style that sets the settings keeps its own.
    s.execute("charStyle.new", &json!({"name": "Heavy", "attrs": {"font_family": TWO_AXIS_FAMILY, "fontAxes": {"wght": 600}}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Heavy", "id": id})).unwrap();
    assert_eq!(font(&s), (TWO_AXIS_FAMILY.to_string(), None, Some(600.0)));
    // Redefined to another font: its users' settings and versions go too.
    s.execute("charStyle.new", &json!({"name": "Sans", "attrs": {"font_family": TWO_AXIS_FAMILY}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Sans", "id": id})).unwrap();
    set_variable(&mut s);
    assert_eq!(runs(&s, id)[0].style.style_name.as_deref(), Some("Sans"));
    s.execute("charStyle.setAttrs", &json!({"name": "Sans", "attrs": {"font_family": "Inter"}})).unwrap();
    assert_eq!(font(&s), ("Inter".to_string(), None, None));
}

#[test]
fn auto_leading_new_distinguishes_unset_attributes() {
    let (mut s, id) = session_with_text("Title");
    s.execute("text.setStyle", &json!({"leading": 30})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Sparse", "attrs": {"tracking": 20}})).unwrap();
    assert!(!s.doc().unwrap().doc.char_styles[0].attrs.contains_key("leading"));
    s.execute("charStyle.apply", &json!({"name": "Sparse", "id": id})).unwrap();
    assert_eq!(runs(&s, id)[0].style.leading, Some(30.0));
    s.execute("charStyle.new", &json!({"name": "Automatic", "attrs": {"leading": null}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Automatic", "id": id})).unwrap();
    assert_eq!(runs(&s, id)[0].style.leading, None);
    assert_eq!(s.doc().unwrap().doc.char_styles[1].attrs.get("leading"), Some(&Value::Null));
    // New from Auto text captures that value too, rather than leaving leading unspecified.
    s.execute("charStyle.new", &json!({"name": "Captured", "id": id})).unwrap();
    s.execute("text.setStyle", &json!({"ids": [id], "leading": 36})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Captured", "id": id})).unwrap();
    assert_eq!(runs(&s, id)[0].style.leading, None);
    assert_eq!(s.doc().unwrap().doc.char_styles[2].attrs.get("leading"), Some(&Value::Null));
}

#[test]
fn paragraph_styles_and_survive_save() {
    let (mut s, id) = session_with_text("Body");
    s.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Center", "space_after": 6.0}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Centered"})).unwrap();
    let para = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.para.clone(),
        _ => panic!(),
    };
    assert_eq!((para(&s).justify, para(&s).space_after), (vectorcraft_doc::Justify::Center, 6.0));
    // The Normal style can be redefined too (stored once redefined).
    s.execute("paraStyle.redefine", &json!({"name": NORMAL_PARA})).unwrap();
    assert_eq!(s.execute("paraStyle.list", &json!({})).unwrap()["styles"][0]["attrs"]["justify"], "Center");
    let d = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(back.para_styles, d.para_styles);
}

fn capture_boundary_command(redefine: bool, text: &str, start: usize, end: Option<usize>, expected_size: f64) {
    let (mut s, id) = session_with_text(text);
    s.execute("text.setStyle", &json!({"id": id, "size": 18})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 2, "end": text.len(), "size": 34})).unwrap();
    if redefine {
        s.execute("charStyle.new", &json!({"name": "Captured", "attrs": {"size": 24}})).unwrap();
    }
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let mut params = json!({"name": "Captured", "id": id, "start": start});
    if let Some(end) = end {
        params["end"] = json!(end);
    }
    let command = if redefine { "charStyle.redefine" } else { "charStyle.new" };
    s.execute(command, &params).unwrap();
    let st = s.doc().unwrap();
    let def = st.doc.char_styles.iter().find(|d| d.name == "Captured").unwrap();
    assert_eq!(def.attrs.get("size").and_then(Value::as_f64), Some(expected_size), "{command} {text:?} {start}..{end:?}");
    assert!(!def.attrs.contains_key("style_name"));
    assert_eq!(st.doc.node(NodeId(id)), before.node(NodeId(id)), "capture does not apply the definition");
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert!(st.is_dirty());
    let after = st.doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().selection, selection);
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, after);
    assert_eq!(s.doc().unwrap().selection, selection);
}

fn capture_boundary_ranges(redefine: bool) {
    for (text, start, end, size) in [
        ("ABcdef", 2, 5, 34.0),
        ("é漢字", 2, 5, 34.0),
        ("ABcdef", 5, 2, 34.0), // command ranges are ordered before selected-character sampling
        ("ABcdef", 1, 5, 18.0), // a mixed range samples its first selected character
        ("ABcdef", 2, 2, 18.0), // collapsed range retains insertion affinity
        ("ABcdef", 3, 3, 34.0),
        ("ABcdef", 99, 100, 34.0), // beyond the text retains the final run
    ] {
        capture_boundary_command(redefine, text, start, Some(end), size);
    }
}

#[test]
fn char_style_new_range_capture_boundary() {
    capture_boundary_ranges(false);
}

#[test]
fn char_style_redefine_range_capture_boundary() {
    capture_boundary_ranges(true);
}

#[test]
fn char_style_start_only_capture_affinity_controls() {
    for redefine in [false, true] {
        for text in ["ABcdef", "é漢字"] {
            for (start, size) in [(0, 18.0), (2, 18.0), (3, 34.0), (99, 34.0)] {
                capture_boundary_command(redefine, text, start, None, size);
            }
        }
    }
}

#[test]
fn para_style_range_capture_samples_start_paragraph() {
    for redefine in [false, true] {
        for (start, end, justify) in [(0, 7, "Left"), (4, 7, "Right"), (4, 0, "Right")] {
            let (mut s, id) = session_with_text("One\nTwo");
            s.execute("text.setStyle", &json!({"id": id, "justify": "left"})).unwrap();
            s.execute("text.setStyle", &json!({"id": id, "start": 4, "end": 7, "justify": "right"})).unwrap();
            if redefine {
                s.execute("paraStyle.new", &json!({"name": "Captured", "attrs": {"justify": "Center"}})).unwrap();
            }
            let before = s.doc().unwrap().doc.clone();
            let selection = s.doc().unwrap().selection.clone();
            let undo = s.doc().unwrap().history.undo.len();
            let command = if redefine { "paraStyle.redefine" } else { "paraStyle.new" };
            s.execute(command, &json!({"name": "Captured", "id": id, "start": start, "end": end})).unwrap();
            let st = s.doc().unwrap();
            let def = st.doc.para_styles.iter().find(|d| d.name == "Captured").unwrap();
            assert_eq!(def.attrs["justify"], json!(justify), "paragraph capture still samples start, not the end or a normalized range");
            assert_eq!(st.doc.node(NodeId(id)), before.node(NodeId(id)));
            assert_eq!(st.selection, selection);
            assert_eq!(st.history.undo.len(), undo + 1);
            let after = st.doc.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, after);
        }
    }
}
