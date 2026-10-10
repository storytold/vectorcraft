//! Tabs panel: tab-stop alignment, X position, leader and "align on", and a ruler on which a click
//! adds a stop, dragging moves one and dragging it off the ruler deletes it.

use egui::{Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind, TabAlign, TabStop, TextKind};

use super::{first_selected, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

const ALIGNS: [(TabAlign, &str, &str); 4] = [
    (TabAlign::Left, "⇥", "Left-Justified Tab"),
    (TabAlign::Center, "⇹", "Center-Justified Tab"),
    (TabAlign::Right, "⇤", "Right-Justified Tab"),
    (TabAlign::Decimal, ".⇥", "Decimal-Justified Tab"),
];

fn align_id(a: TabAlign) -> &'static str {
    match a {
        TabAlign::Left => "left",
        TabAlign::Center => "center",
        TabAlign::Right => "right",
        TabAlign::Decimal => "decimal",
    }
}

/// The panel body's margin and the ruler's own inset: the ruler's 0 is this far right of the panel.
const RULER_LEFT: f32 = 10.0 + 4.0;

/// The text object, the stops of the caret's paragraph while the Type tool edits text (else the
/// first selected text object's first paragraph) and the ruler span in points (the frame width for
/// area type).
fn current(app: &VectorcraftApp) -> Option<(Node, Vec<TabStop>, f64)> {
    let editing = super::character::text_editing(app);
    let n = match editing {
        Some((id, _, _)) => app.session.active()?.doc.node(id)?.clone(),
        None => first_selected(app)?,
    };
    let NodeKind::Text(t) = &n.kind else { return None };
    let para = editing.map_or(0, |(_, a, _)| t.paragraphs_in(a, a).start);
    let span = match &t.kind {
        TextKind::Area { frame } => frame.bounds().map_or(360.0, |b| b.width()),
        _ => 360.0,
    };
    let tabs = t.para_at(para).tabs.clone();
    Some((n, tabs, span.max(72.0)))
}

/// Position Panel Above Text: float the panel just above text `n`, as wide as its ruler needs to
/// put 0 on the text's left edge and its marks at the canvas zoom, so each stop sits over the
/// column it sets (#729). `span`: the ruler's length in the text's points.
fn above_text(app: &mut VectorcraftApp, ctx: &egui::Context, n: &Node, span: f64) -> Option<()> {
    let xf = crate::canvas::Xf::new(app.canvas_rect?, app.view()?);
    let on_screen = xf.rect_to_screen(n.geometric_bounds()?);
    // Screen points per point of the text: across the frame for area type (so a scaled frame
    // lines up too), the zoom for point type.
    let per = match &n.kind {
        NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. }) => on_screen.width() / span as f32,
        _ => xf.zoom as f32,
    };
    if !per.is_finite() || per <= 0.0 {
        return None;
    }
    let width = span as f32 * per + RULER_LEFT * 2.0;
    let height = ctx.memory(|m| m.area_rect(crate::floating::area_id("tabs"))).map_or(240.0, |r| r.height());
    let pos = egui::pos2(on_screen.left() - RULER_LEFT, on_screen.top() - height - 6.0);
    if crate::floating::group_of(&app.ui, "tabs").is_none() {
        crate::floating::float(&mut app.ui, &["tabs"], "tabs", pos);
    }
    let gi = crate::floating::group_of(&app.ui, "tabs")?;
    let g = app.ui.floating_panels.get_mut(gi)?;
    g.pos = [pos.x, pos.y];
    g.width = Some(width);
    Some(())
}

fn stops_json(stops: &[TabStop]) -> Value {
    Value::Array(
        stops
            .iter()
            .map(|s| json!({"position": s.position, "align": align_id(s.align), "leader": s.leader, "alignOn": s.align_on.to_string()}))
            .collect(),
    )
}

