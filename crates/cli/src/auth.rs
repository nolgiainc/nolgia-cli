use std::{
    fs,
    future::Future,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Subcommand;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::Instant;

use crate::output::{OutputFormat, print_json};

pub const SERVICE_NAME: &str = "com.nolgiainc.nolgia";
/// Pre-rename keyring service name. This is the ONLY remaining reference to
/// the old org identifier, and it exists solely so `KeyringTokenStore::load`
/// can perform a one-time migration of any tokens still stored under the old
/// service into `SERVICE_NAME` — otherwise the rename would silently log out
/// users who opted into the keyring store. Safe to delete once users have
/// upgraded past this release.
const LEGACY_SERVICE_NAME: &str = "com.nolgiacorp.nolgia";
pub const ACCESS_TOKEN_ACCOUNT: &str = "access_token";
pub const REFRESH_TOKEN_ACCOUNT: &str = "refresh_token";
const TOKENS_FILE: &str = "tokens.json";
const KEYRING_MIGRATION_MARKER: &str = ".keyring-migration-done";
const CLIENT_ID: &str = "nolgia-cli";
const DEFAULT_SCOPE: &str = "generate:* assets:read";
const EXPIRY_SKEW_SECONDS: i64 = 30;

type SleepFn = Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
type CancelFn = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Log in through your browser (device code flow)
    Login {
        /// Print the link and code without opening a browser
        #[arg(long)]
        no_browser: bool,
    },
    Logout,
    Status,
    Whoami,
    /// Print the current bearer token (for scripts and agents)
    Token,
}

#[derive(Clone)]
pub struct AuthManager<S> {
    base_url: String,
    http: Client,
    store: S,
    sleep: SleepFn,
    cancel: CancelFn,
}

