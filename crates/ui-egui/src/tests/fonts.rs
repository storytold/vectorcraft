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
    let installed = FontDb::with_font_dirs(system_font_dirs());
    installed.load_system_fonts();
    // A test running alongside can add a font (and the installed fonts' scan can end), and the menu
    // is then built again: judge a run in which the fonts held still.
    for _ in 0..20 {
        let generation = FontDb::global().generation();
        let labels = fonts(&app);
        let listed = FontDb::global().menu_family_list();
        // The menu is built every frame, from a list built once.
        let again = fonts(&app);
        if FontDb::global().generation() != generation {
            continue;
        }
        assert_eq!(labels, *listed);
        let missing: Vec<String> = installed.menu_family_list().iter().filter(|f| !labels.contains(f)).cloned().collect();
        assert!(missing.is_empty(), "installed but not in Type › Font: {missing:?}");
        assert_eq!(again, labels);
        return;
    }
    panic!("the fonts kept changing");
}

#[test]
fn the_character_panel_menu_refreshes_the_font_list() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({})).unwrap();
    // The installed fonts are always listed: the menu offers to look for new ones instead.
    let text = crate::tests_labels::painted_text(&mut app, crate::panels::character::menu);
    assert!(text.contains("Refresh Font List") && !text.contains("System Fonts"), "{text}");
}
