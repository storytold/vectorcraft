use super::{
    Arc, Document, EngineError, Interaction, LastTransform, NamedStep, Result, Selection, Session, TOO_DEEP, Value, cmd, deselect_hidden, doc_sane,
    objects_in_range,
};

impl Session {
    /// Run `f` on a mutable copy of the active document. Outside an interaction this records one
    /// undo step labelled `label`; inside one it just mutates (the interaction commits later).
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>) -> Result<T> {
        if self.script_running() {
            return Err(EngineError::Busy);
        }
        // An edit made while typing (the Transform panel) is a step of its own: inside the typing
        // session, the next keystroke's preview would start again from before it (#1881).
        if !self.previewing && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        let before = st.doc.clone();
        let before_sel = st.selection.clone();
        let doc = Arc::make_mut(&mut st.doc);
        // The Global Edit members the edit reached besides the selection.
        let mut spread = vec![];
        let mut dim_changed = false;
        // Objects attached to a path follow it, and dimensions follow the art they measure.
        let result = match f(doc, &mut st.selection).and_then(|v| {
            // Global Edit: the other members take what the edit changed about the one it edited.
            if let Some(ge) = &st.global_edit {
                spread = cmd::globaledit::propagate(&before, doc, ge, [&before_sel.objects, &st.selection.objects]);
            }
            // Pixel-aligned objects the edit changed go back on the pixel grid.
            cmd::refresh_pixel_aligned(&before, doc);
            // Color masks of images whose own pixels the edit replaced are made for the new ones.
            cmd::refresh_color_masks(Some(&*before), doc);
            let placed = cmd::objects_on_path::refresh(&before, doc);
            // Pixel-aligned members that placing moved go back on the pixel grid.
            cmd::keep_aligned_in(doc, &placed);
            dim_changed = cmd::dimension::refresh(&before, doc)?;
            Ok(v)
        }) {
            Ok(v) => match cmd::shaper::refresh(&before, Arc::make_mut(&mut st.doc)) {
                Err(e) => Err(e),
                Ok(shaper_changed) => {
                    // Attached Shaper groups whose art was rebuilt go back on their path.
                    if shaper_changed || dim_changed {
                        let doc = Arc::make_mut(&mut st.doc);
                        let placed = cmd::objects_on_path::refresh(&before, doc);
                        cmd::keep_aligned_in(doc, &placed);
                    }
                    // Symbol editing: the symbol follows its art on the editing layer, before
                    // inline graphics follow their symbols.
                    let mut symbol_synced =
                        if st.doc.symbol_edit.is_some() { cmd::symboledit::sync(&before, Arc::make_mut(&mut st.doc)) } else { Ok(()) };
                    // Inline graphics in text follow their symbols.
                    let measured = cmd::inline::refresh(st.inline_resolved.as_ref(), &before, Arc::make_mut(&mut st.doc));
                    let mut resolved = measured.map(|m| (st.doc.clone(), m));
                    // The document the passes below start from, for the symbol to follow what they
                    // change on its editing layer.
                    let synced = st.doc.symbol_edit.is_some().then(|| st.doc.clone());
                    // Text Wrap: area type follows its wrap objects; then threads re-flow.
                    cmd::textwrap::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Type in no thread numbers its list paragraphs from 1 (before the mask copies
                    // its editing layer's art).
                    cmd::threads::restart_unthreaded_counts(&mut st.doc);
                    // Opacity-mask editing: the mask follows its art on the editing layer.
                    if st.doc.mask_edit.is_some() {
                        cmd::maskedit::sync(Arc::make_mut(&mut st.doc));
                    }
                    // Threaded text re-flows when any of its frames changed.
                    if !st.doc.text_threads.is_empty() {
                        cmd::threads::reflow(&before, Arc::make_mut(&mut st.doc));
                    }
                    // Symbol editing: the symbol also takes the text wrap, list numbering and
                    // re-flow those passes gave its editing layer; when that changes the symbol, its
                    // inline graphics follow it and the threads they are in re-flow.
                    let editing = st.doc.symbol_edit.as_ref().map(|e| e.layer);
                    if let Some(synced) = &synced
                        && symbol_synced.is_ok()
                        && editing.is_some()
                        && before.symbol_edit.as_ref().map(|e| e.layer) == editing
                    {
                        let art =
                            |d: &Document| d.symbol_edit.as_ref().and_then(|e| d.symbols.iter().find(|x| x.name == e.symbol)).map(|x| x.art.clone());
                        let old = art(&st.doc);
                        symbol_synced = cmd::symboledit::sync(synced, Arc::make_mut(&mut st.doc));
                        let new = art(&st.doc);
                        if symbol_synced.is_ok() && !matches!((&old, &new), (Some(a), Some(b)) if Arc::ptr_eq(a, b)) {
                            let resynced = st.doc.clone();
                            let last = resolved.as_ref().or(st.inline_resolved.as_ref());
                            if let Some(m) = cmd::inline::refresh(last, synced, Arc::make_mut(&mut st.doc)) {
                                resolved = Some((st.doc.clone(), m));
                            }
                            if !st.doc.text_threads.is_empty() {
                                cmd::threads::reflow(&resynced, Arc::make_mut(&mut st.doc));
                            }
                        }
                    }
                    // Asset Export: assets let go of deleted art.
                    if !st.doc.assets.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_assets();
                    }
                    // Variables: bindings let go of deleted art.
                    if !st.doc.variables.bindings.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_variable_bindings();
                    }
                    st.inline_resolved = resolved;
                    if let Err(e) = symbol_synced {
                        Err(e)
                    } else if st.doc.too_deep(Some(&before)) {
                        // Deeper trees overflow the stack of what walks them.
                        Err(EngineError::Other(TOO_DEEP.into()))
                    } else if doc_sane(&st.doc, &st.selection) && objects_in_range(&st.doc, &spread) {
                        Ok(v)
                    } else {
                        Err(EngineError::Other("result would exceed the canvas (coordinates out of range)".into()))
                    }
                }
            },
            Err(e) => Err(e),
        };
        let r = match result {
            Ok(v) => {
                st.selection.prune(&st.doc);
                // An edit that selects what it unlocks, shows or pastes leaves locked guides out,
                // and one that selects the slices it makes leaves them out while they are locked
                // or hidden.
                deselect_hidden(&menu, &st.doc, &mut st.selection);
                // The art on a locked artboard can't stay selected, however it got there, except
                // the new art this edit made and new art still selected.
                st.protect_locked_artboards(&before_sel.objects, &before_sel.new_art, Some(&*before));
                // Symbol or opacity mask editing ended because the edit deleted its layer:
                // isolation on the layer or inside it leaves too, out of symbol editing to what it
                // was before.
                let mode_layer = before.symbol_edit.as_ref().map(|e| e.layer).or(before.mask_edit.map(|m| m.layer));
                if let Some(layer) = mode_layer
                    && st.doc.node(layer).is_none()
                    && st.isolation.is_some_and(|i| i == layer || st.doc.node(i).is_none())
                {
                    if let Some(se) = &before.symbol_edit {
                        cmd::symboledit::go_back(st, se);
                    } else {
                        st.isolation = None;
                        st.active_layer = st.doc.default_layer();
                    }
                }
                st.revision += 1;
                if st.interaction.is_none() {
                    let selection = match st.history.undo.last() {
                        Some(l) if *l.selection == before_sel => l.selection.clone(),
                        _ => Arc::new(before_sel),
                    };
                    st.push_undo(label.to_string(), before, selection);
                }
                Ok(v)
            }
            Err(e) => {
                st.doc = before;
                st.selection = before_sel;
                cmd::globaledit::follow_history(st);
                Err(e)
            }
        };
        // An edit a frontend makes itself, outside a command, keeps to the shared budget too.
        if r.is_ok() && self.depth == 0 && !self.in_tool_actions {
            self.trim_histories();
        }
        r
    }

    /// [`Session::edit`] with the step named `template`, its `{name}` placeholder filled with `name`
    /// (a style's name). The undo step it records keeps the template and the name
    /// ([`HistoryEntry::named`]), and a frontend shows the template's translation with the name in
    /// it. An edit that records no step of its own (inside an interaction, or a later edit of an
    /// undo group) keeps neither, and neither does a step an undo group names.
    pub fn edit_named<T>(
        &mut self,
        template: &'static str,
        name: &str,
        built_in: bool,
        f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>,
    ) -> Result<T> {
        let named = NamedStep { template, name: name.to_string(), built_in };
        let step = named.label();
        // The step this edit records keeps the document as it is now.
        let now = Arc::as_ptr(&self.doc()?.doc);
        let v = self.edit(&step, f)?;
        if let Ok(st) = self.doc_mut()
            && let Some(e) = st.history.undo.last_mut()
            && Arc::as_ptr(&e.doc) == now
            && e.label == step
        {
            e.named = Some(named);
        }
        Ok(v)
    }

    /// Change only the selection (not an undo step).
    pub fn select(&mut self, f: impl FnOnce(&Document, &mut Selection)) -> Result<()> {
        self.active_appearance_item = None;
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        let before = st.selection.objects.clone();
        let new_art = st.selection.new_art.clone();
        f(&st.doc, &mut st.selection);
        st.selection.prune(&st.doc);
        // Lock Guides locks the paths made into guides too (#1534), and Lock Slices and Hide
        // Slices keep the slices out, whatever selects them.
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        // The art on a locked artboard can't be selected, whatever selects it, except new art
        // that was selected and still is.
        st.protect_locked_artboards(&before, &new_art, None);
        // Selecting art makes its layer (or sublayer) the current one, as in the Layers panel of
        // the reference app: new art then goes beside it.
        if st.selection.objects != before
            && let Some(layer) = st.selection.objects.last().and_then(|id| st.doc.layer_containing(*id)).filter(|l| st.doc.is_editable(*l))
            && st.active_layer != Some(layer)
        {
            st.active_layer = Some(layer);
            st.layer_rows.clear();
        }
        st.revision += 1;
        // Puppet Warp pins belong to the art they were placed on: another selection starts afresh.
        if st.doc.puppet.as_ref().is_some_and(|p| p.ids != st.selection.objects) {
            cmd::distortcmds::drop_puppet_pins(st);
        }
        Ok(())
    }

    // ---------- interactions (live drags) ----------

    pub(crate) fn clear_deferred_overlay_cache(&mut self) {
        self.deferred_overlay_cache = None;
    }

    /// The interaction's latest preview command when the document is still its snapshot (deferred
    /// Real-time Drawing), regardless of the preference at commit or overlay time.
    pub(crate) fn unapplied_interaction_preview(&self) -> Option<(String, Value)> {
        self.active().and_then(|d| d.interaction.as_ref().filter(|it| std::sync::Arc::ptr_eq(&it.doc, &d.doc)).and_then(|it| it.preview.clone()))
    }

    /// Keep `warning`, from a preview that settling an interaction applied, for whatever collects
    /// such warnings now ([`Session::collect_settled_warnings`]). Nothing keeps it when nothing
    /// collects, so it is never reported with a later, unrelated command.
    pub(crate) fn note_settled_warning(&mut self, warning: &str) {
        if let Some(kept) = &mut self.settled_warnings
            && !kept.iter().any(|w| w == warning)
        {
            kept.push(warning.to_string());
        }
    }

    /// Run `f`, and → its result with the warnings of the previews that settling an interaction
    /// applied while it ran ([`Session::note_settled_warning`]). Warnings an outer `f` collects are
    /// kept for it.
    pub(crate) fn collect_settled_warnings<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> (R, Vec<String>) {
        let outer = self.settled_warnings.replace(Vec::new());
        let r = f(self);
        let settled = std::mem::replace(&mut self.settled_warnings, outer).unwrap_or_default();
        (r, settled)
    }

    /// Run `f`, the top-level command `id` with `params` from outside the tool (a panel, a menu
    /// item, an automation client), with the active document's tool drag set aside: the command
    /// runs on the document the drag started from and records its own undo step and journal entry.
    /// The drag then starts from what the command left, and its preview is applied again, so the
    /// next preview, the commit and a cancel keep the command's edit, and Undo takes back the drag's
    /// step before the command's. While the command runs, [`Session::in_interaction`] is false, so
    /// it updates Recent Colors and converts or remaps the new-art paints at once, as it does with
    /// no drag in progress.
    ///
    /// The interaction stays as it is, and the command runs inside it, for an interaction a dialog
    /// or a panel began (it builds its preview from commands), for typing in progress
    /// ([`Session::edit`] keeps it as a step of its own), for the Quick Pen (an edit made outside
    /// it ends the path it draws), and for a copy made with `object.transform` during a drag that
    /// copies (it copies the document as the drag's move left it). A command that settles,
    /// commits or cancels the interaction, or that hands the active tool actions to apply, gets it
    /// back first ([`Session::put_back_interaction`]), as it found it.
    pub(crate) fn outside_interaction(&mut self, id: &str, params: &Value, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        if cmd::typing_in_progress(self) || self.tool_id() == vectorcraft_tools::quickpen::ID {
            return f(self);
        }
        let Some(st) = self.active_mut() else { return f(self) };
        let Some(it) = st.interaction.as_ref().filter(|it| it.tool) else { return f(self) };
        // A drag that copies: its latest preview, applied or held back by Real-time Drawing off, copies.
        let copying = it.preview.as_ref().is_some_and(|(c, p)| c == "object.transform" && p.get("copy").and_then(Value::as_bool) == Some(true));
        if copying && id == "object.transform" && params.get("copy").and_then(Value::as_bool) == Some(true) {
            return f(self);
        }
        let Some(interaction) = st.interaction.take() else { return f(self) };
        // The document and selection on show, when a preview changed them.
        let previewed = !Arc::ptr_eq(&st.doc, &interaction.doc) || st.selection != *interaction.selection;
        let shown = previewed.then(|| (st.doc.clone(), st.selection.clone()));
        if previewed {
            st.doc = interaction.doc.clone();
            st.selection.clone_from(&interaction.selection);
            st.revision += 1;
            cmd::globaledit::follow_history(st);
        }
        let (uid, state) = (st.uid, crate::interaction::ViewState::of(st));
        let paint = self.paint.clone();
        self.held_interaction = Some(crate::HeldInteraction { uid, interaction, shown, state, paint });
        if previewed {
            self.sync_front_profiles();
        }
        let r = f(self);
        self.put_back_interaction();
        r
    }

    /// Put back the interaction [`Session::outside_interaction`] set aside. When the command left
    /// the document and the selection as it found them, the preview on show before it is on show
    /// again. Otherwise the interaction starts from the document the command left and from its
    /// own selection, without the objects the command removed, hid or locked, and the preview that Real-time
    /// Drawing showed is applied to it again. The warning or the error of that preview goes to
    /// the command's result ([`Session::note_settled_warning`]); a preview that fails leaves the
    /// interaction with nothing to commit. An interaction whose document the command closed ends,
    /// and a document the command left an interaction of its own on keeps that one.
    pub(crate) fn put_back_interaction(&mut self) {
        let Some(crate::HeldInteraction { uid, interaction: mut it, shown, state, paint }) = self.held_interaction.take() else { return };
        // A selection command loaded the paints of the selection it made into the fill and stroke
        // proxies. The drag goes on with its own selection, so they show its paints again.
        if self.paint != paint
            && let Some(st) = self.active_mut().filter(|d| d.uid == uid && d.interaction.is_none())
        {
            let own = drag_selection(&it.selection, &st.doc);
            if st.selection != own {
                let made = std::mem::replace(&mut st.selection, own);
                self.inherit_new_art();
                self.load_selection_paints();
                if let Some(st) = self.active_mut() {
                    st.selection = made;
                }
            }
        }
        // A change the command made to the new-art paints stays when the next preview or a cancel
        // puts back the paints the interaction began with (the Live Paint Bucket's colors); the
        // parts it left alone keep the interaction's.
        let (now, start) = (&self.paint, &mut it.paint);
        if now.fill != paint.fill {
            start.fill.clone_from(&now.fill);
        }
        if now.stroke != paint.stroke {
            start.stroke.clone_from(&now.stroke);
        }
        if now.stroke_width != paint.stroke_width {
            start.stroke_width = now.stroke_width;
        }
        if now.appearance != paint.appearance {
            start.appearance.clone_from(&now.appearance);
        }
        if now.opacity != paint.opacity {
            start.opacity = now.opacity;
        }
        if now.blend != paint.blend {
            start.blend = now.blend;
        }
        if now.style != paint.style {
            start.style.clone_from(&now.style);
        }
        if now.inherited != paint.inherited {
            start.inherited = now.inherited;
        }
        let active = self.active().is_some_and(|d| d.uid == uid);
        let Some(st) = self.docs.iter_mut().find(|d| d.uid == uid) else { return };
        if st.interaction.is_some() {
            return;
        }
        // A change the command made to the current layer, the highlighted rows or variable, or
        // isolation stays when the interaction is canceled.
        state.rebase(crate::interaction::ViewState::of(st), &mut it);
        let previewed = shown.is_some();
        if Arc::ptr_eq(&st.doc, &it.doc) && st.selection == *it.selection {
            if let Some((doc, selection)) = shown {
                st.doc = doc;
                st.selection = selection;
                st.revision += 1;
                cmd::globaledit::follow_history(st);
            }
            st.interaction = Some(it);
            if previewed {
                self.sync_front_profiles();
            }
            return;
        }
        let began = std::mem::replace(&mut it.doc, st.doc.clone());
        // The drag acts on the objects it began with: a selection the command made (a paste, a
        // duplicate, Select All) does not change what the drag moves.
        it.selection = Arc::new(drag_selection(&it.selection, &st.doc));
        // The journal holds the selection the command left. The drag's own selection is journaled
        // after it, so the drag and the commands after it replay on the objects they act on.
        let reselect = (active && st.selection != *it.selection)
            .then(|| selection_steps(&it.selection, &st.selection, &st.doc, st.active_layer, &st.highlighted_rows()));
        // The objects the drag's preview names leave out those the command removed, hid or locked.
        // A preview left naming none has nothing to apply, and the drag records no step.
        if it.preview.as_mut().is_some_and(|(_, p)| !drag_targets(p, &began, &st.doc)) {
            it.preview = None;
        }
        it.copies = None;
        it.move_box = std::sync::OnceLock::new();
        // With Real-time Drawing off the preview waits for the commit, as it did before the command.
        let again = it.preview.clone().filter(|_| previewed && active);
        // Without a preview to apply, the drag's own selection is on show, as a preview would leave
        // it before it acts.
        if again.is_none() && st.selection != *it.selection {
            st.selection.clone_from(&it.selection);
            st.revision += 1;
        }
        st.interaction = Some(it);
        if let Some(steps) = reselect {
            let (from, grouped) = (self.journal.len(), self.undo_group_docs());
            for step in steps {
                self.journal.push(step);
            }
            self.journal_in_undo_groups(Some(uid), &grouped, from);
        }
        self.clear_deferred_overlay_cache();
        if let Some((cmd, p)) = again {
            match self.preview(&cmd, &p) {
                Ok(v) => {
                    if let Some(w) = v.get("warning").and_then(Value::as_str) {
                        self.note_settled_warning(w);
                    }
                }
                Err(e) => self.note_settled_warning(&e.to_string()),
            }
        }
    }

    /// While a command from outside the tool runs: the interaction it set aside
    /// ([`Session::outside_interaction`]).
    pub(crate) fn holds_interaction(&self) -> bool {
        self.held_interaction.is_some()
    }

    /// [`Session::begin_interaction`] for a dialog or a panel that starts a preview → the warnings
    /// of the preview that keeping typing in progress applied, for its status bar.
    pub fn begin_interaction_warned(&mut self, label: &str) -> Result<Vec<String>> {
        let (r, settled) = self.collect_settled_warnings(|s| s.begin_interaction(label));
        r.map(|()| settled)
    }

    /// Keep typing in progress as one undo step, applying the preview Real-time Drawing off left
    /// unapplied, and end the Type tool's typing session → the applied preview's warnings. Nothing
    /// changes when no typing is in progress.
    pub fn keep_typing(&mut self) -> Result<Vec<String>> {
        let (r, settled) = self.collect_settled_warnings(|s| if cmd::typing_in_progress(s) { cmd::settle_interaction(s) } else { Ok(()) });
        r.map(|()| settled)
    }

    pub fn begin_interaction(&mut self, label: &str) -> Result<()> {
        if self.script_running() {
            return Err(EngineError::Busy);
        }
        // Typing in progress is kept first: a drag or a dialog's preview would otherwise start
        // from the snapshot taken before the typing, and drop it (#1881).
        if label != "Typing" && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
        // A command from outside the tool that begins an interaction during a drag joins the drag's.
        self.put_back_interaction();
        // An edit the tool computes on another thread (a Shape Builder merge) is applied first too,
        // so the snapshot the interaction previews from and goes back to holds it.
        if !self.in_interaction() {
            self.finish_tool_edit_later()?;
        }
        self.clear_deferred_overlay_cache();
        let paint = self.paint.clone();
        let st = self.doc_mut()?;
        if st.interaction.is_some() {
            return Ok(());
        }
        st.interaction = Some(Interaction {
            label: label.to_string(),
            doc: st.doc.clone(),
            selection: Arc::new(st.selection.clone()),
            preview: None,
            tool: false,
            paint,
            preview_paint: None,
            color_mode_paints: None,
            deferred_color_mode: None,
            active_layer: st.active_layer,
            layer_rows: st.layer_rows.clone(),
            variables_highlight: st.variables_highlight.clone(),
            isolation: st.isolation,
            perspective_again: None,
            plane_placed: false,
            copies: None,
            move_box: std::sync::OnceLock::new(),
        });
        Ok(())
    }

    /// Re-apply `cmd` on top of the interaction snapshot (replacing the previous preview).
    pub fn preview(&mut self, cmd: &str, params: &Value) -> Result<Value> {
        self.put_back_interaction();
        if self.script_running() {
            return Err(EngineError::Busy);
        }
        // While the Live Paint Bucket is active, paint previews change only the bucket's colors, so
        // each preview starts from the colors the interaction began with.
        let bucket = self.bucket_paints();
        let (paint, restore_paint) = {
            let menu = self.menu.clone();
            let st = self.doc_mut()?;
            let Some(it) = &mut st.interaction else { return Err(EngineError::Other("no interaction in progress".into())) };
            // Undoing the previous preview counts as a change, so views redraw even when the command
            // below returns without an edit.
            let selection_changed = st.selection != *it.selection;
            if !Arc::ptr_eq(&st.doc, &it.doc) || selection_changed {
                st.revision += 1;
            }
            st.doc = it.doc.clone();
            // A batch, or a preview that changed the new-art paints (a pattern rename), started from
            // the paints the interaction began with.
            let restore_paint = bucket
                || cmd == "command.batch"
                || it.preview.as_ref().is_some_and(|(id, _)| id == "command.batch")
                || it.color_mode_paints.is_some();
            let paint = it.paint.clone();
            it.preview_paint = None;
            it.color_mode_paints = None;
            it.deferred_color_mode = None;
            it.plane_placed = false;
            if selection_changed {
                st.selection.clone_from(&it.selection);
            }
            // Guides locked and slices locked or hidden during the interaction stay deselected,
            // even when the preview fails.
            deselect_hidden(&menu, &st.doc, &mut st.selection);
            cmd::globaledit::follow_history(st);
            (paint, restore_paint)
        };
        if restore_paint {
            self.paint = paint.clone();
        }
        self.pending_paint = None;
        // The previous preview's Assign Profile is undone with it.
        self.sync_front_profiles();
        let rw = cmd::distortcmds::perspective_rewrite(self, cmd, params).or_else(|| cmd::bounds_rewrite(self, cmd, params));
        let (cmd, params) = rw.as_ref().map_or((cmd, params), |(c, p)| (c.as_str(), p));
        // A selected object's converted appearance must not replace the current paints while
        // this tentative color-mode preview is still cancellable.
        let color_mode_paint = matches!(cmd, "file.documentColorMode" | "object.convertDocumentColorMode").then(|| self.paint.clone());
        let outer = std::mem::replace(&mut self.previewing, true);
        let r = self.execute(cmd, params);
        self.previewing = outer;
        if let Some(paint) = color_mode_paint {
            self.paint = paint;
        }
        let params = self.noted(params);
        let batch_paint = (r.is_ok() && cmd == "command.batch").then(|| self.paint.clone());
        let st = self.doc_mut()?;
        if let Some(it) = &mut st.interaction {
            // A failed preview leaves the document at the snapshot, so nothing is left to commit
            // (not even an earlier successful preview, which the reset above already undid).
            it.preview = r.is_ok().then(|| (cmd.to_string(), params));
            if it.color_mode_paints.is_some() {
                it.preview_paint = batch_paint.clone();
            }
        }
        if batch_paint.is_some() && self.active().and_then(|st| st.interaction.as_ref()).is_some_and(|it| it.preview_paint.is_some()) {
            self.paint = paint;
        }
        r
    }

    pub fn commit_interaction(&mut self) -> Result<()> {
        self.clear_deferred_overlay_cache();
        self.decoded_images.empty();
        self.put_back_interaction();
        if let Some(paint) = self.active().and_then(|st| st.interaction.as_ref()).and_then(|it| it.preview_paint.clone()) {
            self.paint = paint;
        }
        self.remember_pending_paint();
        cmd::colormgmt::commit_new_art_colors(self)?;
        let paint = self.paint.clone();
        let st = self.doc_mut()?;
        let Some(mut it) = st.interaction.take() else { return Ok(()) };
        let Some(preview) = it.preview.take() else { return Ok(()) };
        if let Some((_, after)) = &mut it.color_mode_paints {
            after.clone_from(&paint);
        }
        let perspective_again = it.perspective_again.take();
        let plane_placed = it.plane_placed;
        let copy = preview.0 == "object.transform" && preview.1.get("copy").and_then(Value::as_bool).unwrap_or(false);
        // Alt pressed or released during a move drag turns it into a copy or back: name it for
        // what it did in the end.
        match (it.label.as_str(), copy) {
            ("Move", true) => it.label = "Copy".into(),
            ("Copy", false) if preview.0 == "object.transform" => it.label = "Move".into(),
            _ => {}
        }
        // A preview that left the document as it was records no Transform Again either (as
        // `keep_interaction` records no undo step): the last real transformation stands.
        let changed = !Arc::ptr_eq(&it.doc, &st.doc);
        // How far a drag of direct-selected anchors moved them (`path.moveAnchors`).
        let moved = vectorcraft_geom::Vec2::new(
            preview.1.get("dx").and_then(Value::as_f64).unwrap_or(0.0),
            preview.1.get("dy").and_then(Value::as_f64).unwrap_or(0.0),
        );
        st.keep_interaction(it);
        // The drag drew an object on, or attached one to, the plane Automatic Plane Positioning
        // held: the plane is back in its place now, as view state (undoing the drag holds it again).
        if plane_placed && changed && vectorcraft_tools::distort::perspective::PerspectiveGrid::positioning(&st.doc) {
            vectorcraft_tools::distort::perspective::PerspectiveGrid::end_positioning(Arc::make_mut(&mut st.doc));
            st.revision += 1;
        }
        if preview.0 == "object.transform" && changed {
            let m = cmd::matrix_param(&preview.1, "matrix");
            if let Some(m) = m {
                // The options are the last preview's params with the values it noted, without the
                // matrix, the ids and `drawBehind` (Transform Again puts its copies above).
                let mut options = preview.1.clone();
                if let Some(o) = options.as_object_mut() {
                    o.remove("matrix");
                    o.remove("ids");
                    o.remove("drawBehind");
                }
                st.last_transform = Some(LastTransform::Matrix(m, options));
                st.last_perspective = None;
            }
        } else if preview.0 == "path.moveAnchors" && (moved.x != 0.0 || moved.y != 0.0) {
            // A drag of direct-selected anchors (from an anchor, or from a fill with anchors
            // selected): Transform Again moves the selected anchors and the objects selected whole
            // by the same distance. A drag that ends where it began leaves the last transform.
            let mut options = serde_json::json!({ "copy": false });
            for key in ["patterns", "interpolate"] {
                if let Some(v) = preview.1.get(key) {
                    options[key] = v.clone();
                }
            }
            st.last_transform = Some(LastTransform::Matrix(vectorcraft_geom::Affine::translate(moved), options));
            st.last_perspective = None;
        } else if preview.0 == "object.transformEach" && changed {
            // Transform Again repeats it as it stood when OK was pressed.
            st.last_transform = Some(LastTransform::Each(preview.1.clone()));
            st.last_perspective = None;
        }
        if perspective_again.is_some() {
            st.last_perspective = perspective_again;
        }
        let (doc, journal, grouped) = (self.active().map(|d| d.uid), self.journal.len(), self.undo_group_docs());
        self.note_shape_drawn(&preview.0, &preview.1);
        self.journal.push(preview);
        self.journal_in_undo_groups(doc, &grouped, journal);
        if self.depth == 0 {
            self.trim_histories();
        }
        Ok(())
    }

    /// Remember the paint a live preview applied (as its interaction is kept).
    pub(crate) fn remember_pending_paint(&mut self) {
        if let Some(p) = self.pending_paint.take() {
            self.remember_paint_now(&p);
        }
    }

    pub fn cancel_interaction(&mut self) -> Result<()> {
        self.put_back_interaction();
        self.clear_deferred_overlay_cache();
        self.decoded_images.empty();
        self.pending_paint = None;
        // While the Live Paint Bucket is active, a previewed color is the bucket's color, which the
        // cancel puts back as well.
        let bucket = self.bucket_paints();
        if let Some(paint) = self
            .active()
            .and_then(|st| st.interaction.as_ref())
            .filter(|it| bucket || it.preview.as_ref().is_some_and(|(id, _)| id == "command.batch") || it.color_mode_paints.is_some())
            .map(|it| it.paint.clone())
        {
            self.paint = paint;
        }
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        st.undo_interaction();
        // Guides locked and slices locked or hidden during the interaction stay deselected.
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        // A canceled preview's Assign Profile is undone with it.
        self.sync_front_profiles();
        // With New Art Has Basic Appearance off, new art takes the appearance of the selection as
        // the document has it again, not as a preview left it.
        self.inherit_new_art();
        Ok(())
    }

    pub fn in_interaction(&self) -> bool {
        self.active().is_some_and(|d| d.interaction.is_some())
    }
}

