//! Export As, driven through the dialog host headlessly.

use std::cell::RefCell;
use std::rc::Rc;

use craft::host::PickRequest;
use craft::ui::dialog::{DialogHost, DialogId, HeldOk};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::tests_picks::Picker;
use crate::{Services, VectorcraftApp};

pub(super) type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// How the save panel answers a pick: the path chosen, `None` for Cancel.
type Answer = Box<dyn Fn(&PickRequest) -> Option<String>>;

/// The save panel of this test's app: its scripted picks, how many of them [`frame`] has answered, and how it
/// answers.
struct Panel {
    picker: Picker,
    answered: usize,
    answer: Answer,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// The suggested name of a pick in `/out`.
fn in_out(p: &PickRequest) -> String {
    format!("/out/{}", p.file_name.as_deref().unwrap_or_default())
}

/// An app with `artboards` 60×40 pt artboards whose save panel answers `/out/<suggested>` and
/// whose writer records what it is given.
pub(super) fn app(artboards: usize) -> (VectorcraftApp, Written) {
    app_saving(artboards, in_out)
}

/// [`app`], with a save panel that answers `pick`.
pub(super) fn app_saving(artboards: usize, pick: impl Fn(&PickRequest) -> String + 'static) -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    save_panel(&mut app, move |p| Some(pick(p)));
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": artboards})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, written)
}

/// Gives `app`, which has a window, a save panel that answers each pick with `answer` (`None`: Cancel) once a frame
/// has passed ([`answer_saves`]). The app keeps its own reader and writer.
pub(super) fn save_panel(app: &mut VectorcraftApp, answer: impl Fn(&PickRequest) -> Option<String> + 'static) {
    let (read, write) = (app.services.read.take(), app.services.write.take());
    let picker = crate::tests_picks::scripted(app);
    (app.services.read, app.services.write) = (read, write);
    PANEL.set(Some(Panel { picker, answered: 0, answer: Box::new(answer) }));
}

/// From now on the save panel answers picks with `answer` (`None`: Cancel).
pub(super) fn on_save(answer: impl Fn(&PickRequest) -> Option<String> + 'static) {
    PANEL.with_borrow_mut(|panel| {
        if let Some(panel) = panel {
            panel.answer = Box::new(answer);
        }
    });
}

/// One headless frame: the app's pass, the dialog layer's pump, then the save panels asked for meanwhile answer and
/// what waited on them runs.
pub(super) fn frame(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a test app has a window");
    // The window's pass with the app's frame input, as the hosts run it: what its controls fire reaches the app.
    crate::testing::part_frame(app, &ctx, Default::default(), |app, ui| app.ui(ui));
    super::host::pump(app, &ctx);
    answer_saves(app);
}

/// The save panels asked for since the last call answer, and what waited on them runs.
pub(super) fn answer_saves(app: &mut VectorcraftApp) {
    let asked = PANEL.with_borrow_mut(|panel| {
        let Some(panel) = panel else { return false };
        let requests = panel.picker.requests();
        // The newest open pick is answered first.
        for request in requests[panel.answered..].iter().rev() {
            match (panel.answer)(request) {
                Some(path) => panel.picker.answer_saved(&path, None),
                None => panel.picker.cancel(),
            }
        }
        let asked = requests.len() > panel.answered;
        panel.answered = requests.len();
        asked
    });
    if asked {
        let ctx = app.ui_ctx.clone().expect("a test app has a window");
        crate::tests_picks::poll(app, &ctx);
    }
}

/// Runs `f` on the host with the top dialog's id; `None` when no dialog is open.
fn on_top<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost, &egui::Context, DialogId) -> R) -> Option<R> {
    let ctx = app.ui_ctx.clone()?;
    app.dialogs()?.with(|host| {
        let id = host.open_dialogs().last()?.id;
        Some(f(host, &ctx, id))
    })
}

