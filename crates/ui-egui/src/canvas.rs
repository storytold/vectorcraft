//! The document canvas: rendering, rulers, navigation, pointer routing to tools and on-canvas
//! selection visuals (bounding box, anchors, handles, smart-guide style labels).

use egui::emath::GuiRounding as _;
use egui::{Color32, CornerRadius, Pos2, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::text::TextKind;
use vectorcraft_doc::{Node, NodeKind, SymbolBoxes, Unit};
use vectorcraft_geom::{Affine, BezPath, PathEl, Point, Rect};
use vectorcraft_tools::{Cursor, Mods, Overlay, PointerEvent, PointerKind};

use crate::opening;
use crate::state::View;
use crate::stylus::StylusFeed;
use crate::theme::{self, Tokens};
use crate::{CacheKey, VectorcraftApp, now_ms, widgets};

mod artboards;
mod context;
mod cursor;
mod drag;
mod fill;
mod grid;
mod home;
mod input;
mod outline;
mod overlays;
mod raster;
mod task_bar;
mod tool_overlays;
mod view;

use artboards::{ArtboardNameEdit, artboard_label, artboard_marks, artboard_name_editor, name_edit_id};
#[cfg(test)]
use artboards::{artboard_name_field_rect, begin_artboard_name_edit, commit_artboard_name, names_an_artboard, tool_name_rect};
use context::{context_menu, panel_drop};
pub use cursor::{HOVER_STILL_SECS, STILL_SECS, ViewKey};
use cursor::{cursor_icon, hover_cursor, hover_wait, still_wait};
use drag::drag_id;
pub use drag::{apply_requests, dispatch, mods};
pub(crate) use drag::{end_document_drag, release_art_in_window, release_over_panel};
#[cfg(test)]
use drag::{frame_moves, tool_drag_doc_id, tool_drag_moved, tool_frame_moves, tool_press_doc_id};
pub use fill::Fill;
use fill::fill_uncovered;
#[cfg(test)]
use fill::{fill_view, polygon_area};
#[cfg(test)]
use grid::ruler_rects;
use grid::{LineLook, PIXEL_GRID, PIXEL_GRID_ZOOM, checker, clip_segment, grid, ruler_input, rulers, textured_quad, view_bounds};
pub(crate) use grid::{RulerScale, checker_texture, ruler_label, ruler_origin_dragging};
#[cfg(test)]
pub(crate) use grid::{TRANSPARENCY_GRID, vertical_label_height};
use home::home;
use input::handle_input;
use outline::KEY_OUTLINE;
#[cfg(test)]
use outline::{OVERLAY_MAX_ANCHORS, c32, node_outline, walk_drawn};
#[cfg(test)]
use overlays::selected_anchor;
use overlays::{HandleLook, draw_overlays, hover_highlight, ime_geometry, selection_overlay};
pub(crate) use raster::color_image;
use raster::{changed_pixels, pixel_region, upload};
use task_bar::task_bar;
pub(crate) use task_bar::{has_colors, task_bar_rect};
pub use tool_overlays::HiddenChars;
#[cfg(test)]
use tool_overlays::{DIAL_RADIUS, HiddenMarks};
use tool_overlays::{
    grid_repeat_overlay, hidden_chars_overlay, live_paint_gap_overlays, mirror_axis_overlay, original_path_overlay, print_tiling_overlay,
    rotate_view_dial, shaper_widget_overlay, slice_overlay, thread_overlays,
};
pub(crate) use view::animated_zoom;
pub use view::fit;
use view::scroll_bars;
#[cfg(test)]
use view::{ScrollRest, Span, scroll_by, scroll_spans, scroll_tracks};

const RULER: f32 = 16.0;
/// The scroll bars' thickness along the window's right and bottom edges.
const SCROLL_BAR: f32 = 12.0;

/// Screen ↔ document mapping for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub rect: egui::Rect,
    pub zoom: f64,
    pub center: Point,
    /// View rotation in radians (Rotate View), clockwise on screen.
    pub rot: f64,
}

