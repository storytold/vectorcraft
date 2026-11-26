//! Copy actual information-panel readouts without replacing them with artwork SVG.
//! These frames prove host clipboard intent; they do not access the OS clipboard.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use craft::platform::clipboard::{Clipboard, ClipboardBackend, ClipboardError, ClipboardImage, Flavor};
use craft::ui::runtime::frame::take_output;
use craft::ui::shell::embed;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Shape, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{Services, VectorcraftApp, background};

const COPIED: &str = "Copied the value";
const LENGTH: &str = "234 pt";

struct Board(Arc<Mutex<Vec<Vec<Flavor>>>>);

impl ClipboardBackend for Board {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        self.0.lock().unwrap().push(flavors.to_vec());
        Ok(0)
    }

    fn read(&mut self, _: &[&str], _: u64) -> Result<Option<Flavor>, ClipboardError> {
        Ok(None)
    }

    fn read_image(&mut self, _: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        Ok(None)
    }

    fn read_files(&mut self, _: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        Ok(None)
    }

    fn has(&mut self, _: &[&str]) -> bool {
        false
    }

    fn owns(&mut self) -> Option<bool> {
        Some(false)
    }

    fn readable_any_time(&self) -> bool {
        true
    }
}

struct App {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
}

impl App {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 150, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 37, "height": 40})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
        let mut a = Self { app, ctx, time: 0.0 };
        a.settle();
        a
    }

    fn frame_after(&mut self, seconds: f64, events: Vec<Event>) -> egui::FullOutput {
        self.time += seconds;
        let mut raw = egui::RawInput {
            events,
            time: Some(self.time),
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1000.0))),
            ..Default::default()
        };
        self.app.adopt_context(&self.ctx);
        self.app.raw_input(&self.ctx, &mut raw);
        embed::before_input(&self.ctx, |out| self.app.frame_input(out));
        let mut out = self.ctx.run_ui(raw, |ui| {
            self.app.tick(ui.ctx());
            self.app.ui(ui);
        });
        let _ = self.app.frame_input(take_output(&self.ctx));
        out.textures_delta.clear();
        out
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.frame_after(0.02, events)
    }

    fn settle(&mut self) -> egui::FullOutput {
        let mut out = self.frame(vec![]);
        for _ in 0..3 {
            out = self.frame(vec![]);
        }
        out
    }

    fn delay(&self) -> f64 {
        self.ctx.options(|o| o.input_options.max_double_click_delay)
    }

    fn deadline(&mut self) -> egui::FullOutput {
        self.frame_after(self.delay() + 0.05, vec![])
    }

    fn snapshot(&self) -> Value {
        let st = self.app.session.active().unwrap();
        json!({"doc": st.doc, "selection": st.selection, "undo": st.history.undo.len(), "redo": st.history.redo.len()})
    }

    fn tap(&mut self, at: Pos2) -> egui::FullOutput {
        self.frame(vec![Event::PointerMoved(at), button(at, true)]);
        self.frame(vec![button(at, false)])
    }

    fn panel(&mut self, panel: &str) -> egui::FullOutput {
        self.app.run("window.panel", json!({"panel": panel})).unwrap();
        self.settle()
    }

    fn close_panel(&mut self, panel: &str) {
        self.app.run("window.panel.close", json!({"panel": panel})).unwrap();
        self.settle();
    }

    fn document_category(&mut self, category: &str) -> egui::FullOutput {
        self.panel("docInfo");
        let id = vectorcraft_engine::cmd::CATEGORIES.iter().find(|(_, label)| *label == category).unwrap().0;
        // These frames exercise the readout, not the removed egui menu's layout.
        self.app.run("ui.docInfo.category", json!({"category": id})).unwrap();
        self.settle()
    }
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn copies(out: &egui::FullOutput) -> Vec<String> {
    out.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(s) = c { Some(s.clone()) } else { None }).collect()
}

