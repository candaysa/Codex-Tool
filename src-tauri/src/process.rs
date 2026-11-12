use std::path::PathBuf;
use std::process::Command;

pub async fn reload_codex_config(codex_home: &std::path::Path) -> Result<(), String> {
    use std::{process::Stdio, time::Duration};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let executable = find_codex_binary().ok_or("Codex bulunamadı.")?;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(["app-server", "proxy"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut input = child.stdin.take().ok_or("Codex bağlantısı açılamadı.")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("Codex yanıtı okunamadı.")?).lines();
    tokio::time::timeout(Duration::from_secs(5), async {
        input.write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"codex-tool\",\"version\":\"1.0.0\"}}}\n").await.map_err(|e| e.to_string())?;
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let Ok(reply) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if reply["id"] == 1 && reply.get("result").is_some() {
                input.write_all(b"{\"method\":\"initialized\"}\n{\"id\":2,\"method\":\"config/batchWrite\",\"params\":{\"edits\":[],\"reloadUserConfig\":true}}\n").await.map_err(|e| e.to_string())?;
            } else if reply["id"] == 2 && reply.get("result").is_some() {
                return Ok(());
            } else if reply.get("error").is_some() {
                return Err("Codex sunucusu canlı yenilemeyi kabul etmedi.".to_owned());
            }
        }
        Err("Codex canlı kontrol bağlantısı kullanılamıyor.".to_owned())
    }).await.map_err(|_| "Codex canlı yenilemesi zaman aşımına uğradı.".to_owned())?
}

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Locates the real official Codex executable binary
pub fn find_codex_binary() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("CODEX_TOOL_REAL_CODEX_BIN") {
        let p = PathBuf::from(val.trim());
        if p.exists() {
            return Some(p);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let bin_dir = PathBuf::from(local_app_data)
                .join("OpenAI")
                .join("Codex")
                .join("bin");
            if bin_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&bin_dir) {
                    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
                    for entry in entries.flatten() {
                        let sub_exe = entry.path().join("codex.exe");
                        if sub_exe.exists() {
                            let mtime = sub_exe
                                .metadata()
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::UNIX_EPOCH);
                            candidates.push((sub_exe, mtime));
                        }
                    }
                    candidates.sort_by(|a, b| b.1.cmp(&a.1));
                    if let Some((best, _)) = candidates.into_iter().next() {
                        return Some(best);
                    }
                }
            }
        }
    }

    // Try PATH
    if let Ok(output) = Command::new(if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    })
    .arg("codex")
    .output()
    {
        if output.status.success() {
            if let Ok(s) = String::from_utf8(output.stdout) {
                if let Some(first_line) = s.lines().next() {
                    let p = PathBuf::from(first_line.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }

    None
}

/// Dispatches a continue message to the official Codex CLI queue runner
pub fn dispatch_codex_queue(thread_id: &str, message: &str) -> Result<String, String> {
    let codex_bin = find_codex_binary().ok_or_else(|| "codex.exe bulunamadı.".to_string())?;

    #[cfg(target_os = "windows")]
    let output = Command::new(&codex_bin)
        .args([
            "queue",
            "--thread",
            thread_id,
            &format!("--message={}", message),
            "--resume",
        ])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("codex queue başlatılamadı: {}", e))?;

    #[cfg(not(target_os = "windows"))]
    let output = Command::new(&codex_bin)
        .args([
            "queue",
            "--thread",
            thread_id,
            &format!("--message={}", message),
            "--resume",
        ])
        .output()
        .map_err(|e| format!("codex queue başlatılamadı: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() || stdout.contains("Queued message") {
        crate::terminal::log_terminal(
            "RESUME",
            &format!(
                "[Auto-Resume] Mesaj kuyruğa eklendi: '{}' -> Thread {}",
                message, thread_id
            ),
        );
        Ok(stdout)
    } else {
        crate::terminal::log_terminal(
            "WARN",
            &format!("[Auto-Resume] codex queue uyarısı: {} {}", stdout, stderr),
        );
        Err(format!("codex queue başarısız oldu: {} {}", stdout, stderr))
    }
}
