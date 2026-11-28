use super::*;
use crate::testing::ui_context;
use crate::tests::picks::{answer_dialog, cancel_dialog, open_dialogs, poll, scripted};
use crate::tests::synthetic::key_frame;
use vectorcraft_doc::{NodeId, NodeKind};

#[test]
fn switching_documents_dismisses_text_styles_row_menus() {
    use crate::tests::panel_harness::Panel;
    use vectorcraft_engine::Session;

    for kind in [Kind::Char, Kind::Para] {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let command = format!("{}.new", kind.prefix());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run(&command, json!({"name": "First Style"})).unwrap();
        let first = style_id(&mut app, kind, "First Style");
        let source = app.session.active().unwrap().uid;
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run(&command, json!({"name": "Second Style"})).unwrap();
        let second = style_id(&mut app, kind, "Second Style");
        let destination = app.session.active().unwrap().uid;
        assert_eq!(first, second, "the two documents carry styles at the same list index");
        app.run("window.previousDocument", json!({})).unwrap();

        let draw: fn(&mut VectorcraftApp, &mut Ui) = match kind {
            Kind::Char => |app, ui| show(app, ui, Kind::Char),
            Kind::Para => |app, ui| show(app, ui, Kind::Para),
        };
        let mut panel = Panel::new(app, draw);
        let activate = |panel: &mut Panel, index, document| {
            panel.app().session.execute("document.activate", &json!({"index": index})).unwrap();
            panel.frame();
            panel.frame();
            assert_eq!(panel.app().session.active().unwrap().uid, document);
        };
        let has_copy = |panel: &Panel, name: &str| styles(&mut panel.app(), kind).iter().any(|style| style.0 == name);
        panel.secondary_click("First Style");
        assert!(panel.menu_item_enabled("Duplicate Style"));
        activate(&mut panel, 1, destination);
        assert!(!panel.shows("Duplicate Style"), "the new document does not inherit the row menu");
        assert!(!has_copy(&panel, "Second Style copy"));

        activate(&mut panel, 0, source);
        assert!(!panel.shows("Duplicate Style"), "the old row menu does not reappear");
        assert!(!has_copy(&panel, "First Style copy"));

        activate(&mut panel, 1, destination);
        panel.secondary_click("Second Style");
        panel.click_menu_item("Duplicate Style");
        assert!(has_copy(&panel, "Second Style copy"));
    }
}

// PLAN: assumed type-w19 items reachable through `super::*`: `run_action(app, kind, action)`,
// `StylesPanel { highlight: Vec<String>, .. }` and `StylesPanels { char, para, made }` at
// `app.ui.text_styles`, `highlighted(app, kind, listed: &[&str]) -> Vec<String>`, `TextStyleAction`, and a
// style id that is the style's row in `styles` (Normal is 0).

/// Runs `action` as the panel's button or menu does.
fn act(app: &mut VectorcraftApp, kind: Kind, action: TextStyleAction) {
    run_action(app, kind, action);
}

/// `kind`'s panel state.
fn panel_of(app: &mut VectorcraftApp, kind: Kind) -> &mut StylesPanel {
    if kind == Kind::Char { &mut app.ui.text_styles.char } else { &mut app.ui.text_styles.para }
}

/// One `show` pass of `kind`'s panel.
fn draw(app: &mut VectorcraftApp, ctx: &egui::Context, kind: Kind) {
    ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui, kind)).textures_delta.clear();
}

/// The id `run_action` takes for the style `name`: its row in the list, Normal being 0.
fn style_id(app: &mut VectorcraftApp, kind: Kind, name: &str) -> u64 {
    styles(app, kind).iter().position(|s| s.0 == name).expect("the style is listed") as u64
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The styles `kind`'s panel highlights after one `show` pass.
fn highlighted_now(app: &mut VectorcraftApp, ctx: &egui::Context, kind: Kind) -> Vec<String> {
    draw(app, ctx, kind);
    let all = styles(app, kind);
    highlighted(app, kind, &all.iter().map(|s| s.0.as_str()).collect::<Vec<_>>())
}

/// Highlights `names` in `kind`'s panel through a `show` pass, as a row click does.
fn highlight(app: &mut VectorcraftApp, ctx: &egui::Context, kind: Kind, names: &[&str]) {
    panel_of(app, kind).highlight = names.iter().map(|n| n.to_string()).collect();
    draw(app, ctx, kind);
}

/// Applies the character style `name` to the selected type and highlights its row, as clicking it does.
fn pick_row(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    let id = style_id(app, Kind::Char, name);
    act(app, Kind::Char, TextStyleAction::apply(id, false));
    highlight(app, ctx, Kind::Char, &[name]);
}

/// The key of the hosted Style Options dialog of `kind`.
fn options_key(kind: Kind) -> &'static str {
    if kind == Kind::Char { "charStyleOptions" } else { "paraStyleOptions" }
}

/// The values of the open hosted dialog `key`.
// PLAN: assumed `OpenDialog::key` and `fields` as in craft-ui-panels, and the field names type-w21 fixes.
fn fields_of(app: &VectorcraftApp, key: &str) -> Option<Value> {
    open_dialogs(app).into_iter().find(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields.clone())
}

fn is_open(app: &VectorcraftApp, key: &str) -> bool {
    open_dialogs(app).iter().any(|d| d.key.as_deref() == Some(key))
}

/// Sets `fields` of the open dialog `key` and confirms it (`ok`) or cancels it.
fn answer(app: &mut VectorcraftApp, key: &str, fields: &[(&str, Value)], ok: bool) {
    let id = open_dialogs(app).into_iter().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id).expect("the dialog is open");
    answer_dialog(app, id, fields, ok);
    if !ok {
        cancel_dialog(app, id);
    }
}

/// Names the style in the open New Style dialog and confirms it. A refused name leaves the dialog open.
fn confirm_new(app: &mut VectorcraftApp, name: &str) {
    answer(app, "newTextStyle", &[("name", json!(name))], true);
}

/// Whether the type the panel acts on overrides the style `name`.
fn overridden(app: &mut VectorcraftApp, kind: Kind, name: &str) -> bool {
    let attrs = styles(app, kind).into_iter().find(|s| s.0 == name).expect("the style is listed").1;
    has_overrides(app, kind, &attrs)
}

