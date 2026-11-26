use super::*;
use crate::i18n::LangExt as _;
use crate::tests::panel_harness::Panel;

/// A selected rectangle, for the panel's rows and footer.
fn app_with_a_selected_rectangle() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({ "ids": [id] })).unwrap();
    app
}

/// The object the panel lists, as the document holds it now.
fn node(app: &VectorcraftApp) -> Arc<Node> {
    first_subject(app).expect("an object")
}

/// The panel's picked rows, lead and anchor.
fn picked(panel: &Panel) -> (Vec<RowKey>, Option<RowKey>, Option<RowKey>) {
    let app = panel.app();
    let s = &app.ui.appearance_panel.selection;
    (s.items.clone(), s.lead, s.anchor)
}

/// Duplicating rows picks the copies, each right above its original, as the Layers panel does:
/// the active row's copy becomes active, the anchor's copy the anchor.
#[test]
fn a_duplicate_highlights_the_copies_above_the_originals() {
    let mut app = app_with_a_selected_rectangle();
    for _ in 0..3 {
        app.run("effect.apply", json!({"effect": "distort.roughen"})).unwrap();
    }
    // Effects 0 and 2 picked, 2 active: the list reads copy, 0, 1, copy, 2 (top down), so the
    // copies are rows 0 and 3.
    app.run("appearance.setActiveItem", json!({ "index": null })).unwrap();
    app.ui.appearance_panel.owner = subject(&app);
    app.ui.appearance_panel.selection = Selection::new(vec![RowKey::Effect(2), RowKey::Effect(0)], Some(RowKey::Effect(2)), Some(RowKey::Effect(0)));
    let mut panel = Panel::new(app, show);
    panel.click("Duplicate Selected Item");
    assert_eq!(picked(&panel), (vec![RowKey::Effect(3), RowKey::Effect(0)], Some(RowKey::Effect(3)), Some(RowKey::Effect(0))));
    // [Fill, Stroke] both picked, the stroke active: paint order fill, copy, stroke, copy, so the
    // copies are 1 and 3 and the stroke's copy is active.
    {
        let mut app = panel.app();
        app.run("appearance.setActiveItem", json!({ "index": 1 })).unwrap();
        app.ui.appearance_panel.selection = Selection::new(vec![RowKey::Item(1), RowKey::Item(0)], Some(RowKey::Item(1)), Some(RowKey::Item(1)));
    }
    panel.click("Duplicate Selected Item");
    assert_eq!(picked(&panel), (vec![RowKey::Item(3), RowKey::Item(1)], Some(RowKey::Item(3)), Some(RowKey::Item(3))));
    assert_eq!(panel.app().session.appearance_item(), Some(3));
    // A row outside the picked ones (dropped on Duplicate): only its copy is picked and active.
    let mut app = panel.app();
    app.run("appearance.setActiveItem", json!({ "index": null })).unwrap();
    app.ui.appearance_panel.selection = Selection::single(RowKey::Effect(0));
    let n = node(&app);
    let sel = current_sel(&app, Some(&*n));
    app.run("effect.duplicate", json!({"item": null, "indices": [4]})).unwrap();
    highlight_copies(&mut app, &n, sel, Rows::Effects(None), &[4]);
    drop(app);
    assert_eq!(picked(&panel), (vec![RowKey::Effect(4)], Some(RowKey::Effect(4)), Some(RowKey::Effect(4))));
}

/// Removing other fills/strokes keeps a fill/stroke's Opacity row selected, renumbered with its
/// item (so the Transparency panel keeps editing it); removing that item clears the selection.
#[test]
fn removing_items_keeps_a_surviving_item_opacity_row_selected() {
    let mut app = app_with_a_selected_rectangle();
    app.run("appearance.duplicateItem", json!({ "indices": [0] })).unwrap();
    let remove = |app: &mut VectorcraftApp, k: usize| {
        let n = node(app);
        let sel = current_sel(app, Some(&*n));
        app.run("appearance.removeItem", json!({ "indices": [k] })).unwrap();
        highlight_after_removal(app, &n, sel, Rows::Items, &[k]);
    };
    select_row(&mut app, Sel::ItemOpacity(2));
    remove(&mut app, 0);
    assert_eq!(current_sel(&app, Some(&*node(&app))), Sel::ItemOpacity(1));
    assert_eq!(app.session.appearance_item(), Some(1));
    remove(&mut app, 1);
    assert_eq!(current_sel(&app, Some(&*node(&app))), Sel::None);
    assert_eq!(app.session.appearance_item(), None);
}

