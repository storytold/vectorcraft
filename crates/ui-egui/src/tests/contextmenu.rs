//! The canvas context menu: right-click selects the object under the pointer and lists what
//! applies to the selection.

use craft::menu::Node;
use egui::{Event, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{VectorcraftApp, canvas, menus};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64) -> NodeId {
    NodeId(app.run("shape.rectangle", json!({"x": x, "y": y, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap())
}

/// One headless frame of the canvas on an 800 × 600 window.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
}

fn press(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers: Default::default() }
}

/// Click `button` at `pos` (move, press, release, then a frame to show the result).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, pos: Pos2, button: PointerButton) {
    frame(app, ctx, vec![Event::PointerMoved(pos)]);
    frame(app, ctx, vec![press(pos, button, true)]);
    frame(app, ctx, vec![press(pos, button, false)]);
    frame(app, ctx, vec![]);
}

/// Escape, which closes an open menu.
fn escape(app: &mut VectorcraftApp, ctx: &egui::Context) {
    frame(app, ctx, vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
}

/// Where document point `(x, y)` is on screen (after a first frame laid the canvas out).
fn screen(app: &VectorcraftApp, x: f64, y: f64) -> Pos2 {
    app.canvas_view().unwrap().to_screen(Point::new(x, y))
}

/// The labels of the menu's rows, submenus and headings included, depth first. A spelling suggestion
/// shows the word it puts in, not a label of the menu.
fn labels(app: &VectorcraftApp) -> Vec<String> {
    fn walk(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Item(it) if it.command.id == "text.editRange" => {}
                Node::Item(it) => out.push(it.label.clone()),
                Node::Heading(l) => out.push(l.clone()),
                Node::Submenu { label, children, .. } => {
                    out.push(label.clone());
                    walk(children, out);
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    walk(&menus::context_items(app), &mut out);
    out
}

/// The command (id and params) of every row of the menu, depth first.
fn commands(app: &VectorcraftApp) -> Vec<(String, serde_json::Value)> {
    fn walk(nodes: &[Node], out: &mut Vec<(String, serde_json::Value)>) {
        for n in nodes {
            match n {
                Node::Item(it) => out.push((it.command.id.clone(), it.command.params.clone())),
                Node::Submenu { children, .. } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = vec![];
    walk(&menus::context_items(app), &mut out);
    out
}

fn has(app: &VectorcraftApp, label: &str) -> bool {
    labels(app).iter().any(|l| l == label)
}

fn has_command(app: &VectorcraftApp, id: &str) -> bool {
    commands(app).iter().any(|(c, _)| c == id)
}

#[test]
fn right_click_selects_the_object_and_lists_its_commands() {
    let mut app = app();
    let id = rect(&mut app, 50.0, 50.0);
    app.run("select.none", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    assert!(app.ui.context_menu_at.is_none() && !has(&app, "Arrange"));
    let p = screen(&app, 100.0, 100.0);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert_eq!(app.session.active().unwrap().selection.objects, [id], "the object right-clicked is selected");
    assert!(app.ui.context_menu_at.is_some(), "the menu is open");
    for label in ["Cut", "Copy", "Make Guides", "Transform", "Arrange", "Select", "Export Selection…"] {
        assert!(has(&app, label), "{label} in {:?}", labels(&app));
    }
    assert!(has_command(&app, "object.arrange.bringToFront"));
    // Nothing that needs several objects, a group or nothing selected.
    for label in ["Group", "Ungroup", "Make Clipping Mask", "Release Compound Path", "Select All", "Zoom In"] {
        assert!(!has(&app, label), "{label} in {:?}", labels(&app));
    }
}

#[test]
fn right_click_on_empty_canvas_offers_the_view_and_selects_nothing() {
    let mut app = app();
    app.select_tool("rectangle");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let p = screen(&app, 300.0, 250.0);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.session.active().unwrap().selection.objects.is_empty());
    for label in ["Zoom In", "Zoom Out", "Fit Artboard in Window", "Show Rulers", "Select All"] {
        assert!(has(&app, label), "{label} in {:?}", labels(&app));
    }
    assert!(!has(&app, "Cut") && !has(&app, "Arrange"));
    // The secondary press starts no drag of the Rectangle tool.
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None);
}

/// The document point at the centre of the canvas view, where a key request reports itself.
fn view_centre(app: &VectorcraftApp) -> Point {
    let view = app.canvas_view().unwrap();
    view.to_doc(view.rect.center())
}

/// `key` with `modifiers`, pressed on the focused canvas, then a frame to show the result.
fn menu_key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    frame(app, ctx, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
    frame(app, ctx, vec![]);
}

/// Rectangle A, selected with a click (which also gives the canvas the keyboard focus), and an unselected
/// rectangle B under the view's centre with a locked rectangle over it.
fn selection_away_from_the_centre(app: &mut VectorcraftApp, ctx: &egui::Context) -> (NodeId, NodeId) {
    let a = rect(app, 0.0, 0.0);
    frame(app, ctx, vec![]);
    let c = view_centre(app);
    let b = rect(app, c.x - 50.0, c.y - 50.0);
    let locked = rect(app, c.x - 25.0, c.y - 25.0);
    app.run("layer.setProps", json!({"ids": [locked.0], "locked": true})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let p = screen(app, 50.0, 50.0);
    click(app, ctx, p, PointerButton::Primary);
    assert_eq!(app.session.active().unwrap().selection.objects, [a]);
    (a, b)
}

/// Asserts the menu opened for `a` as selected, at a point over it, without what lies under the view's centre.
fn opened_for_the_selection(app: &VectorcraftApp, a: NodeId) {
    assert_eq!(app.session.active().unwrap().selection.objects, [a], "a key request keeps the selection");
    let at = app.ui.context_menu_at.expect("the menu is open");
    let over_a = Rect::from_two_pos(screen(app, 0.0, 0.0), screen(app, 100.0, 100.0));
    assert!(over_a.contains(at), "the menu opens at the selection: {at:?} in {over_a:?}");
    assert!(has(app, "Arrange") && has_command(app, "object.arrange.bringToFront"), "{:?}", labels(app));
    assert!(app.ui.context_locked.is_none() && !has(app, "Unlock"), "nothing under the view's centre is measured");
}

#[test]
fn shift_f10_opens_the_menu_for_the_selection_without_changing_it() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    let (a, _) = selection_away_from_the_centre(&mut app, &ctx);
    menu_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
    opened_for_the_selection(&app, a);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn menu_key_opens_the_menu_for_the_selection_without_changing_it() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    let (a, _) = selection_away_from_the_centre(&mut app, &ctx);
    menu_key(&mut app, &ctx, egui::Key::Menu, egui::Modifiers::NONE);
    opened_for_the_selection(&app, a);
}

/// With nothing selected, Shift+F10 opens the menu at the view's centre and selects nothing, even over an object.
#[test]
fn shift_f10_with_nothing_selected_opens_the_menu_at_the_view_centre() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let c = view_centre(&app);
    rect(&mut app, c.x - 50.0, c.y - 50.0);
    app.run("select.none", json!({})).unwrap();
    // A click on the empty artboard gives the canvas the focus and keeps the selection empty.
    let p = screen(&app, 380.0, 280.0);
    click(&mut app, &ctx, p, PointerButton::Primary);
    assert!(app.session.active().unwrap().selection.objects.is_empty());
    menu_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "the object under the view's centre stays unselected");
    let at = app.ui.context_menu_at.expect("the menu is open");
    let centre = app.canvas_view().unwrap().rect.center();
    assert!(at.distance(centre) < 0.5, "{at:?} at {centre:?}");
    assert!(has(&app, "Select All") && !has(&app, "Arrange"), "{:?}", labels(&app));
}

/// With the Artboard tool, Shift+F10 opens the menu for the selected artboard, at it, and selects no other artboard;
/// the menu's Paste would put artboards on it.
#[test]
fn shift_f10_with_the_artboard_tool_keeps_the_selected_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    crate::menus::invoke(&mut app, "view.fitAll", json!({}));
    for _ in 0..3 {
        frame(&mut app, &ctx, vec![]);
    }
    let boards: Vec<vectorcraft_geom::Rect> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let centre = view_centre(&app);
    let away = boards.iter().position(|r| !r.contains(centre)).unwrap();
    app.select_tool("artboard");
    // A click on the artboard selects it and gives the canvas the focus.
    let c = boards[away].center();
    let p = screen(&app, c.x, c.y);
    click(&mut app, &ctx, p, PointerButton::Primary);
    assert_eq!(crate::panels::artboards::picked(&app, boards.len()), [away]);
    menu_key(&mut app, &ctx, egui::Key::F10, egui::Modifiers::SHIFT);
    assert_eq!(crate::panels::artboards::picked(&app, boards.len()), [away], "a key request keeps the selected artboard");
    let at = app.ui.context_menu_at.expect("the menu is open");
    let on_board = Rect::from_two_pos(screen(&app, boards[away].x0, boards[away].y0), screen(&app, boards[away].x1, boards[away].y1));
    assert!(on_board.contains(at), "the menu opens at the selected artboard: {at:?} in {on_board:?}");
    let pasted_at = app.ui.context_artboards.as_ref().map(|f| f.at).expect("the menu's artboard facts");
    assert!(boards[away].contains(pasted_at), "{pasted_at:?} on {:?}", boards[away]);
}

/// A right-click on an object that is not selected still selects it, with another object selected before.
#[test]
fn right_click_still_selects_the_object_under_the_pointer() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    let (_, b) = selection_away_from_the_centre(&mut app, &ctx);
    // Inside B, outside the locked rectangle over its middle.
    let c = view_centre(&app);
    let p = screen(&app, c.x - 40.0, c.y - 40.0);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert_eq!(app.session.active().unwrap().selection.objects, [b]);
    assert!(app.ui.context_menu_at.is_some_and(|at| at.distance(p) < 0.5), "the menu opens at the pointer");
}

/// A right-click over a locked object adds an Unlock submenu that lists the object by its name; once
/// the menu is closed, the menu lists no Unlock items.
#[test]
fn right_click_on_a_locked_object_offers_unlock_by_its_name() {
    let mut app = app();
    let id = rect(&mut app, 50.0, 50.0);
    app.run("layer.setProps", json!({"ids": [id.0], "name": "Sky", "locked": true})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let p = screen(&app, 100.0, 100.0);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert_eq!(app.ui.context_locked, Some((app.session.active().unwrap().uid, vec![id])), "the locked object under the pointer");
    assert!(has(&app, "Unlock"), "{:?}", labels(&app));
    // Agents see the item under the Unlock path, by the object's name.
    let entries = menus::context_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Unlock"]).unwrap();
    assert_eq!((item.label.as_str(), item.command.as_deref()), ("Sky", Some("layer.setProps")));
    assert_eq!(item.params, json!({"ids": [id.0], "locked": false}));
    escape(&mut app, &ctx);
    assert!(app.ui.context_locked.is_none(), "closing the menu forgets it");
    assert!(app.ui.context_menu_at.is_none());
    assert!(!has(&app, "Unlock"));
}

/// Cmd+Alt+Shift+U is Switch Units by default: pressed, the document goes to the unit after its own in
/// the ruler's menu (wrapping from Feet to Points), and Smart Guides (Cmd+U) stay as they are.
#[test]
fn cmd_alt_shift_u_switches_to_the_next_unit() {
    use vectorcraft_doc::Unit;
    let mut app = app();
    crate::tests_synthetic::key_frame(&mut app, &crate::testing::ui_context(), vec![]);
    assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has the key");
    fn press(app: &mut VectorcraftApp) {
        let ctx = crate::testing::ui_context();
        crate::tests_synthetic::key_frame(app, &ctx, vec![]);
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT;
        let ev = Event::Key { key: egui::Key::U, physical_key: None, pressed: true, repeat: false, modifiers };
        crate::tests_synthetic::key_frame(app, &ctx, vec![ev]);
    }
    let guides = app.ui.view.smart_guides;
    press(&mut app);
    assert_eq!(app.session.general_unit(), Unit::Picas, "Points → Picas");
    assert_eq!(app.ui.view.smart_guides, guides, "Smart Guides stay as they are");
    app.run("document.setUnits", json!({"units": "Feet"})).unwrap();
    press(&mut app);
    assert_eq!(app.session.general_unit(), Unit::Points, "Feet → Points");
}

/// With the Quick Pen, the menu has Delete, which removes the selected anchor points; with another
/// tool it does not.
#[test]
fn the_quick_pen_menu_deletes_the_selected_anchors() {
    let mut app = app();
    let a = rect(&mut app, 0.0, 0.0);
    app.run("select.anchors", json!({"id": a.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    assert!(!has(&app, "Delete"), "the Selection tool");
    app.select_tool("quickPen");
    assert!(has(&app, "Delete"), "{:?}", labels(&app));
    let delete = menus::context_entries(&app).into_iter().find(|e| e.label == "Delete").expect("Delete");
    assert_eq!(delete.command.as_deref(), Some("edit.clear"));
    menus::invoke(&mut app, "edit.clear", json!({}));
    let st = app.session.active().unwrap();
    let pd = st.doc.node(a).unwrap().path_data().unwrap().clone();
    assert_eq!(pd.subpaths[0].anchors.len(), 3, "one anchor removed");
    // Nothing selected: no Delete.
    app.run("select.none", json!({})).unwrap();
    assert!(!has(&app, "Delete"));
}

#[test]
fn the_menu_follows_the_selection() {
    let mut app = app();
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 50.0, 50.0));
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    for label in ["Group", "Join", "Make Clipping Mask", "Make Compound Path", "Objects on Path", "Bring to Front", "Move…"] {
        assert!(has(&app, label), "{label} in {:?}", labels(&app));
    }
    assert!(!has(&app, "Ungroup") && !has(&app, "Isolate Selected Group"));
    app.run("object.group", json!({})).unwrap();
    assert!(has(&app, "Ungroup") && has(&app, "Isolate Selected Group"));
    assert!(!has(&app, "Group") && !has(&app, "Join"));
    // Isolated: a way out.
    app.run("object.isolate", json!({})).unwrap();
    assert!(has_command(&app, "object.exitIsolation"));
    // Agents see the same menu, with submenus as paths.
    let entries = menus::context_entries(&app);
    assert!(entries.iter().any(|e| e.command.as_deref() == Some("object.exitIsolation") && e.enabled));
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let entries = menus::context_entries(&app);
    let front = entries.iter().find(|e| e.command.as_deref() == Some("object.arrange.bringToFront")).unwrap();
    assert_eq!(front.path, ["Arrange"]);
    // No separator leads, trails or doubles up.
    let nodes = menus::context_items(&app);
    assert!(!matches!(nodes.first(), Some(Node::Separator)) && !matches!(nodes.last(), Some(Node::Separator)));
    assert!(nodes.windows(2).all(|w| !matches!(w, [Node::Separator, Node::Separator])));
}

/// The labels of the context menu's isolation items.
fn isolate_labels(app: &VectorcraftApp) -> Vec<String> {
    labels(app).into_iter().filter(|l| l.starts_with("Isolate Selected")).collect()
}

/// One selected path has Isolate Selected Path, which isolates that path.
#[test]
fn single_path_context_item_enters_isolation() {
    let mut app = app();
    let a = rect(&mut app, 0.0, 0.0);
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    assert_eq!(isolate_labels(&app), ["Isolate Selected Path"]);
    assert!(has_command(&app, "object.isolate"));
    app.run("object.isolate", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().isolation, Some(a), "the path is isolated");
    assert!(has_command(&app, "object.exitIsolation"));
}

/// The isolation item names the kind of the one selected object: a group or a compound path is a
/// group, a path (a group's member too) is a path, and no object or two objects have neither.
#[test]
fn isolation_context_item_follows_selected_kind() {
    let mut app = app();
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 50.0, 50.0));
    app.run("select.none", json!({})).unwrap();
    assert!(isolate_labels(&app).is_empty(), "nothing selected");
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    assert!(isolate_labels(&app).is_empty(), "two paths");
    app.run("object.compoundPath.make", json!({})).unwrap();
    assert_eq!(isolate_labels(&app), ["Isolate Selected Group"], "a compound path selected whole");
    let compound = app.session.active().unwrap().selection.objects.clone();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, [a], "a compound path's member selected on its own");
    assert_eq!(isolate_labels(&app), ["Isolate Selected Path"], "a path in a compound path");
    app.run("select.set", json!({"ids": [compound[0].0]})).unwrap();
    app.run("object.compoundPath.release", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    assert_eq!(isolate_labels(&app), ["Isolate Selected Group"], "a group");
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, [a], "a member selected on its own");
    assert_eq!(isolate_labels(&app), ["Isolate Selected Path"], "a path in a group");
    app.run("object.isolate", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().isolation, Some(a), "the member is isolated");
}

