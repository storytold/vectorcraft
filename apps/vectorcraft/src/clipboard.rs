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
//!   minute, not each time Paste asks.
//! - Everywhere: files copied in a file manager paste as the first one that is art (SVG, PDF, EPS,
//!   a metafile or a bitmap), before the clipboard's other formats (the files' paths as text).

use std::io::{Cursor, Read};
use std::path::PathBuf;

use vectorcraft_engine::cmd::clipboard::{BITMAP, EPS, FILE_HEAD, Flavour, PDF, PNG, SVG, TEXT, file_flavour};
use vectorcraft_ui_egui::SystemClipboard;

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
        BITMAP, Cursor, Flavour, PNG, SVG, SystemClipboard, TEXT, copied_file, decode_png, dib_image, png_flavour, text_of, windows_formats,
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
        /// The text of our last write.
        text: Option<String>,
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
            self.text = text_of(flavours);
            result
        }

        fn holds_ours(&mut self) -> bool {
            if self.seq.is_some() && raw::seq_num() == self.seq {
                return true;
            }
            // A clipboard manager may have copied our data again as its own.
            let Some(ours) = self.text.as_deref() else { return false };
            let Ok(_open) = clipboard_win::Clipboard::new_attempts(10) else { return false };
            read_one(TEXT).is_some_and(|f| f.data == ours.as_bytes())
        }

        fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
            let mut files = vec![];
            {
                let _open = clipboard_win::Clipboard::new_attempts(10).ok()?;
                // None copied when the clipboard holds no file list.
                let _ = raw::get_file_list_path(&mut files);
            }
            // The files are read with the clipboard closed again, not keeping other apps waiting.
            copied_file(files, mimes).or_else(|| {
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

    /// One flavour, while the clipboard is open.
    fn read_one(mime: &'static str) -> Option<Flavour> {
        let mut data = vec![];
        let mime = match mime {
            BITMAP if read_registered(PNG, &mut data) => PNG,
            BITMAP => {
                return DIBS.into_iter().find_map(|id| {
                    data.clear();
                    raw::get_vec(id, &mut data).ok()?;
                    png_flavour(&dib_image(&data)?)
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
        (!data.is_empty()).then_some(Flavour { mime, data })
    }
}

/// arboard, or on a Wayland compositor without its data-control protocol the core protocol's text
/// clipboard: one format at a time. Built everywhere so it is checked everywhere.
#[cfg_attr(windows, allow(dead_code))]
mod portable {
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::{BITMAP, Flavour, PNG, SystemClipboard, TEXT, copied_file, decode_png, png_flavour, text_of};

    /// What our last write put on the clipboard.
    enum Ours {
        Text(String),
        Image(usize, usize),
    }

    /// An open clipboard connection.
    enum Backend {
        Arboard(arboard::Clipboard),
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
                || arboard::Clipboard::new().map(Self::Arboard).map_err(|e| e.to_string()),
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
                Self::Arboard(cb) => cb.get_text().map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => cb.get_text(),
            }
        }

        fn set_text(&mut self, text: &str) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.set_text(text).map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => {
                    cb.set_text(text);
                    Ok(())
                }
            }
        }

        fn get_image(&mut self) -> Result<arboard::ImageData<'static>, String> {
            match self {
                Self::Arboard(cb) => cb.get_image().map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => Err(TEXT_ONLY.into()),
            }
        }

        fn set_image(&mut self, img: arboard::ImageData<'_>) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.set_image(img).map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(_) => Err(TEXT_ONLY.into()),
            }
        }

        fn clear(&mut self) -> Result<(), String> {
            match self {
                Self::Arboard(cb) => cb.clear().map_err(|e| e.to_string()),
                #[cfg(target_os = "linux")]
                Self::Wayland(cb) => {
                    cb.set_text("");
                    Ok(())
                }
            }
        }

        /// Files copied in a file manager (none where the backend can't read a file list).
        fn file_list(&mut self) -> Vec<std::path::PathBuf> {
            match self {
                Self::Arboard(cb) => cb.get().file_list().unwrap_or_default(),
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
            self.ours = ours;
            Ok(())
        }

        fn holds_ours(&mut self) -> bool {
            let Some(cb) = self.cb.opened() else { return false };
            match &self.ours {
                Some(Ours::Text(t)) => cb.get_text().is_ok_and(|now| now == *t),
                Some(Ours::Image(w, h)) => cb.get_image().is_ok_and(|now| (now.width, now.height) == (*w, *h)),
                None => false,
            }
        }

        fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
            let cb = self.cb(Asker::User).ok()?;
            if let Some(f) = copied_file(cb.file_list(), mimes) {
                return Some(f);
            }
            mimes.iter().find_map(|m| {
                #[cfg(target_os = "macos")]
                if let Some(f) = super::mac::read(m) {
                    return Some(f);
                }
                match *m {
                    TEXT => cb.get_text().ok().filter(|t| !t.is_empty()).map(|t| Flavour { mime: TEXT, data: t.into_bytes() }),
                    BITMAP => {
                        let img = cb.get_image().ok()?;
                        let rgba =
                            image::RgbaImage::from_raw(u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?, img.bytes.into_owned())?;
                        png_flavour(&rgba)
                    }
                    _ => None,
                }
            })
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
/// data with no limit, so reads run on a worker of their own.
#[cfg(target_os = "linux")]
mod wayland {
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The longest a read waits for the selection's owner to send its text.
    const LOAD_WAIT: Duration = Duration::from_secs(2);
    /// What a read says once its worker has stopped.
    const GONE: &str = "the clipboard's reader stopped";

    pub struct TextClipboard {
        /// Writes: a store only hands the text to smithay-clipboard's thread.
        cb: smithay_clipboard::Clipboard,
        /// Reads, on a worker with a second clipboard on the same connection.
        loads: Loads,
        /// Held as long as the clipboards: their connection is the window's.
        _window: Arc<winit::window::Window>,
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
            let loads = Loads::spawn(move || loader.load())?;
            Some(Self { cb, loads, _window: window })
        }

        pub fn get_text(&mut self) -> Result<String, String> {
            self.loads.get(LOAD_WAIT)
        }

        pub fn set_text(&self, text: &str) {
            self.cb.store(text);
        }
    }

    /// Reads made on a worker thread and waited for a bounded time: a selection owner that never
    /// sends its data holds the worker, not the caller.
    struct Loads {
        requests: mpsc::Sender<()>,
        answers: mpsc::Receiver<Result<String, String>>,
        /// A read that timed out and hasn't answered yet: until it does, reads don't queue behind it.
        pending: bool,
    }

    impl Loads {
        fn spawn(mut load: impl FnMut() -> Result<String, String> + Send + 'static) -> Option<Self> {
            let (requests, asked) = mpsc::channel::<()>();
            let (answer, answers) = mpsc::channel();
            let work = move || {
                while asked.recv().is_ok() {
                    if answer.send(load()).is_err() {
                        break;
                    }
                }
            };
            std::thread::Builder::new().name("wayland-clipboard-read".into()).spawn(work).ok()?;
            Some(Self { requests, answers, pending: false })
        }

        /// The text, or why there is none, after `wait` at most.
        fn get(&mut self, wait: Duration) -> Result<String, String> {
            if self.pending {
                match self.answers.try_recv() {
                    // The late answer of the read that timed out: stale by now.
                    Ok(_) => self.pending = false,
                    Err(mpsc::TryRecvError::Empty) => return Err("the clipboard's owner hasn't answered an earlier read".into()),
                    Err(mpsc::TryRecvError::Disconnected) => return Err(GONE.into()),
                }
            }
            self.requests.send(()).map_err(|_| GONE.to_string())?;
            match self.answers.recv_timeout(wait) {
                Ok(text) => text,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.pending = true;
                    Err(format!("the clipboard's owner didn't send its text within {wait:?}"))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(GONE.into()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Instant;

        use super::*;

        #[test]
        fn a_read_that_answers_in_time_gives_the_text() {
            let mut loads = Loads::spawn(|| Ok("<svg/>".into())).unwrap();
            assert_eq!(loads.get(Duration::from_secs(5)), Ok("<svg/>".to_string()));
            assert_eq!(loads.get(Duration::from_secs(5)), Ok("<svg/>".to_string()));
        }

        #[test]
        fn an_owner_that_never_answers_holds_the_worker_not_the_caller() {
            let (release, gate) = mpsc::channel::<()>();
            let mut calls = 0;
            let mut loads = Loads::spawn(move || {
                calls += 1;
                if calls == 1 {
                    // The first owner sends nothing until released.
                    let _ = gate.recv();
                    return Ok("stale".into());
                }
                Ok("fresh".into())
            })
            .unwrap();
            let start = Instant::now();
            assert!(loads.get(Duration::from_millis(50)).is_err(), "the first read times out");
            // While it is unanswered, a read doesn't queue behind it or wait at all.
            assert!(loads.get(Duration::from_secs(60)).is_err());
            assert!(start.elapsed() < Duration::from_secs(30), "no read waited past its bound");
            release.send(()).unwrap();
            // Once the late answer is in, it is dropped and a new read is made.
            let text = (0..200).find_map(|_| {
                std::thread::sleep(Duration::from_millis(10));
                loads.get(Duration::from_secs(5)).ok()
            });
            assert_eq!(text.as_deref(), Some("fresh"));
        }

        #[test]
        fn a_read_after_the_worker_stops_says_so() {
            let mut loads = Loads::spawn(|| Ok(String::new())).unwrap();
            // The worker ends when its answers can't be sent: drop them by hand.
            let (_, dead) = mpsc::channel();
            drop(std::mem::replace(&mut loads.answers, dead));
            assert_eq!(loads.get(Duration::from_millis(50)), Err(GONE.to_string()));
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

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vectorcraft-clipboard-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::new(w, h).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn the_first_copied_file_that_is_art_pastes() {
        let d = dir("art");
        let (notes, red, art) = (d.join("notes.txt"), d.join("red image.png"), d.join("art.svg"));
        std::fs::write(&notes, "not art").unwrap();
        std::fs::write(&red, png(3, 2)).unwrap();
        std::fs::write(&art, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        // Missing files, folders and text are skipped.
        let f = copied_file(vec![d.join("gone.png"), d.clone(), notes.clone(), red.clone(), art.clone()], &PASTE_ORDER).unwrap();
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
        let eps = dir("eps").join("equation.eps");
        std::fs::write(&eps, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n0 0 10 10 rectfill\n").unwrap();
        assert_eq!(copied_file(vec![eps.clone()], &PASTE_ORDER).map(|f| f.mime), Some(EPS));
        assert!(copied_file(vec![eps], &[BITMAP, TEXT]).is_none());
    }

    #[test]
    fn a_crlf_left_by_the_uri_list_is_ignored() {
        let red = dir("crlf").join("red.png");
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
