#[cfg(not(target_arch = "wasm32"))]
use super::dialogs;
use super::{VectorcraftApp, browsable_url};

impl VectorcraftApp {
    /// Open a link in the browser (Help → Discord, website, GitHub…).
    pub fn open_url(&mut self, url: &str) {
        match self.services.open_url.as_mut() {
            Some(open) => open(url),
            None => self.pending_url = Some(url.to_string()),
        }
        self.ui.status = format!("Opened {url}");
    }

    /// Open `url`, which came from a document (an object's URL attribute), in the browser:
    /// [`browsable_url`]s only.
    pub(crate) fn open_document_url(&mut self, url: &str) -> Result<(), String> {
        browsable_url(url)?;
        self.open_url(url);
        Ok(())
    }

    /// Refuse UI command `id`, which hands something to another app, when automation sent it (a
    /// control or MCP request, or a frame of synthetic input): the automation roots don't confine
    /// what that app does.
    pub(crate) fn refuse_automation(&self, id: &str) -> Result<(), String> {
        if self.ui.answering_control || self.synthetic_frame {
            return Err(format!("{id} opens another app: only the person at the keyboard can run it"));
        }
        Ok(())
    }

    /// Run a Help link command and open the URL it returns.
    pub fn open_link(&mut self, id: &str) {
        if let Ok(v) = self.run(id, serde_json::json!({}))
            && let Some(u) = v["url"].as_str()
        {
            let u = u.to_string();
            self.open_url(&u);
        }
    }

    /// Linked files another app changed while their document is open: every two seconds, and as
    /// soon as the window comes back to the front, stamp the active document's linked files on a
    /// worker thread and act on the changed ones as Preferences › File Handling › Update Links
    /// says ([`vectorcraft_engine::link_watch`]). No look starts while a dialog is open, so an
    /// Ask When Modified question never goes unseen.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn tick_link_updates(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(r) = self.session.poll_link_scan() {
            match r {
                Ok(r) => {
                    let n = |k: &str| r[k].as_array().map_or(0, Vec::len);
                    if n("updated") > 0 {
                        self.status(format!("Updated {} linked file(s) changed on disk", n("updated")));
                    }
                    if n("ask") > 0 {
                        dialogs::missing_links::ask_update_changed(self, r["ask"].as_array().cloned().unwrap_or_default());
                    }
                    if n("modified") > 0 {
                        self.status(format!("{} linked image(s) changed on disk: Update Links shows the new versions", n("modified")));
                    }
                }
                Err(e) => self.status(e.to_string()),
            }
        }
        let (last_key, focus_key) = (egui::Id::unique("linkWatch.lastScan"), egui::Id::unique("linkWatch.focused"));
        let focused = ctx.input(|i| i.focused);
        let was_focused: bool = ctx.data(|d| d.get_temp(focus_key)).unwrap_or(focused);
        ctx.data_mut(|d| d.insert_temp(focus_key, focused));
        if self.ui.dialog.is_some() || self.session.active().is_none() {
            return;
        }
        let last: f64 = ctx.data(|d| d.get_temp(last_key)).unwrap_or(f64::NEG_INFINITY);
        // The active document as the last look saw it: its uid and revision.
        let (seen_key, doc) = (egui::Id::unique("linkWatch.seen"), self.session.active().map(|d| (d.uid, d.revision)));
        if now - last >= 2.0 || (focused && !was_focused) {
            // Walking the document's links costs a pass over it: only when a look is due.
            // `start_link_scan` starts none for a document without links, which then doesn't
            // wake the app for looks of its own (any frame still looks once one is due).
            ctx.data_mut(|d| d.insert_temp(last_key, now));
            self.session.start_link_scan();
            let links = self.session.link_scan.is_some();
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::unique("linkWatch.links"), links);
                d.insert_temp(seen_key, doc);
            });
        }
        if self.session.link_scan.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("linkWatch.links"))).unwrap_or(false) {
            ctx.request_repaint_after(std::time::Duration::from_millis(2000));
        } else if ctx.data(|d| d.get_temp::<Option<(u64, u64)>>(seen_key)).is_some_and(|seen| seen != doc) {
            // The document changed (a file placed into one without links) or another came to the
            // front since that look: wake for the next one even if the app sits idle until then,
            // or a file placed and changed meanwhile is first seen after the change, which is then
            // never acted on.
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((last + 2.0 - now).clamp(0.0, 2.0)));
        }
    }
}
