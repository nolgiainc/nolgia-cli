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
const PRODUCT_ID: &str = "88888888-8888-4888-8888-888888888888";
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
        .stdout(predicate::str::contains("jobs"))
        .stdout(predicate::str::contains("voices"))
        .stdout(predicate::str::contains("wait"))
        .stdout(predicate::str::contains("assets"))
        .stdout(predicate::str::contains("characters"))
        .stdout(predicate::str::contains("projects"))
        .stdout(predicate::str::contains("products"))
        .stdout(predicate::str::contains("compositions"))
        .stdout(predicate::str::contains("render"))
        .stdout(predicate::str::contains("account"))
        .stdout(predicate::str::contains("billing"))
        .stdout(predicate::str::contains("pat"))
        .stdout(predicate::str::contains("restore"))
        .stdout(predicate::str::contains("color-presets"))
        .stdout(predicate::str::contains("motions"))
        .stdout(predicate::str::contains("masks"));
}

#[tokio::test]
async fn render_blocks_submits_and_waits_for_the_asset() {
    let api = MockServer::start().await;
    let comp_id = Uuid::new_v4();
    let render_id = Uuid::new_v4();
    let final_asset = Uuid::new_v4();
    Mock::given(method("POST"))
        .and(path("/v1/renders/blocks"))
        .and(body_json(json!({
            "blocks": [
                {"video_asset_id": ASSET_ID, "audio_asset_id": ELEMENT_ASSET_ID},
                {"video_asset_id": ELEMENT_ASSET_ID, "audio_asset_id": ASSET_ID}
            ],
            "block_seconds": 12.5, "aspect_ratio": "9:16", "keep_video_audio": true,
            "name": "narrated explainer", "project_id": PROJECT_ID
        })))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(render_json(render_id, comp_id, "queued", None)),
        )
        .expect(1)
        .mount(&api)
        .await;
    let mut finished = render_json(render_id, comp_id, "succeeded", Some(final_asset));
    finished["warnings"] = json!(["block 1: last frame held"]);
    Mock::given(method("GET"))
        .and(path(format!("/v1/renders/{render_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(finished))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{final_asset}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(video_clip_json(final_asset, "https://files/final.mp4")),
        )
        .expect(1)
        .mount(&api)
        .await;

    let result = run_ok(
        &api,
        &[
            "render",
            "blocks",
            "--pair",
            &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
            "--pair",
            &format!("{ELEMENT_ASSET_ID}:{ASSET_ID}"),
            "--block-seconds",
            "12.5",
            "--aspect",
            "9:16",
            "--keep-video-audio",
            "--name",
            "narrated explainer",
            "--project",
            PROJECT_ID,
            "--wait",
            "--poll-interval",
            "1",
            "--json",
        ],
    )
    .stderr(predicate::str::contains(format!(
        "render {render_id} submitted (2 blocks, 12.5s each)"
    )));
    let output: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(
        output,
        json!({
            "render_id": render_id, "composition_id": comp_id, "asset_id": final_asset,
            "status": "succeeded", "url": "https://files/final.mp4",
            "warnings": ["block 1: last frame held"]
        })
    );
}

#[tokio::test]
async fn render_blocks_wait_preserves_accepted_work_and_terminal_failures() {
    for scenario in [
        "timeout",
        "slow_poll",
        "poll_error",
        "asset_error",
        "failed",
    ] {
        let api = MockServer::start().await;
        let comp_id = Uuid::new_v4();
        let render_id = Uuid::new_v4();
        let asset_id = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(path("/v1/renders/blocks"))
            .respond_with(
                ResponseTemplate::new(202)
                    .set_body_json(render_json(render_id, comp_id, "queued", None)),
            )
            .expect(1)
            .mount(&api)
            .await;
        let response = match scenario {
            "poll_error" => ResponseTemplate::new(500),
            "asset_error" => ResponseTemplate::new(200).set_body_json(render_json(
                render_id,
                comp_id,
                "succeeded",
                Some(asset_id),
            )),
            "failed" => {
                let mut failed = render_json(render_id, comp_id, "failed", None);
                failed["error"] = json!("narration is too long");
                ResponseTemplate::new(200).set_body_json(failed)
            }
            _ => {
                let response = ResponseTemplate::new(200)
                    .set_body_json(render_json(render_id, comp_id, "queued", None));
                if scenario == "slow_poll" {
                    response.set_delay(std::time::Duration::from_secs(10))
                } else {
                    response
                }
            }
        };
        Mock::given(method("GET"))
            .and(path(format!("/v1/renders/{render_id}")))
            .respond_with(response)
            .expect(1)
            .mount(&api)
            .await;
        if scenario == "asset_error" {
            Mock::given(method("GET"))
                .and(path(format!("/v1/assets/{asset_id}")))
                .respond_with(ResponseTemplate::new(500))
                .expect(1)
                .mount(&api)
                .await;
        }
        let result = cmd()
            .arg("--api-url")
            .arg(api.uri())
            .args([
                "render",
                "blocks",
                "--pair",
                &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
                "--wait",
                "--timeout",
                "1",
                "--poll-interval",
                "5",
                "--json",
            ])
            .timeout(std::time::Duration::from_secs(4))
            .assert();
        if scenario == "failed" {
            result
                .code(1)
                .stderr(predicate::str::contains("narration is too long"));
        } else {
            let result = result
                .code(75)
                .stderr(predicate::str::contains(format!(
                    "nolgia compositions status {render_id}"
                )))
                .stderr(predicate::str::contains("Error:").not())
                .stderr(predicate::str::contains("billed").not());
            let output: serde_json::Value =
                serde_json::from_slice(&result.get_output().stdout).unwrap();
            assert_eq!(output["render_id"], render_id.to_string());
            assert_eq!(
                output["outcome"],
                if matches!(scenario, "timeout" | "slow_poll") {
                    "still_running"
                } else {
                    "detached"
                }
            );
            assert_eq!(
                output["follow_up"],
                json!([format!("nolgia compositions status {render_id}")])
            );
            assert!(output.get("job_id").is_none());
            assert!(output.get("billed_twice").is_none());
        }
    }
}

#[tokio::test]
async fn render_blocks_without_wait_prints_the_render_id() {
    let api = MockServer::start().await;
    let comp_id = Uuid::new_v4();
    let render_id = Uuid::new_v4();
    Mock::given(method("POST"))
        .and(path("/v1/renders/blocks"))
        .and(body_json(json!({
            "blocks": [{"video_asset_id": ASSET_ID, "audio_asset_id": ELEMENT_ASSET_ID}],
            "block_seconds": 10.0, "aspect_ratio": "16:9", "keep_video_audio": false
        })))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(render_json(render_id, comp_id, "queued", None)),
        )
        .expect(1)
        .mount(&api)
        .await;

    run_ok(
        &api,
        &[
            "render",
            "blocks",
            "--pair",
            &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
        ],
    )
    .stdout(format!("{render_id} queued\n"))
    .stderr(predicate::str::contains(format!(
        "check it: nolgia compositions status {render_id}"
    )));
    assert_eq!(api.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn render_blocks_without_wait_reports_json_at_duration_limit() {
    let api = MockServer::start().await;
    let comp_id = Uuid::new_v4();
    let render_id = Uuid::new_v4();
    let block = json!({"video_asset_id": ASSET_ID, "audio_asset_id": ELEMENT_ASSET_ID});
    Mock::given(method("POST"))
        .and(path("/v1/renders/blocks"))
        .and(body_json(json!({
            "blocks": vec![block; 60], "block_seconds": 10.0,
            "aspect_ratio": "1:1", "keep_video_audio": false
        })))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(render_json(render_id, comp_id, "queued", None)),
        )
        .expect(1)
        .mount(&api)
        .await;

    let pair = format!("{ASSET_ID}:{ELEMENT_ASSET_ID}");
    let mut args = vec!["render", "blocks", "--json", "--aspect", "1:1"];
    for _ in 0..60 {
        args.extend(["--pair", pair.as_str()]);
    }
    let result = run_ok(&api, &args);
    let output: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(
        output,
        json!({
            "render_id": render_id, "composition_id": comp_id, "status": "queued",
            "blocks": 60, "block_seconds": 10.0, "duration_seconds": 600.0
        })
    );
    assert_eq!(api.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn render_blocks_surfaces_the_refusal_detail() {
    let api = MockServer::start().await;
    let detail = format!(
        "block 2 (video_asset_id {ELEMENT_ASSET_ID}, audio_asset_id {ASSET_ID}): the narration take is 13.20s, longer than 12.50s (1.25x the 10s block); shorten the take or raise block_seconds"
    );
    Mock::given(method("POST"))
        .and(path("/v1/renders/blocks"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "about:blank", "title": "Bad Request", "status": 400, "detail": detail
        })))
        .expect(1)
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "render",
            "blocks",
            "--pair",
            &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
            "--pair",
            &format!("{ELEMENT_ASSET_ID}:{ASSET_ID}"),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(detail));
}

#[test]
fn render_blocks_rejects_malformed_pairs_locally() {
    for pair in [
        "broken",
        "not-a-uuid:also-not",
        &format!("{ASSET_ID}:bad"),
        &format!("bad:{ASSET_ID}"),
        &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}:extra"),
    ] {
        cmd()
            .args([
                "--api-url",
                "http://127.0.0.1:1",
                "render",
                "blocks",
                "--pair",
                &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
                "--pair",
                pair,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "--pair 2 must be <video_asset_id>:<audio_asset_id> with two UUIDs",
            ));
    }
}

#[test]
fn render_blocks_rejects_invalid_block_seconds_locally() {
    for seconds in ["31", "1", "NaN", "inf"] {
        cmd()
            .args([
                "--api-url",
                "http://127.0.0.1:1",
                "render",
                "blocks",
                "--pair",
                &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
                "--block-seconds",
                seconds,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(
                "--block-seconds must be between 2 and 30",
            ));
    }
}

/// `--wait` with a zero timeout or poll interval must fail BEFORE the POST:
/// `poll_render` checks them, but by then the render and its carrier
/// composition exist and the bare argument error would read like nothing
/// happened server side.
#[test]
fn render_blocks_rejects_zero_wait_knobs_before_submitting() {
    for (flag, message) in [
        ("--timeout", "--timeout must be greater than zero"),
        (
            "--poll-interval",
            "--poll-interval must be greater than zero",
        ),
    ] {
        cmd()
            .args([
                "--api-url",
                "http://127.0.0.1:1",
                "render",
                "blocks",
                "--pair",
                &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}"),
                "--wait",
                flag,
                "0",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn render_blocks_rejects_excessive_count_and_duration_locally() {
    for (count, seconds, message) in [
        (61, "2", "--pair requires 1 to 60 blocks"),
        (21, "30", "total duration must not exceed 600 seconds"),
    ] {
        let mut command = cmd();
        command.args([
            "--api-url",
            "http://127.0.0.1:1",
            "render",
            "blocks",
            "--block-seconds",
            seconds,
        ]);
        for _ in 0..count {
            command.args(["--pair", &format!("{ASSET_ID}:{ELEMENT_ASSET_ID}")]);
        }
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

/// `compositions create --render --wait` is the "assemble and compile" path:
/// it must create a composition, upload an index.html timeline, submit a
/// render, poll it to completion, and resolve the produced asset's URL. This
/// walks that whole chain against mocks and asserts the final URL lands on
/// stdout — the finished video the caller actually wanted.
#[tokio::test]
async fn compositions_create_render_wait_resolves_final_asset() {
    let api = MockServer::start().await;
    let clip_id = Uuid::new_v4();
    let comp_id = Uuid::new_v4();
    let render_id = Uuid::new_v4();
    let final_asset = Uuid::new_v4();

    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{clip_id}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(video_clip_json(clip_id, "https://files/clip.mp4")),
        )
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/compositions"))
        .respond_with(ResponseTemplate::new(201).set_body_json(composition_json(comp_id)))
        .mount(&api)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/v1/compositions/{comp_id}/file")))
        .and(query_param("path", "index.html"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "path": "index.html", "content_type": "text/html", "size_bytes": 256,
            "updated_at": "2026-06-13T00:00:00Z"
        })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/compositions/{comp_id}/render")))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(render_json(render_id, comp_id, "queued", None)),
        )
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/renders/{render_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_json(
            render_id,
            comp_id,
            "succeeded",
            Some(final_asset),
        )))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{final_asset}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(video_clip_json(final_asset, "https://files/final.mp4")),
        )
        .mount(&api)
        .await;

    run_ok(
        &api,
        &[
            "--json",
            "compositions",
            "create",
            "--name",
            "test-comp",
            "--clip",
            &clip_id.to_string(),
            "--render",
            "--wait",
            "--poll-interval",
            "1",
        ],
    )
    .stdout(predicate::str::contains("https://files/final.mp4"));
}

/// Without `--render`, `compositions create` builds the timeline and stops,
/// printing the new composition id (no render row).
#[tokio::test]
async fn compositions_create_without_render_prints_composition_id() {
    let api = MockServer::start().await;
    let clip_id = Uuid::new_v4();
    let comp_id = Uuid::new_v4();

    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{clip_id}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(video_clip_json(clip_id, "https://files/clip.mp4")),
        )
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/compositions"))
        .respond_with(ResponseTemplate::new(201).set_body_json(composition_json(comp_id)))
        .mount(&api)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/v1/compositions/{comp_id}/file")))
        .and(query_param("path", "index.html"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "path": "index.html", "content_type": "text/html", "size_bytes": 256,
            "updated_at": "2026-06-13T00:00:00Z"
        })))
        .mount(&api)
        .await;

    run_ok(
        &api,
        &[
            "--json",
            "compositions",
            "create",
            "--name",
            "test-comp",
            "--clip",
            &clip_id.to_string(),
        ],
    )
    .stdout(predicate::str::contains(comp_id.to_string()));
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
        .stdout(predicate::str::contains("3d"))
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

