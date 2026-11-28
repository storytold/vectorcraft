//! The Contextual Task Bar's offer for each kind of selection and tool, and what its controls do.
//! The offer rule is asserted on `task_bar::offered`, in three tables (selection kinds, type, tools);
//! the controls are driven by name through the framework's `Fixture`: open paths offer Join and Edit
//! Path, compound paths and clipping masks their Release, type converts between point and area type,
//! a basic shape shows its properties in place of Lock, the Smooth tool its Fidelity, and Type and
//! the Type tools the font controls.

use std::cell::{RefCell, RefMut};
use std::rc::Rc;

use craft::ui::testing::{Fixture, Target};
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2, vec2};
use serde_json::json;
use vectorcraft_doc::text::TextKind;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::canvas::task_bar::{Offer, chip_paints, images_with_other_art, offered};
use crate::dialogs::simplify_slider::StripKind;
use crate::{VectorcraftApp, canvas};

/// The bar's access name, as the framework's `TaskBar` gives it.
pub(crate) const BAR: &str = "Contextual Task Bar";

/// The room of a canvas fixture.
const CANVAS_ROOM: Vec2 = vec2(1000.0, 700.0);

/// An app shown in a framework fixture. The fixture's closure owns what it shows, so the app lives in
/// an `Rc<RefCell<..>>` that [`driven`] moves it into and back out of.
pub(crate) struct Rig {
    app: Rc<RefCell<VectorcraftApp>>,
    /// The fixture, for the input a test sends beyond a click.
    pub(crate) fx: Fixture,
}

impl Rig {
    /// The shown app, for commands and reads between frames.
    pub(crate) fn app(&self) -> RefMut<'_, VectorcraftApp> {
        self.app.borrow_mut()
    }

    /// Runs the commands the frames since the last call fired, as the window runs them, and gives the fixture the
    /// app's input for the next frame.
    pub(crate) fn run_fired(&mut self) {
        crate::testing::fixture_input(&mut self.app.borrow_mut(), &mut self.fx);
    }

    /// Frames until the app is at rest, with the commands they fired run.
    fn settle(&mut self) -> &mut Fixture {
        self.run_fired();
        self.fx.settle();
        self.run_fired();
        &mut self.fx
    }

    /// The node named `name` once the frames have settled.
    pub(crate) fn target(&mut self, name: &str) -> Option<Target> {
        self.settle().by_label(name)
    }

    /// Clicks the node named `name`.
    pub(crate) fn click(&mut self, name: &str) {
        let target = self.target(name).unwrap_or_else(|| panic!("no node named {name:?}"));
        self.fx.click(&target);
        self.settle();
    }

    /// The runs of text the latest frame painted, with their rects.
    fn texts(&mut self) -> Vec<(String, Rect)> {
        self.settle().texts().iter().map(|t| (t.text.clone(), t.rect)).collect()
    }

    /// Where `text` is painted: a run of it, or a field's value painted as its number with the unit as the next run
    /// on its row ("10 pt" as "10" then "pt").
    fn painted(&mut self, text: &str) -> Vec<Rect> {
        let texts = self.texts();
        let unit_after = |r: &Rect, unit: &str| {
            texts
                .iter()
                .filter(|(_, u)| u.left() >= r.right() && (u.center().y - r.center().y).abs() < 4.0)
                .min_by(|a, b| a.1.left().total_cmp(&b.1.left()))
                .is_some_and(|(t, _)| t == unit)
        };
        // "10 pt" and "360°": the number, then its unit.
        let split = text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-'))).map(|i| (&text[..i], text[i..].trim()));
        let parts = split.filter(|(number, unit)| !number.is_empty() && !unit.is_empty());
        texts.iter().filter(|(t, r)| t == text || parts.is_some_and(|(number, unit)| t == number && unit_after(r, unit))).map(|(_, r)| *r).collect()
    }

    fn has_text(&mut self, text: &str) -> bool {
        !self.painted(text).is_empty()
    }

    fn count_text(&mut self, text: &str) -> usize {
        self.painted(text).len()
    }

    /// Clicks the first painted run of `text`, or with `last` the last: an open list is painted after the
    /// closed menu that shows the same name.
    fn click_text(&mut self, text: &str, last: bool) {
        let centers: Vec<Pos2> = self.painted(text).iter().map(Rect::center).collect();
        let at = if last { centers.last() } else { centers.first() };
        let at = *at.unwrap_or_else(|| panic!("no `{text}` painted"));
        self.fx.click_at(at);
        self.settle();
    }

    /// Where the field painted right of `label`, on its row, centers.
    fn field_after(&mut self, label: &str) -> Pos2 {
        let texts = self.texts();
        let l = texts.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("no `{label}` in {texts:?}")).1;
        texts
            .iter()
            // Tops, not centres: a label's painted rect can reach below its row.
            .filter(|(_, r)| r.left() > l.right() && (r.top() - l.top()).abs() < 8.0)
            .min_by(|a, b| a.1.left().total_cmp(&b.1.left()))
            .unwrap_or_else(|| panic!("no field after `{label}` in {texts:?}"))
            .1
            .center()
    }

    /// Types `value` into the field painting `field` and presses Enter.
    fn type_into(&mut self, field: &str, value: &str) {
        // The first in reading order: the frame paints the fields in an order of its own.
        let first = self.painted(field).into_iter().min_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
        let at = first.unwrap_or_else(|| panic!("no `{field}` painted")).center();
        self.enter_value(at, value);
    }

    fn enter_value(&mut self, at: Pos2, value: &str) {
        self.fx.click_at(at);
        // A click on a field that already has the keyboard (a popover gives its first field the keyboard as it
        // opens) places the caret: the whole field is selected, so the text typed replaces it.
        self.fx.key(Key::A, Modifiers::COMMAND);
        self.fx.type_text(value);
        self.fx.key(Key::Enter, Modifiers::NONE);
        self.settle();
    }
}

/// Shows `show` in a fixture of `room` for the length of `f`; the test keeps its own `app`.
pub(super) fn driven<R>(app: &mut VectorcraftApp, room: Vec2, show: fn(&mut VectorcraftApp, &mut Ui), f: impl FnOnce(&mut Rig) -> R) -> R {
    let held = Rc::new(RefCell::new(std::mem::replace(app, VectorcraftApp::new(Session::new(), Default::default()))));
    let shown = Rc::clone(&held);
    let mut fx = Fixture::new(move |ui| show(&mut shown.borrow_mut(), ui));
    fx.room(room);
    crate::testing::install(fx.ctx());
    // The window's dialog host is the fixture's, as a real window's frame gives the app its context.
    held.borrow_mut().adopt_context(fx.ctx());
    let mut rig = Rig { app: held, fx };
    rig.run_fired();
    let out = f(&mut rig);
    rig.run_fired();
    let Rig { app: held, fx } = rig;
    drop(fx);
    *app = Rc::into_inner(held).expect("the fixture's closure is gone").into_inner();
    out
}

/// [`driven`] on the canvas.
pub(crate) fn on_canvas<R>(app: &mut VectorcraftApp, f: impl FnOnce(&mut Rig) -> R) -> R {
    driven(
        app,
        CANVAS_ROOM,
        |app, ui| {
            canvas::show(app, ui);
        },
        f,
    )
}

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn id(v: serde_json::Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn kind(app: &VectorcraftApp, id: u64) -> NodeKind {
    app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind.clone()
}

fn select(app: &mut VectorcraftApp, ids: &[u64]) {
    app.run("select.set", json!({"ids": ids})).unwrap();
}

/// Clicks the canvas's node named `name`.
pub(crate) fn click(app: &mut VectorcraftApp, name: &str) {
    on_canvas(app, |rig| rig.click(name));
}

/// The bar's offer once the canvas is laid out.
pub(crate) fn offer(app: &mut VectorcraftApp) -> Option<Offer> {
    on_canvas(app, |rig| {
        rig.fx.settle();
        offered(&mut rig.app())
    })
}

/// Whether the bar shows the action labelled `what` or the chip with id `what`.
fn shows(o: &Offer, what: &str) -> bool {
    o.actions.iter().any(|a| a.0 == what) || o.chips.iter().any(|c| c.id(o.shape.is_some()) == what)
}

/// Whether the bar offers `what` by label, by Bar Actions id or as a chip, hidden or not.
fn offers(o: &Offer, what: &str) -> bool {
    shows(o, what) || o.listed.iter().any(|l| l.0 == what || l.1 == Some(what))
}

fn names_of(o: &Offer) -> Vec<&str> {
    o.actions.iter().map(|a| a.0).chain(o.chips.iter().map(|c| c.id(o.shape.is_some()))).chain(o.listed.iter().filter_map(|l| l.1)).collect()
}

/// Turns the bar's action or chip `action` on or off (`window.taskBar.actions`).
fn show_action(app: &mut VectorcraftApp, action: &str, visible: bool) {
    let r = app.run("window.taskBar.actions", json!({"action": action, "visible": visible})).unwrap();
    assert_eq!(r, json!({"action": action, "visible": visible}));
}

/// A table row: a setup, and what the bar then offers (`None`: no bar).
type Setup = Box<dyn Fn(&mut VectorcraftApp)>;

struct Row {
    case: String,
    setup: Setup,
    expect: Option<Expect>,
}

struct Expect {
    has: &'static [&'static str],
    lacks: &'static [&'static str],
    fonts: Option<bool>,
}

fn row(case: impl Into<String>, setup: impl Fn(&mut VectorcraftApp) + 'static, has: &'static [&'static str], lacks: &'static [&'static str]) -> Row {
    Row { case: case.into(), setup: Box::new(setup), expect: Some(Expect { has, lacks, fonts: None }) }
}

fn without(case: impl Into<String>, setup: impl Fn(&mut VectorcraftApp) + 'static) -> Row {
    Row { case: case.into(), setup: Box::new(setup), expect: None }
}

impl Row {
    /// The row also says whether the bar shows the font controls.
    fn fonts(mut self, fonts: bool) -> Self {
        if let Some(expect) = &mut self.expect {
            expect.fonts = Some(fonts);
        }
        self
    }
}

/// Runs every row on a new document under the Selection tool: the offer is what the row says, and the
/// bar's node is there exactly when the offer is.
fn check(rows: Vec<Row>) {
    for Row { case, setup, expect } in rows {
        let mut app = app();
        app.run("tool.select", json!({"tool": "selection"})).unwrap();
        setup(&mut app);
        let (offer, drawn) = on_canvas(&mut app, |rig| {
            let drawn = rig.target(BAR).is_some();
            (offered(&mut rig.app()), drawn)
        });
        assert_eq!(drawn, offer.is_some(), "{case}: the bar's node follows the offer");
        match (expect, offer) {
            (None, None) => {}
            (None, Some(o)) => panic!("{case}: no bar expected, offered {:?}", names_of(&o)),
            (Some(_), None) => panic!("{case}: a bar expected"),
            (Some(e), Some(o)) => {
                for what in e.has {
                    assert!(offers(&o, what), "{case}: {what} in {:?}", names_of(&o));
                }
                for what in e.lacks {
                    assert!(!offers(&o, what), "{case}: {what} is left out of {:?}", names_of(&o));
                }
                if let Some(fonts) = e.fonts {
                    assert_eq!(o.fonts, fonts, "{case}: the font controls");
                }
            }
        }
    }
}

fn two_shapes(app: &mut VectorcraftApp) -> [u64; 2] {
    let a = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 120, "height": 100})).unwrap());
    let b = id(app.run("shape.ellipse", json!({"x": 70, "y": 60, "width": 50, "height": 50})).unwrap());
    [a, b]
}

