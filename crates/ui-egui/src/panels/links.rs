//! Links panel: every image in the document, linked files with their status (missing, modified)
//! and embedded images, as rows with a thumbnail, name, page and status. Clicking a row selects
//! its image; the buttons Relink, Go To, Update and Edit Original act on the selected rows; Link
//! Info shows the details of the first. The flyout adds Relink to Folder, Embed / Unembed,
//! Placement Options, Show filters, Sort, thumbnail size and Show in Folder.
//!
//! Everything runs through the `links.*` commands (`links.editOriginal` and `links.reveal` are UI
//! commands: they hand the file to the system).

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::{pstate, set_pstate};
use crate::theme::{self, Tokens};
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, icons};

pub const ID: &str = "links";
/// Seconds between two looks at the linked files (they change outside the app).
const RECHECK: f64 = 2.0;
const SHOW: [(&str, &str); 4] = [("all", "Show All"), ("missing", "Show Missing"), ("modified", "Show Modified"), ("embedded", "Show Embedded")];
const SORT: [(&str, &str); 3] = [("name", "Sort by Name"), ("kind", "Sort by Kind"), ("status", "Sort by Status")];
const THUMBS: [(f32, &str); 4] = [(0.0, "No Thumbnails"), (18.0, "Small Thumbnails"), (28.0, "Medium Thumbnails"), (44.0, "Large Thumbnails")];

/// The panel's options (flyout).
#[derive(Clone)]
struct Options {
    show: &'static str,
    sort: Option<&'static str>,
    /// Thumbnail side (pt); 0 hides them.
    thumb: f32,
    /// The Link Info section is open.
    info: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { show: "all", sort: None, thumb: 28.0, info: true }
    }
}

/// `links.list` and the first selected image's `links.info`, with what they were fetched for.
#[derive(Clone, Default)]
struct Cache {
    /// (document uid, revision, show, sort).
    key: Option<(u64, u64, &'static str, Option<&'static str>)>,
    at: f64,
    list: Value,
    info: Option<(u64, Value)>,
}

fn options(ctx: &egui::Context) -> Options {
    pstate(ctx, "links-options")
}

fn set_options(ctx: &egui::Context, f: impl FnOnce(&mut Options)) {
    let mut o = options(ctx);
    f(&mut o);
    set_pstate(ctx, "links-options", o);
}

/// The rows to show (cached per document revision, the files looked at again every [`RECHECK`]
/// seconds) and the Link Info of the first selected image when the section is open.
fn cached(app: &mut VectorcraftApp, ctx: &egui::Context, o: &Options) -> Option<Cache> {
    let st = app.session.active()?;
    let key = Some((st.uid, st.revision, o.show, o.sort));
    let first = selected_images(app).first().copied();
    let now = ctx.input(|i| i.time);
    let mut c: Cache = pstate(ctx, "links-cache");
    let stale = c.key != key || now - c.at > RECHECK;
    if stale {
        c.list = app.session.execute("links.list", &json!({ "show": o.show, "sort": o.sort })).ok()?;
        (c.key, c.at) = (key, now);
    }
    let want = first.filter(|_| o.info);
    if stale || c.info.as_ref().map(|(id, _)| *id) != want {
        c.info = want.and_then(|id| Some((id, app.session.execute("links.info", &json!({ "id": id })).ok()?)));
    }
    set_pstate(ctx, "links-cache", c.clone());
    // Files change on disk without a frame of ours: look again later.
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(RECHECK));
    Some(c)
}

/// The ids of the selected image objects and placed documents.
fn selected_images(app: &VectorcraftApp) -> Vec<u64> {
    let Some(st) = app.session.active() else { return vec![] };
    let image = |id: &NodeId| {
        st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_) | vectorcraft_doc::NodeKind::PlacedDocument(_)))
    };
    st.selection.objects.iter().filter(|id| image(id)).map(|id| id.0).collect()
}