/// Neither isolation item shows for an object `object.isolate` refuses, which a selection set by a
/// command can still hold: a hidden or locked object, a path in a locked group, and a guide while
/// guides are locked.
#[test]
fn isolation_context_item_skips_objects_that_cannot_be_isolated() {
    let mut app = app();
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 50.0, 50.0));
    let set = |app: &mut VectorcraftApp, id: NodeId, f: fn(&mut vectorcraft_doc::Node)| {
        let st = app.session.doc_mut().unwrap();
        f(std::sync::Arc::make_mut(&mut st.doc).node_mut(id).unwrap());
        st.selection.set([id]);
    };
    set(&mut app, a, |n| n.locked = true);
    assert!(isolate_labels(&app).is_empty(), "a locked path");
    set(&mut app, a, |n| {
        n.locked = false;
        n.visible = false;
    });
    assert!(isolate_labels(&app).is_empty(), "a hidden path");
    set(&mut app, a, |n| n.visible = true);
    assert_eq!(isolate_labels(&app), ["Isolate Selected Path"]);
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    let g = app.session.active().unwrap().selection.objects[0];
    set(&mut app, g, |n| n.locked = true);
    assert!(isolate_labels(&app).is_empty(), "a locked group");
    app.session.doc_mut().unwrap().selection.set([a]);
    assert!(isolate_labels(&app).is_empty(), "a path in a locked group");
    set(&mut app, g, |n| n.locked = false);
    app.run("object.ungroup", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    app.run("view.guides.make", json!({})).unwrap();
    app.run("view.guides.lock", json!({})).unwrap();
    assert!(app.session.guides_locked());
    app.session.doc_mut().unwrap().selection.set([a]);
    assert!(isolate_labels(&app).is_empty(), "a guide while guides are locked");
}