impl Xf {
    pub fn new(rect: egui::Rect, v: &View) -> Self {
        Self { rect, zoom: v.zoom, center: v.center, rot: v.rotation.to_radians() }
    }
    pub fn to_screen(&self, p: Point) -> Pos2 {
        let c = self.rect.center();
        let (dx, dy) = ((p.x - self.center.x) * self.zoom, (p.y - self.center.y) * self.zoom);
        let (sn, cs) = self.rot.sin_cos();
        pos2(c.x + (dx * cs - dy * sn) as f32, c.y + (dx * sn + dy * cs) as f32)
    }
    pub fn to_doc(&self, p: Pos2) -> Point {
        let c = self.rect.center();
        let (dx, dy) = ((p.x - c.x) as f64, (p.y - c.y) as f64);
        let (sn, cs) = self.rot.sin_cos();
        let (ux, uy) = (dx * cs + dy * sn, -dx * sn + dy * cs);
        Point::new(self.center.x + ux / self.zoom, self.center.y + uy / self.zoom)
    }
    /// Screen-space corners of a document rect (a rotated quad when the view is rotated).
    pub fn quad(&self, r: Rect) -> Vec<Pos2> {
        [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)].iter().map(|p| self.to_screen(*p)).collect()
    }
    /// Convert a screen-space delta to a document delta.
    pub fn delta_to_doc(&self, d: egui::Vec2) -> vectorcraft_geom::Vec2 {
        let (sn, cs) = self.rot.sin_cos();
        let (dx, dy) = (d.x as f64, d.y as f64);
        vectorcraft_geom::Vec2::new((dx * cs + dy * sn) / self.zoom, (-dx * sn + dy * cs) / self.zoom)
    }
    pub fn rect_to_screen(&self, r: Rect) -> egui::Rect {
        egui::Rect::from_two_pos(self.to_screen(Point::new(r.x0, r.y0)), self.to_screen(Point::new(r.x1, r.y1)))
    }
    /// Affine mapping document points to screen points.
    pub fn affine(&self) -> Affine {
        let c = self.rect.center();
        Affine::translate((c.x as f64, c.y as f64)) * Affine::rotate(self.rot) * Affine::scale(self.zoom) * Affine::translate(-self.center.to_vec2())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Tool,
    /// Hand tool / Space drag, or a middle-button drag with any tool (`middle`).
    Pan {
        start: Pos2,
        center: Point,
        middle: bool,
    },
    /// A Zoom tool drag without Animated Zoom: the area to zoom to.
    ZoomBox {
        start: Pos2,
    },
    /// A Zoom tool press with Animated Zoom ([`animated_zoom`]): dragged sideways it zooms about
    /// `start`, held still (for `held` seconds so far) it zooms on, else it's a click.
    ZoomScrub {
        start: Pos2,
        held: f32,
        moved: bool,
    },
    /// A Rotate View tool drag, or a Space+Shift drag with any tool. With `snaps` (the Rotate
    /// View tool), Shift rounds the angle to 15°.
    RotateView {
        start_angle: f64,
        start_rot: f64,
        snaps: bool,
    },
    /// A Selection tool move over a panel or toolbar: the art waits where it last moved on the
    /// canvas, and the panels get it ([`widgets::PanelDrag::Art`]) if it is released there. Back
    /// on the canvas the move goes on, as in Illustrator (#2826).
    Art,
    /// Fingers making a touch gesture ([`crate::touch`]): the first one's press does nothing.
    Gesture,
    /// A press in Presentation Mode: released as a click, it goes to the next artboard.
    Present,
}

