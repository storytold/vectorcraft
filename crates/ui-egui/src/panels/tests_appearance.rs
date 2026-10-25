//! Headless frames of the Appearance panel and the panels that follow its active item.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::Session;

use super::*;

pub(super) fn app_with_rect() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    app
}

pub(super) fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.session.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Every text drawn while `f` runs one headless frame.
fn frame_texts(app: &mut VectorcraftApp, f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<String> {
    let ctx = egui::Context::default();
    frame_in(&ctx, app, f)
}

fn frame_in(ctx: &egui::Context, app: &mut VectorcraftApp, f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<String> {
    frame_events(ctx, app, vec![], f).into_iter().map(|(s, _)| s).collect()
}

/// One headless frame with input `events`: every text drawn, with its screen rect.
pub(super) fn frame_events(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    events: Vec<egui::Event>,
    f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<(String, egui::Rect)> {
    frame_raw(ctx, app, egui::RawInput { events, ..Default::default() }, f)
}

/// One headless frame with input `raw`: every text drawn, with its screen rect.
pub(super) fn frame_raw(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    raw: egui::RawInput,
    mut f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<(String, egui::Rect)> {
    let mut out = ctx.run_ui(raw, |ui| f(app, ui));
    out.textures_delta.clear();
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

/// Click at `pos` over three frames of `f` (hover, press, release).
pub(super) fn click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2, mut f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
        frame_events(ctx, app, vec![e], &mut f);
    }
}

/// The rect of the first drawn text equal to `s`.
pub(super) fn text_rect(texts: &[(String, egui::Rect)], s: &str) -> egui::Rect {
    texts.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("`{s}` not drawn: {texts:?}")).1
}

#[test]
fn stroke_panel_shows_the_active_rows_weight() {
    let mut app = app_with_rect();
    run(&mut app, "appearance.addStroke", json!({})); // [Fill, Stroke 1 pt, Stroke 1 pt]
    run(&mut app, "stroke.set", json!({"item": 1, "weight": 6}));
    let six = Unit::Points.format(6.0);
    let texts = frame_texts(&mut app, stroke::show);
    assert!(!texts.contains(&six), "top stroke shown: {texts:?}");
    // Clicking the lower stroke row in the Appearance panel targets it.
    let ctx = egui::Context::default();
    frame_in(&ctx, &mut app, appearance::show);
    appearance::select_row(&mut app, &ctx, appearance::Sel::Item(1));
    assert_eq!(app.session.appearance_item(), Some(1));
    assert!(frame_texts(&mut app, stroke::show).contains(&six));
    let (_, stroke) = current_paints(&app);
    assert_eq!(stroke, current_stroke(&app).unwrap().paint);
    // The Appearance panel and the proxies draw with the row active; the object row clears it.
    frame_in(&ctx, &mut app, |app, ui| {
        appearance::show(app, ui);
        crate::toolbar::show(app, ui);
        transparency::show(app, ui);
        properties::show(app, ui);
    });
    appearance::select_row(&mut app, &ctx, appearance::Sel::None);
    assert_eq!(app.session.appearance_item(), None);
    assert!(!frame_texts(&mut app, stroke::show).contains(&six));
}

/// Cmd+/ is Add New Fill and Cmd+Alt+/ is Add New Stroke by default. On Windows and Linux the keys
/// are Ctrl+/ and Ctrl+Alt+/, and on macOS they are Command+/ and Option+Command+/. No other
/// command, tool or panel has either key by default.
#[test]
fn cmd_slash_is_add_new_fill_and_cmd_alt_slash_add_new_stroke() {
    use crate::shortcut_editor::{command_for_key, default_command_shortcut};
    let mac = cfg!(target_os = "macos");
    let (cmd, alt) = (egui::Modifiers::COMMAND, egui::Modifiers::ALT);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for (id, key, label, mods, shown) in [
        ("appearance.addFill", "Cmd+/", "Add New Fill", cmd, if mac { "⌘/" } else { "Ctrl+/" }),
        ("appearance.addStroke", "Cmd+Alt+/", "Add New Stroke", cmd | alt, if mac { "⌥⌘/" } else { "Ctrl+Alt+/" }),
    ] {
        assert_eq!(default_command_shortcut(id), Some(key));
        assert_eq!(command_for_key(key), Some(id));
        let chord = crate::shortcuts::parse(key).expect("parses");
        assert_eq!((chord.modifiers, chord.logical_key), (mods, egui::Key::Slash));
        assert_eq!(crate::menus::pretty_shortcut(key), shown);
        let listed = app.run("shortcuts.list", json!({ "query": id })).unwrap();
        let rows: Vec<_> = listed.as_array().expect("rows").iter().map(|r| (r["label"].clone(), r["group"].clone(), r["default"].clone())).collect();
        assert_eq!(rows, [(json!(label), json!("Window › Appearance"), json!(key))], "Keyboard Shortcuts lists {id}");
    }
    assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has either key");
}

/// Cmd+/ and Cmd+Alt+/, pressed through the shortcut handler, add a fill and a stroke to the
/// selected objects as the Appearance panel's Add New Fill and Add New Stroke buttons do, on a path
/// and on type. With nothing selected the buttons are off, and the keys leave the objects as they are.
#[test]
fn cmd_slash_adds_a_fill_and_cmd_alt_slash_a_stroke_as_the_panel_buttons_do() {
    fn press(app: &mut VectorcraftApp, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        let ev = egui::Event::Key { key: egui::Key::Slash, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
        out.textures_delta.clear();
    }
    /// The Appearance panel's button named `label`: the center of its rect and whether it is on.
    fn button(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) -> (egui::Pos2, bool) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| appearance::show(app, ui));
        out.textures_delta.clear();
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let (_, n) = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label) && n.role() == egui::accesskit::Role::Button)
            .unwrap_or_else(|| panic!("no button named {label:?}"));
        let b = n.bounds().expect("bounds");
        (egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5), !n.is_disabled())
    }
    /// A rectangle and a line of type, with the rectangle (`type_too`: both) selected.
    fn two_objects(type_too: bool) -> (VectorcraftApp, Vec<Value>) {
        let mut app = app_with_rect();
        let rect = first_selected(&app).expect("the rectangle").id.0;
        let text = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi"}))["id"].clone();
        let ids = if type_too { vec![json!(rect), text.clone()] } else { vec![json!(rect)] };
        run(&mut app, "select.set", json!({ "ids": ids }));
        (app, vec![json!(rect), text])
    }
    let stacks = |app: &VectorcraftApp, ids: &[Value]| -> Vec<vectorcraft_doc::Appearance> {
        let d = &app.session.active().expect("a document").doc;
        ids.iter().map(|id| (*d.node(vectorcraft_doc::NodeId(id.as_u64().expect("an id"))).expect("the object").appearance).clone()).collect()
    };
    let cmd = egui::Modifiers::COMMAND;
    for (type_too, mods, label, fill) in [
        (false, cmd, "Add New Fill", true),
        (false, cmd | egui::Modifiers::ALT, "Add New Stroke", false),
        (true, cmd, "Add New Fill", true),
        (true, cmd | egui::Modifiers::ALT, "Add New Stroke", false),
    ] {
        let (mut keyed, ids) = two_objects(type_too);
        let before = stacks(&keyed, &ids);
        press(&mut keyed, mods);
        let (mut clicked, _) = two_objects(type_too);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let (at, on) = button(&ctx, &mut clicked, label);
        assert!(on, "{label} is on with a selection");
        click(&ctx, &mut clicked, at, appearance::show);
        let after = stacks(&keyed, &ids);
        assert_eq!(after, stacks(&clicked, &ids), "{label}: the key adds what the button adds");
        let selected = if type_too { 2 } else { 1 };
        for (i, (a, b)) in after.iter().zip(&before).enumerate() {
            let added = if i < selected { 1 } else { 0 };
            assert_eq!(a.items.len(), b.items.len() + added, "{label}, object {i}: {a:?}");
            if added == 1 {
                assert_eq!(a.items.last().map(|it| it.is_fill()), Some(fill), "{label}: on top of the stack");
            }
        }
    }
    // With nothing selected, both buttons are off and neither key changes the objects.
    let (mut app, ids) = two_objects(true);
    run(&mut app, "select.none", json!({}));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    assert!(!button(&ctx, &mut app, "Add New Fill").1 && !button(&ctx, &mut app, "Add New Stroke").1);
    let before = stacks(&app, &ids);
    press(&mut app, cmd);
    press(&mut app, cmd | egui::Modifiers::ALT);
    assert_eq!(stacks(&app, &ids), before);
}

