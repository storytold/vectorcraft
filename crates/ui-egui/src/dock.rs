//! Panel bodies: what the dock area ([`crate::panel_docking`]) draws inside a docked, floating or
//! flyout panel.

use egui::Ui;

use crate::state::DockTab;
use crate::{VectorcraftApp, panels, widgets};

/// Panel `id`'s contents in a popped-out or floating panel `width` wide; the tabbed group's panels
/// fill (Layers) or scroll (Properties) `tall` points.
pub(crate) fn panel_body(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, width: f32, tall: f32) {
    egui::Frame::NONE.inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(width - 20.0);
        match DockTab::from_id(id) {
            Some(DockTab::Properties) => {
                // At least `tall` when the sections are taller: the flyout's area keeps the size its
                // content needs, and egui sizes a new area at most 400 points high, so a scroll area
                // that only took the room it was given never grew past it.
                widgets::panel_scroll(
                    ui,
                    egui::ScrollArea::vertical().id_salt("props-flyout").max_height(tall).min_scrolled_height(tall).auto_shrink([false, true]),
                    |ui| panels::properties::show(app, ui),
                );
            }
            Some(DockTab::Layers) => {
                ui.set_max_height(tall);
                panels::layers::show(app, ui);
            }
            Some(DockTab::Libraries) => {
                // Its list scrolls inside `tall` (as in the dock) instead of growing the flyout.
                ui.set_max_height(tall);
                panels::libraries(app, ui);
            }
            None => panels::show_icon_panel(app, ui, id),
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::theme;

    /// The main column's width in the default layout.
    const DOCK_WIDTH: f32 = 310.0;

    fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
        crate::panel_docking::show(app, ui);
    }

    #[test]
    fn the_properties_scroll_bar_covers_no_control() {
        use egui::accesskit::{Action, Role};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
        app.run("object.rasterize", json!({})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 600.0));
        let mut nodes = vec![];
        for k in 0..4 {
            let input = egui::RawInput { time: Some(f64::from(k) * 0.1), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(&mut app, ui));
            out.textures_delta.clear();
            nodes = out.platform_output.accesskit_update.unwrap().nodes;
        }
        let rect = |n: &egui::accesskit::Node| {
            n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
        };
        // The Properties panel's is the right-most (the icon column's slim strip bar is left of it).
        let bar = nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::ScrollBar)
            .filter_map(|(_, n)| rect(n))
            .max_by(|a, b| a.left().total_cmp(&b.left()))
            .filter(|bar| bar.left() > screen.right() - DOCK_WIDTH);
        let bar = bar.expect("the Properties panel scrolls in a 600 pt window");
        let mut covered = vec![];
        for (_, n) in &nodes {
            let Some(r) = rect(n) else { continue };
            let control = n.role() != Role::ScrollBar && (n.supports_action(Action::Click) || n.supports_action(Action::Focus));
            if control && r.intersect(bar).is_positive() {
                covered.push(format!("{:?} {:?} at {r:?}", n.role(), n.label()));
            }
        }
        assert!(covered.is_empty(), "under the scroll bar at {bar:?}: {covered:#?}");
    }

    /// #3443: the Layers list's scroll bar sits beside the rows, not over the target and selection
    /// marks at their right edge.
    #[test]
    fn the_layers_scroll_bar_covers_no_row() {
        use egui::accesskit::Role;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": k, "y": k, "width": 20, "height": 20})).unwrap();
        }
        app.run("window.panel", json!({"panel": "layers"})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1440.0, 600.0));
        let mut nodes = vec![];
        for k in 0..4 {
            let input = egui::RawInput { time: Some(f64::from(k) * 0.1), screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(&mut app, ui));
            out.textures_delta.clear();
            nodes = out.platform_output.accesskit_update.unwrap().nodes;
        }
        let rect = |n: &egui::accesskit::Node| {
            n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
        };
        let bar = nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::ScrollBar)
            .filter_map(|(_, n)| rect(n))
            .max_by(|a, b| a.left().total_cmp(&b.left()))
            .filter(|bar| bar.left() > screen.right() - DOCK_WIDTH)
            .expect("40 layers scroll in a 600 pt window");
        let rows: Vec<egui::Rect> = nodes
            .iter()
            .filter(|(_, n)| n.role() != Role::ScrollBar)
            .filter_map(|(_, n)| rect(n))
            .filter(|r| r.top() >= bar.top() && r.bottom() <= bar.bottom() && r.right() > bar.left() - 40.0)
            .collect();
        assert!(!rows.is_empty(), "the rows beside the bar are in the tree");
        let covered: Vec<_> = rows.iter().filter(|r| r.intersect(bar).is_positive()).collect();
        assert!(covered.is_empty(), "under the scroll bar at {bar:?}: {covered:#?}");
    }
}
