use craft::menu::Node;

use super::*;
use crate::i18n::LangExt;

/// The nodes of the top-level menu `title`.
fn menu_nodes(title: &str) -> Vec<Node> {
    menu_tree().into_iter().find(|(name, _)| *name == title).map(|(_, children)| children).unwrap_or_default()
}

/// The children of the submenu `label` among `nodes`.
fn submenu_nodes<'a>(nodes: &'a [Node], label: &str) -> Option<&'a [Node]> {
    nodes.iter().find_map(|n| match n {
        Node::Submenu { label: l, children, .. } if l == label => Some(children.as_slice()),
        _ => None,
    })
}

/// `(label, command id, params)` of the command items directly among `nodes`.
fn command_rows(nodes: &[Node]) -> Vec<(&str, &str, &Value)> {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Item(it) => Some((it.label.as_str(), it.command.id.as_str(), &it.command.params)),
            _ => None,
        })
        .collect()
}

/// The command ids of the items labelled `label` directly among `nodes`.
fn ids_labelled(nodes: &[Node], label: &str) -> Vec<String> {
    command_rows(nodes).into_iter().filter(|(l, ..)| *l == label).map(|(_, id, _)| id.to_string()).collect()
}

/// The positions of the separators among `nodes`.
fn separators(nodes: &[Node]) -> Vec<usize> {
    nodes.iter().enumerate().filter(|(_, n)| matches!(n, Node::Separator)).map(|(i, _)| i).collect()
}

/// The label the built bar shows for `id`, with the step-naming wording (Undo Move) applied.
fn shown_label(app: &mut VectorcraftApp, id: &str) -> Option<String> {
    let ctx = crate::testing::ui_context();
    crate::menus::model::refresh(app, &ctx);
    // The bar keeps an item's own label; the row shows its state's title, which the model makes so.
    let app = &*app;
    app.menus.bar.items().into_iter().find(|it| it.command.id == id).map(|it| crate::menus::shown_label(app, id, &it.command.params, &it.label))
}

/// One frame of the whole app on `ctx`, with `events` as the host's input.
fn app_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw =
        egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0))), ..Default::default() };
    crate::testing::app_frame(app, &ctx, raw);
}

/// The first open dialog and its fields.
fn open_dialog(app: &VectorcraftApp) -> (craft::ui::dialog::OpenDialog, serde_json::Map<String, Value>) {
    let dialog = crate::tests::picks::open_dialogs(app).into_iter().next().expect("a dialog is open");
    let fields = match &dialog.fields {
        Some(Value::Object(fields)) => fields.clone(),
        _ => serde_json::Map::new(),
    };
    (dialog, fields)
}

/// A text field of an open dialog's fields.
fn text_of(fields: &serde_json::Map<String, Value>, key: &str) -> String {
    fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Sets `fields` on the open dialog, then confirms it (`ok`) or cancels it.
fn answer_open(app: &mut VectorcraftApp, fields: Value, ok: bool) {
    let (dialog, _) = open_dialog(app);
    let fields: Vec<(&str, Value)> = fields.as_object().into_iter().flatten().map(|(k, v)| (k.as_str(), v.clone())).collect();
    crate::tests::picks::answer_dialog(app, dialog.id, &fields, ok);
}

#[test]
fn help_f1_opens_discover_without_a_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let help = menu_nodes("Help");
    assert!(command_rows(&help).iter().any(|(label, id, _)| (*label, *id) == ("VectorCraft Help…", "help.discover")));
    assert!(enabled(&app, "help.discover"));
    let ctx = crate::testing::ui_context();
    for _ in 0..3 {
        app_frame(&mut app, &ctx, vec![]);
    }
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    app_frame(&mut app, &ctx, vec![key(egui::Key::F1)]);
    assert!(crate::panel_docking::showing(&app.ui, "discover"));
    assert!(app.session.active().is_none());
    app.run("help.discover", json!({})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "discover"));

    let tools = app.run("help.discover.search", json!({"query": "rectangle"})).unwrap();
    let tool = tools["results"].as_array().unwrap().iter().position(|hit| hit["kind"] == "tool" && hit["id"] == "rectangle").unwrap();
    let active_tool = app.session.tool_id();
    let picked = app.run("help.discover.choose", json!({"index": tool})).unwrap();
    assert_eq!(picked["result"]["id"], "rectangle");
    assert!(app.ui.learn_locate.is_some());
    assert_eq!(app.session.tool_id(), active_tool);
    for _ in 0..2 {
        app_frame(&mut app, &ctx, vec![]);
    }
    app_frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(!crate::panel_docking::showing(&app.ui, "discover"));
    app.run("help.discover", json!({})).unwrap();
    app.run("help.discover.back", json!({})).unwrap();
    assert_eq!(app.ui.discover.query, "rectangle");
    assert!(app.ui.discover.selected.is_none());
    // PLAN: assumed `shortcuts.set` takes the tool's binding key `tool:rectangle` as its id
    app.run("shortcuts.set", json!({"id": "tool:rectangle", "shortcut": "Shift+9"})).unwrap();
    let tools = app.run("help.discover.search", json!({"query": "rectangle"})).unwrap();
    assert!(tools["results"].as_array().unwrap().iter().any(|hit| hit["kind"] == "tool" && hit["id"] == "rectangle" && hit["shortcut"] == "Shift+9"));

    let commands = app.run("help.discover.search", json!({"query": "file.newDialog"})).unwrap();
    let command = commands["results"].as_array().unwrap().iter().position(|hit| hit["id"] == "file.newDialog").unwrap();
    app.run("help.discover.choose", json!({"index": command})).unwrap();
    assert!(!app.any_dialog_open());
    assert!(app.session.active().is_none());

    let tutorial = crate::panels::learn::catalog::SKILLS[0].tutorials.first().unwrap();
    let found = app.run("help.discover.search", json!({"query": tutorial.title})).unwrap();
    let index = found["results"].as_array().unwrap().iter().position(|hit| hit["kind"] == "tutorial" && hit["id"] == tutorial.id).unwrap();
    app.run("help.discover.choose", json!({"index": index})).unwrap();
    assert!(app.session.active().is_none());
    let text = crate::tests_labels::painted_text(&mut app, crate::panels::discover::show);
    assert!(text.contains(tutorial.title), "{text}");
    app.ui.discover.focus = true;
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(!crate::panel_docking::showing(&app.ui, "discover"));

    app.run("file.new", json!({})).unwrap();
    app.run("help.discover", json!({})).unwrap();
    app.select_tool(vectorcraft_tools::addons::polygonal_lasso::ID);
    let view = app.view_info();
    for (x, y) in [(40.0, 40.0), (80.0, 40.0)] {
        let down = vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Down, x, y);
        let up = vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Up, x, y);
        app.session.pointer(&down, view).unwrap();
        app.session.pointer(&up, view).unwrap();
    }
    assert!(app.session.tool_ends_on_escape(view));
    let canvas_ctx = crate::testing::ui_context();
    app_frame(&mut app, &canvas_ctx, vec![key(egui::Key::Escape)]);
    assert!(!app.session.tool_ends_on_escape(view));
    assert!(crate::panel_docking::showing(&app.ui, "discover"));
}

/// With the toolbars hidden by Tab, choosing a tool in Help, or a tool's link in a tutorial Help
/// shows, marks nothing and the status bar says why.
#[test]
fn discover_says_why_a_tool_cannot_be_marked() {
    fn pass(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| crate::panels::discover::show(app, ui));
        out.textures_delta.clear();
        out
    }
    fn texts(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        let out = pass(ctx, app, events);
        fn collect(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut texts = vec![];
        for c in &out.shapes {
            collect(&c.shape, &mut texts);
        }
        texts
    }
    fn click(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) {
        texts(ctx, app, vec![]);
        let drawn = texts(ctx, app, vec![]);
        let pos = drawn.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("`{label}` not drawn: {drawn:?}")).1.center();
        press_at(ctx, app, pos);
    }
    fn press_at(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
            pass(ctx, app, vec![e]);
        }
    }
    // A step's tool name is a link inside its sentence: the Link node named by its words.
    fn click_link(ctx: &egui::Context, app: &mut VectorcraftApp, name: &str) {
        ctx.enable_accesskit();
        pass(ctx, app, vec![]);
        let nodes = pass(ctx, app, vec![]).platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        let link = nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::Link && n.label().is_some_and(|l| l == name));
        let b = link.and_then(|n| n.bounds()).unwrap_or_else(|| panic!("no link `{name}`"));
        press_at(ctx, app, egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    // Tab hides the toolbars in the shell's next pass.
    app.run("window.dock", json!({})).unwrap();
    app_frame(&mut app, &ctx, vec![]);
    assert!(craft::ui::shell::chrome_state(&ctx, egui::ViewportId::ROOT).hidden);
    let reason = app.run("learn.locateTool", json!({"tool": "rectangle"})).unwrap()["reason"].as_str().unwrap().to_string();

    let found = app.run("help.discover.search", json!({"query": "rectangle"})).unwrap();
    let hit = found["results"].as_array().unwrap().iter().find(|hit| hit["kind"] == "tool" && hit["id"] == "rectangle").unwrap().clone();
    app.status("");
    click(&ctx, &mut app, hit["label"].as_str().unwrap());
    assert_eq!(app.ui.discover.selected.map(|i| app.ui.discover.results[i].id.as_str()), Some("rectangle"));
    assert!(app.ui.learn_locate.is_none());
    assert_eq!(app.ui.status, reason);

    let (tutorial, link) = crate::panels::learn::catalog::SKILLS
        .iter()
        .flat_map(|skill| skill.tutorials.iter())
        .find_map(|t| t.steps.iter().find_map(|s| s.tool_name()).map(|(_, name, _)| (t, name)))
        .unwrap();
    let found = app.run("help.discover.search", json!({"query": tutorial.title})).unwrap();
    let index = found["results"].as_array().unwrap().iter().position(|hit| hit["kind"] == "tutorial" && hit["id"] == tutorial.id).unwrap();
    app.run("help.discover.choose", json!({"index": index})).unwrap();
    app.status("");
    click_link(&ctx, &mut app, link);
    assert!(app.ui.learn_locate.is_none());
    assert_eq!(app.ui.status, reason);
}

