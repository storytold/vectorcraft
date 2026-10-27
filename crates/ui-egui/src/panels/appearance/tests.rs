use super::*;

#[test]
fn opacity_labels() {
    assert_eq!(opacity_text(1.0, BlendMode::Normal), "Default");
    assert_eq!(opacity_text(0.5, BlendMode::Normal), "50%");
    assert_eq!(opacity_text(0.5, BlendMode::Multiply), "50% Multiply");
}

#[test]
fn labels() {
    assert_eq!(param_label("offsetX", true), "Offset X");
    assert_eq!(param_label("blur", true), "Blur");
    assert!(!effect_label("stylize.dropShadow").contains('…'));
    assert_eq!(effect_label("path.outlineObject"), "Outline Object");
    assert_eq!(effect_label("x.unknownThing"), "unknownThing");
    // Only our own effects are interface labels; a plug-in's name is its own.
    assert!(built_in_effect("stylize.dropShadow") && !built_in_effect("plugin.example.glow"));
    assert_eq!(shown_effect_label("plugin.example.glow"), effect_label("plugin.example.glow"));
}

/// A Drop Shadow's inline editor has Color and Darkness as radio buttons in its Shade row; a
/// click on Darkness sets the effect's `shade`.
#[test]
fn the_drop_shadow_editor_chooses_color_or_darkness() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    let mut time = 0.0;
    let mut run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.1;
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    };
    run(&mut app, vec![]);
    let out = run(&mut app, vec![]);
    // The Shade row's Darkness, below the Darkness value's own row.
    fn find(s: &egui::Shape, out: &mut Vec<egui::Pos2>) {
        match s {
            egui::Shape::Text(t) if t.galley.text() == "Darkness" => out.push(t.visual_bounding_rect().center()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| find(s, out)),
            _ => {}
        }
    }
    let mut found = vec![];
    out.shapes.iter().for_each(|c| find(&c.shape, &mut found));
    assert_eq!(found.len(), 2, "{found:?}");
    let at = found.into_iter().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        run(&mut app, events);
    }
    let n = app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    assert_eq!(n.appearance.effects[0].params["shade"], json!("darkness"));
}

/// A rectangle with `effect` applied, its inline editor open, after a first frame in `ctx`.
fn editing(effect: &str) -> (VectorcraftApp, egui::Context, NodeId) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": effect})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    panel_frame(&mut app, &ctx, vec![]);
    (app, ctx, NodeId(id))
}

/// One frame of the panel in `ctx` with `events`: the texts drawn and where.
fn panel_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let time = ctx.input(|i| i.time) + 0.1;
    let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// Clicks the text `label` drawn in the panel (the last one drawn when there are several).
fn click_text(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let shown = panel_frame(app, ctx, vec![]);
    let at = shown.iter().rev().find(|(s, _)| s == label).unwrap_or_else(|| panic!("no {label:?} in {shown:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        panel_frame(app, ctx, events);
    }
}

fn shown_texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    panel_frame(app, ctx, vec![]).into_iter().map(|(s, _)| s).collect()
}

/// Roughen's Points and Offset Path's Joins are dropdowns with the effect dialog's labels, and
/// a pick sends the dialog's value as one undo step.
#[test]
fn the_effect_editor_picks_choices_from_the_dialogs_dropdowns() {
    for (effect, key, shown, pick, value) in
        [("distort.roughen", "points", "Smooth", "Corner", "corner"), ("path.offsetPath", "joins", "Miter", "Round", "round")]
    {
        let (mut app, ctx, id) = editing(effect);
        let texts = shown_texts(&mut app, &ctx);
        assert!(texts.iter().any(|s| s == shown), "{effect}: {texts:?}");
        let undo = app.session.doc().unwrap().history.undo.len();
        click_text(&mut app, &ctx, shown);
        click_text(&mut app, &ctx, pick);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.node(id).unwrap().appearance.effects[0].params[key], json!(value), "{effect}");
        assert_eq!(st.history.undo.len(), undo + 1, "{effect}");
        assert!(shown_texts(&mut app, &ctx).iter().any(|s| s == pick), "{effect}");
    }
}

