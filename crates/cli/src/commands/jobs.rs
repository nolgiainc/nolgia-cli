use anyhow::Result;
use clap::{Args, Subcommand};
use nolgia_client::ClientExt;
use nolgia_client::types::{JobStatus, Modality};
use std::num::NonZeroU64;
use uuid::Uuid;

use super::CommandContext;
use crate::output::{OutputFormat, print_json};

#[derive(Subcommand, Debug)]
pub enum JobsCommand {
    /// Show current job status
    Get(super::status::StatusArgs),
    /// List your generation jobs
    List(ListArgs),
    /// Cancel a queued or running job on the server and stop it at the model provider
    #[command(
        long_about = "Cancel a queued or running job on the server and stop it at the model provider.\n\n\
The job ends as `canceled` straight away and is never delivered: no asset is added to your library, \
whatever the provider does next. The output says what the provider did and what happened to the \
credits. A job that had not reached the provider is refunded in full. A render the provider had \
already started is refunded only when the provider stops it without billing; until the provider \
answers, the credits read `pending`, and `nolgia jobs get <JOB_ID>` shows how they settled.\n\n\
Canceling cannot be undone. Canceling a job twice prints the same result. A job that already \
finished, or whose result is being delivered, cannot be canceled and is left unchanged.\n\n\
Stopping a wait (Ctrl-C, or `wait`/`gen` timing out) never cancels a job; this command does."
    )]
    Cancel(CancelArgs),
}

#[derive(Args, Debug)]
pub struct CancelArgs {
    /// The job to cancel
    pub job_id: Uuid,
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
        JobsCommand::Cancel(args) => cancel(args, ctx).await,
    }
}

async fn cancel(args: CancelArgs, ctx: &CommandContext) -> Result<()> {
    // ClientExt helper rather than the generated `cancel_job` builder: the
    // spec declares no request body, so the generated call sends no
    // Content-Length and the production load balancer answers `411 Length
    // Required` before the API ever sees it (NOL-542).
    let job = match ctx.client().cancel_job_with_body(args.job_id).await {
        Ok(job) => job,
        Err(err) => return Err(super::cancel_error(err, args.job_id).await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &job),
        OutputFormat::Text => {
            println!("{} {} {}", job.id, job.modality, job.status);
            // The server's sentence, printed as written, then the credits.
            if let Some(details) = crate::canceled::describe(&job) {
                println!("{details}");
            }
            Ok(())
        }
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
        OutputFormat::Json => print_json(ctx.output(), &page),
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
