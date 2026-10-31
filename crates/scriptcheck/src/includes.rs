//! Following `#include` lines: the file each one names, found inside the script's folder and read
//! through the caller's [`Loader`].
//!
//! The preprocessor page (https://extendscript.docsforadobe.dev/extendscript-tools-features/preprocessor-directives/)
//! says that a name is looked for as given and then in each `#includepath` folder in turn, that a
//! name starting with `/` is absolute, and that `JSINCLUDE` is a fallback; it doesn't say which
//! folder a relative name starts from. The checker takes a name, and an `#includepath` folder, from
//! the folder of the file the line is in. It never follows an absolute name, a name or folder that
//! leads out of the script's folder (`..`), a name or folder longer than [`MAX_PATH`] bytes, or
//! `JSINCLUDE`. Each file is read once. A queue holds the files still to follow, in the order they
//! were read, and nothing recurses: each file is reached first at its smallest depth. At most
//! [`crate::Limits::lookups`] paths are looked up. Once every file is read, an
//! `#include` line is a cycle when the file it names includes the file the line is in, directly or
//! through other files.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use crate::analyze::{self, IncludeRef, Known};
use crate::directives::MAX_PATH;
use crate::report::{Acc, IncludeStatus};

/// An included file that was read and is checked: its path inside the script's folder, its bytes,
/// and the `#includepath` folders in force at its start.
pub(crate) struct ReadFile {
    pub path: String,
    pub bytes: Vec<u8>,
    pub paths: Rc<[String]>,
}

/// What a [`Loader`] found at a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loaded {
    /// The file's first bytes (at most the `max` asked for).
    Bytes(Vec<u8>),
    /// No file there, or it can't be read.
    Missing,
    /// A file the caller may not read (outside the folders automation may read).
    Refused,
}

/// Where the files a script includes come from: the folder the script is in.
pub trait Loader {
    /// The first `max` bytes of the file at `path`, a path inside the script's folder with `/`
    /// between its names (`lib/util.jsxinc`).
    fn load(&mut self, path: &str, max: usize) -> Loaded;
}

