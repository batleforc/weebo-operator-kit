//! Status polling for envtest-backed controller tests (the kit's helpers;
//! each caller picks its timeout and poll interval).

pub use weebo_kit_testkit::polling::{wait_for, wait_for_absence};
