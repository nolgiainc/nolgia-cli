//! `nolgia org` / `nolgia workspace` and the organization line of
//! `nolgia auth status`, exercised offline against wiremock fixtures shaped
//! like the vendored spec's `User`, `OrganizationMembership`,
//! `OrganizationMember`, `CreateOrganizationInviteResponse` and
//! `OrganizationCredits` schemas.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

const USER_ID: &str = "22222222-2222-4222-8222-222222222222";
const ACME_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const BETA_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const MEMBER_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const INVITE_ID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const INVITE_TOKEN: &str = "inv_s3cr3tT0kenThatMustAppearOnlyInsideTheUrl";

// ---------------------------------------------------------------------------
// Help surface
// ---------------------------------------------------------------------------

#[test]
fn top_level_help_lists_org_and_its_workspace_alias() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("org"))
        .stdout(predicate::str::contains("workspace"));
}

#[test]
fn org_help_lists_every_subcommand_under_both_names() {
    for group in ["org", "workspace"] {
        cmd()
            .args([group, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("list"))
            .stdout(predicate::str::contains("status"))
            .stdout(predicate::str::contains("switch"))
            .stdout(predicate::str::contains("create"))
            .stdout(predicate::str::contains("members"))
            .stdout(predicate::str::contains("invite"))
            .stdout(predicate::str::contains("credits"));
    }
}

/// The one sentence the lane asked for: in an organization context a
/// generation spends the organization's pool, for JWTs and PATs alike, while
/// a personal PAT keeps drawing only from the top-up pool.
#[test]
fn status_and_switch_help_state_the_credit_pool_rule() {
    for sub in ["status", "switch"] {
        let assert = cmd().args(["org", sub, "--help"]).assert().success();
        let help = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 help");
        assert!(
            help.contains("shared credit pool"),
            "`org {sub} --help` must state the organization credit pool rule:\n{help}"
        );
        assert!(
            help.contains("personal access token") && help.contains("top-up"),
            "`org {sub} --help` must cover PATs and the personal top-up pool:\n{help}"
        );
        assert!(
            !help.contains('\u{2014}'),
            "`org {sub} --help` contains an em dash:\n{help}"
        );
    }
}

/// NOL-317 rule applied to the new env-backed selector: `--help` names
/// `NOLGIA_ORG` and never renders what it holds.
#[test]
fn org_help_never_renders_nolgia_org_value() {
    const SENTINEL: &str = "sentinel-org-value-must-not-appear";
    for sub in ["members", "invite", "credits"] {
        let assert = cmd()
            .env("NOLGIA_ORG", SENTINEL)
            .args(["org", sub, "--help"])
            .assert()
            .success();
        let help = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 help");
        assert!(help.contains("NOLGIA_ORG"), "`org {sub} --help`:\n{help}");
        assert!(!help.contains(SENTINEL), "`org {sub} --help`:\n{help}");
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn org_list_marks_the_active_organization_and_shows_roles() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_organizations(&api).await;
    run_ok(&api, &["org", "list"])
        .stdout(predicate::str::contains("* acme-studios"))
        .stdout(predicate::str::contains("Acme Studios"))
        .stdout(predicate::str::contains("owner"))
        .stdout(predicate::str::contains("  beta-films"))
        .stdout(predicate::str::contains("member"))
        .stdout(predicate::str::contains("* = active organization"));
}

#[tokio::test]
async fn org_list_in_personal_space_says_so() {
    let api = MockServer::start().await;
    mount_me(&api, None).await;
    mount_organizations(&api).await;
    run_ok(&api, &["org", "list"])
        .stdout(predicate::str::contains("active: personal space"))
        .stdout(predicate::str::contains("* acme").not());
}

#[tokio::test]
async fn json_org_list_carries_an_active_flag_per_membership() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_organizations(&api).await;
    let doc = json_stdout(&api, &["--json", "org", "list"]);
    assert_eq!(doc["active_organization_id"], ACME_ID);
    let items = doc["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["organization"]["slug"], "acme-studios");
    assert_eq!(items[0]["organization"]["seat_limit"], 5);
    assert_eq!(items[0]["role"], "owner");
    assert_eq!(items[0]["active"], true);
    assert_eq!(items[1]["organization"]["slug"], "beta-films");
    assert_eq!(items[1]["active"], false);
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn org_status_in_an_organization_shows_plan_with_seats() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_subscription(&api, team_subscription_json()).await;
    run_ok(&api, &["org", "status"])
        .stdout(predicate::str::contains(
            "Organization: Acme Studios (acme-studios, team) as owner",
        ))
        .stdout(predicate::str::contains(
            "Plan: team active (3 seats, limit 5)",
        ))
        .stdout(predicate::str::contains(
            "spend the organization's shared credit pool",
        ));
}

#[tokio::test]
async fn org_status_in_the_personal_space_shows_personal_plan() {
    let api = MockServer::start().await;
    mount_me(&api, None).await;
    mount_subscription(&api, pro_subscription_json()).await;
    run_ok(&api, &["org", "status"])
        .stdout(predicate::str::contains("Organization: Personal space"))
        .stdout(predicate::str::contains("Plan: pro active"))
        .stdout(predicate::str::contains("seats").not())
        .stdout(predicate::str::contains("personal credits"));
}

/// A freshly created team organization has no subscription until billing is
/// attached in the web app; the API answers `404` and the CLI must say
/// "none", not fail and not claim a plan.
#[tokio::test]
async fn org_status_without_a_subscription_reports_none() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    Mock::given(method("GET"))
        .and(path("/v1/billing/subscription"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "type": "about:blank", "title": "Not Found", "status": 404,
            "detail": "no subscription"
        })))
        .mount(&api)
        .await;
    run_ok(&api, &["org", "status"])
        .stdout(predicate::str::contains("Organization: Acme Studios"))
        .stdout(predicate::str::contains("Plan: none"));
}

