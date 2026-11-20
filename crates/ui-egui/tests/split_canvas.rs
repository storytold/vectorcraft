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

// Each integration case needs a fresh process: render workers read SYNC during startup,
// and a placed-bitmap Loader must be registered before any Session is created.
// Command::env changes only the child's environment, safely before that process starts.
const CHILD_CASE: &str = "VECTORCRAFT_SPLIT_CANVAS_TEST_CASE";
const CASE_COMPLETED: &str = "VECTORCRAFT_SPLIT_CANVAS_COMPLETED";

macro_rules! synchronous_test {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            run_canvas_case(stringify!($name), true, || $body);
        }
    };
}

macro_rules! asynchronous_test {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            run_canvas_case(stringify!($name), false, || $body);
        }
    };
}

fn run_canvas_case(name: &str, synchronous: bool, body: impl FnOnce()) {
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        assert_eq!(selected, std::ffi::OsStr::new(name), "the child must run exactly its selected case");
        assert_eq!(std::env::var_os("VECTORCRAFT_SYNC_RENDER").is_some(), synchronous, "the child must receive its rendering mode before startup");
        body();
        // The closure and its locals have completed. A zero-selected libtest run cannot emit this.
        eprintln!("\n{CASE_COMPLETED}:{name}");
        return;
    }

    // Serialize launches in this process. The child owns a different GLOBALS lock.
    let _lock = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
    let mut command = std::process::Command::new(std::env::current_exe().expect("the actual integration test binary"));
    if synchronous {
        command.env("VECTORCRAFT_SYNC_RENDER", "1");
    } else {
        command.env_remove("VECTORCRAFT_SYNC_RENDER");
    }
    let output = command
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, name)
        .output()
        .expect("run the exact canvas case in this test binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "isolated canvas case {name} failed: {}\n{stdout}\n{stderr}", output.status);
    let completed = format!("{CASE_COMPLETED}:{name}");
    assert_eq!(
        stderr.lines().filter(|line| *line == completed).count(),
        1,
        "the isolated child must complete this case, not pass with zero selected tests\n{stdout}\n{stderr}"
    );
}

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

synchronous_test!(split_canvas_real_modes_share_geometry_and_complementary_clips, {
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
    review_598151_outline_edge_ink(&s, comparison);
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
});

synchronous_test!(split_canvas_divider_owns_only_its_press_and_escape_restores_fraction, {
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
});

synchronous_test!(split_canvas_side_specific_picking_and_crossing_drag_keep_one_undo, {
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
});

synchronous_test!(split_canvas_comparison_modes_independent_pixel_region_and_shared_camera, {
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
});

synchronous_test!(split_canvas_disable_presentation_and_document_transfer_release_art_pair, {
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
});

synchronous_test!(split_canvas_first_shown_pair_retires_the_current_documents_effect_budget, {
    let _globals = Globals::new();
    let mut s = Scene::new();
    let effects = s.app.session.active().unwrap().plugin_effects.clone();
    assert!(effects.opening(), "the document has not had a first frame");
    s.app.run("view.splitView", json!({})).unwrap();
    // Do not use Scene::enable/command here: they render idle frames before the assertion.
    assert!(effects.opening(), "enabling/submitting no frame must not retire the budget");
    s.frame(vec![]);
    for name in [MAIN, COMPARISON] {
        assert!(s.uploaded.names.values().any(|n| n.as_str() == name), "both actual artwork textures were uploaded");
    }
    assert!(!effects.opening(), "a complete current-document pair ends the first-view budget");
    s.frame(vec![]);
    assert!(!effects.opening(), "reusing the pair is idempotent");

    // Opening a different document must not retire that new document before its own pair exists.
    s.app.run("file.new", json!({"width": 320, "height": 240})).unwrap();
    let next = s.app.session.active().unwrap().plugin_effects.clone();
    assert!(next.opening(), "an old document's textures cannot retire the new document");
    s.app.run("view.splitView", json!({})).unwrap();
    assert!(next.opening(), "the new view has not been drawn");
    s.frame(vec![]);
    assert!(!next.opening(), "only the new document's own complete pair retires it");
    assert!(!effects.opening(), "the old document remains settled");
});

synchronous_test!(split_canvas_pending_placed_bitmap_requests_a_repaint_without_input, {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, mpsc};
    use std::time::{Duration, Instant};

    const FILE: &str = "8211-channel-held-blue-page";
    struct LoaderGate {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    static GATE: OnceLock<LoaderGate> = OnceLock::new();
    // Failure-before must still release the actual bitmap worker before the child exits.
    struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    fn held_blue_page(
        placed: &vectorcraft_doc::PlacedDocument,
        _bytes: &[u8],
    ) -> Option<(vectorcraft_doc::Document, vectorcraft_doc::Node, vectorcraft_geom::Rect)> {
        if placed.key != FILE {
            return None;
        }
        let gate = GATE.get()?;
        gate.entered.send(()).ok()?;
        gate.release.lock().ok()?.recv_timeout(Duration::from_secs(30)).ok()?;
        let mut doc = vectorcraft_doc::Document::new(40.0, 20.0);
        doc.layers.clear();
        let fill = vectorcraft_doc::Appearance::basic(
            vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)),
            vectorcraft_color::Paint::None,
            0.0,
        );
        let rect = vectorcraft_geom::Rect::new(0.0, 0.0, 40.0, 20.0);
        let art = vectorcraft_doc::Node::path(vectorcraft_doc::NodeId(1), vectorcraft_geom::shapes::rectangle(rect), fill);
        Some((doc, art, rect))
    }

    let (entered, began) = mpsc::channel();
    let (release, held) = mpsc::channel();
    assert!(GATE.set(LoaderGate { entered, release: Mutex::new(held) }).is_ok());
    let mut release_on_drop = ReleaseOnDrop(Some(release));
    // This child has initialized no Session yet. The existing registration is first-wins.
    vectorcraft_doc::placed_document::set_loader(held_blue_page);
    let _globals = Globals::new();
    let mut s = Scene::new();
    // A fresh blank document: the generated blue page cannot hide behind Scene's original art.
    s.app.run("file.new", json!({"width":320,"height":240})).unwrap();
    let st = s.app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    doc.images.insert(FILE.into(), vectorcraft_doc::ImageBlob::new("application/json", b"blue-page-fixture".to_vec()));
    let id = doc.alloc_id();
    let layer = doc.layers[0].id;
    let placed = vectorcraft_doc::PlacedDocument {
        link: vectorcraft_doc::LinkInfo::new("split-lifecycle.vectorcraft"),
        key: FILE.into(),
        bounding: false,
        page_box: Default::default(),
        width: 40.0,
        height: 20.0,
        xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
        placement: Default::default(),
    };
    doc.insert(Some(layer), 0, vectorcraft_doc::Node::new(id, vectorcraft_doc::NodeKind::PlacedDocument(Box::new(placed)))).unwrap();
    st.revision += 1;
    let view = s.app.view_mut().unwrap();
    view.fitted = true;
    view.center = Point::new(160.0, 120.0);
    view.zoom = 1.5;
    view.split.enabled = true;
    // Outline draws placed art exactly, invoking the loader on the UI thread. Both Preview
    // sides exercise the real progressive bitmap route and its pending-set deduplication.
    view.split.comparison = SplitMode::Preview;
    assert!(!vectorcraft_render::placed_document::busy());
    let generation = vectorcraft_render::placed_document::generation();

    let polls = Arc::new(AtomicUsize::new(0));
    let observed = polls.clone();
    s.ctx.set_repaint_observer(move |info| {
        if info.delay == Duration::from_millis(30) {
            observed.fetch_add(1, Ordering::SeqCst);
        }
    });
    s.frame(vec![]);
    began.recv_timeout(Duration::from_secs(10)).expect("the real bitmap loader entered while its pending key is held");
    assert!(vectorcraft_render::placed_document::busy(), "the worker is actually pending, not merely slow by assumption");
    let pair = s.pair();
    let at = Point::new(120.0, 110.0);
    for texture in pair {
        assert_ne!(s.raster_at(texture, at), Color32::BLUE, "the held real bitmap has not replaced the placeholder");
    }
    // Only idle time, no new pointer or document input. Observe the exact request, not egui's
    // effective deadline, which an unrelated immediate repaint could otherwise mask.
    polls.store(0, Ordering::SeqCst);
    s.frame(vec![]);
    assert!(polls.load(Ordering::SeqCst) > 0, "a pending placed bitmap schedules an idle repaint while Split View owns rendering");

    release_on_drop.0.take().unwrap().send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while vectorcraft_render::placed_document::busy() && Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(!vectorcraft_render::placed_document::busy(), "the signaled bitmap completed within the bound");
    assert_ne!(vectorcraft_render::placed_document::generation(), generation, "completion advances generation before pending clears");
    // Advance to the already-observed scheduled wakeup. No artificial user event forces it.
    s.time += 0.03;
    s.frame(vec![]);
    for texture in s.pair() {
        assert_eq!(s.raster_at(texture, at), Color32::BLUE, "the scheduled idle frame shows the real completed bitmap");
    }
    assert!(!s.uploaded.art_sets.is_empty(), "actual artwork textures were refreshed after bitmap completion");
    polls.store(0, Ordering::SeqCst);
    s.frame(vec![]);
    assert_eq!(polls.load(Ordering::SeqCst), 0, "settled bitmap work does not keep requesting 30 ms polls");
    assert!(s.uploaded.art_sets.is_empty(), "the settled pair is reused");
});

