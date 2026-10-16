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
            && !owner.is(other)
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
            .is_some_and(|(o, _)| owner.is(o)))
    }

    async fn delete(
        &self,
        target: &SecretTarget,
        owner: &SecretOwner<'_>,
    ) -> Result<(), SecretStoreError> {
        let key = Self::key(target, owner);
        let mut secrets = self.secrets.lock().unwrap();
        if secrets.get(&key).is_some_and(|(o, _)| owner.is(o)) {
            secrets.remove(&key);
        }
        Ok(())
    }
}

/// The objects of managed instances, in memory: applied by name, a
/// conflict for the names set foreign, a `Deployment` whose rollout
/// completes when told (or at once), pods set by the test.
#[derive(Default)]
pub struct FakeOwnedObjects {
    objects: Mutex<BTreeMap<(String, String), crate::workload::OwnedObject>>,
    created: Mutex<BTreeMap<(String, String), k8s_openapi::jiff::Timestamp>>,
    foreign: Mutex<Vec<(String, String)>>,
    /// Rollouts complete as soon as applied unless held.
    hold_rollout: Mutex<bool>,
    pods: Mutex<Vec<crate::workload::PodState>>,
    log: Mutex<Vec<String>>,
}

impl FakeOwnedObjects {
    /// `kind/name` exists and isn't the operator's.
    pub fn foreign(&self, kind: &str, name: &str) {
        self.foreign
            .lock()
            .unwrap()
            .push((kind.into(), name.into()));
    }

    /// `kind/name` is the operator's again (or gone).
    pub fn unforeign(&self, kind: &str, name: &str) {
        self.foreign
            .lock()
            .unwrap()
            .retain(|(k, n)| (k.as_str(), n.as_str()) != (kind, name));
    }

    /// Whether `kind/name` is set foreign.
    pub fn is_foreign(&self, kind: &str, name: &str) -> bool {
        self.foreign
            .lock()
            .unwrap()
            .iter()
            .any(|(k, n)| (k.as_str(), n.as_str()) == (kind, name))
    }

    pub fn object(&self, kind: &str, name: &str) -> Option<crate::workload::OwnedObject> {
        self.objects
            .lock()
            .unwrap()
            .get(&(kind.into(), name.into()))
            .cloned()
    }

    /// The names of the objects of `kind`.
    pub fn names(&self, kind: &str) -> Vec<String> {
        self.objects
            .lock()
            .unwrap()
            .keys()
            .filter(|(k, _)| k == kind)
            .map(|(_, n)| n.clone())
            .collect()
    }

    pub fn hold_rollout(&self, hold: bool) {
        *self.hold_rollout.lock().unwrap() = hold;
    }

    /// What [`crate::workload::OwnedObjects::pods`] returns.
    pub fn set_pods(&self, pods: Vec<crate::workload::PodState>) {
        *self.pods.lock().unwrap() = pods;
    }

    /// What was removed (`kind/name`) and restarted (`restarted name`).
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// The text of key `key` of the `Secret` `name`.
    pub fn secret(&self, name: &str, key: &str) -> Option<String> {
        let secret = self.object("Secret", name)?;
        let encoded = secret.manifest["data"][key].as_str()?.to_string();
        let bytes: k8s_openapi::ByteString =
            serde_json::from_value(serde_json::Value::String(encoded)).ok()?;
        String::from_utf8(bytes.0).ok()
    }
}

#[async_trait::async_trait]
impl crate::workload::OwnedObjects for FakeOwnedObjects {
    async fn apply(
        &self,
        _owner: &crate::workload::Owner,
        objects: &[crate::workload::OwnedObject],
    ) -> Result<(), crate::workload::WorkloadError> {
        for object in objects {
            let key = (object.kind.clone(), object.name.clone());
            if self.foreign.lock().unwrap().contains(&key) {
                return Err(crate::workload::WorkloadError::Conflict(format!(
                    "{} {} exists and isn't the operator's",
                    object.kind, object.name
                )));
            }
            let mut objects = self.objects.lock().unwrap();
            if object.create_only && objects.contains_key(&key) {
                continue;
            }
            // Creation times one second apart, in apply order.
            let mut created = self.created.lock().unwrap();
            let next = created.len() as i64;
            created.entry(key.clone()).or_insert_with(|| {
                k8s_openapi::jiff::Timestamp::from_second(next).expect("a small second")
            });
            objects.insert(key, object.clone());
        }
        Ok(())
    }

    async fn remove(
        &self,
        _owner: &crate::workload::Owner,
        _api_version: &str,
        kind: &str,
        name: &str,
    ) -> Result<(), crate::workload::WorkloadError> {
        let key = (kind.to_string(), name.to_string());
        if self.objects.lock().unwrap().remove(&key).is_some() {
            self.created.lock().unwrap().remove(&key);
            self.log.lock().unwrap().push(format!("{kind}/{name}"));
        }
        Ok(())
    }

