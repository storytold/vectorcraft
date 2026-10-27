use super::*;

impl Session {
    /// Run `f` on a mutable copy of the active document. Outside an interaction this records one
    /// undo step labelled `label`; inside one it just mutates (the interaction commits later).
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Selection) -> Result<T>) -> Result<T> {
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
        // Objects attached to a path follow it, and dimensions follow the art they measure.
        let result = match f(doc, &mut st.selection).and_then(|v| {
            // Global Edit: the other members take what the edit changed about the one it edited.
            if let Some(ge) = &st.global_edit {
                spread = cmd::globaledit::propagate(&before, doc, ge, [&before_sel.objects, &st.selection.objects]);
            }
            cmd::objects_on_path::refresh(&before, doc);
            cmd::dimension::refresh(&before, doc)?;
            Ok(v)
        }) {
            Ok(v) => match cmd::shaper::refresh(&before, Arc::make_mut(&mut st.doc)) {
                Err(e) => Err(e),
                Ok(()) => {
                    // Attached Shaper groups whose art was rebuilt go back on their path.
                    cmd::objects_on_path::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Inline graphics in text follow their symbols.
                    cmd::inline::refresh(&before, Arc::make_mut(&mut st.doc));
                    // Text Wrap: area type follows its wrap objects; then threads re-flow.
                    cmd::textwrap::refresh(Arc::make_mut(&mut st.doc));
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
                    // Asset Export: assets let go of deleted art.
                    if !st.doc.assets.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_assets();
                    }
                    // Variables: bindings let go of deleted art.
                    if !st.doc.variables.bindings.is_empty() {
                        Arc::make_mut(&mut st.doc).prune_variable_bindings();
                    }
                    if st.doc.too_deep(Some(&before)) {
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
        match result {
            Ok(v) => {
                st.selection.prune(&st.doc);
                // An edit that selects what it unlocks, shows or pastes leaves locked guides out,
                // and one that selects the slices it makes leaves them out while they are locked
                // or hidden.
                deselect_hidden(&menu, &st.doc, &mut st.selection);
                // The art on a locked artboard can't stay selected, however it got there, except
                // the new art this edit made and new art still selected.
                st.protect_locked_artboards(&before_sel.objects, &before_sel.new_art, Some(&*before));
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
        }
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

    pub fn begin_interaction(&mut self, label: &str) -> Result<()> {
        // Typing in progress is kept first: a drag or a dialog's preview would otherwise start
        // from the snapshot taken before the typing, and drop it (#1881).
        if label != "Typing" && cmd::typing_in_progress(self) {
            cmd::settle_interaction(self)?;
        }
        self.clear_deferred_overlay_cache();
        let st = self.doc_mut()?;
        if st.interaction.is_some() {
            return Ok(());
        }
        st.interaction = Some(Interaction {
            label: label.to_string(),
            doc: st.doc.clone(),
            selection: Arc::new(st.selection.clone()),
            preview: None,
            active_layer: st.active_layer,
            layer_rows: st.layer_rows.clone(),
            variables_highlight: st.variables_highlight.clone(),
            isolation: st.isolation,
            perspective_again: None,
            copies: None,
        });
        Ok(())
    }

    /// Re-apply `cmd` on top of the interaction snapshot (replacing the previous preview).
    pub fn preview(&mut self, cmd: &str, params: &Value) -> Result<Value> {
        {
            let menu = self.menu.clone();
            let st = self.doc_mut()?;
            let Some(it) = &st.interaction else { return Err(EngineError::Other("no interaction in progress".into())) };
            // Undoing the previous preview counts as a change, so views redraw even when the command
            // below returns without an edit.
            let selection_changed = st.selection != *it.selection;
            if !Arc::ptr_eq(&st.doc, &it.doc) || selection_changed {
                st.revision += 1;
            }
            st.doc = it.doc.clone();
            if selection_changed {
                st.selection.clone_from(&it.selection);
            }
            // Guides locked and slices locked or hidden during the interaction stay deselected,
            // even when the preview fails.
            deselect_hidden(&menu, &st.doc, &mut st.selection);
            cmd::globaledit::follow_history(st);
        }
        // The previous preview's paint is undone with it.
        self.pending_paint = None;
        let rw = cmd::distortcmds::perspective_rewrite(self, cmd, params).or_else(|| cmd::bounds_rewrite(self, cmd, params));
        let (cmd, params) = rw.as_ref().map_or((cmd, params), |(c, p)| (c.as_str(), p));
        let outer = std::mem::replace(&mut self.previewing, true);
        let r = self.execute(cmd, params);
        self.previewing = outer;
        let params = self.noted(params);
        let st = self.doc_mut()?;
        if let Some(it) = &mut st.interaction {
            // A failed preview leaves the document at the snapshot, so nothing is left to commit
            // (not even an earlier successful preview, which the reset above already undid).
            it.preview = r.is_ok().then(|| (cmd.to_string(), params));
        }
        r
    }

    pub fn commit_interaction(&mut self) -> Result<()> {
        self.clear_deferred_overlay_cache();
        self.remember_pending_paint();
        let st = self.doc_mut()?;
        let Some(mut it) = st.interaction.take() else { return Ok(()) };
        let Some(preview) = it.preview.take() else { return Ok(()) };
        let perspective_again = it.perspective_again.take();
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
        st.keep_interaction(it);
        if preview.0 == "object.transform" {
            let m = cmd::matrix_param(&preview.1, "matrix");
            if let Some(m) = m {
                // The options are the last preview's params with the values it noted, without the
                // matrix and the ids.
                let mut options = preview.1.clone();
                if let Some(o) = options.as_object_mut() {
                    o.remove("matrix");
                    o.remove("ids");
                }
                st.last_transform = Some(LastTransform::Matrix(m, options));
                st.last_perspective = None;
            }
        } else if preview.0 == "object.transformEach" && changed {
            // Transform Again repeats it as it stood when OK was pressed.
            st.last_transform = Some(LastTransform::Each(preview.1.clone()));
            st.last_perspective = None;
        }
        if perspective_again.is_some() {
            st.last_perspective = perspective_again;
        }
        let (doc, journal, grouped) = (self.active().map(|d| d.uid), self.journal.len(), self.undo_group_docs());
        self.journal.push(preview);
        self.journal_in_undo_groups(doc, &grouped, journal);
        Ok(())
    }

    /// Remember the paint a live preview applied (as its interaction is kept).
    pub(crate) fn remember_pending_paint(&mut self) {
        if let Some(p) = self.pending_paint.take() {
            self.remember_paint_now(&p);
        }
    }

    pub fn cancel_interaction(&mut self) -> Result<()> {
        self.clear_deferred_overlay_cache();
        self.pending_paint = None;
        let menu = self.menu.clone();
        let st = self.doc_mut()?;
        st.undo_interaction();
        // Guides locked and slices locked or hidden during the interaction stay deselected.
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        Ok(())
    }

    pub fn in_interaction(&self) -> bool {
        self.active().is_some_and(|d| d.interaction.is_some())
    }
}
