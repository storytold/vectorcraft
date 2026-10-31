//! Window → Color Themes: five-colour themes made on a harmony wheel, kept in a local library in the
//! preferences (no online service). `colorTheme.addToSwatches` adds a theme to the Swatches panel as a
//! colour group.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::harmony::{Harmony, THEME_SIZE};
use vectorcraft_color::recolor::ColorKey;
use vectorcraft_tools::params::color_json;

use super::{CommandSpec, Result, Session, always, bad, cmd, color_value, has_doc, str_param};

/// A saved colour theme ([`crate::Prefs::color_themes`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorTheme {
    pub name: String,
    /// 1 to [`THEME_SIZE`] colours, each in its own model.
    pub colors: Vec<Color>,
    /// The harmony rule it was made with (a rule id), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "colorTheme.list",
            "Color Themes",
            [],
            None,
            "{} the saved colour themes, in order → {themes: [{name, colors: [\"#rrggbb\"], keys: [colour keys, exact in each colour's model], rule?}]}",
            always,
            list
        ),
        cmd!(
            "colorTheme.save",
            "Save Theme",
            ["Window", "Color Themes"],
            None,
            "{colors: [1–5 colours] | color + rule (a harmony rule id or label, see color.harmony: its 5-colour theme from that base colour, base first), rule?: the rule it was made with, name?: (default \"Theme N\"), replace?: a saved theme's name (overwrite that theme in place, e.g. after editing it; it may be renamed), color?: one colour (as colors: [colour])} save a colour theme to the local theme library (the preferences) → {name, count: themes saved}",
            always,
            save
        ),
        cmd!(
            "colorTheme.delete",
            "Delete Theme",
            ["Window", "Color Themes"],
            None,
            "{name} delete a saved colour theme → {deleted}",
            has_themes,
            delete
        ),
        cmd!(
            "colorTheme.addToSwatches",
            "Add to Swatches",
            ["Window", "Color Themes"],
            None,
            "{name} add a saved theme's colours to the Swatches panel as a colour group named after it (as swatch.newGroup), as one undo step → {name: the group's, swatches: [names]}",
            has_doc,
            add_to_swatches
        ),
    ]
}

fn has_themes(s: &Session) -> std::result::Result<(), String> {
    if s.prefs.color_themes.is_empty() { Err("no saved themes".into()) } else { Ok(()) }
}

/// A theme as the commands list it.
pub fn theme_json(t: &ColorTheme) -> Value {
    let mut v = json!({
        "name": t.name,
        "colors": t.colors.iter().map(Color::to_hex).collect::<Vec<_>>(),
        "keys": t.colors.iter().map(|c| ColorKey::of(c).to_string()).collect::<Vec<_>>(),
    });
    if let Some(r) = &t.rule {
        v["rule"] = json!(r);
    }
    v
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "themes": s.prefs.color_themes.iter().map(theme_json).collect::<Vec<_>>() }))
}

/// The index of the saved theme called `name`.
fn find(s: &Session, name: &str, cmd: &str) -> Result<usize> {
    s.prefs.color_themes.iter().position(|t| t.name == name).ok_or_else(|| bad(cmd, format!("no saved theme called {name} (see colorTheme.list)")))
}

/// The first free "Theme N".
fn next_name(s: &Session) -> String {
    (1..).map(|n| format!("Theme {n}")).find(|n| !s.prefs.color_themes.iter().any(|t| t.name.eq_ignore_ascii_case(n))).unwrap_or_default()
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "colorTheme.save";
    let rule = match str_param(p, "rule") {
        Some(r) => Some(Harmony::parse(r).ok_or_else(|| bad(C, format!("unknown rule `{r}` (see color.harmony)")))?),
        None => None,
    };
    let colors: Vec<Color> = match (p.get("colors"), p.get("color")) {
        (Some(Value::Array(cs)), _) => {
            cs.iter().map(|c| color_value(c).ok_or_else(|| bad(C, format!("invalid colour {c}")))).collect::<Result<_>>()?
        }
        (None, Some(c)) => {
            let base = color_value(c).ok_or_else(|| bad(C, "invalid `color`"))?;
            rule.ok_or_else(|| bad(C, "`color` needs a `rule` to make the theme from"))?.theme(base)
        }
        _ => return Err(bad(C, "give `colors`, or `color` and `rule`")),
    };
    if colors.is_empty() || colors.len() > THEME_SIZE {
        return Err(bad(C, format!("a theme has 1 to {THEME_SIZE} colours")));
    }
    let replace = str_param(p, "replace").map(|r| find(s, r, C)).transpose()?;
    let name = match str_param(p, "name").map(str::trim) {
        Some("") => return Err(bad(C, "the name is empty")),
        Some(n) => n.to_string(),
        None => replace.map_or_else(|| next_name(s), |i| s.prefs.color_themes[i].name.clone()),
    };
    if s.prefs.color_themes.iter().enumerate().any(|(i, t)| Some(i) != replace && t.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(C, format!("a theme is already called {name} (replace: \"{name}\" overwrites it)")));
    }
    let theme = ColorTheme { name: name.clone(), colors, rule: rule.map(Harmony::id) };
    match replace {
        Some(i) => s.prefs.color_themes[i] = theme,
        None => s.prefs.color_themes.push(theme),
    }
    Ok(json!({ "name": name, "count": s.prefs.color_themes.len() }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "colorTheme.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    let i = find(s, name, C)?;
    Ok(json!({ "deleted": s.prefs.color_themes.remove(i).name }))
}

fn add_to_swatches(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "colorTheme.addToSwatches";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    let t = &s.prefs.color_themes[find(s, name, C)?];
    // The theme's colors keep their model, in a CMYK document too.
    let params = json!({ "name": t.name, "colors": t.colors.iter().map(color_json).collect::<Vec<_>>(), "keepModel": true });
    let r = s.execute("swatch.newGroup", &params)?;
    Ok(json!({ "name": r["name"], "swatches": r["swatches"] }))
}