/// The editor labels a parameter as the effect dialog does, without the ":".
#[test]
fn the_effect_editor_labels_parameters_as_the_dialog_does() {
    for (effect, labels) in [
        ("pixelate.colorHalftone", &["Max. Radius", "Channel 1", "Channel 4"][..]),
        ("video.deinterlace", &["Create New Fields by", "Duplication", "Eliminate", "Odd Fields"][..]),
    ] {
        let (mut app, ctx, _) = editing(effect);
        let texts = shown_texts(&mut app, &ctx);
        for label in labels {
            assert!(texts.iter().any(|s| s == label), "{effect}: no {label:?} in {texts:?}");
        }
    }
}

/// A plug-in's choice is a dropdown of the plug-in's own values, also when the parameter is
/// called `mode` and its value names a blend mode.
#[test]
fn a_plug_in_choice_lists_the_plug_ins_values() {
    use vectorcraft_plugins::wat as tpl;
    let params = r#"{"mode":{"type":"choice","options":["normal","screen","dots"],"default":"screen"}}"#;
    let src = tpl::module(&tpl::manifest("test.ui.inlinechoice", "effect", params), tpl::ECHO, "", 1);
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap())})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "plugin.test.ui.inlinechoice"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    panel_frame(&mut app, &ctx, vec![]);
    click_text(&mut app, &ctx, "screen");
    click_text(&mut app, &ctx, "dots");
    let mode = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.effects[0].params["mode"].clone();
    assert_eq!(mode(&app), json!("dots"));
    click_text(&mut app, &ctx, "dots");
    click_text(&mut app, &ctx, "screen");
    assert_eq!(mode(&app), json!("screen"));
    // Once the plug-in is removed, its value shows as it is, not as a blend mode.
    app.run("plugin.remove", json!({"id": "test.ui.inlinechoice"})).unwrap();
    let texts = shown_texts(&mut app, &ctx);
    assert!(texts.iter().any(|s| s == "screen") && !texts.iter().any(|s| s == "Screen"), "{texts:?}");
}

/// A plug-in effect's editor labels its parameters, shows their units and leaves out those whose
/// `visibleWhen` isn't met, as the effect's dialog does, and reads a typed number list the same
/// way.
#[test]
fn a_plug_in_effect_s_editor_follows_its_manifest() {
    use vectorcraft_plugins::wat as tpl;
    let params = r#"{"outline":{"type":"bool","default":true,"label":"Outline"},
            "spacing":{"type":"number","min":0.1,"max":10,"default":1.5,"label":"Spacing","unit":"mm","help":"Distance between the lines."},
            "angles":{"type":"numberList","min":0,"max":360,"default":[45,135],"minItems":1,"maxItems":4,"unit":"°"},
            "width":{"type":"number","min":0,"max":10,"default":0.5,"label":"Width","unit":"pt","visibleWhen":{"outline":[true]}}}"#;
    let src = tpl::module(&tpl::manifest("test.ui.inlinekeys", "effect", params), tpl::ECHO, "", 1);
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap())})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "plugin.test.ui.inlinekeys"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    let texts = shown_texts(&mut app, &ctx);
    for shown in ["Spacing", "1.5 mm", "Angles", "45, 135", "°", "Width", "0.5 pt"] {
        assert!(texts.iter().any(|s| s == shown), "no {shown:?} in {texts:?}");
    }
    assert!(!texts.iter().any(|s| s == "spacing" || s == "width"), "the labels replace the names: {texts:?}");
    // Without the outline, its width isn't shown.
    app.run("effect.setParams", json!({"item": null, "index": 0, "params": {"outline": false}})).unwrap();
    let texts = shown_texts(&mut app, &ctx);
    assert!(!texts.iter().any(|s| s == "Width") && texts.iter().any(|s| s == "Spacing"), "{texts:?}");
    // A list typed in the field: separated by a semicolon and a space, and clamped.
    click_text(&mut app, &ctx, "45, 135");
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    panel_frame(&mut app, &ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    panel_frame(&mut app, &ctx, vec![egui::Event::Text("10; 400 20".into())]);
    panel_frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    panel_frame(&mut app, &ctx, vec![]);
    let angles = app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().appearance.effects[0].params["angles"].clone();
    assert_eq!(angles, json!([10.0, 360.0, 20.0]));
    app.run("plugin.remove", json!({"id": "test.ui.inlinekeys"})).unwrap();
}

