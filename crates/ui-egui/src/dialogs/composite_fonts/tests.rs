use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn dialog(app: &mut VectorcraftApp) -> &mut Dialog {
    app.ui.dialog.as_mut().expect("Composite Fonts open")
}

fn names(app: &VectorcraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.composite_fonts.iter().map(|f| f.name.clone()).collect()
}

fn steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Do `act` with the dialog as it is, as its button does.
fn press(app: &mut VectorcraftApp, act: Action) {
    let mut d = app.ui.dialog.clone().unwrap();
    run(app, &mut d, act).unwrap();
    app.ui.dialog = Some(d);
}

/// Type › Composite Fonts… opens on the document's fonts; New adds one, its parts edited in the
/// table; nothing changes until OK, which sets them as one undo step; Cancel leaves them.
#[test]
fn composite_fonts_change_on_ok_as_one_undo_step() {
    let mut app = app();
    crate::menus::invoke(&mut app, COMMAND, json!({}));
    assert_eq!(dialog(&mut app).kind, KIND);
    frame(&mut app);
    press(&mut app, Action::New);
    frame(&mut app);
    let d = dialog(&mut app);
    assert_eq!(d.str("name"), "Composite Font 1");
    let mut list = fonts(d);
    list[0].parts.roman.font = "JetBrains Mono".into();
    list[0].parts.roman.size = 80.0;
    list[0].parts.kana.scale_at_center = true;
    put_fonts(d, &list);
    d.fields.insert("name".into(), json!("UI Composite A"));
    frame(&mut app);
    assert!(names(&app).is_empty());
    let before = steps(&app);
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(names(&app), ["UI Composite A"]);
    assert_eq!(steps(&app), before + 1);
    let f = &app.session.active().unwrap().doc.composite_fonts[0];
    assert_eq!((f.parts.roman.font.as_str(), f.parts.roman.size, f.parts.kana.scale_at_center), ("JetBrains Mono", 80.0, true));
    // OK with nothing changed adds no step; Cancel leaves the fonts.
    open(&mut app).unwrap();
    super::super::confirm(&mut app).unwrap();
    assert_eq!(steps(&app), before + 1);
    open(&mut app).unwrap();
    press(&mut app, Action::Delete);
    super::super::cancel(&mut app);
    assert_eq!(names(&app), ["UI Composite A"]);
    // Delete Font and OK takes it out.
    open(&mut app).unwrap();
    press(&mut app, Action::Delete);
    assert_eq!(dialog(&mut app).str("name"), "");
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert!(names(&app).is_empty());
}

/// New is based on the font picked and takes the next free name; a name two fonts share, or one
/// left empty, keeps the dialog open and changes nothing.
#[test]
fn shared_or_empty_names_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    press(&mut app, Action::New);
    let d = dialog(&mut app);
    let mut list = fonts(d);
    list[0].parts.numerals.font = "Inter".into();
    put_fonts(d, &list);
    press(&mut app, Action::New);
    let list = fonts(dialog(&mut app));
    assert_eq!(list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Composite Font 1", "Composite Font 2"]);
    assert_eq!(list[1].parts.numerals.font, "Inter");
    for bad in ["composite font 1", "  "] {
        dialog(&mut app).fields.insert("name".into(), json!(bad));
        frame(&mut app);
        assert!(super::super::confirm(&mut app).is_err(), "{bad:?}");
        assert!(app.ui.dialog.is_some());
        assert!(names(&app).is_empty());
    }
    dialog(&mut app).fields.insert("name".into(), json!("UI Composite B"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!(names(&app), ["Composite Font 1", "UI Composite B"]);
}

/// The menu item shows only with Show East Asian Options on, and needs a document.
#[test]
fn the_menu_item_shows_with_east_asian_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.show_east_asian_options = true;
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})).map(|s| s.0), Some(false));
    app.run("file.new", json!({})).unwrap();
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})).map(|s| s.0), Some(true));
    app.session.prefs.show_east_asian_options = false;
    assert_eq!(crate::menus::shown_state(&app, COMMAND, &json!({})), None);
}

