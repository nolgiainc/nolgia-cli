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
