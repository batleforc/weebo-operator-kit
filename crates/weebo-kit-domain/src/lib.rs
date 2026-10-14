//! Pure building blocks shared by every weebo operator. Zero `kube`/`http`
//! dependency, like each operator's own `domain` crate.

pub mod allow_list;
pub mod failure;
pub mod reason;
pub mod vault_paths;

pub use failure::{Advisory, Failure};
pub use reason::{Reason, Severity};
