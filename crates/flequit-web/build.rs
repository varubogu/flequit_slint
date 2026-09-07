//! Compiles the same Slint UI as `flequit-ui`, with the same translations.
//!
//! Compiling it a second time rather than depending on `flequit-ui` is
//! deliberate: that crate pulls in `flequit-core` and `flequit-infrastructure`,
//! and therefore `sea-orm`/`sqlx`, none of which build for
//! `wasm32-unknown-unknown`. See the crate documentation for what that costs.

use std::path::Path;

/// The gettext domain the shared `.po` files are written under.
///
/// `slint-build` derives the domain it looks for from `CARGO_PKG_NAME` and
/// offers no way to override it, so the catalogues are restaged under this
/// crate's name before compiling.
const SHARED_DOMAIN: &str = "flequit-ui";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let translations = root.join("../../i18n");
    let ui = root.join("../flequit-ui/ui");

    let staged = stage_translations(&translations);

    let config = slint_build::CompilerConfiguration::new().with_bundled_translations(&staged);

    slint_build::compile_with_config(ui.join("main.slint"), config)
        .expect("failed to compile the shared ui");

    println!("cargo:rerun-if-changed={}", ui.display());
    println!("cargo:rerun-if-changed={}", translations.display());
}

/// Copies `<lang>/LC_MESSAGES/flequit-ui.po` into `OUT_DIR` under this crate's
/// own domain name, and returns the directory to hand to `slint-build`.
fn stage_translations(source: &Path) -> std::path::PathBuf {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let staged = Path::new(&out_dir).join("i18n");
    let domain = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME is set by cargo");

    let entries = std::fs::read_dir(source)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));

    for entry in entries.flatten() {
        let catalogue = entry.path().join("LC_MESSAGES");
        let po = catalogue.join(format!("{SHARED_DOMAIN}.po"));
        if !po.is_file() {
            // `i18n/` also holds the .pot template, which has no language dir.
            continue;
        }

        let target_dir = staged.join(entry.file_name()).join("LC_MESSAGES");
        std::fs::create_dir_all(&target_dir)
            .unwrap_or_else(|error| panic!("could not create {}: {error}", target_dir.display()));
        let target = target_dir.join(format!("{domain}.po"));
        std::fs::copy(&po, &target)
            .unwrap_or_else(|error| panic!("could not copy {}: {error}", po.display()));
    }

    staged
}
