//! Selected Dimension extension-line controls through the real Properties panel.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::accesskit::{Node, NodeId as AccessId, Role};
use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{NodeId, NodeKind, dimension::Dimension};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> egui::FullOutput {
    let mut raw = RawInput { events, focused: true, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 1500.0))), ..Default::default() };
    app.raw_input_hook(ctx, &mut raw);
    ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    })
}

fn snapshot(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(AccessId, Node)> {
    let mut nodes = Vec::new();
    for _ in 0..4 {
        let mut output = frame(app, ctx, vec![]);
        nodes = output.platform_output.accesskit_update.map(|update| update.nodes).unwrap_or_default();
        output.textures_delta.clear();
    }
    nodes
}

fn named(node: &Node, text: &str) -> bool {
    node.label() == Some(text) || node.value() == Some(text)
}

fn number(node: &Node) -> Option<f64> {
    node.numeric_value().or_else(|| {
        // range_field displays the suffix without a space, for example "0.5pt".
        let text = node.value()?.trim();
        text.strip_suffix("pt").unwrap_or(text).trim().parse().ok()
    })
}

fn click_named(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
    let nodes = snapshot(app, ctx);
    let bounds = nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("missing real UI button: {label}"));
    let at = egui::pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);
    frame(app, ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at)]).textures_delta.clear();
    for pressed in [true, false] {
        frame(app, ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }])
            .textures_delta
            .clear();
    }
    let _ = snapshot(app, ctx);
}

/// Both real controls must sit in the Extension Lines row, rather than in another panel or dialog.
fn extension_controls(nodes: &[(AccessId, Node)]) -> Option<(AccessId, AccessId)> {
    let heading = nodes.iter().find(|(_, node)| named(node, "Extension Lines"))?.1.bounds()?;
    let in_row = |node: &Node| {
        node.bounds().is_some_and(|b| {
            let y = (b.y0 + b.y1) * 0.5;
            y >= heading.y1 && y <= heading.y1 + 60.0 && b.x0 >= heading.x0 - 2.0
        })
    };
    let weight = nodes.iter().find_map(|(label_id, label)| {
        if !named(label, "Line Weight:") || !in_row(label) {
            return None;
        }
        nodes
            .iter()
            .find(|(_, input)| input.labelled_by().contains(label_id) && number(input).is_some_and(|v| (v - 0.5).abs() < 1e-9))
            .map(|(id, _)| *id)
    })?;
    let color = nodes.iter().find(|(_, node)| node.role() == Role::ColorWell && in_row(node) && node.value() == Some("#000000ff"))?.0;
    Some((weight, color))
}

fn recipe(app: &VectorcraftApp, id: NodeId) -> Dimension {
    app.session.active().unwrap().doc.node(id).unwrap().extras().dimension.as_deref().unwrap().clone()
}

fn generated_styles(app: &VectorcraftApp, id: NodeId, extensions: [u8; 3], weight: f64) {
    let state = app.session.active().unwrap();
    let children = state.doc.node(id).unwrap().children().unwrap();
    let paths: Vec<_> = children.iter().filter(|node| matches!(&node.kind, NodeKind::Path { .. })).collect();
    assert_eq!(paths.len(), 3, "two extension paths and the main dimension line");
    for extension in &paths[..2] {
        let stroke = extension.appearance.stroke().unwrap();
        assert_eq!(stroke.width, weight);
        assert_eq!(stroke.paint, Paint::solid(Color::rgb8(extensions[0], extensions[1], extensions[2])));
    }
    let main = paths[2].appearance.stroke().unwrap();
    assert_eq!(main.width, 2.1);
    assert_eq!(main.paint, Paint::solid(Color::rgb8(255, 0, 0)));
    let text = children.iter().find(|node| matches!(&node.kind, NodeKind::Text(_))).unwrap();
    assert_eq!(text.appearance.fill().unwrap().paint, Paint::solid(Color::rgb8(0, 0, 255)));
}

