use crate::proxy::{self, ProxyState};
use axum::{
    extract::ws::{Message, WebSocket},
    http::{header, HeaderMap, StatusCode},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, client::IntoClientRequest},
};

const MAX_CONTEXT_BYTES: usize = 8 * 1024 * 1024;

fn error(code: &str, message: &str) -> Message {
    Message::Text(json!({"type":"error","error":{"type":"invalid_request_error","code":code,"message":message}}).to_string().into())
}

fn rejection(event: &Value) -> Option<StatusCode> {
    let reason = event
        .get("error")
        .or_else(|| event.get("response")?.get("error"))?;
    match reason["code"].as_str().or_else(|| reason["type"].as_str()) {
        Some("usage_limit_reached" | "rate_limit_exceeded" | "insufficient_quota") => {
            Some(StatusCode::TOO_MANY_REQUESTS)
        }
        Some("token_expired" | "invalid_api_key" | "invalid_authentication") => {
            Some(StatusCode::UNAUTHORIZED)
        }
        _ => reason["status"]
            .as_u64()
            .and_then(|status| StatusCode::from_u16(status as u16).ok())
            .filter(|status| proxy::can_rotate(*status)),
    }
}

fn complete_input(mut request: Value, history: &HashMap<String, Value>) -> Result<Value, Message> {
    if request["type"] != "response.create" || !request["input"].is_array() {
        return Err(error(
            "invalid_request_body",
            "response.create ve input gerekli.",
        ));
    }
    if let Some(previous) = request["previous_response_id"].as_str() {
        let Some(chain) = history.get(previous) else {
            return Err(error(
                "previous_response_not_found",
                "Retry with complete input and no previous_response_id.",
            ));
        };
        let mut input = chain["input"].as_array().cloned().unwrap_or_default();
        input.extend(chain["output"].as_array().cloned().unwrap_or_default());
        input.extend(request["input"].as_array().cloned().unwrap_or_default());
        request["input"] = json!(input);
    }
    request
        .as_object_mut()
        .unwrap()
        .remove("previous_response_id");
    request["store"] = json!(false);
    request["stream"] = json!(true);
    if request.to_string().len() > MAX_CONTEXT_BYTES {
        return Err(error(
            "websocket_context_too_large",
            "Konuşma bağlamı yerel sınırı aşıyor.",
        ));
    }
    Ok(request)
}

