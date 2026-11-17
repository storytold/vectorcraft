//! One app per user (#4892): a file opened from the file manager (double-click, Open With, a drop
//! on the executable) while VectorCraft runs opens as a new document tab in the running app, which
//! comes to the front, instead of in a second app.
//!
//! The running app (the primary) listens on a loopback port and records the port and two random
//! tokens in `instance.json` next to the preferences (owner-only on Unix; on Windows the settings
//! folder is the user's own). A later launch with files connects; the primary greets it with
//! `hello`, which only a process that could read the file (or talk to the primary) knows; only then
//! does the launch send `token` and the absolute paths of its files, one JSON line. The primary
//! checks the token, keeps the paths that are existing files, acknowledges and opens them; the
//! launch exits. Anything else (no file, no listener, a wrong greeting, no answer) starts the launch
//! as its own app, which replaces a stale file. Nothing received is ever run.
//!
//! Runs that start many apps side by side (`--control`, `VECTORCRAFT_NO_PREFS`) neither forward
//! nor listen, and neither does a `--safe-mode` launch, which opens its files in an app of its own
//! and leaves later launches to start theirs with plug-ins. `--new-instance` opens a launch's files
//! in an app of its own. An app that stays in Safe Mode after the choice about a stopped plug-in
//! stops listening ([`resign`]), and files later launches handed it before then open in a new app
//! with plug-ins ([`take_for_frame`]). macOS hands files opened from Finder to the running app itself
//! (`open_documents`), so this is off there.

use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher as _, RandomState};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use super::prefs::{prefs_enabled, prefs_path};

/// The endpoint file, next to the preferences.
const FILE_NAME: &str = "instance.json";
/// The longest message either side reads, and the longest endpoint file.
const MAX_MESSAGE: u64 = 64 * 1024;
/// The most files one launch hands over.
const MAX_FILES: usize = 256;
/// A live primary's kernel accepts at once, even while its window is busy; a stale port on Windows
/// would otherwise retry for about two seconds.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the primary waits on one launch; it serves launches one at a time.
const SERVER_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a launch waits on the primary: longer than a launch served before it can take.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Paths handed over by later launches that the UI hasn't opened yet.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// This app while it is the one later launches hand their files to ([`hold`]).
static HELD: Mutex<Option<Primary>> = Mutex::new(None);
/// The UI to wake when files arrive (unset until the window exists; the first frame opens them).
static UI: OnceLock<egui::Context> = OnceLock::new();

/// What this launch does about an app already running.
pub enum Start {
    /// The running app took the files: this launch exits.
    Forwarded,
    /// This launch runs; `Some` while it is the app later launches hand their files to.
    Running(Option<Primary>),
}

/// Hand `files` to the app already running, or become the app later launches hand theirs to.
/// `automation`: started with `--control`.
pub fn start(automation: bool, new_instance: bool, safe_mode: bool, files: &[String]) -> Start {
    if automation || !prefs_enabled() {
        return Start::Running(None);
    }
    let Some(path) = prefs_path().and_then(|p| Some(p.parent()?.join(FILE_NAME))) else { return Start::Running(None) };
    start_at(&path, new_instance, safe_mode, files)
}

/// [`start`] with the endpoint file `path`. A launch with `--new-instance` or `--safe-mode` keeps
/// its files and runs as its own app: a running app would open them with its plug-ins loaded. A
/// `--safe-mode` launch never becomes the app later launches hand their files to, which would open
/// them without plug-ins.
fn start_at(path: &Path, new_instance: bool, safe_mode: bool, files: &[String]) -> Start {
    if safe_mode {
        return Start::Running(None);
    }
    // With nothing to hand over, the same exchange only asks whether an app is running.
    let send: &[String] = if new_instance { &[] } else { files };
    match forward(path, send) {
        Ok(()) if send.is_empty() => Start::Running(None),
        Ok(()) => Start::Forwarded,
        Err(e) => {
            log::debug!("no running VectorCraft to hand files to ({e})");
            Start::Running(listen(path, queue))
        }
    }
}

/// Keep `primary` ([`start`]'s) until [`resign`].
pub fn hold(primary: Option<Primary>) {
    *HELD.lock().unwrap_or_else(PoisonError::into_inner) = primary;
}

/// Later launches start their own app from now on: the endpoint file goes. The app calls it when
/// its window is gone, and when it stays in Safe Mode after the choice about a stopped plug-in.
pub fn resign() {
    let primary = HELD.lock().unwrap_or_else(PoisonError::into_inner).take();
    drop(primary);
}

