use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const JOB_ID: &str = "11111111-1111-4111-8111-111111111111";

fn cmd(api: &MockServer) -> Command {
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
        .expect(1)
        .mount(api)
        .await;
}

fn document(assertion: &assert_cmd::assert::Assert) -> Value {
    serde_json::from_slice(&assertion.get_output().stdout).expect("one JSON document on stdout")
}

fn assert_live(assertion: &assert_cmd::assert::Assert, outcome: &str) {
    let body = document(assertion);
    assert_eq!(body["job_id"], JOB_ID);
    assert_eq!(body["outcome"], outcome);
    assert_eq!(body["billed_twice"], false);
    let follow_up = body["follow_up"].as_array().expect("recovery commands");
    assert!(follow_up.contains(&json!(format!("nolgia status {JOB_ID}"))));
    assert!(follow_up.contains(&json!(format!("nolgia wait {JOB_ID}"))));
}

#[tokio::test]
async fn encoded_agent_mutations_refuse_before_reading_body_or_sending_request() {
    let api = MockServer::start().await;
    let config = tempfile::tempdir().expect("body directory");
    let missing_body = format!("@{}", config.path().join("missing.json").display());
    for (verb, route, command) in [
        ("PUT", "/me/%61ctive-organization", "org switch"),
        ("PUT", "/v1/%6De/active%2Dorganization?x=1", "org switch"),
        ("PUT", "/me%2factive-organization", "org switch"),
        ("PUT", "/unused/../me/%61ctive-organization", "org switch"),
        (
            "PUT",
            "/unused/%2e%2E/me/%61ctive-organization",
            "org switch",
        ),
        ("POST", "/%6Frganizations", "org create"),
        ("POST", "/v1/organi%7Aations?x=1", "org create"),
        ("POST", "/%6frganizations#ignored", "org create"),
    ] {
        for body in [missing_body.as_str(), "-"] {
            let mut invocation = cmd(&api);
            invocation
                .env("HERMES_HOME", "pod-home")
                .env("HERMES_DASHBOARD", "pod-dashboard")
                .args([
                    "--token",
                    "nol_test",
                    "api",
                    verb,
                    route,
                    "--body",
                    body,
                    "--field",
                    "missing",
                    "--output",
                    "table",
                    "--header",
                    "X-Nolgia-Surface: cli",
                ]);
            if body == "-" {
                invocation.write_stdin("not JSON");
            }
            let result = invocation.assert().code(77);
            let refusal = document(&result);
            assert_eq!(refusal["error"], "agent_refused", "{route}");
            assert_eq!(refusal["command"], command, "{route}");
        }
    }
    assert!(api.received_requests().await.expect("requests").is_empty());
}

#[tokio::test]
async fn encoded_read_routes_remain_available_to_agents() {
    let api = MockServer::start().await;
    // Do not rely on the mock router's percent-decoding behavior.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&api)
        .await;
    cmd(&api)
        .env("HERMES_HOME", "pod-home")
        .env("HERMES_DASHBOARD", "pod-dashboard")
        .args(["api", "GET", "/%6Frganizations"])
        .assert()
        .success()
        .stdout("[]\n");
}

#[tokio::test]
async fn api_generation_duplicates_return_one_unselected_live_job_report() {
    for modality in ["image", "audio", "video"] {
        let api = MockServer::start().await;
        let route = format!("/v1/generate/{modality}");
        respond(
            &api,
            "POST",
            &route,
            409,
            json!({"title": "Conflict", "detail": format!(
                "this exact request was already submitted as job {JOB_ID} and has not been billed twice"
            )}),
        )
        .await;
        let result = cmd(&api)
            .args([
                "api", "POST", &route, "--field", "missing", "--output", "table",
            ])
            .assert()
            .code(75)
            .stderr(predicate::str::contains("has not been billed twice"))
            .stderr(predicate::str::contains("Error:").not())
            .stderr(predicate::str::contains("field").not());
        assert_live(&result, "duplicate");
        assert!(
            document(&result)["follow_up"]
                .as_array()
                .expect("recovery commands")
                .iter()
                .any(|value| value
                    .as_str()
                    .is_some_and(|command| command
                        .starts_with(&format!("nolgia api POST /generate/{modality}"))))
        );
    }
}

#[tokio::test]
async fn api_wait_timeout_recovers_job_from_path_with_or_without_problem_body() {
    for route in [
        format!("/jobs/{JOB_ID}/wait?timeout_seconds=1"),
        format!("/v1/jobs/{JOB_ID}/wait"),
        format!("/jobs/{JOB_ID}/%77ait"),
    ] {
        let api = MockServer::start().await;
        let response = if route.starts_with("/v1/") {
            ResponseTemplate::new(408).set_body_json(json!({
                "title": "Request Timeout",
                "detail": "job did not finish before timeout"
            }))
        } else {
            ResponseTemplate::new(408)
        };
        Mock::given(method("GET"))
            .respond_with(response)
            .expect(1)
            .mount(&api)
            .await;
        let result = cmd(&api)
            .args([
                "api",
                "GET",
                &route,
                "--query",
                "timeout_seconds=1",
                "--field",
                "missing",
                "--output",
                "value",
            ])
            .assert()
            .code(75)
            .stderr(predicate::str::contains("Nothing failed."))
            .stderr(predicate::str::contains("Error:").not())
            .stderr(predicate::str::contains("field").not());
        assert_live(&result, "still_running");
    }
}

#[tokio::test]
async fn api_accepted_generation_selection_failure_preserves_job_id() {
    let api = MockServer::start().await;
    respond(
        &api,
        "POST",
        "/v1/generate/image",
        202,
        json!({"id": JOB_ID, "status": "queued"}),
    )
    .await;
    let result = cmd(&api)
        .args([
            "api",
            "POST",
            "/generate/image",
            "--field",
            "missing",
            "--output",
            "table",
        ])
        .assert()
        .code(75)
        .stderr(predicate::str::contains(JOB_ID))
        .stderr(predicate::str::contains("field \"missing\" not found"))
        .stderr(predicate::str::contains("Error:").not());
    assert_live(&result, "detached");
}

#[tokio::test]
async fn api_unrelated_errors_and_conflicts_without_job_ids_stay_generic() {
    let wait_route = format!("/jobs/{JOB_ID}/wait");
    for (verb, route, status, detail) in [
        (
            "POST",
            "/generate/image",
            409,
            "a conflicting change was made elsewhere".to_owned(),
        ),
        (
            "GET",
            "/generate/image",
            409,
            format!("conflict involving {JOB_ID}"),
        ),
        (
            "POST",
            "/example",
            409,
            format!("conflict involving {JOB_ID}"),
        ),
        ("GET", "/example", 408, "request timed out".to_owned()),
        (
            "POST",
            wait_route.as_str(),
            408,
            "request timed out".to_owned(),
        ),
        (
            "GET",
            "/jobs/not-a-uuid/wait",
            408,
            "request timed out".to_owned(),
        ),
    ] {
        let api = MockServer::start().await;
        let body = json!({"detail": detail});
        respond(&api, verb, &format!("/v1{route}"), status, body.clone()).await;
        let result = cmd(&api)
            .args(["api", verb, route, "--field", "missing"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains("Error: api"))
            .stderr(predicate::str::contains(detail));
        assert_eq!(document(&result), body);
    }
}
