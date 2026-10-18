//! Checks the GitHub releases for a newer version and installs it when the user asks.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{app, log, tray};

/// The newest update found so far, waiting for the user to install it.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

/// Returns the available version, if any. Announces a newly found version once.
pub async fn check(app: &AppHandle) -> Result<Option<String>, String> {
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let version = update.as_ref().map(|u| u.version.clone());

    let pending = app.state::<Pending>();
    let is_new = {
        let mut slot = pending.0.lock().unwrap();
        let is_new = version.is_some() && slot.as_ref().map(|u| &u.version) != version.as_ref();
        *slot = update;
        is_new
    };
    if let (true, Some(v)) = (is_new, &version) {
        log::line(format!("update {v} available"));
        tray::show_update(app, v);
        app::notify(app, &format!("ems-solver {v} is available"), "Install it from the tray menu or the settings window.");
    }
    Ok(version)
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    let pending = app.state::<Pending>();
    let update = pending.0.lock().unwrap().clone().ok_or("No update is waiting to be installed")?;
    log::line(format!("installing update {}", update.version));
    app::notify(app, &format!("Updating to {}", update.version), "ems-solver will restart when it's done.");
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| format!("Update failed: {e}"))?;
    app.restart()
}

/// Checks shortly after startup and then every few hours. Skipped in dev builds,
/// which never match a published release.
pub fn start_background_checks(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            if let Err(e) = check(&app).await {
                log::line(format!("update check failed: {e}"));
            }
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}
