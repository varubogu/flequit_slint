#!/usr/bin/env bash
# Generate .codex/skills/ from .claude/skills/ (the canonical source).
#
#   ./scripts/sync-agent-skills.sh          # write .codex/skills/
#   ./scripts/sync-agent-skills.sh --check  # fail if out of sync (for CI)
#
# .claude/skills/ is authoritative. Never edit .codex/skills/ by hand.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$repo_root/.claude/skills"
dst="$repo_root/.codex/skills"

check_mode=0
if [[ "${1:-}" == "--check" ]]; then
  check_mode=1
elif [[ $# -gt 0 ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi

if [[ ! -d "$src" ]]; then
  echo "error: canonical skill directory not found: $src" >&2
  exit 1
fi

# Claude -> Codex terminology rewrite.
# Note: references to `.claude/skills/` are intentionally NOT rewritten; that
# path stays canonical for both agents. Only the user-level skill location and
# the agent name differ.
transform() {
  sed -e 's|^# Flequit (Slint) Agent Skills$|# Flequit (Slint) Codex Skills|' \
      -e 's|`~/\.claude/skills/` or `~/\.codex/skills/`|`~/.codex/skills/`|g' \
      -e 's|`~/\.claude/skills/`|`~/.codex/skills/`|g' \
      -e 's|Claude Code|Codex|g' \
      -e 's|`CLAUDE\.md` / `AGENTS\.md`|`AGENTS.md`|g'
}

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

while IFS= read -r -d '' file; do
  rel="${file#"$src"/}"
  mkdir -p "$staging/$(dirname "$rel")"
  transform < "$file" > "$staging/$rel"
done < <(find "$src" -type f -name '*.md' -print0)

# Non-markdown assets are copied verbatim.
while IFS= read -r -d '' file; do
  rel="${file#"$src"/}"
  mkdir -p "$staging/$(dirname "$rel")"
  cp "$file" "$staging/$rel"
done < <(find "$src" -type f ! -name '*.md' -print0)

if [[ $check_mode -eq 1 ]]; then
  if [[ ! -d "$dst" ]]; then
    echo "error: $dst is missing. Run ./scripts/sync-agent-skills.sh" >&2
    exit 1
  fi
  if ! diff -r -q "$staging" "$dst" >/dev/null 2>&1; then
    echo "error: .codex/skills/ is out of sync with .claude/skills/" >&2
    diff -r "$staging" "$dst" || true
    echo "" >&2
    echo "Run: ./scripts/sync-agent-skills.sh" >&2
    exit 1
  fi
  echo "OK: .codex/skills/ is in sync with .claude/skills/"
  exit 0
fi

rm -rf "$dst"
mkdir -p "$(dirname "$dst")"
cp -r "$staging" "$dst"
echo "Generated $dst from $src ($(find "$dst" -type f | wc -l | tr -d ' ') files)"