asynchronous_test!(split_canvas_rejects_a_bitmap_completion_between_side_renders, {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, mpsc};
    use std::time::{Duration, Instant};

    const FILE: &str = "8211-between-sides-blue-page";
    struct LoaderGate {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    static GATE: OnceLock<LoaderGate> = OnceLock::new();
    // Failure-before must still release the actual bitmap worker before the child exits.
    struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(release) = self.0.take() {
                let _ = release.send(());
            }
        }
    }
    fn held_blue_page(
        placed: &vectorcraft_doc::PlacedDocument,
        _bytes: &[u8],
    ) -> Option<(vectorcraft_doc::Document, vectorcraft_doc::Node, vectorcraft_geom::Rect)> {
        if placed.key != FILE {
            return None;
        }
        let gate = GATE.get()?;
        gate.entered.send(()).ok()?;
        gate.release.lock().ok()?.recv_timeout(Duration::from_secs(30)).ok()?;
        let mut doc = vectorcraft_doc::Document::new(40.0, 20.0);
        doc.layers.clear();
        let fill = vectorcraft_doc::Appearance::basic(
            vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0)),
            vectorcraft_color::Paint::None,
            0.0,
        );
        let rect = vectorcraft_geom::Rect::new(0.0, 0.0, 40.0, 20.0);
        let art = vectorcraft_doc::Node::path(vectorcraft_doc::NodeId(1), vectorcraft_geom::shapes::rectangle(rect), fill);
        Some((doc, art, rect))
    }

    let (entered, began) = mpsc::channel();
    let (release, held) = mpsc::channel();
    assert!(GATE.set(LoaderGate { entered, release: Mutex::new(held) }).is_ok());
    let _release_on_drop = ReleaseOnDrop(Some(release.clone()));
    // This child has initialized no Session yet. The existing registration is first-wins.
    vectorcraft_doc::placed_document::set_loader(held_blue_page);
    let _globals = Globals::new();
    let mut s = Scene::new();
    // A fresh blank document: the generated blue page cannot hide behind Scene's original art.
    s.app.run("file.new", json!({"width":320,"height":240})).unwrap();
    let st = s.app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    doc.images.insert(FILE.into(), vectorcraft_doc::ImageBlob::new("application/json", b"blue-page-fixture".to_vec()));
    let id = doc.alloc_id();
    let layer = doc.layers[0].id;
    let placed = vectorcraft_doc::PlacedDocument {
        link: vectorcraft_doc::LinkInfo::new("split-lifecycle.vectorcraft"),
        key: FILE.into(),
        bounding: false,
        page_box: Default::default(),
        width: 40.0,
        height: 20.0,
        xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
        placement: Default::default(),
    };
    doc.insert(Some(layer), 0, vectorcraft_doc::Node::new(id, vectorcraft_doc::NodeKind::PlacedDocument(Box::new(placed)))).unwrap();
    st.revision += 1;
    let view = s.app.view_mut().unwrap();
    view.fitted = true;
    view.center = Point::new(160.0, 120.0);
    view.zoom = 1.5;
    view.split.enabled = true;
    // Outline draws placed art exactly, invoking the loader on the UI thread. Both Preview
    // sides exercise the real progressive bitmap route and its pending-set deduplication.
    view.split.comparison = SplitMode::Preview;
    assert!(!vectorcraft_render::placed_document::busy());
    let generation = vectorcraft_render::placed_document::generation();

    let first_completion = Arc::new(AtomicBool::new(false));
    let observed = first_completion.clone();
    let began = Mutex::new(began);
    let (report, result) = mpsc::channel::<Result<(), String>>();
    s.ctx.set_repaint_observer(move |_| {
        if std::thread::current().name() != Some("vectorcraft-split-canvas") || observed.swap(true, Ordering::SeqCst) {
            return;
        }
        // ThumbWorker sends side 0's actual image, then calls request_repaint, then starts
        // side 1. Do not re-enter Context here: this callback may hold egui's context lock.
        // Errors return to the test thread; never panic and disconnect the background worker.
        let outcome = (|| -> Result<(), String> {
            began
                .lock()
                .map_err(|_| "loader receiver poisoned".to_owned())?
                .recv_timeout(Duration::from_secs(10))
                .map_err(|e| format!("loader did not enter: {e}"))?;
            if !vectorcraft_render::placed_document::busy() {
                return Err("held loader was not actually pending".into());
            }
            release.send(()).map_err(|e| format!("could not release loader: {e}"))?;
            let until = Instant::now() + Duration::from_secs(10);
            while vectorcraft_render::placed_document::busy() && Instant::now() < until {
                std::thread::yield_now();
            }
            if vectorcraft_render::placed_document::busy() {
                return Err("released bitmap did not finish within bound".into());
            }
            if vectorcraft_render::placed_document::generation() == generation {
                return Err("completion did not advance generation before pending cleared".into());
            }
            Ok(())
        })();
        let _ = report.send(outcome);
    });

    // No ordinary canvas idle frame has primed a bitmap or uploaded either split texture.
    s.frame(vec![]);
    result.recv_timeout(Duration::from_secs(20)).expect("first actual split completion callback").unwrap();
    assert!(first_completion.load(Ordering::SeqCst));
    // Observe the FIRST published pair, not a later repaired frame. In the original, side 0
    // is a placeholder and side 1 is blue. The guard must discard that entire pair before
    // upload; a fresh pair renders after generation is stable and both sides are blue.
    let until = Instant::now() + Duration::from_secs(10);
    while s.uploaded.art_sets.is_empty() && Instant::now() < until {
        s.frame(vec![]);
        std::thread::yield_now();
    }
    assert_eq!(s.uploaded.art_sets.len(), 2, "first publication uploads both actual artwork textures");
    let colors = s.pair().map(|texture| s.raster_at(texture, Point::new(120.0, 110.0)));
    assert_eq!(colors, [Color32::BLUE; 2], "first published pair must not mix placeholder and ready pixels");
});

// The real artboard-name press keeps ownership when its release crosses the divider.
synchronous_test!(review_4625287675_name_release_over_divider_keeps_its_owner, {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.app.run("file.new", json!({"width":320,"height":240,"artboards":2})).unwrap();
    let v = s.app.view_mut().unwrap();
    v.fitted = true;
    v.center = Point::new(160.0, 120.0);
    v.zoom = 1.5;
    v.artboard = 1;
    s.enable();
    fn label_point(shape: &Shape) -> Option<Pos2> {
        match shape {
            Shape::Vec(shapes) => shapes.iter().find_map(label_point),
            Shape::Text(text) if text.galley.job.text.starts_with("01 -") => Some(text.pos + text.galley.rect.center().to_vec2()),
            _ => None,
        }
    }
    let name = s.output.shapes.iter().find_map(|shape| label_point(&shape.shape)).expect("the actual first artboard name is painted");
    assert!(s.rect().contains(name), "CONTROL: the name is inside the real canvas");
    let divider = s.point_on_divider();
    assert!((name.x - divider.x).abs() > 20.0, "CONTROL: the name press is away from the divider");
    let before = s.snapshot();
    let fraction = s.fraction();
    s.press(name, PointerButton::Primary);
    assert_eq!(s.app.view().unwrap().artboard, 1, "CONTROL: a name selects on release, not press");
    s.release(divider, PointerButton::Primary);
    assert_eq!(s.app.view().unwrap().artboard, 0, "REGRESSION: release still completes the original name press over the divider");
    assert_eq!(s.fraction(), fraction);
    assert_eq!(s.snapshot(), before, "a name press changes neither artwork, selection nor Undo history");
    s.app.view_mut().unwrap().artboard = 1;
    let blank = s.screen(Point::new(-20.0, 100.0));
    assert!(s.rect().contains(blank));
    s.click(blank);
    assert_eq!(s.app.view().unwrap().artboard, 1, "a later canvas release must not complete a stale name press");
    let divider = s.point_on_divider();
    let moved = pos2(s.rect().left() + s.rect().width() * 0.7, divider.y);
    s.press(divider, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - 0.7).abs() < 0.02, "CONTROL: the divider admits its own next press");
    s.release(moved, PointerButton::Primary);
});

synchronous_test!(review_4625287680_divider_escape_owns_bound_command_and_repeat, {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.enable();
    s.app.run("shortcuts.set", json!({"id":"view.outline","shortcut":"Escape","force":true})).unwrap();
    s.idle();
    let initial_outline = s.app.outline_view();
    s.escape();
    assert_ne!(s.app.outline_view(), initial_outline, "CONTROL: an idle Escape runs the legitimate configured command");
    s.command("view.outline");
    assert_eq!(s.app.outline_view(), initial_outline);
    let before = s.snapshot();
    let fraction = s.fraction();
    let divider = s.point_on_divider();
    let moved = pos2(s.rect().left() + s.rect().width() * 0.72, divider.y);
    s.press(divider, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - fraction).abs() > 0.15, "CONTROL: a real divider drag has started");
    s.frame(vec![key(Key::Escape, true)]);
    assert!((s.fraction() - fraction).abs() < 1e-6, "Escape restores the divider's starting fraction");
    assert_eq!(s.app.outline_view(), initial_outline, "REGRESSION: the same Escape cannot run its bound command");
    s.frame(vec![Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed: true, repeat: true, modifiers: Modifiers::NONE }]);
    assert_eq!(s.app.outline_view(), initial_outline, "the cancelling Escape keeps its repeat until release");
    s.frame(vec![key(Key::Escape, false)]);
    s.release(moved, PointerButton::Primary);
    assert_eq!(s.snapshot(), before);
    s.escape();
    assert_ne!(s.app.outline_view(), initial_outline, "CONTROL: a fresh Escape after cancellation runs the command again");
});

// Append-only witnesses. Reuse the existing process isolation, Scene and Globals fixtures.
fn review8211_materials_label(shape: &Shape) -> Option<Pos2> {
    match shape {
        Shape::Vec(children) => children.iter().find_map(review8211_materials_label),
        Shape::Text(text) if text.galley.text() == "Materials" => Some(text.pos + text.galley.size() * 0.5),
        _ => None,
    }
}

impl Scene {
    fn review8211_materials_frame(&mut self, events: Vec<Event>) {
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
            vectorcraft_ui_egui::shortcuts::handle(&mut self.app, ui.ctx());
            egui::Window::new("Review mapped graphic")
                .fixed_pos(pos2(850.0, 10.0))
                .default_width(300.0)
                .show(ui.ctx(), |ui| vectorcraft_ui_egui::panels::three_d::show(&mut self.app, ui));
            canvas::show(&mut self.app, ui);
        });
        self.uploaded.apply(&self.ctx, &output.textures_delta);
        output.textures_delta.clear();
        self.output = output;
    }
}

