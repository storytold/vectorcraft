/// Seconds since the process started, from a monotonic clock (`web_time::Instant`, which maps to
/// `performance.now()` in the browser and `std::time::Instant` natively). Background work, poll
/// pacing and deadlines use this, not egui's `InputState::time`: a logic-only pass replays the
/// last shown frame's time, and a wall clock jumps with the system's.
pub fn now_secs() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::LazyLock;
        use std::time::Instant;
        static START: LazyLock<Instant> = LazyLock::new(Instant::now);
        START.elapsed().as_secs_f64()
    }
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::LazyLock;
        use web_time::Instant;
        static START: LazyLock<Instant> = LazyLock::new(Instant::now);
        START.elapsed().as_secs_f64()
    }
}

/// [`now_secs`] in milliseconds, for frame timing and capture deadlines.
pub fn now_ms() -> f64 {
    now_secs() * 1000.0
}
