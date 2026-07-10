use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, body_partial_json, method, path, query_param},
};

const JOB_ID: &str = "11111111-1111-4111-8111-111111111111";
const USER_ID: &str = "22222222-2222-4222-8222-222222222222";
const PAT_ID: &str = "33333333-3333-4333-8333-333333333333";
const CHARACTER_ID: &str = "44444444-4444-4444-8444-444444444444";
const PROJECT_ID: &str = "55555555-5555-4555-8555-555555555555";
const ASSET_ID: &str = "66666666-6666-4666-8666-666666666666";
const ELEMENT_ASSET_ID: &str = "77777777-7777-4777-8777-777777777777";
const R2V_MODEL: &str = "fal-ai/bytedance/seedance/v2/pro/reference-to-video";
const I2V_MODEL: &str = "fal-ai/bytedance/seedance/v2/pro/image-to-video";

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
        .stdout(predicate::str::contains("characters"))
        .stdout(predicate::str::contains("projects"))
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

#[test]
fn video_help_lists_quality_and_reference_flags() {
    cmd()
        .args(["gen", "video", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--quality"))
        .stdout(predicate::str::contains("--bitrate"))
        .stdout(predicate::str::contains("--video-ref"))
        .stdout(predicate::str::contains("--element"))
        .stdout(predicate::str::contains("--end-frame"));
}

#[tokio::test]
async fn gen_video_sends_quality_and_reference_fields() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({
            "model": R2V_MODEL,
            "quality": "1080p",
            "bitrate_mode": "high",
            "video_asset_ids": [ASSET_ID],
            "element_asset_ids": [ELEMENT_ASSET_ID],
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--model",
            R2V_MODEL,
            "--prompt",
            "@Video1 restyled with @Image1",
            "--quality",
            "1080p",
            "--bitrate",
            "high",
            "--video-ref",
            ASSET_ID,
            "--element",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

#[tokio::test]
async fn gen_video_sends_end_frame_asset_id() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{ASSET_ID}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(asset_json("https://files/start.png")),
        )
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({
            "image_url": "https://files/start.png",
            "end_image_asset_id": ELEMENT_ASSET_ID,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--model",
            I2V_MODEL,
            "--prompt",
            "x",
            "--input",
            ASSET_ID,
            "--end-frame",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

#[test]
fn gen_video_end_frame_requires_input() {
    cmd()
        .args([
            "gen",
            "video",
            "--prompt",
            "x",
            "--end-frame",
            ASSET_ID,
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--end-frame requires --input"));
}

#[test]
fn gen_video_rejects_more_than_three_video_refs() {
    let mut args = vec!["gen", "video", "--prompt", "x"];
    for _ in 0..4 {
        args.extend(["--video-ref", ASSET_ID]);
    }
    args.extend(["--api-url", "http://127.0.0.1:9"]);
    cmd()
        .args(args)
        .assert()
        .failure()
        .stderr(predicate::str::contains("at most 3 reference videos"));
}

#[tokio::test]
async fn gen_video_unknown_quality_lists_tiers_with_credits() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "video",
            "--model",
            R2V_MODEL,
            "--prompt",
            "x",
            "--quality",
            "8k",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "720p — 165 credits per 5s clip (default)",
        ))
        .stderr(predicate::str::contains(
            "4k — 778 credits per 5s clip (premium)",
        ));
}

#[tokio::test]
async fn gen_video_bitrate_on_wrong_model_is_prechecked() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "video",
            "--model",
            I2V_MODEL,
            "--prompt",
            "x",
            "--bitrate",
            "high",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no bitrate selection"));
}

#[tokio::test]
async fn gen_video_400_surfaces_server_detail_verbatim() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "https://nolgia.ai/errors/invalid-request",
            "title": "Invalid request",
            "status": 400,
            "detail": "`video_asset_ids` requires a reference-to-video model"
        })))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "video", "--prompt", "x", "--no-wait"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "`video_asset_ids` requires a reference-to-video model",
        ));
}

#[tokio::test]
async fn gen_video_cost_only_prices_quality_tier() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--model",
            R2V_MODEL,
            "--prompt",
            "x",
            "--duration-seconds",
            "10",
            "--quality",
            "4k",
            "--cost-only",
        ],
    )
    .stdout(predicate::str::contains("1556 credits"));
}

