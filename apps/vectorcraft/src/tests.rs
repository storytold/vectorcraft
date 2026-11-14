use super::*;

fn ui_state_with_prefs(engine_prefs: serde_json::Value) -> vectorcraft_ui_egui::UiState {
    vectorcraft_ui_egui::UiState { engine_prefs, ..Default::default() }
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
    let ui = decode_saved_prefs(&serde_json::to_vec(&raw).unwrap()).unwrap().sanitized();
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
            let loaded = decode_saved_prefs(&serde_json::to_vec(&raw).unwrap()).unwrap();
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
    let roundtrip = decode_saved_prefs(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), original);
    assert!(
        decode_saved_prefs(&serde_json::to_vec(&json!({"toolbar":"invalid"})).unwrap()).is_none(),
        "unrelated fields retain normal strict parsing"
    );
}

#[test]
fn an_unreadable_ui_json_is_kept_beside_it() {
    let temp = tempfile::Builder::new().prefix("vectorcraft-ui-prefs-").tempdir().unwrap();
    let dir = temp.path();
    let path = dir.join("ui.json");
    super::keep_prefs_copy(&path, b"{\"docking\": {\"root\": {\"FutureNode\": {}}}}", "ui.unreadable");
    super::keep_prefs_copy(&path, b"not json", "ui.unreadable");
    super::keep_prefs_copy(&path, b"{}", "ui.before-layout-migration");
    let first = std::fs::read(dir.join("ui.unreadable.json")).unwrap();
    let second = std::fs::read(dir.join("ui.unreadable-2.json")).unwrap();
    let before = std::fs::read(dir.join("ui.before-layout-migration.json")).unwrap();
    assert!(first.starts_with(b"{\"docking\""), "the first copy is not replaced");
    assert_eq!(second, b"not json");
    assert_eq!(before, b"{}");
    assert!(!super::KEEP_UI_PREFS.load(std::sync::atomic::Ordering::Relaxed), "ui.json can be saved once a copy is kept");
}

#[test]
fn the_image_trace_preview_setting_is_kept_across_a_restart() {
    for preview in [false, true] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.ui.image_trace_preview = preview;
        let saved = prefs_bytes(&app).unwrap();
        let loaded = decode_saved_prefs(&saved).unwrap().sanitized();
        assert_eq!(loaded.image_trace_preview, preview, "Preview {preview} after a restart");
    }
}

#[test]
fn the_system_title_bar_preference_keeps_the_window_decorations() {
    use craft_window::{Frame, Layout};

    // The System Title Bar preference (Preferences › User Interface) is read the same way on
    // every platform; a missing or unreadable value is `false`.
    assert!(!super::opted_into_system_title_bar(None), "no preferences: the default");
    let off = ui_state_with_prefs(serde_json::json!({"systemTitleBar": false}));
    assert!(!super::opted_into_system_title_bar(Some(&off)));
    let on = ui_state_with_prefs(serde_json::json!({"systemTitleBar": true}));
    assert!(super::opted_into_system_title_bar(Some(&on)));

    // The suite-wide mapping (Frame + preference -> whether VectorCraft draws its own title
    // bar). The desktop frame (macOS, Windows, X11, KDE/Wayland, …) always keeps native
    // decorations; the app frame (GNOME/Wayland, or a `VECTORCRAFT_TITLEBAR` override) draws
    // our own title bar — unless the user opted into the system title bar, which forces the
    // native frame back on.
    let app = Frame::App(Layout::default());
    let desktop = Frame::Desktop;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        assert!(super::custom_titlebar_from(&app, false), "GNOME/Wayland: VectorCraft draws its own");
        assert!(!super::custom_titlebar_from(&app, true), "System Title Bar preference forces native");
        assert!(!super::custom_titlebar_from(&desktop, false), "KDE/X11/…: keep the compositor's frame");
        assert!(!super::custom_titlebar_from(&desktop, true));
    }
    // macOS and Windows keep today's behavior, independent of the probed frame: the pref still
    // forces the native frame back on where a custom one would otherwise be drawn.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        assert_eq!(super::custom_titlebar_from(&desktop, false), super::CUSTOM_TITLEBAR);
        assert_eq!(super::custom_titlebar_from(&app, false), super::CUSTOM_TITLEBAR, "the probe does not change macOS/Windows");
        assert!(!super::custom_titlebar_from(&desktop, true));
    }

    assert_eq!(super::native_options(false).viewport.decorations, Some(true));
    assert_eq!(super::native_options(true).viewport.decorations, Some(false));
}

