//! In-memory [`SecretStore`] for use-case and controller tests.

use std::collections::BTreeMap;
use std::sync::Mutex;

use weebo_kit_api::{SecretStoreBackend, SecretTarget};

use crate::emit::check_target;
use crate::ports::{SecretOwner, SecretStore, SecretStoreError};

/// Owner id and data of one stored secret.
type Stored = (String, BTreeMap<String, String>);

/// Secrets keyed `<namespace>/<name>` (`<none>` without a namespace), with
/// the same ownership and namespace rules as the real stores.
#[derive(Default)]
pub struct FakeSecretStore {
    secrets: Mutex<BTreeMap<String, Stored>>,
    fail_writes: Mutex<bool>,
}

impl FakeSecretStore {
    fn key(target: &SecretTarget, owner: &SecretOwner<'_>) -> String {
        let ns = target
            .namespace
            .as_deref()
            .or(owner.namespace)
            .unwrap_or("<none>");
        format!("{ns}/{}", target.name)
    }

    pub fn get(&self, key: &str) -> Option<BTreeMap<String, String>> {
        self.secrets
            .lock()
            .unwrap()
            .get(key)
            .map(|(_, data)| data.clone())
    }

    /// The owner id recorded on `key`.
    pub fn owner_of(&self, key: &str) -> Option<String> {
        self.secrets
            .lock()
            .unwrap()
            .get(key)
            .map(|(owner, _)| owner.clone())
    }

    /// Seeds a value written by someone else (`owner` id).
    pub fn insert_foreign(&self, key: &str, owner: &str, data: BTreeMap<String, String>) {
        self.secrets
            .lock()
            .unwrap()
            .insert(key.to_string(), (owner.to_string(), data));
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
        owner: &SecretOwner<'_>,
        data: &BTreeMap<String, String>,
    ) -> Result<(), SecretStoreError> {
        check_target(target, owner).map_err(|e| SecretStoreError(e.to_string()))?;
        if *self.fail_writes.lock().unwrap() {
            return Err(SecretStoreError("write refused".into()));
        }
        let key = Self::key(target, owner);
        let mut secrets = self.secrets.lock().unwrap();
        if let Some((other, _)) = secrets.get(&key)
            && *other != owner.id()
        {
            return Err(SecretStoreError(format!("{key} belongs to {other}")));
        }
        secrets.insert(key, (owner.id(), data.clone()));
        Ok(())
    }

    async fn exists(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<bool, SecretStoreError> {
        Ok(self
            .secrets
            .lock()
            .unwrap()
            .get(&Self::key(target, owner))
            .is_some_and(|(o, _)| *o == owner.id()))
    }

    async fn delete(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<(), SecretStoreError> {
        let key = Self::key(target, owner);
        let mut secrets = self.secrets.lock().unwrap();
        if secrets.get(&key).is_some_and(|(o, _)| *o == owner.id()) {
            secrets.remove(&key);
        }
        Ok(())
    }
}
