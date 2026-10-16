//! #1575: commands that take `ids` work on the objects given, with nothing selected; the
//! selection is left as it was.

use serde_json::json;

use super::*;

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
