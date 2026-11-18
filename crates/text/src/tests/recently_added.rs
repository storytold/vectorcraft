//! Recently added fonts ([`FontDb::recently_added`]): which installed families count, in which
//! order, and until when, at a time the tests choose.

use std::time::{Duration, SystemTime};

use crate::fontdb::recently_added_of;
use crate::*;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// A fixed instant the tests count from.
fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

/// `days` days (and `secs` seconds) before [`now`].
fn ago(days: u32, secs: u64) -> Option<SystemTime> {
    now().checked_sub(DAY * days + Duration::from_secs(secs))
}

/// [`recently_added_of`] for families with these creation times, at `at`.
fn added(families: &[(&str, &[Option<SystemTime>])], at: SystemTime) -> RecentlyAdded {
    recently_added_of(families.iter().map(|(name, times)| (*name, times.iter().copied())), at)
}

/// A family counts for 30 days after its newest file was created, the 30th day's last instant
/// included; the newest comes first.
#[test]
fn families_added_in_the_last_30_days_are_listed_newest_first() {
    let families: &[(&str, &[Option<SystemTime>])] =
        &[("Older", &[ago(29, 0)]), ("Boundary", &[ago(30, 0)]), ("Expired", &[ago(30, 1)]), ("Newest", &[ago(0, 5)]), ("Middle", &[ago(3, 0)])];
    let list = added(families, now());
    assert_eq!(list.families, ["Newest", "Middle", "Older", "Boundary"]);
    assert_eq!(list.recheck_at, Some(now()), "Boundary leaves after this instant");
    // A day and a second later, the two oldest have left, without the files changing.
    let later = added(families, now() + DAY + Duration::from_secs(1));
    assert_eq!(later.families, ["Newest", "Middle"]);
    assert_eq!(later.recheck_at, ago(3, 0).map(|t| t + RECENTLY_ADDED_WITHIN));
}

/// A family is dated by its newest file at or before the time asked for. Files without a creation
/// time, or dated after that time, do not count, and a family with no other file is left out until
/// its future-dated file's time comes.
#[test]
fn a_family_is_dated_by_its_newest_valid_file() {
    let future = now() + DAY;
    let families: &[(&str, &[Option<SystemTime>])] = &[
        ("Several", &[ago(40, 0), ago(2, 0), None, Some(future)]),
        ("Unknown", &[None, None]),
        ("Future", &[Some(future)]),
        ("Old", &[ago(31, 0)]),
        ("One", &[ago(1, 0)]),
    ];
    let list = added(families, now());
    assert_eq!(list.families, ["One", "Several"]);
    assert_eq!(list.recheck_at, Some(future), "the future-dated files count from their time on");
    assert_eq!(added(families, future).families, ["Future", "Several", "One"]);
    assert!(added(&[("Unknown", &[None])], now()).recheck_at.is_none());
}

/// Families added at the same instant are in name order, case ignored, whatever order the
/// catalog gives them in.
#[test]
fn families_added_at_the_same_instant_are_in_name_order() {
    let t = ago(1, 0);
    let families: &[(&str, &[Option<SystemTime>])] = &[("delta", &[t]), ("Bravo", &[t]), ("alpha", &[t]), ("Charlie", &[t])];
    assert_eq!(added(families, now()).families, ["alpha", "Bravo", "Charlie", "delta"]);
    let reversed: Vec<_> = families.iter().rev().copied().collect();
    assert_eq!(added(&reversed, now()).families, ["alpha", "Bravo", "Charlie", "delta"]);
}

/// Nothing installed, or nothing dated: an empty list with nothing to recheck.
#[test]
fn no_dated_families_list_nothing() {
    assert_eq!(added(&[], now()), RecentlyAdded::default());
    assert_eq!(FontDb::with_font_dirs(vec![]).recently_added(now()), RecentlyAdded::default(), "the bundled fonts are not installed fonts");
}
