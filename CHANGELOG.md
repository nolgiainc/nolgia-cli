# Changelog

Release notes for the Nolgia CLI. Each `## vX.Y.Z` section becomes the body of
the matching GitHub release.

## v0.2.20

- **New `nolgia restore video`: the AI footage-restorer lane.** Re-renders
  existing footage with de-noise, de-haze, detail recovery and up-res to a
  target resolution tier (`--quality 720p|1080p|1440p|2160p`, default 1080p on
  `seedvr2-restore`). `--input` takes a video asset UUID, a local file
  (uploaded first), or a raw https URL; URL and local-file sources need
  `--duration-seconds` (it prices the job, and a fresh upload's duration is
  probed asynchronously, so it is not known at submit time; existing assets
  bill from stored metadata). Every client-side check runs before a local file
  is uploaded, so a rejected option cannot cost an upload. `--noise-scale`
  tunes detail injection. Restore models take no prompt, and `--model` forwards
  any id verbatim (the NOL-439 rule), so future restore drivers work without a
  CLI re-release.
- **The twelve Topaz master upscalers join `nolgia restore video`.** Same
  command, same flags: `--model topaz-proteus` (general live action),
  `topaz-rhea` (texture), `topaz-iris` (faces), `topaz-nyx` (denoise plus
  detail), `topaz-theia` (fine detail), `topaz-artemis` (high-quality
  restore), `topaz-gaia` (CG and animation), `topaz-dione` (interlaced), or
  the generative `topaz-starlight-fast`, `topaz-starlight`, `topaz-wonder`,
  `topaz-hyperion`. `--quality` gains a `4320p` (8K) rung on the eight classic
  engines and on `topaz-starlight-fast`; the generative trio tops out at
  2160p. Tiers and per-tier credit rates are per-model — read them from
  `nolgia models get <model>` — and the API is the one that refuses a tier a
  model does not publish, so the CLI never rejects a combination the platform
  would have run. `--noise-scale` remains a `seedvr2-restore` control; the
  Topaz engines take the engine choice and the tier as their whole input.
- `nolgia models list`/`get` now mark restore-lane models with `restore` in
  their capability column, so the lane is visible without `--json`.
- A duplicate-submission (`409`) recovery block now names the command that
  submitted (`nolgia restore video ... --idempotency-key <new-value>`) instead
  of always suggesting `nolgia gen ...`.
- **`gen video --cost-only` now reflects the audio flag.** Video models whose
  provider charges extra for a soundtrack carry an `audio_surcharge` on
  `GET /models`; asking for a silent clip with `--generate-audio false` is
  charged `credits - audio_surcharge`. The estimate now subtracts the surcharge
  from the per-baseline rate *before* duration scaling, matching the server's
  order of operations, so a quoted silent clip matches what is actually billed.
  Audio is on by default, so default quotes are unchanged; Kling v3 is the only
  family carrying a nonzero surcharge today (720p 32 credits with audio vs 21
  silent, 1080p 42 vs 28, 4k 158 vs 110).
- Re-vendored the OpenAPI spec so the generated client carries `audio_surcharge`
  on `ModelCost` and `QualityOption`.

## v0.2.19

- **`gen video/image/audio --model` now accepts any model id and lets the API
  validate it.** The `--model` args used to validate against the generated,
  closed `{Video,Image,Audio}Model` enums, which only track the vendored
  OpenAPI spec — and the spec reaches users only through a release. So any model
  the API added after the last released binary was cut was rejected at argument
  parsing, even though the raw API accepted it (NOL-439):

  ```
  error: invalid value 'flux-3-video' for '--model <MODEL>': invalid value
  ```

  while `POST /generate/video {model: flux-3-video}` succeeded. The request-body
  model selectors are now relaxed to plain strings at codegen and the `--model`
  args are typed as `String`, so the id is forwarded verbatim and validated
  server-side. A model the server rejects still fails legibly via the API's RFC
  7807 response. This fixes the whole class: adopting a future model no longer
  needs a CLI re-release.

