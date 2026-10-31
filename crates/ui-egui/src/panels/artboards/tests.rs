use super::*;
use crate::widgets::accessibility_tests::{named, tree};
use vectorcraft_engine::Session;

/// Clicking a row makes its artboard the active one (the navigator's, Fit Artboard in Window's).
#[test]
fn clicking_a_row_makes_its_artboard_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 200, "height": 150})).unwrap();
    app.view_mut().unwrap().artboard = 0;
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    // Where the second row's name is painted.
    let at = frame(&mut app, vec![])
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Artboard 2" => Some(t.pos + vec2(4.0, 4.0)),
            _ => None,
        })
        .unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [egui::Event::PointerMoved(at), button(true), button(false)] {
        frame(&mut app, vec![e]);
    }
    assert_eq!(app.view().unwrap().artboard, 1, "Artboard 2 is active");
    // The navigator moves it on; the panel follows.
    crate::menus::invoke(&mut app, "view.goToArtboard", json!({"index": "previous"}));
    assert_eq!(selected(&app, 2), 0, "and the panel follows the navigator");
}

/// #1006: Select › All on Active Artboard selects the art of the artboard made active here,
/// and an explicit artboard still wins.
#[test]
fn all_on_active_artboard_takes_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
    let mut rect = |x| app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let (a, b) = (rect(10), rect(130));
    let picked = |app: &mut VectorcraftApp, params| {
        crate::menus::invoke(app, "select.allOnArtboard", params);
        let ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|n| n.0).collect();
        json!(ids)
    };
    select(&mut app, 1);
    assert_eq!(picked(&mut app, json!({})), json!([b]));
    select(&mut app, 0);
    assert_eq!(picked(&mut app, json!({})), json!([a]));
    assert_eq!(picked(&mut app, json!({"artboard": 1})), json!([b]));
}

/// Object › Artboards › Fit to Artwork Bounds and Fit to Selected Art resize the artboard made
/// active here, as the panel menu's Fit to Artwork Bounds does, and leave artboard 1 as it was;
/// an explicit index still wins.
#[test]
fn fit_to_artwork_bounds_and_selected_art_resize_the_active_artboard() {
    for id in ["artboard.fitToArt", "artboard.fitToSelection"] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 2})).unwrap();
        // Partly on artboard 1 and partly on artboard 2.
        app.session.execute("shape.rectangle", &json!({"x": 90, "y": 10, "width": 60, "height": 30})).unwrap();
        app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
        let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
        let (first, art) = (boards(&app)[0], vectorcraft_geom::Rect::new(90.0, 10.0, 150.0, 40.0));
        select(&mut app, 1);
        crate::menus::invoke(&mut app, id, json!({}));
        assert_eq!(boards(&app), [first, art], "{id} resizes Artboard 2, the active one");
        crate::menus::invoke(&mut app, id, json!({"index": 0}));
        assert_eq!(boards(&app), [art, art], "{id} with an index resizes that artboard");
    }
}

/// With artboards 1 and 3 selected with the Artboard tool, Object › Artboards › Fit to Artwork
/// Bounds fits each of them to the art on it and leaves artboard 2 as it was.
#[test]
fn fit_to_artwork_bounds_fits_each_selected_artboard_to_its_own_art() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    for (x, h) in [(10, 30), (130, 20), (250, 40)] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": h})).unwrap();
    }
    app.session.execute("select.all", &json!({})).unwrap();
    app.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let was = boards(&app);
    app.select_tool("artboard");
    select(&mut app, 0);
    app.session.set_tool_option("selected", &json!([0, 2]));
    crate::menus::invoke(&mut app, "artboard.fitToArt", json!({}));
    let rect = vectorcraft_geom::Rect::new;
    assert_eq!(boards(&app), [rect(10.0, 10.0, 30.0, 40.0), was[1], rect(250.0, 10.0, 270.0, 50.0)]);
}

/// With the Artboard tool, Shift-click on a row selects the artboards from the last row clicked
/// without Shift to it and makes it the active one, Ctrl/Cmd-click adds its artboard to the
/// tool's selection or takes it out (#1139), a plain click selects it alone, and every selected
/// row is highlighted.
#[test]
fn shift_clicking_rows_selects_several_artboards() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800, 1200] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    app.select_tool("artboard");
    select(&mut app, 1);
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let shapes = frame(&mut app, 0.0, vec![]);
    // `row(k)` returns the point where row `k`'s name is painted.
    let row = |k: usize| {
        shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == format!("Artboard {k}") => Some(t.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap()
    };
    let click = |k: usize, modifiers: egui::Modifiers| {
        let at = row(k);
        let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(at), b(true), b(false)]
    };
    let highlighted = |shapes: Vec<egui::epaint::ClippedShape>| {
        shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected)).count()
    };
    // With no row clicked yet, Shift-click ranges from the active artboard.
    frame(&mut app, 1.0, click(4, egui::Modifiers::SHIFT));
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![1, 2, 3], 3));
    frame(&mut app, 1.5, click(3, egui::Modifiers::SHIFT));
    assert_eq!(picked(&app, 4), [1, 2], "the next Shift-click ranges from the same artboard");
    frame(&mut app, 2.0, click(1, egui::Modifiers::NONE));
    assert_eq!(picked(&app, 4), [0]);
    frame(&mut app, 3.0, click(3, egui::Modifiers::SHIFT));
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![0, 1, 2], 2), "Artboards 1 to 3, Artboard 3 active");
    assert_eq!(highlighted(frame(&mut app, 4.0, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)])), 3, "three rows highlighted");
    // A second Shift-click ranges from the same row.
    frame(&mut app, 5.0, click(2, egui::Modifiers::SHIFT));
    assert_eq!(picked(&app, 4), [0, 1]);
    frame(&mut app, 6.0, click(3, egui::Modifiers::SHIFT));
    frame(&mut app, 7.0, click(2, egui::Modifiers::COMMAND));
    assert_eq!(picked(&app, 4), [0, 2], "Ctrl/Cmd-click takes Artboard 2 out");
    frame(&mut app, 8.0, click(2, egui::Modifiers::COMMAND));
    assert_eq!(picked(&app, 4), [0, 1, 2], "and puts it back");
    frame(&mut app, 9.0, click(4, egui::Modifiers::NONE));
    assert_eq!(picked(&app, 4), [3]);
    assert_eq!(highlighted(frame(&mut app, 10.0, vec![])), 1);
    // With another tool, a Shift-click makes the row's artboard active, as a plain click does.
    app.select_tool("selection");
    frame(&mut app, 11.0, click(2, egui::Modifiers::SHIFT));
    assert_eq!((picked(&app, 4), selected(&app, 4)), (vec![1], 1));
}