fn open_line(app: &mut VectorcraftApp) -> u64 {
    id(app.run("path.create", json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}]})).unwrap())
}

fn compound(app: &mut VectorcraftApp) -> u64 {
    let ids = two_shapes(app);
    select(app, &ids);
    id(app.run("object.compoundPath.make", json!({})).unwrap())
}

fn clipping_group(app: &mut VectorcraftApp) -> u64 {
    let ids = two_shapes(app);
    select(app, &ids);
    let g = id(app.run("object.clippingMask.make", json!({})).unwrap());
    select(app, &[g]);
    g
}

/// Type on a path, along an open line or around a closed circle.
fn path_type(app: &mut VectorcraftApp, closed: bool) -> u64 {
    let (d, at) =
        if closed { ("M100 300 A100 100 0 1 1 300 300 A100 100 0 1 1 100 300 Z", json!([90, 300])) } else { ("M60 150 L340 150", json!([60, 150])) };
    let path = id(app.run("path.create", json!({ "d": d })).unwrap());
    id(app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": at})).unwrap())
}

/// Puts type `t` in a thread with a new frame, then selects it alone.
fn thread(app: &mut VectorcraftApp, t: u64) {
    let frame = id(app.run("shape.rectangle", json!({"x": 60, "y": 300, "width": 200, "height": 100})).unwrap());
    select(app, &[t, frame]);
    app.run("text.thread.create", json!({})).unwrap();
    select(app, &[t]);
}

/// Places a 2 by 2 pixel image from `dir` and returns its id.
fn place_image(app: &mut VectorcraftApp, dir: &crate::tests_folder::Folder) -> u64 {
    let raw = [0u8, 128, 255, 255].repeat(4);
    let image = craft_image_codecs::Image::from_raw(2, 2, craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, raw).unwrap();
    let png = craft_image_codecs::encode(&image, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
    let path = dir.file("tile.png", &png);
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    app.run("file.place", json!({"path": path, "link": false})).unwrap();
    let img = app.session.active().unwrap().selection.objects[0].0;
    assert!(matches!(kind(app, img), NodeKind::Image(_)));
    img
}

/// An image selected with a new ellipse.
fn image_with_art(app: &mut VectorcraftApp) {
    let dir = crate::tests_folder::Folder::new("ui-taskbar-rows");
    let img = place_image(app, &dir);
    let shape = id(app.run("shape.ellipse", json!({"x": 150, "y": 100, "width": 60, "height": 60})).unwrap());
    select(app, &[img, shape]);
}

/// Whether the bar offers Make Clipping Mask, worked out by counting the selected images: some of
/// the selected ids, but not all, are images.
fn mixed_by_count(st: &vectorcraft_engine::DocState) -> bool {
    let n = st.selection.len();
    let images = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).filter(|o| matches!(o.kind, NodeKind::Image(_))).count();
    images > 0 && images < n
}

/// Sets the selected objects and anchors directly, as no command can select an id with no object,
/// and moves the document's revision as every selection change does.
fn set_selection(app: &mut VectorcraftApp, ids: &[u64], anchors: &[u64]) {
    let st = app.session.active_mut().unwrap();
    st.selection.objects = ids.iter().map(|i| NodeId(*i)).collect();
    st.selection.anchors = anchors.iter().map(|i| (NodeId(*i), [(0, 0)].into_iter().collect())).collect();
    st.revision += 1;
}

/// The path's anchors (its first subpath).
fn anchors(app: &VectorcraftApp, id: u64) -> Vec<vectorcraft_geom::Anchor> {
    match kind(app, id) {
        NodeKind::Path { path, .. } => path.subpaths[0].anchors.to_vec(),
        k => panic!("not a path: {k:?}"),
    }
}

fn live(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::LiveShape {
    match kind(app, id) {
        NodeKind::Path { live: Some(l), .. } => *l,
        k => panic!("not a live shape: {k:?}"),
    }
}

fn locked(app: &VectorcraftApp, id: u64) -> bool {
    app.session.active().unwrap().doc.node(NodeId(id)).unwrap().locked
}

/// The basic shape tools: the tool ids, and the Shape Properties heading each tool's shape shows
/// (none for the star, a plain path).
const SHAPE_TOOLS: [(&str, Option<&str>); 5] = [
    ("rectangle", Some("Rectangle Properties:")),
    ("roundedRectangle", Some("Rectangle Properties:")),
    ("ellipse", Some("Ellipse Properties:")),
    ("polygon", Some("Polygon Properties:")),
    ("star", None),
];

/// The Type tools whose bar shows the font and Fill controls.
const TYPE_TOOLS: [&str; 6] = ["type", "areaType", "typeOnPath", "verticalType", "verticalAreaType", "verticalTypeOnPath"];

/// Draw with `tool` by dragging across the canvas from the document point `from` to `to`, and return
/// whether the task bar showed partway through the drag.
fn drag_with(app: &mut VectorcraftApp, tool: &str, from: (f64, f64), to: (f64, f64)) -> bool {
    app.run("tool.select", json!({ "tool": tool })).unwrap();
    on_canvas(app, |rig| {
        rig.fx.settle();
        let view = rig.app().canvas_view().expect("the canvas is laid out");
        let (a, b) = (view.to_screen(Point::new(from.0, from.1)), view.to_screen(Point::new(to.0, to.1)));
        rig.fx.drag_hold_at(a, b - a);
        let shown_mid_drag = rig.fx.by_label(BAR).is_some();
        rig.fx.release_at(b);
        shown_mid_drag
    })
}

/// Point type "Hello" at 20 pt, made with `tool` and edited by it as after a click with it.
fn edited_type(app: &mut VectorcraftApp, tool: &str) -> u64 {
    app.run("tool.select", json!({ "tool": tool })).unwrap();
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(crate::panels::character::text_editing(app).is_some(), "{tool} edits the type");
    t
}

/// The size of each run of type `t`, with its text.
fn sizes(app: &VectorcraftApp, t: u64) -> Vec<(String, f64)> {
    match kind(app, t) {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.size)).collect(),
        k => panic!("{k:?}"),
    }
}

#[test]
fn selection_kinds_offer_their_actions() {
    let basic = |command: &'static str, params: serde_json::Value| {
        move |app: &mut VectorcraftApp| {
            let s = id(app.run(command, params.clone()).unwrap());
            select(app, &[s]);
        }
    };
    let rects = |app: &mut VectorcraftApp| {
        let a = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap());
        let b = id(app.run("shape.ellipse", json!({"x": 60, "y": 20, "width": 30, "height": 30})).unwrap());
        select(app, &[a, b]);
    };
    let shape = || json!({"x": 40, "y": 40, "width": 80, "height": 60});
    let direct_whole = |app: &mut VectorcraftApp| {
        let l = open_line(app);
        app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
        select(app, &[l]);
    };
    check(vec![
        row(
            "an open path",
            |app| {
                let l = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
                select(app, &[l]);
            },
            &["Join", "Edit Path", "Offset Path", "Simplify", "Duplicate"],
            &[],
        ),
        row(
            "a closed path",
            |app| {
                let p = id(app
                    .run("path.create", json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}], "closed": true}))
                    .unwrap());
                select(app, &[p]);
            },
            &["Edit Path", "Offset Path"],
            &["Join"],
        ),
        row(
            "a compound path",
            |app| {
                compound(app);
            },
            &["Edit Path", "Release Compound Path"],
            &["Offset Path"],
        ),
        row(
            "a clipping mask",
            |app| {
                clipping_group(app);
            },
            &["Edit Clipping Path", "Release Clipping Mask"],
            &["Ungroup"],
        ),
        row("an image with vector art", image_with_art, &["Make Clipping Mask", "Group"], &["Unite"]),
        row(
            "vector art alone",
            |app| {
                let ids = two_shapes(app);
                select(app, &ids);
            },
            &["Unite", "Group"],
            &["Make Clipping Mask"],
        ),
        row(
            "a compound path under Direct Selection",
            |app| {
                let c = compound(app);
                app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
                select(app, &[c]);
            },
            &["Release Compound Path"],
            &["Edit Path"],
        ),
        row(
            "a path selected whole under Direct Selection",
            direct_whole,
            &["Connect", "convertToCorner", "convertToSmooth", "Simplify", "Smooth", "Duplicate"],
            &["Edit Path", "Offset Path", "Join", "Remove Anchor Points", "Cut Path"],
        ),
        row(
            "anchors picked out",
            |app| {
                let l = open_line(app);
                app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
                app.run("select.anchors", json!({"id": l, "anchors": [[0, 1]]})).unwrap();
            },
            &["Remove Anchor Points", "Connect", "Cut Path", "convertToCorner", "convertToSmooth"],
            &["Edit Path"],
        ),
        row("a rectangle", basic("shape.rectangle", shape()), &["shapeProperties"], &["lock"]),
        row(
            "a rounded rectangle",
            basic("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})),
            &["shapeProperties"],
            &["lock"],
        ),
        row("an ellipse", basic("shape.ellipse", shape()), &["shapeProperties"], &["lock"]),
        row("a polygon", basic("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})), &["shapeProperties"], &["lock"]),
        row(
            "a closed path keeps Lock",
            |app| {
                let p = id(app
                    .run("path.create", json!({"anchors": [{"x": 50, "y": 200}, {"x": 150, "y": 80}, {"x": 250, "y": 200}], "closed": true}))
                    .unwrap());
                select(app, &[p]);
            },
            &["lock"],
            &["shapeProperties"],
        ),
        row("a star keeps Lock", basic("shape.star", json!({"cx": 200, "cy": 150, "radius1": 40})), &["lock"], &["shapeProperties"]),
        row(
            "a star with live corners keeps Lock",
            |app| {
                let s = id(app.run("shape.star", json!({"cx": 300, "cy": 150, "radius1": 40})).unwrap());
                select(app, &[s]);
                app.run("object.setLiveShape", json!({"radius": 4})).unwrap();
                assert!(matches!(live(app, s), vectorcraft_doc::LiveShape::Path { .. }));
            },
            &["lock"],
            &["shapeProperties"],
        ),
        row(
            "a group keeps Lock",
            move |app| {
                rects(app);
                app.run("object.group", json!({})).unwrap();
            },
            &["lock"],
            &["shapeProperties"],
        ),
        row("several objects keep Lock", rects, &["lock"], &["shapeProperties"]),
        row(
            "a rectangle under Direct Selection keeps Lock",
            |app| {
                let s = id(app.run("shape.rectangle", json!({"x": 300, "y": 40, "width": 30, "height": 30})).unwrap());
                app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
                select(app, &[s]);
            },
            &["lock"],
            &["shapeProperties"],
        ),
        row(
            "a rectangle under Group Selection keeps Lock",
            |app| {
                let s = id(app.run("shape.rectangle", json!({"x": 300, "y": 40, "width": 30, "height": 30})).unwrap());
                app.run("tool.select", json!({"tool": "groupSelection"})).unwrap();
                select(app, &[s]);
            },
            &["lock"],
            &["shapeProperties"],
        ),
    ]);
}

