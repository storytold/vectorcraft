//! Export As and the raster options dialogs, drawn headlessly.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;

use super::*;
use crate::Services;
use crate::theme;

pub(super) type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with `artboards` 60×40 pt artboards whose save dialog answers `/out/<suggested>` and
/// whose writer records what it is given.
pub(super) fn app(artboards: usize) -> (VectorcraftApp, Written) {
    app_saving(artboards, |p| format!("/out/{}", p.name))
}

/// [`app`], with a save dialog that answers `pick`.
pub(super) fn app_saving(artboards: usize, pick: fn(&crate::FilePick) -> String) -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        pick_save: Some(Box::new(move |p: &crate::FilePick| Some(pick(p)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": artboards})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, written)
}

/// One headless frame of the dialog layer.
pub(super) fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
}

pub(super) fn kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

pub(super) fn set(app: &mut VectorcraftApp, field: &str, value: serde_json::Value) {
    app.ui.dialog.as_mut().expect("a dialog is open").fields.insert(field.into(), value);
}

pub(super) fn names(w: &Written) -> Vec<String> {
    w.borrow().iter().map(|(p, _)| p.clone()).collect()
}

#[test]
fn png_options_draw_and_export_with_the_chosen_options() {
    let (mut app, written) = app(2);
    png_options::open(&mut app, fileio::format("png").unwrap(), json!({"format": "png", "path": "/out/a.png", "artboard": 1}));
    assert_eq!(kind(&app), Some("pngOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("ppi", 0.0), d.str("background"), d.str("antiAlias"), d.bool("interlaced")), (72.0, "transparent".into(), "art".into(), false));
    assert_eq!(d.fields["__size"], json!([60.0, 40.0]), "the chosen artboard");
    frame(&mut app);
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("__estimatedBytes").and_then(|v| v.as_u64()).is_some());
    // Other resolution and an Other background colour draw their own field and colour button.
    set(&mut app, "__otherPpi", json!(true));
    set(&mut app, "ppi", json!(200));
    set(&mut app, "background", json!("#204080"));
    set(&mut app, "antiAlias", json!("type"));
    set(&mut app, "interlaced", json!(true));
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/a.png");
    assert!(app.ui.dialog.is_none());
    let png = written.borrow()[0].1.clone();
    assert_eq!(png[28], 1, "interlaced");
    let phys = png.windows(4).position(|w| w == b"pHYs").unwrap();
    assert_eq!(&png[phys + 4..phys + 8], ((200.0f64 / 0.0254).round() as u32).to_be_bytes());
    let img = craft_image_codecs::decode(&png)
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
    assert_eq!(img.dimensions(), (167, 111), "60 × 40 pt at 200 ppi");
    assert_eq!(img.get_pixel(0, 0).0, [0x20, 0x40, 0x80, 255]);
    // JPEG Options: quality, and no transparent background.
    png_options::open(&mut app, fileio::format("jpg").unwrap(), json!({"format": "jpg", "path": "/out/a.jpg"}));
    assert_eq!(kind(&app), Some("jpgOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("background"), "white");
    frame(&mut app);
    set(&mut app, "quality", json!(40));
    confirm(&mut app).unwrap();
    assert_eq!(&written.borrow()[1].1[..2], [0xFF, 0xD8]);
}

#[test]
fn png_options_estimated_file_size_follows_resolution() {
    let (mut app, _) = app(1);
    png_options::open(&mut app, fileio::format("png").unwrap(), json!({"format": "png", "path": "/out/a.png"}));
    frame(&mut app);
    let small = app.ui.dialog.as_ref().unwrap().fields["__estimatedBytes"].as_u64().unwrap();
    set(&mut app, "ppi", json!(300.0));
    frame(&mut app);
    let large = app.ui.dialog.as_ref().unwrap().fields["__estimatedBytes"].as_u64().unwrap();
    assert!(large > small, "{large} vs {small}");
}

#[test]
fn export_as_png_picks_the_file_then_shows_png_options() {
    let (mut app, written) = app(1);
    app.run("file.export.png", json!({})).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("path"), "/out/Untitled-1.png");
    assert!(written.borrow().is_empty());
    app.ui.dialog = None;
    let r = app.run("file.export.png", json!({"path": "/x/a.png", "ppi": 144})).unwrap();
    assert_eq!(r["path"], "/x/a.png");
    assert_eq!(&written.borrow()[0].1[16..20], 120u32.to_be_bytes(), "60 pt at 144 ppi");
}

/// File › Export › Export As PNG… exports the artboard in use, not the first (#5502), and each
/// artboard the Artboard tool has selected, one file each.
#[test]
fn export_as_png_exports_the_active_artboard() {
    let (mut app, written) = app(2);
    // The ellipse is on the first artboard; the second is empty.
    let opaque = |bytes: &[u8]| {
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
            .pixels()
            .any(|p| p.0[3] > 0)
    };
    app.view_mut().unwrap().artboard = 1;
    app.run("file.export.png", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["artboard"], json!(1));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(!opaque(&written.borrow()[0].1), "the second artboard, which is empty");
    app.view_mut().unwrap().artboard = 0;
    app.run("file.export.png", json!({})).unwrap();
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(opaque(&written.borrow()[1].1), "the first artboard, with the ellipse");
    // A choice given is kept.
    app.run("file.export.png", json!({"useArtboards": false})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("artboard").is_none_or(serde_json::Value::is_null));
    app.ui.dialog = None;
    // Artboards the Artboard tool selected: one file each.
    app.select_tool("artboard");
    app.session.set_tool_option("selected", &json!([0, 1]));
    app.run("file.export.png", json!({})).unwrap();
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(names(&written)[2..], ["/out/Untitled-1-Artboard-1.png", "/out/Untitled-1-Artboard-2.png"]);
}

#[test]
fn export_as_with_a_range_then_png_options_writes_one_file_per_artboard() {
    let (mut app, written) = app(3);
    app.run("file.exportAs", json!({})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("range"), "1-3");
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    set(&mut app, "all", json!(false));
    set(&mut app, "range", json!("1, 3"));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"), "PNG Options follow");
    assert!(written.borrow().is_empty(), "nothing is written before the options are confirmed");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("range"), d.str("background")), ("/out/Untitled-1.png".into(), "1, 3".into(), "transparent".into()));
    assert!(!d.fields.contains_key("__size"), "two artboards: no single size to show");
    frame(&mut app);
    set(&mut app, "ppi", json!(144));
    set(&mut app, "interlaced", json!(true));
    set(&mut app, "antiAlias", json!("none"));
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(names(&written), ["/out/Untitled-1-Artboard-1.png", "/out/Untitled-1-Artboard-3.png"]);
    let png = &written.borrow()[0].1;
    assert_eq!(png[28], 1, "interlaced");
    let img = craft_image_codecs::decode(png)
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
    assert_eq!(img.dimensions(), (120, 80), "144 ppi");
    assert!(img.pixels().all(|p| p[3] == 0 || p[3] == 255), "no anti-aliasing");
}

#[test]
fn a_bad_range_keeps_export_as_open() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"useArtboards": true, "range": "5"})).unwrap();
    assert!(!app.ui.dialog.as_ref().unwrap().bool("all"));
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some("exportAs"));
    assert!(written.borrow().is_empty());
}

