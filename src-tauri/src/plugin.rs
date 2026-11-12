#[cfg(target_os = "windows")]
mod windows {
    use serde_json::{json, Value};
    use std::{
        env, fs, io,
        io::Write,
        os::windows::process::CommandExt,
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    const MARKETPLACE: &str = "candaysa-codex-tool";
    const PLUGIN: &str = "codex-tool";

    fn desktop_cli() -> Result<PathBuf, String> {
        let root = PathBuf::from(
            env::var_os("LOCALAPPDATA").ok_or("Windows uygulama dizini bulunamadı.")?,
        )
        .join("OpenAI/Codex/bin");
        fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path().join("codex.exe"))
            .filter(|path| path.is_file())
            .max_by_key(|path| path.metadata().and_then(|m| m.modified()).ok())
            .or_else(|| Some(root.join("codex.exe")).filter(|path| path.is_file()))
            .ok_or_else(|| "Codex masaüstü uygulaması bulunamadı.".to_string())
    }

    fn resume_home() -> Result<PathBuf, String> {
        for key in ["CODEX_AUTO_RESUME_PLUGIN_HOME", "CODEX_AUTO_RESUME_HOME"] {
            if let Some(path) = env::var_os(key).filter(|value| !value.is_empty()) {
                return Ok(path.into());
            }
        }
        Ok(
            PathBuf::from(env::var_os("USERPROFILE").ok_or("Kullanıcı dizini bulunamadı.")?)
                .join(".codex-auto-resume"),
        )
    }

    fn runtime_ready(root: &Path) -> bool {
        root.join("runtime/python.exe").is_file()
            && root.join("app/mcp/codex-auto-resume-mcp.exe").is_file()
            && root
                .join("app/src/codex_auto_resume/mcpserver.py")
                .is_file()
    }

    fn write_json(path: &Path, value: &Value) -> io::Result<()> {
        fs::write(path, serde_json::to_vec_pretty(value)?)
    }

    fn stage_plugin(root: &Path, runtime: &Path) -> io::Result<()> {
        let plugin = root.join("plugins").join(PLUGIN);
        let skill = plugin.join("skills/codex-tool-auto-resume");
        for dir in [
            root.join(".agents/plugins"),
            plugin.join(".codex-plugin"),
            plugin.join("mcp"),
            plugin.join("assets"),
            skill.clone(),
        ] {
            fs::create_dir_all(dir)?;
        }
        write_json(
            &plugin.join(".codex-plugin/plugin.json"),
            &json!({
                "name": PLUGIN,
                "version": env!("CARGO_PKG_VERSION"),
                "description": "Windows’ta kesintiye uğrayan Codex konuşmalarını Auto Resume motoruyla yönetir.",
                "author": {"name":"CanDaysa"},
                "skills": "./skills/",
                "mcpServers": "./.mcp.json",
                "interface": {
                    "displayName":"Codex Tool",
                    "developerName":"CanDaysa",
                    "shortDescription":"Codex görevlerini kaldığı yerden sürdür",
                    "longDescription":"Kullanım sınırı veya geçici hata nedeniyle kesilen konuşmaları Windows kurtarma motoruyla takip eder ve devam ettirir.",
                    "category":"Developer Tools",
                    "capabilities":["Interactive","Read","Write"],
                    "defaultPrompt":["Otomatik devam durumunu göster","Bekleyen kurtarmaları göster","Otomatik devam ayarlarını aç"],
                    "brandColor":"#0284C7",
                    "composerIcon":"./assets/icon.png",
                    "logo":"./assets/icon.png"
                }
            }),
        )?;
        write_json(
            &plugin.join(".mcp.json"),
            &json!({
                "mcpServers": {"codex-tool-resume": {
                    "type":"stdio", "command":"./mcp/codex-auto-resume-mcp.exe", "cwd":".",
                    "env": {"CODEX_AUTO_RESUME_PLUGIN_HOME": runtime, "CODEX_AUTO_RESUME_HOME": runtime}
                }}
            }),
        )?;
        fs::write(
            plugin.join("assets/icon.png"),
            include_bytes!("../icons/icon.png"),
        )?;
        fs::copy(
            runtime.join("app/mcp/codex-auto-resume-mcp.exe"),
            plugin.join("mcp/codex-auto-resume-mcp.exe"),
        )?;
        fs::copy(
            runtime.join("app/LICENSE"),
            plugin.join("AUTO-RESUME-LICENSE"),
        )?;
        fs::write(skill.join("SKILL.md"), include_str!("plugin-skill.md"))?;
        write_json(
            &root.join(".agents/plugins/marketplace.json"),
            &json!({
                "name": MARKETPLACE,
                "interface": {"displayName":"CanDaysa"},
                "plugins":[{"name":PLUGIN, "source":{"source":"local","path":"./plugins/codex-tool"},
                    "policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Developer Tools"}]
            }),
        )
    }

    fn run(command: &mut Command, action: &str) -> Result<(), String> {
        let output = command
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| format!("{action}: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        let detail = detail
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        Err(format!("{action} ({}): {detail}", output.status))
    }

    pub fn recovery_command(args: &[&str]) -> Result<String, String> {
        let runtime = resume_home()?;
        if !runtime_ready(&runtime) {
            return Err("Kurtarma motoru kurulu değil.".to_string());
        }
        let output = Command::new(runtime.join("runtime/python.exe"))
            .creation_flags(0x08000000)
            .args(["-I", "-X", "disable-remote-debug", "-X", "utf8", "-c",
                "import sys; sys.path.insert(0, sys.argv.pop(1)); from codex_auto_resume.cli import main; sys.exit(main())"])
            .arg(runtime.join("app/src"))
            .arg("--home").arg(&runtime)
            .arg("--codex-home").arg(crate::commands::get_codex_home())
            .args(args)
            .output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("Auto Resume komutu başarısız oldu.".to_string());
        }
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    }

