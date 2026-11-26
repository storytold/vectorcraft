//! Windows: a monitor's raw density, the pixels per inch of the screen itself as Windows reads it
//! from the monitor, for Display Print Size at 100% Zoom (`crate::screen_density`).
//!
//! `GetDpiForMonitor` is a C API with no safe binding, hence the scoped `unsafe_code` allowance.
#![allow(unsafe_code)]

use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_RAW_DPI};

/// The horizontal and vertical pixels per inch of the monitor winit's handle `monitor` names:
/// `None` when Windows reports no density for it, an error when the call fails.
pub(crate) fn raw_dpi(monitor: isize) -> Result<Option<[f64; 2]>, String> {
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: `monitor` is a handle winit got from Windows for a connected monitor; a handle that
    // has gone stale makes the call fail, which returns an error. Both pointers point at live
    // `u32`s for the length of the call.
    unsafe { GetDpiForMonitor(HMONITOR(monitor as *mut core::ffi::c_void), MDT_RAW_DPI, &mut x, &mut y) }
        .map_err(|e| format!("couldn't read a monitor's density: {e}"))?;
    Ok((x > 0 && y > 0).then(|| [f64::from(x), f64::from(y)]))
}