#[test]
fn selected_dimension_properties_exposes_extension_weight_and_color() {
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.setFill", json!({"color": "#d0d0d0"})).unwrap();
    app.run("shape.ellipse", json!({"x": 100, "y": 129.435, "width": 41.13, "height": 41.13})).unwrap();
    let made = app
        .run(
            "dimension.create",
            json!({"a": [100,150], "b": [141.13,150], "orientation": "horizontal", "offset": -25,
        "options": {"units": "pt", "precision": 2, "lineWeight": 2.1, "lineColor": [255,0,0], "textColor": [0,0,255],
            "extensionWeight": 0.5, "extensionColor": [0,0,0]}}),
        )
        .unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    assert!(made["label"].as_str().unwrap().contains("41.13"));
    let original = recipe(&app, id);
    assert!((original.length - 41.13).abs() < 1e-9);
    generated_styles(&app, id, [0, 0, 0], 0.5);
    let document = app.session.active().unwrap().doc.clone();
    let paint = app.session.paint.clone();

    // Backend/artwork control only: it is restored before testing the missing selected UI route.
    let mut control = original.options.clone();
    control.extension_weight = 0.35;
    control.extension_color = [128, 128, 128];
    app.run("dimension.update", json!({"ids": [id.0], "options": control})).unwrap();
    generated_styles(&app, id, [128, 128, 128], 0.35);
    let changed = recipe(&app, id);
    assert_eq!((changed.a, changed.b, changed.offset, changed.length), (original.a, original.b, original.offset, original.length));
    assert_eq!(changed.label().unwrap(), original.label().unwrap());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(recipe(&app, id), original);
    assert_eq!(app.session.paint, paint);

    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Positive UI control: Tool Options already exposes extensions. Close it through its real
    // Cancel button so its heading and controls cannot satisfy the final Properties assertion.
    app.run("tool.options", json!({"tool": "dimension"})).unwrap();
    let tool_options = snapshot(&mut app, &ctx);
    assert!(tool_options.iter().any(|(_, node)| named(node, "Dimension Tool Options")));
    if extension_controls(&tool_options).is_none() {
        let header = tool_options.iter().find(|(_, node)| named(node, "Extension Lines")).and_then(|(_, node)| node.bounds());
        for (id, node) in &tool_options {
            let row = header.is_some_and(|h| {
                node.bounds().is_some_and(|b| {
                    let y = (b.y0 + b.y1) * 0.5;
                    y >= h.y1 && y <= h.y1 + 60.0 && b.x0 >= h.x0 - 2.0
                })
            });
            if row || !node.labelled_by().is_empty() {
                eprintln!(
                    "CONTROL {id:?} role={:?} label={:?} value={:?} numeric={:?} labelled_by={:?} bounds={:?}",
                    node.role(),
                    node.label(),
                    node.value(),
                    node.numeric_value(),
                    node.labelled_by(),
                    node.bounds()
                );
            }
        }
    }
    assert!(extension_controls(&tool_options).is_some(), "the existing Tool Options route has both extension controls");
    click_named(&mut app, &ctx, "Cancel");
    assert!(app.ui.dialog.is_none());
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.run("window.panel", json!({"panel": "properties"})).unwrap();
    let properties = snapshot(&mut app, &ctx);
    assert!(!properties.iter().any(|(_, node)| named(node, "Dimension Tool Options")));
    for label in ["Dimension", "Line Weight:", "Font Size:", "Apply to All", "Set as Default"] {
        assert!(properties.iter().any(|(_, node)| named(node, label)), "actual selected Properties baseline control: {label}");
    }
    assert!(properties.iter().any(|(_, node)| number(node).is_some_and(|v| (v - 2.1).abs() < 1e-9)), "the selected main-line weight is visible");
    for value in ["#ff0000ff", "#0000ffff"] {
        assert!(
            properties.iter().any(|(_, node)| node.role() == Role::ColorWell && node.value() == Some(value)),
            "main-line/text color control: {value}"
        );
    }
    assert_eq!(recipe(&app, id), original);
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(app.session.paint, paint);

    assert!(
        extension_controls(&properties).is_some(),
        "selected Properties must expose Extension Lines with an independent weight input and color control; Tool Options is closed"
    );
}