#[test]
fn type_offers_its_actions_and_font_controls() {
    let text =
        |app: &mut VectorcraftApp, y: u32, content: &str| id(app.run("text.create", json!({"x": 60, "y": y, "text": content, "size": 20})).unwrap());
    let mut rows = vec![
        row(
            "point type",
            |app| {
                let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
                select(app, &[t]);
            },
            &["Create Outlines", "Convert To Area Type", "Dynamic Text", "Duplicate"],
            &["Convert To Point Type"],
        )
        .fonts(true),
        row(
            "area type",
            |app| {
                let t = id(app.run("text.create", json!({"x": 60, "y": 80, "area": {"width": 200, "height": 100}, "text": "Fit me"})).unwrap());
                select(app, &[t]);
            },
            &["Convert To Point Type", "Dynamic Text"],
            &["Convert To Area Type"],
        ),
        row(
            "area type in a thread",
            |app| {
                let t = id(app.run("text.create", json!({"x": 60, "y": 80, "area": {"width": 200, "height": 100}, "text": "Fit me"})).unwrap());
                thread(app, t);
            },
            &["Convert To Point Type"],
            &["Dynamic Text"],
        ),
        row(
            "type on an open path",
            |app| {
                let t = path_type(app, false);
                select(app, &[t]);
            },
            &["Create Outlines", "Type on a Path Options", "Dynamic Text"],
            &["Convert To Area Type", "Convert To Point Type", "Center on Top", "Center on Bottom"],
        ),
        row(
            "type on a closed path",
            |app| {
                let t = path_type(app, true);
                select(app, &[t]);
            },
            &["Type on a Path Options", "Center on Top", "Center on Bottom", "Dynamic Text"],
            &[],
        ),
        row(
            "type on a path in a thread",
            |app| {
                let t = path_type(app, false);
                thread(app, t);
            },
            &["Create Outlines"],
            &["Dynamic Text"],
        ),
        without("a Type tool before it adds type", |app| {
            app.run("tool.select", json!({"tool": "type"})).unwrap();
        }),
        without("a Type tool with a rectangle", |app| {
            let r = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
            select(app, &[r]);
            app.run("tool.select", json!({"tool": "type"})).unwrap();
        }),
        without("Touch Type with type selected", |app| {
            let t = id(app.run("text.create", json!({"x": 60, "y": 180, "text": "Hello", "size": 20})).unwrap());
            app.run("tool.select", json!({"tool": "touchType"})).unwrap();
            select(app, &[t]);
        }),
    ];
    rows.push(
        row(
            "several type objects",
            move |app| {
                let (a, b) = (
                    id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap()),
                    id(app.run("text.create", json!({"x": 60, "y": 160, "text": "World", "size": 20})).unwrap()),
                );
                select(app, &[a, b]);
            },
            &["Duplicate"],
            &[],
        )
        .fonts(true),
    );
    rows.push(
        row(
            "type with other art",
            |app| {
                let a = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
                let r = id(app.run("shape.rectangle", json!({"x": 200, "y": 40, "width": 80, "height": 60})).unwrap());
                select(app, &[a, r]);
            },
            &["Group"],
            &[],
        )
        .fonts(false),
    );
    rows.push(
        row(
            "a rectangle",
            |app| {
                let r = id(app.run("shape.rectangle", json!({"x": 200, "y": 40, "width": 80, "height": 60})).unwrap());
                select(app, &[r]);
            },
            &["Duplicate"],
            &[],
        )
        .fonts(false),
    );
    for tool in TYPE_TOOLS {
        rows.push(
            row(
                format!("the {tool} tool editing type"),
                move |app| {
                    edited_type(app, tool);
                },
                &["fill"],
                &["createOutlines", "convertToAreaType", "dynamicText", "duplicate", "stroke", "recolor", "lock"],
            )
            .fonts(true),
        );
    }
    let _ = text;
    check(rows);
}

/// The objects the shape tools' bar is tried with.
struct Scene {
    rect: u64,
    ellipse: u64,
    group: u64,
    line: u64,
    text: u64,
}

fn scene(app: &mut VectorcraftApp) -> Scene {
    let rect = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap());
    let ellipse = id(app.run("shape.ellipse", json!({"x": 60, "y": 20, "width": 30, "height": 30})).unwrap());
    let c = id(app.run("shape.rectangle", json!({"x": 20, "y": 250, "width": 30, "height": 30})).unwrap());
    let d = id(app.run("shape.rectangle", json!({"x": 60, "y": 250, "width": 30, "height": 30})).unwrap());
    select(app, &[c, d]);
    let group = id(app.run("object.group", json!({})).unwrap());
    let line = id(app.run("shape.line", json!({"x1": 150, "y1": 50, "x2": 300, "y2": 120})).unwrap());
    let text = id(app.run("text.create", json!({"x": 150, "y": 200, "text": "Type"})).unwrap());
    Scene { rect, ellipse, group, line, text }
}

#[test]
fn tools_offer_their_bar_or_none() {
    type Pick = fn(&Scene) -> Vec<u64>;
    let picks: [(&str, Pick); 5] = [
        ("nothing", |_| vec![]),
        ("several", |s| vec![s.rect, s.ellipse]),
        ("a group", |s| vec![s.group]),
        ("an open path", |s| vec![s.line]),
        ("type", |s| vec![s.text]),
    ];
    let mut rows = vec![];
    // A basic shape tool's bar is for one closed path selected whole.
    for (tool, _) in SHAPE_TOOLS {
        for (what, pick) in picks {
            rows.push(without(format!("{tool} with {what}"), move |app| {
                let s = scene(app);
                app.run("tool.select", json!({ "tool": tool })).unwrap();
                select(app, &pick(&s));
            }));
        }
        rows.push(without(format!("{tool} with anchors"), move |app| {
            let s = scene(app);
            app.run("tool.select", json!({ "tool": tool })).unwrap();
            app.run("select.anchors", json!({"id": s.rect, "anchors": [[0, 1]]})).unwrap();
        }));
        rows.push(row(
            format!("{tool} with the rectangle"),
            move |app| {
                let s = scene(app);
                app.run("tool.select", json!({ "tool": tool })).unwrap();
                select(app, &[s.rect]);
            },
            &["editPath", "fill", "stroke"],
            &["duplicate", "offsetPath", "simplify", "recolor", "lock"],
        ));
    }
    rows.push(without("the Line Segment tool with a rectangle", |app| {
        let s = scene(app);
        app.run("tool.select", json!({"tool": "lineSegment"})).unwrap();
        select(app, &[s.rect]);
    }));
    // The Smooth tool's bar is for a path it can smooth.
    let smooth = |ids: fn(u64, u64, u64) -> Vec<u64>| {
        move |app: &mut VectorcraftApp| {
            let path = id(app.run("shape.ellipse", json!({"x": 60, "y": 60, "width": 120, "height": 80})).unwrap());
            let line = id(app.run("shape.line", json!({"x1": 150, "y1": 200, "x2": 300, "y2": 240})).unwrap());
            let text = id(app.run("text.create", json!({"x": 220, "y": 60, "text": "Type"})).unwrap());
            app.run("tool.select", json!({"tool": "smooth"})).unwrap();
            select(app, &ids(path, line, text));
        }
    };
    rows.push(without("the Smooth tool with nothing", smooth(|_, _, _| vec![])));
    rows.push(without("the Smooth tool with a straight line", smooth(|_, l, _| vec![l])));
    rows.push(without("the Smooth tool with type", smooth(|_, _, t| vec![t])));
    rows.push(row(
        "the Smooth tool with a path",
        smooth(|p, _, _| vec![p]),
        &["fidelity", "smoothToolOptions"],
        &["fill", "stroke", "recolor", "lock", "duplicate", "editPath", "offsetPath", "simplify", "smooth"],
    ));
    rows.push(row("the Smooth tool with a path and a line", smooth(|p, l, _| vec![p, l]), &["fidelity"], &[]));
    // A blank canvas offers Draw and Place Image under a selection tool; ruler guides are not artwork.
    for tool in ["selection", "directSelection", "groupSelection"] {
        rows.push(row(
            format!("a blank canvas with {tool}"),
            move |app| {
                app.run("tool.select", json!({ "tool": tool })).unwrap();
            },
            &["Draw (Pencil)", "Place Image"],
            &["fill", "stroke", "recolor", "lock", "duplicate"],
        ));
    }
    for tool in ["pen", "rectangle", "type"] {
        rows.push(without(format!("a blank canvas with {tool}"), move |app| {
            app.run("tool.select", json!({ "tool": tool })).unwrap();
        }));
    }
    rows.push(without("artwork with nothing selected", |app| {
        let r = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
        select(app, &[r]);
        select(app, &[]);
    }));
    rows.push(row(
        "a ruler guide is not artwork",
        |app| {
            let r = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
            select(app, &[r]);
            app.run("edit.clear", json!({})).unwrap();
            app.run("guide.add", json!({"pos": 50})).unwrap();
        },
        &["Draw (Pencil)"],
        &[],
    ));
    check(rows);

    // The blank canvas bar hangs under the active artboard.
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let board = app.session.active().unwrap().doc.artboards[0].rect;
    let o = offer(&mut app).expect("the blank canvas offers actions");
    assert!(
        !o.anchor.is_empty()
            && o.anchor.iter().all(|p| p.x >= board.x0 - 1.0 && p.x <= board.x1 + 1.0 && p.y >= board.y0 - 1.0 && p.y <= board.y1 + 1.0),
        "{:?} under {board:?}",
        o.anchor
    );

    // The Artboard tool offers the artboards' actions under the selected one, the Selection tool the object's.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let labels = |app: &mut VectorcraftApp| offer(app).map(|o| o.actions.iter().map(|a| a.0).collect::<Vec<_>>()).unwrap_or_default();
    app.select_tool("artboard");
    crate::panels::artboards::select(&mut app, 1);
    assert_eq!(labels(&mut app), ["Resize", "Duplicate", "Rearrange", "Export"]);
    let b = app.session.active().unwrap().doc.artboards[1].rect;
    let o = offer(&mut app).unwrap();
    assert!(
        !o.anchor.is_empty() && o.anchor.iter().all(|p| p.x >= b.x0 - 1.0 && p.x <= b.x1 + 1.0 && p.y >= b.y0 - 1.0 && p.y <= b.y1 + 1.0),
        "{:?} on artboard 2 {b:?}",
        o.anchor
    );
    app.select_tool("selection");
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    assert_eq!(labels(&mut app), ["Edit Path", "Offset Path", "Simplify", "Duplicate"]);
    // One artboard, selected: Rearrange places it.
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.select_tool("artboard");
    assert_eq!(labels(&mut app), ["Resize", "Duplicate", "Rearrange", "Export"]);
}

/// Three 200 by 100 pt artboards with the Artboard tool, `selected` selected.
fn rearrange_app(selected: serde_json::Value) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    app.session.set_tool_option("selected", &selected);
    app
}

