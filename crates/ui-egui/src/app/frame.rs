use super::VectorcraftApp;

impl VectorcraftApp {
    /// The canvas on screen in document coordinates (none before it is laid out).
    pub fn screen_frame(&self) -> Option<vectorcraft_tools::ScreenFrame> {
        let view = self.canvas_view()?;
        Some(vectorcraft_tools::ScreenFrame {
            origin: view.to_doc(view.rect.left_top()),
            right: view.unmap_vec(egui::vec2(1.0, 0.0)),
            down: view.unmap_vec(egui::vec2(0.0, 1.0)),
            size: (f64::from(view.rect.width()), f64::from(view.rect.height())),
        })
    }

    /// Is the window minimized or fully occluded (as far as the host reports)?
    pub(crate) fn viewport_hidden(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            let v = i.viewport();
            v.minimized == Some(true) || v.occluded == Some(true)
        })
    }
}
