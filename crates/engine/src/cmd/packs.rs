//! Language packs as commands (#4936): list, install and remove the packs that spell-check and
//! hyphenate languages other than English ([`crate::packs`]). They open nothing and never turn
//! downloading on: an install needs the Craft apps' shared download setting, which only the person
//! using an app turns on.

use serde_json::{Value, json};

use super::{CommandSpec, EngineError, Result, Session, always, bad, cmd, text_param};
use crate::packs::{self, Outcome};

/// The Language pack commands.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "proof.packs",
            "Language Packs",
            [],
            None,
            "{refresh?: bool (fetch the pinned pack index again; needs downloading on; refused under automation roots; in the desktop app it runs in the background: see refreshing)} the Language packs that spell-check and hyphenate languages other than English, shared by the Craft apps → {packs: [{locale, name, size: bytes, licence: SPDX, class, spelling: bool, hyphenation: bool, installed: bool}…] (the index fetched last; empty before the first fetch), installed: [{locale, name, licence}…] (the packs spelling and hyphenation use), installing: [{locale, done, total}…] (bytes), refreshing: bool, errors: {locale: message} (the last failure of each install or removal; \"\": of the index fetch), downloads: bool (the person allowed downloading), indexOverride: string|null (a test index used instead of the pinned one), available: bool (there is a font store folder to install into)}",
            always,
            list
        ),
        cmd!(
            query "proof.installPack",
            "Install Language Pack",
            [],
            None,
            "{locale: language tag (a pack's locale from proof.packs, like nl or pt-br)} downloads the pack, checks its sha256 and installs it; spelling and hyphenation in that language use it at once, in every open document. Needs downloading on, which only the person using a Craft app can turn on. Refused under automation roots (the store is outside them). Opens nothing. In the desktop app it runs in the background → {locale, started: true}: follow it with proof.packs (installing, then installed or errors); elsewhere it returns once installed → {locale, installed: true}",
            always,
            install
        ),
        cmd!(
            query "proof.removePack",
            "Remove Language Pack",
            [],
            None,
            "{locale} removes the installed pack (not while it installs; refused under automation roots); that language is no longer checked with it → {locale, installed: false}",
            always,
            remove
        ),
    ]
}

fn locale_param(p: &Value, command: &str) -> Result<String> {
    let raw = text_param(p, "locale", command)?.ok_or_else(|| bad(command, "`locale` (a language tag like nl) is required"))?;
    packs::parse_locale(raw).map_err(|e| bad(command, e))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "proof.packs";
    match p.get("refresh") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => {
            not_confined(C)?;
            packs::refresh().map_err(EngineError::Other)?;
        }
        Some(_) => return Err(bad(C, "`refresh` must be true or false")),
    }
    // The jobs before the installed packs: an install makes craft-proof read its pack before it
    // leaves the jobs, so one that ends between the two reads is in one list or the other.
    let packs::Status { installing, refreshing, errors } = packs::status();
    let installed = craft_proof::packs::installed();
    let backend = packs::backend();
    let rows: Vec<Value> = packs::available()
        .iter()
        .map(|p| {
            json!({
                "locale": p.locale, "name": p.name, "size": p.size, "licence": p.licence, "class": p.class,
                "spelling": p.spelling, "hyphenation": p.hyphenation,
                "installed": installed.iter().any(|i| i.locale == p.locale),
            })
        })
        .collect();
    // An install that ended since the last frame shows in every document's layout.
    s.poll_language_packs();
    Ok(json!({
        "packs": rows,
        "installed": installed.iter().map(|i| json!({"locale": i.locale, "name": i.name, "licence": i.licence})).collect::<Vec<_>>(),
        "installing": installing.iter().map(|(l, done, total)| json!({"locale": l, "done": done, "total": total})).collect::<Vec<_>>(),
        "refreshing": refreshing,
        "errors": errors,
        "downloads": backend.as_ref().is_some_and(|b| b.downloads_on()),
        "indexOverride": backend.as_ref().and_then(|b| b.index_override()),
        "available": backend.is_some(),
    }))
}

/// Refuses `command` for a caller confined to automation roots: the packs are written into the font
/// store the Craft apps share, which is outside them.
fn not_confined(command: &str) -> Result<()> {
    if crate::file_access::current().is_some() {
        return Err(EngineError::Other(format!(
            "automation path rejected: {command} writes the Language packs into the font store the Craft apps share, outside the automation roots"
        )));
    }
    Ok(())
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    let locale = locale_param(p, "proof.installPack")?;
    not_confined("proof.installPack")?;
    match packs::install(&locale).map_err(EngineError::Other)? {
        Outcome::Started => Ok(json!({"locale": locale, "started": true})),
        Outcome::Done => {
            s.poll_language_packs();
            Ok(json!({"locale": locale, "installed": true}))
        }
    }
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let locale = locale_param(p, "proof.removePack")?;
    not_confined("proof.removePack")?;
    packs::remove(&locale).map_err(EngineError::Other)?;
    s.poll_language_packs();
    Ok(json!({"locale": locale, "installed": false}))
}
