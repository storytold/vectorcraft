//! The original notification tick, played by one host-owned worker through safe rodio APIs.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rodio::{OutputStream, Sink, buffer::SamplesBuffer};
use vectorcraft_ui_egui::toasts;

const MAX_QUEUED: usize = 32;
const STOP_WAIT: Duration = Duration::from_millis(100);
// Feedback is useful near presentation, even when the toast itself lives much longer.
const FEEDBACK_FRESHNESS: Duration = Duration::from_millis(100);

struct Request {
    expires_at: Instant,
}

impl Request {
    fn new(now: Instant, remaining: Duration) -> Option<Self> {
        let lifetime = remaining.min(FEEDBACK_FRESHNESS);
        if lifetime.is_zero() {
            return None;
        }
        Some(Self { expires_at: now.checked_add(lifetime)? })
    }

    fn is_fresh(&self, now: Instant) -> bool {
        now < self.expires_at
    }
}

fn queue_tick(sender: &SyncSender<Request>, ready: &AtomicBool, now: Instant, remaining: Duration) -> bool {
    if !ready.load(Ordering::Acquire) {
        return false;
    }
    let Some(request) = Request::new(now, remaining) else {
        return false;
    };
    // Device loss or a full playback queue does not block the UI or the visual toast.
    sender.try_send(request).is_ok()
}

// Clear readiness on every worker exit, including unwinding after successful initialization.
struct Readiness(Arc<AtomicBool>);

impl Drop for Readiness {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct Worker {
    sender: Option<SyncSender<Request>>,
    ready: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    finished: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn tick(&self, remaining: Duration) {
        if let Some(sender) = &self.sender {
            queue_tick(sender, &self.ready, Instant::now(), remaining);
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Drop occurs when the host's Services is torn down, not on each notification. Pending
        // cues are discarded and the current cue stops. Like the clipboard probe, wait only
        // briefly for the worker to release its handles: a platform driver can remain blocked.
        self.stopping.store(true, Ordering::Relaxed);
        self.ready.store(false, Ordering::Release);
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

pub(crate) fn start() -> Option<Box<dyn FnMut(Duration)>> {
    let (sender, receiver) = sync_channel::<Request>(MAX_QUEUED);
    let ready = Arc::new(AtomicBool::new(false));
    let device_ready = ready.clone();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let (finished_tx, finished) = channel::<()>();
    let thread = match std::thread::Builder::new().name("notification-audio".into()).spawn(move || {
        // Dropped after the stream and Sink on every return path, including an unavailable device.
        let _finished = finished_tx;
        let readiness = Readiness(device_ready);
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
        if stop.load(Ordering::Relaxed) {
            return;
        }
        readiness.0.store(true, Ordering::Release);
        while let Ok(request) = receiver.recv() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let cue = SamplesBuffer::new(1, toasts::TICK_SAMPLE_RATE, samples.clone());
            if !request.is_fresh(Instant::now()) {
                continue;
            }
            sink.append(cue);
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
    let worker = Worker { sender: Some(sender), ready, stopping, finished, thread: Some(thread) };
    Some(Box::new(move |remaining| worker.tick(remaining)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unready_requests_are_dropped_without_replay() {
        let (sender, receiver) = sync_channel(MAX_QUEUED);
        let ready = Arc::new(AtomicBool::new(false));
        let now = Instant::now();
        assert!(!queue_tick(&sender, &ready, now, Duration::from_secs(4)));
        assert!(receiver.try_recv().is_err());
        {
            let readiness = Readiness(ready.clone());
            readiness.0.store(true, Ordering::Release);
            assert!(receiver.try_recv().is_err(), "initialization cannot replay an unready request");
        }
        assert!(!ready.load(Ordering::Acquire), "worker exit clears readiness");
    }

    #[test]
    fn ready_fresh_requests_are_accepted() {
        let (sender, receiver) = sync_channel(MAX_QUEUED);
        let ready = AtomicBool::new(true);
        let now = Instant::now();
        assert!(queue_tick(&sender, &ready, now, Duration::from_secs(4)));
        let request = receiver.try_recv().unwrap();
        assert!(request.is_fresh(now));
        assert_eq!(request.expires_at.duration_since(now), FEEDBACK_FRESHNESS);
    }

    #[test]
    fn queued_expired_requests_do_not_hide_a_later_fresh_request() {
        let (sender, receiver) = sync_channel(MAX_QUEUED);
        let ready = AtomicBool::new(true);
        let now = Instant::now();
        assert!(queue_tick(&sender, &ready, now, Duration::from_secs(4)));
        let later = now + Duration::from_secs(1);
        assert!(queue_tick(&sender, &ready, later, Duration::from_secs(4)));
        assert!(!receiver.try_recv().unwrap().is_fresh(later), "a stalled queued cue is discarded");
        assert!(receiver.try_recv().unwrap().is_fresh(later), "the next fresh cue can still play");
    }

    #[test]
    fn remaining_toast_lifetime_limits_feedback_freshness() {
        let now = Instant::now();
        let lifetime = Duration::from_millis(5);
        let request = Request::new(now, lifetime).unwrap();
        assert_eq!(request.expires_at.duration_since(now), lifetime);
        assert!(request.is_fresh(now + Duration::from_millis(4)));
        assert!(!request.is_fresh(now + lifetime), "a cue cannot start at or after the toast's deadline");
    }

    #[test]
    fn zero_and_large_lifetimes_are_handled_safely() {
        let now = Instant::now();
        assert!(Request::new(now, Duration::ZERO).is_none());
        let request = Request::new(now, Duration::MAX).unwrap();
        assert_eq!(request.expires_at.duration_since(now), FEEDBACK_FRESHNESS);
        assert!(!request.is_fresh(now + FEEDBACK_FRESHNESS));
    }

    #[test]
    fn full_queue_drops_without_blocking() {
        let (sender, receiver) = sync_channel(MAX_QUEUED);
        let ready = AtomicBool::new(true);
        let now = Instant::now();
        for _ in 0..MAX_QUEUED {
            assert!(queue_tick(&sender, &ready, now, Duration::from_secs(4)));
        }
        assert!(!queue_tick(&sender, &ready, now, Duration::from_secs(4)), "try_send drops the full-queue cue");
        assert_eq!(receiver.try_iter().count(), MAX_QUEUED);
        let later = now + Duration::from_secs(1);
        assert!(queue_tick(&sender, &ready, later, Duration::from_secs(4)));
        assert!(receiver.try_recv().unwrap().is_fresh(later));
    }
}