fn fixture() -> (VectorcraftApp, egui::Context, NodeId) {
    vectorcraft_ui_egui::i18n::set_current(vectorcraft_ui_egui::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.setFill", json!({"color": "#d0d0d0"})).unwrap();
    app.run("shape.ellipse", json!({"x": 100, "y": 129.435, "width": 41.13, "height": 41.13})).unwrap();
    let made = app
        .run(
            "dimension.create",
            json!({"a": [100,150], "b": [141.13,150], "orientation": "horizontal", "offset": -25,
        "options": {"units": "pt", "precision": 2, "lineWeight": 2.1, "lineColor": [255,0,0], "textColor": [0,0,255],
            "extensionWeight": 0.5, "extensionColor": [0,0,0]}}),
        )
        .unwrap();
    let id = NodeId(made["id"].as_u64().unwrap());
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.session.prefs.monitor_profile_display = false;
    app.canvas.worker_started = true;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    ctx.all_styles_mut(|style| style.visuals.numeric_color_space = egui::style::NumericColorSpace::GammaByte);
    let _ = snapshot(&mut app, &ctx);
    let view = app.view_mut().unwrap();
    view.zoom = 8.0;
    view.center = vectorcraft_geom::Point::new(120.565, 140.0);
    view.rotation = 0.0;
    select_properties(&mut app, &ctx, id);
    (app, ctx, id)
}

fn select_properties(app: &mut VectorcraftApp, ctx: &egui::Context, id: NodeId) {
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.run("window.panel", json!({"panel": "properties"})).unwrap();
    let nodes = snapshot(app, ctx);
    assert!(nodes.iter().any(|(_, node)| named(node, "Apply to All")));
}

fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
    frame(app, ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at)]).textures_delta.clear();
    for pressed in [true, false] {
        frame(app, ctx, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }])
            .textures_delta
            .clear();
    }
    let _ = snapshot(app, ctx);
}

fn node_center(nodes: &[(AccessId, Node)], id: AccessId) -> Pos2 {
    let b = nodes.iter().find(|(candidate, _)| *candidate == id).unwrap().1.bounds().unwrap();
    egui::pos2(((b.x0 + b.x1) * 0.5) as f32, ((b.y0 + b.y1) * 0.5) as f32)
}

fn key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }])
            .textures_delta
            .clear();
    }
}

fn type_at(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2, text: &str) {
    click_at(app, ctx, at);
    key(app, ctx, egui::Key::A, Modifiers { ctrl: true, command: true, ..Modifiers::NONE });
    frame(app, ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::Text(text.into())]).textures_delta.clear();
    key(app, ctx, egui::Key::Enter, Modifiers::NONE);
    let _ = snapshot(app, ctx);
}

fn extension_pair(nodes: &[(AccessId, Node)], weight: f64, color: &str) -> (AccessId, AccessId) {
    let heading = nodes.iter().find(|(_, node)| named(node, "Extension Lines")).unwrap().1.bounds().unwrap();
    let in_row = |node: &Node| {
        node.bounds().is_some_and(|b| {
            let y = (b.y0 + b.y1) * 0.5;
            y >= heading.y1 && y <= heading.y1 + 60.0 && b.x0 >= heading.x0 - 2.0
        })
    };
    let input = nodes
        .iter()
        .find_map(|(label_id, label)| {
            if !named(label, "Line Weight:") || !in_row(label) {
                return None;
            }
            nodes
                .iter()
                .find(|(_, input)| input.labelled_by().contains(label_id) && number(input).is_some_and(|v| (v - weight).abs() < 1e-9))
                .map(|(id, _)| *id)
        })
        .expect("the selected extension weight's actual input");
    let color = nodes
        .iter()
        .find(|(_, node)| node.role() == Role::ColorWell && in_row(node) && node.value() == Some(color))
        .expect("the selected extension ColorWell")
        .0;
    (input, color)
}

