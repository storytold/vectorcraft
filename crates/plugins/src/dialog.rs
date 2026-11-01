//! How the host's generated dialogs show a plug-in's parameters: the optional `label`, `unit`,
//! `group`, `help`, `helpUrl` and `visibleWhen` keys of each one (docs/plugins.md › Parameters).
//!
//! These keys are presentation only. A host from before them ignores them, so a plug-in that
//! declares them still installs there, and a plug-in is given every parameter whether its dialog
//! showed it or not: a run never depends on what was on screen. The dialogs of filters, live
//! effects and exporters draw the parameters outside any group first, then each group as a section,
//! in the order the groups first appear, and leave out a parameter while its `visibleWhen` isn't met.
//!
//! `visibleWhen` names parameters declared before the one it belongs to, so the parameter that
//! decides comes first in the dialog and conditions can't go round in a circle. A parameter is shown
//! when each one it names is shown and has one of the values listed for it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::manifest::{Manifest, Param, ParamSpec};
use crate::{Error, Result};

/// Longest `label` or `group`, in characters.
pub const MAX_LABEL: usize = 64;
/// Longest `unit`, in characters.
pub const MAX_UNIT: usize = 16;
/// Longest `help`, in characters.
pub const MAX_HELP: usize = 1_000;
/// Longest `helpUrl`, in bytes.
pub const MAX_HELP_URL: usize = 512;
/// Most parameters one `visibleWhen` names.
pub const MAX_CONDITIONS: usize = 8;
/// Most values a `visibleWhen` lists for one parameter.
pub const MAX_CONDITION_VALUES: usize = 64;

/// One section of a dialog ([`Manifest::sections`]): its group (`None`: the parameters outside any
/// group) and its parameters, in declaration order.
pub type Section<'a> = (Option<&'a str>, Vec<&'a (String, Param)>);

/// How dialogs show one parameter (every key optional).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamUi {
    /// The field's label; without one, the name spaced out (`rowSpacing` → "Row Spacing").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The unit the value is in, shown after the field (the plug-in's own: nothing is converted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The section of the dialog the parameter is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// What the parameter does, shown when the pointer rests on its label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// An `https://` page about the parameter, opened by the help button after its field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
    /// Shown only while each parameter named here has one of the values listed for it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visible_when: BTreeMap<String, Vec<Value>>,
}

/// Is `s` 1 to `max` characters, not all spaces, without control characters (`\n` too, unless
/// `lines`)?
fn printable(s: &str, max: usize, lines: bool) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && s.chars().all(|c| !c.is_control() || (lines && c == '\n'))
}

impl ParamUi {
    /// Checks the keys of parameter `name`, the `index`th of manifest `m`'s parameters.
    pub(crate) fn validate(&self, m: &Manifest, index: usize, name: &str) -> Result<()> {
        let bad = |what: &str| Err(Error::Manifest(format!("parameter `{name}`: {what}")));
        let text = |v: &Option<String>, max: usize, lines: bool| v.as_deref().is_none_or(|s| printable(s, max, lines));
        if !text(&self.label, MAX_LABEL, false) || !text(&self.group, MAX_LABEL, false) {
            return bad(&format!("`label` and `group` must be 1-{MAX_LABEL} printable characters"));
        }
        if !text(&self.unit, MAX_UNIT, false) {
            return bad(&format!("`unit` must be 1-{MAX_UNIT} printable characters"));
        }
        if !text(&self.help, MAX_HELP, true) {
            return bad(&format!("`help` must be 1-{MAX_HELP} printable characters"));
        }
        if let Some(url) = &self.help_url {
            let rest = url.strip_prefix("https://").unwrap_or_default();
            if rest.is_empty() || url.len() > MAX_HELP_URL || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return bad(&format!("`helpUrl` must be an https:// address of at most {MAX_HELP_URL} bytes"));
            }
        }
        if self.visible_when.len() > MAX_CONDITIONS {
            return bad(&format!("`visibleWhen` names at most {MAX_CONDITIONS} parameters"));
        }
        let before = m.params.get(..index).unwrap_or_default();
        for (other, values) in &self.visible_when {
            let Some((_, p)) = before.iter().find(|(k, _)| k == other) else {
                return bad(&format!("`visibleWhen` names `{other}`, which isn't a parameter declared before this one"));
            };
            if values.is_empty() || values.len() > MAX_CONDITION_VALUES {
                return bad(&format!("`visibleWhen` lists 1 to {MAX_CONDITION_VALUES} values for `{other}`"));
            }
            let fits = |v: &Value| match &p.spec {
                ParamSpec::Bool { .. } => v.is_boolean(),
                ParamSpec::Choice { options, .. } => v.as_str().is_some_and(|s| options.iter().any(|o| o == s)),
                _ => false,
            };
            if !values.iter().all(fits) {
                return bad(&format!("`visibleWhen` takes `{other}`'s values, of a checkbox or a choice"));
            }
        }
        Ok(())
    }
}

