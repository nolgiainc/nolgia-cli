#!/usr/bin/env bash
# Hermetic smoke tests for install.sh: runs in a throwaway HOME, installs a
# fake binary from a local path (NOLGIA_INSTALL_SOURCE), and never touches
# the network, the real HOME, sudo, or any keychain.
#
#   bash tests/install_sh_test.sh

set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
installer="$repo_root/install.sh"

failures=0
check() {
  local label="$1"
  shift
  if "$@"; then
    echo "ok: $label"
  else
    echo "FAIL: $label" >&2
    failures=$((failures + 1))
  fi
}

sandbox=$(mktemp -d)
trap 'rm -rf "$sandbox"' EXIT

# A stand-in release binary that only answers --version.
cat > "$sandbox/fake-nolgia" <<'FAKE'
#!/usr/bin/env bash
echo "nolgia 9.9.9"
FAKE
chmod +x "$sandbox/fake-nolgia"

# Every remote call uses this stub, including unexpected checksum requests.
mkdir -p "$sandbox/bin"
cat > "$sandbox/bin/curl" <<'CURL'
#!/usr/bin/env bash
set -euo pipefail
url=""
destination=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) destination="$2"; shift 2 ;;
    https://*) url="$1"; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  */releases/latest)
    source_file="${TEST_RELEASE_JSON:-}"
    [ -z "${TEST_CURL_LOG:-}" ] || printf '%s\n' "$destination" >> "$TEST_CURL_LOG"
    ;;
  */SHA256SUMS) source_file="${TEST_REMOTE_SUMS:-}" ;;
  */nolgia-*) source_file="${TEST_REMOTE_BINARY:-}" ;;
  *) exit 99 ;;
esac
[ -n "$source_file" ] || exit 22
if [ -n "$destination" ]; then
  cp "$source_file" "$destination"
else
  cat "$source_file"
fi
CURL
chmod +x "$sandbox/bin/curl"
test_path="$sandbox/bin:/usr/bin:/bin"

check_export() {
  check "$1" test "${2##*$'\n'}" = "export PATH=\"$3:\$PATH\""
}

# env -i: a leaked ZDOTDIR/XDG_* from the developer's real environment must
# never let the installer-under-test write outside the sandbox.
run_installer() {
  env -i \
    HOME="$sandbox/home" \
    SHELL=/bin/zsh \
    PATH="$test_path" \
    NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
    bash "$installer" --tag v9.9.9 "$@"
}

mkdir -p "$sandbox/home"

# 1. Fresh install: lands in ~/.local/bin without sudo and wires PATH.
out=$(run_installer)
check "installs to ~/.local/bin" test -x "$sandbox/home/.local/bin/nolgia"
check "reports the install" grep -q "installed nolgia 9.9.9 to $sandbox/home/.local/bin/nolgia" <<<"$out"
check "appends PATH export to .zshrc" grep -qF 'export PATH="'"$sandbox"'/home/.local/bin:$PATH"' "$sandbox/home/.zshrc"
check "records install metadata" test -f "$sandbox/home/.config/nolgia/install-metadata.json"
check "metadata points at the prefix" grep -q '.local/bin' "$sandbox/home/.config/nolgia/install-metadata.json"
check "local binary without sums skips verification" grep -qF 'skipping checksum verification' <<<"$out"
check_export "fresh install ends with export" "$out" "$sandbox/home/.local/bin"

# 2. Idempotent re-run: same version means no-op and no duplicate PATH line.
out=$(run_installer)
check "re-run is a no-op" grep -q "already installed" <<<"$out"
check "PATH line not duplicated" test "$(grep -cF '# Added by the Nolgia CLI installer' "$sandbox/home/.zshrc")" = 1
check_export "no-op ends with export" "$out" "$sandbox/home/.local/bin"
check "keeps already-exported profile note" grep -qF "is already exported in" <<<"$out"

