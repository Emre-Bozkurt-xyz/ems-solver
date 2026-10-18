//! The tray app: shared state, action handling, hotkeys and the settings window.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use crate::api::{self, Part};
use crate::cli::{self, Action, Command};
use crate::config::{Config, Providers};
use crate::output::{self, Delivery};
use crate::platform::{self, Capability, Environment, Status};
use crate::{capture, log, router, settings, tray, updater};

pub struct Core {
    pub env: Mutex<Environment>,
    buffer: Mutex<Vec<Part>>,
    busy: AtomicBool,
    last_error: Mutex<Option<String>>,
    // Kept for the app's lifetime: on Linux the process serves clipboard contents
    // it has set only while its clipboard handle is alive.
    clipboard: Mutex<Option<arboard::Clipboard>>,
    http: reqwest::Client,
    hotkeys: Mutex<HashMap<u32, Action>>,
}

pub enum TrayState {
    Setup,
    Idle,
    Buffered(String),
    Busy,
    Error(String),
}

impl Core {
    fn new(env: Environment) -> Result<Core> {
        let http = router::http_client()?;
        Ok(Core {
            env: Mutex::new(env),
            buffer: Mutex::new(Vec::new()),
            busy: AtomicBool::new(false),
            last_error: Mutex::new(None),
            clipboard: Mutex::new(None),
            http,
            hotkeys: Mutex::new(HashMap::new()),
        })
    }

    fn with_clipboard<T>(&self, f: impl FnOnce(&mut arboard::Clipboard) -> Result<T>) -> Result<T> {
        let mut guard = self.clipboard.lock().unwrap();
        if guard.is_none() {
            *guard = Some(arboard::Clipboard::new().map_err(|e| anyhow!("couldn't open the clipboard: {e}"))?);
        }
        f(guard.as_mut().unwrap())
    }

    pub fn buffer_summary(&self) -> String {
        api::summarize(&self.buffer.lock().unwrap())
    }

    pub fn tray_state(&self, configured: bool) -> TrayState {
        if self.busy.load(Ordering::SeqCst) {
            return TrayState::Busy;
        }
        if !configured {
            return TrayState::Setup;
        }
        if let Some(err) = self.last_error.lock().unwrap().clone() {
            return TrayState::Error(err);
        }
        let buffer = self.buffer.lock().unwrap();
        if buffer.is_empty() {
            TrayState::Idle
        } else {
            TrayState::Buffered(api::summarize(&buffer))
        }
    }
}

pub fn core(app: &AppHandle) -> Arc<Core> {
    app.state::<Arc<Core>>().inner().clone()
}

pub fn notify(app: &AppHandle, title: &str, body: impl AsRef<str>) {
    log::line(format!("notify: {title}: {}", body.as_ref().replace('\n', " / ")));
    let _ = app.notification().builder().title(title).body(body.as_ref()).show();
}

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        format!("{}…", flat.chars().take(120).collect::<String>())
    } else {
        flat
    }
}

pub fn dispatch(app: &AppHandle, action: Action) {
    log::line(format!("action: {action:?}"));
    match action {
        Action::Settings => open_settings(app),
        Action::Quit => app.exit(0),
        Action::Add | Action::Send | Action::Clear => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { run_action(&app, action).await });
        }
    }
}

async fn read_clipboard(core: &Arc<Core>) -> Result<Part> {
    let core = core.clone();
    tauri::async_runtime::spawn_blocking(move || core.with_clipboard(capture::read)).await?
}

async fn run_action(app: &AppHandle, action: Action) {
    let core = core(app);
    let config = match Config::load() {
        Ok(config) => config,
        Err(e) => return notify(app, "Settings file problem", format!("{e:#}")),
    };
    match action {
        Action::Add => add(app, &core, &config).await,
        Action::Send => send(app, &core, &config).await,
        Action::Clear => {
            core.buffer.lock().unwrap().clear();
            *core.last_error.lock().unwrap() = None;
            if config.notify_on_add {
                notify(app, "Buffer cleared", "Nothing is waiting to be sent.");
            }
        }
        Action::Settings | Action::Quit => {}
    }
    tray::refresh(app);
}