- Re-vendored the OpenAPI spec from nolgia-api (#85–#93).

## v0.2.18

- **A job the server accepted is never lost again — and a wait timeout no
  longer reads as a failure.** These were the two behaviours that made a human
  re-run a generation and pay for it twice (NOL-344 cost 84 credits exactly
  this way).

  Previously, if anything went wrong after a submission had already succeeded,
  the CLI printed a transport error and **no job id** — so the command looked
  like it had failed *before* submitting, and re-running was the natural
  response. And a `408` from `GET /jobs/{id}/wait`, which only means the
  server's long-poll window closed while the job kept running, surfaced as:

  ```
  Error: waiting for generation job

  Caused by:
      Unexpected Response: Response { url: "…/wait?timeout_seconds=300", status: 408, … }
  ```

  Nothing had failed, but the word `Error:` is the strongest possible prompt to
  try again.

  Four endings now deliver the same fact — *work is live under this id, follow
  it, do not re-submit*:

  ```
  still running after 300s — job 60893909-3123-42bd-b04f-ed946b136c0f
    Nothing failed. The server's long-poll window closed while the job was
    still running — the job was not cancelled and is still being worked on.
    It will be billed once, whether or not you keep waiting. Re-running this
    command would start a second job.
      nolgia wait 60893909-…    # keep waiting for it
      nolgia status 60893909-…  # check it once
  ```

  - `gen` now prints the job id on stderr the moment the server accepts it,
    before any waiting begins. This is the robust half: an error path can only
    speak if the process lives long enough to reach it, and in the incident it
    did not. The line is already in the operator's scrollback even if the CLI
    is killed outright or the pipe is torn down.
  - Ctrl-C after submission, and any other post-submission failure, report the
    job id and how to follow it instead of a bare error.
  - A duplicate submission — refused by the API with `409` since NOL-344 — is
    rendered as the job that already exists, restating the API's advice as
    commands a shell can actually run (the server says "check it with
    `GET /jobs/{id}`", which is true and unusable at a prompt).
  - `gen audio` refusals go through the RFC 7807 handler like `image` and
    `video` already did; they previously surfaced as raw `Unexpected Response`
    debug dumps. `nolgia status` likewise no longer dumps a raw response on a
    404.

  **New exit code `75`** (sysexits `EX_TEMPFAIL`) means "a job is live; do not
  re-run" and is used for all four cases above. It replaces exit `1` *only*
  there; every other failure still exits `1` with the same `Error:` text as
  before. This also makes the situation machine-readable for the first time:
  a `408` was previously indistinguishable from a genuine failure, which is
  why nolgia-agent's CLI backend could not implement the "keep polling"
  contract its SDK backend has always had. Under `--json`, stdout carries a
  document (`job_id`, `outcome`, `billed_twice`, `follow_up`) while the human
  block goes to stderr, so a program's stdout stays parseable.

- **`--idempotency-key` (also `NOLGIA_IDEMPOTENCY_KEY`).** The API's duplicate
  guard fingerprints the request body, so a *deliberate* second take of an
  identical prompt is refused too; the documented escape hatch is an
  `Idempotency-Key` header, which the CLI previously had no way to send — the
  header is accepted by the API but is not declared in the OpenAPI spec, so the
  generated client emits no parameter for it. Passing a fresh key runs an
  identical request again on purpose; reusing one collapses a client's own
  retries into a single job.

## v0.2.17

- **The model catalog no longer fails to parse because the API added a value
  this CLI has never heard of.** Generated response types treated every
  OpenAPI `enum` as a closed set, so a single unrecognised value failed the
  whole payload:

  ```
  unknown variant `3:1`, expected one of `16:9`, `9:16`, `1:1`, …
  ```

  `models list`, `models get`, `gen video --cost-only` and every capability
  precheck fetch `GET /models` before doing anything else, so one unknown
  aspect ratio took out jobs that never mentioned an aspect ratio — a startup
  failure rather than a submission failure. This is the third occurrence of
  the same shape (NOL-48, NOL-69, NOL-351); the first two were fixed by
  re-vendoring the spec, which repairs only binaries built afterwards and has
  never prevented the next one.

  Enums the client only ever *receives* are now generated as plain strings, so
  an unknown value parses and is preserved verbatim — a ratio this build
  cannot offer is still a ratio it can list. Enums the client *sends* are
  unchanged and still validated client-side, so `--aspect-ratio` still names
  every accepted value on a miss.

  Behaviour change: `nolgia models get <id>` prints `modality: video` rather
  than `modality: Video`, matching `models list` and the `--json` output. JSON
  output is otherwise unchanged — the same wire strings, at the same keys.

## v0.2.16

- **`--shot` no longer submits a contradictory `duration_seconds`, which was
  400-ing every multi-shot job.** `duration_seconds` is declared `default: 5`
  in the OpenAPI spec — a description of what the *server* does when the field
  is absent. The generated client materialized that default into a
  non-`Option` field with no `skip_serializing_if`, so every request carried
  `duration_seconds: 5` whether or not the caller asked for it, and the CLI had
  no way to express "absent" at all. Any job whose `--shot` durations summed to
  something other than 5 was rejected:

  ```
  400 duration_seconds (5) must equal the sum of shot durations (10) — or omit it
  ```

  That is every multi-shot job at the film pipeline's default 12s batch, so the
  `short-film` preset — featured on the landing rail — could not run and had
  never once completed. `--cost-only` was unaffected (it sums the shots
  locally), which is why the defect never surfaced during estimation.

  The client now omits `duration_seconds` entirely unless the caller passed
  `--duration-seconds`, letting the server derive the length: the shot sum when
  shots are given, its own 5s default when they are not. Passing
  `--duration-seconds` alongside `--shot` is still allowed when it *equals* the
  shot sum (the API accepts that, and the nolgia-agent film pipeline relies on
  it); a value that contradicts the shots is now refused client-side, naming
  both numbers, before any asset upload or API call happens (NOL-342).

- **`nolgia gen image` can now request an aspect ratio.** `gen video` has had
  `--aspect-ratio` all along; `gen image` had no way to ask for anything but
  the model's native default, and putting "vertical 9:16" in the prompt does
  not work (flux-pro returned 512x512, gpt-image-2 returned 1024x1024). The
  three vertical UGC presets therefore had to generate square and crop in
  ffmpeg, discarding ~44% of the frame, handing the composition to whoever
  wrote the crop, and — on a 512x512 source — yielding a 288x512 image that
  Kling rejects outright with `Image pixel is invalid`.

  `--aspect-ratio` maps to the API's `aspect_ratio` field, whose vocabulary is
  ratios (`9:16`, `16:9`, `1:1`, …), *not* the `image_size` aliases
  (`portrait_16_9`). The two are different knobs; the API prefers
  `aspect_ratio` and validates it per-model, while `image_size` only expresses
  the 16:9/4:3/1:1 families.

  Bad values fail fast with a useful message instead of a server 400. A value
  outside the enum lists every real ratio and points out the alias confusion; a
  ratio the selected model does not publish is caught before the request is
  sent and lists that model's actual options, taken from `image.aspect_ratios`
  on `GET /models` — the same list the API validates against. `nolgia models
  get <model>` and `models list` now show that list, which they previously
  rendered nothing of for image models (NOL-345).

## v0.2.15

- **Security: `--help` no longer prints the values of the environment
  variables it reads.** clap's default rendering for an `env`-backed argument
  shows the *resolved value* of the variable, so on any machine with
  `NOLGIA_TOKEN` exported, `nolgia --help` printed the token inline in its
  options block (`--token <TOKEN>  [env: NOLGIA_TOKEN=...]`). Help output is
  the least-guarded text in a system — it lands in terminal scrollback, CI
  logs, agent transcripts, screenshots and bug reports — so this defeated the
  rest of the CLI's secret handling. Both env-backed globals (`--token` /
  `NOLGIA_TOKEN` and `--api-url` / `NOLGIA_API_URL`) now carry
  `hide_env_values`: help still names the variable, so the flag stays
  discoverable, but never shows what it holds (NOL-317).

  Two regression guards ship with the fix. A structural test walks the entire
  clap command tree and fails if *any* env-backed argument — including ones
  added in future, anywhere in the tree — is missing `hide_env_values`. A
  black-box test renders the real binary's help with both variables set to
  sentinel values and asserts neither value appears.

  Anyone who ran `nolgia --help` with a real token exported on a released
  build up to and including v0.2.14 should treat that token as disclosed and
  rotate it.

- **Housekeeping: version metadata now matches the tag.** The v0.2.13 and
  v0.2.14 tags were cut on trees whose manifests still read 0.2.12, so their
  npm publish step failed the tag/version check and neither crates.io nor npm
  ever received them; the binaries attached to those two GitHub releases also
  self-report `0.2.12`. This release carries the correct version in every
  manifest, so it is the first publish since v0.2.12 to reach crates.io and
  npm — and it includes the vendored-spec work that was intended for v0.2.13
  and v0.2.14 (MiniMax models, model-specific image aspect ratios, color
  presets, `start_frame_required`).

## v0.2.12

- **`nolgia assets upload` now accepts video and audio, not just images.**
  Previously the command only handled `png`/`jpeg`/`webp` via the base64
  `POST /assets` path, so an agent or preset workflow could never deliver its
  final stitched master — the finished MP4 stayed pod-local while only the
  component clips reached the Library (NOL-109). Video (`mp4`/`mov`/`webm`) and
  audio (`mp3`/`wav`/`ogg`/`m4a`) now upload through the signed-upload flow
  (`POST /assets/uploads` → direct PUT to storage → `POST
  /assets/uploads/{id}/complete`), so the bytes stream straight to storage
  without the base64 JSON size limit. Images keep their existing base64 path
  unchanged; unknown extensions fail fast with the supported list.

## v0.2.11

- **Fix: `models list` / `models get` against the live API.** nolgia-api#158
  added an `image` capabilities field to `GET /models`; released CLIs rejected
  the whole catalog with `unknown field 'image'` because their vendored spec
  predated it. This release ships the current spec (including the new
  `image.aspect_ratios` capability surface).
- **Hardening: unknown response fields no longer break the CLI.** The
  generated client used to compile `additionalProperties: false` into
  `deny_unknown_fields`, so every additive API field broke released binaries.
  Codegen now strips that strictness from response deserialization — future
  additive fields are ignored instead of fatal. Covered by a regression test
  that parses a catalog payload carrying unknown fields at every level.

## v0.2.10

- **`nolgia color-presets`** — the built-in color-grade preset looks for
  Studio compositions. `list` prints the catalog (slug, name, description;
  `--json` includes the manifest version), and `cube <slug> [-o FILE]`
  downloads a preset's 33-point `.cube` LUT (stdout by default, so it pipes
  straight into grading tools). Both endpoints are public — no login needed —
  and unknown slugs surface the server's 404 detail verbatim.