/// While a symbol is edited, a path in the symbol's art has Isolate Selected Path, and a path
/// outside it, which a selection set by a command can still hold, has no isolation item.
#[test]
fn isolation_context_item_skips_objects_outside_the_edited_symbol() {
    let mut app = app();
    let outside = rect(&mut app, 0.0, 0.0);
    rect(&mut app, 200.0, 0.0);
    app.run("symbol.new", json!({"name": "Box"})).unwrap();
    app.run("symbol.edit", json!({"name": "Box"})).unwrap();
    assert!(app.session.active().unwrap().doc.symbol_edit.is_some(), "symbol editing mode");
    let inside = rect(&mut app, 100.0, 100.0);
    app.session.doc_mut().unwrap().selection.set([inside]);
    assert_eq!(isolate_labels(&app), ["Isolate Selected Path"], "a path in the symbol's art");
    app.session.doc_mut().unwrap().selection.set([outside]);
    assert!(isolate_labels(&app).is_empty(), "a path outside the symbol's art");
    assert!(app.run("object.isolate", json!({"id": outside.0})).is_err(), "object.isolate refuses it");
}

/// Simplify… and Smooth… are listed while the selection holds a path: a path, a group of paths, a
/// compound path, a path with type, and anchors selected with Direct Selection (whose Remove
/// Anchor Points stays). They are left out with nothing, only type or a group of type selected.
#[test]
fn simplify_and_smooth_follow_the_selections_paths() {
    let mut app = app();
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 150.0, 0.0));
    let t = NodeId(app.run("text.create", json!({"x": 10, "y": 250, "text": "Type"})).unwrap()["id"].as_u64().unwrap());
    // Both or neither, never one without the other.
    let smooth = |app: &VectorcraftApp| {
        let (simplify, smooth) = (has(app, "Simplify…"), has(app, "Smooth…"));
        assert_eq!(simplify, smooth, "Simplify… and Smooth… follow the same rule: {:?}", labels(app));
        smooth
    };
    app.run("select.none", json!({})).unwrap();
    assert!(!smooth(&app), "nothing selected");
    app.run("select.set", json!({"ids": [t.0]})).unwrap();
    assert!(!smooth(&app), "only type selected");
    let u = NodeId(app.run("text.create", json!({"x": 10, "y": 300, "text": "More"})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [t.0, u.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    assert!(!smooth(&app), "a group of type only");
    app.run("object.ungroup", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    assert!(smooth(&app), "a path");
    app.run("select.set", json!({"ids": [a.0, t.0]})).unwrap();
    assert!(smooth(&app), "a path and type");
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.group", json!({})).unwrap();
    assert!(smooth(&app), "a group of paths");
    app.run("object.ungroup", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.compoundPath.make", json!({})).unwrap();
    assert!(smooth(&app), "a compound path");
    app.run("object.compoundPath.release", json!({})).unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": a.0, "anchors": [[0, 1]]})).unwrap();
    assert!(smooth(&app), "an anchor point direct-selected");
    assert!(has(&app, "Remove Anchor Points"), "the anchor entries stay");
}

/// Selected type has Bullets and Numbering › Apply Bullets in the context menu and the Type menu,
/// which makes its paragraphs a bulleted list; other art has no such item.
#[test]
fn selected_type_has_apply_bullets() {
    let mut app = app();
    let r = rect(&mut app, 0.0, 0.0);
    app.run("select.set", json!({"ids": [r.0]})).unwrap();
    let bullets = |app: &VectorcraftApp| menus::context_entries(app).into_iter().find(|e| e.command.as_deref() == Some("type.applyBullets"));
    assert!(bullets(&app).is_none(), "a rectangle has no paragraphs");
    let t = NodeId(app.run("text.create", json!({"x": 10, "y": 200, "text": "Item"})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [t.0]})).unwrap();
    let e = bullets(&app).expect("Apply Bullets");
    assert_eq!((e.path.as_slice(), e.label.as_str(), e.enabled), (&["Bullets and Numbering".to_string()][..], "Apply Bullets", true));
    let typed = menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("type.applyBullets")).expect("in the Type menu");
    assert_eq!(typed.path, ["Type", "Bullets and Numbering"]);
    menus::invoke(&mut app, "type.applyBullets", json!({}));
    let Some(NodeKind::Text(text)) = app.session.active().unwrap().doc.node(t).map(|n| &n.kind) else { panic!("text") };
    assert!(text.para_at(0).list.is_some());
}

/// Selected type has Bullets and Numbering › Apply Numbers in the context menu and the Type menu,
/// after Apply Bullets, which makes its paragraphs a numbered list; other art has no such item.
#[test]
fn selected_type_has_apply_numbers() {
    let mut app = app();
    let r = rect(&mut app, 0.0, 0.0);
    app.run("select.set", json!({"ids": [r.0]})).unwrap();
    let numbers = |app: &VectorcraftApp| menus::context_entries(app).into_iter().position(|e| e.command.as_deref() == Some("type.applyNumbers"));
    assert!(numbers(&app).is_none(), "a rectangle has no paragraphs");
    let t = NodeId(app.run("text.create", json!({"x": 10, "y": 200, "text": "Item"})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [t.0]})).unwrap();
    let entries = menus::context_entries(&app);
    let at = numbers(&app).expect("Apply Numbers");
    let e = &entries[at];
    assert_eq!((e.path.as_slice(), e.label.as_str(), e.enabled), (&["Bullets and Numbering".to_string()][..], "Apply Numbers", true));
    assert_eq!(entries.get(at.wrapping_sub(1)).and_then(|e| e.command.as_deref()), Some("type.applyBullets"));
    let typed = menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("type.applyNumbers")).expect("in the Type menu");
    assert_eq!(typed.path, ["Type", "Bullets and Numbering"]);
    menus::invoke(&mut app, "type.applyNumbers", json!({}));
    let Some(NodeKind::Text(text)) = app.session.active().unwrap().doc.node(t).map(|n| &n.kind) else { panic!("text") };
    assert_eq!(text.para_at(0).list.and_then(|l| l.number), Some(vectorcraft_doc::text::NumberStyle::Arabic));
}

