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

if [ "$failures" -gt 0 ]; then
  echo "$failures failure(s)" >&2
  exit 1
fi
echo "all install.sh tests passed"
