//! Real `kube-apiserver` + `etcd` (the kit's `EnvTestCluster`) with this
//! operator's CRDs from `deploy/crd/` installed.
//!
//! Building it needs Go and `libclang.so` — see `task doctor` and the
//! Taskfile's `env:` block. The control plane: with `KUBEBUILDER_ASSETS`
//! naming a directory that holds `kube-apiserver` and `etcd`, those, and
//! no network call; otherwise envtest's latest release, downloaded into
//! setup-envtest's cache when missing. It stops when the cluster is
//! dropped, and when the start fails once it is up (a CRD refused).

use std::path::Path;

pub use weebo_kit_testkit::envtest::EnvTestCluster;

/// Starts a control plane with every CRD of `deploy/crd/` installed
/// (`task recu` regenerates them).
pub async fn start() -> EnvTestCluster {
    EnvTestCluster::start(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/crd")).await
}
