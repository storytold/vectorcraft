//! The system clipboard with every format Copy offers and Paste reads
//! (`Services::system_clipboard`).
//!
//! - Windows: text, SVG, PDF, PNG and an opaque bitmap (for apps that don't read PNG) in one go;
//!   Paste reads each of them and EPS, a bitmap as PNG, else as the device-independent bitmap that
//!   screenshots and most apps copy (`CF_DIBV5`, `CF_DIB`; the system makes them of a `CF_BITMAP`).
//! - macOS and Linux: one format at a time, the text (a type-only copy's text or the SVG markup),
//!   else the PNG as a bitmap; Paste reads text and bitmaps, and on macOS the pasteboard's SVG, PDF
//!   and EPS as vector art (#1499) and its PNG or TIFF as they are, as screenshots copy them (on
//!   Linux arboard reads no other formats). On Wayland, arboard's data-control
//!   backend reads the compositor's clipboard (X11 through XWayland only sees what X11 apps
//!   copied), falling back to X11 where the compositor lacks the protocol, and where there is no
//!   X11 either (a Flatpak on GNOME) to the compositor's core protocol on the window's connection,
//!   text only. A clipboard that can't be opened is tried again after a wait that grows to a
//!   minute, not each time Paste asks. On Linux and the BSDs a read waits 2 s at most for the app
//!   that copied to send its data (a bitmap 10 s), and until that app answers, later reads fail at
//!   once; a paste then waits for that read's answer off the UI thread.
//! - Everywhere: files copied in a file manager paste as the first one that is art (SVG, PDF, EPS,
//!   a metafile or a bitmap), before the clipboard's other formats (the files' paths as text).

use std::io::{Cursor, Read};
use std::path::PathBuf;

use vectorcraft_engine::cmd::clipboard::{BITMAP, EPS, FILE_HEAD, Flavour, PDF, PNG, SVG, TEXT, file_flavour};
use vectorcraft_ui_egui::{ClipboardRead, SystemClipboard};

/// This platform's system clipboard. On Linux, `window`'s Wayland connection carries text where
/// arboard can't open (GNOME has no data-control protocol, a Flatpak there no X11).
pub fn system_clipboard(window: Option<std::sync::Arc<winit::window::Window>>) -> Box<dyn SystemClipboard> {
    #[cfg(windows)]
    let cb = {
        let _ = window;
        Box::<win::Clipboard>::default()
    };
    #[cfg(not(windows))]
    let cb = Box::new(portable::Clipboard::new(window));
    cb
}

/// The registered Windows clipboard formats a flavour is written under (and read from, first held
/// first).
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_formats(mime: &str) -> &'static [&'static str] {
    match mime {
        SVG => &["image/svg+xml"],
        PDF => &["Portable Document Format", "application/pdf"],
        EPS => &["Encapsulated PostScript", "application/postscript"],
        PNG => &["PNG"],
        _ => &[],
    }
}

/// The macOS pasteboard types Paste reads for `mime` (`image/*`: any bitmap), best first, each with
/// the MIME type of its data. They are the system's uniform type identifiers (the values of
/// `NSPasteboardTypePDF`, `NSPasteboardTypePNG` and `NSPasteboardTypeTIFF` among them) and the
/// older name some apps still give PDF. Text is read through arboard.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn pasteboard_types(mime: &str) -> &'static [(&'static str, &'static str)] {
    match mime {
        SVG => &[("public.svg-image", SVG)],
        PDF => &[("com.adobe.pdf", PDF), ("Apple PDF pasteboard type", PDF)], // brand-ok: the system's type identifier
        EPS => &[("com.adobe.encapsulated-postscript", EPS)],                 // brand-ok: the system's type identifier
        BITMAP => &[("public.png", PNG), ("public.tiff", "image/tiff")],
        _ => &[],
    }
}

/// The text flavour among `flavours`.
fn text_of(flavours: &[Flavour]) -> Option<String> {
    flavours.iter().find(|f| f.mime == TEXT).map(|f| String::from_utf8_lossy(&f.data).into_owned())
}

/// Most pixels a side of a bitmap another app put on the clipboard (untrusted data).
const MAX_SIDE: u32 = 32_768;

fn decode_png(png: &[u8]) -> Option<image::RgbaImage> {
    Some(image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8())
}

/// `img` as a PNG flavour.
fn png_flavour(img: &image::RgbaImage) -> Option<Flavour> {
    let mut data = vec![];
    img.write_to(&mut Cursor::new(&mut data), image::ImageFormat::Png).ok()?;
    Some(Flavour { mime: PNG, data })
}

