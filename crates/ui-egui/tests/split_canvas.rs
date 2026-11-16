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