/// `--character-id` binds a clip to a stored character: the id must reach the
/// wire as `character_id` (NOL-542), where the server attaches the
/// character's primary reference as an element ref and appends its canonical
/// description to the prompt.
#[tokio::test]
async fn gen_video_forwards_character_id() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({
            "prompt": "the pilot walks away from the wreck",
            "character_id": CHARACTER_ID,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--prompt",
            "the pilot walks away from the wreck",
            "--character-id",
            CHARACTER_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// The image lane's Aura identity flags must reach the wire under the spec's
/// exact field names: `face_reference_asset_id` conditions the render on a
/// face, `aura: true` asks for the character engine explicitly (NOL-542).
#[tokio::test]
async fn gen_image_forwards_face_reference_and_aura() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({
            "prompt": "portrait in the rain",
            "face_reference_asset_id": ASSET_ID,
            "aura": true,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--prompt",
            "portrait in the rain",
            "--face-reference-asset-id",
            ASSET_ID,
            "--aura",
            "true",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// An explicit `--aura false` must serialize `false` rather than be dropped
/// as "unset": the server's default is dynamic (ON for person-subject
/// prompts on compatible models), so omission and `false` mean different
/// renders. `Some(false)` is exactly the value `skip_serializing_if =
/// "Option::is_none"` keeps.
#[tokio::test]
async fn gen_image_sends_explicit_aura_false() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({ "aura": false })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--prompt",
            "a lighthouse",
            "--aura",
            "false",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// The image lane also accepts `character_id` (same identity pipeline, the
/// stored character supplies the face reference). Forward it — and refuse
/// the flag alongside `--face-reference-asset-id` at parse time, exactly as
/// the API refuses two competing identities with a 400.
#[tokio::test]
async fn gen_image_forwards_character_id_and_refuses_competing_face_reference() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({ "character_id": CHARACTER_ID })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--prompt",
            "portrait",
            "--character-id",
            CHARACTER_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));

    cmd()
        .args([
            "gen",
            "image",
            "--prompt",
            "portrait",
            "--character-id",
            CHARACTER_ID,
            "--face-reference-asset-id",
            ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

/// The restore lane (`POST /restore/video`) takes a source clip and no
/// prompt. A URL source must forward `source_url` plus the restore controls
/// verbatim; `duration_seconds` prices the job.
#[tokio::test]
async fn restore_video_submits_url_source_with_restore_controls() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({
            "model": "seedvr2-restore",
            "source_url": "https://cdn.example/clip.mp4",
            "duration_seconds": 10,
            "quality": "2160p",
            "noise_scale": 0.2,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "restore",
            "video",
            "--input",
            "https://cdn.example/clip.mp4",
            "--duration-seconds",
            "10",
            "--quality",
            "2160p",
            "--noise-scale",
            "0.2",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// The Topaz engines REQUIRE `source_fps`: their rates price the 30 fps basis
/// exactly, so an undeclared 60 fps source would be reserved at half its cost
/// and the API 400s when it is missing. The flag therefore has to exist and
/// reach the wire, or every `topaz-*` restore is unsubmittable from the CLI.
#[tokio::test]
async fn restore_video_forwards_source_fps_for_topaz_engines() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({
            "model": "topaz-proteus",
            "source_url": "https://cdn.example/clip.mp4",
            "duration_seconds": 10,
            "quality": "2160p",
            "source_fps": 60,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "restore",
            "video",
            "--model",
            "topaz-proteus",
            "--input",
            "https://cdn.example/clip.mp4",
            "--duration-seconds",
            "10",
            "--quality",
            "2160p",
            "--source-fps",
            "60",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// The Topaz engines also REQUIRE the source's pixel geometry whenever it is
/// not already stored on the asset: they derive the output frame from the
/// source's aspect ratio, so the API refuses to guess one. Without these flags
/// a `topaz-*` restore of any asset whose dimensions were never backfilled is
/// unsubmittable from the CLI, which is exactly how `--source-fps` was missed
/// one release earlier.
#[tokio::test]
async fn restore_video_forwards_source_dimensions_for_topaz_engines() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({
            "model": "topaz-proteus",
            "source_url": "https://cdn.example/clip.mp4",
            "duration_seconds": 4,
            "source_fps": 24,
            "source_width": 1344,
            "source_height": 768,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "restore",
            "video",
            "--model",
            "topaz-proteus",
            "--input",
            "https://cdn.example/clip.mp4",
            "--duration-seconds",
            "4",
            "--source-fps",
            "24",
            "--source-width",
            "1344",
            "--source-height",
            "768",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// One dimension without the other cannot describe a frame, and the API says so
/// with a 400. Refusing it in the parser keeps that round trip from happening at
/// all, and matters more for a local-file source, where the upload would
/// otherwise precede the refusal.
#[tokio::test]
async fn restore_video_requires_both_source_dimensions_together() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "restore",
            "video",
            "--model",
            "topaz-proteus",
            "--input",
            "https://cdn.example/clip.mp4",
            "--duration-seconds",
            "4",
            "--source-fps",
            "24",
            "--source-width",
            "1344",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--source-height"));
    assert!(
        api.received_requests().await.unwrap().is_empty(),
        "the refusal must happen before any API request"
    );
}

/// An asset UUID `--input` is sent as `source_asset_id` with no client-side
/// duration requirement: the server bills from the asset's stored duration.
#[tokio::test]
async fn restore_video_sends_asset_uuid_as_source_asset_id() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({
            "model": "seedvr2-restore",
            "source_asset_id": ASSET_ID,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["restore", "video", "--input", ASSET_ID, "--no-wait"],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// A raw-URL source cannot be measured server-side, so the CLI refuses it
/// without `--duration-seconds` before spending a round trip; the message
/// tells the caller what to pass.
#[tokio::test]
async fn restore_video_url_source_requires_duration_client_side() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "restore",
            "video",
            "--input",
            "https://cdn.example/clip.mp4",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--duration-seconds is required"));
    assert!(
        api.received_requests().await.unwrap().is_empty(),
        "the refusal must happen before any API request"
    );
}

/// A local file is uploaded before it can be restored, and an upload's
/// duration is probed asynchronously — so without `--duration-seconds` the
/// submission would be rejected *after* the whole file went over the wire.
/// The refusal has to land before the upload, and before any option
/// validation that would also strand an asset.
#[tokio::test]
async fn restore_video_local_file_requires_duration_before_uploading() {
    let api = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mp4");
    std::fs::write(&clip, b"not really an mp4").unwrap();

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["restore", "video", "--input"])
        .arg(&clip)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--duration-seconds is required"));
    assert!(
        api.received_requests().await.unwrap().is_empty(),
        "nothing may be uploaded before the duration check"
    );

    // Same rule for a bounded option the generated request rejects: it must
    // fail before the upload rather than leaving an orphaned asset behind.
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["restore", "video", "--input"])
        .arg(&clip)
        .args([
            "--duration-seconds",
            "10",
            "--quality",
            "a-tier-name-far-too-long-for-the-schema",
        ])
        .assert()
        .failure();
    assert!(
        api.received_requests().await.unwrap().is_empty(),
        "option validation must precede the upload"
    );
}

/// A duplicate restore must be told how to repeat *this* command. The 409
/// recovery block used to hard-code `nolgia gen ...`, which a restore caller
/// cannot run.
#[tokio::test]
async fn a_duplicate_restore_names_the_restore_command() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .respond_with(ResponseTemplate::new(409).set_body_json(duplicate_problem()))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["restore", "video", "--input", ASSET_ID, "--no-wait"])
        .assert()
        .code(EXIT_LIVE_JOB)
        .stderr(predicate::str::contains(format!(
            "already submitted — job {JOB_ID}"
        )))
        .stderr(predicate::str::contains(
            "nolgia restore video ... --idempotency-key <new-value>",
        ))
        .stderr(predicate::str::contains("nolgia gen ").not());
}

/// Restore forwards whatever `--model` the caller names (the NOL-439 rule):
/// the Topaz master upscalers reached released binaries this way, and the
/// next restore driver must work on this one the day the API serves it, with
/// no CLI re-release.
#[tokio::test]
async fn restore_video_forwards_unknown_future_model_verbatim() {
    let api = MockServer::start().await;
    let future_model = "topaz-master-upscale-added-after-this-binary";
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({ "model": future_model })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "restore",
            "video",
            "--model",
            future_model,
            "--input",
            ASSET_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// The Topaz master upscalers are siblings of `seedvr2-restore` on the same
/// command: engine in `--model`, restored output resolution in `--quality`,
/// and nothing else changes. `4320p` is a real tier on the classic engines,
/// so a two-character-longer tier string must reach the wire intact.
#[tokio::test]
async fn restore_video_submits_a_topaz_engine_at_the_8k_tier() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({
            "model": "topaz-proteus",
            "source_asset_id": ASSET_ID,
            "quality": "4320p",
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "restore",
            "video",
            "--model",
            "topaz-proteus",
            "--input",
            ASSET_ID,
            "--quality",
            "4320p",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// Which tiers an engine publishes is per-model and lives in `GET /models`,
/// so the tier ladder is the API's to enforce — `topaz-hyperion` stops at
/// 2160p while its classic siblings go to 4320p. The CLI must forward the
/// request and let the server's `400` explain, exactly as it does for every
/// other per-model capability: a client-side ladder would be a second copy of
/// the catalog, and a stale one would refuse combinations the platform runs.
#[tokio::test]
async fn restore_video_leaves_the_per_model_tier_ladder_to_the_api() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/restore/video"))
        .and(body_partial_json(json!({
            "model": "topaz-hyperion",
            "quality": "4320p",
        })))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "about:blank", "title": "Bad Request", "status": 400,
            "detail": "quality \"4320p\" is not available on topaz-hyperion; available tiers: \
                       720p, 1080p (default), 1440p (premium), 2160p (premium)"
        })))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "restore",
            "video",
            "--model",
            "topaz-hyperion",
            "--input",
            ASSET_ID,
            "--quality",
            "4320p",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "is not available on topaz-hyperion",
        ))
        .stderr(predicate::str::contains("available tiers"));
    assert!(
        !api.received_requests().await.unwrap().is_empty(),
        "the tier must be forwarded, not refused client-side against a hard-coded ladder"
    );
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

/// NOL-451 follow-up: `--cost-only` reflects the audio flag. Audio is on by
/// default, so the quote includes the surcharge and matches the headline; an
/// explicit `--generate-audio false` renders the clip silent and drops
/// `audio_surcharge` from the per-baseline rate BEFORE duration scaling — the
/// same subtract-then-scale order the server uses. R2V 4k is 778 per 5s (+48
/// audio): audio-on 10s = ceil(778*10/5) = 1556, silent 10s =
/// ceil((778-48)*10/5) = 1460.
#[tokio::test]
async fn gen_video_cost_only_drops_audio_surcharge_when_silent() {
    let api = MockServer::start().await;
    mount_video_models(&api).await;
    // Audio on by default: the surcharge stays in, matching the headline price.
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
    // Audio off: the 48-credit surcharge is subtracted before duration scaling.
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
            "--generate-audio",
            "false",
            "--cost-only",
        ],
    )
    .stdout(predicate::str::contains("1460 credits"));
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

/// A restore-lane model takes a source clip and no prompt, so the human
/// catalog has to say so: without a marker there is nothing to look for
/// unless the caller knows `--json` carries `restore: true`.
#[tokio::test]
async fn models_catalog_marks_restore_lane_models() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{
            "id": "seedvr2-restore", "modality": "video", "recommended": false, "restore": true,
            "cost": {"credits": 240, "unit": "per_clip", "baseline_seconds": 5},
            "quality": {"default": "1080p", "options": [
                {"id": "720p", "credits": 180, "premium": false},
                {"id": "1080p", "credits": 240, "premium": false},
            ]},
        }]})))
        .mount(&api)
        .await;
    // The marker sits in the capability column, so the assertion cannot be
    // satisfied by the model id alone.
    run_ok(&api, &["models", "list", "--modality", "video"])
        .stdout(predicate::str::contains("restore  720p/1080p"));
    run_ok(&api, &["models", "get", "seedvr2-restore"])
        .stdout(predicate::str::contains("supports:"))
        .stdout(predicate::str::contains("restore  720p/1080p"));
}

