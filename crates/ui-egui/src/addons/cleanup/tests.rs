use serde_json::json;
use vectorcraft_engine::Session;

use super::{ID, STATE, check, fix, show};
use crate::panels::set_pstate;
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// An app with a rectangle and a stray point (a path of one anchor), nothing selected.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    app.run("path.create", json!({"anchors": [{"x": 80, "y": 80}]})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app
}

#[test]
fn the_plugins_submenu_opens_the_panel_and_lists_the_tools() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Artwork Cleanup"] && e.label == label);
    let item = under("Document Health Check").expect("listed");
    assert_eq!((item.command.as_deref(), &item.params), (Some("window.panel"), &json!({ "panel": ID })));
    for (label, id) in [
        ("Simplify Anchors (Keep Shape)…", "addon.cleanup.simplify"),
        ("Rejoin Path Pieces…", "addon.cleanup.rejoin"),
        ("Deep Ungroup…", "addon.cleanup.deepUngroup"),
        ("Fix All Issues", "addon.cleanup.fix"),
    ] {
        assert_eq!(under(label).and_then(|e| e.command.as_deref()), Some(id), "{label}");
    }
    // Install Plug-in… still follows.
    assert!(entries.iter().any(|e| e.path == ["Object", "Plug-ins"] && e.label == "Install Plug-in…"));
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, "window.panel", item.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    // The tools open their options first, as dialogs.
    let (target, p) = menus::click_target("Deep Ungroup…", "addon.cleanup.deepUngroup", &json!({"includeClipGroups": false}));
    assert_eq!((target.as_str(), &p["command"]), ("ui.paramDialog", &json!("addon.cleanup.deepUngroup")));
}

#[test]
fn check_lists_what_it_found_and_fix_clears_it() {
    let mut app = app();
    let text = painted_text(&mut app, show);
    assert!(text.contains("Check Document") && !text.contains("Stray Points"), "{text}");
    let r = check(&mut app, None);
    assert_eq!(r.scope, "document");
    assert_eq!(r.rows, [("strayPoints".to_string(), "Stray Points".to_string(), 1)]);
    let shown = r.clone();
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), STATE, shown.clone());
        show(app, ui);
    });
    assert!(text.contains("Stray Points") && text.contains("Fix Selected") && text.contains("Fix All"), "{text}");
    let steps = app.session.doc().unwrap().history.undo.len();
    let after = fix(&mut app, &r, vec!["strayPoints".into()]);
    assert!(after.rows.is_empty() && after.status.contains('1'), "{}", after.status);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), STATE, after.clone());
        show(app, ui);
    });
    assert!(text.contains("No issues found."), "{text}");
}

/// An app with the point type `text`, selected.
fn type_app(text: &str) -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 60, "text": text, "size": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    (app, id)
}

fn texts(app: &VectorcraftApp) -> Vec<String> {
    let mut out = vec![];
    for l in &app.session.doc().unwrap().doc.layers {
        l.walk(&mut |n| {
            if let vectorcraft_doc::NodeKind::Text(t) = &n.kind {
                out.push(t.plain_text());
            }
        });
    }
    out
}

#[test]
fn the_plugins_submenu_runs_the_text_tools() {
    let (mut app, _) = type_app("one two");
    let entries = menus::menu_entries(&app);
    let under = |path: &[&str], label: &str| entries.iter().find(|e| e.path == path && e.label == label);
    let cleanup = ["Object", "Plug-ins", "Artwork Cleanup"];
    for (label, id) in [
        ("Combine Point Text", "addon.cleanup.combineText"),
        ("Break Text into Lines", "addon.cleanup.breakText"),
        ("Break Text into Paragraphs", "addon.cleanup.breakText"),
        ("Break Text into Glyphs", "addon.cleanup.breakText"),
        ("Split Text at Cursor", "addon.cleanup.splitText"),
        ("Reset Text Transform", "addon.cleanup.resetTextTransform"),
        ("Reset Text Transform and Scale", "addon.cleanup.resetTextTransform"),
        ("Remove Line Breaks", "addon.cleanup.removeBreaks"),
        ("Remove Paragraph Breaks", "addon.cleanup.removeBreaks"),
        ("Replace Missing Fonts…", "addon.cleanup.replaceFonts"),
    ] {
        assert_eq!(under(&cleanup, label).and_then(|e| e.command.as_deref()), Some(id), "{label}");
    }
    let center = under(&["Object", "Plug-ins", "Artwork Cleanup", "Set Alignment, Keep Position"], "Center").expect("listed");
    assert_eq!((center.command.as_deref(), &center.params), (Some("addon.cleanup.alignInPlace"), &json!({"align": "center"})));
    // Replace Missing Fonts opens its options first, with the default type font.
    let fonts = under(&cleanup, "Replace Missing Fonts…").unwrap();
    let (target, p) = menus::click_target(&fonts.label, "addon.cleanup.replaceFonts", &fonts.params);
    assert_eq!((target.as_str(), &p["params"]["font"]), ("ui.paramDialog", &json!("Source Sans 3")));
    let words = under(&cleanup, "Break Text into Words").unwrap();
    menus::invoke(&mut app, "addon.cleanup.breakText", words.params.clone());
    assert_eq!(texts(&app), ["one", "two"]);
    let combine = under(&cleanup, "Combine Point Text").unwrap();
    menus::invoke(&mut app, "addon.cleanup.combineText", combine.params.clone());
    assert_eq!(texts(&app), ["one two"]);
}

/// One frame of the panel in `ctx` with `events` and `modifiers` held: the texts drawn and where.
fn frame_with(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<(String, egui::Rect)> {
    fn collect(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let time = ctx.input(|i| i.time) + 0.1;
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| collect(&c.shape, &mut v));
    v
}

fn panel_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    frame_with(app, ctx, events, egui::Modifiers::NONE)
}

/// Click the text `label` the panel draws, with `modifiers` held.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str, modifiers: egui::Modifiers) {
    let shown = panel_frame(app, ctx, vec![]);
    let at = shown.iter().find(|(s, _)| s == label).unwrap_or_else(|| panic!("no {label:?} in {shown:?}")).1.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)]] {
        frame_with(app, ctx, events, modifiers);
    }
}

#[test]
fn the_panels_text_row_runs_the_text_tools_and_hides() {
    let (mut app, _) = type_app("Hello there\nworld");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let steps = app.session.doc().unwrap().history.undo.len();
    // Shift-click on Lines breaks into paragraphs.
    click(&mut app, &ctx, "Lines", egui::Modifiers::SHIFT);
    assert_eq!(texts(&app), ["Hello there", "world"]);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    click(&mut app, &ctx, "Words", egui::Modifiers::NONE);
    assert_eq!(texts(&app), ["Hello", "there", "world"]);
    click(&mut app, &ctx, "Combine Point Text", egui::Modifiers::NONE);
    assert_eq!(texts(&app), ["Hello there", "world"]);
    // The row hides and shows again.
    click(&mut app, &ctx, "Text", egui::Modifiers::NONE);
    assert!(!panel_frame(&mut app, &ctx, vec![]).iter().any(|(s, _)| s == "Words"));
    click(&mut app, &ctx, "Text", egui::Modifiers::NONE);
    assert!(panel_frame(&mut app, &ctx, vec![]).iter().any(|(s, _)| s == "Words"));
}