fn review8211_mapped_scene() -> (Scene, Pos2) {
    let mut s = Scene::new();
    s.size = vec2(1200.0, 700.0);
    s.enable();
    let mark = s.app.run("path.create", json!({"d":"M0 0L8 0L8 6L0 6Z"})).unwrap()["id"].clone();
    s.app.run("threeD.addGraphic", json!({"name":"Review mark","ids":[mark]})).unwrap();
    s.app.run("select.set", json!({"ids":[s.rectangle.0]})).unwrap();
    s.app
        .run("effect.apply", json!({"effect":vectorcraft_effects::ROTATE,"item":null,"params":{"rotationX":0.0,"rotationY":0.0,"rotationZ":0.0}}))
        .unwrap();
    s.app.run("threeD.mapGraphic", json!({"symbol":"Review mark"})).unwrap();
    for _ in 0..3 {
        s.review8211_materials_frame(vec![]);
    }
    let tab = s.output.shapes.iter().find_map(|s| review8211_materials_label(&s.shape)).expect("CONTROL: actual Materials tab is shown");
    s.review8211_materials_frame(vec![Event::PointerMoved(tab)]);
    s.review8211_materials_frame(vec![Event::PointerButton { pos: tab, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    s.review8211_materials_frame(vec![Event::PointerButton { pos: tab, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    s.review8211_materials_frame(vec![]);
    let handle = {
        let doc = &s.app.session.active().unwrap().doc;
        let node = doc.node(s.rectangle).unwrap();
        let frame = vectorcraft_effects::graphic_frame_at(Some(doc), node, None, 0).expect("CONTROL: a generated plane has a graphic frame");
        let graphics = vectorcraft_effects::mapped_graphics(&node.appearance.effects[0].params);
        assert_eq!(graphics.len(), 1);
        let (at, seen) = frame.handle(&graphics[0]).expect("CONTROL: the mapped graphic has a handle");
        assert!(seen, "CONTROL: its face is visible");
        s.screen(at)
    };
    let rect = s.rect();
    s.app.view_mut().unwrap().split.fraction = (handle.x + 35.0 - rect.left()) / rect.width();
    s.review8211_materials_frame(vec![]);
    let divider = pos2(s.point_on_divider().x, handle.y);
    assert!(rect.contains(handle) && rect.contains(divider));
    assert!((divider.x - handle.x).abs() > 20.0, "CONTROL: the new press is well away from the divider");
    (s, handle)
}

fn review8211_graphics(s: &Scene) -> Value {
    s.app.session.active().unwrap().doc.node(s.rectangle).unwrap().appearance.effects[0].params["graphics"].clone()
}

fn review8211_mapped_handle_case(on_divider: bool) {
    let (mut s, handle) = review8211_mapped_scene();
    let fraction = s.fraction();
    let divider = pos2(s.point_on_divider().x, handle.y);
    let graphics = review8211_graphics(&s);
    let before = s.document();
    let history = s.app.session.active().unwrap().history.undo.len();
    s.review8211_materials_frame(vec![Event::PointerMoved(handle)]);
    s.review8211_materials_frame(vec![Event::PointerButton {
        pos: handle,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    s.review8211_materials_frame(vec![Event::PointerMoved(handle + vec2(12.0, 0.0))]);
    assert_ne!(s.document(), before, "CONTROL: the real graphic handle has previewed a move");
    assert_ne!(review8211_graphics(&s), graphics, "CONTROL: the preview changed the mapped graphic, not an ordinary object transform");
    let release = if on_divider { divider } else { handle + vec2(24.0, 0.0) };
    s.review8211_materials_frame(vec![Event::PointerMoved(release)]);
    s.review8211_materials_frame(vec![Event::PointerButton {
        pos: release,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    let released = s.document();
    assert_ne!(released, before, "CONTROL: the completed handle drag changed its graphic");
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1, "one completed handle drag is one undo step");
    assert_eq!(s.fraction(), fraction, "a handle-owned press never moves the divider");
    s.review8211_materials_frame(vec![Event::PointerMoved(release + vec2(15.0, 10.0))]);
    assert_eq!(s.document(), released, "REGRESSION: a released handle cannot move again when the pointer leaves the divider");
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1, "a hover after release creates no additional history");
    assert_eq!(s.fraction(), fraction);
}

synchronous_test!(review_4626118623_mapped_handle_release_on_divider_ends_ownership, {
    let _globals = Globals::new();
    review8211_mapped_handle_case(false); // Legitimate ordinary release control first.
    review8211_mapped_handle_case(true);
});

fn review8211_coalesced_handle_press(on_handle: bool) {
    let (mut s, handle) = review8211_mapped_scene();
    let graphics = review8211_graphics(&s);
    let fraction = s.fraction();
    let divider = pos2(s.point_on_divider().x, handle.y);
    let (press, hover) = if on_handle { (handle, divider) } else { (handle + vec2(0.0, 30.0), handle) };
    assert!((press - hover).length() > 20.0, "CONTROL: the press and later hover are distinct targets");
    s.review8211_materials_frame(vec![Event::PointerMoved(press)]);
    s.review8211_materials_frame(vec![
        Event::PointerButton { pos: press, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerMoved(hover),
    ]);
    let moved = hover + vec2(12.0, 0.0);
    s.review8211_materials_frame(vec![Event::PointerMoved(moved)]);
    if on_handle {
        assert_ne!(review8211_graphics(&s), graphics, "REGRESSION: a handle press must start from P even when a later move Q is on the divider");
    } else {
        assert_eq!(
            review8211_graphics(&s),
            graphics,
            "REGRESSION: a later hover on a handle cannot turn a non-handle press into a mapped-graphic drag"
        );
    }
    assert_eq!(s.fraction(), fraction, "neither non-divider press may start a divider drag from its later hover");
    s.review8211_materials_frame(vec![Event::PointerButton {
        pos: moved,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
}

synchronous_test!(review_4626118623_mapped_handle_uses_actual_press_before_later_divider_move, {
    let _globals = Globals::new();
    review8211_coalesced_handle_press(true);
});

synchronous_test!(review_4626118623_mapped_handle_does_not_start_from_later_hover, {
    let _globals = Globals::new();
    review8211_coalesced_handle_press(false);
});

synchronous_test!(review_4626118652_divider_drag_is_bound_to_its_document_view, {
    let _globals = Globals::new();
    let mut s = Scene::new();
    s.enable();
    s.app.view_mut().unwrap().split.fraction = 0.3;
    assert_eq!(s.app.run("window.newWindow", json!({})).unwrap()["view"], 2);
    s.app.view_mut().unwrap().split.fraction = 0.7;
    assert_eq!(s.app.run("window.nextDocument", json!({})).unwrap()["view"], 1);
    s.idle();
    let uid = s.app.session.active().unwrap().uid;
    let before = s.snapshot();
    let start = s.point_on_divider();
    let moved = pos2(s.rect().left() + s.rect().width() * 0.45, start.y);
    s.press(start, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - 0.45).abs() < 1e-6, "CONTROL: dragging within the original view works");
    assert_eq!(s.app.run("window.nextDocument", json!({})).unwrap()["view"], 2);
    assert_eq!(s.app.session.active().unwrap().uid, uid, "CONTROL: this is another view of the same document");
    assert!((s.fraction() - 0.7).abs() < 1e-6, "CONTROL: the other view starts with its own fraction");
    s.frame(vec![]);
    assert!((s.fraction() - 0.7).abs() < 1e-6, "REGRESSION: a held drag from another view cannot write this view's fraction");
    s.release(moved, PointerButton::Primary);
    assert!((s.fraction() - 0.7).abs() < 1e-6);
    assert_eq!(s.snapshot(), before, "view switching and divider cancellation do not alter art or history");
    assert_eq!(s.app.run("window.nextDocument", json!({})).unwrap()["view"], 1);
    s.idle();
    assert!((s.fraction() - 0.45).abs() < 1e-6, "the original view retains the last value reached before switching");
    assert_eq!(s.app.run("window.nextDocument", json!({})).unwrap()["view"], 2);
    s.idle();
    let fresh = s.point_on_divider();
    let target = pos2(s.rect().left() + s.rect().width() * 0.8, fresh.y);
    s.press(fresh, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(target)]);
    s.release(target, PointerButton::Primary);
    assert!((s.fraction() - 0.8).abs() < 1e-6, "CONTROL: a fresh press in the new view still drags its divider");
    assert_eq!(s.snapshot(), before);
});
synchronous_test!(review_4626118640_primary_down_owns_split_mode_before_later_move, {
    let _globals = Globals::new();
    // Establish the real Preview/Outline fill-picking distinction before testing coalescing.
    for (point, picks_fill) in [(Point::new(100.0, 100.0), true), (Point::new(220.0, 100.0), false)] {
        let mut s = Scene::new();
        s.enable();
        s.command("view.splitOutline");
        s.app.run("select.set", json!({"ids":[]})).unwrap();
        s.idle();
        let original = s.document();
        let start = s.screen(point);
        assert!(s.rect().contains(start));
        s.press(start, PointerButton::Primary);
        assert_eq!(
            s.app.session.active().unwrap().selection.objects,
            if picks_fill { vec![s.rectangle] } else { vec![] },
            "CONTROL: separate-frame presses use the actual side's fill-picking mode"
        );
        s.release(start, PointerButton::Primary);
        assert_eq!(s.document(), original, "CONTROL: an ordinary click does not move the artwork");
    }

    for (label, later_divider, outside_prefix) in [
        ("Preview press then Outline hover", false, false),
        ("Preview press then divider hover", true, false),
        ("outside completed press then current Preview press", false, true),
    ] {
        let mut s = Scene::new();
        s.enable();
        s.command("view.splitOutline");
        s.app.run("select.set", json!({"ids":[]})).unwrap();
        s.idle();
        let original = s.document();
        let before = s.bounds();
        let undo = s.app.session.active().unwrap().history.undo.len();
        let fraction = s.fraction();
        let start = s.screen(Point::new(100.0, 100.0));
        let comparison = s.screen(Point::new(220.0, 100.0));
        let divider = s.point_on_divider();
        let end = s.screen(Point::new(230.0, 100.0));
        assert!(s.rect().contains(start) && s.rect().contains(comparison) && s.rect().contains(end));
        assert!(start.x < divider.x - 4.0 && comparison.x > divider.x + 4.0);
        assert!(!s.app.outline_view(), "the main side starts in Preview");
        assert_eq!(s.app.view().unwrap().split.comparison, SplitMode::Outline);
        let later = if later_divider { divider } else { comparison };
        let mut events = vec![];
        if outside_prefix {
            let outside = pos2(s.rect().left() - 20.0, start.y);
            assert!(!s.rect().contains(outside));
            events.extend([
                Event::PointerMoved(outside),
                Event::PointerButton { pos: outside, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                Event::PointerButton { pos: outside, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
            ]);
        }
        events.extend([
            Event::PointerMoved(start),
            Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerMoved(later),
        ]);
        s.frame(events);
        assert_eq!(
            s.app.session.active().unwrap().selection.objects,
            vec![s.rectangle],
            "REGRESSION: {label}: the admitted Down picks the Preview fill before the later hover"
        );
        assert_eq!(s.fraction(), fraction, "{label}: a later hover does not acquire the divider");
        s.frame(vec![Event::PointerMoved(end)]);
        s.release(end, PointerButton::Primary);
        let after = s.bounds();
        assert!((after.x0 - before.x0 - 130.0).abs() < 1e-6, "{label}: translation is measured from Down100, not the later hover: {after:?}");
        assert!((after.y0 - before.y0).abs() < 1e-6, "{label}: the artwork keeps its original vertical position");
        assert_eq!(s.app.session.active().unwrap().selection.objects, vec![s.rectangle]);
        assert_eq!(s.fraction(), fraction, "{label}: the press remains an artwork gesture across the seam");
        assert_eq!(s.app.session.active().unwrap().history.undo.len(), undo + 1, "{label}: one real move records one Undo step");
        s.command("edit.undo");
        assert_eq!(s.document(), original, "{label}: Undo restores the actual document");
    }
});

fn review_597489_divider_point(s: &Scene, fraction: f32) -> Pos2 {
    pos2(s.rect().left() + s.rect().width() * fraction, s.rect().center().y)
}

fn review_597489_divider_scene() -> Scene {
    let mut s = Scene::new();
    s.enable();
    s.command("view.splitVertical");
    s.command("view.splitPreview");
    let p = s.xf().to_doc(review_597489_divider_point(&s, 0.8));
    s.rectangle =
        NodeId(s.app.run("shape.rectangle", json!({"x":p.x - 12.0,"y":p.y - 12.0,"width":24,"height":24})).unwrap()["id"].as_u64().unwrap());
    s.app.run("paint.setFill", json!({"color":"#8050c0"})).unwrap();
    s.app.run("paint.setStroke", json!({"none":true})).unwrap();
    s.app.run("select.set", json!({"ids":[]})).unwrap();
    s.idle();
    assert!((s.fraction() - 0.5).abs() < 1e-6);
    let bounds = s.bounds();
    assert!(bounds.x0 < p.x && p.x < bounds.x1 && bounds.y0 < p.y && p.y < bounds.y1);
    s
}

fn review_597489_primary(p: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn review_597489_ordinary_drag(s: &mut Scene, fraction: f32) {
    assert_eq!(
        s.app.session.active().unwrap().selection.objects,
        vec![s.rectangle],
        "the ordinary Down reaches Selection and picks the new rectangle"
    );
    let before = s.bounds();
    let undo = s.app.session.active().unwrap().history.undo.len();
    let start = review_597489_divider_point(s, 0.8);
    let end = review_597489_divider_point(s, 0.85);
    let delta = s.xf().to_doc(end) - s.xf().to_doc(start);
    s.frame(vec![Event::PointerMoved(end)]);
    s.release(end, PointerButton::Primary);
    let after = s.bounds();
    assert!((after.x0 - before.x0 - delta.x).abs() < 1e-6, "the held ordinary gesture translates from its fresh Down");
    assert!((after.y0 - before.y0 - delta.y).abs() < 1e-6);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), undo + 1, "the ordinary drag commits exactly once");
    assert!((s.fraction() - fraction).abs() < 1e-6, "the later ordinary Up cannot edit the old divider");
    let released = s.snapshot();
    s.frame(vec![Event::PointerMoved(review_597489_divider_point(s, 0.9))]);
    assert_eq!(s.snapshot(), released, "button-up hover cannot continue either released gesture");
    assert!((s.fraction() - fraction).abs() < 1e-6);
}

fn review_597489_release_then_press(coalesced: bool) {
    let mut s = review_597489_divider_scene();
    let before = s.snapshot();
    let start = s.point_on_divider();
    s.press(start, PointerButton::Primary);
    let held = review_597489_divider_point(&s, 0.6);
    s.frame(vec![Event::PointerMoved(held)]);
    assert!((s.fraction() - 0.6).abs() < 1e-6, "CONTROL: the old divider really is held and moving");
    assert_eq!(s.snapshot(), before, "CONTROL: the held divider edits no document, selection or history");
    let release = review_597489_divider_point(&s, 0.65);
    let ordinary = review_597489_divider_point(&s, 0.8);
    if coalesced {
        s.frame(vec![review_597489_primary(release, false), review_597489_primary(ordinary, true)]);
    } else {
        s.release(release, PointerButton::Primary);
        assert!((s.fraction() - 0.65).abs() < 1e-6, "CONTROL: a separate release commits its own position");
        assert_eq!(s.snapshot(), before, "CONTROL: the divider edits no document, selection or history");
        s.press(ordinary, PointerButton::Primary);
    }
    assert!((s.fraction() - 0.65).abs() < 1e-6, "REGRESSION: old Up65 commits before the independent Down80, got {}", s.fraction());
    review_597489_ordinary_drag(&mut s, 0.65);
}

synchronous_test!(review_597489_owned_divider_release_before_fresh_down_keeps_both_owners, {
    let _globals = Globals::new();
    let mut ordinary = review_597489_divider_scene();
    let p = review_597489_divider_point(&ordinary, 0.8);
    ordinary.press(p, PointerButton::Primary);
    review_597489_ordinary_drag(&mut ordinary, 0.5);
    review_597489_release_then_press(false);
    review_597489_release_then_press(true);
});

fn review_597489_same_frame_divider(coalesced: bool) {
    let mut s = review_597489_divider_scene();
    let before = s.snapshot();
    let start = s.point_on_divider();
    let release = review_597489_divider_point(&s, 0.7);
    if coalesced {
        s.frame(vec![
            Event::PointerMoved(start),
            review_597489_primary(start, true),
            Event::PointerMoved(release),
            review_597489_primary(release, false),
        ]);
    } else {
        s.press(start, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(release)]);
        s.release(release, PointerButton::Primary);
    }
    assert!(
        (s.fraction() - 0.7).abs() < 1e-6,
        "REGRESSION: a completed divider gesture commits Up70 even with Down50 in the same frame, got {}",
        s.fraction()
    );
    assert_eq!(s.snapshot(), before, "a completed divider gesture edits no document, selection or history");
    let hover = review_597489_divider_point(&s, 0.9);
    s.frame(vec![Event::PointerMoved(hover)]);
    assert!((s.fraction() - 0.7).abs() < 1e-6, "button-up hover cannot continue the completed divider");
    assert_eq!(s.snapshot(), before);
    let ordinary = review_597489_divider_point(&s, 0.8);
    s.press(ordinary, PointerButton::Primary);
    review_597489_ordinary_drag(&mut s, 0.7);
}

synchronous_test!(review_597489_same_frame_divider_drag_commits_its_release_and_ends, {
    let _globals = Globals::new();
    review_597489_same_frame_divider(false);
    review_597489_same_frame_divider(true);
});

// The clipping path's real live effect moves its Preview region 100 pt to the right.
// Outline uses its plain path, so the two empty-space marquees distinguish the modes.
fn review_597489_mode_scene(split_on: bool) -> (Scene, NodeId) {
    let mut s = Scene::new();
    s.app.run("select.all", json!({})).unwrap();
    s.app.run("edit.clear", json!({})).unwrap();
    let ink = s.app.run("shape.rectangle", json!({"x":120,"y":100,"width":120,"height":60})).unwrap()["id"].clone();
    s.app.run("paint.setFill", json!({"color":"#d83020"})).unwrap();
    s.app.run("paint.setStroke", json!({"none":true})).unwrap();
    let mask = s.app.run("shape.rectangle", json!({"x":100,"y":80,"width":160,"height":100})).unwrap()["id"].clone();
    s.app.run("paint.setStroke", json!({"none":true})).unwrap();
    s.app.run("select.set", json!({"ids":[ink,mask.clone()]})).unwrap();
    let group = NodeId(s.app.run("object.clippingMask.make", json!({})).unwrap()["id"].as_u64().unwrap());
    // Making the mask clears its appearance; apply the real effect afterwards.
    s.app.run("select.set", json!({"ids":[mask]})).unwrap();
    s.app.run("effect.apply", json!({"effect":"distort.transform","item":null,"params":{"moveH":100.0}})).unwrap();
    s.app.run("select.set", json!({"ids":[]})).unwrap();
    s.enable();
    s.command("view.splitOutline");
    if !split_on {
        s.command("view.splitView");
    }
    assert_eq!(s.app.view().unwrap().split.enabled, split_on);
    assert_eq!(s.app.view().unwrap().split.comparison, SplitMode::Outline);
    assert!(!s.app.outline_view(), "CONTROL: the ordinary view starts in Preview");
    assert_eq!(s.app.session.prefs.marquee_selection, "touching", "CONTROL: the real default marquee touches objects");
    s.idle();
    (s, group)
}

fn review_597489_marquee_mode_case(split_on: bool, enable_while_held: bool) {
    let (mut s, group) = review_597489_mode_scene(split_on);
    let before = s.document();
    let history = (s.app.session.active().unwrap().history.undo.len(), s.app.session.active().unwrap().history.redo.len());
    let p = s.screen(Point::new(140.0, 120.0));
    let q = s.screen(Point::new(180.0, 150.0));
    assert!(s.rect().contains(p) && s.rect().contains(q));
    assert!(p.x < s.point_on_divider().x - 10.0 && q.x > s.point_on_divider().x + 10.0);
    s.press(p, PointerButton::Primary);
    assert!(s.ctx.input(|i| i.pointer.primary_down()));
    assert!(s.app.session.tool_busy(), "CONTROL: the ordinary Selection tool owns the held marquee");
    assert!(!s.app.session.tool_waiting(), "CONTROL: this ordinary marquee is not a deferred tool command");
    assert!(s.app.session.active().unwrap().selection.objects.is_empty(), "CONTROL: the press starts on empty Preview canvas");
    if enable_while_held {
        // Use the supported command without an idle/release frame in between.
        s.app.run("view.splitView", json!({})).unwrap();
        assert!(s.app.view().unwrap().split.enabled);
        assert!(s.ctx.input(|i| i.pointer.primary_down()));
        assert!(s.app.session.tool_busy(), "enabling Split View keeps the ordinary held gesture alive");
    }
    s.frame(vec![Event::PointerMoved(q)]);
    s.release(q, PointerButton::Primary);
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    assert!(!s.app.session.tool_busy(), "the ordinary marquee ends at release");
    assert!(
        s.app.session.active().unwrap().selection.objects.is_empty(),
        "REGRESSION: enabling Split View cannot change an already-held Preview marquee into an Outline marquee"
    );
    assert_eq!(s.document(), before, "a selection-only gesture leaves the art unchanged");
    assert_eq!((s.app.session.active().unwrap().history.undo.len(), s.app.session.active().unwrap().history.redo.len()), history);
    if s.app.view().unwrap().split.enabled {
        // A new press wholly in the comparison side must use Outline, proving the
        // real mode-sensitive fixture and retirement of the previous gesture's mode.
        let p = s.screen(Point::new(170.0, 120.0));
        let q = s.screen(Point::new(190.0, 150.0));
        assert!(p.x > s.point_on_divider().x + 10.0 && q.x > s.point_on_divider().x + 10.0);
        s.press(p, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(q)]);
        s.release(q, PointerButton::Primary);
        assert_eq!(
            s.app.session.active().unwrap().selection.objects,
            vec![group],
            "CONTROL: a fresh comparison-side Outline marquee reaches the plain clip group"
        );
        assert_eq!(s.document(), before);
        assert_eq!((s.app.session.active().unwrap().history.undo.len(), s.app.session.active().unwrap().history.redo.len()), history);
    }
}

synchronous_test!(review_597489_split_enabled_during_a_tool_gesture_keeps_its_start_mode, {
    let _globals = Globals::new();
    review_597489_marquee_mode_case(false, false); // Ordinary Split-OFF control.
    review_597489_marquee_mode_case(true, false); // Existing latch and fresh Outline control.
    review_597489_marquee_mode_case(false, true);
});

fn review_597489_completed_handle(coalesced: bool) {
    let (mut s, handle) = review8211_mapped_scene();
    let fraction = s.fraction();
    let before = s.document();
    let graphics = review8211_graphics(&s);
    let selection = s.app.session.active().unwrap().selection.objects.clone();
    let history = s.app.session.active().unwrap().history.undo.len();
    assert!(s.app.session.active().unwrap().history.redo.is_empty());
    let release = handle + vec2(24.0, 0.0);
    let moved = handle + vec2(12.0, 4.0);
    let hover = release + vec2(15.0, 10.0);
    let expected_graphics = {
        let doc = &s.app.session.active().unwrap().doc;
        let node = doc.node(s.rectangle).unwrap();
        let frame = vectorcraft_effects::graphic_frame_at(Some(doc), node, None, 0).expect("CONTROL: the mapped plane has a graphic frame");
        let list = vectorcraft_effects::mapped_graphics(&node.appearance.effects[0].params);
        let (at, seen) = frame.handle(&list[0]).expect("CONTROL: the mapped graphic has a visible handle");
        assert!(seen);
        let grab = s.xf().to_doc(handle) - at;
        let (x, y) = frame.offset_at(list[0].face, s.xf().to_doc(release) - grab).expect("CONTROL: Up maps onto the graphic's face");
        let mut expected = graphics.clone();
        expected[0]["x"] = json!((x * 10.0).round() / 10.0);
        expected[0]["y"] = json!((y * 10.0).round() / 10.0);
        expected
    };
    assert!(s.rect().contains(release) && release.x < s.point_on_divider().x - 8.0, "CONTROL: the release is on the canvas, away from the divider");
    s.review8211_materials_frame(vec![Event::PointerMoved(handle)]);
    let down = Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
    let up = Event::PointerButton { pos: release, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
    if coalesced {
        s.review8211_materials_frame(vec![down, Event::PointerMoved(moved), up, Event::PointerMoved(hover)]);
    } else {
        s.review8211_materials_frame(vec![down]);
        s.review8211_materials_frame(vec![Event::PointerMoved(release)]);
        assert_ne!(review8211_graphics(&s), graphics, "CONTROL: the real mapped handle previews the move while held");
        s.review8211_materials_frame(vec![up]);
    }
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    let released = s.document();
    let released_graphics = review8211_graphics(&s);
    assert_eq!(released_graphics, expected_graphics, "REGRESSION: the full mapped graphic must commit at Up, not the last move or later hover");
    assert_ne!(released_graphics, graphics, "REGRESSION: a mapped-handle press, move and release must complete at its release even in one frame");
    assert_ne!(released, before);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1, "the completed handle drag commits exactly once at release");
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    assert_eq!(s.fraction(), fraction);
    s.review8211_materials_frame(vec![Event::PointerMoved(release + vec2(15.0, 10.0))]);
    assert_eq!(s.document(), released, "REGRESSION: a later button-up hover cannot edit the released graphic");
    assert_eq!(review8211_graphics(&s), released_graphics);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1, "a later hover cannot commit again");
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    assert_eq!(s.fraction(), fraction);
    s.app.run("edit.undo", json!({})).unwrap();
    s.review8211_materials_frame(vec![]);
    assert_eq!(s.document(), before, "one Undo restores the whole document before the handle drag");
    assert_eq!(review8211_graphics(&s), graphics);
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    assert_eq!(s.app.session.active().unwrap().history.redo.len(), 1);
}

fn review_597489_divider_release_before_mapped_press() {
    let (mut s, handle) = review8211_mapped_scene();
    let before = s.document();
    let graphics = review8211_graphics(&s);
    let selection = s.app.session.active().unwrap().selection.objects.clone();
    let history = s.app.session.active().unwrap().history.undo.len();
    assert!(s.app.session.active().unwrap().history.redo.is_empty());
    let divider = pos2(s.point_on_divider().x, handle.y);
    let held = pos2(s.rect().left() + s.rect().width() * 0.60, handle.y);
    let old_release = pos2(s.rect().left() + s.rect().width() * 0.65, handle.y);
    assert!(s.rect().contains(held) && s.rect().contains(old_release));
    assert!(old_release.x < 830.0, "CONTROL: the divider release is left of the real Materials window at x850");
    assert!((old_release.x - handle.x).abs() > 35.0, "CONTROL: the fresh mapped-handle Down is away from the final divider");
    s.review8211_materials_frame(vec![Event::PointerMoved(divider)]);
    s.review8211_materials_frame(vec![Event::PointerButton {
        pos: divider,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    s.review8211_materials_frame(vec![Event::PointerMoved(held)]);
    assert!((s.fraction() - 0.60).abs() < 1e-6, "CONTROL: the real divider owns and follows its initial drag");
    assert_eq!(s.document(), before);
    assert_eq!(review8211_graphics(&s), graphics);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    s.review8211_materials_frame(vec![
        Event::PointerButton { pos: old_release, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: handle, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    assert!(s.ctx.input(|i| i.pointer.primary_down()));
    assert!((s.fraction() - 0.65).abs() < 1e-6, "REGRESSION: the old divider ends at its own Up before the fresh mapped-handle Down");
    let release = handle + vec2(24.0, 0.0);
    assert!((release.x - old_release.x).abs() > 8.0 && s.rect().contains(release));
    s.review8211_materials_frame(vec![Event::PointerMoved(release)]);
    assert_ne!(review8211_graphics(&s), graphics, "CONTROL: the fresh Down owns a real mapped-graphic preview");
    s.review8211_materials_frame(vec![Event::PointerButton {
        pos: release,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    let released = s.document();
    assert_ne!(released, before);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1, "only the fresh mapped gesture commits, exactly once");
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    assert!((s.fraction() - 0.65).abs() < 1e-6);
    s.review8211_materials_frame(vec![Event::PointerMoved(release + vec2(15.0, 10.0))]);
    assert_eq!(s.document(), released, "button-up hover cannot edit the completed mapped gesture");
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1);
    assert!((s.fraction() - 0.65).abs() < 1e-6, "REGRESSION: button-up hover cannot revive the old divider drag");
    s.app.run("edit.undo", json!({})).unwrap();
    s.review8211_materials_frame(vec![]);
    assert_eq!(s.document(), before, "one Undo restores the fresh mapped edit without an extra divider history entry");
    assert_eq!(review8211_graphics(&s), graphics);
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    assert_eq!(s.app.session.active().unwrap().history.redo.len(), 1);
    assert!((s.fraction() - 0.65).abs() < 1e-6, "Undo of an art edit leaves the completed divider position alone");
}

synchronous_test!(review_597489_mapped_handle_finishes_a_coalesced_press_and_release, {
    let _globals = Globals::new();
    review_597489_completed_handle(false); // Ordinary separate-frame mapped-handle control first.
    review_597489_completed_handle(true);
});

synchronous_test!(review_597489_divider_release_before_mapped_press_is_independent, {
    let _globals = Globals::new();
    review_597489_divider_release_before_mapped_press();
});

fn review_597704_zoom_scene() -> Scene {
    let mut s = Scene::new();
    s.enable();
    s.command("view.splitPreview");
    s.app.run("prefs.set", json!({"key":"animatedZoom","value":false})).unwrap();
    s.app.select_tool("zoom");
    s.idle();
    assert_eq!(s.app.session.tool_id(), "zoom");
    assert!(!s.app.session.prefs.animated_zoom);
    s
}

fn review_597704_zoom_expected(s: &Scene, p: Pos2, up: Pos2) -> (Point, f64) {
    let r = Rect::from_two_pos(p, up);
    assert!(r.width() > 8.0 && r.height() > 8.0, "CONTROL: a real zoom rectangle, not a click zoom");
    let fill = (s.rect().width() / r.width()).min(s.rect().height() / r.height());
    (s.xf().to_doc(r.center()), (s.app.view().unwrap().zoom * f64::from(fill)).clamp(0.0313, 640.0))
}

fn review_597704_assert_camera(s: &Scene, expected: (Point, f64), label: &str) {
    let v = s.app.view().unwrap();
    assert!(v.center.distance(expected.0) < 1e-6, "{label}: center {:?}, expected {:?}", v.center, expected.0);
    assert!((v.zoom - expected.1).abs() < 1e-6, "{label}: zoom {}, expected {}", v.zoom, expected.1);
}

fn review_597704_zoom_release_then_divider(coalesced: bool) {
    let mut s = review_597704_zoom_scene();
    let document = s.snapshot();
    let p = s.rect().left_top() + vec2(70.0, 70.0);
    let up = p + vec2(110.0, 65.0);
    let expected = review_597704_zoom_expected(&s, p, up);
    let original = (s.app.view().unwrap().center, s.app.view().unwrap().zoom);
    s.press(p, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(up)]);
    review_597704_assert_camera(&s, original, "CONTROL: preview does not apply the zoom before Up");
    assert_eq!(s.fraction(), 0.5);
    let divider = s.point_on_divider();
    if coalesced {
        s.frame(vec![review_597489_primary(up, false), review_597489_primary(divider, true)]);
    } else {
        s.release(up, PointerButton::Primary);
        review_597704_assert_camera(&s, expected, "CONTROL: separate old Zoom release");
        s.press(divider, PointerButton::Primary);
    }
    review_597704_assert_camera(&s, expected, "REGRESSION: old Zoom completes before fresh divider admission");
    let moved = review_597489_divider_point(&s, 0.7);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - 0.7).abs() < 1e-6, "the fresh press owns the divider, not another zoom");
    s.release(moved, PointerButton::Primary);
    review_597704_assert_camera(&s, expected, "divider completion cannot apply another zoom");
    assert_eq!(s.snapshot(), document, "view gestures change no art, selection or history");
}

fn review_597704_old_zoom_up_before_alt_down(coalesced: bool) -> (Point, f64) {
    let mut s = review_597704_zoom_scene();
    let before = s.snapshot();
    let p = s.rect().left_top() + vec2(70.0, 70.0);
    let initial_zoom = s.app.view().unwrap().zoom;
    s.press(p, PointerButton::Primary);
    let divider = s.point_on_divider();
    let alt_down = Event::PointerButton { pos: divider, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT };
    if coalesced {
        s.frame(vec![review_597489_primary(p, false), Event::ModifiersChanged(Modifiers::ALT), alt_down]);
    } else {
        s.release(p, PointerButton::Primary);
        assert!(s.app.view().unwrap().zoom > initial_zoom, "CONTROL: the old no-Alt stationary Zoom release zooms in");
        s.frame(vec![Event::ModifiersChanged(Modifiers::ALT), alt_down]);
    }
    assert!(s.app.view().unwrap().zoom > initial_zoom, "REGRESSION: a later Alt Down cannot turn the old no-Alt Zoom Up into zoom-out");
    let expected = (s.app.view().unwrap().center, s.app.view().unwrap().zoom);
    let moved = review_597489_divider_point(&s, 0.7);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - 0.7).abs() < 1e-6, "the Alt press still belongs to the fresh divider");
    s.release(moved, PointerButton::Primary);
    review_597704_assert_camera(&s, expected, "the fresh divider cannot modify the old zoom");
    assert_eq!(s.snapshot(), before);
    expected
}

synchronous_test!(review_597704_old_zoom_release_precedes_fresh_divider, {
    let _globals = Globals::new();
    // No release: crossing the seam cannot transfer an existing Zoom owner.
    let mut held = review_597704_zoom_scene();
    let p = held.rect().left_top() + vec2(70.0, 70.0);
    let up = held.point_on_divider();
    let expected = review_597704_zoom_expected(&held, p, up);
    let original = (held.app.view().unwrap().center, held.app.view().unwrap().zoom);
    held.press(p, PointerButton::Primary);
    held.frame(vec![Event::PointerMoved(up)]);
    review_597704_assert_camera(&held, original, "CONTROL: unreleased Zoom stays held");
    assert_eq!(held.fraction(), 0.5);
    held.release(up, PointerButton::Primary);
    review_597704_assert_camera(&held, expected, "CONTROL: held Zoom still completes at its own Up");
    review_597704_zoom_release_then_divider(false);
    review_597704_zoom_release_then_divider(true);
    let expected = review_597704_old_zoom_up_before_alt_down(false);
    let actual = review_597704_old_zoom_up_before_alt_down(true);
    assert!(actual.0.distance(expected.0) < 1e-6);
    assert!((actual.1 - expected.1).abs() < 1e-6, "coalesced old Up then Alt Down matches the split-frame no-Alt release");
});

fn review_597704_handoff_completed_zoom(coalesced: bool, space_after_up: bool) {
    let mut s = review_597704_zoom_scene();
    let document = s.snapshot();
    let start = s.point_on_divider();
    s.press(start, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(review_597489_divider_point(&s, 0.6))]);
    assert!((s.fraction() - 0.6).abs() < 1e-6, "CONTROL: a real divider owner is held");
    let old_up = review_597489_divider_point(&s, 0.65);
    let p = s.rect().left_top() + vec2(70.0, 70.0);
    let q = p + vec2(50.0, 25.0);
    let up = p + vec2(110.0, 65.0);
    let hover = p + vec2(180.0, 120.0);
    let expected = review_597704_zoom_expected(&s, p, up);
    if coalesced {
        let mut events =
            vec![review_597489_primary(old_up, false), review_597489_primary(p, true), Event::PointerMoved(q), review_597489_primary(up, false)];
        if space_after_up {
            events.push(key(Key::Space, true));
        }
        events.push(Event::PointerMoved(hover));
        s.frame(events);
    } else {
        s.release(old_up, PointerButton::Primary);
        assert!((s.fraction() - 0.65).abs() < 1e-6);
        s.press(p, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(q)]);
        s.release(up, PointerButton::Primary);
        let mut events = vec![];
        if space_after_up {
            events.push(key(Key::Space, true));
        }
        events.push(Event::PointerMoved(hover));
        s.frame(events);
    }
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    assert!((s.fraction() - 0.65).abs() < 1e-6, "old divider ends only at its own Up");
    review_597704_assert_camera(&s, expected, "REGRESSION: new Zoom finishes at its associated Up in this frame");
    s.frame(vec![Event::PointerMoved(hover + vec2(15.0, 10.0))]);
    review_597704_assert_camera(&s, expected, "button-up hover cannot finish a stale ZoomBox");
    assert_eq!(s.snapshot(), document);
}

fn review_597704_handoff_completed_tool(coalesced: bool) {
    let mut s = review_597489_divider_scene();
    let before = s.document();
    let history = s.app.session.active().unwrap().history.undo.len();
    let start = s.point_on_divider();
    s.press(start, PointerButton::Primary);
    let old_up = review_597489_divider_point(&s, 0.65);
    let p = review_597489_divider_point(&s, 0.8);
    let q = p + vec2(12.0, 5.0);
    let up = p + vec2(30.0, 12.0);
    let hover = p + vec2(50.0, 20.0);
    let before_bounds = s.bounds();
    let delta = s.xf().to_doc(up) - s.xf().to_doc(p);
    if coalesced {
        s.frame(vec![
            review_597489_primary(old_up, false),
            review_597489_primary(p, true),
            Event::PointerMoved(q),
            review_597489_primary(up, false),
            Event::PointerMoved(hover),
        ]);
    } else {
        s.release(old_up, PointerButton::Primary);
        s.press(p, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(q)]);
        s.release(up, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(hover)]);
    }
    let bounds = s.bounds();
    assert!(
        (bounds.x0 - before_bounds.x0 - delta.x).abs() < 1e-6,
        "the new ordinary Tool previews and commits its own final Up, not either owner's hover"
    );
    assert!((bounds.y0 - before_bounds.y0 - delta.y).abs() < 1e-6);
    assert!((s.fraction() - 0.65).abs() < 1e-6);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1);
    let after = s.document();
    s.frame(vec![Event::PointerMoved(hover + vec2(5.0, 5.0))]);
    assert_eq!(s.document(), after);
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.document(), before, "one Undo restores the whole new Tool edit");
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
}

fn review_597704_handoff_stationary_tool_click() {
    let mut s = review_597489_divider_scene();
    let before = s.document();
    let history = s.app.session.active().unwrap().history.undo.len();
    let start = s.point_on_divider();
    s.press(start, PointerButton::Primary);
    let old_up = review_597489_divider_point(&s, 0.65);
    let p = review_597489_divider_point(&s, 0.8);
    s.frame(vec![review_597489_primary(old_up, false), review_597489_primary(p, true), Event::PointerMoved(p), review_597489_primary(p, false)]);
    assert_eq!(
        s.app.session.active().unwrap().selection.objects,
        vec![s.rectangle],
        "CONTROL: the stationary ordinary click selects the real object"
    );
    assert_eq!(s.document(), before, "a same-point click must not invent a drag");
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    s.frame(vec![Event::PointerMoved(p + vec2(30.0, 12.0))]);
    assert_eq!(s.document(), before, "the completed click leaves no stale ordinary owner");
    assert!((s.fraction() - 0.65).abs() < 1e-6);
}

fn review_597704_handoff_view_only(coalesced: bool, tool: &str) -> (Point, f64) {
    let mut s = review_597704_zoom_scene();
    s.app.select_tool(tool);
    if tool == "zoom" {
        s.app.run("prefs.set", json!({"key":"animatedZoom","value":true})).unwrap();
        s.app.run("prefs.set", json!({"key":"gpuPerformance","value":true})).unwrap();
        assert!(s.app.session.prefs.animated_zoom && s.app.session.prefs.gpu_performance, "CONTROL: this is actual animated ZoomScrub");
    }
    s.idle();
    let document = s.snapshot();
    let initial = (s.app.view().unwrap().center, s.app.view().unwrap().zoom);
    let start = s.point_on_divider();
    s.press(start, PointerButton::Primary);
    let old_up = review_597489_divider_point(&s, 0.65);
    let p = s.rect().left_top() + vec2(70.0, 70.0);
    let up = p + vec2(70.0, 0.0);
    let hover = up + vec2(45.0, 25.0);
    if coalesced {
        s.frame(vec![
            review_597489_primary(old_up, false),
            review_597489_primary(p, true),
            Event::PointerMoved(up),
            review_597489_primary(up, false),
            Event::PointerMoved(hover),
        ]);
    } else {
        s.release(old_up, PointerButton::Primary);
        s.press(p, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(up)]);
        s.release(up, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(hover)]);
    }
    let expected = (s.app.view().unwrap().center, s.app.view().unwrap().zoom);
    if tool == "hand" {
        assert!(expected.0.distance(initial.0) > 1e-6, "the real Hand gesture applies its pan before release");
    } else {
        assert!((expected.1 - initial.1).abs() > 1e-6, "a moved animated Zoom gesture applies its scrub, not a click");
    }
    assert!((s.fraction() - 0.65).abs() < 1e-6);
    s.frame(vec![Event::PointerMoved(hover + vec2(15.0, 10.0))]);
    review_597704_assert_camera(&s, expected, "the released view-only owner cannot move on later hover");
    assert_eq!(s.snapshot(), document, "view-only completion changes no art, selection or history");
    expected
}

synchronous_test!(review_597704_divider_handoff_completes_new_zoom_at_its_own_up, {
    let _globals = Globals::new();
    review_597704_handoff_completed_zoom(false, false);
    review_597704_handoff_completed_zoom(true, false);
    review_597704_handoff_completed_zoom(false, true);
    review_597704_handoff_completed_zoom(true, true);
    review_597704_handoff_completed_tool(false);
    review_597704_handoff_completed_tool(true);
    review_597704_handoff_stationary_tool_click();
    for tool in ["hand", "zoom"] {
        let expected = review_597704_handoff_view_only(false, tool);
        let actual = review_597704_handoff_view_only(true, tool);
        assert!(actual.0.distance(expected.0) < 1e-6, "{tool}: associated-Up preview matches separate frames");
        assert!((actual.1 - expected.1).abs() < 1e-6, "{tool}: coalescing cannot change the final zoom");
    }
});

fn review_597704_completed_divider_then_down(coalesced: bool, on_divider: bool) {
    let mut s = review_597489_divider_scene();
    let before = s.snapshot();
    let first = review_597489_divider_point(&s, if on_divider { 0.5 } else { 0.2 });
    let up = review_597489_divider_point(&s, 0.7);
    let fresh = review_597489_divider_point(&s, 0.8);
    let fraction = if on_divider { 0.7 } else { 0.5 };
    if coalesced {
        s.frame(vec![review_597489_primary(first, true), review_597489_primary(up, false), review_597489_primary(fresh, true)]);
    } else {
        s.press(first, PointerButton::Primary);
        s.release(up, PointerButton::Primary);
        assert!((s.fraction() - fraction).abs() < 1e-6, "CONTROL: completed pair's separate-frame result");
        assert_eq!(s.snapshot(), before);
        s.press(fresh, PointerButton::Primary);
    }
    assert!((s.fraction() - fraction).abs() < 1e-6, "REGRESSION: earlier completed qualifying divider applies before latest Down");
    review_597489_ordinary_drag(&mut s, fraction);
}

synchronous_test!(review_597704_initial_completed_divider_precedes_latest_canvas_down, {
    let _globals = Globals::new();
    review_597704_completed_divider_then_down(false, true);
    review_597704_completed_divider_then_down(true, false); // Off-seam earlier press is not a divider.
    review_597704_completed_divider_then_down(true, true);
    review_597943_completed_divider_then_fresh_seam(false);
    review_597943_completed_divider_then_fresh_seam(true);
});

fn review_597943_camera(s: &Scene) -> (Point, f64, f64) {
    let v = s.app.view().unwrap();
    (v.center, v.zoom, v.rotation)
}

fn review_597943_camera_eq(s: &Scene, expected: (Point, f64, f64)) {
    let actual = review_597943_camera(s);
    assert!(actual.0.distance(expected.0) < 1e-6);
    assert!((actual.1 - expected.1).abs() < 1e-6);
    assert!((actual.2 - expected.2).abs() < 1e-6);
}

fn review_597943_old_pan_then_mapped(split_on: bool, coalesced: bool, divider_priority: bool) -> Value {
    let (mut s, handle) = review8211_mapped_scene();
    if !split_on {
        s.app.run("view.splitView", json!({})).unwrap();
    }
    if divider_priority {
        let rect = s.rect();
        s.app.view_mut().unwrap().split.fraction = (handle.x - rect.left()) / rect.width();
    }
    s.review8211_materials_frame(vec![]);
    assert_eq!(s.app.view().unwrap().split.enabled, split_on);
    let before = s.document();
    let graphics = review8211_graphics(&s);
    let camera = review_597943_camera(&s);
    let history = s.app.session.active().unwrap().history.undo.len();
    let selection = s.app.session.active().unwrap().selection.objects.clone();
    let start = s.rect().left_top() + vec2(25.0, 25.0);
    s.review8211_materials_frame(vec![key(Key::Space, true), Event::PointerMoved(start), review_597489_primary(start, true)]);
    assert!(s.ctx.input(|i| i.pointer.primary_down()));
    assert_eq!(s.app.session.tool_id(), "selection", "CONTROL: Space borrows Hand without replacing the Selection tool");
    assert_eq!(s.document(), before);
    review_597943_camera_eq(&s, camera);
    let up = handle + vec2(24.0, 0.0);
    if coalesced {
        s.review8211_materials_frame(vec![
            review_597489_primary(start, false),
            key(Key::Space, false),
            review_597489_primary(handle, true),
            Event::PointerMoved(up),
            review_597489_primary(up, false),
        ]);
    } else {
        s.review8211_materials_frame(vec![review_597489_primary(start, false), key(Key::Space, false)]);
        s.review8211_materials_frame(vec![review_597489_primary(handle, true)]);
        s.review8211_materials_frame(vec![Event::PointerMoved(up)]);
        s.review8211_materials_frame(vec![review_597489_primary(up, false)]);
    }
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    review_597943_camera_eq(&s, camera);
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    if divider_priority {
        let expected = (up.x - s.rect().left()) / s.rect().width();
        assert!((s.fraction() - expected).abs() < 1e-6, "the fresh Down at the updated divider keeps divider priority");
        assert_eq!(review8211_graphics(&s), graphics);
        assert_eq!(s.document(), before);
        assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    } else {
        assert_ne!(review8211_graphics(&s), graphics, "the fresh handle Down after an ordinary Up must map the graphic even with Split View OFF");
        assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1);
    }
    let after = s.document();
    let result = review8211_graphics(&s);
    s.review8211_materials_frame(vec![Event::PointerMoved(up + vec2(15.0, 10.0))]);
    assert_eq!(s.document(), after, "neither completed owner may resume on hover");
    if !divider_priority {
        s.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(s.document(), before, "one Undo restores only the fresh mapped edit");
        assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    }
    result
}

synchronous_test!(review_597943_ordinary_up_admits_fresh_mapped_handle_with_split_off, {
    let _globals = Globals::new();
    review_597943_old_pan_then_mapped(true, false, true);
    review_597943_old_pan_then_mapped(true, true, true);
    let on = review_597943_old_pan_then_mapped(true, false, false);
    assert_eq!(review_597943_old_pan_then_mapped(true, true, false), on);
    let off = review_597943_old_pan_then_mapped(false, false, false);
    assert_eq!(review_597943_old_pan_then_mapped(false, true, false), off);
});

fn review_597943_type_space(command: bool) {
    let mut s = Scene::new();
    s.app.session.prefs.placeholder_text = false;
    s.enable();
    s.command("view.splitPreview");
    s.app.run("prefs.set", json!({"key":"animatedZoom","value":false})).unwrap();
    s.app.select_tool("type");
    s.idle();
    let p = s.screen(Point::new(280.0, 40.0));
    s.press(p, PointerButton::Primary);
    s.release(p, PointerButton::Primary);
    assert!(s.app.session.tool_wants_text(), "CONTROL: the real Type tool is editing text");
    let modifiers = if command { Modifiers::COMMAND } else { Modifiers::NONE };
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers };
    s.frame(vec![Event::ModifiersChanged(modifiers), space(true)]);
    assert!(s.app.session.tool_wants_text());
    let camera = review_597943_camera(&s);
    let q = p + vec2(60.0, 40.0);
    s.frame(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers }]);
    s.frame(vec![Event::PointerMoved(q)]);
    s.frame(vec![Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: false, modifiers }]);
    if command {
        assert!(s.app.view().unwrap().zoom > camera.1, "CONTROL: Cmd+Space still borrows the real Zoom marquee while Type edits");
    } else {
        review_597943_camera_eq(&s, camera);
    }
    assert_eq!(s.app.session.tool_id(), "type");
    s.frame(vec![space(false), Event::ModifiersChanged(Modifiers::NONE)]);
}