/// The restore lane now holds a whole family of engines whose tier ladders
/// differ, and the catalog is the only honest source for which is which: the
/// classic Topaz engines publish 4320p, the generative ones stop at 2160p.
/// `models list`/`get` must render each engine's own tiers (premium marked
/// `*`) so picking an engine and a tier needs no second lookup.
#[tokio::test]
async fn models_catalog_shows_each_topaz_engines_own_tier_ladder() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {
                "id": "topaz-proteus", "modality": "video", "recommended": false, "restore": true,
                "cost": {"credits": 12, "unit": "per_clip", "baseline_seconds": 5},
                "quality": {"default": "1080p", "options": [
                    {"id": "720p", "credits": 6, "premium": false},
                    {"id": "1080p", "credits": 12, "premium": false},
                    {"id": "1440p", "credits": 24, "premium": true},
                    {"id": "2160p", "credits": 51, "premium": true},
                    {"id": "4320p", "credits": 174, "premium": true},
                ]},
            },
            {
                "id": "topaz-hyperion", "modality": "video", "recommended": false, "restore": true,
                "cost": {"credits": 72, "unit": "per_clip", "baseline_seconds": 5},
                "quality": {"default": "1080p", "options": [
                    {"id": "720p", "credits": 72, "premium": false},
                    {"id": "1080p", "credits": 72, "premium": false},
                    {"id": "1440p", "credits": 153, "premium": true},
                    {"id": "2160p", "credits": 153, "premium": true},
                ]},
            },
        ]})))
        .mount(&api)
        .await;
    run_ok(&api, &["models", "list", "--modality", "video"])
        .stdout(predicate::str::contains(
            "restore  720p/1080p/1440p*/2160p*/4320p*",
        ))
        .stdout(predicate::str::contains(
            "restore  720p/1080p/1440p*/2160p*",
        ));
    run_ok(&api, &["models", "get", "topaz-proteus"])
        .stdout(predicate::str::contains(
            "1080p — 12 credits per 5s clip (default)",
        ))
        .stdout(predicate::str::contains(
            "4320p — 174 credits per 5s clip (premium)",
        ));
    // The generative engines never publish 8K, so the catalog must not offer
    // it on them: a tier shown here is a tier the platform will run.
    run_ok(&api, &["models", "get", "topaz-hyperion"])
        .stdout(predicate::str::contains(
            "2160p — 153 credits per 5s clip (premium)",
        ))
        .stdout(predicate::str::contains("4320p").not());
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

// NOL-1150: consent to the face identity check rides the create/update body
// only after the explicit agreement, never from an agent, and never assumed
// in a non-interactive run.
#[tokio::test]
async fn characters_create_sends_face_check_consent_after_agreement() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/characters"))
        .and(body_json(json!({
            "name": "Captain Nova",
            "reference_asset_ids": [ASSET_ID],
            "face_check_consent": true
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
            "--reference-asset-id",
            ASSET_ID,
            "--face-check-consent",
            "--yes",
        ],
    )
    .stderr(predicate::str::contains(
        "I am the person in these photos, or I have their permission to use them.",
    ))
    .stderr(predicate::str::contains(
        "https://nolgia.ai/privacy#face-check",
    ))
    .stdout(predicate::str::contains(CHARACTER_ID));
}

#[tokio::test]
async fn characters_face_check_consent_is_never_assumed_without_a_terminal() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/characters"))
        .respond_with(ResponseTemplate::new(201).set_body_json(character_json()))
        .expect(0)
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "characters",
            "create",
            "--name",
            "Captain Nova",
            "--reference-asset-id",
            ASSET_ID,
            "--face-check-consent",
        ])
        .write_stdin("yes\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("add --yes"));
}

#[tokio::test]
async fn characters_face_check_consent_refused_for_the_agent() {
    let api = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/characters/{CHARACTER_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(character_json()))
        .expect(0)
        .mount(&api)
        .await;
    cmd()
        .env("NOLGIA_SURFACE", "hermes")
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "characters",
            "update",
            CHARACTER_ID,
            "--face-check-consent",
            "--yes",
        ])
        .assert()
        .code(77)
        .stderr(predicate::str::contains(
            "an agent cannot consent to the face check",
        ));
}

#[tokio::test]
async fn characters_update_withdraws_face_check_consent() {
    let api = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(format!("/v1/characters/{CHARACTER_ID}")))
        .and(body_json(json!({"face_check_consent": false})))
        .respond_with(ResponseTemplate::new(200).set_body_json(character_json()))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "characters",
            "update",
            CHARACTER_ID,
            "--withdraw-face-check-consent",
        ],
    )
    .stdout(predicate::str::contains(CHARACTER_ID));
}

#[tokio::test]
async fn characters_list_says_when_the_face_check_needs_consent() {
    let api = MockServer::start().await;
    let mut character = character_json();
    character["face_check"] =
        json!({"enabled": false, "needs_consent": true, "consented_asset_ids": []});
    Mock::given(method("GET"))
        .and(path("/v1/characters"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"characters": [character]})))
        .mount(&api)
        .await;
    run_ok(&api, &["characters", "list"])
        .stdout(predicate::str::contains("face check off, needs consent"));
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
        .stderr(predicate::str::contains("at most 8"));
}

#[tokio::test]
async fn products_list_outputs_products() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/products"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"products": [product_json()]})),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["products", "list"])
        .stdout(predicate::str::contains(PRODUCT_ID))
        .stdout(predicate::str::contains("Trail Cup"))
        .stdout(predicate::str::contains("Acme"))
        .stdout(predicate::str::contains("29.00 USD"))
        .stdout(predicate::str::contains("1 image"));
}

#[tokio::test]
async fn products_import_sends_url_and_reports_images() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/products/import"))
        .and(body_json(
            json!({"url": "https://shop.example.com/p/trail-cup"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "product": product_json(), "images_found": 8, "images_imported": 5
        })))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &["products", "import", "https://shop.example.com/p/trail-cup"],
    )
    .stdout(predicate::str::contains(PRODUCT_ID))
    .stdout(predicate::str::contains("imported 5 of 8 images"));
}

#[tokio::test]
async fn products_import_forwards_project_id() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/products/import"))
        .and(body_json(json!({
            "url": "https://shop.example.com/p/trail-cup", "project_id": PROJECT_ID
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "product": product_json(), "images_found": 8, "images_imported": 5
        })))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "products",
            "import",
            "https://shop.example.com/p/trail-cup",
            "--project-id",
            PROJECT_ID,
        ],
    );
}

#[tokio::test]
async fn products_get_shows_reference_images() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/products/{PRODUCT_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(product_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["products", "get", PRODUCT_ID])
        .stdout(predicate::str::contains(ASSET_ID))
        .stdout(predicate::str::contains("https://files/product.png"))
        .stdout(predicate::str::contains(
            "https://shop.example.com/p/trail-cup",
        ))
        .stdout(predicate::str::contains("A blue enamel camping cup."))
        .stdout(predicate::str::contains("(primary)"));
}

#[tokio::test]
async fn products_delete_removes_product() {
    let api = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/products/{PRODUCT_ID}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&api)
        .await;
    run_ok(&api, &["products", "delete", PRODUCT_ID])
        .stdout(predicate::str::contains(format!("deleted {PRODUCT_ID}")));
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

/// The spec added a nullable `generation_limits` object to `User` (shared
/// generation concurrency). The `User` schema is `additionalProperties: false`
/// and the generated client is strict about it, so the CLI must deserialize the
/// field when present and pass it through `--json` unchanged. Text output stays
/// `id email`.
#[tokio::test]
async fn account_me_json_carries_generation_limits() {
    let api = MockServer::start().await;
    let mut user = user_json();
    user["generation_limits"] = json!({"concurrent_max": 4, "concurrent_active": 1});
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(user))
        .mount(&api)
        .await;
    let out = run_ok(&api, &["--json", "account", "me"])
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(value["email"], "ada@nolgia.ai");
    assert_eq!(value["generation_limits"]["concurrent_max"], 4);
    assert_eq!(value["generation_limits"]["concurrent_active"], 1);
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

/// NOL-1213: `--expires-in-days` reaches the API, and the created token's
/// expiry is printed next to the one-time plaintext.
#[tokio::test]
async fn pat_create_sends_expiry_and_prints_it() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pat"))
        .and(body_json(json!({"name": "ci-bot", "expires_in_days": 30})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "pat": pat_json(),
            "token": "nol_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6"
        })))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "pat",
            "create",
            "--name",
            "ci-bot",
            "--expires-in-days",
            "30",
        ],
    )
    .stdout(predicate::str::contains(
        "\nexpires 2099-06-13T00:00:00+00:00\ntoken: ",
    ));
}

/// Without the flag the body carries no expiry: the server applies its
/// 365-day default.
#[tokio::test]
async fn pat_create_without_expiry_leaves_the_default_to_the_server() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pat"))
        .and(body_json(json!({"name": "ci-bot"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "pat": pat_json(),
            "token": "nol_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6"
        })))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["pat", "create", "--name", "ci-bot"]);
}

#[test]
fn pat_create_refuses_an_expiry_outside_1_to_365() {
    for days in ["0", "366"] {
        Command::cargo_bin("nolgia")
            .unwrap()
            .args([
                "pat",
                "create",
                "--name",
                "ci-bot",
                "--expires-in-days",
                days,
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("expires-in-days"));
    }
}

/// `nolgia pat list` shows each token's expiry: a date, EXPIRED, or never for
/// a token created before tokens expired.
#[tokio::test]
async fn pat_list_shows_expiry_states() {
    let api = MockServer::start().await;
    let mut expired = pat_json();
    expired["name"] = json!("old-ci");
    expired["expires_at"] = json!("2020-01-01T00:00:00Z");
    let mut legacy = pat_json();
    legacy["name"] = json!("legacy");
    legacy["expires_at"] = json!(null);
    Mock::given(method("GET"))
        .and(path("/v1/pat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items": [pat_json(), expired, legacy]})),
        )
        .mount(&api)
        .await;
    run_ok(&api, &["pat", "list"])
        .stdout(predicate::str::contains(
            "expires 2099-06-13T00:00:00+00:00",
        ))
        .stdout(predicate::str::contains(
            "EXPIRED 2020-01-01T00:00:00+00:00",
        ))
        .stdout(predicate::str::contains(
            "expires never (created before tokens expired; rotate recommended)",
        ));
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

/// `auth login --no-browser` against a code that is already out of time:
/// the link leads, the code sits on its own line, the waiting line prints
/// once (stdout is a pipe here), and the failure says the code expired and
/// how to get a new one. No browser is opened and no token poll is made.
#[tokio::test]
async fn auth_login_no_browser_prints_link_and_code_then_reports_expiry() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dev-1",
            "user_code": "YKKQ-RXKS",
            "verification_uri": "https://nolgia.ai/device",
            "verification_uri_complete": "https://nolgia.ai/device?code=YKKQ-RXKS",
            "expires_in": 0,
            "interval": 1
        })))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/device/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "expired_token" })))
        .expect(0)
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["auth", "login", "--no-browser"])
        .assert()
        .failure()
        .stdout(predicate::eq(
            "Open: https://nolgia.ai/device?code=YKKQ-RXKS\n\
             \n\
             \x20 Code: YKKQ-RXKS\n\
             \n\
             Waiting for you to approve in the browser... (expires in 0:00)\n",
        ))
        .stderr(predicate::str::contains("expired before it was approved"))
        .stderr(predicate::str::contains("run `nolgia auth login`"));
    api.verify().await;
}