async fn add(app: &AppHandle, core: &Arc<Core>, config: &Config) {
    match read_clipboard(core).await {
        Ok(part) => {
            let description = part.describe();
            let summary = {
                let mut buffer = core.buffer.lock().unwrap();
                buffer.push(part);
                api::summarize(&buffer)
            };
            *core.last_error.lock().unwrap() = None;
            if config.notify_on_add {
                notify(app, &format!("Added {description}"), format!("Buffer: {summary}"));
            }
        }
        Err(e) => notify(app, "Nothing added", format!("{e:#}")),
    }
}

async fn send(app: &AppHandle, core: &Arc<Core>, config: &Config) {
    if core.busy.swap(true, Ordering::SeqCst) {
        return notify(app, "Still working", "The previous request hasn't finished yet.");
    }
    tray::refresh(app);
    let result = solve_and_deliver(app, core, config).await;
    core.busy.store(false, Ordering::SeqCst);

    match result {
        Ok(()) => *core.last_error.lock().unwrap() = None,
        Err(message) => {
            notify(app, "No answer", &message);
            *core.last_error.lock().unwrap() = Some(message);
        }
    }
}

async fn solve_and_deliver(app: &AppHandle, core: &Arc<Core>, config: &Config) -> Result<(), String> {
    let providers = Providers::resolve(config);
    if !providers.is_configured() {
        return Err("Not set up yet. Open Settings, or run `ems-solver set gemini-key` / `ems-solver set server-url`.".into());
    }

    let buffered = core.buffer.lock().unwrap().clone();
    let parts = if buffered.is_empty() { vec![read_clipboard(core).await.map_err(|e| format!("{e:#}"))?] } else { buffered.clone() };

    log::line(format!("sending {} via {}", api::summarize(&parts), providers.describe()));
    let answer = router::solve(&core.http, &providers, config.mode, &parts).await.map_err(|e| e.message)?;
    log::line(format!("answer from {} ({}), {} chars", answer.provider, answer.model, answer.text.len()));

    // Only drop what was sent; anything added while waiting stays for next time.
    {
        let mut buffer = core.buffer.lock().unwrap();
        let sent = buffered.len().min(buffer.len());
        buffer.drain(..sent);
    }

    let paste_method = core.env.lock().unwrap().paste;
    let delivery = {
        let core = core.clone();
        let text = answer.text.clone();
        let (output, restore) = (config.output, config.restore_clipboard);
        tauri::async_runtime::spawn_blocking(move || {
            core.with_clipboard(|cb| output::deliver(cb, &text, output, paste_method, restore))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Got an answer but couldn't use the clipboard: {e:#}"))?
    };

    let mut notes: Vec<String> = Vec::new();
    if let Some(notice) = &answer.notice {
        notes.push(notice.clone());
    }
    if answer.truncated {
        notes.push("The answer was cut off because it hit the length limit.".into());
    }
    match delivery {
        Delivery::Pasted if notes.is_empty() => {}
        Delivery::Pasted => notify(app, "Answer pasted", notes.join(" ")),
        Delivery::Copied { reason } => {
            notes.extend(reason.map(|r| format!("Not pasted: {r}.")));
            notes.push(preview(&answer.text));
            notify(app, &format!("Answer copied · {} ({})", answer.provider, answer.model), notes.join("\n"));
        }
    }
    Ok(())
}

/// Unregisters every hotkey, e.g. while the user records a new one in Settings.
pub fn unregister_hotkeys(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
    core(app).hotkeys.lock().unwrap().clear();
}

/// (Re)registers the hotkeys from `config` and records the outcome as a capability.
pub fn register_hotkeys(app: &AppHandle, config: &Config) -> Capability {
    let core = core(app);
    unregister_hotkeys(app);
    if !core.env.lock().unwrap().hotkeys_supported {
        return core.env.lock().unwrap().capabilities.iter().find(|c| c.name == platform::HOTKEYS).cloned().unwrap_or(
            Capability { name: platform::HOTKEYS, status: Status::Unavailable, detail: "not supported here".into(), fix: None },
        );
    }
    let mut registered = Vec::new();
    let mut failed = Vec::new();
    let mut map = core.hotkeys.lock().unwrap();
    for (action, name, combo) in [
        (Action::Add, "add", &config.hotkeys.add),
        (Action::Send, "send", &config.hotkeys.send),
        (Action::Clear, "clear", &config.hotkeys.clear),
    ] {
        if combo.trim().is_empty() {
            continue;
        }
        match combo.parse::<Shortcut>() {
            Ok(shortcut) => match app.global_shortcut().register(shortcut) {
                Ok(()) => {
                    map.insert(shortcut.id(), action);
                    registered.push(format!("{name} {combo}"));
                }
                Err(e) => failed.push(format!("{combo} ({e})")),
            },
            Err(e) => failed.push(format!("{combo} (not a valid shortcut: {e})")),
        }
    }
    drop(map);
    let capability = if registered.is_empty() && failed.is_empty() {
        Capability { name: platform::HOTKEYS, status: Status::Limited, detail: "none set".into(), fix: None }
    } else if failed.is_empty() {
        Capability { name: platform::HOTKEYS, status: Status::Available, detail: registered.join(" · "), fix: None }
    } else {
        Capability {
            name: platform::HOTKEYS,
            status: if registered.is_empty() { Status::Unavailable } else { Status::Limited },
            detail: format!("couldn't register {}", failed.join(", ")),
            fix: Some("another app may be using these keys; choose different ones in Settings".into()),
        }
    };
    core.env.lock().unwrap().set(capability.clone());
    capability
}

pub fn hotkey_label(app: &AppHandle, action: Action) -> Option<String> {
    let core = core(app);
    let config = Config::load().ok()?;
    let registered = core.hotkeys.lock().unwrap().values().any(|a| *a == action);
    let combo = match action {
        Action::Add => config.hotkeys.add,
        Action::Send => config.hotkeys.send,
        Action::Clear => config.hotkeys.clear,
        _ => return None,
    };
    registered.then_some(combo)
}

pub fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title("ems-solver settings")
        .inner_size(680.0, 820.0)
        .min_inner_size(520.0, 480.0)
        .build();
    match built {
        Ok(window) => {
            // Recording a hotkey pauses them all; make sure they come back however the window closes.
            let app = app.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::Destroyed = event {
                    register_hotkeys(&app, &Config::load().unwrap_or_default());
                    tray::refresh(&app);
                }
            });
        }
        Err(e) => notify(app, "Couldn't open settings", e.to_string()),
    }
}

