//! Split View draws and edits one document through the real canvas entry point.
// Integration assertions may panic. No graphics device or host window is opened.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use egui::{Color32, ColorImage, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, TextureId, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::cms;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;
use vectorcraft_geom::{Point, Rect as DocRect};
use vectorcraft_ui_egui::{
    VectorcraftApp, canvas, i18n,
    state::{SplitMode, SplitOrientation},
    theme,
};

static GLOBALS: Mutex<()> = Mutex::new(());
const MAIN: &str = "canvas-split-main";
const COMPARISON: &str = "canvas-split-comparison";

struct Globals {
    language: i18n::Lang,
    colors: cms::ColorSettings,
    proof: vectorcraft_render::proof::ProofView,
    _lock: MutexGuard<'static, ()>,
}

impl Globals {
    fn new() -> Self {
        let lock = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        assert!(std::env::var_os("VECTORCRAFT_SYNC_RENDER").is_some(), "run this actual canvas target with VECTORCRAFT_SYNC_RENDER=1");
        let language = i18n::current();
        let colors = cms::active_settings();
        let proof = vectorcraft_render::proof::view();
        i18n::set_current(i18n::Lang::EN);
        cms::set_active(&cms::ColorSettings { rgb: cms::SRGB.into(), ..Default::default() }).unwrap();
        vectorcraft_render::proof::set_view(Default::default());
        Self { language, colors, proof, _lock: lock }
    }
}

impl Drop for Globals {
    fn drop(&mut self) {
        cms::set_active(&self.colors).unwrap();
        vectorcraft_render::proof::set_view(self.proof.clone());
        i18n::set_current(self.language);
    }
}

fn key(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
}

fn meshes(shape: &Shape, clip: Rect, out: &mut Vec<(Rect, egui::epaint::Mesh)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                meshes(shape, clip, out);
            }
        }
        Shape::Mesh(mesh) => out.push((clip, (**mesh).clone())),
        _ => {}
    }
}

#[derive(Clone)]
struct PaintedGeometry {
    kind: &'static str,
    clip: Rect,
    bounds: Rect,
    points: Vec<Pos2>,
    fill: Color32,
    stroke: egui::Stroke,
}

impl PaintedGeometry {
    fn visible(&self) -> bool {
        self.fill.a() > 0 || (self.stroke.width > 0.0 && self.stroke.color.a() > 0)
    }

    fn json(&self) -> Value {
        json!({"kind":self.kind,"clip":[self.clip.left(),self.clip.top(),self.clip.right(),self.clip.bottom()],
            "bounds":[self.bounds.left(),self.bounds.top(),self.bounds.right(),self.bounds.bottom()],
            "points":self.points.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>(),
            "fill":self.fill.to_array(),"stroke":{"width":self.stroke.width,"color":self.stroke.color.to_array()}})
    }
}

fn painted_geometry(shape: &Shape, clip: Rect, out: &mut Vec<PaintedGeometry>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                painted_geometry(shape, clip, out);
            }
        }
        Shape::LineSegment { points, stroke } => out.push(PaintedGeometry {
            kind: "line",
            clip,
            bounds: Rect::from_two_pos(points[0], points[1]),
            points: points.to_vec(),
            fill: Color32::TRANSPARENT,
            stroke: *stroke,
        }),
        Shape::Rect(rect) => {
            out.push(PaintedGeometry { kind: "rect", clip, bounds: rect.rect, points: vec![], fill: rect.fill, stroke: rect.stroke })
        }
        Shape::Circle(circle) => out.push(PaintedGeometry {
            kind: "circle",
            clip,
            bounds: Rect::from_center_size(circle.center, vec2(circle.radius * 2.0, circle.radius * 2.0)),
            points: vec![circle.center],
            fill: circle.fill,
            stroke: circle.stroke,
        }),
        _ => {}
    }
}

/// A CPU mirror of exactly the uploads emitted by egui, including partial updates and frees.
#[derive(Default)]
struct Uploads {
    names: HashMap<TextureId, String>,
    images: HashMap<TextureId, ColorImage>,
    art_sets: Vec<TextureId>,
    art_frees: Vec<TextureId>,
}

