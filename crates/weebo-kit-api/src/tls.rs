//! TLS verification of an instance's API connection.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::refs::SecretKeyRef;

/// How the operator verifies the instance's TLS certificate. This covers
/// the **API** connection only — a Vault connection has its own
/// `secretStore.vault.caSecretRef`, since the two endpoints are routinely
/// signed by different CAs.
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TlsOptions {
    /// Skip certificate verification entirely — insecure, intended only for
    /// throwaway/self-signed instances. Prefer `caSecretRef` below, which
    /// trusts a private CA without turning verification off.
    #[serde(default)]
    pub insecure_skip_verify: bool,
    /// Optional custom CA certificate to verify the instance's TLS with,
    /// read from a Kubernetes `Secret` (the referenced `key` must hold a PEM
    /// bundle). Use this to trust a private CA without mounting it into the
    /// operator pod — the operator reads the Secret via the API and hands the
    /// PEM to its HTTP client. It is added *on top of* the platform trust
    /// store, so publicly-signed endpoints keep working. When unset, only the
    /// platform trust store (plus the usual `SSL_CERT_FILE`/`SSL_CERT_DIR`
    /// env vars) is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_secret_ref: Option<SecretKeyRef>,
}
