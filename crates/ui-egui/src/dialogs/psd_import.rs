//! PSD Import Options: opening a PSD or PSB file with two or more layers or with slices, or
//! placing one without Link, asks whether its layers become objects or one flat image, whether
//! hidden layers come in, and whether its slices come in (Import Slices, offered only for a file
//! with slices)
//! (<https://helpx.adobe.com/illustrator/desktop/add-and-import-files/import-from-other-apps/photoshop-import-options.html>).
//! A linked file shows its flat image, so placing with Link doesn't ask. The two-layer threshold,
//! the starting choices, Import Slices applying to both conversions and remembering the choices
//! are VectorCraft's own.
//!
//! Fields: `mode` (`open` | `place`), `name`, `path?`, `width`, `height` (the canvas, px),
//! `layers`, `hidden` and `slices` (counts, from `document.psdInfo`), `convert` (`objects` |
//! `flatten`), `hiddenLayers` and `importSlices`. `__place` keeps the other `file.place` params.
//! The file's bytes stay in [`crate::state::UiState::dialog_file`]. OK opens or places the file
//! with `layers`, `hiddenLayers` and (for a file with slices) `importSlices`, and remembers the
//! choices for the next file; errors keep the dialog open.

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
/// are a PSD or PSB file with [`MIN_LAYERS`] layers or more, or with slices; false when they aren't
/// (the loader then opens or places the file, or says why it can't).
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    let Ok(info) = fileio::psd_info(bytes) else { return false };
    if info.layers < MIN_LAYERS && info.slices == 0 {
        return false;
    }
    app.ui.dialog_file = Some(DialogFile::new(bytes.to_vec()));
    let (layers, hidden, slices) = last_choices(app);
    let mut fields = json!({
        "mode": if place.is_some() { "place" } else { "open" },
        "name": name,
        "width": info.width,
        "height": info.height,
        "layers": info.layers,
        "hidden": info.hidden,
        "slices": info.slices,
        "convert": if layers { "objects" } else { "flatten" },
        "hiddenLayers": hidden,
        "importSlices": slices,
    });
    if let Some(p) = path {
        fields["path"] = json!(p);
    }
    if let Some(p) = place {
        fields["__place"] = other_place_params(p, &["layers", "hiddenLayers", "importSlices"]);
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

/// The PSD Import Options chosen last (layers, hidden layers, slices), at first Convert Layers to
/// Objects without hidden layers, with Import Slices.
fn last_choices(app: &VectorcraftApp) -> (bool, bool, bool) {
    let last = |k: &str, default: bool| app.ui.psd_import.get(k).and_then(Value::as_bool).unwrap_or(default);
    (last("layers", true), last("hiddenLayers", false), last("importSlices", true))
}

/// Several files placed without Link load the place cursor without asking: the `file.place.queue`
/// params `q` take the PSD Import Options chosen last, which apply to each PSD or PSB file among them.
pub fn queue_options(app: &VectorcraftApp, q: &mut Value) {
    let (layers, hidden, slices) = last_choices(app);
    q["layers"] = json!(layers);
    if layers && hidden {
        q["hiddenLayers"] = json!(true);
    }
    q["importSlices"] = json!(slices);
}

/// `file.place` params `p` placed without asking (Show Import Options off) that embed a PSD or PSB
/// file and give no `layers` take the PSD Import Options chosen last; `hiddenLayers` and
/// `importSlices` that the params give are kept.
pub fn last_options(app: &VectorcraftApp, p: &mut Value) {
    let embedded = p.get("path").is_none() || p.get("link").and_then(Value::as_bool) == Some(false);
    let name = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).unwrap_or_default();
    if embedded && p.get("layers").is_none() && matches!(fileio::extension(name).as_str(), "psd" | "pdd" | "psb") {
        let (layers, hidden, slices) = last_choices(app);
        p["layers"] = json!(layers);
        if layers && hidden && p.get("hiddenLayers").is_none() {
            p["hiddenLayers"] = json!(true);
        }
        if p.get("importSlices").is_none() {
            p["importSlices"] = json!(slices);
        }
        if p.get("path").is_none() {
            // A file from bytes has no file to link to; it may carry the Place dialog's Link.
            p["link"] = json!(false);
        }
    }
}