impl Uploads {
    fn apply(&mut self, ctx: &egui::Context, delta: &egui::TexturesDelta) {
        for (id, meta) in ctx.tex_manager().read().allocated() {
            self.names.insert(*id, meta.name.clone());
        }
        self.art_sets.clear();
        self.art_frees.clear();
        for (id, changes) in &delta.set {
            if self.art(*id) {
                self.art_sets.push(*id);
            }
            for change in changes {
                let egui::ImageData::Color(image) = &change.image;
                if let Some([x, y]) = change.pos {
                    let target = self.images.get_mut(id).expect("a partial upload follows an actual full upload");
                    assert!(x + image.size[0] <= target.size[0] && y + image.size[1] <= target.size[1]);
                    for row in 0..image.size[1] {
                        let start = (y + row) * target.size[0] + x;
                        target.pixels[start..start + image.size[0]].copy_from_slice(&image.pixels[row * image.size[0]..(row + 1) * image.size[0]]);
                    }
                } else {
                    self.images.insert(*id, (**image).clone());
                }
            }
        }
        for id in &delta.free {
            if self.art(*id) {
                self.art_frees.push(*id);
            }
            self.images.remove(id);
        }
    }

    fn art(&self, id: TextureId) -> bool {
        self.names.get(&id).is_some_and(|name| name == MAIN || name == COMPARISON)
    }

    fn id(&self, ctx: &egui::Context, name: &str) -> Option<TextureId> {
        ctx.tex_manager().read().allocated().find_map(|(id, meta)| (meta.name == name).then_some(*id))
    }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    output: egui::FullOutput,
    uploaded: Uploads,
    size: egui::Vec2,
    time: f64,
    rectangle: NodeId,
}

