#!/usr/bin/env bash
# Nolgia CLI installer
#
#   curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash
#
# Installs to a user-writable directory (no sudo, no password prompts):
# ~/.local/bin by default, falling back to ~/bin. If the chosen directory is
# not on PATH, the matching export line is appended to your shell profile.
# Re-running with the requested version already installed is a no-op.
# Every successful run ends stdout with export PATH="<PREFIX>:$PATH".
# This is a documented contract: run the last line in your current shell, e.g.
#   eval "$(curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash | tail -n 1)"
#
# Options:
#   --prefix <dir>   install directory (default: ~/.local/bin, else ~/bin)
#   --system         install to /usr/local/bin instead (needs write access
#                    there — typically root; NOT the default on purpose)
#   --tag <vX.Y.Z>   release tag to install (default: latest)
#
# Test hooks: NOLGIA_INSTALL_SOURCE=<path> copies a local binary instead of
# downloading it; NOLGIA_INSTALL_SUMS=<path> copies local SHA256SUMS instead of
# downloading them. A missing sums file (or a local binary without a sums hook)
# skips verification. Used by tests/install_sh_test.sh without network access.
# Test hooks: NOLGIA_INSTALL_OS=<os> and NOLGIA_INSTALL_ARCH=<arch> override
# Bash's platform variables for hermetic platform/asset selection tests.

set -euo pipefail

REPO="nolgiainc/nolgia-cli"
PREFIX=""
TAG=""
SYSTEM=0

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --system)
      SYSTEM=1
      shift
      ;;
    --tag)
      TAG="$2"
      shift 2
      ;;
    *)
      echo "unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# uname, not bash's $OSTYPE/$HOSTTYPE: those are unset when the script is run
# by a non-bash shell, and `set -u` would then abort before we can even name
# the platform. uname is POSIX and present on every target, including a bare
# debian:bookworm-slim container.
os=${NOLGIA_INSTALL_OS:-$(uname -s)}
arch=${NOLGIA_INSTALL_ARCH:-$(uname -m)}
case "$os" in
  Darwin)
    # The darwin asset is a universal binary covering x86_64 and arm64.
    asset="nolgia-x86_64-apple-darwin"
    ;;
  Linux)
    case "$arch" in
      x86_64 | amd64)
        asset="nolgia-x86_64-unknown-linux-gnu"
        ;;
      aarch64 | arm64)
        asset="nolgia-aarch64-unknown-linux-gnu"
        ;;
      *)
        echo "no prebuilt binary for Linux/$arch yet; install with: cargo install nolgia-cli" >&2
        exit 1
        ;;
    esac
    ;;
  MINGW* | MSYS* | CYGWIN*)
    case "$arch" in
      aarch64 | arm64) asset="nolgia-aarch64-pc-windows-msvc.exe" ;;
      *) asset="nolgia-x86_64-pc-windows-msvc.exe" ;;
    esac
    echo "on Windows, download $asset from https://github.com/$REPO/releases or install with: cargo install nolgia-cli" >&2
    exit 1
    ;;
  *)
    echo "unsupported platform: $os/$arch; install with: cargo install nolgia-cli" >&2
    exit 1
    ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ -z "$TAG" ]; then
  if curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" -o "$tmp/release.json"; then
    TAG=$(awk -F'"' '/"tag_name"/ {print $4; exit}' "$tmp/release.json")
  fi
  if [ -z "$TAG" ]; then
    echo "could not resolve the latest release tag; pass one with --tag vX.Y.Z" >&2
    exit 1
  fi
fi

# Pick the install directory. The default deliberately avoids anything that
# could need elevated permissions: a password prompt in an installer is how
# users end up canceling, "installing" nowhere, and getting re-prompted by
# tooling forever.
if [ -z "$PREFIX" ]; then
  if [ "$SYSTEM" = "1" ]; then
    PREFIX="/usr/local/bin"
  elif mkdir -p "$HOME/.local/bin" 2>/dev/null && [ -w "$HOME/.local/bin" ]; then
    PREFIX="$HOME/.local/bin"
  else
    PREFIX="$HOME/bin"
  fi
fi

if ! mkdir -p "$PREFIX" 2>/dev/null || [ ! -w "$PREFIX" ]; then
  echo "error: $PREFIX is not writable." >&2
  if [ "$SYSTEM" = "1" ]; then
    echo "system-wide installs need root, e.g.:" >&2
    echo "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sudo bash -s -- --system" >&2
    echo "or drop --system to install to ~/.local/bin without a password." >&2
  else
    echo "pass --prefix <dir> to choose a writable directory." >&2
  fi
  exit 1
fi

# Wire the install dir onto PATH: append the export line to
# the user's shell profile once, and tell them how to pick it up now. A
# binary that lands off-PATH looks "not installed" to every tool that checks
# `command -v nolgia`, which re-triggers install prompts.
ensure_on_path() {
  case ":$PATH:" in
    *":$PREFIX:"*) return 0 ;;
  esac

  case "$(basename "${SHELL:-sh}")" in
    zsh) profile="${ZDOTDIR:-$HOME}/.zshrc" ;;
    bash)
      if [ "$os" = "Darwin" ]; then
        profile="$HOME/.bash_profile"
      else
        profile="$HOME/.bashrc"
      fi
      ;;
    *) profile="$HOME/.profile" ;;
  esac

  line="export PATH=\"$PREFIX:\$PATH\""
  if [ -f "$profile" ] && grep -qsF "$line" "$profile"; then
    echo "note: $PREFIX is already exported in $profile — restart your shell to pick it up"
  else
    printf '\n# Added by the Nolgia CLI installer\n%s\n' "$line" >> "$profile"
    echo "added $PREFIX to PATH in $profile"
  fi
  echo "run this to use nolgia in the current shell:"
  echo "  $line"
}

