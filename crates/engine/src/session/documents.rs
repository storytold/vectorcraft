use std::collections::HashSet;

use vectorcraft_doc::{Node, TextObject};

use super::{
    Arc, Clipboard, Color, CommandInfo, DocState, Document, DrawMode, EngineError, GradientPaint, Journal, NodeKind, PaintDefaults, Prefs, Result,
    Session, ViewInfo, cmd, command_specs, file_access,
};

impl Session {
    /// A session with no document open, the default preferences and the Selection tool. It also
    /// installs the hooks the document and effects crates call back into: the loader and file reader
    /// of placed documents, where an image's effects draw its fills and strokes, for clicks, and how
    /// Vector Halftone's dots are sampled, for exports.
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
        // Clicks find a blend's steps and an envelope's distorted art where the canvas draws them,
        // evaluated again when the fonts or the plug-ins change.
        vectorcraft_doc::hit::set_live_art(vectorcraft_render::effects::expand_live, || {
            vectorcraft_text::FontDb::global().generation().rotate_left(32) ^ vectorcraft_render::effects::plugin_revision()
        });
        // Envelopes map type from the bounds of its glyph outlines, underlines and strikethroughs.
        vectorcraft_text::register_envelope_ink();
        // Exports bake Vector Halftone into the dots the renderer samples.
        vectorcraft_render::effects::addons::halftone::set_art(vectorcraft_render::addons::halftone::art);
        // OpenType SVG glyphs are drawn as the SVG importer reads their documents.
        vectorcraft_text::set_svg_glyph_reader(vectorcraft_svg::glyph_art);
        Self {
            docs: vec![],
            active: None,
            prefs: Prefs::default(),
            caps_lock: false,
            paint: PaintDefaults::default(),
            fill_active: true,
            clipboard: Clipboard::default(),
            journal: Journal::default(),
            tool: vectorcraft_tools::create("selection"),
            lender: None,
            held_input: Default::default(),
            input_count: 0,
            edit_started_by: None,
            settling_tool_edit: false,
            held_ui: vec![],
            tool_step: None,
            selection_at_lend: None,
            lent_selection: None,
            last_selection_tool: None,
            last_view: ViewInfo::default(),
            preview_memo: Default::default(),
            depth: 0,
            replaying: false,
            tool_panic: None,
            tool_resets: 0,
            tool_set_since_reset: false,
            draw_mode: DrawMode::Normal,
            draw_inside: None,
            untitled_counter: 0,
            menu: Default::default(),
            gradient_stop: None,
            active_appearance_item: None,
            eyedropper_sample: None,
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
            trace_preset_changes: Default::default(),
            ui_language: None,
            journal_note: Default::default(),
            note_depth: 1,
            journal_query: false,
            explicit_undo_group: None,
            batch_stash: None,
            batch_steps: None,
            batch_paint_docs: Vec::new(),
            composite_names: Vec::new(),
            composite_names_registered: HashSet::new(),
            reading: false,
            place_hand_off: None,
            queue_hand_off: None,
            place_queue_roots: None,
            image_hand_off: None,
            trace_hand_off: None,
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
            press_options: None,
            press_type_selection: None,
            press_point: None,
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
            keyboard_text: false,
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
        self.press_type_selection = None;
        let view = self.last_view;
        // The edit the tool computes on another thread and the input held for it belong to the
        // document being left. The switch goes on whatever they did, as for the deactivation.
        let _ = self.finish_tool_edit_later();
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

    /// Finish the tool's pending work and give it fresh state, as a switch to another document
    /// does, while the current document stays the same (a switch to another view of it).
    pub fn reset_tool(&mut self) {
        self.reset_tool_for_doc_switch();
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index < self.docs.len() {
            let switched = self.active != Some(index);
            if switched {
                self.reset_tool_for_doc_switch();
            }
            self.active = Some(index);
            if switched {
                self.sync_front_composite_fonts();
            }
            true
        } else {
            false
        }
    }

    /// Register the composite fonts of the document now in front ([`cmd::compositefonts::sync`]),
    /// after a switch or a close brought it there. Its type set in a composite font that differs
    /// from the one of that name before (added, changed or removed) is measured again, and type not
    /// measured yet (in a document replaced while in the back) is measured now.
    fn sync_front_composite_fonts(&mut self) {
        let changed: HashSet<String> = cmd::compositefonts::sync(self).into_iter().collect();
        let Some(index) = self.active else { return };
        self.refresh_document_text_bounds(index, |t| t.cached_bounds.is_none() || sets_in_any(t, &changed));
    }

    /// The name [`Session::next_untitled`] gives next (New Document's Name field).
    pub fn peek_untitled(&self) -> String {
        format!("Untitled-{}", self.untitled_counter + 1)
    }

    pub fn next_untitled(&mut self) -> String {
        self.untitled_counter += 1;
        format!("Untitled-{}", self.untitled_counter)
    }

    /// Text layout bounds are a cache (not saved): compute them after its fonts are registered, or
    /// selection boxes and hit testing would use the rough estimate until each text is edited, and
    /// what is fitted to type's bounds (an envelope warping it) would come out differently. That
    /// holds for the type in symbol definitions, pattern tiles, graph designs and opacity masks too,
    /// not only in the layers (#1738).
    pub(crate) fn refresh_text_bounds(doc: &mut Document) {
        // Inline graphics in text take their size from their symbols' art (not saved either).
        doc.resolve_inline_art();
        let mut uncached = |t: &TextObject| t.cached_bounds.is_none();
        for tree in text_trees_mut(doc) {
            each_text_where(tree, &mut uncached, &mut cmd::typecmd::refresh_bounds_for_first_frame);
        }
    }

    /// Measure the type in open document `index` that `stale` picks (in its trees as
    /// [`Session::refresh_text_bounds`] walks them), as a change that is not an edit: a saved
    /// document stays saved, and its undo history keeps its documents as they are. The document is
    /// left as it is when `stale` picks nothing.
    fn refresh_document_text_bounds(&mut self, index: usize, mut stale: impl FnMut(&TextObject) -> bool) {
        let Some(st) = self.docs.get_mut(index) else { return };
        if !text_trees(&st.doc).any(|n| has_text_where(n, &mut stale)) {
            return;
        }
        st.change_unsaved(|doc| {
            let doc = Arc::make_mut(doc);
            for tree in text_trees_mut(doc) {
                each_text_where(tree, &mut stale, &mut cmd::typecmd::refresh_bounds_for_first_frame);
            }
            // Inline graphics take their size from their symbols' art, whose type may just have
            // been measured again: the text holding one whose size changed is measured again too.
            if !doc.resolve_inline_art().is_empty() {
                for tree in text_trees_mut(doc) {
                    each_text_where(tree, &mut |t| t.cached_bounds.is_none(), &mut cmd::typecmd::refresh_bounds_for_first_frame);
                }
            }
        });
    }

    /// Register the composite fonts of open document `index`, which is in front, and measure its
    /// type not measured yet and its type set in a font of a name that a composite font of an open
    /// document or of the font database has, or had before: that type may have been laid out in
    /// another document's composite fonts (those registered while its file was read for opening).
    fn sync_opened_composite_fonts(&mut self, index: usize) {
        let mut names = cmd::compositefonts::names_in_use(self);
        names.extend(cmd::compositefonts::sync(self));
        self.refresh_document_text_bounds(index, |t| t.cached_bounds.is_none() || sets_in_any(t, &names));
    }

    /// Add a document and make it active. Between `edit.undoGroup.begin` and `edit.undoGroup.end`,
    /// an undo group with the begun group's name opens in it: here outside a command, and after the
    /// command that adds it otherwise ([`Session::execute`]).
    pub fn add_document(&mut self, mut doc: Document, path: Option<String>) -> usize {
        // Every edit is checked against the document before it, so the first one is within the cap
        // (importers and the native reader have seen to theirs already, with a warning).
        doc.limit_depth();
        cmd::compositefonts::filter_loaded(&mut doc);
        // Inline graphics in text take their size from their symbols' art (not saved).
        doc.resolve_inline_art();
        // Guides from files saved before guides had layers go on a layer.
        doc.adopt_guides();
        cmd::dimension::conform(&mut doc);
        self.reset_tool_for_doc_switch();
        let mut st = DocState::new(doc, path);
        st.history.limit = self.prefs.history_states as usize;
        st.opened_as_tab = self.prefs.open_documents_as_tabs;
        self.docs.push(st);
        let i = self.docs.len() - 1;
        self.active = Some(i);
        self.sync_opened_composite_fonts(i);
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
        cmd::compositefonts::filter_loaded(&mut doc);
        doc.resolve_inline_art();
        doc.adopt_guides();
        cmd::dimension::conform(&mut doc);
        // In the back, type set in a font named like a composite font is measured when the document
        // comes to the front and its own composite fonts are registered, not in the front
        // document's.
        let mut back_names = None;
        if self.active == Some(index) {
            // Pending tool work (typing, a drag) belongs to the content being thrown away.
            self.reset_tool_for_doc_switch();
        } else {
            let mut names = cmd::compositefonts::names_in_use(self);
            names.extend(doc.composite_fonts.iter().map(|f| f.name.to_lowercase()));
            for tree in text_trees_mut(&mut doc) {
                each_text_where(tree, &mut |t| t.cached_bounds.is_some() && sets_in_any(t, &names), &mut |t| {
                    t.cached_bounds = None;
                    t.cached_baselines.clear();
                });
            }
            back_names = Some(names);
        }
        let old = &self.docs[index];
        let mut st = DocState::new(doc, old.path.clone());
        st.history.limit = old.history.limit;
        // Same open document (caches keyed by uid stay valid); a new revision redraws it.
        st.uid = old.uid;
        st.opened_as_tab = old.opened_as_tab;
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
        match back_names {
            Some(names) => self.refresh_document_text_bounds(index, |t| t.cached_bounds.is_none() && !sets_in_any(t, &names)),
            None => self.sync_opened_composite_fonts(index),
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
        self.sync_front_composite_fonts();
        true
    }

    /// Move the document in tab `from` to tab `to` (a document tab dragged along the strip,
    /// `document.move`); the documents between them shift by one. The current document stays the
    /// current one, wherever it moves to, and its tool keeps its state: no document is switched to.
    /// False, changing nothing, when either index is past the last document.
    pub fn move_document(&mut self, from: usize, to: usize) -> bool {
        let n = self.docs.len();
        if from >= n || to >= n {
            return false;
        }
        if from == to {
            return true;
        }
        let active = self.active().map(|d| d.uid);
        let d = self.docs.remove(from);
        self.docs.insert(to, d);
        if let Some(uid) = active {
            self.active = self.docs.iter().position(|d| d.uid == uid);
        }
        true
    }

    /// Commands with enablement (for menus, palette, MCP `list_commands`), the installed plug-ins'
    /// commands last.
    pub fn commands(&self) -> Vec<CommandInfo> {
        command_specs().iter().chain(crate::cmd::plugin::command_specs()).map(|c| c.info(self)).collect()
    }
}

/// Whether some of type `t` is set in a font named (in any case) one of `names`, lowercased.
fn sets_in_any(t: &TextObject, names: &HashSet<String>) -> bool {
    !names.is_empty() && t.runs.iter().any(|r| names.contains(&r.style.font_family.to_lowercase()))
}

/// The trees of `doc` that hold type: its layers, symbol definitions, graph designs and pattern
/// tiles.
fn text_trees(doc: &Document) -> impl Iterator<Item = &Arc<Node>> {
    let trees = doc.layers.iter().chain(doc.symbols.iter().map(|s| &s.art));
    let trees = trees.chain(doc.graph_designs.iter().map(|g| &g.art));
    trees.chain(doc.patterns.iter().flat_map(|p| p.art.iter()))
}

/// [`text_trees`], to change.
fn text_trees_mut(doc: &mut Document) -> impl Iterator<Item = &mut Arc<Node>> {
    let trees = doc.layers.iter_mut().chain(doc.symbols.iter_mut().map(|s| &mut s.art));
    let trees = trees.chain(doc.graph_designs.iter_mut().map(|g| &mut g.art));
    trees.chain(doc.patterns.iter_mut().flat_map(|p| p.art.iter_mut()))
}

/// Whether tree `n` (opacity mask art included) holds type that `pick` picks.
fn has_text_where(n: &Node, pick: &mut dyn FnMut(&TextObject) -> bool) -> bool {
    matches!(&n.kind, NodeKind::Text(t) if pick(t))
        || n.extras().mask.as_deref().is_some_and(|m| has_text_where(&m.art, pick))
        || n.children().is_some_and(|ch| ch.iter().any(|c| has_text_where(c, pick)))
}

/// Run `act` on the type in tree `n` (opacity mask art included) that `pick` picks. Only the nodes
/// on the way to such type are copied.
fn each_text_where(n: &mut Arc<Node>, pick: &mut dyn FnMut(&TextObject) -> bool, act: &mut dyn FnMut(&mut TextObject)) {
    if !has_text_where(n, pick) {
        return;
    }
    let n = Arc::make_mut(n);
    if let NodeKind::Text(t) = &mut n.kind
        && pick(t)
    {
        act(t);
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
        each_text_where(&mut m.art, pick, act);
    }
    for c in n.children_mut().into_iter().flatten() {
        each_text_where(c, pick, act);
    }
}
