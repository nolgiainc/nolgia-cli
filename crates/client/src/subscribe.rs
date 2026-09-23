//! Hand-written. This module is NOT generated. `build.rs` runs progenitor over
//! the vendored `openapi.yaml` into `OUT_DIR/codegen.rs`; nothing in the codegen
//! path writes to this file, so re-vendoring the spec can never clobber this
//! ergonomics layer.

mod types;
use crate::{Client, generated::ClientInfo};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};
use tokio::{
    sync::watch,
    time::{sleep, timeout},
};
use types::text;
pub use types::{
    ErrorCode, GenerationError, GenerationResult, Media, StatusUpdate, SubscribeOptions,
};

/// Submit a generation and wait for its assets. Timeout stops only this wait;
/// the server keeps generating and credits are still spent.
pub async fn subscribe(
    client: &Client,
    endpoint: &str,
    arguments: Value,
    options: SubscribeOptions,
) -> Result<GenerationResult, GenerationError> {
    submit(client, endpoint, arguments, options)
        .await?
        .result()
        .await
}

/// Submit without polling. Await `JobHandle::result` to start waiting.
pub async fn submit(
    client: &Client,
    endpoint: &str,
    arguments: Value,
    options: SubscribeOptions,
) -> Result<JobHandle, GenerationError> {
    // /generate/set returns an OutputSet polled at /sets/{id}, not a Job.
    // Accepting it here would use the wrong response shape and poll route.
    if !matches!(
        endpoint,
        "/generate/image"
            | "/generate/audio"
            | "/generate/video"
            | "/generate/3d"
            | "/restore/video"
    ) {
        return Err(GenerationError::local(
            ErrorCode::Validation,
            format!("unsupported generation endpoint: {endpoint}"),
        ));
    }
    let mut request = client
        .client()
        .post(format!("{}{endpoint}", client.baseurl()))
        .json(&arguments);
    for (name, value) in &options.headers {
        request = request.header(name, value);
    }
    let job = request_json(request).await?;
    let id = text(&job, "id")
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            GenerationError::local(ErrorCode::JobFailed, "submit response is missing a job id")
                .with_job(&job)
        })?;
    if let Some(callback) = &options.on_status {
        callback(&StatusUpdate::from_job(&job));
    }
    Ok(JobHandle {
        client: client.clone(),
        id,
        job,
        options: Arc::new(options),
        stop: watch::channel(Stop::Waiting).0,
    })
}

/// Why a `result()` wait ended early, shared by every clone of a handle.
#[derive(Clone)]
enum Stop {
    Waiting,
    /// The deprecated local-only [`JobHandle::cancel`].
    Local,
    /// [`JobHandle::cancel_job`] canceled the job on the server; this is the
    /// canceled job it returned.
    Server(Value),
}