/// Double-clicking a row's number goes to its artboard and its name renames it; a click on
/// another row's options icon makes that artboard the active one, and the Artboard tool's
/// artboard is the active one.
#[test]
fn double_clicks_go_to_or_rename_and_options_edits_the_rows_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 200, "height": 150})).unwrap();
    app.view_mut().unwrap().artboard = 0;
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let shapes = frame(&mut app, 0.0, vec![]);
    let name = shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Artboard 2" => Some(t.pos + vec2(4.0, 4.0)),
            _ => None,
        })
        .unwrap();
    // The highlighted row (Artboard 1's), for its Options button at the right end.
    let row = shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected => Some(r.rect),
            _ => None,
        })
        .unwrap();
    let events = |at: egui::Pos2, clicks: usize| {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        std::iter::once(egui::Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect::<Vec<_>>()
    };
    // Artboard 2's number: it is active, no rename starts.
    frame(&mut app, 1.0, events(name - vec2(16.0, 0.0), 2));
    assert_eq!(app.view().unwrap().artboard, 1);
    assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), None);
    // Its name: a rename.
    frame(&mut app, 2.0, events(name, 2));
    assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), Some(1));
    set_pstate::<Option<usize>>(&ctx, "ab-edit", None);
    frame(&mut app, 3.0, vec![]);
    // A click on Artboard 1's options icon makes Artboard 1 active, and the Artboard tool,
    // chosen next, starts on it.
    frame(&mut app, 4.0, events(egui::pos2(row.right() - 14.0, row.center().y), 1));
    app.select_tool("artboard");
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["active"].clone()), (0, json!(0)));
    // While the Artboard tool is chosen, clicking a row moves the tool to that artboard too.
    frame(&mut app, 5.0, events(name, 1));
    assert_eq!(app.session.tool_options()["active"], json!(1));
    // And clicking an artboard with the tool makes it the active one, highlighted in the panel.
    let view = app.view_info();
    for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &vectorcraft_tools::PointerEvent::new(kind, 100.0, 100.0), view);
    }
    assert_eq!((app.session.tool_options()["active"].clone(), selected(&app, 2)), (json!(0), 0));
}

/// Shift+Enter in a row's name field renames the artboard and keeps the field open, the new
/// name selected; Enter then renames it again and closes the field.
#[test]
fn shift_enter_renames_an_artboard_and_keeps_the_field_open() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
    };
    let name = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards[1].name.clone();
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    // What a double-click on the second row's name does: its name field opens.
    set_pstate::<Option<usize>>(&ctx, "ab-edit", Some(1));
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    frame(&mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Cover".into())]);
    frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)]);
    assert_eq!(name(&app), "Cover", "Shift+Enter applies the name");
    let field = ctx.memory(|m| m.focused()).expect("the field keeps the focus");
    assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), Some(1), "and stays open");
    frame(&mut app, vec![]);
    let selected = egui::TextEdit::load_state(&ctx, field).and_then(|st| st.cursor.char_range()).map(|r| r.as_sorted_char_range());
    assert_eq!(selected.map(|r| (r.start.0, r.end.0)), Some((0, 5)), "the name shown is selected");
    frame(&mut app, vec![egui::Event::Text("Back".into())]);
    frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    frame(&mut app, vec![]);
    assert_eq!(name(&app), "Back", "typing replaced it, and Enter applies that");
    assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), None, "and closes the field");
}

/// With artboards 1, 3 and 4 selected with the Artboard tool, the panel menu's Rename opens an
/// empty name field on the active artboard's row, and the name typed there goes to all three,
/// numbered in panel order, in one undo step. With one artboard selected, Rename gives it the
/// name as typed.
#[test]
fn the_panel_menus_rename_numbers_several_selected_artboards() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800, 1200] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    app.select_tool("artboard");
    select(&mut app, 2);
    app.session.set_tool_option("selected", &json!([2, 0, 3]));
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let raw = |events| egui::RawInput { events, screen_rect: screen, ..Default::default() };
    let rename = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(raw(vec![]), |ui| menu(app, ui));
        out.textures_delta.clear();
        let item = out
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().trim_start() == "Rename" => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
            .unwrap();
        let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        ctx.run_ui(raw(vec![egui::Event::PointerMoved(item), button(true), button(false)]), |ui| menu(app, ui)).textures_delta.clear();
    };
    let frame = |app: &mut VectorcraftApp, events| ctx.run_ui(raw(events), |ui| show(app, ui)).textures_delta.clear();
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let names = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    rename(&mut app);
    assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), Some(2), "the field opens on the active artboard's row");
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Text("abc".into())]);
    frame(&mut app, vec![enter.clone()]);
    frame(&mut app, vec![]);
    assert_eq!(names(&app), ["abc-1", "Artboard 2", "abc-2", "abc-3"]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(names(&app), ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 4"], "one undo step");
    // One artboard selected: the field shows its name, and the name typed replaces it.
    select(&mut app, 1);
    rename(&mut app);
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let all = egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
    frame(&mut app, vec![all, egui::Event::Text("Solo".into())]);
    frame(&mut app, vec![enter]);
    frame(&mut app, vec![]);
    assert_eq!(names(&app), ["Artboard 1", "Solo", "Artboard 3", "Artboard 4"]);
}

/// Paste run on the session alone (`session.execute`, which leaves the view as it is) with the
/// Artboard tool selects the pasted artboard and leaves the active artboard where it was. The
/// panel menu's Rename then opens the name field on the selected artboard's row and renames
/// that artboard, not the active one.
#[test]
fn the_panel_menus_rename_opens_on_a_selected_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    app.session.execute("edit.copy", &json!({})).unwrap();
    app.session.execute("edit.paste", &json!({})).unwrap();
    assert_eq!((selected(&app, 3), picked(&app, 3)), (1, vec![2]), "Artboard 2 is active, and the pasted Artboard 3 is selected");
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let raw = |events| egui::RawInput { events, screen_rect: screen, ..Default::default() };
    let mut out = ctx.run_ui(raw(vec![]), |ui| menu(&mut app, ui));
    out.textures_delta.clear();
    let item = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().trim_start() == "Rename" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    ctx.run_ui(raw(vec![egui::Event::PointerMoved(item), button(true), button(false)]), |ui| menu(&mut app, ui)).textures_delta.clear();
    assert_eq!(pstate::<Option<usize>>(&ctx, "ab-edit"), Some(2), "the field opens on Artboard 3's row");
    let mut frame = |events| ctx.run_ui(raw(events), |ui| show(&mut app, ui)).textures_delta.clear();
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(vec![]);
    frame(vec![]);
    frame(vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text("Pasted".into())]);
    frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    frame(vec![]);
    let names: Vec<String> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 2", "Pasted"]);
}