#[test]
fn auto_leading_style_marks_only_real_overrides() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("charStyle.new", json!({"name": "Automatic", "attrs": {"leading": null}})).unwrap();
    app.run("charStyle.apply", json!({"name": "Automatic"})).unwrap();
    for (leading, expected) in [(json!("auto"), false), (json!(30), true)] {
        app.run("text.setStyle", json!({"leading": leading})).unwrap();
        assert_eq!(overridden(&mut app, Kind::Char, "Automatic"), expected, "leading {leading}");
    }
    app.run("charStyle.apply", json!({"name": "Automatic", "clearOverrides": true})).unwrap();
    assert!(!overridden(&mut app, Kind::Char, "Automatic"), "no override after clearing");
}

/// Styles made from new type hold its Auto alignment and its attributes at their defaults;
/// the type they were made from shows no override until one of those attributes changes.
#[test]
fn styles_made_from_type_mark_only_real_overrides() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    for kind in [Kind::Char, Kind::Para] {
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Plain"})).unwrap();
        app.run(&format!("{}.apply", kind.prefix()), json!({"name": "Plain"})).unwrap();
    }
    for (change, expected) in [(None, false), (Some(json!({"kerning": 40, "composer": "singleLine"})), true)] {
        if let Some(p) = change {
            app.run("text.setFormat", p).unwrap();
        }
        for kind in [Kind::Char, Kind::Para] {
            assert_eq!(current(&app, kind), Some(Some("Plain".to_string())), "{} Styles", kind.what());
            assert_eq!(overridden(&mut app, kind, "Plain"), expected, "{} Styles", kind.what());
        }
    }
}

/// A document with type that both kinds' style `Body` is applied to, the type selected.
fn drag_app() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "Hi", "size": 18})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    for kind in [Kind::Char, Kind::Para] {
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Body"})).unwrap();
        app.run(&format!("{}.apply", kind.prefix()), json!({"name": "Body"})).unwrap();
    }
    (app, id)
}

/// Each panel action runs its style command in one undo step and leaves the highlight as the panel's
/// own buttons do: Duplicate highlights the copy, Delete clears the highlight and never deletes Normal,
/// the highlighted variants act on every highlighted style in list order, and Options opens the hosted
/// Style Options of that style.
#[test]
fn a_panel_action_runs_its_style_command() {
    for kind in [Kind::Char, Kind::Para] {
        let what = kind.what();
        let (mut app, id) = drag_app();
        let ctx = ui_context();
        app.ui_ctx = Some(ctx.clone());
        let before = styles(&mut app, kind);
        let body = style_id(&mut app, kind, "Body");
        let undo = undo_len(&app);

        act(&mut app, kind, TextStyleAction::Duplicate(body));
        let after = styles(&mut app, kind);
        let source = before.iter().find(|s| s.0 == "Body").unwrap();
        assert_eq!(after.iter().find(|s| s.0 == "Body copy").map(|s| &s.1), Some(&source.1), "{what} Styles: the copy has Body's attributes");
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(undo_len(&app), undo + 1, "{what} Styles: one undo step");
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["Body copy"], "{what} Styles: the copy is highlighted");

        let copy = style_id(&mut app, kind, "Body copy");
        act(&mut app, kind, TextStyleAction::Delete(copy));
        assert_eq!(styles(&mut app, kind), before, "{what} Styles: the copy is deleted");
        assert_eq!(undo_len(&app), undo + 2);
        assert!(highlighted_now(&mut app, &ctx, kind).is_empty(), "{what} Styles: Delete clears the highlight");
        assert_eq!(text_runs(&app, id)[0].1, 18.0, "the type keeps its look");

        act(&mut app, kind, TextStyleAction::Delete(0));
        assert_eq!(styles(&mut app, kind), before, "{what} Styles: Normal is never deleted");

        act(&mut app, kind, TextStyleAction::Options(body));
        assert!(is_open(&app, options_key(kind)), "{what} Styles: Options opens the hosted dialog");

        let (mut app, _) = several_app();
        let ctx = ui_context();
        let before = styles(&mut app, kind);
        highlight(&mut app, &ctx, kind, &["A", "C"]);
        let undo = undo_len(&app);
        act(&mut app, kind, TextStyleAction::DuplicateHighlighted);
        let names: Vec<String> = styles(&mut app, kind).into_iter().skip(1).map(|s| s.0).collect();
        assert_eq!(names, ["A", "B", "C", "D", "A copy", "C copy"], "{what} Styles: A and C are duplicated");
        assert_eq!(undo_len(&app), undo + 1, "{what} Styles: one undo step");
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["A copy", "C copy"]);
        highlight(&mut app, &ctx, kind, &["A", "C", "A copy"]);
        act(&mut app, kind, TextStyleAction::DeleteHighlighted);
        let names: Vec<String> = styles(&mut app, kind).into_iter().skip(1).map(|s| s.0).collect();
        assert_eq!(names, ["B", "D", "C copy"], "{what} Styles: the highlighted styles are deleted");
        assert_eq!(undo_len(&app), undo + 2);
        assert!(highlighted_now(&mut app, &ctx, kind).is_empty());
        app.run("edit.undo", json!({})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(styles(&mut app, kind), before);
    }
}

fn capture_app() -> (VectorcraftApp, NodeId, egui::Context) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("charStyle.new", json!({"name": "Untouched", "attrs": {"size": 22}})).unwrap();
    let other = app.run("text.create", json!({"x": 20, "y": 100, "text": "Other", "size": 22})).unwrap()["id"].clone();
    app.run("charStyle.apply", json!({"name": "Untouched", "id": other})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "ABcdef", "size": 18})).unwrap()["id"].as_u64().unwrap());
    app.run("text.setRangeStyle", json!({"id": id.0, "start": 2, "end": 6, "size": 34})).unwrap();
    let ctx = ui_context();
    app.ui_ctx = Some(ctx.clone());
    assert_eq!(text_editing(&app), None);
    (app, id, ctx)
}

fn select_type_range(app: &mut VectorcraftApp, id: NodeId, start: usize, end: usize) {
    // The same editing handshake and selection API as character::tests::scaled_range_fields_follow_the_type_tool_selection.
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": start, "end": end}));
    super::super::character::end_typing(app);
    assert_eq!(app.session.tool_options()["typing"], json!(false));
    assert!(!app.session.in_interaction(), "settle typing before taking the history baseline");
    assert_eq!(text_editing(app), Some((id, start, end)));
}

