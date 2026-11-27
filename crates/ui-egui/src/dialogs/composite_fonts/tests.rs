use craft::ui::dialog::{HeldOk, dialog_id};
use craft::ui::patterns::presets::PresetCommands;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;
// PLAN: assumed fl.A20 lives in `crate::tests::picks` (files-w7): `scripted(&mut app) -> Picker`, `poll(&mut app, &ctx)`.
use crate::tests::picks::{poll, scripted};

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

fn ctx(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the app has a context")
}

/// Runs `f` on the open dialog.
fn dialog<R>(app: &mut VectorcraftApp, f: impl FnOnce(&mut CompositeFontsDialog) -> R) -> R {
    with_dialog(app, f).expect("Composite Fonts open")
}

fn is_open(app: &VectorcraftApp) -> bool {
    with_dialog(app, |_| ()).is_some()
}

/// OK as the user gives it: the held OK is filed, [`pump`] runs the command and answers it, and an accepted
/// answer closes the dialog. The command's refusal comes back as the error and keeps the dialog open.
fn ok(app: &mut VectorcraftApp) -> Result<(), String> {
    let (ctx, id) = (ctx(app), dialog_id(KIND));
    let dialogs = app.dialogs().expect("a dialog host");
    dialogs.with(|host| host.confirm_by_id(&ctx, id));
    pump(app, &ctx);
    match dialogs.with(|host| host.held_state(id)) {
        Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_owned()),
        _ => {
            assert!(dialogs.with(|host| host.confirm_by_id(&ctx, id)), "the accepted OK closes the dialog");
            Ok(())
        }
    }
}

fn cancel(app: &mut VectorcraftApp) {
    let (ctx, id) = (ctx(app), dialog_id(KIND));
    app.dialogs().expect("a dialog host").with(|host| host.cancel_by_id(&ctx, id));
}

/// The file dialog the Composite Font row's `command` asks for, for the font named `key`, taken by [`pump`].
fn request(app: &mut VectorcraftApp, command: PresetCommands, key: Option<&str>) {
    let key = key.map(str::to_owned);
    let asked = dialog(app, |d| files(&d.fonts).request(command, key.as_ref()));
    assert!(asked.is_some(), "the entry asks for a file dialog");
    dialog(app, |d| d.file = asked);
    let ctx = ctx(app);
    pump(app, &ctx);
}

fn names(app: &VectorcraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.composite_fonts.iter().map(|f| f.name.clone()).collect()
}

#[test]
fn delayed_load_and_replace_leave_the_reopened_composite_fonts_draft_alone() {
    let choices: [(PresetCommands, &[&str]); 2] = [(PresetCommands::Load, &["New draft", "From file"]), (PresetCommands::Replace, &["From file"])];
    for (command, expected_live) in choices {
        let mut app = app();
        let picker = scripted(&mut app);
        let ctx = ctx(&app);
        open(&mut app).unwrap();
        let bytes = dialog(&mut app, |d| {
            d.new_font("From file".into());
            file_bytes(&d.fonts).unwrap()
        });
        let dir = vectorcraft_testkit::TempDir::new("composite-fonts-late-picker");
        let file = dir.join("picked.vccomposite");
        std::fs::write(&file, bytes).unwrap();
        let path = file.to_str().unwrap();
        request(&mut app, command, None);
        assert_eq!(picker.requests().len(), 1);
        cancel(&mut app);
        open(&mut app).unwrap();
        dialog(&mut app, |d| d.new_font("New draft".into()));

        picker.answer(&[path]);
        poll(&mut app, &ctx);
        let draft_names = |app: &mut VectorcraftApp| dialog(app, |d| d.fonts.iter().map(|font| font.name.clone()).collect::<Vec<_>>());
        assert_eq!(draft_names(&mut app), ["New draft"], "the stale picker leaves the new draft alone");

        request(&mut app, command, None);
        assert_eq!(picker.requests().len(), 2);
        picker.answer(&[path]);
        poll(&mut app, &ctx);
        assert_eq!(draft_names(&mut app), expected_live, "a picker opened by the new draft still works");
        assert!(names(&app).is_empty(), "a picker only edits the draft");
    }
}

