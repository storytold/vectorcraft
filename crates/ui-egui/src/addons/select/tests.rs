use serde_json::json;
use vectorcraft_engine::Session;

use super::{click, shown};
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_search_filters_artboards_by_name_ignoring_case() {
    let n = names(&["Cover", "Page 2", "Back cover", "Inside"]);
    assert_eq!(shown(&n, ""), vec![0, 1, 2, 3]);
    assert_eq!(shown(&n, "  COVER "), vec![0, 2]);
    assert_eq!(shown(&n, "zzz"), Vec::<usize>::new());
}

#[test]
fn clicks_choose_toggle_and_extend_rows() {
    let rows = [0, 2, 3, 5];
    let mut c = vec![];
    let a = click(&mut c, None, 2, &rows, false, false);
    assert_eq!((c.clone(), a), (vec![2], Some(2)));
    // Shift adds the rows shown between the last one clicked and this one.
    let a = click(&mut c, a, 5, &rows, true, false);
    assert_eq!((c.clone(), a), (vec![2, 3, 5], Some(2)));
    // Cmd/Ctrl toggles a row.
    let a = click(&mut c, a, 3, &rows, false, true);
    assert_eq!((c.clone(), a), (vec![2, 5], Some(3)));
    let a = click(&mut c, a, 0, &rows, false, true);
    assert_eq!((c.clone(), a), (vec![0, 2, 5], Some(0)));
    // A plain click chooses that row alone; Shift without an anchor does too.
    let a = click(&mut c, a, 3, &rows, false, false);
    assert_eq!((c.clone(), a), (vec![3], Some(3)));
    let a = click(&mut c, None, 5, &rows, true, false);
    assert_eq!((c, a), (vec![5], Some(5)));
}

#[test]
fn select_by_artboards_lists_the_artboards_and_selects_on_ok() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("artboard.new", json!({"x": 150, "y": 0, "width": 200, "height": 50, "name": "Banner"})).unwrap();
    let inside = app.run("shape.rectangle", json!({"x": 160, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Selection Tools"] && e.label == "Select by Artboards…").unwrap();
    let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &cmd, p);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.kind, super::KIND);
    assert_eq!(d.fields["artboards"], json!([0]), "the active artboard is chosen");
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Select by Artboards", "Artboard 1", "Banner", "100 × 100 pt", "200 × 50 pt", "1 of 2 chosen", "Overlapping", "Select All", "None"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("artboards".into(), json!([]));
    assert!(!(super::SPEC.ok_enabled.unwrap())(&app, app.ui.dialog.as_ref().unwrap()), "OK is off with nothing chosen");
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("artboards".into(), json!([1]));
    d.fields.insert("relation".into(), json!("inside"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(inside)]);
}