/// The key of the top dialog, `None` when none is open.
pub(super) fn kind(app: &VectorcraftApp) -> Option<String> {
    app.dialogs().and_then(|d| d.with(|host| host.open_dialogs().last().and_then(|o| o.key.clone())))
}

/// The top dialog's values as JSON.
pub(super) fn fields(app: &VectorcraftApp) -> Value {
    app.dialogs().and_then(|d| d.with(|host| host.open_dialogs().last().and_then(|o| o.fields.clone()))).unwrap_or(Value::Null)
}

/// Sets a value of the top dialog as the user editing it would.
pub(super) fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
    try_set(app, field, value).expect("the field is set");
}

/// [`set`], with the reason when the row refuses the value (a value the row can't hold, or a greyed-out row).
pub(super) fn try_set(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
    on_top(app, |host, ctx, id| host.set_json_field(ctx, id, field, value)).expect("a dialog is open")
}

/// Presses OK on the top dialog: the first frame runs its confirmation, the second closes it and runs what follows.
pub(super) fn confirm(app: &mut VectorcraftApp) {
    // A refusal is read from the dialog staying open.
    let _ = try_confirm(app);
}

/// [`confirm`], with the reason when the dialog refused OK and stays open.
pub(super) fn try_confirm(app: &mut VectorcraftApp) -> Result<(), String> {
    // A held OK files its answer here (`confirm_by_id` reports `false`) and is answered in the first frame.
    let id = on_top(app, |host, ctx, id| {
        host.confirm_by_id(ctx, id);
        id
    })
    .expect("a dialog is open");
    frame(app);
    if let Some(HeldOk::Refused(reason)) = app.dialogs().and_then(|d| d.with(|host| host.held_state(id))) {
        return Err(reason.as_str().to_string());
    }
    frame(app);
    Ok(())
}

/// Cancels the top dialog.
pub(super) fn cancel(app: &mut VectorcraftApp) {
    on_top(app, |host, ctx, id| host.cancel_by_id(ctx, id)).expect("a dialog is open");
    frame(app);
    frame(app);
}

pub(super) fn names(w: &Written) -> Vec<String> {
    w.borrow().iter().map(|(p, _)| p.clone()).collect()
}

/// The picture a raster file holds.
pub(super) fn rgba(bytes: &[u8]) -> image::RgbaImage {
    craft_image_codecs::decode(bytes)
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
        .to_rgba8()
}