impl Scene {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":320,"height":240})).unwrap();
        let rectangle = NodeId(app.run("shape.rectangle", json!({"x":60,"y":60,"width":200,"height":120})).unwrap()["id"].as_u64().unwrap());
        app.run("paint.setFill", json!({"color":"#d83020"})).unwrap();
        app.run("paint.setStroke", json!({"none":true})).unwrap();
        let curve = app.run("path.create", json!({"d":"M70 185 C100 145 190 245 230 185 L230 230 L70 230 Z"})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"color":"#208040"})).unwrap();
        let mask = app.run("shape.rectangle", json!({"x":90,"y":180,"width":100,"height":45})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids":[curve,mask]})).unwrap();
        app.run("object.clippingMask.make", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x":330,"y":85,"width":20,"height":40})).unwrap();
        app.run("paint.setFill", json!({"color":"#2040d0"})).unwrap();
        app.run("shape.rectangle", json!({"x":20,"y":20,"width":20,"height":20})).unwrap();
        app.run("paint.setFill", json!({"keepModel":true,"color":{"c":1,"m":0,"y":0,"k":0}})).unwrap();
        app.run("shape.rectangle", json!({"x":25,"y":25,"width":10,"height":10})).unwrap();
        app.run("paint.setFill", json!({"keepModel":true,"color":{"c":0,"m":1,"y":0,"k":0}})).unwrap();
        app.run("object.setOverprint", json!({"fill":true})).unwrap();
        app.run("select.set", json!({"ids":[]})).unwrap();
        app.select_tool("selection");
        app.ui.task_bar = false;
        app.ui.view.rulers = false;
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_grid = false;
        app.ui.view.snap_to_pixel = false;
        app.ui.view.snap_to_point = false;
        let v = app.view_mut().unwrap();
        v.fitted = true;
        v.center = Point::new(160.0, 120.0);
        v.zoom = 1.5;
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        Self { app, ctx, output: Default::default(), uploaded: Default::default(), size: vec2(640.0, 480.0), time: 0.0, rectangle }
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 60.0;
        let mut raw = RawInput {
            events,
            time: Some(self.time),
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            ..Default::default()
        };
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut output = self.ctx.run_ui(raw, |ui| {
            // App::ui dispatches this actual frontend key handler before painting its canvas.
            vectorcraft_ui_egui::shortcuts::handle(&mut self.app, ui.ctx());
            canvas::show(&mut self.app, ui);
        });
        self.uploaded.apply(&self.ctx, &output.textures_delta);
        output.textures_delta.clear();
        self.output = output;
    }

    fn idle(&mut self) {
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }
    fn command(&mut self, id: &str) {
        self.app.run(id, json!({})).unwrap();
        self.idle();
    }
    fn enable(&mut self) {
        self.command("view.splitView");
        assert!(self.app.view().unwrap().split.enabled);
    }
    fn rect(&self) -> Rect {
        self.app.canvas_rect.unwrap()
    }
    fn xf(&self) -> canvas::Xf {
        canvas::Xf::at(self.rect(), self.app.view().unwrap(), self.ctx.zoom_factor())
    }
    fn screen(&self, p: Point) -> Pos2 {
        self.xf().to_screen(p)
    }
    fn fraction(&self) -> f32 {
        self.app.view().unwrap().split.ratio()
    }

    fn point_on_divider(&self) -> Pos2 {
        let rect = self.rect();
        match self.app.view().unwrap().split.orientation {
            SplitOrientation::Vertical => pos2(rect.left() + rect.width() * self.fraction(), rect.center().y),
            SplitOrientation::Horizontal => pos2(rect.center().x, rect.top() + rect.height() * self.fraction()),
        }
    }

    fn press(&mut self, p: Pos2, button: PointerButton) {
        self.frame(vec![Event::PointerMoved(p)]);
        self.frame(vec![Event::PointerButton { pos: p, button, pressed: true, modifiers: Modifiers::NONE }]);
    }
    fn release(&mut self, p: Pos2, button: PointerButton) {
        self.frame(vec![Event::PointerButton { pos: p, button, pressed: false, modifiers: Modifiers::NONE }]);
        self.idle();
    }
    fn click(&mut self, p: Pos2) {
        self.press(p, PointerButton::Primary);
        self.release(p, PointerButton::Primary);
    }
    fn escape(&mut self) {
        self.frame(vec![key(Key::Escape, true)]);
        self.frame(vec![key(Key::Escape, false)]);
    }

    fn snapshot(&self) -> Value {
        let s = self.app.session.active().unwrap();
        json!({"doc":s.doc.as_ref(),"selection":s.selection,
            "undo":s.history.undo.iter().map(|e| json!({"label":e.label,"doc":e.doc.as_ref(),"selection":e.selection.as_ref()})).collect::<Vec<_>>(),
            "redo":s.history.redo.iter().map(|e| json!({"label":e.label,"doc":e.doc.as_ref(),"selection":e.selection.as_ref()})).collect::<Vec<_>>(),"step":s.history.step()})
    }
    fn document(&self) -> Value {
        json!(self.app.session.active().unwrap().doc.as_ref())
    }
    fn bounds(&self) -> DocRect {
        self.app.session.active().unwrap().doc.bounds_of(&[self.rectangle], false).unwrap()
    }
    fn pair(&self) -> [TextureId; 2] {
        [
            self.uploaded.id(&self.ctx, MAIN).expect("actual main artwork texture"),
            self.uploaded.id(&self.ctx, COMPARISON).expect("actual comparison artwork texture"),
        ]
    }
    fn image(&self, id: TextureId) -> &ColorImage {
        self.uploaded.images.get(&id).expect("canvas texture received its real pixels")
    }

    fn raster_at(&self, id: TextureId, p: Point) -> Color32 {
        let image = self.image(id);
        let at = self.screen(p);
        let rect = self.rect();
        let x = (((at.x - rect.left()) / rect.width()) * image.size[0] as f32).floor() as usize;
        let y = (((at.y - rect.top()) / rect.height()) * image.size[1] as f32).floor() as usize;
        image.pixels[y * image.size[0] + x]
    }

    fn assert_complementary_meshes(&self) {
        let mut drawn = vec![];
        for clipped in &self.output.shapes {
            meshes(&clipped.shape, clipped.clip_rect, &mut drawn);
        }
        let rect = self.rect();
        let divider = self.point_on_divider();
        for (side, id) in self.pair().into_iter().enumerate() {
            let matching: Vec<_> = drawn.iter().filter(|(_, mesh)| mesh.texture_id == id).collect();
            assert!(!matching.is_empty(), "both actual textures are painted");
            for (clip, mesh) in matching {
                let mut expected = rect;
                match (self.app.view().unwrap().split.orientation, side) {
                    (SplitOrientation::Vertical, 0) => expected.max.x = divider.x,
                    (SplitOrientation::Vertical, _) => expected.min.x = divider.x,
                    (SplitOrientation::Horizontal, 0) => expected.max.y = divider.y,
                    (SplitOrientation::Horizontal, _) => expected.min.y = divider.y,
                }
                assert!(
                    (clip.min - expected.min).length() < 0.1 && (clip.max - expected.max).length() < 0.1,
                    "complementary actual side clips: {clip:?} vs {expected:?}"
                );
                assert!(!mesh.indices.is_empty());
                for vertex in &mesh.vertices {
                    let expected = pos2((vertex.pos.x - rect.left()) / rect.width(), (vertex.pos.y - rect.top()) / rect.height());
                    assert!((vertex.uv - expected).length() < 0.002, "wipe crops one shared full-viewport image, not two resized panes");
                }
            }
        }
        let _ = self.divider_geometry();
    }

    fn painted_geometry(&self) -> Vec<PaintedGeometry> {
        let mut all = vec![];
        for clipped in &self.output.shapes {
            painted_geometry(&clipped.shape, clipped.clip_rect, &mut all);
        }
        all
    }

    fn divider_geometry(&self) -> (Vec<PaintedGeometry>, Vec<PaintedGeometry>) {
        let rect = self.rect();
        let at = self.point_on_divider();
        let vertical = self.app.view().unwrap().split.orientation == SplitOrientation::Vertical;
        let epsilon = 1.0 / self.ctx.pixels_per_point();
        let all = self.painted_geometry();
        let line = |g: &&PaintedGeometry| {
            let b = g.bounds;
            let (a, z, cross, length, extent) = if vertical {
                (pos2(at.x, rect.top()), pos2(at.x, rect.bottom()), (b.center().x - at.x).abs(), b.height(), rect.height())
            } else {
                (pos2(rect.left(), at.y), pos2(rect.right(), at.y), (b.center().y - at.y).abs(), b.width(), rect.width())
            };
            let thickness = if vertical { b.width() } else { b.height() };
            g.visible()
                && g.kind != "circle"
                && cross <= epsilon
                && length >= extent - epsilon
                && thickness < rect.width().min(rect.height()) / 20.0
                && g.clip.expand(epsilon).contains(a)
                && g.clip.expand(epsilon).contains(z)
        };
        let grip = |g: &&PaintedGeometry| {
            let b = g.bounds;
            let shown = b.intersect(g.clip);
            g.visible()
                && g.kind != "line"
                && (b.center() - at).length() <= epsilon
                && b.width() > epsilon
                && b.height() > epsilon
                && b.width() < rect.width() / 4.0
                && b.height() < rect.height() / 4.0
                && shown.width() > epsilon
                && shown.height() > epsilon
        };
        let lines: Vec<_> = all.iter().filter(line).cloned().collect();
        let grips: Vec<_> = all.iter().filter(grip).cloned().collect();
        assert!(!lines.is_empty(), "the actual output paints a visible full-viewport divider at this fraction and orientation");
        assert!(!grips.is_empty(), "the actual output paints a small visible grip centered on the divider");
        (lines, grips)
    }

    fn assert_no_art_uploads(&self, ids: [TextureId; 2]) {
        assert_eq!(self.pair(), ids, "divider changes keep the same real artwork texture handles");
        assert!(self.uploaded.art_sets.is_empty() && self.uploaded.art_frees.is_empty(), "divider changes upload/free no artwork");
    }

    fn save_uploaded_pair(&self) {
        let Some(root) = std::env::var_os("VECTORCRAFT_SPLIT_CANVAS_EVIDENCE") else { return };
        let root = std::path::PathBuf::from(root);
        assert!(root.is_absolute(), "explicit evidence directory is absolute");
        std::fs::create_dir_all(&root).unwrap();
        for (name, id) in [MAIN, COMPARISON].into_iter().zip(self.pair()) {
            let img = self.image(id);
            let pixels = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let rendered = vectorcraft_render::Rendered { width: img.size[0] as u32, height: img.size[1] as u32, pixels };
            let bytes = rendered.to_png().unwrap();
            use std::io::Write as _;
            std::fs::OpenOptions::new().write(true).create_new(true).open(root.join(format!("{name}.png"))).unwrap().write_all(&bytes).unwrap();
        }
        let mut all = vec![];
        for clipped in &self.output.shapes {
            meshes(&clipped.shape, clipped.clip_rect, &mut all);
        }
        let art: Vec<_> = all
            .into_iter()
            .filter(|(_, m)| self.uploaded.art(m.texture_id))
            .map(|(clip, m)| {
                json!({
            "texture":format!("{:?}",m.texture_id),"clip":[clip.left(),clip.top(),clip.right(),clip.bottom()],
            "vertices":m.vertices.iter().map(|v|json!({"position":[v.pos.x,v.pos.y],"uv":[v.uv.x,v.uv.y]})).collect::<Vec<_>>() })
            })
            .collect();
        let (divider_lines, divider_grips) = self.divider_geometry();
        let divider_lines: Vec<_> = divider_lines.iter().map(PaintedGeometry::json).collect();
        let divider_grips: Vec<_> = divider_grips.iter().map(PaintedGeometry::json).collect();
        let non_raster_geometry: Vec<_> = self.painted_geometry().iter().map(PaintedGeometry::json).collect();
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("split-canvas-geometry.json"))
            .unwrap()
            .write_all(
                serde_json::to_string_pretty(
                    &json!({"ceiling":"actual uploaded artwork and emitted geometry; not a composed GUI screenshot","meshes":art,
                        "divider_lines":divider_lines,"divider_grips":divider_grips,"non_raster_geometry":non_raster_geometry}),
                )
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
    }
}

