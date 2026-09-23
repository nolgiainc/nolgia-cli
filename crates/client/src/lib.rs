//! The Rust client for the [Nolgia](https://nolgia.ai) API.
//!
//! `cargo add nolgia-client` is the whole install. The crate carries its own
//! async runtime, JSON macro and HTTP stack and re-exports what a first
//! program needs, so no example here asks you to add a second crate.
//!
//! ```no_run
//! use nolgia_client::{ClientExt, tokio};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let nolgia = nolgia_client::client()?; // reads NOLGIA_TOKEN
//!     let image = nolgia_client::subscribe(
//!         &nolgia,
//!         "/generate/image",
//!         nolgia_client::json!({"model": "flux-pro", "prompt": "a paper-cut mountain range at dawn"}),
//!         Default::default(),
//!     )
//!     .await?;
//!     nolgia.download(&image.url.unwrap_or_default(), "first.png").await?;
//!     Ok(())
//! }
//! ```
//!
//! `use nolgia_client::tokio;` is what makes `#[tokio::main]` resolve — see
//! [`rt`] for that and for the synchronous alternative.

mod generated {
    #![allow(clippy::all)]
    #![allow(clippy::unwrap_used)]
    #![allow(unused_imports)]
    // Schemas with `minLength: 0` (e.g. SubmitAgentMessageRequest.content)
    // generate an always-false `chars().count() < 0usize` guard.
    #![allow(unused_comparisons)]
    // progenitor's `defaults` module can carry helpers nothing calls. typify
    // (typify-impl 0.6.2, `src/defaults.rs`) accounts for a default twice, and
    // the two halves disagree for `type: number` properties: `validate_value`
    // maps `TypeEntryDetails::Float` with a non-zero default to
    // `DefaultKind::Generic(DefaultImpl::I64)`, which *registers* the generic
    // `defaults::default_i64` helper, while `default_fn` has no `Float` arm at
    // all and falls through to emitting a *bespoke* `<type>_<prop>()` function
    // instead. So every float property with a non-zero default emits
    // `default_i64` and then never references it (`default_i64` is only ever
    // called for a negative *integer* default). `MotionAnchor.{x,y}`
    // (`default: 50`, api#345 motion keyframes) is the first such property to
    // reach the vendored spec, and under CI's `-D warnings` its dead helper is
    // a hard build error.
    //
    // This cannot be fixed in build.rs's spec rewrites: the emission is
    // unconditional for the schema shape, so the only way to suppress it there
    // is to delete the `default` (as `unmaterialize_server_side_defaults`
    // does for `Mask`), which changes the generated type and would have to be
    // repeated by hand for every numeric default a future re-vendor adds —
    // re-breaking the automated re-vendor branch each time. Dead code in a
    // generated file is never actionable anyway, so it is allowed here, at the
    // only place `codegen.rs` is included, and nowhere else in the workspace.
    #![allow(dead_code)]

    include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
}

use std::{fmt, result::Result as StdResult};

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use uuid::Uuid;

pub use generated::{Client, ClientInfo, Error as ApiError, ResponseValue, types};

/// The production API, and the default for [`ClientBuilder::from_env`].
pub const DEFAULT_BASE_URL: &str = "https://api.nolgia.ai";

// ONE COMMAND MUST BE ENOUGH (NOL-1066).
//
// This crate already compiles tokio, serde_json and reqwest, yet a caller who
// ran `cargo add nolgia-client` and nothing else could not write the first
// example: `#[tokio::main]` and `json!` need those crates in THEIR namespace,
// so the docs had to say `cargo add nolgia-client tokio reqwest --features
// tokio/full` — telling people to add crates we already carry. Re-exporting
// them here is what makes `cargo add nolgia-client` the whole install.
//
// These are part of the public API and are covered by semver: a major bump of
// tokio or serde_json is a breaking change for this crate too.
pub use serde_json;
pub use serde_json::{Value, json};
pub use tokio;

