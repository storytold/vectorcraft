//! The script object model's swatches, color groups, spots, symbols, graphic styles and brushes.

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

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

/// A 400 × 300 pt RGB document with one rectangle.
fn doc() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "rgb"})).unwrap();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    s.execute("select.none", &json!({})).unwrap();
    (s, a)
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

fn swatch_names(s: &mut Session) -> Vec<String> {
    let l = s.execute("swatch.list", &json!({})).unwrap();
    l["swatches"].as_array().unwrap().iter().map(|w| w["name"].as_str().unwrap().to_string()).collect()
}

/// The document's resources, to show that a failed call changed nothing.
fn resources(s: &Session) -> String {
    let d = &s.active().unwrap().doc;
    format!("{:?}{:?}{:?}{:?}{:?}", *d.swatches, *d.swatch_groups, *d.graphic_styles, d.symbols, d.layers)
}

#[test]
fn swatches_are_live_and_keep_reserved_entries() {
    let (mut s, _) = doc();
    let names = swatch_names(&mut s);
    let out = log(
        &mut s,
        "var w = app.activeDocument.swatches; vc.log(w.length); vc.log(w.typename); vc.log(w.parent.typename);\
         vc.log(w[0].name); vc.log(w[1].name); vc.log(w[1].typename); vc.log(w[w.length] === undefined);\
         vc.log(w.getByName('Black').color.typename); vc.log(w.getSelected().length);",
    );
    assert_eq!(
        out,
        vec![
            json!(names.len()),
            json!("Swatches"),
            json!("Document"),
            json!(names[0]),
            json!("[Registration]"),
            json!("Swatch"),
            json!(true),
            json!("RGBColor"),
            json!(0)
        ]
    );
    let out = log(
        &mut s,
        "var w = app.activeDocument.swatches, n = w.add(); n.name = 'Script Ink';\
         var c = new RGBColor(); c.red = 10; c.green = 20; c.blue = 30; n.color = c;\
         vc.log(w.getByName('Script Ink').color.green); vc.log(n.name);",
    );
    assert_eq!(out, vec![json!(20), json!("Script Ink")]);
    assert!(swatch_names(&mut s).contains(&"Script Ink".to_string()));
    let steps = undo_steps(&s);
    log(&mut s, "app.activeDocument.swatches.removeAll();");
    assert_eq!(undo_steps(&s), steps + 1);
    let left = swatch_names(&mut s);
    assert_eq!(left, vec![names[0].clone(), "[Registration]".to_string()]);
    for script in ["app.activeDocument.swatches[1].name = 'R';", "app.activeDocument.swatches[0].remove();"] {
        assert!(failure(&mut s, script).contains("is built in"), "{script}");
    }
    let steps = undo_steps(&s);
    log(&mut s, "app.activeDocument.swatches.removeAll();");
    assert_eq!(undo_steps(&s), steps, "a removeAll with nothing to remove records no step");
}

#[test]
fn swatches_add_saves_a_gradient_fill_as_a_gradient() {
    let (mut s, _) = doc();
    let ramp = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
    s.execute("paint.setFill", &json!({"gradient": ramp})).unwrap();
    let out = log(&mut s, "vc.log(app.activeDocument.swatches.add().color.typename);");
    assert_eq!(out, vec![json!("GradientColor")]);
}

#[test]
fn resource_objects_go_stale_after_remove_and_follow_renames() {
    let (mut s, _) = doc();
    let out = log(
        &mut s,
        "var w = app.activeDocument.swatches.getByName('Black'); w.name = 'Ink'; vc.log(w.name);\
         var again = app.activeDocument.swatches.getByName('Ink'); vc.log(again.name);",
    );
    assert_eq!(out, vec![json!("Ink"), json!("Ink")]);
    let cases = [
        ("var w = app.activeDocument.swatches.getByName('Ink'); w.remove(); w.name;", "Stale Swatch"),
        ("var w = app.activeDocument.swatches.getByName('White'); w.remove(); w.typename;", "Stale Swatch"),
        ("var w = app.activeDocument.swatches.getByName('White'); vc.run('swatch.delete', {name: 'White'}); w.color;", "Stale Swatch"),
        (
            "var g = app.activeDocument.graphicStyles.getByName('Default Graphic Style'); vc.run('graphicStyle.rename', {name: g.name, to: 'X'}); g.name;",
            "Stale GraphicStyle",
        ),
        ("var w = app.activeDocument.swatches.getByName('Red'); w.remove(); app.activeDocument.swatches.add().name = 'Red'; w.name;", "Stale Swatch"),
        (
            "var w = app.activeDocument.swatches.getByName('Lime'); vc.run('swatch.delete', {name: 'Lime'}); vc.run('swatch.new', {name: 'Lime', color: '#00ff00'}); w.name;",
            "Stale Swatch",
        ),
    ];
    for (script, message) in cases {
        let e = failure(&mut s, script);
        assert!(e.contains(message), "{script}: {e}");
    }
}

