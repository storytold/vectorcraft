use egui::accesskit::{Role, Toggled};
use egui::{Event, PointerButton, Pos2};
use vectorcraft_engine::Session;

use super::*;
use crate::widgets::accessibility_tests::{role_and_state, tree};

/// One headless frame of the toolbar; returns the tool buttons' rects, top to bottom.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<egui::Rect> {
    frame_in(app, ctx, time, events, 1200.0)
}

/// [`frame`] in a window `height` points tall.
fn frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>, height: f32) -> Vec<egui::Rect> {
    let input = egui::RawInput {
        time: Some(time),
        events,
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, height))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| show(app, ui));
    out.textures_delta.clear();
    widget_rects(ctx, vec2(36.0, PITCH - 1.0))
}

/// The rects of the last frame's widgets of `size`, top to bottom.
pub(crate) fn widget_rects(ctx: &egui::Context, size: egui::Vec2) -> Vec<egui::Rect> {
    let mut r: Vec<egui::Rect> =
        ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size).map(|w| w.rect).collect());
    r.sort_by(|a, b| a.top().total_cmp(&b.top()));
    r
}

/// A mouse wheel turn over `at`, scrolling the content up by `dy` points.
pub(crate) fn wheel(at: Pos2, dy: f32) -> Vec<Event> {
    let unit = egui::MouseWheelUnit::Point;
    vec![
        Event::PointerMoved(at),
        Event::MouseWheel {
            unit,
            delta: vec2(0.0, -dy),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]
}

#[test]
fn the_toolbar_scrolls_in_a_short_window() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    // 500 pt: even two columns don't fit, so Edit Toolbar (the last button) starts below the window.
    let tools = frame_in(&mut app, &ctx, 0.0, vec![], 500.0);
    let edit_toolbar = || widget_rects(&ctx, vec2(26.0, 26.0)).last().copied().unwrap();
    assert!(edit_toolbar().bottom() > 500.0);
    frame_in(&mut app, &ctx, 0.1, wheel(tools[0].center(), 2000.0), 500.0);
    for k in 2..40 {
        frame_in(&mut app, &ctx, f64::from(k) * 0.1, vec![], 500.0);
    }
    assert!(edit_toolbar().bottom() <= 500.0, "scrolled into view: {:?}", edit_toolbar());
    // Back up to the top.
    frame_in(&mut app, &ctx, 4.0, wheel(tools[0].center(), -2000.0), 500.0);
    for k in 41..80 {
        frame_in(&mut app, &ctx, f64::from(k) * 0.1, vec![], 500.0);
    }
    assert_eq!(frame_in(&mut app, &ctx, 8.0, vec![], 500.0)[0], tools[0]);
}

/// The tool buttons sit in the middle of the panel, between its border and the separator
/// line, docked and floating, in one or two columns (docked, they sat 2½ pt right of centre).
#[test]
fn the_tool_buttons_are_centred_in_the_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    for (double, floating) in [(false, None), (true, None), (false, Some([100.0, 100.0])), (true, Some([100.0, 100.0]))] {
        app.run("window.toolbarColumns", json!({ "double": double })).unwrap();
        app.ui.toolbar_pos = floating;
        frame(&mut app, &ctx, 0.0, vec![]);
        let tools = frame(&mut app, &ctx, 0.1, vec![]);
        // The title bar spans the panel's inside.
        let hdr = ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.rect.height() == 14.0 && w.rect.width() > 40.0)
                .map(|w| w.rect)
                .next()
        });
        let hdr = hdr.unwrap();
        let first = tools[0];
        let row_right = tools.iter().filter(|r| r.top() == first.top()).map(|r| r.right()).fold(first.right(), f32::max);
        assert_eq!(row_right - hdr.center().x, hdr.center().x - first.left(), "double {double}, floating {floating:?}: {first:?} in {hdr:?}");
        assert!(first.left() >= hdr.left() && row_right <= hdr.right(), "double {double}, floating {floating:?}: {first:?} in {hdr:?}");
    }
}

/// Screen readers name each tool button by its tool tip and say which tool is active, and name
/// the title bar's column toggle and grip and the paint buttons under the tools.
#[test]
fn screen_readers_name_the_tool_buttons_and_the_active_tool() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.select_tool("pen");
    let tree = tree(|ui| show(&mut app, ui));
    let name = |id| tool_info(id).map(tip).unwrap_or_default();
    assert_eq!(role_and_state(&tree, &name("pen")), (Role::Button, Some(Toggled::True)));
    assert_eq!(role_and_state(&tree, &name("selection")), (Role::Button, Some(Toggled::False)));
    for label in ["Toggle single/double column", "Drag to float the toolbar", "Color (,)", "Gradient (.)", "None (/)"] {
        assert_eq!(role_and_state(&tree, label), (Role::Button, None), "{label}");
    }
}

/// #812: double-clicking the Artboard tool opens Artboard Options of the active artboard, a
/// graph tool Graph Type for the selected graph, and the Magic Wand tool its panel.
#[test]
fn more_tools_open_their_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    // Without the Artboard tool, the window's artboard.
    assert_eq!(open_options(&mut app, "artboard").unwrap()["dialog"], "artboardOptions");
    let d = app.ui.dialog.take().unwrap();
    assert_eq!((d.f64("index", 9.0), d.f64("width", 0.0), d.str("name")), (0.0, 300.0, "Artboard 1".to_string()));
    // With it, its active artboard.
    app.run("artboard.new", json!({"x": 400, "y": 0, "width": 100, "height": 50})).unwrap();
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    app.run("tool.setOption", json!({"key": "active", "value": 1})).unwrap();
    open_options(&mut app, "artboard").unwrap();
    let d = app.ui.dialog.take().unwrap();
    assert_eq!((d.f64("index", 9.0), d.f64("x", 0.0), d.f64("width", 0.0)), (1.0, 400.0, 100.0));
    // Graph Type needs a selected graph.
    app.run("select.set", json!({"ids": []})).unwrap();
    assert!(open_options(&mut app, "pieGraph").is_err());
    assert!(app.ui.dialog.is_none());
    app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150})).unwrap();
    assert_eq!(open_options(&mut app, "pieGraph").unwrap()["dialog"], "command");
    let d = app.ui.dialog.take().unwrap();
    assert_eq!((d.str("__command"), d.str("type")), ("graph.setType".to_string(), "column".to_string()));
    // The Magic Wand panel opens, and stays open on a second double-click.
    open_options(&mut app, "magicWand").unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "magicWand"));
    open_options(&mut app, "magicWand").unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "magicWand"));
}

