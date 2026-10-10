//! The folder search: where it goes and where it doesn't, its limits, its threads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use super::rules::{Siblings, unprefixed, without_verbatim};
use super::walk::{Pool, start_in};
use super::*;

/// The walker threads of these tests' searches, apart from the searches of other tests.
static TESTS: Pool = Pool::new(32);

/// [`start`] on the threads of [`TESTS`].
fn begin(folder: PathBuf, rules: Rules, limits: Limits, threads: usize, items: usize, visitor: Arc<dyn Visitor>) -> Search {
    start_in(&TESTS, folder, Some(rules), limits, threads, items, visitor).unwrap()
}

/// Reads `.ttf` files; each is the item its name is in the list.
struct Names(&'static [&'static str]);

impl Visitor for Names {
    fn wants(&self, name: &str) -> bool {
        name.ends_with(".ttf")
    }
    fn items(&self, path: &Path) -> Vec<usize> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        self.0.iter().position(|n| *n == name).into_iter().collect()
    }
}

/// Reads every file, and waits on reading one until its channel sends or closes.
struct Blocked(Mutex<mpsc::Receiver<()>>);

impl Visitor for Blocked {
    fn wants(&self, _: &str) -> bool {
        true
    }
    fn items(&self, _: &Path) -> Vec<usize> {
        let _ = self.0.lock().unwrap().recv();
        vec![]
    }
}

/// A fresh folder (canonical, as a search reports it) holding `paths`: files, and folders where
/// they end in `/`.
fn tree(tag: &str, paths: &[&str]) -> PathBuf {
    let root = vectorcraft_testkit::temp_dir(&format!("findfiles-{tag}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    for p in paths {
        let path = root.join(p);
        if p.ends_with('/') {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"font").unwrap();
        }
    }
    plain(std::fs::canonicalize(&root).unwrap())
}

/// The rules of a search of `root`: its `Users/me` is the home folder, `refused` an app's folder.
fn rules(root: &Path) -> Rules {
    Rules { home: Some(root.join("Users/me")), user_content: vec![root.to_path_buf()], refused: vec![root.join("refused")] }
}

