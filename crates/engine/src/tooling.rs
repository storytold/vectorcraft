//! Hosting the active tool: pointer/key events → actions → commands.

use serde_json::{Map, Value};
use vectorcraft_doc::Selection;
use vectorcraft_geom::Point;
use vectorcraft_tools::distort::perspective::widget::WidgetPlace;
use vectorcraft_tools::draw2::FEEDBACK;
use vectorcraft_tools::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, settings};

use crate::{EngineError, Prefs, Result, Session};

/// View state the tools need from the frontend.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo {
    pub zoom: f64,
    pub outline: bool,
    pub smart_guides: bool,
    /// View → Show Guides: the ruler guides show (and, unlocked, can be picked).
    pub guides: bool,
    pub snap_to_grid: bool,
    pub show_bbox: bool,
    /// View → Snap to Pixel: drawing and moving land on whole pixels (points at 72 ppi).
    pub snap_to_pixel: bool,
    /// View → Snap to Point: picked points (a transform's reference point) land on anchors.
    pub snap_to_point: bool,
    /// View → Snap to Glyph: drawing snaps to live-text baselines and glyph bounds (needs Smart Guides).
    pub snap_to_glyph: bool,
    /// View → Show Corner Widget.
    pub corner_widgets: bool,
    /// The document window (none headless): screen-fixed widgets sit in it.
    pub screen: Option<vectorcraft_tools::ScreenFrame>,
    /// The view's active artboard (an index): artboard rulers measure from it.
    pub artboard: usize,
}

impl Default for ViewInfo {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            outline: false,
            smart_guides: true,
            guides: true,
            snap_to_grid: false,
            show_bbox: true,
            snap_to_pixel: false,
            snap_to_point: true,
            snap_to_glyph: false,
            corner_widgets: true,
            screen: None,
            artboard: 0,
        }
    }
}

/// The RGB of a colour preference (`#rrggbb`), or `fallback` when it doesn't parse.
fn rgb(hex: &str, fallback: [u8; 3]) -> [u8; 3] {
    vectorcraft_color::Color::from_hex(hex).map_or(fallback, |c| {
        let [r, g, b, _] = c.to_rgba8(1.0);
        [r, g, b]
    })
}

/// Requests from tools that only the frontend can fulfil.
#[derive(Clone, Debug, PartialEq)]
pub enum UiRequest {
    Dialog(String, Value),
    SwitchTool(String),
    /// A message for the status bar: the `warning` of a command the tool ran (Liquify skipping
    /// type under the brush).
    Status(String),
    /// Place a file with these `file.place` params: a click or drag of the place cursor, when
    /// [`Session::place_cursor_requests`] is set.
    Place(Value),
}