fn artboard_rects(app: &VectorcraftApp) -> Vec<vectorcraft_geom::Rect> {
    app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect()
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Opens Rearrange for the selected artboards and presses OK with one row 10 pt apart, inside `rig`.
fn confirm_rearrange(rig: &mut Rig) {
    use crate::dialogs::rearrange_artboards::{PLACE_HINT, RearrangeArtboardsDialog};
    rig.run_fired();
    let mut app = rig.app();
    crate::panels::artboards::open_rearrange_selected(&mut app);
    let id = app.dialogs().and_then(|d| d.with(|h| h.open_dialogs().last().map(|o| o.id))).expect("Rearrange opens");
    assert!(app.dialogs().unwrap().with(|h| h.is_open::<RearrangeArtboardsDialog>()));
    crate::tests::picks::answer_dialog(&mut app, id, &[("layout", json!("row")), ("spacing", json!(10))], true);
    assert!(app.ui.rearrange_placement.is_some() && app.ui.status == PLACE_HINT, "{}", app.ui.status);
}

/// Rearrange for the selected artboards, confirmed, waits for a click: the grid's outline follows
/// the pointer, and a press on the canvas puts the selected artboards' grid there with its top-left
/// at the press, as one undo step, leaving the other artboards and giving the tool no press. Esc
/// cancels a waiting one, and so does another tool, with nothing moved.
#[test]
fn rearrange_places_the_selected_artboards_where_the_canvas_is_clicked() {
    use crate::dialogs::rearrange_artboards::{PLACE_HINT, placement_overlays};
    let mut app = rearrange_app(json!([0, 2]));
    let (before, steps) = (artboard_rects(&app), undo_steps(&app));
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        let view = rig.app().canvas_view().expect("the canvas is laid out");
        let at = view.rect.center() + vec2(-150.0, -120.0);
        let doc = view.to_doc(at);

        // Esc: nothing moves.
        confirm_rearrange(rig);
        rig.fx.frame(vec![egui::Event::PointerMoved(at)]);
        rig.fx.key(Key::Escape, Modifiers::NONE);
        rig.run_fired();
        assert!(rig.app().ui.rearrange_placement.is_none(), "Esc cancels");
        assert_ne!(rig.app().ui.status, PLACE_HINT);
        // Another tool: nothing moves.
        confirm_rearrange(rig);
        rig.app().select_tool("selection");
        rig.fx.frame(vec![egui::Event::PointerMoved(at)]);
        assert!(rig.app().ui.rearrange_placement.is_none(), "another tool cancels");
        {
            let app = rig.app();
            assert_eq!((artboard_rects(&app), undo_steps(&app)), (before.clone(), steps));
        }

        rig.app().select_tool("artboard");
        rig.app().session.set_tool_option("selected", &json!([0, 2]));
        confirm_rearrange(rig);
        rig.fx.frame(vec![egui::Event::PointerMoved(at)]);
        let outline = placement_overlays(&rig.app());
        assert_eq!(outline.len(), 2, "an outline for each selected artboard");
        rig.fx.click_at(at);
        rig.run_fired();
        let app = rig.app();
        let after = artboard_rects(&app);
        assert_eq!(after.len(), 3, "the Artboard tool made no artboard");
        assert_eq!(after[1], before[1], "artboard 2 is not selected");
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        assert!(near(after[0].x0, doc.x) && near(after[0].y0, doc.y), "{:?} at {doc:?}", after[0]);
        assert!(near(after[2].x0, doc.x + 210.0) && near(after[2].y0, doc.y), "{:?}", after[2]);
        assert_eq!(undo_steps(&app), steps + 1);
        assert!(app.ui.rearrange_placement.is_none() && app.ui.status != PLACE_HINT);
    });
}

/// A waiting Rearrange takes Enter from the keyboard: with the pointer off the canvas, the grid
/// starts at the first selected artboard's top-left, in one undo step. A press with Space held
/// pans and places nothing.
#[test]
fn rearrange_places_the_selected_artboards_with_enter_and_lets_space_pan() {
    let mut app = rearrange_app(json!([2, 0]));
    let (before, steps) = (artboard_rects(&app), undo_steps(&app));
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        confirm_rearrange(rig);
        // Space held: the press pans.
        let at = rig.app().canvas_view().expect("the canvas is laid out").rect.center();
        let origin = |rig: &Rig| rig.app().canvas_view().unwrap().to_screen(Point::ZERO);
        let start = origin(rig);
        let key = |key, pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let end = at + vec2(30.0, 20.0);
        for ev in [
            key(Key::Space, true),
            egui::Event::PointerMoved(at),
            button(at, true),
            egui::Event::PointerMoved(at.lerp(end, 0.5)),
            egui::Event::PointerMoved(end),
            button(end, false),
        ] {
            rig.fx.frame(vec![ev]);
        }
        rig.fx.frame(vec![key(Key::Space, false)]);
        rig.run_fired();
        assert!(origin(rig) != start, "the Space-drag pans the view");
        {
            let app = rig.app();
            assert!(app.ui.rearrange_placement.is_some(), "a Space-drag places nothing");
            assert_eq!((artboard_rects(&app), undo_steps(&app)), (before.clone(), steps));
        }
        rig.fx.frame(vec![egui::Event::PointerGone]);
        rig.fx.key(Key::Enter, Modifiers::NONE);
        rig.run_fired();
    });
    let after = artboard_rects(&app);
    assert!(app.ui.rearrange_placement.is_none());
    assert_eq!((after[0], after[1]), (before[0], before[1]));
    assert_eq!((after[2].x0, after[2].y0), (before[0].x0 + 210.0, before[0].y0));
    assert_eq!(undo_steps(&app), steps + 1);
}

/// Once a pen has been seen, a finger press on the canvas navigates and does not place a waiting
/// Rearrange: the finger's drag pans the view while the artboards and the history stay as they
/// were. A mouse click then places it.
#[test]
fn rearrange_is_not_placed_by_a_finger_once_a_pen_was_seen() {
    use crate::touch::{Source, set_pen_seen, set_source};
    let mut app = rearrange_app(json!([1]));
    let (before, steps) = (artboard_rects(&app), undo_steps(&app));
    let (touched, panned) = on_canvas(&mut app, |rig| {
        rig.fx.settle();
        confirm_rearrange(rig);
        let at = rig.app().canvas_view().expect("the canvas is laid out").rect.center() + vec2(40.0, 30.0);
        let origin = |rig: &Rig| rig.app().canvas_view().unwrap().to_screen(Point::ZERO);
        let start = origin(rig);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let end = at + vec2(60.0, 40.0);
        // The platform says a finger drives the pointer once a pen has been seen.
        set_source(Source::Pen);
        set_source(Source::Finger);
        for ev in [
            egui::Event::PointerMoved(at),
            button(at, true),
            egui::Event::PointerMoved(at.lerp(end, 0.5)),
            egui::Event::PointerMoved(end),
            button(end, false),
        ] {
            rig.fx.frame(vec![ev]);
        }
        rig.run_fired();
        let touched = {
            let app = rig.app();
            (app.ui.rearrange_placement.is_some(), artboard_rects(&app), undo_steps(&app))
        };
        let panned = origin(rig) != start;
        // The mouse, with the pen still seen.
        set_source(Source::Mouse);
        rig.fx.click_at(at);
        rig.run_fired();
        set_pen_seen(false);
        (touched, panned)
    });
    assert_eq!(touched, (true, before.clone(), steps), "the finger navigates");
    assert!(panned, "the finger pans the view");
    assert!(app.ui.rearrange_placement.is_none(), "the mouse places it");
    assert_ne!(artboard_rects(&app), before);
    assert_eq!(undo_steps(&app), steps + 1);
}

/// A waiting Rearrange is dropped when its artboards change places in the list before the click,
/// so the click moves no artboard the tool did not select.
#[test]
fn rearrange_waiting_for_a_click_is_dropped_when_its_artboards_are_reordered() {
    use crate::dialogs::rearrange_artboards::PLACE_HINT;
    let mut app = rearrange_app(json!([0]));
    on_canvas(&mut app, |rig| {
        rig.fx.settle();
        confirm_rearrange(rig);
        rig.app().run("artboard.reorder", json!({"index": 2, "to": 0})).unwrap();
        let before = artboard_rects(&rig.app());
        let at = rig.app().canvas_view().unwrap().rect.center();
        rig.fx.click_at(at);
        rig.run_fired();
        let app = rig.app();
        assert!(app.ui.rearrange_placement.is_none() && app.ui.status != PLACE_HINT);
        assert_eq!(artboard_rects(&app), before);
    });
}

/// A Rearrange placement that moves nothing says the artboard is locked only when every selected
/// artboard is locked; a locked artboard beside one already in its cell leaves no such message.
#[test]
fn rearrange_says_locked_only_when_every_selected_artboard_is_locked() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let layout = json!({"layout": "row", "spacing": 10});
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3, "artboardLayout": layout})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "locked": true})).unwrap();
    app.select_tool("artboard");
    for (selected, status) in [(json!([0, 1]), ""), (json!([0]), "the artboard is locked")] {
        app.session.set_tool_option("selected", &selected);
        // The status the placement leaves as the canvas takes Enter.
        let placed = on_canvas(&mut app, |rig| {
            rig.fx.settle();
            confirm_rearrange(rig);
            rig.fx.frame(vec![egui::Event::PointerGone]);
            rig.fx.key(Key::Enter, Modifiers::NONE);
            let placed = rig.app().ui.status.clone();
            rig.run_fired();
            placed
        });
        assert!(app.ui.rearrange_placement.is_none(), "{selected}");
        assert_eq!(placed, status, "{selected}");
    }
}

/// Where [`rearrange_is_not_placed_by_a_press_on_a_panel_over_the_canvas`] floats its panel.
const PANEL_OVER_CANVAS: Rect = Rect::from_min_max(Pos2::new(300.0, 250.0), Pos2::new(420.0, 370.0));

/// A press on a panel floating over the canvas does not place a waiting Rearrange; a press on the
/// open canvas does.
#[test]
fn rearrange_is_not_placed_by_a_press_on_a_panel_over_the_canvas() {
    let mut app = rearrange_app(json!([0]));
    let (before, steps) = (artboard_rects(&app), undo_steps(&app));
    let show = |app: &mut VectorcraftApp, ui: &mut Ui| {
        canvas::show(app, ui);
        egui::Area::new(egui::Id::unique("panel over the canvas")).fixed_pos(PANEL_OVER_CANVAS.min).show(ui.ctx(), |ui| {
            ui.allocate_exact_size(PANEL_OVER_CANVAS.size(), egui::Sense::click_and_drag());
        });
    };
    let on_panel = driven(&mut app, CANVAS_ROOM, show, |rig| {
        rig.fx.settle();
        confirm_rearrange(rig);
        assert!(rig.app().canvas_view().expect("the canvas is laid out").rect.contains_rect(PANEL_OVER_CANVAS));
        rig.fx.click_at(PANEL_OVER_CANVAS.center());
        rig.run_fired();
        let on_panel = {
            let app = rig.app();
            (app.ui.rearrange_placement.is_some(), artboard_rects(&app), undo_steps(&app))
        };
        rig.fx.click_at(PANEL_OVER_CANVAS.left_top() - vec2(40.0, 40.0));
        rig.run_fired();
        on_panel
    });
    assert_eq!(on_panel, (true, before.clone(), steps), "the press is the panel's");
    assert!(app.ui.rearrange_placement.is_none(), "placed on the open canvas");
    assert_ne!(artboard_rects(&app), before);
    assert_eq!(undo_steps(&app), steps + 1);
}

#[test]
fn clicking_an_action_runs_it() {
    let mut app = app();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    click(&mut app, "Join");
    assert!(matches!(kind(&app, line), NodeKind::Path { path, .. } if path.is_closed()));
    click(&mut app, "Edit Path");
    assert_eq!(app.session.tool_id(), "directSelection");
    assert_eq!(app.session.active().unwrap().selection.objects, [NodeId(line)]);

    let mut app = self::app();
    compound(&mut app);
    click(&mut app, "Release Compound Path");
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.len(), 2, "the paths come apart, selected");
    assert!(st.selection.objects.iter().all(|i| matches!(st.doc.node(*i).unwrap().kind, NodeKind::Path { .. })));

    let mut app = self::app();
    let g = clipping_group(&mut app);
    click(&mut app, "Release Clipping Mask");
    assert!(!matches!(kind(&app, g), NodeKind::Group { clip: true, .. }), "no longer clipped");
    app.run("edit.undo", json!({})).unwrap();
    assert!(matches!(kind(&app, g), NodeKind::Group { clip: true, .. }));

    let mut app = self::app();
    image_with_art(&mut app);
    click(&mut app, "Make Clipping Mask");
    let st = app.session.active().unwrap();
    assert!(matches!(st.doc.node(st.selection.objects[0]).unwrap().kind, NodeKind::Group { clip: true, .. }));
}