#[tokio::test]
async fn json_org_status_reports_context_organization_and_subscription() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_subscription(&api, team_subscription_json()).await;
    let doc = json_stdout(&api, &["--json", "org", "status"]);
    assert_eq!(doc["context"], "organization");
    assert_eq!(doc["organization"]["id"], ACME_ID);
    assert_eq!(doc["organization"]["slug"], "acme-studios");
    assert_eq!(doc["organization"]["role"], "owner");
    assert_eq!(doc["subscription"]["tier"], "team");
    assert_eq!(doc["subscription"]["seats"], 3);
    assert_eq!(doc["subscription"]["seat_limit"], 5);

    let api = MockServer::start().await;
    mount_me(&api, None).await;
    mount_subscription(&api, pro_subscription_json()).await;
    let doc = json_stdout(&api, &["--json", "org", "status"]);
    assert_eq!(doc["context"], "personal");
    assert!(doc["organization"].is_null());
    assert_eq!(doc["subscription"]["tier"], "pro");
}

// ---------------------------------------------------------------------------
// switch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn org_switch_by_slug_puts_the_resolved_organization_id() {
    let api = MockServer::start().await;
    mount_me(&api, None).await;
    Mock::given(method("PUT"))
        .and(path("/v1/me/active-organization"))
        .and(body_json(json!({ "organization_id": ACME_ID })))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_json(Some("acme-studios"))))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["org", "switch", "acme-studios"])
        .stdout(predicate::str::contains(
            "switched to Acme Studios (acme-studios, team) as owner",
        ))
        .stdout(predicate::str::contains("shared credit pool"));
}

#[tokio::test]
async fn org_switch_accepts_an_organization_id() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    Mock::given(method("PUT"))
        .and(path("/v1/me/active-organization"))
        .and(body_json(json!({ "organization_id": BETA_ID })))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_json(Some("beta-films"))))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["workspace", "switch", BETA_ID]).stdout(predicate::str::contains(
        "switched to Beta Films (beta-films, team) as member",
    ));
}

/// `personal` must reach the wire as an explicit `null`, not an omitted key:
/// the request schema requires `organization_id`.
#[tokio::test]
async fn org_switch_personal_sends_an_explicit_null() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    Mock::given(method("PUT"))
        .and(path("/v1/me/active-organization"))
        .and(body_json(json!({ "organization_id": null })))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_json(None)))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["org", "switch", "personal"])
        .stdout(predicate::str::contains("switched to your personal space"))
        .stdout(predicate::str::contains("personal credits"));
}

