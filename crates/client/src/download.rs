//! Fetching a finished asset — the last reason a first program needed a
//! second crate (NOL-1087).
//!
//! Hand-written, like [`crate::subscribe`]: nothing in the codegen path writes
//! here, so re-vendoring the spec cannot clobber it.
//!
//! The quickstart used to end with `reqwest::get(&asset.signed_url)`, which is
//! why the install line said `cargo add nolgia-client tokio reqwest`. The
//! crate already compiles reqwest; it just never handed callers a way to use
//! it. [`ClientExt::download`](crate::ClientExt::download) and
//! [`download_bytes`](crate::ClientExt::download_bytes) close that, so the
//! whole install is `cargo add nolgia-client`.

use std::{
    fmt,
    path::{Path, PathBuf},
    result::Result as StdResult,
    sync::OnceLock,
};

use reqwest::Url;
use tokio::io::AsyncWriteExt;

use crate::{Client, generated::ClientInfo};

/// Why a download did not produce a file.
///
/// `Debug` mirrors `Display` for the same reason [`crate::ClientBuilderError`]
/// does: `fn main() -> Result<_, Box<dyn Error>>` prints the error with `{:?}`,
/// and a derived `Debug` would show a new user `Status { status: 403, .. }`
/// instead of the sentence telling them what to do about it.
#[non_exhaustive]
pub enum DownloadError {
    /// The string was not a URL. Already redacted; see [`DownloadError`]'s
    /// note on query strings.
    InvalidUrl(String),
    /// The request never reached a response: DNS, TLS, connection, timeout.
    ///
    /// Constructed through `reqwest::Error::without_url`, because a reqwest
    /// error Displays the URL it failed on and an asset URL carries its
    /// signature in the query string.
    Transport(reqwest::Error),
    /// The host answered with a non-success status. A signed URL that has
    /// outlived its expiry answers `403` here.
    Status {
        /// The HTTP status the host returned.
        status: u16,
        /// The URL, with its query string removed.
        url: String,
    },
    /// The response began but the body could not be read to the end.
    Body(reqwest::Error),
    /// The destination could not be created, written, flushed or renamed.
    Io {
        /// The path being written when this failed.
        path: PathBuf,
        /// The underlying filesystem error.
        source: std::io::Error,
    },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "not a URL: {url}"),
            Self::Transport(err) => write!(f, "download failed: {err}"),
            Self::Status { status: 403, url } => write!(
                f,
                "{url} answered 403 — a signed asset URL expires, so fetch the \
                 job or asset again for a fresh one"
            ),
            Self::Status { status, url } => write!(f, "{url} answered {status}"),
            Self::Body(err) => write!(f, "download was cut short: {err}"),
            Self::Io { path, source } => write!(f, "could not write {}: {source}", path.display()),
        }
    }
}

impl fmt::Debug for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for DownloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(err) | Self::Body(err) => Some(err),
            Self::Io { source, .. } => Some(source),
            Self::InvalidUrl(_) | Self::Status { .. } => None,
        }
    }
}

/// Read the whole body into memory.
pub(crate) async fn bytes(client: &Client, url: &str) -> StdResult<Vec<u8>, DownloadError> {
    // A hostile or mistaken Content-Length must not let a caller allocate an
    // arbitrary buffer up front; 8 MiB covers an image, and Vec grows.
    const PREALLOCATE_LIMIT: u64 = 8 * 1024 * 1024;

    let mut response = send(client, url).await?;
    let hint = response
        .content_length()
        .unwrap_or(0)
        .min(PREALLOCATE_LIMIT);
    let mut buffer = Vec::with_capacity(hint as usize);
    while let Some(chunk) = response.chunk().await.map_err(body_error)? {
        buffer.extend_from_slice(&chunk);
    }
    Ok(buffer)
}

/// Stream the body to `path`, returning the number of bytes written.
///
/// The bytes land in a sibling `<path>.part` and are renamed into place only
/// once the body has been read to the end, so an interrupted video download
/// never leaves a half-written file that looks finished.
pub(crate) async fn to_path(
    client: &Client,
    url: &str,
    path: &Path,
) -> StdResult<u64, DownloadError> {
    let mut response = send(client, url).await?;
    let partial = partial_path(path);

    let file = tokio::fs::File::create(&partial)
        .await
        .map_err(|source| DownloadError::Io {
            path: partial.clone(),
            source,
        })?;
    let mut writer = tokio::io::BufWriter::new(file);

    let mut written: u64 = 0;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => match writer.write_all(&chunk).await {
                Ok(()) => written += chunk.len() as u64,
                Err(source) => {
                    return Err(discard(
                        &partial,
                        DownloadError::Io {
                            path: partial.clone(),
                            source,
                        },
                    )
                    .await);
                }
            },
            Ok(None) => break,
            Err(err) => return Err(discard(&partial, body_error(err)).await),
        }
    }

    if let Err(source) = writer.flush().await {
        return Err(discard(
            &partial,
            DownloadError::Io {
                path: partial.clone(),
                source,
            },
        )
        .await);
    }
    // Flushing the BufWriter only moves bytes into the OS; a rename over an
    // unsynced file can survive a crash as a zero-length destination.
    if let Err(source) = writer.into_inner().sync_all().await {
        return Err(discard(
            &partial,
            DownloadError::Io {
                path: partial.clone(),
                source,
            },
        )
        .await);
    }
    if let Err(source) = tokio::fs::rename(&partial, path).await {
        return Err(discard(
            &partial,
            DownloadError::Io {
                path: path.to_path_buf(),
                source,
            },
        )
        .await);
    }
    Ok(written)
}

