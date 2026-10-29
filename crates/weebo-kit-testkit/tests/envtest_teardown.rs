//! The control plane never outlives its `EnvTestCluster`: stopped when the
//! cluster is dropped, and when the start fails after it came up (a CRD
//! the API server refuses). Real `kube-apiserver` + `etcd`, so ignored by
//! `cargo test`: `task test:envtest` (with `KUBEBUILDER_ASSETS` naming the
//! binaries, nothing is downloaded).
//!
//! Linux only: the control plane's processes are found in `/proc`, as the
//! children of this test process.

#![cfg(target_os = "linux")]
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers fail the test on purpose"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use weebo_kit_testkit::envtest::EnvTestCluster;

/// `kube-apiserver` and `etcd` processes started by this process.
fn control_plane_children() -> Vec<String> {
    let me = std::process::id().to_string();
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        // `<pid> (<comm>) <state> <ppid> …`; comm may hold spaces.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((comm, rest)) = stat
            .split_once('(')
            .and_then(|(_, rest)| rest.rsplit_once(')'))
        else {
            continue;
        };
        let ppid = rest.split_whitespace().nth(1);
        if ppid == Some(me.as_str()) && matches!(comm, "kube-apiserver" | "etcd") {
            found.push(comm.to_string());
        }
    }
    found
}

/// Waits (a few seconds at most) for the control plane's processes to be gone.
async fn assert_no_control_plane_left() {
    for _ in 0..50 {
        if control_plane_children().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("left running: {:?}", control_plane_children());
}

/// A directory holding one CRD manifest, `crd`.
fn crd_dir(name: &str, crd: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("crd.yaml"), crd).unwrap();
    // Not a CRD: skipped, like envtest skips it.
    std::fs::write(dir.join("links.json"), r#"{"kinds": []}"#).unwrap();
    dir
}

fn widget_crd(name: &str) -> String {
    format!(
        "apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: {name}
spec:
  group: example.weebo.io
  scope: Namespaced
  names: {{plural: widgets, singular: widget, kind: Widget, listKind: WidgetList}}
  versions:
    - name: v1
      served: true
      storage: true
      schema:
        openAPIV3Schema: {{type: object, x-kubernetes-preserve-unknown-fields: true}}
"
    )
}

#[tokio::test]
#[ignore = "starts a control plane (envtest): task test:envtest"]
async fn a_cluster_installs_its_crds_and_stops_when_dropped() {
    let dir = crd_dir("served-crd", &widget_crd("widgets.example.weebo.io"));
    let cluster = EnvTestCluster::try_start(&dir).await.unwrap();
    // The CRD is served: a widget can be listed.
    let widgets = kube::Api::<kube::api::DynamicObject>::namespaced_with(
        cluster.client(),
        "default",
        &kube::api::ApiResource {
            group: "example.weebo.io".into(),
            version: "v1".into(),
            api_version: "example.weebo.io/v1".into(),
            kind: "Widget".into(),
            plural: "widgets".into(),
        },
    );
    assert!(
        widgets
            .list(&Default::default())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    // What the teardown checks below look for is there while it runs.
    let running = control_plane_children();
    assert!(running.iter().any(|p| p == "kube-apiserver"), "{running:?}");
    assert!(running.iter().any(|p| p == "etcd"), "{running:?}");
    drop(cluster);
    assert_no_control_plane_left().await;
}

#[tokio::test]
#[ignore = "starts a control plane (envtest): task test:envtest"]
async fn a_crd_the_api_server_refuses_stops_the_control_plane() {
    // The name must be `<plural>.<group>`: refused by the API server, once
    // the control plane is up.
    let dir = crd_dir("refused-crd", &widget_crd("gadgets.example.weebo.io"));
    let err = match EnvTestCluster::try_start(&dir).await {
        Ok(_) => panic!("a CRD named after another plural was installed"),
        Err(e) => e.to_string(),
    };
    assert!(
        err.contains("installing the CRD \"gadgets.example.weebo.io\""),
        "{err}"
    );
    assert_no_control_plane_left().await;
}