/// The rows of `list` whose image is selected.
fn selected_rows<'a>(list: &'a Value, selected: &[u64]) -> Vec<&'a Value> {
    list["links"].as_array().into_iter().flatten().filter(|r| r["id"].as_u64().is_some_and(|id| selected.contains(&id))).collect()
}

/// The ids of `rows` that `keep` accepts.
fn ids(rows: &[&Value], keep: impl Fn(&Value) -> bool) -> Vec<u64> {
    rows.iter().filter(|r| keep(r)).filter_map(|r| r["id"].as_u64()).collect()
}

fn status_is(r: &Value, status: &str) -> bool {
    r["status"].as_str() == Some(status)
}

fn report(app: &mut VectorcraftApp, r: Result<Value, String>, done: impl FnOnce(&Value) -> String) {
    let msg = match r {
        Ok(v) => done(&v),
        Err(e) => e,
    };
    app.status(msg);
}

/// Relink images `ids` to a picked file.
pub(crate) fn relink(app: &mut VectorcraftApp, ids: Vec<u64>) {
    crate::picks::button(app, move |app| relink_picked(app, ids.clone()));
}

fn relink_picked(app: &mut VectorcraftApp, ids: Vec<u64>) {
    let pick = crate::FilePick { filters: vectorcraft_engine::cmd::fileio::place_filters().collect(), ..Default::default() };
    let Some(path) = crate::picks::open(app, &pick) else { return };
    let r = app.run("links.relink", json!({ "ids": ids, "path": path }));
    report(app, r, |v| format!("Relinked {} image(s)", v["relinked"].as_array().map_or(0, Vec::len)));
}

/// Relink images `ids` (none: every missing one) to the files of their names in a picked folder.
fn relink_to_folder(app: &mut VectorcraftApp, ids: Vec<u64>) {
    crate::picks::button(app, move |app| relink_to_picked_folder(app, ids.clone()));
}

fn relink_to_picked_folder(app: &mut VectorcraftApp, ids: Vec<u64>) {
    let Some(folder) = crate::picks::folder(app) else { return };
    let ids = Some(ids).filter(|i| !i.is_empty());
    let r = app.run("links.relink", json!({ "ids": ids, "folder": folder }));
    report(app, r, |v| {
        let missed: Vec<&str> = v["notFound"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let n = v["relinked"].as_array().map_or(0, Vec::len);
        if missed.is_empty() { format!("Relinked {n} image(s)") } else { format!("Relinked {n} image(s); not in that folder: {}", missed.join(", ")) }
    });
}

/// Select image `id` and scroll it into view.
pub(crate) fn go_to(app: &mut VectorcraftApp, id: u64) {
    let Ok(r) = app.run("links.goTo", json!({ "id": id })) else { return };
    let b: Vec<f64> = r["bounds"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    if let (&[x0, y0, x1, y1], Some(v)) = (&b[..], app.view_mut()) {
        v.center = vectorcraft_geom::Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        v.fitted = true;
    }
}

/// Write embedded image `id` to a picked file (the web downloads it) and link it there.
pub(crate) fn unembed(app: &mut VectorcraftApp, id: u64, name: &str) {
    if let Some(dl) = app.services.download.as_mut() {
        if let Ok(v) = app.session.execute("links.unembed", &json!({ "id": id }))
            && let (Some(name), Some(bytes)) = (v["name"].as_str(), v["dataBase64"].as_str().and_then(vectorcraft_format::base64_decode))
        {
            dl(name, &bytes);
        }
        return;
    }
    let name = name.to_string();
    crate::picks::button(app, move |app| {
        let Some(path) = crate::picks::save(app, &crate::FilePick::named(&name)) else { return };
        let r = app.run("links.unembed", json!({ "id": id, "path": path }));
        report(app, r, |v| format!("Unembedded to {}", v["path"].as_str().unwrap_or_default()));
    });
}

/// The UI commands `links.editOriginal` (open the linked file in its app) and `links.reveal`
/// (show it in its folder) for image `id` (default: the selected linked image) → `{path}`.
pub(crate) fn open_file(app: &mut VectorcraftApp, p: &Value, reveal: bool) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).or_else(|| {
        let list = app.session.execute("links.list", &json!({})).ok()?;
        let sel = selected_images(app);
        ids(&selected_rows(&list, &sel), |r| r["linked"] == true).first().copied()
    });
    let info = app.session.execute("links.info", &json!({ "id": id })).map_err(|e| e.to_string())?;
    let path = info["path"].as_str().ok_or("select a linked image: an embedded one has no file")?.to_string();
    if info["status"] == "missing" {
        return Err(format!("{path} can't be found: relink it first"));
    }
    // A placed document's file opens here, in a new tab: saving it updates this document.
    if !reveal && info["document"] == true {
        let opened = app.run("document.open", json!({ "path": path }))?;
        // Its missing fonts are asked about as for a document opened with File › Open.
        crate::dialogs::missing_fonts::after_open(app, &opened);
        crate::dialogs::settle(app);
        return Ok(json!({ "path": path }));
    }
    if reveal { crate::io::reveal_path(app, &path) } else { crate::io::open_in_app(app, &path) }?;
    Ok(json!({ "path": path }))
}

