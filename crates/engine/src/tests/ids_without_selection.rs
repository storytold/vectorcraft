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
    let r = s.execute(cmd, params).map(|v| without_plugin_effects(cmd, v)).map_err(|e| e.to_string());
    (r, s.doc().ok().map(|d| d.doc.clone()))
}

/// `cmd`'s result with the plug-in effects (`plugin.<id>`) taken out of `effect.list`'s catalog.
/// The plug-in tests install and remove plug-ins in the process-wide registry while these
/// comparisons run, so two runs of `effect.list` can list different plug-ins.
fn without_plugin_effects(cmd: &str, mut r: serde_json::Value) -> serde_json::Value {
    if cmd == "effect.list"
        && let Some(catalog) = r.get_mut("catalog").and_then(serde_json::Value::as_array_mut)
    {
        catalog.retain(|e| !e["id"].as_str().is_some_and(|id| id.starts_with(vectorcraft_plugins::PREFIX)));
    }
    r
}

/// Whether two results of `cmd` are the same, apart from the plug-in effects in `effect.list`'s
/// catalog ([`without_plugin_effects`]).
fn same_without_plugin_effects(cmd: &str, x: &serde_json::Value, y: &serde_json::Value) -> bool {
    if cmd == "effect.list" { without_plugin_effects(cmd, x.clone()) == without_plugin_effects(cmd, y.clone()) } else { x == y }
}

