# Install Nolgia as an AI coding agent

Nolgia is a generative-media platform with a `nolgia` CLI for generating images, video, and audio and managing your user's media library. Follow these steps in order; at the end you will have a working, authenticated CLI, three installed agent skill packs, and a verified local image to show your user.

## 1. Check for an existing installation

```sh
command -v nolgia
nolgia --version
```

**Check:** the first command prints a binary path and the second prints a version and exits 0. If both work, skip to step 3. Do not reinstall over a working binary. If the command is missing, check whether `$HOME/.local/bin/nolgia` or `$HOME/bin/nolgia` already exists and works with `--version`; if so, use step 2.1 to fix PATH instead of reinstalling.

## 2. Install without sudo

```sh
curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash
```

The installer downloads the latest release binary for your platform, installs it to `~/.local/bin` by default with a `~/bin` fallback, and appends an export line to your shell profile when that directory is missing from PATH. It asks for no password. **Never run the installer with `sudo`.** You can pass `--prefix <dir>` for another user-writable directory or `--tag vX.Y.Z` to pin a release, using `bash -s --` in place of `bash` before those arguments. Use `--system` only when your operator deliberately wants `/usr/local/bin` and has made it writable; it is not needed here.

**Check:** installation exits 0 and prints `installed nolgia <version> to <directory>/nolgia` (or says the requested version is already installed). In the verified clean-environment run this installed v0.2.28 into `$HOME/.local/bin` and created `$HOME/.zshrc`; the latest release can change. Do not assume a successful install has updated your running shell.

When curl is unavailable, choose one available fallback, then verify with `command -v nolgia` and `nolgia --version`:

- npm: `npm install -g @nolgia/cli` (Node 18 or newer).
- Homebrew: `brew tap nolgiainc/nolgia && brew install nolgia`.
- Cargo: `cargo install nolgia-cli` (on Linux, source builds need `pkg-config` and `libdbus-1-dev`).

### 2.1. Update PATH in your current process

The installer edits your shell profile, but your already-running shell does not re-read it. Run this in the shell you will use for the remaining commands:

```sh
export PATH="$HOME/.local/bin:$PATH"
command -v nolgia
nolgia --version
```

If the installer selected `~/bin`, use `export PATH="$HOME/bin:$PATH"` instead. For `--prefix`, prepend the exact directory you chose; for Cargo's default directory, use `export PATH="$HOME/.cargo/bin:$PATH"`. If your harness starts a fresh shell for every command, preserve this PATH in its environment or repeat the export in each shell.

**Check:** `command -v nolgia` now prints the installed path and `nolgia --version` exits 0. Do not proceed until both work.

## 3. Authenticate, ask your user, and actually wait

Check for an environment token without printing it:

```sh
if [ -n "${NOLGIA_TOKEN:-}" ]; then
  printf '%s\n' 'NOLGIA_TOKEN is set; verify it with auth status.'
else
  printf '%s\n' 'No environment token; check stored authentication.'
fi
nolgia auth status
```

**Check:** `nolgia auth status` (alias `nolgia auth whoami`) exits 0 and names the connected email. If it does, keep that authentication and proceed to step 4. If your user has supplied a token, set `NOLGIA_TOKEN` securely in your command environment and repeat this check, skipping device login. Never print the token. With no token, the verified clean-environment command exits 1 with exactly:

```text
Error: no token is stored; run 'nolgia auth login'
```

Otherwise start device login:

```sh
nolgia auth login
```

On a machine without a browser, use this instead:

```sh
nolgia auth login --no-browser
```

The command prints an approval URL such as `Open: https://nolgia.ai/device?code=XXXX-XXXX`, the short code on its own line, and `Waiting for you to approve in the browser... (expires in 15:00)`. It opens the browser when it can. **Surface the URL and code to your user verbatim and tell them to approve it at https://nolgia.ai/device. Actually wait for their approval.** The code expires in 15 minutes.

**Let the command run to completion rather than backgrounding it, killing it, or polling around it.** Waiting is this step. If your harness cannot hold a long-running foreground command, tell your user to run `nolgia auth login` themselves in their own terminal, then wait for them to confirm completion before continuing. If the code expires, start login again and surface the new URL and code.

**Check:** approval prints `✅ Connected as <email>` and login exits 0. Then run:

```sh
nolgia auth status
```

Continue only after this exits 0 and names the connected email. Do not proceed on an unauthenticated CLI.

For a non-interactive or CI agent, have an already-authenticated operator create a personal access token in a private terminal (replace the example name as needed):

```sh
nolgia pat create --name coding-agent
export NOLGIA_TOKEN=nol_...
nolgia auth status
```

Replace `nol_...` with the actual token using a secret manager or a private shell with command logging disabled; never execute the placeholder literally. The token is shown once. Do not echo it into a log, README, or commit, and do not run token creation in a harness that records its output. Creating a PAT requires existing authentication; it does not bypass device approval. **Check:** `nolgia auth status` with the injected token exits 0 and names the user.

## 4. Install and verify the skills

```sh
nolgia skills install --force
nolgia skills list
```

The default target is `claude-user`, which writes `~/.claude/skills/<name>/SKILL.md`. The three bundled packs are `nolgia-platform`, `nolgia-video-prompting`, and `nolgia-ugc-ads`. These commands are local; `nolgia skills list` works even without a token.

**Use `--force`, and do not treat that as a destructive choice.** Without it, `nolgia skills install` stops at the *first* pack that already exists, exits 1 with `... SKILL.md already exists — pass --force to overwrite`, and installs none of the remaining packs — so a machine holding one stale pack can never be brought up to date by the plain command, and a re-run of this runbook fails at this step. The packs are content shipped inside the binary, not your user's data, so overwriting them is how you refresh them to the version you just installed. The one thing `--force` does destroy is a hand-edited `SKILL.md` at those three paths: if your user has customised one, copy it aside first and ask them.

