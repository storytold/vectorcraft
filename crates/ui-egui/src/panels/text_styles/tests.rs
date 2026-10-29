use super::*;
use vectorcraft_doc::{NodeId, NodeKind};

use super::super::tests_appearance::{click, frame_events, frame_raw, text_rect};

#[test]
fn panels_draw_and_apply_headless() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.session.execute("charStyle.new", &json!({"name": "Big", "attrs": {"size": 30.0}})).unwrap();
    for kind in [Kind::Char, Kind::Para] {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui, kind);
            menu(&mut app, ui, kind);
        });
        out.textures_delta.clear();
    }
    app.run("charStyle.apply", target(&app, Kind::Char, json!({"name": "Big"}))).unwrap();
    let (name, attrs) = current(&app, Kind::Char).unwrap();
    assert_eq!((name.as_deref(), attrs["size"].as_f64()), (Some("Big"), Some(30.0)));
}

#[test]
fn auto_leading_style_marks_only_real_overrides() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("charStyle.new", json!({"name": "Automatic", "attrs": {"leading": null}})).unwrap();
    app.run("charStyle.apply", json!({"name": "Automatic"})).unwrap();
    for (leading, label) in [(json!("auto"), "Automatic"), (json!(30), "Automatic+")] {
        app.run("text.setStyle", json!({"leading": leading})).unwrap();
        let shown = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui, Kind::Char));
        assert!(shown.lines().any(|line| line == label), "expected {label} in {shown}");
    }
    app.run("charStyle.apply", json!({"name": "Automatic", "clearOverrides": true})).unwrap();
    let shown = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui, Kind::Char));
    assert!(shown.lines().any(|line| line == "Automatic"), "no override after clearing: {shown}");
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
    for (change, label) in [(None, "Plain"), (Some(json!({"kerning": 40, "composer": "singleLine"})), "Plain+")] {
        if let Some(p) = change {
            app.run("text.setFormat", p).unwrap();
        }
        for kind in [Kind::Char, Kind::Para] {
            let shown = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui, kind));
            assert!(shown.lines().any(|line| line == label), "expected {label} in the {} styles: {shown}", kind.what());
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

/// Drag the row labeled `label` in `kind`'s panel onto the footer button whose center is
/// `from_right` points left of the panel's right edge (Create New Style: 40, Delete: 12), and
/// release it there. `other`: once the drag has started, a row of the other panel is dragged in its
/// place. → whether the button was outlined while the row was held over it.
fn drag_row_onto_footer(ctx: &egui::Context, app: &mut VectorcraftApp, kind: Kind, label: &str, from_right: f32, other: bool) -> bool {
    let button = std::cell::Cell::new(egui::Pos2::ZERO);
    let mut draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| {
        ui.set_width(280.0);
        let right = ui.max_rect().right();
        show(app, ui, kind);
        button.set(egui::pos2(right - from_right, ui.min_rect().bottom() - 12.0));
    };
    frame_events(ctx, app, vec![], &mut draw);
    let row = text_rect(&frame_events(ctx, app, vec![], &mut draw), label).center();
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame_events(ctx, app, vec![egui::Event::PointerMoved(row), press(row, true)], &mut draw);
    frame_events(ctx, app, vec![egui::Event::PointerMoved(row + vec2(0.0, 12.0))], &mut draw);
    assert!(egui::DragAndDrop::payload::<RowDrag>(ctx).is_some_and(|d| d.kind == kind && d.name == label), "the row is dragged");
    if other {
        let kind = if kind == Kind::Char { Kind::Para } else { Kind::Char };
        egui::DragAndDrop::set_payload(ctx, RowDrag { kind, name: label.to_string() });
    }
    let to = button.get();
    frame_events(ctx, app, vec![egui::Event::PointerMoved(to)], &mut draw);
    let accent = Tokens::get(ctx).accent;
    let mut out = ctx.run_ui(egui::RawInput { events: vec![egui::Event::PointerMoved(to)], ..Default::default() }, |ui| draw(app, ui));
    out.textures_delta.clear();
    let outlined = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.stroke.color == accent && r.rect.contains(to)));
    frame_events(ctx, app, vec![press(to, false)], &mut draw);
    frame_events(ctx, app, vec![], &mut draw);
    assert!(egui::DragAndDrop::payload::<RowDrag>(ctx).is_none(), "the drag ended");
    outlined
}

