//! The Import PDF dialog: opening or placing a multi-page or password-protected PDF (or `.ai`)
//! asks which pages (a preview with page navigation), the box to crop to and the password.
//!
//! [`ImportPdfDialog`] is a host dialog. Locked, it shows the framework's password question; once
//! the file reads, it shows the pages form. Its values, as `ui.inspect` lists them and
//! `ui.dialog.set` takes them: `mode` (`open` | `place`), `name`, `path?`, `pageCount`, `page` (the
//! page shown, and the one placed; 1-based), `allPages`, `range` (the pages picked, as "1-3, 5")
//! and `linkPages` (open: Import PDF Pages as Links, for a file read from a path and opened
//! without a password; `null` until it is checked or cleared, when it follows the pages picked: on
//! for All, off for a range; picking the other of All and Range sets it back to `null`; `linksOn`
//! is the effective flag, `__linkable` is false for a file `linkPages` does not take, such as one
//! over its size limit), `cropTo`, `textAs` (`text` | `outlines`; it starts from the `pdfTextAs`
//! preference for a PDF (`__pdf`; Editable for a `.ai` or `.ait` file), or from the `textAs` a place
//! gives (`__textAsGiven`), and OK keeps a PDF's choice there; a linked place shows the file's own
//! text, so placing linked leaves it as it is, and Outlines clears Import PDF Pages as Links) and
//! `password`. `__locked` is set until the password is accepted: OK then unlocks the file, and with
//! `__passwordOnly` (Place with Show Import Options off) places its first page at once. `__place`
//! keeps the other `file.place` params (Link, Template, `at`…). The dialog owns the file's bytes.
//!
//! OK is held: the dialog files its values, the pump opens or places the file and answers, and a
//! refusal keeps the dialog open with the reason.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

// PLAN: `IconId` / `Revision` / `dialog_id` / `HeldAnswer` paths below are assumed.
use craft::behavior::IconId;
use craft::pages::range::{format_pages, parse_pages};
use craft::pages::ui::range::PageRangeField;
use craft::ui::buttons::IconButton;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbSize;
use craft::ui::patterns::form_dialogs::{FormDialog, PasswordDialog, PasswordPurpose};
use craft::ui::pickers::Select;
use craft::ui::primitives::textures::Revision;
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_pdf::{Choice, CropTo, PdfError, TextAs};

use crate::VectorcraftApp;
use crate::io;

pub(super) const KIND: &str = "importPdf";

/// The longest side of the rendered page preview, in points; the picture shows it at its token size.
const PREVIEW_PX: f64 = 200.0;

/// The file an import dialog reads (Import PDF, DXF Import Options).
#[derive(Debug, Default)]
pub struct DialogFile {
    pub bytes: Vec<u8>,
    /// Unique per file the dialog opened (keys its preview).
    pub(super) token: u64,
}

impl DialogFile {
    pub fn new(bytes: Vec<u8>) -> Arc<Self> {
        static TOKEN: AtomicU64 = AtomicU64::new(0);
        Arc::new(Self { bytes, token: TOKEN.fetch_add(1, Ordering::Relaxed) })
    }
}

/// Equal by token: a token names one file's bytes, so a dialog's values compare without reading them.
impl PartialEq for DialogFile {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token
    }
}

/// The file `file.place` params `p` name when it is one `wanted` takes by its extension (a path,
/// read through the app's services) or given as `dataBase64` → (name, bytes, path).
pub(super) fn place_source(app: &VectorcraftApp, p: &Value, wanted: impl Fn(&str) -> bool) -> Option<(String, Vec<u8>, Option<String>)> {
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    match (s("path"), s("dataBase64")) {
        (Some(path), _) if wanted(&fileio::extension(path)) => {
            let bytes = app.services.read.as_ref().and_then(|r| r(path).ok())?;
            Some((path.to_string(), bytes, Some(path.to_string())))
        }
        (None, Some(b64)) => Some((s("name").unwrap_or("Untitled").to_string(), vectorcraft_format::base64_decode(b64)?, None)),
        _ => None,
    }
}

