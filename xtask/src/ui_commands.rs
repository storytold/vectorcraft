//! `cargo xtask ui-commands`: regenerate or check the MCP mirror of desktop `UI_COMMANDS`.
//!
//! Source of truth: `crates/ui-egui/src/menus/ui_commands.rs`.
//! Mirror: `crates/mcp/src/ui_catalog.rs` (mcp must not depend on ui-egui).
//!
//! `cargo xtask ui-commands` writes the mirror.
//! `cargo xtask ui-commands --check` fails when the committed file drifts.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const SOURCE: &str = "crates/ui-egui/src/menus/ui_commands.rs";
pub const MIRROR: &str = "crates/mcp/src/ui_catalog.rs";
const UI_EGUI_SRC: &str = "crates/ui-egui/src";

pub fn run(root: &Path, check_only: bool) -> Result<(), String> {
    let expected = generate(root)?;
    let mirror_path = root.join(MIRROR);
    if check_only {
        let actual = std::fs::read_to_string(&mirror_path).map_err(|e| format!("{MIRROR}: {e}"))?;
        if actual == expected {
            println!("ui-commands: {MIRROR} matches {SOURCE}");
            return Ok(());
        }
        return Err(format!("{MIRROR} is out of date with {SOURCE}. Run `cargo xtask ui-commands` and commit the result."));
    }
    if let Some(parent) = mirror_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{parent:?}: {e}"))?;
    }
    std::fs::write(&mirror_path, expected).map_err(|e| format!("{MIRROR}: {e}"))?;
    println!("ui-commands: wrote {MIRROR}");
    Ok(())
}

/// Build the mirror file contents from the desktop table (rustfmt'd, same as `cargo fmt`).
pub fn generate(root: &Path) -> Result<String, String> {
    let source = std::fs::read_to_string(root.join(SOURCE)).map_err(|e| format!("{SOURCE}: {e}"))?;
    let entries = parse_ui_commands(&source)?;
    let resolved = resolve_entries(root, &entries)?;
    rustfmt_source(root, &render_mirror(&resolved))
}

/// Format `src` with the workspace `rustfmt` (edition 2024, same as `cargo fmt -p vectorcraft-mcp`).
fn rustfmt_source(root: &Path, src: &str) -> Result<String, String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("vectorcraft-ui-catalog-{}.rs", std::process::id()));
    std::fs::write(&path, src).map_err(|e| format!("temp mirror: {e}"))?;
    let config = root.join("rustfmt.toml");
    let status =
        Command::new("rustfmt").args(["--edition", "2024", "--config-path"]).arg(&config).arg(&path).status().map_err(|e| format!("rustfmt: {e}"))?;
    let result = if status.success() {
        std::fs::read_to_string(&path).map_err(|e| format!("temp mirror read: {e}"))
    } else {
        Err(format!("rustfmt failed with {status}"))
    };
    let _ = std::fs::remove_file(&path);
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    id: IdExpr,
    label: String,
    shortcut: String,
    params: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IdExpr {
    Lit(String),
    Path(String),
}

fn parse_ui_commands(src: &str) -> Result<Vec<Entry>, String> {
    let start = src.find("pub const UI_COMMANDS").ok_or_else(|| format!("{SOURCE}: no `pub const UI_COMMANDS`"))?;
    let after = &src[start..];
    // Prefer `= &[` so the type `&[(&str, …)]` is not mistaken for the value.
    let bracket = after.find("= &[").ok_or_else(|| format!("{SOURCE}: UI_COMMANDS is not an `= &[...]`"))?;
    let open_idx = start + bracket + 3; // index of '['
    let body_start = open_idx + 1;
    let body_end = find_matching_bracket(src, open_idx).ok_or_else(|| format!("{SOURCE}: unclosed UI_COMMANDS array"))?;
    let body = &src[body_start..body_end];
    let tuples = split_top_level_tuples(body)?;
    let mut out = Vec::with_capacity(tuples.len());
    for t in tuples {
        out.push(parse_tuple(t)?);
    }
    if out.is_empty() {
        return Err(format!("{SOURCE}: UI_COMMANDS is empty"));
    }
    Ok(out)
}

