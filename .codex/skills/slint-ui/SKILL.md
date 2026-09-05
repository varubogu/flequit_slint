---
name: slint-ui
description: Implement Slint UI components and ViewModels for Flequit, covering property/callback wiring, model updates, optimistic updates, thread boundaries, and responsive layout. Use when adding or modifying anything under crates/flequit-ui.
---

# Slint UI Skill

Use this skill for work in `crates/flequit-ui` (`.slint` files, ViewModels, Adapters).

## Layer responsibilities

| Layer | Path | Allowed |
| --- | --- | --- |
| View | `ui/**/*.slint` | Layout, display, input. No logic, no domain types |
| Adapter | `src/adapters/` | Pure domain-type ↔ Slint-type conversion. No I/O |
| ViewModel | `src/viewmodels/` | UI state, callback impls, facade calls, optimistic updates |

Only ViewModels call `flequit-core` facades. Never call repositories or concrete
infrastructure crates from the UI.

## Global singletons

Five globals only. Do not add more without a design reason.

- `Theme` — colors, spacing, fonts, radii
- `Layout` — breakpoints and size tokens
- `AppState` — selection, expansion, filters, loading, error
- `Capabilities` — platform feature availability
- `Actions` — callbacks implemented in Rust
- `I18n` — locale list and code→text mapping

## Property rules

- Default to `in`. Use `in-out` only where two-way binding is required (text input).
- Express derived values declaratively: `private property <T>: <expr>;`
- Slint `kebab-case` maps to Rust `snake_case` (`is-selected` → `set_is_selected`).
- Slint `struct` cannot express `Option`. Resolve it in the Adapter
  (empty string + a `has-*: bool` flag).

## Model updates

- ViewModel owns `Rc<VecModel<T>>`; the UI receives a `ModelRc`.
- Full reset (`set_vec`) only on initial load and filter change.
- Single-row change: `set_row_data`. Add: `push`/`insert`. Remove: `remove`.
- Lists that may exceed a few dozen rows must use `ListView` (virtual scrolling).

## Callback wiring

```rust
let weak = app.as_weak();
app.global::<Actions>().on_add_task(move |title| {
    let Some(app) = weak.upgrade() else { return };
    // 1. snapshot  2. optimistic update  3. tokio::spawn  4. rollback on failure
});
```

- Capture only `slint::Weak` in closures (strong refs create cycles).
- Return from callbacks immediately; offload work to `tokio::spawn`.

## Thread boundary

Slint components and models are **not `Send`**.

```rust
tokio::spawn(async move {
    let result = facade.add_task(input).await;
    let _ = weak.upgrade_in_event_loop(move |app| { /* only here touch UI */ });
});
```

Log `upgrade` failures rather than swallowing them.

## Optimistic update pattern

1. Snapshot current row (in UI type, via `row_data()`)
2. Update model/property immediately
3. `tokio::spawn` the facade call
4. On failure: `upgrade_in_event_loop` → restore snapshot (match by ID) → record error

Always test the rollback path with a failing mock.

## Responsive layout

- Branch on **window width**, never on target OS.
- Breakpoints: `Compact < 600px`, `Medium 600–1023px`, `Expanded >= 1024px`.
- Use `if` for structural differences, `states` for property/size changes.
- Min tap target 48px in `Compact`. No hover-only affordances.

## Accessibility (mandatory)

Every interactive element needs **both** a role and a default action:

```slint
TouchArea { clicked => { Actions.select-project(project.id); } }
accessible-role: button;
accessible-label: @tr("Sidebar" => "Expand {0}", project.name);
accessible-action-default => { Actions.select-project(project.id); }
```

Without `accessible-action-default` the control is readable but not operable by
keyboard or a screen reader — and unreachable from the interaction tests, which
drive the UI through the accessibility tree.

Never concatenate words into a label; use a placeholder (word order differs
across languages).

## Full-screen overlays swallow input

A full-screen `TouchArea` (the `AppState.loading` veil, the compact sidebar
overlay) blocks every click beneath it. If the flag that hides it can get stuck,
the app looks fine and responds to nothing. Log `upgrade_in_event_loop` errors
instead of discarding them, and confirm every overlay has a path that clears it.

## Anti-patterns

- Business logic in `.slint` (search parsing, recurrence math, overdue rules)
- `set_vec()` for a single-row change
- Hardcoded pixel values instead of `Theme`/`Layout` tokens
- `#[cfg(target_os = ...)]` anywhere in `flequit-ui`
- Blocking I/O on the UI thread
- Global/`static` singletons for ViewModel state
- `accessible-role` without `accessible-action-default`
- Referencing an `id` declared inside an `if` block from outside it (compile error)
- Carrying a Slint type (`TaskItem`, `ModelRc`) into `tokio::spawn` — they are
  `!Send`; snapshot the plain fields instead

## Verification

- `cargo check --quiet` → `cargo clippy --all-targets -- -D warnings`
- `cargo test -p flequit-ui -j 4` (includes `tests/interaction.rs`)
- Preview a single file: `slint-viewer crates/flequit-ui/ui/views/<...>.slint`
- Check layout at 599 / 600 / 1023 / 1024px

Details: `docs/ja/develop/design/ui/` and `docs/ja/develop/rules/ui.md`.
