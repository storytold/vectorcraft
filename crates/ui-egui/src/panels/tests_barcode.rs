//! Headless frames of the Barcode panel: what it shows for a draft and for a selected barcode, and
//! that the button draws one through the same commands an agent would run.

use serde_json::json;
use vectorcraft_engine::Session;

use super::tests_appearance::{frame_events, run};
use super::*;

/// Every text drawn in one headless frame of the panel.
fn texts(app: &mut VectorcraftApp) -> Vec<String> {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame_events(&ctx, app, vec![], barcode::show);
    let ctx2 = egui::Context::default();
    crate::theme::install_fonts(&ctx2);
    frame_events(&ctx2, app, vec![], barcode::show).into_iter().map(|(s, _)| s).collect()
}

fn shows(t: &[String], s: &str) -> bool {
    t.iter().any(|x| x == s)
}

/// Whether a text starting with `s` was drawn: the panel's pickers read "EAN-13 · GS1 · GB 12904",
/// naming the standard beside the symbology.
fn starts(t: &[String], s: &str) -> bool {
    t.iter().any(|x| x.starts_with(s))
}

fn app_with_doc() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 800, "height": 800}));
    app
}

/// The panel is registered with its own icon, is in the collapsed column, and Window › Barcode
/// opens it.
#[test]
fn the_panel_is_registered_and_opens_from_the_window_menu() {
    let row = crate::state::ICON_PANELS.iter().find(|(id, ..)| *id == barcode::ID).expect("registered");
    assert_eq!(row.1, "Barcode");
    assert!(crate::icons::exists(row.2), "{} is not a bundled icon", row.2);
    assert!(crate::state::ICON_PANEL_GROUPS.iter().any(|g| g.contains(&barcode::ID)), "it reaches the collapsed column");

    let app = app_with_doc();
    let entries = crate::menus::menu_entries(&app);
    let item = entries
        .iter()
        .find(|e| e.command.as_deref() == Some("window.panel") && e.params.get("panel").and_then(serde_json::Value::as_str) == Some(barcode::ID))
        .expect("Window › Barcode");
    assert_eq!((item.path.clone(), item.label.as_str()), (vec!["Window".to_string()], "Barcode"));
}

/// Picking another type in the menu redraws the selected barcode straight away. The panel used to
/// read the old settings back off the object on the next frame, so the menu looked dead: the art
/// kept the type it was made with however many times the row was changed.
#[test]
fn choosing_a_type_redraws_the_selected_barcode() {
    let mut app = app_with_doc();
    let id = run(&mut app, "barcode.create", json!({"symbology": "ean13", "data": "6907045517163"}))["id"].as_u64().expect("id");
    run(&mut app, "select.set", json!({"ids": [id]}));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);

    let t = frame_events(&ctx, &mut app, vec![], barcode::show);
    let at = t.iter().find(|(s, _)| s.starts_with("EAN-13 · ")).map(|(_, r)| r.center()).expect("the type row");
    super::tests_appearance::click(&ctx, &mut app, at, barcode::show);

    let t = frame_events(&ctx, &mut app, vec![], barcode::show);
    let Some(at) = t.iter().find(|(s, _)| s.starts_with("UPC-A · ")).map(|(_, r)| r.center()) else {
        panic!("the menu lists UPC-A: {:?}", t.iter().map(|(s, _)| s).collect::<Vec<_>>())
    };
    super::tests_appearance::click(&ctx, &mut app, at, barcode::show);

    let spec = app
        .session
        .doc()
        .unwrap()
        .doc
        .node(vectorcraft_doc::NodeId(id))
        .and_then(|n| n.extras().barcode.as_deref())
        .cloned()
        .expect("the spec is still there");
    assert_eq!(spec.symbology, vectorcraft_barcode::Symbology::UpcA, "the object took the new type");
    assert_eq!(spec.data, "036000291452", "and the new type's example value");
}