/// Paint the document canvas, complete or cached artwork, navigation and tool overlays.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    // `hidden_chars` holds the marks Show Hidden Characters kept and the text nodes they were laid
    // out from. Only the assignment right before the overlay call below puts them back. A frame
    // that does not draw the overlay drops them, for example with the option off, with the edges
    // hidden, on the Home screen or with no document open.
    let hidden_chars = std::mem::take(&mut app.canvas.hidden_chars);
    // A release ends the Image Trace panel's eye-button hold, also when the panel is no longer
    // drawn, so a later press elsewhere does not show the source image.
    if app.ui.trace_hold_source.is_some() && !ui.input(|i| i.pointer.any_down()) {
        app.ui.trace_hold_source = None;
    }
    // The IME geometry this pass, if the Type tool gets one; `ime::end_of_pass` consumes it after
    // the dialogs and panels draw.
    app.ime_caret_rect = None;
    // A document that opened, closed or became active since Home was chosen replaces it.
    if app.ui.home.is_some_and(|k| k != crate::menus::home_key(app)) {
        app.ui.home = None;
    }
    if crate::menus::home_showing(app) {
        // No IME owner this pass (`ime_caret_rect` stays None): the end-of-pass observation marks
        // the context absent and keeps any marked text as typed.
        home(app, ui, full);
        return;
    }
    if app.session.active().is_none() {
        // No document and Show The Home Screen When No Documents Are Open off: an empty window.
        ui.painter().rect_filled(full, 0.0, t.panel_darker);
        return;
    }
    // Normal Screen Mode and Full Screen Mode with Menu Bar keep the scroll bars along the right
    // and bottom edges; Full Screen and Presentation Mode have none.
    let outer = full;
    let bars = app.ui.screen_mode < 2;
    let full = if bars { egui::Rect::from_min_max(full.min, full.max - vec2(SCROLL_BAR, SCROLL_BAR)) } else { full };
    let rect = if app.ui.view.rulers && app.ui.screen_mode < 3 { egui::Rect::from_min_max(full.min + vec2(RULER, RULER), full.max) } else { full };
    // Dock layout can differ by fractions of a device pixel between fresh windows.
    // Use one pixel-aligned rectangle for drawing, inspection and pointer transforms.
    let rect = rect.round_to_pixels(ui.ctx().pixels_per_point());
    app.canvas_rect = Some(rect);
    let fitted = app.view().is_some_and(|v| v.fitted);
    if !fitted {
        fit(app, "view.fitArtboard");
    }
    // Before the canvas's own interaction, so the view the canvas draws is the one they moved to.
    if bars {
        scroll_bars(app, ui, rect, outer, &t);
    }
    let resp = ui.interact(rect, egui::Id::unique("canvas"), Sense::click_and_drag());
    canvas_input(app, ui, &resp, rect);
    let v = *app.view().unwrap_or(&View::default());
    let xf = Xf::new(rect, &v);
    // While the view pans, zooms or turns under the pointer, the art under it isn't hit-tested.
    let wait = hover_wait(&mut app.canvas.view_moved, ViewKey { rect, zoom: v.zoom, center: v.center, rotation: v.rotation }, ui.input(|i| i.time));
    if let Some(secs) = wait {
        ui.ctx().request_repaint_after_secs(secs as f32);
    }
    let hover_still = wait.is_none();
    let origin_preview = if app.ui.view.rulers && app.ui.screen_mode < 3 { ruler_input(app, ui, full, rect, &xf) } else { None };
    crate::document_drag::drop_on_canvas(app, &resp, &xf);
    panel_drop(app, ui, &resp, &xf);
    // In Presentation Mode a right click or a long touch opens no context menu.
    if app.ui.screen_mode < 3 {
        context_menu(app, &resp, &xf);
    }
    let painter = ui.painter_at(rect);
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let mask_view = st.shown_mask();

    paint_paper(app, ui, &painter, &xf, rect, &t);
    // The grid behind the art (Guides & Grid › Grids In Back), else over it below.
    let grid_look = app.ui.view.grid.then(|| LineLook::grid(&app.session.prefs));
    let grids_in_back = app.session.prefs.grids_in_back;
    if let Some(look) = grid_look.filter(|_| grids_in_back) {
        grid(&painter, &xf, doc.grid.spacing, doc.grid.subdivisions, look);
    }

    // Artwork raster.
    let ppp = ui.ctx().pixels_per_point();
    // Match Renderer::render_as before recording the cache extent or building its view.
    let (w, h) = (
        ((rect.width() * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX)),
        ((rect.height() * ppp).round().max(1.0) as u32).min(u32::from(u16::MAX)),
    );
    // View › Pixel Preview: the art as it rasterizes, one pixel per point, while a document pixel
    // is bigger than a screen pixel (below that the screen render already shows it).
    let pixel = (app.ui.view.pixel_preview && v.zoom * ppp as f64 > 1.0).then(|| pixel_region(&xf)).flatten();
    // File Handling › Display Bitmaps as Anti-aliased Images in Pixel Preview: off, images show
    // their pixels as they rasterize, hard-edged.
    let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let key = cache_key(app, ui, st, &t, &v, (w, h, ppp), (pixel, smooth_images));
    // Placed documents' bitmaps are being made: draw again when they are ready.
    if vectorcraft_render::placed_document::busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
    poll_worker(app, ui, key.doc);
    // With `waits_for_still` set (the web build, which has no render worker), a heavy document is
    // not rendered while only the view changes: the frame shown is drawn reprojected below, and the
    // first frame after the view has been still for STILL_SECS renders.
    let wait = if app.canvas.waits_for_still {
        let slow = app.canvas.last_ms > 8.0 && app.canvas.texture.is_some();
        still_wait(&mut app.canvas.still, app.canvas.key.as_ref(), &key, slow, ui.input(|i| i.time))
    } else {
        None
    };
    if let Some(secs) = wait {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(secs));
    }
    if wait.is_none() && (app.canvas.key.as_ref() != Some(&key) || app.canvas.texture.is_none()) {
        let (w, h, view) = frame_view(pixel, (w, h), &v, ppp);
        let opts = frame_options(app, &key, mask_view, smooth_images);
        draw_frame(app, ui, Frame { key: &key, doc: &doc, size: (w, h), view, opts, pixel });
    }
    // The status bar reports each image that a renderer (this thread's or the worker's) could not
    // decode, once per image in the process.
    let undecoded = vectorcraft_render::newly_undecodable(doc.images.values());
    if undecoded > 0 {
        app.status(format!("{undecoded} image(s) couldn't be decoded and show as crossed boxes"));
    }
    fill_uncovered(app, ui.ctx(), &painter, &key, &doc, rect, &xf);
    reproject_frame(app, &painter, &xf, rect, (key.doc, v.rotation));
    if let Some(look) = grid_look.filter(|_| !grids_in_back) {
        grid(&painter, &xf, doc.grid.spacing, doc.grid.subdivisions, look);
    }
    // The pixel grid (Guides & Grid › Show Pixel Grid (Above 600% Zoom)): in Pixel Preview from
    // 600% zoom, a line at every document pixel over the art, so the pixels it rasterizes to show.
    if app.ui.view.pixel_preview && v.zoom >= PIXEL_GRID_ZOOM && app.session.prefs.show_pixel_grid {
        grid(&painter, &xf, 1.0, 1, LineLook { color: PIXEL_GRID, dots: false });
    }
    artboard_outlines(app, ui, &painter, &xf, &t);
    if app.ui.view.guides {
        guide_lines(app, &painter, &xf, rect, &t);
    }

    if !app.session.slices_hidden() {
        slice_overlay(app, &painter, &xf);
    }
    if app.session.active().is_some_and(|d| d.print_tiling) || app.session.tool_id() == "printTiling" {
        print_tiling_overlay(app, &painter, &xf);
    }
    original_path_overlay(app, &painter, &xf);

    let revolve_edit = app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active);
    let view_info = app.view_info();
    let overlays = selection_visuals(app, &painter, &xf, (hidden_chars, hover_still), revolve_edit, view_info);
    draw_overlays(&painter, &xf, &overlays, &t, HandleLook::of(&app.session.prefs));
    if !revolve_edit {
        shaper_widget_overlay(app, &painter, &xf);
    }
    ime_geometry(app, ui.ctx(), &xf);
    crate::place::paint_drop_highlight(app, ui.ctx(), &painter, rect);
    // A Rotate View drag (the tool, or Space+Shift with any tool) shows the view's angle on a dial
    // at the canvas center, the point the view turns about.
    if app.ui.screen_mode < 3 && matches!(ui.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::RotateView { .. })) {
        rotate_view_dial(&painter, rect.center(), v.rotation, &t);
    }

    if app.ui.view.rulers && app.ui.screen_mode < 3 {
        ruler_bands(app, ui, &xf, (full, rect), origin_preview, &t);
    }
    // The task bar is not drawn while the Simplify or Smooth slider shows.
    let strip = crate::dialogs::simplify_slider::showing(app) || crate::dialogs::smooth_slider::showing(app);
    if app.ui.task_bar && !revolve_edit && !app.session.tool_busy() && app.ui.screen_mode < 3 && !strip {
        task_bar(app, ui, &xf);
    }
    if app.ui.screen_mode < 3 {
        crate::free_transform::show(app, ui, xf.rect);
    }
    artboard_name_editor(app, ui, &xf);
    if resp.hovered() {
        canvas_cursor(app, ui, view_info, hover_still);
    }
}

