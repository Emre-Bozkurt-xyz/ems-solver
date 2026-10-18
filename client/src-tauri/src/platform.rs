//! What this machine supports, detected at runtime, and how to paste on it.

use anyhow::Result;
use serde::Serialize;

use crate::config;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Available,
    Limited,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
pub struct Capability {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

impl Capability {
    fn new(name: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self { name, status, detail: detail.into(), fix: None }
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

// Each OS constructs only its own variants.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PasteMethod {
    SendInput,
    Xdotool,
    Wtype,
    Ydotool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Environment {
    /// e.g. "Windows", "Linux (Wayland, ubuntu:GNOME)"
    pub platform: String,
    pub hotkeys_supported: bool,
    pub paste: Option<PasteMethod>,
    pub capabilities: Vec<Capability>,
}

pub const HOTKEYS: &str = "Global hotkeys";
pub const PASTE: &str = "Paste at cursor";

impl Environment {
    pub fn set(&mut self, capability: Capability) {
        match self.capabilities.iter_mut().find(|c| c.name == capability.name) {
            Some(existing) => *existing = capability,
            None => self.capabilities.push(capability),
        }
    }

    pub fn report(&self) -> String {
        let mut out = format!("ems-solver on {}\n\n", self.platform);
        for c in &self.capabilities {
            let mark = match c.status {
                Status::Available => "✓",
                Status::Limited => "!",
                Status::Unavailable => "✗",
            };
            out += &format!("  {mark} {:<18} {}\n", c.name, c.detail);
            if let Some(fix) = &c.fix {
                out += &format!("      → {fix}\n");
            }
        }
        out
    }
}

fn clipboard_capability() -> Capability {
    match arboard::Clipboard::new() {
        Ok(_) => Capability::new("Clipboard", Status::Available, "can read text and images"),
        Err(e) => Capability::new("Clipboard", Status::Unavailable, format!("can't open the clipboard: {e}")),
    }
}

fn keyring_capability() -> Capability {
    match config::keyring_status() {
        Ok(()) => Capability::new("Credential store", Status::Available, "secrets are kept in the system credential store"),
        Err(e) => Capability::new(
            "Credential store",
            Status::Limited,
            format!("not available ({e}); secrets go to a private file in the config folder"),
        )
        .fix(linux_keyring_fix()),
    }
}

fn linux_keyring_fix() -> &'static str {
    if cfg!(target_os = "linux") {
        "install and unlock a Secret Service keyring (e.g. gnome-keyring or KWallet)"
    } else {
        "check that Windows Credential Manager is working"
    }
}

#[cfg(windows)]
pub fn detect() -> Environment {
    Environment {
        platform: "Windows".into(),
        hotkeys_supported: true,
        paste: Some(PasteMethod::SendInput),
        capabilities: vec![
            clipboard_capability(),
            Capability::new(HOTKEYS, Status::Available, "registered with Windows"),
            Capability::new(PASTE, Status::Available, "simulated Ctrl+V"),
            Capability::new("Tray icon", Status::Available, "notification area"),
            keyring_capability(),
        ],
    }
}

#[cfg(target_os = "linux")]
pub fn detect() -> Environment {
    linux::detect(clipboard_capability(), keyring_capability())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn detect() -> Environment {
    Environment {
        platform: std::env::consts::OS.into(),
        hotkeys_supported: true,
        paste: None,
        capabilities: vec![clipboard_capability(), keyring_capability()],
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;

    use super::*;

    #[derive(PartialEq, Eq)]
    enum Session {
        X11,
        Wayland,
        Unknown,
    }

    fn session() -> Session {
        match std::env::var("XDG_SESSION_TYPE").unwrap_or_default().as_str() {
            "wayland" => Session::Wayland,
            "x11" => Session::X11,
            _ if std::env::var_os("WAYLAND_DISPLAY").is_some() => Session::Wayland,
            _ if std::env::var_os("DISPLAY").is_some() => Session::X11,
            _ => Session::Unknown,
        }
    }

    fn has(tool: &str) -> bool {
        which::which(tool).is_ok()
    }

    fn ydotool_socket() -> Option<PathBuf> {
        let candidates = [
            std::env::var_os("YDOTOOL_SOCKET").map(PathBuf::from),
            std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join(".ydotool_socket")),
            Some(PathBuf::from("/tmp/.ydotool_socket")),
        ];
        candidates.into_iter().flatten().find(|p| p.exists())
    }

    fn shortcut_fix(desktop: &str) -> String {
        let commands = "`ems-solver add`, `ems-solver send` and `ems-solver clear`";
        if desktop.contains("GNOME") {
            format!("bind {commands} in Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts")
        } else if desktop.contains("KDE") {
            format!("bind {commands} in System Settings → Shortcuts → Add New → Command or Script")
        } else {
            format!("bind {commands} to keys in your desktop's or compositor's shortcut settings")
        }
    }

    fn tray(desktop: &str) -> Capability {
        if !desktop.contains("GNOME") {
            return Capability::new("Tray icon", Status::Available, "status notifier");
        }
        let fix = "enable the AppIndicator extension (on Ubuntu: `gnome-extensions enable ubuntu-appindicators@ubuntu.com`)";
        match Command::new("gnome-extensions").args(["list", "--enabled"]).output() {
            Ok(out) if String::from_utf8_lossy(&out.stdout).to_lowercase().contains("appindicator") => {
                Capability::new("Tray icon", Status::Available, "AppIndicator extension is enabled")
            }
            Ok(_) => Capability::new("Tray icon", Status::Unavailable, "GNOME shows tray icons only with the AppIndicator extension").fix(fix),
            Err(_) => Capability::new("Tray icon", Status::Limited, "couldn't check for the AppIndicator extension").fix(fix),
        }
    }

    pub fn detect(clipboard: Capability, keyring: Capability) -> Environment {
        let session = session();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let gnome = desktop.contains("GNOME");
        let session_name = match session {
            Session::X11 => "X11",
            Session::Wayland => "Wayland",
            Session::Unknown => "unknown session",
        };
        let platform = if desktop.is_empty() {
            format!("Linux ({session_name})")
        } else {
            format!("Linux ({session_name}, {desktop})")
        };

        let (hotkeys_supported, hotkeys) = match session {
            Session::X11 => (true, Capability::new(HOTKEYS, Status::Available, "registered through X11")),
            _ => (
                false,
                Capability::new(HOTKEYS, Status::Unavailable, "Wayland doesn't let apps register global shortcuts")
                    .fix(shortcut_fix(&desktop)),
            ),
        };

        let (paste, paste_capability) = match session {
            Session::X11 if has("xdotool") => (Some(PasteMethod::Xdotool), Capability::new(PASTE, Status::Available, "xdotool")),
            Session::X11 => (
                None,
                Capability::new(PASTE, Status::Unavailable, "xdotool isn't installed; answers are copied instead")
                    .fix("sudo apt install xdotool"),
            ),
            _ if has("wtype") && !gnome => (Some(PasteMethod::Wtype), Capability::new(PASTE, Status::Available, "wtype")),
            _ if has("ydotool") && ydotool_socket().is_some() => {
                (Some(PasteMethod::Ydotool), Capability::new(PASTE, Status::Available, "ydotool"))
            }
            _ if has("ydotool") => (
                None,
                Capability::new(PASTE, Status::Limited, "ydotool is installed but its daemon isn't running; answers are copied instead")
                    .fix("start ydotoold (it needs access to /dev/uinput)"),
            ),
            _ => (
                None,
                Capability::new(
                    PASTE,
                    Status::Unavailable,
                    if gnome {
                        "GNOME on Wayland doesn't allow simulated keystrokes; answers are copied to the clipboard instead"
                    } else {
                        "no keystroke tool found; answers are copied to the clipboard instead"
                    },
                )
                .fix(if gnome {
                    "optional: install ydotool and run ydotoold, or log in with \"Ubuntu on Xorg\""
                } else {
                    "optional: install wtype (or ydotool with ydotoold running)"
                }),
            ),
        };

        Environment {
            platform,
            hotkeys_supported,
            paste,
            capabilities: vec![clipboard, hotkeys, paste_capability, tray(&desktop), keyring],
        }
    }
}

/// Sends the platform's paste shortcut to the focused window.
pub fn paste(method: PasteMethod) -> Result<()> {
    match method {
        PasteMethod::SendInput => send_input_paste(),
        PasteMethod::Xdotool => run(&["xdotool", "key", "--clearmodifiers", "ctrl+v"]),
        PasteMethod::Wtype => run(&["wtype", "-M", "ctrl", "-k", "v", "-m", "ctrl"]),
        // Linux input event codes: 29 = left Ctrl, 47 = V.
        PasteMethod::Ydotool => run(&["ydotool", "key", "29:1", "47:1", "47:0", "29:0"]),
    }
}

#[allow(dead_code)]
fn run(argv: &[&str]) -> Result<()> {
    let status = std::process::Command::new(argv[0]).args(&argv[1..]).status()?;
    anyhow::ensure!(status.success(), "{} exited with {status}", argv[0]);
    Ok(())
}

#[cfg(windows)]
fn send_input_paste() -> Result<()> {
    use std::mem::size_of;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
    };

    // If the user is still holding the hotkey's modifiers, Ctrl+V would arrive as
    // e.g. Ctrl+Alt+Shift+V. Give them a moment to let go.
    let deadline = Instant::now() + Duration::from_millis(1500);
    let held = || [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } < 0);
    while held() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(30));
    }

    let key = |vk: VIRTUAL_KEY, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [key(VK_CONTROL, 0), key(VK_V, 0), key(VK_V, KEYEVENTF_KEYUP), key(VK_CONTROL, KEYEVENTF_KEYUP)];
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    anyhow::ensure!(sent as usize == inputs.len(), "Windows blocked the simulated keystrokes");
    Ok(())
}

#[cfg(not(windows))]
fn send_input_paste() -> Result<()> {
    anyhow::bail!("SendInput is only available on Windows")
}