/// The early-exit walk gives the count's answer for every selection: empty, one object, paths only,
/// images only, images with paths or type, an id selected twice, anchors picked out, and ids with no
/// object.
#[test]
fn images_with_other_art_matches_the_image_count() {
    let mut app = app();
    let dir = crate::tests_folder::Folder::new("ui-taskbar-mixed");
    let img = place_image(&mut app, &dir);
    let img2 = place_image(&mut app, &dir);
    let a = id(app.run("shape.rectangle", json!({"x": 10, "y": 200, "width": 40, "height": 40})).unwrap());
    let b = id(app.run("shape.rectangle", json!({"x": 30, "y": 210, "width": 40, "height": 40})).unwrap());
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello"})).unwrap());
    let stale = 1_000_000;
    assert!(app.session.active().unwrap().doc.node(NodeId(stale)).is_none());
    let cases: [(&[u64], &[u64], bool); 17] = [
        (&[], &[], false),
        (&[a], &[], false),
        (&[img], &[], false),
        (&[a, b], &[], false),
        (&[img, img2], &[], false),
        (&[img, a], &[], true),
        (&[a, img], &[], true),
        (&[a, b, img], &[], true),
        (&[img, t], &[], true),
        (&[img, img], &[], false),
        (&[img, img, a], &[], true),
        (&[img, a], &[a], true),
        (&[a, b], &[a, b], false),
        (&[img, stale], &[], true),
        (&[img, img2, stale], &[], true),
        (&[a, stale], &[], false),
        (&[stale, stale], &[], false),
    ];
    for (ids, anchors, mixed) in cases {
        set_selection(&mut app, ids, anchors);
        let st = app.session.active().unwrap();
        assert_eq!(mixed_by_count(st), mixed, "{ids:?} with anchors on {anchors:?}: the count");
        assert_eq!(images_with_other_art(st, &mut None), mixed, "{ids:?} with anchors on {anchors:?}");
    }
}

/// The answer is kept while the document's revision is unchanged, and worked out again after a change.
#[test]
fn images_with_other_art_is_kept_for_the_revision() {
    let mut app = app();
    let dir = crate::tests_folder::Folder::new("ui-taskbar-mixed-kept");
    let img = place_image(&mut app, &dir);
    let a = id(app.run("shape.rectangle", json!({"x": 10, "y": 200, "width": 40, "height": 40})).unwrap());
    let b = id(app.run("shape.rectangle", json!({"x": 30, "y": 210, "width": 40, "height": 40})).unwrap());
    select(&mut app, &[img, a]);
    let mut memo = None;
    assert!(images_with_other_art(app.session.active().unwrap(), &mut memo));
    // Changing the objects behind the revision's back shows the kept answer is used.
    app.session.active_mut().unwrap().selection.objects = vec![NodeId(a), NodeId(b)];
    assert!(images_with_other_art(app.session.active().unwrap(), &mut memo), "the revision's answer is kept");
    select(&mut app, &[a, b]);
    assert!(!images_with_other_art(app.session.active().unwrap(), &mut memo), "a new revision works it out again");
    select(&mut app, &[b, img]);
    assert!(images_with_other_art(app.session.active().unwrap(), &mut memo));
}

#[test]
fn type_converts_between_point_and_area_type() {
    let mut app = app();
    let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello"})).unwrap());
    select(&mut app, &[t]);
    let text_kind = |app: &VectorcraftApp| match kind(app, t) {
        NodeKind::Text(t) => t.kind.clone(),
        other => panic!("{other:?}"),
    };
    click(&mut app, "Convert To Area Type");
    assert!(matches!(text_kind(&app), TextKind::Area { .. }));
    let o = offer(&mut app).unwrap();
    assert!(shows(&o, "Convert To Point Type") && !shows(&o, "Convert To Area Type"));
    click(&mut app, "Convert To Point Type");
    assert!(matches!(text_kind(&app), TextKind::Point));
}

/// With the Direct Selection tool, the bar's Smooth and Simplify open their sliders and change
/// nothing until the slider is confirmed (#6784).
#[test]
fn direct_selection_smooth_opens_the_smooth_slider() {
    let kind = |name: &str| if name == "Smooth" { StripKind::Smooth } else { StripKind::Simplify };
    let is_open = |app: &VectorcraftApp, name: &str| app.object_dialogs.strip.as_ref().is_some_and(|s| s.kind == kind(name));
    for name in ["Smooth", "Simplify"] {
        let mut app = app();
        // A jagged line of 40 anchor points, which Smooth changes at its opening level.
        let anchors_in: Vec<_> = (0..40).map(|i| json!({"x": 50 + i * 5, "y": 150 + (i % 2) * 6})).collect();
        let line = id(app.run("path.create", json!({"anchors": anchors_in})).unwrap());
        app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
        select(&mut app, &[line]);
        let before = anchors(&app, line);
        let undo = app.session.active().unwrap().history.undo.len();
        on_canvas(&mut app, |rig| {
            rig.click(name);
            assert!(is_open(&rig.app(), name), "{name} opens its slider");
            assert_eq!(rig.app().session.active().unwrap().history.undo.len(), undo, "{name} adds no undo step before it is confirmed");
            rig.fx.key(Key::Escape, Modifiers::NONE);
            rig.fx.settle();
            assert!(!is_open(&rig.app(), name), "Escape cancels the {name} slider");
        });
        assert_eq!(anchors(&app, line), before, "{name}: canceling leaves the path as it was");
        if name != "Smooth" {
            continue;
        }
        // Confirmed, the slider smooths the path as one undo step, which is not the anchor convert.
        on_canvas(&mut app, |rig| {
            rig.click(name);
            rig.fx.key(Key::Enter, Modifiers::NONE);
            rig.fx.settle();
            assert!(!is_open(&rig.app(), name), "Enter keeps the {name} slider's result");
        });
        let history = &app.session.active().unwrap().history.undo;
        assert_eq!(history.len(), undo + 1, "one undo step");
        assert_ne!(history.last().unwrap().label, "Convert Anchor Points");
        assert_ne!(anchors(&app, line), before, "Smooth changes the path's curvature");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(anchors(&app, line), before, "undo restores the path");
    }
}

/// A basic shape shows Shape Properties in place of Lock, under its own heading; the button doesn't
/// lock the shape.
#[test]
fn a_basic_shape_shows_its_properties_in_place_of_lock() {
    let mut app = app();
    let shapes = [
        (id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap()), "Rectangle Properties:"),
        (id(app.run("shape.ellipse", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap()), "Ellipse Properties:"),
        (id(app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})).unwrap()), "Polygon Properties:"),
    ];
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    for (shape, heading) in shapes {
        select(&mut app, &[shape]);
        on_canvas(&mut app, |rig| {
            assert!(!rig.has_text(heading), "the popup starts closed");
            rig.click("Shape Properties");
            assert!(rig.has_text(heading), "{heading}");
        });
        assert!(!locked(&app, shape), "the button doesn't lock the shape");
    }
}

/// The popup's fields edit the shape through `object.setLiveShape`, and Undo restores it.
#[test]
fn the_shape_properties_popup_edits_the_shape() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let corners = |app: &VectorcraftApp, id: u64| match live(app, id) {
        vectorcraft_doc::LiveShape::Rectangle { radii, .. } => radii,
        l => panic!("{l:?}"),
    };
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap());
    select(&mut app, &[rect]);
    let edit = |app: &mut VectorcraftApp, field: &str, value: &str| {
        on_canvas(app, |rig| {
            rig.click("Shape Properties");
            rig.type_into(field, value);
        })
    };
    edit(&mut app, "10 pt", "20");
    assert_eq!(corners(&app, rect), [20.0; 4]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(corners(&app, rect), [10.0; 4]);

    let ellipse = id(app.run("shape.ellipse", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
    select(&mut app, &[ellipse]);
    let pie = |app: &VectorcraftApp| match live(app, ellipse) {
        vectorcraft_doc::LiveShape::Ellipse { pie, .. } => pie,
        l => panic!("{l:?}"),
    };
    edit(&mut app, "360°", "180");
    assert_eq!(pie(&app), (0.0, 180.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(pie(&app), (0.0, 360.0));

    let polygon = id(app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})).unwrap());
    select(&mut app, &[polygon]);
    let sides = |app: &VectorcraftApp| match live(app, polygon) {
        vectorcraft_doc::LiveShape::Polygon { sides, .. } => sides,
        l => panic!("{l:?}"),
    };
    edit(&mut app, "6", "5");
    assert_eq!(sides(&app), 5);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sides(&app), 6);
}

/// The popup stays open while a polygon's Corner dropdown is open, so a corner type can be picked from it.
#[test]
fn the_shape_properties_popup_keeps_its_corner_dropdown() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let polygon = id(app.run("shape.polygon", json!({"cx": 100, "cy": 100, "radius": 40, "sides": 6})).unwrap());
    select(&mut app, &[polygon]);
    on_canvas(&mut app, |rig| {
        rig.click("Shape Properties");
        rig.click_text("Round", false);
        assert!(rig.has_text("Polygon Properties:") && rig.has_text("Chamfer"), "the popup and its dropdown");
        rig.click_text("Chamfer", false);
    });
    let vectorcraft_doc::LiveShape::Polygon { kinds, .. } = live(&app, polygon) else { panic!("not a polygon") };
    assert!(!kinds.is_empty() && kinds.iter().all(|k| *k == vectorcraft_geom::shapes::CornerKind::Chamfer), "{kinds:?}");
}

/// A rectangle's Corner menu in the popup sets the kind of all four corners in one undo step while
/// Corner Radius is linked; picking the kind they already have adds no step. Unlinked, each corner's
/// menu sets that corner alone.
#[test]
fn the_shape_properties_popup_changes_a_rectangle_corner_type() {
    use vectorcraft_geom::shapes::CornerKind::{Chamfer, Round};
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    app.run("prefs.set", json!({"key": "singleCornerRadius", "value": true})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap());
    select(&mut app, &[rect]);
    let kinds = |app: &VectorcraftApp| match live(app, rect) {
        vectorcraft_doc::LiveShape::Rectangle { kinds, radii, .. } => (kinds, radii),
        l => panic!("{l:?}"),
    };
    let steps = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
    on_canvas(&mut app, |rig| {
        rig.click("Shape Properties");
        let before = steps(&rig.app());
        rig.click_text("Round", false);
        rig.click_text("Chamfer", true);
        assert_eq!(kinds(&rig.app()), ([Chamfer; 4], [10.0; 4]), "all four corners, radii kept");
        assert_eq!(steps(&rig.app()), before + 1, "one undo step");
        // The list is open: its Chamfer row shows as well as the closed menu's, and the pick closes it.
        rig.click_text("Chamfer", false);
        assert_eq!(rig.count_text("Chamfer"), 2);
        assert!(rig.has_text("Inverted Round"));
        rig.click_text("Chamfer", true);
        // The closed list fades out on the clock.
        rig.fx.advance(1.0);
        assert!(!rig.has_text("Inverted Round"), "the pick closes the list");
        assert_eq!(steps(&rig.app()), before + 1, "the same kind adds no undo step");

        rig.app().run("prefs.set", json!({"key": "singleCornerRadius", "value": false})).unwrap();
        assert_eq!(rig.count_text("Chamfer"), 4);
        // The first menu is the top left corner's.
        rig.click_text("Chamfer", false);
        rig.click_text("Round", true);
        assert_eq!(kinds(&rig.app()), ([Round, Chamfer, Chamfer, Chamfer], [10.0; 4]));
        assert_eq!(steps(&rig.app()), before + 2);
        rig.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(kinds(&rig.app()).0, [Chamfer; 4]);
    });
}

