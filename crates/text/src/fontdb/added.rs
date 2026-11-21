//! Recently added fonts: the installed families with a font file created on this computer in the
//! last 30 days ([`FontDb::recently_added`]).

use std::time::{Duration, SystemTime};

use super::{FontDb, HashMap, Path};

/// How long a family counts as recently added after its newest font file was created: 30 days.
pub const RECENTLY_ADDED_WITHIN: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// The installed families added recently ([`FontDb::recently_added`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentlyAdded {
    /// The families by their catalog names, the one with the newest font file first. Families
    /// whose newest files were created at the same instant are in name order, case ignored.
    pub families: Vec<String>,
    /// The list is the same for every time from the one asked for up to this instant, and is to be
    /// asked for again from it on: when a listed family's newest file turns
    /// [`RECENTLY_ADDED_WITHIN`] old, or when a file dated after the time asked for starts to
    /// count. `None`: the list stays as it is until fonts are installed or removed.
    pub recheck_at: Option<SystemTime>,
}

impl FontDb {
    /// The installed families with a font file created within [`RECENTLY_ADDED_WITHIN`] before
    /// `now`, the boundary included, newest first. A family's date is that of its newest font file
    /// created at or before `now`. A file without a creation time (the file system keeps none)
    /// or dated after `now` does not count, so a family with no other file is left out. The
    /// creation time is when the file was made on this computer: copying a font in or installing
    /// it makes a new file, while a font kept elsewhere and linked or activated by a font manager
    /// is dated by the file the link leads to. Fonts loaded from bytes and the bundled fonts are
    /// never listed. Empty on wasm, which has no installed fonts.
    pub fn recently_added(&self, now: SystemTime) -> RecentlyAdded {
        let faces = self.catalog.faces();
        // Each file's creation time once for all its faces (a collection or a variable font's
        // named instances share one).
        let mut created: HashMap<&Path, Option<SystemTime>> = HashMap::new();
        // The families in catalog order, by ASCII-lowercased name, with their files' times.
        let mut families: Vec<(&str, Vec<Option<SystemTime>>)> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for f in faces.iter() {
            let Some(path) = f.path.as_deref() else { continue };
            let time = *created.entry(path).or_insert_with(|| std::fs::metadata(path).and_then(|m| m.created()).ok());
            let i = *index.entry(f.family.to_ascii_lowercase()).or_insert_with(|| {
                families.push((f.family.as_str(), Vec::new()));
                families.len() - 1
            });
            if let Some((_, times)) = families.get_mut(i) {
                times.push(time);
            }
        }
        recently_added_of(families, now)
    }
}

/// [`FontDb::recently_added`] of `families`, each a name with the creation times of its faces'
/// files (`None`: unknown).
pub(crate) fn recently_added_of<'a, T>(families: impl IntoIterator<Item = (&'a str, T)>, now: SystemTime) -> RecentlyAdded
where
    T: IntoIterator<Item = Option<SystemTime>>,
{
    let mut dated: Vec<(SystemTime, &str)> = Vec::new();
    let mut recheck_at: Option<SystemTime> = None;
    let mut sooner = |t: SystemTime| recheck_at = Some(recheck_at.map_or(t, |r| r.min(t)));
    for (name, times) in families {
        let mut newest: Option<SystemTime> = None;
        for t in times.into_iter().flatten() {
            if t > now {
                sooner(t);
            } else if newest.is_none_or(|n| t > n) {
                newest = Some(t);
            }
        }
        let Some(newest) = newest else { continue };
        match newest.checked_add(RECENTLY_ADDED_WITHIN) {
            Some(leaves) if leaves < now => {}
            Some(leaves) => {
                sooner(leaves);
                dated.push((newest, name));
            }
            None => dated.push((newest, name)),
        }
    }
    dated.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())).then_with(|| a.1.cmp(b.1)));
    RecentlyAdded { families: dated.into_iter().map(|(_, name)| name.to_string()).collect(), recheck_at }
}
