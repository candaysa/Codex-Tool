use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn usage_window(window: &Value, now: u64) -> Option<Value> {
    let seconds = window.get("limit_window_seconds")?.as_u64()?;
    let used = window.get("used_percent")?.as_f64()?;
    if seconds == 0 {
        return None;
    }
    let reset = window
        .get("reset_at")
        .and_then(Value::as_u64)
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.saturating_mul(1000))
        .or_else(|| {
            window
                .get("reset_after_seconds")
                .and_then(Value::as_u64)
                .filter(|seconds| *seconds > 0)
                .map(|seconds| now.saturating_add(seconds.saturating_mul(1000)))
        });
    Some(json!({
        "windowMinutes": seconds.div_ceil(60),
        "usedPercent": used.clamp(0.0, 100.0).floor() as u32,
        "resetAtMs": reset,
    }))
}

fn snapshot(payload: &Value, now: u64) -> Option<Value> {
    let plan = payload.get("plan_type").and_then(Value::as_str);
    let limits = payload
        .get("rate_limit")
        .filter(|limits| limits.is_object());
    if plan.is_none() && limits.is_none() {
        return None;
    }
    Some(json!({
        "updatedAt": now,
        "planType": plan,
        "status": if limits.is_some_and(|limits| limits["limit_reached"].as_bool() == Some(true)) { 429 } else { 200 },
        "primary": limits.and_then(|limits| usage_window(&limits["primary_window"], now)),
        "secondary": limits.and_then(|limits| usage_window(&limits["secondary_window"], now)),
    }))
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

pub fn reuse_codex_session(account: &mut Value, tokens: &Value) -> bool {
    let Some(refresh) = tokens["refresh_token"]
        .as_str()
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(access) = tokens["access_token"]
        .as_str()
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let incoming_expiry = crate::oauth::token_claims(access)
        .and_then(|claims| claims["exp"].as_u64())
        .map(|seconds| seconds.saturating_mul(1000));
    if account["accessToken"].as_str() != Some(access)
        && incoming_expiry
            .zip(account["expiresAt"].as_u64())
            .is_some_and(|(incoming, stored)| incoming < stored)
    {
        return false;
    }
    let same_token = account["refreshToken"].as_str() == Some(refresh);
    if !same_token {
        if tokens["account_id"].as_str().is_none() || tokens["account_id"] != account["accountId"] {
            return false;
        }
        let id_claims = tokens["id_token"]
            .as_str()
            .and_then(crate::oauth::token_claims)
            .unwrap_or_default();
        let access_claims = crate::oauth::token_claims(access).unwrap_or_default();
        let email = id_claims["email"]
            .as_str()
            .or_else(|| access_claims["https://api.openai.com/profile"]["email"].as_str());
        if !email
            .zip(account["email"].as_str())
            .is_some_and(|(live, stored)| live.eq_ignore_ascii_case(stored))
        {
            return false;
        }
    }
    account["accessToken"] = json!(access);
    account["refreshToken"] = json!(refresh);
    if let Some(id_token) = tokens["id_token"].as_str().filter(|token| *token != access) {
        account["idToken"] = json!(id_token);
    }
    if let Some(expiry) = incoming_expiry {
        account["expiresAt"] = json!(expiry);
    }
    true
}

async fn fetch(
    client: &reqwest::Client,
    id: &str,
    token: &str,
) -> Result<reqwest::Response, String> {
    client
        .get(format!("{}/wham/usage", crate::proxy::CHATGPT_API))
        .bearer_auth(token)
        .header("chatgpt-account-id", id)
        .header("originator", "codex_cli_rs")
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|_| "Kota sunucusuna bağlanılamadı.".to_string())
}

