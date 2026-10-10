//! The document canvas: rendering, rulers, navigation, pointer routing to tools and on-canvas
//! selection visuals (bounding box, anchors, handles, smart-guide style labels).

use egui::{Color32, CornerRadius, Pos2, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::{Node, NodeKind, Unit};
use vectorcraft_geom::{Affine, BezPath, PathEl, Point, Rect};
use vectorcraft_tools::{Cursor, Mods, Overlay, PointerEvent, PointerKind};

use crate::state::View;
use crate::theme::{self, Tokens};
use crate::{CacheKey, VectorcraftApp, now_ms, widgets};

const RULER: f32 = 16.0;

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
    RotateView {
        start_angle: f64,
        start_rot: f64,
    },
    /// A Selection tool move dragged off the canvas: the panels get the art
    /// ([`widgets::PanelDrag::Art`]).
    Art,
    /// Fingers making a touch gesture ([`crate::touch`]): the first one's press does nothing.
    Gesture,
}

fn drag_id() -> egui::Id {
    egui::Id::new("canvas-drag")
}
/// Where the pointer was last seen during a press on the canvas.
fn drag_pos_id() -> egui::Id {
    egui::Id::new("canvas-drag-pos")
}

/// The pen pressure (0..1) of the press in progress: the force of this frame's pen or touch
/// input, else the last one seen since the press (`pressed`: a new press, whose mouse has none
/// until a pen reports it). A mouse presses fully (1).
fn pen_pressure(ui: &Ui, pressed: bool) -> f32 {
    let id = egui::Id::new("canvas-pressure");
    let force = ui.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } if f.is_finite() => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    });
    let p = force.or_else(|| if pressed { None } else { ui.data(|d| d.get_temp::<f32>(id)) }).unwrap_or(1.0);
    ui.data_mut(|d| d.insert_temp(id, p));
    p
}

pub fn mods(m: egui::Modifiers, space: bool) -> Mods {
    Mods { shift: m.shift, alt: m.alt, cmd: m.command, ctrl: m.ctrl, space }
}

/// Fit the view (View → Fit Artboard / Fit All / Actual Size).
pub fn fit(app: &mut VectorcraftApp, how: &str) {
    let Some(rect) = app.canvas_rect else {
        if let Some(v) = app.view_mut() {
            v.fitted = false;
        }
        return;
    };
    let current = app.view().map_or(0, |v| v.artboard);
    let Some(st) = app.session.active() else { return };
    let target = match how {
        "view.fitAll" => {
            st.doc.art_bounds().map(|a| st.doc.artboards.iter().fold(a, |r, ab| r.union(ab.rect))).or(st.doc.artboards.first().map(|a| a.rect))
        }
        // The navigator's artboard (the first if it has gone).
        _ => st.doc.artboards.get(current).or(st.doc.artboards.first()).map(|a| a.rect),
    };
    let Some(target) = target else { return };
    let zoom = if how == "view.actualSize" {
        actual_size_zoom(app.session.prefs.display_print_size)
    } else {
        let zx = (rect.width() as f64 - 60.0) / target.width().max(1.0);
        let zy = (rect.height() as f64 - 60.0) / target.height().max(1.0);
        zx.min(zy).clamp(0.0313, 640.0)
    };
    let Some(v) = app.view_mut() else { return };
    v.center = target.center();
    v.fitted = true;
    v.zoom = zoom;
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    // A document that opened, closed or became active since Home was chosen replaces it.
    if app.ui.home.is_some_and(|k| k != crate::menus::home_key(app)) {
        app.ui.home = None;
    }
    if crate::menus::home_showing(app) {
        home(app, ui, full);
        return;
    }
    if app.session.active().is_none() {
        // No document and Show The Home Screen When No Documents Are Open off: an empty window.
        ui.painter().rect_filled(full, 0.0, t.panel_darker);
        return;
    }
    let rect = if app.ui.view.rulers && app.ui.screen_mode < 3 { egui::Rect::from_min_max(full.min + vec2(RULER, RULER), full.max) } else { full };
    app.canvas_rect = Some(rect);
    let fitted = app.view().is_some_and(|v| v.fitted);
    if !fitted {
        fit(app, "view.fitArtboard");
    }
    let resp = ui.interact(rect, egui::Id::new("canvas"), Sense::click_and_drag());
    // A press on the canvas while the context menu is open only closes it (a drag too, which egui
    // alone would leave open); the tool doesn't get it.
    if resp.context_menu_opened() && resp.hovered() && ui.input(|i| i.pointer.any_pressed()) {
        egui::Popup::close_all(ui.ctx());
    } else {
        handle_input(app, ui, &resp, rect);
    }
    let v = *app.view().unwrap_or(&View::default());
    let xf = Xf::new(rect, &v);
    if app.ui.view.rulers && app.ui.screen_mode < 3 {
        ruler_guides(app, ui, full, rect, &xf);
    }
    panel_drop(app, ui, &resp, &xf);
    context_menu(app, &resp, &xf);
    let painter = ui.painter_at(rect);
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let mask_view = st.shown_mask();

    // Pasteboard, artboard shadows and paper (Document Setup: the transparency grid's look, the
    // simulated paper colour; a white Background Contents hides the grid).
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
                Some((tex, cell)) => checker(&painter, &q, ab.rect, xf.zoom, tex.id(), *cell),
                None => {
                    painter.add(Shape::convex_polygon(q, paper, Stroke::NONE));
                }
            }
        }
    } else if app.ui.view.outline {
        for ab in &doc.artboards {
            painter.add(Shape::convex_polygon(xf.quad(ab.rect), Color32::WHITE, Stroke::NONE));
        }
    }
    // The grid behind the art (Guides & Grid › Grids In Back), else over it below.
    let grid_look = app.ui.view.grid.then(|| LineLook::grid(&app.session.prefs));
    let grids_in_back = app.session.prefs.grids_in_back;
    if let Some(look) = grid_look.filter(|_| grids_in_back) {
        grid(&painter, &xf, doc.grid.spacing, doc.grid.subdivisions, look);
    }

    // Artwork raster.
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);
    // View › Pixel Preview: the art as it rasterizes, one pixel per point, while a document pixel
    // is bigger than a screen pixel (below that the screen render already shows it).
    let pixel = (app.ui.view.pixel_preview && v.zoom * ppp as f64 > 1.0).then(|| pixel_region(&xf)).flatten();
    // File Handling › Display Bitmaps as Anti-aliased Images in Pixel Preview: off, images show
    // their pixels as they rasterize, hard-edged.
    let smooth_images = pixel.is_none() || app.session.prefs.anti_aliased_bitmaps;
    let key = CacheKey {
        doc: st.uid as usize,
        revision: st.revision,
        zoom: v.zoom,
        cx: v.center.x,
        cy: v.center.y,
        w,
        h,
        outline: app.ui.view.outline,
        trim: app.ui.view.trim_view,
        ppp,
        hidden: vec![],
        rot: v.rotation,
        anti_alias: app.session.prefs.anti_aliased_artwork,
        placed: vectorcraft_render::placed_document::generation(),
        pixel,
        smooth_images,
        isolated: st.isolation.map(|id| id.0),
    };
    // Placed documents' bitmaps are being made: draw again when they are ready.
    if vectorcraft_render::placed_document::busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
    }
    if !app.canvas.worker_started {
        app.canvas.worker_started = true;
        if std::env::var_os("VECTORCRAFT_SYNC_RENDER").is_none() {
            app.canvas.worker = crate::render_worker::Worker::spawn(ui.ctx().clone());
        }
    }
    // Upload finished background renders.
    if let Some(done) = app.canvas.worker.as_mut().and_then(|w| w.poll())
        && done.key.doc == key.doc
    {
        upload(app, ui.ctx(), &done.img, done.key.pixel.is_some());
        app.canvas.key = Some(done.key);
        app.perf.render_ms = done.ms;
        app.canvas.last_ms = done.ms;
    }
    if app.canvas.key.as_ref() != Some(&key) || app.canvas.texture.is_none() {
        let (w, h, view) = match pixel {
            Some([x0, y0, x1, y1]) => ((x1 - x0) as u32, (y1 - y0) as u32, Affine::translate((-x0 as f64, -y0 as f64))),
            None => (
                w,
                h,
                Affine::translate((w as f64 / 2.0, h as f64 / 2.0))
                    * Affine::rotate(v.rotation.to_radians())
                    * Affine::scale(v.zoom * ppp as f64)
                    * Affine::translate(-v.center.to_vec2()),
            ),
        };
        let opts = vectorcraft_render::RenderOptions { outline: app.ui.view.outline, background: None, artboards: false, ..Default::default() };
        let opts = vectorcraft_render::RenderOptions {
            proof: vectorcraft_render::proof::active_proof(),
            overprint_preview: vectorcraft_render::proof::overprint_preview_on(),
            trim: app.ui.view.trim_view,
            tile_edge: vectorcraft_color::Color::from_hex(&app.session.prefs.pattern_tile_edge_color).map_or(opts.tile_edge, |c| {
                let [r, g, b, _] = c.to_rgba8(1.0);
                [r, g, b]
            }),
            mask_view,
            highlight_substitutions: true,
            // General › Anti-aliased Artwork: off, edges are hard on screen (raster effects and
            // pattern tiles stay smooth), as in Illustrator.
            anti_alias: if app.session.prefs.anti_aliased_artwork { vectorcraft_render::AntiAlias::Art } else { vectorcraft_render::AntiAlias::None },
            progressive_placed: true,
            trace_views: true,
            smooth_images,
            // Isolation mode: the art around the isolated group or layer is dimmed (#833).
            isolated: key.isolated.map(vectorcraft_doc::NodeId),
            ..opts
        };
        // Light documents render synchronously (no lag vs overlays); heavy ones go to the worker.
        // A document switch renders synchronously so another document's frame is never shown.
        let same_doc = app.canvas.key.as_ref().is_some_and(|k| k.doc == key.doc);
        let heavy = app.canvas.last_ms > 8.0 && app.canvas.texture.is_some() && same_doc;
        match (&mut app.canvas.worker, heavy) {
            (Some(worker), true) => worker.submit(crate::render_worker::Job { key: key.clone(), doc: doc.clone(), w, h, view, opts }),
            _ => {
                let t0 = now_ms();
                let img = app.canvas.renderer.render(&doc, w, h, view, &opts);
                upload(app, ui.ctx(), &img, pixel.is_some());
                app.canvas.key = Some(key.clone());
                app.perf.render_ms = now_ms() - t0;
                app.canvas.last_ms = app.perf.render_ms;
            }
        }
    }
    if let (Some(tex), Some(k)) = (&app.canvas.texture, &app.canvas.key)
        && k.doc == key.doc
    {
        match k.pixel {
            // Document pixels: placed over the document rect they cover, turning with the view.
            Some([x0, y0, x1, y1]) => {
                let q = xf.quad(Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64));
                let mut mesh = egui::Mesh::with_texture(tex.id());
                for (pos, uv) in q.into_iter().zip([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]) {
                    mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(Shape::mesh(mesh));
            }
            None if (k.rot - v.rotation).abs() < 1e-9 => {
                // Reproject the last frame if it was rendered for a different view.
                let old = Xf { rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: xf.rot };
                let a = xf.to_screen(old.to_doc(rect.min));
                let b = xf.to_screen(old.to_doc(rect.max));
                painter.image(tex.id(), egui::Rect::from_min_max(a, b), egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {}
        }
    }
    if let Some(look) = grid_look.filter(|_| !grids_in_back) {
        grid(&painter, &xf, doc.grid.spacing, doc.grid.subdivisions, look);
    }
    // The pixel grid (Guides & Grid › Show Pixel Grid (Above 600% Zoom)): in Pixel Preview from
    // 600% zoom, a line at every document pixel over the art, so the pixels it rasterizes to show.
    if app.ui.view.pixel_preview && v.zoom >= PIXEL_GRID_ZOOM && app.session.prefs.show_pixel_grid {
        grid(&painter, &xf, 1.0, 1, LineLook { color: PIXEL_GRID, dots: false });
    }
    // Artboard edges and names. The Artboard tool labels its active artboard itself (following
    // a move or resize): that one isn't named twice.
    let active_ab = 0;
    let artboard_tool = app.session.tool_id() == "artboard";
    let tool_labelled = artboard_tool.then(|| app.session.tool_options()["active"].as_u64()).flatten();
    for (i, ab) in doc.artboards.iter().enumerate() {
        let r = xf.rect_to_screen(ab.rect);
        let c = if i == active_ab { Color32::from_gray(0) } else { Color32::from_gray(120) };
        painter.add(Shape::closed_line(xf.quad(ab.rect), Stroke::new(if i == active_ab { 1.0 } else { 0.6 }, c)));
        // The bleed (Document Setup) as a red outline around the artboard.
        if doc.setup.has_bleed() {
            painter.add(Shape::closed_line(xf.quad(doc.setup.bleed_rect(ab.rect)), Stroke::new(1.0, t.bleed)));
        }
        if (artboard_tool || doc.artboards.len() > 1) && tool_labelled != u64::try_from(i).ok() {
            painter.text(
                r.left_top() - vec2(0.0, 4.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:02} - {}", i + 1, ab.name),
                egui::FontId::proportional(11.0),
                t.text_dim,
            );
        }
    }
    if app.ui.view.guides {
        let look = LineLook::guides(&app.session.prefs, &t);
        // Selected guides in the selection colour.
        let picked = LineLook { color: t.selection, ..look };
        let selected: &[usize] = app.session.active().map_or(&[], |st| &st.selection.guides);
        for (i, g) in doc.guides.iter().enumerate() {
            let at = |along: f64| xf.to_screen(if g.vertical { Point::new(g.pos, along) } else { Point::new(along, g.pos) });
            // An artboard guide runs across its artboard, a canvas guide across the window.
            let (a, b) = match doc.guide_span(g) {
                Some((from, to)) => (at(from), at(to)),
                None if g.vertical => {
                    let x = at(0.0).x;
                    (pos2(x, rect.top()), pos2(x, rect.bottom()))
                }
                None => {
                    let y = at(0.0).y;
                    (pos2(rect.left(), y), pos2(rect.right(), y))
                }
            };
            if selected.contains(&i) { picked } else { look }.guide(&painter, a, b);
        }
    }

    if !app.session.slices_hidden() {
        slice_overlay(app, &painter, &xf);
    }
    if app.session.active().is_some_and(|d| d.print_tiling) || app.session.tool_id() == "printTiling" {
        print_tiling_overlay(app, &painter, &xf);
    }

    // Selection visuals and tool overlays.
    if app.ui.view.edges {
        hover_highlight(app, &painter, &xf);
        selection_overlay(app, &painter, &xf);
        if app.ui.view.text_threads {
            thread_overlay(app, &painter, &xf);
        }
        if app.ui.view.hidden_chars {
            hidden_chars_overlay(app, &painter, &xf);
        }
    }
    let view_info = app.view_info();
    // View → Hide Gradient Annotator hides the Gradient tool's annotator.
    if app.session.tool_id() != "gradient" || app.ui.view.gradient_annotator {
        let overlays = app.session.overlays(view_info);
        draw_overlays(&painter, &xf, &overlays, &t, HandleLook::of(&app.session.prefs));
    }
    ime_output(app, ui.ctx(), &xf);
    crate::place::paint_drop_highlight(app, ui.ctx(), &painter, rect);

    if app.ui.view.rulers && app.ui.screen_mode < 3 {
        rulers(ui, full, &xf, app.hover_doc, app.session.general_unit(), &t);
    }
    if app.ui.task_bar && !app.session.tool_busy() && app.ui.screen_mode < 3 {
        task_bar(app, ui, &xf);
    }
    if app.ui.screen_mode < 3 {
        crate::free_transform::show(app, ui, xf.rect);
    }
    // Cursor.
    if resp.hovered() {
        let m = ui.input(|i| i.modifiers);
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let panning = matches!(ui.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Pan { .. }));
        let zooming = if space { m.command } else { app.session.tool_id() == "zoom" };
        let cur = if panning {
            egui::CursorIcon::Grabbing
        } else if zooming {
            if m.alt { egui::CursorIcon::ZoomOut } else { egui::CursorIcon::ZoomIn }
        } else if space || app.session.tool_id() == "hand" {
            egui::CursorIcon::Grab
        } else if let Some(p) = app.hover_doc {
            let c = app.session.cursor(p, mods(m, space), view_info);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("tool-cursor")));
            // The loaded place cursor carries the file's thumbnail.
            if app.session.tool_id() == "place"
                && let Some(hp) = ui.input(|i| i.pointer.hover_pos())
            {
                crate::place::paint_cursor(app, ui.ctx(), &painter, hp);
            }
            // Caps Lock gives precise (crosshair) cursors, like Illustrator; env opt-out for system cursors.
            let custom = std::env::var_os("VECTORCRAFT_SYSTEM_CURSORS").is_none();
            // User Interface › Scale Cursor Proportional to UI: the glyphs grow with UI Scaling;
            // off, they keep their size.
            let ui_scale = if app.session.prefs.scale_cursor_with_ui { 1.0 } else { ui.ctx().zoom_factor() };
            match ui.input(|i| i.pointer.hover_pos()) {
                // An OS cursor: the system moves it at once, where a painted one trails the pointer
                // (#444). The system cursor stands in for cursors without a glyph.
                Some(_) if custom && crate::cursors::OS_CURSORS => {
                    let ppp = ui.ctx().pixels_per_point() / ui_scale;
                    ui.ctx().set_cursor_image(app.canvas.cursors.get(c, ppp));
                    cursor_icon(c)
                }
                Some(hp) if custom && crate::cursors::paint(&painter, c, hp, 1.0 / ui_scale) => egui::CursorIcon::None,
                _ => cursor_icon(c),
            }
        } else {
            egui::CursorIcon::Default
        };
        ui.ctx().set_cursor_icon(cur);
    }
}

fn cursor_icon(c: Cursor) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    match c {
        Cursor::Arrow | Cursor::ArrowHollow | Cursor::CornerRadius | Cursor::PathBracket | Cursor::TypeWidget => C::Default,
        Cursor::Move => C::Move,
        Cursor::Crosshair => C::Crosshair,
        Cursor::ResizeH => C::ResizeHorizontal,
        Cursor::ResizeV => C::ResizeVertical,
        Cursor::ResizeNwSe => C::ResizeNwSe,
        Cursor::ResizeNeSw => C::ResizeNeSw,
        Cursor::Rotate => C::Alias,
        Cursor::Pen | Cursor::PenAdd | Cursor::PenDelete | Cursor::PenClose | Cursor::PenContinue | Cursor::PenJoin | Cursor::PenConvert => {
            C::Crosshair
        }
        Cursor::Text => C::Text,
        Cursor::Hand => C::Grab,
        Cursor::HandGrab => C::Grabbing,
        Cursor::ZoomIn => C::ZoomIn,
        Cursor::ZoomOut => C::ZoomOut,
        Cursor::Eyedropper => C::Crosshair,
        Cursor::NotAllowed => C::NotAllowed,
        Cursor::AddStop => C::Copy,
        Cursor::RemoveStop => C::NotAllowed,
        Cursor::Slice => C::Crosshair,
        Cursor::SliceSelect => C::Default,
        Cursor::Width | Cursor::WidthAdd => C::Crosshair,
        Cursor::WidthPoint => C::Move,
        Cursor::Blend | Cursor::BlendObject | Cursor::BlendAnchor | Cursor::ShapeBuilder | Cursor::ShapeBuilderErase => C::Crosshair,
    }
}

