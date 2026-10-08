//! Port traits and use-case helpers shared by every weebo operator. Like
//! each operator's own `application` crate: no concrete adapter, no
//! Kubernetes client — adapters live in `weebo-kit-adapters`.

pub mod emit;
pub mod ports;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