#[test]
fn export_as_png_picks_the_file_then_shows_png_options() {
    let (mut app, written) = app(1);
    app.run("file.export.png", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    assert_eq!(fields(&app)["path"], "/out/Untitled-1.png");
    assert!(written.borrow().is_empty());
    cancel(&mut app);
    let r = app.run("file.export.png", json!({"path": "/x/a.png", "ppi": 144})).unwrap();
    assert_eq!(r["path"], "/x/a.png");
    assert_eq!(&written.borrow()[0].1[16..20], 120u32.to_be_bytes(), "60 pt at 144 ppi");
}

/// PNG Options' pixel size is in the window's accessibility tree, and it follows the resolution.
#[test]
fn png_options_name_the_pixel_size_in_the_accessibility_tree() {
    let (mut app, _) = app(1);
    app.run("file.export.png", json!({})).unwrap();
    answer_saves(&mut app);
    let sizes = |app: &mut VectorcraftApp| -> Vec<String> {
        super::tests::settle(app);
        let out = super::tests::frame(app, vec![]);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        nodes.iter().filter_map(|(_, node)| node.label()).filter(|label| label.ends_with(" px")).map(str::to_owned).collect()
    };
    let at_72 = sizes(&mut app);
    assert_eq!(at_72, ["60 × 40 px"]);
    set(&mut app, "ppi", json!(144));
    assert_eq!(sizes(&mut app), ["120 × 80 px"]);
}

/// The role and name of each node of the window's accessibility tree, in a frame drawn once the dialogs have settled.
fn access_names(app: &mut VectorcraftApp) -> Vec<(egui::accesskit::Role, String)> {
    super::tests::settle(app);
    let out = super::tests::frame(app, vec![]);
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    nodes.iter().filter_map(|(_, node)| node.label().map(|label| (node.role(), label.to_owned()))).collect()
}

/// The hints under Export As, DXF Options and EPS Options, the words of the DXF scale row and the HP-GL pen numbers
/// are in the window's accessibility tree, and the HP-GL pen error is announced as an alert.
#[test]
fn export_dialogs_name_their_hints_rows_and_errors_in_the_accessibility_tree() {
    let names = |app: &mut VectorcraftApp| -> Vec<String> { access_names(app).into_iter().map(|(_, name)| name).collect() };
    let (mut app, _) = app(1);
    app.run("file.exportAs", json!({"format": "png", "useArtboards": false})).unwrap();
    let shown = names(&mut app);
    assert!(shown.iter().any(|n| n == "The bounds of the visible art."), "{shown:?}");
    cancel(&mut app);

    crate::menus::invoke(&mut app, "ui.dxfOptionsDialog", json!({}));
    let shown = names(&mut app);
    for word in ["=", "Units"] {
        assert!(shown.iter().any(|n| n == word), "{word} is not named in {shown:?}");
    }
    assert!(shown.iter().any(|n| n.starts_with("Type becomes outlines")), "{shown:?}");
    cancel(&mut app);

    crate::menus::invoke(&mut app, "ui.epsOptionsDialog", json!({}));
    let shown = names(&mut app);
    assert!(shown.iter().any(|n| n.starts_with("Transparency is flattened with the preset.")), "{shown:?}");
    cancel(&mut app);

    app.run("ui.hpglOptionsDialog", json!({})).unwrap();
    set(&mut app, "pens", json!(["#ff0000", "#000000"]));
    set(&mut app, "__penError", json!("The pens could not be listed."));
    let shown = access_names(&mut app);
    assert!(shown.iter().any(|(_, n)| n == "2"), "the second pen's number is not named in {shown:?}");
    assert!(
        shown.iter().any(|(role, n)| *role == egui::accesskit::Role::Alert && n.contains("The pens could not be listed.")),
        "the pen error is not an alert in {shown:?}"
    );
}

/// File › Export › Export As PNG… exports the artboard in use, not the first (#5502), and each
/// artboard the Artboard tool has selected, one file each.
#[test]
fn export_as_png_exports_the_active_artboard() {
    let (mut app, written) = app(2);
    // The ellipse is on the first artboard; the second is empty.
    let opaque = |bytes: &[u8]| rgba(bytes).pixels().any(|p| p.0[3] > 0);
    app.view_mut().unwrap().artboard = 1;
    app.run("file.export.png", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(fields(&app)["artboard"], json!(1));
    frame(&mut app);
    confirm(&mut app);
    assert!(!opaque(&written.borrow()[0].1), "the second artboard, which is empty");
    app.view_mut().unwrap().artboard = 0;
    app.run("file.export.png", json!({})).unwrap();
    answer_saves(&mut app);
    frame(&mut app);
    confirm(&mut app);
    assert!(opaque(&written.borrow()[1].1), "the first artboard, with the ellipse");
    // A choice given is kept.
    app.run("file.export.png", json!({"useArtboards": false})).unwrap();
    answer_saves(&mut app);
    assert!(fields(&app).get("artboard").is_none_or(Value::is_null));
    cancel(&mut app);
    // Artboards the Artboard tool selected: one file each.
    app.select_tool("artboard");
    app.session.set_tool_option("selected", &json!([0, 1]));
    app.run("file.export.png", json!({})).unwrap();
    answer_saves(&mut app);
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(names(&written)[2..], ["/out/Untitled-1-Artboard-1.png", "/out/Untitled-1-Artboard-2.png"]);
}

#[test]
fn export_as_with_a_range_then_png_options_writes_one_file_per_artboard() {
    let (mut app, written) = app(3);
    app.run("file.exportAs", json!({})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(fields(&app)["range"], "1-3");
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    set(&mut app, "all", json!("false"));
    set(&mut app, "range", json!("1, 3"));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"), "PNG Options follow");
    assert!(written.borrow().is_empty(), "nothing is written before the options are confirmed");
    let d = fields(&app);
    assert_eq!((&d["path"], &d["range"], &d["background"]), (&json!("/out/Untitled-1.png"), &json!("1, 3"), &json!("transparent")));
    frame(&mut app);
    set(&mut app, "ppi", json!(144));
    set(&mut app, "interlaced", json!(true));
    set(&mut app, "antiAlias", json!("none"));
    frame(&mut app);
    confirm(&mut app);
    assert!(kind(&app).is_none());
    assert_eq!(names(&written), ["/out/Untitled-1-Artboard-1.png", "/out/Untitled-1-Artboard-3.png"]);
    let png = &written.borrow()[0].1;
    assert_eq!(png[28], 1, "interlaced");
    let img = rgba(png);
    assert_eq!(img.dimensions(), (120, 80), "144 ppi");
    assert!(img.pixels().all(|p| p[3] == 0 || p[3] == 255), "no anti-aliasing");
}

#[test]
fn a_bad_range_keeps_export_as_open() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"useArtboards": true, "range": "5"})).unwrap();
    // All or Range holds its choice's id.
    assert_eq!(fields(&app)["all"], json!("false"));
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert!(written.borrow().is_empty());
}

