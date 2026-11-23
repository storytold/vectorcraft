//! Export As → EPS Options, drawn headlessly.

use serde_json::json;

use super::tests_export::{app, frame, kind, names, set};
use super::*;

/// The PostScript of an EPS file.
fn ps(bytes: &[u8]) -> String {
    String::from_utf8(vectorcraft_eps::sections(bytes).unwrap().0.to_vec()).unwrap()
}

#[test]
fn export_as_eps_shows_eps_options_then_writes_and_remembers_them() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "eps"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("epsOptions"), "EPS Options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("path"), d.str("previewFormat"), d.str("flattenerPreset")),
        ("/out/Untitled-1.eps".into(), "tiffColor".into(), "Medium Resolution".into())
    );
    frame(&mut app);
    // Every control draws: the black-and-white preview greys out Transparent / Opaque.
    set(&mut app, "previewFormat", json!("tiffBw"));
    set(&mut app, "level", json!(2));
    set(&mut app, "cmykPostScript", json!(false));
    frame(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("level"), "2", "a number reads as the level");
    set(&mut app, "flattenerPreset", json!("High Resolution"));
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    // The central file, then one file per artboard, named with an underscore.
    assert_eq!(names(&written), ["/out/Untitled-1.eps", "/out/Untitled-1_Artboard-1.eps", "/out/Untitled-1_Artboard-2.eps"]);
    assert_eq!(r["files"].as_array().map(Vec::len), Some(3));
    let bytes = &written.borrow()[0].1;
    let (text, tiff) = vectorcraft_eps::sections(bytes).unwrap();
    assert!(tiff.is_some() && String::from_utf8_lossy(text).contains("%%LanguageLevel: 2"));
    assert!(ps(bytes).contains(" rg\n"), "RGB PostScript");
    // The next EPS Options start from these choices, but not from this export's path.
    crate::menus::invoke(&mut app, "ui.epsOptionsDialog", json!({}));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("previewFormat"), d.str("level"), d.str("flattenerPreset")), ("tiffBw".into(), "2".into(), "High Resolution".into()));
    assert!(!d.bool("cmykPostScript") && !d.fields.contains_key("path"));
    assert!(app.ui.eps_options.get("useArtboards").is_none());
}

#[test]
fn eps_options_take_saved_flattener_presets_and_refuse_bad_ones() {
    let (mut app, written) = app(1);
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 0})).unwrap();
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "flattenerPreset": "medium"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("flattenerPreset"), "Medium Resolution", "an id shows as its name");
    frame(&mut app);
    set(&mut app, "flattenerPreset", json!("Nope"));
    let e = confirm(&mut app).unwrap_err();
    assert!(e.contains("preset"), "{e}");
    assert_eq!(kind(&app), Some("epsOptions"));
    set(&mut app, "flattenerPreset", json!("Mine"));
    set(&mut app, "previewFormat", json!("none"));
    confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/out/a.eps"]);
    assert!(written.borrow()[0].1.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"), "{:?}", &written.borrow()[0].1[..8]);
}

/// Two overlapping squares, the second half transparent, so that an EPS export flattens them.
fn transparent_art(app: &mut VectorcraftApp) {
    for (x, color) in [(5, "#ff0000"), (20, "#0000ff")] {
        app.run("select.none", json!({})).unwrap();
        app.run("paint.setFill", json!({"color": color})).unwrap();
        app.run("shape.rectangle", json!({"x": x, "y": 5, "width": 30, "height": 30})).unwrap();
    }
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
}

/// Custom… over EPS Options, as its button opens it.
fn open_custom(app: &mut VectorcraftApp) {
    let parent = app.ui.dialog.clone().expect("EPS Options are open");
    app.ui.dialog = Some(eps_options::over(&parent, &app.session.prefs.flattener_presets));
    frame(app);
}