/// Not a member: refuse locally with the organizations the user does belong
/// to, and never send the PUT (the server would 404 it, but a clear message
/// beats a problem dump, and a non-member switch must not even be attempted).
#[tokio::test]
async fn org_switch_refuses_a_non_member_slug_before_any_put() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "switch", "gamma-pictures"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a member"))
        .stderr(predicate::str::contains("\"gamma-pictures\""))
        .stderr(predicate::str::contains("acme-studios, beta-films"))
        .stderr(predicate::str::contains("nolgia org list"));
    assert_no_request(&api, "PUT", "/v1/me/active-organization").await;
}

#[tokio::test]
async fn json_org_switch_reports_the_new_context() {
    let api = MockServer::start().await;
    mount_me(&api, None).await;
    Mock::given(method("PUT"))
        .and(path("/v1/me/active-organization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_json(Some("acme-studios"))))
        .mount(&api)
        .await;
    let doc = json_stdout(&api, &["--json", "org", "switch", "acme-studios"]);
    assert_eq!(doc["context"], "organization");
    assert_eq!(doc["organization"]["slug"], "acme-studios");
    assert!(
        doc.get("subscription").is_none(),
        "switch does not read billing"
    );
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

#[tokio::test]
async fn org_create_sends_name_and_slug_and_prints_the_new_organization() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/organizations"))
        .and(body_json(
            json!({ "name": "Acme Studios", "slug": "acme-studios" }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(membership_json(
            ACME_ID,
            "Acme Studios",
            "acme-studios",
            "owner",
        )))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["org", "create", "Acme Studios", "--slug", "acme-studios"],
    )
    .stdout(predicate::str::contains(format!(
        "created Acme Studios (acme-studios, team) {ACME_ID} as owner"
    )))
    .stdout(predicate::str::contains("now your active organization"));
}

/// Without `--slug` the key must be absent so the server derives it; an
/// explicit `null` would be a schema violation.
#[tokio::test]
async fn org_create_without_slug_omits_the_key() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/organizations"))
        .and(body_json(json!({ "name": "Acme Studios" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(membership_json(
            ACME_ID,
            "Acme Studios",
            "acme-studios",
            "owner",
        )))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["org", "create", "Acme Studios"])
        .stdout(predicate::str::contains("acme-studios"));
}

#[tokio::test]
async fn org_create_rejects_a_malformed_slug_before_any_request() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "create", "Acme Studios", "--slug", "Acme Studios!"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --slug"));
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn json_org_create_emits_the_membership() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/organizations"))
        .respond_with(ResponseTemplate::new(201).set_body_json(membership_json(
            ACME_ID,
            "Acme Studios",
            "acme-studios",
            "owner",
        )))
        .mount(&api)
        .await;
    let doc = json_stdout(&api, &["--json", "org", "create", "Acme Studios"]);
    assert_eq!(doc["organization"]["id"], ACME_ID);
    assert_eq!(doc["organization"]["kind"], "team");
    assert_eq!(doc["role"], "owner");
}

// ---------------------------------------------------------------------------
// members
// ---------------------------------------------------------------------------

#[tokio::test]
async fn org_members_lists_the_active_organization() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_members(&api, ACME_ID).await;
    run_ok(&api, &["org", "members"])
        .stdout(predicate::str::contains(
            "organization: Acme Studios (acme-studios)",
        ))
        .stdout(predicate::str::contains(USER_ID))
        .stdout(predicate::str::contains("ada@nolgia.ai"))
        .stdout(predicate::str::contains("owner"))
        .stdout(predicate::str::contains("budget unlimited"))
        .stdout(predicate::str::contains(MEMBER_ID))
        .stdout(predicate::str::contains("bob@nolgia.ai"))
        .stdout(predicate::str::contains("budget 500"))
        .stdout(predicate::str::contains("joined 2026-06-14T00:00:00+00:00"))
        .stdout(predicate::str::contains("2 member(s)"));
}

/// `NOLGIA_ORG` picks the organization the read addresses and nothing else:
/// no PUT to the active-organization endpoint, no request for the active
/// organization's members.
#[tokio::test]
async fn org_members_honors_nolgia_org_without_switching_context() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_members(&api, BETA_ID).await;
    cmd()
        .env("NOLGIA_ORG", "beta-films")
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "members"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "organization: Beta Films (beta-films)",
        ));
    assert_no_request(&api, "PUT", "/v1/me/active-organization").await;
    assert_no_request(&api, "GET", &format!("/v1/organizations/{ACME_ID}/members")).await;
}