/// The async runtime, for callers who would rather not name tokio.
///
/// # Getting an `async fn main`, three ways that work
///
/// Measured against a fresh `cargo new` with `nolgia-client` as the only
/// dependency (NOL-1087) — this is not theory, and the obvious fourth way
/// does not work:
///
/// ```ignore
/// #[nolgia_client::rt::main]        // ✗ error[E0433]: cannot find crate `tokio`
/// ```
///
/// [`main`] is tokio's own attribute macro, and its expansion names a **bare**
/// `tokio`, which has to resolve in *your* crate. So either put it there:
///
/// ```no_run
/// use nolgia_client::tokio;         // ✓ the shortest form, and idiomatic
///
/// #[tokio::main]
/// async fn main() {}
/// ```
///
/// or tell the macro where the runtime lives, which needs no `use` at all:
///
/// ```no_run
/// #[nolgia_client::rt::main(crate = "nolgia_client::tokio")]   // ✓
/// async fn main() {}
/// ```
///
/// or keep `fn main` synchronous and run the one async call with
/// [`block_on`]:
///
/// ```no_run
/// fn main() {                                                  // ✓
///     nolgia_client::rt::block_on(async {
///         let _ = nolgia_client::client();
///     });
/// }
/// ```
// The `block_on` example's whole point is a synchronous `fn main`, which is
// exactly what `needless_doctest_main` exists to remove. Keeping the example
// compiled is worth the allow; the alternative is an `ignore` fence that
// nothing checks.
#[allow(clippy::needless_doctest_main)]
pub mod rt {
    /// tokio's `main` attribute.
    ///
    /// Its expansion names a bare `tokio`, so applied as
    /// `#[nolgia_client::rt::main]` it fails to compile in a crate that does
    /// not depend on tokio directly. Pass
    /// `#[nolgia_client::rt::main(crate = "nolgia_client::tokio")]`, or
    /// `use nolgia_client::tokio;` and write `#[tokio::main]`. See the
    /// [module docs](self).
    pub use tokio::main;

    /// Run `future` to completion on a fresh current-thread runtime.
    ///
    /// For a plain `fn main` that needs one async call. Do **not** call this
    /// from inside an existing runtime (including from `#[tokio::main]`):
    /// starting a runtime within a runtime panics.
    pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("building a current-thread Tokio runtime")
            .block_on(future)
    }
}

/// A [`Client`] from the environment: `NOLGIA_TOKEN`, and `NOLGIA_API_URL`
/// when it is set (otherwise [`DEFAULT_BASE_URL`]).
///
/// ```no_run
/// use nolgia_client::{ClientExt, tokio};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let nolgia = nolgia_client::client()?;
///     let result = nolgia_client::subscribe(
///         &nolgia,
///         "/generate/image",
///         nolgia_client::json!({"model": "flux-pro", "prompt": "a paper-cut mountain range"}),
///         Default::default(),
///     )
///     .await?;
///     nolgia.download(&result.url.unwrap_or_default(), "first.png").await?;
///     Ok(())
/// }
/// ```
pub fn client() -> StdResult<Client, ClientBuilderError> {
    ClientBuilder::from_env()?.build()
}

