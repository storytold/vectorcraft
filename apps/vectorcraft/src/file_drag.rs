//! Current Windows file-drag coordinates. winit does not send CursorMoved during an OLE drag;
//! using egui's last pointer position would place the file where the pointer was before the drag.

use std::sync::Arc;

/// Client coordinates in physical pixels, using safe Win32 wrappers already used by the app.
pub fn service(window: Option<&Arc<winit::window::Window>>) -> Option<vectorcraft_ui_egui::NativeFileDragPosition> {
    let window = window?.clone();
    Some(Arc::new(move || {
        let cursor = winsafe::GetCursorPos().ok()?;
        let origin = window.inner_position().ok()?;
        Some([f64::from(cursor.x) - f64::from(origin.x), f64::from(cursor.y) - f64::from(origin.y)])
    }))
}
