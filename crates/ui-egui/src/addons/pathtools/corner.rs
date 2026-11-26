//! The Anchor Inspector panel's Corners section: the Corner Tool's radius and kind, how many corners
//! the selection holds, Apply (`addon.pathtools.cornerSet`) and Set Corners… (its dialog, which
//! can also take every other anchor, or only the corners in an angle range).

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
        typed(field.show(ui).value())
    };
    if let Some(v) = row(ui, "Radius", "corner-tool-radius", radius_field) {
        set = Some(json!({ "radius": v }));
    }
    let kind_field = |ui: &mut Ui| Select::new(tl!("Corner"), Bind::given(kind), &entries).show(ui).value().copied();
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
