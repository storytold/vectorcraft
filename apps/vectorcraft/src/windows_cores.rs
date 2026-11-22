//! Windows: the cores the process may run on, from its affinity mask, for the renderer's worker
//! count (`vectorcraft_render::set_usable_cores`). `available_parallelism` counts every processor
//! in the machine, also when the process may run on fewer (started with `start /affinity`, or
//! pinned by a launcher). The desktop app and `vectorcraft-cli` (exports, MCP) both install it; the
//! CLI shares this file.
//!
//! `GetProcessAffinityMask` is a C API with no safe binding, hence the scoped `unsafe_code`
//! allowance.
#![allow(unsafe_code)]

use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};

/// Tells the renderer how many cores the process may run on.
pub fn install() {
    vectorcraft_render::set_usable_cores(usable_cores());
}

/// The number of cores in the process's affinity mask, or `None` when Windows reports none (a
/// process with threads in more than one processor group).
fn usable_cores() -> Option<usize> {
    let (mut process, mut system) = (0usize, 0usize);
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no closing, and both pointers
    // point at live `usize`s for the length of the call.
    unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) }.ok()?;
    (process != 0).then(|| process.count_ones() as usize)
}

#[cfg(test)]
mod tests {
    /// The mask Windows reports for this process gives at least one core and no more than the
    /// machine has. (A child process started with `start /affinity` would test a one-core mask
    /// end to end, but a process spawned while other tests hold sockets inherits them and resets
    /// their connections; `automatic_threads` covers the one-core count.)
    #[test]
    fn the_affinity_mask_gives_the_cores_the_process_may_use() {
        let cores = super::usable_cores().unwrap();
        let all = std::thread::available_parallelism().unwrap().get();
        assert!((1..=all).contains(&cores), "{cores} of {all}");
    }
}
