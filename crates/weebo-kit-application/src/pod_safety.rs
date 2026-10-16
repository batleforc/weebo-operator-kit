//! What a managed instance's pods may be. Creating an instance must not
//! give more than the instance's own software:
//!
//! - images from an allow-list of repositories only (the operator's
//!   setting, its product's images by default);
//! - no `hostPath` volume, no privileged container, no privilege
//!   escalation, no added capability, no host namespace, no `procMount`;
//! - every `Secret` the pods read (`env[].valueFrom.secretKeyRef`,
//!   `envFrom[].secretRef`, a `secret` or projected volume) labelled with
//!   the operator's credential label — the operator writes the pods, so it
//!   would otherwise hand any `Secret` of the namespace to a pod the CR's
//!   author configures.
//!
//! Pure: checked on the pod templates the operator is about to apply (the
//! webhook builds them from the spec, the apply checks them again).
//! Neutral: problems come back as words, the operator words the refusal
//! and picks its reason code.

use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::{PodSpec, SecurityContext};

use crate::workload::OwnedObject;

/// How many problems [`summary`] names.
const SHOWN: usize = 5;

/// The allow-list as the operator's setting writes it: comma-, space- or
/// newline-separated repositories; empty or unset, `defaults`.
pub fn parse_allowed_images(setting: Option<&str>, defaults: &[&str]) -> Vec<String> {
    let list: Vec<String> = setting
        .unwrap_or_default()
        .split([',', ' ', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if list.is_empty() {
        defaults.iter().map(|d| d.to_string()).collect()
    } else {
        list
    }
}

/// The repository of an image reference: without its digest and tag.
pub fn repository(image: &str) -> &str {
    let image = image.split_once('@').map_or(image, |(name, _)| name);
    match image.rfind(':') {
        // A colon after the last slash is the tag's, before it a port's.
        Some(at) if !image[at..].contains('/') => &image[..at],
        _ => image,
    }
}

fn container_violations(
    out: &mut Vec<String>,
    container: &str,
    image: Option<&str>,
    security: Option<&SecurityContext>,
    allowed: &[String],
) {
    match image {
        Some(image) if allowed.iter().any(|a| a == repository(image)) => {}
        Some(image) => out.push(format!(
            "container {container}: image {image} is not from an allowed repository ({})",
            allowed.join(", ")
        )),
        None => out.push(format!("container {container}: no image")),
    }
    let Some(sc) = security else {
        return;
    };
    if sc.privileged == Some(true) {
        out.push(format!("container {container}: privileged"));
    }
    if sc.allow_privilege_escalation == Some(true) {
        out.push(format!("container {container}: allowPrivilegeEscalation"));
    }
    if sc
        .capabilities
        .as_ref()
        .and_then(|c| c.add.as_ref())
        .is_some_and(|add| !add.is_empty())
    {
        out.push(format!("container {container}: capabilities.add"));
    }
    if sc
        .windows_options
        .as_ref()
        .and_then(|w| w.host_process)
        .unwrap_or(false)
    {
        out.push(format!("container {container}: windowsOptions.hostProcess"));
    }
    if sc.proc_mount.as_deref().is_some_and(|m| m != "Default") {
        out.push(format!("container {container}: procMount"));
    }
}

/// What `pod` may not do, in words (empty: nothing).
pub fn violations(pod: &PodSpec, allowed: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for c in pod
        .containers
        .iter()
        .chain(pod.init_containers.iter().flatten())
    {
        container_violations(
            &mut out,
            &c.name,
            c.image.as_deref(),
            c.security_context.as_ref(),
            allowed,
        );
    }
    for c in pod.ephemeral_containers.iter().flatten() {
        container_violations(
            &mut out,
            &c.name,
            c.image.as_deref(),
            c.security_context.as_ref(),
            allowed,
        );
    }
    for v in pod.volumes.iter().flatten() {
        if v.host_path.is_some() {
            out.push(format!("volume {}: hostPath", v.name));
        }
    }
    for (set, what) in [
        (pod.host_network, "hostNetwork"),
        (pod.host_pid, "hostPID"),
        (pod.host_ipc, "hostIPC"),
    ] {
        if set == Some(true) {
            out.push(what.to_string());
        }
    }
    if pod
        .security_context
        .as_ref()
        .and_then(|s| s.windows_options.as_ref())
        .and_then(|w| w.host_process)
        .unwrap_or(false)
    {
        out.push("podSecurityContext.windowsOptions.hostProcess".into());
    }
    out
}

/// The `Secret`s `pod` reads (`imagePullSecrets` aside: the kubelet only
/// presents them to the image's registry, an allowed one).
pub fn secret_names(pod: &PodSpec) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for c in pod
        .containers
        .iter()
        .chain(pod.init_containers.iter().flatten())
    {
        for env in c.env.iter().flatten() {
            if let Some(r) = env
                .value_from
                .as_ref()
                .and_then(|v| v.secret_key_ref.as_ref())
            {
                names.insert(r.name.clone());
            }
        }
        for from in c.env_from.iter().flatten() {
            if let Some(r) = &from.secret_ref {
                names.insert(r.name.clone());
            }
        }
    }
    for v in pod.volumes.iter().flatten() {
        if let Some(name) = v.secret.as_ref().and_then(|s| s.secret_name.as_ref()) {
            names.insert(name.clone());
        }
        for source in v
            .projected
            .as_ref()
            .and_then(|p| p.sources.as_ref())
            .into_iter()
            .flatten()
        {
            if let Some(s) = &source.secret {
                names.insert(s.name.clone());
            }
        }
    }
    names
}

/// Where the pod template of a workload kind is, in its manifest.
fn template_pointer(kind: &str) -> Option<&'static str> {
    match kind {
        "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Job" => {
            Some("/spec/template/spec")
        }
        "CronJob" => Some("/spec/jobTemplate/spec/template/spec"),
        "Pod" => Some("/spec"),
        _ => None,
    }
}

