#!/usr/bin/env bash
# check-version-consistency_test.sh — offline tests for the release version guard.
#
# The case that matters is the historical one: a tree whose manifests all read
# 0.2.12 being tagged v0.2.14. That is what shipped, twice, and every check in
# the pipeline at the time let it through (NOL-328).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
CHECK="$REPO/scripts/check-version-consistency.sh"

TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT

fails=0
check() { # check <description> <actual> <expected>
  if [ "$2" = "$3" ]; then
    printf '  ok   %s\n' "$1"
  else
    printf '  FAIL %s (got %s, want %s)\n' "$1" "$2" "$3" >&2
    fails=$((fails + 1))
  fi
}

VERSION_CHECK_LIB=1 source "$CHECK"

# ------------------------------------------------------------------ fixtures

# Build a miniature repo. Each version is settable independently so a single
# manifest can be made to lag — which is the whole failure mode.
mk_tree() { # mk_tree <ws> <dep> <client> <lock-cli> <lock-client> <npm>
  local d="$TEST_DIR/tree"
  rm -rf "$d"; mkdir -p "$d/crates/client" "$d/npm"

  cat >"$d/Cargo.toml" <<EOF
[workspace]
members = ["crates/cli", "crates/client"]

[workspace.dependencies]
clap = { version = "4", features = ["derive", "env"] }
reqwest = { version = "0.13", default-features = false }
nolgia-client = { path = "crates/client", version = "$2" }

[workspace.package]
version = "$1"
edition = "2024"
EOF

  cat >"$d/crates/client/Cargo.toml" <<EOF
[package]
name = "nolgia-client"
version = "$3"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
EOF

  cat >"$d/Cargo.lock" <<EOF
version = 4

[[package]]
name = "clap"
version = "4.5.20"

[[package]]
name = "nolgia-cli"
version = "$4"
dependencies = [
 "clap",
]

[[package]]
name = "nolgia-client"
version = "$5"
EOF

  cat >"$d/npm/package.json" <<EOF
{
  "name": "@nolgia/cli",
  "version": "$6",
  "bin": { "nolgia": "bin/nolgia.js" }
}
EOF
  printf '%s\n' "$d"
}

# All six in agreement at <version>.
mk_consistent() { mk_tree "$1" "$1" "$1" "$1" "$1" "$1"; }

run_check() { # run_check <tree> [args…] -> exit status; output in $OUT
  local tree="$1"; shift
  set +e
  OUT=$(REPO_ROOT="$tree" bash "$CHECK" "$@" 2>&1)
  local st=$?
  set -e
  return $st
}

# ------------------------------------------------------------- unit: parsers
T=$(mk_consistent 0.2.15)

check "workspace.package version"        "$(toml_table_version "$T/Cargo.toml" '[workspace.package]')" "0.2.15"
check "client crate [package] version"   "$(toml_table_version "$T/crates/client/Cargo.toml" '[package]')" "0.2.15"
check "nolgia-client dep version"        "$(toml_dep_version "$T/Cargo.toml" '[workspace.dependencies]' nolgia-client)" "0.2.15"
check "Cargo.lock nolgia-cli"            "$(lock_version "$T/Cargo.lock" nolgia-cli)" "0.2.15"
check "Cargo.lock nolgia-client"         "$(lock_version "$T/Cargo.lock" nolgia-client)" "0.2.15"
check "npm/package.json"                 "$(npm_version "$T/npm/package.json")" "0.2.15"

# Section-awareness: clap's `version = "4"` sits ABOVE [workspace.package] and a
# naive `grep '^version'` would return it. That is how a version parser silently
# reads the wrong number.
check "parser is not fooled by a dependency version" \
  "$(toml_table_version "$T/Cargo.toml" '[workspace.package]')" "0.2.15"
check "lock parser is not fooled by an earlier package" \
  "$(lock_version "$T/Cargo.lock" nolgia-cli)" "0.2.15"

# ------------------------------------------------------------------ behaviour

# Consistent manifests, no tag.
T=$(mk_consistent 0.2.15)
run_check "$T" && st=0 || st=$?
check "consistent manifests pass" "$st" "0"

# Consistent manifests matching the tag.
run_check "$T" --tag v0.2.15 && st=0 || st=$?
check "manifests matching the tag pass" "$st" "0"
check "…and say so" "$(printf '%s' "$OUT" | grep -c 'match the tag')" "1"

# THE HISTORICAL CASE. Every manifest reads 0.2.12; the tag being cut is
# v0.2.14. This is precisely what happened, and it must be refused.
T=$(mk_consistent 0.2.12)
run_check "$T" --tag v0.2.14 && st=0 || st=$?
check "v0.2.14 on a 0.2.12 tree FAILS" "$st" "1"
check "…and names the mismatch" "$(printf '%s' "$OUT" | grep -c 'tag/manifest mismatch')" "1"
check "…and the manifests were internally consistent" \
  "$(printf '%s' "$OUT" | grep -c 'all manifests agree on 0.2.12')" "1"

# v0.2.13 likewise.
run_check "$T" --tag v0.2.13 && st=0 || st=$?
check "v0.2.13 on a 0.2.12 tree FAILS" "$st" "1"

# A partial bump — the shape where someone edits Cargo.toml but forgets the lock
# and npm. The old npm-only check would have caught the npm half alone.
T=$(mk_tree 0.2.16 0.2.16 0.2.16 0.2.12 0.2.12 0.2.12)
run_check "$T" --tag v0.2.16 && st=0 || st=$?
check "stale Cargo.lock + npm fails" "$st" "1"
check "…and reports disagreement, not a tag mismatch" \
  "$(printf '%s' "$OUT" | grep -c 'manifest versions disagree')" "1"

# Only npm lagging: the exact defect the old guard did catch, still caught.
T=$(mk_tree 0.2.16 0.2.16 0.2.16 0.2.16 0.2.16 0.2.12)
run_check "$T" --tag v0.2.16 && st=0 || st=$?
check "npm/package.json alone lagging fails" "$st" "1"

# Only the client crate lagging: a manifest the OLD guard never looked at.
T=$(mk_tree 0.2.16 0.2.16 0.2.12 0.2.16 0.2.16 0.2.16)
run_check "$T" --tag v0.2.16 && st=0 || st=$?
check "crates/client alone lagging fails" "$st" "1"

# Only the workspace dep pin lagging.
T=$(mk_tree 0.2.16 0.2.12 0.2.16 0.2.16 0.2.16 0.2.16)
run_check "$T" --tag v0.2.16 && st=0 || st=$?
check "nolgia-client dep pin alone lagging fails" "$st" "1"

# ------------------------------------------------------------------ real repo
# Not a fixture: the tree as it stands must be internally consistent, so a
# forgotten manifest reddens CI on the PR rather than at tag time.
run_check "$REPO" && st=0 || st=$?
check "the real repo is self-consistent" "$st" "0"

if [ "$fails" -ne 0 ]; then
  echo "$fails version consistency test(s) failed" >&2
  exit 1
fi
echo "version consistency tests passed"