/// Glass with a loaded texture lists Loaded Texture in its Texture menu, as its dialog does.
#[test]
fn the_effect_editor_lists_a_loaded_glass_texture() {
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 120, 150, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let stored = vectorcraft_engine::cmd::fileio::texture_image(&png).unwrap();
    let (mut app, ctx, id) = editing("distort.glass");
    app.run("effect.setParams", json!({"item": null, "index": 0, "params": {"texture": "loaded", "textureImage": stored}})).unwrap();
    let texts = shown_texts(&mut app, &ctx);
    assert!(texts.iter().any(|s| s == "Loaded Texture") && !texts.iter().any(|s| s == "loaded"), "{texts:?}");
    click_text(&mut app, &ctx, "Loaded Texture");
    click_text(&mut app, &ctx, "Frosted");
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.node(id).unwrap().appearance.effects[0].params["texture"], json!("frosted"));
}

/// The choices that their dialogs list in their own bodies are dropdowns with those dialogs'
/// labels too: Revolve's edge, Extrude & Bevel's bevel, Warp's orientation, Rasterize's
/// options and the channel of Levels and Curves. A pick sends the dialog's value as one undo
/// step.
#[test]
fn the_effect_editor_picks_choices_their_own_dialogs_list() {
    for (effect, key, shown, pick, value) in [
        ("threeD.revolve", "edge", "Left", "Right", "right"),
        ("threeD.extrude", "bevel", "None", "Rounded", "rounded"),
        ("warp.arc", "orientation", "Horizontal", "Vertical", "vertical"),
        ("rasterize", "colorModel", "RGB", "Grayscale", "grayscale"),
        ("rasterize", "background", "Transparent", "White", "white"),
        ("rasterize", "antiAlias", "Art Optimized", "Type Optimized", "type"),
        ("adjust.levels", "channel", "RGB", "Red", "red"),
        ("adjust.curves", "channel", "RGB", "Blue", "blue"),
    ] {
        let (mut app, ctx, id) = editing(effect);
        let texts = shown_texts(&mut app, &ctx);
        assert!(texts.iter().any(|s| s == shown), "{effect} {key}: {texts:?}");
        let undo = app.session.doc().unwrap().history.undo.len();
        click_text(&mut app, &ctx, shown);
        click_text(&mut app, &ctx, pick);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.node(id).unwrap().appearance.effects[0].params[key], json!(value), "{effect} {key}");
        assert_eq!(st.history.undo.len(), undo + 1, "{effect} {key}");
        assert!(shown_texts(&mut app, &ctx).iter().any(|s| s == pick), "{effect} {key}");
    }
}