- Re-vendored the OpenAPI spec with the nolgia-api color-grade contract
  (`/color-presets`, `/color-presets/{slug}/cube`, `ColorPreset*` schemas).

## v0.2.9

- **`gen image|video|audio --project-id <uuid>`** files the generated asset
  into a project directly at submit time (`nolgia projects list` for ids),
  and `assets upload` gains the same flag.

## v0.2.8

### Fixed

- **`nolgia ability install` now uses POST (was PUT).** The 0.2.7 binary sent
  a `PUT` with an empty body, which the API rejected (411/405), so ability
  install was broken. Re-vendored the OpenAPI spec so the generated client
  uses `POST` for the install endpoint.

### Added

- **Quality tiers**: `gen video` and `gen image` gain `--quality` for
  model-specific resolution tiers (e.g. `720p`/`1080p`/`4k` on Seedance 2.0
  Pro; premium tiers cost more). `gen video --cost-only` prices the selected
  tier, and unknown tiers fail fast with the model's available tiers and
  per-tier credits (premium marked) from the live catalog.
- **Reference-to-video (Seedance 2.0 Pro)**: `gen video` gains `--video-ref`
  (reference video asset id, up to 3; MP4/MOV, 480p–720p, 2–15s and 50MB
  combined), `--element` (element/reference image asset id, up to 9),
  `--bitrate standard|high`, and `--end-frame` (image asset UUID or local
  file; requires `--input`) for start+end frame pinning. Reference/quality
  flags are pre-validated against the model's published capabilities where
  cheap, and server-side capability 400s are surfaced verbatim.
