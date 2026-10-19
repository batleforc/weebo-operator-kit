//! RFC 0005: no library of the kit enables a `v1_*` feature of
//! k8s-openapi. The Kubernetes version is the application's choice (its
//! `lib.rs`): an operator compiled for another version than the kit's would
//! otherwise hit k8s-openapi's "Both v1_x and v1_y features are enabled".
//! Only a `[dev-dependencies]` may pick one, for the kit's own tests.

use std::fs;
use std::path::{Path, PathBuf};

fn manifests() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = vec![root.join("Cargo.toml")];
    for entry in fs::read_dir(root.join("crates")).expect("crates/ is readable") {
        let manifest = entry.expect("a crates/ entry").path().join("Cargo.toml");
        if manifest.exists() {
            found.push(manifest);
        }
    }
    found
}

/// The lines of `manifest` that enable a version feature outside a
/// `[dev-dependencies]` section, as `file:line`.
fn violations(manifest: &str, name: &str) -> Vec<String> {
    let mut section = String::new();
    let mut out = Vec::new();
    for (i, line) in manifest.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        let in_k8s_openapi = line.starts_with("k8s-openapi")
            || (section.contains("k8s-openapi") && line.starts_with("features"));
        let dev = section.contains("dev-dependencies");
        if in_k8s_openapi && !dev && line.contains("\"v1_") {
            out.push(format!("{name}:{}", i + 1));
        }
    }
    out
}

#[test]
fn no_library_of_the_kit_picks_the_kubernetes_version() {
    let found: Vec<String> = manifests()
        .iter()
        .flat_map(|path| {
            let text = fs::read_to_string(path).expect("a readable Cargo.toml");
            violations(&text, &path.display().to_string())
        })
        .collect();
    assert!(
        found.is_empty(),
        "a `v1_*` feature of k8s-openapi outside [dev-dependencies] (RFC 0005: \
         the operator picks the version): {found:?}"
    );
}

#[test]
fn the_guard_catches_every_form() {
    let workspace =
        "[workspace.dependencies]\nk8s-openapi = { version = \"0.28\", features = [\"v1_36\"] }\n";
    assert_eq!(violations(workspace, "w"), ["w:2"]);
    let table = "[dependencies.k8s-openapi]\nversion = \"0.28\"\nfeatures = [\"v1_34\"]\n";
    assert_eq!(violations(table, "t"), ["t:3"]);
    let dev = "[dev-dependencies]\nk8s-openapi = { workspace = true, features = [\"v1_36\"] }\n";
    assert!(violations(dev, "d").is_empty());
    let plain = "[dependencies]\nk8s-openapi = { workspace = true } # v1_36 in a comment\n";
    assert!(violations(plain, "p").is_empty());
}