#[test]
fn hostile_names_and_values_are_refused_without_change() {
    let (mut s, _) = doc();
    let before = resources(&s);
    let steps = undo_steps(&s);
    let long = "x".repeat(1025);
    let cases = [
        ("app.activeDocument.swatches.getByName('Black').name = 'a\\u0007b';".to_string(), "control characters"),
        ("app.activeDocument.swatches.getByName('Black').name = '   ';".to_string(), "cannot be empty"),
        (format!("app.activeDocument.swatches.getByName('Black').name = '{long}';"), "longer than 1024 bytes"),
        ("app.activeDocument.swatches.getByName('Black').name = 7;".to_string(), "must be a string"),
        ("app.activeDocument.swatches.getByName('Black').name = 'White';".to_string(), "already exists"),
        ("var d = app.activeDocument, p = d.spots.add(); p.name = 'Black';".to_string(), "a swatch or color group named Black already exists"),
        (
            "var d = app.activeDocument; d.swatchGroups.add().name = 'Mine'; d.swatches.getByName('White').name = 'Mine';".to_string(),
            "a swatch or color group named Mine already exists",
        ),
        ("app.activeDocument.swatches.getByName('Black').color = new NoColor();".to_string(), "cannot be NoColor"),
        ("app.activeDocument.swatches.getByName('Black').color = {typename: 'RGBColor', red: 300};".to_string(), "from 0 to 255"),
        ("app.activeDocument.swatches.getByName('nope');".to_string(), "Not found"),
        ("app.activeDocument.swatches.index({});".to_string(), "name or an index"),
        ("app.activeDocument.symbols.add(app.activeDocument);".to_string(), "not a page item"),
        ("app.activeDocument.symbols.add(app.activeDocument.pageItems[0], 'SymbolRegistrationPoint.NOPE');".to_string(), "SymbolRegistrationPoint"),
        ("app.activeDocument.symbolItems.add(app.activeDocument.swatches[2]);".to_string(), "not a Symbol"),
        ("app.activeDocument.graphicStyles[0].applyTo([app.activeDocument.pageItems[0]]);".to_string(), "one page item"),
    ];
    for (script, message) in &cases {
        let e = failure(&mut s, script);
        assert!(e.contains(message), "{script}: {e}");
    }
    assert_eq!(resources(&s), before);
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn spots_keep_their_kind_and_their_color_space() {
    let (mut s, _) = doc();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.spots.add(); p.name = 'Pea'; p.colorType = ColorModel.SPOT;\
         var c = new RGBColor(); c.red = 90; c.green = 200; c.blue = 30; p.color = c;\
         vc.log(d.spots.length); vc.log(p.typename); vc.log(p.colorType); vc.log(p.spotKind); vc.log(p.getInternalColor());\
         var w = d.swatches.getByName('Pea').color; vc.log(w.typename); vc.log(w.spot.name); vc.log(w.tint);",
    );
    assert_eq!(
        out,
        vec![
            json!(2),
            json!("Spot"),
            json!("ColorModel.SPOT"),
            json!("SpotColorKind.SPOTRGB"),
            json!([90, 200, 30]),
            json!("SpotColor"),
            json!("Pea"),
            json!(100)
        ]
    );
    let before = resources(&s);
    for (script, message) in [
        ("var p = app.activeDocument.spots.getByName('Pea'); p.color = new GrayColor();", "must be an RGBColor, a CMYKColor or a LabColor"),
        ("app.activeDocument.spots.getByName('Pea').colorType = ColorModel.PROCESS;", "stays a spot color"),
        ("app.activeDocument.spots.getByName('Pea').colorType = 3;", "ColorModel value"),
    ] {
        let e = failure(&mut s, script);
        assert!(e.contains(message), "{script}: {e}");
    }
    assert_eq!(resources(&s), before);
    log(&mut s, "var p = app.activeDocument.spots.getByName('Pea'); var l = new LabColor(); l.l = 50; p.color = l;");
    assert_eq!(log(&mut s, "vc.log(app.activeDocument.spots.getByName('Pea').spotKind);"), vec![json!("SpotColorKind.SPOTLAB")]);
    log(&mut s, "app.activeDocument.spots.removeAll();");
    assert!(!swatch_names(&mut s).contains(&"Pea".to_string()));
}

