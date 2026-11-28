use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::ui::testing::Fixture;
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;

use crate::addons::dialogs::tests::ctx;
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
    {
        let __dyn: image::DynamicImage = img.into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
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
    let px = craft_image_codecs::decode(&image(app).1)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    [(0, 0), (1, 1), (4, 4)].map(|(x, y)| px.get_pixel(x, y).0[3])
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
    let px = craft_image_codecs::decode(&app.session.doc().unwrap().doc.images[&m.key].bytes)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    [(0, 0), (1, 1), (4, 4)].map(|(x, y)| px.get_pixel(x, y).0[3])
}

/// The open dialog's values as automation lists them.
fn dialog_fields(app: &VectorcraftApp) -> Value {
    app.dialogs().expect("window host").with(|host| host.open_dialogs().first().and_then(|d| d.fields.clone())).expect("a dialog opens")
}

fn set(app: &VectorcraftApp, key: &str, value: Value) {
    let ctx = ctx(app);
    app.dialogs()
        .expect("window host")
        .with(|host| {
            let id = host.open_dialogs().first().map(|d| d.id).expect("a dialog is open");
            host.set_json_field(&ctx, id, key, value)
        })
        .unwrap();
}

/// Runs the frame's pump: the open dialog's preview.
fn pump(app: &mut VectorcraftApp) {
    let ctx = ctx(app);
    crate::dialogs::host::pump(app, &ctx);
}

/// Answers the open dialog with OK or Cancel, then runs the frame's pump.
fn answer(app: &mut VectorcraftApp, ok: bool) {
    let ctx = ctx(app);
    app.dialogs().expect("window host").with(|host| {
        let id = host.open_dialogs().first().map(|d| d.id).expect("a dialog is open");
        if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) }
    });
    pump(app);
    // A held OK closes at the dialog's next pass.
    crate::tests_synthetic::key_frame(app, &ctx, vec![]);
}

fn closed(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.open_dialogs().is_empty()))
}

/// Object › Plug-ins › Image Tools › Color Mask… opens the command dialog on the defaults, previews
/// a mask while the image's pixels stay, and keeps it on OK as one step; opened again it shows the
/// image's mask settings. Remove Color Mask and Apply Color Mask are enabled only for an image
/// with a mask. Remove Background… opens on Edge Color and runs as one step.
#[test]
fn color_mask_previews_keeps_its_settings_and_comes_off() {
    let mut app = app();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let (start, n) = (image(&app), steps(&app));
    assert!(!menu_item(&app, "Remove Color Mask").enabled && !menu_item(&app, "Apply Color Mask").enabled);

    click(&mut app, "Color Mask…");
    let fields = dialog_fields(&app);
    assert_eq!((fields["background"].as_str(), fields["tolerance"].as_f64()), (Some("edges"), Some(10.0)));
    assert!(fields.get("masked").is_none());
    // The Preview box starts off (#5360).
    set(&app, "preview", json!(true));
    set(&app, "tolerance", json!(20));
    pump(&mut app);
    assert_eq!(masked_alphas(&app), [0, 0, 255], "the preview hides the white and the ring");
    assert_eq!(image(&app), start, "the image's pixels stay");
    answer(&mut app, true);
    assert_eq!((steps(&app), image(&app)), (n + 1, start.clone()), "one undo step; the pixels as they were");
    assert_eq!(mask(&app).map(|m| m.tolerance), Some(20.0));

    // Opened again: the image's mask settings.
    click(&mut app, "Color Mask…");
    assert_eq!(dialog_fields(&app)["tolerance"].as_f64(), Some(20.0));
    answer(&mut app, false);
    assert_eq!(steps(&app), n + 1);

    assert!(menu_item(&app, "Apply Color Mask").enabled);
    click(&mut app, "Remove Color Mask");
    assert_eq!((mask(&app), image(&app), steps(&app)), (None, start, n + 2));

    // Remove Background… opens on Edge Color, Contiguous and Match distance; OK is one undo step.
    click(&mut app, "Remove Background…");
    let fields = dialog_fields(&app);
    assert_eq!(
        (fields["background"].as_str(), fields["contiguous"].as_bool(), fields["match"].as_str()),
        (Some("edges"), Some(true), Some("distance"))
    );
    answer(&mut app, true);
    assert!(closed(&app));
    assert_eq!(steps(&app), n + 3, "one undo step");
    assert_eq!(alphas(&app)[0], 0, "the corner's white is gone");
}

/// Properties' Quick Actions and the Control bar offer Remove Background for a selected image, and
/// the Control bar's button removes it at once (no dialog) with the defaults, as one step.
#[test]
fn properties_and_the_control_bar_remove_the_background_at_once() {
    let app = Rc::new(RefCell::new(app()));
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let n = steps(&app.borrow());
    let shown = Rc::clone(&app);
    let mut properties = Fixture::new(move |ui| crate::panels::properties::show(&mut shown.borrow_mut(), ui));
    properties.room(egui::vec2(280.0, 900.0)).settle();
    assert!(properties.by_label("Remove Background").is_some(), "Properties offers it");
    let shown = Rc::clone(&app);
    let mut bar = Fixture::new(move |ui| crate::chrome::control_bar(&mut shown.borrow_mut(), ui));
    bar.room(egui::vec2(2400.0, 300.0)).settle();
    let button = bar.by_label("Remove Background").expect("the Control bar offers it");
    bar.click(&button);
    // The bar's press fires its command; the window runs what a frame fired.
    let ctx = bar.ctx().clone();
    for fired in &craft::ui::runtime::frame::take_output(&ctx).fired {
        crate::shortcuts::run_fired(&mut app.borrow_mut(), &ctx, fired);
    }
    let app = app.borrow();
    assert!(closed(&app), "it runs at once");
    assert_eq!(steps(&app), n + 1, "one undo step");
    // The defaults (tolerance and softness 10): the white goes, the near-white ring fades.
    let [corner, ring, block] = alphas(&app);
    assert!(corner == 0 && ring > 0 && ring < 255 && block == 255, "{:?}", alphas(&app));
}
