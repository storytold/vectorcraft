// PLAN: control names are the accessible names of the controls (a field "X", not its label "X:"); a name that
// differs from the one assumed here fails with the visible names listed.
use egui::{Event, Key, Modifiers};
use serde_json::Value;
use vectorcraft_engine::Session;

use super::appearance_section::global_edit_results;
use super::*;
use crate::tests::panel_harness::Panel;
use crate::tests::synthetic::key_frame;

/// Two selected groups offer Ungroup and not Isolate Group. The same holds when a path is
/// selected ahead of a group, and when a group that holds another group is selected with other art.
#[test]
fn two_groups_offer_ungroup_without_isolate_group() {
    let mut app = new_app();
    let (first, second) = (group_at(&mut app, 20.0), group_at(&mut app, 160.0));
    // The later group is first in the selection, so a check of only the first node still sees a group.
    app.run("select.set", json!({"ids": [second, first]})).unwrap();

    let mut panel = Panel::new(app, quick_action_buttons);
    assert!(panel.shows("Ungroup") && !panel.shows("Isolate Group"));
    assert!(!panel.shows("Unite"), "groups are not a vector selection");
    // A path selected ahead of a group that itself holds a group.
    let inner = group_at(&mut panel.app(), 300.0);
    let beside = rect(&mut panel.app(), 380.0, 20.0, 30.0, 30.0);
    panel.app().run("select.set", json!({"ids": [inner, beside]})).unwrap();
    let outer = panel.app().run("object.group", json!({})).unwrap()["id"].clone();
    let ahead = rect(&mut panel.app(), 20.0, 120.0, 24.0, 24.0);
    panel.app().run("select.set", json!({"ids": [ahead, outer]})).unwrap();
    panel.frame();
    assert!(panel.shows("Ungroup") && !panel.shows("Isolate Group"), "path then nested group");
    assert!(panel.shows("Group") && panel.shows("Make Clipping Mask"));
    assert!(!panel.shows("Unite"), "a group in the selection");
}

/// One selected group offers Ungroup and Isolate Group, including a group that holds another group.
#[test]
fn one_group_offers_ungroup_and_isolate_group() {
    let mut app = new_app();
    let inner = group_at(&mut app, 20.0);
    let beside = rect(&mut app, 120.0, 20.0, 30.0, 30.0);
    app.run("select.set", json!({"ids": [inner, beside]})).unwrap();
    let outer = app.run("object.group", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [outer]})).unwrap();

    let panel = Panel::new(app, quick_action_buttons);
    assert!(panel.shows("Ungroup") && panel.shows("Isolate Group"));
    assert!(!panel.shows("Unite") && !panel.shows("Make Clipping Mask"), "one group");
}

/// Several paths, or a compound path with another path, offer Group, Make Clipping Mask and Unite.
/// Unite joins them in one undo step. A path selected with type does not offer Unite.
#[test]
fn several_paths_offer_group_clipping_mask_and_unite() {
    let mut app = new_app();
    let (a, b) = (rect(&mut app, 20.0, 20.0, 40.0, 40.0), rect(&mut app, 80.0, 20.0, 40.0, 40.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();

    let mut panel = Panel::new(app, quick_action_buttons);
    assert!(panel.shows("Group") && panel.shows("Make Clipping Mask") && panel.shows("Unite"));
    let before = panel.app().session.active().unwrap().selection.objects.len();
    let depth = undo_depth(&panel.app());
    panel.click("Unite");
    let after = panel.app().session.active().unwrap().selection.objects.len();
    assert!(after < before, "Unite joins the paths");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    panel.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(panel.app().session.active().unwrap().selection.objects.len(), before, "Undo restores the paths");
    // A compound path and another path.
    panel.app().run("select.set", json!({"ids": [a.clone(), b.clone()]})).unwrap();
    let compound = panel.app().run("object.compoundPath.make", json!({})).unwrap()["id"].clone();
    let c = rect(&mut panel.app(), 160.0, 20.0, 40.0, 40.0);
    panel.app().run("select.set", json!({"ids": [compound, c]})).unwrap();
    panel.frame();
    assert!(panel.shows("Group") && panel.shows("Make Clipping Mask") && panel.shows("Unite"), "compound path");
    // Type with a path is not a vector-only selection.
    let text = panel.app().run("text.create", json!({"x": 50, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
    panel.app().run("select.set", json!({"ids": [c, text]})).unwrap();
    panel.frame();
    assert!(panel.shows("Group") && !panel.shows("Unite"), "path and type");
}

/// One selected clip group has Edit Clipping Path and Edit Contents at the top of the panel,
/// above the Transform section and before every other section, and they select its clipping path
/// or its clipped art; one that clips nothing has Edit Clipping Path alone. A plain group, or a
/// clip group selected with another object, has neither.
#[test]
fn one_clip_group_has_edit_clipping_path_and_edit_contents_at_the_top() {
    let mut app = new_app();
    let art = rect(&mut app, 100.0, 80.0, 200.0, 140.0);
    let mask = app.run("shape.ellipse", json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [art, mask]})).unwrap();
    let group = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let mut panel = Panel::new(app, show);
    let selected = |panel: &Panel| panel.app().session.active().unwrap().selection.objects.iter().map(|n| json!(n.0)).collect::<Vec<_>>();
    assert!(panel.rect("Edit Clipping Path").top() < panel.rect("Transform").top(), "above the Transform section");
    assert!(panel.shows("Edit Contents"));
    panel.click("Edit Contents");
    assert_eq!(selected(&panel), std::slice::from_ref(&art), "Edit Contents selects the clipped art");
    panel.app().run("select.set", json!({"ids": [group]})).unwrap();
    panel.frame();
    panel.click("Edit Clipping Path");
    assert_eq!(selected(&panel), std::slice::from_ref(&mask), "Edit Clipping Path selects the clipping path");
    let neither = |panel: &mut Panel| {
        panel.frame();
        !panel.shows("Edit Clipping Path") && !panel.shows("Edit Contents")
    };
    // The clip group with another object, whichever is selected first.
    let other = rect(&mut panel.app(), 10.0, 10.0, 40.0, 40.0);
    panel.app().run("select.set", json!({"ids": [group, other]})).unwrap();
    assert!(neither(&mut panel), "a clip group and another object");
    panel.app().run("select.set", json!({"ids": [other, group]})).unwrap();
    assert!(neither(&mut panel), "another object and a clip group");
    // A plain group.
    let b = rect(&mut panel.app(), 60.0, 10.0, 40.0, 40.0);
    panel.app().run("select.set", json!({"ids": [other, b]})).unwrap();
    let plain = panel.app().run("object.group", json!({})).unwrap()["id"].clone();
    panel.app().run("select.set", json!({"ids": [plain]})).unwrap();
    assert!(neither(&mut panel), "a plain group");
    assert!(panel.shows("Ungroup"));
    // A clip group whose clipped art is deleted: Edit Clipping Path alone.
    panel.app().run("select.set", json!({"ids": [art]})).unwrap();
    panel.app().run("edit.clear", json!({})).unwrap();
    panel.app().run("select.set", json!({"ids": [group]})).unwrap();
    panel.frame();
    assert!(panel.shows("Edit Clipping Path") && !panel.shows("Edit Contents"), "nothing to edit");
    panel.click("Edit Clipping Path");
    assert_eq!(selected(&panel), std::slice::from_ref(&mask), "Edit Clipping Path selects the clipping path");
}

/// Start Global Edit in Quick Actions starts it on the one selected path, the button then reads
/// Stop Global Edit, and Escape stops it. Selected type has no Start Global Edit button.
#[test]
fn start_global_edit_quick_action_and_escape() {
    let mut app = new_app();
    let a = rect(&mut app, 20.0, 20.0, 40.0, 40.0);
    rect(&mut app, 200.0, 20.0, 80.0, 80.0);
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let mut panel = Panel::new(app, show);
    let on = |panel: &Panel| panel.app().session.active().unwrap().global_edit.is_some();
    assert!(panel.shows("Start Global Edit") && !panel.shows("Stop Global Edit"));
    panel.click("Start Global Edit");
    assert!(on(&panel), "the Quick Action starts Global Edit");
    assert_eq!(panel.app().session.active().unwrap().global_edit.as_ref().unwrap().members.len(), 2);
    assert!(panel.shows("Stop Global Edit") && !panel.shows("Start Global Edit"));
    // The escape chain is the app's key stage, which the panel harness does not run.
    let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    key_frame(&mut panel.app(), &crate::testing::ui_context(), vec![escape]);
    panel.frame();
    assert!(!on(&panel), "Escape stops Global Edit");
    assert!(panel.shows("Start Global Edit"));
    let selection = panel.app().session.active().unwrap().selection.objects.iter().map(|n| json!(n.0)).collect::<Vec<_>>();
    assert_eq!(selection, [a]);
    let t = panel.app().run("text.create", json!({"x": 50, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
    panel.app().run("select.set", json!({"ids": [t]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Start Global Edit"), "type has no Global Edit");
}

/// The Quick Actions' Global Edit Options close when Global Edit starts while they are open, and
/// stay closed once it stops.
#[test]
fn global_edit_starting_closes_the_quick_action_options() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = new_app();
    let a = rect(&mut app, 20.0, 20.0, 40.0, 40.0);
    rect(&mut app, 200.0, 20.0, 40.0, 40.0);
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let mut panel = Panel::new(app, super::appearance_section::global_edit_row);
    let results = global_edit_results(2);
    panel.click("Global Edit Options");
    assert!(panel.app().ui.properties_panel.global_edit_open && panel.shows(&results), "the options open");
    panel.app().run("select.globalEdit", json!({})).unwrap();
    panel.frame();
    assert!(!panel.app().ui.properties_panel.global_edit_open && !panel.shows(&results), "Global Edit starting closes the open options");
    panel.app().run("select.globalEdit", json!({})).unwrap();
    panel.frame();
    panel.frame();
    assert!(!panel.app().ui.properties_panel.global_edit_open && !panel.shows(&results), "the options stay closed once Global Edit stops");
}

/// The Global Edit Options are two groups, Match and On Artboards, and the Results line: the On Artboards caption is a
/// stack gap below the Size option, and the Results line a stack gap below Include Objects On Canvas.
#[test]
fn global_edit_options_groups_are_a_stack_gap_apart() {
    use craft::ui::contract::Role;
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let stack = std::rc::Rc::new(std::cell::Cell::new(0.0));
    let seen = std::rc::Rc::clone(&stack);
    craft::ui::testing::Fixture::new(move |ui| seen.set(craft::ui::runtime::env::Env::of(ui).tokens.space.gap.stack)).frame(vec![]);
    let stack = stack.get();
    assert!(stack > 0.0, "the interface resolved its stack gap");
    let mut app = new_app();
    let a = rect(&mut app, 20.0, 20.0, 40.0, 40.0);
    rect(&mut app, 200.0, 20.0, 40.0, 40.0);
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let mut panel = Panel::new(app, super::appearance_section::global_edit_row);
    panel.click("Global Edit Options");
    let results = global_edit_results(2);
    assert!(panel.shows(&results), "the options open");
    let size = panel.rect_role("Size", Role::CHECK_BOX);
    let caption = panel.rect_role("On Artboards", Role::LABEL);
    let canvas = panel.rect_role("Include Objects On Canvas", Role::CHECK_BOX);
    let line = panel.rect(&results);
    let (above, below) = (caption.top() - size.bottom(), line.top() - canvas.bottom());
    assert!(above >= stack - 0.5, "On Artboards is {above} below Size, under the stack gap {stack}");
    assert!(below >= stack - 0.5, "Results is {below} below the last option, under the stack gap {stack}");
}

/// The options' Results count is found once per document revision, options and selected
/// object, not every frame; another selected object or other options find it again.
#[test]
fn the_global_edit_result_count_is_cached() {
    // PLAN: assumed `appearance_section::global_edit_count(&mut VectorcraftApp) -> usize` over `app.canvas.global_edit_count`.
    use super::appearance_section::global_edit_count;
    let mut app = new_app();
    let a = rect(&mut app, 20.0, 20.0, 40.0, 40.0);
    rect(&mut app, 200.0, 20.0, 80.0, 80.0);
    let e = app.run("shape.ellipse", json!({"x": 20, "y": 150, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(global_edit_count(&mut app), 2);
    // The next frame reads the cache: a count planted under the same key is what it shows.
    let (key, _) = app.canvas.global_edit_count.take().expect("the count is memoised");
    app.canvas.global_edit_count = Some((key, 7));
    assert_eq!(global_edit_count(&mut app), 7);
    app.run("select.set", json!({"ids": [e]})).unwrap();
    assert_eq!(global_edit_count(&mut app), 1, "another selected object");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(global_edit_count(&mut app), 2);
    app.run("select.globalEditOptions", json!({"size": true})).unwrap();
    assert_eq!(global_edit_count(&mut app), 1, "other options");
}

/// Both panels forward the Move Artwork toggle when their actual X field is edited.
#[test]
fn artboard_coordinate_fields_move_art_in_properties_and_transform() {
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), super::super::transform::show] {
        for move_art in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
            app.select_tool("artboard");
            app.session.set_tool_option("moveArt", &json!(move_art));
            let mut panel = Panel::new(app, draw);
            panel.click("X position");
            panel.key(Key::ArrowUp, Modifiers::NONE);
            let app = panel.app();
            let d = &app.session.doc().unwrap().doc;
            assert_eq!(d.artboards[0].rect.x0, 1.0, "the focused X field steps its position");
            assert_eq!(d.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, if move_art { 11.0 } else { 10.0 });
            drop(app);
            panel.app().run("edit.undo", json!({})).unwrap();
            let app = panel.app();
            let d = &app.session.doc().unwrap().doc;
            assert_eq!(d.artboards[0].rect.x0, 0.0);
            assert_eq!(d.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, 10.0);
        }
    }
}

/// #2401: a percentage typed alone into W in Properties or the Transform panel is that share of the width shown, in a
/// unit other than the stored points too.
#[test]
fn a_lone_percentage_in_w_is_that_share_of_the_width() {
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), super::super::transform::show] {
        let mut app = new_app();
        app.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
        let id = rect(&mut app, 10.0, 20.0, 30.0, 40.0);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let mut panel = Panel::new(app, draw);
        panel.type_into("Width", "50%");
        let id = vectorcraft_doc::NodeId(id.as_u64().unwrap());
        let width = panel.app().session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().width();
        assert!((width - 15.0).abs() < 1e-6, "the width is {width}");
    }
}

/// Three landscape artboards, with Artboards 1 and 3 selected with the Artboard tool (Artboard
/// 1 active): 200 × 100, 200 × 100 and 300 × 150.
fn two_selected_artboards() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.run("artboard.setProps", json!({"index": 2, "width": 300, "height": 150})).unwrap();
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 0);
    app.session.set_tool_option("selected", &json!([0, 2]));
    app
}

/// Each artboard's (x0, y0, width, height).
fn boards(app: &VectorcraftApp) -> Vec<(f64, f64, f64, f64)> {
    app.session.doc().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height())).collect()
}

/// #1482: with Artboards 1 and 3 selected, a W typed into Properties or the Transform panel
/// resizes both in one undo step, and Artboard 2 keeps its size. Both resize each about its own
/// center, their shared default reference point (#5219).
#[test]
fn artboard_width_resizes_every_selected_artboard_in_properties_and_transform() {
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), super::super::transform::show] {
        let mut panel = Panel::new(two_selected_artboards(), draw);
        let before = boards(&panel.app());
        let undo = undo_depth(&panel.app());
        panel.type_into("Width", "250");
        let mut want = before.clone();
        for (i, shift) in [(0, 25.0), (2, -25.0)] {
            want[i].2 = 250.0;
            want[i].0 -= shift;
        }
        assert_eq!(boards(&panel.app()), want);
        assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step");
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&panel.app()), before);
    }
}

