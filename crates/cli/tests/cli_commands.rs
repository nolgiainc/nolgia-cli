use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, body_partial_json, header, method, path, query_param},
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
        .stdout(predicate::str::contains("pat"))
        .stdout(predicate::str::contains("color-presets"));
}

/// NOL-317: `--help` must name the env vars it reads but never render their
/// values. clap's default for `env`-backed args prints the resolved value,
/// which put a live PAT into `nolgia --help` on the pod — and help output is
/// the least-guarded text there is (scrollback, CI logs, agent transcripts,
/// screenshots, bug reports).
///
/// The structural guard lives in `main.rs`
/// (`env_backed_args_never_render_their_values`) and covers every arg in the
/// tree; this one renders the real help of the real binary with the vars
/// actually set, so the two failure modes stay independent.
#[test]
fn help_never_renders_env_var_values() {
    const SENTINEL_TOKEN: &str = "nol_NOL317x0000_sentinel_must_not_appear";
    const SENTINEL_URL: &str = "https://sentinel-nol317.invalid";

    for args in [
        ["--help"].as_slice(),
        ["gen", "--help"].as_slice(),
        ["auth", "--help"].as_slice(),
        ["assets", "list", "--help"].as_slice(),
    ] {
        let assert = cmd()
            .env("NOLGIA_TOKEN", SENTINEL_TOKEN)
            .env("NOLGIA_API_URL", SENTINEL_URL)
            .args(args)
            .assert()
            .success();
        let help = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 help");
        let invocation = args.join(" ");

        assert!(
            !help.contains(SENTINEL_TOKEN),
            "`nolgia {invocation}` rendered the value of NOLGIA_TOKEN into its help output"
        );
        assert!(
            !help.contains(SENTINEL_URL),
            "`nolgia {invocation}` rendered the value of NOLGIA_API_URL into its help output"
        );
    }

    // The variable names themselves must still be discoverable.
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("NOLGIA_TOKEN"))
        .stdout(predicate::str::contains("NOLGIA_API_URL"));
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

/// NOL-439: the CLI must forward whatever `--model` the caller names and let
/// the API decide whether it exists. `flux-3-video` went live in the API (and
/// in the vendored spec) but the closed client-side enum in the last released
/// binary rejected it at argument parsing —
/// `error: invalid value 'flux-3-video' for '--model <MODEL>': invalid value`
/// — even though `POST /generate/video {model: "flux-3-video"}` accepted it.
/// The build-time relaxation of the request `model` selector (client
/// `build.rs::relax_request_model_selectors`) is what makes the id reach the
/// wire; this asserts it is sent verbatim rather than gated locally.
#[tokio::test]
async fn gen_video_forwards_flux_3_video_model_verbatim() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({ "model": "flux-3-video" })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--model",
            "flux-3-video",
            "--prompt",
            "x",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// The durable half of NOL-439: a model this binary has never heard of — one
/// added to the API after it was built — must still be forwarded, so adopting
/// a new model never again requires a CLI re-release. A closed enum would
/// reject this at parse time; a plain-string selector cannot.
#[tokio::test]
async fn gen_video_forwards_unknown_future_model_verbatim() {
    let api = MockServer::start().await;
    let future_model = "some-model-added-after-this-binary-v99";
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({ "model": future_model })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--model",
            future_model,
            "--prompt",
            "x",
            "--no-wait",
        ],
    )
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
        .stdout(predicate::str::contains("--end-frame"))
        .stdout(predicate::str::contains("--project-id"));
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

