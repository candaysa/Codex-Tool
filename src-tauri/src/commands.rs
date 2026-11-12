use rusqlite::{Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub const RUNTIME_PROXY_PROVIDER_ID: &str = "codex-tool-proxy";
pub const ORIGINAL_CONFIG_BACKUP_NAME: &str = "config.toml.original.bak";
const PROXY_DISABLED_MARKER: &str = ".codex-tool-proxy-disabled";
const NATIVE_PROXY_MARKER: &str = ".codex-tool-native-proxy";
const MANAGED_MODEL_CATALOG: &str = "accounts/codex-tool-model-catalog.json";

static ACCOUNT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LOGIN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn quota_key(account: &Value) -> String {
    json!([account["accountId"], account["email"]]).to_string()
}

fn read_json(path: &Path) -> Result<Value, String> {
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    write_atomic(
        path,
        &serde_json::to_string_pretty(value).map_err(|e| e.to_string())?,
    )
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EngineStatusResponse {
    pub is_watching: bool,
    pub is_paused: bool,
    pub waiting_count: u32,
    pub recovering_count: u32,
    pub is_available: bool,
    pub status_text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GoalItem {
    pub thread_id: String,
    pub goal_id: String,
    pub objective: String,
    pub status: String,
    pub status_badge: String,
    pub time_until_reset: String,
    pub auto_resume: bool,
    pub tokens_used: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigBindStatus {
    pub is_bound: bool,
    pub has_backup: bool,
    pub config_path: String,
    pub backup_path: String,
    pub active_provider: Option<String>,
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn get_codex_home() -> PathBuf {
    if let Some(val) = non_empty_env("CODEX_HOME") {
        return PathBuf::from(val);
    }
    let home = non_empty_env("USERPROFILE")
        .or_else(|| non_empty_env("HOME"))
        .unwrap_or_else(|| ".".to_string());
    PathBuf::from(home).join(".codex")
}

fn parse_db_generation(file_name: &str, prefix: &str) -> Option<u32> {
    let rest = file_name.strip_prefix(prefix)?;
    let num_part = rest.strip_suffix(".sqlite")?;
    num_part.parse::<u32>().ok()
}

/// Discovers candidate SQLite databases in ~/.codex, prioritizing `goals_*.sqlite`
/// (sorted numerically descending by generation) followed by `state_*.sqlite`.
fn find_candidate_sqlite_dbs(codex_home: &Path) -> Vec<PathBuf> {
    let mut goal_dbs: Vec<(u32, PathBuf)> = Vec::new();
    let mut state_dbs: Vec<(u32, PathBuf)> = Vec::new();

    if !codex_home.exists() {
        return Vec::new();
    }

    if let Ok(entries) = fs::read_dir(codex_home) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
                if let Some(gen_num) = parse_db_generation(file_name, "goals_") {
                    goal_dbs.push((gen_num, path));
                } else if let Some(gen_num) = parse_db_generation(file_name, "state_") {
                    state_dbs.push((gen_num, path));
                }
            }
        }
    }

    goal_dbs.sort_by(|a, b| b.0.cmp(&a.0));
    state_dbs.sort_by(|a, b| b.0.cmp(&a.0));

    goal_dbs
        .into_iter()
        .chain(state_dbs.into_iter())
        .map(|(_, path)| path)
        .collect()
}

#[tauri::command]
pub async fn get_engine_status() -> EngineStatusResponse {
    let output = crate::plugin::recovery_command(vec!["status".into()]).await;
    let raw = output.as_deref().unwrap_or_default();
    let read_field = |name: &str| {
        raw.lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.trim() == name)
            .map(|(_, value)| value.trim())
    };
    let is_available = read_field("auto-resume").is_some();
    let is_paused = read_field("auto-resume") != Some("ENABLED");
    let is_watching = read_field("watcher") == Some("running");
    let status = if !is_available {
        "Kurtarma motoruna erişilemiyor"
    } else if is_paused {
        "Durduruldu"
    } else if is_watching {
        "İzlemede"
    } else {
        "İzleyici çalışmıyor"
    };
    EngineStatusResponse {
        is_watching,
        is_paused,
        is_available,
        waiting_count: read_field("pending")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        recovering_count: raw
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                match fields.next()? {
                    "submitting" | "queued" => fields.next()?.parse::<u32>().ok(),
                    _ => None,
                }
            })
            .sum(),
        status_text: status.to_string(),
    }
}

#[tauri::command]
pub async fn set_engine_paused(paused: bool) -> Result<bool, String> {
    let command = if paused { "disable" } else { "enable" };
    crate::plugin::recovery_command(vec![command.to_owned()]).await?;
    Ok(paused)
}

