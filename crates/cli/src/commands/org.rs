//! `nolgia org` (alias `workspace`): the organization context a user works in.
//!
//! The active organization is SERVER state (`users.active_organization_id`,
//! switched with `PUT /me/active-organization`), so switching here moves the
//! web app, MCP and every credential the user holds at once. Nothing in this
//! module sends a per-request override header: the API has none yet, so the
//! `--org` / `NOLGIA_ORG` selector only picks which organization the read-only
//! and admin subcommands (`members`, `invite`, `credits`) address, without
//! touching the server-side context.

use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand, ValueEnum};
use nolgia_client::types::{
    ActiveOrganizationRequest, CreateOrganizationInviteRequest, CreateOrganizationRequest,
    CreateOrganizationRequestSlug, OrganizationInvite, OrganizationMembership, OrganizationRole,
    Subscription, User, UserOrganization,
};
use reqwest::StatusCode;
use serde::Serialize;
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

const CREDIT_POOL_NOTE: &str = "In an organization context every generation, whether authenticated by \
     device login or a personal access token, spends the organization's shared credit pool. \
     In the personal space a PAT still draws only from your prepaid API top-up pool.";

#[derive(Subcommand, Debug)]
pub enum OrgCommand {
    #[command(
        about = "List the organizations you belong to, with your role and the active one marked"
    )]
    List,
    #[command(
        about = "Show the organization you are working in (or the personal space) and the effective plan",
        long_about = "Show the organization you are working in, or the personal space, plus the \
                      effective subscription plan for that context. In an organization context the \
                      plan is the organization's (with seats and seat limit when present).\n\n\
                      In an organization context every generation, whether authenticated by device \
                      login or a personal access token, spends the organization's shared credit pool. \
                      In the personal space a PAT still draws only from your prepaid API top-up pool."
    )]
    Status,
    #[command(
        about = "Switch the active context to an organization (by slug or id) or back to `personal`",
        long_about = "Switch the active context to an organization, by slug or id, or back to your \
                      personal space with `personal`. This is server-side state: the web app, MCP \
                      and every PAT you hold follow it on their next request. Refuses with a clear \
                      message when you are not a member of the named organization.\n\n\
                      In an organization context every generation, whether authenticated by device \
                      login or a personal access token, spends the organization's shared credit pool. \
                      In the personal space a PAT still draws only from your prepaid API top-up pool."
    )]
    Switch(SwitchArgs),
    #[command(about = "Create a team organization you own; it becomes your active organization")]
    Create(CreateArgs),
    #[command(about = "List the members of the active organization (or --org / NOLGIA_ORG)")]
    Members(TargetArgs),
    #[command(
        about = "Invite an email address to the organization; the accept link is printed once",
        long_about = "Invite an email address to the active organization (or --org / NOLGIA_ORG) \
                      with a role. Owner or admin only. The API emails the link when mail is \
                      configured and returns it once here; the CLI prints that URL exactly once and \
                      never writes the invite token anywhere else. Copy it now if you need it."
    )]
    Invite(InviteArgs),
    #[command(about = "Show the organization's shared credit pool and per-member spend this month")]
    Credits(TargetArgs),
}

#[derive(Args, Debug)]
pub struct SwitchArgs {
    /// Organization slug or UUID, or `personal` for your personal space
    #[arg(value_name = "SLUG|ID|personal")]
    pub target: String,
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Display name (1 to 120 characters)
    pub name: String,
    /// URL handle (lowercase letters, digits, hyphens); derived from the name when omitted
    #[arg(long)]
    pub slug: Option<String>,
}

#[derive(Args, Debug)]
pub struct TargetArgs {
    /// Organization slug or UUID to address instead of the active one. Does not
    /// switch the server-side context; use `nolgia org switch` for that.
    #[arg(
        long,
        env = "NOLGIA_ORG",
        hide_env_values = true,
        value_name = "SLUG|ID"
    )]
    pub org: Option<String>,
}