/// With the Artboard tool, Paste and Paste in Place make the pasted artboard the active one
/// (the navigator's and the panel's), and Cut makes the artboard before the cut one active, the
/// one the tool then has active.
#[test]
fn paste_and_cut_with_the_artboard_tool_make_its_artboard_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let active = |app: &VectorcraftApp| (app.view().unwrap().artboard, app.session.tool_options()["active"].clone());
    app.run("edit.copy", json!({})).unwrap();
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(active(&app), (3, json!(3)), "Paste: the pasted Artboard 4");
    app.run("edit.pasteInPlace", json!({})).unwrap();
    assert_eq!(active(&app), (4, json!(4)), "Paste in Place: the pasted Artboard 5");
    select(&mut app, 2);
    app.run("edit.cut", json!({})).unwrap();
    assert_eq!(active(&app), (1, json!(1)), "Cut Artboard 3: Artboard 2");
}

/// `tool.setOption` with the Artboard tool's `selected` or `active` makes the artboard the tool
/// then has active the active one.
#[test]
fn the_artboard_tools_options_make_its_artboard_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    app.run("tool.setOption", json!({"key": "selected", "value": [2, 0]})).unwrap();
    assert_eq!((app.view().unwrap().artboard, picked(&app, 3)), (2, vec![0, 2]));
    app.run("tool.setOption", json!({"tool": "artboard", "values": {"active": 1}})).unwrap();
    assert_eq!((app.view().unwrap().artboard, picked(&app, 3)), (1, vec![1]));
}

/// With the Artboard tool, a `command.batch` that stays in the document makes the artboard the
/// tool then has active the active one, as its steps do on their own: a batch that leaves the
/// tool's artboards keeps the active artboard, and Copy and Paste make the pasted one active.
#[test]
fn a_batch_in_one_document_makes_the_artboard_tools_artboard_active() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let active = |app: &VectorcraftApp| (app.view().unwrap().artboard, app.session.tool_options()["active"].clone());
    app.run("command.batch", json!({"commands": [{"command": "object.unlockAll", "params": {}}]})).unwrap();
    assert_eq!(active(&app), (1, json!(1)));
    let steps = [json!({"command": "edit.copy", "params": {}}), json!({"command": "edit.paste", "params": {}})];
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(active(&app), (3, json!(3)), "the pasted Artboard 4");
}

/// With the Artboard tool, `document.activate` makes the active artboard of the document it
/// switches to the tool's, as choosing the document's tab does.
#[test]
fn switching_documents_with_the_artboard_tool_keeps_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 2);
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (2, json!([2])));
}

/// With the Artboard tool, a `command.batch` that switches to another document and back keeps
/// the active artboard, and the tool starts on it. When a later step of the batch pastes an
/// artboard, the pasted one becomes active, as Paste on its own makes it. A later Cut of the
/// tool's artboard 1 makes the new first artboard active, as Cut on its own makes it.
#[test]
fn a_batch_that_switches_documents_and_back_keeps_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 3})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 2);
    let step = |command: &str, params: Value| json!({"command": command, "params": params});
    let back = [step("document.activate", json!({"index": 1})), step("document.activate", json!({"index": 0}))];
    app.run("command.batch", json!({"commands": back})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (2, json!([2])));
    app.run("edit.copy", json!({})).unwrap();
    let mut steps = back.to_vec();
    steps.push(step("edit.paste", json!({})));
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (3, json!([3])), "the pasted Artboard 4");
    let mut steps = back.to_vec();
    steps.push(step("edit.cut", json!({})));
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.artboards.len(), 3, "Artboard 1 cut");
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (0, json!([0])), "the new first artboard");
}

/// With the Artboard tool, a double-click on a name with Shift or Ctrl/Cmd held that leaves
/// several artboards selected opens the name field for all of them. A Ctrl/Cmd double-click on
/// an unselected row's name adds its artboard to the selection, and the second click leaves it
/// selected. A plain double-click selects the row's artboard alone and opens the field for it.
#[test]
fn a_shift_or_ctrl_double_click_on_a_name_renames_every_selected_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800, 1200] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    app.select_tool("artboard");
    select(&mut app, 0);
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let shapes = frame(&mut app, 0.0, vec![]);
    // `row(k)` returns the point where row `k`'s name is painted.
    let row = |k: usize| {
        shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text() == format!("Artboard {k}") => Some(t.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap()
    };
    let click = |k: usize, modifiers: egui::Modifiers| {
        let at = row(k);
        let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(at), b(true), b(false)]
    };
    // Two clicks on row `k`'s name in two frames 0.1 s apart, then the row and the artboards
    // the name field opened for, and the field closed again.
    let double_click = |app: &mut VectorcraftApp, time: f64, k: usize, modifiers: egui::Modifiers| {
        frame(app, time, click(k, modifiers));
        frame(app, time + 0.1, click(k, modifiers));
        let opened = (pstate::<Option<usize>>(&ctx, "ab-edit"), pstate::<Vec<usize>>(&ctx, "ab-edit-all"));
        set_pstate::<Option<usize>>(&ctx, "ab-edit", None);
        set_pstate::<Vec<usize>>(&ctx, "ab-edit-all", vec![]);
        frame(app, time + 0.5, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        opened
    };
    frame(&mut app, 1.0, click(1, egui::Modifiers::NONE));
    assert_eq!(double_click(&mut app, 2.0, 3, egui::Modifiers::SHIFT), (Some(2), vec![0, 1, 2]), "Shift: Artboards 1 to 3");
    assert_eq!(double_click(&mut app, 4.0, 2, egui::Modifiers::COMMAND), (Some(1), vec![0, 1, 2]), "Ctrl/Cmd on a selected one");
    assert_eq!(double_click(&mut app, 5.0, 4, egui::Modifiers::COMMAND), (Some(3), vec![0, 1, 2, 3]), "Ctrl/Cmd on another one");
    assert_eq!(picked(&app, 4), [0, 1, 2, 3], "which stays selected");
    assert_eq!(double_click(&mut app, 6.0, 4, egui::Modifiers::NONE), (Some(3), vec![3]), "a plain double-click");
}