fn handle_input(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) {
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let wheel_zooms = app.session.prefs.zoom_with_mouse_wheel;
    let turn_id = egui::Id::new("canvas-wheel-turn");
    let turn_before: WheelTurn = ui.data(|d| d.get_temp(turn_id)).unwrap_or_default();
    let mut turn = turn_before;
    let (pointer, m, space, (factor, scroll)) =
        ui.input(|i| (i.pointer.clone(), i.modifiers, i.key_down(egui::Key::Space), wheel(i, wheel_zooms, line, rect.height(), &mut turn)));
    if turn != turn_before {
        ui.data_mut(|d| d.insert_temp(turn_id, turn));
    }
    if turn.glide != 0.0 {
        ui.ctx().request_repaint();
    }
    let v = *app.view().unwrap_or(&View::default());
    let xf = Xf::new(rect, &v);
    let hover = pointer.hover_pos().filter(|p| rect.contains(*p));
    // A lifted pen or finger takes the pointer away in the frame it lifts (egui's `PointerGone`),
    // so a tap whose press and lift come in one frame has no hover position: it presses (and
    // double-taps) where it was seen last (#491).
    let at = hover.or_else(|| pointer.interact_pos().filter(|p| rect.contains(*p)));
    app.hover_doc = hover.map(|p| xf.to_doc(p));
    let view = app.view_info();
    touch_gestures(app, ui, rect, view);
    let drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id()));
    // A modifier pressed or released over the canvas re-hovers the tool, so what it changes shows
    // without moving the mouse (Alt switches the Shape Builder to erase mode).
    let mods_changed = ui.data_mut(|d| {
        let id = egui::Id::new("canvas-mods");
        let prev = d.get_temp::<egui::Modifiers>(id);
        d.insert_temp(id, m);
        prev.is_some_and(|prev| prev != m)
    });

    // Zoom around the pointer, and scroll ([`wheel`]): fingers pinching on a touch screen may also
    // slide.
    if resp.hovered() {
        if (factor - 1.0).abs() > 1e-6
            && let (Some(p), Some(vm)) = (hover, app.view_mut())
        {
            zoom_about(vm, rect, p, vm.zoom * factor);
        }
        if scroll != egui::Vec2::ZERO
            && let Some(vm) = app.view_mut()
        {
            let d = Xf { rect, zoom: vm.zoom, center: vm.center, rot: vm.rotation.to_radians() }.delta_to_doc(scroll);
            vm.center -= d;
        }
    }

    let tool = app.session.tool_id();
    // Held, Space is the Hand tool for the moment, and Cmd+Space the Zoom tool (with Alt, zooming
    // out), whatever the tool.
    let zoom_mode = if space { m.command } else { tool == "zoom" };
    let pan_mode = if space { !m.command } else { tool == "hand" };
    let middle_pan = matches!(drag, Some(Drag::Pan { middle: true, .. }));
    // egui counts a press a few pixels outside the canvas as on it (its interaction radius): only a
    // press on the canvas itself reaches the tools, else it would land at the canvas's centre.
    if pointer.primary_pressed()
        && resp.hovered()
        && !middle_pan
        && let Some(p) = at
    {
        ui.ctx().memory_mut(|mem| mem.stop_text_input());
        app.ui.flyout = None;
        let d = if zoom_mode {
            if animated_zoom(&app.session.prefs) { Drag::ZoomScrub { start: p, held: 0.0, moved: false } } else { Drag::ZoomBox { start: p } }
        } else if pan_mode {
            Drag::Pan { start: p, center: v.center, middle: false }
        } else if tool == "rotateView" {
            let c = rect.center();
            Drag::RotateView { start_angle: (p.y - c.y).atan2(p.x - c.x) as f64, start_rot: v.rotation }
        } else {
            // Cmd with another tool drags with the selection tool used last (the session lends it).
            let ev = PointerEvent { kind: PointerKind::Down, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, true) };
            dispatch(app, &ev, view);
            Drag::Tool
        };
        ui.data_mut(|dd| {
            dd.insert_temp(drag_id(), d);
            dd.insert_temp(drag_pos_id(), p);
        });
    } else if drag.is_none()
        && resp.hovered()
        && pointer.button_pressed(egui::PointerButton::Middle)
        && let Some(start) = hover
    {
        // Middle-button drag pans the view whatever the tool.
        ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Pan { start, center: v.center, middle: true }));
    } else if let Some(d) = drag {
        // The pointer gone (a pen lifted with its press), the drag ends where it was last seen.
        let p = pointer.interact_pos().or_else(|| ui.data(|dd| dd.get_temp(drag_pos_id()))).unwrap_or(rect.center());
        let held = if middle_pan { pointer.button_down(egui::PointerButton::Middle) } else { pointer.primary_down() };
        if held {
            ui.data_mut(|dd| dd.insert_temp(drag_pos_id(), p));
            match d {
                Drag::Pan { start, center, .. } => {
                    let d = xf.delta_to_doc(p - start);
                    if let Some(vm) = app.view_mut() {
                        vm.center = center - d;
                    }
                }
                Drag::RotateView { start_angle, start_rot } => {
                    let c = rect.center();
                    let a = (p.y - c.y).atan2(p.x - c.x) as f64;
                    let mut deg = start_rot + (a - start_angle).to_degrees();
                    if m.shift {
                        deg = (deg / 15.0).round() * 15.0;
                    }
                    if let Some(vm) = app.view_mut() {
                        vm.rotation = vectorcraft_geom::normalize_deg(deg);
                    }
                }
                Drag::ZoomBox { start } => marquee(ui.painter(), egui::Rect::from_two_pos(start, p)),
                Drag::ZoomScrub { start, held, moved } => {
                    let dt = ui.input(|i| i.unstable_dt).min(0.25);
                    let (held, moved) = (held + dt, moved || (p - start).length() > SCRUB_SLOP);
                    // Dragged sideways it zooms with the pointer, held still it zooms on (Alt: out).
                    let exponent = if moved {
                        f64::from(pointer.delta().x) * SCRUB_ZOOM
                    } else if held > HOLD_DELAY {
                        f64::from(dt) * HOLD_ZOOM * if m.alt { -1.0 } else { 1.0 }
                    } else {
                        0.0
                    };
                    if exponent != 0.0
                        && let Some(vm) = app.view_mut()
                    {
                        zoom_about(vm, rect, start, vm.zoom * exponent.exp());
                    }
                    ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::ZoomScrub { start, held, moved }));
                }
                Drag::Art | Drag::Gesture => {}
                Drag::Tool if drag_art_out(app, ui, resp, p, view) => {
                    ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Art));
                }
                Drag::Tool => {
                    if pointer.delta() != egui::Vec2::ZERO {
                        let ev = PointerEvent { kind: PointerKind::Drag, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, false) };
                        dispatch(app, &ev, view);
                    }
                    // Time held (Twirl, Pucker and Bloat keep applying); a stalled frame counts
                    // a quarter second at most.
                    if app.session.tool_wants_ticks() {
                        let dt = f64::from(ui.input(|i| i.unstable_dt).min(0.25));
                        let r = app.session.tool_tick(dt, view);
                        apply_requests(app, r);
                    }
                }
            }
        } else {
            ui.data_mut(|dd| {
                dd.remove::<Drag>(drag_id());
                dd.remove::<Pos2>(drag_pos_id());
            });
            match d {
                Drag::ZoomBox { start } => {
                    let r = egui::Rect::from_two_pos(start, p);
                    if let Some(vm) = app.view_mut() {
                        if r.width() > 8.0 && r.height() > 8.0 {
                            let a = xf.to_doc(r.min);
                            let b = xf.to_doc(r.max);
                            vm.center = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                            vm.zoom = (rect.width() as f64 / (b.x - a.x)).min(rect.height() as f64 / (b.y - a.y)).clamp(0.0313, 640.0);
                        } else {
                            zoom_about(vm, rect, p, crate::state::next_zoom(vm.zoom, !m.alt));
                        }
                    }
                }
                // A click (not held, not dragged) steps the zoom as with the marquee.
                Drag::ZoomScrub { held, moved: false, .. } if held <= HOLD_DELAY => {
                    if let Some(vm) = app.view_mut() {
                        zoom_about(vm, rect, p, crate::state::next_zoom(vm.zoom, !m.alt));
                    }
                }
                Drag::ZoomScrub { .. } => {}
                Drag::Tool => {
                    let ev = PointerEvent { kind: PointerKind::Up, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, false) };
                    dispatch(app, &ev, view);
                }
                Drag::Art | Drag::Gesture => {}
                Drag::Pan { .. } | Drag::RotateView { .. } => {}
            }
        }
    } else if let Some(p) = hover
        && (pointer.is_moving() || mods_changed)
    {
        let ev = PointerEvent { kind: PointerKind::Move, pos: xf.to_doc(p), mods: mods(m, space), pressure: 1.0 };
        dispatch(app, &ev, view);
    }
    if resp.double_clicked()
        && let Some(p) = at
    {
        let ev = PointerEvent { kind: PointerKind::DoubleClick, pos: xf.to_doc(p), mods: mods(m, space), pressure: 1.0 };
        let before = app.session.tool_id().to_string();
        dispatch(app, &ev, view);
        // A double-click on type switched a selection tool to the Type tool: the caret goes where
        // the type was clicked, so typing edits it at once.
        if before != "type" && app.session.tool_id() == "type" {
            for kind in [PointerKind::Down, PointerKind::Up] {
                dispatch(app, &PointerEvent { kind, ..ev }, view);
            }
        }
    }
    if drag.is_some() || pointer.is_moving() || mods_changed {
        ui.ctx().request_repaint();
    }
}

/// How much one point of wheel motion zooms (as `exp(points × WHEEL_ZOOM)`).
const WHEEL_ZOOM: f64 = 0.01;
/// How fast a wheel notch's zoom glides in with Zoom with Mouse Wheel: the time constant (s) of
/// the part still to come.
const WHEEL_GLIDE: f64 = 0.05;

/// What the wheel did across frames ([`wheel`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct WheelTurn {
    /// The last wheel turn was an Alt-wheel one.
    alt: bool,
    /// The zoom (natural log) wheel notches still have to glide in.
    glide: f64,
}
/// Animated Zoom: how much one point of sideways drag zooms (100 points to the right double it).
const SCRUB_ZOOM: f64 = std::f64::consts::LN_2 / 100.0;
/// Animated Zoom: how far the pointer moves before a press is a drag, in points.
const SCRUB_SLOP: f32 = 3.0;
/// Animated Zoom: how long a press is held still before it zooms on, in seconds…
const HOLD_DELAY: f32 = 0.3;
/// …and how fast it zooms then (as `exp(seconds × HOLD_ZOOM)`: twice as close each second).
const HOLD_ZOOM: f64 = std::f64::consts::LN_2;

/// Performance › Animated Zoom, which needs GPU Performance as in Illustrator: the Zoom tool zooms
/// as it is dragged sideways or held, instead of zooming to the area dragged across.
pub(crate) fn animated_zoom(p: &vectorcraft_engine::Prefs) -> bool {
    p.animated_zoom && p.gpu_performance
}

/// Zoom for View → Actual Size (`view.actualSize`). With Preferences › General › Display Print
/// Size at 100% Zoom off, one document point is one screen point. On, one document inch (72 pt)
/// fills an inch of the screen as the system counts it: 96 screen points, the reference density
/// that display scaling is set against (a CSS inch), whatever the scale factor.
pub(crate) fn actual_size_zoom(display_print_size: bool) -> f64 {
    if display_print_size { 96.0 / 72.0 } else { 1.0 }
}

/// Zoom view `vm` of the canvas `rect` to `zoom` (clamped to the zoom range), keeping the document
/// point under screen point `p` where it is.
fn zoom_about(vm: &mut View, rect: egui::Rect, p: Pos2, zoom: f64) {
    let before = Xf::new(rect, vm).to_doc(p);
    vm.zoom = zoom.clamp(0.0313, 640.0);
    vm.center += before - Xf::new(rect, vm).to_doc(p);
}

/// A marquee around screen rectangle `r`: dark dashes over a light line, so it shows on a white
/// artboard and on the grey pasteboard alike (#725).
fn marquee(p: &egui::Painter, r: egui::Rect) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.add(Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::from_white_alpha(230))));
    for w in pts.windows(2) {
        p.extend(Shape::dashed_line(&[w[0], w[1]], Stroke::new(1.0, Color32::from_gray(40)), 3.0, 3.0));
    }
}

/// What the wheel, a pinch and a two-finger drag on a touch screen did over the canvas this frame:
/// a zoom factor (about the pointer) and a scroll (screen points the content moves). The wheel scrolls, Cmd- and Alt-wheel (Option
/// on the Mac) zoom. With General › Zoom with Mouse Wheel (`wheel_zooms`) the wheel and Alt-wheel
/// zoom, Shift-wheel scrolls up and down and Cmd/Ctrl-wheel sideways. `line` and `page`: points
/// per wheel line and page. `turn`: kept by the caller across frames. A mouse wheel's notches zoom
/// in a glide over a few frames, as scrolling does (#888); a trackpad's fine steps at once.
fn wheel(i: &egui::InputState, wheel_zooms: bool, line: f32, page: f32, turn: &mut WheelTurn) -> (f64, egui::Vec2) {
    // Fingers on a touch screen pan the content with them (a trackpad sends scrolls instead).
    let pan = i.multi_touch().map_or(egui::Vec2::ZERO, |t| t.translation_delta);
    if !wheel_zooms {
        // egui makes Cmd-wheel (and a pinch) its zoom and the rest a scroll it spreads over a few
        // frames: the rest of an Alt-wheel turn zooms too, however soon Alt is let go.
        if let Some(m) = i.events.iter().rev().find_map(|e| if let egui::Event::MouseWheel { modifiers, .. } = e { Some(modifiers) } else { None }) {
            turn.alt = m.alt && !m.command;
        }
        let (zoom, scroll) = (f64::from(i.zoom_delta()), i.smooth_scroll_delta);
        return if turn.alt && scroll != egui::Vec2::ZERO {
            (zoom * (f64::from(scroll.x + scroll.y) * WHEEL_ZOOM).exp(), pan)
        } else {
            (zoom, scroll + pan)
        };
    }
    // The wheel events themselves: egui's own handling turns Cmd-wheel into a zoom.
    let mut zoom = i.multi_touch().map_or(1.0, |t| f64::from(t.zoom_delta));
    let mut scroll = pan;
    for e in &i.events {
        match e {
            egui::Event::MouseWheel { unit, delta, modifiers, .. } => {
                let d = match unit {
                    egui::MouseWheelUnit::Point => *delta,
                    egui::MouseWheelUnit::Line => *delta * line,
                    egui::MouseWheelUnit::Page => *delta * page,
                };
                if modifiers.command {
                    scroll.x += d.x + d.y;
                } else if modifiers.shift {
                    scroll.y += d.x + d.y;
                } else if *unit == egui::MouseWheelUnit::Point {
                    zoom *= (f64::from(d.y) * WHEEL_ZOOM).exp();
                    scroll.x += d.x;
                } else {
                    turn.glide += f64::from(d.y) * WHEEL_ZOOM;
                    scroll.x += d.x;
                }
            }
            egui::Event::Zoom(f) => zoom *= f64::from(*f),
            _ => {}
        }
    }
    // This frame's part of the glide: what is left shrinks by e every WHEEL_GLIDE seconds.
    let part = if turn.glide.abs() < 1e-3 { 1.0 } else { 1.0 - (-f64::from(i.stable_dt.min(0.1)) / WHEEL_GLIDE).exp() };
    let step = turn.glide * part;
    turn.glide -= step;
    (zoom * step.exp(), scroll)
}

/// Enable Touch Gestures: this frame's finger contacts on the canvas ([`crate::touch`]). A second
/// finger drops the press the first one began (its tool interaction rolled back; the rest of the
/// gesture presses nothing), and a tap of two fingers undoes, one of three redoes.
fn touch_gestures(app: &mut VectorcraftApp, ui: &Ui, rect: egui::Rect, view: vectorcraft_engine::ViewInfo) {
    if !app.session.prefs.touch_gestures {
        return;
    }
    let (touches, now): (Vec<_>, f64) = ui.input(|i| {
        let t = i.events.iter().filter_map(|e| match e {
            egui::Event::Touch { id, phase, pos, .. } => Some((*id, *phase, *pos)),
            _ => None,
        });
        (t.collect(), i.time)
    });
    if touches.is_empty() {
        return;
    }
    let key = egui::Id::new("canvas-touch");
    let mut taps: crate::touch::Taps = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    let mut tap = None;
    for (id, phase, pos) in touches {
        // A gesture begins on the canvas.
        if phase == egui::TouchPhase::Start && !taps.is_down() && !rect.contains(pos) {
            continue;
        }
        match taps.feed(id, phase, pos, now) {
            Some(crate::touch::Touch::Fingers) => {
                if ui.data(|d| d.get_temp::<Drag>(drag_id())) == Some(Drag::Tool) {
                    // Nothing it began is kept; the tool's mouse-up returns it to rest.
                    let _ = app.session.cancel_interaction();
                    dispatch(app, &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), pressure: 1.0 }, view);
                }
                ui.data_mut(|d| d.insert_temp(drag_id(), Drag::Gesture));
            }
            Some(crate::touch::Touch::Tap(n)) => tap = crate::touch::tap_command(n),
            None => {}
        }
    }
    ui.data_mut(|d| d.insert_temp(key, taps));
    if let Some(cmd) = tap {
        crate::menus::invoke(app, cmd, serde_json::json!({}));
    }
}

/// A Selection tool move dragged off the canvas (to `p`, over a panel) turns into a panel drag of
/// the selected art ([`widgets::PanelDrag::Art`]): the move is dropped, so the art stays where it
/// was, and the panel it is released on takes it (the Graphic Styles panel makes a style of it).
fn drag_art_out(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, p: Pos2, view: vectorcraft_engine::ViewInfo) -> bool {
    let off = !ui.clip_rect().contains(p) || ui.ctx().layer_id_at(p).is_some_and(|l| l != resp.layer_id);
    let Some(st) = app.session.active().filter(|_| off && app.session.tool_id() == "selection") else { return false };
    // The Selection tool's interaction while it moves (or Alt-copies) the selection.
    if !st.interaction.as_ref().is_some_and(|i| i.label == "Move" || i.label == "Copy") {
        return false;
    }
    let ids = st.selection.objects.clone();
    if app.session.cancel_interaction().is_err() {
        return false;
    }
    // With nothing left to commit, the tool's mouse-up just returns it to rest.
    dispatch(app, &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), pressure: 1.0 }, view);
    egui::DragAndDrop::set_payload(ui.ctx(), widgets::PanelDrag::Art(ids));
    true
}

/// Send a pointer event to the active tool and act on UI requests (dialogs, tool switches).
pub fn dispatch(app: &mut VectorcraftApp, ev: &PointerEvent, view: vectorcraft_engine::ViewInfo) {
    let r = app.session.pointer(ev, view);
    // The artboard the Artboard tool makes active is the active one (the navigator's, the
    // Artboards panel's).
    if app.session.tool_id() == "artboard"
        && let Some(i) = app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok())
        && let Some(v) = app.view_mut()
    {
        v.artboard = i;
    }
    // A press with a selection tool inside an artboard makes it the active one (#693), as the
    // Artboard tool's does: Paste in Place and in Front or Back then paste onto it.
    if ev.kind == PointerKind::Down
        && vectorcraft_tools::catalog::is_selection_tool(app.session.tool_id())
        && let Some(i) = app.session.active().and_then(|d| d.doc.artboard_at(ev.pos))
        && let Some(v) = app.view_mut()
    {
        v.artboard = i;
    }
    apply_requests(app, r);
}

/// Act on what a tool event asked the UI for (dialogs, tool switches), or show its error.
pub fn apply_requests(app: &mut VectorcraftApp, r: vectorcraft_engine::Result<Vec<vectorcraft_engine::UiRequest>>) {
    match r {
        Ok(reqs) => {
            for r in reqs {
                match r {
                    vectorcraft_engine::UiRequest::Dialog(kind, p) => crate::dialogs::open_tool_dialog(app, &kind, p),
                    vectorcraft_engine::UiRequest::SwitchTool(t) => app.select_tool(&t),
                    vectorcraft_engine::UiRequest::Status(msg) => app.status(msg),
                }
            }
        }
        Err(e) => app.status(e.to_string()),
    }
}

/// The transparency grid's 2 × 2-cell tile in the document's colours (Document Setup), cached per
/// look, and its cell size in screen points.
pub(crate) fn checker_texture(ctx: &egui::Context, setup: &vectorcraft_doc::DocSetup) -> (egui::TextureHandle, f32) {
    let colors = setup.grid_colors.map(|c| crate::panels::c32(&c));
    let id = egui::Id::new("transparency-grid-tile");
    let cached = ctx.data(|d| d.get_temp::<([Color32; 2], egui::TextureHandle)>(id)).filter(|(c, _)| *c == colors);
    let tex = cached.map(|(_, t)| t).unwrap_or_else(|| {
        let [a, b] = colors;
        let img = egui::ColorImage::new([2, 2], vec![a, b, b, a]);
        let opts = egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::NEAREST };
        let t = ctx.load_texture(TRANSPARENCY_GRID, img, opts);
        ctx.data_mut(|d| d.insert_temp(id, (colors, t.clone())));
        t
    });
    (tex, setup.grid_size.cell())
}

/// Name of the transparency grid's texture.
pub(crate) const TRANSPARENCY_GRID: &str = "transparency grid";