/// An emptied Range field shows an example of the artboard numbers it takes, as Save As's does.
#[test]
fn an_empty_export_as_range_shows_an_example() {
    let (mut app, _) = app(2);
    app.run("file.exportAs", json!({"useArtboards": true, "range": ""})).unwrap();
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
    assert!(text.lines().any(|l| l == "1-3, 5"), "{text}");
}

/// #5941: the Format list showed no scroll bar until the pointer came over it, so nothing told
/// the user that more formats lay below the last one shown, and it opened again where it was last
/// scrolled, with its first formats out of view. Opened with the pointer elsewhere and a format
/// near the end chosen, a list that does not show every format shows its bar, the chosen format shows
/// whole, and a list opened again after a scroll shows the same rows as when it first opened.
#[test]
fn the_format_list_shows_its_scroll_bar_at_rest_and_opens_with_the_chosen_format_in_view() {
    fn walk(shape: &egui::Shape, clip: egui::Rect, texts: &mut Vec<(String, egui::Rect, egui::Rect)>, bars: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) => texts.push((t.galley.text().into(), t.visual_bounding_rect(), clip)),
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.width() <= 8.0 && r.rect.height() > 40.0 => bars.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, texts, bars)),
            _ => {}
        }
    }
    // A built-in format six from the end of the built-in ones: out of view when the list opens at
    // its top, and far enough from the end that a plug-in format another test adds after the
    // built-in ones does not move the rows shown around it. The window is short enough that the
    // list centered on it stops short of its end.
    let built_in: Vec<_> = fileio::FORMATS.iter().filter(|f| f.write && !matches!(f.id, "vectorcraft" | "ai")).collect();
    let labels: Vec<&str> = built_in.iter().map(|f| f.label).collect();
    let chosen = built_in.len().checked_sub(6).and_then(|i| built_in.get(i)).copied().expect("Export As lists formats");
    let (mut app, _) = app(1);
    app.run("file.exportAs", json!({"format": chosen.id})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    // Every scroll area takes the bar at rest from the theme, whatever style the list sets itself.
    assert_eq!(ctx.global_style().spacing.scroll, crate::widgets::list_scroll_style());
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 280.0)));
    let mut time = 0.0;
    let popup = std::cell::RefCell::new(vec![]);
    // The texts and bars painted on the open list (without the dialog under it), and every text.
    let mut run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.25;
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, events, time: Some(time), ..Default::default() }, |ui| {
            show(app, ui.ctx());
            popup.replace(crate::widgets::top_popup_shapes(ui.ctx()));
        });
        out.textures_delta.clear();
        let (mut texts, mut bars, mut all) = (vec![], vec![], vec![]);
        popup.take().iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut texts, &mut bars));
        out.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut all, &mut vec![]));
        (texts, bars, all)
    };
    let away = egui::Event::PointerMoved(egui::pos2(1.0, 1.0));
    run(&mut app, vec![away.clone()]);
    let (_, _, texts) = run(&mut app, vec![]);
    let field = texts.iter().find(|(s, ..)| s == chosen.label).map(|(_, r, _)| r.center()).expect("the Format field shows the chosen format");
    let click_at = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // Opens the list, moves the pointer away and returns the list's rect, the format names drawn in
    // it with their clip rects, and the bars drawn.
    type Texts = Vec<(String, egui::Rect, egui::Rect)>;
    type Run<'r> = dyn FnMut(&mut VectorcraftApp, Vec<egui::Event>) -> (Texts, Vec<egui::Rect>, Texts) + 'r;
    let open = |run: &mut Run<'_>, app: &mut VectorcraftApp| {
        run(app, vec![egui::Event::PointerMoved(field), click_at(field, true)]);
        run(app, vec![click_at(field, false)]);
        run(app, vec![away.clone()]);
        let (mut texts, mut bars) = (vec![], vec![]);
        for _ in 0..4 {
            (texts, bars, _) = run(app, vec![]);
        }
        let list = ctx
            .memory(|m| {
                m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id))
            })
            .expect("the Format list is open");
        let rows: Vec<_> = texts.into_iter().filter(|(s, r, _)| labels.contains(&s.as_str()) && list.contains_rect(*r)).collect();
        (list, rows, bars)
    };
    let whole = |rows: &[(String, egui::Rect, egui::Rect)], name: &str| {
        rows.iter().any(|(s, r, clip)| s == name && r.top() >= clip.top() && r.bottom() <= clip.bottom())
    };
    let tops = |rows: &[(String, egui::Rect, egui::Rect)]| rows.iter().map(|(s, r, _)| (s.clone(), r.top().round())).collect::<Vec<_>>();
    let (list, rows, bars) = open(&mut run, &mut app);
    assert!(!rows.is_empty(), "the Format list is open");
    assert!(whole(&rows, chosen.label), "the chosen format shows whole: {:?}", tops(&rows));
    // A list that shows every format needs no bar; one that does not shows its bar.
    if rows.len() < labels.len() {
        assert!(
            bars.iter().any(|b| list.contains_rect(*b) && b.left() > list.center().x),
            "the list's scroll bar is drawn at rest in {list:?}: {bars:?}"
        );
    }

    // Scrolled, closed and opened again, the list shows the rows it first opened with, not the
    // offset it was scrolled to.
    run(&mut app, vec![egui::Event::PointerMoved(list.center())]);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 30.0),
        modifiers: Default::default(),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    };
    run(&mut app, vec![wheel]);
    let mut scrolled = vec![];
    for _ in 0..4 {
        (scrolled, ..) = run(&mut app, vec![]);
    }
    let scrolled: Vec<_> = scrolled.into_iter().filter(|(s, r, _)| labels.contains(&s.as_str()) && list.contains_rect(*r)).collect();
    // Escape closes it: a list taller than the room beside its field covers the field.
    let escape = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    run(&mut app, vec![escape(true)]);
    run(&mut app, vec![escape(false)]);
    let (_, reopened, _) = open(&mut run, &mut app);
    if rows.len() < labels.len() {
        assert_ne!(tops(&scrolled), tops(&rows), "the wheel scrolled the list");
    }
    assert_eq!(tops(&reopened), tops(&rows), "the reopened list shows the rows it first opened with");
    assert!(whole(&reopened, chosen.label), "the chosen format shows whole: {:?}", tops(&reopened));
}