#[test]
fn export_as_opened_without_a_drawing_does_not_take_a_later_drawing() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
    save_panel(&mut app, |_| Some("/out/untitled.png".into()));
    app.run("file.exportAs", json!({"format": "png"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(fields(&app)["format"], "png");

    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let error = try_confirm(&mut app).expect_err("the old Export As draft has no source drawing");
    assert!(error.contains("Open Export As from the drawing"), "{error}");
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    PANEL.with_borrow(|panel| assert!(panel.as_ref().unwrap().picker.requests().is_empty()));
}

#[test]
fn export_as_waits_for_the_drawing_that_opened_it() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "png", "useArtboards": true, "range": "1"})).unwrap();
    app.run("file.new", json!({ "width": 200, "height": 100 })).unwrap();
    let error = try_confirm(&mut app).expect_err("Export As must refuse the other drawing");
    assert!(error.contains("original drawing"), "{error}");
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(fields(&app)["range"], "1");
    PANEL.with_borrow(|panel| assert!(panel.as_ref().unwrap().picker.requests().is_empty()));
    app.run("document.activate", json!({ "index": 0 })).unwrap();
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    confirm(&mut app);
    assert_eq!(written.borrow().len(), 1);
    assert_eq!(&written.borrow()[0].1[16..20], 60u32.to_be_bytes());
}