/// A double-click at `at`, `time` seconds in (a second apart from the last).
fn double_click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2) {
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, time, vec![Event::PointerMoved(at), b(true), b(false), b(true), b(false)]);
    frame(app, ctx, time + 0.1, vec![]);
}

#[test]
fn double_clicking_a_tool_button_opens_its_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let buttons = frame(&mut app, &ctx, 0.0, vec![]);
    // The Color category's slots close the list: Gradient, then Eyedropper.
    let (gradient, eyedropper) = (buttons[buttons.len() - 2], buttons[buttons.len() - 1]);
    double_click(&mut app, &ctx, 1.0, gradient.center());
    assert_eq!((app.session.tool_id(), app.ui.open_panel.as_deref()), ("gradient", Some("gradient")));
    // Again: the panel stays open.
    double_click(&mut app, &ctx, 2.0, gradient.center());
    assert_eq!(app.ui.open_panel.as_deref(), Some("gradient"));
    double_click(&mut app, &ctx, 3.0, eyedropper.center());
    let d = app.ui.dialog.clone().expect("Eyedropper Options");
    assert_eq!((app.session.tool_id(), d.kind.as_str()), ("eyedropper", crate::dialogs::eyedropper::KIND));
    // OK applies the options.
    app.ui.dialog.as_mut().unwrap().fields.insert("apply".into(), json!({"appearance": {"transparency": false}}));
    crate::dialogs::confirm(&mut app).unwrap();
    let o = app.session.prefs.eyedropper;
    assert!(o.pick_up.appearance.transparency && !o.apply.appearance.transparency && o.apply.appearance.fill.color);
    assert!(app.run("tool.options", json!({"tool": "lasso"})).is_err());
}

/// A click on the Blend tool's button while the Blend tool holds a first pick finishes the
/// blend: the tool stays chosen, and the next click on the canvas picks a new first object
/// instead of blending with the first.
#[test]
fn clicking_the_active_blend_tool_button_finishes_the_blend() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui.toolbar_advanced = true;
    app.run("file.new", json!({"width": 600, "height": 300})).unwrap();
    for x in [0, 200] {
        app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap();
    }
    app.select_tool("blend");
    let index = slots(&app).iter().position(|(_, tools)| tools.contains(&"blend")).expect("a toolbar slot");
    let ctx = egui::Context::default();
    let at = frame_in(&mut app, &ctx, 0.0, vec![], 2000.0)[index].center();
    let down = |x, y| vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Down, x, y);
    let view = app.view_info();
    app.session.pointer(&down(20.0, 20.0), view).unwrap();
    assert!(app.session.tool_busy(), "the first rectangle is picked");
    let layers = app.session.active().unwrap().doc.layers.clone();
    click_in(&mut app, &ctx, 1.0, at, PointerButton::Primary, 2000.0);
    assert_eq!(app.session.tool_id(), "blend");
    assert!(!app.session.tool_busy(), "the click on the button drops the pick");
    app.session.pointer(&down(220.0, 20.0), view).unwrap();
    assert!(app.session.tool_busy(), "the second rectangle is a new first pick");
    assert_eq!(app.session.active().unwrap().doc.layers, layers, "no blend was made");
}

/// A double-click on the Type on a Path or Vertical Type on a Path tool opens Type on a Path
/// Options for the selected type on a path, as the menu item does; with nothing selected, or
/// with no type on a path selected, nothing opens.
#[test]
fn double_clicking_a_type_on_a_path_tool_opens_type_on_a_path_options() {
    for tool in ["typeOnPath", "verticalTypeOnPath"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.toolbar_advanced = true;
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        app.select_tool(tool);
        let index = slots(&app).iter().position(|(_, tools)| tools.contains(&tool)).expect("a toolbar slot");
        let ctx = egui::Context::default();
        let at = frame_in(&mut app, &ctx, 0.0, vec![], 2000.0)[index].center();
        let double_click = |app: &mut VectorcraftApp, time: f64| {
            let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame_in(app, &ctx, time, vec![Event::PointerMoved(at), b(true), b(false), b(true), b(false)], 2000.0);
            frame_in(app, &ctx, time + 0.1, vec![], 2000.0);
        };
        // Nothing selected: nothing opens.
        double_click(&mut app, 1.0);
        assert!(app.ui.dialog.is_none(), "{tool}");
        assert_eq!(app.run("tool.options", json!({ "tool": tool })), Err("nothing selected".into()));
        // A path that holds no type: nothing opens either.
        let path = app.run("path.create", json!({"d": "M100 200 L500 200"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [path]})).unwrap();
        double_click(&mut app, 2.0);
        assert!(app.ui.dialog.is_none(), "{tool}");
        let e = app.run("tool.options", json!({ "tool": tool })).unwrap_err();
        assert!(e.contains("`type.pathOptions`: no text objects selected"), "{tool}: {e}");
        assert!(app.ui.dialog.is_none(), "{tool}");
        // Type on a path selected: the dialog opens on its options.
        let r = app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [300, 210]})).unwrap();
        app.run("select.set", json!({"ids": [r["id"]]})).unwrap();
        app.run("type.pathOptions", json!({"effect": "skew"})).unwrap();
        assert_eq!(app.session.tool_id(), tool);
        double_click(&mut app, 3.0);
        let d = app.ui.dialog.clone().unwrap_or_else(|| panic!("{tool}: Type on a Path Options"));
        assert_eq!((d.str("__command"), d.str("__label")), ("type.pathOptions".to_string(), "Type on a Path Options".to_string()));
        assert_eq!(d.fields.get("effect"), Some(&json!("skew")), "{tool}");
        assert!(!d.fields.contains_key("start") && !d.fields.contains_key("end"), "{tool}: the menu item's fields");
        // OK applies what the dialog holds.
        app.ui.dialog.as_mut().unwrap().fields.insert("effect".into(), json!("gravity"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.execute("type.pathOptions", &json!({})).unwrap()["effect"], "gravity", "{tool}");
    }
}

