pub mod ability;
pub mod account;
pub mod assets;
pub mod billing;
pub mod characters;
pub mod color_presets;
pub mod r#gen;
pub mod masks;
pub mod models;
pub mod org;
pub mod pat;
pub mod projects;
pub mod restore;
pub mod skills;
pub mod status;
pub mod wait;

use crate::livejob::{self, LiveJob};
use crate::output::OutputFormat;
use nolgia_client::Client;
use reqwest::StatusCode;
use uuid::Uuid;

/// RFC 7807 problem body the API returns on every error response.
#[derive(serde::Deserialize)]
struct Problem {
    title: Option<String>,
    detail: Option<String>,
}

/// The server's RFC 7807 `detail` (falling back to `title`, then to the raw
/// body), or `None` when there is nothing readable to show.
async fn problem_message(response: reqwest::Response) -> Option<String> {
    let body = response.text().await.ok()?;
    serde_json::from_str::<Problem>(&body)
        .ok()
        .and_then(|p| p.detail.or(p.title))
        .or_else(|| {
            let trimmed = body.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn describe(action: &str, status: StatusCode, message: Option<String>) -> anyhow::Error {
    match message {
        Some(message) => anyhow::anyhow!("{action}: {status}: {message}"),
        None => anyhow::anyhow!("{action}: {status}"),
    }
}

/// Convert a generated-client error into an anyhow error that surfaces the
/// server's RFC 7807 `detail` verbatim. The API validates requests against
/// per-model capabilities and names the violated capability in `detail`
/// (e.g. available quality tiers, reference caps) — far more actionable
/// than progenitor's opaque "Unexpected Response" debug dump.
pub(crate) async fn api_error(err: nolgia_client::ApiError<()>, action: &str) -> anyhow::Error {
    if let nolgia_client::ApiError::UnexpectedResponse(response) = err {
        let status = response.status();
        let message = problem_message(response).await;
        return describe(action, status, message);
    }
    anyhow::Error::new(err).context(action.to_string())
}

/// [`api_error`] for a generation submission, with one extra case: a `409`
/// means the request was refused because it is already a job.
///
/// That refusal is not really an error from the caller's point of view — it is
/// the platform telling them the work they asked for is already underway and
/// was not billed twice — and it carries the one fact the original incident
/// lost: the existing job's id. Recognising it here promotes it out of a
/// generic error string and into the same "a job is live" rendering as every
/// other way of arriving at that fact.
///
/// If the id cannot be found in the prose, this degrades to exactly the
/// previous behavior: the server's `detail`, verbatim.
///
/// `retry_command` is the invocation that submitted (`nolgia gen video`,
/// `nolgia restore video`, ...); the duplicate rendering echoes it so its
/// "run it again deliberately" hint is a command the caller can actually run.
pub(crate) async fn submit_error(
    err: nolgia_client::ApiError<()>,
    action: &str,
    retry_command: &str,
) -> anyhow::Error {
    if let nolgia_client::ApiError::UnexpectedResponse(response) = err {
        let status = response.status();
        let message = problem_message(response).await;
        if status == StatusCode::CONFLICT
            && let Some(detail) = message.as_deref()
            && let Some(job_id) = livejob::find_job_id(detail)
        {
            return LiveJob::Duplicate {
                job_id,
                detail: detail.to_string(),
                retry_command: retry_command.to_string(),
            }
            .into();
        }
        return describe(action, status, message);
    }
    anyhow::Error::new(err).context(action.to_string())
}

/// [`api_error`] for `GET /jobs/{id}/wait`, with one extra case: a `408` is
/// not a failure.
///
/// The long-poll window closed; the job is still running and still ours to
/// follow. The response body does not name the job (`{"detail":"job did not
/// finish before timeout"}` is all the server sends), so the id comes from the
/// caller — which has it, because it just put it in the request URL.
pub(crate) async fn wait_error(
    err: nolgia_client::ApiError<()>,
    action: &str,
    job_id: Uuid,
    waited_seconds: u64,
) -> anyhow::Error {
    if let nolgia_client::ApiError::UnexpectedResponse(response) = err {
        let status = response.status();
        if status == StatusCode::REQUEST_TIMEOUT {
            return LiveJob::StillRunning {
                job_id,
                waited_seconds,
            }
            .into();
        }
        let message = problem_message(response).await;
        return describe(action, status, message);
    }
    anyhow::Error::new(err).context(action.to_string())
}

pub struct CommandContext {
    client: Client,
    format: OutputFormat,
}

impl CommandContext {
    pub fn new(client: Client, format: OutputFormat) -> Self {
        Self { client, format }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn format(&self) -> OutputFormat {
        self.format
    }
}
