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
- [Installation for AI coding agents](INSTALL_FOR_AGENTS.md)
- [Quick start](#quick-start)
- [Generation](#generation)
- [Models and cost estimates](#models-and-cost-estimates)
- [Bundled skills and marketplace Abilities](#bundled-skills-and-marketplace-abilities)
- [Authentication](#authentication)
- [Credits](#credits)
- [Organizations](#organizations)
- [Output and scripting](#output-and-scripting)
- [Raw API requests](#raw-api-requests)
- [Command index](#command-index)
- [Global flags and environment](#global-flags-and-environment)
- [Shell completions](#shell-completions)
- [Development and spec sync](#development-and-spec-sync)

## Installation

Installing from an AI coding agent? Paste [INSTALL_FOR_AGENTS.md](INSTALL_FOR_AGENTS.md) into it.

### Shell installer (macOS and Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash
```

The installer picks the binary for your platform (macOS universal; Linux x86_64 or arm64, arm64 from the first release after v0.2.26) from the latest GitHub release and installs without `sudo` to `~/.local/bin` (falling back to `~/bin`). It adds the selected directory to your shell profile when needed. Use `--prefix <DIR>` for another user-writable directory, `--tag vX.Y.Z` to pin a release, or `--system` when you intentionally want `/usr/local/bin` and will provide any required privilege yourself. Re-running the same version is idempotent.

Downloads are checked against the release's `SHA256SUMS` using the first available `sha256sum`, `shasum`, or `openssl`; mismatches or missing asset entries fail, while older releases without sums and machines without a digest tool receive a notice and continue.
Every successful run ends stdout with `export PATH="<PREFIX>:$PATH"`; run that line in your current shell to use the binary immediately, including after a no-op reinstall.

#### Supported platforms

macOS (universal, 11+) and Linux both work everywhere. The Linux binaries are **statically linked musl builds that depend on no system library at all**, so they start on Debian 12, Ubuntu 22.04, Alpine and slim container images as well as current distributions. They are published both as `nolgia-<arch>-unknown-linux-musl` and, for compatibility, under the historical `-gnu` names. The OS keyring is not compiled into them: the default token store is the same `0600` file used everywhere, and `NOLGIA_TOKEN_STORE=keyring` reports that plainly instead of failing obscurely. Build from source (`cargo install nolgia-cli`) for keyring support on Linux.

This command executes a script fetched from the repository's `main` branch and then downloads a release binary. If your environment requires review or provenance checks, save and inspect `install.sh` first and pin the binary with `--tag`; on macOS the script removes the downloaded binary's quarantine attribute so it can run.

### npm

```bash
npm install -g @nolgia/cli
```

The package requires Node 18 or newer and downloads a matching prebuilt binary during postinstall: macOS universal, Linux x86_64 or arm64, or Windows x86_64 or arm64 (the arm64 builds ship from the first release after v0.2.26). See the [npm README](npm/README.md) for package-specific caveats.

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

All generation requests require `--prompt`, except `gen image --expand-to`, where it is optional. The server catalog is authoritative for model IDs, supported durations, aspect ratios, quality tiers, and pricing; do not assume that an option accepted by one model is accepted by another. Every `gen` subcommand (and `assets upload`) accepts `--project-id <PROJECT_UUID>` to file the resulting asset(s) into one of your projects at creation (`nolgia projects list` for ids); without it, assets land in your default Library project.

### Images

```bash
nolgia gen image --prompt "a paper-cut mountain range" --out mountains.png
nolgia gen image --model <IMAGE_MODEL_ID> --quality <TIER> --prompt "..."

# Outpaint: grow an image you already have to a new ratio (flux-expand).
nolgia gen image --expand-to 9:16 --input still.png --prompt "more of the beach" --out vertical.png
```

`--out` downloads the completed asset. `--quality` is optional and model-specific. `--input <FILE|ASSET_UUID>` sends one reference image on models that accept one (`nolgia models get <IMAGE_MODEL_ID>`). `--expand-to <RATIO>` outpaints that reference: it keeps the source pixels and paints new content into the added margins, on `flux-expand` unless `--model` names another model that publishes `image_expand`; `--prompt` is optional there and describes the new area, and the ratio must be one the model lists.

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

Discover voices with `nolgia voices list` (every TTS model) or `nolgia voices list --model <TTS_MODEL_ID>`; `--format` selects the CLI's supported output format. The server validates model-specific audio options.

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

  The three bundled packs are `nolgia-platform`, `nolgia-video-prompting`, and `nolgia-ugc-ads`. Re-runs install missing packs, leave identical files unchanged, and skip differing copies without failing. Add `--force` to overwrite differing files; every pack reports its outcome. The Hermes target writes to `$HERMES_HOME/skills` and defaults `HERMES_HOME` to `/opt/data` when it is unset.

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
nolgia auth login --no-browser   # print the link and code only
nolgia auth status       # `whoami` is an alias
nolgia auth token        # print the resolved access token for a script
nolgia auth logout
```

`auth login` prints the approval link, opens it in your default browser when one is available (skip that with `--no-browser`), and shows the code on its own line for typing it in at `nolgia.ai/device` from another device. While it waits it shows one status line with the time left on the code; on success it prints `Connected as <email>`. If the code runs out before it is approved the command exits with an error that says so; run `nolgia auth login` again for a fresh code. With `--json`, stdout carries only the JSON result and the narration goes to stderr.

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

## Organizations

Team and Enterprise plans put several users in one organization with one shared credit pool and one shared library. You work either in your personal space or inside one active organization, and that choice is stored server-side (`PUT /me/active-organization`), so the web app, MCP, and every credential you hold follow it on their next request. `nolgia org` (alias `nolgia workspace`) manages that context:

```bash
nolgia org list                              # your organizations, role in each, active one starred
nolgia org status                            # active context plus the effective plan (seats in an organization)
nolgia org switch acme-studios               # by slug or UUID; server-side, follows you everywhere
nolgia org switch personal                   # back to your personal space
nolgia org create "Acme Studios" --slug acme-studios
nolgia org members                           # user_id, email, name, role, budget, joined
nolgia org invite ada@example.com --role member   # accept link printed once; roles admin|billing|member|viewer
nolgia org credits                           # shared pool balance and per-member spend this month
```

`members`, `invite`, and `credits` address the active organization by default. Pass `--org <slug|id>` or set `NOLGIA_ORG=<slug|id>` to address another organization you belong to without switching the server-side context (the API has no per-request override yet, so this only selects which organization the command reads). `switch` and the selector both refuse an organization you are not a member of and list the ones you are.

Credit semantics in an organization context: every generation, whether authenticated by device login or a PAT, spends the organization's shared credit pool, and per-member monthly budgets apply. In the personal space a PAT still draws only from your prepaid API top-up pool. `nolgia auth status` prints an `Organization:` line so a script can confirm where a token's requests will land before spending.

## Output and scripting

`--json` is a global flag for machine-readable output on commands that implement a JSON response. It is not a promise that every invocation emits JSON. In particular:

- `gen ... --no-wait` prints `{"job_id":"..."}` so it can be passed to `wait`.
- `gen video --cost-only` prints a human-readable estimate and creates no job.
- `auth login` and `auth status`/`whoami` print human prompts/status text even when `--json` is also supplied; `auth token` prints the resolved access token, `skills show` prints the `SKILL.md`, and `completion <SHELL>` prints shell code.

For a fire-and-poll script:

```bash
job_uuid=$(nolgia gen video --prompt "..." --no-wait --field job_id)
nolgia wait "$job_uuid" --timeout 600 --field asset.signed_url
```

### Field selection and output formats

`--field <PATH>` selects a value from a command's JSON response and implies JSON
output. Repeat it to print several values in order, one per line. Paths use
literal dotted keys and zero-based array indexes: `asset.signed_url`,
`items[0].id`, `models[0].cost.credits`, or `[0].name` for a top-level array.
A leading dot is optional (`.id` is `id`); wildcards, filters, and fallback
expressions are not supported. A Job's asset URL is `asset.signed_url`, not
`asset.url`; `/pricing/models` returns its catalog under `models`.

`--output <json|table|value>` also implies JSON output and controls rendering:

- `json`: pretty JSON, like `--json`. One selected field is its value; multiple
  fields become an array in selection order.
- `value`: bare strings, numbers, booleans, and `null`; objects and arrays use
  compact JSON. This is the default with `--field`. Without fields, it prints
  the whole response on one line.
- `table`: aligned columns separated by two spaces, with one row per object in
  an array. Paginated responses such as `jobs list` render the rows in `items`.
  Scalar arrays use a `VALUE` column, other objects use `KEY` and `VALUE`, and
  scalars print directly.

These flags work before or after subcommands on JSON-producing commands. With no
new flags, existing text and `--json` output stay the same. A missing key, an
out-of-range index, or traversal through a scalar exits `1`, prints nothing to
stdout, and names the failing field and available keys or array length on stderr.
The exit-code reports for a live job (`75`), content-filter block (`65`), or agent
refusal (`77`) always print their full JSON object when JSON output is requested;
field selection and output formatting never trim these recovery details.

### Exit code 75: a job is live, do not re-run

`wait` and `gen` exit **75** (sysexits `EX_TEMPFAIL`) to mean *the job exists
and is still running* — the long-poll window closed, Ctrl-C arrived, the
connection dropped after a successful submission, or the API refused a
duplicate submission naming the job it already created. None of these is a
failure, and **re-submitting starts a second billable job**. Every one of them
prints the job id and the commands to follow it; under `--json` stdout also
carries `{"job_id", "outcome", "billed_twice", "follow_up"}` while the human
text goes to stderr. Other failures exit `1`; a content-filter block exits `65`
(below).

So a polling loop keeps waiting rather than giving up or re-submitting:

```bash
while true; do
  nolgia wait "$job_uuid" --timeout 300 --json > job.json && break
  status=$?
  # 75: the long-poll expired and the job is still running — keep waiting.
  # Anything else is a real error; re-submitting would bill a second job.
  [ "$status" -eq 75 ] || exit "$status"
done
```

Stopping the wait never stops the job. To stop the job itself, cancel it on
the server with `nolgia jobs cancel <JOB_ID>` (below); every exit-75 report
offers that command last, after the ones that keep following the job.

Generation is stochastic, so an identical prompt is sometimes a deliberate
second take rather than an accidental re-run. Pass a fresh `--idempotency-key`
(or `NOLGIA_IDEMPOTENCY_KEY`) to say so; reuse one to collapse your own retries
into a single job.

Human output otherwise depends on the command: completed image/audio generations print a signed URL, completed video prints the job UUID and status, and `--out <FILE>` downloads the asset. Signed URLs are temporary bearer capabilities; avoid sending them to persistent CI logs or telemetry, and save the file or query the asset again when needed.

### Exit code 65: blocked by the content filter

`gen image|video|audio` and `restore video` exit **65** when waiting reveals
that the provider's content filter refused the request or blocked the result.
The message names the content filter, repeats the provider's reason, and states
the refund truth from `failure.credits_refunded`: `true` means refunded, `false`
means charged, and absent or null means no refund outcome was recorded. Edit the
prompt or reference media, or switch models, before running the command again.

With `--json`, stdout carries the full failed Job with `failure.kind: "moderated"`,
and the human text goes to stderr. `wait` and `status` report the job and exit `0`
for terminal jobs, including moderated ones. They also print the content-filter
message to stderr in text mode. To inspect a known moderated job, select its
`failure.kind` through `wait` (which exits `0`):

```bash
nolgia wait "$job_uuid" --field failure.kind
```

### Canceling a job

`nolgia jobs cancel <JOB_ID>` cancels a queued or running job on the server.
The job ends as `canceled` straight away and is never delivered, whatever the
model provider does next. The command prints the job's status line, the
server's own sentence about what the provider did and what happened to the
credits, then the credits: `Credits: 30 refunded.`, `Credits: 20 refunded, 10
charged.` (the provider stopped part way and bills what it rendered), or
`Credits: 30 charged.`. A job that had not reached the provider is refunded in
full. When the provider has not answered yet the credits read `pending`: the
job is already terminal, so check how they settle later with
`nolgia jobs get <JOB_ID>` (`wait` returns at once). `--json` prints the
canceled Job, whose `cancellation` object carries `stage`, `provider_cancel`,
`settlement`, `credits_refunded`, `credits_charged` and `message`. Canceling
twice prints the same result.

A job that already finished, or whose result is being delivered, cannot be
canceled (`409 job_not_cancellable`: nothing was changed). A job that is not in
your library in the active workspace answers `404`, and an organization role
that may not cancel it (viewers and billing contacts, or a member touching a
teammate's job) answers `403`. Each exits `1` with the server's detail and what
to do next.

`canceled` is a terminal state of its own, not a failure: `status`, `jobs get`
and `wait` exit `0` for it and print the cancel sentence and credits on stderr.
A `gen` or `restore` command whose job is canceled while it waits exits `1`
(there is no result), prints the same explanation instead of `Error:`, and
under `--json` puts the canceled Job on stdout.

## Raw API requests

Use `nolgia api <METHOD> <PATH>` for the CLI equivalent of an authenticated curl
request. It uses the same token or stored login and `--api-url` as other commands;
public routes also work without a token. JSON responses are printed automatically
(`--json` is implied), so field selection works directly:

```bash
nolgia api POST /jobs/<id>/sse-ticket --field ticket
nolgia api GET /me --field email
nolgia api POST /generate/image --body '{"model":"flux-pro","prompt":"..."}' --field id
nolgia api GET /pricing/models --field 'models[0].id'
```

Replace `<id>` with a real job UUID. Methods are case-insensitive `GET`, `POST`,
`PUT`, `PATCH`, `DELETE`, and `HEAD`. The path must begin with `/`; absolute URLs
are refused so credentials cannot be sent to another host. A leading `/v1` is
stripped because the client already includes it: `/pricing/models` and
`/v1/pricing/models` address the same route.

`--body` accepts inline JSON, `@file.json`, or `-` to read JSON from stdin; invalid
JSON fails before making a request. POST, PUT, and PATCH without a body send
`Content-Length: 0`. Repeat `--query KEY=VALUE` for URL-encoded query parameters
and `--header 'NAME: VALUE'` for additional headers. Non-JSON responses pass
through unchanged, and empty responses print nothing. A non-2xx response prints
its full body without field selection, reports the status and problem detail on
stderr, and exits `1`. Workspace switching and creation keep the same agent
refusals as `org switch` and `org create`.

## Command index

Use `nolgia <COMMAND> --help` (and, where applicable, `nolgia <COMMAND> <SUBCOMMAND> --help`) for the complete flags and current server-facing details.

Replace every `<PLACEHOLDER>` below with a real value; angle-bracket placeholders are documentation notation, not literal shell arguments. Flag highlights include `gen image/video/audio --project-id` and `assets upload --project-id` (file the result into a project), `assets list --limit/--cursor/--modality/--tag/--project-id`, `assets get --out`, `assets tag --tag` (repeatable) or `--clear`, `assets frame --at/--last/--out`, `projects create/update --auto-tag` and `update --clear-auto-tags`, `projects add-assets --asset-id` (repeatable), and up to four `characters ... --reference-asset-id` values. `ability sync`/`init` accept `--dir`, while `ability pack` accepts `--out`, as shown by their help. Video waits by default; use `--no-wait` for the JSON job object and `--timeout` to bound waiting.

| Command | Subcommands and purpose |
|---|---|
| `auth` | `login`, `logout`, `status`/`whoami` (email, plan, and the active organization or personal space), `token` |
| `api` | `<METHOD> <PATH>` sends an authenticated request with optional `--body`, repeatable `--query`, and repeatable `--header`; JSON output is implied |
| `gen` | `image`, `video`, `audio` generation |
| `restore` | `video` footage restoration/upscale (de-noise, de-haze, up-res to a target tier) on `seedvr2-restore` or a `topaz-*` master upscaler |
| `status`, `wait` | Inspect or wait for a job by UUID |
| `jobs` | `get <JOB_ID>` inspects a job exactly like `status`; `list` your jobs, newest first, with `--status queued\|running\|succeeded\|failed\|canceled`, `--modality image\|video\|audio`, `--limit` and `--cursor`; `cancel <JOB_ID>` stops a queued or running job on the server and prints what happened to its credits |
| `assets` | `list`, `get`, `delete`, `upload`, `tag`, `frame` |
| `characters` | `list`, `get`, `create`, `update`, `delete` reusable characters |
| `projects` | `list`, `get`, `create`, `update`, `delete`, `add-assets`, `remove-asset` |
| `products` | `list`, `import <URL> [--project-id]`, `get`, `delete` products imported from a store link (reusable with `product_id` on the API) |
| `account` | `me`, `usage` (identity and default-page job/asset item counts) |
| `billing` | `subscription`, `credits`, `portal` |
| `pat` | `create`, `list`, `revoke` personal access tokens |
| `org` (alias `workspace`) | `list`, `status`, `switch <slug\|id\|personal>`, `create <name> [--slug]`, `members`, `invite <email> --role`, `credits`; the last three accept `--org <slug\|id>` (or `NOLGIA_ORG`) |
| `skills` | `list`, `show`, `install` embedded agent packs; repeated installs report installed, unchanged, or skipped packs, and `--force` replaces differing copies |
| `ability` | `list`, `show`, `installed`, `install`, `uninstall`, `sync`, `init`, `pack`, `publish` marketplace Abilities |
| `models` | `list`, `get` live catalog |
| `voices` | `list [--model <TTS_MODEL_ID>]` the voice ids `gen audio --voice` accepts, from the live catalog |
| `motions` | `list` the camera-move library (push-in, orbit, crane, rack focus, ...) for `gen video --motion <id> [--motion-strength subtle\|medium\|strong]`; the server appends the move to your prompt |
| `color-presets` | `list` the built-in color-grade preset looks for Studio compositions; `cube <slug> [-o FILE]` downloads the `.cube` LUT |
| `masks` | `validate <MASK>` runs the timeline-mask sanitizer on inline JSON, `@file`, or `-` (stdin) and prints the canonical mask plus every clamp/drop diagnostic (`--strict` exits 1 on any problem); `example rectangle`, `ellipse`, or `polygon` prints a contract-true starter mask offline |
| `completion` | `bash`, `zsh`, `fish`, `elvish`, or `powershell` completion script |

## Global flags and environment

| Flag or variable | Default | Purpose |
|---|---|---|
| `--api-url` / `NOLGIA_API_URL` | `https://api.nolgia.ai` | API base URL; the client appends `/v1` unless already present |
| `--token` / `NOLGIA_TOKEN` | stored login | Bearer token; an explicit flag wins |
| `--json` | off | Request structured output where the command supports it |
| `--field <PATH>` | none | Select a JSON field with dotted keys and array indexes; repeatable, implies JSON output and defaults to bare values |
| `--output <json\|table\|value>` | `value` with fields, otherwise `json` | Render JSON responses as pretty JSON, aligned tables, or bare values; implies JSON output |
| `--help-json` | off | Print the whole visible command tree, aliases, descriptions, flags, and environment variable names as JSON, then exit without authentication |
| `NOLGIA_TOKEN_STORE` | file with one-time migration | `file` disables keyring access; `keyring` opts into the OS keyring |
| `--org` / `NOLGIA_ORG` | active organization | For `org members`, `org invite`, `org credits`: address another organization you belong to (slug or UUID) without switching the server-side context |
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

### Submit and subscribe (Rust)

`cargo add nolgia-client` is the whole install: the crate re-exports the async
runtime and `serde_json` and downloads finished assets itself, so this compiles
and runs end to end on a fresh project with no second dependency. `client()`
reads `NOLGIA_TOKEN` (and `NOLGIA_API_URL` when set).

```rust
use nolgia_client::{ClientExt, tokio};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nolgia = nolgia_client::client()?;
    let result = nolgia_client::subscribe(
        &nolgia,
        "/generate/image",
        nolgia_client::json!({"model": "flux-pro", "prompt": "a paper-cut mountain range"}),
        Default::default(),
    )
    .await?;
    nolgia.download(&result.url.unwrap_or_default(), "first.png").await?;
    Ok(())
}
```

`use nolgia_client::tokio;` is what makes `#[tokio::main]` resolve — the
attribute expands to a bare `tokio`, so a re-export satisfies it only once it
is in scope. `#[nolgia_client::rt::main(crate = "nolgia_client::tokio")]` is
the equivalent with no `use`, and `nolgia_client::rt::block_on(future)` runs
one future from a synchronous `fn main`. `ClientBuilder` remains available for
a hand-built client.

`ClientExt::download(url, path)` streams a finished asset to disk through a
sibling `<path>.part`, so an interrupted download leaves no truncated file;
`download_bytes(url)` returns it in memory instead. The client's bearer token
is sent only when the URL is on the same origin as the client's base URL — an
`asset.signed_url` carries its own credential and is fetched anonymously — and
a query string never reaches an error message.

Every generate request requires `model`; `flux-pro` is the CLI's default image
model. `result.media` contains all assets (deduplicated in server order), while
`result.url` is the first signed URL, or `None` when there are no assets. Treat
signed URLs as short-lived bearer capabilities.

Use `submit` for a `JobHandle`: `job_id()` and `job()` inspect the submission,
`status().await` fetches once, and `result().await` starts polling. Clone the
handle before consuming it with `result()` if you need to call `cancel_job()`
while waiting. Options control polling (500 ms by default), the wait budget (30 minutes
from `result()`), change-only status callbacks, and submission headers such as
`Idempotency-Key`. Server error codes, including unknown codes, and raw terminal
job fields survive in `GenerationError`.

`handle.cancel_job().await` cancels the job on the server
(`POST /jobs/{id}/cancel`) and stops the wait. It returns the canceled job, raw
like `status()`, whose `cancellation` says what the model provider did and
whether the credits were refunded: a job that had not reached the provider is
refunded in full, and a started render is refunded only when the provider
stops it without billing (`settlement` can read `pending` until the provider
answers). A canceled job is never added to your library. A pending or later
`result()` on the handle or any clone then fails with `ErrorCode::Canceled`,
whose message is the server's cancellation sentence. A job that already
finished is refused with `ErrorCode::JobNotCancellable` (HTTP `409`); nothing
is changed and the wait carries on to the job's own result. Without a handle,
`client.cancel_job_with_body(job_id)` (from `ClientExt`) sends the same request
and returns the typed `Job`.

A wait timeout stops only the client waiting; the job keeps running and credits
are still spent. `JobHandle::cancel()` is deprecated for the same reason: it
stops only the local wait, and the job keeps running and is billed. Keep the job
ID to inspect the existing job instead of submitting another paid generation.

Supported endpoints are `/generate/image`, `/generate/audio`, `/generate/video`,
`/generate/3d`, and `/restore/video`. `/generate/set` returns an `OutputSet` and
uses `/sets/{id}` polling, so it is deliberately excluded from this job helper.

## License

[MIT](LICENSE)
