use super::*;

/// A render context on the calling thread. vello_cpu's `RenderContext::new` defaults to a
/// multithreaded dispatcher, which panics on filter effects (glows, shadows, blur).
pub(crate) fn single_threaded_context(w: u16, h: u16) -> RenderContext {
    RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() })
}

/// Preferred rasterizer thread count set by the app (Preferences → Performance); negative = automatic.
static THREADS_OVERRIDE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Override the worker-thread count used by renderers created from now on (`None` = automatic).
pub fn set_default_threads(n: Option<u16>) {
    THREADS_OVERRIDE.store(n.map_or(-1, i32::from), std::sync::atomic::Ordering::Relaxed);
}

/// Rasterizer worker threads: 0 on wasm; otherwise one per core up to 8, leaving a core free. On a
/// 10-core machine 8 workers render a 50,000-path page 1.4× faster than 4 (9 are slower again);
/// the pixels don't depend on the count (`pixels_do_not_depend_on_the_thread_count`).
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
        std::thread::available_parallelism().map(|n| (n.get().saturating_sub(1)).min(8) as u16).unwrap_or(0)
    }
}