/// A status badge: missing (red, ?), modified (amber, !), embedded (the image icon).
fn badge(ui: &Ui, r: egui::Rect, status: &str) {
    let t = Tokens::get(ui.ctx());
    let c = r.center();
    let p = ui.painter();
    let font = theme::semibold(10.0);
    match status {
        "missing" => {
            p.circle_filled(c, 6.5, t.error);
            p.text(c, egui::Align2::CENTER_CENTER, "?", font, egui::Color32::WHITE);
        }
        "modified" => {
            let pts = vec![c + vec2(0.0, -7.0), c + vec2(7.0, 6.0), c + vec2(-7.0, 6.0)];
            p.add(egui::Shape::convex_polygon(pts, t.warning, egui::Stroke::NONE));
            p.text(c + vec2(0.0, 1.5), egui::Align2::CENTER_CENTER, "!", font, egui::Color32::BLACK);
        }
        "embedded" => icons::paint(ui, "image", egui::Rect::from_center_size(c, vec2(13.0, 13.0)), t.text_dim),
        _ => {}
    }
}

fn status_tip(r: &Value) -> String {
    let path = r["found"].as_str().or(r["path"].as_str()).unwrap_or_default();
    match r["status"].as_str().unwrap_or_default() {
        "missing" => crate::i18n::fmt(tl!("Missing: {path}"), &[("path", path)]),
        "modified" => crate::i18n::fmt(tl!("Modified since it was read: {path}"), &[("path", path)]),
        "embedded" => tl!("Embedded").into(),
        _ => path.to_string(),
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let o = options(ui.ctx());
    let Some(cache) = cached(app, ui.ctx(), &o) else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return };
    let selected = selected_images(app);
    let rows: &[Value] = cache.list["links"].as_array().map_or(&[], Vec::as_slice);
    let mut clicked: Option<(u64, bool, bool)> = None;
    widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.set_width(ui.available_width());
        if rows.is_empty() {
            let detail = if o.show == "all" {
                tl!("File › Place… brings in an image, linked or embedded.")
            } else {
                tl!("Nothing matches the Show filter.")
            };
            super::empty_state(ui, "link", tl!("No images"), detail);
            return;
        }
        egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
            let h = o.thumb.max(16.0) + 6.0;
            for r in rows {
                let Some(id) = r["id"].as_u64() else { continue };
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                if selected.contains(&id) {
                    ui.painter().rect_filled(rect, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.hover);
                }
                let mut x = rect.left() + 6.0;
                if o.thumb > 0.0 {
                    let tr = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - o.thumb / 2.0), vec2(o.thumb, o.thumb));
                    ui.painter().rect_filled(tr, 0.0, egui::Color32::WHITE);
                    if let Some(n) = doc.node(NodeId(id)) {
                        super::layers::real_thumb(ui, &doc, n, tr);
                    }
                    ui.painter().rect_stroke(tr, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    x = tr.right() + 6.0;
                }
                let right = rect.right() - 22.0;
                badge(
                    ui,
                    egui::Rect::from_center_size(egui::pos2(right + 10.0, rect.center().y), vec2(16.0, 16.0)),
                    r["status"].as_str().unwrap_or_default(),
                );
                let mut text_right = right - 4.0;
                if let Some(page) = r["page"].as_u64() {
                    let g = ui.painter().layout_no_wrap(format!("{page}"), egui::FontId::proportional(11.5), t.text_dim);
                    text_right -= g.size().x;
                    ui.painter().galley(egui::pos2(text_right, rect.center().y - g.size().y / 2.0), g, t.text_dim);
                    text_right -= 6.0;
                }
                let name = r["name"].as_str().unwrap_or_default();
                let clip = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(text_right.max(x), rect.bottom()));
                ui.painter().with_clip_rect(clip).text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    name,
                    egui::FontId::proportional(12.5),
                    t.text,
                );
                let resp = resp.on_hover_text(status_tip(r));
                if resp.double_clicked() {
                    clicked = Some((id, false, true));
                } else if resp.clicked() {
                    clicked = Some((id, ui.input(|i| i.modifiers.shift || i.modifiers.command), false));
                }
            }
        });
    });
    match clicked {
        Some((id, _, true)) => go_to(app, id),
        Some((id, true, _)) => {
            app.run("select.toggle", json!({ "id": id })).ok();
        }
        Some((id, false, _)) => {
            app.run("select.set", json!({ "ids": [id] })).ok();
        }
        None => {}
    }
    if o.info {
        let units = app.session.general_unit();
        link_info(ui, cache.info.as_ref().map(|(_, i)| i), units);
    }
    let sel = selected_rows(&cache.list, &selected);
    let modified = ids(&sel, |r| status_is(r, "modified") || r["preview"] == true && !status_is(r, "missing"));
    let can_edit = sel.iter().any(|r| r["linked"] == true && !status_is(r, "missing"));
    let linked = ids(&sel, |r| r["linked"] == true);
    widgets::bottom_bar(ui, |ui| {
        let info = !o.info;
        if widgets::icon_button(ui, if o.info { "chevron-down" } else { "chevron-right" }, tl!("Show Link Info"), false, 24.0).clicked() {
            set_options(ui.ctx(), |o| o.info = info);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button_enabled(ui, "pencil", tl!("Edit Original"), false, can_edit, 24.0).clicked() {
                crate::menus::invoke(app, "links.editOriginal", json!({}));
            }
            if widgets::icon_button_enabled(ui, "rotate-cw", tl!("Update Link"), false, !modified.is_empty(), 24.0).clicked() {
                let r = app.run("links.update", json!({ "ids": modified }));
                report(app, r, |v| format!("Updated {} image(s)", v["updated"].as_array().map_or(0, Vec::len)));
            }
            if widgets::icon_button_enabled(ui, "zoom-in", tl!("Go To Link"), false, sel.len() == 1, 24.0).clicked()
                && let Some(id) = sel.first().and_then(|r| r["id"].as_u64())
            {
                go_to(app, id);
            }
            if widgets::icon_button_enabled(ui, "link", tl!("Relink…"), false, !sel.is_empty() && can_pick(app), 24.0).clicked() {
                relink(app, ids(&sel, |_| true));
            }
            if widgets::icon_button_enabled(ui, "link-2-off", tl!("Break Link"), false, !linked.is_empty(), 24.0).clicked() {
                break_link(app, linked.clone());
            }
        });
    });
}