/// With nothing selected, Add New Fill and Add New Stroke are off in menus and the command palette
/// as the Appearance panel's buttons are, and their keys run nothing (no error in the status bar);
/// an empty `ids` list runs nothing either. Given ids, they still add to those objects.
#[test]
fn add_new_fill_and_stroke_are_off_without_a_selection() {
    let mut app = app_with_rect();
    let rect = first_selected(&app).expect("the rectangle").id;
    let items = |app: &VectorcraftApp| app.session.active().expect("a document").doc.node(rect).expect("the rectangle").appearance.items.len();
    assert!(crate::menus::enabled(&app, "appearance.addFill") && crate::menus::enabled(&app, "appearance.addStroke"));
    run(&mut app, "select.none", json!({}));
    let before = items(&app);
    for (id, modifiers) in
        [("appearance.addFill", egui::Modifiers::COMMAND), ("appearance.addStroke", egui::Modifiers::COMMAND | egui::Modifiers::ALT)]
    {
        assert!(!crate::menus::enabled(&app, id), "{id} is off");
        let ctx = egui::Context::default();
        let ev = egui::Event::Key { key: egui::Key::Slash, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
        out.textures_delta.clear();
        crate::menus::invoke(&mut app, id, json!({}));
        assert_eq!((items(&app), app.ui.status.as_str()), (before, ""), "{id} ran nothing");
        crate::menus::invoke(&mut app, id, json!({"ids": []}));
        assert_eq!((items(&app), app.ui.status.as_str()), (before, ""), "{id} with no ids ran nothing");
    }
    crate::menus::invoke(&mut app, "appearance.addFill", json!({"ids": [rect.0]}));
    assert_eq!((items(&app), app.ui.status.as_str()), (before + 1, ""), "given ids, it adds a fill");
}

#[test]
fn transparency_panel_reads_the_active_item() {
    let mut app = app_with_rect();
    run(&mut app, "transparency.set", json!({"item": 0, "opacity": 40}));
    assert_eq!(current_transparency(&app).unwrap().0, 1.0);
    run(&mut app, "appearance.setActiveItem", json!({"index": 0}));
    assert!((current_transparency(&app).unwrap().0 - 0.4).abs() < 1e-6);
    assert!(frame_texts(&mut app, transparency::show).iter().any(|t| t.starts_with("40")));
}

#[test]
fn item_effect_rows_toggle_their_eye() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    let ctx = egui::Context::default();
    // The stroke row's disclosure is open: its Opacity sub-row and its effect show under it.
    super::set_pstate(&ctx, "ap-open-1", true);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    let label = text_rect(&texts, "Roughen");
    // The eye sits in the first column of the effect's row (the label is indented one level).
    let eye = egui::pos2(label.left() - 12.0 - 24.0 - appearance::EYE_W / 2.0, label.center().y);
    click(&ctx, &mut app, eye, appearance::show);
    let visible = |app: &VectorcraftApp| current_stroke_effects(app)[0].visible;
    assert!(!visible(&app));
    click(&ctx, &mut app, eye, appearance::show);
    assert!(visible(&app));
    // Clicking the effect's row selects it and makes its stroke the active item.
    click(&ctx, &mut app, egui::pos2(label.right() + 40.0, label.center().y), appearance::show);
    assert_eq!(app.session.appearance_item(), Some(1));
}

/// The effects of the selected object's stroke (item 1).
fn current_stroke_effects(app: &VectorcraftApp) -> Vec<vectorcraft_doc::Effect> {
    first_selected(app).unwrap().appearance.items[1].effects().clone()
}

/// The blend mode list's separators come before the first mode of each group.
#[test]
fn blend_separators_follow_the_groups() {
    use vectorcraft_color::BlendMode;
    let starts: Vec<BlendMode> =
        (0..BlendMode::ALL.len()).filter(|i| crate::widgets::blend_separator_before(*i)).map(|i| BlendMode::ALL[i]).collect();
    assert_eq!(starts, [BlendMode::Darken, BlendMode::Lighten, BlendMode::Overlay, BlendMode::Difference, BlendMode::Hue]);
    assert!(starts.iter().all(|m| m.group() > 0) && !crate::widgets::blend_separator_before(BlendMode::ALL.len()));
}

/// An Opacity row's empty space (its value, "Default") opens nothing: neither a popup nor a panel.
/// Only its "Opacity:" link edits it (the Transparency panel).
#[test]
fn an_opacity_rows_empty_space_opens_nothing() {
    let mut app = app_with_rect();
    let ctx = egui::Context::default();
    super::set_pstate(&ctx, "ap-open-1", true);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    for value in [texts.iter().find(|(t, _)| t == "Default"), texts.iter().rev().find(|(t, _)| t == "Default")] {
        let value = value.expect("an Opacity row").1;
        click(&ctx, &mut app, value.center(), appearance::show);
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        assert!(!texts.iter().any(|(t, _)| t == "Normal"), "no popup with the blend mode: {texts:?}");
        assert_eq!(app.ui.transient_panel, None);
    }
}

/// The object row names the selection as the Control bar does.
#[test]
fn object_row_names_the_selection_like_the_control_bar() {
    let mut app = app_with_rect();
    let a = first_selected(&app).unwrap().id.0;
    let ctx = egui::Context::default();
    assert!(frame_in(&ctx, &mut app, appearance::show).contains(&"Rectangle".to_string()));
    let b = run(&mut app, "shape.rectangle", json!({"x": 120, "y": 10, "width": 50, "height": 50}))["id"].clone();
    // Objects of one kind with one appearance are named by their kind and list it (#4174).
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    let texts = frame_in(&ctx, &mut app, appearance::show);
    assert!(texts.contains(&"Rectangle".to_string()) && texts.contains(&"Stroke:".to_string()), "{texts:?}");
    assert!(!texts.contains(&"Mixed Appearances".to_string()), "{texts:?}");
    // Objects whose opacity differs list what they share, the opacity as mixed.
    run(&mut app, "select.set", json!({"ids": [b]}));
    run(&mut app, "transparency.set", json!({"opacity": 50}));
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    let texts = frame_in(&ctx, &mut app, appearance::show);
    assert!(texts.contains(&"Mixed Appearances".to_string()) && texts.contains(&"Stroke:".to_string()), "{texts:?}");
    assert!(texts.contains(&"Mixed".to_string()), "the opacity reads Mixed: {texts:?}");
    // Type objects are "Type" in both places.
    let t = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi"}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [t] }));
    assert_eq!(appearance::object_label(&app), "Type");
    assert!(frame_in(&ctx, &mut app, appearance::show).contains(&"Type".to_string()));
    let fonts = egui::Context::default();
    crate::theme::install_fonts(&fonts);
    assert!(frame_in(&fonts, &mut app, crate::chrome::control_bar).contains(&"Type".to_string()));
}

/// The Appearance panel's button named `label`: the center of its rect (`ctx` needs accesskit on).
fn panel_button(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) -> egui::Pos2 {
    panel_button_in(ctx, app, label, appearance::show)
}

/// [`panel_button`] in a frame of `f`.
fn panel_button_in(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, f: fn(&mut VectorcraftApp, &mut egui::Ui)) -> egui::Pos2 {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(app, ui));
    out.textures_delta.clear();
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let (_, n) = nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label) && n.role() == egui::accesskit::Role::Button)
        .unwrap_or_else(|| panic!("no button named {label:?}"));
    let b = n.bounds().expect("bounds");
    egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5)
}

/// Drag from `a` to `b` in the Appearance panel: press, move past the drag threshold, move, release.
fn drag_in_panel(ctx: &egui::Context, app: &mut VectorcraftApp, a: egui::Pos2, b: egui::Pos2) {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [
        vec![egui::Event::PointerMoved(a)],
        vec![button(a, true)],
        vec![egui::Event::PointerMoved(a + egui::vec2(0.0, 6.0))],
        vec![egui::Event::PointerMoved(b)],
        vec![button(b, false)],
        vec![],
    ] {
        frame_events(ctx, app, e, appearance::show);
    }
}

/// A point on the row whose label is the `nth` drawn `label`, clear of its eye, swatch and fields.
fn row_point(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, nth: usize) -> egui::Pos2 {
    let texts = frame_events(ctx, app, vec![], appearance::show);
    let r = texts.iter().filter(|(t, _)| t == label).nth(nth).unwrap_or_else(|| panic!("no {label:?} #{nth}: {texts:?}")).1;
    egui::pos2(r.left() + 400.0, r.center().y)
}

/// The selected object's fills/strokes as (is fill, stroke weight).
fn stack_items(app: &VectorcraftApp) -> Vec<(bool, Option<f32>)> {
    first_selected(app)
        .unwrap()
        .appearance
        .items
        .iter()
        .map(|it| match it {
            vectorcraft_doc::AppearanceItem::Stroke(s) => (false, Some(s.width as f32)),
            _ => (true, None),
        })
        .collect()
}

/// A context with accessibility on, so tests find buttons by their labels.
fn drop_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    ctx
}

/// Fill and stroke rows dropped on Delete Selected Item are removed, on Duplicate Selected Item
/// copied, as rows dropped on the Layers panel's buttons are.
#[test]
fn rows_dropped_on_delete_are_removed_and_on_duplicate_copied() {
    // [Fill, Stroke]: dropping the stroke on Delete leaves the fill.
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let stroke = row_point(&ctx, &mut app, "Stroke:", 0);
    drag_in_panel(&ctx, &mut app, stroke, delete);
    assert_eq!(stack_items(&app), [(true, None)]);
    // Dropping the fill on Duplicate adds one copy of it.
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    let duplicate = panel_button(&ctx, &mut app, "Duplicate Selected Item");
    let fill = row_point(&ctx, &mut app, "Fill:", 0);
    drag_in_panel(&ctx, &mut app, fill, duplicate);
    let items = stack_items(&app);
    assert_eq!(items.iter().filter(|(f, _)| *f).count(), 2, "{items:?}");
    assert_eq!(items.len(), 3, "{items:?}");
}

