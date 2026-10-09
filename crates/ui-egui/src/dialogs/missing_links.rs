//! The linked files a document opened without, one at a time: Replace picks another file
//! (`links.relink`), and with Apply to All the other missing files are looked for by name in its
//! folder; Ignore keeps the images showing their saved preview, and with Apply to All ignores the
//! rest. Afterwards, with Preferences › Update Links: Ask When Modified, the modified links are
//! offered for update (`links.update`).
//!
//! Fields: `missing` (`[{name, path, ids}]`, the one asked about first), `modified` (the ids of
//! images whose file was modified), `applyToAll`, `path` (the replacement file; picked when empty)
//! and `discard` (Ignore).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of a missing linked file.
pub const KIND: &str = "missingLinks";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Linked File Not Found").into(),
    body,
    confirm,
    ok: Some("Replace…"),
    discard: Some("Ignore"),
    min_width: 380.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};

/// The ids in `rows` (`[{ids}]`).
fn ids(rows: &[Value]) -> Vec<Value> {
    rows.iter().flat_map(|r| r["ids"].as_array().cloned().unwrap_or_default()).collect()
}

/// After a document opened (`document.open`'s result `r`): ask about its missing linked files,
/// then about its modified ones, then about the fonts it opened without
/// ([`super::missing_fonts`]). The web reads no linked files, so missing ones just show their
/// previews there.
pub fn after_open(app: &mut VectorcraftApp, r: &Value) {
    // The fonts' dialog waits for these questions (one dialog at a time).
    super::missing_fonts::after_open(app, r);
    let rows = |k: &str| r[k].as_array().cloned().unwrap_or_default();
    let (missing, modified) = (rows("missingLinks"), ids(&rows("modifiedLinks")));
    if missing.is_empty() || cfg!(target_arch = "wasm32") {
        if !missing.is_empty() {
            app.status(format!("{} linked file(s) can't be read here: their previews show", missing.len()));
        }
        ask_update(app, modified);
    } else {
        open(app, missing, modified);
    }
    super::settle(app);
}

fn open(app: &mut VectorcraftApp, missing: Vec<Value>, modified: Vec<Value>) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "missing": missing, "modified": modified, "applyToAll": false, "path": "" })));
}

/// With Update Links: Ask When Modified, offer to read the modified linked files `ids` again.
fn ask_update(app: &mut VectorcraftApp, ids: Vec<Value>) {
    if ids.is_empty() {
        return;
    }
    if app.session.prefs.update_links != "askWhenModified" {
        app.status(format!("{} linked image(s) changed on disk: Update Links shows the new versions", ids.len()));
        return;
    }
    let detail = crate::i18n::fmt(
        tl!("{count} linked image(s) changed since this document was saved. Show the new versions?"),
        &[("count", &ids.len().to_string())],
    );
    super::confirm::ask(app, tl!("Update Modified Links"), &detail, "links.update", json!({ "ids": ids }));
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let missing = d.fields.get("missing").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let Some(current) = missing.first() else { return true };
    let rest = missing.len() - 1;
    let name = current["name"].as_str().unwrap_or_default();
    widgets::list_box(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.label(egui::RichText::new(name).font(theme::semibold(12.0)).color(t.text_strong));
                let path = current["path"].as_str().unwrap_or_default();
                ui.add(egui::Label::new(egui::RichText::new(path).size(11.5).color(t.text_dim)).truncate()).on_hover_text(path);
                ui.add_space(3.0);
            });
        });
    });
    ui.add_space(8.0);
    widgets::dim_label(ui, tl!("Replace it with another file, or ignore it: its images keep showing the preview saved with the document."));
    if rest > 0 {
        ui.add_space(10.0);
        let all = d.bool("applyToAll");
        let label = crate::i18n::fmt(tl!("Apply to All ({count} more missing)"), &[("count", &rest.to_string())]);
        if widgets::check(ui, &label, all, true) {
            d.fields.insert("applyToAll".into(), json!(!all));
        }
    }
    false
}

