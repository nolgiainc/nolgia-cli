use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

const JOB_ID: &str = "11111111-1111-4111-8111-111111111111";
const USER_ID: &str = "22222222-2222-4222-8222-222222222222";
const ASSET_ID: &str = "66666666-6666-4666-8666-666666666666";
const SIGNED_URL: &str = "https://files.example/video.mp4?signature=example";

fn cmd(api: &MockServer) -> Command {
    // Isolate credentials and state; no subprocess may probe the real keychain.
    static ISOLATED_CONFIG: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let isolated_config =
        ISOLATED_CONFIG.get_or_init(|| tempfile::tempdir().expect("isolated config dir"));
    let mut command = Command::cargo_bin("nolgia").expect("nolgia binary");
    for name in [
        "NOLGIA_TOKEN",
        "HERMES_HOME",
        "HERMES_DASHBOARD",
        "NOLGIA_SURFACE",
        "NOLGIA_ORG",
    ] {
        command.env_remove(name);
    }
    command
        .env("NOLGIA_TOKEN_STORE", "file")
        .env("XDG_CONFIG_HOME", isolated_config.path())
        .env("XDG_STATE_HOME", isolated_config.path())
        .env("NOLGIA_NO_UPDATE_CHECK", "1")
        .args(["--api-url", &api.uri()]);
    command
}

async fn respond(api: &MockServer, verb: &str, route: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(api)
        .await;
}

fn job_json() -> Value {
    json!({
        "id": JOB_ID, "user_id": USER_ID, "modality": "video", "model": "video-model",
        "status": "succeeded", "created_at": "2026-06-13T00:00:00Z",
        "updated_at": "2026-06-13T00:00:00Z", "asset": {
            "id": ASSET_ID, "user_id": USER_ID, "modality": "video", "model": "video-model",
            "signed_url": SIGNED_URL, "expires_at": "2026-06-14T00:00:00Z",
            "created_at": "2026-06-13T00:00:00Z"
        }
    })
}

fn document(assertion: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assertion.get_output().stdout).expect("JSON stdout")
}

#[tokio::test]
async fn status_and_jobs_get_select_signed_url_before_or_after_subcommand() {
    let api = MockServer::start().await;
    respond(&api, "GET", &format!("/v1/jobs/{JOB_ID}"), 200, job_json()).await;
    for args in [
        vec!["status", JOB_ID, "--field", "asset.signed_url"],
        vec!["--field", "asset.signed_url", "status", JOB_ID],
        vec!["jobs", "get", JOB_ID, "--field", "asset.signed_url"],
    ] {
        cmd(&api)
            .args(args)
            .assert()
            .success()
            .stdout(format!("{SIGNED_URL}\n"));
    }
}

#[tokio::test]
async fn status_selects_repeated_fields_in_order_and_accepts_leading_dot() {
    let api = MockServer::start().await;
    respond(&api, "GET", &format!("/v1/jobs/{JOB_ID}"), 200, job_json()).await;
    cmd(&api)
        .args(["status", JOB_ID, "--field", "status", "--field", ".id"])
        .assert()
        .success()
        .stdout(format!("succeeded\n{JOB_ID}\n"));
}

#[tokio::test]
async fn repeated_fields_keep_command_line_order_across_subcommand_levels() {
    let api = MockServer::start().await;
    cmd(&api)
        .args([
            "--field",
            "[0].name",
            "skills",
            "--field=[1].name",
            "list",
            "--field",
            "[2].name",
        ])
        .assert()
        .success()
        .stdout("nolgia-platform\nnolgia-video-prompting\nnolgia-ugc-ads\n");
}

#[tokio::test]
async fn jobs_list_selects_array_index_and_nested_key() {
    let api = MockServer::start().await;
    respond(
        &api,
        "GET",
        "/v1/jobs",
        200,
        json!({"items": [job_json()], "total": 1}),
    )
    .await;
    for (field, expected) in [
        ("items[0].id", JOB_ID),
        ("items[0].asset.signed_url", SIGNED_URL),
    ] {
        cmd(&api)
            .args(["jobs", "list", "--field", field])
            .assert()
            .success()
            .stdout(format!("{expected}\n"));
    }
}