/// `--json` keeps stdout for the document: the narration moves to stderr.
#[tokio::test]
async fn auth_login_json_keeps_stdout_clean_of_narration() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/device"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dev-1",
            "user_code": "YKKQ-RXKS",
            "verification_uri": "https://nolgia.ai/device",
            "verification_uri_complete": "https://nolgia.ai/device?code=YKKQ-RXKS",
            "expires_in": 0,
            "interval": 1
        })))
        .mount(&api)
        .await;

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["--json", "auth", "login", "--no-browser"])
        .assert()
        .failure()
        .stdout(predicate::eq(""))
        .stderr(predicate::str::contains("Code: YKKQ-RXKS"));
}

#[test]
fn auth_login_help_documents_no_browser() {
    cmd()
        .args(["auth", "login", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-browser"));
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

/// The install POST must carry an explicit `{}` body and a `Content-Length`
/// header. The spec declares no request body for the operation, so the
/// generated builder sent a bodyless POST with no `Content-Length` — which
/// the production load balancer rejects with `411 Length Required` before
/// the API ever sees it (NOL-542). The matchers here fail the test if either
/// the empty-object body or the header ever goes missing again.
#[tokio::test]
async fn ability_install_sends_empty_json_body_with_content_length() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/agent/abilities/nolgia-cli-basics"))
        .and(body_json(json!({})))
        .and(header("content-length", "2"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "slug": "nolgia-cli-basics", "name": "NOLGIA CLI Basics", "description": "d",
            "latest_version": "1.0.0", "installed_at": "2026-06-13T00:00:00Z"
        })))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["ability", "install", "nolgia-cli-basics"]).stdout(predicate::str::contains(
        "installed nolgia-cli-basics v1.0.0",
    ));
}

/// A refused install (not entitled, unknown slug, already installed) must
/// surface the server's RFC 7807 `detail`, not a raw HTTP error dump — the
/// raw-request 411 fix must not regress error legibility.
#[tokio::test]
async fn ability_install_renders_problem_detail_on_refusal() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/agent/abilities/pro-only-ability"))
        .respond_with(ResponseTemplate::new(402).set_body_json(json!({
            "title": "Payment Required",
            "detail": "ability pro-only-ability requires the pro plan",
        })))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["ability", "install", "pro-only-ability"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "ability pro-only-ability requires the pro plan",
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

/// The sanitizer's verdict on the ellipse starter with two authoring slips:
/// an out-of-range `x` (clamped) and a key the contract does not know
/// (dropped). `mask` is the canonical sparse result, not the input.
fn mask_verdict_json() -> serde_json::Value {
    json!({
        "mask": {"shape": "ellipse", "x": 200, "y": 40, "width": 60, "height": 45, "feather": 30},
        "identity": false,
        "problems": [
            {"path": "x", "message": "clamped from 250 to 200 (range -100..200)"},
            {"path": "softness", "message": "unknown field dropped"}
        ]
    })
}

fn mount_validate_mask(
    api: &MockServer,
    expected_body: serde_json::Value,
    verdict: serde_json::Value,
) -> impl std::future::Future<Output = ()> + '_ {
    Mock::given(method("POST"))
        .and(path("/v1/masks:validate"))
        .and(body_json(expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(verdict))
        .mount(api)
}

#[tokio::test]
async fn masks_validate_prints_canonical_mask_then_problems() {
    let api = MockServer::start().await;
    let candidate = json!({"shape": "ellipse", "x": 250, "y": 40, "width": 60, "height": 45,
                           "feather": 30, "softness": 3});
    mount_validate_mask(&api, json!({"mask": candidate}), mask_verdict_json()).await;
    let assert = run_ok(&api, &["masks", "validate", &candidate.to_string()]);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // The canonical mask, in contract order, with whole numbers as integers.
    let mask_end = stdout.find("\n}\n").expect("pretty mask object") + 3;
    let mask: serde_json::Value = serde_json::from_str(&stdout[..mask_end]).unwrap();
    assert_eq!(
        mask,
        json!({"shape": "ellipse", "x": 200, "y": 40, "width": 60, "height": 45, "feather": 30})
    );
    assert!(
        stdout.contains("\"shape\": \"ellipse\",\n  \"x\": 200,"),
        "{stdout}"
    );
    assert_eq!(
        &stdout[mask_end..],
        "  x: clamped from 250 to 200 (range -100..200)\n  softness: unknown field dropped\n"
    );
}

#[tokio::test]
async fn masks_validate_without_strict_exits_zero_despite_problems() {
    let api = MockServer::start().await;
    mount_validate_mask(&api, json!({"mask": {"x": 250}}), mask_verdict_json()).await;
    run_ok(&api, &["masks", "validate", r#"{"x": 250}"#])
        .stdout(predicate::str::contains("softness: unknown field dropped"));
}

#[tokio::test]
async fn masks_validate_strict_exits_one_on_problems() {
    let api = MockServer::start().await;
    mount_validate_mask(&api, json!({"mask": {"x": 250}}), mask_verdict_json()).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["masks", "validate", "--strict", r#"{"x": 250}"#])
        .assert()
        .code(1)
        // The verdict is still printed in full before the gate trips.
        .stdout(predicate::str::contains("\"shape\": \"ellipse\""))
        .stdout(predicate::str::contains("x: clamped from 250 to 200"))
        .stderr(predicate::str::contains("mask has 2 problems (--strict)"));
}

#[tokio::test]
async fn masks_validate_strict_passes_a_clean_mask() {
    let api = MockServer::start().await;
    let mask = json!({"shape": "ellipse", "width": 60});
    mount_validate_mask(
        &api,
        json!({"mask": mask}),
        json!({"mask": mask, "identity": false, "problems": []}),
    )
    .await;
    run_ok(&api, &["masks", "validate", "--strict", &mask.to_string()]).stdout(predicate::eq(
        "{\n  \"shape\": \"ellipse\",\n  \"width\": 60\n}\n",
    ));
}

#[tokio::test]
async fn masks_validate_identity_renders_null_as_a_clear() {
    let api = MockServer::start().await;
    mount_validate_mask(
        &api,
        json!({"mask": {}}),
        json!({"mask": null, "identity": true, "problems": []}),
    )
    .await;
    run_ok(&api, &["masks", "validate", "{}"])
        .stdout(predicate::eq("null (identity — clears an authored mask)\n"));
}

#[tokio::test]
async fn masks_validate_undrawable_renders_null_and_names_the_mask() {
    let api = MockServer::start().await;
    mount_validate_mask(
        &api,
        json!({"mask": {"shape": "star"}}),
        json!({"mask": null, "identity": false, "problems": [
            {"path": "shape", "message": "unknown shape \"star\" — the mask will not be drawn"},
            {"path": "", "message": "not a drawable mask"}
        ]}),
    )
    .await;
    run_ok(&api, &["masks", "validate", r#"{"shape": "star"}"#])
        .stdout(predicate::str::starts_with("null (not a drawable mask)\n"))
        .stdout(predicate::str::contains("  shape: unknown shape \"star\""))
        .stdout(predicate::str::contains("  (mask): not a drawable mask\n"));
}

#[tokio::test]
async fn masks_validate_reads_the_mask_from_an_at_file() {
    let api = MockServer::start().await;
    let candidate =
        json!({"shape": "polygon", "points": [[0, 0], [100, 0], [50, 100]], "feather": 8});
    let verdict = json!({"mask": candidate, "identity": false, "problems": []});
    mount_validate_mask(&api, json!({"mask": candidate}), verdict).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mask.json");
    std::fs::write(&file, serde_json::to_string_pretty(&candidate).unwrap()).unwrap();
    run_ok(
        &api,
        &["masks", "validate", &format!("@{}", file.display())],
    )
    .stdout(predicate::str::contains(
        "\"points\": [\n    [\n      0,\n      0\n    ],",
    ));
}

#[tokio::test]
async fn masks_validate_reads_the_mask_from_stdin() {
    let api = MockServer::start().await;
    let candidate = json!({"shape": "ellipse", "feather": 12.5});
    let verdict = json!({"mask": candidate, "identity": false, "problems": []});
    mount_validate_mask(&api, json!({"mask": candidate}), verdict).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["masks", "validate", "-"])
        .write_stdin(candidate.to_string())
        .assert()
        .success()
        .stdout(predicate::eq(
            "{\n  \"shape\": \"ellipse\",\n  \"feather\": 12.5\n}\n",
        ));
}

#[tokio::test]
async fn masks_validate_forwards_non_object_junk_for_diagnosis() {
    let api = MockServer::start().await;
    mount_validate_mask(
        &api,
        json!({"mask": "ellipse"}),
        json!({"mask": null, "identity": false, "problems": [
            {"path": "", "message": "not a mask: expected an object, got a string"}
        ]}),
    )
    .await;
    run_ok(&api, &["masks", "validate", "\"ellipse\""]).stdout(predicate::str::contains(
        "(mask): not a mask: expected an object, got a string",
    ));
}

#[tokio::test]
async fn masks_validate_json_prints_the_verdict() {
    let api = MockServer::start().await;
    mount_validate_mask(&api, json!({"mask": {"x": 250}}), mask_verdict_json()).await;
    let assert = run_ok(&api, &["--json", "masks", "validate", r#"{"x": 250}"#]);
    let verdict: serde_json::Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout is one JSON document");
    assert_eq!(verdict, mask_verdict_json());
}

#[tokio::test]
async fn masks_validate_json_strict_still_prints_the_verdict_before_failing() {
    let api = MockServer::start().await;
    mount_validate_mask(&api, json!({"mask": {"x": 250}}), mask_verdict_json()).await;
    let assert = cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["--json", "masks", "validate", "--strict", r#"{"x": 250}"#])
        .assert()
        .code(1);
    let verdict: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(verdict["problems"].as_array().unwrap().len(), 2);
}

#[test]
fn masks_validate_rejects_unparseable_json_before_any_request() {
    cmd()
        .args([
            "--api-url",
            "http://127.0.0.1:9",
            "masks",
            "validate",
            "{not json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("parsing mask JSON from <MASK>"));
}

#[tokio::test]
async fn masks_validate_surfaces_server_problem_detail_verbatim() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/masks:validate"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "https://nolgia.ai/errors/bad-request",
            "title": "Bad request",
            "status": 400,
            "detail": "body must be an object with a \"mask\" key"
        })))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["masks", "validate", "{}"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("validating mask: 400"))
        .stderr(predicate::str::contains(
            "body must be an object with a \"mask\" key",
        ));
}

/// `masks example` is offline: no login, no request. The API URL points at a
/// closed port so any attempt to build a request would fail loudly.
#[test]
fn masks_example_prints_contract_true_starters_offline() {
    for (shape, expected) in [
        (
            "rectangle",
            json!({"x": 75, "y": 22, "width": 40, "height": 24,
                   "cornerRadius": 24, "feather": 2}),
        ),
        (
            "ellipse",
            json!({"shape": "ellipse", "y": 40, "width": 60, "height": 45, "feather": 30}),
        ),
        (
            "polygon",
            json!({"shape": "polygon", "points": [[0, 0], [100, 0], [50, 100]], "feather": 8}),
        ),
    ] {
        let assert = cmd()
            .args(["--api-url", "http://127.0.0.1:9", "masks", "example", shape])
            .assert()
            .success()
            .stdout(predicate::str::contains("nolgia masks validate"));
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        let (mask_text, _hint) = stdout.split_once("\n\n").expect("mask, blank line, hint");
        let mask: serde_json::Value = serde_json::from_str(mask_text).unwrap();
        assert_eq!(mask, expected, "`nolgia masks example {shape}`");
        if shape != "rectangle" {
            assert!(
                mask_text.starts_with(&format!("{{\n  \"shape\": \"{shape}\",")),
                "{mask_text}"
            );
        }

        // `--json` is the bare starter, with no hint to strip.
        let assert = cmd()
            .args([
                "--json",
                "--api-url",
                "http://127.0.0.1:9",
                "masks",
                "example",
                shape,
            ])
            .assert()
            .success();
        let mask: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        assert_eq!(mask, expected);
    }
}

#[test]
fn masks_example_rejects_unknown_shapes() {
    cmd()
        .args(["masks", "example", "star"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rectangle"))
        .stderr(predicate::str::contains("ellipse"))
        .stderr(predicate::str::contains("polygon"));
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
    command.env_remove("HERMES_HOME");
    command.env_remove("HERMES_DASHBOARD");
    command.env_remove("NOLGIA_SURFACE");
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
            "cost": {"credits": 165, "unit": "per_clip", "baseline_seconds": 5, "audio_surcharge": 11},
            "video": {"min_duration": 2, "max_duration": 15, "aspect_ratios": ["16:9", "9:16"], "image_input": false},
            "quality": {"default": "720p", "options": [
                {"id": "720p", "credits": 165, "premium": false, "audio_surcharge": 11},
                {"id": "1080p", "credits": 360, "premium": false, "audio_surcharge": 14},
                {"id": "4k", "credits": 778, "premium": true, "audio_surcharge": 48},
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

fn product_json() -> serde_json::Value {
    let mut image = asset_json("https://files/product.png");
    image["id"] = json!(ASSET_ID);
    json!({
        "id": PRODUCT_ID, "user_id": USER_ID, "name": "Trail Cup",
        "source_url": "https://shop.example.com/p/trail-cup",
        "description": "Enamel camping cup", "canonical_description": "A blue enamel camping cup.",
        "price": "29.00 USD", "brand": "Acme",
        "primary_image_asset_id": ASSET_ID, "images": [image],
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

fn composition_json(id: Uuid) -> serde_json::Value {
    json!({
        "id": id, "user_id": USER_ID, "name": "test-comp",
        "description": "", "meta": {},
        "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
    })
}

fn render_json(
    id: Uuid,
    composition_id: Uuid,
    status: &str,
    asset_id: Option<Uuid>,
) -> serde_json::Value {
    json!({
        "id": id, "composition_id": composition_id, "user_id": USER_ID,
        "status": status, "params": {}, "warnings": [], "asset_id": asset_id,
        "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z"
    })
}

/// A clip asset the composition timeline references (video with a duration).
fn video_clip_json(id: Uuid, url: &str) -> serde_json::Value {
    json!({
        "id": id, "user_id": USER_ID, "modality": "video", "model": "fal-ai/kling-video/v3/text-to-video",
        "signed_url": url, "duration_seconds": 5.0,
        "expires_at": "2026-06-13T00:00:00Z", "created_at": "2026-06-13T00:00:00Z"
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
        "created_at": "2026-06-13T00:00:00Z", "last_used_at": null, "revoked_at": null,
        "expires_at": "2099-06-13T00:00:00Z"
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

// --- masked image edits (nolgia-api#402) ------------------------------------

/// A 1x1 PNG at the given colour type, built by hand so the tests can assert
/// on the exact bytes the mask contract reads: the IHDR width, height and
/// colour type. Colour type 6 is RGBA (a real per-pixel alpha channel, what a
/// mask must be); 2 is truecolour with none.
fn png_fixture(width: u32, height: u32, colour_type: u8) -> Vec<u8> {
    fn chunk(tag: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let body: Vec<u8> = tag.iter().chain(data.iter()).copied().collect();
        let mut crc = 0xffff_ffff_u32;
        for byte in &body {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        out.extend(body);
        out.extend((crc ^ 0xffff_ffff).to_be_bytes());
        out
    }
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, colour_type, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &ihdr));
    // No IDAT: nothing decodes these, and the contract is read from IHDR.
    png.extend(chunk(b"IEND", &[]));
    png
}

fn image_models_json(inpaint_mask: bool) -> serde_json::Value {
    json!({"models": [
        {
            "id": "gpt-image-2", "modality": "image", "recommended": true,
            "cost": {"credits": 22, "unit": "per_image"},
            "image": {"aspect_ratios": ["1:1", "16:9"], "reference_images_max": 4, "inpaint_mask": inpaint_mask},
        },
        {
            "id": "nano-banana-2", "modality": "image", "recommended": false,
            "cost": {"credits": 10, "unit": "per_image"},
            "image": {"aspect_ratios": ["1:1"], "reference_images_max": 1, "inpaint_mask": false},
        },
    ]})
}

/// The happy path: --input becomes `reference_asset_ids` (the id, so the
/// server re-signs it after a backlog) and --mask becomes `mask_asset_id`.
/// Both are asset UUIDs here, so nothing is uploaded.
#[tokio::test]
async fn gen_image_forwards_mask_and_reference_asset_ids() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_models_json(true)))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({
            "model": "gpt-image-2",
            "reference_asset_ids": [ASSET_ID],
            "mask_asset_id": ELEMENT_ASSET_ID,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "a fern on the desk",
            "--input",
            ASSET_ID,
            "--mask",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// A mask on a model whose catalog entry says it cannot take one is refused
/// before the request. The API refuses it too, before any hold, so this saves
/// no money — it saves the round trip, and the message names models that can.
#[tokio::test]
async fn gen_image_refuses_mask_on_a_model_without_the_capability() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_models_json(true)))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "nano-banana-2",
            "--prompt",
            "a fern",
            "--input",
            ASSET_ID,
            "--mask",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot edit part of an image"))
        .stderr(predicate::str::contains("gpt-image-2"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(
            request.url.path(),
            "/v1/generate/image",
            "a mask on an incapable model reached the API"
        );
    }
}

/// A mask with no alpha channel is refused on the bytes, before either file is
/// uploaded: the transparent pixels ARE the region to repaint, so a flat
/// black-and-white PNG would repaint everything.
#[tokio::test]
async fn gen_image_refuses_a_mask_with_no_alpha_channel() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_models_json(true)))
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mask = dir.path().join("mask.png");
    std::fs::write(&mask, png_fixture(64, 64, 2)).unwrap();

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "a fern",
            "--input",
            ASSET_ID,
            "--mask",
            mask.to_str().unwrap(),
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no alpha channel"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(
            request.url.path(),
            "/v1/assets",
            "the mask was uploaded before it was checked"
        );
    }
}

/// A mask that is not a PNG at all gets the format message rather than the
/// alpha one — the fix is different, so the wording has to be.
#[tokio::test]
async fn gen_image_refuses_a_mask_that_is_not_a_png() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_models_json(true)))
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let mask = dir.path().join("mask.png");
    std::fs::write(&mask, b"\xff\xd8\xff\xe0not a png at all, just jpeg bytes").unwrap();

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "a fern",
            "--input",
            ASSET_ID,
            "--mask",
            mask.to_str().unwrap(),
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a PNG"));
}

/// Dimensions are compared client-side when both files are local, because
/// that is the mistake a mask painter makes and the one the operator can fix
/// instantly. Both uploads are skipped.
#[tokio::test]
async fn gen_image_refuses_a_mask_that_does_not_match_the_image() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(image_models_json(true)))
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("desk.png");
    let mask = dir.path().join("mask.png");
    std::fs::write(&image, png_fixture(1024, 1024, 6)).unwrap();
    std::fs::write(&mask, png_fixture(512, 512, 6)).unwrap();

    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "a fern",
            "--input",
            image.to_str().unwrap(),
            "--mask",
            mask.to_str().unwrap(),
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("512x512"))
        .stderr(predicate::str::contains("1024x1024"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(
            request.url.path(),
            "/v1/assets",
            "a file was uploaded for a request that could never have been submitted"
        );
    }
}

/// A mask needs something to paint over. clap enforces it at parse time, so
/// the refusal costs no network at all.
#[test]
fn gen_image_mask_requires_an_input() {
    cmd()
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "a fern",
            "--mask",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--input"));
}

// --- reference audio / lip sync (nolgia-cli#168) ----------------------------

const LIP_SYNC_MODEL: &str = "heygen-avatar-iv";

fn lip_sync_models_json() -> serde_json::Value {
    json!({"models": [
        {
            "id": LIP_SYNC_MODEL, "modality": "video", "recommended": false,
            "cost": {"credits": 120, "unit": "per_clip", "baseline_seconds": 5},
            "video": {"min_duration": 2, "max_duration": 60, "aspect_ratios": ["16:9", "9:16"], "image_input": true},
            "references": {
                "start_frame": true, "start_frame_required": true, "end_frame": false,
                "video_refs_max": 0, "element_refs_max": 0,
                "audio_refs_max": 1, "audio_refs_min": 1,
            },
        },
        {
            "id": "seedance-2.5", "modality": "video", "recommended": true,
            "cost": {"credits": 90, "unit": "per_clip", "baseline_seconds": 5},
            "video": {"min_duration": 3, "max_duration": 15, "aspect_ratios": ["16:9"], "image_input": true},
            "references": {
                "start_frame": true, "start_frame_required": false, "end_frame": true,
                "video_refs_max": 0, "element_refs_max": 4, "audio_refs_max": 0,
            },
        },
    ]})
}

/// The lip sync path end to end: a portrait as --input and a voice track as
/// --audio-ref, which lands on the wire as `audio_asset_ids`. Nothing else
/// reaches that field from the CLI, which is why the whole capability was
/// unreachable (#168).
#[tokio::test]
async fn gen_video_forwards_audio_refs_as_audio_asset_ids() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lip_sync_models_json()))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{ASSET_ID}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(asset_json("https://files/portrait.png")),
        )
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({
            "model": LIP_SYNC_MODEL,
            "audio_asset_ids": [ELEMENT_ASSET_ID],
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
            LIP_SYNC_MODEL,
            "--prompt",
            "she reads the line to camera",
            "--input",
            ASSET_ID,
            "--audio-ref",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// A model that takes no reference audio says so by name, rather than letting
/// the caller find out from a 400.
#[tokio::test]
async fn gen_video_refuses_audio_ref_on_a_model_without_one() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lip_sync_models_json()))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "video",
            "--model",
            "seedance-2.5",
            "--prompt",
            "a wind chime",
            "--audio-ref",
            ELEMENT_ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("takes no reference audio"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(request.url.path(), "/v1/generate/video");
    }
}

