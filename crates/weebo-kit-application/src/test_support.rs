//! In-memory [`SecretStore`] for use-case and controller tests.

use std::collections::BTreeMap;
use std::sync::Mutex;

use weebo_kit_api::{SecretStoreBackend, SecretTarget};

use crate::ports::{SecretStore, SecretStoreError};

/// Secrets keyed `<namespace>/<name>` (`<none>` without a namespace).
#[derive(Default)]
pub struct FakeSecretStore {
    secrets: Mutex<BTreeMap<String, BTreeMap<String, String>>>,
    fail_writes: Mutex<bool>,
}

impl FakeSecretStore {
    fn key(target: &SecretTarget, default_namespace: Option<&str>) -> String {
        let ns = target
            .namespace
            .as_deref()
            .or(default_namespace)
            .unwrap_or("<none>");
        format!("{ns}/{}", target.name)
    }

    pub fn get(&self, key: &str) -> Option<BTreeMap<String, String>> {
        self.secrets.lock().unwrap().get(key).cloned()
    }

    pub fn remove(&self, key: &str) {
        self.secrets.lock().unwrap().remove(key);
    }

    pub fn fail_writes(&self, fail: bool) {
        *self.fail_writes.lock().unwrap() = fail;
    }
}

#[async_trait::async_trait]
impl SecretStore for FakeSecretStore {
    fn default_backend(&self) -> SecretStoreBackend {
        SecretStoreBackend::Kubernetes
    }

    async fn write(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        if *self.fail_writes.lock().unwrap() {
            return Err(SecretStoreError("write refused".into()));
        }
        self.secrets
            .lock()
            .unwrap()
            .insert(Self::key(target, default_namespace), data.clone());
        Ok(())
    }

    async fn exists(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
    ) -> Result<bool, SecretStoreError> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .contains_key(&Self::key(target, default_namespace)))
    }

    async fn delete(
        &self,
        target: &SecretTarget,
        default_namespace: Option<&str>,
    ) -> Result<(), SecretStoreError> {
        self.secrets
            .lock()
            .unwrap()
            .remove(&Self::key(target, default_namespace));
        Ok(())
    }
}
