//! Escape cancels an existing ruler-guide gesture through the window's actual input path.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::{Document, Selection};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{VectorcraftApp, canvas::Xf};

const SELECTION_TOOLS: [&str; 3] = ["selection", "directSelection", "groupSelection"];

struct GuideDrag {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    vertical: bool,
    copy: bool,
    tool: &'static str,
    borrowed: bool,
    mods: Modifiers,
    from: Pos2,
    to: Pos2,
    end: Pos2,
    before: Arc<Document>,
    selection: Selection,
    redo: Arc<Document>,
    undo: usize,
    tool_options: serde_json::Value,
}

impl GuideDrag {
    fn new(tool: &'static str, vertical: bool, copy: bool, borrowed: bool) -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":400,"height":300})).unwrap();
        app.run("view.guides.lock", json!({"locked":false})).unwrap();
        app.run("shape.rectangle", json!({"x":20,"y":20,"width":20,"height":20})).unwrap();
        app.run("guide.add", json!({"vertical":vertical,"pos":100,"artboard":0})).unwrap();
        app.run("guide.add", json!({"vertical":!vertical,"pos":220,"artboard":0})).unwrap();
        app.run("guide.select", json!({"indexes":[0,1]})).unwrap();
        app.run("shape.rectangle", json!({"x":300,"y":260,"width":10,"height":10})).unwrap();
        let redo = app.session.active().unwrap().doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        // Switch away first so even the initial Selection tool becomes the last chosen one.
        // Cmd borrows that tool and returns the Pen on pointer Up.
        app.select_tool("pen");
        app.select_tool(tool);
        if borrowed {
            app.select_tool("pen");
        }
        let tool_options = app.session.tool_options();
        app.ui.view.rulers = true;
        app.ui.view.guides = true;
        app.ui.view.smart_guides = false;
        app.ui.view.snap_to_point = false;
        app.ui.view.snap_to_grid = false;
        app.ui.view.snap_to_pixel = false;
        let st = app.session.active().unwrap();
        let (before, selection, undo) = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
        let mut f = Self {
            app,
            ctx: egui::Context::default(),
            time: 0.0,
            vertical,
            copy,
            tool,
            borrowed,
            mods: Modifiers { command: borrowed, alt: copy, ..Default::default() },
            from: Pos2::ZERO,
            to: Pos2::ZERO,
            end: Pos2::ZERO,
            before,
            selection,
            redo,
            undo,
            tool_options,
        };
        for _ in 0..3 {
            f.frame(vec![]);
        }
        let view = f.app.view_mut().unwrap();
        view.center = Point::new(200.0, 150.0);
        view.zoom = 1.0;
        view.rotation = 0.0;
        f.frame(vec![]);
        let rect = f.app.canvas_rect.unwrap();
        let xf = Xf::new(rect, f.app.view().unwrap());
        let point = |at| if vertical { Point::new(at, 60.0) } else { Point::new(60.0, at) };
        (f.from, f.to, f.end) = (xf.to_screen(point(100.0)), xf.to_screen(point(140.0)), xf.to_screen(point(160.0)));
        assert!([f.from, f.to, f.end].into_iter().all(|p| rect.contains(p)));
        assert_eq!(f.selection.guides, vec![0, 1]);
        f.app.session.active_mut().unwrap().mark_saved();
        f.unchanged();
        f
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let raw = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0))),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        assert!(!app.ui.status.starts_with("Internal error:"), "{}", app.ui.status);
    }

    fn button(&self, pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: self.mods }
    }

    fn key(&mut self, key: Key) {
        let events = [true, false].map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: self.mods }).to_vec();
        self.frame(events);
    }

    fn press(&mut self) {
        self.frame(vec![Event::ModifiersChanged(self.mods), Event::PointerMoved(self.from)]);
        self.frame(vec![self.button(self.from, true)]);
        assert!(self.ctx.input(|i| i.pointer.primary_down()));
        assert!(self.app.session.tool_busy());
        assert_eq!(self.app.session.tool_id(), self.tool);
    }

    fn start(&mut self) {
        self.press();
        self.frame(vec![Event::PointerMoved(self.to)]);
        self.guides_at_target();
        let st = self.app.session.active().unwrap();
        assert_eq!(st.interaction.as_ref().unwrap().preview.as_ref().unwrap().0, "guide.move");
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (self.undo, 1));
    }

    fn guides_at_target(&self) {
        let mut moved = self.before.guides.clone();
        for guide in &mut moved {
            if guide.vertical == self.vertical {
                guide.pos += 40.0;
            }
        }
        let expected = if self.copy { self.before.guides.iter().cloned().chain(moved).collect::<Vec<_>>() } else { moved };
        let st = self.app.session.active().unwrap();
        assert_eq!(st.doc.guides, expected);
        assert_eq!(st.doc.layers, self.before.layers);
        assert_eq!(st.selection.guides, if self.copy { vec![2, 3] } else { vec![0, 1] });
    }

    fn unchanged(&self) {
        let st = self.app.session.active().unwrap();
        assert_eq!(st.doc, self.before);
        assert_eq!(st.selection, self.selection);
        assert!(!self.app.session.in_interaction() && !self.app.session.tool_busy());
        assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (self.undo, 1, false));
    }

    fn finish_cancel(&mut self) {
        assert!(self.ctx.input(|i| i.pointer.primary_down()), "Escape does not release the pointer");
        assert_eq!(self.app.session.tool_id(), self.tool, "the borrowed tool stays until Up");
        self.unchanged();
        // Further motion and modifier changes while still held cannot restart the canceled drag.
        self.mods = Modifiers::NONE;
        self.frame(vec![Event::ModifiersChanged(self.mods), Event::PointerMoved(self.end)]);
        self.unchanged();
        self.frame(vec![self.button(self.end, false)]);
        self.unchanged();
        assert!(!self.ctx.input(|i| i.pointer.primary_down()));
        assert_eq!(self.app.session.tool_id(), if self.borrowed { "pen" } else { self.tool });
        assert_eq!(self.app.session.tool_options(), self.tool_options);
        self.app.run("edit.redo", json!({})).unwrap();
        assert_eq!(self.app.session.active().unwrap().doc, self.redo, "Escape preserves the original Redo branch");
    }

    fn finish_commit(&mut self) {
        self.frame(vec![self.button(self.to, false)]);
        self.guides_at_target();
        assert_eq!(self.app.session.tool_id(), if self.borrowed { "pen" } else { self.tool });
        assert_eq!(self.app.session.tool_options(), self.tool_options);
        let st = self.app.session.active().unwrap();
        assert!(!self.app.session.in_interaction() && !self.app.session.tool_busy());
        assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (self.undo + 1, 0, true));
        assert_eq!(st.history.undo.last().unwrap().label, if self.copy { "Copy Guide" } else { "Move Guide" });
        let (after, selected) = (st.doc.clone(), st.selection.clone());
        self.app.run("edit.undo", json!({})).unwrap();
        self.unchanged();
        self.app.run("edit.redo", json!({})).unwrap();
        let st = self.app.session.active().unwrap();
        assert_eq!(st.doc, after);
        assert_eq!(st.selection, selected);
    }
}