#[test]
fn discover_rejects_invalid_search_and_selection_parameters() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("help.discover.search", json!({"query": "rectangle"})).unwrap();
    let before = app.ui.discover.query.clone();
    for params in
        [json!({}), json!({"query": null}), json!({"query": false}), json!({"query": "rectangle", "unknown": 1}), json!({"query": "a".repeat(1025)})]
    {
        assert!(app.run("help.discover.search", params).is_err());
        assert_eq!(app.ui.discover.query, before);
    }
    for index in [json!(-1), json!(1.5), json!(1000000), json!("0"), Value::Null] {
        assert!(app.run("help.discover.choose", json!({"index": index})).is_err());
    }
    for params in [json!(5), json!("rectangle"), json!([1]), json!(true)] {
        for id in ["help.discover", "help.discover.search", "help.discover.choose", "help.discover.back"] {
            assert!(app.run(id, params.clone()).is_err(), "{id} {params}");
        }
        assert_eq!(app.ui.discover.query, before);
    }
    app.run("help.discover.search", json!({"query": "   "})).unwrap();
    assert!(app.ui.discover.results.is_empty());
    assert!(app.run("help.discover.choose", json!({"index": 0})).is_err());
}

/// A kept command result follows its menu item: Show Grid reads Hide Grid once the grid shows.
#[test]
fn discover_command_results_follow_their_menu_label() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    // Help searches in the window's interface language.
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    app.run("file.new", json!({})).unwrap();
    let found = app.run("help.discover.search", json!({"query": "view.grid"})).unwrap();
    let index = found["results"].as_array().unwrap().iter().position(|hit| hit["id"] == "view.grid" && hit["path"][0] == "View").unwrap();
    assert_eq!(found["results"][index]["label"], "Show Grid");
    app.run("help.discover.choose", json!({"index": index})).unwrap();
    app.run("view.grid", json!({})).unwrap();
    let back = app.run("help.discover", json!({})).unwrap();
    assert_eq!(back["selected"], index);
    assert_eq!(back["results"][index]["label"], "Hide Grid");
}

/// A menu entry's shown path translates the menus' own names and keeps a Scripts folder's name, as
/// the menus draw it; Help shows that path.
#[test]
fn menu_entry_shown_paths_keep_script_folder_names() {
    let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let de = crate::i18n::Lang::from_code("de").unwrap();
    assert_ne!(crate::i18n::tr(de, "File"), "File");
    let script = json!({"path": "File/a.jsx"});
    // PLAN: assumed `cp(label, id, params)` and `sub(label, children)` build `Node`s
    let folder = vec![cp("a", crate::scripts_menu::COMMAND, script)];
    let items = vec![sub("Scripts", vec![sub("File", folder), cp("Other Script…", "ui.runScript", Value::Null)])];
    let mut out = vec![];
    crate::i18n::set_current(de);
    flatten(&app, vec!["File".into()], vec![crate::i18n::t("File").into()], &items, &mut out);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let script = out.iter().find(|e| e.command.as_deref() == Some(crate::scripts_menu::COMMAND)).unwrap();
    assert_eq!(script.path, ["File", "Scripts", "File"]);
    assert_eq!(script.shown_path, [crate::i18n::tr(de, "File"), crate::i18n::tr(de, "Scripts"), "File"]);
}

/// Outside macOS the window's menu bar has no VectorCraft menu, so Help lists no command under it.
#[test]
fn discover_leaves_out_menus_the_window_does_not_show() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let app_menu: Vec<String> =
        menu_entries(&app).into_iter().filter(|e| e.path.first().is_some_and(|t| t == "VectorCraft")).filter_map(|e| e.command).collect();
    assert!(!app_menu.is_empty());
    for id in app_menu {
        let found = app.run("help.discover.search", json!({"query": id})).unwrap();
        let listed = found["results"].as_array().unwrap().iter().any(|hit| hit["path"][0] == "VectorCraft");
        assert_eq!(listed, cfg!(target_os = "macos"), "{id}: {found}");
    }
}

/// Help finds a command by the label its menu shows in the UI language, also where the menu names
/// an effect by its own id (Effect › Pathfinder › Add reads Réunion in French).
#[test]
fn discover_finds_commands_by_their_shown_menu_label() {
    // Help searches in the window's interface language.
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    let shown = crate::i18n::tr_id(fr, "pathfinder.add", "Add");
    assert_ne!(shown, crate::i18n::tr(fr, "Add"));
    crate::i18n::set_current(fr);
    let found = app.run("help.discover.search", json!({"query": shown}));
    let effect = crate::i18n::tr(fr, "Effect").to_string();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let found = found.unwrap();
    let hits = found["results"].as_array().unwrap();
    assert!(
        hits.iter().any(|hit| hit["kind"] == "command"
            && hit["id"].as_str().is_some_and(|id| id.starts_with("effect."))
            && hit["label"] == shown
            && hit["path"][0] == effect),
        "{found}"
    );
}

#[test]
fn discover_results_show_the_current_shortcut_and_language_after_a_change() {
    // Help searches in the window's interface language.
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    // The keymap is the window's: its frame builds it, and builds it again with the shortcuts as they changed.
    let ctx = app.ui_ctx.clone().unwrap();
    app_frame(&mut app, &ctx, vec![]);
    let found = app.run("help.discover.search", json!({"query": "file.newDialog"})).unwrap();
    let index = found["results"].as_array().unwrap().iter().position(|hit| hit["id"] == "file.newDialog").unwrap();
    assert_eq!(found["results"][index]["shortcut"], "Cmd+N");
    app.run("help.discover.choose", json!({"index": index})).unwrap();
    app.run("window.panel.close", json!({"panel": "discover"})).unwrap();
    app.run("shortcuts.set", json!({"id": "file.newDialog", "shortcut": "Cmd+Alt+Shift+F9"})).unwrap();
    app_frame(&mut app, &ctx, vec![]);

    let reopened = app.run("help.discover", json!({})).unwrap();
    assert_eq!(reopened["query"], "file.newDialog");
    assert_eq!(reopened["selected"], index);
    assert_eq!(reopened["results"][index]["id"], "file.newDialog");
    assert_eq!(reopened["results"][index]["shortcut"], "Cmd+Alt+Shift+F9");
    let picked = app.run("help.discover.choose", json!({"index": index})).unwrap();
    assert_eq!(picked["result"]["shortcut"], "Cmd+Alt+Shift+F9");
    assert_eq!(picked["location"]["shortcut"], "Cmd+Alt+Shift+F9");

    app.run("shortcuts.set", json!({"id": "file.newDialog", "shortcut": "Cmd+Alt+Shift+F8"})).unwrap();
    app_frame(&mut app, &ctx, vec![]);
    let text = crate::tests_labels::painted_text(&mut app, crate::panels::discover::show);
    // PLAN: assumed `keymap::chord_text(&str) -> String`, the text the chord shows as
    assert!(text.contains(&crate::keymap::chord_text("Cmd+Alt+Shift+F8")), "{text}");
    assert!(!text.contains(&crate::keymap::chord_text("Cmd+Alt+Shift+F9")), "{text}");

    let de = crate::i18n::Lang::from_code("de").unwrap();
    assert_ne!(crate::i18n::tr(de, "New…"), "New…");
    crate::i18n::set_current(de);
    let back = app.run("help.discover.back", json!({})).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(back["query"], "file.newDialog");
    assert_eq!(back["results"][index]["label"], crate::i18n::tr(de, "New…"));
    assert_eq!(back["results"][index]["path"][0], crate::i18n::tr(de, "File"));
}

#[test]
fn help_bar_toggle_is_independent_and_saved_in_workspaces() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.ui.help_bar);
    assert_eq!(checked(&app, "window.helpBar", &Value::Null), Some(true));
    assert_eq!(run_ui_command(&mut app, "window.helpBar", &Value::Null).unwrap().unwrap(), json!(false));
    assert!(app.ui.status_bar);
    assert_eq!(checked(&app, "window.helpBar", &Value::Null), Some(false));
    let saved = crate::workspaces::record(&app.ui);
    let saved: crate::workspaces::WorkspaceRecord = serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
    assert!(!saved.help_bar, "the record keeps the Help Bar off");
    // A saved workspace brings its bars back after another workspace changed them.
    crate::workspaces::save_as(&mut app.ui, "Bars").unwrap();
    let other = crate::workspaces::names(&app.ui).into_iter().find(|n| n != "Bars").unwrap();
    crate::workspaces::switch(&mut app, &other).unwrap();
    app.ui.help_bar = true;
    app.ui.status_bar = false;
    crate::workspaces::switch(&mut app, "Bars").unwrap();
    assert!(!app.ui.help_bar && app.ui.status_bar);
    assert_eq!(run_ui_command(&mut app, "window.helpBar", &Value::Null).unwrap().unwrap(), json!(true));
    let legacy: crate::workspaces::WorkspaceRecord = serde_json::from_value(json!({"name":"Old workspace"})).unwrap();
    assert!(legacy.help_bar);
    let restored: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(restored.help_bar);
    let window = menu_nodes("Window");
    assert!(command_rows(&window).iter().any(|(label, id, _)| (*label, *id) == ("Help Bar", "window.helpBar")));
}

