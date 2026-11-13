use super::fonts::MAX_RECENT_FONTS;
use super::{Value, VectorcraftApp, canvas, dialogs, fileio, io, menus, panels, picks, toolbar};
use crate::placing;
use crate::state::ReplaceFontWarning;

/// The last repeatable Pathfinder operation completed by a command or atomic batch.
fn completed_pathfinder_operation<'a>(id: &'a str, params: &'a Value) -> Option<&'a str> {
    let tracked = |id: &'a str| id.strip_prefix("object.pathfinder.").filter(|op| panels::pathfinder::operation_label(op).is_some());
    if id == "command.batch" {
        params.get("commands")?.as_array()?.iter().rev().find_map(|step| step.get("command").and_then(Value::as_str).and_then(tracked))
    } else {
        tracked(id)
    }
}

fn substitute_style_description(font: &Value) -> Option<String> {
    if font["status"] != "substitute" {
        return None;
    }
    let family = font["family"].as_str()?;
    let style = font["style"].as_str()?;
    let resolved_family = font["resolved"]["family"].as_str()?;
    let resolved_style = font["resolved"]["style"].as_str()?;
    Some(format!("{family} {style} is drawn in {resolved_family} {resolved_style}"))
}

fn substitute_warnings(fonts: &[Value], styles: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let mut missing = Vec::new();
    let mut live_styles = Vec::new();
    for font in fonts.iter().filter(|font| {
        styles.iter().any(|(family, style)| {
            font["family"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(family))
                && font["style"].as_str().is_some_and(|name| name.eq_ignore_ascii_case(style))
        })
    }) {
        match font["status"].as_str() {
            Some("missing") => {
                if let Some(family) = font["family"].as_str() {
                    missing.push(family.to_string());
                }
            }
            Some("substitute") => {
                if let Some(description) = substitute_style_description(font) {
                    live_styles.push(description);
                }
            }
            _ => {}
        }
    }
    missing.sort();
    missing.dedup();
    live_styles.sort();
    live_styles.dedup();
    (missing, live_styles)
}

#[cfg(test)]
#[test]
fn substitute_warnings_follow_current_font_inventory() {
    let styles = vec![("Source Sans 3".to_string(), "Black Wide".to_string())];
    let replaced_face = serde_json::json!([{
        "family": "Source Sans 3", "style": "Black Wide", "status": "substitute",
        "resolved": {"family": "Inter", "style": "Bold"}
    }]);
    assert_eq!(
        substitute_warnings(replaced_face.as_array().unwrap(), &styles),
        (Vec::<String>::new(), vec!["Source Sans 3 Black Wide is drawn in Inter Bold".to_string()])
    );
    let removed_family = serde_json::json!([{"family": "Source Sans 3", "style": "Black Wide", "status": "missing"}]);
    assert_eq!(substitute_warnings(removed_family.as_array().unwrap(), &styles), (vec!["Source Sans 3".to_string()], Vec::<String>::new()));
}

