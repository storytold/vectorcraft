//! Edit → Edit Colors → Overprint Black… opens its parameter dialog, which applies the command.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::nativeoptions::{answer, host, pass};
use super::picks::open_dialogs;
use crate::{VectorcraftApp, menus};

#[test]
fn overprint_black_menu_opens_the_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    host(&mut app);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    let rich = json!({"c": 60, "m": 40, "y": 40, "k": 100});
    app.run("paint.setFill", json!({"ids": [id.0], "color": rich})).unwrap();
    app.run("paint.setStroke", json!({"ids": [id.0], "color": rich})).unwrap();
    app.run("select.set", json!({"ids": [id.0]})).unwrap();

    let entry = menus::menu_entries(&app).into_iter().find(|e| e.label == "Overprint Black…").expect("menu item");
    assert_eq!(entry.path, ["Edit", "Edit Colors"]);
    assert!(entry.enabled);
    let (cmd, params) = menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    menus::invoke(&mut app, &cmd, params);
    let open = open_dialogs(&app);
    let dialog = open.last().expect("a dialog opens");
    assert_eq!(dialog.key.as_deref(), Some("edit.colors.overprintBlack"));
    let f = dialog.fields.clone().unwrap();
    for (k, v) in [("remove", json!(false)), ("fill", json!(true)), ("stroke", json!(true)), ("includeCmyBlacks", json!(false))] {
        assert_eq!(f[k], v, "{k}");
    }
    assert_eq!(f["percentage"].as_f64(), Some(100.0));

    // Fill only, with rich blacks: the stroke stays knocked out.
    answer(&mut app, &[("includeCmyBlacks", json!(true)), ("stroke", json!(false))], true);
    pass(&mut app);
    assert!(open_dialogs(&app).is_empty());
    let a = &app.session.doc().unwrap().doc.node(id).unwrap().appearance;
    assert!(a.fill().unwrap().overprint && !a.stroke().unwrap().overprint);
}
