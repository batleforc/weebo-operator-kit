//! One `reconcile_*` use-case per CRD. Each returns an [`Outcome`] the
//! controller turns into a status patch and Events — always through a
//! `ReasonCode`, never a free string.

pub mod reconcile_instance;
pub mod reconcile_object;

use domain::Failure;
use domain::error::ReasonCode;

use crate::ports::{GatewayError, GatewayFactoryError};

impl From<GatewayError> for Failure {
    fn from(err: GatewayError) -> Self {
        Failure::new(err.reason(), err.to_string())
    }
}

impl From<GatewayFactoryError> for Failure {
    fn from(err: GatewayFactoryError) -> Self {
        Failure::new(err.reason(), err.to_string())
    }
}

/// What a reconcile ended on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The remote object this CR now owns, to record in its status.
    pub remote_id: Option<String>,
    /// `Ok(message)` for `Ready: True`.
    pub ready: Result<String, Failure>,
}

impl Outcome {
    pub fn synced(remote_id: Option<String>, message: impl Into<String>) -> Self {
        Self {
            remote_id,
            ready: Ok(message.into()),
        }
    }

    pub fn failed(failure: impl Into<Failure>) -> Self {
        Self {
            remote_id: None,
            ready: Err(failure.into()),
        }
    }

    /// The reason `Ready` ends on — what metrics and Events are keyed by.
    pub fn reason(&self) -> ReasonCode {
        match &self.ready {
            Ok(_) => ReasonCode::Reconciled,
            Err(f) => f.reason,
        }
    }
}
