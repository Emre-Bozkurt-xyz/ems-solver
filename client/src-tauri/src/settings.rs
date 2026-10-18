//! Commands behind the settings window.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::Shortcut;

use crate::api::Mode;
use crate::config::{self, Config, Hotkeys, OutputMode, Providers, Secret};
use crate::gemini::{self, GeminiCredentials};
use crate::platform::Capability;
use crate::server::{self, ServerCredentials};
use crate::{app, log, router, tray, updater};

#[derive(Serialize)]
pub struct SettingsView {
    version: &'static str,
    platform: String,
    answers_from: String,
    configured: bool,
    buffer: String,
    config: Config,
    has_client_secret: bool,
    has_gemini_key: bool,
    secrets_in_keyring: bool,
    /// None when the start-at-login state can't be read.
    autostart: Option<bool>,
    hotkeys_supported: bool,
    paste_available: bool,
    capabilities: Vec<Capability>,
    config_dir: String,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum SecretChange {
    Keep,
    Set { value: String },
    Remove,
}

#[derive(Deserialize)]
pub struct SettingsInput {
    server_url: String,
    client_id: String,
    gemini_model: String,
    mode: Mode,
    output: OutputMode,
    restore_clipboard: bool,
    notify_on_add: bool,
    hotkeys: Hotkeys,
    autostart: bool,
    client_secret: SecretChange,
    gemini_key: SecretChange,
}

#[derive(Serialize)]
pub struct SaveReport {
    hotkeys: Capability,
    warnings: Vec<String>,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> SettingsView {
    let core = app::core(&app);
    let config = Config::load().unwrap_or_default();
    let providers = Providers::resolve(&config);
    let env = core.env.lock().unwrap().clone();
    SettingsView {
        version: env!("CARGO_PKG_VERSION"),
        platform: env.platform,
        answers_from: providers.describe(),
        configured: providers.is_configured(),
        buffer: core.buffer_summary(),
        has_client_secret: config::get_secret(Secret::ClientSecret).is_some(),
        has_gemini_key: config::get_secret(Secret::GeminiApiKey).is_some(),
        secrets_in_keyring: config::keyring_status().is_ok(),
        autostart: app.autolaunch().is_enabled().ok(),
        hotkeys_supported: env.hotkeys_supported,
        paste_available: env.paste.is_some(),
        capabilities: env.capabilities,
        config_dir: config::config_dir().display().to_string(),
        config,
    }
}

fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Ok(String::new());
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| format!("Server URL {url:?} isn't a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Server URL must start with https:// (or http://)".into());
    }
    Ok(url.to_string())
}

fn validate_hotkeys(hotkeys: &Hotkeys) -> Result<(), String> {
    let mut seen: Vec<Shortcut> = Vec::new();
    for (name, combo) in [("Add", &hotkeys.add), ("Send", &hotkeys.send), ("Clear", &hotkeys.clear)] {
        if combo.trim().is_empty() {
            continue;
        }
        let shortcut = combo.parse::<Shortcut>().map_err(|e| format!("{name} hotkey {combo:?} isn't valid: {e}"))?;
        if seen.contains(&shortcut) {
            return Err(format!("{combo} is used for more than one action"));
        }
        seen.push(shortcut);
    }
    Ok(())
}