#[test]
fn polygon_and_star_canvas_counts_persist_without_new_toolbar_dialogs() {
    for (tool, key, count) in [("polygon", "sides", 3), ("star", "points", 7)] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":300,"height":300})).unwrap();
        app.select_tool(tool);
        assert!(open_options(&mut app, tool).is_err(), "upstream toolbar opening behavior stays unchanged");
        assert!(app.ui.dialog.is_none());
        let objects =
            |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum::<usize>();
        crate::dialogs::open_tool_dialog(&mut app, tool, json!({"x":120,"y":130}));
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(d.fields.contains_key(if tool == "polygon" { "radius" } else { "radius1" }));
        d.fields.insert(key.into(), json!(count));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(objects(&app), 1, "deliberate exact-size placement");
        assert_eq!(app.session.tool_options()[key], count);
        for round in 0..3 {
            if round == 2 {
                app.select_tool("ellipse");
                app.select_tool(tool);
            }
            let view = app.view_info();
            for (kind, x, y) in [
                (vectorcraft_tools::PointerKind::Down, 50.0, 50.0),
                (vectorcraft_tools::PointerKind::Drag, 90.0, 90.0),
                (vectorcraft_tools::PointerKind::Up, 90.0, 90.0),
            ] {
                app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, x, y), view).unwrap();
            }
            assert_eq!(objects(&app), round + 2);
            let st = app.session.active().unwrap();
            let node = st.doc.node(*st.selection.objects.first().unwrap()).unwrap();
            assert_eq!(node.path_data().unwrap().anchor_count(), if tool == "polygon" { 3 } else { 14 });
        }
        crate::dialogs::open_tool_dialog(&mut app, tool, json!({"x":140,"y":150}));
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields[key], count, "reopening uses the chosen count");
        d.fields.insert(key.into(), json!(9));
        crate::dialogs::cancel(&mut app);
        assert_eq!(objects(&app), 4);
        assert_eq!(app.session.tool_options()[key], count, "Cancel keeps the count");
    }
}

/// #2691: a double-click on the Line Segment, Arc, Rectangular Grid or Polar Grid tool opens
/// its Tool Options (the Rectangular Grid and Arc help pages, step 3). Cancel leaves the next
/// drag as it was; OK draws nothing and the next drag, and a click's dialog, use the options.
#[test]
fn double_clicking_the_line_arc_and_grid_tools_opens_their_tool_options() {
    // (tool, the option changed, its new value, the drag's command, the param it shows in)
    let cases = [
        ("lineSegment", "fillLine", json!(true), "shape.line", "fill"),
        ("arc", "slope", json!(-30.0), "shape.arc", "slope"),
        ("rectangularGrid", "rows", json!(2), "shape.rectangularGrid", "rows"),
        ("polarGrid", "radial", json!(3), "shape.polarGrid", "radial"),
    ];
    let objects = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum::<usize>();
    let drag = |app: &mut VectorcraftApp| {
        let view = app.view_info();
        for (kind, x, y) in [
            (vectorcraft_tools::PointerKind::Down, 50.0, 50.0),
            (vectorcraft_tools::PointerKind::Drag, 150.0, 120.0),
            (vectorcraft_tools::PointerKind::Up, 150.0, 120.0),
        ] {
            app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, x, y), view).unwrap();
        }
        app.session.journal.last().cloned().expect("the drag's command")
    };
    for (tool, key, value, cmd, param) in cases {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.toolbar_advanced = true;
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.select_tool(tool);
        let index = slots(&app).iter().position(|(_, tools)| tools.contains(&tool)).expect("a toolbar slot");
        let ctx = egui::Context::default();
        let at = frame_in(&mut app, &ctx, 0.0, vec![], 2000.0)[index].center();
        let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame_in(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), b(true), b(false), b(true), b(false)], 2000.0);
        let d = app.ui.dialog.clone().unwrap_or_else(|| panic!("{tool}: Tool Options"));
        assert_eq!((d.kind.as_str(), d.bool("__toolOptions")), (tool, true));
        assert!(!d.fields.contains_key("x"), "{tool}: no click to draw at");
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        let title = match tool {
            "lineSegment" => "Line Segment Tool Options",
            "arc" => "Arc Segment Tool Options",
            "rectangularGrid" => "Rectangular Grid Tool Options",
            _ => "Polar Grid Tool Options",
        };
        assert!(text.contains(title) && !text.contains("Reference:"), "{tool}: {text}");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(key.into(), value.clone());
        crate::dialogs::cancel(&mut app);
        let (c, p) = drag(&mut app);
        assert_eq!(c, cmd);
        assert_ne!(p.get(param), Some(&value), "{tool}: Cancel changes nothing");
        assert_eq!(objects(&app), 1);
        app.run("tool.options", json!({ "tool": tool })).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert(key.into(), value.clone());
        if tool.ends_with("Grid") {
            d.fields.insert("width".into(), json!("150 pt"));
        }
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(objects(&app), 1, "{tool}: OK draws nothing");
        let (c, p) = drag(&mut app);
        assert_eq!((c.as_str(), &p[param]), (cmd, &value), "{tool}: the next drag uses the option");
        // A click's dialog opens at them too, a grid at its Default Size.
        crate::dialogs::open_tool_dialog(&mut app, tool, json!({"x": 100, "y": 100}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.fields[key], value, "{tool}");
        if tool.ends_with("Grid") {
            assert_eq!(d.f64("width", 0.0), 150.0);
        }
    }
}

#[test]
fn double_clicking_the_hand_zoom_and_transform_tools() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.canvas_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(660.0, 460.0)));
    app.run("view.setZoom", json!({"zoom": 333, "center": [10, 10]})).unwrap();
    // Hand: the artboard fits the window. Zoom: 100%.
    app.run("tool.options", json!({"tool": "hand"})).unwrap();
    let v = *app.view().unwrap();
    assert_eq!((v.center.x, v.center.y, v.zoom), (150.0, 100.0, 2.0));
    app.run("tool.options", json!({"tool": "zoom"})).unwrap();
    assert_eq!(app.view().unwrap().zoom, 1.0);
    // The transform tools open their dialogs, like Object › Transform: not with nothing selected.
    assert_eq!(app.run("tool.options", json!({"tool": "rotate"})), Err("nothing selected".into()));
    assert!(app.ui.dialog.is_none());
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
    for tool in ["rotate", "scale", "reflect", "shear"] {
        app.run("tool.options", json!({"tool": tool})).unwrap();
        assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some(tool.to_string()));
    }
    // OK in Rotate turns the selection about its centre.
    app.run("tool.options", json!({"tool": "rotate"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("angle".into(), json!(90));
    crate::dialogs::confirm(&mut app).unwrap();
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert!((b.x0 - 20.0).abs() < 1e-9 && (b.y0 - 0.0).abs() < 1e-9 && (b.width() - 20.0).abs() < 1e-9, "{b:?}");
}

#[test]
fn double_clicking_a_selection_tool_opens_the_move_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    // The Selection tool's button is the first one.
    let selection = frame(&mut app, &ctx, 0.0, vec![])[0];
    // Nothing selected: Move is disabled, so nothing opens.
    double_click(&mut app, &ctx, 1.0, selection.center());
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.run("tool.options", json!({"tool": "directSelection"})), Err("nothing selected".into()));
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
    double_click(&mut app, &ctx, 2.0, selection.center());
    assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some("move".to_string()));
    // The Direct Selection and Group Selection tools open it too.
    for tool in ["directSelection", "groupSelection"] {
        app.run("tool.options", json!({"tool": tool})).unwrap();
        assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some("move".to_string()), "{tool}");
    }
    // OK moves the selection by what was typed.
    app.run("tool.options", json!({"tool": "selection"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("dx".into(), json!("25 pt"));
    crate::dialogs::confirm(&mut app).unwrap();
    let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0), (35.0, 10.0));
}

