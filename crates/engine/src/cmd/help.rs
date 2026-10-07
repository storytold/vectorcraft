//! Help → project links (the Print That 204 website and this app's GitHub repository). The
//! commands return the URL; the frontend opens it.

use serde_json::{Value, json};

use super::*;

/// The app's display name and its publisher.
pub const APP_NAME: &str = "Vector W3K2";
pub const PUBLISHER: &str = "Print That 204";
/// The app's repository id on GitHub.
pub const APP_ID: &str = "vectorcraft";
pub const WEBSITE_URL: &str = "https://printthat.ca";

/// This app's source repository.
pub fn github_url() -> String {
    format!("https://github.com/printthat204-bot/{APP_ID}")
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "help.website", "Print That 204 Website", ["Help"], None, "{} → {url}", always, |_, _| url(WEBSITE_URL.into())),
        cmd!(query "help.github", "Vector W3K2 on GitHub", ["Help"], None, "{} → {url} source code, issues and releases", always, |_, _| url(github_url())),
        cmd!(query "help.links", "Links", [], None, "{} → {website, github}", always, |_, _| Ok(json!({ "website": WEBSITE_URL, "github": github_url() }))),
    ]
}

fn url(u: String) -> Result<Value> {
    Ok(json!({ "url": u }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let mut s = Session::new();
        assert_eq!(s.execute("help.website", &json!({})).unwrap()["url"], "https://printthat.ca");
        assert_eq!(s.execute("help.github", &json!({})).unwrap()["url"], "https://github.com/printthat204-bot/vectorcraft");
        assert_eq!(s.execute("help.links", &json!({})).unwrap()["website"], "https://printthat.ca");
    }
}
