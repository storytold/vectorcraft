//! Looking for the files of linked images and placed documents in a folder the user picks, and its
//! subfolders (`links.findFiles`, searched on separate threads: [`super::findfiles`]). A file with
//! a linked file's name, in any letter case, is ranked by what the link saved
//! ([`vectorcraft_doc::LinkInfo`]): the same file (its size and hash), the same size (when the link
//! saved no hash, or the file is larger than 1 GB and isn't hashed), or the name only.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::findfiles::{self, Limits, Visitor};
use super::*;

/// The most linked files one search looks for.
const MAX_LINKS: usize = 1_000;
/// The longest folder path given (bytes).
const MAX_PATH: usize = 4096;
/// The largest file a search hashes to compare with a link's saved hash. A larger file of the
/// saved size counts as the same size only.
const MAX_HASHED: u64 = 1 << 30;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "links.findFiles",
        "Find Linked Files",
        [],
        None,
        "{folder?: an absolute path, ids?: [image or placed document ids] (default: the active document's missing links), maxSeconds?: 60 (1 to 600), stop?: false (true: stop the last search)} with folder: look in it and its subfolders, on separate threads, for files with the names of the linked files (in any letter case), and answer at once with the search's state (searching while it runs); without folder: the last search's state, with the files found so far. Each linked file's files come best first: same (the size and hash saved with the link), size (the saved size, where there is no saved hash or the file is larger than 1 GB), then name. Within each kind, a file whose name has the link's letter case comes first. Then each file's folder names are compared with the saved path's, from the innermost outward up to the first one that differs, and a file with more matching names comes first; files that still tie are listed by path. A search looks for at most 1000 linked files and keeps the first 10 files of each one's name, and up to 10 that are the same file; it ends once every linked file has its same file, and searches only the folders text.findFontFiles may search, with the same maxSeconds, depth and count limits; none on the web → {id (absent while idle), state: idle|searching|done|stopped|failed, folder?, links: [{name, path, ids, files: [{path, size, match: same|size|name, caseDiffers?: true}]}], searched?: {folders, files, read}, skipped?, unreadable?, stopped?: stop|time|limit, error?, seconds?}",
        always,
        find
    )]
}

/// A linked file a search looks for: a file and content the objects `ids` show.
#[derive(Clone, Debug)]
pub(crate) struct Sought {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) ids: Vec<u64>,
    /// The file's size and hash when it was last read.
    pub(crate) size: Option<u64>,
    pub(crate) hash: Option<String>,
}

/// The session's last search for linked files (`links.findFiles`).
pub(crate) struct LinkSearch {
    /// The search's number in the session; [`stop`] takes it.
    pub(crate) id: u64,
    /// The folder as given.
    pub(crate) folder: String,
    pub(crate) links: Vec<Sought>,
    /// The size of each file read, by the path the search reports ([`LinkFiles::sizes`]).
    pub(crate) sizes: Arc<Mutex<HashMap<PathBuf, u64>>>,
    pub(crate) search: findfiles::Search,
}

/// The files a search for linked files reads: those with a linked file's name. Linked file `i` is
/// item `i` (a file of its name) and item `n + i` (the same file), of `n` linked files.
struct LinkFiles {
    /// The linked files' names in lowercase, each with the linked files of that name.
    names: HashMap<String, Vec<usize>>,
    /// Each linked file's saved size and hash.
    saved: Vec<(Option<u64>, Option<String>)>,
    /// The size of each file read, by the path the search reports ([`findfiles::plain`]).
    sizes: Arc<Mutex<HashMap<PathBuf, u64>>>,
}

impl LinkFiles {
    fn new(links: &[Sought], sizes: Arc<Mutex<HashMap<PathBuf, u64>>>) -> Self {
        let mut names: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, l) in links.iter().enumerate() {
            names.entry(l.name.to_lowercase()).or_default().push(i);
        }
        Self { names, saved: links.iter().map(|l| (l.size, l.hash.clone())).collect(), sizes }
    }
}

