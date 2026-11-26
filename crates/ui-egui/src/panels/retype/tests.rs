use egui::accesskit::Node;
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;

use super::{ID, Size, show};
use crate::VectorcraftApp;

const SCREEN: egui::Rect = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(600.0, 900.0));

fn app() -> VectorcraftApp {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    app
}

/// Point text, outlined: the id of the outlines' group.
fn outlined(app: &mut VectorcraftApp, p: Value) -> u64 {
    let id = app.session.execute("text.create", &p).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.session.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap()
}

fn select(app: &mut VectorcraftApp, ids: &[u64]) {
    app.session.execute("select.set", &json!({ "ids": ids })).unwrap();
}

/// One frame of `draw` with screen-reader nodes on → the nodes.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<Node> {
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(SCREEN), ..Default::default() }, |ui| draw(app, ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

fn center(nodes: &[Node], name: &str) -> Option<egui::Pos2> {
    let b = nodes.iter().find(|n| n.label().is_some_and(|l| l == name))?.bounds()?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// Click the node labeled `name` in the panel.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    frame(app, ctx, vec![], show);
    let at = center(&frame(app, ctx, vec![], show), name).unwrap_or_else(|| panic!("{name} is drawn"));
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![egui::Event::PointerMoved(at), press(true)], show);
    frame(app, ctx, vec![press(false)], show);
}

fn context() -> egui::Context {
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    ctx
}

fn families(list: &Value) -> Vec<(String, String)> {
    list["fonts"].as_array().unwrap().iter().map(|f| (f["family"].as_str().unwrap().to_string(), f["style"].as_str().unwrap().to_string())).collect()
}

#[test]
fn find_lists_the_matching_fonts_and_shows_the_panel() {
    let mut app = app();
    let g = outlined(&mut app, json!({"x": 40, "y": 100, "text": "Find me", "size": 30, "font": "Source Sans 3", "style": "Bold"}));
    select(&mut app, &[g]);
    let before = app.session.active().unwrap().doc.clone();
    let r = app.run("retype.find", json!({})).unwrap();
    assert_eq!(r["searched"], json!(true), "{r}");
    assert_eq!(r["shapes"], json!(6));
    // The panel reads the app's shared font database, where other tests load renamed copies of the
    // bundled fonts; a copy ties with its original, so the font is checked among the first rank.
    let fonts = r["fonts"].as_array().unwrap();
    let top = fonts.first().map(|f| f["matched"].clone());
    assert!(fonts.iter().any(|f| f["family"] == json!("Source Sans 3") && f["style"] == json!("Bold") && Some(f["matched"].clone()) == top), "{r}");
    assert!(families(&r).len() <= 10, "ten at first: {r}");
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel shows");
    assert!(app.session.active().unwrap().doc == before, "the document does not change");
    assert_eq!(app.run("retype.list", json!({})).unwrap(), r);
}

#[test]
fn more_adds_fonts_until_none_is_left() {
    let mut app = app();
    let g = outlined(&mut app, json!({"x": 40, "y": 100, "text": "More", "size": 30}));
    select(&mut app, &[g]);
    // The installed fonts are left out, so the searches compare the same fonts.
    let first = app.run("retype.find", json!({"installed": false})).unwrap();
    // How many fonts match depends on the fonts loaded: ask until none is left.
    let mut last = first.clone();
    for _ in 0..20 {
        if last["more"] == json!(false) {
            break;
        }
        let next = app.run("retype.find", json!({"more": true})).unwrap();
        let added = families(&next).len() - families(&last).len();
        assert!((1..=5).contains(&added), "{added} added: {next}");
        last = next;
    }
    assert_eq!(last["more"], json!(false), "{last}");
    assert_eq!(app.run("retype.find", json!({"more": true})).unwrap(), last, "nothing is left to add");
    // A null more is omitted: a new search lists the first ten again. A font added to the list
    // tells the new search apart from adding more when ten or fewer fonts match.
    app.ui.retype.fonts.push(super::Found { family: "Not Found".into(), style: "Regular".into(), matched: 0 });
    assert_eq!(app.run("retype.find", json!({"more": null, "installed": false})).unwrap(), first);
    // Before any search, more is a first search.
    app.run("retype.reset", json!({})).unwrap();
    assert_eq!(app.run("retype.find", json!({"more": true, "installed": false})).unwrap(), first);
    // A bad installed is the command's error, and leaves the list as it was.
    for p in [json!({"installed": 1}), json!({"more": true, "installed": "yes"})] {
        let e = app.run("retype.find", p.clone()).unwrap_err();
        assert!(e.contains("`installed` must be true, false or null"), "{p}: {e}");
        assert_eq!(app.run("retype.list", json!({})).unwrap(), first, "{p}");
    }
    assert!(app.run("retype.find", json!({"more": "yes"})).is_err());
}

