//! Outbound adapters shared by every weebo operator: the secret stores
//! behind `weebo_kit_application::ports::SecretStore` — Kubernetes
//! `Secret`s, Vault KV v2, and the per-target routing between them — plus
//! reading a key of a `Secret`.
//!
//! What ties them to an operator's instance CRD (which instance, which
//! Vault settings) stays in the operator: it builds a [`VaultConfig`] from
//! its own spec and a [`RoutingSecretStore`] per instance.

pub mod secret_k8s;
pub mod secret_ref;
pub mod secret_routing;
pub mod secret_vault;

pub use secret_k8s::K8sSecretStore;
pub use secret_ref::read_secret_key;
pub use secret_routing::RoutingSecretStore;
pub use secret_vault::{VaultConfig, VaultLogins, VaultSecretStore};
