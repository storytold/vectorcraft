//! The script object model's perspective grid members and `app.getPresetSettings`.

use serde_json::{Value, json};

use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The message of the run's failure.
fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

fn grid(s: &mut Session) -> Value {
    s.execute("perspective.grid.get", &json!({})).unwrap()["grid"].clone()
}

#[test]
fn show_and_hide_change_the_view_without_an_undo_step() {
    let mut s = doc();
    let before = undo_steps(&s);
    let out =
        log(&mut s, "var d = app.activeDocument; vc.log(d.hidePerspectiveGrid()); vc.log(d.showPerspectiveGrid()); vc.log(d.showPerspectiveGrid());");
    assert_eq!(out, vec![json!(false), json!(true), json!(true)]);
    assert_eq!(grid(&mut s)["visible"], true);
    assert_eq!(undo_steps(&s), before);
    let out = log(&mut s, "var d = app.activeDocument; vc.log(d.hidePerspectiveGrid()); vc.log(d.hidePerspectiveGrid());");
    assert_eq!(out, vec![json!(true), json!(false)]);
    assert_eq!(grid(&mut s)["visible"], false);
    assert_eq!(undo_steps(&s), before);
}

#[test]
fn grid_view_commands_in_a_run_add_no_undo_step_but_an_edit_still_does() {
    let mut s = doc();
    let before = undo_steps(&s);
    run(&mut s, "vc.run('perspective.grid.show', {visible: true}); vc.run('perspective.plane.set', {plane: 'ground'});").unwrap();
    assert_eq!((grid(&mut s)["visible"].clone(), grid(&mut s)["plane"].clone()), (json!(true), json!("ground")));
    assert_eq!(undo_steps(&s), before);
    run(&mut s, "app.activeDocument.hidePerspectiveGrid(); vc.run('shape.rectangle', {x: 0, y: 0, width: 10, height: 10});").unwrap();
    assert_eq!(undo_steps(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(grid(&mut s)["visible"], true);
}

#[test]
fn a_grid_view_run_after_an_edit_adds_no_undo_step() {
    let mut s = doc();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let before = undo_steps(&s);
    let drawn = s.active().unwrap().doc.node_count();
    run(&mut s, "app.activeDocument.showPerspectiveGrid(); vc.run('perspective.plane.set', {plane: 'ground'});").unwrap();
    assert_eq!((grid(&mut s)["visible"].clone(), grid(&mut s)["plane"].clone()), (json!(true), json!("ground")));
    assert_eq!(undo_steps(&s), before);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.node_count(), drawn - 1);
}

#[test]
fn a_grid_edit_that_changes_only_view_fields_keeps_its_undo_step_in_a_run() {
    let mut s = doc();
    let before = undo_steps(&s);
    s.execute("perspective.grid.set", &json!({"locked": true})).unwrap();
    assert_eq!(undo_steps(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let mut s = doc();
    let source = "vc.run('perspective.grid.set', {locked: true}); app.activeDocument.setActivePlane(PerspectiveGridPlaneType.GRIDFLOORPLANETYPE);";
    run(&mut s, source).unwrap();
    assert_eq!(undo_steps(&s), before + 1);
    assert_eq!((grid(&mut s)["locked"].clone(), grid(&mut s)["plane"].clone()), (json!(true), json!("ground")));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!grid(&mut s).get("locked").and_then(Value::as_bool).unwrap_or(false));
}

#[test]
fn the_active_plane_round_trips_and_a_failed_run_puts_it_back() {
    let mut s = doc();
    let out = log(
        &mut s,
        "var d = app.activeDocument; vc.log(d.getPerspectiveActivePlane());\
         vc.log(d.setActivePlane(PerspectiveGridPlaneType.GRIDFLOORPLANETYPE)); vc.log(d.getPerspectiveActivePlane());\
         d.setActivePlane(PerspectiveGridPlaneType.INVALIDGRIDPLANETYPE); vc.log(d.getPerspectiveActivePlane());",
    );
    let expected = ["PerspectiveGridPlaneType.GRIDLEFTPLANETYPE", "PerspectiveGridPlaneType.GRIDFLOORPLANETYPE"];
    assert_eq!(out[..3], [json!(expected[0]), json!(true), json!(expected[1])]);
    assert_eq!(out[3], json!("PerspectiveGridPlaneType.INVALIDGRIDPLANETYPE"));
    assert_eq!(grid(&mut s)["plane"], "none");
    failure(&mut s, "var d = app.activeDocument; d.setActivePlane(PerspectiveGridPlaneType.GRIDRIGHTPLANETYPE); d.showPerspectiveGrid(); throw 1;");
    assert_eq!(grid(&mut s)["plane"], "none");
    assert_eq!(grid(&mut s)["visible"], false);
}

#[test]
fn a_bad_plane_or_a_right_plane_on_a_one_point_grid_throws() {
    let mut s = doc();
    for bad in ["'left'", "0", "PerspectiveGridType.OnePointPerspectiveGridType", "undefined"] {
        let e = failure(&mut s, &format!("app.activeDocument.setActivePlane({bad});"));
        assert!(e.contains("PerspectiveGridPlaneType"), "{bad}: {e}");
    }
    s.execute("perspective.grid.preset", &json!({"kind": 1})).unwrap();
    let e = failure(&mut s, "app.activeDocument.setActivePlane(PerspectiveGridPlaneType.GRIDRIGHTPLANETYPE);");
    assert!(e.contains("one-point grid"), "{e}");
    assert_eq!(grid(&mut s)["plane"], "left");
    let out = log(
        &mut s,
        "var d = app.activeDocument; d.setActivePlane(PerspectiveGridPlaneType.GRIDFLOORPLANETYPE); vc.log(d.getPerspectiveActivePlane());",
    );
    assert_eq!(out, vec![json!("PerspectiveGridPlaneType.GRIDFLOORPLANETYPE")]);
}

#[test]
fn select_perspective_preset_checks_the_type_and_the_name() {
    let mut s = doc();
    let one = s.execute("perspective.presets.list", &json!({})).unwrap()["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == 1)
        .map(|p| p["name"].as_str().unwrap().to_string())
        .unwrap();
    let before = undo_steps(&s);
    for (call, message) in [
        (format!("PerspectiveGridType.TwoPointPerspectiveGridType, '{one}'"), "not a grid of that type"),
        (format!("PerspectiveGridType.InvalidPerspectiveGridType, '{one}'"), "has no presets"),
        ("PerspectiveGridType.OnePointPerspectiveGridType, 'No Such Preset'".to_string(), "no perspective grid preset called"),
        (format!("'one', '{one}'"), "PerspectiveGridType value"),
        ("PerspectiveGridType.OnePointPerspectiveGridType, 7".to_string(), "presetName must be a string"),
        (format!("PerspectiveGridType.OnePointPerspectiveGridType, '{}'", "x".repeat(5000)), "too long"),
    ] {
        let e = failure(&mut s, &format!("app.activeDocument.selectPerspectivePreset({call});"));
        assert!(e.contains(message), "{call}: {e}");
    }
    assert_eq!(undo_steps(&s), before);
    assert_eq!(grid(&mut s)["kind"], 2);
    let out = log(
        &mut s,
        &format!("vc.log(app.activeDocument.selectPerspectivePreset(PerspectiveGridType.OnePointPerspectiveGridType, '{}'));", one.to_uppercase()),
    );
    assert_eq!(out, vec![json!(true)]);
    let g = grid(&mut s);
    assert_eq!((g["kind"].clone(), g["visible"].clone()), (json!(1), json!(true)));
    assert_eq!(undo_steps(&s), before + 1);
}

#[test]
fn hostile_names_and_numbers_throw_with_nothing_changed() {
    let mut s = doc();
    s.execute("perspective.presets.save", &json!({"name": "Mine"})).unwrap();
    let before = (undo_steps(&s), grid(&mut s));
    for name in ["../Mine", "..\\\\Mine", "/Mine", "C:\\\\Mine", "Mine/..", "Mi\\u0000ne", "~/Mine"] {
        for call in [
            format!("app.activeDocument.selectPerspectivePreset(PerspectiveGridType.TwoPointPerspectiveGridType, '{name}');"),
            format!("app.getPresetSettings('{name}');"),
        ] {
            let e = failure(&mut s, &call);
            assert!(e.contains("no perspective grid preset called") || e.contains("no startup preset called"), "{call}: {e}");
        }
    }
    // The script host refuses a number that is not finite before the member runs.
    for value in ["NaN", "Infinity", "-1e308", "1e308", "2"] {
        let refused = |e: &str, message: &str| e.contains(message) || e.contains("is not a finite number");
        let e = failure(&mut s, &format!("app.activeDocument.selectPerspectivePreset({value}, 'Mine');"));
        assert!(refused(&e, "PerspectiveGridType value"), "{value}: {e}");
        let e = failure(&mut s, &format!("app.activeDocument.setActivePlane({value});"));
        assert!(refused(&e, "PerspectiveGridPlaneType value"), "{value}: {e}");
        let e = failure(&mut s, &format!("app.activeDocument.selectPerspectivePreset(PerspectiveGridType.TwoPointPerspectiveGridType, {value});"));
        assert!(refused(&e, "presetName must be a string"), "{value}: {e}");
    }
    assert_eq!((undo_steps(&s), grid(&mut s)), before);
}

#[test]
fn a_closed_document_throws_on_every_grid_member() {
    let mut s = doc();
    run(&mut s, "#targetengine keep\nvar held = app.activeDocument;").unwrap();
    let e = failure(&mut s, "#targetengine keep\nheld.showPerspectiveGrid();");
    assert!(e.contains("Stale document"), "{e}");
    let e = failure(&mut s, "var d = app.activeDocument; vc.run('file.close', {}); d.hidePerspectiveGrid();");
    assert!(e.contains("Stale document"), "{e}");
    let e = failure(&mut s, "Document.prototype.getPerspectiveActivePlane.call({});");
    assert!(e.contains("not a Document"), "{e}");
}

#[test]
fn get_preset_settings_gives_a_copy_add_document_takes() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "var p = app.getPresetSettings('Basic CMYK'); vc.log(p.typename); vc.log(p.colorMode); vc.log(p.width); vc.log(p.height);\
         vc.log(p.numArtboards); p.width = 100; vc.log(app.getPresetSettings('Basic CMYK').width);\
         var d = app.documents.addDocument('Basic CMYK', p); vc.log(d.width); vc.log(d.documentColorSpace);\
         vc.log(app.getPresetSettings(DocumentPresetType.BasicRGB).colorMode);",
    );
    let expected = [json!("DocumentPreset"), json!("DocumentColorSpace.CMYK"), json!(612), json!(792), json!(1), json!(612)];
    assert_eq!(out[..6], expected);
    assert_eq!(out[6..], [json!(100), json!("DocumentColorSpace.CMYK"), json!("DocumentColorSpace.RGB")]);
    for (call, message) in [("'No Such Preset'", "no startup preset called"), ("3", "startup preset's name"), ("undefined", "startup preset's name")]
    {
        let e = failure(&mut s, &format!("app.getPresetSettings({call});"));
        assert!(e.contains(message), "{call}: {e}");
    }
    let e = failure(&mut s, &format!("app.getPresetSettings('{}');", "x".repeat(5000)));
    assert!(e.contains("too long"), "{e}");
}

#[test]
fn get_preset_settings_of_a_profile_gives_what_add_document_starts_from() {
    let mut s = Session::new();
    for preset in ["Video", "Web", "Mobile", "Print", "DocumentPresetType.Video"] {
        let out = log(
            &mut s,
            &format!(
                "var p = app.getPresetSettings('{preset}'); var d = app.documents.addDocument('{preset}');\
                 vc.log([p.width, p.height, p.units, p.colorMode]); vc.log([d.width, d.height, d.rulerUnits, d.documentColorSpace]);"
            ),
        );
        assert_eq!(out[0], out[1], "{preset}");
    }
    let out = log(&mut s, "var p = app.getPresetSettings('Video'); vc.log([p.width, p.height]);");
    assert_ne!(out[0], json!([612, 792]));
}

#[test]
fn get_preset_settings_given_back_to_add_document_makes_the_same_document() {
    let mut s = Session::new();
    s.execute("prefs.set", &json!({"key": "unitsGeneral", "value": "feet"})).unwrap();
    let layout = json!({"layout": "gridByColumn", "rows": 3, "spacing": 30});
    s.execute("file.newPresets.save", &json!({"name": "Letter", "artboards": 5, "artboardLayout": layout})).unwrap();
    let out = log(
        &mut s,
        "var p = app.getPresetSettings('Print'); vc.log(p.artboardRowsOrCols); vc.log(p.units === undefined);\
         function boards(d) { var r = []; for (var i = 0; i < d.artboards.length; i++) r.push(d.artboards[i].artboardRect); return r; }\
         var a = app.documents.addDocument('Print'); var b = app.documents.addDocument('Print', p);\
         vc.log(boards(a)); vc.log(boards(b)); vc.log(a.rulerUnits === b.rulerUnits);",
    );
    assert_eq!(out[..2], [json!(3), json!(true)]);
    assert_eq!(out[2], out[3]);
    assert_eq!(out[2].as_array().map(Vec::len), Some(5));
    assert_eq!(out[4], json!(true));
    // Settings a DocumentPreset cannot hold are left out, and addDocument keeps the preset's own.
    for (artboards, layout, has_count, has_rows) in [
        (6, json!({"layout": "gridByColumn", "columns": 4}), true, true),
        (5, json!({"layout": "gridByColumn", "columns": 4, "spacing": 7.3}), true, true),
        (6, json!({"layout": "gridByColumn", "columns": 3}), true, true),
        (150, json!({"layout": "gridByRow", "columns": 12}), false, false),
        (7, json!({"layout": "gridByRow", "columns": 3, "rightToLeft": true}), true, true),
    ] {
        s.execute("file.newPresets.save", &json!({"name": "Letter", "artboards": artboards, "artboardLayout": layout})).unwrap();
        let out = log(
            &mut s,
            "var p = app.getPresetSettings('Print'); vc.log(p.numArtboards !== undefined); vc.log(p.artboardRowsOrCols !== undefined);\
             function boards(d) { var r = []; for (var i = 0; i < d.artboards.length; i++) r.push(d.artboards[i].artboardRect); return r; }\
             vc.log(boards(app.documents.addDocument('Print'))); vc.log(boards(app.documents.addDocument('Print', p)));",
        );
        assert_eq!(out[..2], [json!(has_count), json!(has_rows)], "{artboards} {layout}");
        assert_eq!(out[2].as_array().map(Vec::len), Some(artboards), "{layout}");
        assert_eq!(out[2], out[3], "{artboards} {layout}");
    }
}

#[test]
fn startup_presets_list_names_presets_add_document_takes() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "var l = app.startupPresetsList; vc.log(l); l.push('Mine'); l[0] = 'x'; vc.log(app.startupPresetsList.length === l.length - 1);\
         vc.log(app.startupPresetsList[0] !== 'x');\
         for (var i = 0; i < l.length - 1; i++) { var n = app.startupPresetsList[i]; var p = app.getPresetSettings(n);\
         var d = app.documents.addDocument(n); vc.log([n, d.width === p.width && d.documentColorSpace === p.colorMode]); }",
    );
    let names: Vec<&str> = out[0].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    // The New Document profiles in the dialog's order, then the Basic presets.
    let listed: Vec<&str> = crate::cmd::newdoc::profile_names().chain(["Basic CMYK", "Basic RGB"]).collect();
    assert_eq!(names, listed);
    assert_eq!(out[1..3], [json!(true), json!(true)]);
    assert_eq!(out.len(), 3 + names.len());
    for (name, made) in names.iter().zip(&out[3..]) {
        assert_eq!(made, &json!([name, true]), "{name}");
    }
    assert_eq!(s.documents().len(), names.len());
}

#[test]
fn members_not_built_yet_say_so() {
    let mut s = doc();
    for (owner, member) in [
        ("app", "saveWorkspace"),
        ("app", "switchWorkspace"),
        ("app", "deleteWorkspace"),
        ("app", "resetWorkspace"),
        ("app", "loadColorSettings"),
        ("app", "showPresets"),
        ("app.activeDocument", "exportPerspectiveGridPreset"),
        ("app.activeDocument", "importPerspectiveGridPreset"),
    ] {
        let e = failure(&mut s, &format!("{owner}.{member}('x');"));
        assert!(e.contains(&format!("{member} is not available yet")), "{member}: {e}");
    }
}
