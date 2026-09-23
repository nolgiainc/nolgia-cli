---
name: nolgia-platform
description: "Generate images, video, audio, and 3D models on the NOLGIA platform with the `nolgia` CLI or the NOLGIA MCP server (https://mcp.nolgia.ai/mcp): auth, model choice from the live catalog, async jobs, reference images, outpainting, voices, credits, and failure recovery. Use when: the user asks for AI-generated media and NOLGIA is available, e.g. 'generate an image', 'make a video', 'animate this photo', 'extend this image to 9:16', 'make a voiceover', 'check my jobs', or 'assemble these clips into one video'. NOT for: directing craft and multi-shot planning (use nolgia-video-prompting), vertical UGC ad production (use nolgia-ugc-ads), or editing media locally without the platform."
version: 1.1.0
author: NOLGIA
license: MIT
metadata:
  tags: [nolgia, video-generation, image-generation, audio-generation, cli, mcp]
  related_skills: [nolgia-video-prompting, nolgia-ugc-ads]
---

# NOLGIA Platform

NOLGIA generates images, video (with native audio), audio/TTS, and 3D models through
one API. Two ways in from an agent:

- **CLI** (this skill's default): `nolgia`. Add `--json` for machine
  output. Install one of: `brew install nolgiainc/nolgia/nolgia`,
  `npm install -g @nolgia/cli`,
  `curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash`,
  or `cargo install nolgia-cli`.
- **MCP**: `https://mcp.nolgia.ai/mcp` (Streamable HTTP, `Authorization:
  Bearer <token>`). Tools `nolgia_text_to_image`, `nolgia_image_to_image`,
  `nolgia_text_to_video`, `nolgia_image_to_video`, `nolgia_text_to_audio`,
  `nolgia_list_models`, `nolgia_list_jobs`, `nolgia_list_assets`,
  `nolgia_get_asset`, `nolgia_get_account`, plus characters, projects and
  presets. The same catalog and rules as the CLI flags below.

## Auth

```bash
export NOLGIA_TOKEN=nol_...   # PAT from nolgia.ai/settings/api-tokens (spends API credits)
nolgia auth status            # verify: prints "email (tier)"
# interactive alternative: nolgia auth login  (device flow)
```

`NOLGIA_API_URL=https://api.stg.nolgia.ai` targets staging: use it for
experiments, production for deliverables.

## The catalog is the source of truth

Never guess a model id, a ratio, a tier or a price. Read them live:

```bash
nolgia models list --modality video      # ids, durations, ratios, credits
nolgia models get seedance-2.5           # one model: tiers, references, ratios
nolgia voices list --model <tts model>   # voice ids for --voice
```

## Generate

```bash
# Image (prints the signed URL; --out saves it). Default model: flux-pro.
nolgia gen image --prompt "isometric server room, dramatic lighting" --out img.png

# 3D (GLB): 1 to 4 photos; --draft costs 2 credits. Estimate with --cost-only.
nolgia gen 3d --input front.png --out model.glb

# Video (async job; waits by default, --no-wait returns the job id).
# Default model: seedance-2.5.
nolgia gen video --prompt "drone shot over a rocky coastline at dawn" \
  --duration-seconds 10 --aspect-ratio 16:9 --out clip.mp4

# Image-to-video (character/product consistency): --input uploads the file.
# Needs a model that takes a start image (`nolgia models get <model>`).
nolgia gen video --model minimax-h3 \
  --input portrait.png --prompt "she turns to camera and smiles" --out talk.mp4

# Reference-to-video (remix existing footage; Seedance 2.0 Pro):
# --video-ref = video asset UUID (up to 3; MP4/MOV, 480p-720p, 2-15s and
# 50MB combined; this model REQUIRES at least one), --element = image
# asset UUID (up to 9). Address them in the prompt as @Video1.. / @Image1..
nolgia gen video --model fal-ai/bytedance/seedance/v2/pro/reference-to-video \
  --video-ref <video asset uuid> --element <image asset uuid> \
  --prompt "@Video1 restaged in the style of @Image1" \
  --quality 1080p --bitrate high --out remix.mp4

# Outpaint: grow an image you already have to a new ratio (flux-expand).
# Keeps the source pixels and paints the added margins; --prompt is optional.
nolgia gen image --expand-to 9:16 --input still.png \
  --prompt "more of the sandy beach" --out vertical.png

# Speech: pick a voice, then generate. Music/SFX models take no voice.
nolgia voices list --model fal-ai/elevenlabs/tts/eleven-v3
nolgia gen audio --model fal-ai/elevenlabs/tts/eleven-v3 --voice <voice id> \
  --prompt "Welcome back. Here is what changed this week." --out vo.mp3
nolgia gen audio --prompt "warm lofi beat, vinyl crackle" --out bed.mp3
```

`--quality` selects a model-specific resolution tier (premium tiers cost
more; tiers and per-tier credits in `nolgia models get <model>`;
`--cost-only` prices the request without submitting it). `--end-frame
<image asset uuid|file>` pins the final frame next to `--input` on models
with end-frame support, and `nolgia assets frame <video asset id> [--at
SECONDS]` extracts a still (default: last frame) to chain clips.

## Video model selection (credits differ several times over)

| Model | Duration | Best for |
|---|---|---|
| `seedance-2.5` (default) | per catalog | general cinematic text-to-video |
| `minimax-h3` | 5-15s | photoreal humans, identity across shots, native speech |
| `fal-ai/bytedance/seedance/v2/pro/text-to-video` | 4-15s | **multi-shot** (`--shot`), native audio |
| `veo-3.1` / `veo-3.1-fast` | 4/6/8s only | hero quality / fast previz |
| `heygen-avatar-iv` | length of the voice track | lip sync: a portrait (`--input`) speaks an audio track (`--audio-ref`) |

Always set `--aspect-ratio` explicitly for vertical content, and check the
model publishes it (`nolgia models get <model>`).

## Multi-shot (Seedance 2.0 Pro cuts between shots natively)

```bash
nolgia gen video --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
  --prompt "Gritty 35mm film look." --generate-audio true \
  --shot "8:WIDE SHOT. Rural highway, one car heading south.|engine, wind" \
  --shot "4:MCU. The driver glances at the dead radio.|AM static cuts out"
```

Up to 8 shots; clip duration = sum; `--prompt` becomes overall
style/context; `|` separates an optional per-shot audio direction. See the
`nolgia-video-prompting` skill for the directing craft.

## Jobs, assets, credits

```bash
nolgia gen video --prompt "..." --no-wait --json   # {"job_id": "..."}
nolgia wait <job_id> --timeout 600 --json          # blocks to terminal state
nolgia status <job_id>                             # snapshot of one job
nolgia jobs list --status running                  # every job in flight
nolgia jobs list --status failed --limit 10        # what went wrong lately
nolgia jobs cancel <job_id>                        # stop a job; prints the refund
nolgia assets list --modality video --limit 5      # id, modality, signed URL
nolgia billing credits                             # both pools
```

Video jobs take minutes. A job the server accepted is never lost: if the
wait times out, the CLI names the job id and exits 75; follow it with
`nolgia wait <job_id>` instead of submitting again (a re-run bills again).
Ctrl-C or a timeout only stops the watching; `nolgia jobs cancel <job_id>`
stops the job itself. Unstarted work is refunded in full; a render the
provider already started is refunded only if the provider stops it without
billing, and credits can read `pending` until it answers (check again with
`nolgia jobs get <job_id>`). A finished job cannot be canceled.
Asset signed URLs **expire in 15 minutes**: download promptly (`--out`
handles this). PAT requests spend the `shared_topup` (API) pool only; `402`
means top up.

## Characters, projects, tags

```bash
nolgia characters create --name "Captain Nova" \
  --description "silver-haired astronaut, teal flight suit" \
  --reference-asset-id <uuid>                      # up to 4 reference images
nolgia characters list                             # id, name, ref count
nolgia projects create --name "Q3 launch"
nolgia projects add-assets <project_id> --asset-id <uuid> --asset-id <uuid>
nolgia gen image --prompt "..." --project-id <uuid>  # file the result at creation
nolgia assets upload ref.png --project-id <uuid>   # uploads file the same way
nolgia assets tag <asset_id> --tag hero --tag campaign   # REPLACES the set; --clear wipes
nolgia assets list --tag hero --project-id <uuid>  # filter by tag / project
```

Characters keep a recurring subject consistent: `gen image --character-id
<uuid>` and `gen video --character-id <uuid>` bind a character's reference
and description to the render. Projects group assets (an asset can be in
many); tags label and filter them. Prefer `--project-id` on
`gen`/`assets upload` over after-the-fact `projects add-assets` when the
destination project is known up front.

## Assemble clips into ONE finished video (timeline + render)

A project is a FOLDER of assets, not a video. To turn several clips into a
single finished cut, build a Studio **composition** (a timeline) and
**render** it.

```bash
# Build the timeline from clips IN ORDER, render it, wait for the finished file.
nolgia compositions create --name "launch-trailer" \
  --clip <shot1_uuid> --clip <shot2_uuid> --clip <shot3_uuid> \
  --project <project_uuid> \
  --render --wait --json        # -> {"asset_id","url",...} of the finished MP4

# Or in two steps / to re-check a slow render:
nolgia compositions render <composition_id> --wait
nolgia compositions status <render_id>
```

- Clips play back to back in the given order; each clip's own audio is mixed
  in (pass `--mute` for a silent cut). Canvas defaults to `1920x1080`; set
  `--width`/`--height` (e.g. `--width 1080 --height 1920` for vertical).
- Renders take a few minutes and are capped at 15.

**Never** report a "final video" or "final cut" until a render has
**succeeded** and you can name the produced asset id/URL. Clips dropped into
a project folder are not the video the customer asked for.

## Failure recovery

- `content_policy_violation` / `partner_validation_failed`: the upstream
  provider sometimes rejects benign prompts. Retry once verbatim; if it
  repeats, rephrase the flagged sentence rather than fighting the filter.
- `400` naming a capability (tier, ratio, reference count): the model does
  not publish it. Read `nolgia models get <model>` and adjust; the message
  lists what the model accepts.
- `409` on submit: an identical request is already a job; the CLI names it.
  Follow that job, or pass `--idempotency-key <new key>` to run the same
  request again on purpose.
- Estimate before big batches with `--cost-only`, and confirm with the user
  before anything over about 2,000 credits.
