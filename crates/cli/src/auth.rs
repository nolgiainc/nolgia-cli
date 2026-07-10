use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
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

pub const SERVICE_NAME: &str = "com.nolgiacorp.nolgia";
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
    Login,
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

    pub async fn login(&self) -> std::result::Result<LoginOutcome, AuthError> {
        let device = self.start_device_auth().await?;
        let prompt = LoginPrompt::from(&device);
        print_login_prompt(&prompt);

        let token = self.poll_device_token(&device).await?;
        let tokens = StoredTokens::from_token_response(token);
        self.store.save(&tokens)?;

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
        let status = AuthStatus {
            email: user.email,
            tier,
        };
        println!("{} ({})", status.email, status.tier);
        Ok(status)
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

        let status = AuthStatus {
            email: user.email,
            tier,
        };
        println!("{} ({})", status.email, status.tier);
        Ok(status)
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

    async fn poll_device_token(
        &self,
        device: &DeviceAuthResponse,
    ) -> std::result::Result<DeviceTokenResponse, AuthError> {
        let deadline = Instant::now() + Duration::from_secs(device.expires_in);
        let mut interval = Duration::from_secs(device.interval);

        loop {
            if Instant::now() >= deadline {
                return Err(AuthError::Expired);
            }

            tokio::select! {
                () = (self.sleep)(interval) => {},
                () = (self.cancel)() => return Err(AuthError::Canceled),
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
        let access = entry(ACCESS_TOKEN_ACCOUNT)?.get_password();
        let access_json = match access {
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

    fn save(&self, tokens: &StoredTokens) -> std::result::Result<(), AuthError> {
        let mut access_only = tokens.clone();
        access_only.refresh_token = None;
        entry(ACCESS_TOKEN_ACCOUNT)?
            .set_password(&serde_json::to_string(&access_only)?)
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
            use std::io::Write;
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
        AuthCommand::Login => emit_login(format, &manager.login().await?),
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
        OutputFormat::Text => Ok(()),
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

fn print_login_prompt(prompt: &LoginPrompt) {
    println!("Open: {}", prompt.verification_uri);
    println!("Code: {}", prompt.user_code);
    if let Some(uri) = &prompt.verification_uri_complete {
        println!("Direct link: {uri}");
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

fn entry(account: &str) -> std::result::Result<keyring::Entry, AuthError> {
    keyring::Entry::new(SERVICE_NAME, account).map_err(|err| AuthError::Keyring(err.to_string()))
}

fn delete_entry(account: &str) -> std::result::Result<(), AuthError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(AuthError::Keyring(err.to_string())),
    }
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

        let outcome = auth.login().await.expect("login succeeds");

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

        let outcome = auth.login().await.expect("login succeeds after pending");

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

        let outcome = auth.login().await.expect("login succeeds after slow_down");

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

        let err = auth.login().await.expect_err("login expires");

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

        let err = auth.login().await.expect_err("login canceled");

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
        let login = tokio::spawn(async move { auth.login().await });

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