fn apply_secret(secret: Secret, change: SecretChange, warnings: &mut Vec<String>) -> Result<(), String> {
    match change {
        SecretChange::Keep => {}
        SecretChange::Remove => config::delete_secret(secret).map_err(|e| e.to_string())?,
        SecretChange::Set { value } if value.trim().is_empty() => {}
        SecretChange::Set { value } => {
            let location = config::set_secret(secret, value.trim()).map_err(|e| e.to_string())?;
            if location.starts_with("secrets.json") {
                warnings.push("No system credential store was available, so secrets were saved to secrets.json.".into());
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn save_settings(app: AppHandle, input: SettingsInput) -> Result<SaveReport, String> {
    let server_url = normalize_url(&input.server_url)?;
    validate_hotkeys(&input.hotkeys)?;

    let mut warnings = Vec::new();
    apply_secret(Secret::ClientSecret, input.client_secret, &mut warnings)?;
    apply_secret(Secret::GeminiApiKey, input.gemini_key, &mut warnings)?;

    let config = Config {
        server_url,
        client_id: input.client_id.trim().to_string(),
        gemini_model: match input.gemini_model.trim() {
            "" => gemini::DEFAULT_MODEL.to_string(),
            model => model.to_string(),
        },
        mode: input.mode,
        output: input.output,
        restore_clipboard: input.restore_clipboard,
        notify_on_add: input.notify_on_add,
        hotkeys: input.hotkeys,
    };
    config.save().map_err(|e| format!("{e:#}"))?;

    let autostart = app.autolaunch();
    let result = if input.autostart { autostart.enable() } else { autostart.disable() };
    if let Err(e) = result {
        warnings.push(format!("Couldn't change start at login: {e}"));
    }

    let hotkeys = app::register_hotkeys(&app, &config);
    tray::refresh(&app);
    log::line("settings saved");
    Ok(SaveReport { hotkeys, warnings })
}

/// Tests the values currently in the form; an empty secret means "use the saved one".
#[tauri::command]
pub async fn test_server(url: String, client_id: String, secret: Option<String>) -> Result<String, String> {
    let url = normalize_url(&url)?;
    if url.is_empty() || client_id.trim().is_empty() {
        return Err("Enter the server URL and client ID first".into());
    }
    let secret = secret
        .filter(|s| !s.trim().is_empty())
        .or_else(|| config::get_secret(Secret::ClientSecret))
        .ok_or("Enter the client secret first")?;
    let creds = ServerCredentials { url, client_id: client_id.trim().into(), secret: secret.trim().into() };
    let http = router::http_client().map_err(|e| e.to_string())?;
    server::ping(&http, &creds).await.map_err(|e| e.message)
}

#[tauri::command]
pub async fn test_gemini(key: Option<String>, model: String) -> Result<String, String> {
    let api_key = key
        .filter(|k| !k.trim().is_empty())
        .or_else(|| config::get_secret(Secret::GeminiApiKey))
        .ok_or("Enter a Gemini API key first")?;
    let model = match model.trim() {
        "" => gemini::DEFAULT_MODEL.to_string(),
        m => m.to_string(),
    };
    let http = router::http_client().map_err(|e| e.to_string())?;
    gemini::check(&http, &GeminiCredentials { api_key: api_key.trim().into(), model }).await.map_err(|e| e.message)
}

#[derive(Serialize)]
pub struct UpdateStatus {
    current: &'static str,
    available: Option<String>,
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateStatus, String> {
    let available = updater::check(&app).await.map_err(|e| format!("Couldn't check for updates: {e}"))?;
    Ok(UpdateStatus { current: env!("CARGO_PKG_VERSION"), available })
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install(&app).await
}

/// While a hotkey field is being recorded, the current hotkeys must not fire.
#[tauri::command]
pub fn pause_hotkeys(app: AppHandle, paused: bool) {
    if paused {
        app::unregister_hotkeys(&app);
    } else {
        app::register_hotkeys(&app, &Config::load().unwrap_or_default());
    }
}

#[tauri::command]
pub fn open_config_folder() -> Result<(), String> {
    let dir = config::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(&dir).spawn().map_err(|e| format!("couldn't open {}: {e}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_normalized() {
        assert_eq!(normalize_url(" https://x.example.com/ ").unwrap(), "https://x.example.com");
        assert_eq!(normalize_url("").unwrap(), "");
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("not a url").is_err());
    }

    #[test]
    fn hotkeys_are_validated() {
        let ok = Hotkeys { add: "Ctrl+Alt+Shift+A".into(), send: "Ctrl+Alt+Shift+S".into(), clear: String::new() };
        assert!(validate_hotkeys(&ok).is_ok());
        let dup = Hotkeys { add: "Ctrl+Shift+A".into(), send: "Shift+Ctrl+A".into(), clear: String::new() };
        assert!(validate_hotkeys(&dup).unwrap_err().contains("more than one"));
        let bad = Hotkeys { add: "Ctrl+Nope".into(), ..ok };
        assert!(validate_hotkeys(&bad).is_err());
    }

    #[test]
    fn recorder_output_parses() {
        // Names produced by the settings page's key recorder.
        for combo in ["Ctrl+Alt+Shift+A", "Ctrl+Shift+1", "Super+Alt+Space", "Alt+F9", "F8", "Ctrl+Shift+ArrowUp", "Ctrl+Alt+Backquote", "Ctrl+Numpad5"] {
            assert!(combo.parse::<Shortcut>().is_ok(), "{combo} should parse");
        }
    }
}