/// A style row dropped on Create New Style is duplicated as `<name> copy` in one undo step, the
/// button is outlined while the row is held over it, and the panel then highlights the copy. A row
/// of the other panel is not accepted.
#[test]
fn dragging_a_style_onto_create_new_style_duplicates_it() {
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, _) = drag_app();
        let ctx = egui::Context::default();
        let before = styles(&mut app, kind);
        let undo = app.session.doc().unwrap().history.undo.len();
        let commands = app.run_count;
        assert!(
            !drag_row_onto_footer(&ctx, &mut app, kind, "Body", 40.0, true),
            "{} Styles: New is not outlined for the other panel's row",
            kind.what()
        );
        assert_eq!((app.run_count, styles(&mut app, kind)), (commands, before.clone()), "New accepts no row of the other panel");
        assert!(drag_row_onto_footer(&ctx, &mut app, kind, "Body", 40.0, false), "{} Styles: New is outlined", kind.what());
        let after = styles(&mut app, kind);
        let body = before.iter().find(|s| s.0 == "Body").unwrap();
        let copy = after.iter().find(|s| s.0 == "Body copy");
        assert_eq!(copy.map(|s| &s.1), Some(&body.1), "{} Styles: the copy has Body's attributes: {after:?}", kind.what());
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        let selected: Option<String> = pstate(&ctx, kind.key());
        assert_eq!(selected.as_deref(), Some("Body copy"));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(styles(&mut app, kind), before);
    }
}

