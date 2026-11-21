use super::*;

/// The saved UI state of an old `ui.json`, as the legacy import reads it.
fn legacy_ui(bytes: &[u8]) -> Option<vectorcraft_ui_egui::UiState> {
    vectorcraft_ui_egui::parse_legacy(bytes).ok().map(|(settings, _)| settings.ui)
}

/// Shortcut overrides an earlier version saved still load: its spellings, `panel:` keys, an
/// override that can't fire (#487), an entry for a command this version lacks, and an old set
/// name.
#[test]
fn shortcut_overrides_from_an_earlier_version_load() {
    use serde_json::json;
    use vectorcraft_ui_egui::{keymap, shortcut_editor};
    let mut raw = serde_json::to_value(vectorcraft_ui_egui::UiState::default()).unwrap();
    raw["shortcut_overrides"] = json!({
        "tool:pen": "Shift+9",
        "edit.preferences": "",
        "panel:layers": "Cmd+Alt+Shift+F7",
        "tool:groupSelection": "Shift+ShiftLeft",
        "test.unknown": "Cmd+Alt+Shift+F9",
        "view.grid": "Cmd+Period",
    });
    raw["shortcut_set"] = json!(shortcut_editor::LEGACY_PRESETS[0].0);
    let ui = legacy_ui(&serde_json::to_vec(&raw).unwrap()).unwrap().sanitized();
    assert_eq!(ui.shortcut_set, shortcut_editor::LEGACY_PRESETS[0].1);
    let km = keymap::keymap_with(&ui.shortcut_overrides);
    let text = |key: &str| keymap::effective_with(&km, key);
    assert_eq!(text("tool:pen").as_deref(), Some("Shift+9"));
    assert_eq!(text("edit.preferences"), None);
    assert_eq!(text("panel:layers").as_deref(), Some("Cmd+Alt+Shift+F7"));
    assert_eq!(text("tool:groupSelection").as_deref(), shortcut_editor::default_of("tool:groupSelection"), "back on its default");
    assert_eq!(text("view.grid").as_deref(), Some("Cmd+Period"));
    let period = keymap::vc_chord("Cmd+.").unwrap();
    assert_eq!(keymap::lookup(&km, &period).map(|c| c.id), Some("view.grid".to_string()));
    assert_eq!(text("test.unknown").as_deref(), Some("Cmd+Alt+Shift+F9"), "kept");
    assert!(km.problems().iter().any(|p| format!("{p:?}") == r#"UnknownCommand { key: "test.unknown" }"#), "reported");
}

/// The command line's files wait for the choice about a stopped plug-in, so they open with the
/// plug-ins it loads; with no choice up they open at once.
#[test]
fn command_line_files_wait_for_the_choice_about_a_stopped_plug_in() {
    let temp = tempfile::Builder::new().prefix("vectorcraft-held-files-").tempdir().unwrap();
    let file = temp.path().join("a.svg");
    std::fs::write(&file, r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#).unwrap();
    let files = vec![file.to_string_lossy().into_owned()];
    // The files the app reads to open them.
    let read = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let reads = read.clone();
    let services = vectorcraft_ui_egui::Services {
        read: Some(Box::new(move |p: &str| {
            reads.borrow_mut().push(p.to_owned());
            std::fs::read(p).map_err(|e| e.to_string())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
    app.ui.stopped_plugin = Some(serde_json::json!({"id": "test.held", "name": "Held"}));
    assert_eq!(open_or_hold(&mut app, files.clone()), files, "held while the choice is up");
    assert!(read.borrow().is_empty(), "nothing opened before the choice");
    app.ui.stopped_plugin = None;
    assert!(open_or_hold(&mut app, files.clone()).is_empty());
    assert_eq!(*read.borrow(), files, "opened with no choice up");
}

#[test]
fn opening_files_unminimizes_and_focuses_the_window() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| bring_window_forward(ui.ctx()));
    // Nothing paints in the test: drop the font atlas upload egui expects to be applied.
    out.textures_delta.clear();
    let cmds = &out.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(cmds.contains(&egui::ViewportCommand::Minimized(false)));
    assert!(cmds.contains(&egui::ViewportCommand::Focus));
}

#[test]
fn unreadable_saved_docking_keeps_preferences_and_every_workspace() {
    use serde_json::json;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("window.panel.float", json!({"panel":"layers"})).unwrap();
    app.ui.toolbar = false;
    app.ui.engine_prefs = json!({"interfaceLanguage":"fr", "uiBrightness":"light"});
    app.ui.custom_workspaces =
        vec![vectorcraft_ui_egui::workspaces::capture(&app.ui, "Future"), vectorcraft_ui_egui::workspaces::capture(&app.ui, "Valid")];
    app.ui.workspace_changes = vec![vectorcraft_ui_egui::workspaces::capture(&app.ui, "Future")];
    let original = serde_json::to_value(&app.ui).unwrap();
    let valid_location = original["docking_hidden"]["layers"].clone();
    assert!(valid_location.is_object());
    for invalid_layout in [json!({"root":{"FutureNode":{}},"floating":[]}), json!(false)] {
        for invalid_hidden in [json!({"layers":{"placement":{"FuturePlacement":{}}},"swatches":valid_location}), json!(["malformed"])] {
            let mut raw = original.clone();
            raw["docking"] = invalid_layout.clone();
            raw["docking_hidden"] = invalid_hidden.clone();
            raw["custom_workspaces"][0]["docking"] = invalid_layout.clone();
            raw["custom_workspaces"][0]["dockingHidden"] = invalid_hidden.clone();
            raw["workspace_changes"][0]["docking"] = invalid_layout.clone();
            raw["workspace_changes"][0]["dockingHidden"] = invalid_hidden.clone();
            assert!(serde_json::from_value::<vectorcraft_ui_egui::UiState>(raw.clone()).is_err(), "runtime serde stays strict");
            let loaded = legacy_ui(&serde_json::to_vec(&raw).unwrap()).unwrap();
            assert_eq!(loaded.engine_prefs, app.ui.engine_prefs);
            assert!(!loaded.toolbar);
            assert!(loaded.docking.is_none());
            assert!(!loaded.docking_hidden.contains_key("layers"));
            if invalid_hidden.is_object() {
                assert_eq!(serde_json::to_value(loaded.docking_hidden.get("swatches")).unwrap(), valid_location);
            } else {
                assert!(loaded.docking_hidden.is_empty());
            }
            assert_eq!(loaded.custom_workspaces.len(), 2);
            assert_eq!(loaded.custom_workspaces[0].name, "Future");
            assert!(loaded.custom_workspaces[0].docking.is_none());
            assert!(!loaded.custom_workspaces[0].docking_hidden.contains_key("layers"));
            assert_eq!(serde_json::to_value(&loaded.custom_workspaces[1]).unwrap(), original["custom_workspaces"][1]);
            assert_eq!(loaded.workspace_changes.len(), 1, "a remembered layout is recovered, not the whole file lost");
            assert!(loaded.workspace_changes[0].docking.is_none());
        }
    }
    let roundtrip = legacy_ui(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), original);
    assert!(legacy_ui(&serde_json::to_vec(&json!({"toolbar":"invalid"})).unwrap()).is_none(), "unrelated fields retain normal strict parsing");
}

/// The format boundary of the preferences file: a state with its saved fields off their defaults
/// goes through `config.toml` and comes back equal (a field TOML cannot hold fails here).
#[test]
fn a_saved_ui_state_survives_a_config_toml_round_trip() {
    use craft::settings::SettingsStore;
    use serde_json::json;
    use vectorcraft_ui_egui::{AppSettings, from_app, workspaces};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("window.panel.float", json!({"panel":"layers"})).unwrap();
    app.run("prefs.set", json!({"key": "showToolTips", "value": false})).unwrap();
    app.ui.toolbar = false;
    app.ui.image_trace_preview = false;
    app.ui.custom_workspaces = vec![workspaces::capture(&app.ui, "Mine")];
    app.ui.workspace_changes = vec![workspaces::capture(&app.ui, "Mine")];
    let temp = tempfile::Builder::new().prefix("vectorcraft-config-toml-").tempdir().unwrap();
    let wanted = from_app(&app);
    let (mut store, _) = SettingsStore::<AppSettings>::open_at(temp.path());
    store.set(wanted.clone());
    store.flush().unwrap();
    let (back, report) = SettingsStore::<AppSettings>::open_at(temp.path());
    assert!(report.section_errors.is_empty(), "{:?}", report.section_errors);
    assert_eq!(back.get(), &wanted);
}

/// The file extensions the macOS bundle declares: its document types and its own exported type.
fn plist_extensions(plist: &str) -> Vec<&str> {
    ["<key>CFBundleTypeExtensions</key>", "<key>public.filename-extension</key>"]
        .iter()
        .flat_map(|key| plist.split(key).skip(1))
        .filter_map(|rest| rest.split("</array>").next())
        .flat_map(|array| array.split("<string>").skip(1))
        .filter_map(|s| s.split("</string>").next())
        .collect()
}

/// Finder offers the app for every file File › Open reads (#295, #354) but those with an
/// extension in `UNASSOCIATED_EXTS`, takes over no other app's files, and hands them to the app
/// rather than to AppKit's document machinery.
#[test]
fn the_macos_bundle_opens_every_readable_format() {
    let plist = include_str!("../../../packaging/macos/Info.plist.in");
    let declared = plist_extensions(plist);
    for e in fileio::OPEN_EXTS.iter().filter(|e| !fileio::UNASSOCIATED_EXTS.contains(e)) {
        assert!(declared.contains(e), "Info.plist.in doesn't declare .{e}");
    }
    for e in &declared {
        assert!(fileio::OPEN_EXTS.contains(e), "Info.plist.in declares .{e}, which the app doesn't open");
    }
    assert!(!plist.contains("<key>NSDocumentClass</key>"), "not an NSDocument app: AppKit would refuse the files");
    let types = plist.matches("<key>CFBundleTypeName</key>").count();
    assert!(types > 1 && plist.matches("<key>LSHandlerRank</key>").count() == types, "every document type has a rank");
    assert_eq!(plist.matches("<string>Owner</string>").count(), 2, "only VectorCraft documents and templates are owned");
}

/// The MIME types a Linux desktop gives each extension in `OPEN_EXTS` but `UNASSOCIATED_EXTS`
/// (one row per type): shared-mime-info's, or one that `ai.storyteller.vectorcraft.mime.xml`
/// declares.
const LINUX_TYPES: &[(&str, &str)] = &[
    ("vectorcraft", "application/x-vectorcraft"),
    ("drawcraft", "application/x-vectorcraft"),
    ("svg", "image/svg+xml"),
    ("svgz", "image/svg+xml-compressed"),
    ("pdf", "application/pdf"),
    ("ai", "application/illustrator"), // brand-ok: shared-mime-info's type
    ("ait", "application/x-ai-template"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("bmp", "image/bmp"),
    ("vctemplate", "application/x-vectorcraft-template"),
    ("dxf", "image/vnd.dxf"),
    ("emf", "image/emf"),
    ("wmf", "image/wmf"),
    ("eps", "image/x-eps"),
    ("af", "application/x-affinity"),
    ("afdesign", "application/x-affinity"),
    ("afpub", "application/x-affinity"),
];

/// The types in `LINUX_TYPES` that shared-mime-info defines. The MIME package declares every
/// other one.
const LINUX_SHARED_TYPES: &[&str] = &[
    "image/svg+xml",
    "image/svg+xml-compressed",
    "application/pdf",
    "application/illustrator", // brand-ok: shared-mime-info's type
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/bmp",
    "image/vnd.dxf",
    "image/emf",
    "image/wmf",
    "image/x-eps",
];

/// Extensions in `LINUX_TYPES` whose type the MIME package matches by file content and not by a
/// glob, with the package's `<magic>` rule for that type (its lines joined, indentation removed).
const LINUX_BY_CONTENT: &[(&str, &str)] = &[(
    "af",
    // `MAGIC` at offset 0 and `DOCUMENT_KIND` at offset 8 (crates/affinity/src/container.rs).
    r#"<magic><match type="string" offset="0" value="\x00\xff\x4b\x41"><match type="string" offset="8" value="nsrP"/></match></magic>"#,
)];

/// The desktop entry lists a MIME type for every extension in `OPEN_EXTS` but
/// `UNASSOCIATED_EXTS`, and the app's MIME package declares the types shared-mime-info doesn't
/// have.
#[test]
fn the_linux_desktop_entry_opens_every_readable_format() {
    let desktop = include_str!("../../../packaging/linux/ai.storyteller.vectorcraft.desktop");
    let mime = include_str!("../../../packaging/linux/ai.storyteller.vectorcraft.mime.xml");
    let listed: Vec<&str> =
        desktop.lines().find_map(|l| l.strip_prefix("MimeType=")).unwrap_or_default().split(';').filter(|t| !t.is_empty()).collect();
    // Each type the MIME package declares, with the extensions of its globs and its `<magic>`
    // rule (empty when it has none) in the form `LINUX_BY_CONTENT` gives.
    let declared: Vec<(&str, Vec<&str>, String)> = mime
        .split("<mime-type type=\"")
        .skip(1)
        .map(|block| {
            let body = block.split("</mime-type>").next().unwrap_or_default();
            let globs = body.split("<glob pattern=\"*.").skip(1).filter_map(|g| g.split('"').next()).collect();
            let magic = match (body.find("<magic"), body.find("</magic>")) {
                (Some(start), Some(end)) => body[start..end + "</magic>".len()].lines().map(str::trim).collect(),
                _ => String::new(),
            };
            (block.split('"').next().unwrap_or_default(), globs, magic)
        })
        .collect();
    let mut associated: Vec<&str> = fileio::OPEN_EXTS.iter().copied().filter(|e| !fileio::UNASSOCIATED_EXTS.contains(e)).collect();
    let mut typed: Vec<&str> = LINUX_TYPES.iter().map(|(e, _)| *e).collect();
    associated.sort_unstable();
    typed.sort_unstable();
    typed.dedup();
    assert_eq!(typed, associated, "a MIME type for each extension the packaging associates with the app");
    for (e, t) in LINUX_TYPES {
        assert!(listed.contains(t), "the desktop entry doesn't list {t} (.{e})");
        let package = declared.iter().find(|(name, ..)| name == t);
        if LINUX_SHARED_TYPES.contains(t) {
            assert!(package.is_none(), "{t} is shared-mime-info's, and the MIME package declares it again");
            continue;
        }
        let Some((_, globs, magic)) = package else { panic!("the MIME package doesn't declare {t} (.{e})") };
        if let Some((_, rule)) = LINUX_BY_CONTENT.iter().find(|(content, _)| content == e) {
            assert!(!globs.contains(e), "{t} matches .{e} files by content only");
            assert_eq!(magic, rule, "{t}'s magic rule for .{e} files");
        } else {
            assert!(globs.contains(e), "{t} has no *.{e} glob");
        }
    }
    for t in &listed {
        assert!(LINUX_TYPES.iter().any(|(_, known)| known == t), "the desktop entry lists {t}, which the app doesn't open");
    }
    for (t, globs, _) in &declared {
        assert!(listed.contains(t), "the desktop entry doesn't list the declared {t}");
        for g in globs {
            assert!(LINUX_TYPES.contains(&(*g, *t)), "{t}'s *.{g} isn't a file the app opens as {t}");
        }
    }
}