impl<S: TokenStore> AuthManager<S> {
    pub fn new(base_url: impl Into<String>, store: S) -> Self {
        Self {
            base_url: normalize_base_url(&base_url.into()),
            http: Client::new(),
            store,
            sleep: Arc::new(|duration| Box::pin(tokio::time::sleep(duration))),
            cancel: Arc::new(|| {
                Box::pin(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
            }),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn with_hooks(mut self, sleep: SleepFn, cancel: CancelFn) -> Self {
        self.sleep = sleep;
        self.cancel = cancel;
        self
    }

    /// Runs the device-code login end to end, narrating it on `screen`: the
    /// link and code, a waiting line while the user approves, and one
    /// "Connected" line at the end.
    pub async fn login<W: Write>(
        &self,
        options: LoginOptions,
        screen: &mut LoginScreen<W>,
    ) -> std::result::Result<LoginOutcome, AuthError> {
        let device = self.start_device_auth().await?;
        let prompt = LoginPrompt::from(&device);
        let copied = options.copy_code && copy_to_clipboard(&prompt.user_code);
        screen.prompt(&prompt, copied);
        if options.open_browser {
            open_in_browser(prompt.link());
        }

        let token = match self.poll_device_token(&device, screen).await {
            Ok(token) => token,
            Err(err) => {
                screen.end_waiting();
                return Err(err);
            }
        };
        let tokens = StoredTokens::from_token_response(token);
        self.store.save(&tokens)?;

        // The account name is a courtesy, and the tokens are already saved:
        // a failed lookup must not turn a login that worked into an error.
        let email = self
            .fetch_user(&tokens.access_token)
            .await
            .ok()
            .map(|user| user.email);
        screen.connected(email.as_deref());

        Ok(LoginOutcome { prompt, tokens })
    }

    pub async fn status_with_token(
        &self,
        access_token: &str,
    ) -> std::result::Result<AuthStatus, AuthError> {
        let user = self.fetch_user(access_token).await?;
        let tier = self
            .fetch_subscription_tier(access_token)
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        Ok(AuthStatus {
            email: user.email,
            tier,
            organization: user.active_organization,
        })
    }

    pub async fn status(&self) -> std::result::Result<AuthStatus, AuthError> {
        let mut tokens = self.valid_tokens().await?;

        let user = match self.fetch_user(&tokens.access_token).await {
            Ok(user) => user,
            Err(AuthError::Unauthorized) => {
                tokens = self.refresh_tokens(&tokens).await?;
                self.fetch_user(&tokens.access_token).await?
            }
            Err(err) => return Err(err),
        };

        let tier = match self.fetch_subscription_tier(&tokens.access_token).await {
            Ok(tier) => tier,
            Err(AuthError::Unauthorized) => {
                let refreshed = self.refresh_tokens(&tokens).await?;
                self.fetch_subscription_tier(&refreshed.access_token)
                    .await?
            }
            Err(_) => "unknown".to_string(),
        };

        Ok(AuthStatus {
            email: user.email,
            tier,
            organization: user.active_organization,
        })
    }

    pub fn logout(&self) -> std::result::Result<(), AuthError> {
        self.store.delete()
    }

    pub async fn valid_tokens(&self) -> std::result::Result<StoredTokens, AuthError> {
        let tokens = self.store.load()?.ok_or(AuthError::NotLoggedIn)?;
        if tokens.is_expired() {
            self.refresh_tokens(&tokens).await
        } else {
            Ok(tokens)
        }
    }

    pub async fn refresh_tokens(
        &self,
        tokens: &StoredTokens,
    ) -> std::result::Result<StoredTokens, AuthError> {
        let refresh_token = tokens
            .refresh_token
            .as_deref()
            .ok_or(AuthError::MissingRefreshToken)?;
        let response = self
            .http
            .post(format!("{}/auth/device/token", self.base_url))
            .json(&DeviceTokenRequest {
                client_id: CLIENT_ID,
                device_code: refresh_token,
            })
            .send()
            .await?
            .error_for_status()?;
        let token = response.json::<DeviceTokenResponse>().await?;
        let refreshed =
            StoredTokens::from_token_response_with_refresh(token, Some(refresh_token.to_string()));
        self.store.save(&refreshed)?;
        Ok(refreshed)
    }

    async fn start_device_auth(&self) -> std::result::Result<DeviceAuthResponse, AuthError> {
        let response = self
            .http
            .post(format!("{}/auth/device", self.base_url))
            .json(&DeviceAuthRequest {
                client_id: CLIENT_ID,
                scope: Some(DEFAULT_SCOPE),
            })
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    async fn poll_device_token<W: Write>(
        &self,
        device: &DeviceAuthResponse,
        screen: &mut LoginScreen<W>,
    ) -> std::result::Result<DeviceTokenResponse, AuthError> {
        let deadline = Instant::now() + Duration::from_secs(device.expires_in);
        let mut interval = Duration::from_secs(device.interval.max(1));
        screen.waiting(deadline.saturating_duration_since(Instant::now()));

        loop {
            // Sleep out the poll interval a second at a time so the countdown
            // on the waiting line stays honest, and never past the deadline.
            let mut slept = Duration::ZERO;
            while slept < interval {
                let now = Instant::now();
                if now >= deadline {
                    return Err(AuthError::Expired);
                }
                let step = Duration::from_secs(1)
                    .min(interval - slept)
                    .min(deadline - now);
                tokio::select! {
                    () = (self.sleep)(step) => {},
                    () = (self.cancel)() => return Err(AuthError::Canceled),
                }
                slept += step;
                screen.waiting(deadline.saturating_duration_since(Instant::now()));
            }

            let response = self
                .http
                .post(format!("{}/auth/device/token", self.base_url))
                .json(&DeviceTokenRequest {
                    client_id: CLIENT_ID,
                    device_code: device.device_code.as_str(),
                })
                .send()
                .await?;

            match response.status() {
                StatusCode::OK => return Ok(response.json().await?),
                StatusCode::FORBIDDEN => continue,
                StatusCode::BAD_REQUEST => match response
                    .json::<Problem>()
                    .await
                    .ok()
                    .and_then(|p| p.error.or(p.title).or(p.kind))
                {
                    Some(error) if error == "authorization_pending" => continue,
                    Some(error) if error == "slow_down" => {
                        interval += Duration::from_secs(5);
                        continue;
                    }
                    Some(error) if error == "expired_token" => return Err(AuthError::Expired),
                    _ => return Err(AuthError::Api("device authorization failed".to_string())),
                },
                status => return Err(AuthError::Status(status)),
            }
        }
    }

    async fn fetch_user(&self, access_token: &str) -> std::result::Result<User, AuthError> {
        let response = self
            .http
            .get(format!("{}/me", self.base_url))
            .bearer_auth(access_token)
            .send()
            .await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(AuthError::Unauthorized);
        }
        Ok(response.error_for_status()?.json().await?)
    }

    async fn fetch_subscription_tier(
        &self,
        access_token: &str,
    ) -> std::result::Result<String, AuthError> {
        let response = self
            .http
            .get(format!("{}/billing/subscription", self.base_url))
            .bearer_auth(access_token)
            .send()
            .await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(AuthError::Unauthorized);
        }
        Ok(response
            .error_for_status()?
            .json::<Subscription>()
            .await?
            .tier)
    }
}

pub trait TokenStore: Send + Sync {
    fn load(&self) -> std::result::Result<Option<StoredTokens>, AuthError>;
    fn save(&self, tokens: &StoredTokens) -> std::result::Result<(), AuthError>;
    fn delete(&self) -> std::result::Result<(), AuthError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringTokenStore;

impl TokenStore for KeyringTokenStore {
    fn load(&self) -> std::result::Result<Option<StoredTokens>, AuthError> {
        match load_current_keyring()? {
            Some(tokens) => Ok(Some(tokens)),
            // Nothing under the current service name: the tokens may still be
            // under the pre-rename service. Try to migrate them once so the
            // rename doesn't log the user out.
            None => migrate_legacy_keyring(),
        }
    }

    fn save(&self, tokens: &StoredTokens) -> std::result::Result<(), AuthError> {
        entry(ACCESS_TOKEN_ACCOUNT)?
            .set_password(&access_entry_payload(tokens)?)
            .map_err(|err| AuthError::Keyring(err.to_string()))?;
        if let Some(refresh_token) = &tokens.refresh_token {
            entry(REFRESH_TOKEN_ACCOUNT)?
                .set_password(refresh_token)
                .map_err(|err| AuthError::Keyring(err.to_string()))?;
        }
        Ok(())
    }

    fn delete(&self) -> std::result::Result<(), AuthError> {
        delete_entry(ACCESS_TOKEN_ACCOUNT)?;
        delete_entry(REFRESH_TOKEN_ACCOUNT)?;
        // Also drop any not-yet-migrated pre-rename entries: otherwise logout
        // reports success and the next load migrates them back, silently
        // logging the user in again.
        delete_legacy_entry(ACCESS_TOKEN_ACCOUNT)?;
        delete_legacy_entry(REFRESH_TOKEN_ACCOUNT)?;
        Ok(())
    }
}

/// File-backed token store: `$XDG_CONFIG_HOME/nolgia/tokens.json` (default
/// `~/.config/nolgia/tokens.json`), written `0600` in a `0700` directory.
///
/// This is the DEFAULT store. The OS keyring is opt-in
/// (`NOLGIA_TOKEN_STORE=keyring`) because on macOS keychain items are
/// ACL'd to the exact binary that created them — every upgrade or rebuild
/// of `nolgia` is a new (ad-hoc) signing identity, so each new binary
/// re-triggered a "nolgia wants to use your login keychain" password
/// prompt on every command. A `0600` file matches how `gh` and `gcloud`
/// store credentials and never prompts.
#[derive(Debug, Clone)]
pub struct FileTokenStore {
    path: PathBuf,
}

impl FileTokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `${XDG_CONFIG_HOME:-$HOME/.config}/nolgia/tokens.json`.
    pub fn from_env() -> Option<Self> {
        Some(Self::new(config_dir()?.join(TOKENS_FILE)))
    }

    fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    fn write_secret(&self, contents: &str) -> std::io::Result<()> {
        fs::create_dir_all(self.dir())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let _ = fs::set_permissions(self.dir(), fs::Permissions::from_mode(0o700));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)?;
            file.write_all(contents.as_bytes())?;
            // In case the file pre-existed with looser permissions.
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            fs::write(&self.path, contents)
        }
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> std::result::Result<Option<StoredTokens>, AuthError> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(AuthError::Store(err.to_string())),
        };
        Ok(Some(serde_json::from_str::<StoredTokens>(&raw)?))
    }

    fn save(&self, tokens: &StoredTokens) -> std::result::Result<(), AuthError> {
        self.write_secret(&serde_json::to_string_pretty(tokens)?)
            .map_err(|err| AuthError::Store(err.to_string()))
    }

    fn delete(&self) -> std::result::Result<(), AuthError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AuthError::Store(err.to_string())),
        }
    }
}

/// The store the CLI actually uses, selected by `NOLGIA_TOKEN_STORE`:
///
/// - unset (default): the token file, plus a ONE-TIME migration read of the
///   OS keyring for users who logged in before the file store existed
/// - `file`: the token file only — the keyring is never touched
/// - `keyring`: the OS keyring (pre-file behavior)
pub enum CliTokenStore {
    File {
        store: FileTokenStore,
        migrate_from_keyring: bool,
    },
    Keyring(KeyringTokenStore),
}

pub fn default_store() -> CliTokenStore {
    let file = || {
        FileTokenStore::from_env().unwrap_or_else(|| {
            // No resolvable home directory; keep a deterministic (if odd)
            // fallback rather than failing every command.
            FileTokenStore::new(PathBuf::from(".nolgia-tokens.json"))
        })
    };
    match std::env::var("NOLGIA_TOKEN_STORE").as_deref() {
        Ok("keyring") => CliTokenStore::Keyring(KeyringTokenStore),
        Ok("file") => CliTokenStore::File {
            store: file(),
            migrate_from_keyring: false,
        },
        _ => CliTokenStore::File {
            store: file(),
            migrate_from_keyring: true,
        },
    }
}

impl TokenStore for CliTokenStore {
    fn load(&self) -> std::result::Result<Option<StoredTokens>, AuthError> {
        match self {
            Self::File {
                store,
                migrate_from_keyring,
            } => {
                if let Some(tokens) = store.load()? {
                    return Ok(Some(tokens));
                }
                if *migrate_from_keyring {
                    return Ok(migrate_keyring_once(store, &KeyringTokenStore));
                }
                Ok(None)
            }
            Self::Keyring(store) => store.load(),
        }
    }