/// A style row dropped on Delete deletes that style in one undo step, also while no style is
/// selected in the panel and the button is dimmed; the type keeps its look. Normal dropped on
/// Delete stays, and Delete is not outlined for it or for a row of the other panel.
#[test]
fn dragging_a_style_onto_delete_deletes_it() {
    for kind in [Kind::Char, Kind::Para] {
        let (mut app, id) = drag_app();
        let ctx = egui::Context::default();
        let before = styles(&mut app, kind);
        let undo = app.session.doc().unwrap().history.undo.len();
        assert_eq!(pstate::<Option<String>>(&ctx, kind.key()), None, "no style is selected in the panel");
        let commands = app.run_count;
        assert!(
            !drag_row_onto_footer(&ctx, &mut app, kind, "Body", 12.0, true),
            "{} Styles: Delete is not outlined for the other panel's row",
            kind.what()
        );
        assert_eq!((app.run_count, styles(&mut app, kind)), (commands, before.clone()), "Delete accepts no row of the other panel");
        assert!(drag_row_onto_footer(&ctx, &mut app, kind, "Body", 12.0, false), "{} Styles: Delete is outlined", kind.what());
        let after = styles(&mut app, kind);
        assert_eq!(after.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), [before[0].0.as_str()], "{} Styles: Body is deleted", kind.what());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert_eq!(text_runs(&app, id)[0].1, 18.0, "the type keeps its look");
        let normal = super::super::label_or_name(&before[0].0, true).to_string();
        let commands = app.run_count;
        assert!(!drag_row_onto_footer(&ctx, &mut app, kind, &normal, 12.0, false), "{} Styles: Delete is not outlined for Normal", kind.what());
        assert_eq!((app.run_count, styles(&mut app, kind)), (commands, after), "Normal stays");
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
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
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

/// Click production panel/menu content with the existing hover/press/release helper.
/// A live range guard and command counter distinguish a dispatch failure from a missed click or lost Type selection.
fn click_style_ui(ctx: &egui::Context, app: &mut VectorcraftApp, id: NodeId, range: Option<(usize, usize)>, is_menu: bool, label: &str) {
    let expected = range.map(|(start, end)| (id, start, end));
    let mut expected_target = json!({});
    if let Some((start, end)) = range {
        expected_target["id"] = json!(id.0);
        if end > start {
            expected_target["start"] = json!(start);
            expected_target["end"] = json!(end);
        }
    }
    let commands = app.run_count;
    let mut dispatched = false;
    let mut draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| {
        // On the release frame this is immediately before the production menu/row handles the click and builds its target.
        assert_eq!(text_editing(app), expected, "Type selection before {label}");
        assert_eq!(target(app, Kind::Char, json!({})), expected_target, "actual panel target before {label}");
        let before = app.run_count;
        if is_menu {
            menu(app, ui, Kind::Char);
        } else {
            show(app, ui, Kind::Char);
        }
        if app.run_count != before {
            assert!(!dispatched, "{label} dispatched more than once");
            assert_eq!(app.run_count, before.wrapping_add(1), "one command from {label}");
            dispatched = true;
        }
    };
    frame_events(ctx, app, vec![], &mut draw);
    let texts = frame_events(ctx, app, vec![], &mut draw);
    // widgets::menu_item prefixes an unchecked label with three spaces; the panel row does not.
    let painted = if is_menu { format!("   {label}") } else { label.to_string() };
    let pos = text_rect(&texts, &painted).center();
    click(ctx, app, pos, &mut draw);
    assert!(dispatched, "pointer click did not dispatch {label}; do not interpret this as parameter rejection");
    assert_eq!(app.run_count, commands.wrapping_add(1), "exactly one real panel command");
    assert_eq!(text_editing(app), expected, "Type selection after {label}");
}

/// Click `label` where `draw` paints it (a style row of the panel, or a panel menu item with
/// its three-space prefix). → how many commands the click ran.
fn click_label(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, mut draw: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) -> u64 {
    frame_events(ctx, app, vec![], &mut draw);
    let texts = frame_events(ctx, app, vec![], &mut draw);
    let pos = text_rect(&texts, label).center();
    let before = app.run_count;
    click(ctx, app, pos, &mut draw);
    app.run_count.wrapping_sub(before)
}

/// The panel menu's Duplicate highlights the copy and its Delete clears the highlight, as the
/// panel's New and Delete buttons do; after a menu delete, Duplicate runs nothing.
#[test]
fn panel_menu_duplicate_and_delete_update_the_highlighted_style() {
    for kind in [Kind::Char, Kind::Para] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run(&format!("{}.new", kind.prefix()), json!({"name": "Body"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let what = kind.what();
        let has = |app: &mut VectorcraftApp, name: &str| styles(app, kind).iter().any(|s| s.0 == name);
        let highlighted = |ctx: &egui::Context| pstate::<Option<String>>(ctx, kind.key());
        click_label(&ctx, &mut app, "Body", |app, ui| show(app, ui, kind));
        assert_eq!(highlighted(&ctx).as_deref(), Some("Body"), "{what}: clicking the row highlights it");

        let duplicate = format!("   Duplicate {what} Style");
        assert_eq!(click_label(&ctx, &mut app, &duplicate, |app, ui| menu(app, ui, kind)), 1, "{what}: Duplicate runs");
        assert!(has(&mut app, "Body copy"), "{what}: the copy exists");
        assert_eq!(highlighted(&ctx).as_deref(), Some("Body copy"), "{what}: the menu's Duplicate highlights the copy");

        let delete = format!("   Delete {what} Style");
        assert_eq!(click_label(&ctx, &mut app, &delete, |app, ui| menu(app, ui, kind)), 1, "{what}: Delete runs");
        assert!(!has(&mut app, "Body copy"), "{what}: the copy is deleted");
        assert_eq!(highlighted(&ctx), None, "{what}: the menu's Delete clears the highlight");
        assert_eq!(click_label(&ctx, &mut app, &duplicate, |app, ui| menu(app, ui, kind)), 0, "{what}: Duplicate is disabled after the delete");
        assert!(has(&mut app, "Body"), "{what}: the original stays");
    }
}

/// In a panel scrolled to the top of a long list, the menu's Duplicate scrolls the copy's row into
/// view, and so does the New button's `highlight_made` for the new style.
#[test]
fn a_style_made_from_the_panel_is_scrolled_into_view() {
    for kind in [Kind::Char, Kind::Para] {
        let what = kind.what();
        for from_menu in [true, false] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            app.run(&format!("{}.new", kind.prefix()), json!({"name": "Body"})).unwrap();
            for i in 1..=12 {
                app.run(&format!("{}.new", kind.prefix()), json!({"name": format!("Item {i:02}")})).unwrap();
            }
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let panel = |app: &mut VectorcraftApp, ui: &mut egui::Ui| {
                egui::ScrollArea::vertical().id_salt("styles").max_height(80.0).show(ui, |ui| show(app, ui, kind));
            };
            let before = frame_events(&ctx, &mut app, vec![], panel);
            let body = text_rect(&before, "Body");
            let made = if from_menu {
                set_pstate(&ctx, kind.key(), Some("Body".to_string()));
                let duplicate = format!("   Duplicate {what} Style");
                assert_eq!(click_label(&ctx, &mut app, &duplicate, |app, ui| menu(app, ui, kind)), 1, "{what}: Duplicate runs");
                "Body copy".to_string()
            } else {
                // The New button's path: the style it made, highlighted with `highlight_made`.
                let name = app.run(&format!("{}.new", kind.prefix()), json!({})).unwrap()["name"].as_str().unwrap_or_default().to_string();
                highlight_made(&ctx, kind, Some(name.clone()));
                name
            };
            assert_eq!(pstate::<Option<String>>(&ctx, kind.key()).as_deref(), Some(made.as_str()), "{what}: the new style is highlighted");
            let mut after = vec![];
            for _ in 0..3 {
                let time = ctx.input(|i| i.time) + 1.0;
                after = frame_raw(&ctx, &mut app, egui::RawInput { time: Some(time), ..Default::default() }, panel);
            }
            let row = text_rect(&after, &made);
            let top = body.top() - 30.0;
            assert!(row.top() >= top && row.bottom() <= top + 80.0 + 30.0, "{what}, menu {from_menu}: {made} at {row:?}, panel from {top}");
        }
    }
}

/// A highlighted style renamed in Style Options (`<kind>.rename`) or deleted by a command is no
/// longer highlighted once the panel draws, so the panel menu's Duplicate runs nothing.
#[test]
fn a_highlighted_style_that_is_no_longer_listed_is_not_highlighted() {
    for kind in [Kind::Char, Kind::Para] {
        for (change, p) in [("rename", json!({"name": "Body", "to": "Heading"})), ("delete", json!({"name": "Body"}))] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            app.run(&format!("{}.new", kind.prefix()), json!({"name": "Body"})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let what = kind.what();
            click_label(&ctx, &mut app, "Body", |app, ui| show(app, ui, kind));
            assert_eq!(pstate::<Option<String>>(&ctx, kind.key()).as_deref(), Some("Body"), "{what}: clicking the row highlights it");
            app.run(&format!("{}.{change}", kind.prefix()), p).unwrap();
            frame_events(&ctx, &mut app, vec![], |app, ui| show(app, ui, kind));
            assert_eq!(pstate::<Option<String>>(&ctx, kind.key()), None, "{what}, {change}: the old name is not highlighted");
            let duplicate = format!("   Duplicate {what} Style");
            assert_eq!(click_label(&ctx, &mut app, &duplicate, |app, ui| menu(app, ui, kind)), 0, "{what}, {change}: Duplicate is disabled");
        }
    }
}

fn text_runs(app: &VectorcraftApp, id: NodeId) -> Vec<(String, f64, Option<String>)> {
    let st = app.session.doc().unwrap();
    let NodeKind::Text(text) = &st.doc.node(id).unwrap().kind else { panic!("expected text") };
    text.runs.iter().map(|r| (r.text.clone(), r.style.size, r.style.style_name.clone())).collect()
}

fn capture_from_menu(range: Option<(usize, usize)>, redefine: bool) {
    capture_from_ui(range, redefine, false, false);
}

fn capture_from_ui(range: Option<(usize, usize)>, redefine: bool, footer: bool, unicode: bool) {
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
        // Select the definition through its real panel row; no test-only panel selection is installed.
        click_style_ui(&ctx, &mut app, id, None, false, "Body");
        let selected: Option<String> = pstate(&ctx, Kind::Char.key());
        assert_eq!(selected.as_deref(), Some("Body"));
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
    let undo = app.session.doc().unwrap().history.undo.len();
    assert!(!app.session.doc().unwrap().is_dirty());
    let label = if redefine { "Redefine Character Style" } else { "New Character Style…" };
    if footer {
        click_new_style_footer_range(&ctx, &mut app, id, range.expect("footer fixture has a range"));
    } else {
        click_style_ui(&ctx, &mut app, id, range, true, label);
    }
    let after = app.session.doc().unwrap().doc.clone();
    let name = if redefine { "Body" } else { "Character Style 1" };
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
    capture_from_menu(Some((3, 5)), false);
}

#[test]
fn selected_type_range_redefine_character_style_from_menu() {
    capture_from_menu(Some((3, 5)), true);
}

#[test]
fn new_character_style_object_and_caret_controls() {
    // The current collapsed target omits start even at caret 3, so capture still uses object-start size 18.
    for range in [None, Some((3, 3))] {
        capture_from_menu(range, false);
    }
}

#[test]
fn redefine_character_style_object_and_caret_controls() {
    for range in [None, Some((3, 3))] {
        capture_from_menu(range, true);
    }
}

#[test]
fn character_style_range_apply_from_panel_control() {
    let (mut app, id, ctx) = capture_app();
    app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 26}})).unwrap();
    select_type_range(&mut app, id, 3, 5);
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    click_style_ui(&ctx, &mut app, id, Some((3, 5)), false, "Body");
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