/// The `file.place` params besides the file and `taken` (kept by an import dialog for its OK).
pub(super) fn other_place_params(p: &Value, taken: &[&str]) -> Value {
    let rest =
        p.as_object().into_iter().flatten().filter(|(k, _)| !matches!(k.as_str(), "path" | "name" | "dataBase64") && !taken.contains(&k.as_str()));
    Value::Object(rest.map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// `place` naming the dialog's file again: its `path`, or its `name` and bytes.
pub(super) fn with_file(place: &Value, path: Option<&str>, name: &str, file: &DialogFile) -> Value {
    let mut p = if place.is_object() { place.clone() } else { json!({}) };
    match path {
        Some(path) => p["path"] = json!(path),
        None => {
            p["name"] = json!(name);
            p["dataBase64"] = json!(vectorcraft_format::base64_encode(&file.bytes));
        }
    }
    p
}

/// `doc`'s first artboard rendered for a preview picture, its longer side [`PREVIEW_PX`] points; `None` for a
/// document without an artboard.
pub(super) fn render_preview(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    doc: &vectorcraft_engine::doc::Document,
) -> Option<Arc<egui::ColorImage>> {
    let region = doc.artboards.first()?.rect;
    let px = PREVIEW_PX * ctx.pixels_per_point() as f64;
    let scale = px / region.width().max(region.height()).max(1.0);
    let img = app.canvas.renderer.render_region(doc, region, scale, true);
    Some(Arc::new(crate::canvas::color_image(&img)))
}

/// Everything the dialog asks and shows: the form's value, and what an OK needs to open or place the file.
#[derive(Clone, Default, PartialEq)]
struct State {
    /// Place mode: the `file.place` params besides the file. `None` opens.
    place: Option<Value>,
    name: String,
    path: Option<String>,
    file: Arc<DialogFile>,
    page_count: usize,
    /// The page shown (1-based).
    page: usize,
    all_pages: bool,
    /// The pages picked under Range (0-based, as the range field reports them); empty opens every page.
    pages: Vec<usize>,
    link_pages: Option<bool>,
    crop: CropTo,
    text_as: TextAs,
    pdf: bool,
    password: String,
    locked: bool,
    password_only: bool,
    linkable: bool,
    text_as_given: bool,
}

impl State {
    /// The page shown (1-based, within the file).
    fn page(&self) -> usize {
        self.page.max(1).min(self.page_count.max(1))
    }

    fn password(&self) -> Option<String> {
        Some(self.password.clone()).filter(|p| !p.is_empty())
    }

    /// A place that links the page: from a path, with Link on, without a password (`file.place`
    /// embeds a page read with a password) and without a `textAs` given with the place (`__textAsGiven`:
    /// the Text choice then decides, and Outlines embeds the page). A linked page shows the file's own
    /// text, so the Text choice does not apply to it.
    fn places_linked(&self) -> bool {
        self.place.as_ref().is_some_and(|p| p.get("link").and_then(Value::as_bool) != Some(false))
            && self.path.is_some()
            && self.password().is_none()
            && !self.text_as_given
    }

    /// Can the file's pages open linked: it was read from a path (a linked page is read from its
    /// file again), `document.open`'s `linkPages` takes its size, and it opens without a password
    /// (`linkPages` refuses one).
    fn file_can_link(&self) -> bool {
        self.path.is_some() && self.linkable && self.password().is_none()
    }

    /// Can the pages open linked: [`file_can_link`](Self::file_can_link), with the text kept editable
    /// (`linkPages` refuses `textAs: outlines`).
    fn can_link(&self) -> bool {
        self.file_can_link() && self.text_as == TextAs::Text
    }

    /// Import PDF Pages as Links as checked or cleared, or, when it is `null`, as the pages picked
    /// default it: on for All, off for a range.
    fn link_pages(&self) -> bool {
        self.link_pages.unwrap_or(self.all_pages)
    }

    /// Import PDF Pages as Links is checked, and can be.
    fn links_on(&self) -> bool {
        self.link_pages() && self.can_link()
    }

    /// Picks All (true) or Range. A change of pick sets Import PDF Pages as Links back to that pick's
    /// default; picking the one already picked keeps the box as it is.
    fn pick_all_pages(&mut self, all: bool) {
        if self.all_pages != all {
            self.all_pages = all;
            self.link_pages = None;
        }
    }

    /// Reads the file with the typed password. `Ok(true)` when the file has pages to pick; `Ok(false)` when OK
    /// goes straight on to open or place it.
    fn unlock(&mut self) -> Result<bool, String> {
        let info = vectorcraft_pdf::info(&self.file.bytes, self.password().as_deref()).map_err(|e| e.to_string())?;
        self.locked = false;
        self.page_count = info.pages.len();
        Ok(info.pages.len() > 1 && !self.password_only)
    }

    /// The Text choice the preview renders with (a linked place: as editable text).
    fn preview_text_as(&self) -> TextAs {
        if self.places_linked() { TextAs::Text } else { self.text_as }
    }

    fn to_json(&self) -> Value {
        let mut fields = json!({
            "mode": if self.place.is_some() { "place" } else { "open" },
            "name": self.name,
            "pageCount": self.page_count,
            "page": self.page(),
            "allPages": self.all_pages,
            "range": format_pages(&self.pages),
            "linkPages": self.link_pages,
            "linksOn": self.links_on(),
            "cropTo": self.crop.id(),
            "textAs": self.text_as.id(),
            "password": self.password,
            "__pdf": self.pdf,
            "__locked": self.locked,
            "__passwordOnly": self.password_only,
            "__linkable": self.linkable,
            "__textAsGiven": self.text_as_given,
        });
        if let Some(path) = &self.path {
            fields["path"] = json!(path);
        }
        if let Some(place) = &self.place {
            fields["__place"] = place.clone();
        }
        fields
    }

    /// Sets the field as the user editing that control would, or the file's path as automation does; the `__` fields
    /// and the file's name are not editable.
    fn set_field(&mut self, field: &str, value: Value) -> Result<(), String> {
        let text = || value.as_str().ok_or_else(|| format!("{field} takes text"));
        match field {
            "page" => {
                let n = value.as_f64().ok_or("page takes a number")?;
                self.page = (n.max(1.0) as usize).min(self.page_count.max(1));
            }
            "allPages" => self.pick_all_pages(value.as_bool().ok_or("allPages takes true or false")?),
            "range" => {
                let text = text()?;
                self.pages = if text.trim().is_empty() { Vec::new() } else { parse_pages(text, self.page_count).map_err(|e| e.to_string())? };
            }
            "linkPages" => self.link_pages = if value.is_null() { None } else { Some(value.as_bool().ok_or("linkPages takes true, false or null")?) },
            "cropTo" => {
                let text = text()?;
                let i = CropTo::IDS.iter().position(|id| *id == text).ok_or("cropTo is not a crop box")?;
                self.crop = CropTo::ALL.get(i).copied().unwrap_or_default();
            }
            "textAs" => {
                let id = text()?;
                if !TextAs::IDS.iter().any(|i| i.eq_ignore_ascii_case(id)) {
                    return Err("textAs is not a text choice".into());
                }
                self.text_as = text_as_of(id);
            }
            "password" => self.password = text()?.to_string(),
            // As production's fields took it; OK checks the file against the caller's roots.
            "path" => self.path = Some(text()?.to_string()),
            _ => return Err(format!("no field {field}")),
        }
        Ok(())
    }
}

/// The Import PDF dialog on the host.
pub(super) struct ImportPdfDialog {
    id: DialogId,
    s: State,
    /// The preview shown for a (page, crop, text) revision; `None` for a page that could not be read, which is
    /// not asked for again.
    preview: Option<(Revision, Option<Arc<egui::ColorImage>>)>,
    /// Why the last password did not open the file.
    password_error: Option<String>,
    /// The OK the pump has not answered.
    held: HeldAnswer<State>,
}

impl ImportPdfDialog {
    fn new(s: State) -> Self {
        ImportPdfDialog { id: dialog_id(KIND), s, preview: None, password_error: None, held: HeldAnswer::new() }
    }

    /// The preview to render next: the revision of the shown page, crop and text, when the stored one differs.
    fn wanted_preview(&self) -> Option<Revision> {
        if self.s.locked {
            return None;
        }
        let revision = Revision::of(&(self.s.page(), self.s.crop.id(), self.s.preview_text_as().id()));
        (self.preview.as_ref().map(|(shown, _)| *shown) != Some(revision)).then_some(revision)
    }

    /// Stores the rendering of the preview `revision` (`None`: the page could not be read).
    fn set_preview(&mut self, revision: Revision, image: Option<Arc<egui::ColorImage>>) {
        self.preview = Some((revision, image));
    }

    /// The OK the pump has to judge, once.
    fn take_ok(&mut self) -> Option<State> {
        self.held.take()
    }

    /// The pump's answer to the held OK.
    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// OK: unlocks a locked file (and shows its pages when it has several), else holds the values for the pump.
    fn ok(&mut self) {
        if self.s.locked {
            match self.s.unlock() {
                Err(e) => {
                    self.password_error = Some(e);
                    return;
                }
                Ok(asks_pages) => {
                    self.password_error = None;
                    if asks_pages {
                        return;
                    }
                }
            }
        }
        self.held.file(self.s.clone());
    }

    fn show_locked(&mut self, ui: &mut egui::Ui) -> Option<DialogResult<()>> {
        let file_name = std::path::Path::new(&self.s.name).file_name().map_or(self.s.name.clone(), |f| f.to_string_lossy().into_owned());
        let mut dialog = PasswordDialog::new(PasswordPurpose::open(file_name));
        if let Some(error) = &self.password_error {
            dialog = dialog.error(Label::from(error.clone()));
        }
        let out = dialog.show(ui);
        match (out.answer, out.closed) {
            (Some(answer), _) => {
                self.s.password = answer.secret.expose().to_string();
                self.ok();
                None
            }
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn show_pages(&mut self, ui: &mut egui::Ui) -> Option<DialogResult<()>> {
        let title = if self.s.place.is_some() { tl!("Place PDF") } else { tl!("Import PDF") };
        let refusal = self.held.refusal().map(|(reason, _)| reason.clone());
        let preview = self.preview.as_ref().and_then(|(revision, image)| Some((*revision, image.clone()?)));
        let token = self.s.file.token;
        let waiting = self.held.is_waiting();
        let out =
            FormDialog::new(title, Bind::bound(&mut self.s)).ok_enabled(!waiting).accept_stays_open(true).alt_resets(false).show(ui, |ui, _, s| {
                pages_body(ui, s, preview.as_ref(), token, refusal.as_ref());
                true
            });
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.s) {
            self.held.clear_refusal();
        }
        if out.closed.is_some() {
            return Some(DialogResult::Cancel);
        }
        if out.picked == Some(0) && !waiting {
            self.ok();
        }
        None
    }
}

impl StackDialog for ImportPdfDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.held.finish().is_some() {
            return Some(DialogResult::Ok(()));
        }
        if self.s.locked { self.show_locked(ui) } else { self.show_pages(ui) }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        if self.held.finish().is_some() {
            return Some(());
        }
        if !self.held.is_waiting() {
            self.ok();
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        Some(self.s.to_json())
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        self.s.set_field(field, value)
    }
}

/// The pages form: the preview and page navigation on the left, the choices on the right.
fn pages_body(ui: &mut egui::Ui, s: &mut State, preview: Option<&(Revision, Arc<egui::ColorImage>)>, token: u64, refusal: Option<&Label>) {
    let count = s.page_count;
    let current = s.page();
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            if let Some((revision, image)) = preview {
                let image = || (**image).clone();
                Picture::new(tl!("Page preview"), token, &image).revision(*revision).extent(PictureSize::Thumb(ThumbSize::Large)).show(ui);
            }
            let mut to = None;
            Row::new().show(ui, |ui, _| {
                let first = nav(ui, "chevrons-left", tl!("First Page"), 1, current > 1);
                let previous = nav(ui, "chevron-left", tl!("Previous Page"), current.saturating_sub(1).max(1), current > 1);
                let typed = NumberField::new(tl!("Page"), Bind::given(Some(current as f64))).range(1.0..=count.max(1) as f64).show(ui);
                Text::new(crate::i18n::fmt(tl!("of {count}"), &[("count", &count.to_string())])).shade(Shade::Muted).show(ui);
                let next = nav(ui, "chevron-right", tl!("Next Page"), (current + 1).min(count), current < count);
                let last = nav(ui, "chevrons-right", tl!("Last Page"), count, current < count);
                to = first.or(previous).or(next).or(last).or(typed.value().copied().flatten().map(|v| v as usize));
            });
            if let Some(page) = to {
                s.page = page.clamp(1, count.max(1));
            }
        });
        Stack::new().show(ui, |ui, _| {
            let crops: Vec<Entry<CropTo>> = CropTo::ALL.iter().zip(CropTo::LABELS).map(|(crop, label)| Entry::new(*crop, *label)).collect();
            Field::new(tl!("Crop To")).show(ui, |ui, _| Select::new(tl!("Crop To"), Bind::bound(&mut s.crop), &crops).show(ui));
            let linked = s.places_linked();
            let texts = [Entry::new(TextAs::Text, tl!("Editable")), Entry::new(TextAs::Outlines, tl!("Outlines"))];
            Field::new(tl!("Text")).show(ui, |ui, _| Select::new(tl!("Text"), Bind::bound(&mut s.text_as), &texts).enabled(!linked).show(ui));
            // Outlines alone dims the box too.
            let open = s.place.is_none();
            if linked || (open && s.text_as == TextAs::Outlines && s.file_can_link()) {
                Text::new(tl!("Linked pages show the file's own text.")).shade(Shade::Muted).show(ui);
            }
            if open {
                pick_pages(ui, s, count, current);
            }
            if let Some(reason) = refusal {
                Text::new(reason.clone()).show(ui);
            }
        });
    });
}