/// #5742: Delete in the list on a selected Opacity row sets it back to Default (100%, Normal): the
/// object's own, or a fill's, leaving the other untouched, in one undo step.
#[test]
fn deleting_an_opacity_row_sets_it_back_to_default() {
    let mut app = app_with_a_selected_rectangle();
    app.run("transparency.set", json!({"item": null, "opacity": 50, "blend": "Multiply"})).unwrap();
    app.run("transparency.set", json!({"item": 0, "opacity": 40, "blend": "Screen"})).unwrap();
    app.ui.appearance_panel.open.insert(RowKey::Item(0));
    let mut panel = Panel::new(app, show);
    // A click on the fill's row gives the list the keyboard.
    panel.click("Fill");
    let pick = |panel: &mut Panel, sel: Sel| {
        select_row(&mut panel.app(), sel);
        panel.frame();
    };
    // The fill's Opacity row: only the fill goes back to Default, and the row stays selected.
    pick(&mut panel, Sel::ItemOpacity(0));
    panel.key(egui::Key::Delete, egui::Modifiers::NONE);
    {
        let app = panel.app();
        let n = node(&app);
        assert_eq!((n.appearance.items[0].opacity(), n.appearance.items[0].blend()), (1.0, BlendMode::Normal));
        assert_eq!((n.opacity, n.blend), (0.5, BlendMode::Multiply));
        assert_eq!(current_sel(&app, Some(&*n)), Sel::ItemOpacity(0));
    }
    // The object's Opacity row.
    pick(&mut panel, Sel::Opacity);
    panel.key(egui::Key::Delete, egui::Modifiers::NONE);
    let mut app = panel.app();
    let n = node(&app);
    assert_eq!(opacity_text(n.opacity, n.blend), "Default");
    // One step, so Undo brings the object's opacity back.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(node(&app).opacity, 0.5);
}

/// An edit in an effect row's inline editor sends what it changed through `effect.setParams`, as
/// one undo step: Drop Shadow's Color or Darkness, and the choices the effect dialogs list
/// (Roughen's Points, Offset Path's Joins), found by the dialogs' labels. The editor lists earlier
/// parameters in their current form.
#[test]
fn the_effect_editor_sets_what_its_dialog_rows_change() {
    for (effect, field, pick, key, value) in [
        ("stylize.dropShadow", None, "Darkness", "shade", "darkness"),
        ("distort.roughen", Some("Points"), "Corner", "points", "corner"),
        ("path.offsetPath", Some("Joins"), "Round", "joins", "round"),
    ] {
        let mut app = app_with_a_selected_rectangle();
        app.run("effect.apply", json!({ "effect": effect })).unwrap();
        app.ui.appearance_panel.open.insert(RowKey::Effect(0));
        let mut panel = Panel::new(app, show);
        let undo = panel.app().session.active().unwrap().history.undo.len();
        if let Some(field) = field {
            panel.click(field);
        }
        panel.click(pick);
        let app = panel.app();
        assert_eq!(node(&app).appearance.effects[0].params[key], json!(value), "{effect}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "{effect}: one undo step");
    }
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
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("effect.apply", json!({"effect": "plugin.test.ui.inlinechoice"})).unwrap();
    app.ui.appearance_panel.open.insert(RowKey::Effect(0));
    let mut panel = Panel::new(app, show);
    let mode = |panel: &Panel| node(&panel.app()).appearance.effects[0].params["mode"].clone();
    panel.click("Mode");
    panel.click("dots");
    assert_eq!(mode(&panel), json!("dots"));
    panel.click("Mode");
    panel.click("screen");
    assert_eq!(mode(&panel), json!("screen"));
    // Once the plug-in is removed, its value shows as it is, not as a blend mode.
    panel.app().run("plugin.remove", json!({"id": "test.ui.inlinechoice"})).unwrap();
    panel.frame();
    assert_eq!(panel.value("Mode").as_deref(), Some("screen"));
}

/// A plug-in effect's editor labels its parameters, shows their values in their units and leaves
/// out those whose `visibleWhen` isn't met, as the effect's dialog does, and reads a typed number
/// list the same way.
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
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("effect.apply", json!({"effect": "plugin.test.ui.inlinekeys"})).unwrap();
    app.ui.appearance_panel.open.insert(RowKey::Effect(0));
    let mut panel = Panel::new(app, show);
    for shown in ["Outline", "Spacing", "Angles", "Width"] {
        assert!(panel.shows(shown), "no {shown:?}");
    }
    assert!(!panel.shows("spacing") && !panel.shows("width"), "the labels replace the names");
    assert!(panel.value("Spacing").is_some_and(|v| v.contains("1.5") && v.contains("mm")), "{:?}", panel.value("Spacing"));
    assert!(panel.value("Angles").is_some_and(|v| v.contains("45, 135")), "{:?}", panel.value("Angles"));
    // Without the outline, its width isn't shown.
    panel.app().run("effect.setParams", json!({"item": null, "index": 0, "params": {"outline": false}})).unwrap();
    panel.frame();
    assert!(!panel.shows("Width") && panel.shows("Spacing"));
    // A list typed in the field: separated by a semicolon and a space, and clamped.
    panel.type_into("Angles", "10; 400 20");
    assert_eq!(node(&panel.app()).appearance.effects[0].params["angles"], json!([10.0, 360.0, 20.0]));
    panel.app().run("plugin.remove", json!({"id": "test.ui.inlinekeys"})).unwrap();
}