/// With the Intertwine tool, a right-click where three or more objects of the group overlap lists
/// Bring to Front, Bring Forward, Send Backward and Send to Back for the object nearest to the
/// pointer in place of the Arrange submenu. Elsewhere the menu is the usual one.
#[test]
fn right_click_where_three_intertwined_objects_overlap_arranges_the_nearest_one() {
    let mut app = app();
    // Rectangles from x = 50, 90 and 130, 120 wide: all three overlap from x = 130 to 170.
    let wide = |app: &mut VectorcraftApp, x: f64| {
        NodeId(app.run("shape.rectangle", json!({"x": x, "y": 50, "width": 120, "height": 100})).unwrap()["id"].as_u64().unwrap())
    };
    let (a, b, c) = (wide(&mut app, 50.0), wide(&mut app, 90.0), wide(&mut app, 130.0));
    app.run("select.set", json!({"ids": [a.0, b.0, c.0]})).unwrap();
    let g = NodeId(app.run("object.intertwine.make", json!({})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(app.session.tool_id(), "intertwine");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    // By the right edge of A (the first rectangle), where all three overlap.
    let p = screen(&app, 169.0, 100.0);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert_eq!(app.session.active().unwrap().selection.objects, [g], "the selection stays the group");
    assert!(app.ui.context_intertwine.is_some());
    assert!(!has(&app, "Arrange"), "no Arrange submenu: {:?}", labels(&app));
    let arrange: Vec<_> = commands(&app).into_iter().filter(|(id, _)| id == "object.intertwine.arrange").collect();
    let orders: Vec<_> = arrange.iter().map(|(_, params)| (params["id"].clone(), params["order"].clone(), params["member"].clone())).collect();
    let want: Vec<_> = ["front", "forward", "backward", "back"].iter().map(|o| (json!(g.0), json!(o), json!(a.0))).collect();
    assert_eq!(orders, want, "the four orders for the nearest member, in that order");
    // Where only two overlap, the menu is the usual one.
    escape(&mut app, &ctx);
    let p = screen(&app, 110.0, 100.0);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_intertwine.is_none());
    assert!(has(&app, "Arrange") && !has_command(&app, "object.intertwine.arrange"), "{:?}", labels(&app));
}

/// Every label the context menu can show is a menu string, so complete catalogs must translate it.
#[test]
fn every_context_label_is_a_menu_string() {
    let strings = menus::menu_strings();
    let mut app = app();
    let mut shown = vec![];
    // Nothing selected, one path, two paths, a group, a clipping group, a compound path, isolation.
    shown.extend(labels(&app));
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 50.0, 50.0));
    shown.extend(labels(&app));
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    shown.extend(labels(&app));
    app.run("object.compoundPath.make", json!({})).unwrap();
    shown.extend(labels(&app));
    app.run("object.compoundPath.release", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.clippingMask.make", json!({})).unwrap();
    shown.extend(labels(&app));
    app.run("object.group", json!({})).unwrap();
    shown.extend(labels(&app));
    app.run("object.isolate", json!({})).unwrap();
    shown.extend(labels(&app));
    // The Unlock submenu that a right-click over a locked object adds.
    app.run("object.exitIsolation", json!({})).unwrap();
    app.run("layer.setProps", json!({"ids": [a.0], "locked": true})).unwrap();
    app.ui.context_locked = Some((app.session.active().unwrap().uid, vec![a]));
    shown.extend(labels(&app));
    // A word Auto Spell Check underlined, with no suggestions: Ignore All, Add to Dictionary.
    let text = NodeId(app.run("text.create", json!({"x": 10, "y": 200, "text": "Zqxvbtkw"})).unwrap()["id"].as_u64().unwrap());
    app.ui.context_spelling = Some(crate::state::ContextSpelling {
        uid: app.session.active().unwrap().uid,
        id: text,
        start: 0,
        end: 8,
        word: "Zqxvbtkw".into(),
        suggestions: vec![],
    });
    shown.extend(labels(&app));
    app.ui.context_spelling = None;
    // The Lock Artboards and Unlock Artboards submenus of the Artboard tool.
    app.run("artboard.new", json!({"x": 400, "y": 0, "width": 100, "height": 80})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "locked": true})).unwrap();
    app.select_tool("artboard");
    shown.extend(labels(&app));
    // Lock Selected with the artboard that isn't locked active.
    crate::panels::artboards::select(&mut app, 1);
    shown.extend(labels(&app));
    let missing: Vec<_> = shown.iter().filter(|l| !strings.contains(l.as_str())).collect();
    assert!(missing.is_empty(), "context menu labels missing from menu_strings: {missing:?}");
    for label in menus::CONTEXT_LABELS {
        assert!(shown.iter().any(|s| s == *label), "`{label}` is listed but the context menu never shows it");
    }
}