#[test]
fn png_options_wait_for_the_drawing_that_opened_them() {
    let (mut app, written) = app(1);
    app.run("file.export.png", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    let draft = fields(&app);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let error = try_confirm(&mut app).expect_err("PNG Options must refuse the other drawing");
    assert!(error.contains("original drawing"), "{error}");
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    assert_eq!(fields(&app)["path"], draft["path"]);
    assert!(written.borrow().is_empty());

    app.run("document.activate", json!({"index": 0})).unwrap();
    confirm(&mut app);
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(&w[0].1[16..20], 60u32.to_be_bytes());
}

/// Each format's Export As continues in its options dialog, writes nothing before that dialog's OK, and then
/// writes its files: (case, artboards, Export As params, the options' key, the files, the pages of a PDF, the
/// page size of a PDF of the art's bounds).
#[test]
#[allow(clippy::type_complexity)]
fn a_format_continues_in_its_options_dialog() {
    let cases: [(&str, usize, Value, &str, Vec<&str>, Option<usize>, Option<(f64, f64)>); 3] = [
        (
            "svg with artboards",
            2,
            json!({"format": "svg", "useArtboards": true}),
            "svgOptions",
            vec!["/out/Untitled-1-Artboard-1.svg", "/out/Untitled-1-Artboard-2.svg"],
            None,
            None,
        ),
        ("pdf of the art", 2, json!({"format": "pdf"}), "pdf-export", vec!["/out/Untitled-1.pdf"], Some(1), Some((30.0, 20.0))),
        (
            "pdf of two artboards",
            3,
            json!({"format": "pdf", "useArtboards": true, "range": "2-3"}),
            "pdf-export",
            vec!["/out/Untitled-1.pdf"],
            Some(2),
            None,
        ),
    ];
    for (case, boards, params, options, files, pages, bounds) in cases {
        let (mut app, written) = app(boards);
        app.run("file.exportAs", params).unwrap();
        confirm(&mut app);
        assert_eq!(kind(&app).as_deref(), Some(options), "{case}: its options follow");
        // Save PDF's fields are its settings: the file it writes is its job's.
        let job = app.save_pdf.as_ref().and_then(|o| o.job.as_ref()?.path.clone());
        let path = fields(&app)["path"].as_str().map(str::to_string).or(job);
        assert!(path.is_some_and(|p| p.starts_with("/out/Untitled-1.")), "{case}: {}", fields(&app));
        assert!(written.borrow().is_empty(), "{case}: nothing is written before OK");
        frame(&mut app);
        confirm(&mut app);
        assert!(kind(&app).is_none(), "{case}");
        assert_eq!(names(&written), files, "{case}");
        if let Some(pages) = pages {
            let pdf = vectorcraft_pdf::import(&written.borrow()[0].1).unwrap();
            assert_eq!(pdf.artboards.len(), pages, "{case}: one page each");
            if let Some(size) = bounds {
                let rect = pdf.artboards[0].rect;
                assert_eq!((rect.width().round(), rect.height().round()), size, "{case}: the ellipse's bounds");
            }
        }
    }
}

#[test]
fn old_ids_and_agents_export_directly_with_a_path() {
    let (mut app, written) = app(2);
    let r = app.run("file.export.png", json!({"path": "/x/a.png", "artboard": 1, "ppi": 144})).unwrap();
    assert_eq!(r["path"], "/x/a.png");
    let r = app.run("file.exportAs", json!({"path": "/x/b.webp", "useArtboards": true})).unwrap();
    assert_eq!(r["files"], json!(["/x/b-Artboard-1.webp", "/x/b-Artboard-2.webp"]));
    assert!(kind(&app).is_none());
    assert_eq!(written.borrow().len(), 3);
    assert_eq!(&written.borrow()[0].1[16..20], 120u32.to_be_bytes(), "60 pt at 144 ppi");
    assert!(crate::menus::enabled(&app, "file.exportAs"));
}

#[test]
fn nothing_to_export_keeps_export_as_open_before_the_save_dialog() {
    let (mut app, written) = app(1);
    on_save(|_| panic!("no save dialog without art"));
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("file.exportAs", json!({})).unwrap();
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    // With Use Artboards the empty artboard is exported.
    set(&mut app, "useArtboards", json!(true));
    on_save(|p| Some(in_out(p)));
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    assert!(written.borrow().is_empty());
}

/// Art that a plot leaves out (here at 0% opacity) is nothing to export as HP-GL of the art's
/// bounds, before the save dialog; other formats still export it.
#[test]
fn art_a_plot_leaves_out_keeps_hpgl_export_as_open_before_the_save_dialog() {
    let (mut app, _) = app(1);
    on_save(|_| panic!("no save dialog without art the plot draws"));
    app.run("select.all", json!({})).unwrap();
    app.run("transparency.set", json!({"opacity": 0})).unwrap();
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    on_save(|p| Some(in_out(p)));
    set(&mut app, "format", json!("png"));
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
}

/// #546, #550: Export As with Use Artboards and a range writes the chosen artboard alone in every
/// format, each file holding that artboard's art (the ellipse is on artboard 1).
#[test]
fn export_as_writes_only_the_chosen_artboard_in_every_format() {
    for (format, ellipse) in [("svg", "<ellipse"), ("dxf", "HATCH"), ("pdf", ""), ("png", "")] {
        for (range, on) in [("1", true), ("3", false)] {
            let (mut app, written) = app(3);
            app.run("file.exportAs", json!({"format": format})).unwrap();
            set(&mut app, "useArtboards", json!(true));
            set(&mut app, "all", json!("false"));
            set(&mut app, "range", json!(range));
            // Export As, the format's own options, then the file.
            for _ in 0..4 {
                if kind(&app).is_none() {
                    break;
                }
                confirm(&mut app);
            }
            assert!(kind(&app).is_none(), "{format} {range}: every dialog confirmed");
            assert_eq!(names(&written), [format!("/out/Untitled-1.{format}")], "{format} {range}");
            if !ellipse.is_empty() {
                let text = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
                assert_eq!(text.contains(ellipse), on, "{format} {range}: {text}");
            }
        }
    }
}

fn native_export_as_source() -> (VectorcraftApp, Written, crate::tests_picks::Picker, vectorcraft_testkit::TempDir) {
    let (mut app, written) = app(1);
    let dir = vectorcraft_testkit::TempDir::new("export-as-after-revert");
    let capture = written.clone();
    app.services.write = Some(Box::new(move |path, bytes| {
        if path.ends_with(".vectorcraft") {
            std::fs::write(path, bytes).map_err(|error| error.to_string())
        } else {
            capture.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }
    }));
    app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|error| error.to_string())));
    let picker = crate::tests_picks::scripted(&mut app);
    app.run("file.saveAs", json!({"path": dir.join("Export.vectorcraft")})).unwrap();
    app.run("shape.ellipse", json!({"x": 45, "y": 20, "width": 10, "height": 10})).unwrap();
    (app, written, picker, dir)
}

