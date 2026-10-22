//! CRD field types shared by every weebo operator. Never a whole CRD: the
//! `#[derive(CustomResource)]` types stay in each operator's `api` crate
//! (their group, kind and status differ), and embed these.
//!
//! Doc comments here end up as descriptions in every operator's CRD
//! schema: they talk about "the instance" and "the remote object", never a
//! product name.

pub mod annotations;
pub mod instance;
pub mod links;
pub mod refs;
pub mod schematic;
pub mod secret;
pub mod status;
pub mod tls;

pub use annotations::Annotations;
pub use instance::{
    AttachNamespaces, BoundInstance, InstanceKind, InstanceRef, NamespacesFrom, SharedWith,
    SharesItself,
};
pub use refs::{LocalSecretKeyRef, ObjectRef, SecretKeyRef};
pub use secret::{SecretStoreBackend, SecretTarget, VaultPathGrant};
pub use status::Condition;
pub use tls::{LocalTlsOptions, TlsOptions};
