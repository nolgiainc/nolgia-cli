use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const JOB_ID: &str = "11111111-1111-4111-8111-111111111111";
const USER_ID: &str = "22222222-2222-4222-8222-222222222222";
const PAT_ID: &str = "33333333-3333-4333-8333-333333333333";

#[test]
fn help_lists_full_command_surface() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("gen"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("wait"))
        .stdout(predicate::str::contains("assets"))
        .stdout(predicate::str::contains("account"))
        .stdout(predicate::str::contains("billing"))
        .stdout(predicate::str::contains("pat"));
}

#[test]
fn gen_help_lists_modalities() {
    cmd()
        .args(["gen", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("image"))
        .stdout(predicate::str::contains("video"))
        .stdout(predicate::str::contains("audio"));
}

#[tokio::test]
async fn gen_image_writes_output_file() {
    let api = MockServer::start().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/video.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3]))
        .mount(&files)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(job_json("succeeded", Some(&files.uri()))),
        )
        .mount(&api)
        .await;
    let out = tempfile::tempdir().unwrap().path().join("x.png");
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--prompt",
            "x",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(std::fs::read(out).unwrap(), vec![1, 2, 3]);
}

#[tokio::test]
async fn json_gen_image_no_wait_returns_job_id() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["--json", "gen", "image", "--prompt", "x", "--no-wait"],
    )
    .stdout(predicate::str::contains("job_id"));
}

#[tokio::test]
async fn gen_video_no_wait_returns_job_id() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(&api, &["gen", "video", "--prompt", "x", "--no-wait"])
        .stdout(predicate::str::contains(JOB_ID));
}

#[tokio::test]
async fn gen_video_wait_downloads_asset() {
    let api = MockServer::start().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/video.mp4"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9]))
        .mount(&files)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(job_json("succeeded", Some(&files.uri()))),
        )
        .mount(&api)
        .await;
    let out = tempfile::tempdir().unwrap().path().join("x.mp4");
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--prompt",
            "x",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(std::fs::read(out).unwrap(), vec![9]);
}

#[tokio::test]
async fn gen_audio_prints_asset_url() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/audio"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(job_json("succeeded", Some("https://files"))),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["gen", "audio", "--prompt", "x"]).stdout(predicate::str::contains("video.mp4"));
}

#[tokio::test]
async fn status_fetches_job() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(job_json("running", None)))
        .mount(&api)
        .await;
    run_ok(&api, &["status", JOB_ID]).stdout(predicate::str::contains("running"));
}

#[tokio::test]
async fn wait_fetches_terminal_job() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(200).set_body_json(job_json("succeeded", None)))
        .mount(&api)
        .await;
    run_ok(&api, &["wait", JOB_ID, "--timeout", "1"]).stdout(predicate::str::contains("succeeded"));
}

#[tokio::test]
async fn assets_list_outputs_asset() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/assets"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items": [asset_json("https://files/a.png")]})),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "list"]).stdout(predicate::str::contains("a.png"));
}

#[tokio::test]
async fn assets_get_downloads_asset() {
    let api = MockServer::start().await;
    let files = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/asset.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7, 7]))
        .mount(&files)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{JOB_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(asset_json(&format!("{}/asset.png", files.uri()))),
        )
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("asset.bin");
    run_ok(
        &api,
        &["assets", "get", JOB_ID, "--out", out.to_str().unwrap()],
    );
    assert_eq!(std::fs::read(&out).unwrap(), vec![7, 7]);
}

#[tokio::test]
async fn assets_get_prints_metadata_without_out() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{JOB_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(asset_json("https://files/a.png")))
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "get", JOB_ID]).stdout(predicate::str::contains("a.png"));
}

#[tokio::test]
async fn assets_delete_removes_asset() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/assets/{JOB_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "delete", JOB_ID])
        .stdout(predicate::str::contains(format!("deleted {JOB_ID}")));
}

#[tokio::test]
async fn account_me_outputs_email() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["account", "me"]).stdout(predicate::str::contains("ada@nolgia.ai"));
}

#[tokio::test]
async fn account_usage_combines_jobs_and_assets() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items": [job_json("queued", None)], "total": 1})),
        )
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []})))
        .mount(&api)
        .await;
    run_ok(&api, &["account", "usage"]).stdout(predicate::str::contains("jobs: 1"));
}

#[tokio::test]
async fn billing_subscription_outputs_status() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/billing/subscription"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"tier":"pro","status":"active","current_period_end":"2026-06-13T00:00:00Z"}),
        ))
        .mount(&api)
        .await;
    run_ok(&api, &["billing", "subscription"]).stdout(predicate::str::contains("active"));
}

#[tokio::test]
async fn billing_portal_outputs_url() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/portal-link"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"url":"https://billing.example","expires_at":"2026-06-13T00:00:00Z"}),
        ))
        .mount(&api)
        .await;
    run_ok(&api, &["billing", "portal"]).stdout(predicate::str::contains("billing.example"));
}

