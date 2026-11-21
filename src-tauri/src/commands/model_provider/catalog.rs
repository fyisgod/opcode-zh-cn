use std::{collections::HashSet, time::Duration};

use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;

use super::{
    config_path, load, normalized_url, unprotect_credential, validate_credential, Auth, Protocol,
    ProviderModel, ReasoningEffort,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRequest {
    id: String,
    base_url: String,
    auth: Auth,
    protocol: Protocol,
    credential: Option<String>,
}

impl CatalogRequest {
    fn credential(&self, app: &AppHandle, base_url: &str) -> Result<String, String> {
        if let Some(credential) = self
            .credential
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            validate_credential(credential)?;
            return Ok(credential.to_owned());
        }
        // Reuse the stored secret only for the exact same saved connection.
        let saved = load(&config_path(app)?)?
            .providers
            .into_iter()
            .find(|saved| {
                let connection = &saved.provider.connection;
                saved.provider.id == self.id
                    && !saved.credential.is_empty()
                    && connection.base_url == base_url
                    && connection.auth == self.auth
                    && connection.protocol == self.protocol
            });
        let saved =
            saved.ok_or("Enter an API credential for this endpoint and authentication method.")?;
        let credential = unprotect_credential(&saved.credential)?;
        validate_credential(&credential)?;
        Ok(credential)
    }
}

#[tauri::command]
pub async fn fetch_provider_models(
    app: AppHandle,
    request: CatalogRequest,
) -> Result<Vec<ProviderModel>, String> {
    let base_url = normalized_url(request.base_url.trim())?;
    let endpoint = catalog_endpoint(request.protocol, &base_url)?;
    let credential = request.credential(&app, &base_url)?;
    tokio::time::timeout(
        Duration::from_secs(25),
        fetch(endpoint, request.protocol, request.auth, &credential),
    )
    .await
    .map_err(|_| "Fetching the model list timed out.")?
}

fn catalog_endpoint(protocol: Protocol, base_url: &str) -> Result<Url, String> {
    let mut url = Url::parse(base_url).map_err(|_| "Enter a valid base URL.")?;
    let path = url.path().trim_end_matches('/');
    if path.ends_with("/models") {
        return Ok(url);
    }
    if protocol == Protocol::Anthropic {
        let api_path = if path.ends_with("/v1") {
            path.to_owned()
        } else {
            format!("{path}/v1")
        };
        url.set_path(&format!("{api_path}/models"));
    } else {
        url = super::super::provider_bridge::endpoint(protocol, base_url)?;
        let resource = if protocol == Protocol::OpenAiResponses {
            "/responses"
        } else {
            "/chat/completions"
        };
        let api_path = url
            .path()
            .strip_suffix(resource)
            .ok_or("The API endpoint does not match the selected protocol.")?;
        url.set_path(&format!("{api_path}/models"));
    }
    Ok(url)
}

#[derive(Deserialize)]
struct CatalogPage {
    data: Vec<Value>,
    #[serde(default)]
    has_more: bool,
    last_id: Option<String>,
}

fn parse_model(entry: Value) -> Option<ProviderModel> {
    let id = entry["id"].as_str()?.trim();
    if !ProviderModel::valid_id(id) {
        return None;
    }
    let mut model = ProviderModel::from_id(id.to_owned());
    for field in ["display_name", "name"] {
        if let Some(name) = entry[field]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
        {
            model.display_name = name.to_owned();
            break;
        }
    }
    model.context_length = ["context_length", "max_input_tokens", "context_window"]
        .into_iter()
        .find_map(|field| {
            let length = u32::try_from(entry[field].as_u64()?).ok()?;
            (length > 0).then_some(length)
        });
    model.reasoning_efforts = [
        ("low", ReasoningEffort::Low),
        ("medium", ReasoningEffort::Medium),
        ("high", ReasoningEffort::High),
        ("xhigh", ReasoningEffort::Xhigh),
        ("max", ReasoningEffort::Max),
    ]
    .into_iter()
    .filter_map(|(name, effort)| {
        let capability = &entry["capabilities"]["effort"][name];
        (capability["supported"].as_bool() == Some(true)).then_some(effort)
    })
    .collect();
    Some(model)
}

