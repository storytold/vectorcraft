use super::{Value, VectorcraftApp, View, ViewInfo, panels};

impl VectorcraftApp {
    /// Keep `views` aligned with the session's documents (a new one starts at its saved view).
    pub fn sync_views(&mut self) {
        let docs = self.session.documents();
        self.views.truncate(docs.len());
        self.views.extend(docs.iter().skip(self.views.len()).map(View::of));
        for (view, doc) in self.views.iter_mut().zip(docs) {
            view.artboard = view.artboard.min(doc.doc.artboards.len().saturating_sub(1));
        }
        // Undo can remove the Artboard tool's active or additionally selected boards. Repair only
        // invalid indices: valid multiselection, drag state and tool options must survive history.
        if self.session.tool_id() != "artboard" || self.session.tool_busy() {
            return;
        }
        let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0) else { return };
        let Some(view) = self.session.active_index().and_then(|i| self.views.get_mut(i)) else { return };
        let options = self.session.tool_options();
        let Some(selected) = options["selected"].as_array() else { return };
        let mut valid: Vec<usize> = selected.iter().filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).filter(|i| *i < n).collect();
        if valid.len() != selected.len() {
            if valid.is_empty() {
                valid.push(view.artboard);
            }
            if let Some(active) = valid.first() {
                view.artboard = *active;
            }
            self.session.set_tool_option("selected", &serde_json::json!(valid));
        }
    }

    /// The active document's uid, how many times the tool has started afresh
    /// ([`vectorcraft_engine::Session::tool_resets`]) and the artboards the Artboard tool has
    /// selected (null with another tool), which [`Self::follow_artboard_tool`] compares before and
    /// after a command.
    pub(super) fn artboard_tool_state(&self) -> (Option<u64>, u64, Value) {
        let selected = if self.session.tool_id() == "artboard" { self.session.tool_options()["selected"].clone() } else { Value::Null };
        (self.session.active().map(|d| d.uid), self.session.tool_resets(), selected)
    }

    /// After a command run with the Artboard tool, given [`Self::artboard_tool_state`] from before
    /// it: when the command chose another document, or started the tool afresh in the same one
    /// (File › Revert, a `command.batch` or `action.batch` that reverts it or switches to another
    /// document and back), the tool takes the document's active artboard, as choosing its tab
    /// does. When the command changed the artboards the tool has selected (Paste and Cut with the
    /// tool, `tool.setOption`), also after starting it afresh (a batch step after the revert or the
    /// switch), the tool's active artboard becomes the active one (the view's), as a click with the
    /// tool makes it.
    pub(super) fn follow_artboard_tool(&mut self, before: (Option<u64>, u64, Value)) {
        self.sync_views();
        if self.session.tool_id() != "artboard" {
            return;
        }
        let Some(n) = self.session.active().map(|d| d.doc.artboards.len()).filter(|n| *n > 0) else { return };
        let (doc, resets, selected) = self.artboard_tool_state();
        // The tool started afresh during the command, and no later step (a Paste or Cut) set its options.
        let reset = resets != before.1;
        let fresh = reset && !self.session.tool_set_since_reset();
        if fresh || (doc != before.0 && !reset) {
            panels::artboards::select(self, panels::artboards::selected(self, n));
        } else if (reset || selected != before.2)
            && let Some(i) = self.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
            && i < n
            && let Some(v) = self.view_mut()
        {
            v.artboard = i;
        }
    }

    pub fn view(&self) -> Option<&View> {
        self.session.active_index().and_then(|i| self.views.get(i))
    }

    /// The zero point shown by rulers and position fields: the document's with global rulers, the
    /// active artboard's with artboard rulers. Command coordinates stay absolute.
    pub(crate) fn ruler_origin(&self) -> vectorcraft_geom::Point {
        let artboard = self.view().map_or(0, |v| v.artboard);
        self.session.active().map_or(vectorcraft_geom::Point::ZERO, |d| d.doc.ruler_origin_for(artboard))
    }

    pub fn view_mut(&mut self) -> Option<&mut View> {
        self.sync_views();
        let i = self.session.active_index()?;
        self.views.get_mut(i)
    }

    pub fn view_info(&self) -> ViewInfo {
        ViewInfo {
            zoom: self.view().map(|v| v.zoom).unwrap_or(1.0),
            outline: self.ui.view.outline,
            smart_guides: self.ui.view.smart_guides,
            guides: self.ui.view.guides,
            snap_to_grid: self.ui.view.snap_to_grid,
            snap_to_pixel: self.ui.view.snap_to_pixel,
            // View › Hide Edges: the canvas draws neither the bounding box (with the type widget)
            // nor the Live Corners widgets, and the tools do not answer the pointer on them.
            show_bbox: self.ui.view.bounding_box && self.ui.view.edges,
            snap_to_point: self.ui.view.snap_to_point,
            snap_to_glyph: self.ui.view.snap_to_glyph,
            corner_widgets: self.ui.view.corner_widgets && self.ui.view.edges,
            screen: self.screen_frame(),
            artboard: self.view().map_or(0, |v| v.artboard),
        }
    }
}

impl VectorcraftApp {
    /// The selection's bounding box, rotated with rotated objects ([`Session::transform_box`]); with
    /// anchors selected with Direct Selection, the box around them square to the page, which
    /// `object.setBounds` then edits (#1824). The canvas and the transform fields read it every
    /// frame: it is measured once per revision.
    pub fn selection_box(&mut self) -> Option<vectorcraft_doc::OrientedBox> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision, self.session.prefs.use_preview_bounds);
        if let Some((k, b)) = self.canvas.selection_box
            && k == key
        {
            return b;
        }
        let b = match vectorcraft_effects::preview::anchor_bounds(&st.selection, &st.doc, self.session.prefs.use_preview_bounds) {
            Some(r) => Some(vectorcraft_doc::OrientedBox::aligned(r)),
            None => self.session.transform_box(&st.selection.objects),
        };
        self.canvas.selection_box = Some((key, b));
        b
    }

    /// The selection's visual bounds (strokes included, effects left out), square to the page:
    /// measured once per revision, as the canvas reads it every frame (a traced photo selects a
    /// group of hundreds of thousands of paths).
    pub fn selection_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.selection_bounds
            && k == key
        {
            return b;
        }
        let b = st.doc.bounds_of(&st.selection.objects, true);
        self.canvas.selection_bounds = Some((key, b));
        b
    }

    /// The art and the artboards together, the span of the canvas's scroll bars: measured once
    /// per revision, as the canvas reads it every frame.
    pub fn content_bounds(&mut self) -> Option<vectorcraft_geom::Rect> {
        let st = self.session.active()?;
        let key = (st.uid, st.revision);
        if let Some((k, b)) = self.canvas.content_bounds
            && k == key
        {
            return b;
        }
        let boards = st.doc.artboards.iter().map(|a| a.rect).reduce(|r, a| r.union(a));
        let b = vectorcraft_geom::union_opt(st.doc.art_bounds(), boards);
        self.canvas.content_bounds = Some((key, b));
        b
    }
}