/// #5941: the Format list stopped at the room on one side of its field, so a window with room for
/// every format still cut the list short. In windows of 1280×900 and 1000×856 the list shows every
/// built-in format whole, the grayed-out DWG row after them included, inside the window, without a
/// scroll bar. A plug-in format that another test adds or removes meanwhile is not counted.
#[test]
fn the_format_list_shows_every_format_when_the_window_has_room() {
    fn walk(shape: &egui::Shape, clip: egui::Rect, texts: &mut Vec<(String, egui::Rect, egui::Rect)>, bars: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Text(t) => texts.push((t.galley.text().into(), t.visual_bounding_rect(), clip)),
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.width() <= 8.0 && r.rect.height() > 40.0 => bars.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, texts, bars)),
            _ => {}
        }
    }
    let labels: Vec<&str> =
        fileio::FORMATS.iter().filter(|f| f.write && !matches!(f.id, "vectorcraft" | "ai")).map(|f| f.label).chain(["DWG (use DXF)"]).collect();
    for (width, height) in [(1280.0, 900.0), (1000.0, 856.0)] {
        let case = format!("{width}×{height}");
        let (mut app, _) = app(1);
        app.run("file.exportAs", json!({"format": "png"})).unwrap();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        let mut time = 0.0;
        let popup = std::cell::RefCell::new(vec![]);
        // The texts and bars painted on the open list (without the dialog under it), and every text.
        let mut run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.25;
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, time: Some(time), ..Default::default() }, |ui| {
                show(app, ui.ctx());
                popup.replace(crate::widgets::top_popup_shapes(ui.ctx()));
            });
            out.textures_delta.clear();
            let (mut texts, mut bars, mut all) = (vec![], vec![], vec![]);
            popup.take().iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut texts, &mut bars));
            out.shapes.iter().for_each(|c| walk(&c.shape, c.clip_rect, &mut all, &mut vec![]));
            (texts, bars, all)
        };
        let away = egui::Event::PointerMoved(egui::pos2(1.0, 1.0));
        run(&mut app, vec![away.clone()]);
        let (_, _, all) = run(&mut app, vec![]);
        let field = all.iter().find(|(s, ..)| s == "PNG").map(|(_, r, _)| r.center()).expect("the Format field shows PNG");
        let click = |pressed| egui::Event::PointerButton { pos: field, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut app, vec![egui::Event::PointerMoved(field), click(true)]);
        run(&mut app, vec![click(false)]);
        run(&mut app, vec![away.clone()]);
        let list_rect = || {
            ctx.memory(|m| {
                m.layer_ids().filter(|l| l.order == egui::Order::Foreground && m.areas().is_visible(l)).last().and_then(|l| m.area_rect(l.id))
            })
        };
        // A plug-in format that another test installs or removes changes the list's height, and the
        // list is placed for its new height on the pass after. Read it once it keeps its rect.
        let (mut texts, mut bars, mut list) = (vec![], vec![], None);
        for _ in 0..12 {
            (texts, bars, _) = run(&mut app, vec![]);
            let now = list_rect();
            if now.is_some() && now == list {
                break;
            }
            list = now;
        }
        let list = list.expect("the Format list is open");
        assert!(screen.contains_rect(list), "{case}: the list {list:?} is inside the window");
        let whole: Vec<&str> = labels
            .iter()
            .copied()
            .filter(|name| {
                texts.iter().any(|(s, r, clip)| s == name && list.contains_rect(*r) && r.top() >= clip.top() && r.bottom() <= clip.bottom())
            })
            .collect();
        assert_eq!(whole, labels, "{case}: every format shows whole in {list:?}");
        assert!(bars.is_empty(), "{case}: a list that shows every format draws no scroll bar: {bars:?}");
    }
}