#[tokio::test]
async fn org_members_org_flag_accepts_an_id_and_beats_the_env() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_members(&api, BETA_ID).await;
    cmd()
        .env("NOLGIA_ORG", "acme-studios")
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "members", "--org", BETA_ID])
        .assert()
        .success()
        .stdout(predicate::str::contains("Beta Films"));
    assert_no_request(&api, "GET", &format!("/v1/organizations/{ACME_ID}/members")).await;
}

#[tokio::test]
async fn org_members_in_the_personal_space_needs_a_selector() {
    let api = MockServer::start().await;
    mount_me(&api, None).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "members"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("personal space"))
        .stderr(predicate::str::contains("--org"))
        .stderr(predicate::str::contains("NOLGIA_ORG"));
}

#[tokio::test]
async fn org_members_refuses_an_organization_the_user_is_not_in() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "members", "--org", "gamma-pictures"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a member"))
        .stderr(predicate::str::contains("acme-studios, beta-films"));
}

#[tokio::test]
async fn json_org_members_emits_the_member_page() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_members(&api, ACME_ID).await;
    let doc = json_stdout(&api, &["--json", "org", "members"]);
    let items = doc["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["user_id"], MEMBER_ID);
    assert_eq!(items[1]["role"], "member");
    assert_eq!(items[1]["monthly_credit_budget"], 500);
}

// ---------------------------------------------------------------------------
// invite
// ---------------------------------------------------------------------------

/// The accept link is printed exactly once; the plaintext token the API also
/// returns must not surface anywhere else, in either stream.
#[tokio::test]
async fn org_invite_prints_the_accept_url_once_and_the_token_nowhere_else() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/organizations/{ACME_ID}/invites")))
        .and(body_json(
            json!({ "email": "carol@example.com", "role": "member" }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(invite_response_json()))
        .expect(1)
        .mount(&api)
        .await;
    let output = cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "invite", "carol@example.com", "--role", "member"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stdout.contains(&format!(
            "accept link: https://nolgia.ai/invite/{INVITE_TOKEN}"
        )),
        "{stdout}"
    );
    assert!(stdout.contains("invited carol@example.com to Acme Studios (acme-studios) as member"));
    assert!(stdout.contains("shown once"), "{stdout}");
    assert_eq!(
        stdout.matches(INVITE_TOKEN).count(),
        1,
        "the token may appear only inside the accept URL:\n{stdout}"
    );
    assert!(!stderr.contains(INVITE_TOKEN), "{stderr}");
}

#[tokio::test]
async fn json_org_invite_has_no_raw_token_field() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/organizations/{ACME_ID}/invites")))
        .and(body_json(
            json!({ "email": "carol@example.com", "role": "admin" }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(invite_response_json()))
        .mount(&api)
        .await;
    let raw = cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "--json",
            "org",
            "invite",
            "carol@example.com",
            "--role",
            "admin",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(raw).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).expect("one JSON document");
    assert_eq!(doc["invite"]["id"], INVITE_ID);
    assert_eq!(doc["invite"]["email"], "carol@example.com");
    assert_eq!(
        doc["invite_url"],
        format!("https://nolgia.ai/invite/{INVITE_TOKEN}")
    );
    assert!(doc.get("token").is_none(), "{text}");
    assert_eq!(text.matches(INVITE_TOKEN).count(), 1, "{text}");
}