/// The transparency grid over the artboard `ab` (screen corners `quad`): one textured quad whose
/// cells stay `cell` screen points at every zoom and turn with a rotated view.
fn checker(p: &egui::Painter, quad: &[Pos2], ab: Rect, zoom: f64, tex: egui::TextureId, cell: f32) {
    let mut mesh = egui::Mesh::with_texture(tex);
    // Two cells per texture repeat.
    let per = (2.0 * cell as f64 / zoom).max(1e-9);
    let (u, v) = ((ab.width() / per) as f32, (ab.height() / per) as f32);
    for (pos, uv) in quad.iter().zip([pos2(0.0, 0.0), pos2(u, 0.0), pos2(u, v), pos2(0.0, v)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: *pos, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    p.add(Shape::mesh(mesh));
}

/// The zoom the pixel grid shows from in Pixel Preview (600%), and its colour: a translucent grey,
/// so the art's pixels read through it.
const PIXEL_GRID_ZOOM: f64 = 6.0;
const PIXEL_GRID: Color32 = Color32::from_rgba_premultiplied(64, 64, 64, 96);

/// The look of the grid or of the guides (Preferences › Guides & Grid › Color and Style).
#[derive(Clone, Copy)]
struct LineLook {
    color: Color32,
    dots: bool,
}

impl LineLook {
    fn grid(prefs: &vectorcraft_engine::Prefs) -> Self {
        Self { color: pref_color(&prefs.grid_color, Color32::from_gray(200)), dots: prefs.grid_style == "dots" }
    }

    fn guides(prefs: &vectorcraft_engine::Prefs, t: &Tokens) -> Self {
        Self { color: pref_color(&prefs.guide_color, t.guide), dots: prefs.guide_style == "dots" }
    }

    /// A guide from `a` to `b`: a line, or dots.
    fn guide(self, p: &egui::Painter, a: Pos2, b: Pos2) {
        if self.dots {
            p.extend(Shape::dotted_line(&[a, b], self.color, 4.0, 0.75));
        } else {
            p.line_segment([a, b], Stroke::new(1.0, self.color));
        }
    }
}

/// A `#rrggbb` colour preference, `fallback` when it doesn't parse.
fn pref_color(hex: &str, fallback: Color32) -> Color32 {
    vectorcraft_color::Color::from_hex(hex).map_or(fallback, |c| crate::panels::c32(&c))
}

/// The grid (View → Show Grid): a gridline every `spacing` and `subdiv` lighter lines between
/// (left out when they would crowd). In the Dots style, a dot where gridlines cross the
/// subdivisions instead.
fn grid(p: &egui::Painter, xf: &Xf, spacing: f64, subdiv: u32, look: LineLook) {
    let r = xf.rect;
    // The view in document space: the box round the canvas's corners (the view may be rotated;
    // the painter clips to the canvas).
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|c| xf.to_doc(c));
    let view = corners.iter().fold(Rect::from_points(corners[0], corners[0]), |b, c| b.union_pt(*c));
    let sub = spacing / subdiv.max(1) as f64;
    let step = if sub * xf.zoom >= 6.0 { sub } else { spacing };
    // Dots crowd sooner than lines.
    if step * xf.zoom < 4.0 || (look.dots && spacing * xf.zoom < 8.0) {
        return;
    }
    let major = |v: f64| (v / spacing - (v / spacing).round()).abs() < 1e-6;
    // The lines across the view (where along their axis), at least 4 px apart (and capped).
    let lines = |from: f64, to: f64| {
        let first = (from / step).floor();
        (0..10_000u32).map(move |i| (first + f64::from(i)) * step).take_while(move |v| *v <= to)
    };
    if look.dots {
        let mut mesh = egui::Mesh::default();
        for x in lines(view.x0, view.x1) {
            for y in lines(view.y0, view.y1).filter(|y| major(x) || major(*y)) {
                mesh.add_colored_rect(egui::Rect::from_center_size(xf.to_screen(Point::new(x, y)), vec2(1.5, 1.5)), look.color);
            }
        }
        p.add(Shape::mesh(mesh));
        return;
    }
    let minor = look.color.gamma_multiply(0.4);
    let stroke = |major: bool| Stroke::new(1.0, if major { look.color } else { minor });
    for x in lines(view.x0, view.x1) {
        p.line_segment([xf.to_screen(Point::new(x, view.y0)), xf.to_screen(Point::new(x, view.y1))], stroke(major(x)));
    }
    for y in lines(view.y0, view.y1) {
        p.line_segment([xf.to_screen(Point::new(view.x0, y)), xf.to_screen(Point::new(view.x1, y))], stroke(major(y)));
    }
}

/// A ruler label for `v` (in the ruler's unit) on a ruler whose labels are `step` apart: whole
/// numbers, or as many decimals as the step has.
pub(crate) fn ruler_label(v: f64, step: f64) -> String {
    let decimals = if step >= 1.0 { 0 } else { (-step.log10()).ceil() as usize };
    let s = format!("{v:.decimals$}");
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') { "0".into() } else { s }
}

/// The rulers, numbered in `unit` (the General unit).
/// The top ruler, the left ruler and the box where they meet.
fn ruler_rects(full: egui::Rect) -> [egui::Rect; 3] {
    let top = egui::Rect::from_min_max(pos2(full.left() + RULER, full.top()), pos2(full.right(), full.top() + RULER));
    let left = egui::Rect::from_min_max(pos2(full.left(), full.top() + RULER), pos2(full.left() + RULER, full.bottom()));
    [top, left, egui::Rect::from_min_size(full.min, vec2(RULER, RULER))]
}

/// A drag from a ruler onto the canvas makes a guide where the button is released (the engine's
/// `Session::ruler_guide`): a horizontal one from the top ruler, a vertical one from the left
/// ruler, snapped as a moved guide is (with Shift to the ruler's ticks). Released anywhere else,
/// it makes none.
fn ruler_guides(app: &mut VectorcraftApp, ui: &Ui, full: egui::Rect, canvas: egui::Rect, xf: &Xf) {
    let [top, left, corner] = ruler_rects(full);
    // Right-click on a ruler or the origin box: the document units, to swap between them as
    // Preferences ▸ Units ▸ General does ([`crate::menus::ruler_menu_body`]).
    let mut unit_clicked = None;
    for (r, vertical, id) in [(top, false, "ruler-top"), (left, true, "ruler-left")] {
        // Click-and-drag so the same widget drags out guides (left) and opens the unit menu (right).
        let resp = ui.interact(r, egui::Id::new(id), Sense::click_and_drag());
        resp.context_menu(|ui| crate::menus::ruler_menu_body(app, ui, &mut unit_clicked));
        let kind = if resp.drag_stopped() {
            PointerKind::Up
        } else if resp.dragged() {
            PointerKind::Drag
        } else {
            continue;
        };
        let p = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
        let on_canvas = p.is_some_and(|p| canvas.contains(p));
        if on_canvas {
            // The new guide shows even if guides were hidden.
            app.ui.view.guides = true;
        }
        let ev = PointerEvent { kind, pos: p.map_or(Point::ZERO, |p| xf.to_doc(p)), mods: mods(ui.input(|i| i.modifiers), false), pressure: 1.0 };
        if let Err(e) = app.session.ruler_guide(vertical, &ev, on_canvas, app.view_info()) {
            app.status(e.to_string());
        }
    }
    let corner = ui.interact(corner, egui::Id::new("ruler-corner"), Sense::click());
    corner.context_menu(|ui| crate::menus::ruler_menu_body(app, ui, &mut unit_clicked));
    if let Some((id, p)) = unit_clicked {
        crate::menus::invoke(app, &id, p);
    }
}

fn rulers(ui: &Ui, full: egui::Rect, xf: &Xf, hover: Option<Point>, unit: Unit, t: &Tokens) {
    let p = ui.painter();
    let [top, left, corner] = ruler_rects(full);
    for r in [top, left, corner] {
        p.rect_filled(r, 0.0, t.ruler);
    }
    p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, t.border));
    p.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.border));
    // Crosshair in the origin box.
    p.line_segment([corner.center() - vec2(4.0, 0.0), corner.center() + vec2(4.0, 0.0)], Stroke::new(1.0, t.ruler_tick));
    p.line_segment([corner.center() - vec2(0.0, 4.0), corner.center() + vec2(0.0, 4.0)], Stroke::new(1.0, t.ruler_tick));
    // Pick a label step (in `unit`) that gives ≥ 50 px between labels; positions below are in `unit`
    // (`per` points each).
    let per = unit.points();
    let step = unit.ruler_step(xf.zoom);
    let minor = step / 10.0;
    let font = egui::FontId::proportional(9.5);
    let near = |v: f64, every: f64| ((v / every).round() * every - v).abs() < minor * 0.01;
    // A tick's length: the labelled ones longest, the halves between them shorter.
    let tick = |v: f64| {
        if near(v, step) {
            RULER
        } else if near(v, step / 2.0) {
            7.0
        } else {
            4.0
        }
    };
    // Each ruler measures along its own edge, in the canvas's coordinates turned with the view
    // (#826): at 0° the top one reads x and the left one y. `draw` gets each tick's distance
    // (screen points) from `from` along the ruler, and its value in `unit`.
    let ticks = |from: Pos2, to: Pos2, axis: vectorcraft_geom::Vec2, draw: &mut dyn FnMut(f32, f64)| {
        let along = |s: Pos2| xf.to_doc(s).to_vec2().dot(axis);
        let (a, b) = (along(from), along(to));
        let mut v = (a / per / minor).floor() * minor;
        while v * per <= b {
            draw(((v * per - a) * xf.zoom) as f32, v);
            v += minor;
        }
    };
    // The canvas's directions along the screen's x and y.
    let (sn, cs) = xf.rot.sin_cos();
    let clip_top = p.with_clip_rect(top);
    ticks(top.left_top(), top.right_top(), vectorcraft_geom::Vec2::new(cs, -sn), &mut |d, x| {
        let sx = top.left() + d;
        clip_top.line_segment([pos2(sx, top.bottom() - tick(x)), pos2(sx, top.bottom())], Stroke::new(1.0, t.ruler_tick));
        if near(x, step) {
            clip_top.text(pos2(sx + 2.0, top.top() + 1.0), egui::Align2::LEFT_TOP, ruler_label(x, step), font.clone(), t.ruler_tick);
        }
    });
    let clip_left = p.with_clip_rect(left);
    ticks(left.left_top(), left.left_bottom(), vectorcraft_geom::Vec2::new(sn, cs), &mut |d, y| {
        let sy = left.top() + d;
        clip_left.line_segment([pos2(left.right() - tick(y), sy), pos2(left.right(), sy)], Stroke::new(1.0, t.ruler_tick));
        if near(y, step) {
            // Vertical labels read top-to-bottom, one digit per line like Illustrator.
            for (k, ch) in ruler_label(y, step).chars().enumerate() {
                clip_left.text(
                    pos2(left.left() + 4.0, sy + 2.0 + k as f32 * 8.5),
                    egui::Align2::LEFT_TOP,
                    ch.to_string(),
                    font.clone(),
                    t.ruler_tick,
                );
            }
        }
    });
    if let Some(h) = hover {
        let s = xf.to_screen(h);
        clip_top.line_segment([pos2(s.x, top.top()), pos2(s.x, top.bottom())], Stroke::new(1.0, t.text));
        clip_left.line_segment([pos2(left.left(), s.y), pos2(left.right(), s.y)], Stroke::new(1.0, t.text));
    }
}

fn to_screen_path(bp: &BezPath, xf: &Xf) -> Vec<Vec<Pos2>> {
    // Flatten for drawing: a polyline per subpath (tolerance ~0.25 screen px).
    let mut out: Vec<Vec<Pos2>> = vec![];
    let a = xf.affine();
    let mut t = bp.clone();
    t.apply_affine(a);
    let mut cur: Vec<Pos2> = vec![];
    kurbo_flatten(&t, 0.25, &mut |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() > 1 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            cur.push(pos2(p.x as f32, p.y as f32));
        }
        PathEl::LineTo(p) => cur.push(pos2(p.x as f32, p.y as f32)),
        PathEl::ClosePath => {
            if let Some(f) = cur.first().copied() {
                cur.push(f);
            }
        }
        _ => {}
    });
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

fn stroke_path(p: &egui::Painter, bp: &BezPath, xf: &Xf, s: Stroke) {
    for line in to_screen_path(bp, xf) {
        p.add(Shape::line(line, s));
    }
}

fn c32(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Outline of a node for highlighting (paths, compound children, area type frames, other
/// text/image bounds).
fn node_outline(n: &Node) -> BezPath {
    let mut bp = BezPath::new();
    walk_drawn(n, &mut |c| match &c.kind {
        NodeKind::Path { path, .. } => bp.extend(path.to_bezpath()),
        // Area type shows its frame: the type area Direct Selection reshapes; type on a path its
        // path, which the selection tools' brackets sit on.
        NodeKind::Text(t) if c.perspective.is_none() && !matches!(t.kind, vectorcraft_doc::TextKind::Point) => {
            if let Some(path) = t.area_frame().or_else(|| t.type_path()) {
                bp.extend(path.to_bezpath());
            }
        }
        NodeKind::Text(_) | NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => {
            if let Some(b) = c.geometric_bounds() {
                bp.extend(vectorcraft_geom::shapes::rectangle(b).to_bezpath());
            }
        }
        // A placed document shows its box, turned with it.
        NodeKind::PlacedDocument(p) => {
            let [a, b, c, d] = p.corners();
            bp.move_to(a);
            [b, c, d].into_iter().for_each(|q| bp.line_to(q));
            bp.close_path();
        }
        // An envelope shows its mesh (or top object), not its content.
        NodeKind::Envelope { .. } => {
            if let Some((lines, _)) = vectorcraft_doc::live::envelope_overlay(c) {
                bp.extend(lines.to_bezpath());
            }
        }
        _ => {}
    });
    bp
}

/// The width of the key object's outline (Align to Key Object), thicker than the selection's.
const KEY_OUTLINE: f32 = 2.5;

/// The most anchor points the selection's outlines and anchors, and the hover highlight, are drawn
/// for: an Image Trace of a photo selects hundreds of thousands of paths at once, whose outlines
/// would make a mesh larger than the GPU takes in one buffer (#525).
const OVERLAY_MAX_ANCHORS: usize = 100_000;

/// Whether `nodes` have more than [`OVERLAY_MAX_ANCHORS`] anchor points to outline (counting
/// stops there).
fn too_many_anchors<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> bool {
    fn over(n: &Node, left: &mut usize) -> bool {
        if let NodeKind::Path { path, .. } = &n.kind {
            match left.checked_sub(path.anchor_count()) {
                Some(l) => *left = l,
                None => return true,
            }
        }
        !matches!(n.kind, NodeKind::Envelope { .. })
            && n.children().into_iter().flatten().skip(usize::from(n.shaper.is_some())).any(|c| over(c, left))
    }
    let mut left = OVERLAY_MAX_ANCHORS;
    nodes.into_iter().any(|n| over(n, &mut left))
}

/// [`Node::walk`] over what a selection highlight shows: an envelope's content is left out (the
/// envelope shows its mesh instead).
fn walk_drawn<'a>(n: &'a Node, f: &mut impl FnMut(&'a Node)) {
    f(n);
    if matches!(n.kind, NodeKind::Envelope { .. }) {
        return;
    }
    for c in n.children().into_iter().flatten().skip(usize::from(n.shaper.is_some())) {
        walk_drawn(c, f);
    }
}

/// The topmost editable object under document point `p` at `zoom`, as the selection tools pick it
/// (Selection & Anchor Display › Tolerance and Object Selection by Path Only, Type › Type Object
/// Selection by Path Only).
fn hit_at(app: &VectorcraftApp, p: Point, zoom: f64) -> Option<vectorcraft_doc::hit::Hit> {
    let prefs = &app.session.prefs;
    let opt = vectorcraft_doc::hit::HitOptions {
        tol: prefs.selection_tolerance / zoom,
        outline: app.ui.view.outline,
        path_only: prefs.object_selection_by_path_only,
        type_path_only: prefs.type_selection_by_path_only,
        scope: app.session.active().and_then(|st| st.isolation),
    };
    vectorcraft_doc::hit::hit_test(&app.session.active()?.doc, p, opt)
}

/// Right-click: the object under the pointer is selected first unless it already is, then the
/// context menu lists what applies to the selection ([`crate::menus::context_items`]).
fn context_menu(app: &mut VectorcraftApp, resp: &egui::Response, xf: &Xf) {
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some(st) = app.session.active()
        && let Some(top) = hit_at(app, xf.to_doc(p), xf.zoom).map(|h| h.top_object(st.isolation))
        && !st.selection.contains(top)
    {
        // A locked or hidden object can't be selected; the menu is then for the selection as is.
        let _ = app.run("select.set", json!({ "ids": [top.0] }));
    }
    let mut clicked = None;
    resp.context_menu(|ui| crate::menus::context_menu_body(app, ui, &mut clicked));
    if let Some((id, p)) = clicked {
        crate::menus::invoke(app, &id, p);
    }
}

/// A panel drag ([`widgets::PanelDrag`]) dropped on art acts on the object under the pointer,
/// selected or not: a paint (swatches, a Fill/Stroke proxy, the Gradient panel's thumbnail) goes
/// to its active proxy (`paint.setFill`/`paint.setStroke` with its `ids`; a gradient fits it), the
/// Appearance panel's thumbnail gives the object (the topmost one hit) the appearance it carries
/// (`appearance.copyFrom`), a graphic style is applied to it (`graphicStyle.apply`; with Alt, on
/// top of its appearance). A chip follows the pointer meanwhile.
fn panel_drop(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, xf: &Xf) {
    crate::panels::swatches::drag_preview(app, ui.ctx());
    let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else { return };
    // Not through a floating panel over the canvas.
    if ui.ctx().layer_id_at(pos).is_some_and(|l| l != resp.layer_id) {
        return;
    }
    let Some(d) = resp.dnd_release_payload::<widgets::PanelDrag>() else { return };
    // A Libraries panel graphic: a copy centred where it is dropped.
    if let widgets::PanelDrag::LibraryGraphic { library, item } = &*d {
        let at = xf.to_doc(pos);
        if let Err(e) = app.run("library.use", json!({"library": library, "kind": "graphic", "item": item, "center": [at.x, at.y]})) {
            app.status(e);
        }
        return;
    }
    // A symbol from the Symbols panel: an instance centred where it is dropped, on art or not.
    if let widgets::PanelDrag::Symbol(name) = &*d {
        let at = xf.to_doc(pos);
        if let Err(e) = app.run("symbol.place", json!({"name": name, "x": at.x, "y": at.y})) {
            app.status(e);
        }
        return;
    }
    let Some(hit) = hit_at(app, xf.to_doc(pos), xf.zoom) else { return };
    let Some(st) = app.session.active() else { return };
    let (cmd, params) = match &*d {
        widgets::PanelDrag::Paint { params, .. } => {
            // Colour groups paint nothing.
            if params.is_null() {
                return;
            }
            let mut params = params.clone();
            params["ids"] = json!([vectorcraft_tools::xform::paint_owner(&st.doc, hit.leaf).0]);
            params["focus"] = json!(false);
            (crate::panels::proxy_cmd(app, false), params)
        }
        widgets::PanelDrag::Appearance(source) => {
            let target = hit.top_object(st.isolation);
            if target == *source {
                return;
            }
            ("appearance.copyFrom", json!({"source": source.0, "ids": [target.0]}))
        }
        widgets::PanelDrag::GraphicStyle(name) => {
            let add = ui.input(|i| i.modifiers.alt);
            ("graphicStyle.apply", json!({"name": name, "ids": [hit.top_object(st.isolation).0], "add": add}))
        }
        // A brush from the Brushes panel: the path it lands on takes it (a compound path as a whole).
        widgets::PanelDrag::Brush { name, .. } => {
            ("brush.apply", json!({"name": name, "ids": [vectorcraft_tools::xform::paint_owner(&st.doc, hit.leaf).0]}))
        }
        // Art dragged back onto the canvas: its move was already dropped. (A symbol was placed
        // above.)
        widgets::PanelDrag::Art(_) | widgets::PanelDrag::Symbol(_) | widgets::PanelDrag::LibraryGraphic { .. } => return,
    };
    if let Err(e) = app.run(cmd, params) {
        app.status(e);
    }
}

fn hover_highlight(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(h) = app.hover_doc else { return };
    // Smart Guides › Object Highlighting: a Smart Guides display option, so it needs them on.
    if !app.ui.view.smart_guides || !app.session.prefs.object_highlighting {
        return;
    }
    if app.session.tool_busy() || !matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection") {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(hit) = hit_at(app, h, xf.zoom) else { return };
    let id = if app.session.tool_id() == "selection" { hit.top_object(st.isolation) } else { hit.leaf };
    if st.selection.contains(id) {
        return;
    }
    if let Some(n) = st.doc.node(id).filter(|n| !too_many_anchors([*n])) {
        let color = c32(st.doc.layer_color(id));
        stroke_path(p, &node_outline(n), xf, Stroke::new(1.5, color));
    }
}

/// Selected anchors are drawn slightly deeper than the layer colour (#4f80ff → #3d82ff for Layer 1).
fn selected_anchor(c: Color32) -> Color32 {
    if c == Color32::from_rgb(0x4f, 0x80, 0xff) { Color32::from_rgb(0x3d, 0x82, 0xff) } else { c }
}

/// How anchors and handles look: Selection & Anchor Display › Size and Handles.
#[derive(Clone, Copy)]
struct HandleLook<'a> {
    /// Points bigger (or smaller) than the default size 3, per step of Size (1–7).
    grow: f32,
    /// `solid` (the default), `hollow` or `large`.
    style: &'a str,
}

impl<'a> HandleLook<'a> {
    fn of(prefs: &'a vectorcraft_engine::Prefs) -> Self {
        Self { grow: prefs.anchor_size.clamp(1, 7) as f32 - 3.0, style: &prefs.handle_style }
    }

    /// A direction handle's end at `c`: a solid dot, a hollow one or a larger solid one.
    fn draw(self, p: &egui::Painter, c: Pos2, color: Color32) {
        let r = 2.75 + self.grow / 2.0;
        let hollow = self.style == "hollow";
        p.circle_filled(c, if self.style == "large" { r + 1.5 } else { r }, if hollow { Color32::WHITE } else { color });
        if hollow {
            p.circle_stroke(c, r, Stroke::new(1.0, color));
        }
    }
}

fn anchor_square(p: &egui::Painter, c: Pos2, color: Color32, filled: bool, size: f32) {
    let r = egui::Rect::from_center_size(c, vec2(size, size));
    if filled {
        p.rect_filled(r, 0.0, color);
    } else {
        p.rect_filled(r, 0.0, Color32::WHITE);
        p.rect_stroke(r, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
    }
}

/// Type → Show Hidden Characters: spaces as dots, ¶ at paragraph ends, # at the end of a story.
fn hidden_chars_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let clip = p.clip_rect();
    let color = Color32::from_rgb(0x4f, 0x9d, 0xff);
    let font = egui::FontId::proportional(((10.0 * xf.zoom) as f32).clamp(7.0, 18.0));
    st.doc.walk(|n| {
        let NodeKind::Text(tx) = &n.kind else { return };
        if !n.visible || n.geometric_bounds().is_none_or(|b| !xf.rect_to_screen(b).intersects(clip)) {
            return;
        }
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
        if lay.on_path {
            return;
        }
        let text = tx.plain_text();
        let to = |q: Point| xf.to_screen(tx.xf * q);
        for g in &lay.glyphs {
            if text.get(g.byte..).is_some_and(|s| s.starts_with(' ')) {
                p.circle_filled(to(Point::new(g.origin.x + g.advance / 2.0, g.origin.y - 3.0)), 1.2, color);
            }
        }
        for (i, line) in lay.lines.iter().enumerate() {
            let last = i + 1 == lay.lines.len();
            let breaks = text.get(line.end..).is_some_and(|s| s.starts_with('\n'));
            let (glyph, at) = if last && line.end >= text.len() {
                ("#", true)
            } else if breaks {
                ("¶", true)
            } else {
                ("", false)
            };
            if at {
                p.text(to(Point::new(line.x1 + 1.0, line.baseline)), egui::Align2::LEFT_BOTTOM, glyph, font.clone(), color);
            }
        }
    });
}

/// Text threads of the selected frames: a line from each frame's out port (bottom right) to the
/// next frame's in port (top left), like Illustrator's thread indicators.
fn thread_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    for thread in &st.doc.text_threads {
        if !thread.iter().any(|id| st.selection.objects.contains(id)) {
            continue;
        }
        let color = c32(st.doc.layer_color(thread[0]));
        let frames: Vec<vectorcraft_geom::Rect> = thread.iter().filter_map(|id| st.doc.node(*id).and_then(|n| n.geometric_bounds())).collect();
        for w in frames.windows(2) {
            let (out, inp) = (xf.to_screen(Point::new(w[0].x1, w[0].y1)), xf.to_screen(Point::new(w[1].x0, w[1].y0)));
            p.line_segment([out, inp], Stroke::new(1.0, color));
            for port in [out, inp] {
                p.rect_filled(egui::Rect::from_center_size(port, egui::vec2(7.0, 7.0)), 0.0, Color32::WHITE);
                p.rect_stroke(egui::Rect::from_center_size(port, egui::vec2(7.0, 7.0)), 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
                p.line_segment([port - egui::vec2(2.0, 0.0), port + egui::vec2(2.0, 0.0)], Stroke::new(1.0, color));
            }
        }
    }
}

/// The slices (unless View → Hide Slices): user and object slices outlined in the Slices
/// preferences' line colour, auto slices dashed and dimmer, each numbered at its top left (Show
/// Slice Numbers); selected slices have a bolder outline. The layout is cached per revision.
fn slice_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision);
    let slices = match &app.canvas.slices {
        Some((k, s)) if *k == key => s.clone(),
        _ => {
            let s = std::sync::Arc::new(st.doc.slice_layout());
            app.canvas.slices = Some((key, s.clone()));
            s
        }
    };
    if slices.is_empty() {
        return;
    }
    let prefs = &app.session.prefs;
    let line = pref_color(&prefs.slice_line_color, Color32::from_rgb(0xff, 0x3f, 0x3f));
    let dim = line.gamma_multiply(0.55);
    let selected = &st.selection.slices;
    let objects = &st.selection.objects;
    for a in slices.iter() {
        let auto = a.source == vectorcraft_doc::SliceSource::Auto;
        let on = a.id.is_some_and(|id| selected.contains(&id) || objects.contains(&id));
        let q = xf.quad(a.rect);
        let ring: Vec<Pos2> = q.iter().chain(q.first()).copied().collect();
        if auto {
            p.extend(Shape::dashed_line(&ring, Stroke::new(1.0, dim), 3.0, 3.0));
        } else {
            p.add(Shape::line(ring, Stroke::new(if on { 2.0 } else { 1.0 }, line)));
        }
        if prefs.show_slice_numbers
            && let Some(tl) = q.first()
        {
            let font = egui::FontId::proportional(9.0);
            let galley = p.layout_no_wrap(format!("{:02}", a.number), font, Color32::WHITE);
            let badge = egui::Rect::from_min_size(*tl + vec2(1.0, 1.0), galley.size() + vec2(6.0, 2.0));
            p.rect_filled(badge, CornerRadius::ZERO, if auto { dim } else { line });
            p.galley(badge.min + vec2(3.0, 1.0), galley, Color32::WHITE);
        }
    }
}