/// A lip sync model cannot render without a voice track, and an ABSENCE is
/// invisible to a "did the caller pass a flag" gate — so the precheck runs
/// unconditionally and the message says what to pass, plus the one thing that
/// surprises people: the clip's length comes from the audio.
#[tokio::test]
async fn gen_video_requires_an_audio_ref_on_a_lip_sync_model() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lip_sync_models_json()))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "video",
            "--model",
            LIP_SYNC_MODEL,
            "--prompt",
            "she reads the line to camera",
            "--input",
            ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--audio-ref"))
        .stderr(predicate::str::contains("--duration-seconds"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(request.url.path(), "/v1/generate/video");
    }
}

/// A local voice track is uploaded first and only its ASSET ID is sent: the
/// API bills a lip sync clip on the track's stored duration, so it refuses a
/// raw URL, and uploading is what makes the flag usable from a shell.
#[tokio::test]
async fn gen_video_uploads_a_local_audio_ref_and_sends_its_asset_id() {
    let api = MockServer::start().await;
    let uploaded = Uuid::new_v4();
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(lip_sync_models_json()))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/assets/{ASSET_ID}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(asset_json("https://files/portrait.png")),
        )
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/assets/uploads"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "asset_id": uploaded,
            "upload_id": uploaded,
            "upload_url": format!("{}/signed-put", api.uri()),
            "expires_at": "2030-01-01T00:00:00Z",
        })))
        .mount(&api)
        .await;
    Mock::given(method("PUT"))
        .and(path("/signed-put"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/assets/uploads/{uploaded}/complete")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": uploaded,
            "user_id": USER_ID,
            "modality": "audio",
            "model": "user-upload",
            "status": "ready",
            "signed_url": "https://files/voice.mp3",
            "created_at": "2026-09-10T00:00:00Z",
            "expires_at": "2030-01-01T00:00:00Z",
            "favorite": false,
        })))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({ "audio_asset_ids": [uploaded] })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let voice = dir.path().join("voice.mp3");
    std::fs::write(
        &voice,
        b"not really mp3, but the upload path only reads bytes",
    )
    .unwrap();

    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--model",
            LIP_SYNC_MODEL,
            "--prompt",
            "she reads the line to camera",
            "--input",
            ASSET_ID,
            "--audio-ref",
            voice.to_str().unwrap(),
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

// --- render quality, the second image axis (nolgia-api#416) -----------------

fn render_quality_models_json() -> serde_json::Value {
    json!({"models": [
        {
            "id": "gpt-image-2.5-sunburst", "modality": "image", "recommended": true,
            "cost": {"credits": 8, "unit": "per_image"},
            "image": {
                "aspect_ratios": ["1:1", "16:9"], "reference_images_max": 4, "inpaint_mask": true,
                "render_quality": {"default": "auto", "options": [
                    {"id": "auto", "credits_added": 0, "premium": false},
                    {"id": "low", "credits_added": 0, "premium": false},
                    {"id": "medium", "credits_added": 0, "premium": false},
                    {"id": "high", "credits_added": 0, "premium": false},
                    {"id": "xhigh", "credits_added": 11, "premium": true},
                    {"id": "max", "credits_added": 24, "premium": true},
                ]},
            },
            "quality": {"default": "native", "options": [
                {"id": "native", "credits": 8, "premium": false},
                {"id": "4k", "credits": 58, "premium": true},
            ]},
        },
        {
            "id": "gpt-image-2", "modality": "image", "recommended": false,
            "cost": {"credits": 22, "unit": "per_image"},
            "image": {
                "aspect_ratios": ["1:1"], "reference_images_max": 4, "inpaint_mask": true,
                "render_quality": {"default": "auto", "options": [
                    {"id": "auto", "credits_added": 0, "premium": false},
                    {"id": "low", "credits_added": 0, "premium": false},
                    {"id": "medium", "credits_added": 0, "premium": false},
                    {"id": "high", "credits_added": 0, "premium": false},
                ]},
            },
        },
        {
            "id": "nano-banana-2", "modality": "image", "recommended": false,
            "cost": {"credits": 4, "unit": "per_image"},
            "image": {"aspect_ratios": ["1:1"], "reference_images_max": 1, "inpaint_mask": false},
        },
    ]})
}

