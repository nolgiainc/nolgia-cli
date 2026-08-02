#!/usr/bin/env bash
# check-version-consistency.sh — refuse to cut a release the artifacts will lie about.
#
# WHY THIS EXISTS
#   v0.2.13 and v0.2.14 were tagged on trees whose manifests still read 0.2.12.
#   The tags were created, the binaries were built, the GitHub Releases were
#   published — and every one of those artifacts self-reports `nolgia 0.2.12`,
#   because the version a binary prints comes from CARGO_PKG_VERSION, i.e. from
#   the manifest, not from the tag. Downstream, nolgia-agent pinned v0.2.14 and
#   its pods could never converge: the install reconcile compares
#   `nolgia --version` against the pinned tag, and those can never be equal
#   (NOL-328, NOL-294).
#
#   A guard already existed, but it was the wrong guard in the wrong place: it
#   checked ONLY npm/package.json, and it lived in `publish-npm`, the LAST job.
#   By the time it fired, crates.io had already been published to, the binaries
#   were built and the GitHub Release existed. It could redden a run; it could
#   not prevent a bad release. Worse, `publish-crates` reported SUCCESS on both
#   bad tags — it tried to publish 0.2.12, hit "already on crates.io", and took
#   the skip branch — so the run looked half-fine while shipping mislabelled
#   binaries.
#
#   This runs FIRST and gates every publishing job, and it checks every
#   version-bearing manifest, not one of them.
#
# WHAT IT CHECKS
#   1. agree  — all six version declarations across the four manifests match
#               each other:
#                 Cargo.toml            [workspace.package] version
#                 Cargo.toml            [workspace.dependencies] nolgia-client
#                 crates/client/Cargo.toml  [package] version   (NOT inherited)
#                 Cargo.lock            nolgia-cli
#                 Cargo.lock            nolgia-client
#                 npm/package.json      version
#   2. tag    — with --tag <vX.Y.Z>, that agreed version equals the tag. This is
#               the check that would have stopped v0.2.13 and v0.2.14.
#
# EXIT CODES
#   0  consistent (and matching the tag, if one was given)
#   1  a mismatch
#   2  usage / a manifest could not be parsed
#
# USAGE
#   scripts/check-version-consistency.sh                 # manifests agree?
#   scripts/check-version-consistency.sh --tag v0.2.15   # …and match the tag?
#
# Sourcing this file (VERSION_CHECK_LIB=1) defines the functions without running
# the checks — see tests/check-version-consistency_test.sh.
set -euo pipefail

REPO_ROOT="${REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
TAG=""

_annot() { # _annot <error|warning|notice> <message>
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    printf '::%s::%s\n' "$1" "$2"
  else
    printf '%s: %s\n' "$(printf '%s' "$1" | tr '[:lower:]' '[:upper:]')" "$2"
  fi
}
fail_msg() { _annot error "$*"; }

# ------------------------------------------------------------------ extraction
# All parsers are section-aware. A bare `grep '^version'` over a Cargo.toml would
# happily read a dependency's version, which is how this class of bug hides.

