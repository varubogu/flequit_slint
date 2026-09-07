import org.gradle.internal.os.OperatingSystem

plugins {
    id("com.android.application")
}

// ABIs the APK ships. arm64 covers every device sold since 2017; x86_64 is
// there so the emulator works on CI and on desktop machines.
val abis = listOf("arm64-v8a" to "aarch64-linux-android", "x86_64" to "x86_64-linux-android")

android {
    namespace = "com.flequit.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.flequit.app"
        // 26: `NotificationChannel` and `Notification.Builder(Context, String)`,
        // both of which the platform layer uses unconditionally.
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += abis.map { it.first }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // cargo-ndk writes the shared objects straight into the packaged layout.
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")
}

/**
 * Builds the Rust cdylib for every packaged ABI.
 *
 * Uses cargo-ndk rather than raw cargo so the NDK's linker, sysroot and API
 * level are configured consistently; `cargo build --target` alone leaves the
 * linker unset and fails at the first C dependency (SQLite).
 */
val cargoNdk = tasks.register<Exec>("cargoNdkBuild") {
    group = "build"
    description = "Compiles flequit-app into src/main/jniLibs for each ABI."

    val repoRoot = rootProject.projectDir.parentFile.parentFile
    workingDir = repoRoot

    val command = mutableListOf(
        if (OperatingSystem.current().isWindows) "cargo-ndk.exe" else "cargo-ndk",
        "--output-dir", file("src/main/jniLibs").absolutePath,
        "--platform", "26",
    )
    abis.forEach { (abi, _) -> command += listOf("--target", abi) }
    command += listOf("build", "--release", "-p", "flequit-app", "--features", "android")

    commandLine(command)
}

tasks.named("preBuild") { dependsOn(cargoNdk) }
