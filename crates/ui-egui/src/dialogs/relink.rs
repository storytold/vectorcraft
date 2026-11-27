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

// PLAN: assumed craft_ui paths for CloseReason, HoldsOk, List, ListRow, SelectMode, ResetFooter, ResetPick, Field and craft_behavior's DisplayOrder/ItemSource.
use std::borrow::Cow;

use craft::behavior::{DisplayOrder, ItemSource};
use craft::host::{Host, PickId, PickRequest};
use craft::ui::contract::SelectMode;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::{FormDialog, ResetFooter, ResetPick};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::overlay::CloseReason;
use serde_json::{Value, json};

use crate::VectorcraftApp;

const KEY: &str = "relink";
const CMD: &str = "links.relink";
/// The Replace choices, in the order the select lists them.
const SCOPES: [&str; 4] = ["each", "selected", "all", "missingInFolder"];
/// The rows the link list shows before it scrolls.
const LIST_ROWS: u16 = 6;

fn scope_label(scope: &str) -> Label {
    match scope {
        "each" => tl!("Each Link Separately").into(),
        "selected" => tl!("Selected Instances").into(),
        "all" => tl!("All Instances").into(),
        _ => tl!("Missing Links from This Folder").into(),
    }
}

/// Open the dialog on the images and placed documents `ids` (default: the selected ones).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let document = app.session.active().ok_or("no document open")?.uid;
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
    let dialog = RelinkDialog::new(document, ids, links);
    crate::dialogs::host::open_held(app, dialog, relink).ok_or("no window")?;
    Ok(Value::Null)
}

/// The relink rule: one `links.relink` call with the answer's parameters, and the status it earns.
fn relink(app: &mut VectorcraftApp, (document, params): &(u64, Value)) -> Result<(), String> {
    if app.session.active().is_none_or(|st| st.uid != *document) {
        let reason = tl!("Relink can only change links in the document where it was opened.");
        app.status(reason);
        return Err(reason.to_owned());
    }
    let r = app.run(CMD, params.clone())?;
    let missed: Vec<&str> = r["notFound"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    app.status(crate::panels::links::relinked_status_missed(r["relinked"].as_array().map_or(0, Vec::len), &missed));
    Ok(())
}

/// The link names as the list's rows; a row's key is its place.
struct Names<'a>(&'a [String]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|name| Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Names<'_> {}

/// Relink…: the links to relink and the Replace choice. OK holds its answer, the `links.relink` parameters, for the
/// app (`HoldsOk`); with Each Link Separately it stays open until every link has a file.
pub(crate) struct RelinkDialog {
    id: DialogId,
    /// The document that owns the link IDs.
    document: u64,
    ids: Vec<Value>,
    links: Vec<Value>,
    scope: String,
    path: String,
    paths: Vec<String>,
    /// The file picker OK asked for, answered in a later pass.
    pick: Option<PickId>,
    held: HeldAnswer<Value>,
}

impl RelinkDialog {
    fn new(document: u64, ids: Vec<Value>, links: Vec<Value>) -> Self {
        RelinkDialog {
            id: dialog_id(KEY),
            document,
            ids,
            links,
            scope: "selected".into(),
            path: String::new(),
            paths: vec![],
            pick: None,
            held: HeldAnswer::new(),
        }
    }

    fn names(&self) -> Vec<String> {
        self.links.iter().map(|l| l["name"].as_str().unwrap_or_default().to_owned()).collect()
    }

    /// OK: the file in `path` (set by automation), else a file picked from the user. Does nothing while an answer is
    /// held or a pick is open.
    fn ok_step(&mut self, ctx: &egui::Context) {
        if self.held.is_waiting() || self.pick.is_some() {
            return;
        }
        if !self.path.is_empty() {
            let path = std::mem::take(&mut self.path);
            self.advance(path);
            return;
        }
        let Some(host) = Host::try_of_ctx(ctx) else { return };
        let mut request = PickRequest::open("relink");
        for (name, extensions) in vectorcraft_engine::cmd::fileio::place_filters() {
            request = request.filter(name, extensions);
        }
        self.pick = Some(host.files().pick(request));
    }

    /// `path` is the file for the next link. Nothing relinks before the last link has one.
    fn advance(&mut self, path: String) {
        if self.scope == "each" {
            self.paths.push(path);
            if self.paths.len() < self.links.len() {
                return;
            }
            let entries: Vec<Value> = self.links.iter().zip(&self.paths).map(|(l, p)| json!({ "ids": l["ids"], "path": p })).collect();
            self.held.file(json!({ "scope": "each", "paths": entries }));
        } else {
            self.held.file(json!({ "ids": self.ids, "path": path, "scope": self.scope }));
        }
    }

    fn forget_pick(&mut self, ctx: &egui::Context) {
        if let (Some(ticket), Some(host)) = (self.pick.take(), Host::try_of_ctx(ctx)) {
            host.files().forget(ticket);
        }
    }
}

impl HoldsOk for RelinkDialog {
    type Answer = (u64, Value);

    fn held_ok(self) -> Self {
        self
    }

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.held.take().map(|params| (self.document, params))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        // A refused complete pass starts over so each new pick is assigned to a link.
        if answer.is_err() && self.scope == "each" {
            self.paths.clear();
        }
        self.held.answer(answer);
    }
}

