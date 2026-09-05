---
name: architecture-review
description: Check Flequit architecture compliance, including UI layer boundaries, crate dependency direction, platform cfg isolation, and thread-boundary rules. Use when reviewing whether code follows the project's structural constraints.
---

# Architecture Review Skill

Use this skill when reviewing whether code follows Flequit architecture constraints.

## Crate dependency direction

Expected order:

```
flequit-types
  ├→ flequit-platform                     (leaf; depends on flequit-types only)
  └→ flequit-model → flequit-repository → flequit-core
       → flequit-infrastructure-* → flequit-infrastructure
         → flequit-ui → flequit-app
```

Checks:

- `flequit-ui` must not depend on `flequit-infrastructure-sqlite`,
  `flequit-infrastructure-automerge`, or `flequit-repository`.
- `flequit-core` must not depend on any infrastructure crate.
- No crate depends on something later in the chain.
- Run `./scripts/check-crate-deps.sh` and `cargo tree -p <crate>` to confirm.

## UI layer checks

- `.slint` files contain no business logic and reference no domain types.
- `.slint` uses `Theme` / `Layout` tokens, not hardcoded pixel values.
- Adapters are pure: no I/O, no facade calls, no global state.
- Only ViewModels register Slint callbacks and call `flequit-core` facades.
- ViewModels do not import repository traits or concrete infrastructure.
- Model updates use row-level notifications; `set_vec` only on load/filter change.
- Lists that can grow use `ListView`.

## Backend checks

- Facades call services; services call repository traits.
- Facades own transaction boundaries; services/repositories never commit or roll back.
- Infrastructure adapters implement persistence only, no domain rules.
- No circular dependencies between facades/services.

## Platform checks

- `#[cfg(target_os = ...)]` / `#[cfg(target_arch = ...)]` appear only inside
  `crates/flequit-platform/src/platform/`.
- No hardcoded filesystem paths; everything goes through `flequit_platform::paths`.
- Unsupported features are gated by `Capability`, not by returning errors at call time.

## Concurrency checks

- No blocking I/O on the UI thread.
- Background→UI updates go through `slint::Weak::upgrade_in_event_loop()`.
- Closures passed to `tokio::spawn` capture `Weak`, not strong references.
- No Tokio runtime construction outside `flequit-app`.
- No save-on-exit logic (mobile processes are killed without notice).

## Responsive checks

- Layout branches on window width, never on target OS or a mobile flag.
- Breakpoint logic lives in `.slint`, not in ViewModels.

## Review output format

1. Violations with concrete file paths.
2. Why each violation is risky.
3. Minimal fix proposal per violation.
