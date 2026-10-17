//! The document canvas: rendering, rulers, navigation, pointer routing to tools and on-canvas
//! selection visuals (bounding box, anchors, handles, smart-guide style labels).

use egui::{Color32, CornerRadius, Pos2, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::{Node, NodeKind, SymbolBoxes, Unit};
use vectorcraft_geom::{Affine, BezPath, PathEl, Point, Rect};
use vectorcraft_tools::{Cursor, Mods, Overlay, PointerEvent, PointerKind};

use crate::state::View;
use crate::theme::{self, Tokens};
use crate::{CacheKey, VectorcraftApp, now_ms, widgets};

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
    /// A Selection tool move dragged off the canvas: the panels get the art
    /// ([`widgets::PanelDrag::Art`]).
    Art,
    /// Fingers making a touch gesture ([`crate::touch`]): the first one's press does nothing.
    Gesture,
    /// A press in Presentation Mode: released as a click, it goes to the next artboard.
    Present,
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

/// The positions this frame's pointer moves passed through before its last one, each with the
/// pen force a touch event gave just before it (if any). The last move is the pointer's position.
/// Only for pens and fingers (frames with touch events): a mouse's moves are left as they were.
fn frame_moves(ui: &Ui) -> Vec<(Pos2, Option<f32>)> {
    ui.input(|i| {
        if !i.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })) {
            return vec![];
        }
        let mut force = None;
        let mut moves = vec![];
        for e in &i.events {
            match e {
                egui::Event::Touch { force: Some(f), .. } if f.is_finite() && *f > 0.0 => force = Some(f.clamp(0.0, 1.0)),
                egui::Event::PointerMoved(p) => moves.push((*p, force)),
                _ => {}
            }
        }
        moves.pop();
        // A burst of events (a stalled frame) is thinned to a bounded count.
        const MAX: usize = 64;
        if moves.len() > MAX {
            let step = moves.len().div_ceil(MAX);
            moves = moves.into_iter().step_by(step).collect();
        }
        moves
    })
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
        // The width and height of the box round the target turned with the view, at zoom 1.
        let (sn, cs) = app.view().map_or(0.0, |v| v.rotation.to_radians()).sin_cos();
        let (w, h) = (target.width(), target.height());
        let zx = (rect.width() as f64 - 60.0) / (w * cs.abs() + h * sn.abs()).max(1.0);
        let zy = (rect.height() as f64 - 60.0) / (w * sn.abs() + h * cs.abs()).max(1.0);
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
    // `hidden_chars` holds the marks Show Hidden Characters kept and the text nodes they were laid
    // out from. Only the assignment right before the overlay call below puts them back. A frame
    // that does not draw the overlay drops them, for example with the option off, with the edges
    // hidden, on the Home screen or with no document open.
    let hidden_chars = std::mem::take(&mut app.canvas.hidden_chars);
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
    // Normal Screen Mode and Full Screen Mode with Menu Bar keep the scroll bars along the right
    // and bottom edges; Full Screen and Presentation Mode have none.
    let outer = full;
    let bars = app.ui.screen_mode < 2;
    let full = if bars { egui::Rect::from_min_max(full.min, full.max - vec2(SCROLL_BAR, SCROLL_BAR)) } else { full };
    let rect = if app.ui.view.rulers && app.ui.screen_mode < 3 { egui::Rect::from_min_max(full.min + vec2(RULER, RULER), full.max) } else { full };
    app.canvas_rect = Some(rect);
    let fitted = app.view().is_some_and(|v| v.fitted);
    if !fitted {
        fit(app, "view.fitArtboard");
    }
    // Before the canvas's own interaction, so the view the canvas draws is the one they moved to.
    if bars {
        scroll_bars(app, ui, rect, outer, &t);
    }
    let resp = ui.interact(rect, egui::Id::new("canvas"), Sense::click_and_drag());
    // A press on the canvas while the context menu is open only closes it (a drag too, which egui
    // alone would leave open); the tool doesn't get it.
    if app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active) {
        // The artboard's 3D gizmo owns these gestures while its options are open.
        // Do not let a ring drag also move or deselect the editable source path.
    } else if resp.context_menu_opened() && resp.hovered() && ui.input(|i| i.pointer.any_pressed()) {
        egui::Popup::close_all(ui.ctx());
    } else {
        handle_input(app, ui, &resp, rect);
    }
    let v = *app.view().unwrap_or(&View::default());
    let xf = Xf::new(rect, &v);
    let origin_preview = if app.ui.view.rulers && app.ui.screen_mode < 3 { ruler_input(app, ui, full, rect, &xf) } else { None };
    panel_drop(app, ui, &resp, &xf);
    // In Presentation Mode a right click or a long touch opens no context menu.
    if app.ui.screen_mode < 3 {
        context_menu(app, &resp, &xf);
    }
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
    // Paths made into guides are drawn as the ruler guides are (Preferences › Guides & Grid).
    let guide_look = LineLook::guides(&app.session.prefs, &t);
    let key = CacheKey {
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
        ppp,
        hidden: vec![],
        rot: v.rotation,
        anti_alias: app.session.prefs.anti_aliased_artwork,
        highlight_alternate_glyphs: app.session.prefs.highlight_alternate_glyphs,
        placed: vectorcraft_render::placed_document::generation(),
        pixel,
        smooth_images,
        isolated: st.isolation.map(|id| id.0),
        // Appearance of Black › On Screen: Display All Blacks as Rich Black.
        rich_black: app.session.prefs.black_on_screen == "rich",
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
        app.canvas.moved = done.moved > 0;
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
            hide_templates: !app.ui.view.templates,
            tile_edge: vectorcraft_color::Color::from_hex(&app.session.prefs.pattern_tile_edge_color).map_or(opts.tile_edge, |c| {
                let [r, g, b, _] = c.to_rgba8(1.0);
                [r, g, b]
            }),
            guide: key.guide,
            mask_view,
            highlight_substitutions: true,
            // Type › Highlight Alternate Glyphs (#394): pink behind OpenType substitutions.
            highlight_alternate_glyphs: app.session.prefs.highlight_alternate_glyphs,
            // General › Anti-aliased Artwork: off, edges are hard on screen (raster effects and
            // pattern tiles stay smooth), as in Illustrator.
            anti_alias: if app.session.prefs.anti_aliased_artwork { vectorcraft_render::AntiAlias::Art } else { vectorcraft_render::AntiAlias::None },
            progressive_placed: true,
            trace_views: true,
            smooth_images,
            // Isolation mode: the art around the isolated group or layer is dimmed (#833).
            isolated: key.isolated.map(vectorcraft_doc::NodeId),
            // Images far smaller on screen than they are sample their mip levels (#1244).
            image_mips: true,
            // During a drag or a dialog's preview, objects that only moved draw their raster
            // effects from the cached rasters of an earlier frame (#1257).
            interacting: key.interacting,
            rich_black: key.rich_black,
            ..opts
        };
        // Light documents render synchronously (no lag vs overlays); heavy ones go to the worker.
        // A document switch renders synchronously so another document's frame is never shown.
        // The first frame after an interaction, when the frame shown drew moved rasters, also goes
        // to the worker.
        let same_doc = app.canvas.key.as_ref().is_some_and(|k| k.doc == key.doc);
        let settling = app.canvas.moved && !key.interacting;
        let heavy = (app.canvas.last_ms > 8.0 || settling) && app.canvas.texture.is_some() && same_doc;
        match (&mut app.canvas.worker, heavy) {
            (Some(worker), true) => worker.submit(crate::render_worker::Job { key: key.clone(), doc: doc.clone(), w, h, view, opts }),
            _ => {
                let t0 = now_ms();
                let img = app.canvas.renderer.render(&doc, w, h, view, &opts);
                upload(app, ui.ctx(), &img, pixel.is_some());
                app.canvas.key = Some(key.clone());
                app.perf.render_ms = now_ms() - t0;
                app.canvas.last_ms = app.perf.render_ms;
                app.canvas.moved = app.canvas.renderer.stats.moved > 0;
            }
        }
    }
    if let (Some(tex), Some(k)) = (&app.canvas.texture, &app.canvas.key)
        && k.doc == key.doc
    {
        match k.pixel {
            // Document pixels: placed over the document rect they cover, turning with the view.
            Some([x0, y0, x1, y1]) => {
                textured_quad(&painter, tex.id(), &xf.quad(Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64)), (1.0, 1.0));
            }
            None if (k.rot - v.rotation).abs() < 1e-9 => {
                // Reproject the last frame if it was rendered for a different view.
                let old = Xf { rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: xf.rot };
                let a = xf.to_screen(old.to_doc(rect.min));
                let b = xf.to_screen(old.to_doc(rect.max));
                painter.image(tex.id(), egui::Rect::from_min_max(a, b), egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            // The view turned after the last frame was rendered, and the worker renders the frame at
            // the new angle. Until it arrives, the last frame is drawn turned into this view: each
            // corner goes where this view puts the document point that the corner showed.
            None => {
                let old = Xf { rect, zoom: k.zoom, center: Point::new(k.cx, k.cy), rot: k.rot.to_radians() };
                let q = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].map(|c| xf.to_screen(old.to_doc(c)));
                textured_quad(&painter, tex.id(), &q, (1.0, 1.0));
            }
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
        if editing_name != Some(i) && (artboard_tool || doc.artboards.len() > 1) && tool_labelled != u64::try_from(i).ok() {
            // At the artboard's own top-left corner, which a rotated view turns away from its
            // bounding box's (#1095).
            let corner = xf.to_screen(Point::new(ab.rect.x0, ab.rect.y0));
            artboard_label(&painter, corner, (ab.rect.width() * xf.zoom) as f32, &format!("{:02} - {}", i + 1, ab.name), t.text_dim);
        }
    }
    if app.ui.view.guides {
        let look = LineLook::guides(&app.session.prefs, &t);
        // Selected guides in the selection colour.
        let picked = LineLook { color: t.selection, ..look };
        let selected: &[usize] = app.session.active().map_or(&[], |st| &st.selection.guides);
        let view = view_bounds(&xf);
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
            if selected.contains(&i) { picked } else { look }.guide(&painter, a, b);
        }
    }

    if !app.session.slices_hidden() {
        slice_overlay(app, &painter, &xf);
    }
    if app.session.active().is_some_and(|d| d.print_tiling) || app.session.tool_id() == "printTiling" {
        print_tiling_overlay(app, &painter, &xf);
    }
    original_path_overlay(app, &painter, &xf);

    // Selection visuals and tool overlays.
    let revolve_edit = app.ui.dialog.as_ref().is_some_and(crate::dialogs::revolve_gizmo::active);
    let view_info = app.view_info();
    // View → Hide Gradient Annotator hides the Gradient tool's annotator.
    let mut overlays = if !revolve_edit && (app.session.tool_id() != "gradient" || app.ui.view.gradient_annotator) {
        app.session.overlays(view_info)
    } else {
        vec![]
    };
    // Gap Options' Preview shows the gaps it finds.
    overlays.extend(crate::dialogs::live_paint_gaps::overlays(app));
    if app.ui.view.edges && !revolve_edit {
        hover_highlight(app, &painter, &xf);
        selection_overlay(app, &painter, &xf, overlays.iter().any(|o| matches!(o, Overlay::CurvePoint { .. })));
        if app.ui.view.text_threads {
            thread_overlay(app, &painter, &xf);
        }
        if app.ui.view.hidden_chars {
            app.canvas.hidden_chars = hidden_chars;
            hidden_chars_overlay(app, &painter, &xf);
        }
    }
    draw_overlays(&painter, &xf, &overlays, &t, HandleLook::of(&app.session.prefs));
    if !revolve_edit {
        shaper_widget_overlay(app, &painter, &xf);
    }
    ime_output(app, ui.ctx(), &xf);
    crate::place::paint_drop_highlight(app, ui.ctx(), &painter, rect);

    if app.ui.view.rulers && app.ui.screen_mode < 3 {
        if let Some(origin) = origin_preview {
            let at = xf.to_screen(origin);
            painter.line_segment([pos2(at.x, rect.top()), pos2(at.x, rect.bottom())], Stroke::new(1.0, t.text_dim));
            painter.line_segment([pos2(rect.left(), at.y), pos2(rect.right(), at.y)], Stroke::new(1.0, t.text_dim));
        }
        rulers(ui, full, &xf, app.hover_doc, app.session.general_unit(), origin_preview.unwrap_or_else(|| app.ruler_origin()), &t);
    }
    if app.ui.task_bar && !revolve_edit && !app.session.tool_busy() && app.ui.screen_mode < 3 {
        task_bar(app, ui, &xf);
    }
    if app.ui.screen_mode < 3 {
        crate::free_transform::show(app, ui, xf.rect);
    }
    artboard_name_editor(app, ui, &xf);
    // Cursor.
    if resp.hovered() {
        let m = ui.input(|i| i.modifiers);
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let drag = ui.data(|d| d.get_temp::<Drag>(drag_id()));
        let panning = matches!(drag, Some(Drag::Pan { .. }));
        let zooming = if space { m.command } else { app.session.tool_id() == "zoom" };
        // A rotating drag keeps its cursor when Shift is let go.
        let rotating =
            matches!(drag, Some(Drag::RotateView { .. })) || if space { m.shift && !m.command } else { app.session.tool_id() == "rotateView" };
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
        let tool_cursor =
            (app.ui.screen_mode < 3).then(|| view_cursor.or_else(|| app.hover_doc.map(|p| app.session.cursor(p, mods(m, space), view_info))));
        let cur = if let Some(c) = tool_cursor.flatten() {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("tool-cursor")));
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
}

fn cursor_icon(c: Cursor) -> egui::CursorIcon {
    use egui::CursorIcon as C;
    match c {
        Cursor::Arrow
        | Cursor::ArrowHollow
        | Cursor::DirectAnchor
        | Cursor::DirectAnchorSelected
        | Cursor::ReshapeSegment
        | Cursor::GroupSelection
        | Cursor::CornerRadius
        | Cursor::PathBracket
        | Cursor::TypeWidget => C::Default,
        Cursor::SelectionCopy => C::Copy,
        Cursor::Move => C::Move,
        Cursor::Crosshair => C::Crosshair,
        Cursor::ResizeH | Cursor::ShearH => C::ResizeHorizontal,
        Cursor::ResizeV | Cursor::ShearV => C::ResizeVertical,
        Cursor::ResizeNwSe => C::ResizeNwSe,
        Cursor::ResizeNeSw => C::ResizeNeSw,
        Cursor::Rotate | Cursor::RotateToward(_) => C::Alias,
        Cursor::Pen
        | Cursor::PenDrawing
        | Cursor::PenAdd
        | Cursor::PenDelete
        | Cursor::PenClose
        | Cursor::PenContinue
        | Cursor::PenJoin
        | Cursor::PenConvert
        | Cursor::AnchorPoint
        | Cursor::Curvature => C::Crosshair,
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
        Cursor::TypeVertical => C::VerticalText,
        // Not drawn yet: the cursor each borrows.
        Cursor::Lasso
        | Cursor::MagicWand
        | Cursor::Pencil
        | Cursor::Paintbrush
        | Cursor::BlobBrush
        | Cursor::Eraser
        | Cursor::Smooth
        | Cursor::PathEraser
        | Cursor::Join
        | Cursor::Knife
        | Cursor::Shaper
        | Cursor::LivePaintBucket
        | Cursor::Measure
        | Cursor::PrintTiling
        | Cursor::SymbolSprayer
        | Cursor::SymbolShifter
        | Cursor::SymbolScruncher
        | Cursor::SymbolSizer
        | Cursor::SymbolSpinner
        | Cursor::SymbolStainer
        | Cursor::SymbolScreener
        | Cursor::SymbolStyler
        | Cursor::Scissors
        | Cursor::ScissorsNotAllowed
        | Cursor::MeshAdd
        | Cursor::PuppetPin
        | Cursor::LoadedText => cursor_icon(c.stand_in().unwrap_or(Cursor::Crosshair)),
    }
}