/// One page navigation button; the page it goes to when pressed.
fn nav(ui: &mut egui::Ui, icon: &'static str, label: &'static str, to: usize, enabled: bool) -> Option<usize> {
    clicked(&IconButton::new(IconId::new(icon)).label(label).enabled(enabled).show(ui)).then_some(to)
}

/// The All / Range choice, the range field and Import PDF pages as links.
fn pick_pages(ui: &mut egui::Ui, s: &mut State, count: usize, current: usize) {
    let mut all = s.all_pages;
    RadioGroup::new(tl!("Pages"), Bind::bound(&mut all)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(true).label(tl!("All")).show(ui);
            Radio::new(false).label(tl!("Range:")).show(ui);
        });
    });
    if all != s.all_pages {
        s.pick_all_pages(all);
    }
    if let Some(pages) = PageRangeField::new(tl!("Range:"), &s.pages, count, current.saturating_sub(1)).show(ui).picked {
        s.pages = pages;
        s.pick_all_pages(false);
    }
    let on = s.links_on();
    let linked = Checkbox::new(Bind::given(on)).label(tl!("Import PDF pages as links")).enabled(s.can_link()).show(ui);
    if let Some(checked) = linked.value().copied() {
        s.link_pages = Some(checked);
    }
}

/// Open `bytes` (or place them with the `file.place` params `place`) through the dialog when the
/// file is a PDF (or `.ai`/`.ait`) with several pages or a password; false when there's nothing to
/// ask (the caller goes on), including, when opening, a file that carries its VectorCraft document
/// and a `.ai`/`.ait` with its editing data.
pub fn offer(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>) -> bool {
    offer_with(app, name, bytes, path, place, false)
}

