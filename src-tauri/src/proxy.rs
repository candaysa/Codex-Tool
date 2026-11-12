use axum::{
    body::{to_bytes, Body},
    extract::{ws, OriginalUri, Request, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::TcpListener,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

pub const DEFAULT_PORT: u16 = 8080;
pub const CHATGPT_API: &str = "https://chatgpt.com/backend-api";
static STARTED: OnceLock<u16> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct ProxyState {
    client: reqwest::Client,
    upstream: String,
    codex_home: Option<std::path::PathBuf>,
    routing: Arc<Mutex<Routing>>,
}

#[derive(Default)]
struct Routing {
    cooldowns: HashMap<String, Instant>,
    calls: HashMap<String, VoiceCall>,
}

#[derive(Clone)]
struct VoiceCall {
    owner: String,
    headers: HeaderMap,
    expires: Instant,
}

fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/v1/responses", post(forward).get(websocket))
        .route("/v1/responses/compact", post(forward))
        .route("/responses", post(forward).get(websocket))
        .route("/responses/compact", post(forward))
        .route("/codex/responses", post(forward).get(websocket))
        .route("/codex/responses/compact", post(forward))
        .route("/v1/codex/responses", post(forward).get(websocket))
        .route("/v1/codex/responses/compact", post(forward))
        .route("/v1/models", get(forward))
        .route("/v1/images/generations", post(forward))
        .route("/v1/images/edits", post(forward))
        .route("/v1/transcribe", post(forward))
        .route("/transcribe", post(forward))
        .route("/v1/alpha/search", post(forward))
        .route("/v1/realtime/calls", post(forward))
        .route("/v1/live", post(forward))
        .route("/v1/live/{call_id}", get(websocket))
        .route("/v1/realtime", get(websocket))
        .with_state(state)
}

pub fn running_port() -> Option<u16> {
    STARTED.get().copied()
}

pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

fn bind_listener(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)) {
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            TcpListener::bind(("127.0.0.1", 0))
        }
        result => result,
    }
}

pub fn ensure_started(preferred_port: u16) -> Result<u16, String> {
    static START_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = START_LOCK.lock().map_err(|e| e.to_string())?;
    if let Some(port) = running_port() {
        return Ok(port);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let listener = bind_listener(preferred_port)
        .map_err(|e| format!("Proxy için yerel port açılamadı: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let router = router(ProxyState {
        client,
        upstream: CHATGPT_API.to_owned(),
        codex_home: Some(crate::commands::get_codex_home()),
        routing: Arc::default(),
    });
    tauri::async_runtime::spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener)
            .expect("proxy listener must be nonblocking");
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("Codex Tool proxy: {error}");
        }
    });
    let _ = STARTED.set(port);
    crate::terminal::log_terminal(
        "PROXY",
        &format!("Proxy motoru aktif: http://127.0.0.1:{port}/v1 -> {CHATGPT_API}"),
    );
    Ok(port)
}