fn handle_input(app: &mut VectorcraftApp, ui: &Ui, resp: &egui::Response, rect: egui::Rect) {
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let turn_id = egui::Id::new("canvas-wheel-turn");
    let turn_before: WheelTurn = ui.data(|d| d.get_temp(turn_id)).unwrap_or_default();
    let mut turn = turn_before;
    let (pointer, m, space, (factor, scroll)) =
        ui.input(|i| (i.pointer.clone(), i.modifiers, i.key_down(egui::Key::Space), wheel(i, &app.session.prefs, line, rect.height(), &mut turn)));
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
    // Pressed and held still, a pen or a finger opens the context menu (egui's long touch): the
    // tool's press is dropped, as a right-click presses nothing.
    if resp.long_touched() {
        drop_press(app, ui, view);
    }
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
    // slide. Then turn the view about the pointer for a trackpad rotation ([`trackpad_turn`]).
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
        if let Some(p) = hover
            && let Some(deg) = trackpad_turn(ui, app.session.prefs.trackpad_rotate_view)
            && let Some(vm) = app.view_mut()
        {
            rotate_about(vm, rect, p, deg);
        }
    }

    // In Presentation Mode a click anywhere goes to the next artboard. The tool receives no press,
    // drag or hover, and a drag does not pan the view. A drag that began before the mode was turned
    // on ends as usual.
    if app.ui.screen_mode == 3 && matches!(drag, None | Some(Drag::Present | Drag::Gesture)) {
        let pressed = drag.is_none() && pointer.primary_pressed() && resp.hovered() && at.is_some();
        // `press` is this frame's new press, or else the press held so far. A quick tap can be
        // pressed and released in one frame.
        let press = if pressed { Some(Drag::Present) } else { drag };
        if press.is_some() && !pointer.primary_down() {
            ui.data_mut(|dd| dd.remove::<Drag>(drag_id()));
            // A tap of several fingers leaves `Drag::Gesture` (`touch_gestures`), and its release
            // does not move the view to the next artboard.
            if press == Some(Drag::Present) && resp.clicked() {
                crate::shortcuts::presentation_step(app, true);
            }
        } else if pressed {
            ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Present));
        }
        return;
    }

    let tool = app.session.tool_id();
    // Held, Space is the Hand tool for the moment, Space+Shift is the Rotate View tool and
    // Cmd+Space is the Zoom tool (with Alt, zooming out), whatever the tool.
    let zoom_mode = if space { m.command } else { tool == "zoom" };
    let rotate_mode = if space { m.shift && !m.command } else { tool == "rotateView" };
    let pan_mode = if space { !m.command && !m.shift } else { tool == "hand" };
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
        // The artboard's name sits above its corner. A press there renames on a double-click
        // and does not start a tool gesture (a fitted artboard, a new one, Artboard Options).
        if let Some(i) = artboard_name_at(app, ui, &xf, p) {
            ui.data_mut(|dd| dd.insert_temp(name_press_id(), i));
        } else {
            let d = if crate::touch::finger_navigates_only() {
                // A finger (or a palm) once a pen has been seen: it navigates, it doesn't draw.
                Drag::Gesture
            } else if zoom_mode {
                if animated_zoom(&app.session.prefs) { Drag::ZoomScrub { start: p, held: 0.0, moved: false } } else { Drag::ZoomBox { start: p } }
            } else if pan_mode {
                Drag::Pan { start: p, center: v.center, middle: false }
            } else if rotate_mode {
                let c = rect.center();
                Drag::RotateView { start_angle: (p.y - c.y).atan2(p.x - c.x) as f64, start_rot: v.rotation, snaps: !space }
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
        }
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
                Drag::RotateView { start_angle, start_rot, snaps } => {
                    let c = rect.center();
                    let a = (p.y - c.y).atan2(p.x - c.x) as f64;
                    let mut deg = start_rot + (a - start_angle).to_degrees();
                    if snaps && m.shift {
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
                Drag::Art | Drag::Gesture | Drag::Present => {}
                Drag::Tool if drag_art_out(app, ui, resp, p, view) => {
                    ui.data_mut(|dd| dd.insert_temp(drag_id(), Drag::Art));
                }
                Drag::Tool => {
                    if pointer.delta() != egui::Vec2::ZERO {
                        // Every position the pen or mouse passed this frame, not just the last: a
                        // fast stroke keeps its curve (a pen reports far more often than frames come).
                        for (at, force) in frame_moves(ui) {
                            let pressure = force.unwrap_or_else(|| pen_pressure(ui, false));
                            dispatch(app, &PointerEvent { kind: PointerKind::Drag, pos: xf.to_doc(at), mods: mods(m, space), pressure }, view);
                        }
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
                            // The marquee's center and its size on screen set the view, at any rotation.
                            vm.center = xf.to_doc(r.center());
                            let fill = (rect.width() / r.width()).min(rect.height() / r.height());
                            vm.zoom = (xf.zoom * f64::from(fill)).clamp(0.0313, 640.0);
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
                    // The second click of a double-click: don't fit another artboard on this release.
                    // A double-click on an object does not open Artboard Options. Empty artboard
                    // space still does.
                    if resp.double_clicked() && app.session.tool_id() == "artboard" {
                        app.session.set_tool_option("suppressFitClick", &json!(true));
                    }
                    let ev = PointerEvent { kind: PointerKind::Up, pos: xf.to_doc(p), mods: mods(m, space), pressure: pen_pressure(ui, false) };
                    dispatch(app, &ev, view);
                }
                Drag::Art | Drag::Gesture | Drag::Present => {}
                Drag::Pan { .. } | Drag::RotateView { .. } => {}
            }
        }
    } else if let Some(p) = hover
        && (pointer.is_moving() || mods_changed)
    {
        let ev = PointerEvent { kind: PointerKind::Move, pos: xf.to_doc(p), mods: mods(m, space), pressure: 1.0 };
        dispatch(app, &ev, view);
    }
    if pointer.primary_released()
        && let Some(i) = ui.data_mut(|d| d.remove_temp::<usize>(name_press_id()))
    {
        select_named_artboard(app, i);
    }
    if resp.double_clicked()
        && let Some(p) = at
    {
        if let Some(i) = artboard_name_at(app, ui, &xf, p) {
            begin_artboard_name_edit(ui, app, i);
        } else {
            let ev = PointerEvent { kind: PointerKind::DoubleClick, pos: xf.to_doc(p), mods: mods(m, space), pressure: 1.0 };
            let before = app.session.tool_id().to_string();
            dispatch(app, &ev, view);
            // A double-click on type switched a selection tool to the Type tool: the caret goes where
            // the type was clicked, so typing edits it at once.
            if before != "type" && app.session.tool_id() == "type" {
                app.session.set_tool_option("prevTool", &json!(before));
                for kind in [PointerKind::Down, PointerKind::Up] {
                    dispatch(app, &PointerEvent { kind, ..ev }, view);
                }
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
    /// The modifiers egui's wheel state holds, followed while Zoom with Mouse Wheel is off: each
    /// wheel event's own, or, through a trackpad gesture, all those held since it began; none
    /// while the wheel is idle.
    held: egui::Modifiers,
    /// A trackpad gesture is under way: egui has seen its Start phase and has not yet seen its End
    /// or Cancel phase.
    gesture: bool,
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

/// Turn view `vm` of the canvas `rect` by `degrees` clockwise, keeping the document point under
/// screen point `p` where it is.
fn rotate_about(vm: &mut View, rect: egui::Rect, p: Pos2, degrees: f64) {
    let before = Xf::new(rect, vm).to_doc(p);
    vm.rotation = vectorcraft_geom::normalize_deg(vm.rotation + degrees);
    vm.center += before - Xf::new(rect, vm).to_doc(p);
}

/// How far a trackpad rotation turns before it turns the view, in degrees.
const ROTATE_DEAD_ZONE: f64 = 5.0;
/// How long after its last rotation event a trackpad rotation ends, in seconds.
const ROTATE_GESTURE_END: f64 = 0.25;

/// A trackpad rotation under way ([`trackpad_turn`]).
#[derive(Clone, Copy, Debug)]
struct TrackpadTurn {
    /// Degrees it has turned so far, clockwise.
    total: f64,
    /// When its last rotation event came (egui's input time, in seconds).
    last: f64,
    /// It has reached [`ROTATE_DEAD_ZONE`] and turns the view.
    turning: bool,
}

/// General › Trackpad Gesture to Rotate View (`on`): the degrees, clockwise, that a two-finger
/// rotation on a Mac trackpad turns the view this frame. egui sends the rotation as
/// [`egui::Event::Rotate`], from winit on macOS and from Safari's gesture events in the web build.
/// A rotation does not turn the view until it reaches [`ROTATE_DEAD_ZONE`]; it then turns the view
/// by all of it, and by each later event as it comes. It ends [`ROTATE_GESTURE_END`] after its last
/// event. Rotation events that come while fingers are on a touch screen do not turn the view.
fn trackpad_turn(ui: &Ui, on: bool) -> Option<f64> {
    if !on {
        return None;
    }
    let (deg, now) = ui.input(|i| {
        let rad: f32 =
            if i.multi_touch().is_some() { 0.0 } else { i.events.iter().map(|e| if let egui::Event::Rotate(r) = e { *r } else { 0.0 }).sum() };
        (f64::from(rad).to_degrees(), i.time)
    });
    if deg == 0.0 || !deg.is_finite() {
        return None;
    }
    let id = egui::Id::new("canvas-trackpad-turn");
    let kept: Option<TrackpadTurn> = ui.data(|d| d.get_temp(id));
    let mut t = kept.filter(|t| now - t.last <= ROTATE_GESTURE_END).unwrap_or(TrackpadTurn { total: 0.0, last: now, turning: false });
    t.total += deg;
    t.last = now;
    let turn = if t.turning {
        Some(deg)
    } else if t.total.abs() >= ROTATE_DEAD_ZONE {
        t.turning = true;
        Some(t.total)
    } else {
        None
    };
    ui.data_mut(|d| d.insert_temp(id, t));
    turn
}

/// One scroll bar along its axis, in screen points: the span it scrolls over (`lo..hi`) and the
/// part of it the canvas shows (`v0..v1`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    lo: f32,
    hi: f32,
    v0: f32,
    v1: f32,
}

impl Span {
    /// The thumb on a track `len` long: where it starts and how long it is.
    fn thumb(self, len: f32) -> (f32, f32) {
        let total = (self.hi - self.lo).max(1.0);
        let size = (len * (self.v1 - self.v0) / total).clamp(MIN_THUMB.min(len), len);
        let room = total - (self.v1 - self.v0);
        let at = if room > 0.0 { (self.v0 - self.lo) / room * (len - size) } else { 0.0 };
        (at.clamp(0.0, (len - size).max(0.0)), size)
    }
    /// Screen points the view moves for each point the thumb moves on a track `len` long.
    fn ratio(self, len: f32) -> f32 {
        let room = (self.hi - self.lo) - (self.v1 - self.v0);
        let free = len - self.thumb(len).1;
        if room > 0.0 && free > 0.0 { room / free } else { 0.0 }
    }
}

const MIN_THUMB: f32 = 20.0;

/// The scroll bars' spans, horizontal then vertical: the art and the artboards (`content`) as the
/// view turns them on screen, half a window past each side, and the view itself, so the thumb is
/// always on its track. The canvas has no edge of its own (no drawing area), so the content sets it.
fn scroll_spans(xf: &Xf, content: Option<Rect>) -> [Span; 2] {
    let r = xf.rect;
    let around = content.map(|c| egui::Rect::from_points(&xf.quad(c)).expand2(r.size() / 2.0)).filter(|b| b.is_finite());
    let b = around.map_or(r, |b| b.union(r));
    [Span { lo: b.left(), hi: b.right(), v0: r.left(), v1: r.right() }, Span { lo: b.top(), hi: b.bottom(), v0: r.top(), v1: r.bottom() }]
}

/// The tracks of the horizontal and the vertical scroll bar, along the bottom and the right edge
/// of the window (`outer`), as long as the canvas (`rect`) beside them.
fn scroll_tracks(rect: egui::Rect, outer: egui::Rect) -> [egui::Rect; 2] {
    let corner = outer.max - vec2(SCROLL_BAR, SCROLL_BAR);
    [
        egui::Rect::from_min_max(pos2(rect.left(), corner.y), pos2(corner.x, outer.bottom())),
        egui::Rect::from_min_max(pos2(corner.x, rect.top()), pos2(outer.right(), corner.y)),
    ]
}

/// The window's scroll bars ([`scroll_spans`]): a thumb dragged scrolls the view along the
/// screen's axis (a turned view too), a click on the track pages towards it, and the wheel over a
/// bar scrolls along it.
fn scroll_bars(app: &mut VectorcraftApp, ui: &Ui, rect: egui::Rect, outer: egui::Rect, t: &Tokens) {
    let content = app.content_bounds();
    let Some(v) = app.view().copied() else { return };
    let xf = Xf::new(rect, &v);
    let painter = ui.painter();
    painter.rect_filled(egui::Rect::from_min_max(outer.max - vec2(SCROLL_BAR, SCROLL_BAR), outer.max), 0.0, t.ruler);
    let hover = ui.input(|i| i.pointer.hover_pos());
    // Screen points the view moves this frame, or the centre a dragged thumb puts it at.
    let mut moved = egui::Vec2::ZERO;
    let mut center = None;
    for (k, (track, span)) in scroll_tracks(rect, outer).into_iter().zip(scroll_spans(&xf, content)).enumerate() {
        let horizontal = k == 0;
        let along = |p: Pos2| if horizontal { p.x - track.left() } else { p.y - track.top() };
        let axis = if horizontal { vec2(1.0, 0.0) } else { vec2(0.0, 1.0) };
        let len = if horizontal { track.width() } else { track.height() };
        let (at, size) = span.thumb(len);
        let on_thumb = |p: Pos2| (at..=at + size).contains(&along(p));
        let id = egui::Id::new(("canvas-scroll-bar", k));
        let resp = ui.interact(track, id, Sense::click_and_drag());
        if resp.drag_started()
            && let Some(p) = ui.input(|i| i.pointer.press_origin())
            && on_thumb(p)
        {
            // The span moves with the view: the drag keeps the one it started with.
            let d = ThumbDrag { from: along(p), center: v.center, ratio: span.ratio(len), min: span.lo - span.v0, max: span.hi - span.v1 };
            ui.data_mut(|dd| dd.insert_temp(id, d));
        }
        let drag: Option<ThumbDrag> = ui.data(|dd| dd.get_temp(id));
        if let Some(d) = drag {
            if resp.dragged()
                && let Some(p) = resp.interact_pointer_pos()
            {
                let by = ((along(p) - d.from) * d.ratio).clamp(d.min, d.max);
                center = Some(d.center + xf.delta_to_doc(axis * by));
            }
            if !resp.dragged() || resp.drag_stopped() {
                ui.data_mut(|dd| dd.remove::<ThumbDrag>(id));
            }
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            // A page is most of the window, and never past the end of the span.
            let page = 0.9 * (span.v1 - span.v0);
            if along(p) < at {
                moved -= axis * page.min(span.v0 - span.lo);
            } else if along(p) > at + size {
                moved += axis * page.min(span.hi - span.v1);
            }
        }
        if resp.hovered() {
            let d = ui.input(|i| i.smooth_scroll_delta);
            moved -= axis * if horizontal { d.x + d.y } else { d.y };
        }
        painter.rect_filled(track, 0.0, t.ruler);
        let edge = if horizontal { [track.left_top(), track.right_top()] } else { [track.left_top(), track.left_bottom()] };
        painter.line_segment(edge, Stroke::new(1.0, t.border));
        let thumb = if horizontal {
            egui::Rect::from_min_size(pos2(track.left() + at, track.top()), vec2(size, track.height())).shrink2(vec2(1.0, 3.0))
        } else {
            egui::Rect::from_min_size(pos2(track.left(), track.top() + at), vec2(track.width(), size)).shrink2(vec2(3.0, 1.0))
        };
        let hot = drag.is_some() || hover.is_some_and(|p| thumb.contains(p));
        painter.rect_filled(thumb, CornerRadius::same(3), if hot { t.text_dim } else { t.text_disabled });
    }
    let Some(vm) = app.view_mut() else { return };
    if let Some(c) = center {
        vm.center = c;
    }
    if moved != egui::Vec2::ZERO {
        vm.center += xf.delta_to_doc(moved);
    }
}

/// A scroll bar thumb being dragged: where along the track it was pressed, and the view's centre,
/// the span's travel ratio ([`Span::ratio`]) and how far the view could move either way then.
#[derive(Clone, Copy, Debug)]
struct ThumbDrag {
    from: f32,
    center: Point,
    ratio: f32,
    min: f32,
    max: f32,
}

/// A marquee around screen rectangle `r`: dark dashes over a light line, so it shows on a white
/// artboard and on the grey pasteboard alike (#725).
fn marquee(p: &egui::Painter, r: egui::Rect) {
    marquee_through(p, &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]);
}

/// A [`marquee`] through the screen corners `q` in turn, back to the first: a document rectangle's
/// [`Xf::quad`] in a rotated view.
fn marquee_through(p: &egui::Painter, q: &[Pos2]) {
    let pts: Vec<Pos2> = q.iter().chain(q.first()).copied().collect();
    p.add(Shape::line(pts.clone(), Stroke::new(1.0, Color32::from_white_alpha(230))));
    for w in pts.windows(2) {
        p.extend(Shape::dashed_line(&[w[0], w[1]], Stroke::new(1.0, Color32::from_gray(40)), 3.0, 3.0));
    }
}

/// What the wheel, a pinch and a two-finger drag on a touch screen did over the canvas this frame:
/// a zoom factor (about the pointer) and a scroll (screen points the content moves). The wheel scrolls, Cmd- and Alt-wheel (Option
/// on the Mac) zoom; with General › Scroll Sideways with Cmd/Ctrl + Wheel, Cmd/Ctrl-wheel scrolls
/// sideways instead, and Alt + Cmd/Ctrl-wheel still zooms. With General › Zoom with Mouse Wheel the
/// wheel and Alt-wheel zoom, Shift-wheel scrolls up and down and Cmd/Ctrl-wheel sideways. `line`
/// and `page`: points per wheel line and page. `turn`: kept by the caller across frames. A mouse
/// wheel's notches zoom in a glide over a few frames, as scrolling does (#888); a trackpad's fine
/// steps at once.
fn wheel(i: &egui::InputState, prefs: &vectorcraft_engine::Prefs, line: f32, page: f32, turn: &mut WheelTurn) -> (f64, egui::Vec2) {
    // Fingers on a touch screen pan the content with them (a trackpad sends scrolls instead).
    let pan = i.multi_touch().map_or(egui::Vec2::ZERO, |t| t.translation_delta);
    if !prefs.zoom_with_mouse_wheel {
        // egui makes Cmd-wheel (and a pinch) its zoom and the rest a scroll it spreads over a few
        // frames: the rest of an Alt-wheel turn zooms too, however soon Alt is let go.
        if let Some(m) = i.events.iter().rev().find_map(|e| if let egui::Event::MouseWheel { modifiers, .. } = e { Some(modifiers) } else { None }) {
            turn.alt = m.alt && !m.command;
        }
        // Each wheel event changes `turn.held` and `turn.gesture` as `WheelState::on_wheel_event`
        // changes egui's wheel modifiers and status. Both are cleared while egui's wheel is idle.
        for e in &i.events {
            if let egui::Event::MouseWheel { phase, modifiers, .. } = e {
                match phase {
                    egui::TouchPhase::Start => {
                        turn.gesture = true;
                        turn.held = *modifiers;
                    }
                    egui::TouchPhase::Move if turn.gesture => turn.held |= *modifiers,
                    egui::TouchPhase::Move => turn.held = *modifiers,
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        turn.gesture = false;
                        turn.held = egui::Modifiers::NONE;
                    }
                }
            }
        }
        if !i.is_scrolling() {
            turn.gesture = false;
            turn.held = egui::Modifiers::NONE;
        }
        let (mut zoom, mut scroll) = (f64::from(i.zoom_delta()), i.smooth_scroll_delta);
        // Scroll Sideways with Cmd/Ctrl + Wheel: while egui's wheel state holds Cmd/Ctrl without
        // Alt, egui's zoom, less the pinches', is `exp(points × scroll_zoom_speed)`, at egui's
        // default speed, which the app keeps. The canvas scrolls sideways by those points instead.
        // A trackpad or web pinch still zooms. With fingers on a touch screen, egui's zoom is the
        // fingers' alone, and the canvas leaves it as it is.
        if prefs.ctrl_wheel_scrolls_sideways && turn.held.command && !turn.held.alt && i.multi_touch().is_none() {
            let pinch: f64 = i.events.iter().filter_map(|e| if let egui::Event::Zoom(f) = e { Some(f64::from(*f)) } else { None }).product();
            let wheel_zoom = zoom / pinch;
            if wheel_zoom.is_finite() && wheel_zoom > 0.0 {
                scroll.x += (wheel_zoom.ln() / f64::from(egui::InputOptions::default().scroll_zoom_speed)) as f32;
                zoom = pinch;
            }
        }
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
        // A cancelled contact (a palm the shell drops when a pen lands) keeps nothing it began.
        if phase == egui::TouchPhase::Cancel {
            drop_press(app, ui, view);
        }
        match taps.feed(id, phase, pos, now) {
            Some(crate::touch::Touch::Fingers) => {
                drop_press(app, ui, view);
                ui.data_mut(|d| d.insert_temp(drag_id(), Drag::Gesture));
            }
            Some(crate::touch::Touch::Tap(n)) => tap = crate::touch::tap_command(n),
            None => {}
        }
    }
    ui.data_mut(|d| d.insert_temp(key, taps));
    // In Presentation Mode a tap of two or three fingers undoes or redoes nothing.
    if let Some(cmd) = tap
        && app.ui.screen_mode < 3
    {
        crate::menus::invoke(app, cmd, serde_json::json!({}));
    }
}

/// Drop the tool press in progress, if any: nothing it began is kept, and the rest of the press
/// does nothing ([`Drag::Gesture`]).
fn drop_press(app: &mut VectorcraftApp, ui: &Ui, view: vectorcraft_engine::ViewInfo) {
    match ui.data(|d| d.get_temp::<Drag>(drag_id())) {
        Some(Drag::Tool) => {
            // The tool's mouse-up returns it to rest.
            let _ = app.session.cancel_interaction();
            dispatch(app, &PointerEvent { kind: PointerKind::Up, pos: Point::ZERO, mods: Mods::default(), pressure: 1.0 }, view);
        }
        Some(_) => {}
        None => return,
    }
    ui.data_mut(|d| d.insert_temp(drag_id(), Drag::Gesture));
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
    // Artboard tool's does: Paste in Place and in Front or Back then paste onto it, Align to
    // Artboard aligns to it, and All on Active Artboard selects on it.
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
    // Two cells per texture repeat.
    let per = (2.0 * cell as f64 / zoom).max(1e-9);
    textured_quad(p, tex, quad, ((ab.width() / per) as f32, (ab.height() / per) as f32));
}

/// One quad drawn with `tex`: its corners `quad` (from the top left, clockwise) show the texture
/// from (0, 0) to (`u`, `v`).
fn textured_quad(p: &egui::Painter, tex: egui::TextureId, quad: &[Pos2], (u, v): (f32, f32)) {
    let mut mesh = egui::Mesh::with_texture(tex);
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

/// The view in document space: the box round the canvas's corners (the view may be rotated).
fn view_bounds(xf: &Xf) -> Rect {
    let r = xf.rect;
    let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|c| xf.to_doc(c));
    corners.iter().fold(Rect::from_points(corners[0], corners[0]), |b, c| b.union_pt(*c))
}

/// The part of the segment from `a` to `b` inside `r`, none when the segment misses it.
fn clip_segment(a: Pos2, b: Pos2, r: egui::Rect) -> Option<(Pos2, Pos2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    // Each edge as (the run toward it, the room before it).
    for (run, room) in [(-d.x, a.x - r.left()), (d.x, r.right() - a.x), (-d.y, a.y - r.top()), (d.y, r.bottom() - a.y)] {
        if run == 0.0 {
            if room < 0.0 {
                return None;
            }
        } else if run < 0.0 {
            t0 = t0.max(room / run);
        } else {
            t1 = t1.min(room / run);
        }
    }
    (t0 <= t1).then(|| (a + d * t0, a + d * t1))
}

/// The grid (View → Show Grid): a gridline every `spacing` and `subdiv` lighter lines between
/// (left out when they would crowd). In the Dots style, a dot where gridlines cross the
/// subdivisions instead.
fn grid(p: &egui::Painter, xf: &Xf, spacing: f64, subdiv: u32, look: LineLook) {
    // The painter clips to the canvas.
    let view = view_bounds(xf);
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
/// it makes none. The corner drags the rulers' origin or resets it on a double-click.
fn ruler_input(app: &mut VectorcraftApp, ui: &Ui, full: egui::Rect, canvas: egui::Rect, xf: &Xf) -> Option<Point> {
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
    let corner = ui.interact(corner, egui::Id::new("ruler-corner"), Sense::click_and_drag());
    corner.context_menu(|ui| crate::menus::ruler_menu_body(app, ui, &mut unit_clicked));
    if let Some((id, p)) = unit_clicked {
        crate::menus::invoke(app, &id, p);
    }
    if corner.double_clicked_by(egui::PointerButton::Primary) {
        let artboard = app.view().map_or(0, |v| v.artboard);
        if let Err(e) = app.run("document.resetRulerOrigin", json!({"artboard": artboard})) {
            app.status(e.to_string());
        }
    }
    // Keep previews in the UI: only a release on the canvas edits the document, once. Cancelling
    // never adds an undo step or interferes with the drawing tool's interaction.
    let drag_id = ruler_origin_drag_id();
    let uid = app.session.active().map(|d| d.uid);
    if corner.drag_started_by(egui::PointerButton::Primary) {
        ui.data_mut(|d| d.insert_temp(drag_id, (uid, false)));
    }
    let mut preview = None;
    if let Some((started_in, mut cancelled)) = ui.data(|d| d.get_temp::<(Option<u64>, bool)>(drag_id)) {
        cancelled |= started_in != uid || ui.input(|i| !i.focused || i.key_pressed(egui::Key::Escape));
        let at = corner.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos())).filter(|p| canvas.contains(*p)).map(|p| xf.to_doc(p));
        if corner.drag_stopped_by(egui::PointerButton::Primary) {
            ui.data_mut(|d| d.remove::<(Option<u64>, bool)>(drag_id));
            if !cancelled
                && let Some(at) = at
                && let Err(e) = app.run("document.setRulerOrigin", json!({"x": at.x, "y": at.y, "artboard": app.view().map_or(0, |v| v.artboard)}))
            {
                app.status(e.to_string());
            }
        } else if !ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|d| d.remove::<(Option<u64>, bool)>(drag_id));
        } else {
            ui.data_mut(|d| d.insert_temp(drag_id, (started_in, cancelled)));
            if !cancelled {
                preview = at;
            }
        }
    }
    preview
}

/// The key of the ruler origin drag's state in egui's temporary data.
fn ruler_origin_drag_id() -> egui::Id {
    egui::Id::new("ruler-origin-drag")
}

/// Is a drag from the rulers' corner in progress? It stays true after Escape cancels the move, until
/// the button is released.
pub(crate) fn ruler_origin_dragging(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<(Option<u64>, bool)>(ruler_origin_drag_id()).is_some())
}

