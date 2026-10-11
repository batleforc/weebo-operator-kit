//! Inbound adapters: the controllers (Kubernetes watches → use-cases →
//! status) and the HTTPS router served next to them.

pub mod controller;
pub mod webhook;
