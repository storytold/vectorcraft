//! Character alternates on the canvas: with a Type tool and exactly one character selected, a
//! small row of that character's alternate glyphs shows beside it. A click on a glyph draws the
//! character with it, as a double-click on the glyph in the Glyphs panel's Show › Alternates for
//! Current Selection does, through the same command and so as one undo step. An arrow at the end
//! of the row opens the Glyphs panel when the character has more alternates than the row shows.
//! Preferences › Type › Show Character Alternates (`showCharacterAlternates`) turns the row off.
//!
//! Public behavior: selecting a character shows its alternate glyphs next to it, a click replaces
//! the character's glyph, at most five alternates show, and a control to their right opens the
//! Glyphs panel for the rest ("Replace a character with on-canvas alternate glyphs",
//! <https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=479>;
//! [Replace characters with alternate glyphs](https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/replace-characters-with-alternate-glyphs.html)).
//! The row's place, its size and the preference are VectorCraft's own.

use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Point;
use vectorcraft_text::FontDb;

use super::{CornerRadius, Pos2, Sense, Shape, Stroke, StrokeKind, Tokens, Ui, Vec2, VectorcraftApp, Xf, pos2, vec2};
use crate::panels::glyphs;

/// The side of one glyph cell, in points.
const CELL: f32 = 28.0;
/// The width of the arrow that opens the Glyphs panel.
const MORE: f32 = 16.0;
/// The space between the row's edge and its cells: the frame's 1-point line and 3 points inside it.
const PAD: f32 = 4.0;
/// The space between the selected character and the row.
const GAP: f32 = 6.0;

fn bar_id() -> egui::Id {
    egui::Id::unique("character-alternates-bar")
}

/// Where the row was last drawn (none while it isn't shown).
#[cfg(test)]
pub(super) fn bar_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = bar_id();
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// The size of a row of `count` glyphs, with the arrow when `more`.
fn bar_size(count: usize, more: bool) -> Vec2 {
    let cells = count as f32 * CELL + if more { MORE } else { 0.0 };
    vec2(cells + 2.0 * PAD, CELL + 2.0 * PAD)
}

/// The document-space highlight quads of bytes `a..b` of text `id`, and whether the text is set
/// vertically. Kept until the document or the selection changes, so the text is laid out once for
/// each, not in every frame. Empty for a character the text does not draw (overset area type).
fn character_quads(app: &VectorcraftApp, ui: &Ui, id: NodeId, (a, b): (usize, usize)) -> Option<(Vec<[Point; 4]>, bool)> {
    type Quads = (Vec<[Point; 4]>, bool);
    let st = app.session.active()?;
    let key = (st.uid, st.revision, id, a, b);
    let slot = bar_id().with("quads");
    if let Some((cached_key, quads)) = ui.ctx().data(|d| d.get_temp::<((u64, u64, NodeId, usize, usize), Quads)>(slot))
        && cached_key == key
    {
        return Some(quads);
    }
    let NodeKind::Text(tx) = &st.doc.node(id)?.kind else { return None };
    let lay = vectorcraft_text::layout(FontDb::global(), tx);
    let quads: Vec<[Point; 4]> = vectorcraft_text::selection_quads(&lay, a, b).into_iter().map(|q| q.map(|c| tx.xf * c)).collect();
    let found = (quads, tx.vertical);
    ui.ctx().data_mut(|d| d.insert_temp(slot, (key, found.clone())));
    Some(found)
}

/// The top-left corner of a row of `size` beside the selected character's screen box `sel`, inside
/// `area`. The row goes under a character of horizontal type, or over it when there is no room
/// under it; beside a character of vertical type, on its right, or on its left when there is no
/// room on the right. A side is also given up when the row would cover `avoid` there (the
/// Contextual Task Bar). With no room on either side the row takes the first one, moved into
/// `area`.
fn place(sel: egui::Rect, size: Vec2, area: egui::Rect, vertical: bool, avoid: Option<egui::Rect>) -> Pos2 {
    let sides = if vertical {
        [pos2(sel.right() + GAP, sel.center().y - size.y / 2.0), pos2(sel.left() - GAP - size.x, sel.center().y - size.y / 2.0)]
    } else {
        [pos2(sel.center().x - size.x / 2.0, sel.bottom() + GAP), pos2(sel.center().x - size.x / 2.0, sel.top() - GAP - size.y)]
    };
    let last = (area.max - size).max(area.min);
    // Along the character's line the row may slide to stay on the canvas; across it, it may not.
    let slide = |at: Pos2| if vertical { pos2(at.x, at.y.clamp(area.min.y, last.y)) } else { pos2(at.x.clamp(area.min.x, last.x), at.y) };
    let fits = |at: Pos2| {
        let r = egui::Rect::from_min_size(at, size);
        area.contains_rect(r) && avoid.is_none_or(|bar| !bar.intersects(r))
    };
    let sides = sides.map(slide);
    sides.into_iter().find(|at| fits(*at)).unwrap_or_else(|| sides[0].clamp(area.min, last))
}

/// The row of alternate glyphs beside the Type tool's one selected character.
pub(super) fn alternates_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    // Not while a press or a drag is choosing the selection: the row would follow the pointer.
    if !app.session.prefs.show_character_alternates || !app.session.tool_wants_text() || app.session.tool_busy() {
        return;
    }
    let Some(found) = glyphs::on_canvas(app, ui) else { return };
    let Some((quads, vertical)) = character_quads(app, ui, found.id, found.bytes) else { return };
    let corners: Vec<Pos2> = quads.iter().flatten().map(|p| xf.to_screen(*p)).collect();
    if corners.is_empty() {
        return;
    }
    let sel = egui::Rect::from_points(&corners);
    // A character scrolled off the canvas has no row.
    if !sel.intersects(xf.rect) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let count = found.count();
    let at = place(sel, bar_size(count, found.more), xf.rect.shrink(4.0), vertical, super::task_bar_rect(ui.ctx()));
    let px = (CELL * ui.ctx().pixels_per_point()).round().clamp(8.0, 256.0) as u32;
    let (mut pick, mut more) = (None, false);
    egui::Area::new(bar_id()).order(egui::Order::Middle).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(PAD as i8 - 1))
            .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: egui::Color32::from_black_alpha(70) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    for i in 0..count {
                        let (rect, resp) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 2, t.hover);
                        }
                        ui.painter().rect_stroke(rect.shrink(0.5), 0, Stroke::new(0.5, t.border), StrokeKind::Inside);
                        if let Some(tex) = found.texture(ui.ctx(), i, px) {
                            let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                            ui.painter().image(tex.id(), rect, uv, t.text_strong);
                        }
                        let tip = found.tip(i);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
                        if resp.on_hover_text(&tip).clicked() {
                            pick = Some(i);
                        }
                    }
                    if found.more {
                        let (rect, resp) = ui.allocate_exact_size(vec2(MORE, CELL), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 2, t.hover);
                        }
                        let c = rect.center();
                        let arrow = vec![pos2(c.x - 2.5, c.y - 5.0), pos2(c.x + 3.5, c.y), pos2(c.x - 2.5, c.y + 5.0)];
                        ui.painter().add(Shape::convex_polygon(arrow, t.text_strong, Stroke::NONE));
                        let tip = tl!("Show all alternates in the Glyphs panel");
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
                        more = resp.on_hover_text(tip).clicked();
                    }
                });
            });
    });
    if let Some(i) = pick {
        found.pick(app, i);
    } else if more {
        glyphs::show_alternates(app, ui.ctx());
    }
}

#[cfg(test)]
mod tests;
