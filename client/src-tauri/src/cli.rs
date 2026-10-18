//! Command-line interface.
//!
//! Actions (add, send, clear, ...) are handled by the running tray app: a second
//! invocation forwards its arguments there and exits. Configuration commands run
//! directly in the invoking process, so their output shows up in the terminal.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{bail, Context, Result};

use crate::api::{Mode, Part};
use crate::config::{self, Config, OutputMode, Providers, Secret};
use crate::{capture, gemini, platform, router, server};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Send,
    Clear,
    Settings,
    Quit,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Start the tray app (or do nothing if it's already running).
    Run,
    Action(Action),
    Set { key: String, value: Option<String> },
    Unset { key: String },
    Mode(Mode),
    Output(OutputMode),
    Status,
    Capabilities,
    Test,
    Ask { texts: Vec<String>, images: Vec<String> },
    Help,
}

pub const USAGE: &str = "\
Usage: ems-solver [command]

With no command, starts the tray app.

Actions (sent to the running app, which is started if needed):
  add                  Add the clipboard (text or image) to the buffer
  send                 Send the buffer, or just the clipboard if the buffer is empty
  clear                Empty the buffer
  settings             Open the settings window
  quit                 Quit the tray app

Ask from the terminal (no tray app needed):
  ask [--image FILE]... [TEXT]...
                       Send text and/or image files, print the answer

Configuration:
  set server-url <url>         e.g. https://solver.example.com
  set client-id <id>
  set client-secret [value]    Reads from stdin when no value is given
  set gemini-key [value]       Reads from stdin when no value is given
  set gemini-model <model>
  unset <key>                  Remove one of the values above
  mode <solution|hint|checkwork>
  output <auto|paste|clipboard>
  status                       Show the current configuration
  test                         Check the server credentials and Gemini key
  capabilities                 Show what this system supports
";

pub fn parse(args: &[String]) -> Result<Command> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(match args.as_slice() {
        [] => Command::Run,
        ["add"] => Command::Action(Action::Add),
        ["send"] => Command::Action(Action::Send),
        ["clear"] => Command::Action(Action::Clear),
        ["settings"] => Command::Action(Action::Settings),
        ["quit"] => Command::Action(Action::Quit),
        ["set", key] => Command::Set { key: key.to_string(), value: None },
        ["set", key, value] => Command::Set { key: key.to_string(), value: Some(value.to_string()) },
        ["unset", key] => Command::Unset { key: key.to_string() },
        ["mode", mode] => Command::Mode(Mode::parse(mode).with_context(|| format!("unknown mode {mode:?}"))?),
        ["output", output] => {
            Command::Output(OutputMode::parse(output).with_context(|| format!("unknown output mode {output:?}"))?)
        }
        ["status"] => Command::Status,
        ["capabilities"] => Command::Capabilities,
        ["test"] => Command::Test,
        ["help" | "--help" | "-h"] => Command::Help,
        ["ask", rest @ ..] => parse_ask(rest)?,
        _ => bail!("unrecognized command: {}", args.join(" ")),
    })
}

fn parse_ask(args: &[&str]) -> Result<Command> {
    let (mut texts, mut images) = (Vec::new(), Vec::new());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match *arg {
            "--image" => images.push(iter.next().context("--image needs a file path")?.to_string()),
            text => texts.push(text.to_string()),
        }
    }
    if texts.is_empty() && images.is_empty() {
        bail!("`ask` needs some text or --image FILE");
    }
    Ok(Command::Ask { texts, images })
}

fn ask(texts: Vec<String>, images: Vec<String>) -> Result<String> {
    let config = Config::load()?;
    let providers = Providers::resolve(&config);
    if !providers.is_configured() {
        bail!("not set up: run `ems-solver set gemini-key` or configure a server first");
    }
    let mut parts = Vec::new();
    for path in &images {
        let img = image::open(path).with_context(|| format!("couldn't read image {path}"))?.to_rgba8();
        parts.push(capture::encode_image(img.width(), img.height(), img.into_raw())?);
    }
    // Images first, then the text: the question usually refers to the screenshot.
    parts.extend(texts.into_iter().map(|text| Part::Text { text }));

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let answer = runtime.block_on(async {
        let http = router::http_client()?;
        anyhow::Ok(router::solve(&http, &providers, config.mode, &parts).await)
    })??;
    if let Some(notice) = &answer.notice {
        eprintln!("note: {notice}");
    }
    eprintln!("[{} · {}{}]", answer.provider, answer.model, if answer.truncated { " · cut off" } else { "" });
    Ok(answer.text)
}

