# Android build

Produces `com.flequit.app` from the same Rust sources as the desktop app.
Gradle is only here to compile `FlequitActivity.java` and package the APK; all
of the application code is the `flequit-app` cdylib.

## Prerequisites

- Android SDK (platform 35, build-tools) and NDK r26+
- `ANDROID_HOME` and `ANDROID_NDK_HOME` set
- `rustup target add aarch64-linux-android x86_64-linux-android`
- `cargo install cargo-ndk`

## Build

```sh
cd mobile/android
./gradlew assembleDebug        # or assembleRelease
```

`preBuild` runs `cargo-ndk`, which writes `libflequit_app.so` into
`app/src/main/jniLibs/<abi>/` for every ABI listed in `app/build.gradle.kts`.

## Run

```sh
./gradlew installDebug
adb shell am start -n com.flequit.app/.FlequitActivity
adb logcat -s flequit          # the tag the platform log sink writes under
```

## What lives where

| Concern | Location |
| --- | --- |
| Entry point (`android_main`) | `crates/flequit-app/src/entry_android.rs` |
| Paths, notifications, intents | `crates/flequit-platform/src/platform/android.rs` |
| SAF picker + lifecycle glue | `app/src/main/java/com/flequit/app/FlequitActivity.java` |

`FlequitActivity` is the only Java in the project, and it exists because the
Storage Access Framework and the activity lifecycle report their results to the
`Activity` rather than to the caller. It forwards both across JNI to the
`Java_com_flequit_app_FlequitActivity_*` functions in `android.rs`.

## Known gaps

- The launcher icon is a framework placeholder.
- Scheduled reminders use an in-process timer, so one whose time arrives after
  Android has killed the process is lost. `AlarmManager` would fix this and
  needs a `BroadcastReceiver` here.
- Nothing in this directory has been run on a device yet; see `plans/plan.md`.