fn edit_weight(app: &mut VectorcraftApp, ctx: &egui::Context, old: f64, color: &str, next: &str) {
    let nodes = snapshot(app, ctx);
    let (input, _) = extension_pair(&nodes, old, color);
    type_at(app, ctx, node_center(&nodes, input), next);
}

fn edit_gray(app: &mut VectorcraftApp, ctx: &egui::Context, weight: f64) -> serde_json::Value {
    let nodes = snapshot(app, ctx);
    let (input, color) = extension_pair(&nodes, weight, "#000000ff");
    let control_bounds = json!({"weight": format!("{:?}", nodes.iter().find(|(id, _)| *id == input).unwrap().1.bounds()),
        "color": format!("{:?}", nodes.iter().find(|(id, _)| *id == color).unwrap().1.bounds())});
    click_at(app, ctx, node_center(&nodes, color));
    // The pinned egui picker uses existing R/G/B DragValues (prefix names), not a hex field.
    for channel in ["R", "G", "B"] {
        let nodes = snapshot(app, ctx);
        let id = nodes.iter().find(|(_, node)| node.label() == Some(channel)).unwrap_or_else(|| panic!("actual picker channel {channel}")).0;
        type_at(app, ctx, node_center(&nodes, id), "128");
    }
    key(app, ctx, egui::Key::Escape, Modifiers::NONE);
    let _ = snapshot(app, ctx);
    control_bounds
}

fn geometry(dimension: &Dimension) -> serde_json::Value {
    let mut value = serde_json::to_value(dimension).unwrap();
    value.as_object_mut().unwrap().remove("options");
    value
}

/// Capture the actual canvas texture using ordinary view changes to request a full upload.
fn capture(app: &mut VectorcraftApp, ctx: &egui::Context) -> (vectorcraft_render::Rendered, vectorcraft_geom::Affine) {
    let center = app.view().unwrap().center;
    app.view_mut().unwrap().center.x += 1.0;
    frame(app, ctx, vec![]).textures_delta.clear();
    app.view_mut().unwrap().center = center;
    let mut output = frame(app, ctx, vec![]);
    let texture = app.canvas.texture.as_ref().unwrap().id();
    let upload =
        output.textures_delta.set.iter().find(|(id, _)| **id == texture).and_then(|(_, updates)| updates.last()).expect("actual CPU canvas upload");
    assert!(upload.pos.is_none(), "the view change requests a full texture");
    let egui::ImageData::Color(image) = &upload.image;
    let rendered = vectorcraft_render::Rendered {
        width: image.width() as u32,
        height: image.height() as u32,
        pixels: image.pixels.iter().flat_map(|color| color.to_array()).collect(),
    };
    let cache = app.canvas.key.as_ref().unwrap();
    assert!(cache.pixel.is_none());
    assert_eq!(cache.doc, app.session.active().unwrap().uid as usize);
    assert_eq!(cache.revision, app.session.active().unwrap().revision);
    assert_eq!((cache.w, cache.h), (rendered.width, rendered.height));
    // The existing frame_view mapping, bound to this actual upload's public cache key.
    let transform = vectorcraft_geom::Affine::translate((f64::from(cache.w) / 2.0, f64::from(cache.h) / 2.0))
        * vectorcraft_geom::Affine::rotate(cache.rot.to_radians())
        * vectorcraft_geom::Affine::scale(cache.zoom * f64::from(cache.ppp))
        * vectorcraft_geom::Affine::translate((-cache.cx, -cache.cy));
    output.textures_delta.clear();
    (rendered, transform)
}

fn extension_coverage(image: &vectorcraft_render::Rendered, transform: vectorcraft_geom::Affine, x: f64, rgb: [u8; 3]) -> f64 {
    let point = transform * vectorcraft_geom::Point::new(x, 137.5);
    let (x, y) = (point.x.floor() as usize, point.y.floor() as usize);
    assert!(x >= 6 && x + 6 < image.width as usize && y < image.height as usize);
    let mut coverage = 0.0;
    let mut solid = false;
    for column in x - 6..=x + 6 {
        let at = (y * image.width as usize + column) * 4;
        let pixel = &image.pixels[at..at + 4];
        coverage += f64::from(pixel[3]) / 255.0;
        solid |= pixel[3] == 255 && pixel[..3] == rgb;
    }
    assert!(solid, "actual extension midpoint has the expected opaque color {rgb:?}");
    coverage
}