/// A canvas in the same fixture and command lifecycle as a window.
struct CanvasMenu {
    app: std::rc::Rc<std::cell::RefCell<VectorcraftApp>>,
    fixture: craft::ui::testing::Fixture,
}

impl CanvasMenu {
    fn new(app: VectorcraftApp) -> Self {
        let app = std::rc::Rc::new(std::cell::RefCell::new(app));
        let shown = std::rc::Rc::clone(&app);
        let mut fixture = craft::ui::testing::Fixture::new(move |ui| canvas::show(&mut shown.borrow_mut(), ui));
        fixture.room(vec2(800.0, 600.0));
        app.borrow_mut().ui_ctx = Some(fixture.ctx().clone());
        let mut canvas = Self { app, fixture };
        canvas.frame(vec![]);
        canvas
    }

    fn frame(&mut self, events: Vec<Event>) {
        crate::testing::fixture_input(&mut self.app.borrow_mut(), &mut self.fixture);
        self.fixture.frame(events);
        crate::testing::fixture_input(&mut self.app.borrow_mut(), &mut self.fixture);
    }

    fn right_click(&mut self, x: f64, y: f64) {
        let p = screen(&self.app.borrow(), x, y);
        self.frame(vec![Event::PointerMoved(p)]);
        self.frame(vec![press(p, PointerButton::Secondary, true)]);
        self.frame(vec![press(p, PointerButton::Secondary, false)]);
        self.frame(vec![]);
        assert!(self.app.borrow().ui.context_menu_at.is_some(), "the menu opened");
    }

