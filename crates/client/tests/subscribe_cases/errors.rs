use super::*;

#[test]
fn codes_round_trip_including_open_vocabulary() {
    for (wire, code) in [
        ("out_of_credits", ErrorCode::OutOfCredits),
        ("rate_limit", ErrorCode::RateLimit),
        ("prompt_nsfw", ErrorCode::PromptNsfw),
        ("ip_detected", ErrorCode::IpDetected),
        ("job_failed", ErrorCode::JobFailed),
        ("timeout", ErrorCode::Timeout),
        ("validation", ErrorCode::Validation),
        ("confirmation_rejected", ErrorCode::ConfirmationRejected),
        ("future_code", ErrorCode::Other("future_code".into())),
    ] {
        assert_eq!(ErrorCode::from_wire(wire), code);
        assert_eq!(wire.parse::<ErrorCode>().unwrap(), code);
        assert_eq!(code.as_str(), wire);
        assert_eq!(code.to_string(), wire);
    }
}

#[tokio::test]
async fn submit_http_errors_derive_the_documented_codes() {
    for (status, code) in [
        (402, ErrorCode::OutOfCredits),
        (429, ErrorCode::RateLimit),
        (400, ErrorCode::Validation),
        (422, ErrorCode::Validation),
        (500, ErrorCode::JobFailed),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/generate/image"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({"detail":"request rejected"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = submit(&client(&server), "/generate/image", arguments(), options())
            .await
            .err()
            .expect("submit fails");
        assert_eq!(error.code, code);
        assert_eq!(error.http_status, Some(status));
        assert!(error.message.contains("request rejected"));
    }
}

#[tokio::test]
async fn explicit_http_codes_override_derived_codes_in_priority_order() {
    for (body, expected) in [
        (
            json!({"code":"confirmation_rejected"}),
            ErrorCode::ConfirmationRejected,
        ),
        (
            json!({"code":"future:refusal"}),
            ErrorCode::Other("future:refusal".into()),
        ),
        (
            json!({"failure":{"code":"ip_detected"},"error":{"code":"job_failed"},"code":"out_of_credits"}),
            ErrorCode::IpDetected,
        ),
        (
            json!({"failure":{"code":""},"error":{"code":"rate_limit"},"code":"out_of_credits"}),
            ErrorCode::RateLimit,
        ),
        (
            json!({"failure":{"code":42},"error":{"code":""},"code":"timeout"}),
            ErrorCode::Timeout,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/generate/image"))
            .respond_with(ResponseTemplate::new(402).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let error = submit(&client(&server), "/generate/image", arguments(), options())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, expected);
    }
}

#[tokio::test]
async fn terminal_failures_preserve_raw_job_and_server_codes() {
    for (status, failure, error_field, expected) in [
        (
            "failed",
            json!({"kind":"moderated"}),
            Value::Null,
            ErrorCode::PromptNsfw,
        ),
        (
            "failed",
            json!({"kind":"provider"}),
            Value::Null,
            ErrorCode::JobFailed,
        ),
        ("canceled", Value::Null, Value::Null, ErrorCode::JobFailed),
        (
            "failed",
            json!({"kind":"moderated", "code":"ip_detected"}),
            json!({"code":"rate_limit"}),
            ErrorCode::IpDetected,
        ),
        (
            "failed",
            json!({"code":"future_terminal_code"}),
            Value::Null,
            ErrorCode::Other("future_terminal_code".into()),
        ),
        (
            "failed",
            json!({"code":""}),
            json!({"code":"confirmation_rejected"}),
            ErrorCode::ConfirmationRejected,
        ),
    ] {
        let server = MockServer::start().await;
        mount_submit(&server, job("queued")).await;
        let terminal = json!({"id":"job-1", "status":status, "failure":failure, "error":error_field, "future_field":{"preserve":true}});
        mount_jobs(&server, vec![response(terminal.clone())]).await;
        let error = subscribe(&client(&server), "/generate/image", arguments(), options())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, expected);
        assert_eq!(error.job_id.as_deref(), Some("job-1"));
        assert_eq!(error.job, Some(terminal));
    }
}

#[tokio::test]
async fn unsupported_endpoints_are_rejected_without_sending_a_request() {
    let server = MockServer::start().await;
    for endpoint in [
        "/generate/set",
        "/generate/imag",
        "generate/image",
        "https://other.example/generate/image",
    ] {
        let error = submit(&client(&server), endpoint, arguments(), options())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::Validation);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn every_supported_endpoint_accepts_a_job() {
    for endpoint in [
        "/generate/image",
        "/generate/audio",
        "/generate/video",
        "/generate/3d",
        "/restore/video",
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1{endpoint}")))
            .respond_with(ResponseTemplate::new(202).set_body_json(job("queued")))
            .expect(1)
            .mount(&server)
            .await;
        let handle = submit(&client(&server), endpoint, arguments(), options())
            .await
            .unwrap();
        assert_eq!(handle.job_id(), "job-1");
    }
}