/// A row's options icon opens Artboard Options for that row's artboard and leaves the tool as
/// it is (#1876): a click on the active artboard's icon, or a double-click on another row's
/// icon, whose first click only makes that artboard active. With the Artboard tool in use, the
/// tool moves to that artboard too.
#[test]
fn the_options_icon_opens_artboard_options_for_its_rows_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
    app.view_mut().unwrap().artboard = 0;
    let tool = app.session.tool_id().to_string();
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    // The highlighted row (Artboard 1's). The rows are 24 points apart, and each icon's
    // center is 14 points in from the row's right end.
    let row = frame(&mut app, 0.0, vec![])
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected => Some(r.rect),
            _ => None,
        })
        .unwrap();
    let icon = |k: f32| egui::pos2(row.right() - 14.0, row.center().y + 24.0 * k);
    let events = |at: egui::Pos2, clicks: usize| {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        std::iter::once(egui::Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect::<Vec<_>>()
    };
    let opened = |app: &mut VectorcraftApp| app.ui.dialog.take().map(|d| (d.kind.clone(), d.f64("index", 9.0), d.f64("width", 0.0)));
    // Artboard 2's icon: the first click makes it active, the second (a double-click) opens its
    // options; no rename starts.
    frame(&mut app, 1.0, events(icon(1.0), 1));
    assert_eq!((app.view().unwrap().artboard, opened(&mut app)), (1, None));
    frame(&mut app, 1.1, events(icon(1.0), 1));
    assert_eq!(opened(&mut app), Some(("artboardOptions".to_string(), 1.0, 100.0)));
    assert_eq!((app.session.tool_id(), pstate::<Option<usize>>(&ctx, "ab-edit")), (tool.as_str(), None));
    // The active artboard's icon: one click opens its options.
    frame(&mut app, 3.0, events(icon(1.0), 1));
    assert_eq!(opened(&mut app), Some(("artboardOptions".to_string(), 1.0, 100.0)));
    // With the Artboard tool, a double-click on Artboard 1's icon (both clicks in one frame)
    // opens Artboard 1's options, and the tool moves to it.
    app.select_tool("artboard");
    frame(&mut app, 5.0, events(icon(0.0), 2));
    assert_eq!(opened(&mut app), Some(("artboardOptions".to_string(), 0.0, 200.0)));
    assert_eq!((app.session.tool_id(), app.session.tool_options()["active"].clone()), ("artboard", json!(0)));
}

/// The panel menu's Artboard Options… opens Artboard Options for the active artboard and leaves
/// the tool as it is.
#[test]
fn the_panel_menus_artboard_options_opens_the_active_artboards_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
    select(&mut app, 1);
    let tool = app.session.tool_id().to_string();
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| menu(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let item = frame(&mut app, vec![])
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().trim_start() == "Artboard Options…" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(item), button(true), button(false)]);
    let d = app.ui.dialog.take().unwrap();
    assert_eq!((d.kind.as_str(), d.f64("index", 9.0), d.f64("x", 0.0)), ("artboardOptions", 1.0, 400.0));
    assert_eq!(app.session.tool_id(), tool);
}

/// With two artboards selected with the Artboard tool, the panel menu's Duplicate Artboards
/// copies both, with their art, in one undo step, and the tool then selects the copies, the
/// active artboard's copy active.
#[test]
fn the_panel_menus_duplicate_artboards_copies_every_selected_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    for x in [10, 410] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
    }
    app.select_tool("artboard");
    select(&mut app, 1);
    app.session.set_tool_option("selected", &json!([1, 0]));
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| menu(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let item = frame(&mut app, vec![])
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().trim_start() == "Duplicate Artboards" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(item), button(true), button(false)]);
    let d = &app.session.active().unwrap().doc;
    let names: Vec<&str> = d.artboards.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 1 copy", "Artboard 2 copy"]);
    assert_eq!(d.layers[0].children().unwrap().len(), 4, "the squares and their copies");
    assert_eq!((app.view().unwrap().artboard, app.session.tool_options()["selected"].clone()), (4, json!([4, 3])), "Artboard 2's copy is active");
    app.run("edit.undo", json!({})).unwrap();
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (3, 2), "one undo step");
}

/// The panel menu's Convert to Artboards turns the geometric bounds of each selected rectangle
/// into a new artboard and removes the rectangles, in one undo step, as Object › Artboards ›
/// Convert to Artboards does; with nothing selected it is off.
#[test]
fn the_panel_menus_convert_to_artboards_converts_the_selected_objects() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [10, 60] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 30})).unwrap();
    }
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| menu(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let item = frame(&mut app, vec![])
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().trim_start() == "Convert to Artboards" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
        .unwrap();
    let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let click = |app: &mut VectorcraftApp| frame(app, vec![egui::Event::PointerMoved(item), button(true), button(false)]);
    let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let first = boards(&app)[0];
    app.session.execute("select.none", &json!({})).unwrap();
    frame(&mut app, vec![]);
    click(&mut app);
    // An enabled item would run the command, and its refusal would show in the status bar.
    assert_eq!((boards(&app), app.ui.status.as_str()), (vec![first], ""), "off with nothing selected");
    app.session.execute("select.all", &json!({})).unwrap();
    // Draw one frame with the item enabled, as the app redraws the panel after the selection changes.
    frame(&mut app, vec![]);
    click(&mut app);
    let rect = vectorcraft_geom::Rect::new;
    assert_eq!(boards(&app), [first, rect(10.0, 10.0, 30.0, 40.0), rect(60.0, 10.0, 80.0, 40.0)]);
    let d = app.session.active().unwrap();
    assert_eq!((d.doc.layers[0].children().unwrap().len(), d.selection.objects.len()), (0, 0), "the rectangles are removed");
    app.run("edit.undo", json!({})).unwrap();
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (1, 2), "one undo step");
}

/// One headless frame of the Control bar, 2,000 points wide, with `events` → the texts painted,
/// with their rects.
fn bar_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    bar_frame_in(app, ctx, events, 2000.0)
}