/// The value reaches the wire under the spec's field name, beside — not
/// instead of — the upscale tier. The two axes compose, and a request carrying
/// both is the case that proves they are separate fields.
#[tokio::test]
async fn gen_image_forwards_render_quality_alongside_quality() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_quality_models_json()))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({
            "model": "gpt-image-2.5-sunburst",
            "quality": "4k",
            "render_quality": "max",
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--model",
            "gpt-image-2.5-sunburst",
            "--prompt",
            "a fern",
            "--quality",
            "4k",
            "--render-quality",
            "max",
            "--no-wait",
        ],
    )
    // The adder is announced before the spend, and it is per IMAGE.
    .stderr(predicate::str::contains("+24 credits per image"));
}

/// An omitted flag sends NOTHING. `auto` is the model's own choice and the
/// state every published base price was measured against, so an ordinary
/// request must stay byte-identical to what it always was.
#[tokio::test]
async fn gen_image_omits_render_quality_when_not_asked_for() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_quality_models_json()))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--model",
            "gpt-image-2.5-sunburst",
            "--prompt",
            "a fern",
            "--no-wait",
        ],
    );

    let submits: Vec<_> = api
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/generate/image")
        .collect();
    assert_eq!(submits.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&submits[0].body).unwrap();
    assert!(
        body.get("render_quality").is_none(),
        "an omitted --render-quality must put no field on the wire: {body}"
    );
}

/// A free value is accepted and says nothing about cost, because there is no
/// cost to announce. low/medium/high buy speed, not savings.
#[tokio::test]
async fn gen_image_announces_no_adder_for_a_free_render_quality() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_quality_models_json()))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({ "render_quality": "low" })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--model",
            "gpt-image-2.5-sunburst",
            "--prompt",
            "a fern",
            "--render-quality",
            "low",
            "--no-wait",
        ],
    )
    .stderr(predicate::str::contains("credits per image").not());
}

/// xhigh and max exist only on the 2.5 pair. On an older id the CLI refuses
/// before the request and lists what that model does take, rather than letting
/// the API 400 after a round trip.
#[tokio::test]
async fn gen_image_refuses_a_render_quality_the_model_does_not_publish() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_quality_models_json()))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--prompt",
            "a fern",
            "--render-quality",
            "xhigh",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not available on gpt-image-2"))
        .stderr(predicate::str::contains("high"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(request.url.path(), "/v1/generate/image");
    }
}

/// A model with no ladder at all refuses ANY value, and says where the axis
/// lives instead of failing generically.
#[tokio::test]
async fn gen_image_refuses_render_quality_on_a_model_without_the_axis() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_quality_models_json()))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "nano-banana-2",
            "--prompt",
            "a fern",
            "--render-quality",
            "max",
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no render-quality ladder"));
    for request in api.received_requests().await.unwrap() {
        assert_ne!(request.url.path(), "/v1/generate/image");
    }
}

/// `models get` prints the ladder with the per-image adder, so the estimate is
/// available without submitting anything. It is printed as its own block, not
/// merged into the quality tiers: a tier's credits are the whole price, an
/// adder is what it puts on top, and one list would invite adding them wrong.
#[tokio::test]
async fn models_get_prints_the_render_quality_ladder_with_its_adders() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(render_quality_models_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["models", "get", "gpt-image-2.5-sunburst"])
        .stdout(predicate::str::contains("render quality:"))
        .stdout(predicate::str::contains(
            "auto — no extra credits (default)",
        ))
        .stdout(predicate::str::contains(
            "xhigh — +11 credits per image (premium)",
        ))
        .stdout(predicate::str::contains(
            "max — +24 credits per image (premium)",
        ));
}

// --- Camera-move library (NOL-864) --------------------------------------------

fn motions_json() -> serde_json::Value {
    json!({
        "motions": [
            {"id": "push-in", "name": "Push-in", "category": "dolly",
             "description": "The camera moves toward the subject, tightening the frame.",
             "default_strength": "medium", "preview_url": null,
             "strengths": [
                {"strength": "subtle", "prompt_fragment": "Camera: a gentle push-in."},
                {"strength": "medium", "prompt_fragment": "Camera: a steady push-in."},
                {"strength": "strong", "prompt_fragment": "Camera: a fast push-in."}
             ]},
            {"id": "orbit-left", "name": "Orbit left", "category": "orbit",
             "description": "The camera circles the subject counterclockwise.",
             "default_strength": "medium", "preview_url": null,
             "strengths": [
                {"strength": "subtle", "prompt_fragment": "Camera: a shallow orbit left."},
                {"strength": "medium", "prompt_fragment": "Camera: a smooth orbit left."},
                {"strength": "strong", "prompt_fragment": "Camera: a sweeping orbit left."}
             ]}
        ]
    })
}

#[tokio::test]
async fn motions_list_outputs_catalog_table() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/motions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(motions_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["motions", "list"])
        .stdout(predicate::str::contains("push-in"))
        .stdout(predicate::str::contains("Orbit left"))
        .stdout(predicate::str::contains(
            "The camera circles the subject counterclockwise.",
        ))
        .stdout(predicate::str::contains("gen video --motion <id>"));
}

#[tokio::test]
async fn motions_list_json_carries_every_strength_fragment() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/motions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(motions_json()))
        .mount(&api)
        .await;
    run_ok(&api, &["--json", "motions", "list"])
        .stdout(predicate::str::contains("\"id\": \"orbit-left\""))
        .stdout(predicate::str::contains("\"strength\": \"strong\""))
        .stdout(predicate::str::contains("Camera: a sweeping orbit left."))
        .stdout(predicate::str::contains("\"preview_url\": null"));
}

/// `--motion` / `--motion-strength` ride the request as `motion_id` /
/// `motion_strength`; the server does the prompt append, so the prompt is
/// sent exactly as typed.
#[tokio::test]
async fn gen_video_motion_flags_ride_the_request() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(body_partial_json(json!({
            "prompt": "a rocket on the pad",
            "motion_id": "orbit-left",
            "motion_strength": "strong"
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "video",
            "--prompt",
            "a rocket on the pad",
            "--motion",
            "orbit-left",
            "--motion-strength",
            "strong",
            "--no-wait",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
}

/// Without the flags nothing about the request changes: neither field is
/// sent (the API's own default strength applies only to a named move).
#[tokio::test]
async fn gen_video_without_motion_sends_neither_field() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/video"))
        .and(|request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            body.get("motion_id").is_none() && body.get("motion_strength").is_none()
        })
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    run_ok(&api, &["gen", "video", "--prompt", "x", "--no-wait"])
        .stdout(predicate::str::contains(JOB_ID));
}

#[test]
fn gen_video_motion_strength_requires_motion() {
    cmd()
        .args([
            "gen",
            "video",
            "--prompt",
            "x",
            "--motion-strength",
            "strong",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--motion"));
}

#[test]
fn gen_video_rejects_unknown_motion_strength_locally() {
    cmd()
        .args([
            "gen",
            "video",
            "--prompt",
            "x",
            "--motion",
            "push-in",
            "--motion-strength",
            "extreme",
            "--api-url",
            "http://127.0.0.1:9",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("extreme"));
}

#[tokio::test]
async fn jobs_list_sends_filters_and_prints_jobs() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .and(query_param("status", "failed"))
        .and(query_param("modality", "video"))
        .and(query_param("limit", "5"))
        .and(query_param("cursor", "page-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [job_json("failed", None)], "total": 1
        })))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "jobs",
            "list",
            "--status",
            "failed",
            "--modality",
            "video",
            "--limit",
            "5",
            "--cursor",
            "page-1",
        ],
    )
    .stdout(predicate::str::contains(format!("{JOB_ID}  failed  video")))
    .stdout(predicate::str::contains("2026-06-13T00:00:00"));
}

#[tokio::test]
async fn jobs_list_json_prints_whole_page() {
    let api = MockServer::start().await;
    let page = json!({"items": [job_json("queued", None)], "total": 3, "next_cursor": "page-2"});
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&page))
        .mount(&api)
        .await;
    let output = run_ok(&api, &["jobs", "list", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["items"][0]["id"], JOB_ID);
    assert_eq!(value["total"], 3);
    assert_eq!(value["next_cursor"], "page-2");
}

#[tokio::test]
async fn jobs_list_cursor_hint_preserves_filters() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [job_json("failed", None)], "total": 10, "next_cursor": "page-2"
        })))
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "jobs",
            "list",
            "--status",
            "failed",
            "--modality",
            "video",
            "--limit",
            "5",
        ],
    )
    .stderr(predicate::str::contains(
        "more jobs: nolgia jobs list --cursor page-2 --status failed --modality video --limit 5",
    ));
}

#[tokio::test]
async fn jobs_list_rejects_unknown_status_before_request() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["jobs", "list", "--status", "bogus"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid value"));
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn jobs_list_empty_text() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [], "total": 0})))
        .mount(&api)
        .await;
    run_ok(&api, &["jobs", "list"]).stdout("no jobs\n");
}

#[tokio::test]
async fn jobs_list_surfaces_problem_detail() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/jobs"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"detail": "invalid cursor"})))
        .mount(&api)
        .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["jobs", "list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("listing jobs"))
        .stderr(predicate::str::contains("invalid cursor"));
}

// --- jobs cancel (NOL-1025) ---------------------------------------------------

/// A `canceled` Job as `POST /jobs/{id}/cancel` returns it.
fn canceled_job_json(
    settlement: &str,
    refunded: Option<u64>,
    charged: Option<u64>,
    message: &str,
) -> serde_json::Value {
    let mut job = job_json("canceled", None);
    job["status_message"] = json!(message);
    job["cancellation"] = json!({
        "canceled_at": "2026-06-13T00:00:05Z",
        "stage": if settlement == "pending" { "in_progress" } else { "before_submit" },
        "provider_cancel": if settlement == "pending" { "requested" } else { "not_needed" },
        "settlement": settlement,
        "credits_refunded": refunded,
        "credits_charged": charged,
        "message": message
    });
    job
}

/// Only a cancel that carries `Content-Length: 0` matches: a bodyless POST
/// without it is refused with `411` by the production load balancer before
/// the API sees it (NOL-542), and the generated builder sends exactly that.
async fn mock_cancel(response: ResponseTemplate) -> MockServer {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/jobs/{JOB_ID}/cancel")))
        .and(header("content-length", "0"))
        .respond_with(response)
        .mount(&api)
        .await;
    api
}

const REFUNDED_MESSAGE: &str =
    "Canceled before it reached the model provider. All 30 credits were refunded.";
const PENDING_MESSAGE: &str = "Canceled. The provider was asked to stop the render; the 30 credits \
     are held until it answers, then refunded if it does not bill for the render.";

#[tokio::test]
async fn jobs_cancel_prints_the_server_sentence_and_the_refund() {
    let api = mock_cancel(ResponseTemplate::new(200).set_body_json(canceled_job_json(
        "refunded",
        Some(30),
        None,
        REFUNDED_MESSAGE,
    )))
    .await;
    run_ok(&api, &["jobs", "cancel", JOB_ID])
        .stdout(format!(
            "{JOB_ID} video canceled\n{REFUNDED_MESSAGE}\nCredits: 30 refunded.\n"
        ))
        .stderr(predicate::str::contains("Error").not());
}

#[tokio::test]
async fn jobs_cancel_with_a_pending_settlement_says_so_and_how_to_see_it_land() {
    let api = mock_cancel(ResponseTemplate::new(200).set_body_json(canceled_job_json(
        "pending",
        None,
        None,
        PENDING_MESSAGE,
    )))
    .await;
    run_ok(&api, &["jobs", "cancel", JOB_ID])
        .stdout(predicate::str::starts_with(format!(
            "{JOB_ID} video canceled\n{PENDING_MESSAGE}\nCredits: pending."
        )))
        .stdout(predicate::str::contains(
            "nothing is refunded or charged so far",
        ))
        .stdout(predicate::str::contains(format!(
            "nolgia jobs get {JOB_ID}"
        )))
        // Nothing has been refunded yet, so no figure may be printed.
        .stdout(predicate::str::contains("refunded.").not());
}

