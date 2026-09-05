# AGENTS.md

Project-specific guidance for Codex in this repository. Claude Code uses `CLAUDE.md` instead.

## Response Guidelines

- Always respond in Japanese.
- After loading this file, first print `✅️ AGENTS.md loaded`.
- Keep `AGENTS.md` and skill files primarily in English to reduce context size.

## Core Rules

- Use **Cargo only**; there is no Node.js / Bun / npm in this repo.
- Always cap cargo workers: `cargo test -j 4`.
- Keep crate dependency order:
  `flequit-types -> flequit-model -> flequit-repository -> flequit-core ->
  flequit-infrastructure-* -> flequit-infrastructure -> flequit-ui -> flequit-app`
  (`flequit-platform` is a leaf crate depending only on `flequit-types`).
- Keep `#[cfg(target_os = ...)]` **inside `flequit-platform` only**.
- Keep `.slint` free of business logic; it must not reference domain types.
- Only ViewModels call `flequit-core` facades; never call repositories from UI.
- Route all OS API access through `flequit-platform`.
- Never block the UI thread; update UI from background threads only via
  `slint::Weak::upgrade_in_event_loop()`.
- Branch responsive layout on window width, never on target OS.
- Do not change unrelated code unless the user explicitly approves.
- Verify scope and side effects before broad regex-style replacements.
- If a command fails with missing path/file errors, check `pwd` first.
- Let the user run `git commit` and `git push`.

## Critical Commands

- Check: `cargo check --quiet`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt --all`
- Test: `cargo test -j 4` (always cap workers)
- Run: `cargo run -p flequit-app`
- Dependency/cfg guard: `./scripts/check-crate-deps.sh`
- i18n extract: `find crates/flequit-ui/ui -name '*.slint' | xargs slint-tr-extractor -o i18n/flequit-ui.pot`

## Codex Skills

- Remaining work, priorities and settled decisions live in `plans/plan.md`. Read it before
  proposing a direction that section 2 already settled.
- Repo-local skills live under `.codex/skills/`; use them for task-specific details.
- Use `project-orientation` for app overview, tech stack, docs index, and scope policy.
- Use focused skills for Slint UI work, platform abstraction, testing, architecture review,
  debugging, i18n, documentation, and coding standards.
- Reusable cross-project procedures belong in user-level skills (`~/.codex/skills/`), not this repo.
- `.codex/skills/` is **generated** from `.claude/skills/` by `./scripts/sync-agent-skills.sh`.
  Edit `.claude/skills/` and re-run the script; do not edit `.codex/skills/` directly.