#[test]
fn properties_edits_both_extensions_preserves_main_text_and_roundtrips_editable_recipe() {
    let (mut app, ctx, id) = fixture();
    let original = recipe(&app, id);
    let selection = app.session.active().unwrap().selection.clone();
    let paint = app.session.paint.clone();
    let (before, before_xf) = capture(&mut app, &ctx);
    edit_weight(&mut app, &ctx, 0.5, "#000000ff", "0.35");
    let weighted = recipe(&app, id);
    assert_eq!(weighted.options.extension_weight, 0.35);
    assert_eq!(geometry(&weighted), geometry(&original));
    generated_styles(&app, id, [0, 0, 0], 0.35);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(recipe(&app, id), original);
    generated_styles(&app, id, [0, 0, 0], 0.5);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(recipe(&app, id), weighted);
    let _ = snapshot(&mut app, &ctx);
    let controls = edit_gray(&mut app, &ctx, 0.35);
    let edited = recipe(&app, id);
    let mut expected = original.clone();
    expected.options.extension_weight = 0.35;
    expected.options.extension_color = [128, 128, 128];
    assert_eq!(edited, expected, "only the selected independent extension fields change");
    generated_styles(&app, id, [128, 128, 128], 0.35);
    assert_eq!(app.session.active().unwrap().selection, selection);
    assert_eq!(app.session.paint, paint);
    let (after, after_xf) = capture(&mut app, &ctx);
    for x in [100.0, 141.13] {
        let old = extension_coverage(&before, before_xf, x, [0, 0, 0]);
        let new = extension_coverage(&after, after_xf, x, [128, 128, 128]);
        assert!(new < old, "both actual extension lines become thinner: {new} < {old}");
    }
    for rgb in [[255, 0, 0], [0, 0, 255]] {
        for image in [&before, &after] {
            assert!(
                image.pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] == 255 && pixel[..3] == rgb),
                "main/text color remains on the actual canvas: {rgb:?}"
            );
        }
    }
    if let Some(dir) = std::env::var_os("VECTORCRAFT_DIMENSION_EVIDENCE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("before.png"), before.to_png().unwrap()).unwrap();
        std::fs::write(dir.join("after.png"), after.to_png().unwrap()).unwrap();
        std::fs::write(
            dir.join("binding.json"),
            serde_json::to_vec_pretty(&json!({"doc": app.session.active().unwrap().uid,
            "target": id.0, "revision": app.session.active().unwrap().revision, "recipe": edited, "controls": controls}))
            .unwrap(),
        )
        .unwrap();
    }

    // Use the existing native save/open commands in memory; no asset file is needed.
    let saved = app.run("document.save", json!({"format": "vectorcraft", "preview": false})).unwrap();
    let data = saved["dataBase64"].as_str().expect("an untitled native save returns bytes");
    app.run("document.open", json!({"name": "dimension.vectorcraft", "dataBase64": data})).unwrap();
    assert_eq!(recipe(&app, id), edited);
    select_properties(&mut app, &ctx, id);
    edit_weight(&mut app, &ctx, 0.35, "#808080ff", "0.6");
    assert_eq!(recipe(&app, id).options.extension_weight, 0.6, "the native recipe remains editable through Properties");
    assert_eq!(geometry(&recipe(&app, id)), geometry(&edited));
    generated_styles(&app, id, [128, 128, 128], 0.6);
    assert_eq!(app.session.paint, paint);
}