/// The selection a drag began with, without the objects that a command run during it removed,
/// hid or locked in `doc`.
fn drag_selection(began: &Selection, doc: &Document) -> Selection {
    let mut selection = began.clone();
    selection.prune(doc);
    let editable = doc.editable();
    let held: Vec<_> = selection.objects.iter().copied().filter(|&id| !editable(id)).collect();
    selection.remove_all(held);
    selection
}

/// Take out of the `ids` of a drag's preview `params` the objects that a command run during the
/// drag removed from `doc`, or hid or locked there while they were editable in `began` → false
/// when the preview named objects and none of them is left.
fn drag_targets(params: &mut Value, began: &Document, doc: &Document) -> bool {
    let Some(ids) = params.get_mut("ids").and_then(Value::as_array_mut) else { return true };
    if ids.is_empty() {
        return true;
    }
    let (was, now) = (began.editable(), doc.editable());
    ids.retain(|v| v.as_u64().map(vectorcraft_doc::NodeId).is_none_or(|id| doc.node(id).is_some() && (now(id) || !was(id))));
    !ids.is_empty()
}

/// The journal entries that select `selection` again in `doc` after `made`, the selection a
/// command left with `layer` current and `rows` highlighted in the Layers panel ([`object_steps`]).
/// When selecting the objects would make another layer current, `layer.setCurrent` makes `layer`
/// current again, as the drag leaves it, and `layer.highlight` gives back the highlighted rows.
fn selection_steps(
    selection: &Selection,
    made: &Selection,
    doc: &Document,
    layer: Option<vectorcraft_doc::NodeId>,
    rows: &[vectorcraft_doc::NodeId],
) -> Vec<(String, Value)> {
    let mut steps = object_steps(selection, doc);
    // `layer.target` on a layer makes it current; any other change to the selected objects makes
    // the layer of the last selected object current.
    let targeted = steps.first().is_some_and(|(id, _)| id == "layer.target");
    let selects = match selection.target.filter(|t| targeted && doc.node(*t).is_some_and(|n| n.is_layer())) {
        Some(t) => Some(t),
        None if selection.objects != made.objects => {
            selection.objects.last().and_then(|id| doc.layer_containing(*id)).filter(|l| doc.is_editable(*l))
        }
        None => None,
    };
    if let Some(layer) = layer.filter(|l| selects.is_some_and(|s| s != *l)) {
        steps.push(("layer.setCurrent".into(), serde_json::json!({ "id": layer.0 })));
        // `layer.setCurrent` highlights the layer's row. `layer.highlight` sets the rows the command
        // left instead; it keeps `layer` current while the last of them is on `layer`, or none is left.
        if rows != [layer] && rows.last().is_none_or(|r| doc.layer_containing(*r) == Some(layer)) {
            let ids: Vec<u64> = rows.iter().map(|r| r.0).collect();
            steps.push(("layer.highlight".into(), serde_json::json!({ "ids": ids, "mode": "set" })));
        }
    }
    steps
}

