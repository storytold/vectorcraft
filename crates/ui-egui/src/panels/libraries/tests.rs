use serde_json::json;
use vectorcraft_engine::Session;

use super::*;

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        show(app, ui);
        menu(app, ui);
    });
    out.textures_delta.clear();
}

/// The panel draws with no library, an empty one and one holding every kind of item (#745).
#[test]
fn the_panel_draws_its_libraries() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    frame(&mut app);
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    frame(&mut app);
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("library.add", json!({"kind": "graphic"})).unwrap();
    app.run("library.add", json!({"kind": "fillColor"})).unwrap();
    let t = app.run("text.create", json!({"x": 10, "y": 80, "text": "Type"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [t]})).unwrap();
    app.run("library.add", json!({"kind": "charStyle"})).unwrap();
    app.run("library.add", json!({"kind": "paraStyle"})).unwrap();
    frame(&mut app);
    let lib = app.session.libraries.get("Brand").unwrap();
    assert_eq!((lib.graphics.len(), lib.colors.len(), lib.char_styles.len(), lib.para_styles.len()), (1, 1, 1, 1));
    // The thumbnail decodes.
    let ctx = egui::Context::default();
    assert!(thumbnail(&ctx, "Brand", &lib.graphics[0], 256).is_some());
    // Shrunk to the size it is drawn at.
    let small = thumbnail(&ctx, "Brand", &lib.graphics[0], 40).unwrap();
    assert_eq!(small.size().into_iter().max(), Some(40), "{:?}", small.size());
}

/// The panel's text: every label it shows in one frame (`query` typed in the search field).
fn texts(app: &mut VectorcraftApp, query: &str) -> Vec<String> {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("library-search"), query.to_string()));
    let mut shapes = vec![];
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    shapes.append(&mut out.shapes);
    let mut texts = vec![];
    for s in shapes {
        if let egui::epaint::Shape::Text(t) = s.shape {
            texts.push(t.galley.text().to_string());
        }
    }
    texts
}

/// The search field filters items by name, ignoring case; groups show after the ungrouped
/// items, and a search nothing matches says so (#926).
#[test]
fn the_panel_searches_and_shows_groups() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    for (name, color) in [("Ocean Blue", "#0044aa"), ("Sun Yellow", "#ffcc00")] {
        app.run("library.add", json!({"kind": "fillColor", "color": color, "name": name})).unwrap();
    }
    let t = app.run("text.create", json!({"x": 10, "y": 80, "text": "Type"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [t]})).unwrap();
    app.run("library.add", json!({"kind": "charStyle", "name": "Headline"})).unwrap();
    app.run("library.createGroup", json!({"name": "Warm", "items": [{"kind": "fillColor", "item": "Sun Yellow"}]})).unwrap();
    let all = texts(&mut app, "");
    let at = |s: &str| all.iter().position(|t| t == s);
    assert!(at("Headline").is_some() && at("Warm").is_some(), "{all:?}");
    assert!(at("Headline") < at("Warm"), "groups come after the ungrouped items: {all:?}");
    // "BLUE" finds the blue swatch only: the style and the group without a match are hidden.
    let found = texts(&mut app, "BLUE");
    assert!(!found.iter().any(|t| t == "Headline" || t == "Warm"), "{found:?}");
    let found = texts(&mut app, "yellow");
    assert!(found.iter().any(|t| t == "Warm") && !found.iter().any(|t| t == "Headline"), "{found:?}");
    let none = texts(&mut app, "zzz");
    assert!(none.iter().any(|t| t == "No results"), "{none:?}");
}

/// A library imported from `styles` (character style names) and `groups` ([(name, its styles)])
/// → its id.
fn styled(app: &mut VectorcraftApp, styles: &[&str], groups: &[(&str, &[&str])]) -> String {
    let styles: Vec<Value> = styles.iter().enumerate().map(|(i, n)| json!({"name": n, "attrs": {"size": 8 + i}})).collect();
    let groups: Vec<Value> = groups
        .iter()
        .map(|(g, items)| json!({"name": g, "items": items.iter().map(|i| json!({"kind": "charStyle", "item": i})).collect::<Vec<_>>()}))
        .collect();
    let data = json!({"name": "Order", "charStyles": styles, "groups": groups}).to_string();
    app.run("library.import", json!({"data": data})).unwrap()["id"].as_str().unwrap().to_string()
}

/// One frame of the panel in a 260 × `tall` window with `events` → where each text it shows is
/// (its rect), and the panel's height.
fn frame_at(app: &mut VectorcraftApp, ctx: &egui::Context, tall: f32, events: Vec<egui::Event>) -> (Vec<(String, Rect)>, f32) {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(260.0, tall))), events, ..Default::default() };
    let mut height = 0.0;
    let mut out = ctx.run_ui(raw, |ui| {
        show(app, ui);
        height = ui.min_rect().height();
    });
    out.textures_delta.clear();
    let mut texts = vec![];
    for s in out.shapes {
        if let egui::epaint::Shape::Text(t) = s.shape {
            texts.push((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2())));
        }
    }
    (texts, height)
}