/// NOL-342, the exact regression: a `--shot`-only invocation must send **no**
/// `duration_seconds` at all.
///
/// `duration_seconds` is declared `default: 5` in the spec, which describes
/// what the *server* does when the field is absent. Progenitor materialized
/// that default into a non-`Option` field with no `skip_serializing_if`, so
/// every request carried `duration_seconds: 5` whether or not the caller asked
/// for it — and the CLI had no way to express "absent". Against shots summing
/// to anything other than 5 the API rejected the contradiction:
///
/// ```text
/// 400 duration_seconds (5) must equal the sum of shot durations (10) — or omit it
/// ```
///
/// That is every multi-shot job at the film pipeline's default 12s batch, which
/// is why `short-film` — a featured preset — had never once run.
///
/// Asserted as an *exact* body match on purpose: `body_partial_json` would
/// happily pass with a stray `duration_seconds` still in the body, which is
/// precisely the bug being locked out.
#[tokio::test]
async fn gen_video_shots_only_sends_no_duration_seconds() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_json(json!({
            "model": I2V_MODEL,
            "prompt": "overall style",
            "shots": [
                {"prompt": "alpha", "duration_seconds": 5},
                {"prompt": "beta", "duration_seconds": 7},
            ],
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
            "overall style",
            "--shot",
            "5:alpha",
            "--shot",
            "7:beta",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// A shot-less job with no `--duration-seconds` also omits the field entirely
/// and lets the server apply its own 5s default — same resulting clip length as
/// before the fix, without the client asserting a duration nobody asked for.
#[tokio::test]
async fn gen_video_without_duration_flag_sends_no_duration_seconds() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_json(json!({
            "model": I2V_MODEL,
            "prompt": "a single shot",
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
            "a single shot",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// An explicitly requested duration is still sent verbatim.
#[tokio::test]
async fn gen_video_explicit_duration_seconds_is_sent() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({"duration_seconds": 8})))
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
            "--duration-seconds",
            "8",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// `--shot` plus a `--duration-seconds` that *agrees* with the shot sum stays
/// legal, because the API accepts it (it only rejects a mismatch).
///
/// This is not a nicety: the nolgia-agent film pipeline works around the bug by
/// passing the shot sum explicitly (nolgia-agent#152, live on the pod). Erroring
/// on the mere co-presence of both flags would break that pipeline the moment
/// the chart pin moved to this version, so the check is on contradiction only.
#[tokio::test]
async fn gen_video_shots_allow_matching_duration_seconds() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({
            "duration_seconds": 12,
            "shots": [
                {"prompt": "alpha", "duration_seconds": 5},
                {"prompt": "beta", "duration_seconds": 7},
            ],
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
            "overall style",
            "--shot",
            "5:alpha",
            "--shot",
            "7:beta",
            "--duration-seconds",
            "12",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// A `--duration-seconds` that contradicts the shot sum is refused client-side,
/// naming both numbers — instead of being shipped to the API for an opaque 400
/// after an asset upload has already happened.
#[tokio::test]
async fn gen_video_rejects_duration_seconds_contradicting_shots() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "video",
            "--model",
            I2V_MODEL,
            "--prompt",
            "overall style",
            "--shot",
            "5:alpha",
            "--shot",
            "7:beta",
            "--duration-seconds",
            "5",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--duration-seconds 5 contradicts the --shot durations",
        ))
        .stderr(predicate::str::contains("sum to 12"));

    // It must fail before anything is submitted or uploaded.
    assert!(
        api.received_requests().await.unwrap_or_default().is_empty(),
        "contradictory duration must be caught before any API call"
    );
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

/// NOL-345: `gen image` can request an aspect ratio, and it reaches the API as
/// the `aspect_ratio` field (the ratio vocabulary), not an `image_size` alias.
///
/// The three vertical UGC presets need a 9:16 start frame; before this the only
/// route was to generate square and crop in ffmpeg, throwing away ~44% of the
/// frame and — on a 512x512 source — producing a 288x512 image that Kling
/// rejects outright with `Image pixel is invalid`.
#[tokio::test]
async fn gen_image_sends_aspect_ratio() {
    let api = MockServer::start().await;
    mount_image_models(&api).await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({"aspect_ratio": "9:16"})))
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
            "vertical phone photo",
            "--aspect-ratio",
            "9:16",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains("job_id"));
}

/// A ratio the selected model does not publish is refused client-side, naming
/// the model's actual options, rather than becoming a server 400.
#[tokio::test]
async fn gen_image_rejects_aspect_ratio_the_model_does_not_publish() {
    let api = MockServer::start().await;
    mount_image_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "x",
            "--aspect-ratio",
            "21:9",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported by gpt-image-2"))
        .stderr(predicate::str::contains("16:9, 9:16, 1:1"));

    let submitted = api
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/generate/image"))
        .count();
    assert_eq!(submitted, 0, "must not submit an unsupported aspect ratio");
}

/// The `image_size` alias vocabulary (`portrait_16_9`, and NOL-331's
/// `portrait_1080_1920`) is what people reach for first. Rejecting it at parse
/// time with the real ratio list beats an opaque server 400.
#[test]
fn gen_image_rejects_image_size_aliases_with_the_real_ratio_list() {
    for bad in ["portrait_1080_1920", "portrait_16_9"] {
        cmd()
            .args([
                "gen",
                "image",
                "--prompt",
                "x",
                "--aspect-ratio",
                bad,
                "--api-url",
                "http://127.0.0.1:9",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("expected a ratio, one of:"))
            .stderr(predicate::str::contains("9:16"));
    }
}

/// `models get` must surface the per-model ratio list, so the values are
/// discoverable the way the video knobs already are.
#[tokio::test]
async fn models_get_lists_image_aspect_ratios() {
    let api = MockServer::start().await;
    mount_image_models(&api).await;
    run_ok(&api, &["models", "get", "gpt-image-2"])
        .stdout(predicate::str::contains("aspect ratios:"))
        .stdout(predicate::str::contains("9:16"));
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
async fn gen_commands_send_project_id() {
    let api = MockServer::start().await;
    for modality in ["image", "video", "audio"] {
        Mock::given(method("POST"))
            .and(path(format!("/v1/generate/{modality}")))
            .and(body_partial_json(json!({"project_id": PROJECT_ID})))
            .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
            .mount(&api)
            .await;
        run_ok(
            &api,
            &[
                "--json",
                "gen",
                modality,
                "--prompt",
                "x",
                "--project-id",
                PROJECT_ID,
                "--no-wait",
            ],
        )
        .stdout(predicate::str::contains("job_id"));
    }
}

#[tokio::test]
async fn assets_upload_sends_project_id() {
    let api = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ref.png");
    std::fs::write(&file, [1u8, 2, 3]).unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/assets"))
        .and(body_partial_json(json!({
            "content_type": "image/png",
            "project_id": PROJECT_ID,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(asset_json("https://files/ref.png")))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "assets",
            "upload",
            file.to_str().unwrap(),
            "--project-id",
            PROJECT_ID,
        ],
    )
    .stdout(predicate::str::contains("ref.png"));
}

#[tokio::test]
async fn assets_upload_video_uses_signed_flow() {
    let api = MockServer::start().await;
    // A separate server stands in for GCS: the signed PUT target the API hands
    // back must be a real URL the CLI can PUT the bytes to.
    let storage = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("master.mp4");
    std::fs::write(&file, [0u8, 1, 2, 3, 4]).unwrap();
    let put_url = format!("{}/signed-put", storage.uri());

    // 1. Start the signed upload: video/mp4 declared, size + filename derived
    //    from the file, project routed through.
    Mock::given(method("POST"))
        .and(path("/v1/assets/uploads"))
        .and(body_partial_json(json!({
            "content_type": "video/mp4",
            "filename": "master.mp4",
            "size_bytes": 5,
            "project_id": PROJECT_ID,
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "upload_id": ASSET_ID,
            "asset_id": ASSET_ID,
            "upload_url": put_url,
            "expires_at": "2026-06-13T00:30:00Z",
        })))
        .expect(1)
        .mount(&api)
        .await;

    // 2. Bytes go straight to storage with the declared Content-Type.
    Mock::given(method("PUT"))
        .and(path("/signed-put"))
        .and(header("content-type", "video/mp4"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&storage)
        .await;

    // 3. Complete flips the asset to ready and returns it.
    Mock::given(method("POST"))
        .and(path(format!("/v1/assets/uploads/{ASSET_ID}/complete")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ASSET_ID, "user_id": USER_ID, "modality": "video", "model": "user-upload",
            "signed_url": "https://files/master.mp4", "expires_at": "2026-06-13T00:00:00Z",
            "created_at": "2026-06-13T00:00:00Z"
        })))
        .expect(1)
        .mount(&api)
        .await;

    run_ok(
        &api,
        &[
            "assets",
            "upload",
            file.to_str().unwrap(),
            "--project-id",
            PROJECT_ID,
        ],
    )
    .stdout(predicate::str::contains("master.mp4"))
    .stdout(predicate::str::contains("video"));
}

#[tokio::test]
async fn assets_upload_rejects_unknown_extension() {
    let api = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, b"hello").unwrap();
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["assets", "upload", file.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported file extension"));
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

#[tokio::test]
async fn color_presets_list_outputs_catalog_table() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/color-presets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": 1,
            "presets": [
                {"slug": "teal-orange", "name": "Teal & Orange",
                 "description": "Blockbuster complementary grade."},
                {"slug": "noir", "name": "Noir",
                 "description": "High-contrast black and white."},
            ]
        })))
        .mount(&api)
        .await;
    run_ok(&api, &["color-presets", "list"])
        .stdout(predicate::str::contains("teal-orange"))
        .stdout(predicate::str::contains("Teal & Orange"))
        .stdout(predicate::str::contains("High-contrast black and white."));
}