/// Those choices read as their dialogs show them in the UI language: in French, the channel's
/// RGB as the "effect" row translates it, and in Spanish, Extrude & Bevel's Rounded as the "3d"
/// row does, not the plain row. Rasterize names the document's color mode.
#[test]
fn the_effect_editor_shows_those_choices_as_their_dialogs_do() {
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    let (mut app, ctx, _) = editing("adjust.levels");
    let texts = shown_texts(&mut app, &ctx);
    let rgb = crate::i18n::tr_ctx(fr, "effect", "RGB");
    assert!(texts.iter().any(|s| s == rgb), "{rgb}: {texts:?}");
    let es = crate::i18n::Lang::from_code("es").unwrap();
    crate::i18n::set_current(es);
    let (mut app, ctx, _) = editing("threeD.extrude");
    app.run("effect.setParams", json!({"item": null, "index": 0, "params": {"bevel": "rounded"}})).unwrap();
    let rounded = crate::i18n::tr_ctx(es, "3d", "Rounded");
    assert_ne!(rounded, crate::i18n::tr(es, "Rounded"));
    assert!(shown_texts(&mut app, &ctx).iter().any(|s| s == rounded), "{rounded}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, ctx, _) = editing("rasterize");
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let texts = shown_texts(&mut app, &ctx);
    assert!(texts.iter().any(|s| s == "CMYK") && !texts.iter().any(|s| s == "document"), "{texts:?}");
}

#[test]
fn the_effect_editor_lists_earlier_parameters_in_their_current_form() {
    // A Scribble saved with one `variation` lists it as its Path Overlap and Spacing variations.
    let e = Effect { id: "stylize.scribble".into(), params: json!({"variation": 2, "angle": 270}), visible: true };
    let p = editor_params(&e);
    assert!(!p.contains_key("variation"), "{p:?}");
    assert_eq!((&p["overlapVariation"], &p["spacingVariation"], &p["curvinessVariation"]), (&json!(2), &json!(2), &json!(0.0)));
    assert_eq!(p["angle"].as_f64(), Some(-90.0));
    // Another effect lists its values over its defaults, as before.
    let e = Effect { id: "distort.twist".into(), params: json!({"angle": 270}), visible: true };
    assert_eq!(editor_params(&e).get("angle"), Some(&json!(270)));
}

/// A text field of the inline editor keeps what is typed from one frame to the next: a Drop
/// Shadow's color typed a character a frame and confirmed with Enter is set in one undo step.
/// Escape drops what is typed.
#[test]
fn the_effect_editor_keeps_text_typed_across_frames() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    set_pstate(&ctx, "ap-fx-open-None-0", true);
    let mut time = 0.0;
    let mut run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.1;
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    };
    // Every text drawn, with its center.
    fn texts(s: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect().center())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let drawn = |out: &egui::FullOutput| {
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    };
    run(&mut app, vec![]);
    let shown = drawn(&run(&mut app, vec![]));
    let at = shown.iter().find(|(t, _)| t == "#000000").map(|(_, p)| *p);
    let Some(at) = at else { panic!("no color field: {shown:?}") };
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        run(&mut app, events);
    }
    run(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    let undo = app.session.active().unwrap().history.undo.len();
    for ch in "#ff0000".chars() {
        run(&mut app, vec![egui::Event::Text(ch.to_string())]);
    }
    let shown = drawn(&run(&mut app, vec![]));
    assert!(shown.iter().any(|(t, _)| t == "#ff0000"), "the field shows what was typed: {shown:?}");
    run(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    run(&mut app, vec![]);
    let color = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(NodeId(id)).unwrap().appearance.effects[0].params["color"].clone();
    assert_eq!(color(&app), json!("#ff0000"));
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    // Escape drops what is typed.
    for events in [vec![button(true)], vec![button(false)]] {
        run(&mut app, events);
    }
    run(&mut app, vec![egui::Event::Text("0".into())]);
    let shown = drawn(&run(&mut app, vec![]));
    assert!(shown.iter().any(|(t, _)| t.len() == 8 && t.starts_with('#')), "a 0 typed into the color: {shown:?}");
    run(&mut app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    run(&mut app, vec![]);
    assert_eq!(color(&app), json!("#ff0000"));
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn drops_pick_the_command() {
    use vectorcraft_doc::{Appearance, FillLayer};
    let fx = |id: &str| Effect { id: id.into(), params: Value::Null, visible: true };
    let mut n = Node::path(NodeId(1), vectorcraft_geom::PathData::default(), Appearance::default_art());
    n.appearance.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None))); // [F, S, F]
    n.appearance.items[1].effects_mut().extend([fx("distort.twist"), fx("distort.roughen")]);
    n.appearance.effects.push(fx("stylize.dropShadow"));
    let slots = [
        Slot::Object { top: true },
        Slot::Item { index: 2, sub: false },
        Slot::Item { index: 1, sub: false },
        Slot::Effect(Some(1), 0),
        Slot::Effect(Some(1), 1),
        Slot::Item { index: 0, sub: false },
        Slot::Effect(None, 0),
        Slot::Object { top: false },
    ];
    let rows: Vec<(Slot, Rect)> =
        slots.iter().enumerate().map(|(i, s)| (*s, Rect::from_min_size(pos2(0.0, i as f32 * ROW), vec2(200.0, ROW)))).collect();
    // The upper or lower half of row `i`.
    let at = |i: usize, lower: bool| pos2(50.0, i as f32 * ROW + if lower { ROW * 0.75 } else { ROW * 0.25 });
    let drop = |what, p, copy| drop_command(&n, &rows, what, p, copy);
    // Fill/stroke rows: moves, Alt copies to where it was dropped.
    assert_eq!(drop(Dragged::Item(0), at(1, false), false), Some(("appearance.moveItem", json!({"from": 0, "to": 2}))));
    assert_eq!(drop(Dragged::Item(0), at(0, false), true), Some(("appearance.duplicateItem", json!({"index": 0, "to": 3}))));
    assert_eq!(drop(Dragged::Item(2), at(7, false), true), Some(("appearance.duplicateItem", json!({"index": 2, "to": 0}))));
    assert_eq!(drop(Dragged::Item(1), at(3, false), false), None, "onto its own effect");
    // Effects: within the list (below the next one), into a fill, out onto the object.
    let mv = |from: usize, fi: Value, to: usize, ti: Value, copy: bool| {
        Some(("effect.move", json!({"from": from, "fromItem": fi, "to": to, "toItem": ti, "copy": copy})))
    };
    assert_eq!(drop(Dragged::Effect(Some(1), 0), at(4, true), false), mv(0, json!(1), 1, json!(1), false));
    assert_eq!(drop(Dragged::Effect(Some(1), 0), at(3, true), false), None, "dropped on itself");
    assert_eq!(drop(Dragged::Effect(Some(1), 1), at(5, false), true), mv(1, json!(1), 0, json!(0), true));
    assert_eq!(drop(Dragged::Effect(Some(1), 1), at(7, false), false), mv(1, json!(1), 1, Value::Null, false));
    assert_eq!(drop(Dragged::Effect(None, 0), at(3, false), false), mv(0, Value::Null, 0, json!(1), false));
    assert_eq!(drop(Dragged::Effect(None, 0), pos2(500.0, 10.0), false), None, "outside the stack");
}

