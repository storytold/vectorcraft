use craft::behavior::{CanvasView, EditEvent};
use craft::ui::canvas::{ArtboardEdge, PageBorder, PageFrame, Pasteboard, PasteboardColor, PasteboardOutput};
use craft::ui::contract::{Rename, Show as _};
use craft::ui::primitives::Checker;
use craft::ui::style::DataColor;
use egui::Color32;
use serde_json::json;
use vectorcraft_geom::{BezPath, Point};

use super::{BodyKind, CanvasSpace, VectorcraftApp};
use crate::View;
use crate::doc_windows::Win;

/// The length on screen, in points, of each arm of an artboard's center mark (a cross), at any zoom.
const CENTER_MARK_ARM: f32 = 6.0;

fn closed(r: vectorcraft_geom::Rect) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(Point::new(r.x0, r.y0));
    for (x, y) in [(r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
        path.line_to(Point::new(x, y));
    }
    path.close_path();
    path
}

/// The transparency grid's cells: the setup's two colors and its cell size.
pub(super) fn checker_of(setup: &vectorcraft_doc::DocSetup) -> Checker {
    let [light, dark] = setup.grid_colors.map(|c| DataColor::new(crate::panels::c32(&c)));
    Checker::new().size(setup.grid_size.cell()).colors(light, dark)
}

/// The page frames of document `st` seen through `view`: one per artboard, named "01 - name" with the
/// Artboard tool or several boards, filled with its paper or the transparency grid (nothing in Outline).
/// Empty while the artboards are hidden.
pub(super) fn frames(app: &VectorcraftApp, st: &vectorcraft_engine::DocState, view: &View, outline: bool) -> Vec<PageFrame> {
    if !shows_artboards(app) {
        return Vec::new();
    }
    let setup = &st.doc.setup;
    let paper = paper_color(setup);
    let grid = shows_transparency_grid(st);
    let named = app.session.tool_id() == "artboard" || st.doc.artboards.len() > 1;
    st.doc
        .artboards
        .iter()
        .enumerate()
        .map(|(i, ab)| {
            let mut frame = PageFrame::new(ab.rect).active(i == view.artboard).renames(!ab.locked);
            if named {
                frame = frame.name(format!("{:02} - {}", i + 1, ab.name));
            }
            if outline {
                frame
            } else if grid {
                frame.checker(checker_of(setup))
            } else {
                frame.fill(DataColor::new(artboard_paper(ab, paper)))
            }
        })
        .collect()
}

/// A line round each artboard in Outline, the drop shadow in Preview, and no border in Overprint Preview (and
/// Separations Preview, which turns it on).
pub(super) fn border(outline: bool) -> PageBorder {
    if outline {
        PageBorder::Line
    } else if vectorcraft_render::proof::overprint_preview_on() {
        PageBorder::None
    } else {
        PageBorder::DropShadow
    }
}

/// The pasteboard's color: white in Outline, the paper under Hide Artboards, white in Overprint Preview (and Separations
/// Preview, which turns it on), else User Interface › Canvas Color, or the interface's when that matches the interface
/// brightness.
pub(super) fn pasteboard_color(app: &VectorcraftApp, st: &vectorcraft_engine::DocState, outline: bool) -> PasteboardColor {
    if outline {
        PasteboardColor::Custom(DataColor::new(Color32::WHITE))
    } else if !shows_artboards(app) {
        PasteboardColor::Custom(DataColor::new(paper_color(&st.doc.setup)))
    } else if vectorcraft_render::proof::overprint_preview_on() {
        PasteboardColor::Custom(DataColor::new(Color32::WHITE))
    } else {
        crate::prefs_dialog::canvas_color(&app.session.prefs.canvas_color)
            .map_or(PasteboardColor::Default, |c| PasteboardColor::Custom(DataColor::new(c)))
    }
}

/// Applies what the pasteboard reported: a click on a name selects its artboard, a committed rename
/// renames it. Returns the document point for the pasteboard menu. `Begin` and `Cancel` change nothing.
pub(super) fn apply(app: &mut VectorcraftApp, out: &PasteboardOutput) -> Option<Point> {
    if let Some((i, mods)) = out.name_clicked {
        select_named_artboard(app, i, mods.shift);
    }
    if let Some(EditEvent::Commit { after: Rename { key, name, .. }, .. }) = &out.rename {
        commit_artboard_name(app, *key, name);
    }
    out.context_at
}

/// Shows the pasteboard and artboards of window `win`'s document. `Live` applies what it reports and
/// returns the menu point; `Pane` is display only.
pub(super) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, space: &CanvasSpace, win: Win, kind: BodyKind) -> Option<Point> {
    let (Some(st), Some(view)) = (app.session.documents().get(win.0), crate::doc_windows::view(app, win)) else { return None };
    let outline = view.outline;
    let list = frames(app, st, view, outline);
    // The pasteboard's colour is the canvas's (`Canvas::pasteboard`).
    let out =
        Pasteboard::new("Artboards", &list).border(border(outline)).artboard_edge(ArtboardEdge::Plain).shows_outside().show_at(ui, space.view.rect);
    match kind {
        BodyKind::Live => apply(app, &out),
        BodyKind::Pane => None,
    }
}

