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
                        "start_frame": true,
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