- **`assets frame <id> [--at SECONDS|--last] [--out FILE]`** extracts a still
  frame from a video asset as a new image asset (omit `--at` for the last
  frame — handy as the `--input` of a follow-up clip).
- **`models list`/`get` show quality tiers and reference capabilities**
  (per-tier credits with default/premium markers, start/end frame support,
  video/element/audio reference caps, bitrate modes).
- **No more keychain password prompts**: login tokens now default to a
  `0600` file at `~/.config/nolgia/tokens.json` (like `gh`/`gcloud`)
  instead of the OS keyring. On macOS, keychain items are ACL'd to the
  exact binary that created them, so every upgrade/reinstall re-triggered
  a "nolgia wants to use your login keychain" password prompt on every
  command. Existing keyring tokens are migrated with a single one-time
  read (the keyring item is left in place). `NOLGIA_TOKEN_STORE=keyring`
  restores the old behavior; `NOLGIA_TOKEN_STORE=file` skips even the
  one-time migration read.
- **Installer never needs a password**: `install.sh` now defaults to
  `~/.local/bin` (falling back to `~/bin`) instead of preferring
  `/usr/local/bin`, appends the `export PATH=...` line to your shell
  profile when the install dir is not on `PATH` (off-PATH installs looked
  "not installed" to tooling, causing endless re-install prompts), and is
  idempotent — re-running with the requested version already installed is
  a no-op with no download. `--system` opts in to `/usr/local/bin`.