#[tokio::test]
async fn jobs_cancel_json_prints_the_whole_canceled_job() {
    let body = canceled_job_json(
        "partially_refunded",
        Some(20),
        Some(10),
        "Stopped part way.",
    );
    let api = mock_cancel(ResponseTemplate::new(200).set_body_json(&body)).await;
    let output = run_ok(&api, &["jobs", "cancel", JOB_ID, "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["id"], JOB_ID);
    assert_eq!(value["status"], "canceled");
    assert_eq!(value["cancellation"], body["cancellation"]);
    // Field selection works like every other JSON-producing command.
    run_ok(
        &api,
        &[
            "jobs",
            "cancel",
            JOB_ID,
            "--field",
            "cancellation.settlement",
        ],
    )
    .stdout("partially_refunded\n");
}

#[tokio::test]
async fn jobs_cancel_of_a_finished_job_explains_nothing_changed_and_exits_1() {
    let api = mock_cancel(ResponseTemplate::new(409).set_body_json(json!({
        "type": "about:blank", "title": "Conflict", "status": 409,
        "code": "job_not_cancellable", "detail": "This job already finished."
    })))
    .await;
    for json_mode in [false, true] {
        let mut command = cmd();
        command.arg("--api-url").arg(api.uri());
        if json_mode {
            command.arg("--json");
        }
        command
            .args(["jobs", "cancel", JOB_ID])
            .assert()
            .code(1)
            .stdout("")
            .stderr(predicate::str::contains(format!("canceling job {JOB_ID}")))
            .stderr(predicate::str::contains("409 Conflict"))
            .stderr(predicate::str::contains("This job already finished."))
            .stderr(predicate::str::contains("Nothing was changed."))
            .stderr(predicate::str::contains(format!(
                "nolgia jobs get {JOB_ID}"
            )))
            .stderr(predicate::str::contains("Unexpected Response").not());
    }
}

#[tokio::test]
async fn jobs_cancel_of_someone_elses_job_says_it_is_not_in_your_library() {
    let api = mock_cancel(ResponseTemplate::new(404).set_body_json(json!({
        "type": "about:blank", "title": "Not Found", "status": 404, "detail": "job not found"
    })))
    .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["jobs", "cancel", JOB_ID])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("404 Not Found: job not found"))
        .stderr(predicate::str::contains(
            "in your library in the active workspace",
        ))
        .stderr(predicate::str::contains("nolgia jobs list"));
}

#[tokio::test]
async fn jobs_cancel_without_the_role_names_who_may_cancel() {
    let api = mock_cancel(ResponseTemplate::new(403).set_body_json(json!({
        "type": "about:blank", "title": "Forbidden", "status": 403,
        "detail": "members may cancel only their own jobs"
    })))
    .await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["jobs", "cancel", JOB_ID])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "403 Forbidden: members may cancel only their own jobs",
        ))
        .stderr(predicate::str::contains("owners and admins any job"));
}

#[tokio::test]
async fn jobs_cancel_rejects_a_malformed_id_before_any_request() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["jobs", "cancel", "not-a-uuid"])
        .assert()
        .code(2);
    assert!(api.received_requests().await.unwrap().is_empty());
}

/// A canceled job read back through `status`, `jobs get` or `wait` is a
/// terminal job like any other (exit 0), and the reader is told what the
/// cancel did rather than being left with a bare status word.
#[tokio::test]
async fn read_commands_show_a_canceled_job_as_its_own_terminal_state() {
    let api = MockServer::start().await;
    let job = canceled_job_json("refunded", Some(30), None, REFUNDED_MESSAGE);
    for endpoint in [
        format!("/v1/jobs/{JOB_ID}"),
        format!("/v1/jobs/{JOB_ID}/wait"),
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_json(&job))
            .mount(&api)
            .await;
    }
    for command in [vec!["status"], vec!["jobs", "get"], vec!["wait"]] {
        let args: Vec<&str> = command.iter().copied().chain([JOB_ID]).collect();
        run_ok(&api, &args)
            .stdout(format!("{JOB_ID} video canceled\n"))
            .stderr(predicate::str::contains(REFUNDED_MESSAGE))
            .stderr(predicate::str::contains("Credits: 30 refunded."))
            .stderr(predicate::str::contains("fail").not())
            .stderr(predicate::str::contains("Blocked by the content filter").not());
    }
}

/// A job canceled (from another terminal, or the web) while `gen` waits for
/// it is finished: no asset will come. It must not be reported as a live job
/// that "will be billed once", and not as a bare `Error:` either.
#[tokio::test]
async fn gen_wait_on_a_canceled_job_reports_the_cancel_not_a_live_job() {
    let api = MockServer::start().await;
    let job = canceled_job_json("refunded", Some(30), None, REFUNDED_MESSAGE);
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&job))
        .mount(&api)
        .await;
    for json_mode in [false, true] {
        let mut command = cmd();
        command.arg("--api-url").arg(api.uri());
        if json_mode {
            command.arg("--json");
        }
        let result = command
            .args(["gen", "image", "--prompt", "a cat"])
            .assert()
            .code(1)
            .stderr(predicate::str::contains(format!(
                "canceled: job {JOB_ID} was canceled, so there is no result to download."
            )))
            .stderr(predicate::str::contains(REFUNDED_MESSAGE))
            .stderr(predicate::str::contains("Credits: 30 refunded."))
            .stderr(predicate::str::contains("Error:").not())
            .stderr(predicate::str::contains("still being worked on").not())
            .stderr(predicate::str::contains("billed once").not());
        let stdout = &result.get_output().stdout;
        if json_mode {
            let value: serde_json::Value = serde_json::from_slice(stdout).unwrap();
            assert_eq!(value["status"], "canceled");
            assert_eq!(value["cancellation"]["settlement"], "refunded");
            assert_eq!(value["cancellation"]["credits_refunded"], 30);
            assert_eq!(value["cancellation"]["message"], REFUNDED_MESSAGE);
        } else {
            assert!(stdout.is_empty());
        }
    }
}

/// The announcement printed at submit time still says Ctrl-C only stops the
/// watching, and now names the command that stops the job.
#[tokio::test]
async fn gen_announcement_names_the_real_cancel() {
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
        .stderr(predicate::str::contains(
            "Ctrl-C is safe: it does not cancel the job",
        ))
        .stderr(predicate::str::contains(format!(
            "`nolgia jobs cancel {JOB_ID}` does"
        )))
        // ...and the timeout report keeps saying the job was not cancelled
        // while offering the cancel as the last follow-up.
        .stderr(predicate::str::contains("the job was not cancelled"))
        .stderr(predicate::str::contains(format!(
            "nolgia jobs cancel {JOB_ID}"
        )));
}

async fn mount_voice_models(api: &MockServer) {
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"id": "tts-one", "modality": "audio", "recommended": true,
             "audio": {"voices": [{"id": "bella", "label": "Bella"}, {"id": "river", "label": null}]}},
            {"id": "gpt-image-2", "modality": "image", "recommended": false},
            {"id": "tts-two", "modality": "audio", "recommended": false,
             "audio": {"voices": [{"id": "echo", "label": "Echo"}]}}
        ]})))
        .mount(api).await;
}

#[tokio::test]
async fn voices_list_prints_catalog_order_and_optional_labels() {
    let api = MockServer::start().await;
    mount_voice_models(&api).await;
    run_ok(&api, &["voices", "list"])
        .stdout("tts-one  bella  Bella\ntts-one  river\ntts-two  echo  Echo\n");
}

#[tokio::test]
async fn voices_list_filters_model() {
    let api = MockServer::start().await;
    mount_voice_models(&api).await;
    run_ok(&api, &["voices", "list", "--model", "tts-one"])
        .stdout("tts-one  bella  Bella\ntts-one  river\n");
}

#[tokio::test]
async fn voices_list_json_shape() {
    let api = MockServer::start().await;
    mount_voice_models(&api).await;
    let output = run_ok(&api, &["voices", "list", "--model", "tts-one", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(
        value,
        json!([
            {"model": "tts-one", "id": "bella", "label": "Bella"},
            {"model": "tts-one", "id": "river", "label": null}
        ])
    );
}

#[tokio::test]
async fn voices_list_unknown_model_names_available_models() {
    let api = MockServer::start().await;
    mount_voice_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["voices", "list", "--model", "missing"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unknown model missing; audio models with voices: tts-one, tts-two",
        ));
}

#[tokio::test]
async fn voices_list_model_without_voices_errors() {
    let api = MockServer::start().await;
    mount_voice_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["voices", "list", "--model", "gpt-image-2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "gpt-image-2 publishes no voice catalog (see nolgia models get gpt-image-2)",
        ));
}

#[tokio::test]
async fn voices_list_empty_text() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": []})))
        .mount(&api)
        .await;
    run_ok(&api, &["voices", "list"]).stdout("no voices\n");
}

async fn mount_expand_models(api: &MockServer) {
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"id": "flux-expand", "modality": "image", "recommended": false, "image_expand": true,
             "image": {"aspect_ratios": ["16:9", "1:1", "9:16", "4:5"], "reference_images_max": 1}},
            {"id": "gpt-image-2", "modality": "image", "recommended": true, "image_expand": false,
             "image": {"aspect_ratios": ["16:9", "9:16", "1:1", "3:2", "2:3"], "reference_images_max": 4, "num_images_max": 4}}
        ]})))
        .mount(api).await;
}

#[tokio::test]
async fn gen_image_expand_omits_prompt_and_resolves_default_model() {
    let api = MockServer::start().await;
    mount_expand_models(&api).await;
    Mock::given(method("POST")).and(path("/v1/generate/image"))
        .and(body_partial_json(json!({"model": "flux-expand", "aspect_ratio": "9:16", "reference_asset_ids": [ASSET_ID]})))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .expect(1).mount(&api).await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--expand-to",
            "9:16",
            "--input",
            ASSET_ID,
            "--no-wait",
            "--json",
        ],
    )
    .stdout(predicate::str::contains(JOB_ID));
    let requests = api.received_requests().await.unwrap();
    let request = requests.iter().find(|r| r.method == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert!(body.get("prompt").is_none());
}

#[tokio::test]
async fn gen_image_expand_forwards_prompt() {
    let api = MockServer::start().await;
    mount_expand_models(&api).await;
    Mock::given(method("POST")).and(path("/v1/generate/image"))
        .and(body_partial_json(json!({"model": "flux-expand", "aspect_ratio": "9:16", "reference_asset_ids": [ASSET_ID], "prompt": "sandy beach"})))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .expect(1).mount(&api).await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--model",
            "flux-pro",
            "--expand-to",
            "9:16",
            "--input",
            ASSET_ID,
            "--prompt",
            "sandy beach",
            "--no-wait",
        ],
    );
}