/// A submitted job. Clone the handle before consuming it with `result()` to
/// retain access to [`cancel_job`](Self::cancel_job) while waiting; a cancel
/// is shared by all clones.
#[derive(Clone)]
pub struct JobHandle {
    client: Client,
    id: String,
    job: Value,
    options: Arc<SubscribeOptions>,
    stop: watch::Sender<Stop>,
}
impl JobHandle {
    pub fn job_id(&self) -> &str {
        &self.id
    }
    /// The original submit response, including unknown fields.
    pub const fn job(&self) -> &Value {
        &self.job
    }
    /// Fetch the current job once, without starting the polling loop.
    pub async fn status(&self) -> Result<Value, GenerationError> {
        let url = format!(
            "{}/jobs/{}",
            self.client.baseurl(),
            progenitor_client::encode_path(&self.id)
        );
        request_json(self.client.client().get(url))
            .await
            .map_err(|err| err.with_job(&self.job))
    }
    /// Cancel the job on the server (`POST /jobs/{id}/cancel`) and stop
    /// waiting for it.
    ///
    /// Returns the canceled job, raw like [`status`](Self::status): its
    /// `cancellation` says what the model provider did and whether the credits
    /// were refunded. A job that had not reached the provider is refunded in
    /// full; a started render is refunded only when the provider stops it
    /// without billing, so `cancellation.settlement` can read `pending` until
    /// the provider answers (fetch the job again later to see how it settled).
    /// A canceled job is never added to the library. A pending or later
    /// [`result`](Self::result) on this handle or any clone then fails with
    /// [`ErrorCode::Canceled`], whose message is the server's
    /// `cancellation.message`.
    ///
    /// A job that already finished, or whose result is being delivered, is
    /// refused with [`ErrorCode::JobNotCancellable`] (HTTP `409`); nothing is
    /// changed and a pending `result()` keeps waiting for its own outcome.
    pub async fn cancel_job(&self) -> Result<Value, GenerationError> {
        let canceled = request_json(crate::cancel_job_request(&self.client, &self.id))
            .await
            .map_err(|err| err.with_job(&self.job))?;
        self.stop.send_replace(Stop::Server(canceled.clone()));
        Ok(canceled)
    }
    /// Stop this client waiting, including pending `result()` calls on clones.
    /// This sends no server call: it does not cancel the generation or refund
    /// credits, and the job keeps running, is billed, and its asset still
    /// lands in the library. Use [`cancel_job`](Self::cancel_job) to cancel
    /// the job on the server.
    #[deprecated(
        note = "stops only the local wait; the job keeps running and is billed. Use cancel_job() to cancel it on the server"
    )]
    pub fn cancel(&self) {
        self.stop.send_replace(Stop::Local);
    }
    /// Wait for completion. The budget starts here and also bounds in-flight
    /// requests. Timeout does not cancel generation or refund credits; use
    /// [`cancel_job`](Self::cancel_job) for that.
    pub async fn result(self) -> Result<GenerationResult, GenerationError> {
        let mut stopped = self.stop.subscribe();
        let mut job = self.job.clone();
        let outcome = tokio::select! {
            biased;
            stop = stopped.wait_for(|stop| !matches!(stop, Stop::Waiting)) => Err(stop.map_or(Stop::Local, |stop| stop.clone())),
            outcome = timeout(self.options.max_poll_time, self.poll(&mut job)) => Ok(outcome.unwrap_or_else(|_| Err(GenerationError::local(ErrorCode::Timeout, "wait timed out; generation continues on the server and credits are still spent")))),
        };
        match outcome {
            Ok(outcome) => outcome.map_err(|err| err.with_job(&job)),
            // The server's canceled job, not the last one polled: it carries
            // the cancellation the error's message comes from.
            Err(Stop::Server(canceled)) => {
                Err(GenerationError::from_body(&canceled, None).with_job(&canceled))
            }
            Err(Stop::Local | Stop::Waiting) => Err(GenerationError::local(
                ErrorCode::JobFailed,
                "wait was cancelled locally; generation continues on the server",
            )
            .with_job(&job)),
        }
    }
    async fn poll(&self, job: &mut Value) -> Result<GenerationResult, GenerationError> {
        let mut failures = 0;
        loop {
            match job["status"].as_str() {
                Some("succeeded") => return self.resolve_media(job).await,
                Some("failed" | "canceled") => return Err(GenerationError::from_body(job, None)),
                Some("queued" | "running") => {}
                _ => {
                    return Err(GenerationError::local(
                        ErrorCode::JobFailed,
                        "unrecognized job status",
                    ));
                }
            }
            sleep(self.options.poll_interval).await;
            match self.status().await {
                Ok(next) => {
                    failures = 0;
                    let before = StatusUpdate::from_job(job);
                    let after = StatusUpdate::from_job(&next);
                    let changed = before.status != after.status
                        || before.status_detail != after.status_detail
                        || before.status_message != after.status_message
                        || before.progress != after.progress;
                    *job = next;
                    if changed && let Some(callback) = &self.options.on_status {
                        callback(&after);
                    }
                }
                Err(err) => {
                    failures += 1;
                    if failures >= 5 || !matches!(err.http_status, None | Some(500..=599)) {
                        return Err(err);
                    }
                }
            }
        }
    }
    async fn resolve_media(&self, job: &Value) -> Result<GenerationResult, GenerationError> {
        // A job can produce multiple assets; the inline job.asset carries only
        // one. List by job id first to preserve the complete server ordering.
        //
        // A failed listing does NOT fail the generation. By the time we get
        // here the job has succeeded and the customer has been charged, so
        // throwing away a paid, completed render because a secondary read got
        // a 503 is the wrong default: fall back to the inline asset and let
        // the caller inspect `job`. The TypeScript and Python layers behave
        // identically (nolgia-api#527) and all three READMEs document it, so
        // do not make one of them diverge.
        let list = request_json(
            self.client
                .client()
                .get(format!("{}/assets", self.client.baseurl()))
                .query(&[("job_id", self.id.as_str()), ("limit", "100")]),
        )
        .await
        .unwrap_or(Value::Null);
        let items = list["items"].as_array();
        let assets: Vec<&Value> = match items {
            Some(items) if !items.is_empty() => items.iter().collect(),
            _ => job
                .get("asset")
                .filter(|asset| asset.is_object())
                .into_iter()
                .collect(),
        };
        let mut seen = HashSet::new();
        let media: Vec<Media> = assets
            .into_iter()
            .filter_map(|asset| {
                let id = text(asset, "id")?;
                if !seen.insert(id.clone()) {
                    return None;
                }
                Some(Media {
                    asset_id: id,
                    url: text(asset, "signed_url")?,
                    modality: text(asset, "modality")?,
                    expires_at: text(asset, "expires_at"),
                })
            })
            .collect();
        Ok(GenerationResult {
            job_id: self.id.clone(),
            job: job.clone(),
            url: media.first().map(|media| media.url.clone()),
            media,
        })
    }
}

async fn request_json(request: reqwest::RequestBuilder) -> Result<Value, GenerationError> {
    let response = request.send().await.map_err(|err| {
        GenerationError::local(ErrorCode::JobFailed, err.without_url().to_string())
    })?;
    let status = response.status();
    let body = response.bytes().await;
    if !status.is_success() {
        return Err(GenerationError::from_body(
            &body
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .unwrap_or(Value::Null),
            Some(status.as_u16()),
        ));
    }
    let bytes = body.map_err(|err| {
        GenerationError::local(ErrorCode::JobFailed, err.without_url().to_string())
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        let mut error = GenerationError::local(ErrorCode::JobFailed, err.to_string());
        error.http_status = Some(status.as_u16());
        error
    })
}
