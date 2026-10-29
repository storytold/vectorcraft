use super::{
    Arc, CaptureEpoch, MAX_SCREENSHOT_BYTES, MAX_SCREENSHOT_TOTAL_BYTES, QueuedScreenshot, SCREENSHOT_SETTLE_FRAMES, SCREENSHOT_TIMEOUT_ERROR,
    VectorcraftApp, file_access, json, now_ms,
};
use crate::control;

impl VectorcraftApp {
    /// The image size (pixels) and checked byte count a capture of the current viewport would
    /// have: the window's content rect times the current DPI/zoom. The viewport's own `inner_rect`
    /// is the fresh source; `content_rect` is only a fallback for hosts that do not report one
    /// (Wayland, headless), and a logic-only pass leaves it from the last shown frame. Issuance in
    /// a real UI pass recomputes this and re-checks the budget, so a stale admission size cannot
    /// let an oversized image through.
    pub(super) fn screenshot_pixels(&self, ctx: &egui::Context) -> Result<(usize, usize, usize), String> {
        let rect = ctx.input(|i| i.viewport().inner_rect).unwrap_or_else(|| ctx.content_rect());
        let scale = f64::from(ctx.pixels_per_point());
        let pixels = |points: f32, what: &str| -> Result<usize, String> {
            let value = f64::from(points) * scale;
            if !value.is_finite() || !(1.0..=100_000.0).contains(&value) {
                return Err(format!("the window's {what} ({points} points × {scale} DPI) cannot size a screenshot"));
            }
            Ok(value.round() as usize)
        };
        let width = pixels(rect.width(), "width")?;
        let height = pixels(rect.height(), "height")?;
        let bytes = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| format!("a {width}×{height} screenshot's byte count overflows"))?;
        Ok((width, height, bytes))
    }

    /// Issue the first due capture of a settled UI pass, at most one in flight: the one-shot
    /// callback makes correlation trivial, and serializing bounds readback memory.
    pub(super) fn issue_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.iter().any(|p| p.slot.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        for q in &mut self.queued_screenshots {
            q.frames = q.frames.saturating_add(1);
        }
        let now = now_ms();
        let Some(pos) = self.queued_screenshots.iter().position(|q| now >= q.due_ms && q.frames >= SCREENSHOT_SETTLE_FRAMES) else {
            if !self.queued_screenshots.is_empty() {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            return;
        };
        let queued = self.queued_screenshots.remove(pos);
        let Some(index) = self.pending_screenshots.iter().position(|p| p.id == queued.id) else { return };
        let document = self.document_identity();
        if self.pending_screenshots[index].document != document {
            self.fail_screenshot(index, "the document changed while the screenshot was pending");
            return;
        }
        let (width, height, bytes) = match self.screenshot_pixels(ctx) {
            Ok(v) => v,
            Err(e) => {
                self.fail_screenshot(index, &e);
                return;
            }
        };
        let (data, path, previous) =
            (self.pending_screenshots[index].data, self.pending_screenshots[index].path.is_some(), self.pending_screenshots[index].reserved_bytes);
        if data && !path && bytes > control::MAX_SCREENSHOT_PNG_REPLY_BYTES {
            let e = format!(
                "a PNG reply of this {width}×{height} screenshot ({bytes} bytes of pixels) no longer fits the {}-byte reply-encoding limit; write it to a path instead",
                control::MAX_SCREENSHOT_PNG_REPLY_BYTES
            );
            self.fail_screenshot(index, &e);
            return;
        }
        if bytes > MAX_SCREENSHOT_BYTES || self.screenshot_reserved.saturating_sub(previous).saturating_add(bytes) > MAX_SCREENSHOT_TOTAL_BYTES {
            let e = format!("the window grew since the screenshot was admitted: {width}×{height} no longer fits the capture budget");
            self.fail_screenshot(index, &e);
            return;
        }
        self.screenshot_reserved = self.screenshot_reserved.saturating_sub(previous).saturating_add(bytes);
        let viewport = ctx.viewport_id();
        let (slot, callback) = craft_ui::capture::CaptureSlot::new(ctx, viewport, MAX_SCREENSHOT_BYTES);
        {
            let pending = &mut self.pending_screenshots[index];
            pending.reserved_bytes = bytes;
            pending.issued = Some(CaptureEpoch { geometry: self.geometry_epoch, viewport, document, width, height });
            pending.slot = Some(slot);
        }
        // A pending `Screenshot` asks for the paint it needs, even while hidden.
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(callback));
        ctx.request_repaint();
    }

    /// Answer a pending request with an error and release its reservation.
    fn fail_screenshot(&mut self, index: usize, error: &str) {
        if index >= self.pending_screenshots.len() {
            return;
        }
        let pending = self.pending_screenshots.remove(index);
        self.queued_screenshots.retain(|q| q.id != pending.id);
        self.screenshot_reserved = self.screenshot_reserved.saturating_sub(pending.reserved_bytes);
        if let Some(slot) = pending.slot {
            slot.cancel();
        }
        let _ = pending.reply.send(json!({"ok": false, "error": error}));
    }

    /// Take completed captures on the UI thread only, verify they still match the document and
    /// geometry they were issued for, and write them under the automation roots. A late completion
    /// (after a timeout or close) has already been canceled with its slot, so it writes nothing.
    ///
    /// This runs in `tick`, as both hosts call logic and the UI on the same thread. `save_screenshot`
    /// and `file_access::confine` here, and the `&mut self` write, must move with it if logic ever
    /// runs off the UI thread.
    pub(super) fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let mut completed: Vec<(usize, Result<Arc<egui::ColorImage>, craft_ui::capture::CaptureError>)> = vec![];
        for (index, pending) in self.pending_screenshots.iter().enumerate() {
            if let Some(slot) = &pending.slot
                && let Some(result) = slot.take()
            {
                completed.push((index, result));
            }
        }
        let now = now_ms();
        let document = self.document_identity();
        for (index, result) in completed.into_iter().rev() {
            let Some((id, issued, deadline)) = self.pending_screenshots.get(index).map(|p| (p.id, p.issued, p.deadline_ms)) else { continue };
            match result {
                // The frame came only after the request's deadline: it is late even though the
                // callback had stored it before this pass took it. Do not encode or write it.
                _ if now >= deadline => {
                    self.fail_screenshot(index, SCREENSHOT_TIMEOUT_ERROR);
                }
                Ok(_) if issued.is_some_and(|e| e.document != document) => {
                    self.fail_screenshot(index, "the document changed while the screenshot was pending");
                }
                Ok(image) => {
                    let size = (image.size[0], image.size[1]);
                    let stale =
                        issued.is_none_or(|e| e.geometry != self.geometry_epoch || e.viewport != ctx.viewport_id() || (e.width, e.height) != size);
                    if stale {
                        // Resize, DPI or zoom changed while the capture was in flight: a fresh slot
                        // and a fresh frame, not the old pixels. Attempts are bounded.
                        let retry = self.pending_screenshots[index].attempts == 0;
                        self.pending_screenshots[index].attempts = self.pending_screenshots[index].attempts.saturating_add(1);
                        self.pending_screenshots[index].slot = None;
                        self.pending_screenshots[index].issued = None;
                        if retry {
                            self.queued_screenshots.insert(0, QueuedScreenshot { id, due_ms: now_ms(), frames: 0 });
                        } else {
                            self.fail_screenshot(index, "the window changed while the screenshot was in flight");
                        }
                        continue;
                    }
                    let pending = self.pending_screenshots.remove(index);
                    self.screenshot_reserved = self.screenshot_reserved.saturating_sub(pending.reserved_bytes);
                    let roots = self.automation_roots.clone();
                    let _ = pending
                        .reply
                        .send(file_access::confine(roots.as_ref(), || control::save_screenshot(self, &image, pending.path.as_deref(), pending.data)));
                }
                Err(e) => {
                    let error = format!("the screenshot could not be used: {e}");
                    self.fail_screenshot(index, &error);
                }
            }
        }
        let timed_out: Vec<usize> = self.pending_screenshots.iter().enumerate().filter(|(_, p)| now >= p.deadline_ms).map(|(i, _)| i).collect();
        for index in timed_out.into_iter().rev() {
            self.fail_screenshot(index, SCREENSHOT_TIMEOUT_ERROR);
        }
        if !self.pending_screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