#[tokio::test]
async fn gen_image_sends_quality() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{
            "id": "gpt-image-2", "modality": "image", "recommended": true,
            "quality": {"default": "standard", "options": [
                {"id": "standard", "credits": 10, "premium": false},
                {"id": "hd", "credits": 25, "premium": true},
            ]},
        }]})))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({"quality": "hd"})))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "--json",
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "x",
            "--quality",
            "hd",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains("job_id"));
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
async fn assets_list_sends_tag_filter() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/assets"))
        .and(query_param("tag", "hero"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items": [asset_json("https://files/a.png")]})),
        )
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "list", "--tag", "hero"]).stdout(predicate::str::contains("a.png"));
}

#[tokio::test]
async fn assets_tag_sends_patch_body_and_prints_tags() {
    let api = MockServer::start().await;
    let mut asset = asset_json("https://files/a.png");
    asset["tags"] = json!(["hero", "draft"]);
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/assets/{ASSET_ID}")))
        .and(body_json(json!({"tags": ["hero", "draft"]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(asset))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["assets", "tag", ASSET_ID, "--tag", "hero", "--tag", "draft"],
    )
    .stdout(predicate::str::contains("tags: [hero, draft]"));
}

#[tokio::test]
async fn assets_tag_clear_sends_empty_tag_set() {
    let api = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/assets/{ASSET_ID}")))
        .and(body_json(json!({"tags": []})))
        .respond_with(ResponseTemplate::new(200).set_body_json(asset_json("https://files/a.png")))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "tag", ASSET_ID, "--clear"])
        .stdout(predicate::str::contains("tags: []"));
}

#[test]
fn assets_tag_requires_tag_or_clear() {
    cmd()
        .args(["assets", "tag", ASSET_ID, "--api-url", "http://127.0.0.1:9"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--tag"));
}

#[tokio::test]
async fn assets_frame_sends_timestamp() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/assets/{ASSET_ID}/frames")))
        .and(body_json(json!({"t_seconds": 3.2})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(asset_json("https://files/frame.png")),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "frame", ASSET_ID, "--at", "3.2"])
        .stdout(predicate::str::contains("frame.png"));
}

#[tokio::test]
async fn assets_frame_defaults_to_last_frame() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/assets/{ASSET_ID}/frames")))
        .and(body_json(json!({})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(asset_json("https://files/last.png")),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["assets", "frame", ASSET_ID, "--last"])
        .stdout(predicate::str::contains("last.png"));
}

#[tokio::test]
async fn assets_frame_surfaces_server_detail_verbatim() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/assets/{ASSET_ID}/frames")))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "https://nolgia.ai/errors/invalid-request",
            "title": "Invalid request",
            "status": 400,
            "detail": "frame extraction requires a video asset"
        })))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["assets", "frame", ASSET_ID])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "frame extraction requires a video asset",
        ));
}

#[test]
fn assets_frame_rejects_at_with_last() {
    cmd()
        .args([
            "assets",
            "frame",
            ASSET_ID,
            "--at",
            "1.5",
            "--last",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[tokio::test]
async fn models_list_shows_quality_and_reference_capabilities() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    run_ok(&api, &["models", "list"])
        .stdout(predicate::str::contains("720p/1080p/4k*"))
        .stdout(predicate::str::contains("video-refs:3"))
        .stdout(predicate::str::contains("end-frame"));
}

#[tokio::test]
async fn models_get_shows_quality_pricing_and_references() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    run_ok(&api, &["models", "get", R2V_MODEL])
        .stdout(predicate::str::contains(
            "720p — 165 credits per 5s clip (default)",
        ))
        .stdout(predicate::str::contains(
            "4k — 778 credits per 5s clip (premium)",
        ))
        .stdout(predicate::str::contains("video-refs <=3"))
        .stdout(predicate::str::contains("elements <=9"))
        .stdout(predicate::str::contains("bitrate standard|high"));
}

#[tokio::test]
async fn characters_list_outputs_characters() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/characters"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"characters": [character_json()]})),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["characters", "list"])
        .stdout(predicate::str::contains(CHARACTER_ID))
        .stdout(predicate::str::contains("Captain Nova"))
        .stdout(predicate::str::contains("1 reference"));
}

#[tokio::test]
async fn characters_create_sends_body() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/characters"))
        .and(body_json(json!({
            "name": "Captain Nova",
            "description": "Silver-haired astronaut",
            "reference_asset_ids": [ASSET_ID]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(character_json()))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "characters",
            "create",
            "--name",
            "Captain Nova",
            "--description",
            "Silver-haired astronaut",
            "--reference-asset-id",
            ASSET_ID,
        ],
    )
    .stdout(predicate::str::contains(CHARACTER_ID));
}

