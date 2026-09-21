use anyhow::Result;
use clap::{Args, Subcommand};
use nolgia_client::types::{JobStatus, Modality};
use std::num::NonZeroU64;

use super::CommandContext;
use crate::output::{OutputFormat, print_json};

#[derive(Subcommand, Debug)]
pub enum JobsCommand {
    /// Show current job status
    Get(super::status::StatusArgs),
    /// List your generation jobs
    List(ListArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Only jobs in this state: queued, running, succeeded, failed or canceled
    #[arg(long)]
    pub status: Option<JobStatus>,
    /// Only jobs of this modality: image, video, audio or 3d
    #[arg(long)]
    pub modality: Option<Modality>,
    /// Page size (newest first)
    #[arg(long)]
    pub limit: Option<NonZeroU64>,
    /// Continue from a previous page's `next_cursor`
    #[arg(long)]
    pub cursor: Option<String>,
}

pub async fn run(command: JobsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        JobsCommand::Get(args) => super::status::run(args, ctx).await,
        JobsCommand::List(args) => list(args, ctx).await,
    }
}

async fn list(args: ListArgs, ctx: &CommandContext) -> Result<()> {
    let mut request = ctx.client().list_jobs();
    if let Some(status) = args.status {
        request = request.status(status);
    }
    if let Some(modality) = args.modality {
        request = request.modality(modality);
    }
    if let Some(limit) = args.limit {
        request = request.limit(limit);
    }
    if let Some(cursor) = args.cursor {
        request = request.cursor(cursor);
    }
    let page = match request.send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "listing jobs").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&page),
        OutputFormat::Text => {
            if page.items.is_empty() {
                println!("no jobs");
            }
            for job in page.items {
                println!(
                    "{}  {}  {}  {}  {}",
                    job.id,
                    job.status,
                    job.modality,
                    job.model,
                    job.created_at
                        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
                );
            }
            if let Some(cursor) = page.next_cursor {
                eprint!("more jobs: nolgia jobs list --cursor {cursor}");
                if let Some(status) = args.status {
                    eprint!(" --status {status}");
                }
                if let Some(modality) = args.modality {
                    eprint!(" --modality {modality}");
                }
                if let Some(limit) = args.limit {
                    eprint!(" --limit {limit}");
                }
                eprintln!();
            }
            Ok(())
        }
    }
}
