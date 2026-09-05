# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.
A Japanese mirror of this file is kept at `CLAUDE_ja.md` for human readers; keep both in sync when editing.

## Response Guidelines

- Always respond in **Japanese**.
- After loading this file, first say `✅️ CLAUDE.md loaded`, then follow the instructions.

## Project

Native task management app built with **Rust + Slint** (single process, no WebView, no IPC).
Ported from the SvelteKit + Tauri implementation at `varubogu/flequit`.

- UI: Slint 1.x (`.slint` declarative UI + Rust bindings), bundled translations for i18n
- Core: Rust (edition 2024), Sea-ORM + SQLite, Automerge (CRDT), Tokio, `tracing`
- Targets: Windows / macOS / Linux (Phase 1) + Android / iOS (Phase 2). Web is out of scope.
- Package manager: **Cargo only**. No Node.js / Bun / npm in this repo.

## Critical Rules

- Do not modify unrelated code without asking the user first.
- For regex / bulk replacements, always review the diff before applying.
- On "file/directory not found" errors, check `pwd` first.
- Always cap cargo workers: `cargo test -j 4`.
- Let the user run `git commit` and `git push`.

## Architecture Invariants

These are enforced by CI (`./scripts/check-crate-deps.sh`). Do not break them.

- Crate dependency direction:
  `flequit-types → flequit-model → flequit-repository → flequit-core →
  flequit-infrastructure-* → flequit-infrastructure → flequit-ui → flequit-app`
  (`flequit-platform` is a leaf crate depending only on `flequit-types`)
- `#[cfg(target_os = ...)]` lives **only inside `flequit-platform`**.
- `.slint` files hold no business logic and never see domain types.
- Only ViewModels call `flequit-core` facades. Never call repositories directly from UI.
- UI and core never call OS APIs directly; go through `flequit-platform`.
- Never block the UI thread; update UI from background only via `upgrade_in_event_loop()`.
- Responsive layout branches on **window width**, never on target OS.

## Pointers

- Remaining work, priorities and settled decisions: `plans/plan.md`
- Design / rules / requirements / commands: `docs/ja/develop/{design,rules,requirements,commands.md}`
- `docs/ja/` is the source of truth; `docs/en/` does not exist yet (separate task).
- Task-specific guidance lives in skills (`.claude/skills/`) which auto-trigger —
  do not duplicate skill content here.
- `.codex/skills/` is generated from `.claude/skills/` by `./scripts/sync-agent-skills.sh`.
  Edit `.claude/skills/` and re-run the script; never edit `.codex/skills/` directly.
