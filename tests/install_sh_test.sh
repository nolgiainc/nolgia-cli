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

# env -i: a leaked ZDOTDIR/XDG_* from the developer's real environment must
# never let the installer-under-test write outside the sandbox.
run_installer() {
  env -i \
    HOME="$sandbox/home" \
    SHELL=/bin/zsh \
    PATH="/usr/bin:/bin" \
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

# 2. Idempotent re-run: same version means no-op and no duplicate PATH line.
out=$(run_installer)
check "re-run is a no-op" grep -q "already installed" <<<"$out"
check "PATH line not duplicated" test "$(grep -cF '# Added by the Nolgia CLI installer' "$sandbox/home/.zshrc")" = 1

# 3. Explicit --prefix is honored.
out=$(run_installer --prefix "$sandbox/home/custom-bin")
check "installs to --prefix" test -x "$sandbox/home/custom-bin/nolgia"

# 4. An unwritable target fails fast with guidance — it never password-prompts.
mkdir -p "$sandbox/readonly"
chmod 500 "$sandbox/readonly"
if out=$(env -i HOME="$sandbox/home" PATH="/usr/bin:/bin" \
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
  if out=$(env -i HOME="$sandbox/home" SHELL=/bin/zsh PATH="/usr/bin:/bin" \
    NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
    NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH="$arch" \
    bash "$installer" --tag v9.9.9 --prefix "$prefix" 2>&1); then
    check "Linux/$arch installs" test -x "$prefix/nolgia"
    check "Linux/$arch selects its asset" grep -qF "installing nolgia v9.9.9 ($asset) from $sandbox/fake-nolgia" <<<"$out"
  else
    check "Linux/$arch installs" false
  fi
done

if out=$(env -i HOME="$sandbox/home" PATH="/usr/bin:/bin" \
  NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
  NOLGIA_INSTALL_OS=MINGW64_NT-10.0 NOLGIA_INSTALL_ARCH=aarch64 \
  bash "$installer" --tag v9.9.9 --prefix "$sandbox/home/windows-arm64" 2>&1); then
  check "Windows arm64 refuses a Unix install" false
else
  check "Windows arm64 names its asset" grep -qF "nolgia-aarch64-pc-windows-msvc.exe" <<<"$out"
fi

if out=$(env -i HOME="$sandbox/home" PATH="/usr/bin:/bin" \
  NOLGIA_INSTALL_SOURCE="$sandbox/fake-nolgia" \
  NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH=riscv64 \
  bash "$installer" --tag v9.9.9 --prefix "$sandbox/home/linux-riscv64" 2>&1); then
  check "Linux/riscv64 refuses unsupported architecture" false
else
  check "Linux/riscv64 suggests cargo" grep -qF "no prebuilt binary for Linux/riscv64 yet; install with: cargo install nolgia-cli" <<<"$out"
fi

# A missing old-release asset is simulated by a curl stub, never the network.
mkdir -p "$sandbox/bin"
cat > "$sandbox/bin/curl" <<'CURL'
#!/usr/bin/env bash
exit 22
CURL
chmod +x "$sandbox/bin/curl"
if out=$(env -i HOME="$sandbox/home" PATH="$sandbox/bin:/usr/bin:/bin" \
  NOLGIA_INSTALL_OS=Linux NOLGIA_INSTALL_ARCH=aarch64 \
  bash "$installer" --tag v9.9.9 --prefix "$sandbox/home/download-failure" 2>&1); then
  check "missing release asset fails" false
else
  check "download failure names asset and tag" grep -qF "could not download nolgia-aarch64-unknown-linux-gnu for nolgia v9.9.9" <<<"$out"
  check "download failure suggests cargo" grep -qF "install with: cargo install nolgia-cli" <<<"$out"
  check "download failure suggests newer release" grep -qF -- "--tag" <<<"$out"
fi

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all install.sh tests passed"