/// The pointer on the canvas: the tool's input, unless the context menu or the 3D gizmo has the press.
fn canvas_input(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) {
    // A press on the canvas while the context menu is open only closes it (a drag too, which egui
    // alone would leave open); the tool doesn't get it.
    if app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active) {
        // The artboard's 3D gizmo owns these gestures while its options are open.
        // Do not let a ring drag also move or deselect the editable source path.
    } else if resp.context_menu_opened() && resp.hovered() && ui.input(|i| i.pointer.any_pressed()) {
        egui::Popup::close_all(ui.ctx());
    } else {
        handle_input(app, ui, resp, rect);
        // `hover_doc` is the document point under the pointer in the view `handle_input` left. The
        // wheel, a pan, a Rotate View drag, the Zoom tool and a Presentation Mode click move the view
        // there. A dialog, floating panel or menu over the canvas hides it (Object Highlighting
        // showed through them: #4518), unless a drag that began on the canvas goes on there.
        let moved = Xf::new(rect, app.view().unwrap_or(&View::default()));
        let dragging = ui.data(|d| d.get_temp::<Drag>(drag_id()).is_some());
        app.hover_doc =
            ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p) && (dragging || drag::on_canvas(ui, resp, *p))).map(|p| moved.to_doc(p));
    }
}

/// The pointer over the canvas: the view tools' cursors, else the tool's own.
fn canvas_cursor(app: &mut VectorcraftApp, ui: &Ui, view_info: vectorcraft_engine::ViewInfo, hover_still: bool) {
    if opening::long_operation_busy(app) {
        ui.ctx().set_cursor_icon(cursor_icon(Cursor::Busy));
        return;
    }
    let m = ui.input(|i| i.modifiers);
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    let drag = ui.data(|d| d.get_temp::<Drag>(drag_id()));
    let panning = matches!(drag, Some(Drag::Pan { .. }));
    // A zoom drag keeps its cursor when Space is pressed (with or without Shift) or Cmd is let go.
    let zoom_drag = matches!(drag, Some(Drag::ZoomBox { .. } | Drag::ZoomScrub { .. }));
    let zooming = zoom_drag || if space { m.command } else { app.session.tool_id() == "zoom" };
    // A rotating drag keeps its cursor when Shift is let go.
    let rotating = matches!(drag, Some(Drag::RotateView { .. }))
        || (!zoom_drag && if space { m.shift && !m.command } else { app.session.tool_id() == "rotateView" });
    // The hand and the magnifier are drawn like the tool cursors below: Windows has no
    // system cursor for them (winit shows the four-headed move cursor for Grab).
    let view_cursor = if panning {
        Some(Cursor::HandGrab)
    } else if rotating {
        Some(Cursor::Rotate)
    } else if zooming {
        Some(if m.alt { Cursor::ZoomOut } else { Cursor::ZoomIn })
    } else if space || app.session.tool_id() == "hand" {
        Some(Cursor::Hand)
    } else {
        None
    };
    // In Presentation Mode the pointer is the default one, with no tool cursor.
    let tool_cursor = (app.ui.screen_mode < 3).then(|| view_cursor.or_else(|| hover_cursor(app, mods(m, space), view_info, hover_still)));
    let cur = if let Some(c) = tool_cursor.flatten() {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("tool-cursor")));
        // The loaded place cursor carries the file's thumbnail.
        if app.session.tool_id() == "place"
            && let Some(hp) = ui.input(|i| i.pointer.hover_pos())
        {
            crate::place::paint_cursor(app, ui.ctx(), &painter, hp);
        }
        // The glyph cursors are drawn here; General › Use Precise Cursors (the session turns the drawing tools' pointers
        // into crosshairs, `Cursor::precise`) needs no handling. VECTORCRAFT_SYSTEM_CURSORS opts out of the glyphs.
        let custom = std::env::var_os("VECTORCRAFT_SYSTEM_CURSORS").is_none();
        // User Interface › Scale Cursor Proportional to UI: the glyphs grow with UI Scaling;
        // off, they keep their size.
        let ui_scale = if app.session.prefs.scale_cursor_with_ui { 1.0 } else { ui.ctx().zoom_factor() };
        match ui.input(|i| i.pointer.hover_pos()) {
            // An OS cursor: the system moves it at once, where a painted one trails the pointer
            // (#444). The system cursor stands in for cursors without a glyph. Over a window
            // without the focus, macOS shows its own pointer whichever cursor the window sets,
            // and the glyph is not painted there.
            Some(_) if custom && crate::cursors::os_cursors(crate::cursors::OS_CURSORS, app.services.wayland) => {
                let ppp = ui.ctx().pixels_per_point() / ui_scale;
                ui.ctx().set_cursor_image(app.canvas.cursors.get(c, ppp));
                cursor_icon(c)
            }
            Some(hp)
                if custom
                    && crate::cursors::paint_glyphs(crate::cursors::HIDES_ONLY_WHEN_KEY, ui.input(|i| i.focused))
                    && crate::cursors::paint(&painter, c, hp, 1.0 / ui_scale) =>
            {
                // No cursor theme (Wayland): the glyph is painted, the system pointer stays (#2450).
                if crate::cursors::hide_system_pointer(app.services.no_cursor_theme) { egui::CursorIcon::None } else { cursor_icon(c) }
            }
            _ => cursor_icon(c),
        }
    } else {
        egui::CursorIcon::Default
    };
    ui.ctx().set_cursor_icon(cur);
}