/// Normal has no Style Options: the action opens no dialog and runs no command.
#[test]
fn double_clicking_the_normal_style_opens_no_options() {
    for kind in [Kind::Char, Kind::Para] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.ui_ctx = Some(ui_context());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        let id = app.run("text.create", json!({"x": 20, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let commands = app.run_count;
        act(&mut app, kind, TextStyleAction::Options(0));
        assert!(!is_open(&app, options_key(kind)), "{} Styles: Normal has no Style Options", kind.what());
        assert_eq!(app.run_count, commands, "{} Styles: no command runs", kind.what());
    }
}

/// The panel menu's Duplicate highlights the copy and its Delete clears the highlight; after a
/// delete, Duplicate has nothing highlighted to act on.
#[test]
fn panel_menu_duplicate_and_delete_update_the_highlighted_style() {
    for kind in [Kind::Char, Kind::Para] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Body"})).unwrap();
        let ctx = ui_context();
        let what = kind.what();
        let has = |app: &mut VectorcraftApp, name: &str| styles(app, kind).iter().any(|s| s.0 == name);
        highlight(&mut app, &ctx, kind, &["Body"]);

        let body = style_id(&mut app, kind, "Body");
        act(&mut app, kind, TextStyleAction::Duplicate(body));
        assert!(has(&mut app, "Body copy"), "{what}: the copy exists");
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["Body copy"], "{what}: Duplicate highlights the copy");

        let copy = style_id(&mut app, kind, "Body copy");
        act(&mut app, kind, TextStyleAction::Delete(copy));
        assert!(!has(&mut app, "Body copy"), "{what}: the copy is deleted");
        assert!(highlighted_now(&mut app, &ctx, kind).is_empty(), "{what}: Delete clears the highlight");
        let commands = app.run_count;
        act(&mut app, kind, TextStyleAction::DuplicateHighlighted);
        assert_eq!((app.run_count, has(&mut app, "Body copy")), (commands, false), "{what}: Duplicate has nothing to act on");
        assert!(has(&mut app, "Body"), "{what}: the original stays");
    }
}

#[test]
fn character_style_options_menu_opens_format_sections() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.ui_ctx = Some(ui_context());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
    let body = style_id(&mut app, Kind::Char, "Body");
    act(&mut app, Kind::Char, TextStyleAction::Options(body));
    let fields = fields_of(&app, "charStyleOptions").expect("Options opens Character Style Options");
    assert_eq!(fields["name"], "Body");
    assert_eq!(fields["tab"], "general");
    // The dialog holds opened numbers as their text.
    assert_eq!(fields["size"].as_f64().or_else(|| fields["size"].as_str()?.parse().ok()), Some(24.0), "{}", fields["size"]);
    assert_eq!(fields["preview"], true);
}

fn text_runs(app: &VectorcraftApp, id: NodeId) -> Vec<(String, f64, Option<String>)> {
    let st = app.session.doc().unwrap();
    let NodeKind::Text(text) = &st.doc.node(id).unwrap().kind else { panic!("expected text") };
    text.runs.iter().map(|r| (r.text.clone(), r.style.size, r.style.style_name.clone())).collect()
}

fn assert_new_style_highlight(app: &mut VectorcraftApp, redefine: bool, name: &str) {
    if !redefine {
        assert_eq!(app.ui.text_styles.char.highlight, [name], "New highlights the style it made");
    }
}

fn capture_from_ui(range: Option<(usize, usize)>, redefine: bool, unicode: bool) {
    let (mut app, id, ctx) = capture_app();
    let (first, tail) = if unicode { ("é", "漢字") } else { ("AB", "cdef") };
    let len = first.len() + tail.len();
    if unicode {
        app.run("text.setText", json!({"id": id.0, "text": format!("{first}{tail}")})).unwrap();
        app.run("text.setRangeStyle", json!({"id": id.0, "start": first.len(), "end": len, "size": 34})).unwrap();
    }
    let mut follower = None;
    let mut local_override = None;
    if redefine {
        app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24}})).unwrap();
        pick_row(&mut app, &ctx, "Body");
        assert_eq!(highlighted_now(&mut app, &ctx, Kind::Char), ["Body"]);
        for (slot, text, y, size) in [(&mut follower, "Follower", 130, 24), (&mut local_override, "Override", 160, 48)] {
            let user = NodeId(app.run("text.create", json!({"x": 20, "y": y, "text": text, "size": 24})).unwrap()["id"].as_u64().unwrap());
            app.run("charStyle.apply", json!({"name": "Body", "id": user.0})).unwrap();
            app.run("text.setRangeStyle", json!({"id": user.0, "start": 0, "end": text.len(), "size": size})).unwrap();
            *slot = Some(user);
        }
        app.run("select.set", json!({"ids": [id.0]})).unwrap();
        app.run("text.setRangeStyle", json!({"id": id.0, "start": 0, "end": len, "size": 18})).unwrap();
        app.run("text.setRangeStyle", json!({"id": id.0, "start": first.len(), "end": len, "size": 34})).unwrap();
    }
    let link = redefine.then(|| "Body".to_string());
    assert_eq!(text_runs(&app, id), vec![(first.into(), 18.0, link.clone()), (tail.into(), 34.0, link)]);
    if let Some((start, end)) = range {
        select_type_range(&mut app, id, start, end);
    }
    assert!(!app.session.in_interaction());
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo = undo_len(&app);
    assert!(!app.session.doc().unwrap().is_dirty());
    let label = if redefine { "Redefine Character Style" } else { "New Character Style…" };
    let action = if redefine { TextStyleAction::Redefine(style_id(&mut app, Kind::Char, "Body")) } else { TextStyleAction::new_style(true, false) };
    act(&mut app, Kind::Char, action);
    draw(&mut app, &ctx, Kind::Char);
    assert_eq!(text_editing(&app), range.map(|(start, end)| (id, start, end)), "Type selection after {label}");
    let after = app.session.doc().unwrap().doc.clone();
    let name = if redefine { "Body" } else { "Character Style 1" };
    assert_new_style_highlight(&mut app, redefine, name);
    let definition = after.char_styles.iter().find(|s| s.name == name);
    let size = definition.and_then(|s| s.attrs.get("size")).and_then(Value::as_f64);
    let expected_size = if range.is_some_and(|(a, b)| b > a && a >= first.len()) { 34.0 } else { 18.0 };
    let count = before.char_styles.len() + usize::from(!redefine);
    let history = &app.session.doc().unwrap().history;
    assert_eq!(
        (after.char_styles.len(), size, history.undo.len(), history.redo.len()),
        (count, Some(expected_size), undo + 1, 0),
        "{label} dispatched with live range {range:?}; capture must use the selected start and make one style edit"
    );
    assert!(!definition.unwrap().attrs.contains_key("style_name"));
    for old in &before.char_styles {
        if old.name != name {
            assert_eq!(after.char_styles.iter().find(|s| s.name == old.name), Some(old));
        }
    }
    // New captures without applying. Redefine changes its unmodified follower, preserving overrides and unrelated text.
    before.walk(|node| {
        if matches!(&node.kind, NodeKind::Text(_)) && Some(node.id) != follower {
            assert_eq!(after.node(node.id), Some(node));
        }
    });
    if let Some(user) = follower {
        assert_eq!(text_runs(&app, user), vec![("Follower".into(), expected_size, Some("Body".into()))]);
    }
    if let Some(user) = local_override {
        assert_eq!(text_runs(&app, user), vec![("Override".into(), 48.0, Some("Body".into()))]);
    }
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert!(app.session.doc().unwrap().is_dirty());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert!(!app.session.doc().unwrap().is_dirty());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, after);
    assert!(app.session.doc().unwrap().is_dirty());
}

