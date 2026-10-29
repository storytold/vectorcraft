use super::{Color32, Point, Pos2, Rect, Shape, Stroke, Ui, VectorcraftApp, Xf, json, vec2};

/// The width an artboard needs on screen for its name to show (px): narrower, the name is left
/// out rather than spilling over its neighbours.
const MIN_LABELLED_ARTBOARD: f32 = 24.0;

/// Temp-memory key recording an artboard-name press before its edit field opens.
pub(super) fn name_press_id() -> egui::Id {
    egui::Id::unique("canvas-artboard-name-press")
}

/// Temp-memory key for the artboard name field currently being edited.
pub(super) fn name_edit_id() -> egui::Id {
    egui::Id::unique("canvas-artboard-name-edit")
}

/// A name being typed above an artboard. `fresh`: this frame opened it, so a focus loss from the
/// press that started it is not the user leaving the field.
#[derive(Clone)]
pub(super) struct ArtboardNameEdit {
    pub(super) index: usize,
    text: String,
    fresh: bool,
}

/// The canvas label for `name`, laid out as [`artboard_label`] draws it.
fn canvas_name_galley(painter: &egui::Painter, corner: Pos2, width: f32, name: &str, color: Color32) -> Option<(std::sync::Arc<egui::Galley>, Pos2)> {
    if !width.is_finite() || width < MIN_LABELLED_ARTBOARD {
        return None;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), egui::FontId::proportional(11.0), color);
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let at = corner - vec2(0.0, 4.0 + galley.size().y);
    Some((galley, at))
}

/// The length on screen, in points, of each arm of an artboard's center mark (a cross), at any zoom.
const CENTER_MARK_ARM: f32 = 6.0;

/// Artboard `ab`'s display marks in `color`, each line 1 point wide on screen at any zoom and
/// turned with the view: with Show Cross Hairs, a line from the middle of each side through the
/// center to the middle of the opposite side; with Show Center Mark, a cross at the center whose
/// arms run along the artboard's sides, [`CENTER_MARK_ARM`] long on screen
/// (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/modify-display-settings-of-artboards.html);
/// with Show Video Safe Areas, the outlines of its action-safe and title-safe areas
/// ([`vectorcraft_doc::Artboard::video_safe_areas`],
/// https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/set-video-display-options-for-artboards.html).
pub(super) fn artboard_marks(painter: &egui::Painter, xf: &Xf, ab: &vectorcraft_doc::Artboard, color: Color32) {
    let (r, stroke) = (ab.rect, Stroke::new(1.0, color));
    let c = r.center();
    let at = |x: f64, y: f64| xf.to_screen(Point::new(x, y));
    if ab.show_cross_hairs {
        painter.line_segment([at(r.x0, c.y), at(r.x1, c.y)], stroke);
        painter.line_segment([at(c.x, r.y0), at(c.x, r.y1)], stroke);
    }
    if ab.show_center_mark {
        let mid = at(c.x, c.y);
        for side in [at(c.x + 1.0, c.y), at(c.x, c.y + 1.0)] {
            let arm = (side - mid).normalized() * CENTER_MARK_ARM;
            painter.line_segment([mid - arm, mid + arm], stroke);
        }
    }
    if ab.show_video_safe_areas {
        for area in ab.video_safe_areas() {
            painter.add(Shape::closed_line(xf.quad(area), stroke));
        }
    }
}

/// Artboard `name` above its top-left `corner` on screen, no wider than the artboard (`width` on
/// screen): a longer name ends in "…", and an artboard too narrow shows none (#949).
pub(super) fn artboard_label(painter: &egui::Painter, corner: Pos2, width: f32, name: &str, color: Color32) {
    if let Some((galley, at)) = canvas_name_galley(painter, corner, width, name, color) {
        painter.galley(at, galley, color);
    }
}

fn canvas_name_rect(painter: &egui::Painter, corner: Pos2, width: f32, name: &str) -> Option<egui::Rect> {
    canvas_name_galley(painter, corner, width, name, Color32::WHITE).map(|(galley, at)| egui::Rect::from_min_size(at, galley.size()).expand(2.0))
}

/// Where the Artboard tool draws "01 - name" for its active artboard (see its overlay label).
pub(super) fn tool_name_rect(painter: &egui::Painter, xf: &Xf, rect: Rect, index: usize, name: &str) -> egui::Rect {
    let z = xf.zoom.max(1e-9);
    let anchor = Point::new(rect.x0 - 8.0 / z, rect.y0 - 4.0 / z);
    let sp = xf.to_screen(anchor) + vec2(8.0, -14.0);
    let galley = painter.layout(format!("{:02} - {name}", index + 1), egui::FontId::proportional(11.0), Color32::WHITE, f32::INFINITY);
    egui::Rect::from_min_size(sp, galley.size()).expand(2.0)
}