fn apply(app: &mut VectorcraftApp, stops: &[TabStop]) {
    if let Some((id, a, b)) = super::character::text_editing(app) {
        super::character::end_typing(app);
        app.run("text.tabs.set", json!({"stops": stops_json(stops), "ids": [id.0], "start": a, "end": b})).ok();
    } else {
        app.run("text.tabs.set", json!({"stops": stops_json(stops)})).ok();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((node, mut stops, span)) = current(app) else {
        super::empty_state(ui, "pilcrow", tl!("No text selected"), tl!("Select a text object to set its tab stops."));
        return;
    };
    let mut sel: usize = pstate(ui.ctx(), "tabs-sel");
    if sel >= stops.len() {
        sel = stops.len().saturating_sub(1);
    }
    let mut align_idx: usize = pstate(ui.ctx(), "tabs-align");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (i, (a, label, tip)) in ALIGNS.iter().enumerate() {
            let on = stops.get(sel).map_or(align_idx == i, |s| s.align == *a);
            if ui
                .add(egui::Button::new(egui::RichText::new(*label).size(13.0)).selected(on).min_size(vec2(28.0, 24.0)))
                .on_hover_text(tl!(*tip))
                .clicked()
            {
                align_idx = i;
                set_pstate(ui.ctx(), "tabs-align", i);
                if let Some(s) = stops.get_mut(sel) {
                    s.align = *a;
                    apply(app, &stops);
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Above Text")).on_hover_text(tl!("Position Panel Above Text")).clicked() {
                above_text(app, ui.ctx(), &node, span);
            }
        });
    });
    ui.add_space(4.0);
    egui::Grid::new("tabs-grid").num_columns(2).spacing([6.0, 4.0]).show(ui, |ui| {
        ui.label(egui::RichText::new("X:").color(t.text));
        let x = stops.get(sel).map(|s| s.position);
        if let Some(v) = widgets::spin_field(ui, "tabs-x", x, app.session.general_unit(), 90.0, 1.0, 0.0, &[])
            && let Some(s) = stops.get_mut(sel)
        {
            s.position = v.max(0.0);
            stops.sort_by(|a, b| a.position.total_cmp(&b.position));
            apply(app, &stops);
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Leader:")).color(t.text));
        // The fields keep what is typed until Enter or a click away commits it (#924).
        let leader = stops.get(sel).map(|s| s.leader.clone()).unwrap_or_default();
        let typed = ui.add_enabled_ui(stops.get(sel).is_some(), |ui| widgets::exact_text_field(ui, "tabs-leader", &leader, 90.0)).inner;
        if let (Some(leader), Some(s)) = (typed, stops.get_mut(sel)) {
            // Up to eight characters repeat in the leader.
            s.leader = leader.chars().take(8).collect();
            apply(app, &stops);
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Align On:")).color(t.text));
        let decimal = stops.get(sel).is_some_and(|s| s.align == TabAlign::Decimal);
        let on = stops.get(sel).map(|s| s.align_on.to_string()).unwrap_or_else(|| ".".into());
        let typed = ui.add_enabled_ui(decimal, |ui| widgets::exact_text_field(ui, "tabs-align-on", &on, 30.0)).inner;
        if let (Some(c), Some(s)) = (typed.and_then(|t| t.chars().next()), stops.get_mut(sel)) {
            s.align_on = c;
            apply(app, &stops);
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    // The ruler.
    let w = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click_and_drag());
    let p = ui.painter_at(rect.expand(2.0));
    p.rect_filled(rect, 2.0, t.input);
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let scale = (w as f64 - 8.0) / span;
    let to_x = |pt: f64| rect.left() + 4.0 + (pt * scale) as f32;
    let to_pt = |x: f32| (((x - rect.left() - 4.0) as f64) / scale).max(0.0);
    let mut tick = 0.0;
    while tick <= span + 1e-6 {
        let x = to_x(tick);
        let major = (tick / 36.0).round() * 36.0 == tick && ((tick / 36.0) as i64) % 2 == 0;
        p.line_segment([egui::pos2(x, rect.bottom() - if major { 10.0 } else { 5.0 }), egui::pos2(x, rect.bottom())], Stroke::new(1.0, t.text_dim));
        if major {
            p.text(
                egui::pos2(x + 2.0, rect.top() + 1.0),
                egui::Align2::LEFT_TOP,
                format!("{}", (tick / 72.0 * 100.0).round() / 100.0),
                egui::FontId::proportional(9.0),
                t.text_dim,
            );
        }
        tick += 18.0;
    }
    for (i, s) in stops.iter().enumerate() {
        let x = to_x(s.position);
        let c = if i == sel { t.accent } else { t.text };
        let y = rect.bottom() - 2.0;
        // An original marker: a stem with a foot showing which way the text runs.
        p.line_segment([egui::pos2(x, y - 12.0), egui::pos2(x, y)], Stroke::new(1.5, c));
        let foot = match s.align {
            TabAlign::Left => [egui::pos2(x, y), egui::pos2(x + 6.0, y)],
            TabAlign::Right => [egui::pos2(x - 6.0, y), egui::pos2(x, y)],
            TabAlign::Center | TabAlign::Decimal => [egui::pos2(x - 4.0, y), egui::pos2(x + 4.0, y)],
        };
        p.line_segment(foot, Stroke::new(1.5, c));
        if s.align == TabAlign::Decimal {
            p.circle_filled(egui::pos2(x + 3.0, y - 6.0), 1.5, c);
        }
    }
    let drag: Option<usize> = pstate(ui.ctx(), "tabs-drag");
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let hit = stops.iter().position(|s| (to_x(s.position) - pos.x).abs() <= 5.0);
        set_pstate(ui.ctx(), "tabs-drag", hit);
        if let Some(i) = hit {
            set_pstate(ui.ctx(), "tabs-sel", i);
        }
    }
    if resp.drag_stopped()
        && let (Some(i), Some(pos)) = (drag, resp.interact_pointer_pos())
    {
        set_pstate::<Option<usize>>(ui.ctx(), "tabs-drag", None);
        if i < stops.len() {
            if pos.y > rect.bottom() + 20.0 {
                // Dragged off the ruler: delete.
                stops.remove(i);
            } else {
                stops[i].position = to_pt(pos.x);
                stops.sort_by(|a, b| a.position.total_cmp(&b.position));
            }
            apply(app, &stops);
        }
    } else if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        match stops.iter().position(|s| (to_x(s.position) - pos.x).abs() <= 5.0) {
            Some(i) => set_pstate(ui.ctx(), "tabs-sel", i),
            None => {
                let position = to_pt(pos.x);
                stops.push(TabStop { position, align: ALIGNS[align_idx.min(3)].0, leader: String::new(), align_on: '.' });
                stops.sort_by(|a, b| a.position.total_cmp(&b.position));
                let i = stops.iter().position(|s| s.position == position).unwrap_or(0);
                set_pstate(ui.ctx(), "tabs-sel", i);
                apply(app, &stops);
            }
        }
    }
    resp.on_hover_text(tl!("Click to add a tab stop, drag to move it, drag it off the ruler to delete it"));
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((_, mut stops, _)) = current(app) else {
        ui.add_enabled(false, egui::Button::new(tl!("Select text")).frame(false));
        return;
    };
    let sel: usize = pstate(ui.ctx(), "tabs-sel");
    if menu_item(ui, tl!("Clear All Tabs"), false, !stops.is_empty()) {
        apply(app, &[]);
    }
    if menu_item(ui, tl!("Delete Tab"), false, sel < stops.len()) {
        stops.remove(sel);
        apply(app, &stops);
    }
    // Repeat Tab: copies of the selected stop at its distance from the previous one, across the ruler.
    if menu_item(ui, tl!("Repeat Tab"), false, sel < stops.len()) {
        let prev = if sel == 0 { 0.0 } else { stops[sel - 1].position };
        let step = (stops[sel].position - prev).max(1.0);
        let base = stops[sel].clone();
        stops.truncate(sel + 1);
        let mut pos = base.position + step;
        while pos <= base.position + step * 20.0 && pos < 2000.0 {
            stops.push(TabStop { position: pos, ..base.clone() });
            pos += step;
        }
        apply(app, &stops);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(app, ui);
            menu(app, ui);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn panel_draws_empty_with_text_and_with_stops() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({})).unwrap();
        frame(&mut app);
        app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a\tb", "area": {"width": 300, "height": 100}})).unwrap();
        frame(&mut app);
        app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50}, {"position": 120, "align": "decimal"}]})).unwrap();
        frame(&mut app);
        let (_, stops, span) = current(&app).unwrap();
        assert_eq!(stops.len(), 2);
        assert_eq!(span, 300.0);
        // The stops survive a JSON round trip through the panel's encoding.
        let back: Vec<serde_json::Value> = stops_json(&stops).as_array().unwrap().clone();
        assert_eq!(back[1]["align"], json!("decimal"));
    }

    /// #924: Leader and Align On keep what is typed, frame after frame, until Enter applies it
    /// (spaces kept, eight characters at most).
    #[test]
    fn leader_and_align_on_keep_what_is_typed_until_enter() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 20, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
        app.session.execute("text.tabs.set", &json!({"stops": [{"position": 50, "align": "decimal"}]})).unwrap();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 400.0));
        // One frame with `events` → the texts painted and where.
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.visual_bounding_rect())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        let click = |at| {
            [true, false].map(|pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            })
        };
        let stop = |app: &VectorcraftApp| current(app).unwrap().1[0].clone();
        for (label, typed, want) in [("Leader:", [". ", "_", "123456789"], ". _12345"), ("Align On:", [",", "", ""], ",")] {
            let texts = frame(&mut app, vec![]);
            let r = texts.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("{label} in {texts:?}")).1;
            let at = egui::pos2(r.right() + 30.0, r.center().y);
            let [down, up] = click(at);
            frame(&mut app, vec![egui::Event::PointerMoved(at), down]);
            frame(&mut app, vec![up]);
            // Select what is there, then type in pieces over several frames.
            frame(
                &mut app,
                vec![egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND }],
            );
            for piece in typed.iter().filter(|p| !p.is_empty()) {
                frame(&mut app, vec![egui::Event::Text(piece.to_string())]);
                frame(&mut app, vec![]);
            }
            frame(&mut app, vec![key(egui::Key::Enter)]);
            frame(&mut app, vec![]);
            let s = stop(&app);
            let got = if label == "Leader:" { s.leader } else { s.align_on.to_string() };
            assert_eq!(got, want, "{label}");
        }
    }

    /// Position Panel Above Text floats the panel over the text, its ruler's 0 on the frame's left
    /// edge and a stop's mark over the column it sets, at any zoom (#729).
    #[test]
    fn above_text_puts_the_ruler_over_the_text_at_the_zoom() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        app.session.execute("text.create", &json!({"x": 40, "y": 200, "text": "a	b", "area": {"width": 300, "height": 100}})).unwrap();
        app.canvas_rect = Some(egui::Rect::from_min_size(egui::pos2(50.0, 40.0), vec2(1000.0, 700.0)));
        for zoom in [1.0, 2.0] {
            if let Some(v) = app.view_mut() {
                v.zoom = zoom;
            }
            let (node, _, span) = current(&app).unwrap();
            let ctx = egui::Context::default();
            above_text(&mut app, &ctx, &node, span).unwrap();
            let g = &app.ui.floating_panels[crate::floating::group_of(&app.ui, "tabs").unwrap()];
            let xf = crate::canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
            let frame = xf.rect_to_screen(node.geometric_bounds().unwrap());
            assert!((g.pos[0] + RULER_LEFT - frame.left()).abs() < 0.01, "0 on the frame's left edge");
            assert!(g.pos[1] < frame.top(), "above the text");
            // The ruler (the panel less its margins) spans the frame at this zoom.
            let ruler = g.width.unwrap() - RULER_LEFT * 2.0;
            assert!((ruler - frame.width()).abs() < 0.01, "{ruler} {}", frame.width());
            assert!((ruler - 300.0 * zoom as f32).abs() < 0.01);
        }
        assert_eq!(app.ui.floating_panels.len(), 1, "placed again, not floated twice");
    }
}