/// One headless frame of the panel: the texts drawn, top to bottom.
fn frame(app: &mut VectorcraftApp) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<(f32, String)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.pos.y, t.galley.text().to_string())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, t)| t).collect()
}

#[test]
fn groups_show_a_contents_row_and_type_a_characters_row_in_their_slot() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    let b = run(&mut app, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "object.group", json!({}));
    run(&mut app, "appearance.addFill", json!({}));
    run(&mut app, "appearance.addStroke", json!({}));
    let rows = |t: Vec<String>| t.into_iter().filter(|t| ["Fill:", "Stroke:", "Contents", "Characters"].contains(&t.as_str())).collect::<Vec<_>>();
    assert_eq!(rows(frame(&mut app)), ["Stroke:", "Fill:", "Contents"]);
    run(&mut app, "appearance.moveItem", json!({"from": "contents", "to": 1}));
    assert_eq!(rows(frame(&mut app)), ["Stroke:", "Contents", "Fill:"]);
    let t = run(&mut app, "text.create", json!({"x": 0, "y": 60, "text": "Hi"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [t]}));
    assert_eq!(rows(frame(&mut app)), ["Characters"]);
    run(&mut app, "appearance.addFill", json!({}));
    assert_eq!(rows(frame(&mut app)), ["Fill:", "Characters"]);
}

