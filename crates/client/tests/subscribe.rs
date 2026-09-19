use nolgia_client::{Client, ClientBuilder, ErrorCode, SubscribeOptions, submit, subscribe};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

#[path = "subscribe_cases/errors.rs"]
mod errors;
#[path = "subscribe_cases/polling.rs"]
mod polling;

fn client(server: &MockServer) -> Client {
    ClientBuilder::new(server.uri())
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds")
}
fn options() -> SubscribeOptions {
    SubscribeOptions {
        poll_interval: Duration::from_millis(1),
        max_poll_time: Duration::from_secs(2),
        ..Default::default()
    }
}
fn arguments() -> Value {
    json!({"model":"flux-pro", "prompt":"a paper-cut mountain range"})
}
fn job(status: &str) -> Value {
    json!({"id":"job-1", "status":status})
}
fn asset(id: &str) -> Value {
    json!({"id":id,"signed_url":format!("https://media.example/{id}"),"modality":"image","expires_at":"2026-09-20T00:00:00Z"})
}
async fn mount_submit(server: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(header("authorization", "Bearer nol_test_token"))
        .and(body_json(arguments()))
        .respond_with(ResponseTemplate::new(202).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}
async fn mount_jobs(server: &MockServer, replies: Vec<ResponseTemplate>) {
    let count = replies.len();
    let index = AtomicUsize::new(0);
    Mock::given(method("GET"))
        .and(path("/v1/jobs/job-1"))
        .and(header("authorization", "Bearer nol_test_token"))
        .respond_with(move |_: &Request| {
            replies[index.fetch_add(1, Ordering::SeqCst).min(count - 1)].clone()
        })
        .expect(u64::try_from(count).expect("small response count"))
        .mount(server)
        .await;
}
async fn mount_assets(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/v1/assets"))
        .and(query_param("job_id", "job-1"))
        .and(query_param("limit", "100"))
        .and(header("authorization", "Bearer nol_test_token"))
        .respond_with(response)
        .expect(1)
        .mount(server)
        .await;
}
fn response(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

#[tokio::test]
async fn subscription_reports_only_status_changes_and_resolves_every_asset_in_order() {
    let server = MockServer::start().await;
    let updates = Arc::new(Mutex::new(Vec::new()));
    let collected = Arc::clone(&updates);
    let mut opts = options();
    opts.headers
        .push(("Idempotency-Key".into(), "request-1".into()));
    opts.on_status = Some(Box::new(move |update| {
        collected.lock().unwrap().push(update.clone())
    }));
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(header("authorization", "Bearer nol_test_token"))
        .and(header("Idempotency-Key", "request-1"))
        .and(body_json(arguments()))
        .respond_with(ResponseTemplate::new(202).set_body_json(job("queued")))
        .expect(1)
        .mount(&server)
        .await;
    let mut running = job("running");
    running["progress"] = json!(0.5);
    running["status_detail"] = json!("rendering");
    running["status_message"] = json!("Working");
    let mut detail = running.clone();
    detail["status_detail"] = json!("encoding");
    let mut message = detail.clone();
    message["status_message"] = json!("Finishing");
    let mut progress = message.clone();
    progress["progress"] = json!(0.9);
    let mut unchanged = progress.clone();
    unchanged["future_field"] = json!(true);
    let mut terminal = job("succeeded");
    terminal["asset"] = asset("inline-not-in-list");
    mount_jobs(
        &server,
        vec![
            response(job("queued")),
            response(running),
            response(detail),
            response(message),
            response(progress),
            response(unchanged),
            response(terminal),
        ],
    )
    .await;
    mount_assets(
        &server,
        response(json!({"items":[asset("asset-2"),asset("asset-1"),asset("asset-2")]})),
    )
    .await;
    let result = subscribe(&client(&server), "/generate/image", arguments(), opts)
        .await
        .unwrap();
    assert_eq!(result.job_id, "job-1");
    assert_eq!(result.job["status"], "succeeded");
    assert_eq!(
        result
            .media
            .iter()
            .map(|item| item.asset_id.as_str())
            .collect::<Vec<_>>(),
        ["asset-2", "asset-1"]
    );
    assert_eq!(result.url.as_deref(), Some("https://media.example/asset-2"));
    assert_eq!(result.media[0].modality, "image");
    assert_eq!(
        result.media[0].expires_at.as_deref(),
        Some("2026-09-20T00:00:00Z")
    );
    let updates = updates.lock().unwrap();
    assert_eq!(updates.len(), 6);
    assert_eq!(updates[0].status, "queued");
    assert_eq!(updates[1].job_id, "job-1");
    assert_eq!(updates[1].progress, Some(0.5));
    assert_eq!(updates[2].status_detail.as_deref(), Some("encoding"));
    assert_eq!(updates[3].status_message.as_deref(), Some("Finishing"));
    assert_eq!(updates[4].job["progress"], 0.9);
    assert_eq!(updates[5].status, "succeeded");
}

#[tokio::test]
async fn empty_asset_list_falls_back_to_inline_asset() {
    let server = MockServer::start().await;
    let mut terminal = job("succeeded");
    terminal["asset"] = asset("inline");
    mount_submit(&server, job("queued")).await;
    mount_jobs(&server, vec![response(terminal)]).await;
    mount_assets(&server, response(json!({"items":[]}))).await;
    let result = subscribe(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    assert_eq!(result.media.len(), 1);
    assert_eq!(result.media[0].asset_id, "inline");
    assert_eq!(result.url.as_deref(), Some("https://media.example/inline"));
}

#[tokio::test]
async fn zero_assets_is_success_when_the_asset_list_is_empty() {
    let server = MockServer::start().await;
    mount_submit(&server, job("succeeded")).await;
    mount_assets(&server, response(json!({"items":[]}))).await;
    let result = subscribe(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    assert!(result.media.is_empty());
    assert!(result.url.is_none());
}

#[tokio::test]
async fn asset_list_failures_fall_back_to_the_inline_asset_rather_than_losing_a_paid_render() {
    // The job has already succeeded and been charged by the time the asset
    // listing runs, so a failed secondary read must not throw the render away.
    // Same behaviour as the TypeScript and Python layers (nolgia-api#527).
    for (label, assets_response) in [
        ("503", ResponseTemplate::new(503)),
        ("403", ResponseTemplate::new(403)),
        (
            "malformed",
            ResponseTemplate::new(200).set_body_string("not json"),
        ),
    ] {
        let server = MockServer::start().await;
        let mut terminal = job("succeeded");
        terminal["asset"] = asset("inline");
        mount_submit(&server, job("queued")).await;
        mount_jobs(&server, vec![response(terminal)]).await;
        mount_assets(&server, assets_response).await;
        let result = subscribe(&client(&server), "/generate/image", arguments(), options())
            .await
            .unwrap_or_else(|err| panic!("{label} listing failure lost the render: {err}"));
        assert_eq!(result.media.len(), 1, "{label}");
        assert_eq!(result.media[0].asset_id, "inline", "{label}");
    }
}

#[tokio::test]
async fn a_failed_asset_list_with_no_inline_asset_is_an_empty_success() {
    let server = MockServer::start().await;
    mount_submit(&server, job("succeeded")).await;
    mount_assets(&server, ResponseTemplate::new(503)).await;
    let result = subscribe(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    assert!(result.media.is_empty());
    assert!(result.url.is_none());
}