#[test]
fn custom_flattener_options_start_from_the_preset_and_cancel_goes_back_unchanged() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "flattenerPreset": "high"})).unwrap();
    frame(&mut app);
    let before = app.ui.dialog.as_ref().unwrap().fields.clone();
    open_custom(&mut app);
    assert_eq!(kind(&app), Some(eps_options::CUSTOM_KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("balance", 0.0), d.f64("lineArtPpi", 0.0), d.bool("clipComplexRegions")), (100.0, 1200.0, true), "High Resolution's");
    assert!(!d.bool("preserveAlpha"), "EPS has no alpha");
    set(&mut app, "balance", json!(10));
    cancel(&mut app);
    assert_eq!(kind(&app), Some("epsOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields, before, "Cancel leaves EPS Options as they were");
}

#[test]
fn custom_flattener_options_are_exported_and_remembered_until_a_preset_is_chosen() {
    let (mut app, written) = app(1);
    transparent_art(&mut app);
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 100})).unwrap();
    let opts = json!({"path": "/out/a.eps", "flattenerPreset": "Mine", "previewFormat": "none", "thumbnails": false});
    app.run("ui.epsOptionsDialog", opts).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("balance", 0.0), 100.0, "the saved preset's");
    set(&mut app, "balance", json!(0));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("epsOptions"), "OK goes back to EPS Options");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields["flattener"]["balance"].as_f64(), d.fields["flattener"]["preserveAlpha"].as_bool()), (Some(0.0), Some(false)));
    assert_eq!(d.str("flattenerPreset"), "Medium Resolution", "a built-in preset under options that set everything");
    frame(&mut app);
    // The custom options don't need the saved preset they started from.
    app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
    confirm(&mut app).unwrap();
    assert!(ps(&written.borrow()[0].1).contains("image"), "balance 0 rasterizes the flattened art");
    // The next EPS Options start from them; a preset asked for without options drops them.
    crate::menus::invoke(&mut app, "ui.epsOptionsDialog", json!({}));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["flattener"]["balance"].as_f64(), Some(0.0));
    cancel(&mut app);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/b.eps", "flattenerPreset": "medium"})).unwrap();
    assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("flattener"));
    confirm(&mut app).unwrap();
    assert!(!ps(&written.borrow()[1].1).contains("image"), "Medium Resolution keeps the squares vector");
}

#[test]
fn a_flattener_preset_set_from_outside_drops_the_custom_options() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps"})).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    set(&mut app, "balance", json!(20));
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().fields.contains_key("flattener"));
    // `ui.dialog.set {flattenerPreset: "low"}`, as the control channel sets it.
    let old = app.ui.dialog.as_ref().unwrap().fields.get("flattenerPreset").cloned();
    set(&mut app, "flattenerPreset", json!("low"));
    field_set(&mut app, "flattenerPreset", old.as_ref()).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!d.fields.contains_key("flattener"), "the preset replaces the custom options");
    assert_eq!(d.str("flattenerPreset"), "Low Resolution");
}

#[test]
fn bad_custom_flattener_options_keep_the_dialog_open() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "overprints": "discard"})).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    set(&mut app, "gradientPpi", json!(9000));
    frame(&mut app);
    let e = confirm(&mut app).unwrap_err();
    assert!(e.contains("resolutions"), "{e}");
    assert_eq!(kind(&app), Some(eps_options::CUSTOM_KIND));
    // Overprints decides whether overprints are kept, so OK keeps the option as it was.
    set(&mut app, "gradientPpi", json!(150));
    set(&mut app, "preserveAlpha", json!(true));
    confirm(&mut app).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields["flattener"]["preserveOverprints"].as_bool(), d.fields["flattener"]["preserveAlpha"].as_bool()), (Some(true), Some(false)));
}

/// Headless frames of the dialog layer with screen-reader output, input events and a clock.
pub(super) struct Frames {
    ctx: egui::Context,
    time: f64,
}

impl Frames {
    pub(super) fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0 }
    }

    fn run(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.1;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    /// The screen-reader node of the control of `role` named `label`, with its center.
    pub(super) fn control(&mut self, app: &mut VectorcraftApp, label: &str, role: egui::accesskit::Role) -> (egui::accesskit::NodeId, egui::Pos2) {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let (id, n) =
            nodes.iter().find(|(_, n)| n.label() == Some(label) && n.role() == role).unwrap_or_else(|| panic!("no {role:?} named {label:?}"));
        let b = n.bounds().expect("bounds");
        (*id, egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5))
    }

    pub(super) fn click(&mut self, app: &mut VectorcraftApp, at: egui::Pos2) {
        for pressed in [true, false] {
            let button = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            self.run(app, vec![egui::Event::PointerMoved(at), button]);
        }
    }

    /// Click the text field named `label`, type `text` over its text and press Tab.
    pub(super) fn type_into(&mut self, app: &mut VectorcraftApp, label: &str, text: &str) {
        let (_, at) = self.control(app, label, egui::accesskit::Role::TextInput);
        self.click(app, at);
        self.run(app, vec![egui::Event::Text(text.into())]);
        let tab = |pressed| egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        self.run(app, vec![tab(true)]);
        self.run(app, vec![tab(false)]);
        self.run(app, vec![]);
    }
}