#[tokio::test]
async fn color_presets_list_json_outputs_versioned_catalog() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/color-presets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": 3,
            "presets": [
                {"slug": "noir", "name": "Noir", "description": "High-contrast black and white."},
            ]
        })))
        .mount(&api)
        .await;
    run_ok(&api, &["--json", "color-presets", "list"])
        .stdout(predicate::str::contains("\"version\": 3"))
        .stdout(predicate::str::contains("\"slug\": \"noir\""));
}

const CUBE_TEXT: &str = "TITLE \"teal-orange\"\nLUT_3D_SIZE 33\n0.0 0.0 0.0\n";

#[tokio::test]
async fn color_presets_cube_prints_lut_to_stdout() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/color-presets/teal-orange/cube"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain; charset=utf-8")
                .set_body_string(CUBE_TEXT),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["color-presets", "cube", "teal-orange"]).stdout(predicate::eq(CUBE_TEXT));
}

#[tokio::test]
async fn color_presets_cube_writes_output_file() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/color-presets/teal-orange/cube"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain; charset=utf-8")
                .set_body_string(CUBE_TEXT),
        )
        .mount(&api)
        .await;
    let out = tempfile::tempdir().unwrap().path().join("teal-orange.cube");
    run_ok(
        &api,
        &[
            "color-presets",
            "cube",
            "teal-orange",
            "-o",
            out.to_str().unwrap(),
        ],
    )
    .stdout(predicate::str::contains("wrote"));
    assert_eq!(std::fs::read_to_string(out).unwrap(), CUBE_TEXT);
}

