//! The schematic kinds every operator shares (group [`GROUP`]): what lives
//! outside the cluster and several operators point at — an OIDC issuer, a
//! registry mirrored, a Vault server. Declared by the same kit, they are
//! the same kind on every operator's site: the CRD IDE draws one node for
//! the issuer two operators trust. An operator declares them all with
//! [`shared`] and makes them from its fields with `x-weebo-produces`:
//!
//! ```ignore
//! #[schemars(extend("x-weebo-produces" = { "kind": { "group": "schematic.weebo.io", "kind": "UpstreamRegistry" }, "each": "upstreams[]", "name": ["."] }))]
//! pub config: serde_json::Value,
//! ```
//!
//! What only one operator draws stays in its own group
//! (`schematic.<its group>`), with [`crate::links::schematic`].

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::links::{LinkError, Roles, Schematic, Scope, schematic};

/// The group of the shared schematic kinds.
pub const GROUP: &str = "schematic.weebo.io";

/// An OpenID Connect issuer, named by its URL (the `iss` its tokens carry).
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OidcIssuerSpec {
    /// Where its discovery document is, when not
    /// `<issuer>/.well-known/openid-configuration`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_url: Option<String>,
    /// The audiences its tokens are minted for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audiences: Vec<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A package or image registry outside the cluster that the instance
/// mirrors or proxies, named by its URL.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamRegistrySpec {
    /// Its format: `oci`, `npm`, `cargo`, `pypi`…
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Whether pulling from it needs credentials.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub authenticated: bool,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A Vault server the instance writes secrets to, named by its address.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VaultServerSpec {
    /// The Kubernetes auth mount the instances log in with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mount: Option<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An HTTP service outside the cluster that the instance calls — a webhook
/// receiver, a scanner, a telemetry collector — named by its URL.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HttpEndpointSpec {
    /// What it is to the instance: `webhook`, `scanner`, `otlp`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One DNS record of a name.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DnsRecord {
    /// `A`, `CNAME`, `MX`, `TXT`…
    #[serde(rename = "type")]
    pub r#type: String,
    pub value: String,
}

/// A DNS name the instance serves or uses (a domain, a host), named by
/// the name itself.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DnsNameSpec {
    /// The zone that holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// The records it needs, published outside the cluster.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<DnsRecord>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An S3-compatible bucket, named by the bucket.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct S3BucketSpec {
    /// The endpoint's URL; unset, the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A database server the instance uses, outside its pods.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSpec {
    /// `postgres`, `mysql`, `sqlite`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Where it listens (`db.example.com:5432`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Free text, for the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Every shared schematic kind, by name, for the crdgen's `links.json`.
/// What a URL names is cluster-scoped: two namespaces' objects naming one
/// issuer meet on one node.
pub fn shared(roles: &Roles) -> Result<BTreeMap<String, Schematic>, LinkError> {
    let kinds = [
        (
            "OidcIssuer",
            schematic::<OidcIssuerSpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "UpstreamRegistry",
            schematic::<UpstreamRegistrySpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "VaultServer",
            schematic::<VaultServerSpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "HttpEndpoint",
            schematic::<HttpEndpointSpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "DnsName",
            schematic::<DnsNameSpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "S3Bucket",
            schematic::<S3BucketSpec>(GROUP, Scope::Cluster, roles)?,
        ),
        (
            "Database",
            schematic::<DatabaseSpec>(GROUP, Scope::Namespaced, roles)?,
        ),
    ];
    Ok(kinds
        .into_iter()
        .map(|(kind, s)| (kind.to_string(), s))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::GroupKind;

    #[test]
    fn every_shared_kind_has_a_described_schema_in_the_shared_group() {
        let roles = Roles {
            instance: GroupKind::new("example.weebo.io", "ExampleInstance"),
            local_instance: GroupKind::new("example.weebo.io", "ExampleLocalInstance"),
        };
        let kinds = shared(&roles).unwrap();
        assert_eq!(kinds.len(), 7);
        for (kind, s) in &kinds {
            assert_eq!(s.group, GROUP, "{kind}");
            assert!(
                s.schema["description"].is_string(),
                "{kind} has no description"
            );
            assert!(s.schema["properties"]["description"].is_object(), "{kind}");
        }
        assert_eq!(kinds["Database"].scope, Scope::Namespaced);
        assert_eq!(kinds["OidcIssuer"].scope, Scope::Cluster);
    }
}
