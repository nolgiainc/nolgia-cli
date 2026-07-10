# Changelog

Release notes for the Nolgia CLI. Each `## vX.Y.Z` section becomes the body of
the matching GitHub release.

## Unreleased

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
