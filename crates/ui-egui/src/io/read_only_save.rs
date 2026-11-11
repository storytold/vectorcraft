//! The continuation of ordinary File > Save when its own location refuses writes (#6146).
//! It reuses Save As, including extension and overwrite checks, under its own picker entry.

use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{self, SaveMode, SavePlan};

use crate::picks::{self, Entry};
use crate::{FilePick, VectorcraftApp};

pub(super) struct Retry {
    path: Option<String>,
    pick: FilePick,
    params: Value,
}

impl Retry {
    pub(super) fn new(plan: &SavePlan, folder: Option<String>) -> Self {
        let name = plan.path.as_deref().map(fileio::file_name).unwrap_or_else(|| plan.name.clone());
        let mut params = json!({ "format": plan.format.id, "options": plan.options });
        if let Some(modified) = plan.modified {
            params["modified"] = json!(modified);
        }
        Self { path: plan.path.clone(), pick: FilePick { name, folder, filters: vec![] }, params }
    }

    /// None when this continuation is stale or another picker prevents it. The existing failed-save
    /// dialog then stays in charge; there is no queue of deferred Save As requests.
    pub(super) fn instead(self, app: &mut VectorcraftApp, uid: u64) -> Option<Result<Value, String>> {
        if app.automation_is_acting() || app.services.pick_save.is_none() || app.picks.is_waiting() || super::save::is_web(app) {
            return None;
        }
        let index = app.session.documents().iter().position(|st| st.uid == uid && st.path == self.path)?;
        if app.session.active_index() != Some(index) && app.run("document.activate", json!({ "index": index })).is_err() {
            return None;
        }
        let name = &self.pick.name;
        let status = format!("Cannot write to the location of “{name}”; choose where to save it");
        app.status(status.clone());
        let save = Rc::new(move |app: &mut VectorcraftApp| {
            // An agent can retarget this document while the sheet is open. Its answer must not
            // replace that later choice. The picker itself already checks the document's uid.
            if !app.session.active().is_some_and(|st| st.uid == uid && st.path == self.path) {
                return Err("cancelled".into());
            }
            super::save::save_suggesting(app, SaveMode::SaveAs, &self.params, false, Some(&self.pick))
        });
        let again = save.clone();
        let entry = Entry::Call(Rc::new(move |app| {
            if let Err(e) = again(app) {
                app.status(e);
            }
        }));
        let r = picks::as_own_entry(app, entry, |app| save(app));
        Some(match r {
            Err(e) if e == "cancelled" && app.picks.is_waiting() => {
                app.status(status);
                Ok(json!({ "pending": "fileDialog" }))
            }
            r => r,
        })
    }
}