// Start 3 is inside the second run; style_at chooses the preceding run at an exact boundary.
#[test]
fn selected_type_range_new_character_style_from_menu() {
    capture_from_ui(Some((3, 5)), false, false);
}

#[test]
fn selected_type_range_redefine_character_style_from_menu() {
    capture_from_ui(Some((3, 5)), true, false);
}

#[test]
fn new_character_style_object_and_caret_controls() {
    // The current collapsed target omits start even at caret 3, so capture still uses object-start size 18.
    for range in [None, Some((3, 3))] {
        capture_from_ui(range, false, false);
    }
}

#[test]
fn redefine_character_style_object_and_caret_controls() {
    for range in [None, Some((3, 3))] {
        capture_from_ui(range, true, false);
    }
}

#[test]
fn character_style_range_apply_from_panel_control() {
    let (mut app, id, _ctx) = capture_app();
    app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 26}})).unwrap();
    select_type_range(&mut app, id, 3, 5);
    let before = app.session.doc().unwrap().doc.clone();
    let undo = undo_len(&app);
    let body = style_id(&mut app, Kind::Char, "Body");
    act(&mut app, Kind::Char, TextStyleAction::apply(body, false));
    let after = app.session.doc().unwrap().doc.clone();
    assert_eq!(
        text_runs(&app, id),
        vec![("AB".into(), 18.0, None), ("c".into(), 34.0, None), ("de".into(), 26.0, Some("Body".into())), ("f".into(), 34.0, None)]
    );
    assert_eq!(after.char_styles, before.char_styles, "Apply does not redefine Body");
    before.walk(|node| {
        if node.id != id && matches!(&node.kind, NodeKind::Text(_)) {
            assert_eq!(after.node(node.id), Some(node));
        }
    });
    assert_eq!((app.session.doc().unwrap().history.undo.len(), app.session.doc().unwrap().history.redo.len()), (undo + 1, 0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, after);
}

/// A document with selected type and both kinds' styles A, B, C and D, the type using B.
fn several_app() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = NodeId(app.run("text.create", json!({"x": 20, "y": 50, "text": "Hi", "size": 18})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    for kind in [Kind::Char, Kind::Para] {
        for n in ["A", "B", "C", "D"] {
            app.run(&format!("{}.new", kind.prefix()), json!({"name": n, "attrs": {}})).unwrap();
        }
        app.run(&format!("{}.apply", kind.prefix()), json!({"name": "B"})).unwrap();
    }
    (app, id)
}

#[test]
fn switching_documents_clears_highlighted_styles_with_matching_names() {
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, _) = several_app();
        let ctx = ui_context();
        highlight(&mut app, &ctx, kind, &["A", "B"]);
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["A", "B"]);

        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        for name in ["A", "B"] {
            app.run(&format!("{}.new", kind.prefix()), json!({"name": name, "attrs": {}})).unwrap();
        }
        assert!(highlighted_now(&mut app, &ctx, kind).is_empty(), "{} Styles: the new document has no highlighted styles", kind.what());
        assert!(panel_of(&mut app, kind).highlight.is_empty());
    }
}

/// The panel menu's Select All Unused highlights, in both panels, the styles no text uses, never
/// the Normal style; Delete then deletes them in one undo step.
#[test]
fn select_all_unused_highlights_the_unused_styles_for_delete() {
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, _) = several_app();
        let ctx = ui_context();
        act(&mut app, kind, TextStyleAction::SelectUnused);
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["A", "C", "D"], "{} Styles", kind.what());
        let undo = undo_len(&app);
        act(&mut app, kind, TextStyleAction::DeleteHighlighted);
        let left: Vec<String> = styles(&mut app, kind).into_iter().map(|s| s.0).collect();
        assert_eq!(left[1..], ["B"], "{} Styles: the unused styles are deleted", kind.what());
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
        assert!(highlighted_now(&mut app, &ctx, kind).is_empty());
        // With every style used, nothing is highlighted.
        act(&mut app, kind, TextStyleAction::SelectUnused);
        assert!(highlighted_now(&mut app, &ctx, kind).is_empty());
    }
}