/// The print tiling (View → Show Print Tiling, or the Print Tiling tool): each page of the
/// document's print settings as `print.preview` lays it out, its paper edge and its imageable
/// area dashed, numbered at its top left; tiles outside the tile range dimmer. Cached per revision.
fn print_tiling_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let key = (st.uid, st.revision);
    let pages = match &app.canvas.print_tiling {
        Some((k, pages)) if *k == key => pages.clone(),
        _ => {
            // Settings that can't print have no pages to show.
            let pages = std::sync::Arc::new(vectorcraft_engine::cmd::printtiling::pages(&st.doc).unwrap_or_default());
            app.canvas.print_tiling = Some((key, pages.clone()));
            pages
        }
    };
    let font = egui::FontId::proportional(10.0);
    let ring = |r: [f64; 4]| {
        let q = xf.quad(Rect::new(r[0], r[1], r[2], r[3]));
        q.iter().chain(q.first()).copied().collect::<Vec<Pos2>>()
    };
    for page in pages.iter() {
        let color = if page.printed { PRINT_TILING } else { PRINT_TILING.gamma_multiply(0.45) };
        p.add(Shape::line(ring(page.page), Stroke::new(0.75, color)));
        let inner = ring(page.imageable);
        p.extend(Shape::dashed_line(&inner, Stroke::new(1.0, color), 4.0, 3.0));
        if let Some(corner) = inner.iter().copied().reduce(|a, b| pos2(a.x.min(b.x), a.y.min(b.y))) {
            p.text(corner + vec2(3.0, 2.0), egui::Align2::LEFT_TOP, page.number.to_string(), font.clone(), color);
        }
    }
}

/// The print tiling's lines: a neutral grey that reads on the paper and on the pasteboard.
const PRINT_TILING: Color32 = Color32::from_gray(96);

fn selection_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    // The Selection tool's bounding box (rotated with the objects after a rotation). It hides while
    // a drag moves or resizes the selection, so only the art is seen going with the pointer (#712).
    let show_box = app.session.tool_id() == "selection"
        && !app.session.tool_transforming()
        && app.ui.view.bounding_box
        && app.session.active().is_some_and(|st| !st.selection.is_empty() && st.selection.anchors.is_empty());
    let bbox = if show_box { app.selection_box() } else { None };
    let st = app.session.active();
    let big = st.is_some_and(|st| too_many_anchors(st.selection.objects.iter().filter_map(|id| st.doc.node(*id))));
    let big_bounds = if big { app.selection_bounds() } else { None };
    let app = &*app;
    let Some(st) = app.session.active() else { return };
    let tool = app.session.tool_id();
    let direct = vectorcraft_tools::catalog::edits_anchors(tool);
    // Selection & Anchor Display › Size (1–7, 3 the default): anchors, handles and the bounding
    // box's handles a point bigger or smaller per step.
    let look = HandleLook::of(&app.session.prefs);
    let grow = look.grow;
    let anchor = |direct: bool| grow + if direct { 5.0 } else { 4.0 };
    // The selected anchors' handles (drawn once they are counted: Show handles when multiple
    // anchors are selected off shows them for a single one only).
    let (mut handles, mut with_handles) = (vec![], 0);
    // Too many paths to outline: the selection's bounds stand for them.
    if let Some((b, id)) = big_bounds.zip(st.selection.objects.first()) {
        stroke_path(p, &vectorcraft_geom::shapes::rectangle(b).to_bezpath(), xf, Stroke::new(1.0, c32(st.doc.layer_color(*id))));
    }
    // The key object (Align to Key Object): its outline drawn thicker, or its bounds when it has
    // too many paths to outline.
    if let Some((k, n)) = st.selection.key.and_then(|k| Some((k, st.doc.node(k)?))) {
        let outline = if too_many_anchors([n]) {
            n.geometric_bounds().map(|b| vectorcraft_geom::shapes::rectangle(b).to_bezpath())
        } else {
            Some(node_outline(n))
        };
        if let Some(bp) = outline {
            stroke_path(p, &bp, xf, Stroke::new(KEY_OUTLINE, c32(st.doc.layer_color(k))));
        }
    }
    for id in st.selection.objects.iter().filter(|_| !big) {
        let Some(n) = st.doc.node(*id) else { continue };
        let color = c32(st.doc.layer_color(*id));
        let partial = st.selection.partial(*id);
        // Path outlines.
        stroke_path(p, &node_outline(n), xf, Stroke::new(1.0, color));
        // Anchors (and handles for selected anchors in direct mode); a mesh envelope's points.
        walk_drawn(n, &mut |c| {
            if let Some((_, Some(grid))) = vectorcraft_doc::live::envelope_overlay(c) {
                for q in &grid.points {
                    anchor_square(p, xf.to_screen(q.p), color, false, anchor(direct));
                }
                return;
            }
            let NodeKind::Path { path, .. } = &c.kind else { return };
            for (si, ai, a) in path.anchors() {
                let sel = match partial {
                    Some(set) => set.contains(&(si, ai)),
                    None => !direct || c.id == *id,
                };
                let sp = xf.to_screen(a.p);
                if sel && (direct || partial.is_some()) {
                    with_handles += 1;
                    handles.extend([a.h_in, a.h_out].into_iter().filter(|h| h.distance(a.p) > 1e-6).map(|h| (sp, xf.to_screen(h), color)));
                }
                anchor_square(p, sp, if sel && partial.is_some() { selected_anchor(color) } else { color }, sel, anchor(partial.is_some() || direct));
            }
        });
        // Centre point (Attributes panel → Show Center).
        if n.shows_center()
            && let Some(b) = n.geometric_bounds()
        {
            anchor_square(p, xf.to_screen(b.center()), color, true, anchor(false));
        }
        // Point type: baseline marker (area type shows its frame instead, which may not be a
        // rectangle, and type on a path its path).
        if let NodeKind::Text(tx) = &n.kind
            && matches!(tx.kind, vectorcraft_doc::TextKind::Point)
        {
            let o = xf.to_screen(tx.xf * Point::ZERO);
            let b = n.geometric_bounds().unwrap_or_default();
            let e = xf.to_screen(Point::new(b.x1, (tx.xf * Point::ZERO).y));
            p.line_segment([o, e], Stroke::new(1.0, color));
            p.circle_filled(o, 2.5, color);
        }
    }
    if app.session.prefs.show_handles_multiple_anchors || with_handles <= 1 {
        for (sp, hp, color) in handles {
            p.line_segment([sp, hp], Stroke::new(1.0, color));
            look.draw(p, hp, color);
        }
    }
    // The spine of each selected blend (or of the blend a selected key object belongs to).
    let mut spines = vec![];
    for id in &st.selection.objects {
        let is_blend = |b: &vectorcraft_doc::NodeId| st.doc.node(*b).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. }));
        let Some(b) = [Some(*id), st.doc.parent_of(*id)].into_iter().flatten().find(is_blend).filter(|b| !spines.contains(b)) else { continue };
        spines.push(b);
        let Some(NodeKind::Blend { children, spec }) = st.doc.node(b).map(|n| &n.kind) else { continue };
        let Some((path, _)) = vectorcraft_doc::live::blend_spine(children, spec) else { continue };
        let color = c32(st.doc.layer_color(b));
        stroke_path(p, &path.to_bezpath(), xf, Stroke::new(1.0, color));
        for (_, _, a) in path.anchors() {
            anchor_square(p, xf.to_screen(a.p), color, false, anchor(direct));
        }
    }
    // Live Corners widgets (Selection on a live rectangle or polygon, Direct Selection on any path).
    if matches!(tool, "selection" | "directSelection")
        && let Some(w) = vectorcraft_tools::corners::CornerWidgets::showing(
            &st.doc,
            &st.selection,
            xf.zoom,
            tool == "directSelection",
            app.ui.view.corner_widgets,
            app.session.prefs.hide_corner_widget_above,
        )
    {
        let color = c32(st.doc.layer_color(w.id));
        for sp in w.visible().map(|q| xf.to_screen(q)) {
            p.circle_filled(sp, 3.0, Color32::WHITE);
            p.circle_stroke(sp, 3.0, Stroke::new(1.0, color));
            p.circle_filled(sp, 1.0, color);
        }
    }
    // Bounding box with handles (Selection tool).
    if let Some(b) = bbox {
        let color = c32(st.doc.layer_color(st.selection.objects[0]));
        p.add(Shape::closed_line(b.corners().iter().map(|q| xf.to_screen(*q)).collect(), Stroke::new(1.0, color)));
        for h in vectorcraft_tools::bbox::Handle::ALL {
            let c = xf.to_screen(b.to_doc() * h.pos(b.rect));
            let hr = egui::Rect::from_center_size(c, vec2(6.0 + grow, 6.0 + grow));
            p.rect_filled(hr, 0.0, Color32::WHITE);
            p.rect_stroke(hr, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
        }
        // The type widget beside it: hollow on point type, filled on area type.
        if let Some(w) = vectorcraft_tools::typewidget::TypeWidget::of(&st.doc, &st.selection, &b, xf.zoom) {
            let (c, r) = (xf.to_screen(w.at), vectorcraft_tools::typewidget::RADIUS_PX);
            p.circle_filled(c, r, if w.area { color } else { Color32::WHITE });
            p.circle_stroke(c, r, Stroke::new(1.0, color));
        }
    }
}

/// While the Type tool edits, let the system IME compose (egui-winit allows it only in frames that
/// set `ime`) and keep its candidate window under the caret.
fn ime_output(app: &mut VectorcraftApp, ctx: &egui::Context, xf: &Xf) {
    let ours = app.session.tool_wants_text() && !ctx.egui_wants_keyboard_input() && app.ui.dialog.is_none() && !app.ui.palette_open;
    let caret = if ours { app.session.tool_ime_caret(app.view_info()) } else { None };
    let Some((a, b)) = caret else {
        // The IME goes away with its marked text: keep that text as typed, like a click away.
        crate::shortcuts::keep_marked_text(app);
        return;
    };
    let r = egui::Rect::from_two_pos(xf.to_screen(a), xf.to_screen(b)).expand2(vec2(1.0, 0.0));
    // The tool ended the composition itself: the IME must drop what it still has marked.
    let interrupt = app.ime_marked.is_some() && !app.session.tool_composing();
    if interrupt {
        app.ime_marked = None;
        app.ime_discard = true;
    }
    ctx.output_mut(|o| {
        o.ime = Some(egui::output::IMEOutput { purpose: egui::IMEPurpose::Normal, rect: r, cursor_rect: r, should_interrupt_composition: interrupt });
    });
}

/// Is overlay label `text` the Artboard tool's "01 - <artboard name>"? It holds a name, so it is
/// shown as it is; the tools' other labels ("anchor", "path", Puppet Warp's warning) are ours.
fn names_an_artboard(text: &str) -> bool {
    text.split_once(" - ").is_some_and(|(n, _)| n.len() >= 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

fn draw_overlays(p: &egui::Painter, xf: &Xf, overlays: &[Overlay], t: &Tokens, look: HandleLook) {
    for o in overlays {
        match o {
            Overlay::Marquee(r) => marquee(p, xf.rect_to_screen(*r)),
            Overlay::Path { path, color, width, dashed } => {
                let s = Stroke::new(*width, c32(*color));
                if *dashed {
                    for line in to_screen_path(path, xf) {
                        p.extend(Shape::dashed_line(&line, s, 4.0, 3.0));
                    }
                } else {
                    stroke_path(p, path, xf, s);
                }
            }
            Overlay::Line { a, b, color, dashed } => {
                let s = Stroke::new(1.0, c32(*color));
                let (a, b) = (xf.to_screen(*a), xf.to_screen(*b));
                if *dashed {
                    p.extend(Shape::dashed_line(&[a, b], s, 4.0, 3.0));
                } else {
                    p.line_segment([a, b], s);
                }
            }
            Overlay::Anchor { p: pt, color, filled, size } => anchor_square(p, xf.to_screen(*pt), c32(*color), *filled, *size),
            Overlay::Handle { p: pt, color } => look.draw(p, xf.to_screen(*pt), c32(*color)),
            Overlay::Label { p: pt, text, color } => {
                let sp = xf.to_screen(*pt) + vec2(8.0, -14.0);
                p.text(
                    sp,
                    egui::Align2::LEFT_TOP,
                    crate::panels::label_or_name(text, !names_an_artboard(text)),
                    egui::FontId::proportional(11.0),
                    c32(*color),
                );
            }
            Overlay::Highlight { quad, color } => {
                let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                p.add(Shape::convex_polygon(quad.iter().map(|q| xf.to_screen(*q)).collect(), c, Stroke::NONE));
            }
            Overlay::Measure { p: pt, text } => {
                let sp = xf.to_screen(*pt) + vec2(14.0, 14.0);
                let galley = p.layout(text.clone(), egui::FontId::proportional(11.0), Color32::WHITE, 200.0);
                let r = egui::Rect::from_min_size(sp, galley.size() + vec2(12.0, 8.0));
                p.rect_filled(r, CornerRadius::same(3), t.measure_bg);
                p.galley(sp + vec2(6.0, 4.0), galley, Color32::WHITE);
            }
            Overlay::GridLine { a, b, color } => {
                let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                p.line_segment([xf.to_screen(*a), xf.to_screen(*b)], Stroke::new(1.0, c));
            }
            Overlay::Swatch { p: pt, color, selected } => {
                // A white disc under the colour shows its opacity; a dark rim keeps it readable on
                // any art, and the accent ring marks the selected stop.
                let c = xf.to_screen(*pt);
                p.circle_filled(c, 6.0, Color32::WHITE);
                p.circle_filled(c, 5.0, Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]));
                p.circle_stroke(c, 6.0, Stroke::new(1.0, Color32::from_gray(32)));
                if *selected {
                    p.circle_stroke(c, 8.0, Stroke::new(2.0, t.accent));
                }
            }
        }
    }
}

