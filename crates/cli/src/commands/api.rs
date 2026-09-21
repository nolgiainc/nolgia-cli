use std::io::{Read, Write};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use nolgia_client::ClientInfo;
use reqwest::header::{CONTENT_LENGTH, HeaderName, HeaderValue};
use serde_json::Value;

use super::CommandContext;
use crate::agent_guard::AgentRefused;
use crate::output::{print_json, print_json_unselected};

#[derive(Args, Debug)]
#[command(
    long_about = "Send one authenticated request to the API and print its response. JSON output is implied; --json is not required. Pass --body as inline JSON, @FILE, or - for stdin. Paths start with /; a leading /v1 is stripped because the client already adds it."
)]
pub struct ApiArgs {
    /// Choose the HTTP method (case-insensitive)
    #[arg(value_enum, ignore_case = true)]
    pub method: Method,
    /// Pass an API path starting with /; a leading /v1 is stripped
    pub path: String,
    /// Send JSON from inline text, @FILE, or - for stdin
    #[arg(long, value_name = "JSON|@FILE|-")]
    pub body: Option<String>,
    /// Append a URL-encoded query parameter; repeatable
    #[arg(long, value_name = "KEY=VALUE")]
    pub query: Vec<String>,
    /// Add a request header after the client's defaults; repeatable
    #[arg(long, value_name = "NAME: VALUE")]
    pub header: Vec<String>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
#[value(rename_all = "UPPER")]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

impl Method {
    const fn http(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
            Self::Head => reqwest::Method::HEAD,
        }
    }
}

fn api_path(path: &str) -> Result<&str> {
    if !path.starts_with('/') {
        bail!("pass an API path such as /jobs/<id>; the server comes from --api-url");
    }
    Ok(match path.strip_prefix("/v1") {
        Some("") => "/",
        Some(suffix) if suffix.starts_with(['/', '?', '#']) => suffix,
        Some(_) | None => path,
    })
}

fn read_body(source: &str) -> Result<Value> {
    let text = match source {
        "-" => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("reading JSON body from stdin")?;
            text
        }
        source if source.starts_with('@') => std::fs::read_to_string(&source[1..])
            .with_context(|| format!("reading JSON body from {}", &source[1..]))?,
        source => source.to_owned(),
    };
    serde_json::from_str(&text).context("parsing --body as JSON")
}

pub async fn run(args: ApiArgs, ctx: &CommandContext) -> Result<()> {
    let path = api_path(&args.path)?;
    // Append rather than join: even a path beginning with // cannot replace
    // the authenticated client's host.
    let url = reqwest::Url::parse(&format!("{}{path}", ctx.client().baseurl()))
        .context("building API request URL")?;
    if ctx.agent().is_some() {
        match (args.method, api_path(url.path())?) {
            (Method::Put, "/me/active-organization") => return Err(AgentRefused::Switch.into()),
            (Method::Post, "/organizations") => return Err(AgentRefused::Create.into()),
            _ => {}
        }
    }
    let method = args.method.http();
    let mut request = ctx.client().client().request(method.clone(), url);
    for query in &args.query {
        let (key, value) = query.split_once('=').context("pass --query as KEY=VALUE")?;
        request = request.query(&[(key, value)]);
    }
    match args.body.as_deref() {
        Some(source) => request = request.json(&read_body(source)?),
        None => match args.method {
            Method::Post | Method::Put | Method::Patch => {
                // The production load balancer rejects bodyless writes with
                // 411; match ClientExt::finish_asset_upload's explicit body.
                request = request.header(CONTENT_LENGTH, "0").body(Vec::<u8>::new());
            }
            Method::Get | Method::Delete | Method::Head => {}
        },
    }
    for header in &args.header {
        let (name, value) = header
            .split_once(':')
            .context("pass --header as NAME: VALUE")?;
        let name = HeaderName::from_bytes(name.trim().as_bytes()).context("invalid header name")?;
        let value = HeaderValue::from_str(value.trim_start()).context("invalid header value")?;
        request = request.header(name, value);
    }
    let action = format!("api {method} {}", args.path);
    let response = request.send().await.with_context(|| action.clone())?;
    let status = response.status();
    let body = response.bytes().await.context("reading API response")?;
    if !body.is_empty() {
        match serde_json::from_slice::<Value>(&body) {
            Ok(value) if status.is_success() => print_json(&value)?,
            Ok(value) => print_json_unselected(&value)?,
            Err(_) => std::io::stdout().lock().write_all(&body)?,
        }
    }
    if !status.is_success() {
        let message = serde_json::from_slice::<super::Problem>(&body)
            .ok()
            .and_then(|problem| problem.detail.or(problem.title))
            .or_else(|| {
                let text = String::from_utf8_lossy(&body);
                let text = text.trim();
                (!text.is_empty()).then(|| text.to_owned())
            });
        return Err(super::describe(&action, status, message));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::api_path;

    #[test]
    fn strips_only_the_v1_path_segment() {
        for (input, expected) in [
            ("/pricing/models", "/pricing/models"),
            ("/v1/pricing/models", "/pricing/models"),
            ("/v1", "/"),
            ("/v10/models", "/v10/models"),
            ("/v1?key=value", "?key=value"),
        ] {
            assert_eq!(api_path(input).expect("valid path"), expected);
        }
    }

    #[test]
    fn rejects_urls_and_relative_paths() {
        for path in ["https://example.com/me", "http://example.com/me", "me"] {
            assert!(api_path(path).is_err());
        }
    }
}