/// Remove the half-written file, then return the error that caused it.
async fn discard(partial: &Path, error: DownloadError) -> DownloadError {
    // A failure to clean up is not the failure worth reporting.
    let _ = tokio::fs::remove_file(partial).await;
    error
}

async fn send(client: &Client, url: &str) -> StdResult<reqwest::Response, DownloadError> {
    let target: Url = url
        .parse()
        .map_err(|_| DownloadError::InvalidUrl(redact(url)))?;

    let response = if same_origin(client.baseurl(), &target) {
        // Our own API: the client's Authorization, calling surface and
        // idempotency key all belong on this request.
        client.client().get(target).send().await
    } else {
        // Somewhere else — in practice the storage host behind
        // `asset.signed_url`. The URL already carries its own credential in
        // the query string, so our bearer token is not needed; sending it
        // would hand a third party a token that can spend money, and Google
        // Cloud Storage rejects a V4 signed request that also presents an
        // Authorization header. reqwest applies a client's default headers at
        // send time, so the only way to leave them off is a second client.
        anonymous().get(target).send().await
    }
    .map_err(|err| DownloadError::Transport(err.without_url()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(DownloadError::Status {
            status: status.as_u16(),
            url: redact(url),
        });
    }
    Ok(response)
}

/// The body is what failed, not the URL — and the URL must not travel with
/// the error (see [`DownloadError::Transport`]).
fn body_error(err: reqwest::Error) -> DownloadError {
    DownloadError::Body(err.without_url())
}

/// A client with no default headers, shared so repeated downloads reuse one
/// connection pool.
fn anonymous() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

fn same_origin(base_url: &str, target: &Url) -> bool {
    Url::parse(base_url).is_ok_and(|base| {
        base.scheme() == target.scheme()
            && base.host_str() == target.host_str()
            && base.port_or_known_default() == target.port_or_known_default()
    })
}

/// A signed URL's query string *is* the credential. It must never reach an
/// error message, a log line or a terminal.
fn redact(url: &str) -> String {
    match url.split_once('?') {
        Some((head, _)) => format!("{head}?…"),
        None => url.to_string(),
    }
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_string_never_survives_into_an_error() {
        let signed = "https://storage.googleapis.com/bucket/first.png?X-Goog-Signature=deadbeef";
        let redacted = redact(signed);
        assert_eq!(
            redacted,
            "https://storage.googleapis.com/bucket/first.png?…"
        );
        assert!(!redacted.contains("deadbeef"), "{redacted}");
        let error = DownloadError::Status {
            status: 403,
            url: redacted,
        };
        // Both renderings, because `Box<dyn Error>` from `main` uses Debug.
        assert!(!format!("{error}").contains("deadbeef"));
        assert!(!format!("{error:?}").contains("deadbeef"));
        assert!(format!("{error}").contains("expires"), "{error}");
    }

    #[test]
    fn origin_compares_scheme_host_and_port_with_the_default_filled_in() {
        let parse = |url: &str| Url::parse(url).expect(url);
        assert!(same_origin(
            "https://api.nolgia.ai/v1",
            &parse("https://api.nolgia.ai/v1/assets/1")
        ));
        // The API's own base URL never names :443, an absolute URL may.
        assert!(same_origin(
            "https://api.nolgia.ai/v1",
            &parse("https://api.nolgia.ai:443/v1/assets/1")
        ));
        // A storage host is a different origin even though it is also https.
        assert!(!same_origin(
            "https://api.nolgia.ai/v1",
            &parse("https://storage.googleapis.com/bucket/first.png")
        ));
        // Same host, different port: a local API and a local stub are not
        // one origin.
        assert!(!same_origin(
            "http://127.0.0.1:8080/v1",
            &parse("http://127.0.0.1:9090/first.png")
        ));
        // A base URL we cannot parse is treated as "not ours", which errs
        // toward withholding the token.
        assert!(!same_origin("not a url", &parse("https://example.com/x")));
    }

    #[test]
    fn the_partial_file_is_a_sibling_so_the_rename_is_atomic() {
        let partial = partial_path(Path::new("/tmp/out/first.mp4"));
        assert_eq!(partial, Path::new("/tmp/out/first.mp4.part"));
        assert_eq!(partial.parent(), Path::new("/tmp/out/first.mp4").parent());
    }
}
