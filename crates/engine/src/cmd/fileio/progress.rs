//! Progress and Cancel of an export running off the UI thread (Background Export), on the shared
//! [`vectorcraft_doc::cancel`] mechanism.
//!
//! The frontend makes a [`Progress`], runs the export inside [`scope`] on its worker and reads the
//! counts from the UI thread to draw them; its Cancel button calls [`Progress::cancel`]. Inside the
//! scope the encoders that work in steps (raster formats: one artboard at a time) report each step
//! through [`vectorcraft_doc::cancel::progress`] and stop at the next check once cancelled; the
//! renderer stops drawing and keeps no half-drawn pattern tile or effect raster. Encoders that
//! make one file in one go report nothing: the frontend shows them as busy and checks
//! [`Progress::cancelled`] itself before writing, so a cancelled export never writes a file.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vectorcraft_doc::cancel::{self, CancelToken, ProgressSink};

/// How far an export has got, shared between the worker and the UI thread.
#[derive(Debug, Default)]
pub struct Progress {
    token: CancelToken,
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// `(steps done, steps in all)`; `total` is 0 while the export can't say (one file made in one go).
    pub fn counts(&self) -> (u64, u64) {
        let total = self.total.load(Ordering::Relaxed);
        (self.done.load(Ordering::Relaxed).min(total), total)
    }

    /// The share of the export done, 0 to 1; `None` while the export can't say.
    pub fn fraction(&self) -> Option<f64> {
        let (done, total) = self.counts();
        (total > 0).then(|| done as f64 / total as f64)
    }

    /// Ask the export to stop: it writes nothing more, and a render under way stops drawing.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// Run `f` with `progress`'s token in effect on this thread and the steps reported to it.
pub fn scope<R>(progress: &Arc<Progress>, f: impl FnOnce() -> R) -> R {
    let to = Arc::clone(progress);
    let sink: ProgressSink = Rc::new(move |done, total, _| {
        to.total.store(total, Ordering::Relaxed);
        to.done.store(done, Ordering::Relaxed);
    });
    cancel::scope(&progress.token, Some(sink), f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raster_export_counts_its_artboards_and_stops_once_cancelled() {
        let mut doc = vectorcraft_doc::Document::new(100.0, 100.0);
        let second = doc.artboards[0].clone();
        doc.artboards.push(second);
        let p = Progress::new();
        assert_eq!(p.fraction(), None, "nothing reported yet");
        let png = scope(&p, || super::super::encode_all(&doc, "png", &serde_json::json!({"useArtboards": true})));
        assert!(png.is_ok(), "{:?}", png.err());
        assert_eq!(p.counts(), (1, 2), "the step under way when the last artboard started");
        p.cancel();
        let r = scope(&p, || super::super::encode_all(&doc, "png", &serde_json::json!({"useArtboards": true})));
        assert!(matches!(r, Err(crate::EngineError::Cancelled)), "{:?}", r.err());
        assert!(!cancel::is_cancelled(), "the scope ended");
    }
}
