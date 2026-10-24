//! The model list a user may choose from, taken from their own GitHub Copilot entitlement.
//!
//! GitHub does not document a model-listing endpoint for Copilot CLI; the Copilot API's `GET /models` is what
//! editors use and returns exactly what the calling token's account may use (plan, organisation policy). It is
//! therefore best-effort: any failure falls back to the operator catalog (`config/models.yaml`) and the API
//! response says so (`source: "catalog"` plus a `warning`), so the portal never presents a guess as an
//! entitlement.

use std::time::Duration;

use bcp_core::models::ModelEntry;
use bcp_core::validate;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_MODELS: usize = 200;

#[derive(Debug, Clone)]
pub struct CopilotModelsClient {
    http: reqwest::Client,
    url: Url,
    integration_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelListError {
    #[error("the Copilot API rejected the token ({0})")]
    Rejected(u16),
    #[error("the Copilot API answered with HTTP {0}")]
    Status(u16),
    #[error("could not reach the Copilot API: {0}")]
    Network(String),
    #[error("unexpected Copilot API response: {0}")]
    Decode(String),
    #[error("the Copilot API listed no usable chat models")]
    Empty,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    data: Vec<RawModel>,
}

#[derive(Debug, Deserialize)]
struct RawModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    model_picker_enabled: Option<bool>,
    #[serde(default)]
    capabilities: Option<RawCapabilities>,
    #[serde(default)]
    policy: Option<RawPolicy>,
    #[serde(default)]
    billing: Option<RawBilling>,
}

#[derive(Debug, Deserialize)]
struct RawCapabilities {
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawPolicy {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawBilling {
    #[serde(default)]
    multiplier: Option<f64>,
}

impl CopilotModelsClient {
    pub fn new(base: &Url, integration_id: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build()?;
        let mut url = base.clone();
        url.path_segments_mut()
            .map_err(|()| anyhow::anyhow!("Copilot API URL cannot be a base"))?
            .pop_if_empty()
            .push("models");
        Ok(Self { http, url, integration_id: integration_id.to_owned() })
    }

    /// Chat models the token's account can use, in the order Copilot lists them.
    pub async fn list(&self, token: &str) -> Result<Vec<ModelEntry>, ModelListError> {
        let resp = self
            .http
            .get(self.url.clone())
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, "bounded-context-portal")
            .header("Copilot-Integration-Id", &self.integration_id)
            .send()
            .await
            .map_err(|e| ModelListError::Network(e.without_url().to_string()))?;
        let status = resp.status().as_u16();
        match status {
            200 => {}
            401 | 403 => return Err(ModelListError::Rejected(status)),
            other => return Err(ModelListError::Status(other)),
        }
        let body = resp.bytes().await.map_err(|e| ModelListError::Network(e.without_url().to_string()))?;
        parse_models(&body)
    }
}

/// Accepts `{ "data": [...] }` (what the API returns) or a bare array.
pub fn parse_models(body: &[u8]) -> Result<Vec<ModelEntry>, ModelListError> {
    let raw: Vec<RawModel> = match serde_json::from_slice::<Envelope>(body) {
        Ok(env) => env.data,
        Err(_) => serde_json::from_slice(body).map_err(|e| ModelListError::Decode(e.to_string()))?,
    };
    let mut out: Vec<ModelEntry> = Vec::new();
    for m in raw {
        let is_chat = m.capabilities.as_ref().and_then(|c| c.kind.as_deref()).is_none_or(|k| k == "chat");
        let picker = m.model_picker_enabled.unwrap_or(true);
        let policy_ok = m.policy.as_ref().and_then(|p| p.state.as_deref()).is_none_or(|s| s != "disabled");
        if !(is_chat && picker && policy_ok) || validate::model_id(&m.id).is_err() || out.iter().any(|e| e.id == m.id) {
            continue;
        }
        out.push(ModelEntry {
            name: m.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| m.id.clone()),
            vendor: m.vendor.filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "GitHub Copilot".to_owned()),
            id: m.id,
            description: None,
            multiplier: m.billing.and_then(|b| b.multiplier),
            verified: true,
        });
        if out.len() >= MAX_MODELS {
            break;
        }
    }
    if out.is_empty() {
        return Err(ModelListError::Empty);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_enabled_chat_models_only() {
        let body = br#"{ "data": [
          { "id": "gpt-5", "name": "GPT-5", "vendor": "OpenAI", "model_picker_enabled": true,
            "capabilities": { "type": "chat" }, "policy": { "state": "enabled" }, "billing": { "multiplier": 1.0 } },
          { "id": "text-embedding-3-small", "name": "Embedding", "capabilities": { "type": "embeddings" } },
          { "id": "claude-sonnet-4.5", "name": "Claude Sonnet 4.5", "vendor": "Anthropic",
            "capabilities": { "type": "chat" }, "policy": { "state": "unconfigured" } },
          { "id": "old-model", "capabilities": { "type": "chat" }, "policy": { "state": "disabled" } },
          { "id": "hidden", "model_picker_enabled": false, "capabilities": { "type": "chat" } },
          { "id": "Bad Id;rm", "capabilities": { "type": "chat" } },
          { "id": "gpt-5", "name": "duplicate", "capabilities": { "type": "chat" } }
        ] }"#;
        let models = parse_models(body).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gpt-5", "claude-sonnet-4.5"]);
        assert_eq!(models[0].multiplier, Some(1.0));
        assert_eq!(models[1].vendor, "Anthropic");
    }

    #[test]
    fn empty_or_malformed_responses_are_errors() {
        assert!(matches!(parse_models(br#"{"data": []}"#), Err(ModelListError::Empty)));
        assert!(matches!(parse_models(b"<html>"), Err(ModelListError::Decode(_))));
        assert_eq!(parse_models(br#"[{"id": "gpt-5"}]"#).unwrap().len(), 1);
    }
}
