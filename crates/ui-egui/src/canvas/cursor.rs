use craft::behavior::CanvasView;
use vectorcraft_geom::Point;

use crate::CacheKey;

/// How long (seconds) the view must stay still before a canvas with
/// [`crate::CanvasCache::waits_for_still`] set renders a heavy document again.
pub const STILL_SECS: f64 = 0.2;

/// How long (seconds) the view must stay still before the canvas hit-tests the art under the
/// pointer again, for Object Highlighting and the tool's cursor. While the view pans, zooms or
/// turns under a resting pointer, the document point under it changes every frame, and hit-testing
/// detailed art each time slowed every frame of the move (#2452).
pub const HOVER_STILL_SECS: f64 = 0.15;

/// The view [`hover_wait`] watches: the canvas's rectangle and the view's zoom, centre and rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewKey {
    pub(super) rect: egui::Rect,
    pub(super) zoom: f64,
    pub(super) center: Point,
    pub(super) rotation: f64,
}

impl ViewKey {
    /// The key of the view `view` shows.
    pub fn of(view: &CanvasView) -> Self {
        Self { rect: view.rect, zoom: view.zoom, center: view.center, rotation: view.rotation }
    }
}

/// Note the view `key` at time `now` (seconds; the pass time, `frame::clock`, which every pass of a
/// frame shares) → how many more seconds the canvas leaves the art under the pointer alone: the
/// view changed less than [`HOVER_STILL_SECS`] ago. The first view noted counts as still.
pub(super) fn hover_wait(moved: &mut Option<(ViewKey, f64)>, key: ViewKey, now: f64) -> Option<f64> {
    match moved {
        Some((k, at)) if *k != key => (*k, *at) = (key, now),
        Some(_) => {}
        None => *moved = Some((key, f64::NEG_INFINITY)),
    }
    let waited = moved.map_or(f64::INFINITY, |(_, at)| now - at);
    // The clock went back (a new egui context): don't wait for it to catch up.
    (0.0..HOVER_STILL_SECS).contains(&waited).then_some(HOVER_STILL_SECS - waited)
}

/// Note that the canvas asks for `key` at time `now` (seconds; the pass time, `frame::clock`) → how
/// many more seconds it draws the frame `shown` reprojected instead of rendering `key`: while the last
/// render was `slow`, only the view differs between the two, and the canvas has asked for the view
/// of `key` for less than [`STILL_SECS`]. `still` holds the key last asked for and when the canvas first
/// asked for its view. An edit leaves that time as it is: with the view still and edits drawn into
/// the frame shown at each step of a drag, the canvas still renders whole once the view has been
/// still for [`STILL_SECS`]. None: the canvas renders `key` now, unless it is the frame shown.
pub(super) fn still_wait(still: &mut Option<(CacheKey, f64)>, shown: Option<&CacheKey>, key: &CacheKey, slow: bool, now: f64) -> Option<f64> {
    if still.as_ref().is_none_or(|(k, _)| !same_view(k, key)) {
        *still = Some((key.clone(), now));
    }
    let waited = still.as_ref().map_or(0.0, |(_, at)| now - at);
    // The clock went back (a new egui context): don't wait for it to catch up.
    let waiting = (0.0..STILL_SECS).contains(&waited);
    (slow && waiting && shown.is_some_and(|s| s != key && only_view_differs(s, key))).then_some(STILL_SECS - waited)
}

/// Whether two cache keys are of the same document at the same view (zoom, center, rotation,
/// canvas size, pixel density and Pixel Preview's document pixels), whatever else differs.
fn same_view(a: &CacheKey, b: &CacheKey) -> bool {
    (a.doc, a.zoom, a.cx, a.cy, a.w, a.h, a.ppp, a.rot, a.pixel) == (b.doc, b.zoom, b.cx, b.cy, b.w, b.h, b.ppp, b.rot, b.pixel)
}

/// Whether two cache keys differ only in the view (zoom, center, rotation, canvas size, pixel
/// density and Pixel Preview's document pixels), with Pixel Preview on in both or off in both.
pub(super) fn only_view_differs(a: &CacheKey, b: &CacheKey) -> bool {
    a.pixel.is_some() == b.pixel.is_some()
        && *a == CacheKey { zoom: a.zoom, cx: a.cx, cy: a.cy, w: a.w, h: a.h, ppp: a.ppp, rot: a.rot, pixel: a.pixel, ..b.clone() }
}