#[test]
fn images_show_a_contents_row_with_new_fills_and_strokes_below_it() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let rows = |t: Vec<String>| t.into_iter().filter(|t| ["Fill:", "Stroke:", "Contents"].contains(&t.as_str())).collect::<Vec<_>>();
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    run(&mut app, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png)}));
    assert_eq!(rows(frame(&mut app)), ["Contents"]);
    run(&mut app, "appearance.addStroke", json!({}));
    assert_eq!(rows(frame(&mut app)), ["Contents", "Stroke:"]);
}

#[test]
fn drops_move_items_across_the_contents_row_and_the_row_itself() {
    use vectorcraft_doc::FillLayer;
    // A group [F0 | F1 F2]: the Contents row sits above item 0.
    let mut n = Node::group(NodeId(1), vec![]);
    n.appearance.items = (0..3).map(|_| AppearanceItem::Fill(FillLayer::new(Paint::None))).collect();
    n.appearance.set_contents_at(1);
    let slots = [
        Slot::Object { top: true },
        Slot::Item { index: 2, sub: false },
        Slot::Item { index: 1, sub: false },
        Slot::Contents,
        Slot::Item { index: 0, sub: false },
        Slot::Object { top: false },
    ];
    let rows: Vec<(Slot, Rect)> =
        slots.iter().enumerate().map(|(i, s)| (*s, Rect::from_min_size(pos2(0.0, i as f32 * ROW), vec2(200.0, ROW)))).collect();
    let at = |i: usize| pos2(50.0, i as f32 * ROW + ROW / 2.0);
    let drop = |what, i| drop_command(&n, &rows, what, at(i), false);
    // Onto the Contents row: from above it lands just below, from below just above.
    assert_eq!(drop(Dragged::Item(2), 3), Some(("appearance.moveItem", json!({"from": 2, "to": 1, "contents": 2}))));
    assert_eq!(drop(Dragged::Item(0), 3), Some(("appearance.moveItem", json!({"from": 0, "to": 0, "contents": 0}))));
    // Onto the object row: above everything; under the stack: below everything.
    assert_eq!(drop(Dragged::Item(0), 0), Some(("appearance.moveItem", json!({"from": 0, "to": 2, "contents": 0}))));
    assert_eq!(drop(Dragged::Item(2), 5), Some(("appearance.moveItem", json!({"from": 2, "to": 0, "contents": 2}))));
    // The row itself: above item 2, below item 0; nowhere new on itself.
    assert_eq!(drop(Dragged::Contents, 1), Some(("appearance.moveItem", json!({"from": "contents", "to": 3}))));
    assert_eq!(drop(Dragged::Contents, 4), Some(("appearance.moveItem", json!({"from": "contents", "to": 0}))));
    assert_eq!(drop(Dragged::Contents, 3), None);
    assert_eq!(drop(Dragged::Contents, 2), Some(("appearance.moveItem", json!({"from": "contents", "to": 2}))));
}