    async fn list(
        &self,
        _owner: &crate::workload::Owner,
        _api_version: &str,
        kind: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<Vec<crate::workload::Listed>, crate::workload::WorkloadError> {
        let created = self.created.lock().unwrap();
        Ok(self
            .objects
            .lock()
            .unwrap()
            .iter()
            .filter(|((k, _), o)| {
                k == kind
                    && labels.iter().all(|(key, value)| {
                        o.manifest["metadata"]["labels"][key].as_str() == Some(value)
                    })
            })
            .map(|(key, _)| crate::workload::Listed {
                name: key.1.clone(),
                created: created.get(key).copied(),
            })
            .collect())
    }

    async fn secret_text(
        &self,
        _owner: &crate::workload::Owner,
        name: &str,
        key: &str,
    ) -> Result<Option<String>, crate::workload::WorkloadError> {
        Ok(self.secret(name, key))
    }

    async fn rollout(
        &self,
        _owner: &crate::workload::Owner,
        name: &str,
    ) -> Result<Option<crate::workload::Rollout>, crate::workload::WorkloadError> {
        let Some(deployment) = self.object("Deployment", name) else {
            return Ok(None);
        };
        let spec = &deployment.manifest["spec"];
        let replicas = spec["replicas"].as_i64().unwrap_or(1) as i32;
        let ready = if *self.hold_rollout.lock().unwrap() {
            0
        } else {
            replicas
        };
        Ok(Some(crate::workload::Rollout {
            replicas,
            updated: ready,
            available: ready,
            observed: true,
            template_annotations: serde_json::from_value(
                spec["template"]["metadata"]["annotations"].clone(),
            )
            .unwrap_or_default(),
        }))
    }

    async fn annotate_template(
        &self,
        _owner: &crate::workload::Owner,
        name: &str,
        annotations: &BTreeMap<String, String>,
        restart: bool,
    ) -> Result<bool, crate::workload::WorkloadError> {
        let mut objects = self.objects.lock().unwrap();
        let Some(deployment) = objects.get_mut(&("Deployment".into(), name.into())) else {
            return Ok(false);
        };
        for (key, value) in annotations {
            deployment.manifest["spec"]["template"]["metadata"]["annotations"][key] =
                serde_json::Value::String(value.clone());
        }
        if restart {
            self.log.lock().unwrap().push(format!("restarted {name}"));
        }
        Ok(true)
    }

    async fn pods(
        &self,
        _owner: &crate::workload::Owner,
        _selector: &BTreeMap<String, String>,
    ) -> Result<Vec<crate::workload::PodState>, crate::workload::WorkloadError> {
        Ok(self.pods.lock().unwrap().clone())
    }

    async fn claim_size(
        &self,
        _owner: &crate::workload::Owner,
        name: &str,
    ) -> Result<Option<String>, crate::workload::WorkloadError> {
        Ok(self
            .object("PersistentVolumeClaim", name)
            .and_then(|claim| {
                claim.manifest["spec"]["resources"]["requests"]["storage"]
                    .as_str()
                    .map(String::from)
            }))
    }
}

/// Pod logs in memory: lines per `namespace/pod`, filtered by `since` like
/// the kubelet; a pod set unreadable answers like a missing `pods/log`
/// grant.
#[derive(Default)]
pub struct FakePodLogs {
    lines: Mutex<BTreeMap<String, Vec<crate::workload::LogLine>>>,
    unreadable: Mutex<Vec<String>>,
}

impl FakePodLogs {
    /// Appends `text` to the log of `namespace/pod`, logged at `time`.
    pub fn push(&self, namespace: &str, pod: &str, time: k8s_openapi::jiff::Timestamp, text: &str) {
        self.lines
            .lock()
            .unwrap()
            .entry(format!("{namespace}/{pod}"))
            .or_default()
            .push(crate::workload::LogLine {
                time,
                text: text.into(),
            });
    }

    /// Reading `namespace/pod` is forbidden.
    pub fn unreadable(&self, namespace: &str, pod: &str) {
        self.unreadable
            .lock()
            .unwrap()
            .push(format!("{namespace}/{pod}"));
    }
}

#[async_trait::async_trait]
impl crate::workload::PodLogs for FakePodLogs {
    async fn read(
        &self,
        namespace: &str,
        pod: &str,
        query: &crate::workload::LogQuery,
    ) -> Result<Vec<crate::workload::LogLine>, crate::workload::WorkloadError> {
        let key = format!("{namespace}/{pod}");
        if self.unreadable.lock().unwrap().contains(&key) {
            return Err(crate::workload::WorkloadError::Kubernetes(format!(
                "pods/log {key}: forbidden"
            )));
        }
        let mut budget = query.limit_bytes.unwrap_or(i64::MAX);
        Ok(self
            .lines
            .lock()
            .unwrap()
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|l| query.since.is_none_or(|since| l.time >= since))
            .take_while(|l| {
                budget -= l.text.len() as i64 + 1;
                budget >= 0
            })
            .cloned()
            .collect())
    }
}
