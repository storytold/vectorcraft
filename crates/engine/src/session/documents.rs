use super::*;

impl Session {
    /// A session with no document open, the default preferences and the Selection tool. It also
    /// installs the hooks the document crate calls back into: the loader and file reader of placed
    /// documents, and where an image's effects draw its fills and strokes, for clicks.
    pub fn new() -> Self {
        // Placed documents are read by the native format's loader.
        vectorcraft_doc::placed_document::set_loader(cmd::place::document::read_document);
        vectorcraft_doc::placed_document::set_file_reader(cmd::place::document::read_file_again);
        vectorcraft_doc::placed_document::set_read_check(|path| file_access::check_read(path).is_ok());
        // The background workers of placed documents read files in the scope that asked.
        vectorcraft_render::placed_document::set_scope_capture(|| {
            let scope = file_access::ScopeHandle::capture();
            Box::new(move |f: &mut dyn FnMut()| scope.run(f))
        });
        // Clicks find an image's own fills and strokes where their effects draw them.
        vectorcraft_doc::hit::set_image_paint(vectorcraft_render::effects::image_paint);
        // OpenType SVG glyphs are drawn as the SVG importer reads their documents.
        vectorcraft_text::set_svg_glyph_reader(vectorcraft_svg::glyph_art);
        Self {
            docs: vec![],
            active: None,
            prefs: Prefs::default(),
            paint: PaintDefaults::default(),
            fill_active: true,
            clipboard: Clipboard::default(),
            journal: Journal::default(),
            tool: vectorcraft_tools::create("selection"),
            lender: None,
            last_selection_tool: None,
            last_view: ViewInfo::default(),
            preview_memo: Default::default(),
            depth: 0,
            tool_panic: None,
            tool_resets: 0,
            tool_set_since_reset: false,
            draw_mode: DrawMode::Normal,
            draw_inside: None,
            untitled_counter: 0,
            menu: Default::default(),
            gradient_stop: None,
            active_appearance_item: None,
            last_solid: Color::WHITE,
            last_gradient: GradientPaint::new(Default::default()),
            recent_colors: vec![],
            pending_paint: None,
            swatch_libraries: Default::default(),
            freeform_point: None,
            style_libraries: Default::default(),
            symbol_libraries: Default::default(),
            brush_libraries: Default::default(),
            libraries: Default::default(),
            pdf_preset_dir: None,
            profile_dir: None,
            plugin_dir: None,
            plugin_output: None,
            recent_urls: vec![],
            ui_language: None,
            journal_note: Default::default(),
            note_depth: 1,
            journal_query: false,
            explicit_undo_group: None,
            batch_stash: None,
            batch_steps: None,
            place_hand_off: None,
            queue_hand_off: None,
            place_cursor_requests: false,
            package_hand_off: None,
            screens_hand_off: None,
            recovery: Default::default(),
            in_tool_actions: false,
            previewing: false,
            envelope_defaults: None,
            liquify_stroke: None,
            plane_widget_press: false,
            pointer_held: false,
            ruler_guide: None,
            port_marks: Default::default(),
            font_search: None,
            link_search: None,
            search_rules: None,
            deferred_overlay_cache: None,
            search_threads: None,
            link_stamps: Default::default(),
            link_scan: None,
            global_edit_options: Default::default(),
        }
    }

    pub fn documents(&self) -> &[DocState] {
        &self.docs
    }

    /// The open document with [`DocState::uid`] `uid` (it may have closed since it was looked up).
    pub fn document_mut(&mut self, uid: u64) -> Option<&mut DocState> {
        self.docs.iter_mut().find(|d| d.uid == uid)
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn active(&self) -> Option<&DocState> {
        self.active.and_then(|i| self.docs.get(i))
    }

    pub fn active_mut(&mut self) -> Option<&mut DocState> {
        self.active.and_then(|i| self.docs.get_mut(i))
    }

    pub fn doc(&self) -> Result<&DocState> {
        self.active().ok_or(EngineError::NoDocument)
    }

    pub fn doc_mut(&mut self) -> Result<&mut DocState> {
        self.active_mut().ok_or(EngineError::NoDocument)
    }

    /// Finish the tool's pending work in the current document and give it fresh state, so nothing
    /// (e.g. uncommitted typing) leaks into the document we are switching to.
    fn reset_tool_for_doc_switch(&mut self) {
        if self.active.is_none() {
            return;
        }
        let view = self.last_view;
        // The switch goes on whatever the lent tool's last actions did, as for the deactivation.
        let _ = self.give_back_tool(view);
        let acts = self.with_tool_cx(view, |t, cx| t.deactivate(cx));
        let _ = self.apply_actions(acts);
        // A batch leaves its interaction open in the document it leaves, to keep or roll back with
        // the rest of the batch; anything else in progress (a drag) is cancelled.
        if self.batch_stash.is_none() {
            let _ = self.cancel_interaction();
        }
        self.keep_tool_settings();
        self.tool = self.make_tool(self.tool.id());
        self.tool_resets = self.tool_resets.wrapping_add(1);
        self.tool_set_since_reset = false;
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            if self.active != Some(index) {
                self.reset_tool_for_doc_switch();
            }
            self.active = Some(index);
            true
        } else {
            false
        }
    }

    /// The name [`Session::next_untitled`] gives next (New Document's Name field).
    pub fn peek_untitled(&self) -> String {
        format!("Untitled-{}", self.untitled_counter + 1)
    }

    pub fn next_untitled(&mut self) -> String {
        self.untitled_counter += 1;
        format!("Untitled-{}", self.untitled_counter)
    }