// Hand-written modules; no codegen target writes to either.
pub mod download;
pub mod subscribe;
pub use download::DownloadError;
pub use subscribe::{
    ErrorCode, GenerationError, GenerationResult, JobHandle, Media, StatusUpdate, SubscribeOptions,
    submit, subscribe,
};

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
    /// Save what is at `url` to `path`, returning the number of bytes written.
    ///
    /// This is how a generation's output reaches the disk: pass the finished
    /// job's `asset.signed_url`, or a [`Media::url`] from
    /// [`subscribe`]. It exists so a first program needs no HTTP crate of its
    /// own — the whole install stays `cargo add nolgia-client` (NOL-1087).
    ///
    /// The body streams through a sibling `<path>.part` and is renamed into
    /// place only after the last byte arrives, so an interrupted download
    /// never leaves a truncated file behind. Any existing file at `path` is
    /// replaced. The parent directory must already exist.
    ///
    /// **Credentials.** The client's `Authorization`, `X-Nolgia-Surface` and
    /// `Idempotency-Key` headers are sent only when `url` is on the same
    /// origin as the client's base URL. An `asset.signed_url` points at a
    /// storage host and carries its own credential in the query string, so
    /// the request is made anonymously rather than handing a third party a
    /// token that can spend money.
    ///
    /// ```no_run
    /// # use nolgia_client::{ClientExt, tokio};
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let nolgia = nolgia_client::client()?;
    /// let bytes = nolgia.download("https://…/first.png", "first.png").await?;
    /// println!("{bytes} bytes -> first.png");
    /// # Ok(()) }
    /// ```
    fn download(
        &self,
        url: &str,
        path: impl AsRef<std::path::Path> + Send,
    ) -> impl std::future::Future<Output = StdResult<u64, DownloadError>> + Send;

    /// [`download`](Self::download) into memory instead of onto the disk.
    ///
    /// For piping an asset somewhere else — an upload, an image library, a
    /// response body. Prefer [`download`](Self::download) for video: this
    /// holds the whole file in memory.
    fn download_bytes(
        &self,
        url: &str,
    ) -> impl std::future::Future<Output = StdResult<Vec<u8>, DownloadError>> + Send;

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

    /// POST `/agent/abilities/{slug}` with an explicit `{}` JSON body to
    /// install a marketplace ability, returning the new
    /// [`types::AgentInstalledAbility`].
    ///
    /// This mirrors the generated `install_agent_ability` builder, which —
    /// because the spec declares no request body for the operation — sends a
    /// bodyless POST with no `Content-Length` header. The production load
    /// balancer rejects that with `411 Length Required` before the request
    /// ever reaches the API (NOL-542), exactly like the bodyless
    /// `complete_asset_upload` above. Sending `{}` gives the request a sized
    /// body, so `Content-Length` is emitted; the API accepts the empty
    /// object. Non-2xx responses surface as
    /// [`ApiError::UnexpectedResponse`], same as the generated method, so
    /// the CLI's RFC 7807 problem rendering still applies.
    fn install_agent_ability_with_body(
        &self,
        slug: &str,
    ) -> impl std::future::Future<Output = StdResult<types::AgentInstalledAbility, ApiError<()>>> + Send;

    /// POST `/jobs/{id}/cancel` to cancel a job on the server, returning the
    /// canceled [`types::Job`] (its `cancellation` says what the model
    /// provider did and what happened to the credits).
    ///
    /// This mirrors the generated `cancel_job` builder, which, because the
    /// spec declares no request body for the operation, sends a bodyless POST
    /// with no `Content-Length` header: the production load balancer rejects
    /// that with `411 Length Required` before the request reaches the API
    /// (NOL-542), exactly like `complete_asset_upload` above. This sends an
    /// explicit empty body with `Content-Length: 0`. Non-2xx responses
    /// surface as [`ApiError::UnexpectedResponse`], same as the generated
    /// method, so a `409` problem's `code: job_not_cancellable` stays
    /// readable.
    fn cancel_job_with_body(
        &self,
        id: Uuid,
    ) -> impl std::future::Future<Output = StdResult<types::Job, ApiError<()>>> + Send;
}

impl ClientExt for Client {
    async fn download(
        &self,
        url: &str,
        path: impl AsRef<std::path::Path> + Send,
    ) -> StdResult<u64, DownloadError> {
        download::to_path(self, url, path.as_ref()).await
    }

    async fn download_bytes(&self, url: &str) -> StdResult<Vec<u8>, DownloadError> {
        download::bytes(self, url).await
    }

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

    async fn install_agent_ability_with_body(
        &self,
        slug: &str,
    ) -> StdResult<types::AgentInstalledAbility, ApiError<()>> {
        let url = format!(
            "{}/agent/abilities/{}",
            self.baseurl(),
            progenitor_client::encode_path(slug)
        );
        let response = self
            .client()
            .post(url)
            // An empty JSON object, not a bodyless POST: `{}` is a sized
            // body, so the request carries `Content-Length: 2` and the
            // production LB's 411 never fires.
            .json(&serde_json::json!({}))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(ApiError::UnexpectedResponse(response));
        }
        Ok(response.json::<types::AgentInstalledAbility>().await?)
    }

    async fn cancel_job_with_body(&self, id: Uuid) -> StdResult<types::Job, ApiError<()>> {
        let response = cancel_job_request(self, &id.to_string()).send().await?;
        if !response.status().is_success() {
            return Err(ApiError::UnexpectedResponse(response));
        }
        Ok(response.json::<types::Job>().await?)
    }
}