#[test]
fn svg_continues_in_svg_options() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "svg", "useArtboards": true})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("svgOptions"), "the SVG options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.bool("useArtboards"), d.str("range")), ("/out/Untitled-1.svg".into(), true, "all".into()));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(names(&written), ["/out/Untitled-1-Artboard-1.svg", "/out/Untitled-1-Artboard-2.svg"]);
}

/// Without Use Artboards, a PDF continues in Save PDF too, without its artboard choice, and the
/// file is one page of the art's bounds. Its options apply: a drawing 10 m long is written at
/// full size in points with "Write pages longer than 200 in at full size in points" (on by
/// default), and with a `UserUnit` without it.
#[test]
fn pdf_art_continues_in_the_save_pdf_dialog() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "pdf"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("savePdf"), "the PDF options follow");
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("path"), "/out/Untitled-1.pdf");
    assert!(written.borrow().is_empty());
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/Untitled-1.pdf");
    let pdf = vectorcraft_pdf::import(&written.borrow()[0].1).unwrap();
    assert_eq!(pdf.artboards.len(), 1);
    assert_eq!((pdf.artboards[0].rect.width().round(), pdf.artboards[0].rect.height().round()), (30.0, 20.0), "the ellipse's bounds");

    app.run("shape.rectangle", json!({"x": 0, "y": 100, "width": 28_346.46, "height": 50})).unwrap();
    app.run("file.exportAs", json!({"format": "pdf"})).unwrap();
    confirm(&mut app).unwrap();
    frame(&mut app);
    confirm(&mut app).unwrap();
    let bytes = written.borrow()[1].1.clone();
    assert!(!String::from_utf8_lossy(&bytes).contains("/UserUnit"));
    let width = vectorcraft_pdf::import(&bytes).unwrap().artboards[0].rect.width();
    assert!((28_346.0..28_350.0).contains(&width), "{width}");

    app.run("file.exportAs", json!({"format": "pdf"})).unwrap();
    confirm(&mut app).unwrap();
    set(&mut app, "longPagesInPoints", json!(false));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let bytes = written.borrow()[2].1.clone();
    assert!(String::from_utf8_lossy(&bytes).contains("/UserUnit"));
    let width = vectorcraft_pdf::import(&bytes).unwrap().artboards[0].rect.width();
    assert!((28_346.0..28_350.0).contains(&width), "{width}");
}

