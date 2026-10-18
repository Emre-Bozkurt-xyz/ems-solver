mod api;
mod app;
mod capture;
mod cli;
mod config;
mod gemini;
mod log;
mod output;
mod platform;
mod prompt;
mod router;
mod server;
mod settings;
mod signing;
mod sse;
mod tray;
mod updater;

use cli::Command;

/// Release builds on Windows have no console; reattach to the terminal that
/// launched us so command output is visible.
fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn main_with_args(args: Vec<String>) {
    log::install_panic_hook();
    let command = match cli::parse(&args) {
        Ok(command) => command,
        Err(e) => {
            attach_console();
            eprintln!("error: {e}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    match command {
        Command::Run => app::run(None),
        Command::Action(action) => app::run(Some(action)),
        local => {
            attach_console();
            match cli::run_local(local) {
                Ok(output) => println!("{output}"),
                Err(e) => {
                    eprintln!("error: {e:#}");
                    std::process::exit(1);
                }
            }
        }
    }
}