/// Dragging one of the selected rows onto Delete removes every selected row; dragging a row outside
/// the selection removes only that row.
#[test]
fn dropping_a_selected_row_acts_on_the_selection() {
    let setup = || {
        // [Fill, Stroke 1 pt, Stroke 6 pt]
        let mut app = app_with_rect();
        run(&mut app, "appearance.addStroke", json!({}));
        run(&mut app, "stroke.set", json!({"item": 2, "weight": 6}));
        let ctx = drop_ctx();
        frame_events(&ctx, &mut app, vec![], appearance::show);
        // Both strokes highlighted (the top one, then Ctrl-click the lower one).
        let (top, lower) = (row_point(&ctx, &mut app, "Stroke:", 0), row_point(&ctx, &mut app, "Stroke:", 1));
        click_mod(&ctx, &mut app, top, egui::Modifiers::NONE);
        click_mod(&ctx, &mut app, lower, egui::Modifiers::COMMAND);
        (app, ctx)
    };
    // The top stroke row (drawn first) is in the selection: both strokes go.
    let (mut app, ctx) = setup();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let top = row_point(&ctx, &mut app, "Stroke:", 0);
    drag_in_panel(&ctx, &mut app, top, delete);
    assert_eq!(stack_items(&app), [(true, None)]);
    // The fill is not: only it goes.
    let (mut app, ctx) = setup();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let fill = row_point(&ctx, &mut app, "Fill:", 0);
    drag_in_panel(&ctx, &mut app, fill, delete);
    assert_eq!(stack_items(&app), [(false, Some(1.0)), (false, Some(6.0))]);
    // Onto Duplicate, the selected strokes are both copied.
    let (mut app, ctx) = setup();
    let duplicate = panel_button(&ctx, &mut app, "Duplicate Selected Item");
    let top = row_point(&ctx, &mut app, "Stroke:", 0);
    drag_in_panel(&ctx, &mut app, top, duplicate);
    let items = stack_items(&app);
    assert_eq!(items.len(), 5, "{items:?}");
    assert_eq!(items.iter().filter(|(_, w)| *w == Some(6.0)).count(), 2, "{items:?}");
}

/// An effect row dropped on Delete removes that effect, on Duplicate copies it; the stroke it
/// belongs to stays.
#[test]
fn effect_rows_dropped_on_delete_or_duplicate() {
    for (button, left) in [("Delete Selected Item", 0), ("Duplicate Selected Item", 2)] {
        let mut app = app_with_rect();
        run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
        let ctx = drop_ctx();
        super::set_pstate(&ctx, "ap-open-1", true);
        let at = panel_button(&ctx, &mut app, button);
        let fx = row_point(&ctx, &mut app, "Roughen", 0);
        drag_in_panel(&ctx, &mut app, fx, at);
        assert_eq!(current_stroke_effects(&app).len(), left, "{button}");
        assert_eq!(stack_items(&app).len(), 2, "{button}: the fill and stroke stay");
    }
}

/// The Contents row can be neither copied nor removed: dropped on either button it changes nothing.
/// A row released outside the panel changes nothing either, and the next drop still works.
#[test]
fn contents_rows_and_drops_elsewhere_change_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    let b = run(&mut app, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "object.group", json!({}));
    run(&mut app, "appearance.addFill", json!({}));
    let ctx = drop_ctx();
    let before = first_selected(&app).unwrap().appearance.clone();
    for button in ["Delete Selected Item", "Duplicate Selected Item"] {
        let at = panel_button_in(&ctx, &mut app, button, boxed_panel);
        let contents = row_point(&ctx, &mut app, "Contents", 0);
        drag_in_panel(&ctx, &mut app, contents, at);
        assert_eq!(first_selected(&app).unwrap().appearance, before, "{button}");
    }
    // Released far below the panel: nothing happens.
    let fill = row_point(&ctx, &mut app, "Fill:", 0);
    drag_in_panel(&ctx, &mut app, fill, egui::pos2(fill.x, 9000.0));
    assert_eq!(first_selected(&app).unwrap().appearance, before);
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let fill = row_point(&ctx, &mut app, "Fill:", 0);
    drag_in_panel(&ctx, &mut app, fill, delete);
    assert!(first_selected(&app).unwrap().appearance.items.is_empty());
}

/// A row held over Delete Selected Item outlines the button as the Layers panel outlines its trash
/// (1.5 px accent, corner radius 3).
#[test]
fn a_row_held_over_delete_outlines_it_as_layers_does() {
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let stroke = row_point(&ctx, &mut app, "Stroke:", 0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [vec![egui::Event::PointerMoved(stroke)], vec![button(stroke, true)], vec![egui::Event::PointerMoved(stroke + egui::vec2(0.0, 6.0))]] {
        frame_events(&ctx, &mut app, e, appearance::show);
    }
    let mut out =
        ctx.run_ui(egui::RawInput { events: vec![egui::Event::PointerMoved(delete)], ..Default::default() }, |ui| appearance::show(&mut app, ui));
    out.textures_delta.clear();
    let accent = crate::theme::Tokens::get(&ctx).accent;
    let outlines: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.stroke.color == accent && r.rect.contains(delete) => Some((r.stroke.width, r.corner_radius)),
            _ => None,
        })
        .collect();
    assert_eq!(outlines, [(1.5, egui::CornerRadius::same(3))]);
}

/// Click at `pos` with modifiers `m` held (hover, press, release).
fn click_mod(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2, m: egui::Modifiers) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: m };
    for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
        frame_events(ctx, app, vec![egui::Event::ModifiersChanged(m), e], appearance::show);
    }
    frame_events(ctx, app, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)], appearance::show);
}

/// The selected object's own effect ids, in order.
fn object_effects(app: &VectorcraftApp) -> Vec<String> {
    first_selected(app).unwrap().appearance.effects.iter().map(|e| e.id.clone()).collect()
}

/// A rectangle with Outer Glow, Drop Shadow and Twist on the object.
fn rect_with_three_effects() -> VectorcraftApp {
    let mut app = app_with_rect();
    for e in ["stylize.outerGlow", "stylize.dropShadow", "distort.twist"] {
        run(&mut app, "effect.apply", json!({"effect": e, "item": null}));
    }
    app
}

/// The owner's case: the stack Stroke, Fill, Outer Glow, Gaussian Blur-like effects; Ctrl-clicking
/// two of the object's effects highlights both, and Delete Selected Item removes both in one undo
/// step, leaving the fill and stroke.
#[test]
fn ctrl_clicked_effects_are_deleted_together() {
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let glow = row_point(&ctx, &mut app, "Outer Glow", 0);
    let shadow = row_point(&ctx, &mut app, "Drop Shadow", 0);
    click_mod(&ctx, &mut app, glow, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, shadow, egui::Modifiers::COMMAND);
    click(&ctx, &mut app, delete, appearance::show);
    assert_eq!(object_effects(&app), ["distort.twist"]);
    assert_eq!(stack_items(&app).len(), 2, "the fill and stroke stay");
    run(&mut app, "edit.undo", json!({}));
    assert_eq!(object_effects(&app).len(), 3, "one undo step");
}

/// Shift ranges from the anchor (the last row clicked without Shift), Shift+Ctrl adds a range, and
/// Ctrl can take the highlight off the last row, as in the Layers panel.
#[test]
fn effect_rows_follow_the_layers_click_rules() {
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let row = |ctx: &egui::Context, app: &mut VectorcraftApp, name: &str| row_point(ctx, app, name, 0);
    // Shift from Outer Glow to Twist: all three.
    let (glow, twist) = (row(&ctx, &mut app, "Outer Glow"), row(&ctx, &mut app, "Twist"));
    click_mod(&ctx, &mut app, glow, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, twist, egui::Modifiers::SHIFT);
    click(&ctx, &mut app, delete, appearance::show);
    assert!(object_effects(&app).is_empty());
    run(&mut app, "edit.undo", json!({}));
    // Twist, then Shift+Ctrl from it to Drop Shadow adds that range: Shadow and Twist; Glow stays.
    let (shadow, twist) = (row(&ctx, &mut app, "Drop Shadow"), row(&ctx, &mut app, "Twist"));
    click_mod(&ctx, &mut app, twist, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, shadow, egui::Modifiers { shift: true, command: true, ..Default::default() });
    click(&ctx, &mut app, delete, appearance::show);
    assert_eq!(object_effects(&app), ["stylize.outerGlow"]);
    run(&mut app, "edit.undo", json!({}));
    // Ctrl-clicking the only highlighted effect takes its highlight off: Delete removes nothing.
    let glow = row(&ctx, &mut app, "Outer Glow");
    click_mod(&ctx, &mut app, glow, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, glow, egui::Modifiers::COMMAND);
    click(&ctx, &mut app, delete, appearance::show);
    assert_eq!(object_effects(&app).len(), 3);
}