    pub fn install() -> Result<String, String> {
        let cli = desktop_cli()?;
        let codex_home = crate::commands::get_codex_home();
        let runtime = resume_home()?;
        if runtime_ready(&runtime) {
            let installed = Command::new(&cli)
                .creation_flags(0x08000000)
                .env("CODEX_HOME", &codex_home)
                .args(["plugin", "list", "--marketplace", MARKETPLACE, "--json"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
            if installed
                .as_ref()
                .and_then(|list| list["installed"].as_array())
                .is_some_and(|plugins| plugins.iter().any(|plugin| plugin["name"] == PLUGIN))
            {
                return Ok("Codex Tool eklentisi kurulu.".to_owned());
            }
        }
        if !runtime_ready(&runtime) {
            if !cfg!(target_arch = "x86_64") {
                return Err(
                    "Kurtarma motorunun otomatik kurulumu Windows x64 gerektirir.".to_string(),
                );
            }
            let work = tempfile::tempdir().map_err(|e| e.to_string())?;
            run(
                Command::new("powershell.exe")
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "RemoteSigned",
                        "-Command",
                    ])
                    .arg(include_str!("install-resume-runtime.ps1"))
                    .env("CODEX_TOOL_INSTALL_TEMP", work.path())
                    .env("CODEX_HOME", &codex_home)
                    .env("CODEX_AUTO_RESUME_PLUGIN_HOME", &runtime),
                "Kurtarma motoru kurulamadı",
            )?;
            if !runtime_ready(&runtime) {
                return Err("Kurulum tamamlandı ancak kurtarma motoru bulunamadı.".to_string());
            }
        }
        let root = codex_home.join("codex-tool-marketplace");
        stage_plugin(&root, &runtime)
            .map_err(|e| format!("Eklenti dosyaları hazırlanamadı: {e}"))?;
        run(
            Command::new(&cli)
                .env("CODEX_HOME", &codex_home)
                .args(["plugin", "marketplace", "add"])
                .arg(&root),
            "Eklenti kaynağı eklenemedi",
        )?;
        run(
            Command::new(cli).env("CODEX_HOME", &codex_home).args([
                "plugin",
                "add",
                &format!("{PLUGIN}@{MARKETPLACE}"),
            ]),
            "Codex Tool eklentisi kurulamadı",
        )?;
        Ok(format!(
            "Codex Tool {} (CanDaysa) kuruldu. Codex’i yeniden açın.",
            env!("CARGO_PKG_VERSION")
        ))
    }

    pub fn continuation_message(message: Option<String>) -> Result<String, String> {
        let runtime = resume_home()?;
        if !runtime_ready(&runtime) {
            return Err(
                "Kurtarma motoru henüz hazır değil. Ayrıntılar için Terminal’e bakın.".to_owned(),
            );
        }
        let script = r#"
import json, sys
sys.path.insert(0, sys.argv[1])
from codex_auto_resume.config import Paths, load_settings, update_settings
paths = Paths(sys.argv[2])
message = json.load(sys.stdin)
if message is not None:
    if message:
        update_settings(paths, {"custom_message": message, "custom_message_mode": "global", "continuation_style": "custom"})
    else:
        update_settings(paths, {"custom_message": None, "continuation_style": "standard"})
settings = load_settings(paths)
active = settings["continuation_style"] == "custom" and settings["custom_message_mode"] == "global"
print(json.dumps(settings["custom_message"] or "" if active else ""))
"#;
        let mut child = Command::new(runtime.join("runtime/python.exe"))
            .creation_flags(0x08000000)
            .args([
                "-I",
                "-X",
                "disable-remote-debug",
                "-X",
                "utf8",
                "-c",
                script,
            ])
            .arg(runtime.join("app/src"))
            .arg(&runtime)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = serde_json::to_vec(&message.map(|value| value.trim().to_owned()))
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .ok_or("Ayar işlemi başlatılamadı.")?
            .write_all(&input)
            .map_err(|e| e.to_string())?;
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(stderr
                .lines()
                .last()
                .unwrap_or("Devam mesajı kaydedilemedi.")
                .to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn continuation_message(message: Option<String>) -> Result<String, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || windows::continuation_message(message))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = message;
        Err("Otomatik devam ayarları Windows gerektirir.".to_owned())
    }
}

pub async fn recovery_command(args: Vec<String>) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            windows::recovery_command(&args.iter().map(String::as_str).collect::<Vec<_>>())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = args;
        Err("Kurtarma motoru Windows gerektirir.".to_string())
    }
}

#[tauri::command]
pub async fn install_codex_plugin() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = INSTALL_LOCK
            .try_lock()
            .map_err(|_| "Eklenti kurulumu zaten devam ediyor.")?;
        tauri::async_runtime::spawn_blocking(windows::install)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "windows"))]
    Err("Auto Resume eklentisinin kurtarma motoru Windows gerektirir.".to_string())
}
