//! The original notification tick, played by one host-owned worker through safe rodio APIs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
use vectorcraft_ui_egui::toasts;

const MAX_QUEUED: usize = 32;
const STOP_WAIT: Duration = Duration::from_millis(100);

struct Worker {
    sender: Option<SyncSender<()>>,
    stopping: Arc<AtomicBool>,
    finished: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn tick(&self) {
        if let Some(sender) = &self.sender {
            // Device loss or a full playback queue does not block the UI or the visual toast.
            let _ = sender.try_send(());
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Drop occurs when the host's Services is torn down, not on each notification. Pending
        // cues are discarded and the current cue stops. Like the clipboard probe, wait only
        // briefly for the worker to release its handles: a platform driver can remain blocked.
        self.stopping.store(true, Ordering::Relaxed);
        drop(self.sender.take());
        let _ = self.finished.recv_timeout(STOP_WAIT);
        if let Some(thread) = self.thread.take() {
            if thread.is_finished() {
                if thread.join().is_err() {
                    log::warn!("notification audio worker ended unexpectedly");
                }
            } else {
                // Detaching transfers no borrowed data: the worker owns its stream and stop flag.
                log::debug!("notification audio is still stopping after the bounded wait");
            }
        }
    }
}

pub(crate) fn start() -> Option<Box<dyn FnMut()>> {
    let (sender, receiver) = sync_channel(MAX_QUEUED);
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let (finished_tx, finished) = channel::<()>();
    let thread = match std::thread::Builder::new().name("notification-audio".into()).spawn(move || {
        // Dropped after the stream and Sink on every return path, including an unavailable device.
        let _finished = finished_tx;
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let (_stream, handle) = match OutputStream::try_default() {
            Ok(stream) => stream,
            Err(error) => {
                log::warn!("notification sound is unavailable: {error}");
                return;
            }
        };
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let sink = match Sink::try_new(&handle) {
            Ok(sink) => sink,
            Err(error) => {
                log::warn!("notification sound is unavailable: {error}");
                return;
            }
        };
        let samples = toasts::tick_samples();
        while receiver.recv().is_ok() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            sink.append(SamplesBuffer::new(1, toasts::TICK_SAMPLE_RATE, samples.clone()));
            // Only this worker waits, never the UI. One active cue plus the bounded channel
            // prevents an unbounded Sink queue. Shutdown can stop a stalled device's cue too.
            while !sink.empty() {
                if stop.load(Ordering::Relaxed) {
                    sink.stop();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }) {
        Ok(thread) => thread,
        Err(error) => {
            log::warn!("could not start notification audio: {error}");
            return None;
        }
    };
    let worker = Worker { sender: Some(sender), stopping, finished, thread: Some(thread) };
    Some(Box::new(move || worker.tick()))
}