/// With several styles highlighted, Duplicate copies each of them in one undo step and highlights the copies.
#[test]
fn panel_menu_duplicate_copies_every_highlighted_style() {
    for kind in [Kind::Char, Kind::Para] {
        let what = kind.what();
        let (mut app, _) = several_app();
        let ctx = ui_context();
        act(&mut app, kind, TextStyleAction::SelectUnused);
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["A", "C", "D"], "{what} Styles");
        let before = styles(&mut app, kind);
        let undo = undo_len(&app);
        act(&mut app, kind, TextStyleAction::DuplicateHighlighted);
        let names: Vec<String> = styles(&mut app, kind).into_iter().skip(1).map(|s| s.0).collect();
        assert_eq!(names, ["A", "B", "C", "D", "A copy", "C copy", "D copy"], "{what} Styles: A, C and D are duplicated");
        assert_eq!(undo_len(&app), undo + 1, "{what} Styles: one undo step");
        assert_eq!(highlighted_now(&mut app, &ctx, kind), ["A copy", "C copy", "D copy"], "{what} Styles: the copies are highlighted");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(styles(&mut app, kind), before);
    }
}

/// The panel lists the styles, and counts their uses, once per document version: more passes of an
/// unchanged document list nothing again, and an edit lists them again.
#[test]
fn the_styles_are_listed_once_per_document_version() {
    for kind in [Kind::Char, Kind::Para] {
        let what = kind.what();
        let (mut app, _) = several_app();
        let ctx = ui_context();
        let listed = || LISTED.with(|n| n.get());
        draw(&mut app, &ctx, kind);
        let first = listed();
        for _ in 0..4 {
            draw(&mut app, &ctx, kind);
        }
        assert_eq!(listed(), first, "{what} Styles: unchanged passes list nothing again");
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "E", "attrs": {}})).unwrap();
        draw(&mut app, &ctx, kind);
        assert_eq!(listed(), first + 1, "{what} Styles: an edit lists the styles again");
        assert_eq!(styles(&mut app, kind).last().map(|s| s.0.as_str()), Some("E"), "{what} Styles: the new style is listed");
    }
}

/// The style names of `id`'s runs (`Kind::Para`: of its paragraphs).
fn frame_style_names(app: &VectorcraftApp, kind: Kind, id: u64) -> Vec<Option<String>> {
    match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) if kind == Kind::Char => t.runs.iter().map(|r| r.style.style_name.clone()).collect(),
        NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| t.para_at(i).style_name.clone()).collect(),
        _ => panic!("not text"),
    }
}

