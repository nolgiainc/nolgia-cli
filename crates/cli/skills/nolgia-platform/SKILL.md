---
name: nolgia-platform
description: "Generate images, video, and audio on the NOLGIA platform via the `nolgia` CLI or the nolgia_* MCP tools (mcp.nolgia.ai). Use whenever the user asks for AI-generated media and NOLGIA is available. Covers auth (PAT/device flow), model selection (Kling v3 / Seedance v2 Pro / Veo 3.1 / flux-pro), async job polling, reference images, credits, and failure recovery."
version: 1.0.0
author: NOLGIA
license: MIT
metadata:
  tags: [nolgia, video-generation, image-generation, audio-generation, cli, mcp]
---

# NOLGIA Platform

NOLGIA generates images, video (with native audio), and audio/TTS through
one API. Two ways in from an agent:

- **CLI** (this skill's default): `nolgia` — add `--json` for machine
  output. Install: `brew install nolgiacorp/nolgia/nolgia` or
  `cargo install nolgia-cli`.
- **MCP**: `https://mcp.nolgia.ai` — tools `nolgia_text_to_image`,
  `nolgia_image_to_image`, `nolgia_text_to_video`, `nolgia_image_to_video`,
  `nolgia_text_to_audio`, `nolgia_list_jobs`, `nolgia_list_assets`,
  `nolgia_get_asset`, `nolgia_delete_asset`, `nolgia_get_account`,
  `nolgia_get_usage`. Same parameters as the CLI flags below.

## Auth

```bash
export NOLGIA_TOKEN=nol_...   # PAT from nolgia.ai/account/tokens (spends API credits)
nolgia auth status            # verify: prints "email (tier)"
# interactive alternative: nolgia auth login  (device flow, keyring-stored)
```

`NOLGIA_API_URL=https://api.stg.nolgia.ai` targets staging — use it for
experiments; production for deliverables.

## Generate

```bash
# Image (sync — prints signed URL; --out saves it)
nolgia gen image --prompt "isometric server room, dramatic lighting" --out img.png

# Video (async job; waits by default, --no-wait returns the job id)
nolgia gen video --prompt "drone shot over a rocky coastline at dawn" \
  --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
  --duration-seconds 12 --aspect-ratio 16:9 --generate-audio true \
  --out clip.mp4

# Image-to-video (character/product consistency): --input uploads the file
nolgia gen video --model fal-ai/kling-video/v3/pro/image-to-video \
  --input portrait.png --prompt "she turns to camera and smiles" --out talk.mp4

# Audio / TTS / music / SFX
nolgia gen audio --prompt "warm lofi beat, vinyl crackle" --out bed.mp3
```

## Video model selection (credits differ ~5x)

| Model | Duration | Best for |
|---|---|---|
| `fal-ai/kling-video/v3/text-to-video` (`/master`, `/pro`) | 3–15s | drafts, volume, UGC |
| `fal-ai/bytedance/seedance/v2/pro/text-to-video` | 4–15s | cinematic, **multi-shot**, native audio |
| `veo-3.1` / `veo-3.1-fast` | 4/6/8s only | hero quality / fast previz |

Every t2v except Veo has an `image-to-video` sibling for `--input`.
Aspect ratios: 16:9, 9:16, 1:1 (Seedance adds 4:3, 3:4). Always set
`--aspect-ratio` explicitly for vertical content.

## Multi-shot (Seedance cuts between shots natively)

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
nolgia status <job_id>                             # snapshot
nolgia assets list --modality video --limit 5      # id, modality, signed URL
nolgia billing credits                             # both pools
nolgia account usage
```

Video jobs take minutes. Asset signed URLs **expire in 15 minutes** —
download promptly (`--out` handles this). PAT requests spend the
`shared_topup` (API) pool only; `402` means top up.

## Characters, projects, tags

```bash
nolgia characters create --name "Captain Nova" \
  --description "silver-haired astronaut, teal flight suit" \
  --reference-asset-id <uuid>                      # ≤4 reference images
nolgia characters list                             # id, name, ref count
nolgia projects create --name "Q3 launch"
nolgia projects add-assets <project_id> --asset-id <uuid> --asset-id <uuid>
nolgia assets tag <asset_id> --tag hero --tag campaign   # REPLACES the set; --clear wipes
nolgia assets list --tag hero --project-id <uuid>  # filter by tag / project
```

Characters keep a recurring subject consistent: seed image-to-video with a
character's reference image (`gen video --input <reference asset uuid>`)
and fold its description into the prompt. Projects group assets (an asset
can be in many); tags label and filter them.

## Failure recovery

- `content_policy_violation` / `partner_validation_failed`: the upstream
  provider sometimes rejects benign prompts, especially with
  `--generate-audio true`. Retry once verbatim; if it repeats, rephrase
  the flagged sentence — don't fight the filter.
- `422` on `--shot`/`--generate-audio`: the API deployment predates those
  features — fall back to a single composed prompt and plain generation.
- Estimate before big batches: Seedance ≈ 30 credits/sec, Kling ≈ 8,
  Veo ≈ 40. Confirm with the user before anything over ~2,000 credits.