impl StackDialog for RelinkDialog {
    type Result = Value;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<Value>> {
        let finish = self.held.is_accepted();
        let ticket = self.pick;
        let names = self.names();
        let (each, done) = (self.scope == "each", self.paths.len());
        let entries: Vec<Entry<String>> = SCOPES.iter().map(|s| Entry::new((*s).to_owned(), scope_label(s))).collect();
        let paths = &self.paths;
        let mut taken = None;
        let footer = ResetFooter::new(ui, None, true);
        let shown = FormDialog::new(tl!("Relink"), Bind::bound(&mut self.scope))
            .ok_label(tl!("Relink…"))
            .accept_stays_open(true)
            .enter_confirms(false)
            .ok_enabled(!self.held.is_waiting() && ticket.is_none())
            .show(ui, |ui, cx, scope| {
                // A measuring run closes nothing: the real run of the pass makes the close.
                if finish && !craft::ui::runtime::measure::active(ui.ctx()) {
                    cx.request_close_as(CloseReason::Activated);
                }
                taken = ticket.and_then(|ticket| cx.take_pick_answer(ticket));
                let source = Names(&names);
                List::new(tl!("Links")).id_salt("links").select(SelectMode::None).max_rows(LIST_ROWS).show_items(ui, &source, &mut |&i, _| {
                    let row = ListRow::new(names[i].clone()).strong(each && i == done);
                    match paths.get(i).filter(|_| each) {
                        Some(path) => {
                            let name = std::path::Path::new(path).file_name().map_or(path.clone(), |n| n.to_string_lossy().into_owned());
                            row.trailing_named(format!("→ {name}"), path.clone())
                        }
                        None => row,
                    }
                });
                Field::new(tl!("Replace:")).show(ui, |ui, _| {
                    if let Some(next) = Select::new(tl!("Replace:"), Bind::given(scope.clone()), &entries).show(ui).value().cloned() {
                        *scope = next;
                    }
                });
                true
            });
        let ok = matches!(footer.pick(shown.picked), Some(ResetPick::Accept));
        if let Some(picked) = taken {
            self.pick = None;
            if let Some(path) = picked.paths().into_iter().next() {
                self.advance(path.to_string_lossy().into_owned());
            }
        }
        if ok {
            self.ok_step(ui.ctx());
        }
        match shown.closed {
            Some(CloseReason::Activated) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok)),
            Some(_) => {
                self.forget_pick(ui.ctx());
                Some(DialogResult::Cancel)
            }
            None => None,
        }
    }

    fn confirm(&mut self, ctx: &egui::Context) -> Option<Value> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        self.ok_step(ctx);
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn cancelled(&mut self, ctx: &egui::Context) {
        self.forget_pick(ctx);
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "ids": self.ids, "links": self.links, "scope": self.scope, "path": self.path, "paths": self.paths }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let text = || value.as_str().map(str::to_owned).ok_or_else(|| FieldRefusal::Rejects(&value).message(field));
        match field {
            "ids" | "links" => return Err(FieldRefusal::ReadOnly.message(field)),
            "scope" => {
                let scope = text()?;
                if !SCOPES.contains(&scope.as_str()) {
                    return Err(FieldRefusal::Rejects(&value).message(field));
                }
                self.scope = scope;
            }
            "path" => self.path = text()?,
            "paths" => {
                let list = value.as_array().and_then(|a| a.iter().map(|p| p.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>());
                self.paths = list.ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        ctx.request_repaint();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::picks::{answer_dialog, open_dialogs, scripted};
    use vectorcraft_engine::Session;

    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let [r, g, b] = rgb;
        let img = craft_image_codecs::Image::from_raw(
            40,
            30,
            craft_image_codecs::ChannelLayout::Rgba,
            craft_image_codecs::SampleType::U8,
            [r, g, b, 255].repeat(40 * 30),
        )
        .unwrap();
        craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap()
    }

    /// A fresh folder with the PNG files `names` → (app with a document, folder).
    fn setup(name: &str, names: &[&str]) -> (VectorcraftApp, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("vectorcraft-relink-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("new")).unwrap();
        for n in names {
            std::fs::write(dir.join(n), png([200, 0, 0])).unwrap();
        }
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
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

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("a window")
    }

    /// One frame of the dialogs, the held-OK pump, and the frame that closes an accepted dialog.
    fn pass(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        let dialogs = app.dialogs().expect("a window");
        for _ in 0..2 {
            // With the app's frame input, as the hosts give it: the dialog takes its pick's answer from the host there.
            crate::testing::part_frame(app, &ctx, egui::RawInput::default(), |_, ui| dialogs.with(|host| host.run(ui)));
            crate::dialogs::host::pump(app, &ctx);
        }
    }

    fn relink_dialog(app: &VectorcraftApp) -> craft::ui::dialog::OpenDialog {
        open_dialogs(app).into_iter().find(|d| d.key.as_deref() == Some(KEY)).expect("the relink dialog")
    }

    fn fields(app: &VectorcraftApp) -> Value {
        relink_dialog(app).fields.expect("fields")
    }

    /// Sets `set` and presses OK of the open Relink dialog.
    fn ok(app: &mut VectorcraftApp, set: &[(&str, Value)]) {
        let id = relink_dialog(app).id;
        answer_dialog(app, id, set, true);
        pass(app);
    }

    fn check_relink_stays_in_opening_drawing(native_picker: bool) {
        let (mut app, dir) = setup(if native_picker { "opening-picker" } else { "opening-path" }, &["source.png", "destination.png"]);
        let picker = scripted(&mut app);
        let original = dir.join("source.png");
        let destination = dir.join("destination.png");
        let source_id = place(&mut app, &original);
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let destination_id = place(&mut app, &destination);
        assert_eq!(source_id, destination_id, "linked objects may have the same local ID");
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.relinkDialog", json!({"ids": [source_id]})).unwrap();
        if native_picker {
            ok(&mut app, &[]);
        }

        let selected = new_file(&dir, "replacement.png");
        app.run("document.activate", json!({"index": 1})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        if native_picker {
            picker.answer(&[&selected]);
            pass(&mut app);
        } else {
            ok(&mut app, &[("path", json!(selected))]);
        }
        assert_eq!(link_path(&app, destination_id), destination.to_string_lossy());
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert_eq!(app.ui.status, "Relink can only change links in the document where it was opened.");
        let dialog = relink_dialog(&app).id;
        assert!(matches!(app.dialogs().unwrap().with(|h| h.held_state(dialog)), Some(HeldOk::Refused(_))));

        app.run("document.activate", json!({"index": 0})).unwrap();
        ok(&mut app, &[("path", json!(selected))]);
        assert!(open_dialogs(&app).is_empty());
        assert_eq!(link_path(&app, source_id), selected);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn relink_opening_drawing_is_required_for_a_path_confirmation() {
        check_relink_stays_in_opening_drawing(false);
    }

    #[test]
    fn relink_opening_drawing_is_required_when_the_file_picker_answers() {
        check_relink_stays_in_opening_drawing(true);
    }

    #[test]
    fn relink_opens_with_the_replace_choice_and_all_instances_relinks_every_copy() {
        let (mut app, dir) = setup("all", &["photo.png", "other.png"]);
        let (a, b, c) = (place(&mut app, &dir.join("photo.png")), place(&mut app, &dir.join("photo.png")), place(&mut app, &dir.join("other.png")));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        assert!(crate::menus::enabled(&app, "ui.relinkDialog"));
        app.run("ui.relinkDialog", json!({})).unwrap();
        let f = fields(&app);
        assert_eq!((f["scope"].as_str(), f["links"].as_array().map(Vec::len)), (Some("selected"), Some(1)));
        let picked = new_file(&dir, "blue.png");
        ok(&mut app, &[("scope", json!("all")), ("path", json!(picked))]);
        assert!(open_dialogs(&app).is_empty());
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
        let picker = scripted(&mut app);
        let (a, b) = (place(&mut app, &dir.join("one.png")), place(&mut app, &dir.join("two.png")));
        let (x, y) = (new_file(&dir, "x.png"), new_file(&dir, "y.png"));
        app.run("ui.relinkDialog", json!({"ids": [a, b]})).unwrap();
        ok(&mut app, &[("scope", json!("each"))]);
        picker.answer(&[&x]);
        pass(&mut app);
        assert_eq!(fields(&app)["paths"], json!([x]), "asks for the next link's file");
        assert_eq!(link_path(&app, a), dir.join("one.png").to_string_lossy(), "nothing relinked yet");
        ok(&mut app, &[]);
        picker.answer(&[&y]);
        pass(&mut app);
        assert!(open_dialogs(&app).is_empty());
        assert_eq!((link_path(&app, a), link_path(&app, b)), (x.clone(), y.clone()));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(
            (link_path(&app, a), link_path(&app, b)),
            (dir.join("one.png").to_string_lossy().into_owned(), dir.join("two.png").to_string_lossy().into_owned()),
            "one undo step"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn each_link_separately_lets_the_user_choose_new_paths_after_a_refusal() {
        let (mut app, dir) = setup("each-after-refusal", &["one.png", "two.png"]);
        let picker = scripted(&mut app);
        let ids = [place(&mut app, &dir.join("one.png")), place(&mut app, &dir.join("two.png"))];
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.relinkDialog", json!({"ids": ids})).unwrap();
        ok(&mut app, &[("scope", json!("each"))]);
        picker.answer(&[&new_file(&dir, "old-one.png")]);
        pass(&mut app);
        ok(&mut app, &[]);
        app.run("document.activate", json!({"index": 1})).unwrap();
        picker.answer(&[&new_file(&dir, "old-two.png")]);
        pass(&mut app);
        let dialog = relink_dialog(&app).id;
        assert!(matches!(app.dialogs().unwrap().with(|h| h.held_state(dialog)), Some(HeldOk::Refused(_))));

        app.run("document.activate", json!({"index": 0})).unwrap();
        let paths = [new_file(&dir, "new-one.png"), new_file(&dir, "new-two.png")];
        for (i, path) in paths.iter().enumerate() {
            ok(&mut app, &[]);
            picker.answer(&[path]);
            pass(&mut app);
            if i == 0 {
                assert_eq!(fields(&app)["paths"], json!([path]), "the second link still needs its new file");
            }
        }
        assert!(open_dialogs(&app).is_empty());
        assert_eq!(ids.map(|id| link_path(&app, id)), paths);
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
        let f = fields(&app);
        let rows = f["links"].as_array().unwrap();
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
        assert_eq!(fields(&app)["links"], json!([{"name": "photo.png", "ids": [a, b]}]));
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
        let (ctx, dialogs) = (ctx(&app), app.dialogs().expect("a window"));
        for open in open_dialogs(&app) {
            dialogs.with(|host| host.cancel_by_id(&ctx, open.id));
        }
        let chosen = new_file(&dir, "renamed.png");
        let found = new_file(&dir, "b.png");
        app.run("ui.relinkDialog", json!({"ids": [a]})).unwrap();
        ok(&mut app, &[("scope", json!("missingInFolder")), ("path", json!(chosen))]);
        assert!(open_dialogs(&app).is_empty());
        assert_eq!((link_path(&app, a), link_path(&app, b)), (chosen, found));
        let _ = std::fs::remove_dir_all(dir);
    }
}