/// The rulers, and the origin being dragged out of their corner.
fn ruler_bands(app: &VectorcraftApp, ui: &Ui, xf: &Xf, (full, rect): (egui::Rect, egui::Rect), origin_preview: Option<Point>, t: &Tokens) {
    let painter = ui.painter_at(rect);
    if let Some(origin) = origin_preview {
        let at = xf.to_screen(origin);
        painter.line_segment([pos2(at.x, rect.top()), pos2(at.x, rect.bottom())], Stroke::new(1.0, t.text_dim));
        painter.line_segment([pos2(rect.left(), at.y), pos2(rect.right(), at.y)], Stroke::new(1.0, t.text_dim));
    }
    // View › Rulers › Show Video Rulers counts the active artboard in video pixels.
    let video = app.ui.view.video_rulers.then(|| crate::dialogs::artboard_options::active_index(app));
    let scale = match video.and_then(|i| app.session.active()?.doc.artboards.get(i)) {
        Some(ab) => RulerScale::video(ab),
        None => RulerScale::document(app.session.general_unit(), origin_preview.unwrap_or_else(|| app.ruler_origin())),
    };
    rulers(ui, full, xf, app.hover_doc, scale, t);
}

/// What the cached frame depends on.
fn cache_key(
    app: &VectorcraftApp,
    ui: &Ui,
    st: &vectorcraft_engine::DocState,
    t: &Tokens,
    v: &View,
    (w, h, ppp): (u32, u32, f32),
    (pixel, smooth_images): (Option<[i64; 4]>, bool),
) -> CacheKey {
    // Paths made into guides are drawn as the ruler guides are (Preferences › Guides & Grid).
    let guide_look = LineLook::guides(&app.session.prefs, t);
    CacheKey {
        guide: ([guide_look.color.r(), guide_look.color.g(), guide_look.color.b()], guide_look.dots),
        doc: st.uid as usize,
        revision: st.revision,
        interacting: st.interaction.is_some(),
        zoom: v.zoom,
        cx: v.center.x,
        cy: v.center.y,
        w,
        h,
        outline: app.ui.view.outline,
        trim: app.ui.view.trim_view,
        templates: app.ui.view.templates,
        guides: app.ui.view.guides,
        ppp,
        hidden: vec![],
        rot: v.rotation,
        anti_alias: app.session.prefs.anti_aliased_artwork,
        highlight_alternate_glyphs: app.session.prefs.highlight_alternate_glyphs,
        highlight_substituted_fonts: app.session.prefs.highlight_substituted_fonts,
        placed: vectorcraft_render::placed_document::generation(),
        pixel,
        smooth_images,
        isolated: st.isolation.map(|id| id.0),
        // Appearance of Black › On Screen: Display All Blacks as Rich Black.
        rich_black: app.session.prefs.black_on_screen == "rich",
        // A tool's drag on the canvas, during an interaction (not a dialog's preview).
        dragging: app.session.in_interaction() && ui.data(|d| d.get_temp::<Drag>(drag_id())) == Some(Drag::Tool),
        // The Image Trace panel's eye button, while it is held.
        trace_source: app.ui.trace_hold_source.clone(),
    }
}

/// Start the render worker, and upload the frame it finished.
fn poll_worker(app: &mut VectorcraftApp, ui: &Ui, key_doc: usize) {
    if !app.canvas.worker_started {
        app.canvas.worker_started = true;
        if std::env::var_os("VECTORCRAFT_SYNC_RENDER").is_none() {
            app.canvas.worker = crate::render_worker::Worker::spawn(ui.ctx().clone());
            app.canvas.fill_worker = crate::render_worker::Worker::spawn(ui.ctx().clone());
        }
    }
    // Upload finished background renders.
    if let Some(done) = app.canvas.worker.as_mut().and_then(|w| w.poll())
        && done.key.doc == key_doc
    {
        upload(app, ui.ctx(), &done.img, done.key.pixel.is_some());
        app.canvas.key = Some(done.key);
        app.canvas.shown = Some((done.doc, done.opts));
        app.perf.render_ms = done.ms;
        app.canvas.last_ms = done.ms;
        app.canvas.moved = done.moved > 0;
        app.canvas.from_worker = true;
    }
}

