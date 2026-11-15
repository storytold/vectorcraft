//! Wheel streams for the fields that step on the wheel (number fields and the closed font family
//! field). A field takes a wheel stream only when the stream begins with the pointer over it and
//! the pointer was moved onto it after the field last moved or was covered by a window or overlay.

/// The seconds without wheel input that end a wheel stream. This is VectorCraft's own value.
pub(crate) const WHEEL_STREAM_GAP: f64 = 0.5;

/// Where the closed font family field, which takes its wheel events from the raw input, keeps the
/// input time of the last one it took in `viewport`, so number fields see that stream too.
pub(crate) fn taken_from_raw_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("wheel-taken-from-raw").with(viewport)
}

/// The field the pointer was last moved onto, while that field stays where it was and uncovered.
#[derive(Clone, Copy)]
pub(crate) struct ArmedWheelField {
    pub(crate) id: egui::Id,
    pub(crate) rect: egui::Rect,
    pub(crate) pass: u64,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WheelStreamOwner {
    Field(egui::Id),
    Elsewhere,
}

/// One run of wheel input with no gap of [`WHEEL_STREAM_GAP`] in it, trackpad momentum included.
#[derive(Clone, Copy, Default)]
pub(crate) struct WheelStream {
    last: Option<f64>,
    pub(crate) owner: Option<WheelStreamOwner>,
}

impl WheelStream {
    /// Note wheel input at `now`; a gap since the last input starts a new stream with no owner.
    pub(crate) fn note_input(&mut self, now: f64) {
        if !self.last.is_some_and(|last| (0.0..WHEEL_STREAM_GAP).contains(&(now - last))) {
            self.owner = None;
        }
        self.last = Some(now);
    }

    /// Note wheel input at `at` that another kind of field took: the stream it belongs to is not
    /// one of this kind's fields.
    pub(crate) fn note_taken_elsewhere(&mut self, at: f64) {
        if self.last.is_none_or(|last| last <= at) {
            self.note_input(at);
        }
        self.owner = Some(WheelStreamOwner::Elsewhere);
    }

    /// Leave the rest of the stream to the panel when `field` holds it.
    pub(crate) fn release(&mut self, field: egui::Id) {
        if self.owner == Some(WheelStreamOwner::Field(field)) {
            self.owner = Some(WheelStreamOwner::Elsewhere);
        }
    }

    /// Leave the rest of the stream to the panel when any field holds it.
    pub(crate) fn release_any(&mut self) {
        if let Some(WheelStreamOwner::Field(field)) = self.owner {
            self.release(field);
        }
    }
}