/// [`offer`]; `password_only` asks only for the password (Place with Show Import Options off).
fn offer_with(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, place: Option<&Value>, password_only: bool) -> bool {
    let Some(format) = fileio::detect(name, bytes).map(|f| f.id).filter(|id| matches!(*id, "pdf" | "ai" | "ait")) else {
        return false;
    };
    // A file carrying its VectorCraft document (a saved .ai, or a PDF saved with Preserve Editing)
    // opens as that document, every artboard and layer: there are no pages to pick. So does a .ai
    // or .ait with its editing data, which holds them all (#6135); a PDF with editing data still
    // asks, as its pages are what it is. Placing still asks, since a place takes one page.
    // The file is read once for both checks.
    let opened = vectorcraft_pdf::open_pdf(bytes, None);
    if place.is_none() && opened.as_ref().is_ok_and(|o| o.editing().is_some() || (format != "pdf" && o.has_illustrator_data())) {
        return false;
    }
    let (pages, locked) = match opened.map(|o| o.page_count()) {
        Ok(count) if count > 1 => (count, false),
        Err(PdfError::NeedsPassword) => (0, true),
        _ => return false,
    };
    let Some(dialogs) = app.dialogs() else { return false };
    let mut state = State {
        place: None,
        name: name.to_string(),
        path,
        file: DialogFile::new(bytes.to_vec()),
        page_count: pages,
        page: 1,
        all_pages: true,
        pages: Vec::new(),
        link_pages: None,
        crop: CropTo::default(),
        // The PDF Text preference is for PDFs: a `.ai` or `.ait` file starts from Editable, as it
        // opens or is placed without the dialog.
        text_as: if format == "pdf" { preferred_text_as(app) } else { TextAs::Text },
        pdf: format == "pdf",
        password: String::new(),
        locked,
        password_only: password_only && place.is_some(),
        linkable: fileio::pages_can_link(name, bytes),
        text_as_given: false,
    };
    if let Some(p) = place {
        // A `textAs` the place gives starts the Text choice; one that isn't a choice stays with
        // the other params, for `file.place` to refuse.
        let given = p.get("textAs").and_then(Value::as_str).filter(|id| TextAs::IDS.iter().any(|i| i.eq_ignore_ascii_case(id)));
        if let Some(id) = given {
            state.text_as = text_as_of(id);
            state.text_as_given = true;
        }
        let taken: &[&str] = if given.is_some() { &["page", "crop", "password", "textAs"] } else { &["page", "crop", "password"] };
        let mut rest = other_place_params(p, taken);
        // Asked only for the password: a `page` and `crop` given are placed as given, for `file.place` to check.
        if password_only {
            for k in ["page", "crop"] {
                if let Some(v) = p.get(k) {
                    rest[k] = v.clone();
                }
            }
        }
        state.place = Some(rest);
    }
    dialogs.with(|host| host.open(ImportPdfDialog::new(state)));
    true
}

