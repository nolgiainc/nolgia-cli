# Nolgia CLI

[![Crates.io](https://img.shields.io/crates/v/nolgia-cli)](https://crates.io/crates/nolgia-cli)
[![Release](https://github.com/nolgiacorp/nolgia-cli/actions/workflows/release.yml/badge.svg)](https://github.com/nolgiacorp/nolgia-cli/actions/workflows/release.yml)
[![Downloads](https://img.shields.io/crates/d/nolgia-cli)](https://crates.io/crates/nolgia-cli)
[![License](https://img.shields.io/crates/l/nolgia-cli)](#license)

Command-line client for the [Nolgia](https://nolgia.ai) generative media platform — image, audio, and video generation from your terminal, scripts, and agents.

```console
$ nolgia gen image --prompt "A serene mountain lake at dawn" --out lake.png
$ nolgia gen video --prompt "A drone shot over a coastline" --no-wait
9b2f5c1e-...   # job id; check with `nolgia wait <id>`
```

> A [Nolgia account](https://nolgia.ai) with an active subscription or prepaid API credits is required.

## Installation

### Homebrew (macOS, Linux)

```bash
brew tap nolgiacorp/nolgia
brew install nolgia
```

Homebrew prompts once to trust third-party taps: `brew trust nolgiacorp/nolgia`.

### Cargo

```bash
cargo install nolgia-cli
```

Building from source requires the Rust 2024 toolchain; on Linux you also need `libdbus-1-dev` and `pkg-config` for keyring support.

### Prebuilt binaries

Download the binary for your platform from the [latest release](https://github.com/nolgiacorp/nolgia-cli/releases/latest) (macOS universal, Linux x86_64, Windows x86_64), rename it to `nolgia` (`nolgia.exe` on Windows), and place it on your `PATH`.

## Quick start

```bash
nolgia auth login                                  # device-code sign-in via your browser
nolgia gen image --prompt "watercolor fox" --out fox.png
nolgia assets list --limit 5
nolgia billing credits                             # see what you have left
```

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

If the applicable pool can't cover a generation the API returns `402 Payment Required`. Check balances with `nolgia billing credits`; buy top-ups and manage your plan from the [billing dashboard](https://nolgia.ai/billing) (`nolgia billing portal` deep-links there).

## Command reference

| Command | Description |
|---|---|
| `nolgia auth login` / `status` / `logout` | Device-code sign-in, current identity + tier, sign out |
| `nolgia gen image --prompt <p> [--model <m>] [--out <file>]` | Generate an image (waits; prints signed URL or saves) |
| `nolgia gen audio --prompt <p>` | Generate audio |
| `nolgia gen video --prompt <p> [--no-wait] [--input <img>]` | Generate video (async; `--input` for image-to-video) |
| `nolgia status <job-id>` | Current status of a job |
| `nolgia wait <job-id> [--timeout <s>]` | Block until a job finishes (default 300s) |
| `nolgia assets list [--limit N] [--modality m]` | List generated assets |
| `nolgia assets get <id> [--out <file>]` | Asset metadata, or download with `--out` |
| `nolgia assets delete <id>` | Delete an asset |
| `nolgia account me` / `usage` | Identity; job and asset counts |
| `nolgia billing subscription` / `credits` / `portal` | Plan status, credit pools, Stripe portal link |
| `nolgia pat create --name <n>` / `list` / `revoke <id>` | Manage personal access tokens |

Model selection is via `--model`; defaults are `flux-pro` (image), `fal-ai/stable-audio-25/text-to-audio` (audio), and `fal-ai/kling-video/v3/text-to-video` (video).

## Global flags and environment

| Flag | Env | Default | Purpose |
|---|---|---|---|
| `--api-url` | `NOLGIA_API_URL` | `https://api.nolgia.ai` | API base URL (the client appends `/v1`) |
| `--token` | `NOLGIA_TOKEN` | keyring | Bearer token (PAT or JWT); overrides the stored login |
| `--json` | — | off | Machine-readable output for scripting |

## Development

```bash
cargo build --release      # binary at target/release/nolgia
cargo test --workspace
```

The API client crate (`crates/client`) is generated at build time from the [Nolgia OpenAPI spec](crates/client/openapi.yaml); don't hand-edit generated shapes. Built with `tokio`, `reqwest`, and `clap`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
