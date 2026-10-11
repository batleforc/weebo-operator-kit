//! The CRDs of the operator. Pure types: `serde`/`schemars` derives and
//! the kit's shared field types, no client code. Doc comments become the
//! CRD schema descriptions (`crdgen` → `deploy/crd/`).
//!
//! The kinds' Rust types carry their kind's name (`{{product}}Instance`,
//! the CRD's title); the workspace uses them through short re-exports,
//! [`Instance`] and [`Object`].

pub mod instance;
pub mod object;
pub mod status;

/// API group shared by every CRD of this operator.
pub const GROUP: &str = "{{product | downcase}}.weebo.io";

/// The kit's common annotations under [`GROUP`]:
/// `{{product | downcase}}.weebo.io/force-orphan` (release a CR without
/// cleanup), `allow-disruptive-update`, `rotate-secret`, `import-pending`.
pub const ANNOTATIONS: weebo_kit_api::Annotations = weebo_kit_api::Annotations::new(GROUP);

pub use instance::Instance;
pub use object::Object;
pub use status::Status;