fn test_connections() -> Result<String> {
    let config = Config::load()?;
    let providers = Providers::resolve(&config);
    if !providers.is_configured() {
        bail!("nothing to test: no server credentials or Gemini key are set");
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let http = router::http_client()?;
    let line = |name: &str, result: Result<String, crate::api::SolveError>| match result {
        Ok(message) => format!("✓ {name}: {message}"),
        Err(e) => format!("✗ {name}: {e}"),
    };
    let mut lines = Vec::new();
    runtime.block_on(async {
        lines.push(match &providers.server {
            Some(creds) => line("Server", server::ping(&http, creds).await),
            None => "- Server: not configured".into(),
        });
        lines.push(match &providers.gemini {
            Some(creds) => line("Gemini", gemini::check(&http, creds).await),
            None => "- Gemini: not configured".into(),
        });
    });
    Ok(lines.join("\n"))
}

fn read_secret_value(name: &str) -> Result<String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        print!("{name}: ");
        std::io::stdout().flush()?;
    }
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn set(key: &str, value: Option<String>) -> Result<String> {
    let mut config = Config::load()?;
    let need = |value: Option<String>| value.with_context(|| format!("`set {key}` needs a value"));
    match key {
        "server-url" => {
            let url = need(value)?.trim_end_matches('/').to_string();
            reqwest::Url::parse(&url).with_context(|| format!("{url:?} is not a valid URL"))?;
            config.server_url = url;
        }
        "client-id" => config.client_id = need(value)?,
        "gemini-model" => config.gemini_model = need(value)?,
        "client-secret" | "gemini-key" => {
            let secret = if key == "client-secret" { Secret::ClientSecret } else { Secret::GeminiApiKey };
            let value = match value {
                Some(v) if v != "-" => v,
                _ => read_secret_value(key)?,
            };
            if value.is_empty() {
                bail!("empty value; use `unset {key}` to remove it");
            }
            let location = config::set_secret(secret, &value)?;
            return Ok(format!("Saved {key} to {location}."));
        }
        _ => bail!("unknown setting {key:?}"),
    }
    config.save()?;
    Ok(format!("Saved {key}."))
}

fn unset(key: &str) -> Result<String> {
    let mut config = Config::load()?;
    match key {
        "server-url" => config.server_url.clear(),
        "client-id" => config.client_id.clear(),
        "gemini-model" => config.gemini_model = Config::default().gemini_model,
        "client-secret" => config::delete_secret(Secret::ClientSecret)?,
        "gemini-key" => config::delete_secret(Secret::GeminiApiKey)?,
        _ => bail!("unknown setting {key:?}"),
    }
    config.save()?;
    Ok(format!("Removed {key}."))
}

fn status() -> Result<String> {
    let config = Config::load()?;
    let providers = Providers::resolve(&config);
    let shown = |s: &str| if s.is_empty() { "(not set)".to_string() } else { s.to_string() };
    let secret = |s: Secret| if config::get_secret(s).is_some() { "set" } else { "(not set)" };
    Ok(format!(
        "Answers from:   {}\n\
         Server URL:     {}\n\
         Client ID:      {}\n\
         Client secret:  {}\n\
         Gemini key:     {}\n\
         Gemini model:   {}\n\
         Mode:           {}\n\
         Output:         {}\n\
         Hotkeys:        add {} · send {} · clear {}\n\
         Config folder:  {}",
        providers.describe(),
        shown(&config.server_url),
        shown(&config.client_id),
        secret(Secret::ClientSecret),
        secret(Secret::GeminiApiKey),
        config.gemini_model,
        config.mode.as_str(),
        config.output.as_str(),
        config.hotkeys.add,
        config.hotkeys.send,
        config.hotkeys.clear,
        config::config_dir().display(),
    ))
}

/// Runs a configuration command and returns what to print.
pub fn run_local(command: Command) -> Result<String> {
    match command {
        Command::Set { key, value } => set(&key, value),
        Command::Unset { key } => unset(&key),
        Command::Mode(mode) => {
            let mut config = Config::load()?;
            config.mode = mode;
            config.save()?;
            Ok(format!("Mode set to {}.", mode.as_str()))
        }
        Command::Output(output) => {
            let mut config = Config::load()?;
            config.output = output;
            config.save()?;
            Ok(format!("Output set to {}.", output.as_str()))
        }
        Command::Status => status(),
        Command::Capabilities => {
            let env = platform::detect();
            let config = Config::load().unwrap_or_default();
            let mut report = env.report();
            if env.hotkeys_supported {
                report += "\n  (Hotkeys are checked again when the tray app registers them.)\n";
            }
            let output = match (config.output, env.paste) {
                (OutputMode::Clipboard, _) => "copied to the clipboard (output: clipboard)",
                (_, Some(_)) => "pasted at the cursor",
                (_, None) => "copied to the clipboard (pasting isn't available here)",
            };
            Ok(report + &format!("\nAnswers will be {output}.\n"))
        }
        Command::Ask { texts, images } => ask(texts, images),
        Command::Test => test_connections(),
        Command::Help => Ok(USAGE.to_string()),
        Command::Run | Command::Action(_) => unreachable!("handled by the tray app"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command> {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p(&[]).unwrap(), Command::Run);
        assert_eq!(p(&["send"]).unwrap(), Command::Action(Action::Send));
        assert_eq!(p(&["mode", "hint"]).unwrap(), Command::Mode(Mode::Hint));
        assert_eq!(p(&["output", "clipboard"]).unwrap(), Command::Output(OutputMode::Clipboard));
        assert_eq!(p(&["set", "client-secret"]).unwrap(), Command::Set { key: "client-secret".into(), value: None });
        assert_eq!(
            p(&["set", "server-url", "https://x"]).unwrap(),
            Command::Set { key: "server-url".into(), value: Some("https://x".into()) }
        );
    }

    #[test]
    fn rejects_unknown_input() {
        assert!(p(&["mode", "essay"]).is_err());
        assert!(p(&["frobnicate"]).is_err());
        assert!(p(&["send", "now"]).is_err());
        assert!(p(&["ask"]).is_err());
        assert!(p(&["ask", "--image"]).is_err());
    }

    #[test]
    fn parses_ask() {
        assert_eq!(
            p(&["ask", "--image", "a.png", "what is", "this?"]).unwrap(),
            Command::Ask { texts: vec!["what is".into(), "this?".into()], images: vec!["a.png".into()] }
        );
    }
}