synchronous_test!(review_597943_plain_space_while_typing_does_not_lend_pan, {
    let _globals = Globals::new();
    review_597943_type_space(true);
    review_597943_type_space(false);
});

fn review_597943_space_at_down(space_at_down: bool, coalesced: bool, mapped: bool) -> ((Point, f64, f64), f32, Value) {
    let (mut s, p) = if mapped {
        review8211_mapped_scene()
    } else {
        let mut s = Scene::new();
        s.enable();
        s.command("view.splitPreview");
        let p = s.point_on_divider();
        (s, p)
    };
    let camera = review_597943_camera(&s);
    let fraction = s.fraction();
    let graphics = if mapped { review8211_graphics(&s) } else { Value::Null };
    let document = s.document();
    let history = s.app.session.active().unwrap().history.undo.len();
    let q = p + vec2(60.0, 30.0);
    let first = if space_at_down { key(Key::Space, true) } else { review_597489_primary(p, true) };
    let second = if space_at_down { review_597489_primary(p, true) } else { key(Key::Space, true) };
    let send = |s: &mut Scene, events| {
        if mapped {
            s.review8211_materials_frame(events);
        } else {
            s.frame(events);
        }
    };
    if coalesced {
        let mut events = vec![Event::PointerMoved(p), first, second];
        if space_at_down {
            events.push(key(Key::Space, false));
        }
        events.extend([Event::PointerMoved(q), review_597489_primary(q, false)]);
        send(&mut s, events);
    } else {
        send(&mut s, vec![Event::PointerMoved(p), first]);
        send(&mut s, vec![second]);
        if space_at_down {
            send(&mut s, vec![key(Key::Space, false)]);
        }
        send(&mut s, vec![Event::PointerMoved(q)]);
        send(&mut s, vec![review_597489_primary(q, false)]);
    }
    let actual_graphics = if mapped { review8211_graphics(&s) } else { Value::Null };
    if space_at_down {
        assert!(s.app.view().unwrap().center.distance(camera.0) > 1e-6, "Space at Down owns the pan even after Space is released in the same frame");
        assert_eq!(s.fraction(), fraction);
        assert_eq!(actual_graphics, graphics, "a Space-pan press cannot be stolen by a mapped handle");
        assert_eq!(s.document(), document);
        assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    } else {
        review_597943_camera_eq(&s, camera);
        if mapped {
            assert_ne!(actual_graphics, graphics, "a later Space key cannot steal a mapped handle's original Down");
            assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1);
        } else {
            let expected = (q.x - s.rect().left()) / s.rect().width();
            assert!((s.fraction() - expected).abs() < 1e-6, "a later Space key cannot steal the original divider Down");
            assert_eq!(s.document(), document);
            assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
        }
    }
    let result = (review_597943_camera(&s), s.fraction(), actual_graphics);
    send(&mut s, vec![key(Key::Space, false)]);
    result
}