- **`projects create`/`update` gain `--auto-tag`** (repeatable, up to 10) so
  new assets carrying a matching tag are auto-added to the project. `update`
  also gains `--clear-auto-tags` to empty the set. This resyncs the vendored
  OpenAPI spec with the current nolgia-api contract, which added `auto_tags`.
- **`assets tag --clear` fix**: the regenerated client drops empty arrays on
  serialization, so `--clear` now sends `{"tags": []}` via a raw request helper
  (`ClientExt::clear_asset_tags`) to actually clear the tag set server-side.
- **Spec drift is now gated in CI.** A `spec-check` job fails the build if
  `crates/client/openapi.yaml` drifts from the canonical nolgia-api contract
  (fetched from the public docs endpoint). A `revendor-spec` workflow
  (repository_dispatch `openapi-updated` + manual + nightly) re-vendors the
  spec and opens a PR when it changes. `build.rs` no longer silently prefers a
  sibling `nolgia-api` checkout — that dev convenience is now opt-in via
  `NOLGIA_USE_SIBLING_SPEC=1`, so CI always uses the vendored spec.

## v0.2.7

- **`nolgia skill` renamed to `nolgia ability`** — the marketplace command for
  Hermes agents (`list`, `show`, `installed`, `install`, `uninstall`, `sync`,
  `init`, `pack`, `publish`) now lives under `nolgia ability`, mirroring the
  API's `/abilities` surface. The old `nolgia skill` command is **removed** with
  no alias. The generated API client targets `/abilities` and `Ability*` types.
- Ability packages use **`ability.json`** as the manifest — `ability init`
  scaffolds it, and `ability pack`/`ability publish` read and emit it. The synced
  install marker is now `.nolgia-ability.json`. (The on-disk install root stays
  `$HERMES_HOME/skills/` and per-package instructions stay in `SKILL.md`, for
  compatibility with existing agent pods.)
- Unrelated: `nolgia skills` (the bundled AI-agent SKILL.md packs) is a separate
  feature and is unchanged.

## v0.2.6

- **Skill authoring in the CLI** — `nolgia skill init <slug>` scaffolds an
  authoring directory (skill.json manifest, SKILL.md template, `payload/`
  for code), and `nolgia skill pack <dir>` validates the manifest and
  assembles the exact package layout `skill publish` consumes. Both work
  offline; the loop is init -> pack -> publish.
- Optional `python_requirements` manifest field (pip requirement strings)
  is validated by `skill pack` and passed through to the marketplace on
  publish.

