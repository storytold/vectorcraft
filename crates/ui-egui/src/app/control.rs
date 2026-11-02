use super::{
    MAX_PENDING_SCREENSHOTS, MAX_SCREENSHOT_BYTES, MAX_SCREENSHOT_TOTAL_BYTES, PendingScreenshot, QueuedScreenshot, SCREENSHOT_TIMEOUT_MS,
    VectorcraftApp, file_access, json, now_ms,
};
use crate::control;

impl VectorcraftApp {
    pub(crate) fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        let mut handled = 0;
        while handled < control::MAX_CONTROL_REQUESTS_PER_FRAME {
            let Ok(req) = rx.try_recv() else { break };
            // Its caller stopped waiting (the app was busy or its window not drawn for too long)
            // and was told it won't run: an edit applied now would come from nowhere (#1725).
            if !req.claim() {
                continue;
            }
            handled += 1;
            let reply = req.reply.clone();
            // Guarded per request: a panic must not drop the channel (taken out of `self` above).
            let roots = self.automation_roots.clone();
            self.ui.answering_control = true;
            let answer = file_access::confine(roots.as_ref(), || vectorcraft_engine::guard::catch_panic(|| control::handle(self, ctx, &req)));
            self.ui.answering_control = false;
            let outcome = match answer {
                Ok(outcome) => outcome,
                Err(msg) => {
                    self.picks.recover();
                    control::err(format!("internal error: {msg} (please report this bug)"))
                }
            };
            match outcome {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path, data } => {
                    // Admit with the shared ceilings (4 requests, 128 MiB each, 256 MiB together)
                    // before issue, reserving the checked bytes this viewport would need.
                    match self.screenshot_pixels(ctx) {
                        Ok((width, height, bytes)) if bytes > MAX_SCREENSHOT_BYTES => {
                            let _ = reply.send(json!({"ok": false, "error": format!(
                                "screenshot would use {bytes} bytes ({width}×{height}), over the {MAX_SCREENSHOT_BYTES}-byte limit"
                            )}));
                        }
                        Ok((width, height, bytes)) if data && path.is_none() && bytes > control::MAX_SCREENSHOT_PNG_REPLY_BYTES => {
                            let _ = reply.send(json!({"ok": false, "error": format!(
                                "a PNG reply of this {width}×{height} screenshot ({bytes} bytes of pixels) would exceed the {}-byte reply-encoding limit; write it to a path instead",
                                control::MAX_SCREENSHOT_PNG_REPLY_BYTES
                            )}));
                        }
                        Ok((_, _, _)) if self.pending_screenshots.len() >= MAX_PENDING_SCREENSHOTS => {
                            let _ = reply.send(json!({"ok": false, "error": format!(
                                "already {MAX_PENDING_SCREENSHOTS} screenshots are waiting; wait for one to finish"
                            )}));
                        }
                        Ok((width, height, bytes)) if self.screenshot_reserved.saturating_add(bytes) > MAX_SCREENSHOT_TOTAL_BYTES => {
                            let _ = reply.send(json!({
                                "ok": false,
                                "error": format!(
                                    "screenshots already reserved {} bytes; {bytes} more ({width}×{height}) would pass the {MAX_SCREENSHOT_TOTAL_BYTES}-byte limit",
                                    self.screenshot_reserved
                                )
                            }));
                        }
                        Ok((_, _, bytes)) => {
                            self.screenshot_token += 1;
                            let id = self.screenshot_token;
                            let settle = ctx.global_style().animation_time as f64 * 2000.0 + 80.0;
                            let due_ms = now_ms() + settle;
                            self.screenshot_reserved = self.screenshot_reserved.saturating_add(bytes);
                            self.queued_screenshots.push(QueuedScreenshot { id, due_ms, frames: 0 });
                            let document = self.document_identity();
                            self.pending_screenshots.push(PendingScreenshot {
                                id,
                                path,
                                data,
                                reply,
                                deadline_ms: due_ms + SCREENSHOT_TIMEOUT_MS,
                                reserved_bytes: bytes,
                                document,
                                slot: None,
                                issued: None,
                                attempts: 0,
                            });
                        }
                        Err(e) => {
                            let _ = reply.send(json!({"ok": false, "error": e}));
                        }
                    }
                }
            }
        }
        if handled == control::MAX_CONTROL_REQUESTS_PER_FRAME {
            // Leave later requests on the channel and schedule another frame to serve them.
            ctx.request_repaint();
        }
        self.control_rx = Some(rx);
    }

    /// The active document's identity: its process-unique uid and revision. A capture records it
    /// at admission and issuance and refuses to answer with a different document or undo state.
    pub(super) fn document_identity(&self) -> Option<(u64, u64)> {
        self.session.active().map(|document| (document.uid, document.revision))
    }
}