# 3. Explicit --prefix is honored.
out=$(run_installer --prefix "$sandbox/home/custom-bin")
check "installs to --prefix" test -x "$sandbox/home/custom-bin/nolgia"
check_export "custom prefix ends with export" "$out" "$sandbox/home/custom-bin"

for state in fresh noop; do
  prefix="$sandbox/home/on-path"
  out=$(env -i HOME="$sandbox/home" SHELL=/bin/zsh PATH="$prefix:$test_path" \
    NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
    bash "$installer" --tag v9.9.9 --prefix "$prefix")
  check_export "prefix already on PATH ($state) ends with export" "$out" "$prefix"
done

# 4. An unwritable target fails fast with guidance — it never password-prompts.
mkdir -p "$sandbox/readonly"
chmod 500 "$sandbox/readonly"
if out=$(env -i HOME="$sandbox/home" PATH="$test_path" \
  NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
  bash "$installer" --tag v9.9.9 --system --prefix "$sandbox/readonly/bin" 2>&1 < /dev/null); then
  echo "FAIL: unwritable install should fail" >&2
  failures=$((failures + 1))
else
  check "unwritable install explains itself" grep -q "not writable" <<<"$out"
fi
chmod 700 "$sandbox/readonly"

# 5. Resolve assets independently of the host platform, without downloading.
for arch in aarch64 arm64 x86_64; do
  case "$arch" in
    aarch64 | arm64) asset="nolgia-aarch64-unknown-linux-gnu" ;;
    x86_64) asset="nolgia-x86_64-unknown-linux-gnu" ;;
  esac
  prefix="$sandbox/home/linux-$arch"
  if out=$(env -i HOME="$sandbox/home" SHELL=/bin/zsh PATH="$test_path" \
    NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
    NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH="$arch" \
    bash "$installer" --tag v9.9.9 --prefix "$prefix" 2>&1); then
    check "Linux/$arch installs" test -x "$prefix/nolgia"
    check "Linux/$arch selects its asset" grep -qF "installing nolgia v9.9.9 ($asset) from $sandbox/fake-nolgia" <<<"$out"
  else
    check "Linux/$arch installs" false
  fi
done

if out=$(env -i HOME="$sandbox/home" PATH="$test_path" \
  NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
  NOLGIA_INSTALL_OS=MINGW64_NT-10.0 NOLGIA_INSTALL_ARCH=aarch64 \
  bash "$installer" --tag v9.9.9 --prefix "$sandbox/home/windows-arm64" 2>&1); then
  check "Windows arm64 refuses a Unix install" false
else
  check "Windows arm64 names its asset" grep -qF "nolgia-aarch64-pc-windows-msvc.exe" <<<"$out"
fi

if out=$(env -i HOME="$sandbox/home" PATH="$test_path" \
  NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
  NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH=riscv64 \
  bash "$installer" --tag v9.9.9 --prefix "$sandbox/home/linux-riscv64" 2>&1); then
  check "Linux/riscv64 refuses unsupported architecture" false
else
  check "Linux/riscv64 suggests cargo" grep -qF "no prebuilt binary for Linux/riscv64 yet; install with: cargo install nolgia-cli" <<<"$out"
fi

# A missing old-release asset is simulated by a curl stub, never the network.
if out=$(env -i HOME="$sandbox/home" PATH="$test_path" \
  NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH=aarch64 \
  bash "$installer" --tag v9.9.9 --prefix "$sandbox/home/download-failure" 2>&1); then
  check "missing release asset fails" false
else
  check "download failure names asset and tag" grep -qF "could not download nolgia-aarch64-unknown-linux-gnu for nolgia v9.9.9" <<<"$out"
  check "download failure suggests cargo" grep -qF "install with: cargo install nolgia-cli" <<<"$out"
  check "download failure suggests newer release" grep -qF -- "--tag" <<<"$out"
fi

