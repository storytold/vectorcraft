//! Tabs panel: tab-stop alignment, X position (Y in vertical type), leader and "align on", and a
//! ruler numbered in the General unit on which a click adds a stop, dragging moves one and dragging
//! it off the ruler deletes it. A stop placed or dragged on the ruler lands on the nearest whole
//! point. With Snap to Unit on, or Shift held, it lands on the ruler's nearest tick.

use egui::{Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind, ParaStyle, TabAlign, TabStop, TextKind};

use super::{first_selected, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

const ALIGNS: [(TabAlign, &str); 4] = [(TabAlign::Left, "⇥"), (TabAlign::Center, "⇹"), (TabAlign::Right, "⇤"), (TabAlign::Decimal, ".⇥")];

/// The name of alignment `a`'s button: its tool tip and what screen readers call it. A line of
/// vertical type runs from top to bottom, so in vertical type a Left stop is named Top-Justified
/// and a Right stop is named Bottom-Justified.
fn align_name(a: TabAlign, vertical: bool) -> &'static str {
    match (a, vertical) {
        (TabAlign::Left, false) => tl!("Left-Justified Tab"),
        (TabAlign::Left, true) => tl!("Top-Justified Tab"),
        (TabAlign::Center, _) => tl!("Center-Justified Tab"),
        (TabAlign::Right, false) => tl!("Right-Justified Tab"),
        (TabAlign::Right, true) => tl!("Bottom-Justified Tab"),
        (TabAlign::Decimal, _) => tl!("Decimal-Justified Tab"),
    }
}

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
    current_para(app).map(|(n, para, span)| (n, para.tabs, span))
}

