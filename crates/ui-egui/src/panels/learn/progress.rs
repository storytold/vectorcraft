//! What a learner has done in the Learn panel: the steps marked done in each tutorial and the
//! tutorials finished. The UI preferences keep it (`UiState::learn_progress`), so it lasts from
//! one start to the next where the preferences are saved. Saved progress is read leniently: an
//! unknown tutorial id, a step the tutorial does not have, a repeated entry or a value of the wrong
//! kind is skipped, and the rest of the preferences read as usual.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::catalog;

/// The most entries read from one saved list or map; the rest are skipped.
const MAX_READ: usize = 1024;

/// The tutorials finished and the steps marked done.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The tutorials finished, in the order they were finished. Each id is listed once.
    pub finished: Vec<&'static str>,
    /// For each tutorial not finished, the zero-based indexes of the steps marked done, ascending.
    /// A tutorial with no step marked has no entry.
    pub steps: BTreeMap<&'static str, Vec<usize>>,
}

impl Progress {
    /// Nothing is finished and no step is marked.
    pub fn is_empty(&self) -> bool {
        self.finished.is_empty() && self.steps.is_empty()
    }

    /// Whether the tutorial `id` is finished.
    pub fn is_finished(&self, id: &str) -> bool {
        self.finished.contains(&id)
    }

    /// The zero-based indexes of `tutorial`'s steps marked done, ascending: every step of a
    /// finished tutorial.
    pub fn completed_steps(&self, tutorial: &catalog::Tutorial) -> Vec<usize> {
        if self.is_finished(tutorial.id) {
            return (0..tutorial.steps.len()).collect();
        }
        self.steps.get(tutorial.id).cloned().unwrap_or_default()
    }

    /// Whether step `step` of `tutorial` is marked done.
    pub fn step_done(&self, tutorial: &catalog::Tutorial, step: usize) -> bool {
        self.is_finished(tutorial.id) || self.steps.get(tutorial.id).is_some_and(|s| s.contains(&step))
    }

    /// Mark step `step` of the unfinished tutorial `id` done (`complete`) or not done.
    pub fn mark(&mut self, id: &'static str, step: usize, complete: bool) {
        let marks = self.steps.entry(id).or_default();
        match (marks.binary_search(&step), complete) {
            (Err(at), true) => marks.insert(at, step),
            (Ok(at), false) => {
                marks.remove(at);
            }
            _ => {}
        }
        if marks.is_empty() {
            self.steps.remove(id);
        }
    }

    /// Mark the tutorial `id` finished, with all its steps done. Finishing it again keeps one entry.
    pub fn finish(&mut self, id: &'static str) {
        if !self.is_finished(id) {
            self.finished.push(id);
        }
        self.steps.remove(id);
    }

    /// How many of `skill`'s tutorials are finished.
    pub fn done(&self, skill: &catalog::Skill) -> usize {
        skill.tutorials.iter().filter(|t| self.is_finished(t.id)).count()
    }

    /// Progress read from a saved value, keeping only what the catalog knows (see the module).
    fn from_value(saved: &Value) -> Self {
        let mut progress = Progress::default();
        let known = |id: &Value| id.as_str().and_then(catalog::tutorial).map(|(_, t)| t);
        if let Some(finished) = saved.get("finished").and_then(Value::as_array) {
            for tutorial in finished.iter().take(MAX_READ).filter_map(known) {
                progress.finish(tutorial.id);
            }
        }
        if let Some(steps) = saved.get("steps").and_then(Value::as_object) {
            for (id, marks) in steps.iter().take(MAX_READ) {
                let Some((_, tutorial)) = catalog::tutorial(id) else { continue };
                if progress.is_finished(tutorial.id) {
                    continue;
                }
                let indexes = marks.as_array().into_iter().flatten().take(MAX_READ).filter_map(Value::as_u64);
                for step in indexes.filter_map(|s| usize::try_from(s).ok()).filter(|s| *s < tutorial.steps.len()) {
                    progress.mark(tutorial.id, step, true);
                }
            }
        }
        progress
    }
}

/// The saved form: `{finished: [id], steps: {id: [index]}}`.
#[derive(Serialize)]
struct Saved<'a> {
    finished: &'a [&'static str],
    steps: &'a BTreeMap<&'static str, Vec<usize>>,
}

impl Serialize for Progress {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Saved { finished: &self.finished, steps: &self.steps }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Progress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Progress::from_value(&Value::deserialize(deserializer)?))
    }
}