#[tokio::test]
async fn characters_update_sends_only_provided_fields() {
    let api = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/characters/{CHARACTER_ID}")))
        .and(body_json(json!({"name": "Nova Prime"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(character_json()))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["characters", "update", CHARACTER_ID, "--name", "Nova Prime"],
    )
    .stdout(predicate::str::contains(CHARACTER_ID));
}

#[tokio::test]
async fn characters_delete_removes_character() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/characters/{CHARACTER_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&api)
        .await;
    run_ok(&api, &["characters", "delete", CHARACTER_ID])
        .stdout(predicate::str::contains(format!("deleted {CHARACTER_ID}")));
}

#[test]
fn characters_create_rejects_more_than_four_references() {
    let a = "77777777-7777-4777-8777-777777777777";
    cmd()
        .args([
            "characters",
            "create",
            "--name",
            "x",
            "--reference-asset-id",
            a,
            "--reference-asset-id",
            a,
            "--reference-asset-id",
            a,
            "--reference-asset-id",
            a,
            "--reference-asset-id",
            a,
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("at most 4"));
}

#[tokio::test]
async fn projects_list_outputs_projects() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"projects": [project_json()]})),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["projects", "list"])
        .stdout(predicate::str::contains(PROJECT_ID))
        .stdout(predicate::str::contains("Launch teaser"))
        .stdout(predicate::str::contains("3 assets"));
}

#[tokio::test]
async fn projects_create_sends_body() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects"))
        .and(body_json(json!({
            "name": "Launch teaser",
            "description": "Spring launch assets"
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(project_json()))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "projects",
            "create",
            "--name",
            "Launch teaser",
            "--description",
            "Spring launch assets",
        ],
    )
    .stdout(predicate::str::contains(PROJECT_ID));
}

#[tokio::test]
async fn projects_add_assets_sends_body() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/projects/{PROJECT_ID}/assets")))
        .and(body_json(json!({"asset_ids": [ASSET_ID]})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["projects", "add-assets", PROJECT_ID, "--asset-id", ASSET_ID],
    )
    .stdout(predicate::str::contains("added 1 asset"));
}

#[tokio::test]
async fn projects_remove_asset_deletes_membership() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/projects/{PROJECT_ID}/assets/{ASSET_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&api)
        .await;
    run_ok(&api, &["projects", "remove-asset", PROJECT_ID, ASSET_ID]).stdout(
        predicate::str::contains(format!("removed {ASSET_ID} from {PROJECT_ID}")),
    );
}

#[test]
fn projects_add_assets_requires_asset_id() {
    cmd()
        .args([
            "projects",
            "add-assets",
            PROJECT_ID,
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--asset-id"));
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

#[test]
fn auth_token_reads_the_file_store() {
    let home = tempfile::tempdir().unwrap();
    write_token_file(home.path(), "file-access-token");
    cmd()
        .env("XDG_CONFIG_HOME", home.path())
        .args(["auth", "token"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file-access-token"));
}

#[test]
fn auth_logout_deletes_the_token_file() {
    let home = tempfile::tempdir().unwrap();
    write_token_file(home.path(), "soon-gone");
    cmd()
        .env("XDG_CONFIG_HOME", home.path())
        .args(["auth", "logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("logged out"));
    assert!(!home.path().join("nolgia/tokens.json").exists());
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

#[tokio::test]
async fn ability_list_shows_marketplace_catalog() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/abilities"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([ability_json("public", true)])),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["ability", "list"])
        .stdout(predicate::str::contains("nolgia-cli-basics"))
        .stdout(predicate::str::contains("v1.0.0"));
}

#[tokio::test]
async fn ability_list_marks_private_abilities() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/abilities"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([ability_json("private", true)])),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["ability", "list"]).stdout(predicate::str::contains("[private]"));
}

#[tokio::test]
async fn ability_install_reports_pod_delivery() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/agent/abilities/nolgia-cli-basics"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "slug": "nolgia-cli-basics", "name": "NOLGIA CLI Basics", "description": "d",
            "latest_version": "1.0.0", "installed_at": "2026-06-13T00:00:00Z"
        })))
        .mount(&api)
        .await;
    run_ok(&api, &["ability", "install", "nolgia-cli-basics"]).stdout(predicate::str::contains(
        "installed nolgia-cli-basics v1.0.0",
    ));
}

#[tokio::test]
async fn ability_sync_materializes_installed_abilities() {
    use base64::Engine as _;
    // Build a tiny ability tarball to serve as content.
    let targz = {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let body = b"---\nname: nolgia-cli-basics\n---\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "SKILL.md", &body[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    };

    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/agent/abilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "slug": "nolgia-cli-basics", "name": "NOLGIA CLI Basics", "description": "d",
            "latest_version": "1.0.0", "installed_at": "2026-06-13T00:00:00Z"
        }])))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/abilities/nolgia-cli-basics/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "slug": "nolgia-cli-basics", "version": "1.0.0", "manifest": {},
            "content_base64": base64::engine::general_purpose::STANDARD.encode(&targz)
        })))
        .mount(&api)
        .await;

    let dir = tempfile::tempdir().unwrap();
    run_ok(
        &api,
        &["ability", "sync", "--dir", dir.path().to_str().unwrap()],
    )
    .stdout(predicate::str::contains(
        "synced   nolgia-cli-basics v1.0.0",
    ));
    assert!(dir.path().join("nolgia-cli-basics/SKILL.md").is_file());
    assert!(
        dir.path()
            .join("nolgia-cli-basics/.nolgia-ability.json")
            .is_file()
    );

    // Second sync is a no-op ("current"), driven by the version marker.
    run_ok(
        &api,
        &["ability", "sync", "--dir", dir.path().to_str().unwrap()],
    )
    .stdout(predicate::str::contains(
        "current  nolgia-cli-basics v1.0.0",
    ));
}

