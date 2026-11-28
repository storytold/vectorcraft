//! Windows: the thumbnail handler that Explorer and the file dialogs run for `.vectorcraft`,
//! `.drawcraft` and `.vctemplate` files. It is a COM server DLL whose class implements
//! `IThumbnailProvider` and `IInitializeWithStream`
//! (<https://learn.microsoft.com/windows/win32/shell/thumbnail-providers>,
//! <https://learn.microsoft.com/windows/win32/api/thumbcache/nn-thumbcache-ithumbnailprovider>).
//!
//! Explorer gives the handler the file's bytes as a stream. The handler writes them to a new file
//! in `%USERPROFILE%\AppData\LocalLow\VectorCraft\Thumbnails` (falling back to the temp folder),
//! runs `vectorcraft-cli thumbnail --size N IN OUT` from the DLL's folder on
//! it, and returns the PNG that command writes (the first artboard, else the art, drawn by
//! `vectorcraft_render::encode::first_artboard`) as a 32-bit DIB section with alpha. A file larger
//! than 256 MiB, a document that can't be opened or shows nothing, and a `vectorcraft-cli` run
//! longer than [`DEADLINE`] give Explorer an error, and Explorer shows the file's icon. Rendering
//! runs in the `vectorcraft-cli` process, so a file that makes it fail or hang ends that process.
//! The handler is not given the file's path (`IInitializeWithStream` is the only initialization
//! an isolated handler gets), so a linked image found only relative to the document's folder
//! shows the preview saved with it.
//!
//! `regsvr32 vectorcraft_thumbnail_handler.dll` registers the handler for the current user, and
//! `regsvr32 /u` removes it ([`registry_values`]). The COM parts build on Windows only; the parts
//! in this file build everywhere and their tests run on every system.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod com;

#[cfg(test)]
mod tests;

/// The handler's class id: the key under `CLSID`, and the value of each extension's `ShellEx` key.
pub const CLSID: &str = "{1971F5DA-D761-4655-AB25-6B3825BD1748}";
/// [`CLSID`] as a number, the form COM compares.
pub const CLSID_VALUE: u128 = 0x1971_f5da_d761_4655_ab25_6b38_25bd_1748;
/// The `ShellEx` subkey that names an extension's thumbnail handler, the interface id of
/// `IThumbnailProvider` (<https://learn.microsoft.com/windows/win32/shell/thumbnail-providers>).
pub const THUMBNAIL_SHELL_EX: &str = "{e357fccd-a995-4576-b01f-234630154e96}";
/// The extensions the handler is registered for: documents, documents under their former
/// extension, and templates (the globs of `packaging/linux/ai.storyteller.vectorcraft.mime.xml`).
pub const EXTENSIONS: &[&str] = &[".vectorcraft", ".drawcraft", ".vctemplate"];
/// The description in the class key.
pub const DESCRIPTION: &str = "VectorCraft document thumbnail handler";
/// The largest file the handler reads, in bytes, the limit `vectorcraft-cli thumbnail` applies.
pub const MAX_FILE: u64 = 256 << 20;
/// The largest `--size` `vectorcraft-cli thumbnail` takes. Explorer asks for sizes up to 1024.
pub const MAX_SIDE: u32 = 1024;
/// The largest PNG the handler reads back, in bytes: an uncompressed 1024 by 1024 RGBA picture
/// with room for the chunks around it.
pub const MAX_PNG: u64 = 8 << 20;
/// How long the handler waits for `vectorcraft-cli` before it ends the process: the command's
/// own 20-second limit and 5 seconds to start.
pub const DEADLINE: Duration = Duration::from_secs(25);
/// How often the handler checks whether `vectorcraft-cli` has exited.
const POLL: Duration = Duration::from_millis(10);
/// The program the handler runs, in the DLL's folder.
pub const CLI: &str = "vectorcraft-cli.exe";

/// A picture in the layout Explorer takes: rows from top to bottom, 4 bytes per pixel in blue,
/// green, red and alpha order, with the colors premultiplied by alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// One registry value the handler writes, under `Software\Classes` of the current user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryValue {
    /// The key, relative to `Software\Classes`.
    pub key: String,
    /// The value's name; `None` is the key's default value.
    pub name: Option<&'static str>,
    /// The string the value holds.
    pub data: String,
}

/// The class key, relative to `Software\Classes`.
pub fn class_key() -> String {
    format!("CLSID\\{CLSID}")
}

/// The key that names `extension`'s thumbnail handler, relative to `Software\Classes`.
pub fn shell_ex_key(extension: &str) -> String {
    format!("{extension}\\ShellEx\\{THUMBNAIL_SHELL_EX}")
}

/// The values that register the handler in the DLL at path `dll`: the class with its server and
/// threading model, and the class as the thumbnail handler of each of [`EXTENSIONS`], as
/// <https://learn.microsoft.com/windows/win32/shell/thumbnail-providers> describes.
pub fn registry_values(dll: &str) -> Vec<RegistryValue> {
    let class = class_key();
    let server = format!("{class}\\InprocServer32");
    let mut values = vec![
        RegistryValue { key: class, name: None, data: DESCRIPTION.into() },
        RegistryValue { key: server.clone(), name: None, data: dll.into() },
        RegistryValue { key: server, name: Some("ThreadingModel"), data: "Apartment".into() },
    ];
    values.extend(EXTENSIONS.iter().map(|ext| RegistryValue { key: shell_ex_key(ext), name: None, data: CLSID.into() }));
    values
}

