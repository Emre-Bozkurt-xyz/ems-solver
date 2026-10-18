use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Mode;
use crate::gemini::{self, GeminiCredentials};
use crate::server::ServerCredentials;

pub const APP_NAME: &str = "ems-solver";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    /// Paste at the cursor when this machine supports it, otherwise copy.
    #[default]
    Auto,
    Paste,
    Clipboard,
}

impl OutputMode {
    pub const ALL: [OutputMode; 3] = [OutputMode::Auto, OutputMode::Paste, OutputMode::Clipboard];

    pub fn as_str(self) -> &'static str {
        match self {
            OutputMode::Auto => "auto",
            OutputMode::Paste => "paste",
            OutputMode::Clipboard => "clipboard",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OutputMode::Auto => "Automatic",
            OutputMode::Paste => "Paste at cursor",
            OutputMode::Clipboard => "Copy to clipboard",
        }
    }

    pub fn parse(s: &str) -> Option<OutputMode> {
        OutputMode::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub add: String,
    pub send: String,
    pub clear: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        // Ctrl+Alt alone is AltGr on many layouts, so Shift is added to stay clear of typed characters.
        Self {
            add: "Ctrl+Alt+Shift+A".into(),
            send: "Ctrl+Alt+Shift+S".into(),
            clear: "Ctrl+Alt+Shift+X".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server_url: String,
    pub client_id: String,
    pub gemini_model: String,
    pub mode: Mode,
    pub output: OutputMode,
    /// Put back whatever was on the clipboard after pasting the answer.
    pub restore_clipboard: bool,
    pub notify_on_add: bool,
    pub hotkeys: Hotkeys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            client_id: String::new(),
            gemini_model: gemini::DEFAULT_MODEL.into(),
            mode: Mode::Solution,
            output: OutputMode::Auto,
            restore_clipboard: true,
            notify_on_add: true,
            hotkeys: Hotkeys::default(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_NAME)
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("{} is not valid", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(config_dir())?;
        let path = config_path();
        fs::write(&path, serde_json::to_string_pretty(self)? + "\n").with_context(|| format!("writing {}", path.display()))
    }
}

/// Secrets live in the OS credential store (Windows Credential Manager, Secret Service on
/// Linux). If that isn't available, they fall back to a file only the user can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secret {
    ClientSecret,
    GeminiApiKey,
}

impl Secret {
    fn key(self) -> &'static str {
        match self {
            Secret::ClientSecret => "client_secret",
            Secret::GeminiApiKey => "gemini_api_key",
        }
    }
}

fn secrets_file() -> PathBuf {
    config_dir().join("secrets.json")
}

fn read_secrets_file() -> BTreeMap<String, String> {
    fs::read_to_string(secrets_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_secrets_file(map: &BTreeMap<String, String>) -> Result<()> {
    fs::create_dir_all(config_dir())?;
    let path = secrets_file();
    if map.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    fs::write(&path, serde_json::to_string_pretty(map)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn entry(secret: Secret) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(APP_NAME, secret.key())
}

/// Whether the OS credential store works on this machine.
pub fn keyring_status() -> Result<(), String> {
    match entry(Secret::ClientSecret).and_then(|e| e.get_password()) {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn get_secret(secret: Secret) -> Option<String> {
    let from_keyring = entry(secret).and_then(|e| e.get_password());
    match from_keyring {
        Ok(value) if !value.is_empty() => Some(value),
        _ => read_secrets_file().remove(secret.key()).filter(|v| !v.is_empty()),
    }
}

/// Stores the secret and returns where it went.
pub fn set_secret(secret: Secret, value: &str) -> Result<&'static str> {
    let mut file = read_secrets_file();
    if entry(secret).and_then(|e| e.set_password(value)).is_ok() {
        // Don't leave an older copy behind in the fallback file.
        if file.remove(secret.key()).is_some() {
            write_secrets_file(&file)?;
        }
        return Ok("the system credential store");
    }
    file.insert(secret.key().into(), value.into());
    write_secrets_file(&file)?;
    Ok("secrets.json (no system credential store available)")
}

pub fn delete_secret(secret: Secret) -> Result<()> {
    if let Ok(e) = entry(secret) {
        let _ = e.delete_credential();
    }
    let mut file = read_secrets_file();
    if file.remove(secret.key()).is_some() {
        write_secrets_file(&file)?;
    }
    Ok(())
}

pub struct Providers {
    pub server: Option<ServerCredentials>,
    pub gemini: Option<GeminiCredentials>,
}

impl Providers {
    pub fn resolve(config: &Config) -> Providers {
        let server = match (config.server_url.trim(), config.client_id.trim(), get_secret(Secret::ClientSecret)) {
            (url, id, Some(secret)) if !url.is_empty() && !id.is_empty() => {
                Some(ServerCredentials { url: url.into(), client_id: id.into(), secret })
            }
            _ => None,
        };
        let gemini = get_secret(Secret::GeminiApiKey).map(|api_key| GeminiCredentials {
            api_key,
            model: if config.gemini_model.trim().is_empty() { gemini::DEFAULT_MODEL.into() } else { config.gemini_model.trim().into() },
        });
        Providers { server, gemini }
    }

    pub fn is_configured(&self) -> bool {
        self.server.is_some() || self.gemini.is_some()
    }

    pub fn describe(&self) -> String {
        match (&self.server, &self.gemini) {
            (Some(_), Some(_)) => "Claude via server, Gemini as fallback".into(),
            (Some(_), None) => "Claude via server (no fallback)".into(),
            (None, Some(_)) => "Gemini only".into(),
            (None, None) => "not set up".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_config_files_use_defaults() {
        let config: Config = serde_json::from_str(r#"{"mode": "hint", "hotkeys": {"send": "F9"}}"#).unwrap();
        assert_eq!(config.mode, Mode::Hint);
        assert_eq!(config.hotkeys.send, "F9");
        assert_eq!(config.hotkeys.add, Hotkeys::default().add);
        assert_eq!(config.gemini_model, gemini::DEFAULT_MODEL);
        assert!(config.restore_clipboard);
    }
}