# 6. Large latest-release JSON must be downloaded before parsing its tag.
awk 'BEGIN {
  print "{\n  \"tag_name\": \"v9.9.9\",\n  \"body\": ["
  for (i = 0; i < 8192; i++) print "\"release notes padded to exercise an early pipe close\","
  print "\"end\"]}"
}' > "$sandbox/release.json"
check "latest-release fixture is at least 256 KiB" test "$(wc -c < "$sandbox/release.json")" -ge 262144
prefix="$sandbox/home/latest"
if out=$(env -i HOME="$sandbox/home" SHELL=/bin/zsh PATH="$test_path" \
  NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" TEST_RELEASE_JSON="$sandbox/release.json" \
  TEST_CURL_LOG="$sandbox/latest-curl.log" \
  bash "$installer" --prefix "$prefix" 2>&1); then
  check "large latest response installs" test -x "$prefix/nolgia"
  check "large latest response selects tag" grep -qF 'installing nolgia v9.9.9' <<<"$out"
  check "latest JSON is downloaded to a file" grep -q '/release.json$' "$sandbox/latest-curl.log"
  check_export "latest install ends with export" "$out" "$prefix"
else
  check "large latest response installs" false
fi

printf '{}\n' > "$sandbox/no-tag.json"
for response in no-tag missing; do
  rc=0
  out=$(env -i HOME="$sandbox/home" PATH="$test_path" \
    TEST_RELEASE_JSON="$sandbox/$response.json" \
    bash "$installer" --prefix "$sandbox/home/unresolved-$response" 2>&1) || rc=$?
  check "unresolved tag ($response) exits 1" test "$rc" = 1
  check "unresolved tag ($response) keeps guidance" grep -qFx \
    'could not resolve the latest release tag; pass one with --tag vX.Y.Z' <<<"$out"
done

asset="nolgia-x86_64-unknown-linux-gnu"
if command -v sha256sum >/dev/null 2>&1; then
  digest=$(sha256sum "$sandbox/fake-nolgia" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  digest=$(shasum -a 256 "$sandbox/fake-nolgia" | awk '{print $1}')
else
  digest=$(openssl dgst -sha256 "$sandbox/fake-nolgia" | awk '{print $NF}')
fi
wrong=0000000000000000000000000000000000000000000000000000000000000000
printf '%s  %s\n' "$wrong" "$asset.extra" "$digest" "$asset" > "$sandbox/match.sums"
printf '%s  %s\n' "$wrong" "$asset" > "$sandbox/mismatch.sums"
printf '%s  %s\n' "$digest" "$asset.extra" > "$sandbox/missing-entry.sums"

checksum_install() {
  local prefix="$1"
  shift
  env -i HOME="$sandbox/home" SHELL=/bin/zsh PATH="$test_path" \
    NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH=x86_64 \
    NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" "$@" \
    bash "$installer" --tag v9.9.9 --prefix "$prefix"
}

out=$(checksum_install "$sandbox/home/checksum-match" NOLGIA_INSTALL_SUMS="$sandbox/match.sums")
check "matching checksum reports digest and tool" grep -qE \
  "^verified $asset sha256 ${digest:0:12}… with (sha256sum|shasum|openssl)$" <<<"$out"
check "matching checksum installs" test -x "$sandbox/home/checksum-match/nolgia"

for scenario in mismatch missing-entry; do
  prefix="$sandbox/home/$scenario"
  mkdir -p "$sandbox/tmp-$scenario"
  rc=0
  out=$(checksum_install "$prefix" NOLGIA_INSTALL_SUMS="$sandbox/$scenario.sums" \
    TMPDIR="$sandbox/tmp-$scenario" 2> "$sandbox/checksum-error") || rc=$?
  check "$scenario exits 1" test "$rc" = 1
  check "$scenario leaves no installed binary" test ! -e "$prefix/nolgia"
  check "$scenario cleans temporary downloads" test -z "$(ls -A "$sandbox/tmp-$scenario")"
  if [ "$scenario" = mismatch ]; then
    message="error: checksum mismatch for $asset (expected $wrong, got $digest); refusing to install"
  else
    message="error: SHA256SUMS for nolgia v9.9.9 has no entry for $asset"
  fi
  check "$scenario reports exact error on stderr" grep -qFx "$message" "$sandbox/checksum-error"
done

for scenario in local remote; do
  prefix="$sandbox/home/no-sums-$scenario"
  if [ "$scenario" = local ]; then
    out=$(checksum_install "$prefix" NOLGIA_INSTALL_SUMS="$sandbox/nonexistent.sums")
  else
    out=$(checksum_install "$prefix" NOLGIA_INSTALL_SOURCE= TEST_REMOTE_BINARY="$sandbox/fake-nolgia")
  fi
  check "missing $scenario sums installs" test -x "$prefix/nolgia"
  check "missing $scenario sums reports skip" grep -qFx \
    'note: no SHA256SUMS published for nolgia v9.9.9; skipping checksum verification' <<<"$out"
  check_export "missing $scenario sums ends with export" "$out" "$prefix"
done
out=$(checksum_install "$sandbox/home/remote-sums" NOLGIA_INSTALL_SOURCE= \
  TEST_REMOTE_BINARY="$sandbox/fake-nolgia" TEST_REMOTE_SUMS="$sandbox/match.sums")
check "remote sums verify downloaded asset" grep -qF "verified $asset sha256" <<<"$out"

mkdir -p "$sandbox/minimal"
for program in bash awk grep sed mktemp chmod mv cp mkdir date basename rm; do
  ln -s "$(command -v "$program")" "$sandbox/minimal/$program"
done
cp "$sandbox/bin/curl" "$sandbox/minimal/curl"
cat > "$sandbox/digest-tool" <<'DIGEST'
#!/usr/bin/env bash
set -euo pipefail
case "${0##*/}" in
  sha256sum) test "$#" = 1; test -s "$1"; printf '%s  %s\n' "$TEST_DIGEST" "$1" ;;
  shasum) test "$1" = -a; test "$2" = 256; test -s "$3"; printf '%s  %s\n' "$TEST_DIGEST" "$3" ;;
  openssl) test "$1" = dgst; test "$2" = -sha256; test -s "$3"; printf 'SHA2-256(%s)= %s\n' "$3" "$TEST_DIGEST" ;;