/// Preferences › User Interface › VectorCraft Title Bar on Every Desktop gives the app frame where
/// the probe chose the desktop's (KDE, X11, a window opened through XWayland); without it the probe
/// decides, and `VECTORCRAFT_TITLEBAR` set to `app`, `system` or `desktop` takes precedence over
/// both.
#[test]
fn the_app_title_bar_preference_gives_the_app_frame_on_desktop_frames() {
    assert!(!super::opted_into_app_title_bar(None), "no preferences: the default");
    let off = ui_state_with_prefs(serde_json::json!({"appTitleBar": false}));
    assert!(!super::opted_into_app_title_bar(Some(&off)));
    let on = ui_state_with_prefs(serde_json::json!({"appTitleBar": true}));
    assert!(super::opted_into_app_title_bar(Some(&on)));

    // (probed_app, overridden, forced_to_x11): without the preference, then with it.
    let cases = [
        ((false, None, false), false, true),        // KDE, Sway, X11: the desktop frame
        ((true, None, false), true, true),          // GNOME on Wayland: the app frame either way
        ((true, None, true), false, true),          // GNOME, the window opened through XWayland
        ((false, Some(true), false), true, true),   // VECTORCRAFT_TITLEBAR=app
        ((false, Some(true), true), true, true),    // VECTORCRAFT_TITLEBAR=app, the window opened through XWayland
        ((true, Some(false), false), false, false), // VECTORCRAFT_TITLEBAR=desktop takes precedence over the preference
        ((false, Some(false), true), false, false),
    ];
    for ((probed_app, overridden, forced_to_x11), without, with) in cases {
        assert_eq!(super::app_draws_frame(probed_app, overridden, forced_to_x11, false), without, "{probed_app} {overridden:?} {forced_to_x11}");
        assert_eq!(super::app_draws_frame(probed_app, overridden, forced_to_x11, true), with, "{probed_app} {overridden:?} {forced_to_x11}");
    }
}

/// The frame a start uses: the preference turns the desktop frame the probe chose into the app frame
/// with the default window buttons, a probed app frame keeps its own buttons, and the desktop frame
/// stays where the app frame is not drawn. System Title Bar takes precedence over the preference.
#[test]
fn the_start_frame_applies_the_app_title_bar_preference() {
    use craft_window::{Button, Frame, Layout};
    let gnome = Layout { left: vec![Button::Close], right: vec![] };
    let cases = [
        // (probed, overridden, forced_to_x11, opted_into_app), expected
        ((Frame::Desktop, None, false, false), Frame::Desktop),
        ((Frame::Desktop, None, false, true), Frame::App(Layout::default())),
        ((Frame::Desktop, None, true, true), Frame::App(Layout::default())),
        ((Frame::App(gnome.clone()), None, false, false), Frame::App(gnome.clone())),
        ((Frame::App(gnome.clone()), None, true, false), Frame::Desktop),
        ((Frame::App(gnome.clone()), None, true, true), Frame::App(gnome.clone())),
        ((Frame::Desktop, Some(false), false, true), Frame::Desktop),
        ((Frame::App(gnome.clone()), Some(true), true, false), Frame::App(gnome.clone())),
    ];
    for ((probed, overridden, forced_to_x11, opted_into_app), expected) in cases {
        let label = format!("{probed:?} {overridden:?} {forced_to_x11} {opted_into_app}");
        assert_eq!(super::start_frame(probed, overridden, forced_to_x11, opted_into_app), expected, "{label}");
    }
    // With both preferences on, System Title Bar takes precedence: the desktop draws the frame.
    let both = super::start_frame(Frame::Desktop, None, false, true);
    assert!(!super::custom_titlebar_from(&both, true), "System Title Bar takes precedence over the app title bar preference");
}

/// `VECTORCRAFT_TITLEBAR` chooses the frame for the run it is set in, with System Title Bar off and
/// on. The saved preference still decides while the variable is unset.
#[test]
fn the_titlebar_variable_takes_precedence_over_system_title_bar() {
    use craft_window::Frame;
    assert!(super::system_title_bar_applies(true, false));
    assert!(!super::system_title_bar_applies(true, true));
    assert!(!super::system_title_bar_applies(false, false));
    assert!(!super::system_title_bar_applies(false, true));
    // VECTORCRAFT_TITLEBAR=app with System Title Bar on: VectorCraft draws its own title bar.
    let frame = super::start_frame(Frame::Desktop, Some(true), false, false);
    assert_eq!(super::custom_titlebar_from(&frame, super::system_title_bar_applies(true, true)), !cfg!(target_os = "macos"));
    // VECTORCRAFT_TITLEBAR=desktop with System Title Bar off keeps the desktop's frame.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let frame = super::start_frame(Frame::App(craft_window::Layout::default()), Some(false), false, false);
        assert!(!super::custom_titlebar_from(&frame, super::system_title_bar_applies(false, true)));
    }
}

