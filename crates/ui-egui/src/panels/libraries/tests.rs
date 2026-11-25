use std::cell::RefCell;

use egui::{Key, Modifiers, Pos2, pos2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::tests::panel_harness::Panel as Harness;
use crate::tests::picks::{answer_dialog, open_dialogs, poll, scripted};

/// A point inside the panel's list below every item, in the harness's room (300 x 1600).
const BELOW: Pos2 = pos2(150.0, 1400.0);

/// The name of the row that drags what [`CARRIED`] holds.
const SOURCE: &str = "Dragged";

thread_local! {
    /// What the test's drag source carries: art off the canvas or a swatch from another panel.
    static CARRIED: RefCell<Option<PanelDrag>> = const { RefCell::new(None) };
}

/// An app with a 200 x 200 document.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// The panel under a one-row list whose row drags what [`CARRIED`] holds when the drag starts, through the drag
/// session a real source starts. The row is always there, so the panel's rows stay where a test measured them.
fn with_source(app: &mut VectorcraftApp, ui: &mut Ui) {
    let rows = [SOURCE.to_string()];
    let _ = List::<String>::new("Source")
        .select(SelectMode::None)
        .natural_height(true)
        .drag_payload(|_: &ListDrag<String>| CARRIED.with(|carried| carried.borrow().clone()))
        .show_items(ui, &keyed(&rows, String::clone), &mut |key, _| ListRow::new(key.clone()));
    show(app, ui);
}

/// `drag` dragged from the source row and released at `at`.
fn drop_at(panel: &mut Harness, drag: PanelDrag, at: Pos2) {
    CARRIED.with(|carried| *carried.borrow_mut() = Some(drag));
    panel.frame();
    panel.drag_to(SOURCE, at, Modifiers::NONE);
    panel.frame();
}

/// A point just inside the top edge of the control named `name`.
fn top(panel: &Harness, name: &str) -> Pos2 {
    let r = panel.rect(name);
    pos2(r.center().x, r.top() + 2.0)
}

/// A point just inside the bottom edge of the control named `name`.
fn bottom(panel: &Harness, name: &str) -> Pos2 {
    let r = panel.rect(name);
    pos2(r.center().x, r.bottom() - 2.0)
}

/// A 10-point-tall rectangle at `x`, added to the library as graphic `name` when there is one →
/// its ids.
fn art(app: &mut VectorcraftApp, x: f64, name: Option<&str>) -> Vec<vectorcraft_doc::NodeId> {
    let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 10.0 + x, "height": 20})).unwrap()["id"].as_u64().unwrap();
    if let Some(name) = name {
        app.run("library.add", json!({"kind": "graphic", "ids": [id], "name": name})).unwrap();
    }
    vec![vectorcraft_doc::NodeId(id)]
}

/// New art at `x`, as the canvas drags it.
fn art_drag(app: &mut VectorcraftApp, x: f64) -> PanelDrag {
    let ids = art(app, x, None);
    PanelDrag::Art { document: app.session.active().unwrap().uid, ids }
}

fn graphics(app: &mut VectorcraftApp) -> Vec<String> {
    let got = app.run("library.get", json!({})).unwrap();
    got["graphics"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap().to_string()).collect()
}

