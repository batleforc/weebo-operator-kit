//! Test-only `GatewayFactory`: hands out the one gateway it was built
//! with, whatever the instance — no `{{product}}Instance` nor token
//! `Secret` is needed. An unknown instance can still be simulated.

use std::collections::BTreeSet;
use std::sync::Arc;

use application::ports::{GatewayFactory, GatewayFactoryError, RemoteGateway};

pub struct StaticGatewayFactory {
    gateway: Arc<dyn RemoteGateway>,
    missing: BTreeSet<String>,
}

impl StaticGatewayFactory {
    pub fn new(gateway: Arc<dyn RemoteGateway>) -> Self {
        Self {
            gateway,
            missing: BTreeSet::new(),
        }
    }

    /// `instance` resolves to `InstanceNotFound`.
    pub fn without(mut self, instance: &str) -> Self {
        self.missing.insert(instance.to_string());
        self
    }
}

#[async_trait::async_trait]
impl GatewayFactory for StaticGatewayFactory {
    async fn gateway_for(
        &self,
        instance: &str,
    ) -> Result<Arc<dyn RemoteGateway>, GatewayFactoryError> {
        if self.missing.contains(instance) {
            return Err(GatewayFactoryError::InstanceNotFound(instance.to_string()));
        }
        Ok(self.gateway.clone())
    }
}