/// Export… writes the font picked to a file; Import… adds the fonts of a file to the dialog's,
/// replacing one of the same name, and opening the file adds them to the document.
#[test]
fn composite_fonts_go_through_files() {
    let dir = std::env::temp_dir().join(format!("vc-composite-fonts-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fonts.vccomposite").to_string_lossy().to_string();
    let (saved, picked) = (file.clone(), file.clone());
    let services = crate::Services {
        pick_save: Some(Box::new(move |_: &FilePick| Some(saved.clone()))),
        pick_open: Some(Box::new(move |_: &FilePick| Some(picked.clone()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    open(&mut app).unwrap();
    press(&mut app, Action::New);
    let d = dialog(&mut app);
    let mut list = fonts(d);
    list[0].name = "UI Composite IO".into();
    list[0].parts.roman.baseline = -12.0;
    put_fonts(d, &list);
    press(&mut app, Action::Export);
    assert!(std::fs::read_to_string(&file).unwrap().contains("UI Composite IO"));
    // Import into the dialog: the same name is replaced, not added.
    let d = dialog(&mut app);
    let mut list = fonts(d);
    list[0].parts.roman.baseline = 0.0;
    put_fonts(d, &list);
    press(&mut app, Action::Import);
    let list = fonts(dialog(&mut app));
    assert_eq!((list.len(), list[0].parts.roman.baseline), (1, -12.0));
    super::super::cancel(&mut app);
    assert!(names(&app).is_empty());
    // Opening the file adds its fonts to the document.
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    assert_eq!(names(&app), ["UI Composite IO"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The dialog lists the fonts and draws its table without a document font picked too.
#[test]
fn the_dialog_draws_with_and_without_fonts() {
    let mut app = app();
    open(&mut app).unwrap();
    frame(&mut app);
    assert_eq!(dialog(&mut app).fields.get("fonts"), Some(&Value::Array(vec![])));
    press(&mut app, Action::New);
    frame(&mut app);
    assert_eq!(fonts(dialog(&mut app)).len(), 1);
}

fn try_press(app: &mut VectorcraftApp, act: Action) -> Result<(), String> {
    let mut d = app.ui.dialog.clone().unwrap();
    let r = run(app, &mut d, act);
    app.ui.dialog = Some(d);
    r
}

fn sets(app: &mut VectorcraftApp) -> Vec<CustomCharSet> {
    fonts(dialog(app)).first().map(|f| f.custom.clone()).unwrap_or_default()
}

/// Custom shows the font's custom character sets: New adds a set (based on the one picked), Add
/// puts typed characters or the characters of Unicode codes into it once each, Delete takes the
/// picked character out, Delete Set takes the set out; OK sets them with the font as one step.
#[test]
fn custom_character_sets_are_edited_in_the_dialog() {
    let mut app = app();
    open(&mut app).unwrap();
    press(&mut app, Action::New);
    press(&mut app, Action::Custom);
    assert!(dialog(&mut app).bool("custom"));
    frame(&mut app);
    press(&mut app, Action::NewSet);
    frame(&mut app);
    assert_eq!(dialog(&mut app).str("setName"), "Character Set 1");
    let kanji = fonts(dialog(&mut app))[0].parts.kanji.clone();
    assert_eq!(sets(&mut app)[0].part, kanji);
    // Direct Entry adds each character once.
    dialog(&mut app).fields.insert("entry".into(), json!("※A※"));
    press(&mut app, Action::AddChars);
    assert_eq!(sets(&mut app)[0].characters, "※A");
    assert_eq!(dialog(&mut app).str("entry"), "");
    // Unicode takes codes, with or without U+, and skips characters already in the set.
    dialog(&mut app).fields.insert("entryMode".into(), json!("unicode"));
    dialog(&mut app).fields.insert("entry".into(), json!("U+3042, 41"));
    press(&mut app, Action::AddChars);
    assert_eq!(sets(&mut app)[0].characters, "※Aあ");
    for bad in ["zz", "1F600", "0009"] {
        dialog(&mut app).fields.insert("entry".into(), json!(bad));
        assert!(try_press(&mut app, Action::AddChars).is_err(), "{bad}");
        assert_eq!(sets(&mut app)[0].characters, "※Aあ");
    }
    dialog(&mut app).fields.insert("entryMode".into(), json!("direct"));
    dialog(&mut app).fields.insert("entry".into(), json!("B😀"));
    assert!(try_press(&mut app, Action::AddChars).is_err());
    assert_eq!(sets(&mut app)[0].characters, "※Aあ");
    frame(&mut app);
    // Delete takes the picked character out.
    dialog(&mut app).fields.insert("char".into(), json!(0));
    press(&mut app, Action::DeleteChar);
    assert_eq!(sets(&mut app)[0].characters, "Aあ");
    // The set's name is edited in Name; New is based on the set picked.
    dialog(&mut app).fields.insert("setName".into(), json!("Letters"));
    frame(&mut app);
    press(&mut app, Action::NewSet);
    let list = sets(&mut app);
    assert_eq!(list.iter().map(|s| (s.name.as_str(), s.characters.as_str())).collect::<Vec<_>>(), [("Letters", "Aあ"), ("Character Set 1", "Aあ")]);
    assert_eq!(dialog(&mut app).fields.get("set"), Some(&json!(1)));
    press(&mut app, Action::DeleteSet);
    assert_eq!(sets(&mut app).len(), 1);
    assert_eq!(dialog(&mut app).str("setName"), "Letters");
    frame(&mut app);
    dialog(&mut app).fields.insert("name".into(), json!("UI Composite Sets"));
    frame(&mut app);
    let before = steps(&app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!(steps(&app), before + 1);
    let f = &app.session.active().unwrap().doc.composite_fonts[0];
    assert_eq!((f.custom.len(), f.custom[0].name.as_str(), f.custom[0].characters.as_str()), (1, "Letters", "Aあ"));
}

/// `ui.dialog.set` takes `custom`, `entry` and `char` as soon as the dialog opens, before Custom is
/// clicked or anything is typed, and Add then adds the characters it set.
#[test]
fn custom_set_fields_are_set_right_after_open() {
    let mut app = app();
    let ctx = egui::Context::default();
    let font =
        json!({"name": "UI Composite Open", "parts": {"kanji": {"font": "Inter"}}, "custom": [{"name": "Marks", "characters": "", "font": "Inter"}]});
    app.run(COMMAND, json!({ "fonts": [font] })).unwrap();
    open(&mut app).unwrap();
    for (field, value) in [("custom", json!(true)), ("entry", json!("※〒")), ("char", json!(0))] {
        let (req, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"field": field, "value": value}));
        let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("not done") };
        assert!(r.get("error").is_none(), "{field}: {r}");
        assert_eq!(dialog(&mut app).fields.get(field), Some(&value), "{field}");
    }
    frame(&mut app);
    press(&mut app, Action::AddChars);
    assert_eq!(sets(&mut app)[0].characters, "※〒");
}

/// Two sets of the same name keep the dialog open and change nothing.
#[test]
fn custom_sets_sharing_a_name_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    press(&mut app, Action::New);
    press(&mut app, Action::NewSet);
    press(&mut app, Action::NewSet);
    dialog(&mut app).fields.insert("setName".into(), json!("character set 1"));
    dialog(&mut app).fields.insert("custom".into(), json!(true));
    frame(&mut app);
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some());
    assert!(names(&app).is_empty());
}

/// A composite font set in Inter, with its Roman part as `roman` changes it.
fn inter_font(roman: impl FnOnce(&mut vectorcraft_doc::CompositePart)) -> CompositeFont {
    let mut parts = vectorcraft_doc::CompositeParts::all(vectorcraft_doc::CompositePart::new("Inter", "Regular"));
    roman(&mut parts.roman);
    CompositeFont { name: "UI Composite Sample".into(), parts, custom: Vec::new() }
}

/// Show Sample opens hidden at 100% with the ICF box, em box and baseline shown; its button shows
/// and hides the sample, which draws with every line shown and at every zoom; nothing changes in
/// the document.
#[test]
fn show_sample_shows_and_hides_the_sample() {
    let mut app = app();
    open(&mut app).unwrap();
    let d = dialog(&mut app);
    assert!(!d.bool("sample"));
    assert_eq!(d.fields.get("zoom"), Some(&json!(100)));
    let shown: Vec<bool> = sample::Mark::ALL.iter().map(|m| d.bool(m.field())).collect();
    assert_eq!(shown, [true, true, true, false, false, false, false]);
    press(&mut app, Action::New);
    press(&mut app, Action::Sample);
    assert!(dialog(&mut app).bool("sample"));
    frame(&mut app);
    for m in sample::Mark::ALL {
        dialog(&mut app).fields.insert(m.field().into(), json!(true));
    }
    for z in sample::ZOOMS {
        dialog(&mut app).fields.insert("zoom".into(), json!(z));
        frame(&mut app);
    }
    press(&mut app, Action::Sample);
    assert!(!dialog(&mut app).bool("sample"));
    super::super::cancel(&mut app);
    assert!(names(&app).is_empty());
}

/// `ui.dialog.set` takes `sample`, `zoom` and the line fields; a zoom outside the Zoom menu or a
/// line that is not true or false is refused and leaves the dialog as it was.
#[test]
fn sample_fields_are_set_and_checked() {
    let mut app = app();
    let ctx = egui::Context::default();
    open(&mut app).unwrap();
    let set = |app: &mut VectorcraftApp, field: &str, value: Value| {
        let (req, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"field": field, "value": value}));
        let crate::control::Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
        r
    };
    for (field, value) in [("sample", json!(true)), ("zoom", json!(400)), ("lineCapHeight", json!(true)), ("lineBaseline", json!(false))] {
        let r = set(&mut app, field, value.clone());
        assert!(r.get("error").is_none(), "{field}: {r}");
        assert_eq!(dialog(&mut app).fields.get(field), Some(&value), "{field}");
    }
    for (field, value) in [("zoom", json!(30)), ("zoom", json!("200")), ("sample", json!(1)), ("lineXHeight", json!("yes"))] {
        let r = set(&mut app, field, value.clone());
        assert!(r.get("error").is_some(), "{field} {value}: {r}");
    }
    let d = dialog(&mut app);
    assert_eq!((d.fields.get("zoom"), d.bool("sample"), d.bool("lineXHeight")), (Some(&json!(400)), true, false));
}

/// The sample's lines: the em box and ICF box of the face the Kanji are set in (a face without
/// ideographs centers its em box 0.38 em above the baseline), the cap height and x-height of the
/// Roman part's face raised by its baseline and scaled by its vertical scale, the lowest descent
/// and highest ascent, and the top of the tallest glyph; all of them grow with the size.
#[test]
fn sample_lines_follow_the_parts() {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let (path, h) = sample::measure(&inter_font(|_| {}), 36.0);
    assert!(!path.elements().is_empty());
    let (bottom, top) = h.em_box.unwrap();
    assert!(close(bottom, -0.12 * 36.0) && close(top, 0.88 * 36.0), "{h:?}");
    let (ib, it) = h.icf_box.unwrap();
    assert!(ib >= bottom && it <= top && ib < it, "{h:?}");
    let (cap, x) = (h.cap_height.unwrap(), h.x_height.unwrap());
    assert!(0.0 < x && x < cap && cap < top, "{h:?}");
    let (low, high) = h.ascent_descent.unwrap();
    assert!(low < 0.0 && high > cap, "{h:?}");
    assert!(h.ascender.unwrap() > x, "{h:?}");
    // The Roman part raised by 10% and squeezed to half height moves its lines; the Kanji's stay.
    let (_, raised) = sample::measure(
        &inter_font(|p| {
            p.baseline = 10.0;
            p.v_scale = 50.0;
        }),
        36.0,
    );
    assert!(close(raised.cap_height.unwrap(), 3.6 + cap * 0.5), "{raised:?}");
    assert!(close(raised.x_height.unwrap(), 3.6 + x * 0.5), "{raised:?}");
    assert_eq!(raised.em_box, h.em_box);
    // At 200% every line is twice as far from the baseline.
    let (_, double) = sample::measure(&inter_font(|_| {}), 72.0);
    assert!(close(double.cap_height.unwrap(), cap * 2.0) && close(double.em_box.unwrap().1, top * 2.0), "{double:?}");
}

/// The ICF box is the Kanji face's ideographic character face inside its em box, and a Roman face
/// without an OS/2 cap height still gets a Cap Height line, estimated from its ascent.
#[test]
fn sample_lines_read_the_icf_box_and_estimate_a_missing_cap_height() {
    use vectorcraft_text::test_fonts::{HEIGHT_FAMILY, HEIGHTLESS_FAMILY, height_font};
    let db = vectorcraft_text::FontDb::global();
    db.add_font(height_font(true).expect("the test font builds"));
    db.add_font(height_font(false).expect("the test font builds"));
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let mut f = inter_font(|p| p.font = HEIGHTLESS_FAMILY.into());
    f.parts.kanji.font = HEIGHT_FAMILY.into();
    let (_, h) = sample::measure(&f, 36.0);
    let (bottom, top) = h.icf_box.unwrap();
    assert!(close(bottom, -0.07 * 36.0) && close(top, 0.83 * 36.0), "{h:?}");
    let cap = h.cap_height.expect("an estimated cap height");
    assert!(cap > h.x_height.unwrap() && cap < h.ascent_descent.unwrap().1, "{h:?}");
}

/// A character its part's font lacks is drawn in a fallback face, and the lines measure that face:
/// in a font whose box runs to 0.9 em and which maps no `A`, `g`, `h`, `1` or `2`, the highest
/// ascent is the fallback face's.
#[test]
fn sample_lines_measure_the_faces_that_draw_the_glyphs() {
    use vectorcraft_text::test_fonts::{PATH_ALIGN_HHEA_FAMILY, path_align_font};
    vectorcraft_text::FontDb::global().add_font(path_align_font(PATH_ALIGN_HHEA_FAMILY).expect("the test font builds"));
    let mut f = inter_font(|_| {});
    f.parts = vectorcraft_doc::CompositeParts::all(vectorcraft_doc::CompositePart::new(PATH_ALIGN_HHEA_FAMILY, "Regular"));
    let (_, h) = sample::measure(&f, 36.0);
    let (_, high) = h.ascent_descent.unwrap();
    assert!(high > 0.9 * 36.0 + 0.5, "{h:?}");
}

/// The lines follow a vertical scale as the glyphs do: a custom set that holds the first Kanji at
/// half height halves the em box about its baseline, and Kana scaled at center keep the middle of
/// their em box, so their ascent moves up by the half they lose below it.
#[test]
fn sample_lines_follow_vertical_scale() {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let mut f = inter_font(|_| {});
    let mut half = vectorcraft_doc::CompositePart::new("Inter", "Regular");
    half.v_scale = 50.0;
    f.custom = vec![CustomCharSet { name: "Half".into(), characters: "永".into(), part: half }];
    let (_, h) = sample::measure(&f, 36.0);
    let (bottom, top) = h.em_box.unwrap();
    assert!(close(bottom, (0.38 - 0.5) * 18.0) && close(top, (0.38 + 0.5) * 18.0), "{h:?}");
    let kana = |at_center: bool| {
        let mut f = inter_font(|_| {});
        f.parts.kana.size = 300.0;
        f.parts.kana.v_scale = 50.0;
        f.parts.kana.scale_at_center = at_center;
        sample::measure(&f, 36.0).1.ascent_descent.unwrap()
    };
    // The Kana, at three times the size, give the highest ascent either way.
    let ((_, high), (_, raised)) = (kana(false), kana(true));
    assert!(close(raised, high + 0.38 * 108.0 * 0.5), "{high} {raised}");
}

/// The sample adds the first character of each custom set it does not hold yet, and a font whose
/// sets can't be used yet (two sharing a name while it is typed) still draws, without them.
#[test]
fn the_sample_takes_the_custom_sets() {
    let mut f = inter_font(|_| {});
    let set = |name: &str, characters: &str| CustomCharSet {
        name: name.into(),
        characters: characters.into(),
        part: vectorcraft_doc::CompositePart::new("Inter", "Regular"),
    };
    f.custom = vec![set("Marks", "〒※"), set("Letters", "A")];
    assert_eq!(sample::text(&f), format!("{}〒", sample::TEXT));
    f.custom = vec![set("Marks", "〒"), set("marks", "〒")];
    let (path, h) = sample::measure(&f, 36.0);
    assert!(!path.elements().is_empty() && h.cap_height.is_some());
}

/// The sample draws a color font's glyphs in their own colors, not in the dialog's text color.
#[test]
fn the_sample_draws_color_glyphs_in_their_colors() {
    use vectorcraft_text::test_fonts::{COLOUR_CHAR, colr_family, colr_font};
    vectorcraft_text::FontDb::global().add_font(colr_font(false).expect("the test font builds"));
    let mut f = inter_font(|_| {});
    let mut colr = vectorcraft_doc::CompositePart::new(colr_family(false), "Regular");
    colr.size = 300.0;
    f.custom = vec![CustomCharSet { name: "Color".into(), characters: COLOUR_CHAR.to_string(), part: colr }];
    let img = sample::draw(&f, 36.0, 120.0, (900, 160), 1.0, egui::Color32::BLACK);
    let red = (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).any(|(x, y)| {
        let p = img.pixel(x, y);
        p[0] > 200 && p[1] < 60 && p[2] < 60 && p[3] > 200
    });
    assert!(red, "the glyph's red layer is drawn");
    // Max Ascender reaches the top of the color glyph, three times the size of the rest.
    let (_, plain) = sample::measure(&inter_font(|_| {}), 36.0);
    let (_, tall) = sample::measure(&f, 36.0);
    assert!(tall.ascender.unwrap() > plain.ascender.unwrap() * 1.5, "{plain:?} {tall:?}");
}
