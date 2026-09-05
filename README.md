# Flequit (Slint)

A native, local-first task management application built with **Rust + [Slint](https://slint.dev)**.

This is a port of [varubogu/flequit](https://github.com/varubogu/flequit) (SvelteKit + Tauri)
to a single-process Rust application with no WebView and no IPC.

日本語版: [README.ja.md](./README.ja.md)

## Status

Early development. The domain, persistence and platform layers are in place and the
UI shell runs; feature parity with the original is in progress.

| Area | State |
| --- | --- |
| Domain / persistence (SQLite + Automerge) | Ported from the original |
| Platform abstraction (`flequit-platform`) | Desktop implemented, mobile stubbed |
| Slint UI shell (sidebar / list / detail, responsive) | Implemented and wired |
| Add task, complete, edit title/notes, subtask toggle | Implemented |
| Delete task | Blocked: the facade leaks `sea_orm` types to callers |
| Search keywords (`@today`, `#tag`), recurrence, settings screen | Not started |

## Platforms

| Platform | Phase | Status |
| --- | --- | --- |
| Windows / macOS / Linux | 1 | In development |
| Android / iOS | 2 | Structure in place, implementation pending |
| Web | — | Out of scope |

The UI is a single codebase; layout switches on **window width**, not on the target OS,
so a narrow desktop window behaves exactly like a phone.

## Requirements

- Rust (see [`rust-toolchain.toml`](./rust-toolchain.toml); [mise](https://mise.jdx.dev/) users get it automatically)
- Linux additionally needs: `libxkbcommon-x11-0`, `libwayland-dev`, `libfontconfig1-dev`, `libdbus-1-dev`

There is **no Node.js / Bun / npm dependency**.

## Getting started

```sh
# Build and run
cargo run -p flequit-app

# With logging
RUST_LOG=debug cargo run -p flequit-app
```

## Development

```sh
cargo check --quiet                              # type check
cargo clippy --all-targets -- -D warnings        # lint
cargo fmt --all                                  # format
./scripts/test-prepare.sh                        # prepare test fixtures (once)
cargo test -j 4                                  # test (-j 4 is required)
./scripts/check-crate-deps.sh                    # architecture invariants
```

Full command reference: [`docs/ja/develop/commands.md`](./docs/ja/develop/commands.md)

## Architecture

```text
Slint UI (.slint)
    ↕  generated Rust bindings
ViewModel          crates/flequit-ui/src/viewmodels/
    ↓
Facade / Service   crates/flequit-core/
    ↓
Repository trait   crates/flequit-repository/
    ↓
SQLite / Automerge crates/flequit-infrastructure-*/

flequit-platform   OS abstraction (paths, notifications, dialogs, capabilities)
```

Invariants enforced by `./scripts/check-crate-deps.sh`:

- Crate dependencies flow in one direction only
- `#[cfg(target_os = ...)]` exists only inside `flequit-platform`
- `.slint` holds no business logic and never sees domain types
- Only ViewModels call `flequit-core` facades
- No hardcoded filesystem paths

## Roadmap

Remaining work, priorities and settled design decisions: [`plans/plan.md`](./plans/plan.md)

## Documentation

Japanese is the source of truth: [`docs/ja/`](./docs/ja/)

- [Architecture](./docs/ja/develop/design/architecture.md)
- [Tech stack and crate layout](./docs/ja/develop/design/tech-stack.md)
- [UI layers](./docs/ja/develop/design/ui/layers.md)
- [Responsive layout](./docs/ja/develop/design/ui/responsive-layout.md)
- [Platform abstraction](./docs/ja/develop/design/platform/platform-abstraction.md)
- [Development workflow](./docs/ja/develop/rules/workflow.md)

## License

MIT
