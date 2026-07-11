# Nolgia CLI

[![Crates.io](https://img.shields.io/crates/v/nolgia-cli?logo=rust)](https://crates.io/crates/nolgia-cli)
[![npm](https://img.shields.io/npm/v/%40nolgia%2Fcli?logo=npm)](https://www.npmjs.com/package/@nolgia/cli)
[![Homebrew](https://img.shields.io/badge/homebrew-nolgiainc%2Fnolgia-orange?logo=homebrew)](https://github.com/nolgiainc/homebrew-nolgia)
[![Release](https://github.com/nolgiainc/nolgia-cli/actions/workflows/release.yml/badge.svg)](https://github.com/nolgiainc/nolgia-cli/actions/workflows/release.yml)
[![License](https://img.shields.io/crates/l/nolgia-cli)](LICENSE)

The `nolgia` command-line client for the [Nolgia](https://nolgia.ai) generative-media platform. Generate images, video, and audio; inspect the live model catalog; manage assets, projects, and characters; and install the bundled agent instructions or marketplace Abilities.

> **Source versus releases.** `main` is the development source and can contain unreleased commands and flags. The shell installer, Homebrew, prebuilt binaries, npm, and crates.io each install a tagged release. The npm postinstall downloads the binary matching its package version, and a package version without a matching GitHub release cannot install. Check `nolgia --version` and `nolgia --help` for the binary you actually installed. To exercise this checkout, use `cargo run -p nolgia-cli --bin nolgia -- --help`.

## Contents

- [Installation](#installation)
- [Quick start](#quick-start)
- [Generation](#generation)
- [Models and cost estimates](#models-and-cost-estimates)
- [Bundled skills and marketplace Abilities](#bundled-skills-and-marketplace-abilities)
- [Authentication](#authentication)
- [Credits](#credits)
- [Output and scripting](#output-and-scripting)
- [Command index](#command-index)
- [Global flags and environment](#global-flags-and-environment)
- [Shell completions](#shell-completions)
- [Development and spec sync](#development-and-spec-sync)

## Installation

### Shell installer (macOS and Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash
```

The installer uses the latest GitHub release and installs without `sudo` to `~/.local/bin` (falling back to `~/bin`). It adds the selected directory to your shell profile when needed. Use `--prefix <DIR>` for another user-writable directory, `--tag vX.Y.Z` to pin a release, or `--system` when you intentionally want `/usr/local/bin` and will provide any required privilege yourself. Re-running the same version is idempotent.

This command executes a script fetched from the repository's `main` branch and then downloads a release binary. If your environment requires review or provenance checks, save and inspect `install.sh` first and pin the binary with `--tag`; on macOS the script removes the downloaded binary's quarantine attribute so it can run.

### npm

```bash
npm install -g @nolgia/cli
```

The package requires Node 18 or newer and downloads a matching prebuilt binary during postinstall: macOS universal, Linux x86_64, or Windows x86_64. See the [npm README](npm/README.md) for package-specific caveats.

### Homebrew

```bash
brew tap nolgiainc/nolgia
brew install nolgia
```

### Cargo or a prebuilt binary

```bash
cargo install nolgia-cli
```

The public binary is named `nolgia`. Building this Rust 2024 workspace from source on Linux requires `pkg-config` and `libdbus-1-dev` for the keyring dependency. Alternatively, download the platform binary from the [GitHub releases](https://github.com/nolgiainc/nolgia-cli/releases) and put `nolgia` (or `nolgia.exe`) on `PATH`.

## Quick start

```bash
nolgia auth login                         # browser device-code flow
nolgia models list                        # live models, capabilities, and pricing
nolgia gen image --prompt "watercolor fox" --out fox.png
nolgia gen video --prompt "a slow dolly through a studio" --out clip.mp4
nolgia billing credits                    # subscription and API top-up balances
```

Generation is account-backed and consumes the applicable credit pool. Use `nolgia models get <MODEL_ID>` before selecting model-specific options.

## Generation

All generation requests require `--prompt`. The server catalog is authoritative for model IDs, supported durations, aspect ratios, quality tiers, and pricing; do not assume that an option accepted by one model is accepted by another.

### Images

```bash
nolgia gen image --prompt "a paper-cut mountain range" --out mountains.png
nolgia gen image --model <IMAGE_MODEL_ID> --quality <TIER> --prompt "..."
```

`--out` downloads the completed asset. `--quality` is optional and model-specific. Image references belong in a video request; the image command has no working reference-input path in this source tree.

### Video

```bash
# A local image is uploaded, or an existing image asset can be addressed by UUID.
nolgia gen video --model <VIDEO_MODEL_ID> --input portrait.png \
  --prompt "the subject turns toward the camera" --out shot.mp4

# Multi-shot segments use SECONDS:PROMPT, optionally followed by |AUDIO DIRECTION.
nolgia gen video --prompt "gritty 35mm film look" \
  --shot "4:wide shot of a rural road|engine and wind" \
  --shot "3:the driver checks the radio|static cuts out" \
  --generate-audio true --out sequence.mp4
```

`--input` is a local image path or an existing image asset UUID; the selected model must support image input. `--video-ref <ASSET_UUID>` (repeat up to three) and `--element <ASSET_UUID>` (repeat up to nine) are for models that advertise reference-to-video support. Reference videos must be MP4/MOV, 480p–720p, 2–15 seconds, and 50 MB combined; reference prompts address them as `@Video1`…`@Video3` and elements as `@Image1`…`@Image9`. `--end-frame <ASSET_UUID|FILE>` pins a final image and requires `--input`. `--quality` and `--bitrate` are validated against the model's published capabilities. `--negative-prompt`, `--aspect-ratio`, `--duration-seconds`, and `--seed` are also model-dependent.

Use `--cost-only` to query the live catalog and print an estimate without creating a job. It is an estimate, not a reservation or a hard-coded price.

### Audio

```bash
nolgia gen audio --model <AUDIO_MODEL_ID> --prompt "rain on a window" --out rain.mp3
nolgia gen audio --model <TTS_MODEL_ID> --voice <VOICE_ID> --prompt "Welcome" --format mp3
```

Discover voices with `nolgia models get <AUDIO_MODEL_ID>`; `--format` selects the CLI's supported output format. The server validates model-specific audio options.

## Models and cost estimates

```bash
nolgia models list
nolgia models list --modality video
nolgia models get <MODEL_ID>
nolgia gen video --model <MODEL_ID> --prompt "..." --cost-only
```

The catalog includes modality, credit pricing, duration and aspect-ratio support, image-input support, quality tiers, reference limits, and (for audio) voices. New models and capability changes can appear without a CLI release.

## Bundled skills and marketplace Abilities

These are separate surfaces:

- **Bundled skills** are embedded `SKILL.md` packs that teach an agent how to use Nolgia. They install locally and do not call the API:

  ```bash
  nolgia skills list
  nolgia skills show nolgia-platform
  nolgia skills install --target claude-user
  nolgia skills install --target claude-project
  nolgia skills install --target hermes
  nolgia skills install --target dir --dir ./agent-skills
  ```

  The three bundled packs are `nolgia-platform`, `nolgia-video-prompting`, and `nolgia-ugc-ads`. `--force` is required to overwrite an existing file. The Hermes target writes to `$HERMES_HOME/skills` and defaults `HERMES_HOME` to `/opt/data` when it is unset.

- **Marketplace Abilities** are registry-backed packages installed for a Hermes agent through the API. The package manifest is `ability.json`; its agent instructions remain `SKILL.md` for Hermes compatibility:

  ```bash
  nolgia ability list
  nolgia ability show <ABILITY_SLUG>
  nolgia ability installed
  nolgia ability install <ABILITY_SLUG>
  nolgia ability uninstall <ABILITY_SLUG>
  nolgia ability sync --dir "${HERMES_HOME:-/opt/data}/skills"
  ```

  Administrators can author and publish an Ability with `nolgia ability init <SLUG>`, `nolgia ability pack <DIR>`, and `nolgia ability publish <DIR>`. `publish` is an admin-only API operation; `init` and `pack` work locally. `ability pack` passes the optional `python_requirements` manifest field through to the marketplace. Synced marketplace directories carry a `.nolgia-ability.json` version marker. Review an Ability's `ability.json`, `SKILL.md`, and payload before installing it: Hermes may execute the instructions and code it contains. Marketplace commands use the `ability` name; the separate bundled command remains `skills`.

## Authentication

Networked commands resolve a bearer token from `--token`, then `NOLGIA_TOKEN`, then the stored device-login token. Local commands such as `skills`, `completion`, and Ability `init`/`pack` do not need a token.

### Device login

```bash
nolgia auth login
nolgia auth status       # `whoami` is an alias
nolgia auth token        # print the resolved access token for a script
nolgia auth logout
```

The default store is `${XDG_CONFIG_HOME:-$HOME/.config}/nolgia/tokens.json`. On Unix, the CLI creates a `0600` file in a `0700` directory; Windows uses the platform's normal file ACLs. This avoids repeated macOS keychain prompts after upgrades. Set `NOLGIA_TOKEN_STORE=file` to use only that file and never probe the keyring; set `NOLGIA_TOKEN_STORE=keyring` to opt into the OS keyring. With the variable unset, a one-time migration read may import an older keyring token into the file store.

### Personal access tokens

```bash
nolgia pat create --name build-server   # shown once; store it securely
export NOLGIA_TOKEN=nol_...
nolgia account me
nolgia pat list
nolgia pat revoke <PAT_UUID>
```

Use PATs for CI, scripts, and agents. Prefer `NOLGIA_TOKEN` or a secret manager over `--token`: command-line arguments can appear in shell history and process listings. Do not put a token in a README or command log.

## Credits

`nolgia billing credits` reports subscription and API top-up balances separately, plus the overall total (use `--json` for additional fields). Device-login sessions and PAT-authenticated requests use the credential-appropriate pool; the API rejects a generation when that pool cannot cover it. `billing subscription` shows plan status, and `billing portal` prints a Stripe customer-portal link. `account usage` reports the number of job and asset items on its default visible pages, not credit spend or an all-account total.

## Output and scripting

`--json` is a global flag for machine-readable output on commands that implement a JSON response. It is not a promise that every invocation emits JSON. In particular:

- `gen ... --no-wait` prints `{"job_id":"..."}` so it can be passed to `wait`.
- `gen video --cost-only` prints a human-readable estimate and creates no job.
- `auth login` and `auth status`/`whoami` print human prompts/status text even when `--json` is also supplied; `auth token` prints the resolved access token, `skills show` prints the `SKILL.md`, and `completion <SHELL>` prints shell code.

For a fire-and-poll script:

```bash
job_uuid=$(nolgia gen video --prompt "..." --no-wait | jq -r .job_id)
nolgia wait "$job_uuid" --timeout 600 --json | jq .asset.signed_url
```

Human output otherwise depends on the command: completed image/audio generations print a signed URL, completed video prints the job UUID and status, and `--out <FILE>` downloads the asset. Signed URLs are temporary bearer capabilities; avoid sending them to persistent CI logs or telemetry, and save the file or query the asset again when needed.

## Command index

Use `nolgia <COMMAND> --help` (and, where applicable, `nolgia <COMMAND> <SUBCOMMAND> --help`) for the complete flags and current server-facing details.

Replace every `<PLACEHOLDER>` below with a real value; angle-bracket placeholders are documentation notation, not literal shell arguments. Flag highlights include `assets list --limit/--cursor/--modality/--tag/--project-id`, `assets get --out`, `assets tag --tag` (repeatable) or `--clear`, `assets frame --at/--last/--out`, `projects create/update --auto-tag` and `update --clear-auto-tags`, `projects add-assets --asset-id` (repeatable), and up to four `characters ... --reference-asset-id` values. `ability sync`/`init` accept `--dir`, while `ability pack` accepts `--out`, as shown by their help. Video waits by default; use `--no-wait` for the JSON job object and `--timeout` to bound waiting.

| Command | Subcommands and purpose |
|---|---|
| `auth` | `login`, `logout`, `status`/`whoami`, `token` |
| `gen` | `image`, `video`, `audio` generation |
| `status`, `wait` | Inspect or wait for a job by UUID |
| `assets` | `list`, `get`, `delete`, `upload`, `tag`, `frame` |
| `characters` | `list`, `get`, `create`, `update`, `delete` reusable characters |
| `projects` | `list`, `get`, `create`, `update`, `delete`, `add-assets`, `remove-asset` |
| `account` | `me`, `usage` (identity and default-page job/asset item counts) |
| `billing` | `subscription`, `credits`, `portal` |
| `pat` | `create`, `list`, `revoke` personal access tokens |
| `skills` | `list`, `show`, `install` embedded agent packs |
| `ability` | `list`, `show`, `installed`, `install`, `uninstall`, `sync`, `init`, `pack`, `publish` marketplace Abilities |
| `models` | `list`, `get` live catalog |
| `completion` | `bash`, `zsh`, `fish`, `elvish`, or `powershell` completion script |

## Global flags and environment

| Flag or variable | Default | Purpose |
|---|---|---|
| `--api-url` / `NOLGIA_API_URL` | `https://api.nolgia.ai` | API base URL; the client appends `/v1` unless already present |
| `--token` / `NOLGIA_TOKEN` | stored login | Bearer token; an explicit flag wins |
| `--json` | off | Request structured output where the command supports it |
| `NOLGIA_TOKEN_STORE` | file with one-time migration | `file` disables keyring access; `keyring` opts into the OS keyring |
| `NOLGIA_SURFACE` | auto-detected | Override the `X-Nolgia-Surface` value sent with API requests; any non-empty value also suppresses update hints |
| `NOLGIA_NO_UPDATE_CHECK` | unset | Disable the once-per-day release hint |
| `XDG_CONFIG_HOME` | `$HOME/.config` | Parent for token and install-metadata files |
| `XDG_STATE_HOME` | `$HOME/.local/state` | Parent for the update-check cache |
| `HERMES_HOME` | `/opt/data` for Hermes targets | Parent of the Hermes `skills` directory |

The update hint reads a local cache and refreshes it in the background at most once per day. It is suppressed for JSON output, CI, non-interactive stderr, and any non-empty `NOLGIA_SURFACE`; a short best-effort grace may run at process exit. Set `NOLGIA_NO_UPDATE_CHECK=1` when a completely quiet invocation is required.

## Shell completions

```bash
nolgia completion zsh > "${fpath[1]}/_nolgia"
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions"
nolgia completion bash > "${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions/nolgia"
```

Use the equivalent `fish`, `elvish`, or `powershell` subcommand for those shells.

## Development and spec sync

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
bash tests/install_sh_test.sh
cargo build --release --locked
```

The Rust client is generated at build time from the vendored [OpenAPI snapshot](crates/client/openapi.yaml). CI compares it with the canonical API contract. Do not hand-edit generated client output. For local development, the sibling `nolgia-api` spec is used only when you explicitly set `NOLGIA_USE_SIBLING_SPEC=1`; otherwise builds use the vendored snapshot. The release workflow publishes tagged crates and release binaries, then attempts npm publishing only when `npm/package.json` matches the tag; a mismatch fails that npm job. A commit on `main` is not itself a release.

## License

[MIT](LICENSE)
