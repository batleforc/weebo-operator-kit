//! [`OwnedObjects`] and [`PodLogs`] on the Kubernetes API: the objects of a
//! managed instance, applied by server-side apply (the operator's field
//! manager, forced: the operator owns every field it writes), each read
//! first.
//!
//! **No implicit adoption**: an object of the same name that carries
//! neither an owner reference to the instance nor the labels
//! `app.kubernetes.io/managed-by: <managed_by>` and
//! `app.kubernetes.io/instance: <instance>` is a conflict — server-side
//! apply would take it over silently. Labelling it hands it over. An
//! object another instance controls (its controller owner reference) is a
//! conflict whatever its labels: two instances never take one object from
//! each other.
//!
//! **What the pods may be** (`weebo_kit_application::pod_safety`): before
//! anything is written, the pod templates are checked against the image
//! allow-list and the privileges refused, and every `Secret` they read must
//! carry the operator's credential label — the operator's webhook refuses
//! the same, this is the check of what is actually applied.

use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::api::{
    Api, ApiResource, DeleteParams, DynamicObject, GroupVersionKind, ListParams, LogParams, Patch,
    PatchParams,
};
use kube::{Client, ResourceExt};
use serde_json::json;
use weebo_kit_application::pod_safety;
use weebo_kit_application::workload::{
    Listed, LogLine, LogQuery, OwnedObject, OwnedObjects, Owner, PodLogs, PodState, Rollout,
    WorkloadError, parse_timestamped,
};

/// What ties [`KubeOwnedObjects`] to an operator.
#[derive(Debug, Clone)]
pub struct WorkloadSettings {
    /// The field manager of every apply (`weebo-<product>-operator`).
    pub field_manager: String,
    /// The `app.kubernetes.io/managed-by` value that hands an unowned
    /// object over.
    pub managed_by: String,
    /// The instance kind owning the objects: `apiVersion` and `kind` of
    /// the owner references.
    pub owner_api_version: String,
    pub owner_kind: String,
    /// The label (`"true"`) a `Secret` read by the pods must carry.
    pub credential_label: String,
    /// The image repositories a pod may run.
    pub allowed_images: Vec<String>,
}

pub struct KubeOwnedObjects {
    client: Client,
    settings: WorkloadSettings,
}

impl KubeOwnedObjects {
    pub fn new(client: Client, settings: WorkloadSettings) -> Self {
        Self { client, settings }
    }

    pub fn settings(&self) -> &WorkloadSettings {
        &self.settings
    }

