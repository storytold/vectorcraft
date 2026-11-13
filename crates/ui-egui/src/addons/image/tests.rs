use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;

use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// An app with one image open and selected: 12 × 10 pixels, white, a near-white ring one pixel in
/// (13.7 % from white) and a red block inside it.
fn app() -> VectorcraftApp {
    let img = image::RgbaImage::from_fn(12, 10, |x, y| {
        image::Rgba(if (3..9).contains(&x) && (3..7).contains(&y) {
            [200, 0, 0, 255]
        } else if (1..11).contains(&x) && (1..9).contains(&y) && (x == 1 || x == 10 || y == 1 || y == 8) {
            [220, 220, 220, 255]
        } else {
            [255; 4]
        })
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("document.open", json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app
}

/// The selected image's pixel key and bytes.
fn image(app: &VectorcraftApp) -> (String, Arc<Vec<u8>>) {
    let st = app.session.doc().unwrap();
    let Some(NodeKind::Image(im)) = st.selection.objects.first().and_then(|id| st.doc.node(*id)).map(|n| &n.kind) else { panic!("an image") };
    (im.key.clone(), st.doc.images[&im.key].bytes.clone())
}

/// The alpha of the selected image's pixels at the corner, on the ring and in the block.
fn alphas(app: &VectorcraftApp) -> [u8; 3] {
    let px = image::load_from_memory(&image(app).1).unwrap().to_rgba8();
    [(0, 0), (1, 1), (4, 4)].map(|(x, y)| px.get_pixel(x, y).0[3])
}

fn open(app: &mut VectorcraftApp) {
    click(app, "Remove Background…");
}

/// Choose Object › Plug-ins › Image Tools › `label`, which must be enabled.
fn click(app: &mut VectorcraftApp, label: &str) {
    let item = menu_item(app, label);
    assert!(item.enabled, "{label} enabled");
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &id, p);
}

fn menu_item(app: &VectorcraftApp, label: &str) -> menus::MenuEntry {
    menus::menu_entries(app).into_iter().find(|e| e.path == ["Object", "Plug-ins", "Image Tools"] && e.label == label).unwrap()
}

/// The selected image's color mask, if it has one.
fn mask(app: &VectorcraftApp) -> Option<vectorcraft_doc::color_mask::ColorMask> {
    let st = app.session.doc().unwrap();
    st.selection.objects.first().and_then(|id| st.doc.node(*id)).and_then(vectorcraft_doc::Document::color_mask_of).cloned()
}

/// The alpha of the selected image's masked pixels at the corner, on the ring and in the block
/// (all 255 without a mask).
fn masked_alphas(app: &VectorcraftApp) -> [u8; 3] {
    let Some(m) = mask(app) else { return [255; 3] };
    let px = image::load_from_memory(&app.session.doc().unwrap().doc.images[&m.key].bytes).unwrap().to_rgba8();
    [(0, 0), (1, 1), (4, 4)].map(|(x, y)| px.get_pixel(x, y).0[3])
}

/// Object › Plug-ins › Image Tools › Color Mask… opens the command dialog on the defaults, previews
/// a mask while the image's pixels stay, and keeps it on OK as one step; opened again it shows the
/// image's mask settings. Remove Color Mask and Apply Color Mask are enabled only for an image
/// with a mask.
#[test]
fn color_mask_previews_keeps_its_settings_and_comes_off() {
    let mut app = app();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let (start, n) = (image(&app), steps(&app));
    assert!(!menu_item(&app, "Remove Color Mask").enabled && !menu_item(&app, "Apply Color Mask").enabled);

    click(&mut app, "Color Mask…");
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), (super::COLOR_MASK.to_string(), "Color Mask".to_string()));
    assert_eq!((d.str("background"), d.f64("tolerance", -1.0), d.bool("preview")), ("edges".to_string(), 10.0, false));
    assert!(!d.fields.contains_key("masked"));
    let shown = show(&mut app);
    for s in ["Color Mask", "Edge Color", "Contiguous", "Match", "Tolerance", "Softness", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    set(&mut app, "preview", json!(true));
    set(&mut app, "tolerance", json!(20));
    show(&mut app);
    assert_eq!(masked_alphas(&app), [0, 0, 255], "the preview hides the white and the ring");
    assert_eq!(image(&app), start, "the image's pixels stay");
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((steps(&app), image(&app)), (n + 1, start.clone()), "one undo step; the pixels as they were");
    assert_eq!(mask(&app).map(|m| m.tolerance), Some(20.0));

    // Opened again: the image's mask settings.
    click(&mut app, "Color Mask…");
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("tolerance", -1.0), 20.0);
    crate::dialogs::cancel(&mut app);
    assert_eq!(steps(&app), n + 1);

    assert!(menu_item(&app, "Apply Color Mask").enabled);
    click(&mut app, "Remove Color Mask");
    assert_eq!((mask(&app), image(&app), steps(&app)), (None, start, n + 2));
}