#[test]
fn a_spot_takes_a_cmyk_color_in_an_rgb_document() {
    let (mut s, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.spots.add(); p.name = 'Pea'; var c = new CMYKColor(); c.cyan = 35; c.yellow = 50; p.color = c;\
         var q = app.activeDocument.spots.getByName('Pea'); vc.log(q.color.typename); vc.log([q.color.cyan, q.color.yellow]); vc.log(q.spotKind);",
    );
    assert_eq!(out, vec![json!("CMYKColor"), json!([35, 50]), json!("SpotColorKind.SPOTCMYK")]);
    let d = &s.active().unwrap().doc;
    let cmyk =
        matches!(d.swatch("Pea").map(|w| &w.paint), Some(vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Cmyk { .. }, .. }));
    assert!(cmyk, "the spot keeps its CMYK color in the RGB document");
    let steps = undo_steps(&s);
    log(&mut s, "var p = app.activeDocument.spots.getByName('Pea'); p.color = p.color;");
    assert_eq!(undo_steps(&s), steps, "assigning the color the spot has records no step");
}

#[test]
fn a_spot_keeps_an_rgb_color_in_a_cmyk_document() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 80, "m": 40, "y": 0, "k": 0}, "spot": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Leaf", "color": {"c": 60, "m": 0, "y": 90, "k": 10}})).unwrap();
    let rgb = |s: &Session, name: &str| {
        matches!(
            s.active().unwrap().doc.swatch(name).map(|w| &w.paint),
            Some(vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Rgb { .. }, .. })
        )
    };
    let assign = |what: &str| format!("var c = new RGBColor(); c.red = 200; c.green = 60; c.blue = 20; {what}.color = c;");
    for (what, name) in [("app.activeDocument.spots.getByName('Pea')", "Pea"), ("app.activeDocument.swatches.getByName('Ink')", "Ink")] {
        if name == "Pea" {
            log(&mut s, "app.activeDocument.spots.add().name = 'Pea';");
        }
        let out = log(&mut s, &format!("{} var q = app.activeDocument.spots.getByName('{name}').color; vc.log([q.typename, q.red]);", assign(what)));
        assert_eq!(out, vec![json!(["RGBColor", 200])], "{what}");
        assert!(rgb(&s, name), "{name} keeps its RGB color in the CMYK document");
        let kind = log(&mut s, &format!("vc.log(app.activeDocument.spots.getByName('{name}').spotKind);"));
        assert_eq!(kind, vec![json!("SpotColorKind.SPOTRGB")], "{name}");
        let steps = undo_steps(&s);
        log(&mut s, &assign(what));
        assert_eq!(undo_steps(&s), steps, "assigning {name} the color it has records no step");
    }
    let what = "app.activeDocument.swatches.getByName('Leaf')";
    log(&mut s, &assign(what));
    assert!(!rgb(&s, "Leaf"), "a process swatch stores the RGB color as CMYK in the CMYK document");
    let steps = undo_steps(&s);
    log(&mut s, &assign(what));
    assert_eq!(undo_steps(&s), steps, "assigning Leaf the RGB color it was given records no step");
}

