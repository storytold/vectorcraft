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
//!   X11 either (a Flatpak on GNOME) to the compositor's core protocol on the window's connection:
//!   Copy offers SVG (`image/svg+xml`), PNG (`image/png`) and text together, and Paste reads copied
//!   files (`text/uri-list`), SVG, text and PNG. A clipboard that can't be opened is tried again
//!   after a wait that grows to a minute, not each time Paste asks. On Linux and the BSDs a read waits 2 s at most for the app
//!   that copied to send its data (a bitmap 10 s), and until that app answers, later reads fail at
//!   once; a paste then waits for that read's answer off the UI thread.
//! - Everywhere: files copied in a file manager paste as the first one that is art (SVG, PDF, EPS,
//!   a metafile or a bitmap), before the clipboard's other formats (the files' paths as text).

use std::io::{Cursor, Read};
use std::path::PathBuf;

use vectorcraft_engine::cmd::clipboard::{BITMAP, EPS, FILE_HEAD, Flavour, PDF, PNG, SVG, TEXT, file_flavour};
use vectorcraft_ui_egui::{ClipboardRead, SystemClipboard};

/// This platform's system clipboard. On Linux, `window`'s Wayland connection carries text, SVG and
/// PNG where arboard can't open (GNOME has no data-control protocol, a Flatpak there no X11).
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

/// The most URIs a copied file list may hold, VectorCraft's own limit: a longer list (untrusted
/// data) names no files.
const MAX_URIS: usize = 1024;

/// The local files a `text/uri-list` names, in order ([`file_uri_path`]). Each line holds one URI
/// and a line that starts with `#` is a comment (RFC 2483, section 5); lines end in CRLF or LF.
/// Lines that are not UTF-8 and URIs that don't name a local file are skipped. A list of more than
/// [`MAX_URIS`] URIs names none.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn uri_list_paths(list: &[u8]) -> Vec<PathBuf> {
    let uris: Vec<&[u8]> = list
        .split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .take(MAX_URIS + 1)
        .collect();
    if uris.len() > MAX_URIS {
        return vec![];
    }
    uris.into_iter().filter_map(|uri| file_uri_path(std::str::from_utf8(uri).ok()?)).collect()
}

/// The local files GNOME Files' `x-special/gnome-copied-files` names: a first line of `copy` or
/// `cut`, then one URI a line, read as [`uri_list_paths`] reads them. A paste places the files'
/// art either way and leaves the files where they are. No files when the first line is neither.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gnome_copied_paths(list: &[u8]) -> Vec<PathBuf> {
    let (action, uris) = match list.iter().position(|&b| b == b'\n') {
        Some(end) => (list.get(..end).unwrap_or_default(), list.get(end + 1..).unwrap_or_default()),
        None => (list, &[][..]),
    };
    match action.strip_suffix(b"\r").unwrap_or(action) {
        b"copy" | b"cut" => uri_list_paths(uris),
        _ => vec![],
    }
}