    /// The owner reference every object carries.
    pub fn owner_reference(&self, owner: &Owner) -> OwnerReference {
        OwnerReference {
            api_version: self.settings.owner_api_version.clone(),
            kind: self.settings.owner_kind.clone(),
            name: owner.name.clone(),
            uid: owner.uid.clone(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }
    }

    /// Whether `meta` is an object the operator may write over for `owner`.
    pub fn is_ours(&self, meta: &ObjectMeta, owner: &Owner) -> bool {
        is_ours(meta, owner, &self.settings.managed_by)
    }

    /// The conflict of an object `kind/name` that isn't the owner's.
    pub fn conflict(&self, kind: &str, name: &str, namespace: &str) -> WorkloadError {
        let managed_by = &self.settings.managed_by;
        WorkloadError::Conflict(format!(
            "{kind} {namespace}/{name} exists and isn't this instance's (no owner reference to it, \
             nor the labels app.kubernetes.io/managed-by={managed_by} and \
             app.kubernetes.io/instance=<instance> without another controller): never taken over. \
             Delete or rename it, or label it to hand it over"
        ))
    }

    /// The pods of `objects` checked, and the `Secret`s they read: nothing
    /// is written when one is refused.
    async fn check_pods(
        &self,
        owner: &Owner,
        objects: &[OwnedObject],
    ) -> Result<(), WorkloadError> {
        let problems = pod_safety::problems(objects, &self.settings.allowed_images);
        if !problems.is_empty() {
            return Err(WorkloadError::Refused(format!(
                "the instance asks for pods the operator doesn't run: {}",
                pod_safety::summary(&problems)
            )));
        }
        let api: Api<Secret> = Api::namespaced(self.client.clone(), &owner.namespace);
        let label = &self.settings.credential_label;
        for name in pod_safety::secrets_to_check(objects) {
            let meta = api
                .get_metadata_opt(&name)
                .await
                .map_err(|e| kube_error(&format!("Secret {name}"), e))?;
            // Absent: the pod waits for it (the kubelet says so); present,
            // it must consent.
            if let Some(meta) = meta
                && meta.labels().get(label).map(String::as_str) != Some("true")
            {
                return Err(WorkloadError::SecretNotAllowed(
                    pod_safety::secret_not_labelled(&owner.namespace, &name, label),
                ));
            }
        }
        Ok(())
    }

    fn dynamic(&self, owner: &Owner, api_version: &str, kind: &str) -> Api<DynamicObject> {
        let (group, version) = match api_version.split_once('/') {
            Some((group, version)) => (group, version),
            None => ("", api_version),
        };
        let resource = ApiResource::from_gvk(&GroupVersionKind::gvk(group, version, kind));
        Api::namespaced_with(self.client.clone(), &owner.namespace, &resource)
    }
}

/// A Kubernetes error on `what`, as a [`WorkloadError`].
pub fn kube_error(what: &str, e: kube::Error) -> WorkloadError {
    WorkloadError::Kubernetes(format!("{what}: {e}"))
}

/// Whether `meta` is an object the operator may write over for `owner`:
/// owned by it, or labelled for it (`managed-by` and `instance`) and
/// controlled by no other object.
pub fn is_ours(meta: &ObjectMeta, owner: &Owner, managed_by: &str) -> bool {
    let references = meta.owner_references.iter().flatten();
    if references.clone().any(|r| r.uid == owner.uid) {
        return true;
    }
    if references
        .clone()
        .any(|r| r.controller == Some(true) && r.uid != owner.uid)
    {
        return false;
    }
    let label = |key: &str| meta.labels.as_ref().and_then(|l| l.get(key));
    label("app.kubernetes.io/managed-by").is_some_and(|v| v == managed_by)
        && label("app.kubernetes.io/instance").is_some_and(|v| *v == owner.name)
}

/// The `metadata` map of a manifest, created when missing; `None` when the
/// manifest (or its `metadata`) isn't a JSON object.
fn metadata_of(
    manifest: &mut serde_json::Value,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    manifest
        .as_object_mut()?
        .entry("metadata")
        .or_insert_with(|| json!({}))
        .as_object_mut()
}

fn selector(labels: &BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Where a pod stands, from its status.
pub fn pod_state(pod: &Pod) -> PodState {
    let status = pod.status.clone().unwrap_or_default();
    let ready = status
        .conditions
        .iter()
        .flatten()
        .any(|c| c.type_ == "Ready" && c.status == "True");
    let container = status.container_statuses.iter().flatten().next();
    let state = container.and_then(|c| c.state.as_ref());
    let last_error = container
        .and_then(|c| c.last_state.as_ref())
        .and_then(|s| s.terminated.as_ref())
        .or_else(|| state.and_then(|s| s.terminated.as_ref()))
        .filter(|t| t.exit_code != 0)
        .map(|t| {
            let reason = t.reason.clone().unwrap_or_else(|| "Error".into());
            match &t.message {
                Some(message) if !message.trim().is_empty() => {
                    format!("{reason} (exit code {}): {}", t.exit_code, message.trim())
                }
                _ => format!("{reason} (exit code {})", t.exit_code),
            }
        });
    PodState {
        name: pod.name_any(),
        ready,
        started: state
            .and_then(|s| s.running.as_ref())
            .and_then(|r| r.started_at.as_ref())
            .map(|t| t.0),
        restarts: container.map_or(0, |c| c.restart_count),
        waiting: state
            .and_then(|s| s.waiting.as_ref())
            .and_then(|w| w.reason.clone()),
        last_error,
        annotations: pod.annotations().clone(),
    }
}

#[async_trait::async_trait]
impl OwnedObjects for KubeOwnedObjects {
    async fn apply(&self, owner: &Owner, objects: &[OwnedObject]) -> Result<(), WorkloadError> {
        self.check_pods(owner, objects).await?;
        let params = PatchParams::apply(&self.settings.field_manager).force();
        for object in objects {
            let api = self.dynamic(owner, &object.api_version, &object.kind);
            let what = format!("{} {}", object.kind, object.name);
            if let Some(existing) = api
                .get_opt(&object.name)
                .await
                .map_err(|e| kube_error(&what, e))?
            {
                if !self.is_ours(&existing.metadata, owner) {
                    return Err(self.conflict(&object.kind, &object.name, &owner.namespace));
                }
                if object.create_only {
                    continue;
                }
            }
            let mut manifest = object.manifest.clone();
            let Some(metadata) = metadata_of(&mut manifest) else {
                return Err(WorkloadError::Refused(format!(
                    "{what}: its manifest is not an object"
                )));
            };
            metadata.insert(
                "ownerReferences".into(),
                json!([self.owner_reference(owner)]),
            );
            api.patch(&object.name, &params, &Patch::Apply(manifest))
                .await
                .map_err(|e| kube_error(&what, e))?;
        }
        Ok(())
    }

    async fn remove(
        &self,
        owner: &Owner,
        api_version: &str,
        kind: &str,
        name: &str,
    ) -> Result<(), WorkloadError> {
        let api = self.dynamic(owner, api_version, kind);
        let what = format!("{kind} {name}");
        match api.get_opt(name).await {
            Ok(Some(existing)) if self.is_ours(&existing.metadata, owner) => api
                .delete(name, &DeleteParams::background())
                .await
                .map(|_| ())
                .map_err(|e| kube_error(&what, e)),
            Ok(_) => Ok(()),
            // A kind the cluster doesn't serve (no autoscaling/v2…): nothing
            // of it to remove.
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
            Err(e) => Err(kube_error(&what, e)),
        }
    }

    async fn list(
        &self,
        owner: &Owner,
        api_version: &str,
        kind: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<Vec<Listed>, WorkloadError> {
        let api = self.dynamic(owner, api_version, kind);
        let listed = api
            .list_metadata(&ListParams::default().labels(&selector(labels)))
            .await
            .map_err(|e| kube_error(kind, e))?;
        Ok(listed
            .items
            .iter()
            .filter(|o| self.is_ours(&o.metadata, owner))
            .map(|o| Listed {
                name: o.name_any(),
                created: o.metadata.creation_timestamp.as_ref().map(|t| t.0),
            })
            .collect())
    }

    async fn secret_text(
        &self,
        owner: &Owner,
        name: &str,
        key: &str,
    ) -> Result<Option<String>, WorkloadError> {
        let api: Api<Secret> = Api::namespaced(self.client.clone(), &owner.namespace);
        let Some(secret) = api
            .get_opt(name)
            .await
            .map_err(|e| kube_error(&format!("Secret {name}"), e))?
        else {
            return Ok(None);
        };
        if !self.is_ours(&secret.metadata, owner) {
            return Ok(None);
        }
        Ok(secret
            .data
            .as_ref()
            .and_then(|d| d.get(key))
            .and_then(|b| String::from_utf8(b.0.clone()).ok()))
    }

    async fn rollout(&self, owner: &Owner, name: &str) -> Result<Option<Rollout>, WorkloadError> {
        let api: Api<Deployment> = Api::namespaced(self.client.clone(), &owner.namespace);
        let Some(deployment) = api
            .get_opt(name)
            .await
            .map_err(|e| kube_error(&format!("Deployment {name}"), e))?
        else {
            return Ok(None);
        };
        let spec = deployment.spec.as_ref();
        let status = deployment.status.clone().unwrap_or_default();
        Ok(Some(Rollout {
            replicas: spec.and_then(|s| s.replicas).unwrap_or(1),
            updated: status.updated_replicas.unwrap_or(0),
            available: status.available_replicas.unwrap_or(0),
            observed: status.observed_generation >= deployment.metadata.generation
                && deployment.metadata.generation.is_some(),
            template_annotations: spec
                .and_then(|s| s.template.metadata.as_ref())
                .and_then(|m| m.annotations.clone())
                .unwrap_or_default(),
        }))
    }

    async fn annotate_template(
        &self,
        owner: &Owner,
        name: &str,
        annotations: &BTreeMap<String, String>,
        restart: bool,
    ) -> Result<bool, WorkloadError> {
        let api: Api<Deployment> = Api::namespaced(self.client.clone(), &owner.namespace);
        let what = format!("Deployment {name}");
        let Some(existing) = api.get_opt(name).await.map_err(|e| kube_error(&what, e))? else {
            return Ok(false);
        };
        if !self.is_ours(&existing.metadata, owner) {
            return Err(self.conflict("Deployment", name, &owner.namespace));
        }
        let mut patch: serde_json::Map<String, serde_json::Value> = annotations
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect();
        if restart {
            patch.insert(
                "kubectl.kubernetes.io/restartedAt".into(),
                json!(k8s_openapi::jiff::Timestamp::now().to_string()),
            );
        }
        // A merge patch under its own manager: the operator's next apply
        // leaves `restartedAt` alone, and agrees on the annotations it sets.
        api.patch(
            name,
            &PatchParams {
                field_manager: Some(format!("{}-restart", self.settings.field_manager)),
                ..Default::default()
            },
            &Patch::Merge(json!({
                "spec": {"template": {"metadata": {"annotations": patch}}}
            })),
        )
        .await
        .map_err(|e| kube_error(&what, e))?;
        Ok(true)
    }

    async fn pods(
        &self,
        owner: &Owner,
        selector_labels: &BTreeMap<String, String>,
    ) -> Result<Vec<PodState>, WorkloadError> {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), &owner.namespace);
        let pods = api
            .list(&ListParams::default().labels(&selector(selector_labels)))
            .await
            .map_err(|e| kube_error("Pods", e))?;
        Ok(pods.items.iter().map(pod_state).collect())
    }

    async fn claim_size(&self, owner: &Owner, name: &str) -> Result<Option<String>, WorkloadError> {
        let api: Api<PersistentVolumeClaim> =
            Api::namespaced(self.client.clone(), &owner.namespace);
        let claim = api
            .get_opt(name)
            .await
            .map_err(|e| kube_error(&format!("PersistentVolumeClaim {name}"), e))?;
        Ok(claim
            .and_then(|c| c.spec)
            .and_then(|s| s.resources)
            .and_then(|r| r.requests)
            .and_then(|r| r.get("storage").map(|q| q.0.clone())))
    }
}

/// [`PodLogs`] through `pods/log` (the chart grants `get`).
pub struct KubePodLogs {
    client: Client,
}

impl KubePodLogs {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl PodLogs for KubePodLogs {
    async fn read(
        &self,
        namespace: &str,
        pod: &str,
        query: &LogQuery,
    ) -> Result<Vec<LogLine>, WorkloadError> {
        let api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
        let text = api
            .logs(
                pod,
                &LogParams {
                    container: query.container.clone(),
                    since_time: query.since,
                    limit_bytes: query.limit_bytes,
                    timestamps: true,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| kube_error(&format!("pods/log {namespace}/{pod}"), e))?;
        Ok(parse_timestamped(&text))
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateRunning, ContainerStateTerminated, ContainerStateWaiting,
        ContainerStatus, PodCondition, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

    use super::*;

    const MANAGED_BY: &str = "weebo-example";

    #[test]
    fn metadata_of_refuses_a_manifest_that_is_not_an_object() {
        let mut empty = json!({});
        assert!(metadata_of(&mut empty).is_some());
        assert_eq!(empty, json!({ "metadata": {} }));
        assert!(metadata_of(&mut json!("pod")).is_none());
        assert!(metadata_of(&mut json!({ "metadata": 1 })).is_none());
    }

    fn owner() -> Owner {
        Owner {
            namespace: "team".into(),
            name: "hub".into(),
            uid: "u-1".into(),
        }
    }

    fn reference(uid: &str, controller: Option<bool>) -> OwnerReference {
        OwnerReference {
            api_version: "example.weebo.io/v1alpha1".into(),
            kind: "ExampleLocalInstance".into(),
            name: "hub".into(),
            uid: uid.into(),
            controller,
            block_owner_deletion: None,
        }
    }

    #[test]
    fn only_owned_or_labelled_objects_are_ours() {
        let ours = |meta: &ObjectMeta| is_ours(meta, &owner(), MANAGED_BY);
        let mut meta = ObjectMeta::default();
        assert!(!ours(&meta));
        meta.owner_references = Some(vec![reference("u-1", Some(true))]);
        assert!(ours(&meta));
        meta.owner_references = Some(vec![reference("other", Some(true))]);
        assert!(!ours(&meta));
        // Labelled for it, but controlled by another instance: never.
        meta.labels = Some(BTreeMap::from([
            ("app.kubernetes.io/managed-by".into(), MANAGED_BY.into()),
            ("app.kubernetes.io/instance".into(), "hub".into()),
        ]));
        assert!(!ours(&meta));
        // A non-controller reference doesn't hold it.
        meta.owner_references = Some(vec![reference("other", None)]);
        assert!(ours(&meta));
        meta.owner_references = None;
        assert!(ours(&meta));
        // `managed-by` alone, or another instance's name: not this one's
        // (instance `hub` and `hub-worker` share names).
        meta.labels = Some(BTreeMap::from([(
            "app.kubernetes.io/managed-by".into(),
            MANAGED_BY.into(),
        )]));
        assert!(!ours(&meta));
        meta.labels = Some(BTreeMap::from([
            ("app.kubernetes.io/managed-by".into(), MANAGED_BY.into()),
            ("app.kubernetes.io/instance".into(), "hub-worker".into()),
        ]));
        assert!(!ours(&meta));
    }

    fn pod(status: PodStatus) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some("hub-1".into()),
                annotations: Some(BTreeMap::from([("h".into(), "1".into())])),
                ..Default::default()
            },
            status: Some(status),
            ..Default::default()
        }
    }

