//! The canvas context menu: right-click selects the object under the pointer and lists what
//! applies to the selection.

use egui::{Event, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::canvas::Xf;
use crate::menus::{self, Item};
use crate::{VectorcraftApp, canvas};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64) -> NodeId {
    NodeId(app.run("shape.rectangle", json!({"x": x, "y": y, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap())
}

/// One headless frame of the canvas on an 800 × 600 window → the text painted, with its rect.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut v = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
    v
}

fn press(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers: Default::default() }
}

/// Click `button` at `pos` (move, press, release, then a frame to show the result).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, pos: Pos2, button: PointerButton) -> Vec<(String, Rect)> {
    frame(app, ctx, vec![Event::PointerMoved(pos)]);
    frame(app, ctx, vec![press(pos, button, true)]);
    frame(app, ctx, vec![press(pos, button, false)]);
    frame(app, ctx, vec![])
}

/// Where document point `p` is on screen (after a first frame laid the canvas out).
fn screen(app: &VectorcraftApp, x: f64, y: f64) -> Pos2 {
    Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(x, y))
}

fn has(texts: &[(String, Rect)], label: &str) -> bool {
    texts.iter().any(|(t, _)| t.trim_start_matches(['✓', ' ']) == label)
}

fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"))
}

#[test]
fn right_click_selects_the_object_and_lists_its_commands() {
    let mut app = app();
    let id = rect(&mut app, 50.0, 50.0);
    app.run("select.none", json!({})).unwrap();
    let ctx = egui::Context::default();
    assert!(!has(&frame(&mut app, &ctx, vec![]), "Arrange"));
    let p = screen(&app, 100.0, 100.0);
    let texts = click(&mut app, &ctx, p, PointerButton::Secondary);
    assert_eq!(app.session.active().unwrap().selection.objects, [id], "the object right-clicked is selected");
    for label in ["Undo Rectangle", "Cut", "Copy", "Make Guides", "Transform", "Arrange", "Select", "Export Selection…"] {
        assert!(has(&texts, label), "{label} in {texts:?}");
    }
    // Nothing that needs several objects, a group or nothing selected.
    for label in ["Group", "Ungroup", "Make Clipping Mask", "Release Compound Path", "Select All", "Zoom In"] {
        assert!(!has(&texts, label), "{label} in {texts:?}");
    }
    // Choosing an item runs its command and closes the menu.
    let texts = click(&mut app, &ctx, at(&texts, "Make Guides"), PointerButton::Primary);
    let n = app.session.active().unwrap().doc.node(id).cloned().unwrap();
    assert!(matches!(n.kind, NodeKind::Path { guide: true, .. }));
    assert!(!has(&texts, "Arrange"));
}

#[test]
fn right_click_on_empty_canvas_offers_the_view_and_a_dismissing_click_does_nothing() {
    let mut app = app();
    app.select_tool("rectangle");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let p = screen(&app, 300.0, 250.0);
    let texts = click(&mut app, &ctx, p, PointerButton::Secondary);
    for label in ["Zoom In", "Zoom Out", "Fit Artboard in Window", "Show Rulers", "Select All"] {
        assert!(has(&texts, label), "{label} in {texts:?}");
    }
    assert!(!has(&texts, "Cut") && !has(&texts, "Arrange"));
    // A click on the canvas away from the menu only closes it: the Rectangle tool draws nothing.
    let away = pos2(100.0, 500.0);
    frame(&mut app, &ctx, vec![Event::PointerMoved(away)]);
    frame(&mut app, &ctx, vec![press(away, PointerButton::Primary, true)]);
    frame(&mut app, &ctx, vec![Event::PointerMoved(away + vec2(40.0, 30.0))]);
    frame(&mut app, &ctx, vec![press(away + vec2(40.0, 30.0), PointerButton::Primary, false)]);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(!has(&texts, "Zoom In"));
    assert_eq!(app.session.active().unwrap().doc.art_bounds(), None);
}