/// Follow the script's `includes`: `script` is its file name and `bytes` its size. Each file read
/// is scanned here for its own includes and for what its variables hold, with its tallies left
/// out; it starts with what `known` holds, which then gains what the file adds → the files to
/// check, in the order they were read.
pub(crate) fn follow(
    acc: &mut Acc,
    includes: Vec<IncludeRef>,
    loader: &mut dyn Loader,
    script: &str,
    bytes: usize,
    known: &mut Known,
) -> Vec<ReadFile> {
    /// The `#include` lines of a file that was read, still to follow.
    struct Work {
        /// The file the lines are in: its index in `files` (0: the script).
        file: usize,
        includes: Vec<IncludeRef>,
        depth: usize,
    }
    /// An `#include` line as the report lists it.
    struct Row {
        value: String,
        line: usize,
        /// The file the line is in.
        from: usize,
        status: IncludeStatus,
        path: Option<String>,
        /// The file it names, when that file was read.
        to: Option<usize>,
    }
    let limits = acc.limits;
    // The files read, by their path inside the script's folder; the script first.
    let mut files: Vec<String> = vec![script.to_string()];
    // What became of each path found, and the file read there: a file is read once.
    let mut read: BTreeMap<String, (IncludeStatus, Option<usize>)> = BTreeMap::from([(script.to_string(), (IncludeStatus::Checked, Some(0)))]);
    // The files still to follow, in the order they were read: each file is reached first at its
    // smallest depth.
    let mut work = VecDeque::from([Work { file: 0, includes, depth: 0 }]);
    let mut rows: Vec<Row> = vec![];
    // Which file includes which.
    let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    // The bytes read and the included files read, those not checked too, and the paths looked up.
    let (mut total, mut loaded, mut lookups) = (bytes, 0usize, 0usize);
    // The files to check.
    let mut checked: Vec<ReadFile> = vec![];
    while let Some(w) = work.pop_front() {
        let folder = if w.file == 0 { String::new() } else { files.get(w.file).map_or("", |f| parent(f)).to_string() };
        for inc in w.includes {
            acc.steps = acc.steps.saturating_add(1);
            let mut status = IncludeStatus::NotFollowed;
            let (mut found, mut to): (Option<String>, Option<usize>) = (None, None);
            let mut refused = false;
            for path in candidates(&folder, &inc.value, &inc.paths) {
                acc.steps = acc.steps.saturating_add(1);
                if let Some(&(before, file)) = read.get(&path) {
                    (status, found, to) = (before, Some(path), file);
                    break;
                }
                if w.depth + 1 > limits.include_depth || loaded >= limits.includes || total >= limits.total_bytes || lookups >= limits.lookups {
                    (status, found) = (IncludeStatus::Limit, None);
                    break;
                }
                lookups += 1;
                status = IncludeStatus::Missing;
                // One byte more than a file or the rest of the total may take shows that it is too large.
                let max = limits.file_bytes.min(limits.total_bytes.saturating_sub(total)).saturating_add(1);
                let b = match loader.load(&path, max) {
                    Loaded::Bytes(b) => b,
                    Loaded::Missing => continue,
                    Loaded::Refused => {
                        refused = true;
                        continue;
                    }
                };
                // Every file read counts, also one that is too large, compiled or not text.
                loaded += 1;
                total = total.saturating_add(b.len());
                acc.steps = acc.steps.saturating_add(b.len());
                status = if b.len() > limits.file_bytes || total > limits.total_bytes {
                    IncludeStatus::Limit
                } else if crate::is_compiled(&path, &b) || !crate::is_text(&b) {
                    IncludeStatus::Unreadable
                } else {
                    IncludeStatus::Checked
                };
                if status != IncludeStatus::Checked {
                    read.insert(path.clone(), (status, None));
                    found = Some(path);
                    break;
                }
                files.push(path.clone());
                let file = files.len() - 1;
                read.insert(path.clone(), (status, Some(file)));
                let (text, _) = crate::decode(&b);
                let mut tallies = Acc::new(limits);
                let text = analyze::unescaped(text, &mut tallies);
                let declared = analyze::declarations(&text, &mut tallies);
                let scan = analyze::scan(&text, &declared, Some(&path), Rc::clone(&inc.paths), known, &mut tallies);
                acc.steps = acc.steps.saturating_add(tallies.steps);
                known.add(scan.known, limits.entries);
                checked.push(ReadFile { path: path.clone(), bytes: b, paths: Rc::clone(&inc.paths) });
                work.push_back(Work { file, includes: scan.includes, depth: w.depth + 1 });
                (found, to) = (Some(path), Some(file));
                break;
            }
            if status == IncludeStatus::Missing && refused {
                status = IncludeStatus::NotFollowed;
            }
            if let Some(to) = to {
                edges.insert((w.file, to));
            }
            // One row more than the report keeps marks it truncated.
            if rows.len() <= limits.entries {
                rows.push(Row { value: acc.echo(&inc.value), line: inc.line, from: w.file, status, path: found, to });
            }
        }
    }
    let component = components(files.len(), &edges, acc);
    for r in rows {
        // An include closes a cycle when the file it includes leads back to the including one:
        // both are then in one strongly connected component.
        let cycle = r.to.and_then(|to| component.get(to)).is_some_and(|c| component.get(r.from) == Some(c));
        let status = if cycle { IncludeStatus::Cycle } else { r.status };
        let file = files.get(r.from).filter(|_| r.from > 0).map(String::as_str);
        acc.include(&r.value, r.line, file, status, r.path.as_deref());
    }
    checked
}