/// [`bar_frame`] in a window `width` points wide.
fn bar_frame_in(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, width: f32) -> Vec<(String, egui::Rect)> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 300.0));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| crate::chrome::control_bar(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
            _ => None,
        })
        .collect()
}

/// A context with the app's fonts, which the Control bar's label uses.
fn bar_context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// Two frames of the Control bar → the texts the second one painted.
fn bar_texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect)> {
    bar_frame(app, ctx, vec![]);
    bar_frame(app, ctx, vec![])
}

/// Where `label` is painted in `texts`.
fn text_rect(texts: &[(String, egui::Rect)], label: &str) -> egui::Rect {
    texts.iter().find(|(t, _)| t == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
}

/// A press and a release at `at` on the Control bar, a frame each.
fn bar_click(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    bar_frame(app, ctx, vec![egui::Event::PointerMoved(at), button(true)]);
    bar_frame(app, ctx, vec![button(false)]);
}

/// Click the Control bar's field at `at`, select all of it, type `typed` and press Enter.
fn bar_type(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2, typed: &str) {
    let key = |key, modifiers| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
    bar_click(app, ctx, at);
    bar_frame(app, ctx, key(egui::Key::A, egui::Modifiers::COMMAND));
    bar_frame(app, ctx, vec![egui::Event::Text(typed.into())]);
    bar_frame(app, ctx, key(egui::Key::Enter, egui::Modifiers::NONE));
    bar_frame(app, ctx, vec![]);
}

/// The Control bar's clickable widgets of the last frame that lie between x `from` and `to`,
/// left to right.
fn bar_buttons(ctx: &egui::Context, from: f32, to: f32) -> Vec<egui::Rect> {
    let mut all: Vec<egui::Rect> = ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && w.rect.left() >= from && w.rect.right() <= to)
            .map(|w| w.rect)
            .collect()
    });
    all.sort_by(|a, b| a.left().total_cmp(&b.left()));
    all
}

/// The artboards' rectangles, the undo steps and the artboards' names.
fn boards(app: &VectorcraftApp) -> (Vec<vectorcraft_geom::Rect>, usize, Vec<String>) {
    let st = app.session.active().unwrap();
    (st.doc.artboards.iter().map(|a| a.rect).collect(), st.history.undo.len(), st.doc.artboards.iter().map(|a| a.name.clone()).collect())
}

/// With the Artboard tool in use, the Control bar shows the active artboard's controls in place
/// of the object controls: its preset, name, the Transform link, Move and Scale Artwork with
/// Artboard, X, Y, W and H, left to right, and in a narrow window the link without the fields.
/// Another tool shows the object controls.
#[test]
fn the_control_bar_shows_the_artboard_tools_controls() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "units": "Points"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let ctx = bar_context();
    let has = |texts: &[(String, egui::Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    let texts = bar_texts(&mut app, &ctx);
    assert!(has(&texts, "Opacity:") && !has(&texts, "Name:") && !has(&texts, "Preset:"), "{texts:?}");
    app.select_tool("artboard");
    let texts = bar_texts(&mut app, &ctx);
    let order = [
        "Artboard",
        "Preset:",
        "Name:",
        "Artboard 1",
        "Transform",
        "Move Artwork with Artboard",
        "Scale Artwork with Artboard",
        "X:",
        "Y:",
        "W:",
        "200 pt",
        "H:",
    ];
    let lefts: Vec<f32> = order.iter().map(|s| text_rect(&texts, s).left()).collect();
    assert!(lefts.windows(2).all(|w| w[0] < w[1]), "{order:?} at {lefts:?}");
    for s in ["Opacity:", "Stroke:"] {
        assert!(!has(&texts, s), "{s}: {texts:?}");
    }
    // In a window that leaves 300 points right of Scale Artwork with Artboard, the inline fields
    // go (X: would start inside the window).
    let width = text_rect(&texts, "Scale Artwork with Artboard").right() + 300.0;
    bar_frame_in(&mut app, &ctx, vec![], width);
    let texts = bar_frame_in(&mut app, &ctx, vec![], width);
    assert!(has(&texts, "Scale Artwork with Artboard") && !has(&texts, "X:") && !has(&texts, "W:"), "{texts:?}");
    // The Snapping menu sits first on a wide bar and leaves the narrow bar to the artboard controls.
    assert!(has(&bar_texts(&mut app, &ctx), "Snapping"));
    // In the app's narrowest window, 800 points wide, the name and the Transform link still fit,
    // and a click on the link shows the artboard's X, Y, W and H in the Transform panel's popover.
    let narrow = |app: &mut VectorcraftApp, events| bar_frame_in(app, &ctx, events, 800.0);
    narrow(&mut app, vec![]);
    let texts = narrow(&mut app, vec![]);
    assert!(!has(&texts, "Snapping"), "{texts:?}");
    let link = text_rect(&texts, "Transform");
    assert!(link.right() < 790.0 && text_rect(&texts, "Artboard 1").right() < link.left(), "{texts:?}");
    // Move and Scale Artwork with Artboard show as their boxes alone, inside the window, and the
    // second one turns Scale Artwork with Artboard on.
    assert!(!has(&texts, "Move Artwork with Artboard"), "{texts:?}");
    let boxes = bar_buttons(&ctx, link.right(), 790.0);
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    let scale = |app: &VectorcraftApp| app.session.tool_options()["scaleArt"].as_bool();
    assert_eq!(scale(&app), Some(false));
    let press =
        |pressed| egui::Event::PointerButton { pos: boxes[1].center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    narrow(&mut app, vec![egui::Event::PointerMoved(boxes[1].center()), press(true)]);
    narrow(&mut app, vec![press(false)]);
    assert_eq!(scale(&app), Some(true));
    narrow(&mut app, vec![]);
    let button =
        |pressed| egui::Event::PointerButton { pos: link.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    narrow(&mut app, vec![egui::Event::PointerMoved(link.center()), button(true)]);
    narrow(&mut app, vec![button(false)]);
    let texts = narrow(&mut app, vec![]);
    assert!(has(&texts, "W:") && has(&texts, "200 pt"), "the popover: {texts:?}");
}

/// The Control bar's W resizes the active artboard about the reference point in one undo step:
/// about its center at first, and from its top-left corner once the locator's top-left point is
/// chosen.
#[test]
fn the_control_bar_resizes_the_active_artboard_about_the_reference_point() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "units": "Points", "artboards": 2})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let ctx = bar_context();
    let (before, undo, _) = boards(&app);
    let w = text_rect(&bar_texts(&mut app, &ctx), "W:");
    bar_type(&mut app, &ctx, egui::pos2(w.right() + 40.0, w.center().y), "300");
    let b = before[1];
    let mut want = before.clone();
    want[1] = vectorcraft_geom::Rect::new(b.x0 - 50.0, b.y0, b.x1 + 50.0, b.y1);
    assert_eq!(boards(&app).0, want, "Artboard 2 widens about its center");
    assert_eq!(boards(&app).1, undo + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app).0, before);
    // The locator's points are the 9-point-square buttons; the top-left one is drawn first.
    let texts = bar_texts(&mut app, &ctx);
    let points: Vec<egui::Rect> = bar_buttons(&ctx, 0.0, text_rect(&texts, "X:").left())
        .into_iter()
        .filter(|r| (r.width() - 9.0).abs() < 0.5 && (r.height() - 9.0).abs() < 0.5)
        .collect();
    let top_left = points.iter().min_by(|a, b| (a.top(), a.left()).partial_cmp(&(b.top(), b.left())).unwrap()).unwrap();
    bar_click(&mut app, &ctx, top_left.center());
    assert_eq!(ctx.data(|d| d.get_temp::<usize>(egui::Id::unique("refpt"))), Some(0), "the shared reference point is the top-left one");
    let w = text_rect(&bar_texts(&mut app, &ctx), "W:");
    bar_type(&mut app, &ctx, egui::pos2(w.right() + 40.0, w.center().y), "300");
    want[1] = vectorcraft_geom::Rect::new(b.x0, b.y0, b.x0 + 300.0, b.y1);
    assert_eq!(boards(&app).0, want, "Artboard 2 widens from its left edge");
}