/// The one `POST /jobs/{id}/cancel` request this crate sends, shared by
/// [`ClientExt::cancel_job_with_body`] and [`JobHandle::cancel_job`] so the
/// `411` fix below cannot drift between them.
pub(crate) fn cancel_job_request(client: &Client, id: &str) -> reqwest::RequestBuilder {
    client
        .client()
        .post(format!(
            "{}/jobs/{}/cancel",
            client.baseurl(),
            progenitor_client::encode_path(id)
        ))
        .header(reqwest::header::ACCEPT, "application/json")
        // reqwest omits `Content-Length` for a bodyless (or empty-body)
        // POST, which the production LB rejects with 411 (NOL-542). Set it
        // explicitly so the request carries `Content-Length: 0`.
        .header(reqwest::header::CONTENT_LENGTH, "0")
        .body(Vec::<u8>::new())
}

#[derive(Debug, Clone)]
pub struct ClientBuilder {
    base_url: String,
    auth_token: Option<String>,
    surface: Option<String>,
    idempotency_key: Option<String>,
}

pub enum ClientBuilderError {
    MissingToken,
    InvalidAuthorization(reqwest::header::InvalidHeaderValue),
    InvalidIdempotencyKey(String),
    Transport(reqwest::Error),
}

impl fmt::Display for ClientBuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingToken => write!(
                f,
                "NOLGIA_TOKEN is not set — create a personal access token at \
                 https://nolgia.com/settings/api-tokens, or build the client with \
                 ClientBuilder::new(url).pat(token)"
            ),
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

// Debug mirrors Display on purpose. `fn main() -> Result<_, Box<dyn Error>>`
// — the shape of our own quickstart — prints the error with `{:?}`, so a
// derived Debug would greet a new user with `MissingToken` and swallow the
// sentence telling them how to fix it.
impl fmt::Debug for ClientBuilderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for ClientBuilderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidAuthorization(err) => Some(err),
            Self::MissingToken | Self::InvalidIdempotencyKey(_) => None,
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

    /// A builder configured from the environment: `NOLGIA_TOKEN` (a PAT
    /// `nol_...` or a JWT), `NOLGIA_API_URL` when set, and `NOLGIA_SURFACE`
    /// when set. No surface is invented when the variable is absent, because
    /// server-side attribution keys on that header.
    pub fn from_env() -> StdResult<Self, ClientBuilderError> {
        let token = std::env::var("NOLGIA_TOKEN")
            .ok()
            .filter(|token| !token.trim().is_empty())
            .ok_or(ClientBuilderError::MissingToken)?;
        let base_url = std::env::var("NOLGIA_API_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let builder = Self::new(base_url).pat(token);
        Ok(match std::env::var("NOLGIA_SURFACE") {
            Ok(surface) if !surface.trim().is_empty() => builder.surface(surface),
            _ => builder,
        })
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

#[cfg(test)]
mod one_command_tests {
    //! NOL-1066: `cargo add nolgia-client` must be the whole install. These
    //! pin the surface the first example uses, so a refactor cannot quietly
    //! send users back to `cargo add tokio serde_json`.

    /// Environment variables are process-global and Cargo runs tests in
    /// threads, so every env-mutating assertion lives in this ONE test.
    #[test]
    fn client_from_env_reads_the_token_and_names_what_is_missing() {
        use super::ClientInfo as _;
        let restore = std::env::var("NOLGIA_TOKEN").ok();
        // SAFETY (edition 2024): no other test in this crate touches the
        // environment, and this test restores what it found.
        unsafe { std::env::remove_var("NOLGIA_TOKEN") };
        let err = super::client().expect_err("no token must be an error, not a panic");
        let message = err.to_string();
        assert!(message.contains("NOLGIA_TOKEN is not set"), "{message}");
        assert!(message.contains("settings/api-tokens"), "{message}");

        unsafe { std::env::set_var("NOLGIA_TOKEN", "nol_test_token") };
        let client = super::client().expect("a token is all the client needs");
        assert!(client.baseurl().ends_with("/v1"), "{}", client.baseurl());

        unsafe {
            match restore {
                Some(value) => std::env::set_var("NOLGIA_TOKEN", value),
                None => std::env::remove_var("NOLGIA_TOKEN"),
            }
        }
    }

    #[test]
    fn the_runtime_and_json_reach_callers_without_a_second_crate() {
        // Exactly what the quickstart uses, through this crate alone.
        let body = crate::json!({"model": "flux-pro", "prompt": "a paper-cut mountain range"});
        assert_eq!(body["model"], "flux-pro");
        let answer: u8 = crate::rt::block_on(async { 7 });
        assert_eq!(answer, 7);
        let _: crate::Value = body;
    }
}
