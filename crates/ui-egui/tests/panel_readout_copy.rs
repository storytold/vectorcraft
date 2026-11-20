//! Copy actual information-panel readouts without replacing them with artwork SVG.
//! These frames prove host clipboard intent; they do not access the OS clipboard.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Shape, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::Flavour;
use vectorcraft_ui_egui::{Services, SystemClipboard, VectorcraftApp, background};

const COPIED: &str = "Copied to clipboard";
const LENGTH: &str = "234 pt";

struct Board(Rc<Cell<usize>>);

impl SystemClipboard for Board {
    fn write(&mut self, _: &[Flavour]) -> Result<(), String> {
        self.0.set(self.0.get() + 1);
        Ok(())
    }

    fn holds_ours(&mut self) -> bool {
        false
    }

    fn read(&mut self, _: &[&'static str]) -> Option<Flavour> {
        None
    }

    fn has(&mut self, _: &[&'static str]) -> bool {
        false
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
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut out = self.ctx.run_ui(raw, |ui| {
            self.app.logic(ui.ctx());
            self.app.ui(ui);
        });
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
        let out = self.panel("docInfo");
        let title = painted(&out, "Document Info").0;
        let update = out.platform_output.accesskit_update.as_ref().expect("actual panel accessibility");
        let at = update
            .nodes
            .iter()
            .filter(|(_, n)| n.label() == Some("Panel menu"))
            .filter_map(|(_, n)| n.bounds())
            .map(|b| Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
            .filter(|r| (r.center().y - title.center().y).abs() < 16.0 && r.center().x > title.center().x)
            .min_by(|a, b| a.center().distance(title.center()).total_cmp(&b.center().distance(title.center())))
            .expect("the Document Info title's own Panel menu")
            .center();
        self.tap(at);
        // A newly opened egui popup first measures an invisible sizing pass. Read its painted
        // category only after normal subsequent layout frames, as the user sees it.
        let menu = self.settle();
        let item = painted(&menu, category).0.center();
        self.tap(item);
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
    assert!(copies(&release).is_empty(), "the first release cannot yet exclude a double-click");
    let commit = a.deadline();
    let next = a.frame(vec![]);
    let mut all = copies(&commit);
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

fn hold_art_formats(a: &mut App) -> (Rc<Cell<usize>>, mpsc::Sender<()>) {
    let writes = Rc::new(Cell::new(0));
    a.app.services.system_clipboard = Some(Box::new(Board(writes.clone())));
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
    assert_eq!(writes.get(), 0, "the existing worker holds those formats");
    (writes, hold)
}

#[test]
fn panel_readout_document_value_copies_once_with_feedback_and_cancels_pending_art() {
    let mut a = App::new();
    let out = a.document_category("Objects");
    let (writes, hold) = hold_art_formats(&mut a);
    let before = a.snapshot();
    copy_at(&mut a, painted(&out, LENGTH).0.center(), LENGTH);
    drop(hold);
    background::wait_all(&mut a.app);
    assert_eq!(writes.get(), 0, "unwritten art formats cannot replace the new readout text");
    assert_eq!(a.snapshot(), before);
    let next = a.frame(vec![]);
    assert_eq!(visible_text_count(&next, COPIED), 1, "no duplicate confirmation was queued");
    assert!(copies(&next).is_empty());
}

fn dismiss_feedback(a: &mut App, out: &egui::FullOutput) {
    a.tap(painted(out, COPIED).0.center());
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
    let (value, elided) = out
        .shapes
        .iter()
        .find_map(|s| {
            let t = text_shape(&s.shape, &name)?;
            let r = t.galley.rect.translate(t.pos.to_vec2()).intersect(s.clip_rect);
            (r.is_positive() && (r.center().y - label.center().y).abs() < 8.0).then_some((r, t.galley.elided))
        })
        .expect("the actual selected image's Link Info Name value");
    assert!(elided, "this fixture really truncates the painted readout");
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

fn assert_no_delayed_copy(a: &mut App) {
    assert!(copies(&a.deadline()).is_empty());
    assert!(copies(&a.frame(vec![])).is_empty());
}

fn cancelled_gesture(kind: &str) {
    let mut a = App::new();
    let out = a.document_category("Objects");
    let at = painted(&out, LENGTH).0.center();
    let before = a.snapshot();
    match kind {
        "outside" => {
            a.frame(vec![Event::PointerMoved(at), button(at, true)]);
            let up = a.frame(vec![button(at + vec2(-80.0, 0.0), false)]);
            assert!(copies(&up).is_empty(), "release outside does not accept the value");
        }
        "double" => {
            assert!(copies(&a.tap(at)).is_empty());
            let second = a.frame_after(a.delay() * 0.25, vec![button(at, true)]);
            assert!(copies(&second).is_empty());
            assert!(copies(&a.frame(vec![button(at, false)])).is_empty());
        }
        "hidden" => {
            assert!(copies(&a.tap(at)).is_empty());
            a.close_panel("docInfo");
            assert_no_delayed_copy(&mut a);
            a.panel("docInfo");
        }
        "changed" => {
            assert!(copies(&a.tap(at)).is_empty());
            a.app.run("shape.rectangle", json!({"x": 70, "y": 70, "width": 1, "height": 1})).unwrap();
            let changed = a.frame(vec![]);
            painted(&changed, "238 pt");
        }
        _ => panic!("unknown fixture"),
    }
    assert_no_delayed_copy(&mut a);
    if kind != "changed" {
        assert_eq!(a.snapshot(), before, "cancelled gestures do not edit the document or selection");
    }
}

#[test]
fn panel_readout_double_click_release_outside_and_stale_values_do_not_auto_copy() {
    for kind in ["outside", "double", "hidden", "changed"] {
        cancelled_gesture(kind);
    }
}