esac
DIGEST
chmod +x "$sandbox/digest-tool"
for tool in sha256sum shasum openssl; do
  cp "$sandbox/digest-tool" "$sandbox/minimal/$tool"
done
for tool in sha256sum shasum openssl none; do
  prefix="$sandbox/home/tool-$tool"
  out=$(checksum_install "$prefix" PATH="$sandbox/minimal" \
    NOLGIA_INSTALL_SUMS="$sandbox/match.sums" TEST_DIGEST="$digest" 2> "$sandbox/tool-error")
  check "$tool fallback installs" test -x "$prefix/nolgia"
  if [ "$tool" = none ]; then
    check "no digest tool warns" grep -qFx \
      'warning: cannot verify the download (no sha256sum, shasum or openssl on this machine)' "$sandbox/tool-error"
  else
    check "$tool priority and normalization" grep -qFx \
      "verified $asset sha256 ${digest:0:12}… with $tool" <<<"$out"
    rm "$sandbox/minimal/$tool"
  fi
  check_export "$tool fallback ends with export" "$out" "$prefix"
done

rc=0
out=$(checksum_install "$sandbox/home/no-tool-missing-entry" PATH="$sandbox/minimal" \
  NOLGIA_INSTALL_SUMS="$sandbox/missing-entry.sums" 2>&1) || rc=$?
check "missing entry fails even without a digest tool" test "$rc" = 1
check "missing entry without a tool names asset" grep -qFx \
  "error: SHA256SUMS for nolgia v9.9.9 has no entry for $asset" <<<"$out"

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all install.sh tests passed"