/// #1616: in every language a flyout is wide enough that no tool's name runs into its shortcut
/// (the Spanish Live Paint names need more than the 250 pt the English ones fit in).
#[test]
fn a_flyout_is_as_wide_as_its_longest_translated_row() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for lang in crate::i18n::Lang::all() {
            for group in TOOL_GROUPS {
                let rows: Vec<RowText> = group.iter().map(|tool| row_text(ui, lang, tool, Color32::WHITE)).collect();
                let w = rows_width(rows.iter());
                assert!(w >= 250.0, "{lang:?}");
                for (tool, (name, sc)) in group.iter().zip(&rows) {
                    let name_end = NAME_X + name.size().x;
                    let shortcut_start = w - ROW_END - sc.as_ref().map_or(0.0, |sc| sc.size().x);
                    assert!(
                        name_end <= shortcut_start + 0.01,
                        "{lang:?} {}: the name ends at {name_end}, the shortcut starts at {shortcut_start}",
                        tool.id
                    );
                }
            }
        }
        let es = crate::i18n::Lang::from_code("es").unwrap();
        let live_paint = TOOL_GROUPS.iter().find(|g| g.iter().any(|t| t.id == "livePaintSelection")).expect("the Live Paint group");
        let rows: Vec<RowText> = live_paint.iter().map(|tool| row_text(ui, es, tool, Color32::WHITE)).collect();
        assert!(rows_width(rows.iter()) > 250.0, "the Spanish Live Paint flyout is wider than the English one");
    });
    out.textures_delta.clear();
}

#[test]
fn right_clicking_a_tool_group_opens_its_flyout() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let buttons = frame(&mut app, &ctx, 0.0, vec![]);
    // Selection, Direct Selection, Lasso, then the Shapes group (Rectangle and its sub-tools).
    let at = buttons[3].center();
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), b(true)]);
    frame(&mut app, &ctx, 1.05, vec![b(false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
    assert!(app.ui.flyout.is_some(), "the right-click opens the flyout and the same click doesn't close it");
    let menu = ctx.memory(|m| m.area_rect(egui::Id::unique("tool-flyout"))).expect("the flyout is shown");
    assert_eq!(app.session.tool_id(), "selection", "a right-click only opens the flyout");
    // Its second row is Rounded Rectangle (30 pt rows).
    let row = egui::pos2(menu.left() + 60.0, menu.top() + 45.0);
    let p = |pressed| Event::PointerButton { pos: row, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 2.0, vec![Event::PointerMoved(row), p(true)]);
    frame(&mut app, &ctx, 2.05, vec![p(false)]);
    assert_eq!((app.session.tool_id(), app.ui.flyout), ("roundedRectangle", None));
    // A long press opens it too, and releasing the press (a click, being short of 0.8 s) leaves
    // it open without choosing the button's tool.
    let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 3.0, vec![Event::PointerMoved(at), p(true)]);
    frame(&mut app, &ctx, 3.5, vec![]);
    assert!(app.ui.flyout.is_some(), "a long press opens the flyout");
    frame(&mut app, &ctx, 3.6, vec![p(false)]);
    frame(&mut app, &ctx, 3.7, vec![]);
    assert!(app.ui.flyout.is_some(), "releasing the long press leaves the flyout open");
    // A click anywhere else closes it.
    let away = egui::pos2(300.0, 1000.0);
    let c = |pressed| Event::PointerButton { pos: away, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 4.0, vec![Event::PointerMoved(away), c(true)]);
    frame(&mut app, &ctx, 4.05, vec![c(false)]);
    assert_eq!((app.session.tool_id(), app.ui.flyout), ("roundedRectangle", None));
}

/// The Shapes group's button (Selection, Direct Selection, Lasso, then Rectangle).
fn shapes_button(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::Rect {
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    frame(app, ctx, 0.0, vec![])[3]
}

#[test]
fn a_held_press_asks_for_the_frame_that_opens_the_flyout() {
    // egui draws only on input: a still press must schedule the frame where it becomes long.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let at = shapes_button(&mut app, &ctx).center();
    let p = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), p]);
    // The frames egui runs after the input settle; then nothing moves.
    frame(&mut app, &ctx, 1.02, vec![]);
    let input =
        egui::RawInput { time: Some(1.04), screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1200.0))), ..Default::default() };
    let out = ctx.run_ui(input, |ui| show(&mut app, ui));
    let delay = out.viewport_output.get(&egui::ViewportId::ROOT).unwrap().repaint_delay;
    assert!(delay <= std::time::Duration::from_secs_f64(LONG_PRESS), "repaint in {delay:?}");
    assert!(app.ui.flyout.is_none(), "not open before the press is long");
}