/// Help starts with Search Commands…, the command search's own command, showing its shortcut,
/// then a separator; the menus list it once.
#[test]
fn help_starts_with_search_commands() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let tree = menu_tree();
    let help = &tree.iter().find(|(title, _)| *title == "Help").expect("Help menu").1;
    assert!(
        matches!(&help[0], Node::Item(it) if (it.label.as_str(), it.command.id.as_str()) == ("Search Commands…", "help.commandPalette")),
        "{:?}",
        help[0]
    );
    assert!(matches!(help[1], Node::Separator), "{:?}", help[1]);
    assert!(enabled(&app, "help.commandPalette"));
    crate::tests_synthetic::key_frame(&mut app, &crate::testing::ui_context(), vec![]);
    assert!(shortcut_of(&app, "help.commandPalette").is_some(), "the search's shortcut shows next to the item");
    fn count(nodes: &[Node]) -> usize {
        nodes
            .iter()
            .map(|n| match n {
                Node::Item(it) if it.command.id == "help.commandPalette" => 1,
                Node::Submenu { children, .. } => count(children),
                _ => 0,
            })
            .sum()
    }
    assert_eq!(tree.iter().map(|(_, children)| count(children)).sum::<usize>(), 1);
}

/// No application menu in the window (as in Illustrator): the bar starts at File.
#[test]
fn the_bar_starts_at_file() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = crate::testing::ui_context();
    crate::menus::model::refresh(&mut app, &ctx);
    // PLAN: assumed `app.menus.laid` is the `craft::menu::MenuBar` laid out for the window
    let titles: Vec<&str> = app.menus.laid.menus.iter().map(|menu| menu.title.as_str()).collect();
    assert_eq!(titles, ["File", "Edit", "Object", "Type", "Select", "Effect", "View", "Window", "Help"]);
}

#[test]
fn names_in_menus_are_not_interface_labels() {
    for (id, label) in [
        ("file.openRecent1", "Layers.svg"),
        ("view.goto2", "Layers"),
        ("type.recentFont1", "Regular"),
        ("text.setStyle", "Black"),
        ("window.workspace", "Layers"),
        ("plugin.dialog", "Group"),
        ("window.userSwatchLibrary3", "Default"),
    ] {
        assert!(shows_a_name(id, label), "{id} {label}");
    }
    for (id, label) in [("object.group", "Group"), ("window.workspace", "Essentials"), ("effect.apply", "Drop Shadow")] {
        assert!(!shows_a_name(id, label), "{id} {label}");
    }
}

#[test]
fn last_effect_dialog_and_view_toggles() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    assert!(!enabled(&app, "effect.last"));
    app.last_effect = Some(("distort.roughen".into(), json!({"size": 9})));
    assert!(enabled(&app, "effect.last"));
    assert_eq!(dynamic_label(&app, "effect.last", "Last Effect…"), "Last Effect: Roughen…");
    app.run("effect.last", json!({})).unwrap();
    // PLAN: the effect dialog's `__effect` and `size` fields need the effect dialog's own typed reader (effects set)
    assert!(app.any_dialog_open(), "Last Effect… opens the effect's dialog");
    for (id, on, off) in [
        ("view.textThreads", "Hide Text Threads", "Show Text Threads"),
        ("view.gradientAnnotator", "Hide Gradient Annotator", "Show Gradient Annotator"),
        ("view.livePaintGaps", "Show Live Paint Gaps", "Hide Live Paint Gaps"),
        ("type.hiddenCharacters", "Show Hidden Characters", "Hide Hidden Characters"),
        ("view.template", "Hide Template", "Show Template"),
    ] {
        assert_eq!(dynamic_label(&app, id, ""), on);
        app.run(id, json!({})).unwrap();
        assert_eq!(dynamic_label(&app, id, ""), off);
    }
}

/// Edit › Undo and Edit › Redo name the step in the UI language as the History panel lists it
/// (`panels::history::step_name`), including a step named after an effect whose catalog holds
/// only the effect's menu row with an ellipsis, as Japanese does for Feather.
#[test]
fn undo_and_redo_name_the_step_as_the_history_panel_does() {
    let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
    crate::i18n::set_current(ja);
    let mut app = app_with_a_rectangle();
    app.run("effect.apply", json!({"effect": "stylize.feather"})).unwrap();
    assert_eq!(app.session.active().unwrap().history.undo.last().map(|h| h.label.as_str()), Some("Feather"));
    let shown = |app: &mut VectorcraftApp, id: &'static str| shown_label(app, id).unwrap_or_else(|| panic!("{id} is not a menu item"));
    let step = |template: &str, name: &str| {
        let name = crate::panels::history::step_name(ja, name).unwrap_or_else(|| name.to_string());
        crate::i18n::fmt(crate::i18n::tr(ja, template), &[("name", &name)])
    };
    assert_eq!(shown(&mut app, "edit.undo"), step("Undo {name}", "Feather"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(shown(&mut app, "edit.redo"), step("Redo {name}", "Feather"));
    assert_eq!(shown(&mut app, "edit.undo"), step("Undo {name}", "Rectangle"), "a step with a plain row");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(shown(&mut app, "edit.redo"), "Redo Feather");
}

/// Type › Optical Margin Alignment runs `type.opticalMarginAlignment`, needs a selection, and
/// is checked while the selected type has it on. Its check, and Type Orientation's, read the
/// type inside a selected group too.
#[test]
fn optical_margin_alignment_is_a_checked_type_menu_item() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let ids = ids_labelled(&menu_nodes("Type"), "Optical Margin Alignment");
    assert_eq!(ids, ["type.opticalMarginAlignment"]);
    let id = "type.opticalMarginAlignment";
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    assert!(!enabled(&app, id));
    let text = app.run("text.create", json!({"x": 10, "y": 10, "text": "“Hi”"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    assert!(enabled(&app, id));
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    // With the type in a selected group, the checks show the type the commands change.
    app.run("object.group", json!({})).unwrap();
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
    app.run("type.orientation.vertical", json!({})).unwrap();
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(true));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
}

/// Type › Dynamic Text runs `type.dynamicText`, is enabled for point type and for type on a path
/// and area type that are in no thread, and is checked while the selected type has it on.
#[test]
fn dynamic_text_is_a_checked_type_menu_item() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let ids = ids_labelled(&menu_nodes("Type"), "Dynamic Text");
    assert_eq!(ids, ["type.dynamicText"]);
    let id = "type.dynamicText";
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    assert!(!enabled(&app, id));
    let point = app.run("text.create", json!({"x": 10, "y": 10, "text": "Point"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [point]})).unwrap();
    assert!(enabled(&app, id), "point type, which it turns into area type with Block on");
    assert_eq!(checked(&app, id, &Value::Null), Some(false), "point type");
    let path = app.run("path.create", json!({"d": "M60 150 L340 150"})).unwrap()["id"].clone();
    let t = app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [60, 150]})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [t]})).unwrap();
    assert!(enabled(&app, id));
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    let frame = app.run("shape.rectangle", json!({"x": 60, "y": 200, "width": 200, "height": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [t, frame]})).unwrap();
    app.run("text.thread.create", json!({})).unwrap();
    app.run("select.set", json!({"ids": [t]})).unwrap();
    assert!(!enabled(&app, id), "type on a path in a thread");
    // Area type takes Block, and the item is checked while it is on.
    let area = app.run("text.create", json!({"x": 10, "y": 400, "text": "Block", "area": {"width": 100, "height": 50}})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [area]})).unwrap();
    assert!(enabled(&app, id), "area type");
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
    // Point type first: unchecked, as the item turns it on.
    app.run("select.set", json!({"ids": [point, area]})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(false), "point type first");
    app.run(id, json!({})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true), "both on");
}

/// Type › Type on a Path › Center on Top and Center on Bottom run `type.pathCenter.top` and
/// `.bottom`, which the command palette also runs without params, and are enabled for type on a
/// closed path only.
#[test]
fn center_on_top_and_bottom_are_type_on_a_path_items() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let type_menu = menu_nodes("Type");
    let items: Vec<(&str, &str)> = command_rows(submenu_nodes(&type_menu, "Type on a Path").unwrap_or_default())
        .into_iter()
        .filter(|(_, id, params)| id.starts_with("type.pathCenter") && params.is_null())
        .map(|(label, id, _)| (label, id))
        .collect();
    let ids = ["type.pathCenter.top", "type.pathCenter.bottom"];
    assert_eq!(items, [("Center on Top", ids[0]), ("Center on Bottom", ids[1])]);
    let palette = crate::palette::items();
    for (label, id) in items {
        assert!(palette.iter().any(|(l, i)| l == &format!("Type › Type on a Path › {label}") && i == id), "{label}");
    }
    let line = app.run("path.create", json!({"d": "M60 150 L340 150"})).unwrap()["id"].clone();
    let open = app.run("text.createInPath", json!({"path": line, "mode": "onPath", "text": "Along", "at": [60, 150]})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [open]})).unwrap();
    assert!(ids.iter().all(|id| !enabled(&app, id)), "type on an open path");
    let square = app.run("path.create", json!({"d": "M60 200 L160 200 L160 300 L60 300 Z"})).unwrap()["id"].clone();
    let closed = app.run("text.createInPath", json!({"path": square, "mode": "onPath", "text": "Round", "at": [60, 200]})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [closed]})).unwrap();
    assert!(ids.iter().all(|id| enabled(&app, id)));
    app.run("select.set", json!({"ids": [closed, open]})).unwrap();
    assert!(ids.iter().all(|id| !enabled(&app, id)), "type on a closed path and on an open one");
    // A palette pick runs the command with no params.
    app.run("select.set", json!({"ids": [closed]})).unwrap();
    use craft::ui::runtime::commands::{FirePhase, FireSource, Fired};
    let fired = Fired::new(craft::menu::CommandRef::new(ids[0]), FireSource::Palette, FirePhase::Press, 0.0);
    crate::shortcuts::run_fired(&mut app, &crate::testing::ui_context(), &fired);
    let doc = &app.session.active().unwrap().doc;
    let start = match &doc.node(vectorcraft_doc::NodeId(closed.as_u64().unwrap())).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => match t.kind {
            vectorcraft_doc::TextKind::OnPath { start, .. } => start,
            ref k => panic!("{k:?}"),
        },
        k => panic!("{k:?}"),
    };
    // Once round, the middle of the top edge an eighth of the way round.
    assert!((start - 0.625).abs() < 1e-6, "{start}");
}