#[derive(Args, Debug)]
pub struct InviteArgs {
    /// Email address to invite
    pub email: String,
    /// Role the invitee joins with (owners are made by ownership transfer, not invite)
    #[arg(long, value_enum)]
    pub role: InviteRole,
    #[command(flatten)]
    pub target: TargetArgs,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum InviteRole {
    /// Members, invites, domains, SSO, storage, budgets, any library item, audit log
    Admin,
    /// Seats and invoices; reads members
    Billing,
    /// Create, generate, own items; reads the whole library (consumes a seat)
    Member,
    /// Read-only library; does not consume a seat
    Viewer,
}

impl From<InviteRole> for OrganizationRole {
    fn from(role: InviteRole) -> Self {
        match role {
            InviteRole::Admin => OrganizationRole::Admin,
            InviteRole::Billing => OrganizationRole::Billing,
            InviteRole::Member => OrganizationRole::Member,
            InviteRole::Viewer => OrganizationRole::Viewer,
        }
    }
}

pub async fn run(command: OrgCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        OrgCommand::List => list(ctx).await,
        OrgCommand::Status => status(ctx).await,
        OrgCommand::Switch(args) => switch(args, ctx).await,
        OrgCommand::Create(args) => create(args, ctx).await,
        OrgCommand::Members(args) => members(args, ctx).await,
        OrgCommand::Invite(args) => invite(args, ctx).await,
        OrgCommand::Credits(args) => credits(args, ctx).await,
    }
}

// ---------------------------------------------------------------------------
// Context resolution
// ---------------------------------------------------------------------------

async fn current_user(ctx: &CommandContext) -> Result<User> {
    match ctx.client().get_current_user().send().await {
        Ok(response) => Ok(response.into_inner()),
        Err(err) => Err(super::api_error(err, "fetching current user").await),
    }
}

/// Match a `slug|id` selector against the organizations the user belongs to.
fn find_membership<'a>(user: &'a User, selector: &str) -> Option<&'a UserOrganization> {
    let wanted_id = Uuid::parse_str(selector).ok();
    user.organizations.iter().find(|org| {
        wanted_id.is_some_and(|id| id == org.id) || org.slug.eq_ignore_ascii_case(selector)
    })
}

fn not_a_member(user: &User, selector: &str) -> anyhow::Error {
    if user.organizations.is_empty() {
        return anyhow!(
            "you are not a member of any organization matching \"{selector}\" \
             (you belong to none yet; `nolgia org create <name>` starts one)"
        );
    }
    let slugs: Vec<&str> = user.organizations.iter().map(|o| o.slug.as_str()).collect();
    anyhow!(
        "you are not a member of an organization matching \"{selector}\"; \
         your organizations: {} (`nolgia org list`)",
        slugs.join(", ")
    )
}

