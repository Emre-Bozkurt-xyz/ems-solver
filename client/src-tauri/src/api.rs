use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Solution,
    Hint,
    Checkwork,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Solution, Mode::Hint, Mode::Checkwork];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Solution => "solution",
            Mode::Hint => "hint",
            Mode::Checkwork => "checkwork",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Solution => "Solution",
            Mode::Hint => "Hint",
            Mode::Checkwork => "Check my work",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

/// One item of the input buffer, serialized exactly as the server API expects.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Part {
    Text { text: String },
    Image { media_type: String, data: String, #[serde(skip)] width: u32, #[serde(skip)] height: u32 },
}

impl Part {
    pub fn describe(&self) -> String {
        match self {
            Part::Text { text } => {
                let chars = text.chars().count();
                format!("text ({chars} chars)")
            }
            Part::Image { width, height, .. } => format!("image ({width}×{height})"),
        }
    }
}

pub fn summarize(parts: &[Part]) -> String {
    let images = parts.iter().filter(|p| matches!(p, Part::Image { .. })).count();
    let texts = parts.len() - images;
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    match (texts, images) {
        (0, 0) => "empty".into(),
        (t, 0) => plural(t, "text"),
        (0, i) => plural(i, "image"),
        (t, i) => format!("{}, {}", plural(t, "text"), plural(i, "image")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Server,
    Gemini,
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Provider::Server => "Claude (server)",
            Provider::Gemini => "Gemini",
        })
    }
}

#[derive(Clone, Debug)]
pub struct Answer {
    pub text: String,
    pub provider: Provider,
    pub model: String,
    pub truncated: bool,
    /// Set when this answer came from a fallback, explaining why.
    pub notice: Option<String>,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct SolveError {
    pub message: String,
    /// Whether another provider might still answer the same request.
    pub retryable: bool,
}

impl SolveError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self { message: message.into(), retryable: true }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        Self { message: message.into(), retryable: false }
    }
}
