# iOS build

Produces `com.flequit.app` from the same Rust sources as the desktop app.
Xcode is here to compile the Swift bridge, sign, and package; all of the
application code is the `flequit-app` static library.

Slint's iOS support runs on the winit backend and was a tech preview as of
Slint 1.12. Nothing in this directory has been built or run yet — see
"Status" below.

## Prerequisites

- macOS with Xcode 15+
- `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- `brew install xcodegen`

## Build

```sh
cd mobile/ios
xcodegen generate     # writes Flequit.xcodeproj, which is not checked in
open Flequit.xcodeproj
```

The target's pre-build script runs `cargo build -p flequit-app --features ios`
for the matching triple, so building from Xcode is enough; there is no separate
cargo step.

## Who owns `main`

winit's iOS backend calls `UIApplicationMain` itself from `EventLoop::run_app`,
so the app must not have a Swift `@main` type or an app delegate — there would
be two owners of the run loop.

Instead `Sources/FlequitMain.swift` provides the C `main`: it installs the
lifecycle observers and then calls `flequit_ios_main`, which does not return.

## What lives where

| Concern | Location |
| --- | --- |
| Entry point (`flequit_ios_main`) | `crates/flequit-app/src/entry_ios.rs` |
| Paths, capabilities, FFI declarations | `crates/flequit-platform/src/platform/ios.rs` |
| UIKit: notifications, picker, URLs, lifecycle | `Sources/FlequitBridge.swift` |

The container paths are resolved in Rust from `HOME`, which iOS points at the
app container, so they work without the bridge. Everything else in `ios.rs` is
an `extern "C"` declaration whose implementation is a `@_cdecl` function in
`FlequitBridge.swift`; the two files have to be changed together.

## Status

Untested. The Rust side compiles for `aarch64-apple-ios` in CI, but no part of
this has been linked against the Swift bridge or run on a device or simulator,
because the project has no macOS machine. Expect the first build to need
fixing. See `plans/plan.md` section 6.
