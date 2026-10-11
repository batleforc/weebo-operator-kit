use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use weebo_kit_api::SecretKeyRef;

use crate::status::Status;

/// The short names the rest of the workspace uses.
pub use self::{{product}}Instance as Instance;
pub use self::{{product}}InstanceSpec as InstanceSpec;

/// Connection to one {{product}} server. Cluster-scoped: an org-wide piece
/// of infrastructure, not owned by a namespace. `Ready` tells whether the
/// server answers with the token below.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[kube(
    group = "{{product | downcase}}.weebo.io",
    version = "v1alpha1",
    kind = "{{product}}Instance",
    plural = "{{product | downcase}}instances",
    status = "Status",
    printcolumn = r#"{"name":"URL","type":"string","jsonPath":".spec.url"}"#,
    printcolumn = r#"{"name":"Ready","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"Reason","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].reason"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct {{product}}InstanceSpec {
    /// Base URL of the {{product}} server, e.g. `https://example.com`.
    pub url: String,
    /// The API token the operator authenticates with. Read on every
    /// reconcile: rotating it needs no restart.
    pub token_secret_ref: SecretKeyRef,
}
