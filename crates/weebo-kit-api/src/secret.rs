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
    /// `kubernetes` (default) or `vault`. A Vault target reuses the Vault
    /// connection (address, mount, auth) the instance configures, which it
    /// must then do.
    #[serde(default)]
    pub backend: SecretStoreBackend,
    /// Kubernetes: the `Secret` name. Vault: names the default path
    /// `<pathPrefix>/<namespace>/<name>` when `path` is unset.
    pub name: String,
    /// Kubernetes `Secret` namespace (Vault: the namespace segment of the
    /// default path). Defaults to the CR's own namespace; required on
    /// cluster-scoped CRs. A namespaced CR may only name its own namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Vault only: explicit KV v2 path under the instance's mount. It must
    /// stay under `<pathPrefix>/<namespace>/` (this object's namespace, or
    /// the target's on a cluster-scoped object), unless the instance's
    /// `pathGrants` open it to this object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A path pattern: segments separated by `/`, each a literal, `+` (any one
/// segment) or, last only, `*` (one or more segments).
pub const PATH_PATTERN: &str = r"^(?:(?:[^/*+]+|\+)/)*(?:[^/*+]+|\+|\*)$";

/// Bounds keeping the CEL rules' estimated cost within the apiserver's
/// budget. The field embedding a list of grants must bound it too
/// (`#[schemars(length(max = 32))]`): unbounded, the apiserver refuses the
/// CRD.
const MAX_PATHS: u32 = 64;
const MAX_PATH_LENGTH: u32 = 512;
const MAX_NAMESPACES: u32 = 64;

/// Paths of the instance's secret backend that objects may write to
/// outside their namespace's subtree `<pathPrefix>/<namespace>/`. Opens
/// them to every subject allowed to create an emitting object in a
/// covered namespace, and cannot hand over a value another object owns.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("x-kubernetes-validations" = [{
    "rule": "(has(self.namespaces) && size(self.namespaces) > 0) != (has(self.clusterScoped) && self.clusterScoped)",
    "message": "a grant names namespaces or is clusterScoped, not both"
}]))]
pub struct VaultPathGrant {
    /// Namespaces whose objects may write `paths`: exact names, `*`, or a
    /// prefix glob such as `team-*`. `team-*` opens the paths to every
    /// matching namespace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(schema_with = "namespace_patterns")]
    pub namespaces: Vec<String>,
    /// The grant opens `paths` to cluster-scoped objects (instead of
    /// `namespaces`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cluster_scoped: bool,
    /// Path patterns under the mount, compared segment by segment: a
    /// literal segment matches itself, `+` exactly one segment, and a last
    /// `*` one or more. Each holds at least one literal segment, e.g.
    /// `shared/auth`, `+/auth`, `platform/*`. Prefer exact paths where
    /// values not written by the operator live: a covered object may take
    /// over a value no object owns yet.
    #[schemars(schema_with = "path_patterns")]
    pub paths: Vec<String>,
}

fn namespace_patterns(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "maxItems": MAX_NAMESPACES,
        "items": {"type": "string", "minLength": 1, "maxLength": 64},
    })
}

fn path_patterns(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "minItems": 1,
        "maxItems": MAX_PATHS,
        "items": {
            "type": "string",
            "maxLength": MAX_PATH_LENGTH,
            "pattern": PATH_PATTERN,
            "x-kubernetes-validations": [
                {
                    "rule": r"!self.matches('(^|/)\\.\\.?(/|$)')",
                    "message": "a path pattern has no \".\" or \"..\" segment",
                },
                {
                    "rule": "self.matches('(^|/)[^/*+]+(/|$)')",
                    "message": "a path pattern has at least one literal segment",
                },
            ],
        },
    })
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

    #[test]
    fn the_grant_schema_carries_its_bounds_and_rules() {
        let schema = serde_json::to_value(schemars::schema_for!(VaultPathGrant)).unwrap();
        let paths = &schema["properties"]["paths"];
        assert_eq!(paths["minItems"], 1);
        assert_eq!(paths["items"]["pattern"], PATH_PATTERN);
        assert_eq!(
            paths["items"]["x-kubernetes-validations"][1]["rule"],
            "self.matches('(^|/)[^/*+]+(/|$)')"
        );
        assert!(
            paths["description"]
                .as_str()
                .unwrap()
                .contains("segment by segment")
        );
        assert_eq!(schema["required"], serde_json::json!(["paths"]));
        assert_eq!(
            schema["x-kubernetes-validations"][0]["message"],
            "a grant names namespaces or is clusterScoped, not both"
        );
    }
}