#[tauri::command]
pub async fn list_thread_goals() -> Vec<GoalItem> {
    let output = crate::plugin::recovery_command(vec!["pending".into(), "--json".into()]).await;
    let records: Vec<Value> = output
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();

    // Group records by thread_id, picking the latest detected interruption per thread
    let mut by_thread: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for record in records {
        if let Some(tid) = record["thread_id"].as_str() {
            let replace = match by_thread.get(tid) {
                None => true,
                Some(prev) => {
                    let d_prev = prev["detected_at"].as_str().unwrap_or_default();
                    let d_cur = record["detected_at"].as_str().unwrap_or_default();
                    d_cur >= d_prev
                }
            };
            if replace {
                by_thread.insert(tid.to_string(), record);
            }
        }
    }

    by_thread
        .into_values()
        .filter_map(|record| {
            let thread_id = record["thread_id"].as_str()?.to_owned();
            let status = record["state"].as_str().unwrap_or_default();
            let reset_raw = record["next_retry_at"]
                .as_str()
                .or_else(|| record["reset_at"].as_str())
                .unwrap_or_default();

            // Strip trailing timezone offsets (+03:00, Z) according to User Rule #3
            let reset_clean = reset_raw
                .split('+')
                .next()
                .unwrap_or(reset_raw)
                .trim_end_matches('Z')
                .trim()
                .to_string();

            Some(GoalItem {
                goal_id: record["interruption_id"].as_str()?.to_owned(),
                objective: format!("Konuşma {}", thread_id.chars().take(8).collect::<String>()),
                thread_id,
                status: status.to_owned(),
                status_badge: if status == "waiting_reset" {
                    "Kullanım sınırı"
                } else {
                    "Kurtarma bekliyor"
                }
                .to_owned(),
                time_until_reset: reset_clean,
                auto_resume: record["thread_enabled"].as_bool().unwrap_or(false),
                tokens_used: 0,
                updated_at_ms: 0,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn reactivate_goal(thread_id: String) -> Result<String, String> {
    let codex_home = get_codex_home();
    let dbs = find_candidate_sqlite_dbs(&codex_home);
    let mut updated = false;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    for db_path in &dbs {
        if let Ok(mut conn) = Connection::open(db_path) {
            let _ = conn.busy_timeout(Duration::from_millis(5000));
            if let Ok(tx) = conn.transaction_with_behavior(TransactionBehavior::Immediate) {
                let res = tx.execute(
                    "UPDATE thread_goals SET status = 'active', updated_at_ms = ?1 WHERE thread_id = ?2",
                    rusqlite::params![now, thread_id],
                );
                if let Ok(count) = res {
                    if count > 0 {
                        let _ = tx.execute(
                            "DELETE FROM thread_goal_continuation_deferrals WHERE thread_id = ?1",
                            [&thread_id],
                        );
                        if tx.commit().is_ok() {
                            updated = true;
                        }
                    }
                }
            }
        }
    }

    // Dispatch official codex queue continuation so CLI/Desktop session wakes up immediately
    let message = crate::plugin::continuation_message(None)
        .await
        .unwrap_or_default();
    let message = if message.is_empty() {
        "continue"
    } else {
        &message
    };
    let queue_res = crate::process::dispatch_codex_queue(&thread_id, message);

    if queue_res.is_ok() {
        crate::terminal::log_terminal(
            "RESUME",
            &format!(
                "Konuşma ({}) aktifleştirildi ve devam sinyali iletildi.",
                thread_id
            ),
        );
        Ok(format!("Görev ({}) başarıyla aktifleştirildi.", thread_id))
    } else if updated {
        Err("Görev aktifleştirildi ancak Codex devam mesajını kabul etmedi.".to_owned())
    } else {
        let err_msg = queue_res
            .err()
            .unwrap_or_else(|| "Bilinmeyen hata".to_string());
        Err(format!(
            "Görev güncellenemedi veya kuyruğa eklenemedi: {} ({})",
            thread_id, err_msg
        ))
    }
}

#[tauri::command]
pub async fn toggle_auto_resume(task_id: String, enabled: bool) -> Result<bool, String> {
    let command = if enabled { "enable" } else { "disable" };
    crate::plugin::recovery_command(vec![command.to_owned(), "--".to_owned(), task_id]).await?;
    Ok(enabled)
}

#[tauri::command]
pub async fn get_config_bind_status() -> ConfigBindStatus {
    let codex_home = get_codex_home();
    let config_path = codex_home.join("config.toml");
    let backup_path = codex_home.join(ORIGINAL_CONFIG_BACKUP_NAME);

    let has_backup = backup_path.exists();
    let exists = config_path.exists();

    if !exists {
        return ConfigBindStatus {
            is_bound: false,
            has_backup,
            config_path: config_path.to_string_lossy().to_string(),
            backup_path: backup_path.to_string_lossy().to_string(),
            active_provider: None,
        };
    }

    let content = fs::read_to_string(&config_path).unwrap_or_default();

    let doc = content.parse::<toml_edit::DocumentMut>().ok();
    let active_provider = doc
        .as_ref()
        .and_then(|doc| doc.get("model_provider")?.as_str().map(str::to_owned));
    let configured_port = doc.as_ref().and_then(configured_proxy_port);
    let is_bound = configured_port.is_some()
        && crate::proxy::running_port() == configured_port
        && !codex_home.join(PROXY_DISABLED_MARKER).exists();

    ConfigBindStatus {
        is_bound,
        has_backup,
        config_path: config_path.to_string_lossy().to_string(),
        backup_path: backup_path.to_string_lossy().to_string(),
        active_provider,
    }
}

pub fn local_proxy_key(codex_home: &Path) -> Result<String, String> {
    let directory = codex_home.join("accounts");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join("proxy-key");
    if path.exists() {
        let key = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(key);
        }
        return Err("Yerel proxy anahtarı okunamadı.".to_owned());
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    let key: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => file.write_all(key.as_bytes()).map_err(|e| e.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return local_proxy_key(codex_home);
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(key)
}

pub fn is_local_proxy_key(codex_home: &Path, incoming: &str) -> bool {
    use sha2::{Digest, Sha256};
    let stored = fs::read(codex_home.join("accounts/proxy-key")).ok();
    stored.is_some_and(|stored| {
        let expected = Sha256::digest(&stored);
        let actual = Sha256::digest(incoming.as_bytes());
        expected
            .iter()
            .zip(actual.iter())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
    })
}

fn configured_proxy_port(doc: &toml_edit::DocumentMut) -> Option<u16> {
    if let Some(base_url) = doc.get("openai_base_url").and_then(|v| v.as_str()) {
        let rest = base_url
            .strip_prefix("http://127.0.0.1:")
            .or_else(|| base_url.strip_prefix("http://localhost:"));
        if let Some(rest) = rest {
            let port_str = rest.strip_suffix("/v1").unwrap_or(rest).trim_matches('/');
            if let Ok(port) = port_str.parse::<u16>() {
                if port != 0 {
                    return Some(port);
                }
            }
        }
    }
    doc.get("model_providers")
        .and_then(|p| p.get(RUNTIME_PROXY_PROVIDER_ID))
        .and_then(|p| p.get("base_url"))
        .and_then(|v| v.as_str())
        .and_then(|url| {
            url.strip_prefix("http://127.0.0.1:")
                .or_else(|| url.strip_prefix("http://localhost:"))
        })
        .map(|rest| rest.strip_suffix("/v1").unwrap_or(rest).trim_matches('/'))
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|port| *port != 0)
}

#[cfg(test)]
fn proxy_config(
    content: &str,
    port: Option<u16>,
    previous_provider: Option<toml_edit::Item>,
) -> Result<String, String> {
    proxy_config_with_key(content, port, previous_provider, "test-configuration-only")
}

pub(crate) fn proxy_config_with_key(
    content: &str,
    port: Option<u16>,
    previous_provider: Option<toml_edit::Item>,
    key: &str,
) -> Result<String, String> {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc = content
        .parse::<DocumentMut>()
        .map_err(|e| format!("config.toml okunamadı: {e}"))?;

    // Always clean up legacy codex-tool-proxy block if present
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_like_mut)
    {
        providers.remove(RUNTIME_PROXY_PROVIDER_ID);
        if providers.is_empty() {
            doc.remove("model_providers");
        }
    }

    if let Some(port) = port {
        // Threads created before the provider switch may still select OpenAI.
        // Keep their traffic on the same managed router.
        doc["openai_base_url"] = value(crate::proxy::base_url(port));
        doc["model_provider"] = value(RUNTIME_PROXY_PROVIDER_ID);
        doc["experimental_realtime_ws_base_url"] = value(format!("ws://127.0.0.1:{port}/v1"));
        let mut provider = Table::new();
        provider.insert("name", value("Codex Tool"));
        provider.insert("base_url", value(crate::proxy::base_url(port)));
        provider.insert("wire_api", value("responses"));
        provider.insert("requires_openai_auth", value(false));
        provider.insert("experimental_bearer_token", value(key));
        provider.insert("supports_websockets", value(false));
        doc["model_providers"][RUNTIME_PROXY_PROVIDER_ID] = Item::Table(provider);
    } else {
        doc.remove("openai_base_url");
        doc.remove("experimental_realtime_ws_base_url");
        if let Some(provider) = previous_provider {
            if provider.as_str() != Some(RUNTIME_PROXY_PROVIDER_ID) {
                doc["model_provider"] = provider;
            } else {
                doc["model_provider"] = value("openai");
            }
        } else {
            doc["model_provider"] = value("openai");
        }
    }
    Ok(doc.to_string())
}

#[cfg(test)]
fn auto_sanitize_legacy_config(codex_home: &Path) -> Result<bool, String> {
    let config_path = codex_home.join("config.toml");
    let backup_path = codex_home.join(ORIGINAL_CONFIG_BACKUP_NAME);
    if backup_path.exists() {
        if let Ok(bak_content) = fs::read_to_string(&backup_path) {
            if bak_content.contains(RUNTIME_PROXY_PROVIDER_ID) {
                if let Ok(sanitized_bak) = proxy_config(&bak_content, None, None) {
                    let _ = write_atomic(&backup_path, &sanitized_bak);
                }
            }
        }
    }
    if !config_path.exists() {
        return Ok(false);
    }
    let content = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    let mut doc = match content.parse::<toml_edit::DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => return Ok(false),
    };
    let mut modified = false;

    let legacy_port = doc
        .get("model_providers")
        .and_then(|p| p.get(RUNTIME_PROXY_PROVIDER_ID))
        .and_then(|p| p.get("base_url"))
        .and_then(|v| v.as_str())
        .and_then(|url| {
            url.strip_prefix("http://127.0.0.1:")
                .or_else(|| url.strip_prefix("http://localhost:"))
        })
        .map(|rest| rest.strip_suffix("/v1").unwrap_or(rest).trim_matches('/'))
        .and_then(|p| p.parse::<u16>().ok());

    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        if providers.contains_key(RUNTIME_PROXY_PROVIDER_ID) {
            providers.remove(RUNTIME_PROXY_PROVIDER_ID);
            if providers.is_empty() {
                doc.remove("model_providers");
            }
            modified = true;
        }
    }

    if doc.get("model_provider").and_then(toml_edit::Item::as_str)
        == Some(RUNTIME_PROXY_PROVIDER_ID)
    {
        doc["model_provider"] = toml_edit::value("openai");
        modified = true;
        if let Some(port) = legacy_port {
            if doc.get("openai_base_url").is_none() {
                doc["openai_base_url"] = toml_edit::value(crate::proxy::base_url(port));
            }
        }
    }

    if modified {
        write_atomic(&config_path, &doc.to_string())?;
    }
    Ok(modified)
}

pub fn restore_proxy_on_startup() {
    let codex_home = get_codex_home();
    if codex_home.join(PROXY_DISABLED_MARKER).exists() {
        return;
    }
    let before = fs::read_to_string(codex_home.join("config.toml")).ok();
    match update_proxy_config(&codex_home, true) {
        Ok(_) if before != fs::read_to_string(codex_home.join("config.toml")).ok() => {
            tauri::async_runtime::spawn(async move {
                match crate::process::reload_codex_config(&codex_home).await {
                    Ok(()) => {
                        crate::terminal::log_terminal("INFO", "Codex ayarları canlı yenilendi.")
                    }
                    Err(error) => crate::terminal::log_terminal(
                        "WARN",
                        &format!(
                            "{error} Yeni bağlantı ayarı Codex yeniden açıldığında uygulanır."
                        ),
                    ),
                }
            });
        }
        Ok(_) => {}
        Err(error) => {
            crate::terminal::log_terminal("ERROR", &format!("Codex entegrasyonu: {error}"))
        }
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("Dosyanın üst dizini bulunamadı.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn reset_codex_integration() -> Result<String, String> {
    let home = get_codex_home();
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    write_atomic(&home.join(PROXY_DISABLED_MARKER), "")?;
    update_proxy_config(&home, false)?;
    Ok(match crate::process::reload_codex_config(&home).await {
        Ok(()) => "Codex Tool bağlantısı kaldırıldı. Codex ayarları yenilendi.",
        Err(_) => "Codex Tool bağlantısı kaldırıldı. Ayar Codex yeniden açıldığında uygulanır.",
    }
    .to_owned())
}

#[tauri::command]
pub async fn enable_codex_integration(keep_existing_chats: bool) -> Result<String, String> {
    let codex_home = get_codex_home();
    update_proxy_binding(&codex_home, true, keep_existing_chats)?;
    let marker = codex_home.join(NATIVE_PROXY_MARKER);
    if keep_existing_chats {
        write_atomic(&marker, "")?;
    } else if marker.exists() {
        fs::remove_file(marker).map_err(|e| e.to_string())?;
    }
    let disabled = codex_home.join(PROXY_DISABLED_MARKER);
    if disabled.exists() {
        fs::remove_file(disabled).map_err(|e| e.to_string())?;
    }
    let mode = if keep_existing_chats {
        "Eski sohbetler için OpenAI bağlantısı"
    } else {
        "Profil gizleyen bağlantı"
    };
    Ok(
        match crate::process::reload_codex_config(&codex_home).await {
            Ok(()) => format!("{mode} kaydedildi. Codex ayarları yenilendi."),
            Err(_) => format!("{mode} kaydedildi. Codex’i yeniden açın."),
        },
    )
}

fn cached_model_catalog(codex_home: &Path) -> Option<Value> {
    ["models_cache.codex_work_desktop.json", "models_cache.json"]
        .into_iter()
        .filter_map(|name| read_json(&codex_home.join(name)).ok())
        .find_map(|cache| {
            let models = cache["models"].as_array()?;
            models
                .iter()
                .any(|model| model["slug"].as_str().is_some() && model["visibility"] == "list")
                .then(|| json!({"models": models}))
        })
}

fn update_proxy_config(codex_home: &Path, enabled: bool) -> Result<Option<u16>, String> {
    update_proxy_binding(
        codex_home,
        enabled,
        codex_home.join(NATIVE_PROXY_MARKER).exists(),
    )
}

fn update_proxy_binding(
    codex_home: &Path,
    enabled: bool,
    keep_existing_chats: bool,
) -> Result<Option<u16>, String> {
    static CONFIG_LOCK: Mutex<()> = Mutex::new(());
    let _guard = CONFIG_LOCK.lock().map_err(|e| e.to_string())?;
    let config_path = codex_home.join("config.toml");
    let backup_path = codex_home.join(ORIGINAL_CONFIG_BACKUP_NAME);
    let content = match fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| e.to_string())?;
    let was_bound = configured_proxy_port(&doc).is_some()
        || doc.get("model_provider").and_then(toml_edit::Item::as_str)
            == Some(RUNTIME_PROXY_PROVIDER_ID);
    let previous_provider = if !enabled && was_bound && backup_path.exists() {
        fs::read_to_string(&backup_path)
            .map_err(|e| e.to_string())?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| e.to_string())?
            .get("model_provider")
            .filter(|provider| provider.as_str() != Some(RUNTIME_PROXY_PROVIDER_ID))
            .cloned()
    } else {
        None
    };
    let port = if enabled {
        Some(crate::proxy::ensure_started(
            configured_proxy_port(&doc).unwrap_or(crate::proxy::DEFAULT_PORT),
        )?)
    } else {
        None
    };
    if !enabled && !was_bound {
        return Ok(None);
    }
    let key = if enabled {
        local_proxy_key(codex_home)?
    } else {
        String::new()
    };
    let mut updated = proxy_config_with_key(&content, port, previous_provider, &key)?;
    let catalog_path = codex_home.join(MANAGED_MODEL_CATALOG);
    let catalog_path_string = catalog_path.to_string_lossy();
    let mut binding = updated
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| e.to_string())?;
    let owns_catalog = binding
        .get("model_catalog_json")
        .and_then(toml_edit::Item::as_str)
        == Some(catalog_path_string.as_ref());
    if !enabled || keep_existing_chats {
        if owns_catalog {
            binding.remove("model_catalog_json");
        }
        if let Some(port) = port {
            binding = proxy_config_with_key(&binding.to_string(), None, None, "")?
                .parse::<toml_edit::DocumentMut>()
                .map_err(|e| e.to_string())?;
            binding["openai_base_url"] = toml_edit::value(crate::proxy::base_url(port));
            binding["experimental_realtime_ws_base_url"] =
                toml_edit::value(format!("ws://127.0.0.1:{port}/v1"));
        }
    } else if !binding.contains_key("model_catalog_json") || owns_catalog {
        if let Some(catalog) = cached_model_catalog(codex_home) {
            write_json(&catalog_path, &catalog)?;
            binding["model_catalog_json"] = toml_edit::value(catalog_path_string.as_ref());
        } else if owns_catalog && !catalog_path.is_file() {
            binding.remove("model_catalog_json");
        }
    }
    updated = binding.to_string();
    if !enabled && backup_path.exists() {
        let backup = fs::read_to_string(&backup_path).map_err(|e| e.to_string())?;
        let prior = backup
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| e.to_string())?;
        let mut restored = updated
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| e.to_string())?;
        for name in ["openai_base_url", "experimental_realtime_ws_base_url"] {
            if let Some(value) = prior.get(name) {
                restored[name] = value.clone();
            }
        }
        updated = restored.to_string();
    }
    if enabled {
        fs::create_dir_all(codex_home).map_err(|e| e.to_string())?;
        if !was_bound || !backup_path.exists() {
            let original = if was_bound {
                proxy_config_with_key(&content, None, None, "")?
            } else {
                content.clone()
            };
            write_atomic(&backup_path, &original)?;
        }
    }
    if updated != content {
        write_atomic(&config_path, &updated)?;
    }
    Ok(port)
}