synchronous_test!(review_597943_divider_and_mapped_admission_use_space_at_each_down, {
    let _globals = Globals::new();
    for mapped in [false, true] {
        for space_at_down in [true, false] {
            let expected = review_597943_space_at_down(space_at_down, false, mapped);
            let actual = review_597943_space_at_down(space_at_down, true, mapped);
            assert!(actual.0.0.distance(expected.0.0) < 1e-6);
            assert!((actual.0.1 - expected.0.1).abs() < 1e-6);
            assert!((actual.1 - expected.1).abs() < 1e-6);
            assert_eq!(actual.2, expected.2);
        }
    }
});

fn review_597943_completed_divider_then_fresh_seam(coalesced: bool) {
    let mut s = review_597489_divider_scene();
    let before = s.snapshot();
    let first = review_597489_divider_point(&s, 0.5);
    let seam = review_597489_divider_point(&s, 0.7);
    if coalesced {
        s.frame(vec![review_597489_primary(first, true), review_597489_primary(seam, false), review_597489_primary(seam, true)]);
    } else {
        s.press(first, PointerButton::Primary);
        s.release(seam, PointerButton::Primary);
        s.press(seam, PointerButton::Primary);
    }
    assert!((s.fraction() - 0.7).abs() < 1e-6);
    let moved = review_597489_divider_point(&s, 0.85);
    s.frame(vec![Event::PointerMoved(moved)]);
    assert!((s.fraction() - 0.85).abs() < 1e-6, "the fresh Down70 owns the updated seam, not the original Down50");
    s.release(moved, PointerButton::Primary);
    assert_eq!(s.snapshot(), before);
}

