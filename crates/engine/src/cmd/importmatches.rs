//! What names and codes in other apps' files stand for in VectorCraft, as the user confirmed them on
//! import: a shortcut set's entry names, and its key and modifier codes. They are kept with the
//! preferences on the user's machine, so a later import proposes them and asks only about the rest
//! (`importMatches.*`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CommandSpec, Result, Session, always, bad, cmd, str_param};
use crate::Prefs;

/// The most matches kept.
const MAX_MATCHES: usize = 20_000;
/// The longest name, target or label kept (bytes).
const MAX_TEXT: usize = 1024;
/// The kinds of names: entry names (commands, tools and panels), and a shortcut set's key and
/// modifier codes.
pub const KINDS: &[&str] = &["name", "key", "modifiers"];

/// A name or code in another app's file and what it stands for in VectorCraft
/// ([`crate::Prefs::import_matches`]). Missing fields read as empty, so a preferences file with a
/// damaged match still loads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportMatch {
    /// One of [`KINDS`].
    pub kind: String,
    /// As the file writes it: an entry's name, or a code in decimal (`48`).
    pub name: String,
    /// Kind `name`: a command id, `tool:<id>` or `panel:<id>`, or "" for no equivalent. Kind `key`: a
    /// key as Edit › Keyboard Shortcuts writes it (`K`, `]`, `F5`), or "" for no equivalent. Kind
    /// `modifiers`: modifiers as it writes them (`Cmd+Shift`), or "" for none.
    pub target: String,
    /// What the target was called when it was chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "importMatches.list",
            "Import Matches",
            [],
            None,
            "{kind?: name|key|modifiers, query?: text (in the name, target or label; any case)} the matches the user confirmed when importing other apps' files (shortcut sets), kept with the preferences → [{kind, name, target, label?}] (target \"\": no equivalent)",
            always,
            list
        ),
        cmd!(
            query "importMatches.set",
            "Remember Import Matches",
            [],
            None,
            "{matches: [{kind: name|key|modifiers, name: as the file writes it (codes in decimal), target: a command id, tool:<id> or panel:<id> (name), a key as Edit › Keyboard Shortcuts writes it (key), modifiers as it writes them, such as Cmd+Shift (modifiers), \"\" for no equivalent (no modifiers for kind modifiers), or null to forget the match, label?}]} remember what names and codes in another app's file stand for in VectorCraft, for later imports (at most 20,000 matches; names, targets and labels up to 1,024 bytes) → {count}",
            always,
            set
        ),
        cmd!(
            query "importMatches.reset",
            "Forget Import Matches",
            [],
            None,
            "{kind?: name|key|modifiers} forget the import matches, all of them or those of one kind → {removed}",
            always,
            reset
        ),
    ]
}

/// The target the user confirmed for each kind and name.
pub fn targets(prefs: &Prefs) -> HashMap<(&str, &str), &str> {
    let mut out = HashMap::new();
    for m in &prefs.import_matches {
        out.entry((m.kind.as_str(), m.name.as_str())).or_insert(m.target.as_str());
    }
    out
}

/// `kind` when it is one of [`KINDS`].
fn kind_param(v: Option<&Value>, cmd: &str) -> Result<Option<&'static str>> {
    match v.filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(k) => k
            .as_str()
            .and_then(|k| KINDS.iter().copied().find(|x| *x == k))
            .map(Some)
            .ok_or_else(|| bad(cmd, format!("`kind` is one of {}", KINDS.join(", ")))),
    }
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = kind_param(p.get("kind"), "importMatches.list")?;
    let q = str_param(p, "query").unwrap_or_default().to_lowercase();
    let hit = |m: &ImportMatch| {
        q.is_empty() || [Some(&m.name), Some(&m.target), m.label.as_ref()].into_iter().flatten().any(|t| t.to_lowercase().contains(&q))
    };
    let rows: Vec<&ImportMatch> = s.prefs.import_matches.iter().filter(|m| kind.is_none_or(|k| m.kind == k) && hit(m)).collect();
    Ok(serde_json::to_value(rows).unwrap_or_else(|_| json!([])))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "importMatches.set";
    let rows = p.get("matches").and_then(Value::as_array).ok_or_else(|| bad(C, "give `matches`: [{kind, name, target}]"))?;
    if rows.len() > MAX_MATCHES {
        return Err(bad(C, format!("at most {MAX_MATCHES} matches are kept")));
    }
    let text = |v: Option<&Value>, what: &str| -> Result<Option<String>> {
        match v.filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => {
                let t = v.as_str().ok_or_else(|| bad(C, format!("`{what}` must be text")))?;
                if t.len() > MAX_TEXT {
                    return Err(bad(C, format!("`{what}` is longer than {MAX_TEXT} bytes")));
                }
                Ok(Some(t.to_string()))
            }
        }
    };
    // Checked whole before anything changes.
    let mut changes: Vec<(ImportMatch, bool)> = vec![];
    for row in rows {
        let kind = kind_param(row.get("kind"), C)?.ok_or_else(|| bad(C, "each match needs `kind`"))?;
        let name = text(row.get("name"), "name")?.filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "each match needs a `name`"))?;
        let forget = row.get("target").is_none_or(Value::is_null);
        let target = text(row.get("target"), "target")?.unwrap_or_default();
        let label = text(row.get("label"), "label")?;
        changes.push((ImportMatch { kind: kind.to_string(), name, target, label }, forget));
    }
    // Forgotten matches leave a gap until the end, so the others keep their places.
    let mut next: Vec<Option<ImportMatch>> = s.prefs.import_matches.iter().cloned().map(Some).collect();
    let mut at: HashMap<(String, String), usize> = HashMap::new();
    for (i, m) in s.prefs.import_matches.iter().enumerate() {
        at.entry((m.kind.clone(), m.name.clone())).or_insert(i);
    }
    for (m, forget) in changes {
        let key = (m.kind.clone(), m.name.clone());
        match (at.get(&key).copied(), forget) {
            (Some(i), true) => {
                if let Some(slot) = next.get_mut(i) {
                    *slot = None;
                }
                at.remove(&key);
            }
            (Some(i), false) => {
                if let Some(slot) = next.get_mut(i) {
                    *slot = Some(m);
                }
            }
            (None, true) => {}
            (None, false) => {
                at.insert(key, next.len());
                next.push(Some(m));
            }
        }
    }
    let next: Vec<ImportMatch> = next.into_iter().flatten().collect();
    if next.len() > MAX_MATCHES {
        return Err(bad(C, format!("at most {MAX_MATCHES} matches are kept")));
    }
    s.prefs.import_matches = next;
    Ok(json!({ "count": s.prefs.import_matches.len() }))
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = kind_param(p.get("kind"), "importMatches.reset")?;
    let before = s.prefs.import_matches.len();
    s.prefs.import_matches.retain(|m| kind.is_some_and(|k| m.kind != k));
    Ok(json!({ "removed": before - s.prefs.import_matches.len() }))
}
