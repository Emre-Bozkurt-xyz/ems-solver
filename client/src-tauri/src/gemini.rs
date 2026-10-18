//! Direct Gemini requests, used when no server is configured or the server is down.

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::api::{Answer, Mode, Part, Provider, SolveError};
use crate::prompt;

pub const DEFAULT_MODEL: &str = "gemini-3-flash-preview";

#[derive(Clone, Debug)]
pub struct GeminiCredentials {
    pub api_key: String,
    pub model: String,
}

fn request_body(mode: Mode, parts: &[Part]) -> Value {
    let mut contents = vec![json!({ "text": prompt::lead(mode) })];
    contents.extend(parts.iter().map(|part| match part {
        Part::Text { text } => json!({ "text": text }),
        Part::Image { media_type, data, .. } => json!({ "inline_data": { "mime_type": media_type, "data": data } }),
    }));
    json!({
        "systemInstruction": { "parts": [{ "text": prompt::SYSTEM_PROMPT }] },
        "contents": [{ "role": "user", "parts": contents }],
    })
}

fn parse_answer(body: &Value, model: &str) -> Result<Answer, SolveError> {
    if let Some(reason) = body.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
        return Err(SolveError::fatal(format!("Gemini blocked the request ({reason})")));
    }
    let candidate = body
        .pointer("/candidates/0")
        .ok_or_else(|| SolveError::fatal("Gemini returned no answer"))?;
    let text: String = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| !p.get("thought").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect();
    let finish = candidate.get("finishReason").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() {
        return Err(SolveError::fatal(format!("Gemini returned no text (finish reason: {finish})")));
    }
    Ok(Answer {
        text: text.trim().to_string(),
        provider: Provider::Gemini,
        model: model.to_string(),
        truncated: finish == "MAX_TOKENS",
        notice: None,
    })
}

/// Checks the key and model name without generating anything.
pub async fn check(http: &reqwest::Client, creds: &GeminiCredentials) -> Result<String, SolveError> {
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}", creds.model);
    let response = http
        .get(url)
        .header("x-goog-api-key", &creds.api_key)
        .send()
        .await
        .map_err(|e| SolveError::retryable(format!("Gemini unreachable: {e}")))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("no details");
        return Err(SolveError::fatal(format!("Gemini HTTP {status}: {detail}")));
    }
    let name = body.get("displayName").and_then(Value::as_str).unwrap_or(&creds.model);
    Ok(format!("Key works · {name}"))
}

pub async fn solve(
    http: &reqwest::Client,
    creds: &GeminiCredentials,
    mode: Mode,
    parts: &[Part],
) -> Result<Answer, SolveError> {
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", creds.model);
    let body = request_body(mode, parts);
    // The free tier often answers 429/503 under load; those usually clear within seconds.
    let backoff = [Duration::from_millis(1500), Duration::from_secs(4)];
    let mut attempt = 0;
    loop {
        let response = http
            .post(&url)
            .header("x-goog-api-key", &creds.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| SolveError::retryable(format!("Gemini unreachable: {e}")))?;

        let status = response.status();
        let reply: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return parse_answer(&reply, &creds.model);
        }
        let busy = status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE;
        if busy && attempt < backoff.len() {
            tokio::time::sleep(backoff[attempt]).await;
            attempt += 1;
            continue;
        }
        let detail = reply.pointer("/error/message").and_then(Value::as_str).unwrap_or("no details");
        return Err(SolveError { message: format!("Gemini HTTP {status}: {detail}"), retryable: status.is_server_error() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_keeps_part_order() {
        let parts = vec![
            Part::Text { text: "context".into() },
            Part::Image { media_type: "image/png".into(), data: "AAAA".into(), width: 1, height: 1 },
        ];
        let body = request_body(Mode::Hint, &parts);
        let sent = &body["contents"][0]["parts"];
        assert!(sent[0]["text"].as_str().unwrap().ends_with("Problem:"));
        assert_eq!(sent[1]["text"], "context");
        assert_eq!(sent[2]["inline_data"]["mime_type"], "image/png");
        assert!(body["systemInstruction"]["parts"][0]["text"].as_str().unwrap().contains("problem solver"));
    }

    #[test]
    fn parse_skips_thoughts_and_flags_truncation() {
        let body = json!({ "candidates": [{
            "content": { "parts": [{ "text": "thinking...", "thought": true }, { "text": "42" }] },
            "finishReason": "MAX_TOKENS"
        }]});
        let answer = parse_answer(&body, "m").unwrap();
        assert_eq!(answer.text, "42");
        assert!(answer.truncated);
    }

    #[test]
    fn parse_reports_blocks() {
        let body = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        assert!(parse_answer(&body, "m").unwrap_err().message.contains("SAFETY"));
    }
}