#[test]
fn split_canvas_real_modes_share_geometry_and_complementary_clips() {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.idle();
    assert!(!s.app.view().unwrap().split.enabled);
    assert!(s.app.canvas.texture.is_some(), "default-off uses the ordinary actual canvas raster");
    assert!(s.uploaded.id(&s.ctx, MAIN).is_none() && s.uploaded.id(&s.ctx, COMPARISON).is_none());
    let before = s.snapshot();
    s.enable();
    let [main, comparison] = s.pair();
    assert_ne!(main, comparison);
    let p = Point::new(110.0, 100.0);
    let filled = s.raster_at(main, p);
    let outlined = s.raster_at(comparison, p);
    assert!(filled.r() > filled.g().saturating_add(60) && filled.a() > 200, "Preview contains the original coloured fill");
    assert!(outlined.a() < 40 || outlined.r().abs_diff(outlined.g()) < 12, "Outline does not rasterize the coloured interior");
    // This painted curved shape is clipped by an actual public clipping-mask group.
    assert!(s.raster_at(main, Point::new(120.0, 215.0)).a() > 200);
    assert!(s.raster_at(main, Point::new(215.0, 215.0)).a() < 40);
    s.assert_complementary_meshes();
    s.save_uploaded_pair();
    let ids = s.pair();
    s.app.view_mut().unwrap().split.fraction = 0.3;
    s.frame(vec![]);
    s.assert_no_art_uploads(ids);
    s.assert_complementary_meshes();
    s.app.run("view.splitHorizontal", json!({})).unwrap();
    s.frame(vec![]);
    s.assert_no_art_uploads(ids);
    s.assert_complementary_meshes();
    assert_eq!(s.snapshot(), before, "view composition never edits art/selection/history");
}

