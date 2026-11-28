//! Channel control for the next Shape Builder merge started on the calling thread.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvError, Sender, channel};

thread_local! {
    static NEXT: RefCell<Option<WorkerGate>> = const { RefCell::new(None) };
}

/// Holds a merge worker until released. Dropping the gate also releases the worker.
pub struct MergeGate {
    id: Arc<()>,
    started: Receiver<()>,
    release: Option<Sender<()>>,
    _thread: PhantomData<Rc<()>>,
}

impl MergeGate {
    /// Waits until the worker reaches the gate.
    pub fn wait_started(&self) -> Result<(), RecvError> {
        self.started.recv()
    }

    /// Allows the worker to compute the merge.
    pub fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for MergeGate {
    fn drop(&mut self) {
        NEXT.with_borrow_mut(|next| {
            if next.as_ref().is_some_and(|gate| Arc::ptr_eq(&gate.id, &self.id)) {
                *next = None;
            }
        });
        self.release();
    }
}

pub(super) struct WorkerGate {
    id: Arc<()>,
    started: Sender<()>,
    release: Receiver<()>,
}

impl WorkerGate {
    pub(super) fn wait(self) {
        let _ = self.started.send(());
        let _ = self.release.recv();
    }
}

pub(super) fn take() -> Option<WorkerGate> {
    NEXT.with_borrow_mut(Option::take)
}

/// Pauses the next merge started on this thread. Returns an error if a gate is already waiting.
pub fn pause_next_merge() -> Result<MergeGate, &'static str> {
    NEXT.with_borrow_mut(|next| {
        if next.is_some() {
            return Err("a Shape Builder merge gate is already waiting on this thread");
        }
        let id = Arc::new(());
        let (started_tx, started) = channel();
        let (release, release_rx) = channel();
        *next = Some(WorkerGate { id: id.clone(), started: started_tx, release: release_rx });
        Ok(MergeGate { id, started, release: Some(release), _thread: PhantomData })
    })
}
