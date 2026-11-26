//! The system clipboard's formats: Copy and Cut publish every flavour the engine makes
//! ([`Session::clipboard_flavours`](vectorcraft_engine::Session::clipboard_flavours): text, SVG,
//! PDF, PNG), and a Paste first turns what another app copied (SVG, PDF, EPS, an EMF, text, a bitmap,
//! or a copied file that is one of them) into the internal clipboard with the `clipboard.import*`
//! commands. Both go through [`VectorcraftApp::clipboard`]. Where the host can read its clipboard only
//! inside a paste event (the web) Copy publishes SVG text and Paste takes SVG text, and the pictures
//! and files the host reads from a paste itself ([`VectorcraftApp::paste_from_host`]).

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::channel;

// PLAN: crates/ui-egui/Cargo.toml needs craft-platform with its `clipboard` feature (hub-w1).
use craft::platform::clipboard::{Clipboard, Flavor, RGBA8, Written};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::clipboard::{
    AI, BITMAP, EMF, EPS, Flavour, ImageRead, PASTE_ORDER, PDF, PNG, SVG, TEXT, looks_like_svg, names_a_picture,
};

use crate::VectorcraftApp;
use crate::background::{self, Writer};

/// What the system clipboard's text was when the formats of a Copy or Cut were about to be written
/// or could not be: a later copy in another app changes it.
type Held = Option<Vec<u8>>;

/// Makes a flavour off the UI thread; `None` when it can't be made.
pub(crate) type MakeFlavour = Box<dyn FnOnce() -> Option<Flavour> + Send>;

/// The text on `cb`; nothing when it holds none or can't be read.
fn held_text(cb: &mut Clipboard) -> Held {
    cb.read(&[TEXT]).ok().flatten().map(|f| f.data)
}

/// The platform clipboard's flavour for an engine one.
fn platform_flavor(f: Flavour) -> Flavor {
    Flavor::new(f.mime, f.data)
}

/// The engine's flavour for one the platform clipboard read. A raw bitmap ([`RGBA8`]) is encoded as
/// PNG, which `clipboard.importImage` reads; a bitmap a backend already encoded keeps its own
/// `image/*` type. A type the engine has no name for is dropped.
pub(crate) fn engine_flavour(f: Flavor) -> Option<Flavour> {
    if f.mime == RGBA8 {
        return rgba8_png(&f.data);
    }
    let own = [TEXT, SVG, PDF, EPS, EMF, AI, PNG].into_iter().find(|m| *m == f.mime);
    let mime = own.or(match f.mime {
        Cow::Borrowed(m) if m.starts_with("image/") => Some(m),
        _ => None,
    })?;
    Some(Flavour { mime, data: f.data })
}

/// A raw bitmap (width and height as little-endian `u32`s, then straight RGBA8 pixels) as a PNG flavour.
fn rgba8_png(data: &[u8]) -> Option<Flavour> {
    let (header, pixels) = data.split_first_chunk::<8>()?;
    let [w0, w1, w2, w3, h0, h1, h2, h3] = *header;
    let (width, height) = (u32::from_le_bytes([w0, w1, w2, w3]), u32::from_le_bytes([h0, h1, h2, h3]));
    let img = image::RgbaImage::from_raw(width, height, pixels.to_vec())?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(Flavour { mime: PNG, data: png })
}

/// Text that only names a picture (its address, or its image markup), or is blank (some hosts put
/// it beside a copied picture), is not the paste: the picture beside it is.
fn stands_in_for_a_picture(text: &str) -> bool {
    text.trim().is_empty() || names_a_picture(text)
}

/// Whether `f` is text that stands in for a picture ([`stands_in_for_a_picture`]).
fn is_picture_stand_in(f: &Flavour) -> bool {
    f.mime == TEXT && stands_in_for_a_picture(&String::from_utf8_lossy(&f.data))
}

