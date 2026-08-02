use anyhow::Result;
use clap::Args;
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Args, Debug)]
pub struct StatusArgs {
    pub job_id: Uuid,
}

pub async fn run(args: StatusArgs, ctx: &CommandContext) -> Result<()> {
    // The recovery advice printed everywhere else in the CLI is
    // `nolgia status <id>`, so this command's own failures have to be legible
    // too — a 404 here used to render as progenitor's raw response dump.
    let job = match ctx.client().get_job().id(args.job_id).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "fetching job status").await),
    };

    match ctx.format() {
        OutputFormat::Json => print_json(&job),
        OutputFormat::Text => {
            println!("{} {} {}", job.id, job.modality, job.status);
            Ok(())
        }
    }
}