impl Manifest {
    /// The names of the parameters shown while the dialog's values are `values` (missing ones at
    /// their defaults): those whose `visibleWhen` names only shown parameters, each with one of the
    /// values listed for it.
    ///
    /// One pass in declaration order decides each parameter once, from the parameters before it.
    /// Following the conditions back instead would revisit a parameter once per path to it, and
    /// overlapping conditions have exponentially many. A condition on a parameter declared later,
    /// which only a manifest built in code can have, hides the parameter, so conditions that go
    /// round in a circle hide every parameter on it.
    pub fn visible(&self, values: &Map<String, Value>) -> BTreeSet<&str> {
        let mut shown = BTreeSet::new();
        for (name, p) in &self.params {
            let met = p.ui.visible_when.iter().all(|(other, allowed)| {
                shown.contains(other.as_str())
                    && match values.get(other) {
                        Some(v) => allowed.contains(v),
                        None => self.param(other).is_some_and(|o| allowed.contains(&o.spec.default_value())),
                    }
            });
            if met {
                shown.insert(name.as_str());
            }
        }
        shown
    }

    /// Is parameter `name` shown while the dialog's values are `values`? See [`Manifest::visible`],
    /// which a dialog calls once for all its parameters.
    pub fn shown(&self, name: &str, values: &Map<String, Value>) -> bool {
        self.visible(values).contains(name)
    }