impl Visitor for LinkFiles {
    fn wants(&self, name: &str) -> bool {
        self.names.contains_key(&name.to_lowercase())
    }
    fn items(&self, path: &Path) -> Vec<usize> {
        let Some(of_name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| self.names.get(&n.to_lowercase())) else { return vec![] };
        let Ok(meta) = std::fs::symlink_metadata(path) else { return vec![] };
        let size = meta.len();
        let n = self.saved.len();
        // Read once, for the first linked file of its name saved with this size and a hash.
        let mut hash: Option<Option<String>> = None;
        let mut items = vec![];
        for &i in of_name {
            items.push(i);
            if let Some((Some(saved_size), Some(saved_hash))) = self.saved.get(i)
                && *saved_size == size
                && size <= MAX_HASHED
                && hash
                    .get_or_insert_with(|| {
                        // Only the size looked at is read: a file growing meanwhile no longer
                        // matches, rather than being hashed to its new end.
                        let file = std::fs::File::open(path).ok()?;
                        let (hash, read) = vectorcraft_doc::links::hash_reader_within(file, size).ok()?;
                        (read == size).then_some(hash)
                    })
                    .as_ref()
                    == Some(saved_hash)
            {
                items.push(n + i);
            }
        }
        lock(&self.sizes).insert(findfiles::plain(path.to_path_buf()), size);
        items
    }
}

/// The sizes map, also when a thread that held it panicked (entries are only inserted).
fn lock(sizes: &Mutex<HashMap<PathBuf, u64>>) -> MutexGuard<'_, HashMap<PathBuf, u64>> {
    sizes.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn find(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.findFiles";
    static NEXT: AtomicU64 = AtomicU64::new(1);
    if bool_or(p, "stop", false) {
        if let Some(l) = &s.link_search {
            l.search.stop();
        }
        return Ok(state(s));
    }
    let Some(folder) = p.get("folder").filter(|f| !f.is_null()) else { return Ok(state(s)) };
    let seconds = p.get("maxSeconds").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(60.0).clamp(1.0, 600.0);
    // Only checked here: the folder is read on a walker thread.
    let folder = folder
        .as_str()
        .filter(|f| !f.is_empty() && f.len() <= MAX_PATH && !f.contains(['\0', '\u{FFFD}']) && Path::new(f).is_absolute())
        .ok_or_else(|| bad(C, "`folder` must be an absolute folder path"))?
        .to_string();
    let st = s.active().ok_or_else(|| bad(C, "open a document to look for its linked files"))?;
    let ids = match p.get("ids") {
        None | Some(Value::Null) => None,
        Some(_) => {
            let ids = checked_ids_param(s, p, "ids", C)?;
            let linkable = |id: &NodeId| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_) | NodeKind::PlacedDocument(_)));
            if let Some(id) = ids.iter().find(|id| !linkable(id)) {
                return Err(bad(C, format!("object {} is not an image or a linked file", id.0)));
            }
            Some(ids)
        }
    };
    let links: Vec<Sought> = super::links::files_to_find(st, ids.as_deref())
        .into_iter()
        .take(MAX_LINKS)
        .map(|(link, ids)| Sought {
            name: link.name().to_string(),
            ids: ids.iter().map(|id| id.0).collect(),
            size: link.size,
            hash: link.hash,
            path: link.path,
        })
        .collect();
    if links.is_empty() {
        return Err(bad(C, if ids.is_some() { "these objects show no linked files" } else { "no linked files are missing" }));
    }
    // A new search stops the last one.
    s.link_search = None;
    let threads = s.search_threads.unwrap_or_else(findfiles::threads);
    let limits = Limits { seconds, ..Limits::default() };
    let sizes = Arc::new(Mutex::new(HashMap::new()));
    let visitor = Arc::new(LinkFiles::new(&links, sizes.clone()));
    let search =
        findfiles::start(PathBuf::from(&folder), s.search_rules.clone(), limits, threads, 2 * links.len(), visitor).map_err(EngineError::Other)?;
    s.link_search = Some(LinkSearch { id: NEXT.fetch_add(1, Ordering::Relaxed), folder, links, sizes, search });
    Ok(state(s))
}

