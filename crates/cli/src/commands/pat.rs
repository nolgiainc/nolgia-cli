use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum PatCommand {
    Create(CreatePatArgs),
    List,
    Revoke(RevokePatArgs),
}

#[derive(Args, Debug)]
pub struct CreatePatArgs {
    #[arg(long)]
    pub name: String,
    /// Days until the token stops working, 1 to 365. Default: 365. Every new
    /// token expires; create its replacement before then.
    #[arg(long, value_name = "DAYS", value_parser = clap::value_parser!(u16).range(1..=365))]
    pub expires_in_days: Option<u16>,
}

#[derive(Args, Debug)]
pub struct RevokePatArgs {
    pub pat_id: Uuid,
}

pub async fn run(command: PatCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        PatCommand::Create(args) => create(args, ctx).await,
        PatCommand::List => list(ctx).await,
        PatCommand::Revoke(args) => revoke(args, ctx).await,
    }
}

async fn create(args: CreatePatArgs, ctx: &CommandContext) -> Result<()> {
    let body: nolgia_client::types::CreatePatRequest =
        nolgia_client::types::CreatePatRequest::builder()
            .name(args.name)
            .expires_in_days(
                args.expires_in_days
                    .and_then(|days| std::num::NonZeroU64::new(u64::from(days))),
            )
            .try_into()
            .context("building create-pat request")?;
    let created = ctx
        .client()
        .create_pat()
        .body(body)
        .send()
        .await
        .context("creating personal access token")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &created),
        OutputFormat::Text => {
            println!("created {} ({})", created.pat.id, created.pat.name.as_str());
            println!("{}", expiry_text(created.pat.expires_at, Utc::now()));
            println!("token: {}", created.token);
            println!("warning: this token will not be shown again; store it securely");
            Ok(())
        }
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let page = ctx
        .client()
        .list_pats()
        .send()
        .await
        .context("listing personal access tokens")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &page),
        OutputFormat::Text => {
            let now = Utc::now();
            for pat in page.items {
                let last_used = pat
                    .last_used_at
                    .map(|at| at.to_rfc3339())
                    .unwrap_or_else(|| "never".to_string());
                println!(
                    "{} {} {} created {} last used {} {}",
                    pat.id,
                    pat.name.as_str(),
                    pat.prefix.as_str(),
                    pat.created_at.to_rfc3339(),
                    last_used,
                    expiry_text(pat.expires_at, now)
                );
            }
            Ok(())
        }
    }
}

async fn revoke(args: RevokePatArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .revoke_pat()
        .id(args.pat_id)
        .send()
        .await
        .context("revoking personal access token")?;
    match ctx.format() {
        OutputFormat::Json => {
            print_json(ctx.output(), &serde_json::json!({ "revoked": args.pat_id }))
        }
        OutputFormat::Text => {
            println!("revoked {}", args.pat_id);
            Ok(())
        }
    }
}

/// How a token's expiry reads in text output (NOL-1213). `null` is a token
/// created before tokens expired: it keeps working, and replacing it is
/// recommended.
fn expiry_text(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    match expires_at {
        None => "expires never (created before tokens expired; rotate recommended)".to_string(),
        Some(at) if at <= now => format!(
            "EXPIRED {} (revoke it and create a new one)",
            at.to_rfc3339()
        ),
        Some(at) => format!("expires {}", at.to_rfc3339()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn expiry_text_reads_each_state() {
        let now = Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0).unwrap();
        assert_eq!(
            expiry_text(None, now),
            "expires never (created before tokens expired; rotate recommended)"
        );
        let later = Utc.with_ymd_and_hms(2027, 9, 25, 12, 0, 0).unwrap();
        assert_eq!(
            expiry_text(Some(later), now),
            "expires 2027-09-25T12:00:00+00:00"
        );
        assert_eq!(
            expiry_text(Some(now), now),
            "EXPIRED 2026-09-25T12:00:00+00:00 (revoke it and create a new one)"
        );
    }
}
