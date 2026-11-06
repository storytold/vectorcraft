//! Transient, non-interactive decoration. Only canvas painters call this; artwork renderers,
//! printing and export never see it. The link watch worker refreshes the ids off the UI thread.

use super::{Point, Shape, Stroke, Tokens, Xf};

pub(super) fn paint(st: &vectorcraft_engine::DocState, p: &egui::Painter, xf: &Xf, t: &Tokens) {
    for id in st.marked_missing_links() {
        let Some(b) = st.doc.node(id).and_then(vectorcraft_doc::Node::geometric_bounds) else { continue };
        let pts = xf.quad(b);
        let rect = egui::Rect::from_points(&pts);
        if !p.clip_rect().intersects(rect.expand(8.0)) {
            continue;
        }
        p.add(Shape::closed_line(pts, Stroke::new(2.0, t.error)));
        let at = xf.to_screen(Point::new(b.x0, b.y0));
        p.circle_filled(at, 7.0, t.error);
        p.text(at, egui::Align2::CENTER_CENTER, "?", crate::theme::semibold(11.0), egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{Document, ImageObject, LinkInfo, Node, NodeKind};

    fn state() -> vectorcraft_engine::DocState {
        let mut doc = Document::new(200.0, 150.0);
        let layer = doc.layers[0].id;
        let id = doc.alloc_id();
        doc.insert(
            Some(layer),
            0,
            Node::new(
                id,
                NodeKind::Image(Box::new(ImageObject {
                    key: "original-test-image".into(),
                    width: 40,
                    height: 20,
                    xf: vectorcraft_geom::Affine::IDENTITY,
                    link: Some(LinkInfo { path: "missing-marker-test.png".into(), ..Default::default() }),
                    placement: Default::default(),
                    pixels: None,
                })),
            ),
        )
        .unwrap();
        let mut st = vectorcraft_engine::DocState::new(doc, None);
        st.show_missing_links = true;
        st.missing_link_markers = Some((st.doc.clone(), vec![id]));
        st
    }

    #[test]
    fn missing_link_canvas_decoration_tracks_the_view_and_can_be_cleared() {
        let mut st = state();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (zoom, rotation) in [(0.25, 0.0), (1.0, 37.0), (8.0, 90.0)] {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
            let bounds = st.doc.node(st.marked_missing_links()[0]).unwrap().geometric_bounds().unwrap();
            let view = crate::state::View { zoom, rotation, center: bounds.center(), ..Default::default() };
            let xf = Xf::new(rect, &view);
            let raw = egui::RawInput { screen_rect: Some(rect), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| paint(&st, ui.painter(), &xf, &Tokens::get(ui.ctx())));
            let outline = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    Shape::Path(path) if path.closed && path.stroke.width == 2.0 => Some(path),
                    _ => None,
                })
                .expect("missing image has a red closed outline");
            assert_eq!(outline.points, xf.quad(bounds));
            assert_eq!(outline.stroke.color, egui::epaint::ColorMode::Solid(Tokens::get(&ctx).error));
            assert!(out.shapes.iter().any(|s| matches!(&s.shape, Shape::Circle(c) if c.radius == 7.0)), "fixed-size badge at every zoom");
            out.textures_delta.clear();
        }
        st.show_missing_links = false;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let xf = Xf::new(ui.max_rect(), &Default::default());
            paint(&st, ui.painter(), &xf, &Tokens::get(ui.ctx()));
        });
        assert!(!out.shapes.iter().any(|s| matches!(&s.shape, Shape::Path(_) | Shape::Circle(_))));
        out.textures_delta.clear();
    }

    #[test]
    fn missing_link_menu_command_uses_the_engine_toggle() {
        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({})).unwrap();
        assert_eq!(crate::menus::dynamic_label(&app, "view.showMissingLinks", ""), "Show Missing Links");
        app.run("view.showMissingLinks", serde_json::json!({"on": true})).unwrap();
        assert!(app.session.active().unwrap().show_missing_links);
        assert_eq!(crate::menus::dynamic_label(&app, "view.showMissingLinks", ""), "Hide Missing Links");
        app.run("view.showMissingLinks", serde_json::json!({"on": false})).unwrap();
        assert!(!app.session.active().unwrap().show_missing_links);
    }
}