fn strip_transport_headers(headers: &mut HeaderMap) {
    let connection_headers: Vec<header::HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| header::HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in connection_headers {
        headers.remove(name);
    }
    for name in [
        "host",
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

pub(crate) fn candidates(state: &ProxyState, headers: &HeaderMap) -> Vec<(String, HeaderMap)> {
    let Some(token) = bearer(headers) else {
        return Vec::new();
    };
    let pool = state
        .codex_home
        .as_deref()
        .map(|home| crate::commands::proxy_accounts(home, token))
        .unwrap_or_default();
    let mut outbound = headers.clone();
    strip_transport_headers(&mut outbound);
    for name in ["cookie", "x-api-key", "x-openai-actor-authorization"] {
        outbound.remove(name);
    }
    if pool.is_empty() {
        if state
            .codex_home
            .as_deref()
            .is_some_and(|home| crate::commands::is_local_proxy_key(home, token))
        {
            crate::terminal::log_terminal(
                "WARN",
                "Havuzda geçerli oturuma sahip kullanılabilir hesap yok.",
            );
            return Vec::new();
        }
        if !outbound.contains_key("chatgpt-account-id") {
            if let Some(id) = crate::oauth::token_claims(token).and_then(|claims| {
                claims["https://api.openai.com/auth"]["chatgpt_account_id"]
                    .as_str()?
                    .parse::<header::HeaderValue>()
                    .ok()
            }) {
                outbound.insert("chatgpt-account-id", id);
            }
        }
        return vec![(String::new(), outbound)];
    }
    let mut accounts: Vec<_> = pool
        .into_iter()
        .filter_map(|(token, id)| {
            let auth = format!("Bearer {token}")
                .parse::<header::HeaderValue>()
                .ok()?;
            let account = id.parse::<header::HeaderValue>().ok()?;
            let mut headers = outbound.clone();
            headers.insert(header::AUTHORIZATION, auth);
            headers.insert("chatgpt-account-id", account);
            headers.insert(
                "originator",
                header::HeaderValue::from_static("codex_cli_rs"),
            );
            headers.insert(
                "openai-beta",
                header::HeaderValue::from_static("responses=experimental"),
            );
            Some((format!("{:x}", Sha256::digest(token.as_bytes())), headers))
        })
        .collect();
    if let Ok(mut routing) = state.routing.lock() {
        routing.cooldowns.retain(|_, until| *until > Instant::now());
        if accounts
            .iter()
            .any(|(id, _)| !routing.cooldowns.contains_key(id))
        {
            accounts.retain(|(id, _)| !routing.cooldowns.contains_key(id));
        } else {
            accounts.truncate(1);
        }
    }
    accounts
}

pub(crate) fn can_rotate(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::UNAUTHORIZED
    )
}

pub(crate) async fn record_routed_account(state: &ProxyState, headers: &HeaderMap) {
    if let (Some(home), Some(token), Some(id)) = (
        state.codex_home.as_deref(),
        bearer(headers),
        headers
            .get("chatgpt-account-id")
            .and_then(|value| value.to_str().ok()),
    ) {
        crate::commands::record_proxy_account(home, token, id).await;
    }
}

pub(crate) fn mark_cooldown(state: &ProxyState, id: &str, status: StatusCode, headers: &HeaderMap) {
    if id.is_empty() || !can_rotate(status) {
        return;
    }
    let retry_after = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let exhausted_window = ["primary", "secondary"]
        .into_iter()
        .filter_map(|window| {
            let used = headers
                .get(format!("x-codex-{window}-used-percent"))?
                .to_str()
                .ok()?
                .parse::<f64>()
                .ok()?;
            if used < 100.0 {
                return None;
            }
            headers
                .get(format!("x-codex-{window}-reset-after-seconds"))?
                .to_str()
                .ok()?
                .parse::<u64>()
                .ok()
        })
        .max();
    let limit = retry_after
        .or(exhausted_window)
        .unwrap_or(if status == StatusCode::UNAUTHORIZED {
            60
        } else {
            30
        });
    if let Ok(mut routing) = state.routing.lock() {
        routing.cooldowns.insert(
            id.to_owned(),
            Instant::now() + Duration::from_secs(limit.clamp(1, 604800)),
        );
    }
}

fn call_id(value: &str) -> bool {
    (value.starts_with("rtc_") || value.len() == 36)
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

async fn voice_body(body: Body, headers: &mut HeaderMap) -> Result<reqwest::Body, StatusCode> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type.starts_with("multipart/form-data") {
        return Ok(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let boundary = multer::parse_boundary(content_type).map_err(|_| StatusCode::BAD_REQUEST)?;
    let bytes = to_bytes(body, 1024 * 1024)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let mut parts = multer::Multipart::new(
        futures_util::stream::once(async move { Ok::<_, std::io::Error>(bytes) }),
        boundary,
    );
    let mut payload = serde_json::Map::new();
    while let Some(field) = parts
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "sdp" || name == "session" {
            let text = field.text().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            let value = if name == "session" {
                serde_json::from_str(&text).map_err(|_| StatusCode::BAD_REQUEST)?
            } else {
                serde_json::Value::String(text)
            };
            payload.insert(name, value);
        }
    }
    if payload
        .get("sdp")
        .and_then(serde_json::Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    headers.remove(header::CONTENT_LENGTH);
    Ok(serde_json::to_string(&payload)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .into())
}

async fn forward(State(state): State<ProxyState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path()
        .strip_prefix("/v1/codex")
        .or_else(|| parts.uri.path().strip_prefix("/v1"))
        .or_else(|| parts.uri.path().strip_prefix("/codex"))
        .unwrap_or(parts.uri.path());
    let voice = matches!(path, "/live" | "/realtime/calls");
    let replayable = matches!(path, "/responses" | "/responses/compact" | "/models");
    let mut accounts = candidates(&state, &parts.headers).into_iter().peekable();
    if accounts.peek().is_none() {
        return (StatusCode::UNAUTHORIZED, "Codex oturumu gerekli.").into_response();
    }
    let upstream_path = match path {
        "/transcribe" => "/transcribe".to_owned(),
        "/live" | "/realtime/calls" => "/codex/realtime/calls".to_owned(),
        _ => format!("/codex{path}"),
    };
    let Ok(mut url) = reqwest::Url::parse(&format!("{}{upstream_path}", state.upstream)) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    url.set_query(parts.uri.query());
    if path == "/live" {
        let has_intent = url.query_pairs().any(|(name, _)| name == "intent");
        let has_architecture = url.query_pairs().any(|(name, _)| name == "architecture");
        let mut query = url.query_pairs_mut();
        if !has_intent {
            query.append_pair("intent", "quicksilver");
        }
        if !has_architecture {
            query.append_pair("architecture", "avas");
        }
    }
    let mut incoming_body = Some(body);
    let replay = if replayable {
        match to_bytes(incoming_body.take().unwrap(), 64 * 1024 * 1024).await {
            Ok(bytes) if path == "/responses" => {
                let Ok(mut request) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    return StatusCode::BAD_REQUEST.into_response();
                };
                let Some(fields) = request.as_object_mut() else {
                    return StatusCode::BAD_REQUEST.into_response();
                };
                fields.insert("store".to_owned(), serde_json::json!(false));
                Some(axum::body::Bytes::from(request.to_string()))
            }
            Ok(bytes) => Some(bytes),
            Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        }
    } else {
        None
    };
    while let Some((id, mut headers)) = accounts.next() {
        let body = if let Some(bytes) = &replay {
            headers.remove(header::CONTENT_LENGTH);
            bytes.clone().into()
        } else if voice {
            match voice_body(incoming_body.take().unwrap(), &mut headers).await {
                Ok(body) => body,
                Err(status) => return status.into_response(),
            }
        } else {
            reqwest::Body::wrap_stream(incoming_body.take().unwrap().into_data_stream())
        };
        let response = state
            .client
            .request(parts.method.clone(), url.clone())
            .headers(headers.clone())
            .body(body)
            .send()
            .await;
        let Ok(upstream) = response else {
            return (StatusCode::BAD_GATEWAY, "ChatGPT sunucusuna bağlanılamadı.").into_response();
        };
        let status = upstream.status();
        mark_cooldown(&state, &id, status, upstream.headers());
        if replay.is_some() && can_rotate(status) && accounts.peek().is_some() {
            crate::terminal::log_terminal(
                "PROXY",
                &format!("{status}: sonraki uygun hesap deneniyor."),
            );
            continue;
        }
        if status.is_success() && path == "/responses" {
            let account_state = state.clone();
            let account_headers = headers.clone();
            tokio::spawn(async move {
                record_routed_account(&account_state, &account_headers).await;
            });
            let exhausted = ["primary", "secondary"].into_iter().any(|window| {
                upstream
                    .headers()
                    .get(format!("x-codex-{window}-used-percent"))
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<f64>().ok())
                    .is_some_and(|used| used >= 100.0)
            });
            if exhausted {
                mark_cooldown(
                    &state,
                    &id,
                    StatusCode::TOO_MANY_REQUESTS,
                    upstream.headers(),
                );
            }
        }
        if voice && status.is_success() {
            let location = upstream
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok());
            if let Some(call) = location
                .and_then(|value| value.split('?').next())
                .and_then(|path| path.rsplit('/').find(|segment| call_id(segment)))
            {
                if let Ok(mut routing) = state.routing.lock() {
                    let now = Instant::now();
                    routing.calls.retain(|_, call| call.expires > now);
                    if routing.calls.len() < 64 {
                        headers.remove(header::CONTENT_TYPE);
                        headers.remove(header::CONTENT_LENGTH);
                        routing.calls.insert(
                            call.to_owned(),
                            VoiceCall {
                                owner: bearer(&parts.headers).unwrap_or_default().to_owned(),
                                headers,
                                expires: now + Duration::from_secs(600),
                            },
                        );
                    }
                }
            }
        }
        let mut headers = upstream.headers().clone();
        strip_transport_headers(&mut headers);
        crate::terminal::log_terminal("PROXY", &format!("{} {} → {status}", parts.method, path));
        for window in ["primary", "secondary"] {
            let used = headers
                .get(format!("x-codex-{window}-used-percent"))
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<f64>().ok());
            if let Some(used) = used {
                let label = if window == "primary" {
                    "Birincil"
                } else {
                    "İkincil"
                };
                crate::terminal::log_terminal("QUOTA", &format!("{label} kota kullanımı: %{used}"));
            }
        }
        let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        return response;
    }
    StatusCode::BAD_GATEWAY.into_response()
}

