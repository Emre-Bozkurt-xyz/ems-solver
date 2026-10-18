//! Client for the ems-solver server's streaming endpoint (docs/api.md).

use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::api::{Answer, Mode, Part, Provider, SolveError};
use crate::{signing, sse};

const STREAM_PATH: &str = "/v1/solve/stream";

#[derive(Clone, Debug)]
pub struct ServerCredentials {
    pub url: String,
    pub client_id: String,
    pub secret: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
    #[serde(default)]
    retryable: bool,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct Done {
    result: String,
    model: String,
    stop_reason: Option<String>,
}

fn error_from_body(err: ErrorBody) -> SolveError {
    SolveError { message: format!("server: {} ({})", err.message, err.code), retryable: err.retryable }
}

/// Splits "https://host/prefix" into the URL to call and the path to sign.
fn endpoint(base_url: &str) -> Result<(String, String), SolveError> {
    let base = reqwest::Url::parse(base_url.trim())
        .map_err(|e| SolveError::fatal(format!("invalid server URL {base_url:?}: {e}")))?;
    let path = format!("{}{STREAM_PATH}", base.path().trim_end_matches('/'));
    let mut url = base.clone();
    url.set_path(&path);
    Ok((url.to_string(), path))
}

pub async fn solve(
    http: &reqwest::Client,
    creds: &ServerCredentials,
    mode: Mode,
    parts: &[Part],
) -> Result<Answer, SolveError> {
    let (url, path) = endpoint(&creds.url)?;
    let body = serde_json::to_vec(&json!({ "mode": mode, "parts": parts })).expect("request serializes");
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().to_string();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let signature = signing::sign(&creds.secret, "POST", &path, &creds.client_id, &timestamp, &nonce, &body);

    let response = http
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("X-Ems-Client-Id", &creds.client_id)
        .header("X-Ems-Timestamp", &timestamp)
        .header("X-Ems-Nonce", &nonce)
        .header("X-Ems-Signature", signature)
        .body(body)
        .send()
        .await
        .map_err(|e| SolveError::retryable(format!("server unreachable: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(match serde_json::from_str::<ErrorEnvelope>(&text) {
            Ok(envelope) => error_from_body(envelope.error),
            // No JSON error body: a proxy (Cloudflare, frp) answered, so the server is down.
            Err(_) => SolveError { message: format!("server returned HTTP {status}"), retryable: status.is_server_error() },
        });
    }

    let mut parser = sse::Parser::default();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| SolveError::retryable(format!("connection to server lost: {e}")))?;
        for event in parser.feed(&String::from_utf8_lossy(&chunk)) {
            match event.event.as_str() {
                "done" => {
                    let done: Done = serde_json::from_str(&event.data)
                        .map_err(|e| SolveError::retryable(format!("bad response from server: {e}")))?;
                    return Ok(Answer {
                        text: done.result,
                        provider: Provider::Server,
                        model: done.model,
                        truncated: done.stop_reason.as_deref() == Some("max_tokens"),
                        notice: None,
                    });
                }
                "error" => {
                    let err: ErrorBody = serde_json::from_str(&event.data)
                        .map_err(|e| SolveError::retryable(format!("bad error from server: {e}")))?;
                    return Err(error_from_body(err));
                }
                // Deltas are for a future live view; the final answer arrives in `done`.
                _ => {}
            }
        }
    }
    Err(SolveError::retryable("server closed the connection before answering"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_keeps_path_prefix() {
        assert_eq!(
            endpoint("https://example.com").unwrap(),
            ("https://example.com/v1/solve/stream".into(), "/v1/solve/stream".into())
        );
        assert_eq!(
            endpoint("https://example.com/solver/").unwrap(),
            ("https://example.com/solver/v1/solve/stream".into(), "/solver/v1/solve/stream".into())
        );
        assert!(endpoint("not a url").is_err());
    }
}