fn rulers(ui: &Ui, full: egui::Rect, xf: &Xf, hover: Option<Point>, unit: Unit, origin: Point, t: &Tokens) {
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
        let along = |s: Pos2| (xf.to_doc(s) - origin).dot(axis);
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
/// text/image bounds, symbol instances' bounds measured by their art from `symbols`; a clip
/// group's clipping path only, as [`walk_drawn`] visits it).
fn node_outline(n: &Node, symbols: &SymbolBoxes) -> BezPath {
    let mut bp = BezPath::new();
    walk_drawn(n, &mut |c| match &c.kind {
        NodeKind::Path { path, .. } => bp.extend(path.to_bezpath()),
        // Area type shows its frame: the type area Direct Selection reshapes; type on a path its
        // path, which the selection tools' brackets sit on.
        NodeKind::Text(t) if c.extras().perspective.is_none() && !matches!(t.kind, vectorcraft_doc::TextKind::Point) => {
            if let Some(path) = t.area_frame().or_else(|| t.type_path()) {
                bp.extend(path.to_bezpath());
            }
        }
        NodeKind::Text(_) | NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => {
            if let Some(b) = c.geometric_bounds_with(symbols) {
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
        // A gradient mesh shows its mesh lines (#1111).
        NodeKind::Mesh(m) => bp.extend(m.lines().to_bezpath()),
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

/// The most points ([`overlay_points`]) the selection's outlines and anchors, and the hover
/// highlight, are drawn for: an Image Trace of a photo selects hundreds of thousands of paths at
/// once, and selecting all of an opened PDF can select as many type objects, images and gradient
/// mesh pieces. Their outlines would make a mesh larger than the GPU takes in one buffer (#525,
/// #2460).
const OVERLAY_MAX_ANCHORS: usize = 100_000;

/// Whether `nodes` have more than [`OVERLAY_MAX_ANCHORS`] points to outline ([`overlay_points`];
/// counting stops there).
fn too_many_anchors<'a>(nodes: impl IntoIterator<Item = &'a Node>) -> bool {
    fn over(n: &Node, left: &mut usize) -> bool {
        match left.checked_sub(overlay_points(n)) {
            Some(l) => *left = l,
            None => return true,
        }
        // A clip group counts only its clipping path, the one child `walk_drawn` visits.
        if n.clips() {
            return n.children().and_then(|children| children.first()).is_some_and(|clip| over(clip, left));
        }
        !matches!(n.kind, NodeKind::Envelope { .. })
            && n.children().into_iter().flatten().skip(usize::from(n.extras().shaper.is_some())).any(|c| over(c, left))
    }
    let mut left = OVERLAY_MAX_ANCHORS;
    nodes.into_iter().any(|n| over(n, &mut left))
}

/// The points counted for `n` itself, without its children, toward [`OVERLAY_MAX_ANCHORS`]: a
/// path's anchors; the anchors of area type's frame or of type on a path's path; the four corners
/// of the box that area type or type on a path in perspective, images, symbol instances and
/// placed documents show; six for point type, its box's corners and the ends of the baseline
/// marker a selected one shows; and the points and the ends of the mesh lines of a gradient mesh
/// or an envelope.
fn overlay_points(n: &Node) -> usize {
    match &n.kind {
        NodeKind::Path { path, .. } => path.anchor_count(),
        NodeKind::Text(t) => match &t.kind {
            vectorcraft_doc::TextKind::Point => 6,
            _ if n.extras().perspective.is_some() => 4,
            vectorcraft_doc::TextKind::Area { frame } => frame.anchor_count(),
            vectorcraft_doc::TextKind::OnPath { path, .. } => path.anchor_count(),
        },
        NodeKind::Image(_) | NodeKind::SymbolInstance { .. } | NodeKind::PlacedDocument(_) => 4,
        NodeKind::Mesh(m) => m.lines().anchor_count() + m.points.len(),
        NodeKind::Envelope { .. } => {
            vectorcraft_doc::live::envelope_overlay(n).map_or(0, |(lines, grid)| lines.anchor_count() + grid.map_or(0, |g| g.points.len()))
        }
        _ => 0,
    }
}

/// [`Node::walk`] over what a selection highlight shows: an envelope's content is left out (the
/// envelope shows its mesh instead), and a clip group or clipping layer shows only its clipping
/// path, its first child, not the art the path clips (#2422).
fn walk_drawn<'a>(n: &'a Node, f: &mut impl FnMut(&'a Node)) {
    f(n);
    if matches!(n.kind, NodeKind::Envelope { .. }) {
        return;
    }
    if n.clips() {
        if let Some(clip) = n.children().and_then(|children| children.first()) {
            walk_drawn(clip, f);
        }
        return;
    }
    for c in n.children().into_iter().flatten().skip(usize::from(n.extras().shaper.is_some())) {
        walk_drawn(c, f);
    }
}

/// The topmost editable object under document point `p` at `zoom`, as the selection tools pick it
/// ([`hit_with`]).
fn hit_at(app: &VectorcraftApp, p: Point, zoom: f64) -> Option<vectorcraft_doc::hit::Hit> {
    hit_with(app, p, zoom, vectorcraft_doc::hit::hit_test)
}

/// `test` on the active document at document point `p`, with the options the selection tools hit
/// objects with at `zoom` (Selection & Anchor Display › Tolerance and Object Selection by Path Only,
/// Type › Type Object Selection by Path Only, isolation mode).
fn hit_with<T>(
    app: &VectorcraftApp,
    p: Point,
    zoom: f64,
    test: impl FnOnce(&vectorcraft_doc::Document, Point, vectorcraft_doc::hit::HitOptions) -> Option<T>,
) -> Option<T> {
    let prefs = &app.session.prefs;
    let opt = vectorcraft_doc::hit::HitOptions {
        tol: prefs.selection_tolerance / zoom,
        outline: app.ui.view.outline,
        path_only: prefs.object_selection_by_path_only,
        type_path_only: prefs.type_selection_by_path_only,
        scope: app.session.active().and_then(|st| st.isolation),
    };
    test(&app.session.active()?.doc, p, opt)
}

/// Right-click: the object under the pointer is selected first unless it already is, then the
/// context menu lists what applies to the selection ([`crate::menus::context_items`]). The locked
/// objects under the pointer ([`vectorcraft_doc::hit::locked_objects_at`]) are kept in
/// `ui.context_locked` while the menu is open, for its Unlock submenu.
fn context_menu(app: &mut VectorcraftApp, resp: &egui::Response, xf: &Xf) {
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let p = xf.to_doc(p);
        let locked = hit_with(app, p, xf.zoom, |doc, p, opt| Some(vectorcraft_doc::hit::locked_objects_at(doc, p, opt)));
        app.ui.context_locked = app.session.active().map(|st| st.uid).zip(locked);
        if let Some(st) = app.session.active()
            && let Some(top) = hit_at(app, p, xf.zoom).map(|h| h.top_object(st.isolation))
            && !st.selection.contains(top)
        {
            // A locked or hidden object can't be selected; the menu is then for the selection as is.
            let _ = app.run("select.set", json!({ "ids": [top.0] }));
        }
    }
    let mut clicked = None;
    if resp.context_menu(|ui| crate::menus::context_menu_body(app, ui, &mut clicked)).is_none() {
        app.ui.context_locked = None;
    }
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
        stroke_path(p, &node_outline(n, &st.doc.symbol_boxes()), xf, Stroke::new(1.5, color));
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

fn curve_point(p: &egui::Painter, c: Pos2, color: Color32, filled: bool, size: f32) {
    p.circle(c, size / 2.0, if filled { color } else { Color32::WHITE }, Stroke::new(1.0, color));
}

/// Type → Show Hidden Characters: spaces as dots, ¶ at paragraph ends, # at the end of a story.
/// The marks of each text object are kept from one frame to the next ([`HiddenChars`]).
fn hidden_chars_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    let clip = p.clip_rect();
    let color = Color32::from_rgb(0x4f, 0x9d, 0xff);
    let font = egui::FontId::proportional(((10.0 * xf.zoom) as f32).clamp(7.0, 18.0));
    let kept = &mut app.canvas.hidden_chars;
    // `last` holds the marks of the last frame. The marks of the objects this frame doesn't draw
    // are dropped at the end.
    let mut last = std::mem::take(&mut kept.marks);
    // The inputs are read before any layout: when fonts load during this frame, the next frame
    // lays all type out again.
    let inputs = (vectorcraft_text::FontDb::global().generation(), vectorcraft_text::hyphenation_exceptions());
    if kept.inputs != inputs {
        last.clear();
        kept.inputs = inputs;
    }
    // `walk` visits each node's `Arc` in the order `Document::walk` visits the nodes (depth first,
    // in paint order).
    fn walk(nodes: &[std::sync::Arc<Node>], f: &mut impl FnMut(&std::sync::Arc<Node>)) {
        for n in nodes {
            f(n);
            if let Some(children) = n.children() {
                walk(children, f);
            }
        }
    }
    walk(&st.doc.layers, &mut |n| {
        let NodeKind::Text(tx) = &n.kind else { return };
        // The box round the object's corners on screen, turned with the view.
        if !n.visible || n.geometric_bounds().is_none_or(|b| !egui::Rect::from_points(&xf.quad(b)).intersects(clip)) {
            return;
        }
        let marks = match last.remove(&n.id) {
            Some((was, marks)) if std::sync::Arc::ptr_eq(&was, n) => marks,
            _ => std::sync::Arc::new(HiddenMarks::of(tx)),
        };
        for &at in &marks.spaces {
            p.circle_filled(xf.to_screen(at), 1.2, color);
        }
        for &(at, glyph) in &marks.ends {
            p.text(xf.to_screen(at), egui::Align2::LEFT_BOTTOM, glyph, font.clone(), color);
        }
        kept.marks.insert(n.id, (n.clone(), marks));
    });
}

/// The Show Hidden Characters marks of the text objects drawn in the last frame
/// ([`crate::CanvasCache::hidden_chars`]). Each is kept with the object it was laid out from: an
/// edited object is a new `Arc` and is laid out again, and all type is laid out again when the
/// fonts or the hyphenation exceptions change.
#[derive(Default)]
pub struct HiddenChars {
    /// The fonts' generation ([`vectorcraft_text::FontDb::generation`]) and the hyphenation
    /// exceptions the marks were laid out with.
    inputs: (u64, String),
    /// Each text object drawn, by id: the node it was laid out from and its marks.
    marks: std::collections::HashMap<vectorcraft_doc::NodeId, (std::sync::Arc<Node>, std::sync::Arc<HiddenMarks>)>,
}

/// One text object's marks in document space: a dot over each space, and ¶ or # after the lines
/// that end a paragraph or the story. Type on a path has none.
#[derive(Default)]
struct HiddenMarks {
    spaces: Vec<Point>,
    ends: Vec<(Point, &'static str)>,
}

impl HiddenMarks {
    /// Lay out `tx` and place its marks.
    fn of(tx: &vectorcraft_doc::TextObject) -> Self {
        let mut marks = Self::default();
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), tx);
        if lay.on_path {
            return marks;
        }
        let text = tx.plain_text();
        let to = |q: Point| tx.xf * q;
        for g in &lay.glyphs {
            if text.get(g.byte..).is_some_and(|s| s.starts_with(' ')) {
                marks.spaces.push(to(Point::new(g.origin.x + g.advance / 2.0, g.origin.y - 3.0)));
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
                marks.ends.push((to(Point::new(line.x1 + 1.0, line.baseline)), glyph));
            }
        }
        marks
    }
}

/// Text threads of the selected frames: a line from each frame's out port (on its right side,
/// just above the bottom right, where the tools draw the red one) to the next frame's in port (top
/// left), like Illustrator's thread indicators.
fn thread_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    for thread in &st.doc.text_threads {
        if !thread.iter().any(|id| st.selection.objects.contains(id)) {
            continue;
        }
        let color = c32(st.doc.layer_color(thread[0]));
        let frames: Vec<vectorcraft_geom::Rect> = thread.iter().filter_map(|id| st.doc.node(*id).and_then(|n| n.geometric_bounds())).collect();
        for w in frames.windows(2) {
            let rise = (vectorcraft_tools::text::PORT_RISE_PX / xf.zoom.max(1e-9)).min(w[0].height() / 2.0);
            let (out, inp) = (xf.to_screen(Point::new(w[0].x1, w[0].y1 - rise)), xf.to_screen(Point::new(w[1].x0, w[1].y0)));
            p.line_segment([out, inp], Stroke::new(1.0, color));
            for port in [out, inp] {
                p.rect_filled(egui::Rect::from_center_size(port, egui::vec2(7.0, 7.0)), 0.0, Color32::WHITE);
                p.rect_stroke(egui::Rect::from_center_size(port, egui::vec2(7.0, 7.0)), 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
                p.line_segment([port - egui::vec2(2.0, 0.0), port + egui::vec2(2.0, 0.0)], Stroke::new(1.0, color));
            }
        }
    }
}

/// Simplify's Show Original Path: the paths its preview ran on, as they were before it, in a band
/// 3 screen points wide of their layer's color at 40%, under the selection's outlines. Nothing is drawn
/// when they have more than [`OVERLAY_MAX_ANCHORS`] anchor points.
fn original_path_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let paths = crate::dialogs::path_ops::original_paths(app);
    let Some(st) = app.session.active().filter(|_| paths.iter().map(|(_, path)| path.anchor_count()).sum::<usize>() <= OVERLAY_MAX_ANCHORS) else {
        return;
    };
    for (id, path) in paths {
        stroke_path(p, &path.to_bezpath(), xf, Stroke::new(3.0, c32(st.doc.layer_color(id)).gamma_multiply(0.4)));
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

/// A Shaper Group's mode widget (Shaper and Selection tools), drawn with selection edges shown or
/// hidden: an arrow pointing right in Face Selection mode and left in Construction mode.
fn shaper_widget_overlay(app: &VectorcraftApp, p: &egui::Painter, xf: &Xf) {
    let Some(st) = app.session.active() else { return };
    if matches!(app.session.tool_id(), "shaper" | "selection")
        && let Some(w) = vectorcraft_tools::shaperwidget::ShaperWidget::of(&st.doc, &st.selection, st.isolation, xf.zoom)
    {
        let (c, r) = (xf.to_screen(w.at), vectorcraft_tools::shaperwidget::RADIUS_PX);
        let color = c32(st.doc.layer_color(w.group));
        p.circle_filled(c, r, Color32::WHITE);
        p.circle_stroke(c, r, Stroke::new(1.0, color));
        let d = if w.construction { -1.0 } else { 1.0 };
        let arrow = vec![c + vec2(3.0 * d, 0.0), c + vec2(-2.0 * d, -3.0), c + vec2(-2.0 * d, 3.0)];
        p.add(Shape::convex_polygon(arrow, color, Stroke::NONE));
    }
}

fn selection_overlay(app: &mut VectorcraftApp, p: &egui::Painter, xf: &Xf, curve_points_from_tool: bool) {
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
    let symbols = st.doc.symbol_boxes();
    let tool = app.session.tool_id();
    let direct = vectorcraft_tools::catalog::edits_anchors(tool);
    let curvature = tool == "curvature";
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
            n.geometric_bounds_with(&symbols).map(|b| vectorcraft_geom::shapes::rectangle(b).to_bezpath())
        } else {
            Some(node_outline(n, &symbols))
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
        stroke_path(p, &node_outline(n, &symbols), xf, Stroke::new(1.0, color));
        // Anchors (and handles for selected anchors in direct mode); a gradient mesh's or a mesh
        // envelope's points.
        walk_drawn(n, &mut |c| {
            let envelope = vectorcraft_doc::live::envelope_overlay(c).and_then(|(_, grid)| grid);
            let grid = match &c.kind {
                NodeKind::Mesh(m) => Some(m),
                _ => envelope.as_ref(),
            };
            if let Some(grid) = grid {
                for q in &grid.points {
                    anchor_square(p, xf.to_screen(q.p), color, false, anchor(direct));
                }
                return;
            }
            let NodeKind::Path { path, .. } = &c.kind else { return };
            // The Curvature tool owns its points while drawing; existing paths use this overlay.
            if curvature && curve_points_from_tool {
                return;
            }
            let selected_segments = st.selection.segments.get(&c.id);
            if let Some(segments) = selected_segments {
                for &(si, seg) in segments {
                    let Some(sp) = path.subpaths.get(si).filter(|sp| seg < sp.segment_count()) else { continue };
                    let curve = sp.segment(seg);
                    let mut outline = vectorcraft_geom::BezPath::new();
                    outline.move_to(curve.p0);
                    if sp.segment_is_line(seg) {
                        outline.line_to(curve.p3);
                    } else {
                        outline.curve_to(curve.p1, curve.p2, curve.p3);
                    }
                    stroke_path(p, &outline, xf, Stroke::new(2.0, color));
                }
            }
            for (si, ai, a) in path.anchors() {
                let sel = match partial {
                    Some(set) => set.contains(&(si, ai)),
                    None => !direct || c.id == *id,
                };
                let sp = xf.to_screen(a.p);
                if !curvature && sel && (direct || partial.is_some()) {
                    with_handles += 1;
                    let sides = path.subpaths.get(si).map(|subpath| st.selection.handle_sides(c.id, si, ai, subpath)).unwrap_or((true, true));
                    handles.extend(
                        [(a.h_in, sides.0), (a.h_out, sides.1)]
                            .into_iter()
                            .filter(|(h, show)| *show && h.distance(a.p) > 1e-6)
                            .map(|(h, _)| (sp, xf.to_screen(h), color)),
                    );
                }
                let anchor_selected = sel && selected_segments.is_none();
                let ink = if anchor_selected && partial.is_some() { selected_anchor(color) } else { color };
                if curvature {
                    curve_point(p, sp, ink, anchor_selected && partial.is_some(), grow + 6.0);
                } else {
                    anchor_square(p, sp, ink, anchor_selected, anchor(partial.is_some() || direct));
                }
            }
        });
        // Centre point (Attributes panel → Show Center).
        if n.shows_center()
            && let Some(b) = n.geometric_bounds_with(&symbols)
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
        for (id, q) in w.visible_on() {
            let (sp, color) = (xf.to_screen(q), c32(st.doc.layer_color(id)));
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
    // Pie widgets (Selection tool, with View › Show Bounding Box, on live ellipses): each joined
    // by a line to the outline's point at its angle. They stay while one is dragged.
    if tool == "selection" && app.ui.view.bounding_box {
        for w in vectorcraft_tools::pie::PieWidget::of(&st.doc, &st.selection, xf.zoom) {
            let (on, c, r, color) = (xf.to_screen(w.on), xf.to_screen(w.at), vectorcraft_tools::pie::RADIUS_PX, c32(st.doc.layer_color(w.id)));
            p.line_segment([on, c], Stroke::new(1.0, color));
            p.circle_filled(c, r, Color32::WHITE);
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

/// The width an artboard needs on screen for its name to show (px): narrower, the name is left
/// out rather than spilling over its neighbours.
const MIN_LABELLED_ARTBOARD: f32 = 24.0;

fn name_press_id() -> egui::Id {
    egui::Id::new("canvas-artboard-name-press")
}

fn name_edit_id() -> egui::Id {
    egui::Id::new("canvas-artboard-name-edit")
}

/// A name being typed above an artboard. `fresh`: this frame opened it, so a focus loss from the
/// press that started it is not the user leaving the field.
#[derive(Clone)]
struct ArtboardNameEdit {
    index: usize,
    text: String,
    fresh: bool,
}

/// The canvas label for `name`, laid out as [`artboard_label`] draws it.
fn canvas_name_galley(painter: &egui::Painter, corner: Pos2, width: f32, name: &str, color: Color32) -> Option<(std::sync::Arc<egui::Galley>, Pos2)> {
    if !width.is_finite() || width < MIN_LABELLED_ARTBOARD {
        return None;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), egui::FontId::proportional(11.0), color);
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let at = corner - vec2(0.0, 4.0 + galley.size().y);
    Some((galley, at))
}

/// Artboard `name` above its top-left `corner` on screen, no wider than the artboard (`width` on
/// screen): a longer name ends in "…", and an artboard too narrow shows none (#949).
fn artboard_label(painter: &egui::Painter, corner: Pos2, width: f32, name: &str, color: Color32) {
    if let Some((galley, at)) = canvas_name_galley(painter, corner, width, name, color) {
        painter.galley(at, galley, color);
    }
}

fn canvas_name_rect(painter: &egui::Painter, corner: Pos2, width: f32, name: &str) -> Option<egui::Rect> {
    canvas_name_galley(painter, corner, width, name, Color32::WHITE).map(|(galley, at)| egui::Rect::from_min_size(at, galley.size()).expand(2.0))
}

/// Where the Artboard tool draws "01 - name" for its active artboard (see its overlay label).
fn tool_name_rect(painter: &egui::Painter, xf: &Xf, rect: Rect, index: usize, name: &str) -> egui::Rect {
    let z = xf.zoom.max(1e-9);
    let anchor = Point::new(rect.x0 - 8.0 / z, rect.y0 - 4.0 / z);
    let sp = xf.to_screen(anchor) + vec2(8.0, -14.0);
    let galley = painter.layout(format!("{:02} - {name}", index + 1), egui::FontId::proportional(11.0), Color32::WHITE, f32::INFINITY);
    egui::Rect::from_min_size(sp, galley.size()).expand(2.0)
}

/// The artboard whose on-canvas name contains `screen`, if that name is being drawn.
fn artboard_name_at(app: &VectorcraftApp, ui: &Ui, xf: &Xf, screen: Pos2) -> Option<usize> {
    let st = app.session.active()?;
    let artboard_tool = app.session.tool_id() == "artboard";
    if !artboard_tool && st.doc.artboards.len() <= 1 {
        return None;
    }
    let tool_labelled = artboard_tool.then(|| app.session.tool_options()["active"].as_u64()).flatten();
    let painter = ui.painter();
    let mut hit = None;
    for (i, ab) in st.doc.artboards.iter().enumerate() {
        if tool_labelled == u64::try_from(i).ok() {
            continue;
        }
        let corner = xf.to_screen(Point::new(ab.rect.x0, ab.rect.y0));
        let width = (ab.rect.width() * xf.zoom) as f32;
        let text = format!("{:02} - {}", i + 1, ab.name);
        if canvas_name_rect(painter, corner, width, &text).is_some_and(|r| r.contains(screen)) {
            hit = Some(i);
        }
    }
    if let Some(i) = tool_labelled.and_then(|i| usize::try_from(i).ok())
        && let Some(ab) = st.doc.artboards.get(i)
        && tool_name_rect(painter, xf, ab.rect, i, &ab.name).contains(screen)
    {
        return Some(i);
    }
    hit
}

fn select_named_artboard(app: &mut VectorcraftApp, index: usize) {
    let n = app.session.active().map(|st| st.doc.artboards.len()).unwrap_or(0);
    if index >= n {
        return;
    }
    if let Some(v) = app.view_mut() {
        v.artboard = index;
    }
    if app.session.tool_id() == "artboard" {
        app.session.set_tool_option("active", &json!(index));
    }
}

fn begin_artboard_name_edit(ui: &Ui, app: &VectorcraftApp, index: usize) {
    let Some(name) = app.session.active().and_then(|st| st.doc.artboards.get(index).map(|a| a.name.clone())) else { return };
    ui.data_mut(|d| d.insert_temp(name_edit_id(), ArtboardNameEdit { index, text: name, fresh: true }));
}

fn commit_artboard_name(app: &mut VectorcraftApp, index: usize, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let current = app.session.active().and_then(|st| st.doc.artboards.get(index).map(|a| a.name.clone()));
    if current.as_deref() == Some(trimmed) {
        return;
    }
    // A refusal is reported by the command; the label just closes.
    app.run("artboard.setProps", json!({ "index": index, "name": trimmed })).ok();
}

/// The text field over an artboard name a double-click opened.
fn artboard_name_editor(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    let Some(mut edit) = ui.data(|d| d.get_temp::<ArtboardNameEdit>(name_edit_id())) else { return };
    let alive = app.session.active().is_some_and(|st| edit.index < st.doc.artboards.len());
    if !alive {
        ui.data_mut(|d| d.remove::<ArtboardNameEdit>(name_edit_id()));
        return;
    }
    let Some(rect) = artboard_name_field_rect(app, ui, xf, edit.index) else {
        ui.data_mut(|d| d.remove::<ArtboardNameEdit>(name_edit_id()));
        return;
    };
    let fresh = edit.fresh;
    edit.fresh = false;
    let te = ui.put(rect, egui::TextEdit::singleline(&mut edit.text).id(name_edit_id().with("field")).font(egui::FontId::proportional(11.0)));
    if fresh || (!te.has_focus() && !te.lost_focus()) {
        te.request_focus();
    }
    let done = !fresh && te.lost_focus();
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let (index, text) = (edit.index, edit.text.clone());
    if done {
        ui.data_mut(|d| d.remove::<ArtboardNameEdit>(name_edit_id()));
        if !escape {
            commit_artboard_name(app, index, &text);
        }
    } else {
        ui.data_mut(|d| d.insert_temp(name_edit_id(), edit));
    }
}

/// Where the rename field sits: the name label's rectangle, at least wide enough to type in.
fn artboard_name_field_rect(app: &VectorcraftApp, ui: &Ui, xf: &Xf, index: usize) -> Option<egui::Rect> {
    let st = app.session.active()?;
    let ab = st.doc.artboards.get(index)?;
    let artboard_tool = app.session.tool_id() == "artboard";
    let tool_labelled = artboard_tool.then(|| app.session.tool_options()["active"].as_u64()).flatten();
    let painter = ui.painter();
    let rect = if tool_labelled == u64::try_from(index).ok() {
        tool_name_rect(painter, xf, ab.rect, index, &ab.name)
    } else {
        let corner = xf.to_screen(Point::new(ab.rect.x0, ab.rect.y0));
        let width = (ab.rect.width() * xf.zoom) as f32;
        canvas_name_rect(painter, corner, width, &format!("{:02} - {}", index + 1, ab.name))?
    };
    Some(egui::Rect::from_min_size(rect.min, vec2(rect.width().max(80.0), rect.height().max(16.0))))
}

/// Is overlay label `text` the Artboard tool's "01 - <artboard name>"? It holds a name, so it is
/// shown as it is; the tools' other labels ("anchor", "path", Puppet Warp's warning) are ours.
fn names_an_artboard(text: &str) -> bool {
    text.split_once(" - ").is_some_and(|(n, _)| n.len() >= 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

fn draw_overlays(p: &egui::Painter, xf: &Xf, overlays: &[Overlay], t: &Tokens, look: HandleLook) {
    for o in overlays {
        match o {
            // Where the tools select: the document rectangle, turned with the view.
            Overlay::Marquee(r) => marquee_through(p, &xf.quad(*r)),
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
            Overlay::CurvePoint { p: pt, color, filled, size } => curve_point(p, xf.to_screen(*pt), c32(*color), *filled, *size + look.grow),
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

/// Whether the objects `ids` (or anything inside them) are painted with a colour: a solid colour,
/// a gradient's stops or a mesh's points, which Recolor Artwork can change.
pub(crate) fn has_colors(doc: &vectorcraft_doc::Document, ids: &[vectorcraft_doc::NodeId]) -> bool {
    ids.iter().filter_map(|id| doc.node(*id)).any(|n| {
        let mut found = false;
        vectorcraft_doc::swatches::node_colors(n, &mut |_, _| found = true);
        found
    })
}

/// The bounds of the artboards the Artboard tool has selected ([`crate::panels::artboards::picked`]).
fn picked_artboards_bounds(app: &VectorcraftApp) -> Option<Rect> {
    let st = app.session.active()?;
    let picked = crate::panels::artboards::picked(app, st.doc.artboards.len());
    picked.iter().filter_map(|i| st.doc.artboards.get(*i)).map(|a| a.rect).reduce(|a, b| a.union(b))
}

/// The Contextual Task Bar: a floating pill under the selection with the most likely next actions.
/// With the Artboard tool it sits under the selected artboards and offers Resize (Artboard
/// Options), Duplicate, Rearrange All (with two or more artboards) and Export (Export for Screens).
/// Its handle drags it anywhere on the canvas; unpinned it keeps that offset as it follows the
/// selection, pinned it stays put ([`crate::state::TaskBarPlace`]).
fn task_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    let t = Tokens::get(ui.ctx());
    let on_artboards = app.session.tool_id() == "artboard";
    if !on_artboards
        && (app.session.active().is_none_or(|st| st.selection.is_empty())
            || !matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection"))
    {
        return;
    }
    let Some(b) = (if on_artboards { picked_artboards_bounds(app) } else { app.selection_bounds() }) else { return };
    let Some(st) = app.session.active() else { return };
    let n = st.selection.len();
    let first = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned();
    let is_group = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Group { .. }));
    let is_text = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Text(_)));
    let is_image = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Image(_)));
    // An Image Trace object: the group the engine names "Image Trace", its source image first.
    let is_trace = is_group
        && first.as_ref().is_some_and(|f| {
            f.name.as_deref() == Some("Image Trace") && f.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
        });
    let mut items: Vec<(&str, &str, &str)> = vec![]; // (label, icon, command)
    if on_artboards {
        // As in the reference app, the Artboard tool offers to resize, duplicate and export the
        // artboards (<https://community.adobe.com/questions-652/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>),
        // and to rearrange them (<https://creativepro.com/managing-artboards-in-illustrator/>).
        items.push((tl!("Resize"), "move-diagonal-2", TASK_BAR_ARTBOARD_OPTIONS));
        items.push((tl!("Duplicate"), "copy", TASK_BAR_DUPLICATE_ARTBOARDS));
        if st.doc.artboards.len() > 1 {
            items.push((tl!("Rearrange All"), "dc-rearrange", TASK_BAR_REARRANGE_ARTBOARDS));
        }
        items.push((tl!("Export"), "share-2", "file.exportForScreens"));
    } else {
        // As in the reference app, an image offers Image Trace and Crop Image, and traced art offers
        // Expand (<https://community.adobe.com/announcements-651/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>).
        if n == 1 && is_image {
            items.push((tl!("Image Trace"), "image", TASK_BAR_IMAGE_TRACE));
            items.push((tl!("Crop Image"), "dc-pf-crop", "ui.cropImage"));
        }
        // Direct-selected anchors take the place of a path's Offset Path and Simplify. For them the bar
        // offers Remove, Connect, Cut and Convert to corner or smooth, as the reference app does
        // (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/contextual-task-bar-overview.html>).
        // These run the same commands as the Control bar's anchor buttons.
        let anchors = !st.selection.anchors.is_empty();
        if anchors {
            items.push((tl!("Remove Anchor Points"), "pen-tool-delete", "path.removeAnchors"));
            items.push((tl!("Connect"), "dc-join", "path.join"));
            items.push((tl!("Cut Path"), "scissors", "path.cutAtAnchors"));
            items.push((tl!("Corner"), "dc-anchor", TASK_BAR_TO_CORNER));
            items.push((tl!("Smooth"), "dc-anchor-smooth", TASK_BAR_TO_SMOOTH));
        }
        if n > 1 {
            items.push((tl!("Group"), "group", "object.group"));
            items.push((tl!("Unite"), "squares-unite", "object.pathfinder.unite"));
        } else if is_trace {
            items.push((tl!("Expand"), "maximize-2", "imageTrace.expand"));
        } else if is_group {
            items.push((tl!("Ungroup"), "ungroup", "object.ungroup"));
            items.push((tl!("Isolate"), "square-dashed", "object.isolate"));
        } else if is_text {
            items.push((tl!("Create Outlines"), "type", "type.createOutlines"));
        } else if is_image {
            // Offset Path and Simplify don't apply to an image.
        } else if !anchors {
            items.push((tl!("Offset Path"), "square-dashed", "object.path.offsetPath"));
            items.push((tl!("Simplify"), "spline", "object.path.simplify"));
        }
        items.push((tl!("Duplicate"), "copy", "edit.duplicate"));
    }
    let fill = first.as_ref().map(|f| f.appearance.fill_paint()).unwrap_or_default();
    let colored = has_colors(&st.doc, &st.selection.objects);
    let doc = st.uid;
    // Under the selection's lowest corner on screen, centered on the selection (the view may be
    // rotated).
    let lowest = xf.quad(b).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
    let anchor = pos2(xf.to_screen(b.center()).x, lowest);
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
                    // The selected objects' Fill, Recolor Artwork and Lock, left out with the Artboard tool.
                    if !on_artboards {
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
                        // Recolor Artwork, when the selection has colours to recolour.
                        if colored && widgets::icon_button(ui, "palette", tl!("Recolor Artwork"), false, 30.0).clicked() {
                            run = Some(("ui.recolorDialog", json!({})));
                        }
                        if widgets::icon_button(ui, "lock", tl!("Lock (⌘2)"), false, 30.0).clicked() {
                            run = Some(("object.lock", json!({})));
                        }
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
    match run {
        // Image Trace traces the image at once with the Default preset. As a quick trace from
        // Properties or the Control bar, it leaves the Image Trace panel as it was.
        Some((TASK_BAR_IMAGE_TRACE, _)) => crate::menus::invoke(app, "imageTrace.make", json!({"preset": "Default"})),
        // The Artboard tool's buttons: Artboard Options for the active artboard, as Properties'
        // icon opens it; Duplicate for every selected artboard; Rearrange All Artboards.
        Some((TASK_BAR_ARTBOARD_OPTIONS, _)) => crate::menus::invoke(app, "tool.options", json!({"tool": "artboard"})),
        Some((TASK_BAR_DUPLICATE_ARTBOARDS, _)) => crate::panels::artboards::duplicate_picked(app),
        Some((TASK_BAR_REARRANGE_ARTBOARDS, _)) => crate::panels::artboards::open_rearrange(app),
        // Corner and Smooth convert the direct-selected anchors, as the Control bar's buttons do.
        Some((TASK_BAR_TO_CORNER, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "corner"})),
        Some((TASK_BAR_TO_SMOOTH, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "smooth"})),
        Some((c, p)) => crate::menus::invoke(app, c, p),
        None => {}
    }
}

/// The Contextual Task Bar's Image Trace button: a trace with the Default preset.
const TASK_BAR_IMAGE_TRACE: &str = "taskBar.imageTrace";
/// The Contextual Task Bar's Resize button with the Artboard tool: Artboard Options.
const TASK_BAR_ARTBOARD_OPTIONS: &str = "taskBar.artboardOptions";
/// The Contextual Task Bar's Duplicate button with the Artboard tool.
const TASK_BAR_DUPLICATE_ARTBOARDS: &str = "taskBar.duplicateArtboards";
/// The Contextual Task Bar's Rearrange All button with the Artboard tool.
const TASK_BAR_REARRANGE_ARTBOARDS: &str = "taskBar.rearrangeArtboards";
/// The Contextual Task Bar's Corner button for direct-selected anchors.
const TASK_BAR_TO_CORNER: &str = "taskBar.toCorner";
/// The Contextual Task Bar's Smooth button for direct-selected anchors.
const TASK_BAR_TO_SMOOTH: &str = "taskBar.toSmooth";

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_geom::Shape as _;

    /// With VECTORCRAFT_SYSTEM_CURSORS set the glyphs aren't drawn and the system's cursors stand in: those of
    /// the Pen and the other drawing tools are the system crosshair already, so Use Precise Cursors makes no
    /// difference to them there.
    #[test]
    fn system_cursors_show_the_same_crosshair_with_precise_cursors_on_or_off() {
        for c in [
            Cursor::Pen,
            Cursor::PenDrawing,
            Cursor::PenAdd,
            Cursor::PenDelete,
            Cursor::PenClose,
            Cursor::PenContinue,
            Cursor::PenJoin,
            Cursor::PenConvert,
            Cursor::Eyedropper,
            Cursor::Slice,
            Cursor::Blend,
            Cursor::BlendObject,
            Cursor::BlendAnchor,
            Cursor::ShapeBuilder,
            Cursor::ShapeBuilderErase,
        ] {
            assert_eq!(cursor_icon(c), egui::CursorIcon::Crosshair, "{c:?}");
            assert_eq!(cursor_icon(c), cursor_icon(c.precise()), "{c:?}");
        }
    }

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
            let mut out = ctx.run_ui(Default::default(), |ui| rulers(ui, full, &xf, None, Unit::Points, Point::ZERO, &Tokens::get(ui.ctx())));
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
    fn ruler_origin_zero_ticks_follow_the_origin_at_any_rotation_and_unit() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let full = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let [top, left, _] = ruler_rects(full);
        let origin = Point::new(350.25, 250.5);
        for deg in [0.0_f64, 30.0, 90.0, 200.0] {
            for unit in [Unit::Points, Unit::Millimeters] {
                let xf = Xf { rect: full, zoom: 1.7, center: Point::new(300.0, 200.0), rot: deg.to_radians() };
                let mut out = ctx.run_ui(Default::default(), |ui| rulers(ui, full, &xf, None, unit, origin, &Tokens::get(ui.ctx())));
                out.textures_delta.clear();
                let at = xf.to_screen(origin);
                for (r, horizontal) in [(top, true), (left, false)] {
                    let zeros: Vec<_> = out
                        .shapes
                        .iter()
                        .filter(|c| c.clip_rect == r)
                        .filter_map(|c| match &c.shape {
                            Shape::Text(t) if t.galley.text() == "0" => Some(t.pos),
                            _ => None,
                        })
                        .collect();
                    // Left-ruler labels are separate digits: a zero in "100" isn't the zero label.
                    assert!(
                        zeros.iter().any(|zero| {
                            let delta = if horizontal { zero.x - 2.0 - at.x } else { zero.y - 2.0 - at.y };
                            delta.abs() < 0.02
                        }),
                        "{deg}°, {unit:?}: zeros at {zeros:?}, origin {at:?}"
                    );
                }
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
        assert!(g.extras().shaper.is_some());
        let symbols = st.doc.symbol_boxes();
        let bounds = node_outline(g, &symbols).bounding_box();
        assert!((bounds.x0 - 100.0).abs() < 0.01 && (bounds.x1 - 200.0).abs() < 0.01, "{bounds:?}");
        assert_eq!(node_outline(st.doc.node(line).unwrap(), &symbols).bounding_box().x0, 50.0, "an isolated original still shows its full outline");
    }

    /// A clip group made from scratch: a 320 × 40 image and a 300 × 200 ellipse, both reaching
    /// outside a 100 × 100 square at (100, 100) that clips them, and a rectangle beside them. The
    /// task bar and rulers are off. → the app, the clip group, the ellipse and the rectangle.
    fn clip_group_app() -> (VectorcraftApp, serde_json::Value, serde_json::Value, serde_json::Value) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        let img = image::RgbaImage::from_pixel(320, 40, image::Rgba([200, 120, 40, 255]));
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let data = vectorcraft_format::base64_encode(&png);
        app.run("file.place", json!({"name": "texture.png", "dataBase64": data, "link": false, "at": [150, 150]})).unwrap();
        let image = json!(app.session.active().unwrap().selection.objects[0].0);
        let ellipse = app.run("shape.ellipse", json!({"x": 0, "y": 50, "width": 300, "height": 200})).unwrap()["id"].clone();
        let square = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [image, ellipse, square]})).unwrap();
        let group = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
        let beside = app.run("shape.rectangle", json!({"x": 330, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
        (app, group, ellipse, beside)
    }

    /// The screen points of what one frame draws in `color` with lines `width` px wide: outlines and
    /// the bounding box, and with `width` 1 the anchors and the bounding box's handles too.
    fn drawn_in(app: &mut VectorcraftApp, ctx: &egui::Context, color: Color32, width: f32) -> Vec<Pos2> {
        let solid = egui::epaint::ColorMode::Solid(color);
        let mut at = vec![];
        for s in shapes(app, ctx) {
            match s {
                Shape::Path(ps) if ps.stroke.color == solid && ps.stroke.width == width => at.extend(ps.points),
                Shape::Rect(r) if width == 1.0 && (r.fill == color || (r.stroke.color == color && r.stroke.width == width)) => {
                    at.extend([r.rect.min, r.rect.max]);
                }
                _ => {}
            }
        }
        at
    }

    /// #2422: a selected clip group shows its clipping path's outline and anchors, the bounding
    /// box of what shows, and not the art the path clips; so do its hover highlight and its key
    /// object outline.
    #[test]
    fn a_selected_clip_group_shows_its_clipping_path_not_its_hidden_art() {
        let (mut app, group, _, beside) = clip_group_app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(group.as_u64().unwrap())));
        // The square on screen. `inside` allows 8 px around it for the anchors and the bounding box's handles.
        let clip = egui::Rect::from_two_pos(xf.to_screen(Point::new(100.0, 100.0)), xf.to_screen(Point::new(200.0, 200.0)));
        // Each case that draws nothing, or draws outside the square.
        let mut wrong = vec![];
        let mut inside = |at: Vec<Pos2>, what: &str| {
            if at.is_empty() || !at.iter().all(|q| clip.expand(8.0).contains(*q)) {
                wrong.push(format!("{what}: {at:?}"));
            }
        };
        for tool in ["selection", "directSelection", "groupSelection"] {
            app.select_tool(tool);
            app.run("select.set", json!({"ids": [group]})).unwrap();
            inside(drawn_in(&mut app, &ctx, color, 1.0), tool);
        }
        app.select_tool("selection");
        app.run("select.set", json!({"ids": [group, beside]})).unwrap();
        app.run("select.key", json!({"id": group})).unwrap();
        inside(drawn_in(&mut app, &ctx, color, KEY_OUTLINE), "the key object");
        app.run("select.none", json!({})).unwrap();
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(150.0, 150.0)))]);
        inside(drawn_in(&mut app, &ctx, color, 1.5), "the hover highlight");
        assert!(wrong.is_empty(), "drawn outside {clip:?}:\n{}", wrong.join("\n"));
    }

    /// #2422: a clip group whose hidden art has more anchors than the overlay draws (#525) still
    /// shows its clipping path's anchors and hover highlight.
    #[test]
    fn a_clip_group_over_a_huge_path_shows_its_clipping_path() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        let n = OVERLAY_MAX_ANCHORS + 10;
        let points = (0..n).map(|i| format!("L{} {}", 20.0 + 360.0 * i as f64 / n as f64, 100 + (i % 2) * 50)).collect::<String>();
        let huge = app.run("path.create", json!({"d": format!("M20 20 {points} Z")})).unwrap()["id"].clone();
        let square = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [huge, square]})).unwrap();
        let group = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
        let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(group.as_u64().unwrap())));
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        app.select_tool("selection");
        app.run("select.set", json!({"ids": [group]})).unwrap();
        let anchors = shapes(&mut app, &ctx).iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == color)).count();
        app.run("select.none", json!({})).unwrap();
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(150.0, 150.0)))]);
        let hovered = !drawn_in(&mut app, &ctx, color, 1.5).is_empty();
        assert_eq!((anchors, hovered), (4, true), "the square's four anchors, and the hover highlight");
    }

    /// #2422: the art inside a clip group still shows its whole outline when it is picked on its
    /// own: by Edit Contents, by a Direct Selection or Group Selection click, and in isolation mode;
    /// so does its Group Selection hover highlight.
    #[test]
    fn the_art_a_clip_group_clips_shows_whole_when_picked() {
        let (mut app, group, ellipse, _) = clip_group_app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let color = c32(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(group.as_u64().unwrap())));
        // Something is drawn at the ellipse's left and right ends, outside the square.
        let reaches = |at: &[Pos2], what: &str| {
            let (left, right) = (xf.to_screen(Point::new(0.0, 150.0)).x, xf.to_screen(Point::new(300.0, 150.0)).x);
            assert!(at.iter().any(|q| (q.x - left).abs() < 2.0) && at.iter().any(|q| (q.x - right).abs() < 2.0), "{what}: {at:?}");
        };
        app.select_tool("selection");
        app.run("select.set", json!({"ids": [group]})).unwrap();
        app.run("object.clippingMask.editContents", json!({})).unwrap();
        reaches(&drawn_in(&mut app, &ctx, color, 1.0), "Edit Contents");
        app.run("select.none", json!({})).unwrap();
        app.select_tool("directSelection");
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 150.0, 120.0), view).unwrap();
        }
        assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(ellipse.as_u64().unwrap())]);
        reaches(&drawn_in(&mut app, &ctx, color, 1.0), "a Direct Selection click");
        app.run("select.none", json!({})).unwrap();
        app.select_tool("groupSelection");
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(xf.to_screen(Point::new(150.0, 120.0)))]);
        reaches(&drawn_in(&mut app, &ctx, color, 1.5), "the Group Selection hover highlight");
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 150.0, 120.0), view).unwrap();
        }
        assert_eq!(app.session.active().unwrap().selection.objects, vec![vectorcraft_doc::NodeId(ellipse.as_u64().unwrap())]);
        reaches(&drawn_in(&mut app, &ctx, color, 1.0), "a Group Selection click");
        app.select_tool("selection");
        app.run("object.isolate", json!({"id": group})).unwrap();
        app.run("select.all", json!({})).unwrap();
        reaches(&drawn_in(&mut app, &ctx, color, 1.0), "isolation mode");
    }

    /// A Shaper Group's mode widget, drawn for the Shaper and Selection tools with selection edges
    /// shown or hidden: its arrow points right in Face Selection mode and left in Construction mode.
    #[test]
    fn the_shaper_widget_points_to_the_other_mode() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        for x in [0, 50] {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
        }
        let points = json!({"points": [[25, 20], [75, 35], [30, 50], [125, 65]]});
        let g = app.session.execute("shaper.scribble", &points).unwrap()["id"].as_u64().unwrap();
        let ctx = egui::Context::default();
        // For each widget drawn, whether its arrow's tip is right of the circle's center.
        let right = |app: &mut VectorcraftApp| -> Vec<bool> {
            let drawn = shapes(app, &ctx);
            let r = vectorcraft_tools::shaperwidget::RADIUS_PX;
            let centers: Vec<egui::Pos2> = drawn
                .iter()
                .filter_map(|s| if let Shape::Circle(c) = s { (c.radius == r && c.fill == Color32::WHITE).then_some(c.center) } else { None })
                .collect();
            drawn
                .iter()
                .filter_map(|s| match s {
                    Shape::Path(p) if p.points.len() == 3 => centers.iter().find(|c| c.distance(p.points[0]) <= r).map(|c| p.points[0].x > c.x),
                    _ => None,
                })
                .collect()
        };
        app.select_tool("shaper");
        assert_eq!(right(&mut app), vec![true], "the scribble selects the group");
        app.session.execute("object.isolate", &json!({"id": g})).unwrap();
        assert_eq!(right(&mut app), vec![false]);
        app.select_tool("selection");
        assert_eq!(right(&mut app), vec![false]);
        // View › Hide Edges keeps the widget, which still answers clicks.
        app.ui.view.edges = false;
        assert_eq!(right(&mut app), vec![false]);
        app.select_tool("pen");
        assert!(right(&mut app).is_empty());
    }

    #[test]
    fn a_symbol_instance_is_outlined_around_its_art() {
        // A symbol holding the square of the SVG import tests' `Star` symbol, 20 × 20 from (0, 0),
        // and an instance of it at (20, 30).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
        let mut s = Session::new();
        s.execute("document.open", &json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
        let st = s.active().unwrap();
        let mut inst = None;
        st.doc.walk(|n| {
            if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
                inst = Some(n);
            }
        });
        assert_eq!(node_outline(inst.unwrap(), &st.doc.symbol_boxes()).bounding_box(), Rect::new(20.0, 30.0, 40.0, 50.0));
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

    /// A Shift-click with the Selection tool on an empty spot inside an artboard makes it the active
    /// one and keeps the selection, and Align to Artboard then aligns the objects to it unless the
    /// call names another artboard.
    #[test]
    fn a_shift_click_activates_its_artboard_and_align_to_artboard_aligns_to_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        let mut rect = |x: f64, y: f64| {
            let r = app.session.execute("shape.rectangle", &json!({"x": boards[0].x0 + x, "y": boards[0].y0 + y, "width": 20, "height": 20}));
            vectorcraft_doc::NodeId(r.unwrap()["id"].as_u64().unwrap())
        };
        let ids = [rect(10.0, 15.0), rect(50.0, 40.0)];
        app.session.execute("select.set", &json!({"ids": [ids[0].0, ids[1].0]})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("selection");
        let view = app.view_info();
        let shift = Mods { shift: true, ..Default::default() };
        for kind in [PointerKind::Down, PointerKind::Up] {
            dispatch(&mut app, &PointerEvent { kind, pos: boards[2].center(), mods: shift, pressure: 1.0 }, view);
        }
        assert_eq!(app.view().unwrap().artboard, 2, "the clicked artboard is the active one");
        assert_eq!(app.session.active().unwrap().selection.objects, ids, "the selection stays");
        let lefts = |app: &VectorcraftApp| ids.map(|id| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0);
        app.run("object.align", json!({"horizontal": "left", "to": "artboard"})).unwrap();
        assert_eq!(lefts(&app), [boards[2].x0; 2], "the left edges are on the active artboard's");
        app.run("object.align", json!({"horizontal": "left", "to": "artboard", "artboard": 1})).unwrap();
        assert_eq!(lefts(&app), [boards[1].x0; 2], "an artboard the caller names is kept");
    }

    /// The canvas outlines the active artboard in black, 1 px wide, and outlines the other
    /// artboards in gray, 0.6 px wide. In the test, the active artboard is Artboard 1 at first,
    /// then the one Go to Artboard went to, and then, with several artboards selected by the
    /// Artboard tool, the tool's active one (the one Shift-clicked last).
    #[test]
    fn the_active_artboard_is_outlined_in_black() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        let ctx = egui::Context::default();
        // → each artboard's edge as drawn: (width, color).
        let edges = |app: &mut VectorcraftApp| {
            let s = shapes(app, &ctx);
            let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
            let on = |p: &[Pos2], q: &[Pos2]| p.len() == q.len() && p.iter().zip(q).all(|(a, b)| (*a - *b).length() < 0.01);
            let boards: Vec<Rect> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
            boards
                .iter()
                .map(|r| {
                    let q = xf.quad(*r);
                    s.iter()
                        .find_map(|s| match s {
                            Shape::Path(p) if p.closed && p.fill == Color32::TRANSPARENT && on(&p.points, &q) => {
                                Some((p.stroke.width, p.stroke.color.clone()))
                            }
                            _ => None,
                        })
                        .expect("the artboard's edge")
                })
                .collect::<Vec<_>>()
        };
        let (black, gray) = (egui::epaint::ColorMode::Solid(Color32::from_gray(0)), egui::epaint::ColorMode::Solid(Color32::from_gray(120)));
        let active = |i: usize| -> Vec<(f32, egui::epaint::ColorMode)> {
            (0..3).map(|j| if j == i { (1.0, black.clone()) } else { (0.6, gray.clone()) }).collect()
        };
        assert_eq!(edges(&mut app), active(0));
        app.run("view.goToArtboard", json!({"index": 1})).unwrap();
        assert_eq!(edges(&mut app), active(1), "Artboard 2 is active: its edge is black and Artboard 1's edge is gray");
        // The Artboard tool starts on Artboard 2; a Shift-click on Artboard 3 selects it too and
        // makes it the active one.
        app.select_tool("artboard");
        let at = app.session.active().unwrap().doc.artboards[2].rect.center();
        let view = app.view_info();
        let shift = Mods { shift: true, ..Default::default() };
        for kind in [PointerKind::Down, PointerKind::Up] {
            dispatch(&mut app, &PointerEvent { kind, pos: at, mods: shift, pressure: 1.0 }, view);
        }
        assert_eq!(app.session.tool_options()["selected"], json!([2, 1]));
        assert_eq!(edges(&mut app), active(2), "Artboard 3 is the active one of the two selected");
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

    /// Double-clicking the name drawn above an artboard edits that name and does not fit a new
    /// artboard or open Artboard Options.
    #[test]
    fn double_clicking_an_artboard_name_edits_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("artboard.new", &json!({"x": 500, "y": 0, "width": 100, "height": 80, "name": "Far"})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 30, "height": 20})).unwrap();
        app.select_tool("artboard");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let xf = Xf::new(rect, app.view().unwrap());
        let mut target = None;
        let _ =
            ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() }, |ui| {
                let ab = &app.session.active().unwrap().doc.artboards[0];
                target = Some(tool_name_rect(ui.painter(), &xf, ab.rect, 0, &ab.name).center());
            });
        let at = target.unwrap();
        assert!(rect.contains(at), "the name is in the working area, at {at:?} canvas {rect:?}");
        let before = app.session.active().unwrap().doc.artboards.len();
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let click = |app: &mut VectorcraftApp| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(true)]);
            frame(app, &ctx, vec![button(false)]);
        };
        click(&mut app);
        click(&mut app);
        assert!(ctx.data(|d| d.get_temp::<ArtboardNameEdit>(name_edit_id()).is_some()), "the name is being edited");
        assert_eq!(app.session.active().unwrap().doc.artboards.len(), before, "clicking the name does not fit an artboard");
        assert!(app.ui.dialog.is_none(), "Artboard Options stays closed");
        commit_artboard_name(&mut app, 0, "Poster");
        assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Poster");
        commit_artboard_name(&mut app, 0, "   ");
        assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Poster", "an empty name is left as it is");
    }

    /// #949: an artboard's name stays within the artboard's width on screen, ending in "…" when
    /// longer, and an artboard too narrow shows none.
    #[test]
    fn artboard_labels_fit_their_artboard() {
        let ctx = egui::Context::default();
        let drawn = |width: f32| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                artboard_label(ui.painter(), pos2(50.0, 50.0), width, "01 - A long artboard name", Color32::WHITE)
            });
            out.textures_delta.clear();
            out.shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.size().x)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let full = drawn(400.0);
        assert_eq!(full.first().map(|(s, _)| s.as_str()), Some("01 - A long artboard name"));
        // Laid out no wider than the artboard (the galley keeps the whole text; it draws "…").
        let (_, w) = drawn(30.0).first().cloned().unwrap();
        assert!(full[0].1 > 30.0 && w <= 30.0 + 0.5, "{full:?} {w}");
        assert!(drawn(10.0).is_empty());
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

    #[test]
    fn new_ruler_guides_follow_alt_through_preview_release_and_undo() {
        use egui::{Event, Modifiers};
        for source_vertical in [false, true] {
            for initially_alt in [false, true] {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
                app.session.execute("document.setRulerOrigin", &json!({"x": 17, "y": 23})).unwrap();
                app.ui.view.rulers = true;
                app.ui.view.smart_guides = false;
                app.ui.view.snap_to_point = false;
                app.select_tool("rectangle");
                let ctx = egui::Context::default();
                frame(&mut app, &ctx, vec![]);
                let rect = app.canvas_rect.unwrap();
                let xf = Xf::new(rect, app.view().unwrap());
                let from =
                    if source_vertical { pos2(rect.left() - RULER / 2.0, rect.center().y) } else { pos2(rect.center().x, rect.top() - RULER / 2.0) };
                let at = xf.to_screen(Point::new(80.0, 120.0));
                assert!(rect.contains(at));
                let held = |alt| if alt { Modifiers::ALT } else { Modifiers::NONE };
                let button = |pos, pressed, alt| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: held(alt) };
                let before = app.session.active().unwrap().doc.clone();
                let undo = app.session.active().unwrap().history.undo.len();
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(held(initially_alt)), Event::PointerMoved(from)]);
                frame(&mut app, &ctx, vec![button(from, true, initially_alt)]);
                frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
                let assert_guide = |app: &VectorcraftApp, alt: bool, pos: Point| {
                    let guides = &app.session.active().unwrap().doc.guides;
                    assert_eq!(guides.len(), 1);
                    assert_eq!(guides[0].vertical, source_vertical != alt);
                    let expected = if source_vertical != alt { pos.x } else { pos.y };
                    assert!((guides[0].pos - expected).abs() < 1e-4, "{:?}, expected {expected}", guides[0]);
                    assert_eq!(guides[0].artboard, None);
                };
                assert_guide(&app, initially_alt, Point::new(80.0, 120.0));
                // Modifier changes with no pointer movement refresh the existing preview.
                for alt in [!initially_alt, initially_alt] {
                    frame(&mut app, &ctx, vec![Event::ModifiersChanged(held(alt))]);
                    assert_guide(&app, alt, Point::new(80.0, 120.0));
                    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
                }
                // Equal coordinates must not hide an orientation change in the preview.
                let equal = xf.to_screen(Point::new(120.0, 120.0));
                frame(&mut app, &ctx, vec![Event::PointerMoved(equal)]);
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(held(!initially_alt))]);
                assert_guide(&app, !initially_alt, Point::new(120.0, 120.0));
                let release = Point::new(147.0, 183.0);
                let to = xf.to_screen(release);
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(to), button(to, false, true)]);
                assert_guide(&app, true, release);
                let st = app.session.active().unwrap();
                assert_eq!(st.history.undo.len(), undo + 1);
                assert_eq!(st.history.undo.last().unwrap().label, "New Guide");
                assert_eq!(st.doc.ruler_origin, before.ruler_origin);
                assert_eq!(st.doc.art_bounds(), before.art_bounds(), "the rectangle tool never drew");
                let committed = st.doc.clone();
                // Canvas guides still span the viewport after changing orientation.
                let painted = shapes(&mut app, &ctx);
                let window = app.canvas_rect.unwrap();
                let position = Xf::new(window, app.view().unwrap()).to_screen(release);
                let ends = if source_vertical {
                    [pos2(window.left(), position.y), pos2(window.right(), position.y)]
                } else {
                    [pos2(position.x, window.top()), pos2(position.x, window.bottom())]
                };
                assert!(painted.iter().any(|s| matches!(s, Shape::LineSegment { points, .. }
                    if (points[0] - ends[0]).length() < 0.01 && (points[1] - ends[1]).length() < 0.01)));
                app.session.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(&*app.session.active().unwrap().doc, &*before);
                app.session.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(&*app.session.active().unwrap().doc, &*committed);
            }
        }
    }

    #[test]
    fn new_ruler_guide_alt_cancel_keeps_document_and_redo() {
        use egui::{Event, Modifiers};
        for source_vertical in [false, true] {
            for tool in ["rectangle", "artboard"] {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
                app.session.execute("document.setRulerOrigin", &json!({"x": 17, "y": 23})).unwrap();
                app.session.execute("shape.rectangle", &json!({"x": 40, "y": 50, "width": 20, "height": 30})).unwrap();
                app.session.execute("edit.undo", &json!({})).unwrap();
                app.ui.view.rulers = true;
                app.ui.view.smart_guides = false;
                app.ui.view.snap_to_point = false;
                app.select_tool(tool);
                let ctx = egui::Context::default();
                frame(&mut app, &ctx, vec![]);
                let rect = app.canvas_rect.unwrap();
                let xf = Xf::new(rect, app.view().unwrap());
                let from =
                    if source_vertical { pos2(rect.left() - RULER / 2.0, rect.center().y) } else { pos2(rect.center().x, rect.top() - RULER / 2.0) };
                let at = xf.to_screen(Point::new(80.0, 120.0));
                let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::ALT };
                let before = app.session.active().unwrap().doc.clone();
                let history = |app: &VectorcraftApp| {
                    let st = app.session.active().unwrap();
                    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
                };
                let before_history = history(&app);
                assert_eq!(before_history.1, 1);
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::ALT), Event::PointerMoved(from)]);
                frame(&mut app, &ctx, vec![button(from, true)]);
                frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
                let guides = &app.session.active().unwrap().doc.guides;
                assert_eq!(guides.len(), 1);
                assert_eq!(guides[0].vertical, !source_vertical);
                let artboard = (tool == "artboard").then_some(before.artboards[0].id);
                assert_eq!(guides[0].artboard, artboard);
                // Returning to the source ruler cancels the preview and keeps the existing Redo.
                frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, false)]);
                assert_eq!(&*app.session.active().unwrap().doc, &*before);
                assert_eq!(history(&app), before_history);
                app.session.execute("edit.redo", &json!({})).unwrap();
                assert!(app.session.active().unwrap().doc.art_bounds().is_some());
                assert!(app.session.active().unwrap().doc.guides.is_empty());
            }
        }
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
        // Direct Selection used last: the drag moves the rectangle's bottom-right corner only (once
        // the rectangle isn't selected whole, which would move all its anchors).
        app.select_tool("directSelection");
        app.session.execute("select.none", &json!({})).unwrap();
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

    /// #2460: type, images, symbol instances, gradient meshes and envelopes count toward the
    /// points the overlay outlines, as paths' anchors do. 32,768 of one kind (an opened PDF can
    /// hold as many type lines, images or mesh pieces) add only the selection's bounds to the
    /// frame's mesh, under 100 vertices; one alone adds its outline.
    #[test]
    fn a_huge_selection_of_type_images_symbols_or_meshes_shows_its_bounds() {
        for kind in ["point type", "area type", "type on a path", "image", "symbol", "mesh", "envelope"] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
            app.ui.task_bar = false;
            // The Rectangle tool draws nothing of its own (no bounding box, no type on a path
            // brackets): only the selection's overlay changes the frame.
            app.select_tool("rectangle");
            match kind {
                "point type" => {
                    app.session.execute("text.create", &json!({"x": 20, "y": 40, "text": "Type"})).unwrap();
                }
                "area type" => {
                    let area = json!({"width": 150, "height": 20});
                    app.session.execute("text.create", &json!({"x": 20, "y": 40, "text": "Type", "area": area})).unwrap();
                }
                "type on a path" => {
                    let path = app.session.execute("path.create", &json!({"d": "M20 100 L140 100 L260 100 L380 100"})).unwrap()["id"].clone();
                    app.session.execute("text.createInPath", &json!({"path": path, "mode": "onPath", "text": "Type"})).unwrap();
                }
                "image" => {
                    let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
                    app.run("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
                }
                "symbol" => {
                    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 20})).unwrap();
                    app.session.execute("symbol.new", &json!({})).unwrap();
                }
                "mesh" => {
                    app.session.execute("shape.rectangle", &json!({"x": 150, "y": 50, "width": 30, "height": 20})).unwrap();
                    app.session.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
                }
                _ => {
                    app.session.execute("shape.rectangle", &json!({"x": 150, "y": 50, "width": 30, "height": 20})).unwrap();
                    app.session.execute("object.envelope.makeWithMesh", &json!({"rows": 1, "cols": 1})).unwrap();
                }
            }
            let ctx = egui::Context::default();
            // The vertices that selecting everything adds to the frame's mesh.
            let overlay = |app: &mut VectorcraftApp| {
                app.session.execute("select.none", &json!({})).unwrap();
                let none = frame_vertices(app, &ctx);
                app.session.execute("select.all", &json!({})).unwrap();
                frame_vertices(app, &ctx).saturating_sub(none)
            };
            let one = overlay(&mut app);
            assert!(one > 0, "{kind}: one shows its outline");
            for _ in 0..15 {
                app.session.execute("select.all", &json!({})).unwrap();
                app.run("edit.duplicate", json!({})).unwrap();
            }
            let many = overlay(&mut app);
            assert_eq!(app.session.active().unwrap().selection.objects.len(), 32_768, "{kind}");
            assert!(many < 100, "{kind}: 32,768 show the selection's bounds, not {many} vertices");
        }
    }

    /// #2460: a group of 32,768 images is too much to outline: under the pointer it is not
    /// highlighted, and as the key object its outline is its bounds.
    #[test]
    fn a_group_of_too_many_images_has_no_hover_outline_and_a_key_outline_of_its_bounds() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
        app.run("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
        for _ in 0..15 {
            app.session.execute("select.all", &json!({})).unwrap();
            app.run("edit.duplicate", json!({})).unwrap();
        }
        app.session.execute("select.all", &json!({})).unwrap();
        let group = app.session.execute("object.group", &json!({})).unwrap()["id"].clone();
        app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
        app.select_tool("selection");
        app.session.execute("select.none", &json!({})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let over = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(200.0, 150.0));
        let lines = |app: &mut VectorcraftApp, width: f32| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(over)]);
            shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Path(ps) if ps.stroke.width == width)).count()
        };
        assert_eq!(lines(&mut app, 1.5), 0, "no hover outline");
        app.session.execute("select.all", &json!({})).unwrap();
        app.session.execute("select.key", &json!({"id": group})).unwrap();
        assert_eq!(lines(&mut app, KEY_OUTLINE), 1, "the key object's bounds");
    }

    /// #1111: a selected gradient mesh shows its mesh lines and points to the selection tools, not
    /// only to the Mesh tool.
    #[test]
    fn a_selected_gradient_mesh_shows_its_lines_and_points() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 200, "height": 100})).unwrap();
        app.session.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
        let ctx = egui::Context::default();
        let drawn = |app: &mut VectorcraftApp| {
            frame(app, &ctx, vec![]);
            let shapes = shapes(app, &ctx);
            let lines = shapes.iter().filter(|s| matches!(s, Shape::Path(ps) if ps.points.len() > 1)).count();
            (lines, shapes.iter().filter(|s| matches!(s, Shape::Rect(_))).count())
        };
        let (none_lines, none_points) = {
            app.session.execute("select.none", &json!({})).unwrap();
            drawn(&mut app)
        };
        for tool in ["selection", "directSelection"] {
            app.select_tool(tool);
            app.session.execute("select.all", &json!({})).unwrap();
            let (lines, points) = drawn(&mut app);
            // The outline, two inner lines each way; the 3 × 3 points.
            assert!(lines >= none_lines + 3 && points >= none_points + 9, "{tool}: {lines} lines, {points} points");
        }
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

    /// Two headless canvas frames → the vertices of the second one's tessellated mesh, which the
    /// GPU takes in one buffer.
    fn frame_vertices(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
        frame(app, ctx, vec![]);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let mesh = |p: &egui::ClippedPrimitive| if let egui::epaint::Primitive::Mesh(m) = &p.primitive { m.vertices.len() } else { 0 };
        ctx.tessellate(out.shapes, out.pixels_per_point).iter().map(mesh).sum()
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

    /// #1278: a segment click marks the edge, not its endpoint anchors or neighboring handles.
    #[test]
    fn a_segment_pick_has_hollow_anchors_and_only_its_own_handles() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":400,"height":300})).unwrap();
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        let id = app.run("shape.rectangle", json!({"x":50,"y":50,"width":200,"height":150})).unwrap()["id"].as_u64().unwrap();
        app.run("select.none", json!({})).unwrap();
        app.select_tool("directSelection");
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 150.0, 50.0), view).unwrap();
        }
        let before = app.session.doc().unwrap().doc.clone();
        let color = c32(before.layer_color(vectorcraft_doc::NodeId(id)));
        let selected = selected_anchor(color);
        let ctx = egui::Context::default();
        let painted = shapes(&mut app, &ctx);
        assert_eq!(painted.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == selected)).count(), 0);
        assert_eq!(
            painted.iter().filter(|s| matches!(s, Shape::Path(p) if p.stroke.width == 2.0 && p.points.len() == 2)).count(),
            1,
            "only the chosen straight edge has emphasis"
        );
        assert!(!painted.iter().any(|s| matches!(s, Shape::Circle(_))), "segment picks have no corner widgets");
        assert_eq!(app.session.doc().unwrap().doc, before);
        app.run("select.anchors", json!({"id":id,"anchors":[[0,0],[0,1]]})).unwrap();
        let painted = shapes(&mut app, &ctx);
        assert_eq!(painted.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == selected)).count(), 2);
        app.run(
            "path.create",
            json!({"anchors":[{"x":50,"y":260,"out":[100,220]},{"x":200,"y":260,"in":[150,220],"out":[250,300]},{"x":330,"y":260,"in":[280,300]}]}),
        )
        .unwrap();
        app.run("select.none", json!({})).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 125.0, 230.0), view).unwrap();
        }
        let before = app.session.doc().unwrap().doc.clone();
        let painted = shapes(&mut app, &ctx);
        assert_eq!(
            painted.iter().filter(|s| matches!(s, Shape::LineSegment {stroke,..} if stroke.color == color)).count(),
            2,
            "only the chosen curve's two controls show"
        );
        app.ui.view.edges = false;
        let painted = shapes(&mut app, &ctx);
        assert!(!painted.iter().any(|s| matches!(s, Shape::LineSegment {stroke,..} if stroke.color == color)));
        assert_eq!(app.session.doc().unwrap().doc, before);
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

    /// View › Hide Template (#2000): a template layer's art leaves the canvas, and Show Template
    /// draws it again, dimmed as before; each re-renders at once.
    #[test]
    fn hide_template_leaves_template_layers_off_the_canvas() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
        let layer = app.session.active().unwrap().doc.layers[0].id.0;
        app.session.execute("layer.template", &json!({"ids": [layer], "on": true})).unwrap();
        let ctx = egui::Context::default();
        // How many pixels of the art's raster one frame uploads are painted, None when it was not
        // re-rendered.
        let painted = |app: &mut VectorcraftApp| -> Option<usize> {
            let (_, mut delta) = frame_output(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let n = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
                let egui::ImageData::Color(img) = &d.image;
                img.pixels.iter().filter(|c| c.a() != 0).count()
            });
            delta.clear();
            n
        };
        let shown = painted(&mut app).expect("the first frame renders the art");
        assert!(shown > 1000, "the template layer's art is drawn: {shown} pixels");
        assert_eq!(app.run("view.template", json!({})).unwrap(), json!(false));
        assert_eq!(painted(&mut app), Some(0), "Hide Template re-renders without the template layer");
        assert_eq!(painted(&mut app), None, "nothing changed: no re-render");
        assert_eq!(app.run("view.template", json!({})).unwrap(), json!(true));
        assert_eq!(painted(&mut app), Some(shown), "Show Template draws it again as before");
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

    /// A drag of an object with raster effects (#1257): the drag's frames render while
    /// interacting and draw the object's drop shadow and filtered content from the rasters of the
    /// frame before; the frame after the drag renders without `interacting`, on the render worker,
    /// and draws what a fresh renderer draws.
    #[test]
    fn dragging_an_object_with_raster_effects_draws_its_rasters_moved() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 120, "height": 90})).unwrap()["id"].clone();
        for effect in ["stylize.dropShadow", "texture.craquelure"] {
            app.run("effect.apply", json!({"effect": effect, "ids": [id]})).unwrap();
        }
        let ctx = egui::Context::default();
        // The art's raster uploaded by one frame, None when it was not re-rendered.
        let upload = |app: &mut VectorcraftApp| -> Option<Vec<Color32>> {
            let (_, mut delta) = frame_output(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
                let egui::ImageData::Color(img) = &d.image;
                img.pixels.clone()
            });
            delta.clear();
            px
        };
        upload(&mut app).expect("the first frame renders the art");
        app.select_tool("selection");
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 160.0, 125.0), view).unwrap();
        for step in [1.0, 2.0, 3.0] {
            app.session.pointer(&PointerEvent::new(PointerKind::Drag, 160.0 + 7.3 * step, 125.0 + 2.1 * step), view).unwrap();
            upload(&mut app).expect("each frame of the drag renders");
            assert!(app.canvas.key.as_ref().is_some_and(|k| k.interacting), "step {step}");
            assert_eq!(app.canvas.renderer.stats.moved, 2, "the shadow and the filtered content, step {step}");
            assert!(app.canvas.moved);
        }
        // The frame after the drag runs the effects again: on the worker, while the canvas shows
        // the drag's last frame.
        app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 181.9, 131.3), view).unwrap();
        assert!(upload(&mut app).is_none(), "not rendered on this thread");
        assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| !k.interacting), "sent to the worker");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut after = None;
        while after.is_none() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
            after = upload(&mut app);
        }
        let after = after.expect("the worker's frame arrives");
        assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting));
        assert!(!app.canvas.moved);
        app.canvas.worker = None;
        app.canvas.renderer = vectorcraft_render::Renderer::new();
        app.canvas.key = None;
        assert!(upload(&mut app) == Some(after), "the frame after the drag is what a fresh renderer draws");
    }

    /// The frame after an interaction renders without `interacting` also when the interaction
    /// leaves the document as it was (#1257): here a drag back to where it started.
    #[test]
    fn a_drag_back_to_its_start_renders_again_after_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 120, "height": 90})).unwrap()["id"].clone();
        app.run("effect.apply", json!({"effect": "texture.craquelure", "ids": [id]})).unwrap();
        let ctx = egui::Context::default();
        shapes(&mut app, &ctx);
        app.select_tool("selection");
        let view = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 160.0, 125.0), view).unwrap();
        for (x, y) in [(173.3, 129.1), (160.0, 125.0)] {
            app.session.pointer(&PointerEvent::new(PointerKind::Drag, x, y), view).unwrap();
            shapes(&mut app, &ctx);
            assert_eq!(app.canvas.renderer.stats.moved, 1, "the filtered content at ({x}, {y})");
        }
        let revision = app.session.active().unwrap().revision;
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 160.0, 125.0), view).unwrap();
        assert_eq!(app.session.active().unwrap().revision, revision, "the drag left the document as it was");
        shapes(&mut app, &ctx);
        assert!(app.canvas.key.as_ref().is_some_and(|k| !k.interacting), "rendered again");
        assert_eq!(app.canvas.renderer.stats.moved, 0);
        assert!(!app.canvas.moved);
    }

    /// When the view turns while the worker renders the frame at the new angle (Reset Rotate View on
    /// a heavy document), the canvas draws the last frame turned until that frame arrives: each
    /// corner goes where the new view puts the document point that the corner showed. The new frame
    /// is then drawn unturned. In Pixel Preview the last frame's pixels stay over the document rect
    /// they cover.
    #[test]
    fn a_turned_view_draws_the_last_frame_turned_until_its_own_arrives() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 200, "height": 200})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        let ctx = egui::Context::default();
        shapes(&mut app, &ctx); // the first frame fits the view to the artboard
        app.view_mut().unwrap().rotation = 30.0;
        shapes(&mut app, &ctx);
        let rect = app.canvas_rect.unwrap();
        let old = Xf::new(rect, app.view().unwrap());
        // `art` runs one frame and finds the mesh that draws the art's texture.
        let art = |app: &mut VectorcraftApp| {
            let s = shapes(app, &ctx);
            let tex = app.canvas.texture.as_ref().unwrap().id();
            s.into_iter().find_map(|s| match s {
                Shape::Mesh(m) if m.texture_id == tex => Some(m),
                _ => None,
            })
        };
        // With a worker and a last frame that took 50 ms, the next frame renders on the worker.
        app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
        app.canvas.last_ms = 50.0;
        app.run("view.rotateReset", json!({})).unwrap();
        let mesh = art(&mut app).expect("the last frame, turned");
        assert!(app.canvas.worker.as_ref().and_then(|w| w.pending.as_ref()).is_some_and(|k| k.rot == 0.0), "sent to the worker");
        let new = Xf::new(rect, app.view().unwrap());
        let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        assert_eq!(mesh.vertices.len(), 4);
        for ((v, corner), uv) in mesh.vertices.iter().zip(corners).zip([pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)]) {
            let at = new.to_screen(old.to_doc(corner));
            assert!((v.pos - at).length() < 0.01 && v.uv == uv, "{v:?}: texture corner {uv:?} at {at:?}");
        }
        // The point where the last frame drew the rectangle's top-left corner lands where the new view puts that corner.
        let p = Point::new(100.0, 50.0);
        let uv = (old.to_screen(p) - rect.min) / rect.size();
        let [a, b, _, d] = [0, 1, 2, 3].map(|i| mesh.vertices[i].pos);
        assert!((a + (b - a) * uv.x + (d - a) * uv.y - new.to_screen(p)).length() < 0.01, "the rectangle's corner");
        // The worker's frame at 0° then covers the canvas unturned.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app.canvas.key.as_ref().is_some_and(|k| k.rot != 0.0) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
            shapes(&mut app, &ctx);
        }
        assert!(app.canvas.key.as_ref().is_some_and(|k| k.rot == 0.0), "the worker's frame arrives");
        let b = art(&mut app).expect("the worker's frame").calc_bounds();
        assert!((b.min - rect.min).length() < 0.01 && (b.max - rect.max).length() < 0.01, "{b:?} over {rect:?}");
        // In Pixel Preview the last frame's pixels are drawn over the document rect they cover.
        app.canvas.worker = None;
        app.ui.view.pixel_preview = true;
        app.view_mut().unwrap().zoom = 8.0;
        shapes(&mut app, &ctx);
        let [x0, y0, ..] = app.canvas.key.as_ref().and_then(|k| k.pixel).expect("the pixels rendered");
        app.canvas.worker = crate::render_worker::Worker::spawn(ctx.clone());
        app.canvas.last_ms = 50.0;
        app.view_mut().unwrap().rotation = 30.0;
        let mesh = art(&mut app).expect("the last frame's pixels");
        let at = Xf::new(rect, app.view().unwrap()).to_screen(Point::new(x0 as f64, y0 as f64));
        assert!((mesh.vertices[0].pos - at).length() < 0.01, "{:?} at {at:?}", mesh.vertices[0].pos);
        app.canvas.worker = None;
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

    /// Appearance of Black › On Screen (#394): Display All Blacks Accurately (the default) shows
    /// pure CMYK black through the CMYK profile, a dark grey; Display All Blacks as Rich Black
    /// shows it as RGB black. A rich black shows the same either way.
    #[test]
    fn appearance_of_black_on_screen_shows_pure_black_as_rgb_black() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300, "colorMode": "cmyk"})).unwrap();
        app.canvas.worker_started = true; // render on this thread
        // Larger than the view: the middle of the art's raster is its fill.
        let id = app.run("shape.rectangle", json!({"x": -5000, "y": -5000, "width": 10000, "height": 10000})).unwrap()["id"].clone();
        let fill = |app: &mut VectorcraftApp, cmyk: [u8; 4]| {
            let [c, m, y, k] = cmyk;
            app.run("paint.setFill", json!({"ids": [id], "color": {"c": c, "m": m, "y": y, "k": k}})).unwrap();
        };
        let ctx = egui::Context::default();
        // The middle pixel of the art's raster uploaded by one frame, None when it was not re-rendered.
        let middle = |app: &mut VectorcraftApp| -> Option<[u8; 4]> {
            let (_, mut delta) = frame_output(app, &ctx);
            let art = app.canvas.texture.as_ref().unwrap().id();
            let px = delta.set.iter().find(|e| e.0.eq(&art)).and_then(|e| e.1.last()).map(|d| {
                let egui::ImageData::Color(img) = &d.image;
                let [w, h] = img.size;
                img.pixels[h / 2 * w + w / 2].to_array()
            });
            delta.clear();
            px
        };
        let shown = |cmyk: [f32; 4]| vectorcraft_color::Color::cmyk(cmyk[0], cmyk[1], cmyk[2], cmyk[3]).to_rgba8(1.0);
        let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1);

        fill(&mut app, [0, 0, 0, 100]);
        let accurate = middle(&mut app).expect("the first frame renders the art");
        assert!(near(accurate, shown([0.0, 0.0, 0.0, 1.0])), "accurate: through the CMYK profile, {accurate:?}");
        assert!(accurate[..3].iter().all(|v| *v > 10), "accurate: a dark grey, {accurate:?}");
        app.run("prefs.set", json!({"key": "blackOnScreen", "value": "rich"})).unwrap();
        assert_eq!(middle(&mut app).expect("the preference change re-renders"), [0, 0, 0, 255], "rich: RGB black");

        fill(&mut app, [60, 40, 40, 100]);
        let rich = middle(&mut app).expect("re-rendered");
        assert!(near(rich, shown([0.6, 0.4, 0.4, 1.0])), "a rich black is not pure black: {rich:?}");

        fill(&mut app, [0, 0, 0, 100]);
        app.run("prefs.set", json!({"key": "blackOnScreen", "value": "accurate"})).unwrap();
        assert!(near(middle(&mut app).expect("re-rendered"), accurate), "accurate again");
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

    /// A canvas guide (on no artboard) is drawn along its document line in a rotated view, where it
    /// snaps and is picked: both ends of each guide's line are on x = 100 or y = 120. At 0° the
    /// vertical one runs from the canvas's top to its bottom.
    #[test]
    fn canvas_guides_turn_with_the_view() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
        app.session.execute("guide.add", &json!({"vertical": false, "pos": 120})).unwrap();
        app.session.execute("prefs.set", &json!({"key": "guideColor", "value": "#123456"})).unwrap();
        app.ui.view.guides = true;
        let color = Color32::from_rgb(0x12, 0x34, 0x56);
        let ctx = egui::Context::default();
        let lines = |app: &mut VectorcraftApp| -> Vec<[Pos2; 2]> {
            shapes(app, &ctx)
                .into_iter()
                .filter_map(|s| if let Shape::LineSegment { points, stroke } = s { (stroke.color == color).then_some(points) } else { None })
                .collect()
        };
        let s = lines(&mut app);
        let rect = app.canvas_rect.unwrap();
        let vertical = s.iter().find(|[a, b]| (a.x - b.x).abs() < 1e-3).expect("the vertical guide");
        assert!((vertical[0].y - rect.top()).abs() < 0.5 && (vertical[1].y - rect.bottom()).abs() < 0.5, "0°: {vertical:?} in {rect:?}");
        app.run("view.rotate", json!({"angle": 30})).unwrap();
        let s = lines(&mut app);
        let xf = Xf::new(rect, app.view().unwrap());
        assert_eq!(s.len(), 2, "{s:?}");
        let on = |v: f64, along_x: bool| {
            s.iter().any(|ends| ends.iter().all(|e| ((if along_x { xf.to_doc(*e).x } else { xf.to_doc(*e).y }) - v).abs() < 1e-3))
        };
        assert!(on(100.0, true), "x = 100: {s:?}");
        assert!(on(120.0, false), "y = 120: {s:?}");
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

    /// #1236: Curvature edits points, so existing paths show one circular marker per point and
    /// no direction handles; switching back to Direct Selection keeps its normal handles.
    #[test]
    fn curvature_points_of_an_existing_path_have_no_direction_handles() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = vectorcraft_doc::NodeId(
            app.session
                .execute(
                    "path.curvature",
                    &json!({"points": [
                        {"x": 60, "y": 200}, {"x": 120, "y": 80}, {"x": 210, "y": 160}, {"x": 280, "y": 60}, {"x": 350, "y": 180}
                    ]}),
                )
                .unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        let document = app.session.active().unwrap().doc.clone();
        let ctx = egui::Context::default();
        app.select_tool("curvature");
        let drawn = shapes(&mut app, &ctx);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let points: Vec<_> = document.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| xf.to_screen(a.p)).collect();
        let marker = |shape: &Shape| matches!(shape, Shape::Circle(c) if points.iter().any(|p| p.distance(c.center) < 0.01));
        assert_eq!(drawn.iter().filter(|s| marker(s)).count(), 5, "one circular marker at every curve point");
        assert!(
            !drawn.iter().any(|s| matches!(s, Shape::Rect(r) if points.iter().any(|p| p.distance(r.rect.center()) < 0.01))),
            "no square at a curve point"
        );
        let color = c32(document.layer_color(id));
        let handles = |shapes: &[Shape]| shapes.iter().filter(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color)).count();
        assert_eq!(handles(&drawn), 0, "Curvature hides the direction lines");
        app.session.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "set"})).unwrap();
        let partial = shapes(&mut app, &ctx);
        assert_eq!(partial.iter().filter(|s| marker(s)).count(), 5, "other curve points stay visible");
        assert_eq!(handles(&partial), 0, "a single selected point still shows no direction lines");
        app.ui.view.edges = false;
        assert_eq!(shapes(&mut app, &ctx).iter().filter(|s| marker(s)).count(), 0, "Hide Edges still hides an existing path's markers");
        app.ui.view.edges = true;
        app.select_tool("directSelection");
        assert_eq!(handles(&shapes(&mut app, &ctx)), 2, "Direct Selection keeps the selected point's two handles");
        app.select_tool("pen");
        assert_eq!(handles(&shapes(&mut app, &ctx)), 2, "Pen keeps the selected point's two handles");
        assert_eq!(
            serde_json::to_value(&app.session.active().unwrap().doc).unwrap(),
            serde_json::to_value(&document).unwrap(),
            "tool switches and painting don't change the curve"
        );
    }

    /// While drawing, Curvature's current-point feedback takes the place of generic selection
    /// markers: no duplicate squares or handles, and the path is unchanged by drawing a frame.
    #[test]
    fn curvature_drawing_shows_its_points_once() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        shapes(&mut app, &ctx);
        app.select_tool("curvature");
        for (x, y) in [(60.0, 200.0), (120.0, 80.0), (210.0, 160.0), (280.0, 60.0), (350.0, 180.0)] {
            let view = app.view_info();
            for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
                app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, x, y), view).unwrap();
            }
        }
        let st = app.session.active().unwrap();
        let id = st.selection.objects[0];
        let document = st.doc.clone();
        let selection = st.selection.clone();
        let drawn = shapes(&mut app, &ctx);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let points: Vec<_> = document.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| xf.to_screen(a.p)).collect();
        let markers: Vec<_> = drawn
            .iter()
            .filter_map(|s| match s {
                Shape::Circle(c) if points.iter().any(|p| p.distance(c.center) < 0.01) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(markers.len(), 5, "one marker per point, including the current point");
        assert_eq!(markers.iter().filter(|c| c.fill != Color32::WHITE).count(), 1, "only the last touched point is filled");
        assert!(
            !drawn.iter().any(|s| matches!(s, Shape::Rect(r) if points.iter().any(|p| p.distance(r.rect.center()) < 0.01))),
            "neither tool nor selection draws anchor squares"
        );
        let color = c32(document.layer_color(id));
        assert!(!drawn.iter().any(|s| matches!(s, Shape::LineSegment { stroke, .. } if stroke.color == color)), "no direction lines");
        app.session.execute("prefs.set", &json!({"key": "anchorSize", "value": 7})).unwrap();
        assert_eq!(
            shapes(&mut app, &ctx)
                .iter()
                .filter(|s| matches!(s, Shape::Circle(c) if c.radius == 5.0 && points.iter().any(|p| p.distance(c.center) < 0.01)))
                .count(),
            5,
            "point markers respect Anchor Size"
        );
        assert_eq!(app.session.active().unwrap().selection, selection);
        assert_eq!(serde_json::to_value(&app.session.active().unwrap().doc).unwrap(), serde_json::to_value(&document).unwrap());
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

    /// View › Hide Edges hides a live rectangle's corner widgets, and the Selection tool then
    /// neither shows the corner cursor over one's place nor rounds the corners from it. With
    /// edges shown, a drag from the same widget rounds them.
    #[test]
    fn hidden_corner_widgets_do_not_answer_the_pointer() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("selection");
        let widget = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false).unwrap().visible().next().unwrap()
        };
        let radius = |app: &VectorcraftApp| {
            let c = vectorcraft_doc::LiveCorners::of(app.session.active().unwrap().doc.node(id).unwrap()).unwrap();
            c.style(&c.all()).0
        };
        let drag = |app: &mut VectorcraftApp, from: Point| {
            let (view, to) = (app.view_info(), from + vectorcraft_geom::Vec2::new(10.0, 10.0));
            for (kind, at) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
                app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
            }
        };
        app.ui.view.edges = false;
        let at = widget(&app);
        let view = app.view_info();
        assert_ne!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
        drag(&mut app, at);
        assert_eq!(radius(&app), Some(0.0), "a press on a hidden widget does not round the corners");
        app.ui.view.edges = true;
        let at = widget(&app);
        let view = app.view_info();
        assert_eq!(app.session.cursor(at, Mods::default(), view), Cursor::CornerRadius);
        drag(&mut app, at);
        assert!(radius(&app).is_some_and(|r| r > 0.0), "{:?}", radius(&app));
    }

    /// View › Hide Edges hides the bounding box and the type widget beside it, and the Selection
    /// tool then answers neither: a double-click on the widget's place leaves point type as it is,
    /// and a drag from a corner handle's place does not scale. With edges shown, both answer.
    #[test]
    fn a_hidden_bounding_box_and_type_widget_do_not_answer_the_pointer() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type"})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.select_tool("selection");
        let bx = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            st.doc.oriented_bounds(&st.selection.objects, app.session.prefs.use_preview_bounds).unwrap()
        };
        let widget = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            vectorcraft_tools::typewidget::TypeWidget::of(&st.doc, &st.selection, &bx(app), app.view_info().zoom).unwrap()
        };
        let area = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Text(t) => matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }),
            k => panic!("not type: {k:?}"),
        };
        let double_click = |app: &mut VectorcraftApp, at: Point| {
            let view = app.view_info();
            for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
                app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
            }
        };
        let width = |app: &VectorcraftApp| bx(app).rect.width();
        let scale = |app: &mut VectorcraftApp| {
            let b = bx(app);
            let corner = b.to_doc() * Point::new(b.rect.x1, b.rect.y1);
            let (view, to) = (app.view_info(), corner + vectorcraft_geom::Vec2::new(30.0, 0.0));
            for (kind, at) in [(PointerKind::Down, corner), (PointerKind::Drag, to), (PointerKind::Up, to)] {
                app.session.pointer(&PointerEvent::new(kind, at.x, at.y), view).unwrap();
            }
            app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        };
        let w = widget(&app);
        app.ui.view.edges = false;
        let view = app.view_info();
        assert_ne!(app.session.cursor(w.at, Mods::default(), view), Cursor::TypeWidget);
        double_click(&mut app, w.at);
        assert!(!area(&app), "a double-click on the hidden widget leaves point type");
        app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let before = width(&app);
        scale(&mut app);
        assert!((width(&app) - before).abs() < 1e-6, "a drag from a hidden handle does not scale: {before} -> {}", width(&app));
        app.ui.view.edges = true;
        let w = widget(&app);
        let view = app.view_info();
        assert_eq!(app.session.cursor(w.at, Mods::default(), view), Cursor::TypeWidget);
        scale(&mut app);
        assert!(width(&app) > before + 1.0, "a drag from the handle scales: {before} -> {}", width(&app));
        let w = widget(&app);
        double_click(&mut app, w.at);
        assert!(area(&app), "a double-click on the widget converts to area type");
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

    /// A selected live ellipse shows its two pie widgets with the Selection tool while the
    /// bounding box shows.
    #[test]
    fn a_selected_ellipse_shows_its_pie_widgets() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.session.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
        app.session.execute("object.setLiveShape", &json!({"pieStart": 0, "pieEnd": 90})).unwrap();
        let ctx = egui::Context::default();
        let widgets = |app: &mut VectorcraftApp| {
            let r = vectorcraft_tools::pie::RADIUS_PX;
            shapes(app, &ctx).iter().filter(|s| matches!(s, Shape::Circle(c) if c.radius == r && c.fill == Color32::WHITE)).count()
        };
        app.select_tool("selection");
        assert_eq!(widgets(&mut app), 2);
        app.select_tool("directSelection");
        assert_eq!(widgets(&mut app), 0);
        app.select_tool("selection");
        app.run("view.boundingBox", json!({})).unwrap();
        assert_eq!(widgets(&mut app), 0, "no bounding box, no widgets");
    }

    /// The shapes Type → Show Hidden Characters draws in one frame, seen through `xf`.
    fn hidden_chars(app: &mut VectorcraftApp, ctx: &egui::Context, xf: &Xf) -> Vec<Shape> {
        let raw = egui::RawInput { screen_rect: Some(xf.rect), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| hidden_chars_overlay(app, ui.painter(), xf));
        out.textures_delta.clear();
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    /// Show Hidden Characters lays out type once: a frame keeps the marks of the frame before, and
    /// after an edit only the object edited is laid out again.
    #[test]
    fn show_hidden_characters_lays_out_only_type_that_changed() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ids: Vec<_> = (0..3)
            .map(|i| app.session.execute("text.create", &json!({"x": 20, "y": 40 + 80 * i, "text": "a b\nc d"})).unwrap()["id"].as_u64().unwrap())
            .map(vectorcraft_doc::NodeId)
            .collect();
        let ctx = egui::Context::default();
        let xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
        // `frame` draws one frame and returns each object's marks and whether that frame found the
        // fonts and hyphenation exceptions of the frame before.
        let frame = |app: &mut VectorcraftApp| {
            let inputs = app.canvas.hidden_chars.inputs.clone();
            hidden_chars(app, &ctx, &xf);
            let kept = &app.canvas.hidden_chars;
            (kept.inputs == inputs, ids.iter().map(|id| kept.marks[id].1.clone()).collect::<Vec<_>>())
        };
        // `frames` draws a frame, runs `edit` and draws the next frame. Tests running alongside add
        // fonts and set hyphenation exceptions, which lays all type out again: `frames` draws both
        // frames again until neither the fonts nor the exceptions changed in between.
        let frames = |app: &mut VectorcraftApp, edit: &dyn Fn(&mut VectorcraftApp)| {
            (0..100)
                .find_map(|_| {
                    let (_, before) = frame(app);
                    edit(app);
                    let (same, after) = frame(app);
                    same.then_some((before, after))
                })
                .unwrap()
        };
        let same = |a: &std::sync::Arc<HiddenMarks>, b: &std::sync::Arc<HiddenMarks>| std::sync::Arc::ptr_eq(a, b);
        let (before, after) = frames(&mut app, &|_| {});
        assert!(before.iter().zip(&after).all(|(a, b)| same(a, b)), "a frame lays out no type");
        let (before, after) = frames(&mut app, &|app| {
            app.session.execute("text.setText", &json!({"id": ids[1].0, "text": "a b c"})).unwrap();
        });
        assert!(!same(&before[1], &after[1]), "the edited object is laid out again");
        assert!(same(&before[0], &after[0]) && same(&before[2], &after[2]), "the others are not");
    }

    /// The marks Show Hidden Characters keeps follow the type: after an edit, a zoom and a font
    /// that arrives, a frame draws what a new overlay draws, and the marks moved.
    #[test]
    fn show_hidden_characters_follows_edits_zooms_and_fonts() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        // The type is set in Bold of a family that is not installed and whose name starts with a
        // dot; the font menus do not list such names. Source Sans 3 Bold stands in for it until the
        // family arrives below with a Regular face only.
        let text = json!({"x": 20, "y": 60, "text": "one two\nthree four", "font": ".Hidden Marks", "style": "Bold"});
        let id = app.session.execute("text.create", &text).unwrap()["id"].clone();
        let ctx = egui::Context::default();
        let mut xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 1.0, center: Point::new(200.0, 150.0), rot: 0.0 };
        // `drawn` draws a frame with what the overlay kept and checks it against a new overlay.
        let drawn = |app: &mut VectorcraftApp, xf: &Xf| {
            let kept = hidden_chars(app, &ctx, xf);
            app.canvas.hidden_chars = Default::default();
            assert_eq!(hidden_chars(app, &ctx, xf), kept, "as a new overlay draws them");
            kept
        };
        let start = drawn(&mut app, &xf);
        assert!(start.len() > 2, "{start:?}");
        app.session.execute("text.setText", &json!({"id": id, "text": "one two three\nfour"})).unwrap();
        let edited = drawn(&mut app, &xf);
        assert_ne!(edited, start, "the edit moved the marks");
        xf.zoom = 2.0;
        let zoomed = drawn(&mut app, &xf);
        assert_ne!(zoomed, edited, "zoomed in");
        assert!(vectorcraft_text::FontDb::global().add_font(vectorcraft_testkit::fonts::renamed(".Hidden Marks")) > 0, "the family is new");
        assert_ne!(drawn(&mut app, &xf), zoomed, "set in the family that arrived");
    }

    /// In a rotated view Show Hidden Characters draws the marks of type whose turned outline reaches
    /// into the canvas, also when the box between two of its corners on screen does not.
    #[test]
    fn show_hidden_characters_draws_type_partly_on_a_rotated_canvas() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 60, "text": "a b c d e\nf g h"})).unwrap()["id"].as_u64().unwrap();
        let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        let ctx = egui::Context::default();
        let mut xf = Xf { rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)), zoom: 2.0, center: b.center(), rot: 45f64.to_radians() };
        // The screen moves so that the canvas's left edge falls halfway between the right side of
        // the box and the outline's rightmost corner, level with that corner.
        let q = xf.quad(b);
        let right = q.iter().copied().max_by(|a, c| a.x.total_cmp(&c.x)).unwrap();
        let shift = vec2(-(xf.rect_to_screen(b).right() + right.x) / 2.0, xf.rect.center().y - right.y);
        xf.center -= xf.delta_to_doc(shift);
        assert!(!xf.rect_to_screen(b).intersects(xf.rect), "the box is off the canvas");
        assert!(xf.quad(b).iter().any(|c| xf.rect.contains(*c)), "a corner is on it");
        assert!(!hidden_chars(&mut app, &ctx, &xf).is_empty(), "the marks are drawn");
    }

    /// The shapes Type → Show Hidden Characters draws in one canvas frame ([`show`]).
    fn hidden_char_marks(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Shape> {
        let blue = Color32::from_rgb(0x4f, 0x9d, 0xff);
        shapes(app, ctx)
            .into_iter()
            .filter(|s| matches!(s, Shape::Circle(c) if c.fill == blue) || matches!(s, Shape::Text(t) if t.fallback_color == blue))
            .collect()
    }

    /// While Show Hidden Characters is on, a canvas frame keeps the marks of the frame before.
    /// Turning it off drops the marks with their text nodes, and turning it on again draws the
    /// same marks.
    #[test]
    fn show_hidden_characters_off_releases_the_text_nodes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let text = json!({"x": 20, "y": 60, "text": "one two\nthree four"});
        let id = vectorcraft_doc::NodeId(app.session.execute("text.create", &text).unwrap()["id"].as_u64().unwrap());
        let ctx = egui::Context::default();
        app.run("type.hiddenCharacters", json!({})).unwrap();
        let on = hidden_char_marks(&mut app, &ctx);
        assert!(on.len() > 2, "{on:?}");
        // Tests running alongside add fonts and set hyphenation exceptions, which lays all type out
        // again: frames are drawn until neither changed between two of them.
        let kept = (0..100).any(|_| {
            let (inputs, marks) = (app.canvas.hidden_chars.inputs.clone(), app.canvas.hidden_chars.marks[&id].1.clone());
            hidden_char_marks(&mut app, &ctx);
            let after = &app.canvas.hidden_chars;
            after.inputs == inputs && std::sync::Arc::ptr_eq(&after.marks[&id].1, &marks)
        });
        assert!(kept, "a frame keeps the marks of the frame before");
        app.run("type.hiddenCharacters", json!({})).unwrap();
        assert!(hidden_char_marks(&mut app, &ctx).is_empty(), "off");
        assert!(app.canvas.hidden_chars.marks.is_empty(), "no node is kept");
        app.run("type.hiddenCharacters", json!({})).unwrap();
        assert_eq!(hidden_char_marks(&mut app, &ctx), on, "on again: the same marks");
    }

    /// Closing the document drops the marks Show Hidden Characters kept with its text nodes, and
    /// the same type in a new document shows the same marks.
    #[test]
    fn show_hidden_characters_releases_the_text_nodes_of_a_closed_document() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.run("type.hiddenCharacters", json!({})).unwrap();
        let open = |app: &mut VectorcraftApp| {
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.session.execute("text.create", &json!({"x": 20, "y": 60, "text": "one two\nthree four"})).unwrap();
            hidden_char_marks(app, &ctx)
        };
        let first = open(&mut app);
        assert!(first.len() > 2, "{first:?}");
        assert_eq!(app.canvas.hidden_chars.marks.len(), 1, "the marks are kept with the text object's node");
        app.session.execute("file.close", &json!({})).unwrap();
        assert!(hidden_char_marks(&mut app, &ctx).is_empty(), "the Home screen");
        assert!(app.canvas.hidden_chars.marks.is_empty(), "no node of the closed document is kept");
        assert_eq!(open(&mut app), first, "the same type in a new document");
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

    /// General › Scroll Sideways with Cmd/Ctrl + Wheel, through the control channel's `ui.wheel`:
    /// with Zoom with Mouse Wheel off, Cmd/Ctrl-wheel zooms about the pointer while the option is
    /// off and scrolls sideways while it is on, and a trackpad pinch, two fingers spread on a touch
    /// screen, Alt + Cmd/Ctrl-wheel and Alt-wheel still zoom. A Cmd/Ctrl-wheel notch with a plain
    /// one right after it scrolls two notches in all, and a trackpad gesture during which Cmd/Ctrl
    /// is pressed and released scrolls sideways until the gesture ends. With Zoom with Mouse Wheel
    /// also on, Cmd/Ctrl-wheel scrolls sideways and the wheel zooms.
    #[test]
    fn cmd_wheel_scrolls_sideways_preference() {
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
        assert!(zooms_in(turn(&mut app, "cmd")), "off: Cmd-wheel zooms in about the pointer");
        let (zoom, scroll, _) = turn(&mut app, "shift");
        assert!((zoom - 1.0).abs() < 1e-9 && scroll.x < -1.0 && scroll.y == 0.0, "off: Shift-wheel scrolls sideways ({zoom}, {scroll:?})");
        app.session.execute("prefs.set", &json!({"key": "ctrlWheelScrollsSideways", "value": true})).unwrap();
        // A notch scrolls one wheel line.
        let line = f64::from(ctx.options(|o| o.input_options.line_scroll_speed));
        let (zoom, scroll, _) = turn(&mut app, "cmd");
        assert!(
            (zoom - 1.0).abs() < 1e-9 && (scroll.x + line).abs() < 0.5 && scroll.y == 0.0,
            "on: Cmd-wheel scrolls sideways by a notch ({zoom}, {scroll:?})"
        );
        assert!(zooms_in(turn(&mut app, "pinch")), "on: a pinch right after it zooms in about the pointer");
        // Two fingers spread on a touch screen, with no wheel turn since the Cmd-wheel one.
        let before = app.view().unwrap().zoom;
        let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
        let fingers = |phase, d: f32| vec![touch(1, phase, at - vec2(40.0 + d, 0.0)), touch(2, phase, at + vec2(40.0 + d, 0.0))];
        frame(&mut app, &ctx, fingers(egui::TouchPhase::Start, 0.0));
        for k in 1..=3 {
            frame(&mut app, &ctx, fingers(egui::TouchPhase::Move, 10.0 * k as f32));
        }
        frame(&mut app, &ctx, fingers(egui::TouchPhase::End, 30.0));
        let zoom = app.view().unwrap().zoom / before;
        assert!(zoom > 1.01, "on: fingers spread on a touch screen zoom in ({zoom})");
        assert!(zooms_in(turn(&mut app, "alt+cmd")), "on: Alt + Cmd-wheel zooms in about the pointer");
        assert!(zooms_in(turn(&mut app, "alt")), "on: so does Alt-wheel");
        let (zoom, scroll, _) = turn(&mut app, "");
        assert!((zoom - 1.0).abs() < 1e-9 && scroll.x == 0.0 && scroll.y < -1.0, "on: the wheel scrolls up ({zoom}, {scroll:?})");
        let (zoom, scroll, _) = turn(&mut app, "shift");
        assert!((zoom - 1.0).abs() < 1e-9 && scroll.x < -1.0 && scroll.y == 0.0, "on: Shift-wheel scrolls sideways ({zoom}, {scroll:?})");
        // A Cmd-wheel notch with a plain one right after it, before egui has spread the first: they
        // scroll two notches in all, sideways and up.
        let before = *app.view().unwrap();
        crate::tests_synthetic::control(&mut app, &ctx, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 1, "cmd": true}));
        crate::tests_synthetic::control(&mut app, &ctx, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 1}));
        for _ in 0..40 {
            run(&mut app);
        }
        let after = *app.view().unwrap();
        let scroll = (after.center - before.center) * after.zoom;
        assert!(
            scroll.x < -1.0 && scroll.y < -1.0 && (scroll.x + scroll.y + 2.0 * line).abs() < 0.5,
            "on: a Cmd-wheel notch and a plain one right after it scroll two notches in all ({scroll:?})"
        );
        // A pinch while egui still spreads a Cmd-wheel notch zooms by the pinch alone, and the
        // canvas scrolls sideways by one notch; the pinch magnifies the part scrolled before it.
        let before = *app.view().unwrap();
        let p = Xf::new(rect, &before).to_doc(at);
        crate::tests_synthetic::control(&mut app, &ctx, "ui.wheel", json!({"x": at.x, "y": at.y, "dy": 1, "cmd": true}));
        app.synthetic.extend([egui::Event::PointerMoved(at), egui::Event::Zoom(1.25)]);
        for _ in 0..40 {
            run(&mut app);
        }
        let after = *app.view().unwrap();
        let (zoom, moved) = (after.zoom / before.zoom, Xf::new(rect, &after).to_screen(p) - at);
        let notch = line as f32;
        assert!(
            (zoom - 1.25).abs() < 1e-6 && moved.x > notch - 0.5 && moved.x < 1.25 * notch + 0.5 && moved.y.abs() < 1e-3,
            "on: a pinch during a Cmd-wheel notch zooms by the pinch, and the notch scrolls sideways ({zoom}, {moved:?})"
        );
        // A trackpad gesture starts with no modifier, and Cmd is pressed and released during it:
        // egui keeps Cmd in its wheel state until the gesture ends, and the canvas keeps scrolling
        // sideways until then.
        let before = *app.view().unwrap();
        let swipe = |phase, modifiers| egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, 6.0), phase, modifiers };
        app.synthetic.extend([
            egui::Event::PointerMoved(at),
            swipe(egui::TouchPhase::Start, egui::Modifiers::NONE),
            swipe(egui::TouchPhase::Move, egui::Modifiers::COMMAND),
            swipe(egui::TouchPhase::Move, egui::Modifiers::NONE),
            swipe(egui::TouchPhase::End, egui::Modifiers::NONE),
        ]);
        for _ in 0..40 {
            run(&mut app);
        }
        let after = *app.view().unwrap();
        let scroll = (after.center - before.center) * after.zoom;
        assert!(
            (after.zoom / before.zoom - 1.0).abs() < 1e-9 && (scroll.x + 12.0).abs() < 0.01 && scroll.y == 0.0,
            "on: a trackpad gesture scrolls sideways to its end after Cmd is let go ({}, {scroll:?})",
            after.zoom / before.zoom
        );
        app.session.execute("prefs.set", &json!({"key": "zoomWithMouseWheel", "value": true})).unwrap();
        let (zoom, scroll, _) = turn(&mut app, "cmd");
        assert!(
            (zoom - 1.0).abs() < 1e-9 && scroll.x < -1.0 && scroll.y == 0.0,
            "with Zoom with Mouse Wheel: Cmd-wheel scrolls sideways ({zoom}, {scroll:?})"
        );
        assert!(zooms_in(turn(&mut app, "")), "with Zoom with Mouse Wheel: the wheel zooms in about the pointer");
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

    /// A canvas frame at input time `t` (seconds) with `events`, for the trackpad rotation tests.
    fn timed_frame(app: &mut VectorcraftApp, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    }

    /// General › Trackpad Gesture to Rotate View, on by default: a two-finger rotation on a Mac
    /// trackpad (egui's `Event::Rotate`, clockwise radians) turns the view about the pointer, also
    /// in the frame of a pinch, which zooms about the same point. With the preference off, the view
    /// does not turn.
    #[test]
    fn a_trackpad_rotation_turns_the_view_about_the_pointer() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        timed_frame(&mut app, &ctx, 0.0, vec![]);
        let rect = app.canvas_rect.unwrap();
        let at = rect.center() + vec2(60.0, 40.0);
        timed_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
        let under = |app: &VectorcraftApp| Xf::new(rect, app.view().unwrap()).to_doc(at);
        let before = (*app.view().unwrap(), under(&app));
        timed_frame(&mut app, &ctx, 0.2, vec![egui::Event::Rotate(0.25)]);
        let after = *app.view().unwrap();
        assert!((after.rotation - before.0.rotation - 0.25f64.to_degrees()).abs() < 1e-4, "turned by 14.32°: {}", after.rotation);
        assert!((after.zoom - before.0.zoom).abs() < 1e-12, "no zoom");
        assert!(under(&app).distance(before.1) < 1e-6, "the point under the pointer stays: {:?}", under(&app));
        // A pinch and a rotation in one frame, in a new gesture.
        let before = (*app.view().unwrap(), under(&app));
        timed_frame(&mut app, &ctx, 1.0, vec![egui::Event::Zoom(1.25), egui::Event::Rotate(0.25)]);
        let after = *app.view().unwrap();
        assert!((after.zoom / before.0.zoom - 1.25).abs() < 1e-6, "the pinch zooms: {}", after.zoom / before.0.zoom);
        assert!((after.rotation - before.0.rotation - 0.25f64.to_degrees()).abs() < 1e-4, "and the rotation turns: {}", after.rotation);
        assert!(under(&app).distance(before.1) < 1e-6, "about the same point: {:?}", under(&app));
        app.session.execute("prefs.set", &json!({"key": "trackpadRotateView", "value": false})).unwrap();
        let before = *app.view().unwrap();
        timed_frame(&mut app, &ctx, 2.0, vec![egui::Event::Rotate(0.25)]);
        timed_frame(&mut app, &ctx, 2.1, vec![egui::Event::Rotate(0.25)]);
        let after = *app.view().unwrap();
        assert!(after.rotation == before.rotation && after.center == before.center, "off: the view does not turn ({})", after.rotation);
    }

    /// A trackpad rotation turns the view only once it reaches 5°, so a pinch that twists a little
    /// only zooms. It then turns the view by all of it and follows each later rotation event, until
    /// a quarter of a second passes without one.
    #[test]
    fn a_trackpad_rotation_turns_nothing_until_it_reaches_five_degrees() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        timed_frame(&mut app, &ctx, 0.0, vec![]);
        let at = app.canvas_rect.unwrap().center() + vec2(60.0, 40.0);
        timed_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
        let rotate = |deg: f32| egui::Event::Rotate(deg.to_radians());
        let zoom = app.view().unwrap().zoom;
        timed_frame(&mut app, &ctx, 1.0, vec![egui::Event::Zoom(1.25), rotate(1.0)]);
        assert!(app.view().unwrap().zoom > zoom * 1.2, "the pinch zooms");
        timed_frame(&mut app, &ctx, 1.1, vec![egui::Event::Zoom(1.25), rotate(1.0)]);
        timed_frame(&mut app, &ctx, 1.2, vec![egui::Event::Zoom(1.25), rotate(1.0)]);
        assert_eq!(app.view().unwrap().rotation, 0.0, "3° in all does not turn the view");
        timed_frame(&mut app, &ctx, 1.3, vec![rotate(3.0)]);
        assert!((app.view().unwrap().rotation - 6.0).abs() < 1e-4, "6° in all turns by 6°: {}", app.view().unwrap().rotation);
        timed_frame(&mut app, &ctx, 1.4, vec![rotate(-1.0)]);
        assert!((app.view().unwrap().rotation - 5.0).abs() < 1e-4, "then each event turns it: {}", app.view().unwrap().rotation);
        timed_frame(&mut app, &ctx, 2.0, vec![rotate(3.0)]);
        assert!((app.view().unwrap().rotation - 5.0).abs() < 1e-4, "a new rotation starts under 5° again: {}", app.view().unwrap().rotation);
    }

    /// Rotation events that come while two fingers are on a touch screen (Safari on an iPad sends
    /// them with the touches) do not turn the view.
    #[test]
    fn a_rotation_with_fingers_on_a_touch_screen_turns_nothing() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let ctx = egui::Context::default();
        timed_frame(&mut app, &ctx, 0.0, vec![]);
        let c = app.canvas_rect.unwrap().center();
        let touch = |n: u64, phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(n), phase, pos, force: None };
        timed_frame(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(c - vec2(40.0, 0.0))]);
        let before = app.view().unwrap().rotation;
        let start = vec![touch(1, egui::TouchPhase::Start, c - vec2(40.0, 0.0)), touch(2, egui::TouchPhase::Start, c + vec2(40.0, 0.0))];
        timed_frame(&mut app, &ctx, 0.2, [start, vec![egui::Event::Rotate(0.25)]].concat());
        assert_eq!(app.view().unwrap().rotation, before, "the view does not turn");
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

    /// Fit Artboard in Window fits the artboard as a rotated view shows it: at 45° every corner is
    /// on the canvas and one is within the 30-point margin of an edge. At 0° the zoom is unchanged.
    #[test]
    fn fit_artboard_fits_the_artboard_turned_with_the_view() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 612, "height": 792})).unwrap();
        let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0));
        app.canvas_rect = Some(rect);
        app.run("view.fitArtboard", json!({})).unwrap();
        assert!((app.view().unwrap().zoom - (940.0 / 612.0_f64).min(740.0 / 792.0)).abs() < 1e-12, "0°");
        app.run("view.rotate", json!({"angle": 45})).unwrap();
        app.run("view.fitArtboard", json!({})).unwrap();
        let board = app.session.active().unwrap().doc.artboards[0].rect;
        let q = Xf::new(rect, app.view().unwrap()).quad(board);
        assert!(q.iter().all(|c| rect.contains(*c)), "the turned artboard is on the canvas: {q:?}");
        let gap = q
            .iter()
            .map(|c| (c.x - rect.left()).min(rect.right() - c.x).min(c.y - rect.top()).min(rect.bottom() - c.y))
            .fold(f32::INFINITY, f32::min);
        assert!(gap < 31.0, "and fills it but for the margin: {gap}");
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

    /// With Animated Zoom off, the Zoom tool zooms a rotated view to the area dragged across: at 90°
    /// and 180° a 100 × 50-point marquee fills the canvas as it does at 0°, centered on the canvas.
    #[test]
    fn the_zoom_tool_zooms_a_rotated_view_to_the_area_dragged_across() {
        use egui::Event;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
        app.select_tool("zoom");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let rect = app.canvas_rect.unwrap();
        let at = rect.center() - vec2(100.0, 50.0);
        let to = at + vec2(100.0, 50.0);
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        for angle in [90.0, 180.0] {
            app.run("view.rotate", json!({ "angle": angle })).unwrap();
            let z0 = app.view().unwrap().zoom;
            let middle = Xf::new(rect, app.view().unwrap()).to_doc(at + vec2(50.0, 25.0));
            frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
            frame(&mut app, &ctx, vec![button(at, true)]);
            frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
            frame(&mut app, &ctx, vec![button(to, false)]);
            let v = app.view().unwrap();
            let want = z0 * f64::from((rect.width() / 100.0).min(rect.height() / 50.0));
            assert!((v.zoom - want).abs() < 1e-6, "{angle}°: {z0} → {}, not {want}", v.zoom);
            assert!(v.center.distance(middle) < 1e-6, "{angle}°: the marquee's center goes to the canvas's center");
            app.view_mut().unwrap().zoom = z0;
        }
    }

    /// In a rotated view the Selection tool's marquee is drawn where the tool selects: through the
    /// screen corners of the document rectangle between the press and the pointer.
    #[test]
    fn the_selection_marquee_is_drawn_where_it_selects_in_a_rotated_view() {
        use egui::Event;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        app.run("view.rotate", json!({"angle": 45})).unwrap();
        let rect = app.canvas_rect.unwrap();
        let (from, to) = (rect.center() - vec2(60.0, 20.0), rect.center() + vec2(60.0, 40.0));
        frame(&mut app, &ctx, vec![Event::PointerMoved(from)]);
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }],
        );
        frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
        let xf = Xf::new(rect, app.view().unwrap());
        let want = xf.quad(Rect::from_points(xf.to_doc(from), xf.to_doc(to)));
        let drawn: Vec<Vec<Pos2>> = shapes(&mut app, &ctx)
            .into_iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::from_white_alpha(230)) => Some(p.points),
                _ => None,
            })
            .collect();
        let [line] = &drawn[..] else { panic!("one marquee: {drawn:?}") };
        assert_eq!(line.len(), 5, "closed: {line:?}");
        assert!(line.iter().zip(&want).all(|(a, b)| a.distance(*b) < 0.5), "{line:?} is not {want:?}");
        assert_eq!(line[4], line[0]);
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
    fn space_shift_drag_rotates_the_view_whatever_the_tool() {
        use egui::{Event, Modifiers};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let c = app.canvas_rect.unwrap().center();
        let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
        let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        // `drag` drags from `from` to `to` with `held` down, and with Space as well when `with_space`.
        let drag = |app: &mut VectorcraftApp, held: Modifiers, with_space: bool, from: Pos2, to: Pos2| {
            let keys = |pressed| if with_space { vec![space(pressed, held)] } else { vec![] };
            frame(app, &ctx, [vec![Event::ModifiersChanged(held)], keys(true), vec![Event::PointerMoved(from)]].concat());
            frame(app, &ctx, vec![button(from, true, held)]);
            frame(app, &ctx, vec![Event::PointerMoved(to)]);
            frame(app, &ctx, vec![button(to, false, held)]);
            frame(app, &ctx, [keys(false), vec![Event::ModifiersChanged(Modifiers::NONE)]].concat());
        };
        // `at(deg)` is the point 100 points from the canvas center, `deg` degrees clockwise from the right.
        let at = |deg: f32| c + 100.0 * vec2(deg.to_radians().cos(), deg.to_radians().sin());
        let before = *app.view().unwrap();
        drag(&mut app, Modifiers::SHIFT, true, at(0.0), at(90.0));
        let after = *app.view().unwrap();
        assert!((after.rotation - 90.0).abs() < 1e-3, "{after:?}");
        assert_eq!((after.center, after.zoom), (before.center, before.zoom), "the view turns about its center");
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool drew nothing");
        // Shift does not round the angle: 90° and 20° more make 110°, not 105°.
        drag(&mut app, Modifiers::SHIFT, true, at(0.0), at(20.0));
        assert!((app.view().unwrap().rotation - 110.0).abs() < 1e-3, "{:?}", app.view());
        // Space alone still pans, and Cmd+Shift+Space still zooms.
        let v = *app.view().unwrap();
        drag(&mut app, Modifiers::NONE, true, c, c + vec2(30.0, 0.0));
        let panned = *app.view().unwrap();
        assert_eq!((panned.rotation, panned.zoom), (v.rotation, v.zoom));
        assert_ne!(panned.center, v.center, "Space pans");
        drag(&mut app, Modifiers::COMMAND | Modifiers::SHIFT, true, c, c);
        assert_eq!(app.view().unwrap().zoom, crate::state::next_zoom(v.zoom, true), "Cmd+Shift+Space click zooms in");
        assert_eq!(app.view().unwrap().rotation, v.rotation);
        // The Rotate View tool's Shift still rounds to 15°.
        app.select_tool("rotateView");
        app.view_mut().unwrap().rotation = 0.0;
        drag(&mut app, Modifiers::SHIFT, false, at(0.0), at(20.0));
        assert_eq!(app.view().unwrap().rotation, 15.0);
        drag(&mut app, Modifiers::NONE, false, at(0.0), at(20.0));
        assert!((app.view().unwrap().rotation - 35.0).abs() < 1e-3, "{:?}", app.view());
    }

    #[test]
    fn the_rotate_cursor_shows_for_space_shift_and_the_rotate_view_tool() {
        use egui::{CursorIcon, Event, Modifiers};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let c = app.canvas_rect.unwrap().center();
        // `shown` returns the system cursor the canvas sets over a window without the focus. On macOS
        // the window paints the glyph only with the focus, and on the other desktop builds (not
        // Wayland, which the tests do not set) the window sets the system cursor next to the bitmap.
        let shown = |app: &mut VectorcraftApp, events: Vec<Event>| {
            let screen_rect = Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect, focused: false, events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.platform_output.cursor_icon
        };
        let space = |pressed, modifiers| Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers };
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::SHIFT };
        assert_ne!(shown(&mut app, vec![Event::PointerMoved(c)]), CursorIcon::Alias);
        assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::SHIFT), space(true, Modifiers::SHIFT)]), CursorIcon::Alias);
        // Shift let go during the drag keeps the rotate cursor; after it, Space shows the hand.
        shown(&mut app, vec![button(c, true)]);
        assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(c + vec2(20.0, 30.0))]), CursorIcon::Alias);
        assert_eq!(shown(&mut app, vec![button(c + vec2(20.0, 30.0), false)]), CursorIcon::Grab);
        // Cmd+Shift+Space zooms.
        assert_eq!(shown(&mut app, vec![Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::SHIFT)]), CursorIcon::ZoomIn);
        assert_ne!(shown(&mut app, vec![space(false, Modifiers::NONE), Event::ModifiersChanged(Modifiers::NONE)]), CursorIcon::Alias);
        app.select_tool("rotateView");
        assert_eq!(shown(&mut app, vec![Event::PointerMoved(c)]), CursorIcon::Alias, "the Rotate View tool");
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

    /// #2119: type copied with an Alt-drag, double-clicked, all of its text selected and typed over
    /// keeps its font, size and colour: through an IME (the marked text cleared, then committed by
    /// Return, as Windows sends them) and with Backspace then typing. Whole app frames, one input
    /// event per frame.
    fn retype_all_of_a_copied_type(ime: bool) {
        use egui::{Event, Key, Modifiers};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let original = app
            .session
            .execute("text.create", &json!({"x": 60, "y": 100, "text": "Chair 123", "size": 36, "font": "Georgia", "color": "#808080"}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let time = std::cell::Cell::new(0.0);
        let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
            time.set(time.get() + 0.03);
            let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time.get()), events, ..Default::default() }, |ui| {
                app.logic(ui.ctx());
                app.ui(ui);
            });
            out.textures_delta.clear();
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let node = |app: &VectorcraftApp, id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let styles = |app: &VectorcraftApp, id: u64| match &node(app, id).kind {
            NodeKind::Text(t) => {
                t.runs.iter().map(|r| (r.text.clone(), r.style.font_family.clone(), r.style.size, r.style.fill.clone())).collect::<Vec<_>>()
            }
            _ => panic!("not text"),
        };
        let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        let key = |key, pressed, modifiers| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        // 1. Alt-drag the type: a copy, selected.
        let from = xf.to_screen(node(&app, original).geometric_bounds().unwrap().center());
        let to = from + vec2(0.0, 120.0 * xf.zoom as f32);
        frame(&mut app, vec![Event::PointerMoved(from), Event::ModifiersChanged(Modifiers::ALT)]);
        frame(&mut app, vec![button(from, true, Modifiers::ALT)]);
        for k in 1..=6 {
            frame(&mut app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 6.0))]);
        }
        frame(&mut app, vec![button(to, false, Modifiers::ALT)]);
        frame(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        let copy = app.session.active().unwrap().selection.objects[0].0;
        assert_ne!(copy, original);
        let style = styles(&app, original)[0].clone();
        // 2. Double-click it at once: the Type tool edits it.
        let at = xf.to_screen(node(&app, copy).geometric_bounds().unwrap().center());
        frame(&mut app, vec![Event::PointerMoved(at)]);
        for pressed in [true, false, true, false] {
            frame(&mut app, vec![button(at, pressed, Modifiers::NONE)]);
        }
        frame(&mut app, vec![]);
        assert_eq!(app.session.tool_options()["editing"], json!(copy));
        // 3. Select all of its text and type over it.
        frame(&mut app, vec![key(Key::A, true, Modifiers::COMMAND)]);
        frame(&mut app, vec![key(Key::A, false, Modifiers::COMMAND)]);
        if ime {
            frame(&mut app, vec![preedit("t", 1)]);
            frame(&mut app, vec![preedit("table", 5)]);
            let marked = styles(&app, copy);
            assert_eq!(marked.iter().map(|r| (&r.1, r.2, &r.3)).collect::<Vec<_>>(), vec![(&style.1, style.2, &style.3)], "composing: {marked:?}");
            // Return commits the composition.
            frame(&mut app, vec![preedit("", 0), Event::Ime(egui::ImeEvent::Commit("table".into()))]);
        } else {
            frame(&mut app, vec![key(Key::Backspace, true, Modifiers::NONE)]);
            frame(&mut app, vec![key(Key::Backspace, false, Modifiers::NONE)]);
            frame(&mut app, vec![Event::Text("table".into())]);
            frame(&mut app, vec![key(Key::Enter, true, Modifiers::NONE)]);
            frame(&mut app, vec![key(Key::Enter, false, Modifiers::NONE)]);
        }
        frame(&mut app, vec![]);
        let after = styles(&app, copy);
        assert!(after.iter().any(|r| r.0.starts_with("table")), "ime={ime}: typed over: {after:?}");
        assert!(
            after.iter().all(|r| (&r.1, r.2, &r.3) == (&style.1, style.2, &style.3)),
            "ime={ime}: the copy keeps {:?} {} {:?}, got {after:?}",
            style.1,
            style.2,
            style.3
        );
    }

    #[test]
    fn retyping_all_of_a_copied_type_through_an_ime_keeps_its_style() {
        retype_all_of_a_copied_type(true);
    }

    #[test]
    fn retyping_all_of_a_copied_type_after_backspace_keeps_its_style() {
        retype_all_of_a_copied_type(false);
    }

    #[test]
    fn double_clicking_type_with_the_direct_selection_tool_puts_the_caret_there() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Hello world", "size": 24})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        app.select_tool("directSelection");
        let ctx = egui::Context::default();
        let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
                crate::shortcuts::handle(app, ui.ctx());
                show(app, ui)
            });
            out.textures_delta.clear();
        };
        timed(&mut app, 0.0, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let text = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        };
        // Double-click the middle of the text with Direct Selection.
        let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
        let at = xf.to_screen(b.center());
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
        timed(&mut app, 1.1, vec![]);
        // The Type tool edits the text, with the caret inside it, where it was clicked: typing goes
        // into the text, not to tool shortcuts.
        assert_eq!(app.session.tool_id(), "type");
        assert!(app.session.tool_wants_text());
        let o = app.session.tool_options();
        assert_eq!(o["editing"], json!(id));
        let caret = o["caret"].as_u64().unwrap();
        assert!((1..11).contains(&caret), "caret {caret}");
        app.session.tool_text("X", app.view_info()).unwrap();
        let t = text(&app);
        assert_eq!((t.len(), t.find('X')), (12, Some(caret as usize)), "{t}");

        // Pressing Escape exits text editing and returns to Direct Selection tool.
        let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
        timed(&mut app, 1.2, vec![esc(true), esc(false)]);
        assert_eq!(app.session.tool_id(), "directSelection");
        assert!(!app.session.tool_wants_text());
    }

    #[test]
    fn escape_while_editing_in_isolation_returns_to_selection_tool_and_keeps_isolation_until_second_escape() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let tid =
            app.session.execute("text.create", &json!({"x": 100, "y": 150, "text": "Nested hello", "size": 24})).unwrap()["id"].as_u64().unwrap();
        // Group the text inside a group.
        app.session.execute("select.all", &json!({})).unwrap();
        app.session.execute("object.group", &json!({})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let timed = |app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| {
                crate::shortcuts::handle(app, ui.ctx());
                show(app, ui)
            });
            out.textures_delta.clear();
        };
        timed(&mut app, 0.0, vec![]);
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let b = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(tid)).unwrap().geometric_bounds().unwrap();
        let at = xf.to_screen(b.center());
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let esc = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };

        // 1st double-click on group enters isolation mode.
        timed(&mut app, 1.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
        timed(&mut app, 1.1, vec![]);
        assert!(app.session.active().unwrap().isolation.is_some(), "should be in isolation mode");
        assert_eq!(app.session.tool_id(), "selection");

        // 2nd double-click on the text inside the isolated group enters text editing.
        timed(&mut app, 2.0, vec![egui::Event::PointerMoved(at), button(true), button(false), button(true), button(false)]);
        timed(&mut app, 2.1, vec![]);
        assert_eq!(app.session.tool_id(), "type");
        assert!(app.session.tool_wants_text());
        assert!(app.session.active().unwrap().isolation.is_some(), "isolation mode still active while editing text");

        // 1st Escape exits text edit mode, returns to Selection tool, but keeps isolation mode.
        timed(&mut app, 3.0, vec![esc(true), esc(false)]);
        timed(&mut app, 3.1, vec![]);
        assert_eq!(app.session.tool_id(), "selection");
        assert!(!app.session.tool_wants_text());
        assert!(app.session.active().unwrap().isolation.is_some(), "1st Escape must NOT end isolation mode");

        // 2nd Escape exits isolation mode.
        timed(&mut app, 4.0, vec![esc(true), esc(false)]);
        timed(&mut app, 4.1, vec![]);
        assert_eq!(app.session.tool_id(), "selection");
        assert!(app.session.active().unwrap().isolation.is_none(), "2nd Escape must exit isolation mode");
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
        assert_eq!(st.doc.node(b).unwrap().extras().graphic_style, Some(g.id));
        assert_eq!(st.doc.node(b).unwrap().appearance, g.appearance);
        assert_eq!(st.doc.node(a).unwrap().extras().graphic_style, None);
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

    #[test]
    fn frame_moves_keeps_touch_samples_and_pressure_but_leaves_mouse_unchanged() {
        let ctx = egui::Context::default();
        let a = pos2(10.0, 20.0);
        let b = pos2(20.0, 30.0);
        let c = pos2(30.0, 40.0);
        let events = vec![egui::Event::PointerMoved(a), egui::Event::PointerMoved(b), egui::Event::PointerMoved(c)];
        let mut mouse = vec![];
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| mouse = frame_moves(ui));
        out.textures_delta.clear();
        assert!(mouse.is_empty());
        let events = [pen(egui::TouchPhase::Move, a, 0.2), pen(egui::TouchPhase::Move, b, 0.8), pen(egui::TouchPhase::Move, c, 0.6)].concat();
        let mut samples = vec![];
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| samples = frame_moves(ui));
        out.textures_delta.clear();
        assert_eq!(samples, [(a, Some(0.2)), (b, Some(0.8))], "the last sample follows the normal pointer path");
        let events = (0..200).flat_map(|n| pen(egui::TouchPhase::Move, pos2(n as f32, 0.0), 0.5)).collect();
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| samples = frame_moves(ui));
        out.textures_delta.clear();
        assert!(samples.len() <= 64, "a stalled frame stays bounded");
    }

    #[test]
    fn long_touch_drops_a_tool_press_without_committing_art() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width":800,"height":600})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        let tick = |app: &mut VectorcraftApp, time, events| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
        };
        tick(&mut app, 0.0, vec![]);
        let pos = app.canvas_rect.unwrap().center();
        tick(&mut app, 0.1, pen(egui::TouchPhase::Start, pos, 0.5));
        assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Tool)));
        tick(&mut app, 1.5, vec![]);
        assert!(matches!(ctx.data(|d| d.get_temp::<Drag>(drag_id())), Some(Drag::Gesture)), "long touch drops the tool drag");
        assert!(app.session.active().unwrap().interaction.is_none());
        tick(&mut app, 1.6, pen(egui::TouchPhase::End, pos, 0.5));
        assert!(app.session.active().unwrap().doc.art_bounds().is_none(), "releasing cannot commit the cancelled press");
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

    /// The Contextual Task Bar shows Recolor Artwork (a palette) when the selection has colours,
    /// and it opens the Recolor Artwork dialog; art painted with None has no button.
    #[test]
    fn the_task_bar_recolors_coloured_art() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap()["id"].clone();
        app.session.execute("paint.setFill", &json!({"ids": [id], "color": "#cc3322"})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let click = |app: &mut VectorcraftApp| {
            frame(app, &ctx, vec![]);
            frame(app, &ctx, vec![]);
            let bar = task_bar_rect(&ctx).expect("the task bar shows");
            // Right to left: the frame's margin, More Options, Lock, then Recolor.
            let at = pos2(bar.right() - 8.0 - 30.0 - 6.0 - 30.0 - 6.0 - 15.0, bar.center().y);
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(true)]);
            frame(app, &ctx, vec![button(false)]);
        };
        let st = app.session.active().unwrap();
        assert!(has_colors(&st.doc, &st.selection.objects));
        click(&mut app);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::recolor::KIND));

        app.ui.dialog = None;
        app.session.execute("paint.setFill", &json!({"ids": [id], "none": true})).unwrap();
        app.session.execute("paint.setStroke", &json!({"ids": [id], "none": true})).unwrap();
        let st = app.session.active().unwrap();
        assert!(!has_colors(&st.doc, &st.selection.objects), "None paints have no colours");
        click(&mut app);
        assert!(app.ui.dialog.is_none(), "no Recolor button without colours");
    }

    /// The Contextual Task Bar shows Image Trace first for a selected image: it traces the image
    /// with the Default preset and leaves the Image Trace panel as it was. For the Image Trace
    /// object it made, the first button is Expand, which keeps only the traced paths.
    #[test]
    fn the_task_bar_traces_an_image_then_expands_the_trace() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let data = vectorcraft_format::base64_encode(&png);
        app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let click_first = |app: &mut VectorcraftApp| {
            frame(app, &ctx, vec![]);
            frame(app, &ctx, vec![]);
            let bar = task_bar_rect(&ctx).expect("the task bar shows");
            // Left to right: the frame's margin, the handle and its spacing, then the first button.
            let at = pos2(bar.left() + 8.0 + 4.0 + 6.0 + 12.0, bar.center().y);
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(true)]);
            frame(app, &ctx, vec![button(false)]);
        };
        let first = |app: &VectorcraftApp| {
            let st = app.session.active().unwrap();
            st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned().unwrap()
        };
        assert!(matches!(first(&app).kind, NodeKind::Image(_)));
        click_first(&mut app);
        assert_eq!(first(&app).name.as_deref(), Some("Image Trace"), "the image was traced");
        assert_eq!(app.ui.open_panel, None, "the Image Trace panel is left as it was");

        click_first(&mut app);
        let expanded = first(&app);
        assert_ne!(expanded.name.as_deref(), Some("Image Trace"), "the trace was expanded");
        let mut images = 0;
        expanded.walk(&mut |n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
        assert_eq!(images, 0, "only the traced paths are kept");
    }

    /// The labels the Contextual Task Bar draws in a headless frame (after one to lay it out), with
    /// their rects; none while it isn't shown. Icons, which draw as ⚠ without an image loader, are
    /// left out.
    fn task_bar_labels(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect)> {
        frame(app, ctx, vec![]);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let Some(bar) = task_bar_rect(ctx) else { return vec![] };
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) if bar.contains(t.visual_bounding_rect().center()) && t.galley.text() != "⚠" => {
                    Some((t.galley.text().to_string(), t.visual_bounding_rect()))
                }
                _ => None,
            })
            .collect()
    }

    /// With the Artboard tool, the Contextual Task Bar sits under the selected artboards and offers
    /// Resize, Duplicate, Rearrange All (with two or more artboards) and Export instead of the
    /// selected objects' actions. With the Selection tool it offers the objects' actions again, and
    /// with nothing selected it doesn't show.
    #[test]
    fn the_task_bar_offers_the_artboard_tools_actions() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        assert!(names(&mut app).is_empty() && task_bar_rect(&ctx).is_none(), "nothing selected: no bar");

        app.select_tool("artboard");
        crate::panels::artboards::select(&mut app, 1);
        // Artboard 2 in the middle of the window, at 100%.
        let b = app.session.active().unwrap().doc.artboards[1].rect;
        let view = app.view_mut().unwrap();
        (view.center, view.zoom) = (b.center(), 1.0);
        assert_eq!(names(&mut app), ["Resize", "Duplicate", "Rearrange All", "Export"]);
        // Under artboard 2.
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let bar = task_bar_rect(&ctx).unwrap();
        let center = xf.to_screen(b.center());
        assert!(bar.top() > xf.to_screen(Point::new(b.x0, b.y1)).y && bar.x_range().contains(center.x), "{bar:?} under {b:?}");

        // A selected object with the Selection tool: its own actions, none of the artboards'.
        app.select_tool("selection");
        app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
        assert_eq!(names(&mut app), ["Offset Path", "Simplify", "Duplicate"]);

        // One artboard: nothing to rearrange.
        app.session.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        app.select_tool("artboard");
        assert_eq!(names(&mut app), ["Resize", "Duplicate", "Export"]);
    }

    /// The Artboard tool's Contextual Task Bar buttons act on its selected artboards: Resize opens
    /// Artboard Options for the active one, Rearrange All opens Rearrange All Artboards, Export opens
    /// Export for Screens with the selected artboards checked, and Duplicate copies each selected
    /// artboard in one undo step and selects the copies.
    #[test]
    fn the_artboard_tools_task_bar_buttons_act_on_the_selected_artboards() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        crate::panels::artboards::select(&mut app, 2);
        app.session.set_tool_option("selected", &json!([2, 0]));
        let ctx = egui::Context::default();
        let press = |app: &mut VectorcraftApp, label: &str| {
            let labels = task_bar_labels(app, &ctx);
            let at = labels.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {labels:?}")).1.center();
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(true)]);
            frame(app, &ctx, vec![button(false)]);
            app.ui.dialog.take()
        };
        let d = press(&mut app, "Resize").expect("Artboard Options");
        assert_eq!((d.kind.as_str(), d.f64("index", 9.0)), ("artboardOptions", 2.0));
        let d = press(&mut app, "Rearrange All").expect("Rearrange All Artboards");
        assert_eq!(d.kind, crate::dialogs::rearrange_artboards::KIND);
        let d = press(&mut app, "Export").expect("Export for Screens");
        assert_eq!((d.kind.as_str(), &d.fields["boards"]), (crate::dialogs::EXPORT_FOR_SCREENS, &json!([true, false, true])));

        let boards =
            |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.iter().map(|a| (a.name.clone(), a.rect.size())).collect::<Vec<_>>();
        let before = boards(&app);
        assert!(press(&mut app, "Duplicate").is_none());
        let after = boards(&app);
        assert_eq!(after[..3], before[..]);
        assert_eq!(after[3..], [("Artboard 1 copy".to_string(), before[0].1), ("Artboard 3 copy".to_string(), before[2].1)]);
        assert_eq!(app.session.tool_options()["selected"], json!([4, 3]), "the copies are selected, artboard 3's copy active");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app), before, "one undo step");
    }

    /// With anchors direct-selected, the Contextual Task Bar offers Remove Anchor Points, Connect,
    /// Cut Path, Corner and Smooth. Smooth and Corner convert the selected anchor, Cut Path opens
    /// the path there and Connect closes it again, as the Control bar's anchor buttons do.
    #[test]
    fn the_task_bar_connects_cuts_and_converts_direct_selected_anchors() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 80})).unwrap()["id"].as_u64().unwrap();
        app.select_tool("directSelection");
        app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
        let ctx = egui::Context::default();
        let names = |app: &mut VectorcraftApp| task_bar_labels(app, &ctx).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        assert_eq!(names(&mut app), ["Remove Anchor Points", "Connect", "Cut Path", "Corner", "Smooth", "Duplicate"]);
        let press = |app: &mut VectorcraftApp, label: &str| {
            let labels = task_bar_labels(app, &ctx);
            let at = labels.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {labels:?}")).1.center();
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(app, &ctx, vec![button(true)]);
            frame(app, &ctx, vec![button(false)]);
        };
        let subpath = |app: &VectorcraftApp| {
            app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().path_data().unwrap().subpaths[0].clone()
        };
        let last_undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().unwrap().label.clone();
        press(&mut app, "Smooth");
        let a = subpath(&app).anchors[1];
        assert!(a.has_in() && a.has_out(), "smooth: {a:?}");
        assert_eq!(last_undo(&app), "Convert Anchor Points");
        press(&mut app, "Corner");
        let a = subpath(&app).anchors[1];
        assert!(!a.has_in() && !a.has_out(), "corner: {a:?}");
        press(&mut app, "Cut Path");
        assert_eq!(last_undo(&app), "Cut Path");
        let sp = subpath(&app);
        assert!(!sp.closed && sp.anchors.len() == 5, "the rectangle opens at the anchor: {sp:?}");
        press(&mut app, "Connect");
        assert_eq!(last_undo(&app), "Join");
        let sp = subpath(&app);
        assert!(sp.closed && sp.anchors.len() == 4, "the ends join again: {sp:?}");
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

    /// In a view turned 180° the Contextual Task Bar shows under the selection's lowest corner on
    /// screen, not over the selection.
    #[test]
    fn the_task_bar_stays_under_the_selection_in_a_rotated_view() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        let id = app.session.execute("shape.rectangle", &json!({"x": 170, "y": 90, "width": 60, "height": 120})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        app.select_tool("selection");
        app.run("view.rotate", json!({"angle": 180})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        let bar = task_bar_rect(&ctx).expect("the task bar shows");
        let xf = Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
        let lowest = xf.quad(app.selection_bounds().unwrap()).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
        assert!(bar.top() > lowest, "the bar {bar:?} starts below {lowest}");
    }

    fn ruler_guide_origin_ui(alt: bool, cancel: bool) {
        use egui::{Event, Modifiers};
        for (origin, preview, release) in [
            (Point::new(17.0, 23.0), Point::new(222.0, 238.0), Point::new(232.0, 243.0)),
            (Point::new(-17.0, -23.0), Point::new(223.0, 237.0), Point::new(233.0, 242.0)),
            (Point::ZERO, Point::new(225.0, 235.0), Point::new(230.0, 245.0)),
        ] {
            for source_vertical in [false, true] {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
                app.run("document.setRulerOrigin", json!({"x": origin.x, "y": origin.y})).unwrap();
                app.run("shape.rectangle", json!({"x": 40, "y": 50, "width": 20, "height": 30})).unwrap();
                let redo = app.session.active().unwrap().doc.clone();
                app.run("edit.undo", json!({})).unwrap();
                app.session.active_mut().unwrap().mark_saved();
                app.ui.view.rulers = true;
                app.ui.view.smart_guides = false;
                app.ui.view.snap_to_point = false;
                app.ui.view.snap_to_grid = false;
                app.ui.view.snap_to_pixel = false;
                app.select_tool("rectangle");
                let ctx = egui::Context::default();
                frame(&mut app, &ctx, vec![]);
                let view = app.view_mut().unwrap();
                view.zoom = 1.0;
                view.rotation = 0.0;
                view.center = Point::new(200.0, 150.0);
                frame(&mut app, &ctx, vec![]);
                let rect = app.canvas_rect.unwrap();
                let xf = Xf::new(rect, app.view().unwrap());
                let from =
                    if source_vertical { pos2(rect.left() - RULER / 2.0, rect.center().y) } else { pos2(rect.center().x, rect.top() - RULER / 2.0) };
                let to = xf.to_screen(Point::new(223.0, 237.0));
                let end = xf.to_screen(Point::new(231.0, 244.0));
                assert!(rect.contains(to) && rect.contains(end));
                let mods = if alt { Modifiers::SHIFT | Modifiers::ALT } else { Modifiers::SHIFT };
                let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: mods };
                let before = app.session.active().unwrap().doc.clone();
                let selection = app.session.active().unwrap().selection.clone();
                let history = |app: &VectorcraftApp| {
                    let st = app.session.active().unwrap();
                    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
                };
                let original_history = history(&app);
                assert_eq!(original_history.1, 1);
                frame(&mut app, &ctx, vec![Event::ModifiersChanged(mods), Event::PointerMoved(from)]);
                frame(&mut app, &ctx, vec![button(from, true)]);
                frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
                let check = |app: &VectorcraftApp, switched: bool, expected: Point| {
                    let guides = &app.session.active().unwrap().doc.guides;
                    assert_eq!(guides.len(), 1);
                    let vertical = source_vertical != switched;
                    assert_eq!(guides[0].vertical, vertical);
                    let want = if vertical { expected.x } else { expected.y };
                    assert!((guides[0].pos - want).abs() < 1e-4, "origin={origin:?} vertical={vertical}: {:?}, want {want}", guides[0]);
                };
                assert_eq!(history(&app).0, original_history.0, "preview has no undo entry");
                if cancel {
                    // Cancellation is a baseline positive control, independent of snap correctness.
                    assert_eq!(app.session.active().unwrap().doc.guides.len(), 1);
                    frame(&mut app, &ctx, vec![Event::PointerMoved(from), button(from, false)]);
                    assert_eq!(app.session.active().unwrap().doc, before);
                    assert_eq!(app.session.active().unwrap().selection, selection);
                    assert_eq!(history(&app), original_history);
                    app.run("edit.redo", json!({})).unwrap();
                    assert_eq!(app.session.active().unwrap().doc, redo);
                    continue;
                }
                check(&app, alt, preview);
                if alt {
                    // Used only by the private #1782 compatibility regression, not standalone RED.
                    frame(&mut app, &ctx, vec![Event::ModifiersChanged(Modifiers::SHIFT)]);
                    check(&app, false, preview);
                    frame(&mut app, &ctx, vec![Event::ModifiersChanged(mods)]);
                    check(&app, true, preview);
                }
                frame(&mut app, &ctx, vec![Event::PointerMoved(end), button(end, false)]);
                check(&app, alt, release);
                let st = app.session.active().unwrap();
                assert_eq!(st.doc.ruler_origin_for(0), origin);
                assert_eq!(st.doc.layers, before.layers, "the rectangle tool draws no artwork");
                assert_eq!(st.selection, selection);
                assert_eq!(history(&app), (original_history.0 + 1, 0, true));
                assert_eq!(st.history.undo.last().unwrap().label, "New Guide");
                let after = st.doc.clone();
                app.run("edit.undo", json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc, before);
                assert_eq!(history(&app), (original_history.0, 1, false));
                app.run("edit.redo", json!({})).unwrap();
                assert_eq!(app.session.active().unwrap().doc, after);
            }
        }
    }

    #[test]
    fn ruler_guide_shift_origin_preview_release_and_history() {
        ruler_guide_origin_ui(false, false);
    }

    #[test]
    fn ruler_guide_shift_origin_cancel_preserves_redo() {
        ruler_guide_origin_ui(false, true);
    }

    #[test]
    fn ruler_guide_origin_display_fallback_accepts_only_bounded_finite_pairs() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        // The global zero point as stored (artboard rulers check their own, see `ruler_origin_for`).
        app.run("view.globalRulers", json!({"global": true})).unwrap();
        let max = vectorcraft_engine::MAX_COORD;
        for (origin, expected) in [
            (Point::ZERO, Point::ZERO),
            (Point::new(17.0, -23.0), Point::new(17.0, -23.0)),
            (Point::new(max, -max), Point::new(max, -max)),
            (Point::new(f64::NAN, 23.0), Point::ZERO),
            (Point::new(17.0, f64::INFINITY), Point::ZERO),
            (Point::new(max + 1.0, 23.0), Point::ZERO),
            (Point::new(17.0, -max - 1.0), Point::ZERO),
        ] {
            std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).ruler_origin = origin;
            assert_eq!(app.ruler_origin(), expected);
        }
    }

    #[test]
    fn ruler_guide_origin_tooltip_matches_selected_guide_fields() {
        use egui::Event;
        for (vertical, target, shown) in [(true, 222.0, 205.0), (false, 238.0, 215.0)] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.run("document.setRulerOrigin", json!({"x": 17, "y": 23})).unwrap();
            app.run("guide.add", json!({"vertical": vertical, "pos": 100})).unwrap();
            app.ui.view.guides = true;
            app.ui.view.smart_guides = false;
            app.ui.view.snap_to_point = false;
            app.ui.view.snap_to_grid = false;
            app.ui.view.snap_to_pixel = false;
            app.select_tool("selection");
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            let view = app.view_mut().unwrap();
            view.zoom = 1.0;
            view.rotation = 0.0;
            view.center = Point::new(200.0, 150.0);
            frame(&mut app, &ctx, vec![]);
            app.session.active_mut().unwrap().mark_saved();
            let before = app.session.active().unwrap().doc.clone();
            let undo = app.session.active().unwrap().history.undo.len();
            let rect = app.canvas_rect.unwrap();
            let xf = Xf::new(rect, app.view().unwrap());
            let from = xf.to_screen(if vertical { Point::new(100.0, 50.0) } else { Point::new(50.0, 100.0) });
            let to = xf.to_screen(if vertical { Point::new(target, 50.0) } else { Point::new(50.0, target) });
            assert!(rect.contains(from) && rect.contains(to));
            let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&mut app, &ctx, vec![Event::PointerMoved(from)]);
            frame(&mut app, &ctx, vec![button(from, true)]);
            frame(&mut app, &ctx, vec![Event::PointerMoved(to)]);
            assert!((app.session.active().unwrap().doc.guides[0].pos - target).abs() < 1e-4);
            assert_eq!(app.session.active().unwrap().selection.guides, vec![0]);
            // Observe the real selected-guide field; no expected strings are inserted into UI state.
            let fields = egui::Context::default();
            crate::theme::install_fonts(&fields);
            let mut texts = vec![];
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))), ..Default::default() };
                let mut out = fields.run_ui(input, |ui| crate::panels::transform::show(&mut app, ui));
                out.textures_delta.clear();
                fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
                        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, texts)),
                        _ => {}
                    }
                }
                texts.clear();
                out.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
            }
            assert!(texts.iter().any(|t| t == &Unit::Points.format(shown)), "{texts:?}");
            let view = app.view_info();
            let overlays = app.session.overlays(view);
            let want = format!("{}: {shown:.2} pt", if vertical { "X" } else { "Y" });
            assert!(
                overlays.iter().any(|o| matches!(o, vectorcraft_tools::Overlay::Measure { text, .. } if text == &want)),
                "{overlays:?}, field={shown}"
            );
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
            frame(&mut app, &ctx, vec![button(to, false)]);
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
            assert!((app.session.active().unwrap().doc.guides[0].pos - target).abs() < 1e-4, "stored coordinate remains absolute");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, before);
        }
    }

    /// The 800 × 600 window [`frame`] draws the canvas in.
    const WINDOW: egui::Rect = egui::Rect { min: Pos2::ZERO, max: Pos2 { x: 800.0, y: 600.0 } };

    /// A 400 × 300 pt document at 400%, turned `rotation` degrees, with the Rectangle tool: a press
    /// that reached the tool would draw.
    fn zoomed_in(rotation: f64) -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.select_tool("rectangle");
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let v = app.view_mut().unwrap();
        v.zoom = 4.0;
        v.rotation = rotation;
        frame(&mut app, &ctx, vec![]);
        (app, ctx)
    }

    /// Where the document's point `p` is on screen.
    fn on_screen(app: &VectorcraftApp, p: Point) -> Pos2 {
        Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(p)
    }

    /// Scroll bar `k` (0 horizontal, 1 vertical): its track, its span and the thumb's centre.
    fn scroll_bar(app: &mut VectorcraftApp, k: usize) -> (egui::Rect, Span, Pos2) {
        let rect = app.canvas_rect.unwrap();
        let track = scroll_tracks(rect, WINDOW)[k];
        let span = scroll_spans(&Xf::new(rect, app.view().unwrap()), app.content_bounds())[k];
        let len = if k == 0 { track.width() } else { track.height() };
        let (at, size) = span.thumb(len);
        let c = if k == 0 { pos2(track.left() + at + size / 2.0, track.center().y) } else { pos2(track.center().x, track.top() + at + size / 2.0) };
        (track, span, c)
    }

    fn primary(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// #2322: the Normal and Full Screen with Menu Bar modes keep the scroll bars along the right
    /// and bottom edges, beside the rulers; Full Screen and Presentation Mode give the canvas the
    /// whole window.
    #[test]
    fn scroll_bars_take_the_right_and_bottom_edges_in_the_windowed_screen_modes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        app.ui.view.rulers = true;
        let ctx = egui::Context::default();
        for (mode, max) in
            [(0, WINDOW.max - vec2(SCROLL_BAR, SCROLL_BAR)), (1, WINDOW.max - vec2(SCROLL_BAR, SCROLL_BAR)), (2, WINDOW.max), (3, WINDOW.max)]
        {
            app.ui.screen_mode = mode;
            frame(&mut app, &ctx, vec![]);
            assert_eq!(app.canvas_rect.unwrap().max, max, "screen mode {mode}");
        }
    }

    /// #2322: the thumb is as long, against its track, as the view against the art and the
    /// artboards with half a window around them; zoomed in it is shorter.
    #[test]
    fn a_scroll_bar_thumb_is_as_long_as_the_view_is_against_the_content() {
        assert_eq!(Span { lo: 0.0, hi: 100.0, v0: 0.0, v1: 100.0 }.thumb(50.0), (0.0, 50.0), "all in view: the whole track");
        assert_eq!(Span { lo: 0.0, hi: 400.0, v0: 150.0, v1: 250.0 }.thumb(100.0), (37.5, 25.0));
        assert_eq!(Span { lo: 0.0, hi: 1000.0, v0: 450.0, v1: 550.0 }.thumb(100.0), (40.0, 20.0), "never shorter than MIN_THUMB");
        let (mut app, ctx) = zoomed_in(0.0);
        let len = |app: &mut VectorcraftApp| {
            let (track, span, _) = scroll_bar(app, 0);
            span.thumb(track.width()).1
        };
        let at_400 = len(&mut app);
        app.view_mut().unwrap().zoom = 8.0;
        frame(&mut app, &ctx, vec![]);
        assert!(len(&mut app) < at_400, "zoomed in, the thumb is shorter");
    }

    /// #2322: dragging a thumb scrolls the view along the screen's axis, in a turned view too, by
    /// as much as the thumb's travel stands for; the drag never reaches the tool under the bar.
    #[test]
    fn dragging_a_scroll_bar_thumb_scrolls_the_view_and_draws_nothing() {
        for rotation in [0.0, 90.0, 30.0] {
            for k in 0..2 {
                let (mut app, ctx) = zoomed_in(rotation);
                let p = Point::new(200.0, 150.0);
                let before = on_screen(&app, p);
                let (track, span, from) = scroll_bar(&mut app, k);
                let len = if k == 0 { track.width() } else { track.height() };
                let step = if k == 0 { vec2(40.0, 0.0) } else { vec2(0.0, 40.0) };
                let moved = (40.0 * span.ratio(len)).clamp(span.lo - span.v0, span.hi - span.v1);
                assert!(moved > 40.0, "at 400% the view moves further than the thumb: {moved}");
                frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from)]);
                frame(&mut app, &ctx, vec![primary(from, true)]);
                frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + step / 2.0)]);
                frame(&mut app, &ctx, vec![egui::Event::PointerMoved(from + step)]);
                frame(&mut app, &ctx, vec![primary(from + step, false)]);
                frame(&mut app, &ctx, vec![]);
                assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool under the bar drew nothing");
                let after = on_screen(&app, p);
                let want = before - step.normalized() * moved;
                assert!((after - want).length() < 0.5, "rotation {rotation}, bar {k}: {before:?} -> {after:?}, want {want:?}");
            }
        }
    }

    /// #2322: a click on the track beyond the thumb scrolls a page (most of the window) towards it.
    #[test]
    fn a_click_on_a_scroll_bar_track_scrolls_a_page() {
        let (mut app, ctx) = zoomed_in(0.0);
        let p = Point::new(200.0, 150.0);
        let before = on_screen(&app, p);
        let (track, _, _) = scroll_bar(&mut app, 0);
        let page = 0.9 * app.canvas_rect.unwrap().width();
        let at = pos2(track.right() - 3.0, track.center().y);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, &ctx, vec![primary(at, true)]);
        frame(&mut app, &ctx, vec![primary(at, false)]);
        frame(&mut app, &ctx, vec![]);
        let after = on_screen(&app, p);
        assert!((after.x - (before.x - page)).abs() < 0.5 && (after.y - before.y).abs() < 0.5, "{before:?} -> {after:?}, page {page}");
        assert_eq!(app.session.active().unwrap().doc.art_bounds(), None, "the Rectangle tool drew nothing");
    }

    /// #2322: the wheel over the horizontal bar scrolls sideways, over the vertical one up and
    /// down (over the canvas it scrolls up and down).
    #[test]
    fn the_wheel_over_a_scroll_bar_scrolls_along_it() {
        for k in 0..2 {
            let (mut app, ctx) = zoomed_in(0.0);
            let p = Point::new(200.0, 150.0);
            let before = on_screen(&app, p);
            let (track, _, _) = scroll_bar(&mut app, k);
            let at = track.center();
            let wheel = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -30.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            };
            frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
            frame(&mut app, &ctx, vec![wheel]);
            for _ in 0..30 {
                frame(&mut app, &ctx, vec![]);
            }
            let after = on_screen(&app, p);
            let want = if k == 0 { before - vec2(30.0, 0.0) } else { before - vec2(0.0, 30.0) };
            assert!((after - want).length() < 0.5, "bar {k}: {before:?} -> {after:?}, want {want:?}");
        }
    }
}
