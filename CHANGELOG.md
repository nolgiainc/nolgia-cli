# Changelog

Release notes for the Nolgia CLI. Each `## vX.Y.Z` section becomes the body of
the matching GitHub release.

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
