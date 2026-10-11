//! Port traits: what the use-cases need from the outside world. One
//! [`RemoteGateway`] per `{{product}}Instance`, produced on demand by the
//! [`GatewayFactory`].

use std::sync::Arc;

use domain::error::ReasonCode;

/// A failed call to {{product}}.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    /// No answer: network error, timeout, gateway errors.
    #[error("{{product}} unreachable: {0}")]
    Unreachable(String),
    /// The token was refused.
    #[error("{{product}} refused the token: {0}")]
    Unauthorized(String),
    #[error("not found in {{product}}: {0}")]
    NotFound(String),
    /// Any other unexpected answer.
    #[error("{{product}} API error: {0}")]
    Api(String),
}

impl GatewayError {
    pub fn reason(&self) -> ReasonCode {
        match self {
            GatewayError::Unreachable(_) => ReasonCode::InstanceUnreachable,
            GatewayError::Unauthorized(_) => ReasonCode::InstanceUnauthorized,
            GatewayError::NotFound(_) | GatewayError::Api(_) => ReasonCode::RemoteError,
        }
    }
}

/// The desired state of a remote object, as the gateway sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    pub name: String,
    pub description: Option<String>,
}

/// Talks to one {{product}} instance.
///
/// `#[async_trait]` rather than native `async fn` in trait: it is used as
/// `dyn RemoteGateway`, and native async-fn-in-traits are not
/// `dyn`-compatible.
#[async_trait::async_trait]
pub trait RemoteGateway: Send + Sync {
    /// Whether the instance answers and accepts the token.
    async fn ping(&self) -> Result<(), GatewayError>;
    /// Creates the object, or updates the one of that name; returns its
    /// remote identifier. Idempotent.
    async fn ensure(&self, object: &RemoteObject) -> Result<String, GatewayError>;
    /// Deletes the object `name`; `NotFound` when it is already gone.
    async fn delete(&self, name: &str) -> Result<(), GatewayError>;
}

/// Why no gateway could be built for an instance.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GatewayFactoryError {
    #[error("{{product}}Instance {0:?} not found")]
    InstanceNotFound(String),
    /// The token `Secret` or its key is missing / not UTF-8.
    #[error("token secret: {0}")]
    TokenSecret(String),
    /// A Kubernetes API read failed — usually transient.
    #[error("Kubernetes API error: {0}")]
    Kubernetes(String),
    /// The instance's spec can't produce a client.
    #[error("invalid {{product}}Instance: {0}")]
    InvalidSpec(String),
}

impl GatewayFactoryError {
    pub fn reason(&self) -> ReasonCode {
        match self {
            GatewayFactoryError::InstanceNotFound(_) => ReasonCode::InstanceNotFound,
            GatewayFactoryError::TokenSecret(_) => ReasonCode::TokenSecretMissing,
            GatewayFactoryError::Kubernetes(_) => ReasonCode::KubernetesApiError,
            GatewayFactoryError::InvalidSpec(_) => ReasonCode::InvalidSpec,
        }
    }
}

/// Resolves a `{{product}}Instance` name to a gateway. Implementations
/// re-read the token `Secret` on every call: rotating it needs no restart.
#[async_trait::async_trait]
pub trait GatewayFactory: Send + Sync {
    async fn gateway_for(
        &self,
        instance: &str,
    ) -> Result<Arc<dyn RemoteGateway>, GatewayFactoryError>;
}