/// Type › Size ends with a separator and Other…, which runs `type.highlightFontSize`: dimmed
/// without selected type, enabled with it, and never checked.
#[test]
fn type_size_ends_with_other() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let id = "type.highlightFontSize";
    assert!(!enabled(&app, id), "no document");
    app.run("file.new", json!({})).unwrap();
    let type_menu = menu_nodes("Type");
    let items = submenu_nodes(&type_menu, "Size").unwrap_or_default();
    let n = items.len();
    assert!(n > 2, "{n} items");
    assert!(matches!(items[n - 2], Node::Separator), "a separator comes before Other…");
    assert!(
        matches!(&items[n - 1], Node::Item(it) if it.label == "Other…" && it.command.id == id && it.command.params.is_null()),
        "Other… comes last"
    );
    assert!(items[..n - 2].iter().all(|i| matches!(i, Node::Item(it) if it.command.id == "text.setStyle")), "the sizes come first");
    assert!(!enabled(&app, id), "nothing selected");
    let path = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [path]})).unwrap();
    assert!(!enabled(&app, id), "a path selected");
    let text = app.run("text.create", json!({"x": 10, "y": 100, "text": "Size"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    assert!(enabled(&app, id), "type selected");
    assert_eq!(checked(&app, id, &Value::Null), None);
}

/// Type › Dynamic Text Presets › Circle, Arch and Bow run `type.dynamicTextPreset` with their
/// preset, enabled for point type and type on a path but not for area type alone.
#[test]
fn dynamic_text_presets_are_a_type_submenu() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let type_menu = menu_nodes("Type");
    let items: Vec<(&str, &str, Value)> = command_rows(submenu_nodes(&type_menu, "Dynamic Text Presets").unwrap_or_default())
        .into_iter()
        .map(|(label, id, params)| (label, id, params.clone()))
        .collect();
    let id = "type.dynamicTextPreset";
    assert_eq!(items, [("Circle", id, json!({"preset": "circle"})), ("Arch", id, json!({"preset": "arch"})), ("Bow", id, json!({"preset": "bow"}))]);
    assert!(!enabled(&app, id));
    let area = app.run("text.create", json!({"x": 10, "y": 200, "text": "Area", "area": {"width": 100, "height": 50}})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [area]})).unwrap();
    assert!(!enabled(&app, id), "area type");
    let point = app.run("text.create", json!({"x": 10, "y": 10, "text": "Point"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [point]})).unwrap();
    assert!(enabled(&app, id));
    app.run(id, json!({"preset": "arch"})).unwrap();
    // Now type on a path with Dynamic Text on, so Type › Dynamic Text is checked.
    assert!(enabled(&app, id));
    assert_eq!(checked(&app, "type.dynamicText", &Value::Null), Some(true));
}

/// Type › Optical Margin Alignment menu check uses `shared_text_flag`; mixed values stay unchecked.
#[test]
fn optical_margin_alignment_menu_unchecked_when_shared_flag_none() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let off = app.run("text.create", json!({"x": 10, "y": 40, "text": "Off"})).unwrap()["id"].clone();
    let on = app.run("text.create", json!({"x": 10, "y": 80, "text": "On"})).unwrap()["id"].clone();
    app.run("type.opticalMarginAlignment", json!({"on": true, "id": on})).unwrap();
    app.run("select.set", json!({"ids": [off, on]})).unwrap();
    let id = "type.opticalMarginAlignment";
    assert_eq!(crate::panels::character::shared_text_flag(&app, |t| t.optical_margin), None);
    assert_eq!(checked(&app, id, &Value::Null), Some(false));
    app.run("select.set", json!({"ids": [on]})).unwrap();
    assert_eq!(checked(&app, id, &Value::Null), Some(true));
}

/// Type › Orientation menu checks use `shared_text_flag`; mixed values leave both unchecked.
#[test]
fn type_orientation_menu_unchecked_when_shared_vertical_none() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let h = app.run("text.create", json!({"x": 10, "y": 40, "text": "H"})).unwrap()["id"].clone();
    let v = app.run("text.create", json!({"x": 10, "y": 80, "text": "V", "vertical": true})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [h, v]})).unwrap();
    assert_eq!(crate::panels::character::shared_text_flag(&app, |t| t.vertical), None);
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(false));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
    app.run("select.set", json!({"ids": [v]})).unwrap();
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(true));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
}

#[test]
fn type_orientation_menu_unchecked_for_three_object_mixed_selections() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let v0 = app.run("text.create", json!({"x": 10, "y": 40, "text": "v0", "vertical": true})).unwrap()["id"].clone();
    let h1 = app.run("text.create", json!({"x": 10, "y": 80, "text": "h1"})).unwrap()["id"].clone();
    let h2 = app.run("text.create", json!({"x": 10, "y": 120, "text": "h2"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [v0.clone(), h1.clone(), h2.clone()]})).unwrap();
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(false));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
    let h0 = app.run("text.create", json!({"x": 10, "y": 160, "text": "h0"})).unwrap()["id"].clone();
    let v1 = app.run("text.create", json!({"x": 10, "y": 200, "text": "v1", "vertical": true})).unwrap()["id"].clone();
    let v2 = app.run("text.create", json!({"x": 10, "y": 240, "text": "v2", "vertical": true})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [h0.clone(), v1.clone(), v2.clone()]})).unwrap();
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(false));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
    app.run("select.set", json!({"ids": [v0.clone(), v1.clone(), v2.clone()]})).unwrap();
    assert_eq!(checked(&app, "type.orientation.vertical", &Value::Null), Some(true));
    assert_eq!(checked(&app, "type.orientation.horizontal", &Value::Null), Some(false));
}

#[test]
fn panel_ids_are_case_insensitive_and_labels_work() {
    assert_eq!(normalize_panel("layers"), Some("layers"));
    assert_eq!(normalize_panel("Layers"), Some("layers"));
    assert_eq!(normalize_panel("LAYERS"), Some("layers"));
    assert_eq!(normalize_panel("Swatches"), Some("swatches"));
    assert_eq!(normalize_panel("swatches"), Some("swatches"));
    assert_eq!(normalize_panel("Color Guide"), Some("colorGuide"));
    assert_eq!(normalize_panel("colorguide"), Some("colorGuide"));
    assert_eq!(normalize_panel("  Stroke  "), Some("stroke"));
    assert_eq!(normalize_panel("nope"), None);

    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.run("window.panel", json!({"panel": "Layers"})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "layers"));
    app.run("window.panel", json!({"panel": "Swatches"})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "swatches"));
    assert!(checked(&app, "window.panel", &json!({"panel": "Swatches"})).unwrap());
    assert!(app.run("window.panel", json!({"panel": "nope"})).is_err());
    // A collapsed dock pops the panel out of its icon, by canonical id.
    crate::panel_docking::collapse_dock(&mut app, Some(true)).unwrap();
    app.run("window.panel", json!({"panel": "Layers"})).unwrap();
    assert_eq!(app.ui.open_panel.as_deref(), Some("layers"));
    assert!(checked(&app, "window.panel", &json!({"panel": "Layers"})).unwrap());
}

/// #4542: Edit › Color Settings… and Assign Profile… show the Separations Preview panel, which
/// isn't in an icon column, and leave it showing when it already does.
#[test]
fn color_settings_and_assign_profile_show_the_separations_panel() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let shown = |app: &VectorcraftApp| checked(app, "window.panel", &json!({"panel": "separations"})).unwrap();
    assert!(!shown(&app));
    for id in ["edit.colorSettings", "edit.colorSettings", "edit.assignProfile"] {
        app.run(id, json!({})).unwrap();
        assert!(shown(&app), "{id} shows the panel");
    }
}

/// Edit › Refresh Monitor Profile, after Assign Profile…, asks the desktop app's next frame to read
/// the display's profile; it is off and refused with the preference off or nothing to read it.
#[test]
fn refresh_monitor_profile_asks_for_a_reading_of_the_display_profile() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let entries = menu_entries(&app);
    let at = |label: &str| entries.iter().position(|e| e.label == label && e.path == ["Edit"]);
    assert_eq!(at("Refresh Monitor Profile"), at("Assign Profile…").map(|i| i + 1));
    let e = &entries[at("Refresh Monitor Profile").unwrap()];
    assert_eq!((e.command.as_deref(), e.shortcut.as_str()), (Some("view.refreshMonitorProfile"), ""));
    // Nothing reads the display's profile (the web build, tests).
    assert!(!enabled(&app, "view.refreshMonitorProfile"));
    assert_eq!(app.run("view.refreshMonitorProfile", json!({})).unwrap_err(), "this build doesn't read display profiles");
    app.display_profile.attach_reader();
    assert!(enabled(&app, "view.refreshMonitorProfile"));
    assert_eq!(app.run("view.refreshMonitorProfile", json!({})).unwrap(), json!({"requested": true}));
    assert!(app.display_profile.take_request());
    // With the preference off nothing is asked for.
    app.session.prefs.monitor_profile_display = false;
    assert!(!enabled(&app, "view.refreshMonitorProfile"));
    assert_eq!(app.run("view.refreshMonitorProfile", json!({})).unwrap_err(), "turn on Use Monitor Profile for Canvas first");
    assert!(!app.display_profile.read_requested());
}

#[test]
fn saved_views_store_and_restore_the_view() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    {
        let v = app.view_mut().unwrap();
        v.zoom = 3.0;
        v.center = vectorcraft_geom::Point::new(120.0, 80.0);
    }
    app.run("view.saved.new", json!({"name": "Detail"})).unwrap();
    assert!(enabled(&app, "view.goto1") && !enabled(&app, "view.goto2"));
    assert_eq!(dynamic_label(&app, "view.goto1", ""), "Detail");
    app.view_mut().unwrap().zoom = 0.5;
    app.run("view.goto1", json!({})).unwrap();
    let v = app.view().unwrap();
    assert_eq!((v.zoom, v.center.x, v.center.y), (3.0, 120.0, 80.0));
    // Unused slots stay out of the menu listing agents see.
    assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("view.goto2")));
}