#[test]
fn pressing_the_corner_triangle_opens_the_flyout_at_once() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let button = shapes_button(&mut app, &ctx);
    let at = button.right_bottom() - vec2(4.0, 4.0);
    let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), p(true)]);
    assert!(app.ui.flyout.is_some(), "the press on the triangle opens it");
    frame(&mut app, &ctx, 1.05, vec![p(false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
    assert_eq!((app.session.tool_id(), app.ui.flyout.is_some()), ("selection", true), "the release neither closes it nor picks the tool");
    // A plain click in the middle of the button still just picks its tool.
    let mid = button.center();
    let c = |pressed| Event::PointerButton { pos: mid, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 2.0, vec![Event::PointerMoved(mid), c(true)]);
    frame(&mut app, &ctx, 2.05, vec![c(false)]);
    assert_eq!((app.session.tool_id(), app.ui.flyout), ("rectangle", None));
}

#[test]
fn the_flyout_opens_on_the_right_press_even_when_its_tool_is_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let at = shapes_button(&mut app, &ctx).center();
    app.select_tool("rectangle");
    for (k, t0) in [1.0, 3.0, 5.0].into_iter().enumerate() {
        let r = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, t0, vec![Event::PointerMoved(at), r(true)]);
        assert!(app.ui.flyout.is_some(), "right press #{k} opens it before the release");
        frame(&mut app, &ctx, t0 + 0.05, vec![r(false)]);
        assert!(app.ui.flyout.is_some(), "right press #{k} leaves it open");
        // A click away closes it; then try again.
        let away = egui::pos2(300.0, 1000.0);
        let c = |pressed| Event::PointerButton { pos: away, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, t0 + 1.0, vec![Event::PointerMoved(away), c(true)]);
        frame(&mut app, &ctx, t0 + 1.05, vec![c(false)]);
        assert!(app.ui.flyout.is_none());
    }
    // A long press on the active tool's button opens it too.
    let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 8.0, vec![Event::PointerMoved(at), p(true)]);
    frame(&mut app, &ctx, 8.0 + LONG_PRESS + 0.01, vec![]);
    assert!(app.ui.flyout.is_some(), "a long press on the active tool opens it");
    assert_eq!(app.session.tool_id(), "rectangle");
}

#[test]
fn type_button_long_press_selects_vertical_type_in_both_layouts() {
    for advanced in [false, true] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.toolbar_advanced = advanced;
        let all = slots(&app);
        let index = all.iter().position(|(_, tools)| tools.contains(&"type")).unwrap();
        assert!(all[index].1.contains(&"verticalType"));
        for id in ["type", "verticalType", "areaType", "verticalAreaType", "typeOnPath", "verticalTypeOnPath"] {
            assert_eq!(all.iter().filter(|(_, tools)| tools.contains(&id)).count(), 1);
        }
        let ctx = egui::Context::default();
        let at = frame_in(&mut app, &ctx, 0.0, vec![], 2000.0)[index].center();
        let pointer = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame_in(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), pointer(at, true)], 2000.0);
        frame_in(&mut app, &ctx, 1.36, vec![], 2000.0);
        frame_in(&mut app, &ctx, 1.4, vec![pointer(at, false)], 2000.0);
        frame_in(&mut app, &ctx, 1.45, vec![], 2000.0);
        assert_eq!(app.session.tool_id(), "selection");
        let tools: Vec<String> = ctx.data(|d| d.get_temp(egui::Id::unique("flyout-tools"))).unwrap();
        let row_index = tools.iter().position(|id| id == "verticalType").unwrap();
        let menu = ctx.memory(|m| m.area_rect(egui::Id::unique("tool-flyout"))).unwrap();
        let row = egui::pos2(menu.left() + 60.0, menu.top() + 15.0 + row_index as f32 * 30.0);
        frame_in(&mut app, &ctx, 2.0, vec![Event::PointerMoved(row), pointer(row, true)], 2000.0);
        frame_in(&mut app, &ctx, 2.05, vec![pointer(row, false)], 2000.0);
        assert_eq!(app.session.tool_id(), "verticalType");
        assert_eq!(app.ui.flyout, None);
    }
}

/// A press at `from`, dragged through `to` (one frame each), then released.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, from: Pos2, to: &[Pos2]) {
    let b = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, time, vec![Event::PointerMoved(from), b(from, true)]);
    for (k, p) in to.iter().enumerate() {
        frame(app, ctx, time + 0.05 * (k + 1) as f64, vec![Event::PointerMoved(*p)]);
    }
    let end = to.last().copied().unwrap_or(from);
    frame(app, ctx, time + 0.05 * (to.len() + 1) as f64, vec![b(end, false)]);
    frame(app, ctx, time + 0.05 * (to.len() + 2) as f64, vec![]);
}

/// A click at `at`.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2, button: PointerButton) {
    click_in(app, ctx, time, at, button, 1200.0);
}

/// [`click`] in a window `height` points tall; returns the tool buttons' rects after it.
fn click_in(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, at: Pos2, button: PointerButton, height: f32) -> Vec<egui::Rect> {
    let b = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() };
    frame_in(app, ctx, time, vec![Event::PointerMoved(at), b(true)], height);
    frame_in(app, ctx, time + 0.05, vec![b(false)], height);
    frame_in(app, ctx, time + 0.1, vec![], height)
}

/// The Color, Gradient and None buttons grow with the toolbar, and in two columns the three
/// drawing modes sit side by side, each setting its mode (#1129).
#[test]
fn the_paint_and_drawing_mode_buttons_fit_the_toolbar_width() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    frame_in(&mut app, &ctx, 0.0, vec![], 900.0);
    assert_eq!(widget_rects(&ctx, vec2(11.0, 11.0)).len(), 3, "one column: three 11 pt chips");
    app.ui.toolbar_double = true;
    frame_in(&mut app, &ctx, 1.0, vec![], 900.0);
    assert_eq!(widget_rects(&ctx, vec2(17.0, 17.0)).len(), 3, "two columns: three 17 pt chips");
    let modes = widget_rects(&ctx, vec2(22.0, 22.0));
    assert_eq!(modes.len(), 3, "the three drawing modes side by side");
    assert!(modes.iter().all(|r| (r.top() - modes[0].top()).abs() < 0.5), "{modes:?}");
    click_in(&mut app, &ctx, 2.0, modes[1].center(), PointerButton::Primary, 900.0);
    assert_eq!(app.session.draw_mode, vectorcraft_engine::DrawMode::Behind);
}