/// Ctrl-clicking an effect of another list (the stroke's) starts a new selection there: Delete then
/// removes only that effect.
#[test]
fn ctrl_clicking_an_effect_of_another_list_starts_a_new_selection() {
    let mut app = rect_with_three_effects();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    let ctx = drop_ctx();
    super::set_pstate(&ctx, "ap-open-1", true);
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let glow = row_point(&ctx, &mut app, "Outer Glow", 0);
    click_mod(&ctx, &mut app, glow, egui::Modifiers::NONE);
    let roughen = row_point(&ctx, &mut app, "Roughen", 0);
    click_mod(&ctx, &mut app, roughen, egui::Modifiers::COMMAND);
    click(&ctx, &mut app, delete, appearance::show);
    assert!(current_stroke_effects(&app).is_empty());
    assert_eq!(object_effects(&app).len(), 3);
}

/// Fill/stroke rows use the same rules: Shift ranges from the anchor, not from the active row.
#[test]
fn fill_and_stroke_rows_range_from_the_anchor() {
    // [Fill, Stroke 1 pt, Stroke 6 pt, Stroke 3 pt], listed top-down 3, 6, 1, Fill.
    let mut app = app_with_rect();
    run(&mut app, "appearance.addStroke", json!({}));
    run(&mut app, "stroke.set", json!({"item": 2, "weight": 6}));
    run(&mut app, "appearance.addStroke", json!({}));
    run(&mut app, "stroke.set", json!({"item": 3, "weight": 3}));
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let stroke = |ctx: &egui::Context, app: &mut VectorcraftApp, nth| row_point(ctx, app, "Stroke:", nth);
    let (s1, s3) = (stroke(&ctx, &mut app, 2), stroke(&ctx, &mut app, 0));
    click_mod(&ctx, &mut app, s1, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, s3, egui::Modifiers::COMMAND);
    // Ctrl-click S3 again: off, but it stays the anchor; S1 is the active row.
    click_mod(&ctx, &mut app, s3, egui::Modifiers::COMMAND);
    let fill = row_point(&ctx, &mut app, "Fill:", 0);
    click_mod(&ctx, &mut app, fill, egui::Modifiers::SHIFT);
    click(&ctx, &mut app, delete, appearance::show);
    assert!(stack_items(&app).is_empty(), "the range from S3 down to the fill: all four");
}

/// The panel in a 300×500 pt area at the top left, so clicks can land outside it.
fn boxed_panel(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    ui.allocate_ui(egui::vec2(300.0, 500.0), |ui| appearance::show(app, ui));
}

/// One frame of the boxed panel with `key` pressed.
fn key_frame(ctx: &egui::Context, app: &mut VectorcraftApp, key: egui::Key) {
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame_events(ctx, app, vec![ev], boxed_panel);
}

/// After a click on a row, Delete or Backspace removes the highlighted rows, as in the Layers panel.
/// A click outside the panel, or a focused text field, takes the key away.
#[test]
fn delete_and_backspace_remove_the_highlighted_rows_after_a_click_in_the_panel() {
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
    let at = |name: &str| text_rect(&texts, name).right_center() + egui::vec2(40.0, 0.0);
    let (glow, stroke) = (at("Outer Glow"), at("Stroke:"));
    click(&ctx, &mut app, glow, boxed_panel);
    key_frame(&ctx, &mut app, egui::Key::Delete);
    assert_eq!(object_effects(&app), ["stylize.dropShadow", "distort.twist"]);
    // Backspace too (rows moved up: Drop Shadow is where Outer Glow was).
    click(&ctx, &mut app, glow, boxed_panel);
    key_frame(&ctx, &mut app, egui::Key::Backspace);
    assert_eq!(object_effects(&app), ["distort.twist"]);
    // A press outside the panel: the key no longer reaches the rows.
    click(&ctx, &mut app, stroke, boxed_panel);
    click(&ctx, &mut app, egui::pos2(900.0, 900.0), boxed_panel);
    key_frame(&ctx, &mut app, egui::Key::Delete);
    assert_eq!(stack_items(&app).len(), 2);
    // A focused text field keeps the key.
    click(&ctx, &mut app, stroke, boxed_panel);
    ctx.memory_mut(|m| m.request_focus(egui::Id::unique("some-field")));
    key_frame(&ctx, &mut app, egui::Key::Delete);
    assert_eq!(stack_items(&app).len(), 2);
}

/// A selected group listed top-down `S3, FA, Contents, S1, F0` (fills red F0 and green FA, strokes
/// 1 and 3 pt; paint order F0 S1 FA S3).
fn straddling_group() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    let b = run(&mut app, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "object.group", json!({}));
    run(&mut app, "appearance.addFill", json!({}));
    run(&mut app, "paint.setFill", json!({"color": "#ff0000", "item": 0}));
    run(&mut app, "appearance.addStroke", json!({}));
    run(&mut app, "stroke.set", json!({"item": 1, "weight": 1}));
    run(&mut app, "appearance.addFill", json!({}));
    run(&mut app, "paint.setFill", json!({"color": "#00ff00", "item": 2}));
    run(&mut app, "appearance.addStroke", json!({}));
    run(&mut app, "stroke.set", json!({"item": 3, "weight": 3}));
    run(&mut app, "appearance.moveItem", json!({"from": "contents", "to": 2}));
    app
}

/// The group's items, paint order (F0/FA by colour, S<weight>), and how many paint below Contents.
fn group_stack(app: &VectorcraftApp) -> (Vec<&'static str>, usize) {
    let ap = first_selected(app).unwrap().appearance.clone();
    let red = vectorcraft_color::Paint::solid(vectorcraft_color::Color::from_hex("#ff0000").unwrap());
    let names = ap
        .items
        .iter()
        .map(|it| match it {
            vectorcraft_doc::AppearanceItem::Stroke(st) if st.width == 1.0 => "S1",
            vectorcraft_doc::AppearanceItem::Stroke(_) => "S3",
            it if *it.paint() == red => "F0",
            _ => "FA",
        })
        .collect();
    (names, ap.contents_at())
}

/// The owner's example: FA and S1 highlighted, FA dragged onto S3, lands as one block above it:
/// listed FA, S1, S3, Contents, F0. The moved rows stay highlighted: Delete then removes both.
#[test]
fn highlighted_rows_drag_as_one_block_and_stay_highlighted() {
    let mut app = straddling_group();
    assert_eq!(group_stack(&app), (vec!["F0", "S1", "FA", "S3"], 2));
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let (fa, s1) = (row_point(&ctx, &mut app, "Fill:", 0), row_point(&ctx, &mut app, "Stroke:", 1));
    click_mod(&ctx, &mut app, fa, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, s1, egui::Modifiers::COMMAND);
    let s3 = row_point(&ctx, &mut app, "Stroke:", 0);
    drag_in_panel(&ctx, &mut app, fa, s3 - egui::vec2(0.0, 5.0));
    assert_eq!(group_stack(&app), (vec!["F0", "S3", "S1", "FA"], 1));
    click(&ctx, &mut app, delete, appearance::show);
    // Listed S3, Contents, F0.
    assert_eq!(group_stack(&app), (vec!["F0", "S3"], 1));
}

/// Alt-dragging two highlighted effects onto the stroke copies both into its list; the copies are
/// then the highlighted rows.
#[test]
fn alt_dragging_highlighted_effects_copies_them_together() {
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let (glow, twist) = (row_point(&ctx, &mut app, "Outer Glow", 0), row_point(&ctx, &mut app, "Twist", 0));
    click_mod(&ctx, &mut app, glow, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, twist, egui::Modifiers::COMMAND);
    let stroke = row_point(&ctx, &mut app, "Stroke:", 0);
    let alt = egui::Modifiers::ALT;
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: alt };
    for e in [
        vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(glow)],
        vec![button(glow, true)],
        vec![egui::Event::PointerMoved(glow + egui::vec2(0.0, 6.0))],
        vec![egui::Event::PointerMoved(stroke)],
        vec![button(stroke, false)],
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    ] {
        frame_events(&ctx, &mut app, e, appearance::show);
    }
    let ids: Vec<_> = current_stroke_effects(&app).iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["stylize.outerGlow", "distort.twist"]);
    assert_eq!(object_effects(&app).len(), 3, "the originals stay");
}

