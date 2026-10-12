#!/usr/bin/env bash
# Regenerate the UI translation template from @tr() strings.
#
#   ./scripts/extract-ui-pot.sh                 # write i18n/flequit-ui.pot
#   ./scripts/extract-ui-pot.sh /tmp/out.pot    # write another path
#
# slint-tr-extractor always records each reference as path:line and has no
# flag to omit the line (see its --help). A blank line above an @tr() would
# otherwise change the committed template and fail CI. Drop only that line
# number so the reference stays the file. The comparison in
# .github/workflows/i18n.yml still ignores POT-Creation-Date and diffs the
# rest unchanged.
# See docs/ja/develop/commands.md

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

output="${1:-i18n/flequit-ui.pot}"
if [[ $# -gt 1 ]]; then
  echo "usage: $0 [output.pot]" >&2
  exit 2
fi

raw="$(mktemp)"
trap 'rm -f "$raw"' EXIT

# sort keeps the entry order independent of the filesystem's directory order.
find crates/flequit-ui/ui -name '*.slint' \
  | sort \
  | xargs slint-tr-extractor -o "$raw"

sed -E 's/^(#: .*):[0-9]+$/\1/' "$raw" > "$output"