/// Variable fonts get the same Variable Font button in Properties as in the Character panel.
#[test]
fn properties_variable_font_button_for_variable_type() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let plain = app.run("text.create", json!({"x": 20, "y": 50, "text": "Static"})).unwrap()["id"].clone();
    let id = app.run("text.create", json!({"x": 20, "y": 120, "text": "lIn", "font": TWO_AXIS_FAMILY})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(!panel.shows("Variable Font"));
    panel.app().run("select.set", json!({"ids": [id]})).unwrap();
    panel.frame();
    assert!(panel.shows("Variable Font"));
}

/// The Name field renames the active artboard on Enter or when the field is left, with the text
/// trimmed; an empty name changes nothing. One rename is one command and one undo step.
#[test]
fn edit_artboards_renames_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 1);
    let mut panel = Panel::new(app, show);
    let before = panel.app().session.doc().unwrap().doc.artboards.clone();
    let undo = undo_depth(&panel.app());
    let commands = |panel: &Panel| panel.app().run_count;
    let c = commands(&panel);
    panel.click("Name");
    panel.key(Key::A, Modifiers::COMMAND);
    panel.key(Key::Backspace, Modifiers::NONE);
    panel.key(Key::Enter, Modifiers::NONE);
    assert_eq!(commands(&panel).wrapping_sub(c), 0, "an empty field");
    assert_eq!(panel.app().session.doc().unwrap().doc.artboards, before);
    let c = commands(&panel);
    panel.type_into("Name", " Cover ");
    assert_eq!(commands(&panel).wrapping_sub(c), 1);
    let mut renamed = before.clone();
    renamed[1].name = "Cover".into();
    assert_eq!(panel.app().session.doc().unwrap().doc.artboards, renamed, "Artboard 2 alone is renamed");
    assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step");
    panel.app().run("edit.undo", json!({})).unwrap();
    {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.artboards, before);
        assert!(!st.is_dirty());
    }
    // The click elsewhere is a click on the panel's "Artboard" heading, outside the field.
    panel.frame();
    let c = commands(&panel);
    panel.click("Name");
    panel.key(Key::A, Modifiers::COMMAND);
    panel.type_text("Back");
    panel.click("Artboard");
    assert_eq!(commands(&panel).wrapping_sub(c), 1, "a click elsewhere");
    renamed[1].name = "Back".into();
    assert_eq!(panel.app().session.doc().unwrap().doc.artboards, renamed);
}

/// #5698: with Artboards 1 and 3 selected, a Fill chosen in Edit Artboards goes to both, in one
/// command and one undo step; Artboard 2 keeps none.
#[test]
fn edit_artboards_fill_sets_every_selected_artboard() {
    let mut panel = Panel::new(two_selected_artboards(), show);
    let colors = |panel: &Panel| panel.app().session.doc().unwrap().doc.artboards.iter().map(|a| a.color).collect::<Vec<_>>();
    let undo = undo_depth(&panel.app());
    panel.click("Fill");
    let commands = panel.app().run_count;
    panel.click("Black");
    assert_eq!(panel.app().run_count.wrapping_sub(commands), 1, "one command");
    let black = vectorcraft_color::Color::BLACK;
    assert_eq!(colors(&panel), [Some(black), None, Some(black)]);
    assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step");
}

fn artboard_deletion_app(count: usize) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": count})).unwrap();
    for i in 0..count {
        app.run("shape.rectangle", json!({"x": i * 250 + 10, "y": 10, "width": 30, "height": 20})).unwrap();
    }
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 0);
    app
}

/// The Artboards panel over `app`.
fn artboards_panel(app: VectorcraftApp) -> Panel {
    Panel::new(app, super::super::artboards::show)
}

/// Clicks the Artboards row `name` with `mods` held.
fn artboard_deletion_click(panel: &mut Panel, name: &str, mods: Modifiers) {
    panel.click_with(name, mods);
}

fn select_artboard_rows(panel: &mut Panel, indices: &[usize]) {
    for (at, i) in indices.iter().enumerate() {
        artboard_deletion_click(panel, &format!("Artboard {}", i + 1), if at == 0 { Modifiers::NONE } else { Modifiers::COMMAND });
    }
    let count = panel.app().session.doc().unwrap().doc.artboards.len();
    assert_eq!(super::super::artboards::picked(&panel.app(), count), indices, "the rows are selected before deletion");
}

/// The footer's Delete Artboard button.
fn delete_by_footer(panel: &mut Panel) {
    panel.click("Delete Artboard");
}

/// The panel menu's Delete row. A refusal is read from the unchanged document by the callers.
fn delete_by_command(panel: &mut Panel) {
    // PLAN: assumed `artboards::run(app, id, &Value) -> Option<Result<Value, String>>` with the row id `ui.artboards.delete` (ty.A15, document-panels-w12).
    let _ = super::super::artboards::run(&mut panel.app(), "ui.artboards.delete", &json!({}));
    panel.frame();
}

fn assert_artboard_deletion(delete: fn(&mut Panel)) {
    for indices in [vec![0, 2], vec![2], vec![3], vec![2, 3], vec![0, 1, 2, 3]] {
        let mut panel = artboards_panel(artboard_deletion_app(4));
        select_artboard_rows(&mut panel, &indices);
        let all_selected = indices == [0, 1, 2, 3];
        let (before, undo, commands, expected, artwork) = {
            let mut app = panel.app();
            app.session.doc_mut().unwrap().mark_saved();
            let before = app.session.doc().unwrap().doc.clone();
            let undo = app.session.doc().unwrap().history.undo.len();
            let expected: Vec<_> = if all_selected {
                assert_eq!(app.view().unwrap().artboard, 3, "the active last board must not replace the first-board survivor oracle");
                vec![&before.artboards[0]]
            } else {
                before.artboards.iter().enumerate().filter(|(i, _)| !indices.contains(i)).map(|(_, a)| a).collect()
            };
            let expected = serde_json::to_value(expected).unwrap();
            let artwork = serde_json::to_value(&before.layers).unwrap();
            (before, undo, app.run_count, expected, artwork)
        };

        delete(&mut panel);
        let mut app = panel.app();
        let st = app.session.doc().unwrap();
        assert_eq!(
            serde_json::to_value(&st.doc.artboards).unwrap(),
            expected,
            "selected deletion keeps the expected IDs; all-selected keeps original board 1"
        );
        assert_eq!(app.run_count, commands.wrapping_add(1), "one command from the actual deletion control");
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork, "artwork IDs, geometry and appearance stay intact");
        assert_eq!(st.history.undo.len(), undo + 1, "one deletion is one undo step");
        assert!(st.is_dirty());
        let active = app.view().unwrap().artboard;
        if all_selected {
            assert_eq!(active, 0);
        }
        assert!(active < st.doc.artboards.len(), "the active artboard remains in range");
        assert_eq!(app.session.tool_options()["active"], json!(active));
        assert_eq!(super::super::artboards::picked(&app, st.doc.artboards.len()), [active]);
        assert_eq!(app.session.tool_options()["selected"], json!([active]), "raw tool selection contains only the remaining active board");

        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), serde_json::to_value(&before.artboards).unwrap());
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork);
        assert_eq!(st.history.undo.len(), undo);
        assert!(!st.is_dirty(), "one undo returns to the saved document");
        app.run("edit.redo", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), expected, "redo removes the same artboard IDs");
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork);
        assert_eq!(st.history.undo.len(), undo + 1);
        assert!(st.is_dirty());
    }
}