fn text_shape<'a>(shape: &'a Shape, text: &str) -> Option<&'a egui::epaint::TextShape> {
    match shape {
        Shape::Text(t) if t.galley.text() == text => Some(t),
        Shape::Vec(v) => v.iter().rev().find_map(|s| text_shape(s, text)),
        _ => None,
    }
}

fn painted(out: &egui::FullOutput, text: &str) -> (Rect, bool) {
    out.shapes
        .iter()
        .rev()
        .find_map(|s| {
            let t = text_shape(&s.shape, text)?;
            let r = t.galley.rect.translate(t.pos.to_vec2());
            s.clip_rect.intersects(r).then_some((r.intersect(s.clip_rect), t.galley.elided))
        })
        .unwrap_or_else(|| panic!("fixture: no visible actual readout {text:?}"))
}

fn visible_text_count(out: &egui::FullOutput, text: &str) -> usize {
    fn count(shape: &Shape, clip: Rect, text: &str) -> usize {
        match shape {
            Shape::Text(t) if t.galley.text() == text => usize::from(t.galley.rect.translate(t.pos.to_vec2()).intersect(clip).is_positive()),
            Shape::Vec(v) => v.iter().map(|s| count(s, clip, text)).sum(),
            _ => 0,
        }
    }
    out.shapes.iter().map(|s| count(&s.shape, s.clip_rect, text)).sum()
}

fn copy_at(a: &mut App, at: Pos2, value: &str) -> egui::FullOutput {
    let before = a.snapshot();
    let release = a.tap(at);
    // #9353 copies the first click immediately; a later click selects the word.
    let commit = a.deadline();
    let next = a.frame(vec![]);
    let mut all = copies(&release);
    all.extend(copies(&commit));
    all.extend(copies(&next));
    assert_eq!(all, [value], "exactly one full value, never the art's SVG");
    assert_eq!(visible_text_count(&next, COPIED), 1, "one actual nonmodal confirmation is painted");
    assert_eq!(a.snapshot(), before, "readout copy leaves document, selection and history intact");
    assert!(copies(&a.frame(vec![])).is_empty(), "no copy on the next frame");
    next
}

fn drag_text(a: &mut App, rect: Rect) {
    let from = rect.left_center() + vec2(1.0, 0.0);
    let to = rect.right_center() - vec2(1.0, 0.0);
    assert!(copies(&a.frame(vec![Event::PointerMoved(from), button(from, true)])).is_empty());
    for k in 1..=4 {
        assert!(copies(&a.frame(vec![Event::PointerMoved(from + (to - from) * k as f32 / 4.0)])).is_empty());
    }
    assert!(copies(&a.frame(vec![button(to, false)])).is_empty());
    assert!(copies(&a.deadline()).is_empty(), "drag selection never auto-copies");
}

#[test]
fn panel_readout_existing_label_keyboard_and_art_copy_remain_independent() {
    let mut a = App::new();
    let out = a.document_category("Objects");
    let before = a.snapshot();
    drag_text(&mut a, painted(&out, LENGTH).0);
    for event in [Event::Copy, Event::Cut] {
        let out = a.frame(vec![Event::ModifiersChanged(Modifiers::COMMAND), event]);
        assert_eq!(copies(&out), [LENGTH], "the existing label selection owns Copy and Cut");
        assert!(copies(&a.frame(vec![Event::ModifiersChanged(Modifiers::NONE)])).is_empty());
        assert!(a.app.session.clipboard.is_empty(), "the art was not copied or cut");
        assert_eq!(a.snapshot(), before);
    }
    a.close_panel("docInfo");
    let first = a.frame(vec![Event::Copy]);
    let next = a.frame(vec![]);
    let mut copied = copies(&first);
    copied.extend(copies(&next));
    assert_eq!(copied.len(), 1, "ordinary art Copy still runs when no label consumes it");
    assert!(copied[0].contains("<svg"));
    assert!(!a.app.session.clipboard.is_empty());
    assert_eq!(a.snapshot(), before);
}