/// For each of `n` files, the number of its strongly connected component (files that include each
/// other, directly or through others, share one), with `edges` giving the files each one includes.
/// Tarjan's algorithm without recursion: linear in files and edges, where a row of reached files
/// for every file took their square.
fn components(n: usize, edges: &BTreeSet<(usize, usize)>, acc: &mut Acc) -> Vec<usize> {
    let mut next: Vec<Vec<usize>> = vec![vec![]; n];
    for &(from, to) in edges {
        if let (Some(v), true) = (next.get_mut(from), to < n) {
            v.push(to);
        }
    }
    const UNSEEN: usize = usize::MAX;
    let (mut index, mut low, mut component) = (vec![UNSEEN; n], vec![0; n], vec![UNSEEN; n]);
    let (mut on_stack, mut stack) = (vec![false; n], vec![]);
    let (mut counter, mut components) = (0, 0);
    for root in 0..n {
        if index[root] != UNSEEN {
            continue;
        }
        // Each frame: a file and how many of its includes have been followed.
        let mut frames = vec![(root, 0)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&(f, i)) = frames.last() {
            acc.steps = acc.steps.saturating_add(1);
            if let Some(&to) = next[f].get(i) {
                if let Some(top) = frames.last_mut() {
                    top.1 += 1;
                }
                if index[to] == UNSEEN {
                    index[to] = counter;
                    low[to] = counter;
                    counter += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    frames.push((to, 0));
                } else if on_stack[to] {
                    low[f] = low[f].min(index[to]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent] = low[parent].min(low[f]);
            }
            if low[f] == index[f] {
                while let Some(g) = stack.pop() {
                    on_stack[g] = false;
                    component[g] = components;
                    if g == f {
                        break;
                    }
                }
                components += 1;
            }
        }
    }
    component
}

/// List the script's `includes` as not read.
pub(crate) fn not_read(acc: &mut Acc, includes: Vec<IncludeRef>) {
    for inc in includes {
        acc.include(&inc.value, inc.line, None, IncludeStatus::NotRead, None);
    }
}

/// The folder part of `path` (`lib/a.jsxinc` → `lib`, `a.jsxinc` → ``).
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// A name the checker doesn't follow: absolute (`/x`, `\x`, `C:x`, `~/x`), naming an environment
/// variable (`%APPDATA%`, `$HOME`), or longer than [`MAX_PATH`].
fn unfollowed(name: &str) -> bool {
    name.starts_with(['/', '\\', '~', '$']) || name.contains([':', '%']) || name.len() > MAX_PATH
}

/// `rel` taken from `folder`, both inside the script's folder, as a path from that folder (empty
/// for the folder itself): none when it leads out of it.
fn walk(folder: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = folder.split('/').filter(|s| !s.is_empty()).collect();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The file `rel` names from `folder`, both inside the script's folder: none when it leads out of
/// it, or names the folder itself.
fn join(folder: &str, rel: &str) -> Option<String> {
    walk(folder, rel).filter(|path| !path.is_empty())
}

/// The paths an `#include` of `name` may mean, from `folder`, in the order they are tried and each
/// once: the name itself, then the name in each `#includepath` folder. Each is worked out when it
/// is tried. Names and folders the checker doesn't follow, and names or folders that lead out of
/// the script's folder, give none.
fn candidates<'a>(folder: &'a str, name: &'a str, paths: &'a [String]) -> impl Iterator<Item = String> + 'a {
    let name = Some(name).filter(|n| !unfollowed(n));
    let first = name.and_then(|n| join(folder, n));
    let inside = name
        .into_iter()
        .flat_map(move |n| paths.iter().filter(|p| !unfollowed(p)).filter_map(move |p| walk(folder, p)).filter_map(move |p| join(&p, n)));
    let mut tried: Vec<String> = vec![];
    first.into_iter().chain(inside).filter(move |p| {
        let new = !tried.contains(p);
        if new {
            tried.push(p.clone());
        }
        new
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tried(folder: &str, name: &str, paths: &[String]) -> Vec<String> {
        candidates(folder, name, paths).collect()
    }

    #[test]
    fn names_are_taken_from_the_including_files_folder() {
        assert_eq!(tried("", "a.jsxinc", &[]), ["a.jsxinc"]);
        assert_eq!(tried("lib", "b.jsxinc", &[]), ["lib/b.jsxinc"]);
        assert_eq!(tried("lib/deep", "../c.jsxinc", &[]), ["lib/c.jsxinc"]);
        assert_eq!(tried("lib", "./x\\y.jsxinc", &[]), ["lib/x/y.jsxinc"]);
        // Then each include path, inside the folder.
        let paths = ["shared".to_string(), "/abs".to_string(), "../out".to_string(), "shared".to_string()];
        assert_eq!(tried("", "u.jsxinc", &paths), ["u.jsxinc", "shared/u.jsxinc"]);
        // An include path may lead back to the script's own folder.
        assert_eq!(tried("lib", "shared.jsxinc", &["..".to_string()]), ["lib/shared.jsxinc", "shared.jsxinc"]);
    }

    #[test]
    fn absolute_names_and_names_leading_out_are_not_followed() {
        for name in
            ["/etc/x.jsx", "\\\\server\\x.jsx", "C:/x.jsx", "~/x.jsx", "%APPDATA%/x.jsx", "$HOME/x.jsx", "../x.jsx", "a/../../x.jsx", ".", "a/.."]
        {
            assert!(tried("", name, &[]).is_empty(), "{name}");
        }
        assert!(tried("lib", "../../x.jsx", &[]).is_empty());
        // Names and folders longer than a path can be.
        let long = "d".repeat(MAX_PATH + 1);
        assert!(tried("", &long, &[]).is_empty());
        assert_eq!(tried("", "a.jsxinc", &[long]), ["a.jsxinc"]);
    }
}
