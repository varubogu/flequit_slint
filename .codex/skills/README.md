# Flequit (Slint) Codex Skills

Project-specific skills for Flequit's Slint implementation.

`.claude/skills/` is the **canonical source**. `.codex/skills/` is generated from it by
`./scripts/sync-agent-skills.sh`. Edit `.claude/skills/` and re-run the script;
never edit `.codex/skills/` directly.

## Skill list

| Skill | Purpose | Entry doc |
| --- | --- | --- |
| `project-orientation` | App overview, tech stack, docs index, scope policy | `docs/ja/develop/design/architecture.md` |
| `slint-ui` | Slint UI + ViewModel implementation | `docs/ja/develop/design/ui/slint-patterns.md` |
| `platform-abstraction` | Cross-platform / mobile work | `docs/ja/develop/design/platform/platform-abstraction.md` |
| `architecture-review` | Layer + crate dependency compliance | `docs/ja/develop/design/ui/layers.md`, `docs/ja/develop/design/backend/rust-guidelines.md` |
| `testing` | Rust test workflow (unit / integration / ViewModel) | `docs/ja/develop/rules/testing.md` |
| `debugging` | UI / core debugging workflow | - |
| `i18n` | Slint bundled translations | `docs/ja/develop/design/ui/i18n-system.md` |
| `documentation` | Documentation editing (ja-first policy) | `docs/ja/develop/rules/documentation.md` |
| `coding-standards` | Naming / typing / error handling checks | `docs/ja/develop/rules/coding-standards.md` |

## Usage policy

- Keep these skills in **English** to reduce context size.
- Project-specific rules live in `AGENTS.md` and `docs/ja/develop/`.
- Reusable cross-project workflows belong in user-level skills
  (`~/.codex/skills/`).
- Skills are auto-selected based on the user request.
- Each `SKILL.md` should remain concise and task-oriented; details belong in `docs/ja/`.

## Notes on doc references

- `docs/ja/` is authoritative. `docs/en/` does not exist yet (separate task).
- Code examples are kept only in `docs/ja/develop/rules/` files.
  Source code (`crates/`) is the canonical implementation reference.