fn show(app: &mut VectorcraftApp) -> String {
    painted_text(app, |app, ui| crate::dialogs::show(app, ui.ctx()))
}

fn set(app: &mut VectorcraftApp, key: &str, v: serde_json::Value) {
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), key.into(), v);
}

/// Properties' Quick Actions and the Control bar offer Remove Background for a selected image, and
/// the Control bar's button removes it at once (no dialog) with the defaults, as one step.
#[test]
fn properties_and_the_control_bar_remove_the_background_at_once() {
    use crate::tests_removeanchors::{at, click_control, control_frame, has, properties_frame};
    let mut app = app();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let n = steps(&app);
    assert!(has(&properties_frame(&mut app, 280.0), "Remove Background"), "Properties offers it");
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let bar = control_frame(&mut app, &ctx, vec![]);
    click_control(&mut app, &ctx, at(&bar, "Remove Background"));
    assert!(app.ui.dialog.is_none(), "it runs at once");
    assert_eq!(steps(&app), n + 1, "one undo step");
    // The defaults (tolerance and softness 10): the white goes, the near-white ring fades.
    let [corner, ring, block] = alphas(&app);
    assert!(corner == 0 && ring > 0 && ring < 255 && block == 255, "{:?}", alphas(&app));
}

/// Object › Plug-ins › Image Tools › Remove Background… opens the command dialog on Edge Color
/// with Preview off. With Preview on the canvas shows the result for the values in the dialog,
/// each change worked out again from the image as it was; Cancel puts the image back with no step
/// of history, and OK keeps the result for the values in the dialog as one step.
#[test]
fn remove_background_previews_cancels_and_runs_on_ok() {
    let mut app = app();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let (start, n) = (image(&app), steps(&app));

    open(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), (super::REMOVE_BACKGROUND.to_string(), "Remove Background".to_string()));
    assert_eq!((d.str("background"), d.bool("preview")), ("edges".to_string(), false));
    assert!(crate::dialogs::accepts_field(d, "preview"));
    assert_eq!((d.bool("contiguous"), d.str("match")), (true, "distance".to_string()));
    let shown = show(&mut app);
    for s in ["Remove Background", "Edge Color", "Contiguous", "Match", "Distance", "Tolerance", "Softness", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    assert_eq!(image(&app), start, "no preview without the checkbox");

    // Preview on: at tolerance 10 the ring fades; at 20 it is cleared; at 0, with no softness,
    // only the white is cleared and the ring is whole again.
    set(&mut app, "preview", json!(true));
    show(&mut app);
    let [corner, ring, block] = alphas(&app);
    assert!(corner == 0 && ring > 0 && ring < 255 && block == 255, "{:?}", alphas(&app));
    set(&mut app, "tolerance", json!(20));
    show(&mut app);
    assert_eq!(alphas(&app), [0, 0, 255]);
    set(&mut app, "tolerance", json!(0));
    set(&mut app, "softness", json!(0));
    show(&mut app);
    assert_eq!(alphas(&app), [0, 255, 255], "worked out again from the image as it was");

    // Cancel: as it was, with no step of history.
    crate::dialogs::cancel(&mut app);
    assert_eq!((image(&app), steps(&app)), (start.clone(), n), "Cancel restores the image and adds no undo step");
    assert!(!app.session.in_interaction());

    // OK: the values in the dialog, as one step.
    open(&mut app);
    set(&mut app, "preview", json!(true));
    set(&mut app, "tolerance", json!(20));
    show(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(steps(&app), n + 1, "one undo step");
    assert_eq!(alphas(&app), [0, 0, 255]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(image(&app), start);
}
