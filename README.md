# Nolgia CLI

[![Crates.io](https://img.shields.io/crates/v/nolgia-cli?logo=rust)](https://crates.io/crates/nolgia-cli)
[![npm](https://img.shields.io/npm/v/%40nolgia%2Fcli?logo=npm)](https://www.npmjs.com/package/@nolgia/cli)
[![Homebrew](https://img.shields.io/badge/homebrew-nolgiainc%2Fnolgia-orange?logo=homebrew)](https://github.com/nolgiainc/homebrew-nolgia)
[![Release](https://github.com/nolgiainc/nolgia-cli/actions/workflows/release.yml/badge.svg)](https://github.com/nolgiainc/nolgia-cli/actions/workflows/release.yml)
[![Downloads](https://img.shields.io/crates/d/nolgia-cli?logo=rust&label=crates%20downloads)](https://crates.io/crates/nolgia-cli)
[![npm downloads](https://img.shields.io/npm/dm/%40nolgia%2Fcli?logo=npm&label=npm%20downloads)](https://www.npmjs.com/package/@nolgia/cli)
[![License](https://img.shields.io/crates/l/nolgia-cli)](#license)

Command-line client for the [Nolgia](https://nolgia.ai) generative media platform — Seedance v2 Pro, Kling v3, Veo 3.1, FLUX Pro, ElevenLabs and more, from your terminal, scripts, and AI agents. Multi-shot video sequences, image-to-video with reusable references, native audio tracks, credit estimates before you spend, and bundled agent skills.

```console
$ nolgia gen image --prompt "A serene mountain lake at dawn" --out lake.png
$ nolgia gen video --prompt "A drone shot over a coastline" --no-wait
9b2f5c1e-...   # job id; check with `nolgia wait <id>`
```

> A [Nolgia account](https://nolgia.ai) with an active subscription or prepaid API credits is required.

## Contents

- [Installation](#installation) · [Quick start](#quick-start) · [Examples](#examples) · [Models](#models) · [AI agents & skills](#ai-agents--skills) · [Authentication](#authentication) · [Credits](#credits) · [Command reference](#command-reference) · [Global flags and environment](#global-flags-and-environment) · [Shell completions](#shell-completions) · [Development](#development)

## Installation

### Shell installer (macOS, Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash
```

The installer picks /usr/local/bin when writable and ~/.local/bin otherwise; pass `--prefix <dir>` to choose and `--tag vX.Y.Z` to pin a release. On macOS it clears the quarantine attribute for you

### npm

```bash
npm install -g @nolgia/cli
```

The package downloads the prebuilt binary for your platform (macOS universal, Linux x86_64, Windows x86_64) during postinstall

### Homebrew (macOS, Linux)

```bash
brew tap nolgiainc/nolgia
brew install nolgia
```

Homebrew prompts once to trust third-party taps: `brew trust nolgiainc/nolgia`.

### Cargo

```bash
cargo install nolgia-cli
```

Building from source requires the Rust 2024 toolchain; on Linux you also need `libdbus-1-dev` and `pkg-config` for keyring support.

### Prebuilt binaries

Download the binary for your platform from the [latest release](https://github.com/nolgiainc/nolgia-cli/releases/latest) (macOS universal, Linux x86_64, Windows x86_64), rename it to `nolgia` (`nolgia.exe` on Windows), and place it on your `PATH`.

### Update check

The CLI checks GitHub for a newer release at most once a day and prints a one-line upgrade hint on stderr, matched to how you installed it. It never delays commands, stays quiet for `--json`, piped output, CI, and agent traffic, and can be disabled entirely with `NOLGIA_NO_UPDATE_CHECK=1`

## Quick start

```bash
nolgia auth login                                  # device-code sign-in via your browser
nolgia gen image --prompt "watercolor fox" --out fox.png
nolgia models list                                 # live catalog: models, pricing, capabilities
nolgia billing credits                             # see what you have left
```

## Examples

### Multi-shot cinematic sequence (Seedance v2 Pro, native audio)

The clip is one generation; the model cuts between your shots. `--prompt` sets overall style, each `--shot` is `"SECONDS:PROMPT"` with an optional `|AUDIO DIRECTION`:

```bash
nolgia gen video --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
  --prompt "Gritty 35mm film look, overcast light." --generate-audio true \
  --shot "8:WIDE SHOT. Rural highway, one car heading south.|engine, wind, distant birds" \
  --shot "4:MCU. The driver glances at the dead radio.|AM static cuts out" \
  --out highway.mp4
```

### Image-to-video with a reusable reference

`--input` takes a local file (uploaded automatically) **or the UUID of any previous asset** — generate a character portrait once, reuse it across every clip:

```bash
nolgia gen image --prompt "portrait, wire-rim glasses, olive field jacket" --out maya.png
nolgia gen video --model fal-ai/kling-video/v3/pro/image-to-video \
  --input maya.png --prompt "she turns toward the camera and smiles" --out shot1.mp4
nolgia assets list --modality image --limit 1      # grab the portrait's asset id
nolgia gen video --model fal-ai/kling-video/v3/pro/image-to-video \
  --input <asset-uuid> --prompt "she walks out of frame" --out shot2.mp4
```

### Know the cost before you spend

```bash
$ nolgia gen video --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
    --duration-seconds 12 --prompt "..." --cost-only
365 credits (fal-ai/bytedance/seedance/v2/pro/text-to-video, 12s)
```

### Hero-quality shot (Veo 3.1)

```bash
nolgia gen video --model veo-3.1 --duration-seconds 8 --aspect-ratio 16:9 \
  --prompt "slow cinematic dolly through a warmly lit design studio" --out hero.mp4
```

### Works with your tools

```bash
# newest finished video assets, URLs only
nolgia assets list --modality video --json | jq -r '.items[].signed_url'

# fire-and-poll from a script
id=$(nolgia gen video --prompt "..." --no-wait --json | jq -r .job_id)
nolgia wait "$id" --timeout 600 --json | jq .asset.signed_url
```

## Models

The server is the source of truth — new models appear in the catalog with no CLI update:

```bash
nolgia models list                 # id, modality, credit pricing, durations, aspect ratios
nolgia models list --modality video
nolgia models get fal-ai/bytedance/seedance/v2/pro/text-to-video
```

Current video lineup: **Kling v3** (standard/master/pro, 3–15s, the workhorse), **Seedance v2 Pro** (4–15s, multi-shot + native audio, cinematic), **Veo 3.1 / 3.1-fast** (4/6/8s, hero quality / fast previz) — each with an image-to-video variant (except Veo). Defaults: `flux-pro` (image), `fal-ai/stable-audio-25/text-to-audio` (audio), `fal-ai/kling-video/v3/text-to-video` (video).

## AI agents & skills

The CLI ships **agent skills** — SKILL.md packs that teach Claude Code, hermes, Cursor, or any agent how to generate on Nolgia well:

```bash
nolgia skills list
nolgia skills install                          # → ~/.claude/skills/  (Claude Code)
nolgia skills install --target claude-project  # → ./.claude/skills/
nolgia skills install --target hermes          # → $HERMES_HOME/skills/
```

Bundled: `nolgia-platform` (the full tool surface), `nolgia-video-prompting` (shot grammar, multi-shot directing, consistency), `nolgia-ugc-ads` (vertical ad production recipe).

Also for agents: an **MCP server** at `https://mcp.nolgia.ai` (tools `nolgia_text_to_video`, `nolgia_text_to_image`, …, same params as the CLI flags), **PATs** for headless auth, `nolgia auth token` to extract the current bearer, and `--json` everywhere. The CLI identifies its calling surface (`X-Nolgia-Surface`: `claude-code`, `codex`, `hermes`, `cli`; override with `NOLGIA_SURFACE`) so agent traffic is first-class, not an afterthought.

## Authentication

Two ways to authenticate; every command accepts either.

**Device-code login** (interactive use). Tokens live in your system keyring and refresh automatically:

```bash
nolgia auth login          # approve at the printed https://nolgia.ai/device URL
nolgia auth status         # -> you@example.com (pro)
nolgia auth logout
```

**Personal Access Tokens** (scripts, CI, agents). PATs start with `nol_` and are passed via `--token` or `NOLGIA_TOKEN`:

```bash
nolgia pat create --name build-server   # token is printed once — store it securely
export NOLGIA_TOKEN=nol_...
nolgia account me
```

## Credits

Generation costs credits, drawn from one of two pools depending on how you authenticate:

| Pool | Granted by | Spent by |
|---|---|---|
| Subscription credits | your monthly/yearly plan | device-login sessions (and the web app) |
| API credits | prepaid top-ups (never expire) | PAT-authenticated requests |

If the applicable pool can't cover a generation the API returns `402 Payment Required`. Check balances with `nolgia billing credits`, estimate video jobs with `--cost-only`; buy top-ups and manage your plan from the [billing dashboard](https://nolgia.ai/billing) (`nolgia billing portal` deep-links there).

## Command reference

| Command | Description |
|---|---|
| `nolgia auth login` / `status` / `logout` / `token` | Device-code sign-in, current identity + tier, sign out, print bearer |
| `nolgia gen image --prompt <p> [--model <m>] [--out <file>]` | Generate an image (waits; prints signed URL or saves) |
| `nolgia gen audio --prompt <p>` | Generate audio (TTS, music, SFX) |
| `nolgia gen video --prompt <p> [--shot "S:P\|A"]... [--input <file\|uuid>] [--duration-seconds N] [--aspect-ratio R] [--generate-audio true] [--seed N] [--negative-prompt <p>] [--cost-only] [--no-wait]` | Generate video: multi-shot, image-to-video, native audio, cost estimate |
| `nolgia models list [--modality m]` / `get <id>` | Live model catalog with pricing and capabilities |
| `nolgia status <job-id>` / `wait <job-id> [--timeout <s>]` | Job status; block until finished |
| `nolgia assets list [--limit N] [--modality m] [--tag <t>] [--project-id <id>]` / `get <id> [--out <file>]` / `delete <id>` | List (with tag/project filters), inspect/download, delete assets |
| `nolgia assets tag <id> --tag <t>...` (or `--clear`) | Replace an asset's full tag set (repeat `--tag`; `--clear` removes all) |
| `nolgia characters list` / `get <id>` / `create --name <n> [--description <d>] [--reference-asset-id <id>]...` / `update <id> [--name] [--description] [--reference-asset-id ...]` / `delete <id>` | Reusable characters with up to 4 reference images |
| `nolgia projects list` / `get <id>` / `create --name <n> [--description <d>]` / `update <id> [--name] [--description]` / `delete <id>` | Group assets into projects |
| `nolgia projects add-assets <id> --asset-id <id>...` / `remove-asset <id> <asset-id>` | Add/remove project members (assets themselves are never deleted) |
| `nolgia skills list` / `show <name>` / `install [--target t]` | Bundled AI-agent skills |
| `nolgia account me` / `usage` | Identity; job and asset counts |
| `nolgia billing subscription` / `credits` / `portal` | Plan status, credit pools, Stripe portal link |
| `nolgia pat create --name <n>` / `list` / `revoke <id>` | Manage personal access tokens |
| `nolgia completion <shell>` | Shell completions (bash, zsh, fish, powershell) |

## Global flags and environment

| Flag | Env | Default | Purpose |
|---|---|---|---|
| `--api-url` | `NOLGIA_API_URL` | `https://api.nolgia.ai` | API base URL (the client appends `/v1`) |
| `--token` | `NOLGIA_TOKEN` | keyring | Bearer token (PAT or JWT); overrides the stored login |
| `--json` | — | off | Machine-readable output for scripting |
| — | `NOLGIA_SURFACE` | auto-detected | Calling-surface tag sent as `X-Nolgia-Surface` |

## Shell completions

```bash
# zsh (analogous for bash/fish/powershell)
nolgia completion zsh > "${fpath[1]}/_nolgia" && exec zsh
```

## Development

```bash
cargo build --release      # binary at target/release/nolgia
cargo test --workspace
```

The API client crate (`crates/client`) is generated at build time from the [Nolgia OpenAPI spec](crates/client/openapi.yaml); don't hand-edit generated shapes. Built with `tokio`, `reqwest`, and `clap`.

## License

[MIT](LICENSE)
