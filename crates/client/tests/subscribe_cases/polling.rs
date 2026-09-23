use super::*;

#[tokio::test]
async fn transient_job_errors_retry_and_reset_after_a_successful_poll() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    let mut replies = vec![ResponseTemplate::new(500); 4];
    replies.push(response(job("running")));
    replies.extend(vec![ResponseTemplate::new(503); 4]);
    replies.push(response(job("succeeded")));
    mount_jobs(&server, replies).await;
    mount_assets(&server, response(json!({"items":[]}))).await;
    assert!(
        subscribe(&client(&server), "/generate/image", arguments(), options())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn five_consecutive_server_failures_stop_retrying() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    mount_jobs(
        &server,
        vec![
            ResponseTemplate::new(500).set_body_json(json!({"detail":"temporarily unavailable"}));
            5
        ],
    )
    .await;
    let error = subscribe(&client(&server), "/generate/image", arguments(), options())
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::JobFailed);
    assert_eq!(error.http_status, Some(500));
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
}

#[tokio::test]
async fn permanent_job_errors_fail_on_the_first_request() {
    for status in [401, 403, 404] {
        let server = MockServer::start().await;
        mount_submit(&server, job("queued")).await;
        mount_jobs(&server, vec![ResponseTemplate::new(status)]).await;
        let error = subscribe(&client(&server), "/generate/image", arguments(), options())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::JobFailed);
        assert_eq!(error.http_status, Some(status));
        assert_eq!(error.job_id.as_deref(), Some("job-1"));
    }
}

#[tokio::test]
async fn local_wait_budget_expires_before_a_long_poll_interval() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    let opts = SubscribeOptions {
        poll_interval: Duration::from_secs(60),
        max_poll_time: Duration::from_millis(10),
        ..Default::default()
    };
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        subscribe(&client(&server), "/generate/image", arguments(), opts),
    )
    .await
    .expect("wait budget enforced during sleep")
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn submit_is_lazy_and_status_performs_exactly_one_get() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    mount_jobs(
        &server,
        vec![response(job("running")), response(job("succeeded"))],
    )
    .await;
    mount_assets(&server, response(json!({"items":[]}))).await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    assert_eq!(handle.job_id(), "job-1");
    assert_eq!(handle.job(), &job("queued"));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(handle.status().await.unwrap(), job("running"));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(handle.job(), &job("queued"));
    assert!(handle.result().await.is_ok());
}

#[tokio::test]
// The deprecated local-only `cancel()` keeps its behavior; this pins it.
#[allow(deprecated)]
async fn cancelling_a_pending_result_only_stops_local_waiting() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    let polls = Arc::new(AtomicUsize::new(0));
    let received = Arc::clone(&polls);
    Mock::given(method("GET"))
        .and(path("/v1/jobs/job-1"))
        .respond_with(move |_: &Request| {
            received.fetch_add(1, Ordering::SeqCst);
            response(job("running"))
        })
        .mount(&server)
        .await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    let cancel_handle = handle.clone();
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(handle.result(), async {
            while polls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            cancel_handle.cancel();
        })
    })
    .await
    .expect("cancellation resolves pending wait");
    let error = result.err().unwrap();
    assert_eq!(error.code, ErrorCode::JobFailed);
    assert!(error.message.to_lowercase().contains("cancel"));
    assert!(error.message.to_lowercase().contains("local"));
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    assert_ne!(error.job.unwrap()["status"], "canceled");
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() == "/v1/generate/image"
                || request.url.path() == "/v1/jobs/job-1")
    );
}

#[tokio::test]
async fn transport_failures_exhaust_retries_without_losing_the_job_id() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let server = MockServer::builder().listener(listener).start().await;
    mount_submit(&server, job("queued")).await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    // Destroy this isolated listener after submission to cause connection failures.
    drop(server);
    let error = tokio::time::timeout(Duration::from_secs(1), handle.result())
        .await
        .expect("transport retries are bounded")
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::JobFailed);
    assert_eq!(error.http_status, None);
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
}

#[tokio::test]
async fn deadline_interrupts_an_in_flight_job_request() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    mount_jobs(
        &server,
        vec![response(job("running")).set_delay(Duration::from_secs(5))],
    )
    .await;
    let opts = SubscribeOptions {
        max_poll_time: Duration::from_millis(100),
        ..options()
    };
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        subscribe(&client(&server), "/generate/image", arguments(), opts),
    )
    .await
    .expect("deadline interrupts pending HTTP response")
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
}

#[tokio::test]
// The deprecated local-only `cancel()` keeps its behavior; this pins it.
#[allow(deprecated)]
async fn cancellation_interrupts_an_in_flight_job_request() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    let received = Arc::new(AtomicUsize::new(0));
    let request_seen = Arc::clone(&received);
    Mock::given(method("GET"))
        .and(path("/v1/jobs/job-1"))
        .respond_with(move |_: &Request| {
            request_seen.fetch_add(1, Ordering::SeqCst);
            response(job("running")).set_delay(Duration::from_secs(5))
        })
        .expect(1)
        .mount(&server)
        .await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    let cancel = handle.clone();
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(handle.result(), async {
            while received.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            cancel.cancel();
        })
    })
    .await
    .expect("cancellation interrupts pending HTTP response");
    let error = result.err().unwrap();
    assert_eq!(error.code, ErrorCode::JobFailed);
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    assert!(error.message.to_lowercase().contains("local"));
}

