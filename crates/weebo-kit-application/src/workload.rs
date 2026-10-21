//! The workload of a managed instance: the Kubernetes objects an operator
//! renders from an instance's spec and owns (a `Deployment`, its
//! configuration `Secret`s, a `Service`…), the ports that write and read
//! them, and the pure pieces every managed workload shares (hash-named
//! `Secret`s, restricted security contexts, the `NetworkPolicy` letting the
//! operator in).
//!
//! No client here: [`OwnedObjects`] and [`PodLogs`] are implemented on the
//! Kubernetes API in `weebo-kit-adapters` (`KubeOwnedObjects`,
//! `KubePodLogs`), in memory in `test_support`. Errors are neutral
//! ([`WorkloadError`]): the operator maps them to its reason codes.

use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::{
    Capabilities, PodSecurityContext, SeccompProfile, SecurityContext,
};
use k8s_openapi::api::networking::v1::{
    NetworkPolicy, NetworkPolicyEgressRule, NetworkPolicyIngressRule, NetworkPolicyPeer,
    NetworkPolicyPort, NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use k8s_openapi::jiff::Timestamp;
use k8s_openapi::{Metadata, Resource};
use sha2::{Digest, Sha256};

/// An object the operator owns, as a full manifest (`apiVersion`, `kind`,
/// `metadata`, …): applied by server-side apply.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnedObject {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub manifest: serde_json::Value,
    /// Created once, never applied again (a claim's spec, an immutable
    /// `Secret`).
    pub create_only: bool,
}

impl OwnedObject {
    /// `k` as an owned object, named by its `metadata.name`.
    pub fn of<K>(k: &K, create_only: bool) -> Self
    where
        K: Resource + Metadata<Ty = ObjectMeta> + serde::Serialize,
    {
        #[expect(
            clippy::expect_used,
            reason = "a Kubernetes object's maps all have string keys: serde_json can't refuse it"
        )]
        let manifest = serde_json::to_value(k).expect("a Kubernetes object serializes");
        Self {
            api_version: K::API_VERSION.to_string(),
            kind: K::KIND.to_string(),
            name: k.metadata().name.clone().unwrap_or_default(),
            manifest,
            create_only,
        }
    }
}

/// The instance owning the objects: every object carries a controller
/// owner reference to it, in its namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

/// Why owned objects couldn't be written or read. Neutral: the operator
/// maps each variant to its reason code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkloadError {
    /// An object of that name exists and isn't the operator's (no owner
    /// reference to the instance, no `managed-by` label): never taken
    /// over.
    #[error("{0}")]
    Conflict(String),
    #[error("Kubernetes API error: {0}")]
    Kubernetes(String),
    /// A pod the operator doesn't run (an image off its allow-list, a
    /// `hostPath`, a privilege): nothing is applied.
    #[error("{0}")]
    Refused(String),
    /// A pod would read a `Secret` without the operator's credential
    /// label: nothing is applied.
    #[error("{0}")]
    SecretNotAllowed(String),
}

/// Where a `Deployment` stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rollout {
    pub replicas: i32,
    pub updated: i32,
    pub available: i32,
    /// The controller has seen the last change of the spec.
    pub observed: bool,
    /// The pod template's annotations (the hash that rolls the pods…).
    pub template_annotations: BTreeMap<String, String>,
}

impl Rollout {
    /// Every replica updated and available.
    pub fn complete(&self) -> bool {
        self.observed && self.updated >= self.replicas && self.available >= self.replicas
    }

    /// The pod template's annotation `key`.
    pub fn annotation(&self, key: &str) -> Option<&str> {
        self.template_annotations.get(key).map(String::as_str)
    }
}

/// An owned object as listed: its name and when it was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub created: Option<Timestamp>,
}

/// Where a pod stands, as its status says (its first container, the
/// instance's own).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PodState {
    pub name: String,
    /// The `Ready` condition.
    pub ready: bool,
    /// When the container last started (`state.running.startedAt`): a pod
    /// started after a file was written read it at start.
    pub started: Option<Timestamp>,
    pub restarts: i32,
    /// Why the container waits (`CrashLoopBackOff`, `ImagePullBackOff`…).
    pub waiting: Option<String>,
    /// The last termination's reason and message, when it failed.
    pub last_error: Option<String>,
    /// The pod template's annotations.
    pub annotations: BTreeMap<String, String>,
}

