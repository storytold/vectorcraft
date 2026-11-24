//! The continuation of ordinary File > Save when its own location refuses writes (#6146).
//! It reuses Save As, including extension and overwrite checks, under its own picker entry.

use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::SaveMode;
#[cfg(not(target_arch = "wasm32"))]
use vectorcraft_engine::cmd::fileio::{self, SavePlan};

use crate::VectorcraftApp;

pub(super) struct Retry {
    path: Option<String>,
    name: String,
    directory: Option<PathBuf>,
    params: Value,
}

impl Retry {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn new(plan: &SavePlan, folder: Option<String>) -> Self {
        let name = plan.path.as_deref().map(fileio::file_name).unwrap_or_else(|| plan.name.clone());
        let mut params = json!({ "format": plan.format.id, "options": plan.options });
        if let Some(modified) = plan.modified {
            params["modified"] = json!(modified);
        }
        Self { path: plan.path.clone(), name, directory: folder.map(PathBuf::from), params }
    }

    /// None when this continuation is stale or another picker prevents it. The existing failed-save
    /// dialog then stays in charge; there is no queue of deferred Save As requests. Otherwise the
    /// Save As picker opens (`{pending: "fileDialog"}`); the document takes the new path only once
    /// the write succeeded.
    pub(super) fn instead(self, app: &mut VectorcraftApp, uid: u64) -> Option<Result<Value, String>> {
        if app.automation_asked() || app.file_dialog_open() || super::save::is_web(app) {
            return None;
        }
        let index = app.session.documents().iter().position(|st| st.uid == uid && st.path == self.path)?;
        if app.session.active_index() != Some(index) && app.run("document.activate", json!({ "index": index })).is_err() {
            return None;
        }
        let name = &self.name;
        app.status(format!("Cannot write to the location of “{name}”; choose where to save it"));
        // The failed save is the flow's running write: it ends before the picker opens.
        #[cfg(not(target_arch = "wasm32"))]
        app.saves.release(uid);
        Some(super::save::save_suggesting(app, SaveMode::SaveAs, &self.params, false, Some((&self.name, self.directory))))
    }
}
