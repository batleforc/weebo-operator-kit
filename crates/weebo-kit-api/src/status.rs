//! The condition shape every operator's status carries.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    pub type_: String,
    /// `"True"`, `"False"`, or `"Unknown"`.
    pub status: String,
    /// One of the operator's reason codes — never a hand-written string.
    pub reason: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<Time>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}

impl Condition {
    pub const READY: &'static str = "Ready";
}

/// The `Ready` condition of a condition list.
pub fn ready(conditions: &[Condition]) -> Option<&Condition> {
    conditions.iter().find(|c| c.type_ == Condition::READY)
}