/// `VECTORCRAFT_TITLEBAR` read from the environment at startup, with System Title Bar off and on.
/// The variable is process-wide and read once, so each value runs in a separate test process.
#[test]
fn the_titlebar_variable_decides_the_frame_before_the_window_opens() {
    const CHILD: &str = "VECTORCRAFT_TEST_TITLEBAR_CHILD";
    if std::env::var_os(CHILD).is_none() {
        for value in ["app", "system", "desktop", "unknown"] {
            // Linux and the BSDs: hold `spawning` while the child starts, so it does not keep a
            // copy of a GPU start test's locked start file (see `CHILD_SPAWN`).
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
            let spawning = super::spawning();
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::the_titlebar_variable_decides_the_frame_before_the_window_opens", "--nocapture"])
                .env(CHILD, "1")
                .env("VECTORCRAFT_TITLEBAR", value)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
            drop(spawning);
            let output = child.unwrap().wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "VECTORCRAFT_TITLEBAR={value}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    let value = std::env::var("VECTORCRAFT_TITLEBAR").unwrap();
    let not_read = cfg!(any(target_os = "macos", target_os = "windows"));
    assert_eq!(super::titlebar_from_env(), !not_read && value != "unknown", "VECTORCRAFT_TITLEBAR={value}");
    let off = ui_state_with_prefs(serde_json::json!({"systemTitleBar": false}));
    let on = ui_state_with_prefs(serde_json::json!({"systemTitleBar": true}));
    for (saved, opted_into_system) in [(None, false), (Some(&off), false), (Some(&on), true)] {
        let custom = super::custom_titlebar(saved);
        let expected = match value.as_str() {
            _ if not_read => !cfg!(target_os = "macos") && !opted_into_system,
            "app" => true,
            "system" | "desktop" => false,
            // Not a value the variable takes: System Title Bar decides when it is on. Otherwise the
            // `craft-window` probe decides, the same with System Title Bar off as with no saved
            // preferences.
            _ if opted_into_system => false,
            _ => super::custom_titlebar(None),
        };
        assert_eq!(custom, expected, "{value}, preference {opted_into_system}");
        assert_eq!(super::native_options(custom).viewport.decorations, Some(!custom), "{value}, preference {opted_into_system}");
    }
}

/// Preferences says under System Title Bar that the desktop draws the frame with it on or off
/// exactly where the start frame without VectorCraft Title Bar on Every Desktop is the desktop's
/// with System Title Bar off as well as on and `VECTORCRAFT_TITLEBAR` is unset: Linux and the BSDs
/// only.
#[test]
fn the_desktop_keeps_the_frame_where_system_title_bar_does_not_change_it() {
    use crate::titlebar::desktop_keeps_frame_from;
    use craft_window::{Frame, Layout};
    let linux = cfg!(not(any(target_os = "macos", target_os = "windows")));
    for probed in [Frame::Desktop, Frame::App(Layout::default())] {
        for forced_to_x11 in [false, true] {
            let frame = super::start_frame(probed.clone(), None, forced_to_x11, false);
            let either_way = !super::custom_titlebar_from(&frame, false) && !super::custom_titlebar_from(&frame, true);
            assert_eq!(desktop_keeps_frame_from(&probed, None, forced_to_x11), linux && either_way, "{probed:?} {forced_to_x11}");
        }
    }
    // KDE, Sway and X11, and GNOME through XWayland, keep the desktop's frame; GNOME on Wayland does not.
    assert_eq!(desktop_keeps_frame_from(&Frame::Desktop, None, false), linux);
    assert_eq!(desktop_keeps_frame_from(&Frame::App(Layout::default()), None, true), linux);
    assert!(!desktop_keeps_frame_from(&Frame::App(Layout::default()), None, false));
    // While `VECTORCRAFT_TITLEBAR` chooses the frame, System Title Bar does not apply and there is no note.
    for (frame, overridden) in [(Frame::App(Layout::default()), Some(true)), (Frame::Desktop, Some(false))] {
        for forced_to_x11 in [false, true] {
            assert!(!desktop_keeps_frame_from(&frame, overridden, forced_to_x11), "{overridden:?} {forced_to_x11}");
        }
    }
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

/// On Linux and the BSDs, quitting hides the main window before eframe drops the app, whose
/// clipboard handles may take up to `CLIPBOARD_PROBE_STOP_WAIT` to let go.
#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn quitting_hides_the_window_before_the_clipboards_let_go() {
    struct Window(std::cell::Cell<u32>);
    impl QuittingWindow for Window {
        fn hide(&self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let window = Window(std::cell::Cell::new(0));
    hide_before_teardown(Some(&window));
    assert_eq!(window.0.get(), 1, "the window was not hidden");
    // Without a window (none was made), there is nothing to hide.
    hide_before_teardown::<Window>(None);
}
