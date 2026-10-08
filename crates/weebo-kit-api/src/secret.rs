//! Where emitted secrets go.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SecretStoreBackend {
    #[default]
    Kubernetes,
    Vault,
}

/// Where an emitted secret (generated password or key, OAuth2 credentials,
/// access token) is written. A CR may list several targets — the same
/// value is fanned out to each.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretTarget {
    /// `kubernetes` (default) or `vault`. A Vault target reuses the
    /// connection (address, mount, auth) of the instance's
    /// `secretStore.vault`, which must then be set.
    #[serde(default)]
    pub backend: SecretStoreBackend,
    /// Kubernetes: the `Secret` name. Vault: names the default path
    /// `<pathPrefix>/<namespace>/<name>` when `path` is unset.
    pub name: String,
    /// Kubernetes `Secret` namespace (Vault: the namespace segment of the
    /// default path). Defaults to the CR's own namespace; required on
    /// cluster-scoped CRs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Vault only: explicit KV v2 path under the instance's mount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_defaults_to_kubernetes() {
        let t: SecretTarget = serde_json::from_value(serde_json::json!({"name": "creds"})).unwrap();
        assert_eq!(t.backend, SecretStoreBackend::Kubernetes);
        let v: SecretTarget =
            serde_json::from_value(serde_json::json!({"name": "c", "backend": "vault"})).unwrap();
        assert_eq!(v.backend, SecretStoreBackend::Vault);
    }
}
