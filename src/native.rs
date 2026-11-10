use crate::{
    MAX_BODY, MAX_RESPONSE,
    api::{ApiRequest, ApiResponse},
    protocol::{self, RequestContext},
    security,
};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub async fn execute(request: ApiRequest) -> Result<ApiResponse, &'static str> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Upstream transport failed")?;
    let method = request
        .method
        .parse()
        .map_err(|_| "Upstream transport failed")?;
    let mut builder = client.request(method, request.url);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = request.body {
        builder = builder.json(&body);
    }
    tokio::time::timeout(Duration::from_secs(30), async {
        let response = builder
            .send()
            .await
            .map_err(|_| "Upstream transport failed")?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err("Upstream redirect rejected");
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Upstream transport failed")?;
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err("Upstream response too large");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(ApiResponse {
            status,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    })
    .await
    .map_err(|_| "Upstream request timed out")?
}

struct Session {
    key: String,
    expires: Instant,
    sender: mpsc::Sender<Value>,
}
#[derive(Clone)]
struct HttpState {
    authority: String,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}
const SESSION_TTL: Duration = Duration::from_secs(300);

fn check(headers: &HeaderMap, state: &HttpState) -> Result<Option<String>, StatusCode> {
    if headers.get("origin").is_some_and(|v| v.to_str().is_err())
        || headers
            .get("mcp-protocol-version")
            .is_some_and(|v| v.to_str().is_err())
        || headers.get("host").and_then(|v| v.to_str().ok()) != Some(state.authority.as_str())
        || !security::origin_allowed(
            headers.get("origin").and_then(|v| v.to_str().ok()),
            &state.authority,
            "http",
        )
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let key = headers
        .get("x-herosms-api-key")
        .map(|v| v.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    if key.as_deref().is_some_and(|k| !crate::api::valid_key(k)) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(key)
}

fn parse(body: &[u8]) -> Result<Value, Value> {
    serde_json::from_slice(body).map_err(|_| protocol::error(Value::Null, -32700, "Parse error"))
}
fn content_type(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"))
}
async fn modern(State(state): State<HttpState>, headers: HeaderMap, body: Bytes) -> Response {
    let key = match check(&headers, &state) {
        Ok(key) => key,
        Err(status) => return status.into_response(),
    };
    if !content_type(&headers) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if !security::modern_version_allowed(
        headers
            .get("mcp-protocol-version")
            .and_then(|v| v.to_str().ok()),
    ) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let response = match parse(&body) {
        Ok(request) => {
            protocol::dispatch(request, &RequestContext::new(key).modern_transport()).await
        }
        Err(error) => Some(error),
    };
    match response {
        Some(response) => axum::Json(response).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn sse(State(state): State<HttpState>, headers: HeaderMap) -> Response {
    let key = match check(&headers, &state) {
        Ok(Some(key)) => key,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(s) => return s.into_response(),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::channel(16);
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.retain(|_, s| s.expires > Instant::now() && !s.sender.is_closed());
        if sessions.len() >= 128 {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        sessions.insert(
            id.clone(),
            Session {
                key,
                expires: Instant::now() + SESSION_TTL,
                sender,
            },
        );
    }
    let cleanup = state.clone();
    let cleanup_id = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(SESSION_TTL).await;
        cleanup.sessions.lock().unwrap().remove(&cleanup_id);
    });
    let endpoint = stream::once(async move {
        Ok::<_, Infallible>(
            Event::default()
                .event("endpoint")
                .data(format!("/messages?sessionId={id}")),
        )
    });
    let messages = tokio_stream::wrappers::ReceiverStream::new(receiver).map(|value| {
        Ok::<_, Infallible>(Event::default().event("message").data(value.to_string()))
    });
    Sse::new(endpoint.chain(messages))
        .keep_alive(KeepAlive::default())
        .into_response()
}
#[derive(Deserialize)]
struct SessionQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
}
async fn message(
    State(state): State<HttpState>,
    Query(query): Query<SessionQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = match check(&headers, &state) {
        Ok(Some(key)) => key,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(s) => return s.into_response(),
    };
    if !content_type(&headers) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let sender = {
        let sessions = state.sessions.lock().unwrap();
        match sessions.get(&query.session_id) {
            Some(session) if session.expires > Instant::now() && same_key(&session.key, &key) => {
                session.sender.clone()
            }
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let permit = match sender.try_reserve_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let response = match parse(&body) {
        Ok(request) => protocol::dispatch(request, &RequestContext::new(Some(key))).await,
        Err(error) => Some(error),
    };
    if let Some(response) = response {
        permit.send(response);
    }
    StatusCode::ACCEPTED.into_response()
}

fn same_key(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

pub fn router(port: u16) -> Router {
    Router::new()
        .route("/mcp", post(modern))
        .route("/sse", get(sse))
        .route("/messages", post(message))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    "cache-control",
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
        .with_state(HttpState {
            authority: format!("127.0.0.1:{port}"),
            sessions: Arc::default(),
        })
}

pub async fn serve(port: u16) -> Result<(), &'static str> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|_| "Cannot bind local listener")?;
    axum::serve(listener, router(port))
        .await
        .map_err(|_| "HTTP server failed")
}