/// The search field filters items by name, ignoring case; groups show after the ungrouped
/// items, and a search nothing matches says so (#926).
#[test]
fn the_panel_searches_and_shows_groups() {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    for (name, color) in [("Ocean Blue", "#0044aa"), ("Sun Yellow", "#ffcc00")] {
        app.run("library.add", json!({"kind": "fillColor", "color": color, "name": name})).unwrap();
    }
    let t = app.run("text.create", json!({"x": 10, "y": 80, "text": "Type"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [t]})).unwrap();
    app.run("library.add", json!({"kind": "charStyle", "name": "Headline"})).unwrap();
    app.run("library.createGroup", json!({"name": "Warm", "items": [{"kind": "fillColor", "item": "Sun Yellow"}]})).unwrap();
    let mut panel = Harness::new(app, show);
    panel.frame();
    assert!(panel.shows("Headline") && panel.shows("Warm"));
    assert!(panel.rect("Headline").top() < panel.rect("Warm").top(), "groups come after the ungrouped items");
    // "BLUE" finds the blue swatch only: the style and the group without a match are hidden.
    panel.type_into("Search", "BLUE");
    assert!(panel.shows("Ocean Blue") && !panel.shows("Headline") && !panel.shows("Warm"));
    panel.type_into("Search", "yellow");
    assert!(panel.shows("Warm") && !panel.shows("Headline"));
    panel.type_into("Search", "zzz");
    assert!(panel.shows("No results"));
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

fn style_order(app: &mut VectorcraftApp) -> Vec<String> {
    let got = app.run("library.get", json!({})).unwrap();
    got["charStyles"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap().to_string()).collect()
}

fn group_names(app: &mut VectorcraftApp) -> Vec<(String, Vec<String>)> {
    let got = app.run("library.get", json!({})).unwrap();
    let items = |g: &Value| g["items"].as_array().unwrap().iter().map(|i| i["item"].as_str().unwrap().to_string()).collect();
    got["groups"].as_array().unwrap().iter().map(|g| (g["name"].as_str().unwrap().to_string(), items(g))).collect()
}

/// The panel over styles A to D, D in Warm and Cool empty.
fn ordered() -> Harness {
    let mut app = app();
    styled(&mut app, &["A", "B", "C", "D"], &[("Warm", &["D"]), ("Cool", &[])]);
    let mut panel = Harness::new(app, show);
    panel.frame();
    panel
}

/// The panel, with the drag source, over graphics Badge and Seal as rows, Seal in Logos.
fn logos() -> Harness {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    art(&mut app, 10.0, Some("Badge"));
    art(&mut app, 20.0, Some("Seal"));
    app.run("library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": "Seal"}]})).unwrap();
    app.ui.libraries_list_view = true;
    let mut panel = Harness::new(app, with_source);
    panel.frame();
    panel
}

fn item_onto_an_items_top_half_lands_before_it() {
    let mut panel = ordered();
    let to = top(&panel, "A");
    panel.drag_to("C", to, Modifiers::NONE);
    panel.frame();
    assert_eq!(style_order(&mut panel.app()), ["C", "A", "B", "D"]);
}

fn item_onto_a_grouped_items_bottom_half_lands_after_it_in_its_group() {
    let mut panel = ordered();
    let to = bottom(&panel, "D");
    panel.drag_to("A", to, Modifiers::NONE);
    panel.frame();
    assert_eq!(style_order(&mut panel.app()), ["B", "C", "D", "A"]);
    assert_eq!(group_names(&mut panel.app()), [("Warm".into(), vec!["D".to_string(), "A".into()]), ("Cool".into(), vec![])]);
}

fn item_onto_a_group_header_goes_into_that_group() {
    let mut panel = ordered();
    panel.drag("B", "Cool", Modifiers::NONE);
    panel.frame();
    assert_eq!(style_order(&mut panel.app()), ["A", "B", "C", "D"], "the order is kept");
    assert_eq!(group_names(&mut panel.app())[1], ("Cool".into(), vec!["B".to_string()]));
}

fn group_header_onto_a_groups_top_half_moves_the_group() {
    let mut panel = ordered();
    let to = top(&panel, "Warm");
    // The drag starts on Cool's title row (its heading), not the middle of the section, which lies in its body.
    let from = panel.rect_role("Cool", craft::ui::contract::Role::HEADING).center();
    panel.drag_between(from, to, Modifiers::NONE);
    panel.frame();
    assert_eq!(group_names(&mut panel.app()).iter().map(|(g, _)| g.as_str()).collect::<Vec<_>>(), ["Cool", "Warm"]);
}

