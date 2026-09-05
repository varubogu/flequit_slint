---
name: platform-abstraction
description: Add or modify OS-dependent behavior in Flequit through the flequit-platform crate, covering capability gating, path resolution, notifications, file dialogs, and mobile lifecycle. Use for any Windows/macOS/Linux/Android/iOS specific work.
---

# Platform Abstraction Skill

Use this skill whenever behavior differs by operating system.

## Hard rule

`#[cfg(target_os = ...)]` and `#[cfg(target_arch = ...)]` live **only inside
`crates/flequit-platform/src/platform/`**. If you need an OS check elsewhere,
the abstraction is missing — add it to `flequit-platform` instead.

CI enforces this via `./scripts/check-crate-deps.sh`.

## Capability gating

Unsupported features are reported through `Capability`, not runtime errors.
Query before calling; hide the UI affordance when unavailable.

| Capability | Desktop | Android | iOS |
| --- | --- | --- | --- |
| `SystemTray` | yes | no | no |
| `GlobalHotkey` | yes | no | no |
| `MultiWindow` | yes | no | no |
| `OsTrash` | yes | no | no |
| `ArbitraryFilePath` | yes | no | no |
| `LocalNotification` | yes | yes | yes |
| `FilePicker` | yes | yes | yes |
| `BackgroundSync` | yes | limited | limited |

`PlatformError::Unsupported` in a normal flow means someone skipped the capability check.

## Paths

Never hardcode paths. Use `flequit_platform::paths`:

- `data_dir()` — SQLite DB, Automerge files
- `config_dir()` — settings
- `cache_dir()` — temporary files
- `log_dir()` — log files

Mobile keeps everything inside the app sandbox.

## File handles, not paths

Mobile file pickers return URIs / security-scoped handles, not paths.
The API returns a `FileHandle` abstraction; read and write through it.
Do not assume `PathBuf` is available for user-selected files.

## Notifications

- Mobile requires runtime permission: call `request_permission()` before first use.
- Handle denial by showing the reason and a re-request path in settings.
- Scheduled notifications register with the OS scheduler.
  Do not assume the process stays alive.

## Lifecycle

| Event | Action |
| --- | --- |
| `Suspend` | Flush unsaved changes, stop sync tasks and timers |
| `Resume` | Reload data, restart sync |
| `LowMemory` | Release Automerge documents for inactive projects |

**Never write save-on-exit logic.** Mobile OSes kill the process without warning.
Persist on every operation instead.

## Adding a new platform capability

1. Define the trait / function in the shared interface first
2. Implement for desktop; `cargo test -j 4 -p flequit-platform`
3. Implement for Android and iOS
4. Update the `Capability` table if any platform lacks support
5. Update `docs/ja/develop/design/platform/platform-abstraction.md`
6. Run `./scripts/check-crate-deps.sh`
7. Cross-compile check:
   - `cargo check --target aarch64-linux-android`
   - `cargo check --target aarch64-apple-ios` (macOS only)

## Testing

- Define traits and inject `MockPlatform` in tests
- Swap path resolution for a `tempfile`-backed implementation
- Run the same test suite against each platform implementation

Details: `docs/ja/develop/design/platform/platform-abstraction.md`.