#[tokio::test]
async fn color_presets_cube_404_surfaces_server_detail_verbatim() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/color-presets/nope/cube"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "type": "https://nolgia.ai/errors/not-found",
            "title": "Not found",
            "status": 404,
            "detail": "no color preset with slug \"nope\""
        })))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["color-presets", "cube", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "no color preset with slug \"nope\"",
        ));
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
            "references": {"start_frame": false, "start_frame_required": false, "end_frame": false, "video_refs_max": 3,
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
            "references": {"start_frame": true, "start_frame_required": false, "end_frame": true, "video_refs_max": 0,
                           "element_refs_max": 0, "audio_refs_max": 0},
        },
    ]})
}

async fn mount_image_models(api: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{
            "id": "gpt-image-2", "modality": "image", "recommended": true,
            "image": {
                "aspect_ratios": ["16:9", "9:16", "1:1", "3:2", "2:3"],
                "reference_images_max": 4,
                "num_images_max": 4,
            },
        }]})))
        .mount(api)
        .await;
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

/// NOL-352: the `--generate-audio` help text used to carry a hand-maintained
/// list of model names ("Seedance/Veo"). Nothing kept that list honest, so it
/// drifted — it still omitted MiniMax Hailuo 3 after that model was added, and
/// it advertised audio support for models that turned out not to control the
/// flag at all. That drift is what exposed the underlying defect, so the fix is
/// structural rather than a one-off correction: the flag's help must describe
/// what the model's audio capability decides and must never enumerate models
/// itself, because any enumeration here is a second source of truth that will
/// rot the moment the catalog changes.
///
/// The help stops short of naming `video.audio` / `nolgia models list`: the
/// field is vendored now, but `models list` still does not render it (see the
/// comment on the flag in `commands/gen.rs`), and pointing the reader at a
/// capability they cannot see would just relocate the broken promise. When
/// `models list` shows it, require the citation back here.
#[test]
fn audio_flag_help_stays_capability_driven() {
    let assert = cmd().args(["gen", "video", "--help"]).assert().success();
    let help = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 help");

    let start = help
        .find("--generate-audio")
        .expect("`gen video --help` no longer documents --generate-audio");
    // Take just this flag's entry: everything up to the next flag line, so a
    // neighbouring description (--quality legitimately cites a model as a
    // tier example) cannot make this pass or fail by accident.
    let rest = &help[start..];
    let block = rest
        .match_indices('\n')
        .find(|(offset, _)| {
            let tail = &rest[offset + 1..];
            let line = tail.trim_start();
            let indented = tail.len() > line.len();
            indented
                && (line.starts_with("--") || line.starts_with("-h,") || line.starts_with("-V,"))
        })
        .map_or(rest, |(offset, _)| &rest[..offset]);

    assert!(
        block.to_ascii_lowercase().contains("set by the model"),
        "--generate-audio help must attribute the outcome to the model's \
         capability rather than to the flag, got:\n{block}"
    );

    // The catalog is the only place model-specific audio behaviour is
    // recorded. Naming models here re-creates exactly the list that rotted.
    for model in ["seedance", "veo", "minimax", "hailuo", "kling", "grok"] {
        assert!(
            !block.to_ascii_lowercase().contains(model),
            "--generate-audio help names the model {model:?}; describe the \
             video.audio capability instead so the text cannot drift from the \
             catalog, got:\n{block}"
        );
    }
}

