#!/usr/bin/env bash
# Verifies the architectural invariants that the compiler cannot enforce.
#
#   ./scripts/check-crate-deps.sh
#
# 1. Crate dependency direction (no crate depends on a later layer).
# 2. Platform conditional compilation is confined to flequit-platform.
# 3. The UI does not reach past flequit-infrastructure into concrete storage.
#
# See docs/ja/develop/design/ui/layers.md and
#     docs/ja/develop/design/backend/rust-guidelines.md

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

failures=0

fail() {
  echo "FAIL: $*" >&2
  failures=$((failures + 1))
}

pass() {
  echo "ok:   $*"
}

# --- 1. Forbidden direct dependencies -------------------------------------
#
# Declared in Cargo.toml, so violations are caught before they compile.

check_no_dep() {
  local crate="$1" forbidden="$2"
  local manifest="crates/$crate/Cargo.toml"

  if [[ ! -f "$manifest" ]]; then
    fail "$manifest not found"
    return
  fi

  if grep -qE "^[[:space:]]*${forbidden}([[:space:]]*=|\.workspace)" "$manifest"; then
    fail "$crate must not depend on $forbidden (see $manifest)"
  else
    pass "$crate does not depend on $forbidden"
  fi
}

# The UI talks to the integrated infrastructure facade, never to a concrete
# storage crate or a repository trait.
check_no_dep flequit-ui flequit-infrastructure-sqlite
check_no_dep flequit-ui flequit-infrastructure-automerge
check_no_dep flequit-ui flequit-repository

# The domain layer must not know about persistence implementations.
check_no_dep flequit-core flequit-infrastructure
check_no_dep flequit-core flequit-infrastructure-sqlite
check_no_dep flequit-core flequit-infrastructure-automerge

# Nothing below the UI may depend on Slint.
for crate in flequit-types flequit-platform flequit-model flequit-repository \
             flequit-core flequit-infrastructure flequit-infrastructure-sqlite \
             flequit-infrastructure-automerge flequit-settings; do
  check_no_dep "$crate" slint
done

# The platform crate is a leaf: it must not pull in domain or storage layers.
for forbidden in flequit-model flequit-repository flequit-core flequit-infrastructure; do
  check_no_dep flequit-platform "$forbidden"
done

# --- 2. Platform cfg isolation --------------------------------------------
#
# Conditional compilation on the target OS/arch belongs only to
# flequit-platform/src/platform/. Anywhere else it makes the buildable
# combinations impossible to reason about.

cfg_hits="$(grep -rnE '#\[cfg(_attr)?\([^)]*target_(os|arch|family|vendor)' \
  --include='*.rs' crates/ \
  | grep -v '^crates/flequit-platform/src/platform/' \
  | grep -vE ':[[:space:]]*//' \
  || true)"

if [[ -n "$cfg_hits" ]]; then
  fail "platform cfg outside crates/flequit-platform/src/platform/:"
  echo "$cfg_hits" >&2
else
  pass "platform cfg is confined to flequit-platform/src/platform/"
fi

# --- 3. Hardcoded home-relative paths -------------------------------------
#
# All filesystem access starts from flequit_platform::paths, because mobile
# sandboxes are not predictable at compile time.

path_hits="$(grep -rnE '"(~|/home/|/Users/|C:\\\\Users)' \
  --include='*.rs' crates/ \
  | grep -v '^crates/flequit-platform/' \
  | grep -v '/tests/' \
  || true)"

if [[ -n "$path_hits" ]]; then
  fail "hardcoded user paths found (use flequit_platform::paths):"
  echo "$path_hits" >&2
else
  pass "no hardcoded user paths outside flequit-platform"
fi

# --- Result ----------------------------------------------------------------

echo
if [[ $failures -gt 0 ]]; then
  echo "$failures architectural check(s) failed." >&2
  exit 1
fi
echo "All architectural checks passed."
