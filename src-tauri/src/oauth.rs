use base64::prelude::*;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OAUTH_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

#[derive(Debug, Clone)]
pub struct OAuthTokenResult {
    pub access_token: String,
    pub id_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub email: String,
    pub account_id: String,
    pub plan_type: String,
}

fn generate_random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn generate_pkce() -> Result<(String, String), String> {
    let verifier_bytes = generate_random_bytes::<32>()?;
    let verifier = BASE64_URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    let challenge = BASE64_URL_SAFE_NO_PAD.encode(digest);

    Ok((verifier, challenge))
}

fn generate_state() -> Result<String, String> {
    let bytes = generate_random_bytes::<16>()?;
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

pub async fn run_native_oauth_login(
    on_url: tauri::ipc::Channel<String>,
) -> Result<OAuthTokenResult, String> {
    let (verifier, challenge) = generate_pkce()?;
    let expected_state = generate_state()?;

    let listener = TcpListener::bind("127.0.0.1:1455").await.map_err(|e| {
        format!(
            "1455 portu dinlenemedi (başka bir uygulama kullanıyor olabilir): {}",
            e
        )
    })?;

    let auth_url = format!(
        "https://auth.openai.com/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope=openid%20profile%20email%20offline_access&code_challenge={}&code_challenge_method=S256&state={}&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=codex_cli_rs&prompt=login",
        OAUTH_CLIENT_ID,
        urlencoding_helper(OAUTH_REDIRECT_URI),
        challenge,
        expected_state
    );

    on_url.send(auth_url.clone()).map_err(|e| e.to_string())?;
    open_browser(&auth_url);

    // Wait for incoming redirect on 127.0.0.1:1455 (5 minutes timeout)
    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(300), listener.accept())
            .await
            .map_err(|_| {
                "Giriş işlemi zaman aşımına uğradı (5 dakika). Lütfen tekrar deneyin.".to_string()
            })?
            .map_err(|e| format!("Bağlantı kabul edilemedi: {}", e))?;

    let mut buf = [0u8; 4096];
    let n = tokio::time::timeout(std::time::Duration::from_secs(10), stream.read(&mut buf))
        .await
        .map_err(|_| "Giriş yönlendirmesi zaman aşımına uğradı.")?
        .map_err(|e| format!("İstek okunamadı: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let mut first_line = request.lines().next().unwrap_or("").split_whitespace();
    if first_line.next() != Some("GET") {
        return Err("OAuth yönlendirmesi geçersiz.".to_string());
    }
    let target = first_line
        .next()
        .filter(|path| path.starts_with("/auth/callback?"))
        .ok_or("OAuth yönlendirme yolu geçersiz.")?;
    let callback = reqwest::Url::parse(&format!("http://localhost:1455{target}"))
        .map_err(|_| "OAuth yönlendirmesi okunamadı.")?;
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let state = callback
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();

    if state != expected_state {
        let err_response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nState dogrulamasi basarisiz oldu.";
        let _ = stream.write_all(err_response.as_bytes()).await;
        return Err("OAuth state doğrulaması başarısız oldu.".to_string());
    }

    if code.is_empty() {
        let err_response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nAuthorization code alinamadi.";
        let _ = stream.write_all(err_response.as_bytes()).await;
        return Err("OpenAI yetkilendirme kodu alınamadı.".to_string());
    }

    // Return friendly HTML success page
    let html_body = r#"<!DOCTYPE html>
<html lang="tr">
<head>
  <meta charset="utf-8">
  <title>Yetkilendirme Alındı - Codex Tool</title>
  <style>
    body {
      background: #090d16;
      color: #f1f5f9;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      display: flex;
      align-items: center;
      justify-content: center;
      height: 100vh;
      margin: 0;
    }
    .card {
      background: #111827;
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 20px;
      padding: 36px 48px;
      text-align: center;
      max-width: 420px;
      box-shadow: 0 20px 40px rgba(0, 0, 0, 0.6);
    }
    .badge {
      display: inline-block;
      width: 48px;
      height: 48px;
      line-height: 48px;
      border-radius: 50%;
      background: rgba(14, 165, 233, 0.15);
      border: 1px solid rgba(14, 165, 233, 0.4);
      font-size: 24px;
      margin-bottom: 16px;
    }
    h2 { margin: 0 0 8px; font-size: 20px; font-weight: 700; color: #38bdf8; }
    p { margin: 0 0 24px; font-size: 13px; color: #94a3b8; line-height: 1.5; }
    button {
      background: #0284c7;
      color: white;
      border: none;
      padding: 10px 24px;
      border-radius: 10px;
      font-size: 13px;
      font-weight: 600;
      cursor: pointer;
      transition: background 0.2s;
    }
    button:hover { background: #0369a1; }
  </style>
</head>
<body>
  <div class="card">
    <div class="badge">✓</div>
    <h2>Yetkilendirme Alındı!</h2>
    <p>Hesap ekleme işleminin sonucunu <strong>Codex Tool</strong> üzerinden görebilirsiniz.</p>
    <button onclick="window.close()">Pencereyi Kapat</button>
  </div>
</body>
</html>"#;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html_body.len(),
        html_body
    );

    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    drop(stream);
    drop(listener);

    // Exchange code for tokens via HTTP POST to auth.openai.com
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("HTTP istemcisi oluşturulamadı: {}", e))?;

    let res = client
        .post("https://auth.openai.com/oauth/token")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", OAUTH_CLIENT_ID),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", OAUTH_REDIRECT_URI),
        ])
        .send()
        .await
        .map_err(|e| format!("Token sunucusuna bağlanılamadı: {}", e))?;

    if !res.status().is_success() {
        let status_code = res.status();
        let err_text = res.text().await.unwrap_or_default();
        return Err(format!(
            "Token takası başarısız oldu ({}): {}",
            status_code, err_text
        ));
    }

    let token_json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Token JSON formatı okunamadı: {}", e))?;

    let access_token = token_json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("access_token bulunamadı")?
        .to_string();

    let refresh_token = token_json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .ok_or("refresh_token bulunamadı")?
        .to_string();

    let expires_in = token_json
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(3600);

    let id_token = token_json
        .get("id_token")
        .and_then(|v| v.as_str())
        .ok_or("id_token bulunamadı.")?
        .to_owned();
    let access_claims = token_claims(&access_token).unwrap_or_default();
    let id_claims = token_claims(&id_token).unwrap_or_default();
    let auth = &access_claims["https://api.openai.com/auth"];
    let id_auth = &id_claims["https://api.openai.com/auth"];
    let email = access_claims["https://api.openai.com/profile"]["email"]
        .as_str()
        .or_else(|| id_claims["email"].as_str())
        .or_else(|| access_claims["email"].as_str())
        .ok_or("Hesap e-posta adresi bulunamadı.")?
        .to_owned();
    let account_id = auth["chatgpt_account_id"]
        .as_str()
        .or_else(|| id_auth["chatgpt_account_id"].as_str())
        .ok_or("ChatGPT hesap kimliği bulunamadı.")?
        .to_owned();
    let plan_type = auth["chatgpt_plan_type"]
        .as_str()
        .or_else(|| id_auth["chatgpt_plan_type"].as_str())
        .unwrap_or("unknown")
        .to_owned();

    Ok(OAuthTokenResult {
        access_token,
        id_token,
        refresh_token,
        expires_in,
        email,
        account_id,
        plan_type,
    })
}

pub fn token_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub async fn refresh_tokens(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<serde_json::Value, String> {
    let response = client
        .post("https://auth.openai.com/oauth/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", OAUTH_CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|_| "Oturum yenileme sunucusuna bağlanılamadı.")?;
    if !response.status().is_success() {
        return Err(format!(
            "Oturum yenilenemedi (HTTP {}). Hesabı yeniden ekleyin.",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .await
        .map_err(|_| "Oturum yenileme yanıtı okunamadı.".to_string())
}

fn urlencoding_helper(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}