// These witnesses use only real canvas frames; the original registrations remain above unchanged.
fn review_598046_name_scene() -> (Scene, Pos2) {
    let mut s = Scene::new();
    s.app.run("file.new", json!({"width":320,"height":240,"artboards":2})).unwrap();
    let v = s.app.view_mut().unwrap();
    v.fitted = true;
    v.center = Point::new(160.0, 120.0);
    v.zoom = 1.5;
    v.artboard = 1;
    s.enable();
    fn label(shape: &Shape) -> Option<Pos2> {
        match shape {
            Shape::Vec(shapes) => shapes.iter().find_map(label),
            Shape::Text(text) if text.galley.job.text.starts_with("01 -") => Some(text.pos + text.galley.rect.center().to_vec2()),
            _ => None,
        }
    }
    let name = s.output.shapes.iter().find_map(|shape| label(&shape.shape)).expect("CONTROL: actual first artboard label is painted");
    assert!(s.rect().contains(name));
    assert!((name.x - s.point_on_divider().x).abs() > 20.0);
    (s, name)
}

fn review_598046_name_then_divider(coalesced: bool) {
    let (mut s, name) = review_598046_name_scene();
    let before = s.snapshot();
    let initial = s.fraction();
    let divider = s.point_on_divider();
    let up = review_597489_divider_point(&s, 0.7);
    s.press(name, PointerButton::Primary);
    assert_eq!(s.app.view().unwrap().artboard, 1, "CONTROL: the name is selected only at its Up");
    s.frame(vec![Event::PointerMoved(divider)]);
    assert_eq!(s.fraction(), initial, "CONTROL: a held name press does not move the divider");
    assert_eq!(s.snapshot(), before);
    if coalesced {
        s.frame(vec![review_597489_primary(name, false), review_597489_primary(divider, true)]);
    } else {
        s.release(name, PointerButton::Primary);
        assert_eq!(s.app.view().unwrap().artboard, 0, "CONTROL: the separate name release selects the first artboard");
        s.press(divider, PointerButton::Primary);
    }
    assert_eq!(s.app.view().unwrap().artboard, 0, "REGRESSION: the old name completes before the new divider Down");
    s.frame(vec![Event::PointerMoved(up)]);
    assert!((s.fraction() - 0.7).abs() < 1e-6, "REGRESSION: the fresh divider Down owns its move");
    s.release(up, PointerButton::Primary);
    assert_eq!(s.snapshot(), before, "neither name selection nor divider motion changes art, object selection or history");
    s.app.view_mut().unwrap().artboard = 1;
    let blank = s.screen(Point::new(-20.0, 100.0));
    assert!(s.rect().contains(blank));
    s.click(blank);
    assert_eq!(s.app.view().unwrap().artboard, 1, "a later release cannot finish a stale name press");
}