#[test]
fn defaults_use_half_second_ticks_and_thirty_minute_budget() {
    let opts = SubscribeOptions::default();
    assert_eq!(opts.poll_interval, Duration::from_millis(500));
    assert_eq!(opts.max_poll_time, Duration::from_secs(30 * 60));
    assert!(opts.on_status.is_none());
    assert!(opts.headers.is_empty());
}

#[tokio::test]
// The deprecated local-only `cancel()` keeps its behavior; this pins it.
#[allow(deprecated)]
async fn cancellation_before_result_is_retained_without_a_waiter() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    handle.cancel();
    let error = handle.result().await.err().unwrap();
    assert_eq!(error.code, ErrorCode::JobFailed);
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    assert!(error.message.contains("locally"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

fn canceled_job() -> Value {
    json!({
        "id": "job-1", "status": "canceled",
        "status_message": "Canceled before it reached the model provider. All 30 credits were refunded.",
        "cancellation": {
            "canceled_at": "2026-09-18T10:00:01Z", "stage": "before_submit",
            "provider_cancel": "not_needed", "settlement": "refunded", "credits_refunded": 30,
            "message": "Canceled before it reached the model provider. All 30 credits were refunded."
        },
        "future_field": {"preserve": true}
    })
}

/// NOL-1025. Only a cancel carrying `Content-Length: 0` matches: the
/// production load balancer refuses a bodyless POST without it (`411`).
async fn mount_cancel(server: &MockServer, reply: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/jobs/job-1/cancel"))
        .and(header("authorization", "Bearer nol_test_token"))
        .and(header("content-length", "0"))
        .respond_with(reply)
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn cancel_job_cancels_on_the_server_and_returns_the_raw_canceled_job() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    mount_cancel(&server, response(canceled_job())).await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    let canceled = handle.cancel_job().await.unwrap();
    // Raw, like `status()`: unknown fields survive.
    assert_eq!(canceled, canceled_job());
    // A result() started afterwards ends at once, with no polling.
    let error = handle.result().await.err().unwrap();
    assert_eq!(error.code, ErrorCode::Canceled);
    assert_eq!(
        error.message,
        "Canceled before it reached the model provider. All 30 credits were refunded."
    );
    assert_eq!(error.http_status, None);
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    assert_eq!(error.job, Some(canceled_job()));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() != "/v1/jobs/job-1")
    );
}

#[tokio::test]
async fn cancel_job_ends_a_pending_result_with_the_servers_cancellation() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    mount_cancel(&server, response(canceled_job())).await;
    let polls = Arc::new(AtomicUsize::new(0));
    let received = Arc::clone(&polls);
    Mock::given(method("GET"))
        .and(path("/v1/jobs/job-1"))
        .respond_with(move |_: &Request| {
            received.fetch_add(1, Ordering::SeqCst);
            response(job("running"))
        })
        .mount(&server)
        .await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    let cancel_handle = handle.clone();
    let (result, canceled) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(handle.result(), async {
            while polls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            cancel_handle.cancel_job().await
        })
    })
    .await
    .expect("a server cancel resolves the pending wait");
    assert_eq!(canceled.unwrap()["status"], "canceled");
    let error = result.err().unwrap();
    assert_eq!(error.code, ErrorCode::Canceled);
    assert!(error.message.contains("All 30 credits were refunded."));
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    // The error carries the canceled job, not the last `running` poll.
    let job = error.job.unwrap();
    assert_eq!(job["status"], "canceled");
    assert_eq!(job["cancellation"]["settlement"], "refunded");
}

#[tokio::test]
async fn cancel_job_on_a_finished_job_is_refused_and_the_wait_keeps_going() {
    let server = MockServer::start().await;
    mount_submit(&server, job("queued")).await;
    mount_cancel(
        &server,
        ResponseTemplate::new(409).set_body_json(json!({
            "type": "about:blank", "title": "Conflict", "status": 409,
            "code": "job_not_cancellable", "detail": "This job already finished."
        })),
    )
    .await;
    mount_jobs(
        &server,
        vec![response(job("running")), response(job("succeeded"))],
    )
    .await;
    mount_assets(&server, response(json!({"items": [asset("done")]}))).await;
    let handle = submit(&client(&server), "/generate/image", arguments(), options())
        .await
        .unwrap();
    let error = handle.cancel_job().await.err().unwrap();
    assert_eq!(error.code, ErrorCode::JobNotCancellable);
    assert_eq!(error.http_status, Some(409));
    assert_eq!(error.message, "This job already finished.");
    assert_eq!(error.job_id.as_deref(), Some("job-1"));
    // Nothing was changed: the wait reaches the job's own result.
    let result = handle.result().await.unwrap();
    assert_eq!(result.url.as_deref(), Some("https://media.example/done"));
}