/// Whether `value`, the default value of an extension's `ShellEx` key, names this handler: class
/// ids are compared without regard to letter case. `regsvr32 /u` removes only such keys.
pub fn names_this_handler(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case(CLSID)
}

/// Reads all of `r` when it holds at most `max` bytes, else returns an error.
pub fn read_bounded(r: impl Read, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    r.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |n| n > max) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("larger than {max} bytes")));
    }
    Ok(bytes)
}

/// The document Explorer passed as a stream. Its bytes are read on the first request and kept, so
/// a later request on the same handler, at another size for example, gets the same document.
pub struct Document<R> {
    stream: Option<R>,
    bytes: Option<Vec<u8>>,
}

impl<R: Read> Document<R> {
    pub fn new(stream: R) -> Self {
        Self { stream: Some(stream), bytes: None }
    }

    /// The document's bytes, read from the stream when it holds at most `max` bytes. When the
    /// first read fails, this call and every later one return an error.
    pub fn bytes(&mut self, max: u64) -> io::Result<&[u8]> {
        if let Some(stream) = self.stream.take() {
            self.bytes = Some(read_bounded(stream, max)?);
        }
        self.bytes.as_deref().ok_or_else(|| io::Error::other("the document's stream could not be read"))
    }
}

/// The `--size` for Explorer's request of a picture `cx` pixels on its longer side: `cx`, at most
/// [`MAX_SIDE`]; `None` for 0.
pub fn request_size(cx: u32) -> Option<u32> {
    (cx > 0).then(|| cx.min(MAX_SIDE))
}

/// The arguments of `vectorcraft-cli` that write the thumbnail of `input` to `output`, `size`
/// pixels on its longer side.
pub fn arguments(size: u32, input: &Path, output: &Path) -> Vec<OsString> {
    vec!["thumbnail".into(), "--size".into(), size.to_string().into(), input.into(), output.into()]
}

/// A file the handler made in the temp folder, removed when dropped.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes `bytes` to a new file `vectorcraft-thumbnail-<pid>-<n>.vectorcraft` in `dir`, with the
/// first `n` whose name no file has.
fn write_input(dir: &Path, bytes: &[u8]) -> io::Result<TempFile> {
    use std::io::Write;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    loop {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("vectorcraft-thumbnail-{}-{n}.vectorcraft", std::process::id()));
        match File::options().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let temp = TempFile(path);
                file.write_all(bytes)?;
                return Ok(temp);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
}

/// Runs `cli thumbnail --size SIZE IN OUT` with `bytes` written to a new file IN in `dir` and
/// returns the PNG it writes to OUT, IN's name with `.png`. The process is ended when it runs
/// longer than `deadline`. IN and OUT are removed before this returns.
pub fn render(cli: &Path, bytes: &[u8], size: u32, dir: &Path, deadline: Duration) -> Result<Vec<u8>, String> {
    let input = write_input(dir, bytes).map_err(|e| format!("{}: {e}", dir.display()))?;
    let output = TempFile(input.0.with_extension("png"));
    let mut command = Command::new(cli);
    command.args(arguments(size, &input.0, &output.0)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    no_console_window(&mut command);
    let mut child = command.spawn().map_err(|e| format!("{}: {e}", cli.display()))?;
    let end = Instant::now() + deadline;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("{}: {e}", cli.display()))? {
            break status;
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} ran longer than {} seconds", cli.display(), deadline.as_secs()));
        }
        std::thread::sleep(POLL);
    };
    if !status.success() {
        return Err(format!("{} exited with {status}", cli.display()));
    }
    let png = File::open(&output.0).and_then(|f| read_bounded(f, MAX_PNG)).map_err(|e| format!("{}: {e}", output.0.display()))?;
    Ok(png)
}

/// Starts `command` without a console window (`CREATE_NO_WINDOW`,
/// <https://learn.microsoft.com/windows/win32/procthread/process-creation-flags>).
#[cfg(windows)]
fn no_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_console_window(_: &mut Command) {}

/// The 8-bit RGBA PNG `png` (what `vectorcraft-cli thumbnail` writes, with straight alpha) as a
/// [`Picture`]. An error
/// when it can't be decoded, has another pixel format, or is empty or larger than `max_side`
/// pixels on a side.
pub fn picture(png: &[u8], max_side: u32) -> Result<Picture, String> {
    let mut reader = png::Decoder::new(io::Cursor::new(png)).read_info().map_err(|e| e.to_string())?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > max_side || height > max_side {
        return Err(format!("a {width} by {height} picture, not 1 to {max_side} pixels on each side"));
    }
    let mut buf = vec![0; reader.output_buffer_size().ok_or("the picture is too large")?];
    let frame = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if (frame.color_type, frame.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return Err(format!("a {:?} {:?} picture, not 8-bit RGBA", frame.bit_depth, frame.color_type));
    }
    let rgba = buf.get(..frame.buffer_size()).ok_or("the picture is cut short")?;
    let bgra = rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [premultiply(b, a), premultiply(g, a), premultiply(r, a), a]).collect();
    Ok(Picture { width, height, bgra })
}

/// Color `c` multiplied by alpha `a`, both 0 to 255, rounded to the nearest value.
fn premultiply(c: u8, a: u8) -> u8 {
    u8::try_from((u16::from(c) * u16::from(a) + 127) / 255).unwrap_or(u8::MAX)
}