/// The Copy or Cut whose formats go to the system clipboard ([`VectorcraftApp::publish_clipboard`]).
#[derive(Default)]
pub(crate) struct Publishing {
    /// The number of the latest Copy or Cut, raised by one when a copy through egui drops the
    /// formats not written yet. Only the formats of the Copy or Cut with this number are made and
    /// written.
    latest: Arc<AtomicU64>,
    /// The latest one's formats wait to be written.
    pending: bool,
    /// The latest one's formats could not be written: what the system clipboard's text was when the
    /// write failed, and the text of those formats.
    unwritten: Option<(Held, String)>,
    /// The status bar's message when the latest one ran, with [`crate::state::UiState::status_sets`]
    /// then. Its formats' notes replace the message only while the status bar still shows it and
    /// nothing has set the status since.
    status: (String, u64),
    /// The status bar's message the latest Copy or Cut with notes left, with
    /// [`crate::state::UiState::status_sets`] then.
    notes_status: Option<(String, u64)>,
}

impl Publishing {
    /// Whether the latest Copy or Cut still has formats waiting to be written to the system clipboard.
    pub(crate) fn is_pending(&self) -> bool {
        self.pending
    }

    /// The number of the latest Copy or Cut: it changes with each Copy or Cut, and with each copy
    /// through egui that drops the formats not written yet.
    pub(crate) fn latest(&self) -> u64 {
        self.latest.load(Ordering::SeqCst)
    }

    /// `ui`'s status bar message is the one the latest Copy or Cut left, which its formats' notes
    /// may replace.
    pub(crate) fn ran_with_status(&mut self, ui: &crate::state::UiState) {
        ui.status.clone_into(&mut self.status.0);
        self.status.1 = ui.status_sets;
    }

    /// Does `ui`'s status bar still show the message the latest Copy or Cut left, with nothing
    /// setting the status since?
    fn shows_status(&self, ui: &crate::state::UiState) -> bool {
        ui.status == self.status.0 && ui.status_sets == self.status.1
    }
}

/// The status bar's line after a Copy (or with `cut`, a Cut) whose formats have `n` notes, the
/// first of them `first`.
pub(crate) fn copied_with_notes(cut: bool, n: usize, first: &str) -> String {
    let (one, other) = if cut {
        ("Cut with {n} note: {first}", "Cut with {n} notes: {first}")
    } else {
        ("Copied with {n} note: {first}", "Copied with {n} notes: {first}")
    };
    crate::i18n::msg_n(n as u64, one, other, &[("first", first)])
}

/// Show a Copy's (or with `cut`, a Cut's) formats' `notes` in the status bar: the first of them
/// and how many there are, with the rest for the message's hover. Without notes, the message an
/// earlier Copy's or Cut's notes left is cleared while nothing has set the status since, and any
/// other message stays.
pub(crate) fn show_copy_notes(app: &mut VectorcraftApp, cut: bool, notes: Vec<String>) {
    let left = app.clipboard_publishing.notes_status.take();
    let Some(first) = notes.first() else {
        crate::io::clear_notes_left(app, left);
        return;
    };
    let status = copied_with_notes(cut, notes.len(), first);
    crate::io::show_with_notes(app, status, notes);
    app.clipboard_publishing.notes_status = Some((app.ui.status.clone(), app.ui.status_sets));
}

/// Does a flavour of type `mime` load with `clipboard.importImage` ([`import_command`])?
pub(crate) fn imports_as_image(mime: &str) -> bool {
    ![TEXT, SVG, PDF, AI, EPS, EMF].contains(&mime)
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
        AI => ("clipboard.importPdf", json!({ "dataBase64": b64(), "editingData": true, "center": center })),
        EPS => ("clipboard.importEps", json!({ "dataBase64": b64(), "center": center })),
        EMF => ("clipboard.importEmf", json!({ "dataBase64": b64(), "center": center })),
        mime => ("clipboard.importImage", json!({ "dataBase64": b64(), "mime": mime, "center": center })),
    }
}