#[test]
fn jpeg_and_webp_options_draw_and_export() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "jpg"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("jpgOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("background"), d.f64("quality", 0.0)), ("white".into(), 90.0));
    assert_eq!(d.fields["__size"], json!([30.0, 20.0]), "the art's bounds");
    frame(&mut app);
    set(&mut app, "background", json!("#204080"));
    set(&mut app, "quality", json!(70));
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/Untitled-1.jpg");
    let img = craft_image_codecs::decode(&written.borrow()[0].1)
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
        .to_rgb8();
    assert_eq!(img.dimensions(), (30, 20));
    let px = img.get_pixel(0, 0);
    assert!(px[2] > 100 && px[0] < 60, "the background colour: {px:?}");

    app.run("file.exportAs", json!({"format": "webp", "useArtboards": true})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("webpOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__size"], json!([60.0, 40.0]), "the one artboard");
    set(&mut app, "__otherPpi", json!(true));
    set(&mut app, "ppi", json!(36));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow().last().cloned().unwrap();
    assert_eq!(path, "/out/Untitled-1.webp", "one artboard is one file, named as picked");
    assert_eq!(
        craft_image_codecs::decode(&bytes)
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
            .width(),
        30
    );
}

#[test]
fn old_ids_and_agents_export_directly_with_a_path() {
    let (mut app, written) = app(2);
    let r = app.run("file.export.png", json!({"path": "/x/a.png", "artboard": 1, "ppi": 144})).unwrap();
    assert_eq!(r["path"], "/x/a.png");
    let r = app.run("file.exportAs", json!({"path": "/x/b.webp", "useArtboards": true})).unwrap();
    assert_eq!(r["files"], json!(["/x/b-Artboard-1.webp", "/x/b-Artboard-2.webp"]));
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 3);
    assert_eq!(&written.borrow()[0].1[16..20], 120u32.to_be_bytes(), "60 pt at 144 ppi");
    assert!(crate::menus::enabled(&app, "file.exportAs"));
}

