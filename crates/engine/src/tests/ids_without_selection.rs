//! #1575: commands that take `ids` work on the objects given, with nothing selected; the
//! selection is left as it was.

use serde_json::json;

use crate::*;

fn doc_with_rect() -> (Session, u64) {
    let mut s = Session::new();
    // Separately created fixtures must compare equal across clock ticks.
    s.execute("file.new", &json!({"width": 200, "height": 200, "created": null})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    (s, id)
}

#[test]
fn ids_stand_in_for_the_selection() {
    for (cmd, params) in [
        ("object.repeat.radial", json!({})),
        ("edit.colors.invert", json!({})),
        ("object.pattern.make", json!({"name": "P"})),
        ("symbol.new", json!({"name": "S"})),
        ("object.mesh.create", json!({})),
        ("object.makePixelPerfect", json!({})),
    ] {
        let (mut s, id) = doc_with_rect();
        let mut p = params;
        p["ids"] = json!([id]);
        let r = s.execute(cmd, &p);
        assert!(r.is_ok(), "{cmd}: {r:?}");
    }
}

#[test]
fn the_selection_is_put_back_and_the_check_still_applies() {
    let (mut s, id) = doc_with_rect();
    // Disabled even with the object given: Make with Top Object needs two.
    let r = s.execute("object.envelope.makeWithTopObject", &json!({"ids": [id]}));
    assert!(matches!(r, Err(EngineError::Disabled(..))), "{r:?}");
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the stand-in selection is put back");
    // Without ids, nothing selected is still nothing selected.
    let r = s.execute("object.repeat.radial", &json!({}));
    assert!(matches!(r, Err(EngineError::Disabled(..))), "{r:?}");
}

#[test]
fn only_an_ids_key_counts() {
    let p = json!({"ids": [3]});
    assert_eq!(given_ids("{ids?: [id]}", &p), Some(vec![NodeId(3)]));
    assert_eq!(given_ids("{id?|ids?, name}", &p), Some(vec![NodeId(3)]));
    assert_eq!(given_ids("{grids: n, guides: bool}", &p), None);
    // A word in what the description says about another key isn't the key.
    assert_eq!(given_ids("{name: of the swatch the ids of the selection take}", &p), None);
    assert_eq!(given_ids("{ids?: [id]}", &json!({"ids": []})), None);
    // `id` is `ids` of one where the description names both keys, and `ids` aren't given.
    let one = json!({"id": 3});
    assert_eq!(given_ids("{ids?|id?}", &one), Some(vec![NodeId(3)]));
    assert_eq!(given_ids("{id?, ids?: [id…] (instead of id)}", &one), Some(vec![NodeId(3)]));
    assert_eq!(given_ids("{ids?: [id…]}", &one), None, "a description without `id`");
    assert_eq!(given_ids("{id?: dimension id}", &one), None, "a description without `ids`");
    assert_eq!(given_ids("{ids?|id?}", &json!({"id": "a1"})), None, "not an object id");
    assert_eq!(given_ids("{ids?|id?}", &json!({"id": 3, "ids": [4]})), Some(vec![NodeId(4)]), "`ids` first");
    assert_eq!(given_ids("{ids?|id?}", &json!({"id": 3, "ids": null})), Some(vec![NodeId(3)]));
    // A required `id` beside `ids` is something else (`plugin.run`'s plug-in); `{id} or {ids}` is
    // one object or several.
    assert_eq!(given_ids("{id, params?: {…}, ids?: [..]}", &one), None, "a plug-in's id");
    assert!(!cmd::params::id_stands_for_ids("{id, params?: {…}, ids?: [..]}"));
    assert!(cmd::params::id_stands_for_ids("{id} or {ids: [id…]}: toggles them"));
    assert!(cmd::params::id_stands_for_ids("{ids|id, target}"));
}

/// `{"id": x}` with params besides it, and the same as `{"ids": [x]}`.
fn id_and_ids(cmd: &str, x: u64, d: u64) -> (serde_json::Value, serde_json::Value) {
    let (mut one, mut list) = (params_for(cmd, d), params_for(cmd, d));
    one["id"] = json!(x);
    list["ids"] = json!([x]);
    (one, list)
}

/// [`doc_with_nested_groups`] with point type `Ab` added, nothing selected → its session and the
/// type's id.
fn doc_with_type() -> (Session, [u64; 4], u64) {
    let (mut s, ids) = doc_with_nested_groups();
    let t = s.execute("text.create", &json!({"x": 10, "y": 120, "text": "Ab", "size": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    (s, ids, t)
}

/// What `cmd` gives in a fresh [`doc_with_type`] (nothing selected) with `params`: its result or
/// error, and the document after it.
fn run_fresh(
    cmd: &str,
    params: &serde_json::Value,
) -> (std::result::Result<serde_json::Value, String>, Option<std::sync::Arc<vectorcraft_doc::Document>>) {
    let (mut s, ..) = doc_with_type();
    let r = s.execute(cmd, params).map_err(|e| e.to_string());
    (r, s.doc().ok().map(|d| d.doc.clone()))
}

/// Every command whose params description names both `id` and `ids` (`{ids?|id?}`, `{id?, ids?:
/// … (instead of id)}`), with nothing selected: `id` does what `ids` of that one object does (the
/// same result or the same error, and the same document), for objects of the document and for an
/// id the document doesn't have. Before, the commands that need a selection refused `id` as
/// "nothing selected" and took `ids`.
#[test]
fn id_does_what_ids_of_one_does_with_nothing_selected() {
    let (fresh, [a, inner, _, d], t) = doc_with_type();
    let (mut checked, mut failures, mut refused, mut refused_ran) = (vec![], vec![], vec![], vec![]);
    for spec in command_specs() {
        if !cmd::params::id_stands_for_ids(spec.params) {
            continue;
        }
        // Those the menus refuse with nothing selected are the ones that refused `id`.
        if (spec.enabled)(&fresh).is_err() {
            refused.push(spec.id);
        }
        // A rectangle, a group, type, and an id the document doesn't have.
        for x in [a, inner, t, 999_999] {
            let (one, list) = id_and_ids(spec.id, x, d);
            let (with_id, with_ids) = (run_fresh(spec.id, &one), run_fresh(spec.id, &list));
            if with_id != with_ids {
                failures.push(format!("{} {x}: with id {:.160}; with ids {:.160}", spec.id, format!("{:?}", with_id.0), format!("{:?}", with_ids.0)));
            } else if refused.last() == Some(&spec.id) && x != 999_999 && with_id.0.is_ok() && !refused_ran.contains(&spec.id) {
                refused_ran.push(spec.id);
            }
        }
        checked.push(spec.id);
    }
    assert!(failures.is_empty(), "{} differ:\n{}", failures.len(), failures.join("\n"));
    // The commands that refused `id` with nothing selected (29), and those of them that run on
    // these objects (14: the rest need other art, as with `ids`).
    assert!(refused.len() >= 26 && refused_ran.len() >= 14, "{refused_ran:?} ran of {refused:?}; checked {}", checked.len());
}

/// Every command that takes `id` as `ids` of one ([`cmd::params::id_stands_for_ids`]), with an
/// object selected: an `id` that is malformed (a string, negative, a fraction, an object) or names
/// no object is refused under the command's id, as the same value in `ids` is, and the document
/// and the selection stay as they were. It never stands for the selection instead. (A string `id`
/// given to `effect.apply` without `effect` names the effect, which "bad" isn't: refused too.)
#[test]
fn a_bad_id_is_refused_as_the_same_ids_are_and_changes_nothing() {
    let (_, [.., d], _) = doc_with_type();
    let (mut failures, mut checked) = (vec![], 0);
    for spec in command_specs() {
        if !cmd::params::id_stands_for_ids(spec.params) {
            continue;
        }
        checked += 1;
        for bad in [json!("bad"), json!(-1), json!(1.5), json!({"id": 1}), json!(999_999)] {
            let run = |key: &str, value: serde_json::Value| {
                let (mut s, ..) = doc_with_type();
                s.execute("select.set", &json!({ "ids": [d] })).unwrap();
                let before = s.doc().unwrap().doc.clone();
                let mut p = params_for(spec.id, d);
                p[key] = value;
                let r = s.execute(spec.id, &p).map_err(|e| e.to_string());
                let st = s.doc().unwrap();
                (r, st.doc == before && st.selection.objects == vec![NodeId(d)])
            };
            let (with_id, with_ids) = (run("id", bad.clone()), run("ids", json!([bad.clone()])));
            let effect_named = spec.id == "effect.apply" && bad.is_string();
            if with_id.0.is_ok() || !with_id.1 || (with_id != with_ids && !effect_named) {
                failures.push(format!(
                    "{} {bad}: with id {:.160} (unchanged: {}); with ids {:.160}",
                    spec.id,
                    format!("{:?}", with_id.0),
                    with_id.1,
                    format!("{:?}", with_ids.0)
                ));
            }
        }
        // A step reference no batch resolved is refused under this command's id too.
        let (mut s, ..) = doc_with_type();
        s.execute("select.set", &json!({ "ids": [d] })).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let mut p = params_for(spec.id, d);
        p["id"] = json!("$missing");
        match s.execute(spec.id, &p) {
            Err(EngineError::BadParams { cmd, msg }) if cmd == spec.id && msg.contains("step reference") => {}
            r => failures.push(format!("{} \"$missing\": {r:?}", spec.id)),
        }
        if s.doc().unwrap().doc != before {
            failures.push(format!("{} \"$missing\": the document changed", spec.id));
        }
    }
    assert!(failures.is_empty(), "{} fail:\n{}", failures.len(), failures.join("\n"));
    assert!(checked >= 100, "{checked} commands");
}

/// Every command that names `ids` and is refused with nothing selected: given the object as `ids`
/// it does what it does with the object selected (the same result or the same error, and the same
/// document), so no command is let through to run on an empty selection.
#[test]
fn ids_do_what_the_selection_does_in_every_command_that_takes_them() {
    let mut checked = vec![];
    for spec in command_specs() {
        let (mut given, id) = doc_with_rect();
        if !cmd::params::takes_key(spec.params, "ids") || (spec.enabled)(&given).is_ok() {
            continue;
        }
        let (mut selected, _) = doc_with_rect();
        selected.execute("select.set", &json!({"ids": [id]})).unwrap();
        let (a, b) = (given.execute(spec.id, &json!({"ids": [id]})), selected.execute(spec.id, &json!({})));
        match (&a, &b) {
            (Ok(_), Ok(_)) => {}
            (Err(x), Err(y)) => assert_eq!(x.to_string(), y.to_string(), "{}", spec.id),
            _ => panic!("{}: with ids {a:?}, with the selection {b:?}", spec.id),
        }
        match (given.doc(), selected.doc()) {
            (Ok(g), Ok(s)) => assert!(g.doc == s.doc, "{}: the documents differ", spec.id),
            (g, s) => assert_eq!(g.is_ok(), s.is_ok(), "{}", spec.id),
        }
        checked.push((spec.id, a.is_ok()));
    }
    assert!(checked.iter().filter(|c| c.1).count() >= 20, "{checked:?}");
}

/// Rectangles `a` and `b` grouped in `inner`, `inner` and a third rectangle grouped in `outer`, and
/// a rectangle `d` apart, with nothing selected → `[a, inner, outer, d]`. The document has no
/// creation date.
fn doc_with_nested_groups() -> (Session, [u64; 4]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200, "created": null})).unwrap();
    let rect =
        |s: &mut Session, x: u32| s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let group = |s: &mut Session, ids: [u64; 2]| {
        s.execute("select.set", &json!({ "ids": ids })).unwrap();
        s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut s, 10), rect(&mut s, 40));
    let inner = group(&mut s, [a, b]);
    let c = rect(&mut s, 70);
    let outer = group(&mut s, [inner, c]);
    let d = rect(&mut s, 120);
    s.execute("select.none", &json!({})).unwrap();
    (s, [a, inner, outer, d])
}