#[tokio::test]
async fn gen_image_expand_requires_input() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "image", "--expand-to", "9:16"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--input"));
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gen_image_expand_conflicts_with_incompatible_flags() {
    let api = MockServer::start().await;
    for (flag, value) in [
        ("--aspect-ratio", "1:1"),
        ("--quality", "2k"),
        ("--character-id", CHARACTER_ID),
        ("--face-reference-asset-id", ASSET_ID),
        ("--mask", ASSET_ID),
        ("--render-quality", "high"),
    ] {
        cmd()
            .arg("--api-url")
            .arg(api.uri())
            .args([
                "gen",
                "image",
                "--expand-to",
                "9:16",
                "--input",
                ASSET_ID,
                flag,
                value,
            ])
            .assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("cannot be used with"));
    }
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gen_image_expand_rejects_unsupported_ratio() {
    let api = MockServer::start().await;
    mount_expand_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--expand-to",
            "21:9",
            "--input",
            ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported by flux-expand"));
    assert!(
        api.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn gen_image_expand_rejects_incapable_model() {
    let api = MockServer::start().await;
    mount_expand_models(&api).await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args([
            "gen",
            "image",
            "--model",
            "gpt-image-2",
            "--expand-to",
            "9:16",
            "--input",
            ASSET_ID,
            "--no-wait",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("gpt-image-2"))
        .stderr(predicate::str::contains("flux-expand"));
    assert!(
        api.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn gen_image_still_requires_prompt_without_expand() {
    let api = MockServer::start().await;
    cmd()
        .arg("--api-url")
        .arg(api.uri())
        .args(["gen", "image"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--prompt"));
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gen_image_expand_preserves_explicit_capable_model() {
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{
            "id": "future-expand", "modality": "image", "recommended": false,
            "image_expand": true, "image": {"aspect_ratios": ["9:16"]}
        }]})))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .and(body_partial_json(json!({"model": "future-expand", "aspect_ratio": "9:16", "reference_asset_ids": [ASSET_ID]})))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(
        &api,
        &[
            "gen",
            "image",
            "--model",
            "future-expand",
            "--expand-to",
            "9:16",
            "--input",
            ASSET_ID,
            "--no-wait",
        ],
    );
}

#[tokio::test]
async fn gen_image_expand_precheck_fails_open_when_catalog_cannot_refuse() {
    for catalog_response in [
        ResponseTemplate::new(503),
        ResponseTemplate::new(200).set_body_json(json!({"models": []})),
        ResponseTemplate::new(200).set_body_json(json!({"models": [{
            "id": "future-expand", "modality": "image", "recommended": false,
            "image": {"aspect_ratios": ["9:16"]}
        }]})),
    ] {
        let api = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(catalog_response)
            .mount(&api)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/generate/image"))
            .and(body_partial_json(json!({"model": "future-expand", "aspect_ratio": "9:16", "reference_asset_ids": [ASSET_ID]})))
            .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
            .expect(1)
            .mount(&api)
            .await;
        run_ok(
            &api,
            &[
                "gen",
                "image",
                "--model",
                "future-expand",
                "--expand-to",
                "9:16",
                "--input",
                ASSET_ID,
                "--no-wait",
            ],
        );
    }
}

async fn mock_failed_generation(failure: Option<serde_json::Value>) -> MockServer {
    let api = MockServer::start().await;
    let mut job = job_json("failed", None);
    if let Some(failure) = failure {
        job["failure"] = failure;
    }
    Mock::given(method("POST"))
        .and(path("/v1/generate/image"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(200).set_body_json(job))
        .mount(&api)
        .await;
    api
}

#[tokio::test]
async fn gen_moderated_reports_refund_truth_and_exits_65() {
    for (refund, expected) in [
        (Some(json!(false)), "charged for this attempt"),
        (Some(json!(true)), "refunded, the credit hold was released."),
        (None, "no refund outcome was recorded"),
        (Some(json!(null)), "no refund outcome was recorded"),
    ] {
        let mut failure =
            json!({"kind": "moderated", "message": "Provider blocked reference media"});
        if let Some(refund) = refund {
            failure["credits_refunded"] = refund;
        }
        let api = mock_failed_generation(Some(failure)).await;
        for json_mode in [false, true] {
            let mut command = cmd();
            command.arg("--api-url").arg(api.uri());
            if json_mode {
                command.arg("--json");
            }
            let result = command
                .args(["gen", "image", "--prompt", "a cat"])
                .assert()
                .code(65)
                .stderr(predicate::str::contains("Blocked by the content filter"))
                .stderr(predicate::str::contains(JOB_ID))
                .stderr(predicate::str::contains("Provider blocked reference media"))
                .stderr(predicate::str::contains(expected))
                .stderr(predicate::str::contains("still running").not())
                .stderr(predicate::str::contains("Re-running").not())
                .stderr(predicate::str::contains("Error:").not());
            if json_mode {
                let job: serde_json::Value =
                    serde_json::from_slice(&result.get_output().stdout).unwrap();
                assert_eq!(job["id"], JOB_ID);
                assert_eq!(job["status"], "failed");
                assert_eq!(job["failure"]["kind"], "moderated");
            }
        }
    }
}

#[tokio::test]
async fn gen_other_failed_jobs_keep_the_existing_exit_code() {
    for failure in [
        Some(json!({"kind": "error", "message": "Provider failed"})),
        Some(json!({"kind": "future_kind", "message": "Provider failed"})),
        None,
    ] {
        let api = mock_failed_generation(failure).await;
        cmd()
            .arg("--api-url")
            .arg(api.uri())
            .args(["gen", "image", "--prompt", "a cat"])
            .assert()
            .code(EXIT_LIVE_JOB)
            .stderr(predicate::str::contains(
                "image job completed without asset",
            ))
            .stderr(predicate::str::contains("Blocked by the content filter").not());
    }
}

#[tokio::test]
async fn moderated_read_commands_keep_exit_zero_and_job_output() {
    let api = MockServer::start().await;
    let mut job = job_json("failed", None);
    job["failure"] = json!({"kind": "moderated", "message": "Provider blocked reference media", "credits_refunded": false});
    for endpoint in [
        format!("/v1/jobs/{JOB_ID}"),
        format!("/v1/jobs/{JOB_ID}/wait"),
    ] {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_json(&job))
            .mount(&api)
            .await;
    }
    for command in ["wait", "status"] {
        run_ok(&api, &[command, JOB_ID])
            .stdout(predicate::str::contains(format!("{JOB_ID} video failed")))
            .stderr(predicate::str::contains("Blocked by the content filter"))
            .stderr(predicate::str::contains("Provider blocked reference media"));
        let result = run_ok(&api, &["--json", command, JOB_ID]);
        let output: serde_json::Value =
            serde_json::from_slice(&result.get_output().stdout).unwrap();
        assert_eq!(output["id"], job["id"]);
        assert_eq!(output["status"], job["status"]);
        assert_eq!(output["failure"], job["failure"]);
    }
}

#[tokio::test]
async fn gen_3d_no_wait_omits_defaults() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/3d"))
        .and(body_json(json!({"image_asset_ids": [ASSET_ID]})))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .expect(1)
        .mount(&api)
        .await;
    run_ok(&api, &["gen", "3d", "--input", ASSET_ID, "--no-wait"])
        .stdout(predicate::str::contains(JOB_ID));
}

#[tokio::test]
async fn gen_3d_forwards_options() {
    for (options, expected) in [
        (
            vec![
                "--input",
                ASSET_ID,
                "--input",
                ELEMENT_ASSET_ID,
                "--input",
                CHARACTER_ID,
                "--input",
                PRODUCT_ID,
                "--pbr",
            ],
            json!({"image_asset_ids": [ASSET_ID, ELEMENT_ASSET_ID, CHARACTER_ID, PRODUCT_ID], "pbr": true}),
        ),
        (
            vec!["--input", ASSET_ID, "--draft"],
            json!({"image_asset_ids": [ASSET_ID], "quality": "draft"}),
        ),
        (
            vec!["--image-url", "https://example.com/front.png"],
            json!({"image_url": "https://example.com/front.png"}),
        ),
        (
            vec![
                "--input",
                ASSET_ID,
                "--model",
                "hunyuan3d-v3",
                "--no-texture",
                "--project-id",
                PROJECT_ID,
                "--tag",
                "prop",
            ],
            json!({"image_asset_ids": [ASSET_ID], "model": "hunyuan3d-v3", "texture": false, "project_id": PROJECT_ID, "tags": ["prop"]}),
        ),
    ] {
        let api = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/generate/3d"))
            .and(body_json(expected))
            .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
            .expect(1)
            .mount(&api)
            .await;
        let mut args = vec!["gen", "3d", "--no-wait"];
        args.extend(options);
        run_ok(&api, &args).stdout(predicate::str::contains(JOB_ID));
    }
}

#[tokio::test]
async fn gen_3d_invalid_options_make_no_requests() {
    let api = MockServer::start().await;
    for (options, message) in [
        (
            vec!["--draft", "--input", "front.png", "--input", "back.png"],
            "--input",
        ),
        (
            vec!["--model", "trellis", "--input", "front.png", "--pbr"],
            "--pbr",
        ),
        (
            vec!["--draft", "--input", "front.png", "--no-texture"],
            "--no-texture",
        ),
        (
            vec!["--input", "front.png", "--pbr", "--no-texture"],
            "--no-texture",
        ),
        (
            vec!["--input", "front.png", "--draft", "--model", "trellis"],
            "--model",
        ),
        (
            vec![
                "--input",
                "front.png",
                "--image-url",
                "https://example.com/front.png",
            ],
            "--image-url",
        ),
        (
            vec![
                "--input", "1.png", "--input", "2.png", "--input", "3.png", "--input", "4.png",
                "--input", "5.png",
            ],
            "--input",
        ),
        (vec![], "required"),
    ] {
        cmd()
            .args([
                "--api-url",
                &api.uri(),
                "--token",
                "test-token",
                "gen",
                "3d",
            ])
            .args(options)
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
    assert!(api.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gen_3d_wait_downloads_glb_to_exact_path() {
    let api = MockServer::start().await;
    let mut completed = job_json("succeeded", Some(&api.uri()));
    completed["modality"] = json!("3d");
    completed["asset"]["modality"] = json!("3d");
    completed["asset"]["mime_type"] = json!("model/gltf-binary");
    completed["asset"]["signed_url"] = json!(format!("{}/model.glb", api.uri()));
    Mock::given(method("POST"))
        .and(path("/v1/generate/3d"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(200).set_body_json(completed))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path("/model.glb"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"glTF-test".to_vec()))
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("model");
    run_ok(
        &api,
        &[
            "gen",
            "3d",
            "--input",
            ASSET_ID,
            "--out",
            out.to_str().unwrap(),
        ],
    )
    .stdout(predicate::str::contains(JOB_ID))
    .stdout(predicate::str::contains("/model.glb"));
    assert_eq!(std::fs::read(out).unwrap(), b"glTF-test");
}

async fn mount_three_d_models(api: &MockServer) {
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
            {"id": "hunyuan3d-v3", "name": "Hunyuan3D", "modality": "3d", "recommended": true,
             "cost": {"credits": 21, "unit": "per_generation"},
             "three_d": {"max_images": 4, "multi_view": true, "untextured": true, "pbr": true, "pbr_credits": 9, "multi_view_credits": 9, "untextured_credits": 13}},
            {"id": "trellis", "name": "Trellis", "modality": "3d", "recommended": false,
             "cost": {"credits": 2, "unit": "per_generation"},
             "three_d": {"max_images": 1, "multi_view": false, "untextured": false, "pbr": false, "pbr_credits": null, "multi_view_credits": null, "untextured_credits": null}}
        ]}))).mount(api).await;
}

#[tokio::test]
async fn gen_3d_cost_only_uses_catalog_without_uploading() {
    let api = MockServer::start().await;
    mount_three_d_models(&api).await;
    for (options, credits) in [
        (vec![], "21 credits"),
        (vec!["--no-texture"], "13 credits"),
        (vec!["--pbr"], "30 credits"),
        (
            vec!["--input", "back.png", "--input", "left.png", "--pbr"],
            "39 credits",
        ),
        (vec!["--input", "back.png", "--no-texture"], "22 credits"),
        (vec!["--draft"], "2 credits"),
        (vec!["--model", "trellis"], "2 credits"),
    ] {
        let mut args = vec!["gen", "3d", "--input", "front.png", "--cost-only"];
        args.extend(options);
        run_ok(&api, &args).stdout(predicate::str::contains(credits));
    }
    assert!(
        api.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET" && r.url.path() == "/v1/models")
    );
}

#[tokio::test]
async fn gen_3d_cost_only_catalog_unreachable_does_not_submit() {
    let api = MockServer::start().await;
    run_ok(&api, &["gen", "3d", "--input", "front.png", "--cost-only"])
        .stdout(predicate::str::contains("estimate unavailable"))
        .stdout(predicate::str::contains("No job submitted"));
    assert_eq!(api.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn models_list_filters_and_describes_3d() {
    let api = MockServer::start().await;
    mount_three_d_models(&api).await;
    run_ok(&api, &["models", "list", "--modality", "3d"])
        .stdout(predicate::str::contains("21 credits (per_generation)"))
        .stdout(predicate::str::contains(
            "up to 4 images  multi-view  untextured  PBR",
        ))
        .stdout(predicate::str::contains("trellis"));
}

#[tokio::test]
async fn three_d_modality_filters_reach_jobs_and_assets() {
    let api = MockServer::start().await;
    for resource in ["jobs", "assets"] {
        Mock::given(method("GET"))
            .and(path(format!("/v1/{resource}")))
            .and(query_param("modality", "3d"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"items": [], "total": 0})),
            )
            .expect(1)
            .mount(&api)
            .await;
        run_ok(&api, &[resource, "list", "--modality", "3d"]);
    }
}

#[tokio::test]
async fn gen_3d_wait_timeout_preserves_live_job() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/generate/3d"))
        .respond_with(ResponseTemplate::new(202).set_body_json(job_json("queued", None)))
        .mount(&api)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/jobs/{JOB_ID}/wait")))
        .respond_with(ResponseTemplate::new(408))
        .mount(&api)
        .await;
    cmd()
        .args([
            "--api-url",
            &api.uri(),
            "--token",
            "test-token",
            "gen",
            "3d",
            "--input",
            ASSET_ID,
        ])
        .assert()
        .code(75)
        .stderr(predicate::str::contains(JOB_ID));
}