/// Feed real text/shortcut events through the canvas Type routing. No focus reset or typing
/// settlement here: the capture dispatch itself must close the live typing interaction.
fn capture_input(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    frame_events(ctx, app, events, |app, ui| {
        crate::shortcuts::handle(app, ui.ctx());
        menu(app, ui, Kind::Char);
    });
}

fn click_new_style_footer(ctx: &egui::Context, app: &mut VectorcraftApp, id: NodeId) {
    click_new_style_footer_range(ctx, app, id, (0, 5));
}

fn click_new_style_footer_range(ctx: &egui::Context, app: &mut VectorcraftApp, id: NodeId, range: (usize, usize)) {
    let (start, end) = range;
    let pos = std::cell::Cell::new(egui::Pos2::ZERO);
    let commands = app.run_count;
    let mut dispatched = false;
    let mut draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| {
        assert_eq!(text_editing(app), Some((id, start, end)));
        assert_eq!(target(app, Kind::Char, json!({})), json!({"id": id.0, "start": start, "end": end}));
        ui.set_width(280.0);
        let right = ui.max_rect().right();
        let before = app.run_count;
        show(app, ui, Kind::Char);
        // The final row is the production bottom bar: New and Delete are its two 24-point
        // buttons, four points apart. Use its laid-out bottom/right, not screen coordinates.
        pos.set(egui::pos2(right - 40.0, ui.min_rect().bottom() - 12.0));
        if app.run_count != before {
            assert!(!dispatched, "footer New dispatched more than once");
            assert_eq!(app.run_count, before.wrapping_add(1));
            dispatched = true;
        }
    };
    frame_events(ctx, app, vec![], &mut draw);
    frame_events(ctx, app, vec![], &mut draw);
    click(ctx, app, pos.get(), &mut draw);
    assert!(dispatched, "pointer click did not dispatch footer New");
    assert_eq!(app.run_count, commands.wrapping_add(1));
    let selected: Option<String> = pstate(ctx, Kind::Char.key());
    assert_eq!(selected.as_deref(), Some("Character Style 1"), "production footer stores the new definition");
    assert_eq!(text_editing(app), Some((id, start, end)), "footer capture preserves the Type range");
}

