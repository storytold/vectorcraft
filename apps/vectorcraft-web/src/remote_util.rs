//! Pure helpers of the browser's remote storage: request time limits.
//!
//! No browser types, so it also builds natively for its tests.

use std::time::Duration;

/// Time limit of a request that moves no file data (PROPFIND, HEAD, MOVE, DELETE).
pub const META_TIMEOUT: Duration = Duration::from_secs(20);
/// Time limit of a whole file transfer (GET, PUT), body included.
pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);
/// Time limit of the config file read at startup (a stuck server must not hold the app back).
pub const CONFIG_TIMEOUT: Duration = Duration::from_secs(8);

/// How long a request with `method` may take.
pub fn timeout_for(method: &str) -> Duration {
    match method {
        "GET" | "PUT" => TRANSFER_TIMEOUT,
        _ => META_TIMEOUT,
    }
}

/// The error of a request that ran out of time.
pub fn timeout_message(method: &str, limit: Duration) -> String {
    format!("the server did not finish the {method} request within {} s", limit.as_secs())
}

/// A retried listing keeps showing its last error for this long (seconds) before it says
/// "Loading…": a retry that fails at once would otherwise flash between the two.
pub const LOADING_GRACE_S: f64 = 1.0;

/// What the dialog shows for a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingView {
    Entries,
    Error,
    Loading,
}

/// `loading_for`: how long (seconds) the request in flight has been running, `None` when none is.
/// The last error stays up until a retry has been running for [`LOADING_GRACE_S`].
pub fn listing_view(has_entries: bool, has_error: bool, loading_for: Option<f64>) -> ListingView {
    if has_entries {
        ListingView::Entries
    } else if has_error && loading_for.is_none_or(|t| t < LOADING_GRACE_S) {
        ListingView::Error
    } else {
        ListingView::Loading
    }
}
