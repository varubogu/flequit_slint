---
name: testing
description: Implement and debug Rust tests for Flequit, covering unit tests, integration tests, ViewModel tests without a Slint window, and platform mocks. Use for any cargo test work in this repository.
---

# Testing Skill

Use this skill for test creation, failure analysis, and regression prevention.
All tests run through `cargo test` — there is no separate JS test stack.

## Command rules

- **Always cap workers**: `cargo test -j 4`. Never omit `-j 4`.
- Start with focused scope, then broaden.

```
cargo test -j 4 <test_name>              # single test
cargo test -p flequit-core -j 4          # one crate
cargo test -p flequit-ui -j 4            # ViewModels and Adapters
cargo test -j 4 --test <integration>     # one integration test
cargo test -j 4                          # everything (last)
```

Prepare fixtures first when a test touches SQLite or Automerge:
`./scripts/test-prepare.sh`

## Workflow

1. Reproduce with the smallest failing scope.
2. Fix root cause (logic, mock, data setup, or async behavior).
3. Re-run the same focused test.
4. Run the wider suite only after the focused test passes.

## ViewModel tests

**Do not create a Slint window.** ViewModels hold `Rc<VecModel<T>>` plus internal
state, so they are testable directly.

Assert on:

- Model row count and field values
- Whether updates used row-level notifications (not `set_vec`)
- Optimistic update applied immediately
- Rollback restores the previous value when the facade fails
- `UiError` recorded on failure

Inject mocks via `AppViewModel::new_for_test(mock_infrastructure, mock_platform)`.

Translation mocks are **not needed** — ViewModels handle identifiers, not text.

## Interaction tests (UI shell)

`crates/flequit-ui/tests/interaction.rs` checks that user actions actually reach
their handlers — a gap unit tests cannot see.

Constraints, all non-negotiable:

- **One `#[test]` function for everything.** Slint's backend is process-global;
  parallel tests fail with "EventLoop can't be recreated".
- **`window.show()` before querying.** Repeated (`for`) rows are not instantiated
  until layout runs.
- **Pin the locale**: `slint::select_bundled_translation("en")` after the first
  component is created, or labels follow the developer's system language.
- **Drive through accessibility**: `ElementHandle::find_by_accessible_label(...)`
  then `invoke_accessible_default_action()`. Mouse injection (`send_mouse_click`)
  lives behind the `internal` feature, which does not build from crates.io.
- Debug info is enabled for debug builds in `build.rs`; the element query API
  requires it.

Because input is injected through the accessibility tree, these tests do **not**
cover hit-testing (an overlay stealing clicks). Verify overlays by construction.

Add a case here whenever a new interactive control is added.

## Test design

- Unit tests: isolate service/business logic and Adapter conversions.
- Integration tests: facade → repository, SQLite/Automerge consistency.
- System tests: realistic user scenarios.
- Assert both the success path and the key failure path.
- Use `#[tokio::test]` for async; avoid real sleeps (`tokio::time::pause()`).
- Create fresh state per test; never share globals or rely on test order.

## External file tests

Write under `.tmp/tests/<kind>/<file>/<case>/<yyyymmdd_hhmmss>/`.
Obtain paths from the platform test implementation, not hardcoded literals.
Do not clean up afterwards — directories are timestamped to avoid collisions.

## Error-handling tests

Suppress expected error logs so passing tests stay quiet, then assert on
behavior (rollback, recorded error), not on log output. Do not add debug
logging inside mocks.

## Platform tests

- Inject `MockPlatform`; swap path resolution for `tempfile`.
- Run the same suite against each platform implementation.
- Cross-compile checks in CI: `cargo check --target aarch64-linux-android`,
  `cargo check --target aarch64-apple-ios`.

## Not covered by automated tests

`.slint` layout. Verify manually at 599 / 600 / 1023 / 1024px in both `en` and `ja`.

Details: `docs/ja/develop/rules/testing.md`, `docs/ja/develop/design/testing.md`.