/// A click on the Gradient button applies the last gradient; Cmd/Ctrl-click resets the active
/// proxy's gradient to the linear White, Black gradient instead, as one undo step.
#[test]
fn cmd_click_on_the_gradient_button_resets_to_the_linear_white_black_gradient() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"kind": "radial", "stops": stops}})).unwrap();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    let fill = |app: &VectorcraftApp| match crate::panels::current_paints(app).0 {
        Paint::Gradient(g) => (g.gradient.kind, g.gradient.stops.iter().map(|s| s.color.to_hex()).collect::<Vec<_>>()),
        p => panic!("expected a gradient fill, got {p:?}"),
    };
    let ctx = egui::Context::default();
    frame_in(&mut app, &ctx, 0.0, vec![], 900.0);
    let gradient = widget_rects(&ctx, vec2(11.0, 11.0))[1].center();
    click_in(&mut app, &ctx, 1.0, gradient, PointerButton::Primary, 900.0);
    assert_eq!(fill(&app), (vectorcraft_color::GradientKind::Radial, vec!["#ff0000".into(), "#0000ff".into()]), "the last gradient");
    let m = egui::Modifiers::COMMAND;
    let b = |pressed| Event::PointerButton { pos: gradient, button: PointerButton::Primary, pressed, modifiers: m };
    frame_in(&mut app, &ctx, 2.0, vec![Event::ModifiersChanged(m), Event::PointerMoved(gradient), b(true)], 900.0);
    frame_in(&mut app, &ctx, 2.05, vec![b(false)], 900.0);
    frame_in(&mut app, &ctx, 2.1, vec![Event::ModifiersChanged(egui::Modifiers::NONE)], 900.0);
    assert_eq!(fill(&app), (vectorcraft_color::GradientKind::Linear, vec!["#ffffff".into(), "#000000".into()]));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(fill(&app).0, vectorcraft_color::GradientKind::Radial, "one undo step");
}

/// How many columns of tool buttons `rects` make.
fn columns(rects: &[egui::Rect]) -> usize {
    let mut lefts: Vec<i32> = rects.iter().map(|r| r.left().round() as i32).collect();
    lefts.sort_unstable();
    lefts.dedup();
    lefts.len()
}

/// Rows of tool buttons by their tops: how many buttons each row holds, top to bottom.
fn row_sizes(rects: &[egui::Rect]) -> Vec<usize> {
    let mut rows: Vec<(i32, usize)> = vec![];
    for r in rects {
        let top = r.top().round() as i32;
        match rows.last_mut() {
            Some((t, n)) if *t == top => *n += 1,
            _ => rows.push((top, 1)),
        }
    }
    rows.into_iter().map(|(_, n)| n).collect()
}

/// #1736: without group labels (as it ships), the tools run on from group to group: one column
/// evenly spaced, with nothing between the groups; two columns with every row full but maybe
/// the last. With the labels, each group starts its own row under its name.
#[test]
fn the_tools_run_on_without_group_labels() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let one = frame_in(&mut app, &ctx, 0.0, vec![], 1600.0);
    assert_eq!(one.len(), slots(&app).len());
    let steps: Vec<f32> = one.windows(2).map(|w| w[1].top() - w[0].top()).collect();
    assert!(steps.iter().all(|d| (d - steps[0]).abs() < 0.5), "evenly spaced: {steps:?}");
    app.ui.toolbar_double = true;
    let rows = row_sizes(&frame_in(&mut app, &ctx, 0.5, vec![], 1600.0));
    let slots = slots(&app).len();
    assert_eq!(rows.iter().sum::<usize>(), slots);
    assert!(rows[..rows.len() - 1].iter().all(|n| *n == 2), "{rows:?}");
    assert_eq!(rows.len(), slots.div_ceil(2));
    app.run("prefs.set", json!({"key": "toolGroupLabels", "value": true})).unwrap();
    let labelled = row_sizes(&frame_in(&mut app, &ctx, 1.0, vec![], 1600.0));
    assert!(labelled.len() > rows.len() && labelled.contains(&1), "{labelled:?}");
}

#[test]
fn the_double_arrow_toggles_one_and_two_columns_in_a_short_window() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    // 700 pt: too short for the single column, which scrolls instead of turning into two.
    assert_eq!(columns(&frame_in(&mut app, &ctx, 0.0, vec![], 700.0)), 1);
    let arrow = pos2(8.0, 7.0);
    assert_eq!(columns(&click_in(&mut app, &ctx, 1.0, arrow, PointerButton::Primary, 700.0)), 2);
    assert!(app.ui.toolbar_double && crate::menus::checked(&app, "window.toolbarColumns", &json!({})) == Some(true));
    assert_eq!(columns(&click_in(&mut app, &ctx, 2.0, arrow, PointerButton::Primary, 700.0)), 1);
    assert!(!app.ui.toolbar_double);
}

#[test]
fn the_columns_command_toggles_or_sets_the_toolbar_columns() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    assert_eq!(app.run("window.toolbarColumns", json!({})).unwrap(), json!(true));
    assert_eq!(columns(&frame_in(&mut app, &ctx, 0.0, vec![], 700.0)), 2);
    assert_eq!(app.run("window.toolbarColumns", json!({"double": true})).unwrap(), json!(true));
    assert_eq!(app.run("window.toolbarColumns", json!({})).unwrap(), json!(false));
    assert_eq!(crate::menus::checked(&app, "window.toolbarColumns", &json!({})), Some(false));
    assert_eq!(columns(&frame_in(&mut app, &ctx, 1.0, vec![], 700.0)), 1);
    assert!(app.run("window.toolbarColumns", json!({"double": "yes"})).is_err());
    // Advanced (every tool group) is taller still and keeps the column the user chose too.
    app.run("window.toolbarAdvanced", json!({})).unwrap();
    assert_eq!(columns(&frame_in(&mut app, &ctx, 2.0, vec![], 700.0)), 1);
}