/// With the Selection tool a selected frame of threaded text stands for its whole story, in both
/// panels: no style is current while a locked or hidden frame has another style, Apply styles every
/// frame in one undo step and leaves other type alone, the style shows as overridden while a hidden
/// frame overrides it, and Apply with clear overrides clears that.
#[test]
fn selection_tool_styles_reach_every_frame_of_the_story() {
    for kind in [Kind::Char, Kind::Para] {
        let what = kind.what();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let story = "The quick brown fox jumps over the lazy dog.\n".repeat(12);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
        let other = app.run("text.create", json!({"x": 10, "y": 250, "text": "Other", "size": 12})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [b]})).unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.run("select.set", json!({"ids": [c]})).unwrap();
        app.run("object.hide", json!({})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Plain"})).unwrap();
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Second", "attrs": {}})).unwrap();
        app.run(&format!("{}.apply", kind.prefix()), json!({"name": "Second", "ids": [c]})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        for id in [a, b, c] {
            assert!(!frame_style_names(&app, kind, id).is_empty(), "frame {id} holds text");
        }
        assert_eq!(current(&app, kind), None, "{what} Styles: the hidden frame has another style");

        let plain = style_id(&mut app, kind, "Plain");
        let undo = undo_len(&app);
        act(&mut app, kind, TextStyleAction::apply(plain, false));
        assert_eq!(undo_len(&app), undo + 1, "{what} Styles: one undo step");
        for id in [a, b, c] {
            assert!(frame_style_names(&app, kind, id).iter().all(|n| n.as_deref() == Some("Plain")), "{what} Styles: frame {id}");
        }
        assert!(frame_style_names(&app, kind, other).iter().all(Option::is_none), "{what} Styles: other type keeps its style");
        assert_eq!(current(&app, kind), Some(Some("Plain".to_string())));
        assert!(!overridden(&mut app, kind, "Plain"));

        app.run("text.setFormat", json!({"ids": [c], "kerning": 40, "composer": "singleLine"})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        assert!(overridden(&mut app, kind, "Plain"), "{what} Styles: the hidden frame's override shows");
        act(&mut app, kind, TextStyleAction::apply(plain, true));
        assert!(!overridden(&mut app, kind, "Plain"), "{what} Styles: clearing overrides reaches the hidden frame");
        app.run("edit.undo", json!({})).unwrap();
        assert!(overridden(&mut app, kind, "Plain"), "{what} Styles: undo brings the override back");
    }
}

/// The panel menus' Load Character Styles… (Load Paragraph Styles…) add that kind of style from
/// the picked document, and Load All Styles… adds both kinds, each in one undo step. A cancelled pick
/// loads nothing.
#[test]
fn panel_menus_load_the_styles_of_a_picked_document() {
    let mut src = vectorcraft_engine::Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    src.execute("charStyle.new", &json!({"name": "Lead", "attrs": {"size": 30.0}})).unwrap();
    src.execute("paraStyle.new", &json!({"name": "Centered", "attrs": {"justify": "Center"}})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-panel-load-styles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Styles.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save(&src.doc().unwrap().doc, false)).unwrap();
    let picked = path.to_string_lossy().to_string();
    let names = |app: &mut VectorcraftApp, kind| styles(app, kind).into_iter().skip(1).map(|s| s.0).collect::<Vec<_>>();
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, _) = several_app();
        let ctx = ui_context();
        let picker = scripted(&mut app);
        let one = if kind == Kind::Char { "ui.loadCharStyles" } else { "ui.loadParaStyles" };
        let doc = app.session.doc().unwrap().doc.clone();
        app.run(one, json!({})).unwrap();
        picker.cancel();
        poll(&mut app, &ctx);
        assert_eq!(app.session.doc().unwrap().doc, doc, "{} Styles: a cancelled pick loads nothing", kind.what());

        let undo = undo_len(&app);
        app.run(one, json!({})).unwrap();
        picker.answer(&[picked.as_str()]);
        poll(&mut app, &ctx);
        let (added, other) = if kind == Kind::Char { ("Lead", Kind::Para) } else { ("Centered", Kind::Char) };
        assert_eq!(names(&mut app, kind), ["A", "B", "C", "D", added], "{} Styles", kind.what());
        assert_eq!(names(&mut app, other), ["A", "B", "C", "D"], "{} Styles: the other kind", kind.what());
        assert_eq!(undo_len(&app), undo + 1);
        app.run("ui.loadAllStyles", json!({})).unwrap();
        picker.answer(&[picked.as_str()]);
        poll(&mut app, &ctx);
        assert_eq!(names(&mut app, Kind::Char), ["A", "B", "C", "D", "Lead"], "{} Styles: Load All Styles", kind.what());
        assert_eq!(names(&mut app, Kind::Para), ["A", "B", "C", "D", "Centered"], "{} Styles: Load All Styles", kind.what());
        assert_eq!(undo_len(&app), undo + 2);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Alt on New Character Style asks for the name a plain New's style gets and adds no style yet.
/// Cancel adds none. OK makes the style with the typed name from the Type tool's selected
/// characters, in one undo step, and a taken or empty name keeps the dialog open.
#[test]
fn alt_clicking_new_character_style_asks_for_its_name() {
    let (mut app, id, _ctx) = capture_app();
    select_type_range(&mut app, id, 3, 5);
    let names = |app: &VectorcraftApp| app.session.doc().unwrap().doc.char_styles.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let before = names(&app);
    act(&mut app, Kind::Char, TextStyleAction::new_style(true, true));
    let fields = fields_of(&app, "newTextStyle").expect("New with a name opens a dialog");
    assert_eq!(fields["name"], "Character Style 1");
    assert_eq!(names(&app), before, "no style is added before OK");
    answer(&mut app, "newTextStyle", &[], false);
    assert!(!is_open(&app, "newTextStyle"));
    assert_eq!(names(&app), before, "Cancel adds no style");

    act(&mut app, Kind::Char, TextStyleAction::new_style(true, true));
    confirm_new(&mut app, "Untouched");
    assert!(is_open(&app, "newTextStyle"), "a taken name keeps the dialog open");
    assert!(app.ui.text_styles.made.is_empty(), "a taken name does not request a highlight");
    confirm_new(&mut app, "");
    assert!(is_open(&app, "newTextStyle"), "a name is required");
    assert!(app.ui.text_styles.made.is_empty(), "an empty name does not request a highlight");
    let undo = undo_len(&app);
    confirm_new(&mut app, "Heading");
    assert!(!is_open(&app, "newTextStyle"));
    let doc = &app.session.doc().unwrap().doc;
    let heading = doc.char_styles.iter().find(|s| s.name == "Heading").expect("OK makes the style");
    assert_eq!(heading.attrs.get("size").and_then(Value::as_f64), Some(34.0), "made from the selected characters");
    assert_eq!(doc.char_styles.len(), before.len() + 1);
    assert_eq!(undo_len(&app), undo + 1);
    assert_eq!(text_editing(&app), Some((id, 3, 5)), "the Type selection stays");
}

#[test]
fn new_character_style_without_selection_uses_japanese_defaults() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let ctx = ui_context();
    app.ui_ctx = Some(ctx.clone());
    app.session.prefs.interface_language = "ja".into();
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    assert!(app.session.japanese_interface());

    for (name, size) in [("Default", None), ("Edited", Some(18))] {
        act(&mut app, Kind::Char, TextStyleAction::new_style(false, true));
        assert_eq!(fields_of(&app, "newTextStyle").unwrap()["charAlign"], "emBoxCenter");
        let mut fields = vec![("name", json!(name))];
        if let Some(size) = size {
            fields.push(("size", json!(f64::from(size))));
        }
        answer(&mut app, "newTextStyle", &fields, true);
        let style = app.session.doc().unwrap().doc.char_styles.iter().find(|style| style.name == name).unwrap();
        assert_eq!(style.attrs.get("charAlign"), Some(&json!("emBoxCenter")));
        assert_eq!(style.attrs.get("size").cloned(), size.map(|value| json!(f64::from(value))));
    }
    act(&mut app, Kind::Char, TextStyleAction::new_style(false, true));
    let id = open_dialogs(&app).into_iter().find(|d| d.key.as_deref() == Some("newTextStyle")).map(|d| d.id).unwrap();
    app.dialogs().unwrap().with(|host| {
        host.set_json_field(&ctx, id, "tab", json!("basic")).unwrap();
        host.reset_by_id(&ctx, id);
    });
    assert_ne!(fields_of(&app, "newTextStyle").unwrap().get("charAlign"), Some(&json!("emBoxCenter")), "Reset Panel drops charAlign");
    answer(&mut app, "newTextStyle", &[], false);

    let defaults = vectorcraft_engine::cmd::textstyles::default_new_style_attrs(&app.session, true);
    act(&mut app, Kind::Para, TextStyleAction::new_style(false, true));
    let edited = app.dialogs().unwrap().with(|host| host.get_mut::<crate::dialogs::new_text_style::NewTextStyle>().map(|d| d.edited()));
    assert_eq!(edited, Some(Ok(defaults.clone())), "the dialog opens with the paragraph defaults");
    confirm_new(&mut app, "Lead");
    let style = app.session.doc().unwrap().doc.para_styles.iter().find(|style| style.name == "Lead").unwrap();
    assert_eq!(style.attrs, defaults, "the new style has the paragraph defaults for the interface language");
}

/// The panel highlights the style the dialog made once its own `show` pass runs; the other kind's
/// pass leaves it pending.
#[test]
fn a_style_made_in_the_new_style_dialog_is_highlighted() {
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, id, ctx) = capture_app();
        if kind == Kind::Para {
            app.run("paraStyle.new", json!({"name": "Untouched"})).unwrap();
        }
        select_type_range(&mut app, id, 3, 5);
        highlight(&mut app, &ctx, kind, &["Untouched"]);
        act(&mut app, kind, TextStyleAction::new_style(true, true));
        confirm_new(&mut app, "Heading");
        let other = if kind == Kind::Char { Kind::Para } else { Kind::Char };
        draw(&mut app, &ctx, other);
        assert!(panel_of(&mut app, other).highlight.is_empty(), "{}: the other panel highlights nothing", kind.what());
        assert_eq!(
            app.ui.text_styles.made.first().map(|(_, _, name)| name.as_str()),
            Some("Heading"),
            "{}: the other panel leaves the new style pending",
            kind.what()
        );
        draw(&mut app, &ctx, kind);
        assert_eq!(panel_of(&mut app, kind).highlight, ["Heading"], "{}: the new style replaces the old highlight", kind.what());
        assert!(app.ui.text_styles.made.is_empty(), "{}: the panel consumes its pending highlight", kind.what());
    }
}