#[tokio::test]
async fn missing_field_reports_parent_keys_without_partial_stdout() {
    let api = MockServer::start().await;
    respond(&api, "GET", &format!("/v1/jobs/{JOB_ID}"), 200, job_json()).await;
    cmd(&api)
        .args(["status", JOB_ID, "--field", "id", "--field", "asset.url"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains(
            "Error: field \"asset.url\" not found",
        ))
        .stderr(predicate::str::contains("available at \"asset\""))
        .stderr(predicate::str::contains("id"))
        .stderr(predicate::str::contains("signed_url"));
}

#[tokio::test]
async fn field_index_past_end_and_scalar_descent_fail_without_stdout() {
    let api = MockServer::start().await;
    respond(
        &api,
        "GET",
        "/v1/example",
        200,
        json!({"items": [{"id": "one"}]}),
    )
    .await;
    for (field, reason) in [
        ("items[1].id", "past the end"),
        ("items[0].id.name", "scalar"),
    ] {
        cmd(&api)
            .args(["api", "GET", "/example", "--field", field])
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicate::str::contains(field))
            .stderr(predicate::str::contains(reason));
    }
}

#[tokio::test]
async fn json_output_selects_scalar_or_array_without_changing_plain_json_bytes() {
    let api = MockServer::start().await;
    let job = job_json();
    respond(&api, "GET", &format!("/v1/jobs/{JOB_ID}"), 200, job.clone()).await;
    cmd(&api)
        .args(["status", JOB_ID, "--output", "json", "--field", "status"])
        .assert()
        .success()
        .stdout("\"succeeded\"\n");
    cmd(&api)
        .args([
            "status", JOB_ID, "--output", "json", "--field", "status", "--field", "id",
        ])
        .assert()
        .success()
        .stdout(format!("[\n  \"succeeded\",\n  \"{JOB_ID}\"\n]\n"));
    let typed: nolgia_client::types::Job = serde_json::from_value(job).expect("typed job fixture");
    cmd(&api)
        .args(["status", JOB_ID, "--json"])
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            serde_json::to_string_pretty(&typed).expect("serialize job")
        ));
}

#[tokio::test]
async fn value_output_renders_whole_object_as_one_compact_line() {
    let api = MockServer::start().await;
    let body = json!({"ok": true, "items": [1, 2]});
    respond(&api, "GET", "/v1/example", 200, body.clone()).await;
    cmd(&api)
        .args(["api", "GET", "/example", "--output", "value"])
        .assert()
        .success()
        .stdout(format!("{body}\n"));
}

#[tokio::test]
async fn value_output_handles_top_level_array_and_each_json_value_kind() {
    let api = MockServer::start().await;
    respond(
        &api,
        "GET",
        "/v1/example",
        200,
        json!([{"name": "first"}, 7, true, null, [1, 2]]),
    )
    .await;
    cmd(&api)
        .args([
            "api", "GET", "/example", "--field", "[0].name", "--field", "[1]", "--field", "[2]",
            "--field", "[3]", "--field", "[4]",
        ])
        .assert()
        .success()
        .stdout("first\n7\ntrue\nnull\n[1,2]\n");
}

