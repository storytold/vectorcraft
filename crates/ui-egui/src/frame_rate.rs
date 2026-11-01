//! The status bar's frame rate: frames per second while the window draws continuously.
//!
//! An idle window draws only when something wakes it, and each wake-up draws a couple of frames a
//! few milliseconds apart (the swapchain has a free image, so the second doesn't wait for vsync).
//! An average of `1 / dt` over any two frames therefore climbed above the display's refresh rate at
//! rest and fell back to it as soon as the pointer moved, which looked like a slowdown. Here the
//! gaps the window spent idle start a new measurement, and the rate is the number of frames over
//! the last second of continuous drawing.

use std::collections::VecDeque;

/// The stretch of continuous drawing the rate is taken over, in seconds.
const WINDOW: f64 = 1.0;
/// Continuous drawing needed before the readout changes, in seconds: shorter bursts (a wake-up's
/// frames) keep the last rate.
const MIN_SPAN: f64 = 0.5;
/// Time between frames, beyond the previous frame's own work, that means the window was idle, in
/// seconds. Waiting for vsync takes at most one refresh (33 ms at 30 Hz), so a longer wait is the
/// window waiting for input, not for the display.
const IDLE: f64 = 0.1;

#[derive(Debug, Default)]
pub struct FrameRate {
    /// Seconds between consecutive frames of the current stretch of drawing, oldest first.
    intervals: VecDeque<f64>,
    /// Their sum.
    span: f64,
    /// When the last frame started.
    last: Option<f64>,
    /// The readout: frames per second, 0 until the window has drawn continuously for [`MIN_SPAN`].
    fps: f64,
}

impl FrameRate {
    /// A frame starts at `now` (seconds, monotonic); the previous frame's own work took `work`
    /// seconds. → the frame rate to show.
    pub fn frame(&mut self, now: f64, work: f64) -> f64 {
        let Some(last) = self.last.replace(now) else { return self.fps };
        let dt = now - last;
        if !dt.is_finite() || dt <= 0.0 {
            // A second pass of the same frame, or a clock that went back: nothing to measure.
            return self.fps;
        }
        if dt - work.max(0.0) > IDLE {
            self.intervals.clear();
            self.span = 0.0;
            return self.fps;
        }
        self.intervals.push_back(dt);
        self.span += dt;
        while self.span > WINDOW
            && let Some(old) = self.intervals.pop_front()
        {
            self.span -= old;
        }
        if self.span >= MIN_SPAN {
            self.fps = self.intervals.len() as f64 / self.span;
        }
        self.fps
    }
}

#[cfg(test)]
mod tests {
    use super::FrameRate;

    /// Feed frames starting at `t` every `dt` seconds for `secs`, each taking `work`. → the time
    /// after the last frame and the readout.
    fn run(r: &mut FrameRate, mut t: f64, dt: f64, secs: f64, work: f64) -> (f64, f64) {
        let mut fps = 0.0;
        let end = t + secs;
        while t < end {
            fps = r.frame(t, work);
            t += dt;
        }
        (t, fps)
    }

    #[test]
    fn idle_wake_ups_keep_the_rate_the_display_allowed() {
        let mut r = FrameRate::default();
        // The pointer moves: the window draws at the 30 Hz display's rate.
        let (mut t, fps) = run(&mut r, 0.0, 1.0 / 30.0, 3.0, 0.001);
        assert!((fps - 30.0).abs() < 0.5, "{fps}");
        // It rests: a wake-up every half second draws two frames 4 ms apart.
        for _ in 0..20 {
            t += 0.5;
            r.frame(t, 0.001);
            let fps = r.frame(t + 0.004, 0.001);
            assert!((fps - 30.0).abs() < 0.5, "an idle window read {fps} fps");
        }
        // It moves again on a 60 Hz display: the readout follows once it has drawn for a while.
        let (_, fps) = run(&mut r, t + 1.0, 1.0 / 60.0, 1.5, 0.001);
        assert!((fps - 60.0).abs() < 0.5, "{fps}");
    }

    #[test]
    fn slow_frames_read_slow() {
        // Each frame works 200 ms: the long gaps are its own work, not idling.
        let mut r = FrameRate::default();
        let (_, fps) = run(&mut r, 0.0, 0.21, 3.0, 0.2);
        assert!((fps - 1.0 / 0.21).abs() < 0.05, "{fps}");
    }

    #[test]
    fn no_reading_before_half_a_second_of_drawing() {
        let mut r = FrameRate::default();
        let (_, fps) = run(&mut r, 0.0, 1.0 / 60.0, 0.4, 0.001);
        assert_eq!(fps, 0.0);
    }
}
