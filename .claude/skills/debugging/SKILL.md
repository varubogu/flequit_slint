---
name: debugging
description: Debug Flequit UI and core issues by reproducing reliably, narrowing root causes, and validating fixes. Covers Slint property/binding problems, thread-boundary bugs, and platform-specific failures.
---

# Debugging Skill

Use this skill for compile errors, runtime failures, failed tests, and UI issues.

## Workflow

1. Reproduce the issue with a minimal command/scope.
2. Collect concrete signals (error messages, logs, failing assertions).
3. Narrow to one root cause hypothesis at a time.
4. Apply the smallest fix and verify.
5. Run adjacent checks to avoid regressions.

## General checks

- Build/type check: `cargo check --quiet`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Tests: `cargo test -j 4 <name>`
- Dependency/cfg guard: `./scripts/check-crate-deps.sh`
- Run with logs: `RUST_LOG=debug cargo run -p flequit-app`

## Slint UI symptoms

| Symptom | Likely cause |
| --- | --- |
| Property never updates | Rust setter not called, or value set on a different instance |
| Derived value stale | Computed in Rust and pushed, instead of declared as a binding |
| List flickers / scroll jumps | `set_vec()` used for a single-row change |
| Panic: already borrowed | `RefCell::borrow_mut()` held across a callback that re-enters |
| Panic / no effect from a thread | UI touched outside `upgrade_in_event_loop()` |
| Callback never fires | Callback declared in `.slint` but not registered in Rust |
| Layout wrong at some size | Breakpoint condition, or hardcoded pixel value |

Isolate a single file with `slint-viewer crates/flequit-ui/ui/<path>.slint`
to separate layout problems from data problems.

## Thread-boundary bugs

Slint components and models are not `Send`. If a change works in tests but fails
at runtime, check that every background→UI update goes through
`slint::Weak::upgrade_in_event_loop()`, and that closures capture `Weak` only.

## Core / data symptoms

- Compile errors: read the full compiler message; resolve incrementally with `cargo check`
- Data not persisted: confirm the facade committed the transaction
- SQLite/Automerge divergence: writes must go SQLite first, then Automerge
- Test DB conflicts: confirm `-j 4` and that fixtures were prepared

## Platform-specific failures

- Reproduce on desktop first; most logic bugs are platform independent
- Path issues: confirm `flequit_platform::paths` is used, not a literal path
- Missing feature on mobile: check the `Capability` table before assuming a bug
- Android logs: `adb logcat`; iOS: Console.app / OSLog

## Escalation

If the root cause is unclear after two hypotheses, state what was ruled out and
ask the user rather than making speculative changes.