#[tokio::test]
async fn table_output_lists_one_row_per_job_with_key_headers() {
    let api = MockServer::start().await;
    let mut second = job_json();
    second["id"] = json!("33333333-3333-4333-8333-333333333333");
    second["status"] = json!("queued");
    respond(
        &api,
        "GET",
        "/v1/jobs",
        200,
        json!({"items": [job_json(), second], "total": 2}),
    )
    .await;
    let result = cmd(&api)
        .args(["jobs", "list", "--output", "table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&result.get_output().stdout);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 3, "one header and two jobs: {text}");
    assert!(lines[0].split_whitespace().any(|key| key == "id"));
    assert!(lines[0].split_whitespace().any(|key| key == "status"));
    assert!(lines[1].contains(JOB_ID) && lines[1].contains("succeeded"));
    assert!(
        lines[2].contains("33333333-3333-4333-8333-333333333333") && lines[2].contains("queued")
    );
}

#[tokio::test]
async fn table_output_for_account_me_has_key_value_rows() {
    let api = MockServer::start().await;
    respond(
        &api,
        "GET",
        "/v1/me",
        200,
        json!({
            "id": USER_ID, "email": "ada@example.com", "name": "Ada", "image_url": null,
            "created_at": "2026-06-13T00:00:00Z"
        }),
    )
    .await;
    let result = cmd(&api)
        .args(["account", "me", "--output", "table"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&result.get_output().stdout);
    assert_eq!(
        text.lines()
            .next()
            .expect("table header")
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["KEY", "VALUE"]
    );
    assert!(text.lines().any(|line| line.split_whitespace().collect::<Vec<_>>() == ["email", "ada@example.com"]));
}

#[tokio::test]
async fn api_get_models_is_unauthenticated_and_normalizes_one_v1_prefix() {
    for route in ["/pricing/models", "/v1/pricing/models"] {
        let api = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/pricing/models"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"models": [{"id": "flux-pro"}]})),
            )
            .expect(1)
            .mount(&api)
            .await;
        cmd(&api)
            .args(["api", "get", route, "--field", "models[0].id"])
            .assert()
            .success()
            .stdout("flux-pro\n");
        let requests = api.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }
}

#[tokio::test]
async fn api_inherits_bearer_surface_and_idempotency_headers() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", "Bearer tok"))
        .and(header("x-nolgia-surface", "cli"))
        .and(header("idempotency-key", "ticket-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"email": "ada@example.com"})))
        .expect(1)
        .mount(&api)
        .await;
    cmd(&api)
        .env("NOLGIA_SURFACE", "cli")
        .args([
            "--token",
            "tok",
            "--idempotency-key",
            "ticket-key",
            "api",
            "GET",
            "/me",
            "--field",
            "email",
        ])
        .assert()
        .success()
        .stdout("ada@example.com\n");
}

#[tokio::test]
async fn api_write_methods_without_body_send_explicit_zero_content_length() {
    for verb in ["POST", "PUT", "PATCH"] {
        let api = MockServer::start().await;
        let route = format!("/jobs/{JOB_ID}/sse-ticket");
        Mock::given(method(verb))
            .and(path(format!("/v1{route}")))
            .and(header("content-length", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ticket": "one-ticket"})))
            .expect(1)
            .mount(&api)
            .await;
        cmd(&api)
            .args(["api", verb, &route, "--field", "ticket"])
            .assert()
            .success()
            .stdout("one-ticket\n");
    }
}

#[tokio::test]
async fn api_body_supports_inline_file_and_stdin_json() {
    for source in ["inline", "file", "stdin"] {
        let api = MockServer::start().await;
        let body = r#"{"model":"flux-pro","prompt":"x"}"#;
        Mock::given(method("POST"))
            .and(path("/v1/generate/image"))
            .and(body_json(json!({"model": "flux-pro", "prompt": "x"})))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({"id": JOB_ID})))
            .expect(1)
            .mount(&api)
            .await;
        let file = tempfile::NamedTempFile::new().expect("body file");
        std::fs::write(file.path(), body).expect("write body");
        let file_argument = format!("@{}", file.path().display());
        let argument = match source {
            "file" => file_argument.as_str(),
            "stdin" => "-",
            _ => body,
        };
        let mut command = cmd(&api);
        command.args([
            "api",
            "POST",
            "/generate/image",
            "--body",
            argument,
            "--field",
            "id",
        ]);
        if source == "stdin" {
            command.write_stdin(body);
        }
        command.assert().success().stdout(format!("{JOB_ID}\n"));
    }
}

#[tokio::test]
async fn api_invalid_json_is_rejected_before_request() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    cmd(&api)
        .args(["api", "POST", "/generate/image", "--body", "{broken"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("JSON"));
    assert!(api.received_requests().await.expect("requests").is_empty());
}

#[tokio::test]
async fn api_repeated_query_parameters_and_custom_headers_are_forwarded() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .and(query_param("limit", "1"))
        .and(query_param("search", "a & b=two"))
        .and(header("x-test", "forwarded"))
        .and(header("accept", "application/problem+json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&api)
        .await;
    cmd(&api)
        .args([
            "api",
            "GET",
            "/jobs",
            "--query",
            "limit=1",
            "--query",
            "search=a & b=two",
            "--header",
            "X-Test: forwarded",
            "--header",
            "Accept: application/problem+json",
        ])
        .assert()
        .success()
        .stdout("[]\n");
}

#[tokio::test]
async fn api_problem_response_prints_whole_body_and_error_without_field_selection() {
    let api = MockServer::start().await;
    let route = format!("/jobs/{JOB_ID}");
    let body = json!({"type": "about:blank", "title": "Not Found", "status": 404, "detail": "job is missing"});
    respond(&api, "GET", &format!("/v1{route}"), 404, body.clone()).await;
    let result = cmd(&api)
        .args([
            "api", "GET", &route, "--field", "missing", "--output", "table",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(format!(
            "Error: api GET {route}: 404 Not Found: job is missing"
        )));
    assert_eq!(document(&result), body);
}

#[tokio::test]
async fn api_refuses_absolute_urls_before_sending_token() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&api)
        .await;
    for absolute in [
        format!("{}/v1/me", api.uri()),
        "https://example.com/me".to_owned(),
    ] {
        cmd(&api)
            .args(["--token", "tok", "api", "GET", &absolute])
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicate::str::contains(
                "Error: pass an API path such as /jobs/<id>; the server comes from --api-url",
            ));
    }
    assert!(api.received_requests().await.expect("requests").is_empty());
}

#[tokio::test]
async fn api_agent_org_mutations_match_existing_refusal_and_ignore_selection() {
    let api = MockServer::start().await;
    for (verb, route, subcommand) in [
        ("PUT", "/me/active-organization", "switch"),
        ("POST", "/v1/organizations", "create"),
    ] {
        let result = cmd(&api)
            .env("HERMES_HOME", "pod-home")
            .env("HERMES_DASHBOARD", "pod-dashboard")
            .args([
                "--token", "nol_test", "api", verb, route, "--field", "missing", "--output",
                "table",
            ])
            .assert()
            .code(77);
        let refusal = document(&result);
        let expected = cmd(&api)
            .env("HERMES_HOME", "pod-home")
            .env("HERMES_DASHBOARD", "pod-dashboard")
            .args(["--token", "nol_test", "org", subcommand, "Acme", "--json"])
            .assert()
            .code(77);
        assert_eq!(refusal, document(&expected));
        assert_eq!(refusal["error"], "agent_refused");
    }
    assert!(api.received_requests().await.expect("requests").is_empty());
}

#[tokio::test]
async fn api_agent_other_routes_pass_through() {
    let api = MockServer::start().await;
    respond(&api, "GET", "/v1/organizations", 200, json!([])).await;
    cmd(&api)
        .env("HERMES_HOME", "pod-home")
        .env("HERMES_DASHBOARD", "pod-dashboard")
        .args(["api", "GET", "/organizations"])
        .assert()
        .success()
        .stdout("[]\n");
}

#[tokio::test]
async fn api_non_json_body_is_byte_identical_and_empty_response_has_no_output() {
    let api = MockServer::start().await;
    let bytes = b"raw\0body\nwithout final newline\xff";
    Mock::given(method("GET"))
        .and(path("/v1/raw"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
        .expect(1)
        .mount(&api)
        .await;
    cmd(&api)
        .args(["api", "GET", "/raw", "--field", "missing"])
        .assert()
        .success()
        .stdout(bytes.as_slice());
    for verb in ["DELETE", "HEAD"] {
        Mock::given(method(verb))
            .and(path("/v1/empty"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&api)
            .await;
        cmd(&api)
            .args(["api", verb, "/empty", "--field", "missing"])
            .assert()
            .success()
            .stdout("");
    }
}

#[tokio::test]
async fn live_job_report_ignores_field_and_table_selection() {
    let api = MockServer::start().await;
    respond(&api, "GET", &format!("/v1/jobs/{JOB_ID}/wait"), 408,
        json!({"type": "about:blank", "title": "Request Timeout", "status": 408, "detail": "wait timed out"})).await;
    let result = cmd(&api)
        .args(["wait", JOB_ID, "--field", "missing", "--output", "table"])
        .assert()
        .code(75)
        .stderr(predicate::str::contains("field").not());
    let body = document(&result);
    assert_eq!(body["job_id"], JOB_ID);
    assert_eq!(body["outcome"], "still_running");
    assert_eq!(body["billed_twice"], false);
}

#[tokio::test]
async fn moderated_report_ignores_field_and_table_selection() {
    let api = MockServer::start().await;
    let mut queued = job_json();
    queued["status"] = json!("queued");
    queued["asset"] = Value::Null;
    let mut failed = queued.clone();
    failed["status"] = json!("failed");
    failed["failure"] = json!({"kind": "moderated", "message": "Provider blocked reference media", "credits_refunded": true});
    respond(&api, "POST", "/v1/generate/image", 202, queued).await;
    respond(&api, "GET", &format!("/v1/jobs/{JOB_ID}/wait"), 200, failed).await;
    let result = cmd(&api)
        .args([
            "gen", "image", "--prompt", "a cat", "--field", "missing", "--output", "table",
        ])
        .assert()
        .code(65)
        .stderr(predicate::str::contains("field").not());
    let body = document(&result);
    assert_eq!(body["id"], JOB_ID);
    assert_eq!(body["status"], "failed");
    assert_eq!(body["failure"]["kind"], "moderated");
}