/// The Transform panel in a room `width` points wide.
fn on_transform<R>(app: &mut VectorcraftApp, width: f32, f: impl FnOnce(&mut Rig) -> R) -> R {
    driven(
        app,
        vec2(width, 700.0),
        |app, ui| {
            crate::panels::transform::show(app, ui);
        },
        f,
    )
}

/// With several rectangles selected, the linked Corner menu sets the kind on every corner of each of
/// them, also when the rectangle the panel shows already has it, and leaves a selected polygon as it is.
#[test]
fn the_linked_corner_menu_sets_every_selected_rectangle() {
    use vectorcraft_geom::shapes::CornerKind::{Chamfer, Round};
    let mut app = app();
    app.run("prefs.set", json!({"key": "singleCornerRadius", "value": true})).unwrap();
    let a = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 60, "radius": 8})).unwrap());
    let b = id(app.run("shape.rectangle", json!({"x": 200, "y": 20, "width": 80, "height": 60, "radius": 8})).unwrap());
    let polygon = id(app.run("shape.polygon", json!({"cx": 200, "cy": 200, "radius": 40, "sides": 6})).unwrap());
    app.run("object.setLiveShape", json!({"ids": [a], "kind": "chamfer"})).unwrap();
    select(&mut app, &[a, b, polygon]);
    let kinds = |app: &VectorcraftApp, id: u64| match live(app, id) {
        vectorcraft_doc::LiveShape::Rectangle { kinds, .. } => kinds,
        l => panic!("{l:?}"),
    };
    let steps = app.session.active().unwrap().history.undo.len();
    on_transform(&mut app, 300.0, |rig| {
        rig.click_text("Chamfer", false);
        rig.click_text("Chamfer", true);
    });
    assert_eq!((kinds(&app, a), kinds(&app, b)), ([Chamfer; 4], [Chamfer; 4]));
    let vectorcraft_doc::LiveShape::Polygon { kinds: sides, .. } = live(&app, polygon) else { panic!("not a polygon") };
    assert!(sides.iter().all(|k| *k == Round), "the polygon keeps its corners: {sides:?}");
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(kinds(&app, b), [Round; 4]);
}

/// Unlinked, a corner's radius field rounds that corner of each selected rectangle and leaves a
/// selected polygon as it is.
#[test]
fn an_unlinked_corner_radius_leaves_other_selected_shapes() {
    let mut app = app();
    app.run("prefs.set", json!({"key": "singleCornerRadius", "value": false})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 60, "radius": 7})).unwrap());
    let polygon = id(app.run("shape.polygon", json!({"cx": 200, "cy": 200, "radius": 40, "sides": 6})).unwrap());
    select(&mut app, &[rect, polygon]);
    // The top left corner's field is the first that shows the radius.
    on_transform(&mut app, 300.0, |rig| rig.type_into("7 pt", "15"));
    let vectorcraft_doc::LiveShape::Rectangle { radii, .. } = live(&app, rect) else { panic!("not a rectangle") };
    assert_eq!(radii, [15.0, 7.0, 7.0, 7.0]);
    let vectorcraft_doc::LiveShape::Polygon { radii, .. } = live(&app, polygon) else { panic!("not a polygon") };
    assert!(radii.iter().all(|r| *r == 0.0), "the polygon keeps its corners: {radii:?}");
}

/// Unlinked, the four corner rows keep their Corner menus inside a panel at its narrowest.
#[test]
fn the_unlinked_corner_menus_fit_a_narrow_panel() {
    let mut app = app();
    app.run("prefs.set", json!({"key": "singleCornerRadius", "value": false})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 60, "radius": 8})).unwrap());
    app.run("object.setLiveShape", json!({"ids": [rect], "kind": "invertedRound"})).unwrap();
    select(&mut app, &[rect]);
    let width = 220.0;
    let texts = on_transform(&mut app, width, |rig| rig.texts());
    let menus: Vec<_> = texts.iter().filter(|(t, _)| t.starts_with("Inverted")).collect();
    assert_eq!(menus.len(), 4, "{texts:?}");
    assert!(menus.iter().all(|(_, r)| r.right() <= width), "{menus:?}");
}

/// Each basic shape tool, just after it draws a shape, shows Edit Path, Fill and Stroke for it, and
/// Shape Properties for a live rectangle, rounded rectangle, ellipse or polygon. The bar leaves out the
/// selection tools' Duplicate, Offset Path, Simplify, Recolor Artwork and Lock, and shows nothing
/// during the drag that draws the shape.
#[test]
fn shape_tools_show_edit_path_paint_and_properties_after_drawing() {
    let mut app = app();
    for (i, (tool, heading)) in SHAPE_TOOLS.into_iter().enumerate() {
        let x = 30.0 + 70.0 * i as f64;
        let shown_mid_drag = drag_with(&mut app, tool, (x, 60.0), (x + 50.0, 120.0));
        assert!(!shown_mid_drag, "{tool}: no bar while the shape is dragged out");
        assert_eq!(app.session.tool_id(), tool);
        let st = app.session.active().unwrap();
        assert_eq!(st.selection.objects.len(), 1, "{tool} drew and selected a shape");
        let shape = st.selection.objects[0].0;
        let o = offer(&mut app).unwrap_or_else(|| panic!("{tool}: the bar shows"));
        for what in ["editPath", "fill", "stroke"] {
            assert!(offers(&o, what), "{tool}: {what} in {:?}", names_of(&o));
        }
        assert_eq!(offers(&o, "shapeProperties"), heading.is_some(), "{tool}: {:?}", names_of(&o));
        for what in ["duplicate", "offsetPath", "simplify", "recolor", "lock"] {
            assert!(!offers(&o, what), "{tool}: {what} is left out of {:?}", names_of(&o));
        }
        on_canvas(&mut app, |rig| {
            assert!(rig.target("More options").is_some(), "{tool}: More options");
            if let Some(heading) = heading {
                rig.click("Shape Properties");
                assert!(rig.has_text(heading), "{tool}: {heading}");
            }
        });
        assert!(!locked(&app, shape), "{tool}: the button doesn't lock the shape");
    }
}

/// A basic shape tool's bar follows the selection: Shape Properties shows the shape drawn last, edits
/// it with Undo restoring it, and after an Undo of the drawing shows the shape selected then. Fill
/// makes the Fill active, Stroke the Stroke, and Edit Path switches to the Direct Selection tool.
#[test]
fn shape_tool_bar_retargets_after_another_shape_and_undo() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "ellipse"})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap());
    let corners = |app: &VectorcraftApp| match live(app, rect) {
        vectorcraft_doc::LiveShape::Rectangle { radii, .. } => radii,
        l => panic!("{l:?}"),
    };
    on_canvas(&mut app, |rig| {
        rig.click("Shape Properties");
        assert!(rig.has_text("Rectangle Properties:"));
        rig.type_into("10 pt", "20");
    });
    assert_eq!(corners(&app), [20.0; 4]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(corners(&app), [10.0; 4]);

    let ellipse = id(app.run("shape.ellipse", json!({"x": 200, "y": 40, "width": 80, "height": 60})).unwrap());
    on_canvas(&mut app, |rig| {
        rig.click("Shape Properties");
        assert!(rig.has_text("Ellipse Properties:") && !rig.has_text("Rectangle Properties:"));
    });
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.node(NodeId(ellipse)).is_none(), "the ellipse is undone");
    let selected: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|i| i.0).collect();
    if selected == [rect] {
        assert!(offer(&mut app).is_some(), "the rectangle selected again: the bar");
    } else {
        assert!(offer(&mut app).is_none(), "{selected:?}: no bar");
        select(&mut app, &[rect]);
    }
    on_canvas(&mut app, |rig| {
        rig.click("Shape Properties");
        assert!(rig.has_text("Rectangle Properties:") && !rig.has_text("Ellipse Properties:"));
    });

    app.session.fill_active = false;
    click(&mut app, "Fill");
    assert!(app.session.fill_active, "Fill makes the Fill active");
    click(&mut app, "Stroke");
    assert!(!app.session.fill_active, "Stroke makes the Stroke active");
    assert_eq!(app.session.tool_id(), "ellipse");
    click(&mut app, "Edit Path");
    assert_eq!(app.session.tool_id(), "directSelection");
}

/// The Smooth tool's Fidelity field sets the tool's Fidelity, and Tool Options… opens the tool's
/// options.
#[test]
fn the_smooth_tool_shows_fidelity_and_tool_options_for_a_path() {
    let mut app = app();
    let path = id(app.run("shape.ellipse", json!({"x": 60, "y": 60, "width": 120, "height": 80})).unwrap());
    select(&mut app, &[path]);
    app.run("tool.select", json!({"tool": "smooth"})).unwrap();
    on_canvas(&mut app, |rig| {
        assert!(rig.target("Tool Options…").is_some() && rig.target("More options").is_some());
        let field = rig.field_after("Fidelity:");
        rig.enter_value(field, "7");
    });
    assert_eq!(app.session.tool_options()["fidelity"].as_f64(), Some(7.0));
    assert_eq!(app.session.tool_id(), "smooth");
    assert_eq!(app.session.active().unwrap().selection.objects, [NodeId(path)], "the selection is kept");
    on_canvas(&mut app, |rig| {
        rig.click("Tool Options…");
        let open = rig.app().dialogs().map(|d| d.with(|host| host.open_dialogs())).unwrap_or_default();
        assert!(open.iter().any(|d| d.key.as_deref() == Some(crate::dialogs::freehand::KIND)), "the Smooth tool's options");
    });
}

/// After a switch to another document and back, the task bar shows the actions for the active
/// document's selection: an open path's Join in the first, a rectangle's Shape Properties in the second.
#[test]
fn the_task_bar_follows_the_active_document() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    assert!(shows(&offer(&mut app).unwrap(), "Join"));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
    select(&mut app, &[rect]);
    let o = offer(&mut app).unwrap();
    assert!(!shows(&o, "Join") && shows(&o, "Duplicate"), "{:?}", names_of(&o));
    app.session.set_active(0);
    assert!(shows(&offer(&mut app).unwrap(), "Join"));
    app.session.set_active(1);
    assert!(!shows(&offer(&mut app).unwrap(), "Join"));
    on_canvas(&mut app, |rig| {
        rig.click("Shape Properties");
        assert!(rig.has_text("Rectangle Properties:"));
    });
    assert!(!locked(&app, rect), "the button doesn't lock the shape");
}

/// Draw (Pencil) switches to the Pencil tool; Place Image is offered beside it.
#[test]
fn the_blank_canvas_bar_draws_with_the_pencil_and_places_an_image() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    // PLAN: the pick Place Image runs belongs to the file service sets (fl.A1); only the offer is asserted here.
    assert!(shows(&offer(&mut app).unwrap(), "Place Image"));
    click(&mut app, "Draw (Pencil)");
    assert_eq!(app.session.tool_id(), "pencil");
}

