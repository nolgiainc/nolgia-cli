mod generated {
    #![allow(clippy::all)]
    #![allow(clippy::unwrap_used)]
    #![allow(unused_imports)]
    // Schemas with `minLength: 0` (e.g. SubmitAgentMessageRequest.content)
    // generate an always-false `chars().count() < 0usize` guard.
    #![allow(unused_comparisons)]

    include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
}

use std::{fmt, result::Result as StdResult};

use generated::ClientInfo;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use uuid::Uuid;

pub use generated::{Client, Error as ApiError, ResponseValue, types};

/// Extension helpers on the generated [`Client`] that need behavior the
/// generated builders cannot express.
///
/// The generated `UpdateAssetRequest.tags` field derives
/// `skip_serializing_if = "Vec::is_empty"` (the spec models it as a plain,
/// non-nullable array), so the typed builder can never emit `{"tags": []}` —
/// an empty vec is dropped entirely. The API distinguishes an omitted `tags`
/// (leave unchanged) from an explicit empty array (clear all tags), so
/// `assets tag --clear` must send the empty array literally. This helper does
/// exactly that via a raw request that reuses the client's auth/base-url.
pub trait ClientExt {
    /// PATCH `/assets/{id}` with `{"tags": []}` to clear an asset's tag set,
    /// returning the updated [`types::Asset`].
    fn clear_asset_tags(
        &self,
        id: Uuid,
    ) -> impl std::future::Future<Output = StdResult<types::Asset, ApiError<()>>> + Send;

    /// POST `/assets/uploads/{id}/complete` to finish a signed upload,
    /// returning the ready [`types::Asset`].
    ///
    /// This mirrors the generated `complete_asset_upload` builder but sends an
    /// explicit empty body so a `Content-Length: 0` header is emitted. The
    /// generated bodyless POST omits `Content-Length` entirely, which the
    /// production load balancer rejects with `411 Length Required` before the
    /// request ever reaches the API.
    fn finish_asset_upload(
        &self,
        upload_id: Uuid,
    ) -> impl std::future::Future<Output = StdResult<types::Asset, ApiError<()>>> + Send;
}

impl ClientExt for Client {
    async fn clear_asset_tags(&self, id: Uuid) -> StdResult<types::Asset, ApiError<()>> {
        let url = format!("{}/assets/{}", self.baseurl(), id);
        let response = self
            .client()
            .patch(url)
            .json(&serde_json::json!({ "tags": [] }))
            .send()
            .await?;
        let response = response.error_for_status()?;
        Ok(response.json::<types::Asset>().await?)
    }

    async fn finish_asset_upload(&self, upload_id: Uuid) -> StdResult<types::Asset, ApiError<()>> {
        let url = format!("{}/assets/uploads/{}/complete", self.baseurl(), upload_id);
        let response = self
            .client()
            .post(url)
            // reqwest omits `Content-Length` for a bodyless (or empty-body)
            // POST, which the production LB rejects with 411. Set it
            // explicitly so the request carries `Content-Length: 0`.
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .body(Vec::<u8>::new())
            .send()
            .await?;
        let response = response.error_for_status()?;
        Ok(response.json::<types::Asset>().await?)
    }
}

#[derive(Debug, Clone)]
pub struct ClientBuilder {
    base_url: String,
    auth_token: Option<String>,
    surface: Option<String>,
    idempotency_key: Option<String>,
}

#[derive(Debug)]
pub enum ClientBuilderError {
    InvalidAuthorization(reqwest::header::InvalidHeaderValue),
    InvalidIdempotencyKey(String),
    Transport(reqwest::Error),
}

impl fmt::Display for ClientBuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAuthorization(err) => write!(f, "invalid authorization header: {err}"),
            Self::InvalidIdempotencyKey(key) => write!(
                f,
                "--idempotency-key {key:?} cannot be sent as an HTTP header \
                 (use printable ASCII, no newlines)"
            ),
            Self::Transport(err) => write!(f, "failed to construct HTTP client: {err}"),
        }
    }
}

impl std::error::Error for ClientBuilderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidAuthorization(err) => Some(err),
            Self::InvalidIdempotencyKey(_) => None,
            Self::Transport(err) => Some(err),
        }
    }
}

impl From<reqwest::header::InvalidHeaderValue> for ClientBuilderError {
    fn from(err: reqwest::header::InvalidHeaderValue) -> Self {
        Self::InvalidAuthorization(err)
    }
}

impl From<reqwest::Error> for ClientBuilderError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err)
    }
}

impl ClientBuilder {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            auth_token: None,
            surface: None,
            idempotency_key: None,
        }
    }

    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    pub fn pat(mut self, token: impl Into<String>) -> Self {
        self.auth_token = Some(token.into());
        self
    }

    /// Identify the calling surface ("cli", "claude-code", "codex", ...) —
    /// sent as X-Nolgia-Surface so the platform can understand agent-driven
    /// usage.
    pub fn surface(mut self, surface: impl Into<String>) -> Self {
        self.surface = Some(surface.into());
        self
    }

    /// Set `Idempotency-Key` on every request this client makes.
    ///
    /// The API claims `(user, request fingerprint)` before the credit hold and
    /// before the provider call, and refuses a second identical submission
    /// with `409`. Absent this header the fingerprint is a hash of the request
    /// body — which is what catches a human blindly re-running a command — so
    /// an explicit key is the only way to say "yes, I meant to run that exact
    /// request again". It cannot be expressed through the generated builders:
    /// the header is accepted by the API but is not declared in the OpenAPI
    /// spec, so progenitor emits no parameter for it.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn build(self) -> StdResult<Client, ClientBuilderError> {
        let mut headers = HeaderMap::new();

        if let Some(token) = self.auth_token {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))?;
            headers.insert(AUTHORIZATION, value);
        }

        if let Some(surface) = self.surface
            && let Ok(value) = HeaderValue::from_str(&surface)
        {
            headers.insert("x-nolgia-surface", value);
        }

        // Never dropped silently the way an unusable surface is: a caller who
        // passed a key is deliberately trying to run an identical request
        // again, and quietly omitting it would hand them the very `409` the
        // key exists to avoid.
        if let Some(key) = self.idempotency_key {
            let value = HeaderValue::from_str(&key)
                .map_err(|_| ClientBuilderError::InvalidIdempotencyKey(key))?;
            headers.insert("idempotency-key", value);
        }

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Client::new_with_client(
            &normalize_base_url(&self.base_url),
            http_client,
        ))
    }
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');

    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}