/// Alt-click the production bottom bar's New button of the `kind` panel, then draw the dialog
/// it opened: every text the last frame drew.
fn alt_click_new(ctx: &egui::Context, app: &mut VectorcraftApp, kind: Kind) -> Vec<String> {
    click_new_with(ctx, app, kind, egui::Modifiers::ALT)
}

fn click_new(ctx: &egui::Context, app: &mut VectorcraftApp, kind: Kind) {
    click_new_with(ctx, app, kind, egui::Modifiers::NONE);
}

fn click_new_with(ctx: &egui::Context, app: &mut VectorcraftApp, kind: Kind, modifiers: egui::Modifiers) -> Vec<String> {
    let pos = std::cell::Cell::new(egui::Pos2::ZERO);
    let mut draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| {
        ui.set_width(280.0);
        let right = ui.max_rect().right();
        show(app, ui, kind);
        pos.set(egui::pos2(right - 40.0, ui.min_rect().bottom() - 12.0));
    };
    frame_events(ctx, app, vec![], &mut draw);
    frame_events(ctx, app, vec![], &mut draw);
    let at = pos.get();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    for e in [egui::Event::PointerMoved(at), button(true), button(false)] {
        frame_events(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), e], &mut draw);
    }
    // A window draws its content from its second frame.
    frame_events(ctx, app, vec![], |app, ui| crate::dialogs::show(app, ui.ctx()));
    frame_events(ctx, app, vec![], |app, ui| crate::dialogs::show(app, ui.ctx())).into_iter().map(|(s, _)| s).collect()
}

