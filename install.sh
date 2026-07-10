#!/usr/bin/env bash
# Nolgia CLI installer
#
#   curl -fsSL https://raw.githubusercontent.com/nolgiainc/nolgia-cli/main/install.sh | bash
#
# Installs to a user-writable directory (no sudo, no password prompts):
# ~/.local/bin by default, falling back to ~/bin. If the chosen directory is
# not on PATH, the matching export line is appended to your shell profile.
# Re-running with the requested version already installed is a no-op.
#
# Options:
#   --prefix <dir>   install directory (default: ~/.local/bin, else ~/bin)
#   --system         install to /usr/local/bin instead (needs write access
#                    there — typically root; NOT the default on purpose)
#   --tag <vX.Y.Z>   release tag to install (default: latest)
#
# Test hook: NOLGIA_INSTALL_SOURCE=<path> copies a local file instead of
# downloading the release asset (used by tests/install_sh_test.sh).

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

os=$(uname -s)
arch=$(uname -m)
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
      *)
        echo "no prebuilt binary for Linux/$arch yet; install with: cargo install nolgia-cli" >&2
        exit 1
        ;;
    esac
    ;;
  MINGW* | MSYS* | CYGWIN*)
    echo "on Windows, download nolgia-x86_64-pc-windows-msvc.exe from https://github.com/$REPO/releases or install with: cargo install nolgia-cli" >&2
    exit 1
    ;;
  *)
    echo "unsupported platform: $os/$arch; install with: cargo install nolgia-cli" >&2
    exit 1
    ;;
esac

if [ -z "$TAG" ]; then
  TAG=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
    awk -F'"' '/"tag_name"/ {print $4; exit}')
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
  installed="v$("$PREFIX/nolgia" --version 2>/dev/null | awk '{print $2}')"
  if [ "$installed" = "$TAG" ]; then
    echo "nolgia $TAG is already installed at $PREFIX/nolgia — nothing to do"
    ensure_on_path
    exit 0
  fi
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ -n "${NOLGIA_INSTALL_SOURCE:-}" ]; then
  cp "$NOLGIA_INSTALL_SOURCE" "$tmp/nolgia"
else
  url="https://github.com/$REPO/releases/download/$TAG/$asset"
  echo "downloading nolgia $TAG ($asset)..."
  curl -fL --progress-bar "$url" -o "$tmp/nolgia"
fi
chmod +x "$tmp/nolgia"

if [ "$os" = "Darwin" ]; then
  xattr -d com.apple.quarantine "$tmp/nolgia" 2>/dev/null || true
fi

mv -f "$tmp/nolgia" "$PREFIX/nolgia"

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/nolgia"
mkdir -p "$config_dir"
cat > "$config_dir/install-metadata.json" <<METADATA
{"method":"install.sh","tag":"$TAG","prefix":"$PREFIX","installed_at":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
METADATA

echo "installed $("$PREFIX/nolgia" --version) to $PREFIX/nolgia"
ensure_on_path
