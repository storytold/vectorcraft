//! Links panel → Relink…: the selected links, and the Replace choice of which links the new file
//! takes. Selected Instances relinks the selected images and placed documents, All Instances every
//! object linked to the same files, and Missing Links from This Folder the selected ones and then
//! every other missing link whose file name is in the new file's folder. Each Link Separately asks
//! for a file for each link in turn, one OK each, and relinks them all at the last one. Every
//! choice is one `links.relink` call: one undo step.
//!
//! Fields: `ids` (the images and placed documents), `links` (`[{name, ids}]`, one row per linked
//! file, as `links.check` lists them, then one per embedded image), `scope` (each|selected|all|missingInFolder; default
//! selected), `path` (the file; picked when empty) and `paths` (with each: the files chosen so
//! far, one per row of `links` in order; the next OK takes `path` or picks one for the next row).

use serde_json::{Value, json};

use super::DialogSpec;
use super::form;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "relink";
const CMD: &str = "links.relink";
/// (value, label) of each Replace choice.
const SCOPES: [(&str, &str); 4] = [
    ("each", "Each Link Separately"),
    ("selected", "Selected Instances"),
    ("all", "All Instances"),
    ("missingInFolder", "Missing Links from This Folder"),
];
const LABEL: f32 = 70.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Relink").into(),
    body,
    confirm,
    ok: Some("Relink…"),
    min_width: 400.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};

/// Open the dialog on the images and placed documents `ids` (default: the selected ones).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let ids = match p.get("ids").filter(|v| !v.is_null()) {
        Some(v) => v.as_array().cloned().ok_or_else(|| format!("`ids` must be an array of object ids, not {v}"))?,
        None => crate::panels::links::selected_ids(app).into_iter().map(Value::from).collect(),
    };
    if ids.is_empty() {
        return Err("select the images to relink, or pass ids".into());
    }
    let check = app.session.execute("links.check", &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    // One row per file: objects showing different content of one file (pages of a PDF) share it.
    let mut by_path: Vec<(Value, Value, Vec<Value>)> = vec![];
    for l in check["links"].as_array().into_iter().flatten() {
        let ids = l["ids"].as_array().cloned().unwrap_or_default();
        match by_path.iter_mut().find(|(path, _, _)| *path == l["path"]) {
            Some((_, _, all)) => all.extend(ids),
            None => by_path.push((l["path"].clone(), l["name"].clone(), ids)),
        }
    }
    let mut links: Vec<Value> = by_path.into_iter().map(|(_, name, ids)| json!({ "name": name, "ids": ids })).collect();
    // Embedded images have no link: each gets a row of its own, named as the Links panel lists it.
    let linked: Vec<&Value> = links.iter().flat_map(|l| l["ids"].as_array().into_iter().flatten()).collect();
    let mut embedded: Vec<Value> = vec![];
    for id in ids.iter().filter(|id| !linked.contains(id)) {
        if !embedded.contains(id) {
            embedded.push(id.clone());
        }
    }
    if !embedded.is_empty() {
        let list = app.session.execute("links.list", &json!({})).map_err(|e| e.to_string())?;
        let rows = list["links"].as_array().cloned().unwrap_or_default();
        for id in embedded {
            let name = rows.iter().find(|r| r["id"] == id).map_or(Value::Null, |r| r["name"].clone());
            links.push(json!({ "name": name, "ids": [id] }));
        }
    }
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "ids": ids, "links": links, "scope": "selected", "path": "", "paths": [] })));
    Ok(Value::Null)
}

