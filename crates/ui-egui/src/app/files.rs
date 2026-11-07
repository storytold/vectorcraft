use super::{VectorcraftApp, io, place};

impl VectorcraftApp {
    /// A file dialog is open off the UI thread (a sheet on macOS, [`picks`]): what asked for it runs
    /// again when it answers.
    pub fn file_dialog_open(&self) -> bool {
        self.picks.is_waiting()
    }

    /// A file dialog is open as a sheet on the window ([`Services::dialogs_are_sheets`]): nothing
    /// closes or quits until it answers.
    pub fn file_sheet_open(&self) -> bool {
        self.services.dialogs_are_sheets && self.file_dialog_open()
    }

    pub(super) fn drain_inbox(&mut self) {
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = io::open_bytes(self, &name, &bytes, None) {
                io::report_open_error(self, &name, &e);
            }
        }
        place::drain(self);
    }
}