fn cancel_cases(tool: &'static str, borrowed: bool) {
    for vertical in [false, true] {
        for copy in [false, true] {
            let mut f = GuideDrag::new(tool, vertical, copy, borrowed);
            f.start();
            f.key(Key::Escape);
            assert!(
                !f.app.session.in_interaction(),
                "Escape must cancel the existing-guide interaction: tool={tool} borrowed={borrowed} vertical={vertical} copy={copy}"
            );
            f.finish_cancel();
        }
    }
}

#[test]
fn escape_cancels_existing_guide_selection() {
    cancel_cases("selection", false);
}

#[test]
fn escape_cancels_existing_guide_direct_selection() {
    cancel_cases("directSelection", false);
}

#[test]
fn escape_cancels_existing_guide_group_selection() {
    cancel_cases("groupSelection", false);
}

#[test]
fn escape_cancels_existing_guide_borrowed_selection() {
    for tool in SELECTION_TOOLS {
        cancel_cases(tool, true);
    }
}

#[test]
fn escape_clears_existing_guide_shift_press() {
    for tool in SELECTION_TOOLS {
        for vertical in [false, true] {
            let mut f = GuideDrag::new(tool, vertical, false, false);
            f.mods = Modifiers::SHIFT;
            f.press();
            assert!(!f.app.session.in_interaction(), "no interaction before the drag threshold");
            f.key(Key::Escape);
            assert!(!f.app.session.tool_busy(), "Escape must clear the pressed guide before release: tool={tool}");
            f.finish_cancel();
        }
    }
}

#[test]
fn existing_guide_release_without_escape_control() {
    for tool in SELECTION_TOOLS {
        for vertical in [false, true] {
            for copy in [false, true] {
                for borrowed in [false, true] {
                    let mut f = GuideDrag::new(tool, vertical, copy, borrowed);
                    f.start();
                    f.finish_commit();
                }
            }
        }
    }
}

#[test]
fn existing_guide_enter_keeps_drag_control() {
    for tool in SELECTION_TOOLS {
        for vertical in [false, true] {
            for copy in [false, true] {
                let mut f = GuideDrag::new(tool, vertical, copy, false);
                f.start();
                f.key(Key::Enter);
                assert!(f.app.session.in_interaction() && f.app.session.tool_busy());
                f.guides_at_target();
                f.finish_commit();
            }
        }
    }
}

#[test]
fn existing_guide_idle_escape_control() {
    for tool in SELECTION_TOOLS {
        let mut f = GuideDrag::new(tool, true, false, false);
        f.key(Key::Escape);
        f.unchanged();
        assert_eq!(f.app.session.tool_id(), tool);
        f.app.run("edit.redo", json!({})).unwrap();
        assert_eq!(f.app.session.active().unwrap().doc, f.redo);
    }
}
