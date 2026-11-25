use craft::platform::clipboard::ClipboardBackend;
use craft::ui::runtime::pen::PenSource;

use super::{Arc, Mutex, background, place, print};

pub type CheckApplicationFn = Box<dyn Fn(&str) -> Result<(), String>>;
pub type OpenApplicationFn = Box<dyn FnMut(&str, &str) -> Result<bool, String>>;

pub type ReadFn = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;

/// The first `max` bytes of the file at a path (all of it when it is shorter); the rest of the file
/// is not read.
pub type ReadHeadFn = Box<dyn Fn(&str, usize) -> Result<Vec<u8>, String>>;

pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

/// A save writer that keeps typed filesystem failures, on either the UI or background thread.
pub type SaveWriteFn = Arc<dyn Fn(&str, &[u8]) -> vectorcraft_engine::Result<()> + Send + Sync>;

/// A typed save writer for native files encoded directly into the output.
pub type SaveStreamFn =
    Arc<dyn Fn(&str, &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>) -> vectorcraft_engine::Result<()> + Send + Sync>;

/// Shared, so Background Save and Export can check from a worker thread too.
pub type ExistsFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Starts a browser download of the bytes under a file name; an error when it couldn't be started
/// (the save or export then fails as a refused write does: #1693).
pub type DownloadFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;

/// Whether a URL from a document may open in the browser: an `http:`, `https:` or `mailto:` URL
/// without control characters. The system hands other schemes (`file:`, application schemes) to
/// other programs.
pub(crate) fn browsable_url(url: &str) -> Result<(), String> {
    let scheme = url.split_once(':').map_or("", |(s, _)| s);
    if ["http", "https", "mailto"].iter().any(|k| scheme.eq_ignore_ascii_case(k)) && !url.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(format!("only http, https and mailto URLs open in the browser, not “{url}”"))
    }
}

/// Shows a file in the system file manager, or opens it in its app.
pub type RevealFn = Box<dyn FnMut(&str) -> Result<(), String>>;

/// Opens a file in a fixed app; `Ok(false)` when that app isn't installed.
pub type OpenInFn = Box<dyn FnMut(&str) -> Result<bool, String>>;

/// Platform services injected by the host app (desktop or web). File dialogs, URLs and the screen
/// color picker are the framework host's.
#[derive(Default)]
pub struct Services {
    /// The system clipboard (desktop). Without it (tests, the web before its host sets one) copies
    /// stay inside the app.
    pub clipboard: Option<Box<dyn ClipboardBackend>>,
    /// Makes a second clipboard connection on the paste probe's own thread (Linux), so an
    /// unresponsive clipboard owner never stalls a frame. Without it Paste checks the clipboard in
    /// line, a few times a second.
    pub clipboard_probe: Option<Box<dyn FnOnce() -> Box<dyn ClipboardBackend> + Send>>,
    /// The pen the host reads itself (tablet APIs the window system doesn't route): the same
    /// handle the host installs for the framework's pointer feed.
    pub pen: Option<PenSource>,
    pub read: Option<ReadFn>,
    /// Read at most a number of bytes from a file (desktop). A file with a size limit is read one
    /// byte past the limit, so a larger file is refused without reading all of it. Without it, the
    /// whole file is read with `read`.
    pub read_head: Option<ReadHeadFn>,
    pub write: Option<WriteFn>,
    /// Is there a file or folder at a path (desktop)? The app asks before replacing a file that a
    /// save panel's name names once the app gives it an extension, and before a save or export
    /// replaces a file it writes beside that one (a file per artboard, a linked image). Without it,
    /// no file counts as there.
    pub exists: Option<ExistsFn>,
    /// Files that arrived asynchronously (web drops): (name, bytes).
    pub inbox: Option<Inbox>,
    /// Web: files the page's script handed over (`window.vectorcraft.openFile`). They open as
    /// automation's.
    pub page_inbox: Option<Inbox>,
    /// Web: trigger a browser download instead of writing a path.
    pub download: Option<DownloadFn>,
    /// Files that arrived asynchronously to be placed (web: dropped on the canvas).
    pub place_inbox: Option<place::PlaceInbox>,
    /// File → Show in Folder: select a file in the system file manager (desktop).
    pub reveal: Option<RevealFn>,
    /// Write a file from any thread (desktop): lets Background Save and Export write off the UI
    /// thread ([`background`]). Without it they run at once.
    pub write_shared: Option<background::SharedWriteFn>,
    /// Write a file from any thread with what a filler writes into it (desktop): native saves
    /// stream into the file as they are encoded instead of being held whole in memory first.
    /// Without it they are encoded, then written with `write` / `write_shared`.
    pub stream_shared: Option<background::SharedStreamFn>,
    /// Typed writers for document saves only. Without them the existing string writers are used
    /// and failures keep their normal dialog. Downloads never use these callbacks.
    pub save_write: Option<SaveWriteFn>,
    pub save_stream: Option<SaveStreamFn>,
    /// Open a file (or a folder) in the system's default app for it (desktop: Edit Original, Show
    /// Package). Without it (the web) those answer with an error.
    pub open_file: Option<RevealFn>,
    /// Open a file in PhotoCraft where it is installed (desktop: Edit Original of a raster image
    /// with Preferences › File Handling › Use System Defaults for Edit Original off); `Ok(false)`
    /// when it isn't. Without it Edit Original uses `open_file`.
    pub open_in_photocraft: Option<OpenInFn>,
    /// Validate a native-picked application before keeping it as the raster editor.
    pub check_application: Option<CheckApplicationFn>,
    /// Launch a native-picked application with one file argument; false only when absent.
    pub open_in_application: Option<OpenApplicationFn>,
    /// Where Data Recovery keeps its copies when not in a folder (the web's browser storage;
    /// tests): installed into the session ([`vectorcraft_engine::cmd::recovery`]).
    pub recovery_store: Option<std::sync::Arc<dyn vectorcraft_engine::cmd::recovery::RecoveryStore>>,
    /// File → Print: the system's printers and print queue (desktop), the browser's print dialog
    /// (web). Without it Print saves the job as a PDF.
    pub print: Option<Box<dyn print::PrintService>>,
    /// The macOS menu bar, when the desktop app installed one: the in-window menus are hidden then.
    pub native_menu: Option<craft::menu::NativeMenu>,
    /// The text Help › Copy Troubleshooting Info copies (desktop: the platform, the graphics
    /// processor, recent commands and frame times, and the hang reports of this session and the
    /// one before). Without it the command copies the version only ([`crate::troubleshooting`]).
    pub troubleshooting: Option<Box<dyn Fn() -> String>>,
}
