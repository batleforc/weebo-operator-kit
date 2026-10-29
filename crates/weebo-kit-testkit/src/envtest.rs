//! Real `kube-apiserver` + `etcd` through the `envtest` crate (a Rust
//! wrapper over controller-runtime's envtest). The operator's CRDs are
//! installed for real; no container engine is involved.
//!
//! **Build requirement**: `envtest` generates Go bindings at build time
//! (`rust2go`/`bindgen`), which needs Go (from mise) and `libclang.so` (the
//! C API — mise's `clang` doesn't ship it). The Taskfile's `env:` block sets
//! `LIBCLANG_PATH`/`BINDGEN_EXTRA_CLANG_ARGS` for Fedora's
//! `clang-libs`.
//!
//! **Control-plane binaries**: with `KUBEBUILDER_ASSETS` naming a
//! directory that holds `kube-apiserver` and `etcd`, those, and no network
//! call. Otherwise envtest's default: the latest release from its index,
//! downloaded into setup-envtest's cache when missing.
//!
//! **Teardown**: the control plane stops when the [`EnvTestCluster`] is
//! dropped, a failing test's included. The CRDs are installed by the kit,
//! once the control plane runs and is owned here: a CRD the API server
//! refuses stops it too (envtest's own install would leave `kube-apiserver`
//! and `etcd` running). What happens inside envtest's start itself (the
//! default namespace never showing up…) is out of the kit's reach.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use envtest::{BinaryAssetsSettings, Environment};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::Client;
use kube::api::{Api, PostParams};
use serde::Deserialize;

/// How long the CRDs may take to be served once created.
const CRDS_SERVED_TIMEOUT: Duration = Duration::from_secs(30);
const CRDS_SERVED_POLL: Duration = Duration::from_millis(100);

/// The binaries the control plane needs in `KUBEBUILDER_ASSETS`.
const CONTROL_PLANE_BINARIES: [&str; 2] = ["kube-apiserver", "etcd"];

pub struct EnvTestCluster {
    // Dropping the server stops the control plane: kept alive with `self`.
    _server: envtest::Server,
    client: Client,
}

/// Why [`EnvTestCluster::try_start`] failed. The control plane, if it came
/// up, is stopped already.
#[derive(Debug)]
pub struct StartError(String);

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StartError {}

impl EnvTestCluster {
    /// Panics on any setup failure: a broken harness should fail loudly at
    /// the call site rather than be threaded through every test
    /// ([`Self::try_start`] returns it instead).
    ///
    /// `crd_dir`: the directory of the operator's generated CRD manifests
    /// (`deploy/crd/`), or a single manifest.
    pub async fn start(crd_dir: &Path) -> Self {
        Self::try_start(crd_dir)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// [`Self::start`], the failure returned. Whatever fails once the
    /// control plane is up (a CRD refused, never served) stops it before
    /// returning.
    pub async fn try_start(crd_dir: &Path) -> Result<Self, StartError> {
        // `ring` and `aws-lc-rs` are both linked transitively; rustls needs
        // one picked before the first TLS connection. A second install (a
        // second cluster in the same process) just loses the race.
        // Err: another test installed it first, which is as good.
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();

        // Read before anything starts: nothing to stop on a bad manifest.
        let crds = read_crds(crd_dir)?;

        // No CRD given to envtest: its install, failing, would leave the
        // control plane running with no handle to stop it.
        let environment = Environment {
            binary_assets_settings: binary_assets(local_assets()),
            ..Environment::default()
        };
        let server = environment
            .create()
            .await
            .map_err(|e| StartError(format!("envtest control plane failed to start: {e}")))?;
        let client = server
            .client()
            .map_err(|e| StartError(format!("envtest client: {e}")))?;
        // From here on, returning (or panicking) drops `cluster`, which
        // stops the control plane.
        let cluster = Self {
            _server: server,
            client,
        };
        install_crds(&cluster.client, &crds).await?;
        Ok(cluster)
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    /// A client acting as the ServiceAccount `namespace/name` (admin
    /// impersonation): it gets exactly what RBAC grants that account —
    /// the control plane authorizes with RBAC — to prove an operator runs
    /// on the Roles its chart renders.
    pub async fn client_as_service_account(&self, namespace: &str, name: &str) -> Client {
        let mut config = kube::Config::from_custom_kubeconfig(
            self.kubeconfig(),
            &kube::config::KubeConfigOptions::default(),
        )
        .await
        .expect("envtest kubeconfig is usable");
        config.auth_info.impersonate = Some(format!("system:serviceaccount:{namespace}:{name}"));
        config.auth_info.impersonate_groups = Some(vec![
            "system:serviceaccounts".into(),
            format!("system:serviceaccounts:{namespace}"),
            "system:authenticated".into(),
        ]);
        Client::try_from(config).expect("impersonating client")
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

/// `KUBEBUILDER_ASSETS`, when it names a directory holding the control
/// plane's binaries. Set but without them: `None`, with a note (envtest's
/// download then, as when unset).
fn local_assets() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("KUBEBUILDER_ASSETS").filter(|d| !d.is_empty())?);
    if holds_control_plane(&dir) {
        return Some(dir);
    }
    eprintln!(
        "KUBEBUILDER_ASSETS={} holds no {}: envtest downloads its own",
        dir.display(),
        CONTROL_PLANE_BINARIES.join("/"),
    );
    None
}

fn holds_control_plane(dir: &Path) -> bool {
    CONTROL_PLANE_BINARIES
        .iter()
        .all(|binary| dir.join(binary).is_file())
}

/// Local binaries: used as they are, nothing fetched (envtest reads its
/// release index whenever it may download, even with the binaries cached).
/// None: envtest's default.
fn binary_assets(local: Option<PathBuf>) -> BinaryAssetsSettings {
    match local {
        Some(dir) => BinaryAssetsSettings {
            download_binary_assets: false,
            binary_assets_directory: Some(dir.display().to_string()),
            ..BinaryAssetsSettings::default()
        },
        None => BinaryAssetsSettings::default(),
    }
}

/// The CRDs of `path` (a directory: its `.yaml`, `.yml` and `.json` files,
/// not recursively; or one file), like envtest reads them: every document
/// of kind `CustomResourceDefinition`, anything else skipped (`links.json`).
fn read_crds(path: &Path) -> Result<Vec<CustomResourceDefinition>, StartError> {
    let path = path
        .canonicalize()
        .map_err(|e| StartError(format!("{}: {e} — generate the CRDs first", path.display())))?;
    let unreadable = |e: std::io::Error| StartError(format!("{}: {e}", path.display()));
    let files = if path.is_dir() {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&path).map_err(unreadable)? {
            let file = entry.map_err(unreadable)?.path();
            let manifest = file
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "yaml" | "yml" | "json"));
            if manifest && file.is_file() {
                files.push(file);
            }
        }
        files.sort();
        files
    } else {
        vec![path.clone()]
    };
    let mut crds = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .map_err(|e| StartError(format!("{}: {e}", file.display())))?;
        crds.extend(parse_crds(&text).map_err(|e| StartError(format!("{}: {e}", file.display())))?);
    }
    Ok(crds)
}

