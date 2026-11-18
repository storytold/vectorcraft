//! Which `demand` web faces (`crates/text/web-fonts.txt`) to fetch: each face is fetched at most
//! once at a time and never again once it loaded. A face whose fetch failed is fetched again after
//! a wait that doubles with each failure, and not after [`MAX_TRIES`] failures.

/// The wait after a face's first failed fetch before it is fetched again (ms).
const RETRY_MS: i32 = 30_000;
/// Failed fetches of a face after which it is not fetched again this session.
pub(crate) const MAX_TRIES: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    /// Being fetched, after `tries` failed fetches.
    Loading {
        tries: u32,
    },
    Loaded,
    /// `tries` fetches failed, the last at `failed_at` (ms); the next may start at `retry_at`, or
    /// at once when the clock reads earlier than `failed_at`.
    Failed {
        tries: u32,
        failed_at: f64,
        retry_at: f64,
    },
}

/// The fetch state of each face of a web font list, by index.
#[derive(Debug, Default)]
pub(crate) struct Demand {
    states: Vec<State>,
}

impl Demand {
    /// Of the faces `wanted` (indexes into the list), the ones to fetch at time `now` (ms): not
    /// loaded, not being fetched, and not waiting to be fetched again. They are marked as being
    /// fetched.
    pub(crate) fn start(&mut self, wanted: &[usize], now: f64) -> Vec<usize> {
        let mut out = Vec::new();
        for &i in wanted {
            if self.states.len() <= i {
                self.states.resize(i + 1, State::Idle);
            }
            let Some(state) = self.states.get_mut(i) else { continue };
            let tries = match *state {
                State::Idle => Some(0),
                State::Loading { .. } | State::Loaded => None,
                State::Failed { tries, failed_at, retry_at } => (tries < MAX_TRIES && (now >= retry_at || now < failed_at)).then_some(tries),
            };
            if let Some(tries) = tries {
                *state = State::Loading { tries };
                out.push(i);
            }
        }
        out
    }

    /// The fetch of face `i` started by [`Demand::start`] ended at `now` (ms): loaded when `ok`.
    /// Returns the wait (ms) after which a failed face may be fetched again, or `None` when it
    /// loaded or has failed [`MAX_TRIES`] times.
    pub(crate) fn finished(&mut self, i: usize, ok: bool, now: f64) -> Option<i32> {
        let state = self.states.get_mut(i)?;
        let State::Loading { tries } = *state else { return None };
        if ok {
            *state = State::Loaded;
            return None;
        }
        let tries = tries + 1;
        let wait = RETRY_MS << (tries - 1).min(4);
        *state = State::Failed { tries, failed_at: now, retry_at: now + f64::from(wait) };
        (tries < MAX_TRIES).then_some(wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_is_fetched_once_and_kept() {
        let mut d = Demand::default();
        assert_eq!(d.start(&[2], 0.0), [2]);
        assert!(d.start(&[2], 1.0).is_empty(), "already being fetched");
        assert_eq!(d.finished(2, true, 2.0), None, "loaded: no retry");
        assert!(d.start(&[2], 1e9).is_empty(), "loaded: never fetched again");
        assert_eq!(d.start(&[0, 2, 1], 3.0), [0, 1], "other faces start on their own");
        assert_eq!(d.finished(7, false, 4.0), None, "a face never started");
    }

    #[test]
    fn a_failed_face_waits_longer_after_each_failure_and_then_stops() {
        let mut d = Demand::default();
        let mut now = 0.0;
        // The wait after the 1st, 2nd, ... failure: doubling, up to 16 times the first.
        for wait in [1, 2, 4, 8].map(|k| k * RETRY_MS) {
            assert_eq!(d.start(&[0], now), [0], "at {now}");
            assert_eq!(d.finished(0, false, now), Some(wait), "the wait after the failure at {now}");
            assert!(d.start(&[0], now + f64::from(wait) - 1.0).is_empty(), "waits {wait} ms after the failure at {now}");
            now += f64::from(wait);
        }
        // The fifth fetch, and after it fails, none.
        assert_eq!(d.start(&[0], now), [0]);
        assert_eq!(d.finished(0, false, now), None, "no retry after {MAX_TRIES} failures");
        assert!(d.start(&[0], now + 1e12).is_empty(), "no fetch after {MAX_TRIES} failures");
        // A later success after failures keeps the face.
        let mut d = Demand::default();
        d.start(&[1], 0.0);
        assert_eq!(d.finished(1, false, 0.0), Some(RETRY_MS));
        assert_eq!(d.start(&[1], f64::from(RETRY_MS)), [1]);
        assert_eq!(d.finished(1, true, f64::from(RETRY_MS)), None);
        assert!(d.start(&[1], 1e12).is_empty());
    }

    /// While a face is fetched again after a failure, neither the interface asking for its script
    /// again nor the retry timer starts a second fetch of it.
    #[test]
    fn a_face_being_fetched_again_is_not_fetched_twice_at_once() {
        let mut d = Demand::default();
        d.start(&[0], 0.0);
        assert_eq!(d.finished(0, false, 0.0), Some(RETRY_MS));
        let retry = f64::from(RETRY_MS);
        assert_eq!(d.start(&[0, 0], retry), [0], "one fetch for a face wanted twice");
        assert!(d.start(&[0], retry + 1.0).is_empty(), "the interface asking again");
        assert!(d.start(&[0], retry + 1_000.0).is_empty(), "the retry timer");
        assert_eq!(d.finished(0, true, retry + 2_000.0), None);
        assert!(d.start(&[0], 1e12).is_empty(), "loaded");
    }

    /// A clock that reads earlier than the failure (one set back meanwhile) does not hold the
    /// retry back: the timer that asks for it has waited already.
    #[test]
    fn a_clock_set_back_after_a_failure_does_not_stop_the_retry() {
        let mut d = Demand::default();
        let failed_at = 1e12;
        d.start(&[0], failed_at);
        assert_eq!(d.finished(0, false, failed_at), Some(RETRY_MS));
        assert!(d.start(&[0], failed_at + 1.0).is_empty(), "still waiting");
        assert_eq!(d.start(&[0], failed_at - 3_600_000.0), [0], "the clock went back an hour");
    }
}
