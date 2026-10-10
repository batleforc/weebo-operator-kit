//! Writing a value the remote system shows only once (client secret,
//! token) to every target of a CR, and detecting a target that lost it.
//! Errors stay [`SecretStoreError`]s: the operator maps them onto its own
//! reason code.

use std::collections::BTreeMap;

use weebo_kit_api::SecretTarget;

use crate::ports::{SecretOwner, SecretStore, SecretStoreError};

/// The CR's own targets, or the implicit one: named after the CR, in its
/// namespace, on the instance's default backend.
pub fn targets(
    explicit: &[SecretTarget],
    cr_name: &str,
    store: &dyn SecretStore,
) -> Vec<SecretTarget> {
    if !explicit.is_empty() {
        return explicit.to_vec();
    }
    vec![SecretTarget {
        backend: store.default_backend(),
        name: cr_name.to_string(),
        namespace: None,
        path: None,
    }]
}

/// Why a target is out of reach of its owner.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetViolation {
    /// A namespaced CR naming another namespace: whoever may create the CR
    /// in its namespace would otherwise write (and, on deletion, delete)
    /// Secrets anywhere the operator can.
    #[error(
        "secret target {name:?} names namespace {target:?}; a CR in {own:?} may only write into its own namespace"
    )]
    CrossNamespace {
        name: String,
        target: String,
        own: String,
    },
    /// A cluster-scoped CR's target without a namespace.
    #[error("secret target {name:?} needs a namespace on a cluster-scoped CR")]
    MissingNamespace { name: String },
}

/// The namespace rules of a target, checked before anything is written
/// (and again by the stores): a namespaced CR's targets stay in its own
/// namespace, a cluster-scoped CR's name one.
pub fn check_target(target: &SecretTarget, owner: &SecretOwner<'_>) -> Result<(), TargetViolation> {
    match (owner.namespace, target.namespace.as_deref()) {
        (Some(own), Some(other)) if own != other => Err(TargetViolation::CrossNamespace {
            name: target.name.clone(),
            target: other.to_string(),
            own: own.to_string(),
        }),
        // A Vault target with an explicit path doesn't need a namespace.
        (None, None) if target.path.is_none() => Err(TargetViolation::MissingNamespace {
            name: target.name.clone(),
        }),
        _ => Ok(()),
    }
}

pub fn check_targets(
    targets: &[SecretTarget],
    owner: &SecretOwner<'_>,
) -> Result<(), TargetViolation> {
    targets.iter().try_for_each(|t| check_target(t, owner))
}

pub async fn any_missing(
    targets: &[SecretTarget],
    owner: &SecretOwner<'_>,
    store: &dyn SecretStore,
) -> Result<bool, SecretStoreError> {
    for target in targets {
        if !store.exists(target, owner).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn write_all(
    targets: &[SecretTarget],
    owner: &SecretOwner<'_>,
    data: &BTreeMap<String, String>,
    store: &dyn SecretStore,
) -> Result<(), SecretStoreError> {
    for target in targets {
        store.write(target, owner, data).await?;
    }
    Ok(())
}

pub async fn delete_all(
    targets: &[SecretTarget],
    owner: &SecretOwner<'_>,
    store: &dyn SecretStore,
) -> Result<(), SecretStoreError> {
    for target in targets {
        store.delete(target, owner).await?;
    }
    Ok(())
}

/// A rotation was requested through a nonce the CR hasn't been rotated
/// for yet.
pub fn rotation_requested(nonce: &Option<String>, last_rotation: &Option<String>) -> bool {
    nonce.is_some() && nonce != last_rotation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeSecretStore;

    fn target(name: &str) -> SecretTarget {
        SecretTarget {
            backend: Default::default(),
            name: name.into(),
            namespace: None,
            path: None,
        }
    }

    const OWNER: SecretOwner<'static> = SecretOwner {
        kind: "Token",
        namespace: Some("ns"),
        name: "app",
    };

    #[tokio::test]
    async fn implicit_target_is_named_after_the_cr() {
        let store = FakeSecretStore::default();
        assert_eq!(targets(&[], "app", &store), vec![target("app")]);
        assert_eq!(targets(&[target("x")], "app", &store), vec![target("x")]);
    }

    #[tokio::test]
    async fn fan_out_and_loss_detection() {
        let store = FakeSecretStore::default();
        let all = [target("a"), target("b")];
        let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
        assert!(any_missing(&all, &OWNER, &store).await.unwrap());
        write_all(&all, &OWNER, &data, &store).await.unwrap();
        assert!(!any_missing(&all, &OWNER, &store).await.unwrap());
        store.remove("ns/b");
        assert!(any_missing(&all, &OWNER, &store).await.unwrap());
        delete_all(&all, &OWNER, &store).await.unwrap();
        assert_eq!(store.get("ns/a"), None);
        store.fail_writes(true);
        assert!(write_all(&all, &OWNER, &data, &store).await.is_err());
    }

    #[tokio::test]
    async fn another_owners_value_is_neither_overwritten_nor_deleted() {
        let store = FakeSecretStore::default();
        let other = SecretOwner {
            name: "other",
            ..OWNER
        };
        let data = BTreeMap::from([("k".to_string(), "v".to_string())]);
        store.write(&target("a"), &other, &data).await.unwrap();
        assert!(!store.exists(&target("a"), &OWNER).await.unwrap());
        assert!(store.write(&target("a"), &OWNER, &data).await.is_err());
        store.delete(&target("a"), &OWNER).await.unwrap();
        assert_eq!(store.get("ns/a"), Some(data));
    }

    #[test]
    fn namespaced_owners_stay_in_their_namespace() {
        let mut t = target("a");
        assert_eq!(check_target(&t, &OWNER), Ok(()));
        t.namespace = Some("ns".into());
        assert_eq!(check_target(&t, &OWNER), Ok(()));
        t.namespace = Some("kube-system".into());
        assert!(matches!(
            check_target(&t, &OWNER),
            Err(TargetViolation::CrossNamespace { .. })
        ));
        let cluster = SecretOwner::cluster("User", "alice");
        assert_eq!(check_target(&t, &cluster), Ok(()));
        assert!(matches!(
            check_target(&target("a"), &cluster),
            Err(TargetViolation::MissingNamespace { .. })
        ));
    }

    #[test]
    fn owner_ids() {
        assert_eq!(OWNER.id(), "Token/ns/app");
        assert_eq!(SecretOwner::cluster("User", "alice").id(), "User/alice");
    }

    #[test]
    fn rotation_needs_a_new_nonce() {
        let n = |s: &str| Some(s.to_string());
        assert!(!rotation_requested(&None, &n("1")));
        assert!(rotation_requested(&n("2"), &n("1")));
        assert!(!rotation_requested(&n("1"), &n("1")));
    }
}