/// The pod templates among `objects`: `(object name, pod spec)`.
pub fn pod_templates(objects: &[OwnedObject]) -> Vec<(String, PodSpec)> {
    objects
        .iter()
        .filter_map(|o| {
            let spec = o.manifest.pointer(template_pointer(&o.kind)?)?.clone();
            Some((o.name.clone(), serde_json::from_value(spec).ok()?))
        })
        .collect()
}

/// What the pods of `objects` may not do, each prefixed by its object
/// (empty: nothing).
pub fn problems(objects: &[OwnedObject], allowed: &[String]) -> Vec<String> {
    pod_templates(objects)
        .iter()
        .flat_map(|(object, pod)| {
            violations(pod, allowed)
                .into_iter()
                .map(move |v| format!("{object}: {v}"))
        })
        .collect()
}

/// The first problems, joined, with how many more there are: what a
/// refusal message quotes.
pub fn summary(problems: &[String]) -> String {
    let more = match problems.len().saturating_sub(SHOWN) {
        0 => String::new(),
        n => format!(" and {n} more"),
    };
    format!(
        "{}{more}",
        problems
            .iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    )
}

/// The `Secret`s the pods of `objects` read that `objects` doesn't write
/// itself (the operator's own configuration): each must consent.
pub fn secrets_to_check(objects: &[OwnedObject]) -> BTreeSet<String> {
    let written: BTreeSet<&str> = objects
        .iter()
        .filter(|o| o.kind == "Secret")
        .map(|o| o.name.as_str())
        .collect();
    pod_templates(objects)
        .iter()
        .flat_map(|(_, pod)| secret_names(pod))
        .filter(|name| !written.contains(name.as_str()))
        .collect()
}