fn item_dragged_out_of_every_group_leaves_its_group() {
    let mut app = app();
    styled(&mut app, &["A", "B", "C"], &[("Warm", &["A", "B", "C"])]);
    let mut panel = Harness::new(app, show);
    panel.frame();
    assert!(!panel.shows("Drop here to take it out of its group"), "no strip without a drag");
    // Every item is grouped: released away from the groups, C leaves its group.
    panel.drag_to("C", BELOW, Modifiers::NONE);
    panel.frame();
    assert_eq!(group_names(&mut panel.app()), [("Warm".into(), vec!["A".to_string(), "B".into()])]);
}

fn art_onto_a_graphics_top_half_is_added_before_it_ungrouped() {
    let mut panel = logos();
    let drag = art_drag(&mut panel.app(), 30.0);
    let at = top(&panel, "Badge");
    drop_at(&mut panel, drag, at);
    let names = graphics(&mut panel.app());
    assert_eq!(names.get(1..), Some(&["Badge".to_string(), "Seal".into()][..]));
    assert_eq!(group_names(&mut panel.app())[0].1.len(), 1, "only Seal is in Logos");
}

fn art_onto_a_grouped_graphics_bottom_half_is_added_after_it_in_its_group() {
    let mut panel = logos();
    let drag = art_drag(&mut panel.app(), 40.0);
    let at = bottom(&panel, "Seal");
    drop_at(&mut panel, drag, at);
    let names = graphics(&mut panel.app());
    assert_eq!(names.get(..2), Some(&["Badge".to_string(), "Seal".into()][..]));
    assert_eq!(names.len(), 3);
    assert_eq!(group_names(&mut panel.app())[0].1.len(), 2, "the new graphic is in Logos");
}

fn a_colour_onto_a_group_header_is_added_to_that_group() {
    let mut panel = logos();
    let at = panel.rect("Logos").center();
    drop_at(&mut panel, PanelDrag::paint(Paint::solid(vectorcraft_color::Color::from_hex("#0044aa").unwrap())), at);
    let got = panel.app().run("library.get", json!({})).unwrap();
    let color = got["colors"][0]["name"].as_str().unwrap().to_string();
    assert!(group_names(&mut panel.app())[0].1.contains(&color));
}

fn art_below_the_items_is_added_ungrouped_at_the_end() {
    let mut panel = logos();
    let drag = art_drag(&mut panel.app(), 50.0);
    drop_at(&mut panel, drag, BELOW);
    let got = panel.app().run("library.get", json!({})).unwrap();
    let last = got["graphics"].as_array().unwrap().last().unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(graphics(&mut panel.app()).len(), 3);
    assert!(!group_names(&mut panel.app())[0].1.contains(&last));
}

fn an_empty_library_takes_art_anywhere() {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    let mut panel = Harness::new(app, with_source);
    panel.frame();
    let drag = art_drag(&mut panel.app(), 0.0);
    let at = panel.rect("This library is empty").center();
    drop_at(&mut panel, drag, at);
    assert_eq!(graphics(&mut panel.app()).len(), 1);
}

