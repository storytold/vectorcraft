//! The DEV tag: a development build (one run from a cargo target directory, as `cargo run`
//! makes) marks itself in the app bar, so it can't be mistaken for an installed VectorCraft.
//! Hovering the tag says what is running. The host fills in [`DevBuild`]
//! (`VectorcraftApp::dev_build`); installed apps and the web app leave it `None`.
//! The tag is for developers, so it isn't translated.

use egui::{CornerRadius, Sense, Ui, vec2};

/// What a development build knows about itself, for the tag's tooltip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DevBuild {
    /// The checkout it was built from.
    pub checkout: String,
    /// The checkout's git branch (`None` when detached or unknown).
    pub branch: Option<String>,
    /// The checked-out commit, abbreviated.
    pub commit: Option<String>,
    /// The checkout has uncommitted changes.
    pub changes: bool,
    /// How old the binary was when the app started, in seconds (`None` when unknown).
    pub age_at_start: Option<f64>,
}

impl DevBuild {
    /// The tooltip: what is running, `now` seconds after the app started.
    pub fn describe(&self, now: f64) -> String {
        let mut lines = vec!["Development build of VectorCraft".to_string()];
        let mut git = vec![];
        if let Some(b) = &self.branch {
            git.push(format!("Branch {b}"));
        }
        if let Some(c) = &self.commit {
            git.push(format!("commit {c}"));
        }
        if self.changes {
            git.push("with uncommitted changes".to_string());
        }
        if !git.is_empty() {
            lines.push(git.join(" · "));
        }
        if let Some(age) = self.age_at_start {
            lines.push(format!("Built {}", age_label(age + now)));
        }
        lines.push(format!("From {}", self.checkout));
        lines.join("\n")
    }
}

/// "just now", "5 minutes ago", "3 hours ago", "2 days ago".
fn age_label(secs: f64) -> String {
    if !secs.is_finite() || secs < 60.0 {
        return "just now".into();
    }
    let minutes = (secs / 60.0).floor() as u64;
    let (n, unit) = match minutes {
        0..60 => (minutes, "minute"),
        60..1440 => (minutes / 60, "hour"),
        _ => (minutes / 1440, "day"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Paint the DEV tag (after the brand mark) when this is a development build.
pub fn tag(build: Option<&DevBuild>, ui: &mut Ui) {
    let Some(build) = build else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(10.5);
    let text = ui.painter().layout_no_wrap("DEV".into(), font, t.app_bar);
    let (r, resp) = ui.allocate_exact_size(vec2(text.size().x + 10.0, 16.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(3), t.text_strong);
    ui.painter().galley(r.center() - text.size() / 2.0, text, t.app_bar);
    let now = ui.input(|i| i.time);
    resp.on_hover_text(build.describe(now));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_branch_commit_changes_age_and_checkout() {
        let b = DevBuild {
            checkout: "/src/vectorcraft".into(),
            branch: Some("main".into()),
            commit: Some("44db1c9".into()),
            changes: true,
            age_at_start: Some(90.0),
        };
        assert_eq!(
            b.describe(60.0),
            "Development build of VectorCraft\nBranch main · commit 44db1c9 · with uncommitted changes\nBuilt 2 minutes ago\nFrom /src/vectorcraft"
        );
    }

    #[test]
    fn leaves_out_what_is_unknown() {
        let b = DevBuild { checkout: "/src/vectorcraft".into(), ..Default::default() };
        assert_eq!(b.describe(0.0), "Development build of VectorCraft\nFrom /src/vectorcraft");
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(age_label(5.0), "just now");
        assert_eq!(age_label(f64::NAN), "just now");
        assert_eq!(age_label(60.0), "1 minute ago");
        assert_eq!(age_label(3600.0 * 3.0), "3 hours ago");
        assert_eq!(age_label(86_400.0), "1 day ago");
        assert_eq!(age_label(f64::MAX), format!("{} days ago", u64::MAX / 1440));
    }
}