/// The objects of a managed instance on the cluster. Every write checks
/// first that the object is the owner's (see the adapter's `is_ours`).
#[async_trait::async_trait]
pub trait OwnedObjects: Send + Sync {
    /// Applies `objects`, owned by `owner`; each is read first, and one the
    /// operator doesn't own is [`WorkloadError::Conflict`] (nothing after
    /// it is applied). The pod templates are checked first
    /// (`crate::pod_safety`): nothing is written when one is refused.
    async fn apply(&self, owner: &Owner, objects: &[OwnedObject]) -> Result<(), WorkloadError>;
    /// Deletes `kind/name` of the owner's namespace, if it is the
    /// operator's (none is fine).
    async fn remove(
        &self,
        owner: &Owner,
        api_version: &str,
        kind: &str,
        name: &str,
    ) -> Result<(), WorkloadError>;
    /// The owner's objects of `kind` carrying every label of `labels`.
    async fn list(
        &self,
        owner: &Owner,
        api_version: &str,
        kind: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<Vec<Listed>, WorkloadError>;
    /// A key of a `Secret` of the owner's namespace the operator wrote
    /// (`None`: absent, or not the owner's).
    async fn secret_text(
        &self,
        owner: &Owner,
        name: &str,
        key: &str,
    ) -> Result<Option<String>, WorkloadError>;
    /// The `Deployment` `name`, `None` when absent.
    async fn rollout(&self, owner: &Owner, name: &str) -> Result<Option<Rollout>, WorkloadError>;
    /// Sets `annotations` on the pod template of the `Deployment` `name`
    /// (rolls its pods), with `kubectl.kubernetes.io/restartedAt` when
    /// `restart`; `false` when it doesn't exist.
    async fn annotate_template(
        &self,
        owner: &Owner,
        name: &str,
        annotations: &BTreeMap<String, String>,
        restart: bool,
    ) -> Result<bool, WorkloadError>;
    /// The pods of the owner's namespace matching `selector`.
    async fn pods(
        &self,
        owner: &Owner,
        selector: &BTreeMap<String, String>,
    ) -> Result<Vec<PodState>, WorkloadError>;
    /// The storage the `PersistentVolumeClaim` `name` requests, `None`
    /// when absent.
    async fn claim_size(&self, owner: &Owner, name: &str) -> Result<Option<String>, WorkloadError>;
}

/// What of a pod's log to read: bounded, since a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogQuery {
    /// The container (`None`: the pod's only one).
    pub container: Option<String>,
    /// Lines logged at or after this time only.
    pub since: Option<Timestamp>,
    /// At most this many bytes (the newest are dropped past it: read again
    /// from the last time seen).
    pub limit_bytes: Option<i64>,
}

/// One line of a pod's log, with the time the kubelet recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub time: Timestamp,
    pub text: String,
}

/// The logs of the pods of a managed instance (`pods/log`): what an
/// instance says it did with a file it reloaded.
#[async_trait::async_trait]
pub trait PodLogs: Send + Sync {
    /// The lines of `pod` matching `query`, oldest first.
    async fn read(
        &self,
        namespace: &str,
        pod: &str,
        query: &LogQuery,
    ) -> Result<Vec<LogLine>, WorkloadError>;
}

/// Parses `kubectl logs --timestamps` output: `<RFC 3339> <line>` per line;
/// a line without a readable time is dropped.
pub fn parse_timestamped(text: &str) -> Vec<LogLine> {
    text.lines()
        .filter_map(|line| {
            let (time, text) = line.split_once(' ')?;
            Some(LogLine {
                time: time.parse().ok()?,
                text: text.to_string(),
            })
        })
        .collect()
}