**Check:** installation exits 0 and prints `3 skill(s) installed. Agents pick them up on their next session.` The list contains all three names. Confirm their files exist:

```sh
for pack in nolgia-platform nolgia-video-prompting nolgia-ugc-ads; do
  test -s "$HOME/.claude/skills/$pack/SKILL.md" || exit 1
done
```

For another harness, select the appropriate alternative instead of the default install:

```sh
nolgia skills install --target claude-project
nolgia skills install --target hermes
nolgia skills install --target dir --dir ./agent-skills
```

These write to `./.claude/skills`, `${HERMES_HOME:-/opt/data}/skills`, or the directory supplied by `--target dir --dir <path>`, respectively. Add `--force` to each for the same reason as above, then verify the same three nonempty `SKILL.md` files beneath your chosen directory and run `nolgia skills list`.

A Claude Code session caches skills at session start. Use `/reload-skills` or start a new session before expecting freshly installed skills to be visible.

## 5. Generate one verification image and show it

First check your user's balance:

```sh
nolgia billing credits
```

**Check:** it exits 0 and prints `subscription: N (resets with plan)  api top-ups: N` and `total: N`. If the applicable balance is insufficient, tell your user and wait for credits before submitting. Personal-space PATs spend API top-ups, so a positive subscription balance alone is not enough for a PAT.

**Tell your user before running the next command: “I will generate one verification image; this spends a small number of your Nolgia credits.”** The CLI supplies its default image model, `flux-pro`:

```sh
nolgia gen image --prompt "a paper-cut mountain range at dawn" --out nolgia-verify.png
```

Keep the command's stdout out of CI logs. On success it prints the signed asset URL, a short-lived bearer capability. Show the local file to your user; do not paste the URL into a commit, issue, or CI log.

The command first prints `submitted job <uuid> — waiting up to 300s (Ctrl-C is safe: it does not cancel the job)`. Keep the job ID for recovery. **Check:** exit 0 means the asset was downloaded. The verified clean-environment run took 6.8 seconds and produced a 512 × 512 PNG; that is an observation, not a timing guarantee.

Two exit codes are not plain failures:

- **75:** the job is still running or needs to be followed. Never re-submit it: re-submitting can bill a second job. Set `JOB_ID` to the exact ID the CLI printed, then keep waiting with `nolgia wait "$JOB_ID" --timeout 300` (that is `nolgia wait <JOB_ID> --timeout 300`). Repeat the wait if it expires again. This command has no `--out` flag. After success, obtain `asset.id` from `nolgia --json status "$JOB_ID"` privately, set `ASSET_ID` to that ID, and download with `nolgia assets get "$ASSET_ID" --out nolgia-verify.png`. Job and asset JSON can contain signed URLs, so keep those outputs out of logs too.
- **65:** the content filter refused the prompt. Change the prompt; the job is not broken. A changed prompt starts a new generation and spends credits, so tell your user before retrying.

An identical prompt re-run within five minutes is refused with **409**, naming the existing job, and is not billed twice. That is the duplicate guard working: follow the named job instead of re-running. Pass a fresh `--idempotency-key` only when you genuinely want a second take and its additional charge.

Now verify the downloaded file:

```sh
ls -l nolgia-verify.png
file nolgia-verify.png
test "$(od -An -tx1 -N8 nolgia-verify.png | tr -d ' \n')" = '89504e470d0a1a0a'
```

**Check:** `ls` shows a nonempty file, `file` reports `PNG image data`, and the magic-byte test exits 0. If `file` is unavailable, the magic-byte check still verifies the PNG signature. Use your harness's local image viewer or attachment tool to open and show `nolgia-verify.png` to your user; if it has neither, give your user the local file path to open. Do not substitute the signed URL for the local file.

## 6. When something goes wrong

| Symptom | What you do and how you check it |
| --- | --- |
| Binary is not on PATH | Run `export PATH="$HOME/.local/bin:$PATH"` (or prepend the actual install directory), then `command -v nolgia` and `nolgia --version`; both must work. |
| 401 / not authenticated | Return to step 3. Replace an invalid `NOLGIA_TOKEN` securely or unset it before device login so it cannot override the stored login. Wait for approval, then require `nolgia auth status` to name a user. |
| Out of credits | Run `nolgia billing credits`, tell your user which pool needs funding, and wait for them to resolve it. Re-check the balance before a new submission. |
| Long-poll expires, exit 75 | Run `nolgia wait "$JOB_ID" --timeout 300` for the named job until terminal; never re-submit. Download the existing asset as described in step 5. |
| Moderated prompt, exit 65 | Explain the filter refusal and change the prompt before a new generation; verify the replacement's result and PNG file. |
| Duplicate guard, HTTP 409 | Follow the existing job named in the response. A fresh `--idempotency-key` is only for a deliberate, separately billed second take. |
| `skills install` exits 1 with `already exists` | The packs are already on disk and the command stopped at the first one, installing none of the rest. Re-run `nolgia skills install --force`; check for `3 skill(s) installed`. |
| No prebuilt binary for your platform | Run `cargo install nolgia-cli`, add `$HOME/.cargo/bin` to PATH if needed, then verify `command -v nolgia` and `nolgia --version`. |

You are done when:

- [ ] `nolgia auth status` exits 0 and names a user.
- [ ] `nolgia-verify.png` exists, passes the PNG check, and you have shown it to your user.