async fn websocket(
    State(state): State<ProxyState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    upgrade: ws::WebSocketUpgrade,
) -> Response {
    let Some(owner) = bearer(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let path = uri.path();
    let sideband = !path.ends_with("/responses");
    let suffix = uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    if !sideband {
        if state
            .codex_home
            .as_deref()
            .is_some_and(|home| !crate::commands::is_local_proxy_key(home, owner))
        {
            crate::terminal::log_terminal("PROXY", "OpenAI sohbeti HTTP akışına yönlendirildi.");
            return StatusCode::UPGRADE_REQUIRED.into_response();
        }
        let base = state
            .upstream
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        let url = format!("{base}/codex/responses{suffix}");
        if candidates(&state, &headers).is_empty() {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        crate::terminal::log_terminal("PROXY", "Codex WebSocket bağlantısı açıldı.");
        return upgrade.on_upgrade(move |socket| {
            crate::responses_websocket::relay(state, url, headers, socket)
        });
    }
    let (url, accounts) = if sideband {
        let query = reqwest::Url::parse(&format!("http://localhost{path}{suffix}")).ok();
        let id = path
            .strip_prefix("/v1/live/")
            .map(str::to_owned)
            .or_else(|| {
                query?
                    .query_pairs()
                    .find(|(key, _)| key == "call_id")
                    .map(|(_, id)| id.into_owned())
            });
        let Some(id) = id.filter(|id| call_id(id)) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let saved = state.routing.lock().ok().and_then(|mut routing| {
            let call = routing.calls.get_mut(&id)?;
            if call.owner != owner || call.expires <= Instant::now() {
                return None;
            }
            call.expires = Instant::now() + Duration::from_secs(600);
            Some(call.headers.clone())
        });
        let Some(mut saved) = saved else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if let Some(beta) = headers.get("openai-beta") {
            saved.insert("openai-beta", beta.clone());
        }
        (
            format!("wss://api.openai.com{path}{suffix}"),
            vec![(String::new(), saved)],
        )
    } else {
        let base = state
            .upstream
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        (
            format!("{base}/codex/responses{suffix}"),
            candidates(&state, &headers),
        )
    };
    let mut status = StatusCode::BAD_GATEWAY;
    for (id, outbound) in accounts {
        let Ok(mut request) = url.as_str().into_client_request() else {
            break;
        };
        for (name, value) in &outbound {
            if !name.as_str().starts_with("sec-websocket-") && name != header::CONTENT_LENGTH {
                request.headers_mut().append(name, value.clone());
            }
        }
        if let Some(protocols) = headers.get(header::SEC_WEBSOCKET_PROTOCOL) {
            request
                .headers_mut()
                .insert(header::SEC_WEBSOCKET_PROTOCOL, protocols.clone());
        }
        match tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async(request),
        )
        .await
        {
            Ok(Ok((upstream, response))) => {
                record_routed_account(&state, &outbound).await;
                crate::terminal::log_terminal(
                    "PROXY",
                    &format!("WebSocket {path} → bağlantı kuruldu"),
                );
                let protocol = response
                    .headers()
                    .get(header::SEC_WEBSOCKET_PROTOCOL)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned);
                let upgrade = if let Some(protocol) = protocol {
                    upgrade.protocols([protocol])
                } else {
                    upgrade
                };
                return upgrade.on_upgrade(move |mut local| async move {
                let mut upstream = upstream;
                loop {
                    tokio::select! {
                        frame = local.next() => {
                            let message = match frame {
                                Some(Ok(ws::Message::Text(text))) => tungstenite::Message::Text(text.as_str().into()),
                                Some(Ok(ws::Message::Binary(bytes))) => tungstenite::Message::Binary(bytes),
                                Some(Ok(ws::Message::Ping(bytes))) => tungstenite::Message::Ping(bytes),
                                Some(Ok(ws::Message::Pong(bytes))) => tungstenite::Message::Pong(bytes),
                                _ => { let _ = upstream.close(None).await; break; }
                            };
                            if upstream.send(message).await.is_err() { break; }
                        }
                        frame = upstream.next() => {
                            let message = match frame {
                                Some(Ok(tungstenite::Message::Text(text))) => {
                                    if !sideband && !id.is_empty() {
                                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                                            let status = match event["error"]["code"].as_str() {
                                                Some("usage_limit_reached" | "rate_limit_exceeded" | "insufficient_quota") => Some(StatusCode::TOO_MANY_REQUESTS),
                                                Some("token_expired" | "invalid_api_key") => Some(StatusCode::UNAUTHORIZED),
                                                _ => None,
                                            };
                                            if let Some(status) = status {
                                                mark_cooldown(&state, &id, status, &HeaderMap::new());
                                                let _ = local.send(ws::Message::Text(text.as_str().into())).await;
                                                let _ = local.send(ws::Message::Close(None)).await;
                                                break;
                                            }
                                        }
                                    }
                                    ws::Message::Text(text.as_str().into())
                                }
                                Some(Ok(tungstenite::Message::Binary(bytes))) => ws::Message::Binary(bytes),
                                Some(Ok(tungstenite::Message::Ping(bytes))) => ws::Message::Ping(bytes),
                                Some(Ok(tungstenite::Message::Pong(bytes))) => ws::Message::Pong(bytes),
                                _ => { let _ = local.send(ws::Message::Close(None)).await; break; }
                            };
                            if local.send(message).await.is_err() { break; }
                        }
                    }
                }
                });
            }
            Ok(Err(tungstenite::Error::Http(response))) => {
                status = response.status();
                crate::terminal::log_terminal("WARN", &format!("WebSocket {path} → {status}"));
                mark_cooldown(&state, &id, status, response.headers());
                if !can_rotate(status) {
                    break;
                }
            }
            _ => break,
        }
    }
    status.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires CODEX_TOOL_TEST_ENGINE pointing at the installed Codex executable"]
    async fn installed_codex_hides_profile_and_continues_after_quota_rejection() {
        use serde_json::{json, Value};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let engine =
            std::env::var_os("CODEX_TOOL_TEST_ENGINE").expect("Set CODEX_TOOL_TEST_ENGINE");
        let home = tempfile::tempdir().unwrap();
        let key = crate::commands::local_proxy_key(home.path()).unwrap();
        std::fs::write(home.path().join("accounts/accounts.json"),
            r#"{"activeIndex":0,"accounts":[{"accountId":"limited","accessToken":"first"},{"accountId":"ready","accessToken":"second"}]}"#).unwrap();
        // The native profile is deliberately present. Provider authentication must hide it.
        std::fs::write(home.path().join("auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"native","id_token":"native","refresh_token":"native","account_id":"native"}}"#).unwrap();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = attempts.clone();
        let rejections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let rejected = rejections.clone();
        let mock = Router::new().route(
            "/codex/responses",
            axum::routing::post(move |headers: HeaderMap| {
                let counter = counter.clone();
                let rejected = rejected.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if headers[header::AUTHORIZATION] == "Bearer first" {
                        rejected.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return (StatusCode::TOO_MANY_REQUESTS, "test quota").into_response();
                    }
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer second");
                    let item = json!({"id":"msg_test","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"OK","annotations":[]}]});
                    let events = [
                        json!({"type":"response.created","response":{"id":"resp_test","status":"in_progress"}}),
                        json!({"type":"response.output_item.done","output_index":0,"item":item}),
                        json!({"type":"response.completed","response":{"id":"resp_test","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
                    ];
                    let stream: String = events.iter().map(|event| format!("data: {event}\n\n")).collect();
                    ([(header::CONTENT_TYPE, "text/event-stream")], stream).into_response()
                }
            }),
        );
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let mock_task = tokio::spawn(async move { axum::serve(upstream, mock).await.unwrap() });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = router(ProxyState {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            upstream: format!("http://{address}"),
            codex_home: Some(home.path().to_owned()),
            routing: Arc::default(),
        });
        let proxy_task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let config = crate::commands::proxy_config_with_key("", Some(port), None, &key).unwrap();
        std::fs::write(home.path().join("config.toml"), config).unwrap();
        let mut process = tokio::process::Command::new(engine);
        process
            .arg("app-server")
            .env("CODEX_HOME", home.path())
            .current_dir(home.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        process.creation_flags(0x08000000);
        let mut child = process.spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
        async fn send(input: &mut tokio::process::ChildStdin, value: Value) {
            input
                .write_all(format!("{value}\n").as_bytes())
                .await
                .unwrap();
            input.flush().await.unwrap();
        }
        async fn reply(
            output: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
            id: i64,
        ) -> Value {
            tokio::time::timeout(Duration::from_secs(20), async {
                while let Some(line) = output.next_line().await.unwrap() {
                    let value: Value = serde_json::from_str(&line).unwrap();
                    if value["id"] == id {
                        assert!(value.get("error").is_none(), "{value}");
                        return value["result"].clone();
                    }
                }
                panic!("Codex closed before replying");
            })
            .await
            .unwrap()
        }
        send(&mut input, json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"codex-tool-integration-test","version":"1.0.0"}}})).await;
        reply(&mut output, 1).await;
        send(&mut input, json!({"method":"initialized"})).await;
        send(
            &mut input,
            json!({"id":2,"method":"account/read","params":{"refreshToken":false}}),
        )
        .await;
        let account = reply(&mut output, 2).await;
        assert!(account["account"].is_null(), "{account}");
        assert_eq!(account["requiresOpenaiAuth"], false);
        send(&mut input, json!({"id":3,"method":"thread/start","params":{"model":"gpt-5.4","approvalPolicy":"never","sandbox":"read-only"}})).await;
        let thread = reply(&mut output, 3).await;
        let thread_id = thread["thread"]["id"].as_str().unwrap();
        send(&mut input, json!({"id":4,"method":"turn/start","params":{"threadId":thread_id,"input":[{"type":"text","text":"Say OK","text_elements":[]}]}})).await;
        reply(&mut output, 4).await;
        let turn = tokio::time::timeout(Duration::from_secs(25), async {
            while let Some(line) = output.next_line().await.unwrap() {
                let event: Value = serde_json::from_str(&line).unwrap();
                if event["method"] == "turn/completed" {
                    return event["params"]["turn"].clone();
                }
            }
            panic!("Missing completed turn");
        })
        .await
        .unwrap();
        assert_eq!(turn["status"], "completed", "{turn}");
        assert!(attempts.load(std::sync::atomic::Ordering::Relaxed) >= 2);
        assert_eq!(rejections.load(std::sync::atomic::Ordering::Relaxed), 1);
        send(&mut input, json!({"id":5,"method":"turn/start","params":{"threadId":thread_id,"input":[{"type":"text","text":"Say OK again","text_elements":[]}]}})).await;
        reply(&mut output, 5).await;
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(line) = output.next_line().await.unwrap() {
                let event: Value = serde_json::from_str(&line).unwrap();
                if event["method"] == "turn/completed" {
                    assert_eq!(event["params"]["turn"]["status"], "completed", "{event}");
                    return;
                }
            }
            panic!("Missing second turn");
        })
        .await
        .unwrap();
        assert_eq!(rejections.load(std::sync::atomic::Ordering::Relaxed), 1);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let pool: Value = serde_json::from_str(
                    &std::fs::read_to_string(home.path().join("accounts/accounts.json")).unwrap(),
                )
                .unwrap();
                if pool["activeIndex"] == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        child.kill().await.unwrap();
        proxy_task.abort();
        mock_task.abort();
    }

    #[test]
    fn uses_another_loopback_port_when_the_preferred_port_is_busy() {
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let preferred = occupied.local_addr().unwrap().port();
        let fallback = bind_listener(preferred).unwrap().local_addr().unwrap();
        assert!(fallback.ip().is_loopback());
        assert_ne!(fallback.port(), preferred);
    }

    #[tokio::test]
    async fn forwards_responses_auth_body_and_stream() {
        let home = tempfile::tempdir().unwrap();
        let local_key = crate::commands::local_proxy_key(home.path()).unwrap();
        std::fs::create_dir_all(home.path().join("accounts")).unwrap();
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"tokens":{"access_token":"native"}}"#,
        )
        .unwrap();
        std::fs::write(home.path().join("accounts/accounts.json"),
            r#"{"activeIndex":0,"accounts":[{"accountId":"limited","accessToken":"first"},{"accountId":"test-account","accessToken":"test"}]}"#).unwrap();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = attempts.clone();
        let mock = Router::new()
            .route(
                "/codex/responses",
                post(
                    move |uri: axum::http::Uri, headers: HeaderMap, body: String| async move {
                        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        assert_eq!(uri.query(), Some("test=yes"));
                        assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap(), serde_json::json!({"stream":true,"store":false}));
                        if headers[header::AUTHORIZATION] == "Bearer first" {
                            assert_eq!(headers["chatgpt-account-id"], "limited");
                            return (StatusCode::TOO_MANY_REQUESTS, "quota").into_response();
                        }
                        assert_eq!(headers[header::AUTHORIZATION], "Bearer test");
                        assert_eq!(headers["chatgpt-account-id"], "test-account");
                        (
                            [(header::CONTENT_TYPE, "text/event-stream")],
                            "data: hello\n\n",
                        )
                            .into_response()
                    },
                ),
            )
            .route(
                "/codex/responses",
                get(
                    |upgrade: ws::WebSocketUpgrade, headers: HeaderMap| async move {
                        assert_eq!(headers[header::AUTHORIZATION], "Bearer test");
                        upgrade.on_upgrade(|mut socket| async move {
                            if let Some(Ok(message)) = socket.next().await {
                                assert!(matches!(message, ws::Message::Text(_)));
                                socket.send(ws::Message::Text(r#"{"type":"response.completed","response":{"id":"resp_echo","output":[]}}"#.into())).await.unwrap();
                            }
                        })
                    },
                ),
            )
            .route("/transcribe", post(|body: String| async move { body }))
            .route(
                "/codex/images/edits",
                post(|body: String| async move { body }),
            )
            .route(
                "/codex/realtime/calls",
                post(|body: String| async move {
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(&body).unwrap()["sdp"],
                        "sample SDP"
                    );
                    (
                        [(header::LOCATION, "/v1/realtime/calls/rtc_test")],
                        "answer",
                    )
                }),
            );
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(upstream, mock).await.unwrap() });
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let app = router(ProxyState {
            client: client.clone(),
            upstream: format!("http://{address}"),
            codex_home: Some(home.path().to_owned()),
            routing: Arc::default(),
        });
        let proxy_task = tokio::spawn(async move { axum::serve(proxy, app).await.unwrap() });
        let url = format!("http://{proxy_address}/v1/responses?test=yes");
        let unauthorized = client.post(&url).send().await.unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let mut native_request = format!("ws://{proxy_address}/v1/responses")
            .into_client_request()
            .unwrap();
        native_request
            .headers_mut()
            .insert(header::AUTHORIZATION, "Bearer native".parse().unwrap());
        let result = tokio_tungstenite::connect_async(native_request).await;
        assert!(matches!(
            result,
            Err(tungstenite::Error::Http(response))
                if response.status() == StatusCode::UPGRADE_REQUIRED
        ));
        let response = client
            .post(&url)
            .bearer_auth(&local_key)
            .body(r#"{"stream":true}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/event-stream"
        );
        assert_eq!(response.text().await.unwrap(), "data: hello\n\n");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 2);
        let native_response = client
            .post(&url)
            .bearer_auth("native")
            .body(r#"{"stream":true}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(native_response.status(), StatusCode::OK);
        assert_eq!(native_response.text().await.unwrap(), "data: hello\n\n");
        assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 3);
        let mut request = format!("ws://{proxy_address}/v1/responses")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {local_key}").parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(tungstenite::Message::Text(
                r#"{"type":"response.create","model":"test","input":[]}"#.into(),
            ))
            .await
            .unwrap();
        let completed = socket.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(completed.contains("response.completed"));
        let upload = "--test\r\nContent-Disposition: form-data; name=\"sdp\"\r\n\r\nsample SDP\r\n--test--\r\n";
        for path in ["/v1/transcribe", "/v1/images/edits", "/v1/live"] {
            let response = client
                .post(format!("http://{proxy_address}{path}"))
                .bearer_auth(&local_key)
                .header(header::CONTENT_TYPE, "multipart/form-data; boundary=test")
                .body(upload)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.text().await.unwrap(),
                if path == "/v1/live" { "answer" } else { upload }
            );
        }
        proxy_task.abort();
        server.abort();
    }
}
