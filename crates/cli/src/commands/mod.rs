pub mod ability;
pub mod account;
pub mod api;
pub mod assets;
pub mod billing;
pub mod characters;
pub mod color_presets;
pub mod compositions;
pub mod r#gen;
pub mod jobs;
pub mod masks;
pub mod models;
pub mod motions;
pub mod org;
pub mod pat;
pub mod products;
pub mod projects;
pub mod render;
pub mod restore;
pub mod skills;
pub mod status;
pub mod voices;
pub mod wait;

use crate::agent_guard::AgentMarker;
use crate::livejob::{self, LiveJob};
use crate::output::{OutputContext, OutputFormat};
use nolgia_client::Client;
use reqwest::StatusCode;
use uuid::Uuid;

/// RFC 7807 problem body the API returns on every error response.
#[derive(serde::Deserialize)]
struct Problem {
    title: Option<String>,
    detail: Option<String>,
    /// Machine-readable refusal code (`job_not_cancellable`, ...), when the
    /// route sets one.
    code: Option<String>,
}

/// The server's RFC 7807 `detail` (falling back to `title`, then to the raw
/// body), or `None` when there is nothing readable to show.
async fn problem_message(response: reqwest::Response) -> Option<String> {
    problem_parts(response).await.0
}

/// [`problem_message`] plus the problem's `code`, for callers that branch on
/// the refusal rather than on the status alone.
async fn problem_parts(response: reqwest::Response) -> (Option<String>, Option<String>) {
    let Ok(body) = response.text().await else {
        return (None, None);
    };
    let problem = serde_json::from_str::<Problem>(&body).ok();
    let code = problem.as_ref().and_then(|p| p.code.clone());
    let message = problem.and_then(|p| p.detail.or(p.title)).or_else(|| {
        let trimmed = body.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    (message, code)
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

/// [`api_error`] for `POST /jobs/{id}/cancel`, with the three refusals a
/// person can act on spelled out after the server's own `detail`.
///
/// Exit status stays the ordinary `1`: the cancel did not happen, and none of
/// these is a live job the caller must follow (`409` means the job is already
/// finishing on its own) or a content-filter block.
pub(crate) async fn cancel_error(err: nolgia_client::ApiError<()>, job_id: Uuid) -> anyhow::Error {
    let action = format!("canceling job {job_id}");
    if let nolgia_client::ApiError::UnexpectedResponse(response) = err {
        let status = response.status();
        let (message, code) = problem_parts(response).await;
        let hint = match status {
            StatusCode::CONFLICT if code.as_deref() == Some("job_not_cancellable") => {
                Some(format!(
                    "It already finished, or its finished result is being delivered, so it can no \
                 longer be canceled. Nothing was changed. Check it with: nolgia jobs get {job_id}"
                ))
            }
            StatusCode::NOT_FOUND => Some(format!(
                "There is no job {job_id} in your library in the active workspace. Check the id \
                 with `nolgia jobs list` and the workspace with `nolgia org status`."
            )),
            StatusCode::FORBIDDEN => Some(
                "Your role cannot cancel this job. In an organization, members cancel only their \
                 own jobs, owners and admins any job, and viewers and billing contacts none."
                    .to_string(),
            ),
            _ => None,
        };
        let described = describe(&action, status, message);
        return match hint {
            Some(hint) => anyhow::anyhow!("{described}\n  {hint}"),
            None => described,
        };
    }
    anyhow::Error::new(err).context(action)
}

pub struct CommandContext {
    client: Client,
    output: OutputContext,
    agent: Option<AgentMarker>,
}

impl CommandContext {
    pub fn new(client: Client, output: impl Into<OutputContext>) -> Self {
        Self {
            client,
            output: output.into(),
            agent: None,
        }
    }

    pub const fn with_agent(mut self, marker: Option<AgentMarker>) -> Self {
        self.agent = marker;
        self
    }

    pub const fn agent(&self) -> Option<AgentMarker> {
        self.agent
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn format(&self) -> OutputFormat {
        self.output.format()
    }

    pub fn output(&self) -> &OutputContext {
        &self.output
    }
}