/// Break Link (`links.embed`) of `ids`: images keep their files' pixels, placed documents become
/// editable copies of their art.
fn break_link(app: &mut VectorcraftApp, ids: Vec<u64>) {
    let r = app.run("links.embed", json!({ "ids": ids }));
    report(app, r, |v| {
        let missing = v["missing"].as_array().map_or(0, Vec::len);
        let n = v["embedded"].as_array().map_or(0, Vec::len);
        if missing == 0 { format!("Broke {n} link(s)") } else { format!("Broke {n} link(s); {missing} can't be read: relink or update them first") }
    });
}

/// Pick-a-file actions need a file dialog (the desktop app).
fn can_pick(app: &VectorcraftApp) -> bool {
    app.services.pick_open.is_some()
}

/// Bytes as B, KB or MB.
fn size_label(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Milliseconds since the Unix epoch as a UTC date and time.
pub(crate) fn date_label(ms: u64) -> String {
    let [y, mo, d, h, mi, _] = vectorcraft_doc::metadata::civil(i64::try_from(ms / 1000).unwrap_or(i64::MAX));
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")
}

/// The Link Info section: the details of `info` (`links.info`), or a hint.
fn link_info(ui: &mut Ui, info: Option<&Value>, units: vectorcraft_doc::Unit) {
    widgets::divider(ui);
    let Some(i) = info else {
        widgets::dim_label(ui, tl!("Select an image to see its Link Info."));
        return;
    };
    let pair = |k: &str| {
        let n: Vec<f64> = i[k].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
        match n[..] {
            [x, y] if (x - y).abs() < 0.05 => format!("{x:.0}"),
            [x, y] => format!("{x:.0} × {y:.0}"),
            _ => String::new(),
        }
    };
    let row = super::doc_info::row;
    row(ui, tl!("Name"), i["fileName"].as_str().or(i["name"].as_str()).unwrap_or_default().into());
    row(ui, tl!("Format"), i["format"].as_str().unwrap_or_default().into());
    row(ui, tl!("Color Space"), i["colorMode"].as_str().unwrap_or_default().into());
    if let Some(l) = i["location"].as_str() {
        row(ui, tl!("Location"), l.into());
    }
    // A placed document has no resolution: its rows are left out.
    for (label, k) in [(tl!("PPI"), "ppi"), (tl!("Effective PPI"), "effectivePpi")] {
        if i.get(k).is_some() {
            row(ui, label, pair(k));
        }
    }
    // Pixels, or a placed document's artboard size.
    let dims = match (i["pixelWidth"].as_u64(), i["pageWidth"].as_f64(), i["pageHeight"].as_f64()) {
        (Some(w), _, _) => format!("{w} × {} px", i["pixelHeight"].as_u64().unwrap_or(0)),
        (None, Some(w), Some(h)) => format!("{} × {}", units.format(w), units.format(h)),
        _ => String::new(),
    };
    row(ui, tl!("Dimensions"), dims);
    let scale: Vec<f64> = i["scale"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    if let [x, y] = scale[..] {
        row(ui, tl!("Scale"), format!("H: {x:.1}%  V: {y:.1}%"));
    }
    row(ui, tl!("Rotation"), format!("{:.1}°", i["rotation"].as_f64().unwrap_or(0.0)));
    if let Some(b) = i["fileSize"].as_u64() {
        row(ui, tl!("Size"), size_label(b));
    }
    for (k, label) in [("created", tl!("Created")), ("modified", tl!("Modified"))] {
        if let Some(ms) = i[k].as_u64() {
            row(ui, label, date_label(ms));
        }
    }
    if let Some(p) = i["page"].as_u64() {
        row(ui, tl!("Page"), p.to_string());
    }
    let status = match i["status"].as_str().unwrap_or_default() {
        "ok" => tl!("Linked"),
        "missing" => tl!("Missing"),
        "modified" => tl!("Modified"),
        _ => tl!("Embedded"),
    };
    row(ui, tl!("Status"), status.into());
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let o = options(ui.ctx());
    // The rows the panel shows (the menu opens from it).
    let list = pstate::<Cache>(ui.ctx(), "links-cache").list;
    let selected = selected_images(app);
    let sel = selected_rows(&list, &selected);
    let linked = ids(&sel, |r| r["linked"] == true);
    let embedded: Vec<&Value> = sel.iter().copied().filter(|r| status_is(r, "embedded")).collect();
    let any = !sel.is_empty();
    let missing_any = list["missing"].as_u64().unwrap_or(0) > 0;
    if menu_item(ui, tl!("Relink…"), any && can_pick(app), false) {
        relink(app, ids(&sel, |_| true));
    }
    if menu_item(ui, tl!("Relink to Folder…"), (any || missing_any) && app.services.pick_folder.is_some(), false) {
        relink_to_folder(app, ids(&sel, |_| true));
    }
    if menu_item(ui, tl!("Go To Link"), sel.len() == 1, false)
        && let Some(id) = sel.first().and_then(|r| r["id"].as_u64())
    {
        go_to(app, id);
    }
    if menu_item(ui, tl!("Update Link"), sel.iter().any(|r| status_is(r, "modified")), false) {
        let r = app.run("links.update", json!({ "ids": ids(&sel, |r| status_is(r, "modified")) }));
        report(app, r, |v| format!("Updated {} image(s)", v["updated"].as_array().map_or(0, Vec::len)));
    }
    if menu_item(ui, tl!("Edit Original"), sel.iter().any(|r| r["linked"] == true && !status_is(r, "missing")), false) {
        crate::menus::invoke(app, "links.editOriginal", json!({}));
    }
    ui.separator();
    if menu_item(ui, tl!("Embed Image(s)"), !linked.is_empty(), false) {
        let r = app.run("links.embed", json!({ "ids": linked }));
        report(app, r, |v| {
            let missing = v["missing"].as_array().map_or(0, Vec::len);
            let n = v["embedded"].as_array().map_or(0, Vec::len);
            if missing == 0 { format!("Embedded {n} image(s)") } else { format!("Embedded {n} image(s); {missing} missing: relink them first") }
        });
    }
    if menu_item(ui, tl!("Unembed…"), embedded.len() == 1, false)
        && let Some(r) = embedded.first()
        && let Some(id) = r["id"].as_u64()
    {
        let name = r["name"].as_str().unwrap_or("Image").to_string();
        unembed(app, id, &name);
    }
    if menu_item(ui, tl!("Placement Options…"), any, false) {
        crate::menus::invoke(app, "ui.placementOptionsDialog", json!({ "ids": ids(&sel, |_| true) }));
    }
    if menu_item(ui, tl!("Show in Folder"), sel.iter().any(|r| r["linked"] == true && !status_is(r, "missing")), false) {
        crate::menus::invoke(app, "links.reveal", json!({}));
    }
    ui.separator();
    for (id, label) in SHOW {
        if menu_item(ui, tl!(label), true, o.show == id) {
            set_options(ui.ctx(), |o| o.show = id);
        }
    }
    ui.separator();
    for (id, label) in SORT {
        if menu_item(ui, tl!(label), true, o.sort == Some(id)) {
            set_options(ui.ctx(), |o| o.sort = if o.sort == Some(id) { None } else { Some(id) });
        }
    }
    ui.separator();
    for (px, label) in THUMBS {
        if menu_item(ui, tl!(label), true, o.thumb == px) {
            set_options(ui.ctx(), |o| o.thumb = px);
        }
    }
    if menu_item(ui, tl!("Show Link Info"), true, o.info) {
        set_options(ui.ctx(), |o| o.info = !o.info);
    }
}