/// As [`current`], with all the attributes of the paragraph whose stops the panel shows.
fn current_para(app: &VectorcraftApp) -> Option<(Node, ParaStyle, f64)> {
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
    let para = t.para_at(para).clone();
    Some((n, para, span.max(72.0)))
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
    let height = crate::panel_docking::layout_of(&app.ui).floating.iter().find(|w| w.contains(&"tabs".to_string())).map_or(240.0, |w| w.rect[3]);
    let pos = egui::pos2(on_screen.left() - RULER_LEFT, on_screen.top() - height - 6.0);
    let _ = ctx;
    crate::panel_docking::place_floating(app, "tabs", pos, width).ok()?;
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

/// Run `text.tabs.set` with `stops` on the text the panel acts on. Returns false when it fails.
fn apply(app: &mut VectorcraftApp, stops: &[TabStop]) -> bool {
    if let Some((id, a, b)) = super::character::text_editing(app) {
        super::character::end_typing(app);
        app.run("text.tabs.set", json!({"stops": stops_json(stops), "ids": [id.0], "start": a, "end": b})).is_ok()
    } else {
        app.run("text.tabs.set", json!({"stops": stops_json(stops)})).is_ok()
    }
}

/// Move stop `i` to `position`, sort the stops by position and set them, then select the first
/// stop within 1e-6 pt of `position` on the ruler, the stop `text.tabs.set` keeps of those it
/// merges. Returns its index, or `None` when the stops are not set and `stops` and the selection
/// stay as they were.
fn apply_moved(app: &mut VectorcraftApp, ctx: &egui::Context, stops: &mut Vec<TabStop>, i: usize, position: f64) -> Option<usize> {
    let mut moved = stops.clone();
    moved.get_mut(i)?.position = position;
    moved.sort_by(|a, b| a.position.total_cmp(&b.position));
    let sel = moved.iter().position(|s| (s.position - position).abs() < 1e-6).unwrap_or(0);
    apply(app, &moved).then(|| {
        *stops = moved;
        set_pstate(ctx, "tabs-sel", sel);
        sel
    })
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
    let vertical = matches!(&node.kind, NodeKind::Text(text) if text.vertical);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (i, (a, label)) in ALIGNS.iter().enumerate() {
            let on = stops.get(sel).map_or(align_idx == i, |s| s.align == *a);
            let tip = align_name(*a, vertical);
            let resp = ui.add(egui::Button::new(egui::RichText::new(*label).size(13.0)).selected(on).min_size(vec2(28.0, 24.0)));
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, tip));
            if resp.on_hover_text(tip).clicked() {
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
        ui.label(egui::RichText::new(if vertical { "Y:" } else { "X:" }).color(t.text));
        let x = stops.get(sel).map(|s| s.position);
        if let Some(v) = widgets::spin_field(ui, "tabs-x", x, app.session.general_unit(), 90.0, 1.0, 0.0, &[])
            && let Some(i) = apply_moved(app, ui.ctx(), &mut stops, sel, v.max(0.0))
        {
            sel = i;
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Leader:")).color(t.text));
        // The fields keep what is typed until Enter or a click away commits it (#924).
        let leader = stops.get(sel).map(|s| s.leader.clone()).unwrap_or_default();
        let typed = ui
            .add_enabled_ui(stops.get(sel).is_some(), |ui| widgets::exact_text_field_named(ui, "tabs-leader", Some(tl!("Leader:")), &leader, 90.0))
            .inner;
        if let (Some(leader), Some(s)) = (typed, stops.get_mut(sel)) {
            // Up to eight characters repeat in the leader.
            s.leader = leader.chars().take(8).collect();
            apply(app, &stops);
        }
        ui.end_row();
        ui.label(egui::RichText::new(tl!("Align On:")).color(t.text));
        let decimal = stops.get(sel).is_some_and(|s| s.align == TabAlign::Decimal);
        let on = stops.get(sel).map(|s| s.align_on.to_string()).unwrap_or_else(|| ".".into());
        let typed = ui.add_enabled_ui(decimal, |ui| widgets::exact_text_field_named(ui, "tabs-align-on", Some(tl!("Align On:")), &on, 30.0)).inner;
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
    // The ruler is numbered in the General unit, with the canvas rulers' label step at its scale
    // and a tick every tenth of a step.
    let unit = app.session.general_unit();
    let step = unit.ruler_step(scale);
    let minor = step / 10.0;
    for k in 0..=(unit.from_pt(span) / minor + 1e-6).floor() as usize {
        let v = k as f64 * minor;
        let x = to_x(unit.to_pt(v));
        let len = match k % 10 {
            0 => 10.0,
            5 => 7.0,
            _ => 4.0,
        };
        p.line_segment([egui::pos2(x, rect.bottom() - len), egui::pos2(x, rect.bottom())], Stroke::new(1.0, t.text_dim));
        if k % 10 == 0 {
            p.text(
                egui::pos2(x + 2.0, rect.top() + 1.0),
                egui::Align2::LEFT_TOP,
                crate::canvas::ruler_label(v, step),
                egui::FontId::proportional(9.0),
                t.text_dim,
            );
        }
    }
    // A stop placed or dragged on the ruler lands on the nearest whole point. With Snap to Unit on,
    // or Shift held, it lands on the ruler's nearest tick, which can be a fraction of a point.
    let snap = pstate::<bool>(ui.ctx(), "tabs-snap") || ui.input(|i| i.modifiers.shift);
    let place = |x: f32| if snap { unit.snap_to_ruler_tick(to_pt(x), scale).max(0.0) } else { to_pt(x).round() };
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
    // The stop within 5 px of `x`.
    let hit_at = |x: f32| stops.iter().position(|s| (to_x(s.position) - x).abs() <= 5.0);
    // egui reports the drag only once the pointer is a few pixels from the press, past the hit
    // tolerance, so hit-test where the button went down (#1458).
    if resp.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let hit = hit_at(pos.x);
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
                apply(app, &stops);
            } else {
                apply_moved(app, ui.ctx(), &mut stops, i, place(pos.x));
            }
        }
    } else if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        match hit_at(pos.x) {
            Some(i) => set_pstate(ui.ctx(), "tabs-sel", i),
            None => {
                let position = place(pos.x);
                // A click that lands on a stop's whole point or tick selects that stop and records no
                // undo step.
                let added = !stops.iter().any(|s| s.position == position);
                if added {
                    stops.push(TabStop { position, align: ALIGNS[align_idx.min(3)].0, leader: String::new(), align_on: '.' });
                    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
                }
                let i = stops.iter().position(|s| s.position == position).unwrap_or(0);
                set_pstate(ui.ctx(), "tabs-sel", i);
                if added {
                    apply(app, &stops);
                }
            }
        }
    }
    resp.on_hover_text(tl!("Click to add a tab stop, drag to move it, drag it off the ruler to delete it"));
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((_, para, _)) = current_para(app) else {
        ui.add_enabled(false, egui::Button::new(tl!("Select text")).frame(false));
        snap_to_unit_item(ui);
        return;
    };
    let mut stops = para.tabs;
    // Clamp the selection to the last stop, as `show` does for the ruler.
    let mut sel: usize = pstate(ui.ctx(), "tabs-sel");
    if sel >= stops.len() {
        sel = stops.len().saturating_sub(1);
    }
    if menu_item(ui, tl!("Clear All Tabs"), !stops.is_empty(), false)
        && let Err(e) = super::paragraph::run_on_paragraphs(app, "text.tabs.clear", json!({}))
    {
        app.status(e);
    }
    if menu_item(ui, tl!("Delete Tab"), sel < stops.len(), false) {
        stops.remove(sel);
        apply(app, &stops);
    }
    // Repeat Tab: copies of the selected stop across the ruler, spaced by its distance from the
    // previous stop, or from the paragraph's left indent when it is the first stop.
    if menu_item(ui, tl!("Repeat Tab"), sel < stops.len(), false) {
        let step = match sel {
            0 => (stops[0].position - para.left_indent).abs(),
            _ => stops[sel].position - stops[sel - 1].position,
        }
        .max(1.0);
        let base = stops[sel].clone();
        stops.truncate(sel + 1);
        let mut pos = base.position + step;
        while pos <= base.position + step * 20.0 && pos < 2000.0 {
            stops.push(TabStop { position: pos, ..base.clone() });
            pos += step;
        }
        apply(app, &stops);
    }
    snap_to_unit_item(ui);
}

/// The panel menu's Snap to Unit, checked while it is on: stops placed or dragged on the ruler land
/// on its nearest tick.
fn snap_to_unit_item(ui: &mut Ui) {
    ui.separator();
    let on: bool = pstate(ui.ctx(), "tabs-snap");
    if menu_item(ui, tl!("Snap to Unit"), true, on) {
        set_pstate(ui.ctx(), "tabs-snap", !on);
    }
}

#[cfg(test)]
mod tests;