    /// Text layout bounds are a cache (not saved): compute them for a document just read, or
    /// selection boxes and hit testing would use the rough estimate until each text is edited, and
    /// what is fitted to type's bounds (an envelope warping it) would come out differently. That
    /// holds for the type in symbol definitions, pattern tiles, graph designs and opacity masks too,
    /// not only in the layers (#1738).
    pub(crate) fn refresh_text_bounds(doc: &mut Document) {
        // Inline graphics in text take their size from their symbols' art (not saved either).
        doc.resolve_inline_art();
        let trees = doc.layers.iter_mut().chain(doc.symbols.iter_mut().map(|s| &mut s.art));
        let trees = trees.chain(doc.graph_designs.iter_mut().map(|g| &mut g.art));
        for tree in trees.chain(doc.patterns.iter_mut().flat_map(|p| p.art.iter_mut())) {
            refresh_tree_text_bounds(tree);
        }
    }

    /// Add a document and make it active. Between `edit.undoGroup.begin` and `edit.undoGroup.end`,
    /// an undo group with the begun group's name opens in it: here outside a command, and after the
    /// command that adds it otherwise ([`Session::execute`]).
    pub fn add_document(&mut self, mut doc: Document, path: Option<String>) -> usize {
        // Every edit is checked against the document before it, so the first one is within the cap
        // (importers and the native reader have seen to theirs already, with a warning).
        doc.limit_depth();
        Self::refresh_text_bounds(&mut doc);
        // Guides from files saved before guides had layers go on a layer.
        doc.adopt_guides();
        cmd::dimension::conform(&mut doc);
        self.reset_tool_for_doc_switch();
        let mut st = DocState::new(doc, path);
        st.history.limit = self.prefs.history_states as usize;
        self.docs.push(st);
        let i = self.docs.len() - 1;
        self.active = Some(i);
        if self.depth == 0
            && let Some(label) = self.explicit_undo_group.clone()
        {
            self.begin_named_undo_group(label);
        }
        i
    }

    /// Replace the document in tab `index` (File → Revert): new content, cleared history and
    /// selection, saved state. The tab keeps its place, path, format and view.
    pub fn replace_document(&mut self, index: usize, mut doc: Document) -> bool {
        if index >= self.docs.len() {
            return false;
        }
        doc.limit_depth();
        Self::refresh_text_bounds(&mut doc);
        doc.adopt_guides();
        cmd::dimension::conform(&mut doc);
        if self.active == Some(index) {
            // Pending tool work (typing, a drag) belongs to the content being thrown away.
            self.reset_tool_for_doc_switch();
        }
        let old = &self.docs[index];
        let mut st = DocState::new(doc, old.path.clone());
        st.history.limit = old.history.limit;
        // Same open document (caches keyed by uid stay valid); a new revision redraws it.
        st.uid = old.uid;
        st.revision = old.revision + 1;
        st.format = old.format;
        st.save_options = old.save_options.clone();
        // The remembered artboard pick follows the artboards it named when the file read back has
        // the artboards the document had at the save, in the same order and with the same ids (a
        // native file keeps them; a PDF's pages get new ids in page order); otherwise it is left
        // out.
        if old.save_artboards.is_empty() || st.doc.artboards.iter().map(|a| a.id).eq(old.save_artboards.iter().copied()) {
            st.save_artboards = old.save_artboards.clone();
        } else {
            for k in cmd::fileio::ARTBOARD_PARAMS {
                st.save_options.remove(k);
            }
        }
        st.converted = old.converted;
        st.imported_from = old.imported_from.clone();
        st.import_losses = old.import_losses.clone();
        st.view = old.view.clone();
        st.layers_open = old.layers_open.clone();
        let old = std::mem::replace(&mut self.docs[index], st);
        if let Some(stash) = &mut self.batch_stash {
            stash.push(old);
        }
        true
    }

    pub fn close_document(&mut self, index: usize) -> bool {
        if index >= self.docs.len() {
            return false;
        }
        self.reset_tool_for_doc_switch();
        // Closed (saved or discarded): nothing left to recover.
        let uid = self.docs[index].uid;
        cmd::recovery::forget(self, uid);
        let old = self.docs.remove(index);
        if let Some(stash) = &mut self.batch_stash {
            stash.push(old);
        }
        self.active = if self.docs.is_empty() { None } else { Some(index.min(self.docs.len() - 1)) };
        true
    }

    /// Commands with enablement (for menus, palette, MCP `list_commands`).
    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().map(|c| c.info(self)).collect()
    }
}

/// Lay out the type in tree `n` (opacity mask art included) whose bounds aren't cached. Only the
/// nodes on the way to such type are copied.
fn refresh_tree_text_bounds(n: &mut Arc<vectorcraft_doc::Node>) {
    fn stale(n: &vectorcraft_doc::Node) -> bool {
        matches!(&n.kind, NodeKind::Text(t) if t.cached_bounds.is_none())
            || n.extras().mask.as_deref().is_some_and(|m| stale(&m.art))
            || n.children().is_some_and(|ch| ch.iter().any(|c| stale(c)))
    }
    if !stale(n) {
        return;
    }
    let n = Arc::make_mut(n);
    if let NodeKind::Text(t) = &mut n.kind
        && t.cached_bounds.is_none()
    {
        cmd::typecmd::refresh_bounds(t);
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
        refresh_tree_text_bounds(&mut m.art);
    }
    for c in n.children_mut().into_iter().flatten() {
        refresh_tree_text_bounds(c);
    }
}