/// The organization a `members|invite|credits` call addresses: the explicit
/// `--org` / `NOLGIA_ORG` selector when given, else the active organization.
/// Never changes the server-side context.
async fn resolve_target(ctx: &CommandContext, selector: Option<&str>) -> Result<UserOrganization> {
    let user = current_user(ctx).await?;
    match selector.map(str::trim).filter(|s| !s.is_empty()) {
        Some(selector) => find_membership(&user, selector)
            .cloned()
            .ok_or_else(|| not_a_member(&user, selector)),
        None => user.active_organization.ok_or_else(|| {
            anyhow!(
                "you are in your personal space, which has no members or shared pool; \
                 run `nolgia org switch <slug>` or pass --org <slug|id> (or set NOLGIA_ORG)"
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OrgListEntry {
    #[serde(flatten)]
    membership: OrganizationMembership,
    active: bool,
}

#[derive(Serialize)]
struct OrgList {
    items: Vec<OrgListEntry>,
    active_organization_id: Option<Uuid>,
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let page = match ctx.client().list_organizations().send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "listing organizations").await),
    };
    let user = current_user(ctx).await?;
    let active_id = user.active_organization.as_ref().map(|org| org.id);
    let list = OrgList {
        items: page
            .items
            .into_iter()
            .map(|membership| OrgListEntry {
                active: Some(membership.organization.id) == active_id,
                membership,
            })
            .collect(),
        active_organization_id: active_id,
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&list),
        OutputFormat::Text => {
            if list.items.is_empty() {
                println!("no organizations; you are in your personal space");
                println!("`nolgia org create <name>` starts a team organization you own");
                return Ok(());
            }
            let rows: Vec<[String; 4]> = list
                .items
                .iter()
                .map(|entry| {
                    let org = &entry.membership.organization;
                    [
                        org.slug.to_string(),
                        org.name.to_string(),
                        org.kind.clone(),
                        entry.membership.role.clone(),
                    ]
                })
                .collect();
            let widths = column_widths(&rows);
            for (entry, row) in list.items.iter().zip(&rows) {
                let marker = if entry.active { '*' } else { ' ' };
                println!(
                    "{marker} {}  {}  {}  {}",
                    pad(&row[0], widths[0]),
                    pad(&row[1], widths[1]),
                    pad(&row[2], widths[2]),
                    row[3]
                );
            }
            match active_id {
                Some(_) => println!("* = active organization"),
                None => println!("active: personal space (none of the above)"),
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

enum Plan {
    Known(Subscription),
    /// The API answered `404`: this context has no subscription at all (a
    /// freshly created team organization before billing is attached, or a
    /// personal account that never subscribed).
    None,
    /// The subscription read failed for some other reason; the context is
    /// still reported, the plan just is not.
    Unknown,
}

async fn fetch_plan(ctx: &CommandContext) -> Plan {
    match ctx.client().get_subscription().send().await {
        Ok(response) => Plan::Known(response.into_inner()),
        Err(nolgia_client::ApiError::UnexpectedResponse(response))
            if response.status() == StatusCode::NOT_FOUND =>
        {
            Plan::None
        }
        Err(_) => Plan::Unknown,
    }
}

fn describe_plan(plan: &Plan) -> String {
    match plan {
        Plan::Known(sub) => {
            let mut line = format!("{} {}", sub.tier, sub.status);
            if let Some(seats) = sub.seats {
                line.push_str(&format!(" ({seats} seats"));
                match sub.seat_limit {
                    Some(limit) => line.push_str(&format!(", limit {limit})")),
                    None => line.push_str(", no seat limit)"),
                }
            }
            line
        }
        Plan::None => "none (no subscription in this context)".to_string(),
        Plan::Unknown => "unknown (subscription could not be read)".to_string(),
    }
}

#[derive(Serialize)]
struct ContextReport<'a> {
    context: &'static str,
    organization: Option<&'a UserOrganization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subscription: Option<Option<&'a Subscription>>,
}

fn context_name(org: Option<&UserOrganization>) -> &'static str {
    if org.is_some() {
        "organization"
    } else {
        "personal"
    }
}

fn describe_context(org: Option<&UserOrganization>) -> String {
    match org {
        Some(org) => format!("{} ({}, {}) as {}", org.name, org.slug, org.kind, org.role),
        None => "Personal space".to_string(),
    }
}

async fn status(ctx: &CommandContext) -> Result<()> {
    let user = current_user(ctx).await?;
    let plan = fetch_plan(ctx).await;
    let org = user.active_organization.as_ref();
    match ctx.format() {
        OutputFormat::Json => {
            let subscription = match &plan {
                Plan::Known(sub) => Some(sub),
                Plan::None | Plan::Unknown => None,
            };
            print_json(&ContextReport {
                context: context_name(org),
                organization: org,
                subscription: Some(subscription),
            })
        }
        OutputFormat::Text => {
            println!("Organization: {}", describe_context(org));
            println!("Plan: {}", describe_plan(&plan));
            if org.is_some() {
                println!(
                    "Generations in this context spend the organization's shared credit pool."
                );
            } else {
                println!(
                    "Generations in this context spend your personal credits \
                     (device login: subscription pool; PAT: API top-up pool)."
                );
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// switch
// ---------------------------------------------------------------------------

async fn switch(args: SwitchArgs, ctx: &CommandContext) -> Result<()> {
    let target = args.target.trim();
    let user = current_user(ctx).await?;
    let organization_id = if target.eq_ignore_ascii_case("personal") {
        None
    } else {
        let org = find_membership(&user, target).ok_or_else(|| not_a_member(&user, target))?;
        Some(org.id)
    };

    let body: ActiveOrganizationRequest = ActiveOrganizationRequest::builder()
        .organization_id(organization_id)
        .try_into()
        .context("building active-organization request")?;
    let updated = match ctx
        .client()
        .put_active_organization()
        .body(body)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "switching active organization").await),
    };
    let org = updated.active_organization.as_ref();
    match ctx.format() {
        OutputFormat::Json => print_json(&ContextReport {
            context: context_name(org),
            organization: org,
            subscription: None,
        }),
        OutputFormat::Text => {
            match org {
                Some(org) => {
                    println!(
                        "switched to {} ({}, {}) as {}",
                        org.name, org.slug, org.kind, org.role
                    );
                    println!("{CREDIT_POOL_NOTE}");
                }
                None => {
                    println!("switched to your personal space");
                    println!(
                        "Generations now spend your personal credits \
                         (device login: subscription pool; PAT: API top-up pool)."
                    );
                }
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

async fn create(args: CreateArgs, ctx: &CommandContext) -> Result<()> {
    let slug = args
        .slug
        .as_deref()
        .map(CreateOrganizationRequestSlug::try_from)
        .transpose()
        .map_err(|_| {
            anyhow!(
                "invalid --slug: use 1 to 64 lowercase letters, digits or hyphens, \
                 starting and ending with a letter or digit"
            )
        })?;
    let body: CreateOrganizationRequest = CreateOrganizationRequest::builder()
        .name(args.name)
        .slug(slug)
        .try_into()
        .context("building create-organization request")?;
    let created = match ctx.client().create_organization().body(body).send().await {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "creating organization").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&created),
        OutputFormat::Text => {
            let org = &created.organization;
            println!(
                "created {} ({}, {}) {} as {}",
                org.name.as_str(),
                org.slug.as_str(),
                org.kind,
                org.id,
                created.role
            );
            println!("it is now your active organization; `nolgia org switch personal` returns");
            println!(
                "team billing (seats) is attached in the web app; until then the organization \
                 has no subscription"
            );
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// members
// ---------------------------------------------------------------------------

async fn members(args: TargetArgs, ctx: &CommandContext) -> Result<()> {
    let org = resolve_target(ctx, args.org.as_deref()).await?;
    let page = match ctx
        .client()
        .list_organization_members()
        .id(org.id)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "listing organization members").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&page),
        OutputFormat::Text => {
            println!("organization: {} ({})", org.name, org.slug);
            let rows: Vec<[String; 6]> = page
                .items
                .iter()
                .map(|m| {
                    [
                        m.user_id.to_string(),
                        if m.email.is_empty() {
                            "-".to_string()
                        } else {
                            m.email.clone()
                        },
                        m.name.clone().unwrap_or_else(|| "-".to_string()),
                        m.role.clone(),
                        budget_text(m.monthly_credit_budget),
                        m.joined_at.to_rfc3339(),
                    ]
                })
                .collect();
            let widths = column_widths(&rows);
            for row in &rows {
                println!(
                    "{}  {}  {}  {}  budget {}  joined {}",
                    pad(&row[0], widths[0]),
                    pad(&row[1], widths[1]),
                    pad(&row[2], widths[2]),
                    pad(&row[3], widths[3]),
                    pad(&row[4], widths[4]),
                    row[5]
                );
            }
            println!("{} member(s)", rows.len());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// invite
// ---------------------------------------------------------------------------

/// What `invite` prints. The API also returns the plaintext `token` once; it
/// is deliberately NOT a field here, in either output mode, so the only place
/// it can appear is inside `invite_url`.
#[derive(Serialize)]
struct InviteReport {
    invite: OrganizationInvite,
    invite_url: String,
}

async fn invite(args: InviteArgs, ctx: &CommandContext) -> Result<()> {
    let org = resolve_target(ctx, args.target.org.as_deref()).await?;
    let body: CreateOrganizationInviteRequest = CreateOrganizationInviteRequest::builder()
        .email(args.email)
        .role(OrganizationRole::from(args.role))
        .try_into()
        .context("building invite request")?;
    let created = match ctx
        .client()
        .create_organization_invite()
        .id(org.id)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "creating organization invite").await),
    };
    let report = InviteReport {
        invite: created.invite,
        invite_url: created.invite_url,
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&report),
        OutputFormat::Text => {
            println!(
                "invited {} to {} ({}) as {}; expires {}",
                report.invite.email,
                org.name,
                org.slug,
                report.invite.role,
                report.invite.expires_at.to_rfc3339()
            );
            println!("accept link: {}", report.invite_url);
            println!("warning: this link is shown once and cannot be retrieved again");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// credits
// ---------------------------------------------------------------------------

async fn credits(args: TargetArgs, ctx: &CommandContext) -> Result<()> {
    let org = resolve_target(ctx, args.org.as_deref()).await?;
    let credits = match ctx
        .client()
        .get_organization_credits()
        .id(org.id)
        .send()
        .await
    {
        Ok(response) => response.into_inner(),
        Err(err) => return Err(super::api_error(err, "fetching organization credits").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(&credits),
        OutputFormat::Text => {
            println!("organization: {} ({})", org.name, org.slug);
            println!(
                "pool: subscription {}  api top-ups {}  total {}",
                credits.balance.app_subscription,
                credits.balance.shared_topup,
                credits.balance.total
            );
            let period = match (credits.period_start, credits.period_end) {
                (Some(start), Some(end)) => {
                    format!("{} to {}", start.to_rfc3339(), end.to_rfc3339())
                }
                _ => "no subscription".to_string(),
            };
            let seats = match (credits.seats, credits.seat_limit) {
                (Some(seats), Some(limit)) => format!("  seats: {seats} (limit {limit})"),
                (Some(seats), None) => format!("  seats: {seats} (no limit)"),
                (None, Some(limit)) => format!("  seat limit: {limit}"),
                (None, None) => String::new(),
            };
            println!("period: {period}{seats}");
            if credits.members.is_empty() {
                println!("members: none visible");
                return Ok(());
            }
            println!("members (spend this UTC month):");
            let rows: Vec<[String; 6]> = credits
                .members
                .iter()
                .map(|m| {
                    [
                        m.user_id.to_string(),
                        if m.email.is_empty() {
                            "-".to_string()
                        } else {
                            m.email.clone()
                        },
                        m.role.clone(),
                        m.spent_this_month.to_string(),
                        budget_text(m.monthly_credit_budget),
                        m.remaining
                            .map(|r| r.to_string())
                            .unwrap_or_else(|| "unlimited".to_string()),
                    ]
                })
                .collect();
            let widths = column_widths(&rows);
            for row in &rows {
                println!(
                    "{}  {}  {}  spent {}  budget {}  remaining {}",
                    pad(&row[0], widths[0]),
                    pad(&row[1], widths[1]),
                    pad(&row[2], widths[2]),
                    pad(&row[3], widths[3]),
                    pad(&row[4], widths[4]),
                    row[5]
                );
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn budget_text(budget: Option<i64>) -> String {
    budget
        .map(|b| b.to_string())
        .unwrap_or_else(|| "unlimited".to_string())
}

fn column_widths<const N: usize>(rows: &[[String; N]]) -> [usize; N] {
    let mut widths = [0usize; N];
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    widths
}

fn pad(cell: &str, width: usize) -> String {
    let len = cell.chars().count();
    let mut out = String::with_capacity(width);
    out.push_str(cell);
    out.extend(std::iter::repeat_n(' ', width.saturating_sub(len)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_with(orgs: &[(&str, &str)], active: Option<&str>) -> User {
        let organizations: Vec<UserOrganization> = orgs
            .iter()
            .map(|(id, slug)| UserOrganization {
                id: Uuid::parse_str(id).unwrap(),
                kind: "team".into(),
                name: slug.to_uppercase(),
                role: "member".into(),
                slug: slug.to_string(),
            })
            .collect();
        let active_organization =
            active.and_then(|slug| organizations.iter().find(|org| org.slug == slug).cloned());
        User {
            active_organization,
            created_at: chrono::Utc::now(),
            email: "ada@nolgia.ai".into(),
            id: Uuid::nil(),
            image_url: None,
            name: None,
            organizations,
        }
    }

    const ACME: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const BETA: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    #[test]
    fn selector_matches_slug_case_insensitively_and_id() {
        let user = user_with(&[(ACME, "acme"), (BETA, "beta")], None);
        assert_eq!(find_membership(&user, "ACME").unwrap().slug, "acme");
        assert_eq!(find_membership(&user, BETA).unwrap().slug, "beta");
        assert!(find_membership(&user, "gamma").is_none());
        // A UUID that is not one of ours must not fall through to a slug match.
        assert!(find_membership(&user, &Uuid::nil().to_string()).is_none());
    }

    #[test]
    fn not_a_member_lists_the_users_organizations() {
        let user = user_with(&[(ACME, "acme"), (BETA, "beta")], None);
        let msg = not_a_member(&user, "gamma").to_string();
        assert!(msg.contains("\"gamma\""), "{msg}");
        assert!(msg.contains("acme, beta"), "{msg}");
        assert!(msg.contains("nolgia org list"), "{msg}");

        let none = user_with(&[], None);
        let msg = not_a_member(&none, "gamma").to_string();
        assert!(msg.contains("nolgia org create"), "{msg}");
    }

    #[test]
    fn plan_text_carries_seats_when_present() {
        let sub: Subscription = serde_json::from_value(serde_json::json!({
            "tier": "team", "status": "active", "current_period_end": "2026-10-01T00:00:00Z",
            "scope": "organization", "seats": 3, "seat_limit": 5
        }))
        .unwrap();
        assert_eq!(
            describe_plan(&Plan::Known(sub)),
            "team active (3 seats, limit 5)"
        );
        let personal: Subscription = serde_json::from_value(serde_json::json!({
            "tier": "pro", "status": "active", "current_period_end": "2026-10-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(describe_plan(&Plan::Known(personal)), "pro active");
        assert!(describe_plan(&Plan::None).starts_with("none"));
        assert!(describe_plan(&Plan::Unknown).starts_with("unknown"));
    }

    #[test]
    fn user_facing_copy_has_no_em_dashes() {
        assert!(!CREDIT_POOL_NOTE.contains('\u{2014}'));
        let user = user_with(&[(ACME, "acme")], Some("acme"));
        assert!(!describe_context(user.active_organization.as_ref()).contains('\u{2014}'));
        assert!(!not_a_member(&user, "x").to_string().contains('\u{2014}'));
    }

    #[test]
    fn padding_aligns_columns() {
        let rows = vec![
            ["a".to_string(), "bbb".to_string()],
            ["cc".to_string(), "d".to_string()],
        ];
        assert_eq!(column_widths(&rows), [2, 3]);
        assert_eq!(pad("a", 3), "a  ");
        assert_eq!(pad("abcd", 2), "abcd");
    }
}
