//! References to Kubernetes objects.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A key inside a Kubernetes `Secret` in an explicit namespace — used by
/// cluster-scoped CRDs, which have no namespace of their own to default to.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretKeyRef {
    pub name: String,
    pub namespace: String,
    pub key: String,
}

/// A key of a `Secret` in the referencing CR's own namespace.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalSecretKeyRef {
    pub name: String,
    pub key: String,
}

/// Reference to another CR of this operator, by `metadata.name`. Always a
/// CR, never a raw remote name: a cluster-scoped target is looked up
/// cluster-wide, a namespaced one in the referencing CR's own namespace only.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRef {
    pub name: String,
}