/// Wake `ctx` when files arrive from later launches.
pub fn set_ui(ctx: &egui::Context) {
    // Set once; a second call would hand over the same context.
    let _ = UI.set(ctx.clone());
}

/// The paths later launches handed over since the last call.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The paths later launches handed over since the last call, to open in this app. An app that
/// stays in Safe Mode (`safe_mode`) resigns and hands them to a new app with plug-ins (`launch`,
/// [`launch_with_plug_ins`] in the app) instead: they arrived while it was the primary, before the
/// choice about a stopped plug-in ended in Safe Mode, or just before it resigned. When `launch`
/// fails (logged), they open here.
pub fn take_for_frame(safe_mode: bool, launch: impl FnOnce(&[String]) -> io::Result<()>) -> Vec<String> {
    let files = take();
    if !safe_mode {
        return files;
    }
    // First, so the new app becomes the primary instead of handing the files back.
    resign();
    if files.is_empty() {
        return files;
    }
    match launch(&files) {
        Ok(()) => Vec::new(),
        Err(e) => {
            log::warn!("couldn't start VectorCraft with plug-ins for the files another launch opened: {e}");
            files
        }
    }
}

/// Start this program again, with plug-ins, on `files`.
pub fn launch_with_plug_ins(files: &[String]) -> io::Result<()> {
    app_with_plug_ins(&std::env::current_exe()?, files).spawn().map(drop)
}

/// The command [`launch_with_plug_ins`] runs: `exe -- files…`, so a file whose name starts with '-'
/// stays a file.
fn app_with_plug_ins(exe: &Path, files: &[String]) -> std::process::Command {
    let mut command = std::process::Command::new(exe);
    command.arg("--").args(files);
    command
}

fn queue(files: Vec<String>) {
    PENDING.lock().unwrap_or_else(PoisonError::into_inner).extend(files);
    if let Some(ctx) = UI.get() {
        ctx.request_repaint();
    }
}

/// This app is the one recorded in the endpoint file; dropping it removes the file, unless a later
/// app has replaced it since.
pub struct Primary {
    path: PathBuf,
    token: String,
}