    #[test]
    fn a_pod_state_reads_readiness_start_and_the_last_failure() {
        let started: k8s_openapi::jiff::Timestamp = "2026-10-16T10:00:00Z".parse().unwrap();
        let running = pod(PodStatus {
            conditions: Some(vec![PodCondition {
                type_: "Ready".into(),
                status: "True".into(),
                ..Default::default()
            }]),
            container_statuses: Some(vec![ContainerStatus {
                name: "app".into(),
                restart_count: 1,
                state: Some(ContainerState {
                    running: Some(ContainerStateRunning {
                        started_at: Some(Time(started)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let state = pod_state(&running);
        assert!(state.ready);
        assert_eq!(state.started, Some(started));
        assert_eq!(state.restarts, 1);
        assert_eq!(state.last_error, None);
        assert_eq!(state.annotations["h"], "1");

        let crashing = pod(PodStatus {
            container_statuses: Some(vec![ContainerStatus {
                name: "app".into(),
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("CrashLoopBackOff".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                last_state: Some(ContainerState {
                    terminated: Some(ContainerStateTerminated {
                        exit_code: 1,
                        reason: Some("Error".into()),
                        message: Some("bad config\n".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let state = pod_state(&crashing);
        assert!(!state.ready);
        assert_eq!(state.waiting.as_deref(), Some("CrashLoopBackOff"));
        assert_eq!(
            state.last_error.as_deref(),
            Some("Error (exit code 1): bad config")
        );
    }
}