#[tokio::test]
async fn ability_publish_sends_manifest_and_content() {
    let pkg = tempfile::tempdir().unwrap();
    std::fs::write(
        pkg.path().join("ability.json"),
        json!({
            "slug": "nolgia-cli-basics", "name": "NOLGIA CLI Basics", "version": "1.0.0",
            "description": "CLI basics", "required_env": ["NOLGIA_TOKEN"],
            "min_tier": "", "visibility": "public", "credit_cost_hint": "free"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pkg.path().join("SKILL.md"),
        "---\nname: nolgia-cli-basics\n---\n",
    )
    .unwrap();

    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/abilities"))
        .respond_with(ResponseTemplate::new(201).set_body_json(ability_json("public", true)))
        .mount(&api)
        .await;
    run_ok(&api, &["ability", "publish", pkg.path().to_str().unwrap()]).stdout(
        predicate::str::contains("published nolgia-cli-basics v1.0.0 (public, min_tier: free)"),
    );
}

#[test]
fn ability_help_lists_authoring_verbs() {
    cmd()
        .args(["ability", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("pack"))
        .stdout(predicate::str::contains("publish"));
}

#[tokio::test]
async fn ability_init_pack_publish_roundtrip() {
    let base = tempfile::tempdir().unwrap();
    let authoring = base.path().join("my-ability");
    let api = MockServer::start().await;

    run_ok(
        &api,
        &[
            "ability",
            "init",
            "my-ability",
            "--dir",
            authoring.to_str().unwrap(),
        ],
    )
    .stdout(predicate::str::contains("nolgia ability pack"));

    // Author the ability: drop code into payload/ and declare a pip dep.
    std::fs::write(authoring.join("payload/tool.py"), "print('hi')\n").unwrap();
    let manifest = std::fs::read_to_string(authoring.join("ability.json")).unwrap();
    assert!(manifest.contains("\"python_requirements\": []"));
    std::fs::write(
        authoring.join("ability.json"),
        manifest.replace(
            "\"python_requirements\": []",
            "\"python_requirements\": [\"requests>=2.31\"]",
        ),
    )
    .unwrap();

    let out = base.path().join("dist/my-ability");
    run_ok(
        &api,
        &[
            "ability",
            "pack",
            authoring.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
    )
    .stdout(predicate::str::contains("packed my-ability v0.1.0"))
    .stdout(predicate::str::contains("tool.py"));
    // Payload contents land at the package root, next to SKILL.md.
    assert!(out.join("tool.py").is_file());
    assert!(!out.join("payload").exists());

    // The packed dir publishes as-is; python_requirements travels verbatim
    // inside the manifest.
    Mock::given(method("POST"))
        .and(path("/v1/abilities"))
        .and(body_partial_json(json!({
            "slug": "my-ability", "version": "0.1.0", "visibility": "private",
            "manifest": { "python_requirements": ["requests>=2.31"] }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(ability_json("private", true)))
        .mount(&api)
        .await;
    run_ok(&api, &["ability", "publish", out.to_str().unwrap()])
        .stdout(predicate::str::contains("published"));
}

#[test]
fn ability_pack_rejects_bad_version() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ability.json"),
        json!({
            "slug": "my-ability", "name": "My Ability", "version": "1.0",
            "description": "d", "visibility": "private"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(dir.path().join("SKILL.md"), "---\nname: my-ability\n---\n").unwrap();
    cmd()
        .args(["ability", "pack", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("version"));
}

fn ability_json(visibility: &str, entitled: bool) -> serde_json::Value {
    json!({
        "slug": "nolgia-cli-basics", "name": "NOLGIA CLI Basics",
        "description": "Drive the platform with the nolgia CLI", "required_env": ["NOLGIA_TOKEN"],
        "credit_cost_hint": "free", "min_tier": "", "visibility": visibility, "entitled": entitled,
        "access": "included", "has_code": false, "latest_version": "1.0.0",
        "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
    })
}

fn cmd() -> Command {
    // Keep every spawned binary away from the operator's real credentials
    // and keychain: freshly built test binaries are new signing identities,
    // so a keyring probe from here can trigger macOS keychain password
    // prompts. Force the file token store (no keyring migration probe) and
    // point all config/state at a per-test-process temp dir.
    static ISOLATED_HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let home = ISOLATED_HOME.get_or_init(|| tempfile::tempdir().expect("isolated config dir"));
    let mut command = Command::cargo_bin("nolgia").unwrap();
    command.env_remove("NOLGIA_TOKEN");
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

/// Catalog fixture for the quality/reference-capability surface: the
/// Seedance 2.0 Pro reference-to-video model (quality tiers, video/element
/// refs, bitrate modes) and its image-to-video sibling (start+end frames,
/// no refs, no bitrate knob).
fn video_models_json() -> serde_json::Value {
    json!({"models": [
        {
            "id": R2V_MODEL, "modality": "video", "recommended": true,
            "cost": {"credits": 165, "unit": "per_clip", "baseline_seconds": 5},
            "video": {"min_duration": 2, "max_duration": 15, "aspect_ratios": ["16:9", "9:16"], "image_input": false},
            "quality": {"default": "720p", "options": [
                {"id": "720p", "credits": 165, "premium": false},
                {"id": "1080p", "credits": 360, "premium": false},
                {"id": "4k", "credits": 778, "premium": true},
            ]},
            "references": {"start_frame": false, "end_frame": false, "video_refs_max": 3,
                           "element_refs_max": 9, "audio_refs_max": 3, "bitrate_modes": ["standard", "high"]},
        },
        {
            "id": I2V_MODEL, "modality": "video", "recommended": false,
            "cost": {"credits": 165, "unit": "per_clip", "baseline_seconds": 5},
            "video": {"min_duration": 2, "max_duration": 15, "aspect_ratios": ["16:9"], "image_input": true},
            "quality": {"default": "720p", "options": [
                {"id": "720p", "credits": 165, "premium": false},
                {"id": "1080p", "credits": 360, "premium": false},
            ]},
            "references": {"start_frame": true, "end_frame": true, "video_refs_max": 0,
                           "element_refs_max": 0, "audio_refs_max": 0},
        },
    ]})
}

async fn mount_video_models(api: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(video_models_json()))
        .mount(api)
        .await;
}

fn asset_json(url: &str) -> serde_json::Value {
    json!({
        "id": Uuid::new_v4(), "user_id": USER_ID, "modality": "image", "model": "fal-ai/flux-pro/v1.1",
        "signed_url": url, "expires_at": "2026-06-13T00:00:00Z", "created_at": "2026-06-13T00:00:00Z"
    })
}

fn character_json() -> serde_json::Value {
    json!({
        "id": CHARACTER_ID, "user_id": USER_ID, "name": "Captain Nova",
        "description": "Silver-haired astronaut",
        "reference_assets": [asset_json("https://files/ref.png")],
        "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
    })
}

fn project_json() -> serde_json::Value {
    json!({
        "id": PROJECT_ID, "user_id": USER_ID, "name": "Launch teaser",
        "description": "Spring launch assets", "asset_count": 3,
        "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
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