/// Effect › 3D and Materials › 3D (Classic) › Extrude & Bevel (Classic)… opens its effect dialog,
/// after Materials…, and nowhere else.
#[test]
fn classic_extrude_bevel_is_in_the_3d_classic_submenu() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let entries = menu_entries(&app);
    let classic: Vec<_> = entries.iter().filter(|e| e.params.get("effect").and_then(Value::as_str) == Some("threeD.classicExtrude")).collect();
    assert_eq!(classic.len(), 1, "listed once");
    let e = classic[0];
    assert_eq!(e.path, ["Effect", "3D and Materials", "3D (Classic)"]);
    assert_eq!((e.label.as_str(), e.command.as_deref()), ("Extrude & Bevel (Classic)…", Some("effect.dialog")));
    let at = |label: &str| entries.iter().position(|x| x.label == label && x.path.get(1).is_some_and(|p| p == "3D and Materials"));
    assert!(at("Materials…").unwrap() < at("Extrude & Bevel (Classic)…").unwrap());
}

/// Effect › 3D and Materials › 3D (Classic) › Revolve (Classic)… opens its effect dialog, after
/// Extrude & Bevel (Classic)…, and nowhere else.
#[test]
fn classic_revolve_is_in_the_3d_classic_submenu() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let entries = menu_entries(&app);
    let classic: Vec<_> = entries.iter().filter(|e| e.params.get("effect").and_then(Value::as_str) == Some("threeD.classicRevolve")).collect();
    assert_eq!(classic.len(), 1, "listed once");
    let e = classic[0];
    assert_eq!(e.path, ["Effect", "3D and Materials", "3D (Classic)"]);
    assert_eq!((e.label.as_str(), e.command.as_deref()), ("Revolve (Classic)…", Some("effect.dialog")));
    let at = |label: &str| entries.iter().position(|x| x.label == label && x.path.get(1).is_some_and(|p| p == "3D and Materials"));
    assert!(at("Extrude & Bevel (Classic)…").unwrap() < at("Revolve (Classic)…").unwrap());
}

/// Effect › 3D and Materials › 3D (Classic) › Rotate (Classic)… opens its effect dialog, after
/// Revolve (Classic)…, and nowhere else.
#[test]
fn classic_rotate_is_in_the_3d_classic_submenu() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let entries = menu_entries(&app);
    let classic: Vec<_> = entries.iter().filter(|e| e.params.get("effect").and_then(Value::as_str) == Some("threeD.classicRotate")).collect();
    assert_eq!(classic.len(), 1, "listed once");
    let e = classic[0];
    assert_eq!(e.path, ["Effect", "3D and Materials", "3D (Classic)"]);
    assert_eq!((e.label.as_str(), e.command.as_deref()), ("Rotate (Classic)…", Some("effect.dialog")));
    let at = |label: &str| entries.iter().position(|x| x.label == label && x.path.get(1).is_some_and(|p| p == "3D and Materials"));
    assert!(at("Revolve (Classic)…").unwrap() < at("Rotate (Classic)…").unwrap());
}

/// A document's 25 views are all listed at the bottom of the View menu, by name, and the last
/// one brings back its zoom, centre and rotation after the view changed.
#[test]
fn every_saved_view_is_listed_at_the_bottom_of_the_view_menu() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    app.run("file.new", json!({})).unwrap();
    for i in 1..=25 {
        {
            let v = app.view_mut().unwrap();
            (v.zoom, v.center, v.rotation) = (i as f64 / 4.0, vectorcraft_geom::Point::new(10.0 * i as f64, -5.0 * i as f64), i as f64);
        }
        invoke(&mut app, "view.saved.new", json!({}));
        answer_open(&mut app, json!({}), true);
    }
    assert_eq!(app.session.active().unwrap().doc.views.len(), 25);
    let entries = menu_entries(&app);
    let at = |id: &str| entries.iter().position(|e| e.command.as_deref() == Some(id));
    let (edit, first, last) = (at("view.saved.edit").unwrap(), at("view.goto1").unwrap(), at("view.goto25").unwrap());
    assert!(edit < first && last == first + 24, "listed in order after Edit Views…");
    assert!(enabled(&app, "view.goto25"));
    assert_eq!(display_label(&app, "view.goto25", "Saved View"), "View 25");
    // The document's 26th view is refused, so there is never one the menu leaves out.
    invoke(&mut app, "view.saved.new", json!({}));
    answer_open(&mut app, json!({}), true);
    assert!(app.any_dialog_open(), "the refused name leaves the dialog open");
    answer_open(&mut app, json!({}), false);
    {
        let v = app.view_mut().unwrap();
        (v.zoom, v.center, v.rotation) = (1.0, vectorcraft_geom::Point::new(0.0, 0.0), 0.0);
    }
    assert_eq!(app.run("view.goto25", json!({})).unwrap()["name"], "View 25");
    let v = app.view().unwrap();
    assert_eq!((v.zoom, v.center.x, v.center.y, v.rotation), (6.25, 250.0, -125.0, 25.0));
}

/// Edit Views… is off without a saved view and opens its list on the document's views, the
/// way Edit Selection… does.
#[test]
fn edit_views_opens_its_dialog_on_the_saved_views() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    app.run("file.new", json!({})).unwrap();
    assert!(!enabled(&app, "view.saved.edit"));
    invoke(&mut app, "view.saved.edit", json!({}));
    assert!(!app.any_dialog_open());
    assert!(app.ui.status.contains("No saved views"), "{}", app.ui.status);
    app.run("view.saved.new", json!({"name": "Detail"})).unwrap();
    app.run("view.saved.new", json!({"name": "Page"})).unwrap();
    assert!(enabled(&app, "view.saved.edit"));
    invoke(&mut app, "view.saved.edit", json!({}));
    let (_, fields) = open_dialog(&app);
    assert_eq!((text_of(&fields, "orig1"), text_of(&fields, "name1")), ("Page".to_string(), "Page".to_string()));
    // Deleting a view takes its slot out of the menu.
    answer_open(&mut app, json!({"deleted0": true}), true);
    assert_eq!(dynamic_label(&app, "view.goto1", ""), "Page");
    assert!(shown_state(&app, "view.goto2", &Value::Null).is_none());
}

fn app_with_a_rectangle() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    app.run("file.new", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app
}

