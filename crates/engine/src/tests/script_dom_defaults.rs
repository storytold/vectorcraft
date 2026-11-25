//! The script object model's document defaults for new art and `scaleFactor`.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::LineCap;

use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// A 400 × 300 pt document with one rectangle, selected.
fn doc() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    (s, id)
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

fn node(s: &Session, id: NodeId) -> crate::doc::Node {
    s.active().unwrap().doc.node(id).unwrap().clone()
}

#[test]
fn a_wider_default_stroke_goes_to_the_next_path_drawn() {
    let (mut s, picked) = doc();
    let (look, steps) = (node(&s, picked).appearance, undo_steps(&s));
    let wider = s.paint.stroke_width + 1.0;
    let r = run(&mut s, "var d = app.activeDocument; d.defaultStrokeWidth += 1; vc.log(d.defaultStrokeWidth);").unwrap();
    assert_eq!(r["log"][0].as_f64(), Some(wider));
    assert_eq!(undo_steps(&s), steps, "a default alone is no undo step");
    assert_eq!(node(&s, picked).appearance, look, "the selected path kept its stroke");
    assert_eq!(s.new_art().stroke().unwrap().width, wider);
    let out = log(&mut s, "var p = app.activeDocument.pathItems.rectangle(200, 100, 50, 50); vc.log(p.strokeWidth);");
    assert_eq!(out.iter().map(Value::as_f64).collect::<Vec<_>>(), vec![Some(wider)]);
}

#[test]
fn a_bad_default_throws_and_changes_nothing() {
    let (mut s, _) = doc();
    let before = s.paint.clone();
    for (source, name) in [
        ("app.activeDocument.defaultStrokeWidth = -1", "RangeError"),
        ("app.activeDocument.defaultStrokeWidth = 1001", "RangeError"),
        ("app.activeDocument.defaultStrokeWidth = 'wide'", "TypeError"),
        ("app.activeDocument.defaultStrokeMiterLimit = 0", "RangeError"),
        ("app.activeDocument.defaultStrokeCap = 'round'", "TypeError"),
        ("app.activeDocument.defaultStrokeJoin = 1", "TypeError"),
        ("app.activeDocument.defaultStrokeDashes = [0, 0]", "RangeError"),
        ("app.activeDocument.defaultStrokeDashes = '4 2'", "TypeError"),
        ("app.activeDocument.defaultStrokeDashOffset = 3", "Error"),
        ("app.activeDocument.defaultFillColor = 5", "TypeError"),
        ("var c = new RGBColor(); c.red = 300; app.activeDocument.defaultStrokeColor = c", "RangeError"),
        ("app.activeDocument.defaultFilled = 1", "TypeError"),
        ("app.activeDocument.defaultStrokeOverprint = 'yes'", "TypeError"),
    ] {
        let out = log(&mut s, &format!("try {{ {source}; vc.log('ran'); }} catch (e) {{ vc.log(e.name); }}"));
        assert_eq!(out, vec![json!(name)], "{source}");
        assert_eq!(s.paint, before, "{source} changed new art");
    }
}

#[test]
fn default_colors_and_flags_read_back_and_paint_the_next_path() {
    let (mut s, picked) = doc();
    let look = node(&s, picked).appearance;
    let out = log(
        &mut s,
        "var d = app.activeDocument, c = new RGBColor(); c.red = 255;\
         d.defaultFillColor = c; vc.log(d.defaultFillColor.typename); vc.log(d.defaultFillColor.red); vc.log(d.defaultFilled);\
         d.defaultStroked = false; vc.log(d.defaultStroked); vc.log(d.defaultStrokeColor.typename);\
         d.defaultStrokeColor = new GrayColor(); vc.log(d.defaultStroked);\
         d.defaultFillColor = new NoColor(); vc.log(d.defaultFilled); d.defaultFilled = true; vc.log(d.defaultFilled);\
         d.defaultFillColor = c;",
    );
    assert_eq!(out, vec![json!("RGBColor"), json!(255), json!(true), json!(false), json!("NoColor"), json!(true), json!(false), json!(true)]);
    assert_eq!(node(&s, picked).appearance, look);
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 200, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap());
    let ap = node(&s, id).appearance;
    assert_eq!(ap.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(!ap.stroke_paint().is_none());
}