fn hold_art_formats(a: &mut App) -> (Arc<Mutex<Vec<Vec<Flavor>>>>, mpsc::Sender<()>) {
    let offers = Arc::new(Mutex::new(Vec::new()));
    a.app.clipboard = Clipboard::new(Board(offers.clone()));
    a.app.services.write_shared = Some(Arc::new(|_: &str, _: &[u8]| Err("no fixture file writes".into())));
    let (hold, held) = mpsc::channel();
    background::run(
        &mut a.app,
        true,
        String::new(),
        None,
        move |_| {
            let _ = held.recv_timeout(Duration::from_secs(10));
            Ok(Value::Null)
        },
        |_, r| r,
    )
    .unwrap();
    a.app.run("edit.copy", json!({})).unwrap();
    assert!(!a.app.session.clipboard.is_empty(), "ordinary art Copy really queued its formats");
    assert!(offers.lock().unwrap().is_empty(), "the existing worker holds those formats");
    (offers, hold)
}

#[test]
fn panel_readout_document_value_copies_once_with_feedback_and_cancels_pending_art() {
    let mut a = App::new();
    let out = a.document_category("Objects");
    let (offers, hold) = hold_art_formats(&mut a);
    let before = a.snapshot();
    let release = a.tap(painted(&out, LENGTH).0.center());
    let commit = a.deadline();
    let copied = a.frame(vec![]);
    let expected = vec![vec![Flavor::text(LENGTH)]];
    assert_eq!(*offers.lock().unwrap(), expected, "one actual host offer contains the complete readout text");
    assert!(copies(&release).is_empty() && copies(&commit).is_empty() && copies(&copied).is_empty(), "the injected host handles this copy without duplicate fallback output");
    assert_eq!(visible_text_count(&copied, COPIED), 1, "one actual nonmodal confirmation is painted");
    assert_eq!(a.snapshot(), before, "readout copy leaves document, selection and history intact");
    assert!(copies(&a.frame(vec![])).is_empty(), "no fallback copy on the next frame");
    assert_eq!(*offers.lock().unwrap(), expected, "an idle frame adds no host offer");
    drop(hold);
    background::wait_all(&mut a.app);
    assert_eq!(*offers.lock().unwrap(), expected, "unwritten art formats cannot append an SVG, PDF, image or text offer over the readout");
    assert_eq!(a.snapshot(), before);
    let next = a.frame(vec![]);
    assert_eq!(visible_text_count(&next, COPIED), 1, "no duplicate confirmation was queued");
    assert!(copies(&next).is_empty());
}

fn dismiss_feedback(a: &mut App, out: &egui::FullOutput) {
    assert_eq!(visible_text_count(out, COPIED), 1);
    a.app.ui.status.clear();
    assert_eq!(visible_text_count(&a.frame(vec![]), COPIED), 0, "the previous confirmation cannot satisfy the next readout's oracle");
}

fn copy_graphic_style_name(a: &mut App) {
    let name = "Original information-panel style";
    a.app.run("graphicStyle.new", json!({"name": name})).unwrap();
    let out = a.document_category("Graphic Styles");
    let copied = copy_at(a, painted(&out, name).0.center(), name);
    dismiss_feedback(a, &copied);
    a.close_panel("docInfo");
}

fn place_original_png(a: &mut App, name: &str) {
    let pixels = image::RgbaImage::from_pixel(8, 8, image::Rgba([17, 54, 128, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    a.app.run("file.place", json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes.get_ref()), "link": false})).unwrap();
}

fn copy_long_link_value(a: &mut App) {
    let name = format!("{}.png", "Original linked-information fixture ".repeat(6));
    place_original_png(a, &name);
    let out = a.panel("links");
    let label = painted(&out, "Name").0;
    let (value, clipped) = out
        .shapes
        .iter()
        .find_map(|s| {
            let t = text_shape(&s.shape, &name)?;
            let r = t.galley.rect.translate(t.pos.to_vec2()).intersect(s.clip_rect);
            (r.is_positive() && (r.center().y - label.center().y).abs() < 8.0).then_some((r, t.galley.elided || r.width() < t.galley.rect.width()))
        })
        .expect("the actual selected image's Link Info Name value");
    assert!(clipped, "the framework really clips this long painted readout");
    copy_at(a, value.center(), &name);
}