/// The size and view matrix of the frame to render: the pixel region in Pixel Preview, else the whole canvas.
fn frame_view(pixel: Option<[i64; 4]>, (w, h): (u32, u32), v: &View, ppp: f32) -> (u32, u32, Affine) {
    match pixel {
        Some([x0, y0, x1, y1]) => ((x1 - x0) as u32, (y1 - y0) as u32, Affine::translate((-x0 as f64, -y0 as f64))),
        None => (
            w,
            h,
            Affine::translate((w as f64 / 2.0, h as f64 / 2.0))
                * Affine::rotate(v.rotation.to_radians())
                * Affine::scale(v.zoom * ppp as f64)
                * Affine::translate(-v.center.to_vec2()),
        ),
    }
}

/// The options the canvas renders the document with.
fn frame_options(
    app: &VectorcraftApp,
    key: &CacheKey,
    mask_view: Option<vectorcraft_doc::NodeId>,
    smooth_images: bool,
) -> vectorcraft_render::RenderOptions {
    let opts = vectorcraft_render::RenderOptions { outline: app.ui.view.outline, background: None, artboards: false, ..Default::default() };
    vectorcraft_render::RenderOptions {
        proof: vectorcraft_render::proof::active_proof(),
        overprint_preview: vectorcraft_render::proof::overprint_preview_on(),
        trim: app.ui.view.trim_view,
        hide_templates: !app.ui.view.templates,
        hide_guides: !app.ui.view.guides,
        tile_edge: vectorcraft_color::Color::from_hex(&app.session.prefs.pattern_tile_edge_color).map_or(opts.tile_edge, |c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b]
        }),
        guide: key.guide,
        mask_view,
        highlight_substitutions: true,
        // Type › Highlight Alternate Glyphs (#394): pink behind OpenType substitutions.
        highlight_alternate_glyphs: app.session.prefs.highlight_alternate_glyphs,
        // Type › Highlight Substituted Fonts: pink behind text in a substitute font.
        highlight_substituted_fonts: app.session.prefs.highlight_substituted_fonts,
        // General › Anti-aliased Artwork: off, edges are hard on screen (raster effects and
        // pattern tiles stay smooth), as in Illustrator.
        anti_alias: if app.session.prefs.anti_aliased_artwork { vectorcraft_render::AntiAlias::Art } else { vectorcraft_render::AntiAlias::None },
        progressive_placed: true,
        trace_views: true,
        // Image Trace › the eye button held: the source image of the selected objects.
        trace_source: key.trace_source.clone(),
        smooth_images,
        // Isolation mode: the art around the isolated group or layer is dimmed (#833).
        isolated: key.isolated.map(vectorcraft_doc::NodeId),
        // Images far smaller on screen than they are sample their mip levels (#1244).
        image_mips: true,
        // An image whose pixels can't be decoded shows as a crossed box, and the status message
        // below reports it.
        mark_undecoded_images: true,
        // During a drag or a dialog's preview, objects that only moved draw their raster
        // effects from the cached rasters of an earlier frame (#1257).
        interacting: key.interacting,
        // During a tool's drag, raster effects without a cached raster to draw render at a
        // lower resolution (#2558); the frame after the drag renders them as a still frame does.
        dragging: key.dragging,
        rich_black: key.rich_black,
        ..opts
    }
}

/// A frame to render: here, or on the worker.
struct Frame<'a> {
    key: &'a CacheKey,
    doc: &'a std::sync::Arc<vectorcraft_doc::Document>,
    size: (u32, u32),
    view: Affine,
    opts: vectorcraft_render::RenderOptions,
    pixel: Option<[i64; 4]>,
}