    /// The parameter named `name`.
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, p)| p)
    }

    /// The dialog's sections in order: the parameters outside any group (`None`), then each group
    /// in the order it first appears, each with its parameters in declaration order.
    pub fn sections(&self) -> Vec<Section<'_>> {
        let mut out: Vec<Section> = Vec::new();
        let ungrouped: Vec<_> = self.params.iter().filter(|(_, p)| p.ui.group.is_none()).collect();
        if !ungrouped.is_empty() {
            out.push((None, ungrouped));
        }
        for entry in &self.params {
            let Some(group) = entry.1.ui.group.as_deref() else { continue };
            match out.iter_mut().find(|(g, _)| *g == Some(group)) {
                Some((_, members)) => members.push(entry),
                None => out.push((Some(group), vec![entry])),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const RICH: &str = r#"{"id":"org.example.hatch","name":"Hatch","kind":"effect","params":{
        "pattern":{"type":"choice","options":["lines","crosshatch","dots"],"default":"lines",
                   "label":"Pattern","group":"Fill","help":"How the shape is filled."},
        "spacing":{"type":"number","min":0.1,"max":10,"default":0.25,"label":"Spacing","unit":"mm","group":"Fill",
                   "help":"Distance between the lines.\nSmaller is denser.","helpUrl":"https://example.org/params/spacing",
                   "visibleWhen":{"pattern":["lines","crosshatch"]}},
        "outline":{"type":"bool","default":true,"label":"Outline"},
        "outline_width":{"type":"number","min":0,"max":10,"default":0.5,"unit":"pt","group":"Outline","visibleWhen":{"outline":[true]}},
        "outline_passes":{"type":"int","min":1,"max":4,"default":1,"group":"Outline",
                          "visibleWhen":{"outline":[true],"pattern":["lines"]}},
        "seed":{"type":"int","min":0,"max":9}}}"#;

    fn rich() -> Manifest {
        Manifest::parse(RICH.as_bytes()).unwrap()
    }

    #[test]
    fn keys_parse_and_round_trip_with_the_parameters() {
        let m = rich();
        let spacing = m.param("spacing").unwrap();
        assert_eq!(
            (spacing.ui.label.as_deref(), spacing.ui.unit.as_deref(), spacing.ui.group.as_deref()),
            (Some("Spacing"), Some("mm"), Some("Fill"))
        );
        assert_eq!(spacing.ui.help_url.as_deref(), Some("https://example.org/params/spacing"));
        assert_eq!(spacing.spec, ParamSpec::Number { min: 0.1, max: 10.0, default: 0.25 });
        assert_eq!(m.param("seed").unwrap().ui, ParamUi::default());
        // Written out, the manifest reads back as it was, parameters in their order.
        assert_eq!(Manifest::parse(&serde_json::to_vec(&m).unwrap()).unwrap(), m);
        assert_eq!(serde_json::to_value(&m).unwrap()["params"]["seed"], json!({"type": "int", "min": 0, "max": 9, "default": 0}));
    }

    #[test]
    fn sections_put_ungrouped_parameters_first_then_groups_as_they_appear() {
        let m = rich();
        let names: Vec<(Option<&str>, Vec<&str>)> =
            m.sections().into_iter().map(|(g, ps)| (g, ps.into_iter().map(|(k, _)| k.as_str()).collect())).collect();
        assert_eq!(
            names,
            [(None, vec!["outline", "seed"]), (Some("Fill"), vec!["pattern", "spacing"]), (Some("Outline"), vec!["outline_width", "outline_passes"])]
        );
    }

    #[test]
    fn visible_when_follows_the_values_and_the_parameters_it_names() {
        let m = rich();
        let shown = |values: Value| -> Vec<&str> {
            let values = values.as_object().cloned().unwrap_or_default();
            m.params.iter().filter(|(k, _)| m.shown(k, &values)).map(|(k, _)| k.as_str()).collect()
        };
        // At the defaults: lines, with an outline.
        assert_eq!(shown(json!({})), ["pattern", "spacing", "outline", "outline_width", "outline_passes", "seed"]);
        assert_eq!(shown(json!({"pattern": "dots"})), ["pattern", "outline", "outline_width", "seed"]);
        assert_eq!(shown(json!({"outline": false})), ["pattern", "spacing", "outline", "seed"]);
        assert!(!m.shown("no_such_parameter", &Map::new()));
    }

    /// A manifest built in code rather than parsed can name parameters in a circle: they are
    /// hidden instead of recursing forever.
    #[test]
    fn conditions_in_a_circle_hide_their_parameters() {
        let mut m = rich();
        let outline = m.params.iter().position(|(k, _)| k == "outline").unwrap();
        // `outline` names `outline_width`, which names `outline`.
        m.params[outline].1.ui.visible_when.insert("outline_width".into(), vec![json!(0.5)]);
        assert!(!m.shown("outline", &Map::new()) && !m.shown("outline_width", &Map::new()));
        assert!(m.shown("seed", &Map::new()) && m.shown("pattern", &Map::new()));
    }

    /// Conditions that each name several earlier parameters are checked once per parameter, not
    /// once per path through them: 256 checkboxes, each shown while the 8 before it are checked,
    /// would otherwise take longer than the age of the universe on every frame of the dialog.
    #[test]
    fn overlapping_conditions_are_checked_once_per_parameter() {
        let params: Vec<String> = (0..256)
            .map(|i: usize| {
                let when: Vec<String> = (i.saturating_sub(MAX_CONDITIONS)..i).map(|j| format!(r#""p{j}":[true]"#)).collect();
                format!(r#""p{i}":{{"type":"bool","default":true,"group":"G","visibleWhen":{{{}}}}}"#, when.join(","))
            })
            .collect();
        let m = Manifest::parse(format!(r#"{{"id":"a","name":"x","kind":"filter","params":{{{}}}}}"#, params.join(",")).as_bytes()).unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || done.send(m.visible(&Map::new()).len()));
        let shown = finished.recv_timeout(std::time::Duration::from_secs(10)).expect("the check took more than 10 s");
        assert_eq!(shown, 256, "every checkbox is checked, so every parameter is shown");
        // Unchecking the first hides the rest: each names it or one it hides.
        let m = Manifest::parse(format!(r#"{{"id":"a","name":"x","kind":"filter","params":{{{}}}}}"#, params.join(",")).as_bytes()).unwrap();
        let values = json!({"p0": false}).as_object().cloned().unwrap();
        assert_eq!(m.visible(&values).into_iter().collect::<Vec<_>>(), ["p0"]);
    }

    #[test]
    fn bad_keys_are_refused() {
        let with = |key: &str, value: &str| {
            format!(
                r#"{{"id":"a","name":"x","kind":"filter","params":{{"mode":{{"type":"choice","options":["a","b"]}},
                    "on":{{"type":"bool"}},"n":{{"type":"number","min":0,"max":1,"default":0,"{key}":{value}}}}}}}"#
            )
        };
        let long = format!("\"{}\"", "x".repeat(MAX_LABEL + 1));
        for (key, value) in [
            ("label", r#""""#),
            ("label", r#""  ""#),
            ("label", r#""a\u0007b""#),
            ("label", long.as_str()),
            ("group", r#""line\nbreak""#),
            ("unit", r#""millimetres and more""#),
            ("help", r#""""#),
            ("helpUrl", r#""http://example.org""#),
            ("helpUrl", r#""https://""#),
            ("helpUrl", r#""https://exa mple.org""#),
            ("helpUrl", r#""javascript:alert(1)""#),
            ("visibleWhen", r#"{"missing":[true]}"#),
            ("visibleWhen", r#"{"n":[0]}"#),
            ("visibleWhen", r#"{"on":["yes"]}"#),
            ("visibleWhen", r#"{"mode":["c"]}"#),
            ("visibleWhen", r#"{"mode":[]}"#),
        ] {
            let manifest = with(key, value);
            assert!(Manifest::parse(manifest.as_bytes()).is_err(), "{key}: {value}");
        }
        // A parameter declared after the one that names it, and one naming itself.
        let later = r#"{"id":"a","name":"x","kind":"filter","params":{"n":{"type":"number","min":0,"max":1,"default":0,
            "visibleWhen":{"on":[true]}},"on":{"type":"bool"}}}"#;
        let itself = r#"{"id":"a","name":"x","kind":"filter","params":{"on":{"type":"bool","visibleWhen":{"on":[true]}}}}"#;
        for bad in [later, itself] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        let fine = with("help", r#""Two\nlines.""#);
        assert!(Manifest::parse(fine.as_bytes()).is_ok());
    }
}