/// Edit › Undo and Edit › Redo name a step named after a style as the History panel lists it:
/// the template's translation with the style's name in it.
#[test]
fn undo_and_redo_name_a_style_step_around_the_style_name() {
    let mut app = app_with_a_rectangle();
    let text = app.run("text.create", json!({"x": 0, "y": 40, "text": "Hello"})).unwrap()["id"].clone();
    app.run("charStyle.new", json!({"name": "Brush"})).unwrap();
    app.run("charStyle.apply", json!({"name": "Brush", "id": text})).unwrap();
    let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
    crate::i18n::set_current(ja);
    let step = |template: &str| {
        let applied = crate::i18n::fmt(crate::i18n::tr(ja, "Apply {name}"), &[("name", "Brush")]);
        crate::i18n::fmt(crate::i18n::tr(ja, template), &[("name", &applied)])
    };
    assert_eq!(display_label(&app, "edit.undo", "Undo"), step("Undo {name}"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(display_label(&app, "edit.redo", "Redo"), step("Redo {name}"));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(display_label(&app, "edit.redo", "Redo"), "Redo Apply Brush");
}

#[test]
fn save_selection_asks_for_a_name() {
    let mut app = app_with_a_rectangle();
    invoke(&mut app, "select.save", json!({}));
    let (dialog, fields) = open_dialog(&app);
    assert_eq!(
        (dialog.key.as_deref(), app.command_dialog.label(), text_of(&fields, "name").as_str()),
        (Some("select.save"), Some("Save Selection"), "Selection 1")
    );
    // OK saves under the name typed.
    answer_open(&mut app, json!({"name": "Logo"}), true);
    assert_eq!(app.run("select.savedList", json!({})).unwrap(), json!(["Logo"]));
    // The next one starts from the first free name.
    invoke(&mut app, "select.save", json!({}));
    assert_eq!(text_of(&open_dialog(&app).1, "name"), "Selection 1");
}

#[test]
fn saved_selections_are_listed_at_the_bottom_of_the_select_menu() {
    let mut app = app_with_a_rectangle();
    app.run("shape.ellipse", json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
    // Nothing saved: no slot is listed or enabled.
    assert!(!enabled(&app, "select.recall1"));
    assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall1")));
    app.run("select.save", json!({"name": "Ellipse"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("select.save", json!({"name": "Everything"})).unwrap();
    assert!(enabled(&app, "select.recall1") && enabled(&app, "select.recall2") && !enabled(&app, "select.recall3"));
    assert_eq!(dynamic_label(&app, "select.recall2", "Saved Selection"), "Everything");
    // A name is shown as it is, not translated.
    assert_eq!(display_label(&app, "select.recall2", "Saved Selection"), "Everything");
    assert!(menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall2")));
    assert!(!menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("select.recall3")));
    // Choosing one selects its objects again.
    app.run("select.none", json!({})).unwrap();
    let r = app.run("select.recall2", json!({})).unwrap();
    assert!(r["count"].as_u64().unwrap() >= 1);
    assert_eq!(app.session.active().unwrap().selection.len(), 2);
    // A deleted selection's slot leaves the menus.
    app.run("select.editSaved", json!({"name": "Ellipse", "delete": true})).unwrap();
    assert!(!enabled(&app, "select.recall2"));
    assert!(shown_state(&app, "select.recall2", &Value::Null).is_none());
    assert!(shown_state(&app, "select.recall1", &Value::Null).is_some());
}

#[test]
fn update_selection_follows_save_selection_and_needs_a_recall() {
    let mut app = app_with_a_rectangle();
    let entries = menu_entries(&app);
    let at = |id: &str| entries.iter().position(|e| e.command.as_deref() == Some(id));
    assert_eq!(at("select.update").map(|i| i - 1), at("select.save"));
    app.run("select.save", json!({"name": "Box"})).unwrap();
    assert!(!enabled(&app, "select.update"));
    app.run("shape.ellipse", json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("select.recall1", json!({})).unwrap();
    assert!(enabled(&app, "select.update"));
    app.run("select.all", json!({})).unwrap();
    invoke(&mut app, "select.update", json!({}));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(app.run("select.recall1", json!({})).unwrap()["count"], 2);
}

#[test]
fn select_same_link_block_series_selects_a_thread_from_the_menu() {
    let mut app = app_with_a_rectangle();
    let entry = menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("select.same.linkBlockSeries")).expect("menu entry");
    assert_eq!((entry.path, entry.label.as_str()), (vec!["Select".to_string(), "Same".to_string()], "Link Block Series"));
    // A rectangle selected: no type, so the item is off.
    assert!(!enabled(&app, "select.same.linkBlockSeries"));
    let frame = |app: &mut VectorcraftApp, x: i32| {
        app.run("text.create", json!({"x": x, "y": 40, "text": "words ".repeat(40), "area": {"width": 60, "height": 30}})).unwrap()["id"].clone()
    };
    let (a, b) = (frame(&mut app, 0), frame(&mut app, 100));
    app.run("select.set", json!({"ids": [a.clone(), b.clone()]})).unwrap();
    app.run("text.thread.create", json!({})).unwrap();
    app.run("select.set", json!({"ids": [b.clone()]})).unwrap();
    assert!(enabled(&app, "select.same.linkBlockSeries"));
    invoke(&mut app, "select.same.linkBlockSeries", json!({}));
    let ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|i| i.0).collect();
    assert_eq!(json!(ids), json!([a, b]));
}

#[test]
fn select_same_text_fill_and_stroke_color_follows_text_stroke_color_in_the_menu() {
    let mut app = app_with_a_rectangle();
    let entries = menu_entries(&app);
    let at = |id: &str| entries.iter().position(|e| e.command.as_deref() == Some(id));
    let entry = at("select.same.textFillAndStrokeColor").map(|i| &entries[i]).expect("menu entry");
    assert_eq!((entry.path.clone(), entry.label.as_str()), (vec!["Select".to_string(), "Same".to_string()], "Text Fill & Stroke Color"));
    assert_eq!(at("select.same.textStrokeColor").map(|i| i + 1), at("select.same.textFillAndStrokeColor"));
    let text = |app: &mut VectorcraftApp, y: i32, fill: &str, stroke: &str| {
        let id = app.run("text.create", json!({"x": 0, "y": y, "text": "type"})).unwrap()["id"].clone();
        app.run("text.setRangeStyle", json!({"id": id.clone(), "start": 0, "end": 4, "fill": fill, "stroke": stroke})).unwrap();
        id
    };
    let a = text(&mut app, 40, "#ff0000", "#0000ff");
    let b = text(&mut app, 80, "#ff0000", "#0000ff");
    let _ = text(&mut app, 120, "#ff0000", "#00ff00");
    app.run("select.set", json!({"ids": [a.clone()]})).unwrap();
    invoke(&mut app, "select.same.textFillAndStrokeColor", json!({}));
    let ids: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|i| i.0).collect();
    assert_eq!(json!(ids), json!([a, b]));
}

#[test]
fn edit_selection_opens_its_dialog_on_the_saved_selections() {
    let mut app = app_with_a_rectangle();
    // Nothing saved yet: the item is off, and invoking it (palette, shortcut) only says so.
    assert!(!enabled(&app, "select.editSaved"));
    invoke(&mut app, "select.editSaved", json!({}));
    assert!(!app.any_dialog_open());
    assert!(app.ui.status.contains("No saved selections"), "{}", app.ui.status);
    app.run("select.save", json!({"name": "Logo"})).unwrap();
    assert!(enabled(&app, "select.editSaved"));
    invoke(&mut app, "select.editSaved", json!({}));
    let (dialog, fields) = open_dialog(&app);
    assert_eq!(dialog.key.as_deref(), Some(crate::dialogs::edit_selection::KIND));
    assert_eq!((text_of(&fields, "orig0"), text_of(&fields, "name0")), ("Logo".to_string(), "Logo".to_string()));
}

#[test]
fn recent_fonts_and_corner_widget_toggle() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let fams = vectorcraft_text::FontDb::global().families();
    let (a, b) = (fams[0].clone(), fams[fams.len() - 1].clone());
    app.run("text.create", json!({"x": 10, "y": 10, "text": "Hi"})).unwrap();
    app.run("text.setStyle", json!({"font": a})).unwrap();
    app.run("text.setStyle", json!({"font": b})).unwrap();
    assert_eq!(app.ui.recent_fonts, vec![b.clone(), a.clone()]);
    assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), a);
    assert!(enabled(&app, "type.recentFont2") && !enabled(&app, "type.recentFont3"));
    app.run("type.recentFont2", json!({})).unwrap();
    assert_eq!(app.ui.recent_fonts[0], a);
    assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Hide Corner Widget");
    app.run("view.cornerWidget", json!({})).unwrap();
    assert_eq!(dynamic_label(&app, "view.cornerWidget", ""), "Show Corner Widget");
}

/// Hide Shape Widgets (`view.shapeWidgets`, the shape tools' Control bar button) flips its label and
/// the setting, `on` sets it, a non-boolean `on` is an error that leaves it, the setting is saved
/// with the interface state, and state saved before the setting existed loads with the widgets on.
#[test]
fn shape_widgets_command_toggles_and_takes_on() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.ui.view.shape_widgets, "on by default");
    assert_eq!(dynamic_label(&app, "view.shapeWidgets", ""), "Hide Shape Widgets");
    assert_eq!(app.run("view.shapeWidgets", json!({})).unwrap(), json!(false));
    assert_eq!(dynamic_label(&app, "view.shapeWidgets", ""), "Show Shape Widgets");
    assert!(!app.view_info().shape_widgets);
    assert_eq!(app.run("view.shapeWidgets", json!({"on": false})).unwrap(), json!(false));
    assert!(app.run("view.shapeWidgets", json!({"on": "yes"})).is_err());
    assert!(!app.ui.view.shape_widgets, "a bad `on` changes nothing");
    assert_eq!(app.run("view.shapeWidgets", json!({"on": true})).unwrap(), json!(true));
    assert!(app.view_info().shape_widgets);
    app.ui.view.shape_widgets = false;
    let mut saved = serde_json::to_value(&app.ui).unwrap();
    let loaded: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert!(!loaded.view.shape_widgets, "the setting is saved");
    saved["view"].as_object_mut().unwrap().remove("shape_widgets");
    let older: crate::UiState = serde_json::from_value(saved).unwrap();
    assert!(older.view.shape_widgets, "older saved state shows the widgets");
    assert!(UI_COMMANDS.iter().any(|c| c.0 == "view.shapeWidgets" && c.2.is_empty()), "no default key");
}

/// View › Precise Cursors has no menu item or default key, but the command (assignable in Keyboard Shortcuts)
/// writes General › Use Precise Cursors, the value the dialog and the saved preferences read.
#[test]
fn precise_cursors_command_toggles_the_preference() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(!app.session.prefs.use_precise_cursors);
    assert_eq!(app.run("view.preciseCursors", json!({})).unwrap(), json!(true));
    assert!(app.session.prefs.use_precise_cursors);
    assert_eq!(app.ui.engine_prefs["usePreciseCursors"], json!(true), "saved with the UI state");
    assert!(UI_COMMANDS.iter().any(|c| c.0 == "view.preciseCursors" && c.2.is_empty()), "assignable, no default key");
    assert_eq!(app.run("view.preciseCursors", json!({})).unwrap(), json!(false));
    assert!(!app.session.prefs.use_precise_cursors);
}

/// The command is listed in Edit › Keyboard Shortcuts without a key and takes one the user types; with the
/// key it is the command that runs.
#[test]
fn precise_cursors_command_is_listed_and_assignable_in_the_shortcut_editor() {
    use crate::shortcut_editor as ed;
    const ID: &str = "view.preciseCursors";
    let e = ed::entry(ID).expect("listed in the editor");
    assert_eq!((e.label.as_str(), e.group.as_str(), e.is_tool), ("Use Precise Cursors", "View", false));
    assert_eq!(ed::default_of(ID), None, "no default key");
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    // The `shortcuts.*` commands read the window's keymap, which its first frame builds.
    crate::tests::synthetic::key_frame(&mut app, &crate::testing::ui_context(), vec![]);
    let shortcut = |app: &mut VectorcraftApp| {
        let rows = app.run("shortcuts.list", json!({ "query": ID })).unwrap();
        rows.as_array().and_then(|rows| rows.iter().find(|r| r["id"] == ID)).expect("listed")["shortcut"].clone()
    };
    assert_eq!(shortcut(&mut app), Value::Null);
    app.run("shortcuts.set", json!({"id": ID, "shortcut": "Cmd+Alt+Shift+Q"})).unwrap();
    let typed = craft::keymap::Binding::parse("Cmd+Alt+Shift+Q").unwrap().trigger.to_string();
    assert_eq!(shortcut(&mut app), json!(typed), "assigned");
    // A key another command holds is refused, so the new command can't take it over silently.
    assert!(app.run("shortcuts.set", json!({"id": ID, "shortcut": "Cmd+Alt+Shift+P"})).is_err());
    assert_eq!(shortcut(&mut app), json!(typed), "the refused key changed nothing");
    app.run("shortcuts.set", json!({"id": ID, "shortcut": null})).unwrap();
    assert_eq!(shortcut(&mut app), Value::Null, "cleared again");
}

/// The command is a preference, not an edit: no undo step, no unsaved changes, with a document open.
#[test]
fn precise_cursors_command_is_not_an_edit() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let steps = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
    };
    let before = steps(&app);
    assert!(!before.2);
    app.run("view.preciseCursors", json!({})).unwrap();
    assert!(app.session.prefs.use_precise_cursors);
    assert_eq!(steps(&app), before);
}

