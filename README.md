# Nolgia CLI

Rust CLI for the Nolgia generation platform.

## Installation

You can install the CLI directly using Cargo:

```bash
cargo install nolgia-cli
```

Alternatively, download the latest binary from the [GitHub Releases](https://github.com/nolgiacorp/nolgia-cli/releases) page. The binary is named `nolgia`.

## Authentication

The CLI supports two ways to authenticate:

**Device-code login (interactive).** Tokens are stored in the system keyring and refreshed automatically.

```bash
nolgia auth login          # opens a device-code flow; approve at the printed URL
nolgia auth status         # prints "email (tier)", e.g. admin@nolgia.ai (pro)
nolgia auth logout
```

**Personal Access Token (scripts, CI, agents).** Create a PAT with `nolgia pat create` (or the web dashboard) and pass it with `--token` or the `NOLGIA_TOKEN` env var. PATs start with `nol_`.

```bash
export NOLGIA_TOKEN=nol_...
nolgia auth status         # works with --token / NOLGIA_TOKEN too
```

## Credits and subscription

Generation costs credits. There are two pools, and which one a request spends from depends on how you authenticate:

- **Subscription credits** (`app_subscription`): granted by your monthly or yearly plan. Spent by requests authenticated with a **device-login session** (and by the web app).
- **API credits** (`shared_topup`): prepaid top-ups purchased from the billing dashboard; they never expire. Requests authenticated with a **PAT** spend only from this pool, regardless of subscription tier.

If the applicable pool cannot cover a generation, the API returns `402 Payment Required` and the CLI surfaces it. Top up from the web billing page (`nolgia billing portal` opens Stripe for plan management).

```bash
nolgia billing subscription   # tier + status, e.g. "pro active"
nolgia billing credits        # both pools, e.g. "subscription: 546631 (resets with plan)  api top-ups: 0"
nolgia billing portal         # prints a Stripe customer-portal URL
nolgia account usage          # job and asset counts
```

## Commands

### Generation

```bash
# Generate an image (waits and prints a signed URL; --out saves the file)
nolgia gen image --prompt "A serene mountain lake" --out lake.png

# Generate audio
nolgia gen audio --prompt "Lofi hip hop beats for studying"

# Generate video (async; --no-wait returns the job id immediately)
nolgia gen video --prompt "A drone shot over a coastline"

# Full video control: duration, framing, reproducibility, native audio
nolgia gen video --prompt "A drone shot over a coastline" \
  --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
  --duration-seconds 12 --aspect-ratio 16:9 --seed 42 \
  --negative-prompt "text, watermarks" --generate-audio true \
  --out coastline.mp4

# Image-to-video: --input uploads the file to /assets and passes it as the start image
nolgia gen video --model fal-ai/kling-video/v3/pro/image-to-video \
  --input character.png --prompt "she turns toward the camera and smiles"

# Multi-shot sequences (up to 8 shots; clip duration = sum; --prompt is style/context;
# "|" separates an optional per-shot audio direction)
nolgia gen video --model fal-ai/bytedance/seedance/v2/pro/text-to-video \
  --prompt "Gritty 35mm film look." --generate-audio true \
  --shot "8:WIDE SHOT. Rural highway, a single car heading south.|engine, wind" \
  --shot "4:MCU. The driver glances at the dead radio.|AM static cuts out"
```

Model selection is via `--model`; defaults are `flux-pro` (image), `fal-ai/stable-audio-25/text-to-audio` (audio), and `fal-ai/kling-video/v3/text-to-video` (video). Video durations are model-dependent: Kling v3 3–15s, Seedance v2 Pro 4–15s, Veo 3.1 exactly 4/6/8s. `--generate-audio` is honored by Seedance and Veo. `--shot` and `--generate-audio` require an API deployment with multi-shot support (nolgia-api `feat/video-generate-audio`).

### Jobs

```bash
nolgia status <job-id>     # current job status
nolgia wait <job-id>       # block until the job finishes (default timeout 300s)
```

### Assets

```bash
nolgia assets list [--limit N] [--modality image|video|audio]
nolgia assets get <asset-id> [--out file]   # metadata, or download with --out
nolgia assets delete <asset-id>
```

### Account

```bash
nolgia account me          # id + email for the current token
nolgia account usage       # job and asset counts
nolgia billing credits     # subscription vs API credit pools
```

### API access tokens

Personal Access Tokens authenticate scripts, CI, and agents; they spend the prepaid API credit pool (see [Credits and subscription](#credits-and-subscription)).

```bash
nolgia pat create --name my-laptop-cli   # prints the plaintext token ONCE; store it securely
nolgia pat list                          # id, name, prefix, created, last used
nolgia pat revoke <pat-id>
```

## Global flags and environment

| Flag | Env | Default | Purpose |
|---|---|---|---|
| `--api-url` | `NOLGIA_API_URL` | `https://api.nolgia.ai` | API base URL (the client appends `/v1`) |
| `--token` | `NOLGIA_TOKEN` | keyring | Bearer token (PAT or JWT); overrides the stored login |
| `--json` | — | off | Machine-readable output |

## Development Quickstart

Ensure you have the Rust 2024 edition toolchain installed.

```bash
cargo build --release      # binary at target/release/nolgia
cargo test --workspace
```

The API client crate is generated from `nolgia-api/api/openapi.yaml` (see `crates/client/build.rs`); do not hand-edit generated API shapes. The CLI is built with `tokio`, `reqwest`, and `clap`.