    fn save(&self, tokens: &StoredTokens) -> std::result::Result<(), AuthError> {
        match self {
            Self::File { store, .. } => store.save(tokens),
            Self::Keyring(store) => store.save(tokens),
        }
    }

    fn delete(&self) -> std::result::Result<(), AuthError> {
        match self {
            Self::File { store, .. } => store.delete(),
            Self::Keyring(store) => store.delete(),
        }
    }
}

/// One-time migration from the OS keyring to the token file. The keyring is
/// probed AT MOST ONCE per config dir (a marker file records the attempt,
/// success or not) so a denied/canceled keychain prompt can never recur on
/// every command — that repeated prompt is the exact bug this fixes. The
/// keyring item itself is left untouched.
fn migrate_keyring_once(file: &FileTokenStore, source: &dyn TokenStore) -> Option<StoredTokens> {
    let marker = file.dir().join(KEYRING_MIGRATION_MARKER);
    if marker.exists() {
        return None;
    }
    let tokens = source.load().ok().flatten();
    if let Some(tokens) = &tokens {
        let _ = file.save(tokens);
    }
    let _ = fs::create_dir_all(file.dir());
    let _ = fs::write(&marker, b"keyring migration attempted; delete to retry\n");
    tokens
}

/// `${XDG_CONFIG_HOME:-$HOME/.config}/nolgia` (same convention as the
/// update checker and installer metadata).
fn config_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home_dir()?.join(".config"),
    };
    Some(base.join("nolgia"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
}

impl StoredTokens {
    fn from_token_response(response: DeviceTokenResponse) -> Self {
        let refresh_token = response
            .refresh_token
            .clone()
            .or_else(|| Some(response.access_token.clone()));
        Self::from_token_response_with_refresh(response, refresh_token)
    }

    fn from_token_response_with_refresh(
        response: DeviceTokenResponse,
        refresh_token: Option<String>,
    ) -> Self {
        Self {
            access_token: response.access_token,
            refresh_token,
            expires_at: Utc::now() + chrono::Duration::seconds(response.expires_in as i64),
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at <= Utc::now() + chrono::Duration::seconds(EXPIRY_SKEW_SECONDS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoginPrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
}

impl LoginPrompt {
    /// The link to open: the one with the code filled in when the server
    /// provides it, otherwise the bare approval page.
    pub fn link(&self) -> &str {
        self.verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri)
    }
}

impl From<&DeviceAuthResponse> for LoginPrompt {
    fn from(response: &DeviceAuthResponse) -> Self {
        Self {
            user_code: response.user_code.clone(),
            verification_uri: response.verification_uri.clone(),
            verification_uri_complete: response.verification_uri_complete.clone(),
            expires_in: response.expires_in,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoginOutcome {
    pub prompt: LoginPrompt,
    pub tokens: StoredTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthStatus {
    pub email: String,
    pub tier: String,
    /// The organization the token is working in, or `None` in the personal
    /// space. In an organization context `tier` is the organization's plan.
    pub organization: Option<AuthOrganization>,
}

/// The active organization as `GET /me` reports it (`active_organization`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AuthOrganization {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub role: String,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("no token is stored; run `nolgia auth login`")]
    NotLoggedIn,
    #[error("login canceled")]
    Canceled,
    #[error("device code expired")]
    Expired,
    #[error("refresh token missing")]
    MissingRefreshToken,
    #[error("request was unauthorized")]
    Unauthorized,
    #[error("API returned HTTP {0}")]
    Status(StatusCode),
    #[error("API request failed: {0}")]
    Api(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("token serialization failed: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("keyring error: {0}")]
    Keyring(String),
    #[error("token store error: {0}")]
    Store(String),
}

#[derive(Deserialize, Serialize)]
struct DeviceAuthRequest<'a> {
    client_id: &'a str,
    scope: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: u64,
}

#[derive(Serialize)]
struct DeviceTokenRequest<'a> {
    client_id: &'a str,
    device_code: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize)]
struct User {
    email: String,
    #[serde(default)]
    active_organization: Option<AuthOrganization>,
}

#[derive(Deserialize)]
struct Subscription {
    tier: String,
}

#[derive(Deserialize)]
struct Problem {
    #[serde(rename = "type")]
    kind: Option<String>,
    error: Option<String>,
    // The API answers the token poll with RFC 7807 problem+json and carries
    // the OAuth error code in `title`.
    title: Option<String>,
}

pub async fn run(
    command: AuthCommand,
    format: OutputFormat,
    base_url: &str,
    token: Option<String>,
) -> Result<()> {
    let manager = AuthManager::new(base_url, default_store());
    match command {
        AuthCommand::Token => {
            let resolved = token.or_else(load_token).ok_or_else(|| {
                anyhow::anyhow!("not logged in — run `nolgia auth login` or set NOLGIA_TOKEN")
            })?;
            println!("{resolved}");
            Ok(())
        }
        AuthCommand::Login { no_browser } => {
            let mut screen = LoginScreen::for_cli(format);
            let options = LoginOptions {
                open_browser: !no_browser,
                // The clipboard is for a person at a terminal; a script or an
                // agent reading the output has nowhere to paste.
                copy_code: screen.is_tty(),
            };
            let outcome = manager
                .login(options, &mut screen)
                .await
                .map_err(|err| match err {
                    AuthError::Expired => anyhow::anyhow!(LOGIN_EXPIRED_MESSAGE),
                    other => other.into(),
                })?;
            emit_login(format, &outcome)
        }
        AuthCommand::Logout => {
            manager.logout()?;
            emit_message(format, "logged out")
        }
        AuthCommand::Status | AuthCommand::Whoami => {
            match token.filter(|token| !token.is_empty()) {
                Some(token) => emit_status(format, &manager.status_with_token(&token).await?),
                None => emit_status(format, &manager.status().await?),
            }
        }
    }
}

pub fn load_token() -> Option<String> {
    default_store()
        .load()
        .ok()
        .flatten()
        .map(|tokens| tokens.access_token)
}

fn emit_login(format: OutputFormat, outcome: &LoginOutcome) -> Result<()> {
    match format {
        OutputFormat::Json => print_json(outcome),
        OutputFormat::Text => Ok(()),
    }
}

fn emit_status(format: OutputFormat, status: &AuthStatus) -> Result<()> {
    match format {
        OutputFormat::Json => print_json(status),
        OutputFormat::Text => {
            println!("{} ({})", status.email, status.tier);
            println!("{}", organization_line(status.organization.as_ref()));
            Ok(())
        }
    }
}

/// The "Organization:" line of `auth status`: where this token's requests
/// land, and therefore which credit pool a generation spends.
fn organization_line(organization: Option<&AuthOrganization>) -> String {
    match organization {
        Some(org) => format!("Organization: {} ({}) as {}", org.name, org.slug, org.role),
        None => "Organization: Personal space".to_string(),
    }
}

#[derive(Serialize)]
struct Message<'a> {
    message: &'a str,
}

fn emit_message(format: OutputFormat, message: &'static str) -> Result<()> {
    match format {
        OutputFormat::Json => print_json(&Message { message }),
        OutputFormat::Text => {
            println!("{message}");
            Ok(())
        }
    }
}

/// What `auth login` says when the code runs out before anyone approves it.
pub const LOGIN_EXPIRED_MESSAGE: &str =
    "the login code expired before it was approved; run `nolgia auth login` to get a new one";

const WAITING_PREFIX: &str = "Waiting for you to approve in the browser... (expires in ";

/// How `login` behaves around the terminal, as opposed to what it says.
#[derive(Clone, Copy, Debug)]
pub struct LoginOptions {
    /// Open the approval link in the default browser (best effort, silent).
    pub open_browser: bool,
    /// Put the code on the clipboard when the platform makes that trivial.
    pub copy_code: bool,
}

/// The human narration of a login. Everything it prints is a courtesy: the
/// outcome is carried by the returned tokens, and a write error is ignored.
///
/// On a TTY the waiting line is redrawn in place as the countdown moves; when
/// the output is a pipe or a file, every line is printed exactly once.
pub struct LoginScreen<W: Write> {
    out: W,
    tty: bool,
    /// Width of the line currently occupying the cursor's row on a TTY, so a
    /// shorter redraw can blank what the previous one left behind.
    live_width: usize,
    waiting_shown: bool,
}

impl LoginScreen<Box<dyn Write + Send>> {
    /// Text mode narrates on stdout. `--json` keeps stdout for the JSON
    /// document and moves the narration to stderr.
    pub fn for_cli(format: OutputFormat) -> Self {
        match format {
            OutputFormat::Text => Self::new(Box::new(io::stdout()), io::stdout().is_terminal()),
            OutputFormat::Json => Self::new(Box::new(io::stderr()), io::stderr().is_terminal()),
        }
    }
}

impl<W: Write> LoginScreen<W> {
    pub fn new(out: W, tty: bool) -> Self {
        Self {
            out,
            tty,
            live_width: 0,
            waiting_shown: false,
        }
    }

    pub fn is_tty(&self) -> bool {
        self.tty
    }

    #[cfg(test)]
    fn into_inner(self) -> W {
        self.out
    }

    /// The link first (it is what people click), then the code on its own
    /// line for the type-it-in case.
    fn prompt(&mut self, prompt: &LoginPrompt, copied: bool) {
        let copied = if copied {
            "  (copied to clipboard)"
        } else {
            ""
        };
        let _ = write!(
            self.out,
            "Open: {}\n\n  Code: {}{copied}\n\n",
            prompt.link(),
            prompt.user_code
        );
        let _ = self.out.flush();
    }

    /// One status line while polling. Redrawn in place on a TTY; printed once
    /// otherwise, since a log has no cursor to move.
    fn waiting(&mut self, remaining: Duration) {
        let line = format!("{WAITING_PREFIX}{})", format_remaining(remaining));
        if self.tty {
            self.redraw(&line);
        } else if !self.waiting_shown {
            let _ = writeln!(self.out, "{line}");
            let _ = self.out.flush();
        }
        self.waiting_shown = true;
    }

    /// The one line a successful login ends on.
    fn connected(&mut self, email: Option<&str>) {
        let line = match email {
            Some(email) => format!("\u{2705} Connected as {email}"),
            None => "\u{2705} Connected".to_string(),
        };
        if self.tty {
            self.redraw(&line);
            self.live_width = 0;
            let _ = writeln!(self.out);
        } else {
            let _ = writeln!(self.out, "{line}");
        }
        let _ = self.out.flush();
    }

    /// Clears the in-place waiting line so whatever is said next (an error,
    /// usually) starts on a clean row. Nothing to do when lines are not
    /// being redrawn.
    fn end_waiting(&mut self) {
        if self.tty && self.live_width > 0 {
            self.redraw("");
            self.live_width = 0;
        }
        let _ = self.out.flush();
    }

    /// Overwrites the current row. A carriage return plus blank padding is
    /// used rather than an escape sequence so it renders the same on every
    /// terminal, including consoles without VT processing.
    fn redraw(&mut self, line: &str) {
        let width = line.chars().count();
        let pad = self.live_width.saturating_sub(width);
        let _ = write!(self.out, "\r{line}{:pad$}", "");
        if pad > 0 {
            let _ = write!(self.out, "\r{line}");
        }
        let _ = self.out.flush();
        self.live_width = width;
    }
}

/// `M:SS` of what is left, rounded up so a code good for 15 minutes reads
/// `15:00` rather than `14:59` on the first draw.
fn format_remaining(remaining: Duration) -> String {
    let seconds = remaining.as_millis().div_ceil(1000);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Hands the link to the platform's default opener without waiting on it.
/// Silent by design: the link is on screen regardless, so a machine with no
/// opener or no display (a container, an SSH session) just falls back to it.
fn open_in_browser(url: &str) -> bool {
    let argv: &[&str] = if cfg!(target_os = "macos") {
        &["open"]
    } else if cfg!(windows) {
        &["cmd", "/C", "start", ""]
    } else if has_display() {
        &["xdg-open"]
    } else {
        // Without a display xdg-open may fall back to a text browser that
        // takes over the terminal, which is worse than doing nothing.
        return false;
    };
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let child = std::process::Command::new(program)
        .args(args)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            // Some openers stay alive as long as the browser does; reap it
            // off the main thread so the login never waits on it.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

/// Copies the code with the clipboard tool the platform ships (`pbcopy`,
/// `clip`, or `wl-copy`/`xclip`/`xsel` when a display is up). A clipboard
/// crate would pull in a windowing stack for a one-line nicety, so this stays
/// a bounded, best-effort shell-out: anything slower than a moment counts as
/// not copied, and the login goes on either way.
fn copy_to_clipboard(text: &str) -> bool {
    let candidates: &'static [&'static [&'static str]] = if cfg!(target_os = "macos") {
        &[&["pbcopy"]]
    } else if cfg!(windows) {
        &[&["clip"]]
    } else if has_display() {
        &[
            &["wl-copy"],
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
        ]
    } else {
        &[]
    };
    if candidates.is_empty() {
        return false;
    }
    let text = text.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let copied = candidates.iter().any(|argv| pipe_into(argv, &text));
        let _ = tx.send(copied);
    });
    rx.recv_timeout(Duration::from_millis(750)).unwrap_or(false)
}

fn pipe_into(argv: &[&str], text: &str) -> bool {
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let Ok(mut child) = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    written && matches!(child.wait(), Ok(status) if status.success())
}

fn has_display() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    set("WAYLAND_DISPLAY") || set("DISPLAY")
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

fn entry(account: &str) -> std::result::Result<keyring::Entry, AuthError> {
    keyring::Entry::new(SERVICE_NAME, account).map_err(|err| AuthError::Keyring(err.to_string()))
}

fn delete_entry(account: &str) -> std::result::Result<(), AuthError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(AuthError::Keyring(err.to_string())),
    }
}

fn legacy_entry(account: &str) -> std::result::Result<keyring::Entry, AuthError> {
    keyring::Entry::new(LEGACY_SERVICE_NAME, account)
        .map_err(|err| AuthError::Keyring(err.to_string()))
}

fn delete_legacy_entry(account: &str) -> std::result::Result<(), AuthError> {
    match legacy_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(AuthError::Keyring(err.to_string())),
    }
}

/// Serializes the access entry's payload: every field except the refresh token,
/// which lives in its own entry.
fn access_entry_payload(tokens: &StoredTokens) -> std::result::Result<String, AuthError> {
    let mut access_only = tokens.clone();
    access_only.refresh_token = None;
    Ok(serde_json::to_string(&access_only)?)
}

/// Reads the tokens stored under the current `SERVICE_NAME`, without touching
/// the legacy service. `Ok(None)` means there is no access-token entry.
fn load_current_keyring() -> std::result::Result<Option<StoredTokens>, AuthError> {
    let access_json = match entry(ACCESS_TOKEN_ACCOUNT)?.get_password() {
        Ok(value) => value,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(err) => return Err(AuthError::Keyring(err.to_string())),
    };

    let mut tokens = serde_json::from_str::<StoredTokens>(&access_json)?;
    tokens.refresh_token = match entry(REFRESH_TOKEN_ACCOUNT)?.get_password() {
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => return Err(AuthError::Keyring(err.to_string())),
    };
    Ok(Some(tokens))
}

/// One-time migration off the pre-rename keyring service name. Called only when
/// nothing is stored under the current `SERVICE_NAME`. If tokens exist under
/// `LEGACY_SERVICE_NAME`, they are re-homed under the current service and the
/// legacy entries are removed, so a keyring user is not logged out by the
/// rename. Returns `Ok(None)` when there is nothing under either service.
fn migrate_legacy_keyring() -> std::result::Result<Option<StoredTokens>, AuthError> {
    let access_json = match legacy_entry(ACCESS_TOKEN_ACCOUNT)?.get_password() {
        Ok(value) => value,
        // No legacy entry. A concurrent process may have migrated and removed
        // it between our two lookups, so recheck the current service before
        // concluding the user has no token at all.
        Err(keyring::Error::NoEntry) => return load_current_keyring(),
        Err(err) => return Err(AuthError::Keyring(err.to_string())),
    };

    let mut tokens = serde_json::from_str::<StoredTokens>(&access_json)?;
    tokens.refresh_token = match legacy_entry(REFRESH_TOKEN_ACCOUNT)?.get_password() {
        Ok(value) => Some(value),
        Err(keyring::Error::NoEntry) => None,
        Err(err) => return Err(AuthError::Keyring(err.to_string())),
    };

    // Re-home under the current service name first; only remove the legacy
    // entries once the copy has succeeded, so a failure never loses the token.
    // A partial copy (access written, refresh not) is rolled back: otherwise
    // every later load takes the new-service path and never retries the
    // migration, stranding the still-present legacy refresh token and failing
    // with `MissingRefreshToken` once the access token expires. The rollback
    // only removes what this attempt wrote, and reports its own failures.
    if let Err(save_err) = KeyringTokenStore.save(&tokens) {
        if let Err(rollback_err) = roll_back_partial_migration(
            &access_entry_payload(&tokens)?,
            tokens.refresh_token.as_deref(),
        ) {
            // The partial access entry is still there and would shadow the
            // intact legacy credentials on every later load, so this cannot be
            // reported as a plain copy failure.
            return Err(AuthError::Keyring(format!(
                "migrating credentials to {SERVICE_NAME} failed ({save_err}) and the incomplete copy could not be removed ({rollback_err}); run `nolgia auth login` to re-authenticate"
            )));
        }
        return Err(save_err);
    }
    let _ = delete_legacy_entry(ACCESS_TOKEN_ACCOUNT);
    let _ = delete_legacy_entry(REFRESH_TOKEN_ACCOUNT);
    Ok(Some(tokens))
}

/// Removes the current-service access entry left behind by a failed migration
/// copy, so the next load retries the migration instead of reading a
/// refresh-less credential.
///
/// Only the entry this attempt wrote is removed. Another process may have
/// completed its own migration or a fresh login in the meantime, and deleting
/// its credentials while the successful migrator drops the legacy entries would
/// log the user out of both services. The refresh entry is never removed here:
/// a failed copy means our refresh write is what failed, so any refresh entry
/// present belongs to someone else, and a stale one is overwritten by the next
/// successful save.
///
/// Errors are returned rather than ignored: if the partial entry cannot be
/// confirmed gone it keeps shadowing the legacy credentials, which is the
/// `MissingRefreshToken` dead end the rollback exists to prevent.
fn roll_back_partial_migration(
    access_payload: &str,
    refresh_token: Option<&str>,
) -> std::result::Result<(), AuthError> {
    match entry(ACCESS_TOKEN_ACCOUNT)?.get_password() {
        // Already gone, or replaced with another process's credentials.
        Err(keyring::Error::NoEntry) => return Ok(()),
        Ok(current) if current != access_payload => return Ok(()),
        Ok(_) => {}
        Err(err) => return Err(AuthError::Keyring(err.to_string())),
    }

    // A matching refresh entry means the copy is complete after all (another
    // process finished it), so the pair is usable and must not be torn down.
    if let Some(refresh_token) = refresh_token {
        match entry(REFRESH_TOKEN_ACCOUNT)?.get_password() {
            Ok(current) if current == refresh_token => return Ok(()),
            Ok(_) | Err(keyring::Error::NoEntry) => {}
            Err(err) => return Err(AuthError::Keyring(err.to_string())),
        }
    }

    delete_entry(ACCESS_TOKEN_ACCOUNT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use chrono::Duration as ChronoDuration;
    use serde_json::json;
    use tokio::sync::Notify;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    #[derive(Clone, Default)]
    struct MemoryStore {
        tokens: Arc<Mutex<Option<StoredTokens>>>,
        deletes: Arc<Mutex<usize>>,
    }

    impl MemoryStore {
        fn with(tokens: StoredTokens) -> Self {
            Self {
                tokens: Arc::new(Mutex::new(Some(tokens))),
                deletes: Arc::default(),
            }
        }

        fn saved(&self) -> Option<StoredTokens> {
            self.tokens.lock().expect("tokens lock").clone()
        }

        fn delete_count(&self) -> usize {
            *self.deletes.lock().expect("deletes lock")
        }
    }

    impl TokenStore for MemoryStore {
        fn load(&self) -> std::result::Result<Option<StoredTokens>, AuthError> {
            Ok(self.saved())
        }

        fn save(&self, tokens: &StoredTokens) -> std::result::Result<(), AuthError> {
            *self.tokens.lock().expect("tokens lock") = Some(tokens.clone());
            Ok(())
        }

        fn delete(&self) -> std::result::Result<(), AuthError> {
            *self.tokens.lock().expect("tokens lock") = None;
            *self.deletes.lock().expect("deletes lock") += 1;
            Ok(())
        }
    }

    fn token(
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> StoredTokens {
        StoredTokens {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.map(str::to_string),
            expires_at,
        }
    }

    fn manager(server: &MockServer, store: MemoryStore) -> AuthManager<MemoryStore> {
        AuthManager::new(server.uri(), store).with_hooks(
            Arc::new(|_| Box::pin(async {})),
            Arc::new(|| Box::pin(std::future::pending())),
        )
    }

    /// No browser, no clipboard, narration discarded: the token mechanics only.
    async fn login(
        auth: &AuthManager<MemoryStore>,
    ) -> std::result::Result<LoginOutcome, AuthError> {
        let mut screen = LoginScreen::new(Vec::new(), false);
        auth.login(quiet(), &mut screen).await
    }

    fn quiet() -> LoginOptions {
        LoginOptions {
            open_browser: false,
            copy_code: false,
        }
    }

    /// Runs a login that narrates into a buffer, returning the outcome and
    /// what was printed.
    async fn login_screen(
        auth: &AuthManager<MemoryStore>,
        tty: bool,
    ) -> (std::result::Result<LoginOutcome, AuthError>, String) {
        let mut screen = LoginScreen::new(Vec::new(), tty);
        let outcome = auth.login(quiet(), &mut screen).await;
        let printed = String::from_utf8(screen.into_inner()).expect("utf-8 narration");
        (outcome, printed)
    }

    /// A device grant with the direct link the API really sends
    /// (`?code=`), expiring in `expires_in` seconds.
    async fn mount_direct_link_device(server: &MockServer, expires_in: u64) {
        Mock::given(method("POST"))
            .and(path("/v1/auth/device"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "dev-1",
                "user_code": "YKKQ-RXKS",
                "verification_uri": "https://nolgia.ai/device",
                "verification_uri_complete": "https://nolgia.ai/device?code=YKKQ-RXKS",
                "expires_in": expires_in,
                "interval": 1
            })))
            .mount(server)
            .await;
    }

    async fn mount_me(server: &MockServer, access_token: &str, email: &str) {
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .and(header("authorization", format!("Bearer {access_token}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "email": email })))
            .mount(server)
            .await;
    }

    /// Piped output (an agent, a log): the link, the code, one waiting line,
    /// one connected line. Nothing is redrawn because nothing can be.
    #[tokio::test]
    async fn login_narration_when_not_a_tty_prints_each_line_once() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());
        mount_direct_link_device(&server, 900).await;
        mount_token(&server, "access-1", Some("refresh-1")).await;
        mount_me(&server, "access-1", "admin@nolgia.ai").await;

        let (outcome, printed) = login_screen(&auth, false).await;

        outcome.expect("login succeeds");
        assert_eq!(
            printed,
            "Open: https://nolgia.ai/device?code=YKKQ-RXKS\n\
             \n\
             \x20 Code: YKKQ-RXKS\n\
             \n\
             Waiting for you to approve in the browser... (expires in 15:00)\n\
             \u{2705} Connected as admin@nolgia.ai\n"
        );
    }

    /// A terminal: the same link and code, then the waiting line is redrawn
    /// in place (carriage return, no newline) and finally replaced by the
    /// connected line.
    #[tokio::test]
    async fn login_narration_on_a_tty_redraws_the_waiting_line_in_place() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());
        mount_direct_link_device(&server, 900).await;
        mount_token(&server, "access-1", Some("refresh-1")).await;
        mount_me(&server, "access-1", "admin@nolgia.ai").await;

        let (outcome, printed) = login_screen(&auth, true).await;

        outcome.expect("login succeeds");
        let prompt = "Open: https://nolgia.ai/device?code=YKKQ-RXKS\n\n  Code: YKKQ-RXKS\n\n";
        let rest = printed
            .strip_prefix(prompt)
            .expect("link and code come first");
        let waiting = format!("\r{WAITING_PREFIX}15:00)");
        // Drawn once before the first sleep and once after it: same row.
        assert!(
            rest.starts_with(&format!("{waiting}{waiting}")),
            "waiting line is redrawn with a carriage return, got {rest:?}"
        );
        assert!(
            !rest.contains("Waiting for you to approve in the browser... (expires in 15:00)\n"),
            "the waiting line never ends in a newline on a TTY"
        );
        let last = rest.rsplit('\r').next().expect("a final redraw");
        assert_eq!(
            last.trim_end_matches(' '),
            "\u{2705} Connected as admin@nolgia.ai\n"
        );
    }

    /// The account line is a courtesy: when `GET /me` fails the login still
    /// succeeded and still says so.
    #[tokio::test]
    async fn login_narration_says_connected_without_an_email_when_me_fails() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());
        mount_direct_link_device(&server, 900).await;
        mount_token(&server, "access-1", Some("refresh-1")).await;

        let (outcome, printed) = login_screen(&auth, false).await;

        outcome.expect("login succeeds");
        assert!(printed.ends_with("\u{2705} Connected\n"), "got {printed:?}");
    }

    /// A code that runs out: the waiting line shows 0:00, the login fails
    /// with `Expired`, and on a TTY the row is cleared so the error that
    /// follows starts on a clean line.
    #[tokio::test]
    async fn login_narration_on_expiry_clears_the_waiting_line() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());
        mount_direct_link_device(&server, 0).await;

        let (outcome, printed) = login_screen(&auth, true).await;

        assert!(matches!(outcome, Err(AuthError::Expired)));
        let waiting = format!("\r{WAITING_PREFIX}0:00)");
        assert!(printed.contains(&waiting), "got {printed:?}");
        let last = printed.rsplit('\r').next().expect("a final redraw");
        assert_eq!(
            last.trim_end_matches(' '),
            "",
            "row is blanked, got {last:?}"
        );
        assert!(
            LOGIN_EXPIRED_MESSAGE.contains("expired")
                && LOGIN_EXPIRED_MESSAGE.contains("nolgia auth login"),
            "the expiry message says so and how to retry"
        );
    }

    /// Piped output on expiry: the waiting line was printed once, with a
    /// newline, and nothing else is added.
    #[tokio::test]
    async fn login_narration_on_expiry_when_not_a_tty_adds_nothing() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());
        mount_direct_link_device(&server, 0).await;

        let (outcome, printed) = login_screen(&auth, false).await;

        assert!(matches!(outcome, Err(AuthError::Expired)));
        assert!(
            printed.ends_with(&format!("{WAITING_PREFIX}0:00)\n")),
            "got {printed:?}"
        );
    }

    #[test]
    fn login_prompt_link_prefers_the_direct_link() {
        let mut prompt = LoginPrompt {
            user_code: "YKKQ-RXKS".into(),
            verification_uri: "https://nolgia.ai/device".into(),
            verification_uri_complete: Some("https://nolgia.ai/device?code=YKKQ-RXKS".into()),
            expires_in: 900,
        };
        assert_eq!(prompt.link(), "https://nolgia.ai/device?code=YKKQ-RXKS");
        prompt.verification_uri_complete = None;
        assert_eq!(prompt.link(), "https://nolgia.ai/device");
    }

    #[test]
    fn remaining_time_rounds_up_to_the_next_second() {
        assert_eq!(format_remaining(Duration::from_secs(900)), "15:00");
        assert_eq!(format_remaining(Duration::from_millis(899_500)), "15:00");
        assert_eq!(format_remaining(Duration::from_secs(59)), "0:59");
        assert_eq!(format_remaining(Duration::from_secs(0)), "0:00");
    }

    /// The clipboard suffix rides on the code line only when a copy happened.
    #[test]
    fn prompt_marks_the_code_copied_only_when_it_was() {
        let prompt = LoginPrompt {
            user_code: "YKKQ-RXKS".into(),
            verification_uri: "https://nolgia.ai/device".into(),
            verification_uri_complete: Some("https://nolgia.ai/device?code=YKKQ-RXKS".into()),
            expires_in: 900,
        };
        let mut copied = LoginScreen::new(Vec::new(), true);
        copied.prompt(&prompt, true);
        assert_eq!(
            String::from_utf8(copied.into_inner()).unwrap(),
            "Open: https://nolgia.ai/device?code=YKKQ-RXKS\n\n  Code: YKKQ-RXKS  (copied to clipboard)\n\n"
        );
        let mut plain = LoginScreen::new(Vec::new(), true);
        plain.prompt(&prompt, false);
        assert_eq!(
            String::from_utf8(plain.into_inner()).unwrap(),
            "Open: https://nolgia.ai/device?code=YKKQ-RXKS\n\n  Code: YKKQ-RXKS\n\n"
        );
    }

    #[tokio::test]
    async fn login_starts_device_flow_polls_and_stores_tokens() {
        let server = MockServer::start().await;
        let store = MemoryStore::default();
        let auth = manager(&server, store.clone());

        Mock::given(method("POST"))
            .and(path("/v1/auth/device"))
            .and(body_json(
                json!({ "client_id": CLIENT_ID, "scope": DEFAULT_SCOPE }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "dev-1",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://nolgia.ai/device",
                "verification_uri_complete": "https://nolgia.ai/device?user_code=ABCD-EFGH",
                "expires_in": 900,
                "interval": 1
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/device/token"))
            .and(body_json(
                json!({ "client_id": CLIENT_ID, "device_code": "dev-1" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "access-1",
                "refresh_token": "refresh-1",
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;

        let outcome = login(&auth).await.expect("login succeeds");

        assert_eq!(outcome.prompt.user_code, "ABCD-EFGH");
        assert_eq!(
            store.saved().expect("tokens saved").access_token,
            "access-1"
        );
        assert_eq!(
            store
                .saved()
                .expect("tokens saved")
                .refresh_token
                .as_deref(),
            Some("refresh-1")
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn login_continues_while_authorization_is_pending() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());

        mount_device(&server, 900, 1).await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/device/token"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(json!({ "error": "authorization_pending" })),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_token(
            &server,
            "access-after-pending",
            Some("refresh-after-pending"),
        )
        .await;

        let outcome = login(&auth).await.expect("login succeeds after pending");

        assert_eq!(outcome.tokens.access_token, "access-after-pending");
    }

    #[tokio::test]
    async fn login_honors_slow_down_response() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());

        mount_device(&server, 900, 1).await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/device/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "slow_down" })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        mount_token(&server, "access-after-slow", Some("refresh-after-slow")).await;

        let outcome = login(&auth).await.expect("login succeeds after slow_down");

        assert_eq!(outcome.tokens.access_token, "access-after-slow");
    }

    #[tokio::test]
    async fn login_returns_expired_when_server_expires_device_code() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());

        mount_device(&server, 900, 1).await;
        Mock::given(method("POST"))
            .and(path("/v1/auth/device/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({ "error": "expired_token" })),
            )
            .mount(&server)
            .await;

        let err = login(&auth).await.expect_err("login expires");

        assert!(matches!(err, AuthError::Expired));
    }

    #[tokio::test]
    async fn login_returns_canceled_when_ctrl_c_wins_poll_wait() {
        let server = MockServer::start().await;
        let auth = AuthManager::new(server.uri(), MemoryStore::default()).with_hooks(
            Arc::new(|_| Box::pin(std::future::pending())),
            Arc::new(|| Box::pin(async {})),
        );

        mount_device(&server, 900, 1).await;

        let err = login(&auth).await.expect_err("login canceled");

        assert!(matches!(err, AuthError::Canceled));
    }

    #[tokio::test]
    async fn valid_tokens_refreshes_expired_access_token() {
        let server = MockServer::start().await;
        let store = MemoryStore::with(token(
            "old",
            Some("refresh-old"),
            Utc::now() - ChronoDuration::minutes(1),
        ));
        let auth = manager(&server, store.clone());
        mount_refresh(&server, "refresh-old", "new", Some("refresh-new")).await;

        let tokens = auth.valid_tokens().await.expect("refresh succeeds");

        assert_eq!(tokens.access_token, "new");
        assert_eq!(store.saved().expect("saved").access_token, "new");
    }

    #[tokio::test]
    async fn valid_tokens_rejects_expired_token_without_refresh_token() {
        let server = MockServer::start().await;
        let store = MemoryStore::with(token("old", None, Utc::now() - ChronoDuration::minutes(1)));
        let auth = manager(&server, store);

        let err = auth
            .valid_tokens()
            .await
            .expect_err("missing refresh token");

        assert!(matches!(err, AuthError::MissingRefreshToken));
    }

    #[tokio::test]
    async fn status_prints_email_and_tier_for_valid_token() {
        let server = MockServer::start().await;
        let store = MemoryStore::with(token(
            "access-ok",
            Some("refresh-ok"),
            Utc::now() + ChronoDuration::hours(1),
        ));
        let auth = manager(&server, store);
        mount_user(&server, "access-ok", 200).await;
        mount_subscription(&server, "access-ok", 200, "pro").await;

        let status = auth.status().await.expect("status succeeds");

        assert_eq!(status.email, "ada@nolgia.ai");
        assert_eq!(status.tier, "pro");
        assert_eq!(
            status.organization, None,
            "no active_organization = personal"
        );
    }

    /// `GET /me` carries `active_organization` in an organization context;
    /// `auth status` must surface it so a caller knows which pool a
    /// generation will spend before spending.
    #[tokio::test]
    async fn status_reports_the_active_organization() {
        let server = MockServer::start().await;
        let store = MemoryStore::with(token(
            "access-ok",
            Some("refresh-ok"),
            Utc::now() + ChronoDuration::hours(1),
        ));
        let auth = manager(&server, store);
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .and(header("authorization", "Bearer access-ok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "2f2f1a1d-7d1c-4d34-91fd-28a4d5e5d5e5",
                "email": "ada@nolgia.ai",
                "created_at": "2026-06-13T00:00:00Z",
                "organizations": [{
                    "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "name": "Acme Studios",
                    "slug": "acme-studios", "kind": "team", "role": "owner"
                }],
                "active_organization": {
                    "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "name": "Acme Studios",
                    "slug": "acme-studios", "kind": "team", "role": "owner"
                }
            })))
            .mount(&server)
            .await;
        mount_subscription(&server, "access-ok", 200, "team").await;

        let status = auth.status().await.expect("status succeeds");

        assert_eq!(status.tier, "team");
        let org = status.organization.expect("active organization");
        assert_eq!(org.slug, "acme-studios");
        assert_eq!(org.role, "owner");
        assert_eq!(
            organization_line(Some(&org)),
            "Organization: Acme Studios (acme-studios) as owner"
        );
        assert_eq!(organization_line(None), "Organization: Personal space");
    }

    #[tokio::test]
    async fn status_refreshes_after_401_then_retries_user_call() {
        let server = MockServer::start().await;
        let store = MemoryStore::with(token(
            "stale",
            Some("refresh-stale"),
            Utc::now() + ChronoDuration::hours(1),
        ));
        let auth = manager(&server, store.clone());

        mount_user(&server, "stale", 401).await;
        mount_refresh(&server, "refresh-stale", "fresh", Some("refresh-fresh")).await;
        mount_user(&server, "fresh", 200).await;
        mount_subscription(&server, "fresh", 200, "studio").await;

        let status = auth.status().await.expect("status refreshes");

        assert_eq!(status.email, "ada@nolgia.ai");
        assert_eq!(status.tier, "studio");
        assert_eq!(store.saved().expect("saved").access_token, "fresh");
    }

    #[tokio::test]
    async fn status_returns_not_logged_in_when_keyring_is_empty() {
        let server = MockServer::start().await;
        let auth = manager(&server, MemoryStore::default());

        let err = auth.status().await.expect_err("not logged in");

        assert!(matches!(err, AuthError::NotLoggedIn));
    }

    #[test]
    fn logout_removes_stored_tokens() {
        let store = MemoryStore::with(token(
            "access",
            Some("refresh"),
            Utc::now() + ChronoDuration::hours(1),
        ));
        let auth = AuthManager::new("https://api.nolgia.ai", store.clone());

        auth.logout().expect("logout succeeds");

        assert!(store.saved().is_none());
        assert_eq!(store.delete_count(), 1);
    }

    #[test]
    fn keyring_store_serializes_access_and_refresh_separately() {
        let tokens = token(
            "access",
            Some("refresh"),
            Utc::now() + ChronoDuration::hours(1),
        );
        let mut access_only = tokens.clone();
        access_only.refresh_token = None;

        let access_json = serde_json::to_string(&access_only).expect("serializes");
        let refresh_value = tokens.refresh_token.clone().expect("refresh token");
        let mut map = HashMap::new();
        map.insert(ACCESS_TOKEN_ACCOUNT, access_json);
        map.insert(REFRESH_TOKEN_ACCOUNT, refresh_value);

        let mut loaded: StoredTokens =
            serde_json::from_str(map.get(ACCESS_TOKEN_ACCOUNT).expect("access"))
                .expect("loads access");
        loaded.refresh_token = map.get(REFRESH_TOKEN_ACCOUNT).cloned();

        assert_eq!(loaded.access_token, "access");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh"));
    }

    #[test]
    fn file_store_roundtrips_and_deletes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileTokenStore::new(dir.path().join("nolgia").join("tokens.json"));
        assert!(store.load().expect("empty load").is_none());

        let tokens = token(
            "access",
            Some("refresh"),
            Utc::now() + ChronoDuration::hours(1),
        );
        store.save(&tokens).expect("save succeeds");
        assert_eq!(store.load().expect("load").expect("saved"), tokens);

        store.delete().expect("delete succeeds");
        assert!(store.load().expect("load after delete").is_none());
        store.delete().expect("delete is idempotent");
    }

    #[cfg(unix)]
    #[test]
    fn file_store_writes_0600_in_0700_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileTokenStore::new(dir.path().join("nolgia").join("tokens.json"));
        store
            .save(&token("access", None, Utc::now()))
            .expect("save succeeds");

        let file_mode = std::fs::metadata(dir.path().join("nolgia/tokens.json"))
            .expect("file metadata")
            .permissions()
            .mode();
        let dir_mode = std::fs::metadata(dir.path().join("nolgia"))
            .expect("dir metadata")
            .permissions()
            .mode();
        assert_eq!(file_mode & 0o777, 0o600);
        assert_eq!(dir_mode & 0o777, 0o700);
    }

    #[test]
    fn keyring_migration_runs_at_most_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = FileTokenStore::new(dir.path().join("tokens.json"));
        let legacy = MemoryStore::with(token(
            "keyring-access",
            Some("keyring-refresh"),
            Utc::now() + ChronoDuration::hours(1),
        ));

        // First probe migrates the legacy tokens into the file...
        let migrated = migrate_keyring_once(&file, &legacy).expect("tokens migrate");
        assert_eq!(migrated.access_token, "keyring-access");
        assert_eq!(
            file.load()
                .expect("file load")
                .expect("migrated to file")
                .access_token,
            "keyring-access"
        );

        // ...and never probes the source again, even after logout.
        file.delete().expect("logout");
        assert!(migrate_keyring_once(&file, &legacy).is_none());
    }

    #[test]
    fn keyring_migration_marks_attempt_even_when_source_is_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = FileTokenStore::new(dir.path().join("tokens.json"));
        let legacy = MemoryStore::default();

        assert!(migrate_keyring_once(&file, &legacy).is_none());

        // A later login to the legacy store must NOT resurface: the single
        // permitted probe already happened (this is what stops repeated
        // keychain password prompts when the user denies access).
        legacy
            .save(&token("late", None, Utc::now() + ChronoDuration::hours(1)))
            .expect("save");
        assert!(migrate_keyring_once(&file, &legacy).is_none());
    }

    #[tokio::test]
    async fn login_prompt_is_available_before_first_poll_wait() {
        let server = MockServer::start().await;
        let notify = Arc::new(Notify::new());
        let sleep_notify = notify.clone();
        let auth = AuthManager::new(server.uri(), MemoryStore::default()).with_hooks(
            Arc::new(move |_| {
                let sleep_notify = sleep_notify.clone();
                Box::pin(async move {
                    sleep_notify.notify_one();
                    std::future::pending::<()>().await;
                })
            }),
            Arc::new(|| Box::pin(std::future::pending())),
        );

        mount_device(&server, 900, 1).await;
        let login = tokio::spawn(async move { login(&auth).await });

        tokio::time::timeout(Duration::from_secs(2), notify.notified())
            .await
            .expect("login reached poll sleep within two seconds");
        login.abort();
    }

    async fn mount_device(server: &MockServer, expires_in: u64, interval: u64) {
        Mock::given(method("POST"))
            .and(path("/v1/auth/device"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "dev-1",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://nolgia.ai/device",
                "verification_uri_complete": null,
                "expires_in": expires_in,
                "interval": interval
            })))
            .mount(server)
            .await;
    }

    async fn mount_token(server: &MockServer, access_token: &str, refresh_token: Option<&str>) {
        Mock::given(method("POST"))
            .and(path("/v1/auth/device/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token,
                "refresh_token": refresh_token,
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .mount(server)
            .await;
    }

    async fn mount_refresh(
        server: &MockServer,
        refresh_token: &str,
        access_token: &str,
        new_refresh: Option<&str>,
    ) {
        Mock::given(method("POST"))
            .and(path("/v1/auth/device/token"))
            .and(body_json(
                json!({ "client_id": CLIENT_ID, "device_code": refresh_token }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": access_token,
                "refresh_token": new_refresh,
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .mount(server)
            .await;
    }

    async fn mount_user(server: &MockServer, token: &str, status: u16) {
        let template = if status == 200 {
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "2f2f1a1d-7d1c-4d34-91fd-28a4d5e5d5e5",
                "email": "ada@nolgia.ai",
                "created_at": "2026-06-13T00:00:00Z"
            }))
        } else {
            ResponseTemplate::new(status)
        };
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(template)
            .mount(server)
            .await;
    }

    async fn mount_subscription(server: &MockServer, token: &str, status: u16, tier: &str) {
        let template = if status == 200 {
            ResponseTemplate::new(200).set_body_json(json!({
                "tier": tier,
                "status": "active",
                "current_period_end": "2026-06-13T00:00:00Z"
            }))
        } else {
            ResponseTemplate::new(status)
        };
        Mock::given(method("GET"))
            .and(path("/v1/billing/subscription"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(template)
            .mount(server)
            .await;
    }
}