impl VectorcraftApp {
    /// Run a UI or engine command by id. The single entry point for every frontend path.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.beacon.command(id);
        let touch_workspace = self.session.prefs.touch_workspace;
        // A file dialog it shows off the UI thread runs it again with the path picked.
        let result = if self.picks.is_entry_free() {
            let entry = picks::Entry::Command(id.to_string(), params.clone());
            picks::as_entry(self, move || entry, |app| app.run_now(id, params))
        } else {
            self.run_now(id, params)
        };
        if id == "prefs.import" && result.is_ok() {
            crate::prefs_dialog::imported(self, touch_workspace);
        }
        placing::settle_swatch_paste(self);
        result
    }

    /// [`Self::run`] for saved steps (an action's): keys their commands no longer read are taken,
    /// as [`vectorcraft_engine::Session::replay`] takes them, so an action recorded by an older
    /// version still plays.
    pub fn replay(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let outer = self.session.set_replaying(true);
        let result = self.run(id, params);
        self.session.set_replaying(outer);
        result
    }

    /// A command that waits for the edit the tool computes on another thread and the input held
    /// for it waits before the app adds the view's active artboard to its params: a held press
    /// may change it. The app's own commands (its dialogs, panels and view) wait too.
    fn wait_for_tool_edit(&mut self, id: &str) {
        let ui_command = vectorcraft_engine::find_command(id).is_none();
        if (ui_command || vectorcraft_engine::waits_for_tool_edit(id)) && self.session.tool_waiting() {
            let r = self.session.finish_tool_edit();
            canvas::apply_requests(self, r);
        }
    }

    /// Paste in place, in front and in back (#693), Select › All on Active Artboard (#1006), and Align,
    /// Distribute Objects and Distribute Spacing with Align to Artboard, and Edit Symbol from the Symbols
    /// panel act on the active artboard, the view's, which the app adds to command `id`'s `params` as
    /// `artboard`. A batch passes it on to those steps. While the Artboard tool has several artboards
    /// selected, the paste commands get those as `artboards` instead, and paste a copy on each; in a
    /// document without artboards they get neither.
    fn add_active_artboard(&self, id: &str, params: &mut Value) {
        let paste = matches!(id, "edit.pasteInPlace" | "edit.pasteInFront" | "edit.pasteInBack");
        if (vectorcraft_engine::ON_ACTIVE_ARTBOARD.contains(&id) || id == "command.batch")
            && params.get("artboard").is_none()
            && !(paste && params.get("artboards").is_some())
            && self.view().is_some()
            && let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0 || !paste)
            && let Some(p) = params.as_object_mut()
        {
            let picked = panels::artboards::picked(self, n);
            if paste && picked.len() > 1 {
                p.insert("artboards".into(), serde_json::json!(picked));
            } else {
                p.insert("artboard".into(), serde_json::json!(panels::artboards::selected(self, n)));
            }
        }
    }

    /// [`Self::run`] it, inside what asks for file dialogs.
    fn run_now(&mut self, id: &str, params: Value) -> Result<Value, String> {
        crate::state::sync_zoom_stops_display_print_size(self.session.prefs.display_print_size);
        self.run_count = self.run_count.wrapping_add(1);
        // In debug builds the app's own calls are held to the params descriptions too (#1442): a
        // panel passing a key its command doesn't read shows up in the tests. Saved steps aren't
        // checked ([`Self::replay`]).
        if cfg!(debug_assertions) && !self.session.replaying() {
            menus::check_params(&self.session, id, &params)?;
        }
        self.wait_for_tool_edit(id);
        // A `.ai` file a command writes (a save, an export, a timestamped copy, a batch's save) names
        // the view's active artboard.
        io::remember_active_artboard(self);
        let artboards = self.artboard_tool_state();
        if let Some(r) = menus::run_ui_command(self, id, &params) {
            self.ui.recording_base = None;
            self.follow_artboard_tool(artboards);
            return r;
        }
        let mut params = params;
        self.add_active_artboard(id, &mut params);
        // Edit Symbol from the Symbols panel in a document without artboards centers the symbol's
        // art in the view, whose center the app passes as `viewCenter`, to a batch's symbol.edit
        // steps too.
        if matches!(id, "symbol.edit" | "command.batch")
            && let Some(c) = self.view().map(|v| v.center)
        {
            let fill = |p: &mut serde_json::Value| {
                if let Some(p) = p.as_object_mut() {
                    p.entry("viewCenter").or_insert(serde_json::json!([c.x, c.y]));
                }
            };
            if id == "symbol.edit" {
                fill(&mut params);
            } else if let Some(serde_json::Value::Array(steps)) = params.get_mut("commands") {
                for step in steps {
                    if step.get("command").and_then(serde_json::Value::as_str) == Some("symbol.edit")
                        && let Some(step) = step.as_object_mut()
                    {
                        fill(step.entry("params").or_insert_with(|| serde_json::json!({})));
                    }
                }
            }
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
            // Paste (also without formatting) goes to the centre of the view.
            if matches!(id, "edit.paste" | "edit.pasteWithoutFormatting")
                && ["center", "dx", "dy"].iter().all(|k| params.get(k).is_none())
                && let Some(c) = self.view().filter(|v| v.fitted).map(|v| v.center)
                && let Some(p) = params.as_object_mut()
            {
                p.insert("center".into(), serde_json::json!([c.x, c.y]));
            }
            // A Paste the user gives decodes a bitmap off the UI thread ([`placing::adopt_for_paste`]),
            // and runs with these params once it is decoded.
            match placing::adopt_for_paste(self, id, &params) {
                Err(e) => {
                    self.ui.status = e.clone();
                    return Err(e);
                }
                Ok(Some(started)) => return Ok(started),
                Ok(None) => {}
            }
            if let Some(r) = dialogs::swatch_conflict::ask(self, id, &params) {
                return r;
            }
        }
        // Native files carry the view they reopen at.
        if fileio::SaveMode::of(id).is_some() {
            io::remember_view(self);
        }
        // While an action records, a command that the recording would keep as a step runs only
        // when the step fits in the action.
        let recorded = match panels::actions::before_recorded_command(self, id, &params) {
            Ok(recorded) => recorded,
            Err(e) => {
                self.ui.status = e.clone();
                return Err(e);
            }
        };
        // A command that changes the edited text is an application edit (undo, a panel or menu
        // edit, a control request). Capture the bounded committed context around it so `Other`
        // dominates the frame even if an IME receipt follows. Selection/view commands that leave
        // the committed text and caret unchanged record nothing.
        let ime_view = self.view_info();
        let ime_target = crate::ime::target(&mut self.session, ime_view);
        let ime_before = crate::ime::stamp(&mut self.session, ime_view);
        let cursor_settings = (self.session.caps_lock, self.session.prefs.use_precise_cursors);
        let r = self.session.execute(id, &params).map_err(|e| e.to_string());
        let r = panels::actions::after_recorded_command(self, recorded, r);
        if r.is_ok() && cursor_settings != (self.session.caps_lock, self.session.prefs.use_precise_cursors) {
            self.canvas.hover_cursor = None;
        }
        if r.is_ok()
            && let Some(ctx) = self.ui_ctx.clone()
        {
            crate::ime::note_application_edit(self, &ctx, ime_view, ime_target, ime_before);
        }
        if r.is_ok()
            && let Some(op) = completed_pathfinder_operation(id, &params)
        {
            self.last_pathfinder = Some(op.to_string());
        }
        let copied = r.is_ok() && self.publish_copy(id);
        self.sync_views();
        self.refresh_replace_font_warning(false);
        // File › Revert replaces the document in its tab, keeps the tab's view and starts the tool afresh.
        self.follow_artboard_tool(artboards);
        match &r {
            Err(e) => self.ui.status = e.clone(),
            Ok(v) => {
                if matches!(id, "imageTrace.make" | "imageTrace.makeAndExpand") {
                    self.show_trace_notes(v);
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
                // A font applied with Character's font menu or Find Font's Change becomes the newest recent font.
                // Find Font's Change records the family the text now carries (any name of it is stored as its
                // own name), and only when it changed some text.
                let applied = match id {
                    "text.setStyle" | "text.setRangeStyle" => params.get("font").and_then(Value::as_str).map(str::to_string),
                    "text.replaceFont" if v["runs"].as_u64().is_some_and(|n| n > 0) => {
                        let to = &params["to"];
                        to["family"].as_str().and_then(|family| {
                            let style = to["style"].as_str().unwrap_or("Regular");
                            vectorcraft_text::FontDb::global().stored_family(family, style)
                        })
                    }
                    _ => None,
                };
                if let Some(font) = applied.as_deref() {
                    let r = &mut self.ui.recent_fonts;
                    r.retain(|f| !f.eq_ignore_ascii_case(font));
                    r.insert(0, font.to_string());
                    r.truncate(MAX_RECENT_FONTS);
                }
                if id == "text.replaceFont" {
                    self.update_replace_font_status(&params, v);
                }
                // A command's warning (Rasterize's when an image couldn't be decoded) shows in the
                // status bar, as a tool preview's does.
                if let Some(w) = v.get("warning").and_then(Value::as_str) {
                    self.set_command_warning(id, w);
                }
            }
        }
        // The message the Copy or Cut leaves is the one its clipboard formats' notes may replace.
        if copied {
            self.clipboard_publishing.ran_with_status(&self.ui.status);
        }
        r
    }

    /// After a Copy or Cut `id` that ran: offer what it copied to other apps
    /// ([`Self::publish_clipboard`]) → whether `id` is one.
    fn publish_copy(&mut self, id: &str) -> bool {
        let copy = matches!(id, "edit.copy" | "edit.cut" | "artboard.copy" | "artboard.cut");
        if copy {
            self.publish_clipboard(matches!(id, "edit.cut" | "artboard.cut"));
        }
        copy
    }

    /// Image Trace's notes in `reply` (a large image traces more slowly, or was scaled down): the
    /// first of them and how many there are, with the rest for the message's hover. Without notes
    /// the status bar keeps its message.
    fn show_trace_notes(&mut self, reply: &Value) {
        let notes = io::notes_of(reply);
        if let Some(first) = notes.first() {
            let status = crate::trace_runs::traced_with_notes(notes.len(), first);
            io::show_with_notes(self, status, notes);
        }
    }

    fn set_command_warning(&mut self, id: &str, warning: &str) {
        self.ui.status = warning.to_string();
        if id != "text.replaceFont" {
            self.ui.replace_font_warning = None;
        }
    }

    fn update_replace_font_status(&mut self, params: &Value, result: &Value) {
        if let Some(warning) = result.get("warning").and_then(Value::as_str) {
            let Some((document_uid, revision)) = self.session.active().map(|doc| (doc.uid, doc.revision)) else {
                return;
            };
            let styles = self
                .session
                .execute("text.fonts", &serde_json::json!({}))
                .ok()
                .and_then(|value| value.as_array().cloned())
                .map(|fonts| {
                    fonts
                        .iter()
                        .filter_map(|font| {
                            let description = substitute_style_description(font)?;
                            if !warning.contains(&description) {
                                return None;
                            }
                            Some((font["family"].as_str()?.to_string(), font["style"].as_str()?.to_string()))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let selection: Option<Vec<u64>> = (params["selectionOnly"] == true)
                .then(|| self.session.active().map(|doc| doc.selection.objects.iter().map(|id| id.0).collect()).unwrap_or_default());
            self.ui.replace_font_warning =
                Some(ReplaceFontWarning { document_uid, revision, request: params.clone(), selection, text: warning.to_string(), styles });
            return;
        }
        self.refresh_replace_font_warning(true);
    }

    pub(super) fn refresh_replace_font_warning(&mut self, force: bool) {
        const SKIPPED: &str = "some text is hidden or locked, so its font wasn't replaced";
        const STYLE_PREFIX: &str = "font styles that aren't available are drawn in the closest style: ";
        const STYLE_SUFFIX: &str = " (pick a style the family has)";
        let Some(previous) = self.ui.replace_font_warning.as_ref() else {
            return;
        };
        let Some(revision) = self.session.active().filter(|doc| doc.uid == previous.document_uid).map(|doc| doc.revision) else {
            return;
        };
        if (!force && revision == previous.revision) || previous.text != self.ui.status {
            return;
        }
        let had_skipped = self.ui.status.starts_with(SKIPPED);
        let had_style = !previous.styles.is_empty();
        if !had_skipped && !had_style {
            return;
        }
        let scoped_selection = previous.selection.as_ref().map(|selected| selected.iter().copied().map(vectorcraft_doc::NodeId).collect::<Vec<_>>());
        let skipped_remains = if had_skipped {
            self.session.skipped_font_ids(&previous.request, scoped_selection.as_deref()).is_ok_and(|ids| !ids.is_empty())
        } else {
            false
        };
        let (missing, live_styles) = if had_style {
            let Some(fonts) = self.session.font_inventory(scoped_selection.as_deref()).ok().and_then(|value| value.as_array().cloned()) else {
                return;
            };
            substitute_warnings(&fonts, &previous.styles)
        } else {
            (Vec::new(), Vec::new())
        };
        let mut parts = Vec::new();
        if skipped_remains {
            parts.push(SKIPPED.to_string());
        }
        if !missing.is_empty() {
            parts.push(format!(
                "fonts that aren't available are drawn in the fallback font, {}: {} (install them, or replace them with Find Font)",
                vectorcraft_text::FALLBACK_FAMILY,
                missing.join(", ")
            ));
        }
        if !live_styles.is_empty() {
            parts.push(format!("{STYLE_PREFIX}{}{STYLE_SUFFIX}", live_styles.join(", ")));
        }
        self.ui.status = parts.join("; ");
        if self.ui.status.is_empty() {
            self.ui.replace_font_warning = None;
        } else if let Some(warning) = &mut self.ui.replace_font_warning {
            warning.revision = revision;
            warning.text.clone_from(&self.ui.status);
        }
    }

    /// Select a tool (also used by the toolbar and shortcuts).
    pub fn select_tool(&mut self, id: &str) {
        // The tip of a pen whose eraser end selected the Eraser doesn't undo this choice, and
        // neither does letting go of a held tool shortcut.
        self.pen_end.forget_tool();
        self.held_tool.clear();
        self.activate_tool(id, true);
        let now = self.session.tool_id();
        if crate::touch::is_drawing_tool(now) {
            self.ui.touch_drawing_tool = Some(now.to_string());
        }
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
        if before != self.session.tool_id() {
            // A control gesture cannot resume after another tool has owned the pointer.
            self.control_rotate_press = None;
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