/// The journal entries that select `selection` in `doc`: the selections made on their own (its
/// slices with `object.slice.select`, its target with `layer.target`, its symbol part with
/// `symbol.instance.selectPart`, its Objects on Path member with
/// `object.objectsOnPath.selectMember`), or its objects in their order (`select.set`), the anchors
/// left out on paths that are not selected whole (`select.anchorsMany`), the anchors of paths with
/// every anchor selected (`select.anchors`) and its segments (`select.segments`); after its objects
/// or its target, its ruler guides (`guide.select`) and its key object, key guide or key anchor
/// (`select.key`).
fn object_steps(selection: &Selection, doc: &Document) -> Vec<(String, Value)> {
    use serde_json::json;
    if !selection.slices.is_empty() {
        let slices: Vec<u64> = selection.slices.iter().map(|id| id.0).collect();
        return vec![("object.slice.select".into(), json!({ "slices": slices }))];
    }
    // A target is selected again only while targeting it selects the same objects: when a command
    // added art to a targeted layer, `select.set` selects the drag's objects.
    let targets = |t: vectorcraft_doc::NodeId| match doc.node(t) {
        Some(n) if n.is_layer() => n.layer_art(true) == selection.objects,
        _ => selection.objects == [t],
    };
    let json_refs = |refs: &mut dyn Iterator<Item = &(usize, usize)>| Value::Array(refs.map(|&(a, b)| json!([a, b])).collect());
    // A targeted object keeps the ruler guides and the key selected beside it.
    let mut steps = if let Some(t) = selection.target.filter(|&t| targets(t)) {
        vec![("layer.target".to_string(), json!({ "id": t.0 }))]
    } else if let Some(part) = &selection.symbol_part {
        return vec![("symbol.instance.selectPart".into(), json!({ "id": part.instance.0, "path": part.path }))];
    } else if let Some(member) = selection.on_path_member {
        return vec![("object.objectsOnPath.selectMember".into(), json!({ "member": member.0 }))];
    } else {
        let ids: Vec<u64> = selection.objects.iter().map(|id| id.0).collect();
        let mut steps = vec![("select.set".to_string(), json!({ "ids": ids }))];
        // `select.set` selects each path whole: subtracting the anchors that are not selected leaves
        // the ones that are, and keeps the order of the objects. A path with every anchor selected
        // as anchors gets them back with `select.anchors`, which keeps them all as anchors.
        let mut every = Vec::new();
        let partial: Vec<Value> = selection
            .objects
            .iter()
            .filter_map(|id| {
                let refs = selection.anchors.get(id)?;
                let path = doc.node(*id)?.path_data()?;
                let out: Vec<(usize, usize)> = path.anchors().map(|(s, i, _)| (s, i)).filter(|r| !refs.contains(r)).collect();
                if out.is_empty() {
                    every.push(("select.anchors".to_string(), json!({ "id": id.0, "anchors": json_refs(&mut refs.iter()), "mode": "add" })));
                    return None;
                }
                Some(json!({ "id": id.0, "anchors": json_refs(&mut out.iter()) }))
            })
            .collect();
        if !partial.is_empty() {
            steps.push(("select.anchorsMany".into(), json!({ "items": partial, "mode": "subtract" })));
        }
        steps.extend(every);
        for (id, refs) in &selection.segments {
            steps.push(("select.segments".into(), json!({ "id": id.0, "segments": json_refs(&mut refs.iter()), "mode": "add" })));
        }
        steps
    };
    if !selection.guides.is_empty() {
        let indexes = cmd::guide_indexes(doc, &selection.guides);
        steps.push(("guide.select".into(), json!({ "indexes": indexes, "toggle": true })));
    }
    if let Some((id, (s, i))) = selection.key_anchor {
        steps.push(("select.key".into(), json!({ "id": id.0, "anchor": [s, i] })));
    } else if let Some(g) = selection.key_guide.and_then(|g| cmd::guide_indexes(doc, &[g]).first().copied()) {
        steps.push(("select.key".into(), json!({ "guide": g })));
    } else if let Some(key) = selection.key {
        steps.push(("select.key".into(), json!({ "id": key.0 })));
    }
    steps
}
