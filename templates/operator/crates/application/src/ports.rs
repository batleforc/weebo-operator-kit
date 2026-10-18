//! Port traits: what the use-cases need from the outside world. One
//! [`RemoteGateway`] per instance (`{{product}}Instance` or
//! `{{product}}LocalInstance`), produced on demand by the
//! [`GatewayFactory`]; which instance a CR lives on is the kit's
//! [`InstanceDirectory`].

use std::sync::Arc;

use domain::error::ReasonCode;
pub use weebo_kit_application::instances::{
    BoundInstance, InstanceDirectory, InstanceError, ResolveError,
};

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

/// A remote object: the desired state the gateway creates, or the one it
/// reads back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    pub name: String,
    /// Desired: `None` is no opinion, the remote value is left alone.
    pub description: Option<String>,
}

/// A remote object as read: its identifier and its current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub id: String,
    pub object: RemoteObject,
}

/// The fields of an update: `None` is not sent. Built from the desired
/// state with every field already holding its value removed
/// (`domain::Drift`), so an update only carries what drifted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectPatch {
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
    /// The object `name`, `None` when there is none.
    async fn read(&self, name: &str) -> Result<Option<Remote>, GatewayError>;
    /// Creates the object; returns its remote identifier.
    async fn create(&self, object: &RemoteObject) -> Result<String, GatewayError>;
    /// Sends the fields of `patch` (only those) to the object `name`.
    async fn update(&self, name: &str, patch: &ObjectPatch) -> Result<(), GatewayError>;
    /// Deletes the object `name`; `NotFound` when it is already gone.
    async fn delete(&self, name: &str) -> Result<(), GatewayError>;
}

/// Why no gateway could be built for an instance.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GatewayFactoryError {
    /// The instance is gone since it was resolved.
    #[error("{0} not found")]
    InstanceNotFound(BoundInstance),
    /// The token `Secret` or its key is missing / not UTF-8.
    #[error("token secret: {0}")]
    TokenSecret(String),
    /// A local instance's token `Secret` doesn't carry the consent label.
    #[error("{0}")]
    TokenNotAllowed(String),
    /// A Kubernetes API read failed — usually transient.
    #[error("Kubernetes API error: {0}")]
    Kubernetes(String),
    /// The instance's spec can't produce a client.
    #[error("invalid instance: {0}")]
    InvalidSpec(String),
}

impl GatewayFactoryError {
    pub fn reason(&self) -> ReasonCode {
        match self {
            GatewayFactoryError::InstanceNotFound(_) => ReasonCode::InstanceNotFound,
            GatewayFactoryError::TokenSecret(_) => ReasonCode::TokenSecretMissing,
            GatewayFactoryError::TokenNotAllowed(_) => ReasonCode::TokenSecretNotAllowed,
            GatewayFactoryError::Kubernetes(_) => ReasonCode::KubernetesApiError,
            GatewayFactoryError::InvalidSpec(_) => ReasonCode::InvalidSpec,
        }
    }
}

/// The gateway of a resolved instance. Implementations re-read the token
/// `Secret` on every call: rotating it needs no restart.
#[async_trait::async_trait]
pub trait GatewayFactory: Send + Sync {
    async fn gateway_for(
        &self,
        instance: &BoundInstance,
    ) -> Result<Arc<dyn RemoteGateway>, GatewayFactoryError>;
}

/// The reason code of an instance that couldn't be resolved.
pub fn instance_reason(err: &InstanceError) -> ReasonCode {
    match err {
        InstanceError::Kubernetes(_) => ReasonCode::KubernetesApiError,
        InstanceError::Resolve(e) => match e {
            ResolveError::NotFound { .. } | ResolveError::NoInstance => {
                ReasonCode::InstanceNotFound
            }
            ResolveError::Ambiguous { .. } => ReasonCode::InstanceAmbiguous,
            ResolveError::InvalidRef(_) => ReasonCode::InvalidSpec,
            ResolveError::OutsideScope(_) | ResolveError::NotShared { .. } => {
                ReasonCode::InstanceNotAllowed
            }
            ResolveError::BindingChanged { .. } => ReasonCode::InstanceBindingChanged,
        },
    }
}
