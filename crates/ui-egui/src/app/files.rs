use super::{VectorcraftApp, io, place};
use crate::services::Inbox;

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

    pub(crate) fn drain_inbox(&mut self) {
        fn take(q: Option<&Inbox>) -> Vec<(String, Vec<u8>)> {
            q.map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default()
        }
        let answers = self.services.picked_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner())));
        // The web's file picker answers for automation when automation opened it; the page's script
        // is automation.
        let mut arrived: Vec<_> = take(self.services.inbox.as_ref()).into_iter().map(|f| (f, false)).collect();
        for answer in answers.unwrap_or_default() {
            let automated = self.picks.answer_web_open();
            arrived.extend(answer.map(|f| (f, automated)));
        }
        arrived.extend(take(self.services.page_inbox.as_ref()).into_iter().map(|f| (f, true)));
        for ((name, bytes), automated) in arrived {
            crate::picks::as_automated(self, automated, |app| {
                if let Err(e) = io::open_bytes(app, &name, &bytes, None) {
                    io::report_open_error(app, &name, &e);
                }
            });
        }
        place::drain(self);
    }
}
