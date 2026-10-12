//! Test-only `GatewayFactory` and `InstanceDirectory`: the one gateway it
//! was built with, whatever the instance — no `{{product}}Instance` nor
//! token `Secret` is needed. An unknown instance can still be simulated.

use std::collections::BTreeSet;
use std::sync::Arc;

use application::ports::{
    BoundInstance, GatewayFactory, GatewayFactoryError, InstanceDirectory, InstanceError,
    RemoteGateway, ResolveError,
};
use weebo_kit_api::{InstanceKind, InstanceRef};

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

    /// The instance named `instance` doesn't exist (`InstanceNotFound`).
    pub fn without(mut self, instance: &str) -> Self {
        self.missing.insert(instance.to_string());
        self
    }
}

#[async_trait::async_trait]
impl GatewayFactory for StaticGatewayFactory {
    async fn gateway_for(
        &self,
        instance: &BoundInstance,
    ) -> Result<Arc<dyn RemoteGateway>, GatewayFactoryError> {
        if self.missing.contains(&instance.name) {
            return Err(GatewayFactoryError::InstanceNotFound(instance.clone()));
        }
        Ok(self.gateway.clone())
    }
}

/// Every name is an instance — a `LocalInstance` when the ref says so,
/// a cluster-scoped one otherwise — except the missing ones. No ref: not
/// found.
#[async_trait::async_trait]
impl InstanceDirectory for StaticGatewayFactory {
    async fn resolve(
        &self,
        cr_namespace: Option<&str>,
        instance_ref: Option<&InstanceRef>,
        _recorded: Option<&BoundInstance>,
    ) -> Result<BoundInstance, InstanceError> {
        let Some(r) = instance_ref else {
            return Err(ResolveError::NoInstance.into());
        };
        if self.missing.contains(&r.name) {
            return Err(ResolveError::NotFound {
                kind: "Instance".into(),
                name: r.name.clone(),
            }
            .into());
        }
        Ok(match (r.kind, r.namespace.as_deref().or(cr_namespace)) {
            (Some(InstanceKind::LocalInstance), Some(ns)) => BoundInstance::local(ns, &r.name),
            _ => BoundInstance::cluster(&r.name),
        })
    }
}
