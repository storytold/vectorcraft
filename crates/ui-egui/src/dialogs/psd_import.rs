//! PSD Import Options: opening a PSD or PSB file with two or more layers or with slices, or
//! placing one without Link, asks whether its layers become objects or one flat image, whether
//! hidden layers come in, and whether its slices come in (Import Slices, offered only for a file
//! with slices)
//! (<https://helpx.adobe.com/illustrator/desktop/add-and-import-files/import-from-other-apps/photoshop-import-options.html>).
//! A linked file shows its flat image, so placing with Link doesn't ask. The two-layer threshold,
//! the starting choices, Import Slices applying to both conversions and remembering the choices
//! are VectorCraft's own.
//!
//! Fields: `convert` (`objects` | `flatten`), `hiddenLayers` and `importSlices`, and `hasSlices`,
//! which no row shows and which enables Import Slices. The file, its name and path and the other
//! `file.place` params stay with the dialog's OK, which opens or places the file with `layers`,
//! `hiddenLayers` and (for a file with slices) `importSlices`, and remembers the choices for the
//! next file; errors keep the dialog open.
//!
//! PLAN: no `pump`: `open_held`'s `ok` closure runs the import, so fl.A9's `psd_import::pump`
//! line has no work to call.

use std::sync::Arc;

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use super::host::open_held;
use super::import_pdf::{DialogFile, other_place_params, place_source, with_file};
use crate::{VectorcraftApp, io};

pub const KIND: &str = "psdImport";

/// The fewest layers a file has for the dialog to ask.
const MIN_LAYERS: usize = 2;

/// What the dialog's OK needs besides its values.
struct Source {
    /// Whether the file is placed (the `file.place` params in `place`) rather than opened.
    place: Option<Value>,
    name: String,
    path: Option<String>,
    file: Arc<DialogFile>,
    has_slices: bool,
}

/// The rows of the dialog, over the `summary` line of the file's size and layer count.
fn params(summary: &str) -> Vec<Param> {
    let option = |id: &str, label: &'static str| (id.to_string(), label.into());
    vec![
        Param::read_only_text("summary", "", summary.to_string()).full_width(),
        Param::new(
            "convert",
            tl!("Layers"),
            ParamKind::radio(
                vec![option("objects", tl!("Convert Layers to Objects")), option("flatten", tl!("Flatten Layers to a Single Image"))],
                Some("objects".into()),
            ),
        ),
        Param::new("hiddenLayers", tl!("Import Hidden Layers"), ParamKind::bool(false)).enabled_when("convert", "objects"),
        // Offered only for a file with slices, with either conversion.
        Param::new("importSlices", tl!("Import Slices"), ParamKind::bool(true)).enabled_when("hasSlices", true),
    ]
}

/// The dialog over its values.
fn build<'b>(fields: &'b mut DialogFields, placing: bool, summary: &str) -> ParamDialog<'b> {
    let title = if placing { tl!("Place PSD") } else { tl!("PSD Import Options") };
    ParamDialog::new(KIND, title, params(summary)).values(Bind::bound(fields))
}

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when they
/// are a PSD or PSB file with [`MIN_LAYERS`] layers or more, or with slices; false when they aren't
/// (the loader then opens or places the file, or says why it can't).
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    let Ok(info) = fileio::psd_info(bytes) else { return false };
    if info.layers < MIN_LAYERS && info.slices == 0 {
        return false;
    }
    let (layers, hidden, slices) = last_choices(app);
    let has_slices = info.slices > 0;
    let mut fields = DialogFields::new();
    fields.set("convert", FieldValue::Text(if layers { "objects" } else { "flatten" }.into()));
    fields.set("hiddenLayers", FieldValue::Bool(hidden));
    fields.set("importSlices", FieldValue::Bool(slices));
    fields.set("hasSlices", FieldValue::Bool(has_slices));
    let summary = format!("{} × {} px · {} {}", info.width, info.height, crate::i18n::colon(tl!("Layers")), info.layers);
    let placing = place.is_some();
    let drawing = place.and_then(|params| crate::place::import_options_drawing(app, params));
    let source = Source {
        place: place.map(|p| other_place_params(p, &["layers", "hiddenLayers", "importSlices"])),
        name: name.to_string(),
        path,
        file: DialogFile::new(bytes.to_vec()),
        has_slices,
    };
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| build(fields, placing, &summary)).held_ok();
    open_held(app, dialog, move |app, result| {
        crate::place::check_import_options_drawing(app, drawing)?;
        confirm(app, &source, result)
    })
    .is_some()
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