/// Glass with a loaded texture lists Loaded Texture in its Texture menu, as its dialog does.
#[test]
fn the_effect_editor_lists_a_loaded_glass_texture() {
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(4, 4, image::Rgba([90, 120, 150, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let stored = vectorcraft_engine::cmd::fileio::texture_image(&png).unwrap();
    let mut app = app_with_a_selected_rectangle();
    app.run("effect.apply", json!({"effect": "distort.glass"})).unwrap();
    app.run("effect.setParams", json!({"item": null, "index": 0, "params": {"texture": "loaded", "textureImage": stored}})).unwrap();
    app.ui.appearance_panel.open.insert(RowKey::Effect(0));
    let mut panel = Panel::new(app, show);
    assert_eq!(panel.value("Texture").as_deref(), Some("Loaded Texture"));
    panel.click("Texture");
    panel.click("Frosted");
    assert_eq!(node(&panel.app()).appearance.effects[0].params["texture"], json!("frosted"));
}

/// The command each reorder of the stack's rows runs, by where the rows land (`to`, a place
/// between the rows left once the moved ones are taken out, or the row they are dropped into):
/// fills/strokes alone and as blocks, across the Contents row, the Contents row itself, effects
/// within and across lists, and Alt copies. Places that change nothing or take no such rows run
/// nothing, and the list refuses them.
#[test]
fn drops_pick_the_command() {
    use super::RowKey::{Contents, ItemEffect, ItemOpacity, Object, Opacity};
    use vectorcraft_doc::FillLayer;
    let fx = |id: &str| Effect { id: id.into(), params: Value::Null, visible: true };
    let mv = |keys: &[RowKey], to: usize, copy: bool, parent: Option<RowKey>| ReorderMove::new(keys[0], keys.to_vec(), to, copy).in_group(parent);
    let fills = |count: usize, contents: usize| {
        let mut n = Node::group(NodeId(1), vec![]);
        n.appearance.items = (0..count).map(|_| AppearanceItem::Fill(FillLayer::new(Paint::None))).collect();
        n.appearance.set_contents_at(contents);
        n
    };
    let (item, effect) = (RowKey::Item, RowKey::Effect);
    let moves = |p: Value| Some(("appearance.moveItem", p));
    let copies = |p: Value| Some(("appearance.duplicateItem", p));
    let effects = |p: Value| Some(("effect.move", p));

    // A path [F0, S1 (twist, roughen), F2] with a Drop Shadow, the stroke open.
    let mut path = Node::path(NodeId(1), vectorcraft_geom::PathData::default(), Appearance::default_art());
    path.appearance.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
    path.appearance.items[1].effects_mut().extend([fx("distort.twist"), fx("distort.roughen")]);
    path.appearance.effects.push(fx("stylize.dropShadow"));
    let order = [Object, item(2), item(1), ItemOpacity(1), ItemEffect(1, 0), ItemEffect(1, 1), item(0), effect(0), Opacity];
    for (moved, want) in [
        (mv(&[item(0)], 1, false, None), moves(json!({"from": 0, "to": 2}))),
        (mv(&[item(0)], 1, true, None), copies(json!({"index": 0, "to": 3}))),
        (mv(&[item(2)], 8, true, None), copies(json!({"index": 2, "to": 0}))),
        (mv(&[item(1)], 3, false, None), None),
        (mv(&[item(1)], 3, false, Some(ItemEffect(1, 0))), None),
        (mv(&[ItemEffect(1, 0)], 5, false, None), effects(json!({"from": 0, "fromItem": 1, "to": 1, "toItem": 1, "copy": false}))),
        (mv(&[ItemEffect(1, 0)], 4, false, None), None),
        (mv(&[ItemEffect(1, 1)], 5, true, Some(item(0))), effects(json!({"from": 1, "fromItem": 1, "to": 0, "toItem": 0, "copy": true}))),
        (mv(&[ItemEffect(1, 1)], 7, false, None), effects(json!({"from": 1, "fromItem": 1, "to": 1, "toItem": null, "copy": false}))),
        (mv(&[effect(0)], 4, false, None), effects(json!({"from": 0, "fromItem": null, "to": 0, "toItem": 1, "copy": false}))),
        (mv(&[effect(0)], 2, false, None), None),
    ] {
        assert_eq!(reorder_command(&path, &order, &moved), want, "{moved:?}");
    }
    let drag = |keys: &[RowKey]| ListDrag::new(egui::Id::NULL, keys.to_vec(), keys[0]);
    assert!(accepts(&path, &order, &drag(&[effect(0)]), Some(&item(0)), 2, false), "into a fill's row");
    assert!(!accepts(&path, &order, &drag(&[effect(0)]), Some(&Object), 1, false), "where it is");

    // A group listed top-down S3, FA*, Contents, S1*, F0 (paint order F0 S1 FA S3, Contents above
    // two), FA and S1 dragged together.
    let group = fills(4, 2);
    let order = [Object, item(3), item(2), Contents, item(1), item(0), Opacity];
    let (from_above, from_below) = ([item(2), item(1)], [item(1), item(2)]);
    let block = |to: usize, contents: usize| json!({"indices": [1, 2], "to": to, "contents": contents});
    for (moved, want) in [
        (mv(&from_above, 1, false, Some(item(3))), moves(block(2, 1))),
        (mv(&from_above, 3, false, None), moves(block(1, 3))),
        (mv(&from_below, 2, false, None), moves(block(1, 1))),
        (mv(&from_above, 1, false, None), moves(block(2, 1))),
        (mv(&from_above, 4, false, None), moves(block(0, 3))),
        (mv(&from_above, 4, false, Some(item(0))), moves(block(0, 3))),
        (mv(&from_above, 2, false, Some(item(2))), None),
        (mv(&from_above, 1, true, None), copies(block(4, 2))),
        (mv(&from_above, 4, true, Some(item(0))), copies(block(0, 4))),
        (mv(&from_above, 3, true, None), copies(block(2, 4))),
    ] {
        assert_eq!(reorder_command(&group, &order, &moved), want, "{moved:?}");
    }

    // Effects a, b, c of a path, a and c dragged together.
    let mut abc = Node::path(NodeId(2), vectorcraft_geom::PathData::default(), Appearance::default_art());
    abc.appearance.effects.extend([fx("a"), fx("b"), fx("c")]);
    let order = [Object, item(1), item(0), effect(0), effect(1), effect(2), Opacity];
    let ac = [effect(0), effect(2)];
    for (moved, want) in [
        (mv(&ac, 4, false, None), effects(json!({"from": [0, 2], "fromItem": null, "to": 1, "toItem": null, "copy": false}))),
        (mv(&ac, 1, true, Some(item(1))), effects(json!({"from": [0, 2], "fromItem": null, "to": 0, "toItem": 1, "copy": true}))),
        (mv(&ac, 2, false, Some(effect(0))), None),
        (mv(&ac, 2, false, Some(effect(2))), None),
    ] {
        assert_eq!(reorder_command(&abc, &order, &moved), want, "{moved:?}");
    }

    // A group [F0 | F1 F2]: the Contents row sits above item 0.
    let group = fills(3, 1);
    let order = [Object, item(2), item(1), Contents, item(0), Opacity];
    for (moved, want) in [
        // Below the Contents row from above it, above it from below.
        (mv(&[item(2)], 3, false, None), moves(json!({"from": 2, "to": 1, "contents": 2}))),
        (mv(&[item(0)], 3, false, None), moves(json!({"from": 0, "to": 0, "contents": 0}))),
        // Above everything; below everything.
        (mv(&[item(0)], 1, false, None), moves(json!({"from": 0, "to": 2, "contents": 0}))),
        (mv(&[item(2)], 4, false, None), moves(json!({"from": 2, "to": 0, "contents": 2}))),
        // The row itself: into a row above it lands above that row, into one below it below; where
        // it is, nowhere new.
        (mv(&[Contents], 1, false, Some(item(2))), moves(json!({"from": "contents", "to": 3}))),
        (mv(&[Contents], 4, false, Some(item(0))), moves(json!({"from": "contents", "to": 0}))),
        (mv(&[Contents], 3, false, None), None),
        (mv(&[Contents], 2, false, Some(item(1))), moves(json!({"from": "contents", "to": 2}))),
    ] {
        assert_eq!(reorder_command(&group, &order, &moved), want, "{moved:?}");
    }
    assert!(!accepts(&group, &order, &drag(&[Contents]), Some(&Contents), 3, false), "where it is");
}

#[test]
fn groups_show_a_contents_row_and_type_a_characters_row_in_their_slot() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    app.run("appearance.addFill", json!({})).unwrap();
    app.run("appearance.addStroke", json!({})).unwrap();
    let mut panel = Panel::new(app, show);
    let top = |panel: &Panel, name: &str| panel.rect(name).top();
    assert!(top(&panel, "Stroke:") < top(&panel, "Fill:") && top(&panel, "Fill:") < top(&panel, "Contents"));
    panel.app().run("appearance.moveItem", json!({"from": "contents", "to": 1})).unwrap();
    panel.frame();
    assert!(top(&panel, "Stroke:") < top(&panel, "Contents") && top(&panel, "Contents") < top(&panel, "Fill:"));
    {
        let mut app = panel.app();
        let t = app.run("text.create", json!({"x": 0, "y": 60, "text": "Hi"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [t]})).unwrap();
    }
    panel.frame();
    assert!(panel.shows("Characters") && !panel.shows("Contents") && !panel.shows("Fill:"));
    panel.app().run("appearance.addFill", json!({})).unwrap();
    panel.frame();
    assert!(top(&panel, "Fill:") < top(&panel, "Characters"));
}

/// Add New Fill (button, menu or shortcut) with rows picked: the copy of the active row lands
/// right above it and is then the only picked row.
#[test]
fn add_new_fill_selects_the_new_fill_alone() {
    let mut app = app_with_a_selected_rectangle();
    app.run("appearance.addFill", json!({})).unwrap(); // [Fill, Stroke, Fill]
    // Rows 1 (the stroke) and 0 picked together, row 0 (picked last) active.
    app.run("appearance.setActiveItem", json!({ "index": 0 })).unwrap();
    app.ui.appearance_panel.owner = subject(&app);
    app.ui.appearance_panel.selection = Selection::new(vec![RowKey::Item(1), RowKey::Item(0)], Some(RowKey::Item(0)), Some(RowKey::Item(1)));
    let n = node(&app);
    assert_eq!(selected_items(&app, &n, current_sel(&app, Some(&*n))), [0, 1]);
    app.run("appearance.addFill", json!({})).unwrap();
    let n = node(&app);
    assert_eq!(n.appearance.items.iter().map(AppearanceItem::kind_name).collect::<Vec<_>>(), ["fill", "fill", "stroke", "fill"]);
    let sel = current_sel(&app, Some(&*n));
    assert_eq!(sel, Sel::Item(1));
    assert_eq!(selected_items(&app, &n, sel), [1]);
}

#[test]
fn images_show_a_contents_row_with_new_fills_and_strokes_below_it() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    app.run("file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Contents") && !panel.shows("Fill:") && !panel.shows("Stroke:"));
    panel.app().run("appearance.addStroke", json!({})).unwrap();
    panel.frame();
    assert!(panel.rect("Contents").top() < panel.rect("Stroke:").top());
}

