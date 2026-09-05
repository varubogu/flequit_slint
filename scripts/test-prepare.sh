#!/usr/bin/env bash
# Prepares fixtures that tests touching real storage depend on.
#
#   ./scripts/test-prepare.sh              # everything
#   ./scripts/test-prepare.sh automerge    # Automerge output directories
#   ./scripts/test-prepare.sh db           # SQLite template database
#   ./scripts/test-prepare.sh db --force   # rebuild the template database
#
# Migrating once into a template that individual tests copy keeps the number of
# migrations at one, no matter how many tests run.
# See docs/ja/develop/rules/testing.md

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

tmp_root=".tmp/tests"
template_db="$tmp_root/test_database.db"

prepare_automerge() {
  mkdir -p "$tmp_root/cargo/flequit-infrastructure-automerge/automerge"
  mkdir -p "$tmp_root/cargo/flequit-infrastructure-automerge/json"
  echo "ok:   automerge test directories ready under $tmp_root"
}

prepare_db() {
  local force="${1:-}"

  mkdir -p "$tmp_root"

  if [[ "$force" == "--force" ]]; then
    echo "info: rebuilding $template_db"
  elif [[ -f "$template_db" ]]; then
    echo "ok:   $template_db already exists (pass --force to rebuild)"
    return
  fi

  cargo run --quiet -p flequit-infrastructure-sqlite --bin migration_runner -- \
    "$template_db" ${force:+--force}

  echo "ok:   $template_db migrated"
}

target="${1:-all}"
case "$target" in
  automerge)
    prepare_automerge
    ;;
  db)
    prepare_db "${2:-}"
    ;;
  all)
    prepare_automerge
    prepare_db "${2:-}"
    ;;
  *)
    echo "usage: $0 [all|automerge|db] [--force]" >&2
    exit 2
    ;;
esac