#[tokio::test]
async fn org_invite_honors_org_selector_and_requires_a_role() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/organizations/{BETA_ID}/invites")))
        .and(body_json(
            json!({ "email": "carol@example.com", "role": "viewer" }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(invite_response_json()))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "org",
            "invite",
            "carol@example.com",
            "--role",
            "viewer",
            "--org",
            "beta-films",
        ],
    )
    .stdout(predicate::str::contains("Beta Films"));

    cmd()
        .args(["org", "invite", "carol@example.com"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--role"));
}

/// Ownership is transferred, never invited; the parser refuses `owner`.
#[test]
fn org_invite_rejects_the_owner_role() {
    cmd()
        .args([
            "org",
            "invite",
            "carol@example.com",
            "--role",
            "owner",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'owner'"));
}

// ---------------------------------------------------------------------------
// credits
// ---------------------------------------------------------------------------

#[tokio::test]
async fn org_credits_shows_the_pool_period_seats_and_member_spend() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_credits(&api, ACME_ID).await;
    run_ok(&api, &["org", "credits"])
        .stdout(predicate::str::contains(
            "organization: Acme Studios (acme-studios)",
        ))
        .stdout(predicate::str::contains(
            "pool: subscription 1200  api top-ups 300  total 1500",
        ))
        .stdout(predicate::str::contains(
            "period: 2026-09-01T00:00:00+00:00 to 2026-10-01T00:00:00+00:00  seats: 3 (limit 5)",
        ))
        .stdout(predicate::str::contains("bob@nolgia.ai"))
        .stdout(predicate::str::contains("spent 120"))
        .stdout(predicate::str::contains("budget 500"))
        .stdout(predicate::str::contains("remaining 380"))
        .stdout(predicate::str::contains("remaining unlimited"));
}

#[tokio::test]
async fn org_credits_honors_nolgia_org() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_credits(&api, BETA_ID).await;
    cmd()
        .env("NOLGIA_ORG", BETA_ID)
        .arg("--api-url")
        .arg(api.uri())
        .args(["org", "credits"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "organization: Beta Films (beta-films)",
        ));
    assert_no_request(&api, "PUT", "/v1/me/active-organization").await;
}

#[tokio::test]
async fn json_org_credits_emits_the_raw_document() {
    let api = MockServer::start().await;
    mount_me(&api, Some("acme-studios")).await;
    mount_credits(&api, ACME_ID).await;
    let doc = json_stdout(&api, &["--json", "org", "credits"]);
    assert_eq!(doc["organization_id"], ACME_ID);
    assert_eq!(doc["balance"]["total"], 1500);
    assert_eq!(doc["seats"], 3);
    assert_eq!(doc["seat_limit"], 5);
    assert_eq!(doc["members"][1]["spent_this_month"], 120);
    assert_eq!(doc["members"][1]["remaining"], 380);
    assert_eq!(doc["buckets"].as_array().map(Vec::len), Some(2));
}

// ---------------------------------------------------------------------------
// auth status: the Organization line
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_status_with_token_shows_the_active_organization() {
    let api = MockServer::start().await;
    mount_me_for_token(&api, "tok-org", Some("acme-studios")).await;
    mount_subscription_for_token(&api, "tok-org", team_subscription_json()).await;
    run_ok(&api, &["--token", "tok-org", "auth", "status"])
        .stdout(predicate::str::contains("ada@nolgia.ai (team)"))
        .stdout(predicate::str::contains(
            "Organization: Acme Studios (acme-studios) as owner",
        ));
}

#[tokio::test]
async fn auth_status_with_token_in_personal_space_says_so() {
    let api = MockServer::start().await;
    mount_me_for_token(&api, "tok-personal", None).await;
    mount_subscription_for_token(&api, "tok-personal", pro_subscription_json()).await;
    run_ok(&api, &["--token", "tok-personal", "auth", "whoami"])
        .stdout(predicate::str::contains("ada@nolgia.ai (pro)"))
        .stdout(predicate::str::contains("Organization: Personal space"));
}

/// The stored-login path (`AuthManager::status`) renders the same line.
#[tokio::test]
async fn auth_status_from_the_file_store_shows_the_organization_line() {
    let api = MockServer::start().await;
    mount_me_for_token(&api, "file-token", Some("beta-films")).await;
    mount_subscription_for_token(&api, "file-token", team_subscription_json()).await;
    let home = tempfile::tempdir().unwrap();
    write_token_file(home.path(), "file-token");
    cmd()
        .env("XDG_CONFIG_HOME", home.path())
        .arg("--api-url")
        .arg(api.uri())
        .args(["auth", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ada@nolgia.ai (team)"))
        .stdout(predicate::str::contains(
            "Organization: Beta Films (beta-films) as member",
        ));
}

/// `--json auth status` is one parseable document carrying `organization`
/// (an object in an organization, `null` in the personal space).
#[tokio::test]
async fn json_auth_status_is_one_document_with_the_organization() {
    let api = MockServer::start().await;
    mount_me_for_token(&api, "tok-json", Some("acme-studios")).await;
    mount_subscription_for_token(&api, "tok-json", team_subscription_json()).await;
    let doc = json_stdout(&api, &["--json", "--token", "tok-json", "auth", "status"]);
    assert_eq!(doc["email"], "ada@nolgia.ai");
    assert_eq!(doc["tier"], "team");
    assert_eq!(doc["organization"]["slug"], "acme-studios");
    assert_eq!(doc["organization"]["role"], "owner");

    let api = MockServer::start().await;
    mount_me_for_token(&api, "tok-json", None).await;
    mount_subscription_for_token(&api, "tok-json", pro_subscription_json()).await;
    let doc = json_stdout(&api, &["--json", "--token", "tok-json", "auth", "status"]);
    assert_eq!(doc["tier"], "pro");
    assert!(doc["organization"].is_null(), "{doc}");
}

// ---------------------------------------------------------------------------
// Helpers and fixtures
// ---------------------------------------------------------------------------

fn cmd() -> Command {
    // Same isolation as cli_commands.rs: force the file token store and point
    // every config/state path at a per-test-process temp dir so a spawned
    // binary can never touch the operator's credentials or keychain.
    static ISOLATED_HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let home = ISOLATED_HOME.get_or_init(|| tempfile::tempdir().expect("isolated config dir"));
    let mut command = Command::cargo_bin("nolgia").unwrap();
    command.env_remove("NOLGIA_TOKEN");
    command.env_remove("NOLGIA_ORG");
    command.env("NOLGIA_TOKEN_STORE", "file");
    command.env("XDG_CONFIG_HOME", home.path());
    command.env("XDG_STATE_HOME", home.path());
    command.env("NOLGIA_NO_UPDATE_CHECK", "1");
    command
}

fn run_ok(api: &MockServer, args: &[&str]) -> assert_cmd::assert::Assert {
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(args)
        .assert()
        .success()
}

/// Run and parse stdout as exactly one JSON document.
fn json_stdout(api: &MockServer, args: &[&str]) -> serde_json::Value {
    let raw = run_ok(api, args).get_output().stdout.clone();
    serde_json::from_slice(&raw).unwrap_or_else(|err| {
        panic!(
            "--json stdout must be one parseable document ({err}):\n{}",
            String::from_utf8_lossy(&raw)
        )
    })
}

async fn assert_no_request(api: &MockServer, verb: &str, url_path: &str) {
    let hits: Vec<_> = api
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == url_path)
        .collect();
    assert!(hits.is_empty(), "unexpected {verb} {url_path}: {hits:?}");
}

fn write_token_file(config_home: &std::path::Path, access_token: &str) {
    let dir = config_home.join("nolgia");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("tokens.json"),
        json!({
            "access_token": access_token,
            "refresh_token": null,
            "expires_at": "2030-01-01T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
}

fn user_org_json(id: &str, name: &str, slug: &str, role: &str) -> serde_json::Value {
    json!({ "id": id, "name": name, "slug": slug, "kind": "team", "role": role })
}

/// `GET /me` as the spec's `User`: two memberships, `active_organization`
/// present only in an organization context (the field is omitted in the
/// personal space, exactly as the server does).
fn me_json(active_slug: Option<&str>) -> serde_json::Value {
    let organizations = vec![
        user_org_json(ACME_ID, "Acme Studios", "acme-studios", "owner"),
        user_org_json(BETA_ID, "Beta Films", "beta-films", "member"),
    ];
    let mut user = json!({
        "id": USER_ID, "email": "ada@nolgia.ai", "name": "Ada", "image_url": null,
        "created_at": "2026-06-13T00:00:00Z",
        "organizations": organizations,
    });
    if let Some(slug) = active_slug {
        let active = user["organizations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["slug"] == slug)
            .cloned()
            .expect("known slug");
        user["active_organization"] = active;
    }
    user
}

async fn mount_me(api: &MockServer, active_slug: Option<&str>) {
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_json(active_slug)))
        .mount(api)
        .await;
}

async fn mount_me_for_token(api: &MockServer, token: &str, active_slug: Option<&str>) {
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", format!("Bearer {token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_json(active_slug)))
        .mount(api)
        .await;
}

fn organization_json(id: &str, name: &str, slug: &str) -> serde_json::Value {
    json!({
        "id": id, "name": name, "slug": slug, "kind": "team", "owner_user_id": USER_ID,
        "seat_limit": 5, "contract_monthly_credits": null, "contract_ends_at": null,
        "settings": {}, "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
    })
}

fn membership_json(id: &str, name: &str, slug: &str, role: &str) -> serde_json::Value {
    json!({ "organization": organization_json(id, name, slug), "role": role })
}

async fn mount_organizations(api: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/organizations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "items": [
            membership_json(ACME_ID, "Acme Studios", "acme-studios", "owner"),
            membership_json(BETA_ID, "Beta Films", "beta-films", "member"),
        ]})))
        .mount(api)
        .await;
}