pub fn run(initial: Option<Action>) {
    let env = platform::detect();
    let shared = match Core::new(env) {
        Ok(core) => Arc::new(core),
        Err(e) => {
            eprintln!("ems-solver failed to start: {e:#}");
            std::process::exit(1);
        }
    };

    let app = tauri::Builder::default()
        // Must be first: a second launch hands its arguments to this instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            match cli::parse(argv.get(1..).unwrap_or_default()) {
                Ok(Command::Action(action)) => dispatch(app, action),
                Ok(Command::Run) => open_settings(app),
                _ => {}
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updater::Pending::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let action = core(app).hotkeys.lock().unwrap().get(&shortcut.id()).copied();
                    if let Some(action) = action {
                        dispatch(app, action);
                    }
                })
                .build(),
        )
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::save_settings,
            settings::test_server,
            settings::test_gemini,
            settings::pause_hotkeys,
            settings::open_config_folder,
            settings::check_for_updates,
            settings::install_update,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let config = Config::load().unwrap_or_default();
            register_hotkeys(&handle, &config);
            tray::create(&handle)?;
            updater::start_background_checks(handle.clone());
            if !Providers::resolve(&config).is_configured() && initial.is_none() {
                open_settings(&handle);
            }
            if let Some(action) = initial {
                dispatch(&handle, action);
            }
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("ems-solver failed to start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|_app, event| {
        // Closing the settings window must not quit the tray app.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            api.prevent_exit();
        }
    });
}