/// Where text `name` is in `texts`.
fn at(texts: &[(String, Rect)], name: &str) -> Rect {
    texts.iter().find(|(t, _)| t == name).map(|(_, r)| *r).unwrap_or_else(|| panic!("{name} isn't shown: {texts:?}"))
}

/// `drag` released at `pos`.
fn drop_at(app: &mut VectorcraftApp, ctx: &egui::Context, drag: PanelDrag, pos: egui::Pos2) {
    egui::DragAndDrop::set_payload(ctx, drag);
    let up = egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    frame_at(app, ctx, 600.0, vec![egui::Event::PointerMoved(pos), up]);
    frame_at(app, ctx, 600.0, vec![]);
}

fn style_order(app: &mut VectorcraftApp) -> Vec<String> {
    let got = app.run("library.get", json!({})).unwrap();
    got["charStyles"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect()
}

fn group_names(app: &mut VectorcraftApp) -> Vec<(String, Vec<String>)> {
    let got = app.run("library.get", json!({})).unwrap();
    let items = |g: &Value| g["items"].as_array().unwrap().iter().map(|i| i["item"].as_str().unwrap().to_string()).collect();
    got["groups"].as_array().unwrap().iter().map(|g| (g["name"].as_str().unwrap().to_string(), items(g))).collect()
}

/// Dragging in the list: a style dropped on the top or bottom half of another lands before or
/// after it, in its group; one dropped on a group's header goes into that group; a group's
/// header dropped on another group moves the group; and with every item grouped, a strip
/// takes an item out of its group.
#[test]
fn items_and_groups_move_by_dragging() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let lib = styled(&mut app, &["A", "B", "C", "D"], &[("Warm", &["D"]), ("Cool", &[])]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    let item = |name: &str| PanelDrag::LibraryItem { library: lib.clone(), kind: ItemKind::CharStyle, item: name.into() };
    // C onto A's top half: first.
    let a = at(&texts, "A");
    drop_at(&mut app, &ctx, item("C"), egui::pos2(a.center().x, a.top() - 4.0));
    assert_eq!(style_order(&mut app), ["C", "A", "B", "D"]);
    // A onto D's bottom half (in Warm): after D, in Warm.
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    let d = at(&texts, "D");
    drop_at(&mut app, &ctx, item("A"), egui::pos2(d.center().x, d.bottom() + 4.0));
    assert_eq!(style_order(&mut app), ["C", "B", "D", "A"]);
    assert_eq!(group_names(&mut app), [("Warm".into(), vec!["D".to_string(), "A".into()]), ("Cool".into(), vec![])]);
    // B onto Cool's header: into Cool, the order kept.
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    drop_at(&mut app, &ctx, item("B"), at(&texts, "Cool").center());
    assert_eq!(style_order(&mut app), ["C", "B", "D", "A"]);
    assert_eq!(group_names(&mut app)[1], ("Cool".into(), vec!["B".to_string()]));
    // Cool's header onto Warm's header (its top half): Cool first.
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    let warm = at(&texts, "Warm");
    drop_at(&mut app, &ctx, PanelDrag::LibraryGroup { library: lib.clone(), group: "Cool".into() }, egui::pos2(warm.center().x, warm.top()));
    assert_eq!(group_names(&mut app).iter().map(|(g, _)| g.as_str()).collect::<Vec<_>>(), ["Cool", "Warm"]);
    // Every item grouped: while one is dragged a strip takes it out of its group.
    app.run("library.moveItem", json!({"kind": "charStyle", "item": "C", "group": "Warm"})).unwrap();
    egui::DragAndDrop::set_payload(&ctx, item("C"));
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    let strip = at(&texts, "Drop here to take it out of its group");
    drop_at(&mut app, &ctx, item("C"), strip.center());
    assert!(group_names(&mut app).iter().all(|(_, items)| !items.contains(&"C".to_string())), "{:?}", group_names(&mut app));
    // Without a drag there is no strip.
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    assert!(!texts.iter().any(|(t, _)| t == "Drop here to take it out of its group"));
}

/// The list toggle beside the picker shows the colours and graphics as rows with their names
/// (a colour's hex value too), the icon toggle as chips and tiles again; in the list an item
/// dropped on another's top half lands before it.
#[test]
fn the_list_view_shows_names_and_takes_drops_by_row() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    for (name, color) in [("Ocean Blue", "#0044aa"), ("Sun Yellow", "#ffcc00")] {
        app.run("library.add", json!({"kind": "fillColor", "color": color, "name": name})).unwrap();
    }
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("library.add", json!({"kind": "graphic", "name": "Badge"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let shown = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t.eq_ignore_ascii_case(s));
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    assert!(!shown(&texts, "Ocean Blue") && !shown(&texts, "Badge"), "icons by default: {texts:?}");
    // The list toggle: the first of the two buttons at the right of the picker's row.
    let row = at(&texts, "Brand");
    let list_button = egui::pos2(260.0 - 8.0 - 24.0 - ctx.global_style().spacing.item_spacing.x - 12.0, row.center().y);
    let click =
        |pressed| egui::Event::PointerButton { pos: list_button, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_at(&mut app, &ctx, 600.0, vec![egui::Event::PointerMoved(list_button), click(true)]);
    frame_at(&mut app, &ctx, 600.0, vec![click(false)]);
    assert!(crate::panels::pstate::<bool>(&ctx, LIST_VIEW), "the list toggle was clicked");
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    for s in ["Ocean Blue", "#0044aa", "Sun Yellow", "#ffcc00", "Badge"] {
        assert!(shown(&texts, s), "{s} in the list: {texts:?}");
    }
    // Sun Yellow onto Ocean Blue's top half: first.
    let ocean = at(&texts, "Ocean Blue");
    let drag = PanelDrag::Paint {
        paint: Paint::solid(vectorcraft_color::Color::from_hex("#ffcc00").unwrap()),
        params: json!({}),
        rows: None,
        library: Some(("Brand".into(), "Sun Yellow".into())),
    };
    drop_at(&mut app, &ctx, drag, egui::pos2(ocean.center().x, ocean.top() - 3.0));
    let got = app.run("library.get", json!({})).unwrap();
    let names: Vec<&str> = got["colors"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Sun Yellow", "Ocean Blue"]);
    // Back to icons: no names.
    crate::panels::set_pstate(&ctx, LIST_VIEW, false);
    let (texts, _) = frame_at(&mut app, &ctx, 600.0, vec![]);
    assert!(!shown(&texts, "Ocean Blue"), "{texts:?}");
}

/// A long list scrolls inside the panel (its bottom bar stays in the window), and a drag held
/// near the list's bottom edge scrolls it.
#[test]
fn a_long_list_scrolls_also_while_dragging() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let names: Vec<String> = (0..60).map(|i| format!("Style {i}")).collect();
    let lib = styled(&mut app, &names.iter().map(String::as_str).collect::<Vec<_>>(), &[]);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (texts, height) = frame_at(&mut app, &ctx, 400.0, vec![]);
    assert!(height <= 400.0, "the panel fits the window: {height}");
    let first = at(&texts, "Style 0");
    // The list's bottom edge: the last text drawn before the window's bottom bar area.
    let bottom = texts.iter().filter(|(t, _)| t.starts_with("Style ")).map(|(_, r)| r.bottom()).fold(0.0, f32::max).min(400.0 - BAR - 4.0);
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::LibraryItem { library: lib, kind: ItemKind::CharStyle, item: "Style 0".into() });
    let hold = egui::pos2(first.center().x, bottom - 4.0);
    for _ in 0..5 {
        frame_at(&mut app, &ctx, 400.0, vec![egui::Event::PointerMoved(hold)]);
    }
    let (texts, _) = frame_at(&mut app, &ctx, 400.0, vec![egui::Event::PointerMoved(hold)]);
    let moved = texts.iter().find(|(t, _)| t == "Style 0").map_or(f32::NEG_INFINITY, |(_, r)| r.top());
    assert!(moved < first.top(), "the list scrolled down: {} → {moved}", first.top());
}

/// New Group… and Rename Group… ask for a name and act on the library and group they came
/// from; the dialog shows only the name.
#[test]
fn group_dialogs_run_with_their_library_and_group() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let lib = app.run("library.create", json!({"name": "Brand"})).unwrap()["id"].as_str().unwrap().to_string();
    app.run("library.add", json!({"kind": "fillColor", "color": "#0044aa", "name": "Blue"})).unwrap();
    ask_group_name(&mut app, &lib, None, json!([{"kind": "fillColor", "item": "Blue"}]));
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Cool"));
    crate::dialogs::confirm(&mut app).unwrap();
    ask_group_name(&mut app, &lib, Some("Cool"), Value::Null);
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Cold"));
    crate::dialogs::confirm(&mut app).unwrap();
    let got = app.run("library.get", json!({})).unwrap();
    assert_eq!(got["groups"], json!([{"name": "Cold", "items": [{"kind": "color", "item": "Blue"}]}]));
}

/// Export Library… writes the library's file and Import Library… adds it back as a new library.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn libraries_are_exported_and_imported_from_the_panel_menu() {
    let dir = vectorcraft_testkit::temp_dir("libraries-panel");
    let path = dir.join("Brand.vclibrary").to_string_lossy().to_string();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    app.run("library.add", json!({"kind": "fillColor", "color": "#0044aa", "name": "Blue"})).unwrap();
    let picked = path.clone();
    app.services.pick_save = Some(Box::new(move |pick: &crate::FilePick| {
        assert_eq!(pick.name, "Brand.vclibrary", "the library's name suggested");
        Some(picked.clone())
    }));
    app.services.write = Some(Box::new(|p: &str, bytes: &[u8]| std::fs::write(p, bytes).map_err(|e| e.to_string())));
    export_picked(&mut app, "Brand", "Brand");
    assert!(std::path::Path::new(&path).exists());
    let picked = path.clone();
    app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
    import_picked(&mut app);
    let list = app.run("library.list", json!({})).unwrap();
    let names: Vec<&str> = list["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Brand", "Brand (2)"]);
    assert_eq!(app.ui.status, "Imported library Brand (2)");
}