# Idempotence: if the requested version is already in the install dir, do
# nothing (checking the actual directory, not `command -v`, so a broken PATH
# can't force a pointless re-download).
if [ -x "$PREFIX/nolgia" ]; then
  # A binary that cannot EXECUTE (missing shared library, wrong architecture,
  # truncated download) must not abort the installer: `set -e` plus `pipefail`
  # would make the failing pipeline kill this assignment and end the script
  # with no output at all, leaving the broken binary in place and the user with
  # no way to repair it by re-running. Fall through to a fresh install instead.
  installed=""
  if version_line=$("$PREFIX/nolgia" --version 2>/dev/null); then
    installed="v$(printf '%s\n' "$version_line" | awk '{print $2}')"
  fi
  if [ "$installed" = "$TAG" ]; then
    echo "nolgia $TAG is already installed at $PREFIX/nolgia — nothing to do"
    ensure_on_path
    printf 'export PATH="%s:$PATH"\n' "$PREFIX"
    exit 0
  fi
fi

BASE="https://github.com/$REPO/releases/download/$TAG"
if [ -n "${NOLGIA_INSTALL_SOURCE:-}" ]; then
  echo "installing nolgia $TAG ($asset) from $NOLGIA_INSTALL_SOURCE"
  cp "$NOLGIA_INSTALL_SOURCE" "$tmp/nolgia"
else
  url="$BASE/$asset"
  echo "downloading nolgia $TAG ($asset)..."
  if ! curl -fL --progress-bar "$url" -o "$tmp/nolgia"; then
    echo "could not download $asset for nolgia $TAG" >&2
    echo "the release may predate a build for $os/$arch (Linux and Windows arm64 builds start after v0.2.26); install with: cargo install nolgia-cli (or pass --tag for a newer release)" >&2
    exit 1
  fi
fi

sums_available=0
if [ -n "${NOLGIA_INSTALL_SUMS:-}" ]; then
  if [ -f "$NOLGIA_INSTALL_SUMS" ]; then
    cp "$NOLGIA_INSTALL_SUMS" "$tmp/SHA256SUMS"
    sums_available=1
  fi
elif [ -z "${NOLGIA_INSTALL_SOURCE:-}" ]; then
  if curl -fsSL "$BASE/SHA256SUMS" -o "$tmp/SHA256SUMS"; then
    sums_available=1
  fi
fi

if [ "$sums_available" = 0 ]; then
  echo "note: no SHA256SUMS published for nolgia $TAG; skipping checksum verification"
else
  expected=$(awk -v asset="$asset" '$2 == asset {print tolower($1); exit}' "$tmp/SHA256SUMS")
  if [ -z "$expected" ]; then
    echo "error: SHA256SUMS for nolgia $TAG has no entry for $asset" >&2
    exit 1
  fi

  tool=""
  if command -v sha256sum >/dev/null 2>&1; then
    tool=sha256sum
    actual=$(sha256sum "$tmp/nolgia" | awk '{print tolower($1)}')
  elif command -v shasum >/dev/null 2>&1; then
    tool=shasum
    actual=$(shasum -a 256 "$tmp/nolgia" | awk '{print tolower($1)}')
  elif command -v openssl >/dev/null 2>&1; then
    tool=openssl
    actual=$(openssl dgst -sha256 "$tmp/nolgia" | awk '{print tolower($NF)}')
  fi

  if [ -z "$tool" ]; then
    echo "warning: cannot verify the download (no sha256sum, shasum or openssl on this machine)" >&2
  elif [ "$actual" != "$expected" ]; then
    echo "error: checksum mismatch for $asset (expected $expected, got $actual); refusing to install" >&2
    exit 1
  else
    echo "verified $asset sha256 ${actual:0:12}… with $tool"
  fi
fi
chmod +x "$tmp/nolgia"

if [ "$os" = "Darwin" ]; then
  xattr -d com.apple.quarantine "$tmp/nolgia" 2>/dev/null || true
fi

mv -f "$tmp/nolgia" "$PREFIX/nolgia"

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/nolgia"
mkdir -p "$config_dir"
printf '{"method":"install.sh","tag":"%s","prefix":"%s","installed_at":"%s"}\n' \
  "$TAG" "$PREFIX" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$config_dir/install-metadata.json"

# Report what the binary itself says, but never claim a silent success: if it
# cannot run here (a missing system library, or a glibc older than the one it
# was built against), say so plainly rather than printing "installed  to ...".
if version_line=$("$PREFIX/nolgia" --version 2>/dev/null); then
  echo "installed $version_line to $PREFIX/nolgia"
else
  echo "installed nolgia $TAG to $PREFIX/nolgia, but it does not run on this machine:" >&2
  "$PREFIX/nolgia" --version 2>&1 | sed 's/^/  /' >&2 || true
  echo "  report this at https://github.com/$REPO/issues with the line above" >&2
  exit 1
fi
ensure_on_path
printf 'export PATH="%s:$PATH"\n' "$PREFIX"
