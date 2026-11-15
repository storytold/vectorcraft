//! PSD Import Options: opening a PSD or PSB file with two or more layers, or placing one
//! without Link, asks whether its layers become objects or one flat image, and whether hidden
//! layers come in
//! (<https://helpx.adobe.com/illustrator/desktop/add-and-import-files/import-from-other-apps/photoshop-import-options.html>).
//! A linked file shows its flat image, so placing with Link doesn't ask. The two-layer threshold,
//! the starting choices and remembering them are VectorCraft's own.
//!
//! Fields: `mode` (`open` | `place`), `name`, `path?`, `width`, `height` (the canvas, px),
//! `layers` and `hidden` (counts, from `document.psdInfo`), `convert` (`objects` | `flatten`) and
//! `hiddenLayers`. `__place` keeps the other `file.place` params. The file's bytes stay in
//! [`crate::state::UiState::dialog_file`]. OK opens or places the file with `layers` and
//! `hiddenLayers` and remembers both choices for the next file; errors keep the dialog open.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use super::import_pdf::{DialogFile, other_place_params, place_source, with_file};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub const KIND: &str = "psdImport";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str("mode") == "place" { tl!("Place PSD") } else { tl!("PSD Import Options") }.into(),
    body,
    confirm,
    min_width: 380.0,
    ..DialogSpec::FORM
};

/// The fewest layers a file has for the dialog to ask.
const MIN_LAYERS: usize = 2;

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when they
/// are a PSD or PSB file with [`MIN_LAYERS`] layers or more; false when they aren't (the loader
/// then opens or places the file, or says why it can't).
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    let Ok(info) = fileio::psd_info(bytes) else { return false };
    if info.layers < MIN_LAYERS {
        return false;
    }
    app.ui.dialog_file = Some(DialogFile::new(bytes.to_vec()));
    let (layers, hidden) = last_choices(app);
    let mut fields = json!({
        "mode": if place.is_some() { "place" } else { "open" },
        "name": name,
        "width": info.width,
        "height": info.height,
        "layers": info.layers,
        "hidden": info.hidden,
        "convert": if layers { "objects" } else { "flatten" },
        "hiddenLayers": hidden,
    });
    if let Some(p) = path {
        fields["path"] = json!(p);
    }
    if let Some(p) = place {
        fields["__place"] = other_place_params(p, &["layers", "hiddenLayers"]);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    true
}

/// `file.place` with params `p` asks first (true) when they embed a PSD or PSB file (`link: false`,
/// or the file's bytes) and give no `layers`.
pub fn offer_place(app: &mut VectorcraftApp, p: &Value) -> bool {
    let embedded = p.get("path").is_none() || p.get("link").and_then(Value::as_bool) == Some(false);
    if !embedded || p.get("layers").is_some() {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| matches!(ext, "psd" | "pdd" | "psb")) else { return false };
    offer(app, &name, &bytes, path, Some(p))
}

/// The PSD Import Options chosen last, at first Convert Layers to Objects without hidden layers.
fn last_choices(app: &VectorcraftApp) -> (bool, bool) {
    let last = |k: &str, default: bool| app.ui.psd_import.get(k).and_then(Value::as_bool).unwrap_or(default);
    (last("layers", true), last("hiddenLayers", false))
}

/// Several files placed without Link load the place cursor without asking: the `file.place.queue`
/// params `q` take the PSD Import Options chosen last, which apply to each PSD or PSB file among them.
pub fn queue_options(app: &VectorcraftApp, q: &mut Value) {
    let (layers, hidden) = last_choices(app);
    q["layers"] = json!(layers);
    if layers && hidden {
        q["hiddenLayers"] = json!(true);
    }
}

