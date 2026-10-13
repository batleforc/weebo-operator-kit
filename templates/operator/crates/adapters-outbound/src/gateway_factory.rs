//! One [`HttpGateway`] per instance — a `{{product}}Instance` or a
//! `{{product}}LocalInstance` — built on every call: the instance comes from
//! the kit's [`Instances`] (live reflector `Store`s when `operator::main`
//! wires them in, the API otherwise), its token is read fresh from
//! `tokenSecretRef` — a rotated token or an edited `url` takes effect on the
//! next reconcile.

use std::sync::Arc;

use api::{Instance, LocalInstance};
use application::ports::{BoundInstance, GatewayFactory, GatewayFactoryError, RemoteGateway};
use kube::{Client, ResourceExt};
use weebo_kit_adapters::{
    Instances, Resolved, TokenSecretError, read_local_instance_token, read_secret_key,
};
use weebo_kit_api::SecretKeyRef;

use crate::http_gateway::HttpGateway;

/// The operator's instances, both kinds: also its `InstanceDirectory`.
pub type OperatorInstances = Instances<Instance, LocalInstance>;

pub struct HttpGatewayFactory {
    client: Client,
    instances: Arc<OperatorInstances>,
}

impl HttpGatewayFactory {
    pub fn new(client: Client, instances: Arc<OperatorInstances>) -> Self {
        Self { client, instances }
    }

    /// A local instance's token is only read from a Secret labelled for it
    /// (`<group>/instance-token: "true"`): the operator reads with its own
    /// rights, and the CR names the URL the token is sent to.
    async fn token(
        &self,
        secret_ref: &SecretKeyRef,
        local: bool,
    ) -> Result<String, GatewayFactoryError> {
        let where_ = || {
            format!(
                "secret {}/{} key {:?}",
                secret_ref.namespace, secret_ref.name, secret_ref.key
            )
        };
        let bytes = if local {
            read_local_instance_token(&self.client, secret_ref, &api::ANNOTATIONS)
                .await
                .map_err(|e| match e {
                    TokenSecretError::NotAllowed { .. } => {
                        GatewayFactoryError::TokenNotAllowed(e.to_string())
                    }
                    TokenSecretError::Kubernetes(e) => GatewayFactoryError::Kubernetes(e),
                })?
        } else {
            read_secret_key(&self.client, secret_ref)
                .await
                .map_err(GatewayFactoryError::Kubernetes)?
        }
        .ok_or_else(|| GatewayFactoryError::TokenSecret(format!("{} not found", where_())))?;
        let token = String::from_utf8(bytes)
            .map_err(|_| GatewayFactoryError::TokenSecret(format!("{} is not UTF-8", where_())))?;
        // A token written with `echo` carries a trailing newline.
        Ok(token.trim().to_string())
    }
}

#[async_trait::async_trait]
impl GatewayFactory for HttpGatewayFactory {
    async fn gateway_for(
        &self,
        bound: &BoundInstance,
    ) -> Result<Arc<dyn RemoteGateway>, GatewayFactoryError> {
        let instance = self
            .instances
            .get(bound)
            .await
            .map_err(GatewayFactoryError::Kubernetes)?
            .ok_or_else(|| GatewayFactoryError::InstanceNotFound(bound.clone()))?;
        // A local instance's token lives in its own namespace.
        let (url, secret_ref, local) = match &instance {
            Resolved::Cluster(i) => (&i.spec.url, i.spec.token_secret_ref.clone(), false),
            Resolved::Local(l) => {
                let local = &l.spec.token_secret_ref;
                let secret_ref = SecretKeyRef {
                    name: local.name.clone(),
                    namespace: l.namespace().unwrap_or_default(),
                    key: local.key.clone(),
                };
                (&l.spec.url, secret_ref, true)
            }
        };
        let token = self.token(&secret_ref, local).await?;
        let client = HttpGateway::default_client().map_err(GatewayFactoryError::InvalidSpec)?;
        let gateway =
            HttpGateway::new(url, token, client).map_err(GatewayFactoryError::InvalidSpec)?;
        Ok(Arc::new(gateway))
    }
}