/// #528: right-clicking a ruler offers the document units, the current one checked, and choosing one
/// swaps them — the same effect as Preferences ▸ Units ▸ General (`document.setUnits`).
#[test]
fn right_clicking_a_ruler_swaps_the_document_units() {
    use vectorcraft_doc::Unit;
    let mut app = app();
    app.ui.view.rulers = true;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.general_unit(), Unit::Points, "a new document starts in the prefs unit");
    // Right-click on the top ruler (the band along the top, past the origin box).
    let texts = click(&mut app, &ctx, pos2(400.0, 8.0), PointerButton::Secondary);
    for label in ["Points", "Picas", "Inches", "Millimeters", "Feet & Inches", "Pixels"] {
        assert!(has(&texts, label), "{label} in {texts:?}");
    }
    assert!(texts.iter().any(|(t, _)| t == "✓  Points"), "the current unit is checked: {texts:?}");
    // Choosing a unit runs document.setUnits and closes the menu.
    let mm = texts.iter().find(|(t, _)| t.trim_start_matches(['✓', ' ']) == "Millimeters").map(|(_, r)| r.center()).expect("Millimeters");
    let texts = click(&mut app, &ctx, mm, PointerButton::Primary);
    assert_eq!(app.session.general_unit(), Unit::Millimeters, "the ruler's unit changed");
    assert!(!has(&texts, "Inches"), "the menu closed");
    // The left ruler offers the same menu, now checking the new unit.
    let texts = click(&mut app, &ctx, pos2(8.0, 300.0), PointerButton::Secondary);
    assert!(texts.iter().any(|(t, _)| t == "✓  Millimeters"), "the left ruler reflects the current unit: {texts:?}");
}

fn labels(items: &[Item]) -> Vec<&'static str> {
    items
        .iter()
        .flat_map(|it| match it {
            Item::Cmd(l, ..) => vec![*l],
            Item::Sub(l, ch) => std::iter::once(*l).chain(labels(ch)).collect(),
            _ => vec![],
        })
        .collect()
}

#[test]
fn the_menu_follows_the_selection() {
    let mut app = app();
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 50.0, 50.0));
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    let l = labels(&menus::context_items(&app));
    for label in ["Group", "Join", "Make Clipping Mask", "Make Compound Path", "Bring to Front", "Move…"] {
        assert!(l.contains(&label), "{label} in {l:?}");
    }
    assert!(!l.contains(&"Ungroup") && !l.contains(&"Isolate Selected Group"));
    app.run("object.group", json!({})).unwrap();
    let l = labels(&menus::context_items(&app));
    assert!(l.contains(&"Ungroup") && l.contains(&"Isolate Selected Group"));
    assert!(!l.contains(&"Group") && !l.contains(&"Join"));
    // Isolated: a way out.
    menus::invoke(&mut app, "object.isolate", json!({}));
    assert!(labels(&menus::context_items(&app)).contains(&"Exit Isolation Mode"));
    // Agents see the same menu, with submenus as paths.
    let entries = menus::context_entries(&app);
    assert!(entries.iter().any(|e| e.command.as_deref() == Some("object.exitIsolation") && e.enabled));
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let entries = menus::context_entries(&app);
    let front = entries.iter().find(|e| e.command.as_deref() == Some("object.arrange.bringToFront")).unwrap();
    assert_eq!(front.path, ["Arrange"]);
    // No separator leads, trails or doubles up.
    let items = menus::context_items(&app);
    assert!(!matches!(items.first(), Some(Item::Sep)) && !matches!(items.last(), Some(Item::Sep)));
    assert!(items.windows(2).all(|w| !matches!(w, [Item::Sep, Item::Sep])));
}

/// Every label the context menu can show is a menu string, so complete catalogs must translate it.
#[test]
fn every_context_label_is_a_menu_string() {
    let strings = menus::menu_strings();
    let mut app = app();
    let mut shown = vec![];
    // Nothing selected, one path, two paths, a group, a clipping group, a compound path, isolation.
    shown.extend(labels(&menus::context_items(&app)));
    let (a, b) = (rect(&mut app, 0.0, 0.0), rect(&mut app, 50.0, 50.0));
    shown.extend(labels(&menus::context_items(&app)));
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    shown.extend(labels(&menus::context_items(&app)));
    app.run("object.compoundPath.make", json!({})).unwrap();
    shown.extend(labels(&menus::context_items(&app)));
    app.run("object.compoundPath.release", json!({})).unwrap();
    // A compound shape.
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.compoundShape.make", json!({})).unwrap();
    shown.extend(labels(&menus::context_items(&app)));
    app.run("object.compoundShape.release", json!({})).unwrap();
    app.run("select.set", json!({"ids": [a.0, b.0]})).unwrap();
    app.run("object.clippingMask.make", json!({})).unwrap();
    shown.extend(labels(&menus::context_items(&app)));
    app.run("object.group", json!({})).unwrap();
    shown.extend(labels(&menus::context_items(&app)));
    menus::invoke(&mut app, "object.isolate", json!({}));
    shown.extend(labels(&menus::context_items(&app)));
    let missing: Vec<_> = shown.iter().filter(|l| !strings.contains(**l)).collect();
    assert!(missing.is_empty(), "context menu labels missing from menu_strings: {missing:?}");
    for label in menus::CONTEXT_LABELS {
        assert!(shown.contains(label), "`{label}` is listed but the context menu never shows it");
    }
}
