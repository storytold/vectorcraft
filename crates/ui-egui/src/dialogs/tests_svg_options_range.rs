//! Persistent-context pointer coverage of SVG artboard range validation and recovery.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Pos2, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{Services, VectorcraftApp};

use super::tests_export::Written;

struct ExportHarness {
    app: VectorcraftApp,
    ctx: egui::Context,
    written: Written,
    time: f64,
}

impl ExportHarness {
    fn new() -> Self {
        let written = Rc::new(RefCell::new(vec![]));
        let w = written.clone();
        let services = Services {
            pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/out/{}", p.name)))),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 80, "height": 60, "artboards": 80, "columns": 10, "spacing": 20})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        for (i, name, colour, width, height) in
            [(0, "FirstArt", "#cc2200", 11, 13), (78, "OtherArt", "#2244cc", 9, 7), (79, "LastArt", "#00aa88", 17, 19)]
        {
            let r = app.session.active().unwrap().doc.artboards[i].rect;
            app.run("select.none", json!({})).unwrap();
            app.run("paint.setFill", json!({"color": colour})).unwrap();
            app.run("shape.rectangle", json!({"x": r.x0 + 7.0, "y": r.y0 + 11.0, "width": width, "height": height})).unwrap();
            app.run("object.setProps", json!({"name": name})).unwrap();
        }
        app.run("select.none", json!({})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { app, ctx, written, time: 0.0 }
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.1;
        let input = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 900.0))),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| super::show(&mut self.app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    fn settle(&mut self) -> egui::FullOutput {
        self.frame(vec![]);
        self.frame(vec![])
    }

    fn click(&mut self, at: Pos2) -> egui::FullOutput {
        let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle()
    }

    fn choose_range(&mut self, format: &str, range: &str) {
        self.app.run("file.exportAs", json!({"format": format})).unwrap();
        let out = self.settle();
        let out = self.click(text_at(&out, "Use Artboards"));
        let out = self.click(text_at(&out, "Range:"));
        assert!(!self.app.ui.dialog.as_ref().unwrap().bool("all"));
        self.click(text_at(&out, "1-80"));
        self.frame(vec![Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
        let out = self.frame(vec![Event::Text(range.into())]);
        assert_eq!(self.app.ui.dialog.as_ref().unwrap().str("range"), range);
        assert!(self.written.borrow().is_empty());
        self.click(text_at(&out, "Export…"));
        let d = self.app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("useArtboards"));
        assert_eq!(d.str("range"), range);
    }

    fn finish(&mut self, button: &str) {
        let out = self.settle();
        self.click(text_at(&out, button));
        assert!(self.app.ui.dialog.is_none(), "{}", self.app.ui.status);
    }
}

fn text_at(out: &egui::FullOutput, label: &str) -> Pos2 {
    fn find(shape: &Shape, label: &str) -> Option<Pos2> {
        match shape {
            Shape::Text(t) if t.galley.text().trim() == label => Some(t.pos + vec2(4.0, 4.0)),
            Shape::Vec(v) => v.iter().find_map(|s| find(s, label)),
            _ => None,
        }
    }
    out.shapes.iter().find_map(|s| find(&s.shape, label)).unwrap_or_else(|| panic!("missing {label}: {}", crate::tests_labels::shapes_text(out)))
}

#[test]
fn invalid_range_preserves_export_dialog_and_writes_nothing() {
    let mut h = ExportHarness::new();
    h.app.run("file.exportAs", json!({"format": "svg", "useArtboards": true, "range": "81"})).unwrap();
    let out = h.settle();
    h.click(text_at(&out, "Export…"));
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().kind, "exportAs");
    assert!(h.written.borrow().is_empty());
    assert!(h.app.ui.status.contains("81"));
}

#[test]
fn a_bad_range_in_svg_options_keeps_the_modal_and_saved_options() {
    let mut h = ExportHarness::new();
    h.choose_range("svg", "80");
    let saved = h.app.ui.svg_options.clone();
    let out = h.settle();
    h.click(text_at(&out, "80"));
    h.frame(vec![Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
    let out = h.frame(vec![Event::Text("81".into())]);
    assert_eq!(h.app.ui.dialog.as_ref().unwrap().str("range"), "81");
    h.click(text_at(&out, "OK"));
    assert_eq!(h.app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("svgOptions"));
    assert_eq!(h.app.ui.svg_options, saved);
    assert!(h.written.borrow().is_empty());
    assert!(h.app.ui.status.contains("81"));
    let out = h.settle();
    h.click(text_at(&out, "81"));
    h.frame(vec![Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
    h.frame(vec![Event::Text("80".into())]);
    h.finish("OK");
    assert_eq!(h.written.borrow().len(), 1);
    let written = h.written.borrow();
    let svg = std::str::from_utf8(&written[0].1).unwrap();
    assert!(svg.contains("viewBox=\"0 0 80 60\"") && svg.contains("LastArt"));
}