#[tokio::test]
async fn billing_credits_shows_both_pools() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/billing/credits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(credit_balance_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["billing", "credits"])
        .stdout(predicate::str::contains(
            "subscription: 546631 (resets with plan)  api top-ups: 250",
        ))
        .stdout(predicate::str::contains("total: 546881"));
}

#[tokio::test]
async fn json_billing_credits_emits_raw_balance() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/billing/credits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(credit_balance_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["--json", "billing", "credits"])
        .stdout(predicate::str::contains("app_subscription"))
        .stdout(predicate::str::contains("shared_topup"))
        .stdout(predicate::str::contains("buckets"));
}

#[tokio::test]
async fn pat_create_prints_token_once_with_warning() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pat"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "pat": pat_json(),
            "token": "nol_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6"
        })))
        .mount(&api)
        .await;
    run_ok(&api, &["pat", "create", "--name", "ci-bot"])
        .stdout(predicate::str::contains(
            "token: nol_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
        ))
        .stdout(predicate::str::contains("will not be shown again"));
}

#[tokio::test]
async fn pat_list_outputs_tokens() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [pat_json()]})))
        .mount(&api)
        .await;
    run_ok(&api, &["pat", "list"])
        .stdout(predicate::str::contains(PAT_ID))
        .stdout(predicate::str::contains("ci-bot"))
        .stdout(predicate::str::contains("nol_a1b2"))
        .stdout(predicate::str::contains("never"));
}

#[tokio::test]
async fn pat_revoke_deletes_token() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/pat/{PAT_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&api)
        .await;
    run_ok(&api, &["pat", "revoke", PAT_ID])
        .stdout(predicate::str::contains(format!("revoked {PAT_ID}")));
}

#[test]
fn auth_help_lists_device_flow_commands() {
    cmd()
        .args(["auth", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("logout"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("whoami"));
}

#[test]
fn invalid_timeout_is_rejected() {
    cmd()
        .args(["wait", JOB_ID, "--timeout", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("timeout"));
}

#[tokio::test]
async fn json_global_flag_is_accepted_before_command() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/assets/{JOB_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&api)
        .await;
    run_ok(&api, &["--json", "assets", "delete", JOB_ID])
        .stdout(predicate::str::contains("deleted"));
}

#[test]
fn image_requires_prompt() {
    cmd()
        .args(["gen", "image"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("prompt"));
}

#[test]
fn video_accepts_input_flag() {
    cmd()
        .args([
            "gen",
            "video",
            "--prompt",
            "x",
            "--input",
            "seed.png",
            "--no-wait",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure();
}

#[test]
fn audio_accepts_format_flag() {
    cmd()
        .args([
            "gen",
            "audio",
            "--prompt",
            "x",
            "--format",
            "wav",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure();
}

#[test]
fn status_requires_uuid() {
    cmd()
        .args(["status", "not-a-uuid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn assets_list_accepts_filters() {
    cmd()
        .args([
            "assets",
            "list",
            "--limit",
            "1",
            "--modality",
            "image",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure();
}

#[test]
fn billing_portal_accepts_return_url() {
    cmd()
        .args([
            "billing",
            "portal",
            "--return-url",
            "https://nolgia.ai",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure();
}

#[test]
fn account_help_lists_subcommands() {
    cmd()
        .args(["account", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("me"))
        .stdout(predicate::str::contains("usage"));
}

fn cmd() -> Command {
    let mut command = Command::cargo_bin("nolgia").unwrap();
    command.env_remove("NOLGIA_TOKEN");
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

fn asset_json(url: &str) -> serde_json::Value {
    json!({
        "id": Uuid::new_v4(), "user_id": USER_ID, "modality": "image", "model": "fal-ai/flux-pro/v1.1",
        "signed_url": url, "expires_at": "2026-06-13T00:00:00Z", "created_at": "2026-06-13T00:00:00Z"
    })
}

fn job_json(status: &str, files_base: Option<&str>) -> serde_json::Value {
    json!({
        "id": JOB_ID, "user_id": USER_ID, "modality": "video", "model": "fal-ai/kling-video/v3/text-to-video",
        "status": status, "asset": files_base.map(|base| asset_json(&format!("{base}/video.mp4"))),
        "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
    })
}

fn credit_balance_json() -> serde_json::Value {
    json!({
        "user_id": USER_ID, "app_subscription": 546631, "shared_topup": 250, "total": 546881,
        "available_for_app": 546881, "available_for_api": 250,
        "buckets": [
            {"wallet_id": Uuid::new_v4(), "type": "app_subscription", "balance": 546631, "expires_at": "2026-08-01T00:00:00Z"},
            {"wallet_id": Uuid::new_v4(), "type": "shared_topup", "balance": 250, "expires_at": null}
        ]
    })
}

fn pat_json() -> serde_json::Value {
    json!({
        "id": PAT_ID, "name": "ci-bot", "prefix": "nol_a1b2",
        "created_at": "2026-06-13T00:00:00Z", "last_used_at": null, "revoked_at": null
    })
}

fn user_json() -> serde_json::Value {
    json!({"id": USER_ID, "email": "ada@nolgia.ai", "name": "Ada", "image_url": null, "created_at": "2026-06-13T00:00:00Z"})
}