#[test]
fn properties_apply_all_copies_full_style_to_editable_dimensions_and_undoes_as_one_edit() {
    let (mut app, ctx, id) = fixture();
    let other = NodeId(
        app.run(
            "dimension.create",
            json!({"a": [200,150], "b": [241.13,150], "offset": -30,
        "options": {"lineWeight": 1.3, "lineColor": [0,128,0], "extensionWeight": 0.8, "extensionColor": [0,128,0]}}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    let locked = NodeId(
        app.run(
            "dimension.create",
            json!({"a": [100,220], "b": [141.13,220], "offset": -20,
        "options": {"lineWeight": 1.7, "lineColor": [192,0,192], "extensionWeight": 0.9, "extensionColor": [192,0,192]}}),
        )
        .unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    app.run("object.lock", json!({"ids": [locked.0]})).unwrap();
    let other_original = recipe(&app, other);
    let locked_original = recipe(&app, locked);
    select_properties(&mut app, &ctx, id);
    edit_weight(&mut app, &ctx, 0.5, "#000000ff", "0.35");
    let _ = edit_gray(&mut app, &ctx, 0.35);
    assert_eq!(recipe(&app, other), other_original, "selected edits do not change an unselected dimension");
    assert_eq!(recipe(&app, locked), locked_original);
    let selected = recipe(&app, id);
    let before = app.session.active().unwrap().doc.clone();
    let undo = app.session.active().unwrap().history.undo.len();
    let paint = app.session.paint.clone();
    click_named(&mut app, &ctx, "Apply to All");
    assert_eq!(recipe(&app, other).options, selected.options, "Apply to All copies the complete style, not only extensions");
    assert_eq!(geometry(&recipe(&app, other)), geometry(&other_original));
    assert_eq!(recipe(&app, locked), locked_original, "the locked control is untouched");
    assert_eq!(recipe(&app, id), selected);
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    let after = app.session.active().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, after);
    assert_eq!(app.session.paint, paint);
}

#[test]
fn properties_set_default_is_inherited_by_an_options_free_new_dimension() {
    let (mut app, ctx, id) = fixture();
    let other = NodeId(
        app.run("dimension.create", json!({"a": [200,150], "b": [241.13,150], "options": {"extensionWeight": 0.8}})).unwrap()["id"].as_u64().unwrap(),
    );
    let other_original = recipe(&app, other);
    select_properties(&mut app, &ctx, id);
    edit_weight(&mut app, &ctx, 0.5, "#000000ff", "0.35");
    let _ = edit_gray(&mut app, &ctx, 0.35);
    let selected = recipe(&app, id);
    let document = app.session.active().unwrap().doc.clone();
    let history = (app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len());
    let paint = app.session.paint.clone();
    click_named(&mut app, &ctx, "Set as Default");
    assert_eq!(app.session.active().unwrap().doc, document, "setting tool defaults does not apply them to existing dimensions");
    assert_eq!((app.session.active().unwrap().history.undo.len(), app.session.active().unwrap().history.redo.len()), history);
    assert_eq!(recipe(&app, other), other_original);
    app.run("window.panel.close", json!({"panel": "properties"})).unwrap();
    app.run("tool.options", json!({"tool": "dimension"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["extensionWeight"], json!(0.35));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["extensionColor"], json!([128, 128, 128]));
    // Enter is this modal's legitimate Save action; Tab commits only the focused draft field.
    let nodes = snapshot(&mut app, &ctx);
    let (input, _) = extension_pair(&nodes, 0.35, "#808080ff");
    click_at(&mut app, &ctx, node_center(&nodes, input));
    key(&mut app, &ctx, egui::Key::A, Modifiers { ctrl: true, command: true, ..Modifiers::NONE });
    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::NONE), Event::Text("0.9".into())]).textures_delta.clear();
    key(&mut app, &ctx, egui::Key::Tab, Modifiers::NONE);
    let _ = snapshot(&mut app, &ctx);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["extensionWeight"], json!(0.9), "Tool Options edits a draft until Save");
    click_named(&mut app, &ctx, "Cancel");
    let new = NodeId(app.run("dimension.create", json!({"a": [100,220], "b": [141.13,220], "offset": -20})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(recipe(&app, new).options, selected.options, "no explicit options: new dimensions inherit the full style");
    assert_eq!(recipe(&app, id), selected);
    assert_eq!(recipe(&app, other), other_original);
    assert_eq!(app.session.paint, paint);
}