#[test]
fn default_stroke_options_read_back_and_a_same_value_runs_no_command() {
    let (mut s, _) = doc();
    let out = log(
        &mut s,
        "var d = app.activeDocument; d.defaultStrokeCap = StrokeCap.ROUNDENDCAP; d.defaultStrokeJoin = StrokeJoin.BEVELENDJOIN;\
         d.defaultStrokeMiterLimit = 7; d.defaultStrokeDashes = [6, 2]; d.defaultStrokeDashOffset = 1;\
         d.defaultStrokeOverprint = true; d.defaultFillOverprint = true;\
         vc.log([d.defaultStrokeCap, d.defaultStrokeJoin, d.defaultStrokeMiterLimit, d.defaultStrokeDashes, d.defaultStrokeDashOffset,\
         d.defaultStrokeOverprint, d.defaultFillOverprint]);",
    );
    assert_eq!(out, vec![json!(["StrokeCap.ROUNDENDCAP", "StrokeJoin.BEVELENDJOIN", 7, [6, 2], 1, true, true])]);
    let st = s.new_art().stroke().unwrap().clone();
    assert_eq!((st.cap, st.miter_limit, st.overprint), (LineCap::Round, 7.0, true));
    let r = run(
        &mut s,
        "var d = app.activeDocument; d.defaultStrokeCap = StrokeCap.ROUNDENDCAP; d.defaultStrokeMiterLimit = 7;\
         d.defaultStrokeDashes = [6, 2]; d.defaultStrokeOverprint = true; d.defaultStrokeWidth = d.defaultStrokeWidth;\
         d.defaultFillColor = d.defaultFillColor;",
    )
    .unwrap();
    assert_eq!(r["slowestCommands"], json!([]), "the same values run no command");
    // `{}` and `[]` give a solid line.
    assert_eq!(log(&mut s, "var d = app.activeDocument; d.defaultStrokeDashes = {}; vc.log(d.defaultStrokeDashes.length);"), vec![json!(0)]);
}

#[test]
fn a_slightly_different_number_reaches_new_art() {
    let (mut s, _) = doc();
    log(&mut s, "var d = app.activeDocument; d.defaultStrokeWidth = 1; d.defaultStrokeMiterLimit = 7; d.defaultStrokeDashes = [6, 2];");
    log(
        &mut s,
        "var d = app.activeDocument; d.defaultStrokeWidth = 1.0000000005; d.defaultStrokeMiterLimit = 7.0000000005;\
         d.defaultStrokeDashOffset = 0.0000000005;",
    );
    let st = s.new_art().stroke().unwrap().clone();
    assert_eq!(st.width, 1.000_000_000_5);
    assert_eq!(st.miter_limit, 7.000_000_000_5);
    assert_eq!(st.dash.map(|d| d.offset), Some(0.000_000_000_5));
}

#[test]
fn an_rgb_default_in_a_cmyk_document_runs_no_command_the_second_time() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
    let set = "var d = app.activeDocument, c = new RGBColor(); c.red = 200; c.green = 40; c.blue = 90; d.defaultFillColor = c;";
    let first = run(&mut s, set).unwrap();
    assert_ne!(first["slowestCommands"], json!([]), "the first assignment changes new art");
    let again = run(&mut s, set).unwrap();
    assert_eq!(again["slowestCommands"], json!([]), "the same RGB value runs no command");
}

#[test]
fn scale_factor_is_one_and_read_only() {
    let (mut s, _) = doc();
    let out = log(&mut s, "var d = app.activeDocument; vc.log(d.scaleFactor); d.scaleFactor = 4; vc.log(d.scaleFactor);");
    assert_eq!(out, vec![json!(1), json!(1)]);
}

#[test]
fn defaults_refuse_while_new_art_takes_the_selected_look() {
    let (mut s, _) = doc();
    s.execute("appearance.setNewArtBasic", &json!({"on": false})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let before = s.paint.clone();
    let out = log(
        &mut s,
        "try { app.activeDocument.defaultStrokeWidth = 9; } catch (e) { vc.log(e.message.indexOf('New Art Has Basic Appearance') >= 0); }",
    );
    assert_eq!(out, vec![json!(true)]);
    assert_eq!(s.paint, before);
}