/// The bar's Font Size sets the characters a Type tool selects, one undo step; with type selected it
/// sets every selected type object, as the Character panel's does.
#[test]
fn the_task_bars_font_size_sets_what_the_character_panel_would() {
    let mut app = app();
    let t = edited_type(&mut app, "type");
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    on_canvas(&mut app, |rig| rig.type_into("20 pt", "30"));
    assert_eq!(sizes(&app, t), [("He".to_string(), 30.0), ("llo".to_string(), 20.0)]);
    // Mixed sizes leave the field blank, as in the Character panel.
    app.session.set_tool_option("select", &json!({"start": 0, "end": 5}));
    let blank = on_canvas(&mut app, |rig| !rig.has_text("20 pt") && !rig.has_text("30 pt"));
    assert!(blank, "mixed sizes leave the field blank");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sizes(&app, t), [("Hello".to_string(), 20.0)]);

    let mut app = self::app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let a = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello", "size": 20})).unwrap());
    let b = id(app.run("text.create", json!({"x": 60, "y": 160, "text": "World", "size": 20})).unwrap());
    select(&mut app, &[a, b]);
    on_canvas(&mut app, |rig| rig.type_into("20 pt", "14"));
    assert_eq!(sizes(&app, a), [("Hello".to_string(), 14.0)]);
    assert_eq!(sizes(&app, b), [("World".to_string(), 14.0)]);
}

/// At a caret, with no characters selected, the bar's Font Size sets the edited type as the Character
/// panel's does, and the characters typed next take it.
#[test]
fn the_task_bars_font_size_at_a_caret_sets_what_is_typed_next() {
    let mut app = app();
    let t = edited_type(&mut app, "type");
    app.session.set_tool_option("select", &json!({"start": 5, "end": 5}));
    assert_eq!(crate::panels::character::text_editing(&app), Some((NodeId(t), 5, 5)), "a caret after the type");
    on_canvas(&mut app, |rig| rig.type_into("20 pt", "30"));
    app.session.tool_typed_text("!", app.view_info()).unwrap();
    assert_eq!(sizes(&app, t), [("Hello!".to_string(), 30.0)]);
}

/// The Type tool's Fill chip makes the Fill active and keeps the type in edit.
#[test]
fn the_type_tools_fill_chip_activates_the_fill() {
    let mut app = app();
    edited_type(&mut app, "type");
    app.session.fill_active = false;
    click(&mut app, "Fill");
    assert!(app.session.fill_active, "the Fill is active");
    assert!(crate::panels::character::text_editing(&app).is_some(), "the type is still edited");
}

/// A click on the Stroke chip brings the Stroke proxy forward and shows Swatches, as a click on the
/// Fill chip does for the Fill proxy.
#[test]
fn the_task_bar_shows_a_stroke_chip_beside_fill() {
    let mut app = app();
    let r = id(app.run("shape.rectangle", json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap());
    app.run("paint.setFill", json!({"ids": [r], "color": "#cc3322"})).unwrap();
    app.run("paint.setStroke", json!({"ids": [r], "color": "#2244cc"})).unwrap();
    app.select_tool("selection");
    select(&mut app, &[r]);
    for (chip, fill_active) in [("Stroke", false), ("Fill", true)] {
        app.run("window.panel.close", json!({"panel": "swatches"})).unwrap();
        assert!(!crate::panel_docking::showing(&app.ui, "swatches"));
        app.session.fill_active = !fill_active;
        click(&mut app, chip);
        assert_eq!(app.session.fill_active, fill_active, "the {chip} proxy is in front");
        assert!(crate::panel_docking::showing(&app.ui, "swatches"), "{chip} shows Swatches");
    }
}

/// The Fill and Stroke chips show the paints the proxies show: a group's painted art, and "?" where
/// the selected objects' paints differ. A click on a chip shows Swatches wherever it is, closed included.
#[test]
fn the_task_bar_chips_follow_the_proxies_and_show_swatches_anywhere() {
    let mut app = app();
    let mut square = |x: f64, stroke: &str| {
        let r = id(app.run("shape.rectangle", json!({"x": x, "y": 130, "width": 40, "height": 40})).unwrap());
        app.run("paint.setFill", json!({"ids": [r], "color": "#cc3322"})).unwrap();
        app.run("paint.setStroke", json!({"ids": [r], "color": stroke})).unwrap();
        r
    };
    let first = square(160.0, "#2244cc");
    let second = square(220.0, "#22cc44");
    app.select_tool("selection");
    let ctx = crate::testing::ui_context();
    // Two strokes that differ: "?", as the proxy shows them.
    select(&mut app, &[first, second]);
    let (_, _, _, stroke_mixed) = chip_paints(&app, &ctx);
    assert!(stroke_mixed, "the Stroke chip shows the strokes differ");
    // A group shows its painted art's stroke, as the proxy does.
    select(&mut app, &[first]);
    let (_, blue, _, _) = chip_paints(&app, &ctx);
    select(&mut app, &[second]);
    let (_, green, _, _) = chip_paints(&app, &ctx);
    app.run("paint.setStroke", json!({"ids": [second], "color": "#2244cc"})).unwrap();
    select(&mut app, &[first, second]);
    app.run("object.group", json!({})).unwrap();
    let (_, group, mixed, stroke_mixed) = chip_paints(&app, &ctx);
    assert!(group == blue && group != green && !mixed && !stroke_mixed, "the Stroke chip shows the group's art's stroke");
    // Swatches closed: a click on the Stroke chip shows it.
    app.run("window.panel.close", json!({"panel": "swatches"})).unwrap();
    assert!(!crate::panel_docking::showing(&app.ui, "swatches"));
    click(&mut app, "Stroke");
    assert!(!app.session.fill_active, "the Stroke proxy is in front");
    assert!(crate::panel_docking::showing(&app.ui, "swatches"), "the Swatches panel shows");
}

/// While a Type tool edits type, the Fill chip shows the selected characters' fill, and "?" where
/// their fills differ.
#[test]
fn the_type_tools_fill_chip_shows_the_edited_characters_fill() {
    let mut app = app();
    let t = edited_type(&mut app, "type");
    app.session.set_tool_option("select", &json!({"start": 2, "end": 5}));
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let fills: Vec<_> = match kind(&app, t) {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.fill.clone())).collect(),
        k => panic!("{k:?}"),
    };
    let [(he, he_fill), (llo, llo_fill)] = &fills[..] else { panic!("{fills:?}") };
    assert_eq!((he.as_str(), llo.as_str()), ("He", "llo"));
    assert_ne!(he_fill, llo_fill);
    let ctx = crate::testing::ui_context();
    let (fill, _, mixed, _) = chip_paints(&app, &ctx);
    assert!(fill == *llo_fill && !mixed, "{fill:?} {mixed}");
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    let (fill, _, mixed, _) = chip_paints(&app, &ctx);
    assert!(fill == *he_fill && !mixed, "{fill:?} {mixed}");
    app.session.set_tool_option("select", &json!({"start": 0, "end": 5}));
    let (_, _, mixed, _) = chip_paints(&app, &ctx);
    assert!(mixed, "the fills differ");
}

/// The task bar shows the selection as it is after an edit and after Undo: Join leaves once the open
/// path is closed and comes back with Undo, and Shape Properties shows a rectangle's new corner radius
/// and then its old one.
#[test]
fn the_task_bar_follows_edits_and_undo() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    assert!(shows(&offer(&mut app).unwrap(), "Join"));
    app.run("path.join", json!({})).unwrap();
    assert!(!shows(&offer(&mut app).unwrap(), "Join"));
    app.run("edit.undo", json!({})).unwrap();
    assert!(shows(&offer(&mut app).unwrap(), "Join"));

    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60, "radius": 10})).unwrap());
    select(&mut app, &[rect]);
    let radius = |app: &mut VectorcraftApp, shown: &str, other: &str| {
        on_canvas(app, |rig| {
            rig.click("Shape Properties");
            assert!(rig.has_text(shown) && !rig.has_text(other), "{shown} replaces {other}");
        })
    };
    radius(&mut app, "10 pt", "20 pt");
    app.run("object.setLiveShape", json!({"radius": 20})).unwrap();
    radius(&mut app, "20 pt", "10 pt");
    app.run("edit.undo", json!({})).unwrap();
    radius(&mut app, "10 pt", "20 pt");
    assert!(!locked(&app, rect), "the button doesn't lock the shape");
}

/// An action turned off in Bar Actions leaves the bar for every selection that offers it, stays listed
/// in Bar Actions, and its Object menu command stays enabled. Toggled, it comes back.
#[test]
fn an_action_turned_off_leaves_the_bar_and_stays_in_its_menu() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    let o = offer(&mut app).unwrap();
    assert!(shows(&o, "Offset Path") && shows(&o, "Join"), "{:?}", names_of(&o));
    show_action(&mut app, "offsetPath", false);
    assert_eq!(crate::menus::checked(&app, "window.taskBar.actions", &json!({"action": "offsetPath"})), Some(false));
    assert_eq!(crate::menus::checked(&app, "window.taskBar.actions", &json!({"action": "join"})), Some(true));
    let o = offer(&mut app).unwrap();
    assert!(!shows(&o, "Offset Path"), "{:?}", names_of(&o));
    assert!(shows(&o, "Join") && shows(&o, "Simplify") && shows(&o, "Edit Path"), "the other actions stay: {:?}", names_of(&o));
    assert!(o.listed.iter().any(|l| l.1 == Some("offsetPath")), "it stays listed in Bar Actions");
    assert!(crate::menus::enabled(&app, "object.path.offsetPath"), "Object › Path › Offset Path… still runs");

    // Another selection that offers the same action leaves it out too: a rectangle.
    let rect = id(app.run("shape.rectangle", json!({"x": 220, "y": 40, "width": 80, "height": 60})).unwrap());
    select(&mut app, &[rect]);
    let o = offer(&mut app).unwrap();
    assert!(!shows(&o, "Offset Path") && shows(&o, "Edit Path"), "{:?}", names_of(&o));

    // Toggled (no `visible`), it shows again.
    assert_eq!(app.run("window.taskBar.actions", json!({"action": "offsetPath"})).unwrap(), json!({"action": "offsetPath", "visible": true}));
    assert!(shows(&offer(&mut app).unwrap(), "Offset Path"));
    // A `visible` of null counts as omitted and toggles too.
    let r = app.run("window.taskBar.actions", json!({"action": "offsetPath", "visible": null})).unwrap();
    assert_eq!(r, json!({"action": "offsetPath", "visible": false}));
}

/// The Fill, Stroke, Recolor Artwork, Lock and Shape Properties chips each turn off on their own: Lock
/// off leaves a basic shape's Shape Properties, and Shape Properties off leaves Lock.
#[test]
fn a_chip_turned_off_leaves_the_bar() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let rect = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 150, "x2": 200, "y2": 220})).unwrap());
    select(&mut app, &[rect]);
    let o = offer(&mut app).unwrap();
    for what in ["fill", "stroke", "shapeProperties"] {
        assert!(shows(&o, what), "{what}: {:?}", names_of(&o));
    }
    show_action(&mut app, "fill", false);
    show_action(&mut app, "lock", false);
    let o = offer(&mut app).unwrap();
    assert!(!shows(&o, "fill") && shows(&o, "stroke") && shows(&o, "shapeProperties"), "{:?}", names_of(&o));
    select(&mut app, &[line]);
    assert!(!shows(&offer(&mut app).unwrap(), "lock"), "Lock is off");
    show_action(&mut app, "lock", true);
    show_action(&mut app, "shapeProperties", false);
    assert!(shows(&offer(&mut app).unwrap(), "lock"), "Lock is on again");
    select(&mut app, &[rect]);
    assert!(!shows(&offer(&mut app).unwrap(), "shapeProperties"));
}