fn rows(d: &Dialog, key: &str) -> Vec<Value> {
    d.fields.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The file the next OK uses: `path`, else one picked (none: cancelled).
fn file(app: &mut VectorcraftApp, d: &Dialog) -> Result<String, String> {
    match d.str("path") {
        p if !p.is_empty() => Ok(p),
        _ => {
            let pick = crate::FilePick { filters: crate::FilePick::filters(vectorcraft_engine::cmd::fileio::place_filters()), ..Default::default() };
            crate::picks::open(app, &pick).ok_or_else(|| "cancelled".to_string())
        }
    }
}

/// OK: relink with the chosen scope. With each, a file for the next link: the dialog stays open
/// until every link has one, then relinks them all.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let scope = d.str("scope");
    let r = if scope == "each" {
        let links = rows(d, "links");
        let mut paths = rows(d, "paths");
        if paths.len() < links.len() {
            paths.push(json!(file(app, d)?));
        }
        if paths.len() < links.len() {
            let mut next = d.clone();
            next.fields.insert("paths".into(), json!(paths));
            next.fields.insert("path".into(), json!(""));
            app.ui.dialog = Some(next);
            return Ok(json!({ "next": links.get(paths.len()).map_or(Value::Null, |l| l["name"].clone()) }));
        }
        let entries: Vec<Value> = links.iter().zip(&paths).map(|(l, p)| json!({ "ids": l["ids"], "path": p })).collect();
        app.run(CMD, json!({ "scope": "each", "paths": entries }))?
    } else {
        let path = file(app, d)?;
        app.run(CMD, json!({ "ids": d.fields.get("ids").cloned().unwrap_or(Value::Null), "path": path, "scope": scope }))?
    };
    app.ui.dialog = None;
    let missed: Vec<&str> = r["notFound"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    app.status(crate::panels::links::relinked_status_missed(r["relinked"].as_array().map_or(0, Vec::len), &missed));
    Ok(r)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let links = rows(d, "links");
    let paths = rows(d, "paths");
    let each = d.str("scope") == "each";
    widgets::list_box(ui, |ui| {
        ui.set_width(ui.available_width());
        for (i, l) in links.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                // With each: the link the next OK picks a file for is in bold.
                let font = if each && i == paths.len() { theme::semibold(12.0) } else { egui::FontId::proportional(12.0) };
                ui.label(egui::RichText::new(l["name"].as_str().unwrap_or_default()).font(font).color(t.text_strong));
                if let Some(p) = paths.get(i).and_then(Value::as_str).filter(|_| each) {
                    let name = std::path::Path::new(p).file_name().map_or(p.into(), |n| n.to_string_lossy());
                    ui.add(egui::Label::new(egui::RichText::new(format!("→ {name}")).size(11.5).color(t.text_dim)).truncate()).on_hover_text(p);
                }
            });
        }
    });
    ui.add_space(8.0);
    form::choice(ui, d, "scope", tl!("Replace:"), (LABEL, 260.0), &SCOPES);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let mut out = vec![];
        let [r, g, b] = rgb;
        image::RgbaImage::from_pixel(40, 30, image::Rgba([r, g, b, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A fresh folder with the PNG files `names` → (app with a document, folder).
    fn setup(name: &str, names: &[&str]) -> (VectorcraftApp, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("vectorcraft-relink-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("new")).unwrap();
        for n in names {
            std::fs::write(dir.join(n), png([200, 0, 0])).unwrap();
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        (app, dir)
    }

    fn place(app: &mut VectorcraftApp, path: &std::path::Path) -> u64 {
        app.run("file.place", json!({"path": path.to_string_lossy()})).unwrap()["ids"][0].as_u64().unwrap()
    }

    fn link_path(app: &VectorcraftApp, id: u64) -> String {
        let st = app.session.active().unwrap();
        match st.doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind) {
            Some(vectorcraft_doc::NodeKind::Image(im)) => im.link.as_ref().map(|l| l.path.clone()).unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn new_file(dir: &std::path::Path, name: &str) -> String {
        let p = dir.join("new").join(name);
        std::fs::write(&p, png([0, 0, 200])).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn relink_opens_with_the_replace_choice_and_all_instances_relinks_every_copy() {
        let (mut app, dir) = setup("all", &["photo.png", "other.png"]);
        let (a, b, c) = (place(&mut app, &dir.join("photo.png")), place(&mut app, &dir.join("photo.png")), place(&mut app, &dir.join("other.png")));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.services.pick_open = None;
        assert!(!crate::menus::enabled(&app, "ui.relinkDialog"), "no file picker (the web)");
        let picked = new_file(&dir, "blue.png");
        let answer = picked.clone();
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(answer.clone())));
        assert!(crate::menus::enabled(&app, "ui.relinkDialog"));
        app.run("ui.relinkDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().expect("the dialog");
        assert_eq!((d.kind.as_str(), d.str("scope"), d.fields["links"].as_array().unwrap().len()), (KIND, "selected".to_string(), 1));
        let text = frame(&mut app);
        for label in ["Relink", "photo.png", "Replace:", "Selected Instances", "Relink…", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        app.ui.dialog.as_mut().unwrap().fields.insert("scope".into(), json!("all"));
        // OK picks the file.
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r["relinked"], json!([a, b]));
        assert!(app.ui.dialog.is_none());
        assert_eq!((link_path(&app, a), link_path(&app, b)), (picked.clone(), picked.clone()));
        assert_eq!(link_path(&app, c), dir.join("other.png").to_string_lossy());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(link_path(&app, b), dir.join("photo.png").to_string_lossy(), "one undo step");
        // Nothing selected and no ids: nothing to relink.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.relinkDialog"));
        assert!(app.run("ui.relinkDialog", json!({})).is_err());
        assert!(app.run("ui.relinkDialog", json!({"ids": 3})).is_err(), "ids is a list");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn each_link_separately_asks_for_a_file_for_each_link_then_relinks_them_all() {
        let (mut app, dir) = setup("each", &["one.png", "two.png"]);
        let (a, b) = (place(&mut app, &dir.join("one.png")), place(&mut app, &dir.join("two.png")));
        let (x, y) = (new_file(&dir, "x.png"), new_file(&dir, "y.png"));
        let mut answers = vec![y.clone(), x.clone()];
        app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| answers.pop()));
        app.run("ui.relinkDialog", json!({"ids": [a, b]})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("scope".into(), json!("each"));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r, json!({"next": "two.png"}));
        let d = app.ui.dialog.as_ref().expect("asks for the next link's file");
        assert_eq!(d.fields["paths"], json!([x]));
        assert_eq!(link_path(&app, a), dir.join("one.png").to_string_lossy(), "nothing relinked yet");
        assert!(frame(&mut app).contains("→ x.png"));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r["relinked"], json!([a, b]));
        assert!(app.ui.dialog.is_none());
        assert_eq!((link_path(&app, a), link_path(&app, b)), (x.clone(), y.clone()));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(
            (link_path(&app, a), link_path(&app, b)),
            (dir.join("one.png").to_string_lossy().into_owned(), dir.join("two.png").to_string_lossy().into_owned()),
            "one undo step"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An embedded image in the selection gets a row of its own, so Each Link Separately asks for
    /// its file too.
    #[test]
    fn embedded_images_get_their_own_rows() {
        let (mut app, dir) = setup("embedded", &["one.png"]);
        let (a, b) = (place(&mut app, &dir.join("one.png")), place(&mut app, &dir.join("one.png")));
        app.run("links.embed", json!({"ids": [b]})).unwrap();
        app.run("ui.relinkDialog", json!({"ids": [a, b, b]})).unwrap();
        let d = app.ui.dialog.as_ref().expect("the dialog");
        let rows = d.fields["links"].as_array().unwrap();
        assert_eq!((rows.len(), &rows[0]["ids"], &rows[1]["ids"]), (2, &json!([a]), &json!([b])));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Objects showing different content of one file share its row, so Each Link Separately asks
    /// for that file once.
    #[test]
    fn one_file_with_two_contents_is_one_row() {
        let (mut app, dir) = setup("one-file", &["photo.png"]);
        let a = place(&mut app, &dir.join("photo.png"));
        std::fs::write(dir.join("photo.png"), png([0, 200, 0])).unwrap();
        let b = place(&mut app, &dir.join("photo.png"));
        app.run("ui.relinkDialog", json!({"ids": [a, b]})).unwrap();
        let d = app.ui.dialog.as_ref().expect("the dialog");
        assert_eq!(d.fields["links"], json!([{"name": "photo.png", "ids": [a, b]}]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_links_from_this_folder_relinks_the_others_found_there() {
        let (mut app, dir) = setup("missing", &["a.png", "b.png"]);
        let (a, b) = (place(&mut app, &dir.join("a.png")), place(&mut app, &dir.join("b.png")));
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::remove_file(dir.join(n)).unwrap();
        }
        app.run("file.close", json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        // The Linked File Not Found question is left for the Links panel.
        app.ui.dialog = None;
        let chosen = new_file(&dir, "renamed.png");
        let found = new_file(&dir, "b.png");
        app.run("ui.relinkDialog", json!({"ids": [a]})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("scope".into(), json!("missingInFolder"));
        d.fields.insert("path".into(), json!(chosen));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r, json!({"relinked": [a, b], "notFound": []}));
        assert_eq!((link_path(&app, a), link_path(&app, b)), (chosen, found));
        let _ = std::fs::remove_dir_all(dir);
    }
}
