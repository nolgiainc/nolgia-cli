mod auth;
mod commands;
mod output;
mod update_check;

use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::{
    CommandContext, ability, account, assets, billing, characters, color_presets, r#gen, models,
    pat, projects, skills, status, wait,
};
use nolgia_client::{Client, ClientBuilder};
use output::OutputFormat;

const DEFAULT_BASE_URL: &str = "https://api.nolgia.ai";

#[derive(Parser, Debug)]
#[command(
    name = "nolgia",
    version,
    about = "Nolgia CLI",
    propagate_version = true
)]
pub struct Cli {
    #[arg(long, global = true, help = "Emit machine-readable JSON")]
    pub json: bool,
    // `hide_env_values` is mandatory on every env-backed arg, not just the
    // credential ones: clap's default is to render the *resolved value* of the
    // variable into `--help`, and help output is the least-guarded text in the
    // system (scrollback, CI logs, agent transcripts, screenshots, bug
    // reports). NOL-317 leaked a live PAT this way. Help must show the
    // variable NAME only. Enforced for the whole command tree by
    // `env_backed_args_never_render_their_values` below.
    #[arg(
        long,
        global = true,
        env = "NOLGIA_API_URL",
        hide_env_values = true,
        default_value = DEFAULT_BASE_URL,
        help = "API base URL"
    )]
    pub api_url: String,
    #[arg(
        long,
        global = true,
        env = "NOLGIA_TOKEN",
        hide_env_values = true,
        help = "PAT (nol_...) or JWT to authenticate with, instead of the stored login"
    )]
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
    #[command(subcommand, about = "Manage reusable characters for generation")]
    Characters(characters::CharactersCommand),
    #[command(subcommand, about = "Group assets into projects")]
    Projects(projects::ProjectsCommand),
    #[command(subcommand, about = "Inspect account details and usage")]
    Account(account::AccountCommand),
    #[command(subcommand, about = "Inspect billing state and portal links")]
    Billing(billing::BillingCommand),
    #[command(subcommand, about = "Manage personal access tokens")]
    Pat(pat::PatCommand),
    #[command(subcommand, about = "Bundled AI-agent skills (list, show, install)")]
    Skills(skills::SkillsCommand),
    #[command(
        subcommand,
        about = "Marketplace abilities for your Hermes agent (list, install, sync, init, pack, publish)"
    )]
    Ability(ability::AbilityCommand),
    #[command(subcommand, about = "Live model catalog with capabilities and pricing")]
    Models(models::ModelsCommand),
    #[command(
        subcommand,
        about = "Built-in color-grade preset looks for Studio compositions (list, cube)"
    )]
    ColorPresets(color_presets::ColorPresetsCommand),
    #[command(about = "Generate shell completions (bash, zsh, fish, powershell)")]
    Completion(CompletionArgs),
}

#[derive(clap::Args, Debug)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

/// Detect the calling surface for the X-Nolgia-Surface header. Override
/// with NOLGIA_SURFACE.
fn detect_surface() -> String {
    if let Ok(s) = std::env::var("NOLGIA_SURFACE") {
        return s;
    }
    let has = |k: &str| std::env::var_os(k).is_some();
    if has("CLAUDE_CODE_ENTRYPOINT") || has("CLAUDE_AGENT_SDK_VERSION") || has("CLAUDECODE") {
        return "claude-code".into();
    }
    if has("CODEX_SANDBOX") || has("CODEX_THREAD_ID") {
        return "codex".into();
    }
    if has("HERMES_HOME") && has("HERMES_DASHBOARD") {
        return "hermes".into();
    }
    if has("AI_AGENT") {
        return "agent".into();
    }
    "cli".into()
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let update = update_check::start(cli.json);
    let result = run_cli(cli).await;
    update.finish().await;
    result
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let format = OutputFormat::from_json_flag(cli.json);
    if let Commands::Auth(command) = cli.command {
        return auth::run(command, format, &cli.api_url, cli.token).await;
    }
    if let Commands::Skills(command) = cli.command {
        return skills::run(command, format);
    }
    if let Commands::Completion(args) = cli.command {
        let mut cmd = <Cli as clap::CommandFactory>::command();
        clap_complete::generate(args.shell, &mut cmd, "nolgia", &mut std::io::stdout());
        return Ok(());
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
        Commands::Characters(command) => characters::run(command, &ctx).await,
        Commands::Projects(command) => projects::run(command, &ctx).await,
        Commands::Account(command) => account::run(command, &ctx).await,
        Commands::Billing(command) => billing::run(command, &ctx).await,
        Commands::Pat(command) => pat::run(command, &ctx).await,
        Commands::Skills(_) => unreachable!("skills handled before client construction"),
        Commands::Ability(command) => ability::run(command, &ctx).await,
        Commands::Completion(_) => unreachable!("completion handled before client construction"),
        Commands::Models(command) => models::run(command, &ctx).await,
        Commands::ColorPresets(command) => color_presets::run(command, &ctx).await,
    }
}

fn build_client(base_url: &str, token: String) -> Result<Client> {
    let builder = ClientBuilder::new(base_url).surface(detect_surface());
    let builder = if token.is_empty() {
        builder
    } else {
        builder.pat(token)
    };
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    /// NOL-317: clap renders the *resolved value* of an `env`-backed arg into
    /// `--help` unless `hide_env_values` is set — which is how a live PAT
    /// ended up in `nolgia --help` on the pod. Help must name the variable and
    /// never show what it holds.
    ///
    /// This walks the whole command tree rather than checking `--token` alone,
    /// so the rule binds every env-backed arg added later, anywhere in the
    /// tree, without anyone having to remember it.
    #[test]
    fn env_backed_args_never_render_their_values() {
        fn walk(cmd: &clap::Command, path: &str, offenders: &mut Vec<String>) {
            for arg in cmd.get_arguments() {
                if arg.get_env().is_some() && !arg.is_hide_env_values_set() {
                    offenders.push(format!("`{path}` arg `{}`", arg.get_id()));
                }
            }
            for sub in cmd.get_subcommands() {
                walk(sub, &format!("{path} {}", sub.get_name()), offenders);
            }
        }

        let mut offenders = Vec::new();
        walk(&Cli::command(), "nolgia", &mut offenders);

        assert!(
            offenders.is_empty(),
            "these env-backed args would print their resolved value in --help \
             (leaking whatever the variable holds into scrollback, CI logs and \
             agent transcripts); add `hide_env_values = true` to each: {offenders:?}"
        );
    }
}