#[test]
fn panel_readout_info_names_and_truncated_link_metadata_copy_full_displayed_text() {
    let mut a = App::new();
    let out = a.panel("info");
    let width = "W: 37 pt";
    let copied = copy_at(&mut a, painted(&out, width).0.center(), width);
    dismiss_feedback(&mut a, &copied);
    a.close_panel("info");
    copy_graphic_style_name(&mut a);
    copy_long_link_value(&mut a);
}

fn assert_no_delayed_copy(a: &mut App, kind: &str) {
    let copied = copies(&a.deadline());
    assert!(copied.is_empty(), "{kind}: unexpected delayed copy {copied:?}");
    assert!(copies(&a.frame(vec![])).is_empty(), "{kind}: no later duplicate copy");
}

fn cancelled_gesture(kind: &str) {
    let mut a = App::new();
    let out = a.document_category("Objects");
    let at = painted(&out, LENGTH).0.center();
    let before = a.snapshot();
    match kind {
        "outside" => {
            a.frame(vec![Event::PointerMoved(at), button(at, true)]);
            let outside = at + vec2(-80.0, 0.0);
            // A real release outside includes the pointer motion that egui uses to distinguish
            // dragging from a click; a button's release position alone does not record movement.
            let up = a.frame(vec![Event::PointerMoved(outside), button(outside, false)]);
            assert!(copies(&up).is_empty(), "release outside does not accept the value");
        }
        "double" => {
            assert_eq!(copies(&a.tap(at)), [LENGTH], "the first click follows the landed framework policy");
            let second = a.frame_after(a.delay() * 0.25, vec![button(at, true)]);
            assert!(copies(&second).is_empty());
            assert!(copies(&a.frame(vec![button(at, false)])).is_empty());
        }
        "hidden" => {
            assert_eq!(copies(&a.tap(at)), [LENGTH], "the first click follows the landed framework policy");
            a.close_panel("docInfo");
            assert_no_delayed_copy(&mut a, kind);
            a.panel("docInfo");
        }
        "changed" => {
            assert_eq!(copies(&a.tap(at)), [LENGTH], "the first click follows the landed framework policy");
            a.app.run("shape.rectangle", json!({"x": 70, "y": 70, "width": 1, "height": 1})).unwrap();
            let changed = a.frame(vec![]);
            painted(&changed, "238 pt");
        }
        _ => panic!("unknown fixture"),
    }
    assert_no_delayed_copy(&mut a, kind);
    if kind != "changed" {
        assert_eq!(a.snapshot(), before, "cancelled gestures do not edit the document or selection");
    }
}

#[test]
fn panel_readout_primary_gestures_and_accessible_copy_follow_framework_policy() {
    for kind in ["outside", "double", "hidden", "changed"] {
        cancelled_gesture(kind);
    }
    review_8826_motion_keeps_pending_copy();
    for button in [PointerButton::Secondary, PointerButton::Middle] {
        review_8826_pointer_takeover(button);
    }
    review_8826_accesskit_takeover(true);
    review_8826_accesskit_takeover(false);
}

fn review_8826_motion_keeps_pending_copy() {
    let mut a = App::new();
    let out = a.panel("info");
    let rect = painted(&out, "W: 37 pt").0;
    let before = a.snapshot();
    assert_eq!(copies(&a.tap(rect.center())), ["W: 37 pt"]);
    let at = rect.center() + vec2(1.0, 0.0);
    assert!(rect.contains(at), "ordinary motion stays over the same real readout");
    let moved = a.frame(vec![Event::PointerMoved(at)]);
    painted(&moved, "W: 37 pt");
    assert!(copies(&moved).is_empty());
    assert!(copies(&a.deadline()).is_empty(), "motion cannot repeat the completed primary copy");
    let next = a.frame(vec![]);
    assert!(copies(&next).is_empty());
    assert_eq!(visible_text_count(&next, COPIED), 1);
    assert_eq!(a.snapshot(), before);
}