    fn outline_item(&self) -> Option<craft::ui::testing::Target> {
        let app = self.app.borrow();
        let entry = menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("object.path.outlineStroke")).unwrap();
        let label = menus::shown_label(&app, entry.command.as_deref().unwrap(), &entry.params, &entry.label);
        let tree = self.fixture.tree();
        self.fixture.by_role(craft::ui::contract::Role::MENU_ITEM).into_iter().find(|item| tree.name_of(item.node) == Some(label.as_str()))
    }

    fn click_outline(&mut self) {
        let item = self.outline_item().expect("the rendered outline item is present");
        assert!(!self.fixture.tree().is_disabled_key(item.node), "the rendered outline item is enabled");
        self.fixture.click(&item);
        crate::testing::fixture_input(&mut self.app.borrow_mut(), &mut self.fixture);
        self.frame(vec![]);
    }

    fn escape(&mut self) {
        self.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    }
}

/// #5267: a right-click on a stroked object offers Outline Stroke, which runs
/// `object.path.outlineStroke` (the Object › Path item) and leaves filled art; an object with no
/// stroke to outline has no such item.
#[test]
fn the_canvas_context_menu_offers_outline_stroke_for_a_stroked_selection() {
    use vectorcraft_doc::AppearanceItem;
    let mut app = app();
    let id = rect(&mut app, 50.0, 50.0);
    // Stroked: a visible stroke of some weight.
    app.run("stroke.set", json!({"weight": 6})).unwrap();
    let strokes = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let n = st.doc.node(st.selection.objects[0]).unwrap();
        n.appearance.items.iter().filter(|i| matches!(i, AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() && s.width > 0.0)).count()
    };
    assert!(strokes(&app) > 0, "the rectangle starts with a stroke");
    app.run("select.none", json!({})).unwrap();
    let mut canvas = CanvasMenu::new(app);
    canvas.right_click(100.0, 100.0);
    canvas.click_outline();
    {
        let app = canvas.app.borrow();
        let st = app.session.active().unwrap();
        assert!(st.doc.node(id).is_none(), "the stroked rectangle was replaced by its outline");
        assert_eq!(strokes(&app), 0, "the outline is filled art with no stroke left");
    }

    // The outline has no stroke any more: nothing to outline, so no item.
    canvas.right_click(100.0, 100.0);
    assert!(canvas.outline_item().is_none(), "no stroke, no rendered outline item");
}

