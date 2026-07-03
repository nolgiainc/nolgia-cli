# Changelog

Release notes for the Nolgia CLI. Each `## vX.Y.Z` section becomes the body of
the matching GitHub release.

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

First public release — available via Homebrew (`brew tap nolgiacorp/nolgia`),
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