#[test]
fn new_character_and_paragraph_style_highlights_wait_for_their_panels() {
    let (mut app, id, ctx) = capture_app();
    select_type_range(&mut app, id, 3, 5);
    for (kind, name) in [(Kind::Char, "Heading Char"), (Kind::Para, "Heading Para")] {
        act(&mut app, kind, TextStyleAction::new_style(true, true));
        confirm_new(&mut app, name);
    }
    draw(&mut app, &ctx, Kind::Char);
    assert_eq!(app.ui.text_styles.char.highlight, ["Heading Char"]);
    assert_eq!(app.ui.text_styles.made.len(), 1);
    draw(&mut app, &ctx, Kind::Para);
    assert_eq!(app.ui.text_styles.para.highlight, ["Heading Para"]);
    assert!(app.ui.text_styles.made.is_empty());
}

/// A style made in one document is highlighted only when that document's panel draws.
#[test]
fn a_new_style_dialog_highlight_waits_for_its_document() {
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, id, ctx) = capture_app();
        let first_uid = app.session.active().unwrap().uid;
        select_type_range(&mut app, id, 3, 5);
        act(&mut app, kind, TextStyleAction::new_style(true, true));
        confirm_new(&mut app, "Heading");
        assert_eq!(app.ui.text_styles.made.first().map(|(uid, _, _)| *uid), Some(first_uid));

        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Heading"})).unwrap();
        draw(&mut app, &ctx, kind);
        assert!(panel_of(&mut app, kind).highlight.is_empty(), "{}: the second document highlights nothing", kind.what());
        assert!(!app.ui.text_styles.made.is_empty(), "{}: the first document's highlight stays pending", kind.what());

        assert!(app.session.set_active(0));
        draw(&mut app, &ctx, kind);
        assert_eq!(panel_of(&mut app, kind).highlight, ["Heading"], "{}: the first document is highlighted", kind.what());
        assert!(app.ui.text_styles.made.is_empty());
    }
}

#[test]
fn a_closed_document_drops_its_pending_style_highlight() {
    let (mut app, id, ctx) = capture_app();
    select_type_range(&mut app, id, 3, 5);
    act(&mut app, Kind::Char, TextStyleAction::new_style(true, true));
    confirm_new(&mut app, "Heading");

    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("charStyle.new", json!({"name": "Heading"})).unwrap();
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    draw(&mut app, &ctx, Kind::Char);
    assert!(app.ui.text_styles.char.highlight.is_empty());
    assert!(app.ui.text_styles.made.is_empty(), "the closed document's highlight is discarded");
}

/// OK makes the style from the text's attributes as they were when the dialog opened: a later
/// format change of the Type selection, or a change of the selected objects, does not reach it.
#[test]
fn new_style_dialog_keeps_the_attributes_from_when_it_opened() {
    let (mut app, id, _ctx) = capture_app();
    select_type_range(&mut app, id, 3, 5);
    act(&mut app, Kind::Char, TextStyleAction::new_style(true, true));
    app.run("text.setRangeStyle", json!({"id": id.0, "start": 2, "end": 6, "size": 50})).unwrap();
    confirm_new(&mut app, "Range");

    app.session.select_tool("selection", Default::default()).unwrap();
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    assert_eq!(text_editing(&app), None);
    act(&mut app, Kind::Char, TextStyleAction::new_style(true, true));
    let count = app.session.doc().unwrap().doc.char_styles.len();
    app.run("text.setRangeStyle", json!({"id": id.0, "start": 0, "end": 2, "size": 60})).unwrap();
    confirm_new(&mut app, "Object");

    let doc = &app.session.doc().unwrap().doc;
    assert_eq!(doc.char_styles.len(), count + 1);
    let size = |n: &str| doc.char_styles.iter().find(|s| s.name == n).and_then(|s| s.attrs.get("size")).and_then(Value::as_f64);
    assert_eq!((size("Range"), size("Object")), (Some(34.0), Some(18.0)));
}

fn create_redefined_texts(app: &mut VectorcraftApp, ctx: &egui::Context, id: NodeId) -> (Option<NodeId>, Option<NodeId>) {
    let mut follower = None;
    let mut local_override = None;

    app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24}})).unwrap();
    pick_row(app, ctx, "Body");
    for (slot, text, y, size) in [(&mut follower, "Follower", 130, 24), (&mut local_override, "Override", 160, 48)] {
        let user = NodeId(app.run("text.create", json!({"x": 20, "y": y, "text": text, "size": 24})).unwrap()["id"].as_u64().unwrap());
        app.run("charStyle.apply", json!({"name": "Body", "id": user.0})).unwrap();
        app.run("text.setRangeStyle", json!({"id": user.0, "start": 0, "end": text.len(), "size": size})).unwrap();
        *slot = Some(user);
    }
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    // This override is already the captured size, so Redefine does not alter the edited
    // runs and accidentally make TextTool::replace restart the typing interaction.
    app.run("text.setRangeStyle", json!({"id": id.0, "start": 0, "end": 6, "size": 18})).unwrap();
    (follower, local_override)
}

/// Undo and Redo each commit one distinct typing or style-capture history entry.
fn assert_typing_undo_redo(
    app: &mut VectorcraftApp,
    undo_states: [&std::sync::Arc<vectorcraft_doc::Document>; 3],
    redo_states: [&std::sync::Arc<vectorcraft_doc::Document>; 3],
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    assert_typing_undo(app, undo_states, selection, undo);
    assert_typing_redo(app, redo_states, selection, undo);
}

fn assert_typing_undo(
    app: &mut VectorcraftApp,
    states: [&std::sync::Arc<vectorcraft_doc::Document>; 3],
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    for (step, expected) in states.into_iter().enumerate() {
        let result = app.run("edit.undo", json!({})).unwrap();
        if step != 1 {
            assert_eq!(result["undone"], json!("Typing"));
        }
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.as_ref(), expected.as_ref());
        assert_eq!(&st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 2 - step, step + 1));
        assert_eq!(st.is_dirty(), step != 2);
        assert!(!app.session.in_interaction());
        assert_eq!(app.session.tool_options()["typing"], json!(false));
    }
}

