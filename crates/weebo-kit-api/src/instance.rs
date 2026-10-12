//! Which instance a CR lives on: the `instanceRef` a CR writes, the
//! instance its status records once resolved, and what a namespaced
//! instance shares with other namespaces.
//!
//! An operator has two instance kinds: the cluster-scoped instance
//! (`<Product>Instance`) and the namespaced local instance
//! (`<Product>LocalInstance`). The values here name them by their role,
//! `Instance` and `LocalInstance`, never by the product.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The kind of instance a reference designates.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, JsonSchema, PartialEq, Eq, Hash)]
pub enum InstanceKind {
    /// The cluster-scoped instance kind.
    Instance,
    /// The namespaced instance kind.
    LocalInstance,
}

impl InstanceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InstanceKind::Instance => "Instance",
            InstanceKind::LocalInstance => "LocalInstance",
        }
    }
}

/// The instance this object lives on. Without `kind`, the instance is
/// found by name: a cluster-wide operator looks for a cluster-scoped
/// instance first, then for a local instance in this object's namespace;
/// an operator limited to one namespace only uses local instances of the
/// object's namespace.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("x-kubernetes-validations" = [{
    "rule": "!has(self.namespace) || !has(self.kind) || self.kind == 'LocalInstance'",
    "message": "namespace only applies to kind LocalInstance"
}]))]
pub struct InstanceRef {
    /// `metadata.name` of the instance.
    pub name: String,
    /// `Instance` (cluster-scoped) or `LocalInstance` (namespaced). Setting
    /// it pins the instance: a cluster-scoped instance created later under
    /// the same name never takes over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<InstanceKind>,
    /// Namespace of a `LocalInstance`. Defaults to this object's own;
    /// required on a cluster-scoped object. Another namespace's local
    /// instance must share itself with this one (its `sharedWith`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl InstanceRef {
    /// A reference by name only.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: None,
            namespace: None,
        }
    }
}

/// The instance an object was resolved to, as its status records it.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct BoundInstance {
    /// `Instance` or `LocalInstance`.
    pub kind: InstanceKind,
    /// The local instance's namespace; absent for a cluster-scoped one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub name: String,
}

impl BoundInstance {
    pub fn cluster(name: impl Into<String>) -> Self {
        Self {
            kind: InstanceKind::Instance,
            namespace: None,
            name: name.into(),
        }
    }

    pub fn local(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: InstanceKind::LocalInstance,
            namespace: Some(namespace.into()),
            name: name.into(),
        }
    }

    /// Whether the namespace policy governs the objects bound to it: only
    /// a cluster-scoped instance is shared by namespaces that did not
    /// provide its credentials. A local instance's own token bounds what
    /// its objects can do.
    pub fn governed_by_policy(&self) -> bool {
        self.kind == InstanceKind::Instance
    }
}

impl std::fmt::Display for BoundInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{} {ns}/{}", self.kind.as_str(), self.name),
            None => write!(f, "{} {}", self.kind.as_str(), self.name),
        }
    }
}

/// The local instance kind: what it shares (`spec.sharedWith`).
pub trait SharesItself {
    fn shared_with(&self) -> Option<&SharedWith>;
}

/// Who else may use a local instance — and its credentials. Unset: only
/// the objects of its own namespace.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SharedWith {
    /// Namespaces whose objects may use this instance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<String>,
    /// Namespaces whose labels match this selector may use this instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<LabelSelector>,
    /// Cluster-scoped objects may use this instance.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cluster_scoped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_only_ref_serializes_as_a_name_only_object() {
        assert_eq!(
            serde_json::to_value(InstanceRef::named("main")).unwrap(),
            serde_json::json!({"name": "main"})
        );
        let full: InstanceRef = serde_json::from_value(
            serde_json::json!({"name": "main", "kind": "LocalInstance", "namespace": "team-a"}),
        )
        .unwrap();
        assert_eq!(full.kind, Some(InstanceKind::LocalInstance));
    }

    #[test]
    fn the_ref_schema_carries_its_cel_rule() {
        let schema = serde_json::to_value(schemars::schema_for!(InstanceRef)).unwrap();
        assert_eq!(
            schema["x-kubernetes-validations"][0]["message"],
            "namespace only applies to kind LocalInstance"
        );
    }

    #[test]
    fn only_cluster_instances_are_governed_by_the_policy() {
        assert!(BoundInstance::cluster("main").governed_by_policy());
        assert!(!BoundInstance::local("team-a", "main").governed_by_policy());
        assert_eq!(
            BoundInstance::local("team-a", "main").to_string(),
            "LocalInstance team-a/main"
        );
    }
}
