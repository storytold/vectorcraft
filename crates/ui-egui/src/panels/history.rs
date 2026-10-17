//! History panel: the document's states; click one to go back (or forward) to it.

use egui::{Sense, Ui, vec2};
use serde_json::json;

use crate::theme::Tokens;
use crate::widgets::menu_item;
use crate::{VectorcraftApp, icons};

/// Undo (negative) / redo (positive) steps to reach row `target` when `current` is the current
/// row (row 0 = the document as opened).
pub fn steps_to(current: usize, target: usize) -> i64 {
    target as i64 - current as i64
}

/// A step's name in `lang`: its plain row, else the row of the same name with an ellipsis (a menu
/// item's, such as an effect's) without the ellipsis; `None` when the catalog has neither.
pub fn step_name(lang: crate::i18n::Lang, label: &str) -> Option<String> {
    use crate::i18n::{has, tr};
    if has(lang, label) {
        return Some(tr(lang, label).to_string());
    }
    let dotted = format!("{label}…");
    has(lang, &dotted).then(|| tr(lang, &dotted).trim_end_matches('…').to_string())
}

/// A step's name as the panel lists it: in the UI language, else as recorded.
fn shown(label: &str) -> String {
    step_name(crate::i18n::current(), label).unwrap_or_else(|| label.to_string())
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        super::empty_state(ui, "history", tl!("No document"), tl!("Open a document to see its history."));
        return;
    };
    let mut rows: Vec<(String, bool)> = vec![(tl!("Open").to_string(), false)];
    rows.extend(st.history.undo.iter().map(|h| (shown(&h.label), false)));
    let current = rows.len() - 1;
    rows.extend(st.history.redo.iter().rev().map(|h| (shown(&h.label), true)));
    let mut steps: Option<i64> = None;
    crate::widgets::list_box(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt("hist").max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
            for (i, (label, future)) in rows.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                if i == current {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let icon = if i == 0 { "file-plus" } else { "history" };
                icons::paint(
                    ui,
                    icon,
                    egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(14.0, 14.0)),
                    if *future { t.text_disabled } else { t.icon },
                );
                let col = if *future { t.text_disabled } else { t.text };
                ui.painter().text(r.left_center() + vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), col);
                if resp.clicked() && i != current {
                    steps = Some(steps_to(current, i));
                }
            }
        });
    });
    if let Some(s) = steps {
        let (cmd, n) = if s < 0 { ("edit.undo", -s) } else { ("edit.redo", s) };
        for _ in 0..n {
            if app.run(cmd, json!({})).is_err() {
                break;
            }
        }
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    for (label, cmd) in [(tl!("Step Backward"), "edit.undo"), (tl!("Step Forward"), "edit.redo")] {
        if menu_item(ui, label, crate::menus::enabled(app, cmd), false) {
            crate::menus::invoke(app, cmd, json!({}));
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Set History Limit…"), true, false) {
        crate::menus::invoke(app, "edit.preferences", json!({ "category": "Performance" }));
    }
    menu_item(ui, tl!("History Options…"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps() {
        assert_eq!(steps_to(5, 2), -3);
        assert_eq!(steps_to(2, 4), 2);
        assert_eq!(steps_to(3, 3), 0);
    }

    /// A step named after an effect whose catalog row has only the menu label with its ellipsis
    /// takes that row's translation without the ellipsis; a name with no row has none.
    #[test]
    fn effect_steps_read_the_menu_row() {
        let ja = crate::i18n::Lang::from_code("ja").expect("ja registered");
        assert_eq!(step_name(ja, "Revolve").as_deref(), Some("回転体"));
        assert_eq!(step_name(ja, "Feather").as_deref(), Some("ぼかし"));
        assert_eq!(step_name(ja, "No Such Step"), None);
    }

    fn app_with_two_rectangles() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        for x in [10, 60] {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        }
        app
    }

    /// Returns the number of steps to undo and to redo.
    fn depth(app: &VectorcraftApp) -> (usize, usize) {
        let h = &app.session.active().unwrap().history;
        (h.undo.len(), h.redo.len())
    }

    /// Clicks the panel menu's item labeled `label`.
    fn click(app: &mut VectorcraftApp, label: &str) {
        let ctx = egui::Context::default();
        let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(240.0, 400.0)));
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ui| menu(app, ui));
            out.textures_delta.clear();
            out.shapes
        };
        let item = frame(app, vec![])
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().trim_start() == label => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no `{label}` in the panel menu"));
        let button = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, vec![egui::Event::PointerMoved(item), button(true), button(false)]);
    }

    /// The panel menu's Step Backward undoes one step and Step Forward redoes one, as Edit › Undo
    /// and Edit › Redo do.
    #[test]
    fn the_panel_menu_steps_backward_and_forward() {
        let mut app = app_with_two_rectangles();
        let objects = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
        assert_eq!((depth(&app), objects(&app)), ((2, 0), 2));
        click(&mut app, "Step Backward");
        assert_eq!((depth(&app), objects(&app)), ((1, 1), 1));
        click(&mut app, "Step Backward");
        assert_eq!((depth(&app), objects(&app)), ((0, 2), 0));
        click(&mut app, "Step Forward");
        assert_eq!((depth(&app), objects(&app)), ((1, 1), 1));
    }

    /// Step Backward is disabled when there is nothing to undo, and Step Forward is disabled when
    /// there is nothing to redo. A click on a disabled item changes nothing.
    #[test]
    fn the_step_items_are_disabled_with_nothing_to_undo_or_redo() {
        use crate::widgets::accessibility_tests::tree;
        let disabled = |app: &mut VectorcraftApp| {
            let tree = tree(|ui| menu(app, ui));
            let off =
                |label: &str| tree.iter().find(|n| n.label().map(str::trim) == Some(label)).unwrap_or_else(|| panic!("no `{label}`")).is_disabled();
            (off("Step Backward"), off("Step Forward"))
        };
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(disabled(&mut app), (true, true), "no document");
        app.session.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
        assert_eq!(disabled(&mut app), (true, true), "a new document");
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        assert_eq!(disabled(&mut app), (false, true), "one step to undo");
        click(&mut app, "Step Forward");
        assert_eq!(depth(&app), (1, 0));
        click(&mut app, "Step Backward");
        assert_eq!(disabled(&mut app), (true, false), "one step to redo");
        click(&mut app, "Step Backward");
        assert_eq!(depth(&app), (0, 1));
    }

    /// Set History Limit… opens Preferences at Performance, the page with History States.
    #[test]
    fn set_history_limit_opens_the_history_states_preference() {
        let mut app = app_with_two_rectangles();
        click(&mut app, "Set History Limit…");
        let d = app.ui.dialog.take().expect("a dialog");
        assert_eq!((d.kind.as_str(), d.str("__category")), ("preferences", "Performance".to_string()));
        assert!(d.fields.contains_key("historyStates"), "{:?}", d.fields.keys());
    }
}