/// A Windows device-independent bitmap (`CF_DIB`, `CF_DIBV5`: a BITMAPINFO header, the colour
/// masks of a BITMAPINFOHEADER, the colour table, then the pixels) as a BMP file, whose header
/// says where the pixels start. `None` when the header is cut short or out of range.
#[cfg_attr(not(windows), allow(dead_code))]
fn dib_file(dib: &[u8]) -> Option<Vec<u8>> {
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;
    let u16_at = |i: usize| Some(u16::from_le_bytes(dib.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let header = u32_at(0)?;
    // BITMAPCOREHEADER has 16-bit fields and 3-byte colours; the others 32-bit fields, 4-byte
    // colours and a count of them (0: all a palette of the depth has), the 40-byte one its masks
    // after it.
    let (bits, used, entry, masks) = match header {
        12 => (u16_at(10)?, 0, 3, 0),
        _ => {
            let masks = match (header, u32_at(16)?) {
                (40, BI_BITFIELDS) => 12,
                (40, BI_ALPHABITFIELDS) => 16,
                _ => 0,
            };
            (u16_at(14)?, u32_at(32)?, 4, masks)
        }
    };
    let colours = match used {
        0 if bits <= 8 => 1 << bits,
        n if n <= 256 => n,
        _ => return None,
    };
    let offset = [header, masks, colours * entry].into_iter().try_fold(14u32, u32::checked_add)?;
    let size = u32::try_from(dib.len()).ok()?.checked_add(14)?;
    if offset > size {
        return None;
    }
    let mut bmp = Vec::with_capacity(dib.len() + 14);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// A device-independent bitmap as an image, at most [`MAX_SIDE`] pixels a side. It is opaque when
/// its alpha is zero throughout: screenshots and most apps leave the fourth byte of a 32-bit pixel
/// at zero, even under an alpha mask.
#[cfg_attr(not(windows), allow(dead_code))]
fn dib_image(dib: &[u8]) -> Option<image::RgbaImage> {
    let mut reader = image::ImageReader::with_format(Cursor::new(dib_file(dib)?), image::ImageFormat::Bmp);
    let mut limits = image::Limits::default();
    (limits.max_image_width, limits.max_image_height) = (Some(MAX_SIDE), Some(MAX_SIDE));
    reader.limits(limits);
    let mut img = reader.decode().ok()?.into_rgba8();
    if img.pixels().all(|p| p.0[3] == 0) {
        img.pixels_mut().for_each(|p| p.0[3] = u8::MAX);
    }
    Some(img)
}

/// The first of `paths` (files copied in a file manager) that pastes as one of `mimes`, read
/// whole. Only its first bytes are read to tell, so other files cost little.
fn copied_file(paths: Vec<PathBuf>, mimes: &[&'static str]) -> Option<Flavour> {
    paths.into_iter().find_map(|path| {
        // text/uri-list lines end in CRLF (RFC 2483), but arboard splits them on LF only.
        let path = match path.to_str() {
            Some(p) if p.ends_with('\r') => PathBuf::from(p.trim_end_matches('\r')),
            _ => path,
        };
        // A paste sent by automation reads only what its roots allow.
        match path.to_str() {
            Some(p) => vectorcraft_engine::file_access::check_read(p).ok()?,
            // A lossy name would check another file than the one opened.
            None if vectorcraft_engine::file_access::current().is_none() => {}
            None => return None,
        }
        // A device or pipe may never end: only a regular file is read whole.
        if !std::fs::metadata(&path).ok()?.is_file() {
            return None;
        }
        let mut head = Vec::with_capacity(FILE_HEAD);
        std::fs::File::open(&path).ok()?.take(FILE_HEAD as u64).read_to_end(&mut head).ok()?;
        let mime = file_flavour(&path.to_string_lossy(), &head, mimes)?;
        let data = std::fs::read(&path).ok()?;
        (!data.is_empty()).then_some(Flavour { mime, data })
    })
}

#[cfg(windows)]
mod win {
    use std::num::NonZeroU32;

    use clipboard_win::{formats, options::NoClear, raw};

    use super::{
        BITMAP, ClipboardRead, Cursor, Flavour, PNG, SVG, SystemClipboard, TEXT, copied_file, decode_png, dib_image, png_flavour, windows_formats,
    };

    /// The device-independent bitmaps, the one that carries alpha first. Each is there whenever the
    /// other or a `CF_BITMAP` is: the system converts between them.
    const DIBS: [u32; 2] = [formats::CF_DIBV5, formats::CF_DIB];

    fn formats_of(mime: &str) -> impl Iterator<Item = u32> {
        windows_formats(mime).iter().filter_map(|n| raw::register_format(n)).map(NonZeroU32::get)
    }

    fn err(e: impl std::fmt::Display) -> String {
        format!("{e}")
    }

    /// `png` over white, as a BMP file (what the bitmap format takes).
    fn opaque_bmp(png: &[u8]) -> Option<Vec<u8>> {
        let img = decode_png(png)?;
        let over = |c: u8, a: u8| ((u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8;
        let mut rgb = Vec::with_capacity(img.as_raw().len() / 4 * 3);
        for p in img.pixels() {
            let [r, g, b, a] = p.0;
            rgb.extend([over(r, a), over(g, a), over(b, a)]);
        }
        let rgb = image::RgbImage::from_raw(img.width(), img.height(), rgb)?;
        let mut bmp = vec![];
        rgb.write_to(&mut Cursor::new(&mut bmp), image::ImageFormat::Bmp).ok()?;
        Some(bmp)
    }

    #[derive(Default)]
    pub struct Clipboard {
        /// The clipboard's sequence number right after our last write.
        seq: Option<NonZeroU32>,
    }

    impl SystemClipboard for Clipboard {
        /// Writes every flavour it can; the first failure is reported.
        fn write(&mut self, flavours: &[Flavour]) -> Result<(), String> {
            let mut result = Ok(());
            {
                let _open = clipboard_win::Clipboard::new_attempts(10).map_err(err)?;
                raw::empty().map_err(err)?;
                let mut set = |r: clipboard_win::SysResult<()>| {
                    if result.is_ok() {
                        result = r.map_err(err);
                    }
                };
                for f in flavours {
                    if f.mime == TEXT {
                        set(raw::set_string_with(&String::from_utf8_lossy(&f.data), NoClear));
                        continue;
                    }
                    for id in formats_of(f.mime) {
                        set(raw::set_without_clear(id, &f.data));
                    }
                    if f.mime == PNG
                        && let Some(bmp) = opaque_bmp(&f.data)
                    {
                        set(raw::set_bitmap_with(&bmp, NoClear));
                    }
                }
            }
            self.seq = raw::seq_num();
            result
        }

        fn holds_ours(&mut self) -> bool {
            // The same text copied later by another app is a different clipboard entry. A
            // clipboard manager that republishes our data is also a new entry.
            self.seq.is_some() && raw::seq_num() == self.seq
        }

        fn change_count(&mut self) -> Option<u64> {
            raw::seq_num().map(|n| u64::from(n.get()))
        }

        fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
            self.read_deferred(mimes)?.flavour()
        }

        /// A device-independent bitmap is copied off the clipboard here and decoded and encoded
        /// as PNG by the work that comes back.
        fn read_deferred(&mut self, mimes: &[&'static str]) -> Option<ClipboardRead> {
            let mut files = vec![];
            {
                let _open = clipboard_win::Clipboard::new_attempts(10).ok()?;
                // None copied when the clipboard holds no file list.
                let _ = raw::get_file_list_path(&mut files);
            }
            // The files are read with the clipboard closed again, not keeping other apps waiting.
            copied_file(files, mimes).map(ClipboardRead::Now).or_else(|| {
                let _open = clipboard_win::Clipboard::new_attempts(10).ok()?;
                mimes.iter().find_map(|m| read_one(m))
            })
        }

        fn has(&mut self, mimes: &[&'static str]) -> bool {
            raw::is_format_avail(formats::CF_HDROP)
                || mimes.iter().any(|m| match *m {
                    TEXT => raw::is_format_avail(formats::CF_UNICODETEXT),
                    BITMAP => DIBS.into_iter().chain(formats_of(PNG)).any(raw::is_format_avail),
                    m => formats_of(m).any(raw::is_format_avail),
                })
        }
    }

    /// Read the first registered format of `mime` the clipboard holds into `data`.
    fn read_registered(mime: &str, data: &mut Vec<u8>) -> bool {
        formats_of(mime).filter(|id| raw::is_format_avail(*id)).any(|id| {
            data.clear();
            raw::get_vec(id, data).is_ok() && !data.is_empty()
        })
    }

    /// A device-independent bitmap as a PNG flavour.
    fn dib_png(dib: &[u8]) -> Option<Flavour> {
        png_flavour(&dib_image(dib)?)
    }

    /// Copy both bitmap formats while the clipboard is open, then decode them in preference order.
    pub(super) fn read_dibs(mut get: impl FnMut(u32) -> Option<Vec<u8>>) -> Option<ClipboardRead> {
        let dibs: Vec<_> = DIBS.into_iter().filter_map(&mut get).collect();
        (!dibs.is_empty()).then_some(ClipboardRead::Later(Box::new(move || dibs.iter().find_map(|dib| dib_png(dib)))))
    }

    /// One flavour, while the clipboard is open. A device-independent bitmap is copied here, and
    /// the work that comes back decodes each captured format in preference order.
    fn read_one(mime: &'static str) -> Option<ClipboardRead> {
        let mut data = vec![];
        let mime = match mime {
            BITMAP if read_registered(PNG, &mut data) => PNG,
            BITMAP => {
                return read_dibs(|id| {
                    let mut dib = vec![];
                    raw::get_vec(id, &mut dib).ok()?;
                    Some(dib)
                });
            }
            TEXT => {
                raw::get_string(&mut data).ok()?;
                TEXT
            }
            m if read_registered(m, &mut data) => m,
            _ => return None,
        };
        // Memory blocks can be longer than the data: SVG text ends at its first NUL.
        if mime == SVG
            && let Some(end) = data.iter().position(|b| *b == 0)
        {
            data.truncate(end);
        }
        (!data.is_empty()).then_some(ClipboardRead::Now(Flavour { mime, data }))
    }
}

/// Reads made on a worker thread and waited for a bounded time: a clipboard owner that never sends
/// its data holds the worker, not the caller. On Linux and the BSDs arboard's reads go through one,
/// and on Linux the Wayland core protocol's reads through another. Built everywhere so it is
/// tested everywhere.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")),
    allow(dead_code)
)]
mod loads {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError, Weak, mpsc};
    use std::time::{Duration, Instant};

    /// The longest a read waits for the clipboard's owner to send its data.
    pub(super) const LOAD_WAIT: Duration = Duration::from_secs(2);
    /// What a read says once its worker has stopped.
    pub(super) const GONE: &str = "the clipboard's reader stopped";

    /// Requests of type `R` answered with `A` on a worker thread.
    pub(super) struct Loads<R, A> {
        /// The worker's requests; the worker stops once this is dropped.
        line: Arc<Line<R, A>>,
        /// Disconnected once the worker has let go of what its `load` holds.
        done: mpsc::Receiver<()>,
        /// How long after it was sent dropping `Loads` waits for an unanswered read to end.
        finish: Duration,
    }

    /// Where requests go to a [`Loads`] worker.
    struct Line<R, A> {
        /// Each request goes with the channel its answer is sent on.
        requests: mpsc::Sender<(R, mpsc::Sender<A>)>,
        /// Requests sent and not answered yet, counted down by the worker: while one is open,
        /// reads don't queue behind it.
        open: Arc<AtomicUsize>,
        /// When the last request was sent, and whether the `Loads` has stopped taking requests:
        /// requests are sent with this locked.
        sent: Mutex<Sent>,
    }

    #[derive(Default)]
    struct Sent {
        at: Option<Instant>,
        closed: bool,
    }

    impl<R, A> Line<R, A> {
        /// A line whose worker has stopped.
        fn dead() -> Arc<Self> {
            Arc::new(Self { requests: mpsc::channel().0, open: Arc::default(), sent: Mutex::default() })
        }

        /// `request` sent to the worker ([`Loads::ask`]).
        fn ask(&self, request: R) -> Result<mpsc::Receiver<A>, String> {
            let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
            if sent.closed {
                return Err(GONE.into());
            }
            if self.open.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                return Err("the clipboard's owner hasn't answered an earlier read".into());
            }
            let (reply, answer) = mpsc::channel();
            sent.at = Some(Instant::now());
            if self.requests.send((request, reply)).is_err() {
                self.open.fetch_sub(1, Ordering::SeqCst);
                return Err(GONE.into());
            }
            Ok(answer)
        }

        /// Stops taking requests → when the one still unanswered was sent.
        fn close(&self) -> Option<Instant> {
            let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
            sent.closed = true;
            sent.at.filter(|_| self.open.load(Ordering::SeqCst) > 0)
        }
    }

    /// Sends requests to a [`Loads`] worker from another thread while the `Loads` is open.
    pub(super) struct Asker<R, A>(Weak<Line<R, A>>);

    impl<R, A> Asker<R, A> {
        /// [`Loads::ask`]; Err also once the `Loads` has been dropped.
        pub(super) fn ask(&self, request: R) -> Result<mpsc::Receiver<A>, String> {
            self.0.upgrade().ok_or_else(|| GONE.to_string())?.ask(request)
        }
    }

    /// When dropping `Loads` stops waiting for its worker: `finish` after the unanswered read
    /// `pending` was sent (at once if that time has passed), and with no read unanswered
    /// [`LOAD_WAIT`] from `now` at most (an idle worker ends at once).
    fn stop_by(pending: Option<Instant>, now: Instant, finish: Duration) -> Instant {
        pending.map_or(now + LOAD_WAIT, |sent| sent + finish)
    }

    impl<R: Send + 'static, A: Send + 'static> Loads<R, A> {
        /// A worker thread named `name` that answers each request with `load`; dropping the result
        /// waits up to `finish` after an unanswered read was sent for that read to end. None when the
        /// worker can't start.
        pub(super) fn spawn(name: &str, finish: Duration, mut load: impl FnMut(R) -> A + Send + 'static) -> Option<Self> {
            let (requests, asked) = mpsc::channel::<(R, mpsc::Sender<A>)>();
            let (alive, done) = mpsc::channel::<()>();
            let open = Arc::new(AtomicUsize::new(0));
            let answered = open.clone();
            let work = move || {
                while let Ok((request, reply)) = asked.recv() {
                    let answer = load(request);
                    // Counted down before the answer goes, so its reader can ask again at once.
                    answered.fetch_sub(1, Ordering::SeqCst);
                    // A read that timed out no longer waits: its late answer is dropped.
                    let _ = reply.send(answer);
                }
                // `load` and the clipboard it holds go first: dropping `Loads` waits for `alive`.
                drop(load);
                drop(alive);
            };
            std::thread::Builder::new().name(name.into()).spawn(work).ok()?;
            Some(Self { line: Arc::new(Line { requests, open, sent: Mutex::default() }), done, finish })
        }

        /// `request` sent to the worker; its answer comes on the channel returned, for
        /// [`wait_for_answer`] here or on another thread. Err when an earlier request is still
        /// unanswered or the worker has stopped.
        pub(super) fn ask(&mut self, request: R) -> Result<mpsc::Receiver<A>, String> {
            self.line.ask(request)
        }

        /// Whether a request sent is still unanswered.
        pub(super) fn waiting(&self) -> bool {
            self.line.open.load(Ordering::SeqCst) > 0
        }

        /// A handle that sends requests from another thread until this is dropped.
        pub(super) fn asker(&self) -> Asker<R, A> {
            Asker(Arc::downgrade(&self.line))
        }

        /// The answer to `request`, or why there is none, after `wait` at most.
        #[cfg(test)]
        pub(super) fn get(&mut self, request: R, wait: Duration) -> Result<A, String> {
            wait_for_answer(&self.ask(request)?, wait)
        }
    }

    /// The answer that comes on `answer` ([`Loads::ask`]), or why there is none, after `wait` at most.
    pub(super) fn wait_for_answer<A>(answer: &mpsc::Receiver<A>, wait: Duration) -> Result<A, String> {
        match answer.recv_timeout(wait) {
            Ok(answer) => Ok(answer),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("the clipboard's owner didn't send its data within {wait:?}")),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(GONE.into()),
        }
    }

    impl<R, A> Drop for Loads<R, A> {
        /// Stops the worker and waits until it has let go of what its `load` holds, until
        /// [`stop_by`] at most: on X11 the last arboard handle dropped hands what the app copied to
        /// the clipboard manager, and the app's own handle must be that one.
        fn drop(&mut self) {
            // No `Asker` sends after this, and one that holds the line lets go once it has sent.
            let pending = self.line.close();
            drop(std::mem::replace(&mut self.line, Line::dead()));
            let end = stop_by(pending, Instant::now(), self.finish);
            // Nothing is sent on `done`: this ends when the worker drops `alive`, or at `end`.
            let _ = self.done.recv_timeout(end.saturating_duration_since(Instant::now()));
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::Arc;

        use super::*;

        #[test]
        fn a_read_that_answers_in_time_gives_the_text() {
            let mut loads = Loads::spawn("test", Duration::ZERO, |()| "<svg/>".to_string()).unwrap();
            assert_eq!(loads.get((), Duration::from_secs(5)), Ok("<svg/>".to_string()));
            assert_eq!(loads.get((), Duration::from_secs(5)), Ok("<svg/>".to_string()));
        }

        #[test]
        fn an_owner_that_never_answers_holds_the_worker_not_the_caller() {
            let (release, gate) = mpsc::channel::<()>();
            let mut calls = 0;
            let mut loads = Loads::spawn("test", Duration::ZERO, move |()| {
                calls += 1;
                if calls == 1 {
                    // The first owner sends nothing until released.
                    let _ = gate.recv();
                    return "stale".to_string();
                }
                "fresh".to_string()
            })
            .unwrap();
            assert!(loads.get((), Duration::ZERO).is_err(), "the first read times out");
            // While it is unanswered, a read doesn't queue behind it or wait at all.
            assert!(loads.get((), Duration::from_secs(600)).is_err());
            release.send(()).unwrap();
            // Once the late answer is in, it is dropped and a new read is made.
            let text = loop {
                match loads.get((), Duration::from_secs(600)) {
                    Ok(text) => break text,
                    Err(why) => {
                        // The late answer isn't in yet.
                        assert_ne!(why, GONE);
                        std::thread::yield_now();
                    }
                }
            };
            assert_eq!(text, "fresh");
        }

        #[test]
        fn a_read_after_the_worker_stops_says_so() {
            let mut loads = Loads::spawn("test", Duration::ZERO, |()| String::new()).unwrap();
            // The worker ends when its requests stop: cut them off by hand.
            drop(std::mem::replace(&mut loads.line, Line::dead()));
            assert_eq!(loads.get((), Duration::from_millis(50)), Err(GONE.to_string()));
        }

        #[test]
        fn another_threads_reads_stop_with_the_reads() {
            let loads = Loads::spawn("test", Duration::ZERO, |()| 1).unwrap();
            let asker = loads.asker();
            assert_eq!(asker.ask(()).and_then(|a| wait_for_answer(&a, Duration::from_secs(600))), Ok(1));
            drop(loads);
            assert_eq!(asker.ask(()).err(), Some(GONE.to_string()));
            // Closed while an asker still holds the line: nothing more is sent.
            let line = Line::<(), ()>::dead();
            assert_eq!(line.close(), None);
            assert_eq!(line.ask(()).err(), Some(GONE.to_string()));
        }

        #[test]
        fn an_answer_waited_for_on_another_thread_holds_back_the_next_read() {
            let (release, gate) = mpsc::channel::<()>();
            let mut loads = Loads::spawn("test", Duration::ZERO, move |n: u32| {
                if n == 1 {
                    let _ = gate.recv();
                }
                n
            })
            .unwrap();
            let answer = loads.ask(1).unwrap();
            // The caller doesn't wait: the answer is waited for wherever the channel goes.
            let waiter = std::thread::spawn(move || wait_for_answer(&answer, Duration::from_secs(600)));
            // Until it comes, a read fails at once instead of queuing behind it.
            assert!(loads.get(2, Duration::from_secs(600)).is_err());
            release.send(()).unwrap();
            assert_eq!(waiter.join().ok(), Some(Ok(1)));
            assert_eq!(loads.get(3, Duration::from_secs(600)), Ok(3));
        }

        #[test]
        fn dropping_the_reads_waits_until_the_worker_lets_go_of_its_clipboard() {
            let clipboard = Arc::new(());
            let held = clipboard.clone();
            let mut loads = Loads::spawn("test", Duration::ZERO, move |()| Arc::strong_count(&held)).unwrap();
            assert_eq!(loads.get((), Duration::from_secs(600)), Ok(2));
            drop(loads);
            assert_eq!(Arc::strong_count(&clipboard), 1, "the worker's handle is gone before the app's");
        }

        #[test]
        fn dropping_the_reads_waits_for_an_unanswered_read_up_to_its_limit() {
            let now = Instant::now();
            let limit = Duration::from_secs(25);
            // No read unanswered: the idle worker ends at once, LOAD_WAIT is only the bound.
            assert_eq!(stop_by(None, now, limit), now + LOAD_WAIT);
            // A read sent 2 s ago that timed out: wait for it until 25 s after it was sent.
            assert_eq!(stop_by(Some(now - Duration::from_secs(2)), now, limit), now + Duration::from_secs(23));
            // Its limit already over, or no limit (data-control, the core protocol): no wait.
            assert!(stop_by(Some(now - Duration::from_secs(30)), now, limit) <= now);
            assert_eq!(stop_by(Some(now), now, Duration::ZERO), now);
        }
    }
}

/// arboard, or on a Wayland compositor without its data-control protocol the core protocol's text
/// clipboard: one format at a time. Built everywhere so it is checked everywhere.
#[cfg_attr(windows, allow(dead_code))]
mod portable {
    use std::borrow::Cow;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::{BITMAP, ClipboardRead, Flavour, PNG, SystemClipboard, TEXT, copied_file, decode_png, png_flavour, text_of};

    /// What our last write put on the clipboard.
    #[derive(Clone)]
    enum Ours {
        Text(String),
        Image(usize, usize),
    }

    /// A read of arboard's clipboard.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Request {
        Text,
        Image,
        Files,
    }

    /// What a [`Request`] read.
    enum Answer {
        Text(Result<String, String>),
        Image(Result<arboard::ImageData<'static>, String>),
        Files(Vec<PathBuf>),
    }

    /// What a read says when it gets the answer to another request.
    const MIXED: &str = "the clipboard answered another read";

    impl Answer {
        /// The answer to `request` when it couldn't be read: `why`, or no files.
        fn failed(request: Request, why: String) -> Self {
            match request {
                Request::Text => Self::Text(Err(why)),
                Request::Image => Self::Image(Err(why)),
                Request::Files => Self::Files(vec![]),
            }
        }

        /// The request this answers.
        #[cfg_attr(
            not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")),
            allow(dead_code)
        )]
        fn request(&self) -> Request {
            match self {
                Self::Text(_) => Request::Text,
                Self::Image(_) => Request::Image,
                Self::Files(_) => Request::Files,
            }
        }

        fn text(self) -> Result<String, String> {
            match self {
                Self::Text(text) => text,
                _ => Err(MIXED.into()),
            }
        }

        fn image(self) -> Result<arboard::ImageData<'static>, String> {
            match self {
                Self::Image(image) => image,
                _ => Err(MIXED.into()),
            }
        }

        fn files(self) -> Vec<PathBuf> {
            match self {
                Self::Files(files) => files,
                _ => vec![],
            }
        }
    }

    /// `request` read through `cb`.
    fn answer(cb: &mut arboard::Clipboard, request: Request) -> Answer {
        match request {
            Request::Text => Answer::Text(cb.get_text().map_err(|e| e.to_string())),
            Request::Image => Answer::Image(cb.get_image().map_err(|e| e.to_string())),
            Request::Files => Answer::Files(cb.get().file_list().unwrap_or_default()),
        }
    }

    /// `request` read on `reads`' worker, waiting `wait` at most.
    #[cfg(test)]
    fn read_within(reads: &mut super::loads::Loads<Request, Answer>, request: Request, wait: Duration) -> Answer {
        reads.get(request, wait).unwrap_or_else(|why| Answer::failed(request, why))
    }

    /// The longest a paste's bitmap read of arboard's waits for the app that copied, off the UI
    /// thread: the bitmap's transfer and its PNG decoding happen within the read. A paste waits as
    /// long, from when it was sent, for a read that got no answer in its wait, the core protocol's
    /// text read included. Text, file lists and the UI thread's bitmap check wait
    /// [`LOAD_WAIT`](super::loads::LOAD_WAIT).
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    const IMAGE_WAIT: Duration = Duration::from_secs(10);

    /// How long a read of `request` waits for the app that copied.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    fn wait_for(request: Request) -> Duration {
        match request {
            Request::Image => IMAGE_WAIT,
            Request::Text | Request::Files => super::loads::LOAD_WAIT,
        }
    }

    /// The longest an X11 read of arboard's lasts while the owner sends nothing: a text read tries
    /// six formats and waits up to 4 s for each, and a second more covers the rest of the read.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    const X11_READ_LIMIT: Duration = Duration::from_secs(25);

    /// Whether arboard's handles use the compositor's data-control protocol rather than X11. arboard
    /// 3.7 makes the same check when it opens: data-control where `WAYLAND_DISPLAY` is set and
    /// wl-clipboard-rs finds the protocol, X11 otherwise.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    fn on_data_control() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some() && wl_clipboard_rs::utils::is_primary_selection_supported().is_ok()
    }

    /// How long after an unanswered read was sent dropping the reads waits for it to end: on X11
    /// the last handle dropped hands what the app copied to a clipboard manager, and on
    /// data-control, where a read may never end, nothing is handed over.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    fn finish(data_control: bool) -> Duration {
        if data_control { Duration::ZERO } else { X11_READ_LIMIT }
    }

    /// arboard's clipboard, with `C` for writes (arboard's handle; a stand-in in tests). On Linux
    /// and the BSDs its reads run on a worker with a handle of its own and wait
    /// [`LOAD_WAIT`](super::loads::LOAD_WAIT) at most, a paste's bitmap 10 s off the UI thread: arboard's Wayland
    /// data-control read waits for the app that copied to send its data with no limit, and its X11
    /// read waits up to 4 s for each format it tries.
    struct Arboard<C = arboard::Clipboard> {
        /// Dropped before `cb`, and waits for its worker's handle to go, on X11 up to
        /// [`X11_READ_LIMIT`] after an unanswered read was sent: there the last handle dropped
        /// hands what the app copied to the clipboard manager.
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        reads: super::loads::Loads<Request, Answer>,
        /// The last read that got no answer in its wait, until a paste takes it or the next read
        /// is sent.
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        unanswered: Option<Unanswered>,
        /// Writes, and elsewhere reads.
        cb: C,
    }

    /// A read on arboard's worker that got no answer in its wait.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    struct Unanswered {
        request: Request,
        answer: std::sync::mpsc::Receiver<Answer>,
        sent: Instant,
        /// The worker, for the paste's other reads.
        reads: super::loads::Asker<Request, Answer>,
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    impl Unanswered {
        /// Whether a paste of one of `mimes` can take this read's answer: files paste as any of
        /// them.
        fn fits(&self, mimes: &[&'static str]) -> bool {
            match self.request {
                Request::Text => mimes.contains(&TEXT),
                Request::Image => mimes.contains(&BITMAP),
                Request::Files => true,
            }
        }

        /// What a paste of one of `mimes` takes, once this read's answer comes, waiting for it until
        /// [`IMAGE_WAIT`] after it was sent, as long as a paste's bitmap read waits. `None`, so the
        /// paste takes the internal clipboard, when the answer is what `ours` put on the clipboard
        /// or doesn't come. Otherwise the clipboard is read as [`Clipboard::read_now_or_later`] and
        /// the paste read it, this answer standing in for the read it answers: copied files, else
        /// text, else the bitmap, and the bitmap when the text is blank or only names a picture
        /// (`adopt_system_clipboard_or_defer` in `vectorcraft-ui-egui`).
        fn paste(self, ours: Option<&Ours>, mimes: &[&'static str]) -> Option<Flavour> {
            let wait = (self.sent + IMAGE_WAIT).saturating_duration_since(Instant::now());
            let answer = super::loads::wait_for_answer(&self.answer, wait).ok()?;
            let held = match (&answer, ours) {
                (Answer::Text(Ok(now)), Some(Ours::Text(t))) => now == t,
                (Answer::Image(Ok(now)), Some(Ours::Image(w, h))) => (now.width, now.height) == (*w, *h),
                _ => false,
            };
            if held {
                return None;
            }
            let mut answer = Some(answer);
            let mut read = |request: Request| match answer.take_if(|a| a.request() == request) {
                Some(answer) => answer,
                None => self
                    .reads
                    .ask(request)
                    .and_then(|a| super::loads::wait_for_answer(&a, wait_for(request)))
                    .unwrap_or_else(|why| Answer::failed(request, why)),
            };
            if let Some(f) = copied_file(read(Request::Files).files(), mimes) {
                return Some(f);
            }
            let text = if mimes.contains(&TEXT) { read(Request::Text).text().ok().filter(|t| !t.is_empty()) } else { None };
            let bitmap = |read: &mut dyn FnMut(Request) -> Answer| image_flavour(read(Request::Image).image().ok()?);
            match text {
                Some(t) if t.trim().is_empty() || vectorcraft_engine::cmd::clipboard::names_a_picture(&t) => {
                    bitmap(&mut read).or(Some(Flavour { mime: TEXT, data: t.into_bytes() }))
                }
                Some(t) => Some(Flavour { mime: TEXT, data: t.into_bytes() }),
                None if mimes.contains(&BITMAP) => bitmap(&mut read),
                None => None,
            }
        }
    }

    /// A read that got no answer in its wait, for a paste to wait for off the UI thread.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    enum Late {
        Arboard(Unanswered),
        /// The core protocol's text read.
        #[cfg(target_os = "linux")]
        Wayland(super::wayland::Unanswered),
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    impl Late {
        /// What a paste of one of `mimes` takes once the answer comes ([`Unanswered::paste`]). The
        /// core protocol's text is waited for as long, until [`IMAGE_WAIT`] after its read was
        /// sent, and pasted unless it is empty, what `ours` put on the clipboard, or doesn't come.
        fn paste(self, ours: Option<&Ours>, mimes: &[&'static str]) -> Option<Flavour> {
            match self {
                Self::Arboard(unanswered) => unanswered.paste(ours, mimes),
                #[cfg(target_os = "linux")]
                Self::Wayland(unanswered) => {
                    let text = unanswered.text(IMAGE_WAIT).ok().filter(|t| !t.is_empty())?;
                    let held = matches!(ours, Some(Ours::Text(t)) if *t == text);
                    (!held).then(|| Flavour { mime: TEXT, data: text.into_bytes() })
                }
            }
        }
    }

    /// arboard's bitmap as a PNG flavour.
    fn image_flavour(img: arboard::ImageData<'static>) -> Option<Flavour> {
        let (width, height) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
        png_flavour(&image::RgbaImage::from_raw(width, height, img.bytes.into_owned())?)
    }

    impl Arboard {
        fn open() -> Result<Self, String> {
            let cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
            {
                let mut reader = arboard::Clipboard::new().map_err(|e| e.to_string())?;
                let reads = super::loads::Loads::spawn("clipboard-read", finish(on_data_control()), move |request| answer(&mut reader, request))
                    .ok_or("the clipboard's reader didn't start")?;
                Ok(Self { reads, unanswered: None, cb })
            }
            #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")))]
            Ok(Self { cb })
        }

        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")))]
        fn read(&mut self, request: Request) -> Answer {
            answer(&mut self.cb, request)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    impl<C> Arboard<C> {
        /// Reads on the worker; the write handle `C` is not used.
        fn read(&mut self, request: Request) -> Answer {
            self.read_waiting(request, wait_for(request))
        }

        /// A bitmap read on the worker that waits [`LOAD_WAIT`](super::loads::LOAD_WAIT) at most, for
        /// a check on the UI thread.
        fn image_now(&mut self) -> Result<arboard::ImageData<'static>, String> {
            self.read_waiting(Request::Image, super::loads::LOAD_WAIT).image()
        }

        /// `request` read on the worker, waiting `wait` at most. A read that gets no answer in that
        /// time is kept ([`Self::unanswered`]) for a paste to wait for off the UI thread.
        fn read_waiting(&mut self, request: Request, wait: Duration) -> Answer {
            let sent = Instant::now();
            let answer = match self.ask(request) {
                Ok(answer) => answer,
                Err(why) => return Answer::failed(request, why),
            };
            match super::loads::wait_for_answer(&answer, wait) {
                Ok(answer) => answer,
                Err(why) => {
                    if why != super::loads::GONE {
                        self.unanswered = Some(Unanswered { request, answer, sent, reads: self.reads.asker() });
                    }
                    Answer::failed(request, why)
                }
            }
        }

        /// `request` sent to the worker. A read sent means the one kept unanswered has been
        /// answered, and its answer is dropped.
        fn ask(&mut self, request: Request) -> Result<std::sync::mpsc::Receiver<Answer>, String> {
            let answer = self.reads.ask(request)?;
            self.unanswered = None;
            Ok(answer)
        }

        /// A bitmap read sent to the worker now; the work returned waits [`IMAGE_WAIT`] at most for
        /// it, so the caller can wait off the UI thread. Err when the read can't be sent.
        fn image_later(&mut self) -> Result<ImageWork, String> {
            let answer = self.ask(Request::Image)?;
            Ok(Box::new(move || super::loads::wait_for_answer(&answer, IMAGE_WAIT).and_then(Answer::image)))
        }

        /// The read that got no answer in its wait, while its answer has still not come and a paste
        /// of one of `mimes` can take it: what the clipboard holds is not known until it comes. Once
        /// it has come the answer may be old, and it is dropped: a new read is made.
        fn unanswered(&mut self, mimes: &[&'static str]) -> Option<Unanswered> {
            let unanswered = self.unanswered.take_if(|u| u.fits(mimes))?;
            self.reads.waiting().then_some(unanswered)
        }
    }

    /// A bitmap read whose answer is waited for when it runs.
    type ImageWork = Box<dyn FnOnce() -> Result<arboard::ImageData<'static>, String> + Send>;

    /// An open clipboard connection.
    enum Backend {
        Arboard(Arboard),
        /// The compositor's core data-device protocol on the window's connection (text only).
        #[cfg(target_os = "linux")]
        Wayland(super::wayland::TextClipboard),
    }

    /// What a write of a picture says where the clipboard carries text only.
    #[cfg(target_os = "linux")]
    const TEXT_ONLY: &str = "this clipboard carries text only";

    impl Backend {
        /// arboard (Wayland data-control, else X11), else the core protocol on `window`'s
        /// Wayland connection. Err: why each failed.
        fn open(window: Option<&Arc<winit::window::Window>>) -> Result<Self, String> {
            first_open(
                || Arboard::open().map(Self::Arboard),
                || {
                    #[cfg(target_os = "linux")]
                    return window.and_then(|w| super::wayland::TextClipboard::new(w.clone())).map(Self::Wayland).ok_or("not a Wayland window");
                    #[cfg(not(target_os = "linux"))]
                    {
                        let _ = window;
                        Err("not on Linux")
                    }
                },
            )
        }

        fn name(&self) -> &'static str {
            match self {
                Self::Arboard(_) => "arboard",
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => "the Wayland core protocol (text only)",
            }
        }

        fn get_text(&mut self) -> Result<String, String> {
            match self {
                Self::Arboard(cb) => cb.read(Request::Text).text(),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.get_text(),
            }
        }

        fn set_text(&mut self, text: &str) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.cb.set_text(text).map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => {
                    cb.set_text(text);
                    Ok(())
                }
            }
        }

        /// The bitmap, for a check on the UI thread: on Linux and the BSDs it waits
        /// [`LOAD_WAIT`](super::loads::LOAD_WAIT) at most, as text does.
        fn image_now(&mut self) -> Result<arboard::ImageData<'static>, String> {
            match self {
                #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
                Self::Arboard(cb) => cb.image_now(),
                #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")))]
                Self::Arboard(cb) => cb.read(Request::Image).image(),
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => Err(TEXT_ONLY.into()),
            }
        }

        /// The bitmap as work for another thread: on Linux and the BSDs the wait for the app
        /// that copied happens when the work runs, elsewhere the bitmap is read here.
        fn image_later(&mut self) -> Result<ImageWork, String> {
            match self {
                #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
                Self::Arboard(cb) => cb.image_later(),
                #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd")))]
                Self::Arboard(cb) => {
                    let image = cb.read(Request::Image).image()?;
                    Ok(Box::new(move || Ok(image)))
                }
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => Err(TEXT_ONLY.into()),
            }
        }

        fn set_image(&mut self, img: arboard::ImageData<'_>) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.cb.set_image(img).map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => Err(TEXT_ONLY.into()),
            }
        }

        fn clear(&mut self) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.cb.clear().map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => {
                    cb.set_text("");
                    Ok(())
                }
            }
        }

        /// The read that got no answer in its wait, when a paste of one of `mimes` can take its
        /// answer ([`Arboard::unanswered`], [`TextReads::unanswered`](super::wayland::TextReads::unanswered)).
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        fn unanswered(&mut self, mimes: &[&'static str]) -> Option<Late> {
            match self {
                Self::Arboard(cb) => cb.unanswered(mimes).map(Late::Arboard),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.unanswered(mimes).map(Late::Wayland),
            }
        }

        /// Drops the read that got no answer in its wait: a paste no longer takes its answer.
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        fn forget_unanswered(&mut self) {
            match self {
                Self::Arboard(cb) => cb.unanswered = None,
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.forget_unanswered(),
            }
        }

        /// Files copied in a file manager (none where the backend can't read a file list).
        fn file_list(&mut self) -> Vec<PathBuf> {
            match self {
                Self::Arboard(cb) => cb.read(Request::Files).files(),
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => vec![],
            }
        }
    }

    /// The first of `arboard` and `wayland` that opens, tried in that order. Err: why each failed.
    fn first_open<B>(arboard: impl FnOnce() -> Result<B, String>, wayland: impl FnOnce() -> Result<B, &'static str>) -> Result<B, String> {
        let arboard = match arboard() {
            Ok(cb) => return Ok(cb),
            Err(e) => e,
        };
        wayland().map_err(|wayland| format!("arboard: {arboard}; Wayland core protocol: {wayland}"))
    }

    /// Who asks for the clipboard: the background paste check waits out the whole wait after a
    /// failure, a Copy or Paste of the user's [`USER_RETRY`] at most.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub(super) enum Asker {
        Probe,
        User,
    }

    /// The wait after the first failure to open the clipboard; it doubles with each failed retry.
    const FIRST_WAIT: Duration = Duration::from_secs(1);
    /// The longest wait between retries.
    const MAX_WAIT: Duration = Duration::from_secs(60);
    /// A Copy or Paste of the user's retries at most this often.
    const USER_RETRY: Duration = Duration::from_secs(1);

    /// Whether "unavailable" has been logged and not yet answered by "available again": the
    /// clipboard and its background check hold a handle each, and their failures are one outage.
    static UNAVAILABLE_LOGGED: AtomicBool = AtomicBool::new(false);

    /// A failure to open, and when to try again.
    struct Failed {
        at: Instant,
        wait: Duration,
    }

    /// A handle opened on first use and kept, or after a failure opened again only once its wait is
    /// over: where neither backend opens, asking a few times a second must not reconnect each time.
    pub(super) struct Reopen<B> {
        handle: Option<B>,
        failed: Option<Failed>,
        /// The outage flag shared with the other handles ([`UNAVAILABLE_LOGGED`]).
        logged: &'static AtomicBool,
    }

    impl<B> Reopen<B> {
        pub(super) fn new(logged: &'static AtomicBool) -> Self {
            Self { handle: None, failed: None, logged }
        }

        /// The open handle, opened with `open` when there is none and `asker` may try at `now`.
        /// `name` names an opened handle in the log.
        pub(super) fn get(
            &mut self,
            asker: Asker,
            now: Instant,
            open: impl FnOnce() -> Result<B, String>,
            name: impl Fn(&B) -> &'static str,
        ) -> Option<&mut B> {
            if self.handle.is_none() {
                if let Some(f) = &self.failed {
                    let wait = match asker {
                        Asker::Probe => f.wait,
                        Asker::User => f.wait.min(USER_RETRY),
                    };
                    if now.saturating_duration_since(f.at) < wait {
                        return None;
                    }
                }
                match open() {
                    Ok(handle) => {
                        if self.logged.swap(false, Ordering::Relaxed) {
                            log::info!("the system clipboard is available again, through {}", name(&handle));
                        } else {
                            log::debug!("system clipboard opened through {}", name(&handle));
                        }
                        self.failed = None;
                        self.handle = Some(handle);
                    }
                    Err(why) => {
                        let wait = match &self.failed {
                            Some(f) => (f.wait * 2).min(MAX_WAIT),
                            None => FIRST_WAIT,
                        };
                        if self.logged.swap(true, Ordering::Relaxed) {
                            log::debug!("the system clipboard is still unavailable ({why}); next try in {wait:?}");
                        } else {
                            log::warn!("the system clipboard is unavailable ({why}): Copy and Paste stay inside VectorCraft");
                        }
                        self.failed = Some(Failed { at: now, wait });
                    }
                }
            }
            self.handle.as_mut()
        }

        /// The handle if one is open, without trying to open one.
        pub(super) fn opened(&mut self) -> Option<&mut B> {
            self.handle.as_mut()
        }
    }

    pub struct Clipboard {
        /// Kept open: on Linux the copied data lives as long as the handle does.
        cb: Reopen<Backend>,
        /// The app window, for the Wayland core protocol (on its connection).
        window: Option<Arc<winit::window::Window>>,
        ours: Option<Ours>,
    }

    impl Clipboard {
        pub fn new(window: Option<Arc<winit::window::Window>>) -> Self {
            Self { cb: Reopen::new(&UNAVAILABLE_LOGGED), window, ours: None }
        }

        fn cb(&mut self, asker: Asker) -> Result<&mut Backend, String> {
            let window = self.window.as_ref();
            self.cb.get(asker, Instant::now(), || Backend::open(window), Backend::name).ok_or_else(|| "no system clipboard".to_string())
        }

        /// The work that pastes the answer to a read that got no answer in its wait
        /// ([`Late::paste`]), while that answer has not come.
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        fn paste_unanswered(&mut self, mimes: &[&'static str]) -> Option<ClipboardRead> {
            let ours = self.ours.clone();
            let unanswered = self.cb(Asker::User).ok()?.unanswered(mimes)?;
            let mimes = mimes.to_vec();
            Some(ClipboardRead::Later(Box::new(move || unanswered.paste(ours.as_ref(), &mimes))))
        }

        /// Text and copied files are read here, and work that reads arboard's image comes back.
        fn read_now_or_later(&mut self, mimes: &[&'static str]) -> Option<ClipboardRead> {
            let cb = self.cb(Asker::User).ok()?;
            if let Some(f) = copied_file(cb.file_list(), mimes) {
                return Some(ClipboardRead::Now(f));
            }
            mimes.iter().find_map(|m| {
                #[cfg(target_os = "macos")]
                if let Some(f) = super::mac::read(m) {
                    return Some(ClipboardRead::Now(f));
                }
                match *m {
                    TEXT => cb.get_text().ok().filter(|t| !t.is_empty()).map(|t| ClipboardRead::Now(Flavour { mime: TEXT, data: t.into_bytes() })),
                    BITMAP => {
                        let read = cb.image_later().ok()?;
                        Some(ClipboardRead::Later(Box::new(move || image_flavour(read().ok()?))))
                    }
                    _ => None,
                }
            })
        }
    }

    impl SystemClipboard for Clipboard {
        fn write(&mut self, flavours: &[Flavour]) -> Result<(), String> {
            let text = text_of(flavours);
            let image = match &text {
                Some(_) => None,
                None => flavours.iter().find(|f| f.mime == PNG).and_then(|f| decode_png(&f.data)),
            };
            let cb = self.cb(Asker::User)?;
            let ours = match (text, image) {
                (Some(t), _) => {
                    cb.set_text(t.as_str())?;
                    Some(Ours::Text(t))
                }
                (None, Some(img)) => {
                    let (width, height) = (img.width() as usize, img.height() as usize);
                    cb.set_image(arboard::ImageData { width, height, bytes: Cow::Owned(img.into_raw()) })?;
                    Some(Ours::Image(width, height))
                }
                (None, None) => {
                    cb.clear()?;
                    None
                }
            };
            // What a read got late was on the clipboard before this write.
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
            cb.forget_unanswered();
            self.ours = ours;
            Ok(())
        }

        fn holds_ours(&mut self) -> bool {
            let Some(cb) = self.cb.opened() else { return false };
            match &self.ours {
                Some(Ours::Text(t)) => cb.get_text().is_ok_and(|now| now == *t),
                Some(Ours::Image(w, h)) => cb.image_now().is_ok_and(|now| (now.width, now.height) == (*w, *h)),
                None => false,
            }
        }

        fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
            self.read_now_or_later(mimes)?.flavour()
        }

        /// arboard's image is read here and encoded as PNG by the work that comes back. On Linux and
        /// the BSDs, after a read that got no answer in its wait (such as the check whether the
        /// clipboard still holds what was copied here, or one of the reads here), the work waits for
        /// that read's answer instead and pastes from it, or nothing when it is what was copied here.
        fn read_deferred(&mut self, mimes: &[&'static str]) -> Option<ClipboardRead> {
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
            if let Some(read) = self.paste_unanswered(mimes) {
                return Some(read);
            }
            let read = self.read_now_or_later(mimes);
            // One of these reads got no answer in its wait, and the ones after it failed at once.
            #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
            if read.is_none() {
                return self.paste_unanswered(mimes);
            }
            read
        }

        /// Text, and on macOS vector art or a bitmap (by the pasteboard's types). Linux has no
        /// cheap way to ask for a bitmap (reading one costs as much as pasting it): the paste keys
        /// take it, the menu item waits for text.
        fn has(&mut self, mimes: &[&'static str]) -> bool {
            #[cfg(target_os = "macos")]
            if super::mac::has(mimes) {
                return true;
            }
            mimes.contains(&TEXT) && self.cb(Asker::Probe).is_ok_and(|cb| cb.get_text().is_ok_and(|t| !t.is_empty()))
        }
    }

    #[cfg(test)]
    mod tests {
        use std::cell::Cell;

        use super::super::loads::Loads;
        use super::*;

        /// A clock that starts now and moves only when told.
        struct Clock(Cell<Instant>);

        impl Clock {
            fn new() -> Self {
                Self(Cell::new(Instant::now()))
            }
            fn now(&self) -> Instant {
                self.0.get()
            }
            fn advance(&self, by: Duration) {
                self.0.set(self.0.get() + by);
            }
        }

        fn fail() -> Result<u8, String> {
            Err("no compositor".into())
        }

        fn name(_: &u8) -> &'static str {
            "fake"
        }

        #[test]
        fn the_paste_check_retries_with_a_growing_wait() {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            let (clock, attempts) = (Clock::new(), Cell::new(0));
            let mut cb = Reopen::new(&LOGGED);
            // Every 250 ms for a minute, as `clipboard_probe` asks.
            for _ in 0..240 {
                let open = || {
                    attempts.set(attempts.get() + 1);
                    fail()
                };
                assert!(cb.get(Asker::Probe, clock.now(), open, name).is_none());
                clock.advance(Duration::from_millis(250));
            }
            // 0, 1, 3, 7, 15 and 31 s: not one connection a check.
            assert_eq!(attempts.get(), 6);
            assert!(LOGGED.load(Ordering::Relaxed), "the outage is logged");
        }

        #[test]
        fn the_wait_stops_growing_at_a_minute() {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            let (clock, attempts) = (Clock::new(), Cell::new(0));
            let mut cb = Reopen::new(&LOGGED);
            for _ in 0..(20 * 60) {
                let open = || {
                    attempts.set(attempts.get() + 1);
                    fail()
                };
                let _ = cb.get(Asker::Probe, clock.now(), open, name);
                clock.advance(Duration::from_secs(1));
            }
            // 0, 1, 3, 7, 15, 31, 63 s, then a minute apart until 20 minutes are up.
            assert_eq!(attempts.get(), 7 + (20 * 60 - 63 - 1) / 60);
        }

        #[test]
        fn a_copy_or_paste_retries_at_most_once_a_second() {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            let (clock, attempts) = (Clock::new(), Cell::new(0));
            let mut cb = Reopen::new(&LOGGED);
            let mut ask = |asker| {
                let open = || {
                    attempts.set(attempts.get() + 1);
                    fail()
                };
                let _ = cb.get(asker, clock.now(), open, name);
            };
            // Long after the paste check gave up for a minute, a held Ctrl+V for 3 s at 30 a second.
            for _ in 0..8 {
                ask(Asker::Probe);
                clock.advance(Duration::from_secs(70));
            }
            let before = attempts.get();
            for _ in 0..90 {
                ask(Asker::User);
                clock.advance(Duration::from_millis(33));
            }
            assert_eq!(attempts.get() - before, 3);
        }

        #[test]
        fn a_success_is_kept_and_ends_the_outage() {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            let clock = Clock::new();
            let mut cb = Reopen::new(&LOGGED);
            assert!(cb.get(Asker::Probe, clock.now(), fail, name).is_none());
            assert!(LOGGED.load(Ordering::Relaxed));
            clock.advance(FIRST_WAIT);
            assert_eq!(cb.get(Asker::Probe, clock.now(), || Ok(7), name).copied(), Some(7));
            assert!(!LOGGED.load(Ordering::Relaxed), "the recovery is logged");
            // Kept: not opened again, even where opening would now fail.
            assert_eq!(cb.get(Asker::Probe, clock.now(), fail, name).copied(), Some(7));
            assert_eq!(cb.opened().copied(), Some(7));
        }

        #[test]
        fn arboard_is_tried_before_the_wayland_core_protocol() {
            let wayland_tried = Cell::new(false);
            let wayland = |ok| {
                wayland_tried.set(true);
                if ok { Ok("wayland") } else { Err("not a Wayland window") }
            };
            // Where arboard opens (data-control, X11) the core protocol isn't tried.
            assert_eq!(first_open(|| Ok("arboard"), || wayland(true)), Ok("arboard"));
            assert!(!wayland_tried.get());
            // The Flatpak on GNOME: no data-control, no X11.
            assert_eq!(first_open(|| Err("no X11".into()), || wayland(true)), Ok("wayland"));
            assert!(wayland_tried.get());
            // Neither: every reason is kept for the log.
            assert_eq!(
                first_open(|| Err("no X11".into()), || wayland(false)),
                Err::<&str, _>("arboard: no X11; Wayland core protocol: not a Wayland window".into())
            );
        }

        #[test]
        fn reads_return_while_the_clipboard_owner_sends_nothing() {
            let (release, gate) = std::sync::mpsc::channel::<()>();
            // The app that copied never sends its data until released.
            let mut reads = Loads::spawn("test", Duration::ZERO, move |request| {
                let _ = gate.recv();
                Answer::failed(request, "late".into())
            })
            .unwrap();
            // Copy's look at the text, Paste's file list and its bitmap: none waits for the owner.
            assert!(read_within(&mut reads, Request::Text, Duration::ZERO).text().is_err());
            assert!(read_within(&mut reads, Request::Files, Duration::from_secs(600)).files().is_empty());
            assert!(read_within(&mut reads, Request::Image, Duration::from_secs(600)).image().is_err());
            drop(release);
        }

        #[test]
        fn each_read_takes_its_own_answer() {
            let mut reads = Loads::spawn("test", Duration::ZERO, |request| match request {
                Request::Text => Answer::Text(Ok("<svg/>".into())),
                Request::Image => Answer::Image(Ok(arboard::ImageData { width: 2, height: 1, bytes: Cow::Owned(vec![0; 8]) })),
                Request::Files => Answer::Files(vec![PathBuf::from("art.svg")]),
            })
            .unwrap();
            let wait = Duration::from_secs(600);
            assert_eq!(read_within(&mut reads, Request::Text, wait).text(), Ok("<svg/>".to_string()));
            assert_eq!(read_within(&mut reads, Request::Image, wait).image().map(|i| (i.width, i.height)), Ok((2, 1)));
            assert_eq!(read_within(&mut reads, Request::Files, wait).files(), [PathBuf::from("art.svg")]);
            // The answer to another read is not taken for this one.
            assert_eq!(Answer::Files(vec![]).text(), Err(MIXED.to_string()));
            assert!(Answer::Text(Ok(String::new())).image().is_err());
            assert!(Answer::Text(Ok("art.svg".into())).files().is_empty());
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        #[test]
        fn arboards_reads_run_on_its_reader_and_wait_by_kind() {
            let reads = Loads::spawn("test", Duration::ZERO, |request| match request {
                Request::Text => Answer::Text(Ok("read on the worker".into())),
                _ => Answer::failed(request, "not asked".into()),
            })
            .unwrap();
            // `()` for writes: a read made through the write handle wouldn't build.
            let mut cb = Arboard { reads, unanswered: None, cb: () };
            assert_eq!(cb.read(Request::Text).text(), Ok("read on the worker".to_string()));
            assert_eq!(wait_for(Request::Text), super::super::loads::LOAD_WAIT);
            assert_eq!(wait_for(Request::Files), super::super::loads::LOAD_WAIT);
            assert_eq!(wait_for(Request::Image), Duration::from_secs(10));
            // X11 waits for an unanswered read when the clipboard closes; data-control doesn't.
            assert_eq!(finish(false), X11_READ_LIMIT);
            assert_eq!(finish(true), Duration::ZERO);
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        #[test]
        fn a_bitmap_paste_waits_for_the_owner_off_the_ui_thread() {
            let (release, gate) = std::sync::mpsc::channel::<()>();
            // The app that copied sends its bitmap only when released.
            let reads = Loads::spawn("test", Duration::ZERO, move |request| match request {
                Request::Image => {
                    let _ = gate.recv();
                    Answer::Image(Ok(arboard::ImageData { width: 2, height: 1, bytes: Cow::Owned(vec![0; 8]) }))
                }
                _ => Answer::failed(request, "not asked".into()),
            })
            .unwrap();
            let mut cb = Arboard { reads, unanswered: None, cb: () };
            // The read is sent and the UI thread goes on without its answer.
            let work = cb.image_later().unwrap();
            // A check on the UI thread meanwhile fails at once instead of waiting behind it.
            assert!(cb.read(Request::Text).text().is_err());
            let paste = std::thread::spawn(work);
            release.send(()).unwrap();
            assert_eq!(paste.join().ok().and_then(Result::ok).map(|i| (i.width, i.height)), Some((2, 1)));
        }

        /// An arboard clipboard whose owner answers each read with `answer` only once the sender
        /// returned is used or dropped.
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        fn slow_owner(answer: impl Fn(Request) -> Answer + Send + 'static) -> (Arboard<()>, std::sync::mpsc::Sender<()>) {
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let reads = Loads::spawn("test", Duration::ZERO, move |request| {
                let _ = gate.recv();
                answer(request)
            })
            .unwrap();
            (Arboard { reads, unanswered: None, cb: () }, release)
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        #[test]
        fn a_paste_after_an_unanswered_ownership_check_waits_for_its_answer() {
            let image = |width: usize| Answer::Image(Ok(arboard::ImageData { width, height: 1, bytes: Cow::Owned(vec![0; width * 4]) }));
            // A 2×1 bitmap was copied here; the clipboard's owner sends a `width`×1 one late.
            let paste_after_check = |width: usize| {
                let (mut cb, release) = slow_owner(move |_| image(width));
                // The check whether the clipboard still holds what was copied here gets no answer
                // in its wait (2 s on the UI thread, none here).
                assert!(cb.read_waiting(Request::Image, Duration::ZERO).image().is_err());
                // The paste's own reads fail at once while the owner hasn't answered.
                assert!(cb.read(Request::Files).files().is_empty());
                assert!(cb.read(Request::Text).text().is_err());
                assert!(cb.image_later().is_err());
                // The paste waits for the check's answer off the UI thread instead.
                let unanswered = cb.unanswered(&vectorcraft_engine::cmd::clipboard::PASTE_ORDER).expect("the check's read is kept for the paste");
                let paste = std::thread::spawn(move || unanswered.paste(Some(&Ours::Image(2, 1)), &[BITMAP]));
                drop(release);
                paste.join().ok().flatten()
            };
            // Another app's bitmap is pasted, not the art copied here.
            let pasted = paste_after_check(3).expect("the new bitmap is pasted");
            assert_eq!((pasted.mime, decode_png(&pasted.data).map(|i| i.dimensions())), (PNG, Some((3, 1))));
            // The bitmap copied here: the paste takes the internal clipboard.
            assert!(paste_after_check(2).is_none());
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        #[test]
        fn a_late_answer_to_an_ownership_check_is_pasted_only_while_it_is_awaited() {
            // Text was copied here; the clipboard's owner answers the check with `text` late, and
            // has a 3×1 bitmap.
            let paste_after_check = |text: &'static str| {
                let (mut cb, release) = slow_owner(move |request| match request {
                    Request::Text => Answer::Text(Ok(text.into())),
                    Request::Image => Answer::Image(Ok(arboard::ImageData { width: 3, height: 1, bytes: Cow::Owned(vec![0; 12]) })),
                    Request::Files => Answer::Files(vec![]),
                });
                assert!(cb.read_waiting(Request::Text, Duration::ZERO).text().is_err());
                // A paste of a bitmap alone can't take a text read's answer: the read stays kept.
                assert!(cb.unanswered(&[BITMAP]).is_none());
                let unanswered = cb.unanswered(&vectorcraft_engine::cmd::clipboard::PASTE_ORDER).expect("the check's read is kept for the paste");
                let ours = Ours::Text("copied here".into());
                let paste = std::thread::spawn(move || unanswered.paste(Some(&ours), &vectorcraft_engine::cmd::clipboard::PASTE_ORDER));
                drop(release);
                paste.join().ok().flatten()
            };
            let pasted = paste_after_check("copied elsewhere").expect("the other app's text is pasted");
            assert_eq!((pasted.mime, pasted.data), (TEXT, b"copied elsewhere".to_vec()));
            // Text that only names a picture goes with it: the picture is pasted, as at once.
            let pasted = paste_after_check("<img src=\"art.png\">").expect("the picture is pasted");
            assert_eq!((pasted.mime, decode_png(&pasted.data).map(|i| i.dimensions())), (PNG, Some((3, 1))));
            // The text copied here: the paste takes the internal clipboard.
            assert!(paste_after_check("copied here").is_none());

            // A paste's own file-list read that gets no answer: once it comes, the text is read.
            let (mut cb, release) = slow_owner(|request| match request {
                Request::Text => Answer::Text(Ok("late text".into())),
                _ => Answer::failed(request, "none".into()),
            });
            assert!(cb.read_waiting(Request::Files, Duration::ZERO).files().is_empty());
            let unanswered = cb.unanswered(&[TEXT]).expect("the file-list read is kept for the paste");
            let paste = std::thread::spawn(move || unanswered.paste(None, &[TEXT]));
            drop(release);
            assert_eq!(paste.join().ok().flatten().map(|f| f.data), Some(b"late text".to_vec()));

            // An answer that came before the paste may be old: it is dropped, not pasted.
            let (mut cb, release) = slow_owner(|request| Answer::failed(request, "not asked".into()));
            assert!(cb.read_waiting(Request::Text, Duration::ZERO).text().is_err());
            drop(release);
            while cb.reads.waiting() {
                std::thread::yield_now();
            }
            assert!(cb.unanswered(&[TEXT]).is_none());
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn a_paste_after_an_unanswered_core_protocol_text_read_waits_for_its_answer() {
            use super::super::wayland::TextReads;
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;

            /// Core protocol text reads whose owner answers with `text` only once the sender
            /// returned is used or dropped.
            fn slow_text(text: &'static str) -> (TextReads, std::sync::mpsc::Sender<()>) {
                let (release, gate) = std::sync::mpsc::channel::<()>();
                let loads = Loads::spawn("test", Duration::ZERO, move |()| {
                    let _ = gate.recv();
                    Ok(text.to_string())
                })
                .unwrap();
                (TextReads::new(loads), release)
            }
            // Text was copied here; the clipboard's owner answers the check with `text` late.
            let paste_after_check = |text: &'static str| {
                let (mut reads, release) = slow_text(text);
                // The check whether the clipboard still holds what was copied here gets no answer
                // in its wait (2 s on the UI thread, none here).
                assert!(reads.get(Duration::ZERO).is_err());
                // The paste's own read fails at once while the owner hasn't answered.
                assert!(reads.get(Duration::from_secs(600)).is_err());
                // A paste of a bitmap alone can't take text: the read stays kept.
                assert!(reads.unanswered(&[BITMAP]).is_none());
                let late = Late::Wayland(reads.unanswered(&PASTE_ORDER).expect("the check's read is kept for the paste"));
                let ours = Ours::Text("copied here".into());
                let paste = std::thread::spawn(move || late.paste(Some(&ours), &PASTE_ORDER));
                drop(release);
                paste.join().ok().flatten()
            };
            let pasted = paste_after_check("copied elsewhere").expect("the other app's text is pasted");
            assert_eq!((pasted.mime, pasted.data), (TEXT, b"copied elsewhere".to_vec()));
            // The text copied here, or nothing: the paste takes the internal clipboard.
            assert!(paste_after_check("copied here").is_none());
            assert!(paste_after_check("").is_none());

            // An answer that came before the paste may be old: it is dropped, not pasted.
            let (mut reads, release) = slow_text("old");
            assert!(reads.get(Duration::ZERO).is_err());
            drop(release);
            while reads.waiting() {
                std::thread::yield_now();
            }
            assert!(reads.unanswered(&[TEXT]).is_none());
        }

        #[test]
        fn one_outage_is_logged_once_across_handles() {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            let clock = Clock::new();
            let (mut ui, mut probe) = (Reopen::new(&LOGGED), Reopen::new(&LOGGED));
            assert!(probe.get(Asker::Probe, clock.now(), fail, name).is_none());
            assert!(LOGGED.load(Ordering::Relaxed));
            // The other handle's failure is the same outage: `swap` reports it already logged.
            assert!(ui.get(Asker::User, clock.now(), fail, name).is_none());
            assert!(LOGGED.load(Ordering::Relaxed));
            assert!(probe.opened().is_none() && ui.opened().is_none());
        }
    }
}

/// The compositor's clipboard through the core data-device protocol (`wl_data_device`), on the
/// window's own Wayland connection: what Copy and Paste use where the compositor lacks the
/// data-control protocol arboard needs (GNOME) and there is no X11 to fall back on (the Flatpak on
/// a Wayland session). Text only (smithay-clipboard), and only while the window has focus, as the
/// protocol has it. A read waits [`LOAD_WAIT`] at most: smithay-clipboard waits for the owner's
/// data with no limit, so reads run on a worker of their own. A read that gets no answer in that
/// time is kept for a paste to wait for off the UI thread, as arboard's are.
#[cfg(target_os = "linux")]
mod wayland {
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use super::TEXT;
    use super::loads::{GONE, LOAD_WAIT, Loads, wait_for_answer};

    /// What a text read gives.
    type Text = Result<String, String>;

    pub struct TextClipboard {
        /// Writes: a store only hands the text to smithay-clipboard's thread.
        cb: smithay_clipboard::Clipboard,
        /// Reads, on a worker with a second clipboard on the same connection.
        reads: TextReads,
        /// Held as long as the clipboards: their connection is the window's.
        _window: Arc<winit::window::Window>,
    }

    /// Text reads on a worker, and the last one that got no answer in its wait.
    pub(super) struct TextReads {
        loads: Loads<(), Text>,
        /// The last read that got no answer in its wait, until a paste takes it or the next read
        /// is sent.
        unanswered: Option<Unanswered>,
    }

    /// A text read that got no answer in its wait.
    pub(super) struct Unanswered {
        answer: mpsc::Receiver<Text>,
        sent: Instant,
    }

    impl Unanswered {
        /// The text, once it comes, waiting for it until `limit` after the read was sent.
        pub(super) fn text(self, limit: Duration) -> Text {
            wait_for_answer(&self.answer, (self.sent + limit).saturating_duration_since(Instant::now()))?
        }
    }

    impl TextReads {
        pub(super) fn new(loads: Loads<(), Text>) -> Self {
            Self { loads, unanswered: None }
        }

        /// The text, waiting `wait` at most. A read that gets no answer in that time is kept
        /// ([`Self::unanswered`]) for a paste to wait for off the UI thread; a read sent means the
        /// one kept has been answered, and its answer is dropped.
        pub(super) fn get(&mut self, wait: Duration) -> Text {
            let sent = Instant::now();
            let answer = self.loads.ask(())?;
            self.unanswered = None;
            match wait_for_answer(&answer, wait) {
                Ok(text) => text,
                Err(why) => {
                    if why != GONE {
                        self.unanswered = Some(Unanswered { answer, sent });
                    }
                    Err(why)
                }
            }
        }

        /// The read that got no answer in its wait, while its answer has still not come and a
        /// paste of one of `mimes` can take text. Once it has come the answer may be old, and it is
        /// dropped: a new read is made.
        pub(super) fn unanswered(&mut self, mimes: &[&'static str]) -> Option<Unanswered> {
            let unanswered = self.unanswered.take_if(|_| mimes.contains(&TEXT))?;
            self.loads.waiting().then_some(unanswered)
        }

        /// Whether a read sent is still unanswered.
        #[cfg(test)]
        pub(super) fn waiting(&self) -> bool {
            self.loads.waiting()
        }
    }

    /// The read worker's clipboard and the window whose connection it uses, dropped in that order.
    struct Loader {
        cb: smithay_clipboard::Clipboard,
        _window: Arc<winit::window::Window>,
    }

    impl Loader {
        fn load(&self) -> Result<String, String> {
            self.cb.load().map_err(|e| e.to_string())
        }
    }

    impl TextClipboard {
        /// None when `window` isn't a Wayland window or the read worker can't start.
        #[allow(unsafe_code)]
        pub fn new(window: Arc<winit::window::Window>) -> Option<Self> {
            use winit::raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle};
            let RawDisplayHandle::Wayland(d) = window.display_handle().ok()?.as_raw() else { return None };
            let display = d.display.as_ptr();
            // SAFETY: the display is winit's open `wl_display`, which outlives `window`; each
            // clipboard is held beside a clone of `window` and dropped before it. smithay-clipboard
            // makes its own event queue on the display and doesn't close it.
            let (cb, reader) = unsafe { (smithay_clipboard::Clipboard::new(display), smithay_clipboard::Clipboard::new(display)) };
            let loader = Loader { cb: reader, _window: window.clone() };
            // `loader.load()`, not `loader.cb.load()`: a closure moves only the fields it names,
            // and the worker must hold the window as long as its clipboard.
            // Dropping doesn't wait for an unanswered read: only arboard's X11 handles hand what the
            // app copied on when the last one goes.
            let loads = Loads::spawn("wayland-clipboard-read", Duration::ZERO, move |()| loader.load())?;
            Some(Self { cb, reads: TextReads::new(loads), _window: window })
        }

        pub fn get_text(&mut self) -> Result<String, String> {
            self.reads.get(LOAD_WAIT)
        }

        pub fn set_text(&self, text: &str) {
            self.cb.store(text);
        }

        /// The read that got no answer in its wait, when a paste of one of `mimes` can take its
        /// answer ([`TextReads::unanswered`]).
        pub(super) fn unanswered(&mut self, mimes: &[&'static str]) -> Option<Unanswered> {
            self.reads.unanswered(mimes)
        }

        /// Drops the read that got no answer in its wait: a paste no longer takes its answer.
        pub fn forget_unanswered(&mut self) {
            self.reads.unanswered = None;
        }
    }
}

/// The macOS pasteboard's vector art and bitmaps as they are (arboard reads text, and of the
/// pictures TIFF only, decoded).
#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSArray, NSString};

    use super::{Flavour, pasteboard_types};

    /// Does the pasteboard hold one of `mimes` ([`pasteboard_types`])? Asks for its types only.
    pub fn has(mimes: &[&'static str]) -> bool {
        autoreleasepool(|_| {
            let types: Vec<_> = mimes.iter().flat_map(|m| pasteboard_types(m)).map(|(t, _)| NSString::from_str(t)).collect();
            let types: Vec<&NSString> = types.iter().map(|t| &**t).collect();
            !types.is_empty() && NSPasteboard::generalPasteboard().availableTypeFromArray(&NSArray::from_slice(&types)).is_some()
        })
    }

    /// The pasteboard's data for `mime`, the first of its types held ([`pasteboard_types`]).
    pub fn read(mime: &str) -> Option<Flavour> {
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard_types(mime).iter().find_map(|&(t, mime)| {
                let data = pasteboard.dataForType(&NSString::from_str(t))?.to_vec();
                (!data.is_empty()).then_some(Flavour { mime, data })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::cmd::clipboard::{EMF, PASTE_ORDER};

    use super::*;

    fn dir(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new().prefix(&format!("vectorcraft-clipboard-{name}-")).tempdir().unwrap()
    }

    #[test]
    fn test_directory_is_removed_when_its_handle_is_dropped() {
        let dir = dir("cleanup");
        let path = dir.path().to_path_buf();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists());
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::new(w, h).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn the_first_copied_file_that_is_art_pastes() {
        let d = dir("art");
        let (notes, red, art) = (d.path().join("notes.txt"), d.path().join("red image.png"), d.path().join("art.svg"));
        std::fs::write(&notes, "not art").unwrap();
        std::fs::write(&red, png(3, 2)).unwrap();
        std::fs::write(&art, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        // Missing files, folders and text are skipped.
        let f = copied_file(vec![d.path().join("gone.png"), d.path().to_path_buf(), notes.clone(), red.clone(), art.clone()], &PASTE_ORDER).unwrap();
        assert_eq!((f.mime, f.data), (PNG, png(3, 2)));
        assert_eq!(copied_file(vec![art.clone(), red.clone()], &PASTE_ORDER).map(|f| f.mime), Some(SVG));
        // Only what Paste asks for.
        assert_eq!(copied_file(vec![art, red.clone()], &[BITMAP]).map(|f| f.mime), Some(PNG));
        assert!(copied_file(vec![red], &[TEXT]).is_none());
        assert!(copied_file(vec![notes], &PASTE_ORDER).is_none());
        assert!(copied_file(vec![], &PASTE_ORDER).is_none());
    }

    /// What another app copied as vector art is asked for under the names the platforms give it,
    /// and read as the flavour the paste imports (#1499).
    #[test]
    fn vector_art_is_read_under_each_platforms_names() {
        let mac = |m| pasteboard_types(m).iter().map(|t| t.0).collect::<Vec<_>>();
        assert_eq!(mac(PDF), ["com.adobe.pdf", "Apple PDF pasteboard type"]);
        assert_eq!(mac(EPS), ["com.adobe.encapsulated-postscript"]);
        assert_eq!(mac(SVG), ["public.svg-image"]);
        assert_eq!(pasteboard_types(BITMAP), [("public.png", PNG), ("public.tiff", "image/tiff")]);
        assert!(windows_formats(EPS).contains(&"Encapsulated PostScript") && windows_formats(PDF).contains(&"Portable Document Format"));
        for asked in PASTE_ORDER {
            // Each type's data is the flavour asked for (a bitmap its own type): what
            // `clipboard.import*` then reads. Text and metafiles come another way.
            assert!(pasteboard_types(asked).iter().all(|(_, mime)| *mime == asked || asked == BITMAP), "{asked}");
            assert_eq!(pasteboard_types(asked).is_empty(), [TEXT, EMF].contains(&asked), "{asked}");
        }
        // Vector art before text, text before bitmaps.
        let at = |m| PASTE_ORDER.iter().position(|x| *x == m).unwrap();
        assert!(at(PDF) < at(EPS) && at(EPS) < at(TEXT) && at(TEXT) < at(BITMAP));
    }

    /// An .eps file copied in a file manager pastes as EPS.
    #[test]
    fn a_copied_eps_file_pastes() {
        let d = dir("eps");
        let eps = d.path().join("equation.eps");
        std::fs::write(&eps, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n0 0 10 10 rectfill\n").unwrap();
        assert_eq!(copied_file(vec![eps.clone()], &PASTE_ORDER).map(|f| f.mime), Some(EPS));
        assert!(copied_file(vec![eps], &[BITMAP, TEXT]).is_none());
    }

    #[test]
    fn a_crlf_left_by_the_uri_list_is_ignored() {
        let d = dir("crlf");
        let red = d.path().join("red.png");
        std::fs::write(&red, png(2, 2)).unwrap();
        let with_cr = PathBuf::from(format!("{}\r", red.display()));
        assert_eq!(copied_file(vec![with_cr], &PASTE_ORDER).map(|f| f.mime), Some(PNG));
    }

    /// A DIB of `w`×`h` pixels (top row first when `h` is negative): a `header`-byte header (40,
    /// or 124 for a BITMAPV5HEADER with BGRA masks), `extra` (the masks after a 40-byte header, a
    /// colour table), then `rows`, each padded to 4 bytes.
    fn dib(header: u32, w: i32, h: i32, bits: u16, compression: u32, extra: &[u8], rows: &[u8]) -> Vec<u8> {
        let mut d = vec![];
        d.extend(header.to_le_bytes());
        d.extend(w.to_le_bytes());
        d.extend(h.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(bits.to_le_bytes());
        d.extend(compression.to_le_bytes());
        d.extend((rows.len() as u32).to_le_bytes());
        // 96 ppi, every colour of the table used.
        d.extend([3780u32, 3780, 0, 0].iter().flat_map(|v| v.to_le_bytes()));
        if header == 124 {
            d.extend([0x00FF_0000u32, 0xFF00, 0xFF, 0xFF00_0000, 0x7352_4742].iter().flat_map(|v| v.to_le_bytes()));
            d.resize(124, 0);
        }
        d.extend_from_slice(extra);
        d.extend_from_slice(rows);
        d
    }

    /// 32-bit BGRA rows of `px` (top row first).
    fn bgra(px: &[[u8; 4]]) -> Vec<u8> {
        px.iter().flat_map(|[r, g, b, a]| [*b, *g, *r, *a]).collect()
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn pixels(img: &image::RgbaImage) -> Vec<[u8; 4]> {
        img.pixels().map(|p| p.0).collect()
    }

    #[test]
    #[cfg(windows)]
    fn dib_fallback_captures_both_formats_before_decoding() {
        use clipboard_win::formats::{CF_DIB, CF_DIBV5};

        let valid = dib(40, 1, 1, 32, 0, &[], &bgra(&[RED]));
        let mut captured = Vec::new();
        let read = win::read_dibs(|id| {
            captured.push(id);
            match id {
                CF_DIBV5 => Some(vec![40, 0, 0]),
                CF_DIB => Some(valid.clone()),
                _ => None,
            }
        })
        .unwrap();
        assert_eq!(captured, [CF_DIBV5, CF_DIB]);
        let flavour = read.flavour().unwrap();
        assert_eq!(flavour.mime, PNG);
        assert_eq!(pixels(&decode_png(&flavour.data).unwrap()), [RED]);
    }

    #[test]
    fn a_screenshot_dib_pastes_opaque_and_upright() {
        // CF_DIB as screenshots copy it: 32-bit BI_RGB, bottom row first, the fourth byte zero.
        let rows = bgra(&[[0, 0, 255, 0], [0, 0, 255, 0], [255, 0, 0, 0], [255, 0, 0, 0]]);
        let img = dib_image(&dib(40, 2, 2, 32, 0, &[], &rows)).unwrap();
        assert_eq!(pixels(&img), [RED, RED, BLUE, BLUE]);
        // The same bitmap with an alpha mask (CF_DIBV5), alpha still zero throughout: opaque too.
        let img = dib_image(&dib(124, 2, -2, 32, 3, &[], &bgra(&[[255, 0, 0, 0]; 4]))).unwrap();
        assert_eq!(pixels(&img), [RED; 4]);
    }

    #[test]
    fn a_dibv5_keeps_its_alpha_top_down() {
        // A negative height: top row first.
        let rows = bgra(&[[255, 0, 0, 128], [255, 0, 0, 128], [0, 0, 255, 0], [0, 0, 255, 255]]);
        let img = dib_image(&dib(124, 2, -2, 32, 3, &[], &rows)).unwrap();
        assert_eq!(pixels(&img), [[255, 0, 0, 128], [255, 0, 0, 128], [0, 0, 255, 0], BLUE]);
    }

    #[test]
    fn masks_and_colour_tables_come_before_the_pixels() {
        // BI_BITFIELDS after a 40-byte header: three masks between it and the pixels.
        let masks: Vec<u8> = [0x00FF_0000u32, 0xFF00, 0xFF].iter().flat_map(|v| v.to_le_bytes()).collect();
        let img = dib_image(&dib(40, 1, 1, 32, 3, &masks, &bgra(&[[0, 0, 255, 0]]))).unwrap();
        assert_eq!(pixels(&img), [BLUE]);
        // 8-bit, two colours used, rows padded to 4 bytes, bottom row first.
        let mut table = bgra(&[RED, BLUE]);
        table[3] = 0;
        let mut d = dib(40, 3, 2, 8, 0, &table, &[1, 1, 1, 0, 0, 1, 0, 0]);
        d[32..36].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(pixels(&dib_image(&d).unwrap()), [RED, BLUE, RED, BLUE, BLUE, BLUE]);
        // 24-bit, rows padded.
        let img = dib_image(&dib(40, 1, 2, 24, 0, &[], &[255, 0, 0, 0, 0, 0, 255, 0])).unwrap();
        assert_eq!(pixels(&img), [RED, BLUE]);
    }

    #[test]
    fn untrusted_dibs_are_refused_not_trusted() {
        let one = bgra(&[RED]);
        // Too large a side (the pixels aren't even there), cut short, a header or a colour count
        // out of range, no pixels.
        assert!(dib_image(&dib(40, MAX_SIDE as i32 + 1, 1, 32, 0, &[], &one)).is_none());
        assert!(dib_image(&dib(40, 1, -(MAX_SIDE as i32) - 1, 32, 0, &[], &one)).is_none());
        assert!(dib_image(&dib(40, 4096, 4096, 32, 0, &[], &one)).is_none());
        assert!(dib_image(&[40, 0, 0]).is_none());
        assert!(dib_image(&[]).is_none());
        let mut d = dib(40, 1, 1, 32, 0, &[], &one);
        d[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(dib_image(&d).is_none());
        let mut d = dib(40, 1, 1, 8, 0, &[], &[0; 4]);
        d[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(dib_file(&d).is_none());
        assert!(dib_image(&dib(40, 1, 1, 32, 0, &[], &[])).is_none());
        assert!(dib_image(&dib(40, 0, 0, 32, 0, &[], &[])).is_none());
    }
}
