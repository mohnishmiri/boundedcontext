//! Per-stage timing records stored as JSON on the job row (`stage_times`, `publish_stage_times`) and
//! rendered as the API's `steps`. Real transitions only — no estimated percentages.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::StepState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    pub state: StepState,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
}

/// One step as served by the API.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub key: String,
    pub label: String,
    pub state: StepState,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StageTracker {
    times: BTreeMap<String, StepRecord>,
    current: Option<String>,
}

impl StageTracker {
    /// Resumes from stored JSON (e.g. a retried task). The active stage, if any, becomes current.
    pub fn from_json(value: &Value) -> Self {
        let times: BTreeMap<String, StepRecord> = serde_json::from_value(value.clone()).unwrap_or_default();
        let current = times.iter().find(|(_, r)| r.state == StepState::Active).map(|(k, _)| k.clone());
        Self { times, current }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(&self.times).unwrap_or(Value::Null)
    }

    /// Finishes the current stage (if any) and starts `key`.
    pub fn start(&mut self, key: &str) {
        let now = Utc::now();
        self.finish_current(StepState::Done, now);
        let rec = self.times.entry(key.to_owned()).or_insert(StepRecord {
            state: StepState::Active,
            started_at: None,
            finished_at: None,
        });
        rec.state = StepState::Active;
        rec.started_at = Some(now);
        rec.finished_at = None;
        self.current = Some(key.to_owned());
    }

    /// Marks a stage done instantly (e.g. `queued` when a worker picks the job up).
    pub fn mark_done(&mut self, key: &str, started_at: DateTime<Utc>) {
        self.times.insert(
            key.to_owned(),
            StepRecord { state: StepState::Done, started_at: Some(started_at), finished_at: Some(Utc::now()) },
        );
    }

    pub fn complete(&mut self) {
        self.finish_current(StepState::Done, Utc::now());
    }

    pub fn fail(&mut self) {
        self.finish_current(StepState::Failed, Utc::now());
    }

    /// Clears the active marker without a verdict (job returned to the queue).
    pub fn suspend(&mut self) {
        if let Some(key) = self.current.take() {
            self.times.remove(&key);
        }
    }

    fn finish_current(&mut self, state: StepState, now: DateTime<Utc>) {
        if let Some(key) = self.current.take() {
            if let Some(rec) = self.times.get_mut(&key) {
                rec.state = state;
                rec.finished_at = Some(now);
            }
        }
    }
}

/// Renders the ordered steps. Stages never reached are `pending`, or `skipped` once the run has ended.
pub fn render_steps(order: &[(&str, &str)], stored: &Value, ended: bool) -> Vec<Step> {
    let times: BTreeMap<String, StepRecord> = serde_json::from_value(stored.clone()).unwrap_or_default();
    order
        .iter()
        .map(|(key, label)| match times.get(*key) {
            Some(r) => Step {
                key: (*key).to_owned(),
                label: (*label).to_owned(),
                // A stage still marked active on an ended run was interrupted.
                state: if ended && r.state == StepState::Active { StepState::Failed } else { r.state },
                started_at: r.started_at,
                finished_at: r.finished_at,
            },
            None => Step {
                key: (*key).to_owned(),
                label: (*label).to_owned(),
                state: if ended { StepState::Skipped } else { StepState::Pending },
                started_at: None,
                finished_at: None,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: &[(&str, &str)] = &[("a", "A"), ("b", "B"), ("c", "C")];

    #[test]
    fn tracks_transitions() {
        let mut t = StageTracker::default();
        t.start("a");
        t.start("b");
        let steps = render_steps(ORDER, &t.to_json(), false);
        assert_eq!(
            steps.iter().map(|s| s.state).collect::<Vec<_>>(),
            [StepState::Done, StepState::Active, StepState::Pending]
        );
        t.fail();
        let steps = render_steps(ORDER, &t.to_json(), true);
        assert_eq!(
            steps.iter().map(|s| s.state).collect::<Vec<_>>(),
            [StepState::Done, StepState::Failed, StepState::Skipped]
        );
    }

    #[test]
    fn resumes_and_suspends() {
        let mut t = StageTracker::default();
        t.start("a");
        let mut resumed = StageTracker::from_json(&t.to_json());
        resumed.suspend();
        assert_eq!(render_steps(ORDER, &resumed.to_json(), false)[0].state, StepState::Pending);
        let mut t = StageTracker::from_json(&serde_json::json!({}));
        t.start("c");
        t.complete();
        assert_eq!(render_steps(ORDER, &t.to_json(), true)[2].state, StepState::Done);
    }
}