/// The search's progress once it ended (or after 10 seconds).
fn wait(s: &Search) -> Progress {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let p = s.progress();
        if p.end.is_some() || Instant::now() > until {
            return p;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Search `from` in `root` for the files named `names`, to the end.
fn search(root: &Path, from: &str, limits: Limits, threads: usize, names: &'static [&'static str]) -> Progress {
    let s = begin(root.join(from), rules(root), limits, threads, names.len(), Arc::new(Names(names)));
    wait(&s)
}

/// The files a search found, relative to `root`, sorted.
fn found(root: &Path, p: &Progress) -> Vec<String> {
    let mut v: Vec<String> = p.hits.iter().map(|(f, _)| f.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")).collect();
    v.sort();
    v
}

#[test]
fn the_walk_stays_out_of_other_apps_and_the_systems_folders() {
    let root = tree(
        "roles",
        &[
            "a/b/c/one.ttf",
            "Users/me/Documents/two.ttf",
            "Disk/Fonts/four.ttf",
            "Disk/Users/you/Documents/four.ttf",
            // Never entered: each of these folders counts as skipped once.
            "Program Files/x/three.ttf",
            "Program Files (Arm)/x/three.ttf",
            "Users/me/Library/Fonts/three.ttf",
            "Users/me/.cache/three.ttf",
            "Users/me/AppData/Roaming/three.ttf",
            "Users/you/snap/three.ttf",
            "Users/Shared/x/three.ttf",
            "Pictures/P.photoslibrary/three.ttf",
            "Tools/Foo.app/Contents/three.ttf",
            "Copy/Library/Application Support/x/three.ttf",
            ".hidden/three.ttf",
            "refused/three.ttf",
            "Disk/System/Library/Fonts/three.ttf",
            "Disk/Library/Fonts/three.ttf",
            "Disk/Applications/three.ttf",
            "Disk/Users/you/Library/three.ttf",
        ],
    );
    let p = search(&root, "", Limits::default(), 3, &["one.ttf", "two.ttf", "three.ttf", "four.ttf"]);
    assert_eq!(p.end, Some(End::Done));
    assert_eq!(found(&root, &p), ["Disk/Fonts/four.ttf", "Disk/Users/you/Documents/four.ttf", "Users/me/Documents/two.ttf", "a/b/c/one.ttf"]);
    assert_eq!(p.skipped, 16);
    assert_eq!((p.read, p.unreadable), (4, 0));
}

#[test]
fn a_picked_folder_inside_other_apps_folders_is_refused() {
    let root = tree(
        "picks",
        &[
            "Users/me/Library/Fonts/",
            "Users/me/Library/CloudStorage/Box/F/",
            "Users/me/Library/Mobile Documents/x~y/Documents/",
            "Users/me/Library/Mobile Documents/x~y/Other/",
            "Users/me/.config/fonts/",
            "Users/Shared/x/",
            "Disk/Users/",
            "Disk/System/",
            "Disk/Library/Fonts/",
            "Tools/Foo.app/Contents/",
            "refused/sub/",
            "Program Files/x/",
            "Program Files (Arm)/x/",
            "Users/Public/x/",
            "a/.b/",
            "file.ttf",
        ],
    );
    for picked in [
        "Users/me/Library/Fonts",
        "Users/me/Library/Mobile Documents/x~y/Other",
        "Users/me/.config/fonts",
        "Disk/Library/Fonts",
        "Tools/Foo.app/Contents",
        "refused",
        "refused/sub",
        "Program Files/x",
        "Program Files (Arm)/x",
    ] {
        let p = search(&root, picked, Limits::default(), 1, &[]);
        assert_eq!(p.state(), "failed", "{picked}");
        assert!(p.error().is_some_and(|e| e.contains("doesn't search other apps' or the system's folders")), "{picked}: {p:?}");
    }
    for picked in ["Users/Shared/x", "Users/Public/x"] {
        let p = search(&root, picked, Limits::default(), 1, &[]);
        assert!(p.error().is_some_and(|e| e.contains("doesn't search the Shared or Public folder in Users")), "{picked}: {p:?}");
    }
    // Cloud folders, a hidden folder of the user's and another system's root are searched when
    // picked.
    for picked in ["Users/me/Library/CloudStorage/Box/F", "Users/me/Library/Mobile Documents/x~y/Documents", "a/.b", "Disk", "Disk/Users", ""] {
        let p = search(&root, picked, Limits::default(), 1, &[]);
        assert_eq!((p.state(), p.error()), ("done", None), "{picked}");
    }
    let p = search(&root, "file.ttf", Limits::default(), 1, &[]);
    assert!(p.error().is_some_and(|e| e.ends_with("is not a folder")), "{p:?}");
    let p = search(&root, "missing", Limits::default(), 1, &[]);
    assert!(p.error().is_some_and(|e| e.starts_with(&root.join("missing").display().to_string())), "{p:?}");
}

#[test]
fn the_search_ends_at_its_limits_and_when_stopped() {
    let root = tree("limits", &["one.ttf", "two.ttf", "a/b/c/three.ttf", "x/y.txt"]);
    let names = &["one.ttf", "two.ttf", "three.ttf", "nowhere.ttf"];
    // The picked folder and the folders in it are listed, not the ones below them.
    let p = search(&root, "", Limits { depth: 1, ..Limits::default() }, 2, names);
    assert_eq!(p.end, Some(End::Done));
    assert_eq!(found(&root, &p), ["one.ttf", "two.ttf"]);
    assert_eq!((p.folders, p.skipped), (3, 1));
    let p = search(&root, "", Limits { entries: 3, ..Limits::default() }, 1, names);
    assert_eq!((p.state(), p.stopped()), ("stopped", Some("limit")));
    let p = search(&root, "", Limits { files: 1, ..Limits::default() }, 1, names);
    assert_eq!((p.stopped(), p.read), (Some("limit"), 1));
    let p = search(&root, "", Limits { hits: 1, ..Limits::default() }, 1, names);
    assert_eq!((p.stopped(), p.hits.len()), (Some("limit"), 1));
    // Each item keeps at most `per_item` files.
    let root = tree("per-item", &["a/one.ttf", "b/one.ttf", "c/one.ttf"]);
    let p = search(&root, "", Limits { per_item: 2, ..Limits::default() }, 1, &["one.ttf", "nowhere.ttf"]);
    assert_eq!((p.end, p.hits.len(), p.read), (Some(End::Done), 2, 3));
}

#[test]
fn the_search_answers_at_its_deadline_while_a_walker_is_blocked() {
    let root = tree("blocked", &["f.ttf"]);
    let (tx, rx) = mpsc::channel();
    let s = begin(root.clone(), rules(&root), Limits { seconds: 0.2, ..Limits::default() }, 1, 1, Arc::new(Blocked(Mutex::new(rx))));
    std::thread::sleep(Duration::from_millis(300));
    let p = s.progress();
    assert_eq!((p.state(), p.stopped()), ("stopped", Some("time")), "{p:?}");
    assert!(p.seconds >= 0.2 && p.seconds < 5.0, "{p:?}");
    let (tx2, rx2) = mpsc::channel();
    let s2 = begin(root.clone(), rules(&root), Limits::default(), 1, 1, Arc::new(Blocked(Mutex::new(rx2))));
    let until = Instant::now() + Duration::from_secs(10);
    while s2.progress().read == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(s2.progress().state(), "searching");
    s2.stop();
    assert_eq!(s2.progress().stopped(), Some("stop"));
    drop((tx, tx2));
}

/// Every file is item 0; reading the second file waits until the channel closes.
struct SecondBlocks(Mutex<(usize, mpsc::Receiver<()>)>);

impl Visitor for SecondBlocks {
    fn wants(&self, _: &str) -> bool {
        true
    }
    fn items(&self, _: &Path) -> Vec<usize> {
        let mut read = self.0.lock().unwrap();
        read.0 += 1;
        if read.0 == 2 {
            let _ = read.1.recv();
        }
        vec![0]
    }
}

/// A file is kept as soon as it is read: progress shows it while the rest of its folder is read,
/// and a search that stops or reaches a limit keeps it.
#[test]
fn files_found_are_kept_as_they_are_read() {
    let root = tree("kept", &["a.ttf", "b.ttf", "c.ttf"]);
    let (tx, rx) = mpsc::channel();
    let s = begin(root.clone(), rules(&root), Limits::default(), 1, 2, Arc::new(SecondBlocks(Mutex::new((0, rx)))));
    let until = Instant::now() + Duration::from_secs(10);
    while s.progress().read < 2 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    let p = s.progress();
    assert_eq!((p.state(), p.hits.len()), ("searching", 1), "{p:?}");
    s.stop();
    drop(tx);
    let p = wait(&s);
    assert_eq!(p.stopped(), Some("stop"));
    assert!(!p.hits.is_empty(), "{p:?}");
    let p = search(&root, "", Limits { files: 2, ..Limits::default() }, 1, &["a.ttf", "b.ttf", "c.ttf", "nowhere.ttf"]);
    assert_eq!((p.stopped(), p.hits.len()), (Some("limit"), 2), "{p:?}");
}

#[test]
fn the_search_ends_once_every_item_has_a_file() {
    let root = tree("found", &["a/one.ttf", "b/two.ttf", "c/d/e/f/g/other.ttf"]);
    let p = search(&root, "", Limits::default(), 2, &["one.ttf", "two.ttf"]);
    assert_eq!((p.end.clone(), found(&root, &p)), (Some(End::Found), vec!["a/one.ttf".to_string(), "b/two.ttf".into()]));
}

#[test]
fn one_walker_and_eight_find_the_same_files_and_every_search_ends() {
    let mut paths = vec![];
    for a in 0..6 {
        for b in 0..4 {
            paths.push(format!("d{a}/e{b}/one.ttf"));
            paths.push(format!("d{a}/e{b}/f/two.ttf"));
            paths.push(format!("d{a}/e{b}/f/g/skip.txt"));
        }
    }
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let root = tree("threads", &paths);
    let limits = Limits { per_item: 100, ..Limits::default() };
    let one = search(&root, "", limits, 1, &["one.ttf", "two.ttf", "nowhere.ttf"]);
    assert_eq!((one.end.clone(), one.hits.len(), one.folders), (Some(End::Done), 48, 1 + 6 + 24 + 24 + 24));
    for round in 0..25 {
        for threads in 1..=8 {
            let p = search(&root, "", limits, threads, &["one.ttf", "two.ttf", "nowhere.ttf"]);
            assert_eq!(p.end, Some(End::Done), "round {round}, {threads} threads");
            assert_eq!(found(&root, &p), found(&root, &one), "round {round}, {threads} threads");
        }
    }
}

/// Fails on `boom.ttf` with a panic: a bug in reading one file.
struct Boom;

impl Visitor for Boom {
    fn wants(&self, name: &str) -> bool {
        name.ends_with(".ttf")
    }
    fn items(&self, path: &Path) -> Vec<usize> {
        match path.file_name().and_then(|n| n.to_str()) {
            Some("boom.ttf") => panic!("a bug"),
            Some("ok.ttf") => vec![0],
            _ => vec![1],
        }
    }
}

#[test]
fn a_visitor_that_panics_on_one_file_does_not_end_the_search() {
    // The file beside the one that trips the bug is found too.
    let root = tree("boom", &["a/boom.ttf", "a/ok.ttf", "b/c/fine.ttf"]);
    let s = begin(root.clone(), rules(&root), Limits::default(), 2, 3, Arc::new(Boom));
    let p = wait(&s);
    assert_eq!((p.end.clone(), found(&root, &p), p.unreadable), (Some(End::Done), vec!["a/ok.ttf".to_string(), "b/c/fine.ttf".into()], 1));
}

#[cfg(unix)]
#[test]
fn links_are_not_followed_and_unreadable_folders_count() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tree("links", &["outside/x.ttf", "inner/locked/y.ttf"]);
    let inner = root.join("inner");
    symlink(root.join("outside"), inner.join("folder")).unwrap();
    symlink(&inner, inner.join("loop")).unwrap();
    symlink(root.join("outside/x.ttf"), inner.join("x.ttf")).unwrap();
    let locked = inner.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Where permissions don't hold (root), the folder reads as any other.
    let enforced = std::fs::read_dir(&locked).is_err();
    let p = search(&root, "inner", Limits::default(), 2, &["x.ttf", "y.ttf"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(p.end, Some(End::Done));
    if enforced {
        assert!(p.hits.is_empty() && p.unreadable == 1, "{p:?}");
    } else {
        assert_eq!(found(&root, &p), ["inner/locked/y.ttf"]);
    }
    // The linked file isn't a file of the folder; y.ttf is one where the locked folder reads.
    assert_eq!(p.files, u64::from(!enforced), "{p:?}");
}

#[cfg(unix)]
#[test]
fn a_picked_folder_that_cant_be_listed_fails_the_search() {
    use std::os::unix::fs::PermissionsExt;
    let root = tree("unlistable", &["locked/y.ttf"]);
    let locked = root.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Where permissions don't hold (root), the folder reads as any other.
    let enforced = std::fs::read_dir(&locked).is_err();
    let p = search(&root, "locked", Limits::default(), 2, &["y.ttf"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if enforced {
        assert!(matches!(&p.end, Some(End::Failed(e)) if e.contains("locked")), "{p:?}");
    } else {
        assert_eq!(found(&root, &p), ["locked/y.ttf"]);
    }
}

#[test]
fn system_roots_and_homes_are_told_by_what_they_hold() {
    let r = Rules { home: Some(PathBuf::from("/home/me")), ..Rules::default() };
    let never = |parent: &str, siblings: Siblings, name: &str| r.never(Path::new(parent), siblings, name);
    let none = Siblings::default();
    let mac = Siblings::of(&["Users", "Library", "System", "Applications", "Volumes", "opt"]);
    for name in ["Library", "System", "Applications", "opt", "private"] {
        assert!(never("/", mac, name), "{name}");
    }
    for name in ["Users", "Volumes", "Fonts"] {
        assert!(!never("/", mac, name), "{name}");
    }
    let unix = Siblings::of(&["usr", "etc", "bin", "home", "nix", "srv", "data", "mnt"]);
    for name in ["usr", "etc", "bin", "nix", "var", "proc"] {
        assert!(never("/", unix, name), "{name}");
    }
    for name in ["home", "srv", "data", "mnt", "media"] {
        assert!(!never("/", unix, name), "{name}");
    }
    let windows = Siblings::of(&["Windows", "Users", "Program Files", "Data"]);
    for name in ["Windows", "Program Files", "Recovery"] {
        assert!(never("C:\\", windows, name), "{name}");
    }
    for name in ["Data", "Users"] {
        assert!(!never("C:\\", windows, name), "{name}");
    }
    // A project that holds a bin and an etc folder isn't the root of a system.
    let project = Siblings::of(&["bin", "etc", "src"]);
    assert!(!never("/work/p", project, "bin") && !never("/work/p", project, "etc"));
    // Home folders: the home folder, and every folder in a Users or home folder.
    for name in ["Library", ".config", "snap", "AppData", "Applications"] {
        assert!(never("/home/me", none, name), "{name}");
        assert!(never("/Users/you", none, name), "{name}");
    }
    assert!(!never("/home/me", none, "Documents") && !never("/home/me/Documents", none, "Library"));
    assert!(never("/run/user/1000/doc/a1b2/me", none, "snap"), "a folder the document portal exports");
    assert!(never("/Users", none, "Shared") && never("/Users", none, "Public"));
    assert!(never("/Volumes/Copy/Library", none, "Application Support") && !never("/Volumes/Copy/Library", none, "Fonts"));
    assert!(never("/x", none, "Foo.app") && never("/x", none, "Program Files (x86)") && !never("/x", none, "Fonts.d"));
    assert!(never("/x", none, "Program Files (Arm)"));
    // A system root's walk reaches a home folder below a folder it skips (FreeBSD's /usr/home).
    let bsd = Rules { home: Some(PathBuf::from("/usr/home/me")), ..Rules::default() };
    let root = Path::new("/");
    assert_eq!(bsd.home_below(&bsd.listed(root, unix)), Some(PathBuf::from("/usr/home/me")));
    assert_eq!(r.home_below(&r.listed(root, unix)), None);
    assert_eq!(bsd.home_below(&bsd.listed(root, project)), None);
}

#[test]
fn a_search_starts_with_the_threads_left_and_fails_when_none_is() {
    static POOL: Pool = Pool::new(3);
    let root = tree("pool", &["f.ttf"]);
    let blocked = || {
        let (tx, rx) = mpsc::channel();
        (tx, Arc::new(Blocked(Mutex::new(rx))) as Arc<dyn Visitor>)
    };
    let (tx_a, a) = blocked();
    let (tx_b, b) = blocked();
    let a = start_in(&POOL, root.clone(), Some(rules(&root)), Limits::default(), 2, 1, a).unwrap();
    let b = start_in(&POOL, root.clone(), Some(rules(&root)), Limits::default(), 4, 1, b).unwrap();
    assert_eq!(POOL.live(), 3);
    let (_, c) = blocked();
    let c = start_in(&POOL, root.clone(), Some(rules(&root)), Limits::default(), 1, 1, c);
    assert_eq!(c.err().as_deref(), Some("the search couldn't start: try again in a moment"));
    drop((tx_a, tx_b));
    assert_eq!((wait(&a).end, wait(&b).end), (Some(End::Done), Some(End::Done)));
    let until = Instant::now() + Duration::from_secs(10);
    while POOL.live() > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(POOL.live(), 0, "every walker thread ended");
    let s = start_in(&POOL, root.clone(), Some(rules(&root)), Limits::default(), 8, 1, Arc::new(Names(&[]))).unwrap();
    assert_eq!(wait(&s).end, Some(End::Done));
}

#[test]
fn a_search_takes_the_cpus_the_renderer_leaves_from_two_to_eight() {
    use super::walk::threads_for;
    assert_eq!([threads_for(64, 4), threads_for(12, 4), threads_for(4, 4), threads_for(1, 0)], [8, 7, 2, 2]);
    assert!((2..=8).contains(&threads()));
}

/// A folder holding more folders than a search queues ends the search while it is being listed.
#[test]
fn a_folder_with_more_folders_than_the_search_queues_ends_it_as_it_is_listed() {
    // Folders and files made in turns: whatever order a file system lists them in, about half of a
    // folder's first entries are folders.
    let names: Vec<String> = (0..2048).flat_map(|i| [format!("d{i}/"), format!("f{i}.txt")]).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let root = tree("many-folders", &names);
    let p = search(&root, "", Limits { folders: 10, ..Limits::default() }, 1, &[]);
    assert_eq!(p.end, Some(End::Limit));
    assert!(p.files < 2048, "the folder was listed to its end: {p:?}");
}

/// Wants every file, counting the files it is asked about.
struct Counted(AtomicUsize);

impl Visitor for Counted {
    fn wants(&self, _: &str) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed);
        true
    }
    fn items(&self, _: &Path) -> Vec<usize> {
        vec![]
    }
}

/// A folder's files past those the search reads aren't kept to be read.
#[test]
fn files_past_those_the_search_reads_are_not_kept() {
    let names: Vec<String> = (0..50).map(|i| format!("f{i}.ttf")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let root = tree("many-files", &names);
    let counted = Arc::new(Counted(AtomicUsize::new(0)));
    let p = wait(&begin(root.clone(), rules(&root), Limits { files: 3, ..Limits::default() }, 1, 1, counted.clone()));
    assert_eq!((p.stopped(), p.read, p.files), (Some("limit"), 3, 50), "{p:?}");
    assert_eq!(counted.0.load(Ordering::Relaxed), 4, "one more than the files read");
}

/// Wants `.ttf` files by their names, turns down those whose names start with `skip` by the file,
/// and finds `font.ttf`.
struct Picky;

impl Visitor for Picky {
    fn wants(&self, name: &str) -> bool {
        name.ends_with(".ttf")
    }
    fn worth_reading(&self, path: &Path) -> bool {
        !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("skip"))
    }
    fn items(&self, path: &Path) -> Vec<usize> {
        if path.ends_with("font.ttf") { vec![0] } else { vec![] }
    }
}

/// Files a visitor turns down by the file itself aren't read and don't count among the files
/// read: with room to read one file, the search gets past them to the font in the folder below.
#[test]
fn files_turned_down_by_the_file_are_not_read() {
    let root = tree("turned-down", &["skip1.ttf", "skip2.ttf", "skip3.ttf", "sub/font.ttf"]);
    let p = wait(&begin(root.clone(), rules(&root), Limits { files: 1, ..Limits::default() }, 1, 1, Arc::new(Picky)));
    assert_eq!((p.end.clone(), p.read, found(&root, &p)), (Some(End::Found), 1, vec!["sub/font.ttf".to_string()]), "{p:?}");
}

/// Windows paths lose the `\\?\` prefix `canonicalize` gives them where they read the same
/// without it. Paths compare the same with the prefix and without it.
#[test]
fn windows_paths_lose_the_verbatim_prefix_where_they_read_the_same() {
    for (verbatim, without) in [
        (r"\\?\C:\Users\me\Fonts", r"C:\Users\me\Fonts"),
        (r"\\?\d:\", r"d:\"),
        (r"\\?\UNC\server\share\a b\c.otf", r"\\server\share\a b\c.otf"),
        (r"\\?\UNC\server\share", r"\\server\share"),
        (r"\\?\UNC\server\share\", r"\\server\share\"),
    ] {
        assert_eq!(without_verbatim(verbatim).as_deref(), Some(without), "{verbatim}");
    }
    // For comparing, any name goes.
    assert_eq!(unprefixed(r"\\?\C:\a.\b").as_deref(), Some(r"C:\a.\b"));
    assert_eq!(unprefixed(r"\\?\UNC\server").as_deref(), Some(r"\\server"));
    assert_eq!((unprefixed(r"\\?\C:"), unprefixed(r"C:\a"), unprefixed("/home/me")), (None, None, None));
    for kept in [
        r"\\?\C:",
        r"\\?\C:\a.\b",
        r"\\?\C:\a \b",
        r"\\?\C:\CON\b",
        r"\\?\C:\a\nul.txt",
        r"\\?\C:\a\COM¹",
        r"\\?\C:\a/b",
        r"\\?\C:\a\..\b",
        r"\\?\C:\a\\b",
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\a",
        r"\\?\UNC\server",
        r"C:\a",
        "/home/me",
    ] {
        assert_eq!(without_verbatim(kept), None, "{kept}");
    }
    if !cfg!(windows) {
        assert_eq!(plain(PathBuf::from(r"\\?\C:\a")), PathBuf::from(r"\\?\C:\a"), "other systems' paths stay as they are");
    }
}