/// OK: open or place the file with the choices.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let file = app.ui.dialog_file.clone().ok_or("the file is no longer loaded")?;
    let layers = d.str("convert") != "flatten";
    let hidden = layers && d.bool("hiddenLayers");
    app.ui.psd_import = json!({ "layers": layers, "hiddenLayers": d.bool("hiddenLayers") });
    let r = if d.str("mode") == "place" {
        let mut p = with_file(d.fields.get("__place"), d, &file);
        p["layers"] = json!(layers);
        if layers {
            // Layers are always embedded; a file from bytes may carry the Place dialog's Link.
            p["link"] = json!(false);
        }
        if hidden {
            p["hiddenLayers"] = json!(true);
        }
        crate::place::run_for_user(app, &p).map(|_| ())
    } else {
        let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
        io::open_document(app, &d.str("name"), &file.bytes, path, &json!({ "layers": layers, "hiddenLayers": hidden })).map(|_| ())
    };
    match r {
        Ok(()) => {
            // Placing may have opened another dialog; only this one closes.
            if app.ui.dialog.as_ref().is_some_and(|x| x.kind == KIND) {
                app.ui.dialog = None;
            }
            Ok(Value::Null)
        }
        Err(e) => {
            app.ui.dialog = Some(d.clone());
            Err(e)
        }
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let count = |k: &str| d.fields.get(k).and_then(Value::as_u64).unwrap_or(0);
    widgets::dim_label(ui, &format!("{} × {} px · {}: {}", count("width"), count("height"), tl!("Layers"), count("layers")));
    ui.add_space(8.0);
    let convert = d.str("convert") != "flatten";
    if widgets::radio(ui, tl!("Convert Layers to Objects"), convert, true) {
        d.fields.insert("convert".into(), json!("objects"));
    }
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        let on = d.bool("hiddenLayers");
        if widgets::check(ui, tl!("Import Hidden Layers"), on && convert, convert) {
            d.fields.insert("hiddenLayers".into(), json!(!on));
        }
    });
    if widgets::radio(ui, tl!("Flatten Layers to a Single Image"), !convert, true) {
        d.fields.insert("convert".into(), json!("flatten"));
    }
    false
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    use super::*;

    /// A PSD file of three layers, the middle one hidden, written by `document.export`.
    fn layered() -> Vec<u8> {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
        let back = s.doc().unwrap().doc.layers[0].id.0;
        s.execute("layer.setProps", &json!({"id": back, "name": "Back"})).unwrap();
        for (name, color) in [("Back", "#ff0000"), ("Middle", "#00ff00"), ("Front", "#0000ff")] {
            if name != "Back" {
                s.execute("layer.new", &json!({ "name": name })).unwrap();
            }
            s.execute("paint.setFill", &json!({ "color": color })).unwrap();
            s.execute("shape.rectangle", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        }
        let middle = s.doc().unwrap().doc.layers[1].id.0;
        s.execute("layer.setProps", &json!({"id": middle, "visible": false})).unwrap();
        let r = s.execute("document.export", &json!({"format": "psd", "hiddenLayers": true})).unwrap();
        vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
    }

    fn app() -> VectorcraftApp {
        VectorcraftApp::new(Session::new(), Default::default())
    }

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
    }

    fn layer_names(app: &VectorcraftApp) -> Vec<(String, bool)> {
        app.session.doc().unwrap().doc.layers.iter().map(|l| (l.name.clone().unwrap_or_default(), l.visible)).collect()
    }

    #[test]
    fn opening_a_layered_file_asks_for_the_import_options() {
        let psd = layered();
        let mut app = app();
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("mode"), d.str("convert"), d.bool("hiddenLayers")), (KIND, "open".into(), "objects".into(), false));
        assert_eq!((&d.fields["layers"], &d.fields["hidden"], &d.fields["width"]), (&json!(3), &json!(1), &json!(20)));
        assert!(app.session.active().is_none(), "nothing opens before OK");
        let text = frame(&mut app);
        for label in [
            "PSD Import Options",
            "20 × 20 px",
            "Layers: 3",
            "Convert Layers to Objects",
            "Import Hidden Layers",
            "Flatten Layers to a Single Image",
            "OK",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // Layers as objects, hidden ones left out.
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(layer_names(&app), [("Back".to_string(), true), ("Front".to_string(), true)]);
        // With Import Hidden Layers the hidden layer comes in hidden.
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        set(&mut app, "hiddenLayers", json!(true));
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert_eq!(layer_names(&app), [("Back".to_string(), true), ("Middle".to_string(), false), ("Front".to_string(), true)]);
        // Flatten opens one image, and the next file starts from the choices made last.
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("hiddenLayers"), "remembered");
        set(&mut app, "convert", json!("flatten"));
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.layers.len(), 1);
        assert!(matches!(st.doc.layers[0].children().map(Vec::as_slice), Some([one]) if matches!(one.kind, NodeKind::Image(_))));
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("convert"), "flatten");
    }

    #[test]
    fn files_with_fewer_than_two_layers_open_without_asking() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        let one = s.execute("document.export", &json!({"format": "psd"})).unwrap();
        let flat = s.execute("document.export", &json!({"format": "psd", "layers": false})).unwrap();
        for r in [one, flat] {
            let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
            let mut app = app();
            io::open_bytes(&mut app, "one.psd", &bytes, None).unwrap();
            assert!(app.ui.dialog.is_none());
            assert!(app.session.active().is_some());
        }
    }

    #[test]
    fn placing_without_link_asks_and_places_the_layers_as_a_group() {
        let b64 = vectorcraft_format::base64_encode(&layered());
        let mut app = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let r = crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64, "at": [50, 50]})).unwrap();
        assert_eq!(r["dialog"], KIND);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("mode"), &d.fields["__place"]), ("place".into(), &json!({"at": [50, 50]})));
        assert!(frame(&mut app).contains("Place PSD"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        let [id] = st.selection.objects[..] else { panic!("one object is selected") };
        let g = st.doc.node(id).unwrap();
        assert!(matches!(g.kind, NodeKind::Group { .. }));
        let names: Vec<&str> = g.children().unwrap().iter().filter_map(|c| c.name.as_deref()).collect();
        assert_eq!(names, ["Back", "Front"]);
        // Flatten places the merged image.
        crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64})).unwrap();
        set(&mut app, "convert", json!("flatten"));
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        let [id] = st.selection.objects[..] else { panic!("one object is selected") };
        assert!(matches!(st.doc.node(id).unwrap().kind, NodeKind::Image(_)));
    }

    #[test]
    fn several_files_placed_without_link_take_the_last_choices() {
        let dir = std::env::temp_dir().join(format!("vc-psd-import-queue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = ["a.psd", "b.psd"]
            .iter()
            .map(|n| {
                let p = dir.join(n);
                std::fs::write(&p, layered()).unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        // Place both through the Place dialog, then click the cursor once for each: what was placed.
        let place_two = |app: &mut VectorcraftApp, link: bool| -> Vec<vectorcraft_engine::doc::Node> {
            let files: Vec<Value> = paths.iter().map(|p| json!({ "path": p })).collect();
            let d = Dialog::new(crate::dialogs::place::KIND, json!({"files": files, "link": link, "template": false}));
            crate::place::confirm(app, &d).unwrap();
            assert!(app.ui.dialog.is_none(), "the cursor loads without asking");
            let view = vectorcraft_engine::ViewInfo::default();
            (0..2)
                .map(|_| {
                    for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
                        for r in app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 10.0, 10.0), view).unwrap() {
                            if let vectorcraft_engine::UiRequest::Place(p) = r {
                                crate::place::place_from_cursor(app, &p);
                            }
                        }
                    }
                    let st = app.session.doc().unwrap();
                    let [id] = st.selection.objects[..] else { panic!("one object is placed") };
                    st.doc.node(id).unwrap().clone()
                })
                .collect()
        };
        let visible = |n: &vectorcraft_engine::doc::Node| -> Vec<bool> { n.children().unwrap().iter().map(|c| c.visible).collect() };
        let mut app = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // At first, Convert Layers to Objects without hidden layers.
        for n in place_two(&mut app, false) {
            assert!(matches!(n.kind, NodeKind::Group { .. }));
            assert_eq!(visible(&n), [true, true]);
        }
        // Then the choices made last.
        app.ui.psd_import = json!({"layers": true, "hiddenLayers": true});
        for n in place_two(&mut app, false) {
            assert_eq!(visible(&n), [true, false, true]);
        }
        app.ui.psd_import = json!({"layers": false, "hiddenLayers": true});
        assert!(place_two(&mut app, false).iter().all(|n| matches!(n.kind, NodeKind::Image(_))));
        // Placed with Link, the files keep their flat images.
        app.ui.psd_import = json!({"layers": true, "hiddenLayers": false});
        assert!(place_two(&mut app, true).iter().all(|n| matches!(n.kind, NodeKind::Image(_))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scripted_places_and_places_that_give_layers_dont_ask() {
        let b64 = vectorcraft_format::base64_encode(&layered());
        let mut app = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // `file.place` run as a command places at once, as before.
        let r = crate::place::run(&mut app, &json!({"name": "poster.psd", "dataBase64": b64})).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(r["format"], "psd");
        // The user's place with `layers` given doesn't ask either.
        crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64, "layers": false})).unwrap();
        assert!(app.ui.dialog.is_none());
    }
}