pub async fn refresh(
    client: &reqwest::Client,
    account: &mut Value,
    codex_tokens: &Value,
) -> Result<Value, String> {
    let id = account["accountId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Hesap kimliği eksik. Hesabı yeniden ekleyin.")?
        .to_owned();
    let shared_session = reuse_codex_session(account, codex_tokens);
    let refresh_token = account["refreshToken"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let access_token = account["accessToken"]
        .as_str()
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    let response = match access_token.as_deref() {
        Some(token) => Some(fetch(client, &id, token).await?),
        None => None,
    };
    let response = match response {
        Some(response) if response.status() != reqwest::StatusCode::UNAUTHORIZED => response,
        _ if refresh_token.is_empty() => {
            return Err("Oturumu yenilemek için Codex’te tekrar giriş yapın.".to_string())
        }
        _ => {
            let tokens = crate::oauth::refresh_tokens(client, &refresh_token).await?;
            let token = tokens["access_token"]
                .as_str()
                .filter(|token| !token.is_empty())
                .ok_or("Yeni erişim anahtarı alınamadı.")?;
            account["accessToken"] = json!(token);
            let mut new_refresh = refresh_token.to_string();
            if let Some(refresh) = tokens["refresh_token"]
                .as_str()
                .filter(|token| !token.is_empty())
            {
                account["refreshToken"] = json!(refresh);
                new_refresh = refresh.to_string();
            }
            let mut id_token_val = account["idToken"].as_str().map(str::to_owned);
            if let Some(id_token) = tokens["id_token"].as_str() {
                account["idToken"] = json!(id_token);
                id_token_val = Some(id_token.to_string());
            }
            if let Some(seconds) = tokens["expires_in"].as_u64() {
                account["expiresAt"] = json!(now_ms().saturating_add(seconds.saturating_mul(1000)));
            }

            // If this was the active session in ~/.codex/auth.json, keep auth.json in sync with fresh tokens
            if shared_session {
                let codex_home = crate::commands::get_codex_home();
                let email = account["email"].as_str().unwrap_or_default();
                let acc_id = account["accountId"].as_str().unwrap_or_default();
                let auth_path = codex_home.join("auth.json");
                let iso_now =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let auth_data = json!({
                    "auth_mode": "chatgpt",
                    "OPENAI_API_KEY": null,
                    "tokens": {
                        "id_token": id_token_val.as_deref().unwrap_or(token),
                        "access_token": token,
                        "refresh_token": new_refresh,
                        "oauth_provenance": {
                            "issuer": "https://auth.openai.com",
                            "client_id": crate::oauth::OAUTH_CLIENT_ID
                        },
                        "account_id": acc_id
                    },
                    "last_refresh": iso_now,
                    "email": email,
                    "codexToolSyncVersion": now_ms()
                });
                if let Ok(rendered) = serde_json::to_string_pretty(&auth_data) {
                    let _ = std::fs::write(&auth_path, rendered);
                }
            }

            fetch(client, &id, token).await?
        }
    };
    if !response.status().is_success() {
        return Err(format!(
            "Kota sorgusu başarısız (HTTP {}).",
            response.status().as_u16()
        ));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| "Kota yanıtı okunamadı.")?;
    snapshot(&payload, now_ms()).ok_or_else(|| "Kota yanıtı geçersiz.".to_string())
}

pub fn is_legacy_placeholder(_entry: &Value) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_native_session_does_not_replace_newer_pool_tokens() {
        use base64::prelude::*;
        let old_access = format!(
            "header.{}.signature",
            BASE64_URL_SAFE_NO_PAD.encode(br#"{"exp":1000}"#)
        );
        let mut account = json!({"accessToken":"newer-token","refreshToken":"same-refresh","expiresAt":2_000_000});
        let original = account.clone();
        assert!(!reuse_codex_session(
            &mut account,
            &json!({
                "access_token":old_access,"refresh_token":"same-refresh"
            })
        ));
        assert_eq!(account, original);
    }

    #[test]
    fn weekly_primary_stays_weekly_even_with_five_days_remaining() {
        let now = 1_000_000_000;
        let entry = snapshot(&json!({"plan_type":"free","rate_limit":{
            "primary_window":{"limit_window_seconds":604800,"used_percent":12.5,"reset_after_seconds":432000},
            "secondary_window":null
        }}), now).unwrap();
        assert_eq!(entry["primary"]["windowMinutes"], 10080);
        assert_eq!(entry["primary"]["resetAtMs"], now + 432_000_000);
        assert!(entry["secondary"].is_null());
    }

    #[test]
    fn dual_limits_and_unstarted_windows_remain_distinct() {
        let entry = snapshot(&json!({"rate_limit":{
            "primary_window":{"limit_window_seconds":18000,"used_percent":0,"reset_at":0},
            "secondary_window":{"limit_window_seconds":604800,"used_percent":100,"reset_at":1234567},
            "limit_reached":true
        }}), 1_000_000_000).unwrap();
        assert_eq!(entry["primary"]["windowMinutes"], 300);
        assert!(entry["primary"]["resetAtMs"].is_null());
        assert_eq!(entry["secondary"]["usedPercent"], 100);
        assert_eq!(entry["status"], 429);
    }
}
