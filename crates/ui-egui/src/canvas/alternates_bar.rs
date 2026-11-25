//! Character alternates on the canvas: with a Type tool and exactly one character selected, a
//! small row of that character's alternate glyphs shows beside it. A click on a glyph draws the
//! character with it, as a double-click on the glyph in the Glyphs panel's Show › Alternates for
//! Selection does, through the same command and so as one undo step. An arrow at the end
//! of the row opens the Glyphs panel when the character has more alternates than the row shows.
//! Preferences › Type › Show Character Alternates (`showCharacterAlternates`) turns the row off.
//!
//! Public behavior: selecting a character shows its alternate glyphs next to it, a click replaces
//! the character's glyph, at most five alternates show, and a control to their right opens the
//! Glyphs panel for the rest ("Replace a character with on-canvas alternate glyphs",
//! <https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=479>;
//! [Replace characters with alternate glyphs](https://helpx.adobe.com/illustrator/desktop/design-with-text/special-characters-glyphs/replace-characters-with-alternate-glyphs.html)).
//! The row's place, its size and the preference are VectorCraft's own.

use craft::behavior::overlay::{Align, Side};
use craft::behavior::{DisplayOrder, IconId, ItemSource};
use craft::font_manager::glyphs::{GlyphCell, glyph_painter};
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::IconButton;
use craft::ui::contract::overlay::{Anchor, Dismiss, Outside, Overlay, Place};
use craft::ui::contract::{ActivateOn, Bind, CollectionEvent, Outcome as _, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::popover::Popover;
use craft::ui::runtime::env::{Direction, Env, Label};
use craft::ui::style::SurfaceKind;
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{Pos2, Rect, Ui};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Point;
use vectorcraft_text::FontDb;

use super::CanvasSpace;
use crate::VectorcraftApp;
use crate::panels::glyphs;

/// The document-space highlight quads of bytes `a..b` of text `id`, and whether the text is set
/// vertically. Kept until the document or the selection changes, so the text is laid out once for
/// each, not in every frame. Empty for a character the text does not draw (overset area type).
fn character_quads(app: &mut VectorcraftApp, id: NodeId, (a, b): (usize, usize)) -> Option<(Vec<[Point; 4]>, bool)> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision, id, a, b);
    if let Some((cached, quads)) = &app.canvas.character_quads
        && *cached == key
    {
        return Some(quads.clone());
    }
    let NodeKind::Text(tx) = &st.doc.node(id)?.kind else { return None };
    let lay = vectorcraft_text::layout(FontDb::global(), tx);
    let quads: Vec<[Point; 4]> = vectorcraft_text::selection_quads(&lay, a, b).into_iter().map(|q| q.map(|c| tx.xf * c)).collect();
    let found = (quads, tx.vertical);
    app.canvas.character_quads = Some((key, found.clone()));
    Some(found)
}

/// The side of the selected character's screen box `sel` the row hangs from. The row goes under a
/// character of horizontal type, or over it when the Contextual Task Bar `task_bar` lies under it;
/// beside a character of vertical type, on its physical right, or on its left when the task bar
/// lies on the right. `Side::Start` and `Side::End` follow the reading `direction`, so the physical
/// sides are chosen through it. Room, flipping and sliding are the overlay's.
fn side(sel: Rect, vertical: bool, task_bar: Option<Rect>, direction: Direction) -> Side {
    let column = sel.center().x;
    if !vertical {
        let under = task_bar.is_some_and(|bar| bar.min.x <= column && column <= bar.max.x && bar.min.y >= sel.bottom());
        return if under { Side::Above } else { Side::Below };
    }
    let right = !task_bar.is_some_and(|bar| bar.center().x > column);
    match (right, direction) {
        (true, Direction::Ltr) | (false, Direction::Rtl) => Side::End,
        _ => Side::Start,
    }
}

/// The cells of the row as a tile source: a cell's position is its key.
struct Cells(usize);

impl DisplayOrder<usize> for Cells {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Cells {}

/// The row of alternate glyphs beside the Type tool's one selected character, under the task bar's
/// rect `task_bar` when it shows.
// PLAN: assumed dependencies of vectorcraft-ui-egui: craft-behavior and craft-font-manager (hub-w1's Cargo.toml)
pub(super) fn alternates_bar(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace, task_bar: Option<Rect>) {
    // Not while a press or a drag is choosing the selection: the row would follow the pointer.
    if !app.session.tool_wants_text() || app.session.tool_busy() {
        return;
    }
    let Some(found) = glyphs::on_canvas(app) else { return };
    let Some((quads, vertical)) = character_quads(app, found.id, found.bytes) else { return };
    let corners: Vec<Pos2> = quads.iter().flatten().map(|p| space.view.to_screen(*p)).collect();
    if corners.is_empty() {
        return;
    }
    let sel = Rect::from_points(&corners);
    // A character scrolled off the canvas has no row.
    if !sel.intersects(space.view.rect) {
        return;
    }
    let place = Place::anchored(Anchor::Rect(sel), side(sel, vertical, task_bar, Env::of(ui).direction), Align::Center);
    let bounds = space.view.rect.shrink(4.0);
    let source = found.source();
    let (family, style) = (found.face().family.clone(), found.face().style.clone());
    let ctx = ui.ctx().clone();
    let (mut pick, mut more) = (None, false);
    let _ = Popover::new(tl!("Character Alternates"), Bind::given(true))
        .overlay(|_| Overlay::floating().modeless().bounds(bounds).place(place))
        .dismiss(Dismiss::new().outside(Outside::Keep))
        .escape(false)
        .show(ui, |ui, _| {
            let _ = Bar::new(Label::Empty, BarKind::Tool).surface(SurfaceKind::Floating).show(ui, |ui, _| {
                let grid = TileGrid::<usize>::new(tl!("Character Alternates"))
                    .thumb_size(ThumbSize::Small)
                    .caption(CaptionPlace::None)
                    .card_columns(found.count())
                    .natural_height(true)
                    .activate(ActivateOn::Click)
                    .show_items(ui, &Cells(found.count()), &mut |key, _| {
                        let cell = GlyphCell::Glyph(found.gid(*key).unwrap_or_default());
                        Tile::new(found.tip(*key)).thumb(ThumbAspect::SQUARE, glyph_painter(&ctx, &source, &family, &style, cell))
                    });
                for event in grid.response.events {
                    if let CollectionEvent::Activate(key) = event {
                        pick = Some(key);
                    }
                }
                if found.more {
                    let tip = tl!("Show all alternates in the Glyphs panel");
                    more = IconButton::new(IconId::new("chevron_right")).label(tip).tooltip(tip).show(ui).response().clicked();
                }
            });
        });
    if let Some(i) = pick {
        found.pick(app, i);
    } else if more {
        glyphs::show_alternates(app);
    }
}

#[cfg(test)]
mod tests;