synchronous_test!(review_598046_name_up_precedes_fresh_divider_down, {
    let _globals = Globals::new();
    review_598046_name_then_divider(false);
    review_598046_name_then_divider(true);
});

fn review_598046_zoom_up_ignores_later_input(coalesced: bool) -> (Point, f64, f64) {
    let mut s = review_597704_zoom_scene();
    let snapshot = s.snapshot();
    let original = review_597943_camera(&s);
    let p = s.rect().left_top() + vec2(100.0, 100.0);
    let q = p + vec2(50.0, 50.0);
    let up = p + vec2(80.0, 80.0);
    let hover = up + vec2(35.0, 15.0);
    let expected = review_597704_zoom_expected(&s, p, up);
    assert!(s.rect().contains(hover));
    s.press(p, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(q)]);
    review_597943_camera_eq(&s, original);
    if coalesced {
        // No PointerMoved(up): a real button-up carries the final position itself.
        s.frame(vec![review_597489_primary(up, false), key(Key::Space, true), Event::PointerMoved(hover)]);
    } else {
        s.release(up, PointerButton::Primary);
        review_597704_assert_camera(&s, expected, "CONTROL: ordinary release-only Zoom completes at Up");
        s.frame(vec![key(Key::Space, true), Event::PointerMoved(hover)]);
    }
    review_597704_assert_camera(&s, expected, "REGRESSION: post-Up Space and hover cannot move the completed marquee");
    let camera = review_597943_camera(&s);
    s.frame(vec![Event::PointerMoved(hover + vec2(12.0, 8.0)), key(Key::Space, false)]);
    review_597943_camera_eq(&s, camera);
    assert_eq!(s.snapshot(), snapshot);
    camera
}

synchronous_test!(review_598046_ordinary_zoom_release_cuts_off_post_up_input, {
    let _globals = Globals::new();
    let expected = review_598046_zoom_up_ignores_later_input(false);
    let actual = review_598046_zoom_up_ignores_later_input(true);
    assert!(actual.0.distance(expected.0) < 1e-6);
    assert!((actual.1 - expected.1).abs() < 1e-6);
    assert!((actual.2 - expected.2).abs() < 1e-6);
});

fn review_598046_current_mapped_handle(s: &Scene) -> Pos2 {
    let doc = &s.app.session.active().unwrap().doc;
    let node = doc.node(s.rectangle).unwrap();
    let frame = vectorcraft_effects::graphic_frame_at(Some(doc), node, None, 0).expect("CONTROL: mapped plane still has a graphic frame");
    let list = vectorcraft_effects::mapped_graphics(&node.appearance.effects[0].params);
    let (at, visible) = frame.handle(&list[0]).expect("CONTROL: mapped graphic still has a handle");
    assert!(visible);
    s.screen(at)
}

fn review_598046_pan_final_up_then_handle(expected: Option<((Point, f64, f64), Pos2)>) -> ((Point, f64, f64), Pos2, Value) {
    let (mut s, _) = review8211_mapped_scene();
    let document = s.document();
    let original_graphics = review8211_graphics(&s);
    let selection = s.app.session.active().unwrap().selection.objects.clone();
    let history = s.app.session.active().unwrap().history.undo.len();
    let fraction = s.fraction();
    let start = s.rect().left_top() + vec2(100.0, 80.0);
    let q = start + vec2(12.0, 4.0);
    let old_up = start + vec2(-40.0, 16.0);
    s.review8211_materials_frame(vec![key(Key::Space, true), Event::PointerMoved(start), review_597489_primary(start, true)]);
    s.review8211_materials_frame(vec![Event::PointerMoved(q)]);
    let preview = review_597943_camera(&s);
    let preview_handle = review_598046_current_mapped_handle(&s);
    let (camera, handle) = if let Some(expected) = expected {
        expected
    } else {
        s.review8211_materials_frame(vec![Event::PointerMoved(old_up)]);
        let camera = review_597943_camera(&s);
        s.review8211_materials_frame(vec![review_597489_primary(old_up, false), key(Key::Space, false)]);
        review_597943_camera_eq(&s, camera);
        (camera, review_598046_current_mapped_handle(&s))
    };
    assert!(camera.0.distance(preview.0) > 1e-6, "CONTROL: the last pan release contributes real remaining motion");
    assert!(handle.distance(preview_handle) > 20.0, "CONTROL: using the old camera would miss the new handle press");
    assert!(s.rect().contains(handle) && (handle.x - s.point_on_divider().x).abs() > 8.0);
    let moved = handle + vec2(10.0, 4.0);
    let up = handle + vec2(24.0, 0.0);
    let hover = up + vec2(17.0, 13.0);
    let expected_graphics = {
        // The separate-frame camera supplies the real final coordinate mapping for both variants.
        let xf = canvas::Xf::at(
            s.rect(),
            &vectorcraft_ui_egui::state::View { center: camera.0, zoom: camera.1, rotation: camera.2, ..*s.app.view().unwrap() },
            s.ctx.zoom_factor(),
        );
        let doc = &s.app.session.active().unwrap().doc;
        let node = doc.node(s.rectangle).unwrap();
        let frame = vectorcraft_effects::graphic_frame_at(Some(doc), node, None, 0).unwrap();
        let list = vectorcraft_effects::mapped_graphics(&node.appearance.effects[0].params);
        let (at, _) = frame.handle(&list[0]).unwrap();
        let grab = xf.to_doc(handle) - at;
        let (x, y) = frame.offset_at(list[0].face, xf.to_doc(up) - grab).expect("CONTROL: mapped Up lies on the face");
        let mut graphics = original_graphics.clone();
        graphics[0]["x"] = json!((x * 10.0).round() / 10.0);
        graphics[0]["y"] = json!((y * 10.0).round() / 10.0);
        graphics
    };
    if expected.is_some() {
        s.review8211_materials_frame(vec![
            Event::PointerMoved(old_up),
            review_597489_primary(old_up, false),
            key(Key::Space, false),
            review_597489_primary(handle, true),
            Event::PointerMoved(moved),
            review_597489_primary(up, false),
            Event::PointerMoved(hover),
        ]);
    } else {
        s.review8211_materials_frame(vec![review_597489_primary(handle, true)]);
        s.review8211_materials_frame(vec![Event::PointerMoved(up)]);
        s.review8211_materials_frame(vec![review_597489_primary(up, false), Event::PointerMoved(hover)]);
    }
    review_597943_camera_eq(&s, camera);
    let graphics = review8211_graphics(&s);
    assert_eq!(
        graphics, expected_graphics,
        "REGRESSION: fresh mapped handle uses the final old camera and commits at its own Up, not Q or later hover"
    );
    assert_ne!(graphics, original_graphics);
    assert_eq!(s.app.session.active().unwrap().selection.objects, selection);
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history + 1);
    assert_eq!(s.fraction(), fraction);
    let after = s.document();
    s.review8211_materials_frame(vec![Event::PointerMoved(hover + vec2(15.0, 10.0))]);
    review_597943_camera_eq(&s, camera);
    assert_eq!(s.document(), after);
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.document(), document, "one Undo restores the mapped edit, not view navigation");
    assert_eq!(s.app.session.active().unwrap().history.undo.len(), history);
    (camera, handle, graphics)
}

