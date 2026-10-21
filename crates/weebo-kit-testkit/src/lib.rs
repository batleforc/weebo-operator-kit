//! Test harness shared by every weebo operator.

// A test harness, not production code: its only callers are tests, and a
// helper that cannot reach the control plane or whose status never comes
// must fail the test right there, with the reason — returning a `Result`
// every test then unwraps would only move the panic. So, unlike the other
// crates (each panic site justified on its own), the panic lints are lifted
// for the whole crate; `#[expect]`, so the lift goes once nothing panics.
// The error-handling lints (`let _ =`, …) still apply.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: a panic is a failed test, with its message"
)]

pub mod envtest;
pub mod polling;