/// The absolute path a `file:` URI names on this machine: `file:/path`, `file:///path` or
/// `file://localhost/path` (RFC 8089, section 2; scheme and host in any letter case), its percent
/// escapes decoded once (RFC 3986, section 2.1) and a fragment (`#…`) left out. None for another
/// scheme, another host, a relative path, a query (`?…`), a bad escape, an escaped NUL or a path
/// that is not UTF-8 once decoded.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let uri = uri.trim_matches(|c: char| c.is_ascii_whitespace());
    let uri = uri.split_once('#').map_or(uri, |(before, _)| before);
    if uri.contains('?') {
        return None;
    }
    let scheme = uri.get(..5).filter(|s| s.eq_ignore_ascii_case("file:"))?;
    let rest = uri.get(scheme.len()..)?;
    let path = match rest.strip_prefix("//") {
        Some(authority_and_path) => {
            let slash = authority_and_path.find('/')?;
            let host = authority_and_path.get(..slash)?;
            if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
                return None;
            }
            authority_and_path.get(slash..)?
        }
        None => rest.starts_with('/').then_some(rest)?,
    };
    let mut bytes = Vec::with_capacity(path.len());
    let mut it = path.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = |d: Option<u8>| char::from(d?).to_digit(16);
            let (hi, lo) = (hex(it.next())?, hex(it.next())?);
            bytes.push(u8::try_from(hi * 16 + lo).ok()?);
        } else {
            bytes.push(b);
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
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
        /// A Copy through the Wayland core protocol, whose data source tells whether the
        /// clipboard still holds it.
        #[cfg(target_os = "linux")]
        Source,
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
        #[cfg(any(test, target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
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

    /// How long a paste's read of `request` after a late answer waits at `now`: as long as any read
    /// of `request`, or until [`IMAGE_WAIT`] after the unanswered read was `sent` when that is later.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
    fn follow_up_wait(request: Request, sent: Instant, now: Instant) -> Duration {
        wait_for(request).max((sent + IMAGE_WAIT).saturating_duration_since(now))
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
        /// [`IMAGE_WAIT`] after it was sent, as long as a paste's bitmap read waits. Once it comes,
        /// the same read is made again, since the clipboard may have changed hands meanwhile.
        /// `None`, so the paste takes the internal clipboard, when that answer doesn't come or the
        /// new one is what `ours` put on the clipboard. Otherwise the clipboard is read as
        /// [`Clipboard::read_now_or_later`] and the paste read it, the new answer standing in for
        /// the read it answers: copied files, else
        /// text, else the bitmap, and the bitmap when it is asked for and the text is blank or only
        /// names a picture (`adopt_system_clipboard_or_defer` in `vectorcraft-ui-egui`). Those reads
        /// wait as [`follow_up_wait`] says.
        fn paste(self, ours: Option<&Ours>, mimes: &[&'static str]) -> Option<Flavour> {
            let wait = (self.sent + IMAGE_WAIT).saturating_duration_since(Instant::now());
            super::loads::wait_for_answer(&self.answer, wait).ok()?;
            let fresh = |request: Request| {
                self.reads
                    .ask(request)
                    .and_then(|a| super::loads::wait_for_answer(&a, follow_up_wait(request, self.sent, Instant::now())))
                    .unwrap_or_else(|why| Answer::failed(request, why))
            };
            // The late answer may come from an app that lost the clipboard while it was awaited.
            let answer = fresh(self.request);
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
                None => fresh(request),
            };
            if let Some(f) = copied_file(read(Request::Files).files(), mimes) {
                return Some(f);
            }
            let text = if mimes.contains(&TEXT) { read(Request::Text).text().ok().filter(|t| !t.is_empty()) } else { None };
            let bitmap = |read: &mut dyn FnMut(Request) -> Answer| image_flavour(read(Request::Image).image().ok()?);
            match text {
                Some(t) if mimes.contains(&BITMAP) && (t.trim().is_empty() || vectorcraft_engine::cmd::clipboard::names_a_picture(&t)) => {
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
        /// core protocol's answer is waited for as long, until [`IMAGE_WAIT`] after its read was
        /// sent; then the clipboard is read again and pasted as at once, unless it holds the Copy
        /// made here ([`super::wayland::Unanswered::paste`]).
        fn paste(self, ours: Option<&Ours>, mimes: &[&'static str]) -> Option<Flavour> {
            match self {
                Self::Arboard(unanswered) => unanswered.paste(ours, mimes),
                #[cfg(target_os = "linux")]
                Self::Wayland(unanswered) => unanswered.paste(mimes, IMAGE_WAIT),
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
        /// The compositor's core data-device protocol on the window's connection.
        #[cfg(target_os = "linux")]
        Wayland(super::wayland::DataClipboard),
    }

    /// What a bitmap read or write says on the core protocol, which carries pictures as PNG data
    /// only ([`Clipboard::write`], [`Clipboard::read_now_or_later`]).
    #[cfg(target_os = "linux")]
    const PNG_ONLY: &str = "this clipboard carries pictures as PNG data";

    impl Backend {
        /// arboard (Wayland data-control, else X11), else the core protocol on `window`'s
        /// Wayland connection. Err: why each failed.
        fn open(window: Option<&Arc<winit::window::Window>>) -> Result<Self, String> {
            first_open(
                || Arboard::open().map(Self::Arboard),
                || {
                    #[cfg(target_os = "linux")]
                    return window
                        .and_then(|w| super::wayland::DataClipboard::new(w.clone()))
                        .map(Self::Wayland)
                        .ok_or("not a Wayland window with a seat and a data-device manager");
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
                Self::Wayland(_) => "the Wayland core protocol",
            }
        }

        fn get_text(&mut self) -> Result<String, String> {
            match self {
                Self::Arboard(cb) => cb.read(Request::Text).text(),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.text(),
            }
        }

        fn set_text(&mut self, text: &str) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.cb.set_text(text).map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.copy(&[Flavour { mime: TEXT, data: text.as_bytes().to_vec() }]),
            }
        }

        /// Whether the clipboard holds the core protocol's last Copy made here (never for arboard,
        /// whose writes are checked by what they wrote).
        #[cfg(target_os = "linux")]
        fn holds_source(&mut self) -> bool {
            match self {
                Self::Arboard(_) => false,
                Self::Wayland(cb) => cb.holds_ours(),
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
                Self::Wayland(_) => Err(PNG_ONLY.into()),
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
                Self::Wayland(_) => Err(PNG_ONLY.into()),
            }
        }

        fn set_image(&mut self, img: arboard::ImageData<'_>) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.cb.set_image(img).map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => Err(PNG_ONLY.into()),
            }
        }

        fn clear(&mut self) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.cb.clear().map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.copy(&[]),
            }
        }

        /// The read that got no answer in its wait, when a paste of one of `mimes` can take its
        /// answer ([`Arboard::unanswered`], [`Reads::unanswered`](super::wayland::Reads::unanswered)).
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

        /// Files copied in a file manager, read through arboard. The Wayland core protocol reads
        /// its file list with the rest of a paste
        /// ([`DataClipboard::read`](super::wayland::DataClipboard::read)).
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

        /// Text and copied files are read here, and work that reads arboard's image comes back. On
        /// the Wayland core protocol the copied files, else the first of `mimes` the clipboard
        /// offers, is read: text here, file lists, SVG and PNG by the work that comes back, which
        /// waits [`IMAGE_WAIT`] at most.
        fn read_now_or_later(&mut self, mimes: &[&'static str]) -> Option<ClipboardRead> {
            let cb = self.cb(Asker::User).ok()?;
            #[cfg(target_os = "linux")]
            if let Backend::Wayland(cb) = cb {
                return cb.read(mimes, IMAGE_WAIT);
            }
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
        /// arboard takes the text, else the PNG as a bitmap; the Wayland core protocol offers the
        /// SVG, PNG and text together ([`DataClipboard::copy`](super::wayland::DataClipboard::copy)).
        fn write(&mut self, flavours: &[Flavour]) -> Result<(), String> {
            let cb = self.cb(Asker::User)?;
            #[cfg(target_os = "linux")]
            if let Backend::Wayland(wl) = cb {
                wl.copy(flavours)?;
                // What a read got late was on the clipboard before this write.
                wl.forget_unanswered();
                self.ours = wl.holds_ours().then_some(Ours::Source);
                return Ok(());
            }
            let text = text_of(flavours);
            let image = match &text {
                Some(_) => None,
                None => flavours.iter().find(|f| f.mime == PNG).and_then(|f| decode_png(&f.data)),
            };
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
                #[cfg(target_os = "linux")]
                Some(Ours::Source) => cb.holds_source(),
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

        /// Text, on macOS vector art or a bitmap (by the pasteboard's types), and on the Wayland
        /// core protocol copied files, SVG, text or PNG (by the types offered, nothing read).
        /// arboard has no cheap way to ask for a bitmap (reading one costs as much as pasting it):
        /// the paste keys take it, the menu item waits for text.
        fn has(&mut self, mimes: &[&'static str]) -> bool {
            #[cfg(target_os = "macos")]
            if super::mac::has(mimes) {
                return true;
            }
            let Ok(cb) = self.cb(Asker::Probe) else { return false };
            #[cfg(target_os = "linux")]
            if let Backend::Wayland(wl) = cb {
                return wl.has(mimes);
            }
            mimes.contains(&TEXT) && cb.get_text().is_ok_and(|t| !t.is_empty())
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
            // A paste's reads after a late answer wait until 10 s after the unanswered read was
            // sent, and never less than any read of their kind.
            let sent = Instant::now();
            assert_eq!(follow_up_wait(Request::Text, sent, sent + Duration::from_secs(3)), Duration::from_secs(7));
            assert_eq!(follow_up_wait(Request::Files, sent, sent + Duration::from_secs(9)), super::super::loads::LOAD_WAIT);
            assert_eq!(follow_up_wait(Request::Image, sent, sent + Duration::from_secs(20)), IMAGE_WAIT);
            // X11 waits for an unanswered read when the clipboard closes; data-control doesn't.
            assert_eq!(finish(false), X11_READ_LIMIT);
            assert_eq!(finish(true), Duration::ZERO);
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd", target_os = "openbsd"))]
        #[test]
        fn quitting_waits_for_the_paste_checks_handle_as_long_as_its_reads_last() {
            // The Paste check's handle is one of these: when the app quits, its unanswered X11 read
            // ends `X11_READ_LIMIT` after it was sent, and an idle worker stops within `LOAD_WAIT`.
            // Quitting waits for that handle until both are over.
            assert!(vectorcraft_ui_egui::CLIPBOARD_PROBE_STOP_WAIT >= finish(false) + super::super::loads::LOAD_WAIT);
            assert!(vectorcraft_ui_egui::CLIPBOARD_PROBE_STOP_WAIT >= finish(true) + super::super::loads::LOAD_WAIT);
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

            // A paste of text alone pastes text that names a picture, not the picture.
            let (mut cb, release) = slow_owner(|request| match request {
                Request::Text => Answer::Text(Ok("<img src=\"art.png\">".into())),
                Request::Image => Answer::Image(Ok(arboard::ImageData { width: 3, height: 1, bytes: Cow::Owned(vec![0; 12]) })),
                Request::Files => Answer::Files(vec![]),
            });
            assert!(cb.read_waiting(Request::Text, Duration::ZERO).text().is_err());
            let unanswered = cb.unanswered(&[TEXT]).expect("the text read is kept for the paste");
            let paste = std::thread::spawn(move || unanswered.paste(None, &[TEXT]));
            drop(release);
            let pasted = paste.join().ok().flatten().expect("the text is pasted");
            assert_eq!((pasted.mime, pasted.data), (TEXT, b"<img src=\"art.png\">".to_vec()));

            // The app that answers late may have lost the clipboard meanwhile: the paste reads it
            // again and takes the new owner's text.
            let texts_read = std::sync::atomic::AtomicUsize::new(0);
            let (mut cb, release) = slow_owner(move |request| match request {
                Request::Text if texts_read.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 => Answer::Text(Ok("old owner".into())),
                Request::Text => Answer::Text(Ok("new owner".into())),
                _ => Answer::failed(request, "none".into()),
            });
            assert!(cb.read_waiting(Request::Text, Duration::ZERO).text().is_err());
            let unanswered = cb.unanswered(&[TEXT]).expect("the text read is kept for the paste");
            let paste = std::thread::spawn(move || unanswered.paste(None, &[TEXT]));
            drop(release);
            assert_eq!(paste.join().ok().flatten().map(|f| f.data), Some(b"new owner".to_vec()));

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
            use std::sync::{Arc, Mutex};

            use super::super::wayland::{Reads, Seen};
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;

            /// Core protocol reads of a clipboard offering `types`, whose owner answers each read
            /// of a text type with `text` and of `image/png` with `png`, only once the sender
            /// returned is used or dropped.
            fn slow_owner(types: &[&str], text: &'static str, png: &'static [u8]) -> (Reads, Arc<Mutex<Seen>>, std::sync::mpsc::Sender<()>) {
                let (release, gate) = std::sync::mpsc::channel::<()>();
                let loads = Loads::spawn("test", Duration::ZERO, move |wire: String| {
                    let _ = gate.recv();
                    Ok(if wire == PNG { png.to_vec() } else { text.as_bytes().to_vec() })
                })
                .unwrap();
                let seen = Arc::new(Mutex::new(Seen::offering(types)));
                (Reads::new(loads, seen.clone()), seen, release)
            }
            // The clipboard's owner answers the check with `text` late; `copied_here` when the
            // clipboard holds the Copy made here meanwhile.
            let paste_after_check = |text: &'static str, copied_here: bool| {
                let (mut reads, seen, release) = slow_owner(&["text/plain;charset=utf-8", PNG], text, b"\x89PNG");
                // The check whether the clipboard still holds what was copied here gets no answer
                // in its wait (2 s on the UI thread, none here).
                assert!(reads.text(Duration::ZERO).is_err());
                // The paste's own read fails at once while the owner hasn't answered.
                assert!(reads.text(Duration::from_secs(600)).is_err());
                // A paste of a bitmap alone can't take text: the read stays kept.
                assert!(reads.unanswered(&[BITMAP]).is_none());
                let late = Late::Wayland(reads.unanswered(&PASTE_ORDER).expect("the check's read is kept for the paste"));
                if copied_here {
                    let mut seen = seen.lock().unwrap();
                    seen.copy(vec!["text/plain;charset=utf-8", PNG]);
                    seen.selection(vec!["text/plain;charset=utf-8".into(), PNG.into()]);
                }
                let paste = std::thread::spawn(move || late.paste(None, &PASTE_ORDER));
                drop(release);
                paste.join().ok().flatten()
            };
            let pasted = paste_after_check("copied elsewhere", false).expect("the other app's text is pasted");
            assert_eq!((pasted.mime, pasted.data), (TEXT, b"copied elsewhere".to_vec()));
            // Text that only names a picture goes with it: the picture is pasted, as at once.
            let pasted = paste_after_check("https://example.com/art.png", false).expect("the picture is pasted");
            assert_eq!((pasted.mime, pasted.data), (PNG, b"\x89PNG".to_vec()));
            // The Copy made here, by its data source: the paste takes the internal clipboard.
            assert!(paste_after_check("copied elsewhere", true).is_none());
            // Empty text is not pasted: the picture beside it is.
            assert_eq!(paste_after_check("", false).map(|f| f.mime), Some(PNG));

            // An answer that came before the paste may be old: it is dropped, not pasted.
            let (mut reads, _, release) = slow_owner(&["text/plain"], "old", b"");
            assert!(reads.text(Duration::ZERO).is_err());
            drop(release);
            while reads.waiting() {
                std::thread::yield_now();
            }
            assert!(reads.unanswered(&[TEXT]).is_none());

            // The app that answers late may have lost the clipboard meanwhile: the paste reads it
            // again and takes the new owner's text.
            let (release, gate) = std::sync::mpsc::channel::<()>();
            let mut texts_read = 0;
            let loads = Loads::spawn("test", Duration::ZERO, move |_: String| {
                let _ = gate.recv();
                texts_read += 1;
                Ok(if texts_read == 1 { "old owner" } else { "new owner" }.as_bytes().to_vec())
            })
            .unwrap();
            let mut reads = Reads::new(loads, Arc::new(Mutex::new(Seen::offering(&["UTF8_STRING"]))));
            assert!(reads.text(Duration::ZERO).is_err());
            let late = Late::Wayland(reads.unanswered(&[TEXT]).expect("the read is kept for the paste"));
            let paste = std::thread::spawn(move || late.paste(None, &[TEXT]));
            drop(release);
            assert_eq!(paste.join().ok().flatten().map(|f| f.data), Some(b"new owner".to_vec()));
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
/// a Wayland session). A Copy offers its SVG as `image/svg+xml`, its PNG as `image/png` and its
/// text under each of [`TEXT_TYPES`]; Paste reads the files a file manager copied
/// ([`FILE_LISTS`]), then the clipboard's SVG, text and PNG, and the paste check looks at the types
/// offered without reading any data. The protocol gives the clipboard to the window with the
/// keyboard focus only. A worker thread handles the protocol's events and starts a thread for each
/// paste of a Copy's data by another app ([`MAX_SENDS`] at once). Reads run on another worker and
/// take [`MAX_BYTES`] at most; text waits [`LOAD_WAIT`] at most, file lists, SVG and PNG as long as
/// a paste's bitmap from arboard. A read that gets no answer in its wait is kept for
/// a paste to wait for off the UI thread, as arboard's are. A read or send whose pipe moves no data
/// for [`IDLE_WAIT`] fails and closes the pipe, which frees its worker or thread.
#[cfg(target_os = "linux")]
mod wayland {
    use std::io::{ErrorKind, Read, Write};
    use std::os::fd::{AsFd, BorrowedFd};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
    use std::time::{Duration, Instant};

    use rustix::event::{PollFd, PollFlags, Timespec};
    use vectorcraft_engine::cmd::clipboard::names_a_picture;
    use vectorcraft_ui_egui::ClipboardRead;
    use wayland_client::backend::ObjectData;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_callback::{self, WlCallback};
    use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
    use wayland_client::protocol::wl_data_device_manager::{self, WlDataDeviceManager};
    use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
    use wayland_client::protocol::wl_data_source::{self, WlDataSource};
    use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
    use wayland_client::protocol::wl_pointer::{self, WlPointer};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::protocol::wl_seat::{self, WlSeat};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};

    use super::loads::{Asker, GONE, LOAD_WAIT, Loads, wait_for_answer};
    use super::{BITMAP, Flavour, PNG, PathBuf, SVG, TEXT, copied_file, gnome_copied_paths, uri_list_paths};

    /// The types text is offered under and read from, best first: `text/plain;charset=utf-8` (GTK
    /// and Qt apps), `UTF8_STRING` (X11 apps through XWayland) and `text/plain`.
    const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
    /// What reads the files a file list names.
    type PathsOf = fn(&[u8]) -> Vec<PathBuf>;
    /// The types files copied in a file manager are offered under, each with what reads its files,
    /// best first: `text/uri-list` (RFC 2483), and GNOME Files' own list, read only when the first
    /// names no file.
    const FILE_LISTS: [(&str, PathsOf); 2] = [("text/uri-list", uri_list_paths), ("x-special/gnome-copied-files", gnome_copied_paths)];
    /// The most bytes a read takes from the app that copied (untrusted data), VectorCraft's own
    /// limit: a read of more fails.
    const MAX_BYTES: usize = 64 << 20;
    /// The most pastes of a Copy's data that are sent at once; a paste beyond them gets nothing.
    const MAX_SENDS: usize = 8;
    /// The most types of an offer that are kept (untrusted data).
    const MAX_TYPES: usize = 256;
    /// How long a read or send waits for the other app to move a byte through the pipe before it
    /// fails, VectorCraft's own limit; the wait starts again after each byte moved.
    const IDLE_WAIT: Duration = LOAD_WAIT;
    /// The most bytes a read takes from the pipe at a time.
    const CHUNK: usize = 64 << 10;

    /// What a read gives: the bytes, or why there are none.
    type Got = Result<Vec<u8>, String>;

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The types a Copy of `flavours` offers, each with its data: SVG as `image/svg+xml`, PNG as
    /// `image/png` and text under each of [`TEXT_TYPES`]. Flavours with no data and other flavours
    /// (PDF) are not offered.
    fn copy_types(flavours: &[Flavour]) -> Vec<(&'static str, Arc<[u8]>)> {
        let mut out: Vec<(&'static str, Arc<[u8]>)> = vec![];
        for f in flavours.iter().filter(|f| !f.data.is_empty()) {
            let types: &[&'static str] = match f.mime {
                SVG => &[SVG],
                PNG => &[PNG],
                TEXT => &TEXT_TYPES,
                _ => &[],
            };
            let data: Arc<[u8]> = f.data.as_slice().into();
            for &t in types {
                if !out.iter().any(|(o, _)| *o == t) {
                    out.push((t, data.clone()));
                }
            }
        }
        out
    }

    /// The type among `offered` that a read of `mime` asks for, in any letter case: SVG as
    /// `image/svg+xml`, text as the first of [`TEXT_TYPES`] offered, a bitmap as `image/png`.
    fn wire_type<'a>(mime: &str, offered: &'a [String]) -> Option<&'a str> {
        let wanted: &[&str] = match mime {
            SVG => &[SVG],
            TEXT => &TEXT_TYPES,
            BITMAP | PNG => &[PNG],
            _ => &[],
        };
        wanted.iter().find_map(|w| offered.iter().find(|o| o.eq_ignore_ascii_case(w))).map(String::as_str)
    }

    /// The flavour a read of `mime` makes of `data`: SVG as it is, a bitmap as PNG, text with its
    /// line ends as `\n`. None when `data` is empty.
    fn flavour(mime: &'static str, data: Vec<u8>) -> Option<Flavour> {
        if data.is_empty() {
            return None;
        }
        Some(match mime {
            TEXT => Flavour { mime: TEXT, data: String::from_utf8_lossy(&data).replace("\r\n", "\n").replace('\r', "\n").into_bytes() },
            BITMAP | PNG => Flavour { mime: PNG, data },
            _ => Flavour { mime, data },
        })
    }

    /// What a paste of one of `mimes` takes from a clipboard offering `offered`, `read` reading the
    /// offer's type for each one tried: the first of `mimes` offered with data, except that text
    /// that is blank or only names a picture gives way to the PNG when `mimes` has a bitmap
    /// (`adopt_system_clipboard_or_defer` in `vectorcraft-ui-egui`).
    fn pick(offered: &[String], mimes: &[&'static str], mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<Flavour> {
        let mut take = |mime: &'static str| flavour(mime, read(wire_type(mime, offered)?)?);
        let first = mimes.iter().find_map(|&m| take(m))?;
        if first.mime == TEXT && mimes.contains(&BITMAP) {
            let text = String::from_utf8_lossy(&first.data);
            if text.trim().is_empty() || names_a_picture(&text) {
                return take(BITMAP).or(Some(first));
            }
        }
        Some(first)
    }

    /// Whether a paste of one of `mimes` can take a copied file: one of them is not text.
    fn takes_files(mimes: &[&'static str]) -> bool {
        mimes.iter().any(|&m| m != TEXT)
    }

    /// The first type among `offered` that lists copied files ([`FILE_LISTS`]), in any letter case.
    fn file_list_type(offered: &[String]) -> Option<&str> {
        FILE_LISTS.iter().find_map(|(list, _)| offered.iter().find(|o| o.eq_ignore_ascii_case(list))).map(String::as_str)
    }

    /// The first file a file manager copied that pastes as one of `mimes` ([`copied_file`]), from
    /// the first of [`FILE_LISTS`] among `offered` that names a file, `read` reading each list.
    fn copied(offered: &[String], mimes: &[&'static str], mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<Flavour> {
        if !takes_files(mimes) {
            return None;
        }
        let paths = FILE_LISTS.iter().find_map(|(list, paths_of)| {
            let wire = offered.iter().find(|o| o.eq_ignore_ascii_case(list))?;
            Some(paths_of(&read(wire)?)).filter(|paths| !paths.is_empty())
        })?;
        copied_file(paths, mimes)
    }

    /// What a paste of one of `mimes` takes: the first copied file that is art ([`copied`]), else
    /// what [`pick`] takes.
    fn pick_pasted(offered: &[String], mimes: &[&'static str], mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<Flavour> {
        copied(offered, mimes, &mut read).or_else(|| pick(offered, mimes, read))
    }

    /// The time `idle` from now; None when that is past the clock's range (no limit).
    fn idle_until(idle: Duration) -> Option<Instant> {
        Instant::now().checked_add(idle)
    }

    /// Makes `fd` nonblocking, its other flags kept.
    fn nonblocking(fd: BorrowedFd<'_>) -> std::io::Result<()> {
        let flags = rustix::fs::fcntl_getfl(fd)?;
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
        Ok(())
    }

    /// Waits until `fd` is ready for `events`, closed or failed (true), or `until` passes (false);
    /// None waits without a limit. A wait a signal cuts short goes on until `until`.
    fn ready(fd: BorrowedFd<'_>, events: PollFlags, until: Option<Instant>) -> std::io::Result<bool> {
        loop {
            let left = until.map(|t| t.saturating_duration_since(Instant::now()));
            let timeout = left.and_then(|d| Timespec::try_from(d).ok());
            let mut fds = [PollFd::new(&fd, events)];
            match rustix::event::poll(&mut fds, timeout.as_ref()) {
                Ok(n) => return Ok(n > 0),
                Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// `from` read to its end, `cap` bytes at most: more is an error, as is a failed read and a
    /// pipe that is left open with no byte coming for `idle`. A read that fails gives none of the
    /// bytes it took, and closes `from`.
    fn read_capped(mut from: impl Read + AsFd, cap: usize, idle: Duration) -> Got {
        let failed = |e: std::io::Error| format!("couldn't read the clipboard's data: {e}");
        nonblocking(from.as_fd()).map_err(failed)?;
        let (mut data, mut chunk) = (vec![], vec![0u8; CHUNK]);
        let mut until = idle_until(idle);
        loop {
            match from.read(&mut chunk) {
                Ok(0) => return Ok(data),
                Ok(n) => {
                    let got = chunk.get(..n).ok_or_else(|| failed(ErrorKind::InvalidData.into()))?;
                    if data.len() + got.len() > cap {
                        return Err(format!("the clipboard's data is larger than {} MiB", cap >> 20));
                    }
                    data.extend_from_slice(got);
                    until = idle_until(idle);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if !ready(from.as_fd(), PollFlags::IN, until).map_err(failed)? {
                        return Err("the app that copied stopped sending the clipboard's data".to_string());
                    }
                }
                Err(e) => return Err(failed(e)),
            }
        }
    }

    /// `data` written to `to`: an error when the app that pastes closes its end (the process
    /// ignores `SIGPIPE`) or takes no byte for `idle`.
    fn send(mut to: impl Write + AsFd, data: &[u8], idle: Duration) -> std::io::Result<()> {
        nonblocking(to.as_fd())?;
        let (mut sent, mut until) = (0, idle_until(idle));
        while let Some(rest) = data.get(sent..).filter(|rest| !rest.is_empty()) {
            match to.write(rest) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => {
                    sent += n;
                    until = idle_until(idle);
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    if !ready(to.as_fd(), PollFlags::OUT, until)? {
                        return Err(ErrorKind::TimedOut.into());
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// The threads that send a Copy's data to the apps that paste it, counted.
    #[derive(Clone)]
    struct Sends {
        running: Arc<AtomicUsize>,
        /// How long a send waits for the app that pastes to take a byte.
        idle: Duration,
    }

    impl Default for Sends {
        fn default() -> Self {
            Self { running: Arc::default(), idle: IDLE_WAIT }
        }
    }

    impl Sends {
        /// `data` written to `to` on a thread of its own ([`send`]), which then closes `to`; an app
        /// that closes its end early or takes no byte for the send's idle wait ends the write.
        /// While [`MAX_SENDS`] are being written `to` is closed at once with nothing written → false.
        fn start(&self, to: impl Write + AsFd + Send + 'static, data: Arc<[u8]>) -> bool {
            if self.running.fetch_add(1, Ordering::SeqCst) >= MAX_SENDS {
                self.running.fetch_sub(1, Ordering::SeqCst);
                return false;
            }
            let (count, idle) = (self.running.clone(), self.idle);
            let work = move || {
                let _ = send(to, &data, idle);
                count.fetch_sub(1, Ordering::SeqCst);
            };
            if std::thread::Builder::new().name("wayland-clipboard-send".into()).spawn(work).is_err() {
                self.running.fetch_sub(1, Ordering::SeqCst);
                return false;
            }
            true
        }

        /// How many are being written.
        #[cfg(test)]
        fn running(&self) -> usize {
            self.running.load(Ordering::SeqCst)
        }
    }

    /// What the protocol worker has seen of the clipboard.
    #[derive(Default)]
    pub(super) struct Seen {
        /// The types of the clipboard's offer, none when it is empty.
        offered: Vec<String>,
        /// The types of this connection's last Copy, until its data source is canceled.
        copied: Option<Vec<&'static str>>,
        /// Whether the clipboard holds that Copy.
        ours: bool,
    }

    impl Seen {
        /// A clipboard offering `types`, copied elsewhere.
        #[cfg(test)]
        pub(super) fn offering(types: &[&str]) -> Self {
            Self { offered: types.iter().map(|t| t.to_string()).collect(), ..Self::default() }
        }

        /// A Copy of `types` (none: the clipboard emptied) was given the clipboard.
        pub(super) fn copy(&mut self, types: Vec<&'static str>) {
            self.ours = !types.is_empty();
            self.copied = Some(types).filter(|t| !t.is_empty());
        }

        /// The clipboard's offer has `offered` now. It holds the last Copy while that Copy's data
        /// source is not canceled and the offer has the Copy's types, no more and no fewer, as the
        /// compositor offers a source's types. An offer made before the Copy and handled after it
        /// makes the Copy not ours until the Copy's own offer comes.
        pub(super) fn selection(&mut self, offered: Vec<String>) {
            self.ours = self.copied.as_ref().is_some_and(|c| c.len() == offered.len() && c.iter().all(|t| offered.iter().any(|o| o == t)));
            self.offered = offered;
        }

        /// The last Copy's data source was canceled: another app holds the clipboard.
        pub(super) fn cancelled(&mut self) {
            self.copied = None;
            self.ours = false;
        }
    }

    /// Reads of the clipboard's offer on a worker, and the last one that got no answer in its wait.
    pub(super) struct Reads {
        /// Each request is the offer's type to read.
        loads: Loads<String, Got>,
        seen: Arc<Mutex<Seen>>,
        /// The last text read that got no answer in its wait, until a paste takes it or the next
        /// read is sent.
        unanswered: Option<Unanswered>,
    }

    /// A text read that got no answer in its wait.
    pub(super) struct Unanswered {
        answer: mpsc::Receiver<Got>,
        sent: Instant,
        /// The worker, for the reads made once the answer comes.
        reads: Asker<String, Got>,
        seen: Arc<Mutex<Seen>>,
    }

    impl Unanswered {
        /// What a paste of one of `mimes` takes once this read's answer comes, waiting for it until
        /// `limit` after the read was sent: the clipboard read again as [`pick_pasted`] reads it,
        /// each read waiting as long, [`LOAD_WAIT`] at least, since the app that answers late may
        /// have lost the clipboard meanwhile. None when the answer doesn't come or the clipboard holds this
        /// connection's Copy.
        pub(super) fn paste(self, mimes: &[&'static str], limit: Duration) -> Option<Flavour> {
            let left = |now: Instant| (self.sent + limit).saturating_duration_since(now);
            let _late = wait_for_answer(&self.answer, left(Instant::now())).ok()?;
            let offered = {
                let seen = lock(&self.seen);
                if seen.ours {
                    return None;
                }
                seen.offered.clone()
            };
            pick_pasted(&offered, mimes, |wire| {
                wait_for_answer(&self.reads.ask(wire.to_string()).ok()?, LOAD_WAIT.max(left(Instant::now()))).ok()?.ok()
            })
        }
    }

    impl Reads {
        pub(super) fn new(loads: Loads<String, Got>, seen: Arc<Mutex<Seen>>) -> Self {
            Self { loads, seen, unanswered: None }
        }

        /// The offer's type for `mime`, when it has one.
        fn wire(&self, mime: &str) -> Option<String> {
            wire_type(mime, &lock(&self.seen).offered).map(str::to_string)
        }

        /// The text, waiting `wait` at most; empty when the clipboard has none. A read that gets no
        /// answer in that time is kept ([`Self::unanswered`]) for a paste to wait for off the UI
        /// thread; a read sent means the one kept has been answered, and its answer is dropped.
        pub(super) fn text(&mut self, wait: Duration) -> Result<String, String> {
            let Some(wire) = self.wire(TEXT) else { return Ok(String::new()) };
            let sent = Instant::now();
            let answer = self.loads.ask(wire)?;
            self.unanswered = None;
            match wait_for_answer(&answer, wait) {
                Ok(got) => Ok(flavour(TEXT, got?).map(|f| String::from_utf8_lossy(&f.data).into_owned()).unwrap_or_default()),
                Err(why) => {
                    if why != GONE {
                        self.unanswered = Some(Unanswered { answer, sent, reads: self.loads.asker(), seen: self.seen.clone() });
                    }
                    Err(why)
                }
            }
        }

        /// What a paste of one of `mimes` reads: the files a file manager copied, else the first of
        /// `mimes` the clipboard offers. A file list is asked for here and read, with the files,
        /// by the work that comes back, which pastes the first file that is art ([`copied`]) and
        /// else reads `mimes` as [`pick`] does. Text is read here, [`LOAD_WAIT`] at most, and
        /// pasted unless it is empty; SVG and PNG are asked for here and waited for, `wait` at most,
        /// by the work that comes back. When that read fails or is empty, the work reads the rest
        /// of `mimes` as [`pick`] does. Each read the work makes after the first waits until `wait`
        /// after the first was sent, [`LOAD_WAIT`] at least.
        pub(super) fn read(&mut self, mimes: &[&'static str], wait: Duration) -> Option<ClipboardRead> {
            if takes_files(mimes) {
                let offered = lock(&self.seen).offered.clone();
                if let Some(list) = file_list_type(&offered).map(str::to_string) {
                    let sent = Instant::now();
                    let answer = self.loads.ask(list.clone()).ok()?;
                    self.unanswered = None;
                    let (reads, mimes) = (self.loads.asker(), mimes.to_vec());
                    return Some(ClipboardRead::Later(Box::new(move || {
                        let left = || LOAD_WAIT.max((sent + wait).saturating_duration_since(Instant::now()));
                        // The list asked for here is read from its answer, every other type anew.
                        let mut first = Some((list, answer));
                        pick_pasted(&offered, &mimes, |wire| {
                            let answer = match first.take_if(|(list, _)| list.as_str() == wire) {
                                Some((_, answer)) => answer,
                                None => reads.ask(wire.to_string()).ok()?,
                            };
                            wait_for_answer(&answer, left()).ok()?.ok()
                        })
                    })));
                }
            }
            mimes.iter().enumerate().find_map(|(i, &mime)| {
                let wire = self.wire(mime)?;
                if mime == TEXT {
                    let text = self.text(LOAD_WAIT).ok().filter(|t| !t.is_empty())?;
                    return Some(ClipboardRead::Now(Flavour { mime: TEXT, data: text.into_bytes() }));
                }
                let sent = Instant::now();
                let answer = self.loads.ask(wire).ok()?;
                self.unanswered = None;
                let (reads, seen, rest) = (self.loads.asker(), self.seen.clone(), mimes.get(i + 1..).unwrap_or_default().to_vec());
                Some(ClipboardRead::Later(Box::new(move || {
                    if let Some(f) = wait_for_answer(&answer, wait).ok().and_then(Result::ok).and_then(|data| flavour(mime, data)) {
                        return Some(f);
                    }
                    let offered = lock(&seen).offered.clone();
                    let left = || LOAD_WAIT.max((sent + wait).saturating_duration_since(Instant::now()));
                    pick(&offered, &rest, |wire| wait_for_answer(&reads.ask(wire.to_string()).ok()?, left()).ok()?.ok())
                })))
            })
        }

        /// Whether the clipboard offers one of `mimes`, or files a file manager copied when a paste
        /// of `mimes` can take a file; nothing is read.
        pub(super) fn has(&self, mimes: &[&'static str]) -> bool {
            let seen = lock(&self.seen);
            (takes_files(mimes) && file_list_type(&seen.offered).is_some()) || mimes.iter().any(|m| wire_type(m, &seen.offered).is_some())
        }

        /// Whether the clipboard holds this connection's last Copy.
        pub(super) fn holds_ours(&self) -> bool {
            lock(&self.seen).ours
        }

        /// The text read that got no answer in its wait, while its answer has still not come and a
        /// paste of one of `mimes` can take text. Once it has come the answer may be old, and it is
        /// dropped: a new read is made.
        pub(super) fn unanswered(&mut self, mimes: &[&'static str]) -> Option<Unanswered> {
            let unanswered = self.unanswered.take_if(|_| mimes.contains(&TEXT))?;
            self.loads.waiting().then_some(unanswered)
        }

        /// Drops the read that got no answer in its wait: a paste no longer takes its answer.
        pub(super) fn forget_unanswered(&mut self) {
            self.unanswered = None;
        }

        /// Whether a read sent is still unanswered.
        #[cfg(test)]
        pub(super) fn waiting(&self) -> bool {
            self.loads.waiting()
        }
    }

    /// The protocol objects of a [`DataClipboard`], kept up to date by its worker.
    #[derive(Default)]
    struct Objects {
        device: Option<WlDataDevice>,
        keyboard: Option<WlKeyboard>,
        pointer: Option<WlPointer>,
        /// The serial of the last key press, click or keyboard focus, which a Copy takes the
        /// clipboard with.
        serial: Option<u32>,
        /// The clipboard's offer.
        selection: Option<WlDataOffer>,
        /// The offer of a drag over the window.
        drag: Option<WlDataOffer>,
        /// The last Copy's data source.
        source: Option<WlDataSource>,
    }

    /// An offer's types, as its events give them.
    #[derive(Default)]
    struct OfferData(Mutex<Vec<String>>);

    /// A Copy's types with their data.
    struct SourceData(Vec<(&'static str, Arc<[u8]>)>);

    /// What the protocol worker's event handlers change.
    struct State {
        objects: Arc<Mutex<Objects>>,
        seen: Arc<Mutex<Seen>>,
        sends: Sends,
    }

    impl State {
        /// Lets go of every object, once the clipboard is dropped.
        fn close(&mut self, seat: &WlSeat) {
            let mut o = lock(&self.objects);
            for offer in [o.selection.take(), o.drag.take()].into_iter().flatten() {
                offer.destroy();
            }
            if let Some(source) = o.source.take() {
                source.destroy();
            }
            if let Some(device) = o.device.take().filter(|d| d.version() >= 2) {
                device.release();
            }
            if let Some(keyboard) = o.keyboard.take().filter(|k| k.version() >= 3) {
                keyboard.release();
            }
            if let Some(pointer) = o.pointer.take().filter(|p| p.version() >= 3) {
                pointer.release();
            }
            if seat.version() >= 5 {
                seat.release();
            }
        }
    }

    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlCallback, ()> for State {
        fn event(_: &mut Self, _: &WlCallback, _: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlDataDeviceManager, ()> for State {
        fn event(_: &mut Self, _: &WlDataDeviceManager, _: wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlSeat, ()> for State {
        fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
            let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event else { return };
            let mut o = lock(&state.objects);
            if !caps.contains(wl_seat::Capability::Keyboard) {
                if let Some(keyboard) = o.keyboard.take().filter(|k| k.version() >= 3) {
                    keyboard.release();
                }
            } else if o.keyboard.is_none() {
                o.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if !caps.contains(wl_seat::Capability::Pointer) {
                if let Some(pointer) = o.pointer.take().filter(|p| p.version() >= 3) {
                    pointer.release();
                }
            } else if o.pointer.is_none() {
                o.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }

    /// The serial a Copy takes the clipboard with after the keyboard event `event`, `serial` before
    /// it: a key or the keyboard focus coming in sets it, the focus leaving clears it, and other
    /// events leave it. A modifier change's serial is not an input event's, and the release of Ctrl
    /// after Ctrl+C comes before the Copy's data is ready.
    fn keyboard_serial(event: &wl_keyboard::Event, serial: Option<u32>) -> Option<u32> {
        match event {
            wl_keyboard::Event::Enter { serial, .. } | wl_keyboard::Event::Key { serial, .. } => Some(*serial),
            wl_keyboard::Event::Leave { .. } => None,
            _ => serial,
        }
    }

    impl Dispatch<WlKeyboard, ()> for State {
        fn event(state: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            let mut o = lock(&state.objects);
            o.serial = keyboard_serial(&event, o.serial);
        }
    }

    impl Dispatch<WlPointer, ()> for State {
        fn event(state: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let wl_pointer::Event::Button { serial, .. } = event {
                lock(&state.objects).serial = Some(serial);
            }
        }
    }

    impl Dispatch<WlDataDevice, ()> for State {
        fn event(state: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            let mut o = lock(&state.objects);
            match event {
                wl_data_device::Event::Selection { id } => {
                    let mut offered = id.as_ref().and_then(|offer| offer.data::<OfferData>()).map(|d| lock(&d.0).clone()).unwrap_or_default();
                    offered.dedup();
                    if let Some(old) = std::mem::replace(&mut o.selection, id) {
                        old.destroy();
                    }
                    lock(&state.seen).selection(offered);
                }
                wl_data_device::Event::Enter { id, .. } => {
                    if let Some(old) = std::mem::replace(&mut o.drag, id) {
                        old.destroy();
                    }
                }
                wl_data_device::Event::Leave | wl_data_device::Event::Drop => {
                    if let Some(old) = o.drag.take() {
                        old.destroy();
                    }
                }
                _ => {}
            }
        }

        /// The one event of a data device that makes an object is `data_offer`.
        fn event_created_child(_: u16, qh: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
            qh.make_data::<WlDataOffer, _>(OfferData::default())
        }
    }

    impl Dispatch<WlDataOffer, OfferData> for State {
        fn event(_: &mut Self, _: &WlDataOffer, event: wl_data_offer::Event, data: &OfferData, _: &Connection, _: &QueueHandle<Self>) {
            if let wl_data_offer::Event::Offer { mime_type } = event {
                let mut types = lock(&data.0);
                if types.len() < MAX_TYPES {
                    types.push(mime_type);
                }
            }
        }
    }

    impl Dispatch<WlDataSource, SourceData> for State {
        fn event(state: &mut Self, source: &WlDataSource, event: wl_data_source::Event, data: &SourceData, _: &Connection, _: &QueueHandle<Self>) {
            match event {
                // A type the Copy doesn't offer gets nothing: dropping `fd` closes it.
                wl_data_source::Event::Send { mime_type, fd } => {
                    if let Some((_, bytes)) = data.0.iter().find(|(t, _)| t.eq_ignore_ascii_case(&mime_type)) {
                        state.sends.start(std::fs::File::from(fd), bytes.clone());
                    }
                }
                wl_data_source::Event::Cancelled => {
                    source.destroy();
                    let mut o = lock(&state.objects);
                    if o.source.as_ref() == Some(source) {
                        o.source = None;
                        lock(&state.seen).cancelled();
                    }
                }
                _ => {}
            }
        }
    }

    /// Reads the clipboard's offer for the read worker.
    struct Receiver {
        conn: Connection,
        objects: Arc<Mutex<Objects>>,
        /// Held as long as the connection, which is the window's.
        _window: Arc<winit::window::Window>,
    }

    impl Receiver {
        /// The clipboard's data of type `wire`, read until the app that copied closes the pipe,
        /// [`MAX_BYTES`] at most; an error when that app sends no byte for [`IDLE_WAIT`].
        fn receive(&self, wire: String) -> Got {
            let offer = lock(&self.objects).selection.clone().ok_or("the clipboard is empty")?;
            let (from, to) = std::io::pipe().map_err(|e| format!("couldn't open a pipe for the clipboard's data: {e}"))?;
            offer.receive(wire, to.as_fd());
            // The request carries a copy of the write end: once it is sent, the app that copied
            // holds the only one, and the read ends when that app closes it.
            drop(to);
            self.conn.flush().map_err(|e| format!("couldn't reach the compositor: {e}"))?;
            read_capped(from, MAX_BYTES, IDLE_WAIT)
        }
    }

    pub struct DataClipboard {
        reads: Reads,
        objects: Arc<Mutex<Objects>>,
        manager: WlDataDeviceManager,
        conn: Connection,
        qh: QueueHandle<State>,
        /// Tells the protocol worker to stop.
        stop: Arc<AtomicBool>,
        /// Held as long as the connection, which is the window's.
        _window: Arc<winit::window::Window>,
    }

    impl DataClipboard {
        /// None when `window` isn't a Wayland window, the compositor has no data-device manager or
        /// seat, or a worker can't start.
        pub fn new(window: Arc<winit::window::Window>) -> Option<Self> {
            let conn = connection(&window)?;
            let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
            let qh = queue.handle();
            let manager: WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).ok()?;
            let seat: WlSeat = globals.bind(&qh, 1..=5, ()).ok()?;
            let (objects, seen) = (Arc::<Mutex<Objects>>::default(), Arc::<Mutex<Seen>>::default());
            lock(&objects).device = Some(manager.get_data_device(&seat, &qh, ()));
            let mut state = State { objects: objects.clone(), seen: seen.clone(), sends: Sends::default() };
            // The seat's keyboard and pointer, and the clipboard's offer when the window has focus.
            queue.roundtrip(&mut state).ok()?;
            let receiver = Receiver { conn: conn.clone(), objects: objects.clone(), _window: window.clone() };
            // Dropping doesn't wait for an unanswered read: only arboard's X11 handles hand what the
            // app copied on when the last one goes.
            let loads = Loads::spawn("wayland-clipboard-read", Duration::ZERO, move |wire| receiver.receive(wire))?;
            let stop = Arc::new(AtomicBool::new(false));
            let (stopped, held) = (stop.clone(), window.clone());
            let work = move || {
                while !stopped.load(Ordering::SeqCst) && queue.blocking_dispatch(&mut state).is_ok() {}
                state.close(&seat);
                let _ = queue.flush();
                drop(held);
            };
            std::thread::Builder::new().name("wayland-clipboard".into()).spawn(work).ok()?;
            Some(Self { reads: Reads::new(loads, seen), objects, manager, conn, qh, stop, _window: window })
        }

        /// Gives the clipboard to a Copy of `flavours` ([`copy_types`]); none empties it. Err when
        /// the seat has no data device or the window has had no input to take the clipboard with.
        pub fn copy(&mut self, flavours: &[Flavour]) -> Result<(), String> {
            let offered = copy_types(flavours);
            let mut o = lock(&self.objects);
            let device = o.device.clone().ok_or("the seat has no clipboard")?;
            let serial = o.serial.ok_or("the window has had no key press or click to copy with")?;
            let types: Vec<&'static str> = offered.iter().map(|(t, _)| *t).collect();
            let source = (!offered.is_empty()).then(|| {
                let source = self.manager.create_data_source(&self.qh, SourceData(offered));
                for t in &types {
                    source.offer(t.to_string());
                }
                source
            });
            device.set_selection(source.as_ref(), serial);
            if let Some(old) = std::mem::replace(&mut o.source, source) {
                old.destroy();
            }
            lock(&self.reads.seen).copy(types);
            drop(o);
            self.conn.flush().map_err(|e| format!("couldn't reach the compositor: {e}"))
        }

        pub fn text(&mut self) -> Result<String, String> {
            self.reads.text(LOAD_WAIT)
        }

        /// What a paste of one of `mimes` reads ([`Reads::read`]), SVG and PNG waited for `wait` at
        /// most.
        pub fn read(&mut self, mimes: &[&'static str], wait: Duration) -> Option<ClipboardRead> {
            self.reads.read(mimes, wait)
        }

        pub fn has(&self, mimes: &[&'static str]) -> bool {
            self.reads.has(mimes)
        }

        pub fn holds_ours(&self) -> bool {
            self.reads.holds_ours()
        }

        /// The read that got no answer in its wait, when a paste of one of `mimes` can take its
        /// answer ([`Reads::unanswered`]).
        pub(super) fn unanswered(&mut self, mimes: &[&'static str]) -> Option<Unanswered> {
            self.reads.unanswered(mimes)
        }

        /// Drops the read that got no answer in its wait: a paste no longer takes its answer.
        pub fn forget_unanswered(&mut self) {
            self.reads.forget_unanswered();
        }
    }

    impl Drop for DataClipboard {
        /// Stops the protocol worker, which a sync request's answer wakes.
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.conn.display().sync(&self.qh, ());
            let _ = self.conn.flush();
        }
    }

    /// A connection on `window`'s Wayland display.
    #[allow(unsafe_code)]
    fn connection(window: &winit::window::Window) -> Option<Connection> {
        use wayland_backend::sys::client::Backend;
        use winit::raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle};
        let RawDisplayHandle::Wayland(d) = window.display_handle().ok()?.as_raw() else { return None };
        // SAFETY: the display is winit's open `wl_display`, which outlives `window`; every user of
        // the connection holds a clone of `window`. A backend made from a foreign display does not
        // close it when dropped.
        let backend = unsafe { Backend::from_foreign_display(d.display.as_ptr().cast()) };
        Some(Connection::from_backend(backend))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn flavour_of(mime: &'static str, data: &[u8]) -> Flavour {
            Flavour { mime, data: data.to_vec() }
        }

        fn offered(types: &[&str]) -> Vec<String> {
            types.iter().map(|t| t.to_string()).collect()
        }

        #[test]
        fn a_copy_offers_its_svg_png_and_text_each_with_its_own_data() {
            let flavours = [flavour_of(TEXT, b"<svg/>"), flavour_of(SVG, b"<svg/>"), flavour_of(super::super::PDF, b"%PDF"), flavour_of(PNG, b"png")];
            let types: Vec<(&str, Vec<u8>)> = copy_types(&flavours).into_iter().map(|(t, d)| (t, d.to_vec())).collect();
            assert_eq!(
                types,
                [
                    ("text/plain;charset=utf-8", b"<svg/>".to_vec()),
                    ("UTF8_STRING", b"<svg/>".to_vec()),
                    ("text/plain", b"<svg/>".to_vec()),
                    (SVG, b"<svg/>".to_vec()),
                    (PNG, b"png".to_vec()),
                ]
            );
            // With Include SVG Code off the engine makes no SVG flavour, and none is offered; a
            // flavour with no data is not offered either.
            let types: Vec<&str> = copy_types(&[flavour_of(PNG, b"png"), flavour_of(TEXT, b"")]).into_iter().map(|(t, _)| t).collect();
            assert_eq!(types, [PNG]);
            assert!(copy_types(&[]).is_empty());
        }

        #[test]
        fn a_read_asks_for_the_offers_own_type() {
            let all = offered(&["text/plain", "TEXT", "UTF8_STRING", "Image/PNG", "image/svg+xml", "application/pdf"]);
            assert_eq!(wire_type(TEXT, &all), Some("UTF8_STRING"));
            assert_eq!(wire_type(TEXT, &offered(&["text/plain", "text/plain;charset=UTF-8"])), Some("text/plain;charset=UTF-8"));
            assert_eq!(wire_type(BITMAP, &all), Some("Image/PNG"));
            assert_eq!(wire_type(SVG, &all), Some("image/svg+xml"));
            // PDF, EPS and metafiles are not read here, nor types no reader knows.
            assert_eq!(wire_type(super::super::PDF, &all), None);
            assert_eq!(wire_type(TEXT, &offered(&["text/html", "image/jpeg"])), None);
            assert_eq!(wire_type(SVG, &[]), None);
        }

        /// What a paste of `mimes` takes from a clipboard holding `data` by type, and the types read.
        fn pasted(data: &[(&str, &[u8])], mimes: &[&'static str]) -> (Option<Flavour>, Vec<String>) {
            let types: Vec<String> = data.iter().map(|(t, _)| t.to_string()).collect();
            let mut read = vec![];
            let f = pick(&types, mimes, |wire| {
                read.push(wire.to_string());
                data.iter().find(|(t, _)| *t == wire).map(|(_, d)| d.to_vec())
            });
            (f, read)
        }

        #[test]
        fn a_paste_takes_svg_then_text_then_png() {
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;
            let svg: (&str, &[u8]) = (SVG, b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
            let text: (&str, &[u8]) = ("text/plain;charset=utf-8", b"a\r\nb");
            let png: (&str, &[u8]) = (PNG, b"\x89PNG");
            let (f, read) = pasted(&[text, png, svg], &PASTE_ORDER);
            assert_eq!(f.map(|f| (f.mime, f.data)), Some((SVG, svg.1.to_vec())));
            assert_eq!(read, [SVG]);
            // Text before a bitmap, its line ends made `\n`.
            let (f, _) = pasted(&[png, text], &PASTE_ORDER);
            assert_eq!(f.map(|f| (f.mime, f.data)), Some((TEXT, b"a\nb".to_vec())));
            // An offer of a PNG or SVG alone pastes it.
            assert_eq!(pasted(&[png], &PASTE_ORDER).0.map(|f| (f.mime, f.data)), Some((PNG, png.1.to_vec())));
            assert_eq!(pasted(&[svg], &PASTE_ORDER).0.map(|f| f.mime), Some(SVG));
            // Blank text, or text that only names a picture, gives way to the picture.
            for beside in [&b" \n"[..], b"https://example.com/art.png", b"<img src=\"art.png\">"] {
                let (f, read) = pasted(&[("UTF8_STRING", beside), png], &PASTE_ORDER);
                assert_eq!(f.map(|f| f.mime), Some(PNG));
                assert_eq!(read, ["UTF8_STRING", PNG]);
            }
            // With no picture, or a paste of text alone, the text is what there is.
            assert_eq!(pasted(&[("text/plain", &b"https://example.com/art.png"[..])], &PASTE_ORDER).0.map(|f| f.mime), Some(TEXT));
            assert_eq!(pasted(&[("text/plain", &b" "[..]), png], &[TEXT]).0.map(|f| f.mime), Some(TEXT));
            // An empty clipboard, a type no reader knows, data that doesn't come or is empty.
            assert_eq!(pasted(&[], &PASTE_ORDER).0, None);
            assert_eq!(pasted(&[("application/x-other", &b"x"[..])], &PASTE_ORDER).0, None);
            assert_eq!(pasted(&[(PNG, &b""[..])], &PASTE_ORDER).0, None);
            let types = offered(&[PNG]);
            assert_eq!(pick(&types, &PASTE_ORDER, |_| None), None);
        }

        #[test]
        fn the_clipboard_holds_a_copy_until_another_app_takes_it() {
            let mut seen = Seen::default();
            let types = vec!["text/plain;charset=utf-8", SVG, PNG];
            seen.copy(types.clone());
            assert!(seen.ours, "a Copy holds the clipboard before its offer comes");
            // An offer made before the Copy and handled after it.
            seen.selection(offered(&[PNG]));
            assert!(!seen.ours);
            // The Copy's own offer, its types in another order.
            seen.selection(offered(&[PNG, SVG, "text/plain;charset=utf-8"]));
            assert!(seen.ours);
            // Another app's copy: an offer with other types, then the source canceled.
            seen.selection(offered(&[PNG, SVG, "text/plain;charset=utf-8", "text/html"]));
            assert!(!seen.ours);
            seen.selection(offered(&[PNG, SVG, "text/plain;charset=utf-8"]));
            seen.cancelled();
            assert!(!seen.ours, "a canceled source no longer holds the clipboard");
            // Another app's offer with the same types after that is not ours.
            seen.selection(offered(&[PNG, SVG, "text/plain;charset=utf-8"]));
            assert!(!seen.ours);
            // An empty Copy empties the clipboard.
            seen.copy(vec![]);
            assert!(!seen.ours);
            seen.selection(vec![]);
            assert!(!seen.ours && seen.offered.is_empty());
        }

        #[test]
        fn a_copy_takes_the_clipboard_with_a_key_press_not_a_modifier_change() {
            let key = |serial| wl_keyboard::Event::Key { serial, time: 0, key: 46, state: WEnum::Value(wl_keyboard::KeyState::Pressed) };
            let modifiers = |serial| wl_keyboard::Event::Modifiers { serial, mods_depressed: 0, mods_latched: 0, mods_locked: 0, group: 0 };
            // Ctrl+C, then Ctrl released before the Copy's data is ready.
            let serial = keyboard_serial(&key(7), None);
            assert_eq!(keyboard_serial(&modifiers(8), serial), Some(7));
            assert_eq!(keyboard_serial(&wl_keyboard::Event::RepeatInfo { rate: 25, delay: 600 }, Some(7)), Some(7));
            assert_eq!(keyboard_serial(&modifiers(9), None), None);
        }

        /// A wait no test reaches: the other end of each pipe below either moves data or closes.
        const LONG: Duration = Duration::from_secs(600);

        /// A pipe holding `data`, its write end closed when `close`.
        fn pipe_with(data: &[u8], close: bool) -> (std::io::PipeReader, Option<std::io::PipeWriter>) {
            let (from, mut to) = std::io::pipe().unwrap();
            to.write_all(data).unwrap();
            (from, (!close).then_some(to))
        }

        #[test]
        fn a_read_stops_at_its_limit() {
            assert_eq!(MAX_BYTES, 64 * 1024 * 1024);
            assert_eq!(IDLE_WAIT, LOAD_WAIT);
            assert_eq!(read_capped(pipe_with(b"12345678", true).0, 8, Duration::ZERO), Ok(b"12345678".to_vec()));
            assert_eq!(read_capped(pipe_with(b"", true).0, 8, Duration::ZERO), Ok(vec![]));
            assert_eq!(read_capped(pipe_with(&[0u8; 9], true).0, 8, Duration::ZERO), Err("the clipboard's data is larger than 0 MiB".to_string()));
            // More than a pipe holds, from an app that is still sending.
            let (from, to) = std::io::pipe().unwrap();
            assert!(Sends::default().start(to, vec![0u8; (2 << 20) + 1].into()));
            assert_eq!(read_capped(from, 2 << 20, LONG), Err("the clipboard's data is larger than 2 MiB".to_string()));
            // A pipe that can't be read: the write end.
            let (_, to) = std::io::pipe().unwrap();
            let unreadable = std::fs::File::from(std::os::fd::OwnedFd::from(to));
            assert!(read_capped(unreadable, 8, LONG).is_err_and(|e| e.starts_with("couldn't read the clipboard's data")));
        }

        #[test]
        fn a_read_from_an_app_that_stops_sending_fails_and_closes_the_pipe() {
            let stopped = Err("the app that copied stopped sending the clipboard's data".to_string());
            // Some bytes and then nothing, the pipe left open: none of them are pasted.
            let (from, to) = pipe_with(b"part", false);
            assert_eq!(read_capped(from, MAX_BYTES, Duration::ZERO), stopped);
            // The read end is closed, so that app's next write fails.
            let mut to = to.unwrap();
            assert_eq!(to.write(b"x").map_err(|e| e.kind()), Err(ErrorKind::BrokenPipe));
            // Nothing at all, and data that fills the limit exactly with more never coming.
            let (from, _held) = pipe_with(b"", false);
            assert_eq!(read_capped(from, MAX_BYTES, Duration::ZERO), stopped);
            let (from, _held) = pipe_with(b"12345678", false);
            assert_eq!(read_capped(from, 8, Duration::ZERO), stopped);
            // The next read gets its data.
            assert_eq!(read_capped(pipe_with(b"words", true).0, MAX_BYTES, Duration::ZERO), Ok(b"words".to_vec()));
        }

        #[test]
        fn a_send_to_an_app_that_stops_reading_fails_and_frees_its_thread() {
            let data: Arc<[u8]> = vec![3u8; 300_000].into();
            // More than a pipe holds, to an app that never reads.
            let (_held, to) = std::io::pipe().unwrap();
            assert_eq!(send(to, &data, Duration::ZERO).map_err(|e| e.kind()), Err(ErrorKind::TimedOut));
            // An app that closes its end before reading.
            let (from, to) = std::io::pipe().unwrap();
            drop(from);
            assert_eq!(send(to, &data, LONG).map_err(|e| e.kind()), Err(ErrorKind::BrokenPipe));
            // A send thread to an app that does not read until the send gives up, its end still open.
            let sends = Sends { idle: Duration::ZERO, ..Sends::default() };
            let (from, to) = std::io::pipe().unwrap();
            assert!(sends.start(to, data.clone()));
            while sends.running() > 0 {
                std::thread::yield_now();
            }
            // What the pipe held when the send gave up, then the end of the data: the send closed it.
            let got = read_capped(from, MAX_BYTES, Duration::ZERO).map(|d| d.len());
            assert!(got.as_ref().is_ok_and(|&n| n > 0 && n < data.len()), "{got:?}");
            // The thread is free for the next paste.
            let (from, to) = std::io::pipe().unwrap();
            assert!(sends.start(to, b"words"[..].into()));
            assert_eq!(read_capped(from, MAX_BYTES, LONG), Ok(b"words".to_vec()));
        }

        #[test]
        fn a_copys_data_is_sent_through_the_pipe_and_closed() {
            let sends = Sends::default();
            assert_eq!(sends.idle, IDLE_WAIT);
            let (from, to) = std::io::pipe().unwrap();
            let data: Arc<[u8]> = vec![7u8; 300_000].into();
            assert!(sends.start(to, data.clone()));
            // More than a pipe holds: the write ends once all of it is read, then the pipe closes.
            assert_eq!(read_capped(from, MAX_BYTES, LONG).as_deref(), Ok(&data[..]));
            // An app that closes its end before reading ends the send.
            let (from, to) = std::io::pipe().unwrap();
            drop(from);
            assert!(sends.start(to, data.clone()));
            while sends.running() > 0 {
                std::thread::yield_now();
            }
        }

        #[test]
        fn pastes_that_read_nothing_hold_a_few_sends_at_most() {
            let sends = Sends { idle: LONG, ..Sends::default() };
            let data: Arc<[u8]> = vec![1u8; 300_000].into();
            // Apps that ask for the data and never read it, within the sends' idle wait.
            let stalled: Vec<_> = (0..MAX_SENDS)
                .map(|_| {
                    let (from, to) = std::io::pipe().unwrap();
                    assert!(sends.start(to, data.clone()));
                    from
                })
                .collect();
            // The next paste gets its pipe closed at once, with nothing written.
            let (from, to) = std::io::pipe().unwrap();
            assert!(!sends.start(to, data.clone()));
            assert_eq!(read_capped(from, MAX_BYTES, Duration::ZERO), Ok(vec![]));
            // Once those apps close their pipes, the sends end and others are sent again.
            drop(stalled);
            while sends.running() > 0 {
                std::thread::yield_now();
            }
            let (from, to) = std::io::pipe().unwrap();
            assert!(sends.start(to, data.clone()));
            assert_eq!(read_capped(from, MAX_BYTES, LONG).map(|d| d.len()), Ok(data.len()));
        }

        /// What a clipboard holds, by type.
        type Held = &'static [(&'static str, &'static [u8])];
        const PNG_ONLY: Held = &[(PNG, b"\x89PNG")];
        const SVG_ONLY: Held = &[(SVG, b"<svg/>")];
        const TEXT_ONLY: Held = &[("text/plain;charset=utf-8", b"words")];
        const EMPTY_TEXT: Held = &[("text/plain", b"")];
        const NOTHING: Held = &[];
        const UNKNOWN: Held = &[("application/x-other", b"x")];

        /// Reads of a clipboard holding `data`, answered at once.
        fn reads_of(data: Held) -> Reads {
            let loads = Loads::spawn("test", Duration::ZERO, move |wire: String| {
                data.iter().find(|(t, _)| *t == wire).map(|(_, d)| d.to_vec()).ok_or_else(|| "not offered".to_string())
            })
            .unwrap();
            let types: Vec<&str> = data.iter().map(|(t, _)| *t).collect();
            Reads::new(loads, Arc::new(Mutex::new(Seen::offering(&types))))
        }

        #[test]
        fn a_picture_or_svg_alone_can_be_pasted_and_is() {
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;
            let wait = Duration::from_secs(600);
            for (data, mime) in [(PNG_ONLY, PNG), (SVG_ONLY, SVG)] {
                let mut reads = reads_of(data);
                // The paste check enables Paste from the types alone.
                assert!(reads.has(&PASTE_ORDER));
                let Some(ClipboardRead::Later(work)) = reads.read(&PASTE_ORDER, wait) else { panic!("{mime} is read off the UI thread") };
                assert_eq!(work().map(|f| (f.mime, f.data)), Some((mime, data[0].1.to_vec())));
            }
            let mut reads = reads_of(TEXT_ONLY);
            assert!(reads.has(&[TEXT]) && !reads.has(&[BITMAP, SVG]));
            let Some(ClipboardRead::Now(f)) = reads.read(&PASTE_ORDER, wait) else { panic!("text is read at once") };
            assert_eq!((f.mime, f.data), (TEXT, b"words".to_vec()));
            // An empty clipboard, or one with no type a paste reads, enables nothing and pastes nothing.
            for data in [NOTHING, UNKNOWN] {
                let mut reads = reads_of(data);
                assert!(!reads.has(&PASTE_ORDER));
                assert!(reads.read(&PASTE_ORDER, wait).is_none());
                assert_eq!(reads.text(wait), Ok(String::new()));
            }
            // Empty text is not pasted.
            assert!(reads_of(EMPTY_TEXT).read(&[TEXT], wait).is_none());
        }

        #[test]
        fn an_svg_that_sends_nothing_gives_way_to_the_text_or_png_beside_it() {
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;
            const EMPTY_SVG_TEXT: Held = &[(SVG, b""), ("text/plain", b"words"), (PNG, b"\x89PNG")];
            const EMPTY_SVG_PNG: Held = &[(SVG, b""), (PNG, b"\x89PNG")];
            const EMPTY_SVG_URL: Held = &[(SVG, b""), ("text/plain", b"https://example.com/art.png"), (PNG, b"\x89PNG")];
            let wait = Duration::from_secs(600);
            const EMPTY_PNG: Held = &[(PNG, b"")];
            for (data, mime, bytes) in
                [(EMPTY_SVG_TEXT, TEXT, &b"words"[..]), (EMPTY_SVG_PNG, PNG, &b"\x89PNG"[..]), (EMPTY_SVG_URL, PNG, &b"\x89PNG"[..])]
            {
                // Held while the work runs, as the clipboard is.
                let mut reads = reads_of(data);
                let Some(ClipboardRead::Later(work)) = reads.read(&PASTE_ORDER, wait) else { panic!("the SVG is asked for") };
                assert_eq!(work().map(|f| (f.mime, f.data)), Some((mime, bytes.to_vec())));
            }
            // A failed PNG read, last of all, pastes nothing.
            let mut reads = reads_of(EMPTY_PNG);
            let Some(ClipboardRead::Later(work)) = reads.read(&PASTE_ORDER, wait) else { panic!("the PNG is asked for") };
            assert!(work().is_none());
        }

        #[test]
        fn a_read_that_fails_pastes_nothing() {
            let loads = Loads::spawn("test", Duration::ZERO, |_: String| Err::<Vec<u8>, _>("the clipboard's data is larger than 64 MiB".to_string()))
                .unwrap();
            let mut reads = Reads::new(loads, Arc::new(Mutex::new(Seen::offering(&[PNG, "text/plain"]))));
            assert_eq!(reads.text(Duration::from_secs(600)), Err("the clipboard's data is larger than 64 MiB".to_string()));
            let Some(ClipboardRead::Later(work)) = reads.read(&[BITMAP], Duration::from_secs(600)) else { panic!("the PNG is asked for") };
            assert!(work().is_none());
        }

        /// Reads of a clipboard holding `data` by type, answered at once.
        fn reads_holding(data: Vec<(String, Vec<u8>)>) -> Reads {
            let types: Vec<String> = data.iter().map(|(t, _)| t.clone()).collect();
            let loads = Loads::spawn("test", Duration::ZERO, move |wire: String| {
                data.iter().find(|(t, _)| *t == wire).map(|(_, d)| d.clone()).ok_or_else(|| "not offered".to_string())
            })
            .unwrap();
            let types: Vec<&str> = types.iter().map(String::as_str).collect();
            Reads::new(loads, Arc::new(Mutex::new(Seen::offering(&types))))
        }

        /// The `file:` URI of `path`, its spaces escaped.
        fn file_uri(path: &std::path::Path) -> String {
            format!("file://{}", path.display().to_string().replace(' ', "%20"))
        }

        /// What a paste of `mimes` takes from `reads`.
        fn paste(reads: &mut Reads, mimes: &[&'static str]) -> Option<(&'static str, Vec<u8>)> {
            match reads.read(mimes, Duration::from_secs(600))? {
                ClipboardRead::Now(f) => Some((f.mime, f.data)),
                ClipboardRead::Later(work) => work().map(|f| (f.mime, f.data)),
            }
        }

        #[test]
        fn files_copied_in_a_file_manager_paste_as_their_art() {
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;
            let d = tempfile::Builder::new().prefix("vectorcraft-wayland-files-").tempdir().unwrap();
            let (notes, red) = (d.path().join("notes.txt"), d.path().join("red image.png"));
            let mut png = vec![];
            image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            std::fs::write(&notes, "not art").unwrap();
            std::fs::write(&red, &png).unwrap();
            let list = |paths: &[&std::path::Path]| paths.iter().map(|p| file_uri(p) + "\r\n").collect::<String>().into_bytes();
            let held = |types: &[(&str, Vec<u8>)]| reads_holding(types.iter().map(|(t, d)| (t.to_string(), d.clone())).collect());
            let text = ("text/plain;charset=utf-8", b"the files' paths".to_vec());
            // A file list alone enables Paste and pastes the first file that is art.
            let mut reads = held(&[("text/uri-list", list(&[&notes, &red]))]);
            assert!(reads.has(&PASTE_ORDER) && reads.has(&[BITMAP]));
            assert_eq!(paste(&mut reads, &PASTE_ORDER), Some((PNG, png.clone())));
            // A paste of text alone takes no file.
            assert!(!reads.has(&[TEXT]));
            assert_eq!(paste(&mut reads, &[TEXT]), None);
            // The files come before the other types offered, as on the other platforms; the type in
            // any letter case.
            let mut reads = held(&[(SVG, b"<svg/>".to_vec()), text.clone(), ("Text/URI-List", list(&[&red]))]);
            assert_eq!(paste(&mut reads, &PASTE_ORDER), Some((PNG, png.clone())));
            assert_eq!(paste(&mut reads, &[TEXT]), Some((TEXT, text.1.clone())));
            // With no file that is art, the paste takes the other types.
            let mut reads = held(&[("text/uri-list", list(&[&notes, &d.path().join("gone.png")])), text.clone()]);
            assert_eq!(paste(&mut reads, &PASTE_ORDER), Some((TEXT, text.1.clone())));
            let mut reads = held(&[("text/uri-list", b"https://example.com/art.png\r\n".to_vec())]);
            assert_eq!(paste(&mut reads, &PASTE_ORDER), None);
            // GNOME Files' own list, alone or when the URI list names no local file.
            let gnome = [b"copy\n".to_vec(), file_uri(&red).into_bytes()].concat();
            let mut reads = held(&[("x-special/gnome-copied-files", gnome.clone())]);
            assert!(reads.has(&PASTE_ORDER));
            assert_eq!(paste(&mut reads, &PASTE_ORDER), Some((PNG, png.clone())));
            let mut reads = held(&[("text/uri-list", b"file://server/art.png\r\n".to_vec()), ("x-special/gnome-copied-files", gnome)]);
            assert_eq!(paste(&mut reads, &PASTE_ORDER), Some((PNG, png.clone())));
            // KDE's cut marker is not a file list.
            let mut reads = held(&[("application/x-kde-cutselection", b"1".to_vec())]);
            assert!(!reads.has(&PASTE_ORDER));
            assert_eq!(paste(&mut reads, &PASTE_ORDER), None);
        }

        #[test]
        fn a_file_list_that_cant_be_read_gives_way_to_the_other_types() {
            use vectorcraft_engine::cmd::clipboard::PASTE_ORDER;
            let offered = offered(&["text/uri-list", "text/plain"]);
            let mut asked = vec![];
            let f = pick_pasted(&offered, &PASTE_ORDER, |wire| {
                asked.push(wire.to_string());
                (wire == "text/plain").then(|| b"words".to_vec())
            });
            assert_eq!(f.map(|f| (f.mime, f.data)), Some((TEXT, b"words".to_vec())));
            assert_eq!(asked, ["text/uri-list", "text/plain"]);
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

    fn uri_paths(list: &[u8]) -> Vec<String> {
        uri_list_paths(list).into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_uri_list_names_the_local_files_of_its_lines() {
        // CRLF or LF, comment lines and blank lines.
        assert_eq!(uri_paths(b"# copied\r\nfile:///home/a/one.svg\r\n\r\nfile:///home/a/two.png\n"), ["/home/a/one.svg", "/home/a/two.png"]);
        assert_eq!(uri_paths(b"file:///a.svg"), ["/a.svg"]);
        // Percent escapes are decoded once; `+` and an escaped `#` are part of the name.
        assert_eq!(uri_paths(b"file:///home/a/red%20image.png"), ["/home/a/red image.png"]);
        assert_eq!(uri_paths(b"file:///home/a/caf%C3%A9.svg\nfile:///home/a/caf\xC3\xA9.svg"), ["/home/a/café.svg", "/home/a/café.svg"]);
        assert_eq!(uri_paths(b"file:///a/%231+2.svg"), ["/a/#1+2.svg"]);
        assert_eq!(uri_paths(b"file:///a/100%2525.svg"), ["/a/100%25.svg"]);
        // No host, `localhost` in any case, and the short form name this machine; a fragment is
        // left out.
        assert_eq!(
            uri_paths(b"file://localhost/a.svg\nFILE://LocalHost/b.svg\nfile:/c.svg\nfile:///d.svg#page=2"),
            ["/a.svg", "/b.svg", "/c.svg", "/d.svg"]
        );
        // Other hosts and schemes, relative paths, queries, bad escapes, an escaped NUL, a name
        // that is not UTF-8 once decoded and a line that is not UTF-8 name no file.
        for uri in [
            &b"file://server/a.svg"[..],
            b"https://example.com/a.svg",
            b"file:a.svg",
            b"file://",
            b"file://localhost",
            b"file:///a.svg?x=1",
            b"file:///a%2",
            b"file:///a%zz.svg",
            b"file:///a%00b.svg",
            b"file:///a%FF.svg",
            b"file:///a\xFF.svg",
            b"/home/a/one.svg",
            b"#file:///a.svg",
        ] {
            assert!(uri_list_paths(uri).is_empty(), "{}", String::from_utf8_lossy(uri));
        }
        // The files beside a URI that names none are kept.
        assert_eq!(uri_paths(b"https://example.com/a.svg\r\nfile:///b.svg\r\n"), ["/b.svg"]);
        assert!(uri_list_paths(b"").is_empty());
    }

    #[test]
    fn a_uri_list_longer_than_the_limit_names_no_file() {
        assert_eq!(MAX_URIS, 1024);
        let list = |n: usize| (0..n).map(|i| format!("# file {i}\r\nfile:///f{i}.svg\r\n")).collect::<String>();
        // Comment lines don't count.
        assert_eq!(uri_list_paths(list(MAX_URIS).as_bytes()).len(), MAX_URIS);
        assert!(uri_list_paths(list(MAX_URIS + 1).as_bytes()).is_empty());
    }

    #[test]
    fn gnome_files_list_names_its_files_after_copy_or_cut() {
        let paths = |list: &[u8]| gnome_copied_paths(list).into_iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(paths(b"copy\nfile:///a.svg\nfile:///b%20c.png"), ["/a.svg", "/b c.png"]);
        assert_eq!(paths(b"cut\r\nfile:///a.svg\r\n"), ["/a.svg"]);
        // Another first line, none, or no files.
        for list in [&b"move\nfile:///a.svg"[..], b"file:///a.svg\nfile:///b.svg", b"copy", b"", b"Copy\nfile:///a.svg"] {
            assert!(gnome_copied_paths(list).is_empty(), "{}", String::from_utf8_lossy(list));
        }
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