/// The artboard whose on-canvas name contains `screen`, if that name is being drawn.
pub(super) fn artboard_name_at(app: &VectorcraftApp, ui: &Ui, xf: &Xf, screen: Pos2) -> Option<usize> {
    let st = app.session.active()?;
    let artboard_tool = app.session.tool_id() == "artboard";
    if !artboard_tool && st.doc.artboards.len() <= 1 {
        return None;
    }
    let tool_labelled = artboard_tool.then(|| app.session.tool_options()["active"].as_u64()).flatten();
    let painter = ui.painter();
    let mut hit = None;
    for (i, ab) in st.doc.artboards.iter().enumerate() {
        if tool_labelled == u64::try_from(i).ok() {
            continue;
        }
        let corner = xf.to_screen(Point::new(ab.rect.x0, ab.rect.y0));
        let width = (ab.rect.width() * xf.zoom) as f32;
        let text = format!("{:02} - {}", i + 1, ab.name);
        if canvas_name_rect(painter, corner, width, &text).is_some_and(|r| r.contains(screen)) {
            hit = Some(i);
        }
    }
    if let Some(i) = tool_labelled.and_then(|i| usize::try_from(i).ok())
        && let Some(ab) = st.doc.artboards.get(i)
        && tool_name_rect(painter, xf, ab.rect, i, &ab.name).contains(screen)
    {
        return Some(i);
    }
    hit
}

pub(super) fn select_named_artboard(app: &mut VectorcraftApp, index: usize) {
    let n = app.session.active().map(|st| st.doc.artboards.len()).unwrap_or(0);
    if index >= n {
        return;
    }
    if let Some(v) = app.view_mut() {
        v.artboard = index;
    }
    if app.session.tool_id() == "artboard" {
        app.session.set_tool_option("active", &json!(index));
    }
}

/// Start editing artboard `index`'s name on the canvas, unless the artboard is locked (the
/// Artboards panel still renames it).
pub(super) fn begin_artboard_name_edit(ui: &Ui, app: &VectorcraftApp, index: usize) {
    let Some(name) = app.session.active().and_then(|st| st.doc.artboards.get(index).filter(|a| !a.locked).map(|a| a.name.clone())) else { return };
    ui.data_mut(|d| d.insert_temp(name_edit_id(), ArtboardNameEdit { index, text: name, fresh: true }));
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

/// The text field over an artboard name a double-click opened.
pub(super) fn artboard_name_editor(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    let Some(mut edit) = ui.data(|d| d.get_temp::<ArtboardNameEdit>(name_edit_id())) else { return };
    let alive = app.session.active().is_some_and(|st| edit.index < st.doc.artboards.len());
    if !alive {
        ui.data_mut(|d| d.remove::<ArtboardNameEdit>(name_edit_id()));
        return;
    }
    let Some(rect) = artboard_name_field_rect(app, ui, xf, edit.index) else {
        ui.data_mut(|d| d.remove::<ArtboardNameEdit>(name_edit_id()));
        return;
    };
    let fresh = edit.fresh;
    edit.fresh = false;
    let te = ui.put(
        rect,
        egui::TextEdit::singleline(&mut edit.text)
            .id(name_edit_id().with("field"))
            .font(egui::FontId::proportional(11.0))
            // Keep the original inset so theme button padding cannot move the canvas label.
            .margin(egui::Margin::symmetric(4, 2)),
    );
    if fresh || (!te.has_focus() && !te.lost_focus()) {
        te.request_focus();
    }
    let done = !fresh && te.lost_focus();
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let (index, text) = (edit.index, edit.text.clone());
    if done {
        ui.data_mut(|d| d.remove::<ArtboardNameEdit>(name_edit_id()));
        if !escape {
            commit_artboard_name(app, index, &text);
        }
    } else {
        ui.data_mut(|d| d.insert_temp(name_edit_id(), edit));
    }
}

/// Where the rename field sits: the name label's rectangle, at least wide enough to type in.
pub(super) fn artboard_name_field_rect(app: &VectorcraftApp, ui: &Ui, xf: &Xf, index: usize) -> Option<egui::Rect> {
    let st = app.session.active()?;
    let ab = st.doc.artboards.get(index)?;
    let artboard_tool = app.session.tool_id() == "artboard";
    let tool_labelled = artboard_tool.then(|| app.session.tool_options()["active"].as_u64()).flatten();
    let painter = ui.painter();
    let rect = if tool_labelled == u64::try_from(index).ok() {
        tool_name_rect(painter, xf, ab.rect, index, &ab.name)
    } else {
        let corner = xf.to_screen(Point::new(ab.rect.x0, ab.rect.y0));
        let width = (ab.rect.width() * xf.zoom) as f32;
        canvas_name_rect(painter, corner, width, &format!("{:02} - {}", index + 1, ab.name))?
    };
    Some(egui::Rect::from_min_size(rect.min, vec2(rect.width().max(80.0), rect.height().max(16.0))))
}

/// Is overlay label `text` the Artboard tool's "01 - <artboard name>"? It holds a name, so it is
/// shown as it is; the tools' other labels ("anchor", "path", Puppet Warp's warning) are ours.
pub(super) fn names_an_artboard(text: &str) -> bool {
    text.split_once(" - ").is_some_and(|(n, _)| n.len() >= 2 && n.bytes().all(|b| b.is_ascii_digit()))
}