#[test]
fn spots_list_registration_and_keep_it() {
    let (mut s, _) = doc();
    let out = log(
        &mut s,
        "var d = app.activeDocument, r = d.spots.getByName('[Registration]'); vc.log(r.typename); vc.log(r.colorType);\
         vc.log(d.spots[0].name); vc.log(d.spots.length); r.colorType = ColorModel.REGISTRATION;\
         vc.log(d.swatches.getByName('[Registration]').color.spot.name);",
    );
    assert_eq!(out, vec![json!("Spot"), json!("ColorModel.REGISTRATION"), json!("[Registration]"), json!(1), json!("[Registration]")]);
    let before = resources(&s);
    let steps = undo_steps(&s);
    for script in [
        "app.activeDocument.spots.getByName('[Registration]').name = 'R';",
        "app.activeDocument.spots.getByName('[Registration]').remove();",
        "app.activeDocument.spots.getByName('[Registration]').color = new CMYKColor();",
        "app.activeDocument.spots.getByName('[Registration]').colorType = ColorModel.SPOT;",
    ] {
        assert!(failure(&mut s, script).contains("is built in"), "{script}");
    }
    log(&mut s, "app.activeDocument.spots.removeAll();");
    assert_eq!(resources(&s), before);
    assert_eq!(undo_steps(&s), steps, "removeAll with only Registration left records no step");
    assert_eq!(log(&mut s, "vc.log(app.activeDocument.spots.length);"), vec![json!(1)]);
}

#[test]
fn a_tint_of_a_spot_reads_as_that_spot_at_the_tint() {
    let (mut s, _) = doc();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#204080", "spot": true})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let tint = s.execute("swatch.new", &json!({"tint": 40})).unwrap()["name"].as_str().unwrap().to_string();
    let out = log(
        &mut s,
        &format!("var c = app.activeDocument.swatches.getByName('{tint}').color; vc.log(c.typename); vc.log(c.spot.name); vc.log(c.tint);"),
    );
    assert_eq!(out, vec![json!("SpotColor"), json!("Ink"), json!(40)]);
}

#[test]
fn spots_add_converts_the_fill_to_the_document_color_space() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000", "keepModel": true})).unwrap();
    let rgb = matches!(s.new_art().fill_paint(), vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Rgb { .. }, .. });
    assert!(rgb, "the current fill is RGB in a CMYK document");
    let out = log(&mut s, "var p = app.activeDocument.spots.add(); vc.log(p.spotKind); vc.log(p.color.typename);");
    assert_eq!(out, vec![json!("SpotColorKind.SPOTCMYK"), json!("CMYKColor")]);
}

#[test]
fn spots_add_converts_a_cmyk_or_gray_fill_in_an_rgb_document() {
    for fill in [json!({"c": 0, "m": 100, "y": 100, "k": 0}), json!({"gray": 40})] {
        let (mut s, _) = doc();
        s.execute("paint.setFill", &json!({"color": fill})).unwrap();
        let rgb = matches!(s.new_art().fill_paint(), vectorcraft_color::Paint::Solid { color: vectorcraft_color::Color::Rgb { .. }, .. });
        assert!(!rgb, "the current fill {fill} is not RGB in an RGB document");
        let out = log(
            &mut s,
            "var p = app.activeDocument.spots.add(); vc.log(p.spotKind); vc.log(p.color.typename); p.color = p.color; vc.log(p.name == '');",
        );
        assert_eq!(out, vec![json!("SpotColorKind.SPOTRGB"), json!("RGBColor"), json!(false)], "{fill}");
    }
}

