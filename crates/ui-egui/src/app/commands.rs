use super::fonts::MAX_RECENT_FONTS;
use super::*;

impl VectorcraftApp {
    /// Run a UI or engine command by id. The single entry point for every frontend path.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // A file dialog it shows off the UI thread runs it again with the path picked.
        if self.picks.is_entry_free() {
            let entry = picks::Entry::Command(id.to_string(), params.clone());
            return picks::as_entry(self, move || entry, |app| app.run_now(id, params));
        }
        self.run_now(id, params)
    }

    /// [`Self::run`] it, inside what asks for file dialogs.
    fn run_now(&mut self, id: &str, params: Value) -> Result<Value, String> {
        crate::state::sync_zoom_stops_display_print_size(self.session.prefs.display_print_size);
        self.run_count = self.run_count.wrapping_add(1);
        // In debug builds the app's own calls are held to the params descriptions too (#1442): a
        // panel passing a key its command doesn't read shows up in the tests.
        if cfg!(debug_assertions) {
            menus::check_params(&self.session, id, &params)?;
        }
        let artboards = self.artboard_tool_state();
        if let Some(r) = menus::run_ui_command(self, id, &params) {
            self.follow_artboard_tool(artboards);
            return r;
        }
        let mut params = params;
        // Paste in place, in front and in back (#693), Select › All on Active Artboard (#1006), and Align,
        // Distribute Objects and Distribute Spacing with Align to Artboard act on the active artboard, the
        // view's, which the app passes as `artboard`. A batch passes it on to those steps.
        if (vectorcraft_engine::ON_ACTIVE_ARTBOARD.contains(&id) || id == "command.batch")
            && params.get("artboard").is_none()
            && self.view().is_some()
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
            && let Some(p) = params.as_object_mut()
        {
            p.insert("artboard".into(), serde_json::json!(panels::artboards::selected(self, n)));
        }
        // Object › Artboards › Fit to Artwork Bounds resizes the selected artboards (the Artboard
        // tool's, else the active one), which the app passes as its `indices`, and Fit to Selected
        // Art resizes the active artboard, which the app passes as its `index`.
        if vectorcraft_engine::FIT_ACTIVE_ARTBOARD.contains(&id)
            && params.get("index").is_none()
            && params.get("indices").is_none()
            && self.view().is_some()
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
            && let Some(p) = params.as_object_mut()
        {
            if id == "artboard.fitToArt" {
                p.insert("indices".into(), serde_json::json!(panels::artboards::picked(self, n)));
            } else {
                p.insert("index".into(), serde_json::json!(panels::artboards::selected(self, n)));
            }
        }
        if id.starts_with("edit.paste") {
            if let Err(e) = self.adopt_system_clipboard() {
                self.ui.status = e.clone();
                return Err(e);
            }
            // Paste (also without formatting) goes to the centre of the view.
            if matches!(id, "edit.paste" | "edit.pasteWithoutFormatting")
                && ["center", "dx", "dy"].iter().all(|k| params.get(k).is_none())
                && let Some(c) = self.view().filter(|v| v.fitted).map(|v| v.center)
                && let Some(p) = params.as_object_mut()
            {
                p.insert("center".into(), serde_json::json!([c.x, c.y]));
            }
            if let Some(r) = dialogs::swatch_conflict::ask(self, id, &params) {
                return r;
            }
        }
        // Native files carry the view they reopen at.
        if fileio::SaveMode::of(id).is_some() {
            io::remember_view(self);
        }
        // A command that changes the edited text is an application edit (undo, a panel or menu
        // edit, a control request). Capture the bounded committed context around it so `Other`
        // dominates the frame even if an IME receipt follows. Selection/view commands that leave
        // the committed text and caret unchanged record nothing.
        let ime_view = self.view_info();
        let ime_target = crate::ime::target(&mut self.session, ime_view);
        let ime_before = crate::ime::stamp(&mut self.session, ime_view);
        let r = self.session.execute(id, &params).map_err(|e| e.to_string());
        if r.is_ok()
            && let Some(ctx) = self.ui_ctx.clone()
        {
            crate::ime::note_application_edit(self, &ctx, ime_view, ime_target, ime_before);
        }
        if r.is_ok() && matches!(id, "edit.copy" | "edit.cut") {
            self.publish_clipboard();
        }
        self.sync_views();
        // File › Revert replaces the document in its tab, keeps the tab's view and starts the tool afresh.
        self.follow_artboard_tool(artboards);
        match &r {
            Err(e) => self.ui.status = e.clone(),
            Ok(v) => {
                // Image Trace's notes (a large image traces more slowly, or was scaled down).
                if id.starts_with("imageTrace.make")
                    && let Some(w) = v["warnings"].as_array().and_then(|w| w.first()).and_then(Value::as_str)
                {
                    self.ui.status = w.to_string();
                }
                if id == "file.new" {
                    self.ui.status.clear();
                    // New Document's Pixel preview mode (Overprint Preview is the engine's).
                    if v["previewMode"] == "pixel" {
                        self.ui.view.pixel_preview = true;
                    }
                }
                // Choosing a data set, or stepping to one, clears the status bar. The reply's `warning`
                // below then shows when no object is bound to a variable the data set has a value for.
                if matches!(id, "dataset.select" | "dataset.next" | "dataset.prev") {
                    self.ui.status.clear();
                }
                if matches!(id, "text.setStyle" | "text.setRangeStyle")
                    && let Some(font) = params.get("font").and_then(Value::as_str)
                {
                    let r = &mut self.ui.recent_fonts;
                    r.retain(|f| !f.eq_ignore_ascii_case(font));
                    r.insert(0, font.to_string());
                    r.truncate(MAX_RECENT_FONTS);
                }
                // A command's warning (Rasterize's when an image couldn't be decoded) shows in the
                // status bar, as a tool preview's does.
                if let Some(w) = v.get("warning").and_then(Value::as_str) {
                    self.ui.status = w.to_string();
                }
            }
        }
        r
    }

    /// Select a tool (also used by the toolbar and shortcuts).
    pub fn select_tool(&mut self, id: &str) {
        // The tip of a pen whose eraser end selected the Eraser doesn't undo this choice.
        self.pen_end.forget_tool();
        self.activate_tool(id, true);
    }

    /// Go back to tool `id` after a tool that stood in for it (the pen's eraser end, [`stylus`]):
    /// a perspective grid hidden meanwhile stays hidden.
    pub(crate) fn return_to_tool(&mut self, id: &str) {
        self.activate_tool(id, false);
    }

    /// Make tool `id` active: chosen afresh (`fresh`, [`vectorcraft_engine::Session::select_tool`]),
    /// or back from a tool that stood in for it ([`vectorcraft_engine::Session::switch_tool`]).
    fn activate_tool(&mut self, id: &str, fresh: bool) {
        let v = self.view_info();
        let before = self.session.tool_id();
        let switched = if fresh { self.session.select_tool(id, v) } else { self.session.switch_tool(id, v) };
        if let Err(e) = switched {
            self.ui.status = e.to_string();
        }
        // A newly chosen Artboard tool starts on the active artboard (the view's).
        if before != "artboard"
            && self.session.tool_id() == "artboard"
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len())
        {
            panels::artboards::select(self, panels::artboards::selected(self, n));
        }
        if let Some(g) = vectorcraft_tools::catalog::group_of(id)
            && let Some(slot) = self.ui.group_tool.get_mut(g)
        {
            *slot = id.to_string();
        }
        toolbar::remember(self, id);
        self.ui.flyout = None;
    }

    /// Show a transient status message.
    pub fn status(&mut self, s: impl Into<String>) {
        self.ui.status = s.into();
    }
}