## v0.2.5

- npm and crates.io publishing move to OIDC Trusted Publishing: releases
  publish tokenlessly (with npm build provenance), and the `NPM_TOKEN`
  and `CARGO_REGISTRY_TOKEN` secrets are retired.
- Repository moved to the `nolgiainc` GitHub org. Install URLs, the
  Homebrew tap (`nolgiainc/nolgia`), and the release/update endpoints now
  point at `nolgiainc`; old `nolgiacorp` URLs redirect.

## v0.2.4

- `characters` and `projects` commands, and asset tagging (`assets tag`).

## v0.2.3

- Full package documentation on the npm registry page for `@nolgia/cli`
- The crates.io publish step now skips versions that are already
  uploaded, so partial releases can be re-run safely
- First crates.io publish of the `nolgia-cli` binary crate (the name
  had a reuse cooldown during the v0.2.2 release)

## v0.2.2

- **New install paths** — `npm install -g @nolgia/cli` and a shell installer
  (`curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash`)
  alongside Homebrew, crates.io, and prebuilt binaries.
- **Daily update check** — the CLI prints a once-a-day upgrade hint matched
  to how it was installed (suppressed for `--json`, pipes, CI, agents, and
  `NOLGIA_NO_UPDATE_CHECK`).
- **Image-input capability** — `nolgia models list|get` now surface which
  video models accept a start image (`gen video --input`).

## v0.2.1

- **`nolgia assets upload <file>`** — upload a png/jpeg/webp once and get a
  reusable asset id for `gen video --input <uuid>` (no more re-uploading
  references per generation).
- **`nolgia gen audio --voice <id>`** — pick a TTS voice (discover them via
  `nolgia models get <model>`).
- The nolgia-agent film pipeline now drives the platform exclusively
  through this CLI.

## v0.2.0

The multi-shot and agents release.

- **Multi-shot video** — repeatable `--shot "SECONDS:PROMPT|AUDIO"` (up to 8)
  turns one generation into a cut sequence; the platform composes it and
  derives the clip duration. Best on Seedance v2 Pro with
  `--generate-audio true` for a native soundtrack.
- **Full video controls** — `--aspect-ratio`, `--duration-seconds`, `--seed`,
  `--negative-prompt`, `--generate-audio`; `--input` now accepts a local
  image (auto-uploaded) or the UUID of any previous asset for reusable
  character/product references.
- **Live model catalog** — `nolgia models list|get`: models, capabilities,
  and credit pricing straight from the server; new models appear without a
  CLI update.
- **Know the cost first** — `nolgia gen video ... --cost-only` prints the
  credit estimate without creating a job.
- **Agent skills** — the binary bundles SKILL.md packs that teach AI agents
  the platform: `nolgia skills list|show|install` (targets: Claude Code
  user/project, hermes, custom dir).
- **Agent-aware** — requests carry an `X-Nolgia-Surface` header
  (claude-code / codex / hermes / cli, override with `NOLGIA_SURFACE`);
  `nolgia auth token` prints the active bearer for scripts.
- **Shell completions** — `nolgia completion bash|zsh|fish|powershell`.
- CI now runs tests/clippy/fmt on every pull request.

## v0.1.1

First public release — available via Homebrew (`brew tap nolgiainc/nolgia`),
crates.io (`cargo install nolgia-cli`), and prebuilt binaries.

- **Sign in from the terminal** — `nolgia auth login` runs a browser
  device-code flow; tokens live in your system keyring and refresh
  automatically. Personal Access Tokens (`nolgia pat create`) cover scripts,
  CI, and agents.
- **Generate media** — `nolgia gen image|audio|video` with model selection,
  image-to-video via `--input`, and `--out` to save results locally.
- **Track and manage work** — `nolgia status` / `nolgia wait` for jobs;
  `nolgia assets list|get|delete` for your library.
- **Billing at a glance** — `nolgia billing subscription`, credit pool
  balances with `nolgia billing credits`, and a Stripe portal deep-link.
- **Script-friendly** — every command supports `--json`.

## v0.1.0

Initial tagged build (GitHub Releases binaries only).
