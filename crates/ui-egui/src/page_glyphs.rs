//! Glyphs drawn in code for the page-orientation and artboard-layout toggles.

use craft::ui::contract::MemoProps;
use craft::ui::primitives::Glyph;
use craft::ui::style::UiTokens;
use egui::{Painter, Pos2, Rect, Shape, Stroke, vec2};

/// A portrait or landscape sheet with a folded corner. The selected look is the button's own.
#[derive(MemoProps)]
pub struct PageGlyph {
    /// The sheet lies wide.
    pub landscape: bool,
}

impl Glyph for PageGlyph {
    fn paint(&self, painter: &Painter, _tokens: &UiTokens, rect: Rect, color: egui::Color32, _mirrored: bool) {
        let size = if self.landscape { vec2(16.0, 12.0) } else { vec2(12.0, 16.0) };
        let page = Rect::from_center_size(rect.center(), size);
        let fold = 4.0;
        let stroke = Stroke::new(1.2, color);
        let outline =
            vec![page.left_top(), page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(0.0, fold), page.right_bottom(), page.left_bottom()];
        painter.add(Shape::closed_line(outline, stroke));
        let corner = vec![page.right_top() - vec2(fold, 0.0), page.right_top() + vec2(-fold, fold), page.right_top() + vec2(0.0, fold)];
        painter.add(Shape::line(corner, stroke));
    }
}

/// Small artboards in the grid cells `tiles` (column, row), faded, and over them their order as a
/// line from a dot on the first to an arrowhead on the last.
#[derive(MemoProps)]
pub struct LayoutGlyph {
    /// The cells, in the order of the artboards.
    pub tiles: &'static [(u8, u8)],
}

impl Glyph for LayoutGlyph {
    fn paint(&self, painter: &Painter, _tokens: &UiTokens, rect: Rect, color: egui::Color32, _mirrored: bool) {
        let tiles = self.tiles;
        // Cells at most half the room a side: a row's artboards stand tall, a column's lie flat.
        let room = rect.width().min(rect.height()) * 20.0 / 26.0;
        let (cols, rows) = tiles.iter().fold((1u8, 1u8), |(c, r), &(x, y)| (c.max(x.saturating_add(1)), r.max(y.saturating_add(1))));
        let cell = vec2((room / f32::from(cols)).min(room / 2.0), (room / f32::from(rows)).min(room / 2.0));
        let origin = rect.center() - vec2(f32::from(cols) * cell.x, f32::from(rows) * cell.y) / 2.0;
        let centre = |&(x, y): &(u8, u8)| origin + vec2((f32::from(x) + 0.5) * cell.x, (f32::from(y) + 0.5) * cell.y);
        for tile in tiles {
            painter.rect_filled(Rect::from_center_size(centre(tile), cell - vec2(2.0, 2.0)), 1.0, color.gamma_multiply(0.35));
        }
        let path: Vec<Pos2> = tiles.iter().map(centre).collect();
        if let ([first, ..], [.., from, to]) = (path.as_slice(), path.as_slice()) {
            let (first, from, to) = (*first, *from, *to);
            let dir = (to - from).normalized();
            let side = dir.rot90() * 2.5;
            let head = vec![to + dir * 1.5, to - dir * 2.5 + side, to - dir * 2.5 - side];
            painter.circle_filled(first, 1.6, color);
            painter.add(Shape::line(path, Stroke::new(1.2, color)));
            painter.add(Shape::convex_polygon(head, color, Stroke::NONE));
        }
    }
}