# `version` inside a named TOML table, e.g. [workspace.package] or [package].
toml_table_version() { # toml_table_version <file> <table>
  awk -v want="$2" '
    /^[[:space:]]*\[/ { sect = $0; sub(/[[:space:]]*$/, "", sect); next }
    sect == want && /^[[:space:]]*version[[:space:]]*=/ {
      if (match($0, /"[^"]*"/)) { print substr($0, RSTART + 1, RLENGTH - 2); found = 1; exit }
    }
    END { if (!found) exit 1 }
  ' "$1"
}

# The `version = "..."` field of a path dependency declared in a TOML table,
# e.g. nolgia-client = { path = "crates/client", version = "0.2.15" }.
toml_dep_version() { # toml_dep_version <file> <table> <dep-name>
  awk -v want="$2" -v dep="$3" '
    /^[[:space:]]*\[/ { sect = $0; sub(/[[:space:]]*$/, "", sect); next }
    sect == want && index($0, dep) == 1 {
      if (match($0, /version[[:space:]]*=[[:space:]]*"[^"]*"/)) {
        f = substr($0, RSTART, RLENGTH)
        match(f, /"[^"]*"/)
        print substr(f, RSTART + 1, RLENGTH - 2); found = 1; exit
      }
    }
    END { if (!found) exit 1 }
  ' "$1"
}

# The version of a named [[package]] in Cargo.lock. The lock is generated, but a
# stale one is exactly what a forgotten `cargo update` leaves behind, and it is
# what the build actually resolves.
lock_version() { # lock_version <Cargo.lock> <crate-name>
  awk -v want="$2" '
    /^\[\[package\]\]/ { name = ""; next }
    /^name[[:space:]]*=/    { if (match($0, /"[^"]*"/)) name = substr($0, RSTART + 1, RLENGTH - 2); next }
    /^version[[:space:]]*=/ {
      if (name == want && match($0, /"[^"]*"/)) {
        print substr($0, RSTART + 1, RLENGTH - 2); found = 1; exit
      }
    }
    END { if (!found) exit 1 }
  ' "$1"
}

# package.json version. Prefer a real JSON parser; fall back to a scoped sed so
# the guard still runs on a machine without node or python.
npm_version() { # npm_version <package.json>
  local v=""
  if command -v node >/dev/null 2>&1; then
    v=$(node -p "require('$1').version" 2>/dev/null) || v=""
  fi
  if [ -z "$v" ] && command -v python3 >/dev/null 2>&1; then
    v=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$1" 2>/dev/null) || v=""
  fi
  if [ -z "$v" ]; then
    v=$(sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -1)
  fi
  [ -n "$v" ] || return 1
  printf '%s\n' "$v"
}

# ------------------------------------------------------------------ collection

# Print "<label>\t<version>" for every version-bearing declaration. One line per
# declaration, so adding a manifest means adding exactly one line here.
collect_versions() { # collect_versions <repo-root>
  local root="$1" v
  v=$(toml_table_version "$root/Cargo.toml" "[workspace.package]") ||
    { echo "could not read [workspace.package] version from Cargo.toml" >&2; return 1; }
  printf 'Cargo.toml [workspace.package]\t%s\n' "$v"

  v=$(toml_dep_version "$root/Cargo.toml" "[workspace.dependencies]" "nolgia-client") ||
    { echo "could not read the nolgia-client dep version from Cargo.toml" >&2; return 1; }
  printf 'Cargo.toml nolgia-client dep\t%s\n' "$v"

  v=$(toml_table_version "$root/crates/client/Cargo.toml" "[package]") ||
    { echo "could not read [package] version from crates/client/Cargo.toml" >&2; return 1; }
  printf 'crates/client/Cargo.toml\t%s\n' "$v"

  v=$(lock_version "$root/Cargo.lock" "nolgia-cli") ||
    { echo "could not read nolgia-cli from Cargo.lock" >&2; return 1; }
  printf 'Cargo.lock nolgia-cli\t%s\n' "$v"

  v=$(lock_version "$root/Cargo.lock" "nolgia-client") ||
    { echo "could not read nolgia-client from Cargo.lock" >&2; return 1; }
  printf 'Cargo.lock nolgia-client\t%s\n' "$v"

  v=$(npm_version "$root/npm/package.json") ||
    { echo "could not read version from npm/package.json" >&2; return 1; }
  printf 'npm/package.json\t%s\n' "$v"
}

# ------------------------------------------------------------------ main

main() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --tag) TAG="$2"; shift 2 ;;
      -h|--help)
        awk 'NR > 1 { if (!/^#/) exit; sub(/^# ?/, ""); print }' "${BASH_SOURCE[0]}"
        exit 0 ;;
      *) echo "Unknown argument: $1" >&2; exit 2 ;;
    esac
  done

  local rows
  rows=$(collect_versions "$REPO_ROOT") || exit 2
  printf '%s\n' "$rows" | while IFS=$'\t' read -r label version; do
    printf '  %-32s %s\n' "$label" "$version"
  done

  local distinct count
  distinct=$(printf '%s\n' "$rows" | cut -f2 | sort -u)
  count=$(printf '%s\n' "$distinct" | grep -c .)

  if [ "$count" -ne 1 ]; then
    fail_msg "manifest versions disagree: $(printf '%s' "$distinct" | tr '\n' ' '). \
Every artifact this repo ships takes its version from a manifest, not from the \
tag, so a split here means a release that misreports itself. Set all of them to \
the same value (a bump touches all six declarations)."
    return 1
  fi

  local agreed
  agreed=$(printf '%s\n' "$distinct")
  echo "OK: all manifests agree on $agreed"

  if [ -n "$TAG" ]; then
    if [ "v$agreed" != "$TAG" ]; then
      fail_msg "tag/manifest mismatch: tag $TAG would be cut on a tree whose \
manifests read $agreed, so every binary built from it would report \
'nolgia $agreed' and crates.io would see $agreed. That is exactly what shipped \
as v0.2.13 and v0.2.14 (NOL-328). Bump the manifests to ${TAG#v} and re-tag."
      return 1
    fi
    echo "OK: manifests match the tag $TAG"
  fi

  return 0
}

if [ -z "${VERSION_CHECK_LIB:-}" ]; then
  main "$@"
fi
