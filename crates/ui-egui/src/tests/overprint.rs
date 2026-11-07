//! Edit → Edit Colors → Overprint Black… opens its parameter dialog, which applies the command.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, menus};

#[test]
fn overprint_black_menu_opens_the_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
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
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), ("edit.colors.overprintBlack".to_string(), "Overprint Black".to_string()));
    for (k, v) in
        [("remove", json!(false)), ("percentage", json!(100)), ("fill", json!(true)), ("stroke", json!(true)), ("includeCmyBlacks", json!(false))]
    {
        assert_eq!(d.fields.get(k), Some(&v), "{k}");
    }

    // It draws headlessly, with its options labelled.
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| dialogs::show(app, ui.ctx()));
    for label in ["Overprint Black", "Percentage:", "Include Blacks with CMY:", "Include Spot Blacks:", "Remove:"] {
        assert!(text.contains(label), "{label} in {text}");
    }

    // Fill only, with rich blacks: the stroke stays knocked out.
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("includeCmyBlacks".into(), Value::Bool(true));
    d.fields.insert("stroke".into(), Value::Bool(false));
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let a = &app.session.doc().unwrap().doc.node(id).unwrap().appearance;
    assert!(a.fill().unwrap().overprint && !a.stroke().unwrap().overprint);
}