fn set_name(app: &mut VectorcraftApp, name: &str) {
    app.ui.dialog.as_mut().expect("the dialog is open").fields.insert("name".into(), json!(name));
}

/// Alt-click on New Character Style opens New Character Style with the name a plain click's
/// style gets, and adds no style yet. Cancel adds none. OK makes the style with the typed name
/// from the Type tool's selected characters, in one undo step, and a name another style has keeps
/// the dialog open.
#[test]
fn alt_clicking_new_character_style_asks_for_its_name() {
    let (mut app, id, ctx) = capture_app();
    select_type_range(&mut app, id, 3, 5);
    let names = |app: &VectorcraftApp| app.session.doc().unwrap().doc.char_styles.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let before = names(&app);
    let shown = alt_click_new(&ctx, &mut app, Kind::Char);
    let d = app.ui.dialog.as_ref().expect("Alt-click on New opens a dialog");
    assert_eq!((d.kind.as_str(), d.str("name").as_str()), (crate::dialogs::new_text_style::KIND, "Character Style 1"));
    assert!(shown.iter().any(|s| s == "New Character Style"), "the dialog's heading: {shown:?}");
    assert_eq!(names(&app), before, "no style is added before OK");
    crate::dialogs::cancel(&mut app);
    assert_eq!(names(&app), before, "Cancel adds no style");

    alt_click_new(&ctx, &mut app, Kind::Char);
    set_name(&mut app, "Untouched");
    assert!(crate::dialogs::confirm(&mut app).is_err(), "names are unique");
    assert!(app.ui.dialog.is_some(), "a taken name keeps the dialog open");
    set_name(&mut app, "Heading");
    let undo = app.session.doc().unwrap().history.undo.len();
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let doc = &app.session.doc().unwrap().doc;
    let heading = doc.char_styles.iter().find(|s| s.name == "Heading").expect("OK makes the style");
    assert_eq!(heading.attrs.get("size").and_then(Value::as_f64), Some(34.0), "made from the selected characters");
    assert_eq!(doc.char_styles.len(), before.len() + 1);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(text_editing(&app), Some((id, 3, 5)), "the Type selection stays");
}

/// OK makes the style from the text's attributes as they were when the dialog opened: a later
/// format change of the Type selection, or a change of the selected objects, does not reach it.
#[test]
fn new_style_dialog_keeps_the_attributes_from_when_it_opened() {
    let (mut app, id, ctx) = capture_app();
    select_type_range(&mut app, id, 3, 5);
    alt_click_new(&ctx, &mut app, Kind::Char);
    app.run("text.setRangeStyle", json!({"id": id.0, "start": 2, "end": 6, "size": 50})).unwrap();
    set_name(&mut app, "Range");
    crate::dialogs::confirm(&mut app).unwrap();

    app.session.select_tool("selection", Default::default()).unwrap();
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    assert_eq!(text_editing(&app), None);
    alt_click_new(&ctx, &mut app, Kind::Char);
    let count = app.session.doc().unwrap().doc.char_styles.len();
    app.run("text.setRangeStyle", json!({"id": id.0, "start": 0, "end": 2, "size": 60})).unwrap();
    set_name(&mut app, "Object");
    crate::dialogs::confirm(&mut app).unwrap();

    let doc = &app.session.doc().unwrap().doc;
    assert_eq!(doc.char_styles.len(), count + 1);
    let size = |n: &str| doc.char_styles.iter().find(|s| s.name == n).and_then(|s| s.attrs.get("size")).and_then(Value::as_f64);
    assert_eq!((size("Range"), size("Object")), (Some(34.0), Some(18.0)));
}

/// Alt-click on New Paragraph Style opens New Paragraph Style for the selected type's paragraph;
/// a plain click still makes the style at once.
#[test]
fn alt_clicking_new_paragraph_style_asks_for_its_name() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Body"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("text.setFormat", json!({"composer": "singleLine"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let shown = alt_click_new(&ctx, &mut app, Kind::Para);
    let d = app.ui.dialog.as_ref().expect("Alt-click on New opens a dialog");
    assert_eq!((d.kind.as_str(), d.str("name").as_str()), (crate::dialogs::new_text_style::KIND, "Paragraph Style 1"));
    assert!(shown.iter().any(|s| s == "New Paragraph Style"), "the dialog's heading: {shown:?}");
    assert!(app.session.doc().unwrap().doc.para_styles.is_empty(), "no style is added before OK");
    set_name(&mut app, "Body Copy");
    crate::dialogs::confirm(&mut app).unwrap();
    let (_, attrs) = current(&app, Kind::Para).unwrap();
    let doc = &app.session.doc().unwrap().doc;
    assert_eq!(doc.para_styles.iter().map(|s| (s.name.as_str(), &s.attrs)).collect::<Vec<_>>(), [("Body Copy", &attrs)]);

    // A plain click still makes "Paragraph Style 1" at once, with no dialog.
    click_new(&ctx, &mut app, Kind::Para);
    assert!(app.ui.dialog.is_none());
    let names: Vec<_> = app.session.doc().unwrap().doc.para_styles.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, ["Body Copy", "Paragraph Style 1"]);
}