/// Enter on the focused Custom… button opens the custom options and does not also confirm them.
#[test]
fn enter_on_the_custom_button_opens_the_custom_options() {
    use egui::accesskit::{Action, ActionRequest, Role, TreeId};
    let (mut app, _) = app(1);
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 0})).unwrap();
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "flattenerPreset": "Mine"})).unwrap();
    let mut f = Frames::new();
    let (node, _) = f.control(&mut app, "Custom…", Role::Button);
    f.run(
        &mut app,
        vec![egui::Event::AccessKitActionRequest(ActionRequest { action: Action::Focus, target_node: node, target_tree: TreeId::ROOT, data: None })],
    );
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    f.run(&mut app, vec![enter(true)]);
    f.run(&mut app, vec![enter(false)]);
    assert_eq!(kind(&app), Some(eps_options::CUSTOM_KIND), "the custom options stay open");
    let parent = super::parent(app.ui.dialog.as_ref().unwrap()).unwrap();
    assert_eq!(parent.str("flattenerPreset"), "Mine", "EPS Options keep their preset");
}

/// With a value out of range, the editor shows the values entered, and changing a switch keeps
/// them, so OK still refuses the bad value.
#[test]
fn a_switch_changed_beside_a_bad_value_keeps_the_values_entered() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps"})).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    set(&mut app, "balance", json!(20));
    set(&mut app, "gradientPpi", json!(9000));
    set(&mut app, "lineArtPpi", json!("many"));
    let mut f = Frames::new();
    let (_, at) = f.control(&mut app, "Convert All Text to Outlines", egui::accesskit::Role::CheckBox);
    let before = app.ui.dialog.as_ref().unwrap().bool("textToOutlines");
    f.click(&mut app, at);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.bool("textToOutlines"), !before, "the switch changes");
    assert_eq!((d.f64("balance", 0.0), d.f64("gradientPpi", 0.0)), (20.0, 9000.0), "the other values stay as entered");
    assert_eq!(d.fields["lineArtPpi"], json!("many"), "a value of the wrong type stays to be refused");
    assert!(confirm(&mut app).is_err());
    set(&mut app, "lineArtPpi", json!(300));
    assert!(confirm(&mut app).unwrap_err().contains("resolutions"));
}

/// A value of the wrong type shows blank, and typing the default preset's value into its field
/// replaces it, so OK takes the options.
#[test]
fn the_default_value_typed_over_a_value_of_the_wrong_type_replaces_it() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps"})).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    set(&mut app, "lineArtPpi", json!("many"));
    let default = vectorcraft_engine::cmd::FlattenOptions::default().line_art_ppi;
    let mut f = Frames::new();
    f.type_into(&mut app, "Line Art and Text Resolution:", &default.to_string());
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["lineArtPpi"], json!(default));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("epsOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["flattener"]["lineArtPpi"].as_f64(), Some(default));
}

#[test]
fn the_eps_options_dialog_needs_a_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(app.run("ui.epsOptionsDialog", json!({})).is_err());
    assert!(!crate::menus::enabled(&app, "ui.epsOptionsDialog"));
}

/// Opening an EPS file whose PostScript reads only partly says why, in the status bar and in
/// `file.open`'s result (what `app.open` and MCP's `open_file` return): the error, the operator
/// and the procedure it ran in.
#[test]
fn opening_an_eps_that_reads_partly_says_why() {
    let dir = std::env::temp_dir().join(format!("vc-eps-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("partial.eps");
    std::fs::write(&path, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 10 10 rectfill /p { frobnicate } def p\n%%EOF\n")
        .unwrap();
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let r = app.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    let w = r["warnings"][0].as_str().unwrap();
    assert!(w.contains("`frobnicate`") && w.contains("in `p`"), "{r}");
    assert!(app.ui.status.contains("Opened with 1 note:") && app.ui.status.contains("frobnicate"), "{}", app.ui.status);
    std::fs::remove_dir_all(dir).ok();
}
