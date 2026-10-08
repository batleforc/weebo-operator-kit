//! Real `kube-apiserver` + `etcd` through the `envtest` crate (a Rust
//! wrapper over controller-runtime's envtest). The operator's CRDs are
//! installed for real; no container engine is involved.
//!
//! **Build requirement**: `envtest` generates Go bindings at build time
//! (`rust2go`/`bindgen`), which needs Go (from mise) and `libclang.so` (the
//! C API — mise's `clang` doesn't ship it). The Taskfile's `env:` block sets
//! `LIBCLANG_PATH`/`BINDGEN_EXTRA_CLANG_ARGS` for Fedora's
//! `clang-libs`. The control-plane binaries (`etcd`, `kube-apiserver`) come
//! from mise as well.

use std::path::Path;

use envtest::Environment;
use kube::Client;

pub struct EnvTestCluster {
    // Dropping the server stops the control plane: kept alive with `self`.
    _server: envtest::Server,
    client: Client,
}

impl EnvTestCluster {
    /// Panics on any setup failure: a broken harness should fail loudly at
    /// the call site rather than be threaded through every test.
    ///
    /// `crd_dir`: the directory of the operator's generated CRD manifests
    /// (`deploy/crd/`).
    pub async fn start(crd_dir: &Path) -> Self {
        // `ring` and `aws-lc-rs` are both linked transitively; rustls needs
        // one picked before the first TLS connection. A second install (a
        // second cluster in the same process) just loses the race.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let crd_dir = crd_dir
            .canonicalize()
            .unwrap_or_else(|e| panic!("{}: {e} — generate the CRDs first", crd_dir.display()));

        let server = Environment::default()
            .with_crds_from_paths(vec![crd_dir.display().to_string()])
            .expect("the CRD directory must be readable")
            .create()
            .await
            .expect("envtest control plane failed to start");
        let client = server.client().expect("envtest client");
        Self {
            _server: server,
            client,
        }
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// Admin kubeconfig of the control plane: to run a real binary against
    /// it (`KUBECONFIG`), or to read its URL and CA (Vault's Kubernetes auth).
    pub fn kubeconfig(&self) -> kube::config::Kubeconfig {
        self._server.kubeconfig().expect("envtest kubeconfig")
    }

    /// Writes [`Self::kubeconfig`] as YAML to `path`.
    pub fn write_kubeconfig(&self, path: &Path) {
        let yaml = serde_norway::to_string(&self.kubeconfig()).expect("kubeconfig serializes");
        std::fs::write(path, yaml).expect("kubeconfig written");
    }
}