/// While highlighted rows are dragged, a dimmed copy of the first highlighted row follows the
/// pointer (as in the Layers panel, whichever row was grabbed) and the cursor is the grabbing hand.
#[test]
fn dragged_rows_show_a_dimmed_copy_of_the_first_highlighted_row() {
    let mut app = straddling_group();
    let ctx = drop_ctx();
    let (fa, s1) = (row_point(&ctx, &mut app, "Fill:", 0), row_point(&ctx, &mut app, "Stroke:", 1));
    click_mod(&ctx, &mut app, fa, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, s1, egui::Modifiers::COMMAND);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [vec![egui::Event::PointerMoved(s1)], vec![button(s1, true)], vec![egui::Event::PointerMoved(s1 + egui::vec2(0.0, 6.0))]] {
        frame_events(&ctx, &mut app, e, appearance::show);
    }
    let held = egui::pos2(s1.x, 2000.0);
    // The copy shows from the frame after the drag starts.
    frame_events(&ctx, &mut app, vec![egui::Event::PointerMoved(held - egui::vec2(0.0, 10.0))], appearance::show);
    let mut out =
        ctx.run_ui(egui::RawInput { events: vec![egui::Event::PointerMoved(held)], ..Default::default() }, |ui| appearance::show(&mut app, ui));
    out.textures_delta.clear();
    assert_eq!(out.platform_output.cursor_icon, egui::CursorIcon::Grabbing);
    let mut near = vec![];
    for c in &out.shapes {
        if let egui::Shape::Text(t) = &c.shape
            && (t.pos.y - held.y).abs() < 40.0
        {
            near.push(t.galley.text().to_string());
        }
    }
    assert!(near.contains(&"Fill:".to_string()), "the copy shows the first highlighted row: {near:?}");
    assert!(!near.contains(&"Stroke:".to_string()), "not the grabbed one: {near:?}");
}

/// One frame at time `t` (seconds) with `events`: the texts drawn.
fn timed_frame(ctx: &egui::Context, app: &mut VectorcraftApp, t: f64, events: Vec<egui::Event>) -> Vec<String> {
    frame_raw(ctx, app, egui::RawInput { events, time: Some(t), ..Default::default() }, appearance::show).into_iter().map(|(s, _)| s).collect()
}

/// The tooltip shown after resting the pointer on `pos`.
fn tooltip_at(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2) -> Vec<String> {
    let t = ctx.input(|i| i.time) + 1.0;
    timed_frame(ctx, app, t, vec![egui::Event::PointerMoved(pos)]);
    timed_frame(ctx, app, t + 2.0, vec![]);
    timed_frame(ctx, app, t + 2.1, vec![])
}

/// Double-click at `pos` (two quick clicks).
fn double_click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let t = ctx.input(|i| i.time) + 1.0;
    for (i, e) in [egui::Event::PointerMoved(pos), button(true), button(false), button(true), button(false)].into_iter().enumerate() {
        timed_frame(ctx, app, t + i as f64 * 0.05, vec![e]);
    }
}

/// The text of an effect row (its name) opens the effect's settings dialog on a click, with or
/// without Ctrl/Cmd or Shift held, and says "Click to edit"; the empty space right of it takes the
/// click rules, opens the dialog on a double-click and says "Double-click to edit".
#[test]
fn an_effects_name_edits_it_and_its_empty_space_highlights_it() {
    for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
        let mut app = rect_with_three_effects();
        let ctx = drop_ctx();
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        click_mod(&ctx, &mut app, text_rect(&texts, "Outer Glow").center(), m);
        assert!(app.ui.dialog.is_some(), "{m:?}: a click on the name opens the dialog");
    }
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    // Over the name only its own tooltip shows, not the row's.
    let tip = tooltip_at(&ctx, &mut app, text_rect(&texts, "Outer Glow").center());
    assert!(tip.contains(&"Click to edit".to_string()) && !tip.contains(&"Double-click to edit".to_string()), "{tip:?}");
    let space = row_point(&ctx, &mut app, "Outer Glow", 0);
    assert!(tooltip_at(&ctx, &mut app, space).contains(&"Double-click to edit".to_string()));
    // A single Ctrl-click on the empty space opens nothing; a double-click there does.
    click_mod(&ctx, &mut app, space, egui::Modifiers::COMMAND);
    assert!(app.ui.dialog.is_none());
    double_click(&ctx, &mut app, space);
    assert!(app.ui.dialog.is_some(), "a double-click on the empty space opens the dialog");
}

/// A fill or stroke row's label ("Fill:", "Stroke:") is a link: a click opens the Color or Stroke
/// panel with or without modifiers, with the tooltip "Click to edit". Its empty space only takes the
/// click rules: no tooltip, and neither a Ctrl-click nor a double-click there opens a panel.
#[test]
fn fill_and_stroke_labels_are_links_and_their_empty_space_only_highlights() {
    for (label, panel) in [("Stroke:", "stroke"), ("Fill:", "color")] {
        for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
            let mut app = app_with_rect();
            let ctx = drop_ctx();
            let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
            click_mod(&ctx, &mut app, text_rect(&texts, label).center(), m);
            assert_eq!(app.ui.transient_panel.as_deref(), Some(panel), "{label} {m:?}");
        }
        let mut app = app_with_rect();
        let ctx = drop_ctx();
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        let tip = tooltip_at(&ctx, &mut app, text_rect(&texts, label).center());
        assert!(tip.contains(&"Click to edit".to_string()) && !tip.contains(&"Double-click to edit".to_string()), "{label}: {tip:?}");
        let space = row_point(&ctx, &mut app, label, 0);
        let tip = tooltip_at(&ctx, &mut app, space);
        assert!(!tip.iter().any(|t| t.ends_with("to edit")), "{label}: no tooltip on the empty space: {tip:?}");
        click_mod(&ctx, &mut app, space, egui::Modifiers::COMMAND);
        double_click(&ctx, &mut app, space);
        assert_eq!(app.ui.transient_panel, None, "{label}");
    }
}

/// One app-like frame: the global shortcuts first, then the panel `f` in a 300×500 area (as the app
/// runs `shortcuts::handle` in `logic` before the panels draw).
fn app_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, f: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        ui.allocate_ui(egui::vec2(300.0, 500.0), |ui| f(app, ui));
    });
    out.textures_delta.clear();
}

/// A click at `pos` in app-like frames of `f` ([`app_frame`]): hover, press, release, then a frame with the pointer still there.
fn app_click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2, f: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [egui::Event::PointerMoved(pos), button(true), button(false), egui::Event::PointerMoved(pos)] {
        app_frame(ctx, app, vec![e], f);
    }
}

/// `key` pressed and released in an app-like frame of `f`, then one more frame.
fn app_key(ctx: &egui::Context, app: &mut VectorcraftApp, key: egui::Key, f: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let ev = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
    app_frame(ctx, app, vec![ev(true), ev(false)], f);
    app_frame(ctx, app, vec![], f);
}

/// With the app's shortcuts running first: after a click on an Appearance row, Delete and Backspace
/// remove the highlighted rows and leave the object; after a click outside the panel they delete the
/// selected object again (Edit › Clear).
#[test]
fn after_a_row_click_delete_reaches_the_appearance_panel_not_edit_clear() {
    let ctx = drop_ctx();
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = rect_with_three_effects();
        let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
        let glow = text_rect(&texts, "Outer Glow").right_center() + egui::vec2(40.0, 0.0);
        app_click(&ctx, &mut app, glow, boxed_panel);
        app_key(&ctx, &mut app, key, boxed_panel);
        assert!(first_selected(&app).is_some(), "{key:?}: the object stays");
        assert_eq!(object_effects(&app), ["stylize.dropShadow", "distort.twist"], "{key:?}");
        app_click(&ctx, &mut app, egui::pos2(900.0, 900.0), boxed_panel);
        app_key(&ctx, &mut app, key, boxed_panel);
        assert!(app.session.active().unwrap().doc.layers[0].children().unwrap().is_empty(), "{key:?}: Edit › Clear deletes the object");
    }
}

/// The same after a click on a row's name, which also edits the row: "Stroke:" opens the Stroke
/// panel, the name of an effect without settings its inline editor (one with settings opens a
/// dialog, which keeps the keys from the app's shortcuts anyway). Once the Stroke panel's popup is
/// put away (its field has the keyboard while it is open), Delete removes that row and leaves the
/// object.
#[test]
fn after_a_name_click_delete_reaches_the_appearance_panel_not_edit_clear() {
    for (name, key) in [("Stroke:", egui::Key::Delete), ("Outline Object", egui::Key::Backspace)] {
        // A context of its own: the panel keeps the keys from one click to the next.
        let ctx = drop_ctx();
        let mut app = rect_with_three_effects();
        run(&mut app, "effect.apply", json!({"effect": "path.outlineObject", "item": null}));
        let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
        let at = text_rect(&texts, name).center();
        app_click(&ctx, &mut app, at, boxed_panel);
        app_key(&ctx, &mut app, egui::Key::Escape, boxed_panel);
        app_key(&ctx, &mut app, key, boxed_panel);
        assert!(first_selected(&app).is_some(), "{name}: the object stays");
        if name == "Stroke:" {
            assert_eq!(stack_items(&app), [(true, None)], "{name}: the stroke goes");
        } else {
            assert_eq!(object_effects(&app), ["stylize.outerGlow", "stylize.dropShadow", "distort.twist"], "{name}: the effect goes");
        }
    }
}