fn assert_artboard_deletion_guard(delete: fn(&mut Panel)) {
    let mut app = artboard_deletion_app(1);
    // Leave a redo entry to ensure a disabled action preserves both history branches.
    app.run("artboard.setProps", json!({"index": 0, "name": "Temporary"})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    let mut panel = artboards_panel(app);
    select_artboard_rows(&mut panel, &[0]);
    let (before, history, active, options, commands) = {
        let mut app = panel.app();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let st = app.session.doc().unwrap();
        let history = (st.history.undo.len(), st.history.redo.len());
        assert!(history.1 > 0);
        (before, history, app.view().unwrap().artboard, app.session.tool_options(), app.run_count)
    };

    delete(&mut panel);
    let app = panel.app();
    assert_eq!(app.run_count, commands, "the disabled last-board control dispatches no command");
    let st = app.session.doc().unwrap();
    assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), serde_json::to_value(&before.artboards).unwrap());
    assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), serde_json::to_value(&before.layers).unwrap());
    assert_eq!((st.history.undo.len(), st.history.redo.len()), history);
    assert!(!st.is_dirty(), "deleting the last artboard is a no-op");
    assert_eq!(app.view().unwrap().artboard, active);
    assert_eq!(app.session.tool_options(), options);
}

#[test]
fn artboard_menu_deletes_selected_artboards_in_one_undo_step() {
    assert_artboard_deletion(delete_by_command);
}

#[test]
fn artboard_menu_preserves_last_artboard() {
    assert_artboard_deletion_guard(delete_by_command);
}

#[test]
fn artboard_trash_deletes_selected_artboards_in_one_undo_step() {
    assert_artboard_deletion(delete_by_footer);
}

#[test]
fn artboard_trash_preserves_last_artboard() {
    assert_artboard_deletion_guard(delete_by_footer);
}

/// Click the actual panel footer's New Artboard button.
fn artboard_history_new(panel: &mut Panel) {
    let (count, commands) = {
        let app = panel.app();
        (app.session.doc().unwrap().doc.artboards.len(), app.run_count)
    };
    panel.click("New Artboard");
    let app = panel.app();
    assert_eq!(app.run_count, commands.wrapping_add(1), "the actual New button dispatches once");
    assert_eq!(app.session.doc().unwrap().doc.artboards.len(), count + 1);
    assert_eq!(app.session.tool_options()["active"], json!(count));
    assert_eq!(app.view().unwrap().artboard, count);
}

/// A key through the app's whole frame, as the window delivers it, so Undo and deletion take the
/// route they take in production. A whole frame comes first: the key goes where the window's frame
/// before it showed the canvas, not the panel's.
fn artboard_history_key(panel: &mut Panel, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    key_frame(&mut panel.app(), ctx, vec![]);
    for pressed in [true, false] {
        let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }];
        key_frame(&mut panel.app(), ctx, events);
    }
    panel.frame();
}

/// Duplicate the artboards, or add a new one, the way the test's history step needs.
fn add_board(panel: &mut Panel, duplicate: bool) {
    if duplicate {
        let commands = panel.app().run_count;
        // PLAN: assumed row id `ui.artboards.duplicate` (document-panels-w12).
        super::super::artboards::run(&mut panel.app(), "ui.artboards.duplicate", &json!({})).unwrap().unwrap();
        panel.frame();
        let app = panel.app();
        assert_eq!(app.run_count, commands.wrapping_add(1));
        assert_eq!(app.session.doc().unwrap().doc.artboards.len(), 3);
        assert_eq!(app.session.tool_options()["active"], json!(2));
        assert_eq!(app.view().unwrap().artboard, 2);
    } else {
        artboard_history_new(panel);
    }
}

fn artboard_history_delete_after_undo(key: Key, duplicate: bool) {
    let mut app = artboard_deletion_app(2);
    app.session.set_tool_option("moveArt", &json!(false));
    app.session.set_tool_option("scaleArt", &json!(true));
    app.session.doc_mut().unwrap().mark_saved();
    let (before, selection, undo) = {
        let st = app.session.doc().unwrap();
        assert!(!st.selection.is_empty(), "selected artwork catches a Clear fallthrough");
        (st.doc.clone(), st.selection.clone(), st.history.undo.len())
    };
    let mut panel = artboards_panel(app);
    let ctx = panel.ctx().clone();
    add_board(&mut panel, duplicate);
    assert_eq!(undo_depth(&panel.app()), undo + 1);
    artboard_history_key(&mut panel, &ctx, Key::Z, Modifiers::COMMAND);
    {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, before, "the real Undo shortcut restores the original document");
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
    }
    delete_again_after_undo(&mut panel, &ctx, key, &before, &selection, undo);
}

/// Delete the normalized shown board after Undo, then verify its own Undo and Redo.
fn delete_again_after_undo(
    panel: &mut Panel,
    ctx: &egui::Context,
    key: Key,
    before: &std::sync::Arc<vectorcraft_doc::Document>,
    selection: &vectorcraft_doc::Selection,
    undo: usize,
) {
    // Capture the state before another panel or pointer can repair it: keyboard deletion used to do nothing.
    let after_undo = {
        let app = panel.app();
        (app.view().unwrap().artboard, app.session.tool_options())
    };
    artboard_history_key(panel, ctx, key, Modifiers::NONE);
    let deleted = {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.artboards, vec![before.artboards[0].clone()], "deletion after Undo removes the shown second board");
        assert_eq!(st.doc.layers, before.layers, "ordinary artwork survives without geometry or style changes");
        assert_eq!(&st.selection, selection, "deletion never falls through to Clear");
        assert_eq!(after_undo.0, 1, "Undo normalizes the navigator before another gesture");
        assert_eq!(after_undo.1, json!({"active": 1, "selected": [1], "moveArt": false, "scaleArt": true}));
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
        assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
        assert!(st.is_dirty());
        assert_eq!(app.view().unwrap().artboard, 0);
        assert_eq!(app.session.tool_options(), json!({"active": 0, "selected": [0], "moveArt": false, "scaleArt": true}));
        st.doc.clone()
    };

    artboard_history_key(panel, ctx, Key::Z, Modifiers::COMMAND);
    {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.as_ref(), before.as_ref());
        assert_eq!(&st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
        // Valid current selection stays put; history does not restore an earlier artboard selection.
        assert_eq!(app.session.tool_options()["selected"], json!([0]));
    }
    artboard_history_key(panel, ctx, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    let app = panel.app();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.as_ref(), deleted.as_ref());
    assert_eq!(&st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert!(st.is_dirty());
}

#[test]
fn artboard_history_new_undo_delete_uses_remaining_board() {
    artboard_history_delete_after_undo(Key::Delete, false);
}

#[test]
fn artboard_history_new_undo_backspace_uses_remaining_board() {
    artboard_history_delete_after_undo(Key::Backspace, false);
}

#[test]
fn artboard_history_duplicate_undo_delete_uses_remaining_board() {
    artboard_history_delete_after_undo(Key::Delete, true);
}

#[test]
fn artboard_history_undo_prunes_only_removed_indices() {
    for order in [[0, 1, 2], [0, 2, 1]] {
        let mut app = artboard_deletion_app(2);
        app.session.set_tool_option("moveArt", &json!(false));
        app.session.set_tool_option("scaleArt", &json!(true));
        app.session.doc_mut().unwrap().mark_saved();
        let (before, selection, undo) = {
            let st = app.session.doc().unwrap();
            (st.doc.clone(), st.selection.clone(), st.history.undo.len())
        };
        let mut panel = artboards_panel(app);
        let ctx = panel.ctx().clone();
        artboard_history_new(&mut panel);
        let created = panel.app().session.doc().unwrap().doc.clone();
        for (at, i) in order.into_iter().enumerate() {
            artboard_deletion_click(&mut panel, &format!("Artboard {}", i + 1), if at == 0 { Modifiers::NONE } else { Modifiers::COMMAND });
        }
        let options = panel.app().session.tool_options();
        let expected = if order[2] == 2 { json!([0, 1]) } else { json!([1, 0]) };
        assert_eq!(options["selected"], if order[2] == 2 { json!([2, 0, 1]) } else { json!([1, 0, 2]) });
        artboard_history_key(&mut panel, &ctx, Key::Z, Modifiers::COMMAND);
        let mut normalized = options;
        normalized["active"] = expected[0].clone();
        normalized["selected"] = expected;
        {
            let app = panel.app();
            assert_eq!(app.session.tool_options(), normalized, "only the removed index is pruned, preserving surviving order and options");
            assert_eq!(json!(app.view().unwrap().artboard), normalized["active"]);
            let st = app.session.doc().unwrap();
            assert_eq!(st.doc, before);
            assert_eq!(st.selection, selection);
            assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
            assert!(!st.is_dirty());
        }
        artboard_history_key(&mut panel, &ctx, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        let app = panel.app();
        assert_eq!(app.session.doc().unwrap().doc, created);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(app.session.tool_options(), normalized, "Redo does not replace an already valid multiselection");
    }
}

#[test]
fn artboard_history_valid_multiselection_survives_undo_redo() {
    let mut app = artboard_deletion_app(3);
    app.session.set_tool_option("moveArt", &json!(false));
    app.session.set_tool_option("scaleArt", &json!(true));
    let mut panel = artboards_panel(app);
    let ctx = panel.ctx().clone();
    select_artboard_rows(&mut panel, &[0, 1]);
    let (options, view, before, selection, undo) = {
        let mut app = panel.app();
        let options = app.session.tool_options();
        assert_eq!(options["selected"], json!([1, 0]));
        let view = app.view().unwrap().artboard;
        app.session.doc_mut().unwrap().mark_saved();
        let st = app.session.doc().unwrap();
        (options, view, st.doc.clone(), st.selection.clone(), st.history.undo.len())
    };
    panel.app().run("object.move", json!({"dx": 7, "dy": 9})).unwrap();
    let moved = panel.app().session.doc().unwrap().doc.clone();
    assert_ne!(moved, before);
    artboard_history_key(&mut panel, &ctx, Key::Z, Modifiers::COMMAND);
    {
        let mut app = panel.app();
        app.sync_views();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, before);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
        assert_eq!(app.session.tool_options(), options);
        assert_eq!(app.view().unwrap().artboard, view);
    }
    artboard_history_key(&mut panel, &ctx, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    let mut app = panel.app();
    app.sync_views();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc, moved);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert!(st.is_dirty());
    assert_eq!(app.session.tool_options(), options);
    assert_eq!(app.view().unwrap().artboard, view);
}

/// #671: Edit Artboards shows the artboard's preset (Custom when it matches none) and its
/// orientation; the other orientation swaps its width and height, its corner staying put.
#[test]
fn edit_artboards_shows_the_preset_and_flips_the_orientation() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"preset": "Letter"})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "x": 10, "y": 20})).unwrap();
    app.select_tool("artboard");
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Letter") && panel.shows("Portrait") && panel.shows("Landscape"));
    panel.click("Landscape");
    {
        let app = panel.app();
        let rect = app.session.active().unwrap().doc.artboards[0].rect;
        assert_eq!((rect.x0, rect.y0, rect.width(), rect.height()), (10.0, 20.0, 792.0, 612.0), "landscape, same corner");
    }
    assert!(panel.shows("Letter"), "still Letter, turned");
    panel.app().run("artboard.setProps", json!({"index": 0, "width": 333})).unwrap();
    panel.frame();
    assert!(panel.shows("Custom"));
}