#[test]
fn load_and_replace_accept_a_browser_picked_file() {
    let choices: [(PresetCommands, &[&str]); 2] = [(PresetCommands::Load, &["Draft", "From Web"]), (PresetCommands::Replace, &["From Web"])];
    for (command, expected) in choices {
        let mut app = app();
        let picker = scripted(&mut app);
        let ctx = ctx(&app);
        open(&mut app).unwrap();
        let bytes = dialog(&mut app, |d| {
            d.new_font("From Web".into());
            file_bytes(&d.fonts).unwrap()
        });
        cancel(&mut app);
        open(&mut app).unwrap();
        dialog(&mut app, |d| d.new_font("Draft".into()));
        let dialogs = app.dialogs().unwrap();
        ctx.run_ui(egui::RawInput::default(), |ui| dialogs.with(|host| host.run(ui))).textures_delta.clear();
        let step = steps(&app);

        request(&mut app, command, None);
        picker.answer_contents(&[("picked.vccomposite", &bytes)]);
        poll(&mut app, &ctx);
        let draft_names = dialog(&mut app, |d| d.fonts.iter().map(|font| font.name.clone()).collect::<Vec<_>>());
        assert_eq!(draft_names, expected, "{}", app.ui.status);
        assert!(app.ui.status.contains("picked.vccomposite"), "{}", app.ui.status);
        assert!(names(&app).is_empty(), "picking a file edits only the draft");
        assert_eq!(steps(&app), step);
    }
}

/// Load and Replace refuse an oversized browser file while preserving both the draft and artwork;
/// a later valid file still reaches that same draft.
#[test]
fn oversized_browser_composite_font_files_leave_the_draft_unchanged() {
    for command in [PresetCommands::Load, PresetCommands::Replace] {
        let mut app = app();
        let picker = scripted(&mut app);
        let ctx = ctx(&app);
        open(&mut app).unwrap();
        dialog(&mut app, |d| d.new_font("Draft".into()));
        let dialogs = app.dialogs().unwrap();
        ctx.run_ui(egui::RawInput::default(), |ui| dialogs.with(|host| host.run(ui))).textures_delta.clear();
        let before = dialog(&mut app, |d| d.fonts.clone());
        let bytes = file_bytes(&before).unwrap();
        let step = steps(&app);
        let large = vec![b' '; MAX_FILE_BYTES + 1];
        request(&mut app, command, None);
        picker.answer_contents(&[("large.vccomposite", &large)]);
        poll(&mut app, &ctx);
        assert_eq!(
            app.ui.status,
            vectorcraft_engine::EngineError::BadParams { cmd: IMPORT.into(), msg: "the file is larger than 4 MB".into() }.to_string()
        );
        assert_eq!(dialog(&mut app, |d| d.fonts.clone()), before);
        assert!(names(&app).is_empty());
        assert_eq!(steps(&app), step);
        request(&mut app, command, None);
        picker.answer_contents(&[("valid.vccomposite", &bytes)]);
        poll(&mut app, &ctx);
        assert!(app.ui.status.contains("valid.vccomposite"), "{}", app.ui.status);
        assert_eq!(dialog(&mut app, |d| d.fonts.clone()), before);
        assert!(names(&app).is_empty());
        assert_eq!(steps(&app), step);
    }
}

fn steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// The row of `class` in the parts table.
fn row(class: CharClass) -> usize {
    CharClass::ALL.iter().position(|c| *c == class).unwrap()
}

/// A new font named as the Composite Font row suggests, based on the one picked.
fn new_font(app: &mut VectorcraftApp) {
    dialog(app, |d| d.new_font(new_name(&d.fonts)));
}

