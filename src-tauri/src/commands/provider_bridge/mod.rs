mod request;
mod stream;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::{Client, Url};
use serde_json::{json, Value};

use super::model_provider::{Auth, Protocol};

struct Gateway {
    client: Client,
    endpoint: Url,
    credential: String,
    auth: Auth,
    model: String,
    token: String,
    protocol: Protocol,
    tool_context: ToolContext,
}

// Opaque replay metadata stays in memory for a single CLI tool loop.
#[derive(Default)]
struct ToolContext(Mutex<HashMap<String, Value>>);

impl ToolContext {
    fn remember(&self, id: &str, value: Value) {
        if value.is_null() {
            return;
        }
        if let Ok(mut context) = self.0.lock() {
            context.insert(id.to_owned(), value);
        }
    }

    fn get(&self, id: &str) -> Option<Value> {
        self.0.lock().ok()?.get(id).cloned()
    }
}

pub(super) struct ProviderBridge {
    pub base_url: String,
    pub token: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ProviderBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ProviderBridge {
    pub async fn start(
        protocol: Protocol,
        base_url: &str,
        model: &str,
        auth: Auth,
        credential: &str,
    ) -> Result<Self, String> {
        let endpoint = endpoint(protocol, base_url)?;
        let mut client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(180));
        if matches!(
            endpoint.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]")
        ) {
            client = client.no_proxy();
        }
        let client = client
            .build()
            .map_err(|_| "Could not initialize the provider HTTP client")?;
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "Could not start the local provider adapter")?;
        let address = listener
            .local_addr()
            .map_err(|_| "Could not read the local provider address")?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let state = Arc::new(Gateway {
            client,
            endpoint,
            credential: credential.into(),
            auth,
            model: model.into(),
            token: token.clone(),
            protocol,
            tool_context: ToolContext::default(),
        });
        let router = Router::new()
            .route("/v1/messages", post(messages))
            .route("/v1/messages/count_tokens", post(count_tokens))
            .route("/v1/models", get(models))
            .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
            .with_state(state);
        let task = tokio::spawn(async move {
            if axum::serve(listener, router).await.is_err() {
                log::warn!("Local provider adapter stopped unexpectedly");
            }
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            token,
            task,
        })
    }
}

pub(super) fn endpoint(protocol: Protocol, base: &str) -> Result<Url, String> {
    let mut url = Url::parse(base).map_err(|_| "Enter a valid base URL.")?;
    let resource = match protocol {
        Protocol::OpenAiChat | Protocol::Gemini => "/chat/completions",
        Protocol::OpenAiResponses => "/responses",
        Protocol::Anthropic => {
            return Err("Anthropic Messages does not require a local adapter".into())
        }
    };
    let path = url.path().trim_end_matches('/');
    if path.ends_with(resource) {
        return Ok(url);
    }
    if path.ends_with("/chat/completions") || path.ends_with("/responses") {
        return Err("The API endpoint does not match the selected protocol.".into());
    }
    let path = if path.is_empty() {
        if protocol == Protocol::Gemini {
            "/v1beta/openai"
        } else {
            "/v1"
        }
    } else {
        path
    };
    url.set_path(&format!("{path}{resource}"));
    Ok(url)
}

pub(super) fn validate_endpoint(protocol: Protocol, base: &str) -> Result<(), String> {
    if protocol == Protocol::Anthropic {
        return Ok(());
    }
    endpoint(protocol, base).map(|_| ())
}

fn authorized(state: &Gateway, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == state.token)
}

fn api_error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        Json(json!({"type":"error","error":{"type":kind,"message":message}})),
    )
        .into_response()
}

fn unauthorized() -> Response {
    api_error(
        StatusCode::UNAUTHORIZED,
        "authentication_error",
        "Invalid local adapter token",
    )
}

fn estimate_tokens(body: &Value) -> u64 {
    let bytes = ["system", "messages", "tools"]
        .iter()
        .map(|key| body[*key].to_string().len())
        .sum::<usize>();
    bytes.div_ceil(4) as u64
}

async fn count_tokens(
    State(state): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    Json(json!({"input_tokens":estimate_tokens(&body)})).into_response()
}

async fn models(State(state): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    Json(json!({"data":[{"type":"model","id":state.model,"display_name":state.model,"created_at":"1970-01-01T00:00:00Z"}],"first_id":state.model,"last_id":state.model,"has_more":false})).into_response()
}

async fn messages(
    State(state): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    // Claude Code also makes non-streaming requests, e.g. for session titles.
    let streaming = body["stream"] == true;
    let official_openai = state.endpoint.host_str() == Some("api.openai.com");
    let mut request = match request::translate(
        &body,
        state.protocol,
        &state.model,
        official_openai,
        &state.tool_context,
    ) {
        Ok(request) => request,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, "invalid_request_error", error),
    };
    request["stream"] = json!(streaming);
    if streaming && official_openai && state.protocol == Protocol::OpenAiChat {
        request["stream_options"] = json!({"include_usage":true});
    }
    let request = state.client.post(state.endpoint.clone()).json(&request);
    let request = match state.auth {
        Auth::Bearer => request.bearer_auth(&state.credential),
        Auth::ApiKey => request.header("x-api-key", &state.credential),
    };
    let upstream = match request.send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            // Never forward raw provider error bodies: they can echo request headers or prompts.
            let status = response.status();
            let (code, kind, detail) = match status.as_u16() {
                400 | 422 => (
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    "Check the model ID and its input and tool capabilities.",
                ),
                401 | 403 => (
                    status,
                    "authentication_error",
                    "Check the credential and authentication method.",
                ),
                404 => (
                    status,
                    "not_found_error",
                    "Check the endpoint, protocol and model ID.",
                ),
                429 => (
                    status,
                    "rate_limit_error",
                    "Check the provider quota or retry later.",
                ),
                _ => (
                    StatusCode::BAD_GATEWAY,
                    "api_error",
                    "Check the provider endpoint or service status.",
                ),
            };
            return api_error(
                code,
                kind,
                &format!("Model provider returned HTTP {}. {detail}", status.as_u16()),
            );
        }
        Err(error) => {
            let message = if error.is_timeout() {
                "The model provider timed out while connecting"
            } else {
                "Could not connect to the model provider because of a network error"
            };
            return api_error(StatusCode::BAD_GATEWAY, "api_error", message);
        }
    };
    let count = estimate_tokens(&body);
    if !streaming {
        return match stream::json_message(
            upstream,
            state.protocol,
            &state.model,
            count,
            &state.tool_context,
        )
        .await
        {
            Ok(message) => Json(message).into_response(),
            Err(error) => api_error(StatusCode::BAD_GATEWAY, "api_error", error),
        };
    }
    let sse = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let (sender, receiver) = tokio::sync::mpsc::channel::<String>(8);
    tokio::spawn(async move {
        if let Err(message) = stream::forward(
            upstream,
            state.protocol,
            state.model.clone(),
            count,
            sse,
            &sender,
            &state.tool_context,
        )
        .await
        {
            let _ = sender
                .send(stream::frame(
                    "error",
                    json!({"type":"error","error":{"type":"api_error","message":message}}),
                ))
                .await;
        }
    });
    let frames = futures::stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|data| (Ok::<_, std::convert::Infallible>(data), receiver))
    });
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        Body::from_stream(frames),
    )
        .into_response()
}