/// A comma separates codes only where the data cannot carry one: a spreadsheet column pasted as one
/// line splits for EAN-13, and stays whole for Code 39's Full ASCII, Code 128 and a QR Code.
#[test]
fn a_comma_splits_only_where_it_cannot_be_data() {
    use vectorcraft_barcode::Symbology;
    assert_eq!(barcode::batch_entries("4006381333931,5012345678900", Symbology::Ean13).len(), 2);
    assert_eq!(barcode::batch_entries("4006381333931\n5012345678900", Symbology::Ean13).len(), 2);
    assert_eq!(barcode::batch_entries("ABC,DEF", Symbology::Code39), vec!["ABC,DEF".to_string()]);
    assert_eq!(barcode::batch_entries("https://x/?a=1,2", Symbology::QrCode), vec!["https://x/?a=1,2".to_string()]);
    assert_eq!(barcode::batch_entries("A,B\nC", Symbology::Code128).len(), 2, "a line still ends a code");
}

/// Every committed change draws on the selected object at once. The text fields commit on Enter or
/// when the focus leaves them, so a value being typed never redraws the object halfway through.
#[test]
fn every_committed_change_applies_at_once() {
    use vectorcraft_barcode::{BarcodeSpec, Symbology};
    let base = BarcodeSpec { symbology: Symbology::Ean13, data: "6907045517163".into(), ..Default::default() };
    assert!(!barcode::applies_at_once(&base, &base), "nothing changed");
    // A size, a checkbox, a font: drawn at once.
    assert!(barcode::applies_at_once(&BarcodeSpec { height_mm: Some(30.0), ..base.clone() }, &base));
    assert!(barcode::applies_at_once(&BarcodeSpec { background: false, ..base.clone() }, &base));
    assert!(barcode::applies_at_once(&BarcodeSpec { outline_text: false, ..base.clone() }, &base));
    // A new type comes with its own example value, and is drawn at once.
    assert!(barcode::applies_at_once(&BarcodeSpec { symbology: Symbology::UpcA, data: "036000291452".into(), ..base.clone() }, &base));
    // So is a value the field committed.
    assert!(barcode::applies_at_once(&BarcodeSpec { data: "690704551716".into(), ..base.clone() }, &base));
}

/// A draft shows the symbology it will draw with the standard it belongs to, an example value, the
/// size the symbol comes out at, and the batch that makes many at once.
#[test]
fn the_draft_shows_what_it_will_draw() {
    let mut app = app_with_doc();
    let t = texts(&mut app);
    assert!(starts(&t, "EAN-13 · "), "{t:?}");
    // The standard the selected type belongs to, in full.
    assert!(t.iter().any(|s| s.contains("ISO/IEC 15420") && s.contains("GB 12904")), "{t:?}");
    // The example value a fresh EAN-13 comes with, so the panel never sits on an empty field.
    assert!(shows(&t, "6907045517163"), "{t:?}");
    assert!(shows(&t, "Create"), "{t:?}");
    // The nominal EAN-13 with its quiet zones: 37.29 mm by 25.93 mm.
    assert!(t.iter().any(|s| s.starts_with("37.29")), "{t:?}");
    // The batch is there but folded: most visits draw one code, and the section is tall.
    assert!(shows(&t, "Batch"), "{t:?}");
    assert!(!shows(&t, "Create Batch"), "the batch starts folded: {t:?}");
}

/// A selected barcode's own settings fill the panel, and the button rewrites that object.
#[test]
fn a_selected_barcode_is_what_the_panel_edits() {
    let mut app = app_with_doc();
    let id = run(&mut app, "barcode.create", json!({"symbology": "code39", "data": "ABC-123"}))["id"].as_u64().expect("id");
    run(&mut app, "select.set", json!({"ids": [id]}));
    let t = texts(&mut app);
    assert!(starts(&t, "Code 39 · "), "{t:?}");
    assert!(shows(&t, "Update"), "{t:?}");

    run(&mut app, "barcode.set", json!({"data": "XYZ-9"}));
    let t = texts(&mut app);
    assert!(starts(&t, "Code 39 · "), "the type is unchanged: {t:?}");
    // The batch stays where it is: it leaves the codes it drew selected, and a section that
    // vanished under the pointer after the first run would read as the feature going away.
    assert!(shows(&t, "Batch"), "{t:?}");
}