/// The Control bar's Portrait and Landscape buttons swap the active artboard's width and height,
/// its top-left corner staying put, in one undo step each.
#[test]
fn the_control_bar_orientation_swaps_width_and_height() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "units": "Points"})).unwrap();
    app.select_tool("artboard");
    let ctx = bar_context();
    let (_, undo, _) = boards(&app);
    // Between "Preset:" and "Name:", the buttons that take a click: the preset menu, Portrait,
    // Landscape and New Artboard (Delete Artboard is disabled with one artboard).
    let buttons = |app: &mut VectorcraftApp| {
        let texts = bar_texts(app, &ctx);
        bar_buttons(&ctx, text_rect(&texts, "Preset:").right(), text_rect(&texts, "Name:").left())
    };
    let b = buttons(&mut app);
    bar_click(&mut app, &ctx, b[b.len() - 3].center());
    assert_eq!((boards(&app).0, boards(&app).1), (vec![vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 200.0)], undo + 1), "Portrait");
    let b = buttons(&mut app);
    bar_click(&mut app, &ctx, b[b.len() - 2].center());
    assert_eq!((boards(&app).0, boards(&app).1), (vec![vectorcraft_geom::Rect::new(0.0, 0.0, 200.0, 100.0)], undo + 2), "Landscape");
}

/// Typing a name into the Control bar's Name field and pressing Enter renames the active
/// artboard alone, in one undo step.
#[test]
fn the_control_bar_name_field_renames_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    app.select_tool("artboard");
    select(&mut app, 1);
    let ctx = bar_context();
    let (_, undo, _) = boards(&app);
    let name = text_rect(&bar_texts(&mut app, &ctx), "Artboard 2");
    bar_type(&mut app, &ctx, name.center(), "Cover");
    assert_eq!((boards(&app).2, boards(&app).1), (vec!["Artboard 1".to_string(), "Cover".to_string()], undo + 1));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app).2, ["Artboard 1", "Artboard 2"]);
}

/// The Control bar's New Artboard adds an artboard and makes it the active one, Artboard Options
/// opens the active artboard's options, Rearrange All Artboards opens its dialog, and Delete
/// Artboard deletes the selected artboard.
#[test]
fn the_control_bar_adds_deletes_and_opens_artboard_dialogs() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.select_tool("artboard");
    let ctx = bar_context();
    let texts = bar_texts(&mut app, &ctx);
    // With one artboard Delete Artboard is disabled, so New Artboard is the last button before
    // "Name:"; with two, Delete Artboard is.
    let (preset, name) = (text_rect(&texts, "Preset:").right(), text_rect(&texts, "Name:").left());
    let b = bar_buttons(&ctx, preset, name);
    bar_click(&mut app, &ctx, b[b.len() - 1].center());
    assert_eq!((boards(&app).2.len(), selected(&app, 2)), (2, 1), "New Artboard adds Artboard 2 and makes it active");
    // Artboard Options and Rearrange All Artboards: the two buttons between the name and the
    // Transform link.
    let texts = bar_texts(&mut app, &ctx);
    let between = |texts: &[(String, egui::Rect)]| bar_buttons(&ctx, text_rect(texts, "Artboard 2").right(), text_rect(texts, "Transform").left());
    let options = between(&texts)[0];
    bar_click(&mut app, &ctx, options.center());
    let d = app.ui.dialog.take().unwrap();
    assert_eq!((d.kind.as_str(), d.f64("index", 9.0)), ("artboardOptions", 1.0));
    let texts = bar_texts(&mut app, &ctx);
    let rearrange = between(&texts)[1];
    bar_click(&mut app, &ctx, rearrange.center());
    assert_eq!(app.ui.dialog.take().map(|d| d.kind), Some(crate::dialogs::rearrange_artboards::KIND.to_string()));
    bar_texts(&mut app, &ctx);
    let b = bar_buttons(&ctx, preset, name);
    bar_click(&mut app, &ctx, b[b.len() - 1].center());
    assert_eq!(boards(&app).2, ["Artboard 1"], "Delete Artboard deletes Artboard 2");
}