/// The params besides `ids` that `cmd` needs to change these objects (`d`: the rectangle apart).
fn params_for(cmd: &str, d: u64) -> serde_json::Value {
    match cmd {
        "object.transform" => json!({"matrix": [1, 0, 0, 1, 10, 0]}),
        "object.distort" => json!({"corners": [[0, 0], [150, 0], [150, 60], [0, 60]]}),
        "object.puppetWarp" => json!({"pins": [[10, 10], [100, 10]], "moved": [[10, 10], [100, 20]]}),
        "appearance.copyFrom" => json!({ "source": d }),
        _ => json!({}),
    }
}

/// `cmd` run with the objects at `picks` of [`doc_with_nested_groups`] as `ids`, and with them
/// selected: whether it succeeded, when both give the same result or the same error and the same
/// document; else what each gave.
fn ids_and_selection(cmd: &str, picks: &[usize]) -> std::result::Result<bool, String> {
    let (mut given, ids) = doc_with_nested_groups();
    let (mut selected, _) = doc_with_nested_groups();
    let list: Vec<u64> = picks.iter().map(|i| ids[*i]).collect();
    selected.execute("select.set", &json!({ "ids": list })).unwrap();
    let params = params_for(cmd, ids[3]);
    let mut with_ids = params.clone();
    with_ids["ids"] = json!(list);
    let (a, b) = (given.execute(cmd, &with_ids), selected.execute(cmd, &params));
    let same_result = match (&a, &b) {
        (Ok(x), Ok(y)) => x == y,
        (Err(x), Err(y)) => x.to_string() == y.to_string(),
        _ => false,
    };
    let same_doc = match (given.doc(), selected.doc()) {
        (Ok(g), Ok(s)) => g.doc == s.doc,
        (g, s) => g.is_ok() == s.is_ok(),
    };
    if same_result && same_doc {
        return Ok(a.is_ok());
    }
    let docs = if same_doc { "" } else { "; the documents differ" };
    Err(format!("{cmd} with ids {list:?}: {:.160}; with them selected: {:.160}{docs}", format!("{a:?}"), format!("{b:?}")))
}