/// Selection & Anchor Display › Zoom to Selection (#394): Zoom In and Zoom Out bring the
/// selection to the middle; off, they keep the view's centre.
#[test]
fn zoom_to_selection_centres_the_selection() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 40, "height": 20})).unwrap();
    let center = |app: &VectorcraftApp| app.view().unwrap().center;
    let start = center(&app);
    app.run("prefs.set", json!({"key": "zoomToSelection", "value": false})).unwrap();
    app.run("view.zoomIn", json!({})).unwrap();
    assert_eq!(center(&app), start, "off: the view's centre stays");
    app.run("prefs.set", json!({"key": "zoomToSelection", "value": true})).unwrap();
    app.run("view.zoomOut", json!({})).unwrap();
    assert_eq!(center(&app), vectorcraft_geom::Point::new(30.0, 30.0), "on: the selection's centre");
}

/// Preferences › Type › Number of Recent Fonts (#394): Type › Recent Fonts lists that many.
#[test]
fn number_of_recent_fonts_sets_how_many_are_listed() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.ui.recent_fonts = (1..=15).map(|i| format!("Font {i}")).collect();
    let listed = |app: &VectorcraftApp| RECENT_FONT_IDS.iter().filter(|id| enabled(app, id)).count();
    assert_eq!(listed(&app), 10, "10 by default");
    for n in [15, 2] {
        app.run("prefs.set", json!({"key": "recentFontsCount", "value": n})).unwrap();
        assert_eq!(listed(&app), n);
    }
    assert_eq!(dynamic_label(&app, "type.recentFont2", "Recent Font"), "Font 2");
    assert!(app.run("type.recentFont3", json!({})).is_err(), "past the count");
}

#[test]
fn recent_files_track_opens_and_saves() {
    let dir = std::env::temp_dir().join(format!("dc-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = VectorcraftApp::new(
        vectorcraft_engine::Session::new(),
        crate::Services {
            read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
            write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
            ..Default::default()
        },
    );
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let (a, b) = (dir.join("a.vectorcraft"), dir.join("b.vectorcraft"));
    for p in [&a, &b, &a] {
        app.run("file.saveAs", json!({"path": p.to_string_lossy()})).unwrap();
    }
    assert_eq!(app.ui.recent_files, [a.to_string_lossy(), b.to_string_lossy()]);
    assert_eq!(dynamic_label(&app, "file.openRecent2", ""), "b.vectorcraft");
    assert!(enabled(&app, "file.openRecent2") && !enabled(&app, "file.openRecent3"));
    app.run("file.openRecent2", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.ui.recent_files[0], b.to_string_lossy(), "reopening moves it to the top");
    app.run("file.clearRecent", json!({})).unwrap();
    assert!(app.ui.recent_files.is_empty() && app.run("file.openRecent1", json!({})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_bound_menu_command_exists() {
    fn walk(nodes: &[Node], bad: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Item(it) => {
                    let id = it.command.id.as_str();
                    // A Coming Soon row (`TODO`) is bound to no command.
                    if id != TODO
                        && vectorcraft_engine::find_command(id).is_none()
                        && !UI_COMMANDS.iter().any(|c| c.0 == id)
                        && !id.starts_with("object.path.")
                    {
                        bad.push(id.to_string());
                    }
                }
                Node::Submenu { children, .. } => walk(children, bad),
                _ => {}
            }
        }
    }
    let mut bad = vec![];
    for (_, children) in menu_tree() {
        walk(&children, &mut bad);
    }
    assert!(bad.is_empty(), "menu items bound to unknown commands: {bad:?}");
}

/// #543: Object › Image Trace › Make and Make and Expand trace with the Default preset at once,
/// with no form asking for a preset's name.
#[test]
fn image_trace_make_traces_with_the_default_preset() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    for cmd in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        let e = menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some(cmd)).unwrap();
        assert_eq!(e.path, ["Object", "Image Trace"]);
        assert_eq!(click_target(&e.label, cmd, &e.params), (cmd.to_string(), json!({"preset": "Default"})), "{}", e.label);
    }
}

#[test]
fn expand_appearance_is_an_object_menu_item_only() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let paths: Vec<Vec<String>> =
        menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some("effect.expandAppearance")).map(|e| e.path).collect();
    assert_eq!(paths, [vec!["Object".to_string()]]);
}

/// Object › Distribute follows Object › Align and lists the Align panel's Distribute Objects and
/// Distribute Spacing buttons, each item running the button's command: dimmed with nothing
/// selected, spacing three objects evenly, and with a key object sending the Distribute Spacing value.
#[test]
fn object_distribute_runs_the_distribute_buttons() {
    let object = menu_nodes("Object");
    let subs: Vec<&str> = object.iter().filter_map(|n| if let Node::Submenu { label, .. } = n { Some(label.as_str()) } else { None }).collect();
    let align = subs.iter().position(|l| *l == "Align").unwrap_or_else(|| panic!("Object › Align: {subs:?}"));
    assert_eq!(subs.get(align + 1), Some(&"Distribute"), "{subs:?}");
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let items = |app: &VectorcraftApp| -> Vec<MenuEntry> { menu_entries(app).into_iter().filter(|e| e.path == ["Object", "Distribute"]).collect() };
    let listed: Vec<(String, Option<String>)> = items(&app).into_iter().map(|e| (e.label, e.command)).collect();
    let expected: Vec<(String, Option<String>)> = [
        ("Horizontal Distribute Left", "align.distributeHorizontalLeft"),
        ("Horizontal Distribute Center", "align.distributeHorizontalCenter"),
        ("Horizontal Distribute Right", "align.distributeHorizontalRight"),
        ("Vertical Distribute Top", "align.distributeVerticalTop"),
        ("Vertical Distribute Center", "align.distributeVerticalCenter"),
        ("Vertical Distribute Bottom", "align.distributeVerticalBottom"),
        ("Horizontal Distribute Space", "align.distributeHorizontalSpace"),
        ("Vertical Distribute Space", "align.distributeVerticalSpace"),
    ]
    .into_iter()
    .map(|(l, id)| (l.to_string(), Some(id.to_string())))
    .collect();
    assert_eq!(listed, expected);
    assert!(items(&app).iter().all(|e| !e.enabled), "nothing selected");
    let ids: Vec<Value> = [0, 30, 100]
        .into_iter()
        .map(|x| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone())
        .collect();
    app.run("select.set", json!({"ids": ids})).unwrap();
    let lefts = |app: &VectorcraftApp| -> Vec<f64> {
        let doc = &app.session.active().unwrap().doc;
        ids.iter().map(|id| doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().x0).collect()
    };
    let click = |app: &mut VectorcraftApp, label: &str| {
        let e = items(app).into_iter().find(|e| e.label == label).unwrap_or_else(|| panic!("{label}"));
        assert!(e.enabled, "{label}");
        let (id, p) = click_target(&e.label, e.command.as_deref().unwrap_or_default(), &e.params);
        invoke(app, &id, p);
    };
    click(&mut app, "Horizontal Distribute Left");
    assert_eq!(lefts(&app), [0.0, 50.0, 100.0], "{}", app.ui.status);
    app.ui.align_spacing = 5.0;
    app.run("select.key", json!({"id": ids[0]})).unwrap();
    click(&mut app, "Horizontal Distribute Space");
    assert_eq!(lefts(&app), [0.0, 15.0, 30.0], "{}", app.ui.status);
}

/// Isolation keeps the Lock and Hide actions available for selected members.
#[test]
fn lock_and_hide_menus_are_enabled_in_isolation_mode() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("select.none", json!({})).unwrap();
    let lock_unselected = |app: &VectorcraftApp| crate::menus::enabled(app, "object.lock.unselected");
    assert!(!lock_unselected(&app), "nothing selected");
    app.run("select.set", json!({"ids": [a, b.clone()]})).unwrap();
    assert!(lock_unselected(&app), "selection present");
    let g = app.run("object.group", json!({})).unwrap()["id"].clone();
    let lock_and_hide: Vec<String> = menu_entries(&app)
        .into_iter()
        .filter(|e| {
            let path: Vec<&str> = e.path.iter().map(String::as_str).collect();
            matches!(path[..], ["Object", "Lock" | "Hide"]) || path == ["Object"] && matches!(e.label.as_str(), "Unlock All" | "Show All")
        })
        .filter_map(|e| e.command)
        .collect();
    assert_eq!(
        lock_and_hide,
        [
            "object.lock",
            "object.lock.above",
            "object.lock.otherLayers",
            "object.unlockAll",
            "object.hide",
            "object.hide.above",
            "object.hide.otherLayers",
            "object.hide.unselected",
            "object.showAll"
        ]
    );
    let shown = |app: &VectorcraftApp| -> Vec<bool> {
        let entries = menu_entries(app);
        lock_and_hide
            .iter()
            .chain([&"object.group".to_string()])
            .map(|id| entries.iter().any(|e| e.command.as_ref() == Some(id) && e.enabled))
            .collect()
    };
    app.run("object.isolate", json!({"id": g.clone()})).unwrap();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    assert_eq!(shown(&app), [true; 10], "in isolation mode");
    assert!(lock_unselected(&app), "shortcut command remains enabled in isolation");
    assert!(crate::menus::enabled(&app, "object.hide.unselected"), "Hide Unselected remains enabled in isolation");
    let chord = crate::shortcut_editor::default_of("object.hide.unselected").expect("default Hide Unselected shortcut");
    assert_eq!(craft::menu::Chord::parse(&chord).unwrap(), craft::menu::Chord::parse("Cmd+Alt+Shift+3").unwrap());
    app.run("object.exitIsolation", json!({})).unwrap();
    app.run("select.set", json!({"ids": [g]})).unwrap();
    assert_eq!(shown(&app), [true; 10], "after Exit Isolation Mode");
}