#[test]
fn a_flyout_dragged_by_its_bar_floats_until_its_close_box_puts_it_back() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let at = shapes_button(&mut app, &ctx).center();
    click(&mut app, &ctx, 1.0, at, PointerButton::Secondary);
    let menu = ctx.memory(|m| m.area_rect(egui::Id::unique("tool-flyout"))).expect("the flyout is shown");
    // Its grab bar runs down its right side: dragging it tears the flyout off.
    let bar = pos2(menu.right() - BAR / 2.0, menu.center().y);
    drag(&mut app, &ctx, 2.0, bar, &[bar + vec2(20.0, 0.0), bar + vec2(60.0, 40.0)]);
    assert_eq!(app.ui.flyout, None, "the flyout became the floating one");
    let [f] = app.ui.floating_flyouts.as_slice() else { panic!("one floating flyout: {:?}", app.ui.floating_flyouts) };
    assert_eq!(f.tools.first().map(String::as_str), Some("rectangle"));
    let float = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).expect("the floating flyout is shown");
    // A strip of its nine tools' buttons, its bar under the pointer.
    let drop = bar + vec2(60.0, 40.0);
    assert!(float.height() < 40.0 && float.width() > 9.0 * STRIP_BUTTON.x, "a one-row strip: {float:?}");
    assert!((float.right() - 1.0 - BAR / 2.0 - drop.x).abs() < 1.0 && float.y_range().contains(drop.y), "its bar is under the pointer: {float:?}");
    // The same bar moves it.
    let bar = pos2(float.right() - 1.0 - BAR / 2.0, float.bottom() - 6.0);
    drag(&mut app, &ctx, 3.0, bar, &[bar + vec2(0.0, 20.0), bar + vec2(-10.0, 100.0)]);
    let moved = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).unwrap();
    assert!((moved.min - (float.min + vec2(-10.0, 100.0))).length() < 2.0, "moved to {moved:?} from {float:?}");
    // Picking a tool in it leaves it open.
    click(&mut app, &ctx, 4.0, pos2(moved.left() + STRIP_BUTTON.x * 1.5, moved.center().y), PointerButton::Primary);
    assert_eq!((app.session.tool_id(), app.ui.floating_flyouts.len()), ("roundedRectangle", 1));
    // While it floats, the presses that open the group's flyout raise it instead.
    click(&mut app, &ctx, 5.0, at, PointerButton::Secondary);
    assert_eq!(app.ui.flyout, None, "no flyout beside the floating one");
    assert!(ctx.data(|d| d.get_temp::<f64>(floating_area("rectangle"))).is_some(), "the floating flyout was raised");
    let p = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 6.0, vec![Event::PointerMoved(at), p(true)]);
    frame(&mut app, &ctx, 6.0 + LONG_PRESS + 0.01, vec![]);
    frame(&mut app, &ctx, 6.5, vec![p(false)]);
    assert_eq!((app.ui.flyout, app.session.tool_id()), (None, "roundedRectangle"), "a long press neither opens the flyout nor picks the tool");
    // The × at the top of the bar puts it back: the group opens as a flyout again.
    click(&mut app, &ctx, 7.0, pos2(moved.right() - BAR / 2.0, moved.top() + CLOSE / 2.0), PointerButton::Primary);
    assert!(app.ui.floating_flyouts.is_empty());
    click(&mut app, &ctx, 8.0, at, PointerButton::Secondary);
    assert!(app.ui.flyout.is_some());
}

#[test]
fn a_click_on_a_flyouts_bar_tears_it_off() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let at = shapes_button(&mut app, &ctx).center();
    click(&mut app, &ctx, 1.0, at, PointerButton::Secondary);
    let menu = ctx.memory(|m| m.area_rect(egui::Id::unique("tool-flyout"))).unwrap();
    let bar = pos2(menu.right() - BAR / 2.0, menu.center().y);
    click(&mut app, &ctx, 2.0, bar, PointerButton::Primary);
    frame(&mut app, &ctx, 2.2, vec![]);
    assert_eq!(app.ui.flyout, None);
    assert_eq!(app.ui.floating_flyouts.iter().map(|f| f.tools[0].as_str()).collect::<Vec<_>>(), ["rectangle"]);
    let float = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).expect("the floating flyout is shown");
    assert!(float.contains(bar), "its bar is under the pointer: {float:?}");
    assert_eq!(app.session.tool_id(), "selection", "no tool picked");
}

#[test]
fn releasing_a_long_press_over_the_flyouts_bar_tears_it_off() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    let at = shapes_button(&mut app, &ctx).center();
    let b = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(at), b(at, true)]);
    frame(&mut app, &ctx, 1.0 + LONG_PRESS + 0.01, vec![]);
    frame(&mut app, &ctx, 1.45, vec![]);
    let menu = ctx.memory(|m| m.area_rect(egui::Id::unique("tool-flyout"))).expect("the long press opens the flyout");
    let bar = pos2(menu.right() - BAR / 2.0, menu.center().y);
    frame(&mut app, &ctx, 1.5, vec![Event::PointerMoved(bar)]);
    frame(&mut app, &ctx, 1.55, vec![b(bar, false)]);
    frame(&mut app, &ctx, 1.6, vec![]);
    assert_eq!((app.ui.flyout, app.ui.floating_flyouts.len(), app.session.tool_id()), (None, 1, "selection"));
}

#[test]
fn window_float_tools_floats_and_puts_back_a_group() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let check = |app: &VectorcraftApp| crate::menus::checked(app, "window.floatTools", &json!({"tool": "star"}));
    assert_eq!(app.run("window.floatTools", json!({"tool": "star"})).unwrap(), json!(true));
    let [f] = app.ui.floating_flyouts.as_slice() else { panic!("one strip: {:?}", app.ui.floating_flyouts) };
    assert_eq!(f.tools.first().map(String::as_str), Some("rectangle"), "the Basic toolbar's Shapes group");
    assert_eq!(check(&app), Some(true));
    // Already floating: stays where it is.
    app.ui.floating_flyouts[0].pos = [300.0, 400.0];
    assert_eq!(app.run("window.floatTools", json!({"tool": "rectangle", "floating": true})).unwrap(), json!(true));
    assert_eq!(app.ui.floating_flyouts.iter().map(|f| f.pos).collect::<Vec<_>>(), [[300.0, 400.0]]);
    // Omitted toggles it back.
    assert_eq!(app.run("window.floatTools", json!({"tool": "star"})).unwrap(), json!(false));
    assert!(app.ui.floating_flyouts.is_empty() && check(&app) == Some(false));
    assert_eq!(app.run("window.floatTools", json!({"tool": "pen", "floating": false})).unwrap(), json!(false));
    // A tool alone in its slot, an unknown tool, a bad flag.
    assert!(app.run("window.floatTools", json!({"tool": "ellipse"})).is_err());
    assert!(app.run("window.floatTools", json!({"tool": "nope"})).is_err());
    assert!(app.run("window.floatTools", json!({})).is_err());
    assert!(app.run("window.floatTools", json!({"tool": "pen", "floating": "yes"})).is_err());
    assert!(app.ui.floating_flyouts.is_empty());
}

#[test]
fn workspaces_keep_floating_flyouts_and_reset_puts_them_back() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("window.floatTools", json!({"tool": "pen"})).unwrap();
    app.run("window.workspace.new", json!({"name": "Strips"})).unwrap();
    app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
    assert!(app.ui.floating_flyouts.is_empty(), "the built-in workspaces float none");
    app.run("window.workspace", json!({"name": "Strips"})).unwrap();
    assert_eq!(app.ui.floating_flyouts.iter().map(|f| f.tools[0].as_str()).collect::<Vec<_>>(), ["pen"]);
    // Workspaces saved before floating strips load without any.
    let mut old = serde_json::to_value(crate::workspaces::Workspace::default()).unwrap();
    old.as_object_mut().unwrap().remove("floatingFlyouts");
    assert!(serde_json::from_value::<crate::workspaces::Workspace>(old).unwrap().floating_flyouts.is_empty());
}

