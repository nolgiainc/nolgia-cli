use anyhow::{Context, Result};
use clap::Args;
use std::num::NonZeroU64;
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Args, Debug)]
pub struct WaitArgs {
    pub job_id: Uuid,
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
}

pub async fn run(args: WaitArgs, ctx: &CommandContext) -> Result<()> {
    let timeout = NonZeroU64::new(args.timeout).context("--timeout must be greater than zero")?;
    let job = match ctx
        .client()
        .wait_for_job()
        .id(args.job_id)
        .timeout_seconds(timeout)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        // A 408 here is the long-poll expiring, not the job failing. This is
        // the command we tell people to re-run to keep waiting, so it must not
        // greet them with `Error:` for doing exactly that.
        Err(err) => {
            return Err(super::wait_error(err, "waiting for job", args.job_id, args.timeout).await);
        }
    };

    match ctx.format() {
        OutputFormat::Json => print_json(&job),
        OutputFormat::Text => {
            println!("{} {} {}", job.id, job.modality, job.status);
            Ok(())
        }
    }
}