fn revert_export_source(app: &mut VectorcraftApp) {
    let source = app.session.active().unwrap();
    let (uid, reverts) = (source.uid, source.reverts);
    app.run("file.revert", json!({"confirmed": true})).unwrap();
    let source = app.session.active().unwrap();
    assert_eq!((source.uid, source.reverts), (uid, reverts + 1));
}

#[test]
fn export_as_refuses_an_approval_from_before_native_revert() {
    let (mut app, written, picker, _dir) = native_export_as_source();
    app.run("file.exportAs", json!({"format": "png", "useArtboards": true})).unwrap();
    revert_export_source(&mut app);

    let error = try_confirm(&mut app).expect_err("an old Export As approval must stay in the dialog");
    assert!(error.contains("Reopen this dialog"));
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(fields(&app)["useArtboards"], true);
    assert!(picker.requests().is_empty(), "the retired draft did not open a save picker");
    assert!(written.borrow().is_empty());
}

#[test]
fn export_as_cannot_use_a_destination_selected_after_native_revert() {
    let (mut app, written, picker, _dir) = native_export_as_source();
    app.run("file.exportAs", json!({"format": "png"})).unwrap();
    try_confirm(&mut app).unwrap();
    assert_eq!(picker.requests().len(), 1);
    revert_export_source(&mut app);

    picker.answer_saved("/output/Old.png", None);
    crate::file_asks::poll(&mut app);
    frame(&mut app);
    assert!(kind(&app).is_none(), "the retired pick did not open PNG Options");
    assert!(written.borrow().is_empty());

    app.run("file.exportAs", json!({"format": "png"})).unwrap();
    try_confirm(&mut app).unwrap();
    assert_eq!(picker.requests().len(), 2);
    picker.answer_saved("/output/Fresh.png", None);
    crate::file_asks::poll(&mut app);
    frame(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    try_confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/output/Fresh.png"]);
}
