//! The font menus list the installed fonts without asking for them (#36).

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_text::{FontDb, system_font_dirs};

use crate::{VectorcraftApp, menus};

#[test]
fn type_font_menu_lists_every_available_family() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    let fonts = |app: &VectorcraftApp| -> Vec<String> {
        menus::menu_entries(app).into_iter().filter(|e| e.path == ["Type", "Font"]).map(|e| e.label).collect()
    };
    let labels = fonts(&app);
    assert_eq!(labels, *FontDb::global().menu_family_list());
    let installed = FontDb::with_font_dirs(system_font_dirs());
    installed.load_system_fonts();
    let missing: Vec<String> = installed.menu_family_list().iter().filter(|f| !labels.contains(f)).cloned().collect();
    assert!(missing.is_empty(), "installed but not in Type › Font: {missing:?}");
    // The menu is built every frame, from a list built once.
    assert_eq!(fonts(&app), labels);
}

#[test]
fn the_character_panel_menu_refreshes_the_font_list() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    // The installed fonts are always listed: the menu offers to look for new ones instead.
    let text = crate::tests_labels::painted_text(&mut app, crate::panels::character::menu);
    assert!(text.contains("Refresh Font List") && !text.contains("System Fonts"), "{text}");
}