/// Check asks the encoder about the value in the field and reports what it says, drawing nothing —
/// so a wrong check digit is caught before a print run rather than on the carton.
#[test]
fn the_check_button_reports_what_the_encoder_says() {
    let mut app = app_with_doc();
    app.ui.barcode =
        vectorcraft_barcode::BarcodeSpec { symbology: vectorcraft_barcode::Symbology::Ean13, data: "6907045517161".into(), ..Default::default() };
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let press = |ctx: &egui::Context, app: &mut VectorcraftApp| {
        let t = frame_events(ctx, app, vec![], barcode::show);
        let at = t.iter().find(|(s, _)| s == "Check").map(|(_, r)| r.center()).expect("the Check button");
        super::tests_appearance::click(ctx, app, at, barcode::show);
    };
    let before = app.session.doc().unwrap().doc.node_count();
    press(&ctx, &mut app);
    assert_eq!(app.session.doc().unwrap().doc.node_count(), before, "checking draws nothing");
    assert!(app.ui.barcode_note.contains('3'), "the right check digit is named: {}", app.ui.barcode_note);

    // A value the symbology accepts comes back with the digits the symbol would carry, which for
    // twelve typed digits includes the thirteenth the encoder worked out.
    app.ui.barcode.data = "690704551716".into();
    press(&ctx, &mut app);
    assert!(app.ui.barcode_note.contains("6907045517163"), "{}", app.ui.barcode_note);
    assert_eq!(app.session.doc().unwrap().doc.node_count(), before);
}

/// The batch draws one code a line through the same command an agent would run, counts what was
/// pasted in, and reports the lines the symbology cannot carry instead of drawing them.
#[test]
fn the_batch_button_draws_what_was_pasted_in() {
    let mut app = app_with_doc();
    app.ui.barcode_batch_open = true;
    app.ui.barcode_batch.data = "4006381333931\n5012345678900\nnonsense".into();
    let barcodes = |app: &VectorcraftApp| {
        let mut n = 0;
        app.session.doc().unwrap().doc.walk(|c| n += usize::from(c.extras().barcode.is_some()));
        n
    };
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let t = frame_events(&ctx, &mut app, vec![], barcode::show);
    assert!(t.iter().any(|(s, _)| s == "Codes: 3"), "the lines pasted in are counted");
    assert_eq!(barcodes(&app), 0, "nothing is drawn until the button is pressed");

    let at = t.iter().find(|(s, _)| s == "Create Batch").map(|(_, r)| r.center()).expect("the Create Batch button");
    super::tests_appearance::click(&ctx, &mut app, at, barcode::show);
    assert_eq!(barcodes(&app), 2, "the two codes that encode, and not the third");
    assert!(app.ui.barcode_batch_note.starts_with("Drawn: 2, 1"), "the refused line is reported: {}", app.ui.barcode_batch_note);
}

/// Data no symbol can carry is shown as the encoder's own refusal, in the panel, without a crash.
#[test]
fn data_that_cannot_be_encoded_is_reported_in_the_panel() {
    let mut app = app_with_doc();
    app.ui.barcode =
        vectorcraft_barcode::BarcodeSpec { symbology: vectorcraft_barcode::Symbology::Ean13, data: "nonsense".into(), ..Default::default() };
    let t = texts(&mut app);
    assert!(t.iter().any(|s| s.contains("digits")), "{t:?}");
}

/// A QR Code's draft shows the error correction, version and mask controls.
#[test]
fn a_matrix_symbology_shows_its_own_controls() {
    let mut app = app_with_doc();
    app.ui.barcode = vectorcraft_barcode::BarcodeSpec {
        symbology: vectorcraft_barcode::Symbology::QrCode,
        data: "https://example.com/".into(),
        ..Default::default()
    };
    let t = texts(&mut app);
    assert!(shows(&t, "Error Correction:"), "{t:?}");
    assert!(shows(&t, "Version:"), "{t:?}");
    assert!(shows(&t, "Mask:"), "{t:?}");
    assert!(!shows(&t, "Human Readable Line"), "a QR Code has no human readable line: {t:?}");
}