fn review_8826_pointer_takeover(button: PointerButton) {
    let mut a = App::new();
    let out = a.panel("info");
    let at = painted(&out, "W: 37 pt").0.center();
    let before = a.snapshot();
    let release = a.tap(at);
    assert_eq!(copies(&release), ["W: 37 pt"]);
    let sets = a.app.ui.status_sets;
    painted(&release, "W: 37 pt");
    let press = a.frame_after(a.delay() * 0.25, vec![Event::PointerButton { pos: at, button, pressed: true, modifiers: Modifiers::NONE }]);
    assert!(copies(&press).is_empty(), "{button:?} press must not copy");
    painted(&press, "W: 37 pt");
    let release = a.frame(vec![Event::PointerButton { pos: at, button, pressed: false, modifiers: Modifiers::NONE }]);
    assert!(copies(&release).is_empty(), "{button:?} release must not schedule a copy");
    painted(&release, "W: 37 pt");
    assert_no_delayed_copy(&mut a, &format!("{button:?} takeover"));
    let still_shown = a.frame(vec![]);
    painted(&still_shown, "W: 37 pt");
    assert_eq!(a.app.ui.status_sets, sets, "another pointer action cannot repeat the confirmation");
    assert_eq!(a.snapshot(), before, "{button:?} takeover leaves document, selection and history intact");
}

fn review_8826_readout_node(out: &egui::FullOutput, value: &str) -> egui::accesskit::NodeId {
    let rect = painted(out, value).0;
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("actual readout accessibility tree").nodes;
    let matches: Vec<_> = nodes
        .iter()
        .filter(|(_, n)| n.label() == Some(value) || n.value() == Some(value))
        .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Click))
        .filter(|(_, n)| {
            n.bounds()
                .is_some_and(|b| Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)).intersects(rect))
        })
        .collect();
    assert_eq!(matches.len(), 1, "one live clickable node for the painted {value:?}");
    let (id, node) = matches[0];
    assert_eq!(node.description(), Some("Copy"), "the actual readout advertises its copy action");
    *id
}

fn review_8826_accesskit_takeover(same_target: bool) {
    use egui::accesskit::{Action, ActionRequest, TreeId};
    let mut a = App::new();
    let out = a.panel("info");
    let width = review_8826_readout_node(&out, "W: 37 pt");
    let height = review_8826_readout_node(&out, "H: 40 pt");
    assert_ne!(width, height, "the live W and H rows have distinct semantic targets");
    let before = a.snapshot();
    let release = a.tap(painted(&out, "W: 37 pt").0.center());
    assert_eq!(copies(&release), ["W: 37 pt"]);
    painted(&release, "W: 37 pt");
    painted(&release, "H: 40 pt");
    let (target_node, expected) = if same_target { (width, "W: 37 pt") } else { (height, "H: 40 pt") };
    let activated = a.frame_after(
        a.delay() * 0.25,
        vec![Event::AccessKitActionRequest(ActionRequest { action: Action::Click, target_node, target_tree: TreeId::ROOT, data: None })],
    );
    assert_eq!(copies(&activated), [expected], "the actual AccessKit target copies once immediately");
    painted(&activated, "W: 37 pt");
    painted(&activated, "H: 40 pt");
    assert_no_delayed_copy(&mut a, if same_target { "same-target accessible click" } else { "different-readout accessible click" });
    let still_shown = a.frame(vec![]);
    painted(&still_shown, "W: 37 pt");
    painted(&still_shown, "H: 40 pt");
    assert_eq!(a.snapshot(), before, "accessible copy leaves document, selection and history intact");
}