/// The pasteboard outside the artboards as screen quads, for the fade.
pub(super) fn fade_quads(view: &CanvasView, frames: &[PageFrame]) -> Vec<[egui::Pos2; 4]> {
    Pasteboard::quads(view, frames, &[])
}

/// The outline of each artboard's bleed, in document space.
pub(super) fn bleed_paths(st: &vectorcraft_engine::DocState) -> Vec<BezPath> {
    let setup = &st.doc.setup;
    if !setup.has_bleed() {
        return Vec::new();
    }
    st.doc.artboards.iter().map(|ab| closed(setup.bleed_rect(ab.rect))).collect()
}

/// The display marks of every artboard as document paths: with Show Cross Hairs, a line from the
/// middle of each side to the middle of the opposite side; with Show Center Mark, a cross at the
/// center whose arms run along the sides, [`CENTER_MARK_ARM`] points long on screen at `zoom`; with
/// Show Video Safe Areas, the outlines of its action-safe and title-safe areas.
pub(super) fn mark_paths(st: &vectorcraft_engine::DocState, zoom: f64) -> Vec<BezPath> {
    let arm = f64::from(CENTER_MARK_ARM) / zoom.max(1e-9);
    let line = |a: (f64, f64), b: (f64, f64)| {
        let mut path = BezPath::new();
        path.move_to(Point::new(a.0, a.1));
        path.line_to(Point::new(b.0, b.1));
        path
    };
    let mut paths = Vec::new();
    for ab in &st.doc.artboards {
        let (r, c) = (ab.rect, ab.rect.center());
        if ab.show_cross_hairs {
            paths.push(line((r.x0, c.y), (r.x1, c.y)));
            paths.push(line((c.x, r.y0), (c.x, r.y1)));
        }
        if ab.show_center_mark {
            paths.push(line((c.x - arm, c.y), (c.x + arm, c.y)));
            paths.push(line((c.x, c.y - arm), (c.x, c.y + arm)));
        }
        if ab.show_video_safe_areas {
            paths.extend(ab.video_safe_areas().into_iter().map(closed));
        }
    }
    paths
}

/// Do the artboards show? View › Hide Artboards hides them, except for the Artboard tool, which
/// edits them.
pub(super) fn shows_artboards(app: &VectorcraftApp) -> bool {
    app.ui.view.artboards || app.session.tool_id() == "artboard"
}

/// The color of the paper: Simulate Colored Paper's, else white.
pub(super) fn paper_color(setup: &vectorcraft_doc::DocSetup) -> Color32 {
    if setup.simulate_paper { crate::panels::c32(&setup.paper()) } else { Color32::WHITE }
}

/// The color of artboard `ab`'s paper, the paper `paper`: an artboard with a background color
/// multiplies it with the paper, each sRGB channel artboard × paper / 255, rounded.
pub(super) fn artboard_paper(ab: &vectorcraft_doc::Artboard, paper: Color32) -> Color32 {
    ab.color.map_or(paper, |c| crate::panels::c32(&c) * paper)
}

/// The transparency grid covers the artboards of document `st`: it is shown, and the Background
/// Contents are transparent.
pub(super) fn shows_transparency_grid(st: &vectorcraft_engine::DocState) -> bool {
    st.transparency_grid && st.doc.setup.background == vectorcraft_doc::Background::Transparent
}

/// Does the area outside the artboards draw darker? Artboard Options › Fade Region Outside Artboard
/// darkens it while the Artboard tool is active, and with Update While Dragging off it draws as
/// usual while the tool moves or resizes an artboard. Presentation Mode, exports and prints never
/// draw the fade.
pub(super) fn fades_outside_artboards(app: &VectorcraftApp) -> bool {
    let prefs = &app.session.prefs;
    app.ui.screen_mode != 3
        && app.session.tool_id() == "artboard"
        && prefs.fade_region_outside_artboard
        && (prefs.artboard_update_while_dragging || !app.session.tool_transforming())
}

pub(super) fn select_named_artboard(app: &mut VectorcraftApp, index: usize, shift: bool) {
    let n = app.session.active().map(|st| st.doc.artboards.len()).unwrap_or(0);
    if index >= n {
        return;
    }
    if let Some(v) = app.view_mut() {
        v.artboard = index;
    }
    if app.session.tool_id() == "artboard" {
        if shift {
            crate::panels::artboards::toggle(app, index, n);
        } else {
            app.session.set_tool_option("active", &json!(index));
        }
    }
}

pub(super) fn commit_artboard_name(app: &mut VectorcraftApp, index: usize, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let current = app.session.active().and_then(|st| st.doc.artboards.get(index).map(|a| a.name.clone()));
    if current.as_deref() == Some(trimmed) {
        return;
    }
    // A refusal is reported by the command; the label just closes.
    app.run("artboard.setProps", json!({ "index": index, "name": trimmed })).ok();
}

/// Is overlay label `text` the Artboard tool's "01 - <artboard name>"? It holds a name, so it is
/// shown as it is; the tools' other labels ("anchor", "path", Puppet Warp's warning) are ours.
pub(super) fn names_an_artboard(text: &str) -> bool {
    text.split_once(" - ").is_some_and(|(n, _)| n.len() >= 2 && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests;