async fn fetch(
    endpoint: Url,
    protocol: Protocol,
    auth: Auth,
    credential: &str,
) -> Result<Vec<ProviderModel>, String> {
    const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
    const FETCH_ERROR: &str = "Could not connect to the model list endpoint.";
    const RESPONSE_ERROR: &str = "The provider returned an invalid model list.";
    let mut client = Client::builder().redirect(reqwest::redirect::Policy::none());
    if matches!(
        endpoint.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]")
    ) {
        client = client.no_proxy();
    }
    let client = client.build().map_err(|_| FETCH_ERROR)?;
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    let mut cursors = HashSet::new();
    let mut next = endpoint.clone();
    for _ in 0..25 {
        let request = client.get(next).header("accept", "application/json");
        let request = match auth {
            Auth::Bearer => request.bearer_auth(credential),
            Auth::ApiKey => request.header("x-api-key", credential),
        };
        let request = if protocol == Protocol::Anthropic {
            request.header("anthropic-version", "2023-06-01")
        } else {
            request
        };
        let mut response = request.send().await.map_err(|_| FETCH_ERROR)?;
        match response.status() {
            status if status.is_success() => (),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err("The provider rejected the API credential.".into())
            }
            StatusCode::NOT_FOUND
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::NOT_IMPLEMENTED => {
                return Err("The provider does not expose a model list at this address.".into())
            }
            _ => return Err("The model list request failed.".into()),
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| FETCH_ERROR)? {
            if body.len() + chunk.len() > MAX_PAGE_BYTES {
                return Err("The provider model list is too large.".into());
            }
            body.extend_from_slice(&chunk);
        }
        let page: CatalogPage = serde_json::from_slice(&body).map_err(|_| RESPONSE_ERROR)?;
        models.extend(
            page.data
                .into_iter()
                .filter_map(parse_model)
                .filter(|model| seen.insert(model.id.clone())),
        );
        if protocol != Protocol::Anthropic || !page.has_more {
            models.sort_unstable_by(|a, b| a.id.cmp(&b.id));
            return Ok(models);
        }
        let cursor = page
            .last_id
            .filter(|id| !id.is_empty() && cursors.insert(id.clone()))
            .ok_or(RESPONSE_ERROR)?;
        next = endpoint.clone();
        next.query_pairs_mut().append_pair("after_id", &cursor);
    }
    Err("The provider model list is too large.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derives_model_list_endpoint_from_configured_protocol() {
        for (protocol, base, path) in [
            (Protocol::Anthropic, "https://example.com", "/v1/models"),
            (
                Protocol::Anthropic,
                "https://example.com/proxy/v1",
                "/proxy/v1/models",
            ),
            (
                Protocol::Anthropic,
                "https://example.com/api/anthropic",
                "/api/anthropic/v1/models",
            ),
            (Protocol::OpenAiChat, "https://example.com", "/v1/models"),
            (
                Protocol::OpenAiChat,
                "https://example.com/custom/v4/chat/completions",
                "/custom/v4/models",
            ),
            (
                Protocol::OpenAiResponses,
                "https://example.com/v1/responses",
                "/v1/models",
            ),
            (
                Protocol::Gemini,
                "https://example.com/v1beta/openai",
                "/v1beta/openai/models",
            ),
        ] {
            assert_eq!(catalog_endpoint(protocol, base).unwrap().path(), path);
        }
        assert!(
            catalog_endpoint(Protocol::OpenAiChat, "https://example.com/v1/responses").is_err()
        );
    }

    #[tokio::test]
    async fn reads_paginated_catalog_and_uses_configured_authentication() {
        use axum::{extract::Query, http::HeaderMap, routing::get, Json, Router};
        use std::collections::HashMap;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/v1/models",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let router = Router::new().route("/v1/models", get(|headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
            assert_eq!(headers["anthropic-version"], "2023-06-01");
            assert_eq!(headers["x-api-key"], "dummy-key");
            assert!(!headers.contains_key("authorization"));
            if query.is_empty() {
                Json(json!({"data": [{"id": "z", "display_name": "Zeta", "max_input_tokens": 200000,
                    "capabilities": {"effort": {"high": {"supported": true}, "max": {"supported": false}}}}],
                    "has_more": true, "last_id": "z"}))
            } else {
                assert_eq!(query["after_id"], "z");
                Json(json!({"data": [{"id": "a"}, {"id": "z"}, {"id": "invalid id"}], "has_more": false}))
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let models = fetch(url, Protocol::Anthropic, Auth::ApiKey, "dummy-key")
            .await
            .unwrap();
        server.abort();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0], ProviderModel::from_id("a".into()));
        assert_eq!(models[1].display_name, "Zeta");
        assert_eq!(models[1].context_length, Some(200_000));
        assert_eq!(models[1].reasoning_efforts, [ReasoningEffort::High]);
    }
}
