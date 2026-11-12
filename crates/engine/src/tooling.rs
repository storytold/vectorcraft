//! Hosting the active tool: pointer/key events → actions → commands.

use serde_json::{Map, Value};
use vectorcraft_doc::Selection;
use vectorcraft_geom::Point;
use vectorcraft_tools::distort::perspective::widget::WidgetPlace;
use vectorcraft_tools::draw2::FEEDBACK;
use vectorcraft_tools::shape::MAX_CORNER_RADIUS;
use vectorcraft_tools::{Action, Cursor, ImeSurrounding, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, settings};

use crate::{EngineError, Prefs, Result, Session};

/// The tool whose radius is General › Corner Radius.
const ROUNDED_RECTANGLE: &str = "roundedRectangle";

/// View state the tools need from the frontend.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo {
    pub zoom: f64,
    pub outline: bool,
    pub smart_guides: bool,
    /// View → Show Guides: the ruler guides show (and, unlocked, can be picked).
    pub guides: bool,
    pub snap_to_grid: bool,
    /// View → Pixel Preview: while it is on, the tools use no Smart Guides.
    pub pixel_preview: bool,
    pub show_bbox: bool,
    /// View → Snap to Pixel: drawing and moving land on whole pixels (points at 72 ppi).
    pub snap_to_pixel: bool,
    /// View → Snap to Point: picked points (a transform's reference point) land on anchors.
    pub snap_to_point: bool,
    /// View → Snap to Glyph: drawing snaps to live-text glyph bounds, and on a single line of type to
    /// its baseline, x-height, character widths and glyph anchor points (needs Smart Guides).
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
            pixel_preview: false,
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

/// Whether a tool preview of `cmd` with params `p` sets only a symbol's 9-slice guides: a
/// `symbol.options` whose params hold `nineSliceGuides` and nothing besides `name`.
fn nine_slice_guide_preview(cmd: &str, p: &Value) -> bool {
    cmd == "symbol.options"
        && p.as_object().is_some_and(|m| m.contains_key("nineSliceGuides") && m.keys().all(|k| k == "name" || k == "nineSliceGuides"))
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
    /// A press held while the tool computed an edit on another thread was made now, at `pos` in the
    /// document with [`crate::DocState::uid`] `doc`, with tool `tool` active (the selection tool Cmd
    /// lent): the frontend does what it does after a press it hands over itself.
    HeldPress {
        doc: Option<u64>,
        pos: Point,
        tool: &'static str,
    },
}