/// The `CustomResourceDefinition` documents of one manifest.
fn parse_crds(text: &str) -> Result<Vec<CustomResourceDefinition>, String> {
    let mut crds = Vec::new();
    for document in serde_norway::Deserializer::from_str(text) {
        let value = serde_json::Value::deserialize(document).map_err(|e| e.to_string())?;
        if value.get("kind").and_then(|k| k.as_str()) != Some("CustomResourceDefinition") {
            continue;
        }
        crds.push(serde_json::from_value(value).map_err(|e| e.to_string())?);
    }
    Ok(crds)
}

/// Creates `crds`, then waits until discovery serves every served version
/// of each (what envtest waits for).
async fn install_crds(
    client: &Client,
    crds: &[CustomResourceDefinition],
) -> Result<(), StartError> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    for crd in crds {
        let name = crd.metadata.name.as_deref().unwrap_or_default();
        api.create(&PostParams::default(), crd)
            .await
            .map_err(|e| StartError(format!("installing the CRD {name:?}: {e}")))?;
    }
    let deadline = tokio::time::Instant::now().checked_add(CRDS_SERVED_TIMEOUT);
    for crd in crds {
        let plural = &crd.spec.names.plural;
        for version in crd.spec.versions.iter().filter(|v| v.served) {
            let group_version = format!("{}/{}", crd.spec.group, version.name);
            loop {
                let served = client
                    .list_api_group_resources(&group_version)
                    .await
                    .is_ok_and(|list| list.resources.iter().any(|r| &r.name == plural));
                if served {
                    break;
                }
                if deadline.is_none_or(|d| tokio::time::Instant::now() >= d) {
                    return Err(StartError(format!(
                        "the CRD {plural}.{} was not served as {group_version} within {CRDS_SERVED_TIMEOUT:?}",
                        crd.spec.group
                    )));
                }
                tokio::time::sleep(CRDS_SERVED_POLL).await;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_binaries_are_used_without_any_download() {
        let settings = binary_assets(Some(PathBuf::from("/opt/k8s")));
        assert!(!settings.download_binary_assets);
        assert_eq!(
            settings.binary_assets_directory.as_deref(),
            Some("/opt/k8s")
        );
        assert_eq!(settings.download_binary_assets_index_url, None);
        assert!(binary_assets(None).download_binary_assets);
    }

    #[test]
    fn a_directory_without_the_binaries_is_not_local_assets() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!holds_control_plane(dir));
        let dir = std::env::temp_dir().join(format!("weebo-kit-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kube-apiserver"), "").unwrap();
        assert!(!holds_control_plane(&dir));
        std::fs::write(dir.join("etcd"), "").unwrap();
        assert!(holds_control_plane(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_crd_documents_are_read() {
        let crds = parse_crds(
            "apiVersion: apiextensions.k8s.io/v1\nkind: CustomResourceDefinition\n\
             metadata: {name: things.x.weebo.io}\n\
             spec: {group: x.weebo.io, scope: Namespaced,\n  \
             names: {plural: things, kind: Thing}, versions: []}\n\
             ---\napiVersion: v1\nkind: ConfigMap\nmetadata: {name: c}\n",
        )
        .unwrap();
        assert_eq!(crds.len(), 1);
        assert_eq!(crds[0].spec.names.plural, "things");
        assert!(parse_crds(r#"{"links": []}"#).unwrap().is_empty());
        assert!(parse_crds("kind: CustomResourceDefinition\nspec: 3\n").is_err());
    }
}
