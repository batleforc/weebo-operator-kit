//! Outbound adapters shared by every weebo operator: the secret stores
//! behind `weebo_kit_application::ports::SecretStore` — Kubernetes
//! `Secret`s, Vault KV v2, and the per-target routing between them — plus
//! reading a key of a `Secret`, resolving the operator's instance CRs
//! (cluster-scoped and local), adopting existing remote objects (the
//! importer's flow) and migrating the CRs written before `instanceRef`
//! became an object.
//!
//! What ties them to an operator's instance CRD (which instance, which
//! Vault settings) stays in the operator: it builds a [`VaultConfig`] from
//! its own spec and a [`RoutingSecretStore`] per instance.

pub mod adopt;
pub mod instance;
pub mod instances;
pub mod migrate;
pub mod secret_k8s;
pub mod secret_ref;
pub mod secret_routing;
pub mod secret_vault;

pub use adopt::{Adopter, Adoption, Summary};
pub use instance::{InstanceResolver, SelectError, select};
pub use instances::{Instances, Resolved, SharesItself};
pub use secret_k8s::{K8sSecretStore, MANAGED_BY_LABEL};
pub use secret_ref::{TokenSecretError, read_local_instance_token, read_secret_key};
pub use secret_routing::{RoutingSecretStore, VaultResolver};
pub use secret_vault::{VaultConfig, VaultLogins, VaultSecretStore};