/// Pointer or key input that came while the active tool computed an edit on another thread
/// ([`Tool::editing`]), made in order once the edit is applied ([`Session::tool_poll`],
/// [`Session::finish_tool_edit`]).
#[derive(Clone, Copy, Debug)]
pub(crate) enum HeldInput {
    Pointer(PointerEvent),
    Key(ToolKey, Mods),
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
        self.finish_tool_edit_later()?;
        // Choosing a perspective tool shows the document's perspective grid; Hide Grid hides it
        // again, the tool staying chosen.
        if matches!(id, "perspectiveGrid" | "perspectiveSelection")
            && self.active().is_some_and(|d| !vectorcraft_tools::distort::perspective::PerspectiveGrid::current(&d.doc).visible)
        {
            crate::cmd::distortcmds::silent(self, |g| g.visible = true)?;
        }
        // When the tool already active, or the tool lent a selection tool for a drag, is chosen again,
        // `select_tool` calls its [`Tool::chosen_again`] (the Blend tool finishes its blend).
        let again = self.lender.as_ref().unwrap_or(&self.tool).id() == id;
        self.switch_tool(id, view)?;
        // `switch_tool` returns early for the tool already active.
        self.select_whole_instance(id);
        if again {
            let acts = self.with_tool_cx(view, |t, cx| t.chosen_again(cx));
            self.apply_actions(acts)?;
        }
        Ok(())
    }

    /// Switch to tool `id` without choosing it afresh (back from a temporary tool: a hidden
    /// perspective grid stays hidden), finishing any pending tool work first.
    pub fn switch_tool(&mut self, id: &str, view: ViewInfo) -> Result<()> {
        // The edit the tool computes on another thread and the input held for it come first.
        self.finish_tool_edit_later()?;
        self.give_back_tool(view)?;
        if self.tool.id() == id {
            return Ok(());
        }
        // Sampling type keeps its highlighted characters as the target, and a whole story that
        // Select All selected. Capture before the Type tool deactivates (which commits typing and
        // clears its editing state).
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
            self.select_whole_instance(id);
        }
        Ok(())
    }

    /// The selection tools other than Direct Selection pick whole objects: with one of them chosen,
    /// an object selected in a symbol instance's art gives way to the instance. The revision
    /// changes with it, so panels that cache by revision show the instance.
    fn select_whole_instance(&mut self, id: &str) {
        if id != "directSelection"
            && vectorcraft_tools::catalog::is_selection_tool(id)
            && let Some(st) = self.active_mut()
            && st.selection.symbol_part.take().is_some()
        {
            st.revision += 1;
        }
    }

    /// The selection tool Cmd lends tool `id` for a drag: the one chosen last; until one is,
    /// Direct Selection to the tools that edit anchors (the Pen) and Selection to the others.
    /// Cmd+Alt lends Group Selection while Direct Selection is active. Cmd still lends the last
    /// selection tool from other tools (Alt on the lent Direct Selection gesture stays copy/subtract).
    /// Cmd alone lends Direct Selection the Selection tool, whose bounding box transforms the
    /// object (#3901; <https://community.adobe.com/t5/illustrator-discussions/handles-disappar-on-shape-with-direct-selection-tool/m-p/14848662>).
    /// Perspective Selection keeps Cmd+Alt drags to copy objects on their perspective plane.
    /// None for the other selection tools and the view tools.
    fn lent_tool(&self, id: &str, mods: &Mods) -> Option<&'static str> {
        use vectorcraft_tools::catalog::{edits_anchors, is_selection_tool};
        if matches!(id, "hand" | "zoom" | "rotateView") {
            return None;
        }
        if id == "directSelection" && mods.cmd {
            return Some(if mods.alt { "groupSelection" } else { "selection" });
        }
        if id == "perspectiveSelection" && mods.cmd && mods.alt {
            return None;
        }
        if is_selection_tool(id) {
            return None;
        }
        Some(self.last_selection_tool.unwrap_or(if edits_anchors(id) { "directSelection" } else { "selection" }))
    }

    /// The selection tool a press with `mods` would borrow from the active tool
    /// ([`Self::lend_selection_tool`]), as [`Self::cursor`] shows it: the canvas also shows what
    /// that tool shows (Direct Selection with Cmd held shows the bounding box).
    pub fn lent_tool_id(&self, mods: &Mods) -> Option<&'static str> {
        if !mods.cmd || self.lender.is_some() {
            return None;
        }
        self.lent_tool(self.tool.id(), mods)
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
        // The Selection and Group Selection tools act on the instance whole.
        self.select_whole_instance(id);
        let lent = self.make_tool(id);
        self.selection_at_lend = self.active().map(|d| (d.uid, d.selection.clone()));
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
        // A gesture that left the selection as it found it (a Shift marquee over nothing) picked
        // nothing: the selection keeps what it was, the Pen's last anchor or an earlier pick.
        let left = self.active().map(|d| (d.uid, d.selection.clone()));
        if left != self.selection_at_lend.take() {
            self.lent_selection = left;
        }
        self.after_command(None);
        r.map(drop)
    }

    /// A fresh `id` tool with the options it keeps ([`settings`]): its last values, from this
    /// session or a saved one.
    pub(crate) fn make_tool(&self, id: &str) -> Box<dyn Tool> {
        let mut t = vectorcraft_tools::create(id);
        Self::restore_tool_settings(&self.prefs, t.as_mut());
        // General › Corner Radius is the Rounded Rectangle tool's radius (#394).
        if id == ROUNDED_RECTANGLE {
            t.set_option("cornerRadius", &Value::from(self.prefs.corner_radius));
        }
        t
    }

    /// Give `t` the persistent options stored for it in `prefs`.
    fn restore_tool_settings(prefs: &Prefs, t: &mut dyn Tool) {
        Self::restore_stores(prefs, t, |_| true);
    }

    /// Give `t` the persistent options stored for it in `prefs`, from the stores `wanted` accepts.
    fn restore_stores(prefs: &Prefs, t: &mut dyn Tool, wanted: impl Fn(&str) -> bool) {
        for (store, keys) in settings::stores(t.id()) {
            if !wanted(store) {
                continue;
            }
            let Some(saved) = prefs.tool_settings.get(store) else { continue };
            for k in keys {
                if let Some(v) = saved.get(*k) {
                    t.set_option(k, v);
                }
            }
        }
        // Alt switches the Pencil, Paintbrush or Blob Brush to Smooth, using Smooth Tool Options' Fidelity.
        if matches!(t.id(), "pencil" | "paintbrush" | "blobBrush")
            && wanted("smooth")
            && let Some(v) = prefs.tool_settings.get("smooth").and_then(|s| s.get("fidelity"))
        {
            t.set_option("smoothFidelity", v);
        }
    }

    /// Store the persistent options of `t` in `prefs` (so they are saved with them too).
    fn store_tool_settings(prefs: &mut Prefs, t: &dyn Tool) {
        let opts = t.options();
        // The radius ↑ and ↓ set while drawing a rounded rectangle (or `tool.setOption`) is the
        // Corner Radius preference from then on, as in the reference app
        // (<https://helpx.adobe.com/illustrator/using/drawing-simple-lines-shapes.html>).
        if t.id() == ROUNDED_RECTANGLE
            && let Some(r) = opts.get("cornerRadius").and_then(Value::as_f64).filter(|r| r.is_finite())
        {
            prefs.corner_radius = r.clamp(0.0, MAX_CORNER_RADIUS);
        }
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
        let smart_guides = view.smart_guides && !view.pixel_preview;
        let cx = ToolContext {
            doc: &st.doc,
            ruler_origin: st.doc.ruler_origin_for(view.artboard),
            revision: (st.uid, st.revision),
            selection: &st.selection,
            zoom: view.zoom,
            isolation: st.isolation,
            paint: &self.paint,
            outline: view.outline,
            smart_guides,
            guides: view.guides && !self.menu.guides_locked,
            snap_to_grid: view.snap_to_grid,
            show_bbox: view.show_bbox,
            // Snap to Grid and Snap to Pixel are exclusive, as in Illustrator; left both on, the grid
            // wins instead of pixels quietly rounding every drag to whole points (#1816).
            snap_to_pixel: view.snap_to_pixel && !view.snap_to_grid,
            snap_to_point: view.snap_to_point,
            snap_tangent: self.prefs.snap_tangent,
            snap_perpendicular: self.prefs.snap_perpendicular,
            snap_to_glyph: view.snap_to_glyph && smart_guides && !view.snap_to_grid,
            glyph_snap: self.prefs.glyph_snap,
            corner_widgets: view.corner_widgets,
            fill_active: self.fill_active,
            gradient_stop: self.selected_stop(),
            appearance_item: self.appearance_item(),
            constrain_angle: self.prefs.constrain_angle,
            keyboard_increment: self.prefs.keyboard_increment,
            freeform_point: self.selected_freeform_point(),
            raster_sample: self.prefs.eyedropper.sample_size,
            preview_bounds: self.prefs.use_preview_bounds,
            glyph_bounds: self.prefs.type_glyph_bounds,
            preview_memo: Some(&self.preview_memo),
            unit,
            stroke_unit,
            paste_plain_text: self.prefs.paste_text_formatting == "plain",
            typed_from_keyboard: self.keyboard_text,
            slices_hidden: self.menu.slices_hidden,
            slices_locked: self.menu.slices_locked,
            auto_add_delete: !self.prefs.disable_auto_add_delete,
            selection_tolerance: self.prefs.selection_tolerance,
            marquee_mode: match self.prefs.marquee_selection.as_str() {
                "enclosed" => vectorcraft_tools::MarqueeMode::Enclosed,
                "directional" => vectorcraft_tools::MarqueeMode::Directional,
                _ => vectorcraft_tools::MarqueeMode::Touching,
            },
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
            typed_lists: self.prefs.typed_lists,
            smart_guide_color: rgb(&self.prefs.smart_guide_color, vectorcraft_tools::guides::MAGENTA),
            alignment_guides: self.prefs.alignment_guides,
            anchor_path_labels: self.prefs.anchor_path_labels,
            measurement_labels: self.prefs.measurement_labels,
            transform_tools_guides: self.prefs.transform_tools_guides,
            spacing_guides: self.prefs.spacing_guides,
            distance_guides: self.prefs.distance_guides,
            snap_to_last_location: self.prefs.snap_to_last_location,
            snapping_tolerance: self.prefs.snapping_tolerance,
            construction_angles: if self.prefs.construction_guides {
                vectorcraft_tools::guides::construction_angles(&self.prefs.construction_angles)
            } else {
                &[]
            },
            screen: view.screen,
            plane_widget: self.prefs.perspective_widget.show.then_some(self.prefs.perspective_widget.position),
            precise_cursors: self.precise_cursors(),
            global_edit: st.global_edit.as_ref().map_or(&[], |g| g.members.as_slice()),
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

    /// The active document (its uid) and its [`crate::History::step`].
    fn history_step(&self) -> Option<(u64, (u64, u64))> {
        self.active().map(|d| (d.uid, d.history.step()))
    }

    /// Tell the active tool when the history moved, or another document became active, since it
    /// last handled a pointer event or was last told ([`vectorcraft_tools::Tool::history_moved`]).
    /// A lent selection tool is not told.
    fn tell_tool_history_moved(&mut self) {
        if self.lender.is_some() {
            return;
        }
        let now = self.history_step();
        if self.tool_step.is_some_and(|seen| Some(seen) != now) {
            self.tool.history_moved();
        }
        self.tool_step = now;
    }

    /// Feed a pointer event to the active tool. Returns requests for the UI. While the tool computes
    /// an edit on another thread, a press and what follows it are held and made once the edit is
    /// applied, in order ([`Session::tool_poll`]); moves of the pointer with no button held go to
    /// the tool at once.
    pub fn pointer(&mut self, ev: &PointerEvent, view: ViewInfo) -> Result<Vec<UiRequest>> {
        if self.script_running() {
            return if ev.kind == PointerKind::Move { Ok(vec![]) } else { Err(EngineError::Busy) };
        }
        let n = self.next_input();
        if ev.kind != PointerKind::Move && (!self.held_input.is_empty() || self.tool_edit_pending()) {
            self.held_input.push_back((n, HeldInput::Pointer(*ev), view));
            return Ok(vec![]);
        }
        let r = self.pointer_now(ev, view);
        self.note_edit_start(n);
        r
    }

    /// The number of the next pointer or key event handed to the session.
    fn next_input(&mut self) -> u64 {
        let n = self.input_count;
        self.input_count = n.wrapping_add(1);
        n
    }

    /// How many pointer and key events have been handed to the session ([`Session::pointer`],
    /// [`Session::tool_key`]); with [`Session::input_applied`], a frontend tells when the events it
    /// handed over have been made.
    pub fn input_count(&self) -> u64 {
        self.input_count
    }

    /// Have the events numbered `from..to` ([`Session::input_count`] before and after handing them
    /// over) been made, and the edit they started on another thread applied?
    pub fn input_applied(&self, from: u64, to: u64) -> bool {
        let ours = |n: &u64| (from..to).contains(n);
        !self.held_input.iter().any(|(n, ..)| ours(n)) && !(self.tool_edit_pending() && self.edit_started_by.as_ref().is_some_and(ours))
    }

    /// After event `n` was made: the event that started the edit being computed, if one is.
    fn note_edit_start(&mut self, n: u64) {
        if !self.tool_edit_pending() {
            self.edit_started_by = None;
        } else if self.edit_started_by.is_none() {
            self.edit_started_by = Some(n);
        }
    }

    /// Is the active tool, or the tool Cmd lent the selection tool's drag away from, computing an
    /// edit on another thread?
    fn tool_edit_pending(&self) -> bool {
        self.tool.editing() || self.lender.as_ref().is_some_and(|t| t.editing())
    }

    /// Makes the input held while the tool computed an edit, in order, until a held gesture starts
    /// another such edit.
    fn replay_held_input(&mut self) -> Result<Vec<UiRequest>> {
        let mut ui = vec![];
        let mut failed = None;
        while !self.tool_edit_pending()
            && let Some((n, input, view)) = self.held_input.pop_front()
        {
            let r = match input {
                HeldInput::Pointer(ev) => self.pointer_now(&ev, view).map(|mut r| {
                    if ev.kind == PointerKind::Down {
                        r.push(UiRequest::HeldPress { doc: self.active().map(|d| d.uid), pos: ev.pos, tool: self.tool.id() });
                    }
                    r
                }),
                HeldInput::Key(key, mods) => self.tool_key_now(key, mods, view),
            };
            self.note_edit_start(n);
            match r {
                Ok(r) => ui.extend(r),
                // The input after it is still made, as it was when each event came on its own.
                Err(e) => failed = failed.or(Some(e)),
            }
        }
        failed.map_or(Ok(ui), Err)
    }

    fn pointer_now(&mut self, ev: &PointerEvent, view: ViewInfo) -> Result<Vec<UiRequest>> {
        self.pointer_event(ev, view, true)
    }

    /// Drop the press in progress (a long touch, a canceled touch, a second finger): nothing it
    /// began is kept. Its interaction is undone, the tool drops the press ([`Tool::drop_press`]:
    /// Escape, as during a drag, for most tools), and a release returns the tool to rest. Of what
    /// the tool asks for along the way, only a cancel and a notify run, so a marquee selects
    /// nothing and a click opens no size dialog. The tool gets back the persistent options it had
    /// when the press began, and none of the press's changes to them are stored. A dropped Type
    /// press restores the prior selection. The freeform gradient point selected once the tool
    /// took the press is selected again. A press held while the tool computes an edit on another
    /// thread has not reached the tool: it is removed from the held input with the pointer events
    /// and tool keys held after it, and the tool is left as it is. A perspective plane key held
    /// after it stays held, since the plane it picks outlasts a dropped press. Pointer events held
    /// after the last held release belong to the press the tool has, and are removed with it.
    pub fn drop_tool_press(&mut self, view: ViewInfo) -> Result<()> {
        let (from, held) = self.held_press();
        let keep: Vec<bool> = (self.held_input.iter().enumerate())
            .map(|(j, (_, input, _))| match input {
                _ if j < from => true,
                HeldInput::Pointer(_) => false,
                HeldInput::Key(key, _) => !held || self.plane_key(*key).is_some(),
            })
            .collect();
        let mut keep = keep.into_iter();
        self.held_input.retain(|_| keep.next().unwrap_or(true));
        if held {
            return Ok(());
        }
        // Nothing to roll back without a document; the tool still drops the press and gets the release.
        let _ = self.cancel_interaction();
        let before = self.press_options.take();
        let type_selection = self.press_type_selection.take();
        let point = self.press_point.take();
        let mut acts = self.with_tool_cx(view, |t, cx| t.drop_press(cx));
        acts.retain(|a| matches!(a, Action::Cancel | Action::Notify(_)));
        let dropped = self.take_tool_panic().and_then(|()| self.apply_actions(acts));
        if let Some((selected, layer)) = type_selection {
            if self.active().is_some_and(|st| st.selection != selected) {
                if let Some(st) = self.active_mut() {
                    st.selection = selected.clone();
                }
                self.select(|_, selection| *selection = selected)?;
            }
            if let Some(st) = self.active_mut()
                && st.active_layer != layer
            {
                st.active_layer = layer;
                st.layer_rows.clear();
            }
        }
        let up = PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), backquote: false, pressure: 1.0, pen: None };
        let r = dropped.and(self.pointer_event(&up, view, false));
        // A drag out of the object previews the point's removal, which deselects it.
        if let Some(p) = point {
            self.freeform_point = p;
        }
        if let Some((id, opts)) = before
            && id == self.tool.id()
        {
            let now = self.tool.options();
            for (k, v) in &opts {
                if now.get(k) != Some(v) {
                    self.tool.set_option(k, v);
                }
            }
        }
        r.map(drop)
    }

    /// Where the input of the press in progress starts in the held input, and whether the press
    /// itself is held: the last held press when no release is held after it; otherwise the input
    /// after the last held release (all of it when none is held), which follows a press the tool
    /// already has.
    fn held_press(&self) -> (usize, bool) {
        let ends = |kind: PointerKind| matches!(kind, PointerKind::Down | PointerKind::Up);
        match self.held_input.iter().rposition(|(_, input, _)| matches!(input, HeldInput::Pointer(ev) if ends(ev.kind))) {
            Some(i) if matches!(self.held_input.get(i), Some((_, HeldInput::Pointer(ev), _)) if ev.kind == PointerKind::Down) => (i, true),
            Some(i) => (i + 1, false),
            None => (0, false),
        }
    }

    /// The options of `t` that [`Self::store_tool_settings`] stores.
    fn persistent_options(t: &dyn Tool) -> Map<String, Value> {
        let opts = t.options();
        let radius = (t.id() == ROUNDED_RECTANGLE).then_some("cornerRadius");
        let keys = settings::stores(t.id()).flat_map(|(_, keys)| keys.iter().copied()).chain(radius);
        keys.filter_map(|k| opts.get(k).map(|v| (k.to_string(), v.clone()))).collect()
    }

    /// [`Self::pointer`]. Without `keep` (a dropped press's release), the tool's options are not
    /// stored and only the tool's `Cancel` and `Notify` actions run.
    fn pointer_event(&mut self, ev: &PointerEvent, view: ViewInfo, keep: bool) -> Result<Vec<UiRequest>> {
        self.last_view = view;
        match ev.kind {
            PointerKind::Down | PointerKind::Drag => self.pointer_held = true,
            PointerKind::Up | PointerKind::Move => self.pointer_held = false,
            PointerKind::DoubleClick => {}
        }
        match ev.kind {
            PointerKind::Down => {
                self.press_options = Some((self.tool.id().to_string(), Self::persistent_options(self.tool.as_ref())));
                self.press_type_selection =
                    if matches!(self.tool.id(), "type" | "areaType" | "typeOnPath" | "verticalType" | "verticalAreaType" | "verticalTypeOnPath") {
                        self.active().map(|st| (st.selection.clone(), st.active_layer))
                    } else {
                        None
                    };
            }
            PointerKind::Up => {
                self.press_options = None;
                self.press_type_selection = None;
            }
            _ => {}
        }
        // The Plane Switching Widget takes its clicks whatever the tool.
        if let Some(r) = self.plane_widget_pointer(ev, view) {
            return r;
        }
        self.leave_draw_inside(ev, view)?;
        self.tell_tool_history_moved();
        let ev = &self.lend_selection_tool(ev);
        let lent = self.lender.is_some();
        let mut acts = self.with_tool_cx(view, |t, cx| t.pointer(cx, ev));
        if !keep {
            acts.retain(|a| matches!(a, Action::Cancel | Action::Notify(_)));
        }
        let r = self.take_tool_panic().and_then(|()| {
            // A gesture may change options (Alt-drag sizes a Liquify brush): keep them.
            if keep && ev.kind == PointerKind::Up {
                self.keep_tool_settings();
            }
            self.apply_actions(acts)
        });
        // The tool's own edits do not move the history for it. A selection tool that Cmd lends
        // edits for the lender, which hears of the edit at its next event.
        if !lent {
            self.tool_step = self.history_step();
        }
        match ev.kind {
            PointerKind::Down => self.press_point = Some(self.freeform_point),
            PointerKind::Up => self.press_point = None,
            _ => {}
        }
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

    /// The artboard a guide made from a ruler belongs to: with the Artboard tool, its active
    /// artboard (an artboard guide); with any other tool, none (a canvas guide).
    pub fn ruler_guide_artboard(&self) -> Option<usize> {
        let boards = self.active().map_or(0, |d| d.doc.artboards.len());
        let active = (self.tool.id() == "artboard").then(|| self.tool.options()["active"].as_u64()).flatten();
        active.and_then(|i| usize::try_from(i).ok()).filter(|i| *i < boards)
    }

    /// A guide dragged out of a ruler, whatever the tool: a vertical one out of the left ruler, a
    /// horizontal one out of the top ruler. `ev` is the pointer in document space (a drag, then the
    /// release), over the canvas or not (`on_canvas`). The guide shows where the pointer is over
    /// the canvas, snapped as a moved guide is, and is made where the button is released over it
    /// (one undo step); released anywhere else, none is. With the Artboard tool it is an artboard
    /// guide of the active artboard. With Preferences › Guides & Grid › Artboard Guides with
    /// Cmd/Ctrl + Ruler Drag on, `mods.cmd` on the drag's first event swaps the two: with another
    /// tool the guide is one of the view's active artboard (`view.artboard`), shown and made only
    /// where it lies within that artboard, and with the Artboard tool it is a canvas guide.
    pub fn ruler_guide(&mut self, vertical: bool, ev: &PointerEvent, on_canvas: bool, view: ViewInfo) -> Result<()> {
        use vectorcraft_tools::rulerguide::NewGuide;
        let g = match self.ruler_guide.take() {
            Some(g) => Some(g),
            None => {
                let artboard_tool = self.tool.id() == "artboard";
                let swap = self.prefs.cmd_ruler_drag_artboard_guides && ev.mods.cmd;
                let artboard = self.ruler_guide_artboard();
                let view_board = view.artboard;
                self.with_tool_cx(view, |_, cx| {
                    Some(match (swap, artboard_tool) {
                        (false, _) => NewGuide::new(cx, vertical, artboard),
                        (true, true) => NewGuide::new(cx, vertical, None),
                        (true, false) => NewGuide::within(cx, vertical, view_board),
                    })
                })
            }
        };
        let Some(mut g) = g else { return Ok(()) };
        let acts = self.with_tool_cx(view, |_, cx| g.pointer(cx, ev, on_canvas));
        if ev.kind != PointerKind::Up {
            self.ruler_guide = Some(g);
        }
        self.apply_actions(acts).map(drop)
    }

    /// The rulers' origin dragged out of their corner, whatever the tool, with the pointer at `p` in
    /// document space (none off the canvas): where it goes, snapped as a drawn point is
    /// ([`vectorcraft_tools::rulerguide::OriginDrag`]), the targets gathered on the drag's first
    /// call. It changes nothing: the caller sets the origin where the button is released
    /// (`document.setRulerOrigin`) and ends the drag ([`Session::end_ruler_origin_drag`]).
    pub fn ruler_origin_drag(&mut self, p: Option<Point>, view: ViewInfo) -> Option<Point> {
        let drag = match self.ruler_origin.take() {
            Some(d) => Some(d),
            None => self.with_tool_cx(view, |_, cx| Some(vectorcraft_tools::rulerguide::OriginDrag::new(cx))),
        };
        let Some(mut drag) = drag else { return p };
        let at = self.with_tool_cx(view, |_, cx| drag.pointer(cx, p));
        self.ruler_origin = Some(drag);
        at
    }

    /// The drag of the rulers' origin ended, released or cancelled: its guides go.
    pub fn end_ruler_origin_drag(&mut self) {
        self.ruler_origin = None;
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

    /// Does the active tool build a stroke from every position the pointer passed
    /// ([`vectorcraft_tools::Tool::wants_samples`])?
    pub fn tool_wants_samples(&self) -> bool {
        self.tool.wants_samples()
    }

    /// Is the active tool waiting for work it runs on another thread ([`Tool::waiting`]), is input
    /// held until an edit it computes is applied, or are requests for the UI left from making it?
    pub fn tool_waiting(&self) -> bool {
        self.tool.waiting() || !self.held_input.is_empty() || !self.held_ui.is_empty()
    }

    /// Lets the active tool check on the work [`Session::tool_waiting`] reports, and with `wait`
    /// wait for it to finish ([`Tool::poll`]); an edit the work made is applied, and then the input
    /// held for it is made. Returns requests for the UI, those left from making held input where
    /// nothing could take them first ([`Session::finish_tool_edit_later`]).
    pub fn tool_poll(&mut self, view: ViewInfo, wait: bool) -> Result<Vec<UiRequest>> {
        if self.settling_tool_edit {
            return Ok(vec![]);
        }
        self.settling_tool_edit = true;
        let r = self.tool_poll_now(view, wait);
        self.settling_tool_edit = false;
        // On an error the requests left wait for the next poll.
        let r = r?;
        let mut ui = std::mem::take(&mut self.held_ui);
        ui.extend(r);
        Ok(ui)
    }

    fn tool_poll_now(&mut self, view: ViewInfo, wait: bool) -> Result<Vec<UiRequest>> {
        let acts = self.with_tool_cx(view, |t, cx| t.poll(cx, wait));
        self.take_tool_panic()?;
        let applied = !acts.is_empty();
        let mut ui = self.apply_actions(acts)?;
        if !self.tool_edit_pending() {
            self.edit_started_by = None;
            // The input held for an edit applied now is made at the next poll, so what waits for the
            // edit alone (a `ui.pointer` reply) sees the document as the edit left it.
            if !applied {
                ui.extend(self.replay_held_input()?);
            }
        }
        Ok(ui)
    }

    /// Is the active tool computing an edit on another thread ([`Tool::editing`]), or is input held
    /// until it is applied?
    pub fn tool_editing(&self) -> bool {
        self.tool_edit_pending() || !self.held_input.is_empty()
    }

    /// Waits for the edit the active tool computes on another thread and applies it
    /// ([`Tool::finish_edit`]), then makes the input held for it, waiting in turn for an edit that
    /// starts, so what runs next sees the document as the gestures left it. Commands, saves, option
    /// changes, interactions, undo scopes and tool or document switches call it first. Returns
    /// requests for the UI, with those left from earlier ([`Session::finish_tool_edit_later`]); on
    /// an error, the next [`Session::tool_poll`] returns them.
    pub fn finish_tool_edit(&mut self) -> Result<Vec<UiRequest>> {
        if self.settling_tool_edit {
            return Ok(vec![]);
        }
        let mut ui = std::mem::take(&mut self.held_ui);
        if self.tool_editing() {
            self.settling_tool_edit = true;
            let r = self.finish_tool_edit_now();
            self.settling_tool_edit = false;
            match r {
                Ok(r) => ui.extend(r),
                Err(e) => {
                    ui.append(&mut self.held_ui);
                    self.held_ui = ui;
                    return Err(e);
                }
            }
        }
        Ok(ui)
    }

    /// [`Session::finish_tool_edit`] for a caller that cannot hand requests to the UI (a command, a
    /// tool switch): the next [`Session::tool_poll`] returns them, and [`Session::tool_waiting`]
    /// holds until then.
    pub(crate) fn finish_tool_edit_later(&mut self) -> Result<()> {
        let ui = self.finish_tool_edit()?;
        self.held_ui.extend(ui);
        Ok(())
    }

    fn finish_tool_edit_now(&mut self) -> Result<Vec<UiRequest>> {
        let mut ui = vec![];
        let mut failed = None;
        loop {
            match self.finish_pending_edit() {
                Ok(r) => ui.extend(r),
                Err(e) => failed = failed.or(Some(e)),
            }
            // A tool that cannot finish its edit keeps the input held.
            if self.tool_edit_pending() {
                break;
            }
            self.edit_started_by = None;
            if self.held_input.is_empty() {
                break;
            }
            match self.replay_held_input() {
                Ok(r) => ui.extend(r),
                Err(e) => failed = failed.or(Some(e)),
            }
        }
        match failed {
            None => Ok(ui),
            Some(e) => {
                self.held_ui.extend(ui);
                Err(e)
            }
        }
    }

    /// Waits for the edit being computed and applies it: the active tool's, or that of the tool
    /// Cmd lent the selection tool's drag away from, which is active meanwhile.
    fn finish_pending_edit(&mut self) -> Result<Vec<UiRequest>> {
        if self.tool.editing() {
            let acts = self.with_tool_cx(self.last_view, |t, cx| t.finish_edit(cx));
            self.take_tool_panic()?;
            return self.apply_actions(acts);
        }
        let Some(lender) = self.lender.take_if(|t| t.editing()) else { return Ok(vec![]) };
        let lent = std::mem::replace(&mut self.tool, lender);
        let acts = self.with_tool_cx(self.last_view, |t, cx| t.finish_edit(cx));
        let r = self.take_tool_panic().and_then(|()| self.apply_actions(acts));
        self.lender = Some(std::mem::replace(&mut self.tool, lent));
        r
    }

    /// A key for the active tool. While the tool computes an edit on another thread, the key is held
    /// and made once the edit is applied, after the input held before it.
    pub fn tool_key(&mut self, key: ToolKey, mods: Mods, view: ViewInfo) -> Result<Vec<UiRequest>> {
        if self.script_running() {
            return Err(EngineError::Busy);
        }
        let n = self.next_input();
        if self.tool_editing() {
            self.held_input.push_back((n, HeldInput::Key(key, mods), view));
            return Ok(vec![]);
        }
        let r = self.tool_key_now(key, mods, view);
        self.note_edit_start(n);
        r
    }

    fn tool_key_now(&mut self, key: ToolKey, mods: Mods, view: ViewInfo) -> Result<Vec<UiRequest>> {
        // 1–4 pick the active perspective plane while the grid is shown.
        if let Some(plane) = self.plane_key(key) {
            self.execute("perspective.plane.set", &serde_json::json!({ "plane": plane.id() }))?;
            if self.tool_id() == "perspectiveSelection" {
                let acts = self.with_tool_cx(view, |t, cx| t.key(cx, key, mods));
                self.take_tool_panic()?;
                return self.apply_actions(acts);
            }
            return Ok(vec![]);
        }
        let acts = self.with_tool_cx(view, |t, cx| t.key(cx, key, mods));
        self.take_tool_panic()?;
        // A key that cancels a gesture may put back options the gesture changed (Escape on an
        // arc, spiral or grid drag): keep them.
        if acts.contains(&Action::Cancel) {
            self.keep_tool_settings();
        }
        self.apply_actions(acts)
    }

    /// Typed text for the active tool (Type tool).
    pub fn tool_text(&mut self, text: &str, view: ViewInfo) -> Result<Vec<UiRequest>> {
        if self.script_running() {
            return Err(EngineError::Busy);
        }
        let acts = self.with_tool_cx(view, |t, cx| t.text_input(cx, text));
        self.take_tool_panic()?;
        self.apply_actions(acts)
    }

    /// Text typed on the keyboard (a key or an IME commit) for the active tool: as
    /// [`Session::tool_text`], and Typographer's Quotes may also set a lone typed `”` as an opening
    /// quote ([`vectorcraft_tools::ToolContext::typed_from_keyboard`]). The Glyphs panel and pastes
    /// use [`Session::tool_text`], so their quotes stay as inserted.
    pub fn tool_typed_text(&mut self, text: &str, view: ViewInfo) -> Result<Vec<UiRequest>> {
        self.keyboard_text = true;
        let r = self.tool_text(text, view);
        self.keyboard_text = false;
        r
    }

    /// Would [`Session::tool_text`] insert `text` unchanged (no control-character filtering,
    /// newline normalization or document formatting such as Typographer's Quotes)? An IME commit
    /// the application rewrites is an application edit, not an accepted IME insertion.
    pub fn tool_text_is_verbatim(&mut self, text: &str, view: ViewInfo) -> bool {
        self.with_tool_cx(view, |t, cx| t.text_input_is_verbatim(cx, text))
    }

    /// [`Session::tool_text_is_verbatim`] for text typed on the keyboard, as
    /// [`Session::tool_typed_text`] would insert it: a lone typed `”` that Typographer's Quotes
    /// turns into an opening quote is not verbatim.
    pub fn tool_typed_text_is_verbatim(&mut self, text: &str, view: ViewInfo) -> bool {
        self.keyboard_text = true;
        let v = self.tool_text_is_verbatim(text, view);
        self.keyboard_text = false;
        v
    }

    /// Does the active tool's open typing session still match the document? An IME commit after
    /// an external edit starts a fresh session (an application-side reset), so its receipt must
    /// not be accepted as an unchanged IME insertion.
    pub fn tool_ime_session_matches(&mut self, view: ViewInfo) -> bool {
        self.with_tool_cx(view, |t, cx| t.ime_session_matches_document(cx))
    }

    /// Whether the active tool currently owns keyboard text input, as the Type tool does while editing.
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

    /// An IME replaces text around its composition before committing: delete `before`/`after`
    /// units around the selection or pre-edit. The selection/pre-edit itself is preserved (the
    /// following commit replaces it), and the commit shares the typing session, so the replacement
    /// is one undo step.
    pub fn tool_ime_delete_surrounding(
        &mut self,
        before: usize,
        after: usize,
        unit: vectorcraft_tools::SurroundingUnit,
        view: ViewInfo,
    ) -> Result<Vec<UiRequest>> {
        let acts = self.with_tool_cx(view, |t, cx| t.ime_delete_surrounding(cx, before, after, unit));
        self.take_tool_panic()?;
        self.apply_actions(acts)
    }

    /// The active tool's stable IME owner `(document uid, target id, replacement epoch)`: the
    /// document uid changes on document replacement, the epoch when the same target is started
    /// afresh. Ordinary edits and caret moves leave both unchanged.
    pub fn tool_ime_target(&mut self, view: ViewInfo) -> Option<(u64, u64, u64)> {
        self.with_tool_cx(view, |t, cx| t.ime_target(cx))
    }

    /// The committed logical text around the IME cursor (UTF-8 byte offsets), with any pre-edit
    /// removed and at most `max_bytes` bytes copied: the app's outgoing surrounding-text producer
    /// (egui E21b) reads it. An explicit [`ImeSurrounding::Unavailable`] carries the narrow reason
    /// (no editor, invalid offsets, or a selection wider than the limit).
    pub fn tool_ime_surrounding(&mut self, max_bytes: usize, view: ViewInfo) -> ImeSurrounding {
        self.with_tool_cx(view, |t, cx| t.ime_surrounding(cx, max_bytes))
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

    /// Is the active tool resizing the selection by a bounding-box handle?
    pub fn tool_resizing(&self) -> bool {
        self.tool.resizing()
    }

    /// Does the canvas draw the bounding box around a shape being drawn: the active tool is drawing
    /// a new shape whose box it shows once drawn (see `Tool::drawing_boxed`), and General › Show
    /// Bounding Box While Moving and Drawing is on?
    pub fn draws_new_shape_box(&self) -> bool {
        self.prefs.bounding_box_while_moving_and_drawing && self.tool.drawing_boxed()
    }

    /// Has the interaction no preview applied to the document: none has run yet, the last one failed,
    /// or Real-time Drawing is off and the document is still the interaction's snapshot?
    pub fn interaction_unapplied(&self) -> bool {
        self.active().is_some_and(|d| d.interaction.as_ref().is_some_and(|it| it.preview.is_none() || std::sync::Arc::ptr_eq(&it.doc, &d.doc)))
    }

    /// The bounding box of the selection a move drag is carrying (an Alt-drag copy included), where
    /// the drag has carried it: the box of the selection at the drag's start, measured once per
    /// drag, offset by the move of the latest preview. With General › Show Bounding Box While
    /// Moving and Drawing on, for every tool that moves the selection; off, only for the Selection
    /// tool with Real-time Drawing off, where the document moves on release. `None` for any other
    /// drag, and before a preview has moved the selection.
    pub fn move_drag_box(&self) -> Option<vectorcraft_doc::OrientedBox> {
        if !self.tool.moving() {
            return None;
        }
        let st = self.active()?;
        let it = st.interaction.as_ref()?;
        let deferred = std::sync::Arc::ptr_eq(&it.doc, &st.doc);
        if !self.prefs.bounding_box_while_moving_and_drawing && !(self.tool.id() == "selection" && deferred) {
            return None;
        }
        if !matches!(it.label.as_str(), "Move" | "Copy") {
            return None;
        }
        let (cmd, p) = it.preview.as_ref()?;
        if cmd != "object.transform" || p.get("objects").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let [a, b, c, d, e, f] = crate::cmd::matrix_param(p, "matrix")?.as_coeffs();
        if !(a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0) {
            return None;
        }
        let measure = self.bounds_measure();
        let start =
            (*it.move_box.get_or_init(|| vectorcraft_render::effects::preview::oriented_bounds_with(&it.doc, &it.selection.objects, measure)))?;
        let moved = start.to_local(Point::new(e, f)).to_vec2();
        Some(vectorcraft_doc::OrientedBox { rect: start.rect + moved, ..start })
    }

    /// Does the active tool take `key` ahead of the shortcuts bound to it (see `Tool::claims_key`)?
    pub fn tool_claims_key(&mut self, key: ToolKey, view: ViewInfo) -> bool {
        self.plane_key(key).is_some() || self.with_tool_cx(view, |t, cx| t.claims_key(cx, key))
    }

    /// Has the active tool started something that Escape ends while no button is pressed (see
    /// `Tool::ends_on_escape`)?
    pub fn tool_ends_on_escape(&mut self, view: ViewInfo) -> bool {
        self.with_tool_cx(view, |t, cx| t.ends_on_escape(cx))
    }

    pub fn overlays(&mut self, view: ViewInfo) -> Vec<Overlay> {
        // The Blend tool's first pick is not marked after an edit that dropped it.
        self.tell_tool_history_moved();
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
        if let Some(d) = &self.ruler_origin {
            v.extend(d.overlays());
        }
        let deferred_preview = self.unapplied_interaction_preview();
        if let Some(d) = self.active() {
            let w = self.prefs.perspective_widget;
            let place = w.show.then_some(WidgetPlace { screen: view.screen.as_ref(), corner: w.position });
            v.splice(0..0, vectorcraft_tools::distort::perspective::grid_overlays_in(&d.doc, 1.0 / view.zoom.max(1e-9), place));
        }
        // Global Edit: the objects edited with the selected one are highlighted; before it starts,
        // with the Selection tool, the objects it would take.
        let ge_on = self.active().is_some_and(|d| d.global_edit.is_some());
        let shown = if ge_on || self.tool.id() == "selection" { crate::cmd::globaledit::highlights(self) } else { vec![] };
        for (path, color) in shown {
            v.push(Overlay::Path { path, color, width: 1.0, dashed: false });
        }
        if let Some((cmd, p)) = deferred_preview {
            v.extend(self.deferred_preview_overlays_from_command(&cmd, &p));
        }
        v
    }

    /// The pointer the active tool shows at `p` (General › Use Precise Cursors, or Caps Lock while
    /// it is on, makes the drawing tools' a crosshair, [`Cursor::precise`]). With Cmd held, the pointer of the selection tool a
    /// press would borrow ([`Self::lend_selection_tool`]).
    pub fn cursor(&mut self, p: Point, mods: Mods, view: ViewInfo) -> Cursor {
        // While an edit is computed, a Cmd press is held for the active tool, which shows it is busy.
        let lent = if mods.cmd && self.lender.is_none() && !self.tool_editing() { self.lent_tool(self.tool.id(), &mods) } else { None };
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
        if self.precise_cursors() { c.precise() } else { c }
    }

    /// General › Use Precise Cursors is on, or Caps Lock is.
    fn precise_cursors(&self) -> bool {
        self.prefs.use_precise_cursors || self.caps_lock
    }

    pub fn tool_options(&self) -> Value {
        self.tool.options()
    }
    pub fn set_tool_option(&mut self, key: &str, v: &Value) {
        // The active tool's edit being computed is applied with the options it was started with. A
        // failure to apply it was the edit's own, and the option is set all the same.
        let _ = self.finish_tool_edit_later();
        if key == "queue" && self.tool.id() == "place" {
            self.note_place_queue_roots();
        }
        self.tool.set_option(key, v);
        self.tool_set_since_reset = true;
        self.keep_tool_settings();
    }

    /// Keep the automation roots in force now as those the place cursor's queue, being loaded,
    /// is placed under ([`Session::place_queue_roots`]).
    fn note_place_queue_roots(&mut self) {
        self.place_queue_roots = crate::file_access::current();
    }

    /// The automation roots the place cursor's files are placed under: those in force when it
    /// was loaded. A frontend that places them itself ([`UiRequest::Place`]) reads them within
    /// these roots ([`crate::file_access::confine`]).
    pub fn place_queue_roots(&self) -> Option<std::sync::Arc<crate::file_access::AutomationRoots>> {
        self.place_queue_roots.clone()
    }

    /// The options of tool `id`: the active tool's, or those it would have if chosen now.
    pub fn tool_options_of(&self, id: &str) -> Value {
        if id == self.tool.id() { self.tool.options() } else { self.make_tool(id).options() }
    }

    /// Set options of tool `id` (default: the active tool). Another tool's persistent options are
    /// stored for when it is chosen, and the active tool takes those it shares with it (the
    /// Liquify tools' Global Brush Dimensions, the Smooth tool's Fidelity for Pencil, Paintbrush
    /// and Blob Brush Alt smoothing). Returns the tool's options.
    pub fn set_tool_options(&mut self, id: Option<&str>, values: &Map<String, Value>) -> Value {
        if values.contains_key("queue") && id.unwrap_or(self.tool.id()) == "place" {
            self.note_place_queue_roots();
        }
        let active = id.is_none_or(|id| id == self.tool.id());
        self.tool_set_since_reset |= active;
        let mut other = (!active).then(|| self.make_tool(id.unwrap_or_default()));
        let t = other.as_deref_mut().unwrap_or(self.tool.as_mut());
        for (k, v) in values {
            t.set_option(k, v);
        }
        Self::store_tool_settings(&mut self.prefs, t);
        let out = t.options();
        if let Some(other) = other {
            // The tool that lent a selection tool for a Cmd-drag is kept aside, not made again when
            // it is back: when it is the tool set, it takes the writes too (the Rounded Rectangle's
            // radius stays the Corner Radius preference it just set) and what it shares, like the
            // active tool.
            if let Some(lender) = self.lender.as_deref_mut() {
                if lender.id() == other.id() {
                    for (k, v) in values {
                        lender.set_option(k, v);
                    }
                }
                Self::restore_tool_settings(&self.prefs, lender);
            }
            // Only the stores it shares with the tool set: the active tool's own options stay as they
            // are, also in the middle of a drag that Escape can cancel.
            let set = other.id();
            Self::restore_stores(&self.prefs, self.tool.as_mut(), |store| settings::stores(set).any(|(s, _)| s == store));
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
        // The active tool's merge being computed is applied with the options it was started with.
        if tool.is_none_or(|t| t == self.tool.id()) {
            self.finish_tool_edit_later().map_err(|e| e.to_string())?;
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
    /// After `edit.undo` or `edit.redo` (`cmd`), the tool is told first, so the Type tool puts the
    /// caret where the step changed the text.
    pub(crate) fn after_command(&mut self, cmd: Option<&str>) {
        if self.in_tool_actions || (!self.tool.wants_text() && self.tool.id() != "eyedropper") {
            return;
        }
        if let Some(cmd @ ("edit.undo" | "edit.redo")) = cmd {
            self.notify_text_tool(cmd);
        }
        let acts = self.with_tool_cx(self.last_view, |t, cx| t.after_command(cx));
        // The command itself succeeded: a failure here only leaves the edit as it was.
        if let Err(e) = self.apply_actions(acts) {
            log::warn!("ending the text edit: {e}");
        }
    }

    /// End the Type tool's text editing before a command saves the art of an editing layer (pattern
    /// editing's Done, Stop Editing Opacity Mask): point type its click placed that is still empty is
    /// discarded first, so it is not saved into the pattern tile or the mask. A selection tool that a
    /// Cmd press lent is given back first, so the Type tool behind it ends its edit too; a drag of the
    /// lent tool is kept as its own step, as its release would keep it.
    pub(crate) fn end_text_edit(&mut self) {
        if self.in_tool_actions {
            return;
        }
        if self.lender.is_some() {
            if let Err(e) = self.apply_actions(vec![Action::Commit]) {
                log::warn!("keeping the lent tool's drag before ending the text edit: {e}");
            }
            if let Err(e) = self.give_back_tool(self.last_view) {
                log::warn!("giving back the lent tool before ending the text edit: {e}");
            }
        }
        if !self.tool.wants_text() {
            return;
        }
        let acts = self.with_tool_cx(self.last_view, |t, cx| t.deactivate(cx));
        if let Err(e) = self.apply_actions(acts) {
            log::warn!("ending the text edit: {e}");
        }
    }

    /// Tell the active tool that `edit.undo` or `edit.redo` ran, on its own or as a step of
    /// `command.batch`, so the Type tool puts the caret where the step changed the text; or that a
    /// `command.batch` began, ended or was rolled back (`batch.begin`, `batch.end`,
    /// `batch.rollBack`), so the Type tool puts back what its Undo and Redo steps changed when it fails.
    pub(crate) fn notify_text_tool(&mut self, what: &str) {
        if self.in_tool_actions || (!self.tool.wants_text() && self.tool.id() != "eyedropper") {
            return;
        }
        self.with_tool_cx(self.last_view, |t, cx| t.notify(cx, what));
    }

    /// Apply tool actions. Errors from previews are reported but keep the interaction alive.
    pub fn apply_actions(&mut self, acts: Vec<Action>) -> Result<Vec<UiRequest>> {
        let outer = std::mem::replace(&mut self.in_tool_actions, true);
        let r = self.apply_tool_actions(acts);
        self.in_tool_actions = outer;
        if !outer && self.depth == 0 {
            self.trim_histories();
        }
        r
    }

    fn apply_tool_actions(&mut self, acts: Vec<Action>) -> Result<Vec<UiRequest>> {
        let mut ui = vec![];
        for a in acts {
            match a {
                Action::Begin(label) => self.begin_interaction(&label)?,
                Action::Preview(cmd, p) => {
                    // Performance › Real-time Drawing and Editing (#394): off, remember the latest
                    // params and apply them once on Commit instead of every move. A ruler guide
                    // being dragged still shows: the guide is all there is to see (#4646). So does a
                    // symbol's 9-slice guide being dragged.
                    if !self.prefs.real_time_drawing && !matches!(cmd.as_str(), "guide.add" | "guide.move") && !nine_slice_guide_preview(&cmd, &p) {
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
                // The place cursor's file, read within the roots of whoever loaded the cursor.
                Action::Exec(cmd, p) if cmd == "file.place" => {
                    let roots = self.place_queue_roots.clone();
                    if let Some(w) = crate::file_access::confine(roots.as_ref(), || self.execute(&cmd, &p))?.get("warning").and_then(Value::as_str) {
                        ui.push(UiRequest::Status(w.to_string()));
                    }
                }
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

    /// With Real-time Drawing off, the selection's transform box ([`Session::transform_box`]) as the
    /// pending preview of a handle resize or of a shape being drawn leaves it, measured once per
    /// preview step, with the layer color of the first object it measures; `None` when no such
    /// preview is pending, or when the preview fails.
    pub fn deferred_transform_box(&mut self) -> Option<(vectorcraft_doc::OrientedBox, [u8; 3])> {
        if !self.measures_deferred_box() {
            return None;
        }
        let (cmd, p) = self.unapplied_interaction_preview()?;
        self.deferred_feedback(&cmd, &p)?.transform_box
    }

    /// Does the canvas draw the bounding box of what the pending preview leaves: during a handle
    /// resize, or while a shape is drawn by a tool that shows its box ([`Session::draws_new_shape_box`])?
    fn measures_deferred_box(&self) -> bool {
        self.tool.resizing() || self.draws_new_shape_box()
    }

    fn deferred_preview_overlays_from_command(&mut self, cmd: &str, p: &Value) -> Vec<Overlay> {
        self.deferred_feedback(cmd, p).map(|f| f.overlays.clone()).unwrap_or_default()
    }

    /// Canvas feedback while Real-time Drawing is off: run the pending preview command once, take
    /// outlines from the objects it touched (and, during a handle resize or while a shape is drawn,
    /// their transform box), then put the document back at the interaction snapshot.
    fn deferred_feedback(&mut self, cmd: &str, p: &Value) -> Option<&DeferredFeedback> {
        if !self.deferred_overlay_cache.as_ref().is_some_and(|f| f.cmd == cmd && f.params == *p) {
            let feedback = self.measure_deferred_preview(cmd, p)?;
            self.deferred_overlay_cache = Some(feedback);
        }
        self.deferred_overlay_cache.as_ref()
    }

    fn measure_deferred_preview(&mut self, cmd: &str, p: &Value) -> Option<DeferredFeedback> {
        let (snapshot, selection, revision, paint, interaction_fields) = {
            let st = self.active()?;
            let it = st.interaction.as_ref()?;
            if !std::sync::Arc::ptr_eq(&it.doc, &st.doc) {
                return None;
            }
            let interaction_fields = (it.preview.clone(), it.copies.clone(), it.perspective_again.clone());
            (st.doc.clone(), st.selection.clone(), st.revision, self.pending_paint.clone(), interaction_fields)
        };
        let (overlays, transform_box) = if self.preview(cmd, p).is_ok() {
            let overlays = self.active().map(|st| outlines_for_selection(&st.doc, &st.selection)).unwrap_or_default();
            let (objects, color) = self
                .active()
                .map(|st| (st.selection.objects.clone(), st.selection.objects.first().map(|id| st.doc.layer_color(*id))))
                .unwrap_or_default();
            let transform_box = if self.measures_deferred_box() { self.transform_box(&objects).zip(color) } else { None };
            (overlays, transform_box)
        } else {
            (vec![], None)
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
        Some(DeferredFeedback { cmd: cmd.to_string(), params: p.clone(), overlays, transform_box })
    }
}

/// What the canvas shows for a deferred preview: see [`Session::deferred_transform_box`].
pub(crate) struct DeferredFeedback {
    cmd: String,
    params: Value,
    overlays: Vec<Overlay>,
    transform_box: Option<(vectorcraft_doc::OrientedBox, [u8; 3])>,
}

/// The most points the outlines of [`outlines_for_selection`] are drawn for, the same number as the
/// canvas's limit on the anchor points it outlines (`OVERLAY_MAX_ANCHORS` in
/// `crates/ui-egui/src/canvas/outline.rs`). Past it the selection's bounds are drawn instead.
const FEEDBACK_MAX_POINTS: usize = 100_000;

/// The outlines of every selected object, a group's members each with their own, or the selection's
/// bounds when the outlines' points and the objects evaluated to make them come to more than
/// [`FEEDBACK_MAX_POINTS`].
fn outlines_for_selection(doc: &vectorcraft_doc::Document, selection: &Selection) -> Vec<Overlay> {
    let nodes = selection.objects.iter().filter_map(|&id| doc.node(id));
    let outlines = vectorcraft_render::drawn_outlines(doc, nodes, FEEDBACK_MAX_POINTS).unwrap_or_else(|| {
        doc.bounds_of(&selection.objects, true).map(|b| vectorcraft_geom::shapes::rectangle(b).to_bezpath()).into_iter().collect()
    });
    outlines.into_iter().map(|path| Overlay::Path { path, color: FEEDBACK, width: 1.0, dashed: false }).collect()
}