impl Drop for Primary {
    fn drop(&mut self) {
        if read_endpoint(&self.path).is_some_and(|e| e.token == self.token) {
            // Best effort: a file left behind is stale, and the next launch replaces it.
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct Endpoint {
    port: u16,
    token: String,
    hello: String,
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

fn read_endpoint(path: &Path) -> Option<Endpoint> {
    let mut text = String::new();
    fs::File::open(path).ok()?.take(MAX_MESSAGE).read_to_string(&mut text).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let port = u16::try_from(v.get("port")?.as_u64()?).ok().filter(|p| *p != 0)?;
    Some(Endpoint { port, token: v.get("token")?.as_str()?.to_owned(), hello: v.get("hello")?.as_str()?.to_owned() })
}

/// Hand `files` (made absolute against this launch's folder) to the app recorded at `path`; `Ok`
/// once it acknowledged them.
fn forward(path: &Path, files: &[String]) -> io::Result<()> {
    let endpoint = read_endpoint(path).ok_or_else(|| invalid("no endpoint file"))?;
    let files = files.iter().map(|f| std::path::absolute(f).map(|p| p.to_string_lossy().into_owned())).collect::<io::Result<Vec<_>>>()?;
    let stream = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, endpoint.port).into(), CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    // The token and the paths go only to a listener that knows the greeting.
    if !same(read_message(&stream)?.get("hello"), &endpoint.hello) {
        return Err(invalid("the listener isn't VectorCraft"));
    }
    write_message(&stream, &json!({ "token": endpoint.token, "files": files }))?;
    match read_message(&stream)?.get("ok") {
        Some(Value::Bool(true)) => Ok(()),
        _ => Err(invalid("refused")),
    }
}

/// Listen for later launches and record this app at `path`; each launch's files go to `deliver`.
/// `None` when that fails (logged): the app then runs on its own, as before.
fn listen(path: &Path, deliver: impl Fn(Vec<String>) + Send + 'static) -> Option<Primary> {
    try_listen(path, deliver).inspect_err(|e| log::warn!("files opened while VectorCraft runs will open in another window: {e}")).ok()
}

fn try_listen(path: &Path, deliver: impl Fn(Vec<String>) + Send + 'static) -> io::Result<Primary> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let (token, hello) = random_tokens();
    write_endpoint(path, &json!({ "port": port, "token": token, "hello": hello }))?;
    // From here on a failure removes the file again.
    let primary = Primary { path: path.to_path_buf(), token: token.clone() };
    std::thread::Builder::new().name("open-files-listener".into()).spawn(move || {
        for stream in listener.incoming() {
            match stream.and_then(|s| receive(&s, &hello, &token)) {
                Ok(files) if files.is_empty() => {}
                Ok(files) => deliver(files),
                Err(e) => {
                    log::warn!("refused files from another launch: {e}");
                    // An accept that keeps failing (out of handles) doesn't spin.
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    })?;
    Ok(primary)
}

/// Serve one launch: greet it, check its token, keep the paths that are existing files.
fn receive(stream: &TcpStream, hello: &str, token: &str) -> io::Result<Vec<String>> {
    stream.set_read_timeout(Some(SERVER_TIMEOUT))?;
    stream.set_write_timeout(Some(SERVER_TIMEOUT))?;
    write_message(stream, &json!({ "hello": hello }))?;
    let message = read_message(stream)?;
    if !same(message.get("token"), token) {
        return Err(invalid("wrong token"));
    }
    let files = message.get("files").and_then(Value::as_array).ok_or_else(|| invalid("no file list"))?;
    if files.len() > MAX_FILES {
        return Err(invalid("too many files"));
    }
    let files = files
        .iter()
        .filter_map(Value::as_str)
        .filter(|f| {
            let p = Path::new(f);
            let ok = p.is_absolute() && p.is_file();
            if !ok {
                log::warn!("another launch named {f}, which isn't a file");
            }
            ok
        })
        .map(str::to_owned)
        .collect();
    write_message(stream, &json!({ "ok": true }))?;
    Ok(files)
}

/// One JSON line of at most [`MAX_MESSAGE`] bytes. Each side sends one line and waits for the
/// other's, so nothing read past it is lost.
fn read_message(stream: &TcpStream) -> io::Result<Value> {
    let mut line = Vec::new();
    BufReader::new(stream.take(MAX_MESSAGE)).read_until(b'\n', &mut line)?;
    if line.last() != Some(&b'\n') {
        return Err(invalid("message too long or cut short"));
    }
    serde_json::from_slice(&line).map_err(|e| invalid(&e.to_string()))
}

fn write_message(mut stream: &TcpStream, message: &Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)
}

/// `given` is the string `expected`, compared in constant time.
fn same(given: Option<&Value>, expected: &str) -> bool {
    given
        .and_then(Value::as_str)
        .is_some_and(|g| g.len() == expected.len() && g.bytes().zip(expected.bytes()).fold(0, |acc, (a, b)| acc | (a ^ b)) == 0)
}

/// Two unrelated 128-bit hex tokens. `RandomState` is keyed from the system's random numbers, and
/// its keyed hash of distinct values gives unpredictable, independent output (no crate needed).
fn random_tokens() -> (String, String) {
    let keyed = RandomState::new();
    let token = |a: u8, b: u8| format!("{:016x}{:016x}", keyed.hash_one(a), keyed.hash_one(b));
    (token(1, 2), token(3, 4))
}

/// Replace `path` with `endpoint` at once (a temporary file renamed over it), readable only by the
/// user on Unix.
fn write_endpoint(path: &Path, endpoint: &Value) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    // A leftover of an earlier app with the same process id; `create_new` sets the mode below.
    let _ = fs::remove_file(&tmp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let bytes = serde_json::to_vec(endpoint).map_err(io::Error::other)?;
    let written = options.open(&tmp).and_then(|mut f| f.write_all(&bytes)).and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        // Best effort: the temporary file is all there is to clean up.
        let _ = fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// Held by the tests that use the process-wide [`HELD`] and [`PENDING`], one at a time.
    static GLOBALS: Mutex<()> = Mutex::new(());

    fn globals() -> std::sync::MutexGuard<'static, ()> {
        GLOBALS.lock().unwrap_or_else(PoisonError::into_inner)
    }

    struct TestDir(tempfile::TempDir);

    impl std::ops::Deref for TestDir {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            self.0.path()
        }
    }

    /// A fresh folder for one test, with an existing file in it.
    fn folder(name: &str) -> (PathBuf, TestDir, String) {
        let dir = tempfile::Builder::new().prefix(&format!("vc-single-instance-{name}-")).tempdir().unwrap();
        let file = dir.path().join("a b.svg");
        fs::write(&file, "<svg/>").unwrap();
        let file = file.to_string_lossy().into_owned();
        (dir.path().join(FILE_NAME), TestDir(dir), file)
    }

    fn primary(path: &Path) -> (Primary, mpsc::Receiver<Vec<String>>) {
        let (tx, rx) = mpsc::channel();
        let primary = listen(path, move |files| tx.send(files).unwrap()).unwrap();
        (primary, rx)
    }

    /// Connects as a launch would and sends `message` after the greeting; what came back.
    fn send_raw(path: &Path, message: &[u8]) -> io::Result<Value> {
        let endpoint = read_endpoint(path).unwrap();
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        stream.set_read_timeout(Some(CLIENT_TIMEOUT)).unwrap();
        assert!(same(read_message(&stream).unwrap().get("hello"), &endpoint.hello));
        (&stream).write_all(message)?;
        read_message(&stream)
    }

    #[test]
    fn a_launch_hands_its_files_to_the_running_app() {
        let (path, dir, file) = folder("forward");
        let (primary, rx) = primary(&path);
        let missing = dir.join("missing.svg").to_string_lossy().into_owned();
        forward(&path, &[file.clone(), missing]).unwrap();
        assert_eq!(rx.recv_timeout(CLIENT_TIMEOUT).unwrap(), std::slice::from_ref(&file), "the missing file is left out");
        // An empty list only asks whether the app runs: nothing is delivered.
        forward(&path, &[]).unwrap();
        forward(&path, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rx.recv_timeout(CLIENT_TIMEOUT).unwrap(), [file]);
        drop(primary);
        assert!(!path.exists(), "the endpoint file goes with the app");
    }

    #[test]
    fn a_new_instance_or_safe_mode_launch_keeps_its_files() {
        let (path, _dir, file) = folder("own");
        let (primary, rx) = primary(&path);
        for (new_instance, safe_mode) in [(true, false), (false, true)] {
            let start = start_at(&path, new_instance, safe_mode, std::slice::from_ref(&file));
            assert!(matches!(start, Start::Running(None)), "runs as its own app (new instance {new_instance}, Safe Mode {safe_mode})");
        }
        assert!(matches!(start_at(&path, false, false, std::slice::from_ref(&file)), Start::Forwarded));
        assert_eq!(rx.recv_timeout(CLIENT_TIMEOUT).unwrap(), std::slice::from_ref(&file));
        assert!(rx.try_recv().is_err(), "only the plain launch handed its file over");
        drop(primary);
    }

    #[test]
    fn a_safe_mode_launch_leaves_later_launches_their_own_app() {
        let _globals = globals();
        let (path, _dir, file) = folder("safe-mode-alone");
        // With no app running, a Safe Mode launch runs without listening.
        assert!(matches!(start_at(&path, false, true, std::slice::from_ref(&file)), Start::Running(None)));
        assert!(!path.exists(), "a Safe Mode launch records no endpoint");
        // A plain launch after it starts its own app and takes later launches' files.
        let Start::Running(Some(primary)) = start_at(&path, false, false, std::slice::from_ref(&file)) else {
            panic!("the plain launch runs and listens")
        };
        assert!(path.exists());
        // An app that stays in Safe Mode after the choice gives that up.
        hold(Some(primary));
        resign();
        assert!(!path.exists(), "the endpoint file goes with resign");
    }

    #[test]
    fn files_handed_to_an_app_staying_in_safe_mode_go_to_a_new_app_with_plug_ins() {
        let _globals = globals();
        // Nothing left over from a test that failed.
        resign();
        drop(take());
        let (path, _dir, file) = folder("safe-mode-later");
        let Start::Running(Some(primary)) = start_at(&path, false, false, &[]) else { panic!("the first launch listens") };
        hold(Some(primary));
        // A later launch handed its file over while the choice about a stopped plug-in was up.
        queue(vec![file.clone()]);
        let mut launched = Vec::new();
        let here = take_for_frame(true, |files| {
            launched = files.to_vec();
            Ok(())
        });
        assert!(here.is_empty(), "nothing opens in the Safe Mode app");
        assert_eq!(launched, std::slice::from_ref(&file), "a new app with plug-ins opens it");
        assert!(!path.exists(), "the app resigned before starting the new one");
        // When the new app can't start, the file opens here rather than being lost.
        queue(vec![file.clone()]);
        assert_eq!(take_for_frame(true, |_| Err(io::Error::other("no program"))), std::slice::from_ref(&file));
        // An app with plug-ins opens what it is handed.
        queue(vec![file.clone()]);
        assert_eq!(take_for_frame(false, |_| panic!("no new app")), [file]);
    }

    #[test]
    fn the_new_app_takes_every_name_as_a_file() {
        let command = app_with_plug_ins(Path::new("vectorcraft"), &["-a.svg".to_owned(), "b.svg".to_owned()]);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--", "-a.svg", "b.svg"]);
    }

    #[test]
    fn a_wrong_token_garbage_and_oversized_messages_are_refused() {
        let (path, _dir, file) = folder("refused");
        let (_primary, rx) = primary(&path);
        let token = read_endpoint(&path).unwrap().token;
        let wrong = format!("{}\n", json!({ "token": "0".repeat(token.len()), "files": [file] }));
        assert!(send_raw(&path, wrong.as_bytes()).is_err());
        assert!(send_raw(&path, b"not json\n").is_err());
        assert!(send_raw(&path, &vec![b'a'; MAX_MESSAGE as usize + 10]).is_err());
        let many = format!("{}\n", json!({ "token": token, "files": vec![file.clone(); MAX_FILES + 1] }));
        assert!(send_raw(&path, many.as_bytes()).is_err());
        let relative = format!("{}\n", json!({ "token": token, "files": ["a b.svg", 7] }));
        assert_eq!(send_raw(&path, relative.as_bytes()).unwrap(), json!({ "ok": true }));
        assert!(rx.try_recv().is_err(), "nothing was delivered");
        // The listener still serves a launch with the right token.
        forward(&path, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rx.recv_timeout(CLIENT_TIMEOUT).unwrap(), [file]);
    }

    #[test]
    fn a_stale_endpoint_file_is_ignored_and_replaced() {
        let (path, _dir, file) = folder("stale");
        // The recorded app is gone: whatever holds the port closes without a greeting. (A freed
        // port can be taken by another test's listener meanwhile, which would then accept this
        // connection in place of its own client's.)
        let gone = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = gone.local_addr().unwrap().port();
        write_endpoint(&path, &json!({ "port": port, "token": "t", "hello": "h" })).unwrap();
        let closed = std::thread::spawn(move || drop(gone.accept()));
        assert!(forward(&path, std::slice::from_ref(&file)).is_err());
        closed.join().unwrap();
        // A listener that doesn't know the greeting gets neither the token nor the paths.
        let impostor = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = impostor.local_addr().unwrap().port();
        write_endpoint(&path, &json!({ "port": port, "token": "t", "hello": "h" })).unwrap();
        let heard = std::thread::spawn(move || {
            let (stream, _) = impostor.accept().unwrap();
            write_message(&stream, &json!({ "hello": "x" })).unwrap();
            stream.set_read_timeout(Some(CLIENT_TIMEOUT)).unwrap();
            read_message(&stream).ok()
        });
        assert!(forward(&path, std::slice::from_ref(&file)).is_err());
        assert!(heard.join().unwrap().is_none());
        // Garbage in the file is no endpoint either.
        fs::write(&path, b"\xff not json").unwrap();
        assert!(forward(&path, std::slice::from_ref(&file)).is_err());
        // A new primary replaces the stale file.
        let (_primary, rx) = primary(&path);
        forward(&path, std::slice::from_ref(&file)).unwrap();
        assert_eq!(rx.recv_timeout(CLIENT_TIMEOUT).unwrap(), [file]);
    }

    #[test]
    fn a_replaced_endpoint_file_stays_when_the_earlier_app_ends() {
        let (path, _dir, _) = folder("replaced");
        let (first, _rx1) = primary(&path);
        let (_second, _rx2) = primary(&path);
        let second_token = read_endpoint(&path).unwrap().token;
        drop(first);
        assert_eq!(read_endpoint(&path).map(|e| e.token), Some(second_token));
    }

    #[test]
    fn tokens_are_random_and_distinct() {
        let (a, b) = random_tokens();
        let (c, _) = random_tokens();
        assert_eq!(a.len(), 32);
        assert!(a != b && a != c);
    }

    #[cfg(unix)]
    #[test]
    fn the_endpoint_file_is_the_users_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let (path, _dir, _) = folder("mode");
        let _primary = primary(&path);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