/// Replace (with `path`, else a picked file) or Ignore (`discard`) the first missing file, then
/// ask about the next.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let missing = d.fields.get("missing").and_then(Value::as_array).cloned().unwrap_or_default();
    let modified = d.fields.get("modified").and_then(Value::as_array).cloned().unwrap_or_default();
    let all = d.bool("applyToAll");
    let Some((current, rest)) = missing.split_first() else {
        app.ui.dialog = None;
        return Ok(Value::Null);
    };
    let mut rest = rest.to_vec();
    let mut out = Value::Null;
    if d.bool("discard") {
        if all {
            rest.clear();
        }
    } else {
        let path = match d.str("path") {
            p if !p.is_empty() => p,
            _ => {
                let pick = crate::FilePick { filters: vectorcraft_engine::cmd::fileio::place_filters().collect(), ..Default::default() };
                crate::picks::open(app, &pick).ok_or("cancelled")?
            }
        };
        out = app.run("links.relink", json!({ "ids": current["ids"], "path": path }))?;
        // Apply to All: the others by name in the same folder; any not there are asked about.
        let folder = std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned());
        if let (true, false, Some(folder)) = (all, rest.is_empty(), folder) {
            let r = app.run("links.relink", json!({ "ids": ids(&rest), "folder": folder }))?;
            let done = r["relinked"].as_array().cloned().unwrap_or_default();
            rest.retain(|row| row["ids"].as_array().is_none_or(|ids| ids.iter().any(|id| !done.contains(id))));
        }
    }
    if rest.is_empty() {
        app.ui.dialog = None;
        ask_update(app, modified);
    } else {
        open(app, rest, modified);
    }
    Ok(out)
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
        image::RgbaImage::from_pixel(400, 300, image::Rgba([r, g, b, 255]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A document with two placed linked files, saved to a fresh folder; the files are then
    /// deleted → (app, folder, document path).
    fn opened_without_links(name: &str) -> (VectorcraftApp, std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("vectorcraft-missing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        for n in ["a.png", "b.png"] {
            let p = dir.join(n);
            std::fs::write(&p, png([200, 0, 0])).unwrap();
            app.run("file.place", json!({"path": p.to_string_lossy()})).unwrap();
        }
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::remove_file(dir.join(n)).unwrap();
        }
        crate::io::open_path(&mut app, &doc).unwrap();
        (app, dir, doc)
    }

    #[test]
    fn opening_without_linked_files_asks_to_replace_or_ignore_each() {
        let (mut app, dir, _) = opened_without_links("each");
        let d = app.ui.dialog.as_ref().expect("the dialog");
        assert_eq!((d.kind.as_str(), d.fields["missing"].as_array().unwrap().len()), (KIND, 2));
        let text = frame(&mut app);
        for label in ["Linked File Not Found", "a.png", "Apply to All (1 more missing)", "Replace…", "Ignore", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // Replace the first with a file given by path (the file picker otherwise).
        let blue = dir.join("new/blue.png");
        std::fs::create_dir_all(blue.parent().unwrap()).unwrap();
        std::fs::write(&blue, png([0, 0, 200])).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("path".into(), json!(blue.to_string_lossy()));
        let r = super::super::confirm(&mut app).unwrap();
        assert_eq!(r["relinked"].as_array().unwrap().len(), 1);
        let d = app.ui.dialog.as_ref().expect("asks about the next");
        assert_eq!(d.fields["missing"][0]["name"], "b.png");
        assert!(!d.fields.contains_key("discard"));
        assert!(frame(&mut app).contains("b.png"));
        // Ignore: done, the image keeps its preview.
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let c = app.run("links.check", json!({})).unwrap();
        let status: Vec<&str> = c["links"].as_array().unwrap().iter().map(|l| l["status"].as_str().unwrap()).collect();
        assert_eq!((status, c["missing"].as_u64()), (vec!["ok", "missing"], Some(1)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_to_all_replaces_the_others_from_the_same_folder() {
        let (mut app, dir, _) = opened_without_links("all");
        let found = dir.join("found");
        std::fs::create_dir_all(&found).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::write(found.join(n), png([0, 0, 200])).unwrap();
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("applyToAll".into(), json!(true));
        d.fields.insert("path".into(), json!(found.join("a.png").to_string_lossy()));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none(), "both relinked");
        let c = app.run("links.check", json!({})).unwrap();
        assert_eq!(c["missing"], 0, "{c}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn modified_links_are_offered_for_update_when_the_preference_asks() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-modified-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let pic = dir.join("a.png");
        std::fs::write(&pic, png([200, 0, 0])).unwrap();
        app.run("file.place", json!({"path": pic.to_string_lossy()})).unwrap();
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        std::fs::write(&pic, png([0, 0, 200])).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        let d = app.ui.dialog.as_ref().expect("asks");
        assert_eq!((d.kind.as_str(), d.str("__command")), (super::super::confirm::KIND, "links.update".to_string()));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.run("links.check", json!({})).unwrap()["modified"], 0);
        // Manually: no question.
        app.run("prefs.set", json!({"key": "updateLinks", "value": "manually"})).unwrap();
        std::fs::write(&pic, png([0, 200, 0])).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        assert!(app.ui.dialog.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
