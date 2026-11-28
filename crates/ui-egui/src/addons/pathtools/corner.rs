//! The Anchor Inspector panel's Corners section: the Corner Tool's radius and kind, how many corners
//! the selection holds, Apply (`addon.pathtools.cornerSet`) and Set Corners… (its dialog, which
//! can also take every other anchor, or only the corners in an angle range). Canceling a Radius edit
//! or Corner choice restores its opening tool value; a retired edit supplies no replacement.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, Sizing, clicked};
use craft::ui::display::Text;
use craft::ui::pickers::Select;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::LiveCorners;
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_tools::addons::corner_tool::{ID, MAX_RADIUS, RADIUS};

use super::{row, run, typed};
use crate::canvas::rulers::general_lengths;
use crate::dialogs::corners::kind_label;
use crate::{VectorcraftApp, fields};

/// The menu's command: the tool, with the panel its values are in.
pub const COMMAND: &str = "addon.pathtools.cornerTool";

/// Object › Plug-ins › Path Precision Tools › Corner Tool.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.try_select_tool(ID)?;
    crate::panel_docking::reveal_panel(app, super::ID)
}

/// How many corners Apply would edit: the selected paths' direct-selected ones, or all of a path
/// selected whole.
pub(super) fn selected_corners(app: &VectorcraftApp) -> usize {
    let Some(st) = app.session.active() else { return 0 };
    let of = |id| {
        let lc = st.doc.node(id).and_then(LiveCorners::of)?;
        Some(st.selection.partial(id).filter(|a| !a.is_empty()).map_or(lc.corners.len(), |a| lc.corners_of(a).len()))
    };
    st.selection.objects.iter().filter_map(|id| of(*id)).sum()
}

pub(super) fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let options = app.session.tool_options_of(ID);
    let radius = options["radius"].as_f64().unwrap_or(RADIUS);
    let kind = serde_json::from_value::<CornerKind>(options["kind"].clone()).unwrap_or_default();
    let count = selected_corners(app);
    let lengths = general_lengths(app);
    let entries: Vec<Entry<CornerKind>> = CornerKind::ALL
        .into_iter()
        .map(|k| {
            let label = kind_label(k);
            Entry::new(k, tl!(label))
        })
        .collect();
    Text::new(tl!("Corners")).show(ui);
    Text::new(crate::i18n::fmt(tl!("Corners selected: {count}"), &[("count", &count.to_string())])).show(ui);
    let mut set: Option<Value> = None;
    let radius_field = |ui: &mut Ui| {
        let field = fields::size_field(ui, tl!("Radius"), Some(radius), lengths).range(0.0..=MAX_RADIUS);
        typed(field.show(ui).events.landed())
    };
    if let Some(v) = row(ui, "Radius", "corner-tool-radius", radius_field) {
        set = Some(json!({ "radius": v }));
    }
    let kind_field = |ui: &mut Ui| Select::new(tl!("Corner"), Bind::given(kind), &entries).show(ui).events.landed().copied();
    if let Some(k) = row(ui, "Corner", "corner-tool-kind", kind_field) {
        set = Some(json!({ "kind": k }));
    }
    let apply = clicked(&Button::new(tl!("Apply to Selected Anchors")).sizing(Sizing::FillWidth).enabled(count > 0).show(ui));
    let dialog = clicked(&Button::new(tl!("Set Corners…")).sizing(Sizing::FillWidth).enabled(count > 0).show(ui));
    if let Some(values) = set {
        run(app, "tool.setOption", json!({ "tool": ID, "values": values }));
    }
    if apply {
        run(app, "addon.pathtools.cornerSet", json!({ "radius": radius, "kind": kind }));
    }
    if dialog {
        crate::menus::invoke(app, super::set_corners::COMMAND, json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers};
    use vectorcraft_tools::addons::shared_edge;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("tool.setOption", json!({"tool": ID, "values": {"radius": 12.5, "kind": "round"}})).unwrap();
        app.run("tool.setOption", json!({"tool": shared_edge::ID, "values": {"diameter": 14.25, "tolerance": 0.5}})).unwrap();
        app
    }

    #[test]
    fn canceling_a_corner_kind_popup_restores_the_tools_complete_options() {
        let mut panel = Panel::new(app(), section);
        let before = panel.app().session.tool_options_of(ID);
        let doc = panel.app().session.active().unwrap().doc.clone();
        let revision = panel.app().session.active().unwrap().revision;
        let undo = panel.app().session.active().unwrap().history.undo.len();
        panel.click_role("Corner", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.frame();
        let proposed = panel.app().session.tool_options_of(ID);
        assert_ne!(proposed["kind"], before["kind"], "the choice previews the next corner kind");
        panel.key(Key::Escape, Modifiers::NONE);
        assert_eq!(panel.app().session.tool_options_of(ID), before);
        panel.click_role("Corner", Role::COMBO_BOX);
        panel.key(Key::ArrowDown, Modifiers::NONE);
        panel.key(Key::Enter, Modifiers::NONE);
        assert_eq!(panel.app().session.tool_options_of(ID), proposed, "Enter accepts the new tool setting");
        let app = panel.app();
        let st = app.session.active().unwrap();
        assert_eq!(st.doc, doc);
        assert_eq!(st.revision, revision);
        assert_eq!(st.history.undo.len(), undo, "tool settings do not edit the drawing");
    }

    #[test]
    fn canceling_an_option_text_edit_restores_options_and_enter_keeps_accepted_edits() {
        for (draw, tool, label, key, typed) in [
            (section as fn(&mut VectorcraftApp, &mut Ui), ID, "Radius", "radius", "19.75"),
            (super::super::shared_edge::section, shared_edge::ID, "Diameter", "diameter", "28.5"),
            (super::super::shared_edge::section, shared_edge::ID, "Coincidence", "tolerance", "0.75"),
        ] {
            for cancel in [false, true] {
                let mut panel = Panel::new(app(), draw);
                let before = panel.app().session.tool_options_of(tool);
                let doc = panel.app().session.active().unwrap().doc.clone();
                let revision = panel.app().session.active().unwrap().revision;
                let undo = panel.app().session.active().unwrap().history.undo.len();
                panel.click_role(label, Role::SPIN_BUTTON);
                panel.key(Key::A, Modifiers::COMMAND);
                panel.type_text(typed);
                panel.frame();
                let proposed = panel.app().session.tool_options_of(tool);
                assert_ne!(proposed[key], before[key], "{label} changes while typing");
                if cancel {
                    panel.key(Key::Escape, Modifiers::NONE);
                    assert_eq!(panel.app().session.tool_options_of(tool), before, "{label} restores on Escape");
                } else {
                    panel.key(Key::Enter, Modifiers::NONE);
                }
                panel.frame();
                assert_eq!(panel.app().session.tool_options_of(tool), if cancel { before } else { proposed });
                let app = panel.app();
                let st = app.session.active().unwrap();
                assert_eq!(st.doc, doc);
                assert_eq!(st.revision, revision);
                assert_eq!(st.history.undo.len(), undo, "{label} only changes future tool options");
            }
        }
    }
}
