//! Time limits of remote storage requests.

use std::time::Duration;

use crate::remote_util::{META_TIMEOUT, TRANSFER_TIMEOUT, timeout_for, timeout_message};

/// Limits follow the method.
#[test]
fn limits_follow_the_method() {
    for m in ["PROPFIND", "HEAD", "MOVE", "DELETE"] {
        assert_eq!(timeout_for(m), META_TIMEOUT, "{m}");
    }
    for m in ["GET", "PUT"] {
        assert_eq!(timeout_for(m), TRANSFER_TIMEOUT, "{m}");
    }
    assert!(META_TIMEOUT < TRANSFER_TIMEOUT);
    let msg = timeout_message("PROPFIND", Duration::from_secs(20));
    assert!(msg.contains("PROPFIND") && msg.contains("20 s"), "{msg}");
}

/// A listing keeps its error until the retry runs a while.
#[test]
fn a_listing_keeps_its_error_until_the_retry_runs_a_while() {
    use crate::remote_util::{LOADING_GRACE_S, ListingView::*, listing_view};
    assert_eq!(listing_view(true, true, Some(30.0)), Entries);
    assert_eq!(listing_view(false, true, None), Error);
    assert_eq!(listing_view(false, true, Some(0.0)), Error);
    assert_eq!(listing_view(false, true, Some(LOADING_GRACE_S + 0.1)), Loading);
    assert_eq!(listing_view(false, false, None), Loading);
    assert_eq!(listing_view(false, false, Some(0.0)), Loading);
}