impl Session {
    pub fn tool_id(&self) -> &'static str {
        self.tool.id()
    }

    /// How many times switching or closing documents, or replacing the active one (File › Revert),
    /// has started the tool afresh, also inside a `command.batch` or `action.batch`. A frontend
    /// compares it before and after a command.
    pub fn tool_resets(&self) -> u64 {
        self.tool_resets
    }

    /// Whether the tool's options were set ([`Self::set_tool_option`], `tool.setOption`) after
    /// [`Self::tool_resets`] last counted, as a Paste or Cut with the Artboard tool sets them after a
    /// batch's Revert step.
    pub fn tool_set_since_reset(&self) -> bool {
        self.tool_set_since_reset
    }

    /// Choose a tool (the toolbar, its shortcut, MCP), finishing any pending tool work first.
    pub fn select_tool(&mut self, id: &str, view: ViewInfo) -> Result<()> {
        // Choosing a perspective tool shows the document's perspective grid; Hide Grid hides it
        // again, the tool staying chosen.
        if matches!(id, "perspectiveGrid" | "perspectiveSelection")
            && self.active().is_some_and(|d| !vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&d.doc).visible)
        {
            crate::cmd::distortcmds::silent(self, |g| g.visible = true)?;
        }
        self.switch_tool(id, view)
    }

    /// Switch to tool `id` without choosing it afresh (back from a temporary tool: a hidden
    /// perspective grid stays hidden), finishing any pending tool work first.
    pub fn switch_tool(&mut self, id: &str, view: ViewInfo) -> Result<()> {
        self.give_back_tool(view)?;
        if self.tool.id() == id {
            return Ok(());
        }
        // Sampling type keeps its highlighted characters as the target. Capture before the
        // Type tool deactivates (which commits typing and clears its editing state).
        let is_type = |id: &str| matches!(id, "type" | "areaType" | "typeOnPath" | "verticalType" | "verticalAreaType" | "verticalTypeOnPath");
        let text_selection = ((id == "eyedropper" && is_type(self.tool.id())) || (self.tool.id() == "eyedropper" && is_type(id)))
            .then(|| self.tool.options())
            .filter(|o| o.get("editing").is_some_and(|v| !v.is_null()) && o.get("start") != o.get("end"));
        let acts = self.with_tool_cx(view, |t, cx| t.deactivate(cx));
        self.apply_actions(acts)?;
        self.keep_tool_settings();
        self.tool = self.make_tool(id);
        if let Some(selection) = text_selection {
            self.tool.set_option("textSelection", &selection);
        }
        if vectorcraft_tools::catalog::is_selection_tool(id) {
            self.last_selection_tool = Some(self.tool.id());
        }
        Ok(())
    }

    /// The selection tool Cmd lends tool `id` for a drag: the one chosen last; until one is,
    /// Direct Selection to the tools that edit anchors (the Pen) and Selection to the others.
    /// Cmd+Alt lends Group Selection while Direct Selection is active. Cmd still lends the last
    /// selection tool from other tools (Alt on the lent Direct Selection gesture stays copy/subtract).
    /// None for the other selection tools and the view tools.
    fn lent_tool(&self, id: &str, mods: &Mods) -> Option<&'static str> {
        use vectorcraft_tools::catalog::{edits_anchors, is_selection_tool};
        if matches!(id, "hand" | "zoom" | "rotateView") {
            return None;
        }
        if id == "directSelection" && mods.cmd && mods.alt {
            return Some("groupSelection");
        }
        if is_selection_tool(id) {
            return None;
        }
        Some(self.last_selection_tool.unwrap_or(if edits_anchors(id) { "directSelection" } else { "selection" }))
    }

    /// A press with Cmd held (Ctrl elsewhere) and any other tool than a selection tool: the
    /// selection tool [`Self::lent_tool`] takes the gesture, pressing without Cmd (a plain click,
    /// not Select Behind). The tool lent to stays as it was, unaware, until the release.
    fn lend_selection_tool(&mut self, ev: &PointerEvent) -> PointerEvent {
        if ev.kind != PointerKind::Down || !ev.mods.cmd || self.lender.is_some() {
            return *ev;
        }
        let Some(id) = self.lent_tool(self.tool.id(), &ev.mods) else { return *ev };
        // Typing in progress is kept as its own step first: the lent tool's drag previews from the
        // document its interaction started from, which would drop the typing (#1881).
        if crate::cmd::typing_in_progress(self) {
            // Fails only without a document, and then there is no typing to keep.
            let _ = crate::cmd::settle_interaction(self);
        }
        let lent = self.make_tool(id);
        self.lender = Some(std::mem::replace(&mut self.tool, lent));
        PointerEvent { mods: Mods { cmd: false, ..ev.mods }, ..*ev }
    }

    /// The lent selection tool's gesture is over: the tool it was lent to is back as it was (the
    /// Pen goes on drawing its path), less the work the gesture ended as a command would: a
    /// Cmd+click that takes the type being edited out of the selection ends its edit (#3779).
    pub(crate) fn give_back_tool(&mut self, view: ViewInfo) -> Result<()> {
        let Some(lender) = self.lender.take() else { return Ok(()) };
        let acts = self.with_tool_cx(view, |t, cx| t.deactivate(cx));
        let r = self.apply_actions(acts);
        self.keep_tool_settings();
        self.tool = lender;
        self.after_command();
        r.map(drop)
    }

    /// A fresh `id` tool with the options it keeps ([`settings`]): its last values, from this
    /// session or a saved one.
    pub(crate) fn make_tool(&self, id: &str) -> Box<dyn Tool> {
        let mut t = vectorcraft_tools::create(id);
        Self::restore_tool_settings(&self.prefs, t.as_mut());
        t
    }

    /// Give `t` the persistent options stored for it in `prefs`.
    fn restore_tool_settings(prefs: &Prefs, t: &mut dyn Tool) {
        for (store, keys) in settings::stores(t.id()) {
            let Some(saved) = prefs.tool_settings.get(store) else { continue };
            for k in keys {
                if let Some(v) = saved.get(*k) {
                    t.set_option(k, v);
                }
            }
        }
    }

    /// Store the persistent options of `t` in `prefs` (so they are saved with them too).
    fn store_tool_settings(prefs: &mut Prefs, t: &dyn Tool) {
        let opts = t.options();
        for (store, keys) in settings::stores(t.id()) {
            let saved = prefs.tool_settings.entry(store.to_string()).or_default();
            for k in keys {
                if let Some(v) = opts.get(*k) {
                    saved.insert(k.to_string(), v.clone());
                }
            }
        }
    }

    /// Keep the active tool's persistent options for the next time it, or a tool sharing them, is
    /// made.
    pub(crate) fn keep_tool_settings(&mut self) {
        Self::store_tool_settings(&mut self.prefs, self.tool.as_ref());
    }

    pub(crate) fn with_tool_cx<R>(&mut self, view: ViewInfo, f: impl FnOnce(&mut dyn vectorcraft_tools::Tool, &ToolContext) -> R) -> R
    where
        R: Default,
    {
        let Some(st) = self.active.and_then(|i| self.docs.get(i)) else { return R::default() };
        let (unit, stroke_unit) = (self.general_unit(), self.stroke_unit());
        let cx = ToolContext {
            doc: &st.doc,
            ruler_origin: st.doc.ruler_origin_for(view.artboard),
            revision: (st.uid, st.revision),
            selection: &st.selection,
            zoom: view.zoom,
            isolation: st.isolation,
            paint: &self.paint,
            outline: view.outline,
            smart_guides: view.smart_guides,
            guides: view.guides && !self.menu.guides_locked,
            snap_to_grid: view.snap_to_grid,
            show_bbox: view.show_bbox,
            // Snap to Grid and Snap to Pixel are exclusive, as in Illustrator; left both on, the grid
            // wins instead of pixels quietly rounding every drag to whole points (#1816).
            snap_to_pixel: view.snap_to_pixel && !view.snap_to_grid,
            snap_to_point: view.snap_to_point,
            snap_tangent: self.prefs.snap_tangent,
            snap_perpendicular: self.prefs.snap_perpendicular,
            snap_to_glyph: view.snap_to_glyph && view.smart_guides && !view.snap_to_grid,
            corner_widgets: view.corner_widgets,
            fill_active: self.fill_active,
            gradient_stop: self.selected_stop(),
            appearance_item: self.appearance_item(),
            constrain_angle: self.prefs.constrain_angle,
            keyboard_increment: self.prefs.keyboard_increment,
            freeform_point: self.selected_freeform_point(),
            raster_sample: self.prefs.eyedropper.sample_size,
            preview_bounds: self.prefs.use_preview_bounds,
            preview_memo: Some(&self.preview_memo),
            unit,
            stroke_unit,
            paste_plain_text: self.prefs.paste_text_formatting == "plain",
            slices_hidden: self.menu.slices_hidden,
            slices_locked: self.menu.slices_locked,
            auto_add_delete: !self.prefs.disable_auto_add_delete,
            selection_tolerance: self.prefs.selection_tolerance,
            anchor_size: self.prefs.anchor_size,
            path_only: self.prefs.object_selection_by_path_only,
            type_path_only: self.prefs.type_selection_by_path_only,
            double_click_isolate: self.prefs.double_click_to_isolate,
            select_behind: self.prefs.ctrl_click_selects_behind,
            highlight_anchors: self.prefs.highlight_anchors_on_hover,
            snap_tolerance: self.prefs.snap_to_point_tolerance,
            handles_multiple: self.prefs.show_handles_multiple_anchors,
            corner_widget_max_angle: self.prefs.hide_corner_widget_above,
            move_locked_with_artboard: self.prefs.move_locked_with_artboard,
            pen_rubber_band: self.prefs.pen_rubber_band,
            curvature_rubber_band: self.prefs.curvature_rubber_band,
            placeholder_text: self.prefs.placeholder_text,
            smart_guide_color: rgb(&self.prefs.smart_guide_color, vectorcraft_tools::guides::MAGENTA),
            alignment_guides: self.prefs.alignment_guides,
            anchor_path_labels: self.prefs.anchor_path_labels,
            measurement_labels: self.prefs.measurement_labels,
            transform_tools_guides: self.prefs.transform_tools_guides,
            spacing_guides: self.prefs.spacing_guides,
            distance_guides: self.prefs.distance_guides,
            snapping_tolerance: self.prefs.snapping_tolerance,
            construction_angles: if self.prefs.construction_guides {
                vectorcraft_tools::guides::construction_angles(&self.prefs.construction_angles)
            } else {
                &[]
            },
            screen: view.screen,
            plane_widget: self.prefs.perspective_widget.show.then_some(self.prefs.perspective_widget.position),
        };
        let tool = &mut self.tool;
        match crate::guard::catch_panic(|| f(tool.as_mut(), &cx)) {
            Ok(r) => r,
            Err(msg) => {
                // A bug in the tool: start it afresh and drop its half-done drag.
                let id = self.tool.id();
                self.tool = self.make_tool(id);
                let _ = self.cancel_interaction();
                self.tool_panic = Some(EngineError::Internal { cmd: format!("tool `{id}`"), msg });
                R::default()
            }
        }
    }

    /// The error of a tool that panicked in the last [`Self::with_tool_cx`] call, if any.
    fn take_tool_panic(&mut self) -> Result<()> {
        self.tool_panic.take().map_or(Ok(()), Err)
    }

    /// Feed a pointer event to the active tool. Returns requests for the UI.
    pub fn pointer(&mut self, ev: &PointerEvent, view: ViewInfo) -> Result<Vec<UiRequest>> {
        self.last_view = view;
        match ev.kind {
            PointerKind::Down | PointerKind::Drag => self.pointer_held = true,
            PointerKind::Up | PointerKind::Move => self.pointer_held = false,
            PointerKind::DoubleClick => {}
        }
        // The Plane Switching Widget takes its clicks whatever the tool.
        if let Some(r) = self.plane_widget_pointer(ev, view) {
            return r;
        }
        self.leave_draw_inside(ev, view)?;
        let ev = &self.lend_selection_tool(ev);
        let acts = self.with_tool_cx(view, |t, cx| t.pointer(cx, ev));
        let r = self.take_tool_panic().and_then(|()| {
            // A gesture may change options (Alt-drag sizes a Liquify brush): keep them.
            if ev.kind == PointerKind::Up {
                self.keep_tool_settings();
            }
            self.apply_actions(acts)
        });
        if ev.kind == PointerKind::Up {
            self.give_back_tool(view)?;
        }
        r
    }

    /// In Draw Inside, a double-click with a selection tool outside the geometric bounds of the
    /// object new art goes inside, grown by the selection tolerance, or after that object is gone,
    /// sets the drawing mode to Draw Normal (`view.drawMode {mode: "normal"}`, as Shift+D does).
    /// The tool then takes the double-click as in Draw Normal.
    fn leave_draw_inside(&mut self, ev: &PointerEvent, view: ViewInfo) -> Result<()> {
        if ev.kind != PointerKind::DoubleClick
            || self.draw_mode != crate::DrawMode::Inside
            || !vectorcraft_tools::catalog::is_selection_tool(self.tool.id())
        {
            return Ok(());
        }
        let Some(d) = self.active() else { return Ok(()) };
        let tol = self.prefs.selection_tolerance / view.zoom.max(1e-9);
        let inside = self.draw_inside.and_then(|t| d.doc.bounds_of(&[t], false)).is_some_and(|b| b.inflate(tol, tol).contains(ev.pos));
        if !inside {
            self.execute("view.drawMode", &serde_json::json!({ "mode": "normal" }))?;
        }
        Ok(())
    }

    /// A guide dragged out of a ruler, whatever the tool: a vertical one out of the left ruler, a
    /// horizontal one out of the top ruler. `ev` is the pointer in document space (a drag, then the
    /// release), over the canvas or not (`on_canvas`). The guide shows where the pointer is over
    /// the canvas, snapped as a moved guide is, and is made where the button is released over it
    /// (one undo step); released anywhere else, none is. With the Artboard tool it is an artboard
    /// guide of the active artboard.
    pub fn ruler_guide(&mut self, vertical: bool, ev: &PointerEvent, on_canvas: bool, view: ViewInfo) -> Result<()> {
        let g = match self.ruler_guide.take() {
            Some(g) => Some(g),
            None => {
                let boards = self.active().map_or(0, |d| d.doc.artboards.len());
                let active = (self.tool.id() == "artboard").then(|| self.tool.options()["active"].as_u64()).flatten();
                let artboard = active.and_then(|i| usize::try_from(i).ok()).filter(|i| *i < boards);
                self.with_tool_cx(view, |_, cx| Some(vectorcraft_tools::rulerguide::NewGuide::new(cx, vertical, artboard)))
            }
        };
        let Some(mut g) = g else { return Ok(()) };
        let acts = self.with_tool_cx(view, |_, cx| g.pointer(cx, ev, on_canvas));
        if ev.kind != PointerKind::Up {
            self.ruler_guide = Some(g);
        }
        self.apply_actions(acts).map(drop)
    }

    /// Time passed while the pointer button is held (`dt` seconds; see [`Tool::tick`]): the
    /// desktop app ticks every frame of a press, agents with `holdMs` on a pointer event.
    pub fn tool_tick(&mut self, dt: f64, view: ViewInfo) -> Result<Vec<UiRequest>> {
        if !self.tool.wants_ticks() {
            return Ok(vec![]);
        }
        let acts = self.with_tool_cx(view, |t, cx| t.tick(cx, dt));
        self.take_tool_panic()?;
        self.apply_actions(acts)
    }

    /// Does the active tool want [`Session::tool_tick`]s now?
    pub fn tool_wants_ticks(&self) -> bool {
        self.tool.wants_ticks()
    }

    /// Is the active tool waiting for work it runs on another thread ([`Tool::waiting`])?
    pub fn tool_waiting(&self) -> bool {
        self.tool.waiting()
    }

    /// Lets the active tool check on the work [`Session::tool_waiting`] reports, and with `wait`
    /// wait for it to finish ([`Tool::poll`]).
    pub fn tool_poll(&mut self, view: ViewInfo, wait: bool) -> Result<()> {
        self.with_tool_cx(view, |t, cx| t.poll(cx, wait));
        self.take_tool_panic()
    }

    pub fn tool_key(&mut self, key: ToolKey, mods: Mods, view: ViewInfo) -> Result<Vec<UiRequest>> {
        // 1–4 pick the active perspective plane while the grid is shown.
        if let Some(plane) = self.plane_key(key) {
            self.execute("perspective.plane.set", &serde_json::json!({ "plane": plane.id() }))?;
            return Ok(vec![]);
        }
        let acts = self.with_tool_cx(view, |t, cx| t.key(cx, key, mods));
        self.take_tool_panic()?;
        self.apply_actions(acts)
    }

    /// Typed text for the active tool (Type tool).
    pub fn tool_text(&mut self, text: &str, view: ViewInfo) -> Result<Vec<UiRequest>> {
        let acts = self.with_tool_cx(view, |t, cx| t.text_input(cx, text));
        self.take_tool_panic()?;
        self.apply_actions(acts)
    }

    pub fn tool_wants_text(&self) -> bool {
        self.tool.wants_text()
    }

    /// IME marked text for the active tool (see `Tool::ime_preedit`; `active_chars` counts
    /// characters).
    pub fn tool_preedit(&mut self, text: &str, active_chars: Option<std::ops::Range<usize>>, view: ViewInfo) -> Result<Vec<UiRequest>> {
        let acts = self.with_tool_cx(view, |t, cx| t.ime_preedit(cx, text, active_chars));
        self.take_tool_panic()?;
        self.apply_actions(acts)
    }

    /// Is the active tool showing uncommitted IME text?
    pub fn tool_composing(&self) -> bool {
        self.tool.composing()
    }

    /// The caret line (document space) the IME candidate window follows.
    pub fn tool_ime_caret(&mut self, view: ViewInfo) -> Option<(Point, Point)> {
        self.with_tool_cx(view, |t, cx| t.ime_caret(cx))
    }

    /// The pointer stopped hovering for the active tool ([`vectorcraft_tools::Tool::hover_ended`]).
    pub fn tool_hover_ended(&mut self, view: ViewInfo) {
        self.with_tool_cx(view, |t, cx| t.hover_ended(cx));
    }

    /// Whether the active tool's hover depends on what lies under a still pointer
    /// ([`vectorcraft_tools::Tool::wants_rehover`]).
    pub fn tool_wants_rehover(&self) -> bool {
        self.tool.wants_rehover()
    }

    pub fn tool_busy(&self) -> bool {
        self.tool.busy()
    }

    /// Is the pointer button held: was the last pointer event a press or a drag, not a release or a
    /// move? A tool that is busy while the button is not held is waiting between its clicks (the
    /// Dimension tool after its first point).
    pub fn pointer_held(&self) -> bool {
        self.pointer_held
    }

    /// Is the active tool moving, scaling or rotating the selection with a drag?
    pub fn tool_transforming(&self) -> bool {
        self.tool.transforming()
    }

    /// Does the active tool take `key` ahead of the shortcuts bound to it (see `Tool::claims_key`)?
    pub fn tool_claims_key(&mut self, key: ToolKey, view: ViewInfo) -> bool {
        self.plane_key(key).is_some() || self.with_tool_cx(view, |t, cx| t.claims_key(cx, key))
    }

    pub fn overlays(&mut self, view: ViewInfo) -> Vec<Overlay> {
        let mut v = self.with_tool_cx(view, |t, cx| t.overlays(cx));
        // The Type tools draw the ports of selected type themselves.
        if !matches!(self.tool.id(), "type" | "areaType" | "typeOnPath" | "verticalType" | "verticalAreaType" | "verticalTypeOnPath") {
            let mut marks = std::mem::take(&mut self.port_marks);
            v.extend(self.with_tool_cx(view, |_, cx| vectorcraft_tools::text::thread_ports(&mut marks, cx)));
            self.port_marks = marks;
        }
        if let Some(g) = self.ruler_guide.take() {
            v.extend(self.with_tool_cx(view, |_, cx| g.overlays(cx)));
            self.ruler_guide = Some(g);
        }
        let deferred_preview = self.unapplied_interaction_preview();
        if let Some(d) = self.active() {
            let w = self.prefs.perspective_widget;
            let place = w.show.then_some(WidgetPlace { screen: view.screen.as_ref(), corner: w.position });
            v.splice(0..0, vectorcraft_tools::distort::perspective::grid_overlays_in(&d.doc, 1.0 / view.zoom.max(1e-9), place));
        }
        // Global Edit: the objects edited with the selected one are highlighted.
        for (path, color) in crate::cmd::globaledit::highlights(self) {
            v.push(Overlay::Path { path, color, width: 1.0, dashed: false });
        }
        if let Some((cmd, p)) = deferred_preview {
            v.extend(self.deferred_preview_overlays_from_command(&cmd, &p));
        }
        v
    }

    /// The pointer the active tool shows at `p` (General › Use Precise Cursors makes the drawing
    /// tools' a crosshair, [`Cursor::precise`]). With Cmd held, the pointer of the selection tool a
    /// press would borrow ([`Self::lend_selection_tool`]).
    pub fn cursor(&mut self, p: Point, mods: Mods, view: ViewInfo) -> Cursor {
        let lent = if mods.cmd && self.lender.is_none() { self.lent_tool(self.tool.id(), &mods) } else { None };
        let c = match lent {
            Some(id) => {
                let lent = self.make_tool(id);
                let own = std::mem::replace(&mut self.tool, lent);
                let c = self.with_tool_cx(view, |t, cx| t.cursor(cx, p, Mods { cmd: false, ..mods }));
                self.tool = own;
                c
            }
            None => self.with_tool_cx(view, |t, cx| t.cursor(cx, p, mods)),
        };
        if self.prefs.use_precise_cursors { c.precise() } else { c }
    }

    pub fn tool_options(&self) -> Value {
        self.tool.options()
    }
    pub fn set_tool_option(&mut self, key: &str, v: &Value) {
        self.tool.set_option(key, v);
        self.tool_set_since_reset = true;
        self.keep_tool_settings();
    }

    /// The options of tool `id`: the active tool's, or those it would have if chosen now.
    pub fn tool_options_of(&self, id: &str) -> Value {
        if id == self.tool.id() { self.tool.options() } else { self.make_tool(id).options() }
    }

    /// Set options of tool `id` (default: the active tool). Another tool's persistent options are
    /// stored for when it is chosen, and the active tool takes those it shares with it (the
    /// Liquify tools' Global Brush Dimensions). Returns the tool's options.
    pub fn set_tool_options(&mut self, id: Option<&str>, values: &Map<String, Value>) -> Value {
        let active = id.is_none_or(|id| id == self.tool.id());
        self.tool_set_since_reset |= active;
        let mut other = (!active).then(|| self.make_tool(id.unwrap_or_default()));
        let t = other.as_deref_mut().unwrap_or(self.tool.as_mut());
        for (k, v) in values {
            t.set_option(k, v);
        }
        Self::store_tool_settings(&mut self.prefs, t);
        let out = t.options();
        if other.is_some() {
            Self::restore_tool_settings(&self.prefs, self.tool.as_mut());
        }
        out
    }

    /// `tool.setOption {tool?, key?, value?, values?}` (a command of the desktop app and of the
    /// headless host): set one option (`key`, `value`) or several (`values`) of `tool` (default:
    /// the active tool) → its options.
    pub fn set_tool_option_cmd(&mut self, p: &Value) -> std::result::Result<Value, String> {
        let tool = p.get("tool").and_then(Value::as_str);
        if let Some(t) = tool
            && vectorcraft_tools::tool_info(t).is_none()
        {
            return Err(format!("unknown tool `{t}`"));
        }
        let mut values = p.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
        if let Some(k) = p.get("key").and_then(Value::as_str) {
            values.insert(k.to_string(), p.get("value").cloned().unwrap_or(Value::Null));
        }
        // The Place queue is read later, by a click outside any automation scope: its files must
        // be readable now, under the roots of whoever sets it.
        if let Some(entries) = values.get("queue").and_then(|q| q.get("entries")).and_then(Value::as_array) {
            for path in entries.iter().filter_map(|e| e.pointer("/params/path").and_then(Value::as_str)) {
                crate::file_access::check_read(path)?;
            }
        }
        if tool.unwrap_or(self.tool_id()) == "dimension" {
            if let Some(style) = values.get("style") {
                vectorcraft_doc::dimension::DimensionOptions::from_params(style.clone())?;
            }
            if let Some(mode) = values.get("mode")
                && !mode.as_str().is_some_and(|m| ["linear", "angular", "radial", "radius", "diameter"].contains(&m))
            {
                return Err("Unknown dimension mode".into());
            }
        }
        Ok(self.set_tool_options(tool, &values))
    }

    /// Let the active tool finish work a command from outside it ended ([`vectorcraft_tools::Tool::after_command`]):
    /// the Type tool and the Eyedropper clear a character range whose text is no longer selected.
    pub(crate) fn after_command(&mut self) {
        if self.in_tool_actions || (!self.tool.wants_text() && self.tool.id() != "eyedropper") {
            return;
        }
        let acts = self.with_tool_cx(self.last_view, |t, cx| t.after_command(cx));
        // The command itself succeeded: a failure here only leaves the edit as it was.
        if let Err(e) = self.apply_actions(acts) {
            log::warn!("ending the text edit: {e}");
        }
    }

    /// Apply tool actions. Errors from previews are reported but keep the interaction alive.
    pub fn apply_actions(&mut self, acts: Vec<Action>) -> Result<Vec<UiRequest>> {
        let outer = std::mem::replace(&mut self.in_tool_actions, true);
        let r = self.apply_tool_actions(acts);
        self.in_tool_actions = outer;
        r
    }

    fn apply_tool_actions(&mut self, acts: Vec<Action>) -> Result<Vec<UiRequest>> {
        let mut ui = vec![];
        for a in acts {
            match a {
                Action::Begin(label) => self.begin_interaction(&label)?,
                Action::Preview(cmd, p) => {
                    // Performance › Real-time Drawing and Editing (#394): off, remember the latest
                    // params and apply them once on Commit instead of every move.
                    if !self.prefs.real_time_drawing {
                        self.clear_deferred_overlay_cache();
                        let st = self.doc_mut()?;
                        if let Some(it) = &mut st.interaction {
                            it.preview = Some((cmd, p));
                        }
                        continue;
                    }
                    self.clear_deferred_overlay_cache();
                    match self.preview(&cmd, &p) {
                        Ok(v) => {
                            if let Some(w) = v.get("warning").and_then(Value::as_str)
                                && !ui.iter().any(|r| matches!(r, UiRequest::Status(s) if s == w))
                            {
                                ui.push(UiRequest::Status(w.to_string()));
                            }
                        }
                        Err(e) => log::warn!("preview {cmd}: {e}"),
                    }
                }
                // The drag is over: the dabs of a Liquify stroke have no further use.
                Action::Commit => {
                    self.liquify_stroke = None;
                    let deferred = self.unapplied_interaction_preview();
                    if let Some((cmd, p)) = deferred {
                        match self.preview(&cmd, &p) {
                            Ok(v) => {
                                if let Some(w) = v.get("warning").and_then(Value::as_str)
                                    && !ui.iter().any(|r| matches!(r, UiRequest::Status(s) if s == w))
                                {
                                    ui.push(UiRequest::Status(w.to_string()));
                                }
                            }
                            Err(e) => {
                                log::warn!("preview {cmd}: {e}");
                                self.cancel_interaction()?;
                                continue;
                            }
                        }
                    }
                    self.commit_interaction()?
                }
                Action::Cancel => {
                    self.liquify_stroke = None;
                    self.cancel_interaction()?
                }
                // The place cursor's file, placed by the frontend that asked for it.
                Action::Exec(cmd, p) if cmd == "file.place" && self.place_cursor_requests => ui.push(UiRequest::Place(p)),
                Action::Exec(cmd, p) => {
                    // A command's warning (Crop Image's when the image couldn't be decoded) goes to the
                    // status bar too.
                    if let Some(w) = self.execute(&cmd, &p)?.get("warning").and_then(Value::as_str) {
                        ui.push(UiRequest::Status(w.to_string()));
                    }
                }
                Action::Dialog(k, p) => ui.push(UiRequest::Dialog(k, p)),
                Action::SwitchTool(t) => ui.push(UiRequest::SwitchTool(t)),
                Action::Notify(what) => {
                    let view = self.last_view;
                    self.with_tool_cx(view, |t, cx| t.notify(cx, &what));
                }
            }
        }
        Ok(ui)
    }

    /// Canvas feedback while Real-time Drawing is off: run the pending preview command once, take
    /// outlines from the objects it touched, then put the document back at the interaction snapshot.
    fn deferred_preview_overlays_from_command(&mut self, cmd: &str, p: &Value) -> Vec<Overlay> {
        if self.deferred_overlay_cache.as_ref().is_some_and(|(c, params, _)| c == cmd && params == p) {
            return self.deferred_overlay_cache.as_ref().map(|(_, _, o)| o.clone()).unwrap_or_default();
        }
        let (snapshot, selection, revision, paint, interaction_fields) = {
            let Some(st) = self.active() else { return vec![] };
            let Some(it) = st.interaction.as_ref() else { return vec![] };
            if !std::sync::Arc::ptr_eq(&it.doc, &st.doc) {
                return vec![];
            }
            let interaction_fields = (it.preview.clone(), it.copies.clone(), it.perspective_again.clone());
            (st.doc.clone(), st.selection.clone(), st.revision, self.pending_paint.clone(), interaction_fields)
        };
        let overlays = if self.preview(cmd, p).is_ok() {
            self.active().map(|st| outlines_for_selection(&st.doc, &st.selection)).unwrap_or_default()
        } else {
            vec![]
        };
        if let Ok(st) = self.doc_mut() {
            st.doc = snapshot;
            st.selection = selection;
            st.revision = revision;
            if let Some(it) = &mut st.interaction {
                it.preview = interaction_fields.0;
                it.copies = interaction_fields.1;
                it.perspective_again = interaction_fields.2;
            }
        }
        self.pending_paint = paint;
        self.deferred_overlay_cache = Some((cmd.to_string(), p.clone(), overlays.clone()));
        overlays
    }
}

fn outlines_for_selection(doc: &vectorcraft_doc::Document, selection: &Selection) -> Vec<Overlay> {
    selection
        .objects
        .iter()
        .filter_map(|&id| {
            let n = doc.node(id)?;
            vectorcraft_render::drawn_outline(doc, n).map(|(path, _)| Overlay::Path { path, color: FEEDBACK, width: 1.0, dashed: false })
        })
        .collect()
}
