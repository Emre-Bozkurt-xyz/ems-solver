use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::api::Mode;
use crate::app::{self, Core, TrayState};
use crate::cli::Action;
use crate::config::{Config, OutputMode, Providers};

const TRAY_ID: &str = "main";

struct Handles {
    status: MenuItem<Wry>,
    add: MenuItem<Wry>,
    send: MenuItem<Wry>,
    clear: MenuItem<Wry>,
    modes: Vec<(Mode, CheckMenuItem<Wry>)>,
    outputs: Vec<(OutputMode, CheckMenuItem<Wry>)>,
}

fn icon(state: &TrayState) -> Image<'static> {
    let bytes: &'static [u8] = match state {
        TrayState::Setup => include_bytes!("../icons/tray/setup.png"),
        TrayState::Idle => include_bytes!("../icons/tray/idle.png"),
        TrayState::Buffered(_) => include_bytes!("../icons/tray/buffered.png"),
        TrayState::Busy => include_bytes!("../icons/tray/busy.png"),
        TrayState::Error(_) => include_bytes!("../icons/tray/error.png"),
    };
    Image::from_bytes(bytes).expect("bundled tray icons are valid PNGs")
}

fn status_text(state: &TrayState) -> String {
    match state {
        TrayState::Setup => "Not set up · open Settings".into(),
        TrayState::Idle => "Ready · buffer empty".into(),
        TrayState::Buffered(summary) => format!("Buffer: {summary}"),
        TrayState::Busy => "Working…".into(),
        TrayState::Error(err) => format!("Last request failed: {}", truncate(err, 60)),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_string()
    }
}

fn action_label(app: &AppHandle, action: Action, label: &str) -> String {
    match app::hotkey_label(app, action) {
        Some(combo) => format!("{label}\t{combo}"),
        None => label.to_string(),
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let config = Config::load().unwrap_or_default();

    let status = MenuItem::with_id(app, "status", "", false, None::<&str>)?;
    let add = MenuItem::with_id(app, "add", action_label(app, Action::Add, "Add clipboard to buffer"), true, None::<&str>)?;
    let send = MenuItem::with_id(app, "send", action_label(app, Action::Send, "Send"), true, None::<&str>)?;
    let clear = MenuItem::with_id(app, "clear", action_label(app, Action::Clear, "Clear buffer"), true, None::<&str>)?;

    let modes = Mode::ALL
        .into_iter()
        .map(|m| Ok((m, CheckMenuItem::with_id(app, format!("mode:{}", m.as_str()), m.label(), true, m == config.mode, None::<&str>)?)))
        .collect::<tauri::Result<Vec<_>>>()?;
    let outputs = OutputMode::ALL
        .into_iter()
        .map(|o| {
            Ok((o, CheckMenuItem::with_id(app, format!("output:{}", o.as_str()), o.label(), true, o == config.output, None::<&str>)?))
        })
        .collect::<tauri::Result<Vec<_>>>()?;

    let mode_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = modes.iter().map(|(_, i)| i as _).collect();
    let output_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = outputs.iter().map(|(_, i)| i as _).collect();
    let mode_menu = Submenu::with_items(app, "Mode", true, &mode_items)?;
    let output_menu = Submenu::with_items(app, "Answer output", true, &output_items)?;

    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &add,
            &send,
            &clear,
            &PredefinedMenuItem::separator(app)?,
            &mode_menu,
            &output_menu,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "settings", "Settings and status…", true, None::<&str>)?,
            &MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(&TrayState::Idle))
        .tooltip("ems-solver")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .build(app)?;

    app.manage(Handles { status, add, send, clear, modes, outputs });
    refresh(app);
    Ok(())
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        "add" => app::dispatch(app, Action::Add),
        "send" => app::dispatch(app, Action::Send),
        "clear" => app::dispatch(app, Action::Clear),
        "settings" => app::dispatch(app, Action::Settings),
        "quit" => app::dispatch(app, Action::Quit),
        _ => {
            if let Ok(mut config) = Config::load() {
                if let Some(mode) = id.strip_prefix("mode:").and_then(Mode::parse) {
                    config.mode = mode;
                } else if let Some(output) = id.strip_prefix("output:").and_then(OutputMode::parse) {
                    config.output = output;
                }
                let _ = config.save();
            }
            refresh(app);
        }
    }
}

/// Bring the icon, tooltip and menu in line with the current state and config.
pub fn refresh(app: &AppHandle) {
    let config = Config::load().unwrap_or_default();
    let state = app.state::<std::sync::Arc<Core>>().tray_state(Providers::resolve(&config).is_configured());
    let status = status_text(&state);

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_icon(Some(icon(&state)));
        let _ = tray.set_tooltip(Some(format!("ems-solver · {status}")));
    }
    if let Some(handles) = app.try_state::<Handles>() {
        let _ = handles.status.set_text(status);
        let _ = handles.add.set_text(action_label(app, Action::Add, "Add clipboard to buffer"));
        let _ = handles.send.set_text(action_label(app, Action::Send, "Send"));
        let _ = handles.clear.set_text(action_label(app, Action::Clear, "Clear buffer"));
        for (mode, item) in &handles.modes {
            let _ = item.set_checked(*mode == config.mode);
        }
        for (output, item) in &handles.outputs {
            let _ = item.set_checked(*output == config.output);
        }
    }
}
