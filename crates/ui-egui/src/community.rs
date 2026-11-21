//! Community and project links: the Discord button (app bar) and the URLs About and Home list.
//! Icons are Lucide's (ISC); Discord's own logo is a trademark, so the button uses a chat icon
//! on Discord's colour with the word "Discord".

use craft_ui::buttons::Button;
use craft_ui::contract::{Props, Show};
use craft_ui::primitives::IconSpec;
use craft_ui::style::{DataColor, Variant};
use craft::behavior::IconId;
use egui::{Color32, Ui};

use crate::VectorcraftApp;

/// Discord's brand colour (a colour, not a logo).
const DISCORD: Color32 = Color32::from_rgb(0x58, 0x65, 0xF2);

/// The Discord button: quiet, with Discord's colour on its icon alone. It opens the community link.
pub fn discord_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let icon = IconSpec::svg(IconId::new("message-circle")).data_color(DataColor::new(DISCORD));
    let shown = Button::new(tl!("Discord"))
        .icon(icon)
        .variant(Variant::GHOST)
        .tooltip(tl!("Join the ArtCraft community on Discord (discord.gg/artcraft)"))
        .show(ui);
    if shown.clicks > 0 {
        app.open_link("help.discord");
    }
}

/// The website, app page and GitHub links as (English label, URL), in that order: the one source
/// About and Home list them from.
pub fn link_urls(app: &VectorcraftApp) -> [(&'static str, String); 3] {
    let l = app.session.execute("help.links", &serde_json::json!({})).unwrap_or_default();
    let url = |k: &str| l[k].as_str().unwrap_or("").to_string();
    [
        (tl!("ArtCraft website"), url("website")),
        (tl!("VectorCraft on getartcraft.com"), url("appPage")),
        (tl!("Source code on GitHub"), url("github")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_clicks_open_the_right_urls() {
        use std::sync::{Arc, Mutex};
        let opened = Arc::new(Mutex::new(vec![]));
        let o = opened.clone();
        // PLAN: hub-w3 names the opened-URL seam once `Services::open_url` is deleted.
        let services = crate::Services { open_url: Some(Box::new(move |u: &str| o.lock().unwrap().push(u.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
        for id in ["help.discord", "help.website", "help.appPage", "help.github"] {
            crate::menus::invoke(&mut app, id, serde_json::json!({}));
        }
        assert_eq!(
            *opened.lock().unwrap(),
            [
                "https://discord.gg/artcraft",
                "https://getartcraft.com",
                "https://getartcraft.com/apps/vectorcraft",
                "https://github.com/storytold/vectorcraft"
            ]
        );
        // The control channel / MCP path returns the URL without opening a browser.
        let v = app.run("help.discord", serde_json::json!({})).unwrap();
        assert_eq!(v["url"], "https://discord.gg/artcraft");
        assert_eq!(opened.lock().unwrap().len(), 4);
    }

    #[test]
    fn link_urls_reads_the_three_help_links() {
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let links = link_urls(&app);
        assert_eq!(links.map(|(label, _)| label), ["ArtCraft website", "VectorCraft on getartcraft.com", "Source code on GitHub"]);
        for (label, url) in &links {
            assert!(url.starts_with("https://"), "{label}: {url}");
        }
        assert_eq!(links, link_urls(&app), "the order is stable");
    }
}