/// `file.place` with params `p` asks first (true) when they name a PDF (`.pdf`, `.ai`, `.ait`)
/// with several pages or a password and no `page`.
pub fn offer_place(app: &mut VectorcraftApp, p: &Value) -> bool {
    if p.get("page").is_some() {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| matches!(ext, "pdf" | "ai" | "ait")) else { return false };
    offer(app, &name, &bytes, path, Some(p))
}

/// Whether the `file.place` params `p` give a password. `file.place` reads an empty or null
/// `password` as none; a value that isn't text counts as given, for `file.place` to refuse.
fn gives_password(p: &Value) -> bool {
    p.get("password").is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
}

/// [`offer_place`] when Show Import Options is off: asks (true) only for the password of an
/// encrypted PDF given no password (see [`gives_password`]), which nothing else opens, and places
/// its first page (or `page`) and crop box once it is unlocked; any other PDF is placed that way
/// without asking.
pub fn offer_place_password(app: &mut VectorcraftApp, p: &Value) -> bool {
    // A `page` that isn't a page number is left for `file.place` to refuse.
    if gives_password(p) || p.get("page").is_some_and(|v| v.as_u64().is_none_or(|n| n == 0)) {
        return false;
    }
    let Some((name, bytes, path)) = place_source(app, p, |ext| matches!(ext, "pdf" | "ai" | "ait")) else { return false };
    if !matches!(vectorcraft_pdf::open_pdf(&bytes, None).map(|_| ()), Err(PdfError::NeedsPassword)) {
        return false;
    }
    offer_with(app, &name, &bytes, path, Some(p), true)
}

