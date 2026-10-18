//! Provider order: the server (Claude) first, then Gemini if the server can't answer.

use std::future::Future;
use std::time::Duration;

use crate::api::{Answer, Mode, Part, SolveError};
use crate::config::Providers;
use crate::{gemini, server};

pub fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("ems-solver/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        // The server sends a keepalive every 15 s while the model thinks.
        .read_timeout(Duration::from_secs(90))
        .timeout(Duration::from_secs(600))
        .build()
}

/// Ask the configured providers, in order.
pub async fn solve(http: &reqwest::Client, providers: &Providers, mode: Mode, parts: &[Part]) -> Result<Answer, SolveError> {
    route(
        providers.server.as_ref().map(|creds| server::solve(http, creds, mode, parts)),
        providers.gemini.as_ref().map(|creds| gemini::solve(http, creds, mode, parts)),
    )
    .await
}

/// Futures are lazy, so the Gemini request is only sent if it is awaited here.
pub async fn route<S, G>(server: Option<S>, gemini: Option<G>) -> Result<Answer, SolveError>
where
    S: Future<Output = Result<Answer, SolveError>>,
    G: Future<Output = Result<Answer, SolveError>>,
{
    let server_error = match server {
        Some(request) => match request.await {
            Ok(answer) => return Ok(answer),
            Err(err) if err.retryable && gemini.is_some() => Some(err),
            Err(err) => return Err(err),
        },
        None => None,
    };

    let Some(request) = gemini else {
        return Err(SolveError::fatal("not set up: add server credentials or a Gemini API key"));
    };
    match request.await {
        Ok(mut answer) => {
            answer.notice = server_error.map(|e| format!("Server failed ({e}); answered by Gemini."));
            Ok(answer)
        }
        Err(err) => Err(match server_error {
            Some(first) => SolveError::fatal(format!("{first}; Gemini fallback also failed: {err}")),
            None => err,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::api::Provider;

    fn answer(provider: Provider) -> Answer {
        Answer { text: "ok".into(), provider, model: "m".into(), truncated: false, notice: None }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(f)
    }

    type Pending = std::future::Ready<Result<Answer, SolveError>>;

    #[test]
    fn server_success_skips_gemini() {
        let gemini_called = Cell::new(false);
        let result = block_on(route(
            Some(async { Ok(answer(Provider::Server)) }),
            Some(async {
                gemini_called.set(true);
                Ok(answer(Provider::Gemini))
            }),
        ));
        assert_eq!(result.unwrap().provider, Provider::Server);
        assert!(!gemini_called.get());
    }

    #[test]
    fn retryable_server_error_falls_back() {
        let result = block_on(route(
            Some(async { Err(SolveError::retryable("server unreachable")) }),
            Some(async { Ok(answer(Provider::Gemini)) }),
        ));
        let answer = result.unwrap();
        assert_eq!(answer.provider, Provider::Gemini);
        assert!(answer.notice.unwrap().contains("server unreachable"));
    }

    #[test]
    fn fatal_server_error_does_not_fall_back() {
        let result = block_on(route(
            Some(async { Err(SolveError::fatal("invalid credentials")) }),
            Some(async { Ok(answer(Provider::Gemini)) }),
        ));
        assert_eq!(result.unwrap_err().message, "invalid credentials");
    }

    #[test]
    fn both_failing_reports_both() {
        let result = block_on(route(
            Some(async { Err(SolveError::retryable("server down")) }),
            Some(async { Err(SolveError::fatal("bad key")) }),
        ));
        let message = result.unwrap_err().message;
        assert!(message.contains("server down") && message.contains("bad key"));
    }

    #[test]
    fn gemini_only_and_nothing_configured() {
        assert_eq!(block_on(route(None::<Pending>, Some(async { Ok(answer(Provider::Gemini)) }))).unwrap().provider, Provider::Gemini);
        assert!(block_on(route(None::<Pending>, None::<Pending>)).unwrap_err().message.contains("not set up"));
    }
}