#[test]
fn color_groups_hold_solid_swatches_only() {
    let (mut s, _) = doc();
    s.execute(
        "swatch.new",
        &json!({"name": "Ramp", "gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]}}),
    )
    .unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, g = d.swatchGroups.add(); g.name = 'Script Group';\
         g.addSwatch(d.swatches.getByName('Black')); g.addSwatch(d.swatches.getByName('Black'));\
         var all = g.getAllSwatches(); vc.log(all.length); vc.log(all[0].name); vc.log(d.swatchGroups.getByName('Script Group').typename);",
    );
    assert_eq!(out, vec![json!(1), json!("Black"), json!("SwatchGroup")]);
    let before = resources(&s);
    for (script, message) in [
        ("var d = app.activeDocument; d.swatchGroups.getByName('Script Group').addSwatch(d.swatches.getByName('Ramp'));", "solid colors only"),
        ("var d = app.activeDocument; d.swatchGroups.getByName('Script Group').addSwatch(d.swatches[0]);", "is built in"),
        ("var d = app.activeDocument; d.swatchGroups.getByName('Script Group').addSpot(d.swatches.getByName('Black'));", "not a Spot"),
    ] {
        let e = failure(&mut s, script);
        assert!(e.contains(message), "{script}: {e}");
    }
    assert_eq!(resources(&s), before);
    let out = log(
        &mut s,
        "var d = app.activeDocument, g = d.swatchGroups.getByName('Script Group'), b = g.getAllSwatches()[0];\
         g.removeAll(); vc.log(g.getAllSwatches().length); try { b.name; } catch (e) { vc.log(String(e.message)); }\
         g.remove(); try { g.name; } catch (e) { vc.log(String(e.message)); }",
    );
    assert_eq!(out, vec![json!(0), json!("Stale Swatch"), json!("Stale SwatchGroup")]);
}

#[test]
fn symbols_add_keeps_the_source_and_instances_follow_their_symbol() {
    let (mut s, a) = doc();
    let out = log(
        &mut s,
        "var d = app.activeDocument, n = d.pageItems.length, sym = d.symbols.add(d.pageItems[0], SymbolRegistrationPoint.SYMBOLTOPLEFTPOINT);\
         vc.log(d.pageItems.length === n); vc.log(sym.typename); vc.log(d.symbols.length);\
         sym.name = 'Mark'; var copy = sym.duplicate(); vc.log(copy.name);\
         var inst = d.symbolItems.add(sym); vc.log(inst.typename); vc.log(inst.symbol.name); vc.log(d.symbolItems.length);\
         inst.symbol = copy; vc.log(inst.symbol.name); vc.log(d.selection === null);",
    );
    assert_eq!(
        out,
        vec![
            json!(true),
            json!("Symbol"),
            json!(1),
            json!("Mark copy"),
            json!("SymbolItem"),
            json!("Mark"),
            json!(1),
            json!("Mark copy"),
            json!(true)
        ]
    );
    assert!(s.active().unwrap().doc.node(a).is_some(), "the source art stays");
    let l = s.execute("symbol.list", &json!({})).unwrap();
    assert_eq!(l["symbols"][0]["registration"], json!(0));
    let steps = undo_steps(&s);
    log(&mut s, "var d = app.activeDocument; d.symbolItems.removeAll(); vc.log(d.symbols.length);");
    assert_eq!(undo_steps(&s), steps + 1);
    let out = log(
        &mut s,
        "var d = app.activeDocument, sym = d.symbols[0]; d.symbols.removeAll(); vc.log(d.symbols.length); try { sym.name; } catch (e) { vc.log(String(e.message)); }",
    );
    assert_eq!(out, vec![json!(0), json!("Stale Symbol")]);
}

#[test]
fn resources_refuse_objects_of_another_document() {
    let (mut s, _) = doc();
    log(&mut s, "var d = app.activeDocument; d.symbols.add(d.pageItems[0]);");
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let before = resources(&s);
    for script in [
        "var a = app.documents[0], b = app.documents[1]; b.symbolItems.add(a.symbols[0]);",
        "var a = app.documents[0], b = app.documents[1]; a.graphicStyles[0].applyTo(b.pageItems[0]);",
        "var a = app.documents[0], b = app.documents[1]; b.swatchGroups.add().addSwatch(a.swatches.getByName('Black'));",
        "var a = app.documents[0], b = app.documents[1]; a.symbols.add(b.pageItems[0]);",
    ] {
        let e = failure(&mut s, script);
        assert!(e.contains("Cross-document"), "{script}: {e}");
    }
    assert_eq!(resources(&s), before);
}

#[test]
fn graphic_styles_apply_replaces_and_merge_adds() {
    let (mut s, a) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("graphicStyle.new", &json!({"name": "Red"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let b = rect(&mut s, 100.0, 100.0, 20.0, 20.0);
    s.execute("select.none", &json!({})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, g = d.graphicStyles.getByName('Red'); vc.log(g.typename); vc.log(d.graphicStyles.typename);\
         g.applyTo(d.pageItems[1]);",
    );
    assert_eq!(out, vec![json!("GraphicStyle"), json!("GraphicStyles")]);
    let id = s.active().unwrap().doc.graphic_style("Red").unwrap().id;
    let n = s.active().unwrap().doc.node(b).unwrap().clone();
    assert_eq!(n.extras().graphic_style, Some(id));
    let fills = n.appearance.items.len();
    log(&mut s, "var d = app.activeDocument; d.graphicStyles.getByName('Red').mergeTo(d.pageItems[1]);");
    let n = s.active().unwrap().doc.node(b).unwrap().clone();
    assert!(n.appearance.items.len() > fills, "merge adds the style's rows");
    assert_eq!(n.extras().graphic_style, None);
    let before = resources(&s);
    for script in [
        "app.activeDocument.graphicStyles.getByName('Default Graphic Style').name = 'Plain';",
        "app.activeDocument.graphicStyles.getByName('Default Graphic Style').remove();",
    ] {
        let e = failure(&mut s, script);
        assert!(e.contains("is built in"), "{script}: {e}");
    }
    assert_eq!(resources(&s), before);
    log(&mut s, "app.activeDocument.graphicStyles.removeAll();");
    let left: Vec<String> = s.active().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect();
    assert_eq!(left, vec![crate::doc::DEFAULT_GRAPHIC_STYLE.to_string()]);
}

#[test]
fn brushes_apply_to_paths_only_and_once() {
    let (mut s, a) = doc();
    s.execute("text.create", &json!({"x": 10, "y": 250, "text": "hi"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let first = s.execute("brush.list", &json!({})).unwrap()["brushes"][0]["name"].as_str().unwrap().to_string();
    let out = log(
        &mut s,
        "var b = app.activeDocument.brushes; vc.log(b.typename); vc.log(b[0].typename); vc.log(b[0].name); vc.log(b.removeAll === undefined);",
    );
    assert_eq!(out, vec![json!("Brushes"), json!("Brush"), json!(first), json!(true)]);
    let before = resources(&s);
    let e = failure(&mut s, "var d = app.activeDocument; d.brushes[0].applyTo(d.pageItems[1]);");
    assert!(e.contains("PathItem only"), "{e}");
    assert_eq!(resources(&s), before);
    log(&mut s, "var d = app.activeDocument; d.brushes[0].applyTo(d.pathItems[0]);");
    let st = s.active().unwrap().doc.node(a).unwrap().appearance.stroke().cloned().unwrap();
    assert_eq!(st.brush.as_deref(), Some(first.as_str()));
    let steps = undo_steps(&s);
    log(&mut s, "var d = app.activeDocument; d.brushes[0].applyTo(d.pathItems[0]);");
    assert_eq!(undo_steps(&s), steps, "applying the brush a path already draws runs no command");
}

#[test]
fn brushes_refuse_guides_clipping_paths_and_compound_members() {
    let (mut s, a) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    let (m1, m2) = (rect(&mut s, 100.0, 20.0, 30.0, 40.0), rect(&mut s, 150.0, 20.0, 30.0, 40.0));
    s.execute("select.set", &json!({"ids": [m1.0, m2.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let (c1, c2) = (rect(&mut s, 200.0, 20.0, 30.0, 40.0), rect(&mut s, 210.0, 30.0, 10.0, 10.0));
    s.execute("select.set", &json!({"ids": [c1.0, c2.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let pick = |test: &str| {
        format!(
            "var d = app.activeDocument, p = null; for (var i = 0; i < d.pathItems.length; i++) {{ var q = d.pathItems[i]; if ({test}) p = q; }}\
             d.brushes[0].applyTo(p);"
        )
    };
    for test in ["q.guides", "q.clipping", "q.parent.typename == 'CompoundPathItem'"] {
        let before = resources(&s);
        let steps = undo_steps(&s);
        let e = failure(&mut s, &pick(test));
        assert!(e.contains("does not apply to a guide"), "{test}: {e}");
        assert_eq!((resources(&s), undo_steps(&s)), (before, steps), "{test}");
    }
}

#[test]
fn several_resource_edits_are_one_undo_step_and_a_failure_puts_them_back() {
    let (mut s, _) = doc();
    let steps = undo_steps(&s);
    log(&mut s, "var d = app.activeDocument; d.swatches.add().name = 'One'; d.spots.add().name = 'Two'; d.swatchGroups.add().name = 'Three';");
    assert_eq!(undo_steps(&s), steps + 1);
    let before = resources(&s);
    let e = failure(&mut s, "var d = app.activeDocument; d.swatches.add().name = 'Four'; d.swatches.removeAll(); throw new Error('stop');");
    assert!(e.contains("stop"), "{e}");
    assert_eq!(resources(&s), before);
}