#[test]
fn with_nothing_selected_it_lists_what_new_art_gets() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.run("appearance.addFill", json!({})).unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
    app.run("graphicStyle.new", json!({"name": "Fancy"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("No Selection") && panel.shows("Opacity: Default"));
    // A style clicked with nothing selected: the rows show what the next object gets.
    panel.app().run("graphicStyle.apply", json!({"name": "Fancy"})).unwrap();
    panel.app().run("stroke.set", json!({"dash": [3, 3]})).unwrap();
    panel.frame();
    // The added fill is on top: fill, stroke, fill.
    let new_art = panel.app().session.new_art();
    let rows = StackRows::new(None, &new_art, None, &HashSet::new()).keys();
    let items: Vec<&str> = rows
        .iter()
        .filter_map(|key| match key {
            RowKey::NewArtItem(i) => new_art.items.get(*i).map(AppearanceItem::kind_name),
            _ => None,
        })
        .collect();
    assert_eq!(items, ["fill", "stroke", "fill"]);
    assert!(panel.shows("Fill:") && panel.shows("Stroke:"));
    for want in ["No Selection: Fancy".to_string(), effect_label("stylize.dropShadow"), "Opacity: 50%".into(), "Dashed".into()] {
        assert!(panel.shows(&want), "no {want:?}");
    }
    // The style name follows the UI language's colon (#7991).
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    panel.frame();
    let want = format!("{}\u{a0}: Fancy", crate::i18n::tr(fr, "No Selection"));
    let shown = panel.shows(&want);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(shown, "no {want:?}");
}

/// #4174: two circles with different fills and the same stroke, effect and opacity list the
/// object row with their kind, the "Mixed Appearances" note and what they share: the stroke, the
/// Feather and the opacity, not the fills. A path with type names them "Mixed Objects".
#[test]
fn a_mixed_selection_lists_what_its_objects_share() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let mut ids = vec![];
    for (x, color) in [(0, "#ffff00"), (150, "#ff0000")] {
        let id = app.run("shape.ellipse", json!({"x": x, "y": 50, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": color})).unwrap();
        app.run("appearance.setItem", json!({"ids": [id], "index": 1, "color": "#0000ff", "weight": 11})).unwrap();
        app.run("effect.apply", json!({"ids": [id], "effect": "stylize.feather", "item": null})).unwrap();
        ids.push(id);
    }
    app.run("select.set", json!({"ids": ids})).unwrap();
    let kind = app.session.active().unwrap().doc.node(NodeId(ids[0].as_u64().unwrap())).unwrap().kind_label();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows(kind), "the object row names their kind");
    assert!(panel.shows("Mixed Appearances"));
    assert!(panel.shows("Stroke:") && !panel.shows("Fill:"), "the stroke, not the fills");
    assert!(panel.shows(&effect_row_label("stylize.feather")), "the shared effect");
    assert!(panel.shows("Opacity:"));
    // The object row shows no fill chip: the fills differ, as the Properties panel's "?" chip says.
    let fills_differ = |panel: &Panel| {
        let mut app = panel.app();
        let s = summary(&mut app).unwrap();
        s.mixed(&app).is_some_and(|m| m.fills_differ)
    };
    assert!(fills_differ(&panel));
    // Alike objects list their whole stack, without the note.
    panel.app().run("paint.setFill", json!({"ids": ids, "color": "#ffff00"})).unwrap();
    panel.frame();
    assert!(!panel.shows("Mixed Appearances") && panel.shows("Fill:"));
    assert!(!fills_differ(&panel));
    // A path and type.
    {
        let mut app = panel.app();
        let t = app.run("text.create", json!({"x": 0, "y": 180, "text": "Hi"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [ids[0].clone(), t]})).unwrap();
    }
    panel.frame();
    assert!(panel.shows("Mixed Objects"));
}

/// The [`summary`] the canvas cache holds for the active document, checked field by field against
/// what [`object_label`] and [`mixed`] work out from scratch.
fn fresh_summary(app: &mut VectorcraftApp) -> Arc<Summary> {
    let s = summary(app).unwrap();
    assert!(app.canvas.appearance_summary.as_ref().is_some_and(|held| Arc::ptr_eq(held, &s)), "the cache holds it");
    assert_eq!(s.label, object_label(app), "the kind label");
    let parts = |m: &Mixed| (m.shared.clone(), m.transparency, m.fills_differ);
    assert_eq!(s.mixed(app).map(parts), mixed(app).as_ref().map(parts), "the mixed summary");
    s
}

/// The object row's kind and the Mixed Appearances summary are worked out once per document
/// revision and list of subjects: a repaint with nothing changed reuses them, and they follow an
/// edit, Undo and Redo, a selection that changes only in the middle, a target circle and another
/// document at the same revision.
#[test]
fn the_selection_summary_is_reused_until_the_document_or_its_subjects_change() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
    let art = |app: &mut VectorcraftApp| -> (Vec<NodeId>, NodeId) {
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let r = (0..3).map(|i| id(app.run("shape.rectangle", json!({"x": i * 30, "y": 0, "width": 20, "height": 20})).unwrap())).collect();
        (r, id(app.run("text.create", json!({"x": 0, "y": 100, "text": "Hi"})).unwrap()))
    };
    let (r, t) = art(&mut app);
    app.run("select.set", json!({"ids": r})).unwrap();
    let s = fresh_summary(&mut app);
    assert!(s.mixed(&app).is_none(), "alike rectangles");
    assert!(Arc::ptr_eq(&s, &fresh_summary(&mut app)), "a repaint with nothing changed reuses the summary");
    // An edit of the middle object, then Undo and Redo.
    app.run("transparency.set", json!({"ids": [r[1]], "opacity": 50})).unwrap();
    assert!(fresh_summary(&mut app).mixed(&app).is_some_and(|m| m.transparency), "the middle one's opacity differs");
    app.run("edit.undo", json!({})).unwrap();
    assert!(fresh_summary(&mut app).mixed(&app).is_none(), "undone");
    app.run("edit.redo", json!({})).unwrap();
    assert!(fresh_summary(&mut app).mixed(&app).is_some(), "redone");
    // The middle id changes at the same revision, length, first and last id.
    app.session.active_mut().unwrap().selection.objects[1] = t;
    assert_eq!(fresh_summary(&mut app).label, "Mixed Objects");
    // A target circle with the selected objects unchanged, and back.
    let layer = app.session.active().unwrap().doc.default_layer().unwrap();
    app.session.active_mut().unwrap().selection.target = Some(layer);
    assert_eq!(fresh_summary(&mut app).label, "Layer");
    app.session.active_mut().unwrap().selection.target = None;
    assert_eq!(fresh_summary(&mut app).label, "Mixed Objects");
    // Another document with the same ids selected at the same revision, where the first object's
    // opacity differs instead.
    let revision = app.session.active().unwrap().revision;
    assert!(fresh_summary(&mut app).mixed(&app).is_some_and(|m| !m.transparency));
    let (r2, t2) = art(&mut app);
    assert_eq!((&r2, t2), (&r, t), "the second document numbers its objects alike");
    app.run("transparency.set", json!({"ids": [r[0]], "opacity": 50})).unwrap();
    let st = app.session.active_mut().unwrap();
    st.selection.objects = vec![r[0], t, r[2]];
    st.selection.target = None;
    st.revision = revision;
    assert!(fresh_summary(&mut app).mixed(&app).is_some_and(|m| m.transparency), "the second document's own summary");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(fresh_summary(&mut app).mixed(&app).is_some_and(|m| !m.transparency), "the first document's again");
}

/// The panel's subject is the `Arc` the document holds for the selected object, a targeted group
/// or a targeted layer, not a copy of it; nothing with nothing selected.
#[test]
fn first_subject_is_the_node_the_document_holds() {
    let mut app = app_with_a_selected_rectangle();
    let held = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        super::super::layers::node_arc(&st.doc, st.selection.subjects()[0]).unwrap()
    };
    assert!(std::sync::Arc::ptr_eq(&first_subject(&app).expect("the rectangle"), &held(&app)), "the selected object");
    let group = app.run("object.group", json!({})).unwrap()["id"].clone();
    app.run("layer.target", json!({ "id": group })).unwrap();
    let n = first_subject(&app).expect("the group");
    assert_eq!(n.id.0, group.as_u64().unwrap());
    assert!(std::sync::Arc::ptr_eq(&n, &held(&app)), "the targeted group");
    let layer = app.session.active().unwrap().doc.layers[0].id;
    app.run("layer.target", json!({ "id": layer.0 })).unwrap();
    let n = first_subject(&app).expect("the layer");
    assert_eq!(n.id, layer);
    assert!(std::sync::Arc::ptr_eq(&n, &held(&app)), "the targeted layer");
    app.run("select.none", json!({})).unwrap();
    assert!(first_subject(&app).is_none(), "nothing selected");
}