/// The Home screen (`app.home`; with no document open, unless its preference is off).
fn home(app: &mut VectorcraftApp, ui: &mut Ui, rect: egui::Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.panel_darker);
    let inner = rect.shrink2(vec2((rect.width() - 820.0).max(40.0) / 2.0, 60.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut child;
    ui.label(egui::RichText::new(tl!("Welcome to VectorCraft")).font(theme::semibold(26.0)).color(t.text));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Vector illustration — fast, open, scriptable.")).size(14.0).color(t.text_dim));
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        if widgets::primary_button(ui, tl!("New file")).clicked() {
            app.run("file.newDialog", json!({})).ok();
        }
        ui.add_space(8.0);
        if widgets::secondary_button(ui, tl!("Open")).clicked() {
            app.run("file.open", json!({})).ok();
        }
    });
    recent(app, ui);
    ui.add_space(28.0);
    ui.label(egui::RichText::new(tl!("Quickly start a new file")).font(theme::semibold(14.0)).color(t.text));
    ui.add_space(10.0);
    // A few of New Document's presets (`file.newPresets`), as its cards.
    let presets = ["Letter", "A4", "Web 1920×1080", "Phone 390×844", "Postcard", "Social Square Post 1080×1080"];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
        let cards: Vec<_> = presets.iter().filter_map(|name| vectorcraft_engine::cmd::newdoc::find(&app.session, name)).collect();
        for s in cards {
            if crate::dialogs::preset_card(ui, &s, false).clicked() {
                app.run("file.new", json!({ "preset": s.name })).ok();
            }
        }
    });
    ui.add_space(28.0);
    ui.label(egui::RichText::new(tl!("Community")).font(theme::semibold(14.0)).color(t.text));
    ui.add_space(10.0);
    crate::community::links(app, ui);
}

/// The Home screen's Recent Files (#663): the first of File › Open Recent Files, each its name and
/// folder; a click opens it. Nothing when there are none.
fn recent(app: &mut VectorcraftApp, ui: &mut Ui) {
    const SHOWN: usize = 6;
    let files: Vec<String> = crate::io::recent_files(app).iter().take(SHOWN).cloned().collect();
    if files.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.add_space(28.0);
    ui.label(egui::RichText::new(tl!("Recent Files")).font(theme::semibold(14.0)).color(t.text));
    ui.add_space(8.0);
    for path in files {
        let p = std::path::Path::new(&path);
        let name = p.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        let folder = p.parent().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        ui.horizontal(|ui| {
            crate::icons::icon(ui, "history", 16.0, t.icon);
            let r = ui.add(egui::Button::new(egui::RichText::new(&name).color(t.accent).size(13.0)).frame(false)).on_hover_text(&path);
            ui.add(egui::Label::new(egui::RichText::new(&folder).size(12.0).color(t.text_dim)).truncate());
            if r.clicked() {
                // A failure is reported to the user there.
                let _ = crate::io::open_reporting(app, &path);
            }
        });
    }
}

fn kurbo_flatten(p: &BezPath, tol: f64, f: &mut impl FnMut(PathEl)) {
    vectorcraft_geom::kurbo::flatten(p.elements().iter().copied(), tol, f);
}

/// Upload the art's raster; Pixel Preview's document pixels keep hard edges when magnified.
fn upload(app: &mut VectorcraftApp, ctx: &egui::Context, img: &vectorcraft_render::Rendered, pixel: bool) {
    let color = egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels);
    let options = if pixel { egui::TextureOptions::NEAREST } else { egui::TextureOptions::LINEAR };
    match &mut app.canvas.texture {
        Some(tex) => tex.set(color, options),
        None => app.canvas.texture = Some(ctx.load_texture("canvas", color, options)),
    }
}

/// Pixel Preview: the whole document pixels (x0, y0, x1, y1) under the canvas, rotated view
/// included. None when the view is degenerate.
fn pixel_region(xf: &Xf) -> Option<[i64; 4]> {
    let r = xf.rect;
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|p| xf.to_doc(p));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in corners {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
    // A document pixel is bigger than a screen pixel here, so the region is about canvas-sized.
    let ok = [x0, y0, x1, y1].iter().all(|c| c.is_finite() && c.abs() < 1e9) && x1 > x0 && y1 > y0 && x1 - x0 <= 16384.0 && y1 - y0 <= 16384.0;
    ok.then_some([x0 as i64, y0 as i64, x1 as i64, y1 as i64])
}

/// The Contextual Task Bar's area.
fn task_bar_id() -> egui::Id {
    egui::Id::new("task-bar")
}