#[test]
fn more_shows_the_next_five_the_search_found_whatever_is_selected_now() {
    let mut app = app();
    let font = |i: usize| super::Found { family: format!("Family {i}"), style: "Regular".into(), matched: 1 };
    app.ui.retype.searched = true;
    app.ui.retype.fonts = (0..10).map(font).collect();
    app.ui.retype.rest = (10..17).map(font).collect();
    // Live type is selected to take a listed font: it holds no shape to compare.
    let t = app.session.execute("text.create", &json!({"x": 10, "y": 300, "text": "live"})).unwrap()["id"].as_u64().unwrap();
    select(&mut app, &[t]);
    let r = app.run("retype.find", json!({"more": true})).unwrap();
    assert_eq!(families(&r).len(), 15, "{r}");
    assert_eq!(families(&r)[10].0, "Family 10");
    assert_eq!(r["more"], json!(true), "two are left");
    let r = app.run("retype.find", json!({"more": true})).unwrap();
    assert_eq!(families(&r).len(), 17, "{r}");
    assert_eq!(r["more"], json!(false));
    assert_eq!(app.run("retype.find", json!({"more": true})).unwrap(), r, "nothing is left to add");
}

#[test]
fn a_failed_search_leaves_the_list_as_it_was() {
    let mut app = app();
    let g = outlined(&mut app, json!({"x": 40, "y": 100, "text": "Kept", "size": 30}));
    select(&mut app, &[g]);
    let listed = app.run("retype.find", json!({})).unwrap();
    let t = app.session.execute("text.create", &json!({"x": 10, "y": 300, "text": "live"})).unwrap()["id"].as_u64().unwrap();
    select(&mut app, &[t]);
    assert!(app.run("retype.find", json!({})).is_err(), "live type alone holds no shape");
    assert_eq!(app.run("retype.list", json!({})).unwrap(), listed);
}

#[test]
fn preview_sets_the_size_and_text_and_reset_keeps_them() {
    let mut app = app();
    let r = app.run("retype.preview", json!({"size": "large", "text": "Hamburg"})).unwrap();
    assert_eq!((r["size"].clone(), r["text"].clone()), (json!("large"), json!("Hamburg")));
    assert_eq!(app.ui.retype.size, Size::Large);
    for bad in [json!({"size": "huge"}), json!({"size": 2}), json!({"text": 5}), json!({"text": "x".repeat(101)})] {
        assert!(app.run("retype.preview", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(app.ui.retype.size, Size::Large, "a bad value changes nothing");
    assert_eq!(app.ui.retype.text, "Hamburg");
    let g = outlined(&mut app, json!({"x": 40, "y": 100, "text": "R", "size": 30}));
    select(&mut app, &[g]);
    app.run("retype.find", json!({})).unwrap();
    let r = app.run("retype.reset", json!({})).unwrap();
    assert_eq!(r, json!({"searched": false, "fonts": [], "shapes": 0, "more": false, "truncated": false, "size": "large", "text": "Hamburg"}));
}

#[test]
fn a_search_that_left_out_shapes_says_so() {
    let mut app = app();
    let top = outlined(&mut app, json!({"x": 40, "y": 100, "text": "A", "size": 40}));
    // B, nested deeper than Retype looks.
    let mut deep = outlined(&mut app, json!({"x": 40, "y": 200, "text": "B", "size": 40}));
    for _ in 0..20 {
        select(&mut app, &[deep]);
        deep = app.session.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    }
    select(&mut app, &[top, deep]);
    let r = app.run("retype.find", json!({})).unwrap();
    assert_eq!((r["shapes"].clone(), r["truncated"].clone()), (json!(1), json!(true)), "{r}");
    assert_eq!(app.run("retype.list", json!({})).unwrap()["truncated"], json!(true), "the list says so");
    // A search that left nothing out, and a reset, clear it.
    select(&mut app, &[top]);
    assert_eq!(app.run("retype.find", json!({})).unwrap()["truncated"], json!(false));
    select(&mut app, &[top, deep]);
    app.run("retype.find", json!({})).unwrap();
    assert_eq!(app.run("retype.reset", json!({})).unwrap()["truncated"], json!(false));
}

#[test]
fn choosing_a_listed_font_sets_it_on_the_selected_live_text() {
    let mut app = app();
    let g = outlined(&mut app, json!({"x": 40, "y": 100, "text": "Serif", "size": 30, "font": "Source Serif 4", "style": "Regular"}));
    select(&mut app, &[g]);
    app.run("retype.find", json!({})).unwrap();
    let t = app.session.execute("text.create", &json!({"x": 10, "y": 300, "text": "live", "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
    select(&mut app, &[t]);
    let ctx = context();
    click(&mut app, &ctx, "Source Serif 4 Regular");
    let doc = &app.session.active().unwrap().doc;
    let NodeKind::Text(text) = &doc.node(NodeId(t)).unwrap().kind else { panic!("text") };
    assert_eq!(text.first_style().font_family, "Source Serif 4");
    // Live type inside a selected group takes the font too, as with the font menus.
    let inner =
        app.session.execute("text.create", &json!({"x": 10, "y": 350, "text": "grouped", "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
    select(&mut app, &[inner]);
    let group = app.session.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    select(&mut app, &[group]);
    click(&mut app, &ctx, "Source Serif 4 Regular");
    let doc = &app.session.active().unwrap().doc;
    let NodeKind::Text(text) = &doc.node(NodeId(inner)).unwrap().kind else { panic!("text") };
    assert_eq!(text.first_style().font_family, "Source Serif 4");
    // With no live text selected the rows choose nothing.
    select(&mut app, &[g]);
    let before = app.session.active().unwrap().doc.clone();
    click(&mut app, &ctx, "Source Serif 4 Regular");
    assert!(app.session.active().unwrap().doc == before);
}
