use craft_automation::screenshot::{DocumentId, Finished, settle_ms};
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use serde_json::json;
use vectorcraft_engine::file_access::AutomationRoots;

use super::{VectorcraftApp, file_access};
use crate::control;
use crate::services::WriteFn;

/// Writes a capture's PNG where the app's file authority allows: through the host's writer, inside
/// the automation roots. An error shows the `path` sent as its file name only when it lies outside
/// the read root, as every reply does (`hiding`).
fn write_automation_file(
    roots: Option<&std::sync::Arc<AutomationRoots>>,
    write: &mut Option<WriteFn>,
    hiding: &control::Hiding,
    path: &str,
    png: &[u8],
) -> Result<(), String> {
    file_access::confine(roots, || {
        let written = match write {
            Some(write) => write(path, png),
            None => Err("no writer configured".to_owned()),
        };
        // PLAN: assumed `Hiding::hide_error(&self, error: &str, sent: &Value) -> String` (hub-w2's split of `hide_in_deferred_reply`)
        written.map_err(|error| hiding.hide_error(&error, &json!({"path": path})))
    })
}

impl VectorcraftApp {
    pub(crate) fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        control::find::keep(ctx);
        let mut handled = 0;
        while handled < control::MAX_CONTROL_REQUESTS_PER_FRAME {
            let Some(req) = self.deferred_control.take().or_else(|| rx.try_recv().ok()) else { break };
            // Synthetic input sent before this tool action was dropped (see
            // `discard_stale_injected_batch`): it would reach the tool without that input.
            let tool_action = matches!(req.method.as_str(), "ui.pointer" | "ui.tool.select");
            if tool_action && req.made_before(self.synthetic_dropped_before) {
                // One its caller gave up is skipped without counting toward this frame's limit.
                if req.claim() {
                    handled += 1;
                    let _ = req.reply.send(json!({
                        "ok": false,
                        "error": "input sent before this tool action was dropped because the document, dialog or focus changed"
                    }));
                }
                continue;
            }
            // `ui.pointer` and `ui.tool.select` act at once, while `ui.key` and other synthetic
            // input reach the tool in a later UI pass. Wait for that earlier input, so a key
            // pressed during a drag reaches its tool before a pointer event or tool switch.
            // One its caller gave up meanwhile is dropped below instead of holding back the rest.
            // `ui.find` and a `control` target read the screen the earlier input leaves.
            if (tool_action || control::waits_for_input(&req)) && !req.given_up() && (!self.synthetic.is_empty() || self.synthetic_injected) {
                self.deferred_control = Some(req);
                ctx.request_repaint();
                break;
            }
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
            // Guarded per request: a panic must not stop the requests behind it.
            let answer = file_access::confine(roots.as_ref(), || vectorcraft_engine::guard::catch_panic(|| control::handle(self, ctx, &req)));
            self.ui.answering_control = false;
            let reply = match answer {
                Ok(reply) => reply,
                Err(msg) => {
                    self.picks.recover();
                    self.session.set_replaying(false);
                    control::err(format!("internal error: {msg} (please report this bug)"))
                }
            };
            // PLAN: `job_finished` for a `ui.pointer` reply that waits on the tool's edit (`Reply::AfterJob`) is called where that edit is applied (hub-w2, chunk plan w9)
            let document = self.document_identity();
            self.channel.settle(req, reply, document, settle);
        }
        self.channel.poll_input(&host);
    }

    /// The UI phase of a pass: counts the pass toward the captures' settling and issues the due
    /// ones; a capture that expired or lost its window is answered here.
    pub(crate) fn issue_captures(&mut self, ctx: &egui::Context) {
        let document = self.document_identity();
        self.channel.captures.note_pass();
        let finished = self.channel.captures.issue(&WindowHost::of(ctx), motion::now(), document);
        self.finish_captures(finished);
    }

    /// Replies to finished captures; a capture with a path is written by [`write_automation_file`].
    fn finish_captures(&mut self, finished: Vec<Finished>) {
        if finished.is_empty() {
            return;
        }
        let hiding = control::deferred_hiding(self);
        let roots = self.automation_roots.clone();
        let write = &mut self.services.write;
        self.channel.finish_captures(finished, &mut |path, png| write_automation_file(roots.as_ref(), write, &hiding, path, png));
    }

    /// The active document's identity: its process-unique uid and revision. A capture records it
    /// at admission and issuance and refuses to answer with a different document or undo state.
    pub(super) fn document_identity(&self) -> Option<DocumentId> {
        self.session.active().map(|document| DocumentId::new(document.uid, document.revision))
    }
}
