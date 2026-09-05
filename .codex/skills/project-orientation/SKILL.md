---
name: project-orientation
description: Orient to the Flequit (Slint) repository context, including application overview, tech stack, crate layout, documentation index, and scope policy. Use when starting broad codebase work, deciding where to look, or updating project guidance.
---

# Project Orientation Skill

Use this skill to load repository context without bloating `AGENTS.md`.

## Scope policy

- Project-specific constraints live in `AGENTS.md` and `docs/`.
- Project-specific task procedures live in repo-local skills: `.claude/skills/` (canonical).
- Reusable cross-project procedures belong in user-level skills.
- Do not embed repo-specific rules in shared user-level skills.

## Application overview

- Native task management app with project/task collaboration.
- Rust single process. No WebView, no IPC, no Node.js toolchain.
- Ported from the SvelteKit + Tauri implementation (`varubogu/flequit`).
- Local-first SQLite today; Automerge (CRDT) prepares for future sync.
- Targets: Windows / macOS / Linux (Phase 1), Android / iOS (Phase 2). Web is out of scope.

## Tech stack

- UI: Slint 1.x, `.slint` declarative language, bundled translations for i18n.
- Core: Rust edition 2024, Sea-ORM + SQLite, Automerge, Tokio, `tracing`.
- Platform: `flequit-platform` crate wraps all OS APIs (paths, notifications, dialogs).
- Package manager: Cargo only.

Details: `docs/ja/develop/design/tech-stack.md`.

## Crate layout

```
flequit-types
  ├→ flequit-platform                     (leaf: OS abstraction)
  └→ flequit-model → flequit-repository → flequit-core
       → flequit-infrastructure-{sqlite,automerge} → flequit-infrastructure
         → flequit-ui (Slint + ViewModel) → flequit-app (binary)
```

Verify with `./scripts/check-crate-deps.sh`.

## Primary references

- Architecture and design: `docs/ja/develop/design/`
- Development rules: `docs/ja/develop/rules/`
- Requirements: `docs/ja/develop/requirements/`
- Commands: `docs/ja/develop/commands.md`

Frequently used files:

- `docs/ja/develop/design/architecture.md`
- `docs/ja/develop/design/tech-stack.md`
- `docs/ja/develop/design/ui/layers.md`
- `docs/ja/develop/design/ui/slint-patterns.md`
- `docs/ja/develop/design/ui/viewmodel-architecture.md`
- `docs/ja/develop/design/ui/responsive-layout.md`
- `docs/ja/develop/design/platform/platform-abstraction.md`
- `docs/ja/develop/design/backend/rust-guidelines.md`
- `docs/ja/develop/rules/coding-standards.md`
- `docs/ja/develop/rules/ui.md`
- `docs/ja/develop/rules/backend.md`
- `docs/ja/develop/rules/testing.md`
- `docs/ja/develop/rules/workflow.md`

## Porting note

When a design decision looks unusual, check the "Svelte 版からの変更点" /
"Tauri 版との違い" table at the end of the relevant design doc. It records why
the Slint version diverges from the original implementation.