fn draw_frame(app: &mut VectorcraftApp, ui: &Ui, frame: Frame) {
    let Frame { key, doc, size: (w, h), view, opts, pixel } = frame;
    // to the worker. An interaction's frames stay on the worker once it drew the frame shown:
    // its renderer keeps the rasters of the art's effects that make those frames fast, and this
    // thread's renderer has none, so a frame fast on the worker would be slow here.
    let same_doc = app.canvas.key.as_ref().is_some_and(|k| k.doc == key.doc);
    let settling = app.canvas.moved && !key.interacting;
    let staying = key.interacting && app.canvas.from_worker;
    let heavy = (app.canvas.last_ms > 8.0 || settling || staying) && app.canvas.texture.is_some() && same_doc;
    let changed = changed_pixels(app, key, doc, &opts, view, w, h);
    match (&mut app.canvas.worker, heavy, changed) {
        // Only the document changed since the frame shown, in a part of it: that part is
        // rendered again, here, into the texture. `last_ms` stays the whole frame's.
        (_, _, Some(region)) => {
            let t0 = now_ms();
            if let (Some([x0, y0, x1, y1]), Some(tex)) = (region, &mut app.canvas.texture) {
                let part = app.canvas.renderer.render(doc, x1 - x0, y1 - y0, Affine::translate((-f64::from(x0), -f64::from(y0))) * view, &opts);
                let color = color_image(&part);
                let options = if pixel.is_some() { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
                tex.set_partial([x0 as usize, y0 as usize], color, options);
                app.canvas.moved |= app.canvas.renderer.stats.moved > 0;
            }
            app.canvas.key = Some(key.clone());
            app.canvas.shown = Some((doc.clone(), opts.clone()));
            app.perf.render_ms = now_ms() - t0;
        }
        (Some(worker), true, None) => worker.submit(crate::render_worker::Job { key: key.clone(), doc: doc.clone(), w, h, view, opts }),
        _ => {
            let t0 = now_ms();
            let img = app.canvas.renderer.render(doc, w, h, view, &opts);
            upload(app, ui.ctx(), &img, pixel.is_some());
            app.canvas.key = Some(key.clone());
            app.canvas.shown = Some((doc.clone(), opts.clone()));
            app.perf.render_ms = now_ms() - t0;
            app.canvas.last_ms = app.perf.render_ms;
            app.canvas.moved = app.canvas.renderer.stats.moved > 0;
            app.canvas.from_worker = false;
        }
    }
}

/// The pasteboard, the artboards' shadows and paper, and the outline mode's white artboards.
fn paint_paper(app: &VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, t: &Tokens) {
    let Some(st) = app.session.active() else { return };
    let doc = &st.doc;
    // Pasteboard, artboard shadows and paper (Document Setup: the transparency grid's look, the
    // simulated paper colour; a white Background Contents hides the grid).
    // An artboard with a background color draws its paper in that color. With Simulate Colored Paper
    // on, the canvas multiplies it with the paper color: each sRGB channel is artboard × paper / 255,
    // rounded. The transparency grid, while it shows, covers it as it covers the others.
    painter.rect_filled(rect, 0.0, t.pasteboard);
    if app.ui.view.artboards && !app.ui.view.outline {
        let setup = &doc.setup;
        let grid = (st.transparency_grid && setup.background == vectorcraft_doc::Background::Transparent).then(|| checker_texture(ui.ctx(), setup));
        let paper = if setup.simulate_paper { crate::panels::c32(&setup.paper()) } else { Color32::WHITE };
        for ab in &doc.artboards {
            let q = xf.quad(ab.rect);
            // Hard 2 pt drop shadow, right and bottom (measured: #4d4d4d then #565656 on #606060).
            let shift = |d: f32| q.iter().map(|p| *p + vec2(d, d)).collect::<Vec<_>>();
            painter.add(Shape::convex_polygon(shift(2.0), Color32::from_black_alpha(26), Stroke::NONE));
            painter.add(Shape::convex_polygon(shift(1.0), Color32::from_black_alpha(52), Stroke::NONE));
            match &grid {
                Some((tex, cell)) => checker(painter, &q, ab.rect, xf.zoom, tex.id(), *cell),
                None => {
                    let fill = ab.color.map_or(paper, |c| crate::panels::c32(&c) * paper);
                    painter.add(Shape::convex_polygon(q, fill, Stroke::NONE));
                }
            }
        }
    } else if app.ui.view.outline {
        for ab in &doc.artboards {
            painter.add(Shape::convex_polygon(xf.quad(ab.rect), Color32::WHITE, Stroke::NONE));
        }
    }
}

/// The last rendered frame, placed through the current view while a new one is on its way.
fn reproject_frame(app: &VectorcraftApp, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, (key_doc, rotation): (usize, f64)) {
    if let (Some(tex), Some(k)) = (&app.canvas.texture, &app.canvas.key)
        && k.doc == key_doc
    {
        // The frame being shown covers the canvas rect it was rendered for. While a panel is
        // dragged that rect is a different size, so reprojecting the frame through the *current*
        // rect stretches the art to the new size until the worker's frame arrives, then it snaps
        // back. Rebuild the rect it was rendered for from the key's own pixel size instead.
        let old_rect = {
            let ppp = if k.ppp > 0.0 { k.ppp } else { 1.0 };
            egui::Rect::from_center_size(rect.center(), vec2(k.w as f32 / ppp, k.h as f32 / ppp))
        };
        match k.pixel {
            // Document pixels: placed over the document rect they cover, turning with the view.
            Some([x0, y0, x1, y1]) => {
                textured_quad(painter, tex.id(), &xf.quad(Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64)), (1.0, 1.0));
            }
            None if (k.rot - rotation).abs() < 1e-9 => {
                // Reproject the last frame if it was rendered for a different view.
                let old = Xf { rect: old_rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: xf.rot };
                let a = xf.to_screen(old.to_doc(old_rect.min));
                let b = xf.to_screen(old.to_doc(old_rect.max));
                painter.image(tex.id(), egui::Rect::from_min_max(a, b), egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // The view turned after the last frame was rendered, and the worker renders the frame at
            // the new angle. Until it arrives, the last frame is drawn turned into this view: each
            // corner goes where this view puts the document point that the corner showed.
            None => {
                let old = Xf { rect: old_rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: k.rot.to_radians() };
                let q =
                    [old_rect.left_top(), old_rect.right_top(), old_rect.right_bottom(), old_rect.left_bottom()].map(|c| xf.to_screen(old.to_doc(c)));
                textured_quad(painter, tex.id(), &q, (1.0, 1.0));
            }
        }
    }
}

/// The artboards' edges, bleeds, marks and names.
fn artboard_outlines(app: &VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf, t: &Tokens) {
    let Some(st) = app.session.active() else { return };
    let doc = &st.doc;
    // Artboard edges and names. The Artboard tool labels its active artboard itself (following
    // a move or resize): that one isn't named twice.
    let active_ab = crate::panels::artboards::selected(app, doc.artboards.len());
    let artboard_tool = app.session.tool_id() == "artboard";
    let tool_labelled = artboard_tool.then(|| app.session.tool_options()["active"].as_u64()).flatten();
    let editing_name = ui.data(|d| d.get_temp::<ArtboardNameEdit>(name_edit_id()).map(|e| e.index));
    for (i, ab) in doc.artboards.iter().enumerate() {
        let c = if i == active_ab { Color32::from_gray(0) } else { Color32::from_gray(120) };
        painter.add(Shape::closed_line(xf.quad(ab.rect), Stroke::new(if i == active_ab { 1.0 } else { 0.6 }, c)));
        // The bleed (Document Setup) as a red outline around the artboard.
        if doc.setup.has_bleed() {
            painter.add(Shape::closed_line(xf.quad(doc.setup.bleed_rect(ab.rect)), Stroke::new(1.0, t.bleed)));
        }
        // Show Center Mark, Show Cross Hairs and Show Video Safe Areas (Artboard Options). Only the
        // canvas draws them, as it does the edges: exports and print leave them out.
        artboard_marks(painter, xf, ab, t.artboard_mark);
        if editing_name != Some(i) && (artboard_tool || doc.artboards.len() > 1) && tool_labelled != u64::try_from(i).ok() {
            // At the artboard's own top-left corner, which a rotated view turns away from its
            // bounding box's (#1095).
            let corner = xf.to_screen(Point::new(ab.rect.x0, ab.rect.y0));
            artboard_label(painter, corner, (ab.rect.width() * xf.zoom) as f32, &format!("{:02} - {}", i + 1, ab.name), t.text_dim);
        }
    }
}

/// The ruler guides.
fn guide_lines(app: &VectorcraftApp, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, t: &Tokens) {
    let Some(st) = app.session.active() else { return };
    let doc = &st.doc;
    let look = LineLook::guides(&app.session.prefs, t);
    // Selected guides in the selection colour.
    let picked = LineLook { color: t.selection, ..look };
    let selected: &[usize] = app.session.active().map_or(&[], |st| &st.selection.guides);
    let key = app.session.active().and_then(|st| st.selection.key_guide);
    let view = view_bounds(xf);
    // Guides on a hidden layer hide with it.
    for (i, g) in doc.guides.iter().enumerate().filter(|(_, g)| doc.guide_shown(g)) {
        let at = |along: f64| xf.to_screen(if g.vertical { Point::new(g.pos, along) } else { Point::new(along, g.pos) });
        // An artboard guide runs across its artboard, a canvas guide across the window (the part
        // of its document line that the canvas shows, turned with the view).
        let (a, b) = match doc.guide_span(g) {
            Some((from, to)) => (at(from), at(to)),
            None => {
                let (from, to) = if g.vertical { (view.y0, view.y1) } else { (view.x0, view.x1) };
                let Some(ends) = clip_segment(at(from), at(to), rect) else { continue };
                ends
            }
        };
        if selected.contains(&i) { picked } else { look }.guide(painter, a, b);
        // The key guide (Align to Key Object, #2143): thicker, as a key object's outline is.
        if key == Some(i) {
            painter.line_segment([a, b], Stroke::new(KEY_OUTLINE, picked.color));
        }
    }
}

/// The selection's visuals and the tools' overlays, to be drawn.
fn selection_visuals(
    app: &mut VectorcraftApp,
    painter: &egui::Painter,
    xf: &Xf,
    (hidden_chars, hover_still): (HiddenChars, bool),
    revolve_edit: bool,
    view_info: vectorcraft_engine::ViewInfo,
) -> Vec<Overlay> {
    // View → Hide Gradient Annotator hides the Gradient tool's annotator.
    let mut overlays = if !revolve_edit && (app.session.tool_id() != "gradient" || app.ui.view.gradient_annotator) {
        app.session.overlays(view_info)
    } else {
        vec![]
    };
    // Gap Options' Preview shows the gaps it finds.
    overlays.extend(crate::dialogs::live_paint_gaps::overlays(app));
    if app.ui.view.edges && !revolve_edit {
        if hover_still {
            hover_highlight(app, painter, xf);
        }
        selection_overlay(app, painter, xf, overlays.iter().any(|o| matches!(o, Overlay::CurvePoint { .. })));
        mirror_axis_overlay(app, painter, xf);
        grid_repeat_overlay(app, painter, xf);
        if app.ui.view.text_threads {
            // Under the tools' overlays: a selected frame's own ports are drawn over its thread's.
            let mut threads = thread_overlays(app, xf.zoom);
            threads.append(&mut overlays);
            overlays = threads;
        }
        if app.ui.view.live_paint_gaps {
            overlays.extend(live_paint_gap_overlays(app));
        }
        if app.ui.view.hidden_chars {
            app.canvas.hidden_chars = hidden_chars;
            hidden_chars_overlay(app, painter, xf);
        }
    }
    overlays
}

#[cfg(test)]
mod tests;
