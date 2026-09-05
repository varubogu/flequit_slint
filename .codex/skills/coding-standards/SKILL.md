---
name: coding-standards
description: Validate Flequit coding standards including naming across Rust and Slint, file structure, typing discipline, and error handling patterns. Use when checking or aligning code with project conventions.
---

# Coding Standards Skill

Use this skill to check or align code with project conventions.

## Naming

| Target | Convention | Example |
| --- | --- | --- |
| Rust: vars, functions, modules | `snake_case` | `get_task_by_id` |
| Rust: types, traits, enums | `PascalCase` | `TaskViewModel` |
| Rust: constants | `SCREAMING_SNAKE_CASE` | `MAX_TASK_COUNT` |
| Rust files | `snake_case.rs` | `task_viewmodel.rs` |
| Slint: component, struct, enum, global | `PascalCase` | `TaskItem`, `AppState` |
| Slint: property, callback | `kebab-case` | `is-selected`, `toggle-completed` |
| `.slint` files | `kebab-case.slint` | `task-item.slint` |
| Docs | `kebab-case.md` | `responsive-layout.md` |

Slint `kebab-case` becomes `snake_case` in generated Rust APIs
(`is-selected` → `set_is_selected` / `get_is_selected`).

## File structure

- One responsibility per file/module.
- 200+ lines: split. 100+ lines: consider splitting.
- Exceptions: config files, data definitions, migrations.
- `.slint` lives only in `crates/flequit-ui/ui/`.

## Typing

- Use dedicated ID types (`TaskId`, `ProjectId`), never bare `String` IDs.
- Model domain values as enums, not strings.
- Distinguish `Option<T>` from empty string.
- Slint `struct` cannot express `Option` — resolve in the Adapter
  (empty string plus a `has-*: bool` flag).
- Keep return types and domain/UI boundaries explicit.

## Error handling

- `Result<T, E>` with `thiserror` per layer:
  `RepositoryError` → `ServiceError` → `UiError`.
- Public APIs return typed errors, not `anyhow`.
- Never swallow errors (`let _ = ...`).
- `unwrap()` only where impossibility is provable.
- Convert to user-facing text only at display time, in the ViewModel, via i18n codes.

## Imports

Rust order: std → external crates → workspace crates → `crate`/`super`/`self`,
separated by blank lines.

`.slint`: relative paths; `std-widgets.slint` for built-in widgets.
`export` only what is used externally.

## Functions

- Separate computation from side effects; Adapters must be pure.
- Express derived UI values as Slint bindings, not Rust-side recomputation.
- Public APIs carry rustdoc describing usage and contract, not implementation intent.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `./scripts/check-crate-deps.sh`

Details: `docs/ja/develop/rules/coding-standards.md`.