/// Two `effect.list` results that differ only in a plug-in effect in the catalog compare the same;
/// a difference in a built-in effect, in `applied` or in `activeItem`, or in another command's
/// result, does not.
#[test]
fn plugin_effects_are_left_out_of_the_effect_list_comparison() {
    let list = |catalog: &[&str], applied: serde_json::Value, active: serde_json::Value| {
        let catalog: Vec<_> = catalog.iter().map(|id| json!({"id": id, "label": id})).collect();
        json!({"catalog": catalog, "applied": applied, "activeItem": active})
    };
    let base = list(&["stylize.dropShadow"], json!([]), json!(null));
    let with_plugin = list(&["stylize.dropShadow", "plugin.test.glow"], json!([]), json!(null));
    assert!(same_without_plugin_effects("effect.list", &base, &with_plugin));
    assert!(same_without_plugin_effects("effect.list", &with_plugin, &base));
    assert_eq!(without_plugin_effects("effect.list", with_plugin.clone()), base);
    assert!(!same_without_plugin_effects("effect.list", &base, &list(&[], json!([]), json!(null))), "a built-in effect");
    assert!(!same_without_plugin_effects("effect.list", &base, &list(&["stylize.dropShadow"], json!([{"id": 3}]), json!(null))), "applied");
    assert!(!same_without_plugin_effects("effect.list", &base, &list(&["stylize.dropShadow"], json!([]), json!(0))), "activeItem");
    assert!(!same_without_plugin_effects("object.transform", &base, &with_plugin), "another command");
    assert_eq!(without_plugin_effects("object.transform", with_plugin.clone()), with_plugin);
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
/// no object is refused under the command's id, as the same value in `ids` is (whose error for a
/// malformed entry also names the key and the entry), and the document and the selection stay as
/// they were. It never stands for the selection instead. (A string `id`
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
            let (with_id, mut with_ids) = (run("id", bad.clone()), run("ids", json!([bad.clone()])));
            // The error for an `ids` entry names the key and the entry; the one for `id` names the value.
            let entry = format!("`ids` entry 0 is {bad}, not a non-negative integer object id");
            with_ids.0 = with_ids.0.map_err(|e| e.replace(&entry, &format!("invalid object id {bad}: expected a non-negative integer")));
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
        (Ok(x), Ok(y)) => same_without_plugin_effects(cmd, x, y),
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

/// What `ids` (or `slices`) left out or null gives a command: the selection, or the command's own
/// default.
enum Unset {
    /// The selected objects: a result, the same as the list of the selection gives.
    Selected,
    /// The selected objects, which the command refuses with this error, as it refuses the list of
    /// the selection.
    SelectionRefused(&'static str),
    /// This error, which the list of the selection does not give.
    Fails(&'static str),
    /// This result, which the list of the selection does not give.
    Gives(serde_json::Value),
}

/// The commands that read their `ids` (or `slices`) list themselves or pass it to a colour pass,
/// with the rectangle apart (`d`): the key, the other params, and what leaving the key out gives.
fn ids_cases(d: u64) -> Vec<(&'static str, &'static str, serde_json::Value, Unset)> {
    use Unset::*;
    vec![
        ("appearance.copyFrom", "ids", json!({ "source": d }), Selected),
        ("edit.colors.invert", "ids", json!({}), Selected),
        ("edit.colors.toCMYK", "ids", json!({}), Selected),
        ("edit.colors.toGrayscale", "ids", json!({}), Selected),
        ("edit.colors.toIndexed", "ids", json!({}), Selected),
        ("edit.colors.toRGB", "ids", json!({}), Selected),
        ("edit.colors.saturate", "ids", json!({"intensity": 50}), Selected),
        ("edit.colors.adjustBalance", "ids", json!({"r": 20}), Selected),
        ("paint.invert", "ids", json!({}), Selected),
        ("paint.complement", "ids", json!({}), Selected),
        ("addon.color.filter", "ids", json!({"effect": "addon.color.invert"}), Selected),
        ("addon.color.applyNearestSwatch", "ids", json!({}), Selected),
        ("edit.colors.overprintBlack", "ids", json!({}), Selected),
        ("edit.findReplace", "ids", json!({"find": "A", "replace": "B"}), Gives(json!({"count": 1}))),
        ("layer.highlight", "ids", json!({}), Fails("missing ids")),
        ("layer.move", "ids", json!({ "target": d }), Fails("missing ids")),
        ("library.add", "ids", json!({"kind": "graphic"}), Selected),
        ("library.add", "ids", json!({"kind": "fillColor"}), Selected),
        ("links.check", "ids", json!({}), Selected),
        ("links.update", "ids", json!({}), Selected),
        ("links.relink", "ids", json!({"path": "missing.png"}), Fails("select the images to relink, or pass ids")),
        ("links.embed", "ids", json!({}), Fails("select the linked images or placed documents, or pass ids")),
        ("links.placementOptions", "ids", json!({}), Fails("select the images, or pass ids")),
        ("object.transform", "ids", json!({"matrix": [1, 0, 0, 1, 10, 0]}), Selected),
        ("object.pattern.make", "ids", json!({"name": "P"}), Selected),
        ("object.repeat.radial", "ids", json!({}), Selected),
        ("object.repeat.grid", "ids", json!({}), Selected),
        ("object.repeat.mirror", "ids", json!({}), Selected),
        ("object.slice.make", "ids", json!({}), Selected),
        ("object.slice.move", "slices", json!({"dx": 1, "dy": 1}), Selected),
        ("object.slice.delete", "slices", json!({}), Selected),
        ("object.slice.select", "slices", json!({}), Gives(json!({"selected": []}))),
        ("path.join", "ids", json!({}), SelectionRefused("path is already closed")),
        ("path.averageJoin", "ids", json!({"to": "corner"}), SelectionRefused("select two open path ends, or two open paths, to average and join")),
        ("symbol.edit", "ids", json!({}), Fails("the document has no symbols")),
        ("transparency.makeOpacityMask", "ids", json!({}), Selected),
    ]
}

/// What `cmd` does in a fresh [`doc_with_type`] with `selected` selected (made an object slice
/// first when `slice`): its result or error, the document, the selection and the undo history's
/// length.
fn outcome(
    cmd: &str,
    p: &serde_json::Value,
    selected: &[u64],
    slice: bool,
) -> (std::result::Result<serde_json::Value, String>, vectorcraft_doc::Document, vectorcraft_doc::Selection, usize) {
    let (mut s, ..) = doc_with_type();
    if slice {
        s.execute("object.slice.make", &json!({ "ids": selected })).unwrap();
    }
    s.execute("select.set", &json!({ "ids": selected })).unwrap();
    let r = s.execute(cmd, p).map_err(|e| e.to_string());
    let st = s.doc().unwrap();
    (r, (*st.doc).clone(), st.selection.clone(), st.history.undo.len())
}

/// With the rectangle apart (`d`) selected and with nothing selected, a list with an entry that is
/// not a non-negative integer (a string, true, an object) is refused with an error naming the key,
/// the entry and its value, a value that is not a list with an error naming the key, and the
/// document, the selection and the undo history stay as they were. No entry is dropped. A list of
/// the rectangle is not refused.
#[test]
fn an_ids_entry_that_is_not_an_id_is_refused_and_changes_nothing() {
    let (_, [.., d], _) = doc_with_type();
    let mut failures = vec![];
    for (cmd, key, params, _) in &ids_cases(d) {
        let bads = [json!(["abc"]), json!([d, true]), json!([{}]), json!("abc")];
        for (bad, selected) in bads.iter().flat_map(|b| [(b, true), (b, false)]) {
            // A fresh document each time, so no mode one case enters carries into the next.
            let (mut s, ..) = doc_with_type();
            if selected {
                s.execute("select.set", &json!({ "ids": [d] })).unwrap();
            }
            let st = s.doc().unwrap();
            let before = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
            let mut p = params.clone();
            p[*key] = bad.clone();
            let r = s.execute(cmd, &p);
            let want = match bad.as_array() {
                Some(a) => {
                    let (index, value) = a.iter().enumerate().find(|(_, v)| !v.is_u64()).unwrap();
                    format!("`{key}` entry {index} is {value}, not a non-negative integer object id")
                }
                None => format!("{key} must be an array of non-negative integer object ids"),
            };
            match &r {
                Err(EngineError::BadParams { cmd: c, msg }) if c == cmd && *msg == want => {}
                r => failures.push(format!("{cmd} {key}: {bad} (selected: {selected}): {r:.160?}")),
            }
            let st = s.doc().unwrap();
            if (st.doc.clone(), st.selection.clone(), st.history.undo.len()) != before {
                failures.push(format!("{cmd} {key}: {bad} (selected: {selected}): the document, the selection or the undo history changed"));
            }
        }
        let (mut s, ..) = doc_with_type();
        s.execute("select.set", &json!({ "ids": [d] })).unwrap();
        let mut p = params.clone();
        p[*key] = json!([d]);
        if let Err(EngineError::BadParams { msg, .. }) = s.execute(cmd, &p)
            && (msg.starts_with(&format!("`{key}` entry ")) || msg.starts_with(&format!("{key} must be an array")))
        {
            failures.push(format!("{cmd} {key}: [{d}] is refused: {msg}"));
        }
    }
    assert!(failures.is_empty(), "{} fail:\n{}", failures.len(), failures.join("\n"));
}

/// With the rectangle apart (`d`) selected, the same commands with `ids` (or `slices`) left out and
/// with it null do the same: what the selection's list does for the commands that take the
/// selection, and the command's own default for the others (`object.slice.select` deselects the
/// slices).
#[test]
fn ids_left_out_or_null_take_the_selection_or_the_default() {
    let (_, [.., d], _) = doc_with_type();
    let mut failures = vec![];
    for (cmd, key, params, unset) in ids_cases(d) {
        // The commands on `slices` act on the selected object once it is an object slice.
        let slice = key == "slices";
        let left_out = outcome(cmd, &params, &[d], slice);
        let mut null = params.clone();
        null[key] = serde_json::Value::Null;
        let with_null = outcome(cmd, &null, &[d], slice);
        if with_null != left_out {
            failures.push(format!("{cmd} {key}: null does not do what leaving it out does ({:.160?})", left_out.0));
        }
        let mut listed = params.clone();
        listed[key] = json!([d]);
        let listed = outcome(cmd, &listed, &[d], slice);
        let ok = match &unset {
            Unset::Selected => left_out.0.is_ok() && listed == left_out,
            Unset::SelectionRefused(want) => left_out.0.as_ref().is_err_and(|e| e.ends_with(want)) && listed == left_out,
            Unset::Fails(want) => left_out.0.as_ref().is_err_and(|e| e.ends_with(want)) && listed.0 != left_out.0,
            Unset::Gives(want) => left_out.0.as_ref() == Ok(want) && listed.0 != left_out.0,
        };
        if !ok {
            failures.push(format!("{cmd} {key}: left out: {:.200?}; listed: {:.200?}", left_out.0, listed.0));
        }
    }
    assert!(failures.is_empty(), "{} fail:\n{}", failures.len(), failures.join("\n"));
}

/// The commands that read `ids` through [`cmd::checked_ids_or_selection`] take `ids: null` as left
/// out: with the rectangle apart (`d`) or the type object (`t`) selected, null gives the same
/// result or error, document, selection and undo history as leaving `ids` out. Print Ink Fixes
/// runs in a CMYK document.
#[test]
fn ids_null_takes_the_selection_in_every_command_that_falls_back_to_it() {
    let (_, [.., d], t) = doc_with_type();
    let mut failures = vec![];
    let (rect, both, text) = (&[d][..], &[d, t][..], &[t][..]);
    let cases = [
        ("addon.arrange.randomizeOpacity", json!({"seed": 1}), rect),
        ("addon.arrange.sort", json!({"by": "area", "apply": "x"}), both),
        ("addon.cleanup.simplify", json!({}), rect),
        ("addon.cleanup.deepUngroup", json!({}), rect),
        ("addon.cleanup.rejoin", json!({}), rect),
        ("addon.color.nearestSwatch", json!({}), rect),
        ("addon.pathtools.cornerSet", json!({"radius": 2}), rect),
        ("addon.pathtools.cornerRemove", json!({}), rect),
        ("addon.pathtools.shape.convert", json!({}), rect),
        ("addon.pathtools.erase", json!({"points": [[0, 20], [200, 20]]}), rect),
        ("addon.pathtools.crossings", json!({}), rect),
        ("addon.pathtools.expandDashes", json!({}), rect),
        ("object.path.smooth", json!({}), rect),
        ("object.textWrap.make", json!({}), rect),
        ("object.textWrap.release", json!({}), rect),
        ("object.textWrap.options", json!({}), rect),
        ("variable.bind", json!({"variable": "v"}), rect),
        ("variable.unbind", json!({}), rect),
        ("addon.cleanup.breakText", json!({}), text),
        ("addon.cleanup.combineText", json!({}), text),
        ("addon.cleanup.joinText", json!({}), text),
        ("addon.cleanup.removeBreaks", json!({"kind": "line"}), text),
        ("addon.cleanup.resetTextTransform", json!({}), text),
        ("addon.cleanup.alignInPlace", json!({"align": "left"}), text),
    ];
    for (cmd, params, selected) in cases {
        let left_out = outcome(cmd, &params, selected, false);
        let mut null = params.clone();
        null["ids"] = serde_json::Value::Null;
        let with_null = outcome(cmd, &null, selected, false);
        if with_null != left_out {
            failures.push(format!("{cmd}: null gives {:.160?}; left out gives {:.160?}", with_null.0, left_out.0));
        }
    }
    let print_fixes = |p: &serde_json::Value| {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200, "colorMode": "cmyk", "created": null})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
        let r = s.execute("addon.color.printFixes", p).map_err(|e| e.to_string());
        let st = s.doc().unwrap();
        (r, (*st.doc).clone(), st.selection.clone(), st.history.undo.len())
    };
    let (left_out, with_null) = (print_fixes(&json!({"inkLimit": 200})), print_fixes(&json!({"inkLimit": 200, "ids": null})));
    if with_null != left_out {
        failures.push(format!("addon.color.printFixes: null gives {:.160?}; left out gives {:.160?}", with_null.0, left_out.0));
    }
    assert!(failures.is_empty(), "{} fail:\n{}", failures.len(), failures.join("\n"));
}

/// With no document open, every command that takes `ids` refuses a list with an entry that is not
/// a non-negative integer, and a value that is not a list, with the same error as with a document
/// open (`effect.list` and `links.findFiles` run without a document).
#[test]
fn an_ids_entry_that_is_not_an_id_is_refused_with_no_document() {
    let (mut failures, mut checked) = (vec![], 0);
    for spec in command_specs() {
        if !cmd::params::takes_key(spec.params, "ids") {
            continue;
        }
        checked += 1;
        for bad in [json!(["abc"]), json!([1, true]), json!("abc")] {
            let mut s = Session::new();
            let r = s.execute(spec.id, &json!({ "ids": bad }));
            let want = match bad.as_array() {
                Some(a) => {
                    let (index, value) = a.iter().enumerate().find(|(_, v)| !v.is_u64()).unwrap();
                    format!("`ids` entry {index} is {value}, not a non-negative integer object id")
                }
                None => "ids must be an array of non-negative integer object ids".to_string(),
            };
            match &r {
                Err(EngineError::BadParams { cmd: c, msg }) if c == spec.id && *msg == want => {}
                r => failures.push(format!("{} ids: {bad}: {r:.160?}", spec.id)),
            }
        }
    }
    assert!(failures.is_empty(), "{} fail:\n{}", failures.len(), failures.join("\n"));
    assert!(checked >= 100, "only {checked} commands take ids");
}
