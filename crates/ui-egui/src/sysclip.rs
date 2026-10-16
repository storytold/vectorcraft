//! The system clipboard's formats: Copy and Cut publish every flavour the engine makes
//! ([`Session::clipboard_flavours`](vectorcraft_engine::Session::clipboard_flavours): text, SVG,
//! PDF, PNG), and a Paste first turns what another app copied (SVG, PDF, EPS, an EMF, text, a bitmap,
//! or a copied file that is one of them) into the internal clipboard with the `clipboard.import*`
//! commands. The host installs the platform side as
//! [`Services::system_clipboard`](crate::Services::system_clipboard); without it (the web) Copy
//! publishes SVG text through egui and Paste takes SVG text, and the pictures and files the host
//! reads from a paste itself ([`VectorcraftApp::paste_from_host`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;

use serde_json::{Value, json};
use vectorcraft_engine::cmd::clipboard::{BITMAP, EMF, EPS, Flavour, PASTE_ORDER, PDF, SVG, TEXT, looks_like_svg, names_a_picture};

use crate::VectorcraftApp;
use crate::background::{self, Writer};

/// The platform clipboard, with every format it carries.
pub trait SystemClipboard {
    /// Replace the clipboard's contents with `flavours` (best first; none empties it).
    fn write(&mut self, flavours: &[Flavour]) -> Result<(), String>;
    /// Does the clipboard still hold what [`Self::write`] last put there (nothing copied since)?
    fn holds_ours(&mut self) -> bool;
    /// The first of `mimes` the clipboard holds; `image/*` is any bitmap (returned as PNG, BMP…).
    fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour>;
    /// Does the clipboard hold one of `mimes`? Cheap enough to ask a few times a second.
    fn has(&mut self, mimes: &[&'static str]) -> bool;
}

/// Makes a second system-clipboard handle, to check off the UI thread whether Paste has something
/// to take ([`Services::clipboard_probe`](crate::Services::clipboard_probe)). It is called on that
/// thread and the handle lives and dies there, so the handle needn't be `Send`.
pub type ClipboardProbeFactory = Box<dyn FnOnce() -> Box<dyn SystemClipboard> + Send>;

/// The Copy or Cut whose formats go to the system clipboard ([`VectorcraftApp::publish_clipboard`]).
#[derive(Default)]
pub(crate) struct Publishing {
    /// The number of the latest Copy or Cut, raised by one when a copy through egui drops the
    /// formats not written yet. Only the formats of the Copy or Cut with this number are made and
    /// written.
    latest: Arc<AtomicU64>,
    /// The latest one's formats wait to be written.
    pending: bool,
}

/// The command (and its params) that loads `f` into the internal clipboard, centred on `center`.
pub(crate) fn import_command(f: &Flavour, center: Option<[f64; 2]>) -> (&'static str, Value) {
    let b64 = || vectorcraft_format::base64_encode(&f.data);
    match f.mime {
        TEXT => {
            let text = String::from_utf8_lossy(&f.data);
            if looks_like_svg(&text) {
                ("clipboard.importSvg", json!({ "svg": text, "center": center }))
            } else {
                ("clipboard.importText", json!({ "text": text, "center": center }))
            }
        }
        SVG => ("clipboard.importSvg", json!({ "dataBase64": b64(), "center": center })),
        PDF => ("clipboard.importPdf", json!({ "dataBase64": b64(), "center": center })),
        EPS => ("clipboard.importEps", json!({ "dataBase64": b64(), "center": center })),
        EMF => ("clipboard.importEmf", json!({ "dataBase64": b64(), "center": center })),
        mime => ("clipboard.importImage", json!({ "dataBase64": b64(), "mime": mime, "center": center })),
    }
}

impl VectorcraftApp {
    /// After Copy or Cut: offer the copied objects to other apps. With the system clipboard
    /// service, the background worker makes the formats from a copy of the clipboard
    /// ([`background::run`]) and they are written when they are ready. While the worker has a
    /// save, an export, a print or a recovery copy to finish, they are made and written at once
    /// instead. A later Copy or Cut, or a copy through egui
    /// ([`Self::drop_formats_after_egui_copy`]), drops the formats not written yet. While the
    /// latest Copy or Cut's formats wait to be written, Paste takes the internal clipboard, and
    /// while a text field or type has the keyboard the app waits for them
    /// ([`Self::write_formats_before_text_input`]).
    pub(crate) fn publish_clipboard(&mut self) {
        if self.services.system_clipboard.is_none() {
            if self.session.prefs.copy_as_svg {
                self.clipboard_out = self.session.clipboard_svg();
                self.clipboard_published = self.clipboard_out.clone();
            }
            return;
        }
        let (clip, svg, pdf) = (self.session.clipboard.clone(), self.session.prefs.copy_as_svg, self.session.prefs.copy_as_pdf);
        let latest = self.clipboard_publishing.latest.clone();
        let n = latest.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        self.clipboard_publishing.pending = true;
        let (tx, made) = channel();
        let work = move |_: Writer| {
            // These formats are not made when a later Copy or Cut, or a copy through egui, dropped them.
            if latest.load(Ordering::SeqCst) == n {
                // The receiver is gone only when the job was dropped unapplied.
                let _ = tx.send(clip.flavours(svg, pdf));
            }
            Ok(Value::Null)
        };
        let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
            let publishing = &mut app.clipboard_publishing;
            if publishing.latest.load(Ordering::SeqCst) != n {
                return Ok(Value::Null);
            }
            publishing.pending = false;
            let flavours = r.and_then(|_| made.try_recv().map_err(|e| e.to_string()));
            let written = flavours.and_then(|f| app.services.system_clipboard.as_mut().map_or(Ok(()), |cb| cb.write(&f)));
            if let Err(e) = written {
                app.ui.status = format!("Couldn't copy to the system clipboard: {e}");
            }
            Ok(Value::Null)
        };
        // Only the formats of other copies (jobs without a label) may be ahead of these on the worker.
        let free = self.background.jobs.iter().all(|j| j.label.is_empty());
        // No label: the status bar doesn't show the job, and the status stays as it is.
        if let Err(e) = background::run(self, free, String::new(), None, work, then) {
            self.ui.status = e;
        }
    }

    /// End of a frame: when something copied text or a picture through egui this frame (a text
    /// field, the Type tool), the formats of an earlier Copy or Cut that are not written yet are
    /// dropped, and the system clipboard keeps that copy.
    pub(crate) fn drop_formats_after_egui_copy(&mut self, ctx: &egui::Context) {
        let copied = ctx.output(|o| o.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_))));
        let publishing = &mut self.clipboard_publishing;
        if copied && publishing.pending {
            publishing.latest.fetch_add(1, Ordering::SeqCst);
            publishing.pending = false;
        }
    }

    /// Start of a frame, before the background results that arrived are applied: while a text
    /// field or type has the keyboard (egui's or the Type tool's), the formats a Copy or Cut is
    /// making are written first ([`Self::written_clipboard_text`]). A paste whose text the host read
    /// before that pastes their text instead, or nothing when they have none. A paste chord the
    /// host found no text for pastes their text, also when its key comes up after they were
    /// written, while the system clipboard still holds them. On the canvas the app does not wait
    /// for them: Paste takes the internal clipboard ([`Self::adopt_system_clipboard`]).
    pub(crate) fn write_formats_before_text_input(&mut self, ctx: &egui::Context) {
        if !(ctx.egui_wants_keyboard_input() || self.session.tool_wants_text()) {
            return;
        }
        // A paste chord egui sent no Paste event for: the clipboard held no text when it came.
        let textless = ctx.input(|i| self.paste_chord.clone().textless_paste(&i.events).is_some());
        if !self.clipboard_publishing.pending {
            // The formats the clipboard still holds were written after the chord's key went down.
            if textless
                && self.services.system_clipboard.as_mut().is_some_and(|cb| cb.holds_ours())
                && let Some(text) = self.system_clipboard_text()
            {
                ctx.input_mut(|i| i.events.insert(0, egui::Event::Paste(text)));
            }
            return;
        }
        let text = self.written_clipboard_text();
        ctx.input_mut(|i| {
            i.events.retain_mut(|e| match (e, &text) {
                (egui::Event::Paste(t), Some(text)) => {
                    t.clone_from(text);
                    true
                }
                (egui::Event::Paste(_), None) => false,
                _ => true,
            });
            if textless && let Some(text) = text {
                i.events.insert(0, egui::Event::Paste(text));
            }
        });
    }

    /// Paste `f`, which the host read from the system clipboard itself (a web page's paste carries
    /// pictures and files, egui's Paste event text only), as the paste chord `held` pastes:
    /// unless a field or the Type tool has the keyboard, as with the paste keys.
    pub fn paste_from_host(&mut self, ctx: &egui::Context, f: Flavour, held: egui::Modifiers) {
        if ctx.egui_wants_keyboard_input() || self.session.tool_wants_text() {
            return;
        }
        self.clipboard_in = Some(f);
        crate::menus::invoke(self, crate::shortcuts::paste_command(held), json!({}));
    }

    /// Before a paste: what another app put on the system clipboard replaces the internal
    /// clipboard (centred in the view). What we published ourselves keeps the lossless internal
    /// copy, and so does what we are still making the formats of. An error means nothing should be
    /// pasted.
    pub(crate) fn adopt_system_clipboard(&mut self) -> Result<(), String> {
        let pasted = self.clipboard_in.take();
        let center = self.view().map(|v| [v.center.x, v.center.y]);
        let (cmd, params) = match (pasted, self.services.system_clipboard.as_mut()) {
            // A picture or file the host read from the paste.
            (Some(f), _) if f.mime != TEXT => import_command(&f, center),
            (_, Some(cb)) => {
                if self.clipboard_publishing.pending || cb.holds_ours() {
                    return Ok(());
                }
                let Some(f) = cb.read(&PASTE_ORDER) else { return Ok(()) };
                // Text that only names a picture (its address, or its image markup) goes with a
                // browser's copied picture: the picture.
                let text = String::from_utf8_lossy(&f.data);
                // Blank text is what some hosts put beside a copied picture. It is not the paste.
                let f = if f.mime == TEXT && (text.trim().is_empty() || names_a_picture(&text)) { cb.read(&[BITMAP]).unwrap_or(f) } else { f };
                import_command(&f, center)
            }
            // Text only: SVG markup is art, other text is left alone.
            (pasted, None) => {
                let text =
                    pasted.map(|f| String::from_utf8_lossy(&f.data).into_owned()).or_else(|| self.services.clipboard_read.as_mut().and_then(|f| f()));
                let Some(text) = text.filter(|t| looks_like_svg(t)) else { return Ok(()) };
                if self.clipboard_published.as_deref() == Some(text.as_str()) {
                    return Ok(());
                }
                self.clipboard_published = Some(text.clone());
                ("clipboard.importSvg", json!({ "svg": text, "center": center }))
            }
        };
        self.session.execute(cmd, &params).map(drop).map_err(|e| {
            self.clipboard_published = None;
            format!("Couldn't paste from the clipboard: {e}")
        })
    }

    /// The text on the system clipboard once the formats a Copy or Cut is making are written, for a
    /// paste into a text field or type.
    pub(crate) fn written_clipboard_text(&mut self) -> Option<String> {
        // Only other copies' formats are ahead of them on the worker ([`Self::publish_clipboard`]).
        background::wait_until(self, |app| !app.clipboard_publishing.pending);
        self.system_clipboard_text()
    }

    /// The text on the system clipboard, if any.
    pub(crate) fn system_clipboard_text(&mut self) -> Option<String> {
        match self.services.system_clipboard.as_mut() {
            Some(cb) => cb.read(&[TEXT]).map(|f| String::from_utf8_lossy(&f.data).into_owned()),
            None => self.services.clipboard_read.as_mut().and_then(|read| read()),
        }
    }

    /// Does the system clipboard hold something Paste can take (with nothing copied in the app)?
    pub(crate) fn system_clipboard_pasteable(&mut self) -> bool {
        match self.services.system_clipboard.as_mut() {
            Some(cb) => cb.has(&PASTE_ORDER),
            None => self.services.clipboard_read.as_mut().and_then(|read| read()).is_some_and(|t| looks_like_svg(&t)),
        }
    }
}