/// #1482: with Artboards 1 and 3 selected, Portrait swaps the width and height of both in one
/// undo step, each keeping its top-left corner; Artboard 2 stays landscape. An artboard already
/// in that orientation keeps its size.
#[test]
fn edit_artboards_orientation_turns_every_selected_artboard() {
    let mut panel = Panel::new(two_selected_artboards(), show);
    let before = boards(&panel.app());
    let undo = undo_depth(&panel.app());
    panel.click("Portrait");
    let mut want = before.clone();
    for i in [0, 2] {
        (want[i].2, want[i].3) = (before[i].3, before[i].2);
    }
    assert_eq!(boards(&panel.app()), want);
    assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step");
    panel.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&panel.app()), before);
    // Artboard 3 portrait already: Portrait turns Artboard 1 alone.
    panel.app().run("artboard.setProps", json!({"index": 2, "width": 150, "height": 300})).unwrap();
    let before = boards(&panel.app());
    panel.click("Portrait");
    let mut want = before.clone();
    (want[0].2, want[0].3) = (100.0, 200.0);
    assert_eq!(boards(&panel.app()), want);
}

/// #1482: with Artboards 1 and 3 selected, a preset chosen in Edit Artboards sizes both in one
/// undo step, each in its own orientation and keeping its top-left corner: Letter (612 × 792 pt)
/// stays portrait for Artboard 1 and turns landscape for Artboard 3. Artboard 2 keeps its size.
#[test]
fn edit_artboards_preset_sizes_every_selected_artboard() {
    let mut app = two_selected_artboards();
    app.run("artboard.setProps", json!({"index": 0, "width": 100, "height": 200})).unwrap();
    let before = boards(&app);
    let undo = undo_depth(&app);
    let mut panel = Panel::new(app, show);
    panel.click("Custom");
    panel.click("Print");
    panel.click("Letter");
    let mut want = before.clone();
    (want[0].2, want[0].3) = (612.0, 792.0);
    (want[2].2, want[2].3) = (792.0, 612.0);
    assert_eq!(boards(&panel.app()), want);
    assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step");
}

/// #602: Edit Artboards shows Move and Scale Artwork with Artboard; with Scale on, a new size
/// (here the other orientation) takes the art on the artboard along.
#[test]
fn edit_artboards_scales_the_art_with_scale_artwork_with_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let r = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("artboard");
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Move Artwork with Artboard"));
    assert_eq!(panel.app().session.tool_options()["scaleArt"], false);
    panel.click("Scale Artwork with Artboard");
    assert_eq!(panel.app().session.tool_options()["scaleArt"], true);
    // Portrait: 100 × 200, the art scaled by ½ across and 2 down.
    panel.click("Portrait");
    let app = panel.app();
    let d = &app.session.active().unwrap().doc;
    let ab = d.artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (100.0, 200.0));
    let b = d.node(vectorcraft_doc::NodeId(r)).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (10.0, 40.0, 20.0, 80.0));
}

fn undo_depth(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn new_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64, w: f64, h: f64) -> serde_json::Value {
    app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].clone()
}

/// One selected blend of two rectangles → the blend.
fn blend(app: &mut VectorcraftApp) -> Value {
    let (a, b) = (rect(app, 20.0, 20.0, 40.0, 40.0), rect(app, 200.0, 20.0, 40.0, 40.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let blend = app.run("object.blend.make", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [blend]})).unwrap();
    blend
}

/// One selected repeat made by `command` from a rectangle → the repeat.
fn repeat(app: &mut VectorcraftApp, command: &str) -> Value {
    let a = rect(app, 20.0, 20.0, 40.0, 40.0);
    let repeat = app.run(command, json!({"ids": [a]})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [repeat]})).unwrap();
    repeat
}

