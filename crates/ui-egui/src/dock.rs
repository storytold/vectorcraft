//! Panel bodies: what the dock area ([`crate::panel_docking`]) draws inside a docked, floating or
//! flyout panel.

use egui::Ui;

use crate::state::DockTab;
use crate::{VectorcraftApp, panels, widgets};

/// Panel `id`'s contents in the room its group gives it: Layers and Libraries fill it, Properties
/// scrolls in it.
pub(crate) fn panel_body(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    match DockTab::from_id(id) {
        Some(DockTab::Properties) => {
            widgets::panel_scroll(ui, egui::ScrollArea::vertical().id_salt("props-body").auto_shrink([false, true]), |ui| {
                panels::properties::show(app, ui)
            });
        }
        Some(DockTab::Layers) => panels::layers::show(app, ui),
        Some(DockTab::Libraries) => panels::libraries(app, ui),
        None => panels::show_icon_panel(app, ui, id),
    }
}
