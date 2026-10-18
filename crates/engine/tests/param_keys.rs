//! A caller's params are checked against the command's description, but a journal replays as it
//! was recorded (#1442).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_engine::{Session, check_params};

/// `Session::execute` takes what it is given: the journal of an older version, an action recorded
/// with a key a command no longer reads, still plays. The check is the caller's.
#[test]
fn a_journal_with_a_key_no_longer_read_still_replays() {
    let old = [
        ("file.new", json!({"width": 300, "height": 200, "widht": 1})),
        ("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40, "cornerStyle": "round"})),
    ];
    let mut s = Session::new();
    for (id, params) in &old {
        assert!(check_params(id, params).is_err(), "{id}");
        s.execute(id, params).unwrap();
    }
    let st = s.active().unwrap();
    assert_eq!((st.doc.artboards[0].rect.width(), st.doc.layers[0].children().unwrap().len()), (300.0, 1));
}

/// So do an effect's own parameters (#1592): a journal or a file with a key the effect doesn't
/// take, or a value of another type, replays and opens; the effect reads what it knows, clamped.
#[test]
fn effect_params_no_longer_taken_still_replay_and_open() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let old = [
        ("effect.apply", json!({"effect": "blur.gaussian", "params": {"blurr": 3, "radius": "big"}})),
        ("effect.setParams", json!({"index": 0, "params": {"amount": 1e308}})),
    ];
    for (id, params) in &old {
        assert!(s.check_params(id, params).is_err(), "{id}");
        s.execute(id, params).unwrap();
    }
    let doc = &s.active().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    let stored = |d: &vectorcraft_doc::Document| d.layers[0].children().unwrap()[0].appearance.effects[0].params.clone();
    assert_eq!(stored(&back), stored(doc));
    assert_eq!(stored(&back)["blurr"], json!(3));
}

/// An effect's `color` that the effect can't read (a string `Color::from_hex` doesn't read, an
/// `[r, g, b]` without three numbers, an object that isn't a color, `null`) is refused by
/// `effect.apply`, `appearance.addEffect` and `effect.setParams` themselves, not only by the check,
/// and the document and its undo history stay as they were. The forms the effect reads pass. A
/// document saved with an unreadable color still opens with it.
#[test]
fn an_effect_color_it_cannot_draw_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "params": {"color": "#ff0000"}})).unwrap();
    let state = |s: &Session| {
        let st = s.active().unwrap();
        (st.doc.clone(), st.history.undo.len())
    };
    let before = state(&s);
    let why = "`color` must be a #rrggbb color";
    for bad in [json!("not-a-color"), json!("#ff00"), json!([1, 0]), json!(["#ff0000", 0, 0]), json!({"gray": 50}), json!(null)] {
        for (cmd, params) in [
            ("effect.setParams", json!({"index": 0, "params": {"color": bad}})),
            ("effect.apply", json!({"effect": "stylize.outerGlow", "params": {"color": bad}})),
            ("appearance.addEffect", json!({"effect": "adjust.shiftToColor", "params": {"color": bad}})),
        ] {
            let e = s.execute(cmd, &params).unwrap_err().to_string();
            assert!(e.contains(why), "{cmd} {params}: {e}");
            let checked = s.check_params(cmd, &params).unwrap_err().to_string();
            assert!(checked.contains(why), "{cmd} {params}: {checked}");
            assert!(state(&s) == before, "{cmd} {params} changed the document or its undo history");
        }
    }
    let stored = |s: &Session| s.active().unwrap().doc.layers[0].children().unwrap()[0].appearance.effects[0].params["color"].clone();
    for good in [
        json!("#00ff00"),
        json!("#0f0"),
        json!("00ff00"),
        json!("#00ff00ff"),
        json!("green"),
        json!([0, 1, 0]),
        json!({"model": "rgb", "r": 0.0, "g": 1.0, "b": 0.0}),
        json!({"model": "cmyk", "c": 1.0, "m": 0.0, "y": 1.0, "k": 0.0}),
    ] {
        let params = json!({"index": 0, "params": {"color": good}});
        s.check_params("effect.setParams", &params).unwrap_or_else(|e| panic!("{good}: {e}"));
        s.execute("effect.setParams", &params).unwrap_or_else(|e| panic!("{good}: {e}"));
        assert_eq!(stored(&s), good);
    }
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.node_mut(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["color"] = json!("not-a-color");
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.layers[0].children().unwrap()[0].appearance.effects[0].params["color"], json!("not-a-color"));
}

/// A `channel` that Levels or Curves doesn't have (which they drew as RGB) is refused by
/// `effect.apply`, `appearance.addEffect` and `effect.setParams` themselves, not only by the check,
/// and the document and its undo history stay as they were. The channels they read pass. A
/// document saved with another channel still opens with it.
#[test]
fn a_tone_curve_channel_it_does_not_have_is_refused() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("effect.apply", &json!({"effect": "adjust.levels", "params": {"channel": "red"}})).unwrap();
    let state = |s: &Session| {
        let st = s.active().unwrap();
        (st.doc.clone(), st.history.undo.len())
    };
    let before = state(&s);
    let why = "`channel` must be one of rgb, red, green, blue";
    for bad in [json!("alpha"), json!("cyan"), json!(""), json!("reds"), json!(null)] {
        for (cmd, params) in [
            ("effect.setParams", json!({"index": 0, "params": {"channel": bad}})),
            ("effect.apply", json!({"effect": "adjust.curves", "params": {"channel": bad}})),
            ("appearance.addEffect", json!({"effect": "adjust.levels", "params": {"channel": bad}})),
        ] {
            let e = s.execute(cmd, &params).unwrap_err().to_string();
            assert!(e.contains(why), "{cmd} {params}: {e}");
            let checked = s.check_params(cmd, &params).unwrap_err().to_string();
            assert!(checked.contains(why), "{cmd} {params}: {checked}");
            assert!(state(&s) == before, "{cmd} {params} changed the document or its undo history");
        }
    }
    let stored = |s: &Session| s.active().unwrap().doc.layers[0].children().unwrap()[0].appearance.effects[0].params["channel"].clone();
    for good in [json!("rgb"), json!("green"), json!("blue"), json!("Red"), json!("RGB"), json!("r"), json!("G"), json!("b")] {
        let params = json!({"index": 0, "params": {"channel": good}});
        s.check_params("effect.setParams", &params).unwrap_or_else(|e| panic!("{good}: {e}"));
        s.execute("effect.setParams", &params).unwrap_or_else(|e| panic!("{good}: {e}"));
        assert_eq!(stored(&s), good);
    }
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.node_mut(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["channel"] = json!("alpha");
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.layers[0].children().unwrap()[0].appearance.effects[0].params["channel"], json!("alpha"));
}

/// What a command adds to its own journal entry passes the check, so a recorded journal can be
/// sent back through the control channel or MCP.
#[test]
fn a_recorded_journal_passes_the_check() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.scale", &json!({"sx": 150})).unwrap();
    s.execute("command.batch", &json!({"commands": [{"command": "object.move", "params": {"dx": 5, "dy": 0}}, {"command": "edit.copy"}]})).unwrap();
    assert!(s.journal.iter().count() >= 4);
    for (id, params) in s.journal.iter() {
        check_params(id, params).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
    }
}