fn find_matching_bracket(src: &str, open_idx: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if open_idx >= bytes.len() || bytes[open_idx] != b'[' {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    let mut i = open_idx;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_top_level_tuples(body: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && ((bytes[i] as char).is_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] != b'(' {
            return Err(format!("{SOURCE}: expected `(` in UI_COMMANDS, found {:?}", body[i..].chars().take(20).collect::<String>()));
        }
        let start = i;
        let mut depth = 0usize;
        let mut in_str = false;
        let mut esc = false;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        out.push(&body[start..i]);
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return Err(format!("{SOURCE}: unclosed tuple in UI_COMMANDS"));
        }
    }
    Ok(out)
}

fn parse_tuple(tup: &str) -> Result<Entry, String> {
    let inner = tup.strip_prefix('(').and_then(|s| s.strip_suffix(')')).ok_or_else(|| format!("{SOURCE}: bad tuple {tup:?}"))?;
    let fields = split_fields(inner)?;
    if fields.len() != 4 {
        return Err(format!("{SOURCE}: tuple has {} fields, want 4: {tup}", fields.len()));
    }
    let id = parse_id(fields[0].trim())?;
    let label = parse_string_lit(fields[1].trim())?;
    let shortcut = parse_string_lit(fields[2].trim())?;
    let params = parse_string_lit(fields[3].trim())?;
    Ok(Entry { id, label, shortcut, params })
}

fn split_fields(inner: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let bytes = inner.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut depth = 0isize;
    let mut in_str = false;
    let mut esc = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    Ok(out)
}

fn parse_id(field: &str) -> Result<IdExpr, String> {
    if field.starts_with('"') {
        return Ok(IdExpr::Lit(parse_string_lit(field)?));
    }
    if field.starts_with("crate::") {
        return Ok(IdExpr::Path(field.to_string()));
    }
    Err(format!("{SOURCE}: unsupported id expression `{field}`"))
}

fn parse_string_lit(field: &str) -> Result<String, String> {
    let s = field.trim();
    if !s.starts_with('"') {
        return Err(format!("{SOURCE}: expected string literal, got `{s}`"));
    }
    let mut out = String::new();
    let mut chars = s[1..].chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let Some(n) = chars.next() else {
                return Err(format!("{SOURCE}: trailing escape in {s}"));
            };
            match n {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '\\' | '"' | '\'' => out.push(n),
                other => return Err(format!("{SOURCE}: unknown escape `\\{other}` in {s}")),
            }
        } else if c == '"' {
            return Ok(out);
        } else {
            out.push(c);
        }
    }
    Err(format!("{SOURCE}: unclosed string {s}"))
}

fn resolve_entries(root: &Path, entries: &[Entry]) -> Result<Vec<(String, String, String, String)>, String> {
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let id = match &e.id {
            IdExpr::Lit(s) => s.clone(),
            IdExpr::Path(path) => resolve_const_path(root, path)?,
        };
        out.push((id, e.label.clone(), e.shortcut.clone(), e.params.clone()));
    }
    Ok(out)
}

fn resolve_const_path(root: &Path, path: &str) -> Result<String, String> {
    let trimmed = path.strip_prefix("crate::").ok_or_else(|| format!("{SOURCE}: path `{path}` does not start with crate::"))?;
    let mut parts: Vec<&str> = trimmed.split("::").collect();
    let const_name = parts.pop().ok_or_else(|| format!("{SOURCE}: empty path `{path}`"))?;
    resolve_in_module(root, &parts, const_name, path)
}

fn resolve_in_module(root: &Path, module_parts: &[&str], const_name: &str, original: &str) -> Result<String, String> {
    let file = module_file(root, module_parts).ok_or_else(|| format!("{SOURCE}: no file for `{original}`"))?;
    let src = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    if let Some(v) = find_str_const(&src, const_name) {
        return Ok(v);
    }
    // Follow `pub use path::{..., CONST, ...}` / `pub use path::CONST`.
    for target in reexport_targets(&src, const_name) {
        let mut next = module_parts.to_vec();
        if target.first() == Some(&"super") {
            next.pop();
            next.extend(target.iter().skip(1).copied());
        } else if target.first() == Some(&"crate") {
            next = target.iter().skip(1).copied().collect();
        } else {
            next.extend(target);
        }
        if let Ok(v) = resolve_in_module(root, &next, const_name, original) {
            return Ok(v);
        }
    }
    Err(format!("{SOURCE}: could not resolve `{original}` (`pub const {const_name}: &str` missing under {})", file.display()))
}