/// After a click on a group's Contents row or a type object's Characters link, Delete stays with
/// the panel: the row can't be removed, so nothing is, and Edit › Clear doesn't delete the object.
#[test]
fn after_a_contents_click_delete_removes_nothing() {
    let group = || {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
        let b = run(&mut app, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}))["id"].clone();
        run(&mut app, "select.set", json!({"ids": [a, b]}));
        run(&mut app, "object.group", json!({}));
        run(&mut app, "appearance.addFill", json!({}));
        app
    };
    let type_object = || {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let t = run(&mut app, "text.create", json!({"x": 10, "y": 50, "text": "Hi"}))["id"].clone();
        run(&mut app, "select.set", json!({"ids": [t]}));
        app
    };
    for (mut app, label) in [(group(), "Contents"), (type_object(), "Characters")] {
        let ctx = drop_ctx();
        let before = first_selected(&app).unwrap().clone();
        let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
        // The Contents row's empty space; the Characters name, a link.
        let r = text_rect(&texts, label);
        let at = if label == "Contents" { r.right_center() + egui::vec2(40.0, 0.0) } else { r.center() };
        app_click(&ctx, &mut app, at, boxed_panel);
        app_key(&ctx, &mut app, egui::Key::Delete, boxed_panel);
        let after = first_selected(&app).unwrap_or_else(|| panic!("{label}: the object stays"));
        assert_eq!(after.appearance, before.appearance, "{label}: nothing is removed");
    }
}

/// After a Shift-click on a fill/stroke row's swatch (which opens the Color panel on it) and its
/// popup put away, Delete removes that row, not the object.
#[test]
fn after_a_shift_click_on_a_swatch_delete_removes_its_row() {
    let ctx = drop_ctx();
    let mut app = app_with_rect();
    let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
    // The swatch: 61 pt right of where the row's name starts.
    let r = text_rect(&texts, "Stroke:");
    let at = egui::pos2(r.left() + 61.0, r.center().y);
    let shift = egui::Modifiers::SHIFT;
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: shift };
    for e in [egui::Event::PointerMoved(at), button(true), button(false)] {
        app_frame(&ctx, &mut app, vec![egui::Event::ModifiersChanged(shift), e], boxed_panel);
    }
    app_frame(&ctx, &mut app, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)], boxed_panel);
    assert_eq!(app.session.appearance_item(), Some(1), "the Shift-click selects the stroke row");
    app_key(&ctx, &mut app, egui::Key::Escape, boxed_panel);
    app_key(&ctx, &mut app, egui::Key::Delete, boxed_panel);
    assert!(first_selected(&app).is_some(), "the object stays");
    assert_eq!(stack_items(&app), [(true, None)], "the stroke goes");
}

/// The Layers panel in a 300×500 pt area at the top left, so clicks can land outside it.
fn boxed_layers(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    ui.allocate_ui(egui::vec2(300.0, 500.0), |ui| super::layers::show(app, ui));
}

/// The same in the Layers panel: after a click on a layer's row, Delete deletes that layer, not the
/// art selected on another layer.
#[test]
fn after_a_row_click_delete_reaches_the_layers_panel_not_edit_clear() {
    let ctx = drop_ctx();
    let mut app = app_with_rect();
    run(&mut app, "layer.new", json!({}));
    let rect = first_selected(&app).unwrap().id;
    let texts = frame_events(&ctx, &mut app, vec![], boxed_layers);
    let row = text_rect(&texts, "Layer 2").center();
    app_click(&ctx, &mut app, row, boxed_layers);
    app_key(&ctx, &mut app, egui::Key::Delete, boxed_layers);
    let d = &app.session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 1, "Layer 2 is deleted");
    assert!(d.node(rect).is_some(), "the rectangle on Layer 1 stays");
}

/// "Opacity:" is a link to the Transparency panel ("Click to edit"), whatever modifiers are held: on
/// a stroke's Opacity row it makes that stroke the active row first; on the object's own Opacity row
/// no fill/stroke is active.
#[test]
fn the_opacity_label_opens_the_transparency_panel_on_its_row() {
    for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
        let mut app = app_with_rect();
        let ctx = drop_ctx();
        super::set_pstate(&ctx, "ap-open-1", true);
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        let item = texts.iter().find(|(t, _)| t == "Opacity:").expect("the stroke's Opacity row").1;
        click_mod(&ctx, &mut app, item.center(), m);
        assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (Some("transparency"), Some(1)), "{m:?}");
        app.ui.transient_panel = None;
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        let object = texts.iter().rev().find(|(t, _)| t == "Opacity:").expect("the object's Opacity row").1;
        click_mod(&ctx, &mut app, object.center(), m);
        assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (Some("transparency"), None), "{m:?}");
    }
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    let tip = tooltip_at(&ctx, &mut app, text_rect(&texts, "Opacity:").center());
    assert!(tip.contains(&"Click to edit".to_string()), "{tip:?}");
}

/// Whether each control named in `labels` is on in a frame of `f` (by its accessibility node).
fn controls_on(app: &mut VectorcraftApp, labels: &[&str], f: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<bool> {
    let ctx = drop_ctx();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(app, ui));
    out.textures_delta.clear();
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    labels
        .iter()
        // (Menu items carry a tick or its space before the label.)
        .map(|l| {
            let named = |n: &&(egui::accesskit::NodeId, egui::accesskit::Node)| n.1.label().is_some_and(|t| t.trim_start_matches('✓').trim() == *l);
            !nodes.iter().find(named).unwrap_or_else(|| panic!("no control named {l:?}")).1.is_disabled()
        })
        .collect()
}

/// While the Appearance panel's active row is a fill/stroke, the Transparency panel shows that item's
/// opacity and blend and greys out what only applies to whole objects: the opacity mask, Isolate
/// Blending, Knockout Group, Opacity & Mask Define Knockout Shape, and the menu's mask items.
#[test]
fn the_transparency_panel_greys_out_object_controls_for_a_fill_or_stroke() {
    let object = ["Make Mask", "Isolate Blending", "Knockout Group", "Opacity & Mask Define Knockout Shape"];
    let mut app = app_with_rect();
    assert_eq!(controls_on(&mut app, &object, transparency::show), [true; 4], "the object: all on");
    assert_eq!(controls_on(&mut app, &["Make Opacity Mask"], transparency::menu), [true]);
    run(&mut app, "appearance.setActiveItem", json!({"index": 1}));
    assert_eq!(controls_on(&mut app, &object, transparency::show), [false; 4], "a stroke: all off");
    assert_eq!(controls_on(&mut app, &["Make Opacity Mask"], transparency::menu), [false]);
    run(&mut app, "appearance.setActiveItem", json!({"index": null}));
    assert_eq!(controls_on(&mut app, &object, transparency::show), [true; 4], "back to the object");
}

/// The texts drawn on the highlighted rows of a frame of the panel (rows filled with the
/// `row_selected` colour), top-down.
fn lit_texts(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<String> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| appearance::show(app, ui));
    out.textures_delta.clear();
    let lit = crate::theme::Tokens::get(ctx).row_selected;
    let (mut rows, mut texts) = (vec![], vec![]);
    /// The highlighted rows' rects and every text with its rect in shape `s`.
    fn collect(s: &egui::Shape, lit: egui::Color32, rows: &mut Vec<egui::Rect>, texts: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Rect(r) if r.fill == lit => rows.push(r.rect),
            egui::Shape::Text(t) => texts.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, lit, rows, texts)),
            _ => {}
        }
    }
    for c in &out.shapes {
        collect(&c.shape, lit, &mut rows, &mut texts);
    }
    // (A swatch's out-of-gamut sign and a row's popup aside.)
    let popup = ctx.memory(|m| m.area_rect(egui::Id::unique("ap-row-popup")));
    let shown = |r: &egui::Rect| rows.iter().any(|row| row.contains(r.center())) && !popup.is_some_and(|p| p.contains(r.center()));
    texts.into_iter().filter(|(t, r)| t.trim_end_matches('\u{fe0f}') != "⚠" && shown(r)).map(|(t, _)| t).collect()
}

/// A fill/stroke's "Opacity:" highlights its own Opacity row, not the fill/stroke, whatever modifiers
/// are held; the Transparency panel still edits that fill/stroke. The object's Opacity row
/// highlights itself too, by its link or its empty space (which opens nothing).
#[test]
fn an_opacity_row_highlights_itself() {
    for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
        let mut app = app_with_rect();
        let ctx = drop_ctx();
        super::set_pstate(&ctx, "ap-open-1", true);
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        click_mod(&ctx, &mut app, text_rect(&texts, "Opacity:").center(), m);
        assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (Some("transparency"), Some(1)), "{m:?}");
        assert_eq!(lit_texts(&ctx, &mut app), ["Opacity:", "Default"], "{m:?}: the stroke's Opacity row only");
        app.ui.transient_panel = None;
        let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
        let object = texts.iter().rev().find(|(t, _)| t == "Opacity:").unwrap().1;
        click_mod(&ctx, &mut app, object.center(), m);
        assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (Some("transparency"), None), "{m:?}");
        assert_eq!(lit_texts(&ctx, &mut app), ["Opacity:", "Default"], "{m:?}: the object's Opacity row");
    }
    // The empty space: highlights, opens nothing.
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    super::set_pstate(&ctx, "ap-open-1", true);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    let stroke_opacity = text_rect(&texts, "Default").right_center() + egui::vec2(60.0, 0.0);
    click(&ctx, &mut app, stroke_opacity, appearance::show);
    assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (None, Some(1)));
    assert_eq!(lit_texts(&ctx, &mut app), ["Opacity:", "Default"]);
    let texts = frame_events(&ctx, &mut app, vec![], appearance::show);
    let object = texts.iter().rev().find(|(t, _)| t == "Default").unwrap().1.right_center() + egui::vec2(60.0, 0.0);
    click(&ctx, &mut app, object, appearance::show);
    assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (None, None));
    assert_eq!(lit_texts(&ctx, &mut app), ["Opacity:", "Default"]);
}

