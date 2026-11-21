use super::{VectorcraftApp, io, place};
use crate::services::Inbox;
use craft::host::{Host, PendingPick, PickKind};

impl VectorcraftApp {
    /// The oldest pick the file service has not answered, the one on screen.
    fn pending_pick(&self) -> Option<PendingPick> {
        Host::try_of_ctx(self.ui_ctx.as_ref()?)?.files().pending_pick()
    }

    /// A file dialog is open off the UI thread: what asked for it runs when it answers.
    pub fn file_dialog_open(&self) -> bool {
        self.pending_pick().is_some()
    }

    /// A file dialog is open as a sheet on the window: nothing closes or quits until it answers.
    pub fn file_sheet_open(&self) -> bool {
        self.pending_pick().is_some_and(|p| p.sheet)
    }

    /// The kind of dialog open (`ui.inspect`'s `fileDialog`).
    pub(crate) fn file_dialog_kind(&self) -> Option<&'static str> {
        let PendingPick { request, .. } = self.pending_pick()?;
        Some(match request.kind {
            _ if request.purpose == "place" => "place",
            PickKind::Open | PickKind::OpenMany => "open",
            PickKind::Save => "save",
            PickKind::Folder => "folder",
        })
    }

    pub(crate) fn drain_inbox(&mut self) {
        fn take(q: Option<&Inbox>) -> Vec<(String, Vec<u8>)> {
            q.map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default()
        }
        // The page's script is automation.
        let mut arrived: Vec<_> = take(self.services.inbox.as_ref()).into_iter().map(|f| (f, false)).collect();
        arrived.extend(take(self.services.page_inbox.as_ref()).into_iter().map(|f| (f, true)));
        for ((name, bytes), automated) in arrived {
            crate::file_asks::as_automated(self, automated, |app| {
                if let Err(e) = io::open_bytes(app, &name, &bytes, None) {
                    io::report_open_error(app, &name, &e);
                }
            });
        }
        place::drain(self);
    }
}
