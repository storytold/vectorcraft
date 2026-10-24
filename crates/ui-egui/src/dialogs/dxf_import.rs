//! DXF Import Options: opening or placing a DXF drawing asks which layout to import (model space
//! or a paper layout, previewed), its scale (fitted to the artboard, or 1 unit of the art = N
//! drawing units: by default the drawing at 1:1 in its own unit), whether lineweights scale with
//! it, whether it is centred, and whether its layers merge into one.
//!
//! Fields: `mode` (`open` | `place`), `name`, `path?`, `layouts` (model space first), `units` and
//! `version` (what the drawing declares), and the `dxf` options of `document.open`: `layout`,
//! `fit`, `unit` (a unit name), `scale`, `scaleLineweights`, `center`, `mergeLayers`. `__place`
//! keeps the other `file.place` params. The file's bytes stay in
//! [`crate::state::UiState::dialog_file`]. OK opens or places the drawing and remembers the fit,
//! lineweight, centring and layer choices for the next one; errors keep the dialog open.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use super::import_pdf::{DialogFile, other_place_params, place_source, preview_texture, show_preview, with_file};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub const KIND: &str = "dxfImport";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str("mode") == "place" { tl!("Place DXF") } else { tl!("DXF Import Options") }.into(),
    body,
    confirm,
    min_width: 560.0,
    ..DialogSpec::FORM
};

/// The longest side of the layout preview (pt).
const PREVIEW: f32 = 170.0;
/// The choices the next drawing starts from (the scale belongs to each drawing's units).
const REMEMBERED: [&str; 4] = ["fit", "scaleLineweights", "center", "mergeLayers"];

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when they
/// are a DXF drawing; false when they aren't (or can't be read: the loader then says why).
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    if fileio::detect(name, bytes).is_none_or(|f| f.id != "dxf") {
        return false;
    }
    let Ok(info) = vectorcraft_cad::info(bytes) else { return false };
    app.ui.dialog_file = Some(DialogFile::new(bytes.to_vec()));
    let mut fields = json!({
        "mode": if place.is_some() { "place" } else { "open" },
        "name": name,
        "layouts": info.layouts,
        "layout": info.layouts.first().map_or("Model", String::as_str),
        "units": info.units,
        "version": info.version,
        "fit": false,
        "unit": info.unit.label(),
        "scale": info.scale,
        "scaleLineweights": false,
        "center": true,
        "mergeLayers": false,
    });
    if let Some(last) = app.ui.dxf_import.as_object() {
        for k in REMEMBERED {
            if let Some(v) = last.get(k).filter(|v| v.is_boolean()) {
                fields[k] = v.clone();
            }
        }
    }
    if let Some(p) = path {
        fields["path"] = json!(p);
    }
    if let Some(p) = place {
        fields["__place"] = other_place_params(p, &["dxf"]);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    true
}

/// `file.place` with params `p` asks first (true) when they name a `.dxf` drawing and no `dxf`
/// options.
pub fn offer_place(app: &mut VectorcraftApp, p: &Value) -> bool {
    let named = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::extension);
    if p.get("dxf").is_some() || named.as_deref() != Some("dxf") {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| ext == "dxf") else { return false };
    offer(app, &name, &bytes, path, Some(p))
}

/// The `dxf` options of `document.open` and `file.place` the fields set.
fn options(d: &Dialog) -> Value {
    let fit = d.bool("fit");
    let mut o = json!({
        "layout": d.str("layout"),
        "fit": fit,
        "scaleLineweights": d.bool("scaleLineweights"),
        "center": d.bool("center"),
        "mergeLayers": d.bool("mergeLayers"),
    });
    if !fit {
        o["unit"] = json!(d.str("unit"));
        o["scale"] = json!(d.f64("scale", 1.0));
    }
    o
}

/// OK: open or place the drawing with the options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let file = app.ui.dialog_file.clone().ok_or("the drawing is no longer loaded")?;
    let dxf = options(d);
    app.ui.dxf_import = Value::Object(REMEMBERED.iter().map(|k| (k.to_string(), dxf[*k].clone())).collect());
    let r = if d.str("mode") == "place" {
        let mut p = with_file(d.fields.get("__place"), d, &file);
        p["dxf"] = dxf;
        crate::place::run_for_user(app, &p).map(|_| ())
    } else {
        let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
        io::open_document(app, &d.str("name"), &file.bytes, path, &json!({ "dxf": dxf })).map(|_| ())
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

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            show_preview(ui, preview(app, ui.ctx(), d).as_ref(), PREVIEW);
            ui.add_space(6.0);
            widgets::dim_label(ui, &format!("DXF {} · {}", d.str("version"), d.str("units")));
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            let layouts: Vec<String> =
                d.fields.get("layouts").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            let rows: Vec<(&str, &str)> = layouts.iter().map(|l| (l.as_str(), l.as_str())).collect();
            choice(ui, d, "layout", tl!("Layout:"), &rows);
            ui.add_space(10.0);
            widgets::subheader(ui, tl!("Scaling"));
            check(ui, d, "fit", tl!("Scale to Fit Artboard"));
            ui.add_enabled_ui(!d.bool("fit"), |ui| super::dxf_options::scale_row(ui, d));
            ui.horizontal(|ui| {
                ui.add_space(LABEL + 8.0);
                check(ui, d, "scaleLineweights", tl!("Scale Lineweights"));
            });
            ui.add_space(10.0);
            widgets::subheader(ui, tl!("Options"));
            check(ui, d, "center", tl!("Center Artwork"));
            check(ui, d, "mergeLayers", tl!("Merge Layers"));
        });
    });
    false
}

