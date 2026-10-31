//! The params descriptions name the keys their commands accept (#1442).

use serde_json::json;

use crate::cmd::params::{UNCHECKED, check_effect_params, check_keys, check_params, doc_keys};
use crate::{EngineError, Session, command_specs, find_command};

fn keys(doc: &str) -> Vec<&str> {
    doc_keys(doc).unwrap().keys
}

#[test]
fn a_description_names_its_keys() {
    assert_eq!(keys("{} → {id}"), Vec::<&str>::new());
    assert_eq!(
        keys("{x, y, width?: pt=612|\"210 mm\", sides=6, name?|title?: a name (default: Untitled-N)} → {index}"),
        ["x", "y", "width", "sides", "name", "title"]
    );
    // Brackets, quotes and backticks keep their commas and bars; a nested object isn't listed.
    assert_eq!(
        keys("{at: [x, y] (page, not view), stops: [{position: pt, leader?: \", \"}], csv?: a `|, ` in a label is a break, on?: bool}"),
        ["at", "stops", "csv", "on"]
    );
    // Alternatives after a note are keys when they have their own `?` or `:`; else values.
    assert_eq!(
        keys("{steps?: n | distance?: pt | smooth?: bool, spacing?: smooth|steps|distance, path? | data?: text | dataBase64?}"),
        ["steps", "distance", "smooth", "spacing", "path", "data", "dataBase64"]
    );
    let more = doc_keys("{name, …settings as file.new, …the format's options}").unwrap();
    assert_eq!((more.keys, more.more), (vec!["name"], vec!["settings as file.new", "the format's options"]));
    for bad in ["same as paint.setFill", "{a, b", "{opacity? 0..1}", "{path | name+dataBase64}", "{kind?: Group, Path or Type}"] {
        assert!(doc_keys(bad).is_err(), "{bad}");
    }
}

/// Every command's description reads as a params object, so none is left unchecked by accident.
#[test]
fn every_description_parses() {
    let bad: Vec<String> = command_specs().iter().filter_map(|c| doc_keys(c.params).err().map(|e| format!("{}: {e}", c.id))).collect();
    assert!(bad.is_empty(), "{} descriptions don't parse:\n{}", bad.len(), bad.join("\n"));
    let stale: Vec<_> = UNCHECKED.iter().filter(|id| find_command(id).is_none()).collect();
    assert!(stale.is_empty() && UNCHECKED.is_sorted(), "UNCHECKED: sorted ids of commands; not commands: {stale:?}");
}

fn refusal(id: &str, params: serde_json::Value) -> String {
    match check_params(id, &params) {
        Err(EngineError::BadParams { cmd, msg }) if cmd == id => msg,
        other => panic!("{id} {params}: {other:?}"),
    }
}

#[test]
fn an_unknown_key_is_named_with_the_nearest_accepted_one() {
    assert_eq!(refusal("file.new", json!({"widht": 300})), "unknown parameter `widht`; did you mean `width`?");
    assert_eq!(refusal("file.new", json!({"Width": 300})), "unknown parameter `Width`; did you mean `width`?");
    assert_eq!(refusal("select.all", json!({"zzz": 1})), "unknown parameter `zzz`; the command takes no parameters");
    assert_eq!(
        refusal("shape.line", json!({"x1": 0, "y1": 0, "x2": 5, "y2": 5, "colour": "red"})),
        "unknown parameter `colour`; the command takes: x1, y1, x2, y2, fill"
    );
    for (id, ok) in [
        ("file.new", json!({"width": 300, "title": "a", "artboardLayout": {"anything": 1}})),
        ("file.new", json!({})),
        ("file.new", json!(null)),
        // Keys listed by reference, and any key where the description says so.
        ("file.newPresets.save", json!({"name": "a", "width": 300})),
        ("document.export", json!({"format": "png", "anyFormatOption": 1})),
        // Not a command: running it says so.
        ("no.such", json!({"x": 1})),
    ] {
        assert!(check_params(id, &ok).is_ok(), "{id} {ok}");
    }
    assert!(check_keys("x", "{a, …as file.new}", &json!({"a": 1, "width": 2})).is_ok());
    assert!(check_keys("x", "{a, …as file.new}", &json!({"b": 1})).is_err());
}

/// A command that takes the keys of one that isn't checked yet isn't checked either.
#[test]
fn a_reference_to_an_unchecked_command_is_unchecked() {
    assert!(UNCHECKED.contains(&"paint.setFill") && !UNCHECKED.contains(&"paint.setStroke"), "pick another pair");
    assert!(check_params("paint.setStroke", &json!({"zzz": 1})).is_ok());
    assert!(check_keys("x", "{a, …as paint.setFill}", &json!({"zzz": 1})).is_ok());
}