fn create_redefined_texts(app: &mut VectorcraftApp, ctx: &egui::Context, id: NodeId) -> (Option<NodeId>, Option<NodeId>) {
    let mut follower = None;
    let mut local_override = None;

    app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24}})).unwrap();
    click_style_ui(ctx, app, id, None, false, "Body");
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
fn capture_during_live_typing(redefine: bool, footer: bool) {
    let (mut app, id, ctx) = capture_app();
    let (follower, local_override) = if redefine { create_redefined_texts(&mut app, &ctx, id) } else { (None, None) };
    let link = redefine.then(|| "Body".to_string());
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    app.session.select_tool("type", Default::default()).unwrap();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert_eq!(text_editing(&app), Some((id, 0, 6)));
    capture_input(&ctx, &mut app, vec![egui::Event::Text("Typed".into())]);
    assert_eq!(text_runs(&app, id), vec![("Typed".into(), 18.0, link.clone())]);
    assert_eq!(text_editing(&app), Some((id, 5, 5)));
    assert!(app.session.in_interaction());
    assert_eq!(app.session.tool_options()["typing"], json!(true));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    let typed = app.session.doc().unwrap().doc.clone();

    // Ctrl+A/Cmd+A is Select All's real command route, including key release; it must leave
    // the typing interaction live until the subsequent production menu dispatch.
    for pressed in [true, false] {
        capture_input(
            &ctx,
            &mut app,
            vec![egui::Event::Key { key: egui::Key::A, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND }],
        );
    }
    assert_eq!(text_editing(&app), Some((id, 0, 5)));
    assert!(app.session.in_interaction());
    assert_eq!(app.session.tool_options()["typing"], json!(true));
    assert_eq!(app.session.doc().unwrap().selection, selection);
    capture_style_from_typing(&mut app, &ctx, id, redefine, footer);
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
    capture_input(&ctx, &mut app, vec![egui::Event::Text("After".into())]);
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
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 2);

    // Undo itself commits the new live typing interaction. Three separate snapshots prove
    // that typing, capture and further typing can each be undone/redone independently.
    assert_typing_undo_redo(&mut app, [&captured, &typed, &before], [&typed, &captured, &after], &selection, undo);
}

fn capture_style_from_typing(app: &mut VectorcraftApp, ctx: &egui::Context, id: NodeId, redefine: bool, footer: bool) {
    let label = if redefine { "Redefine Character Style" } else { "New Character Style…" };
    if footer {
        click_new_style_footer(ctx, app, id);
    } else {
        click_style_ui(ctx, app, id, Some((0, 5)), true, label);
    }
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
    capture_during_live_typing(false, false);
}

#[test]
fn redefine_character_style_survives_live_typing_and_undo_redo() {
    capture_during_live_typing(true, false);
}

#[test]
fn new_character_style_from_footer_survives_live_typing_and_undo_redo() {
    capture_during_live_typing(false, true);
}

#[test]
fn character_style_boundary_new_from_menu() {
    capture_from_menu(Some((2, 5)), false);
}

#[test]
fn character_style_boundary_redefine_from_menu() {
    capture_from_menu(Some((2, 5)), true);
}

#[test]
fn character_style_boundary_new_from_footer() {
    capture_from_ui(Some((2, 5)), false, true, false);
}

#[test]
fn character_style_unicode_boundary_capture_controls() {
    // Byte 2 is the start of 漢 after é; 2..5 selects exactly that complete character.
    for (redefine, footer) in [(false, false), (true, false), (false, true)] {
        capture_from_ui(Some((2, 5)), redefine, footer, true);
    }
}
