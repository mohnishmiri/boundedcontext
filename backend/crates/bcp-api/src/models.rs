//! `GET /api/models` and server-side model validation. The list is the signed-in user's own Copilot
//! entitlement when GitHub answers; otherwise the operator catalog (see `copilot_models`).

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use bcp_core::models::ModelEntry;
use bcp_core::{validate, AppError, ErrorCode};
use serde_json::{json, Value};
use url::Url;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::AppState;

const LIST_TTL: Duration = Duration::from_secs(120);
const ENTITLEMENT_NOTE: &str = "Availability is enforced by GitHub Copilot for your account at run time. If your plan \
    or organisation policy does not allow the selected model, the job fails with a clear error and no other model \
    is substituted.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    /// Listed by GitHub Copilot for this user's token.
    Copilot,
    /// The operator catalog (live listing disabled or unavailable).
    Catalog,
}

impl ModelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copilot => "copilot",
            Self::Catalog => "catalog",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelList {
    pub items: Vec<ModelEntry>,
    pub source: ModelSource,
    pub warning: Option<String>,
    pub default_model: String,
}

impl ModelList {
    /// Server-side validation of a requested model against this list.
    pub fn validate(&self, id: &str) -> Result<&ModelEntry, AppError> {
        if validate::model_id(id).is_err() {
            return Err(AppError::validation([("model", "Not a valid model identifier")]));
        }
        self.items.iter().find(|m| m.id == id).ok_or_else(|| {
            let whose = match self.source {
                ModelSource::Copilot => "is not available to your GitHub Copilot account",
                ModelSource::Catalog => "is not available in this portal",
            };
            AppError::new(
                ErrorCode::ModelUnavailable,
                format!("Model '{id}' {whose}. Choose one of the listed models."),
            )
        })
    }
}

/// The models `user_id` may choose from. Never fails: a failed live lookup yields the catalog with a warning.
pub async fn model_list(state: &AppState, user_id: Uuid, token: &str) -> Arc<ModelList> {
    let key = format!("models:{user_id}");
    if let Some(hit) = state.cache.get::<ModelList>(&key, LIST_TTL) {
        return hit;
    }
    let catalog = &state.catalog;
    let catalog_items: Vec<ModelEntry> = catalog.visible().cloned().collect();
    let catalog_list = |warning: Option<String>| ModelList {
        items: catalog_items.clone(),
        source: ModelSource::Catalog,
        warning,
        default_model: catalog.default_model().to_owned(),
    };
    let list = match &state.copilot_models {
        None => catalog_list(None),
        Some(client) => {
            let account_base = match state.github.get_copilot_user(token).await {
                Ok(u) => crate::copilot_models::account_api_base(&u),
                Err(_) => None,
            };
            match client.list(token, account_base.as_ref()).await {
            Ok(live) => {
                let mut items: Vec<ModelEntry> = Vec::with_capacity(live.len() + 1);
                // `auto` (Copilot picks) is a Copilot CLI feature, not a listed model.
                if let Some(auto) = catalog.get("auto") {
                    items.push(auto.clone());
                }
                items.extend(live.into_iter().filter(|m| m.id != "auto"));
                let default_model = if items.iter().any(|m| m.id == catalog.default_model()) {
                    catalog.default_model().to_owned()
                } else {
                    items.first().map(|m| m.id.clone()).unwrap_or_else(|| "auto".to_owned())
                };
                ModelList { items, source: ModelSource::Copilot, warning: None, default_model }
            }
            Err(e) => {
                tracing::warn!(%user_id, error = %e, endpoint = ?account_base.as_ref().map(Url::as_str), "live Copilot model list unavailable; using the catalog");
                metrics::counter!("bcp_model_list_fallback_total").increment(1);
                catalog_list(Some(
                    "Could not load the models available to your GitHub Copilot account, so this is the portal's \
                     standard list. Availability is still checked by GitHub Copilot when the job runs."
                        .to_owned(),
                ))
            }
            }
        }
    };
    let list = Arc::new(list);
    state.cache.put(key, list.clone());
    list
}

/// `GET /api/models`
pub async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    // The account that runs Copilot decides which models are available.
    let token = state.tokens.copilot_run_token(user.user.id).await?;
    let list = model_list(&state, user.user.id, &token).await;
    let items: Vec<Value> = list
        .items
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "name": m.name, "vendor": m.vendor, "description": m.description,
                "multiplier": m.multiplier, "is_default": m.id == list.default_model,
            })
        })
        .collect();
    Ok(Json(json!({
        "items": items,
        "default_model": list.default_model,
        "source": list.source.as_str(),
        "warning": list.warning,
        "entitlement_note": ENTITLEMENT_NOTE,
    })))
}