/// The hex SHA-256 of `content`.
pub fn content_hash(content: &[u8]) -> String {
    Sha256::digest(content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The name of an immutable object holding `content`:
/// `<base>-<first 8 hex of its hash>`. A new content is a new object, so a
/// pod never sees the one it mounts change.
pub fn hashed_name(base: &str, content: &[u8]) -> String {
    format!("{base}-{}", &content_hash(content)[..8])
}

/// The hash-named objects to delete: those no pod mounts (`in_use`),
/// beyond the `keep` newest — kept for a `kubectl rollout undo`.
pub fn stale(listed: &[Listed], in_use: &BTreeSet<String>, keep: usize) -> Vec<String> {
    let mut unused: Vec<&Listed> = listed
        .iter()
        .filter(|l| !in_use.contains(&l.name))
        .collect();
    // Newest first; an unknown time counts as the oldest.
    unused.sort_by(|a, b| b.created.cmp(&a.created).then(a.name.cmp(&b.name)));
    unused
        .into_iter()
        .skip(keep)
        .map(|l| l.name.clone())
        .collect()
}

/// The operator's own pods, as the chart describes them
/// (`weebo_kit_runtime::pod`): what a managed instance's `NetworkPolicy`
/// lets in when the operator must reach the instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorPod {
    /// Its namespace (`POD_NAMESPACE`); `None`: the instance's own.
    pub namespace: Option<String>,
    /// Its selector labels (`WEEBO_OPERATOR_POD_LABELS`).
    pub labels: BTreeMap<String, String>,
}

impl OperatorPod {
    /// The chart's selector label when its `nameOverride` is unset, for an
    /// operator run without `WEEBO_OPERATOR_POD_LABELS`.
    pub fn named(chart: &str) -> Self {
        Self {
            namespace: None,
            labels: BTreeMap::from([("app.kubernetes.io/name".to_string(), chart.to_string())]),
        }
    }

    /// The `NetworkPolicy` peer selecting these pods.
    pub fn peer(&self) -> NetworkPolicyPeer {
        NetworkPolicyPeer {
            namespace_selector: self.namespace.as_ref().map(|ns| LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".into(),
                    ns.clone(),
                )])),
                ..Default::default()
            }),
            pod_selector: Some(LabelSelector {
                match_labels: Some(self.labels.clone()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

/// The pod security context of a managed pod when the spec sets none:
/// non-root `uid`, `RuntimeDefault` seccomp.
pub fn restricted_pod_security(uid: i64) -> PodSecurityContext {
    PodSecurityContext {
        run_as_non_root: Some(true),
        run_as_user: Some(uid),
        run_as_group: Some(uid),
        fs_group: Some(uid),
        seccomp_profile: Some(SeccompProfile {
            type_: "RuntimeDefault".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The container security context of a managed pod when the spec sets
/// none: read-only root filesystem, no capability, no escalation.
pub fn restricted_container_security() -> SecurityContext {
    SecurityContext {
        allow_privilege_escalation: Some(false),
        read_only_root_filesystem: Some(true),
        capabilities: Some(Capabilities {
            drop: Some(vec!["ALL".into()]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// What a managed instance's `NetworkPolicy` lets through.
#[derive(Debug, Clone, Default)]
pub struct PolicyRules<'a> {
    /// The pods it applies to.
    pub pod_labels: BTreeMap<String, String>,
    /// The container port let in (by name).
    pub port: &'a str,
    /// Who may connect; empty: anyone.
    pub ingress_from: &'a [NetworkPolicyPeer],
    /// Where the pods may connect besides DNS; empty: DNS only.
    pub egress_to: &'a [NetworkPolicyPeer],
    /// Let in besides `ingress_from` when it isn't empty: the operator,
    /// when it calls the instance.
    pub operator: Option<&'a OperatorPod>,
}

/// The `NetworkPolicy` `metadata` names, from `rules`.
pub fn network_policy(metadata: ObjectMeta, rules: &PolicyRules<'_>) -> NetworkPolicy {
    let mut from = rules.ingress_from.to_vec();
    if let Some(operator) = rules.operator.filter(|_| !from.is_empty()) {
        from.push(operator.peer());
    }
    let dns = NetworkPolicyEgressRule {
        ports: Some(
            ["UDP", "TCP"]
                .iter()
                .map(|protocol| NetworkPolicyPort {
                    port: Some(IntOrString::Int(53)),
                    protocol: Some(protocol.to_string()),
                    ..Default::default()
                })
                .collect(),
        ),
        ..Default::default()
    };
    let mut egress = vec![dns];
    if !rules.egress_to.is_empty() {
        egress.push(NetworkPolicyEgressRule {
            to: Some(rules.egress_to.to_vec()),
            ..Default::default()
        });
    }
    NetworkPolicy {
        metadata,
        spec: Some(NetworkPolicySpec {
            pod_selector: Some(LabelSelector {
                match_labels: Some(rules.pod_labels.clone()),
                ..Default::default()
            }),
            policy_types: Some(vec!["Ingress".into(), "Egress".into()]),
            ingress: Some(vec![NetworkPolicyIngressRule {
                from: (!from.is_empty()).then_some(from),
                ports: Some(vec![NetworkPolicyPort {
                    port: Some(IntOrString::String(rules.port.into())),
                    protocol: Some("TCP".into()),
                    ..Default::default()
                }]),
            }]),
            egress: Some(egress),
        }),
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::Secret;
    use serde_json::json;

    use super::*;

    fn at(text: &str) -> Option<Timestamp> {
        Some(text.parse().unwrap())
    }

    #[test]
    fn an_object_is_its_manifest() {
        let secret = Secret {
            metadata: ObjectMeta {
                name: Some("s".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let object = OwnedObject::of(&secret, true);
        assert_eq!(
            (
                object.api_version.as_str(),
                object.kind.as_str(),
                object.name.as_str()
            ),
            ("v1", "Secret", "s")
        );
        assert_eq!(object.manifest["metadata"]["name"], "s");
        assert!(object.create_only);
    }

    #[test]
    fn hashed_names_follow_the_content() {
        assert_eq!(hashed_name("i-boot", b"a"), hashed_name("i-boot", b"a"));
        assert_ne!(hashed_name("i-boot", b"a"), hashed_name("i-boot", b"b"));
        assert_eq!(hashed_name("i-boot", b"a").len(), "i-boot-".len() + 8);
        assert_eq!(
            content_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn the_newest_unused_are_kept() {
        let listed = [
            Listed {
                name: "a".into(),
                created: at("2026-01-01T00:00:00Z"),
            },
            Listed {
                name: "b".into(),
                created: at("2026-01-02T00:00:00Z"),
            },
            Listed {
                name: "c".into(),
                created: at("2026-01-03T00:00:00Z"),
            },
            Listed {
                name: "d".into(),
                created: at("2026-01-04T00:00:00Z"),
            },
            Listed {
                name: "x".into(),
                created: None,
            },
        ];
        let in_use = BTreeSet::from(["d".to_string()]);
        assert_eq!(stale(&listed, &in_use, 2), ["a", "x"]);
        assert_eq!(stale(&listed, &in_use, 0), ["c", "b", "a", "x"]);
        assert!(stale(&listed, &in_use, 10).is_empty());
    }

    #[test]
    fn timestamped_logs_parse() {
        let lines = parse_timestamped(
            "2026-10-16T10:00:00.123456789Z {\"a\":1}\nnot a time\n2026-10-16T10:00:01Z b c\n",
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "{\"a\":1}");
        assert_eq!(lines[1].text, "b c");
        assert!(lines[0].time < lines[1].time);
    }

    #[test]
    fn the_operator_is_let_in_only_beside_other_peers() {
        let operator = OperatorPod {
            namespace: Some("ops".into()),
            ..OperatorPod::named("weebo-example")
        };
        let peers = [NetworkPolicyPeer {
            pod_selector: Some(LabelSelector::default()),
            ..Default::default()
        }];
        let rules = PolicyRules {
            pod_labels: BTreeMap::from([("app".into(), "x".into())]),
            port: "http",
            ingress_from: &peers,
            egress_to: &[],
            operator: Some(&operator),
        };
        let policy = serde_json::to_value(network_policy(ObjectMeta::default(), &rules)).unwrap();
        let from = &policy["spec"]["ingress"][0]["from"];
        assert_eq!(from.as_array().unwrap().len(), 2);
        assert_eq!(
            from[1],
            json!({
                "namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "ops"}},
                "podSelector": {"matchLabels": {"app.kubernetes.io/name": "weebo-example"}},
            })
        );
        assert_eq!(policy["spec"]["egress"].as_array().unwrap().len(), 1);
        // Open to anyone: no peer list at all.
        let open = PolicyRules {
            ingress_from: &[],
            ..rules
        };
        let policy = serde_json::to_value(network_policy(ObjectMeta::default(), &open)).unwrap();
        assert!(policy["spec"]["ingress"][0].get("from").is_none());
    }

    #[test]
    fn rollouts_complete_when_every_replica_is_updated_and_available() {
        let mut rollout = Rollout {
            replicas: 2,
            updated: 2,
            available: 1,
            observed: true,
            template_annotations: BTreeMap::from([("h".into(), "1".into())]),
        };
        assert!(!rollout.complete());
        rollout.available = 2;
        assert!(rollout.complete());
        assert_eq!(rollout.annotation("h"), Some("1"));
    }
}
