# PROJECT KNOWLEDGE BASE

## OVERVIEW
Rust 2024 workspace for the `nolgia` developer CLI and its generated API client wrapper.

## STRUCTURE
```
nolgia-cli/
|-- Cargo.toml              # workspace members and shared deps
|-- crates/cli/             # user-facing `nolgia` binary
|   |-- src/main.rs         # clap parser and dispatch
|   |-- src/commands/       # command handlers
|   |-- src/auth.rs         # auth/device-code/token behavior
|   `-- tests/              # CLI integration and e2e tests
`-- crates/client/          # generated API client crate + wrapper
```

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Binary entry | `crates/cli/src/main.rs` | Global flags and subcommand dispatch |
| Commands | `crates/cli/src/commands` | `auth`, `gen`, `status`, `wait`, `assets`, `account`, `billing` |
| Output rules | `crates/cli/src/output.rs` | Human vs JSON formatting |
| Live-job reporting | `crates/cli/src/livejob.rs` | A submitted job must never be lost: wait timeout (408), Ctrl-C, post-submit failure and duplicate refusal (409) all render as "a job is live" and exit 75 |
| Auth flow | `crates/cli/src/auth.rs` | Device-code OAuth and local credentials |
| Public command tests | `crates/cli/tests/cli_commands.rs` | Help, flags, command behavior |
| E2E CLI tests | `crates/cli/tests/e2e_cli.rs` | Gated by `--features e2e` |
| API client wrapper | `crates/client/src/lib.rs` | Re-exported generated client plus `ClientBuilder` |
| Client codegen | `crates/client/build.rs` | Reads local spec in debug, release asset in release |

## CROSS-REPO FLOW
- `nolgia-api/api/openapi.yaml` defines the API contract that generates this repo's client; do not design CLI API shapes locally.
- `nolgia.com` is the parallel user-facing surface; mirror feature availability, status wording, pricing assumptions, and auth expectations when relevant.
- `infra` deploys the API/MCP services the CLI talks to; new base URLs, env names, auth assumptions, or service modes may need infra changes.
- `litellm` provider/model support flows through API generation behavior before it reaches CLI commands.
- Root `AGENTS.md` tracks platform-level order; top-level repos are separate git repos and must be checked independently.

## FEATURE UPDATE CHECKLIST
- For API changes, update `nolgia-api/api/openapi.yaml` first, regenerate the Rust client, then update `ClientBuilder` or command code as needed.
- For user-facing feature parity, compare the matching `nolgia.com` route/component and keep CLI flags, JSON fields, and human output aligned.
- For auth/account/billing changes, verify API handlers plus web flows before changing CLI behavior in isolation.
- For runtime config changes, confirm `infra` exposes the required API URL, secrets, IAM, or service before adding CLI assumptions.
- For provider/model changes, coordinate with `litellm`, API routing/costs, and web display so CLI status/output does not advertise unsupported behavior.

## CONVENTIONS
- Binary name is `nolgia`, even though the crate is `nolgia-cli`.
- Shared dependency versions live in workspace root `Cargo.toml`.
- Pass API client/output format through `CommandContext`; avoid per-command global state.
- Global flags include `--json`, `--api-url`, and `--token`; env support comes from clap/env config.
- Generated client behavior comes from the OpenAPI spec and `build.rs`; do not treat it as hand-written API logic.

## COMMANDS
```bash
cargo run -p nolgia-cli --bin nolgia -- --help
cargo test --workspace
cargo test -p nolgia-cli
cargo test --features e2e --test e2e_cli -p nolgia-cli -- --include-ignored
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo build --release
```

## ANTI-PATTERNS
- Do not report a job the server accepted as a bare error. Any ending after a
  successful submit must name the job id and route through
  `livejob::LiveJob`/`EXIT_LIVE_JOB` — a message that reads like a failure is
  what makes a human re-run and pay twice (NOL-344, NOL-356).
- Do not rename the binary to match the crate; the public command is `nolgia`.
- Do not edit generated client internals when an OpenAPI or build-pipeline change is required.
- Do not run e2e tests accidentally; they are feature-gated and may depend on real service behavior.
