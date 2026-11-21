//! Where a bar the user moved sits, held with the document and the kind of place the move was made in.

use craft::ui::canvas_controls::TaskBarPlace;

/// A [`TaskBarPlace`] with the key of the place it was made in: the document and, for the Quick Pen bar,
/// whether a path is being drawn. A drag's offset holds only there, so one made beside the drawing does not
/// move the resting bar. The framework's place is built through `Default` only (it is `non_exhaustive`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyedPlace {
    /// The place the bar was last left at.
    pub place: TaskBarPlace,
    /// The key `place` was made under; none before the first move.
    pub key: Option<(u64, bool)>,
}

impl KeyedPlace {
    /// The place to show under `key`: a pinned place holds in every key, an unpinned one only in the key it was made in.
    pub fn for_key(&self, key: (u64, bool)) -> TaskBarPlace {
        if self.place.pinned || self.key == Some(key) { self.place } else { TaskBarPlace::default() }
    }

    /// Keeps `place`, as made under `key`.
    pub fn set(&mut self, key: (u64, bool), place: TaskBarPlace) {
        self.place = place;
        self.key = Some(key);
    }
}