#[test]
fn loaded_floating_flyouts_keep_known_tools_one_strip_per_group() {
    let f = |tools: &[&str]| crate::state::FloatingFlyout { tools: tools.iter().map(|s| s.to_string()).collect(), pos: [0.0, 0.0] };
    let ui = crate::state::UiState {
        floating_flyouts: vec![f(&["pen", "nope", "addAnchor"]), f(&["pen", "addAnchor"]), f(&["nope"]), f(&[])],
        ..Default::default()
    };
    let ui = ui.sanitized();
    assert_eq!(ui.floating_flyouts.iter().map(|f| f.tools.join(",")).collect::<Vec<_>>(), ["pen,addAnchor"]);
}

#[test]
fn floating_flyouts_survive_saved_preferences_and_stay_on_screen() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools: vec!["pen".into(), "addAnchor".into()], pos: [5000.0, 10.0] });
    let saved = serde_json::to_value(&app.ui).unwrap();
    let back: crate::state::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.floating_flyouts.len(), 1);
    // Preferences saved before floating flyouts load without any.
    let mut old = saved;
    old.as_object_mut().unwrap().remove("floating_flyouts");
    assert!(serde_json::from_value::<crate::state::UiState>(old).unwrap().floating_flyouts.is_empty());
    let ctx = egui::Context::default();
    shapes_button(&mut app, &ctx);
    frame(&mut app, &ctx, 1.0, vec![]);
    let r = ctx.memory(|m| m.area_rect(floating_area("pen"))).unwrap();
    assert!(r.right() <= 400.5 && r.left() >= 0.0 && r.top() >= 0.0, "on screen: {r:?}");
    // A position that isn't a number puts it near the toolbar.
    app.ui.floating_flyouts[0].pos = [f32::NAN, f32::INFINITY];
    frame(&mut app, &ctx, 2.0, vec![]);
    let r = ctx.memory(|m| m.area_rect(floating_area("pen"))).unwrap();
    assert!(r.min.x.is_finite() && r.min.y.is_finite() && r.left() >= 0.0, "on screen: {r:?}");
}

/// User Interface › Show Tool Group Labels (#663): off as it ships (#1736), the toolbar shows
/// none of its groups' names and nothing between the groups; on, it names them.
#[test]
fn tool_group_labels_can_be_hidden() {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let draw = |app: &mut VectorcraftApp| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let mut shown = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        shown
    };
    let without = draw(&mut app);
    assert!(!without.iter().any(|t| ["Select", "Shapes", "Draw", "Modify", "Type"].contains(&t.as_str())), "{without:?}");
    app.run("prefs.set", json!({"key": "toolGroupLabels", "value": true})).unwrap();
    let with = draw(&mut app);
    assert!(with.iter().any(|t| t == "Shapes") && with.iter().any(|t| t == "Draw"), "{with:?}");
}

/// Hover `at` for two seconds, coming from elsewhere: the texts painted meanwhile.
fn hovered_texts(app: &mut VectorcraftApp, ctx: &egui::Context, t0: f64, at: Pos2) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut shown = vec![];
    for i in 0..20 {
        let events = match i {
            0 => vec![Event::PointerGone],
            1 => vec![Event::PointerMoved(at)],
            _ => vec![],
        };
        let time = t0 + f64::from(i) * 0.1;
        let raw = egui::RawInput {
            events,
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1200.0))),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::prefs_dialog::apply_runtime(app, ui.ctx());
            show(app, ui);
        });
        out.textures_delta.clear();
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
    }
    shown
}

#[test]
fn a_floating_flyouts_buttons_have_tool_tips_when_tool_tips_are_on() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let tools: Vec<String> = ["rectangle", "roundedRectangle", "star"].map(String::from).to_vec();
    app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [150.0, 600.0] });
    let ctx = egui::Context::default();
    shapes_button(&mut app, &ctx);
    frame(&mut app, &ctx, 1.0, vec![]);
    let strip = ctx.memory(|m| m.area_rect(floating_area("rectangle"))).unwrap();
    let star = pos2(strip.left() + STRIP_BUTTON.x * 2.5, strip.center().y);
    let tip = tl!("Star Tool").to_string();
    assert!(hovered_texts(&mut app, &ctx, 10.0, star).contains(&tip), "on by default");
    app.session.prefs.show_tool_tips = false;
    assert!(!hovered_texts(&mut app, &ctx, 20.0, star).contains(&tip), "off: no tool tip");
    app.session.prefs.show_tool_tips = true;
    assert!(hovered_texts(&mut app, &ctx, 30.0, star).contains(&tip), "on again");
}

/// Screen readers name a flyout's rows and a floating strip's buttons by their tools' tool tips,
/// with the active tool's pressed, and name the strip's close box and both tear-off bars.
#[test]
fn screen_readers_name_a_flyouts_rows_and_a_floating_strips_buttons() {
    let name = |id| tool_info(id).map(tip).unwrap_or_default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let group: Vec<String> = ["rectangle", "roundedRectangle", "ellipse"].map(String::from).to_vec();
    let flyout = tree(|ui| {
        app.ui.flyout = Some(0);
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("flyout-tools"), group.clone()));
        show(&mut app, ui);
    });
    assert_eq!(role_and_state(&flyout, &name("roundedRectangle")), (Role::Button, Some(Toggled::False)));
    assert_eq!(role_and_state(&flyout, &name("ellipse")), (Role::Button, Some(Toggled::False)));
    assert_eq!(role_and_state(&flyout, "Drag to float as a toolbar"), (Role::Button, None));
    app.ui.flyout = None;
    app.select_tool("star");
    let tools: Vec<String> = ["polygon", "star"].map(String::from).to_vec();
    app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [150.0, 600.0] });
    let strip = tree(|ui| show(&mut app, ui));
    assert_eq!(role_and_state(&strip, &name("polygon")), (Role::Button, Some(Toggled::False)));
    assert_eq!(role_and_state(&strip, &name("star")), (Role::Button, Some(Toggled::True)));
    assert_eq!(role_and_state(&strip, "Put back in the toolbar"), (Role::Button, None));
    assert_eq!(role_and_state(&strip, "Drag to move"), (Role::Button, None));
}