fn team_subscription_json() -> serde_json::Value {
    json!({
        "tier": "team", "status": "active", "current_period_end": "2026-10-01T00:00:00Z",
        "scope": "organization", "organization_id": ACME_ID, "seats": 3, "seat_limit": 5
    })
}

fn pro_subscription_json() -> serde_json::Value {
    json!({
        "tier": "pro", "status": "active", "current_period_end": "2026-10-01T00:00:00Z",
        "scope": "personal"
    })
}

async fn mount_subscription(api: &MockServer, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v1/billing/subscription"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(api)
        .await;
}

async fn mount_subscription_for_token(api: &MockServer, token: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path("/v1/billing/subscription"))
        .and(header("authorization", format!("Bearer {token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(api)
        .await;
}

async fn mount_members(api: &MockServer, org_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/organizations/{org_id}/members")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "items": [
            {
                "user_id": USER_ID, "email": "ada@nolgia.ai", "name": "Ada", "role": "owner",
                "monthly_credit_budget": null, "invited_by": null,
                "joined_at": "2026-06-13T00:00:00Z"
            },
            {
                "user_id": MEMBER_ID, "email": "bob@nolgia.ai", "name": null, "role": "member",
                "monthly_credit_budget": 500, "invited_by": USER_ID,
                "joined_at": "2026-06-14T00:00:00Z"
            }
        ]})))
        .mount(api)
        .await;
}

