use super::{RenderContext, vello_cpu};

/// A render context on the calling thread. vello_cpu's `RenderContext::new` defaults to a
/// multithreaded dispatcher, which panics on filter effects (glows, shadows, blur).
pub(crate) fn single_threaded_context(w: u16, h: u16) -> RenderContext {
    RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() })
}

/// Preferred rasterizer thread count set by the app (Preferences → Performance); negative = automatic.
static THREADS_OVERRIDE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// The cores the process may run on, as the app read them from the system; 0 = not known.
static USABLE_CORES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Override the worker-thread count used by renderers created from now on (`None` = automatic).
pub fn set_default_threads(n: Option<u16>) {
    THREADS_OVERRIDE.store(n.map_or(-1, i32::from), std::sync::atomic::Ordering::Relaxed);
}

/// The number of cores the process may run on, when the app knows a smaller number than
/// `available_parallelism` gives (`None` = not known). On Windows `available_parallelism` counts
/// every processor in the machine, also when the process's affinity mask allows fewer.
pub fn set_usable_cores(n: Option<usize>) {
    USABLE_CORES.store(n.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
}

/// Rasterizer worker threads: 0 on wasm; otherwise one per core the process may run on, up to 8,
/// leaving a core free. On a 10-core machine 8 workers render a 50,000-path page 1.4× faster than 4
/// (9 are slower again); the pixels don't depend on the count
/// (`pixels_do_not_depend_on_the_thread_count`).
pub fn default_threads() -> u16 {
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let o = THREADS_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
        if o >= 0 {
            return o.min(64) as u16;
        }
        if std::env::var_os("VECTORCRAFT_RENDER_THREADS").is_some() {
            return std::env::var("VECTORCRAFT_RENDER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        let usable = USABLE_CORES.load(std::sync::atomic::Ordering::Relaxed);
        std::thread::available_parallelism().map(|n| automatic_threads(n.get(), usable)).unwrap_or(0)
    }
}

/// [`default_threads`] when automatic, on `cpus` processors of which the process may use `usable`
/// (0 = all).
fn automatic_threads(cpus: usize, usable: usize) -> u16 {
    let cores = if usable == 0 { cpus } else { cpus.min(usable) };
    (cores.saturating_sub(1)).min(8) as u16
}

#[cfg(test)]
mod tests {
    use super::automatic_threads;

    /// A process that may run on one core renders on the calling thread. With workers, vello_cpu's
    /// flush spins on that core while the workers it waits for get no time, and a render crawls or
    /// stalls.
    #[test]
    fn workers_follow_the_cores_the_process_may_use() {
        assert_eq!(automatic_threads(24, 0), 8);
        assert_eq!(automatic_threads(24, 1), 0);
        assert_eq!(automatic_threads(24, 4), 3);
        assert_eq!(automatic_threads(4, 16), 3);
        assert_eq!(automatic_threads(1, 0), 0);
    }
}