pub(crate) async fn relay(
    state: ProxyState,
    url: String,
    headers: HeaderMap,
    mut client: WebSocket,
) {
    let mut history = HashMap::<String, Value>::new();
    let mut history_order = VecDeque::new();
    let mut queue = VecDeque::<Value>::new();
    'requests: loop {
        let request = if let Some(request) = queue.pop_front() {
            request
        } else {
            match client.next().await {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Value>(text.as_str())
                {
                    Ok(request) => request,
                    Err(_) => {
                        if client
                            .send(error(
                                "invalid_websocket_event",
                                "Geçersiz WebSocket mesajı.",
                            ))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                },
                Some(Ok(Message::Ping(bytes))) => {
                    if client.send(Message::Pong(bytes)).await.is_err() {
                        return;
                    }
                    continue;
                }
                Some(Ok(Message::Pong(_))) => continue,
                _ => return,
            }
        };
        let request = match complete_input(request, &history) {
            Ok(request) => request,
            Err(message) => {
                if client.send(message).await.is_err() {
                    return;
                }
                continue;
            }
        };
        let body = request.to_string();
        let mut accounts = proxy::candidates(&state, &headers).into_iter().peekable();
        'accounts: while let Some((identity, outbound)) = accounts.next() {
            let Ok(mut handshake) = url.as_str().into_client_request() else {
                return;
            };
            for (name, value) in &outbound {
                if !name.as_str().starts_with("sec-websocket-") && name != header::CONTENT_LENGTH {
                    handshake.headers_mut().insert(name, value.clone());
                }
            }
            let connection =
                tokio::time::timeout(Duration::from_secs(15), connect_async(handshake)).await;
            let mut upstream = match connection {
                Ok(Ok((socket, _))) => socket,
                Ok(Err(tungstenite::Error::Http(response))) => {
                    proxy::mark_cooldown(&state, &identity, response.status(), response.headers());
                    if proxy::can_rotate(response.status()) && accounts.peek().is_some() {
                        crate::terminal::log_terminal(
                            "ROUTE",
                            &format!("WebSocket {}: sıradaki hesap deneniyor.", response.status()),
                        );
                        continue;
                    }
                    let _ = client
                        .send(error(
                            "upstream_rejected",
                            &format!("Sunucu HTTP {} döndürdü.", response.status()),
                        ))
                        .await;
                    continue 'requests;
                }
                _ => {
                    let _ = client
                        .send(error(
                            "upstream_websocket_disconnected",
                            "Upstream WebSocket bağlantısı açılamadı.",
                        ))
                        .await;
                    continue 'requests;
                }
            };
            if upstream
                .send(tungstenite::Message::Text(body.clone().into()))
                .await
                .is_err()
            {
                let _ = client
                    .send(error(
                        "upstream_websocket_disconnected",
                        "İstek sunucuya iletilemedi.",
                    ))
                    .await;
                continue 'requests;
            }
            let mut output_started = false;
            let mut buffered = VecDeque::<Message>::new();
            let mut buffered_bytes = 0;
            let mut output = Vec::new();
            loop {
                tokio::select! {
                    incoming = client.next() => {
                        match incoming {
                            Some(Ok(Message::Text(text))) => {
                                let event = serde_json::from_str::<Value>(text.as_str()).unwrap_or_default();
                                if event["type"] == "response.create" {
                                    if queue.len() < 4 && text.len() <= MAX_CONTEXT_BYTES { queue.push_back(event); }
                                    else if client.send(error("websocket_queue_full", "Bekleyen istek sınırı doldu.")).await.is_err() { return; }
                                } else if upstream.send(tungstenite::Message::Text(text.as_str().into())).await.is_err() {
                                    continue 'requests;
                                }
                            }
                            Some(Ok(Message::Ping(bytes))) => { if client.send(Message::Pong(bytes)).await.is_err() { return; } }
                            Some(Ok(Message::Pong(_))) => {}
                            _ => { let _ = upstream.close(None).await; return; }
                        }
                    }
                    frame = tokio::time::timeout(Duration::from_secs(300), upstream.next()) => {
                        let text = match frame {
                            Ok(Some(Ok(tungstenite::Message::Text(text)))) => text,
                            Ok(Some(Ok(tungstenite::Message::Ping(bytes)))) => {
                                if upstream.send(tungstenite::Message::Pong(bytes)).await.is_err() { continue 'requests; }
                                continue;
                            }
                            Ok(Some(Ok(tungstenite::Message::Pong(_)))) => continue,
                            _ => {
                                let _ = client.send(error("upstream_websocket_disconnected", "Upstream WebSocket closed before completing the response.")).await;
                                continue 'requests;
                            }
                        };
                        let event = serde_json::from_str::<Value>(text.as_str()).unwrap_or_default();
                        if let Some(status) = rejection(&event) {
                            proxy::mark_cooldown(&state, &identity, status, &HeaderMap::new());
                            if !output_started && accounts.peek().is_some() {
                                crate::terminal::log_terminal("ROUTE", &format!("WebSocket {status}: kota/oturum reddi sonrası aynı istek diğer hesaba aktarılıyor."));
                                let _ = upstream.close(None).await;
                                continue 'accounts;
                            }
                        }
                        let kind = event["type"].as_str().unwrap_or_default();
                        if kind == "response.output_item.done" {
                            if let Some(item) = event.get("item") { output.push(item.clone()); }
                        }
                        let terminal = matches!(kind, "response.completed" | "response.failed" | "response.incomplete" | "response.cancelled" | "error");
                        let preamble = matches!(kind, "response.created" | "response.in_progress" | "rate_limits.updated");
                        if !output_started && preamble && buffered_bytes + text.len() < 1024 * 1024 {
                            buffered_bytes += text.len();
                            buffered.push_back(Message::Text(text.as_str().into()));
                            continue;
                        }
                        if !output_started {
                            output_started = true;
                            if rejection(&event).is_none() { proxy::record_routed_account(&state, &outbound).await; }
                        }
                        while let Some(message) = buffered.pop_front() {
                            if client.send(message).await.is_err() { return; }
                        }
                        if client.send(Message::Text(text.as_str().into())).await.is_err() { return; }
                        if terminal {
                            if kind == "response.completed" || (kind == "response.incomplete" && event["response"]["incomplete_details"]["reason"] == "interrupted") {
                                if let Some(id) = event["response"]["id"].as_str() {
                                    let output = event["response"]["output"].as_array().cloned().unwrap_or(output);
                                    let chain = json!({"input":request["input"], "output":output});
                                    if chain.to_string().len() <= MAX_CONTEXT_BYTES {
                                        history.insert(id.to_owned(), chain);
                                        history_order.push_back(id.to_owned());
                                        while history_order.len() > 2 {
                                            if let Some(old) = history_order.pop_front() { history.remove(&old); }
                                        }
                                    }
                                }
                            }
                            crate::terminal::log_terminal("PROXY", &format!("WebSocket yanıtı: {kind}"));
                            let _ = upstream.close(None).await;
                            continue 'requests;
                        }
                    }
                }
            }
        }
        if client
            .send(error(
                "usage_limit_reached",
                "Havuzda kullanılabilir oturuma sahip hesap yok.",
            ))
            .await
            .is_err()
        {
            return;
        }
    }
}
