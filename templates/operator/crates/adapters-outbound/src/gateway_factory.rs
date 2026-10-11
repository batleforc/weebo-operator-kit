//! One [`HttpGateway`] per `{{product}}Instance`, built on every call: the
//! instance comes from the kit's resolver (a live reflector `Store` when
//! `operator::main` wires one in, the API otherwise), its token is read
//! fresh from `tokenSecretRef` — a rotated token or an edited `url` takes
//! effect on the next reconcile.

use std::sync::Arc;

use api::Instance;
use application::ports::{GatewayFactory, GatewayFactoryError, RemoteGateway};
use kube::Client;
use kube::runtime::reflector::Store;
use weebo_kit_adapters::{InstanceResolver, read_secret_key};

use crate::http_gateway::HttpGateway;

pub struct HttpGatewayFactory {
    client: Client,
    instances: InstanceResolver<Instance>,
}

impl HttpGatewayFactory {
    /// `store`: the shared `{{product}}Instance` reflector, `None` to read
    /// the API on every call (tests).
    pub fn new(client: Client, store: Option<Store<Instance>>) -> Self {
        Self {
            instances: InstanceResolver::new(client.clone(), store),
            client,
        }
    }

    async fn token(&self, instance: &Instance) -> Result<String, GatewayFactoryError> {
        let secret_ref = &instance.spec.token_secret_ref;
        let where_ = || {
            format!(
                "secret {}/{} key {:?}",
                secret_ref.namespace, secret_ref.name, secret_ref.key
            )
        };
        let bytes = read_secret_key(&self.client, secret_ref)
            .await
            .map_err(GatewayFactoryError::Kubernetes)?
            .ok_or_else(|| GatewayFactoryError::TokenSecret(format!("{} not found", where_())))?;
        let token = String::from_utf8(bytes)
            .map_err(|_| GatewayFactoryError::TokenSecret(format!("{} is not UTF-8", where_())))?;
        // A token written with `echo` carries a trailing newline.
        Ok(token.trim().to_string())
    }
}

#[async_trait::async_trait]
impl GatewayFactory for HttpGatewayFactory {
    async fn gateway_for(&self, name: &str) -> Result<Arc<dyn RemoteGateway>, GatewayFactoryError> {
        let instance = self
            .instances
            .get(name)
            .await
            .map_err(GatewayFactoryError::Kubernetes)?
            .ok_or_else(|| GatewayFactoryError::InstanceNotFound(name.to_string()))?;
        let token = self.token(&instance).await?;
        let client = HttpGateway::default_client().map_err(GatewayFactoryError::InvalidSpec)?;
        let gateway = HttpGateway::new(&instance.spec.url, token, client)
            .map_err(GatewayFactoryError::InvalidSpec)?;
        Ok(Arc::new(gateway))
    }
}
