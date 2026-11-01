use super::{GeometryStamp, InputGeneration, VectorcraftApp, canvas};

impl VectorcraftApp {
    /// The host iteration count: one increment per `App::logic`/`App::ui` iteration (or a
    /// logic-only one), for panel state that asks "was this drawn in the last iteration?".
    pub(crate) fn frame_iteration(&self) -> u64 {
        self.host_iteration
    }

    /// The canvas on screen in document coordinates (none before it is laid out).
    pub fn screen_frame(&self) -> Option<vectorcraft_tools::ScreenFrame> {
        let (rect, view) = (self.canvas_rect?, self.view()?);
        let xf = canvas::Xf::new(rect, view);
        let px = |x: f32, y: f32| xf.delta_to_doc(egui::vec2(x, y));
        Some(vectorcraft_tools::ScreenFrame {
            origin: xf.to_doc(rect.left_top()),
            right: px(1.0, 0.0),
            down: px(0.0, 1.0),
            size: (f64::from(rect.width()), f64::from(rect.height())),
        })
    }
}

impl VectorcraftApp {
    /// Is the window minimized or fully occluded (as far as the host reports)?
    pub(crate) fn viewport_hidden(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| {
            let v = i.viewport();
            v.minimized == Some(true) || v.occluded == Some(true)
        })
    }

    /// Is there work that only a UI pass can do, so a hidden window must be asked to paint?
    pub(super) fn needs_hidden_paint(&self) -> bool {
        !self.synthetic.is_empty()
            || self.synthetic_injected
            || !self.queued_screenshots.is_empty()
            || self.pending_screenshots.iter().any(|p| p.slot.is_none())
    }

    /// Capture the viewport identity and geometry (content and inner rectangles, pixels per point and
    /// zoom) whose change advances the screenshot epoch.
    fn geometry_stamp(ctx: &egui::Context) -> GeometryStamp {
        GeometryStamp {
            viewport: ctx.viewport_id(),
            content: ctx.content_rect(),
            inner: ctx.input(|i| i.viewport().inner_rect),
            pixels_per_point: ctx.pixels_per_point(),
            zoom: ctx.zoom_factor(),
        }
    }

    /// Advance the capture geometry epoch when any part of the window geometry changed, even back
    /// to a previous value: a capture issued before the change must not be saved as if it showed
    /// the new one.
    pub(super) fn advance_geometry_epoch(&mut self, ctx: &egui::Context) {
        let stamp = Self::geometry_stamp(ctx);
        if self.geometry_stamp != Some(stamp) {
            self.geometry_stamp = Some(stamp);
            self.geometry_epoch = self.geometry_epoch.wrapping_add(1);
        }
    }

    /// Identify the document, modal dialog and focused widget that the current accepted input batch
    /// belongs to.
    pub(super) fn input_generation(&self) -> InputGeneration {
        InputGeneration {
            document: self.session.active().map_or(0, |document| document.uid),
            dialog: self.ui.dialog.as_ref().map(|dialog| egui::Id::unique(&dialog.kind)),
            focus: self.ui_ctx.as_ref().and_then(|ctx| ctx.memory(|memory| memory.focused())),
        }
    }
}