/// The `textAs` choice `id` (any case); anything else is editable text.
fn text_as_of(id: &str) -> TextAs {
    TextAs::ALL.iter().zip(TextAs::IDS).find(|(_, i)| i.eq_ignore_ascii_case(id)).map(|(t, _)| *t).unwrap_or_default()
}

/// What a PDF's text becomes when an open or place from the app does not say: the `pdfTextAs`
/// preference.
pub(crate) fn preferred_text_as(app: &VectorcraftApp) -> TextAs {
    text_as_of(&app.session.prefs.pdf_text_as)
}

/// The `document.open` options `p` for the file `name` holding `bytes`, with the `pdfTextAs`
/// preference as its `textAs` when it is a PDF opened as Outlines and `p` has no `textAs`. A `.ai`
/// or `.ait` file opens as it says: Outlines would open it from its PDF content, without its
/// editing data. So does an open with `linkPages`, which refuses Outlines: a linked page shows the
/// file's own text.
pub(crate) fn with_preferred_text_as(app: &VectorcraftApp, name: &str, bytes: &[u8], p: &Value) -> Value {
    let given = p.get("textAs").is_some_and(|v| !v.is_null());
    let linked = p.get("linkPages").and_then(Value::as_bool) == Some(true);
    let options = p.is_object() || p.is_null();
    if given || linked || !options || preferred_text_as(app) == TextAs::Text || fileio::detect(name, bytes).map(|f| f.id) != Some("pdf") {
        return p.clone();
    }
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    p["textAs"] = json!(TextAs::Outlines.id());
    p
}