#[test]
fn nothing_to_export_keeps_export_as_open_before_the_save_dialog() {
    let (mut app, written) = app(1);
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| panic!("no save dialog without art")));
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("file.exportAs", json!({})).unwrap();
    assert!(confirm(&mut app).unwrap_err().contains("no visible art"));
    assert_eq!(kind(&app), Some("exportAs"));
    // With Use Artboards the empty artboard is exported.
    set(&mut app, "useArtboards", json!(true));
    app.services.pick_save = Some(Box::new(|p: &crate::FilePick| Some(format!("/out/{}", p.name))));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"));
    assert!(written.borrow().is_empty());
}

/// Art that a plot leaves out (here at 0% opacity) is nothing to export as HP-GL of the art's
/// bounds, before the save dialog; other formats still export it.
#[test]
fn art_a_plot_leaves_out_keeps_hpgl_export_as_open_before_the_save_dialog() {
    let (mut app, _) = app(1);
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| panic!("no save dialog without art the plot draws")));
    app.run("select.all", json!({})).unwrap();
    app.run("transparency.set", json!({"opacity": 0})).unwrap();
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    assert!(confirm(&mut app).unwrap_err().contains("no visible art"));
    assert_eq!(kind(&app), Some("exportAs"));
    app.services.pick_save = Some(Box::new(|p: &crate::FilePick| Some(format!("/out/{}", p.name))));
    set(&mut app, "format", json!("png"));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"));
}

#[test]
fn pdf_artboards_continue_in_the_save_pdf_dialog() {
    let (mut app, written) = app(3);
    app.run("file.exportAs", json!({"format": "pdf", "useArtboards": true, "range": "2-3"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("savePdf"), "the PDF options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("range")), ("/out/Untitled-1.pdf".into(), "2-3".into()));
    assert!(written.borrow().is_empty());
    frame(&mut app);
    confirm(&mut app).unwrap();
    let pdf = vectorcraft_pdf::import(&written.borrow()[0].1).unwrap();
    assert_eq!(pdf.artboards.len(), 2, "one page per artboard in the range");
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
            set(&mut app, "all", json!(false));
            set(&mut app, "range", json!(range));
            confirm(&mut app).unwrap();
            // The format's own options, then the file.
            while app.ui.dialog.is_some() {
                frame(&mut app);
                confirm(&mut app).unwrap();
            }
            assert_eq!(names(&written), [format!("/out/Untitled-1.{format}")], "{format} {range}");
            if !ellipse.is_empty() {
                let text = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
                assert_eq!(text.contains(ellipse), on, "{format} {range}: {text}");
            }
        }
    }
}

/// The Enter that closed the save panel, still held as the format's options open, doesn't write
/// the file; Enter pressed again after it is let go does. So for every options dialog Export As
/// opens after its panel, an exporter plug-in's included (as Save As's options).
#[test]
fn the_enter_that_closed_the_save_panel_does_not_confirm_the_options() {
    let params = r#"{"speed":{"type":"number","min":1,"max":100,"default":40}}"#;
    let module = wat::parse_str(vectorcraft_plugins::wat::returning(
        &vectorcraft_plugins::wat::exporter("test.ui.held", r#"["tuh"]"#, params),
        r#"{"dataBase64":"SGk="}"#,
    ))
    .unwrap();
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    // Each format whose options follow the panel (PDF's: Save PDF, with Use Artboards on or off).
    let formats = [
        ("png", false),
        ("svg", false),
        ("pdf", true),
        ("pdf", false),
        ("txt", false),
        ("dxf", false),
        ("eps", false),
        ("plugin.test.ui.held", false),
    ];
    for (format, artboards) in formats {
        let (mut app, written) = app(1);
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
        app.run("file.exportAs", json!({"format": format})).unwrap();
        set(&mut app, "useArtboards", json!(artboards));
        confirm(&mut app).unwrap();
        let options = kind(&app).map(str::to_string);
        assert!(options.as_deref().is_some_and(|k| k != "exportAs"), "{format}: its options follow the panel, not {options:?}");
        frame(&mut app, vec![enter(true)]);
        assert_eq!(kind(&app).map(str::to_string), options, "{format}: the Enter still held isn't the options'");
        assert!(written.borrow().is_empty(), "{format}: nothing written");
        frame(&mut app, vec![enter(false)]);
        frame(&mut app, vec![enter(true)]);
        assert!(app.ui.dialog.is_none(), "{format}: Enter pressed again confirms");
        assert!(!written.borrow().is_empty(), "{format}: and writes");
    }
    vectorcraft_plugins::registry::remove("test.ui.held");
}