#[test]
fn outline_stroke_menu_ignores_a_point_with_a_nonpainting_cap() {
    for (cap, closed) in [("butt", false), ("round", false), ("square", false), ("butt", true), ("round", true), ("square", true)] {
        let mut app = app();
        let point = NodeId(
            app.run("path.create", json!({"closed": closed, "anchors": [{"x": 250, "y": 200}, {"x": 250, "y": 200}]})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        app.run("paint.setFill", json!({"none": true})).unwrap();
        app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
        app.run("stroke.set", json!({"weight": 8, "cap": cap})).unwrap();
        let visible = rect(&mut app, 50.0, 50.0);
        app.run("paint.setFill", json!({"color": "#ff8010"})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        app.run("select.set", json!({"ids": [point.0, visible.0]})).unwrap();
        let selected = app.session.active().unwrap().selection.objects.clone();
        let mut canvas = CanvasMenu::new(app);
        canvas.right_click(100.0, 100.0);
        assert_eq!(canvas.app.borrow().session.active().unwrap().selection.objects, selected);
        if cap == "butt" {
            assert!(canvas.outline_item().is_none(), "the point paints nothing with a butt cap");
        } else {
            canvas.click_outline();
            let app = canvas.app.borrow();
            let st = app.session.active().unwrap();
            assert!(st.doc.node(point).is_none(), "{cap} cap was outlined");
            assert!(st.doc.node(visible).is_some(), "the fill-only object is unchanged");
            let outlined = st.selection.objects.iter().find(|&&id| id != visible).and_then(|&id| st.doc.node(id)).expect("selected outline art");
            assert!(
                outlined.appearance.items.iter().any(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(f) if f.visible && !f.paint.is_none())),
                "{cap} cap becomes filled art"
            );
            let bounds = outlined.geometric_bounds().expect("the cap outline has geometry");
            assert!(bounds.width() > 0.0 && bounds.height() > 0.0, "{cap} cap paints an area");
        }
    }
}

/// A compound path of two rectangles, filled and stroked with `weight`, isolated, with a member
/// right-clicked. The member stays selected and has no appearance of its own: the compound holds
/// the paint.
fn right_click_member_of_compound(weight: f64) -> (CanvasMenu, NodeId) {
    let mut app = app();
    let (a, b) = (rect(&mut app, 50.0, 50.0), rect(&mut app, 200.0, 50.0));
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.compoundPath.make", json!({})).unwrap();
    let compound = app.session.active().unwrap().selection.objects[0];
    // Filled, so that a click inside a member hits it whether or not the compound has a stroke.
    app.run("paint.setFill", json!({"color": "#ff8010"})).unwrap();
    app.run("stroke.set", json!({"weight": weight})).unwrap();
    app.run("object.isolate", json!({"id": compound.0})).unwrap();
    let member = app.session.active().unwrap().doc.node(compound).and_then(|n| n.children().and_then(|c| c.first()).map(|m| m.id)).unwrap();
    app.run("select.set", json!({"ids": [member.0]})).unwrap();
    let mut canvas = CanvasMenu::new(app);
    canvas.right_click(100.0, 100.0);
    {
        let app = canvas.app.borrow();
        let st = app.session.active().unwrap();
        assert_eq!(st.selection.objects, [member], "the right-click leaves the member selected");
        assert!(st.doc.node(member).is_some_and(|m| m.appearance.items.is_empty()), "a member has no appearance of its own");
    }
    (canvas, compound)
}

/// With a member of an isolated compound path selected (a right-click inside the isolated compound
/// leaves a member selected), the member's own appearance is empty: the compound holds the paint.
/// `object.path.outlineStroke` promotes the member to its compound, so the menu offers Outline
/// Stroke exactly when the compound has a stroke to outline.
#[test]
fn outline_stroke_is_offered_for_a_member_of_a_stroked_compound_path() {
    // No stroke on the compound path: nothing to outline.
    let (canvas, _) = right_click_member_of_compound(0.0);
    assert!(canvas.outline_item().is_none(), "no stroke, no rendered outline item");

    // Stroked: the item is offered for the member, and the command outlines the compound.
    let (mut canvas, compound) = right_click_member_of_compound(6.0);
    canvas.click_outline();
    assert!(canvas.app.borrow().session.active().unwrap().doc.node(compound).is_none(), "the stroked compound path was replaced by its outline");
}

#[test]
fn outline_stroke_ignores_a_linked_dimensions_lines_when_its_source_is_selected() {
    for grouped in [false, true] {
        let mut app = app();
        let source = rect(&mut app, 50.0, 50.0);
        app.run("paint.setFill", json!({"color": "#ff8010"})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        let dimension =
            NodeId(
                app.run("dimension.create", json!({"source": {"kind": "segment", "edge": {"node": source.0, "subpath": 0, "segment": 0}}})).unwrap()
                    ["id"]
                    .as_u64()
                    .unwrap(),
            );
        app.run("select.set", json!({"ids": [source.0, dimension.0]})).unwrap();
        if grouped {
            app.run("object.group", json!({})).unwrap();
        }
        let selected = app.session.active().unwrap().selection.objects.clone();
        let mut canvas = CanvasMenu::new(app);
        canvas.right_click(100.0, 125.0);
        assert_eq!(canvas.app.borrow().session.active().unwrap().selection.objects, selected, "grouped: {grouped}");
        assert!(canvas.outline_item().is_none(), "only the protected dimension has strokes");
        canvas.escape();

        let dimension_art = {
            let mut app = canvas.app.borrow_mut();
            app.run("paint.setStroke", json!({"ids": [source.0], "color": "#000000"})).unwrap();
            app.run("stroke.set", json!({"ids": [source.0], "weight": 6})).unwrap();
            app.session.active().unwrap().doc.node(dimension).unwrap().children().unwrap().to_vec()
        };
        canvas.right_click(100.0, 125.0);
        assert_eq!(canvas.app.borrow().session.active().unwrap().selection.objects, selected, "grouped: {grouped}");
        canvas.click_outline();
        let app = canvas.app.borrow();
        let st = app.session.active().unwrap();
        assert!(st.doc.node(source).is_none(), "the source was replaced by filled outlines");
        assert_eq!(st.doc.node(dimension).unwrap().children().unwrap(), dimension_art.as_slice(), "the dimension keeps its own art");
    }
}

/// Point type holding `t` at (20, 60), with Auto Spell Check on.
fn spelled(app: &mut VectorcraftApp, t: &str) -> NodeId {
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 60, "text": t, "size": 24})).unwrap()["id"].as_u64().unwrap());
    app.run("select.none", json!({})).unwrap();
    app.run("edit.autoSpellCheck", json!({})).unwrap();
    id
}