/// A Live Paint group of two overlapping rectangles, a separate path and a text → (group, path, text).
fn live_paint(app: &mut VectorcraftApp) -> (Value, Value, Value) {
    let (a, b) = (rect(app, 20.0, 20.0, 100.0, 60.0), rect(app, 60.0, 40.0, 100.0, 60.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let group = app.run("livePaint.make", json!({})).unwrap()["id"].clone();
    let path = rect(app, 140.0, 30.0, 60.0, 60.0);
    let text = app.run("text.create", json!({"x": 50, "y": 250, "text": "Hi"})).unwrap()["id"].clone();
    (group, path, text)
}

/// A group of two rectangles at `x`, selected.
fn group_at(app: &mut VectorcraftApp, x: f64) -> Value {
    let (a, b) = (rect(app, x, 20.0, 40.0, 40.0), rect(app, x + 50.0, 20.0, 40.0, 40.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("object.group", json!({})).unwrap()["id"].clone()
}

/// A dynamic symbol instance, selected → the instance.
fn dynamic_instance(app: &mut VectorcraftApp) -> Value {
    let (a, b) = (rect(app, 20.0, 20.0, 40.0, 40.0), rect(app, 80.0, 20.0, 40.0, 40.0));
    let inst = app.run("symbol.new", json!({"ids": [a, b], "type": "dynamic"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [inst]})).unwrap();
    inst
}

/// A case of the Quick Actions table: the selection's setup, the commands it offers and the ones it does not.
type QuickCase = (&'static str, fn(&mut VectorcraftApp), &'static [&'static str], &'static [&'static str]);

/// Quick Actions offer each command only where it can run. A condition dropped from the rules
/// shows a command that fails or does nothing.
#[test]
fn quick_actions_follow_the_selection() {
    let cases: &[QuickCase] = &[
        ("one blend, as with the Selection tool (#4817)", |app| drop(blend(app)), &["ui.blendOptions"], &[]),
        (
            "a blend and another object",
            |app| {
                let blend = blend(app);
                let c = rect(app, 20.0, 200.0, 40.0, 40.0);
                app.run("select.set", json!({"ids": [blend, c]})).unwrap();
            },
            &[],
            &["ui.blendOptions"],
        ),
        (
            "a rectangle",
            |app| {
                let a = rect(app, 20.0, 20.0, 40.0, 40.0);
                app.run("select.set", json!({"ids": [a]})).unwrap();
            },
            &[],
            &["ui.blendOptions", "object.repeat.options", "livePaint.merge", "object.isolate", "symbol.instance.reset"],
        ),
        ("one radial repeat (#5181)", |app| drop(repeat(app, "object.repeat.radial")), &["object.repeat.options"], &[]),
        ("one grid repeat", |app| drop(repeat(app, "object.repeat.grid")), &["object.repeat.options"], &[]),
        ("one mirror repeat", |app| drop(repeat(app, "object.repeat.mirror")), &["object.repeat.options"], &[]),
        (
            "a repeat and another object",
            |app| {
                let repeat = repeat(app, "object.repeat.radial");
                let c = rect(app, 20.0, 200.0, 40.0, 40.0);
                app.run("select.set", json!({"ids": [repeat, c]})).unwrap();
            },
            &[],
            &["object.repeat.options"],
        ),
        (
            "a member in isolation mode",
            |app| {
                let g = group_at(app, 20.0);
                app.run("object.isolate", json!({"id": g})).unwrap();
                let member =
                    app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(g.as_u64().unwrap())).unwrap().children().unwrap()[0].id.0;
                app.run("select.set", json!({"ids": [member]})).unwrap();
            },
            &["object.lock", "object.path.offsetPath"],
            &[],
        ),
        ("a group outside isolation mode", |app| drop(group_at(app, 20.0)), &["object.lock", "object.ungroup", "object.isolate"], &[]),
        (
            "a Live Paint group with a path",
            |app| {
                let (group, path, _) = live_paint(app);
                app.run("select.set", json!({"ids": [group, path]})).unwrap();
            },
            &["livePaint.merge"],
            &[],
        ),
        (
            "a Live Paint group alone",
            |app| {
                let (group, ..) = live_paint(app);
                app.run("select.set", json!({"ids": [group]})).unwrap();
            },
            &[],
            &["livePaint.merge"],
        ),
        (
            "two plain paths",
            |app| {
                let (_, path, _) = live_paint(app);
                let other = rect(app, 300.0, 200.0, 40.0, 40.0);
                app.run("select.set", json!({"ids": [path, other]})).unwrap();
            },
            &[],
            &["livePaint.merge"],
        ),
        (
            "a Live Paint group and type",
            |app| {
                let (group, _, text) = live_paint(app);
                app.run("select.set", json!({"ids": [group, text]})).unwrap();
            },
            &[],
            &["livePaint.merge"],
        ),
        (
            "a Live Paint group, a path and type",
            |app| {
                let (group, path, text) = live_paint(app);
                app.run("select.set", json!({"ids": [group, path, text]})).unwrap();
            },
            &[],
            &["livePaint.merge"],
        ),
        (
            "a group in the art of an opacity mask being edited",
            |app| {
                let (obj, m) = (rect(app, 200.0, 100.0, 80.0, 80.0), rect(app, 200.0, 100.0, 40.0, 80.0));
                app.run("transparency.makeOpacityMask", json!({"ids": [obj, m]})).unwrap();
                app.run("transparency.editOpacityMask", json!({"id": obj})).unwrap();
                group_at(app, 200.0);
            },
            &["object.ungroup"],
            &["object.isolate"],
        ),
        (
            "type alone (#2238)",
            |app| {
                let t = app.run("text.create", json!({"x": 50, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
                app.run("select.set", json!({"ids": [t]})).unwrap();
            },
            &["object.lock"],
            &["object.path.offsetPath", "object.path.simplify"],
        ),
        (
            "type and a path",
            |app| {
                let t = app.run("text.create", json!({"x": 50, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
                let r = rect(app, 20.0, 20.0, 40.0, 40.0);
                app.run("select.set", json!({"ids": [t, r]})).unwrap();
            },
            &["object.path.offsetPath", "object.path.simplify"],
            &[],
        ),
        (
            "a dynamic instance with changes of its own (#4938)",
            |app| {
                dynamic_instance(app);
                app.run("symbol.instance.setAppearance", json!({"path": [0], "fill": "#ff0000"})).unwrap();
            },
            &["symbol.instance.reset"],
            &[],
        ),
        ("a dynamic instance without changes", |app| drop(dynamic_instance(app)), &[], &["symbol.instance.reset"]),
        (
            "one colour",
            |app| {
                rect(app, 10.0, 10.0, 50.0, 50.0);
                app.run("paint.setStroke", json!({"none": true})).unwrap();
            },
            &[],
            &["ui.recolorDialog"],
        ),
        (
            "white and red",
            |app| {
                rect(app, 10.0, 10.0, 50.0, 50.0);
                app.run("paint.setStroke", json!({"none": true})).unwrap();
                app.run("paint.setStroke", json!({"color": "#ff0000"})).unwrap();
            },
            &["ui.recolorDialog"],
            &[],
        ),
    ];
    for (case, setup, present, absent) in cases {
        let mut app = new_app();
        setup(&mut app);
        // PLAN: assumed `quick_actions` is reachable from `properties` (op.A10).
        let offered: Vec<&str> = quick_actions(&mut app).iter().map(|a| a.command).collect();
        for command in *present {
            assert!(offered.contains(command), "{case}: {command} in {offered:?}");
        }
        for command in *absent {
            assert!(!offered.contains(command), "{case}: no {command} in {offered:?}");
        }
    }
}

/// A brushed stroke with a variable width: the Width Profile list lists only Uniform, which removes
/// the profile.
#[test]
fn the_width_profile_of_a_brushed_stroke_offers_only_uniform() {
    let mut app = new_app();
    let line = app.run("shape.line", json!({"x1": 20, "y1": 100, "x2": 300, "y2": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [line]})).unwrap();
    app.run("stroke.set", json!({"profile": "lens"})).unwrap();
    app.run("brush.apply", json!({"ids": [line], "name": "3 pt. Round"})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.click("Profile");
    assert!(!panel.shows("Taper Start"));
    panel.click("Uniform");
    let app = panel.app();
    let profile =
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(line.as_u64().unwrap())).unwrap().appearance.stroke().unwrap().profile.clone();
    assert_eq!(profile, None);
}

/// Two selected strokes, the shown one uniform and the other with a variable width: the Width
/// Profile row still shows, and its Uniform removes the other stroke's profile. With brushes on both
/// strokes the list holds only Uniform. Once both are uniform the row is hidden again.
#[test]
fn the_width_profile_shows_while_another_selected_stroke_has_a_variable_width() {
    for brushed in [false, true] {
        let mut app = new_app();
        let a = app.run("shape.line", json!({"x1": 20, "y1": 100, "x2": 300, "y2": 100})).unwrap()["id"].clone();
        let b = app.run("shape.line", json!({"x1": 20, "y1": 160, "x2": 300, "y2": 160})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [b]})).unwrap();
        app.run("stroke.set", json!({"profile": "lens"})).unwrap();
        if brushed {
            app.run("brush.apply", json!({"ids": [a, b], "name": "3 pt. Round"})).unwrap();
        }
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        assert_eq!(super::super::current_stroke(&app).and_then(|s| s.profile), None, "the shown stroke is uniform");
        let mut panel = Panel::new(app, show);
        assert!(panel.shows("Profile"), "brushed {brushed}");
        panel.click("Profile");
        assert_eq!(panel.shows("Taper Start"), !brushed, "brushed {brushed}");
        panel.click("Uniform");
        let profile = panel
            .app()
            .session
            .active()
            .unwrap()
            .doc
            .node(vectorcraft_doc::NodeId(b.as_u64().unwrap()))
            .unwrap()
            .appearance
            .stroke()
            .unwrap()
            .profile
            .clone();
        assert_eq!(profile, None, "brushed {brushed}");
        assert!(!panel.shows("Profile"), "brushed {brushed}: every stroke is uniform");
    }
}

/// The Width Profile row follows a change of selection that leaves the document as it was: it
/// hides when the stroke with a variable width leaves the selection and shows again when it comes
/// back.
#[test]
fn the_width_profile_row_follows_a_selection_change() {
    let mut app = new_app();
    let a = app.run("shape.line", json!({"x1": 20, "y1": 100, "x2": 300, "y2": 100})).unwrap()["id"].clone();
    let b = app.run("shape.line", json!({"x1": 20, "y1": 160, "x2": 300, "y2": 160})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("stroke.set", json!({"profile": "lens"})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Profile"), "the other stroke has a variable width");
    panel.app().run("select.set", json!({"ids": [a]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Profile"), "the uniform stroke alone");
    panel.app().run("select.set", json!({"ids": [a, b]})).unwrap();
    panel.frame();
    assert!(panel.shows("Profile"), "both strokes again");
}

/// With Direct Selection, a path whose corner is selected shows that corner's kind and radius;
/// each edit sets that corner alone in one undo step. Not with the Selection tool, nor with the
/// whole path selected; a live rectangle keeps its own Corner Radius row instead.
#[test]
fn direct_selected_corners_show_their_kind_and_radius() {
    let mut app = new_app();
    let star = app.run("shape.star", json!({"cx": 200, "cy": 150, "radius1": 80, "radius2": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.anchors", json!({"id": star, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(!panel.shows("Corner"), "not with the Selection tool");
    panel.app().select_tool("directSelection");
    panel.app().run("select.anchors", json!({"id": star, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    panel.frame();
    assert!(panel.shows("Corner") && panel.shows("Corner Radius") && panel.shows("Corners"));
    let corners = |panel: &Panel| {
        let n = panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(star)).unwrap().clone();
        let lc = vectorcraft_doc::LiveCorners::of(&n).unwrap();
        (lc.radius(0), lc.radius(2), lc.kind(0), lc.kind(2))
    };
    let depth = undo_depth(&panel.app());
    panel.type_into("Corner Radius", "6");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    let round = vectorcraft_geom::shapes::CornerKind::Round;
    assert_eq!(corners(&panel), (0.0, 6.0, round, round));
    // The kind, from its list.
    panel.click("Corner");
    panel.click("Chamfer");
    assert_eq!(undo_depth(&panel.app()), depth + 2, "one undo step");
    assert_eq!(corners(&panel), (0.0, 6.0, round, vectorcraft_geom::shapes::CornerKind::Chamfer));
    // The whole path selected.
    panel.app().run("select.set", json!({"ids": [star]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Corner"), "no corner selected");
    // A live rectangle's corner: its own Corner Radius row and Corner menu.
    let r = rect(&mut panel.app(), 20.0, 20.0, 60.0, 40.0).as_u64().unwrap();
    panel.app().run("select.anchors", json!({"id": r, "anchors": [[0, 1]], "mode": "set"})).unwrap();
    panel.frame();
    assert!(panel.shows("Corner") && panel.shows("Corner Radius"));
    // And the Corners link, for the pop-up's rounding: Relative on the picked corner's rectangle.
    panel.click("Corners");
    panel.click("Relative");
    let n = panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(r)).unwrap().clone();
    assert!(vectorcraft_doc::LiveCorners::of(&n).unwrap().radii_relative());
}

/// With Direct Selection, a live rectangle or live polygon selected whole keeps its own rows and
/// shows the Corners link, whose pop-up sets every corner of the shape. A path with Live Corners
/// selected whole shows no corner rows.
#[test]
fn a_whole_live_shape_with_direct_selection_shows_the_corners_link() {
    let mut app = new_app();
    app.select_tool("directSelection");
    let r = rect(&mut app, 20.0, 20.0, 60.0, 40.0).as_u64().unwrap();
    let polygon = app.run("shape.polygon", json!({"cx": 200, "cy": 150, "radius": 50, "sides": 5})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [polygon]})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Corners"), "polygon");
    panel.app().run("select.set", json!({"ids": [r]})).unwrap();
    panel.frame();
    assert!(panel.shows("Corners") && panel.shows("Corner Radius"), "rectangle");
    // The pop-up's rounding: Relative on the rectangle.
    panel.click("Corners");
    let depth = undo_depth(&panel.app());
    panel.click("Relative");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    let n = panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(r)).unwrap().clone();
    assert!(vectorcraft_doc::LiveCorners::of(&n).unwrap().radii_relative());
    // A plain path selected whole: no corner selected.
    let path = square_path(&mut panel.app(), 200.0);
    panel.app().select_tool("directSelection");
    panel.app().run("select.set", json!({"ids": [path]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Corner") && !panel.shows("Corners"));
}

/// A live rectangle with Relative rounding shows its radius as a document length in its Corner
/// Radius field, as the Corners pop-up does, not as the fraction it stores.
#[test]
fn a_relative_live_rectangle_shows_its_radius_as_a_length() {
    let mut app = new_app();
    let r = rect(&mut app, 20.0, 20.0, 60.0, 40.0).as_u64().unwrap();
    app.run("select.set", json!({"ids": [r]})).unwrap();
    app.run("object.setLiveShape", json!({"ids": [r], "radius": 10})).unwrap();
    app.run("object.setLiveShape", json!({"ids": [r], "relativeRadii": true})).unwrap();
    let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(r)).unwrap().clone();
    assert!(vectorcraft_doc::LiveCorners::of(&n).unwrap().radii_relative());
    let shown = crate::panels::corner_radius(&app, &n).unwrap();
    assert!((shown - 10.0).abs() < 1e-6, "the field shows {shown}");
    assert_eq!(crate::panels::picked_corners(&app).unwrap().radius.map(|r| (r - 10.0).abs() < 1e-6), Some(true), "as the pop-up does");
}

/// A square path, not a live shape, its top-left corner at (`x`, 50) → its id.
fn square_path(app: &mut VectorcraftApp, x: f64) -> u64 {
    let id = rect(app, x, 50.0, 100.0, 100.0);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("object.expandShape", json!({})).unwrap();
    id.as_u64().unwrap()
}

/// The radius and kind of each corner of path `id`, from the top-left clockwise.
fn path_corners(app: &VectorcraftApp, id: u64) -> Vec<(f64, vectorcraft_geom::shapes::CornerKind)> {
    let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    let lc = vectorcraft_doc::LiveCorners::of(&n).unwrap();
    (0..4).map(|k| (lc.radius(k), lc.kind(k))).collect()
}

/// The Corners link opens a pop-up with the corner kind, the radius and the rounding of the
/// Direct-Selected corners. A radius typed there, or a kind or rounding picked, sets the three
/// picked corners and leaves the fourth as it is, each in one undo step.
#[test]
fn the_corners_link_opens_the_settings_of_the_picked_corners() {
    let mut app = new_app();
    let id = square_path(&mut app, 50.0);
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": "set"})).unwrap();
    let mut panel = Panel::new(app, show);
    assert!(!panel.shows("Relative") && !panel.shows("Chamfer"), "closed");
    panel.click("Corners");
    for name in ["Round", "Inverted Round", "Chamfer", "Radius", "Relative", "Absolute"] {
        assert!(panel.shows(name), "the pop-up has {name}");
    }
    let depth = undo_depth(&panel.app());
    panel.type_into("Radius", "18");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    let (round, chamfer) = (vectorcraft_geom::shapes::CornerKind::Round, vectorcraft_geom::shapes::CornerKind::Chamfer);
    assert_eq!(path_corners(&panel.app(), id), vec![(18.0, round), (18.0, round), (18.0, round), (0.0, round)]);
    // A kind, picked in the pop-up.
    panel.click("Chamfer");
    assert_eq!(undo_depth(&panel.app()), depth + 2, "one undo step");
    assert_eq!(path_corners(&panel.app(), id), vec![(18.0, chamfer), (18.0, chamfer), (18.0, chamfer), (0.0, round)]);
    // Relative rounding, for the path the corners are on.
    panel.click("Relative");
    assert_eq!(undo_depth(&panel.app()), depth + 3, "one undo step");
    let n = panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    assert!(vectorcraft_doc::LiveCorners::of(&n).unwrap().radii_relative());
}

/// With a corner of each of two paths Direct-Selected and their radii different, the pop-up's
/// radius is blank; a radius typed there sets both corners, and no other, in one undo step.
#[test]
fn the_corners_pop_up_sets_picked_corners_across_paths() {
    let mut app = new_app();
    let (a, b) = (square_path(&mut app, 20.0), square_path(&mut app, 200.0));
    app.run("object.setLiveShape", json!({"ids": [a], "corners": [0], "radius": 5})).unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": a, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    app.run("select.anchors", json!({"id": b, "anchors": [[0, 1]], "mode": "add"})).unwrap();
    assert_eq!(crate::panels::picked_corners(&app).unwrap().radius, None, "blank while the radii differ");
    let mut panel = Panel::new(app, show);
    panel.click("Corners");
    let depth = undo_depth(&panel.app());
    panel.type_into("Radius", "7");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    let round = vectorcraft_geom::shapes::CornerKind::Round;
    assert_eq!(path_corners(&panel.app(), a), vec![(7.0, round), (0.0, round), (0.0, round), (0.0, round)]);
    assert_eq!(path_corners(&panel.app(), b), vec![(0.0, round), (7.0, round), (0.0, round), (0.0, round)]);
}

/// With a corner of one path Direct-Selected and a second path selected whole, the pop-up picks
/// that corner and every corner of the second path. While one path is Relative and the other
/// Absolute, no rounding is chosen; picking Absolute sets both paths in one undo step.
#[test]
fn the_corners_pop_up_takes_every_corner_of_a_whole_path_and_mixed_rounding() {
    let mut app = new_app();
    let (a, b) = (square_path(&mut app, 20.0), square_path(&mut app, 200.0));
    app.run("object.setLiveShape", json!({"ids": [a], "corners": [0], "radius": 5})).unwrap();
    app.run("object.setLiveShape", json!({"ids": [a], "relativeRadii": true})).unwrap();
    app.select_tool("directSelection");
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("select.anchors", json!({"id": a, "anchors": [[0, 0]], "mode": "add"})).unwrap();
    let picked = crate::panels::picked_corners(&app).unwrap();
    assert_eq!(picked.relative, None, "no rounding while the paths differ");
    assert_eq!(picked.items.len(), 2);
    let mut panel = Panel::new(app, show);
    panel.click("Corners");
    let depth = undo_depth(&panel.app());
    panel.click("Absolute");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    assert_eq!(crate::panels::picked_corners(&panel.app()).unwrap().relative, Some(false), "both Absolute");
    panel.type_into("Radius", "7");
    assert_eq!(undo_depth(&panel.app()), depth + 2, "one undo step");
    let round = vectorcraft_geom::shapes::CornerKind::Round;
    assert_eq!(path_corners(&panel.app(), a), vec![(7.0, round), (0.0, round), (0.0, round), (0.0, round)]);
    assert_eq!(path_corners(&panel.app(), b), vec![(7.0, round); 4], "every corner of the path selected whole");
}

/// One effect on the one selected object shows its name and a Delete icon beside the fx button,
/// which removes it in one undo step, the object's own or its stroke's. With several effects the
/// icon activates the Appearance panel; without one, or with several objects selected, no name.
#[test]
fn one_effect_has_a_delete_icon_in_appearance() {
    let mut app = new_app();
    let id = rect(&mut app, 20.0, 20.0, 80.0, 60.0);
    let node = vectorcraft_doc::NodeId(id.as_u64().unwrap());
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let mut panel = Panel::new(app, show);
    let effects = |panel: &Panel| {
        let n = panel.app().session.active().unwrap().doc.node(node).unwrap().clone();
        n.appearance.effects.len() + n.appearance.items.iter().map(|i| i.effects().len()).sum::<usize>()
    };
    let name = super::super::appearance::shown_effect_label("stylize.dropShadow");
    assert!(!panel.shows(&name), "no effect");
    // The object's own effect.
    panel.app().run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    panel.frame();
    assert!(panel.shows(&name));
    let depth = undo_depth(&panel.app());
    panel.click("Remove Effect");
    assert_eq!((effects(&panel), undo_depth(&panel.app())), (0, depth + 1), "removed in one undo step");
    panel.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&panel), 1);
    // The stroke's effect.
    panel.app().run("edit.undo", json!({})).unwrap();
    let stroke = panel.app().session.active().unwrap().doc.node(node).unwrap().appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    panel.app().run("effect.apply", json!({"effect": "stylize.dropShadow", "item": stroke})).unwrap();
    panel.frame();
    let depth = undo_depth(&panel.app());
    panel.click("Remove Effect");
    assert_eq!((effects(&panel), undo_depth(&panel.app())), (0, depth + 1), "the stroke's effect removed in one undo step");
    // Two effects: the icon activates the Appearance panel and removes nothing.
    panel.app().run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    panel.app().run("effect.apply", json!({"effect": "stylize.dropShadow", "item": stroke})).unwrap();
    panel.frame();
    assert!(!panel.shows(&name));
    let depth = undo_depth(&panel.app());
    // PLAN: the icon fires `window.panel.activate {panel: "appearance"}`, which the dock's service handles; only its effect on the document is read here.
    panel.click("Remove Effect");
    assert_eq!((effects(&panel), undo_depth(&panel.app())), (2, depth));
    // Two objects with one effect each: no name.
    panel.app().run("edit.undo", json!({})).unwrap();
    let other = rect(&mut panel.app(), 200.0, 20.0, 40.0, 40.0);
    panel.app().run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    panel.app().run("select.set", json!({"ids": [id, other]})).unwrap();
    panel.frame();
    assert!(!panel.shows(&name), "two objects");
}

#[test]
fn properties_remove_effect_requires_the_whole_symbol_instance() {
    let mut app = new_app();
    let left = rect(&mut app, 20.0, 20.0, 20.0, 20.0);
    let right = rect(&mut app, 60.0, 20.0, 20.0, 20.0);
    let instance = app.run("symbol.new", json!({"type": "dynamic", "ids": [left, right]})).unwrap()["id"].clone();
    let node = vectorcraft_doc::NodeId(instance.as_u64().unwrap());
    app.run("effect.apply", json!({"ids": [instance], "effect": "stylize.dropShadow"})).unwrap();
    app.run("symbol.instance.selectPart", json!({"id": instance, "path": [0]})).unwrap();
    let mut panel = Panel::new(app, show);
    let effects = |panel: &Panel| panel.app().session.active().unwrap().doc.node(node).unwrap().appearance.effects.len();
    let depth = undo_depth(&panel.app());

    assert_eq!(effects(&panel), 1);
    panel.click("Remove Effect");
    assert_eq!((effects(&panel), undo_depth(&panel.app())), (1, depth), "a part cannot remove the whole instance’s effect");
    assert!(!panel.enabled("Remove Effect"));
    assert!(panel.shows("Appearance panel"), "Appearance remains available for inspection");

    panel.app().run("select.set", json!({"ids": [instance]})).unwrap();
    panel.frame();
    assert!(panel.enabled("Remove Effect"));
    panel.click("Remove Effect");
    assert_eq!((effects(&panel), undo_depth(&panel.app())), (0, depth + 1));
}

/// A rectangle at 100, 100, 100 × 60 pt, selected, with `then` run on it, and the Transform
/// panel's reference point on the top-left square.
fn reference_setup(then: &[(&str, Value)]) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 60})).unwrap();
    for (id, p) in then {
        app.run(id, p.clone()).unwrap();
    }
    app.ui.transform_panel.reference_point = 0;
    app
}

/// The Properties panel with the Transform panel below it, as when both are open.
fn properties_and_transform(app: &mut VectorcraftApp, ui: &mut Ui) {
    show(app, ui);
    ui.separator();
    crate::panels::transform::show(app, ui);
}

/// A panel (or panels) drawn in a test frame.
type Draw = fn(&mut VectorcraftApp, &mut Ui);

fn near(a: vectorcraft_geom::Point, b: vectorcraft_geom::Point) -> bool {
    (a - b).hypot() < 1e-6
}

/// #4285: with the locator's top-left square chosen, a rotation typed in the Properties panel's
/// Transform section turns the selection about the box's top-left corner, as the Transform panel
/// does: on a plain rectangle, on one already turned 30°, and with the Transform panel open too.
#[test]
fn a_typed_rotation_turns_about_the_reference_point() {
    // (case, the angle the rectangle starts at, the panels, the angle typed)
    let cases: [(&str, f64, Draw, &str); 4] = [
        ("section", 0.0, transform_section, "45"),
        ("panel", 0.0, show, "45"),
        ("turned 30°", 30.0, transform_section, "60"),
        ("with the Transform panel", 0.0, properties_and_transform, "45"),
    ];
    for (case, start, draw, value) in cases {
        let mut panel = Panel::new(reference_setup(&[("object.rotate", json!({"angle": start}))]), draw);
        assert_eq!(panel.app().ui.transform_panel.reference_point, 0, "{case}: the locator's choice");
        let corner = panel.app().selection_box().unwrap().reference_point(0);
        panel.type_into("Rotate", value);
        let mut app = panel.app();
        let bx = app.selection_box().unwrap();
        assert!((bx.angle - value.parse::<f64>().unwrap()).abs() < 1e-6, "{case}: turned to {}", bx.angle);
        assert!(near(bx.reference_point(0), corner), "{case}: the top-left corner {corner:?} moved to {:?}", bx.reference_point(0));
    }
}

/// #4285: the flip buttons mirror the selection about the chosen reference point: with the
/// top-left square, across the box's left edge (left-right) or top edge (top-bottom).
#[test]
fn the_flip_buttons_mirror_about_the_reference_point() {
    for (button, want) in [("Flip Along Horizontal Axis", (0.0, 100.0, 100.0, 160.0)), ("Flip Along Vertical Axis", (100.0, 40.0, 200.0, 100.0))] {
        let mut panel = Panel::new(reference_setup(&[]), transform_section);
        panel.click(button);
        let r = panel.app().selection_box().unwrap().rect;
        let got = (r.x0, r.y0, r.x1, r.y1);
        assert!([got.0 - want.0, got.1 - want.1, got.2 - want.2, got.3 - want.3].iter().all(|d| d.abs() < 1e-6), "{button}: {got:?}, want {want:?}");
    }
}

/// With Use Registration Point for Symbol on, one symbol instance turns about its symbol's
/// registration point (here the bottom-right one) whatever the locator shows: the locator
/// follows the registration point.
#[test]
fn a_symbol_instance_turns_about_its_registration_point() {
    let mut app = reference_setup(&[]);
    let id = app.run("symbol.new", json!({"name": "Corner", "registration": 8})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.session.prefs.use_registration_point_for_symbol = true;
    let mut panel = Panel::new(app, transform_section);
    panel.type_into("Rotate", "45");
    let mut app = panel.app();
    let bx = app.selection_box().unwrap();
    assert!((bx.angle - 45.0).abs() < 1e-6, "turned to {}", bx.angle);
    assert!(near(bx.reference_point(8), vectorcraft_geom::Point::new(200.0, 160.0)), "the registration point moved to {:?}", bx.reference_point(8));
}

/// Ruler guides selected on their own (a click on one) show a Guides heading and a Transform
/// section, where X moves the vertical ones and Y the horizontal ones; with art selected too, the
/// panel shows the art's sections.
#[test]
fn selected_guides_show_their_position_in_a_transform_section() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let v = app.run("guide.add", json!({"vertical": true, "pos": 30})).unwrap()["index"].as_u64().unwrap();
    let h = app.run("guide.add", json!({"vertical": false, "pos": 20})).unwrap()["index"].as_u64().unwrap();
    let pos = |panel: &Panel, i: u64| panel.app().session.doc().unwrap().doc.ruler_guides()[i as usize].pos;
    let mut panel = Panel::new(app, show);
    for (index, name, before) in [(v, "X", 30.0), (h, "Y", 20.0)] {
        panel.app().run("guide.select", json!({"indexes": [index]})).unwrap();
        panel.frame();
        for shown in ["Guides", "Transform", "X", "Y"] {
            assert!(panel.shows(shown), "{shown:?}");
        }
        assert!(!panel.shows("Document"));
        panel.click(name);
        panel.key(Key::ArrowUp, Modifiers::NONE);
        panel.click("Guides");
        assert_eq!((pos(&panel, index) - before).abs(), 1.0, "{name} moves the selected guide");
    }
    // A guide and a rectangle: the rectangle's sections.
    let id = rect(&mut panel.app(), 10.0, 10.0, 30.0, 40.0);
    panel.app().run("select.set", json!({"ids": [id]})).unwrap();
    panel.app().run("guide.select", json!({"indexes": [v], "toggle": true})).unwrap();
    panel.frame();
    assert!(!panel.shows("Guides") && panel.shows("Width"));
}

/// Two 20 pt squares attached to a horizontal line in an Objects on Path group (`rotate` degrees),
/// with the group selected → the group.
fn objects_on_path(app: &mut VectorcraftApp, y: f64, rotate: f64) -> Value {
    let line = app.run("shape.line", json!({"x1": 20, "y1": y, "x2": 380, "y2": y})).unwrap()["id"].clone();
    let a = rect(app, 40.0, y - 60.0, 20.0, 20.0);
    let b = rect(app, 80.0, y - 60.0, 20.0, 20.0);
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let group = app.run("object.objectsOnPath.attach", json!({"path": line, "rotate": rotate})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    group
}

/// The Pivot and Rotate of the Objects on Path group `id`.
fn pivot_rotate(app: &VectorcraftApp, id: &Value) -> (u8, f64) {
    let d = &app.session.active().unwrap().doc;
    let rec = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().extras().objects_on_path.as_deref().unwrap();
    (rec.pivot, rec.rotate)
}

/// The Objects on Path Options section alone.
fn objects_on_path_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    // PLAN: assumed `objects_on_path_section` lives in `object_sections` beside `selected_objects_on_path`.
    if let Some(oop) = super::object_sections::selected_objects_on_path(app) {
        super::object_sections::objects_on_path_section(app, ui, &oop);
    }
}

/// The help page's Objects on Path Options section of the Properties panel shows for an Objects on
/// Path group selected on its own or with other such groups, and not for a plain group, an object
/// or an Objects on Path group selected with another object.
#[test]
fn objects_on_path_groups_show_objects_on_path_options() {
    let mut app = new_app();
    let group = objects_on_path(&mut app, 150.0, 0.0);
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Objects on Path Options") && panel.shows("Pivot") && panel.shows("Rotate"), "one group");
    let other = objects_on_path(&mut panel.app(), 250.0, 0.0);
    panel.app().run("select.set", json!({"ids": [group, other]})).unwrap();
    panel.frame();
    assert!(panel.shows("Objects on Path Options"), "two groups");
    let r = rect(&mut panel.app(), 300.0, 20.0, 40.0, 40.0);
    panel.app().run("select.set", json!({"ids": [group, r]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Objects on Path Options"), "a group and a rectangle");
    panel.app().run("select.set", json!({"ids": [r]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Objects on Path Options"), "a rectangle");
    let s = rect(&mut panel.app(), 340.0, 20.0, 20.0, 20.0);
    panel.app().run("select.set", json!({"ids": [r, s]})).unwrap();
    panel.app().run("object.group", json!({})).unwrap();
    panel.frame();
    assert!(!panel.shows("Objects on Path Options"), "a plain group");
}

/// A click on a Pivot square and a typed Rotate each set the selected groups' Objects on Path
/// Options in one undo step: one group's, and two groups' at once, where Rotate starts blank as
/// the groups' angles differ.
#[test]
fn objects_on_path_options_set_pivot_and_rotate_in_one_undo_step() {
    let mut app = reference_setup(&[]);
    let group = objects_on_path(&mut app, 150.0, 0.0);
    let mut panel = Panel::new(app, objects_on_path_options);
    let depth = undo_depth(&panel.app());
    panel.click("Top left");
    assert_eq!(pivot_rotate(&panel.app(), &group), (0, 0.0), "Pivot: top left");
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    panel.type_into("Rotate", "30");
    assert_eq!(pivot_rotate(&panel.app(), &group), (0, 30.0), "Rotate: 30°");
    assert_eq!(undo_depth(&panel.app()), depth + 2, "one undo step");
    let other = objects_on_path(&mut panel.app(), 250.0, 45.0);
    panel.app().run("select.set", json!({"ids": [group, other]})).unwrap();
    panel.frame();
    assert!(!matches!(panel.value("Rotate").as_deref(), Some("30°" | "45°")), "Rotate is blank");
    let depth = undo_depth(&panel.app());
    panel.click("Bottom right");
    assert_eq!(
        {
            let app = panel.app();
            (pivot_rotate(&app, &group), pivot_rotate(&app, &other))
        },
        ((8, 30.0), (8, 45.0)),
        "both groups' Pivot"
    );
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step for both groups");
}

/// The objects of the Objects on Path group `id`, in their order along the path.
fn on_path(app: &VectorcraftApp, id: &Value) -> Vec<u64> {
    let d = &app.session.active().unwrap().doc;
    let rec = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().extras().objects_on_path.as_deref().unwrap();
    rec.members.iter().map(|m| m.id.0).collect()
}

/// The help page's Detach in the Objects on Path Options section takes the object chosen with its
/// Select/Move widget off the path in one undo step and selects it. It is dimmed, and a click does
/// nothing, until one group is selected with an object chosen.
#[test]
fn detach_takes_the_chosen_object_off_the_path() {
    let mut app = reference_setup(&[]);
    let group = objects_on_path(&mut app, 150.0, 0.0);
    let [first, second] = on_path(&app, &group)[..] else { panic!("two objects") };
    let mut panel = Panel::new(app, objects_on_path_options);
    assert!(panel.shows("Detach"));
    let depth = undo_depth(&panel.app());
    assert!(!panel.enabled("Detach"));
    panel.click("Detach");
    assert_eq!(
        {
            let app = panel.app();
            (on_path(&app, &group), undo_depth(&app))
        },
        (vec![first, second], depth),
        "no object chosen: dimmed"
    );
    let other = objects_on_path(&mut panel.app(), 250.0, 0.0);
    panel.app().run("object.objectsOnPath.selectMember", json!({"member": first})).unwrap();
    panel.app().run("select.add", json!({"ids": [other]})).unwrap();
    panel.frame();
    assert!(!panel.enabled("Detach"));
    panel.click("Detach");
    assert_eq!(on_path(&panel.app(), &group), [first, second], "two groups: dimmed");
    panel.app().run("object.objectsOnPath.selectMember", json!({"member": first})).unwrap();
    panel.frame();
    assert!(panel.enabled("Detach"));
    let depth = undo_depth(&panel.app());
    panel.click("Detach");
    assert_eq!(on_path(&panel.app(), &group), [second]);
    assert_eq!(undo_depth(&panel.app()), depth + 1, "one undo step");
    let app = panel.app();
    let sel = &app.session.active().unwrap().selection;
    assert_eq!((sel.objects.iter().map(|i| i.0).collect::<Vec<_>>(), sel.on_path_member), (vec![first], None));
}

/// #3759: with area type selected, the Properties panel's Paragraph section shows Align Top, Align
/// Center, Align Bottom and Justify Vertically; a click sets the alignment on every selected frame
/// in one undo step. Point type shows none; frames with rows or columns and threaded frames turn
/// the buttons off, as the public documentation limits vertical alignment to them.
#[test]
fn vertical_alignment_buttons_align_the_selected_area_type() {
    use vectorcraft_doc::{NodeId, VerticalAlign};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 500, "height": 400})).unwrap();
    let make = |app: &mut VectorcraftApp, x: f64, y: f64| {
        app.run("text.create", json!({"x": x, "y": y, "text": "Panel", "area": {"width": 100, "height": 120}})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (make(&mut app, 10.0, 10.0), make(&mut app, 200.0, 10.0));
    let point = app.run("text.create", json!({"x": 10, "y": 350, "text": "Point"})).unwrap()["id"].as_u64().unwrap();
    let align = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.area.vertical_align,
        k => panic!("{k:?}"),
    };
    app.run("select.set", json!({"ids": [point]})).unwrap();
    assert_eq!(vertical_align::shown(&app), None, "point type: no buttons");
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(vertical_align::shown(&app), Some(vertical_align::Shown { shared: Some(VerticalAlign::Top), enabled: true }));
    let mut panel = Panel::new(app, type_sections);
    assert!(panel.shows("Paragraph"));
    let undo = undo_depth(&panel.app());
    panel.click("Align Center");
    assert_eq!(
        {
            let app = panel.app();
            (align(&app, a), align(&app, b))
        },
        (VerticalAlign::Center, VerticalAlign::Center)
    );
    assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step for both frames");
    // Differing alignments show none selected.
    panel.app().run("text.areaOptions", json!({"ids": [b], "verticalAlign": "bottom"})).unwrap();
    assert_eq!(vertical_align::shown(&panel.app()).map(|s| s.shared), Some(None));
    // Columns turn the buttons off, and a click does nothing.
    panel.app().run("text.areaOptions", json!({"ids": [b], "columns": 2})).unwrap();
    panel.frame();
    assert_eq!(vertical_align::shown(&panel.app()).map(|s| s.enabled), Some(false));
    assert!(!panel.enabled("Align Center"));
    let undo = undo_depth(&panel.app());
    panel.click("Align Center");
    assert_eq!(undo_depth(&panel.app()), undo, "turned off: no change");
    assert_eq!(align(&panel.app(), a), VerticalAlign::Center);
    // So does threading: a and c threaded, b left out of the selection.
    let c = make(&mut panel.app(), 10.0, 200.0);
    panel.app().run("select.set", json!({"ids": [a, c]})).unwrap();
    assert_eq!(vertical_align::shown(&panel.app()).map(|s| s.enabled), Some(true));
    panel.app().run("text.thread.create", json!({})).unwrap();
    panel.app().run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(vertical_align::shown(&panel.app()).map(|s| s.enabled), Some(false));
}

/// With area type selected, the Paragraph section shows a Dynamic Text checkbox, and Word Gap and
/// Line Gap while Block is on in every selected frame. Point type in the selection shows none;
/// threaded frames turn the checkbox off.
#[test]
fn dynamic_text_rows_follow_the_selected_area_type() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 500, "height": 400})).unwrap();
    let make = |app: &mut VectorcraftApp, x: f64, y: f64| {
        app.run("text.create", json!({"x": x, "y": y, "text": "Panel\ntext", "area": {"width": 100, "height": 120}})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (make(&mut app, 10.0, 10.0), make(&mut app, 200.0, 10.0));
    let point = app.run("text.create", json!({"x": 10, "y": 350, "text": "Point"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, point]})).unwrap();
    assert_eq!(dynamic_text::shown(&app), None, "point type selected too: no rows");
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let off = dynamic_text::Shown { on: Some(false), word_gap: Some(100.0), line_gap: Some(0.0), enabled: true };
    assert_eq!(dynamic_text::shown(&app), Some(off));
    let mut panel = Panel::new(app, type_sections);
    assert!(panel.shows("Dynamic Text") && !panel.shows("Word Gap"));
    panel.app().run("type.dynamicText", json!({"on": true, "lineGap": 2})).unwrap();
    let on = dynamic_text::Shown { on: Some(true), word_gap: Some(100.0), line_gap: Some(2.0), enabled: true };
    assert_eq!(dynamic_text::shown(&panel.app()), Some(on));
    panel.frame();
    assert!(panel.shows("Word Gap") && panel.shows("Line Gap"));
    // Differing gaps and settings show as mixed.
    panel.app().run("type.dynamicText", json!({"ids": [b], "wordGap": 200})).unwrap();
    assert_eq!(dynamic_text::shown(&panel.app()).map(|s| (s.on, s.word_gap, s.line_gap)), Some((Some(true), None, Some(2.0))));
    panel.app().run("type.dynamicText", json!({"ids": [b], "on": false})).unwrap();
    assert_eq!(dynamic_text::shown(&panel.app()).map(|s| (s.on, s.word_gap)), Some((None, None)));
    // A threaded frame turns the checkbox off.
    panel.app().run("text.thread.create", json!({})).unwrap();
    assert_eq!(dynamic_text::shown(&panel.app()).map(|s| (s.on, s.enabled)), Some((Some(false), false)));
}

/// A document with a graph of `kind` and three series, selected → the graph's id.
fn graph_doc(app: &mut VectorcraftApp, kind: &str, x: f64) -> u64 {
    if app.session.active().is_none() {
        app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    }
    let csv = ",a,b,c\nQ1,1,2,3\nQ2,3,4,5";
    app.run("graph.create", json!({"type": kind, "x": x, "y": 0, "width": 200, "height": 150, "csv": csv})).unwrap()["id"].as_u64().unwrap()
}

/// The ids of graph `graph`'s series groups, in series order.
fn series_groups(app: &VectorcraftApp, graph: u64) -> Vec<u64> {
    let st = app.session.active().unwrap();
    let mut groups: Vec<(u32, u64)> = st
        .doc
        .node(vectorcraft_doc::NodeId(graph))
        .and_then(vectorcraft_doc::Node::children)
        .into_iter()
        .flatten()
        .filter_map(|c| c.extras().series_index.map(|i| (i, c.id.0)))
        .collect();
    groups.sort_unstable();
    groups.into_iter().map(|(_, id)| id).collect()
}

/// Column… shows when every series it would change draws columns or bars, Marker… when every one
/// draws lines, scatter points or radar lines: series picked with Group Selection count on their
/// own, and a combined graph selected whole shows neither.
#[test]
fn the_graph_sections_column_and_marker_follow_the_series_they_would_change() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let g = graph_doc(&mut app, "column", 0.0);
    app.run("graph.setType", json!({"seriesIndexes": [1], "type": "line"})).unwrap();
    app.run("select.set", json!({"ids": [g]})).unwrap();
    assert_eq!(graph::shown(&app), Some(graph::Shown { column: false, marker: false }), "column and line series");
    let groups = series_groups(&app, g);
    assert_eq!(groups.len(), 3, "{groups:?}");
    app.run("select.set", json!({"ids": [groups[1]]})).unwrap();
    assert_eq!(graph::shown(&app), Some(graph::Shown { column: false, marker: true }), "the line series");
    app.run("select.set", json!({"ids": [groups[0], groups[2]]})).unwrap();
    assert_eq!(graph::shown(&app), Some(graph::Shown { column: true, marker: false }), "the column series");
    app.run("select.set", json!({"ids": [groups[0], groups[1]]})).unwrap();
    assert_eq!(graph::shown(&app), Some(graph::Shown { column: false, marker: false }), "one of each");
    for (kind, shown) in [
        ("line", (false, true)),
        ("scatter", (false, true)),
        ("radar", (false, true)),
        ("bar", (true, false)),
        ("pie", (false, false)),
        ("area", (false, false)),
    ] {
        let id = graph_doc(&mut app, kind, 300.0);
        app.run("select.set", json!({"ids": [id]})).unwrap();
        assert_eq!(graph::shown(&app).map(|s| (s.column, s.marker)), Some(shown), "{kind}");
        app.run("edit.clear", json!({})).unwrap();
    }
    // A graph whose data was cleared has no series for Column… or Marker… to change.
    for kind in ["column", "line"] {
        let id = graph_doc(&mut app, kind, 300.0);
        app.run("graph.setData", json!({"id": id, "series": [], "categories": [], "rows": []})).unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        assert_eq!(graph::shown(&app), Some(graph::Shown { column: false, marker: false }), "an empty {kind} graph");
        app.run("edit.clear", json!({})).unwrap();
    }
}

/// The Graph section shows only while everything selected is one graph or lies in it.
#[test]
fn the_graph_section_needs_one_graph_selected() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let a = graph_doc(&mut app, "column", 0.0);
    let b = graph_doc(&mut app, "column", 300.0);
    let rect = app.run("shape.rectangle", json!({"x": 600, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let mut panel = Panel::new(app, show);
    for (ids, what) in [(vec![rect], "a rectangle"), (vec![a, rect], "a graph and a rectangle"), (vec![a, b], "two graphs")] {
        panel.app().run("select.set", json!({ "ids": ids })).unwrap();
        panel.frame();
        assert_eq!(graph::shown(&panel.app()), None, "{what}");
        assert!(!panel.shows("Graph Type…"), "{what}");
    }
    let groups = series_groups(&panel.app(), a);
    panel.app().run("select.set", json!({"ids": [groups[0], groups[1]]})).unwrap();
    assert!(graph::shown(&panel.app()).is_some(), "two series of one graph");
    let other = series_groups(&panel.app(), b)[0];
    panel.app().run("select.set", json!({"ids": [groups[0], other]})).unwrap();
    assert_eq!(graph::shown(&panel.app()), None, "series of two graphs");
}

/// With the Type tool, the Character section's font style shows the style of the first selected
/// character (or of the character before the caret), not the text object's first run; with the
/// Selection tool, the first run's (#8189).
#[test]
fn character_section_shows_the_type_tool_selections_style() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.session.select_tool("type", Default::default()).unwrap();
    // Regular 0..8, Bold 8..13, Condensed 13..22.
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "Regular Bold Condensed", "font": TWO_AXIS_FAMILY})).unwrap()["id"].clone();
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 8, "end": 13, "style": "Bold"})).unwrap();
    app.run("text.setRangeStyle", json!({"id": id, "start": 13, "end": 22, "style": "Condensed"})).unwrap();
    let mut panel = Panel::new(app, type_sections);
    // The style the Style picker shows.
    let shown = |panel: &mut Panel| {
        panel.frame();
        panel.value("Style")
    };
    for (start, end, want) in [(8, 12, "Bold"), (13, 22, "Condensed"), (0, 7, "Regular"), (15, 15, "Condensed"), (13, 13, "Bold")] {
        panel.app().session.set_tool_option("select", &json!({"start": start, "end": end}));
        assert_eq!(crate::panels::character::text_editing(&panel.app()).map(|(_, a, b)| (a, b)), Some((start, end)));
        assert_eq!(shown(&mut panel).as_deref(), Some(want), "{start}..{end}");
    }
    panel.app().session.select_tool("selection", Default::default()).unwrap();
    panel.app().run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(crate::panels::character::text_editing(&panel.app()), None);
    assert_eq!(shown(&mut panel).as_deref(), Some("Regular"), "the Selection tool shows the first run's style");
}

/// With type selected, Quick Actions offers Create Outlines, Retype and Find and Replace… (G12): Create Outlines
/// outlines the type in one undo step, Find and Replace… opens its dialog and Retype shows the Retype panel, leaving it
/// shown when clicked again. A path offers Offset Path and Simplify instead of Create Outlines.
#[test]
fn type_quick_actions_show_text_commands_and_create_outlines() {
    use craft::text::ui::find::FindReplaceDialog;
    let mut app = new_app();
    let t = app.run("text.create", json!({"x": 50, "y": 200, "text": "Hello"})).unwrap()["id"].clone();
    let mut panel = Panel::new(app, show);
    for name in ["Create Outlines", "Retype", "Find and Replace…", "Arrange: Bring to Front", "Lock"] {
        assert!(panel.shows(name), "type shows {name}");
    }
    assert!(!panel.shows("Offset Path") && !panel.shows("Simplify"), "no Offset Path or Simplify for type alone");
    let undo = undo_depth(&panel.app());
    panel.click("Create Outlines");
    assert_eq!(undo_depth(&panel.app()), undo + 1, "one undo step");
    {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert!(!st.selection.objects.is_empty(), "the outlines are selected");
        for id in &st.selection.objects {
            assert!(!matches!(st.doc.node(*id).unwrap().kind, NodeKind::Text(_)), "outlined, not type");
        }
    }
    panel.app().run("edit.undo", json!({})).unwrap();
    {
        let app = panel.app();
        let node = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(t.as_u64().unwrap())).cloned().unwrap();
        assert!(matches!(node.kind, NodeKind::Text(_)), "undo brings the type back");
    }
    let r = rect(&mut panel.app(), 20.0, 20.0, 40.0, 40.0);
    panel.app().run("select.set", json!({"ids": [r]})).unwrap();
    panel.frame();
    assert!(panel.shows("Offset Path") && panel.shows("Simplify"), "a path shows Offset Path and Simplify");
    assert!(!panel.shows("Create Outlines"), "a path shows no Create Outlines");
    panel.app().run("select.set", json!({"ids": [t]})).unwrap();
    let ctx = panel.ctx().clone();
    panel.app().ui_ctx = Some(ctx);
    panel.frame();
    panel.click("Find and Replace…");
    let open = panel.app().dialogs().expect("a window").with(|host| host.get::<FindReplaceDialog>().is_some());
    assert!(open, "Find and Replace… opens its dialog");
    let retype = crate::panels::retype::ID;
    assert!(!crate::panel_docking::visible(&panel.app(), retype), "Retype starts hidden");
    panel.click("Retype");
    assert!(crate::panel_docking::visible(&panel.app(), retype), "Retype shows the Retype panel");
    panel.click("Retype");
    assert!(crate::panel_docking::visible(&panel.app(), retype), "a second click leaves it shown");
}

#[test]
fn canceled_property_corner_kind_keeps_the_selected_anchors_and_undo() {
    use craft::ui::contract::Role;
    use vectorcraft_doc::{LiveCorners, NodeId};
    use vectorcraft_geom::shapes::CornerKind;

    let mut app = new_app();
    let id = app.run("shape.star", json!({"cx": 200, "cy": 150, "radius1": 80, "radius2": 40})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.room(egui::vec2(420.0, 920.0));
    panel.type_into("Corner Radius", "6");
    let original = panel.app().session.active().unwrap().doc.clone();
    let original_corners = LiveCorners::of(original.node(NodeId(id)).unwrap()).unwrap();
    assert_eq!(original_corners.radius(2), 6.0);
    assert_eq!(original_corners.kind(2), CornerKind::Round);
    let undo = undo_depth(&panel.app());

    panel.click_role("Corner", Role::COMBO_BOX);
    panel.key(Key::ArrowDown, Modifiers::NONE);
    assert!(panel.app().session.active().unwrap().doc == original, "highlighting a kind leaves the star as it was");
    panel.key(Key::Escape, Modifiers::NONE);
    assert!(panel.app().session.active().unwrap().doc == original);
    assert_eq!(undo_depth(&panel.app()), undo);

    panel.click_role("Corner", Role::COMBO_BOX);
    panel.key(Key::ArrowDown, Modifiers::NONE);
    panel.key(Key::Enter, Modifiers::NONE);
    {
        let app = panel.app();
        let doc = &app.session.active().unwrap().doc;
        let corners = LiveCorners::of(doc.node(NodeId(id)).unwrap()).unwrap();
        assert_eq!(corners.kind(2), CornerKind::ALL[1]);
        for corner in [0, 1, 3, 4, 5, 6, 7, 8, 9] {
            assert_eq!(corners.kind(corner), original_corners.kind(corner));
        }
        assert_eq!(corners.radius(2), 6.0);
        assert_eq!(undo_depth(&app), undo + 1);
    }
    panel.app().run("edit.undo", json!({})).unwrap();
    assert!(panel.app().session.active().unwrap().doc == original);
}