/// An Opacity row is no item: while one is highlighted Duplicate and Delete are greyed out and the
/// Delete key removes nothing (nor the object); dragging it changes nothing.
#[test]
fn an_opacity_row_cannot_be_deleted_duplicated_or_dragged() {
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    super::set_pstate(&ctx, "ap-open-1", true);
    let before = stack_items(&app);
    let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
    let space = text_rect(&texts, "Default").right_center() + egui::vec2(60.0, 0.0);
    app_click(&ctx, &mut app, space, boxed_panel);
    assert_eq!(lit_texts(&ctx, &mut app), ["Opacity:", "Default"]);
    for button in ["Duplicate Selected Item", "Delete Selected Item"] {
        let at = panel_button_in(&ctx, &mut app, button, boxed_panel);
        app_click(&ctx, &mut app, at, boxed_panel);
        assert_eq!(stack_items(&app), before, "{button} is greyed out");
        app_click(&ctx, &mut app, space, boxed_panel);
    }
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        app_key(&ctx, &mut app, key, boxed_panel);
        assert!(first_selected(&app).is_some(), "{key:?}: the object stays");
        assert_eq!(stack_items(&app), before, "{key:?}");
    }
    let fill = row_point(&ctx, &mut app, "Fill:", 0);
    drag_in_panel(&ctx, &mut app, space, fill + egui::vec2(0.0, 10.0));
    assert_eq!(stack_items(&app), before, "dragged: nothing moves");
}

/// A context with the app's fonts (the flyout draws icons from them).
fn flyout_ctx() -> egui::Context {
    let ctx = drop_ctx();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// The frame of the Appearance panel, 300 points wide at the window's left edge, with a row's popup
/// beside it, as the app draws them.
fn panel_and_flyout(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    ui.allocate_ui(egui::vec2(300.0, 500.0), |ui| appearance::show(app, ui));
}

/// A panel a row's link opens closes on a press outside it (in the Appearance panel, on Delete
/// Selected Item…), with Auto-Collapse Iconic Panels off; its first text field takes the keyboard.
#[test]
fn panels_a_row_opens_close_when_they_lose_focus() {
    let links = [("Stroke:", 0, "stroke"), ("Fill:", 0, "color"), ("Opacity:", 0, "transparency"), ("Opacity:", 1, "transparency")];
    for (label, nth, panel) in links {
        let mut app = app_with_rect();
        let ctx = flyout_ctx();
        super::set_pstate(&ctx, "ap-open-1", true);
        let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
        let link = texts.iter().filter(|(t, _)| t == label).nth(nth).unwrap().1.center();
        app_click(&ctx, &mut app, link, panel_and_flyout);
        for _ in 0..3 {
            app_frame(&ctx, &mut app, vec![], panel_and_flyout);
        }
        assert_eq!(app.ui.transient_panel.as_deref(), Some(panel), "{label}");
        assert!(ctx.memory(|m| m.focused()).is_some(), "{label}: a field of the {panel} panel has the keyboard");
        let object_row = egui::pos2(150.0, 15.0);
        app_click(&ctx, &mut app, object_row, panel_and_flyout);
        assert_eq!(app.ui.transient_panel, None, "{label}: a press in the Appearance panel closes it");
    }
    // Delete Selected Item closes the Stroke panel as it removes the stroke.
    let mut app = app_with_rect();
    let ctx = flyout_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
    app_click(&ctx, &mut app, text_rect(&texts, "Stroke:").center(), panel_and_flyout);
    let delete = panel_button_in(&ctx, &mut app, "Delete Selected Item", panel_and_flyout);
    app_click(&ctx, &mut app, delete, panel_and_flyout);
    assert_eq!((app.ui.transient_panel.as_deref(), stack_items(&app).len()), (None, 1));
}

/// A row's popup opens beside the Appearance panel, level with the row whose link opened it: on
/// the right here, where the panel leaves no room on its left. Escape puts it away.
#[test]
fn a_row_popup_opens_beside_the_panel_at_its_row() {
    let mut app = app_with_rect();
    let ctx = flyout_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
    let link = text_rect(&texts, "Stroke:");
    app_click(&ctx, &mut app, link.center(), panel_and_flyout);
    for _ in 0..3 {
        app_frame(&ctx, &mut app, vec![], panel_and_flyout);
    }
    let popup = ctx.memory(|m| m.area_rect(egui::Id::unique("ap-row-popup"))).expect("the popup shows");
    assert!(popup.left() >= 300.0, "right of the panel: {popup:?}");
    assert!((popup.top() - link.center().y).abs() < 30.0, "level with its row: {popup:?} {link:?}");
    app_frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        panel_and_flyout,
    );
    assert_eq!(app.ui.transient_panel, None, "Escape puts it away");
}

/// A link to a panel that shows already (floating in front of its window) opens no popup: the
/// panel is in sight as it is.
#[test]
fn a_row_link_to_a_panel_in_sight_opens_no_popup() {
    let mut app = app_with_rect();
    app.run("window.panel.float", json!({"panel": "stroke"})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "stroke"));
    let ctx = flyout_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
    app_click(&ctx, &mut app, text_rect(&texts, "Stroke:").center(), panel_and_flyout);
    app_frame(&ctx, &mut app, vec![], panel_and_flyout);
    assert_eq!(app.ui.transient_panel, None);
}

/// On a type object "Characters" is a link ("Click to edit") to the Character panel, which closes
/// on a press outside it.
#[test]
fn characters_opens_the_character_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let t = run(&mut app, "text.create", json!({"x": 0, "y": 60, "text": "Hi"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [t]}));
    let ctx = flyout_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
    let characters = text_rect(&texts, "Characters").center();
    let tip = tooltip_at(&ctx, &mut app, characters);
    assert!(tip.contains(&"Click to edit".to_string()), "{tip:?}");
    app_click(&ctx, &mut app, characters, panel_and_flyout);
    assert_eq!(app.ui.transient_panel.as_deref(), Some("character"));
    app_click(&ctx, &mut app, egui::pos2(150.0, 15.0), panel_and_flyout);
    assert_eq!(app.ui.transient_panel, None);
}

/// The point of the control named `label` (by its accessibility node) in an [`app_frame`] of `f`
/// (the same frame, so popups `f` opened in one stay open).
fn control_at(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, f: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Option<egui::Pos2> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        ui.allocate_ui(egui::vec2(300.0, 500.0), |ui| f(app, ui));
    });
    out.textures_delta.clear();
    let nodes = &out.platform_output.accesskit_update.as_ref()?.nodes;
    let b = nodes.iter().find(|(_, n)| n.label() == Some(label))?.1.bounds()?;
    Some(egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5))
}

/// With nothing selected, the fill and stroke swatches work as an object's do: a click opens the
/// swatches, whose pick paints the next object drawn; Shift-click opens the Color panel on that
/// fill or stroke. "Fill:" and "Stroke:" make theirs the Color panel's paint too.
#[test]
fn with_nothing_selected_the_swatches_set_up_new_art() {
    let swatch = |app: &VectorcraftApp| app.session.active().unwrap().doc.swatches.iter().find(|s| !s.paint.is_none()).unwrap().clone();
    for (label, fill) in [("Fill:", true), ("Stroke:", false)] {
        let mut app = app_with_rect();
        run(&mut app, "select.set", json!({"ids": []}));
        let pick = swatch(&app);
        let ctx = flyout_ctx();
        let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
        // The swatch sits 52 points right of where the label starts, 18 wide.
        let chip = text_rect(&texts, label).left_center() + egui::vec2(61.0, 0.0);
        app_click(&ctx, &mut app, chip, panel_and_flyout);
        let tile = control_at(&ctx, &mut app, &pick.name, panel_and_flyout).unwrap_or_else(|| panic!("{label}: the swatches popup"));
        app_click(&ctx, &mut app, tile, panel_and_flyout);
        let ap = app.session.new_art();
        let item = ap.items.iter().rev().find(|it| it.is_fill() == fill).unwrap();
        assert_eq!(item.paint(), &pick.paint, "{label}: the pick paints new art");
        click_mod(&ctx, &mut app, chip, egui::Modifiers::SHIFT);
        assert_eq!((app.ui.transient_panel.as_deref(), app.session.fill_active), (Some("color"), fill), "{label}: Shift-click");
        app.ui.transient_panel = None;
        run(&mut app, "paint.toggleActive", json!({"fill": !fill}));
        let texts = frame_events(&ctx, &mut app, vec![], panel_and_flyout);
        app_click(&ctx, &mut app, text_rect(&texts, label).center(), panel_and_flyout);
        assert_eq!(
            (app.ui.transient_panel.as_deref(), app.session.fill_active),
            (Some(if fill { "color" } else { "stroke" }), fill),
            "{label}: its link"
        );
    }
}

