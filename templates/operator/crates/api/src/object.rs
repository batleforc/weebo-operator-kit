use kube::{CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::status::Status;

/// The short names the rest of the workspace uses.
pub use self::{{product}}Object as Object;
pub use self::{{product}}ObjectSpec as ObjectSpec;

/// An object managed on one {{product}} instance — the example resource of
/// this operator: replace it with the real kinds. Created (or updated) on
/// every reconcile; deleted when the CR is, unless the CR carries
/// `{{product | downcase}}.weebo.io/force-orphan: "true"`.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[kube(
    group = "{{product | downcase}}.weebo.io",
    version = "v1alpha1",
    kind = "{{product}}Object",
    plural = "{{product | downcase}}objects",
    namespaced,
    status = "Status",
    printcolumn = r#"{"name":"Instance","type":"string","jsonPath":".spec.instanceRef"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Reason","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].reason"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct {{product}}ObjectSpec {
    /// `metadata.name` of the `{{product}}Instance` the object lives on.
    pub instance_ref: String,
    /// Name of the remote object. Defaults to the CR's `metadata.name`.
    /// Changing it creates a new remote object and leaves the old one
    /// behind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Free-form description, sent as is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Object {
    /// The remote object's name: `spec.name`, or the CR's own name.
    pub fn remote_name(&self) -> String {
        self.spec.name.clone().unwrap_or_else(|| self.name_any())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_name_defaults_to_the_cr_name() {
        let mut object = Object::new(
            "docs",
            ObjectSpec {
                instance_ref: "main".into(),
                name: None,
                description: None,
            },
        );
        assert_eq!(object.remote_name(), "docs");
        object.spec.name = Some("documentation".into());
        assert_eq!(object.remote_name(), "documentation");
    }

    #[test]
    fn optional_fields_stay_out_of_the_manifest() {
        let spec: ObjectSpec =
            serde_json::from_value(serde_json::json!({"instanceRef": "main"})).unwrap();
        assert_eq!(
            serde_json::to_value(&spec).unwrap(),
            serde_json::json!({"instanceRef": "main"})
        );
    }
}
