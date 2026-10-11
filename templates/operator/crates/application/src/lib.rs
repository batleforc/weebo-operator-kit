//! Use-cases and the ports they need. No concrete adapter and no
//! Kubernetes client here: adapters implement [`ports`] in
//! `adapters-outbound`, `operator::main` injects them.

pub mod ports;
pub mod use_cases;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