fn module_file(root: &Path, parts: &[&str]) -> Option<PathBuf> {
    let base = root.join(UI_EGUI_SRC);
    if parts.is_empty() {
        return Some(base.join("lib.rs"));
    }
    let mut dir = base.clone();
    for (i, p) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if last {
            let rs = dir.join(format!("{p}.rs"));
            if rs.is_file() {
                return Some(rs);
            }
            let mod_rs = dir.join(p).join("mod.rs");
            if mod_rs.is_file() {
                return Some(mod_rs);
            }
            return None;
        }
        dir = dir.join(p);
    }
    None
}

fn find_str_const(src: &str, name: &str) -> Option<String> {
    // `pub const NAME: &str = "...";` (allow whitespace)
    let needle = format!("pub const {name}:");
    let mut search = src;
    while let Some(idx) = search.find(&needle) {
        let after = &search[idx + needle.len()..];
        let after = after.trim_start();
        if !after.starts_with("&str") {
            search = &search[idx + needle.len()..];
            continue;
        }
        let after = after["&str".len()..].trim_start();
        if !after.starts_with('=') {
            search = &search[idx + needle.len()..];
            continue;
        }
        let after = after[1..].trim_start();
        return parse_string_lit(after).ok();
    }
    None
}

fn reexport_targets<'a>(src: &'a str, const_name: &str) -> Vec<Vec<&'a str>> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if !t.starts_with("pub use ") {
            continue;
        }
        let rest = t.trim_start_matches("pub use ").trim_end_matches(';').trim();
        if let Some((path, braced)) = rest.split_once("::{") {
            let names = braced.trim_end_matches('}');
            let hit = names.split(',').map(str::trim).any(|n| n == const_name);
            if hit {
                out.push(path.split("::").collect());
            }
        } else if let Some((path, name)) = rest.rsplit_once("::")
            && name.trim() == const_name
        {
            out.push(path.split("::").collect());
        }
    }
    out
}

fn render_mirror(rows: &[(String, String, String, String)]) -> String {
    let mut out = String::new();
    out.push_str("//! Desktop UI command catalogue for headless MCP discovery (`ui: true`).\n");
    out.push_str("//! Generated from `crates/ui-egui/src/menus/ui_commands.rs` by `cargo xtask ui-commands`.\n");
    out.push_str("//! Do not edit by hand; re-run the xtask when that table changes.\n");
    out.push('\n');
    out.push_str("/// `(id, label, shortcut, params doc)` — same shape as the desktop `UI_COMMANDS` table.\n");
    out.push_str("pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[\n");
    for (id, label, shortcut, params) in rows {
        out.push_str("    (");
        out.push_str(&rust_string_lit(id));
        out.push_str(", ");
        out.push_str(&rust_string_lit(label));
        out.push_str(", ");
        out.push_str(&rust_string_lit(shortcut));
        out.push_str(", ");
        out.push_str(&rust_string_lit(params));
        out.push_str("),\n");
    }
    out.push_str("];\n");
    out
}

fn rust_string_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literal_and_path_ids() {
        let src = r#"
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    (
        crate::addons::arrange::ZOOM,
        "Zoom to Selection",
        "",
        "{importer: \"plugin.<id>\"}",
    ),
    ("app.language", "Interface Language", "Cmd+L", "{}"),
];
"#;
        let entries = parse_ui_commands(src).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].id, IdExpr::Path(_)));
        assert_eq!(entries[0].params, "{importer: \"plugin.<id>\"}");
        assert_eq!(entries[1].id, IdExpr::Lit("app.language".into()));
    }

    #[test]
    fn escape_round_trip_no_extra_backslash() {
        let s = "{importer: \"plugin.<id>\"}";
        let lit = rust_string_lit(s);
        assert_eq!(lit, r#""{importer: \"plugin.<id>\"}""#);
        assert!(!lit.contains(r#"\\\""#));
    }

    #[test]
    fn find_const_in_source() {
        let src = r#"pub const ZOOM: &str = "addon.arrange.zoomSelection";"#;
        assert_eq!(find_str_const(src, "ZOOM").as_deref(), Some("addon.arrange.zoomSelection"));
    }
}