pub fn get_app_backup_dir() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("src-tauri").exists() || cwd.join("package.json").exists() {
            let dir = cwd.join("backups");
            let _ = fs::create_dir_all(&dir);
            return dir;
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let dir = parent.join("backups");
            let _ = fs::create_dir_all(&dir);
            return dir;
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        let dir = cwd.join("backups");
        let _ = fs::create_dir_all(&dir);
        return dir;
    }

    let fallback = get_codex_home().join("accounts").join("backups");
    let _ = fs::create_dir_all(&fallback);
    fallback
}

#[tauri::command]
pub async fn export_backup() -> Result<String, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let codex_home = get_codex_home();
    let accounts_path = get_accounts_storage_path(&codex_home);
    let quota_path = get_quota_storage_path(&codex_home);

    let backup_dir = get_app_backup_dir();

    let now = std::time::SystemTime::now();
    let ts = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let accounts_val: serde_json::Value = if accounts_path.exists() {
        fs::read_to_string(&accounts_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };

    let quota_val: serde_json::Value = if quota_path.exists() {
        fs::read_to_string(&quota_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };

    let config_val: Option<String> = {
        let config_path = codex_home.join("config.toml");
        if config_path.exists() {
            fs::read_to_string(&config_path).ok()
        } else {
            None
        }
    };

    let full_backup = serde_json::json!({
        "app": "Codex Tool",
        "version": 1,
        "exportedAt": ts,
        "accounts": accounts_val.get("accounts").unwrap_or(&serde_json::json!([])),
        "activeIndex": accounts_val.get("activeIndex").unwrap_or(&serde_json::json!(0)),
        "quota": quota_val,
        "configToml": config_val
    });

    let filename = format!("codex_tool_backup_{}.json", ts);
    let file_path = backup_dir.join(&filename);
    let rendered = serde_json::to_string_pretty(&full_backup).map_err(|e| e.to_string())?;
    write_atomic(&file_path, &rendered)?;

    Ok(format!("Yedek oluşturuldu: backups/{}", filename))
}

#[tauri::command]
pub async fn import_backup(backup_json: String) -> Result<String, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let val: serde_json::Value =
        serde_json::from_str(&backup_json).map_err(|e| format!("Geçersiz yedek JSON: {}", e))?;

    let accounts_to_import = if let Some(accs) = val.get("accounts").and_then(|a| a.as_array()) {
        accs.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        return Err("Seçilen dosyada hesap listesi bulunamadı.".to_string());
    };

    if accounts_to_import.is_empty() {
        return Err("Yedek dosyasında içe aktarılacak hesap bulunamadı.".to_string());
    }

    let codex_home = get_codex_home();
    let accounts_path = get_accounts_storage_path(&codex_home);

    // Save safety snapshot in app backups directory
    let backup_dir = get_app_backup_dir();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if accounts_path.exists() {
        let _ = fs::copy(
            &accounts_path,
            backup_dir.join(format!("pre_import_safety_{}.bak", ts)),
        );
    }

    let active_index = val.get("activeIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let safe_active = std::cmp::min(active_index, accounts_to_import.len() - 1);

    let new_storage = serde_json::json!({
        "version": 3,
        "accounts": accounts_to_import,
        "activeIndex": safe_active
    });

    let rendered = serde_json::to_string_pretty(&new_storage).map_err(|e| e.to_string())?;
    write_atomic(&accounts_path, &rendered)?;

    // If quota cache is present in backup, restore it too
    if let Some(quota_val) = val.get("quota") {
        if !quota_val.is_null() {
            let quota_path = get_quota_storage_path(&codex_home);
            if let Ok(rendered_q) = serde_json::to_string_pretty(quota_val) {
                let _ = write_atomic(&quota_path, &rendered_q);
            }
        }
    }

    Ok(format!(
        "{} hesap başarıyla geri yüklendi!",
        accounts_to_import.len()
    ))
}

#[tauri::command]
pub async fn open_backups_folder() -> Result<String, String> {
    let backup_dir = get_app_backup_dir();
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(&backup_dir)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&backup_dir).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&backup_dir)
            .spawn();
    }
    Ok(format!("{:?}", backup_dir))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccountQuotaWindow {
    pub used_percent: u32,
    pub window_minutes: u32,
    pub reset_at_ms: Option<u64>,
    pub is_exhausted: bool,
    pub reset_time_str: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccountModel {
    pub index: usize,
    pub id: String,
    pub email: String,
    pub label: String,
    pub plan_type: String,
    pub is_active: bool,
    pub tier1_5h: Option<AccountQuotaWindow>,
    pub tier2_weekly: Option<AccountQuotaWindow>,
    pub status: String,
    pub last_used: Option<u64>,
}

fn format_reset_duration(reset_at_ms: Option<u64>, now_ms: u64) -> String {
    let reset_at = match reset_at_ms {
        Some(t) => t,
        None => return "Bilinmiyor".to_string(),
    };

    if reset_at <= now_ms {
        return "Pencere doldu · Yenile".to_string();
    }

    let diff_ms = reset_at - now_ms;
    let total_mins = diff_ms / 60_000;

    let time_str = chrono::DateTime::from_timestamp_millis(reset_at as i64)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default();

    if total_mins < 60 {
        if !time_str.is_empty() {
            format!("{} ({} dk kaldı)", time_str, total_mins.max(1))
        } else {
            format!("{} dk kaldı", total_mins.max(1))
        }
    } else if total_mins < 1440 {
        let hours = total_mins / 60;
        let mins = total_mins % 60;
        if !time_str.is_empty() {
            format!("{} ({} sa {} dk kaldı)", time_str, hours, mins)
        } else {
            format!("{} sa {} dk kaldı", hours, mins)
        }
    } else {
        let days = total_mins / 1440;
        let hours = (total_mins % 1440) / 60;
        let date_str = chrono::DateTime::from_timestamp_millis(reset_at as i64)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%d.%m %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        if !date_str.is_empty() {
            format!("{} ({} gün {} sa kaldı)", date_str, days, hours)
        } else {
            format!("{} gün {} sa kaldı", days, hours)
        }
    }
}

fn get_accounts_storage_path(codex_home: &Path) -> PathBuf {
    let accounts_dir = codex_home.join("accounts");
    let accounts_file = accounts_dir.join("accounts.json");
    if accounts_file.exists() {
        return accounts_file;
    }

    // Auto-migrate legacy account pool on first run so existing user sessions are preserved
    let legacy_rel = format!("{}-{}", "multi", "auth");
    let legacy_file = codex_home
        .join(&legacy_rel)
        .join("openai-codex-accounts.json");
    if legacy_file.exists() {
        let _ = fs::create_dir_all(&accounts_dir);
        if let Ok(content) = fs::read_to_string(&legacy_file) {
            let _ = fs::write(&accounts_file, content);
            let legacy_quota = codex_home.join(&legacy_rel).join("quota-cache.json");
            if legacy_quota.exists() {
                let _ = fs::copy(&legacy_quota, accounts_dir.join("quota.json"));
            }
            return accounts_file;
        }
    }

    let _ = fs::create_dir_all(&accounts_dir);
    accounts_file
}

fn get_quota_storage_path(codex_home: &Path) -> PathBuf {
    let accounts_dir = codex_home.join("accounts");
    let quota_file = accounts_dir.join("quota.json");
    if quota_file.exists() {
        return quota_file;
    }
    let legacy_rel = format!("{}-{}", "multi", "auth");
    let legacy_quota = codex_home.join(&legacy_rel).join("quota-cache.json");
    if legacy_quota.exists() {
        let _ = fs::create_dir_all(&accounts_dir);
        let _ = fs::copy(&legacy_quota, &quota_file);
        return quota_file;
    }
    quota_file
}

#[cfg(test)]
fn selected_proxy_account(codex_home: &Path, incoming: &str) -> Option<(String, String)> {
    proxy_accounts(codex_home, incoming).into_iter().next()
}

pub async fn record_proxy_account(codex_home: &Path, token: &str, account_id: &str) {
    let _guard = ACCOUNT_LOCK.lock().await;
    let path = codex_home.join("accounts/accounts.json");
    let Ok(mut pool) = read_json(&path) else {
        return;
    };
    let previous = pool["activeIndex"].as_u64().unwrap_or(0);
    let Some(accounts) = pool["accounts"].as_array_mut() else {
        return;
    };
    let Some(index) = accounts.iter().position(|account| {
        account["accessToken"].as_str() == Some(token)
            && account["accountId"].as_str() == Some(account_id)
    }) else {
        return;
    };
    let email = accounts[index]["email"]
        .as_str()
        .unwrap_or("Hesap")
        .to_owned();
    accounts[index]["lastUsed"] = json!(crate::quota::now_ms());
    pool["activeIndex"] = json!(index);
    if let Err(error) = write_json(&path, &pool) {
        crate::terminal::log_terminal("WARN", &format!("Aktif hesap kaydedilemedi: {error}"));
    } else if previous != index as u64 {
        crate::terminal::log_terminal(
            "ROUTE",
            &format!(
                "Otomatik hesap geçişi: Hesap {} → Hesap {} ({email})",
                previous + 1,
                index + 1
            ),
        );
    }
}

pub fn proxy_accounts(codex_home: &Path, incoming: &str) -> Vec<(String, String)> {
    load_proxy_accounts(codex_home, incoming).unwrap_or_default()
}

fn load_proxy_accounts(codex_home: &Path, incoming: &str) -> Option<Vec<(String, String)>> {
    // A native session or a live account already in the pool can authorize routing.
    let native = read_json(&codex_home.join("auth.json")).unwrap_or_default();
    let pool = read_json(&codex_home.join("accounts/accounts.json")).ok()?;
    let index = usize::try_from(pool["activeIndex"].as_u64()?).ok()?;
    let accounts = pool["accounts"].as_array()?;
    let cache = read_json(&codex_home.join("accounts/quota.json")).unwrap_or_default();
    let now = crate::quota::now_ms();
    let native_matches = native["tokens"]["access_token"].as_str() == Some(incoming);
    let managed_matches = accounts.iter().any(|account| {
        account["accessToken"].as_str() == Some(incoming)
            && account["expiresAt"]
                .as_u64()
                .is_some_and(|expiry| expiry > now)
            && account["enabled"].as_bool() != Some(false)
            && account["disabled"].as_bool() != Some(true)
    });
    if !is_local_proxy_key(codex_home, incoming) && !native_matches && !managed_matches {
        return None;
    }
    let mut candidates: Vec<_> = accounts
        .iter()
        .enumerate()
        .filter_map(|(position, account)| {
            if account["enabled"].as_bool() == Some(false)
                || account["disabled"].as_bool() == Some(true)
            {
                return None;
            }
            let mut account = account.clone();
            crate::quota::reuse_codex_session(&mut account, &native["tokens"]);
            if account["expiresAt"]
                .as_u64()
                .is_some_and(|expiry| expiry > 0 && expiry <= now)
            {
                return None;
            }
            let token = account["accessToken"]
                .as_str()
                .filter(|value| !value.is_empty())?;
            let account_id = account["accountId"]
                .as_str()
                .filter(|value| !value.is_empty())?;
            let entry = &cache["byIdentity"][quota_key(&account)];
            let reset = ["primary", "secondary"]
                .into_iter()
                .filter_map(|name| {
                    let window = &entry[name];
                    if window["usedPercent"].as_f64()? < 100.0 {
                        return None;
                    }
                    window["resetAtMs"].as_u64().filter(|time| *time > now)
                })
                .max()
                .unwrap_or(0);
            let next_reset = ["primary", "secondary"]
                .into_iter()
                .filter_map(|name| entry[name]["resetAtMs"].as_u64().filter(|time| *time > now))
                .min()
                .unwrap_or(u64::MAX);
            Some((
                (reset, position != index, next_reset),
                (token.to_owned(), account_id.to_owned()),
            ))
        })
        .collect();
    candidates.sort_by_key(|(rank, _)| *rank);
    Some(candidates.into_iter().map(|(_, account)| account).collect())
}

pub fn sync_account_pools(codex_home: &Path) {
    let accounts_path = get_accounts_storage_path(codex_home);
    let legacy_rel = format!("{}-{}", "multi", "auth");
    let multi_auth_path = codex_home
        .join(&legacy_rel)
        .join("openai-codex-accounts.json");

    let mut tool_pool = read_json(&accounts_path).ok().filter(Value::is_object);
    let mut multi_pool = read_json(&multi_auth_path).ok().filter(Value::is_object);

    if tool_pool.is_none() && multi_pool.is_none() {
        return;
    }

    let mut tool_changed = false;
    let mut multi_changed = false;

    if let Some(ref mut tp) = tool_pool {
        if !tp["accounts"].is_array() {
            tp["accounts"] = serde_json::json!([]);
        }
    } else if let Some(ref mp) = multi_pool {
        tool_pool = Some(mp.clone());
        tool_changed = true;
    }

    if let Some(ref mut mp) = multi_pool {
        if !mp["accounts"].is_array() {
            mp["accounts"] = serde_json::json!([]);
        }
    }

    if let (Some(ref mut tp), Some(ref mut mp)) = (&mut tool_pool, &mut multi_pool) {
        if let (Some(tp_accs), Some(mp_accs)) =
            (tp["accounts"].as_array_mut(), mp["accounts"].as_array_mut())
        {
            for m_acc in mp_accs.iter() {
                let m_email = m_acc["email"].as_str().unwrap_or_default();
                let m_id = m_acc["accountId"].as_str().unwrap_or_default();
                let exists_in_tp = tp_accs.iter().any(|a| {
                    let e = a["email"].as_str().unwrap_or_default();
                    let id = a["accountId"].as_str().unwrap_or_default();
                    (!m_email.is_empty() && e.eq_ignore_ascii_case(m_email))
                        || (!m_id.is_empty() && id == m_id)
                });
                if !exists_in_tp {
                    tp_accs.push(m_acc.clone());
                    tool_changed = true;
                }
            }

            for t_acc in tp_accs.iter() {
                let t_email = t_acc["email"].as_str().unwrap_or_default();
                let t_id = t_acc["accountId"].as_str().unwrap_or_default();
                let exists_in_mp = mp_accs.iter().any(|a| {
                    let e = a["email"].as_str().unwrap_or_default();
                    let id = a["accountId"].as_str().unwrap_or_default();
                    (!t_email.is_empty() && e.eq_ignore_ascii_case(t_email))
                        || (!t_id.is_empty() && id == t_id)
                });
                if !exists_in_mp {
                    mp_accs.push(t_acc.clone());
                    multi_changed = true;
                }
            }
        }
    }

    if let Some(ref mut tp) = tool_pool {
        if let Ok(auth) = read_json(&codex_home.join("auth.json")) {
            if let Some(tokens) = auth.get("tokens").filter(|t| t.is_object()) {
                if let Some(accs) = tp["accounts"].as_array_mut() {
                    for acc in accs.iter_mut() {
                        if crate::quota::reuse_codex_session(acc, tokens) {
                            tool_changed = true;
                            break;
                        }
                    }
                }
            }
        }
    }

    if tool_changed {
        if let Some(ref tp) = tool_pool {
            let _ = fs::create_dir_all(codex_home.join("accounts"));
            let _ = write_json(&accounts_path, tp);
        }
    }

    if multi_changed {
        if let Some(ref mp) = multi_pool {
            let _ = fs::create_dir_all(codex_home.join(&legacy_rel));
            let _ = write_json(&multi_auth_path, mp);
        }
    }
}

#[tauri::command]
pub async fn list_accounts() -> Result<Vec<AccountModel>, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let codex_home = get_codex_home();
    sync_account_pools(&codex_home);
    let accounts_path = get_accounts_storage_path(&codex_home);
    let quota_path = get_quota_storage_path(&codex_home);

    if !accounts_path.exists() {
        return Ok(Vec::new());
    }

    let accounts_content = fs::read_to_string(&accounts_path).map_err(|e| e.to_string())?;
    let accounts_json: serde_json::Value =
        serde_json::from_str(&accounts_content).map_err(|e| e.to_string())?;

    let quota_json: serde_json::Value = if quota_path.exists() {
        fs::read_to_string(&quota_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or(serde_json::Value::Null)
    } else {
        serde_json::Value::Null
    };

    let active_index = accounts_json
        .get("activeIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut result = Vec::new();

    if let Some(accounts_arr) = accounts_json.get("accounts").and_then(|v| v.as_array()) {
        for (idx, acc) in accounts_arr.iter().enumerate() {
            let id = acc
                .get("accountId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let email = acc
                .get("email")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let label = acc
                .get("accountLabel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let last_used = acc.get("lastUsed").and_then(|v| v.as_u64());
            let is_active = idx == active_index;

            let quota_entry = quota_json["byIdentity"]
                .get(quota_key(acc))
                .or_else(|| quota_json["byEmail"].get(&email))
                .or_else(|| quota_json["byAccountId"].get(&id))
                .filter(|entry| entry.is_object() && !crate::quota::is_legacy_placeholder(entry));

            let plan_type = quota_entry
                .and_then(|entry| entry["planType"].as_str())
                .or_else(|| acc["planType"].as_str())
                .unwrap_or("Bilinmiyor")
                .to_string();
            let (status, tier1_5h, tier2_weekly) = if let Some(entry) = quota_entry {
                let entry_status = entry.get("status").and_then(|v| v.as_i64()).unwrap_or(200);

                let extract_window =
                    |w_obj: &serde_json::Value| -> Option<(u32, u32, Option<u64>)> {
                        let mins = w_obj
                            .get("windowMinutes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;
                        let used = w_obj.get("usedPercent")?.as_f64()?.clamp(0.0, 100.0) as u32;
                        let reset_ms = w_obj.get("resetAtMs").and_then(|v| v.as_u64());
                        if mins > 0 || used > 0 || reset_ms.is_some() {
                            Some((mins, used, reset_ms))
                        } else {
                            None
                        }
                    };

                let mut t1: Option<AccountQuotaWindow> = None;
                let mut t2: Option<AccountQuotaWindow> = None;

                for (mins, used, reset_ms) in ["primary", "secondary", "fiveHour", "weekly"]
                    .into_iter()
                    .filter_map(|key| entry.get(key).and_then(extract_window))
                {
                    let is_exhaust = used >= 100 && reset_ms.is_none_or(|reset| reset > now_ms);
                    let reset_str = if used == 0 {
                        "Kullanıma Hazır".to_string()
                    } else {
                        format_reset_duration(reset_ms, now_ms)
                    };

                    let window = AccountQuotaWindow {
                        used_percent: used,
                        window_minutes: mins,
                        reset_at_ms: reset_ms,
                        is_exhausted: is_exhaust,
                        reset_time_str: reset_str,
                    };

                    if mins <= 360 && mins > 0 && t1.is_none() {
                        t1 = Some(window);
                    } else if mins > 360 && t2.is_none() {
                        t2 = Some(window);
                    }
                }

                if entry_status != 429 {
                    // Only synthesize if BOTH are missing (brand new account)
                    // If the account only has a weekly window, honor User Rule #3: do NOT show 5h
                    if t1.is_none() && t2.is_none() {
                        t1 = Some(AccountQuotaWindow {
                            used_percent: 0,
                            window_minutes: 300,
                            reset_at_ms: None,
                            is_exhausted: false,
                            reset_time_str: "Kullanıma Hazır".to_string(),
                        });
                        t2 = Some(AccountQuotaWindow {
                            used_percent: 0,
                            window_minutes: 10080,
                            reset_at_ms: None,
                            is_exhausted: false,
                            reset_time_str: "Kullanıma Hazır".to_string(),
                        });
                    }
                }

                let s = if entry_status == 429 {
                    "Kullanım Sınırı (429)".to_string()
                } else if t1.as_ref().map_or(false, |w| w.is_exhausted)
                    || t2.as_ref().map_or(false, |w| w.is_exhausted)
                {
                    "Sınırda (Bekliyor)".to_string()
                } else if t1.as_ref().map_or(true, |w| w.used_percent == 0)
                    && t2.as_ref().map_or(true, |w| w.used_percent == 0)
                {
                    "Kullanıma Hazır".to_string()
                } else {
                    "Hazır".to_string()
                };

                (s, t1, t2)
            } else {
                let t1 = Some(AccountQuotaWindow {
                    used_percent: 0,
                    window_minutes: 300,
                    reset_at_ms: None,
                    is_exhausted: false,
                    reset_time_str: "Kullanıma Hazır".to_string(),
                });
                let t2 = Some(AccountQuotaWindow {
                    used_percent: 0,
                    window_minutes: 10080,
                    reset_at_ms: None,
                    is_exhausted: false,
                    reset_time_str: "Kullanıma Hazır".to_string(),
                });
                ("Kullanıma Hazır".to_string(), t1, t2)
            };

            result.push(AccountModel {
                index: idx,
                id,
                email,
                label,
                plan_type,
                is_active,
                tier1_5h,
                tier2_weekly,
                status,
                last_used,
            });
        }
    }

    Ok(result)
}

#[tauri::command]
pub async fn switch_account(target_index: usize) -> Result<String, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let codex_home = get_codex_home();
    let accounts_path = get_accounts_storage_path(&codex_home);
    if !accounts_path.exists() {
        return Err("Hesap dosyası bulunamadı.".to_string());
    }

    let content = fs::read_to_string(&accounts_path).map_err(|e| e.to_string())?;
    let mut val: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    let account = val["accounts"]
        .as_array()
        .and_then(|accounts| accounts.get(target_index))
        .ok_or("Geçersiz hesap seçimi.")?;
    if account["accessToken"].as_str().is_none_or(str::is_empty) {
        return Err("Hesabın erişim anahtarı eksik. Hesabı yeniden ekleyin.".to_string());
    }
    let email = account["email"].as_str().unwrap_or("Hesap").to_owned();
    val["activeIndex"] = json!(target_index);
    write_json(&accounts_path, &val)?;

    Ok(format!("{} hesabı proxy istekleri için seçildi.", email))
}

#[tauri::command]
pub async fn remove_account(target_index: usize) -> Result<String, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    let codex_home = get_codex_home();
    let accounts_path = get_accounts_storage_path(&codex_home);
    if !accounts_path.exists() {
        return Err("Hesap dosyası bulunamadı.".to_string());
    }

    let content = fs::read_to_string(&accounts_path).map_err(|e| e.to_string())?;
    let mut val: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    let backup_dir = codex_home.join("accounts").join("backups");
    let _ = fs::create_dir_all(&backup_dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = fs::copy(
        &accounts_path,
        backup_dir.join(format!("accounts.{}.bak", ts)),
    );

    let (removed_email, accounts_empty, remaining_count) = {
        let accounts = val
            .get_mut("accounts")
            .and_then(|a| a.as_array_mut())
            .ok_or("Hesap listesi bulunamadı.")?;
        if target_index >= accounts.len() {
            return Err("Geçersiz hesap indeksi.".to_string());
        }
        let removed = accounts.remove(target_index);
        let email = removed
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("Hesap")
            .to_string();
        (email, accounts.is_empty(), accounts.len())
    };

    let current_active = val.get("activeIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let new_active = if accounts_empty {
        0
    } else if current_active >= target_index && current_active > 0 {
        current_active - 1
    } else {
        std::cmp::min(current_active, remaining_count - 1)
    };
    val["activeIndex"] = serde_json::json!(new_active);

    let updated = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
    write_atomic(&accounts_path, &updated)?;

    Ok(format!(
        "{} hesabı başarıyla havuzdan kaldırıldı.",
        removed_email
    ))
}

#[tauri::command]
pub async fn add_account_login(on_url: tauri::ipc::Channel<String>) -> Result<String, String> {
    let _login = LOGIN_LOCK
        .try_lock()
        .map_err(|_| "Bir giriş işlemi zaten devam ediyor.")?;
    let token = crate::oauth::run_native_oauth_login(on_url).await?;
    let _guard = ACCOUNT_LOCK.lock().await;
    let codex_home = get_codex_home();
    let accounts_path = get_accounts_storage_path(&codex_home);
    let mut pool = if accounts_path.exists() {
        read_json(&accounts_path)?
    } else {
        json!({"version": 3, "accounts": [], "activeIndex": 0})
    };
    let now = crate::quota::now_ms();
    let accounts = pool["accounts"]
        .as_array_mut()
        .ok_or("Hesap listesi okunamadı.")?;
    let auth = read_json(&codex_home.join("auth.json")).unwrap_or_default();
    for account in accounts.iter_mut() {
        if crate::quota::reuse_codex_session(account, &auth["tokens"]) {
            break;
        }
    }
    let position = accounts.iter().position(|account| {
        account["accountId"].as_str() == Some(token.account_id.as_str())
            && account["email"]
                .as_str()
                .is_some_and(|email| email.eq_ignore_ascii_case(&token.email))
    });
    let index = position.unwrap_or(accounts.len());
    if position.is_none() {
        accounts.push(json!({
            "addedAt": now,
            "accountLabel": format!("Hesap [{}]", token.email)
        }));
    }
    let account = &mut accounts[index];
    account["accountId"] = json!(token.account_id);
    account["email"] = json!(token.email);
    account["accessToken"] = json!(token.access_token);
    account["refreshToken"] = json!(token.refresh_token);
    account["idToken"] = json!(token.id_token);
    account["expiresAt"] = json!(now.saturating_add(token.expires_in.saturating_mul(1000)));
    account["lastUsed"] = json!(now);
    account["planType"] = json!(token.plan_type);
    pool["activeIndex"] = json!(index);
    write_json(&accounts_path, &pool)?;
    match update_quota(&codex_home, Some(index)).await {
        Ok(_) => Ok(format!(
            "{} hesabı eklendi ve kotası güncellendi.",
            token.email
        )),
        Err(e) => Err(format!(
            "{} hesabı eklendi. Kota alınamadı: {e}",
            token.email
        )),
    }
}

async fn update_quota(codex_home: &Path, only_index: Option<usize>) -> Result<String, String> {
    let accounts_path = get_accounts_storage_path(codex_home);
    if !accounts_path.exists() {
        return Ok("Yenilenecek hesap yok.".to_string());
    }
    let quota_path = get_quota_storage_path(codex_home);
    let mut pool = read_json(&accounts_path)?;
    let mut cache = read_json(&quota_path)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({"version": 1}));
    if !cache["byIdentity"].is_object() {
        cache["byIdentity"] = json!({});
    }
    if !cache["byEmail"].is_object() {
        cache["byEmail"] = json!({});
    }
    if !cache["byAccountId"].is_object() {
        cache["byAccountId"] = json!({});
    }
    let auth = read_json(&codex_home.join("auth.json")).unwrap_or_default();
    let client = crate::quota::client()?;
    let mut refreshed = 0;
    let mut failures = Vec::new();
    let mut changed = false;
    let accounts = pool["accounts"]
        .as_array_mut()
        .ok_or("Hesap listesi okunamadı.")?;
    for (index, account) in accounts.iter_mut().enumerate() {
        if only_index.is_some_and(|target| index != target) {
            continue;
        }
        let old = account.clone();
        match crate::quota::refresh(&client, account, &auth["tokens"]).await {
            Ok(entry) => {
                if let Some(entries) = cache["byIdentity"].as_object_mut() {
                    entries.insert(quota_key(account), entry.clone());
                }
                if let Some(by_email) = cache["byEmail"].as_object_mut() {
                    if let Some(email) = account["email"].as_str().filter(|e| !e.is_empty()) {
                        by_email.insert(email.to_string(), entry.clone());
                    }
                }
                if let Some(by_acc) = cache["byAccountId"].as_object_mut() {
                    if let Some(id) = account["accountId"].as_str().filter(|id| !id.is_empty()) {
                        by_acc.insert(id.to_string(), entry);
                    }
                }
                refreshed += 1;
            }
            Err(error) => failures.push(format!(
                "{}: {error}",
                account["email"].as_str().unwrap_or("Hesap")
            )),
        }
        changed |= *account != old;
    }
    if changed {
        write_json(&accounts_path, &pool)?;
    }
    if refreshed > 0 {
        write_json(&quota_path, &cache)?;
    }
    if failures.is_empty() {
        Ok(format!("{refreshed} hesabın kotası güncellendi."))
    } else {
        Err(format!(
            "{refreshed} hesap güncellendi. {}",
            failures.join(" ")
        ))
    }
}

#[tauri::command]
pub async fn refresh_accounts_quota() -> Result<String, String> {
    let _guard = ACCOUNT_LOCK.lock().await;
    update_quota(&get_codex_home(), None).await
}

#[tauri::command]
pub fn get_terminal_logs(since_id: Option<u64>) -> Vec<crate::terminal::TerminalLogEntry> {
    crate::terminal::get_logs(since_id)
}

#[tauri::command]
pub fn clear_terminal_logs() {
    crate::terminal::clear_logs();
}

pub fn start_background_quota_refresher() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let mut last_error = None;
        loop {
            let codex_home = get_codex_home();
            let result = {
                let _guard = ACCOUNT_LOCK.lock().await;
                update_quota(&codex_home, None).await
            };
            match result {
                Ok(msg) => {
                    last_error = None;
                    crate::terminal::log_terminal(
                        "QUOTA",
                        &format!("Periyodik kota taraması: {msg}"),
                    );
                }
                Err(err) => {
                    if last_error.as_ref() != Some(&err) {
                        crate::terminal::log_terminal("WARN", &format!("Kota kontrolü: {err}"));
                        last_error = Some(err);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_candidate_db_discovery_and_sorting() {
        let temp_dir = std::env::temp_dir().join(format!(
            "codex_test_dbs_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let _ = File::create(temp_dir.join("goals_2.sqlite")).unwrap();
        let _ = File::create(temp_dir.join("goals_10.sqlite")).unwrap();
        let _ = File::create(temp_dir.join("state_5.sqlite")).unwrap();
        let _ = File::create(temp_dir.join("unrelated.txt")).unwrap();

        let candidates = find_candidate_sqlite_dbs(&temp_dir);
        assert_eq!(candidates.len(), 3);
        assert!(candidates[0].ends_with("goals_10.sqlite"));
        assert!(candidates[1].ends_with("goals_2.sqlite"));
        assert!(candidates[2].ends_with("state_5.sqlite"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sqlite_thread_goals_lifecycle() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE thread_goals (
                thread_id TEXT PRIMARY KEY,
                goal_id TEXT NOT NULL,
                objective TEXT NOT NULL,
                status TEXT NOT NULL,
                tokens_used INTEGER DEFAULT 0,
                updated_at_ms INTEGER DEFAULT 0
            )",
            [],
        )
        .unwrap();

        conn.execute(
            "CREATE TABLE thread_goal_continuation_deferrals (
                thread_id TEXT PRIMARY KEY
            )",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO thread_goals (thread_id, goal_id, objective, status, tokens_used, updated_at_ms)
             VALUES ('th_123', 'g_123', 'Fix infinite loop', 'usage_limited', 500, 1000)",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO thread_goal_continuation_deferrals (thread_id) VALUES ('th_123')",
            [],
        )
        .unwrap();

        let count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM thread_goals WHERE status IN ('usage_limited', 'blocked', 'paused')",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "UPDATE thread_goals SET status = 'active', updated_at_ms = 2000 WHERE thread_id = 'th_123'",
            [],
        ).unwrap();
        tx.execute(
            "DELETE FROM thread_goal_continuation_deferrals WHERE thread_id = 'th_123'",
            [],
        )
        .unwrap();
        tx.commit().unwrap();

        let active_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_goals WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active_count, 1);

        let def_count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM thread_goal_continuation_deferrals WHERE thread_id = 'th_123'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(def_count, 0);
    }

    #[tokio::test]
    async fn occupied_proxy_port_is_replaced_in_config_and_serves_requests() {
        let home = tempfile::tempdir().unwrap();
        let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let old_port = occupied.local_addr().unwrap().port();
        let config = home.path().join("config.toml");
        fs::write(&config, proxy_config("", Some(old_port), None).unwrap()).unwrap();
        write_json(
            &home.path().join("models_cache.json"),
            &json!({
                "identity": {"private": true},
                "models": [{"slug":"example","visibility":"list"}]
            }),
        )
        .unwrap();

        let new_port = update_proxy_config(home.path(), true).unwrap().unwrap();
        assert_ne!(old_port, new_port);
        let doc = fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(configured_proxy_port(&doc), Some(new_port));
        let catalog = read_json(Path::new(doc["model_catalog_json"].as_str().unwrap())).unwrap();
        assert_eq!(
            catalog,
            json!({"models":[{"slug":"example","visibility":"list"}]})
        );
        assert_eq!(
            update_proxy_config(home.path(), true).unwrap(),
            Some(new_port)
        );
        assert_eq!(
            update_proxy_binding(home.path(), true, true).unwrap(),
            Some(new_port)
        );
        let native: toml_edit::DocumentMut = fs::read_to_string(&config).unwrap().parse().unwrap();
        assert_eq!(native["model_provider"].as_str(), Some("openai"));
        assert_eq!(configured_proxy_port(&native), Some(new_port));
        assert!(!native.contains_key("model_catalog_json"));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = client
            .get(format!("{}/models", crate::proxy::base_url(new_port)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_remove_proxy_block_cleanliness() {
        let raw = r#"
model_provider = "codex-tool-proxy"

[model_providers.codex-tool-proxy]
base_url = "http://127.0.0.1:8080/v1"
wire_format = "responses"
requires_openai_auth = true

[editor]
line_numbers = true
"#;

        let migrated = proxy_config(raw, Some(49152), None).unwrap();
        let parsed = migrated.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(configured_proxy_port(&parsed), Some(49152));
        assert_eq!(
            parsed["model_provider"].as_str(),
            Some(RUNTIME_PROXY_PROVIDER_ID)
        );
        assert_eq!(
            parsed["model_providers"][RUNTIME_PROXY_PROVIDER_ID]["requires_openai_auth"].as_bool(),
            Some(false)
        );
        assert!(
            parsed["model_providers"][RUNTIME_PROXY_PROVIDER_ID]["experimental_bearer_token"]
                .as_str()
                .is_some()
        );
        assert_eq!(
            parsed["openai_base_url"].as_str(),
            Some("http://127.0.0.1:49152/v1")
        );

        let cleaned = proxy_config(&migrated, None, None).unwrap();
        assert!(!cleaned.contains("openai_base_url"));
        assert!(!cleaned.contains("codex-tool-proxy"));
        assert!(cleaned.contains("model_provider = \"openai\""));
        assert!(cleaned.contains("[editor]"));
        assert!(cleaned.contains("line_numbers = true"));
    }

    #[test]
    fn proxy_account_selection_does_not_replace_native_auth() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = home.path().join("auth.json");
        let auth = json!({"tokens":{"access_token":"native-session"}});
        write_json(&auth_path, &auth).unwrap();
        fs::create_dir(home.path().join("accounts")).unwrap();
        let pool_path = home.path().join("accounts/accounts.json");
        let mut pool = json!({"activeIndex":0,"accounts":[
            {"accountId":"first","accessToken":"first-token"},
            {"accountId":"second","accessToken":"second-token"}
        ]});
        write_json(&pool_path, &pool).unwrap();
        assert_eq!(selected_proxy_account(home.path(), "unrelated"), None);
        assert_eq!(
            selected_proxy_account(home.path(), "native-session")
                .unwrap()
                .1,
            "first"
        );
        let local_key = local_proxy_key(home.path()).unwrap();
        assert_eq!(local_key, local_proxy_key(home.path()).unwrap());
        assert_eq!(
            selected_proxy_account(home.path(), &local_key).unwrap().1,
            "first"
        );
        pool["activeIndex"] = json!(1);
        write_json(&pool_path, &pool).unwrap();
        assert_eq!(
            selected_proxy_account(home.path(), "native-session")
                .unwrap()
                .1,
            "second"
        );
        let now = crate::quota::now_ms();
        pool["accounts"][0]["expiresAt"] = json!(now + 60_000);
        pool["accounts"][1]["expiresAt"] = json!(now + 60_000);
        write_json(&pool_path, &pool).unwrap();
        assert_eq!(
            selected_proxy_account(home.path(), "first-token")
                .unwrap()
                .1,
            "second"
        );
        let second_key = quota_key(&pool["accounts"][1]);
        let cache = json!({"byIdentity": {
            (second_key): {"primary": {"usedPercent": 100, "resetAtMs": now + 30_000}}
        }});
        write_json(&home.path().join("accounts/quota.json"), &cache).unwrap();
        assert_eq!(
            selected_proxy_account(home.path(), "native-session")
                .unwrap()
                .1,
            "first"
        );
        assert_eq!(read_json(&auth_path).unwrap(), auth);
    }

    #[test]
    fn test_auto_sanitize_legacy_config() {
        let home = tempfile::tempdir().unwrap();
        let config_path = home.path().join("config.toml");
        let legacy_content = r#"
model_provider = "codex-tool-proxy"

[model_providers.codex-tool-proxy]
base_url = "http://127.0.0.1:8080/v1"
wire_format = "responses"
requires_openai_auth = true

[editor]
line_numbers = true
"#;
        fs::write(&config_path, legacy_content).unwrap();
        let modified = auto_sanitize_legacy_config(home.path()).unwrap();
        assert!(modified);
        let sanitized = fs::read_to_string(&config_path).unwrap();
        assert!(sanitized.contains("model_provider = \"openai\""));
        assert!(sanitized.contains("openai_base_url = \"http://127.0.0.1:8080/v1\""));
        assert!(!sanitized.contains("[model_providers.codex-tool-proxy]"));
        assert!(sanitized.contains("[editor]"));
        assert!(sanitized.contains("line_numbers = true"));
    }

    #[test]
    fn test_auto_sanitize_legacy_backup() {
        let home = tempfile::tempdir().unwrap();
        let backup_path = home.path().join(ORIGINAL_CONFIG_BACKUP_NAME);
        let legacy_backup = r#"
model_provider = "codex-tool-proxy"

[model_providers.codex-tool-proxy]
base_url = "http://127.0.0.1:8080/v1"

[editor]
line_numbers = true
"#;
        fs::write(&backup_path, legacy_backup).unwrap();
        let config_path = home.path().join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").unwrap();

        auto_sanitize_legacy_config(home.path()).unwrap();
        let cleaned_bak = fs::read_to_string(&backup_path).unwrap();
        assert!(!cleaned_bak.contains("codex-tool-proxy"));
        assert!(!cleaned_bak.contains("openai_base_url"));
        assert!(cleaned_bak.contains("model_provider = \"openai\""));
        assert!(cleaned_bak.contains("[editor]"));
    }

    #[tokio::test]
    async fn test_list_accounts_zero_percent_and_fallback() {
        let home = tempfile::tempdir().unwrap();
        let accounts_dir = home.path().join("accounts");
        fs::create_dir_all(&accounts_dir).unwrap();

        let pool = serde_json::json!({
            "version": 3,
            "activeIndex": 0,
            "accounts": [
                {
                    "accountId": "acc_zero",
                    "email": "zero@test.com",
                    "accountLabel": "Zero Usage",
                    "planType": "plus"
                },
                {
                    "accountId": "acc_new",
                    "email": "new@test.com",
                    "accountLabel": "New Account",
                    "planType": "plus"
                }
            ]
        });
        fs::write(accounts_dir.join("accounts.json"), pool.to_string()).unwrap();

        let quota = serde_json::json!({
            "version": 1,
            "byEmail": {
                "zero@test.com": {
                    "updatedAt": 1794078980924u64,
                    "status": 200,
                    "model": "gpt-5.6-sol",
                    "planType": "plus",
                    "primary": {
                        "usedPercent": 0,
                        "windowMinutes": 300
                    },
                    "secondary": {
                        "usedPercent": 0,
                        "windowMinutes": 10080
                    }
                }
            }
        });
        fs::write(accounts_dir.join("quota.json"), quota.to_string()).unwrap();

        std::env::set_var("CODEX_HOME", home.path().to_str().unwrap());

        let accounts = list_accounts().await.unwrap();
        assert_eq!(accounts.len(), 2);

        // Account 1: 0% usage with quota.json entry
        let acc0 = &accounts[0];
        assert_eq!(acc0.email, "zero@test.com");
        assert_eq!(acc0.status, "Kullanıma Hazır");
        let t1_0 = acc0.tier1_5h.as_ref().unwrap();
        assert_eq!(t1_0.used_percent, 0);
        assert_eq!(t1_0.reset_time_str, "Kullanıma Hazır");
        let t2_0 = acc0.tier2_weekly.as_ref().unwrap();
        assert_eq!(t2_0.used_percent, 0);
        assert_eq!(t2_0.reset_time_str, "Kullanıma Hazır");

        // Account 2: Brand new account without entry in quota.json
        let acc1 = &accounts[1];
        assert_eq!(acc1.email, "new@test.com");
        assert_eq!(acc1.status, "Kullanıma Hazır");
        let t1_1 = acc1.tier1_5h.as_ref().unwrap();
        assert_eq!(t1_1.used_percent, 0);
        assert_eq!(t1_1.reset_time_str, "Kullanıma Hazır");
        let t2_1 = acc1.tier2_weekly.as_ref().unwrap();
        assert_eq!(t2_1.used_percent, 0);
        assert_eq!(t2_1.reset_time_str, "Kullanıma Hazır");

        std::env::remove_var("CODEX_HOME");
    }
}