/// A new custom set named as the Character Set row suggests, based on the one picked.
fn new_set(app: &mut VectorcraftApp) {
    dialog(app, |d| {
        let name = new_set_name(&d.font().map(|f| f.custom.clone()).unwrap_or_default());
        d.edit_sets(SetAction::New(name))
    })
    .unwrap();
}

fn sets(app: &mut VectorcraftApp) -> Vec<CustomCharSet> {
    dialog(app, |d| d.font().map(|f| f.custom.clone()).unwrap_or_default())
}

fn edit(app: &mut VectorcraftApp, act: SetAction) -> Result<(), String> {
    dialog(app, |d| d.edit_sets(act))
}

/// Type › Composite Fonts… opens on the document's fonts; New adds one, its parts edited in the
/// table; nothing changes until OK, which sets them as one undo step; Cancel leaves them.
#[test]
fn composite_fonts_change_on_ok_as_one_undo_step() {
    let mut app = app();
    crate::menus::invoke(&mut app, COMMAND, json!({}));
    assert!(is_open(&app));
    new_font(&mut app);
    dialog(&mut app, |d| {
        assert_eq!(d.font().unwrap().name, "Composite Font 1");
        d.edit_part(row(CharClass::Roman), PartEdit::Font("JetBrains Mono".into(), None));
        d.edit_part(row(CharClass::Roman), PartEdit::Size(80.0));
        d.edit_part(row(CharClass::Kana), PartEdit::ScaleAtCenter(true));
        d.rename_font("Composite Font 1", "UI Composite A".into());
    });
    assert!(names(&app).is_empty());
    let before = steps(&app);
    ok(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(names(&app), ["UI Composite A"]);
    assert_eq!(steps(&app), before + 1);
    let f = &app.session.active().unwrap().doc.composite_fonts[0];
    assert_eq!((f.parts.roman.font.as_str(), f.parts.roman.size, f.parts.kana.scale_at_center), ("JetBrains Mono", 80.0, true));
    // OK with nothing changed adds no step; Cancel leaves the fonts.
    open(&mut app).unwrap();
    ok(&mut app).unwrap();
    assert_eq!(steps(&app), before + 1);
    open(&mut app).unwrap();
    dialog(&mut app, CompositeFontsDialog::delete_font);
    cancel(&mut app);
    assert_eq!(names(&app), ["UI Composite A"]);
    // Delete takes the font out and OK sets none.
    open(&mut app).unwrap();
    dialog(&mut app, |d| {
        d.delete_font();
        assert!(d.font().is_none());
    });
    ok(&mut app).unwrap();
    assert!(names(&app).is_empty());
}

#[test]
fn a_held_composite_fonts_ok_keeps_the_draft_with_its_opening_document() {
    let mut app = app();
    app.run("file.new", json!({"width": 440, "height": 440})).unwrap();
    open(&mut app).unwrap();
    dialog(&mut app, |d| d.new_font("Destination only".into()));
    ok(&mut app).unwrap();
    let destination = (names(&app), steps(&app));

    app.run("document.activate", json!({"index": 0})).unwrap();
    let source = (names(&app), steps(&app));
    open(&mut app).unwrap();
    dialog(&mut app, |d| d.new_font("From the source".into()));
    let ctx = ctx(&app);
    let id = dialog_id(KIND);
    let dialogs = app.dialogs().unwrap();
    dialogs.with(|host| {
        assert!(!host.confirm_by_id(&ctx, id));
        assert_eq!(host.held_state(id), Some(HeldOk::Waiting));
    });

    app.run("document.activate", json!({"index": 1})).unwrap();
    pump(&mut app, &ctx);
    assert_eq!((names(&app), steps(&app)), destination, "the other document keeps its fonts and undo history");
    assert!(matches!(dialogs.with(|host| host.held_state(id)), Some(HeldOk::Refused(_))));
    assert!(is_open(&app));

    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!((names(&app), steps(&app)), source);
    ok(&mut app).unwrap();
    assert_eq!(names(&app), ["From the source"]);
    assert_eq!(steps(&app), source.1 + 1);
    assert!(!is_open(&app));
}

/// New is based on the font picked and takes the next free name; a name two fonts share, or one
/// left empty, keeps the dialog open and changes nothing.
#[test]
fn shared_or_empty_names_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    new_font(&mut app);
    dialog(&mut app, |d| d.fonts[0].parts.numerals.font = "Inter".into());
    new_font(&mut app);
    let list = dialog(&mut app, |d| d.fonts.clone());
    assert_eq!(list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Composite Font 1", "Composite Font 2"]);
    assert_eq!(list[1].parts.numerals.font, "Inter");
    // The name row refuses what the command would.
    let refused = |name: &str| name_error(list.iter().map(|f| f.name.as_str()), name);
    assert!(matches!(refused("composite font 1"), Some(PresetError::NameTaken)));
    assert!(matches!(refused("  "), Some(PresetError::InvalidName)));
    assert!(refused("UI Composite B").is_none());
    for bad in ["composite font 1", "  "] {
        dialog(&mut app, |d| d.fonts[1].name = bad.into());
        assert!(ok(&mut app).is_err(), "{bad:?}");
        assert!(is_open(&app));
        assert!(names(&app).is_empty());
    }
    dialog(&mut app, |d| d.fonts[1].name = "UI Composite B".into());
    ok(&mut app).unwrap();
    assert_eq!(names(&app), ["Composite Font 1", "UI Composite B"]);
}

/// Save… writes the font picked to a file; Load… adds the fonts of a file to the dialog's, replacing one
/// of the same name; a cancelled pick changes nothing; and opening the file adds them to the document.
#[test]
fn composite_fonts_go_through_files() {
    let dir = std::env::temp_dir().join(format!("vc-composite-fonts-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fonts.vccomposite").to_string_lossy().to_string();
    let mut app = app();
    let picker = scripted(&mut app);
    let ctx = ctx(&app);
    open(&mut app).unwrap();
    dialog(&mut app, |d| {
        d.new_font("UI Composite IO".into());
        d.fonts[0].parts.roman.baseline = -12.0;
    });
    request(&mut app, PresetCommands::Save, Some("UI Composite IO"));
    picker.answer_saved(&file, None);
    poll(&mut app, &ctx);
    let saved = picker.file(&file).unwrap();
    assert!(String::from_utf8_lossy(&saved).contains("UI Composite IO"));
    // The file service holds what it saved; Load… reads the file from disk.
    std::fs::write(&file, &saved).unwrap();
    // Load: the same name is replaced, not added.
    dialog(&mut app, |d| d.fonts[0].parts.roman.baseline = 0.0);
    request(&mut app, PresetCommands::Load, None);
    picker.answer(&[file.as_str()]);
    poll(&mut app, &ctx);
    let list = dialog(&mut app, |d| d.fonts.clone());
    assert_eq!((list.len(), list[0].parts.roman.baseline), (1, -12.0));
    // A cancelled pick leaves the fonts.
    dialog(&mut app, |d| d.fonts[0].parts.roman.baseline = 5.0);
    request(&mut app, PresetCommands::Load, None);
    picker.cancel();
    poll(&mut app, &ctx);
    assert_eq!(dialog(&mut app, |d| d.fonts[0].parts.roman.baseline), 5.0);
    // Replace takes the fonts of the file in place of the dialog's.
    dialog(&mut app, |d| d.new_font("UI Composite Extra".into()));
    request(&mut app, PresetCommands::Replace, None);
    picker.answer(&[file.as_str()]);
    poll(&mut app, &ctx);
    let list = dialog(&mut app, |d| d.fonts.clone());
    assert_eq!((list.len(), list[0].name.as_str(), list[0].parts.roman.baseline), (1, "UI Composite IO", -12.0));
    cancel(&mut app);
    assert!(names(&app).is_empty());
    // Opening the file adds its fonts to the document.
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    assert_eq!(names(&app), ["UI Composite IO"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Custom shows the font's custom character sets: New adds a set (based on the one picked), Add
/// puts typed characters or the characters of Unicode codes into it once each, Delete takes the
/// picked character out, Delete Set takes the set out; OK sets them with the font as one step.
#[test]
fn custom_character_sets_are_edited_in_the_dialog() {
    let mut app = app();
    open(&mut app).unwrap();
    new_font(&mut app);
    dialog(&mut app, |d| d.custom = true);
    new_set(&mut app);
    assert_eq!(sets(&mut app)[0].name, "Character Set 1");
    let kanji = dialog(&mut app, |d| d.fonts[0].parts.kanji.clone());
    assert_eq!(sets(&mut app)[0].part, kanji);
    // Direct Entry adds each character once.
    dialog(&mut app, |d| d.entry = "※A※".into());
    edit(&mut app, SetAction::AddChars).unwrap();
    assert_eq!(sets(&mut app)[0].characters, "※A");
    assert_eq!(dialog(&mut app, |d| d.entry.clone()), "");
    // Unicode takes codes, with or without U+, and skips characters already in the set.
    dialog(&mut app, |d| {
        d.entry_mode = EntryMode::Unicode;
        d.entry = "U+3042, 41".into();
    });
    edit(&mut app, SetAction::AddChars).unwrap();
    assert_eq!(sets(&mut app)[0].characters, "※Aあ");
    for bad in ["zz", "1F600", "0009"] {
        dialog(&mut app, |d| d.entry = bad.into());
        assert!(edit(&mut app, SetAction::AddChars).is_err(), "{bad}");
        assert_eq!(sets(&mut app)[0].characters, "※Aあ");
    }
    dialog(&mut app, |d| {
        d.entry_mode = EntryMode::Direct;
        d.entry = "B😀".into();
    });
    assert!(edit(&mut app, SetAction::AddChars).is_err());
    assert_eq!(sets(&mut app)[0].characters, "※Aあ");
    // Delete takes the picked character out.
    dialog(&mut app, |d| d.picked_char = Some(0));
    edit(&mut app, SetAction::DeleteChar).unwrap();
    assert_eq!(sets(&mut app)[0].characters, "Aあ");
    assert_eq!(dialog(&mut app, |d| d.picked_char), None);
    // The set's name is edited through its field; New is based on the set picked.
    let (ctx, id) = (ctx(&app), dialog_id(KIND));
    app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, "setName", json!("Letters"))).unwrap();
    new_set(&mut app);
    let list = sets(&mut app);
    assert_eq!(list.iter().map(|s| (s.name.as_str(), s.characters.as_str())).collect::<Vec<_>>(), [("Letters", "Aあ"), ("Character Set 1", "Aあ")]);
    assert_eq!(dialog(&mut app, |d| d.set), 1);
    edit(&mut app, SetAction::Delete).unwrap();
    assert_eq!(sets(&mut app).len(), 1);
    assert_eq!(dialog(&mut app, |d| d.json_fields()).unwrap()["setName"], "Letters");
    dialog(&mut app, |d| d.rename_font("Composite Font 1", "UI Composite Sets".into()));
    let before = steps(&app);
    ok(&mut app).unwrap();
    assert_eq!(steps(&app), before + 1);
    let f = &app.session.active().unwrap().doc.composite_fonts[0];
    assert_eq!((f.custom.len(), f.custom[0].name.as_str(), f.custom[0].characters.as_str()), (1, "Letters", "Aあ"));
}

/// A set is refused past the most a font keeps, with nothing changed.
#[test]
fn a_set_past_the_most_is_refused() {
    let mut app = app();
    open(&mut app).unwrap();
    new_font(&mut app);
    for _ in 0..CUSTOM_SETS_MAX {
        new_set(&mut app);
    }
    assert!(edit(&mut app, SetAction::New("One too many".into())).is_err());
    assert_eq!(sets(&mut app).len(), CUSTOM_SETS_MAX);
}

/// Two sets of the same name keep the dialog open and change nothing.
#[test]
fn custom_sets_sharing_a_name_keep_the_dialog_open() {
    let mut app = app();
    open(&mut app).unwrap();
    new_font(&mut app);
    new_set(&mut app);
    new_set(&mut app);
    let (ctx, id) = (ctx(&app), dialog_id(KIND));
    app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, "setName", json!("character set 1"))).unwrap();
    assert!(ok(&mut app).is_err());
    assert!(is_open(&app));
    assert!(names(&app).is_empty());
}

/// A composite font set in Inter, with its Roman part as `roman` changes it.
fn inter_font(roman: impl FnOnce(&mut vectorcraft_doc::CompositePart)) -> CompositeFont {
    let mut parts = vectorcraft_doc::CompositeParts::all(vectorcraft_doc::CompositePart::new("Inter", "Regular"));
    roman(&mut parts.roman);
    CompositeFont { name: "UI Composite Sample".into(), parts, custom: Vec::new() }
}

/// `ui.dialog.set` takes `sample`, `zoom` and the line fields; a zoom outside the Zoom menu or a
/// line that is not true or false is refused and leaves the dialog as it was.
#[test]
fn sample_fields_are_set_and_checked() {
    let mut app = app();
    open(&mut app).unwrap();
    let (ctx, id) = (ctx(&app), dialog_id(KIND));
    let dialogs = app.dialogs().unwrap();
    let set = |field: &str, value: Value| dialogs.with(|host| host.set_json_field(&ctx, id, field, value));
    let before = dialog(&mut app, |d| d.json_fields()).unwrap();
    assert_eq!((before["sample"].as_bool(), before["zoom"].as_u64()), (Some(false), Some(100)));
    let shown: Vec<bool> = sample::Mark::ALL.iter().map(|m| before[m.field()].as_bool().unwrap()).collect();
    assert_eq!(shown, [true, true, true, false, false, false, false]);
    for (field, value) in [("sample", json!(true)), ("zoom", json!(400)), ("lineCapHeight", json!(true)), ("lineBaseline", json!(false))] {
        set(field, value.clone()).unwrap_or_else(|e| panic!("{field}: {e}"));
        assert_eq!(dialog(&mut app, |d| d.json_fields()).unwrap()[field], value, "{field}");
    }
    for (field, value) in [("zoom", json!(30)), ("zoom", json!("200")), ("sample", json!(1)), ("lineXHeight", json!("yes"))] {
        assert!(set(field, value.clone()).is_err(), "{field} {value}");
    }
    let after = dialog(&mut app, |d| d.json_fields()).unwrap();
    assert_eq!((after["zoom"].as_u64(), after["sample"].as_bool(), after["lineXHeight"].as_bool()), (Some(400), Some(true), Some(false)));
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

/// With Custom on, the dialog draws the set's characters and its buttons.
#[test]
fn custom_character_sets_draw_their_characters_and_the_buttons() {
    let mut p = crate::tests::panel_harness::Panel::new(app(), crate::tests::picks::dialogs);
    open(&mut p.app()).unwrap();
    new_font(&mut p.app());
    dialog(&mut p.app(), |d| d.custom = true);
    new_set(&mut p.app());
    dialog(&mut p.app(), |d| d.entry = "AB".into());
    edit(&mut p.app(), SetAction::AddChars).unwrap();
    p.frame();
    for name in ["A", "B", "OK", "Cancel"] {
        assert!(p.shows(name), "{name} is drawn");
    }
}