#[test]
fn split_canvas_divider_owns_only_its_press_and_escape_restores_fraction() {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.enable();
    for command in ["view.splitVertical", "view.splitHorizontal"] {
        s.command(command);
        let before = s.snapshot();
        let initial = s.fraction();
        let ids = s.pair();
        let horizontal = s.app.view().unwrap().split.orientation == SplitOrientation::Horizontal;
        let mut press = s.point_on_divider();
        if horizontal {
            press.y += 2.0;
        } else {
            press.x += 2.0;
        }
        s.press(press, PointerButton::Primary);
        s.assert_no_art_uploads(ids);
        let target = if horizontal {
            pos2(press.x, s.rect().top() + s.rect().height() * 0.72)
        } else {
            pos2(s.rect().left() + s.rect().width() * 0.72, press.y)
        };
        s.frame(vec![Event::PointerMoved(target)]);
        assert!((s.fraction() - 0.72).abs() < 0.015);
        s.assert_no_art_uploads(ids);
        s.escape();
        assert!((s.fraction() - initial).abs() < 1e-6);
        s.release(target, PointerButton::Primary);
        assert_eq!(s.snapshot(), before);
        let p = s.point_on_divider();
        s.press(p, PointerButton::Primary);
        let beyond = if horizontal { pos2(p.x, s.rect().bottom() + 40.0) } else { pos2(s.rect().right() + 40.0, p.y) };
        s.frame(vec![Event::PointerMoved(beyond)]);
        assert_eq!(s.fraction(), 1.0, "divider ownership survives leaving the viewport");
        s.release(beyond, PointerButton::Primary);
        assert_eq!(s.snapshot(), before);
        s.command("view.splitReset");
    }
    s.command("view.splitVertical");
    let before = s.snapshot();
    let center = s.app.view().unwrap().center;
    let divider = s.point_on_divider();
    s.frame(vec![key(Key::Space, true)]);
    s.press(divider, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(divider + vec2(24.0, 12.0))]);
    s.release(divider + vec2(24.0, 12.0), PointerButton::Primary);
    s.frame(vec![key(Key::Space, false)]);
    assert_eq!(s.fraction(), 0.5, "Space-before-press navigation wins even on the divider");
    assert_ne!(s.app.view().unwrap().center, center);
    assert_eq!(s.snapshot(), before);
    // Idle Direct Selection normally owns Escape when a key anchor is selected. The divider
    // must retain the key through the real frontend shortcut handler, not merely tool input.
    s.app.select_tool("directSelection");
    s.app.run("select.anchors", json!({"id":s.rectangle.0,"anchors":[[0,0],[0,1]],"mode":"set"})).unwrap();
    s.app.run("select.key", json!({"id":s.rectangle.0,"anchor":[0,0]})).unwrap();
    let key_anchor = Some((s.rectangle, (0, 0)));
    assert_eq!(s.app.session.active().unwrap().selection.key_anchor, key_anchor);
    assert!(!s.app.session.tool_busy(), "the legitimate Direct Selection Escape owner is idle");
    let before = s.snapshot();
    let initial = s.fraction();
    let divider = s.point_on_divider();
    s.press(divider, PointerButton::Primary);
    let moved = pos2(s.rect().left() + s.rect().width() * 0.8, divider.y);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - initial).abs() > 0.2);
    assert_eq!(s.app.session.active().unwrap().selection.key_anchor, key_anchor);
    s.escape();
    assert!((s.fraction() - initial).abs() < 1e-6, "Escape reaches the active divider before the idle tool's select.key action");
    assert_eq!(s.app.session.active().unwrap().selection.key_anchor, key_anchor);
    s.release(moved, PointerButton::Primary);
    assert_eq!(s.snapshot(), before, "divider cancellation preserves the real key-anchor selection, document and history");
    // A lifted pen/finger may press and release within one batch and leave no hover position.
    let before = s.snapshot();
    let initial = s.fraction();
    let tap = s.point_on_divider();
    s.frame(vec![
        Event::PointerMoved(tap),
        Event::PointerButton { pos: tap, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: tap, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        Event::PointerGone,
    ]);
    s.idle();
    assert_eq!(s.fraction(), initial, "same-frame divider tap uses the press position after PointerGone");
    assert_eq!(s.snapshot(), before, "the quick divider tap cannot fall through to Direct Selection");
    // A second finger hands an already-moving divider to the ordinary view gesture.
    let mut fingers = Scene::new();
    fingers.enable();
    fingers.app.session.prefs.touch_gestures = true;
    let before = fingers.snapshot();
    let start = fingers.point_on_divider();
    let finger = |id, phase, pos| Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos, force: None };
    fingers.frame(vec![
        finger(0, egui::TouchPhase::Start, start),
        Event::PointerMoved(start),
        Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    let moved = start + vec2(40.0, 0.0);
    fingers.frame(vec![finger(0, egui::TouchPhase::Move, moved), Event::PointerMoved(moved)]);
    assert!(fingers.fraction() > 0.54, "the first finger actually moved the divider before handoff");
    let second = moved + vec2(0.0, 60.0);
    let handed_off = fingers.fraction();
    fingers.frame(vec![finger(1, egui::TouchPhase::Start, second)]);
    assert_eq!(fingers.fraction(), handed_off, "gesture takeover keeps the moved divider value, without Escape restoration");
    let further = moved + vec2(40.0, 0.0);
    fingers.frame(vec![finger(0, egui::TouchPhase::Move, further), Event::PointerMoved(further)]);
    assert_eq!(fingers.fraction(), handed_off, "the two-finger view gesture releases divider ownership");
    fingers.frame(vec![
        finger(0, egui::TouchPhase::End, further),
        Event::PointerButton { pos: further, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        Event::PointerGone,
    ]);
    fingers.frame(vec![finger(1, egui::TouchPhase::End, second)]);
    fingers.idle();
    assert_eq!(fingers.fraction(), handed_off);
    assert_eq!(fingers.snapshot(), before, "the moved multi-touch handoff is neither an art edit nor an Undo tap");
}

#[test]
fn split_canvas_side_specific_picking_and_crossing_drag_keep_one_undo() {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.enable();
    let original = s.document();
    // Main Preview picks a fill interior; Outline comparison only picks the path.
    s.click(s.screen(Point::new(100.0, 100.0)));
    assert_eq!(s.app.session.active().unwrap().selection.objects, vec![s.rectangle]);
    s.app.run("select.set", json!({"ids":[]})).unwrap();
    s.idle();
    s.click(s.screen(Point::new(220.0, 100.0)));
    assert!(s.app.session.active().unwrap().selection.objects.is_empty(), "comparison Outline ignores fill interior");
    let stationary = s.screen(Point::new(220.0, 100.0));
    s.command("view.splitPreview");
    s.frame(vec![Event::PointerButton { pos: stationary, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    s.release(stationary, PointerButton::Primary);
    assert_eq!(s.app.session.active().unwrap().selection.objects, vec![s.rectangle], "mode changes take effect under a stationary pointer");
    s.command("view.splitOutline");
    s.click(s.screen(Point::new(260.0, 100.0)));
    assert_eq!(s.app.session.active().unwrap().selection.objects, vec![s.rectangle]);
    assert_eq!(s.document(), original);
    for (from, to) in [(Point::new(100.0, 100.0), Point::new(220.0, 100.0)), (Point::new(260.0, 100.0), Point::new(100.0, 100.0))] {
        s.app.run("select.set", json!({"ids":[]})).unwrap();
        s.idle();
        let before = s.bounds();
        let undo = s.app.session.active().unwrap().history.undo.len();
        let start = s.screen(from);
        let end = s.screen(to);
        s.press(start, PointerButton::Primary);
        let center = pos2(s.point_on_divider().x, start.y);
        s.frame(vec![Event::PointerMoved(center)]);
        assert_eq!(s.fraction(), 0.5, "an art drag cannot become a divider drag at the seam");
        s.frame(vec![Event::PointerMoved(end)]);
        s.release(end, PointerButton::Primary);
        let after = s.bounds();
        assert!((after.x0 - before.x0 - (to.x - from.x)).abs() < 1e-6);
        assert!((after.y0 - before.y0 - (to.y - from.y)).abs() < 1e-6);
        assert_eq!(s.app.session.active().unwrap().history.undo.len(), undo + 1);
        s.command("edit.undo");
        assert_eq!(s.document(), original);
        s.command("edit.redo");
        assert_eq!(s.bounds(), after);
        s.command("edit.undo");
        assert_eq!(s.document(), original);
    }
    // A fresh art press eight points from the divider is outside its six-point hit strip.
    let p = s.point_on_divider() + vec2(-8.0, -20.0);
    s.click(p);
    assert_eq!(s.fraction(), 0.5);
    assert_eq!(s.app.session.active().unwrap().selection.objects, vec![s.rectangle]);
}

#[test]
fn split_canvas_comparison_modes_independent_pixel_region_and_shared_camera() {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.enable();
    let before = s.snapshot();
    let main_mode = (s.app.outline_view(), s.app.ui.view.pixel_preview, vectorcraft_render::proof::view().overprint, s.app.ui.view.trim_view);
    s.command("view.splitPreview");
    let [a, b] = s.pair();
    assert_eq!(s.image(a), s.image(b), "two Preview sides render the same original document/view/options");
    for (command, mode) in [
        ("view.splitOutline", SplitMode::Outline),
        ("view.splitOverprintPreview", SplitMode::OverprintPreview),
        ("view.splitTrimView", SplitMode::TrimView),
    ] {
        s.command(command);
        assert_eq!(s.app.view().unwrap().split.comparison, mode);
        assert_eq!(
            (s.app.outline_view(), s.app.ui.view.pixel_preview, vectorcraft_render::proof::view().overprint, s.app.ui.view.trim_view),
            main_mode
        );
        let [a, b] = s.pair();
        assert!(!s.image(a).pixels.is_empty() && !s.image(b).pixels.is_empty());
        if mode == SplitMode::OverprintPreview {
            let point = Point::new(29.0, 29.0);
            assert_ne!(s.raster_at(a, point), s.raster_at(b, point), "actual overlapping CMYK overprint is visible only in comparison");
        }
        if mode == SplitMode::TrimView {
            let point = Point::new(340.0, 100.0);
            assert!(
                s.raster_at(a, point).a() > 200 && s.raster_at(b, point).a() < 40,
                "Trim View removes actual art outside the artboard on its side only"
            );
        }
    }
    s.app.view_mut().unwrap().zoom = 4.0;
    s.command("view.splitPixelPreview");
    let [a, b] = s.pair();
    assert!(
        s.image(b).size[0] < s.image(a).size[0] && s.image(b).size[1] < s.image(a).size[1],
        "comparison actually takes document-pixel region rendering"
    );
    s.command("view.outline");
    assert!(s.app.outline_view());
    assert_eq!(s.app.view().unwrap().split.comparison, SplitMode::PixelPreview);
    s.command("view.outline");
    s.command("view.splitPreview");
    let center = s.rect().center();
    let old_center = s.app.view().unwrap().center;
    s.press(center, PointerButton::Middle);
    s.frame(vec![Event::PointerMoved(center + vec2(24.0, 12.0))]);
    s.release(center + vec2(24.0, 12.0), PointerButton::Middle);
    assert_ne!(s.app.view().unwrap().center, old_center);
    let zoom = s.app.view().unwrap().zoom;
    s.frame(vec![Event::PointerMoved(s.rect().center()), Event::Zoom(1.25)]);
    assert!(s.app.view().unwrap().zoom > zoom, "real canvas zoom input updates the common camera");
    s.app.select_tool("rotateView");
    let center = s.rect().center();
    s.press(center + vec2(60.0, 0.0), PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(center + vec2(0.0, 60.0))]);
    s.release(center + vec2(0.0, 60.0), PointerButton::Primary);
    assert!(s.app.view().unwrap().rotation.abs() > 45.0, "real Rotate View drag rotates the shared camera");
    s.size = vec2(480.0, 360.0);
    s.idle();
    let p = Point::new(180.0, 120.0);
    let xf = s.xf();
    let screen = s.screen(p);
    // GUI positions are f32; judge the round trip in subpixel screen units.
    let screen_error = (xf.to_doc(screen) - p).hypot() * xf.zoom;
    assert!(screen_error < 1e-3, "camera round-trip error: {screen_error} screen points, xf={xf:?}, p={p:?}, screen={screen:?}");
    let [a, b] = s.pair();
    assert_eq!(s.image(a), s.image(b), "shared camera after pan/zoom/rotate/resize");
    s.assert_complementary_meshes();
    assert_eq!(s.snapshot(), before);
}

#[test]
fn split_canvas_disable_presentation_and_document_transfer_release_art_pair() {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.enable();
    s.app.view_mut().unwrap().split.fraction = 0.37;
    s.idle();
    let settings = s.app.view().unwrap().split;
    let ids = s.pair();
    s.command("view.splitView");
    for id in ids {
        assert!(!s.uploaded.images.contains_key(&id));
    }
    assert!(s.uploaded.id(&s.ctx, MAIN).is_none() && s.uploaded.id(&s.ctx, COMPARISON).is_none());
    s.command("view.splitView");
    assert_eq!(s.app.view().unwrap().split, settings);
    s.pair();
    s.app.ui.screen_mode = 3;
    s.idle();
    assert!(s.uploaded.id(&s.ctx, MAIN).is_none() && s.uploaded.id(&s.ctx, COMPARISON).is_none());
    s.app.ui.screen_mode = 0;
    s.idle();
    s.pair();
    assert_eq!(s.app.view().unwrap().split, settings);
    let doc_before = s.document();
    s.app.run("file.new", json!({"width":320,"height":240})).unwrap();
    s.idle();
    assert!(!s.app.view().unwrap().split.enabled, "a new document does not inherit another tab's split choice");
    assert!(s.uploaded.id(&s.ctx, MAIN).is_none() && s.uploaded.id(&s.ctx, COMPARISON).is_none());
    s.app.run("document.activate", json!({"index":0})).unwrap();
    s.idle();
    assert_eq!(s.document(), doc_before);
    assert_eq!(s.app.view().unwrap().split, settings);
    s.pair();
}