/// Where the Contextual Task Bar was last drawn (none while it isn't shown).
pub(crate) fn task_bar_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = task_bar_id();
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// The Contextual Task Bar: a floating pill under the selection with the most likely next actions.
/// Its handle drags it anywhere on the canvas; unpinned it keeps that offset as it follows the
/// selection, pinned it stays put ([`crate::state::TaskBarPlace`]).
fn task_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none_or(|st| st.selection.is_empty())
        || !matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection")
    {
        return;
    }
    let Some(b) = app.selection_bounds() else { return };
    let Some(st) = app.session.active() else { return };
    let n = st.selection.len();
    let first = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned();
    let is_group = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Group { .. }));
    let is_text = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Text(_)));
    let mut items: Vec<(&str, &str, &str)> = vec![]; // (label, icon, command)
    // Direct-selected anchors take the place of a path's Offset Path and Simplify.
    let anchors = !st.selection.anchors.is_empty();
    if anchors {
        items.push((tl!("Remove Anchor Points"), "pen-tool-delete", "path.removeAnchors"));
        items.push((tl!("Cut Path"), "scissors", "path.cutAtAnchors"));
    }
    if n > 1 {
        items.push((tl!("Group"), "group", "object.group"));
        items.push((tl!("Unite"), "squares-unite", "object.pathfinder.unite"));
    } else if is_group {
        items.push((tl!("Ungroup"), "ungroup", "object.ungroup"));
        items.push((tl!("Isolate"), "square-dashed", "object.isolate"));
    } else if is_text {
        items.push((tl!("Create Outlines"), "type", "type.createOutlines"));
    } else if !anchors {
        items.push((tl!("Offset Path"), "square-dashed", "object.path.offsetPath"));
        items.push((tl!("Simplify"), "spline", "object.path.simplify"));
    }
    items.push((tl!("Duplicate"), "copy", "edit.duplicate"));
    let fill = first.as_ref().map(|f| f.appearance.fill_paint()).unwrap_or_default();
    let doc = st.uid;
    let anchor = xf.to_screen(Point::new(b.center().x, b.y1));
    let est_w = 118.0 + items.iter().map(|(l, _, _)| l.len() as f32 * 7.2 + 44.0).sum::<f32>();
    // Its place under the selection, then where the handle moved it, kept on the canvas (with the
    // bar's size last frame, the estimate before it first shows).
    let under = pos2(anchor.x - est_w / 2.0, anchor.y + 28.0);
    let id = task_bar_id();
    let size = egui::AreaState::load(ui.ctx(), id).and_then(|s| s.size).unwrap_or(vec2(est_w, 44.0));
    let bounds = xf.rect.shrink(8.0);
    let keep_in = |p: Pos2| p.clamp(bounds.min, (bounds.max - size).max(bounds.min));
    let place = &mut app.ui.task_bar_place;
    // Just unpinned: follow the selection from the pinned spot.
    if !place.pinned
        && let Some(at) = place.pin_at.take()
    {
        place.offset = Some((doc, xf.rect.min + at - under));
    }
    let pos = keep_in(match place.pin_at {
        Some(at) => xf.rect.min + at,
        None => under + place.offset.filter(|(d, _)| *d == doc).map_or(Vec2::ZERO, |(_, o)| o),
    });
    place.shown_at = Some(pos - xf.rect.min);
    // Pinned before it ever showed: hold it here.
    if place.pinned && place.pin_at.is_none() {
        place.pin_at = place.shown_at;
    }
    let pinned = place.pinned;
    let mut moved = Vec2::ZERO;
    let mut run: Option<(&str, serde_json::Value)> = None;
    egui::Area::new(id).order(egui::Order::Middle).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.tool_active))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(70) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (g, grip) = ui.allocate_exact_size(vec2(4.0, 28.0), Sense::drag());
                    let active = grip.hovered() || grip.dragged();
                    ui.painter().rect_filled(g.shrink2(vec2(0.5, 4.0)), CornerRadius::same(2), if active { t.text_dim } else { t.button_border });
                    if grip.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        moved = grip.drag_delta();
                    } else if grip.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    for (label, icon, cmd) in &items {
                        let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), t.text_strong);
                        let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 38.0, 30.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(3), t.hover);
                        }
                        ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, t.button_border), StrokeKind::Inside);
                        crate::icons::paint(ui, icon, egui::Rect::from_min_size(r.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), t.icon);
                        ui.painter().galley(pos2(r.left() + 30.0, r.center().y - galley.size().y / 2.0), galley, t.text_strong);
                        if resp.clicked() {
                            run = Some((*cmd, json!({})));
                        }
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 30.0), Sense::click());
                    widgets::paint_chip(ui, egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0)), &fill);
                    ui.painter().rect_stroke(
                        egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0)),
                        0.0,
                        Stroke::new(1.0, t.button_border),
                        StrokeKind::Outside,
                    );
                    if resp.on_hover_text(tl!("Fill")).clicked() {
                        app.session.fill_active = true;
                        app.ui.open_panel = Some("swatches".into());
                    }
                    if widgets::icon_button(ui, "lock", tl!("Lock (⌘2)"), false, 30.0).clicked() {
                        run = Some(("object.lock", json!({})));
                    }
                    let more = widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, 30.0);
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(170.0);
                        let bar = [
                            (tl!("Hide Bar"), "window.taskBar", false),
                            (tl!("Pin Bar Position"), "window.taskBar.pin", pinned),
                            (tl!("Reset Bar Position"), "window.taskBar.reset", false),
                        ];
                        for (label, cmd, checked) in bar {
                            if widgets::menu_item(ui, label, true, checked) {
                                run = Some((cmd, json!({})));
                            }
                        }
                        ui.separator();
                        if widgets::menu_item(ui, tl!("Show Properties Panel"), true, false) {
                            run = Some(("window.panel", json!({"panel": "properties"})));
                        }
                    });
                });
            });
    });
    if moved != Vec2::ZERO {
        let to = keep_in(pos + moved);
        let place = &mut app.ui.task_bar_place;
        if place.pinned {
            place.pin_at = Some(to - xf.rect.min);
        } else {
            place.offset = Some((doc, to - under));
        }
        ui.ctx().request_repaint();
    }
    if let Some((c, p)) = run {
        crate::menus::invoke(app, c, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_geom::Shape as _;

    /// #826: in a turned view each ruler still has ticks along its whole length; at 0° the top one
    /// reads x where the canvas has it.
    #[test]
    fn rulers_cover_their_length_at_any_rotation() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let full = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let [top, left, _] = ruler_rects(full);
        for deg in [0.0_f64, 30.0, 90.0, 135.0, 200.0] {
            let xf = Xf { rect: full, zoom: 1.0, center: Point::new(300.0, 200.0), rot: deg.to_radians() };
            let mut out = ctx.run_ui(Default::default(), |ui| rulers(ui, full, &xf, None, Unit::Points, &Tokens::get(ui.ctx())));
            out.textures_delta.clear();
            // The ticks' positions along each ruler.
            let along = |ruler: egui::Rect, x: bool| -> Vec<f32> {
                let mut at: Vec<f32> = out
                    .shapes
                    .iter()
                    .filter(|c| c.clip_rect == ruler)
                    .filter_map(|c| match &c.shape {
                        Shape::LineSegment { points: [a, b], .. } if x && a.x == b.x => Some(a.x),
                        Shape::LineSegment { points: [a, b], .. } if !x && a.y == b.y => Some(a.y),
                        _ => None,
                    })
                    .collect();
                at.sort_by(f32::total_cmp);
                at
            };
            for (ruler, x) in [(top, true), (left, false)] {
                let at = along(ruler, x);
                let (start, end) = if x { (ruler.left(), ruler.right()) } else { (ruler.top(), ruler.bottom()) };
                let (Some(first), Some(last)) = (at.first(), at.last()) else { panic!("{deg}°: no ticks") };
                assert!(first - start < 20.0 && end - last < 20.0, "{deg}°: ticks from {first} to {last} on {start}..{end}");
                assert!(at.windows(2).all(|w| w[1] - w[0] < 20.0), "{deg}°: a gap in the ticks");
            }
            if deg == 0.0 {
                let origin = xf.to_screen(Point::ZERO).x;
                assert!(along(top, true).iter().any(|x| (x - origin).abs() < 0.01), "a tick at x = 0");
            }
        }
    }

    #[test]
    fn shaper_selection_highlights_only_the_visible_result() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
        let line = vectorcraft_doc::NodeId(s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap()["id"].as_u64().unwrap());
        for x in [0.0, 150.0] {
            s.execute("shaper.scribble", &json!({"points":[[60.0+x,140],[70.0+x,160],[80.0+x,140],[90.0+x,160]]})).unwrap();
        }
        let st = s.active().unwrap();
        let g = st.doc.node(st.selection.objects[0]).unwrap();
        assert!(g.shaper.is_some());
        let bounds = node_outline(g).bounding_box();
        assert!((bounds.x0 - 100.0).abs() < 0.01 && (bounds.x1 - 200.0).abs() < 0.01, "{bounds:?}");
        assert_eq!(node_outline(st.doc.node(line).unwrap()).bounding_box().x0, 50.0, "an isolated original still shows its full outline");
    }

    /// One headless canvas frame on an 800 × 600 window.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    }

    /// A click with the Selection tool inside an artboard makes it the active one, and Paste in
    /// Place then pastes onto it, where the objects were on their own artboard (#693).
    #[test]
    fn a_click_activates_its_artboard_and_paste_in_place_goes_onto_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        app.session.execute("shape.rectangle", &json!({"x": boards[0].x0 + 10.0, "y": boards[0].y0 + 15.0, "width": 20, "height": 20})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("selection");
        let view = app.view_info();
        let at = boards[2].center();
        for kind in [PointerKind::Down, PointerKind::Up] {
            dispatch(&mut app, &PointerEvent { kind, pos: at, mods: Default::default(), pressure: 1.0 }, view);
        }
        assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
        let ids: Vec<vectorcraft_doc::NodeId> = app.run("edit.pasteInPlace", json!({})).unwrap()["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| vectorcraft_doc::NodeId(v.as_u64().unwrap()))
            .collect();
        let b = app.session.active().unwrap().doc.bounds_of(&ids, false).unwrap();
        assert_eq!((b.x0, b.y0), (boards[2].x0 + 10.0, boards[2].y0 + 15.0));
    }

    /// Enable Touch Gestures (#585): a two-finger tap undoes and a three-finger one redoes, and the
    /// first finger's press (the pointer egui makes of it) draws nothing. Off, a tap does nothing.
    #[test]
    fn two_finger_tap_undoes_and_three_finger_tap_redoes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
        let c = app.canvas_rect.unwrap().center();
        let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // `fingers` fingers tap at the centre, the first also pressing as the pointer and moving a
        // little (as a rectangle drag would begin).
        let tap = |app: &mut VectorcraftApp, fingers: u64| {
            let at = |n: u64| c + vec2(n as f32 * 50.0, 0.0);
            frame(app, &ctx, vec![touch(0, egui::TouchPhase::Start, at(0)), egui::Event::PointerMoved(at(0)), button(at(0), true)]);
            frame(app, &ctx, vec![touch(0, egui::TouchPhase::Move, at(0) + vec2(6.0, 6.0)), egui::Event::PointerMoved(at(0) + vec2(6.0, 6.0))]);
            frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::Start, at(n))).collect());
            frame(app, &ctx, vec![touch(0, egui::TouchPhase::End, at(0)), button(at(0) + vec2(6.0, 6.0), false), egui::Event::PointerGone]);
            frame(app, &ctx, (1..fingers).map(|n| touch(n, egui::TouchPhase::End, at(n))).collect());
            frame(app, &ctx, vec![]);
        };
        tap(&mut app, 2);
        assert_eq!(count(&app), 0, "the rectangle undone, none drawn");
        tap(&mut app, 3);
        assert_eq!(count(&app), 1, "redone");
        app.session.prefs.touch_gestures = false;
        tap(&mut app, 2);
        assert_eq!(count(&app), 2, "off: the first finger draws as the pointer, and nothing is undone");
    }

    /// The Artboard tool's label holds the artboard's name (never translated); the tools' own
    /// labels are interface text.
    #[test]
    fn artboard_labels_name_the_artboard() {
        assert!(names_an_artboard("01 - Layers") && names_an_artboard("12 - Artboard 12 - copy"));
        assert!(!names_an_artboard("anchor") && !names_an_artboard("1 - x") && !names_an_artboard("Off the mesh - move closer"));
    }

    fn middle(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Default::default() }
    }

    /// One headless frame with the canvas under a 40-point bar, as it is under the document tabs.
    fn frame_under_bar(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            ui.add_space(40.0);
            show(app, ui);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn a_click_just_outside_the_canvas_does_not_reach_the_tool() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // A square in the middle of the artboard, where the view is centred.
        let id = app.session.execute("shape.rectangle", &json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame_under_bar(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let click = |app: &mut VectorcraftApp, p: Pos2| {
            let button =
                |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame_under_bar(app, &ctx, vec![egui::Event::PointerMoved(p)]);
            frame_under_bar(app, &ctx, vec![button(true)]);
            frame_under_bar(app, &ctx, vec![button(false)]);
        };
        let selected = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.clone();
        // Two pixels above the canvas, within egui's interaction radius: nothing happens.
        click(&mut app, pos2(rect.center().x, rect.top() - 2.0));
        assert!(selected(&app).is_empty(), "a click above the canvas selected {:?}", selected(&app));
        // At the canvas's centre: the square.
        click(&mut app, rect.center());
        assert_eq!(selected(&app), vec![vectorcraft_doc::NodeId(id)]);
    }

    #[test]
    fn a_drag_from_a_ruler_onto_the_canvas_makes_a_guide() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.ui.view.rulers = true;
        app.ui.view.guides = false;
        // A drawing tool: the drag must not draw.
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
            frame(app, &ctx, vec![button(from, true)]);
            frame(app, &ctx, vec![egui::Event::PointerMoved(to)]);
            frame(app, &ctx, vec![button(to, false)]);
        };
        let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
        // From the top ruler: a horizontal guide where the button was released.
        let to = pos2(rect.center().x, rect.top() + 120.0);
        drag(&mut app, pos2(rect.center().x, rect.top() - RULER / 2.0), to);
        assert_eq!(guides(&app), [(false, xf.to_doc(to).y)]);
        assert!(app.ui.view.guides, "the new guide shows");
        // From the left ruler: a vertical one.
        let to2 = pos2(rect.left() + 200.0, rect.center().y);
        drag(&mut app, pos2(rect.left() - RULER / 2.0, rect.center().y), to2);
        assert_eq!(guides(&app), [(false, xf.to_doc(to).y), (true, xf.to_doc(to2).x)]);
        // Released back on the ruler: no guide.
        drag(&mut app, pos2(rect.center().x, rect.top() - RULER / 2.0), pos2(rect.center().x + 40.0, rect.top() - 4.0));
        assert_eq!(guides(&app).len(), 2);
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the tool drew nothing");
    }

    /// #451: a guide dragged out of a ruler lands on the art's side midpoints (halving an
    /// artboard-sized rectangle) and on the artboard's edges; an artboard guide is drawn across
    /// its artboard only.
    #[test]
    fn guides_from_the_rulers_snap_and_artboard_guides_span_their_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        app.ui.view.rulers = true;
        app.select_tool("selection");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
            frame(app, &ctx, vec![button(from, true)]);
            frame(app, &ctx, vec![egui::Event::PointerMoved(to)]);
            frame(app, &ctx, vec![button(to, false)]);
        };
        let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
        // 3 px off the middle of the top and bottom sides: onto the line through them.
        let mid = xf.to_screen(Point::new(200.0, 150.0));
        drag(&mut app, pos2(rect.left() - RULER / 2.0, mid.y), mid + vec2(3.0, 40.0));
        // 3 px inside the artboard's bottom edge: onto it.
        let bottom = xf.to_screen(Point::new(100.0, 300.0));
        drag(&mut app, pos2(bottom.x, rect.top() - RULER / 2.0), bottom - vec2(0.0, 3.0));
        assert_eq!(guides(&app), [(true, 200.0), (false, 300.0)]);
        assert_eq!(app.session.active().unwrap().history.undo.last().unwrap().label, "New Guide");
        // An artboard guide runs from the artboard's left edge to its right edge.
        app.session.execute("guide.add", &json!({"vertical": false, "pos": 75, "artboard": 0})).unwrap();
        let s = shapes(&mut app, &ctx);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let (a, b) = (xf.to_screen(Point::new(0.0, 75.0)), xf.to_screen(Point::new(400.0, 75.0)));
        let at = |p: Pos2, q: Pos2| (p - q).length() < 0.01;
        assert!(s.iter().any(|s| matches!(s, Shape::LineSegment { points, .. } if at(points[0], a) && at(points[1], b))), "across the artboard");
        // A canvas guide crosses the whole window.
        let y = xf.to_screen(Point::new(0.0, 300.0)).y;
        let window = app.canvas_rect.unwrap();
        assert!(
            s.iter().any(
                |s| matches!(s, Shape::LineSegment { points, .. } if points[0] == pos2(window.left(), y) && points[1] == pos2(window.right(), y))
            )
        );
    }

    /// #414: guides dragged out of the rulers are picked and dragged with the Selection tool
    /// (highlighted while selected), go with Delete or Backspace, and dragged back onto their
    /// ruler.
    #[test]
    fn the_selection_tool_moves_and_deletes_ruler_guides() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // A square under the guides: the press takes the guide.
        app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 300, "height": 200})).unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let art = app.session.active().unwrap().doc.art_bounds();
        app.ui.view.rulers = true;
        app.select_tool("selection");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(from)]);
            frame(app, &ctx, vec![button(from, true)]);
            frame(app, &ctx, vec![egui::Event::PointerMoved(to)]);
            frame(app, &ctx, vec![button(to, false)]);
        };
        let guides = |app: &VectorcraftApp| app.session.active().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect::<Vec<_>>();
        // Two guides out of the rulers.
        let (h, v) = (pos2(rect.center().x + 30.0, rect.center().y + 20.0), pos2(rect.center().x - 40.0, rect.center().y));
        drag(&mut app, pos2(h.x, rect.top() - RULER / 2.0), h);
        drag(&mut app, pos2(rect.left() - RULER / 2.0, v.y), v);
        assert_eq!(guides(&app), [(false, xf.to_doc(h).y), (true, xf.to_doc(v).x)]);
        // Drag the horizontal one down 30 px (across the vertical one: the nearer is picked).
        let to = pos2(h.x + 10.0, h.y + 30.0);
        drag(&mut app, h, to);
        let st = app.session.active().unwrap();
        assert_eq!(guides(&app), [(false, xf.to_doc(to).y), (true, xf.to_doc(v).x)]);
        assert_eq!((st.selection.guides.clone(), st.selection.objects.clone()), (vec![0], vec![]));
        assert_eq!(st.history.undo.last().unwrap().label, "Move Guide");
        let picked = Tokens::get(&ctx).selection;
        let s = shapes(&mut app, &ctx);
        assert!(s.iter().any(|s| matches!(s, Shape::LineSegment { points, stroke } if stroke.color == picked && points[0].y == to.y)), "highlighted");
        // Backspace deletes the selected guide; Delete too.
        let press = |app: &mut VectorcraftApp, key| {
            let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
            out.textures_delta.clear();
        };
        press(&mut app, egui::Key::Backspace);
        assert_eq!(guides(&app), [(true, xf.to_doc(v).x)]);
        app.session.execute("guide.select", &json!({"indexes": [0]})).unwrap();
        press(&mut app, egui::Key::Delete);
        assert!(guides(&app).is_empty());
        app.session.execute("edit.undo", &json!({})).unwrap();
        // Hide Guides deselects them.
        app.session.execute("guide.select", &json!({"indexes": [0]})).unwrap();
        app.run("view.guides", json!({})).unwrap();
        assert!(app.session.active().unwrap().selection.guides.is_empty());
        app.run("view.guides", json!({})).unwrap();
        // Dragged back onto its ruler, the vertical guide goes.
        drag(&mut app, v, pos2(rect.left() - RULER / 2.0, v.y + 20.0));
        assert!(guides(&app).is_empty(), "{:?}", guides(&app));
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), art, "the square stayed");
    }

    #[test]
    fn cmd_with_another_tool_drags_with_the_selection_tool_used_last() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 50})).unwrap()["id"].as_u64().unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let cmd_frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let events = std::iter::once(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND)).chain(events).collect();
            let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::COMMAND };
        // A Cmd drag from `a` to `b` with the Pen tool; → the tool that did it.
        let cmd_drag = |app: &mut VectorcraftApp, a: Pos2, b: Pos2| {
            app.select_tool("pen");
            cmd_frame(app, vec![egui::Event::PointerMoved(a), button(a, true)]);
            let used = app.session.tool_id().to_string();
            cmd_frame(app, vec![egui::Event::PointerMoved(b)]);
            cmd_frame(app, vec![button(b, false)]);
            assert_eq!(app.session.tool_id(), "pen", "back to the Pen tool");
            used
        };
        let bounds = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        // Direct Selection used last: the drag moves the rectangle's bottom-right corner only.
        app.select_tool("directSelection");
        let corner = xf.to_screen(Point::new(200.0, 150.0));
        assert_eq!(cmd_drag(&mut app, corner, corner + vec2(20.0, 20.0)), "directSelection");
        let b = bounds(&app);
        assert_eq!((b.x0, b.y0), (100.0, 100.0));
        assert!((b.x1 - xf.to_doc(corner + vec2(20.0, 20.0)).x).abs() < 1e-6, "{b:?}");
        // Group Selection, then Selection: each is the one used.
        app.select_tool("groupSelection");
        let inside = xf.to_screen(Point::new(130.0, 120.0));
        assert_eq!(cmd_drag(&mut app, inside, inside), "groupSelection");
        app.select_tool("selection");
        assert_eq!(cmd_drag(&mut app, inside, inside), "selection");
    }

    /// #525: a selection with more anchors than the overlay draws (a traced photo) shows its bounds,
    /// not every outline and anchor, which made a mesh larger than the GPU takes; an ordinary one
    /// still shows its outline and anchors.
    #[test]
    fn a_huge_selection_shows_its_bounds_instead_of_every_outline() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let points = |n: usize| (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
        let ctx = egui::Context::default();
        let mut drawn = |n: usize| {
            app.session.execute("path.create", &json!({"d": format!("M20 20 {} Z", points(n))})).unwrap();
            frame(&mut app, &ctx, vec![]);
            let shapes = shapes(&mut app, &ctx);
            let points: usize = shapes.iter().map(|s| if let Shape::Path(ps) = s { ps.points.len() } else { 0 }).sum();
            let squares = shapes.iter().filter(|s| matches!(s, Shape::Rect(_))).count();
            (points, squares)
        };
        let (points, squares) = drawn(OVERLAY_MAX_ANCHORS + 10);
        assert!(points < 100 && squares < 100, "{points} outline points, {squares} squares");
        let (points, squares) = drawn(200);
        assert!(points > 200 && squares > 200, "{points} outline points, {squares} squares");
    }

    /// #541: the key object's outline is drawn thicker than the rest of the selection's.
    #[test]
    fn the_key_object_has_a_thicker_outline() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let a = app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.session.execute("shape.ellipse", &json!({"x": 200, "y": 100, "width": 60, "height": 40})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let thick =
            |app: &mut VectorcraftApp| shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == KEY_OUTLINE)).count();
        assert_eq!(thick(&mut app), 0, "no key yet");
        app.session.execute("select.key", &json!({"id": b})).unwrap();
        assert!(thick(&mut app) > 0, "the key's outline");
    }

    /// One headless canvas frame → the shapes drawn, `Shape::Vec`s flattened.
    fn shapes(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Shape> {
        let (shapes, mut delta) = frame_output(app, ctx);
        delta.clear();
        shapes
    }

    /// One headless canvas frame → the shapes drawn (`Shape::Vec`s flattened) and the textures
    /// uploaded by it (the art's raster among them, when it was re-rendered).
    fn frame_output(app: &mut VectorcraftApp, ctx: &egui::Context) -> (Vec<Shape>, egui::TexturesDelta) {
        fn flat(s: Shape, out: &mut Vec<Shape>) {
            match s {
                Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
                s => out.push(s),
            }
        }
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let out = ctx.run_ui(raw, |ui| show(app, ui));
        let mut v = vec![];
        out.shapes.into_iter().for_each(|c| flat(c.shape, &mut v));
        (v, out.textures_delta)
    }

    /// View › Pixel Preview: magnified, the art shows as the document pixels it rasterizes to (one
    /// per point, hard-edged), not as smooth vectors; zoomed out or out of Pixel Preview the canvas
    /// renders for the screen again.
    #[test]
    fn pixel_preview_shows_the_document_pixels() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // Its 1 pt stroke covers a quarter of document pixel 99 and most of pixel 100.
        app.session.execute("shape.rectangle", &json!({"x": 100.25, "y": 50, "width": 200, "height": 200})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        let ctx = egui::Context::default();
        // The art's raster uploaded by one frame: (width, height, hard-edged, pixels).
        let upload = |app: &mut VectorcraftApp| {
            let (_, mut delta) = frame_output(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let up = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
                let egui::ImageData::Color(img) = &d.image;
                (img.width(), img.height(), d.options.magnification == egui::TextureFilter::Nearest, img.pixels.clone())
            });
            delta.clear();
            up.expect("the art re-rendered")
        };
        let screen = upload(&mut app); // the first frame fits the view to the artboard
        assert!(screen.0 > 600 && !screen.2, "out of Pixel Preview: the screen's pixels, smoothed");
        app.ui.view.pixel_preview = true;
        let view = app.view_mut().unwrap();
        (view.zoom, view.center) = (8.0, Point::new(100.0, 150.0));
        let (w, h, hard, px) = upload(&mut app);
        assert!(hard, "hard-edged pixels");
        assert!(w <= screen.0 / 8 + 2 && h <= screen.1 / 8 + 2, "one pixel per point: {w} × {h}");
        let [x0, y0, x1, y1] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
        assert_eq!(((x1 - x0) as usize, (y1 - y0) as usize), (w, h));
        // Each document pixel is one texel, anti-aliased by how much of it the art covers.
        let alpha = |x: i64| px[((150 - y0) * w as i64 + x - x0) as usize].a();
        assert!((40..=90).contains(&alpha(99)), "a quarter of pixel 99: {}", alpha(99));
        assert!(alpha(100) > 200, "most of pixel 100: {}", alpha(100));
        assert_eq!((alpha(98), alpha(101)), (0, 255));
        // The pixels are placed over the document rect they cover.
        let s = shapes(&mut app, &ctx);
        let art = app.canvas.texture.as_ref().unwrap().id();
        let Some(Shape::Mesh(mesh)) = s.iter().find(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)) else { panic!("the art") };
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let at = xf.to_screen(Point::new(x0 as f64, y0 as f64));
        assert!((mesh.vertices[0].pos - at).length() < 0.01, "{:?} at {at:?}", mesh.vertices[0].pos);
        // Zoomed out, a document pixel is no bigger than a screen pixel: rendered for the screen.
        app.view_mut().unwrap().zoom = 0.5;
        let (w, _, hard, _) = upload(&mut app);
        assert!(w > 600 && !hard, "zoomed out: the screen's pixels");
    }

    /// General › Anti-aliased Artwork (#394): on (the default), the art's edges are smoothed on
    /// screen; off, every pixel is either painted or not, and turning it either way re-renders.
    #[test]
    fn anti_aliased_artwork_preference_smooths_the_canvas() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // Render on this thread: the worker's frame would land later than the test looks.
        app.canvas.worker_started = true;
        // A disc: its edge crosses pixels at every angle.
        app.session.execute("shape.ellipse", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        // The alphas of the art's raster uploaded by one frame, None when it was not re-rendered.
        let alphas = |app: &mut VectorcraftApp| -> Option<Vec<u8>> {
            let (_, mut delta) = frame_output(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let alphas = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
                let egui::ImageData::Color(img) = &d.image;
                img.pixels.iter().map(|c| c.a()).collect()
            });
            delta.clear();
            alphas
        };
        let partial = |a: &[u8]| a.iter().filter(|&&a| a != 0 && a != 255).count();
        let on = alphas(&mut app).expect("the first frame renders the art");
        assert!(partial(&on) > 50, "anti-aliased: edge pixels partly covered ({} of them)", partial(&on));
        assert!(alphas(&mut app).is_none(), "nothing changed: no re-render");
        app.session.execute("prefs.set", &json!({"key": "antiAliasedArtwork", "value": false})).unwrap();
        let off = alphas(&mut app).expect("the preference change re-renders");
        assert_eq!(partial(&off), 0, "hard edges: every pixel painted or not");
        assert!(off.iter().filter(|&&a| a == 255).count() > 1000, "the disc is still painted");
        app.session.execute("prefs.set", &json!({"key": "antiAliasedArtwork", "value": true})).unwrap();
        assert!(partial(&alphas(&mut app).expect("re-rendered")) > 50, "smooth again");
    }

    /// File Handling › Display Bitmaps as Anti-aliased Images in Pixel Preview (#394): off, Pixel
    /// Preview draws images with each pixel taking its nearest image pixel; on, smoothly, as the
    /// canvas always does out of Pixel Preview.
    #[test]
    fn pixel_preview_smooths_bitmaps_only_when_asked() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        let ctx = egui::Context::default();
        let smooth = |app: &mut VectorcraftApp| {
            let (_, mut delta) = frame_output(app, &ctx);
            delta.clear();
            app.canvas.key.as_ref().map(|k| k.smooth_images).unwrap()
        };
        assert!(smooth(&mut app), "out of Pixel Preview");
        app.ui.view.pixel_preview = true;
        app.view_mut().unwrap().zoom = 8.0;
        assert!(!smooth(&mut app), "Pixel Preview, off by default");
        app.run("prefs.set", json!({"key": "antiAliasedBitmaps", "value": true})).unwrap();
        assert!(smooth(&mut app), "Pixel Preview, on");
    }

    /// Guides & Grid › Show Pixel Grid (Above 600% Zoom) (#394): in Pixel Preview at 600% zoom and
    /// above, a line at every document pixel over the art; none below 600%, out of Pixel Preview,
    /// or with the option off.
    #[test]
    fn pixel_grid_shows_in_pixel_preview_from_600_percent() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        let ctx = egui::Context::default();
        shapes(&mut app, &ctx); // the first frame fits the view to the artboard
        // → (the pixel grid's lines, whether the first of them is drawn over the art).
        let pixel_lines = |app: &mut VectorcraftApp| {
            let s = shapes(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let image = s.iter().position(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)).expect("the art");
            let is_line = |s: &Shape| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == PIXEL_GRID);
            let lines: Vec<usize> = s.iter().enumerate().filter(|(_, s)| is_line(s)).map(|(i, _)| i).collect();
            (lines.len(), lines.first().is_some_and(|&i| i > image))
        };
        app.ui.view.pixel_preview = true;
        app.view_mut().unwrap().zoom = 8.0;
        let (n, over) = pixel_lines(&mut app);
        assert!(n >= 150, "a line every 8 px across the 800 × 600 canvas: {n}");
        assert!(over, "over the art");
        app.view_mut().unwrap().zoom = 5.0;
        assert_eq!(pixel_lines(&mut app).0, 0, "below 600%: none");
        app.view_mut().unwrap().zoom = 6.0;
        assert!(pixel_lines(&mut app).0 >= 200, "from 600%");
        app.ui.view.pixel_preview = false;
        assert_eq!(pixel_lines(&mut app).0, 0, "out of Pixel Preview: none");
        app.ui.view.pixel_preview = true;
        app.session.execute("prefs.set", &json!({"key": "showPixelGrid", "value": false})).unwrap();
        assert_eq!(pixel_lines(&mut app).0, 0, "the option off: none");
        // A rotated view: the lines turn with the pixels, across the whole canvas.
        app.session.execute("prefs.set", &json!({"key": "showPixelGrid", "value": true})).unwrap();
        app.view_mut().unwrap().rotation = 30.0;
        let s = shapes(&mut app, &ctx);
        let turned: Vec<_> = s
            .iter()
            .filter_map(|s| match s {
                Shape::LineSegment { points: [a, b], stroke } if stroke.color == PIXEL_GRID => Some((b.x - a.x).atan2(b.y - a.y).to_degrees().abs()),
                _ => None,
            })
            .collect();
        assert!(turned.len() >= 200, "{}", turned.len());
        assert!(turned.iter().all(|a| [30.0, 60.0, 120.0, 150.0].iter().any(|t| (a - t).abs() < 0.5)), "turned 30°: {turned:?}");
    }

    /// Guides & Grid (#394): the grid in Grid Color, behind the art or, with Grids In Back off,
    /// over it, as dots in the Dots style; the guides in Guides Color and Style.
    #[test]
    fn grid_and_guides_take_their_preferences() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
        app.ui.view.grid = true;
        app.ui.view.guides = true;
        let (red, green) = (Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 255, 0));
        app.session.execute("prefs.set", &json!({"values": {"gridColor": "#ff0000", "guideColor": "#00ff00"}})).unwrap();
        let ctx = egui::Context::default();
        let red_line = |s: &Shape| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == red);
        // → (index of the first red gridline, index of the art's image).
        let order = |app: &mut VectorcraftApp| {
            let s = shapes(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let image = s.iter().position(|s| matches!(s, Shape::Mesh(m) if m.texture_id == art)).expect("the art");
            (s.iter().position(red_line).expect("a gridline in Grid Color"), image, s)
        };
        let (grid, art, s) = order(&mut app);
        assert!(grid < art, "Grids In Back: under the art");
        assert!(s.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == green)), "the guide in Guides Color");
        app.session.execute("prefs.set", &json!({"key": "gridsInBack", "value": false})).unwrap();
        let (grid, art, _) = order(&mut app);
        assert!(grid > art, "Grids In Back off: over the art");
        app.session.execute("prefs.set", &json!({"values": {"gridStyle": "dots", "guideStyle": "dots"}})).unwrap();
        let s = shapes(&mut app, &ctx);
        assert!(!s.iter().any(red_line), "no gridlines in the Dots style");
        let red_dots = |s: &Shape| matches!(s, Shape::Mesh(m) if m.vertices.len() > 40 && m.vertices.iter().all(|v| v.color == red));
        assert!(s.iter().any(red_dots), "the grid's dots");
        assert!(s.iter().any(|s| matches!(s, Shape::Circle(c) if c.fill == green)), "the guide dotted");
    }

    /// Selection & Anchor Display › Size (#394): anchors and the bounding box's handles grow.
    #[test]
    fn anchor_size_scales_anchors_and_handles() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        // The sizes of the squares drawn (anchors and handles are squares).
        let sizes = |app: &mut VectorcraftApp| {
            let mut v: Vec<f32> = shapes(app, &ctx).iter().filter_map(|s| if let Shape::Rect(r) = s { Some(r.rect.width()) } else { None }).collect();
            v.sort_by(f32::total_cmp);
            v.dedup();
            v
        };
        let small = sizes(&mut app);
        assert!(small.contains(&4.0) && small.contains(&6.0), "anchors 4, handles 6 by default: {small:?}");
        app.session.execute("prefs.set", &json!({"key": "anchorSize", "value": 7})).unwrap();
        let big = sizes(&mut app);
        assert!(big.contains(&8.0) && big.contains(&10.0) && !big.contains(&4.0), "4 points bigger at 7: {big:?}");
    }

    /// Selection & Anchor Display › Handles and Show handles when multiple anchors are selected
    /// (#394): handle ends drawn solid, hollow or large; with the latter off, a second selected
    /// anchor hides the handles.
    #[test]
    fn handle_style_and_handles_of_multiple_anchors() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.session.select_tool("directSelection", app.view_info()).unwrap();
        let ctx = egui::Context::default();
        let select = |app: &mut VectorcraftApp, anchors: serde_json::Value| {
            app.session.execute("select.anchors", &json!({"id": id, "anchors": anchors, "mode": "set"})).unwrap();
        };
        // The handle ends: (radius, filled white).
        let dots = |app: &mut VectorcraftApp| -> Vec<(f32, bool)> {
            shapes(app, &ctx)
                .iter()
                .filter_map(|s| match s {
                    Shape::Circle(c) if c.fill != Color32::TRANSPARENT => Some((c.radius, c.fill == Color32::WHITE)),
                    _ => None,
                })
                .collect()
        };
        select(&mut app, json!([[0, 0]]));
        assert_eq!(dots(&mut app), [(2.75, false); 2], "solid by default");
        app.session.execute("prefs.set", &json!({"key": "handleStyle", "value": "hollow"})).unwrap();
        assert_eq!(dots(&mut app), [(2.75, true); 2]);
        app.session.execute("prefs.set", &json!({"key": "handleStyle", "value": "large"})).unwrap();
        assert_eq!(dots(&mut app), [(4.25, false); 2]);
        select(&mut app, json!([[0, 0], [0, 1]]));
        assert_eq!(dots(&mut app).len(), 4, "both anchors' handles");
        app.session.execute("prefs.set", &json!({"key": "showHandlesMultipleAnchors", "value": false})).unwrap();
        assert!(dots(&mut app).is_empty(), "off: none for two anchors");
        select(&mut app, json!([[0, 1]]));
        assert_eq!(dots(&mut app).len(), 2, "one anchor still shows them");
    }

    /// Smart Guides › Object Highlighting (#394): the outline of the object under the pointer
    /// shows with Smart Guides on and the option on, and not otherwise.
    #[test]
    fn object_highlighting_preference_hides_the_hover_outline() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let over = Xf::new(rect, app.view().unwrap()).to_screen(Point::new(150.0, 150.0));
        let layer = egui::epaint::ColorMode::Solid(c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id))));
        // The hover outline: 1.5 px lines in the layer colour.
        let outlines = |app: &mut VectorcraftApp| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(over)]);
            shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == 1.5 && ps.stroke.color == layer)).count()
        };
        assert!(outlines(&mut app) > 0, "highlighted by default");
        app.session.execute("prefs.set", &json!({"key": "objectHighlighting", "value": false})).unwrap();
        assert_eq!(outlines(&mut app), 0, "the option off");
        app.session.execute("prefs.set", &json!({"key": "objectHighlighting", "value": true})).unwrap();
        app.ui.view.smart_guides = false;
        assert_eq!(outlines(&mut app), 0, "Smart Guides off");
    }

    /// Hide Corner Widget for angles greater than (#394): a rectangle's 90° corners lose their
    /// widgets below 90°.
    #[test]
    fn corner_widgets_hide_above_the_preference_angle() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        let widgets = |app: &mut VectorcraftApp| {
            shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == 3.0 && c.fill == Color32::WHITE)).count()
        };
        assert_eq!(widgets(&mut app), 4);
        app.session.execute("prefs.set", &json!({"key": "hideCornerWidgetAbove", "value": 80})).unwrap();
        assert_eq!(widgets(&mut app), 0);
    }

    /// #511: a star shows a widget in each of its ten corners with Direct Selection (none with
    /// the Selection tool: it isn't a live shape); View → Hide Corner Widget hides them.
    #[test]
    fn a_star_shows_corner_widgets_with_direct_selection() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.star", &json!({"cx": 200, "cy": 150, "radius1": 80, "radius2": 40})).unwrap();
        let ctx = egui::Context::default();
        let widgets = |app: &mut VectorcraftApp| {
            shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == 3.0 && c.fill == Color32::WHITE)).count()
        };
        app.select_tool("selection");
        assert_eq!(widgets(&mut app), 0);
        app.select_tool("directSelection");
        assert_eq!(widgets(&mut app), 10);
        app.run("view.cornerWidget", json!({})).unwrap();
        assert_eq!(widgets(&mut app), 0);
    }

    /// The type widget beside selected type's bounding box: hollow (white) on point type, filled
    /// in the layer colour on area type.
    #[test]
    fn the_type_widget_shows_which_kind_the_type_is() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let layer = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id)));
        let ctx = egui::Context::default();
        let widget = |app: &mut VectorcraftApp| -> Vec<Color32> {
            let r = vectorcraft_tools::typewidget::RADIUS_PX;
            shapes(app, &ctx)
                .iter()
                .filter_map(|s| if let Shape::Circle(c) = s { (c.radius == r && c.fill != Color32::TRANSPARENT).then_some(c.fill) } else { None })
                .collect()
        };
        assert_eq!(widget(&mut app), vec![Color32::WHITE]);
        app.session.execute("type.convertToAreaType", &json!({})).unwrap();
        assert_eq!(widget(&mut app), vec![layer]);
        app.ui.view.bounding_box = false;
        assert!(widget(&mut app).is_empty(), "no bounding box, no widget");
    }

    /// General › Zoom with Mouse Wheel (#394), through the control channel's `ui.wheel`: off,
    /// the wheel scrolls and Cmd-wheel zooms; on, the wheel zooms about the pointer, Shift-wheel
    /// scrolls up and down and Cmd/Ctrl-wheel sideways. Alt-wheel (Option on the Mac) and a
    /// trackpad pinch zoom about the pointer either way.
    #[test]
    fn zoom_with_mouse_wheel_preference() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        // A canvas frame through the host's input hook (it feeds the control channel's input).
        let run = |app: &mut VectorcraftApp| {
            let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
            app.raw_input_hook(&mut raw);
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        run(&mut app);
        let rect = app.canvas_rect.unwrap();
        let at = rect.center() + vec2(60.0, 40.0);
        // One notch up at `at` with `mods` held, or a pinch: → (zoom ratio, scroll in points, how
        // far the point under `at` moved).
        let turn = |app: &mut VectorcraftApp, mods: &str| {
            let before = *app.view().unwrap();
            if mods == "pinch" {
                app.synthetic.extend([egui::Event::PointerMoved(at), egui::Event::Zoom(1.25)]);
            } else {
                let mut p = json!({"x": at.x, "y": at.y, "dy": 1});
                for m in mods.split('+').filter(|m| !m.is_empty()) {
                    p[m] = json!(true);
                }
                crate::tests_synthetic::control(app, &ctx, "ui.wheel", p);
            }
            // egui spreads a notch over a few frames.
            for _ in 0..40 {
                run(app);
            }
            let after = *app.view().unwrap();
            let moved = Xf::new(rect, &after).to_doc(at).distance(Xf::new(rect, &before).to_doc(at));
            (after.zoom / before.zoom, (after.center - before.center) * after.zoom, moved)
        };
        let zooms_in = |(zoom, _, moved): (f64, _, f64)| zoom > 1.01 && moved < 1e-6;
        let (zoom, scroll, _) = turn(&mut app, "");
        assert!((zoom - 1.0).abs() < 1e-9 && scroll.x == 0.0 && scroll.y < -1.0, "off: the wheel scrolls up ({zoom}, {scroll:?})");
        assert!(zooms_in(turn(&mut app, "cmd")), "off: Cmd-wheel zooms in about the pointer");
        assert!(zooms_in(turn(&mut app, "alt")), "off: Alt-wheel zooms in about the pointer");
        assert!(zooms_in(turn(&mut app, "pinch")), "off: a pinch zooms in about the pointer");
        app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": true})).unwrap();
        assert!(zooms_in(turn(&mut app, "")), "on: the wheel zooms in about the pointer");
        assert!(zooms_in(turn(&mut app, "alt")), "on: so does Alt-wheel");
        assert!(zooms_in(turn(&mut app, "pinch")), "on: and a pinch");
        let (zoom, scroll, _) = turn(&mut app, "shift");
        assert!((zoom - 1.0).abs() < 1e-9 && scroll.x == 0.0 && scroll.y < -1.0, "on: Shift-wheel scrolls up ({zoom}, {scroll:?})");
        let (zoom, scroll, _) = turn(&mut app, "cmd");
        assert!((zoom - 1.0).abs() < 1e-9 && scroll.x < -1.0 && scroll.y == 0.0, "on: Cmd-wheel scrolls sideways ({zoom}, {scroll:?})");
    }

    /// #888: with Zoom with Mouse Wheel, a wheel notch's zoom glides in over a few frames, as a
    /// scroll does, instead of jumping on the frame it arrives; it ends at the same zoom.
    #[test]
    fn wheel_notches_zoom_in_a_glide() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": true})).unwrap();
        let ctx = egui::Context::default();
        let run = |app: &mut VectorcraftApp| {
            let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
            app.raw_input_hook(&mut raw);
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        run(&mut app);
        let at = app.canvas_rect.unwrap().center();
        let before = app.view().unwrap().zoom;
        crate::tests_synthetic::control(&mut app, &ctx, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 1}));
        // The first frame the zoom moves in (the event may land a frame later).
        let mut first = 1.0;
        for _ in 0..3 {
            run(&mut app);
            first = app.view().unwrap().zoom / before;
            if first != 1.0 {
                break;
            }
        }
        for _ in 0..60 {
            run(&mut app);
        }
        let done = app.view().unwrap().zoom / before;
        assert!(done > 1.01, "the notch zooms in ({done})");
        assert!(first > 1.0 && first < done, "the first frame takes only part of it ({first} of {done})");
        run(&mut app);
        assert_eq!(app.view().unwrap().zoom / before, done, "and it stops");
    }

    /// Two fingers dragged together on a touch screen pan the canvas with them, under either
    /// Zoom with Mouse Wheel setting, and draw nothing (#449).
    #[test]
    fn two_fingers_dragged_together_pan_the_canvas() {
        for wheel_zooms in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
            app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": wheel_zooms})).unwrap();
            app.select_tool("rectangle");
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            let c = app.canvas_rect.unwrap().center();
            let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
            let fingers = |phase, d: egui::Vec2| vec![touch(1, phase, c - vec2(40.0, 0.0) + d), touch(2, phase, c + vec2(40.0, 0.0) + d)];
            let before = *app.view().unwrap();
            // egui-winit moves the pointer with the first finger, as here.
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(c - vec2(40.0, 0.0))]);
            frame(&mut app, &ctx, fingers(egui::TouchPhase::Start, Vec2::ZERO));
            for k in 1..=3 {
                frame(&mut app, &ctx, fingers(egui::TouchPhase::Move, vec2(10.0, 8.0) * k as f32));
            }
            frame(&mut app, &ctx, fingers(egui::TouchPhase::End, vec2(30.0, 24.0)));
            let after = *app.view().unwrap();
            let moved = (after.center - before.center) * after.zoom;
            assert!((after.zoom / before.zoom - 1.0).abs() < 1e-9, "no zoom: {wheel_zooms}");
            assert!((moved.x + 30.0).abs() < 0.5 && (moved.y + 24.0).abs() < 0.5, "the content follows the fingers ({wheel_zooms}): {moved:?}");
            assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 0, "nothing drawn");
        }
    }

    /// General › Display Print Size at 100% Zoom (#394): `view.actualSize` is one document point per
    /// screen point with it off, and 96 screen points per document inch with it on.
    #[test]
    fn view_actual_size_follows_display_print_size() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        app.canvas_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(460.0, 260.0)));
        app.run("view.setZoom", json!({"zoom": 250})).unwrap();
        app.run("view.actualSize", json!({})).unwrap();
        assert!((app.view().unwrap().zoom - 1.0).abs() < 1e-12, "off by default: 100% is 1:1");
        app.session.execute("prefs.set", &json!({"key": "displayPrintSize", "value": true})).unwrap();
        app.run("view.actualSize", json!({})).unwrap();
        assert!((app.view().unwrap().zoom - 96.0 / 72.0).abs() < 1e-12, "an inch is 96 screen points");
    }

    /// Performance › Animated Zoom (#394): the Zoom tool dragged sideways zooms about where it was
    /// pressed (right in, left out), held still it zooms on, and a click still steps. Off, or with
    /// GPU Performance off, a drag zooms to the area dragged across.
    #[test]
    fn animated_zoom_scrubs_and_holds_else_the_zoom_tool_zooms_to_an_area() {
        use egui::Event;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("zoom");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let at = rect.center() - vec2(100.0, 50.0);
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        // Press at `at`, move by `by` and hold for `frames` frames (a 60th of a second each), let go.
        let press = |app: &mut VectorcraftApp, by: egui::Vec2, frames: usize| {
            frame(app, &ctx, vec![Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(at, true)]);
            frame(app, &ctx, vec![Event::PointerMoved(at + by)]);
            for _ in 0..frames {
                frame(app, &ctx, vec![]);
            }
            frame(app, &ctx, vec![button(at + by, false)]);
        };
        let doc_at = |app: &VectorcraftApp, p: Pos2| Xf::new(rect, app.view().unwrap()).to_doc(p);
        let pinned = doc_at(&app, at);
        let z0 = app.view().unwrap().zoom;
        press(&mut app, vec2(100.0, 0.0), 0);
        let z1 = app.view().unwrap().zoom;
        assert!((z1 / z0 - 2.0).abs() < 1e-6, "100 points right doubles the zoom: {z0} → {z1}");
        assert!(doc_at(&app, at).distance(pinned) < 1e-6, "about the press");
        press(&mut app, vec2(-100.0, 0.0), 0);
        assert!((app.view().unwrap().zoom - z0).abs() < 1e-6, "and back to the left");
        press(&mut app, vec2(0.0, 0.0), 0);
        assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(z0, true), "a click steps");
        let z2 = app.view().unwrap().zoom;
        press(&mut app, vec2(0.0, 0.0), 40);
        assert!(app.view().unwrap().zoom > z2 * 1.2, "held, it zooms on");
        assert!(doc_at(&app, at).distance(pinned) < 1e-6, "about the press");
        // Off (or without GPU Performance): the area dragged across fills the view.
        for key in ["animatedZoom", "gpuPerformance"] {
            app.run("prefs.reset", json!({})).unwrap();
            app.run("prefs.set", json!({"key": key, "value": false})).unwrap();
            app.view_mut().unwrap().zoom = z0;
            press(&mut app, vec2(100.0, 50.0), 0);
            let z = app.view().unwrap().zoom;
            assert!((z - z0 * (rect.width() / 100.0).min(rect.height() / 50.0) as f64).abs() < 1e-6, "{key} off: {z0} → {z}");
        }
    }

    /// User Interface › Large Tabs (#394): the document tabs are taller, so the canvas starts lower.
    #[test]
    fn large_tabs_make_the_document_tabs_taller() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let canvas_top = |app: &mut VectorcraftApp| {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                crate::chrome::doc_tabs(app, ui);
                show(app, ui);
            });
            out.textures_delta.clear();
            app.canvas_rect.unwrap().top()
        };
        let small = canvas_top(&mut app);
        app.run("prefs.set", json!({"key": "largeTabs", "value": true})).unwrap();
        assert_eq!(canvas_top(&mut app) - small, 9.0, "44 points instead of 35");
    }

    #[test]
    fn cmd_space_zooms_and_space_pans_whatever_the_tool() {
        use egui::{Event, Modifiers};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let at = app.canvas_rect.unwrap().center();
        let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
        let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        // A drag from `at` to `to` with Space and `held` down.
        let drag = |app: &mut VectorcraftApp, held: Modifiers, to: Pos2| {
            frame(app, &ctx, vec![Event::ModifiersChanged(held), space(true, held), Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(at, true, held)]);
            frame(app, &ctx, vec![Event::PointerMoved(to)]);
            frame(app, &ctx, vec![button(to, false, held)]);
            frame(app, &ctx, vec![space(false, held), Event::ModifiersChanged(Modifiers::NONE)]);
        };
        let z0 = app.view().unwrap().zoom;
        drag(&mut app, Modifiers::COMMAND, at);
        let z1 = app.view().unwrap().zoom;
        assert_eq!(z1, crate::state::next_zoom(z0, true), "Cmd+Space click zooms in");
        drag(&mut app, Modifiers::COMMAND | Modifiers::ALT, at);
        assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(z1, false), "Cmd+Alt+Space click zooms out");
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool drew nothing");
        // Space alone pans, with the Zoom tool too.
        app.select_tool("zoom");
        let before = *app.view().unwrap();
        drag(&mut app, Modifiers::NONE, at + vec2(30.0, 0.0));
        let after = *app.view().unwrap();
        assert_eq!(after.zoom, before.zoom);
        assert!((after.center.x - (before.center.x - 30.0 / before.zoom)).abs() < 1e-6, "{before:?} → {after:?}");
    }

    #[test]
    fn space_pressed_mid_drag_moves_what_the_tool_draws() {
        use egui::{Event, Modifiers};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.ui.view.smart_guides = false;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let space = |pressed| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        // Press at `a`, drag to `b`, then hold Space while moving on by `by` and let go of both.
        let draw = |app: &mut VectorcraftApp, a: Pos2, b: Pos2, by: egui::Vec2| {
            frame(app, &ctx, vec![Event::PointerMoved(a)]);
            frame(app, &ctx, vec![button(a, true)]);
            frame(app, &ctx, vec![Event::PointerMoved(b)]);
            frame(app, &ctx, vec![space(true)]);
            frame(app, &ctx, vec![Event::PointerMoved(b + by)]);
            frame(app, &ctx, vec![space(false)]);
            frame(app, &ctx, vec![button(b + by, false)]);
        };
        let near = |p: Point, q: Point| p.distance(q) < 1e-3;
        let at = app.canvas_rect.unwrap().center() - vec2(100.0, 60.0);
        let center = app.view().unwrap().center;
        // The rectangle moves at its size instead of growing, and the view doesn't pan.
        app.select_tool("rectangle");
        draw(&mut app, at, at + vec2(80.0, 40.0), vec2(30.0, 20.0));
        assert_eq!(app.view().unwrap().center, center, "Space during a drag doesn't pan");
        let st = app.session.active().unwrap();
        let r = st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().bounds().unwrap();
        assert!(
            near(Point::new(r.x0, r.y0), xf.to_doc(at + vec2(30.0, 20.0))) && near(Point::new(r.x1, r.y1), xf.to_doc(at + vec2(110.0, 60.0))),
            "{r:?}"
        );
        // The Pen's anchor being dragged out moves, handles and all.
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("pen");
        let a = at + vec2(0.0, 120.0);
        draw(&mut app, a, a + vec2(40.0, 0.0), vec2(20.0, 10.0));
        let st = app.session.active().unwrap();
        let anchor = st.doc.node(st.selection.objects[0]).unwrap().path_data().unwrap().subpaths[0].anchors[0];
        assert!(near(anchor.p, xf.to_doc(a + vec2(20.0, 10.0))) && near(anchor.h_out, xf.to_doc(a + vec2(60.0, 10.0))), "{anchor:?}");
    }

    #[test]
    fn double_clicking_type_with_the_selection_tool_puts_the_caret_there() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let mut out =
                ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        timed(&mut app, 0.0, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        };
        // Double-click the middle of the text.
        let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        let at = xf.to_screen(b.center());
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
        timed(&mut app, 1.1, vec![]);
        // The Type tool edits the text, with the caret inside it, where it was clicked: typing goes
        // into the text, not to tool shortcuts (X would swap fill and stroke).
        assert_eq!(app.session.tool_id(), "type");
        assert!(app.session.tool_wants_text());
        let o = app.session.tool_options();
        assert_eq!(o["editing"], json!(id));
        let caret = o["caret"].as_u64().unwrap();
        assert!((1..11).contains(&caret), "caret {caret}");
        app.session.tool_text("X", app.view_info()).unwrap();
        let t = text(&app);
        assert_eq!((t.len(), t.find('X')), (12, Some(caret as usize)), "{t}");
    }

    #[test]
    fn a_symbol_dropped_on_the_canvas_is_placed_there() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
        let name = app.session.execute("symbol.new", &json!({})).unwrap()["name"].as_str().unwrap().to_string();
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        // Released over empty canvas: an instance centred there.
        let at = xf.to_screen(Point::new(250.0, 180.0));
        egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Symbol(name.clone()));
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), up]);
        let st = app.session.active().unwrap();
        let placed = st.selection.objects.first().copied().unwrap();
        let n = st.doc.node(placed).unwrap();
        assert!(matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if *symbol == name));
        let c = n.geometric_bounds().unwrap().center();
        assert!((c - xf.to_doc(at)).hypot() < 1e-6, "{c:?}");
    }

    #[test]
    fn a_brush_dropped_on_a_path_is_applied_to_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id =
            app.session.execute("path.create", &json!({"anchors": [{"x": 100, "y": 150}, {"x": 300, "y": 150}]})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let brush = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.stroke().and_then(|s| s.brush.clone())
        };
        let drop = |app: &mut VectorcraftApp, at: Point| {
            let at = xf.to_screen(at);
            egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Brush { name: "Arrow".into(), def: json!({}) });
            let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at), up]);
        };
        // Off the path: nothing.
        drop(&mut app, Point::new(200.0, 250.0));
        assert_eq!(brush(&app), None);
        // On it: the path's stroke takes the brush.
        drop(&mut app, Point::new(200.0, 150.0));
        assert_eq!(brush(&app).as_deref(), Some("Arrow"));
    }

    #[test]
    fn middle_drag_pans_the_view_with_any_tool() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // A drawing tool: the middle button must pan, never draw.
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(400.0, 300.0))]);
        let before = *app.view().unwrap();
        frame(&mut app, &ctx, vec![middle(pos2(400.0, 300.0), true)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(450.0, 330.0))]);
        frame(&mut app, &ctx, vec![middle(pos2(450.0, 330.0), false)]);
        let after = *app.view().unwrap();
        assert_eq!(after.zoom, before.zoom);
        assert!((after.center.x - (before.center.x - 50.0 / before.zoom)).abs() < 1e-6);
        assert!((after.center.y - (before.center.y - 30.0 / before.zoom)).abs() < 1e-6);
        // Released: moving no longer pans, and nothing was drawn.
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(300.0, 200.0))]);
        assert_eq!(app.view().unwrap().center, after.center);
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), None);
    }

    #[test]
    fn swatches_and_proxy_paints_dropped_on_art_fill_the_object_hit() {
        use vectorcraft_color::{Color, GradientGeom, GradientPaint, Paint};
        use widgets::{PanelDrag, SwatchRows};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let drop = |app: &mut VectorcraftApp, d: PanelDrag, at: Point| {
            let at = xf.to_screen(at);
            egui::DragAndDrop::set_payload(&ctx, d);
            let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at), up]);
        };
        let fill = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
        let red = Paint::solid(Color::from_hex("#ed1c24").unwrap());
        let rows = SwatchRows { grabbed: "Red".into(), names: vec!["Red".into()], groups: false };
        let swatch = PanelDrag::Paint { paint: red.clone(), params: json!({"swatch": "Red"}), rows: Some(rows) };
        // Off the art nothing happens.
        drop(&mut app, swatch.clone(), Point::new(300.0, 250.0));
        assert_eq!(fill(&app), Paint::solid(Color::WHITE));
        drop(&mut app, swatch, Point::new(100.0, 100.0));
        assert_eq!(fill(&app), red, "paint.setFill with the hit id");
        assert!(app.session.active().unwrap().selection.is_empty(), "the selection stays as it was");
        // A proxy's paint works the same.
        drop(&mut app, PanelDrag::paint(Paint::solid(Color::rgb(0.0, 0.0, 1.0))), Point::new(60.0, 60.0));
        assert_eq!(fill(&app), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
        // A dragged gradient (the Gradient panel's thumbnail, a proxy) fits the object it lands on.
        let mut g = GradientPaint::new(Default::default());
        g.geom = Some(GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None });
        drop(&mut app, PanelDrag::paint(Paint::Gradient(Box::new(g))), Point::new(100.0, 100.0));
        assert!(matches!(fill(&app), Paint::Gradient(g) if g.geom.is_none()));
    }

    #[test]
    fn dropping_the_appearance_thumbnail_on_art_copies_the_appearance() {
        use vectorcraft_doc::NodeId;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 400, "height": 300}));
        let a = run(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap();
        run(&mut app, "effect.apply", json!({"effect": "distort.twist"}));
        run(&mut app, "transparency.set", json!({"opacity": 40}));
        let b = run(&mut app, "shape.rectangle", json!({"x": 200, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap();
        run(&mut app, "select.set", json!({ "ids": [a] }));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        // The Appearance panel's thumbnail (its drag payload) released over the second rectangle.
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let drop = |app: &mut VectorcraftApp, at: Point| {
            let at = xf.to_screen(at);
            egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::Appearance(NodeId(a)));
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
            frame(app, &ctx, vec![up]);
        };
        drop(&mut app, Point::new(250.0, 70.0));
        let doc = &app.session.active().unwrap().doc;
        let (na, nb) = (doc.node(NodeId(a)).unwrap(), doc.node(NodeId(b)).unwrap());
        assert_eq!((&nb.appearance, nb.opacity), (&na.appearance, na.opacity));
        assert_eq!(nb.appearance.effects[0].id, "distort.twist");
        // Dropped on empty canvas or on the source itself: nothing changes.
        let undo_len = app.session.active().unwrap().history.undo.len();
        drop(&mut app, Point::new(350.0, 250.0));
        drop(&mut app, Point::new(70.0, 70.0));
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo_len);
    }

    #[test]
    fn a_graphic_style_dropped_on_art_applies_to_the_object_hit() {
        use vectorcraft_doc::NodeId;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 400, "height": 300}));
        let a = NodeId(run(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap());
        let b = NodeId(run(&mut app, "shape.rectangle", json!({"x": 200, "y": 20, "width": 100, "height": 100}))["id"].as_u64().unwrap());
        run(&mut app, "select.set", json!({ "ids": [a.0] }));
        let name = app.session.active().unwrap().doc.graphic_styles[2].name.clone();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let at = xf.to_screen(Point::new(250.0, 70.0));
        egui::DragAndDrop::set_payload(&ctx, widgets::PanelDrag::GraphicStyle(name.clone()));
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        let up = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![up]);
        let st = app.session.active().unwrap();
        let g = st.doc.graphic_style(&name).unwrap();
        // `graphicStyle.apply` with the hit object's id: linked, the selection left alone.
        assert_eq!(st.doc.node(b).unwrap().graphic_style, Some(g.id));
        assert_eq!(st.doc.node(b).unwrap().appearance, g.appearance);
        assert_eq!(st.doc.node(a).unwrap().graphic_style, None);
        assert_eq!(st.selection.objects, [a]);
    }

    #[test]
    fn art_moved_off_the_canvas_becomes_a_panel_drag_and_stays_put() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        app.select_tool("selection");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let start = xf.to_screen(Point::new(100.0, 100.0));
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, true)]);
        // Moving on the canvas moves the art...
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start + vec2(40.0, 0.0))]);
        assert!(app.session.active().unwrap().interaction.is_some());
        // ...until the pointer leaves it: the move is dropped and the panels get the art.
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(pos2(850.0, 100.0))]);
        assert!(app.session.active().unwrap().interaction.is_none());
        assert_eq!(*app.session.active().unwrap().doc, *before, "the art is back where it was");
        assert_eq!(egui::DragAndDrop::payload::<widgets::PanelDrag>(&ctx).as_deref(), Some(&widgets::PanelDrag::Art(vec![id])));
        assert!(!app.session.tool_busy());
        // Released anywhere, nothing moves and nothing is recorded.
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start), button(start, false)]);
        let st = app.session.active().unwrap();
        assert_eq!(*st.doc, *before);
        assert_eq!(st.history.undo.len(), undo);
    }

    /// One frame of keyboard handling and canvas, as the app runs them; returns the IME output.
    fn typing_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Option<egui::output::IMEOutput> {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            show(app, ui);
        });
        out.textures_delta.clear();
        out.platform_output.ime
    }

    fn preedit(t: &str, chars: usize) -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::Preedit { text: t.into(), active_range_chars: Some(chars..chars) })
    }

    fn plain(app: &VectorcraftApp, id: u64) -> String {
        match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        }
    }

    #[test]
    fn japanese_ime_composes_on_the_canvas_with_its_window_at_the_caret() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        // The type placed starts empty (Fill New Type Objects With Placeholder Text off).
        app.session.prefs.placeholder_text = false;
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        // Not editing: the IME stays off (single-key tool shortcuts keep working).
        assert!(typing_frame(&mut app, &ctx, vec![]).is_none());
        app.select_tool("type");
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let at = xf.to_screen(Point::new(100.0, 100.0));
        let click =
            |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
        typing_frame(&mut app, &ctx, vec![click(at, false)]);
        assert!(app.session.tool_wants_text());
        let id = app.session.active().unwrap().selection.objects[0].0;
        // Editing: the IME is allowed, its window at the caret (the text's baseline at y = 100).
        let ime = typing_frame(&mut app, &ctx, vec![]).expect("IME allowed while editing");
        assert!(ime.rect.contains(pos2(ime.rect.center().x, at.y)) && (ime.rect.center().x - at.x).abs() < 2.0, "{:?} vs {at:?}", ime.rect);
        assert!(!ime.should_interrupt_composition);
        // gagaku → がが → 雅楽, then commit (the macOS sequence).
        typing_frame(&mut app, &ctx, vec![preedit("g", 1)]);
        typing_frame(&mut app, &ctx, vec![preedit("が", 1), preedit("がg", 2)]);
        typing_frame(&mut app, &ctx, vec![preedit("ががく", 3)]);
        assert_eq!(plain(&app, id), "ががく");
        // While composing: plain text events, the clipboard and Undo (the native menu's ⌘Z
        // reaches `menus::invoke`) all wait.
        typing_frame(&mut app, &ctx, vec![egui::Event::Text("x".into()), egui::Event::Paste("y".into())]);
        crate::menus::invoke(&mut app, "edit.undo", json!({}));
        assert!(!crate::menus::enabled(&app, "edit.undo"));
        assert_eq!(plain(&app, id), "ががく");
        let ime2 =
            typing_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Preedit { text: "雅楽".into(), active_range_chars: Some(0..2) })])
                .unwrap();
        assert!(ime2.rect.center().x <= ime.rect.center().x + 1.0, "the window stays at the clause being converted");
        typing_frame(&mut app, &ctx, vec![preedit("", 0), egui::Event::Ime(egui::ImeEvent::Commit("雅楽".into()))]);
        assert_eq!(plain(&app, id), "雅楽");
        assert!(!app.session.tool_composing());
        // After the commit, text and Undo work again.
        typing_frame(&mut app, &ctx, vec![egui::Event::Text("!".into())]);
        assert_eq!(plain(&app, id), "雅楽!");
        crate::menus::invoke(&mut app, "edit.undo", json!({}));
        assert_eq!(plain(&app, id), "");
    }

    #[test]
    fn clicking_away_mid_composition_keeps_the_text_and_interrupts_the_ime() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        typing_frame(&mut app, &ctx, vec![]);
        app.select_tool("type");
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let (a, b) = (xf.to_screen(Point::new(100.0, 100.0)), xf.to_screen(Point::new(300.0, 250.0)));
        let click =
            |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(a), click(a, true)]);
        typing_frame(&mut app, &ctx, vec![click(a, false)]);
        let id = app.session.active().unwrap().selection.objects[0].0;
        typing_frame(&mut app, &ctx, vec![preedit("しょうこ", 4)]);
        // macOS sends nothing for a click away; the tool keeps the marked text and the IME is
        // told to drop its composition.
        typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b), click(b, true)]);
        let ime = typing_frame(&mut app, &ctx, vec![click(b, false)]);
        assert_eq!(plain(&app, id), "しょうこ");
        assert!(!app.session.tool_composing());
        assert!(ime.expect("editing the new text").should_interrupt_composition);
        assert!(app.ime_marked.is_none());
        assert!(app.take_ime_discard(), "the host tells the system IME to drop its marked text");
        // Interrupted once, not every frame.
        assert!(!typing_frame(&mut app, &ctx, vec![]).unwrap().should_interrupt_composition);
        assert!(!app.take_ime_discard());
    }

    #[test]
    fn ime_edge_orders_never_eat_committed_text_or_lose_the_composition() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        typing_frame(&mut app, &ctx, vec![]);
        app.select_tool("type");
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let at = xf.to_screen(Point::new(100.0, 100.0));
        let click =
            |p: Pos2, pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        typing_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at), click(at, true)]);
        typing_frame(&mut app, &ctx, vec![click(at, false)]);
        let id = app.session.active().unwrap().selection.objects[0].0;
        typing_frame(&mut app, &ctx, vec![egui::Event::Text("雅".into())]);
        let backspace =
            |pressed| egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        // The IME empties its marked text and the Backspace comes in the same frame.
        typing_frame(&mut app, &ctx, vec![preedit("が", 1)]);
        typing_frame(&mut app, &ctx, vec![preedit("", 0), backspace(true), backspace(false)]);
        assert_eq!(plain(&app, id), "雅", "the committed character stays");
        // A bare line-break commit ends the composition with the marked text, no newline.
        typing_frame(&mut app, &ctx, vec![preedit("がく", 2)]);
        typing_frame(&mut app, &ctx, vec![egui::Event::Ime(egui::ImeEvent::Commit("\n".into()))]);
        assert!(!app.session.tool_composing());
        assert_eq!(plain(&app, id), "雅がく");
        assert!(app.take_ime_discard());
        // Switching apps mid-composition: the composition goes on when the window comes back.
        typing_frame(&mut app, &ctx, vec![preedit("らく", 2)]);
        typing_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(false)]);
        typing_frame(&mut app, &ctx, vec![egui::Event::WindowFocused(true), preedit("らくか", 3)]);
        assert!(app.session.tool_composing());
        assert!(!app.take_ime_discard());
        assert_eq!(plain(&app, id), "雅がくらくか");
    }

    /// The events eframe's winit integration sends for a pen (Windows Ink) or a finger: a touch
    /// with its pressure, and the mouse it stands in for (#491).
    fn pen(phase: egui::TouchPhase, pos: Pos2, force: f32) -> Vec<egui::Event> {
        let touch = egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos, force: Some(force) };
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        match phase {
            egui::TouchPhase::Start => vec![touch, egui::Event::PointerMoved(pos), button(true)],
            egui::TouchPhase::Move => vec![touch, egui::Event::PointerMoved(pos)],
            egui::TouchPhase::End | egui::TouchPhase::Cancel => vec![touch, button(false), egui::Event::PointerGone],
        }
    }

    /// A pen stroke across `pts` (document points), one frame per sample, with its pressures.
    fn pen_stroke(app: &mut VectorcraftApp, ctx: &egui::Context, pts: &[(f64, f64, f32)]) {
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let last = pts.len() - 1;
        for (i, &(x, y, force)) in pts.iter().enumerate() {
            let phase = if i == 0 { egui::TouchPhase::Start } else { egui::TouchPhase::Move };
            frame(app, ctx, pen(phase, xf.to_screen(Point::new(x, y)), force));
            if i == last {
                frame(app, ctx, pen(egui::TouchPhase::End, xf.to_screen(Point::new(x, y)), force));
            }
        }
    }

    /// #491: a pen or a finger draws as the mouse does, and its pressure reaches the tools that
    /// use it; the pointer leaving with the lift doesn't move the stroke's end.
    #[test]
    fn pen_and_touch_input_drive_the_tools_with_their_pressure() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("pencil");
        pen_stroke(&mut app, &ctx, &[(100.0, 100.0, 0.5), (140.0, 120.0, 0.6), (180.0, 150.0, 0.7), (220.0, 200.0, 0.4)]);
        let b = app.session.active().unwrap().doc.art_bounds().expect("the pen drew a path");
        assert!(b.x0 > 90.0 && b.y0 > 90.0 && b.x1 < 230.0 && b.y1 < 210.0 && b.x1 > 200.0, "the stroke stays where the pen went: {b:?}");

        // Bloat with Use Pressure Pen: each sample carries the pen's pressure.
        app.session.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 200, "height": 200})).unwrap();
        app.select_tool("bloat");
        app.session.set_tool_option("usePressure", &json!(true));
        pen_stroke(&mut app, &ctx, &[(500.0, 350.0, 0.2), (500.0, 400.0, 0.5), (500.0, 420.0, 0.8)]);
        let (cmd, p) = app.session.journal.last().unwrap().clone();
        assert_eq!(cmd, "object.liquify");
        let pressures: Vec<f64> = p["points"].as_array().unwrap().iter().map(|s| (s[2].as_f64().unwrap() * 100.0).round() / 100.0).collect();
        assert_eq!(pressures.first(), Some(&0.2), "the press's own pressure: {pressures:?}");
        assert!(pressures.contains(&0.5) && pressures.contains(&0.8), "{pressures:?}");
    }

    /// #491: a pen tap whose press and lift come in one frame selects what it tapped, and the
    /// lift (the pointer gone with it) doesn't drag the selection to the canvas's centre.
    #[test]
    fn a_pen_tap_within_one_frame_clicks_where_it_tapped() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("selection");
        let before = app.session.active().unwrap().doc.art_bounds();
        let at = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(150.0, 150.0));
        let mut tap = pen(egui::TouchPhase::Start, at, 0.5);
        tap.extend(pen(egui::TouchPhase::End, at, 0.5));
        frame(&mut app, &ctx, tap);
        frame(&mut app, &ctx, vec![]);
        let doc = app.session.active().unwrap();
        assert_eq!(doc.selection.objects, vec![vectorcraft_doc::NodeId(id)]);
        assert_eq!(doc.doc.art_bounds(), before, "the tap moved nothing");
    }

    /// The Contextual Task Bar's handle drags it: unpinned it keeps that offset as it follows the
    /// selection, pinned it stays put when the selection changes, unpinned again it follows from
    /// there, and Reset Bar Position puts it back under the selection. It never leaves the canvas,
    /// also when the window shrinks.
    #[test]
    fn the_task_bar_is_dragged_by_its_handle_pinned_and_reset() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let mut square =
            |x: f64, y: f64| app.session.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 40, "height": 40})).unwrap()["id"].clone();
        let (a, b) = (square(120.0, 40.0), square(220.0, 150.0));
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let bar = || task_bar_rect(&ctx).expect("the task bar shows");
        let select = |app: &mut VectorcraftApp, id: &serde_json::Value| {
            app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
            frame(app, &ctx, vec![]);
            frame(app, &ctx, vec![]);
            bar().min
        };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let drag = |app: &mut VectorcraftApp, by: Vec2| {
            let grip = pos2(bar().left() + 11.0, bar().center().y);
            frame(app, &ctx, vec![egui::Event::PointerMoved(grip)]);
            frame(app, &ctx, vec![button(grip, true)]);
            frame(app, &ctx, vec![egui::Event::PointerMoved(grip + by)]);
            frame(app, &ctx, vec![button(grip + by, false)]);
            frame(app, &ctx, vec![]);
            bar().min
        };
        let near = |p: Pos2, q: Pos2| assert!((p - q).length() < 1.5, "{p:?} is not at {q:?}");
        let (under_a, under_b) = (select(&mut app, &a), select(&mut app, &b));
        assert!(under_b.y > under_a.y + 100.0, "each bar sits under its selection");

        // Dragged unpinned: it keeps its offset from the selection.
        select(&mut app, &a);
        let by = vec2(40.0, -30.0);
        near(drag(&mut app, by), under_a + by);
        near(select(&mut app, &b), under_b + by);
        assert!(!app.ui.task_bar_place.pinned);

        // Pinned: it stays where it is across selection changes, and the handle still moves it.
        assert_eq!(app.run("window.taskBar.pin", json!({})).unwrap(), json!(true));
        assert_eq!(crate::menus::checked(&app, "window.taskBar.pin", &json!({})), Some(true));
        near(select(&mut app, &a), under_b + by);
        let spot = drag(&mut app, vec2(-20.0, 10.0));
        near(spot, under_b + by + vec2(-20.0, 10.0));
        near(select(&mut app, &b), spot);

        // Unpinned: it follows the selection from where it is, kept on the canvas.
        assert_eq!(app.run("window.taskBar.pin", json!({"pinned": false})).unwrap(), json!(false));
        near(select(&mut app, &a), spot);
        let canvas = app.canvas_rect.unwrap();
        let last = canvas.max - vec2(8.0, 8.0) - bar().size();
        near(select(&mut app, &b), (under_b + (spot - under_a)).min(last));
        assert!(app.run("window.taskBar.pin", json!({"pinned": "yes"})).is_err());

        // Kept on the canvas: dragged far off, then pinned in the corner of a shrinking window.
        near(drag(&mut app, vec2(-2000.0, -2000.0)), canvas.min + vec2(8.0, 8.0));
        app.run("window.taskBar.pin", json!({"pinned": true})).unwrap();
        drag(&mut app, vec2(2000.0, 2000.0));
        near(bar().max, canvas.max - vec2(8.0, 8.0));
        let small = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))), ..Default::default() };
        for _ in 0..2 {
            let mut out = ctx.run_ui(small.clone(), |ui| show(&mut app, ui));
            out.textures_delta.clear();
        }
        let canvas = app.canvas_rect.unwrap();
        assert!(canvas.width() < 700.0, "the canvas shrank: {canvas:?}");
        near(bar().max, canvas.max - vec2(8.0, 8.0));

        // Reset: back under the selection, unpinned.
        frame(&mut app, &ctx, vec![]);
        app.run("window.taskBar.reset", json!({})).unwrap();
        assert!(!app.ui.task_bar_place.pinned);
        near(select(&mut app, &a), under_a);
        near(select(&mut app, &b), under_b);
    }
}
