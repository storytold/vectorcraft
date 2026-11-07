#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn now() -> u64 {
    0
}