fn gradients_and_patterns_come_from_swatches_and_paint_the_selection() {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    let sunset = json!({"stops": [{"offset": 0, "color": "#ffcc00"}, {"offset": 1, "color": "#cc3300"}]});
    app.run("swatch.new", json!({"name": "Sunset", "gradient": sunset})).unwrap();
    let dot = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [dot]})).unwrap();
    app.run("object.pattern.make", json!({"name": "Checks", "width": 16, "height": 16})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    app.ui.libraries_list_view = true;
    let mut panel = Harness::new(app, with_source);
    panel.frame();
    for name in ["Sunset", "Checks"] {
        let paint = panel.app().session.active().unwrap().doc.swatch(name).unwrap().paint.clone();
        drop_at(&mut panel, PanelDrag::Paint { paint, params: json!({"swatch": name}), rows: None, library: None }, BELOW);
    }
    {
        let app = panel.app();
        let lib = app.session.libraries.get("Brand").unwrap();
        assert_eq!((lib.gradients.len(), lib.patterns.len()), (1, 1));
    }
    for s in ["Gradients", "Sunset", "Patterns", "Checks"] {
        assert!(panel.shows(s), "{s} shows");
    }
    let id = panel.app().run("shape.rectangle", json!({"x": 40, "y": 40, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
    panel.app().run("select.set", json!({"ids": [id]})).unwrap();
    let fill = |panel: &Harness| panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
    panel.click("Sunset");
    assert!(matches!(fill(&panel), Paint::Gradient(_)), "{:?}", fill(&panel));
    panel.click("Checks");
    assert!(matches!(fill(&panel), Paint::Pattern { ref pattern, .. } if pattern == "Checks"), "{:?}", fill(&panel));
}

fn the_list_view_shows_hex_values_and_takes_drops_by_row() {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    for (name, color) in [("Ocean Blue", "#0044aa"), ("Sun Yellow", "#ffcc00")] {
        app.run("library.add", json!({"kind": "fillColor", "color": color, "name": name})).unwrap();
    }
    let mut panel = Harness::new(app, show);
    panel.frame();
    // A list row's text after its name is read with the row, as its description.
    let beside = |panel: &Harness, name: &str, hex: &str| panel.description(name).is_some_and(|d| d.contains(hex));
    assert!(!panel.shows("#0044aa"), "chips by default");
    panel.app().ui.libraries_list_view = true;
    panel.frame();
    assert!(beside(&panel, "Ocean Blue", "#0044aa") && beside(&panel, "Sun Yellow", "#ffcc00"), "a colour's hex value beside its name");
    // Sun Yellow onto Ocean Blue's top half: first.
    let to = top(&panel, "Ocean Blue");
    panel.drag_to("Sun Yellow", to, Modifiers::NONE);
    panel.frame();
    let got = panel.app().run("library.get", json!({})).unwrap();
    let names: Vec<&str> = got["colors"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Sun Yellow", "Ocean Blue"]);
    panel.app().ui.libraries_list_view = false;
    panel.frame();
    assert!(!panel.shows("#0044aa"), "back to chips");
}

/// Dragging in the panel and dropping on it: items move before or after another of their kind
/// and into or out of groups, groups move by their headers, art and swatches are added where they
/// are dropped, and a click on a gradient or pattern paints the selection.
#[test]
fn gestures_on_the_panel_run_library_commands() {
    let rows: [(&str, fn()); 12] = [
        ("an item onto another's top half lands before it", item_onto_an_items_top_half_lands_before_it),
        ("an item onto a grouped item's bottom half lands after it, in its group", item_onto_a_grouped_items_bottom_half_lands_after_it_in_its_group),
        ("an item onto a group header goes into that group", item_onto_a_group_header_goes_into_that_group),
        ("a group header onto a group's top half moves the group", group_header_onto_a_groups_top_half_moves_the_group),
        ("an item dragged away from the groups leaves its group", item_dragged_out_of_every_group_leaves_its_group),
        ("art onto a graphic's top half is added before it", art_onto_a_graphics_top_half_is_added_before_it_ungrouped),
        (
            "art onto a grouped graphic's bottom half is added after it, in its group",
            art_onto_a_grouped_graphics_bottom_half_is_added_after_it_in_its_group,
        ),
        ("a colour onto a group header is added to that group", a_colour_onto_a_group_header_is_added_to_that_group),
        ("art below the items is added ungrouped", art_below_the_items_is_added_ungrouped_at_the_end),
        ("an empty library takes art anywhere", an_empty_library_takes_art_anywhere),
        ("gradients and patterns come from swatches and paint the selection", gradients_and_patterns_come_from_swatches_and_paint_the_selection),
        ("the list view shows hex values and takes drops by row", the_list_view_shows_hex_values_and_takes_drops_by_row),
    ];
    for (name, row) in rows {
        eprintln!("row: {name}");
        CARRIED.with(|carried| *carried.borrow_mut() = None);
        row();
    }
}

/// A spot swatch dropped on the list (as the Swatches panel drags it, with its name) is added as
/// a spot colour of that ink, at its tint (#5812).
#[test]
fn a_spot_swatch_dropped_on_the_list_stays_spot() {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    app.run("swatch.new", json!({"name": "Kiss-cut", "color": "#ff00ff", "spot": true})).unwrap();
    app.ui.libraries_list_view = true;
    let mut panel = Harness::new(app, with_source);
    panel.frame();
    for tint in [1.0, 0.5] {
        let paint = panel.app().session.active().unwrap().doc.tint_paint("Kiss-cut", tint).unwrap();
        let drag = PanelDrag::Paint { params: crate::panels::paint_params(&paint), paint, rows: None, library: None };
        drop_at(&mut panel, drag, BELOW);
    }
    let app = panel.app();
    let lib = app.session.libraries.get("Brand").unwrap();
    let inks: Vec<_> = lib.colors.iter().map(|c| (c.spot.as_deref(), c.tint)).collect();
    assert_eq!(inks, [(Some("Kiss-cut"), 1.0), (Some("Kiss-cut"), 0.5)]);
}

/// The open dialog opened last, and the value of its field `name`.
fn last_dialog(app: &VectorcraftApp) -> (craft::ui::dialog::DialogId, Value) {
    let open = open_dialogs(app);
    let last = open.last().expect("a dialog is open");
    (last.id, last.fields.as_ref().and_then(|f| f.get("name")).cloned().unwrap_or_default())
}

fn group_dialogs_run_with_their_library_and_group() {
    let mut app = app();
    let lib = app.run("library.create", json!({"name": "Brand"})).unwrap()["id"].as_str().unwrap().to_string();
    app.run("library.add", json!({"kind": "fillColor", "color": "#0044aa", "name": "Blue"})).unwrap();
    let mut panel = Harness::new(app, show);
    panel.frame();
    ask_group_name(&mut panel.app(), &lib, None, json!([{"kind": "fillColor", "item": "Blue"}]));
    let (id, name) = last_dialog(&panel.app());
    assert_eq!(name, json!("Group"));
    answer_dialog(&mut panel.app(), id, &[("name", json!("Cool"))], true);
    ask_group_name(&mut panel.app(), &lib, Some("Cool"), Value::Null);
    let (id, name) = last_dialog(&panel.app());
    assert_eq!(name, json!("Cool"));
    answer_dialog(&mut panel.app(), id, &[("name", json!("Cold"))], true);
    let got = panel.app().run("library.get", json!({})).unwrap();
    assert_eq!(got["groups"], json!([{"name": "Cold", "items": [{"kind": "color", "item": "Blue"}]}]));
}

fn a_character_styles_menu_applies_adds_renames_and_deletes() {
    let mut app = app();
    let data = json!({"name": "Type", "charStyles": [{"name": "Headline", "attrs": {"size": 31.0}}], "paraStyles": [{"name": "Body", "attrs": {}}]});
    let lib = app.run("library.import", json!({"data": data.to_string()})).unwrap()["id"].as_str().unwrap().to_string();
    let mut panel = Harness::new(app, show);
    panel.frame();
    // No type selected: Apply is there but disabled.
    panel.secondary_click("Headline");
    assert!(panel.shows("Apply Character Style") && !panel.enabled("Apply Character Style"));
    panel.click("Apply Character Style");
    assert!(panel.app().session.active().unwrap().doc.char_styles.is_empty(), "disabled without type");
    panel.key(Key::Escape, Modifiers::NONE);
    // With type selected: a right-click applies nothing; Add adds the style, unapplied, in one step.
    let t = panel.app().run("text.create", json!({"x": 10, "y": 80, "text": "Type", "size": 9})).unwrap()["id"].as_u64().unwrap();
    panel.app().run("select.set", json!({"ids": [t]})).unwrap();
    panel.frame();
    let style_of = |panel: &Harness| match &panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(t)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(text) => (text.runs[0].style.size, text.runs[0].style.style_name.clone()),
        _ => panic!("not text"),
    };
    panel.secondary_click("Headline");
    assert_eq!(style_of(&panel), (9.0, None), "a right-click applies nothing");
    let undo = panel.app().session.active().unwrap().history.undo.len();
    panel.click("Add to Character Styles");
    {
        let app = panel.app();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.char_styles.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Headline"]);
    }
    assert_eq!(style_of(&panel), (9.0, None), "added, not applied");
    assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
    // Apply applies it.
    panel.secondary_click("Headline");
    panel.click("Apply Character Style");
    assert_eq!(style_of(&panel), (31.0, Some("Headline".into())));
    // Rename…: Cancel keeps the name, OK renames.
    for (ok, want) in [(false, "Headline"), (true, "Title")] {
        panel.secondary_click("Headline");
        panel.click("Rename…");
        let (id, name) = last_dialog(&panel.app());
        assert_eq!(name, json!("Headline"), "the dialog opens with the style's name");
        if ok {
            answer_dialog(&mut panel.app(), id, &[("name", json!("Title"))], true);
        } else {
            let ctx = panel.ctx().clone();
            let dialogs = panel.app().dialogs().expect("a window");
            dialogs.with(|host| host.cancel_by_id(&ctx, id));
            crate::dialogs::host::pump(&mut panel.app(), &ctx);
        }
        panel.frame();
        assert_eq!(style_order(&mut panel.app()), [want]);
    }
    // A paragraph style's menu has no Apply or Add.
    panel.secondary_click("Body");
    assert!(panel.shows("Move to Group") && panel.shows("Delete"));
    assert!(!panel.shows("Apply Character Style") && !panel.shows("Add to Character Styles"));
    panel.key(Key::Escape, Modifiers::NONE);
    // Delete removes the character style from the library only.
    panel.secondary_click("Title");
    panel.click("Delete");
    assert!(panel.app().session.libraries.get(&lib).unwrap().char_styles.is_empty());
    assert_eq!(panel.app().session.active().unwrap().doc.char_styles.len(), 1, "the document's style stays");
}

fn the_panel_menu_creates_a_library_and_acts_on_the_current_one() {
    let mut panel = Harness::new(app(), show);
    panel.frame();
    let enabled = |panel: &Harness, id: &str| {
        let mut states = CommandStates::new();
        menu_states(&panel.app(), &mut states);
        states.get(&CommandRef::new(id)).is_some_and(CommandState::is_enabled)
    };
    assert!(enabled(&panel, CREATE) && !enabled(&panel, DELETE) && !enabled(&panel, EXPORT), "nothing to act on without a library");
    panel.app().run(CREATE, json!({})).unwrap();
    let (id, name) = last_dialog(&panel.app());
    assert_eq!(name, json!("My Library"));
    answer_dialog(&mut panel.app(), id, &[("name", json!("Brand"))], true);
    panel.frame();
    assert_eq!(panel.app().session.libraries.all().iter().map(|(_, l)| l.name.as_str()).collect::<Vec<_>>(), ["Brand"]);
    assert!(enabled(&panel, DELETE) && enabled(&panel, EXPORT));
    panel.app().run(RENAME, json!({})).unwrap();
    let (id, name) = last_dialog(&panel.app());
    assert_eq!(name, json!("Brand"), "Rename Library… opens with the library's name");
    answer_dialog(&mut panel.app(), id, &[("name", json!("House"))], true);
    assert_eq!(panel.app().session.libraries.all().iter().map(|(_, l)| l.name.as_str()).collect::<Vec<_>>(), ["House"]);
}

/// The panel menu's rows and the name dialogs: each dialog opens with its name, and OK runs its
/// command with its library, group or item.
#[test]
fn menu_rows_and_dialogs_run_library_commands() {
    let rows: [(&str, fn()); 3] = [
        ("New Group… and Rename Group… run with their library and group", group_dialogs_run_with_their_library_and_group),
        ("a character style's menu applies, adds, renames and deletes", a_character_styles_menu_applies_adds_renames_and_deletes),
        ("the panel menu creates a library and acts on the current one", the_panel_menu_creates_a_library_and_acts_on_the_current_one),
    ];
    for (name, row) in rows {
        eprintln!("row: {name}");
        row();
    }
}

/// Export Library… writes the library's file under its name and Import Library… adds it back as a
/// new library.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn libraries_are_exported_and_imported_from_the_panel_menu() {
    let dir = vectorcraft_testkit::temp_dir("libraries-panel");
    let path = dir.join("Brand.vclibrary").to_string_lossy().to_string();
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    app.run("library.add", json!({"kind": "fillColor", "color": "#0044aa", "name": "Blue"})).unwrap();
    let ctx = crate::testing::ui_context();
    let picker = scripted(&mut app);
    app.run(EXPORT, json!({})).unwrap();
    let asked: Vec<_> = picker.requests().into_iter().map(|r| r.file_name).collect();
    assert_eq!(asked, [Some("Brand.vclibrary".to_string())], "the library's name suggested");
    picker.answer_saved(&path, None);
    poll(&mut app, &ctx);
    assert_eq!(picker.written(), std::slice::from_ref(&path));
    // The file on disk, as the export left it, for Import Library… to read.
    let lib = app.session.libraries.get("Brand").unwrap();
    std::fs::write(&path, serde_json::to_vec(lib).unwrap()).unwrap();
    app.run(IMPORT, json!({})).unwrap();
    picker.answer(&[path.as_str()]);
    poll(&mut app, &ctx);
    let list = app.run("library.list", json!({})).unwrap();
    let names: Vec<&str> = list["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Brand", "Brand (2)"]);
    assert_eq!(app.ui.status, "Imported library Brand (2)");
}

/// A graphic's thumbnail shrunk for its tile keeps the color of the art beside clear pixels: the
/// clear black beside it does not darken it.
#[test]
fn a_shrunk_thumbnail_keeps_its_color_beside_clear_pixels() {
    let img = image::RgbaImage::from_fn(64, 4, |x, _| if x % 2 == 0 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) });
    let small = shrunk_thumbnail(image::DynamicImage::ImageRgba8(img), 16);
    assert_eq!(small.dimensions(), (16, 1));
    let m = |c: u8, a: u8| (u16::from(c) * u16::from(a) / 255) as u8;
    for p in small.pixels() {
        assert!(p[3] > 0 && [200, 30, 30].iter().zip(p.0).all(|(&want, c)| m(c, p[3]).abs_diff(m(want, p[3])) <= 1), "{p:?}");
    }
}

/// Graphic thumbnails decoded while the panel draws one frame.
fn decoded(panel: &mut Harness) -> usize {
    super::counters::DECODED.with(|c| c.set(0));
    panel.frame();
    super::counters::DECODED.with(std::cell::Cell::get)
}

/// A library of many graphics decodes their thumbnails at most 16 a frame, through the framework's
/// thumbnail store, and decodes none again once they show.
#[test]
fn graphic_thumbnails_decode_a_few_a_frame_and_once() {
    let mut app = app();
    app.run("library.create", json!({"name": "Brand"})).unwrap();
    for i in 0..60 {
        art(&mut app, f64::from(i), Some(&format!("Mark {i}")));
    }
    // The harness draws two frames as it opens.
    super::counters::DECODED.with(|c| c.set(0));
    let mut panel = Harness::new(app, show);
    let opening = super::counters::DECODED.with(std::cell::Cell::get);
    assert!(opening > 0 && opening <= 32, "{opening} decoded in the first two frames");
    let mut later = 0;
    for _ in 0..8 {
        later += decoded(&mut panel);
    }
    assert!(later > 0, "the rest decode in later frames");
    assert_eq!(decoded(&mut panel), 0, "a shown thumbnail is not decoded again");
}
