//! The Anchor Inspector panel's Corners section: the Corner Tool's radius and kind, how many corners
//! the selection holds, and Apply (`addon.pathtools.cornerSet`).

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::LiveCorners;
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_tools::addons::corner_tool::{ID, MAX_RADIUS, RADIUS};

use super::{LABEL_W, run};
use crate::VectorcraftApp;
use crate::dialogs::corners::kind_label;
use crate::widgets::{self, dim_label};

/// The menu's command: the tool, with the panel its values are in.
pub const COMMAND: &str = "addon.pathtools.cornerTool";
const FIELD_W: f32 = 96.0;

/// Object › Plug-ins › Path Precision Tools › Corner Tool.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.select_tool(ID);
    app.run("window.panel", json!({ "panel": super::ID }))
}

/// How many corners Apply would edit: the selected paths' direct-selected ones, or all of a path
/// selected whole.
fn selected_corners(app: &VectorcraftApp) -> usize {
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
    let unit = app.session.general_unit();
    widgets::subheader(ui, "Corners");
    dim_label(ui, &crate::i18n::fmt(tl!("Corners selected: {count}"), &[("count", &count.to_string())]));
    let mut set: Option<Value> = None;
    widgets::label_row(ui, tl!("Radius:"), LABEL_W, |ui| {
        if let Some(v) = widgets::size_field(ui, "corner-tool-radius", Some(radius), unit, FIELD_W) {
            set = Some(json!({ "radius": v.clamp(0.0, MAX_RADIUS) }));
        }
    });
    widgets::label_row(ui, tl!("Corner:"), LABEL_W, |ui| {
        let labels = CornerKind::ALL.map(kind_label);
        if let Some(k) = widgets::dropdown(ui, "corner-tool-kind", kind_label(kind), &labels, FIELD_W).and_then(|i| CornerKind::ALL.get(i)) {
            set = Some(json!({ "kind": k }));
        }
    });
    let width = ui.available_width();
    let apply = ui.add_enabled_ui(count > 0, |ui| widgets::flat_button(ui, tl!("Apply to Selected Anchors"), width).clicked()).inner;
    if let Some(values) = set {
        run(app, "tool.setOption", json!({ "tool": ID, "values": values }));
    }
    if apply {
        run(app, "addon.pathtools.cornerSet", json!({ "radius": radius, "kind": kind }));
    }
}
