mod auth;
mod commands;
mod output;

use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::{account, assets, billing, r#gen, status, wait, CommandContext};
use nolgia_client::{Client, ClientBuilder};
use output::OutputFormat;

const DEFAULT_BASE_URL: &str = "https://api.nolgia.ai";

#[derive(Parser, Debug)]
#[command(name = "nolgia", version, about = "Nolgia CLI", propagate_version = true)]
pub struct Cli {
    #[arg(long, global = true, help = "Emit machine-readable JSON")]
    pub json: bool,
    #[arg(long, global = true, env = "NOLGIA_API_URL", default_value = DEFAULT_BASE_URL)]
    pub api_url: String,
    #[arg(long, global = true, env = "NOLGIA_TOKEN")]
    pub token: Option<String>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(subcommand, about = "Authenticate this machine")]
    Auth(auth::AuthCommand),
    #[command(subcommand, about = "Generate images, video, or audio")]
    Gen(r#gen::GenCommand),
    #[command(about = "Show current job status")]
    Status(status::StatusArgs),
    #[command(about = "Wait for a job to finish")]
    Wait(wait::WaitArgs),
    #[command(subcommand, about = "List and manage generated assets")]
    Assets(assets::AssetsCommand),
    #[command(subcommand, about = "Inspect account details and usage")]
    Account(account::AccountCommand),
    #[command(subcommand, about = "Inspect billing state and portal links")]
    Billing(billing::BillingCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_cli(cli).await
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let format = OutputFormat::from_json_flag(cli.json);
    if let Commands::Auth(command) = cli.command {
        return auth::run(command, format, &cli.api_url, cli.token).await;
    }

    let token = cli.token.or_else(auth::load_token).unwrap_or_default();
    let client = build_client(&cli.api_url, token)?;
    let ctx = CommandContext::new(client, format);

    match cli.command {
        Commands::Auth(_) => unreachable!("auth handled before client construction"),
        Commands::Gen(command) => r#gen::run(command, &ctx).await,
        Commands::Status(args) => status::run(args, &ctx).await,
        Commands::Wait(args) => wait::run(args, &ctx).await,
        Commands::Assets(command) => assets::run(command, &ctx).await,
        Commands::Account(command) => account::run(command, &ctx).await,
        Commands::Billing(command) => billing::run(command, &ctx).await,
    }
}

fn build_client(base_url: &str, token: String) -> Result<Client> {
    let builder = ClientBuilder::new(base_url);
    let builder = if token.is_empty() { builder } else { builder.pat(token) };
    Ok(builder.build()?)
}