/// The chosen layout fitted to the preview, rendered once per drawing and layout.
fn preview(app: &mut VectorcraftApp, ctx: &egui::Context, d: &Dialog) -> Option<egui::TextureHandle> {
    let file = app.ui.dialog_file.clone()?;
    let layout = d.str("layout");
    let key = egui::Id::new(("dxf-preview", file.token, layout.as_str()));
    let o = vectorcraft_cad::ImportOptions { layout: Some(layout.clone()), fit: true, ..Default::default() };
    // A layout that can't be read shows an empty preview (and isn't read again).
    preview_texture(app, ctx, key, "dxf-preview", PREVIEW, || vectorcraft_cad::import(&file.bytes, &o).ok().map(|r| r.document))
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    use super::*;

    /// A millimetre drawing: a line in model space and a circle on a paper layout.
    fn drawing() -> Vec<u8> {
        let pairs: &[(i32, &str)] = &[
            (0, "SECTION"),
            (2, "HEADER"),
            (9, "$INSUNITS"),
            (70, "4"),
            (0, "ENDSEC"),
            (0, "SECTION"),
            (2, "ENTITIES"),
            (0, "LINE"),
            (8, "Walls"),
            (10, "0"),
            (20, "0"),
            (11, "100"),
            (21, "50"),
            (0, "CIRCLE"),
            (67, "1"),
            (10, "0"),
            (20, "0"),
            (40, "5"),
            (0, "ENDSEC"),
            (0, "EOF"),
        ];
        pairs.iter().map(|(c, v)| format!("{c:>3}\n{v}\n")).collect::<String>().into_bytes()
    }

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
    }

    #[test]
    fn opening_a_drawing_asks_for_the_import_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        io::open_bytes(&mut app, "plan.dxf", &drawing(), None).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("mode"), d.str("layout"), d.str("unit")), (KIND, "open".into(), "Model".into(), "Millimeters".into()));
        assert_eq!(d.fields["layouts"], json!(["Model", "Layout1"]));
        let text = frame(&mut app);
        for label in [
            "DXF Import Options",
            "Layout:",
            "Scaling",
            "Scale to Fit Artboard",
            "Scale:",
            "Scale Lineweights",
            "Center Artwork",
            "Merge Layers",
            "Millimeters",
            "OK",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // Fit greys out the ratio; a paper layout previews and opens.
        set(&mut app, "fit", json!(true));
        set(&mut app, "layout", json!("Layout1"));
        set(&mut app, "mergeLayers", json!(true));
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        let mut circles = 0;
        st.doc.walk(|n| circles += usize::from(matches!(n.kind, NodeKind::Path { .. })));
        assert_eq!(circles, 1, "the paper layout's circle");
        assert_eq!(st.doc.layers.len(), 1);
        // The next drawing starts from these choices, its own scale.
        io::open_bytes(&mut app, "plan.dxf", &drawing(), None).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("fit") && d.bool("mergeLayers"));
        assert_eq!(d.str("layout"), "Model");
    }

    #[test]
    fn bad_options_keep_the_dialog_open() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        io::open_bytes(&mut app, "plan.dxf", &drawing(), None).unwrap();
        set(&mut app, "layout", json!("Sheet 9"));
        assert!(super::super::confirm(&mut app).is_err());
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        set(&mut app, "layout", json!("Model"));
        set(&mut app, "unit", json!("Points"));
        set(&mut app, "scale", json!(1));
        super::super::confirm(&mut app).unwrap();
        let w = app.session.doc().unwrap().doc.artboards[0].rect.width();
        assert_eq!(w, 100.0, "1 pt = 1 unit");
    }

    #[test]
    fn placing_a_drawing_asks_then_places_one_group() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
        let p = json!({"name": "plan.dxf", "dataBase64": vectorcraft_format::base64_encode(&drawing()), "template": false});
        crate::place::run(&mut app, &p).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("mode")), (KIND, "place".into()));
        assert_eq!(d.fields["__place"], json!({"template": false}));
        assert!(frame(&mut app).contains("Place DXF"));
        set(&mut app, "fit", json!(true));
        let undo = app.session.doc().unwrap().history.undo.len();
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let id = st.selection.objects[0];
        let b = st.doc.node(id).unwrap().geometric_bounds().unwrap();
        assert!((b.width() - 600.0).abs() < 1.0, "fitted to the artboard: {b:?}");
        // Params with dxf options place without asking.
        let mut p = p.clone();
        p["dxf"] = json!({});
        crate::place::run(&mut app, &p).unwrap();
        assert!(app.ui.dialog.is_none());
    }
}
