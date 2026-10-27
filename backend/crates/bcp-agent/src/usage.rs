//! Token and premium-request usage written by Copilot CLI's `--usage-output-file`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// File name inside the run's scratch directory.
pub const USAGE_FILE: &str = "usage.json";
const MAX_USAGE_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenCounts {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
}

impl TokenCounts {
    fn add(&mut self, o: &TokenCounts) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
        self.cache_write_tokens += o.cache_write_tokens;
        self.reasoning_tokens += o.reasoning_tokens;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model: String,
    pub requests: u64,
    pub premium_requests: f64,
    #[serde(default)]
    pub ai_credits: f64,
    #[serde(flatten)]
    pub tokens: TokenCounts,
}

/// Usage of one job, summed over its agent runs (attempts).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageSummary {
    pub runs: u32,
    pub premium_requests: f64,
    /// Estimated from the CLI's nano-AIU totals (1 AI credit = 10^9 nano-AIU).
    #[serde(default)]
    pub ai_credits: f64,
    /// Billed to the worker's configured Copilot token rather than the requesting user.
    #[serde(default)]
    pub shared_token: bool,
    pub api_duration_ms: u64,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub models: Vec<ModelUsage>,
}

const NANO_AIU_PER_CREDIT: f64 = 1e9;

fn num(v: &Value, path: &[&str]) -> f64 {
    path.iter().try_fold(v, |acc, k| acc.get(k)).and_then(Value::as_f64).unwrap_or(0.0)
}

fn count(v: &Value, path: &[&str]) -> u64 {
    num(v, path).max(0.0) as u64
}

impl UsageSummary {
    /// Parses the CLI's usage JSON; `None` when it has no per-model metrics.
    pub fn from_cli_json(v: &Value) -> Option<Self> {
        let metrics = v.get("modelMetrics")?.as_object()?;
        let mut s = UsageSummary { runs: 1, api_duration_ms: count(v, &["totalApiDurationMs"]), ..Default::default() };
        for (model, m) in metrics {
            let usage = ModelUsage {
                model: model.clone(),
                requests: count(m, &["requests", "count"]),
                premium_requests: num(m, &["requests", "cost"]),
                ai_credits: num(m, &["totalNanoAiu"]) / NANO_AIU_PER_CREDIT,
                tokens: TokenCounts {
                    input_tokens: count(m, &["usage", "inputTokens"]),
                    output_tokens: count(m, &["usage", "outputTokens"]),
                    cache_read_tokens: count(m, &["usage", "cacheReadTokens"]),
                    cache_write_tokens: count(m, &["usage", "cacheWriteTokens"]),
                    reasoning_tokens: count(m, &["usage", "reasoningTokens"]),
                },
            };
            s.tokens.add(&usage.tokens);
            s.premium_requests += usage.premium_requests;
            s.ai_credits += usage.ai_credits;
            s.models.push(usage);
        }
        if let Some(total) = v.get("totalPremiumRequestCost").and_then(Value::as_f64) {
            s.premium_requests = total;
        }
        if let Some(total) = v.get("totalNanoAiu").and_then(Value::as_f64) {
            s.ai_credits = total / NANO_AIU_PER_CREDIT;
        }
        Some(s)
    }

    /// Reads `USAGE_FILE` from a run's scratch directory, if the CLI wrote one.
    pub fn read(scratch_dir: &Path) -> Option<Self> {
        let path = scratch_dir.join(USAGE_FILE);
        if std::fs::metadata(&path).ok()?.len() > MAX_USAGE_FILE_BYTES {
            return None;
        }
        let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        Self::from_cli_json(&v)
    }

    pub fn merge(&mut self, other: &UsageSummary) {
        self.runs += other.runs;
        self.premium_requests += other.premium_requests;
        self.ai_credits += other.ai_credits;
        self.shared_token |= other.shared_token;
        self.api_duration_ms += other.api_duration_ms;
        self.tokens.add(&other.tokens);
        for m in &other.models {
            match self.models.iter_mut().find(|x| x.model == m.model) {
                Some(x) => {
                    x.requests += m.requests;
                    x.premium_requests += m.premium_requests;
                    x.ai_credits += m.ai_credits;
                    x.tokens.add(&m.tokens);
                }
                None => self.models.push(m.clone()),
            }
        }
    }

    /// One log line for the job log.
    pub fn describe(&self) -> String {
        let models: Vec<String> =
            self.models.iter().map(|m| format!("{} ({} requests)", m.model, m.requests)).collect();
        format!(
            "Copilot usage: {:.2} AI credits (est.), {} premium request(s); {} input tokens ({} cache read, {} cache write), {} output tokens, {} reasoning tokens; models: {}",
            self.ai_credits,
            self.premium_requests,
            self.tokens.input_tokens,
            self.tokens.cache_read_tokens,
            self.tokens.cache_write_tokens,
            self.tokens.output_tokens,
            self.tokens.reasoning_tokens,
            if models.is_empty() { "none".to_owned() } else { models.join(", ") },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "totalPremiumRequestCost": 1,
            "totalNanoAiu": 155119500,
            "totalApiDurationMs": 4002,
            "modelMetrics": {
                "gpt-6-luna": {
                    "requests": { "count": 1, "cost": 1 },
                    "usage": { "inputTokens": 13769, "outputTokens": 5, "cacheReadTokens": 0,
                               "cacheWriteTokens": 13766, "reasoningTokens": 0 }
                }
            }
        })
    }

    #[test]
    fn parses_cli_usage() {
        let s = UsageSummary::from_cli_json(&sample()).unwrap();
        assert_eq!(s.runs, 1);
        assert_eq!(s.premium_requests, 1.0);
        assert_eq!(s.api_duration_ms, 4002);
        assert!((s.ai_credits - 0.1551195).abs() < 1e-9);
        assert_eq!(s.tokens.input_tokens, 13769);
        assert_eq!(s.tokens.cache_write_tokens, 13766);
        assert_eq!(s.models[0].model, "gpt-6-luna");
        assert!(UsageSummary::from_cli_json(&json!({})).is_none());
    }

    #[test]
    fn merges_attempts_per_model() {
        let one = UsageSummary::from_cli_json(&sample()).unwrap();
        let mut total = UsageSummary::default();
        total.merge(&one);
        total.merge(&one);
        assert_eq!(total.runs, 2);
        assert_eq!(total.premium_requests, 2.0);
        assert_eq!(total.tokens.output_tokens, 10);
        assert_eq!(total.models.len(), 1);
        assert_eq!(total.models[0].requests, 2);
    }

    #[test]
    fn reads_file_from_scratch_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(UsageSummary::read(dir.path()).is_none());
        std::fs::write(dir.path().join(USAGE_FILE), sample().to_string()).unwrap();
        assert_eq!(UsageSummary::read(dir.path()).unwrap().tokens.input_tokens, 13769);
    }
}