#[test]
fn a_batch_checks_its_steps() {
    let step = |params| json!({"commands": [{"command": "select.all"}, {"command": "file.new", "params": params}]});
    assert!(check_params("command.batch", &step(json!({"width": 300}))).is_ok());
    match check_params("command.batch", &step(json!({"widht": 300}))) {
        Err(EngineError::BadParams { cmd, msg }) => {
            assert_eq!((cmd.as_str(), msg.as_str()), ("file.new", "unknown parameter `widht`; did you mean `width`?"))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(refusal("command.batch", json!({"steps": []})), "unknown parameter `steps`; the command takes: label, commands, artboard");
}

/// #1592: the `params` of an effect are held to the effect's own keys and types, where the effect
/// is named (`effect.apply`) and where the document says which it is (`effect.setParams`).
#[test]
fn effect_params_are_the_effects_own() {
    use vectorcraft_render::effects::effect_catalog;
    for e in effect_catalog() {
        doc_keys(e.params).unwrap_or_else(|why| panic!("{}: {why}", e.id));
        check_effect_params("effect.apply", e.id, &e.defaults).unwrap_or_else(|why| panic!("{}'s defaults: {why}", e.id));
    }
    let apply = |effect: &str, params| json!({"effect": effect, "params": params});
    assert_eq!(refusal("effect.apply", apply("blur.gaussian", json!({"blurr": 3}))), "unknown parameter `blurr`; the command takes: radius");
    assert_eq!(
        refusal("appearance.addEffect", apply("stylize.dropShadow", json!({"opacty": 50}))),
        "unknown parameter `opacty`; did you mean `opacity`?"
    );
    assert_eq!(refusal("effect.apply", apply("blur.gaussian", json!({"radius": "big"}))), "`radius` must be a number");
    assert_eq!(refusal("effect.apply", apply("threeD.revolve", json!({"shade": 1}))), "`shade` must be true or false");
    assert_eq!(refusal("effect.apply", apply("threeD.revolve", json!({"edge": 1}))), "`edge` must be a string");
    // Out of range is the effect's to clamp; a key only its description names is one it takes;
    // a plug-in's are its manifest's; an unknown effect is the command's to report.
    for (effect, params) in [
        ("stylize.dropShadow", json!({"opacity": 500, "x": -3, "color": "#ff0000"})),
        ("stylize.dropShadow", json!({"color": [1, 0, 0]})),
        ("threeD.revolve", json!({"segments": 100000})),
        ("path.outlineStroke", json!({"width": 2})),
        ("plugin.anything", json!({"whatever": 1})),
        ("no.such", json!({"whatever": 1})),
    ] {
        assert!(check_params("effect.apply", &apply(effect, params.clone())).is_ok(), "{effect} {params}");
    }
    // `effect.setParams` names an index: the session knows the effect there.
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("effect.apply", &apply("blur.gaussian", json!({"radius": 2}))).unwrap();
    for (params, why) in [(json!({"blurr": 3}), "unknown parameter `blurr`"), (json!({"radius": true}), "`radius` must be a number")] {
        let e = s.execute("effect.setParams", &json!({"index": 0, "params": params})).unwrap_err().to_string();
        assert!(e.contains(why), "{e}");
    }
    s.execute("effect.setParams", &json!({"index": 0, "params": {"radius": 1e9}, "visible": false})).unwrap();
    // No effect at the index: the command says so, not the check.
    assert!(s.check_params("effect.setParams", &json!({"index": 7, "params": {"blurr": 3}})).is_ok());
}

/// The Transform effect (`distort.transform`) is held to its params description: `effect.apply`
/// and `effect.setParams` refuse each of its on/off options given as text, such as `reflectX:
/// "false"`, and each of its numbers given as text, such as `copies: "0"`, and leave the effect
/// as it was; true, false and numbers pass. The loop takes the keys from the effect's defaults, so
/// an option the effect gains is checked too.
#[test]
fn transform_effect_refuses_a_boolean_or_number_given_as_text() {
    let info = vectorcraft_render::effects::effect_info("distort.transform").unwrap();
    let one = |key: &str, v: serde_json::Value| serde_json::Value::Object([(key.to_string(), v)].into_iter().collect());
    let apply = |params| json!({"effect": "distort.transform", "params": params});
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("effect.apply", &apply(json!({"moveH": 20}))).unwrap();
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (st.history.undo.len(), st.doc.layers[0].children().unwrap()[0].appearance.effects[0].params.clone())
    };
    let before = state(&s);
    let mut switches = vec![];
    for (key, default) in info.defaults.as_object().unwrap() {
        let why = match default {
            serde_json::Value::Bool(_) => format!("`{key}` must be true or false"),
            serde_json::Value::Number(_) => format!("`{key}` must be a number"),
            _ => continue,
        };
        if default.is_boolean() {
            switches.push(key.as_str());
        }
        let text = json!(default.to_string());
        assert_eq!(refusal("effect.apply", apply(one(key, text.clone()))), why);
        let e = s.execute("effect.setParams", &json!({"index": 0, "params": one(key, text)})).unwrap_err().to_string();
        assert!(e.contains(&why), "{e}");
        assert!(check_params("effect.apply", &apply(one(key, default.clone()))).is_ok(), "{key}");
    }
    assert!(["reflectX", "reflectY", "random"].iter().all(|k| switches.contains(k)), "{switches:?}");
    assert_eq!(state(&s), before);
    s.execute("effect.setParams", &json!({"index": 0, "params": {"reflectX": true, "random": false, "copies": 2}})).unwrap();
}

/// What a command notes for its journal entry is a key its description names, so the entry
/// passes the check a caller's params get.
#[test]
fn journal_entries_pass_the_check() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.scale", &json!({"sx": 150})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 100, "text": "a"})).unwrap();
    assert!(s.journal.iter().count() >= 5);
    for (id, params) in s.journal.iter() {
        assert!(check_params(id, params).is_ok(), "{id} {params}");
    }
}