/// Why a pod may not read `namespace/name`, and how to let it.
pub fn secret_not_labelled(namespace: &str, name: &str, label: &str) -> String {
    format!(
        "the instance's pods would read secret {namespace}/{name}, which lacks the label \
         {label}=\"true\": the operator writes the pods, so every Secret they read must opt in \
         (kubectl label secret -n {namespace} {name} {label}=true)"
    )
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{
        Capabilities, Container, EnvFromSource, EnvVar, EnvVarSource, HostPathVolumeSource,
        ProjectedVolumeSource, SecretEnvSource, SecretKeySelector, SecretProjection,
        SecretVolumeSource, Volume, VolumeProjection,
    };
    use serde_json::json;

    use super::*;

    const DEFAULTS: &[&str] = &["registry.example/app"];

    fn allowed() -> Vec<String> {
        parse_allowed_images(None, DEFAULTS)
    }

    fn deployment(name: &str, pod: serde_json::Value) -> OwnedObject {
        OwnedObject {
            api_version: "apps/v1".into(),
            kind: "Deployment".into(),
            name: name.into(),
            manifest: json!({"spec": {"template": {"spec": pod}}}),
            create_only: false,
        }
    }

    #[test]
    fn repositories_are_read_without_tag_or_digest() {
        assert_eq!(
            repository("registry.example/app:1.6.0"),
            "registry.example/app"
        );
        assert_eq!(
            repository("registry.example/app@sha256:ab"),
            "registry.example/app"
        );
        assert_eq!(
            repository("registry:5000/app:1.6.0@sha256:ab"),
            "registry:5000/app"
        );
        assert_eq!(repository("registry:5000/app"), "registry:5000/app");
        assert_eq!(allowed(), ["registry.example/app"]);
        assert_eq!(parse_allowed_images(Some(" "), DEFAULTS), allowed());
        assert_eq!(
            parse_allowed_images(Some("a/b, c/d"), DEFAULTS),
            ["a/b", "c/d"]
        );
    }

    #[test]
    fn foreign_images_and_privileges_are_refused() {
        let ok = json!({"containers": [{"name": "app", "image": "registry.example/app:1"}]});
        assert!(problems(&[deployment("d", ok)], &allowed()).is_empty());
        let refused = [
            json!({"containers": [{"name": "app", "image": "evil/miner:1"}]}),
            json!({"containers": [{"name": "app"}]}),
            json!({"containers": [{"name": "app", "image": "registry.example/app:1",
                "securityContext": {"privileged": true}}]}),
            json!({"containers": [{"name": "app", "image": "registry.example/app:1",
                "securityContext": {"procMount": "Unmasked"}}]}),
            json!({"containers": [{"name": "app", "image": "registry.example/app:1"}],
                "initContainers": [{"name": "i", "image": "evil/x"}]}),
            json!({"containers": [{"name": "app", "image": "registry.example/app:1"}],
                "hostPID": true}),
        ];
        for pod in refused {
            assert_eq!(
                problems(&[deployment("d", pod.clone())], &allowed()).len(),
                1,
                "{pod}"
            );
        }
        // A Job's and a CronJob's pods are checked too.
        let job = OwnedObject {
            api_version: "batch/v1".into(),
            kind: "Job".into(),
            name: "j".into(),
            manifest: json!({"spec": {"template": {"spec":
                {"containers": [{"name": "c", "image": "evil/x"}]}}}}),
            create_only: false,
        };
        let cron = OwnedObject {
            kind: "CronJob".into(),
            name: "c".into(),
            manifest: json!({"spec": {"jobTemplate": {"spec": {"template": {"spec":
                {"containers": [{"name": "c", "image": "evil/x"}]}}}}}}),
            ..job.clone()
        };
        assert_eq!(
            problems(&[job, cron], &allowed()),
            [
                "j: container c: image evil/x is not from an allowed repository (registry.example/app)",
                "c: container c: image evil/x is not from an allowed repository (registry.example/app)",
            ]
        );
    }

    #[test]
    fn a_summary_names_the_first_five() {
        let problems: Vec<String> = (1..=7).map(|i| format!("p{i}")).collect();
        assert_eq!(summary(&problems), "p1; p2; p3; p4; p5 and 2 more");
        assert_eq!(summary(&problems[..1]), "p1");
    }

    #[test]
    fn every_secret_the_pods_read_is_listed_but_the_operator_s_own() {
        let objects = [
            OwnedObject {
                api_version: "v1".into(),
                kind: "Secret".into(),
                name: "own".into(),
                manifest: json!({}),
                create_only: false,
            },
            deployment(
                "d",
                json!({
                    "containers": [{
                        "name": "app",
                        "env": [{"name": "A", "valueFrom": {"secretKeyRef": {"name": "env-secret", "key": "a"}}}],
                        "envFrom": [{"secretRef": {"name": "from-secret"}}],
                    }],
                    "volumes": [
                        {"name": "o", "secret": {"secretName": "own"}},
                        {"name": "v", "secret": {"secretName": "volume-secret"}},
                        {"name": "p", "projected": {"sources": [{"secret": {"name": "projected-secret"}}]}},
                    ],
                }),
            ),
        ];
        assert_eq!(
            secrets_to_check(&objects).into_iter().collect::<Vec<_>>(),
            [
                "env-secret",
                "from-secret",
                "projected-secret",
                "volume-secret"
            ]
        );
        let message = secret_not_labelled("team", "s", "example.weebo.io/credential");
        assert!(
            message.contains("kubectl label secret -n team s example.weebo.io/credential=true")
        );
    }

    #[test]
    fn a_pod_spec_is_judged_whole() {
        let pod = PodSpec {
            containers: vec![Container {
                name: "c".into(),
                image: Some("registry.example/app:1".into()),
                security_context: Some(SecurityContext {
                    capabilities: Some(Capabilities {
                        add: Some(vec![]),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                env: Some(vec![EnvVar {
                    name: "X".into(),
                    value_from: Some(EnvVarSource {
                        secret_key_ref: Some(SecretKeySelector {
                            name: "s1".into(),
                            key: "k".into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                env_from: Some(vec![EnvFromSource {
                    secret_ref: Some(SecretEnvSource {
                        name: "s2".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            volumes: Some(vec![
                Volume {
                    name: "h".into(),
                    host_path: Some(HostPathVolumeSource {
                        path: "/var/run".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Volume {
                    name: "s".into(),
                    secret: Some(SecretVolumeSource {
                        secret_name: Some("s3".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Volume {
                    name: "p".into(),
                    projected: Some(ProjectedVolumeSource {
                        sources: Some(vec![VolumeProjection {
                            secret: Some(SecretProjection {
                                name: "s4".into(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ]),
            host_network: Some(true),
            ..Default::default()
        };
        assert_eq!(
            violations(&pod, &allowed()),
            ["volume h: hostPath", "hostNetwork"]
        );
        assert_eq!(
            secret_names(&pod).into_iter().collect::<Vec<_>>(),
            ["s1", "s2", "s3", "s4"]
        );
    }
}