/// Every command whose `ids` stand in for the selection: given an object together with a group that
/// holds it (directly, or inside another group), it does what it does with those objects selected
/// (the same result or the same error, and the same document). `edit::roots_of` drops such an
/// object from the selection and keeps the group.
#[test]
fn ids_of_an_object_and_a_group_that_holds_it_do_what_that_selection_does() {
    let (mut failures, mut succeeded) = (vec![], 0);
    for spec in command_specs() {
        // `outer` and `d` are separate objects. Commands that need `ids`, or whose `ids` are Layers
        // panel rows, images or paths only, differ already there and are left out.
        if !cmd::params::takes_key(spec.params, "ids") || ids_and_selection(spec.id, &[2, 3]).is_err() {
            continue;
        }
        // Each list holds an object and a group around it: `a` and `inner`; `inner`, `a` and `outer`;
        // `a`, `outer` and `d`.
        for picks in [&[0, 1][..], &[1, 0, 2], &[0, 2, 3]] {
            match ids_and_selection(spec.id, picks) {
                Ok(ok) => succeeded += usize::from(ok),
                Err(e) => failures.push(e),
            }
        }
    }
    assert!(failures.is_empty(), "{} differ:\n{}", failures.len(), failures.join("\n"));
    assert!(succeeded >= 150, "only {succeeded} runs succeeded");
}