/// How many folder names the file at `found` has in common with the saved path `saved` (whichever
/// system's separators it uses), counted from the innermost out up to the first that differs,
/// compared in lowercase.
fn shared_folders(saved: &str, found: &Path) -> usize {
    let saved = saved.rsplit(['/', '\\']).skip(1).filter(|c| !c.is_empty());
    let found = found.parent().into_iter().flat_map(|p| p.components().rev()).map_while(|c| match c {
        Component::Normal(n) => n.to_str(),
        _ => None,
    });
    saved.zip(found).take_while(|(a, b)| a.to_lowercase() == b.to_lowercase()).count()
}

/// Where the session's last search stands (`links.findFiles` without `folder`).
fn state(s: &Session) -> Value {
    let Some(l) = &s.link_search else { return json!({ "state": "idle", "links": [] }) };
    let p = l.search.progress();
    let n = l.links.len();
    let sizes = lock(&l.sizes).clone();
    // Per linked file: the files found, each with whether it is the same file.
    let mut found: Vec<Vec<(&Path, bool)>> = vec![vec![]; n];
    for (path, items) in &p.hits {
        let mut of: Vec<(usize, bool)> = vec![];
        for i in items {
            let (k, same) = if *i < n { (*i, false) } else { (i - n, true) };
            match of.iter_mut().find(|(j, _)| *j == k) {
                Some((_, s)) => *s |= same,
                None => of.push((k, same)),
            }
        }
        for (k, same) in of {
            if let Some(v) = found.get_mut(k) {
                v.push((path, same));
            }
        }
    }
    let links: Vec<Value> = l
        .links
        .iter()
        .zip(found)
        .map(|(link, files)| {
            let mut files: Vec<(u8, bool, usize, &Path, Option<u64>)> = files
                .into_iter()
                .map(|(path, same)| {
                    let size = sizes.get(path).copied();
                    // Compared by hash where the link saved one and the file isn't too large to hash.
                    let hashed = link.hash.is_some() && size.is_some_and(|z| z <= MAX_HASHED);
                    let rank = if same {
                        0
                    } else if size.is_some() && size == link.size && !hashed {
                        1
                    } else {
                        2
                    };
                    let case_differs = path.file_name().and_then(|f| f.to_str()) != Some(link.name.as_str());
                    (rank, case_differs, shared_folders(&link.path, path), path, size)
                })
                .collect();
            files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(b.2.cmp(&a.2)).then(a.3.cmp(b.3)));
            let files: Vec<Value> = files
                .into_iter()
                .map(|(rank, case_differs, _, path, size)| {
                    let kind = match rank {
                        0 => "same",
                        1 => "size",
                        _ => "name",
                    };
                    let mut f = json!({ "path": path.to_string_lossy(), "size": size, "match": kind });
                    if case_differs {
                        f["caseDiffers"] = json!(true);
                    }
                    f
                })
                .collect();
            json!({ "name": link.name, "path": link.path, "ids": link.ids, "files": files })
        })
        .collect();
    let folder = if p.folder.as_os_str().is_empty() { l.folder.clone() } else { p.folder.to_string_lossy().into_owned() };
    let mut out = json!({
        "id": l.id, "state": p.state(), "folder": folder, "links": links,
        "searched": { "folders": p.folders, "files": p.files, "read": p.read },
        "skipped": p.skipped, "unreadable": p.unreadable, "seconds": (p.seconds * 10.0).round() / 10.0,
    });
    if let Some(why) = p.stopped() {
        out["stopped"] = json!(why);
    }
    if let Some(e) = p.error() {
        out["error"] = json!(e);
    }
    out
}

/// Stop the session's search when it is the one numbered `id` (`links.findFiles`'s `id`): a newer
/// search, started by someone else, goes on.
pub fn stop(s: &mut Session, id: u64) {
    if let Some(l) = s.link_search.as_ref().filter(|l| l.id == id) {
        l.search.stop();
    }
}
