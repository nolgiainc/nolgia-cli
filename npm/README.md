# @nolgia/cli

The npm wrapper for the [Nolgia](https://nolgia.ai) command-line client. It downloads a platform binary during postinstall and exposes it as `nolgia`; the small Node launcher forwards each invocation to that Rust binary.

## Install

```bash
npm install -g @nolgia/cli
```

The package requires Node 18 or newer and supports macOS universal, Linux x86_64, and Windows x86_64. Postinstall downloads the release asset matching the package version from [GitHub Releases](https://github.com/nolgiainc/nolgia-cli/releases), and the Node launcher forwards each command to that binary. If a package version has no matching release asset for your platform, install from [Cargo](https://github.com/nolgiainc/nolgia-cli#cargo-or-a-prebuilt-binary), the shell installer, Homebrew, or a compatible prebuilt release instead. Review registry and release provenance according to your environment's supply-chain policy before enabling postinstall downloads.

> **Release boundary.** This README is published with the npm package. The repository `main` branch can contain unreleased source changes, while `npm install` runs the tagged binary selected by the package version. Check `nolgia --version` and `nolgia --help`; do not assume a source-only command or flag is present in an older npm install.

## Quick start

```bash
nolgia auth login
nolgia models list
nolgia gen image --prompt "a futuristic city at sunset" --out city.png
nolgia gen video --model <VIDEO_MODEL_ID> --prompt "a slow dolly through a neon atelier" --out clip.mp4
nolgia gen audio --model <AUDIO_MODEL_ID> --prompt "rain on a window" --out rain.mp3
```

The live catalog is the source of truth for model IDs, capabilities, durations, and credit pricing:

```bash
nolgia models get <MODEL_ID>
nolgia gen video --model <VIDEO_MODEL_ID> --prompt "..." --cost-only
```

## What the binary can do

- `gen image`, `gen video`, and `gen audio` submit jobs and can download a completed asset with `--out`.
- Video supports model-dependent image-to-video (`--input <IMAGE_FILE|IMAGE_ASSET_UUID>`), repeated `--shot "SECONDS:PROMPT|AUDIO DIRECTION"` segments, `--generate-audio`, and live `--cost-only` estimates. Do not treat `--input` as an arbitrary asset reference: the selected model must accept image input.
- `assets`, `characters`, and `projects` organize generated media. `assets upload` accepts PNG, JPEG, or WebP files. Newer source/release versions may add more asset verbs; confirm with `nolgia assets --help`.
- `models list|get` exposes the current catalog. `billing credits`, `billing subscription`, and `billing portal` expose account billing operations. `account me|usage` reports identity and visible job/asset counts.
- `pat create|list|revoke` manages personal access tokens. `completion <SHELL>` emits shell completion code.

For the full version-specific command index, run `nolgia --help`; for flags, run `nolgia <COMMAND> --help`. Marketplace `ability` commands, asset-frame extraction, and newer quality/reference options are source/release-version dependent and should be confirmed with that help output. Replace `<PLACEHOLDER>` values before running examples.

## Authentication and output

Interactive login uses a browser device-code flow:

```bash
nolgia auth login
nolgia auth status
nolgia auth token
```

For CI or agents, prefer a personal access token in `NOLGIA_TOKEN` or a secret manager. `--token` is also accepted, but command-line arguments can appear in shell history and process listings. The npm package is release-versioned: the `0.2.6` package metadata in this checkout uses its release's keyring-based login behavior; the file-backed `NOLGIA_TOKEN_STORE` and XDG token path described in the source/main README apply only to a release that contains that newer auth implementation. Confirm the installed binary's behavior with `nolgia --version` and its help/release notes.

`--json` is a global flag for commands that implement structured output; it is not a universal output contract. Generation with `--no-wait` prints a JSON job object, while `gen video --cost-only`, `auth token`, `completion`, and `skills show` remain text. `auth login` and `auth status`/`whoami` also print human text around any JSON response. A script can capture a job UUID and then wait for it:

```bash
job_uuid=$(nolgia gen video --prompt "..." --no-wait | jq -r .job_id)
nolgia wait "$job_uuid" --json | jq .asset.signed_url
```

Treat any signed URL returned by the CLI as a temporary bearer capability; avoid writing it to persistent CI logs or telemetry.

## Environment

| Variable | Effect |
|---|---|
| `NOLGIA_TOKEN` | PAT or JWT used when `--token` is not supplied |
| `NOLGIA_API_URL` | Override the API base URL (the client appends `/v1`) |
| `NOLGIA_TOKEN_STORE` | Supported only by releases containing the file-token implementation; older npm binaries use their release's auth store |
| `NOLGIA_NO_UPDATE_CHECK` | Disable the once-per-day release hint |
| `NOLGIA_SURFACE` | Override the `X-Nolgia-Surface` request value |
| `XDG_CONFIG_HOME` | Parent directory for install metadata; token location is release-version dependent |
| `XDG_STATE_HOME` | Parent directory for the update-check cache |
| `HERMES_HOME` | Parent of the Hermes `skills` directory when supported by the installed binary |

The update hint is cached locally, refreshed at most once per day, and suppressed for JSON, CI, non-interactive, and any non-empty `NOLGIA_SURFACE`. Use `NOLGIA_NO_UPDATE_CHECK=1` for a quiet script.

## Other install paths and development

The root [repository README](https://github.com/nolgiainc/nolgia-cli#installation) covers Homebrew, Cargo, the shell installer, prebuilt binaries, command semantics, and the development checks. The client API shapes are generated from the vendored [OpenAPI snapshot](https://github.com/nolgiainc/nolgia-cli/blob/main/crates/client/openapi.yaml); source builds use it by default.

## License

MIT