// ---------------------------------------------------------------------------
// NOL-356: a job the server accepted must never be lost.
//
// Three endings used to leave the user with no job id and a message that read
// like a failure, which is what made re-running — and paying twice — the
// natural next move. These tests are written against what a user actually
// sees: the exit status, and the text on their terminal.
// ---------------------------------------------------------------------------

/// Exit status meaning "a job is live; do not re-run" (sysexits EX_TEMPFAIL).
const EXIT_LIVE_JOB: i32 = 75;

/// The RFC 7807 body prod returns when the long-poll window closes. Note it
/// does not name the job — the CLI has to supply that itself.
fn wait_timeout_problem() -> serde_json::Value {
    json!({
        "type": "about:blank", "title": "Request Timeout", "status": 408,
        "detail": "job did not finish before timeout"
    })
}

/// The RFC 7807 body prod returns for a duplicate submission, with the job id
/// carried in prose only.
fn duplicate_problem() -> serde_json::Value {
    json!({
        "type": "about:blank", "title": "Conflict", "status": 409,
        "detail": format!(
            "this exact request was already submitted as job {JOB_ID} less than 5m0s ago \
             and has not been billed twice — check it with GET /jobs/{JOB_ID}. To run it \
             again anyway, resubmit with a different Idempotency-Key header."
        )
    })
}

/// Ask 2. A 408 is the long-poll expiring, not the job failing.
#[tokio::test]
async fn wait_timeout_reads_as_still_running_and_names_the_job() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(408).set_body_json(wait_timeout_problem()))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["wait", JOB_ID, "--timeout", "300"])
        .assert()
        .code(EXIT_LIVE_JOB)
        // Reads as the non-event it is...
        .stderr(predicate::str::contains("still running after 300s"))
        .stderr(predicate::str::contains("Nothing failed."))
        // ...names the job, which the 408 body itself never does...
        .stderr(predicate::str::contains(JOB_ID))
        // ...offers both ways to follow it...
        .stderr(predicate::str::contains(format!("nolgia wait {JOB_ID}")))
        .stderr(predicate::str::contains(format!("nolgia status {JOB_ID}")))
        // ...and never claims something went wrong.
        .stderr(predicate::str::contains("Error:").not())
        .stderr(predicate::str::contains("Unexpected Response").not());
}

/// The same 408, hit while `gen` was waiting on a job it had just submitted.
/// This is the Seedance case from the incident.
#[tokio::test]
async fn gen_wait_timeout_reads_as_still_running_and_names_the_job() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(408).set_body_json(wait_timeout_problem()))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "image", "--prompt", "a cat"])
        .assert()
        .code(EXIT_LIVE_JOB)
        .stderr(predicate::str::contains("still running after 300s"))
        .stderr(predicate::str::contains(format!("nolgia status {JOB_ID}")))
        .stderr(predicate::str::contains("Error:").not());
}