fn invite_response_json() -> serde_json::Value {
    json!({
        "invite": {
            "id": INVITE_ID, "organization_id": ACME_ID, "email": "carol@example.com",
            "role": "member", "invited_by": USER_ID,
            "expires_at": "2026-09-10T00:00:00Z", "created_at": "2026-09-03T00:00:00Z",
            "accepted_at": null, "revoked_at": null
        },
        "token": INVITE_TOKEN,
        "invite_url": format!("https://nolgia.ai/invite/{INVITE_TOKEN}")
    })
}

async fn mount_credits(api: &MockServer, org_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/v1/organizations/{org_id}/credits")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "organization_id": org_id,
            "buckets": [
                {"wallet_id": "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee", "type": "app_subscription", "balance": 1200, "expires_at": "2026-10-01T00:00:00Z"},
                {"wallet_id": "ffffffff-ffff-4fff-8fff-ffffffffffff", "type": "shared_topup", "balance": 300, "expires_at": null}
            ],
            "balance": {"app_subscription": 1200, "shared_topup": 300, "total": 1500},
            "period_start": "2026-09-01T00:00:00Z", "period_end": "2026-10-01T00:00:00Z",
            "seats": 3, "seat_limit": 5,
            "members": [
                {"user_id": USER_ID, "email": "ada@nolgia.ai", "name": "Ada", "role": "owner",
                 "monthly_credit_budget": null, "spent_this_month": 40},
                {"user_id": MEMBER_ID, "email": "bob@nolgia.ai", "name": null, "role": "member",
                 "monthly_credit_budget": 500, "spent_this_month": 120, "remaining": 380}
            ]
        })))
        .mount(api)
        .await;
}