/// Choosing the Artboard tool starts it on the active artboard, the one the panel highlights:
/// Artboard Options shows that artboard, the pointer moving over another artboard leaves it
/// active, and Delete removes it. Shift+O while the tool is in use keeps the artboards it has
/// selected.
#[test]
fn choosing_the_artboard_tool_starts_it_on_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    select(&mut app, 1);
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    assert_eq!(app.session.tool_options()["active"], json!(1));
    app.run("tool.options", json!({"tool": "artboard"})).unwrap();
    assert_eq!(app.ui.dialog.take().unwrap().str("name"), "Artboard 2");
    let view = app.view_info();
    crate::canvas::dispatch(&mut app, &vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Move, 100.0, 75.0), view);
    assert_eq!(selected(&app, 3), 1, "the pointer over Artboard 1 leaves Artboard 2 active");
    let r = app.session.tool_key(vectorcraft_tools::ToolKey::Delete, Default::default(), view);
    crate::canvas::apply_requests(&mut app, r);
    let names: Vec<String> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 3"], "Delete removes Artboard 2");
    // Shift+O while the tool is in use keeps the artboards it has selected.
    app.session.set_tool_option("selected", &json!([0, 1]));
    let ctx = egui::Context::default();
    let shift_o = vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::Text("O".into())];
    let mut out = ctx.run_ui(egui::RawInput { events: shift_o, ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert_eq!((app.session.tool_id(), picked(&app, 2)), ("artboard", vec![0, 1]));
}

/// A row dragged onto New Artboard duplicates its artboard with its art (#446), outlining the
/// button while it is held over it, and the copy becomes the active artboard.
#[test]
fn dragging_a_row_onto_new_artboard_duplicates_it() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    let shapes = frame(&mut app, vec![]);
    let t = Tokens::get(&ctx);
    let rect_of = |fill| {
        shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
            _ => None,
        })
    };
    let row = rect_of(t.row_selected).unwrap().center();
    // The bottom bar's buttons sit under its divider, New Artboard second from the right.
    let bar = rect_of(t.divider).unwrap();
    let new = pos2(bar.right() - 24.0 - 4.0 - 12.0, bar.bottom() + 2.0 + 12.0);
    let button = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(row), button(row, true)]);
    frame(&mut app, vec![egui::Event::PointerMoved(row + vec2(0.0, 12.0))]);
    let held = frame(&mut app, vec![egui::Event::PointerMoved(new)]);
    assert!(held.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.stroke.color == t.accent)), "New Artboard is outlined");
    frame(&mut app, vec![button(new, false)]);
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (2, 2), "the artboard and its art");
    assert_eq!(app.view().unwrap().artboard, 1);
}

/// A click in a row's lock column locks its artboard and shows the lock next to the name, a
/// second click unlocks it, and each click is one undo step. Delete Artboard leaves a locked
/// artboard.
#[test]
fn the_lock_column_locks_and_unlocks_an_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 0, "width": 200, "height": 150})).unwrap();
    select(&mut app, 1);
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    // The highlighted row (Artboard 2's): its lock column is 36 points in from its right end.
    let shapes = frame(&mut app, 0.0, vec![]);
    let row = shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).row_selected => Some(r.rect),
            _ => None,
        })
        .unwrap();
    let lock = egui::pos2(row.right() - 36.0, row.center().y);
    let click = |at: egui::Pos2| {
        let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        vec![egui::Event::PointerMoved(at), b(true), b(false)]
    };
    let away = vec![egui::Event::PointerMoved(egui::pos2(-50.0, -50.0))];
    let locked = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
    // Whether something is drawn in the lock column of Artboard 2's row.
    let cell = egui::Rect::from_center_size(lock, vec2(14.0, 14.0));
    let lock_drawn = |shapes: Vec<egui::epaint::ClippedShape>| {
        shapes.iter().any(|c| {
            let b = c.shape.visual_bounding_rect();
            b.is_positive() && cell.contains_rect(b)
        })
    };
    assert!(!lock_drawn(frame(&mut app, 1.0, away.clone())));
    frame(&mut app, 2.0, click(lock));
    assert_eq!(locked(&app), [false, true]);
    assert!(lock_drawn(frame(&mut app, 3.0, away.clone())), "the lock shows");
    assert!(deletable(&app, 2).is_empty(), "Delete Artboard leaves it");
    delete_selected(&mut app);
    assert_eq!(app.session.active().unwrap().doc.artboards.len(), 2);
    frame(&mut app, 5.0, click(lock));
    assert_eq!(locked(&app), [false, false]);
    assert!(!lock_drawn(frame(&mut app, 6.0, away)));
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(locked(&app), [false, true], "one undo step");
}

/// A new size for the selected artboards leaves a locked one among them as it is. The
/// artboard the field belongs to stays listed when it is locked, so `artboard.setProps`
/// refuses the change. A name typed for them goes to the locked one too.
#[test]
fn a_new_size_for_the_selected_artboards_leaves_a_locked_one() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.session.execute("artboard.new", &json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    app.select_tool("artboard");
    app.session.set_tool_option("selected", &json!([0, 1, 2]));
    app.session.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    assert_eq!(resized_with(&app, 0), [0, 2]);
    assert_eq!(resized_with(&app, 1), [0, 1, 2], "the field's own artboard stays listed");
    crate::panels::transform::set_artboard(&mut app, 0, 0, [None, None, Some(300.0), None], false);
    let widths = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.width()).collect::<Vec<_>>();
    assert_eq!(widths, [300.0, 200.0, 300.0]);
    let all = renamed_with(&app, 0);
    assert_eq!(all, [0, 1, 2]);
    rename(&mut app, &all, "abc").unwrap();
    let names = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    assert_eq!(names, ["abc-1", "abc-2", "abc-3"]);
}