/// Ask 1, the robust half: the id is on the terminal the moment the server
/// accepts it, so it survives an ending no error path can reach — a killed
/// process, a torn-down pipe, a closed terminal.
#[tokio::test]
async fn gen_announces_the_job_id_before_it_starts_waiting() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
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

    // Even on the happy path, and even under --json, the id is announced on
    // stderr — stdout stays a single parseable document.
    run_ok(&api, &["--json", "gen", "image", "--prompt", "a cat"])
        .stderr(predicate::str::contains(format!("submitted job {JOB_ID}")))
        .stderr(predicate::str::contains("Ctrl-C is safe"))
        .stdout(predicate::str::contains("succeeded"));
}

/// Ask 1, the recovery half: when something fails *after* a successful
/// submission, the message must make clear a job exists — the incident's
/// error said nothing at all, so the command looked like it had failed before
/// submitting.
#[tokio::test]
async fn a_failure_after_submission_still_names_the_job() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    // The wait blows up in a way that is a genuine error, not a 408.
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "image", "--prompt", "a cat"])
        .assert()
        .code(EXIT_LIVE_JOB)
        .stderr(predicate::str::contains(format!("submitted job {JOB_ID}")))
        .stderr(predicate::str::contains(
            "The submission itself succeeded, so the job exists",
        ))
        .stderr(predicate::str::contains(format!("nolgia status {JOB_ID}")))
        // The diagnosis is kept, just no longer the whole message.
        .stderr(predicate::str::contains("500"));
}

/// The new 409 carries the one fact the dead pipe swallowed. It must be
/// promoted out of a generic error string.
#[tokio::test]
async fn a_duplicate_submission_renders_as_the_existing_job() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .respond_with(ResponseTemplate::new(409).set_body_json(duplicate_problem()))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "video", "--prompt", "x", "--no-wait"])
        .assert()
        .code(EXIT_LIVE_JOB)
        .stderr(predicate::str::contains(format!(
            "already submitted — job {JOB_ID}"
        )))
        // The server's own assurance, verbatim.
        .stderr(predicate::str::contains("has not been billed twice"))
        // ...re-expressed as commands a shell can actually run, rather than
        // the API's `GET /jobs/{id}`.
        .stderr(predicate::str::contains(format!("nolgia status {JOB_ID}")))
        .stderr(predicate::str::contains("--idempotency-key"))
        .stderr(predicate::str::contains("Error:").not());
}

/// `gen audio` never went through the RFC 7807 helper at all, so it rendered
/// every refusal as progenitor's raw debug dump — including the new 409.
#[tokio::test]
async fn a_duplicate_audio_submission_renders_as_the_existing_job() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/audio"))
        .respond_with(ResponseTemplate::new(409).set_body_json(duplicate_problem()))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "audio", "--prompt", "hello"])
        .assert()
        .code(EXIT_LIVE_JOB)
        .stderr(predicate::str::contains(format!(
            "already submitted — job {JOB_ID}"
        )))
        .stderr(predicate::str::contains("Unexpected Response").not());
}

/// A refusal we cannot decode must not get worse than it was: the server's
/// `detail` still reaches the user verbatim.
#[tokio::test]
async fn a_conflict_without_a_job_id_still_shows_the_server_detail() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "type": "about:blank", "title": "Conflict", "status": 409,
            "detail": "a conflicting change was made elsewhere"
        })))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "image", "--prompt", "x", "--no-wait"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "a conflicting change was made elsewhere",
        ));
}

/// The escape hatch the 409 message advertises has to actually work — the
/// header is accepted by the API but absent from the OpenAPI spec, so the
/// generated builders cannot express it.
#[tokio::test]
async fn idempotency_key_is_sent_on_the_submission() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(header("idempotency-key", "second-take"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;

    run_ok(
        &api,
        &[
            "--idempotency-key",
            "second-take",
            "gen",
            "image",
            "--prompt",
            "x",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// `--json` callers get the same fact as a document they can parse, so a
/// program can adopt the live job rather than re-submitting.
#[tokio::test]
async fn json_mode_emits_the_live_job_as_a_document() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(408).set_body_json(wait_timeout_problem()))
        .mount(&api)
        .await;

    let output = cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["--json", "wait", JOB_ID, "--timeout", "300"])
        .assert()
        .code(EXIT_LIVE_JOB)
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value =
        serde_json::from_slice(&output).expect("--json stdout must stay a parseable document");
    assert_eq!(parsed["job_id"], JOB_ID);
    assert_eq!(parsed["outcome"], "still_running");
    assert_eq!(parsed["billed_twice"], false);
}
