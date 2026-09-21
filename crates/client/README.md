# nolgia-client

Rust API client for `nolgia-api`, generated from `openapi.yaml` with Progenitor.

## What this crate does

- Re-exports the generated `Client`, `types`, `Error`, and `ResponseValue` types.
- Re-exports `tokio` and `serde_json` so one `cargo add` is the whole install.
- `client()` / `ClientBuilder::from_env()` build an authenticated client from
  `NOLGIA_TOKEN` and `NOLGIA_API_URL`.
- Provides `ClientBuilder` for convenient base URL normalization and optional auth.
- Automatically targets the `/v1` API stem unless the caller already includes it.

## Install

```bash
cargo add nolgia-client
```

That is the whole install. The crate re-exports the async runtime
(`nolgia_client::tokio`) and `serde_json` (`nolgia_client::json!`,
`nolgia_client::Value`), so the example below needs no second crate.

## Quickstart

`client()` reads `NOLGIA_TOKEN` (a PAT `nol_...` or a JWT) and, when set,
`NOLGIA_API_URL`.

```rust
use nolgia_client::tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nolgia = nolgia_client::client()?;
    let result = nolgia_client::subscribe(
        &nolgia,
        "/generate/image",
        nolgia_client::json!({"model": "flux-pro", "prompt": "a paper-cut mountain range at dawn"}),
        Default::default(),
    )
    .await?;
    println!("{}", result.url.unwrap_or_default());
    Ok(())
}
```

From a synchronous `fn main`, `nolgia_client::rt::block_on(future)` runs one
call without naming tokio at all.

## Usage

```rust
use nolgia_client::ClientBuilder;

let client = ClientBuilder::new("http://localhost:8080")
    .bearer_token("nol_test_token")
    .build()?;
```

## Spec version bumps

1. Update `../nolgia-api/api/openapi.yaml`.
2. Bump `spec_version` in `openapi-version.toml`.
3. Publish a matching `v<spec_version>` release in `nolgia-api` with `openapi.yaml` attached.
4. Rebuild this crate so `build.rs` can pull the new release asset in release builds.

## Local development

- Debug builds read the sibling spec file directly.
- Release builds fetch the release asset from GitHub unless `NOLGIA_OPENAPI_RELEASE_URL` is set.