/// Alt-click on New Artboard opens Artboard Options for the new artboard and adds nothing yet:
/// the fields are those a plain click's artboard gets. Cancel adds nothing; OK adds the artboard
/// with the fields in one undo step and makes it the active one.
#[test]
fn alt_clicking_new_artboard_opens_artboard_options_for_the_new_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.session.execute("artboard.new", &json!({"x": 400, "y": 300, "width": 100, "height": 50})).unwrap();
    select(&mut app, 0);
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let frame = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect: screen, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
    };
    // The bottom bar's buttons sit under its divider, New Artboard second from the right.
    let bar = frame(&mut app, 0.0, vec![])
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == Tokens::get(&ctx).divider => Some(r.rect),
            _ => None,
        })
        .unwrap();
    let new = pos2(bar.right() - 24.0 - 4.0 - 12.0, bar.bottom() + 2.0 + 12.0);
    let click = |modifiers: egui::Modifiers| {
        let b = |pressed| egui::Event::PointerButton { pos: new, button: egui::PointerButton::Primary, pressed, modifiers };
        vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(new), b(true), b(false)]
    };
    let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.len();
    let fields = |app: &VectorcraftApp| {
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, crate::dialogs::artboard_options::KIND);
        (d.str("name"), ["x", "y", "width", "height"].map(|k| d.f64(k, -1.0)))
    };
    // Alt-click: the dialog for Artboard 3, 20 pt right of Artboard 2 and as large; no artboard yet.
    frame(&mut app, 1.0, click(egui::Modifiers::ALT));
    assert_eq!(fields(&app), ("Artboard 3".to_string(), [520.0, 300.0, 100.0, 50.0]));
    assert_eq!(count(&app), 2, "no artboard is added before OK");
    // A plain click adds the artboard the dialog showed, with no dialog.
    crate::dialogs::cancel(&mut app);
    frame(&mut app, 2.0, click(egui::Modifiers::NONE));
    let ab = app.session.active().unwrap().doc.artboards[2].clone();
    assert_eq!((ab.name.as_str(), ab.rect), ("Artboard 3", vectorcraft_geom::Rect::new(520.0, 300.0, 620.0, 350.0)));
    assert!(app.ui.dialog.is_none());
    app.run("edit.undo", json!({})).unwrap();
    select(&mut app, 0);
    // Cancel adds nothing.
    frame(&mut app, 3.0, click(egui::Modifiers::ALT));
    crate::dialogs::cancel(&mut app);
    assert_eq!((count(&app), app.view().unwrap().artboard), (2, 0));
    // OK adds it with the fields, in one undo step, and makes it the active artboard.
    frame(&mut app, 4.0, click(egui::Modifiers::ALT));
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("name".into(), json!("Cover"));
    d.fields.insert("width".into(), json!(300));
    let undo = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(&mut app).unwrap();
    let ab = app.session.active().unwrap().doc.artboards[2].clone();
    assert_eq!((ab.name.as_str(), ab.rect), ("Cover", vectorcraft_geom::Rect::new(520.0, 300.0, 820.0, 350.0)));
    assert_eq!((app.view().unwrap().artboard, app.session.doc().unwrap().history.undo.len()), (2, undo + 1));
    assert!(app.ui.dialog.is_none());
}

/// `ui.newArtboard` answers with the dialog's kind. A name left empty gives the new artboard
/// its default name, and with the Artboard tool in use the tool moves to the new artboard.
#[test]
fn new_artboard_options_with_an_empty_name_add_the_default_name() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert!(app.run("ui.newArtboard", json!({})).is_err(), "no document");
    app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    app.select_tool("artboard");
    assert_eq!(app.run("ui.newArtboard", json!({})).unwrap(), json!({"dialog": "artboardOptions"}));
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(" "));
    crate::dialogs::confirm(&mut app).unwrap();
    let names: Vec<String> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Artboard 1", "Artboard 2"]);
    assert_eq!((selected(&app, 2), app.session.tool_options()["active"].clone()), (1, json!(1)));
}

/// The panel menu's Delete Empty Artboards is disabled while every artboard has art, and
/// enabled otherwise; a click deletes the empty ones in one undo step and makes the artboard
/// left active.
#[test]
fn delete_empty_artboards_deletes_the_empty_ones_and_selects_one_left() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    for x in [400, 800] {
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    let rect =
        |app: &mut VectorcraftApp, x: f64| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let ids: Vec<Value> = [10.0, 450.0, 850.0].into_iter().map(|x| rect(&mut app, x)).collect();
    let ctx = egui::Context::default();
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
    let raw = |events| egui::RawInput { events, screen_rect: screen, ..Default::default() };
    let item_of = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(raw(vec![]), |ui| menu(app, ui));
        out.textures_delta.clear();
        out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().trim_start() == "Delete Empty Artboards" => Some(t.visual_bounding_rect().center()),
            _ => None,
        })
    };
    // Menu rows pad their label where a check mark would go.
    let label = "   Delete Empty Artboards";
    assert!(named(&tree(|ui| menu(&mut app, ui)), label).is_disabled(), "every artboard has art");
    app.run("select.set", json!({"ids": [ids[0], ids[2]]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    select(&mut app, 2);
    assert!(!named(&tree(|ui| menu(&mut app, ui)), label).is_disabled(), "two artboards are empty");
    let item = item_of(&mut app).unwrap();
    let steps = app.session.active().unwrap().history.undo.len();
    let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    ctx.run_ui(raw(vec![egui::Event::PointerMoved(item), button(true), button(false)]), |ui| menu(&mut app, ui)).textures_delta.clear();
    let st = app.session.active().unwrap();
    let names: Vec<String> = st.doc.artboards.iter().map(|a| a.name.clone()).collect();
    assert_eq!(names, ["Artboard 2"]);
    assert_eq!(st.history.undo.len(), steps + 1, "one undo step");
    assert_eq!(selected(&app, 1), 0, "the artboard left is active");
}

/// With the Artboard tool, the canvas context menu locks and unlocks artboards: Lock Selected
/// locks the selected one (the others stay free), Unlock All frees every artboard.
#[test]
fn the_artboard_tools_context_menu_locks_and_unlocks_artboards() {
    use crate::menus::Item;
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100, "units": "Points", "artboards": 3})).unwrap();
    let locked = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
    let run = |app: &mut VectorcraftApp, menu: &str, label: &str| {
        let items = crate::menus::context_items(app);
        let Some(Item::Sub(_, subs)) = items.iter().find(|i| matches!(i, Item::Sub(t, _) if *t == menu)) else { panic!("no {menu}") };
        let Some(Item::Cmd(_, id, p)) = subs.iter().find(|i| matches!(i, Item::Cmd(l, ..) if *l == label)) else { panic!("no {label}") };
        crate::menus::invoke(app, id, p.clone());
    };
    // Without the Artboard tool the menu has no such submenu.
    assert!(!crate::menus::context_items(&app).iter().any(|i| matches!(i, Item::Sub(t, _) if *t == "Lock Artboards")));
    app.select_tool("artboard");
    select(&mut app, 1);
    run(&mut app, "Lock Artboards", "Lock Selected");
    assert_eq!(locked(&app), [false, true, false]);
    run(&mut app, "Lock Artboards", "Lock All Except Selected");
    assert_eq!(locked(&app), [true, true, true], "the others lock, the locked one stays");
    run(&mut app, "Unlock Artboards", "Unlock Selected");
    assert_eq!(locked(&app), [true, false, true]);
    run(&mut app, "Unlock Artboards", "Unlock All");
    assert_eq!(locked(&app), [false, false, false]);
}