/// Object › Unlock All and Object › Show All clicked with Shift+Alt held run with
/// `scope: "group"`; clicked without them, or with only one of the two, they run on the whole
/// document as before. The swap itself is the framework's; this is the model's table of it.
#[test]
fn unlock_all_and_show_all_clicked_with_shift_alt_act_within_the_group() {
    let object = menu_nodes("Object");
    let held = |chord: &str| craft::menu::Chord::parse(chord).unwrap().mods;
    for id in ["object.unlockAll", "object.showAll"] {
        let item = object
            .iter()
            .find_map(|n| match n {
                Node::Item(it) if it.command.id == id => Some(it),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{id} is not in the Object menu"));
        let alternate = item.alternate.as_ref().unwrap_or_else(|| panic!("{id} has no alternate"));
        assert!(alternate.selected_by(held("Shift+Alt+A")), "{id} with Shift+Alt");
        assert_eq!((alternate.command.id.as_str(), &alternate.command.params), (id, &json!({"scope": "group"})), "{id} with Shift+Alt");
        for chord in ["A", "Shift+A", "Alt+A"] {
            assert!(!alternate.selected_by(held(chord)), "{id} with {chord}");
        }
        assert!(item.command.params.get("scope").is_none(), "{id} acts on the whole document");
    }
}

/// #4116: Window › New Window is enabled with a document open and opens a second view of it; with
/// no document open it is greyed out.
#[test]
fn new_window_opens_a_view_of_the_current_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(!enabled(&app, "window.newWindow"), "no document to open a window on");
    assert_eq!(app.run("window.newWindow", json!({})).unwrap_err(), "no document open");
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert!(enabled(&app, "window.newWindow"));
    assert_eq!(app.run("window.newWindow", json!({})).unwrap(), json!({"index": 0, "view": 2}));
    assert_eq!(app.session.documents().len(), 1, "the same document, in a second window");
}

/// #4143: Window menu panels and panel submenus are listed in alphabetical order according to the
/// active interface language (e.g. French), rather than staying frozen in English alphabetical order.
#[test]
fn window_menu_panels_are_sorted_alphabetically_in_active_language() {
    use crate::i18n::{Lang, current, name_sort_key, set_current, tr, tr_id};

    let prev = current();

    // 1. In English: panels between separator 2 and 3 are alphabetical.
    set_current(Lang::EN);
    let window = menu_nodes("Window");

    let seps = separators(&window);
    assert!(seps.len() >= 3, "Window menu should have at least 3 separators");
    let panels = &window[seps[1] + 1..seps[2]];

    let item_display = |it: &Node, lang: Lang| -> String {
        match it {
            Node::Item(item) => tr_id(lang, &item.command.id, &item.label).to_string(),
            Node::Submenu { label, .. } | Node::Heading(label) => tr(lang, label).to_string(),
            _ => String::new(),
        }
    };

    let en_keys: Vec<String> = panels.iter().map(|it| name_sort_key(&item_display(it, Lang::EN))).collect();
    assert!(en_keys.windows(2).all(|w| w[0] <= w[1]), "English panels must be alphabetical: {en_keys:?}");

    // 2. In French: panels are sorted alphabetically by their French names.
    let fr = Lang::from_code("fr").unwrap();
    set_current(fr);
    let window_fr = menu_nodes("Window");
    let seps_fr = separators(&window_fr);
    let panels_fr = &window_fr[seps_fr[1] + 1..seps_fr[2]];

    let fr_labels: Vec<String> = panels_fr.iter().map(|it| item_display(it, fr)).collect();
    let fr_keys: Vec<String> = fr_labels.iter().map(|l| name_sort_key(l)).collect();
    assert!(fr_keys.windows(2).all(|w| w[0] <= w[1]), "French panels must be alphabetical: {fr_labels:?}");

    // Specific French placements:
    // Calques (Layers) is under C, Plans de travail (Artboards) is under P, Nuancier (Swatches) is under N.
    let pos = |name: &str| fr_labels.iter().position(|l| l == name).unwrap_or_else(|| panic!("missing {name} in {fr_labels:?}"));
    assert!(pos("Aspect") < pos("Calques"), "Aspect before Calques");
    assert!(pos("Calques") < pos("Dégradé"), "Calques before Dégradé");
    assert!(pos("Nuancier") < pos("Plans de travail"), "Nuancier before Plans de travail");
    assert!(pos("Plans de travail") < pos("Texte"), "Plans de travail before Texte");

    // 3. Type submenu is also alphabetical in French.
    let type_sub = panels_fr
        .iter()
        .find_map(|it| match it {
            Node::Submenu { label, children, .. } if label == "Type" => Some(children),
            _ => None,
        })
        .unwrap();
    let type_labels: Vec<String> = type_sub.iter().map(|it| item_display(it, fr)).collect();
    let type_keys: Vec<String> = type_labels.iter().map(|l| name_sort_key(l)).collect();
    assert!(type_keys.windows(2).all(|w| w[0] <= w[1]), "Type submenu must be alphabetical: {type_labels:?}");
    assert_eq!(type_labels, vec!["Caractère", "Glyphes", "OpenType", "Paragraphe", "Styles de caractère", "Styles de paragraphe", "Tabulations",]);

    // 4. In Polish: Łącza (Links) folds to L and sorts before Warstwy and Zmienne.
    let pl = Lang::from_code("pl").unwrap();
    set_current(pl);
    let window_pl = menu_nodes("Window");
    let seps_pl = separators(&window_pl);
    let panels_pl = &window_pl[seps_pl[1] + 1..seps_pl[2]];
    let pl_labels: Vec<String> = panels_pl.iter().map(|it| item_display(it, pl)).collect();
    let pl_keys: Vec<String> = pl_labels.iter().map(|l| name_sort_key(l)).collect();
    assert!(pl_keys.windows(2).all(|w| w[0] <= w[1]), "Polish panels must be alphabetical: {pl_labels:?}");
    let pos_pl = |name: &str| pl_labels.iter().position(|l| l == name).unwrap_or_else(|| panic!("missing {name} in {pl_labels:?}"));
    assert!(pos_pl("Łącza") < pos_pl("Warstwy"), "Łącza before Warstwy");
    assert!(pos_pl("Warstwy") < pos_pl("Zmienne"), "Warstwy before Zmienne");

    set_current(prev);
}

/// A command reports the same failed valid tool choice as `tool.select`, rather than success
/// with the old tool. The existing failure hook exercises the activation error boundary.
#[test]
fn selection_tools_toggle_propagates_a_failed_valid_choice() {
    let failure = "selection tool choice failed";
    let mut failures = Vec::new();
    for (from, to) in [("selection", "directSelection"), ("directSelection", "selection")] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        assert!(vectorcraft_tools::tool_info(from).is_some() && vectorcraft_tools::tool_info(to).is_some());
        app.run("tool.select", json!({"tool": from})).unwrap();
        // The real successful command chooses the other valid selection tool.
        assert_eq!(app.run("tool.switchSelectionTools", json!({})).unwrap(), json!({"tool": to}));
        app.run("tool.select", json!({"tool": from})).unwrap();
        app.ui.touch_drawing_tool = Some("paintbrush".into());
        let snapshot = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            (serde_json::to_value(st.doc.as_ref()).unwrap(), st.selection.clone(), st.history.undo.len(), st.history.redo.len())
        };
        let before = snapshot(&app);
        let groups = app.ui.group_tool.clone();
        let drawing = app.ui.touch_drawing_tool.clone();
        app.fail_next_tool_choice = Some(failure.into());
        let result = app.run("tool.switchSelectionTools", json!({}));
        assert!(app.fail_next_tool_choice.is_none(), "the valid activation consumed the failure hook");
        assert_eq!(app.session.tool_id(), from, "a failed choice keeps the current tool");
        assert_eq!(snapshot(&app), before, "failed choice preserves document, selection and history");
        assert_eq!(app.ui.group_tool, groups, "failed choice does not remember the requested tool");
        assert_eq!(app.ui.touch_drawing_tool, drawing);
        // The hook is one-shot: ordinary success and the return direction still work afterwards.
        assert_eq!(app.run("tool.switchSelectionTools", json!({})).unwrap(), json!({"tool": to}));
        assert_eq!(app.run("tool.switchSelectionTools", json!({})).unwrap(), json!({"tool": from}));
        assert_eq!(snapshot(&app), before, "successful selection toggles do not edit the artwork");
        failures.push(result);
    }
    assert_eq!(failures, vec![Err(failure.to_string()), Err(failure.to_string())], "both failed valid choices must reach the command caller");
}

/// #8363: while Trim View is on, View › Guides › Show/Hide Guides and View › Show/Hide Grid are
/// greyed out (in the menu bar, on their keys and in the canvas context menu, where they stay
/// listed), refuse to run and leave the choices they keep alone; turning it off brings them back.
#[test]
fn trim_view_greys_out_show_guides_and_show_grid() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.grid = true;
    let listed = |app: &VectorcraftApp, id: &str| {
        fn find(nodes: &[Node], id: &str) -> bool {
            nodes.iter().any(|n| match n {
                Node::Item(it) => it.command.id == id,
                Node::Submenu { children, .. } => find(children, id),
                _ => false,
            })
        }
        find(&context_items(app), id)
    };
    for id in ["view.guides", "view.grid"] {
        assert!(enabled(&app, id), "{id} runs with Trim View off");
        assert!(listed(&app, id), "{id} is in the canvas context menu");
    }

    app.run("view.trimView", json!({})).unwrap();
    for id in ["view.guides", "view.grid"] {
        assert!(!enabled(&app, id), "{id} is greyed out while Trim View is on");
        assert!(listed(&app, id), "{id} stays in the canvas context menu, greyed out");
        assert!(app.run(id, json!({})).is_err(), "{id} does not run while Trim View is on");
    }
    assert!(app.ui.view.guides && app.ui.view.grid, "the choices stay as they were");
    assert_eq!((checked(&app, "view.grid", &Value::Null), dynamic_label(&app, "view.guides", "Show Guides").as_str()), (Some(true), "Hide Guides"));

    app.run("view.trimView", json!({})).unwrap();
    for id in ["view.guides", "view.grid"] {
        assert!(enabled(&app, id), "{id} runs again with Trim View off");
    }
    app.run("view.guides", json!({})).unwrap();
    assert!(!app.ui.view.guides);
}