/// The `file.place` params `p` of a PDF placed embedded (Link off, from bytes or with a password),
/// by the user or as a command, get the `pdfTextAs` preference as their `textAs` when it is Outlines and they have no
/// `textAs`. Bytes are a PDF by their content ([`places_pdf`]). A linked page shows the file's own
/// text, and a `.ai` or `.ait` file is placed as it says. An empty `password` is none, so a place
/// from a path with Link on and an empty `password` stays linked.
pub(crate) fn place_with_preferred_text_as(app: &VectorcraftApp, p: &mut Value) {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    let path = p.get("path").and_then(Value::as_str);
    let name = path.or_else(|| p.get("name").and_then(Value::as_str)).unwrap_or_default();
    let linked = path.is_some() && p.get("link").and_then(Value::as_bool) != Some(false) && !gives_password(p);
    if given("textAs") || linked || preferred_text_as(app) == TextAs::Text || !places_pdf(p, path, name) {
        return;
    }
    if let Some(o) = p.as_object_mut() {
        o.insert("textAs".into(), json!(TextAs::Outlines.id()));
    }
}

/// Do the `file.place` params `p` place a PDF (not a `.ai` or `.ait` file, which is PDF inside):
/// a `path` by its extension, as [`place_source`] picks one, and `dataBase64` by its content, as
/// `file.place` reads it ([`fileio::detect`]: bytes that start with `%PDF`), so bytes without a
/// `name` or with another extension count too. Only the first bytes are decoded.
fn places_pdf(p: &Value, path: Option<&str>, name: &str) -> bool {
    let ext = fileio::extension(name);
    match (path, p.get("dataBase64").and_then(Value::as_str)) {
        (None, Some(b64)) => {
            let head: String = b64.chars().filter(|c| !c.is_ascii_whitespace()).take(8).collect();
            !matches!(ext.as_str(), "ai" | "ait") && vectorcraft_format::base64_decode(&head).is_some_and(|b| b.starts_with(b"%PDF"))
        }
        _ => ext == "pdf",
    }
}

