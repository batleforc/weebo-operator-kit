//! Writing a value the remote system shows only once (client secret,
//! token) to every target of a CR, and detecting a target that lost it.
//! Errors stay [`SecretStoreError`]s: the operator maps them onto its own
//! reason code.

use std::collections::BTreeMap;

use weebo_kit_api::SecretTarget;

use crate::ports::{SecretStore, SecretStoreError};

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

pub async fn any_missing(
    targets: &[SecretTarget],
    namespace: Option<&str>,
    store: &dyn SecretStore,
) -> Result<bool, SecretStoreError> {
    for target in targets {
        if !store.exists(target, namespace).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn write_all(
    targets: &[SecretTarget],
    namespace: Option<&str>,
    data: &BTreeMap<String, String>,
    store: &dyn SecretStore,
) -> Result<(), SecretStoreError> {
    for target in targets {
        store.write(target, namespace, data).await?;
    }
    Ok(())
}

pub async fn delete_all(
    targets: &[SecretTarget],
    namespace: Option<&str>,
    store: &dyn SecretStore,
) -> Result<(), SecretStoreError> {
    for target in targets {
        store.delete(target, namespace).await?;
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
        assert!(any_missing(&all, Some("ns"), &store).await.unwrap());
        write_all(&all, Some("ns"), &data, &store).await.unwrap();
        assert!(!any_missing(&all, Some("ns"), &store).await.unwrap());
        store.remove("ns/b");
        assert!(any_missing(&all, Some("ns"), &store).await.unwrap());
        delete_all(&all, Some("ns"), &store).await.unwrap();
        assert_eq!(store.get("ns/a"), None);
        store.fail_writes(true);
        assert!(write_all(&all, Some("ns"), &data, &store).await.is_err());
    }

    #[test]
    fn rotation_needs_a_new_nonce() {
        let n = |s: &str| Some(s.to_string());
        assert!(!rotation_requested(&None, &n("1")));
        assert!(rotation_requested(&n("2"), &n("1")));
        assert!(!rotation_requested(&n("1"), &n("1")));
    }
}
