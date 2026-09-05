//! Desktop entry point.
//!
//! Android and iOS use their own entry points (`android_main` / a UIKit shim)
//! but converge on [`flequit_app::run`].

fn main() {
    if let Err(error) = flequit_app::run() {
        // Logging may not be initialised yet if the failure happened early.
        eprintln!("flequit failed to start: {error}");
        std::process::exit(1);
    }
}