/// Keeps the Text choice OK confirmed in the `pdfTextAs` preference.
fn keep_text_as(app: &mut VectorcraftApp, text_as: TextAs) {
    if preferred_text_as(app) != text_as {
        // The open or place has already happened; a preference that can't be written only means
        // the next dialog starts from the old choice.
        let _ = app.run("prefs.set", json!({"key": "pdfTextAs", "value": text_as.id()}));
    }
}

/// OK: open the pages or place the page the held values name.
fn confirm(app: &mut VectorcraftApp, s: &State) -> Result<(), String> {
    let r = if let Some(place) = &s.place {
        let mut p = with_file(place, s.path.as_deref(), &s.name, &s.file);
        // Asked only for the password: a `page` and `crop` given with the file stand.
        let [page_given, crop_given] = ["page", "crop"].map(|k| s.password_only && p.get(k).is_some());
        if !page_given {
            p["page"] = json!(s.page());
        }
        if !crop_given {
            p["crop"] = json!(s.crop.id());
        }
        if let Some(pw) = s.password() {
            p["password"] = json!(pw);
        }
        // A `textAs` left with the other params is one `file.place` refuses; null leaves it out.
        if !s.places_linked() && p.get("textAs").is_none_or(Value::is_null) {
            p["textAs"] = json!(s.text_as.id());
        }
        crate::place::run_for_user(app, &p).map(|_| ())
    } else {
        let mut p = json!({"cropTo": s.crop.id(), "password": s.password(), "textAs": s.text_as.id()});
        // An empty range under Range opens every page.
        if !s.all_pages && !s.pages.is_empty() {
            p["pages"] = json!(format_pages(&s.pages));
        }
        if s.links_on() {
            p["linkPages"] = json!(true);
        }
        io::open_document(app, &s.name, &s.file.bytes, s.path.clone(), &p).map(|_| ())
    };
    // Asked only for the password, the dialog showed no Text choice to keep; the choice for a `.ai`
    // or `.ait` file is not kept for PDFs.
    if r.is_ok() && !s.places_linked() && !s.password_only && s.pdf {
        keep_text_as(app, s.text_as);
    }
    r
}

/// The preview of the page and boxes `s` shows; `None` for a page that can't be read.
fn preview_image(app: &mut VectorcraftApp, ctx: &egui::Context, s: &State) -> Option<Arc<egui::ColorImage>> {
    let opts = fileio::LoadOptions { crop: s.crop, password: s.password(), text_as: s.preview_text_as(), ..Default::default() };
    let (doc, _) = fileio::page_document(&s.file.bytes, s.page().saturating_sub(1), &opts).ok()?;
    render_preview(app, ctx, &doc)
}

/// The host pump: judges the OK the dialog holds (opening or placing the file; a refusal keeps the dialog open with
/// the reason) and renders the preview the dialog wants.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // PLAN: wg.A17's `open_held` / `HoldsOk` are not visible; the dialog's own `take_ok` / `answer_ok` through the
    // host stand in for them.
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(held) = dialogs.with(|host| host.get_mut::<ImportPdfDialog>().and_then(ImportPdfDialog::take_ok)) {
        let answer = confirm(app, &held).map_err(Label::from);
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut::<ImportPdfDialog>() {
                dialog.answer_ok(answer);
            }
        });
    }
    let wanted = dialogs.with(|host| host.get::<ImportPdfDialog>().and_then(|d| Some((d.wanted_preview()?, d.s.clone()))));
    if let Some((revision, s)) = wanted {
        let image = preview_image(app, ctx, &s);
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut::<ImportPdfDialog>() {
                dialog.set_preview(revision, image);
            }
        });
    }
}
