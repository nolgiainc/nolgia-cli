use serde_json::Value;
use std::{convert::Infallible, fmt, str::FromStr, time::Duration};

/// NOL-1019 owns the canonical list; keep unknown server codes intact.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    OutOfCredits,
    RateLimit,
    PromptNsfw,
    IpDetected,
    JobFailed,
    Timeout,
    Validation,
    ConfirmationRejected,
    /// The job was canceled by its owner (`POST /jobs/{id}/cancel`), not
    /// broken. `job["cancellation"]` says what the provider did and what was
    /// refunded.
    Canceled,
    /// `POST /jobs/{id}/cancel` refused (`409`): the job already finished, or
    /// its finished result is being delivered. Nothing was changed.
    JobNotCancellable,
    /// A code the server returned that this build does not know. The set is
    /// OPEN — pass it through, never coerce it.
    Other(String),
}

impl ErrorCode {
    pub fn from_wire(code: &str) -> Self {
        match code {
            "out_of_credits" => Self::OutOfCredits,
            "rate_limit" => Self::RateLimit,
            "prompt_nsfw" => Self::PromptNsfw,
            "ip_detected" => Self::IpDetected,
            "job_failed" => Self::JobFailed,
            "timeout" => Self::Timeout,
            "validation" => Self::Validation,
            "confirmation_rejected" => Self::ConfirmationRejected,
            "canceled" => Self::Canceled,
            "job_not_cancellable" => Self::JobNotCancellable,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::OutOfCredits => "out_of_credits",
            Self::RateLimit => "rate_limit",
            Self::PromptNsfw => "prompt_nsfw",
            Self::IpDetected => "ip_detected",
            Self::JobFailed => "job_failed",
            Self::Timeout => "timeout",
            Self::Validation => "validation",
            Self::ConfirmationRejected => "confirmation_rejected",
            Self::Canceled => "canceled",
            Self::JobNotCancellable => "job_not_cancellable",
            Self::Other(code) => code,
        }
    }
}
impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl FromStr for ErrorCode {
    type Err = Infallible;
    fn from_str(code: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_wire(code))
    }
}

#[derive(Debug, Clone)]
pub struct GenerationError {
    pub code: ErrorCode,
    pub message: String,
    pub http_status: Option<u16>,
    /// The job this error concerns. On a 409 duplicate refusal this is parsed
    /// out of the problem `detail` prose and is best-effort: see `from_body`.
    pub job_id: Option<String>,
    pub job: Option<Value>,
}
impl fmt::Display for GenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(id) = &self.job_id {
            write!(f, " (job {id})")?;
        }
        Ok(())
    }
}
impl std::error::Error for GenerationError {}
impl GenerationError {
    pub(super) fn local(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            http_status: None,
            job_id: None,
            job: None,
        }
    }
    pub(super) fn with_job(mut self, job: &Value) -> Self {
        self.job_id = text(job, "id");
        self.job = Some(job.clone());
        self
    }
    pub(super) fn from_body(body: &Value, http_status: Option<u16>) -> Self {
        // A canceled job is not a failure: it carries no `failure`, and the
        // server's own sentence (`cancellation.message`) is the message, as
        // in the TypeScript and Python layers.
        if http_status.is_none() && body["status"].as_str() == Some("canceled") {
            let message = ["/cancellation/message", "/status_message"]
                .into_iter()
                .find_map(|path| {
                    body.pointer(path)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("the job was canceled")
                .to_owned();
            return Self {
                code: ErrorCode::Canceled,
                message,
                http_status: None,
                job_id: None,
                job: None,
            };
        }
        let supplied = ["/failure/code", "/error/code", "/code"]
            .into_iter()
            .filter(|path| *path != "/code" || http_status.is_some())
            .find_map(|path| {
                body.pointer(path)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            });
        let code = supplied
            .map(ErrorCode::from_wire)
            .unwrap_or_else(|| match http_status {
                Some(402) => ErrorCode::OutOfCredits,
                Some(429) => ErrorCode::RateLimit,
                Some(400 | 422) => ErrorCode::Validation,
                Some(_) => ErrorCode::JobFailed,
                None if body.pointer("/failure/kind").and_then(Value::as_str)
                    == Some("moderated") =>
                {
                    // NOL-1019: the server cannot yet distinguish a safety refusal from a
                    // likeness/IP refusal, so IpDetected is only ever produced by rule 1
                    // (an explicit server code); PromptNsfw here is best-effort.
                    ErrorCode::PromptNsfw
                }
                None => ErrorCode::JobFailed,
            });
        let message = [
            "/failure/message",
            "/error/detail",
            "/error/message",
            "/detail",
            "/message",
            "/title",
        ]
        .into_iter()
        .find_map(|path| body.pointer(path).and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| match http_status {
            Some(status) => format!("HTTP {status}"),
            None => "generation failed".to_owned(),
        });
        // BEST-EFFORT, NOT A CONTRACT. A 409 duplicate refusal names the
        // already-accepted job only inside the English `detail` sentence:
        // the problem body carries no structured job id. Scraping the first
        // UUID out of prose is the only way to surface it today, and it stops
        // being correct the moment that sentence is reworded to mention some
        // other UUID first. Treat this as a hint for a human, never as
        // something to poll blindly. The real fix is a `job_id` field on that
        // problem body, raised with NOL-1019 on NOL-1023.
        let job_id = if http_status == Some(409) {
            body["detail"].as_str().and_then(|detail| {
                detail.as_bytes().windows(36).find_map(|candidate| {
                    let candidate = std::str::from_utf8(candidate).ok()?;
                    uuid::Uuid::parse_str(candidate)
                        .ok()
                        .map(|id| id.to_string())
                })
            })
        } else {
            None
        };
        Self {
            code,
            message,
            http_status,
            job_id,
            job: None,
        }
    }
}

/// The wait budget starts at `result()`. Timing out only stops waiting:
/// it does not cancel generation, and credits are still spent. To stop the
/// job itself, cancel it on the server with
/// [`ClientExt::cancel_job_with_body`](crate::ClientExt::cancel_job_with_body).
pub struct SubscribeOptions {
    pub poll_interval: Duration,
    pub max_poll_time: Duration,
    pub on_status: Option<Box<StatusCallback>>,
    /// Extra headers on submission, such as `Idempotency-Key`.
    pub headers: Vec<(String, String)>,
}
type StatusCallback = dyn Fn(&StatusUpdate) + Send + Sync;
impl Default for SubscribeOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(500),
            max_poll_time: Duration::from_secs(30 * 60),
            on_status: None,
            headers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusUpdate {
    pub job_id: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub status_message: Option<String>,
    pub progress: Option<f64>,
    pub job: Value,
}
impl StatusUpdate {
    pub(super) fn from_job(job: &Value) -> Self {
        Self {
            job_id: text(job, "id").unwrap_or_default(),
            status: text(job, "status").unwrap_or_default(),
            status_detail: text(job, "status_detail"),
            status_message: text(job, "status_message"),
            progress: job["progress"].as_f64(),
            job: job.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Media {
    pub asset_id: String,
    pub url: String,
    pub modality: String,
    pub expires_at: Option<String>,
}
#[derive(Debug, Clone)]
pub struct GenerationResult {
    pub job_id: String,
    pub job: Value,
    pub media: Vec<Media>,
    pub url: Option<String>,
}
pub(super) fn text(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_owned)
}