/// OK: open or place the file with the choices.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let file = app.ui.dialog_file.clone().ok_or("the file is no longer loaded")?;
    let layers = d.str("convert") != "flatten";
    let hidden = layers && d.bool("hiddenLayers");
    // Import Slices is a choice only for a file with slices; otherwise the one made last stays.
    let has_slices = d.fields.get("slices").and_then(Value::as_u64).unwrap_or(0) > 0;
    let slices = if has_slices { d.bool("importSlices") } else { last_choices(app).2 };
    app.ui.psd_import = json!({ "layers": layers, "hiddenLayers": d.bool("hiddenLayers"), "importSlices": slices });
    let r = if d.str("mode") == "place" {
        let mut p = with_file(d.fields.get("__place"), d, &file);
        p["layers"] = json!(layers);
        if layers || has_slices {
            // Layers and slices are always embedded; a file from bytes may carry the Place dialog's
            // Link.
            p["link"] = json!(false);
        }
        if hidden {
            p["hiddenLayers"] = json!(true);
        }
        if has_slices {
            p["importSlices"] = json!(slices);
        }
        crate::place::run_for_user(app, &p).map(|_| ())
    } else {
        let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
        let mut p = json!({ "layers": layers, "hiddenLayers": hidden });
        if has_slices {
            p["importSlices"] = json!(slices);
        }
        io::open_document(app, &d.str("name"), &file.bytes, path, &p).map(|_| ())
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
    // Offered only for a file with slices, with either conversion.
    let sliced = count("slices") > 0;
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
    ui.add_space(8.0);
    let on = d.bool("importSlices");
    if widgets::check(ui, tl!("Import Slices"), on && sliced, sliced) {
        d.fields.insert("importSlices".into(), json!(!on));
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

    /// The OK of the dialog a drop on a canvas opened places in the document the file was dropped
    /// on, made active again, even when another document became active meanwhile; it doesn't
    /// place when that document has been reverted or closed since.
    #[test]
    fn a_dropped_files_ok_places_in_the_document_it_was_dropped_on() {
        use crate::place::{DropAt, DropTarget};
        let psd = layered();
        let mut app = app();
        for _ in 0..2 {
            app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        }
        let drop_on_first = |app: &mut VectorcraftApp| {
            app.session.set_active(0);
            let st = app.session.doc().unwrap();
            let target = DropTarget::Place(DropAt { doc: st.uid, reverts: st.reverts, at: vectorcraft_geom::Point::new(50.0, 50.0), embed: true });
            crate::place::drop_files(app, vec![(target, ("poster.psd".into(), None, psd.clone()))]);
            assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
            // Another document becomes active while the dialog is open.
            app.session.set_active(1);
        };
        let objects =
            |app: &VectorcraftApp, i: usize| app.session.documents()[i].doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum::<usize>();
        drop_on_first(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!((objects(&app, 0), objects(&app, 1)), (1, 0), "placed in the document it was dropped on");
        assert_eq!(app.session.active_index(), Some(0), "which is active again");
        // Reverted meanwhile: the dialog stays open with the reason, and nothing is placed.
        drop_on_first(&mut app);
        let first = app.session.documents()[0].uid;
        app.session.document_mut(first).unwrap().reverts += 1;
        let e = super::super::confirm(&mut app).unwrap_err();
        assert!(e.contains("was reverted"), "{e}");
        assert_eq!((objects(&app, 0), objects(&app, 1)), (1, 0));
        // Closed meanwhile.
        app.ui.dialog = None;
        drop_on_first(&mut app);
        app.session.close_document(0);
        let e = super::super::confirm(&mut app).unwrap_err();
        assert!(e.contains("was closed"), "{e}");
        assert_eq!(objects(&app, 0), 0);
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

    /// `psd` with a slices resource (1050, version 6) holding one user slice over its top-left
    /// 10 × 10 px, named "cell".
    fn sliced(psd: &[u8]) -> Vec<u8> {
        let ustr = |s: &str| -> Vec<u8> {
            let units: Vec<u16> = s.encode_utf16().collect();
            let mut b = (units.len() as u32).to_be_bytes().to_vec();
            b.extend(units.iter().flat_map(|u| u.to_be_bytes()));
            b
        };
        let mut data = 6u32.to_be_bytes().to_vec();
        data.extend([0u8; 16]);
        data.extend(ustr(""));
        data.extend(1u32.to_be_bytes());
        // ID, group ID, origin (made by the user), name, type (Image), left, top, right, bottom.
        data.extend([1u32, 0, 2].iter().flat_map(|v| v.to_be_bytes()));
        data.extend(ustr("cell"));
        data.extend([1u32, 0, 0, 10, 10].iter().flat_map(|v| v.to_be_bytes()));
        // URL, target, message, alt text, cell text is HTML, cell text, alignments, color.
        for _ in 0..4 {
            data.extend(ustr(""));
        }
        data.push(0);
        data.extend(ustr(""));
        data.extend([0u8; 12]);
        let u32_at = |at: usize| u32::from_be_bytes(psd[at..at + 4].try_into().unwrap()) as usize;
        let colors = 26 + 4 + u32_at(26);
        let len = u32_at(colors);
        let mut block = b"8BIM".to_vec();
        block.extend(1050u16.to_be_bytes());
        block.extend([0, 0]);
        block.extend((data.len() as u32).to_be_bytes());
        block.extend(&data);
        block.resize(block.len().next_multiple_of(2), 0);
        let mut out = psd[..colors].to_vec();
        out.extend(((len + block.len()) as u32).to_be_bytes());
        out.extend(&psd[colors + 4..colors + 4 + len]);
        out.extend(block);
        out.extend(&psd[colors + 4 + len..]);
        out
    }

    fn slice_names(app: &VectorcraftApp) -> Vec<String> {
        app.session.doc().unwrap().doc.slices.iter().map(|s| s.options.name.clone()).collect()
    }

    #[test]
    fn a_file_with_slices_asks_with_import_slices_on_either_conversion() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        let r = s.execute("document.export", &json!({"format": "psd"})).unwrap();
        // One layer: without slices it opens at once, with them it asks.
        let psd = sliced(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
        let mut app = app();
        io::open_bytes(&mut app, "one.psd", &psd, None).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), &d.fields["slices"], d.bool("importSlices")), (KIND, &json!(1), true));
        assert!(frame(&mut app).contains("Import Slices"));
        // Flatten keeps Import Slices: the merged image opens with the slice.
        set(&mut app, "convert", json!("flatten"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(slice_names(&app), ["cell"]);
        // Left off, the slice stays out, and the next file starts from that choice.
        io::open_bytes(&mut app, "one.psd", &psd, None).unwrap();
        set(&mut app, "importSlices", json!(false));
        super::super::confirm(&mut app).unwrap();
        assert!(slice_names(&app).is_empty());
        io::open_bytes(&mut app, "one.psd", &psd, None).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("importSlices"), "remembered");
        // A layered file without slices keeps the choice made last for the next sliced one.
        app.ui.dialog = None;
        io::open_bytes(&mut app, "poster.psd", &layered(), None).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["slices"], json!(0));
        set(&mut app, "importSlices", json!(true));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.ui.psd_import["importSlices"], json!(false));
    }

    #[test]
    fn placing_a_sliced_file_without_link_brings_its_slices_with_the_art() {
        let b64 = vectorcraft_format::base64_encode(&sliced(&layered()));
        let mut app = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64, "at": [50, 50]})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("importSlices"));
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.slices.len(), 1);
        assert_eq!(st.doc.slices[0].rect, vectorcraft_geom::Rect::new(40.0, 40.0, 50.0, 50.0));
        // Several files placed together take the choice made last.
        let mut q = json!({});
        app.ui.psd_import = json!({"layers": true, "importSlices": false});
        queue_options(&app, &mut q);
        assert_eq!(q["importSlices"], json!(false));
    }

    #[test]
    fn placing_with_show_import_options_off_takes_the_last_choices() {
        let b64 = vectorcraft_format::base64_encode(&sliced(&layered()));
        let mut app = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // The file's bytes carry the Place dialog's Link, which they have no file for.
        let place = |app: &mut VectorcraftApp, slices: Option<bool>| {
            let mut p = json!({"name": "poster.psd", "dataBase64": b64, "link": true, "showImportOptions": false});
            if let Some(s) = slices {
                p["importSlices"] = json!(s);
            }
            crate::place::run_for_user(app, &p).unwrap();
            assert!(app.ui.dialog.is_none(), "placed without asking");
            let st = app.session.doc().unwrap();
            let [id] = st.selection.objects[..] else { panic!("one object is selected") };
            (st.doc.node(id).unwrap().kind.clone(), st.doc.slices.len())
        };
        // At first, Convert Layers to Objects with Import Slices.
        let (kind, slices) = place(&mut app, None);
        assert!(matches!(kind, NodeKind::Group { .. }));
        assert_eq!(slices, 1);
        // An `importSlices` the params give is kept.
        let (kind, slices) = place(&mut app, Some(false));
        assert!(matches!(kind, NodeKind::Group { .. }));
        assert_eq!(slices, 1, "no new slice");
        // Then the choices made last.
        app.ui.psd_import = json!({"layers": false, "importSlices": false});
        let (kind, slices) = place(&mut app, None);
        assert!(matches!(kind, NodeKind::Image(_)));
        assert_eq!(slices, 1, "no new slice");
        let (_, slices) = place(&mut app, Some(true));
        assert_eq!(slices, 2, "the given importSlices brings the slice in");
    }

    #[test]
    fn files_from_bytes_placed_with_link_take_the_last_choices() {
        let files: Vec<Value> = ["a.psd", "b.psd"].iter().map(|n| json!({ "name": n })).collect();
        let mut app = app();
        for n in ["a.psd", "b.psd"] {
            app.place.picked.push((n.to_string(), sliced(&layered())));
        }
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // The dialog's Link is on, but files from bytes have no file to link to.
        let d = Dialog::new(crate::dialogs::place::KIND, json!({"files": files, "link": true, "template": false, "showImportOptions": false}));
        crate::place::confirm(&mut app, &d).unwrap();
        assert!(app.ui.dialog.is_none(), "the cursor loads without asking");
        let view = vectorcraft_engine::ViewInfo::default();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            for r in app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 10.0, 10.0), view).unwrap() {
                if let vectorcraft_engine::UiRequest::Place(p) = r {
                    crate::place::place_from_cursor(&mut app, &p);
                }
            }
        }
        let st = app.session.doc().unwrap();
        let [id] = st.selection.objects[..] else { panic!("one object is placed") };
        // At first, Convert Layers to Objects with Import Slices.
        assert!(matches!(st.doc.node(id).unwrap().kind, NodeKind::Group { .. }));
        assert_eq!(st.doc.slices.len(), 1);
    }

    #[test]
    fn a_cursor_loaded_with_show_import_options_on_asks_for_each_psd_file() {
        let dir = std::env::temp_dir().join(format!("vc-psd-import-asks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<String> = ["a.psd", "b.psd"]
            .iter()
            .map(|n| {
                let p = dir.join(n);
                std::fs::write(&p, layered()).unwrap();
                p.to_string_lossy().into_owned()
            })
            .collect();
        let files: Vec<Value> = paths.iter().map(|p| json!({ "path": p })).collect();
        let services = crate::Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // The choices made last would place the layers without asking with Show Import Options off.
        app.ui.psd_import = json!({"layers": true, "importSlices": true});
        let d = Dialog::new(crate::dialogs::place::KIND, json!({"files": files, "link": false, "template": false, "showImportOptions": true}));
        crate::place::confirm(&mut app, &d).unwrap();
        assert!(app.ui.dialog.is_none(), "the cursor loads without asking");
        assert_eq!(app.session.tool_options()["count"], 2);
        // A click on the canvas: the file the cursor's request names asks.
        let view = vectorcraft_engine::ViewInfo::default();
        for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
            for r in app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 10.0, 10.0), view).unwrap() {
                if let vectorcraft_engine::UiRequest::Place(p) = r {
                    crate::place::place_from_cursor(&mut app, &p);
                }
            }
        }
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "the clicked file asks");
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