/// Where on screen the middle of bytes `a..b` of type object `id` is.
fn word_at(app: &VectorcraftApp, id: NodeId, a: usize, b: usize) -> Pos2 {
    let st = app.session.active().unwrap();
    let Some(NodeKind::Text(t)) = st.doc.node(id).map(|n| &n.kind) else { panic!("type") };
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let q = vectorcraft_text::selection_quads(&lay, a, b)[0].map(|c| t.xf * c);
    let c = Point::new(q.iter().map(|p| p.x).sum::<f64>() / 4.0, q.iter().map(|p| p.y).sum::<f64>() / 4.0);
    screen(app, c.x, c.y)
}

/// With Auto Spell Check on, right-clicking an underlined word holds the word, its range and what the
/// dictionary suggests, which lead the menu, then Ignore All and Add to Dictionary; the type's
/// Spelling submenu has Check Spelling… as well.
#[test]
fn right_click_on_an_underlined_word_offers_its_suggestions() {
    let mut app = app();
    let id = spelled(&mut app, "the speling cat");
    let want: Vec<String> = app.session.execute("text.spelling.suggest", &json!({"word": "speling"})).unwrap()["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    assert!(!want.is_empty(), "the dictionary has suggestions for `speling`");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let p = word_at(&app, id, 4, 11);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    let w = app.ui.context_spelling.clone().expect("the word right-clicked");
    assert_eq!((w.id, w.start, w.end, w.word.as_str()), (id, 4, 11, "speling"));
    assert_eq!(w.suggestions, want, "the dictionary's suggestions, in order");
    assert!(has(&app, "Ignore All") && has(&app, "Add to Dictionary"), "{:?}", labels(&app));
    // Agents see them at the top of the menu, a suggestion by its word.
    let entries = menus::context_entries(&app);
    assert_eq!((entries[0].label.as_str(), entries[0].command.as_deref()), (want[0].as_str(), Some("text.editRange")));
    let check = entries.into_iter().find(|e| e.command.as_deref() == Some(crate::dialogs::check_spelling::COMMAND));
    assert_eq!(check.map(|e| e.path), Some(vec!["Spelling".to_string()]), "Spelling › Check Spelling…");
}

/// With Auto Spell Check off, or on a word that is right, a right-click holds no spelling word.
#[test]
fn no_spelling_items_off_a_misspelled_word_or_with_auto_spell_check_off() {
    let mut app = app();
    let id = spelled(&mut app, "the Vqzxtrwk");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let p = word_at(&app, id, 0, 3);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_none(), "a right word");
    escape(&mut app, &ctx);
    let p = word_at(&app, id, 4, 12);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_some(), "on, the wrong word is held");
    escape(&mut app, &ctx);
    app.run("edit.autoSpellCheck", json!({})).unwrap();
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_none(), "off");
}

/// A right word in type on top of an underlined word in other type: the right-click is on the
/// top type, so no spelling word is held for the word under it.
#[test]
fn a_word_under_other_type_is_not_the_one_right_clicked() {
    let mut app = app();
    let under = spelled(&mut app, "Qzvxtrwk");
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let p = word_at(&app, under, 0, 8);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_some(), "alone, the word is held");
    escape(&mut app, &ctx);
    app.run("text.create", json!({"x": 20, "y": 60, "text": "the cat sat on the mat", "size": 24})).unwrap();
    app.run("select.none", json!({})).unwrap();
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_none(), "under right type");
}

/// In an Intertwine group, the type drawn on top at the right-click is the one whose word is
/// held: a member brought to the front there, though earlier in the stacking order.
#[test]
fn in_an_intertwine_group_the_type_drawn_on_top_is_the_one_right_clicked() {
    let mut app = app();
    let under = spelled(&mut app, "Qzvxtrwk");
    let over =
        NodeId(app.run("text.create", json!({"x": 20, "y": 60, "text": "the cat sat on the mat", "size": 24})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [under.0, over.0]})).unwrap();
    let g = NodeId(app.run("object.intertwine.make", json!({})).unwrap()["id"].as_u64().unwrap());
    app.select_tool("selection");
    app.run("select.none", json!({})).unwrap();
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    // A point on the word where glyphs of both overlap: Intertwine's areas follow the outlines.
    let quad = {
        let st = app.session.active().unwrap();
        let Some(NodeKind::Text(t)) = st.doc.node(under).map(|n| &n.kind) else { panic!("type") };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        vectorcraft_text::selection_quads(&lay, 0, 8)[0].map(|c| t.xf * c)
    };
    let (x0, x1) = (quad.iter().map(|c| c.x).fold(f64::MAX, f64::min), quad.iter().map(|c| c.x).fold(f64::MIN, f64::max));
    let (y0, y1) = (quad.iter().map(|c| c.y).fold(f64::MAX, f64::min), quad.iter().map(|c| c.y).fold(f64::MIN, f64::max));
    let overlap = (0..40)
        .flat_map(|i| (0..20).map(move |j| Point::new(x0 + (x1 - x0) * (i as f64 + 0.5) / 40.0, y0 + (y1 - y0) * (j as f64 + 0.5) / 20.0)))
        .find(|q| app.run("object.intertwine.arrange", json!({"id": g.0, "at": [q.x, q.y], "order": "back", "member": under.0})).is_ok())
        .expect("glyphs of both overlap somewhere on the word");
    let p = screen(&app, overlap.x, overlap.y);
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_none(), "under the right type");
    escape(&mut app, &ctx);
    app.run("object.intertwine.arrange", json!({"id": g.0, "at": [overlap.x, overlap.y], "order": "front", "member": under.0})).unwrap();
    click(&mut app, &ctx, p, PointerButton::Secondary);
    assert!(app.ui.context_spelling.is_some(), "brought to the front there");
}