/// Reset Bar Actions shows every action and chip again and returns the ids it showed.
#[test]
fn reset_bar_actions_shows_every_action_again() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    show_action(&mut app, "simplify", false);
    show_action(&mut app, "duplicate", false);
    show_action(&mut app, "stroke", false);
    let o = offer(&mut app).unwrap();
    assert!(!shows(&o, "Simplify") && !shows(&o, "Duplicate") && !shows(&o, "stroke"), "{:?}", names_of(&o));
    assert_eq!(app.run("window.taskBar.resetActions", json!({})).unwrap(), json!(["duplicate", "simplify", "stroke"]));
    assert!(app.ui.task_bar_hidden_actions.is_empty());
    let o = offer(&mut app).unwrap();
    assert!(shows(&o, "Simplify") && shows(&o, "Duplicate"), "{:?}", names_of(&o));
    assert_eq!(app.run("window.taskBar.resetActions", json!({})).unwrap(), json!([]), "nothing left to show");
}

/// Each of the bar's actions and chips has its own id, and no two actions run the same command with
/// the same tool.
#[test]
fn bar_action_ids_are_distinct() {
    let ids: Vec<&str> = canvas::TASK_BAR_ACTIONS.iter().map(|a| a.0).chain(canvas::TASK_BAR_CHIPS).collect();
    let set: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
    assert_eq!(set.len(), ids.len(), "{ids:?}");
    let runs: std::collections::BTreeSet<(&str, Option<&str>)> = canvas::TASK_BAR_ACTIONS.iter().map(|a| (a.1, a.2)).collect();
    assert_eq!(runs.len(), canvas::TASK_BAR_ACTIONS.len());
}

/// An action the bar does not have, a missing one, or a `visible` that is not true, false or null is an
/// error and changes nothing.
#[test]
fn bar_actions_refuse_bad_params() {
    let mut app = app();
    show_action(&mut app, "group", false);
    for p in [json!({"action": "teleport"}), json!({}), json!({"action": 3}), json!({"action": "unite", "visible": "yes"})] {
        assert!(app.run("window.taskBar.actions", p.clone()).is_err(), "{p}");
    }
    let err = app.run("window.taskBar.actions", json!({"action": "teleport"})).unwrap_err();
    assert!(err.contains("imageTrace") && err.contains("shapeProperties"), "{err}");
    assert_eq!(app.ui.task_bar_hidden_actions.iter().map(String::as_str).collect::<Vec<_>>(), ["group"]);
    assert_eq!(crate::menus::checked(&app, "window.taskBar.actions", &json!({"action": "teleport"})), Some(false));
}

/// The actions that are off are kept with the preferences. Ids the bar does not have are dropped, a
/// value that is not a list turns none off, and older preferences without it show every action.
#[test]
fn bar_actions_are_kept_with_the_preferences() {
    let mut app = app();
    show_action(&mut app, "unite", false);
    show_action(&mut app, "recolor", false);
    let mut prefs = serde_json::to_value(&app.ui).unwrap();
    let back: crate::state::UiState = serde_json::from_value(prefs.clone()).unwrap();
    let ids = |ui: &crate::state::UiState| ui.task_bar_hidden_actions.iter().cloned().collect::<Vec<_>>();
    assert_eq!(ids(&back), ["recolor", "unite"]);
    prefs["task_bar_hidden_actions"] = json!(["unite", "teleport", 7, "fill"]);
    assert_eq!(ids(&serde_json::from_value(prefs.clone()).unwrap()), ["fill", "unite"], "ids the bar does not have are dropped");
    prefs["task_bar_hidden_actions"] = json!("unite");
    assert!(ids(&serde_json::from_value(prefs.clone()).unwrap()).is_empty(), "a value that is not a list turns none off");
    prefs.as_object_mut().unwrap().remove("task_bar_hidden_actions");
    assert!(ids(&serde_json::from_value(prefs).unwrap()).is_empty(), "older preferences show every action");
    app.run("window.taskBar.resetActions", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&app.ui).unwrap()["task_bar_hidden_actions"], json!([]), "none off");
}

/// More › Bar Actions and Reset Bar Actions are rows of the bar's More menu; Reset Bar Actions shows
/// an action that was turned off again. The Bar Actions rows run `window.taskBar.actions`, whose
/// effect `an_action_turned_off_leaves_the_bar_and_stays_in_its_menu` covers.
#[test]
fn more_options_bar_actions_turns_an_action_off_and_on() {
    let mut app = app();
    app.run("tool.select", json!({"tool": "selection"})).unwrap();
    let line = id(app.run("shape.line", json!({"x1": 50, "y1": 50, "x2": 200, "y2": 120})).unwrap());
    select(&mut app, &[line]);
    show_action(&mut app, "offsetPath", false);
    assert!(!shows(&offer(&mut app).unwrap(), "Offset Path"));
    on_canvas(&mut app, |rig| {
        rig.click("More options");
        assert!(rig.target("Bar Actions").is_some(), "More lists Bar Actions");
        rig.click("Reset Bar Actions");
    });
    assert!(app.ui.task_bar_hidden_actions.is_empty(), "{:?}", app.ui.task_bar_hidden_actions);
    assert!(shows(&offer(&mut app).unwrap(), "Offset Path"));
}

/// Every action and chip the bar offers, for every kind of selection and tool, has an id in Bar
/// Actions, and with all of them turned off the bar shows none of them but still shows, with More
/// options. That covers Connect and Join (both `join`), the actions matched by the tool they switch to
/// (`editPath`, `drawPencil`, `smoothToolOptions`), the Smooth tool's Fidelity, Dynamic Text, an Image
/// Trace object's Expand, the Type tool's Fill with type selected and while it edits type, and the
/// Artboard tool's actions.
#[test]
fn every_action_the_bar_offers_can_be_turned_off() {
    let dir = crate::tests_folder::Folder::new("ui-taskbar-actions");
    type Setup<'a> = Box<dyn Fn(&mut VectorcraftApp) + 'a>;
    let cases: Vec<(&str, Setup, &[&str])> = vec![
        (
            "an open path",
            Box::new(|app| {
                let l = open_line(app);
                select(app, &[l]);
            }),
            &["join", "editPath", "offsetPath", "simplify", "duplicate"],
        ),
        (
            "a path the Direct Selection tool selects whole",
            Box::new(|app| {
                let l = open_line(app);
                select(app, &[l]);
                app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
            }),
            &["join", "convertToCorner", "convertToSmooth", "simplify", "smooth", "duplicate"],
        ),
        (
            "anchors picked out",
            Box::new(|app| {
                let l = open_line(app);
                app.run("tool.select", json!({"tool": "directSelection"})).unwrap();
                app.run("select.anchors", json!({"id": l, "anchors": [[0, 1]]})).unwrap();
            }),
            &["removeAnchorPoints", "join", "cutPath", "convertToCorner", "convertToSmooth"],
        ),
        (
            "two paths",
            Box::new(|app| {
                let ids = two_shapes(app);
                select(app, &ids);
            }),
            &["group", "unite", "duplicate"],
        ),
        (
            "a group",
            Box::new(|app| {
                let ids = two_shapes(app);
                select(app, &ids);
                app.run("object.group", json!({})).unwrap();
            }),
            &["ungroup", "isolate", "duplicate"],
        ),
        (
            "a compound path",
            Box::new(|app| {
                compound(app);
            }),
            &["editPath", "releaseCompoundPath", "duplicate"],
        ),
        (
            "a clipping group",
            Box::new(|app| {
                clipping_group(app);
            }),
            &["editClippingPath", "releaseClippingMask", "duplicate"],
        ),
        (
            "point type",
            Box::new(|app| {
                let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello"})).unwrap());
                select(app, &[t]);
            }),
            &["createOutlines", "convertToAreaType", "dynamicText", "fill", "stroke", "lock"],
        ),
        (
            "the Type tool with type selected",
            Box::new(|app| {
                let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello"})).unwrap());
                select(app, &[t]);
                app.run("tool.select", json!({"tool": "type"})).unwrap();
            }),
            &["fill"],
        ),
        (
            "the Type tool editing type",
            Box::new(|app| {
                edited_type(app, "type");
            }),
            &["fill"],
        ),
        (
            "area type",
            Box::new(|app| {
                let t = id(app.run("text.create", json!({"x": 60, "y": 80, "text": "Hello"})).unwrap());
                select(app, &[t]);
                app.run("type.convertToAreaType", json!({})).unwrap();
                select(app, &[t]);
            }),
            &["createOutlines", "convertToPointType", "dynamicText"],
        ),
        (
            "type on a closed path",
            Box::new(|app| {
                let t = path_type(app, true);
                select(app, &[t]);
            }),
            &["createOutlines", "typeOnPathOptions", "centerOnTop", "centerOnBottom", "dynamicText"],
        ),
        (
            "an Image Trace object",
            Box::new(|app| {
                place_image(app, &dir);
                app.run("imageTrace.make", json!({"params": {}})).unwrap();
            }),
            &["expand", "duplicate"],
        ),
        (
            "an image",
            Box::new(|app| {
                place_image(app, &dir);
            }),
            &["imageTrace", "cropImage", "removeBackground", "duplicate"],
        ),
        (
            "an image with a path",
            Box::new(|app| {
                let img = place_image(app, &dir);
                let r = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap());
                select(app, &[img, r]);
            }),
            &["group", "makeClippingMask", "duplicate"],
        ),
        ("a blank canvas", Box::new(|_| {}), &["drawPencil", "placeImage"]),
        (
            "a shape tool after drawing",
            Box::new(|app| {
                drag_with(app, "rectangle", (60.0, 60.0), (120.0, 120.0));
            }),
            &["editPath"],
        ),
        (
            "the Smooth tool",
            Box::new(|app| {
                let e = id(app.run("shape.ellipse", json!({"x": 60, "y": 60, "width": 120, "height": 80})).unwrap());
                select(app, &[e]);
                app.run("tool.select", json!({"tool": "smooth"})).unwrap();
            }),
            &["fidelity", "smoothToolOptions"],
        ),
        (
            "the Artboard tool",
            Box::new(|app| {
                app.run("artboard.new", json!({"x": 420, "y": 0, "width": 200, "height": 200})).unwrap();
                app.run("tool.select", json!({"tool": "artboard"})).unwrap();
            }),
            &["resizeArtboard", "duplicateArtboards", "rearrangeArtboards", "exportArtboards"],
        ),
    ];
    for (case, setup, expected) in cases {
        let mut app = app();
        app.run("tool.select", json!({"tool": "selection"})).unwrap();
        setup(&mut app);
        let o = offer(&mut app).unwrap_or_else(|| panic!("{case}: the bar shows"));
        assert!(!o.listed.is_empty(), "{case}: the bar lists actions");
        for (label, action, _) in &o.listed {
            assert!(action.is_some(), "{case}: {label} has no id in Bar Actions");
        }
        let ids: Vec<&str> = o.listed.iter().filter_map(|l| l.1).collect();
        for action in expected {
            assert!(ids.contains(action), "{case}: {action} in {ids:?}");
        }
        for action in &ids {
            show_action(&mut app, action, false);
        }
        let off = offer(&mut app).unwrap_or_else(|| panic!("{case}: the bar still shows"));
        assert!(off.actions.is_empty() && off.chips.is_empty(), "{case}: every action is off: {:?}", names_of(&off));
        assert_eq!(off.listed.len(), o.listed.len(), "{case}: Bar Actions still lists them");
        // The bar still shows, with More options, where Bar Actions turns them back on.
        let more = on_canvas(&mut app, |rig| rig.target(BAR).is_some() && rig.target("More options").is_some());
        assert!(more, "{case}: the bar still shows its More options");
    }
}