fn assert_typing_redo(
    app: &mut VectorcraftApp,
    states: [&std::sync::Arc<vectorcraft_doc::Document>; 3],
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    for (step, expected) in states.into_iter().enumerate() {
        let result = app.run("edit.redo", json!({})).unwrap();
        if step != 1 {
            assert_eq!(result["redone"], json!("Typing"));
        }
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.as_ref(), expected.as_ref());
        assert_eq!(&st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + step + 1, 2 - step));
        assert!(st.is_dirty());
    }
}

fn capture_during_live_typing(redefine: bool) {
    let (mut app, id, ctx) = capture_app();
    let (follower, local_override) = if redefine { create_redefined_texts(&mut app, &ctx, id) } else { (None, None) };
    let link = redefine.then(|| "Body".to_string());
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo = undo_len(&app);
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert_eq!(text_editing(&app), Some((id, 0, 6)));
    // The canvas's text editor takes the keyboard in the first frame that shows it.
    key_frame(&mut app, &ctx, Vec::new());
    key_frame(&mut app, &ctx, vec![egui::Event::Text("Typed".into())]);
    assert_eq!(text_runs(&app, id), vec![("Typed".into(), 18.0, link.clone())]);
    assert_eq!(text_editing(&app), Some((id, 5, 5)));
    assert!(app.session.in_interaction());
    assert_eq!(app.session.tool_options()["typing"], json!(true));
    assert_eq!(undo_len(&app), undo);
    let typed = app.session.doc().unwrap().doc.clone();

    // Ctrl+A/Cmd+A is Select All's real command route, including key release; it must leave
    // the typing interaction live until the style action runs.
    for pressed in [true, false] {
        key_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key { key: egui::Key::A, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND }],
        );
    }
    assert_eq!(text_editing(&app), Some((id, 0, 5)));
    assert!(app.session.in_interaction());
    assert_eq!(app.session.tool_options()["typing"], json!(true));
    assert_eq!(app.session.doc().unwrap().selection, selection);
    capture_style_from_typing(&mut app, redefine);
    let captured = app.session.doc().unwrap().doc.clone();
    let capture_history = (app.session.doc().unwrap().history.undo.len(), app.session.doc().unwrap().history.redo.len());
    let capture_settled = !app.session.in_interaction() && app.session.tool_options()["typing"] == json!(false);
    let name = if redefine { "Body" } else { "Character Style 1" };
    let definition = captured.char_styles.iter().find(|s| s.name == name).unwrap();
    assert_eq!(definition.attrs.get("size").and_then(Value::as_f64), Some(18.0));
    assert_eq!(captured.char_styles.len(), typed.char_styles.len() + usize::from(!redefine));
    assert_eq!(text_runs(&app, id), vec![("Typed".into(), 18.0, link.clone())]);
    assert_eq!(app.session.doc().unwrap().selection, selection);
    typed.walk(|node| {
        if matches!(&node.kind, NodeKind::Text(_)) && Some(node.id) != follower {
            assert_eq!(captured.node(node.id), Some(node));
        }
    });
    assert_related_text_styles(&app, follower, local_override);

    // Check persistence before the history assertions: RED must expose the lost definition,
    // not merely report that typing was still live after capture.
    key_frame(&mut app, &ctx, vec![egui::Event::Text("After".into())]);
    let after = app.session.doc().unwrap().doc.clone();
    assert_eq!(text_runs(&app, id), vec![("After".into(), 18.0, link)]);
    assert_eq!(after.char_styles, captured.char_styles, "further typing must preserve the captured definition");
    captured.walk(|node| {
        if node.id != id && matches!(&node.kind, NodeKind::Text(_)) {
            assert_eq!(after.node(node.id), Some(node), "further typing preserves followers and unrelated text");
        }
    });
    assert!(capture_settled, "capture ends the prior typing interaction without ending text editing");
    assert_eq!(capture_history, (undo + 2, 0), "typing and capture are separate history entries");
    assert_eq!(text_editing(&app), Some((id, 5, 5)));
    assert_eq!(app.session.doc().unwrap().selection, selection);
    assert!(app.session.in_interaction(), "further text starts its own typing interaction");
    assert_eq!(app.session.tool_options()["typing"], json!(true));
    assert_eq!(undo_len(&app), undo + 2);

    // Undo itself commits the new live typing interaction. Three separate snapshots prove
    // that typing, capture and further typing can each be undone/redone independently.
    assert_typing_undo_redo(&mut app, [&captured, &typed, &before], [&typed, &captured, &after], &selection, undo);
}

fn capture_style_from_typing(app: &mut VectorcraftApp, redefine: bool) {
    let action = if redefine { TextStyleAction::Redefine(style_id(app, Kind::Char, "Body")) } else { TextStyleAction::new_style(true, false) };
    act(app, Kind::Char, action);
}

fn assert_related_text_styles(app: &VectorcraftApp, follower: Option<NodeId>, local_override: Option<NodeId>) {
    if let Some(user) = follower {
        assert_eq!(text_runs(app, user), vec![("Follower".into(), 18.0, Some("Body".into()))]);
    }
    if let Some(user) = local_override {
        assert_eq!(text_runs(app, user), vec![("Override".into(), 48.0, Some("Body".into()))]);
    }
}

#[test]
fn new_character_style_survives_live_typing_and_undo_redo() {
    capture_during_live_typing(false);
}

#[test]
fn redefine_character_style_survives_live_typing_and_undo_redo() {
    capture_during_live_typing(true);
}

#[test]
fn character_style_boundary_new_from_menu() {
    capture_from_ui(Some((2, 5)), false, false);
}

#[test]
fn character_style_boundary_redefine_from_menu() {
    capture_from_ui(Some((2, 5)), true, false);
}

#[test]
fn character_style_unicode_boundary_capture_controls() {
    // Byte 2 is the start of 漢 after é; 2..5 selects exactly that complete character.
    for redefine in [false, true] {
        capture_from_ui(Some((2, 5)), redefine, true);
    }
}
