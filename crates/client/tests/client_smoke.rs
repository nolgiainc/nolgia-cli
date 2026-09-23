use nolgia_client::ClientBuilder;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[tokio::test]
async fn adds_bearer_token_and_targets_v1_me() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .and(header("authorization", "Bearer nol_test_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "2f2f1a1d-7d1c-4d34-91fd-28a4d5e5d5e5",
            "email": "ada@nolgia.ai",
            "name": "Ada Lovelace",
            "image_url": null,
            "created_at": "2026-06-13T00:00:00Z"
        })))
        .mount(&server)
        .await;

    let client = ClientBuilder::new(server.uri())
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let user = client
        .get_current_user()
        .send()
        .await
        .expect("request succeeds")
        .into_inner();

    assert_eq!(user.email, "ada@nolgia.ai");
    assert_eq!(user.name.as_deref(), Some("Ada Lovelace"));

    server.verify().await;
}

/// Released CLIs must never break when the API adds fields their vendored
/// spec predates (NOL-48: api#158's additive `image` capabilities field made
/// v0.2.9/v0.2.10 fail `models list` with `unknown field 'image'`). The
/// catalog payload here carries unknown fields at every level — top-level,
/// model-level, and inside a capabilities object — and parsing must succeed.
#[tokio::test]
async fn models_catalog_tolerates_unknown_fields() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [
                {
                    "id": "veo-3.1",
                    "modality": "video",
                    "recommended": true,
                    "cost": {"credits": 42, "unit": "per_clip"},
                    "future_capability_block": {"nested": ["unknown"]},
                    "references": {
                        "start_frame": true, "start_frame_required": false,
                        "end_frame": false,
                        "video_refs_max": 0,
                        "element_refs_max": 0,
                        "audio_refs_max": 0,
                        "hologram_refs_max": 9
                    }
                },
                {
                    "id": "gpt-image-2",
                    "modality": "image",
                    "recommended": true,
                    "image": {"aspect_ratios": ["16:9"], "future_flag": true}
                }
            ],
            "catalog_revision": "2099-01-01"
        })))
        .mount(&server)
        .await;

    let client = ClientBuilder::new(server.uri())
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let catalog = client
        .list_models()
        .send()
        .await
        .expect("unknown fields in the catalog must not fail parsing")
        .into_inner();

    assert_eq!(catalog.models.len(), 2);
    assert_eq!(catalog.models[0].id, "veo-3.1");
    assert!(catalog.models[1].image.is_some());

    server.verify().await;
}

