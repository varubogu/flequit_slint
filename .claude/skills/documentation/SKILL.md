---
name: documentation
description: Create and update Flequit project documentation under docs/, following the ja-first policy, naming conventions, and code-example rules. Use when editing anything in the docs directory.
---

# Documentation Skill

Use this skill when editing files under `docs/`.

## Language policy

- **Current state**: only `docs/ja/` exists. `docs/en/` has not been created yet.
- Treat `docs/ja/` as the source of truth. Update it alone; no en sync required yet.
- When `docs/en/` is added (a separate task), the default becomes: keep both
  aligned in structure and content, updated in the same change.
- Do not create `docs/en/` opportunistically as part of an unrelated change.

## Editing rules

Follow `docs/ja/develop/rules/documentation.md`:

- File names: kebab-case (exception: filenames mirroring DB table names)
- No spaces in filenames (use hyphen or underscore)
- Avoid duplicate content across files; cross-reference instead
- Prefer relative links inside the docs tree

## Code examples in docs

- **Allowed**: `docs/ja/develop/rules/` files (they are coding rules and benefit from examples)
- **Allowed**: table design (field tables, constraint tables, indexed columns) in `entity/` files
- **Allowed**: short `.slint` snippets in `design/ui/` when illustrating a pattern
  that cannot be described in prose
- **Not allowed**: SQL DDL, Sea-ORM code, full Rust service implementations in
  `design/` docs — reference `crates/...` instead

## Structure

```
docs/ja/develop/
├── commands.md
├── design/
│   ├── architecture.md, tech-stack.md, testing.md, deployment.md, error-handling.md
│   ├── api/          # future sync server
│   ├── backend/      # Rust guidelines, transactions
│   ├── data/         # data model, Automerge, entities
│   ├── platform/     # platform abstraction
│   └── ui/           # Slint, ViewModel, responsive, i18n, pages
├── requirements/
└── rules/
```

## Porting notes

This repo is a Slint port of a SvelteKit + Tauri app. Several design docs end with
a comparison table ("Svelte 版からの変更点" / "Tauri 版との違い"). When changing a
design that diverges from the original, update that table too — it is the record
of why the divergence exists.

## Workflow

1. Identify the doc to change and any cross-referencing docs.
2. Apply the change; keep terminology consistent.
3. Verify links resolve (relative paths, correct depth).
4. If the change affects crate layout or commands, update
   `design/tech-stack.md`, `rules/file-structure.md`, and `commands.md` together.
