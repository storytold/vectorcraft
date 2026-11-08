//! `Session::execute` checks params against the command's description, but a journal replays as
//! it was recorded (#1442).

use serde_json::json;
use vectorcraft_engine::{Session, check_params};

/// The open document's art and undo history, and the session's journal length.
fn state(s: &Session) -> (Option<vectorcraft_doc::Document>, usize, usize) {
    let st = s.active();
    (st.map(|st| (*st.doc).clone()), st.map_or(0, |st| st.history.undo.len()), s.journal.len())
}

/// `Session::execute` answers a key the command's description doesn't name with a parameter
/// error, in a release build too, and the document, its undo history and the journal stay as
/// they were.
#[test]
fn execute_refuses_a_key_its_command_does_not_name() {
    let mut s = Session::new();
    let e = s.execute("file.new", &json!({"width": 300, "widht": 1})).unwrap_err().to_string();
    assert!(e.contains("widht"), "{e}");
    assert!(s.active().is_none());
    s.execute("file.new", &json!({})).unwrap();
    let before = state(&s);
    let e = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40, "cornerStyle": "round"})).unwrap_err();
    assert!(e.to_string().contains("cornerStyle"), "{e}");
    assert!(state(&s) == before);
}

/// A step of a `command.batch` with such a key fails the batch before any step runs, and a step
/// whose `effect.setParams` gives an effect a parameter it doesn't take fails it too, though that
/// effect is only there once the steps before it ran. Nothing the batch did stays.
#[test]
fn execute_refuses_a_batch_step_with_a_key_its_command_does_not_name() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let before = state(&s);
    let rect = json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 50, "height": 40}});
    let bad_step = json!({"commands": [rect, {"command": "object.move", "params": {"dx": 5, "dyy": 5}}]});
    let e = s.execute("command.batch", &bad_step).unwrap_err().to_string();
    assert!(e.contains("dyy"), "{e}");
    assert!(state(&s) == before);
    let bad_effect = json!({"commands": [
        rect,
        {"command": "effect.apply", "params": {"effect": "blur.gaussian"}},
        {"command": "effect.setParams", "params": {"index": 0, "params": {"blurr": 3}}},
    ]});
    assert!(check_params("command.batch", &bad_effect).is_ok());
    let e = s.execute("command.batch", &bad_effect).unwrap_err().to_string();
    assert!(e.contains("blurr"), "{e}");
    assert!(state(&s) == before);
}

/// The steps of a `command.batch` step aren't checked, however deep they go: batches can't nest,
/// which running the batch reports.
#[test]
fn the_steps_of_a_nested_batch_are_not_checked() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut batch = json!({"commands": [{"command": "object.move", "params": {"dyy": 5}}]});
    assert!(check_params("command.batch", &batch).is_err());
    for _ in 0..3 {
        batch = json!({"commands": [{"command": "command.batch", "params": batch}]});
    }
    assert!(check_params("command.batch", &batch).is_ok());
    let e = s.execute("command.batch", &batch).unwrap_err().to_string();
    assert!(e.contains("batches cannot nest"), "{e}");
}

/// An `action.batch` with a step that has such a key fails before it opens a source file or runs
/// a step.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn execute_refuses_an_action_batch_step_with_a_key_its_command_does_not_name() {
    let dir = std::env::temp_dir().join(format!("vc-param-keys-action-batch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="10" height="10"/></svg>"#;
    std::fs::write(dir.join("a.svg"), svg).unwrap();
    let mut s = Session::new();
    let batch = json!({
        "source": "folder",
        "folder": dir.to_string_lossy(),
        "commands": [{"command": "select.all", "params": {}}, {"command": "object.move", "params": {"dx": 5, "dyy": 5}}],
    });
    let e = s.execute("action.batch", &batch).unwrap_err().to_string();
    std::fs::remove_dir_all(&dir).ok();
    assert!(e.contains("dyy"), "{e}");
    assert!(s.documents().is_empty());
    assert!(s.journal.is_empty());
}

/// `Session::replay` takes what it is given: the journal of an older version, an action recorded
/// with a key a command no longer reads, still plays, as a step of a `command.batch` too.
#[test]
fn a_journal_with_a_key_no_longer_read_still_replays() {
    let old = [
        ("file.new", json!({"width": 300, "height": 200, "widht": 1})),
        ("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40, "cornerStyle": "round"})),
        ("command.batch", json!({"commands": [{"command": "object.move", "params": {"dx": 5, "dy": 5, "dyy": 5}}]})),
    ];
    let mut s = Session::new();
    for (id, params) in &old {
        assert!(check_params(id, params).is_err(), "{id}");
        s.replay(id, params).unwrap();
    }
    assert!(!s.replaying());
    let st = s.active().unwrap();
    assert_eq!((st.doc.artboards[0].rect.width(), st.doc.layers[0].children().unwrap().len()), (300.0, 1));
    assert_eq!(s.journal.len(), 3);
    assert!(s.execute("object.move", &json!({"dx": 5, "dyy": 5})).is_err());
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
        assert!(s.execute(id, params).is_err(), "{id}");
        s.replay(id, params).unwrap();
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
