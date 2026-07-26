pub mod ability;
pub mod account;
pub mod assets;
pub mod billing;
pub mod characters;
pub mod color_presets;
pub mod r#gen;
pub mod models;
pub mod pat;
pub mod projects;
pub mod skills;
pub mod status;
pub mod wait;

use crate::output::OutputFormat;
use nolgia_client::Client;

/// RFC 7807 problem body the API returns on every error response.
#[derive(serde::Deserialize)]
struct Problem {
    title: Option<String>,
    detail: Option<String>,
}

/// Convert a generated-client error into an anyhow error that surfaces the
/// server's RFC 7807 `detail` verbatim. The API validates requests against
/// per-model capabilities and names the violated capability in `detail`
/// (e.g. available quality tiers, reference caps) — far more actionable
/// than progenitor's opaque "Unexpected Response" debug dump.
pub(crate) async fn api_error(err: nolgia_client::ApiError<()>, action: &str) -> anyhow::Error {
    if let nolgia_client::ApiError::UnexpectedResponse(response) = err {
        let status = response.status();
        let message = match response.text().await {
            Ok(body) => serde_json::from_str::<Problem>(&body)
                .ok()
                .and_then(|p| p.detail.or(p.title))
                .or_else(|| {
                    let trimmed = body.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                }),
            Err(_) => None,
        };
        return match message {
            Some(message) => anyhow::anyhow!("{action}: {status}: {message}"),
            None => anyhow::anyhow!("{action}: {status}"),
        };
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