impl VectorcraftApp {
    /// A Type paste may reuse its saved styled runs only while our copy still owns the clipboard.
    /// Pending formats also belong to that copy; a host that reads its clipboard only inside a paste
    /// event has no way to tell, so its copy is taken as ours.
    pub(crate) fn type_paste_has_our_clipboard(&mut self) -> bool {
        self.clipboard_publishing.pending
            || self.unwritten_clipboard_text().is_some()
            || !self.clipboard.readable_any_time()
            || self.clipboard.holds_ours()
    }

    /// The text of the latest Copy or Cut whose formats could not be written to the system
    /// clipboard, while nothing has been copied since that Copy or Cut. A paste into type or a text
    /// field takes this text until another app copies something. A copy is noticed only when it
    /// changes the clipboard's text, so the copy is kept only when the clipboard held text when the
    /// write failed.
    pub(crate) fn unwritten_clipboard_text(&mut self) -> Option<String> {
        let (held, text) = self.clipboard_publishing.unwritten.as_ref()?;
        (held_text(&mut self.clipboard) == *held).then(|| text.clone())
    }

    /// After Copy or Cut: offer the copied objects to other apps. The background worker makes the
    /// formats from a copy of the clipboard ([`background::run`]) and they are written when they are
    /// ready. While the worker has a save, an export, a print or a recovery copy to finish, they are
    /// made and written at once instead. A later Copy or Cut, or a copy through egui
    /// ([`Self::drop_formats_after_egui_copy`]), drops the formats not written yet. While the
    /// latest Copy or Cut's formats wait to be written, Paste takes the internal clipboard, and
    /// while a text field or type has the keyboard the app waits for them
    /// ([`Self::write_formats_before_text_input`]). Once they are written, while the status bar
    /// still shows the message the Copy or Cut (`cut`) left ([`Publishing::ran_with_status`]), it
    /// shows the first of the PNG's and PDF's notes and how many there are; hovering the message
    /// lists them ([`crate::io::show_with_notes`]).
    pub(crate) fn publish_clipboard(&mut self, cut: bool) {
        // Ruler guides copied alone stay in VectorCraft, but the Copy still replaces what the
        // system clipboard held: it is emptied, so the next Paste takes the guides rather than
        // what another app copied before, and what another app copies after still wins (#3226).
        if !self.session.clipboard.has_art() {
            self.clear_system_clipboard();
            // Guides have no formats, so this Copy or Cut has no notes.
            show_copy_notes(self, cut, vec![]);
            return;
        }
        if !self.clipboard.readable_any_time() {
            if self.session.prefs.copy_as_svg
                && let Some(svg) = self.session.clipboard_svg()
            {
                // A failed or fenced write leaves the clipboard as it was, and there is no worker to tell.
                let _ = self.clipboard.write_text(&svg);
            }
            return;
        }
        // An SVG that would name a linked image's file outside the read roots of the automation
        // that asked for the Copy is left out (decided now: the formats are made later).
        let svg = self.session.prefs.copy_as_svg && !self.session.clipboard_svg_refused();
        let (clip, pdf) = (self.session.clipboard.clone(), self.session.prefs.copy_as_pdf);
        let latest = self.clipboard_publishing.latest.clone();
        let n = latest.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        self.clipboard_publishing.pending = true;
        self.clipboard_publishing.unwritten = None;
        self.clipboard_publishing.ran_with_status(&self.ui);
        // What the clipboard holds at the Copy or Cut: a copy made elsewhere before the write is newer.
        let before = held_text(&mut self.clipboard);
        let (tx, made) = channel();
        let work = move |_: Writer| {
            // These formats are not made when a later Copy or Cut, or a copy through egui, dropped them.
            if latest.load(Ordering::SeqCst) == n {
                let (formats, warnings) = clip.flavours_with_warnings(svg, pdf);
                // The receiver is gone only when the job was dropped unapplied.
                let _ = tx.send((formats.into_iter().map(platform_flavor).collect::<Vec<_>>(), warnings));
            }
            Ok(Value::Null)
        };
        let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
            if app.clipboard_publishing.latest.load(Ordering::SeqCst) != n {
                return Ok(Value::Null);
            }
            app.clipboard_publishing.pending = false;
            let written = r
                .and_then(|_| made.try_recv().map_err(|e| e.to_string()))
                .and_then(|(formats, warnings)| app.write_formats(&formats, &before).map(|w| (w, warnings)));
            match written {
                Err(e) => app.ui.status = format!("Couldn't copy to the system clipboard: {e}"),
                // Input injected by automation never replaces what the user copied: nothing was copied.
                Ok((Written::Fenced, _)) => {}
                // The PNG's and PDF's notes (an image that couldn't be decoded, type in the
                // fallback font), unless another message has replaced the Copy or Cut's since.
                Ok((Written::Written, warnings)) => {
                    if app.clipboard_publishing.shows_status(&app.ui) {
                        show_copy_notes(app, cut, warnings);
                    }
                }
            }
            Ok(Value::Null)
        };
        // Only the formats of other copies (jobs without a label) may be ahead of these on the worker.
        let free = self.background.jobs.iter().all(|j| j.label.is_empty());
        // No label: the status bar doesn't show the job, and only a failure or a warning changes
        // the status.
        if let Err(e) = background::run(self, free, String::new(), None, work, then) {
            self.ui.status = e;
        }
    }

    /// Writes `formats` to the system clipboard; `before` is the clipboard's text at the Copy or Cut.
    /// When that fails, another app may hold the clipboard open: it keeps what it held (or what the
    /// failed write left), and a paste into text takes this copy's text until something else is
    /// copied ([`Self::unwritten_clipboard_text`]).
    fn write_formats(&mut self, formats: &[Flavor], before: &Held) -> Result<Written, String> {
        let cb = &mut self.clipboard;
        let before_write = held_text(cb);
        // Nothing copied elsewhere since the Copy or Cut.
        let current = *before == before_write;
        let e = match cb.write(formats) {
            Ok(written) => return Ok(written),
            Err(e) => e,
        };
        let held = held_text(cb);
        let text = formats.iter().find(|t| t.mime == TEXT).map(|t| String::from_utf8_lossy(&t.data).into_owned());
        // A change during the write is the failed write emptying the clipboard and writing some of
        // the formats, or another app's copy: the clipboard then holds nothing to paste, this copy's
        // text, or no text and one of this copy's formats only after the former.
        let left_by_write = held == before_write
            || match cb.read(&[TEXT]).ok().flatten() {
                Some(now) => text.as_deref().is_some_and(|ours| now.data == ours.as_bytes()),
                None => {
                    !cb.has(&PASTE_ORDER)
                        || formats.iter().any(|ours| cb.read(&[&*ours.mime]).ok().flatten().is_some_and(|now| now.data == ours.data))
                }
            };
        // A clipboard with no text can't show a later copy of a picture, or tell an empty clipboard
        // from one that couldn't be read: unless it held text the write emptied, and it reads.
        let trackable = held.is_some() || (before_write.is_some() && cb.read(&[TEXT]).is_ok());
        self.clipboard_publishing.unwritten = text.filter(|_| current && left_by_write && trackable).map(|text| (held, text));
        Err(e.to_string())
    }

    /// A Copy with nothing for other apps (ruler guides alone): the formats of an earlier Copy or
    /// Cut not written yet are dropped and the system clipboard is emptied. Where the host reads its
    /// clipboard only inside a paste event, empty text goes out when Copy As SVG is on, in place of
    /// the SVG objects would put there.
    fn clear_system_clipboard(&mut self) {
        let publishing = &mut self.clipboard_publishing;
        publishing.latest.fetch_add(1, Ordering::SeqCst);
        publishing.pending = false;
        publishing.unwritten = None;
        if self.clipboard.readable_any_time() {
            if let Err(e) = self.clipboard.write(&[]) {
                self.ui.status = format!("Couldn't copy to the system clipboard: {e}");
            }
        } else if self.session.prefs.copy_as_svg {
            // As for a Copy of art: there is nothing to tell if this fails.
            let _ = self.clipboard.write_text("");
        }
    }

    /// Hand a queued text copy to the system clipboard. Called after `tick`, after the UI phase's
    /// input and at the end of the pass; the later calls are what an input- or menu-driven fallback
    /// Copy needs, and `take` keeps it single-shot. A Copy or Cut that labels left to the next pass
    /// hands its text over as soon as it runs ([`crate::shortcuts::run_left_by_labels`]).
    pub(crate) fn flush_clipboard_out(&mut self) {
        if let Some(text) = self.clipboard_out.take() {
            // Something else is being copied: the formats of an earlier Copy not written yet would replace it.
            self.drop_unwritten_formats();
            // A fenced or failed write loses the text: nothing else carries it.
            if let Ok(Written::Written) = self.clipboard.write_text(&text) {
                // Text copied here is not the art in the internal clipboard: a paste takes it as another app's copy.
                self.clipboard.forget_ours();
            }
        }
        // Without a host clipboard, what was copied goes out through egui.
        if let Some(text) = self.fallback_copy.lock().ok().and_then(|mut slot| slot.take())
            && let Some(ctx) = &self.ui_ctx
        {
            ctx.copy_text(text);
        }
    }

    /// End of a frame: when something copied text or a picture through egui this frame (a text
    /// field, the Type tool), the formats of an earlier Copy or Cut that are not written yet are
    /// dropped, and the system clipboard keeps that copy.
    pub(crate) fn drop_formats_after_egui_copy(&mut self, ctx: &egui::Context) {
        let copied = ctx.output(|o| o.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_))));
        if copied {
            self.drop_unwritten_formats();
        }
    }

    /// Drop the formats of an earlier Copy or Cut that are not written yet: something else is
    /// being copied (text through egui, the text selected in a label).
    pub(crate) fn drop_unwritten_formats(&mut self) {
        let publishing = &mut self.clipboard_publishing;
        publishing.unwritten = None;
        if publishing.pending {
            publishing.latest.fetch_add(1, Ordering::SeqCst);
            publishing.pending = false;
        }
    }

    /// Start of a frame, before the background results that arrived are applied: while a text
    /// field or type has the keyboard (egui's or the Type tool's), the formats a Copy or Cut is making
    /// are written first ([`Self::written_clipboard_text`]). A paste whose text the host read before
    /// that pastes their text instead, or nothing when they have none. A paste the host found no
    /// text for pastes their text, while the system clipboard still holds them. On the canvas the app does not wait
    /// for them: Paste takes the internal clipboard ([`Self::adopt_system_clipboard`]).
    pub(crate) fn write_formats_before_text_input(&mut self, ctx: &egui::Context) {
        if !self.pastes_into_text(ctx) {
            return;
        }
        // A paste with no text: the clipboard held none when the chord came.
        let textless = craft::ui::runtime::keys::textless_paste(ctx);
        // Formats that waited when this host iteration began still own a paste event read while
        // they were pending, even if the worker wrote them during this iteration's tick.
        let waited = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("clipboard.pending-at-tick"))).unwrap_or(false);
        ctx.data_mut(|d| d.remove::<bool>(egui::Id::unique("clipboard.pending-at-tick")));
        if !(self.clipboard_publishing.pending || waited) {
            // A copy whose formats could not be written: a paste takes its text, not the text the
            // clipboard kept ([`Self::unwritten_clipboard_text`]).
            let pasting = textless || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
            if pasting && let Some(text) = self.unwritten_clipboard_text() {
                ctx.input_mut(|i| {
                    for e in &mut i.events {
                        if let egui::Event::Paste(t) = e {
                            t.clone_from(&text);
                        }
                    }
                    if textless {
                        i.events.insert(0, egui::Event::Paste(text));
                    }
                });
                return;
            }
            // The formats the clipboard still holds were written after the chord's key went down.
            if textless
                && self.clipboard.holds_ours()
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

    /// Whether a paste goes into text: a text field or the Type tool has the keyboard. Keyboard focus on the canvas or on
    /// a label's selected text (which takes Copy but no paste) is not text input.
    fn pastes_into_text(&self, ctx: &egui::Context) -> bool {
        self.session.tool_wants_text()
            || (ctx.egui_wants_keyboard_input()
                && craft::ui::runtime::focus::focused_rec(ctx).is_none_or(|rec| rec.stop.keys.edits.contains(craft::behavior::EditCommand::Paste)))
    }

    /// Paste `f`, which the host read from the system clipboard itself (a web page's paste carries
    /// pictures and files, egui's Paste event text only), as the paste chord `held` pastes:
    /// unless a field or the Type tool has the keyboard, as with the paste keys.
    pub fn paste_from_host(&mut self, ctx: &egui::Context, f: Flavour, held: egui::Modifiers) {
        if self.pastes_into_text(ctx) {
            return;
        }
        self.clipboard_in = Some(f);
        // The paste command the V chord held belongs to (Paste in Place for Cmd+Shift+V by default), else Paste.
        let command = crate::keymap::chord_of_press(egui::Key::V, held, crate::keymap::platform())
            .and_then(|chord| craft::ui::runtime::commands::holder(ctx, &chord))
            .map(|c| c.id)
            .filter(|id| id.starts_with("edit.paste"));
        crate::menus::invoke(self, command.as_deref().unwrap_or("edit.paste"), json!({}));
    }

    /// Before a paste: what another app put on the system clipboard replaces the internal
    /// clipboard (centred in the view). What we published ourselves keeps the lossless internal
    /// copy, and so does what we are still making the formats of. An error means nothing should be
    /// pasted.
    pub(crate) fn adopt_system_clipboard(&mut self) -> Result<(), String> {
        self.adopt_system_clipboard_or_defer(false).map(drop)
    }

    /// [`Self::adopt_system_clipboard`]. With `defer`, what the system clipboard holds is not loaded
    /// here: the work that makes its flavour comes back (a bitmap's decoding and the wait for the
    /// app that copied it run in it, [`Clipboard::read_later`]; text that names the picture is the
    /// flavour when the picture can't be made), and the internal clipboard is left as it is.
    pub(crate) fn adopt_system_clipboard_or_defer(&mut self, defer: bool) -> Result<Option<MakeFlavour>, String> {
        let pasted = self.clipboard_in.take();
        let center = self.view().map(|v| [v.center.x, v.center.y]);
        let (cmd, params) = match pasted {
            Some(f) if defer && imports_as_image(f.mime) => return Ok(Some(Box::new(move || Some(f)))),
            // A picture or file the host read from the paste.
            Some(f) if f.mime != TEXT => import_command(&f, center),
            _ if self.clipboard.readable_any_time() => {
                if self.clipboard_publishing.pending || self.clipboard.holds_ours() {
                    return Ok(None);
                }
                // Art and text load at once, so a paste that waits behind another takes them as it was given; only a
                // picture is read off the UI thread.
                let read = if defer {
                    let at_once: Vec<&str> = PASTE_ORDER.into_iter().filter(|m| !imports_as_image(m)).collect();
                    self.clipboard.read(&at_once).ok().flatten().and_then(engine_flavour).filter(|f| !is_picture_stand_in(f))
                } else {
                    self.read_system_flavour()
                };
                let Some(f) = read else { return Ok(if defer { self.read_system_flavour_later() } else { None }) };
                import_command(&f, center)
            }
            // The host's text only: SVG markup is art, other text is left alone.
            pasted => {
                let text = match pasted {
                    Some(f) => Some(String::from_utf8_lossy(&f.data).into_owned()),
                    None => self.system_clipboard_text(),
                };
                let Some(text) = text.filter(|t| looks_like_svg(t)) else { return Ok(None) };
                // Our own copy comes back as the lossless internal clipboard.
                if self.clipboard.holds_ours() {
                    return Ok(None);
                }
                ("clipboard.importSvg", json!({ "svg": text, "center": center }))
            }
        };
        self.session.execute(cmd, &params).map(|_| None).map_err(|e| format!("Couldn't paste from the clipboard: {e}"))
    }

    /// The first of the paste order the system clipboard holds, a picture in place of text that only
    /// names it ([`stands_in_for_a_picture`]).
    fn read_system_flavour(&mut self) -> Option<Flavour> {
        let f = self.clipboard.read(&PASTE_ORDER).ok().flatten().and_then(engine_flavour)?;
        if !is_picture_stand_in(&f) {
            return Some(f);
        }
        // The text, when the picture can't be read.
        Some(self.clipboard.read(&[BITMAP]).ok().flatten().and_then(engine_flavour).unwrap_or(f))
    }

    /// [`Self::read_system_flavour`] as the work that finishes the read off the UI thread. Only this
    /// thread may ask the clipboard, so the picture that text standing in for it gives way to is asked
    /// for now, and the work picks between the two.
    fn read_system_flavour_later(&mut self) -> Option<MakeFlavour> {
        let first = self.clipboard.read_later(&PASTE_ORDER).ok().flatten()?;
        let picture = self
            .clipboard
            .read_text()
            .ok()
            .flatten()
            .filter(|t| stands_in_for_a_picture(t))
            .and_then(|_| self.clipboard.read_later(&[BITMAP]).ok().flatten());
        Some(Box::new(move || {
            let f = engine_flavour(first()?)?;
            match picture {
                Some(picture) if is_picture_stand_in(&f) => picture().and_then(engine_flavour).or(Some(f)),
                _ => Some(f),
            }
        }))
    }

    /// Before a paste whose flavour was made off the UI thread ([`crate::placing::adopt_for_paste`]):
    /// `f` replaces the internal clipboard, centered on `center`, as
    /// [`Self::adopt_system_clipboard`] loads it. An error means nothing should be pasted.
    pub(crate) fn adopt_flavour(&mut self, f: &Flavour, center: Option<[f64; 2]>) -> Result<(), String> {
        let (cmd, params) = import_command(f, center);
        self.session.execute(cmd, &params).map(drop).map_err(|e| format!("Couldn't paste from the clipboard: {e}"))
    }

    /// [`Self::adopt_flavour`] for a bitmap of type `mime` decoded off the UI thread: the image `read`
    /// replaces the internal clipboard, centered on `center`.
    pub(crate) fn adopt_image_read(&mut self, mime: &str, center: Option<[f64; 2]>, read: Result<ImageRead, String>) -> Result<(), String> {
        let params = json!({ "mime": mime, "center": center });
        read.and_then(|read| self.session.import_image_read(read, &params).map(drop).map_err(|e| e.to_string()))
            .map_err(|e| format!("Couldn't paste from the clipboard: {e}"))
    }

    /// The text on the system clipboard once the formats a Copy or Cut is making are written, for a
    /// paste into a text field or type. When they could not be written, their text
    /// ([`Self::unwritten_clipboard_text`]).
    pub(crate) fn written_clipboard_text(&mut self) -> Option<String> {
        // Only other copies' formats are ahead of them on the worker ([`Self::publish_clipboard`]).
        background::wait_until(self, |app| !app.clipboard_publishing.pending);
        self.unwritten_clipboard_text().or_else(|| self.system_clipboard_text())
    }

    /// The text on the system clipboard, if any.
    pub(crate) fn system_clipboard_text(&mut self) -> Option<String> {
        self.clipboard.read(&[TEXT]).ok().flatten().map(|f| String::from_utf8_lossy(&f.data).into_owned())
    }

    /// Does the system clipboard hold something Paste can take (with nothing copied in the app)?
    /// Where the host reads its clipboard only inside a paste event, only SVG text is something to take.
    pub(crate) fn system_clipboard_pasteable(&mut self) -> bool {
        if self.clipboard.readable_any_time() {
            self.clipboard.has(&PASTE_ORDER)
        } else {
            self.system_clipboard_text().is_some_and(|t| looks_like_svg(&t))
        }
    }
}