#[test]
fn with_nothing_selected_it_lists_what_new_art_gets() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}));
    run(&mut app, "appearance.addFill", json!({}));
    run(&mut app, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    run(&mut app, "transparency.set", json!({"opacity": 50}));
    run(&mut app, "graphicStyle.new", json!({"name": "Fancy"}));
    run(&mut app, "select.none", json!({}));
    let shown = frame(&mut app);
    assert!(shown.contains(&"No Selection".to_string()) && shown.contains(&"Opacity: Default".to_string()), "{shown:?}");
    // A style clicked with nothing selected: the rows show what the next object gets.
    run(&mut app, "graphicStyle.apply", json!({"name": "Fancy"}));
    run(&mut app, "stroke.set", json!({"dash": [3, 3]}));
    let shown = frame(&mut app);
    let rows: Vec<&String> = shown.iter().filter(|t| ["Fill:", "Stroke:"].contains(&t.as_str())).collect();
    assert_eq!(rows, ["Fill:", "Stroke:", "Fill:"], "the added fill is on top");
    for want in ["No Selection: Fancy".to_string(), effect_label("stylize.dropShadow"), "Opacity: 50%".into(), "Dashed".into()] {
        assert!(shown.contains(&want), "{want} in {shown:?}");
    }
}

/// Both stroke rows expose named Weight inputs, and the drop-shadow effect names its Blur input and
/// Color radio button.
#[test]
fn inline_stroke_weight_and_effect_fields_are_named() {
    use egui_kittest::kittest::Queryable;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("appearance.addStroke", json!({})).unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    let mut first = true;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(360.0, 760.0)).with_accessibility_check(false).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if std::mem::replace(&mut first, false) {
                crate::theme::install_fonts(ui.ctx());
                set_pstate(ui.ctx(), "ap-fx-open-None-0", true);
            }
            show(app, ui);
        },
        app,
    );
    h.run_steps(4);
    assert_eq!(h.get_all_by_role_and_label(egui::accesskit::Role::TextInput, "Weight:").count(), 2, "both stroke rows are named");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Blur");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::RadioButton, "Color");
}

/// #4174: two circles with different fills and the same stroke, effect and opacity list the
/// object row with their kind, the "Mixed Appearances" note and what they share: the stroke, the
/// Feather and the opacity, not the fills. A path with type names them "Mixed Objects".
#[test]
fn a_mixed_selection_lists_what_its_objects_share() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let run = |app: &mut VectorcraftApp, id: &str, p: Value| app.session.execute(id, &p).unwrap();
    run(&mut app, "file.new", json!({"width": 300, "height": 200}));
    let mut ids = vec![];
    for (x, color) in [(0, "#ffff00"), (150, "#ff0000")] {
        let id = run(&mut app, "shape.ellipse", json!({"x": x, "y": 50, "width": 100, "height": 100}))["id"].clone();
        run(&mut app, "paint.setFill", json!({"ids": [id], "color": color}));
        run(&mut app, "appearance.setItem", json!({"ids": [id], "index": 1, "color": "#0000ff", "weight": 11}));
        run(&mut app, "effect.apply", json!({"ids": [id], "effect": "stylize.feather", "item": null}));
        ids.push(id);
    }
    run(&mut app, "select.set", json!({"ids": ids}));
    let shown = frame(&mut app);
    let kind = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(ids[0].as_u64().unwrap())).unwrap().kind_label();
    assert_eq!(shown.first().map(String::as_str), Some(kind), "the object row names their kind: {shown:?}");
    assert!(shown.iter().any(|t| t == "Mixed Appearances"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Stroke:") && !shown.iter().any(|t| t == "Fill:"), "the stroke, not the fills: {shown:?}");
    assert!(shown.iter().any(|t| t.contains("Feather")), "the shared effect: {shown:?}");
    assert!(shown.iter().any(|t| t == "Opacity:"), "{shown:?}");
    // Alike objects list their whole stack, without the note.
    run(&mut app, "paint.setFill", json!({"ids": ids, "color": "#ffff00"}));
    let shown = frame(&mut app);
    assert!(!shown.iter().any(|t| t == "Mixed Appearances") && shown.iter().any(|t| t == "Fill:"), "{shown:?}");
    // A path and type.
    let t = run(&mut app, "text.create", json!({"x": 0, "y": 180, "text": "Hi"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [ids[0].clone(), t]}));
    assert_eq!(frame(&mut app).first().map(String::as_str), Some("Mixed Objects"));
}