fn review_598046_rotate_final_up(coalesced: bool) -> (Point, f64, f64) {
    let mut s = review_597704_zoom_scene();
    s.app.select_tool("rotateView");
    s.idle();
    let before = s.snapshot();
    let center = s.rect().center();
    let start = center + vec2(60.0, 0.0);
    let q = center + vec2(55.0, 15.0);
    let up = center + vec2(10.0, 55.0);
    s.press(start, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(q)]);
    let preview = review_597943_camera(&s);
    let divider = s.point_on_divider();
    if coalesced {
        s.frame(vec![review_597489_primary(up, false), review_597489_primary(divider, true)]);
    } else {
        s.frame(vec![Event::PointerMoved(up)]);
        s.release(up, PointerButton::Primary);
        s.press(divider, PointerButton::Primary);
    }
    let final_camera = review_597943_camera(&s);
    assert!((final_camera.2 - preview.2).abs() > 1.0, "REGRESSION: final rotation is at Up rather than the previous held point");
    let divider_up = review_597489_divider_point(&s, 0.7);
    s.frame(vec![Event::PointerMoved(divider_up)]);
    s.release(divider_up, PointerButton::Primary);
    review_597943_camera_eq(&s, final_camera);
    assert!((s.fraction() - 0.7).abs() < 1e-6);
    assert_eq!(s.snapshot(), before);
    final_camera
}

fn review_598046_scrub_final_up(coalesced: bool) -> (Point, f64, f64) {
    let mut s = review_597704_zoom_scene();
    s.app.run("prefs.set", json!({"key":"animatedZoom","value":true})).unwrap();
    s.app.run("prefs.set", json!({"key":"gpuPerformance","value":true})).unwrap();
    s.idle();
    assert!(s.app.session.prefs.animated_zoom && s.app.session.prefs.gpu_performance);
    let before = s.snapshot();
    let start = s.rect().left_top() + vec2(100.0, 100.0);
    let q = start + vec2(20.0, 0.0);
    let up = start + vec2(55.0, 0.0);
    s.press(start, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(q)]);
    let preview = review_597943_camera(&s);
    let divider = s.point_on_divider();
    if coalesced {
        s.frame(vec![review_597489_primary(up, false), review_597489_primary(divider, true)]);
    } else {
        s.frame(vec![Event::PointerMoved(up)]);
        s.release(up, PointerButton::Primary);
        s.press(divider, PointerButton::Primary);
    }
    let camera = review_597943_camera(&s);
    assert!(camera.1 > preview.1, "REGRESSION: only the remaining physical scrub motion through Up is applied");
    s.release(divider, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(up + vec2(50.0, 20.0))]);
    review_597943_camera_eq(&s, camera);
    assert_eq!(s.snapshot(), before);
    camera
}

fn review_598046_stationary_scrub_release() {
    let mut s = review_597704_zoom_scene();
    s.app.run("prefs.set", json!({"key":"animatedZoom","value":true})).unwrap();
    s.app.run("prefs.set", json!({"key":"gpuPerformance","value":true})).unwrap();
    s.idle();
    let before = s.snapshot();
    let start = s.rect().left_top() + vec2(100.0, 100.0);
    let original = review_597943_camera(&s);
    s.press(start, PointerButton::Primary);
    for _ in 0..3 {
        s.time += 0.5;
        s.frame(vec![Event::PointerMoved(start)]);
    }
    let held = review_597943_camera(&s);
    assert!(held.1 > original.1, "CONTROL: the real stationary held ZoomScrub has passed its hold delay and zooms");
    s.frame(vec![review_597489_primary(start, false)]);
    review_597943_camera_eq(&s, held);
    s.frame(vec![Event::PointerMoved(start + vec2(50.0, 20.0))]);
    review_597943_camera_eq(&s, held);
    assert_eq!(s.snapshot(), before, "release adds no hold-time tick or document edit");
}

synchronous_test!(review_598046_old_view_final_up_precedes_new_hit_testing, {
    let _globals = Globals::new();
    let (camera, handle, expected) = review_598046_pan_final_up_then_handle(None);
    let (_, _, actual) = review_598046_pan_final_up_then_handle(Some((camera, handle)));
    assert_eq!(actual, expected);
    let expected = review_598046_rotate_final_up(false);
    let actual = review_598046_rotate_final_up(true);
    assert!(actual.0.distance(expected.0) < 1e-6);
    assert!((actual.1 - expected.1).abs() < 1e-6);
    assert!((actual.2 - expected.2).abs() < 1e-6);
    let expected = review_598046_scrub_final_up(false);
    let actual = review_598046_scrub_final_up(true);
    assert!(actual.0.distance(expected.0) < 1e-6);
    assert!((actual.1 - expected.1).abs() < 1e-6);
    assert!((actual.2 - expected.2).abs() < 1e-6);
    review_598046_stationary_scrub_release();
});

fn review_598151_outline_edge_ink(s: &Scene, comparison: TextureId) {
    assert_eq!(s.app.view().unwrap().split.comparison, SplitMode::Outline);
    let bounds = s.bounds();
    let edge = s.screen(Point::new(bounds.x1, (bounds.y0 + bounds.y1) * 0.5));
    assert!(s.rect().contains(edge) && edge.x > s.point_on_divider().x + 8.0, "CONTROL: the unobstructed right edge is in the Outline half");
    let mut ink = false;
    for dx in [-1.0, 0.0, 1.0] {
        for dy in [-1.0, 0.0, 1.0] {
            let c = s.raster_at(comparison, s.xf().to_doc(edge + vec2(dx, dy)));
            ink |= c.a() > 80 && c.r() < 200 && c.g() < 200 && c.b() < 200 && c.r().abs_diff(c.g()) < 12 && c.g().abs_diff(c.b()) < 12;
        }
    }
    assert!(ink, "Outline uploads actual nontransparent neutral edge ink, not an empty texture");
}

fn review_598151_hand_before_divider(coalesced: bool, moved: bool) -> (Point, f64, f64) {
    let mut s = review_597704_zoom_scene();
    s.app.select_tool("hand");
    s.idle();
    assert_eq!(s.app.session.tool_id(), "hand");
    let document = s.snapshot();
    let initial = review_597943_camera(&s);
    let start = s.rect().left_top() + vec2(100.0, 100.0);
    let up = start + if moved { vec2(40.0, 20.0) } else { egui::Vec2::ZERO };
    let divider = s.point_on_divider();
    assert!(s.rect().contains(up) && up.x < divider.x - 8.0);
    if coalesced {
        s.frame(vec![
            Event::PointerMoved(start),
            review_597489_primary(start, true),
            Event::PointerMoved(up),
            review_597489_primary(up, false),
            Event::PointerMoved(divider),
            review_597489_primary(divider, true),
        ]);
    } else {
        s.press(start, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(up)]);
        s.release(up, PointerButton::Primary);
        if moved {
            assert!(s.app.view().unwrap().center.distance(initial.0) > 1.0, "CONTROL: a separate completed Hand gesture pans");
        } else {
            review_597943_camera_eq(&s, initial);
        }
        s.press(divider, PointerButton::Primary);
    }
    let completed = review_597943_camera(&s);
    if !moved {
        review_597943_camera_eq(&s, initial);
    }
    let divider_up = review_597489_divider_point(&s, 0.65);
    s.frame(vec![Event::PointerMoved(divider_up)]);
    assert!((s.fraction() - 0.65).abs() < 1e-6, "the later fresh press owns the divider");
    review_597943_camera_eq(&s, completed);
    s.release(divider_up, PointerButton::Primary);
    s.frame(vec![Event::PointerMoved(start)]);
    review_597943_camera_eq(&s, completed);
    assert!((s.fraction() - 0.65).abs() < 1e-6, "button-up hover cannot continue the completed divider");
    assert_eq!(s.app.session.tool_id(), "hand");
    assert_eq!(s.snapshot(), document, "Hand and divider gestures preserve art, selection and history");
    completed
}

synchronous_test!(review_598151_complete_hand_episode_precedes_fresh_divider_down, {
    let _globals = Globals::new();
    review_598151_hand_before_divider(false, false);
    review_598151_hand_before_divider(true, false);
    let expected = review_598151_hand_before_divider(false, true);
    let actual = review_598151_hand_before_divider(true, true);
    assert!(actual.0.distance(expected.0) < 1e-6, "the complete earlier Hand episode must pan before the later divider Down");
    assert!((actual.1 - expected.1).abs() < 1e-6);
    assert!((actual.2 - expected.2).abs() < 1e-6);
});

fn review_598151_scrub_scene() -> Scene {
    let mut s = review_597704_zoom_scene();
    s.app.run("prefs.set", json!({"key":"gpuPerformance","value":true})).unwrap();
    s.app.run("prefs.set", json!({"key":"animatedZoom","value":true})).unwrap();
    s.idle();
    assert!(s.app.session.prefs.gpu_performance && s.app.session.prefs.animated_zoom);
    s
}

fn review_598151_scrub_idle(s: &mut Scene, before: &Value, camera: (Point, f64, f64), at: Pos2) {
    assert!(!s.ctx.input(|i| i.pointer.primary_down()));
    s.frame(vec![Event::PointerMoved(at + vec2(17.0, 13.0))]);
    s.idle();
    review_597943_camera_eq(s, camera);
    assert_eq!(s.snapshot(), *before, "view-only scrub/click gestures preserve document, selection and history");
    assert_eq!(s.fraction(), 0.5);
    assert_eq!(s.app.session.tool_id(), "zoom");
}

fn review_598151_stationary_scrub(coalesced: bool, earlier_motion: bool) -> (Point, f64, f64) {
    let mut s = review_598151_scrub_scene();
    let before = s.snapshot();
    let initial = review_597943_camera(&s);
    let at = s.rect().left_top() + vec2(100.0, 100.0);
    if coalesced {
        let mut events = Vec::new();
        if earlier_motion {
            // These are hover events BEFORE the new Zoom owner exists.
            events.push(Event::PointerMoved(s.rect().left_top() - vec2(20.0, 20.0)));
            events.push(Event::PointerMoved(at + vec2(90.0, 0.0)));
        }
        events.extend([Event::PointerMoved(at), review_597489_primary(at, true), review_597489_primary(at, false)]);
        s.frame(events);
    } else {
        s.click(at);
    }
    let clicked = review_597943_camera(&s);
    assert!(clicked.1 > initial.1, "CONTROL: a stationary Animated Zoom gesture remains an ordinary zoom-in click");
    review_598151_scrub_idle(&mut s, &before, clicked, at);
    clicked
}

fn review_598151_scrub_out_and_back(coalesced: bool, return_with_up: bool) {
    let mut s = review_598151_scrub_scene();
    let before = s.snapshot();
    let initial = review_597943_camera(&s);
    let at = s.rect().left_top() + vec2(100.0, 100.0);
    let out = at + vec2(40.0, 0.0);
    if coalesced {
        s.frame(vec![
            Event::PointerMoved(at),
            review_597489_primary(at, true),
            Event::PointerMoved(out),
            Event::PointerMoved(at),
            review_597489_primary(at, false),
        ]);
    } else {
        s.press(at, PointerButton::Primary);
        s.frame(vec![Event::PointerMoved(out)]);
        assert!(s.app.view().unwrap().zoom > initial.1, "CONTROL: actual held horizontal motion zooms before returning");
        if return_with_up {
            s.frame(vec![Event::PointerMoved(at), review_597489_primary(at, false)]);
        } else {
            s.frame(vec![Event::PointerMoved(at)]);
            review_597943_camera_eq(&s, initial);
            s.release(at, PointerButton::Primary);
        }
    }
    review_597943_camera_eq(&s, initial);
    review_598151_scrub_idle(&mut s, &before, initial, at);
}

synchronous_test!(review_598151_animated_zoom_owned_excursion_is_not_a_click, {
    let _globals = Globals::new();
    let expected = review_598151_stationary_scrub(false, false);
    let ordinary = review_598151_stationary_scrub(true, false);
    let earlier = review_598151_stationary_scrub(true, true);
    for actual in [ordinary, earlier] {
        assert!(actual.0.distance(expected.0) < 1e-6, "earlier hover cannot change the new stationary click's camera");
        assert!((actual.1 - expected.1).abs() < 1e-6);
        assert!((actual.2 - expected.2).abs() < 1e-6);
    }
    review_598151_scrub_out_and_back(false, false);
    review_598151_scrub_out_and_back(false, true);
    review_598151_scrub_out_and_back(true, false);
});