/// The same guarantee for unknown enum *values*, which the test above does
/// not cover and which broke the CLI a third time (NOL-351).
///
/// Tolerating unknown fields was the NOL-48 fix; NOL-208 then added the
/// `3:1` image aspect ratio and every binary built before that re-vendor
/// died on the entire catalog with `unknown variant '3:1'`. Because
/// `models list`, `models get`, `gen video --cost-only` and the capability
/// prechecks all fetch `GET /models` first, one unrecognised ratio failed
/// jobs that never mentioned an aspect ratio.
///
/// So: every enum-valued field the client only ever *receives* must accept a
/// value this build has never heard of, and must preserve it verbatim — a
/// ratio we cannot offer is still a ratio we can list. This payload uses
/// values that are deliberately absent from the vendored spec.
#[tokio::test]
async fn models_catalog_tolerates_unknown_enum_values() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [
                {
                    "id": "future-image-model",
                    "modality": "image",
                    "recommended": false,
                    "cost": {"credits": 7, "unit": "per_generation"},
                    // `3:1` is the NOL-351 value (now in the spec, so it only
                    // reproduces on older binaries); `128:37` stands in for
                    // whatever the API adds next, and is the case that would
                    // break a build made today.
                    "image": {"aspect_ratios": ["16:9", "3:1", "128:37"], "reference_images_max": 0}
                },
                {
                    "id": "future-video-model",
                    "modality": "video",
                    "recommended": false,
                    // Not in AspectRatio (the video enum, which is narrower).
                    "video": {"durations": [5], "aspect_ratios": ["16:9", "3:1"], "image_input": false},
                    "references": {
                        "start_frame": false, "start_frame_required": false,
                        "end_frame": false,
                        "video_refs_max": 0, "element_refs_max": 0, "audio_refs_max": 0,
                        // Not in BitrateMode.
                        "bitrate_modes": ["standard", "ultra"]
                    }
                },
                {
                    // Not in Modality, and not in ModelCostUnit.
                    "id": "future-hologram-model",
                    "modality": "hologram",
                    "recommended": false,
                    "cost": {"credits": 9, "unit": "per_furlong"}
                }
            ]
        })))
        .mount(&server)
        .await;

    let client = ClientBuilder::new(server.uri())
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let catalog = client
        .list_models()
        .send()
        .await
        .expect("unknown enum values in the catalog must not fail parsing")
        .into_inner();

    assert_eq!(catalog.models.len(), 3);

    // Unknown values survive round-trip rather than being dropped or
    // collapsed to a placeholder, so `models list` reports the catalog the
    // server actually published.
    let image = catalog.models[0]
        .image
        .as_ref()
        .expect("image capabilities");
    assert_eq!(image.aspect_ratios, ["16:9", "3:1", "128:37"]);

    let video = catalog.models[1]
        .video
        .as_ref()
        .expect("video capabilities");
    assert_eq!(video.aspect_ratios, ["16:9", "3:1"]);

    let refs = catalog.models[1]
        .references
        .as_ref()
        .expect("reference capabilities");
    assert_eq!(refs.bitrate_modes, ["standard", "ultra"]);

    assert_eq!(catalog.models[2].modality, "hologram");
    let cost = catalog.models[2].cost.as_ref().expect("cost");
    assert_eq!(cost.unit, "per_furlong");

    server.verify().await;
}

/// NOL-1025: the job cancel must carry `Content-Length: 0`. The generated
/// `cancel_job` builder sends a bodyless POST without it, which the
/// production load balancer refuses with `411` before the API sees it
/// (NOL-542); the mock only answers a request that has the header. A refusal
/// stays an `UnexpectedResponse` whose problem body (and its `code`) is
/// still readable.
#[tokio::test]
async fn cancel_job_with_body_sends_a_sized_post_and_keeps_refusals_readable() {
    use nolgia_client::{ApiError, ClientExt};

    let job_id = "11111111-1111-4111-8111-111111111111";
    let canceled = uuid::Uuid::parse_str(job_id).unwrap();
    let finished = uuid::Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/jobs/{canceled}/cancel")))
        .and(header("content-length", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": job_id, "user_id": job_id, "modality": "video", "model": "m",
            "status": "canceled",
            "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z",
            "cancellation": {
                "canceled_at": "2026-06-13T00:00:01Z", "stage": "before_submit",
                "provider_cancel": "not_needed", "settlement": "refunded",
                "credits_refunded": 30, "message": "All 30 credits were refunded."
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/jobs/{finished}/cancel")))
        .and(header("content-length", "0"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "title": "Conflict", "status": 409, "code": "job_not_cancellable",
            "detail": "This job already finished."
        })))
        .expect(1)
        .mount(&server)
        .await;
    let client = ClientBuilder::new(server.uri())
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let job = client
        .cancel_job_with_body(canceled)
        .await
        .expect("canceled");
    assert_eq!(job.status, "canceled");
    let cancellation = job.cancellation.expect("cancellation present");
    assert_eq!(cancellation.settlement, "refunded");
    assert_eq!(cancellation.credits_refunded, Some(30));

    match client.cancel_job_with_body(finished).await {
        Err(ApiError::UnexpectedResponse(response)) => {
            assert_eq!(response.status(), 409);
            let problem: serde_json::Value = response.json().await.unwrap();
            assert_eq!(problem["code"], "job_not_cancellable");
        }
        other => panic!("expected the 409 as an UnexpectedResponse, got {other:?}"),
    }
    server.verify().await;
}
