//! VectorCraft's half of Open and Save against cloud storage.
//!
//! The pages themselves are `craft_ui::cloud`. Every Craft app offers the same ones. What is here
//! is what only VectorCraft can answer. A document is a drawing. These extensions and these
//! content types are what that means, and both come from the engine's own format table
//! ([`vectorcraft_engine::cmd::fileio::FORMATS`]). Bytes become a drawing through the same
//! `document.open` File › Open uses, and go back through the same encoders `document.export` uses.
//! The words are the ones [`crate::i18n`] has.
//!
//! VectorCraft holds no provider credential and speaks no provider protocol. Every call goes to
//! the CollabCraft service, which is the single place a provider is configured and permissions are
//! decided. Replacing one storage provider with another changes nothing here.
//!
//! Nothing in this file opens a socket either. The transport half is a [`CloudBackend`] the host
//! application supplies (see `apps/vectorcraft/src/cloud.rs`). Every service call blocks. A
//! blocking read of a forty megabyte drawing inside a frame would freeze the window for seconds.
//! So the pages state a [`Job`], the host runs it on a worker thread, and the answer arrives on a
//! channel that [`pump`] drains on a later frame.

use std::collections::HashMap;
use std::sync::Arc;

// Re-exported so a host application, and a test, can name these types without depending on the
// crates they come from.
pub use craft_cloud::{Capability, Entry, EntryKind};
pub use craft_ui::cloud::{
    Account, Busy, CloudApp, CloudBackend, CloudFiles, Conflict, CopyFormat, Destination, DocumentKind, Event, Fault, Folder, Job, Link, Origin,
    Post, Source, UNAVAILABLE, available, browse, can_converse, conflict_modal, destination_name, has_target, link, load_shared, open_entry,
    open_page, origin, parent_of, prefers_cloud, progress, pump, reload_active, save, save_as, save_page, share_button, shares, sign_in, sign_out,
    start, who, work_together,
};
use egui::{Color32, FontFamily, Painter, Rect, Ui};
use serde_json::{Value as Json, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::fileio::{self, Format};

use crate::VectorcraftApp;
use crate::theme::{FONT_UI_SEMIBOLD, Tokens};

/// Content types VectorCraft opens, matched against the start of what the provider said.
///
/// Read beside the extension because neither is reliable alone. A WebDAV server hands back
/// `application/octet-stream` for a perfectly good `.svg`, and a file exported by a web
/// application often has a correct content type and no extension at all. `image/svg+xml` is the
/// registered type for SVG, compressed or not, and the one a provider that knows the format sends.
///
/// Every entry is the `mime` of a row of [`fileio::FORMATS`] that VectorCraft reads, in that
/// table's order and without repeats. `application/json` is the one readable type left out, and
/// `the_content_types_are_the_readable_half_of_the_format_table` holds this list to exactly that.
/// A native drawing and a native template are JSON, so the table says so, but their own
/// extensions are in [`fileio::OPEN_EXTS`] and a name already offers them. Reading the type too
/// would offer every JSON file in somebody's storage as a drawing, and every one of them would
/// fail to open.
const MIMES: [&str; 15] = [
    "image/svg+xml",
    "application/pdf",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/bmp",
    "image/vnd.dxf",
    "application/postscript",
    "image/emf",
    "image/wmf",
    "image/x-psd",
    "application/vnd.affinity",
    "image/x-psb",
];

/// The readable content type this list leaves out, and the reason, in one place so the test and
/// the comment above cannot drift apart.
const MIME_LEFT_OUT: &str = "application/json";

/// What a VectorCraft document is, as a cloud file list needs to know.
///
/// The extensions are the engine's own [`fileio::OPEN_EXTS`], which is what File › Open reads and
/// what the desktop file picker offers. A second list here would drift from that one.
pub const DRAWING: DocumentKind =
    DocumentKind { extensions: fileio::OPEN_EXTS, mimes: &MIMES, noun: "drawing", default_extension: vectorcraft_format::EXTENSION };

/// The drawing snapshot each save in flight encoded.
///
/// Only VectorCraft knows what a version of a drawing is. The pages hold none of these. A save
/// that lands clears the modified flag for exactly the version that reached the server. An edit
/// made while the bytes were on the wire stays unsaved.
pub type Sent = HashMap<u64, Arc<Document>>;

/// Whether VectorCraft can open this entry.
pub fn opens(entry: &Entry) -> bool {
    craft_ui::cloud::opens(entry, DRAWING)
}

/// A typed name as a drawing file name. Any folder separators are dropped and a missing extension
/// becomes `.vectorcraft`.
pub fn drawing_name(typed: &str) -> String {
    craft_ui::cloud::document_name(typed, DRAWING)
}

/// The whole of the cloud state, for the control channel and for tests.
pub fn status(app: &VectorcraftApp) -> Json {
    craft_ui::cloud::report(app)
}

/// The Save a Copy formats the Share menu offers under the share itself.
///
/// The formats a drawing can become a file of its own as ([`fileio::SAVE_FORMATS`]), which is the
/// set File › Save a Copy offers. Writing a raster image is an export, which is a different
/// gesture with a dialog of its own.
pub fn copies() -> Vec<CopyFormat> {
    fileio::SAVE_FORMATS
        .iter()
        .filter_map(|id| fileio::format(id))
        .map(|f| CopyFormat::new(f.id, format!("{} (.{})…", f.label, f.extensions.first().copied().unwrap_or_default())))
        .collect()
}

/// Where the CollabCraft service is, as the environment names it.
///
/// Read from the environment and compiled in nowhere, because the service is deployment-specific
/// and a desktop binary must not carry one. The host application reads the same list, so the badge
/// names the deployment the worker actually reaches.
pub fn base_url() -> Option<String> {
    ["VECTORCRAFT_CLOUD_URL", "CRAFT_CLOUD_URL", "COLLABCRAFT_URL"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .find(|value| !value.is_empty())
}

/// The host part of that address, for the account menu's first line.
///
/// Somebody with an account on two deployments needs to know which one a drawing is about to land
/// in.
fn service_host() -> String {
    base_url().map(|value| value.trim_start_matches("https://").trim_start_matches("http://").to_owned()).unwrap_or_default()
}

/// The format a save should be written in, for a destination the provider named.
///
/// The name is the first answer: its extension is what the file claims to be, and it is what the
/// user typed when they typed one. A provider that issued no extension at all still says what the
/// thing is, so the content type is the second. Both cases are common, and a file exported by a
/// web application usually has a correct content type and a bare name.
///
/// A destination this application cannot write is refused by name, with the extensions it can
/// write. Encoding a drawing as `.vectorcraft` under the name `poster.eps` would hand somebody a
/// file nothing opens, and nothing would tell them. A refusal a person can act on is better.
fn format_for(into: Destination<'_>) -> Result<&'static Format, String> {
    let extension = into.extension();
    if !extension.is_empty() {
        return match fileio::format_for_name(into.name) {
            Some(f) if f.write => Ok(f),
            Some(f) => Err(cannot_write(&format!("VectorCraft reads {} files but cannot write one", f.label))),
            None => Err(cannot_write(&format!("VectorCraft does not write a .{extension}"))),
        };
    }
    let mime = into.mime.ok_or_else(|| cannot_write("that name carries no extension"))?;
    let mime = mime.split(';').next().unwrap_or(mime).trim().to_ascii_lowercase();
    // The first writable row with that type. The table's order is the answer a bare name wants:
    // `application/pdf` is a PDF before it is a PDF-compatible `.ai`, and `image/svg+xml` is a
    // plain SVG before it is a compressed one.
    fileio::FORMATS.iter().find(|f| f.write && f.mime == mime).ok_or_else(|| cannot_write(&format!("VectorCraft does not write {mime}")))
}

/// A refusal that says what went wrong and which extensions are available instead.
fn cannot_write(why: &str) -> String {
    let writable: Vec<String> = fileio::export_extensions().iter().map(|e| format!(".{e}")).collect();
    format!("{why}. VectorCraft writes {}", writable.join(", "))
}

impl CloudApp for VectorcraftApp {
    fn cloud(&self) -> &CloudFiles {
        &self.cloud
    }

    fn cloud_mut(&mut self) -> &mut CloudFiles {
        &mut self.cloud
    }

    fn kind(&self) -> DocumentKind {
        DRAWING
    }

    fn active(&self) -> Option<u64> {
        self.session.active().map(|d| d.uid)
    }

    fn title(&self, uid: u64) -> String {
        self.session.documents().iter().find(|d| d.uid == uid).map(|d| d.title()).unwrap_or_default()
    }

    fn open_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<u64>, String> {
        // The ordinary open path, with no file on this computer behind it: a cloud drawing holds a
        // provider handle instead of a path, and Save goes back through that handle.
        let answer = crate::io::open_bytes(self, name, &bytes, None)?;
        // A PDF of several pages asks which to take, and a DXF asks about units. Those answer with
        // a dialog rather than a document, and nothing is said about the bytes yet.
        if answer.is_null() {
            return Ok(None);
        }
        Ok(self.session.active().map(|d| d.uid))
    }

    fn encode(&mut self, uid: u64, into: Destination<'_>) -> Result<Vec<u8>, String> {
        let format = format_for(into)?;
        let doc = self.session.documents().iter().find(|d| d.uid == uid).ok_or("no drawing open")?;
        let snapshot = doc.doc.clone();
        // The options this drawing was last saved with, where it is going back out as the format
        // it already is. One opened from the cloud as an SVG keeps its SVG Options; one going out
        // as something else takes that format's own defaults.
        let options = if doc.format == format.id { Json::Object(doc.save_options.clone()) } else { json!({}) };
        // A native file compresses as Use Compression says, as every other save of one does.
        let options = fileio::with_compression_pref(&self.session.prefs, &options);
        let bytes = fileio::encode(&snapshot, format.id, &options).map_err(|e| e.to_string())?;
        self.cloud_sent.insert(uid, snapshot);
        Ok(bytes)
    }

    fn close(&mut self, uid: u64) {
        if let Some(index) = self.session.documents().iter().position(|d| d.uid == uid) {
            self.session.close_document(index);
        }
    }

    fn mark_saved(&mut self, uid: u64) {
        let Some(snapshot) = self.cloud_sent.remove(&uid) else { return };
        if let Some(doc) = self.session.document_mut(uid) {
            doc.mark_saved_as(&snapshot);
        }
    }

    fn drop_pending(&mut self, uid: u64) {
        self.cloud_sent.remove(&uid);
    }

    fn ready_to_save(&mut self) -> bool {
        // A native file keeps the view it was saved with, and the user may have moved it since.
        // The desktop Save does the same before it writes.
        crate::io::remember_view(self);
        true
    }

    fn sharing(&mut self) -> &mut craft_ui::share::ShareDialog {
        &mut self.share
    }

    fn presence(&mut self) -> &mut craft_ui::presence::PresenceBar {
        &mut self.presence
    }

    fn paint_icon(&self, painter: &Painter, rect: Rect, kind: EntryKind, colour: Color32) {
        let icon = if kind == EntryKind::Folder { "dc-folder" } else { "file-plus" };
        crate::icons::paint_with(painter, icon, rect, colour);
    }

    fn open_url(&self, url: &str) {
        // A shared borrow is all a paint call has, and the host's own opener is `FnMut`. So the URL
        // waits in the queue the frame drains on its way out ([`VectorcraftApp::open_queued_urls`]).
        self.queue_url(url);
    }

    fn say(&mut self, text: String) {
        // Something the user has to see: the dialog a failed open already uses. The text arrives
        // translated, because the pages compose their sentences and pass them through `tr`.
        crate::dialogs::confirm::tell(self, &text, "");
    }

    fn toast(&mut self, text: String) {
        // The status bar, which is where this application says what just happened. The line is
        // already in the interface language, so `i18n::msg` leaves it alone when it is drawn.
        self.ui.status = text;
    }

    fn go_to_save_as(&mut self) {
        crate::dialogs::cloud::open(self, true);
    }

    fn app_name(&self) -> &str {
        "VectorCraft"
    }

    fn tr(&self, text: &str) -> String {
        crate::i18n::t(text).to_owned()
    }

    fn service(&self) -> String {
        service_host()
    }

    fn save_note(&self) -> Option<String> {
        Some(self.tr("A name with no extension is saved as a VectorCraft drawing (.vectorcraft)."))
    }

    fn style(&self, ui: &Ui) -> craft_ui::cloud::CloudStyle {
        let t = Tokens::get(ui.ctx());
        craft_ui::cloud::CloudStyle {
            surface: t.panel,
            border: t.border,
            text: t.text,
            dim: t.text_dim,
            hover: t.hover,
            soft: t.row_selected,
            family: FontFamily::Proportional,
            bold: FontFamily::Name(FONT_UI_SEMIBOLD.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extensions_are_the_ones_file_open_reads() {
        assert_eq!(DRAWING.extensions, fileio::OPEN_EXTS, "one list of readable extensions, and it is the engine's");
        assert!(DRAWING.extensions.contains(&"svg"), "vector art is SVG before it is anything else");
        assert!(DRAWING.extensions.contains(&vectorcraft_format::EXTENSION));
        assert!(DRAWING.extensions.contains(&vectorcraft_format::LEGACY_EXTENSION), "files from before the rename still open");
    }

    #[test]
    fn the_content_types_are_the_readable_half_of_the_format_table() {
        let mut want: Vec<&str> = Vec::new();
        for f in fileio::FORMATS.iter().filter(|f| f.read) {
            if f.mime != MIME_LEFT_OUT && !want.contains(&f.mime) {
                want.push(f.mime);
            }
        }
        assert_eq!(MIMES.to_vec(), want, "every content type a readable format declares, bar the one named above");
        assert!(MIMES.contains(&"image/svg+xml"), "the registered type for SVG, which is what a provider sends");
        assert!(!MIMES.contains(&MIME_LEFT_OUT), "or every JSON file would be offered as a drawing");
        assert!(fileio::FORMATS.iter().any(|f| f.read && f.mime == MIME_LEFT_OUT), "the type left out is a real readable one");
    }

    #[test]
    fn a_name_decides_the_format_and_a_bare_name_asks_the_content_type() {
        let id = |name: &str, mime: Option<&str>| format_for(Destination { name, mime }).map(|f| f.id);
        assert_eq!(id("Poster.svg", None), Ok("svg"));
        assert_eq!(id("Poster.SVGZ", None), Ok("svgz"), "an extension in any case");
        assert_eq!(id("Poster.vectorcraft", None), Ok("vectorcraft"));
        assert_eq!(id("Poster.drawcraft", None), Ok("vectorcraft"), "the former extension still names the native format");
        assert_eq!(id("Poster.pdf", None), Ok("pdf"));
        // The name wins over the type, because the name is what somebody will double-click.
        assert_eq!(id("Poster.svg", Some("application/pdf")), Ok("svg"));
        // No extension at all, and a type that says what the thing is.
        assert_eq!(id("exported", Some("image/svg+xml")), Ok("svg"));
        assert_eq!(id("exported", Some("image/svg+xml; charset=utf-8")), Ok("svg"), "a type with a parameter on it");
        assert_eq!(id("exported", Some("application/pdf")), Ok("pdf"), "a PDF before a PDF-compatible .ai");
    }

    #[test]
    fn a_destination_this_app_cannot_write_is_refused_by_name() {
        let refused = |name: &str, mime: Option<&str>| format_for(Destination { name, mime }).err().unwrap_or_default();
        // PSB is read and never written.
        let psb = refused("Art.psb", None);
        assert!(psb.contains("cannot write one"), "{psb}");
        assert!(psb.contains(".svg") && psb.contains(".pdf"), "and it names what is available instead: {psb}");
        let unknown = refused("Art.wibble", None);
        assert!(unknown.contains(".wibble"), "{unknown}");
        // A bare name with nothing to go on cannot be guessed at.
        let bare = refused("exported", None);
        assert!(bare.contains("no extension"), "{bare}");
        let foreign = refused("exported", Some("application/octet-stream"));
        assert!(foreign.contains("application/octet-stream"), "{foreign}");
    }

    #[test]
    fn a_typed_name_becomes_a_drawing_name() {
        assert_eq!(drawing_name(" Poster "), "Poster.vectorcraft");
        assert_eq!(drawing_name("Poster.svg"), "Poster.svg");
        assert_eq!(drawing_name("Poster.v2"), "Poster.v2.vectorcraft", "an unknown suffix is part of the name");
        assert_eq!(drawing_name("/a/b/Poster"), "Poster.vectorcraft", "a pasted path cannot become a folder");
        assert_eq!(drawing_name("   "), "");
    }

    #[test]
    fn the_share_menu_offers_the_formats_a_drawing_can_be_saved_as() {
        let copies = copies();
        let ids: Vec<&str> = copies.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, fileio::SAVE_FORMATS, "the same set File › Save a Copy offers, in the same order");
        assert!(copies.iter().all(|c| c.label.contains('.')), "each label names the extension it writes");
    }
}