/// The stroke row's weight is the same field with or without a selection: typed in, it sets the
/// selected stroke's weight, or with nothing selected the next object drawn's.
#[test]
fn the_stroke_weight_field_edits_with_or_without_a_selection() {
    for selected in [true, false] {
        let mut app = app_with_rect();
        if !selected {
            run(&mut app, "select.set", json!({"ids": []}));
        }
        let ctx = drop_ctx();
        let one = Unit::Points.format(1.0);
        let texts = frame_events(&ctx, &mut app, vec![], boxed_panel);
        app_click(&ctx, &mut app, text_rect(&texts, &one).center(), boxed_panel);
        let enter =
            |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        app_frame(&ctx, &mut app, vec![egui::Event::Text("6".into())], boxed_panel);
        app_frame(&ctx, &mut app, vec![enter(true), enter(false)], boxed_panel);
        app_frame(&ctx, &mut app, vec![], boxed_panel);
        let width =
            if selected { first_selected(&app).unwrap().appearance.stroke().unwrap().width } else { app.session.new_art().stroke().unwrap().width };
        assert_eq!(width, 6.0, "selected: {selected}");
    }
}

/// The stroke row's weight is the Stroke panel's spinner, with or without a selection: its up arrow
/// steps the weight by 1 pt and its dropdown sets a preset weight.
#[test]
fn the_stroke_weight_has_the_stroke_panels_arrows_and_presets() {
    let preset = super::stroke::weight_presets(Unit::Points)[10];
    for selected in [true, false] {
        let mut app = app_with_rect();
        if !selected {
            run(&mut app, "select.set", json!({"ids": []}));
        }
        let width = |app: &VectorcraftApp| {
            if selected { first_selected(app).unwrap().appearance.stroke().unwrap().width } else { app.session.new_art().stroke().unwrap().width }
        };
        let ctx = drop_ctx();
        let arrows = control_at(&ctx, &mut app, "Step up or down", boxed_panel).unwrap_or_else(|| panic!("selected: {selected}: the arrows"));
        app_click(&ctx, &mut app, arrows - egui::vec2(0.0, 6.0), boxed_panel);
        assert_eq!(width(&app), 2.0, "selected: {selected}: the up arrow");
        let presets = control_at(&ctx, &mut app, "Presets", boxed_panel).unwrap();
        app_click(&ctx, &mut app, presets, boxed_panel);
        let pick =
            control_at(&ctx, &mut app, &Unit::Points.format(preset), boxed_panel).unwrap_or_else(|| panic!("selected: {selected}: the presets"));
        app_click(&ctx, &mut app, pick, boxed_panel);
        assert_eq!(width(&app), preset, "selected: {selected}: a preset");
    }
}

/// Stroke weights step by the ladder (whole numbers from 1, quarters below): the Appearance
/// panel's arrows go from 0.3 pt up to 0.5 pt and back down to 0.25 pt; ↓ in the Stroke panel's
/// focused Weight field goes from 1 pt to 0.75 pt.
#[test]
fn stroke_weights_step_by_quarters_below_one() {
    let mut app = app_with_rect();
    run(&mut app, "stroke.set", json!({"weight": 0.3}));
    let weight = |app: &VectorcraftApp| first_selected(app).unwrap().appearance.stroke().unwrap().width;
    let ctx = drop_ctx();
    let arrows = control_at(&ctx, &mut app, "Step up or down", boxed_panel).unwrap();
    app_click(&ctx, &mut app, arrows - egui::vec2(0.0, 6.0), boxed_panel);
    assert!((weight(&app) - 0.5).abs() < 1e-9, "up: {}", weight(&app));
    app_click(&ctx, &mut app, arrows + egui::vec2(0.0, 6.0), boxed_panel);
    assert!((weight(&app) - 0.25).abs() < 1e-9, "down: {}", weight(&app));
    // The Stroke panel's field, focused, with ↓.
    run(&mut app, "stroke.set", json!({"weight": 1}));
    /// The Stroke panel in a 300×500 pt area at the top left.
    fn stroke_panel(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
        ui.allocate_ui(egui::vec2(300.0, 500.0), |ui| super::stroke::show(app, ui));
    }
    let ctx = drop_ctx();
    let texts = frame_events(&ctx, &mut app, vec![], stroke_panel);
    app_click(&ctx, &mut app, text_rect(&texts, &Unit::Points.format(1.0)).center(), stroke_panel);
    app_key(&ctx, &mut app, egui::Key::ArrowDown, stroke_panel);
    assert!((weight(&app) - 0.75).abs() < 1e-9, "↓: {}", weight(&app));
}

/// Dragging a highlighted effect onto Delete removes every highlighted effect, the active one too:
/// nothing is left active then, so the effect that moved up into the active one's place is not
/// taken for it (Delete Selected Item then removes nothing).
#[test]
fn a_drop_on_delete_that_removes_the_active_effect_leaves_no_effect_active() {
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let (glow, shadow) = (row_point(&ctx, &mut app, "Outer Glow", 0), row_point(&ctx, &mut app, "Drop Shadow", 0));
    // Drop Shadow, then Outer Glow with Ctrl: both highlighted, Outer Glow active.
    click_mod(&ctx, &mut app, shadow, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, glow, egui::Modifiers::COMMAND);
    drag_in_panel(&ctx, &mut app, shadow, delete);
    assert_eq!(object_effects(&app), ["distort.twist"]);
    assert!(lit_texts(&ctx, &mut app).is_empty(), "nothing highlighted");
    // (Two rows shorter, the panel has its button higher up now.)
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    click(&ctx, &mut app, delete, appearance::show);
    assert_eq!(object_effects(&app), ["distort.twist"], "Twist was never highlighted");
}

/// Dragging an effect that isn't highlighted onto Delete removes only it: the highlighted effects
/// below it move up with their rows, so they stay highlighted and active, and Delete Selected Item
/// then removes them.
#[test]
fn a_drop_on_delete_above_the_active_effect_keeps_the_highlight_on_its_rows() {
    let mut app = rect_with_three_effects();
    let ctx = drop_ctx();
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    let (glow, shadow, twist) =
        (row_point(&ctx, &mut app, "Outer Glow", 0), row_point(&ctx, &mut app, "Drop Shadow", 0), row_point(&ctx, &mut app, "Twist", 0));
    // Drop Shadow, then Twist with Ctrl: both highlighted, Twist active.
    click_mod(&ctx, &mut app, shadow, egui::Modifiers::NONE);
    click_mod(&ctx, &mut app, twist, egui::Modifiers::COMMAND);
    drag_in_panel(&ctx, &mut app, glow, delete);
    assert_eq!(object_effects(&app), ["stylize.dropShadow", "distort.twist"]);
    let lit = lit_texts(&ctx, &mut app);
    assert!(["Drop Shadow", "Twist"].iter().all(|t| lit.iter().any(|l| l.contains(t))), "still highlighted: {lit:?}");
    let delete = panel_button(&ctx, &mut app, "Delete Selected Item");
    click(&ctx, &mut app, delete, appearance::show);
    assert!(object_effects(&app).is_empty(), "both highlighted effects removed: {:?}", object_effects(&app));
}

/// Whether Duplicate Selected Item and Delete Selected Item are enabled in a frame of the panel
/// with input `events`.
fn drop_buttons_on(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> [bool; 2] {
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| appearance::show(app, ui));
    out.textures_delta.clear();
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    ["Duplicate Selected Item", "Delete Selected Item"].map(|l| !nodes.iter().find(|(_, n)| n.label() == Some(l)).unwrap().1.is_disabled())
}

/// With no row selected, Duplicate and Delete Selected Item light up while a row is dragged (it can
/// be dropped on them) and go dark again once it is released.
#[test]
fn the_drop_buttons_light_up_while_a_row_is_dragged() {
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let mut app = app_with_rect();
    let ctx = drop_ctx();
    assert_eq!(drop_buttons_on(&ctx, &mut app, vec![]), [false, false], "nothing selected: dark");
    let stroke = row_point(&ctx, &mut app, "Stroke:", 0);
    drop_buttons_on(&ctx, &mut app, vec![egui::Event::PointerMoved(stroke)]);
    drop_buttons_on(&ctx, &mut app, vec![button(stroke, true)]);
    let held = stroke + egui::vec2(0.0, 40.0);
    drop_buttons_on(&ctx, &mut app, vec![egui::Event::PointerMoved(stroke + egui::vec2(0.0, 6.0))]);
    assert_eq!(drop_buttons_on(&ctx, &mut app, vec![egui::Event::PointerMoved(held)]), [true, true], "lit while the row is dragged");
    drop_buttons_on(&ctx, &mut app, vec![button(held, false)]);
    assert_eq!(drop_buttons_on(&ctx, &mut app, vec![]), [false, false], "dark again once it is released");
    assert_eq!(stack_items(&app).len(), 2, "released elsewhere: nothing removed");
}