/// OK: open or place the file with the choices; an error keeps the dialog open.
fn confirm(app: &mut VectorcraftApp, source: &Source, result: &ParamResult) -> Result<(), String> {
    let fields = &result.fields;
    let layers = fields.text("convert") != Some("flatten");
    let import_hidden = fields.bool("hiddenLayers").unwrap_or(false);
    let hidden = layers && import_hidden;
    // Import Slices is a choice only for a file with slices; otherwise the one made last stays.
    let has_slices = source.has_slices;
    let slices = if has_slices { fields.bool("importSlices").unwrap_or(true) } else { last_choices(app).2 };
    app.ui.psd_import = json!({ "layers": layers, "hiddenLayers": import_hidden, "importSlices": slices });
    match &source.place {
        Some(place) => {
            let mut p = with_file(place, source.path.as_deref(), &source.name, &source.file);
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
        }
        None => {
            let mut p = json!({ "layers": layers, "hiddenLayers": hidden });
            if has_slices {
                p["importSlices"] = json!(slices);
            }
            io::open_document(app, &source.name, &source.file.bytes, source.path.clone(), &p).map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    use super::super::text_import::test_support as host;
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

    fn set(app: &VectorcraftApp, k: &str, v: Value) {
        host::set(app, KIND, k, v);
    }

    fn layer_names(app: &VectorcraftApp) -> Vec<(String, bool)> {
        app.session.doc().unwrap().doc.layers.iter().map(|l| (l.name.clone().unwrap_or_default(), l.visible)).collect()
    }

    /// The options of a Place dialog that places without Link or with it.
    fn place_options(link: bool, show_import_options: bool) -> crate::place::PlaceOptions {
        crate::place::PlaceOptions { link, template: false, replace: false, show_import_options }
    }

    #[test]
    fn opening_a_layered_file_asks_for_the_import_options() {
        let psd = layered();
        let mut app = host::app();
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        let f = host::fields(&app, KIND);
        assert_eq!((&f["convert"], &f["hiddenLayers"], &f["hasSlices"]), (&json!("objects"), &json!(false), &json!(false)));
        assert!(app.session.active().is_none(), "nothing opens before OK");
        // Layers as objects, hidden ones left out.
        host::confirm(&mut app, KIND).unwrap();
        assert!(!host::is_open(&app, KIND));
        assert_eq!(layer_names(&app), [("Back".to_string(), true), ("Front".to_string(), true)]);
        // With Import Hidden Layers the hidden layer comes in hidden.
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        set(&app, "hiddenLayers", json!(true));
        host::confirm(&mut app, KIND).unwrap();
        assert_eq!(layer_names(&app), [("Back".to_string(), true), ("Middle".to_string(), false), ("Front".to_string(), true)]);
        // Flatten opens one image, and the next file starts from the choices made last.
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        assert_eq!(host::fields(&app, KIND)["hiddenLayers"], true, "remembered");
        set(&app, "convert", json!("flatten"));
        host::confirm(&mut app, KIND).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.layers.len(), 1);
        assert!(matches!(st.doc.layers[0].children().map(Vec::as_slice), Some([one]) if matches!(one.kind, NodeKind::Image(_))));
        io::open_bytes(&mut app, "poster.psd", &psd, None).unwrap();
        assert_eq!(host::fields(&app, KIND)["convert"], "flatten");
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
            let mut app = host::app();
            io::open_bytes(&mut app, "one.psd", &bytes, None).unwrap();
            assert!(!host::is_open(&app, KIND));
            assert!(app.session.active().is_some());
        }
    }

    #[test]
    fn normal_psd_place_options_wait_for_the_original_drawing() {
        let mut app = host::app();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let encoded = vectorcraft_format::base64_encode(&layered());
        let params = json!({"name": "poster.psd", "dataBase64": encoded, "at": [50, 50]});
        assert_eq!(crate::place::run_for_user(&mut app, &params).unwrap()["dialog"], KIND);
        set(&app, "convert", json!("flatten"));
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        let error = host::confirm(&mut app, KIND).expect_err("the other drawing must not receive the file");
        assert!(error.contains("original drawing"), "{error}");
        assert!(app.session.active().unwrap().history.undo.is_empty());
        assert_eq!(host::fields(&app, KIND)["convert"], "flatten");
        app.run("document.activate", json!({"index": 0})).unwrap();
        host::confirm(&mut app, KIND).unwrap();
        let st = app.session.active().unwrap();
        assert!(matches!(st.doc.layers[0].children().map(Vec::as_slice), Some([node]) if matches!(node.kind, NodeKind::Image(_))));
    }

    #[test]
    fn placing_without_link_asks_and_places_the_layers_as_a_group() {
        let b64 = vectorcraft_format::base64_encode(&layered());
        let mut app = host::app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let r = crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64, "at": [50, 50]})).unwrap();
        assert_eq!(r["dialog"], KIND);
        assert!(host::is_open(&app, KIND));
        host::confirm(&mut app, KIND).unwrap();
        assert!(!host::is_open(&app, KIND));
        let st = app.session.doc().unwrap();
        let [id] = st.selection.objects[..] else { panic!("one object is selected") };
        let g = st.doc.node(id).unwrap();
        assert!(matches!(g.kind, NodeKind::Group { .. }));
        let names: Vec<&str> = g.children().unwrap().iter().filter_map(|c| c.name.as_deref()).collect();
        assert_eq!(names, ["Back", "Front"]);
        // Flatten places the merged image.
        crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64})).unwrap();
        set(&app, "convert", json!("flatten"));
        host::confirm(&mut app, KIND).unwrap();
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
        let mut app = host::app();
        for _ in 0..2 {
            app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        }
        let drop_on_first = |app: &mut VectorcraftApp| {
            app.session.set_active(0);
            let st = app.session.doc().unwrap();
            let target = DropTarget::Place(DropAt { doc: st.uid, reverts: st.reverts, at: vectorcraft_geom::Point::new(50.0, 50.0), embed: true });
            crate::place::drop_files(app, vec![(target, ("poster.psd".into(), None, psd.clone()))]);
            assert!(host::is_open(app, KIND));
            // Another document becomes active while the dialog is open.
            app.session.set_active(1);
        };
        let objects =
            |app: &VectorcraftApp, i: usize| app.session.documents()[i].doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum::<usize>();
        drop_on_first(&mut app);
        host::confirm(&mut app, KIND).unwrap();
        assert!(!host::is_open(&app, KIND));
        assert_eq!((objects(&app, 0), objects(&app, 1)), (1, 0), "placed in the document it was dropped on");
        assert_eq!(app.session.active_index(), Some(0), "which is active again");
        // Reverted meanwhile: the dialog stays open with the reason, and nothing is placed.
        drop_on_first(&mut app);
        let first = app.session.documents()[0].uid;
        app.session.document_mut(first).unwrap().reverts += 1;
        assert!(host::confirm(&mut app, KIND).is_err());
        assert!(host::is_open(&app, KIND));
        assert_eq!((objects(&app, 0), objects(&app, 1)), (1, 0));
        // Closed meanwhile.
        host::cancel(&app, KIND);
        drop_on_first(&mut app);
        app.session.close_document(0);
        assert!(host::confirm(&mut app, KIND).is_err());
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
            crate::place::confirm(app, files, place_options(link, false)).unwrap();
            assert!(!host::is_open(app, KIND), "the cursor loads without asking");
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
        let mut app = host::app();
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
        let mut app = host::app();
        io::open_bytes(&mut app, "one.psd", &psd, None).unwrap();
        let f = host::fields(&app, KIND);
        assert_eq!((&f["hasSlices"], &f["importSlices"]), (&json!(true), &json!(true)));
        // Flatten keeps Import Slices: the choice is offered, and the merged image opens with the slice.
        set(&app, "convert", json!("flatten"));
        set(&app, "importSlices", json!(true));
        host::confirm(&mut app, KIND).unwrap();
        assert_eq!(slice_names(&app), ["cell"]);
        // Left off, the slice stays out, and the next file starts from that choice.
        io::open_bytes(&mut app, "one.psd", &psd, None).unwrap();
        set(&app, "importSlices", json!(false));
        host::confirm(&mut app, KIND).unwrap();
        assert!(slice_names(&app).is_empty());
        io::open_bytes(&mut app, "one.psd", &psd, None).unwrap();
        assert_eq!(host::fields(&app, KIND)["importSlices"], false, "remembered");
        // A layered file without slices takes no Import Slices choice and keeps the one made last.
        host::cancel(&app, KIND);
        io::open_bytes(&mut app, "poster.psd", &layered(), None).unwrap();
        assert_eq!(host::fields(&app, KIND)["hasSlices"], false);
        assert!(host::try_set(&app, KIND, "importSlices", json!(true)).is_err(), "the row takes no input");
        host::confirm(&mut app, KIND).unwrap();
        assert_eq!(app.ui.psd_import["importSlices"], json!(false));
    }

    #[test]
    fn placing_a_sliced_file_without_link_brings_its_slices_with_the_art() {
        let b64 = vectorcraft_format::base64_encode(&sliced(&layered()));
        let mut app = host::app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64, "at": [50, 50]})).unwrap();
        assert_eq!(host::fields(&app, KIND)["importSlices"], true);
        host::confirm(&mut app, KIND).unwrap();
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
        let mut app = host::app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // The file's bytes carry the Place dialog's Link, which they have no file for.
        let place = |app: &mut VectorcraftApp, slices: Option<bool>| {
            let mut p = json!({"name": "poster.psd", "dataBase64": b64, "link": true, "showImportOptions": false});
            if let Some(s) = slices {
                p["importSlices"] = json!(s);
            }
            crate::place::run_for_user(app, &p).unwrap();
            assert!(!host::is_open(app, KIND), "placed without asking");
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
        let mut app = host::app();
        for n in ["a.psd", "b.psd"] {
            app.place.picked.push((n.to_string(), sliced(&layered())));
        }
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // The dialog's Link is on, but files from bytes have no file to link to.
        crate::place::confirm(&mut app, files, place_options(true, false)).unwrap();
        assert!(!host::is_open(&app, KIND), "the cursor loads without asking");
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
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // The choices made last would place the layers without asking with Show Import Options off.
        app.ui.psd_import = json!({"layers": true, "importSlices": true});
        crate::place::confirm(&mut app, files, place_options(false, true)).unwrap();
        assert!(!host::is_open(&app, KIND), "the cursor loads without asking");
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
        assert!(host::is_open(&app, KIND), "the clicked file asks");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scripted_places_and_places_that_give_layers_dont_ask() {
        let b64 = vectorcraft_format::base64_encode(&layered());
        let mut app = host::app();
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        // `file.place` run as a command places at once, as before.
        let r = crate::place::run(&mut app, &json!({"name": "poster.psd", "dataBase64": b64})).unwrap();
        assert!(!host::is_open(&app, KIND));
        assert_eq!(r["format"], "psd");
        // The user's place with `layers` given doesn't ask either.
        crate::place::run_for_user(&mut app, &json!({"name": "poster.psd", "dataBase64": b64, "layers": false})).unwrap();
        assert!(!host::is_open(&app, KIND));
    }
}
